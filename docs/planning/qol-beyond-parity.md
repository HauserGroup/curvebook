# Post-Parity Quality-of-Life Enhancements (`M8+`)

> **Status:** Existing Wave A (`QOL-1`–`QOL-9`, including diagnostics-only `QOL-7` aligned in Waves 20–22b) and active Wave B (`QOL-12`, `QOL-14`, equations-only `QOL-16`, and `QOL-17` `WF-29`–`WF-49` extended `gui_only` suite) are implemented. `QOL-10`, `QOL-11`, `QOL-13`, and `QOL-15` remain deferred without scheduled milestones. Compact Letter Display (`CLD`) stays outside `QOL-16` and is tracked as [`R10.11`](roadmap-history/r10.md#r1011-compact-letter-display-cld). The `QOL-17` workflows run on the semantic agent harness; human-operable GUI completion is tracked in [`R10`](../ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion) · **Last updated:** 2026-11-15 · **Roadmap:** [`ROADMAP.md#r9`](../ROADMAP.md#r9-post-parity-scientific-quality-of-life-suite) · **Implementation evidence:** [`STATUS.md`](../STATUS.md)

This document tracks quality-of-life (QoL) scope and its implementation evidence separately. All active Wave A and Wave B scope items are complete (`0 h` remaining): diagnostics-only assumption inspection (`QOL-7`), table-analysis completions (`QOL-12`), experimental subject randomization and sampling (`QOL-14`), symbolic and live-substituted $\TeX$ equation callouts (`QOL-16`), and the extended 21-workflow multivariable/clustering/GLM/graphing `gui_only` acceptance suite (`QOL-17`). Compact Letter Display (`CLD`) is outside this document's scope and tracked in `R10.11`. Supporting specifications are in [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md) and [`gui-workflows.md`](../gui-workflows.md).

**Deferral policy:** Deferred items are retained for possible future reconsideration, outside the active implementation queue and effort totals. Existing viewer, unpacked-directory, and checkpoint code, file compatibility, and safety fixes remain supported; this planning change does not remove them. Deferring `QOL-10` does not defer core WASM compatibility or the baseline browser host work in `R8.4`. No implementation changes or new release dates are made by this scope revision.

---

## Guardrails & Non-Breaking Invariants

Every item in this backlog must satisfy five invariants:

1. **Zero muscle-memory disruption:** Classic single-sheet navigation, keyboard range habits, and standard 8-archetype table layouts remain the default behavior. Every QoL workflow is additive or opt-in.
2. **Additive, backward-compatible persistence:** New metadata fields in `.curvebook` (`manifest.json`, sheet JSON specs) must use `#[serde(default, skip_serializing_if = "...")]` so older readers and clean-room CSV / `.pzfx` / `.prism` interchange pipelines never fail or silently alter numeric values.
3. **Identical numerical & statistical results:** QoL features never alter solver tolerances, degrees-of-freedom formulas, tie handling, or NIST StRD benchmark outputs.
4. **Everything is a serializable command:** Interactive helpers (table archetype conversion, plate mapping, split-view brushing) dispatch through `crates/engine` (`CommandSpec`) or `UI_COMMANDS` (`crates/ui-egui` -> `crates/mcp`) with full undo/redo coverage.
5. **100% client-side WASM parity (`curvebook-web`):** Features must compile cleanly under `wasm32-unknown-unknown` with `#![forbid(unsafe_code)]`, require no backend server for private experimental data, and degrade gracefully across Chromium, Firefox, and Safari filesystem sandboxes.

---

## Summary & Effort Matrix

Unchanged estimates below are historical scope estimates, following [`target-app-parity.md`](../target-app-parity.md#calibration), rather than verified remaining work. Revised items require re-estimation; deferred estimates are excluded from active totals.

| ID | Enhancement | Area | Crates (`L0`–`L6`) | Est. hours | Status |
|---|---|---|---|---:|---|
| [QOL-1](#qol-1-non-destructive-table-archetype-conversion) | Non-destructive table archetype conversion (`Column` ↔ `Grouped` ↔ `XY`) | Data Tables | `doc`, `transform`, `engine`, `ui-egui` | 8–14 | **Done** (`R9.1`) |
| [QOL-2](#qol-2-plate-reader-8x12--16x24-and-tidy-long-to-subcolumn-paste-mapper) | 96/384-well plate & tidy long-to-wide paste mapper | Data Import | `doc`, `format`, `transform`, `engine`, `ui-egui` | 12–20 | **Done** (`R9.2`) |
| [QOL-3](#qol-3-auditable-cell-exclusion-reasons-and-provenance-tags) | Auditable cell exclusion reasons & provenance tags | Data Integrity | `doc`, `format`, `engine`, `ui-egui` | 5–8 | **Done** (`R9.3`) |
| [QOL-4](#qol-4-optional-side-by-side-split-workspace) | Optional side-by-side split workspace (Data/Results + live Graph) | Workspace UX | `ui-egui`, `mcp` | 8–14 | **Done** (`R9.4`) |
| [QOL-5](#qol-5-two-way-data-to-graph-point-brushing) | Two-way point brushing (hover/click graph glyph ↔ source table cell) | Graphing & QC | `chart`, `render`, `tools`, `ui-egui` | 8–12 | **Done** (`R9.5`) |
| [QOL-6](#qol-6-one-click-methods-and-statistical-reporting-generator) | One-click Methods & statistical reporting paragraph (plain text / Markdown / LaTeX) | Biostatistics | `stats`, `fit`, `engine`, `ui-egui` | 6–10 | **Done** (`R9.6`) |
| [QOL-7](#qol-7-inline-assumption-diagnostic-banners-diagnostics-only) | Informational assumption diagnostics; no test recommendations or remediation actions | Biostatistics | `stats`, `ui-egui` | 0 | **Done (diagnostics only)** (`R9.7`, Waves 20–22b) |
| [QOL-8](#qol-8-journal-figure-dimension-and-typography-lock-presets) | Journal column-width presets with locked print typography & stroke scaling | Graphing & Layouts | `doc`, `chart`, `engine`, `ui-egui` | 6–10 | **Done** (`R9.8`) |
| [QOL-9](#qol-9-live-color-vision-deficiency-cvd-and-grayscale-canvas-preview) | Live colorblind (CVD) & grayscale print preflight toggle on canvas | Accessibility | `palette`, `render`, `ui-egui` | 4–7 | **Done** (`R9.9`) |
| [QOL-10](#qol-10-zero-install-client-side-wasm-viewer-and-browser-folder-workspaces) | Zero-install client-side WASM collaborator viewer & browser folder workspaces | Web / Collaboration | `format`, `ui-egui`, `curvebook-web` | Deferred | **Deferred; existing code retained** (`R9.10`) |
| [QOL-11](#qol-11-unpacked-git-friendly-project-directory-and-in-file-checkpoints) | Unpacked Git-friendly directory mode & named checkpoints | Persistence | `format`, `engine`, `ui-egui` | Deferred | **Deferred; existing code retained** (`R9.11`) |
| **Wave A active scope** | **`QOL-1`–`QOL-9` (incl. diagnostics-only `QOL-7`)** | | | **0 remaining** | **Done (`R9.1`–`R9.9`)** |
| [QOL-12](#qol-12-categorical-and-continuous-table-analysis-completions) | Factorial ANOVA post-hoc tabs, Cohen/Fleiss $\kappa$, `NNT` CIs, custom GOF $\chi^2$, Binomial/Sign tests, $\chi^2$ SD CIs & summary *t*-tests | Biostatistics | `doc`, `stats`, `engine`, `ui-egui` | 0 | **Done** (`R9.12`, Wave 21) |
| [QOL-13](#qol-13-instant-statistical-distribution-and-bayesian-calculators) | Stateless statistical, distribution, and Bayesian calculators | Calculators & DSL | `stats`, `transform`, `engine`, `mcp`, `ui-egui` | Deferred | **Deferred; no scheduled milestone** (`R9.13`) |
| [QOL-14](#qol-14-experimental-subject-randomization-and-random-subset-sampling) | Seeded complete/permuted-block group randomization, uniform/stratified $k$-of-$N$ row sampling & multi-distribution table generation | Experimental Design | `doc`, `transform`, `engine`, `ui-egui` | 0 | **Done** (`R9.14`, Wave 22) |
| [QOL-15](#qol-15-bench-molarity-dilution-and-radiochemical-decay-calculators) | Molarity, dilution, moles/grams, and radiochemical calculators | Bench Lab Tools | `stats`, `engine`, `ui-egui` | Deferred | **Deferred; no scheduled milestone** (`R9.15`) |
| [QOL-16](#qol-16-live-tex-model-equation-annotations-equations-only) | Symbolic and live-substituted $\TeX$-subset best-fit equation callouts only | Graphing | `doc`, `fit`, `text`, `chart`, `engine`, `ui-egui` | 0 | **Done (equations only)** (`R9.16`, Wave 22b) |
| [QOL-17](#qol-17-extended-multivariable-clustering-glm-and-statistical-graphing-gui_only-suite) | 21-workflow extended multivariable, clustering, GLM, survival & graphing `gui_only` acceptance suite (`WF-29`–`WF-49`) | GUI Parity & QA | `ui-egui`, `mcp`, `testkit` | 0 | **Done (`WF-29`–`WF-49`)** (`R9.17`, Wave 23) |
| **Wave B active scope** | **Statistical completions, randomization, equation callouts, and extended `gui_only` workflows** | | | **0 remaining** | **Done (`R9.12`, `R9.14`, `R9.16`, `R9.17`; Waves 21–23)** |

---

## Scoped Specifications

### QOL-1 Non-destructive table archetype conversion
- **Crates:** `crates/doc`, `crates/transform`, `crates/engine`, `crates/ui-egui` · **Est.:** `8–14 h`
- **Problem:** Entering or pasting data into a `Column` table and subsequently realizing the experimental design requires a `Grouped` (two-way) or `XY` table traditionally forces manual re-creation of a new sheet, re-pasting subcolumns, and re-linking downstream analyses and graphs.
- **Scope:**
  - Add an engine command `table.convertArchetype` (`Column <-> Grouped`, `Column <-> Xy`, `Grouped <-> MultipleVariables`, `Xy <-> MultipleVariables`) with a live shape-mapping preview dialog before commit.
  - Explain clearly how replicate subcolumns, row titles, X columns, and cell exclusion masks map into the target archetype, warning explicitly before any lossy projection (and never truncating without confirmation).
  - Record the entire conversion as a single atomic entry in the `curvebook-engine` undo/redo journal, keeping the original `SheetId` intact where downstream `AnalysisSpec` / `GraphSpec` remain valid or prompting cleanly when downstream specs need retargeting.
- **Non-breaking invariant:** Existing `TableArchetype` variants and `DataSheet` storage invariants in `crates/doc` remain unchanged; conversion constructs a validated `DataSheet` replacement via existing constructors.

### QOL-2 Plate-reader (`8x12` / `16x24`) and tidy long-to-subcolumn paste mapper
- **Crates:** `crates/doc`, `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `12–20 h`
- **Problem:** Microplate readers (ELISA, luminescence, viability IC50 screens, qPCR) export `8×12` (96-well) or `16×24` (384-well) matrices, while R/Python/LIMS tools export tidy long-format tables (`Compound, Concentration, Replicate, Readout`). Reshaping these manually in external spreadsheets into wide replicate subcolumns is slow and error-prone.
- **Scope:**
  - Extend the 4-stage `Import & Paste Special` pipeline ([`ui-parity.md`](../ui-parity.md)) with two optional reshape modes in Stage 2 (`Filter & Reshape`):
    1. **Microplate Mapper (`96` / `384` wells):** Interactive visual well-plate grid allowing drag-assignment of dilution series (X values), treatment groups (columns), and technical replicates (subcolumns), plus reusable `.cbtemplate` plate maps and automatic blank-well baseline tagging.
    2. **Tidy Pivot (`Long -> Wide Subcolumns`):** Column picker mapping `(X / Row Factor, Group Column, Optional Replicate Index, Value, Optional Exclusion Flag)` directly into typed `XY`, `Column`, or `Grouped` replicate subcolumns.
- **Non-breaking invariant:** Outputs standard `DataSheet` cells and optional `ImportLinkSpec` provenance; downstream statistical and curve-fitting crates require zero awareness of plate geometry.

### QOL-3 Auditable cell exclusion reasons and provenance tags
- **Crates:** `crates/doc`, `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `5–8 h`
- **Problem:** Non-destructive cell exclusions (`*` / blue-italic styling) preserve raw numbers, but months later collaborators or auditors cannot tell *why* replicate `B3` was excluded (e.g., pipetting error, air bubble, contaminated well, or manual vs. automated ROUT outlier masking).
- **Scope:**
  - Allow an optional structured exclusion annotation `(reason_tag, note, timestamp_utc)` per excluded cell coordinate in `DataSheet` (defaulting to empty when toggling `Cmd/Ctrl+E` for zero extra keystrokes, with an optional `Shift+Cmd/Ctrl+E` *"Exclude with Reason…"* quick popover and hover tooltip).
  - Include exclusion annotations in a dedicated audit summary table when exporting structured reports or `.curvebook` archives.
- **Storage (landed, `R9.3`):** Exclusion masks stay as per-cell `excluded: bool` flags on the hot numerical path, while optional notes live in a sparse sidecar `DataSheet::exclusion_notes: Vec<CellExclusionNote>` (`coord`, `reason_tag`, `note`, `timestamp_utc`; `crates/doc/src/links.rs`) serialized with `skip_serializing_if = "Vec::is_empty"`. CSV and legacy exports ignore the sidecar cleanly.

### QOL-4 Optional side-by-side split workspace
- **Crates:** `crates/ui-egui`, `crates/mcp` · **Est.:** `8–14 h`
- **Problem:** Switching back and forth between a `Data` or `Result` sheet and its downstream `Graph` makes exploratory outlier checks, parameter constraint tuning, and axis adjustment tedious.
- **Scope:**
  - Keep single-sheet view as the default, and add a toolbar/shortcut toggle (`View › Split Right`) docking a live `Graph` or `Result` pane beside the active `Data` or `Result` sheet.
  - Edits in the spreadsheet grid, exclusions (`Cmd/Ctrl+E`), or non-modal inspector sliders immediately re-render the adjacent graph via the reactive `curvebook-engine` DAG.
- **Non-breaking invariant:** Pure `L6` (`curvebook-ui-egui`) viewport state; does not change `ProjectDocument` semantics or headless CLI output.

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

### QOL-7 Inline assumption diagnostic banners (diagnostics only)
- **Crates:** `crates/stats`, `crates/ui-egui` · **Est.:** Re-estimate for revised scope
- **Problem:** Normality, lognormality, homoscedasticity (Brown–Forsythe / Bartlett), and sphericity diagnostics are easy to overlook when buried in secondary result tabs.
- **Scope:**
  - Surface a compact, non-blocking status strip at the top of the primary Result tab showing the diagnostics performed, their values, and their limitations; distinguish unchecked or inconclusive diagnostics from evidence about an assumption.
  - Allow navigation to the underlying diagnostic tables and residual plots for inspection only.
  - Do not recommend tests, transformations, exclusions, or analysis changes. Do not rank alternatives, offer remediation buttons, duplicate an analysis, or change the selected method from the banner. This restriction also covers checklist text surfaced by the banner.
- **Acceptance:** A flagged diagnostic can be inspected without producing an alternative-test suggestion or any document mutation. Small sample size and failed normality checks must not trigger a recommendation.
- **Implementation status:** Aligned to diagnostics-only in Waves 20–22b: `AssumptionDiagnosticBanner` (defined in `crates/ui-egui/src/dialogs_canvas.rs`, drawn by `compositor.rs`; `sanitize_diagnostics_only_text` in `crates/ui-egui/src/dialogs_r9.rs`) renders diagnostic facts and opens existing diagnostic tabs via `ui.result.inspectDiagnostics` (`mutated: false`) without recommendations, remediation buttons, or sibling creation; deprecated `ui.result.applyRemediation` returns `ErrorCode::Conflict` (`details.code: "diagnostics_only_policy"`, `mutated: false`), while explicit `Cmd+T` parameter switching remains available (`WF-10`, `WF-14`, `gui_w22b_qol_and_r9`).
- **Non-breaking invariant:** Informational UI presentation backed by existing diagnostic outputs; no numerical changes or automatic analysis selection.

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
- **Planning status:** Deferred without a scheduled milestone. The scope below is retained for reference, not active implementation. Existing code remains supported; baseline browser hosting and core WASM checks are separate requirements.
- **Crates:** `crates/format`, `crates/ui-egui`, `apps/curvebook-web` · **Est.:** `10–16 h`
- **Problem:** Collaborators, principal investigators, students, and peer reviewers frequently receive project archives on machines without native software installed (or on Linux, Chromebooks, or tablets), while institutional data-governance policies prohibit uploading unpublished raw experimental data to third-party cloud servers.
- **Scope:**
  - **100% local WASM open/inspect/edit:** Visiting the hosted `curvebook-web` URL lets any user open `.curvebook`, CSV/TSV, or clean-room-imported `.prism`/`.pzfx` files via file picker or drag-and-drop, executing every solver and renderer locally inside the browser's WebAssembly sandbox with **zero network upload of user data**.
  - **Browser Folder Attachment:** Support attaching a local experiment directory (read-only drag-and-drop / directory input across Safari, Firefox, and Chromium; direct read/write project & batch-export folder access via the File System Access API on Chromium browsers, falling back cleanly to bundled `.zip` / `.curvebook` downloads elsewhere).
  - **Optional Read-Only Presentation Mode:** A clean viewer toggle hiding editing chrome for lab meetings and supplementary-material inspection.
- **Non-breaking invariant:** Preserves strict `wasm32-unknown-unknown` compatibility (`cargo xtask wasm`) and zero-JS/zero-backend architecture rules in [`AGENTS.md`](../../AGENTS.md).

### QOL-11 Unpacked Git-friendly project directory and in-file checkpoints
- **Planning status:** Deferred without a scheduled milestone. The scope below is retained for reference, not active implementation. Existing unpacked files, checkpoint data, and commands remain supported.
- **Crates:** `crates/format`, `crates/engine`, `crates/ui-egui` · **Est.:** `10–16 h`
- **Problem:** Researchers routinely duplicate entire project files (`Assay_v1.prism`, `Assay_v2_no_outlier.prism`, `Assay_FINAL.prism`) before trying alternative curve fits, and binary containers produce opaque diffs in Git/DVC repositories.
- **Scope:**
  - **Unpacked Directory Format (`.curvebook.d/`):** Mirror the exact internal file tree of the deterministic `.curvebook` ZIP container (`manifest.json`, canonical 2-space-indented JSON specs, RFC-4180 CSV data sheets) directly as a plain directory on disk so computational labs can track line-by-line Git diffs and attach folders directly in desktop or Chromium WASM workflows.
  - **Named Checkpoints:** Allow users to record named snapshots (e.g., *"Pre-exclusion raw fit"*, *"Submitted revision 1"*) inside a `.curvebook` session so they can compare or restore prior analysis states without file sprawl. Existing snapshots store full project JSON; no lightweight-storage claim is made.
- **Existing implementation:** Unpacked-directory I/O already ships as plain functions beside the ZIP path in `crates/format/src/archive.rs` (`is_unpacked_project_dir`, `save_project_unpacked_dir`, `open_project_unpacked_dir`) with the same schema validation; there is no `ArchiveReader` / `ArchiveWriter` trait.

### QOL-12 Categorical and continuous table-analysis completions
- **Crates:** `crates/doc`, `crates/stats`, `crates/engine`, `crates/ui-egui` · **Est.:** `8–13 h` · **Spec:** [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md#2-categorical--continuous-table-analysis-completions-r912--qol-12--implemented-wave-21)
- **Scope:**
  - Emit `ResultTabKind::MultipleComparisons` post-hoc tabs inside `run_factorial_anova` (`FactorialAnovaSpec::multiplicity`, also tracked on `R8.7` for `WF-09`/`WF-10`).
  - Expose `cohens_kappa` (`Unweighted`, `Linear`, `Quadratic`) and `fleiss_kappa` via `ContingencyMethod`, invert Newcombe `2×2` risk-difference bounds into Altman `NNTB`/`NNTH` CIs, emit McNemar discordant-pair OR CIs, add custom expected proportions to `PartsOfWhole` goodness-of-fit $\chi^2$, and add exact one-sample Binomial ($p_0 \in (0, 1)$) & paired Sign tests.
  - Add exact $\chi^2$ standard-deviation CIs (`sd_ci`) to `DescriptiveSummary` and direct `Mean + SD/SEM + N` support to `run_two_group_analysis`.
- **Non-breaking invariant:** Additive `#[serde(default)]` enum variants and optional fields only; existing `.curvebook` files and statistical oracle values remain unchanged.

### QOL-13 Instant statistical, distribution and Bayesian calculators
- **Planning status:** Deferred without a scheduled milestone. The scope below is retained for reference; it does not authorize new calculator commands, formula builtins, or UI drawers.
- **Crates:** `crates/stats`, `crates/transform`, `crates/engine`, `crates/mcp`, `crates/ui-egui` · **Est.:** `10–16 h` · **Spec:** [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md#3-instant-statistical-distribution--bayesian-calculators-r913--qol-13)
- **Scope:**
  - *Proposed (none of these IDs exist today):* add stateless read-only `calc.*` commands (a new `CommandGroup::Calculator` with `READ_ONLY` effects) and a non-modal **Calculators** drawer (`ui.calculator.open`) for table-free calculations: binomial proportion & Garwood exact Poisson count CIs, clinical Bayes PPV/NPV/$\text{LR}^{\pm}$ from sensitivity/specificity/prior prevalence, two-mean error propagation ($A \pm B$, $A \cdot B$, and Fieller's exact quotient CI for $A / B$), summary-table multi-way ANOVA post-tests, forward/inverse probability distributions ($z, t, F, r, \chi^2$, Binomial, Poisson, Gaussian — also exposed as scalar functions in `=` Calculated Variables), Sellke–Bayarri–Berger & Colquhoun Bayesian $P$-value / False Positive Risk interpretation, summary $F$/$\text{AIC}_c$ fit comparisons, and scalar $\text{EC}_F$ from $(\text{EC}_{50}, h, F)$.
- **Non-breaking invariant:** Zero document or undo-stack mutation during calculation; zero `.curvebook` archive version changes.

### QOL-14 Experimental subject randomization and random subset sampling
- **Crates:** `crates/doc`, `crates/transform`, `crates/engine`, `crates/ui-egui` · **Est.:** `6–10 h` · **Spec:** [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md#4-experimental-randomization--subset-sampling-r914--qol-14--implemented-wave-22)
- **Scope:**
  - Add `crates/transform/src/randomize.rs` over `DeterministicRng` providing seeded **Complete** and **Permuted-Block** subject-to-group randomization (`table.randomizeGroups`, writing seed & block metadata to `InfoSheet` for ARRIVE reporting), non-destructive uniform/stratified $k$-of-$N$ row subset sampling without replacement (`table.randomSampleRows` / `TransformAnalysisSpec::RandomSample`), and multi-distribution column simulation (`table.simulateDistributions`).
- **Non-breaking invariant:** Never mutates upstream raw observations in place; deterministic across native and `wasm32` targets.

### QOL-15 Bench molarity, dilution and radiochemical decay calculators
- **Planning status:** Deferred without a scheduled milestone. The scope below is retained for reference; no subset is moved into active work by this revision.
- **Crates:** `crates/stats`, `crates/engine`, `crates/ui-egui` · **Est.:** `6–10 h` · **Spec:** [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md#5-bench-molarity-dilution--radiochemical-calculators-r915--qol-15)
- **Scope:**
  - *Proposed (none of these IDs exist today):* add stateless wet-lab bench calculators (`calc.molarity`, `calc.dilution`, `calc.molesGrams`, `calc.radioactivity`) solving $m = C \cdot V \cdot \text{FW}$, single & serial stock dilutions ($C_1 V_1 = C_2 V_2$), moles $\leftrightarrow$ grams, and NUBASE2020-evaluated radionuclide half-life decay ($A(t) = A_0 2^{-\Delta t / t_{1/2}}$, carrier-free & decay-corrected specific activity, molar radioligand concentration, $\text{Bq} \leftrightarrow \text{Ci} \leftrightarrow \text{dpm}$), with optional one-click export to `InfoSheet` constants or `XY` dilution columns.
- **Non-breaking invariant:** Pure read-only `calc.*` queries unless the user explicitly exports constants or creates a new dilution sheet.

### QOL-16 Live $\TeX$ model equation annotations (equations only)
- **Crates:** `crates/doc`, `crates/fit`, `crates/text`, `crates/chart`, `crates/engine`, `crates/ui-egui` · **Est.:** Re-estimate for equations-only scope · **Spec:** [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md#6-live-best-fit-equation-annotations-r916--qol-16-equations-only--implemented-wave-22b)
- **Scope:**
  - Add one-click insertion of symbolic or live-parameterized $\TeX$-subset best-fit model equations onto fitted `XY` graphs, rendered deterministically across RGBA, SVG, and `%PDF-1.7`.
  - Refresh substituted coefficients when the linked fit recalculates; keep symbolic form available without copying values manually.
- **Excluded from this item:** Compact Letter Display (now tracked separately in [`R10.11`](roadmap-history/r10.md#r1011-compact-letter-display-cld)), group letters, pairwise-display modes, significance-matrix algorithms, and additional fit-statistic callouts. Do not implement them as prerequisites or optional extras under this item.
- **Acceptance:** Symbolic and substituted equations stay linked to the selected fit and render consistently on screen and in exports; existing significance brackets remain unchanged.
- **Implementation status:** Done (`R9.16`, Wave 22b) as `GraphAnnotation::ModelEquation`, inserted via `graph.addAnnotation` (kind `model_equation`) or `ui.graph.insertModelEquation`; the two-dimensional typeset rendering upgrade is planned in [`R10.19`](roadmap-history/r10.md#r1019-typeset-mathematical-equation-annotations).
- **Non-breaking invariant:** Add equation annotations through versioned graph metadata and existing commands; preserve existing graph rendering and archive safety guarantees.

### QOL-17 Extended multivariable, clustering, GLM and statistical-graphing `gui_only` suite
- **Crates:** `crates/ui-egui`, `crates/mcp`, `crates/testkit` · **Status:** Done (`0 h` remaining, Wave 23) · **Spec:** [`gui-workflows.md`](../gui-workflows.md#5-extended-21-workflow-multivariable-clustering-glm--statistical-graphing-gui_only-matrix-r917--qol-17-implemented)
- **Problem:** While `R8.8` (`WF-01`–`WF-28`) verifies Core two-group, ANOVA, regression, contingency, PCA/PCR, and foundational graph workflows under `GuiOnlyPolicy::Strict`, deeper multivariable role-bound workflows (formula-bar `Calculated Variables` circularity guards, $2^{-\Delta\Delta C_t}$ qPCR pipelines, direct `MultipleVariables` Welch *t* / Multifactor ANOVA / Kaplan–Meier / multi-series 4PL dose–response, Cox PH, Poisson count GLMs, full linkage/metric clustering sweeps, synchronized survival `Number at Risk` re-binning, multivariable symbol sizing, and `R9.16` live $\TeX$ equation annotations) benefit from dedicated end-to-end `gui_only` regression shards.
- **Scope:**
  - Add 21 strict `gui_only` integration workflows (`WF-29`–`WF-49`) across 3 shards (`gui_wf_multivariable_and_transforms`, `gui_wf_clustering_glm_and_pharma`, `gui_wf_advanced_statistical_graphs`) driving `UiApp` exclusively through the 8 `ui.*` / `gui` perception and input primitives over original synthetic datasets in `curvebook-testkit`, closing any inspector/dialog `ui.inspect` control gaps surfaced along the way.
- **Excluded from this item:** Compact Letter Display (`CLD`, tracked in `R10.11` as `WF-50`), group-letter displays, extra fit-statistic callouts (excluded per `QOL-16`), and assumption-banner test recommendations or remediation actions (excluded per `QOL-7`).
- **Non-breaking invariant:** Pure `L6` inspection coverage and integration test shards; alters zero `.curvebook` persistence formats or numerical solver tolerances.
