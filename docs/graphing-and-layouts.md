# Publication Graphing, Page Layouts & Export — Implementation Guidelines

Governing specification for `crates/palette` (`L0`), `crates/text` (`L2`), `crates/chart` (`L2`), `crates/render` (`L3`), `crates/svg` (`L3`), `crates/pdf` (`L3`), and canvas tools in `crates/tools` (`L4`). Parity checklist: [`graphing-parity.md`](graphing-parity.md) and [`file-format-parity.md`](file-format-parity.md).

Public behavioural references:
- [Graphs & graph families overview](https://www.graphpad.com/guides/prism/latest/user-guide/making_graphs.htm)
- [Axes, discontinuous segments & nonlinear scales](https://www.graphpad.com/guides/prism/latest/user-guide/axes.htm)
- [Pairwise comparisons on graphs (asterisks & P values)](https://www.graphpad.com/guides/prism/latest/user-guide/adding_pairwise_comparisons.htm)
- [Page layouts & publication figure export](https://www.graphpad.com/guides/prism/latest/user-guide/layouts.htm)
- [Prism 11.1 release notes (Multiple Variables categorical graphs, dashed error bars, level ordering)](https://www.graphpad.com/updates/prism-11-1-0-release-notes)

---

## 1. Scene-Graph Compilation Pipeline

`lumencraft-chart` is a pure, deterministic compiler:
$$\big(\text{GraphSpec},\, \text{DataSheet(s)},\, \text{ResultSheet(s)},\, \text{FontMetrics}\big) \longrightarrow \text{ChartScene}$$
Neither `lumencraft-ui-egui`, `lumencraft-svg`, nor `lumencraft-pdf` ever computes error bars, kernel density estimates, axis ticks, or curve polylines on its own. All output backends (software/interactive workspace compositor, CPU rasterizer `lumencraft-render`, SVG writer `lumencraft-svg`, and pure-Rust `%PDF-1.7` / `1.4` writer `lumencraft-pdf`) consume the exact same `ChartScene` primitives (positioned `Path2D`, `PositionedText`/`RichText`, clip rects, and `StrokePaint`/`FillPaint`), guaranteeing sub-point visual parity between screen and journal export.

---

## 2. Graph Families & Graphical Data Objects (GDOs)

Creating a new `DataSheet` automatically provisions a paired `GraphSheet` (`initialized: false`). On the user's first visit to an uninitialized `GraphSheet` (or via `Change › Graph Type…`), the visual family gallery presents live schematic thumbnails filtered to the upstream `TableArchetype` and `ColumnDesignHint`. Each `GraphSpec` binds one or more data series and configures composable Graphical Data Objects:

1. **XY Graphs:**
   - Scatter symbols (circle, square, triangle, inverted triangle, diamond, cross, plus, open/filled/semi-filled, per-point size/color overrides),
   - Connecting lines (point-to-point, staircase, Akima/monotonic cubic spline) and area-under-curve fills (to $Y = 0$, baseline constant, or between two curves),
   - Vertical and horizontal error bars (SD, SEM, $95\%$ CI, interquartile range, range, or explicit subcolumns; solid or **dashed/dotted line styles** per series; optional end caps; one-sided upper/lower or symmetric),
   - Smooth overlaid regression curves ($256$–$1024$ adaptive segments in axis space so log-X sigmoids stay smooth) + shaded or dashed confidence and prediction ribbons.
2. **Column & Grouped Graphs:**
   - Mean/median bar + error bar,
   - Individual replicate scatter overlay (deterministic symmetric beeswarm / compact-letter-free jitter so points never overlap randomly across redraws),
   - Aligned dot plot with mean/median + error crossbar,
   - **Box-and-whisker** (Tukey $1.5 \times \text{IQR}$ with outlier points, min-to-max, $10\text{–}90\%$, $5\text{–}95\%$, $2.5\text{–}97.5\%$, or $1\text{–}99\%$ whiskers),
   - **Violin plot** (Gaussian kernel density estimate clipped to observed `[min, max]` or extended, overlaid median/quartile dashed rules, optional truncated half-violin + points),
   - Before–after paired subject lines (default when `ColumnDesignHint::PairedRepeatedMeasures`), stacked bars, and interaction plots,
   - **`Grouped` visual factor mapping (`Columns = Color/Legend`, `Rows = X-Axis Position`):**
     - `Interleaved` bars/symbols cluster series at each Row factor $r$ along the X-axis (defaulting X tick labels to `Row titles`), comparing Column series side-by-side within each row.
     - `Separated` bars/symbols render all rows of Column $A$ as Block 1 followed by all rows of Column $B$ as Block 2 (defaulting X tick labels to `Column titles`, or switchable to `Row titles` / `Row numbers` in `Format Axes` while numeric range bounds stay disabled on categorical axes). Swapping which factor controls series color vs. cluster grouping is achieved cleanly via `Transpose`.
3. **Contingency & Parts-of-Whole Graphs:**
   - Interleaved/stacked count or percentage bars, $100\%$ stacked fraction bar, pie, ring/donut (with center total callout), and $10 \times 10$ unit/waffle grid.
4. **Survival Graphs:**
   - Kaplan–Meier right-continuous step curves, vertical/cross censored-observation tick marks, shaded/dashed asymmetrical Greenwood $95\%$ CI bands, and an axis-aligned **Number at Risk** table rendered directly beneath major X-axis ticks.
5. **Multiple Variables & Diagnostic Graphs (Prism 11.1 scope):**
   - **Five-channel Multivariate & Bubble Scatter:** Maps up to five independent columns simultaneously to `X`, `Y`, `Color` (discrete categorical palette or continuous perceptual ramp + colorbar), `Size` (continuous area scaling or ordered discrete step radii), and point `Label` text, with non-modal `Apply` live preview, optional group **convex hulls**, and **confidence ellipses**,
   - **Categorical $\times$ continuous GDOs directly from `MultipleVariables` sheets** (bars, box-and-whisker, violin, column scatter, summary symbols & connecting lines) honoring independent graph-level categorical ordering,
   - **Heatmaps** (grouped cell means/medians or correlation matrix $r/\rho$ with continuous perceptual colorbar and optional cell value labels),
   - **Volcano plots** ($-\log_{10} p$ or $-\log_{10} q$ vs. difference / $\log_2$ fold change with horizontal FDR/alpha threshold and vertical effect-size thresholds),
   - **Bland–Altman** bias + $95\%$ limits-of-agreement plots, **ROC** curves with identity diagonal, and **Forest / parameter-CI** plots.

---

## 3. Coordinate Systems, Nonlinear Scales & Discontinuous Axes

### 3.1 Scale Transforms (`AxisScale`)
Every axis maps domain coordinate $u$ to normalized segment coordinate $t \in [0, 1]$ via a guarded, finite bijection (`Option<f64>`, never producing `NaN` or `±Inf` into scene paths):
- `Linear`: $g(u) = u$,
- `Log10`, `Log2`, `Ln`: defined strictly for $u > 0$ (non-positive data points are omitted from log axes with a structured `ChartWarning` (code `"non_positive_on_log_axis"`); lower error-bar whiskers that extend $\le 0$ clip cleanly to the axis floor with a downward-clipped indicator rather than disappearing or crashing),
- `Logit`: $g(u) = \ln\!\left(\frac{u}{100 - u}\right)$ for percentages $u \in (0, 100)$ (or $u \in (0, 1)$),
- `Probit` (probability axis): $g(u) = \Phi^{-1}(u / 100)$ for $u \in (0, 100)$, placing normal-distribution cumulative curves on a straight line with symmetric probability ticks (`0.1, 1, 5, 10, 20, 50, 80, 90, 95, 99, 99.9`),
- `Reciprocal`: $g(u) = 1 / u$ ($u \ne 0$).

### 3.2 Discontinuous (Broken) Axes
An axis may split into $1$, $2$, or $3$ segments (`Vec<AxisSegment>`), each owning:
- Domain `[min, max]`, independent `AxisScale`, fractional length weight on the physical axis (`length_share`), and independent major/minor tick interval,
- Configurable gap width (mm/pt) and break-mark ornament (`DoubleSlash`, `Zigzag`, `BlankGap`).
Data series and fitted curves clip cleanly per segment without drawing spurious diagonal strokes across the gap unless explicitly requested.

### 3.3 Framing, Ticks & Scientific Typography (`crates/text`)
- **Frame presets:** Open L-axes (default), offset/Tufte range frame, plain boxed, four-axis with secondary right-Y (`Y2`), or origin-crossing axes ($X=0, Y=0$).
- **Tick formatting:** Decimal, power-of-10 superscript ($10^{-6}$), scientific (`1e-6`), antilog (`0.001, 0.01, 0.1, 1, 10`), engineering SI prefixes, or custom text/categorical labels at $0^\circ, 45^\circ, 90^\circ$.
- **Rich scientific text spans:** `lumencraft-text` parses lightweight markup for subscripts (`EC_{50}`), superscripts (`cm^{-2}`), italic mathematical symbols, Greek letters, and live hooked constants (`R^2 = {{result:fit-1:r2}}`) into positioned glyph runs shared identically across GPU, SVG, and PDF.

---

## 4. Live Pairwise Significance Brackets

`GraphSpec::comparisons` links directly to a post-hoc / *t*-test / survival `ResultSheet`:
- **Automatic collision-free stacking:** Brackets sort by column span (`|col_b - col_a|` ascending, then left column ascending), measure the maximum rendered height of bars, error bars, and scatter points beneath `[col_a, col_b]`, and stack upward in crisp vertical increments (`bracket_step_pt`).
- **Live recalculation:** Editing or excluding a replicate in the upstream `DataSheet` recomputes the *p*-value / *q*-value in `lumencraft-stats`, updates the bracket label (`****`, `***`, `**`, `*`, `ns`, or formatted numeric $p$/$q$ according to the analysis `PValueFormat`), and hides/shows non-significant brackets automatically when `"Show only significant comparisons"` is active.
- **Bracket styles:** Classic square bracket, half-tick bracket, flat line, or curly brace, with configurable line weight and font size.
- **Model equation callouts (`R9.16` / `QOL-16`, done; [equations-only specification](calculators-and-quick-stats.md#qol16-equations)):** One-click symbolic or live-substituted $\TeX$-subset best-fit model equation annotations on regression graphs (`GraphAnnotation::ModelEquation`, inserted via `graph.addAnnotation` with kind `model_equation` or the UI command `ui.graph.insertModelEquation`), recalculated when linked fits update. Rendering is currently single-line; the two-dimensional typeset upgrade is planned in [`R10.19`](ROADMAP.md#r1019-typeset-mathematical-equation-annotations). Group letters, pairwise-display modes, and additional fit-statistic callouts are outside this item; Compact Letter Display is planned separately in [`R10.11`](ROADMAP.md#r1011-compact-letter-display-cld) (todo); existing significance brackets remain unchanged.

---

## 5. Consistent Styling ("Wand / Clone Style") & Palettes

- **Perceptual, accessible palettes (`crates/palette`):** Ships original, license-clean scientific schemes (Okabe–Ito colorblind-safe, Paul Tol Bright/Muted/High-Contrast, Viridis, Cividis, Plasma, Monochrome journal print grays) plus live Deuteranopia / Protanopia / Tritanopia preview matrices (`AGENTS.md`: zero vendor colour schemes copied).
- **Style cloning (`graph.applyStyleFrom`):** Copies visual appearance tokens (palette, fonts, axis frame/thickness, symbol shapes, error-bar cap width, background) from a reference graph onto one or multiple target graphs in a single undoable command without altering target data bindings.

---

## 6. Multi-Panel Page Layouts & Publication Export

A `LayoutSheet` arranges multiple live `GraphSheet` panels, live embedded `DataSheet` / `ResultSheet` tables, legends, and rich-text callouts onto physical publication pages (editing or excluding an upstream cell refreshes all placed graphs and embedded table cells reactively):
- **Physical dimensions & journal presets:** Exact width $\times$ height in `mm`, `cm`, `in`, or `pt` (including single-column `85 mm` / `89 mm`, 1.5-column `120 mm` / `136 mm`, and double-column `174 mm` / `183 mm` journal widths).
- **Grid & alignment guides:** Configurable row $\times$ column layout templates, baseline alignment across adjacent Y axes (aligning plot-area origins even when Y-tick label widths differ!), and automatic panel badges (`A`, `B`, `C`, … or `a`, `b`, `c`).
- **Export targets:**
  - **SVG (`crates/svg`):** Crisp vector paths, optional editable `<text>` vs. outlined glyph paths, non-scaling hairlines avoided, deterministic attribute ordering.
  - **PDF 1.7 / 1.4 (`crates/pdf`):** Direct `%PDF-1.7` default vector writer (`--pdf-version 1.4|1.7`), `FlateDecode` streams, optional SFNT TrueType `/FontFile2` embedding (when supplied via `--ttf-font` or `LUMENCRAFT_FONTS_DIR`), Adobe Core-14 Helvetica/Symbol selectable text + outlined vector glyph-path fallback (with `BUNDLED_FONTS` optional/empty via `--no-default-features`), BiDi RTL reordering, sRGB / print CMYK / Grayscale colour spaces, exact `MediaBox` / `TrimBox` / `BleedBox`, and 20-byte `xref`. *(External-viewer PDF QA under `G5` is done; see [`R5`](ROADMAP.md).)*
  - **Raster (`crates/render`):** `72`–`1200` DPI anti-aliased PNG and uncompressed/Deflate/LZW TIFF with transparent, white, or custom paper fill, enforcing WCAG AA contrast checks in UI chrome while keeping figure paper crisp white by default (raster export stays on the CPU rasterizer; the native `eframe`/`wgpu` window from `R8.3` presents CPU-composited frames as a texture).

---

## 7. Risk Spike `S2` Decision (Text & PDF without Bundled Fonts): **GO**

- **Decision:** **GO** — direct pure-Rust **`%PDF-1.7` / `1.4`** writer (`crates/pdf`) + optional `/FontFile2` SFNT embedding + `lumencraft-text` Core-14 metrics/glyph path engine. No external PDF/text engine or heavy desktop font rasterizer dependency is required.
- **Evidence (verified with `--no-default-features`, `BUNDLED_FONTS` empty, and default features):**
  - Historical `S2` baseline measurement (`spike_s2_` under `--no-default-features`): multi-page PDF generation with `BUNDLED_FONTS` empty completed in `0.293–0.395 ms` (`0.422 ms` default; compressed `2,898` B vs uncompressed `4,600` B; `pages = 2`, `total_objs = 15`, `xref_entries = 16`, `p1_stream_bytes = 1,997`, `p2_stream_bytes = 562`).
  - Visual `L2`–`L3` verification (`--offline`) passed with zero failures under both default features and `--no-default-features` (`lumencraft-chart`, `lumencraft-text`, `lumencraft-pdf`, `lumencraft-svg`, `lumencraft-render`), `0` Clippy warnings; live per-crate test counts are tracked in [docs/STATUS.md](STATUS.md).
  - Fidelity verified structurally and at pixel level: `%PDF-1.7` default header (`--pdf-version 1.4|1.7`), optional TrueType `/FontFile2` SFNT embedding, selectable Core-14 Helvetica + Symbol Greek/math text, superscripts & subscripts, BiDi RTL reordering, rotated axis titles, `AdditionalTick` across all 7 scales & broken segments, `sRGB` / `CMYK` / `Grayscale` colour spaces, `/MediaBox` / `/TrimBox` / `/BleedBox`, and outlined glyph path mode.
- **Scope note:** Structural parsers and pixel raster tests validate xref tables, compressed streams, `/FontFile2` objects, operators, and XML/text geometry deterministically; interactive opening inside external GUI PDF viewers with installed external `.ttf` fonts was covered by the `G5` external-viewer QA (done).
