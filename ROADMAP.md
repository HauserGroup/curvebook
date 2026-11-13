# LumenCraft Roadmap

**Stage: pre-alpha (`M0`)** · next: alpha, 6 core scientific workflows end to end (`M1`–`M5`) and ~165–255 h away

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** M0 workspace, CI & clean-room implementation guidelines wired in · **Target:** GraphPad Prism 11 (11.1.x)

LumenCraft is a clean-room, open-source, pure-Rust reimplementation of the scientific data-table, biostatistics, nonlinear regression (curve fitting), and publication graphing workflow. It runs natively on macOS, Windows, Linux, and FreeBSD, and in the browser via WebAssembly (`WASM`), and agents can drive all of it headlessly over MCP, the CLI, and a loopback JSONL control channel. This page is the summary; the evidence is in [`docs/target-app-parity.md`](docs/target-app-parity.md), the ranked work list is in [`docs/gaps.md`](docs/gaps.md), and the subsystem implementation guidelines live in [`docs/`](docs/).

**Why pre-alpha (`M0`):** the layered 21-member workspace (`L0`–`L7`), strict no-panic / no-`unsafe` Clippy gates, cross-platform packaging definitions, CI workflows, UI command sync, clean-room/brand scanners, and subsystem mathematical specifications are complete and passing (`cargo xtask ci`), while the functional data-table, statistical, curve-fitting, graphing, and shell crates are clean M0 architectural stubs (`~2%` infrastructure baseline). Alpha requires all six everyday wet-lab workflows in the [Alpha gate](docs/roadmap.md#alpha-gate) to work end to end with lossless `.lumencraft` save/reopen and NIST StRD verification.

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** (table archetypes, statistical tests, equations, graph types, exports) | **~2%** | M0 baseline (`L0`–`L7` crates, `xtask` gates, i18n registry, MCP/CLI/egui entry points wired; domain algorithms queued in `G1`–`G14`) |
| **Feature depth** (weighted by scientific use, scored by verified behaviour) | **~1%** | estimated ([methodology](docs/target-app-parity.md#methodology)) |
| **To alpha** | **~165–255 h** one agent · ~45–75 h with 4–6 agents | estimated ([Alpha gate](docs/roadmap.md#alpha-gate)) |
| **To beta** | **~290–440 h** one agent · ~80–125 h with 4–6 agents | estimated |
| **To full parity** | **~440–680 h** one agent · ~120–190 h with 4–6 agents | estimated |

**Readiness by audience** (estimated; formula in [`docs/target-app-parity.md`](docs/target-app-parity.md#ready-for-real-work)):

| Audience | Ready % | Agent hours to ~95% (one agent) | With 4–6 agents | Work that dominates |
|---|---:|---:|---:|---|
| **Full Prism 11 (Pro / Enterprise)** (decides the stage) | **~1%** | **~440–680 h** | ~120–190 h | mainstream scope below, plus Multiple Variables unified analyses & Multifactor ANOVA (`G7`, 30–45 h), Cox PH / multivariate GLMs / PCA (`G8`, 30–45 h), custom ODE/implicit equations (`G4`, 20–30 h), `.prism`/`.pzfx` import (`G10`, 20–35 h), localization (`G13`, 35–55 h) |
| **Mainstream wet-lab researcher** | **~2%** | **~260–390 h** | ~75–115 h | special functions & linear algebra (`G1`), typed tables & transforms (`G2`), core *t*-tests/ANOVA/REML/post-hoc (`G3`), LM nonlinear fitting + dose–response/kinetics (`G4`), XY/Column/Grouped/Survival graphs + SVG/PDF (`G5`), desktop/web spreadsheet & inspector UX (`G6`) |
| **Essentials / student biostatistics user** | **~3%** | **~140–210 h** | ~40–60 h | `XY`/`Column`/`Grouped` tables, CSV paste, descriptive stats, *t*-tests, one-/two-way ANOVA, linear + 4PL regression, bar/scatter/box plots, SVG/PDF/PNG export |

The target application binary was **not** inspected or executed: the clean-room rule in [`AGENTS.md`](AGENTS.md) forbids it, so every requirement is derived from public documentation, peer-reviewed biostatistics literature, and public reference datasets (NIST StRD).

## By dimension

| Dimension | % | One agent (h) | Doc |
|---|---:|---:|---|
| Features (depth) | 1% | 290–450 | [target-app-parity.md](docs/target-app-parity.md) |
| UI/UX & spreadsheet fidelity | 2% | 65–100 | [ui-parity.md](docs/ui-parity.md) |
| File formats & figure export | 2% | 35–60 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (GPU, HiDPI, clipboard, print) | 5% | 20–35 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization & number formatting | 5% | 35–55 | [localization-parity.md](docs/localization-parity.md) |
| Numerical accuracy & corpus | 5% | 20–30 | [gaps.md › G1](docs/gaps.md#g1-numerical-foundations-and-nist-strd-oracle) |
| Performance (1M-cell grid, batch fits) | 10% | 15–25 | [gaps.md › G12](docs/gaps.md#g12-performance-budgets-and-large-table-virtualization) |
| Platforms & packaging | 40% | 10–20 | [releasing.md](docs/releasing.md) |
| Agent & headless automation | 15% (beyond target on MCP) | 15–25 | [scripting-and-automation.md](docs/scripting-and-automation.md), [mcp.md](docs/mcp.md) |

## Features

| Area | Weight | % | One agent (h) | Implementation guide |
|---|---:|---:|---:|---|
| Typed data tables (8 archetypes), exclusions & Info constants | 14% | 2% | 25–40 | [data-tables-and-transforms.md](docs/data-tables-and-transforms.md) |
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

## Upcoming (pick work from [`docs/gaps.md`](docs/gaps.md))

1. **G1 — Numerical foundations & NIST StRD harness (`crates/math`, `crates/testkit`):** incomplete beta/gamma, tail-safe Student *t* / *F* / $\chi^2$ / normal / Studentized-range / Dunnett integrals, compensated summation, rank-revealing Householder QR & SVD.
2. **G2 — Typed data tables, `.lumencraft` container & CSV/TSV I/O (`crates/doc`, `crates/format`, `crates/engine`):** all 8 table archetypes, replicate subcolumns, cell exclusions, Info constants, topological reactive DAG, undo/redo command journal.
3. **G3 — Core biostatistics & multiplicity (`crates/stats`):** descriptive suite, normality battery, Grubbs/ROUT, *t*-tests + standardized effect sizes, one-/two-way & RM ANOVA, REML mixed models, Dunnett/Tukey/Šídák/Holm–Šídák/BKY FDR.
4. **G4 — Nonlinear regression & built-in equation library (`crates/fit`):** Moré Levenberg–Marquardt, Cauchy robust & ROUT fitting, shared global parameters, Venzon–Moolgavkar profile-likelihood CIs, AICc & extra-sum-of-squares *F*-test, dose–response/binding/enzyme-kinetics/exponential models.
5. **G5 — Publication chart compiler & SVG/PDF export (`crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`):** XY, Column, Grouped, Survival, and Multiple Variables graphs, nonlinear & discontinuous axes, live pairwise *p*-value brackets, Page Layouts.
6. **G6 — Interactive desktop & WASM shell (`crates/ui-egui`, `apps/lumencraft`, `apps/lumencraft-web`):** five-folder Project Navigator, Family pane, virtualized subcolumn spreadsheet grid, Analyze wizard, Result tabs + Analysis Checklist, Format Graph inspector.

## Progress log

| Date | Milestone | Summary |
|---|---|---|
| 2026-11-13 | M0 | Clean-room subsystem implementation guidelines (`data-tables-and-transforms.md`, `statistics-implementation.md`, `curve-fitting-implementation.md`, `graphing-and-layouts.md`, `scripting-and-automation.md`), Prism 11.1 parity checklists, ranked `G1`–`G14` work list, and crate rustdoc wiring completed. |
| 2026-11-13 | M0 | Initialized 21-crate layered Rust 2024 workspace (`L0`–`L7`), `xtask` CI/layer/asset/brand/cleanroom/cursor/i18n/ui-commands gates, original vector/raster app icons, cross-platform `packaging/`, and GitHub Actions workflows. |
