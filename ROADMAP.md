# LumenCraft Roadmap

**Stage: Wave 4 in progress (headless Alpha, Core & Beta engine/CLI/MCP/`--control`/`UiApp` parity complete; awaiting live OS desktop/`wgpu`/`wasm32` shell hosting for UI Alpha)** · next: Alpha desktop/WASM gate (`eframe`/`egui`/`wgpu` live OS windowing), ~`15–25 h` away

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-14 · **Change:** Wave 1–4 headless parity closed (`G1`–`G4`, `G7`–`G10`, `G12`, `G14`), `< 50 ms` `96`-curve 4PL fast path (`22.669 ms` fit / `26.25 ms` total `--release`), `%PDF-1.7` + optional `/FontFile2` SFNT embedding, WAL crash-recovery journal, `16` locales × `167` keys (`2,672` translations + RTL `ar`/`he` BiDi/UI mirroring), `17` `UI_COMMANDS`, and `602` workspace tests + `4` doc tests passing; remaining roadmap narrowed to `37–60 h` (`~15–25 h` to desktop Alpha) · **Target:** GraphPad Prism 11 (11.1.x)

LumenCraft is a clean-room, open-source, pure-Rust reimplementation of the scientific data-table, biostatistics, nonlinear regression (curve fitting), and publication graphing workflow. It targets macOS, Windows, Linux, FreeBSD, and WebAssembly (`WASM`), and agents can drive all of it headlessly over MCP, the CLI, and a loopback JSONL control channel. This page is the summary; the evidence is in [`docs/target-app-parity.md`](docs/target-app-parity.md), the ranked work list is in [`docs/gaps.md`](docs/gaps.md), and the subsystem implementation guidelines live in [`docs/`](docs/).

**Where things stand:** Wave 1–4 engine, statistical (`lumencraft-stats` `66 passed`), fitting (`lumencraft-fit` `82 passed`), transform (`lumencraft-transform` `15 passed`), visual (`90 passed` default & `--no-default-features` across `chart`/`text`/`svg`/`pdf`/`render` with `%PDF-1.7` + optional `/FontFile2` SFNT embedding), archive/WAL (`lumencraft-format` `43 passed`), reactive engine (`lumencraft-engine` `56 passed` covering all `75` engine commands), `lumencraft-mcp` (`25 passed`), `lumencraft-cli` (`16 passed` incl. `Slice 0` & `< 50 ms` `96`-curve `perf --json` gate), `apps/lumencraft` (`8 passed` incl. `--control` loopback, `--portable-dir`/`portable.txt`, and OS subprocess pasteboard/print adapters), and `apps/lumencraft-web` (`1 passed`) are landed (**`602` unit/integration + `4` doc tests passing**, `0` failed). `lumencraft-ui-egui` (`59 passed`) ships a pure-Rust `UiApp` state shell + software RGBA/PNG compositor (`render_workspace_rgba`/`render_workspace_frame_png`) with all **`16` locales (`2,672` translations + RTL `ar`/`he` mirroring)** and `17` `UI_COMMANDS`. Remaining work to `1.0` (`37–60 h` single agent) centers on live OS desktop `eframe`/`egui`/`wgpu` & `wasm32` UI hosting (`G6`, `15–25 h` Alpha + `5–8 h` Core pointer/inline GUI polish), live external PDF viewer QA with external `.ttf` fonts (`G5`, `1–2 h`), native OS vector clipboard/print dialogs (`G11`, `2–3 h`), Tier-2/CJK desktop QA (`G13`, `2–4 h`), and `M7` cross-OS release hardening (`12–18 h`). See [execution order](docs/roadmap.md#execution-order).

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** (table archetypes, statistical tests, equations, graph types, exports) | **~97%** | `8` table archetypes, stats/ANOVA/REML/survival/Cox/ROC/GLM/PCA, Newman–Keuls/LSD/Row *t*-tests/Barnard/Bland–Altman/Passing–Bablok/MICE, 4PL+binding+DSL+ODE, `8` graph families+layouts, SVG/`%PDF-1.7`+`/FontFile2`/PNG/TIFF, `.lumencraft`/WAL/CSV/`.prism`/`.pzfx`, `75` engine + `17` UI commands & `16`-locale i18n landed |
| **Feature depth** (weighted by scientific use in engine, CLI/MCP & `UiApp` state shell) | **~95%** | estimated ([methodology](docs/target-app-parity.md#methodology)); interactive desktop GPU GUI awaits live OS/`eframe`/`wgpu`/`wasm32` event loop |
| **To desktop Alpha** (Essentials UI gate) | **~15–25 h** one agent · ~8–12 h parallel | headless Alpha workflows already pass via CLI/MCP/`--control`/`UiApp`; `G6` OS/`wgpu`/`wasm32` UI host ([Alpha gate](docs/roadmap.md#alpha-gate)) |
| **To core workflows** | **~21–35 h** one agent · ~10–16 h parallel | Alpha remaining (`G6` Alpha `15–25 h`) plus `G6` Core pointer/inline GUI polish (`5–8 h`) & `G5` PDF viewer TTF QA (`1–2 h`) ([Core-workflow gate](docs/roadmap.md#core-workflow-gate)) |
| **To beta** | **~23–38 h** one agent · ~11–18 h parallel | Core plus `G11` native OS vector clipboard & interactive print dialogs (`2–3 h`) |
| **To full parity (`1.0`)** | **~37–60 h** one agent · ~15–24 h parallel | Beta plus `G13` Tier-2/CJK desktop QA (`2–4 h`) & `M7` cross-OS hardware/GPU/packaging hardening (`12–18 h`; [`docs/gaps.md`](docs/gaps.md)) |

Hours are implementation estimates and exclude human review, which sets the real pace ([landing rules](docs/roadmap.md#landing-rules)).

**Readiness by audience** (estimated; headless CLI/MCP workflows already run end to end, while interactive GUI readiness depends on live OS windowing/`wgpu` integration in `G6`):

| Audience | Ready % | Agent hours to ~95% (one agent) | With 4–6 agents | Work that dominates |
|---|---:|---:|---:|---|
| **Full Prism 11 (Pro / Enterprise)** (decides `1.0`) | **~82%** | **~37–60 h** | ~15–24 h | live OS `eframe`/`wgpu`/`wasm32` UI host (`G6`), live PDF TTF viewer check (`G5`), native OS vector pasteboard/print dialogs (`G11`), Tier-2/CJK OS QA (`G13`), `M7` cross-OS hardware/GPU hardening |
| **Mainstream wet-lab researcher** | **~85%** | **~21–35 h** | ~10–16 h | live OS `eframe`/`wgpu` desktop windowing + pointer drag/inline editing (`G6`), live PDF 1.7 TTF viewer spot-check (`G5`) |
| **Essentials / student biostatistics user** (decides Alpha) | **~80%** *(CLI/MCP/`--control` ~98%+)* | **~15–25 h** | ~8–12 h | live OS `eframe`/`wgpu` + `wasm32` UI hosting over `UiApp` (`G6`, `15–25 h`) |

The target application binary was **not** inspected or executed: the clean-room rule in [`AGENTS.md`](AGENTS.md) forbids it, so every requirement is derived from public documentation, peer-reviewed biostatistics literature, and public reference datasets (NIST StRD).

## By dimension

Dimensions overlap (UI, file-format and hardware work is also counted inside feature areas), so this column does not sum to the headline totals.

| Dimension | % | One agent (h) | Doc |
|---|---:|---:|---|
| Features (depth — engine/CLI/`UiApp`) | 95% | 21–35 | [target-app-parity.md](docs/target-app-parity.md) |
| UI/UX & spreadsheet fidelity | 70% | 20–33 | [ui-parity.md](docs/ui-parity.md) |
| File formats & figure export | 95% | 1–2 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (GPU, HiDPI, clipboard, print) | 45% | 14–21 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization & number formatting | 95% | 2–4 | [localization-parity.md](docs/localization-parity.md) |
| Numerical accuracy & corpus | 100% | — | [gaps.md › G1](docs/gaps.md#g1-numerical-foundations-and-nist-strd-oracle) |
| Performance (1M-cell grid, batch fits) | 100% | — | [gaps.md › G12](docs/gaps.md#g12-performance-budgets-large-table-virtualization-and-fuzzing) |
| Platforms & packaging | 70% | 8–12 | [releasing.md](docs/releasing.md) |
| Agent & headless automation | 100% (beyond target on MCP) | — | [scripting-and-automation.md](docs/scripting-and-automation.md), [mcp.md](docs/mcp.md) |

## Features

| Area | Weight | % | One agent (h) | Implementation guide |
|---|---:|---:|---:|---|
| Typed data tables (8 archetypes), exclusions & Info constants | 14% | 98% | 0 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Calculated variables, formulas & derived transforms | 8% | 98% | 0 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Descriptive stats, normality & Grubbs/ROUT outliers | 8% | 98% | 0 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Two-group, one-/two-/three-way ANOVA, RM, REML & multiplicity | 16% | 96% | 0 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Nonlinear regression solver, profile CIs, global fits & AICc/*F* | 14% | 96% | 0 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Built-in equation library & custom ODE/algebraic DSL | 10% | 94% | 0 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Contingency, Kaplan–Meier/Cox survival, ROC & multivariate/PCA | 10% | 98% | 0 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Publication graphing, broken/log/probit axes & significance brackets | 10% | 96% | 0 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Multi-panel Page Layouts, SVG/PDF/TIFF export & style cloning | 5% | 94% | 1–2 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Project Navigator, spreadsheet UX, `.lumencraft`/`.prism`/`.pzfx` & automation | 5% | 74% | 20–33 | [ui-parity.md](docs/ui-parity.md), [scripting-and-automation.md](docs/scripting-and-automation.md) |

Weights, scoring rubric, and missing items per area: [`docs/target-app-parity.md`](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | Translated |
|---|---|---|---:|
| English | `en` | complete baseline catalog (`cargo xtask i18n-report`) | 100% (`167` keys) |
| Tier-1 scientific locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`) | — | all 8 Tier-1 translations & decimal/CSV presets verified (`G13`) | 100% (`9` × `167` = `1,503` keys, `0` missing) |
| Tier-2 & RTL locales (`it`, `nl`, `pl`, `ru`, `tr`, `ar`, `he`) | — | all 7 Tier-2/RTL translations + BiDi UI layout mirroring (`G13`; live desktop CJK/RTL QA pending) | 100% (`16` × `167` = `2,672` keys, `0` missing) |

## Upcoming

In order; details and exit criteria are in the [execution order](docs/roadmap.md#execution-order), per-gap scope in [`docs/gaps.md`](docs/gaps.md).

1. **Close desktop Alpha (`G6` Alpha, `15–25 h`):** wire live OS `eframe`/`egui`/`wgpu` desktop windowing & `wasm32` sysroot verification over `UiApp`.
2. **Finish Core UI & PDF viewer QA (`G5` + `G6` Core, `6–10 h`):** spot-check external PDF viewers with embedded `.ttf` fonts (`G5`, `1–2 h`), and complete interactive pointer drag/inline GUI editing over live host rendering (`G6` Core, `5–8 h`).
3. **Finish Beta OS integrations (`G11`, `2–3 h`):** upgrade subprocess text pasteboard/print adapters (`pbcopy`/`wl-copy`/`xclip`/`clip`/`lp`/`lpr`) to native OS vector/EMF/PDF clipboard formats and interactive OS print preview/dialogs (`G11`).
4. **Release hardening & Tier-2/RTL desktop QA (`G13` + `M7`, `14–22 h`):** live desktop CJK/RTL window verification (`G13`, `2–4 h`), cross-OS HiDPI/trackpad/installer signing & release smoke testing (`M7`, `12–18 h` to `1.0`).

**Decision `D1` (2026-11-13):** reference values produced by open-source statistical software may be used as test fixtures, within the limits in [`AGENTS.md`](AGENTS.md). Comprehensive `D1` R 4.5.2 & NIST suites are in place (`crates/stats/tests/d1_reference_oracles.rs` `20 passed`, `crates/stats/tests/d1_edge_matrix.rs` `10 passed`, `crates/fit/tests/d1_nist_and_pharma_oracles.rs` `18 passed`, `crates/fit/tests/d1_edge_matrix.rs` `8 passed`); see [verification oracles](docs/roadmap.md#verification-oracles).

## Progress log

| Date | Milestone | Summary |
|---|---|---|
| 2026-11-14 | Wave 4 / Parity & Perf | Closed headless parity gaps `G1`–`G4`, `G7`–`G10`, `G12`, `G14` across the 21-member workspace (`602` unit/integration + `4` doc tests passing, `0` failed; `149` passing under `--no-default-features` visual+UI): `stats` (`66 passed` incl. `d1_edge_matrix` `10`, `parity_g8_barnard_pb_mi` `10`, `parity_g3_g7` `10` — Normal Q–Q, Multiple Row *t*-tests+FDR/Volcano, Newman–Keuls, Fisher LSD, MV two-group/KM, Cox survival plots, Bland–Altman, Barnard's exact, Passing–Bablok+CUSUM, MICE Multiple Imputation), `fit` (`82 passed` incl. `d1_edge_matrix` `8` & `< 50 ms` 96-curve 4PL fast path at `22.669 ms` fit / `26.25 ms` total `--release`, 5 binding/kinetics models, `[InitialValues]`/`[Transforms]`/`<A>`..`<Z>`/`<0>`..`<9999>`, LRT/AD/KS/homoscedasticity/`Residuals`/`CovarianceMatrix`/log-grid bands), `format` (`43 passed` incl. WAL `CrashRecoveryJournal`, `MIGRATION_STEPS`, circular hook sanitization), `chart`/`pdf`/`text`/`svg`/`render` (`90 passed` w/ `%PDF-1.7` + optional `/FontFile2` SFNT embedding, `AdditionalTick` across all 7 scales & broken segments, BiDi RTL), `ui-egui` (`59 passed` w/ `16` locales × `167` keys = `2,672` translations, `ar`/`he` RTL mirroring, `NavigatorDragState`, `PortfolioGalleryState`, `F6` pane cycle, rulers, `ShortcutRegistry`, `17` `UI_COMMANDS`), `apps/lumencraft` (`8 passed` w/ `--portable-dir`/`portable.txt` & subprocess OS pasteboard/print adapters), and `xtask` (`26 passed`). Remaining work (`37–60 h`) is live OS `eframe`/`wgpu`/`wasm32` UI hosting (`G6`), PDF viewer TTF QA (`G5`), native vector clipboard/print dialogs (`G11`), Tier-2/CJK desktop QA (`G13`), and `M7` release hardening. |
| 2026-11-13 | Wave 3 (`L4`–`L5`) | Landed `lumencraft-tools` (`15 passed`, **Risk Spike S1** two-tier grid `1M` cells `p50 = 0.0390 ms`, `max = 0.0612 ms`, tail-jump `0.0150 ms`) and `lumencraft-engine` (`48 passed`: topological reactive DAG, undo/redo journal, `Paste Link`, `command_sweep_props` covering all `73` commands, and `11` end-to-end workflows). |
| 2026-11-13 | Wave 2 (`L2`–`L3`) + Spike `S2` | Landed `lumencraft-stats` (`36 passed` w/ `20` `d1_reference_oracles` & `4` `nist_strd`), `lumencraft-fit` (`55 passed` w/ `18` `d1_nist_and_pharma_oracles` & `16` `dsl_fuzz_props`), `lumencraft-transform` (`=` Calculated Variables & 7 transform suites), `lumencraft-format` (`36 passed` w/ `.lumencraft`, CSV/TSV, clean-room `.prism`/`.pzfx`, `18` `corrupt_archive_fuzz`), and visual stack `text`/`chart`/`svg`/`pdf`/`render` (`73` default / `32` `--no-default-features` tests: **Risk Spike S2 GO** pure-Rust `%PDF-1.4` Core-14 Helvetica/Symbol writer in `0.293–0.395 ms` with `BUNDLED_FONTS` empty). |
| 2026-11-13 | G1 | Landed `lumencraft-math` (`L0`: Neumaier compensated summation & shifted two-pass moments, cancellation-free `erf`/`erfc`/`ln_gamma` and regularized incomplete beta/gamma, tail-safe normal / Student *t* (real df) / *F* / $\chi^2$ / noncentral *t*/*F*/$\chi^2$ CIs / Copenhaver–Holland Studentized range $q(k,\nu)$ / exact multivariate Dunnett *t* integrals, column-pivoted Householder QR with Björck augmented refinement, Golub–Kahan SVD, Cholesky, symmetric eigensolver, quadrature, Brent root-finding, `DeterministicRng`, plus unified `ConfidenceLevel`, `PValueFormat`, and guarded `AxisScale`/`LogitDomain`) alongside `lumencraft-testkit` (106-bit `DoubleDouble` oracle, LRE/ULP metrics, FIPS 180-4 SHA-256 verifier, clean-room NIST StRD `.dat` parsers, and offline stiffness/collinearity benchmarks in `crates/math/tests/nist_strd.rs`) and `cargo xtask corpus` (`--fetch` / `--fetch-only`, 58-file NIST StRD catalog across 9 Univariate, 11 ANOVA, 11 Linear Regression, and 27 Nonlinear Regression datasets verified offline with `LUMENCRAFT_REQUIRE_CORPUS=1`). |
| 2026-11-13 | G2 (partial) | Landed `lumencraft-palette` (`L0`: sRGB/Oklab/OkLCH/CMYK, WCAG contrast, Okabe–Ito & Paul Tol + custom categorical palettes, bounded Viridis/Cividis/Plasma colormaps, Machado CVD simulation, markers & dash styles) and `lumencraft-doc` (`L1`: all 8 `TableArchetype`s, 8 `SubcolumnMode`s + `SummarizedPoint` moment reconstruction, non-destructive exclusions & `*` tokens, freeze-guarded `DataSheet` (`Y`, `X`, and `Survival.times`) accessors, `FloatingNote`, `InfoSheet` constants & `HookSource`, `LinkedDataRegion` + 4-stage `ImportLinkSpec`, `AnalysisSpec`/`ResultSheet`, `GraphSpec`/`LayoutSpec`, `ProjectDocument` DAG dependencies, and enriched `TransformAnalysisSpec::Simulate` with `x_min`, `x_max`, `parameters`), unified `ConfidenceLevel`/`PValueFormat`/`AxisScale`/`LogitDomain` across `math` and `doc`, added workspace-allowlisted `serde`, `serde_json` (`float_roundtrip`), `flate2` (`rust_backend`), `crc32fast`, and `proptest`, and verified `serde` + `proptest!` round-trips across `math`, `palette`, and `doc`; `crates/format` (`.lumencraft` ZIP/CSV I/O) and `crates/engine` (reactive DAG + undo/redo) remain queued. |
| 2026-11-13 | M0 | Clean-room subsystem implementation guidelines (`data-tables-and-transforms.md`, `statistics-implementation.md`, `curve-fitting-implementation.md`, `graphing-and-layouts.md`, `scripting-and-automation.md`), Prism 11.1 parity checklists, ranked `G1`–`G14` work list, and crate rustdoc wiring completed. |
| 2026-11-13 | M0 | Initialized 21-crate layered Rust 2024 workspace (`L0`–`L7`), `xtask` CI/layer/asset/brand/cleanroom/cursor/i18n/ui-commands gates, original vector/raster app icons, cross-platform `packaging/`, and GitHub Actions workflows. |
