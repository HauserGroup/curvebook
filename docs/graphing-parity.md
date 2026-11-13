# Scientific Graphing & Page Layouts Parity (`lumencraft-chart`, `lumencraft-svg`, `lumencraft-pdf`)

> Governing scene-compiler & export specification: [`graphing-and-layouts.md`](graphing-and-layouts.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G5`, `G7`, `G11`)
>
> **Status:** Scene compiler (`lumencraft-chart`, `30` tests), scientific typography (`lumencraft-text`, `18` tests), vector SVG (`lumencraft-svg`, `11` tests), vector PDF 1.4+ (`lumencraft-pdf`, `6` tests, **Risk Spike S2 GO**), raster PNG/TIFF (`lumencraft-render`, `8` tests), and engine/CLI E2E pipelines (`e2e_workflows`, `lumencraft-cli`) pass with `0` Clippy warnings. SVG/PDF/raster outputs are structurally verified by automated tests; interactive opening in external GUI viewers was not executed locally.

## Graph Families & Graphical Data Objects
- [x] **XY Graphs:** Scatter points, straight/staircase/spline connecting lines, area-under-curve fills, vertical & horizontal error bars (SD, SEM, $95\%$ CI, IQR, range, custom subcolumns; solid and **dashed/dotted** line styles), smooth fitted curves & confidence/prediction ribbons
- [x] **Column Graphs:** Bar + error bar, bar + deterministic beeswarm/scatter replicate overlay, aligned dot plot with mean/median crossbar, box-and-whisker (Tukey, min–max, percentile whiskers + outliers), violin plot (KDE + quartiles + points + half-violin), floating min–max bar, before–after paired subject lines
- [x] **Grouped Graphs:** Interleaved vs. Separated bars (`Column = Color/Legend`, `Row = X-Axis Cluster`, configurable `Row titles` / `Column titles` categorical axis ticks), stacked bars, grouped scatter/beeswarm + bars, grouped box-and-whisker, grouped violin, superimposed interaction symbols + connecting lines
- [x] **Contingency & Parts of Whole:** Interleaved/stacked count & percentage bars, $100\%$ stacked fraction bar, ring/donut, pie, and $10 \times 10$ unit/waffle grid
- [x] **Survival Graphs:** Kaplan–Meier staircase curve, censored tick marks, shaded/dashed Greenwood $95\%$ CI envelope, and axis-aligned **Number at Risk** table
- [x] **Multiple Variables Graphs (Prism 11.1 parity):** Five-channel bubble/multivariate scatter (`X`, `Y`, `Color` discrete/continuous ramp, `Size` continuous/categorical, point `Label` text) with non-modal `Apply`, group convex hulls & confidence ellipses, plus **categorical $\times$ continuous GDOs** (bars, box-and-whisker, violin, column scatter, summary connecting lines) with independent graph-level categorical ordering
- [x] **Diagnostic & Specialized:** Grouped & correlation heatmaps (with colorbar), Volcano plot ($-\log_{10} p$ / $-\log_{10} q$ vs. effect), Bland–Altman plot, ROC curve, and Forest / parameter-CI plot

## Axes, Nonlinear Scales & Framing
- [x] Scale transforms: Linear, $\log_{10}$, $\log_2$, $\ln$ (with safe non-positive omission & lower error-bar floor clipping), $\text{logit}$, $\text{probit}$ (probability $\%$), and reciprocal ($1/X$)
- [x] Discontinuous / broken axes (2 or 3 segments with independent domain/scale/tick intervals, configurable gap width, and slash/zigzag ornaments)
- [x] Framing presets: open L-frame, Tufte offset range frame, plain boxed, dual Y axes (`Left Y` + `Right Y2`), and origin-crossing axes
- [ ] Major/minor ticks, logarithmic minor cadences, power-of-10 superscripts ($10^{-6}$), antilog labels, additional ticks/gridlines, and shaded threshold bands *(major/minor ticks, log minor cadences, power-of-10 superscripts, antilog labels, gridlines, and shaded `AxisThreshold` zones/labels verified; standalone arbitrary `additional_ticks` arrays not yet added to `AxisSpec`)*

## Annotations, Style Cloning & Page Layouts
- [x] **Pairwise comparison brackets:** Auto-positioned collision-free brackets live-hooked to upstream *p*-values / FDR *q*-values (`****`, `***`, `**`, `*`, `ns`, or APA/NEJM/4-digit numeric *p*)
- [x] Rich scientific typography (`lumencraft-text`): subscripts, superscripts, Greek symbols, italic variables, and live hooked Info/Result constants preserved in titles, legends, and labels
- [x] Geometric annotations: lines, arrows, brackets, rectangles, circles/ovals, callout text boxes, and embedded live Result parameter tables
- [x] Original colorblind-safe palettes (Okabe–Ito, Paul Tol, Viridis, Cividis) + CVD simulation + one-click style cloning (`graph.applyStyleFrom` / "Wand")
- [ ] **Page Layouts:** Multi-panel publication figure composition with plot-origin alignment across panels, automatic panel lettering (`A`, `B`, `C`), physical rulers (mm / in / journal column presets), vector SVG & PDF 1.7 export, and 72–1200 DPI PNG/TIFF export *(multi-panel composition, `layout.alignOrigins`, `A`/`B`/`C` badges, physical units/journal presets, SVG, pure-Rust vector **PDF 1.4+** with Core-14/outlined fallback, and 72–1200 DPI PNG/Deflate/LZW TIFF verified; PDF 1.7 header + subsetted TTF font embedding remain pending)*
