# CurveBook documentation

Start with the [README](../README.md) or the [online Get Started guide](https://hausergroup.github.io/curvebook/get-started.html) to install and use CurveBook, or the [beginner's guide](mcp-for-beginners.md) to connect your AI assistant, run an analysis, and save your work. For verified capabilities and current limitations, read [Status](STATUS.md) and [Beta known issues](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues). Contributors should start with [Contributing](../CONTRIBUTING.md), [Development](development.md), and the [active roadmap](ROADMAP.md).

## Using CurveBook

| Guide | Contents |
|---|---|
| [Get started & downloads](https://hausergroup.github.io/curvebook/get-started.html) | Installer guide (*Which file do I need?*), per-OS install steps, verification, and first steps |
| [Beta known issues](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues) | Beta supported scope, `[Draft]` non-English locale note, platform caveats, and troubleshooting |
| [Verifying downloads](release/verifying-downloads.md) | Checking release checksums (`SHA256SUMS.txt`) and Sigstore build provenance |
| [Beginner's guide (MCP)](mcp-for-beginners.md) | Step-by-step AI assistant setup, first analysis, saving, and troubleshooting |
| [MCP server](mcp.md) | Protocol, source builds, client configuration, Welch's *t*-test example, toolsets, and desktop connection |
| [Command reference](commands.md) | Generated engine command catalog and parameters |
| [Scripting and automation](implementation/scripting-and-automation.md) | CLI, templates, and batch recipes |
| [Control protocol](control-protocol.md) | Local desktop JSONL interface and security boundaries |
| [File formats](file-formats.md) | Persisted formats, versions, and migrations |

## Contributing and implementation

Read the [contributor guide](../CONTRIBUTING.md), [agent rules](../AGENTS.md), [known pitfalls](pitfalls.md), and [extension checklist](extending.md) before changing code.

| Guide | Contents |
|---|---|
| [Development](development.md) | Prerequisites, source build quick start, everyday commands, tests, coverage, and local configuration |
| [Continuous integration](ci.md) | Quality stages, shared test binaries, deadlines, and local verification |
| [Localization](localization.md) | Catalog entries, locales, fallback, fonts, and translation review |
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
| Overall assessment | [Feature parity](evidence/overview.md) |
| Statistics | [Statistical parity](evidence/statistics.md) |
| Curve fitting | [Fitting parity](fitting-parity.md) |
| Graphing | [Graphing parity](evidence/graphing.md) |
| Interchange | [File-format parity](evidence/file-formats.md) |
| User interface | [UI parity](evidence/user-interface.md) and [GUI workflows](gui-workflows.md) |
| Platforms | [Hardware parity](evidence/platforms.md) |
| Localization | [Localization parity](evidence/localization.md) |
| Scientific quality of life | [Enhancements](planning/qol-enhancements.md) and [calculator reference](planning/calculators-and-quick-stats.md) |
| Historical progress | [Roadmap history](history/roadmap.md) |
| Legacy tracking | [Gap mapping and known defects](history/legacy-ids-and-defects.md) |
| Audits | [Audit protocol](audit/README.md), [workflow](audit/workflow.md), [slices](audit/slices.md), [progress](history/audit/round1-progress.md), [work log](audit/work-log.md), [Round 2 review](history/audit/round2-post-ebcc622.md), and [R0–R9 milestone review ledger](history/audit/review-2026-11-16-ledger.md) |

## Releases and reference files

- [GitHub Pages site](index.html) and [Privacy policy](privacy.html): static entry point introducing CurveBook, complete-workspace sharing, downloads, and zero-telemetry privacy disclosures ([design decisions](release/website-design.md), [preview and publication checklist](release/github-pages.md)).
- [Get Started](https://hausergroup.github.io/curvebook/get-started.html) and [Beta known issues](https://hausergroup.github.io/curvebook/troubleshooting.html#known-issues): single canonical home for Beta installer selection, platform install steps, verification, supported surfaces, `[Draft]` non-English locale status, unsigned Windows / FreeBSD preview caveats, and known limits.
- [Alpha known issues & release notes (historical)](history/release/alpha-known-issues.md): historical `v0.1.0-alpha.1` / Alpha prerelease scope, caveats, and release notes (`R0.20`).
- [Alpha branding clearance](history/release/alpha-branding-clearance.md): strict `R0.21` / `R10.14` branding, clean-room, and asset-provenance verification records.
- [Release guide](release/releasing.md): packaging, verification, and release gates.
- [Verifying a download](release/verifying-downloads.md): checking a release file's checksum and Sigstore build provenance on every platform.
- [Windows signing](release/windows-signing.md): test signing and distribution.
- [Contributor credits](contributors.md), [asset inventory](../ASSETS.md), and [attribution](../ATTRIBUTION.md).
- [Schemas](schemas/): generated JSON schemas for documents, archives, preferences, journals, and batches.
- [Images](images/): clean-room workspace screenshots.

## Directory conventions

Keep shared entry points, governance references, and gate-checked documents at the top level. Put domain implementation guides in `implementation/`, release procedures in `release/`, and supporting plans and historical progress in `planning/`. Audit records, generated schemas, and screenshots have their own directories.

Add new guides to this index and update repository links when moving a document. Regenerate command references and schemas through their `cargo xtask` commands.
