# Desktop GUI getting started (Beta)

<!-- Audience: Tier 1 Scientist Guide — keep sentences short and plain; put detailed options and schemas in tables.md, analyses.md, fitting.md, graphs.md, or files.md. -->

Open CurveBook to enter lab data, run statistical tests and curve fits, and build multi-panel figures offline. Need installers or download checks first? See [Get started & installation](get-started.md).

> [!NOTE]
> **Beta guide scope:** This page introduces the main desktop workspace and dialogs. A complete menu-by-menu guide and step-by-step tutorials will follow after Beta.

---

## 1. Launch CurveBook and start a project {#launch-and-welcome}

When you open CurveBook without a project file, the **Welcome & New Data Table** hub offers four ways to start:

- **Empty Data Table:** Pick a [structured table type](tables.md#table-archetypes) (XY, Column, Grouped, Contingency, Survival, Parts of Whole, Multiple Variables, or Nested). Choose your X-column format and either raw replicates or pre-summarized entry (such as Mean + SD + N or quartiles).
- **Tutorial Sample Data:** Load a built-in sample dataset to explore working tables, analyses, and graphs right away.
- **Templates:** Start from a saved `.cbtemplate` file with analyses, graphs, and layouts already set up.
- **Clone Pipeline:** Copy an existing sheet family's analyses and graphs onto new data.

At the top of the window, standard menus (**File**, **Edit**, **View**, **Insert**, **Data**, **Analyze**, **Graph**, **Help**) sit above a toolbar that adapts to the active sheet. Press `Cmd/Ctrl+K` for the **Command Palette**. Switch screen layouts anytime with **Workspace Presets** (**Default**, **Data Prep**, **Modeling**, **Publication**, and **Compact**).

Starting from a terminal? Run `curvebook --help` for optional launch flags, or see [Settings & storage paths](settings.md#storage-paths) for portable folders.

---

## 2. Enter data, manage exclusions, and check quick statistics {#data-and-exclusions}

1. **Enter or paste data:** Type directly into the grid or paste tabular text with `Cmd/Ctrl+V`. On Column tables, toggle row titles with **View › Row Titles**.
2. **Open or import files:** Use **File › Open…**, **File › Import**, or drag and drop `.curvebook`, `.csv`, `.tsv`, `.pzfx`, `.prism`, or `.xlsx` files onto the window.
   - If your project already has sheets or unsaved changes, choose **Add to current project** or **Open as new project**.
   - Non-fatal import issues appear in **Import Warnings** (reopen from the status bar or **File › Import › Import Warnings…**).
3. **Check live summary numbers:** Select any numeric cells to see count, exclusions, mean, SD, SEM, median, and sum in the footer **Quick Statistics** bar ([Calculators & tools](qol.md#quick-statistics)).
4. **Exclude points without deleting them:** Press `Cmd/Ctrl+E` to exclude or restore cells. Choose **Edit › Exclude Selection with Auditable Reason…** (`Shift+Cmd/Ctrl+E`) to record why ([Cell exclusions](concepts.md#cell-exclusions)).
5. **Add derived columns:** Write [`=` Calculated Variables](tables.md#calculated-variables) in Multiple Variables tables. Open **Manage Variables…** to rename, merge, reorder, or pin reference categories.

---

## 3. Run analyses, customize graphs, and compare side by side {#analysis-and-split-graph}

Start a statistical test, curve fit, or table transform from **Analyze › Analyze…** (`Cmd/Ctrl+Shift+A`), the toolbar **Analyze** button, or the **Command Palette**.

- **Linked results and diagnostics:** CurveBook creates a linked result sheet under **Analyses** with tabbed tables, an assumption-check banner, and one-click [methods reporting](qol.md#active-qol). Linked graphs update whenever your data or settings change.
- **Graphs from transforms or in batches:** Check **Create a new graph of the results** in **Transform / Normalize** to plot derived tables immediately. Use **Insert › New Graph…** to build one graph per dataset with a shared Y-axis title.
- **Reuse graph styles:** In the **Portfolio Gallery**, select multiple graphs with `Shift` or `Cmd/Ctrl`. Choose **Graph › Apply Style From…** to copy styling choices in one step.
- **Embed live result tables:** Copy cells from any result sheet, right-click a graph, and pick **Paste Table**. Adjust layout, typography, and borders in **Graph › Layout › Format Embedded Table…**.
- **Customize plots and axes:** Open **Format Graph…**, **Format Axes…**, or the right-hand Inspector (`Cmd/Ctrl+Shift+I`). Toggle data points and curves independently, switch to horizontal bars, assign a right Y2 axis, or set transparent backgrounds. See [Graphs & Page Layouts](graphs.md) for all plot options.
- **Compare data and graphs side by side:** Choose **View › Toggle Split Data + Graph Workspace** (`Cmd/Ctrl+\`) to view a graph beside its table or results. Edits and exclusions redraw curves and bands live, and clicking a point highlights its matching cell.

![CurveBook light theme split workspace showing Project Navigator, Family tree, XY data table, live 4PL nonlinear dose-response curve fit with confidence bands, and Graph Inspector](../images/workspace-split-fit.png)

![CurveBook light theme split workspace showing multi-tab One-Way ANOVA multiple comparisons, inline assumption diagnostic banner and Interpret inspector, and linked Column graph with Compact Letter Display and significance brackets](../images/workspace-anova-results.png)

---

## 4. Compose multi-panel page layouts and insert images {#page-layouts}

Open **Layouts** in the Project Navigator (or choose **Graph › Layout…**) to arrange graphs, result tables, and images on a page.

- **Size and align panels:** Pick a [journal column-width preset](graphs.md#page-layouts) (88 mm single, 130 mm one-and-a-half, or 180 mm double column). Align axes with **Graph › Layout › Align Plot Origins** and match scales with **Equalize Scaling…**. Reorder overlapping panels with **Bring Forward**, **Send Backward**, **Bring to Front**, and **Send to Back**.
- **Insert and crop images:** Add PNG, JPEG, or TIFF images via **Insert › Image…** or `Cmd/Ctrl+V`. Drag handles to resize or rotate (hold `Shift` to unlock aspect ratio or snap in 15° steps). Adjust crops and borders in **Format Image…**, and toggle guides with **View › Canvas Grid**.
- **Check themes and color accessibility:** Switch between **Light**, **Dark**, and **High Contrast** themes in [Settings & languages](settings.md). Use [color-vision deficiency and grayscale preview](graphs.md#palettes-and-cvd) to inspect figures before export. Broken upstream links show a dashed marker.

![CurveBook dark theme multi-panel Page Layout workspace with physical millimeter rulers, significance brackets, embedded summary table, and Page Layout Composer](../images/workspace-layout-dark.png)

---

## 5. Save projects and export publication figures {#saving-and-exporting}

- **Save projects and checkpoints:** Use **File › Save** or **File › Save As…** to save a single `.curvebook` file or a Git-friendly `.curvebook.d/` folder. CurveBook protects newer files from being overwritten and keeps crash-recovery journals between saves ([Files & interchange](files.md)). Save named snapshots anytime with **File › Checkpoints › Create Checkpoint…**.
- **Export figures and print:** Use **File › Export** or **File › Print…** for vector SVG, PDF, or Windows EMF, or high-DPI PNG and TIFF. You can also copy figures straight to the clipboard. See [Export capabilities](graphs.md#export-capabilities) for color spaces, text outlining, and compression.
- **Export data tables:** Choose **File › Export › CSV Table…** for one table, **Export All Data Tables…** for a folder, or **File › Export › .pzfx file…** for data-only `.pzfx` export. CSV exports match your active locale's delimiter and decimal separator, and `.tsv` files use tabs automatically ([CSV rules](files.md#csv-rules)).
- **Export results and methods reports:**
  - **File › Export › CSV Results…** saves the active result tab or all tabs in tidy format.
  - **File › Export › JSON Results…** writes machine-readable analysis output.
  - **File › Export › Save Methods Report…** writes a methods summary as `.md`, `.txt`, or `.tex`, or copy it with **Analyze › Copy Methods & Statistical Reporting**.
