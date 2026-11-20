# Concepts & Document Architecture

CurveBook stores an entire statistical and graphing study inside a single deterministic project (`ProjectDocument`) organized into five typed workspace sections, connected by a reactive directed acyclic graph (DAG), and protected by non-destructive cell exclusions, bounded session undo/redo, and explicit schema versioning.

---

## Projects, Identifiers & Document Limits {#projects-identifiers-and-limits}

A `ProjectDocument` pairs a project title, persistent analysis/workspace metadata, optional checkpoints, and an ordered collection of typed sheets. All structural sizes obey explicit upper bounds enforced during editing and archive validation:

| Limit Constant | Value | Scope |
|---|---:|---|
| `CURRENT_SCHEMA_VERSION` | `7` | Active project document JSON schema version |
| `MAX_SHEETS_PER_PROJECT` | `500` | Maximum sheets across all five navigator sections in one project |
| `MAX_ROWS` | `1,000,000` | Maximum rows per individual data table |
| `MAX_DATA_SETS` | `1,024` | Maximum dataset/group/variable columns per data table |
| `MAX_SUBCOLUMNS` | `512` | Maximum replicate subcolumns per dataset (`RawReplicates(n)`) |
| `MAX_SHEET_CELLS` | `30,000,000` | Maximum total dense cells (`rows * cols * subcols`) in one table |
| `MAX_PROJECT_CELLS` | `50,000,000` | Maximum total cells across all data sheets in one project |
| `MAX_TITLE_BYTES` | `4,096` | Maximum UTF-8 byte length for project or sheet titles |
| `MAX_SHEET_ID_BYTES` | `256` | Maximum UTF-8 byte length for a `SheetId` |
| `MAX_CHECKPOINTS_PER_PROJECT` | `32` (creation) / `64` (archive ceiling) | `project.createCheckpoint` allows up to `32` per project; archive/loader validation accepts up to `64` |

Every sheet carries a stable, non-whitespace `SheetId` (distinct across the project) and a mutable display `title`.

---

## Sheet Sections & Sheet Families {#sheet-sections-and-families}

CurveBook organizes sheets into five canonical `SheetSection` navigator folders:

| Section | Sheet Type | Role in the Workspace |
|---|---|---|
| **Data** | `DataSheet` | Typed data grid holding one of the [eight table archetypes](tables.md#table-archetypes), row titles, excluded-cell masks, exclusion provenance notes, calculated-variable formulas, and linked data regions. |
| **Info** | `InfoSheet` | Free-form notes plus typed key/value experiment constants (`Text`, `Number`, `Boolean`), including standard metadata keys (`ExperimentDate`, `Experimenter`, `Protocol`, `ProjectCode`, `NotebookId`, `LotNumber`, `Instrument`, `Notes`). Can be linked upstream via `HookSource` parameters. |
| **Analyses** | `ResultSheet` | Multi-tab statistical, transform, simulation, or curve-fitting output derived from an upstream `DataSheet` (or analyzable upstream `ResultSheet`), accompanied by diagnostic checklists (`QOL-7`), hookable scalars, curve overlays, and optional `floating_notes`. |
| **Graphs** | `GraphSheet` | Publication graph specification (`GraphSpec`) referencing one or more upstream `DataSheet`s and optional `ResultSheet` overlays (fitted curves, comparison brackets, compact letter display, live $\TeX$ equation boxes). |
| **Layouts** | `LayoutSheet` | Multi-panel physical page composition (`LayoutSpec`, up to `64` `LayoutItem` panels) arranging `GraphSheet`s and supporting text/badges with physical paper sizes and journal column-width presets. |

### Sheet Families (`family_anchor_sheet_id`) {#sheet-families}

Each sheet tracks its lineage to a root `DataSheet` via `family_anchor_sheet_id`, allowing downstream `ResultSheet`s, derived `DataSheet`s (from transforms), `GraphSheet`s, and `LayoutSheet`s to be inspected together or cloned coherently via `sheet.duplicateFamily` and `sheet.clonePipeline` (see [Command Reference](../commands.md)).

---

## Reactive DAG & Recalculation Lifecycle {#reactive-dag}

Dependencies between sheets form a strict **directed acyclic graph (DAG)** evaluated by `curvebook-engine` (`crates/engine/src/dag.rs`):

```text
InfoSheet (HookSource constants) ──┐
DataSheet (raw observations)       ├──► Formula (= Calculated Variables)
                                   ├──► LinkedDataRegion (paste / transform links)
                                   ├──► ResultSheet (Transform / Stats / Fit) ──► HookSource scalars
                                   └──► GraphSheet (series + fit overlays + CLD/TeX) ──► LayoutSheet
```

- **Cycle detection:** Formula circularity, linked-region loops (`detect_link_cycles`), and cyclic `HookSource` parameter bindings are rejected with `ErrorCode::CycleDetected` before committing edits or running recalculation.
- **Dirty propagation & topological order:** Editing a cell, toggling an exclusion, updating an `InfoSheet` constant, or modifying an `AnalysisSpec` marks downstream dependents dirty and recalculates them in deterministic topological order (`project.recalculate`).
- **Freezing (`sheet.freeze` / `sheet.unfreeze`):** Sets `frozen: bool` on a sheet (`DataSheet`, `ResultSheet`, `GraphSheet`, `LayoutSheet`). A frozen `DataSheet` (`frozen: true`) rejects direct mutations with `ErrorCode::FrozenSheet`; a frozen `ResultSheet` retains its snapshot instead of recomputing when upstream data change.
- **`ResultStatus` lifecycle:** Every `ResultSheet` reports one of three statuses:
  - `Dirty`: Upstream inputs or `AnalysisSpec` parameters changed and recalculation is pending.
  - `UpToDate`: Computed tabs, scalars, pairwise comparisons, fitted curves, and checklist reflect current upstream inputs without error.
  - `Failed { code, message }`: Most recent recalculation could not succeed for current inputs (e.g., all rows excluded or singular model); `lookup_scalar` returns `None` while failed so invalid results never propagate through hooks. When valid upstream data are restored, recalculation resumes `UpToDate` results automatically.

---

## Non-Destructive Cell Exclusions & Provenance {#cell-exclusions}

Every numeric or text cell stores `excluded: bool` independently of `value: CellValue`. Marking a cell excluded:

1. Preserves the underlying raw numeric or text value in memory, `.curvebook` archives, and `.curvebook.d/` sheet CSVs (written with a leading `*` prefix such as `*12.4` when `parse_asterisk_exclusions` is active).
2. Excludes the observation from descriptive summaries, statistical tests, curve fits, transforms, and plotted points/error bars, and omits it from selection [Quick Statistics](qol.md#quick-statistics).
3. Propagates as missing in [`=` Calculated Variables](tables.md#calculated-variables).
4. Supports optional structured provenance tags via `DataSheet::exclusion_notes: Vec<CellExclusionNote>` (`QOL-3`), storing `(coord, reason_tag, note, timestamp_utc)` (`table.excludeCellsWithReason` and `table.exclusionAuditReport`) for reproducible audit trails without touching hot numerical loops.

---

## Session Undo/Redo & Named Checkpoints {#undo-and-checkpoints}

### Bounded Session Undo/Redo {#undo-redo}

Every mutating engine command executes inside `Session` (`crates/engine/src/lib.rs`) as an atomic undoable step (`edit.undo`, `edit.redo`), subject to three simultaneous memory-safety caps:

| Undo Bound | Value | Purpose |
|---|---:|---|
| `MAX_UNDO_ENTRIES` | `64` | Maximum undo steps retained in memory per open session |
| `MAX_UNDO_CELL_BUDGET` | `25,000,000` | Cumulative table cell count cap across retained undo snapshots |
| `MAX_UNDO_CHECKPOINT_BYTES_BUDGET` | `16,777,216` (`16 MiB`) | Byte budget for full serialized snapshots in the undo stack |

Undo/redo history lives only in the active session and is intentionally not written inside `.curvebook` archives. Crash resilience between saves is handled separately by the [WAL `CrashRecoveryJournal`](files.md#crash-recovery).

### Named In-File Checkpoints (`NamedCheckpoint`) {#named-checkpoints}

For persistent milestone snapshots across sessions, `ProjectDocument` supports named in-file checkpoints (`project.createCheckpoint`, `project.listCheckpoints`, `project.restoreCheckpoint`, `project.deleteCheckpoint`, up to `32` created per project via `project.createCheckpoint`, with `crates/format` permitting up to `64` on archive validation). Each `NamedCheckpoint` stores an `id`, human-readable `name`, optional `note`, UTC timestamp, and a complete validated JSON snapshot of the project at creation time (subject to `MAX_ARCHIVE_FILE_BYTES = 128 MiB` on restore). Restoring a checkpoint creates a single undoable action in the live session.

---

## Number Formatting & *P*-Value Conventions {#number-and-pvalue-formatting}

- **Data & Tick Formatting (`LocaleNumberPolicy`, `AxisTickFormat`, `TickLabelFormat`):** Graphs and tables separate stored IEEE-754 `f64` precision from display formatting. `LocaleNumberPolicy` controls decimal separators (`.` vs `,`) and thousands grouping, while numeric axis ticks (`AxisTickFormat`) support `Decimal`, `PowerOfTenSuperscript`, `ScientificE`, `Antilog`, and `EngineeringSi` (with `TickLabelFormat` adding `Categorical(CategoricalTickSource)`) without rounding underlying computed data.
- ***P*-Value Display (`PValueFormat`):** Stored per `AnalysisSpec` (`curvebook-math`):
  - Exact numeric formats: `FourDecimal` (default, floor `< 0.0001`, up to `****`), `Apa` (`.123`, floor `< .001`, up to `***`), `Nejm` (`0.123`, floor `< 0.001`, up to `***`), and `Custom { decimals, scientific_below }` (`decimals` in `1..=15` with optional positive scientific-notation cutoff).
  - Formatting affects presentation only; multiplicity procedures and hypothesis decisions operate on full unrounded IEEE-754 probabilities.

---

## Persisted Analysis Settings (`AnalysisSpec`) {#persisted-analysis-settings}

Every `ResultSheet` embeds the complete `AnalysisSpec` that generated it, recording:

- `id`, `title`, and `source_sheet_id`
- `kind: AnalysisKind` and its method-specific option struct (test selection, tails, pairing, sphericity/REML df method, multiplicity procedure, weighting scheme, fit method, parameter rules, or power targets)
- `selected_data_sets: Vec<usize>` (empty selects all compatible columns/datasets)
- `confidence_level: ConfidenceLevel` (continuous percentage `(0, 100)`, defaulting to `95%`)
- `p_value_format: PValueFormat`

Because `AnalysisSpec` travels inside both `.curvebook` / `.curvebook.d/` projects and `.cbtemplate` reusable templates, re-running (`analysis.run`), modifying parameters (`analysis.updateParams`), or retargeting to new experimental data (`analysis.retargetData`) preserves every statistical choice deterministically. See [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), and [Automation: CLI](automation/cli.md) for programmatic parameter schemas.
