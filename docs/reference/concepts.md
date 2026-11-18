# Concepts & Document Architecture

CurveBook stores an entire statistical and graphing study inside a single deterministic project (`ProjectDocument`) organized into five typed workspace sections, connected by a reactive directed acyclic graph (DAG), and protected by non-destructive cell exclusions, bounded session undo/redo, and explicit schema versioning.

---

## Projects, Identifiers & Document Limits {#projects-identifiers-and-limits}

A `ProjectDocument` pairs a project title, persistent analysis/workspace metadata, optional checkpoints, and an ordered collection of typed sheets. All structural sizes obey explicit upper bounds enforced during editing and archive validation:

| Limit Constant | Value | Scope |
|---|---:|---|
| `CURRENT_SCHEMA_VERSION` | `3` | Active project document JSON schema version |
| `MAX_SHEETS_PER_PROJECT` | `512` | Maximum sheets across all five sections in one project |
| `MAX_ROWS` | `1,000,000` | Maximum rows per individual data table |
| `MAX_DATA_SETS` | `1,024` | Maximum dataset/group/variable columns per data table |
| `MAX_SUBCOLUMNS` | `256` | Maximum replicate or summary subcolumns per dataset |
| `MAX_SHEET_CELLS` | `10,000,000` | Maximum total dense cells (`rows * cols * subcols`) in one table |
| `MAX_PROJECT_CELLS` | `50,000,000` | Maximum total cells across all data sheets in one project |
| `MAX_TITLE_BYTES` | `4,096` | Maximum UTF-8 byte length for project or sheet titles |
| `MAX_SHEET_ID_BYTES` | `128` | Maximum UTF-8 byte length for a `SheetId` |
| `MAX_CHECKPOINTS_PER_PROJECT` | `64` | Maximum in-file `NamedCheckpoint` snapshots per project |

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
- **Freezing (`sheet.freeze` / `sheet.unfreeze`):** A frozen `DataSheet` (`frozen: true`) rejects direct mutations with `ErrorCode::FrozenSheet`; a frozen `ResultSheet` retains its snapshot with `ResultStatus::Frozen` instead of overwriting when upstream data change.
- **`ResultStatus` lifecycle:** Every `ResultSheet` reports one of four statuses:
  - `UpToDate`: Computed from current upstream inputs and parameters without error.
  - `Frozen`: Computation frozen on user snapshot; upstream edits do not update values until unfrozen.
  - `Stale`: Upstream dependencies changed and automatic/manual recalculation has not yet refreshed the sheet.
  - `Failed`: Computation could not succeed for current inputs (e.g., all rows excluded or singular model); structured `ErrorCode` diagnostics explain the failure. When upstream valid data are restored, `up_to_date` results resume automatically.

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

For persistent milestone snapshots across sessions, `ProjectDocument` supports named in-file checkpoints (`project.createCheckpoint`, `project.listCheckpoints`, `project.restoreCheckpoint`, `project.deleteCheckpoint`, up to `64` per project). Each `NamedCheckpoint` stores an `id`, human-readable `name`, optional `note`, UTC timestamp, and a complete validated JSON snapshot of the project at creation time (subject to `MAX_ARCHIVE_FILE_BYTES = 128 MiB` on restore). Restoring a checkpoint creates a single undoable action in the live session.

---

## Number Formatting & *P*-Value Conventions {#number-and-pvalue-formatting}

- **Data & Tick Formatting (`LocaleNumberPolicy`, `AxisTickFormat`):** Graphs and tables separate stored IEEE-754 `f64` precision from display formatting. `LocaleNumberPolicy` controls decimal separators (`.` vs `,`) and thousands grouping, while axis tick labels support `Auto`, `FixedDecimals`, `SignificantFigures`, `Scientific`, `Engineering`, `SiPrefix` (`y`..`Y`), and `Percentage` (`0..=1` scaled by `100%`) without rounding underlying computed data.
- ***P*-Value Display (`PValueFormat`):** Stored per `AnalysisSpec` (`curvebook-math`):
  - Exact numeric *P* formats (`Decimal(digits)`, `AutoSignificance`, `Scientific`, `ThresholdsOnly`) paired with standard threshold markers (`ns`, `*`, `**`, `***`, `****` at `< 0.05`, `< 0.01`, `< 0.001`, `< 0.0001`).
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
