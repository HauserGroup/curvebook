# Development

> **Last updated:** 2026-11-13 · **Change:** added subsystem implementation-guide index, font/web/env/corpus sections, and `#robustness-lumencraft-never-crashes` engineering contract.

## Prerequisites

- **Rust 1.95+** (`edition = "2024"`, `resolver = "3"`).
- Optional tools:
  - [`cargo-nextest`](https://nexte.st/) — used automatically by `cargo xtask test` when installed; falls back to `cargo test`.
  - `wasm32-unknown-unknown` target + [`trunk`](https://trunkrs.dev/) — for `cargo xtask wasm` and browser builds in `apps/lumencraft-web`.
  - `resvg` (or `rsvg-convert`) + `iconutil` (macOS) — only when regenerating `assets/app-icon/` via `packaging/icons.sh`.

## Everyday commands

| Task | Command |
|---|---|
| Run all CI gates locally | `cargo xtask ci` |
| Run unit, integration & doctests | `cargo xtask test` |
| Verify crate dependency layers (`L0`–`L7`) | `cargo xtask layers` |
| Verify asset attributions in `ASSETS.md` | `cargo xtask assets` |
| Verify no competitor trademarks in UI/packaging | `cargo xtask brands` |
| Verify clean-room fixture/header rules | `cargo xtask cleanroom` |
| Verify tool cursor catalog sync (`cursors.toml`) | `cargo xtask cursors` |
| Check translation coverage | `cargo xtask i18n-report` |
| Sync desktop UI commands to MCP catalog | `cargo xtask ui-commands` |
| Print per-crate LOC and test statistics | `cargo xtask stats` |
| Build macOS `dist/LumenCraft.app` bundle | `cargo xtask bundle` |

## Subsystem implementation guides

Before adding or modifying behaviour in any domain crate, read its governing specification:

- [`architecture.md`](architecture.md) — `L0`–`L7` crate graph and dependency boundaries
- [`data-tables-and-transforms.md`](data-tables-and-transforms.md) — 8 data-table archetypes, subcolumns, exclusions, Info constants, Calculated Variables (`=`), and derived transforms (`crates/doc`, `crates/transform`, `crates/engine`)
- [`statistics-implementation.md`](statistics-implementation.md) — tail-safe special functions, *p*-value formatting, *t*-tests, 1/2/3-way & Multifactor ANOVA, REML mixed models, multiplicity/FDR, contingency, survival, ROC, GLMs & PCA (`crates/math`, `crates/stats`)
- [`curve-fitting-implementation.md`](curve-fitting-implementation.md) — Moré Levenberg–Marquardt, weighting, Cauchy robust & ROUT fitting, shared global parameters, Venzon–Moolgavkar profile-likelihood CIs, AICc/*F*-tests, built-in equations & custom equation VM (`crates/fit`)
- [`graphing-and-layouts.md`](graphing-and-layouts.md) — `GraphSpec -> ChartScene` compiler, nonlinear/discontinuous axes, live pairwise *p*-value brackets, style cloning, Page Layouts & SVG/PDF/PNG/TIFF export (`crates/chart`, `crates/text`, `crates/palette`, `crates/render`, `crates/svg`, `crates/pdf`)
- [`scripting-and-automation.md`](scripting-and-automation.md), [`control-protocol.md`](control-protocol.md), [`mcp.md`](mcp.md) — `.lctemplate` batch workflows, `lumencraft-cli`, `--control` TCP JSONL, and `lumencraft-cli mcp`

## Fonts

Font binaries never live inside this repository ([`AGENTS.md`](../AGENTS.md) › Assets).

- Set `LUMENCRAFT_FONTS_DIR` to an absolute directory of SIL OFL font faces when building if you want embedded fonts staged via `crates/text/build.rs` into `lumencraft_text::BUNDLED_FONTS` and copied into `dist/web/fonts/` via `cargo xtask web-fonts`.
- **Leaving out the bundled fonts:** `lumencraft-text` and `lumencraft-ui-egui` gate embedded font bytes behind the `bundled-fonts` Cargo feature. Embedders that supply their own `egui` font definitions can depend on `lumencraft-ui-egui` with `default-features = false` and call `lumencraft_ui_egui::theme::install_fonts`. `cargo xtask ci` explicitly tests both `--all-features` and `--no-default-features`.

## Web build

`apps/lumencraft-web` compiles the exact same `lumencraft-ui-egui` + `lumencraft-engine` workspace to `wasm32-unknown-unknown` (Rust only; zero handwritten JS/TS):

```sh
rustup target add wasm32-unknown-unknown
cd apps/lumencraft-web
trunk build --release   # writes ../../dist/web
trunk serve --release   # serves on http://127.0.0.1:8767
```

## Logs & environment variables (desktop app)

| Variable | Effect |
|---|---|
| `LUMENCRAFT_CONTROL_PORT` | Equivalent to `--control <port>` (`1..=65535`, binds strictly to `127.0.0.1`) |
| `LUMENCRAFT_NO_PREFS` | Disables reading/writing user preferences and disk logs (for isolated CLI/agent test runs) |
| `LUMENCRAFT_CONFIG_DIR` | Overrides the configuration and recovery directory for `lumencraft` and `lumencraft-cli` |
| `LUMENCRAFT_FONTS_DIR` | Optional path to external SIL OFL fonts directory at build time |
| `WGPU_POWER_PREF` / `WGPU_BACKEND` / `WGPU_ADAPTER_NAME` | Graphics adapter selection overrides (`low`, `high`, `metal`, `vulkan`, `dx12`, `gl`) |
| `RUST_LOG` | Standard `env_logger`-style log filter (`info` default for `lumencraft*`, `warn` for dependencies) |

## Cursors

`crates/tools/src/cursors.toml` documents every `Cursor` enum variant in plain behavioural language with its hotspot coordinates (`[x, y]`). `cargo xtask cursors` (run by `cargo xtask ci`) verifies bidirectional 1-to-1 correspondence between `crates/tools/src/lib.rs` and `cursors.toml`.

## Vendor names & clean-room gates

- `cargo xtask brands` fails on competitor vendor or product trademarks in Rust string literals (outside comments and `#[cfg(test)]`), `Cargo.toml` descriptions, `cursors.toml`, and `packaging/` manifests. Use neutral scientific terms ("nonlinear regression", "grouped table", "reference application"); mark genuine file-format compatibility notes with `brand-ok`.
- `cargo xtask cleanroom` scans all tracked files for vendor copyright notices and vendor-authored XML/project signatures. Never copy text, equation prose, screenshots, or sample `.prism`/`.pzfx` files from proprietary products, and never copy GPL/LGPL/AGPL statistical code.

## Tests & numerical verification

- `cargo xtask test` runs workspace unit and integration tests (`cargo nextest run` when installed, else `cargo test --workspace --lib --bins --tests`) followed by workspace doctests (`cargo test --workspace --doc`).
- Every statistical test (`crates/stats`) and regression model (`crates/fit`) must carry analytical unit tests, certified reference-dataset tests (NIST StRD) with explicit relative/ULP tolerances in `lumencraft-testkit`, and property tests covering degenerate inputs.

## Robustness: LumenCraft never crashes

A crash destroys a researcher's unsaved experimental data tables, exclusions, and analyses. Everything the application reads — `.lumencraft`, `.prism`, `.pzfx`, CSV/TSV files, clipboard text, user-entered equations and column formulas, MCP requests, `--control` messages, and command JSON parameters — is untrusted input. Shipped code therefore never panics and contains zero `unsafe` (`#![forbid(unsafe_code)]`).

### Enforced by lints

`Cargo.toml` denies these Clippy lints across the entire workspace (`cargo xtask ci` runs `cargo clippy --workspace --all-targets -- -D warnings`):

| Banned in shipped code | Use instead |
|---|---|
| `x.unwrap()`, `x.expect("…")` | `x?`, `x.ok_or(err)?`, `let Some(v) = x else { return … }`, `unwrap_or` / `unwrap_or_default` |
| `v.last().unwrap()`, `v.pop().unwrap()` | `if let Some(last) = v.last()`, `split_last()` |
| `panic!`, `unreachable!()` | Return a typed `Result::Err` (`StatsError`, `FitError`, `FormatError`, `EngineError`) or safe fallback |
| `todo!`, `unimplemented!` | Return an explicit `Unsupported` error variant; never merge panicking stubs |

`clippy.toml` (`allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`, `allow-panic-in-tests = true`) permits unwrapping inside `#[test]` and `#[cfg(test)]`. Integration test binaries (`tests/*.rs`) and `crates/testkit` may add `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.

### Not caught by lints: avoid implicit panics too

- **Indexing and slicing:** `v[i]` and `&s[a..b]` panic out of range or off UTF-8 char boundaries. Whenever row, column, subcolumn, or byte indices originate from files, selections, or parameters, use `.get(i)` / `.get_mut(i)`.
- **Floating-point & integer edge cases:** Check `f64::is_finite()` before sorting (`total_cmp` or filtered finite slices — never `partial_cmp().unwrap()`) or casting with `as`; guard integer division/modulo against zero and unsigned subtraction with `saturating_sub` or `checked_sub`.
- **Singular & non-convergent math:** Rank-deficient design matrices, zero degrees of freedom, negative log arguments, and non-convergent Levenberg–Marquardt runs are normal scientific outcomes (`FitStatus::Ambiguous`, `FitStatus::NotConverged`, `CiBound::Open`), never panics.
- **Bounded allocations & iterations:** Enforce explicit caps on archive decompression bytes, XML/AST nesting depth (`<= 64`), table row/column counts, and optimizer/ODE step counts.

### The safety net

`lumencraft_engine::guard::catch_panic` wraps top-level native entry points (`Session::execute`, tool input handlers, `--control` requests, and MCP requests) to roll back partial mutations and return `EngineError::Internal` if an unexpected bug ever unwinds. On `wasm32`, panics abort unconditionally, so `Result` discipline remains mandatory everywhere.

### Fuzzing

Untrusted boundaries carry `proptest` / fuzz suites that must never panic:
- `crates/format/tests/prop_format.rs` — lossless `.lumencraft` round-trips plus truncated/mutated ZIP, CSV, XML, and JSON inputs.
- `crates/fit/tests/dsl_fuzz.rs` — arbitrary/hostile user equation and formula strings.
- `crates/engine/tests/command_sweep.rs` — every registered command invoked with arbitrary JSON payloads.
- `crates/mcp/tests/protocol_props.rs` — malformed JSON-RPC frames.

## Verified NIST StRD corpus

Public-domain [NIST Statistical Reference Datasets (StRD)](https://www.itl.nist.gov/div898/strd/) ASCII files (summary statistics, linear ANOVA, linear regression, and all 27 nonlinear least-squares benchmarks) are fetched on demand into the gitignored `corpus/nist-strd/` directory and verified against pinned SHA-256 digests and exact byte sizes before running extended accuracy suites. Small analytical and canonical regression tables are generated directly in `lumencraft-testkit` so offline `cargo xtask ci` requires zero network access.
