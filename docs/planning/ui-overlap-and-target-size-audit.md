# Overlap, Target-Size & Clutter Audit

> **Status:** Implemented (`F1`–`F5`) in `R15.10`. · **Original baseline measured:** 2026-11-22 on branch `audit/full-remediation-2026-11-22` (after `f860679`) · **Related:** [GUI layout specification](gui-layout-spec.md) §8.10, §9.1, §9.4 and §10, [Design language](../design-language.md)

The baseline counts below record the pre-`R15.10` snapshot that motivated `F1`–`F5`. Regression enforcement now runs permanently in `verify_egui_frame_parity` (`crates/ui-egui/src/egui_shell/driver.rs`) and `crates/ui-egui/tests/zz_overlap_probe.rs`.

## Summary

| Question | Baseline answer (pre-`R15.10`) | Resolution in `R15.10` |
|---|---|---|
| Do clickable controls overlap? | **One real case:** the result viewer's "Next tab" arrow sat on top of the first result tab in every window size and locale. No other overlaps in the main views, 26 dialogs or 8 menus. | **Fixed (`F1` + `F4`).** Replaced nested `scope_builder` cursor rewind with `allocate_exact_size`, removed `result.tabs.prev` / `result.tabs.next`, added conditional `⋯ +N` `result.tabs.overflow`, and gated `interactive_partial_overlap` in `verify_egui_frame_parity`. |
| Are controls big enough? | Most controls were **22 pt** tall (some 20 pt), below §9.1's 24 pt minimum. Under [WCAG 2.2, success criterion 2.5.8](https://www.w3.org/TR/WCAG22/#target-size-minimum), only **7 controls failed** (5 grid header bands + 2 scroll-clipped list rows). | **Fixed (`F2` + `F3` + `F4`).** Raised `grid_header_tier_heights` to `[24, 24, 24]`, distinguished scroll-edge clipping via `unclipped_rect`, raised modal close & choice rows to 24 pt, and gated `wcag_258_target_size` in `verify_egui_frame_parity`. |
| Is the interface cluttered? | Most workspace views showed **85–108 clickable controls at once** at normal window sizes, with no action ranking to decide what hides first. | **Fixed (`F5`).** Added `ActionRank` (`Primary`, `Secondary`, `Overflow`) to `UiCommandSpec`, `ToolbarButtonSpec`, and `FrameInspectNodeMeta`; wired `fit_ranked_toolbar` into Tier 2 (`⋯` overflow); gated `single_primary_action` and `RegionBudgets`. |
| Should the result-tab arrows go? | **Yes.** Replace them with an overflow menu that appears only when tabs don't fit, while keeping keyboard and wheel switching. | **Done (`F1`).** `result.tabs.overflow` appears only when `visible_count < tabs.len()`. |

## How it was measured

Originally measured with a probe test (`crates/ui-egui/tests/zz_overlap_probe.rs`, now committed as a permanent regression suite) rendering the desktop shell with `EguiHumanPathDriver`. It reads the position, size, unclipped bounds, rank, layer and enabled state that every widget records in `FrameInspectMeta` each frame.

**Coverage:**

- **Workspace views:** start screen, XY table, grouped table, column table, multiple-variables table, notes (info) sheet, result sheet (single/few tabs and many-tab overflow), graph and layout across narrow (`640×420`), `800×600`, `1024×700`, `1280×800`, `1440×820` and `1920×1080` pt in English, `640×420` and `1024×700` in German (`de`), and `200%` zoom (`ui_scale = 2.0`).
- **Dialogs:** 26 dialogs opened through their UI commands, at 800×600 and 1280×800 in English and 1024×700 in German.
- **Menus:** all 8 menu-bar menus, at the same three sizes.

**Rules applied:**

- **Clickable controls:** roles `button`, `textbox`, `radio`, `tab`, `menuitem`, `checkbox`, `treeitem`, `option`, `spinbutton`, `menu`, `gridcell` and `splitter`. Grid cells and splitters are left out of the size checks.
- **Partial overlap:** two enabled controls on the same layer share more than 1×1 pt and neither contains the other.
- **Undersized:** narrower or shorter than 24 pt.
- **WCAG 2.5.8 fail:** an undersized, enabled control (not merely clipped at a scroll-area edge) where a circle 24 pt across, centred on it, touches another enabled control on the same layer or the circle of another undersized control.

**Remaining scope boundaries:**

- large projects with hundreds of sheets beyond the multi-tab overflow fixture
- right-click context menus, hover tooltips and non-default inspector sub-tabs
- the four layout dialogs that need a selected embedded table or image (`ui.embeddedTable.format.open`, `ui.image.format.open`, `ui.result.inspectDiagnostics`, `ui.clonePipeline.open`)
- subjective visual crowding between controls that satisfy both non-overlap and region budgets

## Baseline Findings (Pre-`R15.10`)

### 1. The result-tab arrow overlapped the first tab

In the result viewer header (`crates/ui-egui/src/egui_shell/results_view.rs`), measured at 1440×820 before `F1`:

| Widget | Left edge (pt) | Right edge (pt) |
|---|---|---|
| `result.tabs.prev` | 447.9 | 471.9 |
| `result.tab.summary` | 475.9 | 568.5 |
| `result.tabs.next` | 479.9 | 503.9 |

The "Next" arrow started 4 pt after the tab strip began, which equalled the header's `item_spacing.x`:

- The code reserved the strip's width with `ui.allocate_space`, then drew the tabs inside `ui.scope_builder(...max_rect(strip_rect)...)` using `ui.interact`, which took up no layout space.
- When the scope closed, egui moved the parent's cursor to just after the scope's *used* area. That area was empty and sat at the strip's left edge.
- So the cursor jumped back to the start of the strip, and the next widget, the "Next" arrow, was placed there, on top of the first tab.

### 2. Dialogs and menus had no real overlaps

The first pass flagged overlaps in dialogs opened from the start screen (Command Palette, Import, Power, Randomize, Simulate, About, Feedback), and nesting in every menu. Both turned out to be measurement artifacts:

- **Dialogs:** the start screen is recorded on the `ModalDialog` layer, and its controls stay recorded underneath a dialog opened on top of it as **disabled**.
- **Menus:** each open menu records its popup container as a `menu` role covering all its items. That's a container, not a competing target.

Both cases are excluded by the permanent gate in `driver.rs` (`F4`).

### 3. Baseline target-size snapshot: 7 WCAG 2.5.8 failures

Before `F2` and `F3`, applying WCAG 2.2 SC 2.5.8 in logical points flagged only these 7 controls:

| View | Control | Size (pt) | Cause / Resolution |
|---|---|---|---|
| Grouped table | `grid.col_super.0` (super-column title) | 192×20 | Raised header bands to `[24, 24, 24]` (`F2`) |
| Grouped table | `grid.col_super.1` | 192×20 | Raised header bands to `[24, 24, 24]` (`F2`) |
| Grouped table | `grid.sheet_title` | 112×20 | Raised header bands to `[24, 24, 24]` (`F2`) |
| Grouped table | `grid.row_titles.toggle` | 112×20 | Raised header bands to `[24, 24, 24]` (`F2`) |
| XY table | `grid.row_titles.toggle` | 112×20 | Raised header bands to `[24, 24, 24]` (`F2`) |
| Analyze dialog | `analyze.entry.stats.glm` | 276×14 | Scroll-area bottom edge clip; `unclipped_rect` is full row height (`F3`) |
| Command palette | `palette.item.9` | 520×13 | Scroll-area bottom edge clip; `unclipped_rect` is full row height (`F3`) |

### 4. Clutter: many controls, no ranking

| Surface | Baseline clickable controls visible |
|---|---|
| Workspace views, 1024 pt wide and up | 76–108 |
| Workspace views at 800×600 | 62–85 |
| Largest dialogs (Simulate, Power, Randomize, Import, Feedback) | 37–49 |

## Implemented Fixes (`F1`–`F5`)

### F1. Remove the result-tab arrows

1. **Fixed cursor advancement:** Allocated the tab strip directly via `ui.allocate_exact_size(vec2(strip_w, strip_h), Sense::hover())` without a nested `scope_builder`.
2. **Deleted `result.tabs.prev` and `result.tabs.next`,** along with `results.prev_tabs` and `results.next_tabs` in `crates/ui-egui/src/i18n/w5_results.rs`.
3. **Added conditional `⋯ +N` overflow (`result.tabs.overflow`)** shown only when `visible_count < tabs.len()`, opening a popup menu (`result.tabs.overflow.item.<id>`) while keeping mouse-wheel scrolling.
4. **Kept keyboard switching** (`Ctrl+PageUp` / `Ctrl+PageDown`).
5. **Updated tests** (`crates/ui-egui/tests/human_wf_13_18.rs` and `crates/ui-egui/tests/r15_3_stage_canvas.rs`) and [GUI layout specification](gui-layout-spec.md) §8.10.

### F2. Make grid header bands meet the target-size rule

Raised `LayoutMetrics::grid_header_tier_heights` from `[20.0, 24.0, 20.0]` to `[24.0, 24.0, 24.0]` (`72.0 pt` total) and updated [GUI layout specification](gui-layout-spec.md) §9.1.

### F3. Distinguish scroll-edge-clipped list rows

Added `unclipped_rect: Rect` onto `FrameInspectNodeMeta` (`response.rect` before parent `clip_rect` intersection). Controls whose `unclipped_rect` already meets `24 × 24 pt` and are clipped solely by a `ScrollArea` boundary are recognized as scroll-clipped rather than undersized authoring defects.

### F4. Turn the probe into a gate

Added `check_clickable_partial_non_overlap` (`interactive_partial_overlap`) and `check_wcag_258_target_size` (`wcag_258_target_size`) to `verify_egui_frame_parity` in `crates/ui-egui/src/egui_shell/driver.rs`, documented in [GUI layout specification](gui-layout-spec.md) §9.4, and committed `crates/ui-egui/tests/zz_overlap_probe.rs` across the workspace, dialog, menu, zoom, and locale matrix.

### F5. Rank controls and give each region a budget

1. **Control rank stored as data:** Added `ActionRank` (`Primary`, `Secondary`, `Overflow`) to `UiCommandSpec` (synced into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands`), `ToolbarButtonSpec`, and `FrameInspectNodeMeta::action_rank`.
2. **Rank-shrinking toolbar:** Implemented `fit_ranked_toolbar` in `crates/ui-egui/src/egui_shell/top_bar_fit.rs` and wired Tier 2 (`top_chrome.rs`) so `Overflow` and then `Secondary` actions collapse into `toolbar.overflow_more` (`⋯`) while keeping the surface's single `Primary` action visible and accented; omitted out-of-context disabled `toolbar.edit_params` on `DataStructured`.
3. **Structural hierarchy & clutter gates:** Added `check_single_primary_per_region` (`single_primary_action`, enforced in `verify_egui_frame_parity`) and `RegionBudgets` / `check_region_control_budgets` across `TopChrome`, `Navigator`, `Center`, `Inspector`, `Footer`, and `Modal`.
