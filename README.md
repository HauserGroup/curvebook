<div align="center">
  <img src="assets/app-icon/curvebook-win95-2508.png" alt="CurveBook logo" width="120" />
  <h1>CurveBook</h1>
  <p><strong>Free, open-source scientific graphing, biostatistics, and curve fitting.</strong></p>
  <p>Analyze your data, fit curves, and build publication-ready figures—100% offline with zero telemetry.<br />Opens <code>.pzfx</code>, <code>.prism</code>, and <code>.csv</code> files on macOS, Windows, and Linux.</p>

  <p>
    <a href="https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/HauserGroup/curvebook/ci.yml?branch=main&style=flat-square&label=CI" alt="CI" /></a>
    <img src="https://img.shields.io/badge/Status-Beta-D96240?style=flat-square" alt="Status: Beta" />
    <a href="https://curvebook.org/privacy.html"><img src="https://img.shields.io/badge/Telemetry-Zero-059669?style=flat-square" alt="Zero Telemetry" /></a>
    <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-2563EB?style=flat-square" alt="MIT OR Apache-2.0" /></a>
  </p>

  <p>
    <a href="#download">Download</a> &middot;
    <a href="https://curvebook.org/manual/">User Manual</a> &middot;
    <a href="https://curvebook.org/manual/latest/get-started.html">Get Started</a> &middot;
    <a href="https://curvebook.org/faq.html">FAQ</a> &middot;
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>
</div>

<p align="center">
  <img src="docs/images/workspace-split-fit.png" alt="CurveBook split view with an XY data table next to a live nonlinear curve fit and confidence bands" width="900" />
  <br />
  <sub><em>Edit dose-response data side by side with live curve fits and confidence bands.</em></sub>
</p>

<table>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/workspace-layout-dark.png" alt="CurveBook dark mode multi-panel figure layout with significance brackets and an embedded summary table" width="100%" />
      <br />
      <sub><em>Arrange linked figures and summary tables into publication-ready multi-panel layouts (Dark mode).</em></sub>
    </td>
    <td align="center" width="50%">
      <img src="docs/images/workspace-anova-results.png" alt="CurveBook One-Way ANOVA results with assumption checks and a linked column graph showing significance brackets" width="100%" />
      <br />
      <sub><em>One-Way ANOVA with automatic assumption checks, multiple comparisons, and significance brackets.</em></sub>
    </td>
  </tr>
</table>

---

## What You Can Do

- **Organize experimental data** — Structured tables for XY, Column, Grouped, Survival, Contingency, and Nested datasets, with replicate columns, one-click outlier exclusion, and data transforms.
- **Run statistics & curve fits** — *t*-tests, ANOVA, nonparametric tests, survival analysis (Kaplan–Meier & Cox), PCA/clustering, and nonlinear regression with model comparison and plain-English assumption checks.
- **Create publication figures** — Scatter, bar, box, violin, survival, heatmap, and forest plots with error bars, significance brackets, colorblind-safe palettes, multi-panel page layouts, and `SVG`, `PDF`, `PNG`, and `TIFF` export.
- **Keep everything in sync** — Data tables, analyses, graphs, and layouts stay linked so edits update everywhere automatically.
- **Automate or use with AI** — Includes a command-line tool (`curvebook-cli`) for batch workflows and a local MCP server so AI assistants can create analyses and graphs you can open and polish in the app.

---

## Download

Get the latest Beta from **[GitHub Releases](https://github.com/HauserGroup/curvebook/releases)**:

- **macOS** — Universal `.dmg` (Apple Silicon & Intel, signed & notarized)
- **Windows** — `.msi` installer (`x64` / `arm64`) — *unsigned in Beta; see the [SmartScreen note](https://curvebook.org/manual/latest/troubleshooting.html#known-issues)*
- **Linux** — Portable `.AppImage` (`x86_64` / `aarch64`)
- **FreeBSD** — CLI-only `.tar.gz` (`x86_64`)

Need help installing, setting up the CLI, or verifying downloads? See the **[Get Started Guide](https://curvebook.org/manual/latest/get-started.html)**.

---

## Quick Start

1. **Add data** — Open CurveBook, choose a table type (or try a built-in sample dataset), and paste or import your data.
2. **Analyze** — Click **Analyze…** to run a statistical test or fit a curve. Results, assumption checks, and a linked graph are created automatically.
3. **Export** — Style your graph or combine panels into a page layout, then export to `SVG`, `PDF`, or `PNG`.

Want to drive CurveBook with a local AI assistant? Read the **[Beginner's MCP Guide](docs/mcp-for-beginners.md)**.

---

## Citing CurveBook

If you use CurveBook in published research, include the exact version you ran (from **Help › Cite CurveBook**, **About CurveBook**, or `curvebook-cli --version`):

```text
CurveBook Contributors (2026). CurveBook [Computer software]. https://github.com/HauserGroup/curvebook
```

<details>
<summary>BibTeX</summary>

```bibtex
@software{curvebook,
  author  = {{CurveBook Contributors}},
  title   = {{CurveBook}},
  year    = {2026},
  license = {MIT OR Apache-2.0},
  url     = {https://github.com/HauserGroup/curvebook}
}
```

</details>

See [`CITATION.cff`](CITATION.cff) and **[How to cite CurveBook](https://curvebook.org/manual/latest/get-started.html#how-to-cite)** for citation metadata and generated methods-paragraph details.

---

## Privacy, Docs & Contributing

- **Private & offline** — Zero telemetry, zero crash reporting, and zero network calls ([Privacy Policy](https://curvebook.org/privacy.html)). Built from scratch in safe Rust.
- **Help & docs** — [User Manual](https://curvebook.org/manual/) · [FAQ](https://curvebook.org/faq.html) · [Troubleshooting](https://curvebook.org/manual/latest/troubleshooting.html#known-issues) · [Report an Issue](https://github.com/HauserGroup/curvebook/issues)
- **Contributors** — [Contributing Guide](CONTRIBUTING.md) · [Architecture](docs/architecture.md) · [Development Guide](docs/development.md)
- **License** — Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) (see [NOTICE](NOTICE) and [ATTRIBUTION.md](ATTRIBUTION.md)).

---

<sub>CurveBook is an independent open-source project and is not affiliated with, endorsed by, or sponsored by Dotmatics, GraphPad Software, or any other provider of scientific or statistical software. All trademarks and product names are the property of their respective owners.</sub> <!-- brand-ok -->
