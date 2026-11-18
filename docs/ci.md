# CI stages and TCP fixture deadlines

`cargo xtask ci` remains the full local verification entry point. Hosted CI runs
the same checks as separate stages, so a failure identifies the responsible
stage and cannot prevent unrelated matrix stages from reporting their results.
The final **Workspace CI gate** requires the build and every check stage to pass;
cancelled or skipped dependencies fail that gate.

| Stage | Checks |
|---|---|
| `cargo xtask ci build` | Build and archive every native workspace test target, without running tests. |
| `cargo xtask ci policy` | Formatting, assets, brands, cleanroom, cursors, exact i18n report, UI/engine command and schema sync, layers, secrets, licences, telemetry, workflow lint, screenshot integrity, web bundle readiness, pending items, and status. Hosted CI also runs the full `cargo deny check`. |
| `cargo xtask ci lint` | Workspace Clippy, every target, with warnings denied. |
| `cargo xtask ci tests` | Every native test except the performance-sensitive binaries selected by `PERFORMANCE_FILTER` in `xtask/src/ci.rs`. |
| `cargo xtask ci performance` | Every `r10_*` binary (including `r10_phase7_gates` and the landed Wave 5 latency checks), every `human_wf_*` human-path workflow binary with its input-to-paint budgets, plus `*perf*` grid and curve-fit benchmark binaries, serially; existing assertions and budgets are unchanged. |
| `cargo xtask ci doctests` | Workspace doctests, then `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"` so broken or private intra-doc links and other rustdoc warnings fail CI. |
| `cargo xtask ci features` | Text tests without default features; every UI test and doctest without default features, with its performance-sensitive binaries serial; MCP compilation without default features. |
| `cargo xtask ci wasm` | Web and engine compilation for `wasm32-unknown-unknown`, including their dependencies. |

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

Hosted jobs use the pinned nextest and actionlint binaries and SHA-256 checksums
in `.github/actions/ci-setup/action.yml`, and only install `cargo-deny` in the
shared-cache writer and `policy` stage. `.config/nextest.toml` keeps the existing
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
