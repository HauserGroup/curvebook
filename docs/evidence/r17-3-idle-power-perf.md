# R17.3 Real-Hardware Idle Power & Large-Project Responsiveness Evidence

> **Scope:** CurveBook **R17.3** (`feature/R17.3-idle-power-perf`) — UI idle-repaint elimination (`apps/curvebook`, `crates/ui-egui`) and large-project frame-time/memory optimization (`crates/ui-egui`, `crates/engine`, `crates/format`).  
> **Host Environment:** macOS (`aarch64-apple-darwin`, Apple Silicon), Rust `release` profile for performance benchmarks and `debug` test profile for idle/correctness harnesses.

---

## 1. Idle Power & Repaint Audit Summary

All `request_repaint`, `request_repaint_after`, and implicit `egui::Context` wakeup paths were audited across `apps/curvebook/src/{gui_window.rs,live_control.rs}` and `crates/ui-egui/src/egui_shell/*`.

### Root Causes Found & Fixed

| Area | Root Cause | Fix & Location |
|---|---|---|
| **Async canvas raster loop** | `composite_async_scene_texture` checked `cached_texture.is_some()` in `slot_matches` and had no failure flag; failed/unavailable renders (`None` from `render_scene`) retained stale textures from previous sheets and looped `ui.ctx().request_repaint_after(16ms)` continuously (~60 FPS wakeups). In-flight jobs also kept a redundant 16 ms polling timer alongside worker wakes. | Added `last_raster_failed: bool` to `CanvasSceneRenderSlot`, introduced `AsyncRasterPoll::{Ready, InFlight, Unavailable}`, installed RAII `WakeGuard` in `raster_worker_loop` so `job.ctx_wake.request_repaint()` fires once on job completion, cleared stale `cached_texture` on failure/unavailable, and removed the 16 ms polling timer (`crates/ui-egui/src/egui_shell/canvas_view.rs`). |
| **Idle focused `TextEdit` caret timer** | `Tokens::to_visuals()` inherited egui's `text_cursor.blink = true`, scheduling `<= 500 ms` timer repaints indefinitely (`2 Hz`) whenever any text input held focus idle (`AnalyzeSheet`, `CommandPalette`, `FindReplace`, `DataGrid` inline edit). | Set `visuals.text_cursor.blink = false` in `Tokens::to_visuals()` (`crates/ui-egui/src/egui_shell/mod.rs`), rendering a solid caret consistent with `driver.rs` and `lib.rs` with zero ongoing idle wakeups. |
| **Indeterminate progress spin** | Indeterminate `ProgressModal` called unconditional `body.ctx().request_repaint()` (`0 ns` immediate wakeup every frame). | Changed to `body.ctx().request_repaint_after(Duration::from_millis(33))` (`~30 FPS` cap strictly while open and indeterminate; `crates/ui-egui/src/egui_shell/center_stage.rs`). |

### Deterministic Idle-Repaint Regression Tests

All idle tests instrument `egui::Context::set_request_repaint_callback` (counting repaints triggered) and assert `repaint_callbacks == 0` plus `viewport_output[ROOT].repaint_delay == Duration::MAX` across settled idle frames:

1. **`crates/ui-egui/src/egui_shell/canvas_view.rs`** (`raster_worker_drop_tests`):
   - `owner_dropped_on_its_own_worker_thread_does_not_self_join` & `async_failed_raster_clears_stale_texture_and_stops_polling`: **`2 passed; 0 failed`** (`0` idle repaint callbacks, `repaint_delay == Duration::MAX` after failed async raster, stale texture cleared).
2. **`crates/ui-egui/tests/r17_3_kittest_idle.rs`** (`egui_kittest::Harness` + `UiApp`):
   - `egui_kittest_surfaces_and_idle_dialog_emit_zero_ongoing_repaints`: **`1 passed; 0 failed`** in `0.36s` across Data Grid, Graph Canvas, Layout Canvas, and idle focused `AnalyzeSheet` dialog.
3. **`apps/curvebook/tests/r17_3_idle_repaint.rs`** (`CurveBookNativeApp` + `DesktopHostRuntime`):
   - `welcome_hub_and_data_sheets_settle_to_zero_idle_repaints`
   - `data_grid_scroll_hover_and_inline_edit_settle_to_zero_idle_repaints`
   - `graph_layout_split_and_uninitialized_canvases_settle_to_zero_idle_repaints`
   - `idle_dialogs_and_progress_modal_schedule_zero_ongoing_repaints` (18 dialogs + `ProgressModal` ~33 ms throttle -> dismiss -> `0` callbacks)
   - `stationary_navigator_hover_timer_settles_to_zero_idle_repaints` (past `> 0.6s` tooltip dwell)
   - Result: **`5 passed; 0 failed`** in `0.94s`.

---

## 2. Large-Project Profiling & Performance Optimizations

### Bottlenecks Diagnosed & Fixed

1. **Uncached Graph Scene Compilations During Layout Drag & Graph Render (`crates/ui-egui/src/egui_shell/`)**
   - `render_layout_hud_overlays` (`canvas_view.rs`) called uncached `app.session().render_sheet_scene(graph_sheet_id)` for every live graph panel on every frame regardless of whether `layout_drag` was active or snapping was enabled. Gated `plot_origins` collection strictly on `state.layout_drag.is_some() && studio_snap_enabled(ui.ctx())` and switched to `resolve_cached_scene(ui.ctx(), app, graph_sheet_id, "layout_hud")`.
   - Exposed `resolve_cached_scene` (`pub(super)`) and updated `center_stage::collect_graph_clipping_warnings` and `trailing_studio::drill_back::ensure_drill_points_cache` to reuse `"canvas"` cached `ChartScene` entries keyed by `mutation_seq`.
2. **Per-Edit Checkpoint JSON Re-Deserialization & Whole-Project Cloning (`crates/format/src/{archive.rs,lib.rs}`, `crates/engine/src/{lib.rs,cmd/table.rs}`)**
   - Every `table.setCell` previously cloned the entire `ProjectDocument` (up to 1M cells plus multi-megabyte `NamedCheckpoint::snapshot_json` payloads) for undo/rollback, and `validate_project_document` re-parsed every `checkpoint.snapshot_json` plus scanned every cell in the project.
   - Added `validate_project_document_skip_checkpoints` in `curvebook-format` for mutations where `doc.checkpoints` is untouched after prior validation (`Session::baseline_verified` tracks initial `Session::from_document` validation and checkpoint mutations so corrupt checkpoints or structural issues are still caught on first edit).
   - Wrapped `UndoEntry` internals in `Arc<UndoEntryInner>` (making stack cloning in `execute_atomic_group` and `execute_undo_redo_with_hook` O(1) pointer increments).
   - Added O(1) `UndoPayload::CellDelta(SingleCellDelta)` via `cmd::table::{prepare_fast_set_cell, fast_set_cell_outcome}` for isolated in-bounds `table.setCell` edits with exact preservation of `MultipleVariables` inferred role transitions, `exclusion_notes`, undo slot/checkpoint byte budgets (`MAX_UNDO_CELL_BUDGET` / `MAX_UNDO_CHECKPOINT_BYTES_BUDGET`), post-commit rollback, and full fallback when row auto-expansion, linked regions, calculated columns, downstream dependents, graph point overrides, dirty analyses, or non-finite floats apply.
   - Added unit test `curvebook_engine::tests::fast_set_cell_delta_preserves_mv_role_exclusion_notes_and_rollback` (**passed**).
3. **Redundant DAG Clones & Active-Series Column Scans (`crates/engine/src/{dag.rs,cmd/helpers.rs}`)**
   - `dag::recalculate_project_scoped` no longer clones the full `ProjectDocument` when no unfrozen dirty `DataSheet` has linked regions or calculated variables, and temporarily detaches `doc.checkpoints` / `doc.image_assets` around fallible dirty-branch evaluation.
   - `evaluate_data_sheet_node` returns before `snapshot_active_series_lengths` when `!had_links && !has_calc_vars`, and `snapshot_active_series_lengths` / `prune_shortened_series_overrides` return immediately when no `GraphSheet` holds per-point style overrides.

---

## 3. Reproducible Release-Build Before / After Results

### A. Existing 1,000,000-Cell Column Input-to-Paint Go/No-Go Gate (`< 50 ms`)

Command:
```sh
CARGO_TARGET_DIR=/tmp/cb-r17-perf cargo test --release -p curvebook-ui-egui --test r15_2_anova_and_1m_bench -- --nocapture --ignored
```

| Metric (`1000 x 1000` Column, `1.0x` / `2.0x`) | Before (`baseline`) | After (`feature/R17.3-idle-power-perf`) | Speedup / Status |
|---|---:|---:|---|
| `1.0x` Frame (`FrMean` / `FrP95` / `FrMax`) | `1.072` / `1.163` / `4.234` ms | `1.052` / `1.283` / `2.342` ms | `FrMax` `1.81x` |
| `1.0x` Input-to-Paint `InP50` | `1.670 ms` | `1.619 ms` | Maintained |
| `1.0x` Input-to-Paint **`InP95`** | `54.641 ms` | **`2.258 ms`** | **`24.2x` faster** |
| `1.0x` Input-to-Paint **`InMax`** | `63.870 ms` (**FAILED `< 50ms`**) | **`19.681 ms`** | **`3.25x` faster (PASSED)** |
| `2.0x` Input-to-Paint **`InP95`** | `47.062 ms` | **`2.321 ms`** | **`20.3x` faster** |
| `2.0x` Input-to-Paint **`InMax`** | `56.580 ms` (**FAILED `< 50ms`**) | **`19.190 ms`** | **`2.95x` faster (PASSED)** |

*(Root-cause diagnostic showed every second `Click` in `r15_2` fell 50 ms after the previous click, triggering inline edit, and the subsequent `ArrowDown` committed `table.setCell`, previously paying 43.541 ms to double-clone the 1M-cell `ProjectDocument` and re-scan 1,000 columns.)*

### B. Five Real-Shaped Stress Scenarios (`apps/curvebook/tests/r17_3_perf_scenarios.rs`)

Command:
```sh
CARGO_TARGET_DIR=/tmp/cb-r17-perf cargo test --release -p curvebook --test r17_3_perf_scenarios -- --nocapture
```

| Scenario | Metric | Before (`--release`) | After (`--release`) | Improvement |
|---|---|---:|---:|---|
| **A. 57-sheet NestedDock switch + frame** | `switch+frame` (`mean` / `max`) | `1.329 ms` / `6.005 ms` (`recalc_all=41.238ms`) | `1.050 ms` / `4.145 ms` (`recalc_all=39.720ms`) | `1.27x` mean (`1.45x` max) |
| **B. 100k-row MV Grid** | `init` / `scroll mean` / `hover mean` / `col_stats_50k` | `6.750` / `2.061` / `2.375` / `1.790` ms | `4.401` / `1.686` / `1.301` / `1.749` ms | `1.53x` init, `1.83x` hover |
| **C. 16-panel Page Layout drag** | `interactive_drag_frame` (`mean` / `p95` / `max`) | `24.183` / `30.976` / `33.779` ms | **`7.739` / `15.360` / `19.400` ms** | **`3.12x` faster mean** (60 FPS budget) |
| **D1. 100k-row MV + CalcVar + 1 checkpoint** | `setCell` `mean` / `atomic_group` `mean` | `90.150 ms` / `98.886 ms` | **`4.893 ms` / `7.425 ms`** | **`18.4x` `setCell`, `13.3x` `atomic_group`** |
| **D2. 57-sheet + 5 checkpoints** | `setCell` `mean` (`undo` / `redo`) | `29.372 ms` (`0.004` / `0.004` ms) | **`0.132 ms`** (`0.001` / `0.002` ms) | **`222x` faster `setCell`** |
| **E. 100k-row Live Cell Commit + Sync + Frame (+1 CP)** | `mean` / `p95` / `max` | `114.478` / `174.139` / `183.677` ms | **`6.231` / `6.431` / `10.936` ms** | **`18.4x` mean, `27.1x` p95** |
| **F. 1000x1000 Column `setCell` burst** | `first` / `steady mean` / `steady max` | `43.541 ms` (`ArrowDown` commit) | `15.779 ms` / **`0.582 ms`** / `0.629 ms` | **`74.8x` faster steady edit** |

### C. Release Memory & CPU Instruction Profile (`/usr/bin/time -l`)

Measured directly on the compiled release `r17_3_perf_scenarios` test binary (`stress_profile_all_five_scenarios`):

```sh
/usr/bin/time -l /tmp/cb-r17-perf/release/deps/r17_3_perf_scenarios-<hash> --exact stress_profile_all_five_scenarios
```

| Metric | Before | After | Delta |
|---|---:|---:|---|
| Wall clock / User CPU | `6.77 s` real / `5.64 s` user | **`2.76 s` real / `1.34 s` user** | **`2.45x` real / `4.21x` user** |
| Maximum resident set size (`RSS`) | `760,709,120 B` (`725.5 MiB`) | **`635,748,352 B` (`606.3 MiB`)** | **`-124.96 MB` (`-16.4%`)** |
| Peak memory footprint | `564,413,544 B` (`538.3 MiB`) | **`475,071,376 B` (`453.1 MiB`)** | **`-89.34 MB` (`-15.8%`)** |
| Instructions retired | `79,951,093,916` | **`16,758,507,296`** | **`4.77x` fewer instructions** |
| Cycles elapsed | `17,432,305,968` | **`4,736,941,897`** | **`3.68x` fewer cycles** |

### D. `curvebook-cli perf --require-release --json`

Command:
```sh
CARGO_TARGET_DIR=/tmp/cb-r17-perf cargo run --release --bin curvebook-cli -- perf --require-release --json
```

| Check | Before (`--release`) | After (`--release`) | Budget / Status |
|---|---:|---:|---|
| `mv_100k_calculated_variable.eval_ms` | `9.31 ms` | `9.123 ms` | Budget `250.0 ms` (`ok: true`) |
| `dose_response_96_curves` (`fit` / `compile` / `total`) | `34.875` / `4.465` / `39.34` ms | `20.18` / `4.49` / `24.67` ms | Budget `50.0 ms` (`ok: true`, `r_squared=0.9995472041275347`) |
| `archive_roundtrip` (`save` / `open` / `total`, `1115511` B) | `111.076` / `41.49` / `152.566` ms | `99.234` / `40.131` / `139.365` ms | `ok: true` |
| `exports` (`svg` / `pdf` / `png` / `text` @ 300 DPI) | `17.954` / `70.995` / `1757.707` / `0.006` ms | `14.101` / `31.509` / `2511.898` / `0.006` ms | `ok: true` *(PNG jitter from host CPU contention; raster path unchanged)* |

---

## 4. Policy, `wasm32`, & Regression Verification

Verified commands in the worktree root (with `CARGO_TARGET_DIR=/tmp/cb-r17-perf`):

- `cargo fmt --all -- --check`: exited `0`.
- `cargo xtask cleanroom`: `cleanroom: no vendor copyright notices or vendor-created fixtures found across 1196 tracked files` (exited `0`).
- `cargo xtask brands`: `brands: 0 unapproved brand or suite mentions across 1196 tracked files` (exited `0`).
- `cargo xtask telemetry`: `telemetry: 0 violations across checks 1-5` (exited `0`).
- `cargo xtask licenses`: `licenses ok` (exited `0`).
- `cargo xtask wasm`: `Finished web-release`, `web-bundle: CI readiness OK` (L0–L5 crates clean for `wasm32-unknown-unknown`, exited `0`).
- `cargo test -p curvebook-format --tests`: all integration and unit tests passed.
- `cargo test -p curvebook-engine`: all unit tests (including `fast_set_cell_delta_preserves_mv_role_exclusion_notes_and_rollback`) and integration suites passed.
- `cargo test -p curvebook-ui-egui --lib raster_worker` (`2 passed`), `--test r17_3_kittest_idle` (`1 passed`), `cargo test -p curvebook --test r17_3_idle_repaint` (`5 passed`), `--test r17_3_perf_scenarios` (`1 passed`).

## 5. Hardware & Cross-OS QA Caveat

- Zero-ongoing-repaint and release benchmark metrics were verified directly on macOS (`arm64` Apple Silicon) using native `DesktopHostRuntime`, `egui_kittest`, and `--release` profiles.
- **Physical hardware idle power draw measurements on Windows (e.g., Task Manager / Windows Performance Recorder / Battery & Energy Usage) and Linux (e.g., `powertop` / laptop power meter) remain cross-OS QA outside this macOS build environment.**
