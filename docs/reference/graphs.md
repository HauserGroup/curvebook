# Graphs, page layouts & export capabilities

<!-- Audience: Tier 2 Technical Reference — exhaustive graph families, axis scales, annotations, layout rules, and export limits belong here; link to Tier 1 guides (desktop-gui.md) for scientist walkthroughs. -->

This technical reference documents CurveBook's graph families, styling overrides, axis scales, annotations, colour palettes, page layouts, and vector/raster export limits. For a hands-on guide to customizing figures and arranging multi-panel layouts, see the [Desktop GUI guide](desktop-gui.md).

CurveBook compiles every graph and multi-panel page layout into one device-independent vector scene. This guarantees identical geometry across the desktop canvas, SVG, PDF 1.7, PNG/TIFF rasters, and ECMA-234 Enhanced Metafiles (`.emf`). See the [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Parametric Equations Registry](registry/equations.html) for full command schemas.

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
| **Survival** (`Survival`) | Survival table or results | Kaplan–Meier step or polyline curves, censor ticks, error bands/SEM, median crosshairs, Number at Risk tables, and Cox covariate-profile survival overlays. |
| **Multiple Variables** (`MultipleVariables`) | Multiple Variables | Bubble and scatter plots mapping columns to X, Y, colour, size, marker symbol, or split panels. |
| **Diagnostic** (`Diagnostic`) | Results sheets & specialized inputs | Heatmaps, parameter correlation heatmaps, volcano plots, Bland–Altman, ROC curves, forest plots, residual plots, homoscedasticity, actual vs. predicted / calibration, residual vs. row, gap statistic, and normal / lognormal Q–Q plots. |

Notable representation options across families:

- **XY curves & trajectories:** Connecting lines support point-to-point, staircase, Fritsch–Carlson / Steffen monotonic cubic spline, and Akima spline modes. XY graphs also support before-after connections, longitudinal individual-replicate spaghetti trajectories, and fitted-curve confidence or prediction bands.
- **Theoretical distribution overlays:** Plot continuous Normal, Student *t*, Fisher *F*, and $\chi^2$ density curves with shaded tail polygons. Discrete Binomial and Poisson distributions support integer-step PMF spikes, bars, and step-CDF curves.
- **Column & Grouped options:** Both families support horizontal orientation (including back-to-back stacked population pyramids) and two-way or three-way designs. Replicate overlays support deterministic beeswarm or aligned columns. Box-and-whisker plots support Tukey 1.5×IQR, min-to-max, 10–90, 5–95, 2.5–97.5, and 1–99 percentiles. Violin plots support full, truncated, and raw-point combinations.
- **Survival metrics:** Curves display percent survival, percent cumulative events, or fraction survival with Greenwood 95% confidence bands or SEM error bars. Cox PH baseline and user-defined covariate-profile survival overlays are also supported.
- **Heatmaps & forest plots:** Heatmaps support continuous ramps, row/column labels at 0°, 45°, or 90°, parameter correlation matrices (`[-1, 1]` colorbar), and optional auto-creation from `transform.run` (`create_graph: true`). Forest plots accept regression estimates or pre-summarized study bounds (`UpperLowerLimits`, `MeanLowHigh`) with optional pooled diamonds and reference lines.
- **Statistical diagnostic plots:**
  - **Volcano & Bland–Altman:** Volcano plots map $-\log_{10} P$ or adjusted $q$ against effect fold change. Bland–Altman plots show mean bias and limits of agreement.
  - **ROC & calibration:** ROC plots show sensitivity vs. $1 - \text{specificity}$ (from standalone ROC or multivariable logistic GLM) with a diagonal reference and AUC label. Actual vs. predicted plots include $y = x$ identity lines and logistic Hosmer–Lemeshow calibration bins.
  - **Residuals, Q–Q & clustering:** Homoscedasticity plots inspect residual spread vs. fitted values (including ANOVA Spearman badges). Residual vs. row plots trace GLM or Cox–Snell residuals in row order. Normal and lognormal Q–Q plots compare sample values to Blom quantiles. Gap statistic plots render consensus K-means Gap curves with 1-SE bars and optimal $k^*$ markers.

---

## Series styling, error bars & overrides {#series-styling}

- **Series styling (`graph.setSeries`):** Controls per-dataset visibility, label, RGBA colour and opacity, and secondary `y2_axis` assignment. It also configures marker shape (circle, square, triangle, diamond, cross), fill (solid, open, translucent), size, line dash style, and summary measure (mean, median, geometric mean). Pass `all_series: true` to update all datasets at once.
- **Positioning and overlays:** Supports individual-replicate trajectory lines and horizontal or point nudging via `offset` (`{ x_data, x_pt, y_pt }`, with `x_data` included in auto-axis bounds). Bar width (`bar_width_override_pt`) and border (`bar_border_width_pt`) overrides customize bar geometry. Independent `show_raw_data` vs. fitted-curve visibility lets subset-filtered fits draw without duplicate markers.
- **Error bars:** Supports six error bar metrics with selectable direction (both, upper only, lower only), end caps, and stroke patterns:
  - **SD:** $\pm 1\text{ SD}$ ($n - 1$ denominator).
  - **SEM:** $\pm 1\text{ SEM} = \text{SD}/\sqrt{n}$.
  - **95% CI:** Two-sided 95% Student-*t* confidence interval around the mean.
  - **IQR:** Asymmetric $[Q_1, Q_3]$ quartiles.
  - **Range:** Full `[min, max]` interval.
  - **Explicit subcolumns:** Pre-summarized bounds read from `MeanSd`, `MeanSem`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr`.
  On log scales, non-positive lower error bounds clamp safely to the axis floor and are tracked in scale diagnostics.
- **Per-point overrides (`graph.formatSelectedPoints`):** Targets `(series_index, point_index, subcol)` to highlight, restyle, or hide single replicates without editing table cells. Exact `subcol` rules take precedence over wildcard overrides.

---

## Axis scales, broken axes & frames {#axes-and-scales}

### The seven axis scales {#axis-scales}

Configure scales independently on `x_axis`, `y_axis`, and secondary `y2_axis`:

| Scale | Forward mapping | Valid domain |
|---|---|---|
| **Linear** | $x$ | All finite real numbers. |
| **Log10** | $\log_{10}(x)$ | Strictly positive $x > 0$. |
| **Log2** | $\log_2(x)$ | Strictly positive $x > 0$. |
| **Ln** | $\ln(x)$ | Strictly positive $x > 0$. |
| **Logit** | $\ln\!\left(\frac{p}{1 - p}\right)$ (unit `[0, 1]` or `[0, 100]`%) | Open interior $0 < p < 1$ (or $0 < p < 100$). |
| **Probit** | $\Phi^{-1}(u / 100)$ | Open percentage interior $u \in (0, 100)$. |
| **Reciprocal** | $1 / x$ (optional direction reversal) | Non-zero $x \ne 0$. |

Points outside a non-linear domain or non-finite values are skipped cleanly and recorded in scale diagnostics rather than failing. Each axis segment generates up to 256 ticks in decimal, unsigned absolute decimal (for back-to-back pyramids), power-of-ten superscript, scientific E, antilog, engineering SI, or categorical format. Tick direction (outside, inside, both, none) and label visibility (`show_tick_labels`) are configured per axis.

### Discontinuous / broken axes {#broken-axes}

Every axis supports 1 to 3 contiguous range segments, each with a `[min, max]` interval and proportional visual share. Segments separate by `gap_width_pt` using a double-slash, zigzag, or blank-gap break ornament. Bars, lines, and points clip cleanly at break boundaries.

### Frame presets, thresholds & additional ticks {#frames-and-ticks}

- **Frame presets:** Choose Open L (default left-and-bottom spines), offset range-trimmed Tufte spines (default 6.0 pt offset), plain 4-sided box, dual-Y with secondary right `y2_axis`, or origin-crossing axes at `(0, 0)`.
- **Thresholds and custom ticks:** Add dashed or shaded threshold bands (`graph.setAxes`), custom labelled tick marks (`graph.setAdditionalTicks`), and vertical reference gridlines (such as null-effect lines on forest plots).
- **Canvas and paper backgrounds:** Set solid or transparent (`None`) plot and page fills (`graph.setStyle` or **Format Graph › Page**). Transparent fills support axis-free foreground overlays and insets in multi-panel layouts.

---

## Annotations, pairwise brackets, Piepho CLD & live TeX equations {#annotations-and-significance}

- **Geometric and text annotations:** Place lines, arrows, rectangles, ellipses, shaded spans, legend positions, and rich text callouts. Text annotations support subscripts, superscripts, Greek Unicode, info constant hooks (`{{Info:Key}}`), and result scalar hooks.
- **Live embedded result tables (`edit.pasteResultTable`):** Paste reactive rectangular cell slices (`selected_rows`, `selected_cols`) from a results sheet onto graphs or page layouts. Styling supports `transpose`, column/row title toggles, bold headers, font size, outer border and gridline widths, and border/fill colours. Embedded tables update via the DAG across RGBA, SVG, PDF 1.7, and `.emf`.
- **Embedded raster images:** Insert content-addressed PNG, JPEG, or TIFF images with `graph.insertImage`, `layout.insertImage`, `edit.pasteImage`, `graph.updateImage`, and `layout.updateImage`. Options include crop insets, borders, `rotation_deg`, and `lock_aspect_ratio`. SVG payloads are rejected and zero remote URLs are fetched. Decoding caps assets at 32 MiB, 16,384 px per side, and 67,108,864 total pixels per asset and per compiled layout.
- **Live TeX best-fit model equations (`graph.addAnnotation` kind `model_equation`, `ui.graph.insertModelEquation`):** Link a graph to a linear regression, nonlinear regression, or Passing–Bablok results dataset. Equations render in symbolic or live coefficient-substituted form and refresh automatically on refit. Typesetting uses two-dimensional Knuth TeXbook Appendix G math boxes (stacked `\frac`/`\dfrac`/`\tfrac`/`\binom`, `\sqrt` with vinculum, stacked super/subscripts, scaled `\left...\right` delimiters, and Greek parameters).
- **Pairwise significance brackets & compact letter display:**
  - **Brackets only:** Draw horizontal or stepped comparison brackets showing stars (`*` to `****` / `ns`), formatted *P*-values, or custom text.
  - **Compact letter display (`graph.setCompactLetterDisplay`):** Computes exact minimal equivalence letter sets (`"a"`, `"ab"`, `"b"`, …) from pairwise significance records using Piepho's (2004) insert-and-absorb and sweeping algorithm. Letters order by descending or ascending mean and sit conflict-free above bars, boxes, or violins.
  - **Brackets and compact letter display:** Renders non-overlapping brackets and compact letters together.

---

## Colour palettes, colormaps & CVD preview {#palettes-and-cvd}

- **Qualitative scientific palettes:** Okabe–Ito (default CVD-safe 8-colour palette), Paul Tol Bright, Paul Tol Muted, Paul Tol High Contrast, Paul Tol Vibrant, and Monochrome Print.
- **Continuous colormaps:** Perceptually uniform Oklab-interpolated ramps (Viridis, Cividis, Plasma, Diverging Blue–Red, Grayscale). Custom ramps accept up to 256 stops and discrete sampling up to 4,096 samples.
- **Live CVD & grayscale preview:** Applies Machado (2009) deuteranopia, protanopia, tritanopia, and grayscale-print (`"grayscale"`) matrices in the canvas viewport. Stored figure colours and exported vector files remain unchanged.

---

## Batch creation & selective style inheritance (`graph.create`, `graph.applyStyleFrom`) {#style-inheritance}

- **One graph per dataset (`graph.create`):** Pass `one_per_data_set: true` (with an optional shared `y_axis_title`) to create one single-series graph per dataset atomically. Works on any data table or transform results sheet (Transpose, Normalize, Select and Transform) and checks the project sheet limit first.
- **Multi-target style cloning (`graph.applyStyleFrom`):** Copies selected styling facets from a donor graph onto one or more `target_sheet_ids` in one undoable step. Selectable facets include axes, palette and series styles, fonts, error bars, legend placement, and canvas dimensions. Each target keeps its own data bindings, series titles, and fitted overlays.

---

## Page layout composition {#page-layouts}

Page layouts compose multi-panel publication figures with up to 64 items per page:

- **Physical units & presets:** Coordinates and margins use points, millimeters, or inches in portrait or landscape orientation. Journal presets cover single-column (85, 88, 89 mm), 1.5-column (120, 130, 136 mm), double-column (174 mm default, 180, 183 mm), full page (including US Letter), 16:9 slide, and poster panel. Resizing can lock exact point typography and line weights or scale proportionally.
- **Arrangement, equalization, Z-order & badges:** Combine live graphs, live result tables, embedded images, shapes, and text headers on one page. Align shared plot origins (`layout.alignOrigins`) and equalize inner plot scales within 0.25 pt (`layout.equalizeScaling`: match smallest, match largest, or explicit percent). Control Z-order stacking with `layout.reorderPanel` (`bring_to_front`, `bring_forward`, `send_backward`, `send_to_back`). Transparent overlays (`paper_fill: None`, hidden ticks/labels) and automatic panel badges (uppercase `"A", "B", …`, lowercase `"a", "b", …`, or none) are built in.

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

- **SVG:** Supports `pretty`, `include_xml_declaration`, `include_metadata`, `outline_text` glyph paths, and coordinate `precision`. `embed_fonts` embeds runtime TrueType faces when available.
- **PDF 1.7:** Supports `compress_streams`, Core-14 WinAnsi + Adobe Symbol text, embedded TrueType fonts, or `outline_text` vector glyphs. Includes alpha transparency, sRGB/CMYK/Grayscale colour spaces, and physical `trim_pt` and `bleed_pt` boxes.
- **PNG & TIFF:** Uses 4×4 subpixel coverage antialiasing (0.15 px Bézier flatness tolerance) and embeds PNG `pHYs` and TIFF `dpi` metadata. TIFF supports uncompressed, LZW, or Deflate compression. Aspect ratio is preserved strictly; oversize uniform DPI targets return an explicit limit error rather than stretching.
- **Windows EMF:** Emits `MM_ANISOTROPIC` mapping with 0.01 mm frame coordinates and 20 twips/pt logical precision. Supports cubic Bézier curves, Unicode text records, and optional BGRA DIB raster fallback.
