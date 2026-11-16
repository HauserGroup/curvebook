# LumenCraft Roadmap

Claimable engineering roadmap for **LumenCraft**. For current verified capabilities, test evidence, and known limits, see [`STATUS.md`](STATUS.md). For historical wave-by-wave progress logs, see [`roadmap-history.md`](roadmap-history.md).

**Toolchain baseline:** [R0.18 — Upgrade the workspace and build toolchains to Rust 1.99.1](#r018-upgrade-the-workspace-and-build-toolchains-to-rust-1991) is **done locally** (`rust-toolchain.toml` + `rust-version = "1.99.1"` across all `22` workspace packages; `cargo xtask ci`, `xtask coverage -p lumencraft-math`, `packaging/web/package.sh`, and `xtask bundle` + `verify.sh` pass; hosted CI/FreeBSD/Windows ARM64 runners and `hdiutil` DMG creation open).

**Next feature priority:** [R7.3 — Fully functional MCP for existing engine capabilities](#r73-mcp-workflow-parity-proof--shared-live-window) and [R7.4 — Accessible MCP guide and minimal README](#r74-accessible-mcp-guide-and-minimal-readme) are **done** (`WF-01`–`WF-50` stdio/headless parity, `lumencraft-mcp` `60`, `lumencraft-cli` `69`, `lumencraft` `51`, minimal `README.md` with verified beginner stdio MCP walkthrough, reconciled `docs/mcp.md`, and `lumencraft-win95-2508.png` icon). Complete [R0.20 — Rename LumenCraft to CurveBook](#r020-rename-lumencraft-to-curvebook) next, as a requirement for the **MCP Alpha launch** and before further unclaimed [`R10`](#r10-human-operable-gui--50-workflow-human-completion) GUI implementation. Alpha launch also requires [R0.21 — Strict Alpha branding and asset-provenance gate](#r021-strict-alpha-branding-and-asset-provenance-gate). Preserve already-landed R10 work, existing claims, and independent audit/scientific fixes; coordinate overlapping changes before the cutover.

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
| **R0** | Align with house principles & typed engine scaffold | Phases A–E | in progress | `R0.1`–`R0.5`, `R0.7`–`R0.13`, `R0.15` (evidence-backed done markers), `R0.17` (clean-room Lilliefors and Bessel/hypergeometric functions), `R0.18` (Rust `1.99.1` minimum, `rust-toolchain.toml`, & CI/packaging pin alignment done locally; hosted CI/FreeBSD/Windows ARM64 runners open) done; `R0.16` (local `cargo-llvm-cov` coverage reports) in progress; `R0.6` (Zero-Telemetry policy, local `xtask telemetry`, `xtask workflows`, & cross-OS `os_integration.rs` cfg gates done; live GH Actions runner execution open), `R0.14` (clean-room PNG screenshots, `.github/workflows/release.yml` `tags: ["v*"]`, `cargo-about` `THIRD-PARTY-LICENSES.html` + `cargo-deny`, web `.zip`, and local universal macOS `.dmg`/`.zip` `package.sh` + `verify.sh` done; hosted `v0.1.0-rc1` CI tag/matrix open) in progress; `R0.19` (Round 2 delta audit remediation: Wave A `S01`–`S09`, `S11`–`S13`, `desktop_host` landed on `main`; `S10` + `interaction-gui_actions` + 2 cross-crate seams scheduled after `R0.20`) in progress; `R0.20` (CurveBook cutover before remaining unclaimed `R10` work; next active launch) in progress; `R0.21` (strict Alpha branding/asset-provenance clearance) todo |
| **R1** | Workspace scaffold, numerical foundations & NIST StRD oracle | `M0`, `M1` (partial), `G1` | done | `58` NIST StRD datasets + `D1` R `4.5.2` oracle matrices verified (`0 h`) |
| **R2** | Typed data tables, reactive DAG, persistence & `.prism`/`.pzfx` import | `M1`, `M2`, `G2`, `G10` | done | Waves 1–6 incl. `.lumencraft.d/`, `G2` & `G10` fixes verified (`0 h`) |
| **R3** | Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate | `M2`, `M6`, `G3`, `G8` | done | `R3.1`–`R3.5` done: Waves 2–5 incl. `BarnardExact`, `PassingBablok`, MICE, `G8` Koopman fix, plus `R3.4` unstructured Cholesky REML covariance/Satterthwaite/KR and `R3.5` statistical method-fidelity oracle suites |
| **R4** | Nonlinear regression solver, `117` equations & custom DSL/ODE | `M3`, `M6`, `G4`, `G9` | done | `R4.5` (robust-mode inference suppression) & `R4.6` (fitting method fidelity) done; `R4.1`–`R4.4` done: Waves 2–5 incl. `47` pharma models, `MAX_AST_NODES`, iterative `Expr` |
| **R5** | Publication graphing, vector SVG/PDF 1.7/TIFF/EMF & Page Layouts | `M4`, `M6`, `S2`, `G5`, `G11` | done | `R5.1`–`R5.4` & `G5` done (Waves 2–13): Poppler+Ghostscript PDF QA, live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux vector clipboard adapters, `ActiveDialog::Print` + OS printer discovery/preview, and live WYSIWYG layout drag-snap/resize grips |
| **R6** | Multiple Variables workflows, Calculated Variables & Multifactor ANOVA | `M6`, `G7` | done | Waves 2–4 end-to-end verified (`0 h`) |
| **R7** | Automation, headless CLI, `--control`, MCP & performance budgets | `M5` (headless), `M6`, `Slice 0`, `G12`, `G14` | done | `R7.1`–`R7.4` done (`R7.3` stdio JSON-RPC `WF-01`–`WF-50`, headless `command_run`/`command_batch`, full MCP surface, & opt-in `--control <port> --window` shared session verified; `R7.4` minimal `README.md`, beginner stdio MCP guide, & reconciled `docs/mcp.md`; `97` engine commands, `96`-curve 4PL `< 50 ms`, `G12` read-only skip & bounded undo); ready for `R10` GUI handoff |
| **R8** | Live desktop & WASM UI shell, GUI-agent parity, localization QA & `1.0` hardening | `M5` (UI), `M7`, `S1`, `G6`, `G13` | in progress | `UiApp`/HiDPI/RTL/`WebWorkspaceHost`, `eframe`/`egui`/`wgpu` native window **presenting the CPU RGBA buffer as a texture** (`R8.3`), `wasm32` Trunk WebGL2 **textured-quad presenter** (`R8.4`), universal macOS `.dmg`/`.zip`, `cjk_rtl_hidpi_desktop_qa`, & **semantic agent-harness** parity/Power wizard state machine/28-workflow suite (`R8.6`–`R8.8`) done; Windows/Linux/FreeBSD VM QA, Apple Developer ID notarization, & live browser QA (`R8.5`) open. **The rendered chrome is not human-operable; see `R10`.** |
| **R9** | Scientific quality-of-life scope | `M8+`, `QOL-1`–`QOL-17` | done (active scope) | Wave A `QOL-1`–`QOL-9` (incl. `R9.7` diagnostics-only banners) plus Wave B `R9.12` (table-analysis completions), `R9.14` (randomization, sampling & distribution simulation), `R9.16` (equations-only live $\TeX$ callouts), and `R9.17` (`WF-29`–`WF-49` Shards 5–7, `49` total agent-harness `gui_only` workflows) done (`0 h`); `R9.10`, `R9.11`, `R9.13`, and `R9.15` deferred without scheduled milestones ([scope](qol-beyond-parity.md)) |
| **R10** | Human-operable GUI & 50-workflow human completion (desktop + web) | Human-GUI audit 2026-11-15 | in progress | Baseline audit found **`0` of `50`** reference workflows completable by a person (`B1`–`B5`). Engine/dialog state machines (`50` of `50`: `R10.10`, `R10.11` done), `R10.14` Phase 1 (`xtask brands`/`assets`), `R10.9` L2 shaping, `R10.19` 2D math box, `R10.6`/`R10.7` command routing, and `R10` Phase 2 core infrastructure (`4e6b8b2`: `LayoutTree`, glyph atlas, aspect-ratio viewport, `HumanPathDriver`, `xtask r10-gate`, ANOVA spike) landed; full 50-workflow interactive surfaces (`R10.3`–`R10.8`, `R10.12`–`R10.13`, `R10.17`–`R10.21`) remain open. Blocks **1.0**. |

**Gate summary:** **Alpha launch is open:** full MCP functionality for the implemented engine scope (`R7.3`, done), accessible MCP onboarding (`R7.4`, done), the full CurveBook cutover (`R0.20`, todo), and strict branding/asset-provenance clearance (`R0.21`, todo) are required. Known bugs are acceptable when documented; completing all audit remediation is not an Alpha prerequisite. Branding, naming, or asset-provenance violations cannot be waived as known bugs for launch. Desktop/WASM presenters and **Agent-harness workflows** remain recorded implementation achievements. **Human GUI workflows** (`R10`) is a separate, open milestone and is not required for Alpha. `R5.4` clipboard/print/layout-drag capabilities are implemented, but that does not establish a launched Beta. **1.0** still requires human GUI acceptance and the external CI/notarization/hardware-VM release gates (`R8.5`, `R0.6`, `R0.14`).

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
- [ ] Verify CI workflows (`cargo xtask ci` including `xtask workflows`) pass cleanly on live GitHub Actions runners (local `actionlint 1.7.12`, `shellcheck 0.11.0`, `xtask workflows`, `os_integration.rs` cross-OS/wasm32 cfg gates, and `mkdir -p "$RUNNER_TEMP/actionlint-bin"` SHA-256 CI fix landed locally; remote GitHub Actions runner execution remains unexecuted locally).

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
Owner: maintainers · Status: done · Was: Human-GUI audit 2026-11-15 (process fix) · Est. note: 4–8 h

- [x] Added Pitfall #6 (`Treating Semantic Agent-Harness Tests as Proof of Human GUI Parity`) to [`pitfalls.md`](pitfalls.md), requiring passing human-path/unit test names or recorded manual runs for human-operable claims and rejecting agent-harness tokens (`--control`, `--event-script`, `dispatch_gui_action`, `gui_wf_*`, `gui_only*`, etc.).
- [x] Added `check_ui_parity_evidence` and `check_gui_workflows_human_path` in [`xtask/src/status.rs`](../xtask/src/status.rs) (executed by `cargo xtask status`), audited [`ui-parity.md`](ui-parity.md) and [`gui-workflows.md`](gui-workflows.md) honestly (`xtask` **`96 passed`** incl. `status::evidence_tests` `8` tests, `cargo +1.99.1 xtask status`, plus `lumencraft` `gui_window::tests` `4` & `desktop_host_loop` `7`, `lumencraft-ui-egui` `audit_r1_s4b_ui` `3` / `cjk_rtl_hidpi_desktop_qa` `4` / `grid_perf_s1` `10` / `interaction_hidpi_qol` `9`, and `lumencraft-tools` `spike_s1_grid_perf` `3`).

### R0.16 Local source-coverage reports (`cargo-llvm-cov`)
Owner: open · Status: in progress — claimed: jsture, branch `chore/R0.16-llvm-cov` · Was: coverage tooling review 2026-11-15 · Est. note: 2–4 h

Coverage is a diagnostic for finding untested edge-case and error paths (`NaN`/`±Inf`, empty input, malformed files). It is never a substitute for `D1` oracle checks and has no percentage gate.

- [x] Add `cargo xtask coverage [-p <crate>]... [-- <test args>]` wrapping `cargo llvm-cov` (nextest when installed), excluding `xtask` and `lumencraft-testkit`, writing `lcov.info` and an HTML report under the coverage target directory, and printing an install hint when `cargo-llvm-cov` or `llvm-tools-preview` is missing (`xtask/src/coverage.rs`, `5` unit tests; local runs on `lumencraft-math` and `lumencraft-format`).
- [x] Add `.github/workflows/coverage.yml` (manual and weekly, not a PR gate) that runs `cargo xtask coverage` and keeps the report as a workflow artifact only; no third-party coverage upload service (local `cargo xtask workflows` + `cargo xtask telemetry` pass).
- [ ] Confirm a green hosted run of `coverage.yml` (`workflow_dispatch`) once it is on `main`.
- [x] Document the command and its policy (no thresholds, no hand-written coverage percentages) in [`development.md`](development.md) and [`AGENTS.md`](../AGENTS.md) §4.

### R0.17 Clean-room provenance review: Lilliefors p-values & Bessel functions
Owner: maintainers · Status: done · Was: `D1` citation pass 2026-11-15 · Est. note: 6–12 h

The `D1` citation pass could not trace two pieces of numerical code to a freely usable published source.

- [x] Replaced unattributed quartic polynomials and ad-hoc `n > 100` `(n/100)^0.49` rescaling in both `crates/fit/src/diagnostics.rs` and `crates/stats/src/column.rs` with shared clean-room `lilliefors_d_from_sorted` & `lilliefors_p_value` in [`crates/math/src/dist/lilliefors.rs`](../crates/math/src/dist/lilliefors.rs) (Lilliefors 1967, Stephens 1974 `sqrt(n)`/`u = 1/sqrt(n)` scaling, original `22_620_000`-trial seeded Rust table across `n = 5..=4000` with quadratic `u -> 0` asymptotic limit and monotone PCHIP in `(W, ln P)`; hold-out validation `max |p_hat - p| = 0.001500` across `p in [0.001, 0.95]`), verified by `dist::lilliefors::tests::{table_rows_are_strictly_increasing_and_reject_invalid_inputs, p_value_is_strictly_monotone_in_d_and_matches_dallal_wilkinson_table1}`, `r017_cleanroom_oracles::r017_lilliefors_fit_diagnostics_parity_and_large_n`, and `r017_lilliefors_oracles::{r017_lilliefors_edge_cases_invariance_and_math_primitive_parity, r017_lilliefors_dallal_wilkinson_1986_and_lilliefors_1967_table1_oracles, r017_lilliefors_independent_seeded_mc_calibration_including_large_n}`.
- [x] Removed Numerical Recipes §6.5 rational polynomials and Miller downward recurrence in [`crates/fit/src/dsl.rs`](../crates/fit/src/dsl.rs), implementing NIST DLMF Ch. 10 / Thompson & Barnett (1986) 106-bit `Dd` `bessel_j0_y0_series`/`bessel_j1_y1_series` (`|x| <= 20`), Hankel asymptotic expansions (`|x| > 20`), Steed/Lentz `bessel_cf1_jprime_over_j` + Wronskian normalization for `bessel_jn`/`bessel_yn`, plus documented domain/accuracy bounds (`<= ~1e-13` relative when accepted for `hyp1f1`, `<= ~1e-12` for `confluent_u`, exact DLMF §13.2.9/§13.2.40 integer-`b` digamma series & reflection, DLMF §13.7.2/§13.7.3 `hyp2f0_asymp`, DLMF §13.4.4 Laplace Gauss–Kronrod quadrature), verified by `r017_cleanroom_oracles::{r017_bessel_j0_j1_y0_y1_wronskians_zeros_and_quadrature, r017_bessel_jn_yn_wronskians_recurrences_and_jacobi_anger_identities, r017_hyp1f1_and_confluent_u_identities_integer_b_and_large_z}`.

---

### R0.18 Upgrade the workspace and build toolchains to Rust 1.99.1
Owner: maintainers · Status: done · Priority: completed baseline · Depends on: none · Was: Rust toolchain upgrade decision 2026-11-15

Rust 1.99.1 is the minimum supported version and the exact local rustup/CI/packaging pin (edition 2024, resolver 3, application version, `Cargo.lock`, dependencies, numerical tolerances, and runtime behavior preserved; hosted GitHub Actions CI, FreeBSD VM, Windows ARM64 runners, and macOS `hdiutil` DMG creation remain unexecuted locally and no release tags were triggered). Existing work claims are preserved; the CurveBook rename is now scheduled separately as [`R0.20`](#r020-rename-lumencraft-to-curvebook).

- [x] Set `[workspace.package] rust-version = "1.99.1"` and added `rust-version.workspace = true` to every member package that did not already inherit it, including libraries, CLI, testkit, and `xtask` (`cargo metadata --locked --no-deps --format-version 1` confirms `22`/`22` packages report `rust_version: "1.99.1"`).
- [x] Added `rust-toolchain.toml` with `channel = "1.99.1"`, `profile = "minimal"`, and `components = ["rustfmt", "clippy"]`; kept additional targets job-specific and exercised `wasm32-unknown-unknown` plus matching `llvm-tools-preview` via `cargo xtask ci` and `cargo xtask coverage -p lumencraft-math`.
- [x] Updated `.github/workflows/{ci,coverage,freebsd,release,windows-arm64}.yml` toolchain pins, switched FreeBSD VM workflows to rustup Rust `1.99.1` instead of rolling system `pkg` Rust, preserved `release.yml` FreeBSD `R2-S13-02` `env:` passing, and included `Cargo.toml`, `Cargo.lock`, and `rust-toolchain.toml` in the FreeBSD/Windows ARM64 PR path filters (verified locally by `cargo xtask workflows`: `actionlint` across `6` workflows and `shellcheck` across `14` shell scripts; hosted CI/FreeBSD/Windows ARM64 runners not executed).
- [x] Updated current prerequisites, README badge, contributor/governance instructions (`AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `docs/development.md`), and `Cargo.toml` lint comment while preserving historical evidence, `README.md`'s seven-pane logo alt text, and `docs/development.md`'s `magick` note; documented rustup proxy selection and checking `rustc --version`, `cargo --version`, and `rustup show active-toolchain`, including when Homebrew binaries take precedence on `PATH`.
- [x] Preserved `Cargo.lock` cleanly without unrelated refactors, dependency upgrades, or broader lint suppressions; `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass cleanly under Rust `1.99.1`.
- [x] Confirmed automatic `1.99.1-aarch64-apple-darwin` selection (`rustc 1.99.1` / `cargo 1.99.1`) in repo root, `crates/math`, `apps/lumencraft-cli`, and `xtask`, all `22` inherited `rust_version: "1.99.1"` metadata entries, and clean `cargo +1.99.0 check -p lumencraft-math --offline` rejection (exit `101`: `requires rustc 1.99.1`). Verified on `main`: `cargo xtask ci` (`ci: all checks passed` — `fmt`, `clippy`, `1,456` non-doc tests across `194` suites + `4` doctests across `20` crates = `1,460` passed, `--no-default-features` for `text`/`ui-egui`/`mcp`, `wasm32`, and all `xtask` gates), `cargo xtask coverage -p lumencraft-math` (`62` tests; `87.0%` lines `4677/5378`, `92.3%` functions `394/427`, `87.2%` regions `10391/11911`), `./packaging/web/package.sh` (Trunk `web-release` build, `lumencraft-web-0.1.0.zip` `3.9M`), and `cargo xtask bundle` + `./packaging/macos/verify.sh dist/LumenCraft.app` (`valid on disk`).

---

### R0.19 Round 2 delta code audit remediation (`ebcc622..fda1cc0`)
Owner: open · Status: in progress · Was: [`docs/audit/round2-post-ebcc622.md`](audit/round2-post-ebcc622.md), [`docs/gaps.md`](gaps.md#round-2-delta-audit-open-findings-on-main-1e9dc59)

Remediate the `149` surviving open items from the Round 2 delta audit after the `2026-11-16` re-check against `main` (`1e9dc59`): **`135` `R2-*` survivors** (`36` `S1`, `66` `S2`, `31` `S3`, `2` `S4` across [`docs/audit/findings/r2/R2-S01.md`](audit/findings/r2/R2-S01.md)–[`R2-S13.md`](audit/findings/r2/R2-S13.md); `R2-S02-09`, `R2-S07-02`, and `R2-S11-08` already fixed) plus **`14` companion survivors** in [`desktop_host.md`](audit/findings/r2/desktop_host.md) (`9` open; `B1` and `U9` fixed) and [`interaction-gui_actions.md`](audit/findings/r2/interaction-gui_actions.md) (`5` open). **Wave A progress (2026-11-16):** `S01`–`S09`, `S11`–`S13`, and `desktop_host.md` remediations landed on `main` (`ac7ef26`, `26b5865`, `e6fcb56`, `ed99e44`, `fc035c7`, `8f6ba6b`, `3a8d19a`, `4adeaf7`, `5429527`); `S10`, `interaction-gui_actions.md`, and 2 cross-crate seams remain open and scheduled immediately after `R0.20`.

- [x] Reorganize [`docs/audit/findings/`](audit/findings/) into `r1/` and `r2/` and re-verify all `154` Round 2 items (`138` `R2-*` + `16` companion) against `main` (`1e9dc59`): `5` `FIXED_ON_MAIN`, `18` `PARTIAL_ON_MAIN`, `131` `LIVE_ON_MAIN` (`2026-11-16`).
- [ ] Remediate all `36` surviving `S1` findings (`S02` stats/power, `S03` pharma/TeX/AUC, `S04` reshape/binomial, `S06` unpacked/versioned save guards, `S07` `--gui-only` alias/methods/Cox overrides, `S08` equation box/superimposed connectors, `S10` `ui.invoke_menu` `--gui-only` bypass & dialog patches, `S11` multi-window close/shortcut save/web validation, `S12` `--control-port` `gui_only` & unpacked dir check) with permanent regression tests.
- [ ] Remediate all `66` surviving `S2` findings and the `14` open companion items in [`desktop_host.md`](audit/findings/r2/desktop_host.md) and [`interaction-gui_actions.md`](audit/findings/r2/interaction-gui_actions.md) with regression tests.
- [ ] Remediate all `31` surviving `S3` and `2` surviving `S4` findings (`S01`–`S13`) and record final Round 2 closure in [`docs/audit/round2-post-ebcc622.md`](audit/round2-post-ebcc622.md) and [`docs/STATUS.md`](STATUS.md).

---

### R0.20 Rename LumenCraft to CurveBook
Owner: maintainers · Status: in progress · Priority: next active launch, required for Alpha launch before further unclaimed `R10` GUI implementation · Depends on: `R0.18`, `R7.3`, `R7.4` (done) · Was: rename scheduling request 2026-11-16

Specification: [`plan/curvebook-rename.md`](../plan/curvebook-rename.md), refreshed against committed local `main` `ed99e44` (newer than the read-only GitHub `main` check at `ac7ef26`). The project is pre-alpha with no users: make a full breaking cutover with no old-brand readers, executable/environment aliases, settings migration, or browser-storage migration. Preserve current artwork, scientific behavior, Rust/dependency versions, landed GUI work, existing claims, and audit fixes. Claimed for immediate launch after the pre-`R0.20` baseline commit; code cutover in progress. **Alpha cannot launch until the full product rename is complete:** binaries, native formats, MCP identity/client instructions, launch artefacts, packaging, public repository links, and documentation must consistently use CurveBook. Complete the planned repository identity handoff needed for those public links before launch; external administration remains a separate maintainer action. Known bugs do not waive this identity requirement.

- [ ] Coordinate existing audit/GUI claims, inventory the then-current `main` and tracked paths, record baseline checks, and prepare an isolated rename checkout without disturbing staged/uncommitted work.
- [ ] Rename product casing, Cargo packages/imports/application paths, binaries, environment variables, typed format/batch identities, native extensions (`.curvebook`, `.curvebook.d/`, `.cbtemplate`), settings/recovery namespaces, and desktop/browser/MCP identities as one coherent cutover; preserve forward-version refusal and atomic writes, and explicitly reject old native/batch identities.
- [ ] Update current icons and attribution references, OS/package/install identities (fresh Windows GUIDs), CI/coverage/packaging paths, and repository/schema metadata for `HauserGroup/curvebook`; retain the approved seven-pane and pixel artwork plus historical mole provenance. Incorporate MSIX/Store identity references only if that separate work has landed; require an explicit package maintainer address where needed and do not invent domains or account identities.
- [ ] Regenerate owned outputs, preserve the minimal README and beginner MCP walkthrough under the new commands, reconcile status/parity/audit links, and document the breaking cutover with a narrow residual-name allowlist.
- [ ] Pass renamed workspace CI and relevant policy/generator gates; verify native/unpacked/template/recovery/batch round trips, future-version safety, old-identity rejection, actual stdio/headless/shared-window MCP, landed GUI/browser routing, and applicable local packaging. Record real desktop/browser smoke evidence and target-OS limits; semantic GUI tests alone do not prove human GUI readiness.
- [ ] Deliver a reviewed cutover, rerun MCP acceptance and the beginner setup under the final CurveBook names, and record the Alpha known-bug list and any pending external checks. Complete the separately authorized repository identity handoff needed for Alpha public links; coordinate the local checkout move after active sessions finish. Signing/account changes, publication, deployments, and release/tag pushes remain separate actions; resume remaining unclaimed R10 work with CurveBook identities after the code cutover.

---

### R0.21 Strict Alpha branding and asset-provenance gate
Owner: maintainers · Status: todo · Priority: mandatory Alpha launch gate, after the CurveBook cutover · Depends on: `R0.20` · Was: Alpha branding exclusion requirement 2026-11-16

**Launch requirement:** no reference-suite product/vendor branding or naming in any casing, including compound identifiers, and no references to, copies of, or derivatives of its proprietary assets. This covers source and paths, UI/window text, localization, MCP names/instructions/resources/prompts, documentation/examples, sample names, icons/images/screenshots, metadata, exports, and assembled launch packages. The only compatibility exception is `.pzfx` / `.prism` file handling: filenames/extensions, import/export filters and format documentation, and indispensable on-disk format tokens confined to parser/exporter boundaries. This exception does not permit a product name, vendor name, asset attribution, logo, tutorial, marketing reference, or copied UI wording elsewhere.

Reuse the existing `cargo xtask brands`, `cargo xtask cleanroom`, and `cargo xtask assets` checks and the implemented portions of `R10.14`; close their coverage gaps for this stricter launch requirement. A normal pass with unexamined path exclusions or bypass markers is not proof of Alpha clearance. This gate applies to existing Alpha surfaces now; it does not wait for completion of future R10 widgets or human GUI readiness.

- [ ] Inventory and scan tracked source/text/path identities and every assembled Alpha launch artefact, including generated/embedded content, MCP replies and previews, document/export metadata, package manifests, and documentation; detect case variants and compound names. Confirm that ignored or excluded source cannot leak prohibited branding or asset references into launch outputs.
- [ ] Enforce the narrow `.pzfx` / `.prism` boundary exception with positive and negative regression cases, including filename filters versus titles/help/marketing text and necessary parser tokens versus general identifiers. Do not accept broad directory exclusions or bypass markers as launch waivers; keep verification evidence explicit about the surfaces actually checked.
- [ ] Review original asset provenance and the project's own rendered outputs: no vendor logos, icons, screenshots, sample datasets, copied captions/dialog wording, asset filenames/URLs, or claims of using its assets. Verify `ASSETS.md` / `ATTRIBUTION.md` and original parent-asset chains without deleting contributor or legitimate third-party licence records; resolve prohibited material rather than concealing provenance. Do not inspect proprietary binaries or bundled assets to perform this review.
- [ ] Run and record the strict gate on the final CurveBook launch revision and assembled artefacts, alongside `brands`, `cleanroom`, and `assets`; record automated coverage and the visual/provenance review. Alpha stays blocked until there are no prohibited branding/naming/asset references outside the file-format exception. The known-bug allowance does not waive this gate. Continue checking newly added GUI surfaces under `R10.14` afterward.

---

## R1 Workspace, numerical foundations & NIST StRD oracle

### R1.1 Layered workspace & clean-room quality gates
Owner: maintainers · Status: done · Was: M0

- App-icon refresh (2026-11-16): done — user-approved seven-pane source, self-contained SVGs, PNGs, ICO, and ICNS exports; verified source fidelity, proportions, alpha, platform sizes, asset attribution, and packaging shell lint. Former artwork and contributor credits retained for provenance.
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
Owner: open · Status: done · Was: spec-vs-code review 2026-11-15 ([`statistics-implementation.md` §3.3](statistics-implementation.md#33-linear-mixed-effects-models-reml--nested-designs) described it; never built) · Est. note: 24–40 h

`fit_random_intercept_reml` (`crates/stats/src/anova.rs`) fits only a random intercept, which implies compound symmetry across repeated timepoints. Repeated-measures data with missing cells therefore cannot relax the equal-correlation assumption.

- [x] Added `ResidualCovariance::{CompoundSymmetry, Unstructured}` (`CompoundSymmetry` default) to `FactorialAnovaSpec` (`CURRENT_SCHEMA_VERSION = 2`), persisted with `migrate_v1_to_v2` in [`crates/format/src/archive.rs`](../crates/format/src/archive.rs) (`schema_migration_v0_to_v1_and_future_version_rejection`, `analysis::tests::residual_covariance_defaults_to_compound_symmetry_and_deserializes`), and implemented log-Cholesky REML (`fit_unstructured_rm_reml`, Pinheiro & Bates 1996; Lindstrom & Bates 1988) with documented convergence and `UnstructuredFallbackReason::{SingleSubject, SmallSampleRankDeficient, SingularCovarianceEstimate, IterationLimitReached}` fallbacks (`anova.rs`; also fixed `effective_factorial_mult_spec` unmatched Dunnett `control_column` error).
- [x] Extended Satterthwaite (1946) / Fai & Cornelius (1996) and Kenward & Roger (1997) fixed-effects inference over the unstructured REML `vech(Sigma)` Fisher information with primary-literature doc comments (`D1`) and `CovarianceMatrix` result tabs reporting the estimated covariance & correlation matrices (`UnstructuredRemlResult`).
- [x] Verified `10` `nlme::gls` + KR97 + Hotelling `T^2` and edge-case tests in [`crates/stats/tests/r34_unstructured_reml_oracles.rs`](../crates/stats/tests/r34_unstructured_reml_oracles.rs): `r34_oracle_1_balanced_rm_matches_nlme_gls_and_exact_hotelling_t2_with_scale_invariance` through `r34_oracle_5_more_timepoints_than_complete_subjects_converges_cleanly`, `r34_edge_single_subject_engages_spd_isotropic_fallback`, `r34_edge_all_cells_missing_for_timepoint_compacts_cleanly_and_matches_dropped_table`, `r34_edge_small_cluster_count_below_timepoints_triggers_rank_deficient_fallback`, `r34_edge_singular_covariance_estimate_triggers_spd_shrinkage_fallback`, and `r34_factorial_dunnett_unmatched_control_column_returns_typed_error`.
- [x] Exposed `residual_covariance` in the RM ANOVA Analyze/`Cmd+T` dialog state & `stats.run` schema (`r34_unstructured_covariance_dialog_wiring_and_reml_tabs`), added `3` 16-locale keys (`16 × 312 = 4,992`, `0` missing via `xtask i18n-report`), and regenerated `docs/commands.md` + `docs/schemas/{archive-manifest,document}.schema.json` (`xtask {commands,schemas,ui-commands} --check` passing).

### R3.5 Statistical method-fidelity fixes from the `D1` citation pass
Owner: open · Status: done · Was: `D1` citation pass 2026-11-15 (code review of `crates/stats`) · Est. note: 30–50 h

While citing sources, reviewers compared each routine with the published method. The defects below were traced in code but are **not yet reproduced by tests**. Each fix starts with a failing test and a `D1` oracle with documented tolerances. This item may land as one PR per checkbox.

- [x] **REML *df* (`compute_reml_wald_terms`, `anova.rs`):** implemented Satterthwaite (1946) / Fai & Cornelius (1996) eigenmode `ν_Sat` and Kenward & Roger (1997) `Φ_A = Φ + 2Λ` + scaled `F* = λF` / `ν_KR` over the joint variance-component REML Fisher information `I_E` (`eval_reml_wald_terms_from_mats`, `fit_random_intercept_reml`, `fit_multi_vc_reml`, verified in `r35_anova_reml_oracles::r35_1a_reml_random_intercept_satterthwaite_and_kenward_roger_oracle`, `d1_edge_matrix::{edge_02a_missing_replicate_rm_reml_satterthwaite_kr_vs_nlme, edge_02b_grouped_rm_missing_cell_fallback_and_sparse_cluster_reml}`).
- [x] **Nested ANOVA post-hoc (`run_nested_analysis`):** switched nested group-mean variance to `σ²_u ∑_j(n_{gj}²)/N_g² + σ²_e/N_g` across all multiplicity modes (`r35_anova_reml_oracles::r35_2_nested_posthoc_uses_hierarchical_group_mean_variance_oracle`).
- [x] **Repeated-measures ANOVA:** computed Mauchly/GG/HF sphericity via orthonormal Kronecker Helmert contrasts across Row, Column, and Interaction factors with applied GG/HF *F* *p*-values (`fit_rm_two_way_anova_with_sphericity`, `rm-sphericity` tab emitting `G-G P` and `H-F P`; `FactorialAnovaSpec` still defaults to `RmSphericityCorrection::None` until a spec/UI selector is added), fixed split-plot pooled within-group covariance on `df_e = N - g` (Huynh & Feldt 1976), and replaced the 1-random-intercept fallback with 4-stratum `(S, S×row, S×col, residual)` AI-REML in `run_rm_two_way_reml_fallback` (`r35_anova_reml_oracles::{r35_1b_rm_two_way_reml_fallback_4_stratum_and_boundary_oracle, r35_3a_two_way_rm_all_factors_sphericity_applied_and_scale_invariance, r35_3b_split_plot_sphericity_pooled_within_group_cov_n_minus_g_oracle}`).
- [x] **ANOVA effect sizes:** guarded 6-arg `compute_anova_effect_sizes` on multi-way designs (`eta_squared_ci = None`, `partial_eta_squared_ci = None`), added explicit-`total_n` `compute_anova_effect_sizes_for_n` keeping Steiger (2004) `partial_eta_squared_ci`/`cohens_f_ci` while returning `eta_squared_ci = None` on multi-way, fixed `cohens_f = +inf` at `partial_eta2 == 1.0`, and aligned Kruskal–Wallis `ε² = H / (N - 1)` (`r35_anova_reml_oracles::r35_4_anova_effect_sizes_multiway_cohens_f_inf_and_kw_eps2`).
- [x] **Survival hazard ratios (`survival.rs`):** implemented genuine Mantel–Haenszel rate ratio + Greenland–Robins (1985) variance on `mantel_haenszel_hr`/`_ci` alongside explicit `peto_one_step_hr`/`_ci`, fixed O/E-ratio `SE = √(1/E_A + 1/E_B)` returning `(NaN, None)` without Peto fallback when `E_A`, `E_B`, or `O_B` is `0`, fixed `weighted_log_rank_stats` at `n_total == 1`, and wired `z_crit` in `brookmeyer_crowley_median_ci` (`r35_survival_twogroup_oracles::{r35_surv_oe_hr_genuine_mh_and_peto_oracles, r35_surv_brookmeyer_crowley_confidence_and_s_zero_oracles}`, `survival::tests::weighted_log_rank_exact_hypergeometric_covariance`).
- [x] **Two-group tests (`two_group.rs`):** switched Welch effect-size noncentral-*t* CIs to `n_a + n_b - 2`, eliminated Pratt signed-rank tied-zero variance double-counting, verified Monahan (1984) large-sample Hodges–Lehmann + Moses CI for `m·n > 40,000`, made pairing-correlation *p* two-sided with exact `r = ±1` bounds, replaced placeholder `Levene p = 1` with `levene_median: Option<LeveneTestResult>` (`None`) + exact two-sample *F*-test on summary inputs, and aligned sign-test / Mann–Whitney one-tailed directions (`r35_survival_twogroup_oracles::{r35_tg_welch_noncentral_t_ci_pooled_df_oracle, r35_tg_pratt_wilcoxon_oracle_no_zero_double_counting, r35_tg_hodges_lehmann_large_n_oracle, r35_tg_pairing_correlation_two_sided_and_boundaries, r35_tg_summary_t_tests_no_placeholder_levene_and_exact_f, r35_tg_one_tailed_conventions_consistent}`).
- [x] **Power & multiplicity:** implemented exact unconditional bivariate-normal *r* distribution quadrature (`hotelling_correlation_power`, `hyp2f1_half_half`) for `CorrelationPowerMethod::ExactTTransform`, added `CorrelationPowerMethod::ConditionalSlopeT` aligned with `SimpleLinearRegression` (`df = n - 2`, `λ = n·f²`), computed genuine unadjusted two-sided pairwise `raw_p` across Tukey–Kramer, Dunnett, Games–Howell, and Dunnett T3 while returning `InvalidParameter` on unmatched Dunnett `control_column` in `compare_dunn_ranks` and `compare_groups_with_procedure`, separated `MultiplicityProcedure::LinearTrend` so `DunnRankSum`/`LinearTrend` error on parametric pairwise paths and fall back to Bonferroni in `adjust_p_values`/`compare_dunn_ranks`/`multiple_row_t_tests`, and fixed `linear_trend_test` `MSE == 0` (`F = +∞, p = 0` when `ss_trend > 0`; `F = 0, p = 1` when `ss_trend == 0`) (`power_oracle::oracle_correlation`, `r35_power_multiplicity_oracles::{multiplicity_raw_p_is_genuine_unadjusted_pairwise_p_for_tukey_dunnett_games_howell_and_t3, linear_trend_test_zero_mse_and_published_maxwell_delaney_oracle, dunn_rank_sum_and_linear_trend_never_emit_silent_unadjusted_output, dunnett_and_dunn_control_column_lookup_errors_on_missing_and_respects_sparse_indices, multiplicity_unbalanced_dunnett_raw_p_and_multiple_row_bonferroni_adjustment, correlation_exact_hotelling_r_oracles_two_sided_and_one_sided, correlation_and_simple_linear_regression_n_convention_alignment}`).
- [x] **Minor:** fixed Bartlett zero-variance handling (`(+inf, 0.0)` when `pooled_var > 0`), Kruskal–Wallis all-tied `tie_correction = 0.0`, aliased factorial `df = 0.0`, and removed dead nested-ANOVA `n0` / unused *t* *p*-value computations (`r35_anova_reml_oracles::r35_8_minor_bartlett_kw_all_tied_and_aliased_factorial_df0`).

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
Owner: open · Status: done · Was: audit `R1-S03-06` (S2); [`curve-fitting-implementation.md`](curve-fitting-implementation.md) described it, never built · Est. note: 8–14 h

A `FitMethod::RobustCauchy` fit now suppresses least-squares inference instead of emitting Cauchy-reweighted SEs, CIs, bands, AICc/BIC, or *F*-test *p*-values.

- [x] Added typed `InferenceAvailability::{Available, NotApplicableInRobustMode}`, `ModelComparisonOutcome::NotApplicableInRobustMode`, and `ModelComparisonResult` (`crates/fit/src/{solver.rs,lib.rs}`), returning `NotApplicableInRobustMode` instead of parameter SEs/CIs, covariance/correlation matrices, confidence/prediction bands, `Sy.x`, AICc/BIC, lack-of-fit, studentized residuals, interpolated unknown CIs, and `compare_models` / `compare_models_checked` while preserving best-fit values, residuals, `RSDR`, and interpolated point predictions (`r45_robust_suppression::{r45_robust_cauchy_typed_suppression_r1_s03_06_and_preserved_quantities, r45_robust_cauchy_shared_independent_and_plot_function_only_suppress_inference}`).
- [x] Rendered `"Not applicable"` / `NotApplicableInRobustMode` on result sheets (`crates/fit/src/result.rs`), suppressed robust bands in `crates/chart/src/xy.rs` (`resolve_fitted_overlay`), updated `methods_report.rs`, MCP (`doc_inspect`/`sheet://`), CLI (`info`), and `compare_models` when either model is robust (`r45_robust_suppression::{r45_compare_models_suppresses_whenever_either_model_uses_robust_cauchy, r45_result_sheet_rendering_robust_vs_rout_all_tabs}`, `r45_robust_e2e::{r45_versioned_serde_roundtrip_and_legacy_compatibility, r45_engine_session_exports_and_methods_report_for_robust_and_rout, r45_chart_xy_suppresses_robust_bands_even_on_tampered_overlay_and_keeps_rout_bands}`, `r45_robust_mcp::r45_mcp_robust_fit_renders_not_applicable_status_and_rsdr`, `r45_robust_cli::r45_cli_info_and_convert_render_robust_not_applicable_status`).
- [x] Preserved full least-squares inference for ROUT Stage 2 (`FitMethod::RoutOutlierElimination`) with regression/audit tests covering both paths (`r45_robust_suppression::r45_rout_stage2_preserves_full_least_squares_inference`, `audit_r1_s4a_fit::r1_s03_06_robust_cauchy_suppresses_parametric_inference_and_comparisons`).

### R4.6 Fitting method-fidelity fixes from the `D1` citation pass
Owner: open · Status: done · Was: `D1` citation pass 2026-11-15 (code review of `crates/fit`) · Est. note: 24–40 h

Traced in code while adding citations; reproduced and resolved by targeted `D1` oracle suites (`r46_linear_oracles`, `r46_library_oracles`, `r46_solver_oracles`, `r46_shapiro_rk45_oracles`, `d1_reference_oracles`, `nist_strd`).

- [x] **Deming regression λ inverted (`linear.rs`):** fixed `fit_deming_core` / `deming_regression` so `DemingOptions::variance_ratio_lambda` is strictly `σx²/σy²` (matching `r46_linear_oracles::{deming_lambda_sigma_x2_over_sigma_y2_reproducer, deming_50_digit_oracles_across_lambda_and_ols_limits, deming_xy_swap_reciprocity_and_edge_cases}` across `λ ∈ {0.25, 0.5, 1.0, 2.0, 4.0}`, OLS limits, and X↔Y swap reciprocity).
- [x] **Poisson fits in model comparison:** added exact `poisson_log_likelihood_from_obs` / `poisson_information_criteria` / `compare_poisson_aicc` (`K = k_free`, `phi = 1`) and guarded `compare_models` to reject OLS `ExtraSumOfSquaresF` for `PoissonDeviance` and mixed Gaussian/Poisson pairs while retaining deviance LRT and `R4.5` `NotApplicableInRobustMode` (`r46_solver_oracles::r46_2_poisson_aic_aicc_bic_and_model_comparison_reproducer_and_oracle`).
- [x] **Replicates (lack-of-fit) test:** unified WLS, pre-summarized within+between pure error/residual SS, single-row `ObservedSd` `None` handling, and Poisson deviance across both `solver.rs` (`replicates_lack_of_fit_obs`, `r46_solver_oracles::r46_3_replicates_lack_of_fit_weighted_and_presummarized_oracles`) and `diagnostics.rs` (`replicates_lack_of_fit` / `group_pure_error_ss`, `r46_shapiro_rk45_oracles::r46_3_replicates_lack_of_fit_presummarized_and_multidataset_oracles`, `diagnostics::tests::replicates_lack_of_fit_guards_non_finite_and_extreme_n`).
- [x] **ROUT:** implemented Motulsky & Brown (2006) top-30% Benjamini–Hochberg scoping across both `solver.rs` (`rout_detect_outliers_mb2006`, `start = floor(0.70 * N)` leaving bottom ~70% untested at `q = 1.0`, `r46_solver_oracles::r46_4_rout_motulsky_brown_2006_top_30_percent_bh_reproducer_and_oracle`) and `diagnostics.rs` (`rout_detect_outliers`, `r46_shapiro_rk45_oracles::r46_4_rout_detect_outliers_top_30_percent_motulsky_brown_2006`).
- [x] **Shapiro–Wilk (residual normality) for n = 4–5:** replaced the `n = 4..5` normalized Blom scores and unreachable `if n > 5` branch with Royston's (`1992`/`1995`, AS R94) polynomial-corrected `a_n` (`royston_sw_weights`) and `p`-value across `crates/fit/src/diagnostics.rs` (`shapiro_wilk_test`, `r46_shapiro_rk45_oracles::r46_5_shapiro_wilk_n3_n4_n5_royston_as_r94_reproducer_and_oracles`) and `crates/stats/src/column.rs` (`shapiro_wilk_p_value`, `d1_reference_oracles::oracle_normality_shapiro_wilk_lognormality_and_outliers`).
- [x] **Inverse interpolation (`interpolate_x_from_y`, linear X-from-Y):** landed exact `linear.rs` `InverseIntervalSet` / `InverseCalibrationResult` / `solve_inverse_quadratic` (`r46_linear_oracles::{inverse_x_from_y_calibration_vs_mean_response_d1_oracles, inverse_x_from_y_fieller_regimes_g_eq_1_and_gt_1_and_edges}`, `linear::tests::solve_inverse_quadratic_exact_g_eq_1_and_extreme_regimes`), added nonlinear `InterpolatedUnknown` `pred_lower..pred_upper` calibration interval inversion alongside mean-response CI in `crates/fit/src/solver.rs`, labeled `"Mean-Response CI"` vs `"Calibration / Prediction Interval"` across linear & nonlinear interpolated result tabs in `crates/fit/src/result.rs`, and documented fixed `DemingOptions::variance_ratio_lambda = 1.0` (`r46_solver_oracles::r46_6_inverse_interpolation_calibration_vs_mean_response_and_deming_note`).
- [x] **Equation library:** fixed `beta_growth_then_decay` (`Ym` hold / `te <= tm` guard), `biphasic_dose_response` sign/guess, `centered_poly_*` `default_parameter_rules` (`XCenter = mean(X)`), `absolute_ic50_4pl` midpoint, `power_law` rename + alias, `exponential_plateau` vs `one_phase_association`, and NIST `Nelson` `log(y)`, AND wired `ParametricModel::default_parameter_rules` into `ParameterLayout::build` in `crates/fit/src/solver.rs` when no explicit caller rule exists (`r46_library_oracles::{r46_1_beta_growth_holds_ym_after_te_and_rejects_te_le_tm, r46_2_biphasic_dose_response_sign_convention_and_direction_guess, r46_3_centered_poly_free_xcenter_is_rank_deficient_and_default_rules_fix_sample_mean, r46_4_absolute_ic50_sebaugh_2011_controls_midpoint, r46_5_power_law_rename_and_legacy_alias, r46_6_exponential_plateau_vs_one_phase_association_resolution, r46_7_nist_nelson_fits_log_y_oracle}`, `r46_solver_oracles::r46_7_centered_poly_default_parameter_rules_wiring_oracle`, `nist_strd`).
- [x] **Minor:** fixed RK45 rejected-step shrink exponent from `−1/4` to `−1/5` (Hairer–Nørsett–Wanner §II.4) in `crates/fit/src/dsl.rs` (`r46_shapiro_rk45_oracles::r46_8_rk45_rejected_step_shrink_exponent_and_ode_accuracy_oracles`, `dsl::tests::rk45_tiny_and_non_finite_spans_do_not_panic_or_hang`), implemented Moré (1978) / MINPACK-1 `lmpar` exact `parc = fp / (delta * ||s||^2)` + active-face step in `crates/fit/src/solver.rs` (`r46_solver_oracles::r46_8_more_newton_trust_region_lm_convergence_oracles`), and removed dead code `ratio` / `denom` (`r46_library_oracles::r46_8_logistic_growth_and_operational_model_oracles`).

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
Owner: open · Status: done · Priority: after `R0.18`, before new `R10` GUI implementation · Depends on: `R0.18`, `R7.1`, `R8.3`, `R8.6` · Was: MCP capability review 2026-11-15 · Est. note: 16–28 h

**Completion criterion:** All currently implemented engine workflows must be usable through the actual MCP transport, with verified results, persistence, exports, and structured failures. This is the functional basis for **MCP Alpha**; known bugs may remain and must be documented. Full audit closure and human GUI readiness are separate milestones. An in-process semantic-harness pass alone does not establish MCP readiness or human GUI usability. Use the existing engine/session and desktop presenter; completion must not depend on new human-operable GUI widgets.

- [x] Verified `WF-01`–`WF-50` end to end through actual spawned `lumencraft-cli mcp --gui-only` stdio JSON-RPC subprocesses plus headless `command_run` / `command_batch` save/reopen/SVG/PDF/CSV workflows (`r73_mcp_gui_only_wf_01_15` `3`, `r73_mcp_gui_only_wf_16_28` `3`, `r73_mcp_gui_only_wf_29_42` `3`, `r73_mcp_gui_only_wf_43_50` `2`, `r73_mcp_surface_and_headless::r73_stdio_headless_workflows_command_run_batch_save_reopen_export`).
- [x] Verified complete MCP surface end to end over stdio JSON-RPC and unsandboxed `127.0.0.1` loopback (`initialize`, `tools/list`, progressive `list_toolsets`/`enable_toolset` + notifications, `sheet://`/`result://` resources & prompts, `WorkdirGuard` path containment & structured `isError` failures, strict `--gui-only` rejection of `doc_inspect`/L5/`ui.screenshot` disk writes while preserving read-only control inspection: `lumencraft-cli` **`69 passed`**, `lumencraft-mcp` **`60 passed`**, unsandboxed `25/25` across `gui_only_surface`/`mcp_e2e`/`mcp_gui_only_parity`/`path_policy_and_mcp_workdir`/`r73_mcp_surface_and_headless`).
- [x] Added opt-in `--window` flag so `lumencraft --control <port> --window` (or `LUMENCRAFT_CONTROL_PORT`) attaches to a visible `eframe`/`egui` desktop window sharing the same live `UiApp`/engine session (`apps/lumencraft/src/{live_control.rs,gui_window.rs,desktop_host.rs,main.rs}`, [`docs/mcp.md`](mcp.md)) while preserving headless default when `--window` is omitted and enforcing loopback-only `validate_loopback_addr` + strict `--gui-only`, verified unsandboxed over `127.0.0.1` TCP (`lumencraft` **`51 passed`** incl. `r73_shared_window` 8/8, `control_loopback` 8/8, `tests::live_window_control_server_roundtrip_and_token_cleanup` 1/1).

**Future-feature follow-up:** Extend MCP acceptance coverage when new engine features land, including `WF-50` after [`R10.11`](#r1011-compact-letter-display-cld). These later additions do not block the existing-capability MCP gate or create a dependency from `R7.3` back to `R10`. The CurveBook rename follows the completed MCP/README baseline as [`R0.20`](#r020-rename-lumencraft-to-curvebook), before further unclaimed R10 work.

---

### R7.4 Accessible MCP guide and minimal README
Owner: open · Status: done · Priority: immediately after verified `R7.3`, before the GUI handoff · Depends on: `R7.3` · Was: MCP onboarding and README simplification request 2026-11-15

Provide a short, welcoming entry point for a researcher or first-time MCP user, written after the `R7.3` MCP implementation and transport acceptance checks passed.

- [x] Reduced [`README.md`](../README.md) (`124` lines) to a presentable functional minimum: concise introduction, restrained seven-pane icon (`assets/app-icon/lumencraft-win95-2508.png`, `width="160"`), honest pre-alpha GUI status (`0/50` human-operable workflows until `R10`), Rust `1.99.1+` (`rust-toolchain.toml`) prerequisites and working `lumencraft-cli` quick start, beginner MCP guide, compact documentation/support links, and MIT/Apache-2.0 + zero-telemetry notices (`cargo fmt --all -- --check`, `cargo xtask {assets,brands,secrets,telemetry,cleanroom,screenshots --check,layers}`: `26` attributed assets, `0/598` brands/secrets, `0` telemetry violations).
- [x] Added a beginner stdio MCP guide directly in [`README.md`](../README.md): one-sentence plain-language explanation, concise glossary (`stdio`, headless mode, `--workdir`), `cargo build --release -p lumencraft-cli` & `cargo run -q -p lumencraft-cli -- mcp` commands, copy-paste default headless config with absolute-path & `--workdir` guidance, `initialize`/`notifications/initialized`/`ping`/`tools/list` connection check, reproducible Welch's *t*-test `command_batch` example (`t = -8.520563361656317`, `PValue = 0.0036599555836826485`, `CohensD = -6.957010852370434`, `HedgesG = -5.550891548444351`), and common startup/path troubleshooting.
- [x] Linked [`docs/mcp.md`](mcp.md) (`139` lines) for protocol framing, opt-in shared live window (`lumencraft --control <port> --window` + `--control-port`), `--gui-only` policy, advanced configuration, and honest limitations, reconciled with final `R7.3` + `R10.10`/`R10.11` behavior (`cargo xtask {commands --check,schemas --check,ui-commands --check}` pass).
- [x] Verified live against `main` (`cargo build --release -p lumencraft-cli --offline`, `--help`, `commands --group analysis` `12` commands, stdio `initialize`/`ping`/`tools/list` with `protocolVersion: "2024-11-05"` and `10` core tools, Welch's *t*-test `command_batch` matching documented scalars, `cargo run -q --offline -p lumencraft-cli -- mcp --workdir` rejecting `../escape.lumencraft` with `path_escape`, and `55`/`55` relative file links & anchors resolving across `README.md` and `docs/mcp.md`).

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
Owner: maintainers · Status: done · Was: G6 (former desktop visual gate + **Core**), M5 · Est. note: 0 h remaining

- [x] Implement `DesktopHostRuntime` (`apps/lumencraft/src/desktop_host.rs`) with `--in-window-menus`, `--new-instance` isolated crash-recovery journals, Retina DPI transitions, IME composition, trackpad pinch/inertial pan, and file-drop integration tests (`desktop_host_loop`; Wave 13).
- [x] Link `eframe 0.37` / `egui 0.37` (`default-features = false`, `wgpu`, `glow`, Linux `wayland` + `x11`) and `rfd 0.17` in `Cargo.toml` / `Cargo.lock` (`417` locked packages) and implement `apps/lumencraft/src/gui_window.rs` (`LumenCraftNativeApp`, `build_egui_font_definitions`, `run_native_desktop`, top `egui::MenuBar` + native `rfd::FileDialog`, Retina `pixels_per_point` resize, pointer/click-count/wheel/pinch/IME/clipboard/file-drop routing, `TextureOptions::NEAREST` RGBA viewport upload, coasting `AnimationTick`, and `egui::Modal` unsaved-close prompt while preserving `--headless-once`, `--event-script`, and `--control`; `lumencraft` `23 passed`, former visual gate / agent-harness foundation).
- **Scope note (2026-11-15 audit):** below the native menu bar the window is a single textured quad of the CPU RGBA buffer. Engine menu items are dispatched with `Value::Null` params (`desktop_host.rs`), so commands with required fields (e.g. `table.excludeCells`, `Analyze > Statistical Test…`) fail with `BadInput`; `--control` runs headless without opening a window. Interactive widgets and parameter routing are tracked in [`R10.1`](#r101-interactive-chrome-architecture-decision--spike), [`R10.3`](#r103-interactive-modal-dialog--wizard-framework-b1), and [`R10.6`](#r106-menu-toolbar--context-menu-command-routing-b4-desktop-b5).

### R8.4 Live `wasm32-unknown-unknown` Trunk WebGPU/WebGL2 browser hosting
Owner: maintainers · Status: done · Was: G6 (former WASM visual gate), M5 · Est. note: 0 h remaining

- [x] Implement `apps/lumencraft-web/src/host_adapters.rs` (`WebStorageManager` 5 MiB `localStorage` + forward-version guard, `WebLockCoordinator` multi-tab read-only contention fallback, `FileSystemAccessBridge` incremental `.lumencraft.d/` sync, `TouchGestureRecognizer`, and `WebBridgeSession` protocol; `lumencraft-web` `13 passed`) plus strict no-egress `index.html` and CSP/COOP/COEP headers (`packaging/web/{_headers,.htaccess}`) (Wave 13).
- [x] Connect `apps/lumencraft-web/src/main.rs` (`connect_wasm_bridge` via `wasm-bindgen 0.2.130` / `web-sys` / `js-sys` under `#![forbid(unsafe_code)]`) and implement the zero-dependency WebGL2 textured-quad RGBA presenter (with lazy 2D fallback) + DOM mouse/wheel/keyboard/IME/paste listeners and `data-wasm-opt-params` in `apps/lumencraft-web/index.html`, verified via `cargo xtask wasm`, `cargo xtask telemetry` (`0 violations`), and `./packaging/web/package.sh` (`Trunk 0.22.1` `--cargo-profile web-release` -> `lumencraft-web-0.1.0.zip`, `4,162,394` B, former visual gate).
- **Scope note (2026-11-15 audit):** `lumencraft-web` does not link `egui`/`eframe`; it blits the RGBA buffer to a WebGL2 `<canvas>` and has no menu bar or toolbar in HTML or WASM. Web chrome parity is tracked in [`R10.7`](#r107-web-menu-bar-toolbar--chrome-parity-b4-web).

### R8.5 Cross-OS hardware, CJK/RTL desktop QA & installer/signing hardening
Owner: open · Status: in progress · Was: G13, M7 (**1.0**)

Windows MSIX and Microsoft Store distribution are planned separately in [`R8.9`](#r89-windows-msix-packaging--microsoft-store-distribution).

- [x] Implement macOS `Info.plist` `plutil -lint` validation and ad-hoc `codesign --force --deep --sign -` + `--verify --strict --deep` in `xtask/src/bundle.rs` (Wave 7), live macOS `NSPasteboard` `%PDF-1.7` roundtrip (`macos_clipboard_live`), `cjk_rtl_hidpi_desktop_qa` (`ja`/`zh-CN`/`ko`/`ar`/`he` across `1.0x`/`1.5x`/`2.0x`/`3.0x`, `4 passed`), `cargo about generate` `THIRD-PARTY-LICENSES.html` (`273K`) + `cargo deny check licenses`, and local universal macOS (`aarch64` + `x86_64`) `lumencraft-0.1.0-macos-universal.dmg` (`36,340,795` B) + `lumencraft-cli-0.1.0-macos-universal.zip` (`13,504,838` B) packaging (`package.sh --arch universal`), `verify.sh --arch universal` mount/signature validation, and `check-app-specific.sh`.
- [ ] Complete Apple Developer ID signing + notarization, Windows VM, Linux X11/Wayland VM (AppImage/deb/rpm/Flatpak), FreeBSD runner, and live browser QA runs.
- [ ] Add keyless GitHub artifact attestations (Sigstore build provenance via `actions/attest-build-provenance`) for every file the `publish` job in [`.github/workflows/release.yml`](../.github/workflows/release.yml) uploads, including `SHA256SUMS.txt`, so Linux (`.AppImage`/`.deb`/`.rpm`/`.tar.gz`), Windows, macOS, and web downloads can be checked with `gh attestation verify <file> --repo <owner>/<repo>`. This needs `id-token: write` + `attestations: write` on the `publish` job only, so widen `cargo xtask telemetry` check 4 (`validate_permission_entry` in `xtask/src/telemetry.rs`, which today allows only `contents: write` there) for exactly those two scopes in that job, with fixture tests that still reject them anywhere else. Document user-side verification in [`docs/releasing.md`](releasing.md). The Sigstore calls happen in CI only; shipped binaries gain no network access.

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

### R8.9 Windows MSIX packaging & Microsoft Store distribution
Owner: maintainers · Status: todo · Was: Windows distribution planning (2026-11-16)

Scope: add MSIX alongside the existing MSI and portable ZIP. No local Windows machine is available: builds, self-signing, and automated Windows checks must run on GitHub-hosted Windows runners. Microsoft Store publishing must not require Azure Trusted Signing or a purchased code-signing certificate; Microsoft signs Store packages after certification. Self-signed packages are for explicitly trusted test machines. Packaging preparation can start now; public Store submission depends on Windows QA (`R8.5`), release verification (`R0.14`), and human GUI acceptance (`R10.13`). This is a separate distribution target, not an additional general **1.0** gate.

- [x] Document Store signing versus local self-signing, certificate creation, explicit test-machine trust, signature verification, installation, and trust removal in [`windows-signing.md`](windows-signing.md); linked from [`releasing.md`](releasing.md). Documentation reviewed against Microsoft guidance on 2026-11-16; Windows command execution remains unverified.
- [ ] Add Rust `xtask` MSIX packaging using Windows SDK tools, with bounded subprocesses and atomic output publication through `lumencraft-platform`; stage desktop and CLI binaries, existing attributed icons, and licence notices without bundled fonts. Keep MSI/portable outputs available; add architecture-specific MSIX outputs and an optional bundle.
- [ ] Define the desktop package manifest with Partner Center identity values, a stable publisher, supported Windows versions, Store-compatible version mapping, required desktop capabilities, document associations at the packaging boundary, and a tested CLI execution alias for MCP clients. Keep development identity separate when needed; do not invent Store identity values.
- [ ] Provide distinct Store-submission and test signing modes: no Azure/CA signing prerequisite for Store packages, and opt-in self-signing for test packages. Add a manual `workflow_dispatch` Windows test job that creates an ephemeral certificate on the runner, explicitly trusts its public certificate only for the disposable MSIX test session, and uploads packages, the public `.cer`, thumbprint, and test reports. Never publish private keys/PFX files; decide separately how a stable test key is retained if needed across runs.
- [ ] Verify MSIX on GitHub-hosted Windows runners: manifest validation, signature and publisher matching, installation/launch, CLI/MCP invocation, document open/save/export, upgrade, uninstall, and coexistence or migration with MSI installs. Record results for supported architectures and check that application data stays outside the read-only package directory. Run Windows App Certification Kit where supported; record runner/session limitations and arrange remote interactive Windows QA for visual/hardware checks that CI cannot establish.
- [ ] Add MSIX build/validation artefacts and checksums to release CI, plus browser-based instructions to dispatch the Windows job and download its artefacts without a local Windows PC. Keep test signing separate from public release publication, credentials out of logs, and application behavior free of egress. Updates are delivered by the Store, with no embedded updater or telemetry SDK.
- [ ] Complete Partner Center registration, reserve the product name, obtain the package identity, prepare listing/screenshots/privacy disclosures, submit for certification, and record the published Store URL and a verified Store update. Account setup and publication require maintainer action.

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

**Sequencing:** Complete [`R0.20`](#r020-rename-lumencraft-to-curvebook), the CurveBook cutover, after the completed `R0.18` toolchain, [`R7.3`](#r73-mcp-workflow-parity-proof--shared-live-window) MCP, and [`R7.4`](#r74-accessible-mcp-guide-and-minimal-readme) README baseline, before starting further unclaimed human-operable GUI implementation. Keep already-landed R10 work and existing in-progress claims intact; coordinate their completion or rebase before the cutover. Independent audit/scientific fixes continue, and future engine features receive MCP coverage when they land.

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
Owner: maintainers · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 16–24 h

- [x] **Decision resolved (2026-11-15):** implement the single retained layout & widget tree in-house in `lumencraft-ui-egui`, painted by `compositor.rs` with shaped text from `lumencraft-text` on desktop and headless runs; `egui` widgets are not used for chrome. Record the decision and the `UiApp` ↔ layout-tree boundary (including a text-measurement interface the web host implements) in [`architecture.md`](architecture.md) as part of the spike below.
- [x] **Decision resolved (2026-11-15):** fonts under the zero-bundled-fonts rule — desktop discovers system fonts (`LUMENCRAFT_FONTS_DIR` override); the web host measures and draws text with the browser's canvas text API and the browser's fonts; desktop/headless determinism is measured against a pinned, untracked test font set (`cargo xtask web-fonts`).
- [ ] Spike `AnalyzeSheet` + `EditAnalysisParameters` (one-way ANOVA) end to end on desktop and web; report web bundle size, `1M`-cell grid frame time, and `cargo xtask wasm`, `telemetry`, `licenses`, and `layers` results. *(Phase 2 core landed in `4e6b8b2`: desktop/headless `LayoutTree` ANOVA spike + `1M`-cell frame budget verified via `r10_core_and_spike::{r10_1_spike_analyze_and_edit_params_one_way_anova_human_path, r10_22_one_million_cell_virtualized_grid_frame_budget}` and `cargo xtask r10-gate`, alongside web `WebTextBridge`/`WebChromeBridge` `lumencraft-web` **`26 passed`**, `xtask wasm`, `xtask telemetry` `0`, `xtask layers` `22` pkgs; unifying `lumencraft-web` onto shared `LayoutTree` awaits Wave 5.)*
- [ ] Select the human-path test driver for `R10.2` and confirm any new dependency's licence with `cargo xtask licenses`. *(In-house `HumanPathDriver` added in `crates/ui-egui/src/human_driver.rs` with zero new external dependencies; `cargo xtask licenses` passes; full 50-workflow driver rollout remains open for Wave 5/6.)*

### R10.2 Human-path acceptance harness & phantom-widget gate
Owner: open · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 30–48 h · Depends on: `R10.1`

- [ ] Add a human-path harness that drives the real frame only through raw pointer/wheel/key/IME events and the layout-tree accessibility view, with semantic widget-ID shortcuts disabled; fail on any use of `ui.invoke_menu` params, `ui.answer_dialog` field patches, or `try_activate_semantic_widget`. *(Phase 2 `HumanPathDriver`, `HumanPathOnly` bypass guard, `verify_pixel_to_hit_box_parity`, and `verify_click_action_parity` landed in `4e6b8b2`, passing `r10_core_and_spike::r10_2_human_path_only_blocks_synthetic_bypasses`; Wave 5/6 50-workflow expansion remains open.)*
- [ ] **Pixel-to-hit-box parity gate:** every control in the accessibility tree is visibly rendered within its parent's bounds, its text is neither clipped nor overlapping, and a raw `(x, y)` click at its centre has the same effect as the semantic automation action. *(Verified for Phase 2 core views via `r10_core_and_spike::r10_2_and_r10_16_parity_across_views_plus_textbox_ime_clipboard_dropdown_and_scroll`; Wave 5 dialogs/inspectors stay open.)*
- [ ] Generate `ui.inspect` from the shared layout tree and route `gui_actions.rs` (`dispatch_gui_action`) through the same input path as `interaction.rs`, so agent and human handling (one keymap, e.g. `Cmd+T` on a result sheet) cannot diverge. *(Phase 2 `inspect.rs` projection from `LayoutTree` & unified `interaction.rs`/`gui_actions.rs` routing landed in `4e6b8b2`; full `dispatch_gui_action` surface parity across remaining Wave 5 modals/sheets continues in Wave 5.)*
- [ ] Wire the harness and gate into `cargo xtask ci` and label the existing `gui_wf_*` shards as agent-harness tests in [`gui-workflows.md`](gui-workflows.md). *(`cargo xtask r10-gate` wired into `cmd_ci` in `xtask/src/main.rs` [`4e6b8b2`] and §6.1/§6.2 updated in [`gui-workflows.md`](gui-workflows.md); Wave 5/6 port of `WF-01`–`WF-50` remains open.)*

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
Owner: open · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 28–44 h · Depends on: `R10.16`

- [ ] Replace the static 3-box header strip with a contextual toolbar of grouped icon-and-label buttons that adapt to the active sheet kind: File/Project, Sheet management, Undo/Redo, Clipboard, Analysis (Analyze, Parameters, Data Analyzed, Interpret), Graph/Layout (Change Graph Type, Axes, Pairwise Comparisons, Rulers, Alignment), Text styling, and Export/Print; icons are original vector assets with `ASSETS.md` rows.
- [x] Replaced `Value::Null` dispatch in `apps/lumencraft/src/{desktop_host.rs,gui_window.rs}` (`resolve_desktop_command_route`, `DesktopCommandRoute`, `DesktopDialogOpener`, `:variant` submenu/shortcut/wheel routing, extended `rfd::FileDialog` import/export/template handling with canonical `.lctemplate`/`NATIVE_OPEN_FILE_EXTENSIONS` filters and foreign-import `file_path` clearing): verified across `lumencraft` **`69 passed`** (`r10_command_routing::{registry_sweep_routes_every_engine_and_ui_command_without_null_params, parameterized_commands_open_expected_dialogs_and_wizards, submenu_variants_and_qol7_diagnostics_route_and_persist_preferences, keyboard_shortcuts_route_context_and_dialogs_correctly, mouse_wheel_scrolls_data_grid_and_ignores_non_canvas_without_zooming, native_file_dialog_routing_covers_all_import_export_template_commands, native_open_template_filters_and_foreign_import_file_path_clearing}`, `desktop_host_loop` `7`, `audit_r2_desktop_host` `8`).
- [ ] Draw and wire right-click context menus for grid cells, column headers, navigator entries, graph objects, and linked regions; make `Cmd/Ctrl+E` and `Exclude With Reason` use the current selection.
- [ ] Render the `Cmd/Ctrl+K` command palette with typing and keyboard selection.

### R10.7 Web menu bar, toolbar & chrome parity (`B4` web)
Owner: open · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 20–32 h · Depends on: `R10.1`, `R10.6`

- [ ] Present the shared menu bar, toolbar, dialogs, and context menus in `lumencraft-web` from the same layout tree as desktop (binding rule 3). *(Web `WebTextBridge` / `ZLayerBand`, `WebChromeBridge` / `WebChromeLayout`, and `<canvas>` `Z_BAND_ORDER` clip/fillText pipeline landed in `apps/lumencraft-web/{index.html,src/{text_bridge,chrome_bridge,host_adapters}.rs}`; consuming desktop's shared `LayoutTree` awaits `R10.1` Wave 5 integration.)*
- [x] Mapped web keyboard shortcuts with browser-reserved key handling (`WebShortcutRouter`, `WEB_SHORTCUT_MAPPINGS`, zero-`Value::Null` `WebCommandRoute` / `:variant` dropdowns, QOL-7 disabled remediation, `toolbar.edit_params` & `toolbar.axes` routing) and open/save/export via `FileSystemAccessBridge::export_bytes` with Blob `<a download>` + `<input type="file">` fallbacks (`lumencraft-web` **`26 passed`** incl. `web_chrome_parity_menu_toolbar_modal_and_alt_t_edit_params_flow`, `file_system_access_and_download_fallback_export_all_supported_formats`, `toolbar_undo_redo_new_axes_edit_params_and_shortcuts_route_without_null`, `dropdown_menus_omit_event_dispatch_disable_remediation_and_route_palette_and_variants`, `browser_host_adapters::*` `8`).
- [x] Re-verified strict CSP (`connect-src 'self'`), `cargo xtask telemetry` (`0` violations across `24` manifests / `417` locked packages / `208` Rust sources / `51` web/workflow/packaging files / `6` workflows / `2` header configs), `cargo xtask wasm` + `RUSTFLAGS="-D warnings" wasm32 -p lumencraft-web`, `xtask {layers,cleanroom,brands,secrets}` (`0/634`), `index_html_enforces_strict_csp_zero_egress_z_compositing_and_blob_fallback`, and `./packaging/web/package.sh` (`lumencraft-web-0.1.0.zip` `4.0M`, `<= 6,243,591` B).

### R10.8 Multi-tier data grid, navigation & calculated-variable editor (`B5`)
Owner: open · Status: todo · Was: Human-GUI audit 2026-11-15 · Est. note: 50–80 h · Depends on: `R10.16`, `R10.9`

- [ ] Multi-row column headers that reflect table structure: column/group identifier band, editable dataset title row, replicate subcolumn headers (`Y1, Y2, …` or `Mean, SD/SEM, N`), and super-column spans for grouped and nested tables; tabular-numeral cell text.
- [ ] Dedicated resizable, collapsible **Row Titles** column next to row numbers, and column-edge drag resizing.
- [ ] Wheel/trackpad scrolling and scrollbars in both axes (zoom only over graph/layout canvases); arrow, `Tab`, `Enter`, `Shift+Arrow`, and `Shift/Cmd+Arrow` boundary navigation; `Delete`/`Backspace` clearing.
- [ ] Clickable `MultipleVariables` variable-type badges (`Continuous`, `Categorical`, `Label`, `Calculated`) as dropdowns with lock/unlock, plus the `Variables…` level-recoding editor.
- [ ] Right-side **Calculated Variable** editor panel on the existing `FormulaBarState`: variable name, live formula input, syntax/circularity validation message, clickable column-variable list, searchable function reference with parameter hints, Apply/Cancel, and a formula-definition view toggle.

### R10.9 Proportional typography, Unicode coverage & glyph gate
Owner: open · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 16–30 h · Depends on: `R10.1`

- L2/L6 text shaping & glyph-atlas foundation (2026-11-16): `crates/text/src/shaping.rs` adds `FontFamilyKind`, `UiTextStyle`, `MeasuredTextMetrics`, `ShapedGlyphRun`, `ShapedTextLine`, `TextMeasurer`, and `SystemFontSet` (`lumencraft-text` **`50 passed`**, `wasm32` `-D warnings` pass, `chart`/`pdf`/`render`/`svg` `147 passed`), and Phase 2 (`4e6b8b2`) landed L6 `TextMeasurer` + 4x4 coverage-mask glyph atlas (`crates/ui-egui/src/text_bridge.rs`, retiring `FONT_5X7` in core views via `r10_core_and_spike::r10_9_text_measurer_and_coverage_atlas_retire_font_5x7`); full 16-locale glyph coverage gate across remaining Wave 5/6 chrome stays open.
- [ ] Retire the uppercase 5×7 ASCII glyph table across chrome, grids, results, inspectors, and modals; route all UI text through mixed-case proportional UI typography and tabular-numeral grid text from system fonts (zero bundled font files).
- [ ] Full Unicode coverage for Greek, mathematical symbols, sub/superscripts, accented Latin, CJK, and bidirectional RTL scripts, crisp at `1.0x`–`3.0x`.
- [ ] **Glyph coverage gate:** zero missing-glyph fallback boxes across all `16` locales, light/dark/high-contrast themes, and HiDPI densities, checked from the layout tree's shaped text runs on desktop and headless runs; on web, where the browser picks fallback fonts, covered by `R10.13` manual QA.
- [ ] Keep deterministic export screenshots (`cargo xtask screenshots --check`) working, regenerating `docs/images/*.png` and `ASSETS.md` rows when the chrome changes.

### R10.10 Standalone "Analyze a stack of *P* values" analysis
Owner: open · Status: done · Was: Human-GUI audit 2026-11-15 (missing analysis) · Est. note: 10–16 h

- [x] Added standalone `PValueStack` analysis (`crates/stats/src/pvalue_stack.rs`, `crates/doc/src/analysis.rs`) delegating to `crates/stats/src/multiplicity.rs` across Bonferroni, Holm, Šídák, Holm–Šídák, Benjamini–Hochberg (1995), Benjamini–Krieger–Yekutieli two-stage (2006), and Benjamini–Yekutieli (2001) with literature citations (`r1010_pvalue_stack_oracles::{hand_calculated_oracle_unsorted_with_ties_all_seven_methods, neuhaus_1992_from_bh_1995_differential_oracle_matrix, bky_stage1_zero_and_all_rejected_boundary_cases}`).
- [x] Enforced strict `[0, 1]` domain checks (`MathError::DomainError`, never coercing `NaN`/`±Inf`/out-of-range), `(0, 1)` thresholds, empty/excluded-column handling, `0.0`/`1.0` bounds, ties, and multi-column row alignment with `1e-12` `D1` oracle matrix (`r1010_pvalue_stack_oracles::{boundary_zero_and_one_inputs_across_all_methods, domain_errors_and_invalid_thresholds_never_coerce, empty_and_all_excluded_columns_and_row_alignment_in_datasheet, multi_column_selection_missing_column_and_incompatible_tables}`).
- [x] Registered `stats.pvalueStack` (`CommandSpec`, `96` engine commands, regenerated `docs/commands.md`), added `stats.pvalue_stack` in `ANALYSIS_CATALOG` + dialog state/i18n (`16×308` keys, `0` missing), and verified MCP/CLI/doc/engine/UI wiring (`prop_serde::pvalue_stack_spec_defaults_compatibility_and_serde_roundtrip`, `undo_and_methods_qol::r10_10_pvalue_stack_executes_updates_undoes_and_generates_methods_report`, `dialogs_canvas::r10_10_pvalue_stack_catalog_analyze_and_edit_params_roundtrip`, `mcp_surface::r10_10_analysis_toolset_exposes_and_executes_stats_pvalue_stack`, `cargo xtask commands --check`, `cargo xtask i18n-report`; human-operable rendered dialog controls remain under `R10.3`).

### R10.11 Compact Letter Display (`CLD`)
Owner: maintainers · Status: done · Was: previously excluded (`R9.16`/`QOL-16`); re-scoped by the human-GUI audit · Est. note: 20–32 h

- [x] **Decision resolved (2026-11-15):** maintainers confirmed reversal of the earlier product-scope exclusion of Compact Letter Display (required by the reference CLD workflow, `WF-50`). `R9.16` stays equations-only.
- [x] Implemented clean-room Piepho (2004) insert-and-absorb + redundant-column sweeping (`crates/stats/src/cld.rs`) consuming all-pairs `PairwiseComparisonRecord` outputs (`Tukey`, `Games–Howell`, `Dunn`, `Šídák`, `Bonferroni`, `Holm–Šídák`, `Newman–Keuls`, etc.), with `D1` paper examples, non-monotonic ties, all-different/all-equal/single-group/empty/`>26`-letter edge cases, and fuzz equivalence/irredundancy invariants (`r1011_cld_piepho_oracles::{piepho_2004_standard_overlapping_monotonic_groups, piepho_2004_non_monotonic_unbalanced_ties_oracle, piepho_2004_four_group_non_monotonic_diamond_oracle, piepho_2004_sweeping_eliminates_redundant_maximal_clique, piepho_2004_unequal_sample_sizes_multi_gap_oracle, edge_cases_empty_single_all_equal_all_different_and_over_26_letters, consumes_all_pairwise_multiplicity_procedures_and_alpha_override, consumes_anova_and_kruskal_wallis_dunn_result_sheets_and_rejects_dunnett, deterministic_fuzz_symmetric_matrices_satisfy_equivalence_and_irredundancy, rejects_non_finite_duplicate_and_inconsistent_inputs_cleanly}`).
- [x] Added `CompactLetterDisplaySpec`/`PairwiseDisplayMode` graph annotations (`crates/doc/src/graph.rs`, `crates/chart/src/{column_grouped,multivariate_layout,survival_cat}.rs`), `graph.setCompactLetterDisplay` (`CommandSpec` #97, regenerated `docs/commands.md`), `PairwiseComparisonsState` & 16-locale `i18n.rs` (`309` keys = `4,944`), deterministic SVG/`%PDF-1.7`/RGBA rendering, and strict `--gui-only` `WF-50` documented under the agent-harness matrix (`Human path = 0 / 1`; `svg_export::cld_compiled_graph_exports_deterministic_editable_and_outlined_svg`, `pdf17_truetype_embed::cld_compiled_graph_exports_deterministic_pdf17_with_selectable_and_outlined_letters`, `render_export::cld_compiled_graph_rasterizes_deterministically_into_rgba_annotation_band`, `gui_wf_50_cld::wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba`, `docs/gui-workflows.md`; interactive `R10.18` right-hand inspector & `R10.3` rendered modal controls remain tracked in `R10.18`/`R10.3`).

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
Owner: open · Status: in progress · Was: GUI alignment plan 2026-11-15 (Phase 1, Phase 7); scope clarified 2026-11-15 · Est. note: 6–10 h

The existing Alpha surfaces must first pass the separate strict launch gate in [`R0.21`](#r021-strict-alpha-branding-and-asset-provenance-gate); this item continues coverage for new R10 surfaces and does not make GUI readiness a prerequisite for Alpha.

LumenCraft's own name, wordmark, icon, and metadata are permitted on every surface (toolbar badge, Welcome and About screens, window and browser titles, file-dialog labels, PDF creator/producer strings, packaging). This item keeps **external** branding out: no product names, logos, wordmarks, watermarks, icon designs, colour schemes, dialog copy, sample-dataset names, or tutorial captions taken from the reference application or any other vendor, including anything visible in the reference video frames.

- [ ] Review every UI surface added or restyled under `R10` against the reference frames used to design it, removing any external names, logos, icons, distinctive wording, or styling; record the review in the PR.
- [ ] **External-branding gate:** extend `cargo xtask brands` from source files to UI strings (`i18n` catalog, compositor literals, window/document/export metadata) and every text run in the rendered layout tree, failing on any external product, suite, or vendor name. Third-party file-format extensions (`.pzfx`, `.prism`) stay allowed only in import/export file-type lists, per [`AGENTS.md`](../AGENTS.md). *(Phase 1 landed in [`xtask/src/brands.rs`](../xtask/src/brands.rs): `check_i18n_catalog`, tightened UI/app `.prism` extension rules, `check_ui_surfaces`, `check_{svg,pdf,html,manifest}_metadata`, `check_window_title`, and reusable `check_text_runs` / `check_text_runs_strict` / `check_labeled_text_runs`; `xtask brands` passes across `634` tracked files incl. `4,992` `i18n` entries, `26` dialogs, `7` inspectors, and export metadata; `xtask` **`116 passed`** incl. `brands::tests::{checks_export_and_window_metadata, checks_text_runs_and_labeled_text_runs}`; wiring live rendered `LayoutTree` frames into `check_text_runs` remains open for Wave 5/6.)*
- [x] Required every new icon, schematic diagram, thumbnail, and screenshot baseline to have an `ASSETS.md` row declaring original LumenCraft provenance (or a valid original parent-asset provenance chain) while preserving `approved-seven-pane.png` and `lumencraft-win95-2508.png` in [`xtask/src/assets.rs`](../xtask/src/assets.rs) (`is_original_only_visual_asset`, `verify_assets`, `assets::tests::enforces_original_provenance_on_ui_icons_schematics_thumbnails_and_screenshots`, `cargo xtask assets` passing on `26` attributed asset files).

### R10.15 Aspect-ratio-preserving canvas viewport
Owner: open · Status: in progress · Was: GUI alignment plan 2026-11-15 (Phase 1) · Est. note: 10–16 h · Depends on: `R10.16`

- [ ] Render graph and page-layout sheets at their true physical page aspect ratio, centred on a neutral workspace mat, with zoom-to-fit, an explicit zoom-percentage selector, and pan/scrollbars, instead of stretching scenes to fill narrow split panes. *(Phase 2 `crates/ui-egui/src/layout/canvas_viewport.rs` aspect-ratio viewport landed in `4e6b8b2` and verified via `r10_core_and_spike::r10_15_aspect_ratio_viewport_and_hover_selected_only_chrome`; full UI zoom selector & interactive Wave 5/6 sheets stay open.)*
- [ ] Hide series-palette debug chips and layout-panel selection borders/badges unless the element is hovered or selected. *(Phase 2 hover/selected-only chrome gating verified in `r10_core_and_spike::r10_15_aspect_ratio_viewport_and_hover_selected_only_chrome`; Wave 5/6 sheet rollout continues.)*

### R10.16 Reusable widget library, modal infrastructure & universal scrolling
Owner: open · Status: in progress · Was: GUI alignment plan 2026-11-15 (Phase 2) · Est. note: 60–100 h · Depends on: `R10.1`, `R10.9`

- [ ] Widgets with hover, active, focused, and disabled states on the shared layout tree: push buttons, icon-and-label toolbar buttons, segmented/tab strips, radio groups, checkboxes, scrollable dropdowns, numeric spinners and sliders, single- and multi-line text fields (caret, selection, clipboard, IME), disclosure trees, splitters, and scrollbars. *(Phase 2 `crates/ui-egui/src/layout/{mod,widgets,hit_test,paint,verify}.rs` & `views/mod.rs` core widget library landed in `4e6b8b2`, passing `r10_core_and_spike::r10_2_and_r10_16_parity_across_views_plus_textbox_ime_clipboard_dropdown_and_scroll`; Wave 5 dialog/inspector consumer surfaces stay open.)*
- [ ] Modal z-order with pointer capture (no click-through), focus trapping and `Tab`/`Shift+Tab` cycling with visible focus rings, `Enter` to confirm, `Escape` to cancel; disabled tabs and controls show explanatory hover tooltips. *(Phase 2 `crates/ui-egui/src/layout/a11y.rs` modal backdrop & focus trapping landed in `4e6b8b2`; Wave 5 `ActiveDialog` wiring stays open.)*
- [ ] Scroll containers with vertical/horizontal scrollbars and wheel/trackpad scrolling for navigator lists, data grids, result tables, info sheets, inspectors, and dialog trees, removing every fixed 4–8 item truncation cap. *(Phase 2 `crates/ui-egui/src/layout/scroll.rs` `ScrollMeta` containers landed in `4e6b8b2`; full multi-tier grid/result/dialog surface scroll wiring continues in Wave 5.)*

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
Owner: open · Status: in progress · Was: GUI alignment plan 2026-11-15 (Phase 6) · Est. note: 16–28 h · Depends on: `R10.9`

- [x] Upgraded `GraphAnnotation::ModelEquation` (`crates/chart/src/equation_box.rs`, `crates/fit/src/tex_format.rs`) to recursive two-dimensional math box layout citing Knuth (*The TeXbook*, 1984) Chapter 17 & Appendix G (Rules 11 `\sqrt`, 15a–15e `\frac`/`\binom`, 18a–18f stacked scripts, 19 scaled delimiters, math-axis $\sigma_{22}$ alignment, Greek operators, and live substituted best-fit coefficients) emitting `ScenePrimitive` geometry shared across RGBA, SVG, and `%PDF-1.7` (`r1019_typeset_equations::{appendix_g_rule_15_nested_fractions_and_math_axis_alignment, appendix_g_rule_18_stacked_sub_sup_and_nested_scripts, appendix_g_rule_19_scaled_delimiters_cover_tall_subexpressions, appendix_g_rule_11_square_root_surd_and_overbar_clearance}`, `tex_format::tests::greek_parameters_fractional_exponents_and_operators_format_for_2d_math`; `lumencraft-chart` **`93 passed`**, `lumencraft-fit` **`158 passed`**, `lumencraft-engine` **`185 passed`** incl. `wave22_integration::graph_add_annotation_model_equation_resize_clone_and_live_recalc`, `pdf` **`19`**, `render` **`20`**, `svg` **`15`**, `ui-egui` **`187`**).
- [ ] Deterministic rendering tests per output format, plus live coefficient refresh on upstream edits (extends `WF-49`). *(Scene-level bit-identity, backend readiness, and upstream table live refresh pass in `r1019_typeset_equations::{greek_parameters_builtins_polynomials_dsl_and_determinism, wf49_live_coefficient_refresh_on_upstream_table_edits_across_models}`; explicit per-format RGBA, SVG, and `%PDF-1.7` deterministic byte assertions for 2D `ModelEquation` scenes remain for Wave 6 alongside `R10.9` completion.)*


### R10.20 Keyboard & screen-reader accessibility
Owner: open · Status: todo · Was: Roadmap survey 2026-11-15 · Est. note: 16–28 h · Depends on: `R10.16`

- [ ] Expose the shared layout tree to the OS accessibility APIs (VoiceOver, NVDA, Orca) on desktop, and to the browser accessibility tree on web.
- [ ] Gate: every interactive control has a role, a label, and a keyboard path; checked in CI from the layout tree.

### R10.21 Translations for new UI strings
Owner: open · Status: todo · Was: Roadmap survey 2026-11-15 · Est. note: 20–40 h · Depends on: all `R10` items that add strings

- [ ] Translate every label, tooltip, and dialog string added under `R10` into all `16` locales.
- [ ] Gate: `cargo xtask i18n-report` shows `0` missing keys before `R10.13` can pass.

### R10.22 UI performance & size budgets
Owner: open · Status: in progress · Was: Roadmap survey 2026-11-15 · Est. note: 8–14 h · Depends on: `R10.1`

- [x] **Decision resolved (2026-11-15):** budgets — frame time ≤ `16.67 ms` for the full chrome plus a `1M`-cell grid; input-to-paint latency ≤ `50 ms` from click or keypress to updated frame; web release zip ≤ `6,243,591` B (1.5 × the `4,162,394` B `lumencraft-web-0.1.0.zip`); startup to first interactive frame < `3 s` on desktop and web, measured on the maintainers' reference macOS machine.
- [ ] Check the budgets in CI alongside the existing `R7.2` performance tests. *(Phase 2 wired `cargo xtask r10-gate` into `cmd_ci` [`4e6b8b2`], checking `r10_core_and_spike::r10_22_one_million_cell_virtualized_grid_frame_budget` `<= 16.67 ms` frame & `<= 50 ms` latency; startup `< 3 s` & web bundle CI budget gates remain open for Wave 5/6.)*

---

## Order and dependencies

| Step | Items | Depends on | Target gate |
|---|---|---|---|
| **Toolchain baseline — R0.18** | Rust `1.99.1` minimum across all `22` packages; pinned local/CI/coverage/packaging toolchains and local verification (**done**; hosted CI/FreeBSD/Windows ARM64 runners open) | None; preserve existing claims | Build-toolchain baseline (**done** locally) |
| **MCP functionality — R7.3** | Fully functional MCP for existing engine capabilities (**done**): real stdio/headless workflow parity (`WF-01`–`WF-50`), protocol coverage, and opt-in `--window` shared visible session | `R0.18`, `R7.1`, `R8.3`, `R8.6` (done) | **Alpha functional scope** (implemented; known bugs acceptable) |
| **MCP onboarding — R7.4** | Short beginner MCP guide in a minimal, presentable README; verified first-run instructions (**done**) | `R7.3` (done) | Documentation handoff before GUI (**done**) |
| **Alpha launch identity — R0.20** | Full CurveBook identity, native-format, desktop/browser/MCP, packaging, and documentation cutover; [rename plan](../plan/curvebook-rename.md) | `R0.18`, `R7.3`, `R7.4` (done); coordinate current audit/GUI claims | **Alpha launch requirement** (in progress); precedes `R0.19` Wave B & further unclaimed R10 work |
| **Alpha branding clearance — R0.21** | Strict branding/naming and original-asset/provenance checks; only `.pzfx` / `.prism` file-format handling is excepted | `R0.20`; reuse existing guards and verify actual launch outputs | **Mandatory Alpha launch gate** (open); known bugs do not waive it |
| **Step 1 (8A)** | `R0.1`, `R0.2` (done); foundations for `R0.3`, `R0.5`, `R0.6`, `R0.7`, `R0.10`, `R0.14` | — | Scaffold |
| **Step 2 (8B)** | `R0.3` (secrets gate), `R0.4` (pending/status), `R0.5` (licenses gate — done), `R0.6` (Zero Telemetry & local `actionlint`/`shellcheck` done; hosted GH Actions run open) | `R0.2` for `R0.4` | Scaffold |
| **Step 3** | `R0.10` (`lumencraft-platform` wiring & source checks), `R0.11` (versioned files) | `R0.6` | Scaffold |
| **Step 4** | `R0.8` (`CommandSpec` registry, area by area), then `R0.9` (structured errors & remediation) | — (runs beside Step 3) | Engine contract |
| **Step 5** | `R0.7` (generated `commands.md` & `schemas`), `R0.12` (format registry) | `R0.8` (commands), `R0.11` (schemas) | Docs & seams |
| **Step 6** | `R0.13` (MCP/CLI/UI alignment) | `R0.8`, `R0.9` | Surfaces |
| **Step 7** | `R8.3` (live `eframe`/`egui`/`wgpu` desktop window — **done**), `R8.4` (`wasm32` Trunk WebGL2 bundle — **done**) | `R8.1` (done) | Desktop/WASM presenters (**done**); human GUI readiness tracked separately in `R10` |
| **Step 7B** | `R8.6` (`ui.inspect`, `gui` MCP toolset & `--gui-only` lock — **done**), `R8.7` (Power & Sample Size wizard, 10 modal predicates & chained GUI surfaces — **done**), then `R8.8` (4-shard 28-workflow `gui_only` acceptance suite — **done**) | `R8.1` (done), `R0.13` (done) | **Agent-harness workflows** (`R8.6`–`R8.8` done) |
| **Step 8** | `R5.4` (native binary OS clipboard, `ActiveDialog::Print` & WYSIWYG layout drag-snap — **done**) | `R8.1` | Beta clipboard/print/layout capabilities (`R5.4` done; Beta launch not claimed) |
| **Step 9** | `R8.5` (universal macOS `.dmg`/`.zip` `verify.sh` done locally; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA open), `R0.14` (`README.md` screenshots, `release.yml` `tags: ["v*"]`, `cargo-about`/`cargo-deny`, & local universal macOS/web packaging done; hosted `v0.1.0-rc1` CI tag/matrix open) | `R0.5` (done), `R8.3` (done), `R8.4` (done), `R8.8` (done), **`R10.13`** | **1.0** |
| **Microsoft Store distribution — R8.9** | MSIX packaging, local self-signing workflow, Windows certification checks, and Store listing/submission; guidance documented, implementation pending | Packaging preparation can start now; public submission needs `R0.14`, `R8.5`, `R10.13`, and maintainer-provided Partner Center identity | **Microsoft Store release** (separate distribution target) |
| **Step 10 (M8+)** | `R9.7` (diagnostics-only banners), `R9.12` (table-analysis completions), `R9.14` (randomization & sampling), `R9.16` (equation callouts only), `R9.17` (extended 21-workflow `gui_only` suite `WF-29`–`WF-49`); `R9.10`, `R9.11`, `R9.13`, and `R9.15` excluded from active sequencing | `R0.8` (done), `R3.1`–`R3.3` (done), `R4.4` (done), `R5.1` (done), `R8.8` (done) | **Done (active scope)** across Waves 20–23 (`0 h` remaining); deferred items unscheduled |
| **Step 11a (`R10` Phases 1–2)** | `R0.15` (evidence-backed done markers; can land now), `R0.16` (local coverage reports; can land now), `R10.1` (in-house layout tree & browser-font decisions made; spike), `R10.22` (performance & size budgets), `R10.14` (external-branding exclusion), `R10.9` (typography & glyph gate), `R10.16` (widgets, modals, scrolling), `R10.15` (canvas viewport), `R10.2` (human-path harness & pixel-to-hit-box gate) | `R0.20` (CurveBook cutover), `R7.3` (existing-capability MCP gate, done), `R7.4` (README handoff, done), then `R8.3`, `R8.4`, `R8.6` (done) | **Human GUI workflows** |
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

**Alpha policy (2026-11-16):** Alpha means full MCP functionality for the currently implemented engine scope, launched as **CurveBook**. Through the actual MCP transport, users must be able to discover and run the supported data, analysis, fitting, graph/layout, persistence, preview/export, batch, resource, and prompt workflows, with the documented modes and structured failures. `R7.3` supplies the recorded functional acceptance evidence; rerun that acceptance and the `R7.4` beginner setup under the final renamed binaries before launch. Deferred features and future engine additions are outside the current Alpha scope.

**Known bugs are acceptable for Alpha**, including documented audit findings. Alpha does not promise production reliability, a stable interface, or completion of `R0.19`; keep failures, limitations, and scientific caveats visible in the known-issues record and report only checks actually run. This milestone definition does not waive the project's scientific, clean-room, or engineering rules. In particular, prohibited branding/naming or asset references are launch blockers, not acceptable known-bug exceptions.

**Strict branding and asset-provenance clearance (`R0.21`) is also required for Alpha launch.** No reference-suite product/vendor branding or naming in any casing, and no mentions or use of its proprietary assets, may remain in the reviewed source, UI/MCP/documentation/export surfaces or launch artefacts. Only `.pzfx` / `.prism` file handling and its indispensable parser/exporter tokens are excepted at the format boundary. Require strict automated checks plus original-asset and rendered-output review on the final launch revision; ordinary guard passes with unchecked exclusions do not establish clearance. Known bugs do not waive this gate, and it does not depend on future GUI readiness.

**The full CurveBook rename (`R0.20`) is required for Alpha launch.** MCP functionality under the old identity alone does not complete the launch gate. Human GUI readiness (`R10`), full signed cross-OS distribution, and target-hardware/browser release QA remain separate later milestones; neither a visible window nor human-completable GUI workflows is required for Alpha. Existing presenter and agent-harness achievements retain their evidence, independently of the pending Alpha launch.

| Release Gate | Required Roadmap Items | Status & Remaining Work |
|---|---|---|
| **Alpha** (full MCP functionality, known bugs acceptable; CurveBook identity and strict branding clearance) | [`R7.3`](#r73-mcp-workflow-parity-proof--shared-live-window) functional MCP scope + [`R7.4`](#r74-accessible-mcp-guide-and-minimal-readme) onboarding + [`R0.20`](#r020-rename-lumencraft-to-curvebook) full rename + [`R0.21`](#r021-strict-alpha-branding-and-asset-provenance-gate) strict naming/asset-provenance clearance; repeat acceptance under the final names | **Open for launch.** MCP/onboarding implementation is recorded done; CurveBook rename, strict branding/asset clearance, and final renamed MCP/setup verification remain pending. Known bugs may remain documented but cannot waive branding/asset violations; no GUI-readiness or full audit-closure requirement. |
| **Agent-harness workflows** (formerly **Core workflows**) | Existing engine/session + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) event loop + [`R8.6`](#r86-semantic-ui-inspection-gui-mcp-toolset--strict---gui-only-lock)–[`R8.8`](#r88-clean-room-28-workflow-gui_only-acceptance-suite) semantic `gui_only` workflow parity | **Done for the agent harness only** (`ui.inspect`, 8-tool `gui` MCP mode, `--gui-only`, Power wizard state machine, `R1`–`R10` modal predicates, `FormatSelectedPoints`, `ResultSheet` `Analyze` chaining, & 28-workflow `gui_wf_*` suite). Not a human-usability gate. |
| **Human GUI workflows** (separate readiness milestone) | [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) / [`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) presenters + [`R10.1`](#r101-interactive-chrome-architecture-decision--spike)–[`R10.22`](#r1022-ui-performance--size-budgets), accepted through [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa) | **Open; not an Alpha launch requirement.** Needs current human-path and manual acceptance evidence. Keep the dated audit baseline and landed engine/GUI work distinct from a claim of full human GUI readiness. |
| **Beta** (OS vector clipboard/print/layout drag) | **Alpha** + **Agent-harness workflows** + [`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) | **Capabilities implemented; Beta launch not claimed.** `R5.4` is done (live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux adapters, `ActiveDialog::Print`, WYSIWYG layout drag-snap) |
| **1.0** (human-operable GUI, full signed installers & target-OS QA) | **Beta** + **Human GUI workflows** + [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) + [`R0`](#r0-align-with-house-principles) release gates | **Human GUI workflows (`R10`) open.** Local `R0.5`, `actionlint`/`shellcheck`, `README.md` screenshots, `release.yml` `tags: ["v*"]`, `cargo-about`/`cargo-deny`, `lumencraft-web-0.1.0.zip`, and universal macOS `.dmg`/`.zip` `package.sh` + `verify.sh` done; Apple Developer ID notarization, Windows/Linux/FreeBSD VMs & live browser QA (`R8.5`), hosted GH Actions CI (`R0.6`), & `v0.1.0-rc1` tag/matrix (`R0.14`) open |

---

## Parked

Items explicitly deferred until needed or post-`1.0`:

- **Translated READMEs:** localized `README.<locale>.md` translations are product polish deferred beyond initial `1.0` release scaffold (`R0.14`).
- **Mutex wrapper & locks check:** `clippy.toml` already disallows raw `std::sync::Mutex`; a deadlock-detecting lock wrapper and `xtask locks` check stay parked while the workspace remains single-owner / message-passing.
- **Plugin SDK / dynamic plugins:** deferred (`**decision**`; closed domain seams use typed traits and registries inside the workspace).
