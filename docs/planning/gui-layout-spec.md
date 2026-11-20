# CurveBook Overall GUI Layout Specification

> **Scope:** Canonical clean-room layout, information-hierarchy, responsive-geometry, and interaction specification for the CurveBook desktop graphical user interface (`apps/curvebook`, `crates/ui-egui`) built under [`R15`](../ROADMAP.md#r15-modern-egui-desktop-gui-overhaul-beta-gate) (**Beta** gate). In `R15.5`, desktop cutover deletes the legacy `LayoutTree`, `SoftwareTextPainter`, `layout/paint.rs`, `compositor.rs`, and legacy `HumanPathDriver` code immediately without retaining legacy GUI code for `apps/curvebook-web`; browser (`apps/curvebook-web`) `eframe::WebRunner` migration waits strictly until post-Beta [`R11.11`](../ROADMAP.md#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) (**1.0** gate).
> **Related Tracked Docs:** [`../architecture.md`](../architecture.md), [`../evidence/user-interface.md`](../evidence/user-interface.md), [`../gui-workflows.md`](../gui-workflows.md), [`../localization.md`](../localization.md), [`../ROADMAP.md`](../ROADMAP.md), [`../../AGENTS.md`](../../AGENTS.md).

---

## 1. Purpose, Scope & Architectural Principles

CurveBook is an integrated scientific workbench for structured tabular data entry, data reshaping, statistical inference, parametric and differential-equation curve fitting, and publication-grade figure composition. Its GUI must let researchers move fluidly across five reactive artifact classes—**Data Tables**, **Project Info**, **Analysis Results**, **Graphs**, and **Page Layouts**—without losing lineage context, numerical provenance, or keyboard/pointer ergonomics.

### 1.1 Five Binding Layout Invariants

1. **Headless-First Engine Parity (`L5` <- `L6`):**
   Every document, cell, formula, analysis, fit, graph, or layout mutation initiated by a GUI control commits exclusively through `curvebook_engine::Session::execute` (`Caller::Ui`), preserving exact parity with CLI batch scripts, crash-recovery journals, undo/redo history, and MCP automation. Under `--gui-only`, semantic automation tools (`ui_inspect`, `ui_pointer_gesture`, `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`, `ui_screenshot`) drive the identical interactive widget tree used by a human operator.
2. **Single Retained Layout Source (`egui` + AccessKit + `FrameInspectMeta`):**
   All chrome regions, scrollable viewports, hit boxes, clip rectangles, focus rings, and accessibility roles derive from a single settled stock `egui 0.37` layout pass (`!ctx.will_discard()`) paired with `egui` AccessKit output and a thin `FrameInspectMeta` side-table, plus a pure-Rust CPU `egui` tessellation rasterizer for GPU-less CI screenshots, so rendered pixels, pointer hit-testing, and desktop accessibility nodes never diverge (`verify_pixel_to_hit_box_parity`). Legacy `LayoutTree` is deleted on desktop cutover (`R15.5`); browser `eframe::WebRunner` adoption awaits post-Beta [`R11.11`](../ROADMAP.md#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) (**1.0**).
3. **Modeless Inspection First, Deep Modals on Escalation:**
   Everyday exploration—changing symbol encodings, rescaling axes, editing column formulas, inspecting assumption checklists, or drilling from an outlier point back to its source row—happens live inside the **Trailing Inspector & Studio Dock** with immediate undoable canvas/grid feedback. Multi-tab modal dialogs (`Edit Analysis Parameters`, `Format Graph`, `Format Axes`, `Import & Paste Special`) handle multi-property batch configuration with explicit `Apply` (non-closing preview), `Cancel`, and primary commit actions.
4. **Original Visual Composition & Attributed Component Reuse:**
   CurveBook uses an original two-tier scientific header (`Application Bar` + single-row `Contextual Action Strip`), a native split-comparison center stage, and a unified context-sensitive right-hand studio rail—deliberately avoiding tall, crowded multi-row legacy icon ribbons. Permissively licensed pure-`egui` widget helpers and AccessKit/top-bar utilities adapted under [`../../AGENTS.md`](../../AGENTS.md) follow the attribution requirements in [`../../ATTRIBUTION.md`](../../ATTRIBUTION.md) and [`../../NOTICE`](../../NOTICE) while retaining CurveBook's own terminology, layout proportions, original vector iconography ([`../../ASSETS.md`](../../ASSETS.md)), and color tokens.
5. **Zero-Crash, Zero-Telemetry, Multilingual & HiDPI-Exact Geometry:**
   Every layout calculation uses saturating/checked arithmetic down to degenerate `0×0` and `64×64` viewports without panicking; scales cleanly across `1.0x`–`4.0x` HiDPI factors; enforces text fitting across all `14` Beta-active LTR/CJK locales (with the `4` RTL locales `ar`, `fa`, `he`, `ur` and horizontal RTL mirroring gated on **1.0** and mapped cleanly to `"en"` on saved Beta `preferences.json` files while keeping mathematical coordinate axes and formulas left-to-right); and never initiates network egress.

---

## 2. Global Window Shell & Spatial Partitioning

### 2.1 Canonical Shell Anatomy (LTR Orientation)

```text
+--------------------------------------------------------------------------------------------------+
| [Home] [File Edit View Insert Data Analyze Graph Arrange Window Help]  [Project/Doc] [Cmd+K][Mode]|  32 pt  Tier 1: Application & Document Bar
+--------------------------------------------------------------------------------------------------+
| [+ Sheet v] [Undo Redo] | [Primary Sheet CTA: Analyze... / Fit... / Graph Type...] [Quick Tools] |  38 pt  Tier 2: Contextual Action Strip
+-----------------------+--------------------------------------------------+-----------------------+
| [Search sheets...  x] | [Primary Pane Header / Sub-Tabs / Split Controls]| [Context Studio Tabs] |
| v Data Tables      [+] |                                                  |                       |
|   # Dose-Response     |                                                  |  Live Context-Driven  |
| v Info             [+] |           CENTER WORKSPACE STAGE                 |  Inspector / Studio   |
|   i Experiment Meta   |                                                  |                       |
| v Results          [+] |   - Single Sheet (Grid / Results / Info / Plot)  |  - MV Formula Studio  |
|   = 4PL Curve Fit     |   - Split Comparison (Primary || Secondary)      |  - Selection & Links  |
| v Graphs           [+] |   - Multi-Sheet Thumbnail Gallery                |  - Interpret Drawer   |
|   * Dose Curve        |   - Empty-Project / New-Sheet Launcher           |  - Graph Inspector    |
| v Layouts          [+] |   - Prospective Power & Sample Size Studio       |  - Layout Composer    |
|   [] Manuscript Fig 1 |                                                  |  - Point Drill-Back   |
|-----------------------|   + [Floating Callout Notes]                     |                       |
| v Lineage (Family)    |   + [Bottom-Edge Non-Blocking Diagnostic Banner] |  [All Options...]     |
|   # Dose-Response     |                                                  |                       |
|     = 4PL Curve Fit   |                                                  |                       |
|     * Dose Curve      |                                                  |                       |
|     [] Manuscript Fig |                                                  |                       |
+-----------------------+--------------------------------------------------+-----------------------+
| [< >] [Data|Info|Res|Graph|Layout|Gallery] [Sheet v] [Link OK] | Selection/Coords | [CVD] [100%] |  26 pt  Tier 3: Workspace Status & Nav Bar
+--------------------------------------------------------------------------------------------------+
     240 pt (resizable)                Protected Flexible Stage                 280 pt (resizable)
```

```mermaid
flowchart TB
    subgraph TopChrome["Top Window Chrome (70 pt total; hidden in Presentation Mode)"]
        AppBar["Tier 1: Application & Document Bar (32 pt)<br/>Home | Menus | Active Project Header | Command Search (Cmd/Ctrl+K) | Workspace Preset"]
        ActionStrip["Tier 2: Contextual Action Strip (38 pt)<br/>Sheet Lifecycle & Undo | Sheet-Adaptive Primary CTAs | Archetype Quick Actions | Overflow (...)"]
    end

    subgraph Body["Three-Column Workspace Body (RTL mirroring at 1.0)"]
        subgraph LeadingDock["Leading Dock (240 pt default, 180-380 pt, collapsible to 36 pt)"]
            NavTree["Project Navigator (~65%)<br/>Search | 5 Folders or Nested Data+Results | Inline +New"]
            FamilyDAG["Scoped Lineage ('Family') (~35%)<br/>Upstream/Downstream DAG & Multi-Source Highlighting"]
        end

        subgraph CenterStage["Protected Center Stage (>= 360 pt)"]
            PrimaryPane["Primary Sheet Viewport<br/>Data Grid | Results | Info | Graph | Layout | Launcher | Gallery | Power Studio"]
            SplitPane["Optional Split Secondary Viewport (Cmd/Ctrl+\\)<br/>15%-85% Draggable Divider + Two-Way Point Brushing"]
            Overlays["Non-Modal Stage Overlays<br/>Draggable Floating Notes | Bottom-Edge Diagnostic & Clipping Banners"]
        end

        subgraph TrailingDock["Trailing Studio Dock (280 pt default, 220-420 pt, collapsible to 36 pt)"]
            Studio["Context-Sensitive Inspector<br/>MV Formula & Roles | Table Selection | Results Interpret | Graph 5-Tab + Point Drill-Back | Page Layout"]
        end
    end

    Footer["Tier 3: Status, Quick-Stats & Viewport Footer (26 pt)<br/>History < > | Section Pills | Active Sheet & Link Status | Live Quick-Stats or Cursor Coords | CVD Preview & Canvas Zoom"]

    AppBar --> ActionStrip --> Body --> Footer
    NavTree --- FamilyDAG
    PrimaryPane --- SplitPane
```

### 2.2 Deterministic Z-Layer Stack (`ZLayer`)

Every painted primitive and interactive hit region belongs to a strict z-order plane so popups and modals never suffer click-through or overlapping hit ambiguity:

| Ordinal | `ZLayer` | Contents & Pointer Capture Rules |
|---:|---|---|
| `0` | `Workspace` | Docked panels (`AppBar`, `ActionStrip`, `Navigator`, `Family`, `CenterStage`, `TrailingDock`, `StatusBar`), virtualized cells, chart scenes, and scrollbars. |
| `10` | `FloatingOverlay` | Sheet-anchored `FloatingNote` cards, non-blocking bottom canvas diagnostic banners, active-pane focus ring (`2 px` accent stroke), and linked-region hover badges. |
| `20` | `SplitHandle` | Draggable resize dividers (`Navigator/Center`, `Navigator/Family`, `Primary/Secondary` center split, `Center/Inspector`, and column-header resize grips). |
| `30` | `DropdownMenu` | Top menu bar popups, workspace switcher popup, action-strip overflow menu (`⋯`), and workspace combobox lists. |
| `40` | `ModalBackdrop` | Dimmed translucent full-window scrim (`rgba(15, 23, 42, 128)`). Captures **100%** of outside pointer clicks and wheel events when any modal is active. |
| `50` | `ModalDialog` | Active modal dialog window, `Cmd/Ctrl+K` Command Palette, or nested detail sub-modal (`Additional Ticks & Reference Lines`, `Clone Pipeline Options`). |
| `60` | `ModalDropdown` | Combobox/dropdown popups, color-swatch popovers, and constant-hook pickers opened *inside* an active modal dialog. |
| `70` | `ContextMenu` | Right-click context menus (grid cells, column headers, navigator items, plotted graph marks/bars, layout panels, linked data regions). |
| `80` | `Tooltip` | Non-interactive hover tooltips (disabled-predicate `disabled_reason` explanations, excluded-cell provenance, plotted-point data readouts, colorbar value readouts, shortcut hints). |

### 2.3 Responsive Viewport Breakpoints & Collapse Priority

Minimum supported desktop window size is **`640 × 420 pt`** (default **`1280 × 800 pt`**); headless/embedded viewports remain panic-free down to `0 × 0 px`.

| Breakpoint | Logical Width (`pt`) | Layout Behavior & Deterministic Auto-Collapse Order |
|---|---|---|
| **Wide Desktop** | `>= 1440 pt` | Both `Navigator` (`240 pt`) and `TrailingDock` (`280–310 pt`) open comfortably alongside a `>= 900 pt` `CenterStage` (ideal for side-by-side Split Comparison). `AppBar` shows full `Cmd/Ctrl+K` search field and workspace preset label. |
| **Standard Laptop** | `960 – 1439 pt` | `Navigator` (`210–240 pt`), `CenterStage` (`>= 480 pt`), and `TrailingDock` (`250–280 pt`) remain simultaneously visible; `ActionStrip` condenses secondary labels into icon+tooltip buttons before spilling to `⋯`. |
| **Compact / Tablet** | `640 – 959 pt` | Auto-collapses in strict priority order to preserve `>= 360 pt` for `CenterStage`: (1) `AppBar` search field condenses to a search icon; (2) `TrailingDock` collapses into a `36 pt` vertical tab strip (opening as an overlay flyout when clicked); (3) if `< 760 pt`, `Navigator` collapses into a `36 pt` leading icon strip; (4) Center Split switches from horizontal (`Left || Right`) to vertical (`Top / Bottom`) below `720 pt`. |
| **Presentation Mode** | Any (`Cmd/Ctrl+Shift+P` / `F11`) | Hides `AppBar`, `ActionStrip`, `Navigator`, `TrailingDock`, and `StatusBar` (`0 pt`), giving **100%** of viewport area to the active `GraphSheet`, `LayoutSheet`, or `PortfolioGallery` (`Escape` exits). |

### 2.4 Keyboard Focus Cycling & RTL Mirroring

- **Pane Focus Traversal (`F6` / `Shift+F6`):** Cycles keyboard focus across visible top-level scopes in deterministic order:
  `Navigator` -> `Family` (if open) -> `Center Primary` -> `Center Secondary` (if split active) -> `Floating Notes` (if present) -> `Trailing Studio Dock` (if open). The active scope renders a `2 px` high-contrast accent frame.
- **RTL Locale Mirroring (`1.0` gate — `ar`, `fa`, `he`, `ur` hidden at Beta with `"en"` fallback in `preferences.json`):**
  - Horizontal panel docking mirrors cleanly (`Navigator` docks right, `TrailingDock` docks left, `AppBar` menus start right, `StatusBar` zoom moves left, disclosure chevrons and history arrows flip).
  - **Scientific invariance in RTL:** Numeric digits, mathematical expressions (`f(x) = ...`), $\TeX$ equations, and standard Cartesian `X` axes remain left-to-right unless an explicit axis-reversal property is toggled by the user.

---

## 3. Top Chrome: Application Bar & Contextual Action Strip

### 3.1 Tier 1 — Application & Document Bar (`32 pt`)

- **Leading Cluster:**
  - Platform window caption clearance on macOS transparent titlebars (with double-click-to-zoom drag area on empty bar space).
  - **Home / Launcher Button** (`20 × 20 pt`, hit box `28 × 28 pt`): Toggles the Project Launcher / Template Hub without closing the current project.
  - **Menu Bar:** Routed to the native macOS menu bar via macOS-only `muda` (`cfg(target_os = "macos")`, `default-features = false`) while keeping the in-`egui` `MenuBar` active on Windows/Linux/FreeBSD, under `--in-window-menus`, and across `EguiHumanPathDriver`/`egui_kittest` runs with an automated `command_id`/enablement parity test:
    `File` · `Edit` · `View` · `Insert` · `Data` · `Analyze` · `Graph` · `Arrange` · `Window` · `Help`.
- **Middle Cluster — Active Project Header / Optional Multi-Project Tabs:**
  - In **Beta**, renders the active project header and dirty-state indicator (`•`) for `apps/curvebook`'s single-session window; multi-project in-window document tabs (`.curvebook` tab strip, `+` new project, OS window drag-out, and `Window › Consolidate / Tile`) are optional post-Beta enhancements.
- **Trailing Cluster (Right-to-Left Fit Solver):**
  - **Command Search Pill (`Cmd/Ctrl+K`)**: Displays `"Search commands, analyses & sheets (Cmd+K)"` when room permits (`>= 180 pt`), degrading cleanly to a `Search` icon button in narrow windows; disabled while a modal dialog owns focus.
  - **Split-Workspace Quick Toggle (`Cmd/Ctrl+\`)**: One-click toggle and layout preset menu (`Off`, `Data + Graph`, `Data + Results`, `Results + Graph`, `Swap Panes`).
  - **Workspace Preset Switcher**: Compact pill (`Default`, `Data Prep`, `Modeling`, `Publication`, `Compact`) that recalls saved panel visibility and widths.

### 3.2 Tier 2 — Sheet-Adaptive Contextual Action Strip (`38 pt`)

A single, uncluttered horizontal strip divided by subtle `1 px` vertical rules into semantic groups. Universal left-hand anchors stay in fixed pixel positions across sheet switches so muscle memory never breaks; right-hand groups adapt to the active sheet kind and table archetype:

| Active Context | Fixed Leading Anchors | Primary Contextual CTA Group | Secondary Contextual Tools | Trailing Output & View Group |
|---|---|---|---|---|
| **`EmptyWorkspace` / Launcher** | `New Project`, `Open…` | `New Data Table…`, `Browse Samples`, `Clone Pipeline…` | `Import CSV / TSV…`, `Power & Sample Size…` | `Theme`, `Language` |
| **`DataSheet` (Structured)** | `+ New Sheet v`, `Undo`, `Redo`, `Cut/Copy/Paste`, `Paste Link` | **`Analyze…`** (accent button) + **Archetype Quick-Run Pills** (e.g., `t-Test` / `ANOVA` on Column; `Fit Curve` / `Linear Reg.` on XY; `2-Way ANOVA` on Grouped; `Contingency` on Contingency; `Kaplan-Meier` on Survival) | `Import / Reshape…`, `Transform…`, `Exclude (Cmd+E)`, `Freeze Table`, `Sort / Prune`, `Decimals (.00)` | `New Graph`, `Inspector`, `⋯` |
| **`DataSheet` (`MultipleVariables`)** | `+ New Sheet v`, `Undo`, `Redo`, `Cut/Copy/Paste`, `Paste Link` | **`Analyze…`** (accent button) + **`+ Calculated Variable (fx)`** + **Quick-Run Pills** (`Linear/GLM`, `PCA`, `Cluster`, `MV Plot`) | `Manage Variables…`, `Filter & Transform…`, `Extract & Rearrange…`, `Exclude (Cmd+E)`, `Freeze Table` | `New MV Graph`, `Formula Studio`, `⋯` |
| **`InfoSheet`** | `+ New Sheet v`, `Undo`, `Redo`, `Copy/Paste` | **`+ Add Constant`** | `Add Date/ID Preset`, `Link Constant…`, `Format Notes` (`Bold`, `Italic`, `List`) | `Inspector`, `⋯` |
| **`ResultSheet`** | `+ New Sheet v`, `Undo`, `Redo`, `Copy`, `Copy Methods` | **`Edit Parameters… (Cmd+T)`** (accent button) + **`Interpret & Checklist`** (badge shows warning count) | `Change Data Analyzed…`, `Chain Analysis…`, `New Graph from Tab…` | `Export Table…`, `Inspector`, `⋯` |
| **`GraphSheet`** | `+ New Sheet v`, `Undo`, `Redo`, `Copy Vector` | **`Graph Type…`** + **`Analyze / Fit…`** | `Format Graph…`, `Format Axes…`, `Pairwise / CLD v`, `Insert Text / TeX / Bracket v`, `Palette` | **`Export Figure…`**, `Print…`, `Inspector`, `⋯` |
| **`LayoutSheet`** | `+ New Sheet v`, `Undo`, `Redo`, `Copy Vector` | **`+ Place Graph`** + **`Arrange Grid…`** | `Align / Distribute v`, `Sync Panel Scales` *(1.0 `R13.5`)*, `Panel Badges (A, B, C)`, `Add Text / TeX / Scale Bar` | **`Export Layout…`**, `Print…`, `Inspector`, `⋯` |

---

## 4. Leading Dock: Project Navigator & Scoped Lineage ("Family")

### 4.1 Upper Pane — Project Navigator (`~65%` height, resizable)

1. **Filter Header (`32 pt`):** Instant substring search input (`navigator.search_input`) with clear (`×`) button and optional sheet-kind filter chip (`All`, `Data`, `Info`, `Results`, `Graphs`, `Layouts`).
2. **Two Organization Modes (toggled via Navigator header menu or `Preferences › Workspace`):**
   - **Five-Folder Category Mode (Default):** Collapsible top-level sections:
     1. `Data Tables`
     2. `Info`
     3. `Results`
     4. `Graphs`
     5. `Layouts`
   - **Nested Pipeline Mode (`Data with Results`):** Folds each derived `ResultSheet` directly underneath its upstream `DataSheet` as an expandable child tree (matching natural experimental notebook hierarchy), while keeping `Info`, `Graphs`, and `Layouts` as sibling sections.
3. **Row Anatomy (`26 pt` height, `>= 24 pt` hit box, scrollbar never overlaps right-edge badges):**
   - Disclosure triangle (if nested children exist) + original sheet-archetype icon (`XY`, `Col`, `Grp`, `Cnt`, `Surv`, `PoW`, `MV`, `Nest`, `Info`, `Res`, `Graph`, `Layout`) + editable sheet title + trailing status badges (`Frozen` snowflake/lock badge, `Color Tag` dot, `Warning/Unconverged` indicator).
   - Clicking a section header's **`Folder Gallery`** icon switches the Center Stage into the Multi-Sheet Thumbnail Gallery filtered to that section.
   - Each section ends with a muted inline **`+ New <Kind>…`** row for zero-menu sheet creation.
4. **Direct Manipulation & Cascading Integrity:**
   - **Double-click title (`F2`):** Inline rename (`sheet.rename`), automatically cascading updated titles to dependent default analysis headers, graph titles, and legend labels.
   - **Drag-reorder within section:** Reorders sheets deterministically without mutating stable `SheetId`s.
   - **Drag an Analysis (`ResultSheet`) onto another compatible `DataSheet`:** Opens the `Retarget Analysis` confirmation preview and rebinds the entire downstream analysis + graph lineage to the target data table.

### 4.2 Lower Pane — Scoped Lineage ("Family") Tree (`~35%` height, `Cmd/Ctrl+Shift+F`)

Separated from the Project Navigator by a `4 pt` vertical splitter:
- Displays **only** the connected topological subgraph of the currently active sheet:
  `Root Data Table(s)` -> indented `Chained Transforms / Analyses` -> `Derived Graphs` -> `Referencing Page Layouts`.
- **Active-Node & Multi-Dependency Highlighting:**
  - The currently active sheet renders with an accent pill background.
  - When a **Composite `LayoutSheet`** or **Multi-Dataset `GraphSheet`** is selected, *all* upstream Data Tables, Analyses, and Graphs contributing panels or curves to that sheet are highlighted in **bold** in both the `Family` tree and the upper `Project Navigator` so multi-experiment dependencies are immediately legible.
- **Hover Quick-Preview:** Hovering an inactive node in `Family` for `>= 450 ms` displays a compact read-only summary tooltip (table dimensions/archetype, test name + primary $P$-value, or thumbnail aspect preview).

---

## 5. Center Stage: Multi-Mode Viewport, Split Comparison & Stage Overlays

The Center Stage is a protected dock target (cannot be closed or floated away) that operates in one of five mutually exclusive primary modes:

1. **Single-Sheet Mode (Default):** Full central area renders the active `DataSheet`, `InfoSheet`, `ResultSheet`, `GraphSheet`, or `LayoutSheet`.
2. **Split-Comparison Mode (`Cmd/Ctrl+\`):**
   - Divides the Center Stage into **Primary** and **Secondary** sheet panes separated by a `4 pt` draggable splitter (`15%`–`85%` clamp, double-click resets to `50%`, button/shortcut swaps panes).
   - Each pane renders a compact `24 pt` **Pane Header Strip** (`Pinned Sheet Selector v`, `Sync Brushing` indicator, `Swap`, `Close Split`).
   - **Live Two-Way Point Brushing:** Selecting or hovering rows in a Data/Residual Grid highlights the corresponding plotted marks/bars on an adjacent `GraphSheet`, and box-selecting or clicking plotted marks on a `GraphSheet` scrolls to and highlights the originating cells in the adjacent `DataSheet`.
3. **Multi-Sheet Portfolio Gallery Mode:**
   - Activated by clicking a Navigator folder gallery icon, the `Gallery` status-bar pill, or `View › Sheet Gallery`.
   - Renders a responsive grid of live-rendered sheet preview cards with folder filter chips (`All`, `Graphs`, `Layouts`, `Data`, `Results`), thumbnail zoom slider, `F6` pane cycling, and click-to-open. Multi-card selection checkboxes (`Shift+Click` range, `Cmd/Ctrl+Click` multi-select) and batch cloning actions (`Apply Style From…`, `Export Selected…`, `Place Selected onto New Layout`) plug into this view on the **1.0** gate (`R13.3`).
4. **Project Launcher Mode:** Shown automatically when `total_sheet_count() == 0` or when `Home` / `File › New Data Table…` is invoked in full-stage mode (see §8.1).
5. **Prospective Power & Sample Size Studio Mode:** Full-stage or large-modal interactive study-planning workflow (see §8.13).

### 5.1 Stage-Level Non-Modal Overlays (`ZLayer::FloatingOverlay`)

- **Floating Callout Notes (`FloatingNote`):**
  - Draggable, collapsible cards pinned to a specific sheet in logical coordinates (`note_positions_pt`), with color-coded header band (neutral slate, amber note, sky info, emerald checklist), collapse chevron, and inline rich-text body; clamped so headers never escape the visible viewport or block split-pane dividers.
- **Non-Blocking Bottom-Edge Diagnostic & Clipping Banner (`28–36 pt`):**
  - Docked flush along the bottom inside edge of a `GraphSheet` or `ResultSheet` viewport whenever data-visibility caveats exist:
    - *Unplotted Missing Rows:* `"4 rows with missing values in mapped variables were omitted from this plot. [Show Rows] [Dismiss]"`
    - *Clipped Error Bars / Confidence Bands:* `"Manual axis range clips 2 error bars or confidence-band segments. [Why?] [Fit Range]"`
    - *Read-Only Linked Region Guard (on `DataSheet`):* `"Cells B1:D12 are live-linked to 'Raw Assay'. [Go to Source] [Unlink to Edit]"`

---

## 6. Trailing Dock: Context-Sensitive Live Inspector & Studio Rail

Instead of forcing users into modal dialogs for routine edits—or hiding multivariable formula tools in detached popups—CurveBook unifies its right-hand dock (`280 pt` default, `Cmd/Ctrl+Shift+I`) into a **Context-Sensitive Studio** whose header badge and tab strip adapt automatically to the focused Center Stage pane:

| Focused Surface | Studio Title & Mode | Sub-Tabs / Accordion Sections | Escalation Footer Button |
|---|---|---|---|
| **`DataSheet` (`MultipleVariables`)** | **Variable & Formula Studio** | 1. **`Formula (fx)`**: Variable Name, live `f(x) =` expression box with inline syntax/cycle status, searchable `Variables` list, searchable `Functions` catalog (`Recent` + categorized `Math`, `Stats/Column`, `Logic`, `Text`) with signature doc card.<br/>2. **`Role & Levels`**: Type selector (`Auto`, `Continuous`, `Categorical`, `Label`), missing-value summary, inline categorical level recoding & explicit level ordering (`Up`/`Down`, reference level). | `Manage All Variables…` |
| **`DataSheet` (Structured)** | **Table & Selection Studio** | 1. **`Structure`**: Table archetype summary, X/subcolumn configuration, `Convert Archetype…` trigger.<br/>2. **`Selection & Provenance`**: Linked-region source inspector (`Go to Source`, `Edit Link`, `Unlink`), excluded-cell reason audit list, and **Selected-Row Graph Override** (`Marker Color`, `Shape`, `Size`, `Show Row Title on Graph`, `Reset to Series`). | `Format Data Table…` |
| **`InfoSheet`** | **Metadata & Hooks Studio** | **Referenced By** list showing every analysis constraint, transform `K` parameter, and graph/layout text box currently hot-linked to the selected `Info` constant, plus typed format controls (`ISO Date`, `Number`, `Text`, `ORCID/DOI`). | `Open Hook Browser…` |
| **`ResultSheet`** | **Interpret & Diagnostics** | 1. **`Assumptions`**: Interactive methodological checklist (`Pass`, `Review`, `Violated` badges + severity filter).<br/>2. **`Guidance`**: Diagnostics-only plain-language explanation of test structure and effect sizes (strictly non-prescriptive).<br/>3. **`Methods`**: Copyable publication methods paragraph interpolating exact model settings, sample sizes, and software version. | `Edit Parameters… (Cmd+T)` |
| **`GraphSheet` (No single point selected)** | **Graph Inspector** | Five top tabs:<br/>1. **`Data`** (Series/Channel encodings, `Symbols`, `Bars/Boxes`, `Error Bars`, `Lines/Curves`, `Ellipses/Hulls`, `Heatmap/Dendrogram`)<br/>2. **`Axes`** (`X` / `Left Y` / `Right Y`: auto/manual range, interval, scale, breaks, titles, label angle)<br/>3. **`Legends`** (visibility, placement, discrete vs 3-point continuous bubble/colorbar legends)<br/>4. **`Annotations`** (`Pairwise Brackets` vs `Compact Letter Display`, equations, threshold lines)<br/>5. **`Canvas`** (palette cards, background/plot-area fill & alpha, aspect ratio, display checkboxes) | `All Graph Options…` / `All Axis Options…` |
| **`GraphSheet` (Single plotted point clicked)** | **Data Point Drill-Back** | Source `DataSheet` link, `Row #` & Row Title, series/variable coordinates (`X`, `Y`, `Color`, `Size`), individual point style override controls, `Exclude Point…`, and primary **`Jump to Row in Data Table`** button (`← Back to Graph Inspector` in header). | `Format Selected Points…` |
| **`LayoutSheet`** | **Page Layout Composer** | 1. **`Page`** (Journal width presets: `85 mm` 1-col, `114 mm` 1.5-col, `174 mm` 2-col, `A4`/`Letter`; orientation; margins; DPI preflight).<br/>2. **`Grid & Align`** (rows×cols template, snap guides, align plot frames vs outer bounds, lock shared scale).<br/>3. **`Annotations`** (auto `A, B, C` panel badges, scale bar generator, hooked constants). | `Export Layout…` |

Every section header inside the `Graph Inspector` includes a compact **`More…`** link that opens the corresponding tab of the full `Format Graph` or `Format Axes` modal dialog when advanced multi-series or sub-tick editing is needed.

---

## 7. Bottom Status, Quick-Stats & Viewport Footer (`26 pt`)

1. **Leading Navigation Cluster:**
   - `Back (<)` / `Forward (>)` sheet-visit history steppers.
   - Five compact section jump pills (`Data`, `Info`, `Results`, `Graphs`, `Layouts`) + `Gallery` grid toggle.
   - Active Sheet Combobox (`Sheet Switcher`) + `Upstream Link Status` icon (green/blue when all live links and formulas are up to date; amber if an external file link is stale).
2. **Center Contextual Readout (`status.summary`):**
   - **When a `DataSheet` or `ResultSheet` is focused:** Live **Selection Quick-Stats** formatted in tabular numerals:
     `n = 24 (2 excl) · Mean = 48.32 · SD = 6.14 · SEM = 1.25 · Median = 47.90 · Sum = 1,159.7` (capped at `50,000` cells per frame to stay under `0.5 ms`).
   - **When a `GraphSheet` or `LayoutSheet` is focused:** Live physical pointer coordinates (`X: 42.5 mm, Y: 18.0 mm` or data-space `(x, y)` readout when hovering inside a plot frame).
3. **Trailing Viewport Cluster:**
   - **Accessibility & Print Preview Filter Pill:** `Normal` · `Deuteranopia` · `Protanopia` · `Tritanopia` · `Grayscale Print`.
   - **Rulers & Snap Toggle** (enabled on `GraphSheet` / `LayoutSheet`; optional non-exported canvas grid overlay joins at `1.0` per `R13.4`).
   - **Canvas Zoom Controls (`-`, slider/preset `100% v`, `+`, `Fit`):** Strictly scoped to `GraphSheet`, `LayoutSheet`, and `PortfolioGallery`—never zooms `DataSheet` or `ResultSheet` text/rows on trackpad or mouse-wheel scroll.

---

## 8. Surface-by-Surface Detailed Layout Specifications

### 8.1 Project Launcher, Table Schema Composer & Pipeline Cloner (`Welcome`)

```text
+------------------------------------------------------------------------------------+
|  New Data Table & Workspace Hub                                              [Esc] |
+--------------------+----------------------------------+----------------------------+
| CREATE DATA TABLE  | Archetype: Coordinate (XY)       |  STRUCTURAL SCHEMATIC      |
| * Coordinate (XY)  | Table Title: [Assay 1         ]  |  +----+--------+--------+  |
|   Column (1-Way)   |                                  |  | X  | Grp A  | Grp B  |  |
|   Grouped (2-Way)  | Data Source:                     |  |Conc| Y1 Y2  | Y1 Y2  |  |
|   Contingency      |  (o) Empty table for data entry  |  +----+--------+--------+  |
|   Survival         |  ( ) Tutorial sample dataset     |                            |
|   Parts of Whole   |                                  |  DEFAULT LINKED PLOT       |
|   Multiple Vars    | X Column Format:                 |     ^  *                   |
|   Nested           |  [Numbers                   v]   |     |    o  *              |
|                    |                                  |     +---------> X          |
| WORKSPACE & REUSE  | Y Subcolumns:                    |                            |
|   Starter Templates|  ( ) Single Y value              |  [x] Auto-create Graph     |
|   Clone Pipeline   |  (o) Replicates: [- 3 +]         |  [x] Attach Info Sheet     |
|   Sample Gallery   |  ( ) Pre-computed: [Mean,SD,N v] |                            |
|   Open File...     |                                  |         [Cancel]  [Create] |
+--------------------+----------------------------------+----------------------------+
```

- When `total_sheet_count() == 0`, the launcher renders inside a centered bounded card on `CenterStage` with `Navigator`/`TrailingDock` collapsed and `Cancel` hidden.
- **Conditional Schema Adaptation across all `8` Table Archetypes:**
  - **`XY` (Coordinate):** Exposes `X Column Format` (`Numbers`, `Numbers + X Error`, `ISO Dates / Elapsed Time`, `Row Sequence`) and `Y Subcolumns` (`Single Y`, `N Side-by-Side Replicates` stepper `1..256`, or `Pre-computed Summary` tuple `Mean/SD/N`, `Mean/SEM/N`, `Mean/95% CI`, `Median/IQR`).
  - **`Column` (Single-Factor):** Hides `X` format; exposes `Experimental Design Hint`:
    - `Unpaired replicates` (defaults auto-created graph to individual-value column scatter + mean/SD),
    - `Paired / Row-matched subjects` (defaults auto-created graph to connected before-after estimation/line plot), or
    - `Pre-computed summary values`.
  - **`Grouped` (Two-Factor):** Exposes `Y Subcolumns` (`Single`, `N Replicates` representing subjects/repeats across row × column factors, or `Pre-computed Summary`).
  - **`Contingency`:** Disables replicate and summary subcolumn controls; enforces non-negative integer count cells across `Row Categories × Column Outcomes`.
  - **`Survival`:** Locks schema to `Time` (`X`), `Group Columns` (`Y`), and configurable numeric `Event / Censored` codes (`1 = Event`, `0 = Censored` default).
  - **`PartsOfWhole`:** Single numeric column + category row labels (optional total check).
  - **`MultipleVariables`:** Tidy observation-by-variable schema (`Rows = Observations`, `Columns = Typed Variables`); exposes initial variable count stepper.
  - **`Nested`:** Hierarchical `GroupColumns × ReplicateSubcolumns` representing sub-replicates nested within biological units (e.g., technical replicates per animal/well).
- **Clone Pipeline Mode (`Clone a Graph / Pipeline`):**
  - Tabs across `Open Projects`, `Recent Files`, and `Saved .cbtemplate Pipelines` with a thumbnail zoom slider.
  - Clicking `Clone…` opens the **Clone Options Sub-Modal** with checkboxes to selectively `Clear raw data values`, `Keep column titles`, `Keep row titles`, and `Rename cloned family prefix`, instantiating the full linked Data -> Transform -> Analysis -> Graph chain ready for new assay measurements.

---

### 8.2 Virtualized Data Grid (`DataSheet`)

```text
+--------+------------+----------+--------------------+--------------------+
| [XY v] | Row Titles |    X     |      Group A       |      Group B       |  Tier 1: Corner Archetype Trigger & Super-Column IDs
|--------+------------+----------+--------------------+--------------------|
|   #    | Sample ID  | Dose(uM) |      Vehicle       |     Compound 7     |  Tier 2: Editable Column / Factor Titles [<||>]
|--------+------------+----------+------+------+------+------+------+------|
|        |            |    X     |  Y1  |  Y2  |  Y3  |  Y1  |  Y2  |  Y3  |  Tier 3: Subcolumn Roles (or MV Variable Type Strip)
+========+============+==========+======+======+======+======+======+======+
|   1    | Well A01   |    0.01  | 98.4 | 97.1 | 99.0 | 96.2 | 95.8 |      |
|   2    | Well A02   |    0.10  | 97.9 |101.2*| 98.1 | 74.5 | 71.0 | 73.2 |  *101.2* = Italic + Strikethrough + Exclusion Tag
+--------+------------+----------+------+------+------+------+------+------+
```

- **Performance & Scrolling Contract:**
  - Only visible `(row, flat_subcol)` cells are formatted and emitted into the UI tree (`<= 16.67 ms` 60-fps budget verified at `1,000 × 1,000 = 1,000,000` cells); inline `TextEdit` changes buffer locally and commit once via `Session::execute` on `Enter`, `Tab`, or focus loss to preserve exact undo and `ops.journal` parity.
  - Two-axis independent scrollbars (`ScrollAxes::Both`); frozen top 3-tier header band and frozen leading `#` (`48 px`) + collapsible/resizable `Row Titles` (`112 px` default, `34 px` collapsed) columns.
- **Top-Left Corner Schema Button:** Displays the current archetype (`XY`, `Col`, `Grp`, `MV`, …) and opens `Format Data Table` / `Convert Table Archetype…` (with live mapping preview and lossy-conversion warning).
- **Multivariable Tier-3 Type & Role Header Strip:**
  - When `TableArchetype::MultipleVariables` is active, Tier 3 renders per-column interactive **Role Chips**:
    - `Auto` (infers `Continuous` when numeric, automatically switching to `Categorical` when text tokens are entered),
    - `NUM` (`Continuous` distribution icon),
    - `CAT` (`Categorical` bar icon),
    - `LBL` (`Label / Identifier` tag icon), and
    - `FX=` (`Calculated` badge — locks cells to read-only italicized formula output; clicking focuses the right-hand Formula Studio).
- **Cell Visual States & Provenance:**
  - **Numeric Alignment:** Right-aligned tabular lining numerals (`tnum`); categorical/label strings start-aligned.
  - **Missing vs Excluded Observations:** Empty cells represent missing values (`NaN`/unbalanced design, tolerated natively by ANOVA/GLM/Regression). Excluded cells (`Cmd/Ctrl+E` or `Exclude with Reason…` `Cmd/Ctrl+Shift+E`) stay visible in place with muted color, strikethrough, italic text, a corner reason-indicator triangle, and a hover tooltip showing timestamp/reason.
  - **Linked Data Regions (`LinkedDataRegion`):** Cell ranges populated via `Paste Link` or linked file import draw a `2 px` color-coded perimeter border (`link_blue`, `link_green`, …) with a header pill showing upstream source (`Raw Plate › A1:H12`) and quick actions (`Go to Source`, `Re-import`, `Unlink`). Typing inside a linked region triggers a non-destructive read-only guard popover rather than silently corrupting provenance.
  - **Row-Subset -> Graph Override Context Menu:** Selecting rows (e.g., after sorting a fold-change/P-value table) and right-clicking exposes `Style Selected Points on Graph…` (color + alpha, shape, size, border), `Show Row Titles on Graph`, `Reset to Dataset Style`, and `Exclude Selected`.

---

### 8.3 Variable & Formula Studio + Bulk Variable Manager (`MultipleVariables`)

1. **Right-Hand Docked Formula Studio (`MvFormulaViewState`):**
   - **Header:** `Variable Z` · `Role: Calculated (Continuous)` · `[Show Definitions Toggle]`.
   - **Variable Name Input:** Editable column title (`mv_formula.name.input`).
   - **Expression Bar (`f(x) =`):** Monospace expression editor with inline commit (`✓` / `Enter`), cancel (`✕` / `Escape`), live AST/type/circular-dependency validation banner directly below the input, and **Live Operand Highlighting**—every variable referenced in the active formula highlights its corresponding grid column with a matching colored outline (`A`, `B`, …) so users can verify operands visually.
   - **Dual Reference Pickers (vertically split inside scrollable studio body):**
     - *Available Variables:* Searchable list displaying column letter, title, and `NUM`/`CAT`/`LBL`/`FX=` badge; clicking inserts the identifier at the caret.
     - *Available Functions:* Searchable list with category pills (`Recent`, `All`, `Math`, `Column Aggregates`, `Conditional`, `Rounding`) covering row-wise functions (`abs`, `ln`, `log10`, `exp`, `round`, `ifelse`, `is_defined`, row-wise `sum`/`min`/`max`) and whole-column aggregators (`mean(Var)`, `sd(Var)`, `median(Var)`, `zscore(Var)` enabling multi-step pipelines such as $\Delta\Delta C_t$ qPCR fold-change).
   - **Signature & Description Footer (`64 pt`):** Displays exact parameter syntax and edge-case `NaN` behavior for the highlighted function.
2. **Bulk Variable & Categorical Level Manager Modal (`Manage Variables…`):**
   - Master-detail modal accessed from Tier-3 `Manage Variables…` or `Data › Manage Variables…`:
     - *Upper Master Table:* Filterable list of all columns (`Display: All | Categorical | Continuous | Calculated`) with `Auto` checkbox and explicit `Continuous / Categorical / Label` radio columns.
     - *Lower Categorical Level Inspector (enabled whenever a `Categorical` variable is selected):* Lists every distinct level with `Total Rows`, `Excluded Rows`, **Inline Level Rename / Merge** (typing an existing level name merges misspelled categories cleanly in one undoable step), and explicit `Level Order` controls (`Alphabetical`, `Frequency`, or `Custom` keyboard/drag reordering + `Reference Level` pin).

---

### 8.4 Project Metadata (`InfoSheet`) Workspace

Two-column split inside the Center Stage (`38%` left / `62%` right default, draggable divider):
- **Left Pane — Structured Constants Table:**
  - Columns: `Constant Name` | `Value` | `Type / Hook Badge`.
  - Pre-seeded customizable rows (`Experiment Date`, `Experiment ID`, `Notebook / ELN`, `Researcher`, `Protocol`, `Lot / Batch`) plus unlimited user-added constants.
  - Typed inline editors (date picker/stepper on date constants, numeric validator on scalar constants). Constants show a chain badge (`🔗 2`) indicating how many downstream analyses, transforms, or figure annotations reference them.
- **Right Pane — Protocol & Experiment Notes:**
  - Multi-paragraph formatted text editor (`Headings`, `Bold`, `Italic`, `Monospace`, `Bulleted/Numbered Lists`) for experimental conditions, reagent lots, and audit notes.

---

### 8.5 Five-Stage Data Ingestion & Reshaping Wizard (`Import & Paste Special`)

Unified modal (`680 × 520 pt` target, clamped to viewport) with a numbered pipeline tab bar and persistent bottom summary (`Source Path / Clipboard Origin`, `Save as Default`, `Reset Defaults`, `Cancel`, primary **`Import`** or **`Paste`**):

1. **`1. Source`:** File picker / clipboard inspector; binding mode (`Insert values only` vs `Insert & maintain live link` with auto-refresh); delimiter & locale controls (`Comma / Tab / Semicolon / Space`, decimal separator `.` vs `,`, thousands separator, UTF-8 BOM handling).
2. **`2. Preview`:** Virtualized read-only parsed `Row × Col` preview grid reflecting `Source` delimiter settings before filtering.
3. **`3. Filter & Unstack`:**
   - Missing-value tokens (`NA`, `N/A`, `null`, custom) and trailing-asterisk (`*`) auto-exclusion rule.
   - Row slicing (`Start row`..`End row`, decimation `Every k-th row`, and conditional predicates `Skip/Keep rows where Col # <op> <value>`).
   - Column range/skipping **or** **`Unstack Tidy Columns`** (`Data values in Col #`, `Group identifier in Col #` — automatically disables manual column-range inputs when checked).
4. **`4. Placement & Reshape`:**
   - Insertion anchor (`Active cell` vs `Append rows` vs `Append columns`), first-row-as-titles toggle, and 2D arrangement rules:
     - `Keep source arrangement`,
     - `Transpose rows and columns`,
     - `Plate / Block Reshape`: *"Place $N$ values per row, and after $M$ rows start a new dataset column"* (converting `96`- or `384`-well plate matrices directly into replicate subcolumns).
5. **`5. Metadata`:** Optional extraction of header comment lines or key-value rows directly into a linked `InfoSheet` + `FloatingNote`.

---

### 8.6 Reactive Data Transformation & Schema Projection Modals

All data-reshaping operations execute as non-destructive `ResultSheet` or derived-table nodes in the reactive DAG (never overwriting raw observations):

1. **`Transform Values` & `Transform Concentrations`:**
   - Function family (`Standard`, `Pharmacology / Biochemistry`, `Custom X / Y Expression`), `Interchange X and Y` checkbox, independent `Transform X` and `Transform Y` equation pickers (`X = log10(X)`, `Y = -log10(Y)`, `Probit`, `Logit`, `Arcsin-Sqrt`, …), global vs per-series `K` constants with **Hook Constant (`🔗`)** buttons, zero-concentration substitution for log dose-response axes, replicate vs mean transform rule, **Nonlinear Error-Bar Policy** (`Omit SD/SEM` vs `Convert to asymmetric 95% CI`), and `Create new graph of results`.
2. **`Remove Baseline & Column Math`:**
   - Baseline definition (`Selected column(s)` with mean vs linear-interpolation option, or `First / Last k rows`), operation (`Difference Y - B`, `Ratio Y / B`, `Percent 100 * Y / B`, `Fractional Change (Y - B) / B`), subcolumn replicate pairing rule, and baseline-column removal toggle.
3. **Multivariable `Filter & Transform` (`3` Tabs: `Transform` · `Select Rows` · `Select Columns`):**
   - `Select Rows` provides conjunctive filter blocks: `Row Index Slice`, **`Variable vs. Value`** multi-row predicate table (`Variable`, `Operator`, `Value` or `🔗 Hook`), and **`Variable vs. Variable`** predicate table (`Variable 1`, `Operator`, `Variable 2`).
4. **Multivariable `Extract & Rearrange` (`2` Tabs: `Target Table Format` · `Data Arrangement`):**
   - Tab 1 selects target structured schema (`XY`, `Column`, `Grouped`, `Contingency`) + prefix option.
   - Tab 2 dynamically swaps variable-binding dropdowns to match Tab 1 (e.g., for `Grouped`: `Response (Y)`, `Row Factor`, `Column Factor`, and `Subject / Replicate ID`).

---

### 8.7 Analysis Launcher, Re-Analysis Disambiguation & Retargeting

1. **`Analyze…` Modal (`640 × 460 pt`):**
   - **Header:** Source sheet indicator + instant search box (`analyze_picker` filter, focused on open so typing + `Enter` launches in `< 2 s`).
   - **Left Pane (`58%`):** Categorized method tree (`Recently Used`, `Transform & Normalize`, `XY Analyses`, `Column Analyses`, `Grouped Analyses`, `Contingency Analyses`, `Survival Analyses`, `Parts of Whole`, `Multiple Variables & GLM`, `Unsupervised Clustering`, `Nested Analyses`, `Simulate & Theoretical Curves`). Pre-expands and highlights the category matching the active table's archetype.
   - **Right Pane (`42%`):** Dataset / Variable checklist (`Select All` / `Clear All`) scoping which columns enter the analysis.
2. **Re-Analysis Disambiguation Prompt (`DuplicateAnalysisPrompt`):**
   - Triggered automatically when invoking an analysis family already attached to the active table: offers **`Edit Existing Analysis Parameters`** vs **`Create New Independent Analysis`**.
3. **`Change Data Analyzed…` Modal (`ChangeDataAnalyzed`):**
   - Opened from any `ResultSheet` (or by dragging a `ResultSheet` onto another compatible `DataSheet`) to retarget the existing parameter configuration to a different compatible `DataSheet` and/or change included datasets, updating downstream graphs and `Family` lineage atomically.

---

### 8.8 Statistical, GLM, Survival, PCA & Clustering Parameter Modals

#### Standardized Modal Anatomy
- **Header Band:** `Parameters: <Analysis Family>` + **Dynamic Resolved-Test Banner** updating live across tab edits (e.g., *"Resolved test: Welch's one-way ANOVA (no equal-SD assumption) + Dunnett T3 pairwise comparisons"*).
- **Horizontal Tab Strip:** Keyboard-navigable tabs; disabled tabs show a muted lock state and hover reason.
- **Body:** Progressive disclosure groups, clean-room vector experimental-design schematics (illustrating independent vs row-matched vs crossover repeated-measures layouts or residual plot shapes), and declarative enablement predicates (`R1`–`R12`). Every disabled control remains visible with a hover tooltip explaining *why* it is unavailable and what table/option change unlocks it.
- **Footer:** `Method Reference`, `Save as Default`, `Cancel`, **`Run Analysis`** (or **`Update Analysis`** when editing via `Cmd/Ctrl+T`).

#### Standardized Multivariable `Data` Role-Binding Tab
Prepended as Tab 1 whenever a classical analysis (`t`-Test, Multifactor ANOVA, Kaplan-Meier, Nonlinear Fit) is launched from a `MultipleVariables` table:
- **Left (`Available Variables`):** Searchable variable list with role filter chips (`All`, `NUM`, `CAT`, `LBL`, `FX=`).
- **Right (`Model Role Slots`):** Cardinality-typed assignment cards (`Click +` or drag-and-drop):
  - *Two-Group Test:* `Response (Y) [1+]` · `Grouping Factor [1, 2 levels]` · `Subject / Pairing ID [0..1, enabled when Paired]`.
  - *Multifactor ANOVA:* `Response (Y) [1]` · `Fixed / Grouping Factors [2..k]`.
  - *Kaplan-Meier Survival:* `Time to Event [1]` · `Status / Censoring [1+]` · `Grouping Factor [0..1]`.
  - *Nonlinear Curve Fit:* `Response (Y) [1+]` · `Predictor (X) [1]` · `Series Grouping Factor [0..1]`.

#### Family-Specific Tab Structures & Conditional Enablement Rules (`R1`–`R12`)

| Analysis Family | Domain Tabs | Mandatory Conditional Adaptation & Enablement Predicates |
|---|---|---|
| **Two-Group & One-Way / RM / Mixed ANOVA** | `[Data]` · `Experimental Design` · `[Repeated Measures]` · `Multiple Comparisons` · `Options` · `Residuals` | **`R1` (Missing cells vs RM/REML):** Complete tables enable Repeated Measures ANOVA; incomplete matched tables automatically switch to `REML` Mixed-Effects Model (`Kenward-Roger` / `Satterthwaite` DF). **`R1b`:** Enabling `REML` disables classical sphericity checkboxes and restricts incompatible post-hoc options with explanatory tooltips. **`R2`:** Nonparametric rank selection (`Mann-Whitney`, `Wilcoxon`, `Kruskal-Wallis`, `Friedman`) disables Gaussian equal-SD toggles and switches `Multiple Comparisons` to rank-based Dunn's test. **`R3`:** Control-group selector enables *only* when `"Compare each group to a control"` is selected. **`R4`:** Welch's *t*-test / nonparametric modes gate equal-variance residual diagnostics and unlock `Estimation Plot` / Hodges-Lehmann median-difference options. |
| **Multifactor (`N`-Way) ANOVA** | `Data` · `Model` · `Multiple Comparisons` · `Options` · `Residuals` | `Model` tab provides hierarchical checkboxes (`Main effects`, `Two-way interactions`, `Three-way interactions`) above a read-only live symbolic formula bar (`Y ~ FactorA + FactorB + FactorA:FactorB`) and Type I / II / III Sum-of-Squares selector. |
| **Contingency ($\chi^2$ / Fisher)** | `Test & Calculations` · `Options` | **`R5`:** $2\times 2$ effect sizes (`Odds Ratio`, `Relative Risk`, `Attributable Risk`, `Sensitivity / Specificity`) disable on $R\times C > 2\times 2$ tables, while `Chi-square test for trend` (Cochran-Armitage) disables on $2\times 2$ tables. **`R6`:** On `Options`, individual CI method dropdowns (Baptista-Pike, Woolf, Koopman, Newcombe/Wilson) enable *only* for the specific effect sizes checked on Tab 1. |
| **Linear Regression & Correlation** | `Model & Comparison` · `Confidence & Range` · `Residuals` | **`R7`:** Correlation matrix heatmap option enables *only* when computing all-pairs correlations across $\ge 2$ columns. **`R8`:** Subcolumn aggregation radios (`Mean`, `Individual replicates`, `Per subcolumn`) enable *only* when source subcolumns $> 1$. Includes **`Switch to Nonlinear Regression…`** escalation button for weighted/constrained linear fits. |
| **GLM & Cox PH (`Linear`, `Logistic`, `Poisson`, `Cox`)** | `Model` · `Reference Levels` · `Predictions` · `Compare` · `Options` · `Diagnostics` · `Residuals` · `Graphs` | `Model` tab hosts outcome binding (incl. `Time` + `Event/Censor` codes + `Efron`/`Breslow`/`Exact` ties for Cox), hierarchical term builder (`Intercept`, `Main effects`, `2-way`, `3-way`, `Transforms`), and copyable symbolic formula preview. `Reference Levels` dynamically lists *only* categorical predictors in the model. **`R11` (GLM Family Gating):** Switching from Gaussian to `Poisson` disables Gaussian `Multiple R`, `Adjusted R²`, and residual normality tests while enabling `Deviance`, `Dispersion`, `Log-Likelihood`, and `Pseudo R²`. **`R12`:** Single-predictor logistic S-curve card on `Graphs` enables *only* when exactly one continuous predictor is active; `ROC Curve`, `Actual vs Predicted`, `Residual vs Predicted`, and `Parameter Covariance Heatmap` cards pair clean-room schematic thumbnails with diagnostic purpose labels. |
| **PCA / PCR & Clustering (`Hierarchical`, `K-Means`)** | `Data` · `Options` · `Output` · `Graphs` | **`R10` (PCR Chaining):** Checking `Run Principal Component Regression (PCR)` on `PCA › Data` unlocks `Outcome Variable` and automatically excludes that target column from the unsupervised predictor checklist; `PCA › Graphs` allows mapping *any* table column (including out-of-model categorical variables) to `PC Scores` point color, size, labels, or connecting lines. Clustering `Options` exposes scaling order (`None`, `Z-Score`, `Mean-Center`), distance (`Euclidean`, `Manhattan`, `Pearson`, `Cosine`), linkage (for Hierarchical) or algorithm + **$k_{min}..k_{max}$ Cluster Sweep** + reproducible `Seed` (for K-Means), plus `Dendrogram`, `Clustered Heatmap`, `Silhouette`, `Elbow`, and `Gap` plot cards. |
| **Kaplan-Meier Survival** | `[Data]` · `Model & Options` · `Multiple Comparisons` | Group-count predicate disables 2-group hazard ratios when $\ge 3$ groups exist (enabling omnibus Log-rank, Trend, and Gehan-Breslow-Wilcoxon plus multiplicity-adjusted pairwise curve comparisons). Includes direct toggles for `Censored tick marks`, `Asymmetric log-log CI bands`, and **`Show Number-at-Risk table below X-axis`**. |

*(Global **`R9` Predicate**: Custom significant-digit / decimal spinners enable only when `"Custom"` is chosen over CurveBook's preset P-value formats `0.0001 / APA / NEJM` [`PValueFormat::{FourDecimal, Apa, Nejm, Custom}`].)*

---

### 8.9 Nonlinear Curve Fitting Modal & Constant Hook Browser

1. **Nonlinear Regression Parameter Modal (`FitParamsViewState`):**
   - **Tabs (`7` core tabs + `Data` when launched from `MultipleVariables`):**
     `[Data]` · `Equation` · `Method & Weights` · `Compare` · `Constraints` · `Initial Values` · `Range & Interpolation` · `Confidence & Diagnostics`.
   - **`Equation` Tab:**
     - Searchable categorized tree of `117+` built-in parametric models (`Dose-Response Stimulation / Inhibition`, `Receptor Binding — Saturation / Competitive / Kinetics`, `Enzyme Kinetics — Michaelis-Menten / Allosteric / Inhibition`, `Exponential Decay / Growth`, `Polynomial / Peak / Gaussian`, `Lines`) plus **`+ Custom Equation (Explicit / Implicit / ODE)`** editor (`Y = ...`, `F(X, Y) = ...`, `dY/dX = ...`).
     - Live rendered $\TeX$ mathematical equation preview + clean-room **Model Guidance Card** explaining parameter interpretation, expected X-scale (e.g., linear vs $\log_{10}$ concentration), and recommended plateau constraints.
     - Standard-curve checkbox: `Interpolate unknowns from fitted curve` (with configurable CI level).
   - **`Constraints` Tab:**
     - Tabular parameter grid (`Parameter` | `Constraint Rule` | `Value / Bounds` | `🔗 Hook`): supports `Unconstrained`, `Fixed to constant`, `Shared across all datasets (Global Fit)`, `Must be > / < / between`, and `Dataset constant (= column title)`.
   - **`Confidence & Diagnostics` Tab:**
     - `Asymmetric (Profile-Likelihood)` vs `Symmetric (Asymptotic SE)` CIs, shaded/dashed `Confidence` or `Prediction` bands, `Compare two equations / global vs individual parameters (AICc / Extra-sum-of-squares F-test)`, weighting (`1/Y²`, `1/Y`, `1/SD²`, `Robust Cauchy/Huber`), outlier detection (`ROUT`), replicate test, and normality/homoscedasticity of residuals.
2. **Constant & Equation Hook Browser Modal (`Hook Constant`):**
   - Invoked from any `🔗` button in `Constraints`, `Transform`, `Axes Range`, or `Insert › Hooked Constant / Equation` on canvases.
   - Displays a 3-branch collapsible tree:
     1. **`Info Constants`** (from `InfoSheet` rows),
     2. **`Analysis Constants & Equations`** (per-series best-fit scalar parameters, $R^2$, $\text{EC}_{50}$, $P$-values, plain-text substituted equation `Equation`, and live mathematical **`Equation (TeX)`**),
     3. **`Document Metadata`** (file name, save timestamp, sheet counts).
   - Footer provides `Unlink`, `Cancel`, and `Link Constant`.

---

### 8.10 Multi-Tab Reactive Analysis Results Viewer (`ResultSheet`)

```text
+------------------------------------------------------------------------------------+
| [Edit Params (Cmd+T)] | [= Tabular Results] [= Multiple Comparisons] [= Residuals] [< >]|
+------------------------------------------------------------------------------------+
| [!] Diagnostic Note: Group B has n=4; normality test has low power. [Open Interpret]|
+------------------------------------------------------------------------------------+
|  v Best-Fit Values & 95% Profile-Likelihood CI                                     |
|  Parameter       | Vehicle          | Compound 7       | Global Shared             |
|  Bottom          | 1.42 (-0.8..3.6) | 2.10 (0.4..3.8)  | --                        |
|  IC50 (uM)       | ??? (ambiguous)  | 0.084 (0.06..0.11)| --                       |
+------------------------------------------------------------------------------------+
```

- **Top Bar (`32 pt`):** Leading **`Edit Parameters (Cmd/Ctrl+T)`** pill + horizontal scrollable **Sub-Sheet Tab Strip** with `<` `>` overflow steppers and keyboard tab switching (`Ctrl+PageUp/PageDown`).
- **Two Semantic Sub-Tab Kinds (`ResultTabKind`):**
  1. *Inferential Report Tabs (`Tabular Results`, `ANOVA`, `Multiple Comparisons`, `Model Comparison`, `Diagnostics`):* Multi-column sectioned grids with bold collapsible section headers, exact $P$-values alongside summary stars, explicit `??? (ambiguous / wide)` markers with inline explanation callouts when profile-likelihood bounds do not close, and blank cells (never `0` or `NaN` text) when a survival curve has not reached median (`50%`) survival.
  2. *Plot-Ready Derived Data Matrix Tabs (`Residuals`, `Row Predictions`, `Interpolated Unknowns`, `PC Scores`, `Loadings`, `Distance Matrix`, `Clustered Data` with appended cluster-membership columns, `Number at Risk`):* Directly selectable as upstream sources for **`New Graph from Tab…`** or **`Chain Analysis…`** without copying data manually (rectangular range `Copy` -> live `EmbeddedResultTable` pasting joins on the **1.0** gate per `R13.4`).

---

### 8.11 Graph Canvas, Type Switcher & Dual-Schema Formatting Modals (`GraphSheet`)

#### A. Interactive Graph Canvas Viewport
- **Aspect-Ratio Paper Frame:** Centered on a neutral `canvas_mat` background with optional physical `mm`/`in`/`pt` rulers and non-intrusive selection chrome (series chips, resize handles, and rotation grips appear *only* on hover or explicit selection so publication previews stay uncluttered).
- **Rich Hover Provenance Tooltips:**
  - Hovering any plotted point, bar, box, violin, or residual mark displays a crisp card: `Source: Dose-Response · Group B (Compound 7) · Row 4 (Well A04) · X = 1.0, Y = 42.1 · Right-click to style or exclude`.
  - Hovering a continuous heatmap or bubble **colorbar** displays the exact interpolated numeric value and hex/RGB color at the pointer position.
- **Specialized Scientific Plot Overlays:**
  1. **Pairwise Significance Brackets vs Compact Letter Display (`CLD`):**
     - Toolbar/Inspector `Pairwise & CLD` menu links directly to any attached post-hoc comparison result:
       - *Pairwise Brackets:* Auto-laid-out horizontal brackets (supporting manual drag and asymmetric vertical leg heights) with inline quick-picker for significance stars (`*`..`****`, `ns`), exact $P$-values, or custom text.
       - *Compact Letter Display (Piepho algorithm):* Places concise shared-group letters (`A`, `B`, `AB`) cleanly above each bar/box/group—automatically recommended by a non-blocking hint whenever $\ge 4$ groups produce $> 6$ overlapping brackets.
  2. **Dual-Axis Estimation Plots:** Pairs raw group distributions on the left Y-axis with the mean/median difference point estimate + `95% CI` error bar on a zero-aligned right Y-axis.
  3. **Axis-Synchronized Kaplan-Meier Number-at-Risk Table:** Rendered directly beneath the survival X-axis with group labels color-matched to survival curves (`Number at risk (number censored)`). Whenever X-axis `Min`, `Max`, or `Major Tick Interval` changes, the risk-set timestamps recompute automatically and align column centers to X-axis major ticks.
  4. **Live Typeset $\TeX$ Model Equations:** Placed via `Insert › Model Equation (TeX)` or `Hook Constant`, rendering multi-line fractions, superscripts, and Greek symbols with live-substituted parameter estimates that update automatically whenever source data or fit constraints change.

#### B. `Change Graph Type` / `Create New Graph` Modal
- **Top Controls:** `Graph Family` selector (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `Parts of Whole`, `Multiple Variables`, `Nested`) + visual **Category Sub-Tabs** (e.g., `Individual Values`, `Box & Violin`, `Summary Bars & Spikes`, `Heatmap`, `Multivariable Bubble / Parallel`).
- **Middle Controls:** Clean-room vector thumbnail cards + adaptive **`Summary & Error` Dropdown** (`Mean ± SD`, `Mean ± SEM`, `Mean ± 95% CI`, `Geometric Mean ± Geometric SD`, `Median ± IQR`, `Tukey Boxplot`, `Min-to-Max + All Points`, `Staggered Replicates`) — or **Initial Channel Pickers** (`X`, `Y`, `Color`, `Size`, or `Aggregation Metric`) for Multivariable graphs.
- **Bottom Live Preview:** Renders the actual active dataset using the selected chart configuration before committing.

#### C. Modal `Format Graph` (Three Clean Context-Switched Schemas)
Includes cross-jump buttons (`Change Graph Type…`, `Format Axes…`) plus `Cancel`, `Apply` (live preview), and `OK`:
1. **Series-Based Schema (`XY`, `Column`, `Grouped`, `Survival`, `PartsOfWhole`, `Nested`):**
   - `Appearance`: Top **Scope Bar** (`Dataset Selector v` + `<` `>` steppers + **`Apply Globally`** scope toggle) above composable layer checkboxes (`Bars / Boxes`, `Symbols` with alpha slider for overplotting and `Show mean as '+'`, `Error Bars` direction/cap/thickness, `Connecting Lines / Curves`, `Area / Confidence-Band Fill` incl. `Within error bands`), z-order selector (`Lines & error bars on top of points` vs `Underneath`), Left/Right Y-axis assignment, and per-series row-title point labels.
   - `Series & Arrangement`: Add/remove/reorder datasets (`Left-to-right` order on categorical charts; `Front-to-back` z-order on scatter charts) plus multi-series topology (`Interleaved`, `Stacked`, `Superimposed`, or `Separated` with conditional vertical divider line & inter-group gap controls).
   - `Graph Settings` & `Value Annotations`: Bar/point value callouts (`Above error bar` vs `Inside bar top/bottom`, precision, sample size $n$, auto-contrasting text color).
2. **Multivariable Channel & Layer Schema (`MultipleVariables` Scatter/Bubble):**
   - Three top tabs: `Axis Variables` · `Data Objects` · `Legends`.
   - `Data Objects` uses a left **Layer List** (`Symbols`, `Connecting Lines`, `Confidence Ellipses`, `Convex Hulls`, `Dendrogram`) and right property sub-tabs:
     - *`Symbols › Size`:* Adapts strictly to bound variable role:
       - `Uniform`: single scalar size.
       - `Continuous`: `Min/Max Diameter`, `Min/Max Data Value` (`Auto` checkboxes), conditional **Out-of-Range Policy** (`Clamp to min/max size` vs `Omit from graph`, enabled when `Auto` is unchecked), and `3`-circle reference size legend.
       - `Categorical`: per-level stepper (`Level v`, `< >`, `Global`, `Size`) + complete discrete level legend.
     - *`Confidence Ellipses` & `Convex Hulls`:* Computed across `All points` or **`Grouped by <Any Categorical Variable>`** (decoupled from symbol color/size encodings), configurable confidence level (`50%`–`99%`), and independent fill/border alpha.
3. **Matrix & Aggregated Heatmap Schema:**
   - Tabs: `Color Mapping` · `Labels` · `Gaps & Borders` · `Legend`.
   - Supports direct matrix cells (Grouped) and **2D Categorical Aggregation** (Multivariable: `Count`, `Mean`, `Median`, `SD`, `Sum` of a 3rd continuous variable, disabling the value-variable dropdown automatically when `Count` is chosen), sequential vs 3-stop diverging colormaps (`Min`, `Baseline`, `Max` auto/manual stops), out-of-range color, optional `'×'` marker on blank/excluded cells, auto-contrasting in-cell numeric labels, and tick-label rotation (`0° / 45° / 90°` vertical column labels + margin reservation join on the **1.0** gate per `R13.3`).

#### D. Modal `Format Axes` & `Additional Ticks / Thresholds` Sub-Modal
- **Tabs:** `Frame & Origin` · `X Axis` · `Left Y Axis` · `Right Y Axis` · `Titles & Typography`.
- **`Frame & Origin`:** Framing presets (`Open L-Frame`, `Boxed`, `Offset Axes`, `Crosshair`) and **Origin Placement** (`Automatic`, `Lower-Left`, `Lower-Right`, `Upper-Left`, `Custom (X0, Y0)` — ensuring volcano and PCA score plots with negative coordinates keep the Y-axis cleanly on the left frame edge when `Lower-Left` is chosen).
- **Axis Tabs:** Scale (`Linear`, `Log10`, `Log2`, `Ln`, `Logit`, `Probability`), `Two/Three-Segment Discontinuous Axis Breaks` (with break position, length proportion, and ornament), `Range & Major/Minor Ticks` (with `🔗` constant hooks), **Categorical Axis Guard** (automatically locking continuous range/log controls with an inline explanation when an axis plots discrete categories, while exposing `Column Titles` vs `Row Titles` label-source switching on Grouped plots), and **`Additional Ticks, Threshold Lines & Shaded Zones`** table with a `Details…` sub-modal for dashed horizontal/vertical significance cutoffs (e.g., $-\log_{10} P = 1.301$, $\text{FC} = \pm 1$) and shaded reference bands.

---

### 8.12 Composite Multi-Panel Page Layout Composer (`LayoutSheet`)

- **Page Canvas:** Renders physical page bounds (`mm` or `in`), bleed/margin guides, and live-linked `GraphSheet` viewports, embedded summary tables, text callouts, $\TeX$ equations, and vector scale bars (`EmbeddedImage`, `EmbeddedResultTable` cropping/transposition, `layout.equalizeScaling`, and `BringToFront` z-order join at the **1.0** gate per `R13.4`–`R13.5`).
- **Direct Manipulation:** Dragging a panel snaps both outer panel bounds and **inner plot-axis frames** (`SnapSource`) against sibling panels so multi-panel figure axes align to `0.1 pt` precision; corner grips resize either the container viewport or zoom scale (`CanvasResizeMode`); **double-clicking** any placed panel drills directly into its source `GraphSheet` (with a `Return to Layout` breadcrumb pill in the Center Stage header).

---

### 8.13 Prospective Experimental Power & Sample Size Studio (`PowerWizard`)

```text
+------------------------------------------------------------------------------------+
|  Power & Sample Size Studio   1. Design  >  2. Effect Size  >  3. Alpha  >  4. Results
+---------------------------------------------------------+--------------------------+
| [Summary & Narrative] [Power Curve] [Sensitivity Table] | LIVE SENSITIVITY INPUTS  |
|                                                         | Design: Unpaired t-Test  |
|  Required Sample Size:  n = 50 per group (100 total)    | Group 1 Mean: [ 70.0   ] |
|  Achieved Power:        0.806 (target 0.80, alpha 0.05) | Group 2 Mean: [ 60.0   ] |
|                                                         | SD Mode: [Per-Group   v] |
|  [Interactive Power vs Total N Curve / Methods Report]  | SD 1 / SD 2: [15] / [20] |
|                                                         | Alpha / Tails:[0.05|2-t] |
|                                     [Copy Report] [Save]| Target Power: [ 0.80   ] |
+---------------------------------------------------------+--------------------------+
```

- **Top Persistent Stepper Bar (`36 pt`):** `1. Experiment Design` -> `2. Effect Size` -> `3. Alpha & Power` -> `4. Results & Sensitivity`.
- **Stage 1 (`Design`):** Clean card grid (`Two Unpaired Means`, `Two Paired Means`, `One-Way ANOVA (k Groups)`, `Single Mean vs Constant`, `Two Proportions`, `Log-Rank Survival`) with two actions: **`Configure Parameters →`** (Step 2) or **`Compute Immediately with Defaults →`** (jumps straight to Step 4).
- **Stages 2 & 3 (`Effect Size` & `Alpha / Power`):** Two-column layout (`55%` conditional inputs with live computed standardized effect size $d / f / h$ at bottom; `45%` plain-language methodological notes explaining variance assumptions and one- vs two-sided tradeoffs).
- **Stage 4 (`Interactive Results Studio`):**
  - **Left Main Pane (`65%`, 3 Sub-Tabs):**
    1. *`Summary & Narrative`:* Required integer per-group and total $N$, achieved power after integer rounding, and a copyable plain-language study-justification narrative interpolating the user's exact parameters.
    2. *`Power Curve`:* Interactive `Power vs Sample Size` curve with pointer crosshair readout.
    3. *`Sensitivity Table`:* Matrix of minimum detectable effect sizes across rows of per-group $N$ and columns of target power (`0.70`, `0.80`, `0.90`, `0.95`).
  - **Right Live Parameter Sidebar (`35%`):** Exposes all effect-size, SD, group-count, $\alpha$, and power controls live alongside the results so researchers can perform sensitivity sweeps without stepping backward.

---

### 8.14 Publication Export, Print & Preferences Modals

1. **`Export Figure / Layout` Modal (`ExportFigure`):**
   - Controls: `Target Scope` (active sheet vs selected gallery sheets), `Format` (`PDF`, `SVG`, `EMF`, `PNG`, `TIFF` matching `crates/render/src/emf.rs` and `crates/engine/src/cmd/export.rs`), `Physical Width` (`Actual Size` vs journal preset `85 / 114 / 174 mm` or custom `mm`/`in`/`px`), `Resolution (DPI)` (`300`, `600`, `1200`), `Color Space` (`sRGB`, `Grayscale`, `CMYK`), `Background` (`Opaque` vs `Transparent`), and `TIFF Compression` (`LZW` / `Deflate` / `None`).
   - **Conditional Gating:** Selecting pure vector formats (`PDF`, `SVG`) disables raster-only `DPI` and `TIFF Compression` controls with explanatory tooltips (while `EMF` preserves vector/device-context scaling); selecting formats without alpha (`PDF`, `EMF`) disables `Transparent` background.
2. **`Application Preferences` Modal:**
   - Five clean sidebar categories: `Workspace & Navigation` (5-Folder vs Nested `Data+Results` tree, default split behavior, sheet-switch zoom policy), `Units & Typography` (`mm` vs `in`, UI/table/note font size scale, en-dash minus signs `−` on axes), `Default Graphs & Palettes` (default error-bar metric, Okabe-Ito / colorblind-safe default scheme, missing-error-bar alerts), `Statistical & Regional Defaults` (default $\alpha$, CI level, P-value style, date-parsing order), and `Keyboard Shortcuts` (searchable conflict-checked rebinding editor persisted in `preferences.json`).

---

## 9. Design Tokens, Reusable GUI Helpers & Verification Gates

### 9.1 Canonical Geometry & Typography Tokens (`1.0x` Logical `pt`)

| Token | Logical Value | Scaling & Ergonomic Rule |
|---|---|---|
| `chrome.app_bar_h` | `32 pt` | Scales by `round(px * scale_factor)`; `0 pt` in Presentation Mode |
| `chrome.action_strip_h` | `38 pt` | Single horizontal row; overflow items collapse into trailing `⋯` popup |
| `chrome.status_bar_h` | `26 pt` | Fixed single-line footer; tabular numerals for quick-stats & zoom |
| `dock.navigator_w` | `240 pt` (`180–380 pt`) | Collapses to `36 pt` icon rail; auto-collapses below `760 pt` window width |
| `dock.inspector_w` | `280 pt` (`220–420 pt`) | Collapses to `36 pt` icon rail; auto-collapses below `960 pt` window width |
| `dock.splitter_w` | `4 pt` (`8 pt` hit slop) | Visual `1 px` border line centered inside an `8 pt` pointer grab zone |
| `grid.row_h` / `header_h` | `24 pt` / `20+24+20 pt` | Minimum interactive target `24 × 24 pt` (`>= 28 × 28 pt` on toolbar buttons) |
| `scroll.bar_thickness` | `10 pt` (`12 pt` hover) | Reserved gutter alongside content—**scrollbars must never overlap controls or row badges** |
| `font.ui_body` / `ui_small` | `13 pt` / `11.5 pt` | System SFNT proportional family discovered via `fontdb` (`memmap` off) + CJK/deterministic SFNT symbol fallback chain (**Beta:** `14` LTR/CJK locales; **1.0:** + `4` RTL locales); zero bundled font binaries |
| `font.grid_numeral` | `12.5 pt` tabular | Proportional tabular-lining or monospace SFNT so decimal points align vertically |

### 9.2 Theme & Semantic Color Tokens (`WCAG 2.1 AA` Verified)

All three shipped themes (`Light`, `Dark`, `HighContrast`) provide validated contrast (`>= 4.5:1` body text, `>= 3.0:1` interactive boundaries and badges):
- **Chrome & Surfaces:** `app_bar`, `panel_bg`, `surface_bg`, `canvas_mat`, `header_bg`, `border`, `hover_bg`, `active_bg`, `focus_ring` (`2 px` accent).
- **Data Provenance & Roles:**
  - Variable Role Badges: `NUM` (blue), `CAT` (amber/vermilion), `LBL` (slate), `FX=` (emerald/teal).
  - Provenance Borders: `link_blue`, `link_green`, `link_red` (broken/stale external link), `excluded_cell_text` (muted + strikethrough).
  - Default Chart Series Palette: 8-color **Okabe-Ito** colorblind-safe categorical palette (`#000000`, `#E69F00`, `#56B4E9`, `#009E73`, `#F0E442`, `#0072B2`, `#D55E00`, `#CC79A7`) + perceptually uniform **Viridis / Cividis / Diverging Blue-White-Vermilion** colormaps.

### 9.3 Attributed Upstream UI Adaptation & Clean-Room Rewrite Boundaries

Per [`../../AGENTS.md`](../../AGENTS.md) (*Attributed VectorCraft GUI source reuse*), permissions and clean-room boundaries split cleanly between pure-`egui` adaptations and engine-coupled CurveBook rewrites (recording exact upstream commit, source paths, and affected CurveBook files strictly inside [`../../ATTRIBUTION.md`](../../ATTRIBUTION.md) and [`../../NOTICE`](../../NOTICE), and omitting external suite branding or identifiers banned by `cargo xtask brands`):

- **Direct Pure-`egui` Adaptations (`ATTRIBUTION.md` + `NOTICE`):**
  1. Standalone `egui 0.37` widget primitives (`toggles`, `buttons`, `labels`, `panel_chrome`, `wheel_stream`).
  2. **Responsive Top-Bar Right-Group Fitting (`fit_right_group`):** Deterministic width solver that degrades the `Cmd/Ctrl+K` search field to an icon and hides secondary switchers cleanly without layout jitter.
  3. **Scroll-Bar Non-Overlap AccessKit Gate:** Automated `egui::accesskit` geometry assertions proving vertical scrollbars in dense property inspectors and sheet trees never intersect clickable controls or row badges.
- **Clean-Room CurveBook Rewrites (Pattern Reference Only; Native CurveBook Implementations):**
  1. **Scrubbable Numeric Spinners & Unit Converters (`mm` <-> `in` <-> `pt`):** Implemented cleanly against `curvebook-doc` / `curvebook-engine` single-commit `Session::execute` (`Caller::Ui`) undo checkpoints for axis bounds, symbol sizes, and page layout margins.
  2. **Geometry-Epoch Screenshot Synchronization (`GeometryStamp`):** Synchronizes `ui.screenshot` captures across post-resize / DPI transitions and settled `!ctx.will_discard()` frames so screenshots never report stale pre-resize frame bounds.
  3. **CJK IME Preedit & Commit:** Implemented cleanly using standard registry `egui 0.37` `Event::Ime` composition handling (avoiding forked `egui` APIs) over inline grid and canvas text editors.
  4. **Conflict-Checked Shortcut Rebinding Editor:** Implemented directly over `UI_COMMANDS` and versioned `preferences.json` (zero external keymap crates and zero `std::sync::Mutex` wrappers).

### 9.4 Mandatory Layout Verification Gates

Every desktop layout change in `crates/ui-egui` and `apps/curvebook` must pass (and browser `apps/curvebook-web` aligns post-Beta in [`R11.11`](../ROADMAP.md#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) on **1.0**):

1. **Pixel-to-Hit-Box & Phantom-Widget Gate (`verify_pixel_to_hit_box_parity`):** Every interactive node in AccessKit-backed `ui.inspect` (`FrameInspectMeta`) and `EguiHumanPathDriver` / `egui_kittest` has non-zero visible bounds matching CPU-rasterized `ui.screenshot` pixels across `Light`/`Dark`/`HighContrast` and `1.0x`–`3.0x` HiDPI scales (**1.0** adds `RTL` and Web ARIA), with `100%` `ZLayer::ModalBackdrop` outside click/wheel capture and zero click-through.
2. **Scrollbar & Clip Non-Overlap Gate:** Zero `Role::ScrollBar` intersections with clickable/focusable child controls at `640 × 420 pt` and `1440 × 600 pt`.
3. **14-Locale Beta Text Fitting, Symbol Coverage & Ellipsis Gate (`R10.9` / `R15.1`):** Zero unclipped text bleed across all `14` Beta-active LTR/CJK locales (extending to all `16` incl. the `4` RTL locales at **1.0**); mandatory non-missing glyph coverage across UI and scientific symbols (`→ ▾ ● ✓ ± ≤ ≥ μ χ²`); and any ellipsized label sets `state.ellipsized = true` and exposes its full un-truncated label via hover tooltip and accessibility name.
4. **1,000,000-Cell 60-FPS Virtualization Gate (`R10.22` / `R15.2`):** Matched `1,000,000`-cell `Column` scrolling, selection, and exclusion benchmarks stay strictly inside `<= 16.67 ms` steady-state frame time and `<= 50 ms` input-to-paint latency (`1.0x`/`2.0x` DPI).
5. **Clean-Room & Brand Gate (`cargo xtask brands && cargo xtask cleanroom && cargo xtask assets`):** Zero competitor trademarks, zero external suite identifiers, zero unattributed assets, and zero bundled font binaries.
