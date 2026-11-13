# LumenCraft

**LumenCraft** is an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing application written in pure Rust — designed for experimental scientists who work with structured replicate tables, linked statistical analyses, curve fitting, and publication-ready figures on desktop and the web.

> **Status:** Early workspace & infrastructure baseline (**Milestone M0**). The 17-crate layered architecture (`L0`–`L7`), CI quality gates, clean-room/brand/asset verification (`cargo xtask ci`), and cross-platform packaging pipelines (macOS, Linux, Windows, FreeBSD, WebAssembly) are in place; scientific domain engines are under active clean-room development. See [`ROADMAP.md`](ROADMAP.md) and [`docs/target-app-parity.md`](docs/target-app-parity.md).

---

## Planned Capabilities

- **Structured scientific data tables** — Eight table archetypes tailored to experimental designs rather than untyped cells: **XY** (with replicate subcolumns or precomputed SD/SEM/CI), **Column**, **Grouped** (two-way factorial), **Contingency**, **Survival** (time-to-event + censoring), **Parts of Whole**, **Multiple Variables**, and **Nested** (hierarchical subcolumns).
- **Reactive analysis DAG** — Analyses (`t`-tests, one-/two-/three-way ANOVA, mixed-effects models, nonparametric tests, multiplicity corrections, Kaplan–Meier & Cox survival, ROC curves, PCA, and nonlinear regression) link live to upstream data tables and automatically recompute downstream results and graphs when observations change or outliers are excluded.
- **Nonlinear regression & curve fitting** — Dose–response (`log(agonist)` / `log(inhibitor)` 3PL, 4PL, 5PL, biphasic), enzyme kinetics (Michaelis–Menten, competitive/noncompetitive/uncompetitive/mixed inhibition, allosteric Hill), binding saturation & competition, exponential decay/association, polynomial, Gaussian/Lorentzian peaks, user-defined equations, parameter constraints, global shared-parameter fitting, robust regression, AICc / extra-sum-of-squares *F*-test model comparison, and profile-likelihood confidence bands.
- **Publication-grade scientific graphing** — Scatter, line, dose–response curves with confidence/prediction bands, bar + individual replicate points, box-and-whisker, violin, volcano, Bland–Altman, Kaplan–Meier staircase with censored ticks and number-at-risk tables, heatmaps, discontinuous/broken axes, $\log_{10}$ / $\log_2$ / logit / probit scales, automatic pairwise significance brackets (`*`, `**`, `***`, *p*-values), and multi-panel figure **Layouts** exported cleanly to vector **SVG** and **PDF**.
- **Automation-first headless CLI & MCP** — Every action in the GUI dispatches through a serializable engine command registry, exposed headlessly via `lumencraft-cli`, live TCP JSON-lines (`lumencraft --control <port>`), and Model Context Protocol (`lumencraft-cli mcp`) for reproducible pipelines and AI assistants.

---

## Workspace Layout

```text
lumencraft/
├── crates/
│   ├── math/        # L0: IEEE-754 helpers, special functions (beta/gamma/erf), dense linear algebra, RNG
│   ├── palette/     # L0: OkLab/sRGB color math, colorblind-safe scientific palettes, CMYK/print helpers
│   ├── doc/         # L1: Typed data tables, subcolumn schemas, analysis/graph/layout specs, SheetId DAG
│   ├── transform/   # L2: Column transforms, normalization, baseline subtraction, row pruning/binning
│   ├── stats/       # L2: Parametric/nonparametric tests, ANOVA, multiplicity, survival, contingency, ROC
│   ├── fit/         # L2: Levenberg–Marquardt, robust/weighted NLLS, built-in & custom equation solvers
│   ├── text/        # L2: Scientific label layout, sub/superscripts, Greek/math symbols, optional bundled fonts
│   ├── chart/       # L2: Axis scaling, broken axes, error bars, significance brackets, scene compilation
│   ├── render/      # L3: GPU/CPU 2D chart & layout raster/frame renderer
│   ├── format/      # L3: Native .lumencraft / .lctemplate archive + clean-room CSV/TSV/.pzfx/.prism I/O
│   ├── svg/         # L3: Publication SVG serializer
│   ├── pdf/         # L3: Publication PDF 1.7 / print vector writer
│   ├── tools/       # L4: Interactive table selection, graph annotation, bracket & layout tools
│   ├── engine/      # L5: Project session, undo/redo journal, reactive recalculation DAG, command registry
│   ├── ui-egui/     # L6: Native/web immediate-mode desktop shell (Navigator, Data Grid, Results, Graphs)
│   ├── mcp/         # L6: Stdio Model Context Protocol server over lumencraft-engine
│   └── testkit/     # Dev-only numerical oracle assertions (NIST StRD), proptest strategies & builders
├── apps/
│   ├── lumencraft/      # Native desktop app (macOS, Linux, Windows, FreeBSD)
│   ├── lumencraft-cli/  # Headless batch runner, exporter, and MCP server
│   └── lumencraft-web/  # Browser WebAssembly app (Trunk + WebGPU/WebGL2)
├── xtask/           # Workspace quality gates, layer linter, clean-room/brand/asset checks, bundler
├── packaging/       # Signed installers & bundles (macOS DMG, Linux deb/rpm/AppImage/Flatpak, Windows MSI, Web)
└── docs/            # Architecture, parity matrices, numerical verification & release guides
```

---

## Quick Start

Requires **Rust 1.95+** (`edition = "2024"`).

```sh
# Run the full local verification suite (fmt, clippy, unit/doc tests, asset/brand/cleanroom/layer gates)
cargo xtask ci

# Inspect workspace crate layers and line/test statistics
cargo xtask stats

# Run the desktop shell or headless CLI
cargo run -p lumencraft -- --help
cargo run -p lumencraft-cli -- --help
```

---

## Documentation & Implementation Guides

- **Roadmap & Parity:** [`ROADMAP.md`](ROADMAP.md) · [`docs/target-app-parity.md`](docs/target-app-parity.md) · [`docs/gaps.md`](docs/gaps.md) · [`docs/roadmap.md`](docs/roadmap.md) · [`docs/qol-beyond-parity.md`](docs/qol-beyond-parity.md)
- **Architecture & Engineering:** [`docs/architecture.md`](docs/architecture.md) · [`docs/development.md`](docs/development.md) · [`docs/releasing.md`](docs/releasing.md) · [`docs/contributors.md`](docs/contributors.md)
- **Subsystem Implementation Guidelines:**
  - [`docs/data-tables-and-transforms.md`](docs/data-tables-and-transforms.md) — 8 data-table archetypes, subcolumns, exclusions, Info constants, Calculated Variables (`=`), and transforms
  - [`docs/statistics-implementation.md`](docs/statistics-implementation.md) — Special functions, *t*-tests, 1/2/3-way & Multifactor ANOVA, REML mixed models, multiplicity/FDR, survival, contingency, ROC & multivariate GLMs
  - [`docs/curve-fitting-implementation.md`](docs/curve-fitting-implementation.md) — Damped Levenberg–Marquardt, weighting, Cauchy robust & ROUT fitting, global parameters, profile-likelihood CIs, AICc/*F*-tests, equation library & custom equation VM
  - [`docs/graphing-and-layouts.md`](docs/graphing-and-layouts.md) — `GraphSpec` scene compilation, nonlinear/broken axes, live pairwise *p*-value brackets, style cloning, Page Layouts & SVG/PDF/PNG/TIFF export
  - [`docs/scripting-and-automation.md`](docs/scripting-and-automation.md) · [`docs/control-protocol.md`](docs/control-protocol.md) · [`docs/mcp.md`](docs/mcp.md) — `.lctemplate` batch pipelines, CLI, `--control` TCP JSONL, and MCP server
- **Domain Checklists:** [`stats-parity.md`](docs/stats-parity.md) · [`fitting-parity.md`](docs/fitting-parity.md) · [`graphing-parity.md`](docs/graphing-parity.md) · [`ui-parity.md`](docs/ui-parity.md) · [`file-format-parity.md`](docs/file-format-parity.md) · [`hardware-parity.md`](docs/hardware-parity.md) · [`localization-parity.md`](docs/localization-parity.md)

---

## Clean-Room & Licensing

LumenCraft is a **100% clean-room implementation**:
- Statistical and curve-fitting algorithms are derived strictly from peer-reviewed statistical literature, open textbooks, and validated against public reference datasets (such as the **NIST Statistical Reference Datasets (StRD)**).
- No proprietary code, decompiled binaries, XML schemas, icons, equation libraries, or vendor-created sample files are used or committed.
- Dual-licensed under **MIT OR Apache-2.0** ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)). Dependency policy and third-party notices live in [`ATTRIBUTION.md`](ATTRIBUTION.md), [`NOTICE`](NOTICE), and [`ASSETS.md`](ASSETS.md).
