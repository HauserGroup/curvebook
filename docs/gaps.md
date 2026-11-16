# Parity Gaps & Known Defects (Legacy Mapping Stub)

> **Notice:** Active task planning and claimable backlog items have moved to [`docs/ROADMAP.md`](ROADMAP.md), and verified test counts and known limits live in [`docs/STATUS.md`](STATUS.md). This file preserves the legacy `G1`–`G14` identifier mapping and the [`#known-defects`](#known-defects) anchor referenced by [`docs/audit/README.md`](audit/README.md).

## `G1`–`G14` to `R<m>.<n>` mapping

| Legacy Gap | Area / Scope | Mapped Roadmap ID(s) | Current Status |
|---|---|---|---|
| `G1` | Mathematical foundations, IEEE-754 edge matrix & NIST StRD/`D1` R oracle | [`R1.2`](ROADMAP.md#r12-ieee-754-foundations-special-functions-linear-algebra--nist-strd--d1-corpus) | Done (Waves 1–4) |
| `G2` | Project archive/WAL/DAG persistence & linked-file error handling | [`R2.1`](ROADMAP.md#r21-palettes-8-typed-table-archetypes-subcolumns--dependency-links)–[`R2.3`](ROADMAP.md#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation) | Done (Waves 1–6) |
| `G3` | Descriptive stats, normality/outliers, *t*-tests, ANOVA/RM/REML & post-hoc multiplicity | [`R3.1`](ROADMAP.md#r31-descriptive-statistics-normality-battery-outliers--two-group-tests)–[`R3.2`](ROADMAP.md#r32-one-two-three-way-anova-rm-reml-mixed-models--multiplicity) | Done (Waves 2–4) |
| `G4` | Nonlinear regression (`LM`/QR/ROUT/`117` built-in equations) & global comparison | [`R4.1`](ROADMAP.md#r41-damped-lm-solver-weighting-robustrout-shared-globals-profile-cis--aiccflrt)–[`R4.2`](ROADMAP.md#r42-built-in-equation-library-117-models-across-all-categories) | Done (Waves 2–5) |
| `G5` | Scientific scene compiler, typography, SVG/`%PDF-1.7`/PNG/TIFF & multi-engine PDF QA | [`R5.1`](ROADMAP.md#r51-scientific-scene-compiler-rich-typography-nonlinearbroken-axes--brackets)–[`R5.2`](ROADMAP.md#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export) | Done (Waves 2–5; Poppler + Ghostscript QA passed) |
| `G6` | Pure-Rust `UiApp` shell, `1M`-cell viewport, HiDPI/RTL input (`S1`), live OS windowing & WASM browser hosting | [`R8.1`](ROADMAP.md#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost), [`R8.3`](ROADMAP.md#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop), [`R8.4`](ROADMAP.md#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | Shell (`R8.1`), `eframe`/`wgpu` desktop presenter (`R8.3`), and `wasm32`/Trunk bundle (`R8.4`) done; human-operable interactive chrome open in [`R10`](ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion) |
| `G7` | Multiple Variables, `=` Calculated Variables, role locking & MV ANOVA/plots | [`R6.1`](ROADMAP.md#r61--calculated-variables-role-locking--level-ordering)–[`R6.2`](ROADMAP.md#r62-multifactor-anova-mv-groupingsplits--five-channel-mv-plots) | Done (Waves 2–4) |
| `G8` | Survival/Cox PH, contingency (Barnard/Koopman), Bland–Altman/Passing–Bablok, ROC, GLMs & MICE | [`R3.3`](ROADMAP.md#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice) | Done (Waves 2–5) |
| `G9` | Custom DSL bytecode VM (`IF`, RK45 ODE, `brent_root` implicit) & derived transforms | [`R4.3`](ROADMAP.md#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm)–[`R4.4`](ROADMAP.md#r44-derived-table-transforms) | Done (Waves 2–5) |
| `G10` | Clean-room `.prism` & `.pzfx` import with authoritative cell-budget tracking | [`R2.4`](ROADMAP.md#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget) | Done (Waves 3–5) |
| `G11` | Page Layouts, style cloning, ECMA-234 `.emf`, raster caps, native OS vector clipboard/print & live layout drag-snap | [`R5.2`](ROADMAP.md#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export)–[`R5.4`](ROADMAP.md#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) | Done (Waves 5–13: live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux adapters, `ActiveDialog::Print`, & WYSIWYG layout drag-snap/resize grips in `R5.4`) |
| `G12` | Performance budgets (`< 50 ms` 96-curve 4PL, `100k` Calculated Vars), clone-skip & bounded undo | [`R2.3`](ROADMAP.md#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation), [`R7.2`](ROADMAP.md#r72-performance-budgets-1m-cell-virtualization--propertyfuzz-suites) | Done (Waves 3–6) |
| `G13` | `16`-locale i18n, RTL mirroring, CJK/RTL PDF QA, live macOS locale clipboard & cross-OS font QA | [`R8.2`](ROADMAP.md#r82-16-locale-i18n-rtl-mirroring--desktop-cjkrtl-fontpasteboard-checks), [`R8.5`](ROADMAP.md#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) | Core/macOS/PDF QA (`R8.2`), `cjk_rtl_hidpi_desktop_qa`, and local universal macOS `.dmg`/`.zip` `verify.sh` done; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA remain open in `R8.5` |
| `G14` | `Slice 0` gate, CLI, `--control` TCP JSONL, MCP server & typed engine command registry (`cmd::registry()`; live count in [`commands.md`](commands.md)) | [`R7.1`](ROADMAP.md#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server) | Done (Waves 2–7) |

---

## Known defects

> Reference table retained after Round 1 audit closure (`Stages 1–5` complete on `audit/r1-local-2` and merged into `main`; referenced from [`docs/audit/README.md`](audit/README.md)). All 6 pre-audit Waves 5–6 defects below (`G8`, `G11`, `G2`, etc.) and all 156 unique Stage 1 Round 1 survivors (plus duplicate `R1-S07-02`) are fixed with regression tests; see [`docs/audit/progress.md`](audit/progress.md#stage-25-remediation-closure) and [`docs/STATUS.md`](STATUS.md) for verified suite outputs.

| Legacy ID | Location | Defect Summary | Wave 5–6 Fix & Regression Coverage | Roadmap ID |
|---|---|---|---|---|
| `G8` | `crates/stats/src/categorical.rs` — `koopman_solve_*` | Failing brackets or `brent_root` errors fell back via `.unwrap_or(endpoint)` to finite extreme RR bounds instead of an open/absent bound. | Fixed in Wave 5: `koopman_solve_*` returns `Option<f64>` (`None` when bracketing fails or score is non-finite), mapped to `None` / open bound in contingency CI reporting (`crates/stats/src/categorical.rs` unit tests `koopman_solvers_return_none_on_non_bracketing_scores` & `koopman_solvers_return_none_on_non_finite_and_non_convergent_scores`). | [`R3.3`](ROADMAP.md#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice) |
| `G11` | `crates/engine/src/cmd_sci.rs`; `crates/render/src/rasterizer.rs` | `RasterImage::new` capped per-axis width/height (`16384`) without a total-pixel cap and used `Vec::with_capacity` (panicking/aborting on OOM at `16384×16384`). | Fixed in Wave 5: added `MAX_RASTER_PIXELS = 67_108_864` (`8192×8192`) budget check and `try_reserve_exact`, returning `RenderError` instead of aborting (`crates/render` & `crates/engine`). | [`R5.2`](ROADMAP.md#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export) |
| `G2` | `crates/engine/src/cmd_data.rs` — external delimited-file link creation | `read_to_string(path).unwrap_or_default()` discarded I/O and non-UTF-8 errors, inferring delimiters/decimal separators on empty text. | Fixed in Wave 6: read/UTF-8 errors now propagate as `EngineError` on link creation and refresh (`crates/engine/tests/data_qol_workflows.rs`, `g2_paste_link_missing_or_deleted_external_file_fails_loudly`). | [`R2.3`](ROADMAP.md#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation) |
| `G12` | `crates/engine/src/lib.rs` — `Session::execute`, `undo_stack` | Every command (including read-only exports/queries) deep-cloned `ProjectDocument`, and `undo_stack` grew without depth or cell-count bounds. | Fixed in Wave 6: read-only commands skip pre-execution document snapshots (`is_read_only`), and `undo_stack` evicts oldest entries against `MAX_UNDO_ENTRIES = 64` and `MAX_UNDO_CELL_BUDGET = 25_000_000` (`crates/engine/tests/undo_and_methods_qol.rs`, `g12_*` tests). | [`R2.3`](ROADMAP.md#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation) |
| `G9` | `crates/fit/src/dsl.rs` — `Parser::parse_*`, `enum Expr` | `MAX_PARSE_DEPTH` did not stop left-deep binary chains (`X+X+…` × `200k`), and `Expr` derived recursive `Clone` and `Debug`. | Fixed in Wave 5: enforced `const MAX_AST_NODES: usize = 4_096` in the parser and replaced derived `Clone`/`Debug` on `Expr` with iterative stack walkers (`crates/fit/tests/pharma_binding_and_dsl.rs`). | [`R4.3`](ROADMAP.md#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm) |
| `G10` | `crates/format/src/pzfx.rs` — `estimate_table_cells` vs `build_table_archetype` | Pre-build cell budget estimation duplicated archetype sizing logic instead of measuring the built table's allocated cell slots. | Fixed in Wave 5: enforced `MAX_IMPORT_CELLS = 10_000_000` directly against `DataSheet::allocated_cell_slots()` post-build (`crates/format/tests/unpacked_and_pzfx_budget.rs`, `pzfx_multi_table_cell_budget_enforcement`). | [`R2.4`](ROADMAP.md#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget) |

### Round 1 audit survivors on `main` (`fda1cc0`)

> **Stage 2–5 Remediation Closure (merged into `main` via `833e98d`, `4a88c84`):** All **156 unique surviving findings** (`32` `S1`, `60` `S2`, `58` `S3`, `6` `S4`), duplicate `R1-S07-02` (closed via the shared engine path policy for `R1-S12-01` and `R1-X1-04`), the 5 `S3` title-only items in [`docs/audit/findings/r1/S02.md`](audit/findings/r1/S02.md#additional-findings-titles-only), and cross-referenced twin pairs (`R1-S06-12` <-> `R1-S04-06`, `R1-S04-01` <-> `R1-S05-01`) are **fixed and regression-tested on `main`** (Stage 2 cross-cutting `05c87be` / `1423eee`, Stage 3A silent-stats/core-fit `9c20a85`, Stage 3B remaining `S1` `981a73f`, Stage 4A `S2` `32bf7c5`, Stage 4B `S3` `dd7241c5`, Stage 4C `S4` `d18ef1e1` merging `e0aa5241`, `4ccd4fd1`, `70173106`, `d47179fd`, `7502d626`). Stage 5 external reproducer sweeps across `S01`–`S12`, `X1`, and `X2` confirmed `0` genuine live defects (`R1-S13-*` verified clean-room from [`docs/audit/findings/r1/S13.md`](audit/findings/r1/S13.md)).
>
> To avoid maintaining duplicate copies of closed historical tables, the full 156-item Round 1 index, per-slice breakdown, and partial-mitigation notes live exclusively in [`docs/audit/progress.md`](audit/progress.md#confirmed-and-plausible-findings) and [`docs/audit/findings/r1/`](audit/findings/r1/).

---

### Round 2 delta audit open findings on `main` (`1e9dc59`)

The Round 2 delta audit ([`docs/audit/round2-post-ebcc622.md`](audit/round2-post-ebcc622.md)) reviewed all `38` commits (`29` code/CI/packaging commits) in `ebcc622..fda1cc0` across `S01`–`S13` plus two companion targeted reviews ([`desktop_host.md`](audit/findings/r2/desktop_host.md) and [`interaction-gui_actions.md`](audit/findings/r2/interaction-gui_actions.md)). Re-verification against `main` (`1e9dc59`, `2026-11-16`, after the `833e98d` Round 1 merge and subsequent `R0`/`R3`/`R4`/`R7`/`R10` commits) established:

- **Total audited (`154` items):** **`5` `FIXED_ON_MAIN`**, **`18` `PARTIAL_ON_MAIN`** (`17` `R2-*` + `desktop_host` `NEW-1`), **`131` `LIVE_ON_MAIN`** (`118` `R2-*` + `13` companion).
- **Open `R2-*` survivors (`135` unique):** **`36` `S1`**, **`66` `S2`**, **`31` `S3`**, **`2` `S4`** (`R2-S02-09` `S1`, `R2-S07-02` `S1`, and `R2-S11-08` `S2` closed on `main`).
- **Open companion survivors (`14` items):** `9` in [`desktop_host.md`](audit/findings/r2/desktop_host.md) (`B2`, `B4`, `U7`, `U8`, `M2`, `M3`, partial `NEW-1`, `NEW-2`, `NEW-3`; `B1` and `U9` fixed) + `5` in [`interaction-gui_actions.md`](audit/findings/r2/interaction-gui_actions.md) (`1`, `3`, `4`, `5`, `NEW-1`).

| Slice / Review | Report | Audited | `FIXED` | `PARTIAL` | `LIVE` | Open Survivors (`S1`/`S2`/`S3`/`S4`) |
|---|---|---:|---:|---:|---:|---|
| `S01` Platform, math & palette | [`R2-S01.md`](audit/findings/r2/R2-S01.md) | 4 | 0 | 0 | 4 | **4** (`0/2/2/0`) |
| `S02` Statistics & power engine | [`R2-S02.md`](audit/findings/r2/R2-S02.md) | 16 | 1 | 0 | 15 | **15** (`10/5/0/0`) |
| `S03` Curve fitting & TeX formatting | [`R2-S03.md`](audit/findings/r2/R2-S03.md) | 9 | 0 | 1 | 8 | **9** (`5/3/1/0`) |
| `S04` Transforms, reshape & randomization | [`R2-S04.md`](audit/findings/r2/R2-S04.md) | 13 | 0 | 3 | 10 | **13** (`5/6/2/0`) |
| `S05` Document model & JSON schemas | [`R2-S05.md`](audit/findings/r2/R2-S05.md) | 4 | 0 | 0 | 4 | **4** (`0/2/2/0`) |
| `S06` File formats & unpacked archives | [`R2-S06.md`](audit/findings/r2/R2-S06.md) | 9 | 0 | 1 | 8 | **9** (`3/4/2/0`) |
| `S07` Engine commands, DAG & methods reports | [`R2-S07.md`](audit/findings/r2/R2-S07.md) | 18 | 1 | 3 | 14 | **17** (`3/9/5/0`) |
| `S08` Chart layout & equation boxes | [`R2-S08.md`](audit/findings/r2/R2-S08.md) | 6 | 0 | 0 | 6 | **6** (`2/3/1/0`) |
| `S09` Text markup, EMF, SVG, PDF & raster | [`R2-S09.md`](audit/findings/r2/R2-S09.md) | 3 | 0 | 0 | 3 | **3** (`0/3/0/0`) |
| `S10` Desktop UI, control & OS integration | [`R2-S10.md`](audit/findings/r2/R2-S10.md) | 19 | 0 | 2 | 17 | **19** (`3/14/2/0`) |
| `S11` Canvas tools, desktop, web & packaging | [`R2-S11.md`](audit/findings/r2/R2-S11.md) | 17 | 1 | 1 | 15 | **16** (`3/6/6/1`) |
| `S12` CLI & MCP server | [`R2-S12.md`](audit/findings/r2/R2-S12.md) | 9 | 0 | 3 | 6 | **9** (`2/5/2/0`) |
| `S13` Xtask gates, CI/release & governance | [`R2-S13.md`](audit/findings/r2/R2-S13.md) | 11 | 0 | 3 | 8 | **11** (`0/4/6/1`) |
| Companion: `desktop_host.rs` | [`desktop_host.md`](audit/findings/r2/desktop_host.md) | 11 | 2 | 1 | 8 | **9** companion items |
| Companion: `interaction.rs` × `gui_actions.rs` | [`interaction-gui_actions.md`](audit/findings/r2/interaction-gui_actions.md) | 5 | 0 | 0 | 5 | **5** companion items |
| **Total** | [`round2-post-ebcc622.md`](audit/round2-post-ebcc622.md) | **154** | **5** | **18** | **131** | **`135` `R2-*` (`36/66/31/2`) + `14` companion** |
