# Contributing to CurveBook

Thank you for contributing to **CurveBook**! This repository is built in Rust (`edition = "2024"`, `rust-version = "1.99.1"`, pinned via `rust-toolchain.toml`) under `MIT OR Apache-2.0`.

Before writing code or opening a pull request, read [AGENTS.md](AGENTS.md) and [docs/pitfalls.md](docs/pitfalls.md)—all engineering, clean-room, statistical, and verification rules apply equally to human contributors and AI coding agents.

---

## 1. Where to Start

- **Engineering & governance rules:** [AGENTS.md](AGENTS.md)
- **Known pitfalls to avoid:** [docs/pitfalls.md](docs/pitfalls.md)
- **How to add features cleanly:** [docs/extending.md](docs/extending.md)
- **Build prerequisites & harnesses:** [docs/development.md](docs/development.md) (check `rustc --version` and `cargo --version` report **`1.99.1+`** per [docs/development.md#prerequisites](docs/development.md#prerequisites), then build with `cargo build --release -p curvebook-cli` or `cargo build --release -p curvebook`)
- **Architecture, formats & protocols:** [docs/architecture.md](docs/architecture.md) · [docs/command-ids.md](docs/command-ids.md) · [docs/commands.md](docs/commands.md) · [docs/file-formats.md](docs/file-formats.md) · [docs/licences.md](docs/licences.md) · [docs/mcp.md](docs/mcp.md) · [docs/control-protocol.md](docs/control-protocol.md) · [docs/implementation/scripting-and-automation.md](docs/implementation/scripting-and-automation.md)
- **Scientific & UI specifications:** [docs/implementation/data-tables-and-transforms.md](docs/implementation/data-tables-and-transforms.md) · [docs/implementation/statistics-implementation.md](docs/implementation/statistics-implementation.md) · [docs/implementation/curve-fitting-implementation.md](docs/implementation/curve-fitting-implementation.md) · [docs/implementation/graphing-and-layouts.md](docs/implementation/graphing-and-layouts.md) · [docs/gui-workflows.md](docs/gui-workflows.md) · [docs/evidence/user-interface.md](docs/evidence/user-interface.md) · [docs/evidence/overview.md](docs/evidence/overview.md)
- **Current plan & claimable tasks (`R<m>.<n>`):** [docs/ROADMAP.md](docs/ROADMAP.md)
- **Live project status & metrics:** [docs/STATUS.md](docs/STATUS.md)
- **Documentation index & issues:** [docs/README.md](docs/README.md) · [Bug report](.github/ISSUE_TEMPLATE/bug_report.yml) · [Numerical result report](.github/ISSUE_TEMPLATE/numerical_result.yml) · [Feature request](.github/ISSUE_TEMPLATE/feature_request.yml)

---

## 2. Rules Every PR Must Respect

1. **Independent implementation and original design:** Never inspect the binaries, memory, or bundled assets of proprietary commercial statistical or curve-fitting applications or copy their code, wording, assets, or distinctive visual design. The [limited local JPG reference policy](AGENTS.md#limited-local-jpg-reference-policy) permits only high-level workflow, information-hierarchy, and generic functional-organization observations from the designated existing frames; its provenance, handling, and verification restrictions apply. Never depend on or translate copyleft (`GPL-*`, `AGPL-*`, `LGPL-*`) code. Write everything in CurveBook's own words (`cargo xtask cleanroom` and `cargo xtask brands` verify this).
2. **Never crash:** Workspace clippy lints deny `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable`, and `unsafe_code`. Handle malformed files, empty selections, degenerate numerical inputs, and RPC/MCP calls via typed `Result` / `Option` errors and bounds-checked slices.
3. **Statistical rigor & `D1` oracles:** Every routine in `crates/math`, `crates/stats`, `crates/fit`, and `crates/transform` must cite a peer-reviewed reference, document degenerate/edge cases, and validate against verified published/oracle datasets (such as NIST StRD) rather than unverified approximations.
4. **Assets & zero bundled fonts:** Every asset added to the repository needs a row in [ASSETS.md](ASSETS.md) (and [ATTRIBUTION.md](ATTRIBUTION.md) if third-party). Never commit `.ttf`, `.otf`, `.woff`, or `.woff2` files (`cargo xtask assets`).
5. **No `chrono` crate & no hardcoded prose counts:** Do not add `chrono` to `Cargo.toml` or hand-roll calendar math; use `std::time::SystemTime::UNIX_EPOCH` / `curvebook-platform` timestamps (see [AGENTS.md](AGENTS.md)). Never hardcode volatile counts (commands, tests, lines) in prose outside machine-checked/generated files (`docs/STATUS.md`, `docs/commands.md`).
6. **Permissive licences only:** New dependencies must conform to the allowlist in [docs/licences.md](docs/licences.md).

---

## 3. Claiming Work in `docs/ROADMAP.md`

1. Browse [docs/ROADMAP.md](docs/ROADMAP.md) for an unclaimed `Owner: open` task (`R<m>.<n>`), and check open PRs/issues so nobody duplicates work.
2. Claim the item first in `docs/ROADMAP.md` (and/or an issue referencing its `R<m>.<n>` ID) before branching.
3. Keep each PR focused on a single `R<m>.<n>` task whenever practical.
4. If you step away before finishing, unclaim the task in `docs/ROADMAP.md` or on the issue so others can pick it up cleanly.

---

## 4. Branching, Editing & Pull Requests

1. **Read first:** [AGENTS.md](AGENTS.md), [docs/pitfalls.md](docs/pitfalls.md), [docs/extending.md](docs/extending.md), and the relevant guide ([docs/command-ids.md](docs/command-ids.md), [docs/file-formats.md](docs/file-formats.md), [docs/licences.md](docs/licences.md)).
2. **Branch name:** Prefix with one of `feature/`, `bugfix/`, `hotfix/`, `refactor/`, `docs/`, `test/`, `chore/`, or `audit/` and include the roadmap ID (`R<m>.<n>`):
   ```sh
   git checkout -b feature/R0.10-atomic-platform-writes
   ```
3. **Regenerate derived files when touching source registries/schemas:**
   - UI commands (`UI_COMMANDS`) -> `cargo xtask ui-commands`
   - Engine / UI command catalog -> `cargo xtask commands` *(R0)*
   - Persisted schemas -> `cargo xtask schemas` *(R0)*
   - Windows icons -> `cargo xtask ico`
4. **Pull request title & merge policy:** PRs are **squash-merged**. Use a plain-English PR title **ending with `(R<m>.<n>)`**, for example:
   `Use atomic temp-and-rename saves via curvebook-platform (R0.10)`
5. **Before you finish:** Fill out [.github/pull_request_template.md](.github/pull_request_template.md) honestly and pass the verification suite:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo xtask test
   cargo xtask layers
   cargo xtask assets
   cargo xtask brands
   cargo xtask cleanroom
   cargo xtask cursors
   cargo xtask ui-commands --check
   cargo xtask wasm
   # Plus R0 gates where active on your branch:
   # cargo xtask secrets && cargo xtask commands --check && cargo xtask schemas --check && cargo xtask status && cargo xtask pending
   ```
   Or run `cargo xtask ci quick` for the policy gates plus Clippy and tests of the crates your branch changed and their dependents; hosted CI runs the full `cargo xtask ci` stages on every pull request.

---

## 5. AI Coding Agents

AI assistants working in this repository must read [AGENTS.md](AGENTS.md) and [CLAUDE.md](CLAUDE.md) (which pulls in [docs/pitfalls.md](docs/pitfalls.md)) before making edits, respect non-overlapping crate ownership (`CARGO_TARGET_DIR` isolation for parallel agents), and never invent unimplemented abstractions in docs or skip failed checks.

---

## 6. Licensing, Attribution & Security

- **Contributions:** All contributions are licensed under **`MIT OR Apache-2.0`** ([Cargo.toml](Cargo.toml)); see [docs/licences.md](docs/licences.md) for dependency and asset rules.
- **Contributor credits:** Never omit or remove existing credits in [docs/contributors.md](docs/contributors.md), [ATTRIBUTION.md](ATTRIBUTION.md), or [NOTICE](NOTICE).
- **Security issues:** Report vulnerabilities privately according to [SECURITY.md](SECURITY.md)—do not open public issues with exploit details.
