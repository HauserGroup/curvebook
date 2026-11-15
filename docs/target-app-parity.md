# Parity with GraphPad Prism 11

> **Last reviewed:** 2026-11-15 · **Last updated:** 2026-11-15 · **Change:** Updated through Waves 20–24 + `R8.3`–`R8.4` (`eframe 0.37` / `egui 0.37` / `wgpu` native desktop window `gui_window.rs`, `wasm32` + Trunk WebGL2 bundle `lumencraft-web-0.1.0.zip`, `R5.4` native clipboard/print/layout drag, `R8.6`–`R8.8` & `R9.17` `49`-workflow `gui_only` parity, active `R9` QoL suite, `95` engine commands / `45` `UI_COMMANDS`, `16 × 307` i18n, and local universal macOS `.dmg`/`.zip` `verify.sh` + `THIRD-PARTY-LICENSES.html` complete; external hosted CI runners, Apple Developer ID notarization, & Windows/Linux/FreeBSD VM QA stay in `R8.5`/`R0.6`/`R0.14`) · **Target:** GraphPad Prism 11 (11.1.x; public guides & release notes through 11.1.0, August 2026)

Authoritative clean-room parity assessment for LumenCraft. Active task queue in [`ROADMAP.md`](ROADMAP.md) and [`STATUS.md`](STATUS.md); legacy mapping stubs in [`gaps.md`](gaps.md), [`ROADMAP.md#legacy-milestone--gate-mapping`](ROADMAP.md#legacy-milestone--gate-mapping), and [`roadmap-history.md`](roadmap-history.md).

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
| 1 | **Typed data tables (8 archetypes), exclusions & Info constants** | 14% | 98% | 0 | [data-tables-and-transforms.md](data-tables-and-transforms.md), [file-format-parity.md](file-format-parity.md) |
| 2 | **Calculated variables (`=`), window functions & derived transforms** | 8% | 98% | 0 | [data-tables-and-transforms.md](data-tables-and-transforms.md) |
| 3 | **Descriptive statistics, normality/lognormality & Grubbs/ROUT outliers** | 8% | 98% | 0 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 4 | **Comparisons: *t*-tests, 1/2/3-way & Multifactor ANOVA, RM, REML & multiplicity** | 16% | 96% | 0 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 5 | **Nonlinear regression core: LM solver, weights, Cauchy/ROUT, global fits, profile CIs, AICc/*F*** | 14% | 96% | 0 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 6 | **Built-in equation library (dose–response, binding, kinetics, growth) & custom ODE/DSL** | 10% | 98% | 0 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 7 | **Contingency, Kaplan–Meier/Cox survival, ROC, multiple GLMs, PCA & clustering** | 10% | 98% | 0 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 8 | **Publication graphing: 8 graph families, nonlinear/broken axes, live *p*-value brackets** | 10% | 96% | 0 | [graphing-and-layouts.md](graphing-and-layouts.md), [graphing-parity.md](graphing-parity.md) |
| 9 | **Page Layouts, style cloning ("Wand"), vector SVG/PDF, `.emf` & TIFF/PNG export** | 5% | 98% | 0 | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) |
| 10 | **Workspace UX, `.lumencraft`/`.lumencraft.d`/`.prism`/`.pzfx` I/O, templates, CLI, `--control` & MCP** | 5% | 98% | 0 | [ui-parity.md](ui-parity.md), [scripting-and-automation.md](scripting-and-automation.md) |
| | **Total / Weighted depth (`eframe`/`wgpu` desktop, `wasm32` WebGL2, engine, CLI/MCP, `UiApp` & active `R9` QoL)** | **100%** | **~97%** | **0 h** | (`R8.3`, `R8.4`, `R5.4`, `R8.6`–`R8.8`, `R9.17`, & local universal macOS `R8.5` done; hosted CI/notarization/cross-OS VMs open in [`ROADMAP.md`](ROADMAP.md)) |

---

## Evidence: what ships today (`Waves 1–7` + `Slice 0` + `S1`/`S2` **GO** + `QOL-1`–`QOL-11`)

- **Workspace & dependency enforcement (`xtask/src/layers.rs`):** 18 library crates (`L0` `platform` added in Wave 8A alongside `math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`, `tools`, `engine`, `ui-egui`, `mcp`, `testkit`), 3 apps (`lumencraft`, `lumencraft-cli`, `lumencraft-web`), and `xtask` compile cleanly under Rust 2024 with `#![forbid(unsafe_code)]` and workspace-wide `deny` lints on `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable` (Wave 8A CI/gate wiring tracked in [`ROADMAP.md`](ROADMAP.md#r0-align-with-house-principles)).
- **Numerical foundations, StRD corpus & statistical/fitting engines (`crates/math`, `crates/stats`, `crates/fit`, `crates/transform`, `crates/testkit`):** `lumencraft-math` (`51` tests incl. doctest), `lumencraft-testkit` (`7` tests incl. doctest), and `cargo xtask corpus` (`58` NIST StRD `.dat` datasets) anchor `lumencraft-stats` (`69 passed`, including `d1_reference_oracles`, `d1_edge_matrix` checked against R 4.5.2, `parity_g8_barnard_pb_mi`, `parity_g3_g7`, `nist_strd`, `prop_stats`, and Wave 5 `G8` `koopman_bounds`), `lumencraft-fit` (`87 passed`, verifying `117` built-in models incl. `47` pharma/receptor-binding models with Wave 5 `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`, `43` curves, `27` NIST StRD datasets, Wave 5 `G9` `MAX_AST_NODES = 4_096` + iterative `Expr` `Clone`/`Debug`, and `< 50 ms` `96`-curve 4PL batch solving at `fit_ms: 22.669 ms` / `total_ms: 26.25 ms` `--release`), and `lumencraft-transform` (`24 passed`, `=` Calculated Variables `100k` rows in `13.841 ms` `--release`, derived transforms, plus Wave 5 `QOL-1` archetype conversion & `QOL-2` `96`/`384`-well plate + tidy long-to-wide reshape).
- **Document model, persistence & `.lumencraft.d/` / `.prism` / `.pzfx` interop (`crates/palette`, `crates/doc`, `crates/format`):** `lumencraft-palette` (`15 passed`), `lumencraft-doc` (`31 passed`: 8 `TableArchetype`s, 8 `SubcolumnMode`s, exclusions + `CellExclusionNote` `QOL-3`, `NamedCheckpoint` `QOL-11`, `FigureCanvasPreset` `QOL-8`, `InfoSheet`, `LinkedDataRegion`), and `lumencraft-format` (`51 passed` incl. `corrupt_archive_fuzz`, `recovery_and_interop_parity`, `CrashRecoveryJournal` WAL/torn-tail replay, `MIGRATION_STEPS`, `sanitize_circular_hooks`, Wave 5 `QOL-11` `.lumencraft.d/` unpacked directory I/O, and Wave 5 `G10` `MAX_IMPORT_CELLS = 10_000_000` enforced post-build on `allocated_cell_slots()`) deliver lossless `.lumencraft`/`.lumencraft.d` save/reopen, RFC-4180 CSV/TSV, and bounded clean-room `.prism`/`.pzfx` import.
- **Scientific text, chart compiler, vector PDF/SVG/`.emf` & software compositor (`crates/text`, `crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`):** `102 passed` across the visual stack (`chart` `40` incl. `QOL-5` `SourceCellId`/`PlottedPointMeta` & `QOL-8` `preflight_check_scene`, `text` `25` under default & `--no-default-features`, `pdf` `14` incl. Wave 5 `external_viewer_qa`, `svg` `11`, `render` `12` incl. Wave 5 `G11` `MAX_RASTER_PIXELS = 67_108_864`/`try_reserve_exact`, `QOL-9` `CanvasPreviewFilter`, and pure-Rust ECMA-234 `.emf`). **Risk Spike `S2` = GO** and **`G5` closed**: Wave 5 Poppler `pdftoppm` + Ghostscript `gs` pixel QA (`3 passed`) verified `%PDF-1.7` + `/FontFile2` rendering across Latin/Greek/ribbon, CJK, and RTL exports.
- **Tools, reactive engine, Slice 0 CLI & MCP automation (`crates/tools`, `crates/engine`, `crates/mcp`, `apps/lumencraft-cli`):** `lumencraft-tools` (`21 passed` incl. `PanZoomInertiaState`, rulers, `NavigatorDragState`), `lumencraft-engine` (`76 passed` covering all **`90`** registered engine commands, Wave 6 `G2` linked-file error propagation, Wave 6 `G12` read-only clone-skip & bounded `undo_stack`, `QOL-1`–`QOL-3`/`QOL-6`/`QOL-8`/`QOL-11` commands), `lumencraft-mcp` (`25 passed`, **`29`** synced `UI_COMMANDS`), and `lumencraft-cli` (`18 passed` incl. `Slice 0`, `perf --json`, `.lumencraft.d`, checkpoints, `.emf`).
- **UI shell, native desktop GPU window, WASM WebGL2 bundle, localization & hardware adapters (`crates/ui-egui`, `apps/lumencraft`, `apps/lumencraft-web`, `packaging/`, `xtask`):** `lumencraft-ui-egui` (`155 passed` across `49` `gui_wf_*.rs` workflows, `16 × 307 = 4,912` translations with RTL mirroring, `45` synced `UI_COMMANDS`, `R5.4` live `NSPasteboard` `%PDF-1.7` roundtrip, `cjk_rtl_hidpi_desktop_qa`, & `1440x900` Light/Dark screenshots) pairs with live native GPU desktop windowing (`apps/lumencraft/src/gui_window.rs` over `eframe 0.37` / `egui 0.37` / `wgpu` + `rfd 0.17`, `23 passed` in `lumencraft`) and the `wasm32-unknown-unknown` `+simd128` Trunk WebGL2 bundle (`apps/lumencraft-web` `connect_wasm_bridge` + WebGL2/2D canvas presenter + `lumencraft-web-0.1.0.zip`, `13 passed`), alongside universal macOS (`aarch64` + `x86_64`) `.dmg`/`.zip` `package.sh` + `verify.sh` + `THIRD-PARTY-LICENSES.html` (`xtask` `55 passed`). Remaining release work is strictly external CI/VM/notarization QA (`R8.5`, `R0.6`, `R0.14`).

## Missing by area (summary)

Detailed item-by-item checkboxes live in [`stats-parity.md`](stats-parity.md), [`fitting-parity.md`](fitting-parity.md), [`graphing-parity.md`](graphing-parity.md), [`ui-parity.md`](ui-parity.md), [`gui-workflows.md`](gui-workflows.md), [`file-format-parity.md`](file-format-parity.md), [`hardware-parity.md`](hardware-parity.md), [`localization-parity.md`](localization-parity.md), [`qol-beyond-parity.md`](qol-beyond-parity.md), and [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md); remaining backlog is tracked by ID in [`ROADMAP.md`](ROADMAP.md) and [`STATUS.md`](STATUS.md).
