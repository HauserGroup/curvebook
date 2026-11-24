# CurveBook Design Language

CurveBook is a scientific data, statistics, and graphing workstation whose interface stays quiet so tables, equations, and figures carry the visual weight. Its design system pairs perceptual colour science and accessibility standards with flat, surface-first desktop hierarchy inspired by native desktop platform conventions.

Every visual token and standard control primitive lives in one canonical theme and widget boundary—[`crates/ui-egui/src/theme.rs`](../crates/ui-egui/src/theme.rs) (`ThemeSeeds`, `UiThemeSpec`) and [`crates/ui-egui/src/egui_shell/`](../crates/ui-egui/src/egui_shell/mod.rs) (`mod.rs`, `ui_kit.rs`, `widgets.rs`)—applied uniformly across every pane, inspector, and dialog and policed by `cargo xtask design-tokens`.

Rules below distinguish **`[Implemented]`** behaviour already verified in `crates/ui-egui` from **`[Target]`** refinements scheduled in [`R20.2`–`R20.4`](ROADMAP.md#r20-desktop-gui-visual-system--layout-overhaul). For shell regions, responsive breakpoints, and split-view interaction contracts, see the [GUI layout specification](planning/gui-layout-spec.md).

---

## 1. Opaque Surface Tiers & OKLab / OKLCH Colour System

Interface colours are derived deterministically from perceptual seeds in `ThemeSeeds` (`crates/ui-egui/src/theme.rs`) rather than hand-tuned RGB triples:

- **`[Implemented]` Neutral slate ramp:** stepped evenly in OKLab lightness (`curvebook_palette::oklab_even_steps`) with near-zero chroma so chrome, panels, recessed tracks, content planes, dividers, and text share a single neutral axis across `Light`, `Dark`, and `HighContrast`.
- **`[Implemented]` Single restrained accent:** one moderate-chroma slate-indigo seed stepped in cylindrical OKLCH with chroma-reduction gamut mapping (`Oklch::to_rgba8_gamut_mapped`) so hover, active, focus, and primary-button states keep consistent hue without sRGB clipping.
- **`[Implemented]` Semantic status & provenance seeds:** dedicated lightness-calibrated seeds for `error`, `warning`, `stale`, live-link provenance, and variable-role badges (`NUM`, `CAT`, `LBL`, `FX=`).
- **`[Target]` Three opaque neutral surface tiers (surfaces replace borders):** instead of outlining every button, chip, and group in `1 pt` grey boxes, hierarchy comes from three opaque neutral background levels:
  1. **`surface_chrome` (`app_bar` / `panel_bg`):** top bars, leading and trailing docks, and the bottom status footer.
  2. **`surface_recessed`:** shared tracks behind segmented controls, inline filter/search wells, inset section wells, and the figure `canvas_mat`.
  3. **`surface_content` (`surface_bg`):** primary workspace sheets—the contiguous data grid, result tables, notes, and figure paper (`paper_white`).
  - *Dark Mode elevation:* higher surface tiers step **lighter** in OKLab lightness (never darker) so elevated controls, sheets, and popovers stay legible without heavy borders.
  - *HighContrast mode:* preserves explicit structural borders alongside surface steps so every interactive boundary remains unmistakably delineated.
- **`[Target]` Quiet `divider` vs. `input_border`:** split subtle structural hairlines (`divider`—pane seams, header baselines, contiguous grid lines) from higher-contrast `input_border` strokes reserved strictly for editable text fields, numeric inputs, and dropdown controls.
- **`[Target]` Emphasized vs. unemphasized selection:** active selection inside the focused pane uses the emphasized accent tint (`active_bg`); selection retained in an unfocused pane steps down to a neutral cool-grey fill (`selection_unemphasized_bg`) so only one region visually claims focus at a time.
- **`[Implemented]` / `[Target]` Contrast verification (WCAG 2.2 by WCAG2ICT analogy):** CurveBook applies WCAG 2.2 criteria to its desktop GUI by analogy following W3C WCAG2ICT guidance:
  - *Text contrast (SC 1.4.3 Level AA / SC 1.4.6 Level AAA):* unit tests verify `>= 4.5:1` for foreground-on-surface text in `Light` and `Dark`, and `>= 7.0:1` (`>= 18:1` primary text) in `HighContrast` (`text_disabled` is exempt per SC 1.4.3).
  - *Non-text contrast (SC 1.4.11 Level AA):* every visual cue needed to identify an interactive UI component and its states (rest, hover, pressed, selected, checked, focused, error) must achieve `>= 3.0:1` contrast against adjacent colours in `Light`, `Dark`, and `HighContrast`. A `1 pt` border alone does **not** establish conformance; `[Target]` `R20.2` verifies borderless state cues—segmented tracks, active tab underlines, checkbox ticks, switch thumbs, hover fills, and focus rings—explicitly per theme.

---

## 2. One Accent Colour — Colour Belongs to the Data

In a scientific workstation, chromatic saturation belongs to experimental curves, data groups, and statistical diagnostics—not to surrounding chrome:

1. **`[Implemented]` Neutral chrome:** toolbars, docks, inspectors, and dialogs remain predominantly neutral grey, keeping UI chrome visually separate from default **Okabe–Ito** categorical series and **Viridis** / **Cividis** / **Plasma** continuous colormaps in `crates/palette`.
2. **`[Target]` Strict single-accent budget:** the interface accent colour is limited exclusively to:
   - the single `Primary` action button on a surface,
   - active selection cues (the active row in the focused pane, active underline-tab rule, active checkbox/segment indicator, and the `2 pt` top accent bar on the focused split-pane header), and
   - the transient `2 pt` keyboard focus ring (`focus_ring`).
   - Accent colour is **not** used on static spreadsheet column headers (`X`, `Y1`), inline `+ New…` creation rows, passive pane labels, or permanent full-pane border cages around blank canvas or table space.
3. **`[Implemented]` Never colour alone:** error, warning, stale, excluded-cell, and broken-link states always combine colour with a distinct icon, stroke pattern, typography style, or text label.
4. **Decoupled figure styling:** exported chart palettes (`crates/palette`) and publication figure typography, inset placement, and axis autoranging (`crates/chart`, tracked independently in [`R20.5`](ROADMAP.md#r205-chart-fit-summary-readability-inset-layout--axis-autoranging)) remain decoupled from UI chrome tokens.

---

## 3. Small Fixed Scales (`Tokens` & `LayoutMetrics`)

All widget dimensions read from [`LayoutMetrics`](../crates/ui-egui/src/egui_shell/layout.rs) and [`Tokens`](../crates/ui-egui/src/egui_shell/tokens.rs) at `1.0x` logical points:

- **`[Target]` `4`-point spacing rhythm (gradual migration):** `4, 8, 12, 16, 24 pt` (`space.xs`, `space.md`, `space.lg`, `space.xl`, `space.xxl`), plus a `2 pt` (`space.xxs`) micro-inset inside tight badges or icon-label pairs. Inter-group spacing is always at least twice intra-group spacing so spatial grouping replaces nested boxes. Retiring the legacy `6 pt` (`space.sm`) dock/form step onto `4 pt` or `8 pt` is a **provisional target** to validate empirically across narrow docks, inspector forms, and dialogs at default and increased text scales (`ui_scale > 1.0`).
- **`[Target]` Four UI type sizes:** strictly four logical sizes across all interface chrome (retiring the legacy `12.5 pt` near-duplicate):
  - `11.5 pt` regular (`type.small`): secondary captions, status readouts, badges, hints.
  - `13 pt` regular (`type.body`) / `13 pt` bold (`type.section`): default control labels, menus, and bold section headings (`[Implemented]` bold system UI face with `14 pt` regular fallback when no bold face exists).
  - `13 pt` aligned numerals (`type.numeral`): right-aligned data-grid and result-table numbers.
  - `15 pt` regular (`type.title`): panel and dialog titles.
  - `18 pt` regular (`type.display`): welcome and empty-state headlines only.
- **`[Target]` Runtime digit-width alignment check (`R20.2`):** because `egui 0.37` does not enable OpenType `tnum` features, `R20.2` measures the advance widths of ASCII `'0'..='9'` on the resolved primary proportional system sans face at runtime (module location chosen in `R20.2`). When digits share uniform advance width, grid and result numerals render in the `13 pt` primary sans face; otherwise, aligned numeric columns select a uniform-digit system sans or cap-height-matched system monospace fallback without bundling font binaries.
- **`[Target]` Shared panel-header datum (`32 pt` provisional):** `Project Navigator`, Center Split pane headers, and `Trailing Studio` share one aligned top header height (**`32 pt` provisional target**, validated at default and increased `ui_scale`), uniform horizontal padding, and a single `1 pt` bottom `divider` rule across the full window body. Pane splitters paint as `1 pt` `divider` lines inside an `8 pt` pointer hit zone.
- **`[Implemented]` Corner radii:** `2 pt` (`radius_sm`: badges, cell highlights), `4 pt` (`radius_md`: buttons, inputs, rows, segments), and `6 pt` (`radius_lg`: menus, popovers, dialogs).
- **`[Implemented]` Stroke widths:** `1 pt` (`stroke_thin_pt`: structural dividers and input borders) and `2 pt` (`stroke_focus_pt`: keyboard focus ring and active tab/header indicator).

---

## 4. Standard Control Kit, Vector Iconography & Writing Rules (`[Target]`)

Every interactive role maps to one recognizable visual control type instead of sharing identical outlined rectangles:

| Semantic Role | Visual Treatment | Usage Rules |
|---|---|---|
| **Primary Action** | Accent fill (`radius_md`), contrasting text | At most **one** per panel, toolbar, or dialog (`ActionRank::Primary`). |
| **Ghost Action** | Borderless at rest on `surface_chrome`; `hover_bg` fill on hover | Default for toolbar, pane-header, and footer actions; keeps chrome light. |
| **Secondary Button** | Subtle `surface_recessed` fill (quiet `1 pt` stroke in `HighContrast` or isolated dialog footers) | Cancel / Apply / isolated dialog actions that need clear button bounds. |
| **Segmented Control** | Shared `surface_recessed` track (`radius_md`) with equal-width segments and elevated `surface_content` active indicator | Mutually exclusive short option sets (e.g., `Symbol Shape`, axis scale). Never mix icons with text in one control or mix push actions with state selection. |
| **Underline Panel Tabs** | Borderless full-word labels on the panel/inspector header with a `2 pt` bottom accent indicator on the active tab | Inspector sections (`Data · Axes · Legend · Annotations · Canvas`) and multi-tab result views. Never abbreviate tabs to clipped fragments (`Leg`, `Annot`, `Can…`). |
| **Checkboxes, Switches & Radios** | Leading-aligned vector checkbox/switch/radio beside an affirmative sentence-case label | Used inside inspector and dialog bodies for boolean/exclusive settings; **never** placed in top toolbars. Replaces mixed `"Show X: On"` / `"Hide Y"` toggle buttons. |
| **Text, Search & Numeric Inputs** | `surface_content` fill with `1 pt` `input_border` (`radius_md`); `2 pt` `focus_ring` when focused | The primary resting-bordered widget family so editable inputs stand out immediately from actions. |
| **Contiguous Data Grid** | Single `surface_content` plane divided by quiet `1 pt` `divider` hairlines; neutral `header_bg` column headers; subtle alternating row fills | Uniform `X` and `Y` cells with zero individually rounded resting cell boxes; identical renderer in single-sheet and split-comparison panes. |

### Vector Iconography & Writing Conventions

- **Crisp vector disclosures & sheet icons:** paint disclosure triangles (`1.5 pt` stroke, pointing inward when collapsed and downward when expanded), sidebar collapse/expand glyphs, and frameless semantic sheet-kind icons directly from CurveBook's vector icon set ([`ASSETS.md`](../ASSETS.md)). Never use dot-like Unicode bullets for expand/collapse controls or checkbox-like framed checkmarks/slashes for sheet categories.
- **Unambiguous state markers:** reserve the bullet glyph (`•`) exclusively for unsaved document dirty state (never for active toolbar toggles such as `Split`).
- **Clear micro-copy & stable IDs:** write boolean checkboxes as affirmative noun phrases (`Data points`, `Fitted curve overlay`), put ellipses (`…`) only at the trailing end of labels that open a modal dialog (`Format Graph…`, `Apply Style From…`), and decouple concise visible labels (`Gallery`, `Annotations`) from longer Command Palette descriptions (`Toggle Portfolio Gallery`) without renaming stable `UI_COMMANDS` or `FrameInspectMeta` IDs.

---

## 5. Action Hierarchy, Single-Owner Navigation & Three-Way Split Targeting

Visual consistency must pair with clear structural ownership so the same sheets and controls do not compete for attention:

1. **`[Implemented]` Data-driven action rank (`ActionRank`):** every UI command and toolbar/dialog slot carries an explicit rank—`Primary`, `Secondary`, or `Overflow`—with **at most one `Primary` action** per surface.
2. **`[Implemented]` Priority-ordered toolbar overflow (`fit_ranked_toolbar`):** horizontal toolbars shed `Overflow` items into `"⋯"` first, then `Secondary` items, preserving left-to-right order and minimum hit dimensions (`min_toolbar_target_pt = 28 pt`, `min_hit_target_pt = 24 pt`) without arbitrarily clipping trailing buttons.
3. **`[Implemented]` Contextual visibility & control budgets (`RegionBudgets`):** toolbars and inspector panels show only actions valid for the active sheet archetype within explicit visible-control budgets.
4. **`[Target]` Single-owner sheet browsing:** `Project Navigator` is the sole primary sheet browser. Duplicate Tier 2 sheet tabs (`sheet.tab.*`) and Bottom Footer category jump pills are retired; `Family` remains dedicated to DAG lineage, split-pane headers control pane assignment, and the Footer provides a compact active-sheet switcher only when the Navigator is collapsed.
5. **`[Target]` Three-way split-view targeting:** Split Comparison separates persisted contextual command targeting (`doc.active_sheet_id`, updated via non-mutating `sheet.select` while preserving the Family anchor), materialized ephemeral pane assignment (`primary_sheet_id`, `secondary_sheet_id`), and ephemeral keyboard focus plus Inspector targeting (`inspector_target_sheet_id`, `inspector_pinned`, auto-pinned to the graph on the `Data + Graph` preset). Every Inspector and split-grid UI call site passes its explicit `sheet_id` in engine command parameters, while project-wide `Undo` / `Redo` remain document-scoped. See [`gui-layout-spec.md` §2.4](planning/gui-layout-spec.md#24-keyboard-focus-cycling--rtl-mirroring) for the full state-transition contract.

---

## 6. Repository Enforcement Gates

### Implemented Gates (`[Implemented]`)

- **Raw-value ratchet (`cargo xtask design-tokens`):** scans `crates/ui-egui/src` outside the canonical token definitions, icon/brand vectors, and tests (honouring explicit `// design-token-exempt: <reason>` annotations for scene/canvas geometry) and fails CI if raw colour or size literals exceed the tracked ceiling.
- **Specimen screenshots (`cargo xtask screenshots --check`):** renders the dev-only widget specimen view across `Light`, `Dark`, and `HighContrast` (`docs/images/specimen-{light,dark,high-contrast}.png`).
- **Frame geometry, non-overlap & target-size gates (`verify_egui_frame_parity` in [`driver.rs`](../crates/ui-egui/src/egui_shell/driver.rs) and the integration suite [`zz_overlap_probe.rs`](../crates/ui-egui/tests/ui_audit_suite/zz_overlap_probe.rs)):** verifies on settled AccessKit + `FrameInspectMeta` trees across narrow (`640 × 420 pt`), standard (`1280 × 800 pt`), and wide (`1440 × 600 pt`) windows, `200%` scale, and long-word locales that:
  - no two enabled clickable leaf controls overlap on the same `ZLayer`,
  - neighbouring standalone controls keep `>= 2 pt` separation and scrollbars never intersect clickable children,
  - every surface honours the single-primary-action rule and `RegionBudgets`, and
  - standalone interactive controls satisfy CurveBook's `wcag_258_target_size` gate (`24 × 24 pt` at `1.0x` `ui_scale`, mapping `1.0 pt` to `1 CSS px` by WCAG2ICT analogy).
- **WCAG 2.2 SC 2.5.8 (Level AA) conformance vs. test-gate exclusions:** CurveBook's automated `wcag_258_target_size` check excludes spreadsheet `gridcell`s, `Role::Splitter` sashes, and canvas corner/rotation grips from the standalone-button bounding-box assertion. Separately, WCAG 2.2 SC 2.5.8 defines five exact exceptions (**Spacing**, **Equivalent**, **Inline**, **User agent control**, **Essential**), where **Equivalent** requires another qualifying **pointer** target (`>= 24 × 24 CSS px`) on the same screen (keyboard alternatives alone do not qualify):
  - *Standalone controls:* directly meet `>= 24 × 24 pt` or satisfy **Spacing** via an unobstructed `24 pt`-diameter offset circle.
  - *`Role::Splitter` sashes (**pending verification**):* the intended basis is **Spacing**, which still has to be checked against the adjacent controls (grid cells, pane headers, canvas) on both sides of the `8 pt` hit zone. **Equivalent** applies only where a visible `>= 24 × 24 pt` pointer control sets the split ratio.
  - *Canvas corner and rotation grips (**pending verification**):* the intended basis is **Equivalent**, via the numeric bounds and rotation inputs in the Inspector and Format dialogs. It remains to verify that those inputs are `>= 24 × 24 pt` and produce the same resize and rotation results.
  - *Spreadsheet `gridcell`s:* default row height (`24 pt`) and column widths (`>= 48 pt`) directly meet `24 × 24 pt`; columns manually narrowed below `24 pt` by the user are documented as a known limitation rather than claiming **Essential**.

### Planned Gates (`[Target]`, `R20.2`–`R20.4`)

- **Theme non-text contrast suite (`R20.2`):** automated WCAG 2.2 SC 1.4.11 (Level AA, `>= 3:1`) assertions covering borderless state cues across `Light`, `Dark`, and `HighContrast`.
- **Shared header, targeting & unified split-grid parity (`R20.3`):** automated `panel_header_baseline_parity`, `split_targeting_parity`, and `split_unified_datagrid_parity` assertions across default, narrow (`640 × 420 pt`), and increased-`ui_scale` frames.
- **Five-stage `WorkflowRun` split-view acceptance proof (`R20.4`):** end-to-end human-path dose-response test from an empty `UiApp` through default `Data + Graph` auto-pinned split editing, live curve refit (`D1` oracle parity), and Inspector adjustment, followed by refreshed `cargo xtask screenshots`.

---

## References

- **Apple Inc.** *Human Interface Guidelines.* <https://developer.apple.com/design/human-interface-guidelines/>
- **W3C (2023).** *Web Content Accessibility Guidelines (WCAG) 2.2*, W3C Recommendation, 5 October 2023 — SC 1.4.3 Contrast (Minimum), SC 1.4.6 Contrast (Enhanced), SC 1.4.11 Non-text Contrast (Level AA), SC 2.4.7 Focus Visible, SC 2.4.11 Focus Not Obscured (Minimum), and SC 2.5.8 Target Size (Minimum, Level AA); applied to desktop software by analogy per *Guidance on Applying WCAG 2.2 to Non-Web Information and Communications Technologies (WCAG2ICT)*. <https://www.w3.org/TR/WCAG22/> · <https://www.w3.org/TR/wcag2ict-22/>
- **Okabe, M. & Ito, K. (2008).** *Color Universal Design (CUD): How to make figures and presentations that are friendly to colorblind people.* Jikei Medical School.
- **Ottosson, B. (2020).** *A perceptual color space for image processing* (OKLab and OKLCH). <https://bottosson.github.io/posts/oklab/>
- **Crameri, F., Shephard, G. E. & Heron, P. J. (2020).** The misuse of colour in science communication. *Nature Communications* **11**, 5444. <https://doi.org/10.1038/s41467-020-19160-7>
- **Tol, P. (2021).** *Colour Schemes*, SRON Technical Note SRON/EPS/TN/09-002, Issue 3.2.
