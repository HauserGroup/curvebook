# LumenCraft Roadmap

Claimable engineering roadmap for **LumenCraft**. For current verified capabilities, test evidence, and known limits, see [`STATUS.md`](STATUS.md). For historical wave-by-wave progress logs, see [`roadmap-history.md`](roadmap-history.md).

**Next immediate item:** [R0.18 — Upgrade the workspace and build toolchains to Rust 1.99.1](#r018-upgrade-the-workspace-and-build-toolchains-to-rust-1991). This is the next item to start; existing claims remain intact. The upgrade is planned, not implemented.

**Next feature priority:** [R7.3 — Fully functional MCP for existing engine capabilities](#r73-mcp-workflow-parity-proof--shared-live-window) follows `R0.18` and precedes new `R10` GUI implementation. Prove actual stdio/loopback workflows and a shared live session before investing in the human-operable GUI; preserve existing claims. Follow verified MCP implementation with [R7.4 — Accessible MCP guide and minimal README](#r74-accessible-mcp-guide-and-minimal-readme) before the GUI handoff.

---

## How to use this file

1. **Pick the next item** from [Order and dependencies](#order-and-dependencies) whose dependencies are `done` and whose gate matches the current release target.
2. **Claim before branching:** mark the item `Status: in progress — claimed: <handle>, branch <branch-name>` (and later `— in PR #n`) in a small claim commit or PR, and read [`AGENTS.md`](../AGENTS.md), [`pitfalls.md`](pitfalls.md), and [`extending.md`](extending.md) before writing code.
3. **One item per PR:** keep pull requests scoped to a single `R<m>.<n>` item, use branch prefixes `feature/`, `bugfix/`, `hotfix/`, `refactor/`, `docs/`, `test/`, `chore/`, or `audit/` with the roadmap ID, and end squash PR titles with `(R<m>.<n>)`.
4. **Metadata & checkboxes:**
   - Every item header has the form `### R<m>.<n> <title>` followed by `Owner: maintainers|open · Status: todo|in progress|blocked (why)|done · Was: <old IDs / notes>`.
   - `- [ ]` lines track concrete pending sub-tasks (surfaced by `cargo xtask pending` under `R0.4`). Mark them `- [x]` only after verification passes.
   - Hour estimates are single-agent implementation estimates (notes only, excluding human review).
5. **Unique table keys:** every Markdown table in `docs/ROADMAP.md` and `docs/STATUS.md` MUST have unique values in Column 1 (`cargo xtask status` enforces this to catch silent merge-conflict duplicates).

---

## Architecture rules for new work

- **Headless first:** domain logic (`L0`–`L5`) never depends on UI toolkits or OS windowing; UI panels (`L6`) act strictly through engine commands (`crates/engine`) or documented `UI_COMMANDS`.
- **Extend through seams:** register commands, formats (`Importer`/`Exporter`), fit equations (`ParametricModel`), and error codes through typed tables and traits rather than closed string `match` chains.
- **Data over code:** parameter schemas, command descriptors, shortcut metadata, and cursor tables live in declarative structs/TOML that generate docs and schemas rather than hand-synced prose counts.
- **Structured results:** propagate typed errors (`EngineError` / `ErrorCode`) and typed assumption diagnostics (`ChecklistItem::remediation`) — never silently coerce invalid domains or keyword-match UI strings.
- **Versioned files & safe I/O:** persist structured files with explicit format versions and forward-upgrade steps (`versioned.rs`), refuse to overwrite newer file versions, and route external subprocesses and atomic writes through `lumencraft-platform` (`L0`).

---

## Milestones at a glance

| Milestone | Scope & target gate | Was | Status | Notes |
|---|---|---|---|---|
| **R0** | Align with house principles & typed engine scaffold | Phases A–E | in progress | **Next immediate: `R0.18` (Rust 1.99.1 minimum and pinned toolchains), todo.** `R0.1`–`R0.5`, `R0.7`–`R0.13` done (Waves 8A–14); `R0.15` (evidence-backed done markers) todo; `R0.16` (local `cargo-llvm-cov` coverage reports) in progress; `R0.17` (clean-room provenance review of Lilliefors and Bessel code) todo; `R0.6` (Zero-Telemetry policy, local `xtask telemetry`, and local `xtask workflows` `actionlint` + `shellcheck` done; live GH Actions runner execution open), `R0.14` (clean-room PNG screenshots, `.github/workflows/release.yml` `tags: ["v*"]`, `cargo-about` `THIRD-PARTY-LICENSES.html` + `cargo-deny`, web `.zip`, and local universal macOS `.dmg`/`.zip` `package.sh` + `verify.sh` done; hosted `v0.1.0-rc1` CI tag/matrix open) in progress |
| **R1** | Workspace scaffold, numerical foundations & NIST StRD oracle | `M0`, `M1` (partial), `G1` | done | `58` NIST StRD datasets + `D1` R `4.5.2` oracle matrices verified (`0 h`) |
| **R2** | Typed data tables, reactive DAG, persistence & `.prism`/`.pzfx` import | `M1`, `M2`, `G2`, `G10` | done | Waves 1–6 incl. `.lumencraft.d/`, `G2` & `G10` fixes verified (`0 h`) |
| **R3** | Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate | `M2`, `M6`, `G3`, `G8` | in progress | `R3.4` (unstructured REML covariance) and `R3.5` (statistical method fidelity) todo; `R3.1`–`R3.3` done: Waves 2–5 incl. `BarnardExact`, `PassingBablok`, MICE, `G8` Koopman fix |
| **R4** | Nonlinear regression solver, `117` equations & custom DSL/ODE | `M3`, `M6`, `G4`, `G9` | in progress | `R4.5` (robust-mode inference suppression) and `R4.6` (fitting method fidelity) todo; `R4.1`–`R4.4` done: Waves 2–5 incl. `47` pharma models, `MAX_AST_NODES`, iterative `Expr` |
| **R5** | Publication graphing, vector SVG/PDF 1.7/TIFF/EMF & Page Layouts | `M4`, `M6`, `S2`, `G5`, `G11` | done | `R5.1`–`R5.4` & `G5` done (Waves 2–13): Poppler+Ghostscript PDF QA, live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux vector clipboard adapters, `ActiveDialog::Print` + OS printer discovery/preview, and live WYSIWYG layout drag-snap/resize grips |
| **R6** | Multiple Variables workflows, Calculated Variables & Multifactor ANOVA | `M6`, `G7` | done | Waves 2–4 end-to-end verified (`0 h`) |
| **R7** | Automation, headless CLI, `--control`, MCP & performance budgets | `M5` (headless), `M6`, `Slice 0`, `G12`, `G14` | in progress | **Priority after `R0.18`, before `R10` GUI: `R7.3` (MCP workflow parity proof & shared live window), todo**; `R7.4` (accessible MCP guide and minimal README) follows verified `R7.3`; `R7.1`–`R7.2` done: `95` engine commands (`90` Wave 11–14 + Wave 15 `sheet.extractResultTab` & `graph.formatSelectedPoints` + Wave 22 `R9.14` `table.randomizeGroups`/`randomSampleRows`/`simulateDistributions`), `96`-curve 4PL `< 50 ms`, `G12` read-only skip & bounded undo (`0 h`) |
| **R8** | Live desktop & WASM UI shell, GUI-agent parity, localization QA & `1.0` hardening | `M5` (UI), `M7`, `S1`, `G6`, `G13` | in progress | `UiApp`/HiDPI/RTL/`WebWorkspaceHost`, `eframe`/`egui`/`wgpu` native window **presenting the CPU RGBA buffer as a texture** (`R8.3`), `wasm32` Trunk WebGL2 **textured-quad presenter** (`R8.4`), universal macOS `.dmg`/`.zip`, `cjk_rtl_hidpi_desktop_qa`, & **semantic agent-harness** parity/Power wizard state machine/28-workflow suite (`R8.6`–`R8.8`) done; Windows/Linux/FreeBSD VM QA, Apple Developer ID notarization, & live browser QA (`R8.5`) open. **The rendered chrome is not human-operable; see `R10`.** |
| **R9** | Scientific quality-of-life scope | `M8+`, `QOL-1`–`QOL-17` | done (active scope) | Wave A `QOL-1`–`QOL-9` (incl. `R9.7` diagnostics-only banners) plus Wave B `R9.12` (table-analysis completions), `R9.14` (randomization, sampling & distribution simulation), `R9.16` (equations-only live $\TeX$ callouts), and `R9.17` (`WF-29`–`WF-49` Shards 5–7, `49` total agent-harness `gui_only` workflows) done (`0 h`); `R9.10`, `R9.11`, `R9.13`, and `R9.15` deferred without scheduled milestones ([scope](qol-beyond-parity.md)) |
| **R10** | Human-operable GUI & 50-workflow human completion (desktop + web) | Human-GUI audit 2026-11-15 | todo | Audit found **`0` of `50`** reference workflows completable by a person using the rendered GUI: modal dialogs are inert text boxes (`B1`), no reachable New Data Table picker (`B2`), result sheets truncated with unclickable tabs (`B3`), menu commands dispatched with `null` params and no web menu (`B4`), toolbar/inspector/formula bar/context menus/grid scrolling unwired (`B5`). Engine and dialog state machines exist for `48` of `50`; `R10.10` (stack of *P* values) and `R10.11` (Compact Letter Display) add the 2 missing features. Also covers external-branding exclusion, proportional typography, aspect-ratio canvas, contextual toolbar, live inspector, typeset equations, accessibility, translations, and performance budgets (`R10.14`–`R10.22`). Blocks **1.0**. |

**Gate summary:** **Alpha** (`R8.3` desktop window + `R8.4` `wasm32` bundle, both presenting a non-interactive RGBA chrome), **Agent-harness workflows** (formerly "Core workflows": `R8.6`–`R8.8` & `R9.17` semantic `gui_only` parity across 49 `gui_wf_*.rs` Shards 1–7), **Beta** (`R5.4`), local universal macOS `.dmg`/`.zip` + web `.zip` packaging (`R8.5`/`R0.14` local), and active **`R9` QoL** pass today. **Human GUI workflows** (`R10.1`–`R10.22`) is **open**: `0 / 50` reference workflows are completable by a person in the rendered desktop or web GUI. **1.0** requires the Human GUI workflows gate in addition to the external CI/notarization/hardware-VM items (`R8.5`, `R0.6`, `R0.14`).

> **What the `gui_only` suite does and does not prove.** The `49` `gui_wf_*` tests drive `UiApp` through `dispatch_gui_action` using virtual widget nodes from `inspect.rs`, JSON parameters passed through `ui.invoke_menu`, and field patches through `ui.answer_dialog`. Those nodes are not drawn and are not hit-tested in the window a person sees. The suite verifies engine and dialog **state machines** and the agent automation surface; it is **not** evidence of human GUI usability. Human usability is gated by `R10.2` (human-path harness) and `R10.13` (acceptance).

---

## R0 Align with house principles

### R0.1 Governance, agent & contributor docs
Owner: maintainers · Status: done · Was: Phase A1, A2, A4

- [x] Restructure `AGENTS.md` with workspace layer map, full `xtask` command list, claim/branch/squash-PR rules, architecture rules, and verification block (Wave 8A).
- [x] Update `CLAUDE.md` and create `docs/pitfalls.md` and `docs/extending.md` (Wave 8A).
- [x] Update `CONTRIBUTING.md`, `.github/pull_request_template.md`, and `.github/ISSUE_TEMPLATE/roadmap_task.yml` for `R<m>.<n>` IDs (Wave 8A).

### R0.2 Claimable roadmap & status conversion
Owner: maintainers · Status: done · Was: Phase A3

- [x] Consolidate active planning into `docs/ROADMAP.md` (`R0`–`R9` claimable format + Legacy Milestone & Gate Mapping), create `docs/STATUS.md` and `docs/roadmap-history.md`, and keep a single case-safe roadmap file on disk (Wave 8A).
- [x] Shrink root `ROADMAP.md` to a 5-line pointer and convert `docs/gaps.md` to a `G1`–`G14` -> `R` mapping stub keeping `docs/gaps.md#known-defects` alive for Round 1 audit (Wave 8A).

### R0.3 Secrets gate & pre-commit hook
Owner: open · Status: done · Was: Phase B1

- [x] Add `.githooks/pre-commit` (`exec cargo xtask secrets`) and update `.gitignore` for `.env*`, `*.pem`, `*.key`, `*.p12`, and `.claude/settings.local.json` (Wave 8A).
- [x] Implement `xtask/src/secrets.rs` (`cargo xtask secrets [--history]`) with unit tests on synthetic keys/paths and wire into `cargo xtask ci` (Wave 8B).

### R0.4 Roadmap pending & status tooling
Owner: open · Status: done · Was: Phase B2

- [x] Implement `cargo xtask pending [--count]` listing open `- [ ]` lines from `docs/ROADMAP.md` (Wave 8B).
- [x] Implement `cargo xtask status` failing on duplicate Column 1 rows in any table in `docs/ROADMAP.md` or `docs/STATUS.md`, and add both to `cargo xtask ci` (Wave 8B).

### R0.5 Licences & supply-chain policy
Owner: maintainers · Status: done · Was: Phase B3 · Est. note: 0 h remaining

- [x] Add `deny.toml`, `about.toml`, `about.hbs`, and `docs/licences.md` aligned with the allowlist and add the `licenses` job generating `THIRD-PARTY-LICENSES.html` in `.github/workflows/release.yml` (Wave 8A).
- [x] Implement `cargo xtask licenses` (`cargo deny check licenses`, `--html` fallback) and wire it into `cmd_ci` and `.github/workflows/ci.yml` (`cargo install --locked cargo-deny && cargo deny check licenses`, Waves 8B & 13; offline workspace passes cleanly via `cargo xtask licenses` when `cargo-deny`/`cargo-about` binaries are absent locally).

### R0.6 Repository hygiene & CI cleanup
Owner: open · Status: in progress · Was: Phase B4

- [x] Add `.gitattributes`, workspace `publish = false` (`publish.workspace = true` across library crates), `unsafe_code = "forbid"`, `[profile.web-release]`, `clippy.toml` (`allow-indexing-slicing-in-tests`, `disallowed-types` for `std::sync::Mutex`), `.github/actionlint.yaml`, remove dead `.github/workflows/nix.yml`, wire `cargo xtask telemetry` (zero telemetry/egress/analytics + workflow `permissions` / `${{ }}` hygiene) and `cargo xtask workflows` (`actionlint 1.7.12` across `5` workflows + `shellcheck -x` across `14` shell scripts, `0` warnings) into `cmd_ci`, and invoke `actionlint` + `shellcheck` in `.github/workflows/ci.yml` (Waves 8A, 13–14, 18). **Decision resolved (Zero-Telemetry policy):** SonarCloud and all external cloud scanners/telemetry services are permanently excluded.
- [ ] Verify CI workflows (`cargo xtask ci` including `xtask workflows`) pass cleanly on live GitHub Actions runners (local `actionlint 1.7.12` + `shellcheck 0.11.0` pass). **Hosted `CI` currently fails on `main`** (every push since 2026-11-14, e.g. at `fda1cc0`): the `actionlint` install step in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) passes `$RUNNER_TEMP/actionlint-bin` to `download-actionlint.bash` before creating that directory, so the script exits with its usage text and no Rust gate runs. Fix the step (create the directory and pin a specific version), then confirm a green hosted run.

### R0.7 Generated command & schema docs
Owner: open · Status: done · Was: Phase B5

- [x] Author hand-written `docs/command-ids.md` and `docs/file-formats.md` (Wave 8A).
- [x] Implement `cargo xtask commands [--check]` generating `docs/commands.md` from `R0.8`'s typed registry and wire `--check` into `cargo xtask ci` (Wave 11).
- [x] Implement `cargo xtask schemas [--check]` generating `docs/schemas/*.schema.json` (archive manifest, document, batch v1, preferences, journal record) after `R0.11` and wire `--check` into `cargo xtask ci` (Wave 11).

### R0.8 Typed `CommandSpec` registry
Owner: open · Status: done · Was: Phase C1

- [x] Replace `CommandDescriptor`, `catalog::COMMANDS` (`90` entries), and string `match` dispatch in `cmd_data.rs` / `cmd_sci.rs` with `crates/engine/src/cmd/mod.rs` (`CommandSpec`, `CommandGroup`, `Effects`, `Ctx`, `Registry`), migrating area-by-area with `serde_path_to_error` typed params and preserving WAL / `lumencraft.batch.v1` compatibility (Waves 9–11; expanded to `92` entries in Wave 15 with `sheet.extractResultTab` & `graph.formatSelectedPoints`, and `95` entries in Wave 22 with `table.randomizeGroups`, `table.randomSampleRows` & `table.simulateDistributions`).
- [x] Convert `UI_COMMANDS` (`29` entries in `crates/ui-egui/src/menus/ui_commands.rs`) to a typed `UiCommandSpec` struct and regenerate `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (Wave 9; expanded to `32` entries through Wave 16 and `45` entries through Waves 20–22).

### R0.9 Structured `EngineError` & `ChecklistItem` remediation
Owner: open · Status: done · Was: Phase C2

- [x] Upgrade `EngineError` to `{ code: ErrorCode, category, message, retryable, remediation, details }` with deterministic `From` mappings from domain errors, structured CLI `--json` errors (exit code `130` on interrupt), MCP `isError` structured payloads, and additive `error_detail` on `--control` (Wave 9).
- [x] Add `remediation: Option<RemediationKind>` to `ChecklistItem` (`crates/doc/src/result.rs`) and replace `ResultStatus::Failed { code: String }` with `ErrorCode`, updating `ui-egui` remediation banners to read the typed field directly (Wave 9).

### R0.10 `lumencraft-platform` safe process & atomic writes
Owner: open · Status: done · Was: Phase C3

- [x] Create `crates/platform` (`L0`, `#![forbid(unsafe_code)]`, wasm-safe) with `write_atomic` / `write_atomic_with_hook`, deadline-bounded `ProcessCommand`/`ProcessOutput`, and shared `WorkdirGuard` / `PathResolver` (`11 passed, 0 failed`), and wire `lumencraft-platform` dependencies into `format`, `engine`, `ui-egui`, `mcp`, `lumencraft-cli`, and `lumencraft` (Wave 8A).
- [x] Register `platform` in `xtask/src/layers.rs`, add source checks banning raw `std::process::Command` and unguarded UI file writes outside `platform`, and migrate `format`, `engine`, `ui-egui`, `mcp`, and `lumencraft-cli` callers onto `lumencraft-platform` (Wave 8B).

### R0.11 Versioned config, journal & lock files
Owner: open · Status: done · Was: Phase C4

- [x] Add `crates/format/src/versioned.rs` generic `upgrade(doc, from, first, steps)` with compile-time step count assertion and apply it to project documents, `preferences.json` (refusing to save over newer versions), WAL `ops.journal`, and lock files (Wave 9).

### R0.12 Format `Importer` / `Exporter` registry
Owner: open · Status: done · Was: Phase C5

- [x] Extract an `Importer` / `Exporter` trait and registry in `crates/format` and replace duplicated extension/format `match` blocks across `crates/mcp`, `apps/lumencraft-cli`, and `apps/lumencraft` (Wave 9).

### R0.13 MCP, CLI & UI alignment over the typed registry
Owner: open · Status: done · Was: Phase D1, D2, D3

- [x] Update `crates/mcp` to derive tool annotations (`readOnlyHint`, `destructiveHint`) from `Effects`, expose `CommandGroup` progressive toolsets (`list_toolsets`/`enable_toolset`), `sheet://<id>`/`result://<id>` resources, workflow prompts, `WorkdirGuard` path containment, and `run_guarded` wrapping (Wave 10).
- [x] Add `lumencraft-cli commands`, typed schema flags (`lumencraft-cli <command-id> --flag ...`), and `mcp --control-port <p>` (Wave 10).
- [x] Wire `CommandSpec` `summary`/`beginner` strings into the UI command palette/tooltips, enforce i18n key coverage for command labels, and add a check verifying every `tr("...")` call exists in `i18n::CATALOG` (Wave 10).

### R0.14 Release verification scripts & README scaffold
Owner: open · Status: in progress · Was: Phase E

- [x] Expand `docs/releasing.md`, add `packaging/macos/verify.sh`, `packaging/linux/apprun-test.sh`, `packaging/windows/check-icons.ps1`, pass `--cargo-profile web-release` in `packaging/web/package.sh`, update `.github/workflows/release.yml` with `tags: ["v*"]` and safe `env:` version resolution, generate `THIRD-PARTY-LICENSES.html` (`cargo about generate` + `cargo deny check licenses`), and modernize `README.md` with badges, section nav, Downloads, `platform/`, Zero-Telemetry callout, and one-line Status.
- [x] Generate clean-room `1440x900` Light Split Workspace and Dark Multi-Panel Layout PNG screenshots (`crates/ui-egui/examples/doc_screenshots.rs`, `cargo xtask screenshots [--check]`, `docs/images/workspace-split-fit.png`, `docs/images/workspace-layout-dark.png`, `ASSETS.md`, `README.md`) and complete local universal macOS (`lumencraft-0.1.0-macos-universal.dmg`, `36,340,795` B; `lumencraft-cli-0.1.0-macos-universal.zip`, `13,504,838` B) & web (`lumencraft-web-0.1.0.zip`, `4,162,394` B) release packaging + `verify.sh --arch universal` + `check-app-specific.sh`.
- [ ] Tag `v0.1.0-rc1` across the 5-OS hosted GitHub Actions release matrix (`release.yml`) and verify hosted CI `cargo deny`/`cargo about` execution across all runners.

### R0.15 Evidence-backed "done" markers
Owner: maintainers · Status: todo · Was: Human-GUI audit 2026-11-15 (process fix) · Est. note: 4–8 h

- [ ] Add pitfall #6 to [`pitfalls.md`](pitfalls.md): the semantic agent harness was taken as proof of human GUI parity, so every UI item was marked done while `0 / 50` workflows were usable by a person.
- [ ] Add an `xtask` check (run by `cargo xtask status`) that fails when a `[x]` line in [`ui-parity.md`](ui-parity.md) or the human-path column of [`gui-workflows.md`](gui-workflows.md) does not name a passing human-path test or a recorded manual run.

### R0.16 Local source-coverage reports (`cargo-llvm-cov`)
Owner: open · Status: in progress — claimed: jsture, branch `chore/R0.16-llvm-cov` · Was: coverage tooling review 2026-11-15 · Est. note: 2–4 h

Coverage is a diagnostic for finding untested edge-case and error paths (`NaN`/`±Inf`, empty input, malformed files). It is never a substitute for `D1` oracle checks and has no percentage gate.

- [x] Add `cargo xtask coverage [-p <crate>]... [-- <test args>]` wrapping `cargo llvm-cov` (nextest when installed), excluding `xtask` and `lumencraft-testkit`, writing `lcov.info` and an HTML report under the coverage target directory, and printing an install hint when `cargo-llvm-cov` or `llvm-tools-preview` is missing (`xtask/src/coverage.rs`, `5` unit tests; local runs on `lumencraft-math` and `lumencraft-format`).
- [x] Add `.github/workflows/coverage.yml` (manual and weekly, not a PR gate) that runs `cargo xtask coverage` and keeps the report as a workflow artifact only; no third-party coverage upload service (local `cargo xtask workflows` + `cargo xtask telemetry` pass).
- [ ] Confirm a green hosted run of `coverage.yml` (`workflow_dispatch`) once it is on `main`.
- [x] Document the command and its policy (no thresholds, no hand-written coverage percentages) in [`development.md`](development.md) and [`AGENTS.md`](../AGENTS.md) §4.

### R0.17 Clean-room provenance review: Lilliefors p-values & Bessel functions
Owner: maintainers · Status: todo · Was: `D1` citation pass 2026-11-15 · Est. note: 6–12 h

The `D1` citation pass could not trace two pieces of numerical code to a freely usable published source.

- [ ] Lilliefors *p*-values in `crates/fit/src/diagnostics.rs`: the quartic polynomial coefficients for *p* > 0.1 and the `n > 100` rescaling `D·(n/100)^0.49` have no recorded published source and appear to match a GPL-licensed R package's own simulation fit. Establish provenance; if it cannot be traced to published literature (e.g. Dallal & Wilkinson 1986), replace it with a clean-room implementation from the paper and document error bounds.
- [ ] Bessel `J0`/`J1`/`Y0`/`Y1` coefficients and the `Jn` Miller recurrence in `crates/fit/src/dsl.rs` follow *Numerical Recipes* §6.5, whose code is not freely licensed. Re-derive from Abramowitz & Stegun / NIST DLMF, and either reach near-`f64` accuracy (currently about `1e-7`–`1e-8`) or document error bounds (`D1` rule 2). Also document error bounds for the `confluent_u` integer-`b` shift and `1F1` at large `|z|`.

---

### R0.18 Upgrade the workspace and build toolchains to Rust 1.99.1
Owner: maintainers · Status: todo · Priority: next immediate item · Depends on: none · Was: Rust toolchain upgrade decision 2026-11-15

Rust 1.99.1 is the minimum supported version and the exact local rustup/CI/packaging pin. Keep edition 2024, resolver 3, application version, dependencies, numerical tolerances, and runtime behavior unchanged. This item is not an implementation claim; the currently declared baseline remains Rust 1.95 until the upgrade lands. Existing work claims are preserved, and the CurveBook rename remains unscheduled.

- [ ] Set `[workspace.package] rust-version = "1.99.1"` and add `rust-version.workspace = true` to every member package that does not already inherit it, including libraries, CLI, testkit, and `xtask`.
- [ ] Add `rust-toolchain.toml` with `channel = "1.99.1"`, `profile = "minimal"`, and `components = ["rustfmt", "clippy"]`; keep additional targets job-specific and install matching `llvm-tools-preview` for coverage.
- [ ] Update CI, coverage, Windows ARM64, release, and helper-job toolchain pins; use rustup Rust 1.99.1 in both FreeBSD VM workflows instead of the rolling system Rust package. Include root manifests, lockfile, and toolchain-file changes in the affected FreeBSD/Windows ARM64 PR path filters.
- [ ] Update current prerequisites, README badge, contributor/governance instructions, and stale toolchain comments while preserving historical evidence. Document rustup proxy selection and checking the actual compiler/Cargo versions, including installations where Homebrew binaries take precedence on `PATH`.
- [ ] Resolve upgrade-related compiler, formatter, and Clippy failures narrowly, without unrelated refactors, dependency upgrades, or broader lint suppression; preserve the lockfile unless a compiler-required metadata-only adjustment is demonstrated.
- [ ] Confirm automatic Rust 1.99.1 selection from the root/member directories and every member's inherited minimum in `cargo metadata --locked --no-deps`; verify explicitly selected Rust 1.99.0 rejects a representative package. Run `cargo +1.99.1 xtask ci`, actual WASM/Trunk builds, coverage with matching LLVM tools, workflow linting, and supported-platform build/package smoke checks. Record only checks actually run and separate existing failures from upgrade regressions; do not tag, publish, or invoke release-publishing workflows for verification.

---

## R1 Workspace, numerical foundations & NIST StRD oracle

### R1.1 Layered workspace & clean-room quality gates
Owner: maintainers · Status: done · Was: M0

- [x] Initialize 21-crate layered Rust 2024 workspace (`L0`–`L7`), `xtask` CI/layer/asset/brand/cleanroom/cursor/i18n/ui-commands gates, original icons, and cross-OS packaging manifests (Wave 1).

### R1.2 IEEE-754 foundations, special functions, linear algebra & NIST StRD / `D1` corpus
Owner: maintainers · Status: done · Was: G1, M1 (partial) · Est. note: 0 h remaining

- [x] Land `lumencraft-math` (`51` tests incl. doctests) and `lumencraft-testkit` (`7` tests incl. doctest, `106`-bit `DoubleDouble`, FIPS 180-4 verifier, NIST `.dat` parsers) with `cargo xtask corpus` (`58` NIST StRD datasets) and `D1` R `4.5.2` oracle & edge matrices across `stats` and `fit` (Waves 1–4).

---

## R2 Typed data tables, reactive DAG, persistence & legacy import

### R2.1 Palettes, 8 typed table archetypes, subcolumns & dependency links
Owner: maintainers · Status: done · Was: G2 (partial), M1 · Est. note: 0 h remaining

- [x] Land `lumencraft-palette` (`15 passed`, sRGB/OkLab/OkLCH/CMYK, Okabe–Ito/Paul Tol/Viridis/Cividis/Plasma, Machado CVD) and `lumencraft-doc` (`31 passed`, `8` `TableArchetype`s, `8` `SubcolumnMode`s, non-destructive exclusions, `FloatingNote`, `InfoSheet`, `LinkedDataRegion`, `AnalysisSpec`/`ResultSheet`, `GraphSpec`/`LayoutSpec`, `CellExclusionNote`, `NamedCheckpoint`).

### R2.2 `.lumencraft` archive, WAL crash recovery & `.lumencraft.d/` unpacked format
Owner: maintainers · Status: done · Was: G2, QOL-11, M1 · Est. note: 0 h remaining

- [x] Land `lumencraft-format` ZIP/JSON/CSV archive I/O, `MIGRATION_STEPS`, `sanitize_circular_hooks`, `CrashRecoveryJournal` WAL + torn-tail replay (`DirectoryJournalBackend` & `MemoryJournalBackend`), and Wave 5 `.lumencraft.d/` unpacked directory I/O with path-containment and `exclusion_notes`/`checkpoints` fidelity (`51 passed`).

### R2.3 Reactive recalculation DAG, bounded undo/redo & linked-file error propagation
Owner: maintainers · Status: done · Was: G2, G12 (defect), M2 · Est. note: 0 h remaining

- [x] Land `lumencraft-engine` reactive topological DAG, `Paste Link`, atomic undo/redo, Wave 6 `G2` linked-file error propagation in `cmd_data.rs`, and Wave 6 `G12` read-only command clone-skip + bounded `undo_stack` (`MAX_UNDO_ENTRIES = 64`, `MAX_UNDO_CELL_BUDGET = 25_000_000`; `76 passed`).

### R2.4 Clean-room `.prism` & `.pzfx` project import with authoritative cell budget
Owner: maintainers · Status: done · Was: G10, M6 · Est. note: 0 h remaining

- [x] Land clean-room `.prism` & `.pzfx` importers with bounded XML/ZIP limits (`corrupt_archive_fuzz` `18`) and Wave 5 `G10` authoritative `allocated_cell_slots()` cell-budget enforcement (`MAX_IMPORT_CELLS = 10_000_000`, verified in `unpacked_and_pzfx_budget`).

---

## R3 Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate

### R3.1 Descriptive statistics, normality battery, outliers & two-group tests
Owner: maintainers · Status: done · Was: G3 (partial), M2 · Est. note: 0 h remaining

- [x] Land descriptive summaries, `NormalQq` tab/plot, $K^2$/Shapiro–Wilk/AD/KS normality & lognormality, Grubbs/ROUT outliers, unpaired/Welch/paired/ratio-paired/lognormal/one-sample *t*-tests + Cohen's $d$/Hedges' $g$/Glass's $\Delta$, `MultipleRowTTests` + FDR/Volcano, Mann–Whitney, Wilcoxon, and two-sample KS (Waves 2–4).

### R3.2 One-/two-/three-way ANOVA, RM, REML mixed models & multiplicity
Owner: maintainers · Status: done · Was: G3, M2 · Est. note: 0 h remaining

- [x] Land Brown–Forsythe/Bartlett/Welch ANOVA, Geisser–Greenhouse/Huynh–Feldt RM ANOVA, Kruskal–Wallis, Friedman, Type I/III two- & three-way ANOVA, Nested ANOVA, Kenward–Roger/Satterthwaite REML, Tukey/Dunnett/Šídák/Bonferroni/Holm–Šídák/Newman–Keuls/Fisher LSD/Games–Howell/Dunnett T3/Tamhane T2/Dunn/FDR, and Analysis Checklists (Waves 2–4).
- **Scope note (2026-11-15 `D1` citation pass):** the REML denominator *df* labelled Kenward–Roger and Satterthwaite are approximations of neither method. Fixes are tracked in [`R3.5`](#r35-statistical-method-fidelity-fixes-from-the-d1-citation-pass).

### R3.3 Survival, Cox PH, contingency (Barnard/Koopman), method comparison, ROC & GLMs/PCA/MICE
Owner: maintainers · Status: done · Was: G8, M6 · Est. note: 0 h remaining

- [x] Land Kaplan–Meier, log-rank/Gehan–Breslow/trend, Cox PH (`BaselineSurvival`/`LogMinusLog`), contingency ($\chi^2$, Fisher, `BarnardExact`, Baptista–Pike, Wilson, Clopper–Pearson, Cochran–Armitage, McNemar), `BlandAltman`, `PassingBablok`+CUSUM, ROC (DeLong/Hanley–McNeil), multiple linear/logistic/Poisson GLMs, PCA/clustering, `MultipleImputation` (MICE + Rubin), and Wave 5 `G8` Koopman `Option<f64>` `None`-bound fix (`lumencraft-stats` `69 passed`).

### R3.4 Unstructured residual covariance for REML mixed models
Owner: open · Status: todo · Was: spec-vs-code review 2026-11-15 ([`statistics-implementation.md` §3.3](statistics-implementation.md#33-linear-mixed-effects-models-reml--nested-designs) described it; never built) · Est. note: 24–40 h

`fit_random_intercept_reml` (`crates/stats/src/anova.rs`) fits only a random intercept, which implies compound symmetry across repeated timepoints. Repeated-measures data with missing cells therefore cannot relax the equal-correlation assumption.

- [ ] Add a typed residual-covariance choice (`CompoundSymmetry` default, `Unstructured`) to the REML mixed-model spec, persisted with a schema migration step, and fit the unstructured $t(t+1)/2$ covariance by REML (Cholesky-parameterised, with a documented convergence and non-positive-definite fallback).
- [ ] Extend Satterthwaite and Kenward–Roger denominator *df* to the unstructured case, cite the published derivations in doc comments (`D1`), and report the estimated covariance/correlation matrix in the results.
- [ ] Add `D1` oracle tests against an independent REML implementation with documented tolerances, plus edge cases: one subject, all cells missing for a timepoint, more timepoints than complete subjects, and a singular covariance estimate.
- [ ] Expose the choice in the RM ANOVA dialog and the engine command schema; regenerate `docs/commands.md` and `docs/schemas/` (`--check`).

### R3.5 Statistical method-fidelity fixes from the `D1` citation pass
Owner: open · Status: todo · Was: `D1` citation pass 2026-11-15 (code review of `crates/stats`) · Est. note: 30–50 h

While citing sources, reviewers compared each routine with the published method. The defects below were traced in code but are **not yet reproduced by tests**. Each fix starts with a failing test and a `D1` oracle with documented tolerances. This item may land as one PR per checkbox.

- [ ] **REML *df* (`compute_reml_wald_terms`, `anova.rs`):** the "Kenward–Roger" *df* are the Satterthwaite *df* × `1/(1 + 1/(clusters − 1))`, with no covariance adjustment or *F* rescaling. The "Satterthwaite" *df* are `2 / Var(log v)` from a finite difference in τ alone, capped at `n − p` over all observations, with a lopsided step near τ = 0. Implement Kenward & Roger (1997) and Satterthwaite over the joint variance-component information. Until then, stop labelling these *df* as Kenward–Roger/Satterthwaite in checklists and nested-ANOVA results.
- [ ] **Nested ANOVA post-hoc (`run_nested_analysis`):** groups get SD `√(σ²_u + σ²_e)` with `n` = all technical replicates, so standard errors are too small (anti-conservative). Use the group-mean variance `σ²_u/m + σ²_e/n`.
- [ ] **Repeated-measures ANOVA:** two-way RM sphericity is computed only for the column factor and never applied to *F* *p*-values; split-plot sphericity should use the pooled within-group covariance with `N − g` (Huynh & Feldt 1976); the fully-repeated REML fallback fits one random intercept instead of separate S×A, S×B and S×A×B error strata.
- [ ] **ANOVA effect sizes:** `eta_squared_ci` reuses the partial-η² CI and `N = df1 + df2 + 1` (both valid only for one-way designs); Kruskal–Wallis ε² uses `(H − (k − 1))/(N − 1)` instead of `H/(N − 1)` or `(H − k + 1)/(N − k)`; `cohens_f` returns 0 instead of `+∞` at partial η² = 1.
- [ ] **Survival hazard ratios (`survival.rs`):** `mantel_haenszel_hr` computes Peto's one-step `exp((O − E)/V)`; rename it (struct field, docs, UI label) or implement the Mantel–Haenszel estimator. The O/E-ratio HR CI uses `SE = 1/√V` instead of `√(1/E_A + 1/E_B)` and silently switches estimator when `E_A`, `E_B` or `O_B` is 0. Brookmeyer–Crowley ignores its `z_crit` argument.
- [ ] **Two-group tests (`two_group.rs`):** Welch effect-size noncentral-*t* CIs use the Welch *df* instead of `n_a + n_b − 2`; Pratt-mode signed-rank variance double-counts tied zeros (understated by `(z³ − z)/48`); Hodges–Lehmann returns the difference of medians with no CI when `m·n > 40,000`; the pairing-correlation *p* is one-sided and returns 0 at `r = −1`; summary-data *t*-tests report a placeholder Levene *p* = 1; one-tailed direction conventions differ between the sign test and Mann–Whitney.
- [ ] **Power & multiplicity:** `CorrelationPowerMethod::ExactTTransform` is the fixed-*x* slope test, not the exact distribution of *r* (rename or implement); correlation and regression power use different `n` conventions for the same effect; `compare_dunnett` silently uses group 0 when `control_column` matches no group (should error); `raw_p` holds the adjusted *p* for Tukey–Kramer, Games–Howell, Dunnett and Dunnett T3; `DunnRankSum` and `LinearTrend` on the parametric path give unadjusted output; `linear_trend_test` returns *F* = 0, *p* = 1 when MSE = 0.
- [ ] **Minor:** Bartlett returns χ² = 0, *p* = 1 when any group has zero variance; the Kruskal–Wallis tie factor is reported as 1.0 when all values are tied; fully aliased factorial terms show *df* 1; remove dead computations (`n0` in nested ANOVA, an unused *t* *p*-value in `run_nested_analysis`).

---

## R4 Nonlinear regression solver, equation library & custom DSL/ODE

### R4.1 Damped LM solver, weighting, robust/ROUT, shared globals, profile CIs & AICc/*F*/LRT
Owner: maintainers · Status: done · Was: G4 (solver), M3 · Est. note: 0 h remaining

- [x] Land Moré QR trust-region LM + Nelder–Mead warm start, `fit_independent_datasets` fast path, differential weighting, Cauchy/ROUT robust regression, constraints, `PerDatasetConstant`/`ColumnTitleConstant`/hooked constants, global fits, profile-likelihood & `CiBound::Open` intervals, log-grid curve overlays, range filters, `InterpolatedUnknowns`/`Residuals`/`CovarianceMatrix`, and AICc/extra-SS *F*/`LikelihoodRatioTest` (Waves 2–4).

### R4.2 Built-in equation library (`117` models across all categories)
Owner: maintainers · Status: done · Was: G4 (library), M3 · Est. note: 0 h remaining

- [x] Land `117` built-in `ParametricModel` equations (`47` pharma/dose–response/receptor-binding/enzyme-kinetics incl. Wave 5 `two_site_total_and_ns_global` and `one_site_competition_ligand_depletion`, `43` exponential/growth/peak/wave/polynomial/line curves incl. centered polys & semi-log/log-log/probit, `27` NIST StRD datasets; `lumencraft-fit` `87 passed`).

### R4.3 Safe custom algebraic/implicit/ODE DSL bytecode VM
Owner: maintainers · Status: done · Was: G9, M6 · Est. note: 0 h remaining

- [x] Land custom bytecode VM (`IF`/`AND`/`OR`, `[InitialValues]`, `[Transforms]`, `<A>`..`<Z>`/`<0>`..`<9999>`, `brent_root` implicit & RK45 ODE models) plus Wave 5 `G9` `MAX_AST_NODES = 4_096` parser cap and iterative `Clone`/`Debug` on `Expr` (Waves 2–5).

### R4.4 Derived table transforms
Owner: maintainers · Status: done · Was: G9 (transforms), M2 · Est. note: 0 h remaining

- [x] Land `lumencraft-transform` derived transforms (`Transform`, `Normalize`, `Baseline`, `Transpose`, `Prune`, `Simulate`, `Smooth/Differentiate/AUC`, `RowMeans`, `FractionOfTotal`) wired through `lumencraft-engine` (`24 passed`).

### R4.5 Suppress least-squares inference in pure Robust fit mode
Owner: open · Status: todo · Was: audit `R1-S03-06` (S2); [`curve-fitting-implementation.md`](curve-fitting-implementation.md) described it, never built · Est. note: 8–14 h

A `FitMethod::RobustCauchy` fit runs through the same covariance, profile-CI, band, information-criterion and `compare_models` code as least squares. It reports SEs, CIs, confidence bands, AICc/BIC and *F*-test *p*-values computed from the Cauchy-reweighted sum of squares, which has no sampling interpretation.

- [ ] Add a typed not-applicable status for robust fits (e.g. `NotApplicableInRobustMode`) and return it instead of values for parameter SEs and CIs, the covariance matrix, confidence/prediction bands, AICc/BIC, extra-SS *F* and likelihood-ratio comparisons. Keep best-fit values, residuals, RSDR and interpolated unknowns (without CIs).
- [ ] Render the status as "not applicable" on result sheets, graph bands, MCP/CLI output, and in `compare_models` when either model is robust.
- [ ] Leave ROUT Stage 2 (least squares after outlier removal) reporting full inference; add tests covering both paths, including the audit's robust-fit reproducer.

### R4.6 Fitting method-fidelity fixes from the `D1` citation pass
Owner: open · Status: in progress · Was: `D1` citation pass 2026-11-15 (code review of `crates/fit`) · Est. note: 24–40 h

Traced in code while adding citations; **not yet reproduced by tests**. Each fix starts with a failing test and a `D1` oracle with documented tolerances. This item may land as one PR per checkbox.

- [x] **Deming regression λ inverted (`linear.rs`):** fixed `fit_deming_core` / `deming_regression` so `DemingOptions::variance_ratio_lambda` is strictly `σx²/σy²` (matching `r46_linear_oracles::{deming_lambda_sigma_x2_over_sigma_y2_reproducer, deming_50_digit_oracles_across_lambda_and_ols_limits, deming_xy_swap_reciprocity_and_edge_cases}` across `λ ∈ {0.25, 0.5, 1.0, 2.0, 4.0}`, OLS limits, and X↔Y swap reciprocity).
- [ ] **Poisson fits in model comparison:** `compare_models` and `compute_information_criteria` take `weighted_ss`, which is the Poisson deviance under `PoissonDeviance`, into the extra-SS *F* test and AICc `N·ln(SS/N)`. Restrict Poisson fits to the deviance likelihood-ratio test and a likelihood-based AIC.
- [ ] **Replicates (lack-of-fit) test:** combines the weighted residual SS with unweighted pure error; SS scales also differ for pre-summarised points.
- [ ] **ROUT:** applies Benjamini–Hochberg to all `N` points, whereas Motulsky & Brown (2006) test only the 30% of points furthest from the curve. Align with the paper or document and justify the deviation.
- [ ] **Shapiro–Wilk (residual normality) for n = 4–5:** uses normalised Blom scores instead of Royston's polynomial-corrected `a_n`; remove the unreachable `if n > 5` branch.
- [ ] **Inverse interpolation (`interpolate_x_from_y`, linear X-from-Y):** inverts the mean-response band, omitting new-observation error, so the interval is narrower than a calibration interval. Offer a prediction-based interval or label the current one accurately in results. *(Core `linear.rs` calibration interval API — `InverseIntervalSet`, `InverseCalibrationResult`, `interpolate_x_from_y`, `solve_inverse_quadratic`, and `pred_lower..pred_upper` — landed and verified in `r46_linear_oracles::{inverse_x_from_y_calibration_vs_mean_response_d1_oracles, inverse_x_from_y_fieller_regimes_g_eq_1_and_gt_1_and_edges}` and `linear::tests::solve_inverse_quadratic_exact_g_eq_1_and_extreme_regimes`; `crates/fit/src/result.rs` output labeling and nonlinear `crates/fit/src/solver.rs::interpolate_x_from_y` remain open.)*
- [ ] **Equation library:** `beta_growth_then_decay` returns 0 for `t > te` (and everywhere when `te ≤ tm`) instead of holding `Ym` per Yin et al. (2003); `biphasic_dose_response` falls with X for positive `nH`, and its initial guess gives a falling curve; `centered_poly_*` leaves `XCenter` free (singular Jacobian), so give it a constant default; `absolute_ic50_4pl` uses `Baseline/2` instead of the mean of the 0% and 100% controls (Sebaugh 2011); rename `power_series` (it is a power law); resolve `exponential_plateau` duplicating `one_phase_association`; confirm the NIST `Nelson` fixture fits `log(y)`. *(Library & TeX fixes landed and verified in `r46_library_oracles::{r46_1_beta_growth_holds_ym_after_te_and_rejects_te_le_tm, r46_2_biphasic_dose_response_sign_convention_and_direction_guess, r46_3_centered_poly_free_xcenter_is_rank_deficient_and_default_rules_fix_sample_mean, r46_4_absolute_ic50_sebaugh_2011_controls_midpoint, r46_5_power_law_rename_and_legacy_alias, r46_6_exponential_plateau_vs_one_phase_association_resolution, r46_7_nist_nelson_fits_log_y_oracle}` + `nist_strd`; wiring `ParametricModel::default_parameter_rules` into `ParameterLayout::build`/`solver.rs` remains open for `r46-solver-diag`.)*
- [ ] **Minor:** RK45 rejected steps shrink with exponent −1/4 instead of −1/5; the LM λ is found heuristically rather than by Moré's Newton iteration (align or document); remove dead code (`ratio` in `logistic_growth`, `denom` in the operational model). *(Library dead computations `ratio` in `LogisticGrowth` and `denom` in `OperationalAgonism` removed and verified in `r46_library_oracles::r46_8_logistic_growth_and_operational_model_oracles`; RK45 & Moré LM λ remain open.)*

---

## R5 Publication graphing, vector export & Page Layouts

### R5.1 Scientific scene compiler, rich typography, nonlinear/broken axes & brackets
Owner: maintainers · Status: done · Was: G5 (chart/text), S2, M4 · Est. note: 0 h remaining

- [x] Land `lumencraft-chart` (`40 passed`) & `lumencraft-text` (`25 passed` default & `--no-default-features`) across 8 graph families + diagnostic plots, all 7 `AxisScale`s, 1–3 broken axis segments, `AdditionalTick`, auto-stacked live *p*/*q* brackets, and BiDi RTL text shaping (Waves 2–4).

### R5.2 Publication SVG, `%PDF-1.7` + `/FontFile2`, PNG/TIFF & ECMA-234 `.emf` export
Owner: maintainers · Status: done · Was: G5 (export & QA), G11 (EMF & raster cap), M4 · Est. note: 0 h remaining

- [x] Land `lumencraft-svg` (`11 passed`), `lumencraft-pdf` (`%PDF-1.7` + optional `/FontFile2` + Core-14/outlined fallback), `lumencraft-render` (PNG + Deflate/LZW TIFF), Wave 5 `G11` `MAX_RASTER_PIXELS = 67_108_864` + `try_reserve_exact` + pure-Rust ECMA-234 `.emf` encoder (`encode_emf`), and Wave 5 `G5`/`G13` Poppler + Ghostscript multi-engine `%PDF-1.7` pixel QA (`external_viewer_qa`, `3 passed` on Latin/Greek/ribbon, CJK, and RTL).

### R5.3 Headless Page Layouts, style cloning, spool PDF & subprocess/macOS clipboard
Owner: maintainers · Status: done · Was: G11 (partial), M6 · Est. note: 0 h remaining

- [x] Land `LayoutSheet` plot-origin alignment (`layout.alignOrigins`) & panel lettering, `graph.applyStyleFrom` ("Wand"), `ClipboardBundle` (with Wave 7 `emf_bytes`), `PrintJobSpec::spool_pdf`, subprocess pasteboard/print adapters, and Wave 7 live macOS `/usr/bin/pbcopy` + `/usr/bin/pbpaste` roundtrip (`macos_clipboard_live`, `EXECUTED_LIVE_ROUNDTRIP`).

### R5.4 Native binary OS vector clipboard, interactive print dialogs & live layout drag-snap
Owner: maintainers · Status: done · Was: G11 (**Beta**) · Est. note: 0 h remaining

- [x] Implement native binary OS vector pasteboard APIs (`write_os_clipboard_bundle` / `read_os_clipboard_pdf_bytes`: macOS `NSPasteboard` `com.adobe.pdf`/`public.png`/`public.svg-image`/`public.html`/`public.utf8-plain-text` verified live with `%PDF-1.7` byte roundtrip in `macos_clipboard_live`, Windows `CF_ENHMETAFILE`/`PNG`/`CF_HTML`/`SVG` STA adapter, Linux `wl-copy`/`xclip` `application/pdf`, and browser host clipboard adapters) (Wave 13).
- [x] Wire interactive `ActiveDialog::Print` / `PrintDialogState` (`ui.print.open`, `file.print`) + OS printer discovery & print preview (`discover_os_printers`, `open_os_print_preview_dialog`) and live WYSIWYG layout `PlotOrigin` drag-snap, resize handles (`LayoutResizeHandle`), and alignment guide hairlines (Wave 13).

---

## R6 Multiple Variables workflows, Calculated Variables & Multifactor ANOVA

### R6.1 `=` Calculated Variables, role locking & level ordering
Owner: maintainers · Status: done · Was: G7 (partial), M6 · Est. note: 0 h remaining

- [x] Land in-table `=` Calculated Variables (`100k` rows `< 15 ms` `--release`), column role inference/locking, bulk recoding, and `LevelOrderSpec` wired through `engine` and `UiApp` (Waves 2–4).

### R6.2 Multifactor ANOVA, MV grouping/splits & five-channel MV plots
Owner: maintainers · Status: done · Was: G7, M6 · Est. note: 0 h remaining

- [x] Land Multifactor ($N$-way Type I/III) ANOVA, `TwoGroupSpec` MV grouping/response columns, `MultivarExtractionSpec` Cartesian splits in `fit`, `KaplanMeier` MV auto-detection, and BubbleScatter/ConvexHull/ConfidenceEllipse/CategoricalSummary plots (Waves 3–4).

---

## R7 Headless automation, CLI, `--control`, MCP & performance budgets

### R7.1 `Slice 0`, headless CLI, `--control` TCP JSONL & MCP server
Owner: maintainers · Status: done · Was: G14, Slice 0, M5 (headless) · Est. note: 0 h remaining

- [x] Land `Slice 0` vertical gate, `lumencraft-cli` (`31 passed` Wave 14 -> `35 passed` Wave 17, incl. `.lctemplate`, `perf --json`, `.lumencraft.d`, `.emf`, & `mcp_gui_only_parity`), `lumencraft --control` (`17 passed` Wave 14 -> `19 passed` Wave 17 across `lumencraft` incl. `gui_only_wiring`), `lumencraft-mcp` (`33 passed` / `30` synced `UI_COMMANDS` Wave 14 -> `39 passed` / `32` Wave 17 -> `45` Wave 22 synced `UI_COMMANDS` incl. `gui_only_surface`), and `90` -> `92` (Wave 15) -> `95` (Wave 22) registered engine commands (Waves 2–22).

### R7.2 Performance budgets, `1M`-cell virtualization & property/fuzz suites
Owner: maintainers · Status: done · Was: G12, M6 · Est. note: 0 h remaining

- [x] Verify `< 50 ms` `96`-curve 4PL batch solve (`--release`), `< 250 ms` `100k` Calculated Variables, `1M`-cell `S1` grid (`< 1 ms` median frame), 7 property/fuzz harnesses, and Wave 6 `G12` read-only clone-skip & bounded undo (`MAX_UNDO_ENTRIES = 64`, `MAX_UNDO_CELL_BUDGET = 25_000_000`).

### R7.3 MCP workflow parity proof & shared live window
Owner: open · Status: todo · Priority: after `R0.18`, before new `R10` GUI implementation · Depends on: `R0.18`, `R7.1`, `R8.3`, `R8.6` · Was: MCP capability review 2026-11-15 · Est. note: 16–28 h

**Completion criterion:** All currently implemented engine workflows must be usable through the actual MCP transport, with verified results, persistence, exports, and structured failures. An in-process semantic-harness pass alone does not establish MCP readiness or human GUI usability. Use the existing engine/session and desktop presenter; completion must not depend on new human-operable GUI widgets.

- [ ] Run `WF-01`–`WF-49` end to end through `lumencraft-cli mcp --gui-only` over stdio JSON-RPC, plus a headless variant through `command_run` / `command_batch`, in CI. Verify the real process/transport boundary and resulting documents/exports rather than substituting calls to `dispatch_gui_action`.
- [ ] Verify the existing MCP surface end to end: initialization, tool discovery/toolsets, resources and prompts, project open/edit/analyze/fit/save/reopen/export, actionable structured errors and path containment, and strict `--gui-only` policy. Resolve transport or exposure gaps for implemented engine capabilities without adding new statistical methods under this item.
- [ ] Add an opt-in flag so `lumencraft --control <port>` attaches to a visible desktop window instead of running headless, letting an agent over MCP and a person share one session; keep loopback-only binding (`validate_loopback_addr`). Verify MCP edits and saved state use that same session through `--control`, without requiring completion of `R10` GUI controls.

**Future-feature follow-up:** Extend MCP acceptance coverage when new engine features land, including `WF-50` after [`R10.11`](#r1011-compact-letter-display-cld). These later additions do not block the existing-capability MCP gate or create a dependency from `R7.3` back to `R10`. The CurveBook rename remains separately unscheduled.

---

### R7.4 Accessible MCP guide and minimal README
Owner: open · Status: todo · Priority: immediately after verified `R7.3`, before the GUI handoff · Depends on: `R7.3` · Was: MCP onboarding and README simplification request 2026-11-15

Provide a short, welcoming entry point for a researcher or first-time MCP user. Write the guide only after the MCP implementation and transport acceptance checks pass, so every instruction describes working behavior. This is a documentation follow-up, not a claim that MCP is complete or an instruction to rewrite the README now.

- [ ] Reduce `README.md` to a presentable functional minimum: a concise introduction, an honest pre-alpha status line, prerequisites and a short working quick start, the beginner MCP guide, a compact set of useful documentation/support links, and brief licence/privacy information. Keep visual identity restrained. Replace long architecture listings, exhaustive capability lists, progress/test counts, and screenshot galleries with links to detailed docs; preserve contributor credits and full legal notices in their canonical files.
- [ ] Add a simple MCP guide directly to the README: explain MCP in one plain-language sentence; state the prerequisites and how to obtain/build the server; give copy-and-paste launch/configuration instructions with executable-path guidance; explain how to verify the connection; and include one small reproducible example with an expected result. Use the actually implemented default headless mode and current product/toolchain names. Explain unfamiliar terms and keep troubleshooting to the common startup/path problems.
- [ ] Link to `docs/mcp.md` for protocol details, the shared-window mode, `--gui-only`, advanced configuration, and limitations. Keep these details out of the first-run path and reconcile the linked guide with the final R7.3 behavior.
- [ ] Follow the README from a clean checkout or equivalent fresh setup through MCP connection and the example, verifying the real transport, commands/configuration, expected output, links, and stated limitations. Record what ran and fix onboarding failures before marking this item done; do not promise features still missing from the implementation.

---

## R8 Desktop & WebAssembly live shell, localization & `1.0` hardening

### R8.1 Pure-Rust `UiApp` shell, software RGBA compositor, HiDPI/RTL input & `WebWorkspaceHost`
Owner: maintainers · Status: done · Was: G6 (shell), S1, M5 (partial) · Est. note: 0 h remaining

- [x] Land `lumencraft-tools` (`22 passed`, **S1 GO**), `lumencraft-ui-egui` (`83 passed` Wave 14 -> `122 passed` Wave 17 -> `155 passed` Waves 20–23, `UiApp`, RGBA/PNG compositor incl. `1440x900` `doc_screenshots`, `30` -> `32` -> `45` `UI_COMMANDS`, `NavigatorDragState`, `PortfolioGalleryState`, `F6` pane cycle, rulers, `ShortcutRegistry`, `ViewportMetrics` `1.0x`–`3.0x` HiDPI, `WorkspaceLayoutMap`, `UiInputEvent`, `InlineEditorState`, `VirtualWorkspaceFolder`), and `apps/lumencraft-web` `WebWorkspaceHost` (`13 passed`, `#![forbid(unsafe_code)]`, zero OS calls).
- **Scope note (2026-11-15 audit):** the compositor draws chrome with an uppercase-only 5×7 ASCII bitmap font, renders modal dialogs as at most 6 static text lines (12 of the 26 dialogs show only a generic placeholder line), truncates result sheets to 4 tabs × 6 rows × column 0, and `WorkspaceLayoutMap::hit_test_physical` never hit-tests the modal overlay, toolbar, column headers, or result tabs. Done here means the state shell and presenter, not interactive widgets; those are tracked in [`R10`](#r10-human-operable-gui--50-workflow-human-completion).

### R8.2 `16`-locale i18n, RTL mirroring & desktop CJK/RTL font/pasteboard checks
Owner: maintainers · Status: done · Was: G13 (core & macOS QA), M7 (partial) · Est. note: 0 h remaining

- [x] Land `16` locales × `307` keys = `4,912` translations (`0` missing catalog or command keys in `cargo xtask i18n-report`; expanded from `287` keys / `4,592` translations in Wave 14 and `291` keys / `4,656` translations in Wave 17), RTL UI mirroring (`ar`/`he`), locale decimal/CSV presets, Poppler+Ghostscript CJK/RTL `%PDF-1.7` QA, and live macOS `pbcopy`/`pbpaste` `de-DE` comma + `=A1*2` formula roundtrip (`crates/ui-egui/tests/macos_clipboard_live.rs`). Remaining live OS target-host verification joins `R8.5`.

### R8.3 Live OS `eframe`/`egui`/`wgpu` desktop windowing & interactive event loop
Owner: maintainers · Status: done · Was: G6 (desktop **Alpha** + **Core**), M5 · Est. note: 0 h remaining

- [x] Implement `DesktopHostRuntime` (`apps/lumencraft/src/desktop_host.rs`) with `--in-window-menus`, `--new-instance` isolated crash-recovery journals, Retina DPI transitions, IME composition, trackpad pinch/inertial pan, and file-drop integration tests (`desktop_host_loop`; Wave 13).
- [x] Link `eframe 0.37` / `egui 0.37` (`default-features = false`, `wgpu`, `glow`, Linux `wayland` + `x11`) and `rfd 0.17` in `Cargo.toml` / `Cargo.lock` (`417` locked packages) and implement `apps/lumencraft/src/gui_window.rs` (`LumenCraftNativeApp`, `build_egui_font_definitions`, `run_native_desktop`, top `egui::MenuBar` + native `rfd::FileDialog`, Retina `pixels_per_point` resize, pointer/click-count/wheel/pinch/IME/clipboard/file-drop routing, `TextureOptions::NEAREST` RGBA viewport upload, coasting `AnimationTick`, and `egui::Modal` unsaved-close prompt while preserving `--headless-once`, `--event-script`, and `--control`; `lumencraft` `23 passed`, **Alpha**/**Core**).
- **Scope note (2026-11-15 audit):** below the native menu bar the window is a single textured quad of the CPU RGBA buffer. Engine menu items are dispatched with `Value::Null` params (`desktop_host.rs`), so commands with required fields (e.g. `table.excludeCells`, `Analyze > Statistical Test…`) fail with `BadInput`; `--control` runs headless without opening a window. Interactive widgets and parameter routing are tracked in [`R10.1`](#r101-interactive-chrome-architecture-decision--spike), [`R10.3`](#r103-interactive-modal-dialog--wizard-framework-b1), and [`R10.6`](#r106-menu-toolbar--context-menu-command-routing-b4-desktop-b5).

### R8.4 Live `wasm32-unknown-unknown` Trunk WebGPU/WebGL2 browser hosting
Owner: maintainers · Status: done · Was: G6 (WASM **Alpha**), M5 · Est. note: 0 h remaining

- [x] Implement `apps/lumencraft-web/src/host_adapters.rs` (`WebStorageManager` 5 MiB `localStorage` + forward-version guard, `WebLockCoordinator` multi-tab read-only contention fallback, `FileSystemAccessBridge` incremental `.lumencraft.d/` sync, `TouchGestureRecognizer`, and `WebBridgeSession` protocol; `lumencraft-web` `13 passed`) plus strict no-egress `index.html` and CSP/COOP/COEP headers (`packaging/web/{_headers,.htaccess}`) (Wave 13).
- [x] Connect `apps/lumencraft-web/src/main.rs` (`connect_wasm_bridge` via `wasm-bindgen 0.2.130` / `web-sys` / `js-sys` under `#![forbid(unsafe_code)]`) and implement the zero-dependency WebGL2 textured-quad RGBA presenter (with lazy 2D fallback) + DOM mouse/wheel/keyboard/IME/paste listeners and `data-wasm-opt-params` in `apps/lumencraft-web/index.html`, verified via `cargo xtask wasm`, `cargo xtask telemetry` (`0 violations`), and `./packaging/web/package.sh` (`Trunk 0.22.1` `--cargo-profile web-release` -> `lumencraft-web-0.1.0.zip`, `4,162,394` B, **Alpha**).
- **Scope note (2026-11-15 audit):** `lumencraft-web` does not link `egui`/`eframe`; it blits the RGBA buffer to a WebGL2 `<canvas>` and has no menu bar or toolbar in HTML or WASM. Web chrome parity is tracked in [`R10.7`](#r107-web-menu-bar-toolbar--chrome-parity-b4-web).

### R8.5 Cross-OS hardware, CJK/RTL desktop QA & installer/signing hardening
Owner: open · Status: in progress · Was: G13, M7 (**1.0**)

- [x] Implement macOS `Info.plist` `plutil -lint` validation and ad-hoc `codesign --force --deep --sign -` + `--verify --strict --deep` in `xtask/src/bundle.rs` (Wave 7), live macOS `NSPasteboard` `%PDF-1.7` roundtrip (`macos_clipboard_live`), `cjk_rtl_hidpi_desktop_qa` (`ja`/`zh-CN`/`ko`/`ar`/`he` across `1.0x`/`1.5x`/`2.0x`/`3.0x`, `4 passed`), `cargo about generate` `THIRD-PARTY-LICENSES.html` (`273K`) + `cargo deny check licenses`, and local universal macOS (`aarch64` + `x86_64`) `lumencraft-0.1.0-macos-universal.dmg` (`36,340,795` B) + `lumencraft-cli-0.1.0-macos-universal.zip` (`13,504,838` B) packaging (`package.sh --arch universal`), `verify.sh --arch universal` mount/signature validation, and `check-app-specific.sh`.
- [ ] Complete Apple Developer ID signing + notarization, Windows VM, Linux X11/Wayland VM (AppImage/deb/rpm/Flatpak), FreeBSD runner, and live browser QA runs.

### R8.6 Semantic UI inspection, `gui` MCP toolset & strict `--gui-only` lock
Owner: maintainers · Status: done · Was: GUI-agent Phase A ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Add `crates/ui-egui/src/inspect.rs` (`UiTreeSnapshot` / `inspect_ui_tree` / `ui.inspect`) exposing localized menus, active modal/wizard `tabs[]` (`UiTabSnapshot`) and recursive `widgets[]` (`UiWidgetNode`) with logical/physical bounds, values, and `enabled` + `disabled_reason` predicates, visible grid viewport cells (`excluded`, selection, link provenance), and hit-testable canvas scene targets, with `UI_COMMANDS` (`32` Wave 16 -> `45` Waves 20–22) synced to `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (Waves 15–22).
- [x] Expose the 8-tool human-equivalent `gui` MCP toolset (`GUI_TOOL_NAMES` in `crates/mcp/src/gui_tools.rs`: `ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`, `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`) operating identically across in-process `UiApp` sessions (`GuiSessionHost` / `InProcessUiGuiHost`) and live `--control` TCP sessions (`crates/mcp/tests/gui_only_surface.rs` `6 passed`, `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs` `3 passed`, Waves 15–17).
- [x] Implement `--gui-only` enforcement (`GuiOnlyPolicy::Strict` in `crates/engine/src/lib.rs`, `ErrorCode::GuiOnlyViolation` / `"gui_only_violation"` in `crates/doc/src/result.rs`, `GUI_ONLY_SERVER_INSTRUCTIONS` in `crates/mcp/src/protocol.rs`) on `lumencraft-cli mcp --gui-only` and `lumencraft --control <port> --gui-only` (`apps/lumencraft/tests/gui_only_wiring.rs` `2 passed`), refusing direct `L5` engine bypass commands, `doc_inspect`, `resources/read`, and `ui.screenshot` disk writes while allowing internal `UiApp` widget handlers (`Caller::Ui`) and read-only UI inspection calls to commit cleanly (Waves 15–17).

### R8.7 Interactive Power & Sample Size wizard, dynamic modal predicates & chained GUI surfaces
Owner: maintainers · Status: done · Was: GUI-agent Phase B ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Implement the standalone 4-stage **Power & Sample Size** wizard (`crates/ui-egui/src/power_wizard.rs` `PowerWizardState` / `PowerWizardStage::{ExperimentDesign, EffectSize, SignificanceAndSolve, Results}` & `crates/stats/src/power.rs` across all `8` `PowerTestFamily` variants — two-sample unpaired *t*, paired *t* with within-pair correlation $r$, one-sample *t*, one-way ANOVA, two proportions, correlation, $\chi^2$, and simple linear regression — solving for Sample Size, Power, or Detectable Effect, with interactive `power.curve_hover` curves, sensitivity tables, live right-sidebar parameter recalculation, and linked XY+Info+Graph+Result sheet creation; `crates/stats/tests/power_oracle.rs` `8 passed`, Waves 15–16).
- [x] Implement all 10 declarative conditional enablement and graying predicates `R1`–`R10` (`crates/ui-egui/src/modal_predicates.rs`: ANOVA pairing -> `Repeated Measures` tab & sphericity/REML `R1`; Gaussian toggle -> parametric vs. `Dunn's` rank post-hoc `R2`; control-column contrast -> `Dunnett` `R3`; two-group mode -> residual/rank/Welch toggles `R4`; contingency $2 \times 2$ clinical effect sizes vs. $R \times 2$ Cochran–Armitage trend `R5` & parent-coupled CI selectors `R6`; correlation matrix heatmap gate `R7`; single-subcolumn aggregation gate `R8`; custom *p*-value digit spinner `R9`; and leakage-safe `PCA` -> `PCR` dependent-variable exclusion `R10`, Waves 15–16).
- [x] Wire multi-row grid selection -> right-click `Format Selected Points` (`ActiveDialog::FormatSelectedPoints`, `PointStyleOverride`, `graph.formatSelectedPoints`, `ui.formatPoints.open` for symbol color, symbol shape, and row-label hit tagging), `GroupedArrangement::Superimposed`, `Cmd+T` / `EditAnalysisParameters` `FactorialAnovaSpec::multiplicity` UI editing (generating main/simple-effect post-hoc comparison tabs in `run_factorial_anova` completed in [`R9.12`](#r912-categorical--continuous-table-analysis-completions-qol-12)), and direct toolbar `Analyze` chaining from tabular intermediate `ResultSheet` tabs (`sheet.extractResultTab` / direct `ResultSheet` analysis for `Transform`, `Transpose`, `Extract and Rearrange`, `Outlier Cleaned Data`, `PCA Scores`; verified in `gui_only_workflows` `4 passed`, Waves 15–16 & Wave 21).

### R8.8 Clean-room 28-workflow `gui_only` acceptance suite
Owner: maintainers · Status: done · Was: GUI-agent Phase C ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Build and pass the 4-shard deterministic `gui_only` acceptance suite (`crates/ui-egui/tests/gui_wf_foundations_and_data.rs` [`6`], `gui_wf_group_comparisons_and_power.rs` [`9`], `gui_wf_regression_contingency_multivariate.rs` [`8`], `gui_wf_statistical_graphs.rs` [`5`] = `28` workflows `wf01`–`wf28`, alongside `gui_only_workflows.rs` [`4`], `crates/mcp/tests/gui_only_surface.rs` [`6`], `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs` [`3`], and `apps/lumencraft/tests/gui_only_wiring.rs` [`2`]) driving original synthetic `lumencraft-testkit` datasets strictly through `ui.inspect` + GUI input primitives + `ui.screenshot` PNG verification under `GuiOnlyPolicy::Strict`, asserting live upstream cell-edit and `Cmd/Ctrl+E` exclusion DAG recomputation (**Core workflows** GUI parity gate, Wave 17).
- **Scope note for `R8.6`–`R8.8` (2026-11-15 audit):** these items deliver the **semantic agent harness**. `ui.inspect` widget nodes (`analyze.entry`, `edit.two_group.welch`, `power.next`, `result.tab.*`, `grid.formula_bar`, …) are synthesized from state and are not drawn; tests pass structured params through `ui.invoke_menu` and patch dialog fields through `ui.answer_dialog` / `try_activate_semantic_widget`. Passing them does not show that a person can complete a workflow. The gate formerly named **Core workflows** is renamed **Agent-harness workflows**; human usability is gated by [`R10.2`](#r102-human-path-acceptance-harness--phantom-widget-gate) and [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa).

---

## R9 Post-parity scientific quality-of-life suite

Planning decisions in [`qol-beyond-parity.md`](qol-beyond-parity.md) separate existing implementation evidence from future scope. Deferred items are not in the active queue; existing code and persisted-file compatibility remain supported. Unchecked tasks below describe work still required, not newly assigned dates or implementation claims.

### R9.1 Non-destructive table archetype conversion (`QOL-1`)
Owner: maintainers · Status: done · Was: QOL-1, M8+ · Est. note: 0 h remaining

- [x] Implemented `preview_datasheet_conversion` & `convert_datasheet_archetype` (`crates/transform`), `table.convertArchetype` (`crates/engine`), and `ConvertArchetypeDialogState` (`crates/ui-egui`) preserving `SheetId` and remapping `exclusion_notes` with lossy guard (Waves 5–7).

### R9.2 `96`/`384`-well microplate mapper & tidy long-to-wide pivot (`QOL-2`)
Owner: maintainers · Status: done · Was: QOL-2, M8+ · Est. note: 0 h remaining

- [x] Implemented `PlateMapTemplate` / `map_plate_to_datasheet` (`96`/`384`-well) & `pivot_tidy_long_to_datasheet` (`crates/transform`), `table.importPlateMap` & `table.importTidyLong` (`crates/engine`), and `ImportReshapeMode` (`crates/ui-egui`) (Waves 5–7).

### R9.3 Auditable cell exclusion reasons & provenance audit (`QOL-3`)
Owner: maintainers · Status: done · Was: QOL-3, M8+ · Est. note: 0 h remaining

- [x] Implemented `CellExclusionNote` & `DataSheet::set_excluded_with_reason` (`crates/doc`), `.lumencraft`/`.lumencraft.d` serialization (`crates/format`), `table.excludeCellsWithReason` & `table.exclusionAuditReport` (`crates/engine`), and `ExcludeWithReasonDialogState` (`Cmd+Shift+E`, `crates/ui-egui`) (Waves 5–7).

### R9.4 Optional side-by-side split workspace (`QOL-4`)
Owner: maintainers · Status: done · Was: QOL-4, M8+ · Est. note: 0 h remaining

- [x] Implemented `SplitWorkspaceState` (`ui.split.toggle`, `Cmd+\`), split divider dragging, and side-by-side Data/Result + live Graph rendering in `crates/ui-egui` (Wave 7).

### R9.5 Two-way data-to-graph point brushing (`QOL-5`)
Owner: maintainers · Status: done · Was: QOL-5, M8+ · Est. note: 0 h remaining

- [x] Implemented `SourceCellId` & `PlottedPointMeta` (`ChartScene::hit_test_point` across XY, Column, Grouped, Nested, Survival, MV, and Layouts in `crates/chart`) and `ui.brush.point` bidirectional highlight in `crates/ui-egui` (Waves 5 & 7).

### R9.6 One-click Methods & statistical reporting generator (`QOL-6`)
Owner: maintainers · Status: done · Was: QOL-6, M8+ · Est. note: 0 h remaining

- [x] Implemented `generate_methods_report` (`PlainText`, `Markdown`, `Latex` in `crates/engine/src/methods_report.rs`), `result.generateReport`, and `ui.result.copyMethodsReport` in `crates/ui-egui` (Waves 6–7).

### R9.7 Inline assumption diagnostic banners (diagnostics only, `QOL-7`)
Owner: maintainers · Status: done · Was: QOL-7, M8+ · Est. note: 0 h remaining

- [x] Implemented `AssumptionDiagnosticBanner` (`crates/ui-egui/src/dialogs_canvas.rs`, `sanitize_diagnostics_only_text` in `crates/ui-egui/src/dialogs_r9.rs`), `ui.result.inspectDiagnostics` (opening diagnostic detail tabs with `mutated: false`), and deprecated `ui.result.applyRemediation` returning `ErrorCode::Conflict` (`details.code: "diagnostics_only_policy"`, `mutated: false`) without creating sibling analyses or recommending alternative tests, while preserving numerical outputs and explicit `Cmd+T` parameter switching (`WF-10`, `WF-14`, `gui_w22b_qol_and_r9`; Waves 20–22b).

### R9.8 Journal figure dimension & typography lock presets (`QOL-8`)
Owner: maintainers · Status: done · Was: QOL-8, M8+ · Est. note: 0 h remaining

- [x] Implemented `FigureCanvasPreset`, `CanvasResizeMode` (`KeepTypographyPt` vs. `ProportionalScale`), `preflight_check_scene`, `graph.applyCanvasPreset`/`layout.applyCanvasPreset`/`graph.preflightCheck`, and `ui.canvas.applyPreset`/`ui.canvas.preflight` (Waves 5–7).

### R9.9 Live color-vision-deficiency & grayscale canvas preview (`QOL-9`)
Owner: maintainers · Status: done · Was: QOL-9, M8+ · Est. note: 0 h remaining

- [x] Implemented `CanvasPreviewFilter`, `apply_canvas_preview_filter` (`crates/render`), and `ui.previewFilter.set` non-mutating viewport filter in `crates/ui-egui` & `WebWorkspaceHost` (Waves 5 & 7).

### R9.10 Zero-install client-side WASM viewer & virtual workspace folder (`QOL-10`)
Owner: maintainers · Status: deferred · Was: QOL-10, M8+ · Scheduling: no milestone assigned

- Existing `VirtualWorkspaceFolder`, `presentation_mode`, and `WebWorkspaceHost` code remains supported. Further collaborator-viewer and folder-workspace QoL work is deferred. Core WASM compatibility and baseline browser hosting in `R8.4` remain separate requirements.

### R9.11 Unpacked Git-friendly `.lumencraft.d/` directory & checkpoints (`QOL-11`)
Owner: maintainers · Status: deferred · Was: QOL-11, M8+ · Scheduling: no milestone assigned

- Existing unpacked-directory I/O, `NamedCheckpoint` commands, and CLI/desktop/web detection remain supported. Further QoL development is deferred; this does not remove file compatibility, recovery, or safety maintenance.

### R9.12 Categorical & continuous table-analysis completions (`QOL-12`)
Owner: maintainers · Status: done · Was: QOL-12, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md)) · Est. note: 0 h remaining

- [x] Exposed `cohens_kappa` (`Unweighted`, `Linear`, `Quadratic`) and `fleiss_kappa` on `ContingencyMethod` (`crates/doc/src/analysis.rs`, `crates/stats/src/categorical.rs`, `crates/engine/src/methods_report.rs`), inverted Newcombe `2×2` risk-difference bounds into Altman `NNTB`/`NNTH` CIs (`altman_nnt_from_rd`), emitted McNemar discordant-pair odds-ratio CIs (`mcnemar_detailed`), added custom expected proportions to `PartsOfWhole` goodness-of-fit $\chi^2$, and added exact one-sample Binomial (`ContingencyMethod::ExactBinomial`, $p_0 \in (0, 1)$) & paired Sign tests (`TwoGroupMethod::PairedSignTest`) (Wave 21).
- [x] Added exact $\chi^2$ standard-deviation confidence intervals (`sd_ci`) to `DescriptiveSummary` / `PooledSummaryResult` (`crates/stats/src/column.rs`), added direct pre-summarized `Mean + SD/SEM + N` support in `run_two_group_analysis` (`crates/stats/src/two_group.rs`), and verified `run_factorial_anova` main/simple-effect multiplicity tabs (`crates/stats/src/anova.rs`, shared with `R8.7`; `wave21_r97_r912_clustering`, `9 passed`, Wave 21).

### R9.13 Instant statistical, distribution & Bayesian calculators (`QOL-13`)
Owner: open · Status: deferred · Was: QOL-13, M8+ · Scheduling: no milestone assigned ([reference specification](calculators-and-quick-stats.md))

- Deferred reference scope: stateless statistical/distribution/Bayesian `calc.*` commands, scalar formula builtins, and a Calculators drawer. No subset is queued for implementation; detailed proposals remain in the reference specification.

### R9.14 Experimental subject randomization & random subset sampling (`QOL-14`)
Owner: maintainers · Status: done · Was: QOL-14, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md)) · Est. note: 0 h remaining

- [x] Added `crates/transform/src/randomize.rs` over `DeterministicRng` implementing `randomize_subjects_to_groups` (`AllocationScheme::{CompleteBalanced, PermutedBlock}`, `table.randomizeGroups`, writing seed/block provenance to `InfoSheet`), `random_sample_rows` / `TransformAnalysisSpec::RandomSample` (`table.randomSampleRows`, uniform & stratified $k$-of-$N$ row sampling without replacement), and `simulate_distribution_columns` (`table.simulateDistributions` across `Uniform`, `Gaussian`, `LogNormal`, `Binomial`, `Poisson`, `Exponential`), plus `RandomizeGroupsDialogState`, `RandomSampleRowsDialogState`, `SimulateDistributionsDialogState`, and `ui.table.{randomizeGroups,randomSampleRows,simulateDistributions}` (Wave 22).

### R9.15 Bench molarity, stock dilution & radiochemical decay calculators (`QOL-15`)
Owner: open · Status: deferred · Was: QOL-15, M8+ · Scheduling: no milestone assigned ([reference specification](calculators-and-quick-stats.md))

- Deferred reference scope: molarity, stock/serial dilution, moles/grams, and radiochemical calculators, including export-to-sheet conveniences. No subset is queued for implementation; detailed proposals remain in the reference specification.

### R9.16 Live best-fit $\TeX$ equation annotations (equations only, `QOL-16`)
Owner: maintainers · Status: done · Was: QOL-16, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md)) · Est. note: 0 h remaining

- [x] Implemented symbolic and live-substituted $\TeX$-subset best-fit model equation annotations (`crates/fit/src/tex_format.rs`, `crates/chart/src/equation_box.rs`, `GraphAnnotation::ModelEquation`, `InsertModelEquationDialogState` / `ui.graph.insertModelEquation`) across RGBA, SVG, and `%PDF-1.7`, recalculating automatically when linked fits update; significance grouping, pairwise-display modes, and additional fit-statistic callouts remain outside this item (`gui_w22b_qol_and_r9`, `9 passed`; `WF-49`; Wave 22b). Compact Letter Display is no longer excluded from the product but stays outside `R9.16`; it is tracked separately in [`R10.11`](#r1011-compact-letter-display-cld).

### R9.17 Extended multivariable, clustering, GLM & statistical-graphing `gui_only` acceptance suite (`QOL-17`)
Owner: maintainers · Status: done · Was: QOL-17, M8+ ([`gui-workflows.md`](gui-workflows.md#5-extended-21-workflow-multivariable-clustering-glm--statistical-graphing-gui_only-matrix-r917--qol-17-implemented)) · Est. note: 0 h remaining

- [x] Added Shards 5–7 (`crates/ui-egui/tests/gui_wf_multivariable_and_transforms.rs` [`8`], `gui_wf_clustering_glm_and_pharma.rs` [`7`], `gui_wf_advanced_statistical_graphs.rs` [`6`] = `21` extended workflows `wf29`–`wf49`, bringing the full suite to `49` workflows across Shards 1–7) under `GuiOnlyPolicy::Strict` covering conjunctive `Select and Transform` filtering, `MultipleVariables` formula-bar `Calculated Variables` (syntax/circularity guards & $2^{-\Delta\Delta C_t}$ qPCR), multi-schema `Extract and Rearrange`, direct `MultipleVariables` role-bound Welch *t* / Multifactor ANOVA / Kaplan–Meier / 4PL dose–response, full linkage/metric Hierarchical & K-Means clustering diagnostics, Cox PH & Poisson count GLMs, confidence ellipses/convex hulls, cross-tabulated `MultipleVariables` heatmaps, synchronized survival `Number at Risk` re-binning, bubble symbol sizing, and `R9.16` equations-only live $\TeX$ annotations while keeping `QOL-7` remediation actions excluded (Wave 23). Same scope limit as `R8.8`: these shards verify the semantic agent harness, not the rendered human GUI (see [`R10`](#r10-human-operable-gui--50-workflow-human-completion)); the CLD workflow becomes `WF-50` under [`R10.11`](#r1011-compact-letter-display-cld).

---

## R10 Human-operable GUI & 50-workflow human completion

**Sequencing:** Start new human-operable GUI implementation after the existing-capability MCP gate in [`R7.3`](#r73-mcp-workflow-parity-proof--shared-live-window) and its accessible README handoff in [`R7.4`](#r74-accessible-mcp-guide-and-minimal-readme), following `R0.18`. Existing claims and independent scientific fixes remain intact; future engine features receive MCP coverage when they land.

A 2026-11-15 audit of the rendered desktop window (`apps/lumencraft`) and web canvas (`apps/lumencraft-web`) found that **`0` of `50`** reference GUI workflows can be completed end to end by a person, although the engine and most dialog state machines behind them exist and pass the agent-harness suite. This milestone closes that gap and brings the GUI to full visual and interactive alignment with no external branding. Systemic blockers:

| Blocker | Summary | Items |
|---|---|---|
| `B1` | Modal dialogs and the Power wizard render as ≤6 static uppercase text lines (12 of the 26 dialogs show only a placeholder); no buttons, tabs, inputs, or selectors; overlay not hit-tested; `Enter` does not confirm | `R10.1`, `R10.16`, `R10.3` |
| `B2` | No reachable New Data Table / Welcome picker; menu and shortcut create a default 12×2 `XY` table only. `WelcomeWizardState`, `TableCreationDraft`, and built-in sample fixtures exist in `workspace_nav.rs` but are never rendered | `R10.4` |
| `B3` | Result sheets show ≤4 tabs × 6 rows × column 0 with no scrolling; sub-tabs not hit-tested; `Cmd+T` on a result sheet creates a blank table (the parameter-dialog intercept exists only in `gui_actions.rs`) | `R10.5` |
| `B4` | Desktop engine menu items dispatch `Value::Null` params and fail on required fields; the web host has no menu bar | `R10.6`, `R10.7` |
| `B5` | Toolbar, inspector, formula bar, column headers / role badges, and context menus are drawn but not interactive; the wheel zooms the canvas instead of scrolling the grid past ~30 rows; arrows and `Delete` do not navigate or clear | `R10.6`, `R10.8`, `R10.17`, `R10.18` |

**Root cause.** The interface is split three ways: `inspect.rs` invents virtual control coordinates, `compositor.rs` paints uppercase 5×7 bitmap wireframes, and `interaction.rs` hit-tests only a handful of regions. None of the three reads the others' geometry.

**Binding rules for every `R10` item:**

1. **Human-operable:** a step counts only when it is performed with raw pointer, wheel, keyboard, or IME events against widgets drawn in the frame the user sees, located by pixel coordinates or by an accessibility tree produced from the same layout pass that drew them. Structured params through `ui.invoke_menu`, field patches through `ui.answer_dialog`, and `inspect.rs` nodes without drawn counterparts do not count.
2. **Single retained layout & widget pass:** each frame computes one shared layout tree. Every toolbar button, navigator row, grid header and cell, scrollbar, result sub-tab, inspector control, and modal widget gets its geometry once, and pixel rendering, physical hit-testing, and `ui.inspect` all read that tree.
3. **Deterministic layout per host:** layout and rendering stay pure and deterministic for a given text-measurement source. Desktop and the headless screenshot/workflow runner share `lumencraft-text` shaping with system fonts, so they produce identical visuals and hit-boxes for the same font set. The web host measures and draws text with the browser's own canvas text rendering and fonts (decided 2026-11-15), so web text may differ from desktop pixel for pixel; widget structure, layout rules, and hit-box parity within the web frame still hold. No network access and no OS-widget divergence on any host.
4. **Clean-room & no external branding:** reference material may inform functional structure, visual hierarchy, and workflow ergonomics only. All wording, vector icons, schematic diagrams, and styling are original; no reference frames, icons, or copy enter the repository. LumenCraft's own branding is permitted everywhere; no external product, suite, or vendor branding (such as that visible in the reference video frames) appears on any visible or metadata UI surface (`R10.14`).

**Phases** (execution order in [Order and dependencies](#order-and-dependencies), Steps 11a–11f). An item listed both in an earlier phase and in Phase 7 lands in the earlier phase; Phase 7 enforces its gate. `R10.12` is split: its grid-side linked-region and exclusion styling belong to Phase 4, its `Import & Paste Special` dialog to Phase 5.

| Phase | Scope | Items |
|---|---|---|
| 1 | External-branding exclusion, proportional typography | `R10.14`, `R10.9` |
| 2 | Shared layout tree, performance budgets, reusable widgets, modal infrastructure, universal scrolling, aspect-ratio viewport | `R10.1`, `R10.22` (budgets set), `R10.16`, `R10.15`, `R10.2` (gate set-up) |
| 3 | Welcome launcher, contextual toolbar & menus, Navigator / Family / floating notes, web host chrome | `R10.4`, `R10.6`, `R10.17`, `R10.7` |
| 4 | Multi-tier data tables, calculated-variable editor, linked regions & exclusions | `R10.8`, `R10.12` (grid side) |
| 5 | Full multi-tab dialogs and wizards, Import & Paste Special | `R10.3`, `R10.12` (dialog) |
| 6 | Multi-column results, live right-hand inspector, typeset equations | `R10.5`, `R10.18`, `R10.19` |
| 7 | Verification & acceptance gates | `R10.2`, `R10.9`, `R10.14`, `R10.20`, `R10.21`, `R10.22`, `R10.13` |
| — | Missing analyses (engine work, can start any time) | `R10.10`, `R10.11` |

### R10.1 Interactive chrome architecture decision & spike
Owner: maintainers · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 16–24 h

- [x] **Decision resolved (2026-11-15):** implement the single retained layout & widget tree in-house in `lumencraft-ui-egui`, painted by `compositor.rs` with shaped text from `lumencraft-text` on desktop and headless runs; `egui` widgets are not used for chrome. Record the decision and the `UiApp` ↔ layout-tree boundary (including a text-measurement interface the web host implements) in [`architecture.md`](architecture.md) as part of the spike below.
- [x] **Decision resolved (2026-11-15):** fonts under the zero-bundled-fonts rule — desktop discovers system fonts (`LUMENCRAFT_FONTS_DIR` override); the web host measures and draws text with the browser's canvas text API and the browser's fonts; desktop/headless determinism is measured against a pinned, untracked test font set (`cargo xtask web-fonts`).
- [ ] Spike `AnalyzeSheet` + `EditAnalysisParameters` (one-way ANOVA) end to end on desktop and web; report web bundle size, `1M`-cell grid frame time, and `cargo xtask wasm`, `telemetry`, `licenses`, and `layers` results.
- [ ] Select the human-path test driver for `R10.2` and confirm any new dependency's licence with `cargo xtask licenses`.

### R10.2 Human-path acceptance harness & phantom-widget gate
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 30–48 h · Depends on: `R10.1`

- [ ] Add a human-path harness that drives the real frame only through raw pointer/wheel/key/IME events and the layout-tree accessibility view, with semantic widget-ID shortcuts disabled; fail on any use of `ui.invoke_menu` params, `ui.answer_dialog` field patches, or `try_activate_semantic_widget`.
- [ ] **Pixel-to-hit-box parity gate:** every control in the accessibility tree is visibly rendered within its parent's bounds, its text is neither clipped nor overlapping, and a raw `(x, y)` click at its centre has the same effect as the semantic automation action.
- [ ] Generate `ui.inspect` from the shared layout tree and route `gui_actions.rs` (`dispatch_gui_action`) through the same input path as `interaction.rs`, so agent and human handling (one keymap, e.g. `Cmd+T` on a result sheet) cannot diverge.
- [ ] Wire the harness and gate into `cargo xtask ci` and label the existing `gui_wf_*` shards as agent-harness tests in [`gui-workflows.md`](gui-workflows.md).

### R10.3 Interactive modal dialog & wizard framework (`B1`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 70–120 h · Depends on: `R10.16`

- [ ] **Analyze picker & disambiguation:** searchable, categorized analysis tree filtered by the active table archetype, plus the two-choice prompt (modify existing vs. create new result sheet) when re-analyzing a table.
- [ ] **Statistical & regression parameter dialogs:** full multi-tab layouts — Experimental Design (with original pairing/matching schematics), Repeated Measures, Multiple Comparisons, Options, Diagnostics/Residuals — and Nonlinear Regression (categorized expandable equation tree, Constraints, Initial Values, Confidence, Compare, live model formula preview).
- [ ] **Change Graph Type, Hook Constant & Export Figure:** plot-family thumbnail cards (original vector art) with live graph preview; expandable constant tree (`Info`, `Analysis`, `Document` constants) for live-linked text and equation annotations; Export dialog with format, resolution (DPI), physical size, background transparency, and colour space.
- [ ] **Four-stage Power & Sample Size wizard:** clickable step header, test-family cards, pilot-data effect-size calculator panel, fully rendered power-vs-sample-size plot with axes and hover callout, scrollable sensitivity table, and live sidebar sliders/spinners.
- [ ] Render the remaining `ActiveDialog` variants with real controls (`DuplicateAnalysisPrompt`, `ChangeDataAnalyzed`, `FormatSelectedPoints`, `ExcludeWithReason`, `ApplyStyleFrom`, `PageLayoutComposer`, `InsertModelEquation`, `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`, `Print`, `About`), replacing their static preview lines and the generic placeholder line (`ResultViewer` is covered by `R10.5`; `FormatGraph`, `FormatAxes`, `PairwiseComparisons`, and `BubblePlotInspector` by `R10.18`); render `R1`–`R10` modal predicates as disabled controls with `disabled_reason` tooltips from `modal_predicates.rs`.
- [ ] Human-path tests (`R10.2`) for each dialog: open from menu, toolbar, or shortcut, change a dependent control, confirm, and assert the resulting sheet.

### R10.4 Welcome & New Data Table dialog (`B2`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 20–32 h · Depends on: `R10.16`

- [ ] Replace the 3-line empty-workspace card with a two-pane launcher built on the existing `WelcomeWizardState`: left rail with all 8 table archetypes, open existing file, and `Clone a Graph / Pipeline` (`ClonePipelineDraft`); right pane with an original table/plot structural diagram, empty vs. synthetic tutorial dataset choice (existing clean-room sample fixtures), X-column mode, and Y replicate count or pre-computed error format (`Mean/SD/N`, etc.), plus `.lctemplate` templates.
- [ ] Route `Insert > New Data Table`, its shortcut, and launch with an empty project to the launcher instead of dispatching `sheet.createData` with `{}`; expose `Column` unpaired/paired hint and auto graph/info toggles through `TableCreationDraft`.
- [ ] Make `Convert Table Archetype…` (`ConvertArchetypeDialogState`) interactive with preview and lossy-change warning.

### R10.5 Scrollable multi-column result viewer (`B3`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 28–44 h · Depends on: `R10.16`

- [ ] Replace the 6-row × first-column summary with a full scrollable multi-column statistical table rendering every result column (estimates, differences, confidence bounds, test statistics, significance stars, adjusted *p*-values) with section headers, column sizing, and cell selection/copy.
- [ ] Clickable sub-sheet tabs with tab-strip overflow controls and keyboard switching (`Multiple comparisons`, `Residuals`, `Cleaned data`, `Clustered data`, `VIF`, `Classification table`, …).
- [ ] Expandable **Interpret / Assumption Checklist** side drawer and clickable diagnostic banners (diagnostics-only per `R9.7`).
- [ ] Route `Cmd/Ctrl+T` on a result sheet to `EditAnalysisParameters` in the human input path.

### R10.6 Menu, toolbar & context-menu command routing (`B4` desktop, `B5`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 28–44 h · Depends on: `R10.16`

- [ ] Replace the static 3-box header strip with a contextual toolbar of grouped icon-and-label buttons that adapt to the active sheet kind: File/Project, Sheet management, Undo/Redo, Clipboard, Analysis (Analyze, Parameters, Data Analyzed, Interpret), Graph/Layout (Change Graph Type, Axes, Pairwise Comparisons, Rulers, Alignment), Text styling, and Export/Print; icons are original vector assets with `ASSETS.md` rows.
- [ ] Replace `Value::Null` dispatch in `desktop_host.rs`: each menu or toolbar item either opens its parameter dialog or builds params from the active sheet and selection; add a registry sweep test that every engine command reachable from a menu, toolbar, shortcut, or context menu with required schema fields is routed through a dialog or a context-param builder.
- [ ] Draw and wire right-click context menus for grid cells, column headers, navigator entries, graph objects, and linked regions; make `Cmd/Ctrl+E` and `Exclude With Reason` use the current selection.
- [ ] Render the `Cmd/Ctrl+K` command palette with typing and keyboard selection.

### R10.7 Web menu bar, toolbar & chrome parity (`B4` web)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 20–32 h · Depends on: `R10.1`, `R10.6`

- [ ] Present the shared menu bar, toolbar, dialogs, and context menus in `lumencraft-web` from the same layout tree as desktop (binding rule 3).
- [ ] Map keyboard shortcuts with browser-reserved key handling, plus open/save via the existing `FileSystemAccessBridge` with download fallback.
- [ ] Re-verify the strict CSP, `cargo xtask telemetry` (`0` violations), and bundle size after the change.

### R10.8 Multi-tier data grid, navigation & calculated-variable editor (`B5`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 50–80 h · Depends on: `R10.16`, `R10.9`

- [ ] Multi-row column headers that reflect table structure: column/group identifier band, editable dataset title row, replicate subcolumn headers (`Y1, Y2, …` or `Mean, SD/SEM, N`), and super-column spans for grouped and nested tables; tabular-numeral cell text.
- [ ] Dedicated resizable, collapsible **Row Titles** column next to row numbers, and column-edge drag resizing.
- [ ] Wheel/trackpad scrolling and scrollbars in both axes (zoom only over graph/layout canvases); arrow, `Tab`, `Enter`, `Shift+Arrow`, and `Shift/Cmd+Arrow` boundary navigation; `Delete`/`Backspace` clearing.
- [ ] Clickable `MultipleVariables` variable-type badges (`Continuous`, `Categorical`, `Label`, `Calculated`) as dropdowns with lock/unlock, plus the `Variables…` level-recoding editor.
- [ ] Right-side **Calculated Variable** editor panel on the existing `FormulaBarState`: variable name, live formula input, syntax/circularity validation message, clickable column-variable list, searchable function reference with parameter hints, Apply/Cancel, and a formula-definition view toggle.

### R10.9 Proportional typography, Unicode coverage & glyph gate
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 16–30 h · Depends on: `R10.1`

- [ ] Retire the uppercase 5×7 ASCII glyph table across chrome, grids, results, inspectors, and modals; route all UI text through mixed-case proportional UI typography and tabular-numeral grid text from system fonts (zero bundled font files).
- [ ] Full Unicode coverage for Greek, mathematical symbols, sub/superscripts, accented Latin, CJK, and bidirectional RTL scripts, crisp at `1.0x`–`3.0x`.
- [ ] **Glyph coverage gate:** zero missing-glyph fallback boxes across all `16` locales, light/dark/high-contrast themes, and HiDPI densities, checked from the layout tree's shaped text runs on desktop and headless runs; on web, where the browser picks fallback fonts, covered by `R10.13` manual QA.
- [ ] Keep deterministic export screenshots (`cargo xtask screenshots --check`) working, regenerating `docs/images/*.png` and `ASSETS.md` rows when the chrome changes.

### R10.10 Standalone "Analyze a stack of *P* values" analysis
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 (missing analysis) · Est. note: 10–16 h

- [ ] Add a column analysis that takes a column of *P* values and reports adjusted values and discoveries for Bonferroni, Holm, Šídák, Holm–Šídák, Benjamini–Hochberg (1995), Benjamini–Krieger–Yekutieli two-stage (2006), and Benjamini–Yekutieli (2001), reusing the existing multiplicity code; cite each reference in doc comments.
- [ ] Define edge cases (`NaN`, values outside `[0, 1]`, empty column, ties) and add a `D1` oracle matrix against an independent implementation with documented tolerances.
- [ ] Register the engine command (`CommandSpec`, `cargo xtask commands`), add the `ANALYSIS_CATALOG` entry and parameter dialog, and regenerate generated docs.

### R10.11 Compact Letter Display (`CLD`)
Owner: maintainers · Status: todo · Was: previously excluded (`R9.16`/`QOL-16`); re-scoped by the human-GUI audit · Est. note: 20–32 h

- [x] **Decision resolved (2026-11-15):** maintainers confirmed reversal of the earlier product-scope exclusion of Compact Letter Display (required by the reference CLD workflow, `WF-50`). `R9.16` stays equations-only.
- [ ] Implement the Piepho (2004) insert-and-absorb + sweeping letter algorithm (*J. Comput. Graph. Stat.* 13(2):456–466, doi:10.1198/1061860043515) over existing all-pairs comparison results (Tukey, Games–Howell, Dunn, etc.), clean-room from the published description; `D1` oracle tests against the paper's published examples with documented tolerances; edge cases for all-different, all-equal, and single-group inputs.
- [ ] Add a graph annotation that places letters above groups (alternative to significance brackets), wired through `Pairwise Comparisons` in the `R10.18` inspector with SVG/PDF/RGBA rendering, and a `WF-50` workflow in [`gui-workflows.md`](gui-workflows.md).

### R10.12 Import & Paste Special and linked-region provenance
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 24–36 h · Depends on: `R10.16`, `R10.8`

- [ ] Multi-tab `Import & Paste Special` dialog extending `ImportReshapeMode`: `Source`, `View` (live parsed grid preview), `Filter` (row/column slicing, unstacking, plate-map reshape, exclusion markers), `Placement` (transpose and row-wrap), and `Notes`.
- [ ] Colour-coded `LinkedDataRegion` borders by upstream source kind, with double-click and right-click popovers to jump to the upstream sheet or unlink; block typing inside linked ranges with a `Go To Source` / `Unlink` prompt.
- [ ] Excluded cells drawn with clear strikethrough/tag styling and a hover tooltip showing exclusion provenance (`CellExclusionNote`).

### R10.13 Human-operable 50-workflow acceptance & manual QA
Owner: maintainers · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 50–80 h · Depends on: `R10.1`–`R10.12`, `R10.14`–`R10.22`

- [ ] Port `WF-01`–`WF-49` plus `WF-50` (CLD) to the `R10.2` human-path harness on desktop and web, using only physical `(x, y)` clicks, drags, scrolls, and keystrokes, covering all 5 protocol stages including large-`N` tables that need scrolling.
- [ ] Visual screenshot baselines of LumenCraft's own rendering for every sheet type and every modal tab, stored as hashes or as `ASSETS.md`-attributed PNGs, checked in CI.
- [ ] Run and record a manual walkthrough of all 50 workflows on macOS desktop and one browser, filing defects against `R10` items.
- [ ] Publish the per-workflow human-path status column in [`gui-workflows.md`](gui-workflows.md) and update [`ui-parity.md`](ui-parity.md) and [`STATUS.md`](STATUS.md) from observed results only.

### R10.14 External-branding exclusion sweep & gate
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 1, Phase 7); scope clarified 2026-11-15 · Est. note: 6–10 h

LumenCraft's own name, wordmark, icon, and metadata are permitted on every surface (toolbar badge, Welcome and About screens, window and browser titles, file-dialog labels, PDF creator/producer strings, packaging). This item keeps **external** branding out: no product names, logos, wordmarks, watermarks, icon designs, colour schemes, dialog copy, sample-dataset names, or tutorial captions taken from the reference application or any other vendor, including anything visible in the reference video frames.

- [ ] Review every UI surface added or restyled under `R10` against the reference frames used to design it, removing any external names, logos, icons, distinctive wording, or styling; record the review in the PR.
- [ ] **External-branding gate:** extend `cargo xtask brands` from source files to UI strings (`i18n` catalog, compositor literals, window/document/export metadata) and every text run in the rendered layout tree, failing on any external product, suite, or vendor name. Third-party file-format extensions (`.pzfx`, `.prism`) stay allowed only in import/export file-type lists, per [`AGENTS.md`](../AGENTS.md).
- [ ] Require every new icon, schematic diagram, thumbnail, and screenshot baseline to have an `ASSETS.md` row declaring it original (`cargo xtask assets`); no reference frames or derived images are committed.

### R10.15 Aspect-ratio-preserving canvas viewport
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 1) · Est. note: 10–16 h · Depends on: `R10.16`

- [ ] Render graph and page-layout sheets at their true physical page aspect ratio, centred on a neutral workspace mat, with zoom-to-fit, an explicit zoom-percentage selector, and pan/scrollbars, instead of stretching scenes to fill narrow split panes.
- [ ] Hide series-palette debug chips and layout-panel selection borders/badges unless the element is hovered or selected.

### R10.16 Reusable widget library, modal infrastructure & universal scrolling
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 2) · Est. note: 60–100 h · Depends on: `R10.1`, `R10.9`

- [ ] Widgets with hover, active, focused, and disabled states on the shared layout tree: push buttons, icon-and-label toolbar buttons, segmented/tab strips, radio groups, checkboxes, scrollable dropdowns, numeric spinners and sliders, single- and multi-line text fields (caret, selection, clipboard, IME), disclosure trees, splitters, and scrollbars.
- [ ] Modal z-order with pointer capture (no click-through), focus trapping and `Tab`/`Shift+Tab` cycling with visible focus rings, `Enter` to confirm, `Escape` to cancel; disabled tabs and controls show explanatory hover tooltips.
- [ ] Scroll containers with vertical/horizontal scrollbars and wheel/trackpad scrolling for navigator lists, data grids, result tables, info sheets, inspectors, and dialog trees, removing every fixed 4–8 item truncation cap.

### R10.17 Project Navigator, Family lineage tree & floating notes
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 3) · Est. note: 20–32 h · Depends on: `R10.16`

- [ ] Five sheet folders with collapsible disclosure arrows, original sheet-type icons, freeze/colour badges, live search, drag-reorder, and inline creation rows ("New Data Table…", "New Graph…", …).
- [ ] Family pane as an indented upstream-to-downstream dependency tree showing human sheet titles (not internal IDs) with active-sheet highlighting.
- [ ] Floating notes as draggable, collapsible callout cards overlaid on the sheet instead of a collapsed corner label.

### R10.18 Live right-hand inspector
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 6) · Est. note: 40–60 h · Depends on: `R10.16`

- [ ] Replace static inspector text with collapsible sections and sub-tabs: **Format Graph** (dataset/group selector, palette and colour pickers, symbol shape/size, bar/box/violin topology, error-bar direction and metric), **Format Axes** (range, interval, log/nonlinear scale, axis breaks), **Multivariable channels** (X, Y, Colour, Size, hull/ellipse toggles), **Pairwise Comparisons** (brackets, and CLD letters once `R10.11` lands), and **Page Layout** (grid arrangement, margins, plot-origin alignment, scale bars).
- [ ] Apply changes instantly to the canvas through engine commands with undo, and support data-point drill-back to the grid.

### R10.19 Typeset mathematical equation annotations
Owner: open · Status: todo · Was: GUI alignment plan 2026-11-15 (Phase 6) · Est. note: 16–28 h · Depends on: `R10.9`

- [ ] Upgrade `GraphAnnotation::ModelEquation` rendering from single-line text to two-dimensional math layout (stacked numerators/denominators, superscripts/subscripts, Greek parameters, substituted best-fit coefficients) across RGBA, SVG, and `%PDF-1.7`, citing the layout rules used (Knuth, *The TeXbook*, Appendix G).
- [ ] Deterministic rendering tests per output format, plus live coefficient refresh on upstream edits (extends `WF-49`).


### R10.20 Keyboard & screen-reader accessibility
Owner: open · Status: todo · Was: Roadmap survey 2026-11-15 · Est. note: 16–28 h · Depends on: `R10.16`

- [ ] Expose the shared layout tree to the OS accessibility APIs (VoiceOver, NVDA, Orca) on desktop, and to the browser accessibility tree on web.
- [ ] Gate: every interactive control has a role, a label, and a keyboard path; checked in CI from the layout tree.

### R10.21 Translations for new UI strings
Owner: open · Status: todo · Was: Roadmap survey 2026-11-15 · Est. note: 20–40 h · Depends on: all `R10` items that add strings

- [ ] Translate every label, tooltip, and dialog string added under `R10` into all `16` locales.
- [ ] Gate: `cargo xtask i18n-report` shows `0` missing keys before `R10.13` can pass.

### R10.22 UI performance & size budgets
Owner: open · Status: todo · Was: Roadmap survey 2026-11-15 · Est. note: 8–14 h · Depends on: `R10.1`

- [x] **Decision resolved (2026-11-15):** budgets — frame time ≤ `16.67 ms` for the full chrome plus a `1M`-cell grid; input-to-paint latency ≤ `50 ms` from click or keypress to updated frame; web release zip ≤ `6,243,591` B (1.5 × the `4,162,394` B `lumencraft-web-0.1.0.zip`); startup to first interactive frame < `3 s` on desktop and web, measured on the maintainers' reference macOS machine.
- [ ] Check the budgets in CI alongside the existing `R7.2` performance tests.

---

## Order and dependencies

| Step | Items | Depends on | Target gate |
|---|---|---|---|
| **Immediate next — R0.18** | Rust 1.99.1 minimum across all packages; pinned local/CI/coverage/packaging toolchains and verification | None; preserve existing claims | Build-toolchain baseline |
| **Before GUI — R7.3** | Fully functional MCP for existing engine capabilities: real stdio/headless workflow parity, protocol coverage, and opt-in shared visible session | `R0.18`; `R7.1`, `R8.3`, `R8.6` (done) | **MCP readiness before new `R10` GUI implementation** |
| **MCP onboarding — R7.4** | Short beginner MCP guide in a minimal, presentable README; verified first-run instructions after implementation | `R7.3` verified | Documentation handoff before GUI |
| **Step 1 (8A)** | `R0.1`, `R0.2` (done); foundations for `R0.3`, `R0.5`, `R0.6`, `R0.7`, `R0.10`, `R0.14` | — | Scaffold |
| **Step 2 (8B)** | `R0.3` (secrets gate), `R0.4` (pending/status), `R0.5` (licenses gate — done), `R0.6` (Zero Telemetry & local `actionlint`/`shellcheck` done; hosted GH Actions run open) | `R0.2` for `R0.4` | Scaffold |
| **Step 3** | `R0.10` (`lumencraft-platform` wiring & source checks), `R0.11` (versioned files) | `R0.6` | Scaffold |
| **Step 4** | `R0.8` (`CommandSpec` registry, area by area), then `R0.9` (structured errors & remediation) | — (runs beside Step 3) | Engine contract |
| **Step 5** | `R0.7` (generated `commands.md` & `schemas`), `R0.12` (format registry) | `R0.8` (commands), `R0.11` (schemas) | Docs & seams |
| **Step 6** | `R0.13` (MCP/CLI/UI alignment) | `R0.8`, `R0.9` | Surfaces |
| **Step 7** | `R8.3` (live `eframe`/`egui`/`wgpu` desktop window — **done**), `R8.4` (`wasm32` Trunk WebGL2 bundle — **done**) | `R8.1` (done) | **Alpha** (done; presenter only, human-operable chrome in `R10`) |
| **Step 7B** | `R8.6` (`ui.inspect`, `gui` MCP toolset & `--gui-only` lock — **done**), `R8.7` (Power & Sample Size wizard, 10 modal predicates & chained GUI surfaces — **done**), then `R8.8` (4-shard 28-workflow `gui_only` acceptance suite — **done**) | `R8.1` (done), `R0.13` (done) | **Agent-harness workflows** (`R8.6`–`R8.8` done) |
| **Step 8** | `R5.4` (native binary OS clipboard, `ActiveDialog::Print` & WYSIWYG layout drag-snap — **done**) | `R8.1` | **Beta** (done) |
| **Step 9** | `R8.5` (universal macOS `.dmg`/`.zip` `verify.sh` done locally; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA open), `R0.14` (`README.md` screenshots, `release.yml` `tags: ["v*"]`, `cargo-about`/`cargo-deny`, & local universal macOS/web packaging done; hosted `v0.1.0-rc1` CI tag/matrix open) | `R0.5` (done), `R8.3` (done), `R8.4` (done), `R8.8` (done), **`R10.13`** | **1.0** |
| **Step 10 (M8+)** | `R9.7` (diagnostics-only banners), `R9.12` (table-analysis completions), `R9.14` (randomization & sampling), `R9.16` (equation callouts only), `R9.17` (extended 21-workflow `gui_only` suite `WF-29`–`WF-49`); `R9.10`, `R9.11`, `R9.13`, and `R9.15` excluded from active sequencing | `R0.8` (done), `R3.1`–`R3.3` (done), `R4.4` (done), `R5.1` (done), `R8.8` (done) | **Done (active scope)** across Waves 20–23 (`0 h` remaining); deferred items unscheduled |
| **Step 11a (`R10` Phases 1–2)** | `R0.15` (evidence-backed done markers; can land now), `R0.16` (local coverage reports; can land now), `R10.1` (in-house layout tree & browser-font decisions made; spike), `R10.22` (performance & size budgets), `R10.14` (external-branding exclusion), `R10.9` (typography & glyph gate), `R10.16` (widgets, modals, scrolling), `R10.15` (canvas viewport), `R10.2` (human-path harness & pixel-to-hit-box gate) | `R7.3` (existing-capability MCP gate), `R7.4` (README handoff), then `R8.3`, `R8.4`, `R8.6` (done) | **Human GUI workflows** |
| **Step 11b (`R10` Phase 3)** | `R10.4` (Welcome launcher), `R10.6` (contextual toolbar, menu & context routing), `R10.17` (Navigator, Family tree, floating notes), `R10.7` (web host chrome) | `R10.16` | **Human GUI workflows** |
| **Step 11c (`R10` Phases 4 & 6)** | `R10.8` (multi-tier grid & calculated-variable editor), `R10.12` grid side (linked regions & exclusions), `R10.5` (multi-column results), `R10.18` (live inspector), `R10.19` (typeset equations) | `R10.16`, `R10.9` | **Human GUI workflows** |
| **Step 11d (`R10` Phase 5)** | `R10.3` (multi-tab dialogs, Power wizard, Graph Type / Hook / Export), `R10.12` dialog side (Import & Paste Special dialog) | `R10.16`, `R10.6` | **Human GUI workflows** |
| **Step 11e (`R10` Phase 7)** | `R10.20` (keyboard & screen-reader gate), `R10.21` (translation gate), `R10.13` (pure mouse/keyboard 50-workflow acceptance, screenshot baselines, manual QA) | all other `R10` items | **Human GUI workflows** -> **1.0** |
| **Step 11f (`R10`, parallel engine work)** | `R10.10` (stack of *P* values), `R10.11` (CLD, scope confirmed) | none for engine parts; dialog parts need `R10.3`, `R10.18` | **Human GUI workflows** |
| **Step 11h (parallel, statistics)** | `R3.5` (statistical method fidelity), `R4.6` (fitting method fidelity), `R4.5` (robust-mode inference suppression), `R0.17` (clean-room provenance review), `R3.4` (unstructured REML covariance) | `R4.1`, `R3.2` (done) | **1.0** (`R3.5`, `R4.6`, `R4.5`, `R0.17`); unscheduled (`R3.4`) |

**Active `**decision**` markers:**
- **Shared layout tree implementation (`R10.1`):** **Decision resolved (2026-11-15):** in-house retained layout tree rendered by `compositor.rs`; web text uses the browser's canvas text rendering and fonts; `R10.22` budgets set (≤ `16.67 ms` frame, ≤ `50 ms` input-to-paint, web zip ≤ 1.5× current, startup < `3 s`).
- **Compact Letter Display scope (`R10.11`):** **Decision resolved (2026-11-15):** CLD is back in product scope as `R10.11` so the CLD reference workflow (`WF-50`) can be completed; `R9.16` remains equations-only.
- **Plugins (`Parked`):** **decision** — dynamic plugin loading / third-party plugin SDKs stay out of `1.0` scope so the `L0`–`L7` workspace, `#![forbid(unsafe_code)]`, and clean-room guarantees stay self-contained.
- **SonarCloud (`R0.6`):** **Decision resolved (Zero-Telemetry policy):** SonarCloud and all external cloud scanners/telemetry services are permanently excluded; `cargo xtask telemetry` enforces zero telemetry/egress/analytics and workflow `permissions` / `${{ }}` hygiene inside `cargo xtask ci`.

---

## Legacy Milestone & Gate Mapping

Legacy `G1`–`G14` parity gaps and the preserved `#known-defects` table live in [`gaps.md`](gaps.md). The tables below map historical milestone, spike, and release-gate IDs onto this roadmap (and [`STATUS.md`](STATUS.md)).

### Legacy Milestones & Spikes

| Legacy ID | Legacy Title / Scope | Mapped Roadmap ID(s) | Current Status |
|---|---|---|---|
| `Slice 0` | Thin end-to-end vertical walking skeleton (archive -> unpaired *t* -> SVG/`%PDF-1.7` -> CLI) | [`R7.1`](#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server) | Done (Wave 1) |
| `S1` | UI performance risk spike (`1M`-cell virtual grid & `UiApp` frame budget, **GO**) | [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost) | Done (**GO**, `0.061 ms` median frame) |
| `S2` | Vector typography & PDF outline/embedding risk spike (`%PDF-1.7` + `/FontFile2` + Poppler/GS QA) | [`R5.1`](#r51-scientific-scene-compiler-rich-typography-nonlinearbroken-axes--brackets)–[`R5.2`](#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export) | Done (Waves 2–5) |
| `M0` | Workspace skeleton, `xtask` gates, clean-room icons & packaging manifests | [`R1.1`](#r11-layered-workspace--clean-room-quality-gates) | Done (Wave 1) |
| `M1` | Numerical foundations (`math`, `testkit`, NIST StRD/`D1`), `palette`, `doc` archetypes & `format`/WAL | [`R1.2`](#r12-ieee-754-foundations-special-functions-linear-algebra--nist-strd--d1-corpus), [`R2.1`](#r21-palettes-8-typed-table-archetypes-subcolumns--dependency-links)–[`R2.2`](#r22-lumencraft-archive-wal-crash-recovery--lumencraftd-unpacked-format) | Done (Waves 1–5) |
| `M2` | Core biostatistics (`t`-tests, ANOVA, RM/REML), transforms & reactive DAG engine | [`R2.3`](#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation), [`R3.1`](#r31-descriptive-statistics-normality-battery-outliers--two-group-tests)–[`R3.2`](#r32-one-two-three-way-anova-rm-reml-mixed-models--multiplicity), [`R4.4`](#r44-derived-table-transforms) | Done (Waves 2–6) |
| `M3` | Nonlinear regression (`LM` solver, weighting, ROUT, global fits, `117` built-in equations) | [`R4.1`](#r41-damped-lm-solver-weighting-robustrout-shared-globals-profile-cis--aiccflrt)–[`R4.2`](#r42-built-in-equation-library-117-models-across-all-categories) | Done (Waves 2–5) |
| `M4` | Publication graphing, rich text, broken axes, SVG/`%PDF-1.7`/PNG/TIFF/`.emf` & PDF viewer QA | [`R5.1`](#r51-scientific-scene-compiler-rich-typography-nonlinearbroken-axes--brackets)–[`R5.2`](#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export) | Done (Waves 2–5) |
| `M5` | Headless CLI, `--control`, MCP, `UiApp` + live OS `eframe`/`wgpu` windowing (`R8.3`) & `wasm32`/Trunk WebGL2 bundle (`R8.4`) | [`R7.1`](#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server), [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost), [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop)–[`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | Done as presenter (live `eframe`/`egui`/`wgpu` desktop window `gui_window.rs` & `wasm32`/Trunk `web-release` bundle verified locally); human-operable chrome open in [`R10`](#r10-human-operable-gui--50-workflow-human-completion) |
| `M6` | Survival, Cox PH, Barnard/Koopman, ROC, MV/Calculated Vars, custom DSL/ODE, `.prism`/`.pzfx`, Layouts & `Beta` clipboard/print/drag | [`R2.4`](#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget), [`R3.3`](#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice), [`R4.3`](#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm), [`R5.3`](#r53-headless-page-layouts-style-cloning-spool-pdf--subprocessmacos-clipboard)–[`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap), [`R6.1`](#r61--calculated-variables-role-locking--level-ordering)–[`R6.2`](#r62-multifactor-anova-mv-groupingsplits--five-channel-mv-plots), [`R7.2`](#r72-performance-budgets-1m-cell-virtualization--propertyfuzz-suites) | Done (Waves 2–13; `R5.4` macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux vector clipboard adapters, `ActiveDialog::Print`, & WYSIWYG layout drag-snap landed) |
| `M7` | `16`-locale i18n/RTL, macOS `pbcopy`/`pbpaste`/`NSPasteboard` & codesign, cross-OS hardware/CJK desktop QA & signed installers | [`R8.2`](#r82-16-locale-i18n-rtl-mirroring--desktop-cjkrtl-fontpasteboard-checks), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) | Core/macOS/PDF/`cjk_rtl_hidpi_desktop_qa` + universal macOS `.dmg`/`.zip` `verify.sh` & `THIRD-PARTY-LICENSES.html` done; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA open (`R8.5`) |
| `M8+` | Active QoL: diagnostics-only banners, table-analysis completions, randomization, equation callouts & extended `gui_only` workflow suite; house-principles alignment (`R0`) | [`R0`](#r0-align-with-house-principles), [`R9`](#r9-post-parity-scientific-quality-of-life-suite) | `R9` active scope done (`R9.1`–`R9.9`, `R9.12`, `R9.14`, `R9.16`, `R9.17`); `R9.10`, `R9.11`, `R9.13`, `R9.15` deferred without milestones; `R0.6` & `R0.14` pending external CI |

### Release Gates

| Release Gate | Required Roadmap Items | Status & Remaining Work |
|---|---|---|
| **Alpha** (first visual desktop & WASM release) | [`R1`](#r1-workspace-numerical-foundations--nist-strd-oracle)–[`R7`](#r7-headless-automation-cli---control-mcp--performance-budgets) headless parity + [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost) + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) + [`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | **Done** (`R8.3` `eframe`/`egui`/`wgpu` native windowing & `R8.4` `wasm32` Trunk WebGL2 bundle `lumencraft-web-0.1.0.zip` verified) |
| **Agent-harness workflows** (formerly **Core workflows**) | **Alpha** + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) event loop + [`R8.6`](#r86-semantic-ui-inspection-gui-mcp-toolset--strict---gui-only-lock)–[`R8.8`](#r88-clean-room-28-workflow-gui_only-acceptance-suite) semantic `gui_only` workflow parity | **Done for the agent harness only** (`ui.inspect`, 8-tool `gui` MCP mode, `--gui-only`, Power wizard state machine, `R1`–`R10` modal predicates, `FormatSelectedPoints`, `ResultSheet` `Analyze` chaining, & 28-workflow `gui_wf_*` suite). Not a human-usability gate. |
| **Human GUI workflows** | **Alpha** + [`R10.1`](#r101-interactive-chrome-architecture-decision--spike)–[`R10.22`](#r1022-ui-performance--size-budgets) | **Open** — `0 / 50` reference workflows human-completable on desktop or web (audit 2026-11-15); blockers `B1`–`B5` plus 2 missing features (`R10.10`, `R10.11`) |
| **Beta** (OS vector clipboard/print/layout drag) | **Agent-harness workflows** + [`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) | `R5.4` **done** (live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux adapters, `ActiveDialog::Print`, WYSIWYG layout drag-snap) |
| **1.0** (human-operable GUI, full signed installers & target-OS QA) | **Beta** + **Human GUI workflows** + [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) + [`R0`](#r0-align-with-house-principles) release gates | **Human GUI workflows (`R10`) open.** Local `R0.5`, `actionlint`/`shellcheck`, `README.md` screenshots, `release.yml` `tags: ["v*"]`, `cargo-about`/`cargo-deny`, `lumencraft-web-0.1.0.zip`, and universal macOS `.dmg`/`.zip` `package.sh` + `verify.sh` done; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA (`R8.5`), hosted GH Actions CI (`R0.6`), & `v0.1.0-rc1` tag/matrix (`R0.14`) open |

---

## Parked

Items explicitly deferred until needed or post-`1.0`:

- **Translated READMEs:** localized `README.<locale>.md` translations are product polish deferred beyond initial `1.0` release scaffold (`R0.14`).
- **Mutex wrapper & locks check:** `clippy.toml` already disallows raw `std::sync::Mutex`; a deadlock-detecting lock wrapper and `xtask locks` check stay parked while the workspace remains single-owner / message-passing.
- **Plugin SDK / dynamic plugins:** deferred (`**decision**`; closed domain seams use typed traits and registries inside the workspace).
