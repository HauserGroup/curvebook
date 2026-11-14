# LumenCraft Roadmap

Claimable engineering roadmap for **LumenCraft**. For current verified capabilities, test evidence, and known limits, see [`STATUS.md`](STATUS.md). For historical wave-by-wave progress logs, see [`roadmap-history.md`](roadmap-history.md).

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
| **R0** | Align with house principles & typed engine scaffold | Phases A–E | in progress | `R0.1`–`R0.5`, `R0.7`–`R0.13` done (Waves 8A–14); `R0.6` (Zero-Telemetry policy & local `xtask telemetry` done; remote GH Actions `actionlint` open), `R0.14` (clean-room PNG screenshots & local `LumenCraft.app` build/`plutil`/`codesign` done; non-sandboxed `.dmg`/`.zip` `verify.sh`, `v0.1.0-rc1` CI tag & `cargo deny`/`cargo about` run open) in progress |
| **R1** | Workspace scaffold, numerical foundations & NIST StRD oracle | `M0`, `M1` (partial), `G1` | done | `58` NIST StRD datasets + `D1` R `4.5.2` oracle matrices verified (`0 h`) |
| **R2** | Typed data tables, reactive DAG, persistence & `.prism`/`.pzfx` import | `M1`, `M2`, `G2`, `G10` | done | Waves 1–6 incl. `.lumencraft.d/`, `G2` & `G10` fixes verified (`0 h`) |
| **R3** | Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate | `M2`, `M6`, `G3`, `G8` | done | Waves 2–5 incl. `BarnardExact`, `PassingBablok`, MICE, `G8` Koopman fix (`0 h`) |
| **R4** | Nonlinear regression solver, `117` equations & custom DSL/ODE | `M3`, `M6`, `G4`, `G9` | done | Waves 2–5 incl. `47` pharma models, `MAX_AST_NODES`, iterative `Expr` (`0 h`) |
| **R5** | Publication graphing, vector SVG/PDF 1.7/TIFF/EMF & Page Layouts | `M4`, `M6`, `S2`, `G5`, `G11` | done | `R5.1`–`R5.4` & `G5` done (Waves 2–13): Poppler+Ghostscript PDF QA, live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux vector clipboard adapters, `ActiveDialog::Print` + OS printer discovery/preview, and live WYSIWYG layout drag-snap/resize grips |
| **R6** | Multiple Variables workflows, Calculated Variables & Multifactor ANOVA | `M6`, `G7` | done | Waves 2–4 end-to-end verified (`0 h`) |
| **R7** | Automation, headless CLI, `--control`, MCP & performance budgets | `M5` (headless), `M6`, `Slice 0`, `G12`, `G14` | done | `92` engine commands (`90` Wave 11–14 + Wave 15 `sheet.extractResultTab` & `graph.formatSelectedPoints`), `96`-curve 4PL `< 50 ms`, `G12` read-only skip & bounded undo (`0 h`) |
| **R8** | Live desktop & WASM UI shell, GUI-agent parity, localization QA & `1.0` hardening | `M5` (UI), `M7`, `S1`, `G6`, `G13` | in progress | `UiApp`/HiDPI/RTL/`WebWorkspaceHost`, `DesktopHostRuntime` (`R8.3` adapters), browser storage/locks/FS (`R8.4` adapters), `cjk_rtl_hidpi_desktop_qa`, arm64 `LumenCraft.app` `plutil`/`codesign`, & `gui_only` agent parity/Power wizard/28-workflow suite (`R8.6`–`R8.8`, `16×291` i18n, `32` `UI_COMMANDS`) done; external `eframe`/`wgpu` GPU crate link (`R8.3`), `wasm32`/`trunk` build (`R8.4`), & non-sandboxed `.dmg`/`.zip` + Windows/Linux/FreeBSD VM QA (`R8.5`) open |
| **R9** | Scientific quality-of-life scope | `M8+`, `QOL-1`–`QOL-17` | in progress; scope revised | Existing Wave A code retained; `R9.7` requires diagnostics-only alignment. Active Wave B: `R9.12`, `R9.14`, equations-only `R9.16`, and `R9.17` (`WF-29`–`WF-49` extended `gui_only` suite). `R9.10`, `R9.11`, `R9.13`, and `R9.15` deferred without scheduled milestones ([scope](qol-beyond-parity.md)) |

**Gate summary:** Headless **Alpha**, **Core**, and **Beta** scientific/engine/CLI/MCP/`UiApp`/`R5.4` workflows — plus full **`R8.6`–`R8.8` `gui_only` workflow parity** (`ui.inspect`, 8-tool `gui` MCP mode, `--gui-only` lock, 4-stage/8-family Power & Sample Size wizard, 10 modal predicates, `FormatSelectedPoints`, `ResultSheet` `Analyze` chaining, and the 28-workflow `gui_wf_*.rs` acceptance suite) — pass today; reaching live GPU desktop **Alpha** (`R8.3` external `eframe`/`wgpu` crate linkage), browser **Alpha** (`R8.4` `wasm32`/`trunk` release bundle), and signed cross-OS **1.0** (`R8.5` non-sandboxed macOS `.dmg`/`.zip` `verify.sh`, Windows/Linux/FreeBSD VMs & live browser QA alongside `R0.6` remote CI `actionlint` and `R0.14` `v0.1.0-rc1` CI tag/matrix) remains open.

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

- [x] Add `.gitattributes`, workspace `publish = false` (`publish.workspace = true` across library crates), `unsafe_code = "forbid"`, `[profile.web-release]`, `clippy.toml` (`allow-indexing-slicing-in-tests`, `disallowed-types` for `std::sync::Mutex`), `.github/actionlint.yaml`, remove dead `.github/workflows/nix.yml`, wire `cargo xtask telemetry` (zero telemetry/egress/analytics + workflow `permissions` / `${{ }}` hygiene) into `cmd_ci`, and invoke `actionlint` + `shellcheck` in `.github/workflows/ci.yml` (Waves 8A, 13–14). **Decision resolved (Zero-Telemetry policy):** SonarCloud and all external cloud scanners/telemetry services are permanently excluded.
- [ ] Verify CI workflows (`actionlint` & `cargo xtask ci`) pass cleanly on live GitHub Actions runners (`actionlint` is not installed locally in this environment, so external live workflow lint/execution remains remote CI-only while `cargo xtask telemetry` and local CI gates pass).

### R0.7 Generated command & schema docs
Owner: open · Status: done · Was: Phase B5

- [x] Author hand-written `docs/command-ids.md` and `docs/file-formats.md` (Wave 8A).
- [x] Implement `cargo xtask commands [--check]` generating `docs/commands.md` from `R0.8`'s typed registry and wire `--check` into `cargo xtask ci` (Wave 11).
- [x] Implement `cargo xtask schemas [--check]` generating `docs/schemas/*.schema.json` (archive manifest, document, batch v1, preferences, journal record) after `R0.11` and wire `--check` into `cargo xtask ci` (Wave 11).

### R0.8 Typed `CommandSpec` registry
Owner: open · Status: done · Was: Phase C1

- [x] Replace `CommandDescriptor`, `catalog::COMMANDS` (`90` entries), and string `match` dispatch in `cmd_data.rs` / `cmd_sci.rs` with `crates/engine/src/cmd/mod.rs` (`CommandSpec`, `CommandGroup`, `Effects`, `Ctx`, `Registry`), migrating area-by-area with `serde_path_to_error` typed params and preserving WAL / `lumencraft.batch.v1` compatibility (Waves 9–11; expanded to `92` entries in Wave 15 with `sheet.extractResultTab` & `graph.formatSelectedPoints`).
- [x] Convert `UI_COMMANDS` (`29` entries in `crates/ui-egui/src/menus/ui_commands.rs`) to a typed `UiCommandSpec` struct and regenerate `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (Wave 9; expanded to `32` entries through Wave 16).

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

- [x] Expand `docs/releasing.md`, add `packaging/macos/verify.sh`, `packaging/linux/apprun-test.sh`, `packaging/windows/check-icons.ps1`, pass `--cargo-profile web-release` in `packaging/web/package.sh`, generate `THIRD-PARTY-LICENSES.html` in `release.yml`, and modernize `README.md` with badges, section nav, Downloads, `platform/`, Zero-Telemetry callout, and one-line Status (Waves 8A, 14).
- [x] Generate clean-room `1440x900` Light Split Workspace and Dark Multi-Panel Layout PNG screenshots (`crates/ui-egui/examples/doc_screenshots.rs`, `cargo xtask screenshots [--check]`, `docs/images/workspace-split-fit.png`, `docs/images/workspace-layout-dark.png`, `ASSETS.md`, `README.md`) and run local macOS `arm64` release app validation (`LumenCraft.app` `24M`, `lumencraft-cli 0.1.0`, `plutil -lint OK`, ad-hoc `codesign --verify --strict --deep valid on disk`) (Wave 14).
- [ ] Run non-sandboxed macOS `.dmg`/`.zip` `package.sh` + `verify.sh` dry-run (`hdiutil create` failed in sandbox with `Operation not permitted`), tag `v0.1.0-rc1` across the 5-OS GitHub Actions release matrix, and verify live `cargo deny`/`cargo about` CI execution.

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

### R3.3 Survival, Cox PH, contingency (Barnard/Koopman), method comparison, ROC & GLMs/PCA/MICE
Owner: maintainers · Status: done · Was: G8, M6 · Est. note: 0 h remaining

- [x] Land Kaplan–Meier, log-rank/Gehan–Breslow/trend, Cox PH (`BaselineSurvival`/`LogMinusLog`), contingency ($\chi^2$, Fisher, `BarnardExact`, Baptista–Pike, Wilson, Clopper–Pearson, Cochran–Armitage, McNemar), `BlandAltman`, `PassingBablok`+CUSUM, ROC (DeLong/Hanley–McNeil), multiple linear/logistic/Poisson GLMs, PCA/clustering, `MultipleImputation` (MICE + Rubin), and Wave 5 `G8` Koopman `Option<f64>` `None`-bound fix (`lumencraft-stats` `69 passed`).

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

- [x] Land `Slice 0` vertical gate, `lumencraft-cli` (`31 passed` Wave 14 -> `35 passed` Wave 17, incl. `.lctemplate`, `perf --json`, `.lumencraft.d`, `.emf`, & `mcp_gui_only_parity`), `lumencraft --control` (`17 passed` Wave 14 -> `19 passed` Wave 17 across `lumencraft` incl. `gui_only_wiring`), `lumencraft-mcp` (`33 passed` / `30` synced `UI_COMMANDS` Wave 14 -> `39 passed` / `32` synced `UI_COMMANDS` Wave 17 incl. `gui_only_surface`), and `90` -> `92` registered engine commands (Waves 2–17).

### R7.2 Performance budgets, `1M`-cell virtualization & property/fuzz suites
Owner: maintainers · Status: done · Was: G12, M6 · Est. note: 0 h remaining

- [x] Verify `< 50 ms` `96`-curve 4PL batch solve (`--release`), `< 250 ms` `100k` Calculated Variables, `1M`-cell `S1` grid (`< 1 ms` median frame), 7 property/fuzz harnesses, and Wave 6 `G12` read-only clone-skip & bounded undo (`MAX_UNDO_ENTRIES = 64`, `MAX_UNDO_CELL_BUDGET = 25_000_000`).

---

## R8 Desktop & WebAssembly live shell, localization & `1.0` hardening

### R8.1 Pure-Rust `UiApp` shell, software RGBA compositor, HiDPI/RTL input & `WebWorkspaceHost`
Owner: maintainers · Status: done · Was: G6 (shell), S1, M5 (partial) · Est. note: 0 h remaining

- [x] Land `lumencraft-tools` (`21 passed`, **S1 GO**), `lumencraft-ui-egui` (`83 passed` Wave 14 -> `122 passed` Wave 17, `UiApp`, RGBA/PNG compositor incl. `1440x900` `doc_screenshots`, `30` -> `32` `UI_COMMANDS`, `NavigatorDragState`, `PortfolioGalleryState`, `F6` pane cycle, rulers, `ShortcutRegistry`, `ViewportMetrics` `1.0x`–`3.0x` HiDPI, `WorkspaceLayoutMap`, `UiInputEvent`, `InlineEditorState`, `VirtualWorkspaceFolder`), and `apps/lumencraft-web` `WebWorkspaceHost` (`13 passed`, `#![forbid(unsafe_code)]`, zero OS calls).

### R8.2 `16`-locale i18n, RTL mirroring & desktop CJK/RTL font/pasteboard checks
Owner: maintainers · Status: done · Was: G13 (core & macOS QA), M7 (partial) · Est. note: 0 h remaining

- [x] Land `16` locales × `291` keys = `4,656` translations (`0` missing catalog or command keys in `cargo xtask i18n-report`; expanded from `287` keys / `4,592` translations in Wave 14), RTL UI mirroring (`ar`/`he`), locale decimal/CSV presets, Poppler+Ghostscript CJK/RTL `%PDF-1.7` QA, and live macOS `pbcopy`/`pbpaste` `de-DE` comma + `=A1*2` formula roundtrip (`crates/ui-egui/tests/macos_clipboard_live.rs`). Remaining live OS target-host verification joins `R8.5`.

### R8.3 Live OS `eframe`/`egui`/`wgpu` desktop windowing & interactive event loop
Owner: open · Status: in progress · Was: G6 (desktop **Alpha** + **Core**), M5

- [x] Implement `DesktopHostRuntime` (`apps/lumencraft/src/desktop_host.rs`) with `--in-window-menus`, `--new-instance` isolated crash-recovery journals, Retina DPI transitions, IME composition, trackpad pinch/inertial pan, and file-drop integration tests (`desktop_host_loop`; `lumencraft` `19 passed`) (Wave 13).
- [ ] Link external `eframe`/`wgpu` GPU windowing crates across macOS, Linux (Wayland/X11), Windows, and FreeBSD once online `Cargo.lock` expansion is permitted (**Alpha**/**Core**).

### R8.4 Live `wasm32-unknown-unknown` Trunk WebGPU/WebGL2 browser hosting
Owner: open · Status: in progress · Was: G6 (WASM **Alpha**), M5

- [x] Implement `apps/lumencraft-web/src/host_adapters.rs` (`WebStorageManager` 5 MiB `localStorage` + forward-version guard, `WebLockCoordinator` multi-tab read-only contention fallback, `FileSystemAccessBridge` incremental `.lumencraft.d/` sync, `TouchGestureRecognizer`, and `WebBridgeSession` protocol; `lumencraft-web` `13 passed`) plus strict no-egress `index.html` and CSP/COOP/COEP headers (`packaging/web/{_headers,.htaccess}`) (Wave 13).
- [ ] Compile the `wasm32-unknown-unknown` / `trunk build --release` bundle (`cargo xtask wasm`) once `wasm32` sysroot and `trunk` are available (**Alpha**).

### R8.5 Cross-OS hardware, CJK/RTL desktop QA & installer/signing hardening
Owner: open · Status: in progress · Was: G13, M7 (**1.0**)

- [x] Implement macOS `Info.plist` `plutil -lint` validation and ad-hoc `codesign --force --deep --sign -` + `--verify --strict --deep` in `xtask/src/bundle.rs` (Wave 7), plus live macOS `NSPasteboard` `%PDF-1.7` roundtrip (`macos_clipboard_live`), `cjk_rtl_hidpi_desktop_qa` (`ja`/`zh-CN`/`ko`/`ar`/`he` across `1.0x`/`1.5x`/`2.0x`/`3.0x`, `4 passed`), and local macOS `arm64` `LumenCraft.app` (`24M`) + `lumencraft-cli` build/`plutil -lint`/`codesign` verification (Waves 13–14).
- [ ] Complete non-sandboxed macOS `.dmg`/`.zip` `package.sh` + `packaging/macos/verify.sh` verification (`hdiutil create` failed inside sandbox with `Operation not permitted`), Windows VM, Linux X11/Wayland VM (AppImage/deb/rpm/Flatpak), FreeBSD runner, and live browser QA runs.

### R8.6 Semantic UI inspection, `gui` MCP toolset & strict `--gui-only` lock
Owner: maintainers · Status: done · Was: GUI-agent Phase A ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Add `crates/ui-egui/src/inspect.rs` (`UiTreeSnapshot` / `inspect_ui_tree` / `ui.inspect`) exposing localized menus, active modal/wizard `tabs[]` (`UiTabSnapshot`) and recursive `widgets[]` (`UiWidgetNode`) with logical/physical bounds, values, and `enabled` + `disabled_reason` predicates, visible grid viewport cells (`excluded`, selection, link provenance), and hit-testable canvas scene targets, with `UI_COMMANDS` (`32` entries) synced to `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (Waves 15–16).
- [x] Expose the 8-tool human-equivalent `gui` MCP toolset (`GUI_TOOL_NAMES` in `crates/mcp/src/gui_tools.rs`: `ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`, `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`) operating identically across in-process `UiApp` sessions (`GuiSessionHost` / `InProcessUiGuiHost`) and live `--control` TCP sessions (`crates/mcp/tests/gui_only_surface.rs` `6 passed`, `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs` `3 passed`, Waves 15–17).
- [x] Implement `--gui-only` enforcement (`GuiOnlyPolicy::Strict` in `crates/engine/src/lib.rs`, `ErrorCode::GuiOnlyViolation` / `"gui_only_violation"` in `crates/doc/src/result.rs`, `GUI_ONLY_SERVER_INSTRUCTIONS` in `crates/mcp/src/protocol.rs`) on `lumencraft-cli mcp --gui-only` and `lumencraft --control <port> --gui-only` (`apps/lumencraft/tests/gui_only_wiring.rs` `2 passed`), refusing direct `L5` engine bypass commands, `doc_inspect`, `resources/read`, and `ui.screenshot` disk writes while allowing internal `UiApp` widget handlers (`Caller::Ui`) and read-only UI inspection calls to commit cleanly (Waves 15–17).

### R8.7 Interactive Power & Sample Size wizard, dynamic modal predicates & chained GUI surfaces
Owner: maintainers · Status: done · Was: GUI-agent Phase B ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Implement the standalone 4-stage **Power & Sample Size** wizard (`crates/ui-egui/src/power_wizard.rs` `PowerWizardState` / `PowerWizardStage::{ExperimentDesign, EffectSize, SignificanceAndSolve, Results}` & `crates/stats/src/power.rs` across all `8` `PowerTestFamily` variants — two-sample unpaired *t*, paired *t* with within-pair correlation $r$, one-sample *t*, one-way ANOVA, two proportions, correlation, $\chi^2$, and simple linear regression — solving for Sample Size, Power, or Detectable Effect, with interactive `power.curve_hover` curves, sensitivity tables, live right-sidebar parameter recalculation, and linked XY+Info+Graph+Result sheet creation; `crates/stats/tests/power_oracle.rs` `8 passed`, Waves 15–16).
- [x] Implement all 10 declarative conditional enablement and graying predicates `R1`–`R10` (`crates/ui-egui/src/modal_predicates.rs`: ANOVA pairing -> `Repeated Measures` tab & sphericity/REML `R1`; Gaussian toggle -> parametric vs. `Dunn's` rank post-hoc `R2`; control-column contrast -> `Dunnett` `R3`; two-group mode -> residual/rank/Welch toggles `R4`; contingency $2 \times 2$ clinical effect sizes vs. $R \times 2$ Cochran–Armitage trend `R5` & parent-coupled CI selectors `R6`; correlation matrix heatmap gate `R7`; single-subcolumn aggregation gate `R8`; custom *p*-value digit spinner `R9`; and leakage-safe `PCA` -> `PCR` dependent-variable exclusion `R10`, Waves 15–16).
- [x] Wire multi-row grid selection -> right-click `Format Selected Points` (`ActiveDialog::FormatSelectedPoints`, `PointStyleOverride`, `graph.formatSelectedPoints`, `ui.formatPoints.open` for symbol color, symbol shape, and row-label hit tagging), `GroupedArrangement::Superimposed`, `Cmd+T` / `EditAnalysisParameters` `FactorialAnovaSpec::multiplicity` UI editing (generating post-hoc comparison tabs in `run_factorial_anova` remains deferred to [`R9.12`](#r912-categorical--continuous-table-analysis-completions-qol-12)), and direct toolbar `Analyze` chaining from tabular intermediate `ResultSheet` tabs (`sheet.extractResultTab` / direct `ResultSheet` analysis for `Transform`, `Transpose`, `Extract and Rearrange`, `Outlier Cleaned Data`, `PCA Scores`; verified in `gui_only_workflows` `4 passed`, Waves 15–16).

### R8.8 Clean-room 28-workflow `gui_only` acceptance suite
Owner: maintainers · Status: done · Was: GUI-agent Phase C ([`gui-workflows.md`](gui-workflows.md)) · Est. note: 0 h remaining

- [x] Build and pass the 4-shard deterministic `gui_only` acceptance suite (`crates/ui-egui/tests/gui_wf_foundations_and_data.rs` [`6`], `gui_wf_group_comparisons_and_power.rs` [`9`], `gui_wf_regression_contingency_multivariate.rs` [`8`], `gui_wf_statistical_graphs.rs` [`5`] = `28` workflows `wf01`–`wf28`, alongside `gui_only_workflows.rs` [`4`], `crates/mcp/tests/gui_only_surface.rs` [`6`], `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs` [`3`], and `apps/lumencraft/tests/gui_only_wiring.rs` [`2`]) driving original synthetic `lumencraft-testkit` datasets strictly through `ui.inspect` + GUI input primitives + `ui.screenshot` PNG verification under `GuiOnlyPolicy::Strict`, asserting live upstream cell-edit and `Cmd/Ctrl+E` exclusion DAG recomputation (**Core workflows** GUI parity gate, Wave 17).

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
Owner: maintainers · Status: todo (revised scope) · Was: QOL-7, M8+ · Est. note: re-estimate

- Existing implementation: `AssumptionDiagnosticBanner` and `ui.result.applyRemediation` create sibling analyses. This records prior behavior, not completion of the revised scope.
- [ ] Align banners, surfaced checklist wording, and acceptance tests with diagnostics-only presentation: show diagnostic values, limitations, and links to details; no test/transform/exclusion recommendations, ranked alternatives, remediation buttons, or banner-triggered analysis creation. Preserve numerical outputs and persisted/replay contracts.

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
Owner: open · Status: todo · Was: QOL-12, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md))

- [ ] Expose `cohens_kappa` (`Unweighted`, `Linear`, `Quadratic`) and `fleiss_kappa` on `ContingencyMethod` (`crates/doc`, `crates/stats/src/categorical.rs`, `methods_report.rs`), invert Newcombe `2×2` risk-difference bounds into Altman `NNTB`/`NNTH` CIs, emit McNemar discordant-pair odds-ratio CIs, add custom expected proportions to `PartsOfWhole` goodness-of-fit $\chi^2$, and add exact one-sample Binomial ($p_0 \in (0, 1)$) & paired Sign tests.
- [ ] Add exact $\chi^2$ standard-deviation confidence intervals (`sd_ci`) to `DescriptiveSummary` (`crates/stats/src/column.rs`), support pre-summarized `Mean + SD/SEM + N` inputs directly in `run_two_group_analysis` (`crates/stats/src/two_group.rs`), and verify `run_factorial_anova` main/simple-effect multiplicity tabs (`crates/stats/src/anova.rs`, shared with `R8.7`).

### R9.13 Instant statistical, distribution & Bayesian calculators (`QOL-13`)
Owner: open · Status: deferred · Was: QOL-13, M8+ · Scheduling: no milestone assigned ([reference specification](calculators-and-quick-stats.md))

- Deferred reference scope: stateless statistical/distribution/Bayesian `calc.*` commands, scalar formula builtins, and a Calculators drawer. No subset is queued for implementation; detailed proposals remain in the reference specification.

### R9.14 Experimental subject randomization & random subset sampling (`QOL-14`)
Owner: open · Status: todo · Was: QOL-14, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md))

- [ ] Add `crates/transform/src/randomize.rs` over `DeterministicRng` implementing `table.randomizeGroups` (seeded complete balanced & permuted-block subject-to-group allocation writing seed/block provenance to `InfoSheet`), `table.randomSampleRows` / `TransformAnalysisSpec::RandomSample` (uniform & stratified $k$-of-$N$ row sampling without replacement), and `table.simulateDistributions` (Uniform, Gaussian, Log-normal, Binomial, Poisson & Exponential column generation).

### R9.15 Bench molarity, stock dilution & radiochemical decay calculators (`QOL-15`)
Owner: open · Status: deferred · Was: QOL-15, M8+ · Scheduling: no milestone assigned ([reference specification](calculators-and-quick-stats.md))

- Deferred reference scope: molarity, stock/serial dilution, moles/grams, and radiochemical calculators, including export-to-sheet conveniences. No subset is queued for implementation; detailed proposals remain in the reference specification.

### R9.16 Live best-fit $\TeX$ equation annotations (equations only, `QOL-16`)
Owner: open · Status: todo · Was: QOL-16, M8+ ([`calculators-and-quick-stats.md`](calculators-and-quick-stats.md))

- [ ] Implement one-click symbolic or live-substituted $\TeX$-subset best-fit model equation annotations across RGBA, SVG, and `%PDF-1.7`, updating coefficients when the linked fit recalculates. Compact Letter Display, significance grouping, pairwise-display modes, and additional fit-statistic callouts are excluded. Re-estimate the narrowed scope.

### R9.17 Extended multivariable, clustering, GLM & statistical-graphing `gui_only` acceptance suite (`QOL-17`)
Owner: open · Status: todo · Was: QOL-17, M8+ ([`gui-workflows.md`](gui-workflows.md#5-extended-21-workflow-multivariable-clustering-glm--statistical-graphing-gui_only-matrix-r917--qol-17-planned))

- [ ] Add Shards 5–7 (`crates/ui-egui/tests/gui_wf_{multivariable_and_transforms,clustering_glm_and_pharma,advanced_statistical_graphs}.rs`, `WF-29`–`WF-49`) under `GuiOnlyPolicy::Strict` covering conjunctive `Select and Transform` filtering, `MultipleVariables` formula-bar `Calculated Variables` (syntax/circularity guards & $2^{-\Delta\Delta C_t}$ qPCR), multi-schema `Extract and Rearrange`, direct `MultipleVariables` role-bound Welch *t* / Multifactor ANOVA / Kaplan–Meier / 4PL dose–response, full linkage/metric Hierarchical & K-Means clustering diagnostics, Cox PH & Poisson count GLMs, confidence ellipses/convex hulls, cross-tabulated `MultipleVariables` heatmaps, synchronized survival `Number at Risk` re-binning, bubble symbol sizing, and `R9.16` equations-only live $\TeX$ annotations (keeping Compact Letter Display and `QOL-7` remediation actions excluded).

---

## Order and dependencies

| Step | Items | Depends on | Target gate |
|---|---|---|---|
| **Step 1 (8A)** | `R0.1`, `R0.2` (done); foundations for `R0.3`, `R0.5`, `R0.6`, `R0.7`, `R0.10`, `R0.14` | — | Scaffold |
| **Step 2 (8B)** | `R0.3` (secrets gate), `R0.4` (pending/status), `R0.5` (licenses gate — done), `R0.6` (Zero Telemetry done; remote CI `actionlint` open) | `R0.2` for `R0.4` | Scaffold |
| **Step 3** | `R0.10` (`lumencraft-platform` wiring & source checks), `R0.11` (versioned files) | `R0.6` | Scaffold |
| **Step 4** | `R0.8` (`CommandSpec` registry, area by area), then `R0.9` (structured errors & remediation) | — (runs beside Step 3) | Engine contract |
| **Step 5** | `R0.7` (generated `commands.md` & `schemas`), `R0.12` (format registry) | `R0.8` (commands), `R0.11` (schemas) | Docs & seams |
| **Step 6** | `R0.13` (MCP/CLI/UI alignment) | `R0.8`, `R0.9` | Surfaces |
| **Step 7** | `R8.3` (`DesktopHostRuntime` done; external `eframe`/`wgpu` crate link open), `R8.4` (`host_adapters.rs` & CSP/COOP/COEP done; `wasm32`/Trunk bundle open) | `R8.1` (done) | **Alpha** & **Core** |
| **Step 7B** | `R8.6` (`ui.inspect`, `gui` MCP toolset & `--gui-only` lock — **done**), `R8.7` (Power & Sample Size wizard, 10 modal predicates & chained GUI surfaces — **done**), then `R8.8` (4-shard 28-workflow `gui_only` acceptance suite — **done**) | `R8.1` (done), `R0.13` (done) | **Core workflows** (`R8.6`–`R8.8` done) |
| **Step 8** | `R5.4` (native binary OS clipboard, `ActiveDialog::Print` & WYSIWYG layout drag-snap — **done**) | `R8.1` | **Beta** (done) |
| **Step 9** | `R8.5` (cross-OS VM QA & non-sandboxed `.dmg`/`.zip` `verify.sh`), `R0.14` (`README.md` screenshots done; non-sandboxed dry-run & `v0.1.0-rc1` CI tag/matrix open) | `R0.5` (done), `R8.3`, `R8.4`, `R8.8` (done) | **1.0** |
| **Step 10 (M8+)** | `R9.12` (table-analysis completions), `R9.14` (randomization & sampling), `R9.16` (equation callouts only), `R9.17` (extended 21-workflow `gui_only` suite `WF-29`–`WF-49`); `R9.10`, `R9.11`, `R9.13`, and `R9.15` excluded from active sequencing | `R0.8` (done), `R3.1`–`R3.3` (done), `R4.4` (done), `R5.1` (done), `R8.8` (done; `R9.12` & `R9.16` for `WF-33`/`WF-49`) | Post-`1.0` QoL; no new dates |

**Active `**decision**` markers:**
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
| `M5` | Headless CLI, `--control`, MCP, pure-Rust `UiApp`/`WebWorkspaceHost` + remaining **Alpha** live OS/WASM GPU/bundler linkage | [`R7.1`](#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server), [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost), [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop)–[`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | Headless + `UiApp` + `DesktopHostRuntime`/web host adapters done; external `eframe`/`wgpu` linkage (`R8.3`) & `wasm32`/Trunk build (`R8.4`) open |
| `M6` | Survival, Cox PH, Barnard/Koopman, ROC, MV/Calculated Vars, custom DSL/ODE, `.prism`/`.pzfx`, Layouts & `Beta` clipboard/print/drag | [`R2.4`](#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget), [`R3.3`](#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice), [`R4.3`](#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm), [`R5.3`](#r53-headless-page-layouts-style-cloning-spool-pdf--subprocessmacos-clipboard)–[`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap), [`R6.1`](#r61--calculated-variables-role-locking--level-ordering)–[`R6.2`](#r62-multifactor-anova-mv-groupingsplits--five-channel-mv-plots), [`R7.2`](#r72-performance-budgets-1m-cell-virtualization--propertyfuzz-suites) | Done (Waves 2–13; `R5.4` macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux vector clipboard adapters, `ActiveDialog::Print`, & WYSIWYG layout drag-snap landed) |
| `M7` | `16`-locale i18n/RTL, macOS `pbcopy`/`pbpaste`/`NSPasteboard` & codesign, cross-OS hardware/CJK desktop QA & signed installers | [`R8.2`](#r82-16-locale-i18n-rtl-mirroring--desktop-cjkrtl-fontpasteboard-checks), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) | Core/macOS/PDF/`cjk_rtl_hidpi_desktop_qa`/arm64 `.app` done; non-sandboxed `.dmg`/`.zip` & Windows/Linux/FreeBSD VMs open (`R8.5`) |
| `M8+` | Active QoL: diagnostics-only banners, table-analysis completions, randomization, and equation callouts; house-principles alignment (`R0`) | [`R0`](#r0-align-with-house-principles), [`R9`](#r9-post-parity-scientific-quality-of-life-suite) | `R9.7` alignment pending; `R9.12`, `R9.14`, `R9.16` planned; `R9.10`, `R9.11`, `R9.13`, `R9.15` deferred without milestones |

### Release Gates

| Release Gate | Required Roadmap Items | Status & Remaining Work |
|---|---|---|
| **Alpha** (first visual desktop & WASM release) | [`R1`](#r1-workspace-numerical-foundations--nist-strd-oracle)–[`R7`](#r7-headless-automation-cli---control-mcp--performance-budgets) headless parity + [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost) + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) + [`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | `DesktopHostRuntime` & browser host adapters landed; external `eframe`/`wgpu` crate link (`R8.3`) & `wasm32`/Trunk build (`R8.4`) open |
| **Core workflows** | **Alpha** + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) native pointer/text/zoom binding + [`R8.6`](#r86-semantic-ui-inspection-gui-mcp-toolset--strict---gui-only-lock)–[`R8.8`](#r88-clean-room-28-workflow-gui_only-acceptance-suite) `gui_only` workflow parity | `R8.6`–`R8.8` **done** (`ui.inspect`, 8-tool `gui` MCP mode, `--gui-only`, 4-stage/8-family Power wizard, `R1`–`R10` modal predicates, `FormatSelectedPoints`, `ResultSheet` `Analyze` chaining, & 28-workflow `gui_wf_*` suite verified) + `DesktopHostRuntime` DPI/IME/pinch/drop tests landed; external GPU linkage/event pump (`R8.3`) and **Alpha** (`R8.4`) open |
| **Beta** (OS vector clipboard/print/layout drag) | **Core workflows** + [`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) | `R5.4` **done** (live macOS `NSPasteboard` `%PDF-1.7` + Win32/Linux adapters, `ActiveDialog::Print`, WYSIWYG layout drag-snap) |
| **1.0** (full signed installers & target-OS QA) | **Beta** + [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) + [`R0`](#r0-align-with-house-principles) release gates | `R0.5`, `README.md` screenshots & local `arm64` `.app` `plutil`/`codesign` done; non-sandboxed `.dmg`/`.zip` `verify.sh`, Windows/Linux/FreeBSD VMs, remote CI `actionlint` (`R0.6`), & `v0.1.0-rc1` tag/matrix (`R0.14`) open |

---

## Parked

Items explicitly deferred until needed or post-`1.0`:

- **Translated READMEs:** localized `README.<locale>.md` translations are product polish deferred beyond initial `1.0` release scaffold (`R0.14`).
- **Mutex wrapper & locks check:** `clippy.toml` already disallows raw `std::sync::Mutex`; a deadlock-detecting lock wrapper and `xtask locks` check stay parked while the workspace remains single-owner / message-passing.
- **Plugin SDK / dynamic plugins:** deferred (`**decision**`; closed domain seams use typed traits and registries inside the workspace).
