# LumenCraft Roadmap

**Stage: pre-alpha (`M0`)** · next: alpha, one experiment from raw data to exported figure at Essentials scope, ~120–185 h away

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** alpha redefined as the Essentials scope; execution order now runs a vertical slice and two risk spikes before widening ([`docs/roadmap.md`](docs/roadmap.md#execution-order)) · **Target:** GraphPad Prism 11 (11.1.x)

LumenCraft is a clean-room, open-source, pure-Rust reimplementation of the scientific data-table, biostatistics, nonlinear regression (curve fitting), and publication graphing workflow. It runs natively on macOS, Windows, Linux, and FreeBSD, and in the browser via WebAssembly (`WASM`), and agents can drive all of it headlessly over MCP, the CLI, and a loopback JSONL control channel. This page is the summary; the evidence is in [`docs/target-app-parity.md`](docs/target-app-parity.md), the ranked work list is in [`docs/gaps.md`](docs/gaps.md), and the subsystem implementation guidelines live in [`docs/`](docs/).

**Why pre-alpha (`M0`):** the layered 21-member workspace (`L0`–`L7`), strict no-panic / no-`unsafe` Clippy gates, cross-platform packaging definitions, CI workflows, UI command sync, clean-room/brand scanners, subsystem mathematical specifications, `G1` numerical foundations + 58-dataset NIST StRD corpus verifier (`lumencraft-math`, `lumencraft-testkit`, `cargo xtask corpus`), and the `L0`/`L1` foundation schemas of `G2` (`lumencraft-palette`, `lumencraft-doc` with `serde` & `proptest` round-trips) are complete and passing, while `crates/format` (`.lumencraft` ZIP/CSV I/O), `crates/engine` (reactive DAG + undo/redo), and the statistical, curve-fitting, chart-compiler, and shell crates remain M0 stubs (`~3%` weighted feature depth). Alpha requires the Essentials scope in the [Alpha gate](docs/roadmap.md#alpha-gate) to work end to end with lossless `.lumencraft` save/reopen and numerics verified against a named reference. The six everyday wet-lab workflows at full depth are the later [Core-workflow gate](docs/roadmap.md#core-workflow-gate).

**Where the work is:** Wave 1 (`L0`–`L1`) is done. Wave 2 (`L2`–`L3` libraries) is in progress with its scope frozen. Next come one vertical slice through every layer and two risk spikes, then the engine and shell for the Alpha scope. The numbers below count only merged, end-to-end behaviour, so they do not move until that work is merged and command-wired. See the [execution order](docs/roadmap.md#execution-order).

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** (table archetypes, statistical tests, equations, graph types, exports) | **~7%** | `G1` numerical foundations & 58-dataset StRD oracle + `G2` `lumencraft-palette` (`L0`) & `lumencraft-doc` (`L1`) schemas landed alongside `xtask` gates, i18n registry, and MCP/CLI/egui entry points; `crates/format`, engine DAG, solvers & UI queued in `G2`–`G14` |
| **Feature depth** (weighted by scientific use, scored by verified behaviour) | **~3%** | estimated ([methodology](docs/target-app-parity.md#methodology)) |
| **To alpha** (Essentials scope) | **~120–185 h** one agent · ~35–55 h with 4–6 agents | sum of Alpha-gated gap hours ([Alpha gate](docs/roadmap.md#alpha-gate)) |
| **To core workflows** | **~210–330 h** one agent · ~60–95 h with 4–6 agents | Alpha plus Core-gated gap hours ([Core-workflow gate](docs/roadmap.md#core-workflow-gate)) |
| **To beta** | **~360–565 h** one agent · ~100–160 h with 4–6 agents | Alpha, Core and Beta-gated gap hours |
| **To full parity** | **~410–645 h** one agent · ~115–185 h with 4–6 agents | all gaps plus `M7` hardening and polish |

Hours are implementation estimates and exclude human review, which sets the real pace ([landing rules](docs/roadmap.md#landing-rules)). The multi-agent figures assume work that splits cleanly across crates; the engine is a single crate on the critical path, so they are optimistic until it lands.

**Readiness by audience** (estimated; formula in [`docs/target-app-parity.md`](docs/target-app-parity.md#ready-for-real-work)):

| Audience | Ready % | Agent hours to ~95% (one agent) | With 4–6 agents | Work that dominates |
|---|---:|---:|---:|---|
| **Full Prism 11 (Pro / Enterprise)** (decides `1.0`) | **~2%** | **~410–645 h** | ~115–185 h | mainstream scope below, plus Multiple Variables unified analyses & Multifactor ANOVA (`G7`, 30–45 h), survival / Cox PH / multivariate GLMs / PCA (`G8`, 39–61 h), custom ODE/implicit equations (`G9`, 25–40 h), `.prism`/`.pzfx` import (`G10`, 20–35 h), localization (`G13`, 35–55 h) |
| **Mainstream wet-lab researcher** | **~4%** | **~235–355 h** | ~65–100 h | `.lumencraft` ZIP/CSV I/O & reactive DAG (`G2`), core *t*-tests/ANOVA/REML/post-hoc (`G3`), LM nonlinear fitting + dose–response/kinetics (`G4`), XY/Column/Grouped graphs + SVG/PDF (`G5`), survival analysis and graph (`G8`), desktop/web spreadsheet & inspector UX (`G6`) |
| **Essentials / student biostatistics user** (decides alpha) | **~5%** | **~125–190 h** | ~35–55 h | `XY`/`Column`/`Grouped` tables + `.lumencraft`/CSV I/O, descriptive stats, *t*-tests, one-/two-way ANOVA, linear + 4PL regression, bar/scatter/box plots, SVG/PDF/PNG export |

The target application binary was **not** inspected or executed: the clean-room rule in [`AGENTS.md`](AGENTS.md) forbids it, so every requirement is derived from public documentation, peer-reviewed biostatistics literature, and public reference datasets (NIST StRD).

## By dimension

Dimensions overlap (UI, file-format and hardware work is also counted inside feature areas), so this column does not sum to the headline totals.

| Dimension | % | One agent (h) | Doc |
|---|---:|---:|---|
| Features (depth) | 3% | 284–442 | [target-app-parity.md](docs/target-app-parity.md) |
| UI/UX & spreadsheet fidelity | 2% | 65–100 | [ui-parity.md](docs/ui-parity.md) |
| File formats & figure export | 2% | 35–60 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (GPU, HiDPI, clipboard, print) | 5% | 20–35 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization & number formatting | 5% | 35–55 | [localization-parity.md](docs/localization-parity.md) |
| Numerical accuracy & corpus | 70% | 2–4 | [gaps.md › G1](docs/gaps.md#g1-numerical-foundations-and-nist-strd-oracle) |
| Performance (1M-cell grid, batch fits) | 10% | 15–25 | [gaps.md › G12](docs/gaps.md#g12-performance-budgets-large-table-virtualization-and-fuzzing) |
| Platforms & packaging | 40% | 10–20 | [releasing.md](docs/releasing.md) |
| Agent & headless automation | 15% (beyond target on MCP) | 15–25 | [scripting-and-automation.md](docs/scripting-and-automation.md), [mcp.md](docs/mcp.md) |

## Features

| Area | Weight | % | One agent (h) | Implementation guide |
|---|---:|---:|---:|---|
| Typed data tables (8 archetypes), exclusions & Info constants | 14% | 15% | 18–30 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Calculated variables, formulas & derived transforms | 8% | 0% | 20–30 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
| Descriptive stats, normality & Grubbs/ROUT outliers | 8% | 0% | 18–26 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Two-group, one-/two-/three-way ANOVA, RM, REML & multiplicity | 16% | 0% | 45–70 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Nonlinear regression solver, profile CIs, global fits & AICc/*F* | 14% | 0% | 40–60 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Built-in equation library & custom ODE/algebraic DSL | 10% | 0% | 30–48 | [curve-fitting-implementation.md](docs/curve-fitting-implementation.md) |
| Contingency, Kaplan–Meier/Cox survival, ROC & multivariate/PCA | 10% | 0% | 35–55 | [statistics-implementation.md](docs/statistics-implementation.md) |
| Publication graphing, broken/log/probit axes & significance brackets | 10% | 0% | 35–55 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Multi-panel Page Layouts, SVG/PDF/TIFF export & style cloning | 5% | 0% | 18–28 | [graphing-and-layouts.md](docs/graphing-and-layouts.md) |
| Project Navigator, spreadsheet UX, `.lumencraft`/`.prism`/`.pzfx` & automation | 5% | 8% | 25–40 | [ui-parity.md](docs/ui-parity.md), [scripting-and-automation.md](docs/scripting-and-automation.md) |

Weights, scoring rubric, and missing items per area: [`docs/target-app-parity.md`](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | Translated |
|---|---|---|---:|
| English | `en` | baseline catalog wired (`cargo xtask i18n-report`) | 100% (M0 strings) |
| Tier-1 scientific locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`) | — | planned (`G13`) | 0% |

## Upcoming

In order; details and exit criteria are in the [execution order](docs/roadmap.md#execution-order), per-gap scope in [`docs/gaps.md`](docs/gaps.md).

1. **Land Wave 2, scope frozen (`crates/format`, `crates/stats`, `crates/fit`, `crates/transform`):** finish what is in flight and land it one gap per pull request, including the `G1` corpus wiring with `G3` and `G4`. Start nothing new beyond what the Alpha scope needs.
2. **Slice 0, one vertical slice (`crates/engine`, `crates/chart`, `crates/svg`, `apps/lumencraft-cli`, `15–25 h`):** `XY` table → 4PL fit → graph → SVG → `.lumencraft` save/reopen, every step an engine command driven from the CLI. Fix any schema or save-format problem it exposes before going further.
3. **Risk spikes, in parallel with Slice 0 (`6–10 h` each):** `S1` spreadsheet grid at 60 fps on desktop and in the browser; `S2` publication-quality text and PDF with no bundled fonts. Each ends in a written go or redesign decision.
4. **Wave 3, engine and commands for the Alpha scope (`crates/engine`, `crates/tools`):** reactive DAG, undo/redo, and a `CommandSpec` for every Alpha-scope analysis, fit, graph, and export. This is the critical path.
5. **Wave 4, shell and automation for the Alpha scope (`crates/ui-egui`, `crates/mcp`, `apps/*`):** Navigator, subcolumn grid, Analyze dialog, Result tabs, basic Format Graph, and the CLI/MCP handlers. **Alpha.**
6. **Widen to the Core workflows**, one workflow at a time through all layers, then Beta scope one gap at a time.

**Decision `D1` (2026-11-13):** reference values produced by open-source statistical software may be used as test fixtures, within the limits in [`AGENTS.md`](AGENTS.md). Methods with no textbook or NIST reference are verified this way; see [verification oracles](docs/roadmap.md#verification-oracles).

## Progress log

| Date | Milestone | Summary |
|---|---|---|
| 2026-11-13 | G1 | Landed `lumencraft-math` (`L0`: Neumaier compensated summation & shifted two-pass moments, cancellation-free `erf`/`erfc`/`ln_gamma` and regularized incomplete beta/gamma, tail-safe normal / Student *t* (real df) / *F* / $\chi^2$ / noncentral *t*/*F*/$\chi^2$ CIs / Copenhaver–Holland Studentized range $q(k,\nu)$ / exact multivariate Dunnett *t* integrals, column-pivoted Householder QR with Björck augmented refinement, Golub–Kahan SVD, Cholesky, symmetric eigensolver, quadrature, Brent root-finding, `DeterministicRng`, plus unified `ConfidenceLevel`, `PValueFormat`, and guarded `AxisScale`/`LogitDomain`) alongside `lumencraft-testkit` (106-bit `DoubleDouble` oracle, LRE/ULP metrics, FIPS 180-4 SHA-256 verifier, clean-room NIST StRD `.dat` parsers, and offline stiffness/collinearity benchmarks in `crates/math/tests/nist_strd.rs`) and `cargo xtask corpus` (`--fetch` / `--fetch-only`, 58-file NIST StRD catalog across 9 Univariate, 11 ANOVA, 11 Linear Regression, and 27 Nonlinear Regression datasets verified offline with `LUMENCRAFT_REQUIRE_CORPUS=1`). |
| 2026-11-13 | G2 (partial) | Landed `lumencraft-palette` (`L0`: sRGB/Oklab/OkLCH/CMYK, WCAG contrast, Okabe–Ito & Paul Tol + custom categorical palettes, bounded Viridis/Cividis/Plasma colormaps, Machado CVD simulation, markers & dash styles) and `lumencraft-doc` (`L1`: all 8 `TableArchetype`s, 8 `SubcolumnMode`s + `SummarizedPoint` moment reconstruction, non-destructive exclusions & `*` tokens, freeze-guarded `DataSheet` (`Y`, `X`, and `Survival.times`) accessors, `FloatingNote`, `InfoSheet` constants & `HookSource`, `LinkedDataRegion` + 4-stage `ImportLinkSpec`, `AnalysisSpec`/`ResultSheet`, `GraphSpec`/`LayoutSpec`, `ProjectDocument` DAG dependencies, and enriched `TransformAnalysisSpec::Simulate` with `x_min`, `x_max`, `parameters`), unified `ConfidenceLevel`/`PValueFormat`/`AxisScale`/`LogitDomain` across `math` and `doc`, added workspace-allowlisted `serde`, `serde_json` (`float_roundtrip`), `flate2` (`rust_backend`), `crc32fast`, and `proptest`, and verified `serde` + `proptest!` round-trips across `math`, `palette`, and `doc`; `crates/format` (`.lumencraft` ZIP/CSV I/O) and `crates/engine` (reactive DAG + undo/redo) remain queued. |
| 2026-11-13 | M0 | Clean-room subsystem implementation guidelines (`data-tables-and-transforms.md`, `statistics-implementation.md`, `curve-fitting-implementation.md`, `graphing-and-layouts.md`, `scripting-and-automation.md`), Prism 11.1 parity checklists, ranked `G1`–`G14` work list, and crate rustdoc wiring completed. |
| 2026-11-13 | M0 | Initialized 21-crate layered Rust 2024 workspace (`L0`–`L7`), `xtask` CI/layer/asset/brand/cleanroom/cursor/i18n/ui-commands gates, original vector/raster app icons, cross-platform `packaging/`, and GitHub Actions workflows. |
