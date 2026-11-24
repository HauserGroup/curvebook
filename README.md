<div align="center">
  <img src="assets/app-icon/curvebook-win95-2508.png" alt="CurveBook logo" width="128" />
  <h1>CurveBook</h1>
  <p><strong>Pure-Rust scientific graphing, statistics, nonlinear curve fitting, and publication layout workbench — desktop GUI &amp; local AI automation (MCP) with zero telemetry.</strong></p>

  <p>
    <a href="https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/HauserGroup/curvebook/ci.yml?branch=main&style=flat-square&label=CI" alt="CI" /></a>
    <img src="https://img.shields.io/badge/Status-Beta-D96240?style=flat-square" alt="Status: Beta" />
    <img src="https://img.shields.io/badge/Rust-Pure%20Rust-B7410E?style=flat-square" alt="Pure Rust" />
    <img src="https://img.shields.io/badge/Platforms-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux-374151?style=flat-square" alt="macOS · Windows · Linux" />
    <a href="https://hausergroup.github.io/curvebook/privacy.html"><img src="https://img.shields.io/badge/Telemetry-Zero-059669?style=flat-square" alt="Zero Telemetry" /></a>
    <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-2563EB?style=flat-square" alt="MIT OR Apache-2.0" /></a>
  </p>

  <p>
    <a href="#download-and-install">Download</a> &middot;
    <a href="https://hausergroup.github.io/curvebook/manual/">Manual</a> &middot;
    <a href="https://hausergroup.github.io/curvebook/manual/v0.1/get-started.html">Get Started</a> &middot;
    <a href="#first-analysis">First Analysis</a> &middot;
    <a href="docs/architecture.md">Architecture</a> &middot;
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>
</div>

<p align="center">
  <img src="docs/images/workspace-split-fit.png" alt="CurveBook split comparison workspace showing a structured XY table beside a reactive sigmoidal nonlinear regression plot with confidence bands" width="900" />
  <br />
  <sub><em>Reactive Split Comparison workspace: edit structured dose-response tables side by side with live nonlinear fits and confidence bands.</em></sub>
</p>

<table>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/workspace-layout-dark.png" alt="CurveBook Dark theme LayoutSheet composer assembling two linked figure panels with significance brackets, embedded summary data table, and physical millimeter rulers" width="100%" />
      <br />
      <sub><em>Dark theme multi-panel <code>LayoutSheet</code> composer arranging linked figures and result annotations at publication page dimensions.</em></sub>
    </td>
    <td align="center" width="50%">
      <img src="docs/images/workspace-anova-results.png" alt="CurveBook Light theme split workspace showing multi-tab One-Way ANOVA results, QOL-7 assumption diagnostic banner and Interpret inspector, and linked Column graph with Compact Letter Display and significance brackets" width="100%" />
      <br />
      <sub><em>One-Way ANOVA workbench: multiple comparisons, non-blocking <code>QOL-7</code> assumption diagnostics, and linked graph with Compact Letter Display and significance brackets.</em></sub>
    </td>
  </tr>
</table>

---

## Capabilities

CurveBook replaces proprietary black-box spreadsheet and curve-fitting silos with a single reactive Rust workspace where data tables, analyses, graphs, and page layouts stay linked by dependency (`Family`), reproducible from headless scripts (`curvebook-cli`), and accessible to local AI assistants via MCP.

| Area | What Is Included |
|---|---|
| **Data & Structured Tables** | Domain-typed tables (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `Parts of Whole`, `Nested`, `Multiple Variables`), replicate subcolumns, non-destructive exclusions, live selection summary bar (`n`, `mean`, `SD`, `SEM`, `median`, `sum`), deterministic transforms, baseline subtraction, simulated distributions, and symbolic calculated columns (`FX=`). |
| **Statistics & Curve Fitting** | Descriptive summaries, *t*-tests and nonparametric comparisons, ordinary, repeated-measures, and mixed-effects ANOVA with multiple-comparison post tests, contingency and Kaplan-Meier / Cox survival modeling, generalized linear models, multivariate methods (PCA, PCR, k-means, hierarchical clustering), weighted nonlinear curve fitting with `AICc` and extra sum-of-squares *F* model comparison, assumption diagnostics, and power/sample-size planning. |
| **Publication Graphs & Layouts** | Publication-quality scatter, bar/box/violin, before-after, Kaplan-Meier survival, forest, population pyramid, heatmap, bubble, QQ, and correlation figures with error bars, pairwise brackets, compact letter display, rendered TeX equations, inset overlays, live embedded result tables, high-resolution images, colorblind-safe palettes (`Okabe-Ito`, `Viridis`, `Cividis`), multi-panel `LayoutSheet` composition, and vector/raster export (`SVG`, `PDF`, `PNG`, `TIFF`, `EMF`, multi-flavor clipboard). |
| **Desktop GUI, CLI & Local MCP** | Declarative Rust desktop shell (`Light`, `Dark`, and `HighContrast` OKLab/OKLCH themes, Split Comparison, Family pipeline cloning, dark/CVD preview), headless `curvebook-cli` batch runner and importer/exporter (`.curvebook`, `.cbtemplate`, `.pzfx`, `.prism`, `.csv`/`.tsv`), crash-safe write-ahead recovery, and local stdio Model Context Protocol (`curvebook-cli mcp`) integration. |

---

## Download and Install

Beta installers ship from **[GitHub Releases](https://github.com/HauserGroup/curvebook/releases)**. Every release archive is paired with `SHA256SUMS.txt` and keyless GitHub Sigstore build provenance (`actions/attest-build-provenance`).

| Platform | Beta Release Asset | Notes |
|---|---|---|
| **macOS** (`arm64` + `x86_64`) | `curvebook-<version>-macos-universal.dmg` | Signed and notarized `.dmg` bundling `CurveBook.app` and `curvebook-cli`. |
| **Windows** (`x64`, `arm64`) | `curvebook-<version>-windows-<arch>.msi` | Dual-scope installer (`curvebook.exe` + `curvebook-cli.exe`); ships **unsigned** in Beta — review [SmartScreen guidance](https://hausergroup.github.io/curvebook/manual/v0.1/troubleshooting.html#known-issues). |
| **Linux** (`x86_64`, `aarch64`) | `curvebook-<version>-linux-<arch>.AppImage` | Portable desktop bundle including `curvebook` and `curvebook-cli`, tested with Wayland and X11 portals. |
| **FreeBSD** (`x86_64`) | `curvebook-<version>-freebsd-x86_64.tar.gz` | Tier 2 CLI-only archive (`curvebook-cli` + local MCP; no desktop GUI). |

- **Installation, PATH setup, and SHA-256 / Sigstore verification:** Follow the **[Get Started Guide](https://hausergroup.github.io/curvebook/manual/v0.1/get-started.html)**.
- **Known Beta issues & workarounds:** Check **[Troubleshooting & Known Issues](https://hausergroup.github.io/curvebook/manual/v0.1/troubleshooting.html#known-issues)** before filing a report.

---

## First Analysis

### In the Desktop App

1. Launch **CurveBook** to open the **Workspace Hub** (`Welcome`), choose a structured table archetype (or start from a tutorial sample dataset or `.cbtemplate`), and enter, paste, or import your data.
2. Click **Analyze…** (`Cmd/Ctrl+T` later edits parameters) to run a statistical comparison or nonlinear curve fit; inspect the reactive `ResultSheet`, assumption guidance, and automatically linked graph.
3. Refine axes, error bars, legends, or layout panels in the Inspector, then save your `.curvebook` project (`File › Save`) or export vector `SVG`/`PDF` and raster `PNG` figures. See the **[Desktop App Guide](https://hausergroup.github.io/curvebook/manual/v0.1/desktop-gui.html)**.

### With a Local AI Assistant (MCP)

Configure your MCP client once to invoke the bundled `curvebook-cli mcp --workdir <folder>` server, ask it in plain English to create tables, fit curves, compare groups, or export figures, and open the resulting `.curvebook` file directly in the CurveBook desktop app to inspect and polish interactively.

Start with the **[Beginner's MCP Guide](docs/mcp-for-beginners.md)** or the full **[MCP Automation Reference](https://hausergroup.github.io/curvebook/manual/v0.1/automation/mcp.html)**.

---

## Privacy, Clean-Room Engineering, and License

- **Zero Telemetry & Offline First:** CurveBook performs zero analytics, zero crash reporting, zero update checks, and zero outbound network calls (`cargo xtask telemetry`). See the **[Privacy Policy](https://hausergroup.github.io/curvebook/privacy.html)** and **[Privacy & Security Reference](https://hausergroup.github.io/curvebook/manual/v0.1/privacy-security.html)**.
- **Safe Rust & Clean-Room Numerical Stack:** Written from scratch in safe Rust (`#![forbid(unsafe_code)]`) from peer-reviewed statistical and numerical literature (`cargo xtask cleanroom`, `cargo xtask brands`). Third-party source code attributions and crate licences live in [`NOTICE`](NOTICE), [`ATTRIBUTION.md`](ATTRIBUTION.md), and [`docs/licences.md`](docs/licences.md).
- **License:** Dual-licensed under **[MIT](LICENSE-MIT)** OR **[Apache-2.0](LICENSE-APACHE)** at your option.

---

## Documentation, Help, and Contributing

- **User Manual:** [Online Documentation](https://hausergroup.github.io/curvebook/manual/) · [Documentation Index](docs/README.md) · [Troubleshooting & Known Issues](https://hausergroup.github.io/curvebook/manual/v0.1/troubleshooting.html#known-issues)
- **Bug Reports & Feature Requests:** [GitHub Issues](https://github.com/HauserGroup/curvebook/issues)
- **Contributors:** Read [`CONTRIBUTING.md`](CONTRIBUTING.md), [`AGENTS.md`](AGENTS.md), [`docs/development.md`](docs/development.md), and [`docs/architecture.md`](docs/architecture.md).

---

## Disclaimer

CurveBook is an independent open-source project and is not affiliated with, endorsed by, or sponsored by Dotmatics, GraphPad Software, or any other provider of scientific or statistical software. All trademarks and product names are the property of their respective owners. <!-- brand-ok -->
