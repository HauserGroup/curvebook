# CurveBook Agent & Contributor Rules

This file is the binding engineering and governance specification for **CurveBook** (crate prefix `curvebook-*`, repository root `curvebook/`). Every contributor and AI coding agent must follow these rules.

For concrete extension checklists, common past mistakes, and subsystem details, also read:
- [docs/pitfalls.md](docs/pitfalls.md) — documented CurveBook mistakes and the rules that prevent them
- [docs/pitfalls-engineering.md](docs/pitfalls-engineering.md) — general Rust, tooling, persistence, and process rules adopted for CurveBook
- [docs/extending.md](docs/extending.md) — where and how to add commands, analyses, fit models, formats, cursors, locales, schema migrations, and assets
- [docs/command-ids.md](docs/command-ids.md) — canonical command ID scheme, area prefixes, and replay stability rules
- [docs/file-formats.md](docs/file-formats.md) — persisted formats, ownership, versioning policy, and migrations
- [docs/licences.md](docs/licences.md) — crate & asset licence allowlist, `deny.toml`, `about.toml`, and attribution files
- [docs/architecture.md](docs/architecture.md) and [docs/development.md](docs/development.md) — layer architecture, build prerequisites, and debug/test harnesses

---

## 1. Non-Negotiable Principles

### Clean-Room Development (`xtask cleanroom`, `xtask brands`)
1. **Independent implementation and original design.** Never copy, translate, decompile, disassemble, or inspect the binaries, memory, or bundled assets of proprietary commercial statistical, graphing, or curve-fitting applications. Permitted design sources are publicly observable file format behaviour (such as `.pzfx` and `.prism` files read/written by `crates/format`), public scientific specifications, peer-reviewed statistical literature, and the narrowly limited local JPG references defined below. Keep CurveBook's visual design original. The attributed GUI source reuse policy below governs the sole exception to independent implementation.
2. **No copyleft dependencies or translations.** Never depend on or translate code from `GPL`, `AGPL`, or `LGPL` projects—even permissive-looking ports of GPL codebases (e.g., R, GNU Scientific Library, JASP, PSPP, Veusz, LabPlot, SciDAVis).
3. **Neutral terminology, original brand identity, and explicit `.pzfx files` / `.prism files` naming.** Write code identifiers, UI labels, comments, examples, and documentation in CurveBook's own words. Exclude other suite brands, placeholder template package names, and proprietary competitor product names from the repository except for the source notices and policy identifiers explicitly permitted below (`cargo xtask brands` and `cargo xtask cleanroom` enforce this).
   - **`.prism files` and `.pzfx files` are specifically allowed over clean-room and branding principles:** the lowercase file extensions `.prism` and `.pzfx` (for example `.prism files`, `.pzfx files`, `.prism`, `*.prism`, `experiment.prism`) are explicitly permitted on every surface—including UI menus, dialogs, tooltips, documentation, tutorials, and page titles/descriptions—and **all user-facing documentation and GUI copy should prefer `".pzfx files"` and `".prism files"` naming where appropriate** rather than opaque euphemisms such as `"External Archive"` or `"XML Project"`. Do **not** avoid or rewrite `.prism files` out of clean-room or branding caution.
   - **Boundaries that still apply:** bare or capitalised competitor product names, capitalised extension spellings, vendor documentation prose or screenshots, and vendor-generated `.prism`/`.pzfx`/`.pzf` sample files remain strictly forbidden (`cargo xtask brands` and `cargo xtask cleanroom`); command and XML identifiers using that token stay strictly limited to file format parser/exporter boundaries. The local archive identifier below is permitted solely to define that policy's scope.

#### Attributed VectorCraft GUI source reuse

- Contributors may copy or adapt permissively licensed (`MIT OR Apache-2.0`) VectorCraft GUI helpers, widget geometry, layout proportions, spacing/typography tokens, and tests into CurveBook. Verify the licence of the actual source files and preserve required copyright, licence, and upstream notices. Record the exact upstream VectorCraft repository URL, commit revision, original file paths, and affected CurveBook files in [ATTRIBUTION.md](ATTRIBUTION.md), and retain required licence text in the shipped [NOTICE](NOTICE).
- The exact upstream VectorCraft GitHub repository URL is permitted solely inside [ATTRIBUTION.md](ATTRIBUTION.md) and [NOTICE](NOTICE) (enforced by `cargo xtask brands` and `cargo xtask cleanroom`); never place external suite org/crate names or doc-comment references in source code, UI strings, documentation, or this file. Keep CurveBook's own UI terminology, product identity, scientific colour palette, icons, and assets. This permission covers source code and tests only; proprietary-source restrictions, workspace-wide `unsafe_code = "forbid"`, registry-only dependency allowlist ([deny.toml](deny.toml)), zero telemetry, and asset/zero-bundled-font policies remain in force.

#### Limited local JPG reference policy

- **Scope:** Existing, lawfully obtained JPG/JPEG reference frames beneath the gitignored local `plan/prism/` directory may be inspected solely to understand high-level workflows, information hierarchy, and generic functional organization. This permission does not override access terms, licences, confidentiality obligations, or third-party rights, and does not extend to other archive contents or proprietary installations.
- **Independent creation:** Implement every feature independently and create original screen compositions, wording, icons, diagrams, and styling. Do not trace, extract, reuse, reproduce, or closely imitate distinctive screen arrangements, proportions, visual treatments, branding, tutorial examples, or other expressive elements from the frames. Generic functional organization is the limit of the layout reference permission.
- **Local handling and provenance:** Keep the frames and their provenance notes gitignored and out of source commits, fixtures, generated assets, packages, and distribution. Record source URLs or archive identifiers, observation dates, and the functional requirements inferred in an honest local provenance note. Tracked design records must explain CurveBook's own requirements and design decisions in neutral terms without embedding the frames or linking into the ignored archive.
- **Verification:** Do not infer numerical algorithms or reference answers from frames, or use them as visual matching targets, acceptance baselines, or evidence that a workflow works. Review for unintended copying may identify similarities to remove; it must not require visual similarity. Validate numerical behaviour through published scientific references and verified oracles, and validate usability through CurveBook's own requirements and tests. The existing branding, asset-provenance, licence, and human-path evidence gates remain in force.

### Statistical Rigor & Verified Oracles (`D1` Rule)
1. **Cite a published reference.** Every statistical test, probability distribution, numerical solver, transform, and curve-fitting routine in `crates/math`, `crates/stats`, `crates/fit`, and `crates/transform` must cite a peer-reviewed paper, standard textbook (e.g., Zar, Motulsky & Christopoulos, Press et al.), or NIST StRD specification in its doc comments.
2. **Document edge cases.** Explicitly define and test behaviour for `NaN`, `±Inf`, empty slices, zero variance, unequal group sizes, degenerate tables, and singular matrices. Never silently fall back to heuristic approximations without documented error bounds.
3. **Verified reference oracles (`D1`).** Every quantitative routine must be checked against published reference datasets (such as the 58 NIST StRD datasets in `crates/testkit` / `crates/fit` / `crates/stats`) or an independent verified implementation, with numerical tolerances documented directly alongside the tests. Never invent "reference" answers by eye or treat unverified estimates as ground truth.

### Assets & Zero Bundled Fonts (`xtask assets`)
1. **Every asset needs attribution.** Every image, icon, SVG, cursor, sample file, or third-party data file in the repository must have a row in [ASSETS.md](ASSETS.md) (and an entry in [ATTRIBUTION.md](ATTRIBUTION.md) when third-party), verified by `cargo xtask assets`.
2. **Zero bundled font files.** Do not commit `.ttf`, `.otf`, `.woff`, or `.woff2` files into the repository (`cargo xtask assets` fails if any font binaries are tracked). Native builds discover system fonts dynamically (overridable via `CURVEBOOK_FONTS_DIR`), and optional `bundled-fonts` / web builds stage untracked local SIL OFL fonts from `CURVEBOOK_FONTS_DIR` (`crates/text/build.rs`, `cargo xtask web-fonts`; never network fetches).

### CurveBook Never Crashes
1. **No panicking paths in shipped code.** Workspace `[lints.clippy]` in [Cargo.toml](Cargo.toml) denies:
   - `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable`
   - `unsafe_code` (and every crate must include `#![forbid(unsafe_code)]` and `[lints] workspace = true`)
   - `wildcard_imports`, `cognitive_complexity`, `too_many_arguments`
2. **Validate every external boundary.** File I/O, `.curvebook`/`.pzfx`/`.prism`/CSV parsing, RPC/MCP requests, user expressions, and slice indexing must use checked access (`.get(...)`, checked arithmetic, typed `Result` / `Option` returns) and surface actionable errors to the caller/user rather than crashing.
3. **Last-resort UI/MCP guard.** Keep `crates/engine/src/guard.rs` in place at UI/MCP boundaries only as defence-in-depth—never rely on `catch_unwind` to excuse panics in libraries.

### Pure Rust, `wasm32` Clean Core, and No `chrono`
1. **Rust only.** All production crates, applications, CLI, MCP server, and `xtask` build/verify automation are written in Rust (`edition = "2024"`, `rust-version = "1.99.1"`).
2. **`wasm32-unknown-unknown` compatibility for headless layers.** Keep `L0`–`L5` headless logic (`math`, `palette`, `doc`, `transform`, `stats`, `fit`, `text`, `chart`, `svg`, `pdf`, `render`, `tools`, `engine`) free of unguarded OS/blocking assumptions so `cargo xtask wasm` continues to pass. Place OS/filesystem/process shims behind `curvebook-platform` (`crates/platform`) or `apps/*`.
3. **No `chrono` crate and no hand-rolled calendar math.** `chrono` is intentionally excluded from `Cargo.toml` to keep dependencies minimal and deterministic. Never add `chrono` to workspace manifests and never hand-roll Gregorian leap-year/calendar arithmetic. For native fallback timestamps use `std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)` (recording epoch milliseconds/seconds or `unix:<secs>`), and keep `wasm32`/deterministic library paths dependent on caller-supplied timestamps or `curvebook-platform`.

### Licence Allowlist (Short Policy)
Only permissive, compatible crate licences are allowed in Cargo dependencies:
`MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`.
Copyleft (`GPL-*`, `AGPL-*`, `LGPL-*`) is strictly forbidden. See [docs/licences.md](docs/licences.md) for asset licences, `deny.toml`, `about.toml`, `THIRD-PARTY-LICENSES.html`, `ATTRIBUTION.md`, and `NOTICE`.

### Contributor Credits & Security
Never delete, truncate, or omit contributor attributions in [`contributors/`](contributors/), [CONTRIBUTING.md](CONTRIBUTING.md#6-licensing-attribution--security), `ATTRIBUTION.md`, or `NOTICE` (`cargo xtask contributors` keeps contributor records synced). Report or handle vulnerability details strictly per [SECURITY.md](SECURITY.md).

### Zero Telemetry (`xtask telemetry`)
1. **Strict zero-telemetry & zero-egress architecture.** Never add analytics, crash reporters, telemetry SDKs, auto-updaters, or HTTP client crates anywhere in Cargo manifests or `Cargo.lock`, and never introduce `fetch(`, `XMLHttpRequest`, `navigator.sendBeacon`, `WebSocket`, or external origins in `apps/curvebook-web/` or `packaging/web/`.
2. **Loopback-only local sockets.** Non-test production Rust code may bind TCP sockets only for explicit user-initiated local loopback IPC (`127.0.0.1` / `::1` via `validate_loopback_addr` in `--control` / MCP modes), and GitHub Actions workflows must pin top-level `permissions: contents: read` (with the sole narrow job-scoped exceptions of `.github/workflows/release.yml` job `publish`, which may declare `contents: write`, `id-token: write`, and `attestations: write` for tag-triggered release publishing and keyless Sigstore artifact attestations, `.github/workflows/ci.yml` job `prune-caches`, which may declare `actions: write` for deleting superseded rolling CI cache entries on `main`, and `.github/workflows/pages.yml` job `deploy`, which may declare `pages: write` and `id-token: write` for deploying the staged documentation site) without unvalidated `${{ github.event.* }}` script injection (`cargo xtask telemetry` enforces all 5 checks inside `cargo xtask ci`).
3. **Web security headers.** Keep strict `Content-Security-Policy` (`default-src 'self'`, `connect-src 'self'` for same-origin `.wasm` streaming only and zero external origins), `Referrer-Policy: no-referrer`, `Permissions-Policy`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`, and `X-Content-Type-Options: nosniff` in `packaging/web/_headers` and `packaging/web/.htaccess`.

---

## 2. Command Registries

One of our documented past pitfalls ([docs/pitfalls.md](docs/pitfalls.md)) came from describing an unimplemented command table before it landed and deserializing raw positional tuples. Keep docs and generated files synchronized with the landed typed registries:

- **Engine command registry:** `crates/engine/src/cmd/mod.rs` owns the single typed `CommandSpec` registry (`106` commands across `9` `CommandGroup` categories in [`docs/commands.md`](docs/commands.md): `CommandGroup`, `Effects` bitflags including `NONE` / `MUTATES_DOCUMENT` / `IDEMPOTENT` with `Effects::read_only_hint()`, JSON Schema input specs, `enabled` predicates, and handlers with `serde_path_to_error` typed parameters), exposed via `cmd::registry()`. Whenever adding or editing an engine command, update its `CommandSpec` entry, tests, and regenerated [`docs/commands.md`](docs/commands.md) (`cargo xtask commands`) in one PR, conforming to [docs/command-ids.md](docs/command-ids.md).
- **UI command registry:** UI-only actions use the typed `UiCommandSpec` struct slice `UI_COMMANDS` (`61` entries) in `crates/ui-egui/src/menus/ui_commands.rs`. `crates/mcp/src/ui_catalog.rs` is generated from `UI_COMMANDS` by `cargo xtask ui-commands`—never hand-edit `ui_catalog.rs`, and always check sync with `cargo xtask ui-commands --check`.

---

## 3. Working Rules (Mandatory)

### Roles & Ownership
- Roadmap items in `docs/ROADMAP.md` carry `Owner: maintainers` (maintainer-steered foundation, security, or release tasks) or `Owner: open` (available for any contributor or agent to pick up). Ownership is **never** assigned to an individual handle as a permanent lock.

### Claim Before Starting
1. Check `docs/ROADMAP.md` and open PRs/issues before starting work so two contributors do not duplicate the same `R<m>.<n>` item.
2. Claim the item first—either by noting your claim in `docs/ROADMAP.md` / the tracking issue before branching—so others see it is in progress.
3. If you stop without finishing, release the claim promptly and note partial findings cleanly.
4. For an issue-driven bug fix (see Git Workflow), claim the GitHub issue with a comment instead of a roadmap item.

### Before You Write Code
Read these before editing code:
1. This file (`AGENTS.md`)
2. [docs/pitfalls.md](docs/pitfalls.md) and [docs/pitfalls-engineering.md](docs/pitfalls-engineering.md)
3. [docs/extending.md](docs/extending.md)
4. The relevant domain/reference guide (`docs/architecture.md`, `docs/development.md`, `docs/command-ids.md`, `docs/file-formats.md`, `docs/licences.md`, or `docs/ROADMAP.md`).

### Git Workflow
- **Branch & worktree isolation:** Never make code changes directly on `main` unless explicitly directed. Create a dedicated branch and `git worktree` in a sibling workspace worktree directory (e.g. `../.cb-worktrees/<task-name>`) for implementation work, and remove the worktree once merged/integrated. If the current checkout is already a task-specific feature branch/worktree, continue working there instead of creating a nested worktree. Only the user or a designated integrator merges into `main`.
- **Branch naming:** Use `<prefix>/<R<m>.<n>-short-slug>` with one of these prefixes:
  `feature/`, `bugfix/`, `hotfix/`, `refactor/`, `docs/`, `test/`, `chore/`, `audit/`
  Example: `feature/R0.10-platform-atomic-write`, `bugfix/R2.3-anova-unequal-n`.
- **Pull requests & merge style:** **All merges into `main` must be squash merges** (both GitHub PRs and local integrator merges via `git merge --squash`; do not create merge commits on `main`). Write a clear, plain-English PR or squash-commit title **ending with `(R<m>.<n>)`**, e.g.:
  `Move atomic saves into curvebook-platform (R0.10)`
  *(This replaces the legacy `M1.1: ...` title-prefix convention.)*
- Keep each PR focused on **one** roadmap item (`R<m>.<n>`) unless splitting/grouping is explicitly planned in `docs/ROADMAP.md`.
- **Issue-driven bug fixes:** A fix for a reported GitHub issue that is not part of planned roadmap work needs no roadmap ID.
  - Branch as `bugfix/issue-<N>-short-slug` (or `hotfix/issue-<N>-short-slug`) and keep the PR to that one issue.
  - Leave the issue number out of the PR title, because squash-merge appends the PR number to it.
  - Write `Fixes #<N>` in the PR body and as a trailer in the commit message, so the issue closes whether the fix reaches `main` through a PR or through an integration merge.
  - Start with a failing regression test named `issue_<N>_*`. For a wrong numerical result it must be a `D1` oracle test that cites a published reference and documents its tolerance.
  - Never loosen an existing oracle tolerance or reference value in a bug fix without saying why under **Not verified** in the PR.
  - Run `cargo xtask ci quick` before opening the PR. The project skill `.claude/skills/fix-issue/SKILL.md` walks an agent through these steps.

### Architecture Rules
1. **Headless first:** All document/data/analysis/graph mutations execute through `curvebook-engine` (`Session::execute`), ensuring parity across GUI (`ui-egui` via `Caller::Ui`), CLI (`curvebook-cli`), MCP (`curvebook-mcp`), crash-recovery replay (`ops.journal`), and batch runs (`curvebook.batch.v1`). UI layers must never mutate `Project` directly.
2. **Extend through seams:** Register new commands, analyses, fit equations, file formats, cursors, locales, and schema steps through the canonical seams documented in [docs/extending.md](docs/extending.md).
3. **Data over code:** Prefer declarative descriptor tables, schema definitions, and TOML/struct registries (`CommandSpec`, `UiCommandSpec`, `cursors.toml`, parametric model metadata, format descriptors) over copy-pasted match arms.
4. **Structured results:** Commands, fits, statistical analyses, and imports return typed result structures and typed errors (`EngineError`, `DocError`, `FormatError`) rather than opaque strings.
5. **Versioned files:** Every persisted document, template, settings file, lock/journal envelope, and batch schema has an explicit format version and migration step, and **refuses to overwrite newer versions** saved by a newer CurveBook release ([docs/file-formats.md](docs/file-formats.md)).
6. **Bounded processes & atomic writes via `curvebook-platform`:** All persistent writes (projects, unpacked directories, templates, preferences, exports) must write via temp file + sync + atomic rename—never bare `std::fs::write` over an existing target—and subprocesses must use bounded timeouts and cleaned environments through `crates/platform` (`curvebook-platform`, introduced in `R0.10`).

### Testing Discipline
1. **One general integration test binary per crate (`xtask layers`):** Place new integration tests as submodules inside the crate's single general integration suite (for example `tests/<crate>_suite/main.rs` or `tests/core_suite/main.rs`), never as new top-level `tests/rNN_*.rs` or `tests/rem_*.rs` binaries. Separate integration binaries are reserved for `PERFORMANCE_FILTER` (`r10_*`, `human_wf_*`, `*perf*`, `r15_2_anova_and_1m_bench`), `cargo xtask corpus` (`nist_strd`), `.config/nextest.toml` isolated OS/loopback suites (`control_loopback`, `desktop_host_loop`, `macos_clipboard_live`), `ci features` (`install_fonts`), and feature-gated suites (`r12_docs_site`). `cargo xtask layers` fails on extra general integration binaries.
2. **Earn every test:** Add a test when it guards behaviour a plausible change could break and no existing test already catches. Prefer behavioural relations, end-to-end invariants, `D1` literature/oracle checks (for numerical routines), hostile-input `proptest!` sweeps, and five-stage `WorkflowRun` checks (for human-path GUI claims) over mirroring internal constants, wiring, or copy-pasted fixture boilerplate.
3. **Keep bugfix and audit regressions concise:** Add `issue_<N>_*` and audit-remediation regression cases directly to the owning module's unit tests or the crate's existing integration suite rather than creating standalone per-wave test files.
4. **Never weaken or skip tests to pass a check:** Never delete, `#[ignore]`, or loosen tolerances on an existing test to make a gate green.

### Done Means & "Before You Finish"
A task is **Done** only when:
- All requested code/docs/tests and any oracle assertions pass without skipped assertions or suppressed warnings.
- Generated files (`crates/mcp/src/ui_catalog.rs`, `docs/commands.md`, and `docs/schemas/*.schema.json`) match source definitions (`--check` passes).
- Relevant roadmap/status entries accurately reflect what landed (never leave stale hardcoded counts in docs).

Verification policy:

1. **Use scoped checks while iterating.** Run the tests, lints, oracle assertions, and generated-file or policy checks affected by the change; for inner-loop iteration on foundational crates, `cargo xtask ci quick --direct` runs `policy`, Clippy, and tests for only the directly modified packages without expanding to transitive reverse dependents. Documentation and workflow edits normally need their corresponding checks, not a workspace rebuild.
2. **Run `cargo xtask ci quick` before opening a PR; hosted CI is the full gate.** The default quick tier runs every policy gate (skipping `i18n-report` unless `curvebook-ui-egui` or a workspace-wide build input changed), then Clippy (`--all-targets -D warnings`) for the packages changed since `main` plus their workspace reverse dependents (compile-checking all downstream crates and tests), and the non-performance nextest suite for the **directly modified packages only** (auto-detecting `cargo-nextest` in `$CARGO_HOME/bin` / `$HOME/.cargo/bin` if absent from `PATH`). Pass `--transitive` when changing a shared wire, schema, or cross-crate API contract and wanting local reverse-dependent test execution; root manifest, lockfile, toolchain, or lint configuration edits still select the whole workspace. Hosted CI runs every full stage on every non-docs-only pull request, and a PR merges only when that passes. Run the full local `cargo xtask ci` only when hosted CI cannot provide the evidence you need—broad changes such as workspace renames, dependency or toolchain updates, or architecture changes; once per multi-agent integration wave; or to reproduce a hosted failure—writing its output to a shared log file so independent reviewers and verifiers inspect and cite that shared evidence instead of launching duplicate runs. Do not restart the full suite after every edit or have multiple workers repeat the same full run with identical inputs.
3. **Reuse passing results and drop covered standalone checks when inputs are unchanged.** Record the commands, source revision (including any uncommitted changes), and relevant configuration with the results. Reuse them—and skip standalone `xtask`, `clippy`, or test invocations already covered by that `cargo xtask ci` run—while the checked source, dependencies/lockfile, toolchain, target, features, flags, environment, and fixtures remain unchanged. Preserve checks that provide distinct coverage outside `cargo xtask ci` (such as live stdio/subprocess MCP smoke runs, `CURVEBOOK_REQUIRE_CORPUS=1` corpus verification, packaging scripts, and independent source reviews). A new worktree, agent handoff, or documentation-only commit does not by itself invalidate unrelated passing checks.
4. **Rerun what a fix invalidates.** After a failure, rerun the failed checks and checks affected by the fix first. Repeat full CI when subsequent changes could invalidate broader results; explain why another full run is needed. Never report a failed or partial full run as a complete pass.

Automatic WASM verification is paused. Run `cargo xtask ci wasm` on demand for the target-specific checks.

Keep each worker's verification worktree and isolated `CARGO_TARGET_DIR` stable across retries during a task or wave to retain build caches unless a clean build is specifically needed; do not share a build directory between concurrent workers, and delete retired worker target directories once the wave lands so `/tmp` does not exhaust disk space. Required checks and numerical tolerances remain unchanged.

Select the applicable verification checks below before opening a PR or reporting completion; this is a catalogue, not a requirement to rerun every command for every change:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # or `-p <touched-crate>` while iterating
cargo xtask test
cargo xtask layers
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask cursors
cargo xtask ui-commands --check
cargo xtask secrets
cargo xtask licenses
cargo xtask telemetry
cargo xtask workflows
cargo xtask screenshots --check
cargo xtask commands --check
cargo xtask schemas --check
cargo xtask status
cargo xtask pending
cargo xtask wasm
```

(Running `cargo xtask ci quick [--direct|--transitive] [--base <git-ref>]` executes the fast pre-PR tier described above—use `--direct` for inner-loop iteration, default mode before opening a PR, and `--transitive` when verifying cross-crate API/wire changes locally; running `cargo xtask ci` executes the full workspace CI bundle. For interactive MCP/control/UI workflow checks, run integration targets `human_wf_suite` in `curvebook-ui-egui`, `mcp_suite` in `curvebook-mcp`, `cli_suite` in `curvebook-cli`, and `desktop_suite` in `curvebook`.)

### Parallel Agents
When multiple agents work concurrently on the repository:
1. **Isolate `CARGO_TARGET_DIR` and clean up after landing:** Give every agent its own build directory (for example `CARGO_TARGET_DIR=/tmp/cb-<agent-name>`) so Cargo package locks and build artefacts never collide, retain it across retries within the wave, and remove retired `/tmp/cb-*` build directories when the wave completes to avoid `ENOSPC`.
2. **Strictly non-overlapping crate/file ownership:** Assign disjoint crates or files before starting. Two agents must never edit the same file simultaneously.
3. **No shared-tree git mutations:** Never run `git add`, `git commit`, `git reset`, `git checkout`, or `git config` inside a shared working copy unless you are working in your own isolated git worktree/share.
4. **Never use `/private/tmp` (or `/tmp` worktrees/file-tool paths):** On macOS `/tmp` resolves to `/private/tmp` outside the workspace, causing permission prompts and sandbox-locked leftovers. Keep `git worktree` checkouts and scratch files inside the workspace (e.g. `../.cb-worktrees/<agent-name>`), never pass `/tmp` or `/private/tmp` to `read_file`/`edit_file`/`write_file`, and limit `/tmp` to `run_command` env vars (`CARGO_TARGET_DIR`, `CARGO_HOME`, `RUSTUP_HOME`).
