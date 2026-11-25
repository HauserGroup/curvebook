# Capability Evidence Overview

> **Last reviewed:** 2026-11-24 · **Last updated:** 2026-11-24 · **Change:** Updated through `R15` (`R15.1`–`R15.10` declarative `egui 0.37` desktop shell & OKLab/OKLCH design language), `R10.25`–`R10.28` (`65 / 65` five-stage desktop GUI workflow completion), `R13` (`R13.1`–`R13.10` composite graphing, embedded tables/images & layout superposition), `R17` (`R17.1`–`R17.5` `.xlsx` import, export interop, idle-power/perf, MCP compaction & legacy `.pzfx`/`.prism` hardening), and `R18` Wave 1 (`151` built-in fit models, custom DSL builtins/`[InitialValues]`, linear/Deming extensions) · **Target:** Scientific biostatistics, nonlinear regression, and publication-graphing workbench parity

Authoritative clean-room capability assessment for CurveBook. Active task queue in [`ROADMAP.md`](../ROADMAP.md) and [`STATUS.md`](../STATUS.md); legacy mapping stubs in [`gaps.md`](../history/legacy-ids-and-defects.md), [`ROADMAP.md#legacy-milestone--gate-mapping`](../ROADMAP.md#legacy-milestone--gate-mapping), and [`roadmap-history.md`](../history/roadmap.md).

## Methodology

Every score in this document is graded strictly from shipped, tested code in `crates/*` and `apps/*` against peer-reviewed biostatistical and numerical methods, primary literature specifications (Motulsky & Christopoulos, *Fitting Models to Biological Data*; Zar, *Biostatistical Analysis*; Klein & Moeschberger, *Survival Analysis*; Wilkinson, *The Grammar of Graphics*), and certified reference benchmarks (NIST StRD and R `4.5.2` independent oracles). In accordance with [`AGENTS.md`](../../AGENTS.md), no proprietary binary, installation directory, or vendor-created sample file is ever inspected or run.

The [limited local JPG reference policy](../../AGENTS.md#limited-local-jpg-reference-policy) allows the designated existing frames to inform high-level workflows, information hierarchy, and generic functional organization while requiring independent implementation and original visual design. Frames provide no numerical reference answers, visual matching targets, acceptance baselines, or evidence of workflow completion. Scores and completion claims still require the shipped-code and test evidence described here.

### Scoring rubric (per feature area)

| Score | Meaning |
|---:|---|
| `0%` | Absent or M0 crate stub only. |
| `10–30%` | Core data types and happy-path math exist headlessly with unit tests, not yet wired through engine commands or UI. |
| `40–65%` | Mainstream variants work end to end (engine + CLI/MCP + UI + `.curvebook` save/reopen) and pass NIST/analytical tests; secondary options or diagnostics missing. |
| `70–85%` | Everyday wet-lab researcher workflow complete, including missing-value edge cases, multiplicity CIs, plain-language checklists, and vector SVG/PDF export. |
| `90–100%` | Indistinguishable mathematical precision and workflow completeness against target biostatistics and nonlinear-regression functional specifications, property-tested against hostile inputs. |

### Ready for real work

`Feature depth` weights the 10 feature areas below by scientific frequency of use. `Ready for real work` discounts feature depth by workflow-blocking multipliers:
1. Lossless native `.curvebook` + CSV/TSV + legacy `.pzfx`/`.prism` project interchange,
2. Interactive spreadsheet ergonomics (subcolumns, paste-transpose, non-destructive exclusions, undo/redo),
3. Certified numerical accuracy on extreme degrees of freedom, tied ranks, missing repeated measures, and stiff nonlinear fits.

### Calibration

Hour ranges are single-agent wall-clock hours calibrated from comparable clean-room Rust workspaces (`~1.0–2.0 h` per self-contained statistical test or equation family with NIST/analytical tests + command wiring; `~6–12 h` per multi-tab dialog or chart family). Parallel agents (`4–6`) achieve `~3.2–3.8×` wall-clock speedup when partitioned cleanly across `math`/`stats`, `fit`, `doc`/`transform`/`format`, `chart`/`svg`/`pdf`, and `ui-egui`.

---

## Feature areas

| # | Area | Weight | Score | Est. remaining (h) | Spec & checklist |
|---:|---|---:|---:|---:|---|
| 1 | **Typed data tables (8 archetypes), exclusions & Info constants** | 14% | 98% | 0 | [data-tables-and-transforms.md](../implementation/data-tables-and-transforms.md), [file-formats.md](file-formats.md) |
| 2 | **Calculated variables (`=`), window functions & derived transforms** | 8% | 98% | 0 | [data-tables-and-transforms.md](../implementation/data-tables-and-transforms.md) |
| 3 | **Descriptive statistics, normality/lognormality & Grubbs/ROUT outliers** | 8% | 98% | 0 | [statistics-implementation.md](../implementation/statistics-implementation.md), [statistics.md](statistics.md) |
| 4 | **Comparisons: *t*-tests, 1/2/3-way & Multifactor ANOVA, RM, REML & multiplicity** | 16% | 96% | 0 | [statistics-implementation.md](../implementation/statistics-implementation.md), [statistics.md](statistics.md) |
| 5 | **Nonlinear regression core: LM solver, weights, Cauchy/ROUT, global fits, profile CIs, AICc/*F*** | 14% | 96% | 0 | [curve-fitting-implementation.md](../implementation/curve-fitting-implementation.md), [curve-fitting.md](curve-fitting.md) |
| 6 | **Built-in equation library (dose–response, binding, kinetics, growth) & custom ODE/DSL** | 10% | 98% | 0 | [curve-fitting-implementation.md](../implementation/curve-fitting-implementation.md), [curve-fitting.md](curve-fitting.md) |
| 7 | **Contingency, Kaplan–Meier/Cox survival, ROC, multiple GLMs, PCA & clustering** | 10% | 98% | 0 | [statistics-implementation.md](../implementation/statistics-implementation.md), [statistics.md](statistics.md) |
| 8 | **Publication graphing: 8 graph families, nonlinear/broken axes, live *p*-value brackets** | 10% | 96% | 0 | [graphing-and-layouts.md](../implementation/graphing-and-layouts.md), [graphing.md](graphing.md) |
| 9 | **Page Layouts, style cloning ("Wand"), vector SVG/PDF, `.emf` & TIFF/PNG export** | 5% | 98% | 0 | [graphing-and-layouts.md](../implementation/graphing-and-layouts.md), [file-formats.md](file-formats.md) |
| 10 | **Workspace UX, `.curvebook`/`.curvebook.d`/`.xlsx`/`.prism`/`.pzfx` I/O, templates, CLI, `--control` & MCP** | 5% | 94% | `1.0` QA & `R11` web | [user-interface.md](user-interface.md), [scripting-and-automation.md](../implementation/scripting-and-automation.md) — declarative `egui 0.37` desktop GUI (`R15`) passes `65 / 65` five-stage `WorkflowRun` suites (`R10.25`); recorded walkthroughs (`R10.13`, `R10.24`) and `WebRunner` browser rebuild (`R11`) target `1.0` |
| | **Total / Weighted depth (`eframe`/`egui 0.37` desktop, engine, CLI/MCP & active `R9`/`R13`/`R17` scope)** | **100%** | **~97%** | **`1.0` QA & `R11` web** | (`R15`, `R13`, `R10.25`, & local `R17` done; hosted CI/rehearsal release & `1.0` walkthroughs/browser open in [`ROADMAP.md`](../ROADMAP.md)) |

> **Human-GUI status (`R15` & `R10.25` done for Beta; `1.0` walkthroughs open).** All `65` reference GUI workflows (`WF-01`–`WF-65`) pass the five-stage `WorkflowRun` suites (`human_wf_01_06`–`human_wf_59_65`) through rendered `egui` desktop widgets (`65 / 65`, `0` failed/skipped/panics; [`gui-workflows.md` §6](../gui-workflows.md#6-human-path-status--blockers-r10)). Recorded computer-use or manual human walkthroughs on macOS desktop and browser ([`R10.13`](../ROADMAP.md#r1013-human-operable-50-workflow-acceptance--manual-qa)), Windows/Linux smoke QA ([`R10.24`](../ROADMAP.md#r1024-windows--linux-desktop-gui-smoke-qa-10-gate)), and the `eframe::WebRunner` browser rebuild ([`R11`](../ROADMAP.md#r11-usable-browser-app)) remain **1.0** requirements.

---

## Evidence: what ships today (`Waves 1–7` + `Slice 0` + `S1`/`S2` **GO** + `QOL-1`–`QOL-11`)

Per-crate test counts, command totals, and locale/key totals are not repeated here; current verified numbers live only in [`STATUS.md`](../STATUS.md) (`cargo xtask stats --exact`, `cargo xtask commands --check`, `cargo xtask i18n-report`).

- **Workspace & dependency enforcement (`xtask/src/layers.rs`):** library crates (`L0` `platform` added in Wave 8A alongside `math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`, `tools`, `engine`, `ui-egui`, `mcp`, `testkit`), 3 apps (`curvebook`, `curvebook-cli`, `curvebook-web`), and `xtask` compile cleanly under Rust 2024 with `#![forbid(unsafe_code)]` and workspace-wide `deny` lints on `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable` (Wave 8A CI/gate wiring tracked in [`ROADMAP.md`](../ROADMAP.md#r0-align-with-house-principles)).
- **Numerical foundations, StRD corpus & statistical/fitting engines (`crates/math`, `crates/stats`, `crates/fit`, `crates/transform`, `crates/testkit`):** `curvebook-math`, `curvebook-testkit`, and `cargo xtask corpus` (`58` NIST StRD `.dat` datasets) anchor `curvebook-stats` (including `d1_reference_oracles`, `d1_edge_matrix` checked against R 4.5.2, `parity_g8_barnard_pb_mi`, `parity_g3_g7`, `nist_strd`, `prop_stats`, and Wave 5 `G8` Koopman non-bracketing unit tests in `crates/stats/src/categorical/ratios_nnt.rs`), `curvebook-fit` (verifying `117` built-in models incl. `47` pharma/receptor-binding models with Wave 5 `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`, `43` curves, `27` NIST StRD datasets, Wave 5 `G9` `MAX_AST_NODES = 4_096` + iterative `Expr` `Clone`/`Debug`, and `< 50 ms` `96`-curve 4PL batch solving at `fit_ms: 22.669 ms` / `total_ms: 26.25 ms` `--release`), and `curvebook-transform` (`=` Calculated Variables `100k` rows in `13.841 ms` `--release`, derived transforms, plus Wave 5 `QOL-1` archetype conversion & `QOL-2` `96`/`384`-well plate + tidy long-to-wide reshape).
- **Document model, persistence & `.curvebook.d/` / `.prism` / `.pzfx` interop (`crates/palette`, `crates/doc`, `crates/format`):** `curvebook-palette`, `curvebook-doc` (8 `TableArchetype`s, 8 `SubcolumnMode`s, exclusions + `CellExclusionNote` `QOL-3`, `NamedCheckpoint` `QOL-11`, `FigureCanvasPreset` `QOL-8`, `InfoSheet`, `LinkedDataRegion`), and `curvebook-format` (incl. `corrupt_archive_fuzz`, `recovery_and_interop_parity`, `CrashRecoveryJournal` WAL/torn-tail replay, `MIGRATION_STEPS`, `sanitize_circular_hooks`, Wave 5 `QOL-11` `.curvebook.d/` unpacked directory I/O, and Wave 5 `G10` `MAX_IMPORT_CELLS = 10_000_000` enforced post-build on `allocated_cell_slots()`) deliver lossless `.curvebook`/`.curvebook.d` save/reopen, RFC-4180 CSV/TSV, and bounded clean-room `.prism`/`.pzfx` import.
- **Scientific text, chart compiler, vector PDF/SVG/`.emf` & software compositor (`crates/text`, `crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`):** tests cover the whole visual stack (`chart` incl. `QOL-5` `SourceCellId`/`PlottedPointMeta` & `QOL-8` `preflight_check_scene`, `text` under default & `--no-default-features`, `pdf` incl. Wave 5 `external_viewer_qa`, `svg`, `render` incl. Wave 5 `G11` `MAX_RASTER_PIXELS = 67_108_864`/`try_reserve_exact`, `QOL-9` `CanvasPreviewFilter`, and pure-Rust ECMA-234 `.emf`). **Risk Spike `S2` = GO** and **`G5` closed**: Wave 5 Poppler `pdftoppm` + Ghostscript `gs` pixel QA (`3 passed`) verified `%PDF-1.7` + `/FontFile2` rendering across Latin/Greek/ribbon, CJK, and RTL exports.
- **Tools, reactive engine, Slice 0 CLI & MCP automation (`crates/tools`, `crates/engine`, `crates/mcp`, `apps/curvebook-cli`):** `curvebook-tools` (incl. `PanZoomInertiaState`, rulers, `NavigatorDragState`), `curvebook-engine` (covering every registered `cmd::registry()` command in [`commands.md`](../commands.md), Wave 6 `G2` linked-file error propagation, Wave 6 `G12` read-only clone-skip & bounded `undo_stack`, `QOL-1`–`QOL-3`/`QOL-6`/`QOL-8`/`QOL-11` commands), `curvebook-mcp` (generated `ui_catalog.rs` synced from `UI_COMMANDS`), and `curvebook-cli` (incl. `Slice 0`, `perf --json`, `.curvebook.d`, checkpoints, `.emf`).
- **UI shell, native `egui` desktop host, WASM placeholder, localization & hardware adapters (`crates/ui-egui`, `apps/curvebook`, `apps/curvebook-web`, `packaging/`, `xtask`):** `curvebook-ui-egui` (`egui_shell`, `65 / 65` five-stage `human_wf_*.rs` desktop workflows, `16`-locale `catalog.tsv` translations, synced `UI_COMMANDS`, live `NSPasteboard` `%PDF-1.7` roundtrip, `cjk_rtl_hidpi_desktop_qa`, & `1440x900` screenshots) pairs with the native desktop host (`apps/curvebook/src/gui_window.rs` over `eframe 0.37` / `egui 0.37` / `wgpu` + `rfd 0.18` + `muda` macOS menus) and the strict-CSP `wasm32-unknown-unknown` placeholder (`apps/curvebook-web`, awaiting `R11.11` `eframe::WebRunner`), alongside universal macOS `.dmg` packaging + `verify.sh` + `THIRD-PARTY-LICENSES.html`. Remaining release work is hosted Beta CI/rehearsal (`R14.1`) plus `1.0` walkthroughs, browser rebuild, and cross-OS VM/signing QA (`R10.13`, `R10.24`, `R11`, `R8.5`, `R8.11`).

## Missing by area (summary)

Detailed item-by-item checkboxes live in [`statistics.md`](statistics.md), [`curve-fitting.md`](curve-fitting.md), [`graphing.md`](graphing.md), [`user-interface.md`](user-interface.md), [`gui-workflows.md`](../gui-workflows.md), [`file-formats.md`](file-formats.md), [`platforms.md`](platforms.md), [`localization.md`](localization.md), [`qol-enhancements.md`](../planning/qol-enhancements.md), and [`calculators-and-quick-stats.md`](../planning/calculators-and-quick-stats.md); remaining backlog is tracked by ID in [`ROADMAP.md`](../ROADMAP.md) and [`STATUS.md`](../STATUS.md).
