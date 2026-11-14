# LumenCraft Status

> **Updated:** 2026-11-14 (Waves 5–11 verification + `R0` typed architecture & B5 codegen) · **Roadmap:** [`ROADMAP.md`](ROADMAP.md) · **Parity overview:** [`target-app-parity.md`](target-app-parity.md)

This document summarizes what works in LumenCraft today, the checks that verified it, and the honest limits of the current offline workspace. Every Markdown table below uses unique Column 1 keys (`cargo xtask status` rule).

---

## Verified Capabilities & Headline Numbers

| Capability area | Verified state | Evidence & location |
|---|---|---|
| **Engine command registry** | **`90`** typed `CommandSpec` commands across `9` `CommandGroup` categories (`cmd::registry()`, `Effects`, JSON Schemas, `serde_path_to_error` typed params, bounded `64`-entry undo) | [`crates/engine/src/cmd/mod.rs`](../crates/engine/src/cmd/mod.rs), [`commands.md`](commands.md), [`command-ids.md`](command-ids.md), `cargo xtask commands --check`, `lumencraft-engine` **`115 passed, 0 failed`** |
| **UI command catalog** | **`29`** typed `UiCommandSpec` entries mirrored cleanly to MCP (`ui.event.dispatch`, `ui.split.toggle`, `ui.brush.point`, `ui.result.copyMethodsReport`, `ui.result.applyRemediation`, `ui.canvas.applyPreset`, `ui.canvas.preflight`, `ui.previewFilter.set`, `ui.presentation.toggle`, `ui.grid.excludeWithReason`, `ui.table.convertArchetype.open`, `ui.import.reshape.open`, etc.) | [`crates/ui-egui/src/menus/ui_commands.rs`](../crates/ui-egui/src/menus/ui_commands.rs), [`crates/mcp/src/ui_catalog.rs`](../crates/mcp/src/ui_catalog.rs), `cargo xtask ui-commands --check` |
| **Built-in fit equation library** | **`117`** built-in parametric models (`47` pharmacology/dose–response/receptor-binding/enzyme-kinetics incl. `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`, `43` curve/growth/peak/wave/line/polynomial models, `27` NIST StRD nonlinear models) + custom algebraic/implicit/ODE DSL (`MAX_AST_NODES = 4_096`) | [`crates/fit/src/library/mod.rs`](../crates/fit/src/library/mod.rs), [`fitting-parity.md`](fitting-parity.md), `lumencraft-fit` **`87 passed, 0 failed`** |
| **Statistical test & oracle suite** | Parametric/nonparametric tests, 1/2/3-way & Multifactor ANOVA, REML, multiplicity, survival/Cox PH, contingency (`BarnardExact`, Baptista–Pike OR, Koopman score RR returning `None` on non-bracketing/non-finite endpoints), `BlandAltman`, `PassingBablok`, ROC, GLMs, PCA, MICE + Rubin pooling; `58` NIST StRD datasets + `D1` R `4.5.2` oracle/edge matrices | [`stats-parity.md`](stats-parity.md), `lumencraft-stats` **`69 passed, 0 failed`**, `cargo xtask corpus` |
| **Data tables, persistence & interop** | `8` table archetypes, `8` subcolumns, `=` Calculated Variables, `.lumencraft`/`.lctemplate`, versioned WAL `CrashRecoveryJournal`, `.lumencraft.d/` unpacked Git-friendly directory, `NamedCheckpoint`, `CellExclusionNote`, `versioned.rs` upgrade + forward refusal, `FormatRegistry`, clean-room `.prism`/`.pzfx` import | [`file-format-parity.md`](file-format-parity.md), [`file-formats.md`](file-formats.md), `docs/schemas/*.schema.json` (`cargo xtask schemas --check`), `lumencraft-doc` **`31 passed`**, `lumencraft-transform` **`24 passed`**, `lumencraft-format` **`51 passed`** |
| **Publication graphing & export** | `8` graph families + diagnostic plots, `7` axis scales, broken axes, live brackets, SVG, `%PDF-1.7` + optional `/FontFile2` SFNT embedding + Core-14 fallback, PNG/Deflate/LZW TIFF (`MAX_RASTER_PIXELS = 67_108_864`), ECMA-234 `.emf` vector export (`encode_emf`), `PlottedPointMeta` brushing, `FigureCanvasPreset` & `preflight_check_scene` | [`graphing-parity.md`](graphing-parity.md), `lumencraft-chart` **`40 passed`**, `lumencraft-render` **`12 passed`**, `lumencraft-pdf` **`14 passed`** |
| **Localization & accessibility** | **`16`** locales × **`167`** keys = **`2,672`** translations (`0` missing), RTL `ar`/`he` BiDi shaping & UI layout mirroring, `CanvasPreviewFilter` CVD/grayscale preview, WCAG AA theme contrast | [`localization-parity.md`](localization-parity.md), `cargo xtask i18n-report` |
| **UI shell, CLI, `--control`, MCP & Web** | Pure-Rust `UiApp` + software RGBA/PNG compositor, HiDPI `1.0x`–`3.0x` & LTR/RTL `UiInputEvent` dispatch, `VirtualWorkspaceFolder`, `WebWorkspaceHost`, `lumencraft-cli` (`commands`, typed flags, `mcp --control-port`, batch, `.emf`, `.lumencraft.d`), `lumencraft --control`, `lumencraft-mcp` (`ToolAnnotations`, `CommandGroup` progressive toolsets, `sheet://`/`result://` resources, `WorkdirGuard`, `run_guarded`) | [`ui-parity.md`](ui-parity.md), [`hardware-parity.md`](hardware-parity.md), `lumencraft-ui-egui` **`69 passed`** (default & `--no-default-features`), `lumencraft-mcp` **`25 passed`**, `lumencraft-cli` **`18 passed`**, `lumencraft` **`9 passed`**, `lumencraft-web` **`5 passed`** |
| **Platform, `R0` alignment & codegen** | `lumencraft-platform` (`L0`, `#![forbid(unsafe_code)]`, wasm-safe) atomic writes, deadline-bounded `ProcessCommand` & `WorkdirGuard`/`PathResolver`; governance docs; `deny.toml`, `about.*`, `clippy.toml`, `.githooks/pre-commit`, `xtask` gates (`commands`, `schemas`, `ui-commands`, `layers`, `secrets`, `pending`, `status`, `licenses`, `assets`, `brands`, `cleanroom`) | [`crates/platform/src/lib.rs`](../crates/platform/src/lib.rs), [`docs/ROADMAP.md`](ROADMAP.md#r0-align-with-house-principles), `lumencraft-platform` **`11 passed, 0 failed`**, `xtask` **`42 passed, 0 failed`** |

---

## Quality-of-Life Suite (`QOL-1`–`QOL-11`) Status

All `11` QoL workflows ([`qol-beyond-parity.md`](qol-beyond-parity.md)) are implemented and tested across `doc`, domain crates, `engine`, `ui-egui`, `lumencraft-cli`, and `WebWorkspaceHost`:

| QoL ID | Roadmap ID | Verified implementation |
|---|---|---|
| `QOL-1` | `R9.1` | `table.convertArchetype` + `ConvertArchetypeDialogState` + `exclusion_notes` coordinate remapping & lossy guard |
| `QOL-2` | `R9.2` | `table.importPlateMap` (`96`/`384`-well + `.lctemplate` JSON) & `table.importTidyLong` + `ImportReshapeMode` |
| `QOL-3` | `R9.3` | `CellExclusionNote` + `table.excludeCellsWithReason` + `table.exclusionAuditReport` + `ExcludeWithReasonDialogState` (`Cmd+Shift+E`) |
| `QOL-4` | `R9.4` | `SplitWorkspaceState` (`ui.split.toggle`, `Cmd+\`) & split-divider drag in `crates/ui-egui` |
| `QOL-5` | `R9.5` | `PlottedPointMeta` / `ChartScene::hit_test_point` across all chart kinds + `ui.brush.point` |
| `QOL-6` | `R9.6` | `generate_methods_report` (`PlainText`/`Markdown`/`Latex`), `result.generateReport`, and `ui.result.copyMethodsReport` |
| `QOL-7` | `R9.7` | `AssumptionDiagnosticBanner` & `ui.result.applyRemediation` (non-destructive sibling analysis creation) |
| `QOL-8` | `R9.8` | `FigureCanvasPreset`, `CanvasResizeMode`, `preflight_check_scene`, `graph/layout.applyCanvasPreset`, `graph.preflightCheck` |
| `QOL-9` | `R9.9` | `CanvasPreviewFilter`, `apply_canvas_preview_filter`, and `ui.previewFilter.set` (non-mutating viewport preview) |
| `QOL-10` | `R9.10` | `VirtualWorkspaceFolder`, `presentation_mode` (`Cmd+Shift+P`), and `WebWorkspaceHost` in `apps/lumencraft-web` |
| `QOL-11` | `R9.11` | `.lumencraft.d/` unpacked directory I/O (`save_project_unpacked_dir`/`open_project_unpacked_dir`) + `NamedCheckpoint` commands & CLI/desktop auto-detection |

---

## Wave 5–7 Targeted Verification Results

These observed runs verified the Wave 5–7 changes after final edits under `--offline` mode (`0` Clippy warnings across all verified crates):

| Check / Suite | Scope | Observed result |
|---|---|---|
| **Wave 5 Verifier A** | `lumencraft-{palette,doc,chart,stats,fit}` | **`242 passed`** (`240` unit/integ + `2` doctests), `0 failed`: `palette` `15`, `doc` `31`, `chart` `40`, `stats` `69`, `fit` `87`; `clippy -D warnings` exit `0` |
| **Wave 5 Verifier B** | `lumencraft-{transform,format,render,pdf}` | **`101 passed`**, `0 failed`: `transform` `24`, `format` `51`, `render` `12`, `pdf` `14`; `clippy -D warnings` exit `0` |
| **Poppler + Ghostscript PDF 1.7 QA** | `crates/pdf/tests/external_viewer_qa.rs` | Executed live `pdftoppm` and `gs` (`1050x750` PPM) on `sci` (Latin/Greek/translucent ribbon/crop marks), `cjk` (`ja`/`zh-CN`/`ko` `/FontFile2`), and `rtl` (`ar`/`he`); `3 passed, 0 skipped`, `0` stderr errors |
| **Wave 6 Verifier (`engine`)** | `lumencraft-engine` (`90` commands) | **`76 passed, 0 failed`** across unit + `9` integration suites (`command_sweep_props`, `data_qol_workflows`, `sci_qol_workflows`, `undo_and_methods_qol`, etc.); `fmt` & `clippy -D warnings` exit `0` |
| **Wave 7 Verifier (UI, MCP, Apps, `xtask`)** | `ui-egui`, `mcp`, `web`, `cli`, `lumencraft`, `xtask` | **`153 passed, 0 failed`**: `ui-egui` `69` (default & `--no-default-features`), `mcp` `25`, `cli` `18`, `lumencraft` `9`, `web` `5`, `xtask` `27`; `ui-commands --check`, `layers`, `brands`, `cleanroom`, `./packaging/check-app-specific.sh` exit `0` |
| **macOS live clipboard roundtrip** | `crates/ui-egui/tests/macos_clipboard_live.rs` | Executed live `/usr/bin/pbcopy` + `/usr/bin/pbpaste` (`EXECUTED_LIVE_ROUNDTRIP`) verifying `de-DE` decimal-comma + `=A1*2` formula + `emf_bytes` (`10232` bytes) |
| **macOS bundle validation & signing** | `cargo run -p xtask --offline -- bundle` | Assembled `dist/LumenCraft.app`; `/usr/bin/plutil -lint` `OK`; `/usr/bin/codesign --verify --strict --deep` `valid on disk` (temporary `dist/` cleaned up after check) |

All 6 defects previously tracked under [`gaps.md#known-defects`](gaps.md#known-defects) (`G2`, `G8`, `G9`, `G10`, `G11`, `G12`) were fixed with regression tests in Waves 5–6.

---

## Parity Checklists (Evidence Index)

| Checklist | Scope covered |
|---|---|
| [`target-app-parity.md`](target-app-parity.md) | Weighted `10`-area parity score, methodology, and workspace crate evidence |
| [`stats-parity.md`](stats-parity.md) | Descriptive stats, *t*-tests, 1/2/3-way & Multifactor ANOVA, REML, multiplicity, survival/Cox, contingency, ROC, GLMs, PCA, MICE |
| [`fitting-parity.md`](fitting-parity.md) | Damped LM solver, weighting, Cauchy/ROUT, global fits, profile CIs, `117` built-in models, custom DSL/ODE VM |
| [`graphing-parity.md`](graphing-parity.md) | `8` graph families, `7` axis scales, broken axes, live brackets, Page Layouts, SVG / `%PDF-1.7` / TIFF / `.emf` |
| [`file-format-parity.md`](file-format-parity.md) | `.lumencraft`, `.lumencraft.d/`, `.lctemplate`, WAL journal, CSV/TSV, `.prism`/`.pzfx`, publication vector/raster exports |
| [`ui-parity.md`](ui-parity.md) | `UiApp` state shell, `S1` `1M`-cell grid, `29` `UI_COMMANDS`, HiDPI/RTL interaction, `QOL-1`–`QOL-10`, `WebWorkspaceHost` |
| [`hardware-parity.md`](hardware-parity.md) | Target OS builds, CPU vs. `wgpu` rendering, clipboard bundles & live macOS pasteboard, print spooling, bundle verification |
| [`localization-parity.md`](localization-parity.md) | `16` locales × `167` keys (`2,672` translations), RTL mirroring, decimal/CSV presets, CJK/RTL PDF QA |
| [`qol-beyond-parity.md`](qol-beyond-parity.md) | `QOL-1`–`QOL-11` non-breaking invariants and landed crate/command/UI evidence |

---

## Known Limits

1. **Live OS GPU desktop windowing (`R8.3`):** Native `eframe`/`egui`/`wgpu` desktop window event loops and OS pointer/inline-edit widget bindings are not yet integrated (`Cargo.lock` stays minimal offline; `UiApp` runs headlessly via the software RGBA compositor and `--control`).
2. **`wasm32-unknown-unknown` sysroot & Trunk (`R8.4`):** The `wasm32-unknown-unknown` target sysroot and `trunk` are not installed locally in this offline environment (`cargo xtask wasm` skips locally unless `CI=true` or the sysroot is present). `apps/lumencraft-web` (`WebWorkspaceHost`) compiles and passes host unit tests with `#![forbid(unsafe_code)]`, zero JS, and zero OS calls; browser WebGPU/WebGL2 Trunk packaging awaits `R8.4`.
3. **Native binary OS clipboard & interactive print dialogs (`R5.4`):** `ClipboardBundle` emits `PDF`, `SVG`, `HTML`, `PNG`, `EMF`, and locale TSV payloads and subprocess text adapters + macOS `pbcopy`/`pbpaste` roundtrips pass, while native binary OS vector pasteboard APIs (`NSPasteboard` PDF / Win32 EMF / browser Clipboard API), interactive OS print dialogs, and live layout drag-snap await `R5.4`.
4. **Cross-OS target validation (`R8.5`):** macOS bundle `plutil`/`codesign` and Poppler+Ghostscript PDF QA ran on macOS; native Windows, Linux (Wayland/X11/AppImage/deb/rpm/Flatpak), and FreeBSD runtime and CJK/RTL desktop GUI window checks remain to be run on target OS hosts.
5. **Remaining `R0` CI & release gates (`R0.5`, `R0.6`, `R0.14`):** `R0.1`–`R0.4` and `R0.7`–`R0.13` are landed across `crates/platform`, `crates/engine/src/cmd/`, `crates/format/{versioned,registry,schemas}.rs`, `crates/mcp`, `lumencraft-cli`, `crates/ui-egui`, `xtask`, `docs/commands.md`, and `docs/schemas/*.schema.json`; remaining `R0` items are wiring `cargo-deny` + `cargo xtask licenses` into `.github/workflows/ci.yml` / `cmd_ci` (`R0.5`), verifying `actionlint` & `cargo xtask ci` on live GitHub Actions runners (`R0.6`), and running the release workflow dry-run + capturing `R8.3` desktop screenshots (`R0.14`).
