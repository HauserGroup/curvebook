# LumenCraft Roadmap

**Stage: Wave 4 in progress (headless Alpha & Core CLI/MCP/`--control`/engine workflows passing; awaiting live OS desktop/`wgpu`/`wasm32` shell hosting for UI Alpha)** · next: Alpha desktop/WASM gate (`eframe`/`wgpu` live OS windowing), ~`15–25 h` away

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Wave 2 (`L2`–`L3`), Wave 3 (`crates/tools`, `crates/engine`), `Slice 0` (`lumencraft-cli`), Risk Spikes `S1` & `S2` (**GO**), `e2e_workflows` (`11 passed`), Tier-1 i18n (`9` locales × `167` keys = `1,503` translations), `lumencraft-mcp` (`25 passed`), `apps/lumencraft` `--control` (`6 passed`), `lumencraft-web` (`1 passed`), and `lumencraft-ui-egui` `UiApp` state shell + software compositor (`41 passed`) landed; remaining roadmap narrowed to `73–118 h` (`~15–25 h` to desktop Alpha) · **Target:** GraphPad Prism 11 (11.1.x)

LumenCraft is a clean-room, open-source, pure-Rust reimplementation of the scientific data-table, biostatistics, nonlinear regression (curve fitting), and publication graphing workflow. It targets macOS, Windows, Linux, FreeBSD, and WebAssembly (`WASM`), and agents can drive all of it headlessly over MCP, the CLI, and a loopback JSONL control channel. This page is the summary; the evidence is in [`docs/target-app-parity.md`](docs/target-app-parity.md), the ranked work list is in [`docs/gaps.md`](docs/gaps.md), and the subsystem implementation guidelines live in [`docs/`](docs/).

**Where things stand:** Wave 1 (`L0`–`L1`), Wave 2 (`L2`–`L3` libraries across `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`), `Slice 0` (`lumencraft-cli`, `7 passed` including `Eckerle4`/`Rat43` and bit-identical save/reopen/SVG), Risk Spikes `S1` (1M-cell grid `0.0390`/`0.061 ms` p50) and `S2` (`%PDF-1.4` w/ `BUNDLED_FONTS` empty in `0.293–0.395 ms`) (**GO**), Wave 3 (`crates/engine` `48 passed` covering all `73` commands and `11` end-to-end Alpha/Core/Beta workflows), `lumencraft-mcp` (`25 passed`), and `apps/lumencraft` (`6 passed` incl. `control_loopback`) + `apps/lumencraft-web` (`1 passed`) are complete. `lumencraft-ui-egui` (`41 passed`) ships a complete pure-Rust `UiApp` state shell + software RGBA/PNG compositor with all 8 Tier-1 locales. Remaining gaps to `1.0` (`73–118 h` single agent) center on live OS desktop/`wgpu` & `wasm32` UI hosting (`G6`), optimizing 96-curve 4PL batch fitting from measured `16.91 s` to `< 50 ms` (`G12`), expanded `D1` R/SciPy edge fixtures, PDF 1.7 TTF embedding, secondary library options, and cross-OS hardware/print/RTL hardening. See [execution order](docs/roadmap.md#execution-order).

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** (table archetypes, statistical tests, equations, graph types, exports) | **~88%** | `8` table archetypes, stats/ANOVA/REML/survival/ROC/GLM/PCA, 4PL+built-ins+DSL+ODE, `8` graph families+layouts, SVG/`%PDF-1.4`/PNG/TIFF, `.lumencraft`/CSV/`.prism`/`.pzfx`, `73` engine commands & Tier-1 i18n landed; minor enum/model & UI host items remain |
| **Feature depth** (weighted by scientific use in engine, CLI/MCP & `UiApp` state shell) | **~83%** | estimated ([methodology](docs/target-app-parity.md#methodology)); interactive desktop GUI awaits live OS/`wgpu`/`wasm32` event loop |
| **To desktop Alpha** (Essentials UI gate) | **~15–25 h** one agent · ~8–12 h parallel | headless Alpha Workflows 1–6 already pass via CLI/MCP/`--control`/`UiApp`; `G6` OS/`wgpu`/`wasm32` UI host ([Alpha gate](docs/roadmap.md#alpha-gate)) |
| **To core workflows** | **~37–61 h** one agent · ~15–25 h parallel | Alpha remaining plus Core edge items, `PDF 1.7` TTF, `D1` edge fixtures & UI pointer polish ([Core-workflow gate](docs/roadmap.md#core-workflow-gate)) |
| **To beta** | **~53–88 h** one agent · ~20–32 h parallel | Core plus `96-curve` `<50ms` perf opt, live OS clipboard/print & Beta edge items |
| **To full parity (`1.0`)** | **~73–118 h** one agent · ~26–42 h parallel | all gaps plus `G13` Tier-2/RTL & `M7` cross-OS hardware/GPU hardening ([`docs/gaps.md`](docs/gaps.md)) |

Hours are implementation estimates and exclude human review, which sets the real pace ([landing rules](docs/roadmap.md#landing-rules)).

**Readiness by audience** (estimated; headless CLI/MCP workflows already run end to end, while interactive GUI readiness depends on live OS windowing/`wgpu` integration in `G6`):

| Audience | Ready % | Agent hours to ~95% (one agent) | With 4–6 agents | Work that dominates |
|---|---:|---:|---:|---|
| **Full Prism 11 (Pro / Enterprise)** (decides `1.0`) | **~68%** | **~73–118 h** | ~26–42 h | live OS/`wgpu`/`wasm32` UI host (`G6`), `96-curve` `<50ms` optimization (`G12`), live OS print/clipboard (`G11`/`M7`), Tier-2/RTL & CJK OS fallback (`G13`), `D1` & secondary library edge items |
| **Mainstream wet-lab researcher** | **~74%** | **~37–61 h** | ~15–25 h | live OS desktop/`wgpu` windowing + pointer drag/inline editing (`G6`), `NewmanKeuls`/log bands/5 binding eqs (`G3`/`G4`), PDF 1.7 TTF + viewer check (`G5`), expanded `D1` oracles (`G1`) |
| **Essentials / student biostatistics user** (decides Alpha) | **~80%** *(CLI/MCP/`--control` ~95%+)* | **~15–25 h** | ~8–12 h | live OS `eframe`/`wgpu` + `wasm32` UI hosting over `UiApp` (`G6`, `15–25 h`) |

The target application binary was **not** inspected or executed: the clean-room rule in [`AGENTS.md`](AGENTS.md) forbids it, so every requirement is derived from public documentation, peer-reviewed biostatistics literature, and public reference datasets (NIST StRD).

## By dimension

Dimensions overlap (UI, file-format and hardware work is also counted inside feature areas), so this column does not sum to the headline totals.

| Dimension | % | One agent (h) | Doc |
|---|---:|---:|---|
| Features (depth — engine/CLI/`UiApp`) | 83% | 49–80 | [target-app-parity.md](docs/target-app-parity.md) |
| UI/UX & spreadsheet fidelity | 56% | 25–40 | [ui-parity.md](docs/ui-parity.md) |
| File formats & figure export | 82% | 6–10 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (GPU, HiDPI, clipboard, print) | 35% | 12–18 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization & number formatting | 80% | 8–12 | [localization-parity.md](docs/localization-parity.md) |
| Numerical accuracy & corpus | 92% | 1–2 | [gaps.md › G1](docs/gaps.md#g1-numerical-foundations-and-nist-strd-oracle) |
| Performance (1M-cell grid, batch fits) | 75% | 3–5 | [gaps.md › G12](docs/gaps.md#g12-performance-budgets-large-table-virtualization-and-fuzzing) |
| Platforms & packaging | 65% | 8–12 | [releasing.md](docs/releasing.md) |
| Agent & headless automation | 100% (beyond target on MCP) | — | [scripting-and-automation.md](docs/scripting-and-automation.md), [mcp.md](docs/mcp.md) |

## Features

| Area | Weight | % | One agent (h) | Implementation guide |
|---|---:|---:|---:|---|
| Typed data tables (8 archetypes), exclusions & Info constants | 14% | 85% | 1–2 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Calculated variables, formulas & derived transforms | 8% | 85% | 2–4 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Descriptive stats, normality & Grubbs/ROUT outliers | 8% | 90% | 1–2 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Two-group, one-/two-/three-way ANOVA, RM, REML & multiplicity | 16% | 85% | 3–5 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Nonlinear regression solver, profile CIs, global fits & AICc/*F* | 14% | 85% | 4–6 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Built-in equation library & custom ODE/algebraic DSL | 10% | 80% | 3–5 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Contingency, Kaplan–Meier/Cox survival, ROC & multivariate/PCA | 10% | 80% | 3–5 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Publication graphing, broken/log/probit axes & significance brackets | 10% | 85% | 2–4 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Multi-panel Page Layouts, SVG/PDF/TIFF export & style cloning | 5% | 80% | 5–7 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Project Navigator, spreadsheet UX, `.lumencraft`/`.prism`/`.pzfx` & automation | 5% | 60% | 25–40 | [ui-parity.md](docs/ui-parity.md), [scripting-and-automation.md](docs/scripting-and-automation.md) |

Weights, scoring rubric, and missing items per area: [`docs/target-app-parity.md`](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | Translated |
|---|---|---|---:|
| English | `en` | complete baseline catalog (`cargo xtask i18n-report`) | 100% (`167` keys) |
| Tier-1 scientific locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`) | — | all 8 Tier-1 translations & decimal/CSV presets verified (`G13`) | 100% (`9` × `167` = `1,503` keys, `0` missing) |

## Upcoming

In order; details and exit criteria are in the [execution order](docs/roadmap.md#execution-order), per-gap scope in [`docs/gaps.md`](docs/gaps.md).

1. **Close desktop Alpha (`G6` Alpha, `15–25 h`):** wire live OS `eframe`/`egui`/`wgpu` desktop windowing & `wasm32` sysroot verification over `UiApp`.
2. **Finish Core edge items (`G1`–`G6` Core, `22–36 h`):** add crash-recovery lock/autosave journal (`G2`), `NewmanKeuls`/`FisherLsd` & `multiple_row_t_tests` (`G3`), log-grid band sampling & 5 receptor-binding equations (`G4`), `PDF 1.7` TTF subset embedding + viewer spot-check (`G5`), interactive pointer/inline UI editing (`G6`), and expanded `D1` R/SciPy edge fixtures (`G1`).
3. **Close Beta edge & performance targets (`G7`–`G12`, `16–27 h`):** optimize 96-curve 4PL batch fit from `16.91 s` down to `< 50 ms` (`G12`), wire standalone `AnalysisKind::BlandAltman` / Barnard / Passing–Bablok / Multiple Imputation (`G8`) and MV grouping *t*-test / KM (`G7`), add `[InitialValues]`/`<C>`+ DSL blocks (`G9`), and connect native OS pasteboard/printer APIs (`G11`).
4. **Release hardening & localization (`G13` + `M7`, `20–30 h`):** Tier-2 locales, RTL layout mirroring (`ar`, `he`), desktop CJK system font fallback verification, trackpad gestures, and signed installers (`1.0`).

**Decision `D1` (2026-11-13):** reference values produced by open-source statistical software may be used as test fixtures, within the limits in [`AGENTS.md`](AGENTS.md). Initial `D1` test suites are in place (`crates/stats/tests/d1_reference_oracles.rs` `20 passed`, `crates/fit/tests/d1_nist_and_pharma_oracles.rs` `18 passed`); see [verification oracles](docs/roadmap.md#verification-oracles).

## Progress log

| Date | Milestone | Summary |
|---|---|---|
| 2026-11-13 | Wave 4 / UI / E2E / CLI | Landed `UiApp` state shell + software RGBA/PNG compositor (`crates/ui-egui`, `41 passed` across default & `--no-default-features`: **Risk Spike S1 GO** `p50 = 0.061 ms` at `1M` cells, `0.044 ms` `Select All + exclude`; `9` locales × `167` keys = `1,503` translations, `0` missing; `wasm32` sysroot skipped locally offline), `lumencraft-mcp` (`25 passed`), `apps/lumencraft` (`6 passed` incl. `control_loopback` `--control`/`--headless-once`) & `apps/lumencraft-web` (`1 passed`), `lumencraft-cli` (`7 passed`, `Slice 0` `slice0_vertical_gate` + `perf --json` measuring `100k` CalcVar `12.023 ms` and identifying `96-curve` 4PL batch `16.91 s` optimization target), `e2e_workflows` (`11 passed`), and `InterpolatedUnknowns` tab in `crates/fit` (`55 passed`). |
| 2026-11-13 | Wave 3 (`L4`–`L5`) | Landed `lumencraft-tools` (`15 passed`, **Risk Spike S1** two-tier grid `1M` cells `p50 = 0.0390 ms`, `max = 0.0612 ms`, tail-jump `0.0150 ms`) and `lumencraft-engine` (`48 passed`: topological reactive DAG, undo/redo journal, `Paste Link`, `command_sweep_props` covering all `73` commands, and `11` end-to-end workflows). |
| 2026-11-13 | Wave 2 (`L2`–`L3`) + Spike `S2` | Landed `lumencraft-stats` (`36 passed` w/ `20` `d1_reference_oracles` & `4` `nist_strd`), `lumencraft-fit` (`55 passed` w/ `18` `d1_nist_and_pharma_oracles` & `16` `dsl_fuzz_props`), `lumencraft-transform` (`=` Calculated Variables & 7 transform suites), `lumencraft-format` (`36 passed` w/ `.lumencraft`, CSV/TSV, clean-room `.prism`/`.pzfx`, `18` `corrupt_archive_fuzz`), and visual stack `text`/`chart`/`svg`/`pdf`/`render` (`73` default / `32` `--no-default-features` tests: **Risk Spike S2 GO** pure-Rust `%PDF-1.4` Core-14 Helvetica/Symbol writer in `0.293–0.395 ms` with `BUNDLED_FONTS` empty). |
| 2026-11-13 | G1 | Landed `lumencraft-math` (`L0`: Neumaier compensated summation & shifted two-pass moments, cancellation-free `erf`/`erfc`/`ln_gamma` and regularized incomplete beta/gamma, tail-safe normal / Student *t* (real df) / *F* / $\chi^2$ / noncentral *t*/*F*/$\chi^2$ CIs / Copenhaver–Holland Studentized range $q(k,\nu)$ / exact multivariate Dunnett *t* integrals, column-pivoted Householder QR with Björck augmented refinement, Golub–Kahan SVD, Cholesky, symmetric eigensolver, quadrature, Brent root-finding, `DeterministicRng`, plus unified `ConfidenceLevel`, `PValueFormat`, and guarded `AxisScale`/`LogitDomain`) alongside `lumencraft-testkit` (106-bit `DoubleDouble` oracle, LRE/ULP metrics, FIPS 180-4 SHA-256 verifier, clean-room NIST StRD `.dat` parsers, and offline stiffness/collinearity benchmarks in `crates/math/tests/nist_strd.rs`) and `cargo xtask corpus` (`--fetch` / `--fetch-only`, 58-file NIST StRD catalog across 9 Univariate, 11 ANOVA, 11 Linear Regression, and 27 Nonlinear Regression datasets verified offline with `LUMENCRAFT_REQUIRE_CORPUS=1`). |
| 2026-11-13 | G2 (partial) | Landed `lumencraft-palette` (`L0`: sRGB/Oklab/OkLCH/CMYK, WCAG contrast, Okabe–Ito & Paul Tol + custom categorical palettes, bounded Viridis/Cividis/Plasma colormaps, Machado CVD simulation, markers & dash styles) and `lumencraft-doc` (`L1`: all 8 `TableArchetype`s, 8 `SubcolumnMode`s + `SummarizedPoint` moment reconstruction, non-destructive exclusions & `*` tokens, freeze-guarded `DataSheet` (`Y`, `X`, and `Survival.times`) accessors, `FloatingNote`, `InfoSheet` constants & `HookSource`, `LinkedDataRegion` + 4-stage `ImportLinkSpec`, `AnalysisSpec`/`ResultSheet`, `GraphSpec`/`LayoutSpec`, `ProjectDocument` DAG dependencies, and enriched `TransformAnalysisSpec::Simulate` with `x_min`, `x_max`, `parameters`), unified `ConfidenceLevel`/`PValueFormat`/`AxisScale`/`LogitDomain` across `math` and `doc`, added workspace-allowlisted `serde`, `serde_json` (`float_roundtrip`), `flate2` (`rust_backend`), `crc32fast`, and `proptest`, and verified `serde` + `proptest!` round-trips across `math`, `palette`, and `doc`; `crates/format` (`.lumencraft` ZIP/CSV I/O) and `crates/engine` (reactive DAG + undo/redo) remain queued. |
| 2026-11-13 | M0 | Clean-room subsystem implementation guidelines (`data-tables-and-transforms.md`, `statistics-implementation.md`, `curve-fitting-implementation.md`, `graphing-and-layouts.md`, `scripting-and-automation.md`), Prism 11.1 parity checklists, ranked `G1`–`G14` work list, and crate rustdoc wiring completed. |
| 2026-11-13 | M0 | Initialized 21-crate layered Rust 2024 workspace (`L0`–`L7`), `xtask` CI/layer/asset/brand/cleanroom/cursor/i18n/ui-commands gates, original vector/raster app icons, cross-platform `packaging/`, and GitHub Actions workflows. |
