# CurveBook

[![CI](https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml/badge.svg)](https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license-and-privacy)

<p align="center">
  <img src="assets/app-icon/curvebook-win95-2508.png" alt="CurveBook logo: seven coloured curved panes meeting at a central point" width="160">
</p>

**CurveBook** is an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing application written in pure Rust.

All calculations run locally on your computer with **zero telemetry**. You can work interactively in the **desktop application**, automate pipelines with the **command-line runner** (`curvebook-cli`), or analyze data through an **AI assistant** using the built-in Model Context Protocol server (`curvebook-cli mcp`).

- **Website:** [hausergroup.github.io/curvebook](https://hausergroup.github.io/curvebook/)
- **Install & Download Guide:** [Get Started (Which File Do I Need, Install & Verify)](https://hausergroup.github.io/curvebook/get-started.html)
- **Documentation:** [Online Manual](https://hausergroup.github.io/curvebook/manual/) · [Repository Docs](docs/README.md)
- **Downloads:** [Latest Release (GitHub Releases)](https://github.com/HauserGroup/curvebook/releases)
- **Known Issues (Beta):** [Beta Known Issues](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues)

## What You Can Do

- **Structured Data Tables & Formulas:** Enter, paste, or link data across structured table types (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `Parts of Whole`, `Multiple Variables`, and `Nested`), compute live `=` Calculated Variables, and exclude outliers with recorded audit notes.
- **Biostatistics & Curve Fitting:** Run parametric and nonparametric group comparisons, one-way/multi-way ANOVA, REML mixed models, multiplicity adjustments, survival and contingency analyses, ROC, GLMs, PCA/clustering, power & sample size planning, and built-in or custom nonlinear regression with profile-likelihood confidence intervals.
- **Publication Graphs & Page Layouts:** Build customizable scientific graphs and multi-panel Page Layouts, copy editable vector figures directly to the system clipboard, and export to `SVG`, `PDF`, `PNG`, `TIFF`, `EMF`, `CSV`, or `TSV`.
- **Self-Contained Projects:** Save data tables, notes, analyses, graphs, and layouts together in one `.curvebook` file (or Git-friendly `.curvebook.d/` directory), reuse pipelines with `.cbtemplate` templates, and import `.csv`, `.tsv`, `.pzfx`, and `.prism` files.

<p align="center">
  <img src="docs/images/workspace-split-fit.png" alt="CurveBook light theme split workspace showing Project Navigator, Family tree, dose-response XY table, 4PL curve fit with confidence band, and Format Graph inspector" width="840"><br>
  <em>Split workspace (Light theme): Project Navigator, Family lineage tree, XY data grid, live 4PL nonlinear fit, and Format Inspector.</em>
</p>

<p align="center">
  <img src="docs/images/workspace-layout-dark.png" alt="CurveBook dark theme multi-panel Page Layout showing linked dose-response and bar graphs with significance brackets, embedded table, physical rulers, and layout inspector" width="840"><br>
  <em>Page Layout view (Dark theme): multi-panel figure with significance brackets, embedded summary table, physical millimeter rulers, and Page Layout controls.</em>
</p>

## Download & Install

Download CurveBook installers from **[GitHub Releases](https://github.com/HauserGroup/curvebook/releases)** (Beta releases appear as prereleases; every desktop installer bundles both `curvebook` and `curvebook-cli`).

- **Which File Do I Need, Install Steps & Checksums:** Follow **[Get Started on the CurveBook website](https://hausergroup.github.io/curvebook/get-started.html)** for platform installer selection, OS setup instructions (including macOS `PATH` setup for `curvebook-cli` and Windows SmartScreen guidance), and SHA-256 / Sigstore download verification (`SHA256SUMS.txt` and `gh attestation verify`).
- **Known Issues & Platform Caveats:** Check **[Known Issues (Beta)](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues)** before installing.

## Your First Analysis

You can run your first analysis directly in the desktop window or through an AI assistant connected over MCP.

### 1. In the Desktop Application (`curvebook`)

1. Launch **CurveBook**. The **Welcome & New Data Table** screen opens automatically (`File › New Project` or **New** on the toolbar reopens it anytime).
2. Choose a data table type on the left (such as **Column** or **XY**), or switch to the **Tutorial Sample Data** tab to start with a ready-made dataset.
3. Click **Create Table** (or **Open Sample Dataset**), then click or double-click cells in the grid to enter or paste (`Cmd/Ctrl+V`) measurements.
4. Click **Analyze** on the toolbar (or **Analyze › Analyze…**, `Cmd/Ctrl+Shift+A`), select a test or curve fit (for example **t tests (Unpaired / Welch / Paired / Nonparametric)** or **Nonlinear Regression (Curve Fitting)**), and confirm with **OK**.
5. Inspect the live Results sheet and linked Graph, customize appearance in the right-hand **Format Inspector**, copy vector figures with `Cmd/Ctrl+C`, and save or export via **File › Save** (`.curvebook`) or **File › Export…** (`SVG`, `PDF`, `PNG`, `TIFF`).

### 2. Through an AI Assistant (MCP)

Follow the step-by-step **[Beginner's Guide to using CurveBook with your AI assistant](docs/mcp-for-beginners.md)** to connect `curvebook-cli mcp --workdir <your-data-folder>` to Claude Desktop or another local MCP client, run a Welch's *t*-test by asking in plain language, and save your work as a `.curvebook` project.

For MCP framing details, attaching to a shared desktop window (`curvebook --control <port> --window`), `--gui-only` mode, and troubleshooting, see [`docs/mcp.md`](docs/mcp.md).

## Documentation, Known Issues & Getting Help

- **Online Manual & Reference:** [hausergroup.github.io/curvebook/manual/](https://hausergroup.github.io/curvebook/manual/)
- **Repository Documentation Index:** [`docs/README.md`](docs/README.md)
- **Known Issues & Release Notes:** [Beta Known Issues](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues) (historical Alpha notes: [`docs/history/release/alpha-known-issues.md`](docs/history/release/alpha-known-issues.md))
- **Report a Bug or Numerical Issue:** Open a [Bug Report](.github/ISSUE_TEMPLATE/bug_report.yml), [Numerical Result Report](.github/ISSUE_TEMPLATE/numerical_result.yml), or [Feature Request](.github/ISSUE_TEMPLATE/feature_request.yml) via GitHub Issues (avoid attaching unpublished or confidential research data).
- **Security Vulnerabilities:** Report privately following [`SECURITY.md`](SECURITY.md).

## License and Privacy

- **Privacy & Zero Telemetry:** Read the full [Privacy Policy](https://hausergroup.github.io/curvebook/privacy.html). CurveBook collects **zero telemetry, analytics, or crash reports**, makes **zero outbound network connections**, and includes **no auto-updater** (`cargo xtask telemetry`).
- **License:** Dual-licensed under **`MIT OR Apache-2.0`** ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)).
- **Clean-Room Development:** CurveBook is implemented independently in pure Rust from published statistical literature and public file-format specifications, without copying proprietary commercial software code, equations, wording, templates, or assets, and without copyleft (`GPL`/`AGPL`/`LGPL`) dependencies. Third-party notices and asset attributions live in [`ASSETS.md`](ASSETS.md), [`ATTRIBUTION.md`](ATTRIBUTION.md), [`NOTICE`](NOTICE), and [`docs/licences.md`](docs/licences.md).

## Contributing & Building from Source

Want to build CurveBook from source, run the workspace test and oracle suites, or submit a pull request?

- Start with [`CONTRIBUTING.md`](CONTRIBUTING.md), [`docs/development.md`](docs/development.md), and [`AGENTS.md`](AGENTS.md).
- Contributor credits are maintained in [`docs/contributors.md`](docs/contributors.md).
