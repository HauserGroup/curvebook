# CurveBook Agent & Contributor Rules

Binding rules for **CurveBook** (crate prefix `curvebook-*`, repo root `curvebook/`). Keep this file slim: always-on rules live here, and task-specific policies live in the guides below.

## Before You Start — Read by Task

| Task | Read first |
|---|---|
| Any code change | [docs/pitfalls.md](docs/pitfalls.md), [docs/pitfalls-engineering.md](docs/pitfalls-engineering.md) |
| Adding commands, analyses, models, formats, locales, cursors, migrations, or assets | [docs/extending.md](docs/extending.md), [docs/command-ids.md](docs/command-ids.md), [docs/file-formats.md](docs/file-formats.md) |
| Clean-room boundaries, naming, `.pzfx`/`.prism` interop, upstream GUI reuse, or local reference frames | [docs/clean-room.md](docs/clean-room.md) |
| Statistical, numerical, or curve-fitting routines (`D1`) | [docs/development.md#tests--numerical-verification](docs/development.md#tests--numerical-verification) and `docs/implementation/` |
| Crate layers, build setup, fonts, robustness, or test layout | [docs/architecture.md](docs/architecture.md), [docs/development.md](docs/development.md) |
| Dependencies, licences, assets, or contributor credits | [docs/licences.md](docs/licences.md), [CONTRIBUTING.md#6-licensing-attribution--security](CONTRIBUTING.md#6-licensing-attribution--security) |
| UI tokens, localization, or human-path GUI workflow tests | [docs/design-language.md](docs/design-language.md), [docs/localization.md](docs/localization.md), [docs/extending.md#writing-a-workflow-test](docs/extending.md#writing-a-workflow-test) |
| Published user docs, manual pages, or website HTML | [docs/release/github-pages.md#audience-tiers-scientist-guides-vs-technical-references](docs/release/github-pages.md#audience-tiers-scientist-guides-vs-technical-references), [docs/extending.md#writing-user-documentation](docs/extending.md#writing-user-documentation) |
| CI stages, `.github/workflows/`, web headers, verification policy, or parallel agents | [docs/ci.md](docs/ci.md) |
| Claiming tasks, branching/PRs, or fixing a reported GitHub issue | [docs/ROADMAP.md](docs/ROADMAP.md), [CONTRIBUTING.md](CONTRIBUTING.md), [.claude/skills/fix-issue/SKILL.md](.claude/skills/fix-issue/SKILL.md) |
| Cutting or verifying releases | [docs/release/releasing.md](docs/release/releasing.md) |

---

## 1. Non-Negotiable Principles

- **Clean-room & neutral naming (`xtask cleanroom`, `xtask brands`):** Implement independently with original visual design; never inspect proprietary binaries/assets or depend on/translate `GPL`/`AGPL`/`LGPL` code. Write copy in CurveBook's own words and exclude competitor product names and vendor fixtures. Lowercase `".pzfx files"` and `".prism files"` naming is explicitly allowed and preferred on user-facing surfaces. Full rules, exceptions, upstream GUI reuse, and local reference-frame limits live in [docs/clean-room.md](docs/clean-room.md).
- **Statistical rigor (`D1`):** Every routine in `crates/{math,stats,fit,transform}` must cite a peer-reviewed paper, textbook, or NIST StRD spec (`cargo xtask citations`), define and test degenerate edge cases (`NaN`, `±Inf`, empty, zero variance, singular matrices), and validate against NIST StRD or an independent verified oracle with documented tolerances ([docs/development.md#tests--numerical-verification](docs/development.md#tests--numerical-verification)).
- **CurveBook never crashes:** Zero `unsafe` (`#![forbid(unsafe_code)]`, `[lints] workspace = true`) and zero panicking paths (`unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`, denied in [Cargo.toml](Cargo.toml)) in shipped code. Validate every external boundary with checked access and typed errors; `crates/engine/src/guard.rs` is defence-in-depth only ([docs/development.md#robustness-curvebook-never-crashes](docs/development.md#robustness-curvebook-never-crashes)).
- **Pure Rust, `wasm32` core & no `chrono`:** All production crates, apps, and `xtask` are Rust (`edition = "2024"`, `rust-version = "1.99.1"`). Keep `L0`–`L5` crates `wasm32-unknown-unknown` clean (OS shims belong in `crates/platform` or `apps/*`). Never add `chrono` or hand-roll calendar/leap-year math; use `SystemTime::UNIX_EPOCH` for native fallbacks and caller/platform timestamps in library/`wasm32` code.
- **Assets, fonts & licences (`xtask assets`, `xtask licenses`):** Track every asset in [ASSETS.md](ASSETS.md) (and [ATTRIBUTION.md](ATTRIBUTION.md) if third-party); never commit font binaries (`.ttf`/`.otf`/`.woff`/`.woff2`; [docs/development.md#fonts](docs/development.md#fonts)). Dependencies must be permissive-only with zero copyleft ([docs/licences.md](docs/licences.md)).
- **Credits & security:** Never delete or omit contributor credits in [`contributors/`](contributors/), [ATTRIBUTION.md](ATTRIBUTION.md), or [NOTICE](NOTICE) (`cargo xtask contributors`). Handle vulnerabilities privately per [SECURITY.md](SECURITY.md).
- **Zero telemetry & zero egress (`xtask telemetry`):** No analytics, crash reporters, auto-updaters, HTTP client crates, or browser egress. Non-test TCP sockets are loopback-only (`127.0.0.1`/`::1`). Workflows must pin `permissions: contents: read` (outside allowlisted deploy/publish/prune jobs) without script injection, and web bundles must preserve strict CSP (`connect-src 'self'`). Full rules in [docs/ci.md#zero-egress-workflow-permissions--web-headers](docs/ci.md#zero-egress-workflow-permissions--web-headers).

---

## 2. Command Registries

Keep generated artifacts synced with the typed registries (never hand-edit generated files or use positional tuples; [docs/pitfalls.md](docs/pitfalls.md)):

- **Engine commands:** `crates/engine/src/cmd/mod.rs` (`cmd::registry()`, `116` commands) -> regenerate [docs/commands.md](docs/commands.md) with `cargo xtask commands`.
- **UI commands:** `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`, `65` entries) -> regenerate `crates/mcp/src/ui_catalog.rs` with `cargo xtask ui-commands`.

Before adding or editing commands, read [docs/extending.md](docs/extending.md) and [docs/command-ids.md](docs/command-ids.md).

---

## 3. Working Rules

### Claiming & Git Workflow

- Claim an unclaimed `R<m>.<n>` item in [docs/ROADMAP.md](docs/ROADMAP.md) (or comment on the GitHub issue for non-roadmap bug fixes) before branching; release the claim if stopping early.
- Never edit `main` directly unless directed: work on a prefixed branch in a sibling worktree (`../.cb-worktrees/<task>`), keep each PR to one item, and squash-merge with a plain-English title ending in `(R<m>.<n>)`. Only the user or a designated integrator merges into `main`.
- For reported GitHub issues, branch as `bugfix/issue-<N>-<slug>`, write a failing `issue_<N>_*` test first (`D1` oracle for numerical bugs), put `Fixes #<N>` in the PR body and commit trailer, and omit `#<N>` from the title. Full workflow in [CONTRIBUTING.md](CONTRIBUTING.md) and [.claude/skills/fix-issue/SKILL.md](.claude/skills/fix-issue/SKILL.md).

### Architecture Rules

1. **Headless first:** Route all document, data, analysis, and graph mutations through `curvebook-engine` (`Session::execute`); UI layers never mutate `Project` directly.
2. **Extend through seams:** Register new capabilities only via the canonical seams in [docs/extending.md](docs/extending.md).
3. **Data over code:** Prefer declarative registries and descriptor tables over copy-pasted match arms.
4. **Structured results:** Return typed results and errors (`EngineError`, `DocError`, `FormatError`), not opaque strings.
5. **Versioned files:** Version every persisted format with forward migrations and refuse to overwrite files from newer releases ([docs/file-formats.md](docs/file-formats.md)).
6. **Atomic writes & bounded processes:** Use `curvebook-platform` temp-file + sync + atomic rename (never bare `std::fs::write` over an existing target) and bounded, clean-environment subprocesses.
7. **Anti-bloat:** Edit existing modules, extend declarative tables, and delete superseded code rather than adding parallel wrappers or per-dialog boilerplate.
8. **Keep `xtask` thin:** Do not add new `xtask` subcommands or bespoke parsers without maintainer approval; reuse `xtask/src/scan_util.rs` and avoid net LOC growth.

### Published Documentation (`docs-site --check`)

Respect the `<!-- Audience: Tier 1/2 -->` header on every published page: keep Tier 1 scientist guides short and plain-language, place flags/schemas/wire details on Tier 2 references, and keep crate paths and roadmap IDs in unpublished Tier 3 docs ([docs/release/github-pages.md#audience-tiers-scientist-guides-vs-technical-references](docs/release/github-pages.md#audience-tiers-scientist-guides-vs-technical-references), [docs/extending.md#writing-user-documentation](docs/extending.md#writing-user-documentation)).

### Testing Discipline

1. **One general integration binary per crate (`xtask layers`):** Add tests inside the crate's single general suite (`tests/<crate>_suite/main.rs` or `tests/core_suite/main.rs`), never extra top-level `tests/*.rs` binaries.
2. **Earn every test:** Guard real behaviour (invariants, `D1` oracles, hostile-input `proptest!`, or five-stage `WorkflowRun` GUI tests), not constants or wiring.
3. **No wave- or audit-named test files:** Never add `rem_*`, `audit_*_s*`, `wave*_*`, or `rNN_*` files/submodules; add regressions to the owning module or canonical domain suite.
4. **Never weaken or skip tests:** Never delete, `#[ignore]`, or loosen tolerances on an existing test to pass a gate.

Details and reserved binary exceptions: [docs/development.md#tests--numerical-verification](docs/development.md#tests--numerical-verification) and [docs/extending.md#writing-a-workflow-test](docs/extending.md#writing-a-workflow-test).

### Verification & Parallel Agents

- **Done means:** Tests, lints, oracles, and generated-file `--check` gates (`ui_catalog.rs`, `docs/commands.md`, `docs/schemas/*.schema.json`) pass with zero warnings; roadmap/status entries stay accurate.
- **Verification:** Iterate with scoped checks (`cargo xtask ci quick --direct`, `cargo xtask workflows`, on-demand `cargo xtask ci wasm`); run `cargo xtask ci quick` before opening a PR (`--transitive` for cross-crate API/wire changes; hosted CI is the full gate). Reuse passing results when inputs are unchanged; never report a partial or failed run as passed.
- **Parallel agents:** Keep crate/file ownership disjoint, make no git mutations in a shared tree, never pass `/tmp` or `/private/tmp` to file tools (keep worktrees/scratch inside the workspace), and isolate concurrent `CARGO_TARGET_DIR` builds using fixed slots (`/tmp/cb-slot-1`..`/tmp/cb-slot-3`, `CARGO_INCREMENTAL=0` for one-shot gates, pruned after landing).

Full `cargo xtask` check catalogue, reuse/rerun policy, and slot-pool protocol: [docs/ci.md](docs/ci.md) and [docs/pitfalls-engineering.md](docs/pitfalls-engineering.md).
