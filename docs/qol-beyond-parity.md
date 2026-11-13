# Post-Parity Quality-of-Life Enhancements (`M8+`)

> **Status:** Deferred post-parity backlog (`M8+`) · **Last updated:** 2026-11-13 · **Prerequisite:** Alpha/Beta/1.0 parity gates ([`roadmap.md`](roadmap.md), [`gaps.md`](gaps.md) `G1`–`G14`)

This document scopes **non-breaking quality-of-life (QoL) improvements** planned after core scientific workflow parity (`G1`–`G14`). None of these items block Alpha, Beta, or `1.0` parity; however, documenting their scope and non-breaking invariants early ensures that foundational `L1`–`L6` schemas (`lumencraft-doc`, `lumencraft-format`, `lumencraft-engine`, `lumencraft-ui-egui`) remain forward-compatible without breaking migrations later.

---

## Guardrails & Non-Breaking Invariants

Every item in this backlog must satisfy five invariants:

1. **Zero muscle-memory disruption:** Classic single-sheet navigation, keyboard range habits, and standard 8-archetype table layouts remain the default behavior. Every QoL workflow is additive or opt-in.
2. **Additive, backward-compatible persistence:** New metadata fields in `.lumencraft` (`manifest.json`, sheet JSON specs) must use `#[serde(default, skip_serializing_if = "...")]` so older readers and clean-room CSV / `.pzfx` / `.prism` interchange pipelines never fail or silently alter numeric values.
3. **Identical numerical & statistical results:** QoL features never alter solver tolerances, degrees-of-freedom formulas, tie handling, or NIST StRD benchmark outputs.
4. **Everything is a serializable command:** Interactive helpers (table archetype conversion, plate mapping, split-view brushing) dispatch through `crates/engine` (`CommandSpec`) or `UI_COMMANDS` (`crates/ui-egui` -> `crates/mcp`) with full undo/redo coverage.
5. **100% client-side WASM parity (`lumencraft-web`):** Features must compile cleanly under `wasm32-unknown-unknown` with `#![forbid(unsafe_code)]`, require no backend server for private experimental data, and degrade gracefully across Chromium, Firefox, and Safari filesystem sandboxes.

---

## Summary & Effort Matrix

Hours follow the single-agent calibration in [`target-app-parity.md`](target-app-parity.md#calibration).

| ID | Enhancement | Area | Crates (`L0`–`L6`) | Est. hours | Earliest hook |
|---|---|---|---|---:|---|
| [QOL-1](#qol-1-non-destructive-table-archetype-conversion) | Non-destructive table archetype conversion (`Column` ↔ `Grouped` ↔ `XY`) | Data Tables | `doc`, `transform`, `engine`, `ui-egui` | 8–14 | `G2` / `G6` |
| [QOL-2](#qol-2-plate-reader-8x12--16x24-and-tidy-long-to-subcolumn-paste-mapper) | 96/384-well plate & tidy long-to-wide paste mapper | Data Import | `doc`, `format`, `engine`, `ui-egui` | 12–20 | `G2` / `G6` |
| [QOL-3](#qol-3-auditable-cell-exclusion-reasons-and-provenance-tags) | Auditable cell exclusion reasons & provenance tags | Data Integrity | `doc`, `format`, `engine`, `ui-egui` | 5–8 | `G2` |
| [QOL-4](#qol-4-optional-side-by-side-split-workspace) | Optional side-by-side split workspace (Data/Results + live Graph) | Workspace UX | `ui-egui`, `mcp` | 8–14 | `G6` |
| [QOL-5](#qol-5-two-way-data-to-graph-point-brushing) | Two-way point brushing (hover/click graph glyph ↔ source table cell) | Graphing & QC | `chart`, `render`, `tools`, `ui-egui` | 8–12 | `G5` / `G6` |
| [QOL-6](#qol-6-one-click-methods-and-statistical-reporting-generator) | One-click Methods & statistical reporting paragraph (plain text / Markdown / LaTeX) | Biostatistics | `stats`, `fit`, `engine`, `ui-egui` | 6–10 | `G3` / `G4` |
| [QOL-7](#qol-7-inline-assumption-diagnostic-banners-with-non-blocking-remediation) | Inline assumption diagnostic banners with one-click test alternatives | Biostatistics | `stats`, `ui-egui` | 5–9 | `G3` / `G6` |
| [QOL-8](#qol-8-journal-figure-dimension-and-typography-lock-presets) | Journal column-width presets with locked print typography & stroke scaling | Graphing & Layouts | `doc`, `chart`, `ui-egui` | 6–10 | `G5` / `G11` |
| [QOL-9](#qol-9-live-color-vision-deficiency-cvd-and-grayscale-canvas-preview) | Live colorblind (CVD) & grayscale print preflight toggle on canvas | Accessibility | `palette`, `render`, `ui-egui` | 4–7 | `G5` / `G6` |
| [QOL-10](#qol-10-zero-install-client-side-wasm-viewer-and-browser-folder-workspaces) | Zero-install client-side WASM collaborator viewer & browser folder workspaces | Web / Collaboration | `format`, `ui-egui`, `lumencraft-web` | 10–16 | `G2` / `G6` |
| [QOL-11](#qol-11-unpacked-git-friendly-project-directory-and-in-file-checkpoints) | Unpacked Git-friendly directory mode & lightweight named checkpoints | Persistence | `format`, `engine`, `ui-egui` | 10–16 | `G2` |
| | **Total post-parity QoL backlog** | | | **82–136 h** | |

---

## Scoped Specifications

### QOL-1 Non-destructive table archetype conversion
- **Crates:** `crates/doc`, `crates/transform`, `crates/engine`, `crates/ui-egui` · **Est.:** `8–14 h`
- **Problem:** Entering or pasting data into a `Column` table and subsequently realizing the experimental design requires a `Grouped` (two-way) or `XY` table traditionally forces manual re-creation of a new sheet, re-pasting subcolumns, and re-linking downstream analyses and graphs.
- **Scope:**
  - Add an engine command `table.convertArchetype` (`Column <-> Grouped`, `Column <-> Xy`, `Grouped <-> MultipleVariables`, `Xy <-> MultipleVariables`) with a live shape-mapping preview dialog before commit.
  - Explain clearly how replicate subcolumns, row titles, X columns, and cell exclusion masks map into the target archetype, warning explicitly before any lossy projection (and never truncating without confirmation).
  - Record the entire conversion as a single atomic entry in the `lumencraft-engine` undo/redo journal, keeping the original `SheetId` intact where downstream `AnalysisSpec` / `GraphSpec` remain valid or prompting cleanly when downstream specs need retargeting.
- **Non-breaking invariant:** Existing `TableArchetype` variants and `DataSheet` storage invariants in `crates/doc` remain unchanged; conversion constructs a validated `DataSheet` replacement via existing constructors.

### QOL-2 Plate-reader (`8x12` / `16x24`) and tidy long-to-subcolumn paste mapper
- **Crates:** `crates/doc`, `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `12–20 h`
- **Problem:** Microplate readers (ELISA, luminescence, viability IC50 screens, qPCR) export `8×12` (96-well) or `16×24` (384-well) matrices, while R/Python/LIMS tools export tidy long-format tables (`Compound, Concentration, Replicate, Readout`). Reshaping these manually in external spreadsheets into wide replicate subcolumns is slow and error-prone.
- **Scope:**
  - Extend the 4-stage `Import & Paste Special` pipeline ([`ui-parity.md`](ui-parity.md)) with two optional reshape modes in Stage 2 (`Filter & Reshape`):
    1. **Microplate Mapper (`96` / `384` wells):** Interactive visual well-plate grid allowing drag-assignment of dilution series (X values), treatment groups (columns), and technical replicates (subcolumns), plus reusable `.lctemplate` plate maps and automatic blank-well baseline tagging.
    2. **Tidy Pivot (`Long -> Wide Subcolumns`):** Column picker mapping `(X / Row Factor, Group Column, Optional Replicate Index, Value, Optional Exclusion Flag)` directly into typed `XY`, `Column`, or `Grouped` replicate subcolumns.
- **Non-breaking invariant:** Outputs standard `DataSheet` cells and optional `ImportLinkSpec` provenance; downstream statistical and curve-fitting crates require zero awareness of plate geometry.

### QOL-3 Auditable cell exclusion reasons and provenance tags
- **Crates:** `crates/doc`, `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `5–8 h`
- **Problem:** Non-destructive cell exclusions (`*` / blue-italic styling) preserve raw numbers, but months later collaborators or auditors cannot tell *why* replicate `B3` was excluded (e.g., pipetting error, air bubble, contaminated well, or manual vs. automated ROUT outlier masking).
- **Scope:**
  - Allow an optional structured exclusion annotation `(reason_tag, note, timestamp_utc)` per excluded cell coordinate in `DataSheet` (defaulting to empty when toggling `Cmd/Ctrl+E` for zero extra keystrokes, with an optional `Shift+Cmd/Ctrl+E` *"Exclude with Reason…"* quick popover and hover tooltip).
  - Include exclusion annotations in a dedicated audit summary table when exporting structured reports or `.lumencraft` archives.
- **Forward-compatibility hook (`G2`):** Keep exclusion masks fast bitsets/sets on the hot numerical path in `DataSheet`, storing optional exclusion notes in a sparse sidecar map (`BTreeMap<CellCoord, ExclusionNote>`) serialized with `skip_serializing_if = "BTreeMap::is_empty"`. CSV and legacy exports ignore the sidecar cleanly.

### QOL-4 Optional side-by-side split workspace
- **Crates:** `crates/ui-egui`, `crates/mcp` · **Est.:** `8–14 h`
- **Problem:** Switching back and forth between a `Data` or `Result` sheet and its downstream `Graph` makes exploratory outlier checks, parameter constraint tuning, and axis adjustment tedious.
- **Scope:**
  - Keep single-sheet view as the default, and add a toolbar/shortcut toggle (`View › Split Right`) docking a live `Graph` or `Result` pane beside the active `Data` or `Result` sheet.
  - Edits in the spreadsheet grid, exclusions (`Cmd/Ctrl+E`), or non-modal inspector sliders immediately re-render the adjacent graph via the reactive `lumencraft-engine` DAG.
- **Non-breaking invariant:** Pure `L6` (`lumencraft-ui-egui`) viewport state; does not change `ProjectDocument` semantics or headless CLI output.

### QOL-5 Two-way data-to-graph point brushing
- **Crates:** `crates/chart`, `crates/render`, `crates/tools`, `crates/ui-egui` · **Est.:** `8–12 h`
- **Problem:** When inspecting scatter plots, dose–response curves, or bar-with-replicate beeswarm graphs, locating the exact `(Row, Column, Subcolumn)` cell responsible for an anomalous point requires manual hunting through large tables.
- **Scope:**
  - Emit deterministic `SourceCellId { sheet_id, row, col, subcol }` hit-test metadata alongside point glyphs in `ChartScene` (`crates/chart`).
  - Hovering a plotted replicate point shows a rich tooltip (row title, column/group, replicate ID, X/Y values) and—when Split View (`QOL-4`) is open—highlights the corresponding cell in the grid (and vice versa).
  - Right-clicking a point offers *Jump to Data Cell* and *Exclude / Include Observation* via standard engine commands.
- **Forward-compatibility hook (`G5`):** Carrying an optional lightweight `SourceCellId` on `ChartScene` point primitives during `G5` costs negligible memory and avoids retrofitting hit-testing later.

### QOL-6 One-click Methods and statistical reporting generator
- **Crates:** `crates/stats`, `crates/fit`, `crates/engine`, `crates/ui-egui` · **Est.:** `6–10 h`
- **Problem:** Manuscript peer review frequently flags incomplete statistical reporting (missing Welch vs. pooled specification, Geisser–Greenhouse $\hat{\varepsilon}$, REML degrees-of-freedom approximation, exact multiplicity procedure, two-sidedness, test statistic with $df$, exact $p$-value, and standardized effect size with CI), and manual transcription introduces typos.
- **Scope:**
  - Add a read-only **Copy Reporting Summary** action on `ResultSheet` producing deterministic, citation-accurate text in **Plain Text**, **Markdown**, or **LaTeX** directly from `AnalysisSpec` + `ResultSheet` values (using original, clean-room scientific prose aligned with ICMJE / ARRIVE reporting guidelines).
  - Cover both statistical comparisons and nonlinear regression (equation formula, weighting scheme, robust/ROUT settings, shared global parameters, AICc / extra-sum-of-squares *F*-test outcome, and profile-likelihood CI method).
- **Non-breaking invariant:** Pure read-only formatter over `AnalysisSpec` and `ResultSheet`; alters zero stored or computed numbers.

### QOL-7 Inline assumption diagnostic banners with non-blocking remediation
- **Crates:** `crates/stats`, `crates/ui-egui` · **Est.:** `5–9 h`
- **Problem:** Normality, lognormality, homoscedasticity (Brown–Forsythe / Bartlett), and sphericity diagnostics are easy to overlook when buried in secondary result tabs.
- **Scope:**
  - Surface a compact, non-blocking status strip at the top of the primary Result tab summarizing checked assumptions (e.g., severe skewness/unequal variance detected on small or unbalanced groups, or candidate lognormal structure).
  - Offer one-click non-destructive actions (*Duplicate as Welch ANOVA*, *View Residual Diagnostics*) that invoke `Change › Analysis Parameters…` or create a sibling analysis without ever overriding the researcher's chosen test automatically.
- **Non-breaking invariant:** Purely advisory UI presentation backed by existing `G3`/`G4` diagnostic outputs; strictly clean-room wording.

### QOL-8 Journal figure dimension and typography lock presets
- **Crates:** `crates/doc`, `crates/chart`, `crates/ui-egui` · **Est.:** `6–10 h`
- **Problem:** Resizing a finished graph to fit a journal's physical column width (`85–89 mm` single column, `120–136 mm` 1.5-column, `175–183 mm` double column, or `16:9` presentation slide) often scales axis titles and tick labels down to illegible `< 5 pt` sizes or leaves hairlines too thin for print.
- **Scope:**
  - Provide built-in **Target Canvas Size Presets** (generic metric/inch column targets—single-column `88 mm`, 1.5-column `130 mm`, double-column `180 mm`, full-page portrait, `16:9` slide, poster panel) with a **Resize Mode** toggle: *Scale Geometries Only (Keep Typography `pt` & Minimum Stroke Width Fixed)* vs. *Proportional Scale*.
  - Optional preflight warning badge before PDF/SVG/TIFF export if any visible label falls below `5 pt` or stroke below `0.25 pt` at target print dimensions.
- **Non-breaking invariant:** Operates through standard `GraphSpec` / `LayoutSpec` physical dimension and style commands; uses neutral dimension presets (no third-party publisher trademarks or templates).

### QOL-9 Live color-vision-deficiency (CVD) and grayscale canvas preview
- **Crates:** `crates/palette`, `crates/render`, `crates/ui-egui` · **Est.:** `4–7 h`
- **Problem:** Multi-group curves and heatmaps that look distinct on an RGB monitor often collapse into indistinguishable shades for readers with deuteranopia/protanopia or when printed in black-and-white.
- **Scope:**
  - Expose the Machado (2009) CVD matrices and luminance grayscale conversion already implemented in `crates/palette` (`L0`) as a non-persistent **Canvas Preview Filter** dropdown (`Normal`, `Deuteranopia`, `Protanopia`, `Tritanopia`, `Grayscale Print`) in the Graph and Layout viewports, paired with WCAG contrast warnings.
- **Non-breaking invariant:** Viewport-only display filter (`L3`/`L6`); never mutates `GraphSpec` colors or exported SVG/PDF bytes unless the user explicitly switches palettes.

### QOL-10 Zero-install client-side WASM viewer and browser folder workspaces
- **Crates:** `crates/format`, `crates/ui-egui`, `apps/lumencraft-web` · **Est.:** `10–16 h`
- **Problem:** Collaborators, principal investigators, students, and peer reviewers frequently receive project archives on machines without native software installed (or on Linux, Chromebooks, or tablets), while institutional data-governance policies prohibit uploading unpublished raw experimental data to third-party cloud servers.
- **Scope:**
  - **100% local WASM open/inspect/edit:** Visiting the hosted `lumencraft-web` URL lets any user open `.lumencraft`, CSV/TSV, or clean-room-imported `.prism`/`.pzfx` files via file picker or drag-and-drop, executing every solver and renderer locally inside the browser's WebAssembly sandbox with **zero network upload of user data**.
  - **Browser Folder Attachment:** Support attaching a local experiment directory (read-only drag-and-drop / directory input across Safari, Firefox, and Chromium; direct read/write project & batch-export folder access via the File System Access API on Chromium browsers, falling back cleanly to bundled `.zip` / `.lumencraft` downloads elsewhere).
  - **Optional Read-Only Presentation Mode:** A clean viewer toggle hiding editing chrome for lab meetings and supplementary-material inspection.
- **Non-breaking invariant:** Preserves strict `wasm32-unknown-unknown` compatibility (`cargo xtask wasm`) and zero-JS/zero-backend architecture rules in [`AGENTS.md`](../AGENTS.md).

### QOL-11 Unpacked Git-friendly project directory and in-file checkpoints
- **Crates:** `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `10–16 h`
- **Problem:** Researchers routinely duplicate entire project files (`Assay_v1.prism`, `Assay_v2_no_outlier.prism`, `Assay_FINAL.prism`) before trying alternative curve fits, and binary containers produce opaque diffs in Git/DVC repositories.
- **Scope:**
  - **Unpacked Directory Format (`.lumencraft.d/`):** Mirror the exact internal file tree of the deterministic `.lumencraft` ZIP container (`manifest.json`, canonical 2-space-indented JSON specs, RFC-4180 CSV data sheets) directly as a plain directory on disk so computational labs can track line-by-line Git diffs and attach folders directly in desktop or Chromium WASM workflows.
  - **Lightweight Named Checkpoints:** Allow users to record named snapshots (e.g., *"Pre-exclusion raw fit"*, *"Submitted revision 1"*) inside a `.lumencraft` session so they can compare or restore prior analysis states without file sprawl.
- **Forward-compatibility hook (`G2`):** Design `crates/format` around a virtual archive trait (`ArchiveReader` / `ArchiveWriter`) backed by deterministic ZIP first (`G2`), making unpacked-directory I/O a thin second backend with identical schema validation.
