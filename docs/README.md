# CurveBook documentation

Start with the [README](../README.md). People using CurveBook should read the [online documentation](https://curvebook.org/manual/latest/get-started.html), which is generated from the `reference/` pages below. Contributors should start with [Contributing](../CONTRIBUTING.md), the [agent and contributor rules](../AGENTS.md), and the [active roadmap](ROADMAP.md).

## Using CurveBook

These pages are published on the documentation site by `cargo xtask docs-site`. Edit the Markdown or landing-page HTML here, not the generated HTML. Published pages follow the two-tier audience split defined in [`AGENTS.md`](../AGENTS.md#documentation-audience-tiers-docs-site---check) and [`release/github-pages.md`](release/github-pages.md#audience-tiers-scientist-guides-vs-technical-references): **Tier 1 scientist guides** stay short and plain-language (enforced by `docs-site --check`), while **Tier 2 technical references** hold exhaustive flags, schemas, equations, and format specifications. Read each file's top-of-file `Audience:` comment before editing.

| Guide | Contents |
|---|---|
| [Get started](reference/get-started.md) | Which file to download, installing on each platform, verifying downloads, fonts, updating, and uninstalling |
| [Beginner's guide (MCP)](mcp-for-beginners.md) | Connecting an AI assistant, a first analysis, and saving work |
| [Concepts](reference/concepts.md), [tables](reference/tables.md), [analyses](reference/analyses.md), [curve fitting](reference/fitting.md), [graphs](reference/graphs.md), [files](reference/files.md), [settings](reference/settings.md), [quality-of-life tools](reference/qol.md) | What CurveBook does and how its parts fit together |
| [Desktop app](reference/desktop-gui.md) | Getting started in the desktop GUI |
| [Automation: MCP](reference/automation/mcp.md), [CLI](reference/automation/cli.md), [control protocol and batch](reference/automation/control-and-batch.md) | Reference for driving CurveBook from scripts and AI assistants |
| [Troubleshooting & known issues](reference/troubleshooting.md) | Install problems, fonts, MCP connections, error codes, and the current release's known issues |
| [Privacy & security](reference/privacy-security.md) and [glossary](reference/glossary.md) | Privacy, licences, security, and terms |

## Contributing

Read [AGENTS.md](../AGENTS.md), [known pitfalls](pitfalls.md), [engineering pitfalls](pitfalls-engineering.md), and the [extension checklist](extending.md) before changing code. The documentation site links to these guides on GitHub, so keep their paths stable.

| Guide | Contents |
|---|---|
| [Known pitfalls](pitfalls.md) and [engineering pitfalls](pitfalls-engineering.md) | Documented CurveBook incidents and general Rust/tooling rules adopted across the codebase |
| [Development](development.md) | Prerequisites, building from source, everyday commands, tests, coverage, and local configuration |
| [Continuous integration](ci.md) | CI stages, shared test binaries, deadlines, and local verification |
| [Architecture](architecture.md) | Crate layers and dependency boundaries |
| [Design language](design-language.md) | OKLab/OKLCH derived UI tokens, data-first colour rule, fixed scales, action hierarchy, and CI enforcement gates |
| [Extending CurveBook](extending.md) | Where and how to add commands, analyses, models, formats, cursors, locales, migrations, and assets |
| [Command IDs](command-ids.md) and [command reference](commands.md) | Naming and replay stability; generated engine command catalog (`cargo xtask commands`) |
| [File formats](file-formats.md) and [schemas](schemas/) | Persisted formats, versions, and migrations; generated JSON schemas (`cargo xtask schemas`) |
| [MCP server internals](mcp.md) and [control protocol specification](control-protocol.md) | Source builds, client presets, and implementation detail behind the published automation reference |
| [Localization](localization.md) | Catalog entries, locales, fallback, fonts, and translation review |
| [Licences](licences.md) | Dependency and asset licence policy (contributor credits live in [CONTRIBUTING.md](../CONTRIBUTING.md#6-licensing-attribution--security)) |
| Implementation specifications | [Data tables and transforms](implementation/data-tables-and-transforms.md), [statistics](implementation/statistics-implementation.md), [curve fitting](implementation/curve-fitting-implementation.md), [graphing and layouts](implementation/graphing-and-layouts.md), and [scripting and automation](implementation/scripting-and-automation.md) |

## Planning, status & evidence

[ROADMAP.md](ROADMAP.md) owns claims and pending work, and [STATUS.md](STATUS.md) records verified capabilities and known limits. The evidence checklists separate engine and agent-harness evidence from human GUI completion, and `cargo xtask status` checks the `[x]` claims in the user-interface checklist and the GUI workflow table.

| Area | Document |
|---|---|
| Overall assessment | [Capability evidence overview](evidence/overview.md) |
| Statistics | [Statistics evidence](evidence/statistics.md) |
| Curve fitting | [Curve-fitting evidence](evidence/curve-fitting.md) |
| Graphing | [Graphing evidence](evidence/graphing.md) |
| Interchange | [File-format evidence](evidence/file-formats.md) and [interop format notes](evidence/interop-format-notes.md) |
| User interface | [User-interface evidence](evidence/user-interface.md), [GUI workflows](gui-workflows.md), and [GUI layout specification](planning/gui-layout-spec.md) |
| Platforms | [Platform evidence](evidence/platforms.md) |
| Localization | [Localization evidence](evidence/localization.md) |
| Quality of life | [Quality-of-life enhancements](planning/qol-enhancements.md) and [calculator reference](planning/calculators-and-quick-stats.md) |

## Releases & the website

| Document | Contents |
|---|---|
| [Release guide](release/releasing.md) | Packaging, verification, and release gates |
| [Desktop smoke QA](release/desktop-smoke-qa.md) | Manual desktop walkthrough for each release |
| [Windows signing](release/windows-signing.md) | Test signing and distribution |
| [GitHub Pages site](release/github-pages.md) | Design decisions, preview, checks, and publication for [`index.html`](index.html), [`privacy.html`](privacy.html), and the generated manual |
| [Search discoverability actions](release/seo-actions.md) | Catalogue of possible search-engine actions for the site, with status and policy constraints |
| [Images](images/) | Clean-room screenshots, attributed in [ASSETS.md](../ASSETS.md) |

## History

[`history/`](history/README.md) holds frozen records: completed roadmap items, legacy ID mappings, Alpha release records, and Pages verification logs. Do not update their numbers; current facts belong in `ROADMAP.md` and `STATUS.md`.

## Directory conventions

| Folder | Holds |
|---|---|
| top level | Entry points, gate-checked governance files (`ROADMAP.md`, `STATUS.md`, `gui-workflows.md`), generated `commands.md`, contributor guides the documentation site links to, and the Pages files (`index.html`, `privacy.html`, `site.css`) |
| `reference/` | User documentation rendered to the documentation site |
| `implementation/` | Domain implementation specifications |
| `evidence/` | Capability evidence checklists |
| `planning/` | Active specifications and deferred scope (`gui-layout-spec.md`, `qol-enhancements.md`, `calculators-and-quick-stats.md`) |
| `release/` | Release and website procedures |
| `history/` | Frozen records (see above) |
| `schemas/`, `images/` | Generated schemas and attributed screenshots (manual HTML and `_root/{sitemap.xml,llms.txt,llms-full.txt}` are generated in CI into `target/docs-manual/` by `.github/workflows/pages.yml`) |

Add new guides to this index and update every repository link when moving a document. Regenerate command references and schemas through their `cargo xtask` commands.
