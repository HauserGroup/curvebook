# LumenCraft Status

> **Updated:** 2026-11-14 (Wave 18 sync over Waves 15–17: Zero Telemetry, `R5.4`, `R8.6`–`R8.8`, `92` engine / `32` UI commands, `16×291` i18n, `296` files / `209,462` lines, `892` all-targets tests + `4` doctests = `896 passed`, `1440x900` workspace screenshots) · **Roadmap:** [`ROADMAP.md`](ROADMAP.md) · **Parity overview:** [`target-app-parity.md`](target-app-parity.md)

This document summarizes what works in LumenCraft today, the checks that verified it, and the honest limits of the current offline workspace. Every Markdown table below uses unique Column 1 keys (`cargo xtask status` rule).

---

## Verified Capabilities & Headline Numbers

| Capability area | Verified state | Evidence & location |
|---|---|---|
| **Engine command registry** | **`92`** typed `CommandSpec` commands across `9` `CommandGroup` categories (`cmd::registry()`, `Effects`, JSON Schemas, `serde_path_to_error` typed params, bounded `64`-entry undo, `GuiOnlyPolicy::{Disabled, Strict}`, Wave 15 `sheet.extractResultTab` & `graph.formatSelectedPoints`) | [`crates/engine/src/cmd/mod.rs`](../crates/engine/src/cmd/mod.rs), [`commands.md`](commands.md), [`command-ids.md`](command-ids.md), `cargo xtask commands --check`, `lumencraft-engine` **`118 passed, 0 failed`** |
| **UI command catalog** | **`32`** typed `UiCommandSpec` entries mirrored cleanly to MCP (`ui.powerWizard.open`, `ui.formatPoints.open`, `ui.event.dispatch`, `ui.split.toggle`, `ui.brush.point`, `ui.result.copyMethodsReport`, `ui.result.applyRemediation`, `ui.canvas.applyPreset`, `ui.canvas.preflight`, `ui.previewFilter.set`, `ui.presentation.toggle`, `ui.grid.excludeWithReason`, `ui.table.convertArchetype.open`, `ui.import.reshape.open`, `ui.print.open`, etc.) | [`crates/ui-egui/src/menus/ui_commands.rs`](../crates/ui-egui/src/menus/ui_commands.rs), [`crates/mcp/src/ui_catalog.rs`](../crates/mcp/src/ui_catalog.rs), `cargo xtask ui-commands --check` |
| **Built-in fit equation library** | **`117`** built-in parametric models (`47` pharmacology/dose–response/receptor-binding/enzyme-kinetics incl. `two_site_total_and_ns_global` & `one_site_competition_ligand_depletion`, `43` curve/growth/peak/wave/line/polynomial models, `27` NIST StRD nonlinear models) + custom algebraic/implicit/ODE DSL (`MAX_AST_NODES = 4_096`) | [`crates/fit/src/library/mod.rs`](../crates/fit/src/library/mod.rs), [`fitting-parity.md`](fitting-parity.md), `lumencraft-fit` **`87 passed, 0 failed`** |
| **Statistical test & oracle suite** | Parametric/nonparametric tests, 1/2/3-way & Multifactor ANOVA, REML, multiplicity, survival/Cox PH, contingency (`BarnardExact`, Baptista–Pike OR, Koopman score RR returning `None` on non-bracketing/non-finite endpoints), `BlandAltman`, `PassingBablok`, ROC, GLMs, PCA, MICE + Rubin pooling, & **8-family Power & Sample Size** engine (`power.rs`); `58` NIST StRD datasets + `D1` R `4.5.2` & power oracle/edge matrices | [`stats-parity.md`](stats-parity.md), [`crates/stats/src/power.rs`](../crates/stats/src/power.rs), `lumencraft-stats` **`83 passed, 0 failed`** (incl. `power_oracle` **`8 passed`**), `cargo xtask corpus` |
| **Data tables, persistence & interop** | `8` table archetypes, `8` subcolumns, `=` Calculated Variables, `.lumencraft`/`.lctemplate`, versioned WAL `CrashRecoveryJournal`, `.lumencraft.d/` unpacked Git-friendly directory, `NamedCheckpoint`, `CellExclusionNote`, `versioned.rs` upgrade + forward refusal, `FormatRegistry`, clean-room `.prism`/`.pzfx` import | [`file-format-parity.md`](file-format-parity.md), [`file-formats.md`](file-formats.md), `docs/schemas/*.schema.json` (`cargo xtask schemas --check`), `lumencraft-doc` **`35 passed`**, `lumencraft-transform` **`24 passed`**, `lumencraft-format` **`57 passed`** |
| **Publication graphing, export & OS print/clipboard** | `8` graph families + diagnostic plots, `7` axis scales, broken axes, live brackets, `GroupedArrangement::Superimposed`, per-point `PointStyleOverride` (`FormatSelectedPoints`), SVG, `%PDF-1.7` + optional `/FontFile2` SFNT embedding + Core-14 fallback, PNG/Deflate/LZW TIFF (`MAX_RASTER_PIXELS = 67_108_864`), ECMA-234 `.emf` (`encode_emf`), native binary vector pasteboard (`NSPasteboard` `%PDF-1.7` macOS roundtrip + Win32 `CF_ENHMETAFILE` + Linux `wl-copy`/`xclip`), `ActiveDialog::Print` + printer discovery, live WYSIWYG layout drag-snap/resize grips (`R5.4`), `PlottedPointMeta` brushing, `preflight_check_scene` | [`graphing-parity.md`](graphing-parity.md), `lumencraft-chart` **`44 passed`**, `lumencraft-render` **`12 passed`**, `lumencraft-pdf` **`14 passed`**, `lumencraft-svg` **`11 passed`** |
| **Localization & accessibility** | **`16`** locales × **`291`** keys = **`4,656`** translations (`0` missing), RTL `ar`/`he` BiDi shaping & UI layout mirroring, `CanvasPreviewFilter` CVD/grayscale preview, WCAG AA theme contrast, `cjk_rtl_hidpi_desktop_qa` (`ja`/`zh-CN`/`ko`/`ar`/`he` across `1.0x`/`1.5x`/`2.0x`/`3.0x`) | [`localization-parity.md`](localization-parity.md), `cargo xtask i18n-report` |
| **UI shell, GUI-agent parity, CLI, `--control`, MCP & Web** | Pure-Rust `UiApp` + deterministic software RGBA/PNG compositor (`1440x900` clean-room `doc_screenshots`), scoped `UiTreeSnapshot` (`ui.inspect`), 8-tool `gui` MCP mode (`GuiSessionHost` / `InProcessUiGuiHost`), strict `--gui-only` lock (`ErrorCode::GuiOnlyViolation`), 4-stage `PowerWizardState`, `R1`–`R10` modal predicates, `ResultSheet` `Analyze` chaining, `28`-workflow `gui_wf_*` acceptance suite, `DesktopHostRuntime`, browser host adapters, `lumencraft-cli`, `lumencraft --control`, `lumencraft-mcp` | [`ui-parity.md`](ui-parity.md), [`gui-workflows.md`](gui-workflows.md), [`hardware-parity.md`](hardware-parity.md), `lumencraft-ui-egui` **`122 passed`**, `lumencraft-mcp` **`39 passed`**, `lumencraft-cli` **`35 passed`**, `lumencraft` **`19 passed`**, `lumencraft-web` **`13 passed`**, `lumencraft-tools` **`21 passed`** |
| **Platform, `R0` alignment & Zero Telemetry** | `lumencraft-platform` (`L0`, `#![forbid(unsafe_code)]`, wasm-safe) atomic writes, `ProcessCommand`, `WorkdirGuard`; **Zero Telemetry** (`cargo xtask telemetry` in `cmd_ci`, zero analytics/egress, strict web CSP/COOP/COEP headers); `cargo xtask licenses` in CI & `cmd_ci` (`R0.5` done); `296` `.rs` files, `209,462` lines across `22` packages (`106` test targets: `892` tests + `4` doctests = `896 passed, 0 failed`) | [`crates/platform/src/lib.rs`](../crates/platform/src/lib.rs), [`docs/ROADMAP.md`](ROADMAP.md#r0-align-with-house-principles), `lumencraft-math` **`50 passed`**, `lumencraft-palette` **`14 passed`**, `lumencraft-platform` **`11 passed`**, `lumencraft-text` **`25 passed`**, `lumencraft-testkit` **`6 passed`**, `xtask` **`52 passed`**, `cargo xtask stats --exact` |

---

## Quality-of-Life Suite (`QOL-1`–`QOL-17`) Status

The table below records existing Wave A implementation evidence, not completion of the revised plan. `QOL-7` now requires diagnostics-only presentation with no recommendations or remediation actions; code alignment remains pending. `QOL-10`, `QOL-11`, `QOL-13`, and `QOL-15` are deferred without scheduled milestones, while existing code and file compatibility remain supported. Active Wave B scope is `QOL-12`, `QOL-14`, equations-only `QOL-16`, and `QOL-17` (`WF-29`–`WF-49` extended `gui_only` suite; [plan](qol-beyond-parity.md), [supporting specification](calculators-and-quick-stats.md), [workflow matrix](gui-workflows.md)).

| QoL ID | Roadmap ID | Verified implementation |
|---|---|---|
| `QOL-1` | `R9.1` | `table.convertArchetype` + `ConvertArchetypeDialogState` + `exclusion_notes` coordinate remapping & lossy guard |
| `QOL-2` | `R9.2` | `table.importPlateMap` (`96`/`384`-well + `.lctemplate` JSON) & `table.importTidyLong` + `ImportReshapeMode` |
| `QOL-3` | `R9.3` | `CellExclusionNote` + `table.excludeCellsWithReason` + `table.exclusionAuditReport` + `ExcludeWithReasonDialogState` (`Cmd+Shift+E`) |
| `QOL-4` | `R9.4` | `SplitWorkspaceState` (`ui.split.toggle`, `Cmd+\`) & split-divider drag in `crates/ui-egui` |
| `QOL-5` | `R9.5` | `PlottedPointMeta` / `ChartScene::hit_test_point` across all chart kinds + `ui.brush.point` |
| `QOL-6` | `R9.6` | `generate_methods_report` (`PlainText`/`Markdown`/`Latex`), `result.generateReport`, and `ui.result.copyMethodsReport` |
| `QOL-7` | `R9.7` | Existing `AssumptionDiagnosticBanner` and `ui.result.applyRemediation` behavior predates the diagnostics-only decision; removing recommendations/actions from the QOL-7 surface is pending, not implemented by this planning edit |
| `QOL-8` | `R9.8` | `FigureCanvasPreset`, `CanvasResizeMode`, `preflight_check_scene`, `graph/layout.applyCanvasPreset`, `graph.preflightCheck` |
| `QOL-9` | `R9.9` | `CanvasPreviewFilter`, `apply_canvas_preview_filter`, and `ui.previewFilter.set` (non-mutating viewport preview) |
| `QOL-10` | `R9.10` | Deferred for further QoL development; existing `VirtualWorkspaceFolder`, `presentation_mode`, and `WebWorkspaceHost` retained. Core WASM/browser-host requirements remain separate |
| `QOL-11` | `R9.11` | Deferred for further QoL development; existing unpacked-directory I/O, `NamedCheckpoint` commands, and CLI/desktop detection retained |

---

## Wave 15–18 Verification Results (Current `HEAD`)

These directly observed runs verified the post-Wave 17 workspace under `--offline` mode (`CARGO_NET_OFFLINE=true`):

| Check / Suite (Wave 18) | Scope | Observed result |
|---|---|---|
| **Full workspace suite** | `--workspace --all-targets` + `--doc` | `106` test targets (`--all-targets`): **`892 passed, 0 failed, 0 ignored`** + **`4 passed`** doctests (`doc`, `math`, `palette`, `testkit`) = **`896 passed, 0 failed`** across all **`22`** workspace packages (`math` `50`, `palette` `14`, `platform` `11`, `doc` `35`, `chart` `44`, `fit` `87`, `stats` `83`, `text` `25`, `transform` `24`, `format` `57`, `pdf` `14`, `render` `12`, `svg` `11`, `tools` `21`, `engine` `118`, `mcp` `39`, `ui-egui` `122`, `lumencraft` `19`, `cli` `35`, `web` `13`, `xtask` `52`, `testkit` `6`) |
| **Stats & i18n audit** | `xtask stats --exact` & `i18n-report` | **`296`** `.rs` files, **`209,462`** lines across **`22`** packages (`15` proptests); **`16` locales × `291` keys = `4,656` translations** (`0` missing catalog/command keys) |
| **`gui_only` parity & 28-workflow suite (`R8.6`–`R8.8`)** | `stats`, `ui-egui`, `mcp`, `cli`, `lumencraft` | `lumencraft-stats/tests/power_oracle.rs` (`8 passed`), `ui-egui/tests/gui_only_workflows.rs` (`4 passed`), `ui-egui/tests/gui_wf_{foundations_and_data [6], group_comparisons_and_power [9], regression_contingency_multivariate [8], statistical_graphs [5]}` (**`28` workflows `wf01`–`wf28` passed** under `GuiOnlyPolicy::Strict` + `ui.screenshot`), `mcp/tests/gui_only_surface.rs` (`6 passed`), `lumencraft-cli/tests/mcp_gui_only_parity.rs` (`3 passed`), `apps/lumencraft/tests/gui_only_wiring.rs` (`2 passed`) |
| **Clean-room screenshots (`R0.14`)** | `cargo xtask screenshots --check` | Verified `docs/images/workspace-split-fit.png` (`1440x900`, `color_type=6`, `idat=70612 B`, `chunks=4`) & `docs/images/workspace-layout-dark.png` (`1440x900`, `color_type=6`, `idat=68921 B`, `chunks=4`) exit `0` |
| **Policy, sync & hygiene gates** | `xtask {commands,ui-commands,schemas} --check`, `telemetry`, `layers`, `brands`, `cleanroom`, `assets`, `cursors`, `licenses`, `secrets`, `status`, `pending --count` | All **`13`** checked CLI gates exit `0` (`commands --check` `92`, `ui-commands --check` `32`, `schemas --check` `5` schemas, `telemetry` `0` violations across `23` manifests / `80` locked packages / `184` Rust sources / `48` web+workflow+packaging files / `5` workflows / `2` header configs, `layers` `22` packages, `brands` `469` files, `cleanroom`, `assets` `22` files, `cursors` `5` variants, `licenses` offline pass, `secrets` `0` findings, `status` `10` tables, `pending --count` `11` open sub-tasks across `10` items) |

---

## Wave 13–14 Targeted Verification Results (Historical Baseline)

These directly observed runs verified the current `HEAD` workspace under `--offline` mode (`CARGO_NET_OFFLINE=true`):

| Check / Suite (Wave 13–14) | Scope | Observed result |
|---|---|---|
| **Full workspace suite (Wave 14)** | `--workspace --all-targets` + `--doc` | `100` test targets: **`822 passed, 0 failed, 0 ignored`** + **`4 passed`** doctests = **`826 passed`** (`math` `50`, `palette` `14`, `platform` `11`, `doc` `34`, `chart` `40`, `fit` `87`, `stats` `72`, `text` `25`, `transform` `24`, `format` `57`, `pdf` `14`, `render` `12`, `svg` `11`, `tools` `21`, `engine` `115`, `mcp` `33`, `ui-egui` `83`, `lumencraft` `17`, `cli` `31`, `web` `13`, `xtask` `52`, `testkit` `6`) |
| **Stats & i18n audit (Wave 14)** | `xtask stats --exact` & `i18n-report` | **`281`** files, **`192,535`** lines (`22` packages); **`16` locales × `287` keys = `4,592` translations** (`0` missing catalog/command keys) |
| **Clean-room screenshots (`R0.14`)** | `cargo xtask screenshots [--check]` | Rendered `docs/images/workspace-split-fit.png` (`1440x900`, `color_type=6`, `idat=70612 B`) & `workspace-layout-dark.png` (`1440x900`, `color_type=6`, `idat=68437 B`); CRC-32 & `ASSETS.md` check exit `0` |
| **Zero-Telemetry & CI hygiene (`R0.6`)** | `cargo xtask telemetry` & CI workflows | Zero network telemetry/analytics/crash-uploader egress, strict `packaging/web/{_headers,.htaccess}` CSP (`connect-src 'none'`) + COOP/COEP, `.github/workflows/*.yml` `permissions` & `${{ }}` hygiene |
| **Wave 13 host/print/clipboard suites** | `ui-egui`, `lumencraft`, `lumencraft-web`, `mcp`, `cli` | `lumencraft-ui-egui` `83 passed` (incl. live macOS `NSPasteboard` `com.adobe.pdf` `%PDF-1.7` roundtrip + `cjk_rtl_hidpi_desktop_qa` `4`), `lumencraft` `17 passed` (`desktop_host_loop` `7`), `lumencraft-web` `13 passed` (`browser_host_adapters` `7`), `mcp` `33 passed`, `cli` `31 passed` |
| **macOS packaging dry-run (`R8.5`/`R0.14`)** | `packaging/macos/package.sh --arch aarch64` | Compiled `arm64` release (`LumenCraft.app` `24M`: `lumencraft` `11,383,872` B, `lumencraft-cli` `11,948,448` B -> `lumencraft-cli 0.1.0`), `plutil -lint OK`, ad-hoc `codesign --verify --strict --deep` `valid on disk`; stopped at `hdiutil create` (`Operation not permitted` in macOS sandbox), so `.dmg`/`.zip` `verify.sh` awaits a non-sandboxed host |

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
| [`ui-parity.md`](ui-parity.md) | `UiApp` state shell, `S1` `1M`-cell grid, `32` `UI_COMMANDS`, `ActiveDialog::{PowerAndSampleSize, FormatSelectedPoints}`, `R1`–`R10` modal predicates, HiDPI/RTL interaction, `QOL-1`–`QOL-10`, `DesktopHostRuntime`, `WebWorkspaceHost`, `gui_wf_*.rs` |
| [`gui-workflows.md`](gui-workflows.md) | Verified `gui_only` agent execution contract, `ui.inspect` & 8-tool `gui` MCP surface (`R8.6`), 4-stage/8-family Power & Sample Size wizard & 10 modal predicates (`R8.7`), 28-workflow Core `gui_wf_*.rs` acceptance matrix (`R8.8`), and planned 21-workflow extended multivariable/clustering/GLM/graphing matrix (`R9.17` / `QOL-17`, `WF-29`–`WF-49`) |
| [`hardware-parity.md`](hardware-parity.md) | Target OS builds, CPU vs. `wgpu` rendering, native binary OS vector clipboard & macOS `NSPasteboard` `%PDF-1.7`, print dialog/spooling, bundle verification |
| [`localization-parity.md`](localization-parity.md) | `16` locales × `291` keys (`4,656` translations), RTL mirroring, decimal/CSV presets, CJK/RTL PDF & `cjk_rtl_hidpi_desktop_qa` |
| [`qol-beyond-parity.md`](qol-beyond-parity.md) | Active and deferred QoL scope, diagnostics-only `QOL-7`, equations-only `QOL-16`, extended `gui_only` `QOL-17`, and separate existing implementation evidence |
| [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md) | Active table-analysis completions, randomization/sampling, and equation callouts; inactive reference designs for deferred statistical and bench calculators |

---

## Known Limits

1. **External OS GPU windowing crate linkage (`R8.3`):** `DesktopHostRuntime` (`apps/lumencraft/src/desktop_host.rs`: `--in-window-menus`, `--new-instance`, Retina DPI transitions, IME, pinch/inertial pan, file drop) passes host integration tests (`lumencraft` `19 passed`), while linking external `eframe`/`wgpu` GPU windowing crates awaits online `Cargo.lock` expansion (`UiApp` renders deterministically via software RGBA/PNG compositing and `--control`).
2. **`wasm32-unknown-unknown` sysroot & Trunk (`R8.4`):** `apps/lumencraft-web` (`WebWorkspaceHost` + `host_adapters.rs` `WebStorageManager`, `WebLockCoordinator`, `FileSystemAccessBridge`, `TouchGestureRecognizer`, `WebBridgeSession`, strict no-egress `index.html`, and CSP/COOP/COEP headers) passes native host tests (`lumencraft-web` `13 passed`, `#![forbid(unsafe_code)]`, zero OS calls); `cargo xtask wasm` / `trunk build --release` skips locally because `wasm32-unknown-unknown` and `trunk` are absent offline.
3. **Non-sandboxed macOS `.dmg`/`.zip` & cross-OS VM QA (`R8.5`):** `R5.4` native binary vector clipboard (macOS `NSPasteboard` `%PDF-1.7` roundtrip + Win32 `CF_ENHMETAFILE` + Linux `wl-copy`/`xclip`), `ActiveDialog::Print` + OS printer discovery, WYSIWYG layout drag-snap/resize grips, and `cjk_rtl_hidpi_desktop_qa` are implemented and verified, and `package.sh --arch aarch64` verified `LumenCraft.app` (`24M`), `plutil -lint`, and ad-hoc `codesign --verify --strict --deep`; `hdiutil create` failed inside the restricted macOS sandbox (`Operation not permitted`), leaving non-sandboxed `.dmg`/`.zip` `verify.sh`, Windows/Linux/FreeBSD VMs, and live browser QA open.
4. **Remote CI & release gates (`R0.6`, `R0.14`):** `R0.5` (`cargo xtask licenses` wired into `cmd_ci` & `.github/workflows/ci.yml`) is **done**, and SonarCloud (`R0.6`) is permanently excluded by the Zero-Telemetry policy (`cargo xtask telemetry`). Remaining remote items are running `actionlint` cleanly on GitHub Actions (not installed locally) plus the `v0.1.0-rc1` tag release dry-run across the 5-OS GitHub Actions matrix with live `cargo deny`/`cargo about`.
5. **Wave B & QoL revision backlog (`R9.7`, `R9.12`, `R9.14`, `R9.16`, `R9.17`):** Strict `gui_only` agent parity, the 4-stage/8-family Power & Sample Size wizard, 10 modal predicates, and the 28-workflow `gui_wf_*.rs` acceptance suite (`R8.6`–`R8.8`) are **done**. Outside `R8.6`–`R8.8`, `run_factorial_anova` multiplicity post-hoc comparison tab emission remains in active `R9.12` alongside randomization/sampling (`R9.14`), equations-only callouts (`R9.16`), and the extended 21-workflow multivariable/clustering/GLM/graphing `gui_only` suite (`R9.17`, `WF-29`–`WF-49`). Diagnostics-only alignment is pending in `R9.7`; `R9.10`, `R9.11`, `R9.13`, and `R9.15` are deferred, and CLD is excluded.
