# Audit slices

The parts of the codebase that reviewers claim, one at a time. Follow the protocol in [`README.md`](README.md). Sizes are approximate lines of Rust under `src/` at `71f37d8`, the first round 1 base. Tests are out of scope unless a slice says otherwise, but reading them helps you see what is already covered.

Slices `S01`–`S13` follow the crate layering. `X1` and `X2` cross crates on purpose: the two defects fixed after the 2026-11-13 review (control server, CLI sandbox) sat on seams between crates, where a single-crate reviewer would miss them. Assign `X1`/`X2` to reviewers who have not reviewed a crate slice.

| ID | Slice | Paths | Size | Governing guide |
|---|---|---|---:|---|
| `S01` | Numerical foundations | `crates/math`, `crates/palette` | 8.5k | [statistics-implementation.md §1](../statistics-implementation.md) |
| `S02` | Statistics | `crates/stats` | 12k | [statistics-implementation.md](../statistics-implementation.md), [stats-parity.md](../stats-parity.md) |
| `S03` | Curve fitting and equation language | `crates/fit` | 11k | [curve-fitting-implementation.md](../curve-fitting-implementation.md) |
| `S04` | Transforms and calculated variables | `crates/transform` | 6k | [data-tables-and-transforms.md](../data-tables-and-transforms.md) |
| `S05` | Document model | `crates/doc` | 5.5k | [data-tables-and-transforms.md](../data-tables-and-transforms.md), [architecture.md](../architecture.md) |
| `S06` | File formats and import | `crates/format` | 6k | [file-format-parity.md](../file-format-parity.md) |
| `S07` | Engine: commands, DAG, undo | `crates/engine` | 7.5k | [architecture.md](../architecture.md), [scripting-and-automation.md](../scripting-and-automation.md) |
| `S08` | Chart compiler and scales | `crates/chart` | 12k | [graphing-and-layouts.md](../graphing-and-layouts.md) |
| `S09` | Text, raster, SVG and PDF output | `crates/text`, `crates/render`, `crates/svg`, `crates/pdf` | 9k | [graphing-and-layouts.md](../graphing-and-layouts.md) |
| `S10` | UI shell | `crates/ui-egui` | 12k | [ui-parity.md](../ui-parity.md), [control-protocol.md](../control-protocol.md) |
| `S11` | Interactive tools and app entry points | `crates/tools`, `apps/curvebook`, `apps/curvebook-web` | 3.5k | [ui-parity.md](../ui-parity.md) |
| `S12` | Headless CLI and MCP server | `apps/curvebook-cli`, `crates/mcp` | 4.5k | [scripting-and-automation.md](../scripting-and-automation.md), [mcp.md](../mcp.md) |
| `S13` | Build guards and test kit | `xtask`, `crates/testkit` | 5.5k | [development.md](../development.md) |
| `X1` | Untrusted input, end to end | Entry points in `S10`–`S12` through `S07` to file I/O and allocation in `S06`/`S09` | — | [control-protocol.md](../control-protocol.md), [mcp.md](../mcp.md) |
| `X2` | Data correctness, end to end | `S05` cells through `S04`/`S02`/`S03` results to `S06` save and reopen | — | [data-tables-and-transforms.md](../data-tables-and-transforms.md) |

## Focus by slice

These questions come first. The general checklist in [`README.md`](README.md#what-to-look-for) still applies.

**`S01` Numerical foundations.** Accuracy and termination of special functions, distributions and root finders at extreme arguments (`NaN`, `±Inf`, subnormals, huge degrees of freedom). Do iteration limits return an error rather than a last guess? Are linear-algebra rank and singularity checks scaled correctly?

**`S02` Statistics.** For each test, check the formula, the degrees of freedom, and the handling of ties, missing values, excluded cells and replicate subcolumns against the cited textbook. Look for confidence limits or p-values that fall back to a plausible number when a solver fails (the Koopman fallback in Known defects is one example of the pattern). Watch for zero variance, n = 1 and all-excluded groups.

**`S03` Curve fitting and equation language.** Solver convergence and the reporting of non-convergence; profile-likelihood CIs; weighting with zero or negative Y. In `dsl.rs`: parser and VM limits, dataset prefixes, bytecode jump targets, ODE integration, and any recursion over `Expr`.

**`S04` Transforms.** The formula parser and evaluator (depth, cycles in `formula/dag.rs`), normalize and baseline with zero or missing references, reshape and select with out-of-range indices, and simulation seeds.

**`S05` Document model.** Invariants that constructors enforce but deserialization or mutation skips: row, column and subcolumn caps, ID uniqueness, link targets, and exclusion masks staying aligned with cells after resizing.

**`S06` File formats.** ZIP, XML, CSV, PZFX, `.prism` and `.curvebook` readers given hostile input: size and count caps, offset arithmetic, depth limits, and the cell budget on every table kind. Also check that save and reopen is lossless, including non-finite values and empty tables. `recovery.rs` is new: review it if it exists at your claimed base.

**`S07` Engine.** Does every command validate its parameters before mutating? Does it roll back cleanly on error? Is the DAG recalculation atomic? Look for filesystem reads and writes driven by parameters, and for errors swallowed while reading files. Undo memory growth is a Known defect. Look for anything else of that kind.

**`S08` Chart compiler.** Tick generation and axis mapping with degenerate domains (zero span, log of non-positive values, huge ratios, broken-axis segments), loop and allocation bounds, and empty or all-missing series.

**`S09` Output.** Raster size and memory limits, TIFF/PNG/PDF offsets and lengths, text escaping in SVG and PDF (injection through titles and labels), and font fallback when the bundled font list is empty.

**`S10` UI shell.** Control protocol parsing and error isolation; every `ui.*` command's parameter bounds; locale and theme switching; grid view indexing from scroll and selection coordinates.

**`S11` Tools and app entry points.** Begin/Preview/Commit sequences that are abandoned or arrive out of order, startup with unreadable or mixed project files, and command-line argument parsing.

**`S12` CLI and MCP.** The `--workdir` sandbox against every parameter key engine commands actually read for paths (a listed key that actually means inline text, `csv`, was fixed in `71f37d8`; now look for path keys missing from the list). Manifest parsing, MCP JSON-RPC framing and size limits, and file writes with no containment.

**`S13` Build guards.** Can `cargo xtask cleanroom`, `brands`, `assets`, `layers` or `ui-commands` pass on input they are meant to reject (case, encoding, path tricks, files they skip)? Corpus hashes and download pinning.

**`X1` Untrusted input, end to end.** Trace each external entry point (control socket, MCP stdio, CLI scripts and manifests, opened files) to the engine and on to every filesystem access and large allocation. For each path, name the check that bounds it, or report that none does.

**`X2` Data correctness, end to end.** Follow excluded cells, missing values, replicate subcolumns and ties from the data table through transforms and analyses into result sheets, graphs and a save-and-reopen. Report any stage that drops, reorders or reinterprets them without saying so.
