# Engineering Pitfalls (General Rules Adopted for CurveBook)

These are general Rust, tooling, and engineering rules that CurveBook adopts because each one maps onto a real seam, gate, or likely change in this repository. They were learned from mistakes made elsewhere, so they are written as rules with CurveBook pointers rather than as incident reports. Incidents that happened in CurveBook itself stay in [docs/pitfalls.md](pitfalls.md); read both before your first change.

Each entry has a one-line rule, a short reason, and an **In CurveBook** pointer naming the code, lint, or gate where the rule applies. Where a pointer says a rule is not yet enforced, it is a rule to follow by hand (and a candidate for a future gate), not a claim that a check exists.

Sections: [Git and pull requests](#git-and-pull-requests) · [Build, CI, and supply chain](#build-ci-and-supply-chain) · [Testing](#testing) · [Errors and failure reporting](#errors-and-failure-reporting) · [Files, persistence, and atomic writes](#files-persistence-and-atomic-writes) · [Bounded input and honest truncation](#bounded-input-and-honest-truncation) · [Child processes and OS integration](#child-processes-and-os-integration) · [Concurrency and shared state](#concurrency-and-shared-state) · [Numerics](#numerics) · [Platform-specific code and wasm32](#platform-specific-code-and-wasm32) · [Registries, naming, and single sources of truth](#registries-naming-and-single-sources-of-truth) · [MCP and control surfaces](#mcp-and-control-surfaces) · [UI and rendering](#ui-and-rendering) · [Shell and agent tooling](#shell-and-agent-tooling)

---

## Git and pull requests

**Claim the roadmap item, read from `main`, before you write code.**
A branch that is behind `main` shows items as open that are already done, and a late claim does not stop duplicate work that already started.
*In CurveBook:* claims live in [docs/ROADMAP.md](ROADMAP.md) (AGENTS.md › Claim Before Starting). Read it with `git show origin/main:docs/ROADMAP.md` or after updating your base branch, check the state of any PR a pending line mentions, and create the claim on a correctly prefixed branch (`feature/`, `bugfix/`, `docs/`, …) before implementation. Never describe a late claim as following the claim-first rule.

**Check the current branch before every commit and push, and push by name.**
A branch switch that half-succeeds (switch works, pull refuses) leaves you on `main` while you believe you are on your branch.
*In CurveBook:* `main` only receives squash-merged PRs (AGENTS.md › Git Workflow). Run `git branch --show-current` before committing, use `git push origin <branch>`, and commit or stash before switching. Agents in a shared tree must not run git mutations at all (AGENTS.md › Parallel Agents).

**Run commit, push, PR creation, and merge as separate steps and read each result.**
In a long `&&`/`;` chain a failed hook can stop one step while later lines still run, merging something other than what you meant.
*In CurveBook:* [.githooks/pre-commit](../.githooks/pre-commit) runs `cargo xtask secrets`; let it run with the pinned toolchain from [rust-toolchain.toml](../rust-toolchain.toml) and the same `CARGO_TARGET_DIR` you built with, and do not continue to push or merge until it has passed.

**Merge only the commit that CI actually tested.**
If a branch receives new commits while a script waits for CI, merging "the head" merges untested code.
*In CurveBook:* when merging from a script, pass the tested SHA (for example `gh pr merge --match-head-commit <sha>`, with the SHA taken from the green run), never a head re-read later.

**Write squash messages to a file with real line breaks and read the first result back.**
Escaped newlines and heredocs that collapse silently turn co-author trailers into plain text, and the credit is lost.
*In CurveBook:* contributor credit is a non-negotiable (AGENTS.md › Contributor Credits & Security). After the first merge a script makes, check `git log -1 --format=%B` before it makes another.

**Only push to a PR that is still open, and never rewrite someone else's branch.**
Fixes pushed to an already merged PR never reach `main`, and printing a PR's state does not stop a command chain.
*In CurveBook:* guard the push with a check that fails, for example `test "$(gh pr view <n> --json state -q .state)" = OPEN && git push origin <branch>`. Land late fixes in a new PR from `main`; to supersede another contributor's PR, open a new one that says so instead of force-pushing their branch.

**After resolving a conflict inside a table, merge the edits into one row and look for duplicates.**
Keeping "both sides" of a conflicted row leaves two copies that disagree.
*In CurveBook:* `cargo xtask status` rejects duplicate first-column keys in [docs/STATUS.md](STATUS.md) and [docs/ROADMAP.md](ROADMAP.md); other tables (evidence checklists, [ASSETS.md](../ASSETS.md)) need the same check by eye.

**Judge a bug report against the release it names, not against `main`.**
A fix on `main` may postdate the release the reporter ran.
*In CurveBook:* releases are cut per [docs/release/releasing.md](release/releasing.md); use `git merge-base --is-ancestor <fix-commit> <release-tag>` and compare the message text the report quotes before calling it fixed or still open.

---

## Build, CI, and supply chain

**Test the merge result of PRs that touch a gate or what a gate covers.**
Two PRs can each pass on their own branch and fail together once both are on `main`.
*In CurveBook:* the `untranslated_report` test in `crates/ui-egui/src/i18n.rs` pins the catalogue size, so two PRs that each add a UI string conflict only after merging; the same applies to `cargo xtask layers`, `status`, and generated files. Before merging a PR whose base moved, build the merge (`git merge-tree --write-tree origin/main origin/<branch>`, then a scratch branch) and run `cargo xtask ci` on it; after merging several PRs in a row, run it once more on `main`.

**A dependency change always ships with its `Cargo.lock` change.**
Staging selected folders easily leaves the root lockfile behind.
*In CurveBook:* CI runs `xtask` with `--locked` (`.github/workflows/ci.yml`) and `cargo xtask licenses` reads `Cargo.lock`, so a missing lock entry fails CI. Check `git status` for `Cargo.lock` whenever any `Cargo.toml` changed.

**Run formatting last, after every edit.**
A scripted edit made after `cargo fmt` reintroduces unformatted code.
*In CurveBook:* `cargo xtask ci` starts with `cargo fmt --all -- --check`; run it again as the final step before committing.

**Lint with the toolchain CI uses.**
An older clippy cannot report a lint it does not know, so a local pass proves nothing about CI.
*In CurveBook:* the toolchain is pinned in [rust-toolchain.toml](../rust-toolchain.toml) and in `ci.yml`; [docs/development.md](development.md) explains how a Homebrew `cargo` on `PATH` bypasses that pin. Confirm with `rustup show active-toolchain`.

**Give every worktree its own `CARGO_TARGET_DIR`, and never let `xtask` from one tree check another.**
`xtask` finds the repository from the path baked in when it was compiled, so a shared target dir makes `cargo xtask` in one tree read another tree's files.
*In CurveBook:* `workspace_root()` in `xtask/src/main.rs` uses `env!("CARGO_MANIFEST_DIR")`. AGENTS.md › Parallel Agents already requires a separate `CARGO_TARGET_DIR` per agent; after accidentally sharing one, rebuild `xtask` before trusting a gate.

**Reuse one target dir for sequential work, and clean up the ones you create.**
Each fresh target dir is a full rebuild, and per-agent dirs plus incremental caches fill disks quickly; a running binary also cannot be rebuilt in place on Windows.
*In CurveBook:* separate dirs are for concurrent agents (AGENTS.md › Parallel Agents). Keep a long-running `curvebook --control` session in its own target dir, delete an agent's target dir once its work has merged, check `df -h` before a large build, and consider `CARGO_INCREMENTAL=0` for gate runs.

**Check a build requirement on every path that ends without the thing required.**
A "required" switch that is only consulted on one failure path lets other paths finish silently without it.
*In CurveBook:* `crates/text/build.rs` reads `CURVEBOOK_FONTS_REQUIRED` and fails when `CURVEBOOK_FONTS_DIR` is unset, the manifest is unreadable, or a listed face is missing. It is only consulted when the `bundled-fonts` feature is enabled; keep every new early return in that script behind the same requirement check.

**Keep secret and local files out through the repository, not personal configuration.**
A file ignored only by one contributor's global ignore list can be committed by anyone else.
*In CurveBook:* the repository [.gitignore](../.gitignore) lists `.env*`, key files, and local agent settings, and the pre-commit hook runs `cargo xtask secrets`. Add new local or secret file patterns there.

**Test a scanner on what must survive as well as on what it must catch.**
Substrings flag harmless names, and a rule written for one kind of text breaks another.
*In CurveBook:* `xtask/src/secrets.rs` matches key suffixes and exact names and skips placeholder values; `xtask/src/brands.rs` has an allow marker and context words. When changing a rule, add a test for a near-miss that must pass alongside the case that must fail, and name exceptions explicitly instead of loosening the pattern.

**Build a scanner's own patterns from parts so it does not match itself.**
A literal pattern in the scanner's source (or a process search's command line) is found by the scan.
*In CurveBook:* `xtask/src/secrets.rs` assembles its PEM markers with `concat!`; do the same for any new self-referential pattern.

---

## Testing

**Tests never touch the network, launch GUI programs, or write outside a temporary directory.**
A test that reaches the developer's real configuration, clipboard, or desktop can corrupt their state, and a widened code path can turn a harmless test into one that launches something.
*In CurveBook:* tests point configuration at a temp dir with `CURVEBOOK_CONFIG_DIR`, `--portable-dir`, or `CURVEBOOK_NO_PREFS`, and OS viewer launches in `crates/ui-egui/src/os_integration.rs` stay disabled unless `CURVEBOOK_ALLOW_OS_GUI_DIALOG` is set. `crates/ui-egui/tests/macos_clipboard_live.rs` still writes the real macOS pasteboard during a normal test run; new live tests should be opt-in in the same way. When you widen what a function does, grep the tests that call it.

**Locate test binaries with `env!("CARGO_BIN_EXE_<name>")`, never by Cargo's folder layout.**
Cargo changes where intermediate artefacts live between releases.
*In CurveBook:* the integration tests in `apps/curvebook/tests/` already do this; keep it that way. When CI breaks on a commit that previously passed, compare the toolchain lines of the two runs first.

**A test that depends on host data or programs must say so, and CI must require it.**
Otherwise it passes where the dependency is missing and nobody notices it stopped testing anything.
*In CurveBook:* the NIST StRD corpus tests in `crates/testkit/src/nist_strd/corpus.rs` skip when the corpus is absent unless `CURVEBOOK_REQUIRE_CORPUS=1`, which `cargo xtask corpus` sets. Follow that pattern for any new host-dependent test, and run new tests once on a machine without the dependency.

**Run the full test set with `--no-fail-fast`, and rerun whole crates after a fix.**
A failure in one test binary stops Cargo before later crates run, and a filtered rerun hides regressions elsewhere.
*In CurveBook:* `cargo xtask test` forwards extra arguments, so use `cargo xtask test --no-fail-fast` when hunting failures, then rerun the affected crates without a filter.

**Anything a test changes for the whole process belongs in a process of its own.**
Process-wide state (environment variables, statics, global flags) leaks into tests running in parallel on other threads.
*In CurveBook:* edition 2024 makes `std::env::set_var` unsafe and the workspace forbids `unsafe_code`, so tests re-run themselves in a child process with the variable set (see the `CURVEBOOK_FONTS_DIR` check in `crates/text/tests/r10_ui_shaping.rs`). Tests that share a mutable `static` need a shared lock or their own test binary.

**Never release a port and hand its number to someone else.**
Between dropping a listener and the child binding the port, another test can take it.
*In CurveBook:* `apps/curvebook/tests/recovery_wiring.rs` binds `127.0.0.1:0`, drops the listener, and passes the port to a child `--control` process; prefer letting the server bind `:0` and report its port, or retry on a bind conflict. For a refused-connection probe, connect to port zero instead of borrowing a released port.

**Give each test case its own temporary directory.**
Code that walks upward or scans siblings finds another case's fixtures.
*In CurveBook:* tests build unique temp dirs from the process id and a counter or timestamp (for example `crates/platform/src/fs.rs` tests); one directory per case, removed at the end.

**Loopback fixtures read the whole request before replying and set socket options right after connecting.**
TCP can split even a short line, closing on unread bytes resets the connection, and setting an option on a socket the peer already closed fails on some systems only.
*In CurveBook:* the fake control servers in `crates/mcp/tests/` read a full JSON line per request; keep a size cap and a deadline on those reads, set accepted sockets to blocking before applying timeouts, and set read timeouts immediately after `connect`.

**A stand-in server answers what it was scripted for and treats anything else as unknown.**
A mock that replies to every request in order gives a new, unexpected request the reply meant for another.
*In CurveBook:* the fake control servers in `crates/mcp/tests/mcp_e2e.rs` dispatch on the request `method` and record which methods arrived; keep new mocks keyed the same way.

**Wait for a condition with a deadline, not for a number of frames or a sleep.**
Timing-based assertions pass on fast machines and fail in CI once work moves to another thread.
*In CurveBook:* desktop and control tests poll for a token file or child exit until a deadline (`apps/curvebook/tests/recovery_wiring.rs`). When work moves off the UI thread, grep the tests that exercise it and make them wait on the result.

**Prove a no-behaviour-change refactor by comparing output bytes.**
Substring assertions miss changed output.
*In CurveBook:* the CPU chart renderer and headless `egui` tessellation rasterizer are deterministic (`crates/render`, `crates/ui-egui/src/egui_shell/rasterizer.rs`), and SVG/PDF writers produce text; dump outputs for the relevant combinations before and after and compare byte for byte.

**Break a guard once to see it fail, and keep its forbidden input precise.**
A guard that matches the very wording it protects, or that can no longer fail, gives false confidence.
*In CurveBook:* `verify_tr_keys_exist_rejects_unregistered_key` in `crates/ui-egui/src/i18n.rs` and the negative self-tests in `xtask/src/layers.rs` show the pattern: every new gate gets a test that feeds it a violation.

**A green check that ran nothing is not a pass.**
Skipped jobs, early `return`s, and "SKIPPED" prints all look like success.
*In CurveBook:* several tests return early when the sandbox denies loopback sockets or the platform lacks a tool (for example `macos_clipboard_live.rs`, `recovery_wiring.rs`); read test output, not only the exit status. `cmd_i18n_report` in `xtask/src/main.rs` verifies that exactly one named test ran, which is the model for gates that delegate to a test. On PRs, read each check's conclusion rather than the merge state.

---

## Errors and failure reporting

**An error says what ran, what it printed, and a next step you actually spell out.**
"See the output below" with no output, or a remediation that names a command it does not give, leaves the person stuck.
*In CurveBook:* `EngineError::with_remediation` and the `context` field of `PlatformError::Io` carry this; test a new error message by causing the failure and reading what the caller sees.

**A best-effort step may fail without stopping the operation, but it still says why.**
Swallowed errors (`let _ =`, `.ok()`, `.is_ok()`) make missing work invisible, while optional steps that fail the main path make it fragile.
*In CurveBook:* `persist_window_preferences_best_effort` in `apps/curvebook/src/main.rs` logs a warning and leaves the file untouched; `sync_dir` in `crates/platform/src/fs.rs` treats unsupported directory-`fsync` OS errors (`EINVAL`, `ENOTSUP`, `EACCES`, `EISDIR`, `EBADF`) as non-fatal no-ops while propagating genuine storage failures (`EIO`, `ENOSPC`). Prefer that shape over a silent `let _ =`.

**Check a child's exit status before treating its output as a result.**
A program that failed before doing anything produces empty output that reads as "zero results".
*In CurveBook:* `ProcessOutput::success` in `crates/platform/src/process.rs`; the uid probe in `crates/mcp/src/tools.rs` checks it before parsing. Embedded helper scripts for other interpreters (the PowerShell clipboard helpers in `os_integration.rs`) should be exercised on their platform, since nothing compiles them during a normal build.

**Every request gets exactly one terminal reply, including on early errors.**
A preflight error that returns without the reply the caller is waiting for hangs the client.
*In CurveBook:* `crates/mcp/src/protocol.rs` returns a JSON-RPC error response for invalid params and unknown methods, and `crates/ui-egui/src/control_io.rs` answers each control line; keep that true for any new early-exit path and test the reply, not only the side effect.

---

## Files, persistence, and atomic writes

**There is one atomic writer; do not write another.**
Hand-rolled temp-and-rename code repeats old bugs (shared temp names, missing sync, leftover temp files).
*In CurveBook:* `curvebook_platform::fs::write_atomic` / `write_atomic_with_hook` in [crates/platform/src/fs.rs](../crates/platform/src/fs.rs) (see [docs/pitfalls.md](pitfalls.md) #5). The journal's create-if-absent lease files in `crates/format/src/recovery.rs` have different semantics (exclusive creation, not replacement); new replace-style writes go through the platform helper.

**Give each persisted file one owner; another writer merges with what is on disk.**
Two writers that each save their own in-memory copy undo each other's changes.
*In CurveBook:* `save_shortcut_preferences` in `apps/curvebook/src/main.rs` re-reads the existing preferences file and updates only the fields it owns before writing.

**Only "not found" means there is nothing to lose.**
A read that fails for any other reason (busy, unreadable, not UTF-8, newer version) must not fall back to defaults that are later saved over the real file.
*In CurveBook:* `read_regular_file_bytes_if_exists` returns `None` only for `NotFound`; preferences loading warns and continues with defaults, while saving refuses when the existing file cannot be read, and `ensure_existing_preferences_not_newer` refuses to overwrite a newer version (`crates/format/src/versioned.rs`). Keep that split for any new settings or state file.

**Save to disk first; change memory only after the write succeeded.**
Otherwise the UI shows a state that was never persisted.
*In CurveBook:* `run_project_save` in `crates/engine/src/cmd/project.rs` writes through the path policy before reporting success; follow the same order for any command that both persists and updates session state.

**Create a private file with its final permissions in the first open.**
Creating with default permissions and narrowing later leaves a window in which another process can open it.
*In CurveBook:* `write_control_token_file` in `apps/curvebook/src/main.rs` narrows the token to 0600 in the pre-rename hook, after the bytes are written (while `write_atomic` sets `0o600` at creation in `create_unique_sibling_temp` only when overwriting an existing file); the private directory checks (`ensure_private_uid_dir` in `apps/curvebook/src/main.rs`) are what protect first creation today. New private files should set the mode at creation. Keep temp names short and independent of the target name, as `create_unique_sibling_temp` does by capping the stem.

**Create a lock's directory first, and treat only "already exists" as held.**
On a fresh install the directory is missing, and reading every create error as "someone else holds it" waits forever.
*In CurveBook:* `DirectoryJournalBackend::open_dir` creates the journal directory and `try_create_file_exclusively` maps only `AlreadyExists` to "held" (`crates/format/src/recovery.rs`).

**A file that other tools can edit is read as untrusted, with an error that says how to go on.**
Users, scripts, and version control edit unpacked project directories; a written convention is not a guard.
*In CurveBook:* unpacked `.curvebook.d/` projects are verified entry by entry in `crates/format/src/archive.rs`; errors should name the file and what to do, not only the parse failure.

**Compare paths as paths, on the text as written.**
String prefixes match sibling folders (`proj-backup` starts with `proj`), and a lower-cased copy made for keyword matching is wrong for case-sensitive file systems.
*In CurveBook:* `WorkdirGuard::resolve` in `crates/platform/src/path_guard.rs` canonicalises and uses component-wise `Path::starts_with`; test confinement rules with a sibling folder whose name extends the root's, and with capital letters.

**Do not store machine-local identities in a file that travels between machines.**
An absolute path or per-machine id saved in a project breaks when the project is opened elsewhere.
*In CurveBook:* `LinkSource::ExternalDelimitedFile` in `crates/doc/src/links.rs` stores a path string inside the project; resolve such references relative to the project where possible and report a missing source as "not available here" rather than failing the whole open.

**Sanitised names can collide; detect the collision instead of losing data.**
Mapping arbitrary titles or file stems to a safe character set maps distinct inputs to the same output.
*In CurveBook:* `sanitize_sheet_id` in `crates/mcp/src/formats.rs` derives a sheet id from a file stem; the engine rejects a duplicate id with a conflict error (`crates/engine/src/cmd/table.rs`), so the caller can pass an explicit `sheet_id`. Any new derivation (export file names, temp names) needs the same check.

---

## Bounded input and honest truncation

**Bound the work, not only what is kept, and check before allocating.**
A limit on retained items does nothing about an input that makes you visit, expand, or allocate without bound.
*In CurveBook:* `ZipLimits` (entry count, per-entry and total uncompressed budgets) in `crates/format/src/zip.rs` and `archive.rs`, `MAX_IMPORT_CELLS` in `crates/format/src/csv.rs`, and `read_regular_file_bytes` in `crates/platform/src/fs.rs` (rejects FIFOs and oversized files from metadata before reading). Apply a budget to every consumer of the same input, and stop at the first item over the limit.

**Apply a cap on every code path, including fallbacks.**
A cap on the main parser does nothing if the path chosen for odd input has none.
*In CurveBook:* the CSV, `.pzfx`, and other project importers in `crates/format/src/` each need the same cell and entry budgets; test the fallback path with more items than the cap.

**A long computation has an overall budget, and running out of it is a reported failure.**
A per-step limit does not bound a sequence of steps, and a computation that is cut off silently looks like a slow one.
*In CurveBook:* `OdeFitBudgetGuard` in `crates/fit/src/dsl/ode.rs` bounds total adaptive integration steps across one nonlinear fit; new iterative, resampling, or permutation routines need a comparable whole-run budget that surfaces as a typed error.

**When a list is capped, say so and keep the full total beside it.**
A reader takes a silently capped list, or counts computed from it, for the whole set.
*In CurveBook:* `crates/engine/src/methods_report.rs` adds a "Showing first N of M stored rows" footnote when it trims a result table. Do the same for any capped MCP output, inspect tree, or report, and test one item past the cap.

**A reader keeps what it understands and reports what it skipped; unknown is not the default.**
Rejecting a whole file for one unknown flag loses readable data, while silently substituting defaults for unread content reports facts nobody checked.
*In CurveBook:* the `.pzfx` importer and its sibling project importers return an `ImportReport` with `ImportWarning`s (`UnsupportedChunk`, `IgnoredSection`, `CoercedValue`, …) in `crates/format/src/error.rs`. A stopped or partial read is typed data, and nothing downstream treats it as complete.

**Size a receiver's limit to the largest legitimate message its sender can produce.**
A line or frame cap smaller than a real reply fragments it into pieces that never parse.
*In CurveBook:* `MAX_REQUEST_BYTES` in `crates/mcp/src/protocol.rs`, `MAX_CONTROL_RESPONSE_BYTES` in `crates/mcp/src/tools.rs`, and `MAX_CONTROL_LINE_BYTES` in `crates/ui-egui/src/control_io.rs` must stay consistent; when a reply can grow (screenshots, large inspect trees), test with a payload near the cap.

**A partial update changes only the fields it was given.**
Treating an omitted field as "reset to default" silently undoes earlier settings.
*In CurveBook:* `merge_json_struct` and `de_present_opt` in `crates/engine/src/cmd/analysis.rs` (also used by `graph.rs` and `layout.rs`) overlay a patch onto the current spec; test new update commands with omitted, null, and explicit values.

**An empty filter selects nothing, or is an error, never "everything".**
An empty allowlist that falls back to "all" quietly enables what the caller meant to exclude.
*In CurveBook:* `handle_enable_toolset` in `crates/mcp/src/toolsets.rs` rejects an empty group selection; keep that behaviour for any new list-valued filter and test the empty case.

---

## Child processes and OS integration

**A deadline covers its own cleanup.**
Killing a child and then waiting for it without a limit can still hang, and joining reader threads can block while a grandchild holds the pipe open.
*In CurveBook:* `ProcessCommand` in [crates/platform/src/process.rs](../crates/platform/src/process.rs) enforces wall and idle timeouts, places children in their own process group (`process_group(0)` on Unix, `CREATE_NEW_PROCESS_GROUP` on Windows), runs `ChildGuard::kill_group_and_wait`, and bounds error-path stream-reader joining with `drain_workers_on_error` (`ERROR_DRAIN_TIMEOUT`), while `.discard_output()` redirects stdout/stderr to null for backgrounded clipboard owners (`wl-copy`, `xclip`). Because `kill_group_and_wait` still calls `child.wait()` directly after killing, keep `.discard_output()` on forking clipboard launchers and verify after live runs that nothing started remains running.

**A launcher that hands off and exits is not the program it launched.**
`open`, `xdg-open`, and `Start-Process` return once the viewer starts, and on some desktops a launcher instead stays alive as long as the viewer.
*In CurveBook:* `os_integration.rs` opens staged PDFs through these launchers under the default wall timeout; treat success as "handed off", and be aware that a launcher which blocks until the viewer closes will hit that timeout.

**Pass anything that can grow on stdin or in a file, never in argv.**
Command lines have OS length limits (notably on Windows) and some launchers mangle multi-line arguments.
*In CurveBook:* `os_integration.rs` already sends clipboard text through `ProcessCommand::stdin_bytes` and staged files; keep new integrations the same.

**Children of the windowed app must not open console windows.**
On Windows each console subprocess started from a GUI process flashes a window unless told not to.
*In CurveBook:* `ProcessCommand::spawn_child` sets `CREATE_NEW_PROCESS_GROUP` on Windows but not `CREATE_NO_WINDOW`, and the desktop app starts `powershell` for clipboard, printer, and viewer integration (`crates/ui-egui/src/os_integration.rs`); check the desktop app, not only the CLI, after adding a Windows subprocess.

**"Not found" from spawn does not always mean the program is missing.**
The same error is returned when a script's interpreter is missing.
*In CurveBook:* `spawn_child` maps every `NotFound` to `PlatformError::ProgramNotFound`, and `os_integration.rs` then tries the next candidate; when reporting that nothing worked, name what was tried rather than asserting it is not installed.

**GUI apps do not get the shell's environment.**
An app started from a dock or desktop launcher sees a short system `PATH`, so bare program names found in a terminal can vanish.
*In CurveBook:* the macOS integrations use absolute paths (`/usr/bin/osascript`, `/usr/bin/open`); Linux and Windows candidates use bare names (`xdg-open`, `wl-copy`, `xclip`, `powershell`). Test new integrations from a launched app, not only from a terminal.

**Hand other programs absolute, native paths.**
A relative path is resolved again against the child's working directory, and some tools ignore paths with the wrong separator.
*In CurveBook:* `WorkdirGuard::resolve` in `crates/platform/src/path_guard.rs` yields canonical paths; build arguments from `Path`/`PathBuf` values rather than formatted strings.

**Text output of system tools differs between platforms and locales.**
Flag syntax and message wording vary across Linux, macOS, and FreeBSD, and messages may be translated.
*In CurveBook:* `is_unix_pid_absent` in `crates/format/src/recovery.rs` matches `/bin/kill` error text and treats an unrecognised message as "owner still alive" (the safe direction); keep new parsers conservative and run them on the nightly FreeBSD job (`.github/workflows/freebsd.yml`) when they matter there.

**A resource that must not outlive a crash is not released only by `Drop`.**
A killed process runs no destructors, so helpers and lock files tied to a guard leak.
*In CurveBook:* the journal lease in `crates/format/src/recovery.rs` records an owner process and checks its liveness instead of relying on drop; `ControlTokenFileGuard` in `apps/curvebook/src/main.rs` removes the token on drop, so a stale token can remain after a crash. Test such resources by killing the owner, not by dropping the guard.

**Smoke-test the finished package, not the build tree.**
A binary that works in `target/` can fail once renamed, relocated, or shipped without its companion binary.
*In CurveBook:* `packaging/macos/verify.sh` and `packaging/linux/apprun-test.sh` exist for this; when packaging adds or renames a binary, run the packaged executables (including `curvebook-cli`) from the output before publishing.

**Choose the GPU backends you can use; do not start every backend the library offers.**
Starting a backend can load a faulty driver that crashes natively before any panic hook or log runs, and default adapter choice can pick a GPU that is not driving the display.
*In CurveBook:* `run_native_desktop` in `apps/curvebook/src/gui_window.rs` starts eframe with the `wgpu` feature and default backend selection. The compositor already renders on the CPU and only uploads a texture, so a narrow backend set is enough; when touching the native options, keep the library's environment overrides working and consider leaving a startup marker that the next launch can read.

---

## Concurrency and shared state

**Bind a lock guard once per statement, and never re-lock what the caller holds.**
A temporary guard lives to the end of its statement, so a second lock in the same expression deadlocks, and a handler that locks the session it is running under waits forever.
*In CurveBook:* [clippy.toml](../clippy.toml) disallows raw `std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard}` across all targets, and command handlers receive the session through `Ctx` (`crates/engine/src/cmd/mod.rs`) rather than reaching for a shared one. Use `curvebook_platform::sync::{Mutex, RwLock}` (`crates/platform/src/sync.rs`), bind each guard to a local variable (`let guard = ...;`), and drop it before calling back into code that could acquire the same or another lock.

**Always use `curvebook_platform::sync::{Mutex, RwLock}` with descriptive names and explicit long-job markers.**
Raw synchronization primitives hang silently on self-reentry, AB/BA lock-order inversions, and wedged holders, and `wasm32-unknown-unknown` panics on `std::time::Instant::now()` or background watchdog threads.
*In CurveBook:* construct locks with `Mutex::named` / `RwLock::named`, handle `LockError::{WouldDeadlock, TimedOut, Poisoned}` explicitly (or use `lock_or_recover` / `read_or_recover` / `write_or_recover` for stubs/caches that opt into poison recovery, never `unwrap()` / `expect()` / `catch_unwind`), keep heavy computation outside the lock guard, and mark intentionally long critical sections with `lock_long_job` / `read_long_job` / `write_long_job` so waiters are not cut off at the normal wait limit. Only `crates/platform/src/sync.rs` may carry narrow `#[allow(clippy::disallowed_types)]` annotations on the wrapped primitives themselves.

**Atomic rename does not make read-modify-write safe across processes.**
Each writer can read, modify, and rename its own copy and silently drop the other's update.
*In CurveBook:* the crash-recovery journal serialises writers with an exclusive lease file (`crates/format/src/recovery.rs`); shared files written by several windows or processes (preferences, recovery state) need a lock that covers the whole load-modify-write sequence, tested with separate processes.

---

## Numerics

**Sum floats from `+0.0` with a compensated accumulator, and normalise `-0` before display.**
`Iterator::sum::<f64>()` over an empty iterator yields `-0.0`, which formats with a minus sign.
*In CurveBook:* `curvebook_math::sum` / `sum_iter` (Neumaier) in `crates/math/src/sum.rs`; `format_number_sig_figs` in `crates/fit/src/tex_format.rs` and the TSV formatter in `crates/tools/src/grid.rs` normalise `-0`. Route new reductions and number formatting through these.

**Compare floats used as identities by their bits, and keep the original bits.**
`==` treats `NaN` as unequal to itself and `+0.0` equal to `-0.0`, and widening or narrowing can change a `NaN`'s payload.
*In CurveBook:* the ODE trajectory cache in `crates/fit/src/dsl/ode.rs` compares keys with `slice_bits_eq`; cache keys, fingerprints, and deduplication use `f64::to_bits`, while numeric comparison and display use values.

---

## Platform-specific code and wasm32

**Code behind a `cfg` is checked only by a job that compiles that configuration.**
A list or signature change can break a `cfg(windows)` test or a `wasm32` stub while the main CI job stays green.
*In CurveBook:* the main CI gate (`.github/workflows/ci.yml`) runs on Linux; the Windows ARM64 and FreeBSD jobs run nightly or when their packaging, manifest, or toolchain paths change (`.github/workflows/windows-arm64.yml`, `freebsd.yml`), and `cargo xtask wasm` type-checks `curvebook-web` and `curvebook-engine` and runs `clippy` on `curvebook-web` and `curvebook-ui-egui` for `wasm32-unknown-unknown`. When you change code that a `cfg(windows)`, `cfg(target_os = …)`, or `cfg(target_arch = "wasm32")` branch uses (for example `os_integration.rs` or the wasm stubs in `crates/format/src/versioned.rs`), build that target with `cargo check --target …` or say in the PR that you could not.

**A `/bin/sh` script must run under a strict POSIX shell, not only bash.**
On many Linux systems `/bin/sh` is a minimal shell whose built-ins differ from bash.
*In CurveBook:* [.githooks/pre-commit](../.githooks/pre-commit) uses `sh` (while packaging scripts under `packaging/` use explicit `bash` shebangs); `cargo xtask workflows` runs `shellcheck` on packaging and hook scripts, which checks the dialect named in the shebang. Try `sh` scripts with `dash` as well.

**Keep line endings stable across platforms.**
Editors and scripts on Windows can rewrite every line of a file to CRLF.
*In CurveBook:* [.gitattributes](../.gitattributes) sets `eol=lf`; after scripted edits, check `git ls-files --eol` for touched files.

---

## Registries, naming, and single sources of truth

**One implementation per rule; call it from every place that needs it.**
A fix applied to one of two copies leaves the other wrong, with no compiler warning.
*In CurveBook:* configuration-directory resolution from `CURVEBOOK_CONFIG_DIR` exists both in `apps/curvebook/src/main.rs` and in `crates/mcp/src/tools.rs`, and control token limits are defined in both `crates/mcp/src/tools.rs` and `crates/ui-egui/src/control_io.rs`. When changing either, change (or better, share) both, and add a regression test at each consumer's public entry point.

**Build each registry through one function that every front end uses.**
A front end that assembles its own registry misses what was added elsewhere.
*In CurveBook:* `cmd::registry()` (`crates/engine/src/cmd/mod.rs`) and `FormatRegistry::default_registry()` (`crates/format/src/registry.rs`) are the canonical builders ([docs/extending.md](extending.md)); the MCP export path wraps the default registry in `crates/mcp/src/formats.rs`, so register new formats in the default registry, not in a front end.

**Put facts about an item on the item, not in a separate list of names.**
A list of ids elsewhere drifts silently when items are renamed or removed.
*In CurveBook:* `CommandSpec` carries `Effects` bitflags, and the MCP layer derives tool annotations from them (`ToolAnnotations::from_effects` in `crates/mcp/src/toolsets.rs`); add new per-command facts as fields or flags there.

**Read the id scheme before naming anything, and check external naming rules before fixing a public contract.**
Ids chosen outside the scheme get renamed later, and protocol clients reject names outside their allowed character set.
*In CurveBook:* [docs/command-ids.md](command-ids.md) and `is_valid_command_id_format` in `crates/engine/tests/registry_and_errors.rs`; engine ids are dotted, MCP tool names are derived in snake case and checked by `per_command_mcp_names_are_valid_and_collision_free` in `crates/mcp/src/tools.rs`.

**Check which layer owns what a feature calls before planning it.**
A feature placed in a lower layer cannot reach a higher one, and discovering that mid-implementation forces a redesign.
*In CurveBook:* the layer rules in [docs/architecture.md](architecture.md) are enforced by `cargo xtask layers`; if a lower layer needs a capability owned higher up, plan a registration seam.

**A user-visible label is a UI string with translation obligations.**
Adding a command or title that the UI displays adds work in every locale table.
*In CurveBook:* `UiCommandSpec` labels are shown through `i18n::command_label` (`crates/ui-egui/src/menus/tooltips.rs`); run `cargo xtask i18n-report` for any PR that adds or renames a UI command, not only for UI-only changes.

**Before adding an option, write one sentence on what it gives that the existing path does not.**
An option that duplicates an existing path under a new name confuses users and costs maintenance; designs built on assumed sizes or costs go wrong.
*In CurveBook:* `UiCommandSpec` has a `summary` field (`crates/ui-egui/src/menus/ui_commands.rs`); if you cannot write it, ask what was meant. Measure on real projects before designing around where you assume the cost is.

---

## MCP and control surfaces

**A restricted mode holds on every entry point, not just the main one.**
One side door that skips the mode check defeats it.
*In CurveBook:* strict `--gui-only` (`GuiOnlyPolicy::Strict`, AGENTS.md › Command Registries) must reject direct mutations, `doc_inspect`, `resources/read`, and disk-path screenshots alike; when adding an MCP or control method, add it to the `gui_only_surface` / `mcp_gui_only_parity` coverage.

**Tools that agents drive do what they safely can and take whole sets.**
A tool that refuses for a fixable reason (missing output folder, one item per call) pushes the agent to work around it, often badly.
*In CurveBook:* batch commands such as `table.setCells` and `EngineError::with_remediation` messages; error text should say how to fix the call.

**Offer a client only the tools its job needs, and measure the list.**
A very large `tools/list` crowds out a client's context and degrades tool choice.
*In CurveBook:* toolsets and groups in `crates/mcp/src/toolsets.rs` (and the dedicated `gui` toolset); measure the serialized size of `tools/list` with the real registry when adding many tools or long descriptions.

**Never print secrets in inspection or log output.**
Tool results reach logs and agent transcripts.
*In CurveBook:* the control token (`CURVEBOOK_CONTROL_TOKEN`, token files under the config directory) must not appear in `ui.inspect` trees, `doc_inspect`, error messages, or logs; any field that can hold a secret is redacted in every tool that prints it, with a test that the value does not appear.

---

## UI and rendering

**Animations request frames at their own pace and only while active.**
A widget that asks for a repaint every frame keeps a core busy for as long as it is on screen.
*In CurveBook:* `tick_animations_and_redraw` in `apps/curvebook/src/gui_window.rs` requests a repaint only while a window is coasting; new animations (spinners, progress) follow that pattern.

**Painting reads only memory; never read files or start programs while drawing.**
Disk or process work in the paint path freezes the window on slow storage and can leave orphaned children.
*In CurveBook:* `crates/ui-egui/src/egui_shell/` renders strictly from in-memory state, and `cargo xtask layers` bans raw file writes and process spawning in `crates/ui-egui/src`; load anything from disk on an explicit action or a worker, and show the last result.

**Lay out from the current settled frame's measurements, not stale previous-frame rects.**
A size measured after drawing and applied next frame shows every change one frame late, which reads as flicker or drift.
*In CurveBook:* the `R15` desktop shell ([docs/architecture.md](architecture.md)) drives paint, pointer hit-testing, AccessKit accessibility nodes, and `ui.inspect` (`crates/ui-egui/src/egui_shell/inspect_bridge.rs`) from the same settled `!ctx.will_discard()` `egui` pass (`verify_egui_frame_parity`); measure inside the settled pass, and test that panel geometry stays stable over consecutive frames.

**When something flickers, render consecutive frames and diff them.**
Unit tests that check text do not see what is painted over it.
*In CurveBook:* `render_egui_screenshot_rgba` in `crates/ui-egui/src/egui_shell/rasterizer.rs` is deterministic, so consecutive frames can be rendered and compared pixel by pixel in a test.

**Content of unbounded length lives inside a scroll area.**
Variable-length content placed outside it squeezes other panels and pushes controls off-screen.
*In CurveBook:* dialogs, inspectors, and result tabs in `crates/ui-egui`; a close button on something that cannot be brought back is a delete, so offer collapsing instead.

**Check theme contrast with numbers in both themes.**
Theme colours borrowed by many widgets can make a part invisible in one theme.
*In CurveBook:* `WCAG_AA_UI_MIN_RATIO` and `wcag_contrast_ratio` in `crates/ui-egui/src/theme.rs`; add a contrast assertion for any new themed element.

**Scroll something into view when it changes, never as a standing per-frame request.**
A per-frame scroll-to request fights the user's own scrolling.
*In CurveBook:* `ensure_coord_visible` in `crates/tools/src/grid.rs` is called when the selection moves; keep new auto-scroll behaviour event-driven.

---

## Shell and agent tooling

**Write multi-line edit scripts to a file and run the file.**
Heredocs carrying quotes, triple quotes, or escaped newlines get mangled by the shell.
*In CurveBook:* use the editor tool or a script file in your scratch area; never pipe a script into an interpreter from an inline heredoc.

**Never run anything that can page or prompt.**
A pager or an interpreter waiting on empty stdin blocks until the tool times out, sometimes with work left in a stash.
*In CurveBook:* use `git --no-pager` (or `GIT_PAGER=cat`), never start an interpreter with an empty script, and after a timeout check `git stash list` and `git status` before continuing.

**Do not hide a command's errors when an empty result means something.**
A missing command plus `2>/dev/null` looks exactly like "no matches".
*In CurveBook:* macOS lacks some GNU tools (for example `timeout`); set time limits on the tool call instead, and keep stderr visible for searches.

**Pipe file lists into `xargs`; do not rely on word splitting.**
zsh, the default macOS shell, does not split an unquoted variable into words.
*In CurveBook:* applies to every scripted edit or search across the workspace; check `git status` that the edit landed before reading test results.

**A scripted edit checks every anchor before writing anything, and is bounded to its section.**
A script that fails half-way leaves a half-edited tree, and a search for "the next item" can run into the next section.
*In CurveBook:* edits to [docs/ROADMAP.md](ROADMAP.md) and other sectioned docs stop at the next heading; read `git diff` of every scripted edit.

**Build test binaries before running them under a time limit.**
The first build can use most of the limit and kill the test mid-run, leaving its children behind.
*In CurveBook:* run `cargo test --no-run` (or `cargo xtask test --no-run`) first, then the tests under the limit.

**Use `git stash -u` and read `git status` before `git add -A`.**
A plain stash leaves new files behind, and they end up in another branch's commit.
*In CurveBook:* applies in your own worktree only; shared trees allow no git mutations (AGENTS.md › Parallel Agents).
