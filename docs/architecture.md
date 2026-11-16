# Architecture

LumenCraft is organized as a strictly layered Rust workspace (`L0` through `L7`). `cargo xtask layers` parses `cargo metadata` on every CI run and rejects upward dependencies, undeclared sideways dependencies, `lumencraft-testkit` outside `[dev-dependencies]`, external suite crates (`craft-*`), and any UI/windowing crate (`egui`, `eframe`, `winit`, `rfd`, `bevy*`) below `L6`; it also rejects raw process spawning outside `crates/platform/src/`.

```mermaid
flowchart TD
    L7["L7 Apps: lumencraft, lumencraft-cli, lumencraft-web"]
    L6["L6 Front-Ends: ui-egui, mcp"]
    L5["L5 Session & Reactive DAG: engine"]
    L4["L4 Interactive Tools: tools"]
    L3["L3 Rendering & File I/O: render, format, svg, pdf"]
    L2["L2 Scientific Engines: transform, stats, fit, text, chart"]
    L1["L1 Pure Project Model: doc"]
    L0["L0 Foundations: math, palette, platform"]

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
- **`lumencraft-platform`**: OS shims kept out of the headless core: atomic temp-file + sync + rename writes (`fs.rs`), bounded subprocesses with cleaned environments (`process.rs`), and `WorkdirGuard` / `PathResolver` path confinement (`path_guard.rs`).

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
- **`lumencraft-render`**: Deterministic CPU rasterizer and compositor for compiled chart scenes and multi-panel page layouts (PNG/TIFF exports and golden snapshots). The desktop and web hosts upload this CPU RGBA output as a texture; there is no GPU rendering backend.
- **`lumencraft-format`**: Reads and writes the native `.lumencraft` and `.lctemplate` ZIP-based project containers and the unpacked `.lumencraft.d/` directory form (versioned manifest + structured JSON/columnar tables, `MIGRATION_STEPS` upgrades), the `ops.journal` crash-recovery journal (`recovery.rs`), and clean-room importers/exporters for CSV, TSV, and third-party `.pzfx` / `.prism` scientific files, all registered in `FormatRegistry` (`registry.rs`).
- **`lumencraft-svg`**: Lossless vector SVG serializer for single graphs and multi-panel layouts (preserving editable `<text>` or optional outlined glyphs).
- **`lumencraft-pdf`**: Direct PDF 1.7 vector writer for journal submission figures and print layouts.

### L4 — Interactive Tools
- **`lumencraft-tools`**: Stateful pointer/keyboard interaction controllers for spreadsheet range selection, data brushing/exclusion, graph object dragging, significance-bracket drawing, axis-break placement, and page-layout alignment guides, plus the `Cursor` catalog verified against `crates/tools/src/cursors.toml`.

### L5 — Engine & Reactive Recalculation
- **`lumencraft-engine`**: Owns the live `Session`, undo/redo transaction log, topological dependency graph (`DataSheet -> LinkedDataRegion / Transform / Analysis -> ResultSheet -> Graph -> Layout`), incremental dirty-node invalidation and recalculation, in-place analysis parameter updates and cross-family dataset retargeting (`Change Data Analyzed`), and the JSON-serializable command dispatcher shared identically by the GUI, CLI, `--control` TCP server, and MCP server.

### L6 — Thin Front-Ends
- **`lumencraft-ui-egui`**: Pure-Rust `UiApp` state shell, `UI_COMMANDS` registry (`menus/ui_commands.rs`), multi-locale/RTL i18n (`i18n.rs`), single retained `LayoutTree` (`layout/{mod.rs,a11y.rs,scroll.rs,canvas_viewport.rs,widgets.rs,hit_test.rs,paint.rs,verify.rs}`), view facade (`views/mod.rs`), L6 `TextMeasurer` & 4x4 coverage-mask glyph atlas (`text_bridge.rs`, retiring `FONT_5X7`), unified input dispatch (`interaction.rs`, `gui_actions.rs`), `LayoutTree`-projected `ui.inspect` (`inspect.rs`), `HumanPathDriver` (`human_driver.rs`), `--control` JSONL server (`control_io.rs`), and deterministic software RGBA/PNG compositor (`compositor.rs`: `render_workspace_rgba` / `render_workspace_frame_png`). Contains zero statistical math; every mutation dispatches an engine command, and `egui` widgets are never used for workspace chrome.
  - **Single Retained Layout & Text Boundary (`R10.1`, `R10.9`, `R10.15`, `R10.16`, `R10.2`, `R10.22`):** `UiApp::layout_tree_cow` builds and caches one retained `LayoutTree` keyed by viewport metrics and state fingerprint. Software rasterization (`paint.rs`), physical/logical hit-testing (`hit_test.rs`), modal `ModalBackdrop` z-order pointer capture & `Tab`/`Shift+Tab` focus trapping (`a11y.rs`), scrollable `ScrollMeta` containers (`scroll.rs`), aspect-ratio paper mapping (`canvas_viewport.rs`), and `inspect_ui_tree` all consume that single `LayoutTree` (`ui.inspect` never synthesizes unrendered placeholder bounds). Desktop and headless runs shape and rasterize glyphs via `DesktopTextMeasurer` (`FontBook` + `LUMENCRAFT_FONTS_DIR` + `lumencraft-text` outlines behind the `TextMeasurer` trait), keeping web `<canvas>` text integration cleanly pluggable (`R10.7`). Anti-phantom `verify_pixel_to_hit_box_parity` + `verify_click_action_parity`, `HumanPathOnly` policy guards, the `R10.1` human-path ANOVA spike, and `R10.22` frame (`<= 16.67 ms` on `1M`-cell grid) / input-to-paint (`<= 50 ms`) budgets are gated in `crates/ui-egui/tests/r10_core_and_spike.rs` and `cargo xtask ci`.
- **`lumencraft-mcp`**: Headless Model Context Protocol server over stdio exposing project inspection, data entry, statistical analysis execution, curve fitting, and SVG/PDF/PNG figure rendering to AI coding and research agents, plus the `gui` toolset (`gui_tools.rs`) that drives `UiApp` in process or over a `--control` TCP session (see [mcp.md](mcp.md)).

### L7 — Executables & Workspace Tooling
- **`apps/lumencraft`**: Native desktop binary (`macOS`, `Linux`, `Windows`, `FreeBSD`): `DesktopHostRuntime` (`desktop_host.rs`) owns the `UiApp` session and event routing, and `gui_window.rs` (`run_native_desktop`) opens an `eframe` window with a native `egui` menu bar and `rfd` file dialogs, presenting the compositor's RGBA frame as a single texture (`R8.3`).
- **`apps/lumencraft-cli`**: Headless batch CLI, format converter, analysis runner, and `mcp` subcommand host.
- **`apps/lumencraft-web`**: Browser WebAssembly crate (`init_web_app`, `connect_wasm_bridge`, `host_adapters.rs`) built with Trunk for `wasm32-unknown-unknown`; it blits the compositor's RGBA frame to a WebGL2 `<canvas>` (2D-context fallback) and has no HTML menu bar or toolbar yet (`R8.4`; web chrome is `R10.7`).
- **`xtask`**: Workspace task runner (depends on `lumencraft-engine`, `lumencraft-format`, `lumencraft-testkit`, and `serde_json` for codegen) enforcing formatting, clippy, tests, layer rules, asset attribution, clean-room checks, brand hygiene, telemetry/secrets/licence gates, generated-doc sync (`commands`, `schemas`, `ui-commands`), and macOS `.app` bundling. Run `cargo xtask --help` for the full list.
