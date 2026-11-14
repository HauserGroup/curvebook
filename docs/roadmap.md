# Roadmap: stage gates, execution order and milestones

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-14 · **Change:** Wave 1–4 parity across `G1`–`G14`, `< 50 ms` 96-curve 4PL optimization (`22.669 ms` fit / `26.25 ms` total in `--release`), `D1` R `4.5.2` edge matrices, WAL crash-recovery journal, `%PDF-1.7` + `/FontFile2` embedding, `16` locales × `167` keys = `2,672` + RTL `ar`/`he` layout mirroring, and `UiApp` UX parity (`NavigatorDragState`, `PortfolioGalleryState`, `F6`/`Shift+F6`, rulers, `ShortcutRegistry`, subprocess OS adapters) landed and verified (`602` tests across `21` packages, `0` failed); remaining hours narrowed to Alpha `15–25 h`, Core `+6–10 h` (`21–35 h`), Beta `+2–3 h` (`23–38 h`), and `1.0` (`37–60 h` incl. `G13` + `M7`) · **Target:** GraphPad Prism 11 (11.1.x)

Forward-looking engineering plan. Current status and headline numbers live in [`ROADMAP.md`](../ROADMAP.md) and [`target-app-parity.md`](target-app-parity.md); the ranked task queue lives in [`gaps.md`](gaps.md). Hours are single-agent implementation estimates and are sums of the hours in [`gaps.md`](gaps.md), which is the source of truth. They do **not** include the time a person needs to review the work (see [Landing rules](#landing-rules)).

There are three gates on the way to `1.0`: **Alpha** (Essentials scope), **Core workflows**, and **Beta**. Each gate is passed only by behaviour that is merged, command-wired, and reachable from the CLI and the desktop UI.

---

## Alpha gate

The smallest release a real person can use: a student or bench scientist takes one experiment from raw numbers to an exported figure. LumenCraft enters **alpha** when every row below works end to end on desktop and headless CLI, with lossless save and reopen in `.lumencraft` (`crates/format/tests/prop_format.rs`) and numerics verified against a named [oracle](#verification-oracles).

| Workflow | Alpha (Essentials) scope | Works end to end? | Gaps | Est. remaining (h) |
|---|---|:---:|---|---:|
| **1. Enter, paste, exclude & transform data** | `XY`, `Column`, and `Grouped` tables with replicate subcolumns; CSV/clipboard paste; non-destructive exclusions; Info constants; log and normalize transforms; `.lumencraft` save/reopen; undo/redo | yes (CLI, MCP & `UiApp` ✅; `e2e_workflows` + `slice0_vertical_gate` + WAL `recovery_and_interop_parity`) | `G1`, `G2`, `G9` | — |
| **2. Compare groups** | descriptive statistics; unpaired, Welch, and paired *t*-tests; ordinary one-way and two-way ANOVA; Tukey, Dunnett, Bonferroni, and Holm–Šídák comparisons | yes (engine, CLI, MCP & `UiApp` ✅; `stats` `66 passed`) | `G3` | — |
| **3. Fit curves & interpolate unknowns** | weighted Levenberg–Marquardt; linear regression; 3PL/4PL dose–response, Michaelis–Menten, and one-phase exponential models; asymptotic CIs; standard-curve interpolation | yes (engine, CLI, MCP & `UiApp` ✅; `fit` `82 passed` w/ `InterpolatedUnknowns`) | `G4` | — |
| **4. Build graphs** | XY scatter with fitted curve; bar with individual replicates; box plot; linear and log axes; error bars | yes (`chart` + `engine` + `UiApp` ✅; `90` visual tests) | `G5` | — |
| **5. Export** | SVG, PDF, and PNG figures; tidy CSV/JSON results | yes (programmatic SVG/`%PDF-1.7` (with `--pdf-version 1.4` & `/FontFile2`)/PNG/TIFF/CSV ✅) | `G5` | — (external GUI viewer QA `1–2 h` in Core) |
| **6. Drive it headlessly** | create tables, run analyses and fits, read results, and render previews through the CLI, `--control`, and MCP | yes (CLI `16 passed`, MCP `25 passed`, `lumencraft` `8 passed`, `web` `1 passed` ✅) | `G14` | — |
| **Desktop & WASM shell** | Project Navigator, subcolumn grid, Analyze dialog, Result tabs, basic Format Graph | partial (`UiApp` state shell + software compositor `59 passed` ✅; live host OS `eframe`/`wgpu` & `wasm32` windowing pending) | `G6` | 15–25 |
| **Remaining to desktop Alpha gate** | *(headless Alpha workflows 1–6 already pass)* | | | **15–25** |

---

## Core-workflow gate

The checkpoint between alpha and beta, and the scope that was previously called the alpha gate: the six everyday workflows at the depth a working wet-lab biologist or pharmacologist needs. It is passed when the Alpha gate holds **and** each workflow gains the scope below. Core-workflow library, engine, and headless support is landed and verified (`stats` `66`, `fit` `82`, `format` `43`, `chart` `35`, `pdf` `11`, `engine` `56`); remaining hours below cover external GUI PDF/SVG viewer QA (`G5`) and wiring live OS pointer/inline-edit event handlers inside `G6` desktop windowing.

| Workflow | Added scope | Governing spec | Gaps | Est. remaining (h) |
|---|---|---|---|---:|
| **1. Data** | live cross-table `Paste Link`, reactive transpose, 4-stage import, `CrashRecoveryJournal` WAL + torn-tail replay, `MIGRATION_STEPS` forward migrations, and `sanitize_circular_hooks` (all landed ✅; browser `localStorage` Web Locks FFI joins `wasm32` UI host in `G6`) | [data-tables-and-transforms.md](data-tables-and-transforms.md) | `G2` | — |
| **2. Compare groups** | normality battery and Grubbs/ROUT outliers, Mann–Whitney, Wilcoxon, Kruskal–Wallis, Friedman, Welch/RM/3-way ANOVA, REML, multiplicity (`NewmanKeuls`, `FisherLsd`, etc.), `MultipleRowTTests`, `NormalQq` tab, Analysis Checklists, and `D1` R `4.5.2` edge matrix (all landed ✅) | [statistics-implementation.md](statistics-implementation.md) | `G3`, `G1` | — |
| **3. Fit curves** | robust/ROUT, shared globals, `PerDatasetConstant`/`ColumnTitleConstant`, profile-likelihood & open CIs, AICc/*F*/`LRT`, log-grid bands, `Residuals` & `CovarianceMatrix` tabs, homoscedasticity/AD/KS, conditional filters, centered polys/semi-log/log-log/probit, and 5 receptor-binding/kinetics models (all landed ✅) | [curve-fitting-implementation.md](curve-fitting-implementation.md) | `G4`, `G1` | — |
| **4. Graphs** | violin and before–after plots, probit/logit/broken axes, `AdditionalTick` across all 7 scales & broken segments, CI bands, diagnostic plots (`Volcano`, `BlandAltman`, `NormalQqPlot`, `ResidualPlot`), auto-stacked live *p*-value brackets (all landed ✅) | [graphing-and-layouts.md](graphing-and-layouts.md) | `G5` | — |
| **5. Export** | `%PDF-1.7` default + optional TrueType `/FontFile2` embedding (`--ttf-font` / `LUMENCRAFT_FONTS_DIR`) & Core-14/outlined fallback landed ✅; spot-check in two independent external GUI PDF/SVG viewers open | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) | `G5` | 1–2 |
| **6. Headless** | nothing beyond alpha | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) | — | — |
| **Desktop & WASM shell** | inspectors, `Import & Paste Special`, `NavigatorDragState` (`sheet.reorder`), `PortfolioGalleryState`, `F6`/`Shift+F6` pane cycling, `InfoSheet`/`FloatingNote` state + command builders, physical rulers, and `ShortcutRegistry` implemented in `UiApp` (`59 passed`) ✅; native OS pointer/inline-edit event binding in `eframe`/`wgpu` host open | [ui-parity.md](ui-parity.md) | `G6` | 5–8 |
| **Added remaining to Core** | | | | **6–10** |
| **Cumulative remaining (Alpha + Core)** | | | | **21–35** |

---

## Beta gate

LumenCraft enters **beta** when the Core-workflow gate holds **and** the remaining edge gaps below (`2–3 h`, `23–38 h` cumulative) are closed alongside the headless/engine Beta capabilities that are already landed:
1. **Prism 11 Multiple Variables workflows (`G7`, `0 h` remaining):** Calculated Variables (`100k` rows in `13.841 ms` `--release`), role inference/locking, level ordering, Multifactor ($N$-way) ANOVA, `TwoGroupSpec` MV grouping/response column, `MultivarExtractionSpec` Cartesian splits in `fit`, MV `KaplanMeier`, and MV Bubble/Hull/Ellipse/CategoricalSummary plots are landed and verified (`parity_g3_g7`, `parity_wave3_workflows`, `e2e_workflows`).
2. **Survival, Contingency, ROC & Multivariate (`G8`, `0 h` parity remaining):** Kaplan–Meier, Cox PH (`BaselineSurvival` & `LogMinusLog` tabs), Contingency CIs + `BarnardExact`, `BlandAltman`, `PassingBablok` + CUSUM, `MultipleImputation` (MICE PMM + Rubin pooling), ROC, GLMs, PCA, and clustering are wired end to end (`stats` `66 passed`, `engine` `56 passed`; Koopman fallback row tracked in [`gaps.md` › Known defects](gaps.md#known-defects)).
3. **Custom Equation DSL & ODEs (`G9`, `0 h` parity) + Multi-panel Page Layouts + Style Cloning (`G11`, `2–3 h` remaining):** Custom bytecode/ODE DSL (`[InitialValues]`, `[Transforms]`, `<A>`..`<Z>`/`<0>`..`<9999>`, `dsl_fuzz_props` `16`, `equations_and_dsl_blocks` `9`), `LayoutSheet` composer, `applyStyleFrom`, TIFF/LZW, `ClipboardBundle`, `PrintJobSpec`, and subprocess OS pasteboard/print adapters are landed; remaining `2–3 h` covers native binary vector OS clipboard (`NSPasteboard` PDF / Win32 EMF / browser Clipboard API), interactive OS print-dialog submission, and live WYSIWYG layout canvas drag-snap inside `G6` host windowing.
4. **Clean-room `.prism`/`.pzfx` import (`G10`, `0 h` parity) & performance budgets (`G12`, `0 h` parity):** `.prism`/`.pzfx` import (`corrupt_archive_fuzz` `18`, `interop_and_export` `4`, `recovery_and_interop_parity` `7`), `1M`-cell `S1` grid, 7 fuzz/prop suites, and **96-curve 4PL optimization** (now **`22.669 ms`** `fit_ms` / **`26.25 ms`** `total_ms` in `--release` `lumencraft-cli perf --json`, well below the `50.0 ms` budget) are landed and verified; see [`gaps.md` › Known defects](gaps.md#known-defects) for remaining unestimated defect fixes (`Session::execute` clone/undo, `estimate_table_cells`, raster pixel cap, `dsl.rs` chain depth, linked-file read error).

---

## Execution order

Wave 1 and Wave 2 were built layer by layer, following the stack in [`architecture.md`](architecture.md). From here the order changes: **prove one path through every layer, retire the two largest technical risks, then widen one gate at a time.** Building further library breadth before the engine exists adds work that nothing can exercise and that passes no gate.

| Step | What | Crates | Exit criterion | Status |
|---:|---|---|---|---|
| 1 | **Wave 1** — `L0`–`L1` foundations | `math`, `palette`, `doc`, `testkit` | merged, CI green | ✅ done |
| 2 | **Wave 2** — `L2`–`L3` libraries | `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf` | library suites & `D1`/StRD oracles passing | ✅ done (`stats` 66, `fit` 82, `format` 43, visual `90` default & `--no-default-features`) |
| 3 | **Slice 0** — one vertical slice | `engine`, `chart`, `svg`, `format`, `lumencraft-cli` | the [slice](#slice-0-vertical-slice) runs from CLI and reopens bit-identically | ✅ done (`lumencraft-cli` `16 passed`, `Eckerle4` & `Rat43`) |
| 3 | **Risk spikes `S1`, `S2`** (parallel with Slice 0) | `tools`, `ui-egui`, `apps/*`; `text`, `pdf` | each [spike](#risk-spikes) ends in a written go or redesign decision | ✅ done — **GO** (`S1` in [`ui-parity.md`](ui-parity.md), `S2` in [`graphing-and-layouts.md`](graphing-and-layouts.md)) |
| 4 | **Wave 3** — engine and commands | `tools`, `engine` | reactive DAG, undo/redo & `CommandSpec` tests pass | ✅ done (`tools` 21, `engine` 56 incl. `75`-cmd sweep, `11` E2E & `8` Wave-3 parity) |
| 5 | **Wave 4** — shell and automation for Alpha scope | `ui-egui`, `mcp`, `apps/*` | **Alpha gate** passes on desktop + WASM + `--control` | in progress (`mcp` 25, `cli` 16, `ui-egui` 59, `lumencraft` 8, `web` 1 ✅; live host OS `eframe`/`wgpu` & `wasm32` UI host open) |
| 6 | **Widen to Core workflows**, one workflow at a time through all layers | all | **Core-workflow gate** passes | mostly landed (library/engine/headless ✅; `G5` viewer QA & `G6` OS pointer/inline-edit event loops open) |
| 7 | **Beta scope**, one gap at a time through all layers | all | **Beta gate** passes | mostly landed (`G7`–`G10`, `G12` `<50ms` & `G14` ✅; `G11` native OS clipboard/print dialog & canvas drag-snap open) |
| 8 | Localization, hardware/print hardening, release polish | `ui-egui`, `text`, `format`, `packaging/` | `1.0` | partially landed (`16`×`167` = `2,672` i18n + RTL + subprocess OS adapters ✅; `G13` OS CJK/RTL desktop font QA & `M7` hardening open) |

**The engine is the critical path.** It is a single crate, so Wave 3 cannot be split across agents the way Wave 2 was, and every gate waits on it. Do not start Core or Beta library work while engine work for the current gate is open.

**Wave 2 scope freeze.** Core- and Beta-scope library work that is already in flight (for example survival, ROC, multivariate, the equation DSL, and legacy import) is finished and landed as "library verified"; it is not thrown away. No further Core- or Beta-scope library work starts until the Alpha gate passes.

### Slice 0: vertical slice

**Status: ✅ DONE (`apps/lumencraft-cli/tests/slice0_vertical_gate.rs`, `3 passed`; `lumencraft-cli` `16 passed` total).** One narrow path through every layer, driven from `lumencraft-cli` with no UI:

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
| **`S1` Spreadsheet grid** | Can the virtualized two-tier subcolumn grid hold 60 fps at one million cells, with range selection, paste, and exclusion marking, on desktop **and** in the browser? | **GO (with WASM sysroot note):** at `1_000 × 1_000` (`1,000,000` cells), `lumencraft-tools` measured `p50 = 0.0390 ms`, `p95 = 0.0432 ms`, `max = 0.0612 ms` (`120` frames, tail-jump `0.0150 ms`); `lumencraft-ui-egui` measured `p50 = 0.061 ms`, `p95 = 0.507 ms`, `max = 7.795 ms`, `Select All` + batched `table.excludeCells` `0.044 ms` (`< 16.67 ms` budget). Engine/UI shell verified headlessly (`tools` `21`, `ui-egui` `59`); native OS `eframe`/`wgpu` windowing & `wasm32` sysroot browser build tracked in `G6`. | [`ui-parity.md`](ui-parity.md#risk-spike-s1-decision-spreadsheet-grid-at-60-fps-go) |
| **`S2` Text and PDF** | Can `text` and `pdf` produce a publication-quality figure (Greek letters, superscripts and subscripts, rotated axis titles, embedded or substituted fonts) with `BUNDLED_FONTS` empty? | **GO:** direct pure-Rust `%PDF-1.7` default writer (with `--pdf-version 1.4` and optional `/FontFile2` TrueType embedding from external `.ttf` via `--ttf-font` / `LUMENCRAFT_FONTS_DIR`) + Core-14 Type1 Helvetica/Symbol selectable text & outlined glyph-path fallback generated 2-page PDFs (`FlateDecode`) under `--no-default-features` (`BUNDLED_FONTS` empty) as well as default features (`90` visual tests passing under both default and `--no-default-features` across `chart` `35`, `text` `25`, `pdf` `11`, `render` `8`, `svg` `11`). Verified structurally in tests; interactive GUI viewer inspection remains in Core `G5`. | [`graphing-and-layouts.md` §7](graphing-and-layouts.md#7-risk-spike-s2-decision-text--pdf-without-bundled-fonts-go) |

### Gap status

Each gap has two statuses. **Library verified** means the crate is merged with tests against a named [oracle](#verification-oracles) or round-trip properties. **End to end** means it is also command-wired and reachable from `lumencraft-engine`, CLI, MCP, and/or the UI shell (`UiApp`); live host OS desktop/`wgpu` windowing status is noted explicitly for `G6` and `G14`.

| Gap | Gates | Library verified | End to end |
|---|:---:|---|---|
| `G1` | Core (`D1` edge matrix) | ✅ NIST StRD (`stats` `4`, `fit` `1`) + `D1` oracles & edge matrices (`stats` `20` + `10`, `fit` `18` + `8`) | n/a (test infrastructure ✅; `0 h`) |
| `G2` | Alpha, Core | ✅ (`palette` `13`, `doc` `25`, `format` `43` incl. WAL `CrashRecoveryJournal` + `MIGRATION_STEPS`) | ✅ CLI/MCP/`UiApp` (`engine` `56`, `slice0` `3` + WAL replay ✅; known `cmd_data` defect tracked) |
| `G3` | Alpha, Core | ✅ (`stats` `66` w/ `NormalQq`, `NewmanKeuls`/`FisherLsd`, `MultipleRowTTests`) | ✅ CLI/MCP/`UiApp` (`parity_g3_g7`, `e2e_workflows`, `parity_wave3_workflows` ✅; `0 h`) |
| `G4` | Alpha, Core | ✅ (`fit` `82` w/ log bands, `LRT`, 5 binding eqs, centered/semi-log/log-log/probit, `Residuals`/`CovarianceMatrix` tabs) | ✅ CLI/MCP/`UiApp` (`Slice 0`, `perf_96_and_fit_parity`, `parity_wave3_workflows` ✅; `0 h`) |
| `G5` | Alpha, Core | ✅ **S2 GO** (`90` visual tests default & `--no-default-features`; `%PDF-1.7` + `/FontFile2` + `AdditionalTick` ✅) | ✅ CLI/MCP/`UiApp` (`ticks_and_diag_parity`, `pdf17_and_perf_gate` ✅; external GUI viewer QA `1–2 h`) |
| `G6` | Alpha, Core | n/a (shell; **S1 GO**, `UiApp` + `NavigatorDragState` + `PortfolioGalleryState` + `F6` + rulers + `ShortcutRegistry` ✅) | partial (`UiApp` + software compositor `59`, `tools` `21`; live OS `eframe`/`wgpu` & `wasm32` host + OS pointer polish `20–33 h`) |
| `G7` | Beta | ✅ (`transform` `15`, `stats`/`fit`/`doc`/`chart`; MV `TwoGroupSpec`, Cartesian MV fit, MV `KaplanMeier` ✅) | ✅ CLI/MCP/`UiApp` (`parity_g3_g7`, `parity_wave3_workflows` ✅; `0 h`) |
| `G8` | Beta | ✅ (`stats` `66` w/ `BlandAltman`, `BarnardExact`, `PassingBablok`, `MI`, Cox `BaselineSurvival`/`LogMinusLog` tabs) | ✅ CLI/MCP/`UiApp` (`parity_g8_barnard_pb_mi`, `parity_wave3_workflows` ✅; known Koopman defect tracked) |
| `G9` | Alpha, Beta | ✅ (`dsl_fuzz_props` `16`, `equations_and_dsl_blocks` `9`: `[InitialValues]`, `[Transforms]`, `<A>`..`<Z>`/`<0>`..`<9999>`) | ✅ CLI/MCP/`UiApp` (`e2e_workflows`, `parity_wave3_workflows` ✅; known `dsl.rs` defect tracked) |
| `G10` | Beta | ✅ (`corrupt_archive_fuzz` `18`, `interop_and_export` `4`, `recovery_and_interop_parity` `7`) | ✅ CLI/MCP/`UiApp` (known `estimate_table_cells` defect tracked; `0 h`) |
| `G11` | Beta | ✅ (`LayoutSheet`, Wand, LZW TIFF, `ClipboardBundle`, `PrintJobSpec`, subprocess OS adapters) | partial (headless & subprocess adapters ✅; native binary OS clipboard/print dialog & canvas drag-snap `2–3 h`) |
| `G12` | Beta | n/a (`perf` + 7 fuzz/prop suites) | ✅ `lumencraft-cli perf --json` (`--release`: **96-curve 4PL `22.669 ms` fit / `26.25 ms` total** `<= 50 ms`, `100k` CalcVar `13.841 ms` `< 250 ms`; known `Session::execute` defect tracked) |
| `G13` | 1.0 | ✅ `16`×`167` = `2,672` translations + RTL `ar`/`he` BiDi/UI mirroring, CSV/decimal presets, `font_diagnostics` | partial (`i18n-report` & RTL compositor ✅; live OS CJK/RTL desktop font QA & OS pasteboard check `2–4 h`) |
| `G14` | Alpha, Beta | ✅ (`ui-commands --check` `17` UI cmds, `mcp` `25 passed`) | ✅ CLI/MCP/`--control` (`cli` `16`, `mcp` `25`, `lumencraft` `8`, `web` `1`, `75` engine cmds; `0 h`) |

---

## Verification oracles

A result is "verified" only against a named public reference. NIST StRD covers four areas and textbook worked examples cover a few cases per method elsewhere. The remaining methods are verified against reference values generated with open-source statistical software, which decision `D1` below allows. A method does not count as library verified until its tests use the reference its row names.

| Area | Reference today | Coverage |
|---|---|---|
| Univariate summary statistics | NIST StRD (9 datasets) | good |
| One-way ANOVA | NIST StRD (11 datasets) | good |
| Linear regression | NIST StRD (11 datasets) | good |
| Nonlinear regression estimates and standard errors | NIST StRD (27 datasets) | good |
| *t*-tests, two-way ANOVA, Tukey, Dunnett, Bonferroni, Holm–Šídák | textbook worked examples (Zar, Montgomery); published critical-value tables; R `4.5.2` | good |
| Nonparametric tests, exact small-sample distributions, Barnard, Passing–Bablok, MI | textbook worked examples (Zar, Rubin, Barnard & Rubin) + Python `3.13` / R `4.5.2` exact fixtures (`parity_g8_barnard_pb_mi`) | good |
| Kaplan–Meier, log-rank, Cox PH | textbook worked examples (Klein & Moeschberger, Hosmer & Lemeshow) + `d1_edge_matrix` (`survival 3.8`) | good |
| Contingency intervals, ROC | worked examples in the original method papers + `d1_reference_oracles` | good |
| Repeated-measures ANOVA, REML mixed models, three-way ANOVA | R `4.5.2` (`stats`, `nlme 3.1`) via `d1_reference_oracles.rs` (`20`) & `d1_edge_matrix.rs` (`10`) | good (unbalanced 3-way Type III, missing REML, ties) |
| Profile-likelihood CIs, robust regression, ROUT, 10-decade weighted fits | analytical special cases + `d1_nist_and_pharma_oracles.rs` (`18`) & `crates/fit/tests/d1_edge_matrix.rs` (`8`) | good (flat plateaus, active bounds, ROUT, 10-decade weights) |
| GLMs, PCA, clustering | R `4.5.2` references (`d1_reference_oracles.rs` + `d1_edge_matrix.rs`) | good |

Rows marked good above should continue adding regression cases alongside any future defect fixes (for example Koopman fallback edge cases in [`gaps.md` › Known defects](gaps.md#known-defects)).

**Decision `D1` (maintainer, 2026-11-13): allowed.** Reference values produced by running open-source statistical software (for example R or SciPy) may be committed as test fixtures. The limits are in the clean-room section of [`AGENTS.md`](../AGENTS.md): run the software only as a black box and never read or port its source; never use the target application this way; use synthetic, NIST, or cited published input data; record the software, version, exact call, and date beside the values; and keep the software out of the build, test, and CI dependencies. `D1` oracle test suites are verified in [`crates/stats/tests/d1_reference_oracles.rs`](../crates/stats/tests/d1_reference_oracles.rs) (`20 passed`), [`crates/stats/tests/d1_edge_matrix.rs`](../crates/stats/tests/d1_edge_matrix.rs) (`10 passed`), [`crates/stats/tests/parity_g8_barnard_pb_mi.rs`](../crates/stats/tests/parity_g8_barnard_pb_mi.rs) (`10 passed`), [`crates/fit/tests/d1_nist_and_pharma_oracles.rs`](../crates/fit/tests/d1_nist_and_pharma_oracles.rs) (`18 passed`), and [`crates/fit/tests/d1_edge_matrix.rs`](../crates/fit/tests/d1_edge_matrix.rs) (`8 passed`).

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
| **M1** | Numerical primitives (`lumencraft-math`), NIST StRD harness & typed data-table model (`lumencraft-doc`, `lumencraft-format`) | `G2` (WAL recovery & migrations) | ✅ parity complete (`math` `50`, `testkit` `6`, `palette` `13`, `doc` `25`, `format` `43`) | — |
| **M2** | Reactive DAG & undo/redo (`lumencraft-engine`), log/normalize transforms, core biostatistics + REML + multiplicity (`lumencraft-stats`) with StRD wiring | `G1`, `G2`, `G3`, `G9` | ✅ parity complete (`stats` `66`, `transform` `15`, `engine` `56`) | — |
| **M3** | Nonlinear regression solver, profile-likelihood CIs & built-in equation library (`lumencraft-fit`) | `G1`, `G4` | ✅ parity complete (`fit` `82`, `d1_edge_matrix` `8`, `perf_96` `< 50 ms`) | — |
| **M4** | Publication chart scene compiler (`lumencraft-chart`), live brackets & SVG/PDF/PNG export | `G5` | **S2 GO** (`90` visual tests, `%PDF-1.7`, `/FontFile2`, `AdditionalTick` landed ✅; external viewer QA open) | 1–2 |
| **M5** | Interactive desktop & WASM shell (`lumencraft-ui-egui`, Navigator, subcolumn grid, inspectors) & Alpha automation — **Core-workflow gate** | `G6`, `G14` (CLI/MCP/`--control` ✅) | **S1 GO**, `UiApp` + UX parity (`59`), `tools` (`21`), `cli` (`16`), `mcp` (`25`), `lumencraft` (`8`), `web` (`1`); live OS `eframe`/`wgpu` & `wasm32` host + pointer/inline-edit event loops open | 20–33 |
| **M6** | Prism 11 Multiple Variables, Calculated Variables, Multifactor ANOVA, Survival/Cox/GLMs/PCA, custom ODE DSL, Page Layouts & `.prism`/`.pzfx` import — **Beta** | `G7`–`G12` | `G7`–`G10`, `G12` (`96-curve` `22.669 ms` fit / `26.25 ms` total) landed ✅; `G11` native binary clipboard/print dialog & canvas drag-snap open | 2–3 |
| **M7** | Localization (`G13`), cross-OS hardware/print hardening & 1.0 release polish | `G13` (`2–4`) plus `12–18` OS GPU/trackpad/installer hardening not tracked as a gap | `16`×`167` = `2,672` i18n + RTL `ar`/`he` + subprocess adapters ✅; OS CJK/RTL desktop font QA & cross-OS `wgpu`/trackpad/installer hardening open | 14–22 |
| **M8+** | Post-parity, non-breaking quality-of-life enhancements ([`qol-beyond-parity.md`](qol-beyond-parity.md): `QOL-1`–`QOL-11`) | post-`1.0` | scoped (deferred) | 82–136 |

**Cumulative remaining, one agent:** Alpha (`G6` live OS `eframe`/`wgpu` & `wasm32` UI host `15–25 h`; headless Alpha already passes) · Core workflows (`M1`–`M5`) `21–35 h` · Beta (`M1`–`M6`) `23–38 h` · `1.0` (`M1`–`M7`) **`37–60 h`** (excluding [`gaps.md` › Known defects](gaps.md#known-defects)).
