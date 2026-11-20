# R16 extension: GUI module splits and R15 ref cleanup (scope)

> **Status:** proposed 2026-11-20 — not yet applied to [`ROADMAP.md`](../ROADMAP.md). This document records the agreed scope so the roadmap edit can be made later.

## Context

Two follow-ups surfaced after the R15 GUI audit:

1. Several `crates/ui-egui` modules are very large (`egui_shell/data_grid.rs`, `egui_shell/stats_modals.rs`, `egui_shell/graph_modals.rs`, `egui_shell/trailing_studio.rs`, `dialogs_canvas.rs`, `i18n.rs`). `R16` ("Core-crate maintainability") already splits oversized core modules (R16.1/R16.2 done the same way) but deliberately kept `crates/ui-egui/**` off-limits until `R15.5`. R15.5 has landed, so R16 can now extend to the GUI crate.
2. Merged R15 branches remain on `origin` (the local slice branches are already gone, and `9ce643a` is already on `origin/main`, so the "when 9ce643a merges/pushes" precondition is met).

Decisions taken with the user: **extend R16 (R16.7+)**; **fix the R15 audit findings in a file before splitting it**; **ref cleanup is scoped only** (no deletion in this change).

This change is documentation only: it scopes claimable roadmap items. No code is split and no refs are deleted.

## Changes

### 1. `docs/ROADMAP.md` — extend R16 (additive edits only)

- **R16 summary row** (milestone table, `| **R16** |` line): widen the title to "Core- and GUI-crate maintainability" and add one clause pointing at `R16.7`–`R16.11`.
- **R16 section intro:** add a short paragraph "GUI crates (added 2026-11-20 after the R15 audit)" stating that the `crates/ui-egui/**` off-limits rule ended with `R15.5`, that GUI items follow the GUI split rules below, and that module sizes are measured with `cargo xtask stats --exact` — **no line counts in prose** (pitfalls §2).
- **GUI split rules (binding for R16.7–R16.9 and R16.11):**
  - Move-only: no behaviour, layout or wording change; no renames of `pub` items. The old file becomes `<name>/mod.rs` and re-exports every currently public item, so imports elsewhere (`apps/curvebook`, `crates/mcp`, tests) are untouched.
  - Every `widget_id`, `command_id`, AccessKit role and `FrameInspectMeta` entry stays byte-identical (MCP `ui_inspect`, `--gui-only` tools and human-path tests match on them).
  - **Fixes first:** an item may start only after the R15 audit findings anchored in its file ([`r15-gui-audit-2026-11-20.md`](../audit/r15-gui-audit-2026-11-20.md)) are fixed or explicitly deferred in the ledger.
  - Frozen engine wire contract and `R16.5` ownership of parameter spellings still apply.
  - Reducing `#[allow(clippy::too_many_arguments)]` is allowed only in moved private code via context structs (mirrors R16.1; addresses R15-GUI-08).
  - Coordination: `R13.8` builds new controls in the same dialogs; a split lands before, or rebases after, any claimed `R13.8` bullet touching that file — never in parallel.
- **New items** appended after `R16.6`, before `## Order and dependencies`, using the existing header format (`Owner · Status · Was · Est. note · Depends on · Gate: none`):
  - **R16.7 Split the data grid by archetype** — `egui_shell/data_grid.rs` → `egui_shell/data_grid/`: shared geometry/scroll/render context, Column single-archetype grid, multi-archetype grid (XY, Grouped, Contingency, Survival, Parts-of-Whole, Nested), Multiple-Variables role chips and calculated columns, inline editing and commits, keyboard/clipboard shortcuts, linked-region strip and guards, headers, tests. Depends on: R15-GUI-06, -11, -12 fixed or deferred.
  - **R16.8 Split the statistics modals** — `egui_shell/stats_modals.rs` → `egui_shell/stats_modals/` by modal family: two-group, factorial/MV design, nested and multiple t-tests, GLM/Cox/multiple-imputation model-and-formula, result viewer, shared tab/formula helpers. `stat_anova_modal.rs` stays as is. Depends on: R15-GUI-03.
  - **R16.9 Split the graph modals** — `egui_shell/graph_modals.rs` → `egui_shell/graph_modals/`: dispatcher/footer/commit, Change Graph Type, Format Graph (series, MV, heatmap), Format Axes (frame/origin, single axis, segments, ticks, additional ticks), Pairwise, Bubble, Apply Style, Selected Points, Insert Equation, Page Layout, record helpers. Depends on: R15-GUI-15, -24, -28; coordinate with `R13.8`.
  - **R16.10 Prune merged R15 branch refs** (Owner: maintainers) — bullets: verify each ref is an ancestor of `origin/main` (`git merge-base --is-ancestor <ref> origin/main`); delete `feature/R15-egui-desktop-overhaul`, `feature/R15.1-{driver,host-raster,policy-fonts}`, `feature/R15.2-{anova-coexist,bench-qa,grid-col}` on `origin` with `git push origin --delete …`; run `git fetch --prune`; never delete a branch checked out in a worktree (`git worktree list`). Record the deleted refs and tip SHAs in the PR, not as counts in prose.
  - **R16.11 Further GUI module splits (optional, one PR each)** — `egui_shell/trailing_studio.rs` by studio context; `dialogs_canvas.rs` dialog-state types by dialog family; `i18n.rs` key tables into `i18n/` shards following the existing `i18n/w5_*.rs` pattern. Same rules; each bullet depends on the R15 findings in its file.
- Leave `R16.1`–`R16.6` text untouched.

### 2. [`r15-gui-audit-2026-11-20.md`](../audit/r15-gui-audit-2026-11-20.md) — cross-reference

Add a short "Follow-ups" section after the Summary listing which findings gate `R16.7`, `R16.8`, `R16.9` and `R16.11`, so the ledger and the roadmap point at each other.

## Known overlap

`refactor/R16.5-callers-and-roadmap-archive` (active worktree) rewrites the R16 header and moves `R16.2`/`R16.6` into `docs/history/roadmap/r16.md`. Our edits stay additive (one row clause, one intro paragraph, new items at the end of the section), so whichever branch lands second resolves a small textual conflict: keep both sets of changes and re-point the new items after the last remaining R16 item.

## Verification when applied (docs only; no test suites)

- `cargo xtask status` — roadmap table keys stay unique (the new items are headings, not table rows, but the R16 row is edited).
- `cargo xtask brands` — new prose stays within branding rules (no external product or format names).
- `cargo xtask pending --count` — confirm the new `- [ ]` bullets are counted under `R16.7`–`R16.11`.
- `git diff --stat` for that change shows only `docs/ROADMAP.md` and the audit ledger.
