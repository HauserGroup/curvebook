# Concepts and document structure

<!-- Audience: Tier 2 Technical Reference — exhaustive project schemas, limits, and reactive lifecycle rules belong here; link to Tier 1 guides (desktop-gui.md or get-started.md) for scientist walkthroughs. -->

This technical reference describes how a CurveBook project is structured and how large it can grow. It also covers the dependency graph that keeps results up to date, non-destructive exclusions, the limits on undo history and checkpoints, and the analysis settings saved with each result. For a quick walkthrough of the workspace, start with the [Desktop GUI guide](desktop-gui.md) or [Get started & installation](get-started.md).

A CurveBook project holds an entire statistical and graphing study, and the same inputs always produce the same results. The project's five workspace sections are linked through a directed acyclic graph (DAG) of dependencies, which tells CurveBook what to recalculate after a change. Non-destructive exclusions, limited undo/redo, and explicit schema versions protect your study at every step.

---

## Projects, identifiers and document limits {#projects-identifiers-and-limits}

A project contains a title, workspace metadata, optional checkpoints, and an ordered set of typed sheets. Every sheet has a unique sheet ID, which must not be blank or whitespace, and a display title that you can edit.

CurveBook enforces these structural limits both while you edit and when it validates an archive:

| Limit | Value | Scope |
|---|---:|---|
| Document schema version | 9 | Current version of the project JSON schema |
| Sheets per project | 500 | Total sheets across all five navigator sections |
| Rows per table | 1,000,000 | Maximum rows in one data table |
| Datasets per table | 1,024 | Maximum dataset, group, or variable columns |
| Replicate subcolumns | 512 | Maximum replicate subcolumns per dataset |
| Cells per table | 30,000,000 | Total dense cells (rows × columns × subcolumns) |
| Cells per project | 50,000,000 | Total cells across all data tables |
| Title length | 4,096 bytes | Maximum UTF-8 length of a project or sheet title |
| Sheet ID length | 256 bytes | Maximum UTF-8 length of a sheet ID |
| Named checkpoints | 32 (64 on open) | Limit for creating checkpoints vs. the ceiling when validating an archive (see [Named in-file checkpoints](#named-checkpoints)) |

---

## Sheet sections and sheet families {#sheet-sections-and-families}

The navigator groups sheets into five sections:

| Section | Role in the workspace |
|---|---|
| **Data** | Typed grids for the [eight table archetypes](tables.md#table-archetypes), with exclusions and calculated formulas. |
| **Info** | Free-form study notes and typed key/value constants for the experiment. |
| **Analyses** | Statistical, transform, simulation, or curve-fitting results, each with one or more tabs. |
| **Graphs** | Publication figures that draw on data tables and add analysis overlays. |
| **Layouts** | Physical pages that combine several panels: graphs, live tables, images, and badges. |

How each kind of sheet behaves:

- **Data tables** store row titles, non-destructive exclusion masks, and notes recording why cells were excluded. They also hold `=` Calculated Variables and linked data regions.
- **Info sheets** hold text, numeric, or boolean constants. Formulas, analyses, and graph text can link to these constants (hook constants). The standard metadata keys are `ExperimentDate`, `Experimenter`, `Protocol`, `ProjectCode`, `NotebookId`, `LotNumber`, `Instrument`, and `Notes`.
- **Results sheets** are computed from a data table or from another analyzable results sheet. Each results sheet includes assumption diagnostics, scalar outputs that other sheets can link to (hook scalars), curve overlays, and optional floating notes.
- **Graphs** draw on one or more data tables and can add overlays: fitted curves, comparison brackets, compact letter displays, and live $\TeX$ equations.
- **Page layouts** combine up to 64 panels on one page. Layouts support physical paper sizes and presets for journal column widths.

### Sheet families {#sheet-families}

Each sheet records the data table it ultimately comes from in `family_anchor_sheet_id`. Results sheets, derived transform tables, graphs, and page layouts downstream of that table stay linked to it as one family.

You can inspect a whole family together in the navigator. To copy an entire pipeline with all its links intact, use `sheet.duplicateFamily` or `sheet.clonePipeline` (see the [Command Reference](../commands.md)).

---

## Dependency graph and recalculation {#reactive-dag}

The CurveBook engine tracks dependencies between sheets as a strict directed acyclic graph (DAG):

```text
Info sheet (hook constants) ──┐
Data table (raw observations) ├──► Formulas (= Calculated Variables)
                              ├──► Linked data regions (paste / transform links)
                              ├──► Results sheet (transform / stats / fit) ──► Hook scalars
                              └──► Graph (series + fit overlays + CLD/TeX) ──► Page layout
```

- **Cycle detection:** CurveBook rejects circular formulas, loops between linked regions, and circular links to constants or scalars with `CycleDetected`, before the edit is applied.
- **Marking dependents for recalculation:** Editing a cell, toggling an exclusion, changing an info constant, or changing analysis parameters marks every downstream dependent as dirty. Recalculation (`project.recalculate`) then updates them in a fixed topological order, so the result never depends on timing.
- **Freezing sheets:** `sheet.freeze` and `sheet.unfreeze` set or clear the `frozen` flag on any data, results, graph, or layout sheet. A frozen data table rejects direct edits with `FrozenSheet`. A frozen results sheet keeps its current snapshot when upstream data change.
- **Result status:** Every results sheet reports one of these states:
  - **Dirty:** Upstream inputs or analysis parameters have changed, and recalculation is pending.
  - **Up to date:** Tabs, scalars, pairwise comparisons, fitted curves, and diagnostics all match the current inputs.
  - **Imported:** A read-only snapshot imported from an external file (such as `.prism files`), whose upstream table has not been edited since import. Imported snapshots never feed linked scalars or downstream analyses.
  - **Imported stale:** A read-only imported snapshot whose upstream table was edited or removed after import. The cached tabs remain available for inspection as a stale reference.
  - **Failed:** Recalculation could not succeed with the current inputs (for example, because all rows are excluded or the model is singular). While a sheet has failed, its linked scalars return no value, so invalid results never spread downstream. When valid upstream inputs are restored, the sheet returns to up to date automatically.

---

## Non-destructive cell exclusions and audit notes {#cell-exclusions}

Every numeric or text cell stores its `excluded` flag separately from its value. Marking a cell as excluded has four effects:

1. **The raw value is kept.** The value stays intact in memory, in `.curvebook` archives, and in `.curvebook.d/` CSV files. When `parse_asterisk_exclusions` is active, CSV exports write excluded numbers with a leading `*` (for example, `*12.4`).
2. **Calculations skip the value.** Descriptive summaries, statistical tests, curve fits, transforms, plotted points, error bars, and selection [Quick Statistics](qol.md#quick-statistics) all leave out the observation.
3. **Formulas treat it as missing.** Excluded cells propagate as missing values in [`=` Calculated Variables](tables.md#calculated-variables).
4. **An audit trail is optional.** With `table.excludeCellsWithReason` and `table.exclusionAuditReport`, you can record structured audit notes holding the cell coordinate, `reason_tag`, `note`, and `timestamp_utc`. Audit notes never affect numerical calculations.

---

## Session undo/redo and named checkpoints {#undo-and-checkpoints}

### Limited session undo/redo {#undo-redo}

Every engine command that changes the document runs as one atomic step that you can undo (`edit.undo`) and redo (`edit.redo`). The undo stack is subject to three memory limits at once:

| Limit | Value | Purpose |
|---|---:|---|
| Undo steps | 64 | Maximum steps kept in memory per open session |
| Retained cells | 25,000,000 | Total table cells across all retained snapshots |
| Snapshot memory | 16 MiB | Memory budget for full serialized snapshots |

Undo/redo history exists only in the current session and is not saved in `.curvebook` archives. Protection against crashes between saves is handled separately by the [WAL crash recovery journal](files.md#crash-recovery).

### Named in-file checkpoints {#named-checkpoints}

Use named checkpoints for milestone snapshots that persist across sessions. You manage them with `project.createCheckpoint`, `project.listCheckpoints`, `project.restoreCheckpoint`, and `project.deleteCheckpoint`. You can create up to 32 checkpoints per project, and CurveBook accepts up to 64 when it opens an existing archive.

Each checkpoint stores an `id`, a display `name`, an optional `note`, a UTC timestamp, and a validated JSON snapshot of the project. Restoring a checkpoint enforces the 128 MiB archive limit and adds one undoable step to the current session.

---

## Number formatting and *P*-value conventions {#number-and-pvalue-formatting}

- **Data and tick formatting:** Tables and graphs keep the stored IEEE-754 double-precision values separate from how they are displayed. Locale settings control the decimal separator (`.` or `,`) and thousands grouping. Numeric axis ticks can use decimal, power-of-ten superscript, scientific E, antilog, engineering SI, or categorical labels, and none of these round the stored values.
- ***P*-value display:** You choose one of four formatting modes for each analysis:
  - **Four decimals** (default): the smallest displayed value is `< 0.0001`, with significance asterisks up to `****`.
  - **APA:** `.123` style; the smallest displayed value is `< .001`, with asterisks up to `***`.
  - **NEJM:** `0.123` style; the smallest displayed value is `< 0.001`, with asterisks up to `***`.
  - **Custom:** `1` to `15` decimal places, with an optional cutoff for switching to scientific notation.
- **Full-precision math:** Display formatting never rounds the numbers used in calculations. Multiplicity procedures and hypothesis decisions always use the full IEEE-754 probabilities.

---

## Saved analysis settings {#persisted-analysis-settings}

Every results sheet stores the complete analysis specification that produced it:

- The sheet `id`, `title`, and `source_sheet_id`.
- The analysis kind and its method-specific options (test selection, tails, pairing, degrees of freedom, multiplicity, weighting, fit method, parameter rules, or power targets).
- `selected_data_sets` (an empty list selects all compatible columns).
- `confidence_level` (a continuous percentage in `(0, 100)`, 95% by default).
- `p_value_format`.

These settings are saved inside `.curvebook` archives, `.curvebook.d/` directories, and `.cbtemplate` templates. Re-running an analysis (`analysis.run`), updating its options (`analysis.updateParams`), or pointing it at new data (`analysis.retargetData`) reproduces every choice exactly. See the [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), and [Automation: CLI](automation/cli.md) for the parameter schemas used in scripts and automation.
