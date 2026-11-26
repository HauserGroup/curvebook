# Graphs, page layouts & export capabilities

CurveBook compiles every graph and multi-panel page layout into one device-independent vector scene. This guarantees identical geometry across the desktop canvas, SVG, PDF 1.7, PNG/TIFF rasters, and ECMA-234 Enhanced Metafiles (`.emf`).

See the [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Parametric Equations Registry](registry/equations.html) for full graph, layout, and overlay parameters.

---

## Graph families & accepted table types {#graph-families}

CurveBook provides eight graph families, each bound to compatible data tables or results sheets:

| Graph family | Accepted sources | Built-in representations |
|---|---|---|
| **XY** (`Xy`) | XY, Multiple Variables | Scatter points, connecting lines, splines, spaghetti trajectories, error envelopes, fitted curves, and AUC shading. |
| **Column** (`Column`) | Column, Nested | Vertical or horizontal bars, replicate dots, summary lines, before-after lines, box plots, and violin plots. |
| **Grouped** (`Grouped`) | Grouped, Nested | Interleaved, separated, stacked, or superimposed bars, boxes, violins, and points (including population pyramids). |
| **Contingency** (`Contingency`) | Contingency | Grouped count bars, stacked count bars, and 100% normalized fraction stacked bars. |
| **Parts of Whole** (`PartsOfWhole`) | Parts of Whole | Pie, donut (with optional center total), 100% horizontal stacked bar, and 10×10 waffle grid. |
| **Survival** (`Survival`) | Survival table or results | Kaplan–Meier step or polyline curves, censor ticks, error bands/SEM, median crosshairs, and Number at Risk tables. |
| **Multiple Variables** (`MultipleVariables`) | Multiple Variables | Bubble and scatter plots mapping columns to X, Y, colour, size, marker symbol, or split panels. |
| **Diagnostic** (`Diagnostic`) | Results sheets & specialized inputs | Heatmaps, volcano plots, Bland–Altman, ROC curves, forest plots, residual plots, and normal Q–Q plots. |

Notable representation options across families:

- **XY curves and trajectories:** Connecting lines support point-to-point, staircase, Fritsch–Carlson / Steffen monotonic cubic spline, Akima spline, before-after connections, longitudinal individual-replicate spaghetti trajectories, and fitted curve confidence/prediction band overlays.
- **Column and Grouped options:** Both support horizontal orientation (including back-to-back stacked population pyramids) and two-way or three-way designs. Replicate overlays support deterministic beeswarm or aligned columns; box-and-whisker plots support Tukey 1.5×IQR, min-to-max, 10–90, 5–95, 2.5–97.5, and 1–99 percentiles; violin plots support full, truncated, and raw-point combinations.
- **Survival metrics:** Curves display percent survival, percent cumulative events, or fraction survival, with selectable Greenwood 95% confidence bands or SEM error bars.
- **Diagnostic plots:** Heatmaps support continuous ramps, configurable row/column labels at 0°, 45°, or 90°, and optional auto-creation from `transform.run` (`create_graph: true`). Forest plots accept regression estimates or pre-summarized study bounds (`UpperLowerLimits`, `MeanLowHigh`) with optional pooled diamonds and reference lines. Volcano plots map $-\log_{10} P$ or adjusted $q$ against effect fold change, Bland–Altman shows bias and limits of agreement, ROC plots sensitivity vs. $1 - \text{specificity}$ with a diagonal reference, and normal Q–Q plots compare sample values to Blom normal quantiles.

---

## Series styling, error bars & overrides {#series-styling}

- **Series styling (`graph.setSeries`):** Controls per-dataset visibility, label, RGBA colour and opacity, and secondary `y2_axis` assignment. Also configures marker shape (circle, square, triangle, diamond, cross), fill (solid, open, translucent), size, line dash style, and summary measure (mean, median, geometric mean). Pass `all_series: true` to update all datasets at once.
- **Positioning and overlays:** Supports individual-replicate trajectory lines and horizontal or point nudging (`offset` `{ x_data, x_pt, y_pt }`, with `x_data` included in auto-axis bounds). Bar width and border overrides (`bar_width_override_pt`, `bar_border_width_pt`) and independent `show_raw_data` vs. fitted-curve visibility let subset-filtered fits draw without duplicate markers.
- **Error bars:** Supports six error bar metrics with selectable direction (both, upper only, lower only), end caps, and stroke patterns:
  - **SD:** $\pm 1\text{ SD}$ ($n - 1$ denominator).
  - **SEM:** $\pm 1\text{ SEM} = \text{SD}/\sqrt{n}$.
  - **95% CI:** Two-sided 95% Student-*t* confidence interval around the mean.
  - **IQR:** Asymmetric $[Q_1, Q_3]$ quartiles.
  - **Range:** Full `[min, max]` interval.
  - **Explicit subcolumns:** Pre-summarized bounds read from `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr`.
  On log scales, non-positive lower error bounds clamp safely to the axis floor and are tracked in scale diagnostics.
- **Per-point overrides (`graph.formatSelectedPoints`):** Targets `(series_index, point_index, subcol)` to highlight, restyle, or hide single replicates without modifying table cells. Exact `subcol` rules take precedence over wildcard overrides.

---

## Axis scales, broken axes & frames {#axes-and-scales}

### The seven axis scales {#axis-scales}

Configured independently on `x_axis`, `y_axis`, and secondary `y2_axis`:

| Scale | Forward mapping | Valid domain |
|---|---|---|
| **Linear** | $x$ | All finite real numbers. |
| **Log10** | $\log_{10}(x)$ | Strictly positive $x > 0$. |
| **Log2** | $\log_2(x)$ | Strictly positive $x > 0$. |
| **Ln** | $\ln(x)$ | Strictly positive $x > 0$. |
| **Logit** | $\ln\!\left(\frac{p}{1 - p}\right)$ (unit `[0, 1]` or `[0, 100]`%) | Open interior $0 < p < 1$ (or $0 < p < 100$). |
| **Probit** | $\Phi^{-1}(u / 100)$ | Open percentage interior $u \in (0, 100)$. |
| **Reciprocal** | $1 / x$ (optional direction reversal) | Non-zero $x \ne 0$. |

Points outside a non-linear domain or non-finite values are skipped cleanly and counted in scale diagnostics rather than failing. Each axis segment generates up to 256 ticks in decimal, unsigned absolute decimal (for back-to-back pyramids), power-of-ten superscript, scientific E, antilog, engineering SI, or categorical format. Tick direction (outside, inside, both, none) and label visibility (`show_tick_labels`) are configured per axis.

### Discontinuous / broken axes {#broken-axes}

Every axis supports 1 to 3 contiguous range segments, each with a `[min, max]` interval and proportional visual share. Segments are separated by `gap_width_pt` using a double-slash, zigzag, or blank-gap break ornament, and bars, lines, and points clip cleanly at break boundaries.

### Frame presets, thresholds & additional ticks {#frames-and-ticks}

- **Frame presets:** Open L (default left-and-bottom spines), offset range-trimmed Tufte spines (default 6.0 pt offset), plain 4-sided box, dual-Y with secondary right `y2_axis`, and origin-crossing axes at `(0, 0)`.
- **Thresholds and custom ticks:** Add dashed or shaded threshold bands (`graph.setAxes`), custom labelled tick marks (`graph.setAdditionalTicks`), and vertical reference gridlines (such as null-effect lines on forest plots).
- **Canvas and paper backgrounds:** Set solid or transparent (`None`) plot and page fills (`graph.setStyle` or **Format Graph › Page**) for axis-free foreground overlays and insets in multi-panel layouts.

---

## Annotations, pairwise brackets, Piepho CLD & live TeX equations {#annotations-and-significance}

- **Geometric and text annotations:** Lines, arrows, rectangles, ellipses, shaded spans, legend placement, and rich text callouts supporting subscripts, superscripts, Greek Unicode, info constant hooks (`{{Info:Key}}`), and result scalar hooks.
- **Live embedded result tables (`edit.pasteResultTable`):** Pastes reactive rectangular cell slices (`selected_rows`, `selected_cols`) from a results sheet onto graphs or page layouts. Supports `transpose`, column/row title toggles, bold headers, font size, outer border and gridline widths, and border/fill colours, updating via the DAG across RGBA, SVG, PDF 1.7, and `.emf`.
- **Embedded raster images:** Inserts content-addressed PNG, JPEG, or TIFF images (`graph.insertImage`, `layout.insertImage`, `edit.pasteImage`, `graph.updateImage`, `layout.updateImage`) with crop insets, borders, `rotation_deg`, and `lock_aspect_ratio`. SVG payloads are rejected and zero remote URLs are fetched. Decoding caps assets at 32 MiB, 16,384 px per side, and 67,108,864 total pixels per asset and per compiled layout.
- **Live TeX best-fit model equations (`graph.addAnnotation` kind `model_equation`, `ui.graph.insertModelEquation`):** Links a graph to a linear regression, nonlinear regression, or Passing–Bablok results dataset. Renders symbolic or live coefficient-substituted equations (refreshing automatically on refit) using two-dimensional Knuth TeXbook Appendix G math box typesetting (stacked `\frac`/`\dfrac`/`\tfrac`/`\binom`, `\sqrt` with vinculum, stacked super/subscripts, scaled `\left...\right` delimiters, and Greek parameters).
- **Pairwise significance brackets & compact letter display:**
  - **Brackets only:** Horizontal or stepped comparison brackets showing stars (`*` to `****` / `ns`), formatted *P*-values, or custom text.
  - **Compact letter display (`graph.setCompactLetterDisplay`):** Computes exact minimal equivalence letter sets (`"a"`, `"ab"`, `"b"`, …) from pairwise significance records using Piepho's (2004) insert-and-absorb and sweeping algorithm. Letters order by descending or ascending mean and sit conflict-free above bars, boxes, or violins.
  - **Brackets and compact letter display:** Renders both non-overlapping brackets and compact letters simultaneously.

---

## Colour palettes, colormaps & CVD preview {#palettes-and-cvd}

- **Qualitative scientific palettes:** Okabe–Ito (default CVD-safe 8-colour palette), Paul Tol Bright, Paul Tol Muted, Paul Tol High Contrast, Paul Tol Vibrant, and Monochrome Print.
- **Continuous colormaps:** Perceptually uniform Oklab-interpolated ramps (Viridis, Cividis, Plasma, Diverging Blue–Red, Grayscale), plus custom ramps with up to 256 stops and discrete sampling up to 4,096 samples.
- **Live CVD & grayscale preview:** Applies Machado (2009) deuteranopia, protanopia, tritanopia, and grayscale-print (`"grayscale"`) transformation matrices in the canvas viewport without altering stored figure colours or exported vector files.

---

## Batch creation & selective style inheritance (`graph.create`, `graph.applyStyleFrom`) {#style-inheritance}

- **One graph per dataset (`graph.create`):** Passing `one_per_data_set: true` (with an optional shared `y_axis_title`) creates one single-series graph per dataset atomically. Works on any data table or transform results sheet (Transpose, Normalize, Select and Transform) and checks the project sheet limit first.
- **Multi-target style cloning (`graph.applyStyleFrom`):** Copies selected styling facets—axes, palette and series styles, fonts, error bars, legend placement, or canvas dimensions—from a donor graph onto one or more `target_sheet_ids` in one undoable step. Each target keeps its own data bindings, series titles, and fitted overlays.

---

## Page layout composition {#page-layouts}

Page layouts compose publication multi-panel figures with up to 64 items per page:

- **Physical units & presets:** Coordinates and margins use points, millimeters, or inches in portrait or landscape orientation. Journal presets cover single-column (85, 88, 89 mm), 1.5-column (120, 130, 136 mm), double-column (174 mm default, 180, 183 mm), full page (including US Letter), 16:9 slide, and poster panel. Resizing can lock exact point typography and line weights or scale proportionally.
- **Arrangement, equalization, Z-order & badges:** Combines live graphs, live result tables, embedded images, shapes, and text headers. Supports shared origin alignment (`layout.alignOrigins`), inner plot-scale equalization within 0.25 pt (`layout.equalizeScaling`: match smallest, match largest, or explicit percent), and Z-order stacking (`layout.reorderPanel`: `bring_to_front`, `bring_forward`, `send_backward`, `send_to_back`). Transparent overlays (`paper_fill: None`, hidden ticks/labels) and automatic panel badges (uppercase `"A", "B", …`, lowercase `"a", "b", …`, or none) are built in.

---

## Publication preflight & export capabilities & limits {#export-capabilities}

### Canvas print preflight (`graph.preflightCheck`) {#preflight}

Preflight inspects compiled vector primitives before export and emits non-blocking warnings when:

- Any visible text or subscript/superscript renders below 5.0 pt (`min_font_size`).
- Any stroked path renders thinner than 0.25 pt (`hairline_stroke`).
- A text anchor or bounding box clips outside the canvas or clip rectangle (`text_outside_canvas`).

### Supported export formats & hard safety limits {#export-formats}

| Format | Commands | Key capabilities | Enforced limits |
|---|---|---|---|
| **SVG 1.1** | `export.svg`, `graph.exportSvg` | Exact vector paths, font embedding or outlined glyphs, structural validation. | Finite bounds; coordinate precision 0 to 6 (default 3). |
| **PDF 1.7** | `export.pdf`, `graph.exportPdf` | Zlib-compressed vectors, sRGB/CMYK/Grayscale, multi-page, trim/bleed boxes, RTL BiDi. | 64 pages, 500,000 primitives, 32 MiB output. |
| **PNG & TIFF** | `export.png`, `export.tiff`, `graph.exportPng` | 4×4 subpixel antialiasing, DPI or pixel sizing, resolution metadata, LZW/Deflate TIFF. | 16,384 px per side; 67,108,864 total pixels (8,192 × 8,192 eq.). |
| **Windows EMF (`.emf`)** | `graph.exportEmf`, `layout.exportEmf` | Clean-room little-endian ECMA-234 vector records with Unicode text. | Finite positive dimensions within 32-bit EMF byte cap. |

Format-specific export options:

- **SVG:** Supports `pretty`, `include_xml_declaration`, `include_metadata`, `embed_fonts` (when runtime TrueType faces are available), `outline_text` glyph paths, and coordinate `precision`.
- **PDF 1.7:** Supports `compress_streams`, Core-14 WinAnsi + Adobe Symbol text or embedded TrueType / `outline_text` vector glyphs, alpha transparency, sRGB/CMYK/Grayscale colour spaces, and physical `trim_pt` and `bleed_pt` boxes.
- **PNG & TIFF:** Uses 4×4 subpixel coverage antialiasing (0.15 px Bézier flatness tolerance), embeds PNG `pHYs` and TIFF `dpi` metadata, and supports uncompressed, LZW, or Deflate TIFF compression. Aspect ratio is preserved strictly; oversize uniform DPI targets return an explicit limit error rather than stretching.
- **Windows EMF:** Emits `MM_ANISOTROPIC` mapping with 0.01 mm frame coordinates, 20 twips/pt logical precision, cubic Bézier curves, Unicode text records, and optional BGRA DIB raster fallback.
