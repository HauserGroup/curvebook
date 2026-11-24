# Implementation plan: R15 GUI audit remediation, maintainability proposals and GUI module splits

> **Status:** proposed 2026-11-20 on branch `audit/R15-gui-audit-ledger`. Nothing here is claimed or applied to [`ROADMAP.md`](../ROADMAP.md) yet. Item IDs marked *proposed* are placeholders until the roadmap edit in Phase 0 assigns them.

## 1. What this plan combines

| Source on this branch | Content | Kind of work |
|---|---|---|
| [`audit/r15-gui-audit-2026-11-20.md`](audit/r15-gui-audit-2026-11-20.md) | `R15-GUI-01`…`R15-GUI-28`: defects in the `egui` desktop GUI, ranked High/Medium/Low | Bug fixes with regression tests |
| [`audit/maintainability-ledger-2026-11-20.md`](audit/maintainability-ledger-2026-11-20.md) | `R0-1`…`R16-1`: duplicated logic and hand-written general-purpose machinery across the workspace | Behaviour-preserving refactors, some library swaps |
| [`planning/r16-gui-maintainability-scope.md`](r16-gui-maintainability-scope.md) | Move-only splits of the largest `crates/ui-egui` modules; pruning merged `R15` branch refs | Refactors and repository housekeeping |

The three sources overlap in the same files. The plan orders them so that **bug fixes land first, shared helpers next, and file splits last**, and so that parallel work never edits the same file.

## 2. Ground rules (from `AGENTS.md`, applied to this plan)

- **One roadmap item per PR**, PR title ending in `(R<m>.<n>)`, branch `<prefix>/R<m>.<n>-slug`. Items below are split into bullets that can each be claimed and landed as their own PR (the `R13.8` pattern).
- **Claim before starting** in `ROADMAP.md`; release the claim if you stop.
- **GUI fixes prove themselves through the GUI.** Every fix for an `R15-GUI-*` finding adds a regression test that drives the rendered widget through `EguiHumanPathDriver` (human-path evidence per [pitfalls §6](../pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)); harness-only tests (`gui_wf_*`, `--control`) do not count on their own.
- **Refactors are behaviour-preserving.** `D1` oracles, NIST StRD suites, export determinism tests and golden outputs pass unchanged; `widget_id`, `command_id`, AccessKit roles and `FrameInspectMeta` entries stay byte-identical; public paths stay reachable (re-export from `mod.rs`).
- **Engine wire contract** changes only through `R16.5` (parameter spellings).
- **Dependencies** must be on the licence allowlist, pass `cargo deny` and `cargo xtask licenses`, and need maintainer approval (§6). No `chrono`, no hand-rolled calendar arithmetic, no HTTP or telemetry crates.
- **No hardcoded mutable counts** in hand-written docs ([pitfalls §2](../pitfalls.md#2-hardcoded-headline-counts-going-stale)); measure module sizes with `cargo xtask stats --exact`.
- **Parallel agents:** disjoint files, isolated `CARGO_TARGET_DIR`, worktrees under `../.cb-worktrees/`, no shared-tree git mutations.
- **Verification:** scoped checks while iterating, `cargo xtask ci quick` before each PR, hosted CI as the full gate. Re-run only what a change invalidates.

## 3. Phase 0 — Land the records and scope the roadmap

1. Merge `audit/R15-gui-audit-ledger` (the three documents plus this plan) into `main` as a docs-only PR.
2. One roadmap PR adds the items below. Keep the edit additive: `refactor/R16.5-callers-and-roadmap-archive` is rewriting the `R16` section, so whichever lands second rebases.
   - **`R15.6` *(proposed)* — Post-cutover GUI audit remediation.** Bullets = Phases 1 and 2. Proposed gate: **Beta** for every High and Medium finding (they affect the `R10.13` desktop walkthrough and `R10.24` smoke QA); Low findings ungated. *Alternative if `R15` stays closed:* file it as `R10.25`.
   - **`R16.7`–`R16.11`** — GUI module splits and ref pruning, as in the scope document (Phase 4).
   - **`R16.12`–`R16.18` *(proposed)*** — maintainability-ledger workstreams grouped by crate (Phase 3).
   - Fold `R11-1` into `R11.11` (browser bridge) and `R13-1` into `R13.4` (rectangular result tables) as bullets; they belong to that work, not to a refactor item.
   - Widen the `R16` title to "Core- and GUI-crate maintainability" and state that the `crates/ui-egui/**` off-limits rule ended with `R15.5`.
3. **`R16.10` ref pruning** (maintainers, needs explicit approval): verify each merged `origin/feature/R15*` ref with `git merge-base --is-ancestor <ref> origin/main`, delete with `git push origin --delete`, then `git fetch --prune`. Do not delete branches checked out in any worktree.

## 4. Phase 1 — Fix the shared GUI intent pipeline first (`R15.6`, wave A)

Every view routes clicks through `PendingUiIntents`; fixing it first removes failure modes that make later findings worse. Run these sequentially (they share `egui_shell/mod.rs` and `apps/curvebook/src/gui_window.rs`).

| Order | Finding(s) | Change | Main files |
|---|---|---|---|
| A1 | R15-GUI-02 | Apply intents one by one and collect per-intent results; always run `sync_views`/history sync; have the desktop host run `record_ui_mutation_checkpoint` whenever `mutation_seq` moved, even on error; report the first error in the banner. | `egui_shell/mod.rs` (`commit_if_settled`), `apps/curvebook/src/gui_window.rs` (`commit_pending_intents`), `crates/ui-egui/src/lib.rs` |
| A2 | R15-GUI-05 | Keep intents from the first pass of a frame (the pass that received input) and de-duplicate across re-runs instead of dropping them on `will_discard`; stop gating local widget state on `settled_frame` in `unit_spinners.rs`. | `egui_shell/mod.rs`, `gui_window.rs`, `lib.rs` (`run_settled_egui_on_ctx`), `egui_shell/inspect_bridge.rs`, `egui_shell/unit_spinners.rs`, `egui_shell/data_grid.rs` (scroll discard guard) |
| A3 | R15-GUI-07, R15-GUI-13, R15-GUI-23 | One host-routed command list (a flag on `UiCommandSpec` or one `pub` table) used by `top_chrome.rs`, `gui_window.rs` and `desktop_host.rs`; host-routed intents carry their params (Welcome "Open" path) and go through the dirty-document prompt; add `graph.exportEmf`/`layout.exportEmf` to the host save path. | `egui_shell/top_chrome.rs`, `gui_window.rs`, `apps/curvebook/src/desktop_host.rs`, `egui_shell/welcome_hub.rs`, `egui_shell/app_modals.rs` |
| A4 | R15-GUI-18 | Command palette: route parameterised commands to their dialog through the same resolver as the menus, or fill the active sheet; hide rows that cannot run. | `egui_shell/command_palette.rs` |
| A5 | R15-GUI-04 | Replace per-request `std::thread::spawn` with one long-lived raster worker and a latest-request slot; no panicking spawn. | `egui_shell/canvas_view.rs` |

Regression tests: a frame that queues a succeeding and a failing intent (A1); a click handled in a discarded pass (A2, using a first-time `egui::Grid`); Welcome open-by-path and EMF export on the desktop host (A3); a palette row that needs `sheet_id` (A4); a resize drag that issues many raster keys (A5).

## 5. Phase 2 — Fix wrong results and data loss (`R15.6`, wave B)

These can run in parallel **by file** once Phase 1 has landed. Each row is one PR.

| Stream | Finding(s) | Change | File(s) | Wait for |
|---|---|---|---|---|
| B1 Transform | R15-GUI-09 (High), R15-GUI-10 | Explicit "no Y transform" state; build `LinearScale` from K at commit time in both paths; reject K = 0 for division; wire or remove "Zero X substitution" and "Create new graph". | `egui_shell/transform_modals.rs` | `R16.5` callers branch (edits this file) |
| B2 Model formula | R15-GUI-03 | Generate every k-subset up to the interaction order, as the engine does. | `egui_shell/stats_modals.rs` | — |
| B3 Fit dialog | R15-GUI-16, R15-GUI-17 | Editable X-domain bounds seeded from the data range; locale-aware numeric fields for initial values and lower/upper bounds; "Auto" computed from the analysed data or labelled as a model default. | `egui_shell/fit_modal.rs` | — |
| B4 Data grid | R15-GUI-11, R15-GUI-12, R15-GUI-06 | Multi-line X entry via `table.setXCells`; clear the Y selection when an X cell is selected and route clipboard shortcuts X-first; `rows.get(r0..=hi)` in quick stats. | `egui_shell/data_grid.rs`, `egui_shell/status_footer.rs` | — |
| B5 Clipboard | R15-GUI-19 | One shared copy/cut helper (uncapped rectangle bounds, per-cell clears for discontiguous selections) used by the shell and the desktop host. | `egui_shell/mod.rs`, `apps/curvebook/src/desktop_host.rs` | A1–A3 (same files) |
| B6 Import | R15-GUI-20 | Separate `target_sheet_id` from `source_sheet_id`; commit to the explicit target. | `egui_shell/mod.rs`, `crates/ui-egui/src/grid_view.rs`, `desktop_host.rs` | B5; `R16.5` callers branch (edits `grid_view.rs`) |
| B7 InfoSheet | R15-GUI-27, R15-GUI-22, R15-GUI-01 | First unused `Constant N` (or `if_exists: false`); locale-aware number parsing in InfoSheet, studio Info panel, heatmap range and extra ticks; remove hand-rolled calendar code (see decision D1). | `egui_shell/info_view.rs`, `egui_shell/trailing_studio.rs`, `egui_shell/graph_modals.rs` | Decision D1 |
| B8 Graph dialogs | R15-GUI-24, R15-GUI-15, R15-GUI-28 | Re-sync Format Axes drafts on every open and on Cancel, send `frame_preset` only when changed; show auto bounds as auto and report rejected bounds; stage extra ticks with the rest of the dialog. | `egui_shell/graph_modals.rs` | B7 (same file) |
| B9 Variables | R15-GUI-21 | First reproduce (formula after unit edit). If confirmed, see decision D2. | `egui_shell/manage_vars_modal.rs` (+ engine if D2 chooses rename propagation) | Decision D2 |
| B10 Cleanup | R15-GUI-14, R15-GUI-25, R15-GUI-26 | `.curvebook` wording via i18n keys; persist and apply Preferences graph defaults or remove them; delete the unreachable Recovery dialog (or wire it to real restore/discard). | `egui_shell/welcome_hub.rs`, `egui_shell/app_modals.rs`, `egui_shell/mod.rs` | A3 (same files) |

`R15-GUI-08` (local `too_many_arguments` suppressions) is handled inside the Phase 4 splits, not as a separate PR.

## 6. Decisions needed before the affected work starts

| ID | Decision | Options | Blocks |
|---|---|---|---|
| D1 | How InfoSheet dates are validated without hand-rolled calendar code or `chrono` | (a) drop the ±day stepper and validate only the `YYYY-MM-DD` shape, leaving calendar validity to an existing doc/engine check if one exists; (b) add an allowlisted date crate with maintainer approval | B7 |
| D2 | How units relate to variable names | (a) a separate `units` field on `VariableColumn` (schema version bump + migration per [`file-formats.md`](../file-formats.md)); (b) keep units in the title and have the engine rewrite `[Title]` references on rename; (c) keep current behaviour and warn | B9 |
| D3 | `R15.6` gate and home | `R15.6` (Beta) vs `R10.25` | Phase 0 |
| D4 | New or promoted dependencies | Promote locked transitive crates to direct dependencies: `weezl` (R5-3), `quick-xml` (R2-1, R5-6), `semver` (R14-1). Add new crates: `sha2` (R1-2), `base64` (R7-1), `pulldown-cmark` (R12-1, xtask only). `png` is already a direct workspace dependency (R5-1 needs only a manifest entry in `curvebook-render`). Each needs `cargo deny`, `cargo xtask licenses` and `about.toml` updates. | The bullets named |

## 7. Phase 3 — Maintainability workstreams (`R16.12`–`R16.18`, proposed)

These touch core crates that the GUI work does not edit, so they can run **in parallel with Phases 1–2**. Within each item, land the "good first" bullets first. Each bullet is its own PR.

| Item *(proposed)* | Ledger entries, in order | Notes and collisions |
|---|---|---|
| `R16.12` xtask tooling | R0-1 (use `serde_json`), R0-3 (copy the UI catalogue module whole), R0-4 (shared text decoding), R0-2 (one metadata snapshot), R14-1 (`semver`, D4), R12-1 (CommonMark parser, D4, optional) | Wait for `perf/R0.32-xtask-fast` (edits `brands.rs`, `ci.rs`, `main.rs`, `status.rs`). R0-3 must keep `cargo xtask ui-commands --check` green. |
| `R16.13` numerical kernels | R1-1 (one Householder reflector update; same FMA sequence), R6-1 (Type 7 interpolation in `curvebook-math`; check exact-rank and extreme cases), R1-2 (`sha2`, D4, optional) | NIST StRD and `D1` suites unchanged. Cite references in new `//!` headers (`cargo xtask citations`). |
| `R16.14` format and persistence | R2-3 (row shift on `TableArchetype`), R2-8 (serialize by reference), R2-9 (reuse first read) → R2-6, R2-7 (recovery snapshot/marker) → R2-5 (platform staging primitive) → R2-2, R2-4 (shared delimited scanner with dialect parity tests) → R2-1 (`quick-xml`, D4) | Recovery changes keep the publication order and pass `recovery_and_interop_parity`; archive bytes stay identical (`archive_and_csv`). |
| `R16.15` stats and fit | R16-1 (pass REML matrices by slice), R3-2 (reuse Welch helper), R3-1 (`finite_ci_cell` in `stats::common`), R4-1 (one equation registration per family) | R16-1 and R3-2 edit `anova/sheet/one_way.rs`: wait for `R16.1` (PR #43). |
| `R16.16` render, export and text | R5-1 (streaming `png` writer), R5-3 (`weezl` LZW, D4), R5-5 (shared raster validation), R5-6 (`quick-xml` in the SVG validator, D4), R5-2 (text script metrics, exact copies only), R9-2 (shared TeX token tables) | R5-2 edits `chart/scene.rs` and `render/emf.rs`: coordinate with `R13.7` (embedded images). Determinism and screenshot gates must stay green; encoded bytes may change, decoded pixels and DPI may not. |
| `R16.17` automation codecs | R7-2 (batch version validator, shared envelope fields), R7-1 (`base64`, D4; writes first, then reads per boundary) | Keep each reader's whitespace, padding and trailing-bit rules; reuse RFC 4648 vectors and recovery fixtures. |
| `R16.18` GUI helpers | R15-1 (i18n fallback helper), R15-2 (one modal inspect adapter), R8-1 (one shortcut candidate builder), R10-1 (a11y state transitions), R9-1 (journal preset names), R10-2 (linked-paste TSV formatter), R5-4 (shared raster sizing for MCP and control) | Edits GUI files: schedule **after** the Phase 2 PR for the same file and **before** the Phase 4 split of that file. |

## 8. Phase 4 — GUI module splits (`R16.7`–`R16.11`)

Move-only splits, per the [scope document](r16-gui-maintainability-scope.md). Each starts only when every Phase 2 finding in its file is fixed or explicitly deferred in the audit ledger, and never runs in parallel with an `R13.8` bullet or an `R16.6` descriptor-table change that edits the same file.

| Item | File → module directory | Starts after |
|---|---|---|
| `R16.7` | `egui_shell/data_grid.rs` → shared geometry/scroll, Column grid, multi-archetype grid, Multiple-Variables chips, inline editing, keyboard/clipboard, linked regions, headers, tests | B4, B5, `R16.18` |
| `R16.8` | `egui_shell/stats_modals.rs` → two-group, factorial/MV, nested and multiple t, GLM/Cox/MI, result viewer, shared helpers | B2; `R16.6` descriptor decision |
| `R16.9` | `egui_shell/graph_modals.rs` → dispatcher/footer, Change Graph Type, Format Graph, Format Axes, Pairwise, Bubble, Apply Style, Selected Points, Insert Equation, Page Layout, helpers | B7, B8; before or after `R13.8` graph-dialog bullets |
| `R16.11` (optional) | `trailing_studio.rs` by context, `dialogs_canvas.rs` by dialog family, `i18n.rs` into `i18n/` shards | Phase 2 for those files; `R16.5` callers branch |

Reduce `#[allow(clippy::too_many_arguments)]` only in moved private code, via context structs (closes R15-GUI-08 file by file).

## 9. Collision map with branches already in flight

| Branch (worktree) | Touches | Wait before |
|---|---|---|
| `refactor/R16.5-callers-and-roadmap-archive` | `ROADMAP.md` (R16 section), `transform_modals.rs`, `trailing_studio.rs`, `grid_view.rs`, `dialogs_canvas.rs`, `control_io.rs`, `lib.rs`, `workspace_nav.rs` | Phase 0 roadmap edit (rebase), B1, B6, B7 (studio part), `R16.11`, R5-4 |
| `refactor/R16.5-retire-param-spellings` | engine spellings | Any GUI fix that changes command JSON spellings (use canonical spellings once it lands) |
| `refactor/R16.6-analysis-descriptors` | `xtask/src/docs_site/registries.rs`; future GUI parameter views | `R16.8`, `R16.12` (docs site parts) |
| `feature/R13-wf51-65` | `menus/tooltips.rs`, `views/inspector.rs`, `xtask/src/status.rs`, `ROADMAP.md` | A3 if it touches menu tooltips; Phase 0 (rebase) |
| `perf/R0.32-xtask-fast` | `xtask/src/{brands,ci,main,status}.rs` | `R16.12` |
| PR #43 (`R16.1`) | `crates/stats/src/anova/` | `R16.15` R16-1, R3-2 |

## 10. Suggested waves

| Wave | Runs in parallel | Sequential inside the wave |
|---|---|---|
| 0 | Phase 0 docs PR; `R16.10` (after approval) | — |
| 1 | Phase 1 (one agent, GUI pipeline) ‖ `R16.13`, `R16.14` small bullets, `R16.15`, `R16.16` (R5-1, R5-3, R5-5), `R16.17` R7-2 | A1 → A2 → A3 → A4 → A5 |
| 2 | B1–B10 by file (one agent per file group) ‖ remaining `R16.12`–`R16.17` bullets as their decisions and waits clear | B5 → B6; B7 → B8 |
| 3 | `R16.18` per file ‖ `R16.7`, `R16.8`, `R16.9` once their gates clear | `R16.18` for a file → split of that file |
| 4 | `R16.11` optional splits | — |

Run the full local `cargo xtask ci` once per wave integration (shared log file, per `AGENTS.md`), not per PR.

## 11. Verification by kind of change

| Change | Checks |
|---|---|
| GUI fix | New `EguiHumanPathDriver` regression test; `cargo test -p curvebook-ui-egui` (lib, `human_wf_*`, affected `gui_wf_*`); `cargo test -p curvebook` for host changes; `cargo xtask ui-commands --check`, `screenshots --check`, `status` |
| Numerical refactor | `D1` and NIST StRD suites unchanged (`CURVEBOOK_REQUIRE_CORPUS=1` for fit/stats); `cargo xtask citations` |
| Format/persistence | `archive_and_csv`, `recovery_and_interop_parity`, journal replay fixtures; byte-identical archives where the ledger says so |
| Export encoders | `render_export`, `r1019_format_determinism`, decoded-pixel and DPI checks, `cargo xtask screenshots --check` |
| xtask | Each gate's own tests; `cargo xtask ci policy` |
| Dependency change | `cargo deny --offline check`, `cargo xtask licenses`, `telemetry`, `about.toml` / `THIRD-PARTY-LICENSES.html` |
| Module split | `git diff` shows moves only; crate tests and Clippy `-D warnings`; `cargo check --workspace --all-targets`; `ui-commands --check` |
| Every PR | `cargo xtask ci quick` |

## 12. Tracking and done criteria

- When a PR fixes a finding, change its `Status` in the audit ledger from `open` to `fixed in <PR>` (or `deferred: <reason>`); do the same for maintainability entries.
- `R15.6` is done when every High and Medium `R15-GUI-*` finding is fixed with a human-path regression test or formally deferred, and the `R10.13` desktop walkthrough reruns the affected workflows.
- `R16.7`–`R16.9` are done when each split lands with no behaviour change and the file's open findings are closed.
- `R16.12`–`R16.18` close entry by entry; an entry may be closed as "won't do" with a recorded reason (for example, a rejected dependency).
- This plan and the scope document move to `docs/history/` once their items are in the roadmap.
