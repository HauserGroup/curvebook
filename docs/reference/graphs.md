# Graphs, Page Layouts & Export Capabilities

CurveBook compiles every graph specification (`GraphSheet`) and multi-panel composition (`LayoutSheet`) into a device-independent vector intermediate representation (`ChartScene`, `crates/chart`), guaranteeing identical geometry across desktop canvas rendering, SVG, `%PDF-1.7`, PNG/TIFF rasterization, and ECMA-234 Enhanced Metafiles (`.emf`).

See [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), and [Analyses Reference](analyses.md) for graph/layout command parameters.

---

## Graph Families & Accepted Table Types (`GraphKind`) {#graph-families}

CurveBook exposes eight `GraphKind` families (`crates/doc/src/graph.rs`), each bound to compatible `DataSheet` or `ResultSheet` inputs:

| `GraphKind` Family | Accepted Sources | Built-In Visual Representations (`GraphicalDataObject` / Layout) |
|---|---|---|
| `Xy` | `Xy`, `MultipleVariables` | Scatter symbols, connecting lines (`PointToPoint`, `Staircase`, Fritsch–Carlson / Steffen `MonotonicCubicSpline`, `AkimaSpline`), error envelopes, fitted curve & confidence/prediction band overlays (`FittedCurveOverlay`), AUC shading, before-after connections. |
| `Column` | `Column`, `Nested` | Column/vertical or horizontal bars with error bars, superimposed replicate dots (`DeterministicBeeswarm` or `AlignedColumn`), mean/median summary lines, before-after paired trajectories, box-and-whisker (`Tukey1_5Iqr`, `MinToMax`, `Percentile10To90`, `Percentile5To95`, `Percentile2_5To97_5`, `Percentile1To99`), kernel violin plots, and truncated/truncated+raw combinations. |
| `Grouped` | `Grouped`, `Nested` | Grouped bars/boxes/violins/points arranged via `GroupedArrangement` (`Interleaved`, `Separated`, `Stacked`, `Superimposed`) for two-way and three-way designs. |
| `Contingency` | `Contingency` | Grouped count bars, stacked count bars, and `100%` normalized fraction stacked bars (`PartsOfWholeLayout` / categorical layout). |
| `PartsOfWhole` | `PartsOfWhole` | `Pie`, `Donut` (annular cutout with optional center total), `StackedFractionBar` (`100%` horizontal bar), and `Waffle10x10` (`10×10` grid colored by rounded percentage). |
| `Survival` | `Survival`, `Survival` `ResultSheet` | Right-continuous Kaplan–Meier survival step curves or `1 - S(t)` cumulative incidence curves, censored-subject tick marks, Greenwood / log-log confidence bands, median survival crosshairs, and aligned **Number at Risk** tables. |
| `MultipleVariables` | `MultipleVariables` | Bubble/scatter encoding via `MultivariateGraphEncoding` mapping categorical/continuous columns independently to X, Y, `ColorChannelMapping`, `SizeChannelMapping`, or marker symbol/split panels. |
| `Diagnostic(DiagnosticPlotKind)` | `ResultSheet` / specialized inputs | Seven statistical & exploratory plots: `Heatmap` (matrix/distance/correlation with continuous ramp), `Volcano` ($-\log_{10} P$ / adjusted $q$ vs. effect fold change), `BlandAltman` (bias & limits of agreement), `RocCurve` (sensitivity vs. $1 - \text{specificity}$ + diagonal reference), `ForestPlot` (point estimates + CIs, odds/hazard ratios), `ResidualPlot` (observed/predicted residuals vs. fitted/order), and `NormalQqPlot` (sample vs. Blom normal quantiles). |

---

## Series Styling, Error Bars & Overrides {#series-styling}

- **Series Controls (`SeriesSpec`):** Per-dataset visibility, label, colour, marker shape (`MarkerShape` circles/squares/triangles/diamonds/crosses), fill mode (`MarkerFillMode` solid/open/translucent), marker size in points, stroke dash style (`LineDashStyle` solid/dashed/dotted/dash-dot), summary measure (`SummaryMeasure::{Mean, Median, GeometricMean}`), and connecting curve style.
- **Error Bars (`ErrorBarSpec`):** Configurable `ErrorBarMetric`:
  - `Sd` ($\pm 1\text{ SD}$, $n - 1$ denominator)
  - `Sem` ($\pm 1\text{ SEM} = \text{SD}/\sqrt{n}$)
  - `ConfidenceInterval95` (two-sided $95\%$ Student-*t* interval around the sample mean)
  - `InterquartileRange` (asymmetric $[Q_1, Q_3]$ quartiles)
  - `Range` (`[min, max]`)
  - `ExplicitSubcolumns` (pre-summarized bounds read directly from `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr` subcolumns)
  with directional options (`ErrorBarDirection::{Both, UpperOnly, LowerOnly}`) and selectable end caps/stroke patterns. On log scales, non-positive lower error bounds clamp safely to the axis floor and increment `ScaleDiagnostics::log_error_bars_clamped`.
- **Per-Point Overrides (`PointStyleOverride`):** Targets `(series_index, point_index, optional subcol)` (`graph.formatSelectedPoints`) to highlight, restyle, or hide single replicates without mutating upstream cell data; exact `subcol` rules take precedence over wildcard overrides.

---

## Axis Scales, Broken Axes & Frames {#axes-and-scales}

### The Seven Axis Scales (`AxisScale` / `GraphAxisScale`) {#axis-scales}

Configured per axis (`x_axis`, `y_axis`, secondary `y2_axis` in `crates/doc/src/graph.rs`, `crates/chart/src/scales.rs`):

| Scale Variant | Forward Mapping | Valid Domain |
|---|---|---|
| `Linear` | $x$ | All finite real numbers |
| `Log10` | $\log_{10}(x)$ | Strictly positive $x > 0$ |
| `Log2` | $\log_2(x)$ | Strictly positive $x > 0$ |
| `Ln` | $\ln(x)$ | Strictly positive $x > 0$ |
| `Logit(LogitDomain)` | $\ln\!\left(\frac{p}{1 - p}\right)$ (unit `[0, 1]` or `[0, 100]`% domain) | Strictly open interior $0 < p < 1$ (or $0 < p < 100$) |
| `Probit` | $\Phi^{-1}(u / 100)$ | Strictly open percentage interior $u \in (0, 100)$ |
| `Reciprocal` | $1 / x$ (with direction reversal option) | Non-zero $x \ne 0$ |

Observations outside a non-linear axis domain (or non-finite points) are safely omitted and counted in `ScaleDiagnostics` (`non_positive_log_skipped`, `out_of_domain_logit_skipped`, `out_of_domain_probit_skipped`, `zero_reciprocal_skipped`, `non_finite_skipped`) rather than collapsing or panicking. Each segment generates at most `MAX_TICKS_PER_SEGMENT = 256` ticks using `AxisTickFormat` (`Decimal`, `PowerOfTenSuperscript`, `ScientificE`, `Antilog`, `EngineeringSi`, with `TickLabelFormat` adding `Categorical(CategoricalTickSource)`).

### Discontinuous / Broken Axes (`AxisSegment`) {#broken-axes}

Every axis (`AxisSpec::segments`) supports **`1` to `3` contiguous `AxisSegment` ranges** (`MAX_SEGMENTS_PER_AXIS = 3`), each specifying its `[min, max]` interval and proportional visual share, separated by `gap_width_pt` with a selectable `BreakOrnament` (`DoubleSlash`, `Zigzag`, `BlankGap`). Series bars, lines, and points clip cleanly at break boundaries.

### Frame Presets, Thresholds & Additional Ticks {#frames-and-ticks}

- **`FramePreset`:** `OpenL` (default left-and-bottom L frame), `OffsetRangeTufte` (`DEFAULT_AXIS_OFFSET_PT = 6.0` pt offset range-trimmed spines), `PlainBoxed` (4-sided box), `DualYSecondaryRight` (primary left *Y* paired with secondary right `y2_axis`), and `OriginCrossing` (axes intersecting at `(0, 0)`).
- **`AxisThreshold` & `AdditionalTick`:** Horizontal/vertical dashed or shaded threshold bands (`graph.setAxes`) and custom labelled tick marks (`graph.setAdditionalTicks`).

---

## Annotations, Pairwise Brackets, Piepho CLD & Live TeX Equations {#annotations-and-significance}

- **Geometric & Text Annotations (`GraphAnnotation`):** Lines, arrows, rectangles, ellipses, shaded spans, free-form rich text callouts (subscripts/superscripts, Greek Unicode, **Info constant hooks** `{{Info:Key}}` and **Result scalar hooks** expanded automatically), and legend placement (`LegendSpec`).
- **Live TeX Best-Fit Model Equations (`QOL-16` / `R10.19`):** `GraphAnnotation::ModelEquation` (`graph.addAnnotation` kind `model_equation`, `ui.graph.insertModelEquation`) links a graph directly to a `LinearRegression`, `NonlinearRegression`, or `PassingBablok` `ResultSheet` dataset. Supports `EquationDisplayMode::Symbolic` or `EquationDisplayMode::Substituted` (refreshing coefficients automatically on fit recalculation) and renders using two-dimensional Knuth TeXbook Appendix G math box typesetting (`crates/chart/src/equation_box.rs`: stacked `\frac`/`\dfrac`/`\tfrac`/`\binom`, `\sqrt` with vinculum, stacked super/subscripts, scaled delimiters `\left...\right`, Greek parameters).
- **Pairwise Significance Brackets & Compact Letter Display (`PairwiseDisplayMode`):**
  - `BracketsOnly`: Horizontal/stepped comparison brackets (`PairwiseBracketSpec`) with `BracketStyle` and `BracketLabelMode` (stars `*`..`****`/`ns`, formatted *P*-value, or custom text).
  - `CompactLetterDisplayOnly` (`graph.setCompactLetterDisplay`): Computes exact minimal equivalence letter sets (`"a"`, `"ab"`, `"b"`, ...) directly from pairwise comparison significance records using Piepho's (2004) insert-and-absorb plus sweeping algorithm (`CldOrdering::{DescendingMean, AscendingMean}`), placing conflict-free letter annotations above bars/boxes/violins.
  - `BracketsAndCompactLetterDisplay`: Renders both non-overlapping brackets and compact letters simultaneously.

---

## Colour Palettes, Colormaps & CVD Preview {#palettes-and-cvd}

- **Qualitative Scientific Palettes (`PaletteId`, `crates/palette`):** `OkabeIto` (default CVD-safe 8-colour palette), `PaulTolBright`, `PaulTolMuted`, `PaulTolHighContrast`, `PaulTolVibrant`, and `MonochromePrint`.
- **Continuous Colormaps (`ColormapId`):** Perceptually uniform Oklab-interpolated ramps (`Viridis`, `Cividis`, `Plasma`, `DivergingBlueRed`, `Grayscale`) plus custom user stops up to `MAX_CUSTOM_STOPS = 256` and discrete sampling up to `MAX_DISCRETE_SAMPLES = 4,096`.
- **Live CVD & Grayscale Canvas Preview (`QOL-9`):** Applies Machado (2009) physiologically validated colour-vision-deficiency transformation matrices (`CanvasPreviewFilter::{Normal, Deuteranopia, Protanopia, Tritanopia, GrayscalePrint}`, serialized `"grayscale"` for grayscale print) in the viewport rasterizer (`crates/render`) without altering stored graph specs or exported vector files.

---

## Selective Style Inheritance (`graph.applyStyleFrom`) {#style-inheritance}

`graph.applyStyleFrom` (`apply_style_from_selective` in `crates/doc/src/graph.rs`) copies selected styling facets (axis scales/ticks/frame, palette/series styling, fonts, error bar defaults, legend placement, or canvas preset dimensions) from any template or peer `GraphSheet` onto a target graph while keeping the target's data bindings, series titles, and fitted overlays untouched.

---

## Page Layout Composition (`LayoutSpec`) {#page-layouts}

`LayoutSheet` composes publication multi-panel figures (`MAX_LAYOUT_ITEMS = 64` per page):

- **Physical Units & Presets:** Coordinates and margins in `PhysicalUnit` (`Points`, `Millimeters`, `Inches`), supporting `JournalWidthPreset` (`SingleColumn85Mm`, `SingleColumn89Mm`, `OneAndHalfColumn120Mm`, `OneAndHalfColumn136Mm`, `DoubleColumn174Mm` default, `DoubleColumn183Mm`, `LetterPortrait`, etc.), `FigureCanvasPreset` (`SingleColumn88mm`, `ColumnAndHalf130mm`, `DoubleColumn180mm`, `FullPagePortrait`, `Slide16x9`, `PosterPanel`), `PageOrientation` (`Portrait`, `Landscape`), and `CanvasResizeMode` (`KeepTypographyPt` to preserve exact `pt` sizes & line weights when resizing, vs. `ProportionalScale`).
- **Panel Arrangement & Badges:** Embedded `GraphSheet` panels, shared alignment (`layout.alignOrigins` snapping plot-area axes cleanly across rows/columns), decorative shapes/text headers, and automatic `PanelBadgeStyle` (`UpperCaseLetters` `"A", "B", ...`, `LowerCaseLetters` `"a", "b", ...`, or `None`).

---

## Publication Preflight & Export Capabilities & Limits {#export-capabilities}

### Canvas Print Preflight (`graph.preflightCheck`, `QOL-8`) {#preflight}

`preflight_check_scene` inspects compiled `ChartScene` primitives prior to export and emits non-blocking warnings when:

- Any visible text element or subscript/superscript span renders below `5.0 pt` (`min_font_size`)
- Any stroked path renders below `0.25 pt` (`hairline_stroke`)
- Text anchor or bounding box clips outside the canvas or clip rectangle (`text_outside_canvas`)

### Supported Export Formats & Hard Safety Limits {#export-formats}

| Format | Engine Commands | Capabilities & Options | Enforced Limits |
|---|---|---|---|
| **SVG 1.1** (`curvebook-svg`) | `export.svg`, `graph.exportSvg` | Exact cubic/quadratic paths, CSS/attribute styling, `pretty`, `include_xml_declaration`, `include_metadata`, `embed_fonts` (when runtime TTF faces present), `outline_text` glyph paths, coordinate `precision` (`0..=6`, default `3`). Validated structurally via `validate_svg_structure`. | Finite bounds, precision capped at `6` |
| **`%PDF-1.7` Vector** (`curvebook-pdf`) | `export.pdf`, `graph.exportPdf` | Zlib-compressed vector streams (`compress_streams`), selectable Core-14 WinAnsi + Adobe Symbol text or optional embedded TrueType / `outline_text` vector glyphs, alpha ExtGState transparency, sRGB / CMYK / Grayscale colour spaces (`PdfColorSpace`), multi-page compilation, physical `trim_pt` / `bleed_pt` boxes, BiDi RTL reordering. | `MAX_PDF_PAGES = 64` pages, `MAX_PDF_TOTAL_PRIMITIVES = 500,000` primitives, `MAX_PDF_OUTPUT_BYTES = 32 MiB` |
| **PNG & TIFF Raster** (`curvebook-render`) | `export.png`, `export.tiff`, `graph.exportPng` | 4x4 subpixel coverage antialiasing (`BEZIER_FLATNESS_TOL_PX = 0.15`), DPI-based or explicit pixel sizing, physical `pHYs` / TIFF resolution metadata (`dpi`), TIFF compression (`None`, `Lzw`, `Deflate`). Aspect ratio is strictly preserved on DPI scaling; oversize uniform targets return an explicit limit error rather than stretching dimensions. | `MAX_RASTER_DIM_PX = 16,384` px per side, `MAX_RASTER_PIXELS = 67,108,864` total pixels (`8192 × 8192` equivalent) |
| **Windows Enhanced Metafile `.emf`** (`curvebook-render`) | `graph.exportEmf`, `layout.exportEmf` | Clean-room little-endian ECMA-234 vector records (`MM_ANISOTROPIC`, 0.01 mm `rclFrame`, 20 twips/pt logical precision, `EMR_POLYBEZIERTO`, Unicode `EMR_EXTTEXTOUTW`, optional BGRA DIB fallback raster). | Finite positive dimensions fitting 32-bit EMF `nBytes` |
