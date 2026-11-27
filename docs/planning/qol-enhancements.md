# Quality-of-Life Enhancements (`M8+`)

> **Status:** All `13` active Wave A (`QOL-1`–`QOL-9`, diagnostics-only `QOL-7`) and Wave B (`QOL-12`, `QOL-14`, equations-only `QOL-16`, and `QOL-17`) items are **done** ([`history/roadmap/r9.md`](../history/roadmap/r9.md)). Four items (`QOL-10`, `QOL-11`, `QOL-13`, `QOL-15`) remain **deferred** without scheduled milestones. Compact Letter Display (`CLD`) shipped separately under [`R10.11`](../history/roadmap/r10.md#r1011-compact-letter-display-cld); legacy `gui_only` `QOL-17` tests were superseded by five-stage `human_wf_*` desktop suites ([`R10.25`](../history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate), [`R21.1`](../history/roadmap/r21.md#r211-retire---gui-only-and-synthetic-ui-automation-plumbing)) · **Updated:** 2026-11-27 · **Roadmap:** [`ROADMAP.md#r9`](../ROADMAP.md#r9-post-parity-scientific-quality-of-life-suite) · **Evidence:** [`STATUS.md`](../STATUS.md)

**Deferral policy:** Deferred items stay outside the active queue; existing unpacked-directory, checkpoint, and `VirtualWorkspaceFolder` code and file compatibility remain supported. Supporting specifications live in [`calculators-and-quick-stats.md`](calculators-and-quick-stats.md) and [`gui-workflows.md`](../gui-workflows.md).

---

## Guardrails & Non-Breaking Invariants

1. **Zero muscle-memory disruption:** Classic single-sheet navigation, keyboard range habits, and standard 8-archetype table layouts remain default; QoL features are additive or opt-in.
2. **Additive, backward-compatible persistence:** New metadata fields in `.curvebook` use `#[serde(default, skip_serializing_if = "...")]` so older readers and CSV / `.pzfx` / `.prism` interchange stay compatible.
3. **Identical numerical & statistical results:** QoL helpers never alter solver tolerances, *df* formulas, tie handling, or NIST StRD benchmark outputs.
4. **Serializable commands:** Interactive helpers dispatch through `crates/engine` (`CommandSpec`) or `UI_COMMANDS` with full undo/redo coverage.
5. **100% offline & WASM-clean (`#![forbid(unsafe_code)]`):** Pure Rust with zero outbound network requests.

---

## Summary & Effort Matrix

| ID | Enhancement | Area | Crates (`L0`–`L6`) | Status |
|---|---|---|---|---|
| [QOL-1](#qol-1-non-destructive-table-archetype-conversion) | Non-destructive table archetype conversion (`Column` ↔ `Grouped` ↔ `XY` ↔ `MultipleVariables`) | Data Tables | `doc`, `transform`, `engine`, `ui-egui` | **Done** (`R9.1`) |
| [QOL-2](#qol-2-plate-reader-8x12--16x24-and-tidy-long-to-subcolumn-paste-mapper) | 96/384-well plate & tidy long-to-wide paste mapper | Data Import | `doc`, `format`, `transform`, `engine`, `ui-egui` | **Done** (`R9.2`, extended in `R17.7`) |
| [QOL-3](#qol-3-auditable-cell-exclusion-reasons-and-provenance-tags) | Auditable cell exclusion reasons & provenance tags | Data Integrity | `doc`, `format`, `engine`, `ui-egui` | **Done** (`R9.3`) |
| [QOL-4](#qol-4-optional-side-by-side-split-workspace) | Optional side-by-side split workspace (Data/Results + live Graph) | Workspace UX | `ui-egui`, `mcp` | **Done** (`R9.4`, `R20.3`) |
| [QOL-5](#qol-5-two-way-data-to-graph-point-brushing) | Two-way point brushing (graph glyph ↔ source table cell) | Graphing & QC | `chart`, `render`, `tools`, `ui-egui` | **Done** (`R9.5`) |
| [QOL-6](#qol-6-one-click-methods-and-statistical-reporting-generator) | One-click Methods & statistical reporting paragraph (Plain / Markdown / LaTeX) | Biostatistics | `stats`, `fit`, `engine`, `ui-egui` | **Done** (`R9.6`, `R20.7`) |
| [QOL-7](#qol-7-inline-assumption-diagnostic-banners-diagnostics-only) | Informational assumption diagnostics (no recommendations or remediation actions) | Biostatistics | `stats`, `ui-egui` | **Done (diagnostics only)** (`R9.7`) |
| [QOL-8](#qol-8-journal-figure-dimension-and-typography-lock-presets) | Journal column-width presets with locked print typography & stroke preflight | Graphing & Layouts | `doc`, `chart`, `engine`, `ui-egui` | **Done** (`R9.8`) |
| [QOL-9](#qol-9-live-color-vision-deficiency-cvd-and-grayscale-canvas-preview) | Live colorblind (CVD) & grayscale print preview toggle on canvas | Accessibility | `palette`, `render`, `ui-egui` | **Done** (`R9.9`) |
| [QOL-10](#qol-10-zero-install-client-side-wasm-viewer-and-browser-folder-workspaces) | Zero-install client-side WASM collaborator viewer & browser folder workspaces | Web / Collaboration | `format`, `ui-egui`, `curvebook-web` | **Deferred; existing code retained** (`R9.10`) |
| [QOL-11](#qol-11-unpacked-git-friendly-project-directory-and-in-file-checkpoints) | Unpacked Git-friendly directory mode & named checkpoints | Persistence | `format`, `engine`, `ui-egui` | **Deferred; existing code retained** (`R9.11`) |
| [QOL-12](#qol-12-categorical-and-continuous-table-analysis-completions) | Factorial ANOVA post-hoc tabs, Cohen/Fleiss $\kappa$, `NNT` CIs, custom GOF $\chi^2$, Binomial/Sign tests, $\chi^2$ SD CIs & summary *t*-tests | Biostatistics | `doc`, `stats`, `engine`, `ui-egui` | **Done** (`R9.12`) |
| [QOL-13](#qol-13-instant-statistical-distribution-and-bayesian-calculators) | Stateless statistical, distribution, and Bayesian calculators | Calculators & DSL | `stats`, `transform`, `engine`, `mcp`, `ui-egui` | **Deferred; no scheduled milestone** (`R9.13`) |
| [QOL-14](#qol-14-experimental-subject-randomization-and-random-subset-sampling) | Seeded complete/permuted-block group randomization, uniform/stratified $k$-of-$N$ row sampling & multi-distribution table generation | Experimental Design | `doc`, `transform`, `engine`, `ui-egui` | **Done** (`R9.14`) |
| [QOL-15](#qol-15-bench-molarity-dilution-and-radiochemical-decay-calculators) | Molarity, dilution, moles/grams, and radiochemical calculators | Bench Lab Tools | `stats`, `engine`, `ui-egui` | **Deferred; no scheduled milestone** (`R9.15`) |
| [QOL-16](#qol-16-live-tex-model-equation-annotations-equations-only) | Symbolic and live-substituted $\TeX$-subset best-fit equation callouts only | Graphing | `doc`, `fit`, `text`, `chart`, `engine`, `ui-egui` | **Done (equations only)** (`R9.16`, `R10.19`) |
| [QOL-17](#qol-17-extended-multivariable-clustering-glm-and-statistical-graphing-gui_only-suite) | Extended 21-workflow multivariable, clustering, GLM & graphing coverage (`WF-29`–`WF-49`) | GUI Parity & QA | `ui-egui`, `mcp`, `testkit` | **Done** (`R9.17`; superseded by `human_wf_*`) |

---

## Implemented Items (Summary)

### QOL-1 Non-destructive table archetype conversion
Implemented (`R9.1`) via `table.convertArchetype` + `ConvertArchetypeDialogState` with shape-mapping preview, lossy-change confirmation, `exclusion_notes` remapping, and atomic undo.

### QOL-2 Plate-reader (`8x12` / `16x24`) and tidy long-to-subcolumn paste mapper
Implemented (`R9.2`, extended in `R17.7`) via `table.importPlateMap` (`.cbplate` `v1` layouts, kinetic reducers, `plate_well_origins`) and `table.importTidyLong` inside `ImportReshapeMode`.

### QOL-3 Auditable cell exclusion reasons and provenance tags
Implemented (`R9.3`) with fast `excluded: bool` flags plus sparse `DataSheet::exclusion_notes: Vec<CellExclusionNote>` sidecar (`table.excludeCellsWithReason`, `table.exclusionAuditReport`, `Cmd/Ctrl+Shift+E`).

### QOL-4 Optional side-by-side split workspace
Implemented (`R9.4`, refined in `R20.3`) via `SplitWorkspaceState` (`ui.split.toggle`, `Cmd/Ctrl+\`) with explicit pane targeting, unified split `DataGrid`, and reactive DAG updates.

### QOL-5 Two-way data-to-graph point brushing
Implemented (`R9.5`) via `PlottedPointMeta` / `ChartScene::hit_test_point` and `ui.brush.point` linking plotted glyphs back to source cells with jump-to-cell and exclude actions.

### QOL-6 One-click Methods and statistical reporting generator
Implemented (`R9.6`, refined in `R20.7`) via `generate_methods_report` / `result.generateReport` (`PlainText`, `Markdown`, `Latex`; `methods` vs. `full` scope) and desktop Copy/Save controls.

### QOL-7 Inline assumption diagnostic banners (diagnostics only)
Implemented (`R9.7`, strictly diagnostics-only): `AssumptionDiagnosticBanner` (`crates/ui-egui/src/dialogs_canvas/`, `dialogs_r9.rs`, `results_view.rs`) reports factual diagnostics and opens diagnostic tabs via `ui.result.inspectDiagnostics` (`mutated: false`) without test recommendations or remediation buttons; deprecated `ui.result.applyRemediation` returns `ErrorCode::Conflict` (`diagnostics_only_policy`).

### QOL-8 Journal figure dimension and typography lock presets
Implemented (`R9.8`) via `FigureCanvasPreset`, `CanvasResizeMode`, `preflight_check_scene`, `graph/layout.applyCanvasPreset`, and `graph.preflightCheck` (`< 5 pt` label / `< 0.25 pt` stroke preflight warnings).

### QOL-9 Live color-vision-deficiency (CVD) and grayscale canvas preview
Implemented (`R9.9`) via `CanvasPreviewFilter` (`Normal`, `Deuteranopia`, `Protanopia`, `Tritanopia`, `Grayscale Print` using Machado 2009 matrices in `crates/palette`) and non-mutating `ui.previewFilter.set`.

### QOL-12 Categorical and continuous table-analysis completions
Implemented (`R9.12`; see [`calculators-and-quick-stats.md` §2](calculators-and-quick-stats.md#2-categorical--continuous-table-analysis-completions-r912--qol-12--implemented-wave-21)): factorial ANOVA post-hoc tabs, Cohen/Fleiss $\kappa$, Altman `NNTB`/`NNTH` CIs, McNemar discordant OR CIs, custom GOF $\chi^2$, exact Binomial & paired Sign tests, exact $\chi^2$ `sd_ci`, and summary `Mean + SD/SEM + N` *t*-tests.

### QOL-14 Experimental subject randomization and random subset sampling
Implemented (`R9.14`; see [`calculators-and-quick-stats.md` §4](calculators-and-quick-stats.md#4-experimental-randomization--subset-sampling-r914--qol-14--implemented-wave-22)) in `crates/transform/src/randomize.rs`: seeded Complete/Permuted-Block randomization (`table.randomizeGroups`), uniform/stratified $k$-of-$N$ row sampling (`table.randomSampleRows`), and distribution simulation (`table.simulateDistributions`).

### QOL-16 Live $\TeX$ model equation annotations (equations only)
Implemented (`R9.16`, upgraded to 2D Knuth $\TeX$ math boxes in [`R10.19`](../history/roadmap/r10.md#r1019-typeset-mathematical-equation-annotations); see [`calculators-and-quick-stats.md` §6](calculators-and-quick-stats.md#6-live-best-fit-equation-annotations-r916--qol-16-equations-only--implemented-wave-22b)) as `GraphAnnotation::ModelEquation` across RGBA, SVG, and `%PDF-1.7`. Compact Letter Display shipped separately under [`R10.11`](../history/roadmap/r10.md#r1011-compact-letter-display-cld).

### QOL-17 Extended multivariable, clustering, GLM and statistical-graphing `gui_only` suite
Implemented (`R9.17`; see [`gui-workflows.md` §5](../gui-workflows.md#5-extended-21-workflow-multivariable-clustering-glm--statistical-graphing-matrix-r917--qol-17-superseded-by-human_wf_)) covering `WF-29`–`WF-49`; the legacy `--gui-only` shards were retired under [`R21.1`](../history/roadmap/r21.md#r211-retire---gui-only-and-synthetic-ui-automation-plumbing) after all `65 / 65` workflows moved to five-stage `human_wf_*` desktop tests (`R10.25`).

---

## Deferred Specifications (Retained for Reference)

### QOL-10 Zero-install client-side WASM viewer and browser folder workspaces
- **Planning status:** Deferred without a scheduled milestone (`R9.10`). Existing `VirtualWorkspaceFolder` code remains supported; baseline browser app work (`eframe::WebRunner`) is tracked under [`R11`](../ROADMAP.md#r11-usable-browser-app).
- **Crates:** `crates/format`, `crates/ui-egui`, `apps/curvebook-web`
- **Problem:** Collaborators, students, and peer reviewers often inspect project archives on machines without native software installed, while institutional policies forbid uploading unpublished data to cloud servers.
- **Scope:**
  - **100% local WASM open/inspect/edit:** Open `.curvebook`, CSV/TSV, `.prism`, or `.pzfx` locally in the browser WebAssembly sandbox with zero network upload of user data.
  - **Browser Folder Attachment:** Attach a local experiment directory (read-only drag-and-drop / directory input across Safari, Firefox, and Chromium; File System Access API read/write on Chromium with `.zip` / `.curvebook` download fallbacks).
  - **Optional Read-Only Presentation Mode:** Viewer toggle hiding editing chrome for lab meetings and supplementary-material inspection.
- **Non-breaking invariant:** Preserves `wasm32-unknown-unknown` compatibility (`cargo xtask wasm`) and zero-egress rules in [`AGENTS.md`](../../AGENTS.md).

### QOL-11 Unpacked Git-friendly project directory and in-file checkpoints
- **Planning status:** Deferred without a scheduled milestone (`R9.11`). Existing unpacked-directory I/O (`is_unpacked_project_dir`, `save_project_unpacked_dir`, `open_project_unpacked_dir` in `crates/format/src/archive.rs`) and `NamedCheckpoint` commands remain supported.
- **Crates:** `crates/format`, `crates/engine`, `crates/ui-egui`
- **Problem:** Duplicating binary project files before trying alternative analyses creates file sprawl and opaque Git/DVC diffs.
- **Scope:**
  - **Unpacked Directory Format (`.curvebook.d/`):** Mirror `.curvebook` archive entries (`manifest.json`, 2-space-indented JSON specs, RFC-4180 CSV data sheets) on disk for line-by-line Git diffs.
  - **Named Checkpoints:** Record and restore named snapshots inside a `.curvebook` session (existing snapshots store full project JSON).

### QOL-13 Instant statistical, distribution and Bayesian calculators
- **Planning status:** Deferred without a scheduled milestone (`R9.13`). Reference only; none of these `calc.*` IDs exist today.
- **Crates:** `crates/stats`, `crates/transform`, `crates/engine`, `crates/mcp`, `crates/ui-egui` · **Spec:** [`calculators-and-quick-stats.md` §3](calculators-and-quick-stats.md#3-instant-statistical-distribution--bayesian-calculators-r913--qol-13)
- **Scope:** Stateless read-only `calc.*` commands (`CommandGroup::Calculator`, `Effects::NONE`) and a non-modal **Calculators** drawer (`ui.calculator.open`) for binomial/Poisson CIs, clinical Bayes PPV/NPV/$\text{LR}^{\pm}$, two-mean error propagation & Fieller's quotient CI, summary ANOVA post-tests, forward/inverse probability distributions (also exposed in `=` Calculated Variables), Sellke–Bayarri–Berger & Colquhoun Bayesian $P$-value / FPR interpretation, summary $F$/$\text{AIC}_c$ fit comparisons, and scalar $\text{EC}_F$.
- **Non-breaking invariant:** Zero document/undo mutation and zero archive-schema changes.

### QOL-15 Bench molarity, dilution and radiochemical decay calculators
- **Planning status:** Deferred without a scheduled milestone (`R9.15`). Reference only; none of these `calc.*` IDs exist today.
- **Crates:** `crates/stats`, `crates/engine`, `crates/ui-egui` · **Spec:** [`calculators-and-quick-stats.md` §5](calculators-and-quick-stats.md#5-bench-molarity-dilution--radiochemical-calculators-r915--qol-15)
- **Scope:** Stateless wet-lab calculators (`calc.molarity`, `calc.dilution`, `calc.molesGrams`, `calc.radioactivity`) solving $m = C \cdot V \cdot \text{FW}$, single & serial stock dilutions ($C_1 V_1 = C_2 V_2$), moles $\leftrightarrow$ grams, and NUBASE2020 radionuclide decay/specific activity with optional export to `InfoSheet` constants or `XY` dilution columns.
- **Non-breaking invariant:** Pure read-only `calc.*` queries unless the user explicitly exports constants or creates a sheet.
