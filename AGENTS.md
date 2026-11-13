# LumenCraft — Engineering & Agent Rules

LumenCraft is a standalone, clean-room, open-source, Rust-native biostatistics, nonlinear regression (curve fitting), typed scientific data-table, and publication graphing application targeting GraphPad Prism parity (and beyond). It runs natively on macOS, Windows, Linux, and FreeBSD, and in the browser via WebAssembly.

## Core Collaboration & Repository Rules

1. **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
2. **Check, then claim.** Before writing code, read the issue comments, search pull requests for `#N` and keywords, and check recent commits on `main`. Comment on the issue first, and release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it. Confirm the cause in the code, even when someone names a location.
4. **Keep fixes narrow and tested.** Fix what the issue describes, where the bug is. Add a test that fails without the fix.
5. **Review the diff before opening a pull request.** Identifiers exist as used, the test fails without the fix, nothing unrelated changed, and the description is true.
6. **Report only what ran.** Say what you tested and what you could not run. Without a local build or test, say so and rely on CI before marking ready.
7. **`main` changes through a pull request.** Do not push directly to `main`.
8. **Stop and ask only about** licensing outside the allowlist, publishing, releases, a new remote, secrets, spending money, destructive git, and anything else hard to undo. Otherwise keep working. `plan/` is a gitignored notebook, not rules; without it, do not ask for it and do not stop.
9. **Keep secrets and personal details out** of the repository, logs, and requests.
10. **Guards say why.** A check that looks removable says what it prevents. Read that before changing it.
11. **Instructions must be readable by everyone.** No notes about one machine, and no links to files a reader cannot open.
12. **No agent attribution.** A pull request does not say an agent wrote it.
13. **Time goes through `chrono`.** Do not hand-roll calendar conversion.

## Licence Allowlist

Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in this list:

- `ISC`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `Apache-2.0`
- `OFL-1.1`
- `Ubuntu-font-1.0`
- `Bitstream-Vera`
- `CC0-1.0`
- `CC-BY-4.0`
- `CC-BY-SA-4.0`
- `LicenseRef-Public-Domain`

## Start Every Session Here
1. Read [`ROADMAP.md`](ROADMAP.md) (stage, numbers by dimension, what's next) and the ranked work list in [`docs/gaps.md`](docs/gaps.md). Unless the user gives you a task, pick work from that list.
2. `plan/` is an optional gitignored notebook. Behaviour notes and local reference material may live under `plan/prism/` and `plan/papers/`. Do not ask for it and do not stop when it is absent. Orient, implement, test, verify, then record in the public docs.
3. Before touching a domain crate, read its governing implementation guide in [`docs/`](docs/): [`data-tables-and-transforms.md`](docs/data-tables-and-transforms.md), [`statistics-implementation.md`](docs/statistics-implementation.md), [`curve-fitting-implementation.md`](docs/curve-fitting-implementation.md), [`graphing-and-layouts.md`](docs/graphing-and-layouts.md), [`scripting-and-automation.md`](docs/scripting-and-automation.md), and [`development.md`](docs/development.md).

## Non-Negotiables
- **Clean-room: never use GraphPad or Dotmatics software (absolute rule).** Proprietary EULAs forbid reverse engineering, including driving or probing an installed binary to recreate it. So:
  - Never launch, run, script, automate (Prism script, AppleScript, COM, UI automation), screenshot or measure GraphPad Prism or any Dotmatics application, and never compare our output against an installed copy. Don't generate test `.prism`, `.pzfx` or `.pzf` files with them, not even "synthetic" ones.
  - Never read, disassemble or copy anything inside a GraphPad install (binaries, equation libraries, sample datasets, templates, colour schemes, help files, schemas, resources).
  - Never copy vendor-authored text, equation wording, guidance checklists, or sample files into this repo, tests included. Build synthetic `.prism` / `.pzfx` / CSV test archives from scratch in Rust test builders (`lumencraft-testkit`), and verify statistical & curve-fitting accuracy against public peer-reviewed literature, standard textbooks (Motulsky, Zar, Montgomery, Klein & Moeschberger, Hosmer & Lemeshow), and public-domain reference benchmarks such as the **NIST Statistical Reference Datasets (StRD)**.
  - Behaviour comes only from public documentation, textbooks, peer-reviewed statistical methods, cited by URL or citation, and from files users send us that they own (used locally, never committed).
  - `cargo xtask cleanroom` and `cargo xtask brands` (run by `cargo xtask ci`) fail on vendor copyright notices, vendor-created fixtures, and competitor brand names in user-visible text.
  - Never copy GPL/AGPL/LGPL code (R core `stats`, GNU Scientific Library, JASP, PSPP, Veusz, LabPlot, SciDAVis…). Every mathematical, statistical and optimization routine must be clean-room Rust or come from a dependency on the licence allowlist.
- **Standalone repository: no external suite dependencies.**
  - LumenCraft has no build, runtime, packaging, or CI dependency on external suite infrastructure or `craft-*` crates (enforced by `cargo xtask layers` and `cargo xtask brands`).
- **Assets: no proprietary iconography, palettes or templates — ever.**
  - Never add, copy, trace, redraw-from, embed or ship any icon, artwork, cursor, graph theme, colour scheme or template from GraphPad products or any unpermitted source.
  - **Every asset file must have a row in [`ASSETS.md`](ASSETS.md)** (path, author, source, licence, notes) and be recorded in [`ATTRIBUTION.md`](ATTRIBUTION.md) when third-party. `cargo xtask assets` (run by `cargo xtask ci`) fails on any unattributed asset.
  - **Font files are never committed to this repo.** Optional SIL OFL font faces may be supplied at build time via `LUMENCRAFT_FONTS_DIR` (`LUMENCRAFT_FONTS_DIR="/path/to/fonts-root" cargo xtask ci`), read by `crates/text/build.rs` into `lumencraft_text::BUNDLED_FONTS`; never add a font repository to `Cargo.toml`. Everything must build, test and run with `BUNDLED_FONTS` empty.
  - Code and original assets are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Prefer procedural generation in code for palettes, markers, axis breaks, error bars and cursors.
- **Never crash: no panics in shipped code.** A crash loses the researcher's unsaved data tables and analyses. Project archives, CSVs, pasted spreadsheets, user equations, MCP requests, control messages and command params are all untrusted input.
  - Fallible work returns `Result` (or `Option`); callers handle it or propagate it with `?`. Singular matrices, non-convergent nonlinear fits, zero degrees of freedom, all-excluded columns, `NaN`/`±Inf` inputs and malformed archives must surface structured domain outcomes or `EngineError`, never abort.
  - Workspace clippy lints ban these outside tests, so `cargo xtask ci` fails on them: `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`.
  - Don't panic implicitly either: use `.get(i)` instead of `v[i]` or `&s[a..b]` when indexing from untrusted row/column/subcolumn coordinates; cap row/column/replicate counts, archive decompression sizes and iteration budgets; check `f64::is_finite()` before casting or sorting.
  - Tests may unwrap and panic (`#[test]`, `#[cfg(test)]`, `tests/`, `testkit`). Integration-test files may declare test-scoped `#![allow(…)]` attributes where needed.
  - `lumencraft_engine::guard` is the last line of defence at entry points, catching unexpected panics to roll the session back and report an internal error, though wasm aborts on panic so `Result` remains mandatory everywhere.
- **Statistical & numerical rigor.**
  - Never silently drop excluded cells (`*`), missing values, or tied ranks: every analysis must document how missingness, replicate subcolumns, ties and degrees of freedom are handled.
  - Test every statistical test and curve-fitting model against analytical solutions and public-domain reference datasets (NIST StRD: Misra1a, Ratkowsky, Eckerle4, Thurber, Norris, Pontius, AtmWtAg…) with explicit relative tolerance bounds.
- **Everything is a command.** User-visible behaviour = a `CommandSpec` in `crates/engine/src/cmd/*` (id, label, menu path, shortcut, params doc, `enabled`, `run`) + tests. Interactive spreadsheet/graph/layout tools emit commands (Begin/Preview/Commit). UI-only commands live in `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`) and are mirrored to `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands`.
- **Adding a cursor.** Add the `Cursor` variant in `crates/tools/src/lib.rs`, return it from the tool with a unit test, and add its `[[cursor]]` entry to `crates/tools/src/cursors.toml` (plain behaviour terms and hotspot; never another vendor's name). Checked by `cargo xtask cursors`.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui/eframe/winit/rfd.
- **The UI is thin**: panels read engine state and act through commands. Colours come from theme tokens.
- **Rust only for native and web apps** (no handwritten JS/TS, Tauri, Electron or webview shells). **Never break wasm** (`cargo xtask wasm`).
- **Shared test corpora.** Never commit large binary fixtures to this repo; fetch public reference corpora (such as NIST StRD ASCII tables) into the gitignored `corpus/` directory pinned by commit/URL and SHA-256 verified (`cargo xtask corpus`).
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy `-D warnings` including the no-panic lints, tests, doctests, assets, brands, cleanroom, cursors, ui-commands, layers, wasm). One task id per commit (`M1.1: XY data table model`).

## Running and Inspecting the App
- `cargo run --release -p lumencraft -- --control 7979 [project.lumencraft|project.prism]` (pick a free port if 7979 is already bound).
- Headless CLI & MCP: `cargo run --release -p lumencraft-cli -- --help` and `lumencraft-cli mcp` (see [`docs/mcp.md`](docs/mcp.md) and [`docs/control-protocol.md`](docs/control-protocol.md)).
- Parallel agents: use separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; write manifests atomically; clean up temporary target dirs when done.

## Roadmap, Parity and Implementation Docs
- [`ROADMAP.md`](ROADMAP.md): stage, headline numbers, dimensions, features, upcoming milestones, progress log.
- [`docs/target-app-parity.md`](docs/target-app-parity.md): authoritative parity assessment (area table, methodology, evidence of what ships).
- [`docs/gaps.md`](docs/gaps.md): ranked shortfall and work list (`G1`–`G14`). [`docs/roadmap.md`](docs/roadmap.md): Alpha/Beta gates and milestones M0–M7.
- Implementation guides: [`architecture.md`](docs/architecture.md), [`development.md`](docs/development.md), [`data-tables-and-transforms.md`](docs/data-tables-and-transforms.md), [`statistics-implementation.md`](docs/statistics-implementation.md), [`curve-fitting-implementation.md`](docs/curve-fitting-implementation.md), [`graphing-and-layouts.md`](docs/graphing-and-layouts.md), [`scripting-and-automation.md`](docs/scripting-and-automation.md), [`control-protocol.md`](docs/control-protocol.md), [`mcp.md`](docs/mcp.md), [`releasing.md`](docs/releasing.md).
- Checklists: [`stats-parity.md`](docs/stats-parity.md), [`fitting-parity.md`](docs/fitting-parity.md), [`graphing-parity.md`](docs/graphing-parity.md), [`ui-parity.md`](docs/ui-parity.md), [`file-format-parity.md`](docs/file-format-parity.md), [`hardware-parity.md`](docs/hardware-parity.md), [`localization-parity.md`](docs/localization-parity.md).

When a task lands, update in the same PR: `docs/target-app-parity.md`, `docs/gaps.md`, the relevant parity checklist, `docs/roadmap.md`, and `ROADMAP.md` (plus `README.md` Status when headline numbers move). Grade by verified mathematical and interactive behaviour against public specifications, not by whether a menu stub exists.

## Contributor Credits (About Window)
- About ▸ Contributors/Models read `contributors/contributors.json` (commit stats; refreshed via `cargo xtask contributors` — never hand-edit) and `contributors/people.toml` (opt-in display/real names). See [`docs/contributors.md`](docs/contributors.md).
- Add or edit **only your own human's** entry in `contributors/people.toml` after asking them. Never guess a name or hand-edit `contributors.json`.

## CI Workflows
- Self-contained GitHub Actions workflows live in `.github/workflows/` and invoke `cargo xtask` and `packaging/*` directly. Keep CI changes separate from feature or bugfix pull requests unless required by the change.

New workspace crates include `[lints] workspace = true` in `Cargo.toml`. Production crate roots declare `#![forbid(unsafe_code)]` (and inherit workspace clippy `deny` lints) before modules; crate-level `//!` documentation may precede them.
