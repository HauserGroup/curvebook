# Desktop GUI Getting Started (Beta)

The CurveBook desktop GUI (`curvebook`) combines five-section workspace navigation, spreadsheet data entry, real-time statistical & curve-fitting DAG recalculation, side-by-side split graph inspection, and multi-panel figure layout export inside an offline GPU-accelerated window (`eframe` / `egui` / `wgpu`).

See [Get Started & Installation](get-started.md) for platform packages and checksum verification.

---

## 1. Launching & Starting a Project (`Welcome & New Data Table`) {#launch-and-welcome}

When launching `curvebook` without an existing `.curvebook` argument (or starting a new workspace), CurveBook presents the **Welcome & New Data Table** launcher (`WelcomeWizardState`) with four creation modes:

- **Empty Data Table:** Choose any of the [eight table archetypes](tables.md#table-archetypes) (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `Parts of Whole`, `Multiple Variables`, `Nested`), configure X-column format (`Numbers`, `Dates`, `Sequence`, `None`), and choose raw replicate subcolumns or pre-summarized entry (`Mean + SD + N`, `Mean + SEM + N`, error limits, quartiles).
- **Tutorial Sample Data:** Instantiate built-in clean-room scientific sample datasets to inspect populated tables, analyses, and graphs immediately.
- **`.cbtemplate Templates`:** Open a saved reusable template carrying preconfigured analyses, parameter rules, graphs, and layouts ready for new observations.
- **Clone Pipeline:** Duplicate an existing sheet family structure (`sheet.clonePipeline`) onto new raw data.

Optional launch flags (`curvebook --help`) include `--portable-dir <dir>` (self-contained config/shortcuts/recovery storage), `--rulers` (physical canvas rulers), `--in-window-menus` (macOS in-window menu bar), `--ttf-font <path>`, and `--new-instance`.

---

## 2. Entering Data, Exclusions & Quick Statistics {#data-and-exclusions}

1. Edit data cells directly in the Data grid, paste delimited clipboard text (`Cmd/Ctrl+V`), or open/import `.curvebook`, `.csv`, `.tsv`, `.pzfx`, or `.prism` files via **File › Open…** or file drag-and-drop.
2. Selecting any numeric cell range updates the footer **Quick Statistics** readout live (`n`, `excl`, `mean`, `SD`, `SEM`, `median`, `sum`; see [Calculators & Quality-of-Life Capabilities](qol.md#quick-statistics)).
3. Toggle non-destructive observation exclusions (`Cmd/Ctrl+E`) or attach audit provenance tags (**Edit › Exclude Selection with Auditable Reason...**, `Shift+Cmd/Ctrl+E`; [`QOL-3`](concepts.md#cell-exclusions)) without deleting underlying values.
4. Create derived columns using [`=` Calculated Variables](tables.md#calculated-variables) directly in `MultipleVariables` tables.

---

## 3. Running an Analysis & Inspecting a Live Split Graph {#analysis-and-split-graph}

Run a statistical test, curve fit, or table transform from **Analyze › Analyze...** (`Cmd/Ctrl+Shift+A`), the toolbar **Analyze** button, or the Command Palette (`Cmd/Ctrl+K`):

- CurveBook attaches a linked `ResultSheet` under **Analyses** (with multi-tab tables, [`QOL-7` diagnostics-only assumption banner](qol.md#active-qol), and [`QOL-6` Copy Methods & Statistical Reporting](qol.md#active-qol)) and updates dependent **Graphs** automatically through the reactive DAG.
- Toggle **View › Toggle Split Data + Graph Workspace** (`Cmd/Ctrl+\`, `QOL-4`) to display a `Graph` alongside your `Data` or `Result` sheet: edits, cell exclusions, and model parameter tweaks immediately redraw the fitted curve and bands, with two-way cell/point brushing (`QOL-5`) highlighting linked coordinates between grid and plot.

![CurveBook split workspace showing an XY data table and live nonlinear dose–response curve fit with confidence bands](../images/workspace-split-fit.png)

---

## 4. Composing Multi-Panel Page Layouts {#page-layouts}

Switch to the **Layouts** section in the Project Navigator to arrange multiple `GraphSheet`s onto a physical page canvas (`LayoutSpec`, `MAX_LAYOUT_ITEMS = 64`), apply [journal column-width presets](graphs.md#page-layouts) (`SingleColumn88mm`, `ColumnAndHalf130mm`, `DoubleColumn180mm`), align axis origins across panels (**Arrange › Align Plot Origins**), customize Light / Dark / High-Contrast themes in [Settings & Languages](settings.md), and toggle live [CVD / grayscale canvas preview](graphs.md#palettes-and-cvd) (`QOL-9`).

![CurveBook multi-panel Page Layout workspace in dark theme with panel badges and publication formatting](../images/workspace-layout-dark.png)

---

## 5. Saving Projects, Templates & Publication Exports {#saving-and-exporting}

- **Save Project (`.curvebook` or `.curvebook.d/`):** **File › Save** and **File › Save As…** write atomic deterministic `.curvebook` ZIP archives (with forward-version overwrite refusal), alongside crash-recovery WAL journals between saves ([Files, Storage & Interchange](files.md)). Create persistent milestone snapshots via **File › Checkpoints › Create Checkpoint…**.
- **Export Figures & Tables:** Use **File › Export** (`SVG`, `PDF`, `PNG`, `TIFF`, or EMF / table exports via Command Palette and CLI/MCP) to generate publication **SVG 1.1**, **`%PDF-1.7`** (sRGB / CMYK / Grayscale, optional glyph outlining), high-DPI **PNG** / **TIFF** (`LZW` / `Deflate` / uncompressed), Windows **`.emf`**, tidy/tab **CSV**, or structured **JSON**.
