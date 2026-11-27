# CurveBook Post-Beta Desktop GUI & Scientific Reporting Improvement Push

> **Scope:** Living engineering and UX specification for the desktop GUI audit fixes and post-**Beta** desktop GUI & scientific-reporting refinement push (`R10.34`–`R10.36`, `R19.6`–`R19.7`, `R20.6`–`R20.22`, and subsequent additions), reconciled against `main` (`eed57fa`). **`R10.34` (already on `main`: `".pzfx files"` / `".prism files"` naming) and `R20.19` (two P1 audit blockers; the `WF-59` offscreen-control production guard already landed on `main` in `ec94ecd` under `R20.4`, with scroll-into-view test verification retained) are Beta-blocking**; all other items tracked here are **non-blocking for Beta** and target the native `egui 0.37` desktop shell (`apps/curvebook`, `crates/ui-egui`) plus scientific result-sheet accuracy and reporting in `crates/doc`, `crates/fit`, and `crates/engine`. All 16 pre-implementation design decisions are locked in the [Resolved Pre-Implementation Decisions](#resolved-pre-implementation-decisions-2026-11-25) ledger below.
> **Related Tracked Docs:** [`../ROADMAP.md`](../ROADMAP.md), [`../design-language.md`](../design-language.md), [`gui-layout-spec.md`](gui-layout-spec.md), [`qol-enhancements.md`](qol-enhancements.md), [`../../AGENTS.md`](../../AGENTS.md).

---

## Purpose & Shared Engineering Invariants

This document preserves the root-cause analysis, concrete design contracts, measured layout budgets, rejected alternatives, single-owner boundaries, code touchpoints, and verification criteria behind each item in the post-Beta GUI improvement push so contributors and agents can claim any item directly without repeating discovery or duplicating work.

All items in this push uphold CurveBook's desktop GUI invariants:

1. **Headless-first mutations:** Document, sheet, graph, and layout changes execute exclusively through `curvebook-engine` (`Session::execute`, `Caller::Ui`); UI layers own only ephemeral view/selection/preview state.
2. **Stable widget IDs & accessibility:** Existing `FrameInspectMeta` widget IDs, `UI_COMMANDS` entries, keyboard shortcuts, and AccessKit roles/labels remain stable unless explicitly noted.
3. **Design-system & WCAG gates:** New controls use canonical `Tokens` and `egui_shell/{ui_kit,widgets}.rs` primitives (single `ActionRank::Primary` CTA per surface, `>= 24 × 24 pt` target sizes, WCAG 2.2 SC 1.4.3 text contrast and SC 1.4.11 `>= 3:1` non-text state contrast across `Light`, `Dark`, and `HighContrast`, enforced by `cargo xtask design-tokens` and `verify_egui_frame_parity`).
4. **Zero-idle-repaint discipline:** Request repaints on actual state transitions and async completions, never via continuous idle spin loops (`R17.3`).
5. **Localization completeness:** Any new user-facing copy is added to `crates/ui-egui/locales/catalog.tsv` across all shipped locales (`cargo xtask i18n-report`).
6. **Micro-copy & link-affordance discipline:** All newly introduced button labels follow [`R20.15`](#14-r2015-eliminate-trailing-ellipses-on-action-buttons-and-unify-ellipsis-semantics) (no trailing `…` on action buttons) and [`R20.14`](#13-r2014-reserve-blue-foreground-text-strictly-for-clickable-links) (blue foreground text reserved strictly for clickable links via `ui_kit::link_button`).
7. **Batched screenshot refresh:** Defer `cargo xtask screenshots` updates to a single end-of-push refresh (rather than churning golden PNGs on every intermediate PR).

---

## Tracked Items Index

| ID | Area | Summary | Dependencies | Primary Files |
|---|---|---|---|---|
| [`R10.34`](../ROADMAP.md#r1034-main-gui-file-extension-naming-for-imported-project-files-beta-gate) | **Beta-Gate** Import Naming *(on `main`)* | Replace opaque `"XML Project"` / `"External Archive"` labels with `".pzfx files"` / `".prism files"` across menus, pickers, wizard, Welcome Hub, and `catalog.tsv` (write new action labels dot-free per `R20.15` Option B; since `323cf38`, `command_palette.rs:930–944` routes `.prism`/`.pzfx` directly to native picker + `ImportProjectMode`) | None *(**Beta gate** / Wave 0; spec lives directly in `ROADMAP.md#r1034`)* | `crates/format/src/registry.rs`, `crates/engine/src/cmd/table.rs`, `crates/engine/src/cmd/table/import.rs`, `apps/curvebook/src/desktop_host.rs`, `import_wizard.rs`, `welcome_hub.rs`, `locales/catalog.tsv` |
| [`R10.35`](#1-r1035-portfolio-gallery-live-thumbnails-batch-workflows--navigation-overhaul) | Portfolio Gallery | Live cached thumbnails, batch/split/print/context-menu workflows, discoverable selection, sorting & footer zoom | `R20.16` recommended (context menu); complements `R10.31` | `center_stage.rs`, `workspace_nav.rs`, `status_footer.rs`, `leading_dock.rs` |
| [`R10.36`](#2-r1036-project-navigator-collapsed-rail--section-vector-icons) | Project Navigator Rail | Replace hardcoded English initials on the `36 pt` collapsed rail (`D \| I \| R \| G \| L \| F`) with shared vector icons & distinct `Σ` Results glyph | Complements `R20.12` | `leading_dock.rs` |
| [`R19.6`](#3-r196-fix-hougaard-skewness-sign-mismatch-and-false-zero-for-uncomputed-entries) | Fit Diagnostics Bugfix | Fix Hougaard `\|g1\|` signed-value mismatch and replace false `0.0` uncomputed entries (`cov.ambiguous`, `k_free > 10`, `df == 0`) with `f64::NAN` | None *(immediate standalone bugfix)* | `crates/fit/src/{diagnostics.rs,solver/fit.rs,solver/results.rs,result.rs}` |
| [`R19.7`](#4-r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics) | Fit Scientific Reporting | Per-dataset `Goodness of Fit` columns, clean `R²`/status footnote, `"Not determined"`/`"Unstable"` default cell suppression, failing-dataset summary & `Y = Mean` null-model comparison | Coordinates with `R19.1`, `R20.6`, `R20.10` | `crates/fit/src/result.rs`, `crates/fit/src/solver/`, `crates/engine/` |
| [`R20.6`](#5-r206-consolidate-duplicate-nonlinear-fit-parameter-identifiability-warning-banners) | ResultSheet Banners | Merge overlapping top `ambiguity_callout` and bottom `QOL-7` diagnostic banner into one dataset-named banner with compact actions (`Constraints`, `Compare vs. Horizontal Line`, `Why?`, `Dismiss`) | Consumes `R19.7` when present | `results_view.rs`, `center_stage.rs`, `workspace_dialogs.rs` |
| [`R20.7`](#6-r207-overhaul-analysis-sidebar-methods-tab-preview-manuscript-prose--export-wiring) | Analysis Sidebar (Methods) | Manuscript-ready Methods default + `Full Report` toggle, clean Markdown, selectable full-height preview, `Save`, state hygiene | None *(standalone)* | `trailing_studio/results_interpret.rs`, `crates/engine/src/methods_report.rs` |
| [`R20.8`](#7-r208-smooth-data-grid-multi-cell-drag-selection-perimeter-highlight--edge-auto-scroll) | Data Grid Selection | Immediate repaint on intent commit, clamped drag + drag-back + edge auto-scroll, single outer selection perimeter, header/X drag | Complements `R20.11` | `data_grid/{shared,multi_archetype,headers}.rs`, `apps/curvebook/src/gui_window.rs` |
| [`R20.9`](#8-r209-unified-top-toolbar-with-os-native-menu-placement) | Top Chrome | Single `36–38 pt` unified toolbar when `host_draws_menu_bar == true`, thin `26–28 pt` menu bar above identical toolbar otherwise | `R20.16` recommended (popups) | `top_chrome.rs`, `top_bar_fit.rs`, `layout.rs` |
| [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting) | ResultSheet Table & Header | CI-first per-column widths, borderless underline tabs (content-first, left) vs. quiet ghost `Parameters`/`Copy` (right) & `New Graph from Tab`, plus `CiBound::Unavailable` + worded open/unavailable CIs retiring `???` | Coordinates with `R19.1`, `R19.7` | `results_view.rs`, `ui_kit.rs`, `results_hooks.rs`, `crates/doc/src/result.rs`, `crates/fit/src/result.rs` |
| [`R20.11`](#10-r2011-focused-vs-unfocused-split-pane-selection-xy-selection-sync--header-coordinate-disambiguation) | Split-View Focus & Selection | Propagate `pane_focused`, paint unfocused retained selections via `selection_unemphasized_bg`, fix X/Y cell selection desync, scope `R{r}C{c}` header badges | Complements `R20.8` | `center_stage.rs`, `data_grid/{mod,multi_archetype,shortcuts,inline_edit}.rs`, `results_view.rs` |
| [`R20.12`](#11-r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown) | Project Navigator & Family | Two-line disambiguation subtitles (`Graphs`, `Results`, duplicate titles in Navigator/Family/split selector), labeled `widgets::dropdown` kind filter & HIG source-list `Shift`/`⌘` multi-select | Complements `R10.36`, `R10.35` | `workspace_nav.rs`, `leading_dock.rs`, `center_stage.rs` |
| [`R20.13`](#12-r2013-live-desktop-font-installation-chrome-border-cleanup-copy-polish--content-aware-initial-split) | Desktop Fonts, Chrome & Split | Fix `gui_window.rs` bypassing `install_font_definitions`, fix macOS `⌘ ⇧ ⌥ ⌃` `Apple Symbols` fallback + `11.5–12.0 pt` shortcut pills, clean remaining resting borders in `top_chrome.rs`/`status_footer.rs`, drop `[Inferential Report]`/`(24×5)`/`"Archetype"`/`"Xy"`/`"Rng="`/raw `Cmd+T`, add content-aware initial split ratio | `R20.10` (for initial split width) | `apps/curvebook/src/{gui_window,main}.rs`, `fonts.rs`, `top_chrome.rs`, `command_palette.rs`, `status_footer.rs`, `results_view.rs`, `center_stage.rs` |
| [`R20.14`](#13-r2014-reserve-blue-foreground-text-strictly-for-clickable-links) | Link Affordance & Token Policy | Reserve blue foreground text strictly for clickable links (`ui_kit::link_button`), retarget `excluded_cell_text` + non-link sites off blue, enforce via `xtask design-tokens` | Complements `R20.13` | `theme.rs`, `ui_kit.rs`, `crates/ui-egui/src/egui_shell/*`, `xtask/src/design_tokens.rs` |
| [`R20.15`](#14-r2015-eliminate-trailing-ellipses-on-action-buttons-and-unify-ellipsis-semantics) | Action Labels & Ellipsis Policy | Strip trailing `…`/`...` from action buttons (Option A: buttons only; Option B: buttons + menus), fix `"More… X"` labels, guard via `verify_egui_frame_parity` | Governs button copy push-wide | `crates/ui-egui/src/{egui_shell/*,menus/*}`, `locales/catalog.tsv`, `driver.rs` |
| [`R20.16`](#15-r2016-unify-dropdown-overflow--context-menus-with-content-measured-sizing-and-popup-parity-gates) | Popups & Menu Geometry | Shared `ui_kit::action_menu_popup` replacing `10` ad-hoc `Area::new` menus (`Split`, overflow, footer, context), 5 `verify_egui_frame_parity` popup checks & `xtask` ratchet | Complements `R16.19` | `ui_kit.rs`, `top_chrome.rs`, `results_view.rs`, `status_footer.rs`, `leading_dock.rs`, `canvas_view.rs`, `data_grid/`, `driver.rs`, `xtask/` |
| [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect) | Feedback Redirect & LOC Cut | Replace in-app `ActiveDialog::Feedback` modal, query builder & Markdown generator (~1,100 LOC deletion) with direct browser redirect to GitHub's issue template chooser | None *(Wave 0 standalone; simplifies `R20.14`, `R20.15`)* | `feedback.rs`, `os_integration.rs`, `app_modals.rs`, `control_io.rs`, `lib.rs`, `apps/curvebook-web/src/lib.rs`, `locales/catalog.tsv` |
| [`R20.18`](#17-r2018-fix-brand-mark-sizing--mipmap-aliasing-and-make-logo-the-home-button) | Brand Mark & Logo-as-Home | Trim double-letterboxed brand PNG to natural `1.43:1` ratio, CPU premultiplied-alpha downsample per physical pixel size (fixing `egui-wgpu` mipmap aliasing), and make the top-left logo act as `toolbar.home` | Coordinates with `R20.9` (`top_chrome.rs`) | `brand.rs`, `top_chrome.rs`, `welcome_hub.rs`, `app_modals.rs`, `assets/app-icon/` |
| [`R20.19`](#18-r2019-fix-desktop-gui-audit-p1-blockers--wf-59-offscreen-layout-controls-beta-gate) | **Beta-Gate** Audit Blockers | Fix split-table Delete/shortcut fan-out (P1) and Analyze picker `Col A..H` titles & empty-selection boundary (P1; initial check fixed in `eed57fa`, `WF-59` guard landed in `ec94ecd`, scroll-into-view test retained) | None *(**Beta gate** / Wave 0 immediate blockers; feeds `R20.11`)* | `data_grid/shortcuts.rs`, `analyze_dialog.rs`, `stats_modals/analyze_picker.rs`, `trailing_studio/page_layout.rs` |
| [`R20.20`](#19-r2020-fix-desktop-gui-audit-p2-split-focus-cycling-per-sheet-result-tabs-context-menu-copy--first-run-fit-cancel) | Audit P2 Split, Tabs & Cancel | Fix compact split `F6`/`Shift+F6` stall, finish per-sheet Result tab wiring (`per_sheet_active_tab` landed in `eed57fa`), right-click outside results selection copy targeting, and first-run nonlinear fit Cancel behavior | Coordinates with `R20.10`, `R20.11`, `R20.16` | `center_stage.rs`, `results_view.rs`, `intents/`, `fit_modal.rs`, `analyze_picker.rs` |
| [`R20.21`](#20-r2021-structured-hover-tooltips-for-commands-scientific-readouts-navigator--inspector) | Structured Hover Tooltips | Canonical `ui_kit::show_tooltip` card (Title + shortcut pill + 1-line summary + why-disabled), `450–500 ms` delay, no-echo rule, scientific Results/Grid headers & precision, Navigator/Family provenance & Inspector tips | Recommended after `R20.9`, `R20.10`, `R20.12` | `ui_kit.rs`, `tokens.rs`, `top_chrome.rs`, `menus/tooltips.rs`, `results_view.rs`, `leading_dock.rs`, `trailing_studio/`, `data_grid/multi_archetype.rs` |
| [`R20.22`](#21-r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) | Apple HIG Dialogs & macOS Menus | Fix `[Don't Save]   [Cancel] [Save]` alert button order + lightweight `About` panel, add macOS `Window` submenu, deduplicate `About` in `Help`, unify `Settings`/`Preferences` & audit `⌘T` overloading | Coordinates with `R20.15`, `R20.17` | `app_modals.rs`, `apps/curvebook/src/mac_menu.rs`, `menus/{menu_bar,shortcuts,ui_commands}.rs`, `r15_9_visual_design_tokens.rs` |

---

## Execution Waves & File-Ownership Map

To avoid merge conflicts across shared hotspot files (`results_view.rs`, `center_stage.rs`, `top_chrome.rs`, `leading_dock.rs`, `data_grid/multi_archetype.rs`), land work in these parallel-safe waves:

| Wave | Parallel Tracks (Disjoint Primary Files) | Rationale |
|---|---|---|
| **Wave 0 — Beta Blockers, Immediate Bugfixes, Deletions & Primitives** | • **`R20.19`** (**Beta gate**: P1 split `Delete` fan-out, P1 Analyze picker inversion; `WF-59` offscreen-control guard landed in `ec94ecd`, scroll-into-view test retained)<br>• **`R10.34`** (**Beta gate**, on `main`: `".pzfx files"` / `".prism files"` naming across menus, pickers, wizard, `human_wf_common`/`human_wf_import_export` & `catalog.tsv`, dot-free per `R20.15` Option B)<br>• **`R19.6`** (`crates/fit/` Hougaard skewness bugfix)<br>• **`R20.16`** (`ui_kit::action_menu_popup` + 10 menu migrations + popup parity gates)<br>• **`R20.17`** (replace Feedback modal with direct GitHub redirect; ~1,100 LOC deletion)<br>• **`R20.7`** (`results_interpret.rs` + `methods_report.rs`) | Land **`R20.19` + `R10.34` first** to clear all Beta-gate GUI items (writing `R10.34`'s new labels dot-free avoids touching those `catalog.tsv` rows twice in `R20.15`); sibling Wave 0 items touch disjoint files (`crates/fit`, `ui_kit`, `feedback`, `results_interpret`), and `R20.16`/`R20.17` simplify later waves. |
| **Wave 1 — Results Credibility & Table/Header Overhaul** | • **`R19.7`** (`crates/fit/src/result.rs` scientific outputs)<br>• **`R20.10`** (`results_view.rs` CI-first sizing, accurate `ellipsized` metadata, borderless underline tabs, quiet ghost `Parameters`/`Copy`/`New Graph from Tab`, `CiBound::Unavailable` + explicit CI copy)<br>• **`R20.20` (Results slice)** (per-sheet Result tab state & right-click copy targeting in `results_view.rs`, plus first-run fit Cancel in `fit_modal.rs`)<br>• **`R20.6`** (unified dataset-named warning banner in `results_view.rs`/`center_stage.rs`) | Sequence `R20.10` + `R20.20` (Results) → `R20.6` (paired with `R19.7`) so `results_view.rs` is rewritten cleanly once; fixes both the #1 visual complaint and the P2 Results audit bugs together. |
| **Wave 2 — Split Focus, Grid Selection, Navigator & Top Chrome** | • **Track A (Grid + Split):** `R20.8` → `R20.11` + `R20.20` (compact split `F6`/`Shift+F6` in `center_stage.rs`) (`data_grid/`, `center_stage.rs`)<br>• **Track B (Navigator):** `R10.36` + `R20.12` (subtitles, dropdown & source-list multi-select in `leading_dock.rs`, `workspace_nav.rs`)<br>• **Track C (Top Chrome & Brand):** `R20.18` + `R20.9` (`brand.rs`, `top_chrome.rs`, `top_bar_fit.rs`, `layout.rs`, `welcome_hub.rs`, `app_modals.rs`) | Three completely disjoint UI regions (`data_grid`/`center_stage`, `leading_dock`, `top_chrome`/`brand`) that can run in parallel; `R20.18`'s logo-as-Home button directly feeds `R20.9`'s unified toolbar width budget. |
| **Wave 3 — Desktop Fonts, HIG Dialogs/Menus, Tooltips, Guardrails & Gallery** | • **`R20.13`** (live desktop `install_font_definitions`, macOS `⌘ ⇧ ⌥ ⌃` symbol fallback + `FontTweak` + `11.5–12.0 pt` shortcut pills, remaining `top_chrome`/`status_footer` resting borders, `[Inferential Report]`/`(24×5)`/`"Archetype"`/`"Xy"`/`"Rng="`/`⌘T` cleanup, content-aware initial split ratio using `R20.10`)<br>• **`R20.22`** (HIG alert/modal button order, lightweight `About`, macOS `Window` menu, `About` dedup, `Settings`/`Preferences` & `⌘T` parity)<br>• **`R20.21`** (structured `ui_kit::show_tooltip` cards across commands, scientific Results/Grid readouts, Navigator/Family & Inspector)<br>• **`R20.14`** (blue-text-means-link sweep + `ui_kit::link_button` + `design-tokens` check)<br>• **`R20.15`** (button ellipsis sweep + `verify_egui_frame_parity` check)<br>• **`R10.35`** (Portfolio Gallery live thumbnails & batch workflows) | Finish HIG polish (`R20.22`), tooltips (`R20.21`), and global design-system guardrails (`R20.14`, `R20.15`) after major UI restructures settle, then run **one batched `cargo xtask screenshots` refresh**. |

---

## 1. `R10.35` Portfolio Gallery live thumbnails, batch workflows & navigation overhaul

- **Roadmap entry:** [`ROADMAP.md#r1035`](../ROADMAP.md#r1035-portfolio-gallery-live-thumbnails-batch-workflows--navigation-overhaul) · **Gate:** Post-**Beta** · **Complements:** [`R10.31`](../ROADMAP.md#r1031-desktop-graph-image-gallery--linked-region-ui-parity-fixes), [`R20.16`](#15-r2016-unify-dropdown-overflow--context-menus-with-content-measured-sizing-and-popup-parity-gates)

### Current Problem & Root Causes
Today, `render_portfolio_gallery` (`crates/ui-egui/src/egui_shell/center_stage.rs`) acts mainly as a text-filtered sheet launcher with a single batch button (`Apply Style From`):
1. **Placeholder thumbnails (`paint_gallery_thumbnail`):** Every `GraphSheet` card draws the exact same hardcoded 3-segment polyline; `LayoutSheet` cards draw up to 6 hollow wireframe rectangles; `DataSheet`, `ResultSheet`, and `InfoSheet` cards draw 4 blank horizontal lines. `color_tag` dots are painted only for `Data`/`Results`/`Info` and omitted on `Graphs` and `Layouts`; `Frozen`, `Uninitialized`, and `Warning`/`Stale` states are invisible.
2. **Missing batch & comparison actions:** Aside from `Apply Style From`, none of CurveBook's existing multi-sheet capabilities are exposed in the Gallery toolbar or on cards—even though `export.pdf` already accepts `sheet_ids`, `PrintJobSpec` already holds `sheet_ids: Vec<SheetId>`, `PageLayoutComposerState::arrange_grid` already computes grid layouts for `N` graphs, and `ui.split.toggle` already accepts explicit `primary_sheet_id` and `secondary_sheet_id`.
3. **Hidden multi-select & identical active/selected styling:** `Cmd/Ctrl+Click` and `Shift+Click` have no visible checkbox affordance, no `"N selected"` readout, and no `Select All` / `Clear` actions. Both `card.active` and `card.selected` render with the identical `tokens.active_bg` fill and `tokens.accent` border, making active vs. multi-selected indistinguishable.
4. **Partially wired footer zoom & stale readout:** `eed57fa` wired `- / % / +` (`status_footer.rs:1288–1358`) to `views.center_stage.gallery_zoom_pt` (`120..=260 pt`) when `flags.gallery_open`, but `Fit` in gallery mode is still missing and `status.summary` (`status_footer.rs:1239`) still prints total `cards.len()` instead of filtered + selected counts.
5. **Divergent card builders & ordering:** `build_filtered_gallery_cards` in `center_stage.rs` ignores `WorkspaceNavState::sync_gallery` (`workspace_nav.rs`), dropping graph kind, `"Uninitialized"`, and data row counts, and orders sections differently (`Graphs, Layouts, Data, Results, Info` vs. `Data, Info, Results, Graphs, Layouts`), causing `selected_gallery_ids()` order to disagree with `Shift+Click` visual order.
6. **Missing Navigator quick-launch & keyboard nav:** The **Folder Gallery** section-header icon specified in [`gui-layout-spec.md`](gui-layout-spec.md) §4.1 is not wired in `leading_dock.rs`, and the Gallery card grid has no arrow-key / `Space` / `Enter` / `Cmd/Ctrl+A` navigation.

### Target Design & Interaction Contract
- **Live thumbnails & status cues:**
  - `GraphSheet` and `LayoutSheet` cards reuse `canvas_view.rs`'s `resolve_cached_scene` + background `AsyncRasterWorkerHandle` (`cb-canvas-raster`), cached by `(sheet_id, mutation_seq, preview_filter)` with `paint_chart_scene_vector_preview` as instant frame-1 fallback.
  - `DataSheet` cards draw compact column headers, sample cell values, and row×col dimensions; `ResultSheet` cards draw analysis kind, assumption status badge (`Pass`/`Warn`/`Violated`), and a headline readout; `InfoSheet` cards draw constant key–value pairs / note snippet.
  - Render `color_tag` dots across all five sheet kinds plus `Frozen`, `Uninitialized`, and `Warning`/`Stale` badges.
- **Batch & comparison actions:**
  - Complement `R10.31` (`Export Selected`, `Place Selected onto New Layout`, `ExportFigure` `Target Scope` toggle) with:
    - **Compare in Split View** when 2 sheets are selected (`ui.split.toggle` with `primary_sheet_id` and `secondary_sheet_id`),
    - **Print Selected** (populating `PrintJobSpec::sheet_ids`),
    - batch CSV export for selected `DataSheet`s,
    - batch `Duplicate`, `Freeze`/`Unfreeze`, and `Delete` (with downstream cascade confirmation), and
    - card right-click context menu matching the Navigator sheet menu (`Rename`, `Duplicate`, `Duplicate Family`, `Freeze/Unfreeze`, `Export`, `Print`, `Delete`) rendered via `ui_kit::action_menu_popup` (`R20.16`).
- **Selection, filtering, sorting & zoom:**
  - Add a visible hover/selected corner checkbox on each card to decouple toggling selection from single-click-to-open; visually distinguish `active` (current workspace sheet) from `selected` (batch selection).
  - Show `"N selected"` with `Select All` / `Clear` toolbar controls, per-folder counts on filter pills (`Graphs (6)`), a **Current Family** (DAG lineage) filter toggle, a **Sort** selector (Navigator order, Name A–Z, Type, Series/Rows), and collapsible section headers in `All` view when `Sort` is `Navigator order` or `Type` (automatically flattening to one continuous grid for `Name A–Z` / `Series/Rows`).
  - Complete bottom-footer gallery zoom (`Fit` alongside `eed57fa`'s `- / % / +` wiring at `status_footer.rs:1288–1358`) and show filtered + selected counts in `status.summary` (`status_footer.rs:1239`).
  - Unify card construction and ordering between `workspace_nav.rs` and `center_stage.rs`.
  - Add arrow-key/`Space`/`Enter`/`Cmd/Ctrl+A` grid navigation and add the **Folder Gallery** quick-launch icon to Navigator section headers (`leading_dock.rs`).

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{center_stage,canvas_view,status_footer,leading_dock}.rs`, `crates/ui-egui/src/workspace_nav.rs`, `crates/ui-egui/src/egui_shell/app_modals.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** `EguiHumanPathDriver` coverage for live thumbnails, checkbox & `Shift+Click` selection, batch Split/Print/Layout/Export actions, context menu, footer zoom routing, and `cargo xtask design-tokens` / `i18n-report`.

---

## 2. `R10.36` Project Navigator collapsed rail & section vector icons

- **Roadmap entry:** [`ROADMAP.md#r1036`](../ROADMAP.md#r1036-project-navigator-collapsed-rail--section-vector-icons) · **Gate:** Post-**Beta** · **Complements:** [`R20.12`](#11-r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown)

### Current Problem & Root Causes
Every sheet row in the Project Navigator and Family Lineage trees (`crates/ui-egui/src/egui_shell/leading_dock.rs`) already draws custom vector icons via `draw_sheet_vector_icon`:
- **Data:** 2×2 table grid
- **Info:** `i` in a circle
- **Results (`Analyses`):** three rising vertical bars on a baseline
- **Graphs:** L-axes with a rising sigmoidal curve
- **Layouts:** 2-over-1 multi-panel page mosaic

Two issues remain in Navigator iconography and collapsed-rail chrome:
1. **Hardcoded English initials on the `36 pt` collapsed rail (`D`, `I`, `R`, `G`, `L`, `F`):** A dropdown cannot fit inside the `36 pt` rail, and single English letters leak across shipped locales (German *Ergebnisse*/*Diagramme*, French *Graphiques*/*Mises en page*, CJK, Russian, etc.) with uneven proportional glyph widths (`I` vs. `G`). *(The expanded `240 pt` Navigator's inline `All | D | I | R | G | L` filter bar is owned by [`R20.12`](#11-r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown), which replaces it with a labeled `widgets::dropdown` reusing these same vector icons beside full localized labels.)*
2. **Visual ambiguity of `SheetSection::Analyses`:** `draw_sheet_vector_icon` currently draws three rising bars for `Results`, which reads like a bar chart when placed beside the `Graphs` curve icon in tree rows, the collapsed rail, or dropdown items.

### Target Design & Interaction Contract
- **Distinct Results (`Analyses`) glyph:** Replace the three rising bars in `draw_sheet_vector_icon` (`SheetSection::Analyses`) with a crisp vector `Σ` (sigma) glyph so `Data` (grid), `Info` (`i`), `Results` (`Σ`), `Graphs` (axes + curve), and `Layouts` (page mosaic) are immediately distinct across tree rows, the collapsed rail, folder headers, and `R20.12`'s filter dropdown items.
- **Shared vector icons on the `36 pt` collapsed rail & folder headers:**
  - Render `Data`, `Info`, `Results`, `Graphs`, and `Layouts` in the collapsed `36 pt` rail via `draw_sheet_vector_icon` (`12×12` or `14×12 pt`).
  - Replace the collapsed rail's `F` with a 3-node branching lineage-tree icon for the Family pane.
  - Echo the shared section icon on collapsible folder headers in the expanded Navigator so even empty folders teach the icon mapping.
- **Contrast, DPI snapping & accessibility:**
  - Switch icon stroke color cleanly across selected (`tokens.selected_accent_text()`), idle (`tokens.icon` / `tokens.text_strong`), and disabled (`tokens.text_disabled`), meeting WCAG 2.2 SC 1.4.11 (`>= 3:1`) in `Light`, `Dark`, and `HighContrast`.
  - Keep `1.25–1.5 pt` strokes pixel-snapped and legible at `1.0x` and `1.5x` DPI.
  - Preserve full localized hover tooltips, AccessKit labels, and widget IDs (`nav.rail.*`, `family.rail.toggle`).

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/leading_dock.rs` (`draw_sheet_vector_icon`, collapsed rail, optional folder headers).
- **Tests:** Theme non-text contrast checks, `EguiHumanPathDriver` collapsed-rail interaction tests, and visual verification at `1.0x` and `1.5x` DPI.

---

## 3. `R19.6` Fix Hougaard skewness sign mismatch and false zero for uncomputed entries

- **Roadmap entry:** [`ROADMAP.md#r196`](../ROADMAP.md#r196-fix-hougaard-skewness-sign-mismatch-and-false-zero-for-uncomputed-entries) · **Gate:** Post-**Beta** *(recommended as an immediate standalone bugfix PR)*

### Current Problem & Root Causes
Two scientific correctness bugs hide behind the `"Hougaard |g1|"` column of nonlinear regression result tables:
1. **Sign mismatch with column header:** `compute_hougaard_skewness_into` (`crates/fit/src/diagnostics.rs:522–554`) computes signed $g_1$; `FittedParameterSummary::hougaard_g1` (`crates/fit/src/solver/results.rs:281`) and `append_nonlinear_parameter_tab` (`crates/fit/src/result.rs:709–765`) advertise `"Hougaard |g1|"`, yet `result.rs:741,743,753,763` outputs the signed value (e.g., `-13.0577`) while the checklist check (`result.rs:1350`) takes `.abs()`.
2. **Remaining false `0` for uncomputed values (partially fixed in `eed57fa`):** `eed57fa` (`crates/fit/src/solver/fit.rs:571,724`, tested in `crates/fit/tests/rem_solver_result.rs:80`) now sets `*h = f64::NAN` for reduced/reconciled unstable parameters and gates computation on `full_rank && !cov.ambiguous && df > 0` (`L659`). However, `fit_plot_function_only` (`solver/fit.rs:139`: `hougaard_g1: if is_robust { f64::NAN } else { 0.0 }`), newly factored `compute_fit_hougaard` (`solver/fit.rs:519–547`, **`L531, L534, L542`**: `!enabled`, `k_free > 10`, `cov_unscaled.is_none()`, `sqrt_w.is_none()`), and `compute_hougaard_skewness_into` (`diagnostics.rs:522–554`, **`L536, L540, L545, L552`**) still return **`0.0`**—printing `0` ("perfectly linear") on uncomputed rows and global fits with $> 10$ free parameters, contradicting `FittedParameterSummary::hougaard_g1` (`NaN` if uncomputed).

### Target Design & Interaction Contract
- **Uncomputed sentinel = `f64::NAN`:** Return `f64::NAN` on every uncomputed or failed entry across `compute_hougaard_skewness_into` (`diagnostics.rs:522–554`, `L536,540,545,552`), `fit_plot_function_only` (`solver/fit.rs:139`), and `compute_fit_hougaard` (`solver/fit.rs:519–547`, `L531,534,542`) so `ResultCell::Number { value: NaN, .. }` renders blank / `"—"` rather than `0`.
- **Consistent magnitude (`"Hougaard |g1|"`):** Keep `compute_hougaard_skewness_into` returning signed $g_1$ (matching Hougaard 1985 / Ratkowsky 1990) in solver/JSON/tooltip metadata, and apply `.abs()` consistently when populating the `"Hougaard |g1|"` table cell (`result.rs:741,743,753,763`, matching `solver/results.rs:281` and `result.rs:1350`).

### Code Touchpoints & Verification
- **Files:** `crates/fit/src/diagnostics.rs`, `crates/fit/src/solver/fit.rs`, `crates/fit/src/solver/results.rs`, `crates/fit/src/result.rs`.
- **Tests:** Unit and result-sheet regression tests asserting (a) `cov.ambiguous`, `df == 0`, evaluation failure, and `k_free > 10` global fits emit `f64::NAN` (never `0.0`) and blank / `"—"` cells, and (b) finite Hougaard cells match the column header's sign/magnitude contract and checklist evaluation.

---

## 4. `R19.7` Nonlinear fit report clarity, per-dataset goodness of fit & null-model diagnostics

- **Roadmap entry:** [`ROADMAP.md#r197`](../ROADMAP.md#r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics) · **Gate:** Post-**Beta** · **Coordinates with:** [`R19.1`](../ROADMAP.md#r191-descriptive-statistics-resultui-parity--nonlinear-identifiability-formatting), [`R20.6`](#5-r206-consolidate-duplicate-nonlinear-fit-parameter-identifiability-warning-banners) (banner UI), [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting) (CI cell formatting)

### Current Problem & Root Causes
1. **Pooled-only `Goodness of Fit` & footnote hide clean fits inside failing ones:** `append_nonlinear_parameter_tab` (`crates/fit/src/result.rs:709–765`) and `append_nonlinear_gof_tab` (`crates/fit/src/result.rs:837–885`) emit only pooled global `DF`, `R²`, `RSS`, `Sy.x`, `AICc`, and `BIC`—even when all datasets in a multi-dataset fit are fitted independently with zero shared parameters. Mixing a clean sigmoidal response with 5 flat compounds into one `R2 = 0.656` makes the summary uninterpretable. Footnotes also use ASCII `R2` and mixed `:`/`=` punctuation.
2. **Divergent iterates in primary cells (`~-21199.8665`):** `IdentifiabilityMode::IdentifyUnstableParameters` already replaces unstable estimates with `"Unstable"` (and `eed57fa` mirrored `unstable_as_text` in parameter summaries while omitting unstable scalars at `crates/fit/src/result.rs:1591–1673`), but it is not the default (`R19.1` queued reviewing that default), so researchers still see nonsensical numbers like `~-21199.8665` prominently in the Best-fit column. *(Also note `eed57fa` `EXT-01` added `RawXScaleWarning` checklist entries at `result.rs:1006–1220`.)*
3. **No failing-dataset list or flat-vs-partial diagnostic:** Ambiguity summaries do not list which datasets failed, and naively reporting `"no concentration response detected"` on every unbounded 3PL/4PL fit would be scientifically wrong on partial curves whose plateau or $\mathrm{EC}_{50}$ lies outside the tested concentration range.

### Target Design & Interaction Contract
- **Per-dataset `Goodness of Fit` columns (`crates/fit/src/result.rs`):**
  - For independent multi-dataset fits (no shared parameters), emit one column per dataset in `Goodness of Fit` (`DF`, `R²`, `RSS`, `Sy.x`, `AICc`, `BIC`, runs/normality/homoscedasticity). Retain a pooled global column only when global shared parameters are fitted.
  - Clean up the parameter-tab footnote: render Unicode `R²`, use consistent separators, and summarize multi-dataset convergence cleanly (e.g., `"3 of 8 converged; 5 ambiguous"`).
- **Safe default display for unstable estimates (aligned with `R19.1`):**
  - Display **`"Unstable"`** in the primary Best-fit cell when identifiability flags an unstable parameter rather than printing wild iterates such as `~-21199.8665`.
  - Preserve the raw solver iterate in cell metadata / hover tooltips, Inspect Details, and JSON outputs so advanced users debugging starting values or parameter bounds can still inspect *why* the fit diverged.
- **Failing-dataset payload & scientifically grounded `Y = Mean` null-model diagnostic (consumed by `R20.6`):**
  - Expose the failing dataset names on the result checklist/diagnostic summary so `R20.6`'s unified banner reads e.g. `"Ambiguous / unbounded fits in 5 of 8 datasets (VEHICLE, CLOZAPINE, OLANZAPINE, ARIPIPRAZOLE, CARIPRAZINE)"`.
  - **Diagnostics-only flat vs. partial curve distinction (no auto-exclusion):** Never automatically claim `"no concentration response detected"` without testing the null model, and never auto-exclude datasets. Reuse existing F-test / `AICc` model comparison to offer an explicit **`Compare vs. Horizontal Line`** (`Y = Mean`) diagnostic:
    - Report *"Not distinguishable from horizontal line (`P = …`, `ΔAICc = …`)"* **only** when the constant model wins or is not rejected.
    - Otherwise report *"Incomplete / unbounded curve (plateau or EC50 outside tested range)"*.
- *(Ownership boundary: `CiBound::Unavailable` and worded CI formatting retiring `???` are owned by [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting); consolidating the top + bottom banners into one UI strip is owned by [`R20.6`](#5-r206-consolidate-duplicate-nonlinear-fit-parameter-identifiability-warning-banners).)*

### Code Touchpoints & Verification
- **Files:** `crates/fit/src/result.rs`, `crates/fit/src/solver/` (null-model comparison hook), `crates/engine/` (if invoking horizontal-line comparison via command).
- **Tests:** Multi-dataset `crates/fit` tests verifying per-dataset GoF columns for independent fits vs. pooled column for global shared fits, footnote `R²` & convergence summary formatting, unstable-estimate `"Not determined"`/`"Unstable"` cell output with raw-iterate tooltip/JSON retention, failing-dataset summary payload, and flat vs. partial-curve null-model classification.

---

## 5. `R20.6` Consolidate duplicate nonlinear-fit parameter identifiability warning banners

- **Roadmap entry:** [`ROADMAP.md#r206`](../ROADMAP.md#r206-consolidate-duplicate-nonlinear-fit-parameter-identifiability-warning-banners) · **Gate:** Post-**Beta** · **Consumes:** [`R19.7`](#4-r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics)

### Current Problem & Root Causes
When a nonlinear regression fit has ambiguous, wide-CI, or unstable parameters, a `ResultSheet` currently displays **two** warning banners simultaneously for the same condition—sandwiching the result table top and bottom—while **Interpret › Assumptions** also displays the checklist card:
1. A non-dismissible top callout (`result.ambiguity_callout` in `crates/ui-egui/src/egui_shell/results_view.rs`), and
2. The bottom `QOL-7` diagnostic banner (`result.diagnostic_banner` in `crates/ui-egui/src/egui_shell/center_stage.rs`, built from `rs.checklist` via `AssumptionDiagnosticBanner::from_result` in `crates/ui-egui/src/dialogs_canvas/workspace_dialogs.rs`).

### Target Design & Interaction Contract
- **Single canonical, dataset-naming warning banner:** Consolidate the overlapping top `result.ambiguity_callout` and bottom `result.diagnostic_banner` on nonlinear-fit `ResultSheet`s so an ambiguous/unstable fit renders exactly one unified banner naming the affected datasets (e.g., `"Ambiguous / unbounded fits in 5 of 8 datasets (VEHICLE, CLOZAPINE, …)"` from `R19.7`).
- **Compact banner actions & preserved cell cues:** Keep the banner uncluttered with 1–2 targeted actions (**`Constraints`** via `ui.editParams.open`, and **`Compare vs. Horizontal Line`** from `R19.7`) alongside **`Why?`** and **`Dismiss`** (all dot-free per `R20.15`); keep cell-level amber ambiguous/unbounded highlights and tooltips (formatted by `R20.10`) and full checklist cards in **Interpret › Assumptions** (`results_interpret.rs`).
- **Edge cases:** Ambiguity-only cells without a `Violated`/`Warn` checklist item still surface a single banner with **`Constraints`** (`ui.editParams.open`); preserve non-identifiability checklist warnings on nonlinear fits—specifically `eed57fa` `EXT-01`'s `ChecklistStatus::Warn` (`"X-axis scale matches log10(X) equation"`, `crates/fit/src/result.rs:1195–1201`)—so banner consolidation never swallows unrelated warnings; and keep non-fit analyses (ANOVA, *t*-tests, survival, etc.) unchanged.
- **Localization:** Move any remaining hardcoded English ambiguity banner copy into `crates/ui-egui/locales/catalog.tsv` across all shipped locales.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{results_view,center_stage}.rs`, `crates/ui-egui/src/dialogs_canvas/workspace_dialogs.rs`, `crates/ui-egui/src/egui_shell/trailing_studio/results_interpret.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** Update existing `has_ambiguity_callout` / `result.ambiguity_callout` UI assertions and add an `egui` regression test verifying that an ambiguous/unstable nonlinear fit renders exactly one dataset-named warning banner.

---

## 6. `R20.7` Overhaul analysis sidebar Methods tab preview, manuscript prose & export wiring

- **Roadmap entry:** [`ROADMAP.md#r207`](../ROADMAP.md#r207-overhaul-analysis-sidebar-methods-tab-preview-manuscript-prose--export-wiring) · **Gate:** Post-**Beta** *(standalone)*

### Current Problem & Root Causes
The **Methods** tab in `crates/ui-egui/src/egui_shell/trailing_studio/results_interpret.rs` is supposed to present a copyable publication methods paragraph ([`gui-layout-spec.md`](gui-layout-spec.md) §6), but currently fails on both content and UI:
1. **Non-selectable, cramped preview:** `TextEdit::multiline(&mut preview).interactive(false)` (`results_interpret.rs:289–291`) disables mouse selection (preventing highlighting or copying an excerpt), inside a hardcoded `max_height(180.0)` nested vertical `ScrollArea` (inside the Trailing Studio's outer vertical `ScrollArea`), showing only ~8 wrapped lines while leaving lower sidebar height empty.
2. **Audit dump instead of manuscript Methods prose:** `generate_methods_report` (`crates/engine/src/methods_report.rs`) begins with `Study Context and Data Provenance` (project title, internal IDs `res-1`/`ds-1`, `"Recalculation status: Up to date"`, `"0 cells masked as excluded"`), filling the visible `180 pt` window before any methods sentence appears. The Methods section itself leads with internal rounding/formatting rules, omits CurveBook's software version, and then appends up to 30-row Markdown/LaTeX result tables (which wrap into unreadable noise at ~240 pt width) plus the full diagnostic checklist already shown in the **Assumptions** tab.
3. **Markdown over-escaping:** `append_md_text` (`methods_report.rs:2534`) escapes `<`, `>`, `*`, `_`, `[`, `]`, `~`, `$` even inside prose and math runs, producing strings like `ns for p \> 0.05, \* for p \<= 0.05, \*\* for p \<= 0.01`.
4. **Remaining state & export disconnects (partially fixed in `eed57fa` `AUD-09-46`):** `eed57fa` already fixed `last_copied_methods` auto-reset on sheet switch, format switch, and report text change (`results_interpret.rs:62–65,245–256`, tested in `rem_ui_views.rs:1422–1459`), guarded `copy_text` with `!app.is_human_path_only()` (`L274`), and cleared `last_methods_report` on sheet switch (`results_hooks.rs:147`). Still open: no **`Save`** button is exposed even though `result.generateReport` native file save already exists; `state.methods_format` can still drift from `app.canvas_dialogs.result_view.report_format` (used by menu/toolbar `Copy Methods Report`); `last_copied_methods` must also reset on `scope` change; and on error `resolve_methods_paragraph` (`results_interpret.rs:37–43`) still falls back to `last_methods_report`.

### Target Design & Interaction Contract
- **Scope toggle (`Methods` default vs. `Full Report`):**
  - Default **Methods** scope emits concise, manuscript-ready prose interpolating the exact test/model, equation, weighting, multiplicity correction, sample sizes/exclusions, and CurveBook version—without internal sheet IDs, string-formatting boilerplate, 30-row result tables, or duplicate checklist bullets.
  - **Full Report** scope exposes the complete four-section ARRIVE/ICMJE audit report for users who want provenance + tables + diagnostics.
- **Clean Markdown rendering:** Update `append_md_text` (`methods_report.rs:2534`) so benign prose and math symbols do not emit stray `\*`, `\<`, `\>`, or `\_` backslashes while keeping CommonMark/GFM tables valid and safely escaped where required.
- **Usable sidebar preview & actions:**
  - Replace `.interactive(false)` (`results_interpret.rs:289–291`) with selectable read-only text (`TextEdit::multiline(&mut text.as_str())` or formatted selectable prose) and remove the fixed `180 pt` nested scroll cap so the preview uses available sidebar height.
  - Place **`Save`** (`result.generateReport`, dot-free per `R20.15`) alongside **`Copy`** (`ui.result.copyMethodsReport`), keep `state.methods_format` synchronized with `result_view.report_format`, extend `eed57fa`'s `"Copied"` reset to include `scope` changes, and drop the `last_methods_report` error fallback (`results_interpret.rs:37–43`). *(Note: `323cf38` under `R10.29` already dropped hardcoded `"format": "markdown"` from `result.generateReport` default routing in `apps/curvebook/src/desktop_host.rs:4032`, `crates/ui-egui/src/control_io/host_routing.rs:634`, and `crates/ui-egui/src/egui_shell/command_palette.rs:998`, enabling clean `.txt`/`.md`/`.tex` extension inference when wiring `Save`.)*

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/trailing_studio/{results_interpret,mod}.rs`, `crates/engine/src/methods_report.rs`, `crates/engine/src/cmd/analysis.rs` (optional `scope` parameter on `result.generateReport`), `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** Engine unit tests for `Methods` vs. `Full Report` output and clean Markdown escaping; `egui` regression tests for selectable preview, format sync, `Save` intent, and `"Copied"` reset across sheet/format/parameter changes.

---

## 7. `R20.8` Smooth data-grid multi-cell drag selection, perimeter highlight & edge auto-scroll

- **Roadmap entry:** [`ROADMAP.md#r208`](../ROADMAP.md#r208-smooth-data-grid-multi-cell-drag-selection-perimeter-highlight--edge-auto-scroll) · **Gate:** Post-**Beta** · **Complements:** [`R20.11`](#10-r2011-focused-vs-unfocused-split-pane-selection-xy-selection-sync--header-coordinate-disambiguation)

### Current Problem & Root Causes
Highlighting multiple cells in the virtualized data grid feels choppy and visually jagged due to four concrete defects:
1. **In-frame lag (post-commit repaint landed in `eed57fa`):** `apps/curvebook/src/gui_window.rs:1556,1832–1834` now returns `bool` from `commit_pending_intents` and calls `ctx.request_repaint()` when `!will_discard`, fixing the mouse-up stall, though cells still paint from pre-commit `app.grid` state on the event frame.
2. **Jagged per-cell inner borders & muddy tint:** `paint_single_multi_data_cell` (`data_grid/multi_archetype.rs:780`) strokes all 4 inside edges of *every* selected cell with `1.0 pt` `tokens.accent`, after which `1.0 pt` grey `divider` hairlines paint over the right/bottom edges while top/left edges stay blue (plus a `2.0 pt` `focus_ring` on the active cell). Selected cells also fill directly with `tokens.active_bg.gamma_multiply(0.65)` without painting `base_row_bg` underneath.
3. **Broken drag state machine & per-pixel intent spam:**
   - `a.coord != coord` in `shared.rs:859,893` prevents shrinking a dragged range back down to the anchor cell.
   - Per-cell `ptr_pos.filter(|p| resp.rect.contains(*p))` hit-testing freezes selection dead whenever the pointer drifts into headers, row gutter, scrollbar, or past the viewport edge—with zero edge auto-scroll.
   - Moving 1 px inside the *same* cell re-emits `ExtendGridSelection` every frame, repeatedly running `extend_shift_range` → `sync_brush_from_grid_selection` (`all_selected_coords` allocation) → `record_ui_mutation_checkpoint`.
4. **Incomplete drag coverage:** X/Time columns (`render_single_multi_x_cell`) store only a single `selected_x_cell` with no drag-select; row numbers only support single-row clicks; column headers lack click/drag column range selection.

### Target Design & Interaction Contract
- **Immediate drag preview & deduplication:** Build on `eed57fa`'s `commit_pending_intents` repaint (`gui_window.rs:1556,1832–1834`, adding in-frame drag preview as needed); emit `ExtendGridSelection` only when the target cell coordinate actually changes (`shared.rs:859,893`, staying compatible with `ResolvedGridSelection` at `shortcuts.rs:343`).
- **Continuous clamped drag-select & edge auto-scroll:** When a grid drag anchor is active, compute `(row, flat_col)` once per frame from `(pointer_pos - origin)` clamped to `[0..row_count, 0..total_data_flat_cols]` instead of per-cell `resp.rect.contains`. Allow dragging back onto the anchor cell to collapse to a 1-cell selection, remove dead-zone stalls once a drag starts, and smoothly auto-scroll the viewport when dragging past the visible body bounds.
- **Unified outer selection perimeter:** Composite a consistent selection tint over `base_row_bg` across selected cells, keep subtle interior divider hairlines clean, draw **one** outer `1.5–2 pt` accent perimeter around contiguous `(r0..=r1, f0..=f1)` bounds, and keep the active-cell cue clean and unambiguous *(unfocused color styling handled by `R20.11`)*.
- **X/Time & header drag + bounded brushing:** Extend drag/range selection across X/Time columns, row-number gutter, and column headers, and bound/throttle `sync_brush_from_grid_selection` on large selections.

### Code Touchpoints & Verification
- **Files:** `apps/curvebook/src/gui_window.rs`, `crates/ui-egui/src/egui_shell/data_grid/{shared,multi_archetype,headers}.rs`, `crates/ui-egui/src/egui_shell/intents/executors.rs`, `crates/ui-egui/src/lib.rs`.
- **Tests:** `egui` regression tests proving immediate repaint after drag/release, drag-back to anchor collapsing to 1 cell, clamped edge drag & auto-scroll, deduplicated intents within one cell, and single outer selection perimeter rendering.

---

## 8. `R20.9` Unified top toolbar with OS-native menu placement

- **Roadmap entry:** [`ROADMAP.md#r209`](../ROADMAP.md#r209-unified-top-toolbar-with-os-native-menu-placement) · **Gate:** Post-**Beta** · **Recommended after:** [`R20.16`](#15-r2016-unify-dropdown-overflow--context-menus-with-content-measured-sizing-and-popup-parity-gates) · **Coordinates with:** [`R20.18`](#17-r2018-fix-brand-mark-sizing--mipmap-aliasing-and-make-logo-the-home-button) (logo-as-Home button)

### Current Problem & Measured Width Budgets
Top chrome (`crates/ui-egui/src/egui_shell/{top_chrome,top_bar_fit,layout}.rs`) always stacks two full-width rows (`32 pt` Tier 1 + `38 pt` Tier 2 = `70 pt`):
- **On macOS (`host_draws_menu_bar == true`):** Native OS menus live in the screen menu bar via `muda`, and the project title + dirty dot (`•`) already appear in the OS window titlebar (`apps/curvebook/src/gui_window.rs::sync_window_title`). Tier 1 therefore holds only `Brand` (`20 pt`) + `Home` (`~54 pt`) on the left and `Search` (`176 pt`) + `Split` (`58 pt`) on the right (`~315 pt` total), leaving `600–900 pt` of dead air above Tier 2 on `1280–1440 pt` windows. Combined with Tier 2 (`+ New`, `Undo`, `Redo` `~180 pt` + 3–5 contextual buttons `~300–520 pt`), total width is `~800–975 pt`—and drops another `~45–60 pt` once `R20.18` merges `Brand` + `Home` into one clickable logo button—well within one `36–38 pt` toolbar on standard laptops.
- **On Windows / Linux / FreeBSD / `--in-window-menus` (`host_draws_menu_bar == false`):** The 8 inline top-level menus (`File`…`Help`) consume `~380–420 pt` in English and `~520–620 pt` in German/French/Russian. Adding `Home` + `Search` + `Split` reaches `~700–940 pt` before placing any Tier 2 action, so cramming menus and actions into one row would force almost all toolbar buttons into `⋯` overflow on `1024–1280 pt` screens or at `150%–200%` scale.

### Rejected Alternative (Recorded for Future Reference)
- **Option B — Always-single bar on all platforms with a collapsed `☰` hamburger menu:** Explicitly **rejected**. Hiding `File · Edit · View · Insert · Data · Analyze · Graph · Help` behind a hamburger menu hurts menu discoverability and `Alt+F` desktop conventions on Windows/Linux and breaks direct `menu.*` `EguiHumanPathDriver` / `human_wf_*` paths.

### Target Design — Option A: Unified Toolbar + OS-Native Menu Placement
```text
macOS (host_draws_menu_bar == true) — single 36-38 pt bar (reclaims 32 pt):
+---------------------------------------------------------------------------------------------------+
| [Logo/Home] | [+ New] [↩] [↪] | [Primary CTA] [Context...] [...]              [Search] [Split]    |
+---------------------------------------------------------------------------------------------------+

Windows / Linux / --in-window-menus (host_draws_menu_bar == false) — thin menu bar + identical toolbar:
+---------------------------------------------------------------------------------------------------+
| File  Edit  View  Insert  Data  Analyze  Graph  Help                         Project Title •      | 26-28 pt
+---------------------------------------------------------------------------------------------------+
| [Logo/Home] | [+ New] [↩] [↪] | [Primary CTA] [Context...] [...]              [Search] [Split]    | 36-38 pt
+---------------------------------------------------------------------------------------------------+
```

- **Menu-bar-aware height (`layout.rs`, `top_chrome.rs`):** Drive Tier 1 from `ResponsiveShellLayout::host_draws_menu_bar`: `tier1_app_bar_h_pt = 0.0` (single unified toolbar, no extra divider) when `true`; thin `26–28 pt` classic menu strip above the identical unified toolbar when `false`.
- **Coordinated width solver (`top_bar_fit.rs`, `top_chrome.rs`):** Coordinate `fit_right_group` and `fit_ranked_toolbar` against one shared horizontal budget *(note: `top_bar_fit.rs:67–98` already handles `switcher == 0.0` cleanly since `eed57fa`)* with deterministic degradation:
  1. Spill `ActionRank::Overflow` actions into `⋯` (using `ui_kit::action_menu_popup` from `R20.16`).
  2. Collapse the `176 pt` `Search` capsule to a `28–34 pt` icon button *before* dropping `Secondary` contextual tools.
  3. Spill `ActionRank::Secondary` actions into `⋯`.
  4. Keep the clickable logo `Home` button (`R20.18`), the single `ActionRank::Primary` CTA, `⋯`, `Search` icon, and `Split` visible down to `640 × 420 pt` (`~270 pt` minimum footprint).
- **Icon-only `Undo` / `Redo`:** Convert `Undo` and `Redo` to crisp `28 pt` curved-arrow vector icon buttons with localized AccessKit labels and `⌘Z` / `⇧⌘Z` structured tooltips (`R20.21`), recovering `~60–90 pt` across long-word locales.
- **Preserved invariants:** All stable widget IDs (`toolbar.home`, `toolbar.new`, `toolbar.undo`, `toolbar.redo`, `toolbar.analyze`, `toolbar.split`, `toolbar.search_input`, `toolbar.overflow`, `menu.*`) and the single-`Primary`-CTA rule remain unchanged.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{layout,top_chrome,top_bar_fit}.rs`, `docs/planning/gui-layout-spec.md` (§2.1, §3).
- **Tests to update:** `TopChromeFrameReport` (`tier1_rect_pt` / `tier2_rect_pt`) assertions when `host_draws_menu_bar == true` in `crates/ui-egui/tests/r15_shell_suite/r15_3_top_chrome.rs`, `crates/ui-egui/tests/ui_audit_suite/dialogs_canvas.rs`, and `crates/ui-egui/tests/ui_audit_suite/cjk_rtl_hidpi_desktop_qa.rs`.

---

## 9. `R20.10` Results table CI-first column sizing, borderless underline tabs, quiet header actions & explicit CI formatting

- **Roadmap entry:** [`ROADMAP.md#r2010`](../ROADMAP.md#r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting) · **Gate:** Post-**Beta** · **Priority:** #1 in the 2026-11-25 visual review · **Coordinates with:** [`R19.1`](../ROADMAP.md#r191-descriptive-statistics-resultui-parity--nonlinear-identifiability-formatting), [`R19.7`](#4-r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics), [`R20.6`](#5-r206-consolidate-duplicate-nonlinear-fit-parameter-identifiability-warning-banners), [`R20.13`](#12-r2013-live-desktop-font-installation-chrome-border-cleanup-copy-polish--content-aware-initial-split)

### Current Problem & Root Causes
Four closely coupled defects in `crates/ui-egui/src/egui_shell/results_view.rs` obscure results and crowd the `32 pt` header:
1. **Uniform `128 pt` columns clip confidence intervals while leaving wide blank whitespace on the right (and hide truncation in metadata):** `render_result_tab_table` hardcodes `row_stub_w = 190.0` and a single scalar `col_w = 128.0` for all columns (and `eed57fa` added `190.0 + target_col * 128.0 - 128.0` scroll-to-match math at `results_view.rs:575`), then pads `total_w = (row_stub_w + num_cols * col_w).max(ui.available_width())`. On a 4-column parameter tab (`Best-fit value`, `Confidence Interval`, `Dependency`, `Hougaard |g1|`), the table leaves empty canvas even at `2400–3440 pt` while giving `Confidence Interval` only `116 pt` of inner text width—clipping `[-17.50718935, -6.546176294]` / `[0.1753, 0.9412]`—and `results_view.rs:1174` hardcodes `ellipsized: false` / `full_label: None` in `FrameInspectEntryInput` even when `paint_table_cell_text` truncates the rendered cell text (confirmed by the 2026-11-25 Desktop GUI Audit).
2. **Rigid tab width cap (`120 pt` / `96 pt`) clips tab titles despite abundant free space:** `max_tab_cap = if tab_count > 5 { 96.0 } else { 120.0 }` truncates `"Best-fit values & confidence intervals"` to `"Best-fit values & c…"` unconditionally.
3. **`Parameters` looks selected alongside boxed tab chips and boxed utility buttons:** `result.edit_params` uses `egui::Button::new(...).fill(tokens.active_bg)` (the exact selection fill used on the active tab in `paint_result_tab_chip`), while `Copy`, `⋯ +N`, every result tab, and sub-header `New Graph from Tab` all draw 4-sided boxed outlines—clashing directly with the quiet borderless `Swap` / `Close` ghost controls (`ui_kit::ghost_action_button` in `center_stage.rs`).
4. **Cryptic `???` formatting obscures open vs. unavailable bounds and mismatches clipboard output:** `format_result_cell_semantic` (`results_view.rs:234`) emits `"???"` / `"??? (ambiguous)"` / `"[lo, ???] (???)"` (conflicting with `results_hooks.rs`'s `"Unbounded"`), while `CiBound` (`crates/doc/src/result.rs:235`) lacks an `Unavailable` variant and conflates unclosed profile-likelihood bounds with non-finite/uncomputed intervals (`crates/fit/src/result.rs:44–49`).

### Target Design & Interaction Contract

#### A. Per-column content-aware layout (Confidence Intervals first)
- Replace scalar `col_w: f32` in `ResultTableLayout` with cached `row_stub_w: f32`, `col_widths: Vec<f32>`, and cumulative `col_x_offsets: Vec<f32>` (length `num_cols + 1`), exposing the resolved natural table width for `R20.13`'s content-aware initial split ratio.
- **Bounded measurement pass (virtualization-safe):** Measure header and formatted cell widths across up to `256` sampled rows (first `128`, last `64`, plus visible viewport rows) cached per `(sheet_id, tab_id, row_count, col_count, sig_figs, font_pt, avail_w_quantized)`.
- **Column priority tiers** (including `2 × 6 pt` padding + `4 pt` breathing room):

  | Priority | Column Class | `min_w` | `max_auto_w` | Notes |
  |---|---|---|---|---|
  | **1 (Highest)** | **Confidence Interval** (contains `ResultCell::ConfidenceInterval` or CI header) | `144 pt` | `320 pt` | First claim on available width so both bounds render unclipped (`[0.1753, 0.9412]`, `[0.1753, unbounded]`) |
  | **2** | **Primary value / Best-fit / P-value** | `92 pt` | `180 pt` | Preserves numeric/scientific precision + `*` badges |
  | **3** | **Row stub (`Row / Metric`)** | `120 pt` | `260 pt` | Shrinks below `190 pt` for short labels (`Top`, `IC50`), returning reclaimed width to CIs |
  | **4** | **Secondary scalar / diagnostic metrics** (`Dependency`, `Hougaard \|g1\|`, `DF`, `SE`) & text | `84 pt` | `160 pt` (numeric) / `240 pt` (text) | Reclaims unused horizontal padding |

- **Allocation pass for `avail_w` (scroll viewport width):**
  1. Compute `natural_w[i] = clamp(measured_max_w[i] + 16.0, min_w[i], max_auto_w[i])`.
  2. **When `sum(natural_w) <= avail_w`:** Grant every column at least `natural_w[i]` (**zero clipping**), then distribute leftover slack softly (`2×` weight for CI columns, `1×` for others) up to `min(natural_w[i] * 1.45, max_auto_w[i])`.
  3. **When `sum(natural_w) > avail_w`:** Start at `min_w[i]` and allocate `budget = (avail_w - sum(min_w)).max(0.0)` strictly by priority tier (CI → Primary/P-value → Row stub & secondary). If `sum(min_w) > avail_w`, horizontal scrolling activates (`ScrollArea::both()`) while keeping **CI columns at full `natural_w`** (up to `max_auto_w`).
- Replace `vis_x / col_w` horizontal virtualization and `results_view.rs:575`'s `190.0 + target_col * 128.0 - 128.0` scroll-to-match math with `partition_point` and offset lookups over `col_x_offsets`.
- **Accurate truncation metadata (`results_view.rs:1174`):** Record the actual `ellipsized` flag returned by `paint_table_cell_text` (and pass `full_label: ellipsized.then_some(fmt.text.as_str())`) into `FrameInspectEntryInput` instead of hardcoding `ellipsized: false`, so inspection checks and regression tests catch clipping accurately at narrow widths while wide widths (`1280` / `2400` / `3440 pt`) show finite CIs unclipped and preserve horizontal scrolling when columns exceed viewport width.

#### B. Borderless underline result tabs (content-first) vs. quiet ghost header actions
- **Left / Primary zone — Borderless Underline Result Tabs:**
  - Replace boxed `paint_result_tab_chip` tabs with borderless underline tabs matching `ui_kit::underline_tab_bar` (`design-language.md` §4): transparent at rest (`tokens.text_muted`), `tokens.hover_bg` pill on hover (`tokens.text_strong`), and `tokens.text_strong` + `2.0 pt` bottom `tokens.accent` indicator flush with the header bottom rule when active (no 4-sided outline or filled box).
  - Retire `max_tab_cap = 96.0 / 120.0`: if `sum(natural_w) + gaps <= avail_strip_w`, render every tab at full natural width with zero truncation; otherwise prioritize the **active tab** up to `min(active_natural_w, (avail_strip_w * 0.55).max(180.0))`, allow visible inactive tabs up to `200 pt`, and spill remaining tabs into a quiet ghost `result.tabs.overflow` (`⋯ +N`, popup via `R20.16`'s `action_menu_popup`).
- **Right / Utility zone — Quiet Ghost Actions (`Parameters`, `Copy`):**
  - Separate from tabs with a `1.0 pt` vertical `tokens.divider` and right-align `Parameters` and `Copy` (mirroring `Swap` / `Close` in the split header above).
  - Render `result.edit_params` (`Parameters`) and `result.copy_selection` (`Copy`) via `ui_kit::ghost_action_button` with **`selected = false`** at rest (removing `.fill(tokens.active_bg)`), moving the platform shortcut (`⇧⌘P` / `Ctrl+Shift+P` under `R20.22` via `format_shortcut_for_platform`) into the hover tooltip—freeing `~80–110 pt` for the tab strip and letting `Copy` stay visible down to `360 pt` header width.
- **Sub-header harmony:** Convert `New Graph from Tab` (`result.new_graph_from_tab`, dot-free per `R20.15`) to `ui_kit::ghost_action_button` *(developer badges `[Inferential Report]` / `(24×5)` removed by `R20.13`)*.
- **Preserved invariants:** Retain `result.edit_params` (`command_id: Some("ui.editParams.open")`), `result.copy_selection`, `result.tab.<tab_id>`, `result.tab.overflow.<tab_id>`, `result.tabs.overflow`, `result.new_graph_from_tab`, and `Ctrl/Cmd+PageUp/PageDown` switching unchanged.

#### C. Explicit open & unavailable CI formatting (sole owner; retiring literal `???`)
- Extend `curvebook_doc::CiBound` (`crates/doc/src/result.rs:235`) with `Unavailable` (back-compatible serde; update exhaustive `CiBound` matches in `import_paste.rs:1600`, `import_wizard.rs:2153`, `crates/engine/src/dag.rs:1439`, `crates/engine/src/methods_report.rs:2160`, `crates/chart/src/survival_cat/shared.rs:383`, `crates/fit/src/library/pharma.rs:2557`, and `apps/curvebook-cli/src/cmd_info.rs:238`) and emit `CiBound::Unavailable` in `crates/fit/src/result.rs` when bounds are non-finite / uncomputed (`DF = 0`, robust Cauchy, singular covariance), reserving `CiBound::Open` for unclosed profile-likelihood / survival bounds.
- Unify screen (`results_view.rs::format_result_cell_semantic`) and clipboard (`results_hooks.rs::format_result_cell`) formatting across all shipped locales:

  | Cell / Interval State | Displayed Text | Visual Style | Tooltip |
  |---|---|---|---|
  | Both bounds finite | `[0.1753, 0.9412]` | `tokens.text_primary` (tabular numerals) | Full value if ellipsized |
  | Half-open (fit / inference) | `[0.1753, unbounded]` or `[unbounded, 0.9412]` | Amber (`tokens.role_cat`) | Explains which profile-likelihood / covariance bound did not close |
  | Both open (fit / inference) | `Not bounded` *(locked canonical wording across `R19.1`, `R19.7`, `R20.10`)* | Amber (`tokens.role_cat`) | Explains neither bound closed; directs to `Constraints` / `Parameters` |
  | Half-open (survival median) | `[14.0, Not reached]` | `tokens.text_primary` | Explains survival curve did not cross 50% for the upper bound |
  | Both open (survival median) | `Not reached` | `tokens.text_muted` | Explains median survival was not reached |
  | One or both `Unavailable` | `[0.1753, Unavailable]` or `Unavailable` | `tokens.text_muted` | Explains why CI could not be computed |
  | Ambiguous / unstable scalar | `Unstable` by default (`R19.7`; raw iterate in tooltip/JSON) | Amber (`tokens.role_cat`) | Exposes raw solver iterate + identifiability diagnostic |
  | `ResultCell::Text` with `"ambiguous"` | Clean localized label (no prepended `"??? "`) | Amber (`tokens.role_cat`) | Full text + diagnostic explanation |

- Preserve `result_cell_is_ambiguous` detection so amber cell styling and `R20.6`'s unified warning banner trigger identically without literal `"???"` glyphs.

### Code Touchpoints & Verification
- **Files:** `crates/doc/src/result.rs`, `crates/fit/src/result.rs`, `crates/ui-egui/src/egui_shell/{results_view,ui_kit}.rs`, `crates/ui-egui/src/dialogs_canvas/results_hooks.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** Layout tests confirming CI columns claim first width and render unclipped at `720 pt` and `540 pt`; header tests confirming unclipped underline tabs (`2 pt` bottom accent without box stroke) and non-selected ghost `result.edit_params` across `Light`/`Dark`/`HighContrast`; formatter tests verifying zero `"???"` cells across display and TSV copy + `CiBound::Unavailable` serde round-trip; existing workflow suites (`r15_3_stage_canvas.rs`, `human_wf_13_18.rs`, `human_wf_51_58.rs`, `audit_p7_sec17.rs`, `r10_core_and_spike.rs`) pass unchanged.

---

## 10. `R20.11` Focused vs. unfocused split-pane selection, X/Y selection sync & header coordinate disambiguation

- **Roadmap entry:** [`ROADMAP.md#r2011`](../ROADMAP.md#r2011-focused-vs-unfocused-split-pane-selection-xy-selection-sync--header-coordinate-disambiguation) · **Gate:** Post-**Beta** · **Complements:** [`R20.8`](#7-r208-smooth-data-grid-multi-cell-drag-selection-perimeter-highlight--edge-auto-scroll)

### Current Problem & Root Causes
1. **Pane focus is dropped before rendering child sheets:** `render_split_stage` (`crates/ui-egui/src/egui_shell/center_stage.rs`) computes `primary_focused` / `secondary_focused` for the `2 pt` blue header accent rule, but `SinglePaneTarget`, `ColumnGridRenderContext` (`data_grid/mod.rs`), and `render_results_view_for_sheet` (`results_view.rs`) take no `focused: bool` flag.
2. **Existing `tokens.selection_unemphasized_bg` is unused:** `theme.rs` and `tokens.rs` already define and contrast-verify `selection_unemphasized_bg` (`design-language.md` §1), yet `results_view.rs` always paints `tokens.active_bg` + `1.5 pt` `tokens.accent`, and `data_grid/multi_archetype.rs` always paints `tokens.active_bg` + `2.0 pt` `tokens.focus_ring` even when another pane owns keyboard focus.
3. **Global `app.brushed_cell` stamps `R10C4` on both split pane headers:** `render_split_pane_header` (`center_stage.rs`) prints `format!("R{}C{}", brushed.row + 1, brushed.col + 1)` unconditionally for both `Primary` and `Secondary` whenever `app.brushed_cell` is `Some`, even when the secondary pane is a `ResultSheet` with no brushing relation to `R10C4`.
4. **`X`-cell vs. `Y`-cell selection desync (`data_grid/`):** `aux.selected_x_cell` (`multi_archetype.rs`, `shortcuts.rs`, `inline_edit.rs`) tracks `X`/Time cell selection separately from `app.grid.tool.active` (`Y` cells) and `app.grid.active_x_cell`, allowing stale `Y` or `X` highlights/coordinates to persist out of sync across clicks, arrow navigation, and split panes.

### Target Design & Interaction Contract
- **Propagate `pane_focused: bool` end-to-end:**
  - Add `focused: bool` to `SinglePaneTarget`, `ColumnGridRenderContext` (defaulting to `true`, overridden via `.with_focused(focused)`), and `render_results_view_for_sheet`.
  - Compute `pane_focused = stage_enabled && ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid` in single-pane mode, and `&& views.center_stage.focused_split_pane == slot` in split-pane mode.
  - Keep `doc.active_sheet_id` synchronized when `F6` / `Shift+F6` cycles `focused_split_pane` so the active header bar, emphasized cell selection, and Inspector target never disagree.
- **Emphasized (focused) vs. unemphasized (retained) selection styling:**

  | Surface & State | Focused Pane (`pane_focused == true`) | Unfocused Pane (`pane_focused == false`) |
  |---|---|---|
  | **Data Grid — Active cell** | `tokens.active_bg` fill + `2.0 pt` `tokens.focus_ring` inner border | `tokens.selection_unemphasized_bg` fill + `1.0 pt` `tokens.border` inner border (no blue focus ring) |
  | **Data Grid — Selected range / X cell** | Translucent `tokens.active_bg` + `1.5 pt` `tokens.accent` outer perimeter (`R20.8`) | `tokens.selection_unemphasized_bg` + `1.0 pt` `tokens.divider` outer perimeter |
  | **Data Grid — Active row `#` & col header** | Subtle `tokens.active_bg` tint + `tokens.text_strong` | Neutral `tokens.header_bg` |
  | **Results Table — Focal selected cell** | `tokens.active_bg` fill + `2.0 pt` `tokens.focus_ring` inner border | `tokens.selection_unemphasized_bg` fill + `1.0 pt` `tokens.border` inner border |
  | **Results Table — Multi-cell selection** | `tokens.active_bg` fill + `1.5 pt` `tokens.accent` perimeter | `tokens.selection_unemphasized_bg` fill + `1.0 pt` `tokens.divider` perimeter |
  | **Split Pane Header** | `2.0 pt` top `tokens.accent` bar + `tokens.text_strong` selector label | No top accent bar + `tokens.text_primary` / `tokens.text_muted` selector label |

  - In `HighContrast`, unfocused retained selections use a `1.0 pt` dashed `tokens.border` outline (`>= 3:1` against `surface_bg`) so retained selections stay perceivable without competing with the solid `2.0 pt` `tokens.focus_ring`.
- **Pane-aware header coordinate & brush pill (`center_stage.rs`):**
  - **DataSheet pane:** Show the owning DataSheet's active `R{row+1}C{col+1}` coordinate (emphasized when `focused == true`, muted or omitted when unfocused).
  - **ResultSheet pane:** Never display `app.brushed_cell`; omit cell coordinates from the ResultSheet header (or display only the ResultSheet's own `selected_cell` while focused).
  - **Linked GraphSheet pane:** Only when `doc.upstream_dependencies(graph_id)` includes `brushed.sheet_id`, display an explicitly labeled cross-brush pill (`Brushed: R10C4` + quiet `×` button retaining `split.clear_brush`).
- **Mutual-exclusion sync between `X` and `Y` cell selection (`data_grid/{multi_archetype,shortcuts,inline_edit}.rs`):** Selecting an `X`/Time cell clears/supersedes `Y` active focus (and syncs `app.grid.active_x_cell`), and selecting a `Y` cell clears `selected_x_cell` immediately on the same frame so only one active cell highlight and header coordinate exists per grid pane.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/center_stage.rs`, `crates/ui-egui/src/egui_shell/data_grid/{mod,multi_archetype,shortcuts,inline_edit}.rs`, `crates/ui-egui/src/egui_shell/results_view.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** Split-view Data + Results tests verifying focused vs. unemphasized selection colors on click and `F6`, single-pane `R{r}C{c}` header ownership (never shown on Results header), `X` ↔ `Y` mutual exclusion without stale highlights, and Data + Graph explicit `Brushed: R{r}C{c}` pill + `split.clear_brush` clearing.

---

## 11. `R20.12` Navigator & Family disambiguation subtitles and labeled kind filter dropdown

- **Roadmap entry:** [`ROADMAP.md#r2012`](../ROADMAP.md#r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown) · **Gate:** Post-**Beta** · **Complements:** [`R10.36`](#2-r1036-project-navigator-collapsed-rail--section-vector-icons)

### Current Problem & Root Causes
1. **Single-line `26 pt` Navigator rows omit source/analysis metadata:** `build_simple_entry` (`crates/ui-egui/src/workspace_nav.rs`) leaves `archetype_label: None` and computes no subtitle for `GraphSheet`, `InfoSheet`, or `LayoutSheet` entries, even though `ProjectDocument` already links `GraphSpec::primary_source_sheet_id`, `GraphSpec::kind`, `comparisons`, `AnalysisSpec::source_sheet_id`, and `doc.upstream_dependencies`. Multiple graphs sharing a visible title render identically in the Navigator (`leading_dock.rs`), Family pane (`FamilyPaneNode`), and split-pane sheet selector (`center_stage.rs`).
2. **Single-letter `All | D | I | R | G | L` filter bar sacrifices recognition:** `leading_dock.rs` renders single English initials via `ui_kit::segmented_control`, forcing users to decode letters and leaking English initials across locales, whereas [`gui-layout-spec.md`](gui-layout-spec.md) §4.1 already specifies a labeled kind-filter dropdown.

### Target Design & Interaction Contract
- **Disambiguation subtitles (`workspace_nav.rs`, `leading_dock.rs`, `center_stage.rs`):**
  - Add `pub subtitle: Option<String>` and `pub has_duplicate_title: bool` (2+ entries sharing a trimmed case-insensitive title within the same `SheetSection`) to `NavigatorEntry`, and mirror disambiguation on `FamilyPaneNode`.
  - **Derivation rules (concise, non-redundant):**
    - **Graphs:** `"<Primary Source Title> · <GraphKind / Analysis>"` (e.g., `Dose-Response · XY + 4PL Fit`, `Table 1 · Residual Plot`, `Cohort A · Survival`). When the primary source title matches the graph title, omit the redundant prefix and lead with the distinguishing analysis/plot detail (`4PL Fit`, `Residual Plot`, `XY · 2 series`). If two graphs still collide on `(title, subtitle)`, append a deterministic disambiguator (series count, linked analysis title, or index suffix).
    - **Results:** Upstream source title (`From Dose-Response`), plus analysis kind if a `custom_title` hides it.
    - **Data Tables:** Archetype + shape (`XY · 12×3`, `Column · 3 groups`) or `Linked ← <Source>`.
    - **Info / Layouts:** Paired sheet or content count (`4 constants`, `3 panels`).
  - **Display policy:** Render two-line `38 pt` rows on **`Graphs`, `Results`, and any entry with `has_duplicate_title == true`** (keeping unambiguous `Data`, `Info`, and `Layouts` rows compact at `26 pt`; if stricter density is ever preferred, this policy can switch to duplicate-only without changing the data model). Line 1 renders the sheet title (`12.0 / 11.5 pt`), Line 2 renders `subtitle` (`10.0 pt`, `tokens.text_muted`, elided, indented to title `x_cur`), trailing badges stay aligned to Line 1, and hover tooltips show full `"{title}\n{subtitle}"`.
  - **Search, Family pane & split selector:** Match `nav.search_input` against both `title` and `subtitle`, show the disambiguation subtitle on `FamilyPaneNode` when related graphs share a visible title, and append ` — {subtitle}` to colliding sheet titles in `render_split_pane_header`'s sheet selector dropdown (`center_stage.rs`).
- **Labeled `widgets::dropdown` kind filter (`leading_dock.rs`):**
  - Replace the expanded Navigator's `All | D | I | R | G | L` segmented bar with `widgets::dropdown` (never `egui::ComboBox`, banned in `clippy.toml`).
  - **Trigger (`widget_id: "nav.kind_filter"`, `Role::ComboBox`):** Displays the localized active filter label with live count (e.g., `All sheets (14)`, `Graphs (5)`, `Results (3)`), localizing `nav.filter_all` across all shipped locales.
  - **Menu items:** All 6 options (`All sheets`, `Data Tables`, `Info`, `Results`, `Graphs`, `Layouts`), each rendered with its shared `draw_sheet_vector_icon` glyph (`R10.36`, including `Σ` for Results), full localized label, right-aligned count `(N)`, and stable `nav.chip.{all,data,info,results,graphs,layouts}` widget IDs (`ZLayer::DropdownMenu`).
- **HIG source-list multi-select (`leading_dock.rs`, `workspace_nav.rs`):**
  - Support `Shift+Click` range selection and `⌘/Ctrl+Click` discontinuous selection across Project Navigator rows (reusing `leading_dock.rs:1812` `pub fn collect_visible_sheet_ids`, made public and nesting-aware in `eed57fa`, and visually distinguishing multi-selected rows from the single active sheet, matching Apple HIG sidebar conventions).
  - When `> 1` sheets are selected in the sidebar, route right-click context actions (`Delete`, `Duplicate`, `Freeze/Unfreeze`, `Export`, `Compare in Split View` when 2 sheets selected) through the shared batch-action plumbing used by Portfolio Gallery (`R10.35`).

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/workspace_nav.rs`, `crates/ui-egui/src/egui_shell/{leading_dock,center_stage}.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests to update:** Migrate the 9 integration test files clicking `nav.chip.*` directly (`human_wf_common/mod.rs`, `r15_3_studio_footer.rs`, `r15_3_welcome_nav.rs`, `r15_3_stage_canvas.rs`, `r15_4_r10_23_egui_gaps.rs`, `r15_7_gui_addendum.rs`, `interaction_hidpi_qol.rs`, `r10_23_beta_desktop_gaps.rs`, `r10_23_shortcuts_and_copy.rs`) to a shared helper opening `nav.kind_filter` then clicking `nav.chip.<kind>`. Add tests for duplicate-title graph subtitles across Navigator/Family/split selector, subtitle search matching, and `nav.kind_filter` dropdown behavior at `1.0x` and `1.5x` DPI.

---

## 12. `R20.13` Live desktop font installation, chrome border cleanup, copy polish & content-aware initial split

- **Roadmap entry:** [`ROADMAP.md#r2013`](../ROADMAP.md#r2013-live-desktop-font-installation-chrome-border-cleanup-copy-polish--content-aware-initial-split) · **Gate:** Post-**Beta** · **Depends on:** [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting) (for content-aware initial split width)

### Current Problem & Root Causes
1. **Live desktop launch bypasses `install_font_definitions`:** `apps/curvebook/src/gui_window.rs:3017–3018` (and desktop test harnesses at `gui_window.rs:3099` `GuiHarness::new` and `apps/curvebook/src/main.rs:2027`) call `cc.egui_ctx.set_fonts(build_egui_font_definitions(font_bytes.as_deref()))` directly instead of `curvebook_ui_egui::egui_shell::fonts::install_font_definitions`. Consequently, `BOLD_AVAILABLE_DATA_KEY` and `NUMERAL_FAMILY_LOADED_DATA_KEY` are never registered on live desktop windows (only inside `EguiHumanPathDriver` at `driver.rs:1172`), silently breaking `R20.2` bold section headings and `13 pt` aligned grid/result numerals in real user sessions.
2. **Grainy, undersized macOS shortcut symbols (`⌘ ⇧ ⌥ ⌃`) due to `Apple Symbols.ttf` fallback + `10.5 pt` pills:**
   - `query_primary_proportional` (`crates/ui-egui/src/egui_shell/fonts.rs:577–583`) queries `"SF Pro Text"`, `"SF NS Text"`, `"Helvetica Neue"`, `".SF NS"`. On modern macOS, `SFNS.ttf`'s TrueType family name is `"System Font"`, so `Helvetica Neue` wins as primary proportional font—which lacks glyphs for `⌘` (`U+2318`), `⇧` (`U+21E7`), `⌥` (`U+2325`), and `⌃` (`U+2303`).
   - Fallback then hits `OS_SYMBOL_FAMILIES[0] = "Apple Symbols"` (`fonts.rs:633–634`), whose glyphs are severely undersized hairlines relative to `Helvetica Neue`'s `0.71 em` cap-height (measured from `/System/Library/Fonts/`):

     | Font | `K` | `⌘` | `⇧` | `⌥` | `⌃` |
     |---|---|---|---|---|---|
     | `Apple Symbols.ttf` *(current fallback)* | `0.54 em` | **`0.58 em`** | **`0.55 em`** | **`0.28 em`** | `0.39 em` |
     | `SFNS.ttf` (`"System Font"`) | `0.70 em` | **`0.72 em`** | **`0.72 em`** | **`0.70 em`** | `0.38 em` |
     | `LucidaGrande.ttc` (`"Lucida Grande"`) | `0.72 em` | **`0.76 em`** | **`0.78 em`** | **`0.72 em`** | `0.34 em` |

   - Compounded by `10.5 pt` shortcut pills (`top_chrome.rs:1018,1080`, `command_palette.rs:513`), `Apple Symbols`'s `⇧` is only `5.8 pt` tall and `⌥` only `2.9 pt` tall—dropping hairline strokes below `1 px` so `egui`'s unhinted `ab_glyph` rasterizer turns `⌘`/`⇧` into grainy gray specks.
3. **Remaining resting borders across top/bottom chrome:** Several utility controls in `top_chrome.rs` (lines `550, 740, 791, 853, 1080`) and `status_footer.rs` still construct raw `egui::Button::new` or draw resting `rect_stroke` boxes rather than using `ui_kit::ghost_action_button` / `segmented_control`. *(Ownership boundary: `top_chrome.rs:1259` is owned by `R20.14`; `results_view.rs` header/sub-header buttons are owned by `R20.10`.)*
4. **Developer jargon & non-native shortcut copy:**
   - `render_tab_category_bar` (`results_view.rs:693–755`) prints internal classification tags `[Inferential Report]` and matrix dimensions `(24×5)` in the primary results strip.
   - `import_paste.rs:780`, `grid_view.rs:820`, and `dialogs_canvas/workspace_dialogs.rs:145` format `TableKind::Xy` as `"Xy"` instead of `"XY"` (producing `"Xy · 20x8"`), and `status_footer.rs:74–79` prints cryptic `"Rng="` / `"Med="` shorthand (`"Rng=0"`).
   - `results_view.rs:354`, `top_chrome.rs:1677`, and `dialogs_r9.rs:113` render literal `"Cmd+T"` instead of native platform symbols (`⌘T` on macOS, `Ctrl+T` on Windows/Linux via `format_shortcut_for_platform`).
   - Menus, Inspector, and transform/conversion modals (`menus/ui_commands.rs:314`, `menus/tooltips.rs:184`, `trailing_studio/table_selection.rs:207`, `transform_modals.rs:1437–1465`, `import_wizard.rs:1603–1605`) expose internal jargon `"Archetype"` / `"Convert Table Archetype..."` instead of `"Table Format"`.
5. **Fixed `50/50` split ratio on `Data || Results`:** Opening `Data || Results` defaults to `50/50` even when the Results table's auto-sized natural width is narrower than half the stage while a wide multi-group data table is clipped on the left.

### Target Design & Interaction Contract
- **Wire `install_font_definitions` on every live desktop context:** Route `apps/curvebook/src/gui_window.rs:3017–3018` (and `gui_window.rs:3099` / `main.rs:2027` test contexts) through `curvebook_ui_egui::egui_shell::fonts::install_font_definitions` so live windows and headless driver frames use identical font families and metadata.
- **Crisp macOS modifier symbols (`⌘ ⇧ ⌥ ⌃`) & readable shortcut pills (`fonts.rs`, `top_chrome.rs:1018,1080`, `command_palette.rs:513`):**
  - Prepend `"System Font"` (`SFNS.ttf`, already in `curated_os_font_candidates()` at `fonts.rs:826`) and/or `"Lucida Grande"` (`LucidaGrande.ttc`) ahead of `"Apple Symbols"` in `OS_SYMBOL_FAMILIES` (`fonts.rs:633–634`), bringing `⌘`, `⇧`, and `⌥` to full `0.72–0.78 em` cap-height with proper UI stroke weight while keeping `Helvetica Neue` as the primary static Regular/Bold proportional face (avoiding variable-font `fvar` weight issues in `SFNS.ttf`).
  - Apply `egui::FontTweak { scale: 1.25, .. }` when registering `Apple Symbols` so any remaining technical symbols falling through to `Apple Symbols` match `Helvetica Neue`'s `0.71 em` cap-height (`0.71 / 0.55 ≈ 1.29`).
  - Bump shortcut pill font size from `10.5 pt` to **`11.5–12.0 pt`** in `top_chrome.rs:1018,1080` and `command_palette.rs:513` (and in [`R20.21`](#20-r2021-structured-hover-tooltips-for-commands-scientific-readouts-navigator--inspector)'s tooltip shortcut pills) so the four loops of `⌘` have sufficient pixels to rasterize cleanly.
- **Complete top/bottom chrome control-kit migration:** Migrate remaining resting-bordered utility buttons in `top_chrome.rs` (excluding line `1259`) and `status_footer.rs` onto `ui_kit::ghost_action_button` / `segmented_control` (`design-language.md` §4).
- **User-facing copy & platform-shortcut cleanup:**
  - Remove `[Inferential Report]` and `(24×5)` from `render_tab_category_bar` (`results_view.rs:693–755`); collapse the sub-header entirely on inferential reports when it has no action, keeping only the clean bar with `New Graph from Tab` (`R20.10`) on `PlotReadyMatrix` tabs.
  - Fix `"Xy"` → `"XY"` in `import_paste.rs:780`, `grid_view.rs:820`, and `dialogs_canvas/workspace_dialogs.rs:145` (so shape summaries read `"XY · 20×8"`), and replace `"Rng="` / `"Med="` in `status_footer.rs:74–79` with plain-language `"Range="` / `"Median="` (updating `r15_4_r10_23_egui_gaps.rs:287, 376`).
  - Format keyboard shortcuts through `format_shortcut_for_platform` (`⌘T` on macOS, `Ctrl+T` elsewhere).
  - Replace user-facing `"Archetype"` / `"Convert Table Archetype..."` strings with `"Table Format"` (dot-free per `R20.15`) across all shipped locales in `catalog.tsv`, preserving `ui.table.convertArchetype.open` and `table.convertArchetype` command IDs unchanged.
- **Content-aware initial split ratio (`center_stage.rs`):**
  - When opening a `Data || Results` split view (or switching sheet pair to one), compute the **initial** split ratio from the Results tab's auto-sized natural width (`R20.10`) + scrollbar + padding (clamped within `[0.35, 0.72]`), capping the Results pane to its content width when narrower than `50%` so wide multi-group data tables receive the surplus width.
  - Strictly **initial ratio only**: once the user drags the split divider (`split_ratio_override` / `SetSplitRatio`), never auto-override their manual ratio on subsequent frames.

### Code Touchpoints & Verification
- **Files:** `apps/curvebook/src/{gui_window,main}.rs`, `crates/ui-egui/src/egui_shell/{fonts,top_chrome,command_palette,status_footer,results_view,center_stage,transform_modals,import_wizard}.rs`, `crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs`, `crates/ui-egui/src/menus/{ui_commands,tooltips}.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** Desktop launch test asserting `BOLD_FONT_FAMILY` and `ALIGNED_NUMERAL_FONT_FAMILY` metadata are set on live `CurveBookNativeApp` contexts; macOS font-fallback unit test verifying `"System Font"` / `"Lucida Grande"` precedes `"Apple Symbols"` and `Apple Symbols` carries `scale >= 1.2` `FontTweak` (plus shortcut pill size `>= 11.5 pt`); split-ratio tests asserting content-fit initial ratio and preservation of manual user drags; copy/shortcut & localization checks (`cargo xtask i18n-report`, `cargo xtask ui-commands --check`, `cargo xtask design-tokens`); and one batched `cargo xtask screenshots` refresh at the end of the push.

---

## 13. `R20.14` Reserve blue foreground text strictly for clickable links

- **Roadmap entry:** [`ROADMAP.md#r2014`](../ROADMAP.md#r2014-reserve-blue-foreground-text-strictly-for-clickable-links) · **Gate:** Post-**Beta** · **Complements:** [`R20.13`](#12-r2013-live-desktop-font-installation-chrome-border-cleanup-copy-polish--content-aware-initial-split)

### Policy Definition
> **Rule:** Blue foreground text (`tokens.accent` / `tokens.link_blue`) is reserved **exclusively** for interactive clickable links (rendered through one helper with `Sense::click()`, `CursorIcon::PointingHand`, and hover underline). Non-link text—headings, static table headers, status messages, frozen markers, excluded cells, badges, and significant numeric values—must use neutral (`text_strong`, `text_primary`, `text_muted`) or non-blue semantic tokens (`link_green`, `warning`, `error`).

**Explicitly unaffected (not blue text):**
- Solid `Primary` CTA fills (`on_accent` white text on `accent_strong` fill).
- Non-text structural accents (`2 pt` `focus_ring`, `2 pt` active underline-tab bar, `2 pt` focused split-pane header bar, `3 pt` Navigator active bar, selection fills `active_bg`, and `LinkedDataRegion` perimeter borders).

### Exhaustive Audit of Blue-Text Sites (`crates/ui-egui/src/`)

#### A. Definitely Not Links -> Change Off Blue (`22` sites after `R20.13` removes `results_view.rs:711` and `R20.17` deletes `app_modals.rs:2496`, including `5` sites added in `eed57fa`)

| Category | Exact Site(s) | Current Color | Recommended Replacement |
|---|---|---|---|
| **Excluded cells** *(global token bug)* | `theme.rs:180,246,308`, used in `data_grid/multi_archetype.rs:588,798,829`, `import_wizard.rs:2378`, `data_grid/shared.rs:835` | `excluded_cell_text` = blue `accent` / blue OKLCH | Retarget `excluded_cell_text` in `theme.rs` to neutral muted slate (`>= 4.5:1` AA / `>= 7.0:1` `HighContrast`), keeping italic + strikethrough + corner triangle |
| **Static grid `X` header & survival events** | `data_grid/headers.rs:575` (`X` title); `data_grid/multi_archetype.rs:804` (survival event value) | `tokens.accent` | `tokens.text_strong` (bold for event cells if emphasis is needed) |
| **Results significant P-values** | `results_view.rs:1215` (significant P-value) | `tokens.accent` | `tokens.text_strong` (bold; stars already mark significance) |
| **Frozen indicators** | `trailing_studio/table_selection.rs:60` (`Frozen (read-only)`), `leading_dock.rs:1742` (`Frozen`), `leading_dock.rs:1938` (`"F"` row badge) | `tokens.accent`, `tokens.link_blue` | `tokens.text_muted` (paired with a neutral lock glyph) |
| **Section headings & formula prefix** | `power_studio.rs:1249` (`Methods & Sample Size Narrative`), `stats_modals/stat_params.rs:487` (`Method Reference & Citations`), `center_stage.rs:1203` (`fx (…) =`) | `tokens.accent` | `tokens.text_strong` (using bold `section_heading_font_id`) |
| **Status readouts, warnings & previews** | `fit_modal.rs:266` (`fit.warning.log_x_scale`, added in `eed57fa`), `fit_modal.rs:1011` (`Compiled OK`), `fit_modal.rs:2456` (`Selected: …`), `power_studio.rs:1027` (`Derived Standardized …`), `stats_modals/analyze_picker.rs:249` (`duplicate_hint`), `stats_modals/helpers.rs:251` (symbolic formula), `import_wizard.rs:1106,1261` (`prepared.unstack_warning`, added in `eed57fa`), `stats_modals/nested_misc.rs:1246` (LOWESS fraction hint, added in `eed57fa`) *(plus `app_modals.rs:2496` if not yet removed by [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect))* | `tokens.accent` | `tokens.link_green` for `Compiled OK`; `tokens.warning` for `fit.warning.log_x_scale` & `unstack_warning`; `tokens.text_strong` for formula & derived values; `tokens.text_muted` for hints/status |
| **Selected pill / list button text** | `ui_kit.rs:767` (`choice_pill`), `manage_vars_modal.rs:438,935`, `graph_modals/format_graph/mv.rs:373` (selected level pill, added in `eed57fa`) | `tokens.accent` | `tokens.text_strong` (selected background already marks selection) |
| **Palette badge & match highlight** | `command_palette.rs:483` (`Sheet` badge), `command_palette.rs:534` (substring match) | `tokens.accent` | Neutral `surface_recessed` + `text_strong` for badge; bold `text_strong` (or subtle background highlight) for matched characters |
| **Non-clickable info/role badges** | `trailing_studio/results_interpret.rs:21` (`INFO` badge); `data_grid/mv_roles.rs:63` & `stats_modals/helpers.rs:380` (`NUM` badge using `role_num = accent`) | `tokens.link_blue`, `tokens.role_num` | `tokens.text_muted` for `INFO`; retarget `role_num` to neutral `text_strong` on `surface_recessed` (strict zero-exception rule) |

#### B. Resolved Borderline Sites (`4` sites)

| Site | Current Behavior | Locked Action |
|---|---|---|
| `leading_dock.rs:1533` (`3 sources` in `Family`) | Non-clickable `painter.text(..., tokens.link_blue)` | **Turn neutral (`text_muted`)** — the whole row clicks to open the sheet, and `R20.21`'s hover tooltip lists all upstream sources. |
| `fit_modal.rs:1437` (`Linked hook: …`) | Non-clickable `ui.label(..., tokens.accent)` | **Make clickable via `ui_kit::link_button`** — opens the Hook Constant browser for that parameter. |
| `data_grid/headers.rs:255` (corner `XY` button) | Clickable button opening Table Format converter | **Turn neutral (`text_strong`)** — reads as a static grid corner header; hover fill already signals interactivity. |
| `unit_spinners.rs:361` (`mm` / `in` / `pt` unit button) | Clickable bordered button cycling units | **Turn neutral (`text_primary` / `text_strong`)** — already sits inside a button frame; blue text makes unit suffixes look like hyperlinks. |

#### C. True Interactive Links -> Keep Blue via One Shared Link Helper (`4` sites including `fit_modal.rs:1437`)

| Site | Behavior | Action |
|---|---|---|
| `top_chrome.rs:1259` (`Open full search (⌘K)`) | Borderless link footer button opening Command Palette | Keep blue; route through `ui_kit::link_button` (hover underline + `PointingHand`) |
| `info_view.rs:562` (`🔗 N` constant hook badge) | Clickable reference-count jump control | Keep blue; route through `ui_kit::link_button` |
| `fit_modal.rs:1437` (`Linked hook: …`) | Clickable jump to Hook Constant browser | Keep blue; route through `ui_kit::link_button` |
| `canvas_view.rs:1325` (`Switch to CLD`) | Borderless inline action inside bracket-crowding hint | Keep blue; route through `ui_kit::link_button` |

*(Also audit `data_grid/linked_regions.rs` header pills: `Go to Source` uses `ui_kit::link_button`; `Raw Plate › A1:H12` source title stays neutral unless clicking it jumps to the source.)*

### Target Design, Token & Enforcement Contract
- **Theme module (`crates/ui-egui/src/theme.rs`):**
  - Update `UiThemeSpec::accent` doc comment (`theme.rs:82`) to stop advertising `accent` for generic labels.
  - Change `excluded_cell_text` (`theme.rs:180,246,308`) from blue to neutral muted slate (`>= 4.5:1` AA / `>= 7.0:1` `HighContrast`).
  - Keep `link_blue` strictly for `LinkedDataRegion` borders/dots, never text; add a dedicated `text_link` alias (or route all links exclusively through `ui_kit::link_button`) so link text is structurally distinguishable in source scans.
- **Canonical link helper (`crates/ui-egui/src/egui_shell/ui_kit.rs`):**
  - Add `ui_kit::link_button(...)`: renders borderless `text_link`/`tokens.accent` text with `CursorIcon::PointingHand`, hover underline, `>= 24 pt` hit height, and `FrameInspectMeta` recording (`role: "link"` or `"button"`).
- **Spec & CI guardrail:**
  - Document the strict zero-exception rule in [`docs/design-language.md`](../design-language.md) §2 (*Strict single-accent budget*) and [`docs/planning/gui-layout-spec.md`](gui-layout-spec.md) §10.2/§10.7.
  - Add a lightweight policy check in `cargo xtask design-tokens` (reusing `xtask/src/scan_util.rs` `walk_file_paths`/`rel_slash`/`is_regular_file` helpers from `R21.3` and `design_tokens.rs`'s `tokenize_blanked_code`/`CodeTok` token scanner from `eed57fa`; current ceilings at `eed57fa`: `MAX_RAW_COLORS = 55`, `MAX_RAW_SIZE_PAIRS = 232`) flagging `.color(tokens.accent)` / `.color(tokens.link_blue)` / `.color(tokens.excluded_cell_text)` on non-link text primitives (`ui.label`, `RichText`, `painter.text`, `CellTextStyle`) outside `ui_kit::link_button`.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/theme.rs`, `crates/ui-egui/src/egui_shell/ui_kit.rs`, the audited `crates/ui-egui/src/egui_shell/` files listed above, `xtask/src/{design_tokens,scan_util}.rs`, `docs/design-language.md`, `docs/planning/gui-layout-spec.md`.
- **Tests:** `cargo test -p curvebook-ui-egui` (theme contrast + specimen tests) and `cargo xtask design-tokens` (including negative regression check for unauthorized blue non-link text).

---

## 14. `R20.15` Eliminate trailing ellipses on action buttons and unify ellipsis semantics

- **Roadmap entry:** [`ROADMAP.md#r2015`](../ROADMAP.md#r2015-eliminate-trailing-ellipses-on-action-buttons-and-unify-ellipsis-semantics) · **Gate:** Post-**Beta** · **Governs button copy push-wide**

### Exact Footprint Audited on `main`
The codebase currently mixes ASCII `"..."` (`menus/`, `catalog.tsv`) and Unicode `"…"` (`egui_shell/`), plus three leftover leading-ellipsis labels (`"More… Format Axes"`, `"More… Pairwise & CLD"`, `"More… Export Figure"` in `crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs:671,764,938`):

| Bucket | Exact Footprint | Examples |
|---|---|---|
| **1. Hardcoded button & context labels (`egui_shell/`)** | **27** literals across ~14 files | `Format Graph…`, `Style Subset in Graph…`, `Convert Table Archetype…`, `New Graph from Tab…`, `Manage All Variables…`, `Replace…`, `Clone Options…`, `More… Format Axes`, `Exclude with Reason…` |
| **2. Translated command / button keys (`locales/catalog.tsv`, all shipped locales)** | **42** keys (`cmd.*` 28, `r13.*` 4, `ctx.*` 3, `axes.*` 2, `fg.*` 2, `nav.new_analysis` 1, `welcome.*` 1, `feedback.*` 1 — `feedback.open_github` & `cmd.app.feedback` `"..."` retired by [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect)) | `cmd.ui.table.convertArchetype.open` (`Convert Table Archetype...`), `cmd.ui.graph.format.open` (`Format Graph...`), `fg.jump.format_axes` (`Format Axes…`) |
| **3. Top menu bar & command metadata** | **27** `menu.*` keys in `catalog.tsv` + **29** labels in `menus/ui_commands.rs` + **34** path strings in `menus/tooltips.rs` + localized `"Preferences..."` entries in `menus/menu_bar.rs:66–85` *(including `ar`/`he` added in `eed57fa`)* | `File/Open...`, `File/Save As...`, `Analyze/Analyze...`, `Edit/Preferences...` |
| **4. Text-input placeholders (`egui_shell/`)** | **15** `hint_text` literals | `"Search sheets..."`, `"Find in sheet…"`, `"Search variables…"`, `"Optional audit note…"` |
| **5. Ongoing progress copy** | **2** strings | `"Listening…"` (`app_modals.rs:868`), `pw.solving` (`"Solving design parameters..."`) |
| **6. Overflow button & truncation** | `⋯` (`U+22EF`) & `overflow_character: Some('…')` | Toolbar `"⋯"`, Results `"⋯ +N"`, elided cells/tabs/titles |

### Locked Scope — Option B + Clean Placeholders
- **Drop ellipses from Buttons, Menus, AND Input Placeholders (Buckets 1–4):**
  - Strip `…`/`...` from **Buckets 1, 2, and 3** across `egui_shell/` (including the three `"More… X"` labels in `graph_inspector.rs`), all shipped locales in `catalog.tsv`, `ui_commands.rs`, `tooltips.rs`, `menu_bar.rs`, and `apps/curvebook/src/mac_menu.rs`, then run `cargo xtask ui-commands` to sync `crates/mcp/src/ui_catalog.rs`.
  - Strip trailing `…`/`...` from **Bucket 4** (all `15` `hint_text` input placeholders: `"Search sheets"`, `"Find in sheet"`, matching native macOS `NSSearchField` `"Search"`).
  - Reserve `…` strictly for **Bucket 5** (in-progress `"Listening…"`, `"Solving…"`) and **Bucket 6** (real text truncation + `⋯` overflow).
  - **Structural win:** Zero shared-key divergence between menus, toolbars, Command Palette, and inspectors; one global invariant (*"if a label ends in `…`, it was truncated"*).
  - **Test blast radius:** Only **3 test files** (once [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect) removes `"Open on GitHub..."` from `feedback_github_issue.rs`, and `gui_wf_suite/gui_modal_workflows.rs` was retired in `R21.1` commit `a3dd51a`) assert exact menu-label strings with `"..."` (`r15_8_curated_menus.rs`, `audit_p6_sec14.rs`, `r10_27_layout_theme_paths.rs`). **Zero command IDs or widget IDs change.**
- **Rejected alternative:** *Option A (buttons only, keeping OS `…` in the native top menu bar)* — rejected to keep one global truncation-only invariant and eliminate shared-key stripping hacks.

### Docs & Automated Regression Guardrail
- **Design docs sync:** Update [`docs/design-language.md`](../design-language.md) §4 and [`docs/planning/gui-layout-spec.md`](gui-layout-spec.md) §6 / §10.2 / §10.7 (which currently prescribe trailing `…` on dialog-opening buttons) to state: *"Never put `…` on buttons, menus, or input placeholders; reserve `…` strictly for truncated text and active progress copy."*
- **Frame-parity gate (`crates/ui-egui/src/egui_shell/driver.rs`):** Add an assertion in `verify_egui_frame_parity` checking that no non-ellipsized button (`role == "button" && !node.ellipsized && node.label != "⋯"`) ends with `"…"` or `"..."`, catching hardcoded strings and all shipped locales automatically on every frame test.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/` (`trailing_studio/graph_inspector.rs`, `top_chrome.rs`, `results_view.rs`, `driver.rs`, and remaining audited button/placeholder files), `crates/ui-egui/src/menus/{ui_commands,tooltips,menu_bar}.rs`, `apps/curvebook/src/mac_menu.rs`, `crates/ui-egui/locales/catalog.tsv`, `crates/mcp/src/ui_catalog.rs`, `docs/design-language.md`, `docs/planning/gui-layout-spec.md`.
- **Tests:** `verify_egui_frame_parity` button-ellipsis assertion, `cargo xtask i18n-report`, `cargo xtask ui-commands --check`, and exact-label updates across the affected test files.

---

## 15. `R20.16` Unify dropdown, overflow & context menus with content-measured sizing and popup parity gates

- **Roadmap entry:** [`ROADMAP.md#r2016`](../ROADMAP.md#r2016-unify-dropdown-overflow--context-menus-with-content-measured-sizing-and-popup-parity-gates) · **Gate:** Post-**Beta** · **Complements:** [`R16.19`](../ROADMAP.md#r1619-replace-egui-combo-boxes-with-a-dropdown-that-can-grow-while-open), [`R20.9`](#8-r209-unified-top-toolbar-with-os-native-menu-placement), [`R10.35`](#1-r1035-portfolio-gallery-live-thumbnails-batch-workflows--navigation-overhaul)

### Current Problem & Root Causes
The **Split** preset dropdown (`crates/ui-egui/src/egui_shell/top_chrome.rs:1316`) and sibling `egui::Area` menus across `egui_shell/` exhibit a recurring `egui` layout bug:
1. **`set_min_width` + greedy `ui.available_width()` inside an unconstrained `egui::Area`:** `render_split_preset_popup` calls `ui.set_min_width(184.0)` without `set_width` or `set_max_width`, while `popup_action_button` (`top_chrome.rs:2334`) allocates `ui.available_width().max(120.0)`. Inside an unconstrained `Area`, `available_width()` reads the remaining viewport width, stretching rows abnormally wide or causing Frame-1-to-Frame-2 layout jumps as `egui` caches the Area's prior size.
2. **Hardcoded size guesses ignore actual item count and locale lengths:** `clamp_popup_pos` guesses `196 × 176 pt` for Split (actual 5-item height is `~148 pt`), and sibling menus hardcode arbitrary `156` / `172` / `204` / `210` / `232 pt` widths that either bloat short menus (`50% / 100% / Fit`) or truncate longer localized labels (`de`, `fr`, `ru`).
3. **Right-edge trigger misalignment:** `Split` sits at the far right edge of Tier 1 but anchors to `anchor.left()`, relying on `clamp_popup_pos` to push it left by the guessed `196 pt` rather than cleanly right-aligning to `anchor.right()`.
4. **Missing `role: "menu"` container metadata:** `render_split_preset_popup` (and several context menus) never record a `role: "menu"` container node in `FrameInspectMeta`, making them invisible to container-level geometry parity checks.

#### Verified Inventory of All `10` Ad-Hoc Action / Overflow / Context Menu `Area`s (`crates/ui-egui/src/egui_shell/`)
| Menu | Site | Current Sizing Defect |
|---|---|---|
| **Split presets** | `top_chrome.rs:1316` (`render_split_preset_popup`) | `set_min_width(184.0)` + greedy `available_width()`, guessed `196 × 176`, left-anchored on right-edge trigger, no `role: "menu"` |
| **Toolbar `⋯` overflow** | `top_chrome.rs:1455` (`render_tier2_overflow_popup`) | `set_min_width(204.0)` + greedy `popup_action_button` `available_width()` |
| **Results tabs `⋯` overflow** | `results_view.rs:758` (`render_result_tabs_overflow_popup`) | `set_min_width((popup_w - 12.0).max(96.0))` + `ui.available_width().max(80.0)` |
| **Footer Zoom / CVD / Sheet popups** | `status_footer.rs:925` (`render_footer_popup`) | Sets `min_width`/`max_width`; greedy `available_width()` stretches short popups (`50% / 100% / Fit`) to `max_width` |
| **Navigator sheet context menu** | `leading_dock.rs:2422` (`nav_sheet_ctx_menu`) | Hand-rolled `Area::new` + hardcoded `set_min_width(232.0)` |
| **Graph canvas context menu** | `canvas_view.rs:2543` (`graph_ctx_menu`) | Hand-rolled `Area::new` + hardcoded `set_min_width(232.0)` |
| **Layout panel context menu** | `canvas_view.rs:2793` (`layout_panel_ctx_menu`) | Hand-rolled `Area::new` + hardcoded `set_min_width(232.0)` |
| **Results table context menu** | `results_view.rs:1408` (`result_ctx_menu`) | Hand-rolled `Area::new` + hardcoded `set_min_width(156.0)` |
| **Data grid context menu** | `data_grid/multi_archetype.rs:1073` (`grid_ctx_menu`) | Hand-rolled `Area::new` + hardcoded `set_min_width(210.0)` |
| **MV variable-role menu** | `data_grid/mv_roles.rs:132` (`mv_role_popup`) | Hand-rolled `Area::new` + hardcoded `set_min_width(172.0)` |

*(Also verify `top_chrome.rs:1191` `egui_shell.toolbar.search_dropdown` sets explicit `set_width` so its popover cannot stretch across Frame 1 / Frame 2; non-menu overlays such as `FloatingNote`, `ProgressModal`, `sheet_find_bar`, hover previews, and modal dialog cards remain outside `action_menu_popup`.)*

### Target Design — Three-Layer Defense

#### Layer 1 — Canonical `ui_kit::action_menu_popup` primitive (`ui_kit.rs`)
Following the `R16.19` pattern (`widgets::dropdown` replacing `egui::ComboBox`):
- **Measure before layout:** Measure all visible item labels (plus optional icon and right-aligned shortcut columns) with `layout_no_wrap` before painting, compute `content_w = (max_row_w + horizontal_chrome_pad).clamp(min_w, (viewport_w - 16.0).min(max_w))`, and call **`ui.set_width(content_w)`** explicitly inside the popup frame so rows fill exact `content_w` with zero Frame-1-to-Frame-2 jitter.
- **Anchor-aware placement & flipping:**
  - Trigger-anchored dropdowns/overflows align to `anchor.right() - popup_w` when `anchor.center().x > viewport.center().x` (cleanly right-aligning `Split` and right-edge `⋯` triggers), and `anchor.left()` otherwise.
  - Flip above `anchor.top() - 4.0` when insufficient room exists below, then clamp inside `viewport_rect.shrink(4.0)`.
  - Context menus clamp pointer-anchored `(x, y)` cleanly within `viewport_rect.shrink(4.0)` using measured `(content_w, est_h)` and enable vertical scrolling only when taller than `(viewport_h - 16.0)`.
- **Uniform dismissal & inspection:** Handle `Escape` and outside pointer clicks uniformly, and **always** record a `role: "menu"` container node in `FrameInspectMeta` on `ZLayer::DropdownMenu` or `ZLayer::ContextMenu` (preserving every existing child `widget_id`, `command_id`, and AccessKit role).
- Migrate all `10` menus listed above onto `ui_kit::action_menu_popup`.

#### Layer 2 — Generic popup geometry invariants in `verify_egui_frame_parity` (`driver.rs`)
Extend `verify_egui_frame_parity` (`crates/ui-egui/src/egui_shell/driver.rs`) with five automatic checks across every recorded `role: "menu"` node on `ZLayer::DropdownMenu | ContextMenu`:
1. **No runaway bloat or dead space:** Fail if `popup_rect.width() > max(max_child_text_w + 96.0, 180.0)` (using `RenderedTextRun` already captured by `driver.rs`) or if bottom dead space `popup_rect.bottom() - last_item.bottom() > 24.0`.
2. **No needless truncation inside a popup:** Fail if any child item reports `ellipsized == true` while `popup_rect.width() < viewport_rect.width() - 48.0`.
3. **Child-in-container & uniform row widths:** Assert every menu item lies inside `popup_rect` and stacked rows share equal width within `±2 pt`.
4. **Viewport containment & anchor attachment:** Assert `popup.unclipped_rect` stays inside `viewport_rect`, and for trigger-anchored `DropdownMenu`s assert the popup sits within `12 pt` vertically of its trigger anchor and overlaps it horizontally.
5. **Two-frame size stability (`popup_matrix` test):** Open every dropdown, overflow, and context menu (`Split`, `Tier 2 ⋯`, `Results ⋯`, `Footer Zoom/CVD/Sheet`, `Navigator`/`Grid`/`Result`/`Graph`/`Layout`/`MV Role` menus) at `640 × 420`, `1280 × 800`, and `de` locale, asserting `|popup_rect_frame1 - popup_rect_frame2| <= 0.5 pt`.

#### Layer 3 — Static anti-pattern ratchet (`xtask`)
Add a lightweight policy scan in `xtask` (reusing `xtask/src/scan_util.rs` `walk_file_paths`/`rel_slash`/`is_regular_file` helpers from `R21.3` and `design_tokens.rs`'s `tokenize_blanked_code`/`CodeTok` token scanner from `eed57fa`, complementing `clippy.toml`'s `egui::ComboBox` ban) flagging any `egui::Area::new` outside `widgets.rs` / `ui_kit.rs` that calls `set_min_width(` without a matching `set_width(` or `set_max_width(` (or new ad-hoc menu `Area::new` sites outside `ui_kit::action_menu_popup`), with explicit `// popup-size-exempt:` opt-out only for verified non-menu overlays.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{ui_kit,top_chrome,results_view,status_footer,leading_dock,canvas_view,driver}.rs`, `crates/ui-egui/src/egui_shell/data_grid/{multi_archetype,mv_roles}.rs`, `xtask/src/{design_tokens,scan_util}.rs` (static ratchet).
- **Tests:** New `popup_matrix` integration test (`640 × 420`, `1280 × 800`, `de` locale, two-frame stability), `verify_egui_frame_parity` negative/positive unit checks for all 5 popup geometry invariants, and `xtask` unit test for the `Area::new` + `set_min_width` ratchet.

---

## 16. `R20.17` Replace in-app Feedback modal with direct GitHub issue-chooser redirect

- **Roadmap entry:** [`ROADMAP.md#r2017`](../ROADMAP.md#r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect) · **Gate:** Post-**Beta** *(Wave 0 standalone; ~1,100 LOC net deletion extending archived `R21`)*

### Target Behavior
- **Trigger points (unchanged):**
  1. Menu bar: **`Help › Send Feedback`** (`app.feedback`, registered in `crates/ui-egui/src/menus/menu_bar.rs:360–365`).
  2. About modal: **`Send Feedback`** button inside `About CurveBook` (`app_modals.rs:2014–2036`), right beside **`Copy version info`** (which already copies version/SHA/OS if the user wants to paste diagnostics into a GitHub bug report).
  3. Command palette / control channel: `app.feedback` with empty `{}` params.
- **Action:** Immediately invokes `PlatformUrlOpener::open_external_url(GITHUB_FEEDBACK_URL)` with:
  ```rust
  pub const GITHUB_FEEDBACK_URL: &str = "https://github.com/HauserGroup/curvebook/issues/new/choose";
  ```
  - `.github/ISSUE_TEMPLATE/config.yml` already sets `blank_issues_enabled: false` and configures GitHub's native template chooser (`bug_report.yml`, `feature_request.yml`, `numerical_result.yml`, `roadmap_task.yml`) plus both contact links (`Security vulnerability` and `What is planned, and in what order`). Leave `roadmap_task.yml` untouched.
- **Platform behavior (reusing existing `PlatformUrlOpener` adapters):**
  - **Desktop (`SystemPlatformUrlOpener::live()`):** opens in the OS default browser via `/usr/bin/open` (macOS), `xdg-open` (Linux), or PowerShell `Start-Process` via stdin (Windows) (`os_integration.rs:750–803`).
  - **Web/WASM (`WebPlatformUrlOpener`):** calls `window.open(url, "_blank", "noopener,noreferrer")` (`apps/curvebook-web/src/lib.rs:85–116`).
  - **Headless / tests (`SystemPlatformUrlOpener::guarded()` or `human_path_only` without a custom stub):** never spawns an OS browser; returns `Ok(OsAdapterStatus::unavailable(...))` cleanly.
- **Failure / unavailable fallback (no modal, no unsolicited clipboard writes):**
  - On `Err(err)` (spawn failure): `dispatch_ui_app_feedback` returns an error surfaced by `gui_window.rs:2262–2289` in its dismissible bottom-center `desktop.status_banner` toast.
  - On `Ok(status)` with `!status.available` (missing `xdg-open`, WASM popup blocker, or guarded test mode): returns `Ok(json!({ "url": GITHUB_FEEDBACK_URL, "opened": false, "availability": "unavailable", "detail": status.detail }))` and records a concise fallback message in `app.canvas_dialogs.status_banner` (`"Could not open browser. Visit https://github.com/HauserGroup/curvebook/issues/new/choose"`).
  - When invoked from inside **`About CurveBook`**, `active_dialog` is **not** mutated so `Copy version info` remains one click away.

### Security & Zero-Telemetry Contract
- Zero outbound HTTP/network calls from CurveBook (`cargo xtask telemetry` stays unchanged).
- Tighten `validate_github_issue_url(url: &str)` to an **exact constant match** (`url == GITHUB_FEEDBACK_URL`), eliminating query-string construction, percent-encoding, `MAX_GITHUB_ISSUE_URL_BYTES` truncation logic, and shell-metacharacter edge cases while keeping `PlatformUrlOpener` strictly non-generic across desktop and WASM.

### File-by-File Change & Deletion Inventory (~1,100 LOC Net Deletion)
- **Core & URL validation (~600 LOC removed):**
  - **Delete `crates/ui-egui/src/feedback.rs` (632 LOC removed):** Remove `FeedbackCategory`, `FeedbackSurface`, `FeedbackDiagnostics`, `FeedbackDraft`, `BuiltFeedbackUrl`, `MAX_GITHUB_ISSUE_URL_BYTES`, `MAX_ALLOWED_EXTERNAL_URL_BYTES`, `format_version_with_sha`, `percent_encode_query_component`, `build_github_issue_url*`, `build_feedback_markdown`, and query-builder unit tests; fold `GITHUB_FEEDBACK_URL`, exact-match `validate_github_issue_url`, and 1 tight allowlist unit test (~35 LOC) into `crates/ui-egui/src/os_integration.rs` (re-exported from `lib.rs` for `curvebook-web`).
  - `crates/ui-egui/src/os_integration.rs:750–803,2420–2452`: update fallback detail strings to drop Markdown references; update unit test to verify `GITHUB_FEEDBACK_URL` and assert query strings (`?template=...`) are rejected.
  - `apps/curvebook-web/src/lib.rs:85–116`: keep `WebPlatformUrlOpener` calling `validate_github_issue_url`; update popup-blocked detail strings (`104, 112`) to drop `"use Copy Markdown"`.
- **Modal, state, intents & dispatch (~450 LOC removed):**
  - `crates/ui-egui/src/egui_shell/app_modals.rs`: delete `ActiveDialog::Feedback` match arm (`318`), `render_feedback_modal`..`record_feedback_text_edit` (`2308–2600`, ~290 LOC, also removing blue-text site `2496`), and `"dialog.feedback.open_github"` (`2628`); keep `dialog.about.send_feedback` (`2014–2036`).
  - `crates/ui-egui/src/lib.rs`: drop deleted feedback type re-exports (`53–56`), export `GITHUB_FEEDBACK_URL` and `validate_github_issue_url`, delete `feedback_diagnostics` / `build_feedback_issue_url` / `build_feedback_markdown` / `copy_feedback_markdown` (`771–832`), and simplify `open_feedback_on_github` (preserving `!is_custom && is_human_path_only()` guard).
  - `crates/ui-egui/src/control_io.rs`: simplify `dispatch_ui_app_feedback` (`1124–1146`; lookup L1116, `ActiveDialog::Feedback` L1137) to call `app.open_feedback_on_github()` directly; drop `ActiveDialog::Feedback => "Feedback"` (`3698`); keep `"app.feedback"` in handler lookup, read-only guard, `host_routing.rs:114`, `desktop_host.rs:3741`, and `apps/curvebook-web/src/chrome_bridge.rs:1996`. *(Architecture note: `R21.1` commit `a3dd51a` collapsed dispatch/toggle/fallback ladders into `PatchedSheetDialogSpec`/`DialogOpenAction`/`lookup_ui_toggle_or_setter` in `control_io.rs`, `try_toggle_dialog_widget_click` at `dialog_actions.rs:984`, and `append_synthetic_dialog_fallbacks` at `inspect_bridge.rs:723`.)*
  - Remove dead glue across 9 files: `dialogs_canvas/mod.rs:152,268,324`, `dialogs_canvas/dialog_patch/mod.rs:341`, `egui_shell/dialog_actions.rs:481`, `egui_shell/inspect_bridge.rs:285`, `inspect.rs:317`, `modal_predicates.rs:767`, `egui_shell/frame_meta.rs:39,50–51`, `egui_shell/intents/{mod.rs:201–204,executors.rs:488–491,1224–1239}`, and `interaction.rs:1251–1254,1392–1394,1436–1439,1741–1744,1821–1824`.
- **Menus, i18n & MCP catalog:**
  - `crates/ui-egui/src/menus/ui_commands.rs:483–491`: keep `"app.feedback"` (preserving `UI_COMMANDS` count), change label `"Send Feedback..."` → `"Send Feedback"`, `params_doc` → `"{}"`, and update summary/beginner descriptions.
  - `crates/ui-egui/src/menus/tooltips.rs:137`: `"Help/Send Feedback..."` → `"Help/Send Feedback"`.
  - `crates/ui-egui/locales/catalog.tsv`: remove unused `feedback.open_github` (`267`); strip trailing `"..."` from `cmd.app.feedback` (`78`) and `menu.help.feedback` (`454`) across all shipped locales.
  - `crates/mcp/src/ui_catalog.rs:485–491`: regenerate via `cargo xtask ui-commands`.

### Code Touchpoints & Verification
- **Test updates:**
  - `crates/ui-egui/tests/ui_audit_suite/feedback_github_issue.rs` (123 LOC → ~75 LOC after `R21.2` slimming): keep `help_menu_registers_app_feedback_alongside_about`; assert `Help › Send Feedback` opens `GITHUB_FEEDBACK_URL` with `ActiveDialog::None`, `About CurveBook` `Send Feedback` opens `GITHUB_FEEDBACK_URL` without clobbering `ActiveDialog::About`, unavailable/guarded opener sets `status_banner` cleanly, and control-channel `app.feedback` returns `ok: true` with `result.url == GITHUB_FEEDBACK_URL`.
  - Swap `ActiveDialog::Feedback` fixtures in `r15_9_visual_design_tokens.rs:152,175,204` (`ActiveDialog::About` / `Checkpoints`), `r15_3_stage_canvas.rs:1060` (`ActiveDialog::About`), `zz_overlap_probe.rs:185` (drop `"feedback"`), `apps/curvebook/tests/desktop_suite/r17_3_idle_repaint.rs:345` (drop `"feedback"`), and `crates/ui-egui/tests/rem_ui_modals.rs:822–826` (`AUD-10-44` added in `eed57fa`, which clicks `"dialog.feedback.copy_markdown"`).
- **Gate commands:** `cargo xtask ui-commands --check`, `cargo test -p curvebook-ui-egui --test ui_audit_suite feedback`, `cargo test -p curvebook-ui-egui --test r15_shell_suite`, `cargo test -p curvebook --test desktop_suite r17_3_idle_repaint`, `cargo check -p curvebook-web`, `cargo xtask telemetry`, and `cargo xtask ci quick`.

---

## 17. `R20.18` Fix brand mark sizing & mipmap aliasing and make logo the Home button

- **Roadmap entry:** [`ROADMAP.md#r2018`](../ROADMAP.md#r2018-fix-brand-mark-sizing--mipmap-aliasing-and-make-logo-the-home-button) · **Gate:** Post-**Beta** · **Coordinates with:** [`R20.9`](#8-r209-unified-top-toolbar-with-os-native-menu-placement) (`top_chrome.rs`)

### Current Problem & Root Causes
Two compounding bugs (one in asset geometry, one in the GPU renderer) make the upper-left CurveBook mark simultaneously **half the size it looks in code** and **severely aliased**:
1. **Tiny visible mark (`~14.7 × 10.5 pt` inside `20 × 20 pt`):**
   - `crates/ui-egui/src/egui_shell/top_chrome.rs:428` allocates a `20 × 20 pt` square inside the `32 pt` Tier 1 bar.
   - `crates/ui-egui/src/egui_shell/brand.rs:9` embeds `assets/app-icon/hicolor/128x128/apps/org.curvebook.CurveBook.png`.
   - Because `assets/app-icon/source/approved-seven-pane.png` (`1536 × 1024 px`, `3:2`) already includes ~25% transparent margin around the seven-pane artwork, and `curvebook.svg` letterboxes that `3:2` canvas *again* inside a `1:1` square, opaque pixels in `128×128` only span `(21, 31)..(114, 97)` = **`94 × 67 px`** (`73%` width × `52%` height — nearly half the rect is empty transparent padding).
   - Drawn into `20 × 20 pt`, the actual visible logo is only **`14.7 × 10.5 pt`** — shorter than the `"Home"` button beside it. `Welcome Hub` (`welcome_hub.rs:468–471`, `32 × 32 pt`) and `About CurveBook` (`app_modals.rs:1958–1961`, `44 × 44 pt`) suffer the exact same vertical shrinkage.
2. **Grainy / aliased rendering (`egui-wgpu` ignores mipmaps):**
   - `brand.rs:40` sets `TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR }` assuming GPU mipmaps will filter `128×128` down to `20 pt`.
   - **`egui-wgpu 0.37.0` does not generate mipmaps at all:** it hardcodes `mip_level_count: 1` (`egui-wgpu/src/renderer.rs:726`) and `create_sampler` ignores `mipmap_mode`.
   - Consequently, the GPU downsamples `128×128` by **3.2× (2× Retina) to 6.4× (1×)** with plain `2×2` bilinear filtering—skipping `> 75%` of texels across the black bevel outlines and subtle checkered sheen baked into `approved-seven-pane.png`, turning them into visible crunch/grain.
3. **Redundant logo + `"Home"` button pair:** `top_chrome.rs:428–448` renders a non-interactive brand mark immediately beside a separate text `"Home"` button (`toolbar.home`), wasting `~45–60 pt` of toolbar width and violating user expectation that clicking the top-left app mark returns Home.

### Target Design & Interaction Contract
- **Trim padding & draw at natural `~1.43:1` aspect ratio (`brand.rs`):**
  - Crop out the double transparent letterboxing so the visible seven-pane artwork fills `~95%` of its bounding rect at its natural `~1.43:1` aspect ratio (`BRAND_ASPECT_RATIO`, derived from the trimmed opaque bounding box).
  - Source the embedded in-app mark from a high-resolution trimmed asset (e.g., `256×256` hicolor PNG trimmed or a dedicated trimmed PNG derived from `source/approved-seven-pane.png` via `packaging/icons.sh`, leaving OS hicolor/`.icns`/`.ico` icon-grid padding untouched) so every surface strictly downsamples rather than upscaling even at `2×` Retina (`About` at `63 × 44 pt` = `126 × 88 px` @2×, whereas trimmed `128×128` is only `94 × 67 px`).
  - **Surface target sizes (natural `1.43:1`):**
    - **Top toolbar (`top_chrome.rs`):** **`~32–34 pt` wide × `22–24 pt` tall** (only `+12 pt` wider than `20 pt`, zero change to bar height, `> 2×` larger visible height).
    - **Welcome Hub header (`welcome_hub.rs:468–471`):** **`46 × 32 pt`** (updating `text_w` calculation from `32.0` to `mark_w`).
    - **About CurveBook modal (`app_modals.rs:1958–1961`):** **`63 × 44 pt`** (updating `text_w` calculation from `44.0` to `mark_w`).
- **Eliminate graininess via exact-size premultiplied-alpha CPU area downsampling (`brand.rs`):**
  - Decode the trimmed RGBA mark once, compute physical pixel dimensions `(phys_w, phys_h) = ((rect.width() * ppp).round().max(1.0) as usize, (rect.height() * ppp).round().max(1.0) as usize)`, and **CPU area-downsample in premultiplied alpha once per `(phys_w, phys_h)`** (box/Lanczos area filter in premultiplied RGBA so transparent edges never pick up dark alpha-fringe halos).
  - Cache the exact `1:1`-pixel `TextureHandle` keyed by `(phys_w, phys_h)` in `ctx.data()` (small bounded cache covering Toolbar, Welcome, and About at current DPI). On a `2×` Retina display at `32 × 22 pt`, `egui-wgpu` receives an exact `64 × 44 px` pre-filtered texture—zero GPU downsampling aliasing and zero per-frame texture uploads.
- **Make the top-left logo act as the Home button (`top_chrome.rs`):**
  - Wrap the `~32–34 × 22–24 pt` brand mark in a clickable button (`Sense::click()`, `CursorIcon::PointingHand`, `>= 28 pt` hit height, subtle hover fill and selected highlight when `ctx_in.launcher_open`, `focus_ring` on keyboard focus) that dispatches `UiIntent::SetLauncherOpen { open: !ctx_in.launcher_open }` and closes open top-bar popups—removing the separate text `"Home"` button.
  - **Preserve `"toolbar.home"` verbatim:** Record the pure clickable logo button (no `"Home"` text caption) under `widget_id: "toolbar.home"`, `Role::Button`, with localized `localized_home_label(&app.locale)` as its AccessKit accessible label and hover tooltip, so every existing workflow and desktop test (`human_wf_*`, `r15_3_welcome_nav.rs`, `r15_3_menus_chrome.rs`, `r15_1_host_backend.rs`) continues to click `"toolbar.home"` with zero widget-ID churn.

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{brand,top_chrome,welcome_hub,app_modals}.rs`, `assets/app-icon/` & `packaging/icons.sh` (if generating a dedicated trimmed in-app PNG), `assets/app-icon/README.md`.
- **Tests:** Update `brand.rs` unit tests for trimmed `~1.43:1` aspect ratio, premultiplied-alpha downsampling (no dark alpha fringe), and per-`(phys_w, phys_h)` single-upload caching; verify `crates/ui-egui/tests/r15_shell_suite/r15_3_welcome_nav.rs`, `apps/curvebook/tests/desktop_suite/{r15_3_menus_chrome,r15_1_host_backend}.rs`, `human_wf_*`, and `cargo xtask assets` / `design-tokens`.

---

## 18. `R20.19` Fix Desktop GUI Audit P1 blockers & WF-59 offscreen layout controls (Beta gate)

- **Roadmap entry:** [`ROADMAP.md#r2019`](../ROADMAP.md#r2019-fix-desktop-gui-audit-p1-blockers--wf-59-offscreen-layout-controls-beta-gate) · **Gate:** **Beta** *(Wave 0 immediate release blockers from the 2026-11-25 Desktop GUI Audit at rev `622f273`; rebased onto `eed57fa` where `WF-59` production guard landed in `ec94ecd` and Analyze initial checkbox state partially landed in `eed57fa` `AUD-10-07`)* · **Coordinates with:** [`R20.11`](#10-r2011-focused-vs-unfocused-split-pane-selection-xy-selection-sync--header-coordinate-disambiguation)

### Confirmed Defects & Root Causes
1. **P1 — Delete clears cells in both split tables (`crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs:465,553,560`):**
   - **Repro:** Open two editable tables containing `1` and `42` in the same cell coordinate in **Split → 2-Up**, click `42` in the right pane, press `Delete` once, then click `Undo` once. Both cells are emptied; one `Undo` restores only `42`, leaving `1` silently deleted.
   - **Root cause:** `handle_multi_keyboard_shortcuts` (`shortcuts.rs:553`, guard `L560`) checks `!is_grid_focus_active(ui, app, &ds.id) && fstate.aux.selected_x_cell.is_none() && app.grid.tool.active.is_none()`. Because `app.grid.tool.active` is shared globally, **both** rendered split grids pass line 560, process the same keyboard event, and emit separate `table.clearCells` commands for their own `ds.id`.
2. **P1 — Analyze picker substitutes fake `Col A..H` titles & allows empty-selection fallback (`analyze_dialog.rs:596–605,958–995`, `stats_modals/analyze_picker.rs:325,353,384`):**
   - **Partial fix in `eed57fa` (`AUD-10-07`):** `analyze_picker.rs:353` now starts boxes visually checked (`az.selected_data_sets.contains(...) || (az.selected_data_sets.is_empty() && var.selected)`), and fallback `analyze.dataset.*` (`L390–408`) uses `SetAnalyzeSelectedDatasets` with a min-1 guard.
   - **Still open:** `collect_variable_candidates` (`analyze_dialog.rs:958–995`) still populates `variable_candidates` for **every** table archetype (not just `MultipleVariables`) with fake `title: "Col A..H"` (`L987`) and `selected: true`, so `analyze_picker.rs:325` (`if !az.variable_candidates.is_empty()`) still wins and leaves `analyze_picker.rs:384`'s named `"Data Sets / Columns"` path unreachable dead code; meanwhile `toggle_variable` (`analyze_dialog.rs:596–605`) still allows empty `selected_data_sets = []` (silently reverting to All).
3. **P2 (Beta workflow failure) — Layout inspector offscreen size controls (`trailing_studio/page_layout.rs:589,598–615,634–651`) — `[Resolved on main in ec94ecd (R20.4)]`:**
   - **Original repro:** `human_wf_59_65::wf59_human_layout_equalize_scaling_transparent_overlay_and_inset` (`1440 × 600` English frame) failed on `inspector.layout.panel.width_pt` and `inspector.layout.panel.height_pt` with empty/inverted clipped bounds (`top = 596 pt`, clipped `bottom = 572 pt`).
   - **Resolution on `main` (`ec94ecd`):** `page_layout.rs:589` now computes `vis_clip` and guards `meta.record_response` for `inspector.layout.panel.width_pt` (L603) and `inspector.layout.panel.height_pt` (L639) with `rect.intersect(vis_clip).is_positive() && interact_rect.intersect(vis_clip).is_positive()`. Because `ec94ecd` added no test (`r15_3_stage_canvas.rs:1098` exercises visible controls only), Wave 0 `R20.19` retains `wf59` verification and the explicit below-the-fold → scroll-into-view → edit regression test below.

### Target Design & Verification
- **Single grid shortcut recipient (`data_grid/shortcuts.rs`, `data_grid/mod.rs`, `center_stage.rs`):** Thread `pane_focused: bool` (reused by `R20.11`) into `handle_multi_keyboard_shortcuts` (`shortcuts.rs:553`) and return immediately whenever `!pane_focused`, ensuring `Delete`, `Backspace`, `Cut`, `Paste`, `Exclude`, `F2`, typing, and arrow navigation target strictly the focused split table.
- **Accurate Analyze picker titles & selection semantics (`analyze_dialog.rs`, `stats_modals/analyze_picker.rs`):** Reserve `variable_candidates` (`analyze_dialog.rs:958–995`) for `MultipleVariables` tables so XY/Column/Grouped/Survival/Contingency tables take `analyze_picker.rs:384`'s named `"Data Sets / Columns"` path (showing real series names, accurate initial checkbox state matching default inclusion, and toggling only the clicked dataset); tighten `toggle_variable` (`L596–605`) empty-selection boundary for `MultipleVariables` as well.
- **Offscreen visibility guard (`trailing_studio/page_layout.rs`):** Already landed in `ec94ecd` (`R20.4`); verify below-the-fold layout spinbuttons stay unrecorded until scrolled into view and remain editable once scrolled into view.
- **Regression tests:**
  - Two-table split GUI test from both panes verifying `Delete`, `Backspace`, `Cut`, `Paste`, `Exclude`, and arrow navigation mutate only the focused table and single `Undo` fully restores state.
  - Update `crates/ui-egui/tests/rem_ui_modals.rs:423` (`AUD-10-07` added in `eed57fa`, which currently clicks `"analyze.mv.var.0"` on a `Column` table) to assert `"analyze.dataset.0"` (real titles) on `Column` and `"analyze.mv.var.0"` on `MultipleVariables`, plus add a synthetic named-series XY Analyze picker test asserting real series names, accurate initial checked states, single-item toggle semantics, empty-selection boundary, and committed fit `selected_data_sets` membership.
  - Verify `wf59_human_layout_equalize_scaling_transparent_overlay_and_inset` passes isolated + full suite, and add the explicit below-the-fold → scroll-into-view → edit regression test for `inspector.layout.panel.{width_pt,height_pt}` (not covered by `ec94ecd`).

---

## 19. `R20.20` Fix Desktop GUI Audit P2 split focus cycling, per-sheet result tabs, context-menu copy & first-run fit cancel

- **Roadmap entry:** [`ROADMAP.md#r2020`](../ROADMAP.md#r2020-fix-desktop-gui-audit-p2-split-focus-cycling-per-sheet-result-tabs-context-menu-copy--first-run-fit-cancel) · **Gate:** Post-**Beta** · **Coordinates with:** [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting), [`R20.11`](#10-r2011-focused-vs-unfocused-split-pane-selection-xy-selection-sync--header-coordinate-disambiguation), [`R20.16`](#15-r2016-unify-dropdown-overflow--context-menus-with-content-measured-sizing-and-popup-parity-gates)

### Confirmed Defects & Root Causes
1. **Compact split `F6` / `Shift+F6` focus cycling stalls (`center_stage.rs:533,665,675`):** At `640 × 420` in `CompactSinglePane` mode, line 533 consumes `ui.pane.cycle` whenever `split_active` is true and toggles `focused_split_pane` to the hidden slot; line 675 then immediately resets `focused_split_pane = visible_slot`, trapping keyboard focus inside the center stage in both directions.
2. **Split result sheets active-tab isolation (`results_view.rs:83,250–264,304–333`, `center_stage.rs:467,484`) — `[Largely resolved in eed57fa (AUD-09-24/AUD-09-25)]`:** `eed57fa` added `ResultsViewEguiState::per_sheet_active_tab: BTreeMap<SheetId, usize>`, `active_tab_for_sheet`, and `select_result_tab_for_sheet`, and `results_hooks.rs:555` (`copy_result_table_to_clipboard`) already reads `per_sheet_active_tab` (tested in `crates/ui-egui/tests/rem_ui_views.rs:608–720`). Remaining gaps: `Ctrl/Cmd+PageUp/PageDown` (`results_view.rs:322`) still needs `pane_focused` gating (`R20.11`), `UiIntent::SelectResultTab` carries no `sheet_id` (primary-only), and `center_stage.rs:467,484` (`sync_sheet_find_before_stage`) still reads `active_tab_index` and pushes `UiIntent::SelectResultTab` without `per_sheet_active_tab`.
3. **Results context menu copies a previous selection (`results_view.rs:1086`):** On right-click, line 1086 only updates `selection_anchor` / `selected_cell` when `state.selected_range(rs_id, active_tab_idx).is_none()`. Right-clicking a cell outside an existing selection opens the menu over the clicked cell (`19.8000`) while **Copy Selection** copies the old cell (`14.2000`).
4. **First-run nonlinear fit cancellation (`analyze_picker.rs`, `fit_modal.rs`):** First-run nonlinear regression commits a default result sheet before opening Parameters, so clicking `Cancel` leaves the result sheet in the project rather than reverting or clearly communicating the already-committed state.
*(Audit note: `WF-28` passed in isolation; its full-suite `3832 ms` vs. `1500 ms` timing under parallel contention is recorded for a controlled repeat rather than tracked as a logic defect.)*

### Target Design & Verification
- **Compact split landmark traversal (`center_stage.rs:533,665,675`):** Gate internal `Primary ↔ Secondary` `ui.pane.cycle` consumption on `orientation != SplitOrientation::CompactSinglePane` (only consume when both split panes are simultaneously visible), letting `F6` / `Shift+F6` advance to the next/previous visible workspace landmark while keeping the active visible sheet and both split assignments intact.
- **Finish per-sheet / per-pane active Result tab wiring (`results_view.rs:322`, `center_stage.rs:467,484`, `intents/`):** Build on `eed57fa`'s `per_sheet_active_tab` map (`results_view.rs:83,250–264,304–333`, `results_hooks.rs:555`) by adding `sheet_id` to `UiIntent::SelectResultTab`, gating `Ctrl/Cmd+PageUp/PageDown` (`results_view.rs:322`) on `pane_focused` (`R20.11`), and reading `per_sheet_active_tab` in `center_stage.rs:467,484` (`sync_sheet_find_before_stage`).
- **Context-menu cell targeting (`results_view.rs:1086`):** Preserve multi-cell selections when right-clicking *inside* the selected range; when right-clicking *outside* the current selection, move selection to the clicked cell immediately (standard spreadsheet convention) so **Copy Selection** copies the clicked cell, verified via stub clipboard tests.
- **First-run fit Cancel contract (`analyze_picker.rs`, `fit_modal.rs`, aligning with `R19.1`):** Defer creating/committing the first-run nonlinear result sheet until **Parameters › OK** is clicked (`Cancel` creates zero sheets; `OK` commits once with the chosen equation, datasets, weighting, and constraints; editing an existing result sheet via `Parameters` continues to edit in place), covered by a workflow test.

---

## 20. `R20.21` Structured hover tooltips for commands, scientific readouts, Navigator & Inspector

- **Roadmap entry:** [`ROADMAP.md#r2021`](../ROADMAP.md#r2021-structured-hover-tooltips-for-commands-scientific-readouts-navigator--inspector) · **Gate:** Post-**Beta** · **Recommended wave:** Wave 3 (after [`R20.9`](#8-r209-unified-top-toolbar-with-os-native-menu-placement), [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting), [`R20.12`](#11-r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown))

### Current Problem & Root Causes
CurveBook already has command tooltip metadata (`crates/ui-egui/src/menus/tooltips.rs`), but hover UX sits in an awkward middle ground:
1. **Flat, redundant text walls that drop keyboard shortcuts:** `format_command_tooltip` (`top_chrome.rs:2087`) unconditionally concatenates `format!("{}\n{}", t.summary, t.beginner)` (~35 repetitive words) while dropping `CommandTooltip`'s already platform-formatted shortcut (`tooltips.rs:28–36`)—contributing to shortcuts leaking into button labels like `Parameters (Cmd-T)`.
2. **No visual hierarchy:** Every tooltip uses raw `resp.on_hover_text(&str)` with egui's default plain frame (`tokens.rs:228–231,282`), with no structural distinction between **title**, **shortcut badge**, **1-line description**, and **why disabled**.
3. **Missing tooltips in the highest-value scientific & disambiguation surfaces:**
   - **Statistical headers & values (`results_view.rs:1221–1228`):** Only elided or ambiguous cells get tooltips; hovering `Hougaard |g1|`, `Dependency`, `Confidence Interval`, `Sy.x`, `AICc`, or a rounded 4-decimal number yields nothing.
   - **Navigator & Family sheets (`leading_dock.rs:1540`):** Only echoes the raw title when elided—never showing sheet kind/archetype, dimensions/graph type, or the multi-source list called for in [`gui-layout-spec.md`](gui-layout-spec.md) §4 (`lines 239–240`).
   - **Inspector & Graph Canvas:** Zero tooltips across `trailing_studio/` (e.g., `Display decimals`, `Exclude Selected Subset`) and `canvas_view.rs` (even though `canvas_view.rs:1168` already implements `hit_test_point_indexed` returning `PlottedPointMeta`, no hover `(X, Y, series, row)` readout is shown).
   - **Generic disabled reasons:** Controls fall back to `"unavailable"` (`leading_dock.rs:567`) instead of saying what unlocks them.

### Target Design & Interaction Contract
- **One canonical helper (`ui_kit::show_tooltip` / `TooltipSpec` via `on_hover_ui`):**
  - Token-styled surface card (`tokens.surface_bg`, `1 pt` `tokens.border`, subtle shadow, `max_width = 300–320 pt`) with three clear bands:
    1. **Header:** Strong **Title** + right-aligned muted/bordered **shortcut pill** (`⌘K`, `⇧⌘P`, at `11.5–12.0 pt` via `format_shortcut_for_platform` and [`R20.13`](#12-r2013-live-desktop-font-installation-chrome-border-cleanup-copy-polish--content-aware-initial-split)'s crisp symbol fallback).
    2. **Body:** **1 concise muted sentence** (GUI renders `CommandTooltip.summary` only while preserving `.beginner` intact for MCP/docs).
    3. **Disabled footer (when disabled):** Actionable explanation of what unlocks the action (e.g., `"Select a data table to run an analysis"` — never generic `"unavailable"`).
- **Strict interaction hygiene:**
  - Set `style.interaction.tooltip_delay` to **`450 ms`** (`tokens.rs`) so moving across grids, canvases, or toolbars never flashes popups, and dismiss immediately on click, scroll, or drag.
  - **No echo tooltips:** Never attach tooltips that merely repeat self-explanatory un-truncated text (`Cancel`, `OK`, `Close`, unclipped table headers).
- **Four targeted rollout surfaces:**
  1. **Commands & icon-only controls (`top_chrome.rs`, `menus/tooltips.rs`, `ui_kit.rs`):** Title + shortcut pill + 1 concise `summary` sentence; lets button/tab labels stay short without embedded shortcut strings.
  2. **Scientific readouts (`results_view.rs`, `data_grid/multi_archetype.rs`):**
     - Diagnostic column headers (`Hougaard |g1|`, `Dependency`, `Sy.x`, `AICc`, `Confidence Interval`) → crisp 1-line statistical definition + interpretation threshold (e.g., `< 0.1 linear; > 1.0 skewed CI`).
     - Rounded result cells → full numeric precision + bound/warning detail on hover.
     - Excluded data cells (`cell_exclusion_tooltip` at `multi_archetype.rs:852`) → rendered in the structured tooltip card.
  3. **Navigator & Family sheets (`leading_dock.rs`):** Hovering any sheet row or `N sources` badge surfaces sheet kind/archetype, dimensions or graph kind, and upstream source table list.
  4. **Disabled controls, Inspector & Graph Canvas point hover (`leading_dock.rs:567`, `trailing_studio/`, `canvas_view.rs:1168`):** Replace generic `"unavailable"` reasons everywhere; add concise explanations on non-obvious Inspector controls (`Display decimals`, `Exclude Selected Subset`); and add floating `(X, Y, series, row)` hover readouts on Graph Canvas points reusing existing `hit_test_point_indexed` (`450 ms` delay, immediate drag/scroll dismissal).

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/{ui_kit,tokens,top_chrome,results_view,leading_dock,canvas_view}.rs`, `crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs`, `crates/ui-egui/src/egui_shell/trailing_studio/`, `crates/ui-egui/src/menus/tooltips.rs`, `crates/ui-egui/locales/catalog.tsv`.
- **Tests:** `ui_kit` unit/frame tests for structured tooltip layout, shortcut pill rendering, `450 ms` delay, immediate click/scroll/drag dismissal, and no-echo invariant; Results header/precision tooltip tests; Graph Canvas point hover readout test; Navigator multi-source hover test; and `cargo xtask i18n-report` / `design-tokens`.

---

## 21. `R20.22` Apple HIG alert/modal button order, macOS Window menu & menu-bar parity

- **Roadmap entry:** [`ROADMAP.md#r2022`](../ROADMAP.md#r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) · **Gate:** Post-**Beta** · **Coordinates with:** [`R20.15`](#14-r2015-eliminate-trailing-ellipses-on-action-buttons-and-unify-ellipsis-semantics), [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect)

### Current Problem & Root Causes
A platform-conventions audit identified four concrete drifts not already covered by `R20.9`–`R20.21` (and highlighted that `docs/design-language.md` needs an explicit cross-platform boundary rather than citing Apple HIG alone):
1. **Misclick-prone alert & modal button order (`app_modals.rs:371–426`, asserted by `r15_9_visual_design_tokens.rs:147–184`, `UnsavedChanges` at L174):** `Unsaved Changes` renders buttons left-aligned in the order `[Cancel] [Don't Save] [Save]`, placing destructive `Don't Save` directly between `Cancel` and `Save` instead of separating destructive actions on the leading edge (`[Don't Save]     [Cancel] [Save]`). Similarly, `About CurveBook` (`app_modals.rs:1958–2060`) renders as a heavy blocking modal requiring an explicit `OK` click rather than a lightweight dismissible panel.
2. **Missing macOS `Window` menu & duplicated app-menu items (`apps/curvebook/src/mac_menu.rs:518–553`, `menu_bar.rs:261,362`):** The live `muda` macOS menu bar builds `CurveBook` + `File..Help` with **no `Window` submenu** (`Minimize ⌘M`, `Zoom`, `Enter Full Screen`, `Bring All to Front` / open window list), and duplicates both **`About CurveBook`** (in `Help`, `menu_bar.rs:362`) and **`Settings` / `Preferences`** (in `Edit`, `menu_bar.rs:261`) even though `mac_menu.rs:525,527` already places both in the `CurveBook` app menu (`app.quit` at `L533`).
3. **`Settings…` vs. `Preferences...` inconsistency:** `mac_menu.rs:37` uses modern macOS `"Settings…"` (Unicode ellipsis), whereas `crates/ui-egui/src/menus/menu_bar.rs:66–85, 155–160` still uses `"Preferences..."` (ASCII dots; `eed57fa` also added `"ar"` and `"he"` entries).
4. **Shortcut standard drifts (`shortcuts.rs:42–48`, `ui_commands.rs:54,72,108`, `menu_bar.rs:157`, `crates/engine/src/cmd/sheet.rs:1895`):** `⌘T` is overloaded across `ui.editParams.open` (`ShortcutScope::ResultSheet`) and `sheet.createData` (`ShortcutScope::NonResultSheet`); `app.preferences` falls back to `""` outside `mac_menu.rs:527` instead of binding `Cmd+,`/`Ctrl+,`; `ui.navigator.toggle` (`Cmd+Shift+N`) and `ui.family.toggle` (`Cmd+Shift+F`) collide with standard New Window/Folder and Global Find conventions; `Cmd+=`/`Cmd+-` (Zoom In/Out) and `Cmd+W` (Close Window) are missing from the shortcut registry/menus *(note: `mac_menu.rs:188–235` `parse_muda_accelerator` already handles `Cmd++`/`Cmd+/` since `eed57fa`)*; and non-macOS builds do not accept `Ctrl+Y` as a secondary Redo alias alongside `Ctrl+Shift+Z`. *(Verified note: `crates/engine/src/cmd/mod.rs:459`'s `"Ctrl+L"` is inside a unit test mock, not a production binding; normalize it to `"Cmd+L"` alongside this audit.)*

### Cross-Platform Adaptive Boundary (~20 Lines in One Shared Helper) & 4-Layer Shortcut Standard Stack
Everything in `ui_kit` stays 100% shared across macOS, Windows, and Linux; **only three muscle-memory conventions adapt by target OS**, and all keyboard shortcuts follow a clean-room **4-layer public standard precedence stack** (both documented in [`docs/design-language.md`](../design-language.md)):

| Convention | macOS | Windows | Linux |
|---|---|---|---|
| **Menu bar** | Native `NSMenu` (`muda`): `CurveBook` app menu (`About`, `Settings`, `Hide`, `Quit`) + `Window` menu; do **not** duplicate `About` (`Help`), `Settings` (`Edit`), or `Quit` (`File`). | In-window menu bar: `Edit › Settings`, `File › Quit/Exit`, `Help › About CurveBook`. | Same in-window layout as Windows (or native desktop menu if added later). |
| **Shortcuts** | `⌘`, `⇧`, `⌥`, `⌃` (with `SFNS.ttf` / `Lucida Grande` ahead of `Apple Symbols`, `R20.13`). | `Ctrl+`, `Shift+`, `Alt+` (`format_shortcut_for_platform`) + secondary `Ctrl+Y` Redo alias alongside `Ctrl+Shift+Z`. | `Ctrl+`, `Shift+`, `Alt+` + secondary `Ctrl+Y` Redo alias alongside `Ctrl+Shift+Z`. |
| **Modal footer order** | Unified trailing: `[Don't Save]     [Cancel] [Save]` | Unified trailing: `[Don't Save]     [Cancel] [Save]` *(matches macOS, GNOME/GTK4, and modern cross-platform apps like Figma/Linear/VS Code; keeps CI layout assertions deterministic)* | Unified trailing: `[Don't Save]     [Cancel] [Save]` *(matches GNOME/GTK4)* |

**4-Layer Keyboard Shortcut Reference Stack (Precedence Order):**
1. **OS & Core App (Never override):** Apple HIG *Keyboard* (macOS), Microsoft Fluent/WinUI *Keyboard interactions* (Windows), GNOME HIG / KDE *Standard Shortcuts* (Linux), and IBM CUA (`N`, `O`, `S`, `⇧S`, `P`, `W`, `Q`, `Z`, `⇧Z`/+`Ctrl+Y`, `X`/`C`/`V`, `A`, `F`, `,`, `+`/`-`/`0`, `F6`/`⇧F6`, `Esc`, macOS `⌘H`/`⌘M`).
2. **Widget Navigation & Accessibility:** **W3C WAI-ARIA Authoring Practices Guide (APG)** patterns for `grid` (`Arrows`, `Shift+Arrows`, `Home`/`End`, `PageUp`/`PageDown`, `F2`, `Enter`/`Tab`, `Delete`/`Backspace`), `tree` (`↑`/`↓`, `←`/`→`, `Shift+↑/↓`), `tablist` (`Ctrl/Cmd+PageUp`/`PageDown`), and `dialog`/`menu` focus traps.
3. **Spreadsheet Grid Muscle Memory:** Shared open spreadsheet conventions (`Cmd/Ctrl+Arrows` block-edge jump, `Cmd/Ctrl+Shift+Arrows` extend, `Shift+Space` row, `Ctrl+Space` column, `Cmd/Ctrl+Shift+V` Paste Special).
4. **Modern Workbench & Domain Actions:** `Cmd/Ctrl+K` Command Palette, `Cmd/Ctrl+\` Split View, `Cmd/Ctrl+B` left sidebar toggle, `Cmd/Ctrl+Shift+…` domain workflows (`Analyze`, `New Graph`, `Export`, `New Data Table`, `Edit Parameters`, `Family/Lineage`).

### Target Design & Interaction Contract
- **Safe unified trailing alert/modal footer layout (`app_modals.rs`, `r15_9_visual_design_tokens.rs`):**
  - Place the Primary/Default button on the **trailing (right)** edge, `Cancel` immediately to its left, and separate destructive actions (`Don't Save`) on the **leading (left)** edge with flexible space between them: `[Don't Save]     [Cancel] [Save]` consistently across macOS, Linux, and Windows.
  - Apply the consistent trailing-primary footer rhythm (`[Secondary/Destructive]   [Cancel] [Primary]`) across `app_modals.rs` and update `r15_9_visual_design_tokens.rs:147–184` (`UnsavedChanges` at L174) to assert `dont_save_r.left() < cancel_r.left() < save_r.left()` with separation gap.
  - Convert `About CurveBook` (`app_modals.rs:1958–2060`) to a lightweight dismissible panel closing on `Esc`, outside click, or a quiet top-right `×` close button—removing the blocking `OK` button while keeping `Copy version info` (preserving `app_modals.rs:2000–2012`'s `app.write_clipboard_text` adapter routing added in `eed57fa`) and `Send Feedback` one click away.
- **OS-adaptive menu bar placement & `"Settings"` unification (`apps/curvebook/src/mac_menu.rs`, `crates/ui-egui/src/menus/menu_bar.rs`):**
  - Insert a standard macOS **`Window`** submenu before `Help` using `muda::PredefinedMenuItem` (`minimize`, `maximize`/zoom, `fullscreen`, separator, `bring_all_to_front` / window list).
  - Omit `app.about` (`Help`), `app.preferences` (`Edit`), and `app.quit` (`File`) when `host_draws_menu_bar == true` so macOS owns them solely in the `CurveBook` app menu, while Windows/Linux in-window menus include `File › Quit/Exit`, `Edit › Settings`, and `Help › About CurveBook`.
  - Standardize on dot-free **`"Settings"`** (per `R20.15` Option B) across all platforms and all shipped locales (`mac_menu.rs:37`, `menu_bar.rs:66–85`, `catalog.tsv`), preserving command ID `"app.preferences"` and widget ID `"menu.app.preferences"` unchanged.
- **Shortcut standard alignment (`shortcuts.rs`, `ui_commands.rs`, `menu_bar.rs`, `interaction.rs`, `cmd/sheet.rs`, `cmd/mod.rs:459`):**
  - Bind **`Cmd+,`** (`Ctrl+,` on Windows/Linux) for `app.preferences` in `shortcuts.rs` / `menu_bar.rs:157` across all three OSes.
  - Assign **`Cmd+Shift+T`** (`⇧⌘T`) to `sheet.createData` (*New Data Table*, pairing with `Cmd+Shift+G` New Graph and `Cmd+Shift+A` Analyze) and **`Cmd+Shift+P`** (`⇧⌘P`) to `ui.editParams.open` (*Edit Analysis Parameters*, mnemonic **P**arameters), retiring `ShortcutScope` `Cmd+T` overloading.
  - Rebind **`ui.navigator.toggle`** from `Cmd+Shift+N` → **`Cmd+B`** (standard left-sidebar toggle) and **`ui.family.toggle`** from `Cmd+Shift+F` → **`Cmd+Shift+L`** (*Lineage / Family*, freeing `Cmd+Shift+F` for project-wide Find).
  - Ensure standard zoom (`Cmd+=` Zoom In, `Cmd+-` Zoom Out alongside `Cmd+0` Fit), `Cmd+W` (Close Window), and secondary non-macOS **`Ctrl+Y`** Redo (`!cfg!(target_os = "macos")`) are registered cleanly in `shortcuts.rs` and surfaced in `View`/`File` menus and tooltips.
  - Normalize `cmd/mod.rs:459`'s test mock from `"Ctrl+L"` to `"Cmd+L"`, and document both the 3-row **Cross-Platform Conventions** table and the **4-layer shortcut reference stack** in [`docs/design-language.md`](../design-language.md).

### Code Touchpoints & Verification
- **Files:** `crates/ui-egui/src/egui_shell/app_modals.rs`, `apps/curvebook/src/mac_menu.rs`, `crates/ui-egui/src/menus/{menu_bar,shortcuts,ui_commands}.rs`, `crates/ui-egui/src/interaction.rs`, `crates/engine/src/cmd/{sheet,mod}.rs`, `crates/ui-egui/locales/catalog.tsv`, `crates/mcp/src/ui_catalog.rs`, `docs/design-language.md`, `crates/ui-egui/tests/r15_shell_suite/r15_9_visual_design_tokens.rs`.
- **Tests:** Update `r15_9_visual_design_tokens.rs` footer-order assertion (`dont_save < cancel < save` with leading/trailing separation); add `mac_menu.rs` unit tests asserting `Window` submenu presence and zero duplicate `app.about` / `app.preferences` / `app.quit` entries across `Help`/`Edit`/`File`; update shortcut tests for `Cmd+,`, `Cmd+B`, `Cmd+Shift+L`, `Cmd+Shift+T`, `Cmd+Shift+P`, `Cmd+=`/`Cmd+-`, `Cmd+W`, and non-macOS `Ctrl+Y` Redo; run `cargo xtask ui-commands --check` and `cargo xtask i18n-report`.

---

## Resolved Pre-Implementation Decisions (2026-11-25)

All 16 open design choices across the push were resolved during `/grill-me` on 2026-11-25 and folded into the item specs above and [`docs/ROADMAP.md`](../ROADMAP.md):

| # | Item | Topic | Locked Decision |
|---|---|---|---|
| 1 | [`R10.35`](#1-r1035-portfolio-gallery-live-thumbnails-batch-workflows--navigation-overhaul) | Gallery `All` view section headers | Collapsible section headers when `Sort` is `Navigator order` or `Type`; flat grid for `Name A–Z` / `Series-Rows` |
| 2 | [`R10.36`](#2-r1036-project-navigator-collapsed-rail--section-vector-icons) | Results glyph & folder-header icons | Crisp vector `Σ` glyph for `Results` + echo shared section icons on collapsible folder headers |
| 3 | [`R19.6`](#3-r196-fix-hougaard-skewness-sign-mismatch-and-false-zero-for-uncomputed-entries) | Hougaard skewness display | Keep header `"Hougaard \|g1\|"` and display `\|g1\|` via `.abs()` (preserve signed $g_1$ in solver/JSON and hover tooltip) |
| 4 | [`R19.7`](#4-r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics) / [`R20.10`](#9-r2010-results-table-ci-first-column-sizing-borderless-underline-tabs-quiet-header-actions--explicit-ci-formatting) | Unstable & both-open CI copy | `"Unstable"` for unstable Best-fit cells (raw iterate in tooltip/JSON) + `"Not bounded"` for both-open CIs (canonical across `R19.1`, `R19.7`, `R20.10`) |
| 5 | [`R20.9`](#8-r209-unified-top-toolbar-with-os-native-menu-placement) | Unified toolbar `Undo` / `Redo` | Convert `Undo` and `Redo` to crisp `28 pt` vector icon buttons with localized AccessKit labels and `⌘Z` / `⇧⌘Z` tooltips |
| 6 | [`R20.12`](#11-r2012-navigator--family-disambiguation-subtitles-and-labeled-kind-filter-dropdown) | Navigator two-line row density | Two-line `38 pt` rows on `Graphs`, `Results`, and any duplicate-titled sheet (`has_duplicate_title == true`); compact `26 pt` for unambiguous `Data`/`Info`/`Layouts` |
| 7 | [`R20.14`](#13-r2014-reserve-blue-foreground-text-strictly-for-clickable-links) | Borderline blue-text sites | Strict zero-exception rule: neutral `NUM` pill, neutral `3 sources`, clickable `ui_kit::link_button` for `Linked hook: …` and `Switch to CLD` |
| 8 | [`R20.15`](#14-r2015-eliminate-trailing-ellipses-on-action-buttons-and-unify-ellipsis-semantics) | Ellipsis scope & placeholders | **Option B + clean placeholders**: drop `…`/`...` from all buttons, all top-bar menus, and all `15` input placeholders (reserve `…` strictly for truncation and active progress) |
| 9 | [`R20.17`](#16-r2017-replace-in-app-feedback-modal-with-direct-github-issue-chooser-redirect) | Feedback URL & module cleanup | Redirect to `/issues/new/choose` (leave `roadmap_task.yml` untouched); fold ~35 lines into `os_integration.rs` and delete `feedback.rs` |
| 10 | [`R20.18`](#17-r2018-fix-brand-mark-sizing--mipmap-aliasing-and-make-logo-the-home-button) | Brand asset & Logo-as-Home chrome | Pure clickable logo button (`toolbar.home`, no `"Home"` text, localized tooltip/a11y) + premultiplied-alpha CPU-downsampled trimmed high-res PNG |
| 11 | [`R20.20`](#19-r2020-fix-desktop-gui-audit-p2-split-focus-cycling-per-sheet-result-tabs-context-menu-copy--first-run-fit-cancel) | Results right-click outside selection | Move selection to the right-clicked cell immediately when clicking outside selection (preserve range when clicking inside) |
| 12 | [`R20.20`](#19-r2020-fix-desktop-gui-audit-p2-split-focus-cycling-per-sheet-result-tabs-context-menu-copy--first-run-fit-cancel) | First-run nonlinear fit `Cancel` | Defer first-run commit until **Parameters › OK** is clicked (`Cancel` creates nothing; `OK` commits once with chosen equation/settings) |
| 13 | [`R20.21`](#20-r2021-structured-hover-tooltips-for-commands-scientific-readouts-navigator--inspector) | Tooltips & Graph Canvas hover | Include floating `(X, Y, series, row)` Graph Canvas point hover via existing `hit_test_point_indexed`; `450 ms` delay; GUI renders `summary` only while MCP/docs keep `beginner` |
| 14 | [`R20.22`](#21-r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) | `About CurveBook` interaction | Lightweight dismissible panel (`Esc`, outside click, or top-right `×`; no `OK` button; keeps `Copy version info` & `Send Feedback`) |
| 15 | [`R20.22`](#21-r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) | `"Settings"` vs. `"Preferences"` | Standardize on dot-free `"Settings"` across all platforms and shipped locales (keep command ID `app.preferences` unchanged) |
| 16 | [`R20.22`](#21-r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) | Shortcuts & cross-platform edges | Follow 4-layer public shortcut stack in `docs/design-language.md`; split `⌘T` into `Cmd+Shift+T` (`sheet.createData`) & `Cmd+Shift+P` (`ui.editParams.open`); bind `Cmd+,` (`app.preferences`), `Cmd+B` (`ui.navigator.toggle`), `Cmd+Shift+L` (`ui.family.toggle`), `Cmd+=`/`Cmd+-` zoom, `Cmd+W` close, & `Ctrl+Y` non-macOS Redo; dedup `About`/`Settings`/`Quit` on macOS; unified trailing modal footers |

---

## Adding New Items to This Push

When adding a new GUI or scientific-reporting item:
1. Register the claimable `R<m>.<n>` item (with `Gate: post-Beta`, explicit `Depends on:`, and concrete `- [ ]` sub-tasks) in [`docs/ROADMAP.md`](../ROADMAP.md) and update its summary tables.
2. Add a row to the [Tracked Items Index](#tracked-items-index), place it into the [Execution Waves & File-Ownership Map](#execution-waves--file-ownership-map), and add a numbered section following the template: **Current Problem & Root Causes**, **Target Design & Interaction Contract**, **Rejected Alternatives / Open Decisions** (when relevant), and **Code Touchpoints & Verification**.
3. Cross-link the new section from `docs/ROADMAP.md`, follow `R20.14` (blue text = links only) and `R20.15` (no trailing `…` on buttons), and run `xtask status`, `pending`, `cleanroom`, `brands`, and `secrets`.
