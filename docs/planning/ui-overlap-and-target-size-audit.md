# Overlap, Target-Size & Clutter Audit

> **Status:** Findings and proposed fixes. Nothing here is implemented yet. · **Measured:** 2026-11-22 on branch `audit/full-remediation-2026-11-22` (after `f860679`) · **Related:** [GUI layout specification](gui-layout-spec.md) §8.10, §9.1, §9.4 and §10

The counts below are a dated snapshot. They will drift as the UI changes, so rerun the measurement (see [How it was measured](#how-it-was-measured)) before relying on them.

## Summary

| Question | Answer |
|---|---|
| Do clickable controls overlap? | **One real case:** the result viewer's "Next tab" arrow sits on top of the first result tab, in every window size and locale. No other overlaps in the main views, 26 dialogs or 8 menus. |
| Are controls big enough? | Most controls are **22 pt** tall (some 20 pt), below the spec's 24 pt minimum (§9.1). Under the public accessibility rule ([WCAG 2.2, success criterion 2.5.8](https://www.w3.org/TR/WCAG22/#target-size-minimum)), which allows smaller targets with enough space around them, only **7 controls fail**. |
| Is the interface cluttered? | Most workspace views show **85–108 clickable controls at once** at normal window sizes. Nothing ranks them, so nothing decides what to hide first. |
| Should the result-tab arrows go? | **Yes.** Replace them with an overflow menu that appears only when tabs don't fit. Keyboard switching already exists. The layout bug behind the overlap must be fixed as well, or the overflow button will land in the same wrong place (see [F1](#f1-remove-the-result-tab-arrows)). |

## How it was measured

A temporary probe test (`crates/ui-egui/tests/zz_overlap_probe.rs`, ignored by default and not committed) rendered the desktop shell with `EguiHumanPathDriver`. It read the position, size, layer and enabled state that every widget records in `FrameInspectMeta` each frame.

**Coverage:**

- **Workspace views:** start screen, XY table, grouped table, column table, notes (info) sheet, result sheet, graph and layout. Each at 800×600, 1024×700, 1280×800, 1440×820 and 1920×1080 pt in English, plus 1024×700 in German. 48 renders.
- **Dialogs:** 26 dialogs opened through their UI commands, at 800×600 and 1280×800 in English and 1024×700 in German.
- **Menus:** all 8 menu-bar menus, at the same three sizes.

**Rules applied:**

- **Clickable controls:** roles `button`, `textbox`, `radio`, `tab`, `menuitem`, `checkbox`, `treeitem`, `option`, `spinbutton`, `menu`, `gridcell` and `splitter`. Grid cells and splitters are left out of the size checks.
- **Partial overlap:** two enabled controls on the same layer share more than 1×1 pt and neither contains the other.
- **Undersized:** narrower or shorter than 24 pt.
- **WCAG 2.5.8 fail:** an undersized, enabled control where a circle 24 pt across, centred on it, touches another enabled control on the same layer or the circle of another undersized control.

**Not covered:**

- large projects with many sheets
- right-click menus, tooltips and the inspector's tabs beyond their default state
- the four layout dialogs that need an embedded table or image (`ui.embeddedTable.format.open`, `ui.image.format.open`, `ui.result.inspectDiagnostics`, `ui.clonePipeline.open`)
- visual crowding between controls that don't touch: the probe compares rectangles, not what a person perceives

## Findings

### 1. The result-tab arrow overlaps the first tab

In the result viewer header (`crates/ui-egui/src/egui_shell/results_view.rs`), measured at 1440×820:

| Widget | Left edge (pt) | Right edge (pt) |
|---|---|---|
| `result.tabs.prev` | 447.9 | 471.9 |
| `result.tab.summary` | 475.9 | 568.5 |
| `result.tabs.next` | 479.9 | 503.9 |

The "Next" arrow starts 4 pt after the tab strip begins, which equals the header's `item_spacing.x`. The likely cause is how the strip is laid out:

- The code reserves the strip's width with `ui.allocate_space`, then draws the tabs inside `ui.scope_builder(...max_rect(strip_rect)...)` using `ui.interact`, which takes up no layout space.
- When the scope closes, egui moves the parent's cursor to just after the scope's *used* area. That area is empty and sits at the strip's left edge.
- So the cursor jumps back to the start of the strip, and the next widget, the "Next" arrow, is placed there, on top of the first tab.

The arrow is drawn after the tab, so it covers the tab's leading text. With one tab (common) both arrows are disabled but still visible.

### 2. Dialogs and menus have no real overlaps

The first pass flagged overlaps in dialogs opened from the start screen (Command Palette, Import, Power, Randomize, Simulate, About, Feedback), and nesting in every menu. Both turned out to be measurement artifacts:

- **Dialogs:** the start screen is recorded on the `ModalDialog` layer, and its controls stay recorded underneath a dialog opened on top of it. They are recorded as **disabled**. A click test at the overlap point between `welcome.stepper.rows.dec` and About's "Send Feedback" button reached the About button (the dialog switched to Feedback), not the start screen. There is no click-through.
- **Menus:** each open menu records its popup container as a `menu` role covering all its items. That's a container, not a competing target.

A permanent gate needs to ignore both cases (see [F4](#f4-turn-the-probe-into-a-gate)).

### 3. Most controls are 22 pt; only 7 fail the WCAG rule

The spec's minimum is 24×24 pt (§9.1, `LayoutMetrics::min_hit_target_pt`), and `Tokens` applies it as `style.spacing.interact_size.y`. Custom widgets and fixed-height strips still render smaller:

- **20 pt tall:** status footer buttons (history back/forward, section jumps, preview filter, zoom); navigator filter chips and collapse buttons; grid header bands (sheet title, super-column titles, row-titles toggle).
- **22 pt tall:** menu-bar titles; result tabs; the inspector's decimal-place buttons; most dialog controls (Analyze, Edit Parameters, Power, Randomize, Simulate, Random Sample).

Across the 48 workspace renders, 30–62 controls per view are under 24 pt. The graph and layout views have the most, because of their zoom and ruler controls.

The WCAG 2.2 rule (2.5.8, level AA) allows a target under 24×24 CSS px if it has enough space around it. Applied in logical points, only these fail:

| View | Control | Size (pt) | Too close to |
|---|---|---|---|
| Grouped table | `grid.col_super.0` (super-column title) | 192×20 | `grid.col_title.0` directly below |
| Grouped table | `grid.col_super.1` | 192×20 | `grid.col_title.1` directly below |
| Grouped table | `grid.sheet_title` | 112×20 | `grid.row_titles.band` |
| Grouped table | `grid.row_titles.toggle` | 112×20 | `grid.row_titles.band` |
| XY table | `grid.row_titles.toggle` | 112×20 | `grid.row_title.0` |
| Analyze dialog | `analyze.entry.stats.glm` | 276×14 | neighbouring list entry |
| Command palette | `palette.item.9` | 520×13 | neighbouring list entry |

The grid failures are real. The header bands are 20 pt (`grid_header_tier_heights = [20, 24, 20]`) and sit directly on top of the 24 pt title row with no gap. The two list entries at 13–14 pt are most likely rows cut off at the bottom of a scrolling list rather than undersized controls, but this hasn't been confirmed.

### 4. Clutter: many controls, no ranking

| Surface | Clickable controls visible |
|---|---|
| Workspace views, 1024 pt wide and up | 76–108 |
| Workspace views at 800×600 | 62–85 |
| Largest dialogs (Simulate, Power, Randomize, Import, Feedback) | 37–49 |

Only the right-hand group of the top bar shrinks by priority (`crates/ui-egui/src/egui_shell/top_bar_fit.rs`). The spec describes the same idea for the action strip (`⋯` overflow, §9.1), but no surface records which controls matter most. So nothing can enforce "one main action per panel" or decide what moves into an overflow menu first.

## Fixes

Listed in suggested order. Each would be its own roadmap item and PR, following AGENTS.md.

### F1. Remove the result-tab arrows

1. **Fix the cursor first.** After drawing the tabs, make the parent layout advance past the full strip. For example, call `ui.expand_to_include_rect(strip_rect)` inside the scope, or allocate the strip's rect with `ui.allocate_rect`, and paint into it without a nested `scope_builder`. Without this, any control placed after the strip lands on the first tab.
2. **Delete `result.tabs.prev` and `result.tabs.next`,** and the `results.prev_tabs` / `results.next_tabs` translation keys in `crates/ui-egui/src/i18n/w5_results.rs`.
3. **Add an overflow button, only when tabs don't fit.** A `⋯ +N` button at the end of the strip opens a dropdown listing the hidden tabs, using the same `⋯` overflow pattern the spec already defines for the action strip. With all tabs visible, which is the usual case, the button isn't shown. Mouse-wheel scrolling over the strip can stay.
4. **Keep keyboard switching.** `Ctrl+PageUp` / `Ctrl+PageDown` already switch tabs (`results_view.rs`).
5. **Update the tests that click the arrows:**
   - `crates/ui-egui/tests/human_wf_13_18.rs` (`h.click("result.tabs.next")`)
   - `crates/ui-egui/tests/r15_3_stage_canvas.rs` (two places)

   Click the target tab directly, or open the overflow menu.
6. **Update the spec.** Change [GUI layout specification](gui-layout-spec.md) §8.10 ("`<` `>` overflow steppers") and its sketch to describe the overflow menu.

**Result:** one overlap and two buttons fewer on every result screen. The overflow control appears only when it's needed.

### F2. Make grid header bands meet the target-size rule

Pick one:

- **Raise the two 20 pt header bands to 24 pt** (`grid_header_tier_heights = [24, 24, 24]`). This costs 8 pt of vertical space per table.
- **Or keep 20 pt bands with a 2 pt gap** between bands. A 20 pt band then has 12 pt from its centre to the next control, which is what the WCAG spacing rule needs.

Update §9.1's `grid.row_h` / `header_h` row to match.

### F3. Confirm the two short list rows

Check whether `analyze.entry.stats.glm` and `palette.item.9` are rows cut off at a scroll edge. If they are, the gate in F4 should skip controls cut off by a scroll area. If they really are 13–14 pt tall, give list rows the standard 24 pt height.

### F4. Turn the probe into a gate

Add two checks to `verify_egui_frame_parity` in `crates/ui-egui/src/egui_shell/driver.rs`, next to the existing scrollbar and text-bleed checks:

1. **No partial overlap.** Two enabled clickable controls on the same layer must never partly overlap. Nesting is allowed.
2. **Target size per WCAG 2.5.8,** including its spacing exception.

Both checks skip disabled controls and container roles such as menu popups. Run them over the probe's view × window size × locale matrix, all dialogs reachable through UI commands, and the menu bar. Record the gate in spec §9.4.

Also decide whether §9.1 keeps its stricter "24 × 24 pt always" wording or adopts the WCAG wording. If it keeps the stricter rule, most controls need to grow to 24 pt. Many already pass the WCAG rule as they are.

### F5. Rank controls and give each region a budget

This addresses clutter and hierarchy, not just geometry:

1. **Give every control a rank:** `primary`, `secondary` or `overflow`, stored as data next to the command it triggers (for example a field on `UiCommandSpec`, regenerated with `cargo xtask ui-commands`).
2. **Make each toolbar and strip shrink by rank,** the way `fit_right_group` already does for the top bar. Lower-ranked controls move into a `⋯` menu and stay reachable from the command palette. Controls never overlap or shrink below the target size.
3. **Add checks:** at most one `primary` control per panel, and a maximum number of visible controls per region (top chrome, navigator, centre, inspector, footer). Start the budget at today's measured counts so it can only go down.

Before setting budgets, extend the probe to report counts per region, not just per view.
