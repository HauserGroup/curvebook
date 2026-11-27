# CI stages and TCP fixture deadlines

`cargo xtask ci` remains the full local verification entry point. Hosted CI runs
the same checks as separate stages, so a failure identifies the responsible
stage and cannot prevent unrelated matrix stages from reporting their results.
The final **Workspace CI gate** requires `classify` and the selected branch to pass
(`build` and every check stage on full runs, or `docs-policy` on docs-only pull
requests); cancelled or skipped dependencies fail that gate.

On `pull_request` events, a fast `classify` job inspects the PR diff and takes a
`docs-policy` fast path (`cargo fmt --all -- --check` and `cargo xtask ci policy`,
skipping the native build and full 6-stage matrix) only when every changed file
matches a strict documentation allowlist (`docs/**` excluding
`docs/control-protocol.md` and `docs/schemas/**`, `plan/**`, and root prose docs
`README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `CLAUDE.md`, `ROADMAP.md`,
`SECURITY.md`). Any other path, as well as every `push` to `main` and
`workflow_dispatch` run, always executes the full build and all 6 stages.

Automatic WASM verification is paused in GitHub Actions, the release workflow,
and the default local CI run. Run `cargo xtask ci wasm` on demand. The web
crate's native tests and static packaging and telemetry checks still run.

| Stage | Checks |
|---|---|
| `cargo xtask ci quick [--direct\|--transitive] [--base <git-ref>]` | Local pre-PR tier, not a hosted stage: every `policy` gate (`i18n-report` runs only when `curvebook-ui-egui` is directly touched or a workspace-wide build input changed), then Clippy (`-D warnings`, all targets) for the packages changed since the merge base with `main` plus their workspace reverse dependencies, and the non-performance `tests` stage filter for the **directly modified packages only** (auto-detecting `cargo-nextest` in `$CARGO_HOME/bin` / `$HOME/.cargo/bin` when absent from `PATH`). Pass `--direct` during inner-loop iteration to restrict both Clippy and tests to directly modified packages; pass `--transitive` to also run reverse-dependent tests locally (`xtask` tests stay omitted when `xtask` is only pulled in transitively). Staged, unstaged, and untracked files count as changed. Root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`, or `.cargo/` edits select the whole workspace; documentation-only changes run `policy` alone. It skips `doctests`, `features`, and `performance`, which hosted CI runs on every non-docs-only pull request. |
| `cargo xtask ci build` | Build and archive every native workspace test target, without running tests. |
| `cargo xtask ci policy` | Formatting, assets, brands, cleanroom, cursors, exact i18n report, UI/engine command and schema sync, layers, secrets, licences, telemetry, workflow lint, screenshot integrity, web bundle readiness, numerical `D1` literature citations (`cargo xtask citations`), pending items, and status. Hosted CI also runs the full `cargo deny check`. |
| `cargo xtask ci lint` | Workspace Clippy, every target, with warnings denied. |
| `cargo xtask ci tests` | Every native test except the performance-sensitive binaries selected by `PERFORMANCE_FILTER` in `xtask/src/ci.rs`. |
| `cargo xtask ci performance` | Every `r10_*` binary (including `r10_phase7_gates` and the landed Wave 5 latency checks), every `human_wf_*` human-path workflow binary with its input-to-paint budgets, plus `*perf*` grid and curve-fit benchmark binaries, serially; existing assertions and budgets are unchanged. |
| `cargo xtask ci doctests` | Workspace doctests, then `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"` so broken or private intra-doc links and other rustdoc warnings fail CI. |
| `cargo xtask ci features` | Text tests without default features; `curvebook-ui-egui` compile-check of all targets (`--all-targets`) plus library unit tests (`--lib`), the `install_fonts` integration test, and doctests without default features (avoiding repeating all integration and serial performance binaries); MCP compilation check without default features. |
| `cargo xtask ci wasm` | On demand. Web and engine compilation checks plus `curvebook-web` and `curvebook-ui-egui` Clippy (`-D warnings`) for `wasm32-unknown-unknown`, `trunk build --cargo-profile web-release` when `trunk` is installed (`CURVEBOOK_REQUIRE_TRUNK=1` requires `trunk`), and web bundle readiness/size-budget verification (`CURVEBOOK_REQUIRE_WEB_BUNDLE=1` requires `dist/release/curvebook-web-*.zip`). |

The native and performance selections are exact complements. The full local
command uses nextest when available; without it, `cargo test --workspace
--all-targets` includes every performance-sensitive binary once. The standalone `r10-gate` command is
still available for scoped work. Splitting stages does not relax any statistical
oracle, performance budget, warning policy, or assertion.

## Builds, caching, and test execution

The build job creates a nextest archive before running tests and saves its cache
when that job ends. Later test failures therefore do not discard the completed
build cache. Only the build job writes that shared cache. Native correctness and
performance jobs download the exact archive produced for their revision, rather
than compiling their test binaries again. Other stages reuse the build cache and
compile any different metadata, feature, doctest, or target configuration they
need. Jobs that build or unpack full test archives reclaim unused preinstalled
runner SDK directories beforehand so cold-cache compilations and multi-gigabyte
archive extractions do not exhaust runner disk space. Uploaded archives are
removed before the build cache is saved, avoiding a second cached copy.
Artifacts are retained for two days.

The hosted correctness job fetches the locked workspace dependencies for audit
tests that call Cargo metadata offline, including dependencies for other targets.
Before the native correctness stage, hosted CI runs
`cargo xtask corpus --fetch-only` to download and verify the pinned NIST StRD
datasets. It runs the archived tests with `CURVEBOOK_REQUIRE_CORPUS=1` and
`CURVEBOOK_CORPUS_DIR` pointing at the checkout, so missing or invalid fixtures
fail the required check. For the same required-corpus behavior locally, fetch
the corpus first and run `CURVEBOOK_REQUIRE_CORPUS=1 cargo xtask ci`.

Hosted jobs use the pinned nextest and actionlint binaries and SHA-256 checksums
in `.github/actions/ci-setup/action.yml`, install `cargo-deny` only in the
shared-cache writer and `policy` stage, and install `cargo-about` only in the
shared-cache writer and `tests` stage. `.config/nextest.toml` keeps the existing
minimum-version requirement and adds a `ci` profile: no retries, failing
five-minute per-test deadlines, and a one-minute deadline for `control_loopback`.
The live macOS clipboard test reserves all test slots while it runs, preventing
other test binaries from replacing its pasteboard contents. Its byte-for-byte
assertions are unchanged. Performance checks have their own runner and one test thread; the budget values
remain unchanged. A budget failure is still a failure and needs investigation.

To verify archive execution locally after `cargo xtask ci build`, copy the built
`xtask` executable outside the archived `target/debug/` paths and invoke that copy
with `CURVEBOOK_CI_TEST_ARCHIVE=target/ci-tests.tar.zst` and `ci tests` or
`ci performance`. Hosted jobs copy the driver into the runner temporary directory
before extraction, so restoring the archive cannot overwrite a running `xtask`. Build and execution checkouts must have the same source
revision. Hosted jobs restore archived binaries under the workspace's `target/`
paths as well as remapping nextest metadata, preserving integration tests that
use Cargo's compile-time executable paths.

## TCP fixture failure behavior

Tests using the in-process TCP listener run their bodies in child test processes
with a 45-second wall deadline through `curvebook-platform::ProcessCommand`.
The subprocess runner kills and reaps a stalled child. This also bounds plain
`cargo test`, independently of nextest.

The UI app is initialized before starting short client deadlines. Clients read
and validate the `app.quit` acknowledgement before closing their socket. A client
error or panic triggers a separate authenticated cleanup request; the original
error or panic remains a failing result. Negative regression tests exercise both
cleanup paths and the child-process deadline.

Linux can reset a rejected HTTP connection when unread request data remains.
The HTTP rejection test accepts that close only if it already received the exact,
complete forbidden response. It still verifies that the attack creates no file
and performs no document mutation.

## Zero-egress, workflow permissions & web headers

`cargo xtask telemetry` (part of `cargo xtask ci policy`) enforces all five privacy and supply-chain boundaries:

1. **Strict zero-telemetry & zero-egress architecture:** Never add analytics, crash reporters, telemetry SDKs, auto-updaters, or HTTP client crates anywhere in Cargo manifests or `Cargo.lock`, and never introduce `fetch(`, `XMLHttpRequest`, `navigator.sendBeacon`, `WebSocket`, or external origins in `apps/curvebook-web/` or `packaging/web/`.
2. **Loopback-only local sockets:** Non-test production Rust code may bind TCP sockets only for explicit user-initiated local loopback IPC (`127.0.0.1` / `::1` via `validate_loopback_addr` in `--control` / MCP modes).
3. **Pinned GitHub Actions workflow permissions & injection guard:** Every workflow in `.github/workflows/` must pin top-level `permissions: contents: read` and avoid unvalidated `${{ github.event.* }}` script injection. The sole permitted job-scoped write exceptions are:
   - `.github/workflows/release.yml` job `publish`: `contents: write`, `id-token: write`, `attestations: write` (tag-triggered release publishing and keyless Sigstore artifact attestations);
   - `.github/workflows/ci.yml` job `prune-caches`: `actions: write` (deleting superseded rolling CI cache entries on `main`);
   - `.github/workflows/pages.yml` job `deploy`: `pages: write`, `id-token: write` (deploying the staged documentation site).
4. **Web security headers:** Keep strict `Content-Security-Policy` (`default-src 'self'`, `connect-src 'self'` for same-origin `.wasm` streaming only and zero external origins), `Referrer-Policy: no-referrer`, `Permissions-Policy`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`, and `X-Content-Type-Options: nosniff` in `packaging/web/_headers` and `packaging/web/.htaccess`.

## Local verification policy

1. **Use scoped checks while iterating.** Run the tests, lints, oracle assertions, and generated-file or policy checks affected by the change; for inner-loop iteration on foundational crates, `cargo xtask ci quick --direct` runs `policy`, Clippy, and tests for only the directly modified packages without expanding to transitive reverse dependents. Documentation and workflow edits normally need their corresponding checks, not a workspace rebuild.
2. **Run `cargo xtask ci quick` before opening a PR; hosted CI is the full gate.** Pass `--transitive` when changing a shared wire, schema, or cross-crate API contract and wanting local reverse-dependent test execution. Hosted CI runs every full stage on every non-docs-only pull request, and a PR merges only when that passes. Run the full local `cargo xtask ci` only when hosted CI cannot provide the evidence you need—broad changes such as workspace renames, dependency or toolchain updates, or architecture changes; once per multi-agent integration wave; or to reproduce a hosted failure—writing its output to a shared log file so independent reviewers and verifiers inspect and cite that shared evidence instead of launching duplicate runs. Do not restart the full suite after every edit or have multiple workers repeat the same full run with identical inputs.
3. **Reuse passing results and drop covered standalone checks when inputs are unchanged.** Record the commands, source revision (including any uncommitted changes), and relevant configuration with the results. Reuse them—and skip standalone `xtask`, `clippy`, or test invocations already covered by that `cargo xtask ci` run—while the checked source, dependencies/lockfile, toolchain, target, features, flags, environment, and fixtures remain unchanged. Preserve checks that provide distinct coverage outside `cargo xtask ci` (such as live stdio/subprocess MCP smoke runs, `CURVEBOOK_REQUIRE_CORPUS=1` corpus verification, packaging scripts, and independent source reviews). A new worktree, agent handoff, or documentation-only commit does not by itself invalidate unrelated passing checks.
4. **Rerun what a fix invalidates.** After a failure, rerun the failed checks and checks affected by the fix first. Repeat full CI when subsequent changes could invalidate broader results; explain why another full run is needed. Never report a failed or partial full run as a complete pass.

### Verification check catalogue

Select the applicable checks before opening a PR or reporting completion (this is a catalogue, not a requirement to rerun every command for every change):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # or `-p <touched-crate>` while iterating
cargo xtask test
cargo xtask layers
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask design-tokens
cargo xtask ui-commands --check
cargo xtask secrets
cargo xtask licenses
cargo xtask telemetry
cargo xtask workflows
cargo xtask screenshots --check
cargo xtask commands --check
cargo xtask schemas --check
cargo xtask citations
cargo xtask status
cargo xtask pending
cargo xtask wasm
```

For interactive MCP/control/UI workflow checks, run integration targets `human_wf_suite` in `curvebook-ui-egui`, `mcp_suite` in `curvebook-mcp`, `cli_suite` in `curvebook-cli`, and `desktop_suite` in `curvebook`.

## Parallel agents

When multiple agents work concurrently on the repository:

1. **Bound `CARGO_TARGET_DIR` to fixed slots and clean up after landing:** Never share a `CARGO_TARGET_DIR` between concurrent agents (Cargo locks and `xtask`'s baked-in `CARGO_MANIFEST_DIR` would collide), and keep each worker's verification worktree and slot stable across retries during a task or wave to retain build caches unless a clean build is specifically needed. Do not invent unbounded per-task `/tmp/cb-<slug>` directories: assign each active concurrent worker a slot from a small fixed pool (`CARGO_TARGET_DIR=/tmp/cb-slot-1` .. `/tmp/cb-slot-3`), rebuild `xtask` when switching a slot to a different worktree (see [docs/pitfalls-engineering.md](pitfalls-engineering.md)), pass `CARGO_INCREMENTAL=0` on one-shot verifier/audit/gate runs so throwaway builds do not accumulate multi-GB incremental caches, and delete or prune unused `/tmp/cb-*` target directories once a wave lands so `/tmp` does not exhaust disk space.
2. **Strictly non-overlapping crate/file ownership:** Assign disjoint crates or files before starting. Two agents must never edit the same file simultaneously.
3. **No shared-tree git mutations:** Never run `git add`, `git commit`, `git reset`, `git checkout`, or `git config` inside a shared working copy unless you are working in your own isolated git worktree/share.
4. **Never use `/private/tmp` (or `/tmp` worktrees/file-tool paths):** On macOS `/tmp` resolves to `/private/tmp` outside the workspace, causing permission prompts and sandbox-locked leftovers. Keep `git worktree` checkouts and scratch files inside the workspace (e.g. `../.cb-worktrees/<agent-name>`), never pass `/tmp` or `/private/tmp` to `read_file`/`edit_file`/`write_file`, and limit `/tmp` to `run_command` env vars (`CARGO_TARGET_DIR`, `CARGO_HOME`, `RUSTUP_HOME`).
