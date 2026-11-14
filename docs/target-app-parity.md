# Parity with GraphPad Prism 11

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-14 · **Change:** Wave 5–7 completion & Wave 8A roadmap conversion — closed `G5` (multi-engine Poppler + Ghostscript `%PDF-1.7` `/FontFile2` pixel QA across Latin, CJK & RTL), expanded equation library to `117` models (`47` pharma/binding incl. `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`), fixed all `6` Round-1 defects (`G2`, `G8`, `G9`, `G10`, `G11`, `G12`), added ECMA-234 `.emf` export + `.lumencraft.d/` unpacked format + all `QOL-1`–`QOL-11` across engine (`90` commands), `UiApp` (`29` `UI_COMMANDS`), CLI & `WebWorkspaceHost`, verified live macOS `pbcopy`/`pbpaste` (`EXECUTED_LIVE_ROUNDTRIP`) and macOS `plutil`/`codesign` bundle checks; remaining scope stays in [`ROADMAP.md`](ROADMAP.md) (`R5.4`, `R8.3`–`R8.5`, `36–58 h` to `1.0`) · **Target:** GraphPad Prism 11 (11.1.x; public guides & release notes through 11.1.0, August 2026)

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
| 10 | **Workspace UX, `.lumencraft`/`.lumencraft.d`/`.prism`/`.pzfx` I/O, templates, CLI, `--control` & MCP** | 5% | 78% | 20–33 | [ui-parity.md](ui-parity.md), [scripting-and-automation.md](scripting-and-automation.md) |
| | **Total / Weighted depth (engine, CLI/MCP, `UiApp` & `QOL-1`–`QOL-11`)** | **100%** | **~96%** | **20–33 h** | (`R8.3`–`R8.4`; plus `R5.4`/`G11` `2–3 h` **Beta** & `R8.5`/`G13`+`M7` `14–22 h` → **`36–58 h`** to `1.0`; see [`ROADMAP.md`](ROADMAP.md)) |

---

## Evidence: what ships today (`Waves 1–7` + `Slice 0` + `S1`/`S2` **GO** + `QOL-1`–`QOL-11`)

- **Workspace & dependency enforcement (`xtask/src/layers.rs`):** 18 library crates (`L0` `platform` added in Wave 8A alongside `math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`, `tools`, `engine`, `ui-egui`, `mcp`, `testkit`), 3 apps (`lumencraft`, `lumencraft-cli`, `lumencraft-web`), and `xtask` compile cleanly under Rust 2024 with `#![forbid(unsafe_code)]` and workspace-wide `deny` lints on `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable` (Wave 8A CI/gate wiring tracked in [`ROADMAP.md`](ROADMAP.md#r0-align-with-house-principles)).
- **Numerical foundations, StRD corpus & statistical/fitting engines (`crates/math`, `crates/stats`, `crates/fit`, `crates/transform`, `crates/testkit`):** `lumencraft-math` (`51` tests incl. doctest), `lumencraft-testkit` (`7` tests incl. doctest), and `cargo xtask corpus` (`58` NIST StRD `.dat` datasets) anchor `lumencraft-stats` (`69 passed`, including `d1_reference_oracles`, `d1_edge_matrix` checked against R 4.5.2, `parity_g8_barnard_pb_mi`, `parity_g3_g7`, `nist_strd`, `prop_stats`, and Wave 5 `G8` `koopman_bounds`), `lumencraft-fit` (`87 passed`, verifying `117` built-in models incl. `47` pharma/receptor-binding models with Wave 5 `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`, `43` curves, `27` NIST StRD datasets, Wave 5 `G9` `MAX_AST_NODES = 4_096` + iterative `Expr` `Clone`/`Debug`, and `< 50 ms` `96`-curve 4PL batch solving at `fit_ms: 22.669 ms` / `total_ms: 26.25 ms` `--release`), and `lumencraft-transform` (`24 passed`, `=` Calculated Variables `100k` rows in `13.841 ms` `--release`, derived transforms, plus Wave 5 `QOL-1` archetype conversion & `QOL-2` `96`/`384`-well plate + tidy long-to-wide reshape).
- **Document model, persistence & `.lumencraft.d/` / `.prism` / `.pzfx` interop (`crates/palette`, `crates/doc`, `crates/format`):** `lumencraft-palette` (`15 passed`), `lumencraft-doc` (`31 passed`: 8 `TableArchetype`s, 8 `SubcolumnMode`s, exclusions + `CellExclusionNote` `QOL-3`, `NamedCheckpoint` `QOL-11`, `FigureCanvasPreset` `QOL-8`, `InfoSheet`, `LinkedDataRegion`), and `lumencraft-format` (`51 passed` incl. `corrupt_archive_fuzz`, `recovery_and_interop_parity`, `CrashRecoveryJournal` WAL/torn-tail replay, `MIGRATION_STEPS`, `sanitize_circular_hooks`, Wave 5 `QOL-11` `.lumencraft.d/` unpacked directory I/O, and Wave 5 `G10` `MAX_IMPORT_CELLS = 10_000_000` enforced post-build on `allocated_cell_slots()`) deliver lossless `.lumencraft`/`.lumencraft.d` save/reopen, RFC-4180 CSV/TSV, and bounded clean-room `.prism`/`.pzfx` import.
- **Scientific text, chart compiler, vector PDF/SVG/`.emf` & software compositor (`crates/text`, `crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`):** `102 passed` across the visual stack (`chart` `40` incl. `QOL-5` `SourceCellId`/`PlottedPointMeta` & `QOL-8` `preflight_check_scene`, `text` `25` under default & `--no-default-features`, `pdf` `14` incl. Wave 5 `external_viewer_qa`, `svg` `11`, `render` `12` incl. Wave 5 `G11` `MAX_RASTER_PIXELS = 67_108_864`/`try_reserve_exact`, `QOL-9` `CanvasPreviewFilter`, and pure-Rust ECMA-234 `.emf`). **Risk Spike `S2` = GO** and **`G5` closed**: Wave 5 Poppler `pdftoppm` + Ghostscript `gs` pixel QA (`3 passed`) verified `%PDF-1.7` + `/FontFile2` rendering across Latin/Greek/ribbon, CJK, and RTL exports.
- **Tools, reactive engine, Slice 0 CLI & MCP automation (`crates/tools`, `crates/engine`, `crates/mcp`, `apps/lumencraft-cli`):** `lumencraft-tools` (`21 passed` incl. `PanZoomInertiaState`, rulers, `NavigatorDragState`), `lumencraft-engine` (`76 passed` covering all **`90`** registered engine commands, Wave 6 `G2` linked-file error propagation, Wave 6 `G12` read-only clone-skip & bounded `undo_stack`, `QOL-1`–`QOL-3`/`QOL-6`/`QOL-8`/`QOL-11` commands), `lumencraft-mcp` (`25 passed`, **`29`** synced `UI_COMMANDS`), and `lumencraft-cli` (`18 passed` incl. `Slice 0`, `perf --json`, `.lumencraft.d`, checkpoints, `.emf`).
- **UI shell, localization & hardware adapters (`crates/ui-egui`, `apps/lumencraft`, `apps/lumencraft-web`, `packaging/`, `xtask`):** `lumencraft-ui-egui` (`69 passed` across default & `--no-default-features`) ships `UiApp` + software RGBA/PNG compositor, Wave 7 HiDPI `ViewportMetrics` (`1.0x`–`3.0x`), `WorkspaceLayoutMap`, `UiInputEvent` dispatch, `InlineEditorState`, `VirtualWorkspaceFolder`, `SplitWorkspaceState` (`Cmd+\`), `presentation_mode` (`Cmd+Shift+P`), **`29`** synced `UI_COMMANDS` (`QOL-1`–`QOL-10`), `font_diagnostics`, **`16` locales × `167` keys = `2,672` translations (`0` missing)** with RTL mirroring, plus Wave 7 live macOS `/usr/bin/pbcopy`+`/usr/bin/pbpaste` `de-DE` comma/`=A1*2`/`emf_bytes` roundtrip (`macos_clipboard_live`, `EXECUTED_LIVE_ROUNDTRIP`). **Risk Spike `S1` = GO** (`0.0390 ms` tools / `0.061 ms` UI p50 at `1M` cells). `apps/lumencraft` (`9 passed` incl. `--control` JSONL loopback, `--portable-dir`, subprocess pasteboard/print adapters), `apps/lumencraft-web` (`5 passed`, `WebWorkspaceHost` 100% local in-memory viewer/editor), and `xtask` (`27 passed`, incl. macOS `plutil -lint` & ad-hoc `codesign --verify --strict --deep` in `bundle`) complete headless & macOS verification. **Remaining release scope:** live native GPU `eframe`/`egui`/`wgpu` desktop windowing & pointer/inline OS bindings (`R8.3`), `wasm32-unknown-unknown` sysroot & `trunk` browser hosting (`R8.4`), rich native binary OS PDF/EMF clipboard IPC & print dialogs (`R5.4`), and cross-OS desktop CJK/RTL QA & signed notarized installers (`R8.5`).

## Missing by area (summary)

Detailed item-by-item checkboxes live in [`stats-parity.md`](stats-parity.md), [`fitting-parity.md`](fitting-parity.md), [`graphing-parity.md`](graphing-parity.md), [`ui-parity.md`](ui-parity.md), [`gui-workflows.md`](gui-workflows.md), [`file-format-parity.md`](file-format-parity.md), [`hardware-parity.md`](hardware-parity.md), [`localization-parity.md`](localization-parity.md), [`qol-beyond-parity.md`](qol-beyond-parity.md), and [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md); remaining backlog is tracked by ID in [`ROADMAP.md`](ROADMAP.md) and [`STATUS.md`](STATUS.md).
