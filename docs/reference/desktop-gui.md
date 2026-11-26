# Desktop GUI getting started (Beta)

Launch CurveBook to enter data, run statistical tests and curve fits, inspect live split data-and-graph views, and arrange multi-panel figures offline. Need installers or checksum steps first? See [Get started & installation](get-started.md).

> [!NOTE]
> **Beta guide scope:** This page introduces the main desktop workspace and dialogs. A complete menu-by-menu guide and step-by-step workflow tutorials will follow after Beta.

---

## 1. Launch CurveBook and start a project {#launch-and-welcome}

When you open CurveBook without a project file, the **Welcome & New Data Table** hub opens with four ways to begin:

- **Empty Data Table:** Pick one of CurveBook's [structured table types](tables.md#table-archetypes) (XY, Column, Grouped, Contingency, Survival, Parts of Whole, Multiple Variables, or Nested). Set the X-column format (Numbers, Dates, Sequence, or None). Choose raw replicate subcolumns or pre-summarized entry such as Mean + SD + N, Mean + SEM + N, error limits, or quartiles.
- **Tutorial Sample Data:** Load a built-in sample dataset to explore populated tables, analyses, and graphs right away.
- **Templates (`.cbtemplate`):** Start from a saved template with analyses, parameter rules, graphs, and page layouts already configured.
- **Clone Pipeline:** Copy an existing sheet family's analyses and graphs onto a fresh dataset.

Across the top of the window, CurveBook pairs standard menus (**File**, **Edit**, **View**, **Insert**, **Data**, **Analyze**, **Graph**, **Help**) with a contextual action strip that adapts to the active sheet. Press `Cmd/Ctrl+K` to open the **Command Palette**. Switch layouts anytime with **Workspace Presets** (**Default**, **Data Prep**, **Modeling**, **Publication**, and **Compact**).

If you launch `curvebook` from a terminal, optional flags (`curvebook --help`) include `--portable-dir <dir>`, `--rulers`, `--in-window-menus`, `--ttf-font <path>`, and `--new-instance`.

---

## 2. Enter data, manage exclusions, and check quick statistics {#data-and-exclusions}

1. **Enter or import data:** Type values directly in the data grid or paste delimited text with `Cmd/Ctrl+V`. On Column tables, toggle row titles with **View › Row Titles**.
2. **Open existing files:** Use **File › Open…**, **File › Import**, or drag and drop `.curvebook`, `.csv`, `.tsv`, `.pzfx`, `.prism`, or `.xlsx` files onto the window.
   - When you open or import `.prism`, `.pzfx`, or `.xlsx` files into a project with existing sheets or unsaved edits, choose **Add to current project** or **Open as new project**.
   - Non-fatal import issues appear in the **Import Warnings** panel. Reopen it anytime from the status bar or **File › Import › Import Warnings…**.
3. **Check live summary numbers:** Select any numeric cells to update the footer **Quick Statistics** bar (`n`, excluded count, mean, SD, SEM, median, and sum; see [Calculators & quality-of-life tools](qol.md#quick-statistics)).
4. **Exclude observations without deleting them:** Press `Cmd/Ctrl+E` to toggle cell exclusions. To attach an auditable reason, choose **Edit › Exclude Selection with Auditable Reason…** (`Shift+Cmd/Ctrl+E`; see [Cell exclusions](concepts.md#cell-exclusions)).
5. **Add derived columns:** Create [`=` Calculated Variables](tables.md#calculated-variables) directly in Multiple Variables tables. Open **Manage Variables…** to rename, merge, reorder, or pin categorical reference levels.

---

## 3. Run analyses, customize graphs, and compare side by side {#analysis-and-split-graph}

Start a statistical test, curve fit, or table transform from **Analyze › Analyze…** (`Cmd/Ctrl+Shift+A`), the toolbar **Analyze** button, or the **Command Palette** (`Cmd/Ctrl+K`).

- **Automatic linked results:** CurveBook creates a linked result sheet under **Analyses** with multi-tab tables, an inline assumption-diagnostics banner, and one-click [methods reporting](qol.md#active-qol). Linked graphs update automatically when source data or settings change.
- **Create graphs from transforms or in batches:** Check **Create a new graph of the results** in **Transform / Normalize** to plot derived tables (such as normalized heatmaps) right away. Use **Insert › New Graph…** to create single-series graphs across all datasets at once with a shared Y-axis title.
- **Reuse graph styles across sheets:** In the **Portfolio Gallery**, hold `Shift` or `Cmd/Ctrl` to select multiple graphs. Choose **Graph › Apply Style From…** to copy selected styling facets in one undo step.
- **Embed live result tables on a graph:** Copy a rectangular cell range from any result sheet, right-click a graph, and select **Paste Table**. Adjust orientation, headers, typography, and borders via **Graph › Layout › Format Embedded Table…**.
- **Format plots and axes:** Open **Format Graph…**, **Format Axes…**, or the right-hand Inspector (`Cmd/Ctrl+Shift+I`). Toggle data points and fitted curves independently, assign series to Left Y or Right Y2, add horizontal offsets, or switch Column and Grouped plots to horizontal bars. You can also adjust Kaplan–Meier curves, pooled forest diamonds, heatmap labels, tick directions, decimal labels, and solid or transparent backgrounds.
- **Work in split view:** Choose **View › Toggle Split Data + Graph Workspace** (`Cmd/Ctrl+\`) to view a graph beside its data or result sheet with a draggable divider. Edits, exclusions, and parameter tweaks redraw curves and bands live. Two-way brushing highlights matching cells and plotted points, and longitudinal plots show vertex hover readouts.

![CurveBook light theme split workspace showing Project Navigator, Family tree, XY data table, live 4PL nonlinear dose-response curve fit with confidence bands, and Graph Inspector](../images/workspace-split-fit.png)

![CurveBook light theme split workspace showing multi-tab One-Way ANOVA multiple comparisons, inline assumption diagnostic banner and Interpret inspector, and linked Column graph with Compact Letter Display and significance brackets](../images/workspace-anova-results.png)

---

## 4. Compose multi-panel page layouts and insert images {#page-layouts}

Open **Layouts** in the Project Navigator (or choose **Graph › Layout…**) to place multiple graphs, live result tables, and images on a physical page canvas.

- **Size, align, and layer panels:** Apply [journal column-width presets](graphs.md#page-layouts) (88 mm single column, 130 mm column-and-a-half, or 180 mm double column). Align axes with **Graph › Layout › Align Plot Origins** and match plot scales with **Equalize Scaling…** (**Match Smallest**, **Match Largest**, or **Explicit %**). Reorder overlapping panels and transparent insets with **Bring to Front**, **Bring Forward**, **Send Backward**, and **Send to Back**.
- **Add and crop images:** Insert PNG, JPEG, or TIFF images through **Insert › Image…** or paste with `Cmd/Ctrl+V`. Drag corner handles to resize (`Shift` unlocks aspect ratio) and rotate (`Shift` snaps in 15° steps). Edit crop margins and borders in **Format Image…**, and toggle non-exported alignment guides with **View › Canvas Grid**.
- **Check themes and color accessibility:** Switch between **Light**, **Dark**, and **High Contrast** themes in [Settings & languages](settings.md). Turn on live [color-vision deficiency and grayscale preview](graphs.md#palettes-and-cvd) to verify figures before export. Broken upstream references appear with a dashed broken-link marker.

![CurveBook dark theme multi-panel Page Layout workspace with physical millimeter rulers, significance brackets, embedded summary table, and Page Layout Composer](../images/workspace-layout-dark.png)

---

## 5. Save projects and export publication figures {#saving-and-exporting}

- **Save projects and checkpoints:** Choose **File › Save** or **File › Save As…** to write atomic `.curvebook` archives or `.curvebook.d/` folders. CurveBook refuses to overwrite files saved by a newer release and keeps crash-recovery journals between saves ([Files, storage & interchange](files.md)). Create named snapshots anytime with **File › Checkpoints › Create Checkpoint…**.
- **Export figures and print:** Use **File › Export** or **File › Print…** to output vector **SVG 1.1**, **PDF 1.7** (sRGB, CMYK, or Grayscale, with optional text outlining), high-DPI **PNG** and **TIFF** (LZW, Deflate, or uncompressed), or Windows **EMF**. You can also copy vector or bitmap figures directly to the clipboard.
- **Export data tables:** Choose **File › Export › CSV Table…** or **File › Export › Export All Data Tables…**. Set the delimiter and decimal separator in the CSV options dialog (defaulting to your active locale, such as `;` with `,` or `,` with `.`). Saving with a `.tsv` extension uses tabs automatically. Exporting all tables prompts for a folder, writes one file per data table, and asks before overwriting existing files.
- **Export results and methods reports:**
  - **File › Export › CSV Results…** exports either the active result tab or all tabs in tidy format.
  - **File › Export › JSON Results…** saves structured machine-readable analysis output.
  - **File › Export › Save Methods Report…** writes the active result's methods summary as `.md`, `.txt`, or `.tex` based on the filename extension. You can also copy the report directly to the clipboard with **Analyze › Copy Methods & Statistical Reporting**.
