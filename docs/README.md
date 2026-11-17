# CurveBook documentation

Start with the [beginner's guide](mcp-for-beginners.md) to connect your AI assistant, run an analysis, and save your work. For verified capabilities and current limitations, read [Status](STATUS.md). Contributors should start with [Development](development.md) and the [active roadmap](ROADMAP.md).

## Using CurveBook

| Guide | Contents |
|---|---|
| [Beginner's guide](mcp-for-beginners.md) | Setup, first analysis, saving, and troubleshooting |
| [MCP server](mcp.md) | Protocol, configuration, toolsets, and desktop connection |
| [Command reference](commands.md) | Generated engine command catalog and parameters |
| [Scripting and automation](implementation/scripting-and-automation.md) | CLI, templates, and batch recipes |
| [Control protocol](control-protocol.md) | Local desktop JSONL interface and security boundaries |
| [File formats](file-formats.md) | Persisted formats, versions, and migrations |

## Contributing and implementation

Read the [contributor guide](../CONTRIBUTING.md), [agent rules](../AGENTS.md), [known pitfalls](pitfalls.md), and [extension checklist](extending.md) before changing code.

| Guide | Contents |
|---|---|
| [Development](development.md) | Toolchain, builds, tests, coverage, and local configuration |
| [Architecture](architecture.md) | Crate layers and dependency boundaries |
| [Data tables and transforms](implementation/data-tables-and-transforms.md) | Table archetypes, formulas, reactive data, and engine contracts |
| [Statistics](implementation/statistics-implementation.md) | Numerical methods, statistical tests, and oracle requirements |
| [Curve fitting](implementation/curve-fitting-implementation.md) | Solvers, uncertainty, equation library, and expression language |
| [Graphing and layouts](implementation/graphing-and-layouts.md) | Scene compilation, axes, typography, layouts, and exports |
| [Command IDs](command-ids.md) | Naming and replay stability |
| [Licences](licences.md) | Dependency and asset licence policy |

## Status, planning, and evidence

The [active roadmap](ROADMAP.md) owns claims and pending work. [Status](STATUS.md) records verified capabilities and known limits. The checklists below separate engine and agent-harness evidence from human GUI completion.

| Area | Evidence or planning document |
|---|---|
| Overall assessment | [Feature parity](target-app-parity.md) |
| Statistics | [Statistical parity](stats-parity.md) |
| Curve fitting | [Fitting parity](fitting-parity.md) |
| Graphing | [Graphing parity](graphing-parity.md) |
| Interchange | [File-format parity](file-format-parity.md) |
| User interface | [UI parity](ui-parity.md) and [GUI workflows](gui-workflows.md) |
| Platforms | [Hardware parity](hardware-parity.md) |
| Localization | [Localization parity](localization-parity.md) |
| Scientific quality of life | [Enhancements](planning/qol-beyond-parity.md) and [calculator reference](planning/calculators-and-quick-stats.md) |
| Historical progress | [Roadmap history](planning/roadmap-history.md) |
| Legacy tracking | [Gap mapping and known defects](gaps.md) |
| Audits | [Audit protocol](audit/README.md), [workflow](audit/workflow.md), [slices](audit/slices.md), [progress](audit/progress.md), [work log](audit/work-log.md), and [Round 2 review](audit/round2-post-ebcc622.md) |

## Releases and reference files

- [Release guide](release/releasing.md): packaging, verification, and release gates.
- [Windows signing](release/windows-signing.md): test signing and distribution.
- [Contributor credits](contributors.md), [asset inventory](../ASSETS.md), and [attribution](../ATTRIBUTION.md).
- [Schemas](schemas/): generated JSON schemas for documents, archives, preferences, journals, and batches.
- [Images](images/): clean-room workspace screenshots.

## Directory conventions

Keep shared entry points, governance references, and gate-checked documents at the top level. Put domain implementation guides in `implementation/`, release procedures in `release/`, and supporting plans and historical progress in `planning/`. Audit records, generated schemas, and screenshots have their own directories.

Add new guides to this index and update repository links when moving a document. Regenerate command references and schemas through their `cargo xtask` commands.
