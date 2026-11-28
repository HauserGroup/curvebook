# Graphs, page layouts and export capabilities

<!-- Audience: Tier 2 Technical Reference — exhaustive graph families, axis scales, annotations, layout rules, and export limits belong here; link to Tier 1 guides (desktop-gui.md) for scientist walkthroughs. -->

This technical reference describes CurveBook's graph families, styling overrides, axis scales, annotations, colour palettes, page layouts, and the limits that apply to vector and raster export. For a hands-on guide to customizing figures and arranging multi-panel layouts, see the [Desktop GUI guide](desktop-gui.md).

CurveBook compiles every graph and multi-panel page layout into a single device-independent vector scene. This guarantees identical geometry on the desktop canvas and in SVG, PDF 1.7, PNG and TIFF rasters, and ECMA-234 Enhanced Metafiles (`.emf`). For full command schemas, see the [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), the [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and the [Parametric Equations Registry](registry/equations.html).

---

## Graph families and accepted table types {#graph-families}

CurveBook provides eight graph families. Each family accepts specific kinds of data tables or results sheets:

| Graph family | Accepted sources | Built-in representations |
|---|---|---|
| **XY** (`Xy`) | XY, Multiple Variables | Scatter points, connecting lines, splines, spaghetti trajectories, error envelopes, fitted curves, and AUC shading. |
| **Column** (`Column`) | Column, Nested | Vertical or horizontal bars, replicate dots, summary lines, before-after lines, box plots, and violin plots. |
| **Grouped** (`Grouped`) | Grouped, Nested | Interleaved, separated, stacked, or superimposed bars, boxes, violins, and points (including population pyramids). |
| **Contingency** (`Contingency`) | Contingency | Grouped count bars, stacked count bars, and 100% stacked bars of normalized fractions. |
| **Parts of Whole** (`PartsOfWhole`) | Parts of Whole | Pie, donut (with an optional total in the center), and 100% horizontal stacked bar. |
| **Survival** (`Survival`) | Survival table or results | Kaplan–Meier curves drawn as steps or polylines, censor ticks, error bands or SEM, median crosshairs, Number at Risk tables, and Cox survival overlays for covariate profiles. |
| **Multiple Variables** (`MultipleVariables`) | Multiple Variables | Bubble and scatter plots that map columns to X, Y, colour, size, marker symbol, or split panels. |
| **Diagnostic** (`Diagnostic`) | Results sheets and specialized inputs | Heatmaps, parameter correlation heatmaps, volcano plots, Bland–Altman, Gardner–Altman estimation plots, ROC curves, forest plots, residual plots, homoscedasticity, actual vs. predicted (calibration), residual vs. row, gap statistic, normal and lognormal Q–Q plots, standalone hierarchical dendrograms, and PCA biplots. |

Notable representation options across the families:

- **XY curves and trajectories:** Connecting lines can run point to point, as a staircase, as a Fritsch–Carlson or Steffen monotonic cubic spline, or as an Akima spline. XY graphs also support before-after connections, longitudinal spaghetti trajectories for individual replicates, and confidence or prediction bands around fitted curves.
- **Theoretical distribution overlays:** You can plot continuous Normal, Student *t*, Fisher *F*, and $\chi^2$ density curves with shaded tail polygons. The discrete Binomial and Poisson distributions can be drawn as integer-step PMF spikes, as bars, or as step-CDF curves.
- **Column and Grouped options:** Both families support horizontal orientation, including back-to-back stacked population pyramids, and two-way or three-way designs. Replicate overlays can use a deterministic beeswarm or aligned columns. Box-and-whisker plots support Tukey 1.5×IQR, min to max, and the 10–90, 5–95, 2.5–97.5, and 1–99 percentiles. Violin plots can be full or truncated, and can be combined with raw points.
- **Survival metrics:** Curves show percent survival, percent cumulative events, or fraction survival, with Greenwood 95% confidence bands or SEM error bars. CurveBook also draws Cox PH baseline survival overlays and survival overlays for user-defined covariate profiles.
- **Heatmaps and forest plots:** Heatmaps support continuous colour ramps, row and column labels at 0°, 45°, or 90°, and parameter correlation matrices with a `[-1, 1]` colorbar. `transform.run` can also create a heatmap automatically when you pass `create_graph: true`. Forest plots accept regression estimates or pre-summarized study bounds (`UpperLowerLimits`, `MeanLowHigh`), with optional pooled diamonds and reference lines.
- **Statistical diagnostic plots:**
  - **Volcano, Bland–Altman, and Estimation plots:** Volcano plots map $-\log_{10} P$ or adjusted $q$ against the fold change of the effect. Bland–Altman plots show the mean bias and the limits of agreement. Gardner–Altman estimation plots pair raw two-group observations and group means/CIs on the left axis with a scale-matched floating difference axis on the right aligned so Group A's mean equals zero and Group B's mean aligns with the mean difference $\Delta$ and its confidence interval, with an optional paired slope-lines mode (`estimation_paired_lines`; Gardner & Altman 1986; Ho et al. 2019).
  - **ROC and calibration:** ROC plots show sensitivity against $1 - \text{specificity}$, with a diagonal reference line and an AUC label. The curve can come from a standalone ROC analysis or from a multivariable logistic GLM. Actual vs. predicted plots include a $y = x$ identity line and logistic Hosmer–Lemeshow calibration bins.
  - **Residuals, Q–Q and clustering:** Homoscedasticity plots show the spread of residuals against fitted values, including Spearman badges for ANOVA. Residual vs. row plots trace GLM or Cox–Snell residuals in row order. Normal and lognormal Q–Q plots compare sample values with Blom quantiles. Gap statistic plots draw consensus K-means Gap curves with 1-SE bars and a marker at the optimal $k^*$. Standalone **hierarchical dendrograms** render agglomerative merge trees with leaf labels and an optional dashed cut-height threshold, and **PCA biplots** overlay PC1 vs. PC2 observation scores (coloured by any categorical grouping column) with scaled variable loading vectors.

---

## Series styling, error bars and overrides {#series-styling}

- **Series styling (`graph.setSeries`):** This command controls each dataset's visibility, label, RGBA colour and opacity, and assignment to the secondary `y2_axis`. It also sets the marker shape (circle, square, triangle, diamond, cross), fill (solid, open, translucent), size, line dash style, and summary measure (mean, median, geometric mean). Pass `all_series: true` to update every dataset at once.
- **Positioning and overlays:** A series can show trajectory lines for individual replicates. You can nudge a series horizontally or by points with `offset` (`{ x_data, x_pt, y_pt }`); the `x_data` offset is included in the automatic axis bounds. Overrides for bar width (`bar_width_override_pt`) and bar border (`bar_border_width_pt`) adjust bar geometry. Raw data (`show_raw_data`) and the fitted curve have independent visibility settings, so a fit to a filtered subset can be drawn without duplicate markers.
- **Error bars:** Six error bar metrics are available. Each has a selectable direction (both, upper only, lower only), end caps, and stroke patterns:
  - **SD:** $\pm 1\text{ SD}$ ($n - 1$ denominator).
  - **SEM:** $\pm 1\text{ SEM} = \text{SD}/\sqrt{n}$.
  - **95% CI:** Two-sided 95% Student-*t* confidence interval around the mean.
  - **IQR:** Asymmetric $[Q_1, Q_3]$ quartiles.
  - **Range:** Full `[min, max]` interval.
  - **Explicit subcolumns:** Pre-summarized bounds read from `MeanSd`, `MeanSem`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr`.
  On log scales, lower error bounds that are zero or negative are clamped to the axis floor and recorded in the scale diagnostics.
- **Per-point overrides (`graph.formatSelectedPoints`):** Target a single replicate by `(series_index, point_index, subcol)` to highlight, restyle, or hide it without editing table cells. A rule for an exact `subcol` takes precedence over a wildcard override.

---

## Axis scales, broken axes and frames {#axes-and-scales}

### The seven axis scales {#axis-scales}

You can set the scale independently on `x_axis`, `y_axis`, and the secondary `y2_axis`:

| Scale | Forward mapping | Valid domain |
|---|---|---|
| **Linear** | $x$ | All finite real numbers. |
| **Log10** | $\log_{10}(x)$ | Strictly positive $x > 0$. |
| **Log2** | $\log_2(x)$ | Strictly positive $x > 0$. |
| **Ln** | $\ln(x)$ | Strictly positive $x > 0$. |
| **Logit** | $\ln\!\left(\frac{p}{1 - p}\right)$ (unit `[0, 1]` or `[0, 100]`%) | Open interior $0 < p < 1$ (or $0 < p < 100$). |
| **Probit** | $\Phi^{-1}(u / 100)$ | Open percentage interior $u \in (0, 100)$. |
| **Reciprocal** | $1 / x$ (optional direction reversal) | Non-zero $x \ne 0$. |

Points that fall outside a non-linear scale's domain, and non-finite values, are skipped and recorded in the scale diagnostics instead of causing an error. Each axis segment generates up to 256 ticks. Tick labels can use decimal, unsigned absolute decimal (for back-to-back pyramids), power-of-ten superscript, scientific E, antilog, engineering SI, or categorical format. Tick direction (outside, inside, both, none) and label visibility (`show_tick_labels`) are set for each axis.

### Discontinuous and broken axes {#broken-axes}

Every axis can be split into 1 to 3 contiguous range segments. Each segment has a `[min, max]` interval and a proportional visual share of the axis. Segments are separated by `gap_width_pt` and marked with a double-slash, zigzag, or blank-gap break ornament. Bars, lines, and points are clipped at the break boundaries.

### Frame presets, thresholds and additional ticks {#frames-and-ticks}

- **Frame presets:** Choose Open L (the default, with left and bottom spines), offset Tufte spines trimmed to the data range (default offset 6.0 pt), a plain four-sided box, dual-Y with a secondary `y2_axis` on the right, or axes that cross at the origin `(0, 0)`.
- **Thresholds and custom ticks:** Add dashed or shaded threshold bands (`graph.setAxes`), custom labelled tick marks (`graph.setAdditionalTicks`), and vertical reference gridlines, such as null-effect lines on forest plots.
- **Canvas and paper backgrounds:** The plot fill and the page fill can each be solid or transparent (`None`). Set them with `graph.setStyle` or **Format Graph › Page**. Transparent fills let you place axis-free overlays and insets in the foreground of multi-panel layouts.

---

## Annotations, pairwise brackets, Piepho CLD and live TeX equations {#annotations-and-significance}

- **Geometric and text annotations:** Place lines, arrows, rectangles, ellipses, shaded spans, legend positions, and rich text callouts. Text annotations support subscripts, superscripts, Greek Unicode characters, info constant hooks (`{{Info:Key}}`), and result scalar hooks.
- **Live embedded result tables (`edit.pasteResultTable`):** Paste a rectangular slice of cells (`selected_rows`, `selected_cols`) from a results sheet onto a graph or page layout. Styling options include `transpose`, column and row title toggles, bold headers, font size, outer border and gridline widths, and border and fill colours. Embedded tables stay live: they update through the DAG in RGBA, SVG, PDF 1.7, and `.emf` output.
- **Embedded raster images:** Insert content-addressed PNG, JPEG, or TIFF images with `graph.insertImage`, `layout.insertImage`, or `edit.pasteImage`, and change them with `graph.updateImage` or `layout.updateImage`. Options include crop insets, borders, `rotation_deg`, and `lock_aspect_ratio`. SVG images are rejected, and CurveBook never fetches remote URLs. Decoding limits each asset to 32 MiB, 16,384 px per side, and 67,108,864 total pixels; these limits apply per asset and per compiled layout.
- **Live TeX best-fit model equations (`graph.addAnnotation` kind `model_equation`, `ui.graph.insertModelEquation`):** Link a graph to a linear regression, nonlinear regression, or Passing–Bablok results dataset. The equation appears in symbolic form or with the current coefficient values substituted, and it refreshes automatically when you refit. Typesetting uses the two-dimensional math boxes of Knuth's TeXbook, Appendix G. Supported constructs are stacked `\frac`, `\dfrac`, `\tfrac`, and `\binom`; `\sqrt` with a vinculum; stacked superscripts and subscripts; scaled `\left...\right` delimiters; and Greek parameter names.
- **Pairwise significance brackets and compact letter display:**
  - **Brackets only:** Draw horizontal or stepped comparison brackets labelled with stars (`*` to `****`, or `ns`), formatted *P*-values, or custom text.
  - **Compact letter display (`graph.setCompactLetterDisplay`):** CurveBook computes exact minimal sets of equivalence letters (`"a"`, `"ab"`, `"b"`, …) from pairwise significance records, using Piepho's (2004) insert-and-absorb and sweeping algorithm. Letters are ordered by descending or ascending mean and placed conflict-free above bars, boxes, or violins.
  - **Brackets and compact letter display:** Draws brackets and compact letters together without overlap.

---

## Colour palettes, colormaps and CVD preview {#palettes-and-cvd}

- **Qualitative scientific palettes:** Okabe–Ito (the default, an 8-colour palette that is safe for colour vision deficiency), Paul Tol Bright, Paul Tol Muted, Paul Tol High Contrast, Paul Tol Vibrant, and Monochrome Print.
- **Continuous colormaps:** Viridis, Cividis, Plasma, Diverging Blue–Red, and Grayscale are perceptually uniform ramps interpolated in Oklab. Custom ramps accept up to 256 stops, and discrete sampling accepts up to 4,096 samples.
- **Live CVD and grayscale preview:** The canvas viewport applies the Machado (2009) deuteranopia, protanopia, and tritanopia matrices and a grayscale-print matrix (`"grayscale"`). The preview does not change stored figure colours or exported vector files.

---

## Batch creation and selective style inheritance (`graph.create`, `graph.applyStyleFrom`) {#style-inheritance}

- **One graph per dataset (`graph.create`):** Pass `one_per_data_set: true`, optionally with a shared `y_axis_title`, to create a separate single-series graph for each dataset in one atomic step. This works on any data table and on transform results sheets (Transpose, Normalize, Select and Transform). CurveBook checks the project sheet limit before it creates any graphs.
- **Multi-target style cloning (`graph.applyStyleFrom`):** Copy selected style facets from a donor graph to one or more `target_sheet_ids` in a single undoable step. You can choose axes, palette and series styles, fonts, error bars, legend placement, and canvas dimensions. Each target keeps its own data bindings, series titles, and fitted overlays.

---

## Page layout composition {#page-layouts}

A page layout combines up to 64 items on one page to build a multi-panel publication figure:

- **Physical units and presets:** Coordinates and margins are given in points, millimeters, or inches, and the page can be portrait or landscape. Journal presets cover single-column (85, 88, 89 mm), 1.5-column (120, 130, 136 mm), and double-column (174 mm default, 180, 183 mm) widths, as well as full page (including US Letter), 16:9 slide, and poster panel. When you resize a layout, you can either lock typography and line weights at their exact point sizes or scale them proportionally.
- **Arrangement, equalization, Z-order and badges:** A page can combine live graphs, live result tables, embedded images, shapes, and text headers. `layout.alignOrigins` aligns shared plot origins. `layout.equalizeScaling` equalizes inner plot scales to within 0.25 pt, matching the smallest plot, the largest plot, or an explicit percentage. `layout.reorderPanel` controls Z-order stacking (`bring_to_front`, `bring_forward`, `send_backward`, `send_to_back`). Transparent overlays (`paper_fill: None`, with ticks and labels hidden) and automatic panel badges (uppercase `"A", "B", …`, lowercase `"a", "b", …`, or none) are built in.

---

## Publication preflight, export capabilities and limits {#export-capabilities}

### Canvas print preflight (`graph.preflightCheck`) {#preflight}

Preflight inspects the compiled vector primitives before export. It shows a warning, without blocking the export, when:

- Any visible text, including subscripts and superscripts, renders smaller than 5.0 pt (`min_font_size`).
- Any stroked path renders thinner than 0.25 pt (`hairline_stroke`).
- A text anchor or bounding box falls outside the canvas or the clip rectangle (`text_outside_canvas`).

### Supported export formats and hard safety limits {#export-formats}

| Format | Commands | Key capabilities | Enforced limits |
|---|---|---|---|
| **SVG 1.1** | `export.svg`, `graph.exportSvg` | Exact vector paths, embedded fonts or outlined glyphs, and structural validation. | Finite bounds; coordinate precision 0 to 6 (default 3). |
| **PDF 1.7** | `export.pdf`, `graph.exportPdf` | Zlib-compressed vectors, sRGB, CMYK, or Grayscale colour, multiple pages, trim and bleed boxes, and RTL BiDi text. | 64 pages, 500,000 primitives, and 32 MiB of output. |
| **PNG & TIFF** | `export.png`, `export.tiff`, `graph.exportPng` | 4×4 subpixel antialiasing, sizing by DPI or pixels, resolution metadata, and LZW or Deflate TIFF compression. | 16,384 px per side; 67,108,864 total pixels (equivalent to 8,192 × 8,192). |
| **Windows EMF (`.emf`)** | `graph.exportEmf`, `layout.exportEmf` | Clean-room little-endian ECMA-234 vector records with Unicode text. | Finite positive dimensions within the 32-bit EMF byte limit. |

Format-specific export options:

- **SVG:** Options include `pretty`, `include_xml_declaration`, `include_metadata`, `outline_text` (glyphs drawn as paths), and coordinate `precision`. `embed_fonts` embeds the TrueType faces that are available at run time.
- **PDF 1.7:** Options include `compress_streams`. Text can use the 14 standard PDF fonts (WinAnsi text encoding plus the standard Symbol font), embedded TrueType fonts, or `outline_text` vector glyphs. Output supports alpha transparency, sRGB, CMYK, and Grayscale colour spaces, and physical `trim_pt` and `bleed_pt` boxes.
- **PNG & TIFF:** Rasterization uses 4×4 subpixel coverage antialiasing with a Bézier flatness tolerance of 0.15 px. PNG files carry `pHYs` resolution metadata, and TIFF files carry `dpi` metadata. TIFF can be uncompressed or use LZW or Deflate compression. The aspect ratio is always preserved: if a uniform DPI target would exceed the size limits, the export returns an explicit limit error instead of stretching the image.
- **Windows EMF:** Output uses `MM_ANISOTROPIC` mapping, with frame coordinates in units of 0.01 mm and a logical precision of 20 twips per point. It supports cubic Bézier curves, Unicode text records, and an optional BGRA DIB raster fallback.
