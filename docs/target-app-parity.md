# Parity with GraphPad Prism 11

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** initial clean-room Prism 11.1 parity baseline & subsystem implementation guides wired in · **Target:** GraphPad Prism 11 (11.1.x; public guides & release notes through 11.1.0, August 2026)

Authoritative clean-room parity assessment for LumenCraft. Summary in [`ROADMAP.md`](../ROADMAP.md); ranked work list in [`gaps.md`](gaps.md); milestone gates in [`roadmap.md`](roadmap.md).

## Methodology

Every score in this document is graded strictly from shipped, tested code in `crates/*` and `apps/*` against public documentation (`https://www.graphpad.com/guides/prism/latest/{user-guide,statistics,curve-fitting}/` and public release notes at `https://www.graphpad.com/updates`), peer-reviewed statistical methods, and certified reference benchmarks (NIST StRD). In accordance with [`AGENTS.md`](../AGENTS.md), no proprietary binary, installation directory, or vendor-created sample file is ever inspected or run.

### Scoring rubric (per feature area)

| Score | Meaning |
|---:|---|
| `0%` | Absent or M0 crate stub only. |
| `10–30%` | Core data types and happy-path math exist headlessly with unit tests, not yet wired through engine commands or UI. |
| `40–65%` | Mainstream variants work end to end (engine + CLI/MCP + UI + `.lumencraft` save/reopen) and pass NIST/analytical tests; secondary options or diagnostics missing. |
| `70–85%` | Everyday wet-lab researcher workflow complete, including missing-value edge cases, multiplicity CIs, plain-language checklists, and vector SVG/PDF export. |
| `90–100%` | Indistinguishable mathematical precision and workflow completeness against public Prism 11.1 specifications, property-tested against hostile inputs. |

### Ready for real work

`Feature depth` weights the 10 feature areas below by scientific frequency of use. `Ready for real work` discounts feature depth by workflow-blocking multipliers:
1. Lossless native `.lumencraft` + CSV/TSV + legacy `.pzfx`/`.prism` project interchange,
2. Interactive spreadsheet ergonomics (subcolumns, paste-transpose, non-destructive exclusions, undo/redo),
3. Certified numerical accuracy on extreme degrees of freedom, tied ranks, missing repeated measures, and stiff nonlinear fits.

### Calibration

Hour ranges are single-agent wall-clock hours calibrated from sibling Craft repositories (`~1.0–2.0 h` per self-contained statistical test or equation family with NIST/analytical tests + command wiring; `~6–12 h` per multi-tab dialog or chart family). Parallel agents (`4–6`) achieve `~3.2–3.8×` wall-clock speedup when partitioned cleanly across `math`/`stats`, `fit`, `doc`/`transform`/`format`, `chart`/`svg`/`pdf`, and `ui-egui`.

---

## Feature areas

| # | Area | Weight | Score | Est. remaining (h) | Spec & checklist |
|---:|---|---:|---:|---:|---|
| 1 | **Typed data tables (8 archetypes), exclusions & Info constants** | 14% | 2% | 25–40 | [data-tables-and-transforms.md](data-tables-and-transforms.md), [file-format-parity.md](file-format-parity.md) |
| 2 | **Calculated variables (`=`), window functions & derived transforms** | 8% | 0% | 20–30 | [data-tables-and-transforms.md](data-tables-and-transforms.md) |
| 3 | **Descriptive statistics, normality/lognormality & Grubbs/ROUT outliers** | 8% | 0% | 18–26 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 4 | **Comparisons: *t*-tests, 1/2/3-way & Multifactor ANOVA, RM, REML & multiplicity** | 16% | 0% | 45–70 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 5 | **Nonlinear regression core: LM solver, weights, Cauchy/ROUT, global fits, profile CIs, AICc/*F*** | 14% | 0% | 40–60 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 6 | **Built-in equation library (dose–response, binding, kinetics, growth) & custom ODE/DSL** | 10% | 0% | 30–48 | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 7 | **Contingency, Kaplan–Meier/Cox survival, ROC, multiple GLMs, PCA & clustering** | 10% | 0% | 35–55 | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 8 | **Publication graphing: 8 graph families, nonlinear/broken axes, live *p*-value brackets** | 10% | 0% | 35–55 | [graphing-and-layouts.md](graphing-and-layouts.md), [graphing-parity.md](graphing-parity.md) |
| 9 | **Page Layouts, style cloning ("Wand"), vector SVG/PDF & high-DPI TIFF/PNG export** | 5% | 0% | 18–28 | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) |
| 10 | **Workspace UX, `.lumencraft`/`.prism`/`.pzfx` I/O, templates, CLI, `--control` & MCP** | 5% | 8% | 25–40 | [ui-parity.md](ui-parity.md), [scripting-and-automation.md](scripting-and-automation.md) |
| | **Total / Weighted depth** | **100%** | **~1%** | **291–452 h** | (plus QA, localization & platform polish → 440–680 h full parity) |

---

## Evidence: what ships today (`M0`)

- **Workspace & dependency enforcement (`xtask/src/layers.rs`):** 17 library crates (`math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf`, `tools`, `engine`, `ui-egui`, `mcp`, `testkit`) and 3 apps (`lumencraft`, `lumencraft-cli`, `lumencraft-web`) compile cleanly under Rust 2024 with `#![forbid(unsafe_code)]` and workspace-wide `deny` lints on `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable`.
- **Optional font embedding (`crates/text`, `crates/ui-egui`):** Both `--all-features` and `--no-default-features` (`bundled-fonts` disabled) are tested in `cargo xtask ci` (`crates/text/tests/without_bundled_fonts.rs`, `crates/ui-egui/tests/install_fonts.rs`).
- **UI command catalog & MCP sync (`xtask/src/ui_commands.rs`):** `crates/ui-egui/src/menus/ui_commands.rs` syncs deterministically into `crates/mcp/src/ui_catalog.rs` without violating L6 sideways isolation.
- **Clean-room, brand & asset gates (`xtask/src/{cleanroom,brands,assets,cursors}.rs`):** Automated CI verification of `ASSETS.md`, competitor brand exclusion in UI/packaging strings, cursor TOML sync, and vendor signature rejection.
- **Packaging (`packaging/*`):** Validated desktop entries, AppStream metainfo, macOS bundle generator (`cargo xtask bundle`), WiX v5 MSI manifest, FreeBSD manifest, and WASM Trunk shell.

## Missing by area (summary)

Detailed item-by-item checkboxes live in [`stats-parity.md`](stats-parity.md), [`fitting-parity.md`](fitting-parity.md), [`graphing-parity.md`](graphing-parity.md), [`ui-parity.md`](ui-parity.md), [`file-format-parity.md`](file-format-parity.md), [`hardware-parity.md`](hardware-parity.md), and [`localization-parity.md`](localization-parity.md); execution order is ranked in [`gaps.md`](gaps.md).
