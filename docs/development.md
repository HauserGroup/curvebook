# Development

> **Last updated:** 2026-11-15 · **Change:** corrected web build status (`R8.4` landed), environment variables, font feature flags, `cargo xtask cursors` behaviour, and the panic-guard entry point; added `cargo xtask coverage` (`R0.16`).

## Prerequisites

- **Rust 1.99.1+** (`edition = "2024"`, `resolver = "3"`), pinned in [`rust-toolchain.toml`](../rust-toolchain.toml) (`profile = "minimal"`, components `rustfmt`, `clippy`).
  - Run `rustc --version` and `cargo --version` (and `rustup show active-toolchain`) inside the repository to confirm rustup's proxy selects `1.99.1` (or newer) for both the compiler and Cargo.
  - **macOS Homebrew note:** If both Homebrew `rust` and `rustup` are installed, `/opt/homebrew/bin/{cargo,rustc}` link directly to Homebrew's formula and bypass `rust-toolchain.toml` and `+<toolchain>` selection. Prepend `rustup`'s proxy bin directory (`export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`) ahead of `/opt/homebrew/bin` so `rust-toolchain.toml` takes effect.
- Optional tools:
  - [`cargo-nextest`](https://nexte.st/) — used automatically by `cargo xtask test` when installed; falls back to `cargo test`.
  - `wasm32-unknown-unknown` target — for `cargo xtask wasm`; plus [`trunk`](https://trunkrs.dev/) for browser builds in `apps/curvebook-web`.
  - [`actionlint`](https://github.com/rhysd/actionlint) + [`shellcheck`](https://www.shellcheck.net/) — for `cargo xtask workflows`.
  - [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) (`cargo install --locked cargo-llvm-cov`) + the `llvm-tools-preview` rustup component — for `cargo xtask coverage` ([Coverage reports](#coverage-reports)).
  - `resvg`, `rsvg-convert`, or ImageMagick (`magick`), plus `iconutil` (macOS) — only when regenerating `assets/app-icon/` via `packaging/icons.sh`.

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
| Verify offline corpus & NIST StRD suites (`--fetch` to download) | `cargo xtask corpus` |
| Print per-crate LOC and test statistics | `cargo xtask stats` |
| Write a local source-coverage report (lcov + HTML) | `cargo xtask coverage [-p <crate>]` |
| Build macOS `dist/CurveBook.app` bundle | `cargo xtask bundle` |
| List every `xtask` subcommand (secrets, licences, telemetry, generated docs, …) | `cargo xtask --help` (see also [AGENTS.md](../AGENTS.md) §4) |

## Subsystem implementation guides

Before adding or modifying behaviour in any domain crate, read its governing specification:

- [`architecture.md`](architecture.md) — `L0`–`L7` crate graph and dependency boundaries
- [`data-tables-and-transforms.md`](data-tables-and-transforms.md) — 8 data-table archetypes, subcolumns, exclusions, Info constants, Calculated Variables (`=`), and derived transforms (`crates/doc`, `crates/transform`, `crates/engine`)
- [`statistics-implementation.md`](statistics-implementation.md) — tail-safe special functions, *p*-value formatting, *t*-tests, 1/2/3-way & Multifactor ANOVA, REML mixed models, multiplicity/FDR, contingency, survival, ROC, GLMs & PCA (`crates/math`, `crates/stats`)
- [`curve-fitting-implementation.md`](curve-fitting-implementation.md) — Moré Levenberg–Marquardt, weighting, Cauchy robust & ROUT fitting, shared global parameters, Venzon–Moolgavkar profile-likelihood CIs, AICc/*F*-tests, built-in equations & custom equation VM (`crates/fit`)
- [`graphing-and-layouts.md`](graphing-and-layouts.md) — `GraphSpec -> ChartScene` compiler, nonlinear/discontinuous axes, live pairwise *p*-value brackets, style cloning, Page Layouts & SVG/PDF/PNG/TIFF export (`crates/chart`, `crates/text`, `crates/palette`, `crates/render`, `crates/svg`, `crates/pdf`)
- [`scripting-and-automation.md`](scripting-and-automation.md), [`control-protocol.md`](control-protocol.md), [`mcp.md`](mcp.md) — `.cbtemplate` batch workflows, `curvebook-cli`, `--control` TCP JSONL, and `curvebook-cli mcp`

## Fonts

Font binaries never live inside this repository ([`AGENTS.md`](../AGENTS.md) › Assets).

- Set `CURVEBOOK_FONTS_DIR` to an absolute directory of SIL OFL font faces when building if you want embedded fonts staged via `crates/text/build.rs` into `curvebook_text::BUNDLED_FONTS` and copied into `dist/web/fonts/` via `cargo xtask web-fonts`. At runtime, pass `--ttf-font <path>` (to `curvebook` or to `curvebook-cli convert` / `export`) when you want an external `.ttf` file embedded as a TrueType `/FontFile2` font stream in `%PDF-1.7` exports; `CURVEBOOK_FONTS_DIR` itself is read only at build time (`crates/text/build.rs`, `cargo xtask web-fonts`). Set `CURVEBOOK_FONTS_REQUIRED=1` to make the build fail when `CURVEBOOK_FONTS_DIR` is unset.
- **Leaving out the bundled fonts:** `curvebook-text`, `curvebook-engine`, and `curvebook-ui-egui` gate embedded font bytes behind the default-on `bundled-fonts` Cargo feature. Embedders can depend on them with `default-features = false`; `curvebook_ui_egui::theme::install_fonts()` then reports `0` bundled faces. `cargo xtask ci` tests `curvebook-text` and `curvebook-ui-egui` and checks `curvebook-mcp` with `--no-default-features`.

## Web build

`apps/curvebook-web` shares the same pure-Rust `UiApp` + `curvebook-engine` codebase and presents the compositor's RGBA frame on a WebGL2 `<canvas>` (`R8.4`; `init_web_app()` tests also run on the host) using the `window.CurveBookHost` bridge (`curvebook.v1.*` storage namespaces). `cargo xtask wasm` type-checks `curvebook-web` and `curvebook-engine` for `wasm32-unknown-unknown`; `./packaging/web/package.sh` produces the release zip. Web menus and toolbar are not built yet (`R10.7`):

```sh
rustup target add wasm32-unknown-unknown
cd apps/curvebook-web
trunk build --release   # writes ../../dist/web
trunk serve --release   # serves on http://127.0.0.1:8767
```

## Logs & environment variables (desktop app)

| Variable | Effect |
|---|---|
| `CURVEBOOK_CONTROL_PORT` | Equivalent to `--control <port>` (`1..=65535`, binds strictly to `127.0.0.1`) |
| `CURVEBOOK_CONTROL_TOKEN` | Bearer token for `--control` / `--control-port` authentication |
| `CURVEBOOK_CONFIG_DIR` | Override platform config/recovery directory (and `control-<port>.token` location) |
| `CURVEBOOK_NO_PREFS` | Skip reading/writing preferences and shared recovery directories when truthy (`1`/`true`/`yes`) |
| `CURVEBOOK_FONTS_DIR` | Optional path to external SIL OFL fonts directory at build time (and runtime `FontBook` / `SystemFontSet` override) |
| `CURVEBOOK_FONTS_REQUIRED` | Build-time: fail the `curvebook-text` build when `CURVEBOOK_FONTS_DIR` is unset |
| `CURVEBOOK_ALLOW_OS_GUI_DIALOG` | Allow print preview to launch the host OS dialog (otherwise the PDF spool is only staged) |
| `CURVEBOOK_REQUIRE_CORPUS` / `CURVEBOOK_CORPUS_DIR` | Tests: require the cached NIST StRD corpus / override its location (see below) |
| `WGPU_POWER_PREF` / `WGPU_BACKEND` | Graphics adapter overrides honoured by the third-party `wgpu` renderer used by `eframe` (not read by CurveBook code) |

To isolate configuration for test or agent runs, pass `--portable-dir <dir>` (or place a `portable.txt` next to the executable) so preferences, shortcuts, and the recovery journal live under `<dir>/curvebook-data`. CurveBook has no logging framework and reads no `RUST_LOG` filter.

## Cursors

`crates/tools/src/cursors.toml` documents every `Cursor` enum variant in plain behavioural language with its hotspot coordinates (`[x, y]`). `cargo xtask cursors` (run by `cargo xtask ci`) warns about any `Cursor` variant in `crates/tools/src/lib.rs` with no `[[cursor]]` entry in `cursors.toml`; it is warning-only and never fails CI.

## Vendor names & clean-room gates

- `cargo xtask brands` fails on competitor vendor or product trademarks in Rust string literals (outside comments and `#[cfg(test)]`), `Cargo.toml` descriptions, `cursors.toml`, and `packaging/` manifests. Use neutral scientific terms ("nonlinear regression", "grouped table", "reference application"); mark genuine file-format compatibility notes with `brand-ok`.
- The same command rejects the names of external application suites and their infrastructure (`SUITE_BRANDS` in `xtask/src/brands.rs`) in every file outside `xtask/`, and `brand-ok` does not exempt them. The single exception is the non-affiliation disclaimer in `README.md`: a README line carrying `brand-ok` may name the suite itself, but still may not reference its crates or domain.
- `cargo xtask cleanroom` scans all tracked files for vendor copyright notices and vendor-authored XML/project signatures. Never copy text, equation prose, screenshots, or sample `.prism`/`.pzfx` files from proprietary products, and never copy GPL/LGPL/AGPL statistical code.

## Tests & numerical verification

- `cargo xtask test` runs workspace unit and integration tests (`cargo nextest run` when installed, else `cargo test --workspace --lib --bins --tests`) followed by workspace doctests (`cargo test --workspace --doc`).
- Every statistical test (`crates/stats`) and regression model (`crates/fit`) must carry analytical unit tests, certified reference-dataset tests (NIST StRD) with explicit relative/ULP tolerances in `curvebook-testkit`, and property tests covering degenerate inputs.

### Coverage reports

`cargo xtask coverage` (`R0.16`) runs the tests once under [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) instrumentation (through `cargo-nextest` when installed) and writes, under `<target dir>/llvm-cov/`:

- `lcov.info` — for editor gutters and other lcov tools;
- `html/index.html` — browsable per-file line and region report;
- `summary.json` — workspace totals, also printed at the end of the run.

```sh
cargo xtask coverage                        # whole workspace (slow: rebuilds everything instrumented)
cargo xtask coverage -p curvebook-stats     # one crate's tests
cargo xtask coverage -p curvebook-fit -- solver   # forward a test filter to the runner
```

- `-p` limits which tests run; the report still includes every workspace crate those tests reach. `xtask` and `curvebook-testkit` are always left out.
- The instrumented build lives in `<target dir>/llvm-cov-target`, separate from normal builds, and follows `CARGO_TARGET_DIR`, so parallel agents stay isolated.
- `rustup` toolchains need `rustup component add llvm-tools-preview`. Toolchains without that component (for example Homebrew Rust) can set `LLVM_COV` and `LLVM_PROFDATA` to the `llvm-cov` / `llvm-profdata` of an LLVM with the same major version as `rustc -vV` reports.
- Doctests and the `wasm32` build are not measured (doctest coverage needs nightly). Inline `#[cfg(test)]` modules count towards the totals.
- [`.github/workflows/coverage.yml`](../.github/workflows/coverage.yml) runs the same command weekly and on demand, and keeps the report as a workflow artifact. It is not a pull-request gate.

**Policy.** Coverage is a diagnostic for finding untested edge-case and error paths: `NaN`/`±Inf`, empty or degenerate input, singular matrices, and malformed-file branches in `crates/format`. It is not evidence that a routine is correct; only the `D1` oracle checks are. There is no coverage threshold, coverage reports are never uploaded to third-party services (`cargo xtask telemetry`), and coverage percentages are never written into hand-maintained docs ([pitfall #2](pitfalls.md#2-hardcoded-headline-counts-going-stale)). Do not add tests whose only purpose is to execute lines.

## Robustness: CurveBook never crashes

A crash destroys a researcher's unsaved experimental data tables, exclusions, and analyses. Everything the application reads — `.curvebook`, `.prism`, `.pzfx`, CSV/TSV files, clipboard text, user-entered equations and column formulas, MCP requests, `--control` messages, and command JSON parameters — is untrusted input. Shipped code therefore never panics and contains zero `unsafe` (`#![forbid(unsafe_code)]`).

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
- **Singular & non-convergent math:** Rank-deficient design matrices, zero degrees of freedom, negative log arguments, and non-convergent Levenberg–Marquardt runs are normal scientific outcomes (`FitStatus::Ambiguous`, `FitStatus::HitMaxIters`, `CiBound::Open`), never panics.
- **Bounded allocations & iterations:** Enforce explicit caps on archive decompression bytes, XML/AST nesting depth (`<= 64`), table row/column counts, and optimizer/ODE step counts.

### The safety net

`curvebook_engine::guard::run_guarded` wraps command handlers inside `Session::execute` (and therefore GUI, CLI, and `--control` commands) and MCP `tools/call` dispatch (`crates/mcp/src/tools.rs`) to roll back partial mutations and return an `EngineError` with `ErrorCode::InternalPanic` if an unexpected bug ever unwinds. On `wasm32`, panics abort unconditionally, so `Result` discipline remains mandatory everywhere.

### Fuzzing

Untrusted boundaries carry `proptest` / fuzz suites that must never panic:
- `crates/format/tests/prop_format.rs` — lossless `.curvebook` round-trips plus truncated/mutated ZIP, CSV, XML, and JSON inputs.
- `crates/fit/tests/dsl_fuzz_props.rs` — arbitrary/hostile user equation and formula strings.
- `crates/engine/tests/command_sweep_props.rs` — every registered command invoked with arbitrary JSON payloads.
- `crates/mcp/tests/protocol_props.rs` — malformed JSON-RPC frames.

## Verified NIST StRD corpus

Public-domain [NIST Statistical Reference Datasets (StRD)](https://www.itl.nist.gov/div898/strd/) ASCII `.dat` files across all **58 canonical datasets** (`9` Univariate, `11` ANOVA, `11` Linear Regression, and `27` Nonlinear Regression) are fetched on demand into the gitignored `corpus/nist-strd/` directory (`univ/`, `anova/`, `lls/`, `nls/`) and verified against `sha256sums.txt` (SHA-256 digest + exact byte size) and clean-room `.dat` structural parsers in `curvebook-testkit`:

- `cargo xtask corpus` runs strictly offline (zero network I/O): if `corpus/nist-strd/` is present it verifies every cached `.dat` file and manifest entry, then runs `curvebook-format`, `curvebook-testkit`, and `curvebook-math --test nist_strd` with `CURVEBOOK_REQUIRE_CORPUS=1` so corpus-backed checks enforce the cached dataset set when populated; when `corpus/nist-strd/` is absent, it runs the offline test suites directly.
- `cargo xtask corpus --fetch` (or `--fetch-only` to skip running tests afterwards) downloads all 58 NIST StRD `.dat` files into `corpus/nist-strd/`, writes `corpus/nist-strd/sha256sums.txt`, and verifies the downloaded corpus with `CURVEBOOK_REQUIRE_CORPUS=1`.
- Small analytical and canonical stiffness benchmarks (`NumAcc1`–`NumAcc4`, `BinaryStiff1e8`, `Wampler1`–`2`, `WamplerOrtho1`–`3`, `NoInt1`–`2`, `FilipScaleVandermonde`, `SmLs` ANOVA) are generated directly in `curvebook-testkit` so offline `cargo xtask ci` requires zero network access.
