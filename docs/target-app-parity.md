# Parity with GraphPad Prism 11

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Wave 1–3, Slice 0 (`lumencraft-cli`), Risk Spikes `S1`/`S2` (**GO**), headless Alpha/Core/Beta (`G1`–`G14`) engine/CLI/MCP/`--control` pipelines, `UiApp` software-compositor shell, and Tier-1 localization (`9` locales × `167` keys) landed and tested; remaining depth concentrated in live OS windowing/`wgpu`/`wasm32` UI hosting (`G6`), 96-curve batch optimization (`G12`), and remaining library edge items · **Target:** GraphPad Prism 11 (11.1.x; public guides & release notes through 11.1.0, August 2026)

Authoritative clean-room parity assessment for LumenCraft. Summary in [`ROADMAP.md`](../ROADMAP.md); ranked work list in [`gaps.md`](gaps.md); stage gates, execution order, and verification oracles in [`roadmap.md`](roadmap.md).

## Methodology

Every score in this document is graded strictly from shipped, tested code in `crates/*` and `apps/*` against public documentation (`https://www.graphpad.com/guides/prism/latest/{user-guide,statistics,curve-fitting}/` and public release notes at `https://www.graphpad.com/updates`), peer-reviewed statistical methods, and certified reference benchmarks (NIST StRD). In accordance with [`AGENTS.md`](../AGENTS.md), no proprietary binary, installation directory, or vendor-created sample file is ever inspected or run.

### Scoring rubric (per feature area)

| Score | Meaning |
|---:|---|
| `0%` | Absent or M0 crate stub only. |
| `10–30%` | Core data types and happy-path math exist headlessly with unit tests, not yet wired through engine commands or UI. |
| `40–65%` | Mainstream variants work end to end (engine + CLI/MCP + UI + `.lumencraft` save/reopen) and pass NIST/analytical tests; secondary options or diagnostics missing. |
| `70–85%` | Everyday wet-lab researcher workflow complete, including missing-value edge cases, multiplicity CIs, plain-language checklists, and vector SVG/PDF export. |
| `90–100%` | Indistinguishable mathematical precision and workflow completeness against public Prism 11.1 specifications, property-tested against hostile inputs. |

### Ready for real work

`Feature depth` weights the 10 feature areas below by scientific frequency of use. `Ready for real work` discounts feature depth by workflow-blocking multipliers:
1. Lossless native `.lumencraft` + CSV/TSV + legacy `.pzfx`/`.prism` project interchange,
2. Interactive spreadsheet ergonomics (subcolumns, paste-transpose, non-destructive exclusions, undo/redo),
3. Certified numerical accuracy on extreme degrees of freedom, tied ranks, missing repeated measures, and stiff nonlinear fits.

### Calibration

Hour ranges are single-agent wall-clock hours calibrated from sibling Craft repositories (`~1.0–2.0 h` per self-contained statistical test or equation family with NIST/analytical tests + command wiring; `~6–12 h` per multi-tab dialog or chart family). Parallel agents (`4–6`) achieve `~3.2–3.8×` wall-clock speedup when partitioned cleanly across `math`/`stats`, `fit`, `doc`/`transform`/`format`, `chart`/`svg`/`pdf`, and `ui-egui`.

---

## Feature areas

| # | Area | Weight | Score | Est. remaining (h) | Spec & checklist |
|---:|---|---:|---:|---:|---|
| 1 | **Typed data tables (8 archetypes), exclusions & Info constants** | 14% | 85% | 1–2 | [data-tables-and-transforms.md](data-tables-and-transforms.md), [file-format-parity.md](file-format-parity.md) |
| 2 | **Calculated variables (`=`), window functions & derived transforms** | 8% | 85% | 2–4 | [data-tables-and-transforms.md](data-tables-and-transforms.md) |
| 3 | **Descriptive statistics, normality/lognormality & Grubbs/ROUT outliers** | 8% | 90% | 1–2 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 4 | **Comparisons: *t*-tests, 1/2/3-way & Multifactor ANOVA, RM, REML & multiplicity** | 16% | 85% | 3–5 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 5 | **Nonlinear regression core: LM solver, weights, Cauchy/ROUT, global fits, profile CIs, AICc/*F*** | 14% | 85% | 4–6 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 6 | **Built-in equation library (dose–response, binding, kinetics, growth) & custom ODE/DSL** | 10% | 80% | 3–5 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 7 | **Contingency, Kaplan–Meier/Cox survival, ROC, multiple GLMs, PCA & clustering** | 10% | 80% | 3–5 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 8 | **Publication graphing: 8 graph families, nonlinear/broken axes, live *p*-value brackets** | 10% | 85% | 2–4 | [graphing-and-layouts.md](graphing-and-layouts.md), [graphing-parity.md](graphing-parity.md) |
| 9 | **Page Layouts, style cloning ("Wand"), vector SVG/PDF & high-DPI TIFF/PNG export** | 5% | 80% | 5–7 | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) |
| 10 | **Workspace UX, `.lumencraft`/`.prism`/`.pzfx` I/O, templates, CLI, `--control` & MCP** | 5% | 60% | 25–40 | [ui-parity.md](ui-parity.md), [scripting-and-automation.md](scripting-and-automation.md) |
| | **Total / Weighted depth (engine, CLI/MCP & `UiApp` state shell)** | **100%** | **~83%** | **49–80 h** | (plus `G1` `D1` edge fixtures, `G10`/`G12` hardening, `G13` Tier-2/RTL, & `M7` cross-OS hardware/GPU polish → **`73–118 h`** to `1.0`; see [`gaps.md`](gaps.md#ranked)) |

---

## Evidence: what ships today (`Wave 1–3` + `Slice 0` + `S1`/`S2` **GO** + Headless `G1`–`G14` + `UiApp` state shell)

- **Workspace & dependency enforcement (`xtask/src/layers.rs`):** 17 library crates (`math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`, `tools`, `engine`, `ui-egui`, `mcp`, `testkit`), 3 apps (`lumencraft`, `lumencraft-cli`, `lumencraft-web`), and `xtask` compile cleanly under Rust 2024 with `#![forbid(unsafe_code)]` and workspace-wide `deny` lints on `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable`.
- **Numerical foundations, StRD corpus & statistical/fitting engines (`crates/math`, `crates/stats`, `crates/fit`, `crates/transform`, `crates/testkit`):** `lumencraft-math` (`L0`), `lumencraft-testkit`, and `cargo xtask corpus` (`58` NIST StRD `.dat` datasets) anchor `lumencraft-stats` (`36 passed`: `20` `d1_reference_oracles`, `4` `nist_strd`, `6` `prop_stats`, `4` `engine_smoke`) and `lumencraft-fit` (`55 passed`: `18` `d1_nist_and_pharma_oracles`, `16` `dsl_fuzz_props`, `1` `nist_strd`, `4` `prop_fit`, plus `InterpolatedUnknowns` tab), alongside `lumencraft-transform` (`=` Calculated Variables + 7 derived transform suites, `100k` rows in `12.023 ms`). Known open library items include `NewmanKeuls`/`FisherLsd`, `multiple_row_t_tests`, standalone `AnalysisKind::BlandAltman`, Barnard/Passing–Bablok/Multiple Imputation, log-grid band sampling, 5 receptor-binding/allosteric models, and `96-curve` 4PL batch optimization (`16.91 s` vs `< 50 ms` budget).
- **Document model, persistence & `.prism`/`.pzfx` interop (`crates/palette`, `crates/doc`, `crates/format`):** `lumencraft-palette`, `lumencraft-doc` (all 8 `TableArchetype`s, 8 `SubcolumnMode`s, exclusions, `InfoSheet`, `LinkedDataRegion`), and `lumencraft-format` (`36 passed` including `18` `corrupt_archive_fuzz`, `4` `interop_and_export`, `3` `prop_format`) deliver lossless `.lumencraft` save/reopen, RFC-4180 CSV/TSV, schema migration from `v0`, cycle detection, and bounded clean-room `.prism`/`.pzfx` import (atomic crash-recovery journal remains open).
- **Scientific text, chart compiler, vector PDF/SVG & software compositor (`crates/text`, `crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`):** `73 passed` default / `32 passed` `--no-default-features` (`bundled-fonts` disabled). **Risk Spike `S2` = GO** (`0.293–0.395 ms` PDF generation w/ empty `BUNDLED_FONTS`, pure-Rust `%PDF-1.4` writer with Core-14 Helvetica/Symbol selectable Greek/math text + outlined fallback, `FlateDecode`, `MediaBox`/`TrimBox`/`BleedBox`, programmatic structural parser verification; PDF 1.7 + TTF subset glyph embedding & manual GUI viewer inspection remain open). `lumencraft-render` adds 72–1200 DPI PNG and uncompressed/Deflate/LZW TIFF rasterization.
- **Tools, reactive engine, Slice 0 CLI & MCP automation (`crates/tools`, `crates/engine`, `crates/mcp`, `apps/lumencraft-cli`):** `lumencraft-tools` (`15 passed`), `lumencraft-engine` (`48 passed`: `7` lib, `11` `cmd_data`, `7` `cmd_sci`, `8` `dag_reactivity`, `4` `command_sweep_props` covering all `73` commands, `11` `e2e_workflows` spanning Alpha/Core/Beta), `lumencraft-mcp` (`25 passed`, synced `UI_COMMANDS`), and `lumencraft-cli` (`7 passed` including `Slice 0` `slice0_vertical_gate`: `Eckerle4` & `Rat43` checks, bit-identical `.lumencraft` save/reopen & SVG export).
- **UI shell, localization & hardware adapters (`crates/ui-egui`, `apps/lumencraft`, `apps/lumencraft-web`, `packaging/`):** `lumencraft-ui-egui` (`41 passed` across default & `--no-default-features`) ships a pure-Rust `UiApp` state shell + software RGBA/PNG compositor (`9` locales × `167` keys = `1,503` verified translations, `0` missing; **Risk Spike `S1` = GO** at engine/UI-shell layer with `0.0390 ms` tools / `0.061 ms` UI p50 frame times at `1M` cells and `0.044 ms` `Select All + exclude`; `wasm32` sysroot unavailable locally offline). `apps/lumencraft` (`6 passed`: `2` unit + `4` `control_loopback`, including `--control`, `--headless-once`, and loopback address guard) and `apps/lumencraft-web` (`1 passed`, `init_web_app()`) wire headless binary/control automation cleanly. `UiApp` exposes Navigator, Family pane, virtualized grid, Analyze/Clone/Graph-Gallery/Transform/Format inspectors, `ClipboardBundle`, and physical-scale `PrintJobSpec`, while live OS desktop `eframe`/`wgpu` event-loop hosting, OS pasteboard/printer hooks, and pointer drag-reorder remain open before shipping desktop GUI Alpha.
- **Clean-room, brand & asset gates (`xtask/src/{cleanroom,brands,assets,cursors,ui_commands,i18n_report}.rs`):** Automated CI verification of `ASSETS.md`, competitor brand exclusion, cursor TOML sync, `UI_COMMANDS` -> MCP sync, `9`-locale translation completeness, and vendor signature rejection.

## Missing by area (summary)

Detailed item-by-item checkboxes live in [`stats-parity.md`](stats-parity.md), [`fitting-parity.md`](fitting-parity.md), [`graphing-parity.md`](graphing-parity.md), [`ui-parity.md`](ui-parity.md), [`file-format-parity.md`](file-format-parity.md), [`hardware-parity.md`](hardware-parity.md), and [`localization-parity.md`](localization-parity.md); execution order is ranked in [`gaps.md`](gaps.md).
