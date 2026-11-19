# CurveBook documentation

Start with the [README](../README.md). People using CurveBook should read the [online documentation](https://hausergroup.github.io/curvebook/get-started.html), which is generated from the `reference/` pages below. Contributors should start with [Contributing](../CONTRIBUTING.md), the [agent and contributor rules](../AGENTS.md), and the [active roadmap](ROADMAP.md).

## Using CurveBook

These pages are published on the documentation site by `cargo xtask docs-site`. Edit the Markdown here, not the generated HTML.

| Guide | Contents |
|---|---|
| [Get started](reference/get-started.md) | Which file to download, installing on each platform, verifying downloads, fonts, updating, and uninstalling |
| [Beginner's guide (MCP)](mcp-for-beginners.md) | Connecting an AI assistant, a first analysis, and saving work |
| [Concepts](reference/concepts.md), [tables](reference/tables.md), [graphs](reference/graphs.md), [files](reference/files.md), [settings](reference/settings.md), [quality-of-life tools](reference/qol.md) | What CurveBook does and how its parts fit together |
| [Desktop app](reference/desktop-gui.md) | Getting started in the desktop GUI |
| [Automation: MCP](reference/automation/mcp.md), [CLI](reference/automation/cli.md), [control protocol and batch](reference/automation/control-and-batch.md) | Reference for driving CurveBook from scripts and AI assistants |
| [Troubleshooting & known issues](reference/troubleshooting.md) | Install problems, fonts, MCP connections, error codes, and the current release's known issues |
| [Privacy & security](reference/privacy-security.md) and [glossary](reference/glossary.md) | Privacy, licences, security, and terms |

## Contributing

Read [AGENTS.md](../AGENTS.md), [known pitfalls](pitfalls.md), and the [extension checklist](extending.md) before changing code. The documentation site links to these guides on GitHub, so keep their paths stable.

| Guide | Contents |
|---|---|
| [Development](development.md) | Prerequisites, building from source, everyday commands, tests, coverage, and local configuration |
| [Continuous integration](ci.md) | CI stages, shared test binaries, deadlines, and local verification |
| [Architecture](architecture.md) | Crate layers and dependency boundaries |
| [Extending CurveBook](extending.md) | Where and how to add commands, analyses, models, formats, cursors, locales, migrations, and assets |
| [Command IDs](command-ids.md) and [command reference](commands.md) | Naming and replay stability; generated engine command catalog (`cargo xtask commands`) |
| [File formats](file-formats.md) and [schemas](schemas/) | Persisted formats, versions, and migrations; generated JSON schemas (`cargo xtask schemas`) |
| [MCP server internals](mcp.md) and [control protocol specification](control-protocol.md) | Source builds, client presets, and implementation detail behind the published automation reference |
| [Localization](localization.md) | Catalog entries, locales, fallback, fonts, and translation review |
| [Licences](licences.md) and [contributor credits](contributors.md) | Dependency and asset licence policy; how contributors are credited |
| Implementation specifications | [Data tables and transforms](implementation/data-tables-and-transforms.md), [statistics](implementation/statistics-implementation.md), [curve fitting](implementation/curve-fitting-implementation.md), [graphing and layouts](implementation/graphing-and-layouts.md), and [scripting and automation](implementation/scripting-and-automation.md) |

## Planning, status & evidence

[ROADMAP.md](ROADMAP.md) owns claims and pending work, and [STATUS.md](STATUS.md) records verified capabilities and known limits. The evidence checklists separate engine and agent-harness evidence from human GUI completion, and `cargo xtask status` checks the `[x]` claims in the user-interface checklist and the GUI workflow table.

| Area | Document |
|---|---|
| Overall assessment | [Capability evidence overview](evidence/overview.md) |
| Statistics | [Statistics evidence](evidence/statistics.md) |
| Curve fitting | [Curve-fitting evidence](fitting-parity.md) (moves to `evidence/` once its in-flight edits land) |
| Graphing | [Graphing evidence](evidence/graphing.md) |
| Interchange | [File-format evidence](evidence/file-formats.md) |
| User interface | [User-interface evidence](evidence/user-interface.md) and [GUI workflows](gui-workflows.md) |
| Platforms | [Platform evidence](evidence/platforms.md) |
| Localization | [Localization evidence](evidence/localization.md) |
| Quality of life | [Quality-of-life enhancements](planning/qol-enhancements.md) and [calculator reference](planning/calculators-and-quick-stats.md) |
| Numerical oracles | Scope notes for independent `D1` reference tests: [curve transforms](planning/oracle-curve-transforms.md), [distribution tails](planning/oracle-distribution-tails.md), and [non-NIST fitting](planning/oracle-non-nist-fitting.md) |
| Code audits | [Audit playbook](audit/README.md), [workflow](audit/workflow.md), [slices](audit/slices.md), and [work log](audit/work-log.md) |

## Releases & the website

| Document | Contents |
|---|---|
| [Release guide](release/releasing.md) | Packaging, verification, and release gates |
| [Desktop smoke QA](release/desktop-smoke-qa.md) | Manual desktop walkthrough for each release |
| [Windows signing](release/windows-signing.md) | Test signing and distribution |
| [GitHub Pages site](release/github-pages.md) | Design decisions, preview, checks, and publication for [`index.html`](index.html), [`privacy.html`](privacy.html), and the generated manual |
| [Images](images/) | Clean-room screenshots, attributed in [ASSETS.md](../ASSETS.md) |

## History

[`history/`](history/README.md) holds frozen records: completed roadmap items, legacy ID mappings and known defects, finished audit rounds, Alpha release records, and superseded status snapshots. Do not update their numbers; current facts belong in `ROADMAP.md` and `STATUS.md`.

## Directory conventions

| Folder | Holds |
|---|---|
| top level | Entry points, gate-checked governance files (`ROADMAP.md`, `STATUS.md`, `gui-workflows.md`), generated `commands.md`, contributor guides the documentation site links to, and the Pages files (`index.html`, `privacy.html`, `site.css`, `.nojekyll`) |
| `reference/` | User documentation rendered to the documentation site |
| `implementation/` | Domain implementation specifications |
| `evidence/` | Capability evidence checklists |
| `planning/` | Active plans, deferred scope, and oracle scope notes |
| `release/` | Release and website procedures |
| `audit/` | The reusable code-audit playbook |
| `history/` | Frozen records (see above) |
| `schemas/`, `images/`, `manual/` | Generated schemas, attributed screenshots, and documentation-site snapshots |

Add new guides to this index and update every repository link when moving a document. Regenerate command references and schemas through their `cargo xtask` commands.
