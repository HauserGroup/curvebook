# CurveBook Design Language

CurveBook builds its visual language and interface hierarchy directly from perceptual colour science, accessibility standards, and structural layout invariants. Every rule below is implemented in `crates/palette` and `crates/ui-egui` and enforced automatically by `xtask` and headless frame-inspection gates.

For exact workspace region breakpoints and widget conventions, see the [GUI layout specification](planning/gui-layout-spec.md) (§9–§10).

---

## 1. Derive Interface Colours in OKLab and OKLCH

UI colours are generated deterministically from a small set of perceptual seeds (`ThemeSeeds` in `crates/ui-egui/src/theme.rs`) rather than hand-picked RGB triples:

- **Neutral slate ramp:** stepped evenly in OKLab lightness (`curvebook_palette::oklab_even_steps`) with near-zero chroma so panels, headers, inputs, dividers, borders, and text share one coherent neutral axis across `Light`, `Dark`, and `HighContrast`.
- **Single restrained accent:** one moderate-chroma slate-indigo seed stepped in cylindrical OKLCH with chroma-reduction gamut mapping (`Oklch::to_rgba8_gamut_mapped`) so hover, active, focus, and primary-button states preserve hue without sRGB channel clipping.
- **Semantic status & provenance seeds:** dedicated seeds for `error`, `warning`, `stale`, live links, and variable-role badges (`NUM`, `CAT`, `LBL`, `FX=`), each mapped to mode-specific lightness targets.
- **Automated contrast verification:** unit tests verify that every foreground-on-surface text pair meets WCAG 2.2 AA (`>= 4.5:1`; `>= 7.0:1` AAA in `HighContrast`, primary text `>= 18:1`), every interactive border, focus ring, and icon meets WCAG 2.2 non-text contrast (`>= 3.0:1`), and hover fills maintain measurable contrast separation against resting surfaces (`text_disabled` is intentionally exempt per WCAG 2.2 SC 1.4.3).

---

## 2. Colour Belongs to the Data

In a scientific graphing and statistics workstation, chromatic emphasis belongs to plotted data rather than application chrome:

1. **Neutral chrome:** toolbars, docks, inspectors, and dialogs stay predominantly neutral grey.
2. **Scoped accent:** the interface accent is used solely for the keyboard focus ring, active selection, and the single primary action button on a surface. Its restrained slate-indigo chroma and perceptual distance keep UI controls visually distinct from default **Okabe-Ito** categorical series and **Viridis** / **Cividis** / **Plasma** continuous colormaps, while figures sit on crisp `paper_white` inside a neutral `canvas_mat`.
3. **Never colour alone:** error, warning, stale, and broken-link states always pair colour with an icon, dashed stroke pattern, or explicit status label.

---

## 3. Small Fixed Scales (`Tokens` & `LayoutMetrics`)

All widget measurements read from [`LayoutMetrics`](../crates/ui-egui/src/egui_shell/mod.rs) and [`Tokens`](../crates/ui-egui/src/egui_shell/mod.rs) at `1.0x` logical points:

- **Spacing:** 4-point grid (`4, 8, 12, 16, 24 pt`), supplemented by `2 pt` inside compact controls/badges and `6 pt` form/dock rhythm (`gui-layout-spec.md` §10.1). Group separation is at least twice intra-group spacing.
- **Typography:** five UI text roles (`11.5 pt` small, `13 pt` body, `13 pt` bold section heading, `15 pt` panel/dialog title, `18 pt` welcome/empty headline) plus `12.5 pt` tabular numerals (`font_grid_numeral_pt`) for aligned data and result columns.
- **Corner radii:** three steps—`2 pt` (`radius_sm`: chips, badges, cell highlights), `4 pt` (`radius_md`: buttons, inputs, rows), and `6 pt` (`radius_lg`: menus, popovers, dialogs).
- **Stroke widths:** two weights—`1 pt` (`stroke_thin_pt`: structural borders and dividers) and `2 pt` (`stroke_focus_pt`: keyboard focus ring and active selection).

---

## 4. Structural Action Hierarchy & Anti-Clutter Rules

Visual tokens keep widgets consistent, while structural rules prevent clutter and overlapping controls:

1. **Data-driven action rank (`ActionRank`):** every UI command and toolbar/dialog slot carries an explicit rank—`Primary`, `Secondary`, or `Overflow`. Each panel or modal dialog exposes **at most one `Primary` action**.
2. **Priority-ordered toolbar overflow (`fit_ranked_toolbar`):** horizontal toolbars shrink deterministically by rank rather than truncating whatever sits on the right. When space runs short, `Overflow` items move first into the `"⋯"` menu, followed by `Secondary` items, keeping surviving controls in stable left-to-right order (and reachable via menus and the Command Palette) without shrinking below `min_toolbar_target_pt` (`28 pt`) or `min_hit_target_pt` (`24 pt`).
3. **Contextual visibility & control budgets:** toolbars and studio headers surface only actions valid for the active context (data table, multiple-variables sheet, result sheet, graph, info sheet, or page layout), bounded by explicit per-panel visible control budgets.

---

## 5. Repository Enforcement Gates

- **Raw-value ratchet (`cargo xtask design-tokens`):** scans `crates/ui-egui/src` outside the canonical token definitions, icon/brand vectors, and tests (honouring explicit `// design-token-exempt: <reason>` annotations for scene/canvas geometry) and fails CI if raw colour or size literals rise above the tracked ceiling.
- **Specimen screenshots (`cargo xtask screenshots --check`):** renders the dev-only widget specimen view—buttons, fields, dropdowns, toggles, badges, tables, priority toolbars, and modal chrome—in `Light`, `Dark`, and `HighContrast` (`docs/images/specimen-{light,dark,high-contrast}.png`).
- **Frame geometry & hierarchy gates (`verify_egui_frame_parity` & `zz_overlap_probe`):** verifies on settled AccessKit + `FrameInspectMeta` trees across narrow (`640 × 420 pt`), standard (`1280 × 800 pt`), and wide (`1440 × 600 pt`) windows, `200%` zoom, and long-word locales (`de`) that:
  - no two clickable leaf controls overlap on the same `ZLayer`,
  - standalone interactive controls meet `min_hit_target_pt` (`24 × 24 pt`),
  - neighbouring standalone controls maintain minimum separation (`>= 2 pt`), and
  - every surface respects the single-primary-action rule and per-panel control budget.

---

## References

- **W3C (2023).** *Web Content Accessibility Guidelines (WCAG) 2.2*, W3C Recommendation, 5 October 2023 — SC 1.4.3 Contrast (Minimum), SC 1.4.6 Contrast (Enhanced), SC 1.4.11 Non-text Contrast, SC 2.4.7 Focus Visible, SC 2.4.11 Focus Not Obscured (Minimum), and SC 2.5.8 Target Size (Minimum). <https://www.w3.org/TR/WCAG22/>
- **Okabe, M. & Ito, K. (2008).** *Color Universal Design (CUD): How to make figures and presentations that are friendly to colorblind people.* Jikei Medical School.
- **Ottosson, B. (2020).** *A perceptual color space for image processing* (OKLab and OKLCH). <https://bottosson.github.io/posts/oklab/>
- **Crameri, F., Shephard, G. E. & Heron, P. J. (2020).** The misuse of colour in science communication. *Nature Communications* **11**, 5444. <https://doi.org/10.1038/s41467-020-19160-7>
- **Tol, P. (2021).** *Colour Schemes*, SRON Technical Note SRON/EPS/TN/09-002, Issue 3.2.
