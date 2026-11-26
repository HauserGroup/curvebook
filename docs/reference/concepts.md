# Concepts & document architecture

CurveBook stores an entire statistical and graphing study inside one deterministic project. Projects are organized into five workspace sections, connected by a reactive directed acyclic graph (DAG), and protected by non-destructive exclusions, bounded undo/redo, and explicit schema versioning.

---

## Projects, identifiers & document limits {#projects-identifiers-and-limits}

A project pairs a title, workspace metadata, optional checkpoints, and an ordered set of typed sheets. Every sheet carries a unique, non-whitespace sheet ID and an editable display title.

Editing and archive validation enforce these structural limits:

| Limit | Value | Scope |
|---|---:|---|
| Document schema version | 8 | Active project JSON schema version |
| Sheets per project | 500 | Total sheets across all five navigator sections |
| Rows per table | 1,000,000 | Maximum rows in one data table |
| Datasets per table | 1,024 | Maximum dataset, group, or variable columns |
| Replicate subcolumns | 512 | Maximum replicate subcolumns per dataset |
| Cells per table | 30,000,000 | Total dense cells (rows × columns × subcolumns) |
| Cells per project | 50,000,000 | Total cells across all data tables |
| Title length | 4,096 bytes | Maximum UTF-8 byte length for project or sheet titles |
| Sheet ID length | 256 bytes | Maximum UTF-8 byte length for a sheet ID |
| Named checkpoints | 32 (64 on open) | Creation limit vs. archive validation ceiling |

`project.createCheckpoint` allows creating up to 32 checkpoints per project. Opening an existing archive accepts up to 64 checkpoints.

---

## Sheet sections & sheet families {#sheet-sections-and-families}

CurveBook organizes sheets into five navigator sections:

| Section | Role in the workspace |
|---|---|
| **Data** | Typed grids for the [eight table archetypes](tables.md#table-archetypes), exclusions, and calculated formulas. |
| **Info** | Free-form study notes and typed key/value experiment constants. |
| **Analyses** | Multi-tab statistical, transform, simulation, or curve-fitting results. |
| **Graphs** | Publication figures referencing data tables and analysis overlays. |
| **Layouts** | Multi-panel physical pages combining graphs, live tables, images, and badges. |

Key workspace behaviors across sections:

- **Data tables:** Store row titles, non-destructive exclusion masks, exclusion provenance notes, `=` Calculated Variables, and linked data regions.
- **Info sheets:** Hold text, numeric, or boolean constants—including standard metadata keys (`ExperimentDate`, `Experimenter`, `Protocol`, `ProjectCode`, `NotebookId`, `LotNumber`, `Instrument`, `Notes`)—that can be hooked into formulas, analyses, and graph text.
- **Results sheets:** Derive from a data table or another analyzable results sheet. Each results sheet includes assumption diagnostics, hookable scalar outputs, curve overlays, and optional floating notes.
- **Graphs and page layouts:** Graphs reference one or more data tables plus optional overlays (fitted curves, comparison brackets, compact letter display, and live $\TeX$ equations). Page layouts hold up to 64 panels with physical paper sizes and journal column-width presets.

### Sheet families {#sheet-families}

Each sheet tracks its root data table via `family_anchor_sheet_id`. Downstream results sheets, derived transform tables, graphs, and page layouts stay linked to that anchor. You can inspect a family together or clone an entire pipeline coherently with `sheet.duplicateFamily` and `sheet.clonePipeline` (see the [Command Reference](../commands.md)).

---

## Reactive DAG & recalculation lifecycle {#reactive-dag}

Dependencies between sheets form a strict **directed acyclic graph (DAG)** evaluated by the CurveBook engine:

```text
Info sheet (hook constants) ──┐
Data table (raw observations) ├──► Formulas (= Calculated Variables)
                              ├──► Linked data regions (paste / transform links)
                              ├──► Results sheet (transform / stats / fit) ──► Hook scalars
                              └──► Graph (series + fit overlays + CLD/TeX) ──► Page layout
```

- **Cycle detection:** Circular formulas, linked-region loops, and cyclic hook parameter bindings are rejected (`CycleDetected`) before any edit or recalculation commits.
- **Dirty propagation:** Editing a cell, toggling an exclusion, updating an info constant, or changing analysis parameters marks downstream dependents dirty. Recalculation then runs in deterministic topological order (`project.recalculate`).
- **Freezing sheets:** `sheet.freeze` and `sheet.unfreeze` toggle the `frozen` flag on any data, results, graph, or layout sheet. A frozen data table rejects direct mutations (`FrozenSheet`). A frozen results sheet keeps its existing snapshot when upstream data change.
- **Result status lifecycle:** Every results sheet reports one of three states:
  - **Dirty:** Upstream inputs or analysis parameters changed and recalculation is pending.
  - **Up to date:** Tabs, scalars, pairwise comparisons, fitted curves, and diagnostics match current inputs.
  - **Failed:** Recalculation could not succeed for current inputs (for example, all rows excluded or a singular model). Scalar hooks return no value while failed so invalid results never propagate; restoring valid upstream inputs returns results to up to date automatically.

---

## Non-destructive cell exclusions & provenance {#cell-exclusions}

Every numeric or text cell stores its `excluded` flag independently of its value. Marking a cell excluded has four effects:

1. Preserves the raw numeric or text value in memory, `.curvebook` archives, and `.curvebook.d/` CSV files (written with a leading `*`, such as `*12.4`, when `parse_asterisk_exclusions` is active).
2. Excludes the observation from descriptive summaries, statistical tests, curve fits, transforms, plotted points and error bars, and selection [Quick Statistics](qol.md#quick-statistics).
3. Propagates as missing in [`=` Calculated Variables](tables.md#calculated-variables).
4. Supports optional structured audit notes storing cell coordinate, `reason_tag`, `note`, and `timestamp_utc` (`table.excludeCellsWithReason` and `table.exclusionAuditReport`) without affecting numerical computations.

---

## Session undo/redo & named checkpoints {#undo-and-checkpoints}

### Bounded session undo/redo {#undo-redo}

Every mutating engine command executes as one atomic undoable step (`edit.undo`, `edit.redo`). The undo stack obeys three simultaneous memory limits:

| Limit | Value | Purpose |
|---|---:|---|
| Undo steps | 64 | Maximum steps retained in memory per open session |
| Retained cells | 25,000,000 | Cumulative table cell cap across retained snapshots |
| Snapshot memory | 16 MiB | Byte budget for full serialized snapshots in memory |

Undo/redo history lives only in the active session and is not written inside `.curvebook` archives. Crash resilience between saves is handled separately by the [WAL crash recovery journal](files.md#crash-recovery).

### Named in-file checkpoints {#named-checkpoints}

Use named checkpoints for milestone snapshots that persist across sessions (`project.createCheckpoint`, `project.listCheckpoints`, `project.restoreCheckpoint`, `project.deleteCheckpoint`). You can create up to 32 checkpoints per project, and opening existing archives accepts up to 64.

Each checkpoint stores an `id`, display `name`, optional `note`, UTC timestamp, and a validated JSON snapshot of the project (subject to the 128 MiB archive limit on restore). Restoring a checkpoint creates a single undoable action in the active session.

---

## Number formatting & *P*-value conventions {#number-and-pvalue-formatting}

- **Data and tick formatting:** Tables and graphs separate stored IEEE-754 double-precision values from display formatting. Locale settings control decimal separators (`.` or `,`) and thousands grouping. Numeric axis ticks support decimal, power-of-ten superscript, scientific E, antilog, engineering SI, and categorical labels without rounding underlying data.
- ***P*-value display:** Configured per analysis with four format modes:
  - **Four decimals** (default): floor `< 0.0001`, up to `****`.
  - **APA:** `.123` style, floor `< .001`, up to `***`.
  - **NEJM:** `0.123` style, floor `< 0.001`, up to `***`.
  - **Custom:** `1` to `15` decimal places with an optional scientific-notation cutoff.
- Display formatting never rounds numerical calculations. Multiplicity procedures and hypothesis decisions always use full IEEE-754 probabilities.

---

## Persisted analysis settings {#persisted-analysis-settings}

Every results sheet embeds the complete analysis specification that produced it:

- Sheet `id`, `title`, and `source_sheet_id`.
- Analysis kind and method-specific options (test selection, tails, pairing, degrees of freedom, multiplicity, weighting, fit method, parameter rules, or power targets).
- `selected_data_sets` (empty selects all compatible columns).
- `confidence_level` (continuous percentage in `(0, 100)`, defaulting to 95%).
- `p_value_format`.

These settings travel inside `.curvebook` archives, `.curvebook.d/` directories, and `.cbtemplate` templates. Re-running (`analysis.run`), updating options (`analysis.updateParams`), or retargeting to new data (`analysis.retargetData`) reproduces every choice deterministically. See the [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), and [Automation: CLI](automation/cli.md) for programmatic parameter schemas.
