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
| **R0** | Align with house principles & typed engine scaffold | Phases A–E | in progress | `R0.1`–`R0.4`, `R0.7`–`R0.13` done (Waves 8A–11); `R0.5` (`xtask licenses` & `release.yml` done; CI gate wiring deferred), `R0.6` (GH Actions run open), `R0.14` (CI release dry-run & `R8.3` screenshots open) in progress |
| **R1** | Workspace scaffold, numerical foundations & NIST StRD oracle | `M0`, `M1` (partial), `G1` | done | `58` NIST StRD datasets + `D1` R `4.5.2` oracle matrices verified (`0 h`) |
| **R2** | Typed data tables, reactive DAG, persistence & `.prism`/`.pzfx` import | `M1`, `M2`, `G2`, `G10` | done | Waves 1–6 incl. `.lumencraft.d/`, `G2` & `G10` fixes verified (`0 h`) |
| **R3** | Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate | `M2`, `M6`, `G3`, `G8` | done | Waves 2–5 incl. `BarnardExact`, `PassingBablok`, MICE, `G8` Koopman fix (`0 h`) |
| **R4** | Nonlinear regression solver, `117` equations & custom DSL/ODE | `M3`, `M6`, `G4`, `G9` | done | Waves 2–5 incl. `47` pharma models, `MAX_AST_NODES`, iterative `Expr` (`0 h`) |
| **R5** | Publication graphing, vector SVG/PDF 1.7/TIFF/EMF & Page Layouts | `M4`, `M6`, `S2`, `G5`, `G11` | in progress | `G5` Poppler+Ghostscript PDF QA done; `R5.4` native binary clipboard/print dialogs & live layout drag-snap open (`2–3 h`, **Beta**) |
| **R6** | Multiple Variables workflows, Calculated Variables & Multifactor ANOVA | `M6`, `G7` | done | Waves 2–4 end-to-end verified (`0 h`) |
| **R7** | Automation, headless CLI, `--control`, MCP & performance budgets | `M5` (headless), `M6`, `Slice 0`, `G12`, `G14` | done | `90` engine commands, `96`-curve 4PL `< 50 ms`, `G12` read-only skip & bounded undo (`0 h`) |
| **R8** | Live desktop & WASM UI shell, localization QA & `1.0` hardening | `M5` (UI), `M7`, `S1`, `G6`, `G13` | in progress | `UiApp`/HiDPI/RTL/`WebWorkspaceHost`/macOS bundle signing done; `R8.3`–`R8.4` (`eframe`/`wgpu`/`wasm32`, `20–33 h` **Alpha/Core**) & `R8.5` (`14–22 h` **1.0**) open |
| **R9** | Post-parity scientific quality-of-life suite | `M8+`, `QOL-1`–`QOL-11` | done (core/UI/web) | Waves 5–7 wired all `11` QoL workflows across doc, domain, engine, `UiApp`, CLI & `WebWorkspaceHost` (browser Trunk deploy tracked in `R8.4`) |

**Gate summary:** Headless **Alpha**, **Core**, and **Beta** scientific/engine/CLI/MCP/`UiApp` workflows pass today; reaching live GUI **Alpha** (`R8.3` Alpha host `10–17 h` + `R8.4` WASM `5–8 h` = `15–25 h`), **Core** (`R8.3` native pointer/inline GUI polish, `+5–8 h` -> `20–33 h`), **Beta** (`R5.4` native binary clipboard/print/layout drag, `+2–3 h` -> `22–36 h`), and **1.0** (`R8.5` cross-OS CJK/RTL desktop QA `2–4 h` + installer/hardware hardening `12–18 h` = `+14–22 h` -> **`36–58 h`**) leaves an estimated **`36–58 h`** single-agent parity/release scope alongside remaining `R0.5`/`R0.6`/`R0.14` CI and release gates.

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
Owner: open · Status: in progress · Was: Phase B3

- [x] Add `deny.toml`, `about.toml`, `about.hbs`, and `docs/licences.md` aligned with the allowlist and add the `licenses` job generating `THIRD-PARTY-LICENSES.html` in `.github/workflows/release.yml` (Wave 8A).
- [ ] Implement `cargo xtask licenses` (`cargo deny check licenses`, `--html` fallback) and wire into CI gate workflow (`xtask/src/licenses.rs` & `release.yml` landed in Wave 8B; wiring `cargo-deny` + `cargo xtask licenses` into `.github/workflows/ci.yml` / `cmd_ci` deferred).

### R0.6 Repository hygiene & CI cleanup
Owner: open · Status: in progress · Was: Phase B4

- [x] Add `.gitattributes`, workspace `publish = false` (`publish.workspace = true` across library crates), `unsafe_code = "forbid"`, `[profile.web-release]`, `clippy.toml` (`allow-indexing-slicing-in-tests`, `disallowed-types` for `std::sync::Mutex`), `.github/actionlint.yaml`, and remove dead `.github/workflows/nix.yml` (Wave 8A).
- [ ] Verify CI workflows (`actionlint` & `cargo xtask ci`) pass cleanly on GitHub Actions (parked pending remote CI runner execution); **decision** (maintainers): keep SonarCloud external telemetry/service **out** unless maintainers approve adding a third-party service.

### R0.7 Generated command & schema docs
Owner: open · Status: done · Was: Phase B5

- [x] Author hand-written `docs/command-ids.md` and `docs/file-formats.md` (Wave 8A).
- [x] Implement `cargo xtask commands [--check]` generating `docs/commands.md` from `R0.8`'s typed registry and wire `--check` into `cargo xtask ci` (Wave 11).
- [x] Implement `cargo xtask schemas [--check]` generating `docs/schemas/*.schema.json` (archive manifest, document, batch v1, preferences, journal record) after `R0.11` and wire `--check` into `cargo xtask ci` (Wave 11).

### R0.8 Typed `CommandSpec` registry
Owner: open · Status: done · Was: Phase C1

- [x] Replace `CommandDescriptor`, `catalog::COMMANDS` (`90` entries), and string `match` dispatch in `cmd_data.rs` / `cmd_sci.rs` with `crates/engine/src/cmd/mod.rs` (`CommandSpec`, `CommandGroup`, `Effects`, `Ctx`, `Registry`), migrating area-by-area with `serde_path_to_error` typed params and preserving WAL / `lumencraft.batch.v1` compatibility (Waves 9–11).
- [x] Convert `UI_COMMANDS` (`29` entries in `crates/ui-egui/src/menus/ui_commands.rs`) to a typed `UiCommandSpec` struct and regenerate `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (Wave 9).

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

- [x] Expand `docs/releasing.md`, add `packaging/macos/verify.sh`, `packaging/linux/apprun-test.sh`, `packaging/windows/check-icons.ps1`, pass `--cargo-profile web-release` in `packaging/web/package.sh`, generate `THIRD-PARTY-LICENSES.html` in `release.yml`, and modernize `README.md` with badges, section nav, Downloads, `platform/`, and one-line Status (Wave 8A).
- [ ] Verify the signed release workflow dry-run end-to-end across CI artifacts and add `docs/images/` workspace screenshots to `README.md` once `R8.3` live desktop windowing lands.

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
Owner: open · Status: todo · Was: G11 (remaining `2–3 h`, **Beta**) · Est. note: 2–3 h

- [ ] Implement native binary OS vector pasteboard APIs (`NSPasteboard` PDF on macOS, Win32 EMF on Windows, browser Clipboard API on WASM).
- [ ] Wire interactive OS print-dialog submission and live WYSIWYG layout canvas drag-snap inside `R8.3` host windowing.

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

- [x] Land `Slice 0` vertical gate, `lumencraft-cli` (`18 passed`, incl. `.lctemplate`, `perf --json`, `.lumencraft.d`, `.emf`), `lumencraft --control` (`9 passed`), `lumencraft-mcp` (`25 passed`, `29` synced `UI_COMMANDS`), and `90` registered engine commands (Waves 2–7).

### R7.2 Performance budgets, `1M`-cell virtualization & property/fuzz suites
Owner: maintainers · Status: done · Was: G12, M6 · Est. note: 0 h remaining

- [x] Verify `< 50 ms` `96`-curve 4PL batch solve (`--release`), `< 250 ms` `100k` Calculated Variables, `1M`-cell `S1` grid (`< 1 ms` median frame), 7 property/fuzz harnesses, and Wave 6 `G12` read-only clone-skip & bounded undo (`MAX_UNDO_ENTRIES = 64`, `MAX_UNDO_CELL_BUDGET = 25_000_000`).

---

## R8 Desktop & WebAssembly live shell, localization & `1.0` hardening

### R8.1 Pure-Rust `UiApp` shell, software RGBA compositor, HiDPI/RTL input & `WebWorkspaceHost`
Owner: maintainers · Status: done · Was: G6 (shell), S1, M5 (partial) · Est. note: 0 h remaining

- [x] Land `lumencraft-tools` (`21 passed`, **S1 GO**), `lumencraft-ui-egui` (`69 passed` default & `--no-default-features`, `UiApp`, RGBA/PNG compositor, `29` `UI_COMMANDS`, `NavigatorDragState`, `PortfolioGalleryState`, `F6` pane cycle, rulers, `ShortcutRegistry`, Wave 7 `ViewportMetrics` `1.0x`–`3.0x` HiDPI, `WorkspaceLayoutMap`, `UiInputEvent`, `InlineEditorState`, `VirtualWorkspaceFolder`), and `apps/lumencraft-web` `WebWorkspaceHost` (`5 passed`, zero JS, `#![forbid(unsafe_code)]`).

### R8.2 `16`-locale i18n, RTL mirroring & desktop CJK/RTL font/pasteboard checks
Owner: maintainers · Status: done · Was: G13 (core & macOS QA), M7 (partial) · Est. note: 0 h remaining

- [x] Land `16` locales × `167` keys = `2,672` translations (`0` missing in `i18n-report`), RTL UI mirroring (`ar`/`he`), locale decimal/CSV presets, Poppler+Ghostscript CJK/RTL `%PDF-1.7` QA, and live macOS `pbcopy`/`pbpaste` `de-DE` comma + `=A1*2` formula roundtrip (`crates/ui-egui/tests/macos_clipboard_live.rs`). Remaining live OS desktop CJK/RTL window verification joins `R8.5`.

### R8.3 Live OS `eframe`/`egui`/`wgpu` desktop windowing & interactive event loop
Owner: open · Status: todo · Was: G6 (desktop **Alpha** `10–17 h` + **Core** `5–8 h`), M5 · Est. note: 15–25 h (incl. Core pointer polish)

- [ ] Integrate `eframe`/`egui`/`wgpu` live OS window loop over `UiApp` and `UiInputEvent` (`apps/lumencraft`) across macOS, Linux (Wayland/X11), Windows, and FreeBSD (**Alpha**).
- [ ] Bind native OS pointer drag-reorder, split-divider drag, inline grid/`InfoSheet`/title text widgets, trackpad pinch-zoom gestures, and `--in-window-menus`/`--new-instance` flags into the live host loop (**Core**).

### R8.4 Live `wasm32-unknown-unknown` Trunk WebGPU/WebGL2 browser hosting
Owner: open · Status: todo · Was: G6 (WASM **Alpha** `5–8 h`), M5 · Est. note: 5–8 h

- [ ] Install/verify `wasm32-unknown-unknown` sysroot and `trunk` in CI/release environments (`cargo xtask wasm` passing) and wire browser canvas + `localStorage` / Web Locks / File System Access host adapters into `apps/lumencraft-web` (`WebWorkspaceHost`) (**Alpha**).

### R8.5 Cross-OS hardware, CJK/RTL desktop QA & installer/signing hardening
Owner: open · Status: in progress · Was: G13 (live desktop QA `2–4 h`), M7 (`12–18 h`, **1.0**) · Est. note: 14–22 h

- [x] Implement macOS `Info.plist` `plutil -lint` validation and ad-hoc `codesign --force --deep --sign -` + `--verify --strict --deep` in `xtask/src/bundle.rs` (Wave 7).
- [ ] Run live OS desktop CJK/RTL system font & HiDPI/trackpad QA on macOS, Windows, Linux, and FreeBSD hosts (`R8.3` prerequisite).
- [ ] Verify signed/notarized release artifacts (`packaging/macos/verify.sh`, Linux AppImage/deb/rpm/Flatpak, Windows MSI/portable, FreeBSD tarball) on target OS runners.

---

## R9 Post-parity scientific quality-of-life suite

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

### R9.7 Inline assumption diagnostic banners & non-blocking remediation (`QOL-7`)
Owner: maintainers · Status: done · Was: QOL-7, M8+ · Est. note: 0 h remaining

- [x] Implemented `AssumptionDiagnosticBanner` & `RemediationKind` (`ui.result.applyRemediation`) creating non-destructive sibling analyses without altering the original user test (Wave 7).

### R9.8 Journal figure dimension & typography lock presets (`QOL-8`)
Owner: maintainers · Status: done · Was: QOL-8, M8+ · Est. note: 0 h remaining

- [x] Implemented `FigureCanvasPreset`, `CanvasResizeMode` (`KeepTypographyPt` vs. `ProportionalScale`), `preflight_check_scene`, `graph.applyCanvasPreset`/`layout.applyCanvasPreset`/`graph.preflightCheck`, and `ui.canvas.applyPreset`/`ui.canvas.preflight` (Waves 5–7).

### R9.9 Live color-vision-deficiency & grayscale canvas preview (`QOL-9`)
Owner: maintainers · Status: done · Was: QOL-9, M8+ · Est. note: 0 h remaining

- [x] Implemented `CanvasPreviewFilter`, `apply_canvas_preview_filter` (`crates/render`), and `ui.previewFilter.set` non-mutating viewport filter in `crates/ui-egui` & `WebWorkspaceHost` (Waves 5 & 7).

### R9.10 Zero-install client-side WASM viewer & virtual workspace folder (`QOL-10`)
Owner: maintainers · Status: done · Was: QOL-10, M8+ · Est. note: 0 h remaining

- [x] Implemented `VirtualWorkspaceFolder` (`crates/ui-egui`), `presentation_mode` (`ui.presentation.toggle`, `Cmd+Shift+P`), and `WebWorkspaceHost` (`apps/lumencraft-web`) opening `.lumencraft`/`.lctemplate`/`.pzfx`/`.prism`/CSV/TSV/`.lumencraft.d` 100% locally in memory; browser Trunk build/hosting tracked in `R8.4` (Wave 7).

### R9.11 Unpacked Git-friendly `.lumencraft.d/` directory & checkpoints (`QOL-11`)
Owner: maintainers · Status: done · Was: QOL-11, M8+ · Est. note: 0 h remaining

- [x] Implemented `save_project_unpacked_dir`/`open_project_unpacked_dir` (`crates/format`), `NamedCheckpoint` (`project.{create,list,restore,delete}Checkpoint`), and `.lumencraft.d` auto-detection in `engine`, `lumencraft-cli`, `apps/lumencraft`, and `WebWorkspaceHost` (Waves 5–7).

---

## Order and dependencies

| Step | Items | Depends on | Target gate |
|---|---|---|---|
| **Step 1 (8A)** | `R0.1`, `R0.2` (done); foundations for `R0.3`, `R0.5`, `R0.6`, `R0.7`, `R0.10`, `R0.14` | — | Scaffold |
| **Step 2 (8B)** | `R0.3` (secrets gate), `R0.4` (pending/status), `R0.5` (licenses gate), `R0.6` (CI verification) | `R0.2` for `R0.4` | Scaffold |
| **Step 3** | `R0.10` (`lumencraft-platform` wiring & source checks), `R0.11` (versioned files) | `R0.6` | Scaffold |
| **Step 4** | `R0.8` (`CommandSpec` registry, area by area), then `R0.9` (structured errors & remediation) | — (runs beside Step 3) | Engine contract |
| **Step 5** | `R0.7` (generated `commands.md` & `schemas`), `R0.12` (format registry) | `R0.8` (commands), `R0.11` (schemas) | Docs & seams |
| **Step 6** | `R0.13` (MCP/CLI/UI alignment) | `R0.8`, `R0.9` | Surfaces |
| **Step 7** | `R8.3` (live `eframe`/`wgpu` windowing), `R8.4` (`wasm32`/Trunk browser host) | `R8.1` (done) | **Alpha** & **Core** |
| **Step 8** | `R5.4` (native binary OS clipboard, print dialogs & layout drag-snap) | `R8.3` | **Beta** |
| **Step 9** | `R8.5` (cross-OS CJK/RTL QA & installer hardening), `R0.14` (release verification & screenshots) | `R0.5`, `R8.3`, `R8.4` | **1.0** |

**Active `**decision**` markers:**
- **SonarCloud (`R0.6`):** **decision** — adding SonarCloud requires adopting an external service and token management; keep disabled unless maintainers explicitly approve.
- **Plugins (`Parked`):** **decision** — dynamic plugin loading / third-party plugin SDKs stay out of `1.0` scope so the `L0`–`L7` workspace, `#![forbid(unsafe_code)]`, and clean-room guarantees stay self-contained.

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
| `M5` | Headless CLI, `--control`, MCP, pure-Rust `UiApp`/`WebWorkspaceHost` + remaining **Alpha** live OS/WASM hosts | [`R7.1`](#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server), [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost), [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop)–[`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | Headless + `UiApp` done; `20–33 h` remaining (`R8.3`–`R8.4`) |
| `M6` | Survival, Cox PH, Barnard/Koopman, ROC, MV/Calculated Vars, custom DSL/ODE, `.prism`/`.pzfx`, Layouts & `Beta` dialogs | [`R2.4`](#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget), [`R3.3`](#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice), [`R4.3`](#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm), [`R5.3`](#r53-headless-page-layouts-style-cloning-spool-pdf--subprocessmacos-clipboard)–[`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap), [`R6.1`](#r61--calculated-variables-role-locking--level-ordering)–[`R6.2`](#r62-multifactor-anova-mv-groupingsplits--five-channel-mv-plots), [`R7.2`](#r72-performance-budgets-1m-cell-virtualization--propertyfuzz-suites) | Core landed; `2–3 h` Beta OS vector clipboard/print/layout drag remaining (`R5.4`) |
| `M7` | `16`-locale i18n/RTL, macOS `pbcopy`/`pbpaste` & codesign, cross-OS hardware/CJK desktop QA & signed installers | [`R8.2`](#r82-16-locale-i18n-rtl-mirroring--desktop-cjkrtl-fontpasteboard-checks), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) | Core/macOS/PDF QA done; `14–22 h` **1.0** target-OS QA remaining (`R8.5`) |
| `M8+` | `QOL-1`–`QOL-11` post-parity workflows + house-principles architecture alignment (`R0`) | [`R0`](#r0-align-with-house-principles), [`R9`](#r9-post-parity-scientific-quality-of-life-suite) | `QOL-1`–`QOL-11` done in `R9`; `R0` Phases A–E active |

### Release Gates

| Release Gate | Required Roadmap Items | Status & Remaining Work |
|---|---|---|
| **Alpha** (first visual desktop & WASM release) | [`R1`](#r1-workspace-numerical-foundations--nist-strd-oracle)–[`R7`](#r7-headless-automation-cli---control-mcp--performance-budgets) headless parity + [`R8.1`](#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost) + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) + [`R8.4`](#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting) | `15–25 h` remaining (`R8.3` Alpha host `10–17 h` + `R8.4` WASM `5–8 h`) |
| **Core workflows** | **Alpha** + [`R8.3`](#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop) native pointer/text/zoom polish | `+5–8 h` (`20–33 h` cumulative) |
| **Beta** (OS vector clipboard/print/layout drag) | **Core workflows** + [`R5.4`](#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) | `+2–3 h` (`22–36 h` cumulative) |
| **1.0** (full signed installers & target-OS QA) | **Beta** + [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) + [`R0`](#r0-align-with-house-principles) release gates | `+14–22 h` (`36–58 h` cumulative parity/release remaining) |

---

## Parked

Items explicitly deferred until needed or post-`1.0`:

- **Translated READMEs:** localized `README.<locale>.md` translations are product polish deferred beyond initial `1.0` release scaffold (`R0.14`).
- **Mutex wrapper & locks check:** `clippy.toml` already disallows raw `std::sync::Mutex`; a deadlock-detecting lock wrapper and `xtask locks` check stay parked while the workspace remains single-owner / message-passing.
- **Plugin SDK / dynamic plugins:** deferred (`**decision**`; closed domain seams use typed traits and registries inside the workspace).
