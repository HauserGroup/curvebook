# Architecture

LumenCraft is organized as a strictly layered Rust workspace (`L0` through `L7`). `cargo xtask layers` parses `cargo metadata` on every CI run and rejects upward dependencies, undeclared sideways dependencies, `lumencraft-testkit` outside `[dev-dependencies]`, external suite crates (`craft-*`), and any UI/windowing crate (`egui`, `eframe`, `winit`, `rfd`) below `L6`.

```mermaid
flowchart TD
    L7["L7 Apps: lumencraft, lumencraft-cli, lumencraft-web"]
    L6["L6 Front-Ends: ui-egui, mcp"]
    L5["L5 Session & Reactive DAG: engine"]
    L4["L4 Interactive Tools: tools"]
    L3["L3 Rendering & File I/O: render, format, svg, pdf"]
    L2["L2 Scientific Engines: transform, stats, fit, text, chart"]
    L1["L1 Pure Project Model: doc"]
    L0["L0 Foundations: math, palette"]

    L7 --> L6
    L6 --> L5
    L5 --> L4
    L4 --> L3
    L3 --> L2
    L2 --> L1
    L1 --> L0
```

## Layers

### L0 — Numerical & Color Foundations
- **`lumencraft-math`**: Pure IEEE-754 numerical primitives, compensated summation (Kahan/Neumaier), special functions (`ln_gamma`, regularized incomplete beta $I_x(a,b)$ and gamma $P(a,x)$, `erf`/`erfc`, Student *t*, Snedecor *F*, $\chi^2$, normal CDF/quantile), QR/SVD/Cholesky dense linear algebra for design matrices and Jacobians, and deterministic PRNG helpers for Monte Carlo / permutation tests.
- **`lumencraft-palette`** *(may depend on `math`)*: Perceptual color spaces (sRGB, linear RGB, OkLab/OkLCH), print CMYK conversion, colorblind-safe scientific palettes (Okabe–Ito, Viridis, Cividis, Paul Tol), and color-vision-deficiency simulation matrices.

### L1 — Pure Document & Sheet Model
- **`lumencraft-doc`**: Immutable/serializable value types representing a LumenCraft project:
  - **Data sheets** across the eight experimental table archetypes (`Xy`, `Column`, `Grouped`, `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`, `Nested`), typed cells (numeric `f64`, censored/missing states, excluded-value masks, text/categorical columns with role-inference and level-ordering metadata), replicate subcolumn configurations (`N` raw replicates vs. pre-summarized `Mean + SD/SEM/N` or `Mean + Upper/Lower error`), and provenance-tracked **`LinkedDataRegion`** bindings (internal cross-table links, reactive transpose slices, and external file links).
  - **Info sheets & Floating notes** (structured laboratory metadata, constants hookable into equations/titles, freeform notes, plus collapsible `FloatingNote` callouts attachable to any sheet).
  - **Analysis specs & Result sheets** (parameterized statistical or curve-fitting specifications referencing upstream data/result sheets by stable `SheetId`, auto-derived vs. user-pinned sheet titles, and structured tabular/diagnostic outputs).
  - **Graph specs & Layout specs** (first-visit initialization state, data-series bindings, axis models, error-bar styles, regression curve overlays, pairwise comparison brackets, annotations, and multi-graph + live-table publication page layouts).

### L2 — Scientific Domain Engines
- **`lumencraft-transform`**: Data transformations ($Y = \log_{10}(Y)$, Hill/logit/probit transforms, baseline subtraction, `[0, 100%]` normalization, transpose, pruning, and binning) producing derived result tables.
- **`lumencraft-stats`**: Parametric and nonparametric hypothesis tests, descriptive statistics, normality/lognormality tests, outlier detection (Grubbs, ROUT), one-/two-/three-way ANOVA, mixed-effects REML, multiple-comparison procedures (Dunnett, Tukey, Šídák, Bonferroni, Holm–Šídák, Benjamini–Hochberg/Krieger–Yekutieli FDR), contingency table analyses, Kaplan–Meier & Cox survival analysis, ROC curves, and multivariate methods (multiple linear/logistic/Poisson regression, PCA).
- **`lumencraft-fit`**: Nonlinear and linear least-squares / robust regression engine: damped Levenberg–Marquardt with analytical or forward/central-difference Jacobians, differential weighting ($1/Y$, $1/Y^2$, $1/X$, Poisson), Huber/Cauchy/ROUT biweight robust loss, box/equality/shared global parameter constraints, AICc and extra-sum-of-squares *F* model comparison, replicate test for lack of fit, confidence/prediction bands, and asymptotic vs. profile-likelihood confidence intervals.
- **`lumencraft-text`**: Scientific text shaping and measurement (axis tick notation $10^{-6}$, inline superscripts/subscripts, Greek symbols, optional embedded SIL OFL fonts via `LUMENCRAFT_FONTS_DIR` gated behind the `bundled-fonts` Cargo feature).
- **`lumencraft-chart`** *(may depend on `transform`, `stats`, `fit`, `text`)*: Compiles a `GraphSpec` + resolved upstream `DataSheet`/`ResultSheet` data into a resolution-independent 2D vector scene graph (axes, ticks, discontinuous axis breaks, data series, error bars, box/violin density kernels, fitted curve polylines/Béziers, confidence ribbons, legends, and significance brackets).

### L3 — Rendering & Persistence
- **`lumencraft-render`**: Deterministic CPU rasterizer and compositor for compiled chart scenes and multi-panel page layouts (PNG/TIFF exports and golden snapshots; native GPU `wgpu` backend queued in `G6`).
- **`lumencraft-format`**: Reads and writes the native `.lumencraft` and `.lctemplate` ZIP-based project containers (versioned manifest + structured JSON/columnar tables) and provides clean-room importers/exporters for CSV, TSV, and third-party `.pzfx` / `.prism` scientific files.
- **`lumencraft-svg`**: Lossless vector SVG serializer for single graphs and multi-panel layouts (preserving editable `<text>` or optional outlined glyphs).
- **`lumencraft-pdf`**: Direct PDF 1.7 vector writer for journal submission figures and print layouts.

### L4 — Interactive Tools
- **`lumencraft-tools`**: Stateful pointer/keyboard interaction controllers for spreadsheet range selection, data brushing/exclusion, graph object dragging, significance-bracket drawing, axis-break placement, and page-layout alignment guides, plus the `Cursor` catalog verified against `crates/tools/src/cursors.toml`.

### L5 — Engine & Reactive Recalculation
- **`lumencraft-engine`**: Owns the live `Session`, undo/redo transaction log, topological dependency graph (`DataSheet -> LinkedDataRegion / Transform / Analysis -> ResultSheet -> Graph -> Layout`), incremental dirty-node invalidation and recalculation, in-place analysis parameter updates and cross-family dataset retargeting (`Change Data Analyzed`), and the JSON-serializable command dispatcher shared identically by the GUI, CLI, `--control` TCP server, and MCP server.

### L6 — Thin Front-Ends
- **`lumencraft-ui-egui`**: Pure-Rust `UiApp` state shell, `17` `UI_COMMANDS`, `16`-locale/RTL i18n, and deterministic software RGBA/PNG compositor (`render_workspace_rgba` / `render_workspace_frame_png`: Navigator sidebar, Family pane, virtualized subcolumn grid, Analysis/Result/Graph/Format inspectors, Page Layout composer, and `--control` JSONL server). Contains zero statistical math; every mutation dispatches an engine command. Live native OS `eframe`/`egui`/`wgpu` desktop & `wasm32` UI hosting is queued in `G6`.
- **`lumencraft-mcp`**: Headless Model Context Protocol server over stdio exposing project inspection, data entry, statistical analysis execution, curve fitting, and SVG/PDF/PNG figure rendering to AI coding and research agents.

### L7 — Executables & Workspace Tooling
- **`apps/lumencraft`**: Native desktop binary (`macOS`, `Linux`, `Windows`, `FreeBSD`).
- **`apps/lumencraft-cli`**: Headless batch CLI, format converter, analysis runner, and `mcp` subcommand host.
- **`apps/lumencraft-web`**: Browser WebAssembly SPA crate (`init_web_app` tested on host; live `wasm32-unknown-unknown` Trunk + WebGPU/WebGL2 compilation queued in `G6`, external `wasm32` sysroot required).
- **`xtask`**: Pure-`std` workspace task runner enforcing formatting, clippy, tests, layer rules, asset attribution, clean-room checks, brand hygiene, UI command catalog sync, and macOS `.app` bundling.
