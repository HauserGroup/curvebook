# `apps/lumencraft/src/desktop_host.rs`: verified findings

- **Base:** fda1cc0106092834d97b53289ec2a99c1bf0439c (`desktop_host.rs`, `main.rs` and `gui_window.rs` match HEAD; `desktop_host.rs` was last changed in 461ad33)
- **Input:** an external audit report of `desktop_host.rs` dated 2026-11-15, with items B1–B4, U1–U10 and M1–M5. Each item below was checked against the code at the base above. The original IDs are kept so this file can be read alongside that report.
- **Out-of-file context read:** `apps/lumencraft/src/main.rs` (200–379, 493), `apps/lumencraft/src/gui_window.rs` (440–470, 655–665), `crates/ui-egui/src/interaction.rs` (55–68, 156–157, 1480–1513), `crates/ui-egui/src/control_io.rs` (386–460, 1185, 1251, 1311, 1580), `crates/ui-egui/src/os_integration.rs` (181, 835), `crates/ui-egui/src/lib.rs` (147), `crates/format/src/archive.rs` (178, 203–243).
- **Not run:** no tests and no crash reproductions. Every verdict comes from reading the code.

## Verdict summary

| ID | Original severity | Verdict | Corrected severity | Note |
|---|---|---|---|---|
| B1 | High | **Confirmed** | High | Reachable from a default launch. The audit missed a second effect: the session stays marked clean |
| B2 | Med | **Confirmed** | Med | Mechanism is a snapshot checkpoint, not a WAL append |
| B3 | Med | **Incorrect as stated** | Low (latent) | Branch can't be reached from the shipped app. The proposed fix would destroy crash data. See NEW-1 and NEW-2 |
| B4 | Med | **Confirmed** | Med–High | Also affects trackpad scrolling. The audit's "Ctrl + jitter" sentence is wrong |
| U1 | Low | Correct reading, unreachable | Info | `hint_idx` is a file index |
| U2 | Low | Correct reading, unreachable | Info | Each retry tries a new path |
| U3 | Low | **Invalid** | — | One runtime per process. Nothing indexes by window ID across runtimes |
| U4 | Low | **Invalid** | — | Reusing an ID that was never assigned is correct |
| U5 | Low | **Invalid** | — | `from_physical` doesn't round and doesn't accumulate |
| U6 | Low | Correct reading, speculative | Info | `UiInputEvent` has no `TripleClick` variant |
| U7 | Med | **Confirmed** | Med | Worse than reported: a copied graph pastes SVG markup into cells |
| U8 | Low | **Confirmed, fix needs correction** | Low | "Unavailable" comes back as `Ok`, not `Err` |
| U9 | Med | **Confirmed, impact restated** | Low–Med | Effect is a spurious crash recovery on next launch |
| U10 | Low | **Mostly incorrect** | Info | No WAL entries are written, and the double path is normally unreachable |
| M1 | Low | **Partly incorrect** | Info | `set_locale` can't fail. It normalizes the input |
| M2 | Med | Confirmed, impact overstated | Low | The folder isn't imported. The user gets a confusing error instead |
| M3 | Low | Confirmed, size wrong | Low–Med | About 16.4 MB per redraw, not 6.5 MB, and no caller reads the copy |
| M4 | Low | Confirmed | Info | |
| M5 | Low | Confirmed | Info | The window count is already reported |
| NEW-1 | — | New | High | Default single-instance mode never journals edits |
| NEW-2 | — | New | Med (latent) | Opening a window with a document overwrites any crash journal in its directory |
| NEW-3 | — | New | Low | `--new-instance` journals from crashed processes are never offered for recovery |

---

## Confirmed findings

### B1 · High · Opening several project files keeps only the last one, and merged CSVs leave the session marked clean

- **Location:** `apps/lumencraft/src/desktop_host.rs:407`–`441` (`open_document_from_files`)
- **Category:** data-loss
- **Defect:** Each `LoadedPathPayload::Project` overwrites `primary_doc`, `primary_archive_path` and `clean_archive` (`:409`–`:413`). Any earlier project, and any CSV sheets already merged into it, are dropped. If a CSV follows an archive, its sheets are added to the archive's document but `set_file_path(path, clean_archive = true)` (`:437`–`:439`) still marks the session clean.
- **Failure scenario:** This is reachable from the CLI. Without `--new-instance`, `main.rs:240` → `open_startup_session_with_recovery` → `main.rs:349` `load_startup_session(&cfg.files)` passes every file to this function.
  - `lumencraft a.lumencraft b.pzfx`: only `b` opens, with no message.
  - `lumencraft data.csv a.lumencraft`: the CSV sheet is discarded.
  - `lumencraft a.lumencraft data.csv`: the window shows no dirty marker and closing it doesn't prompt (`has_unsaved_changes` is false), so the imported sheet is lost without warning. A Save writes the merged CSV into `a.lumencraft`.
- **Suggested fix:** Return an error when a second `Project` payload arrives, or open each project in its own window in single-instance mode as `--new-instance` already does (`main.rs:233`–`236`). Pass `clean = false` whenever a delimited table was merged into an archive-backed document. Add tests for all three orderings.

### B2 · Med · Menu-driven UI commands that change the document skip the recovery checkpoint

- **Location:** `apps/lumencraft/src/desktop_host.rs:1439`–`1450` (`handle_menu_action`, `dispatch_ui_command` branch)
- **Category:** data-loss (crash recovery only)
- **Defect:** This branch calls `dispatch_ui_command` and `sync_views()` but never captures undo depth, redo depth and the dirty flag beforehand, and never calls `record_ui_mutation_checkpoint`. The pointer and keyboard path does both (`:1145`–`:1151`).
- **Failure scenario:** Some UI commands change the document even with the `Value::Null` params the menu passes. For example, `ui.grid.excludeWithReason` (`control_io.rs:1311`) falls back to the active sheet and the current selection. After such a menu action, the attached journal holds no snapshot of the change and `dirty_commands` has no label for it. A crash before the next checkpointed edit loses the change. The close prompt still works, because `has_unsaved_changes` also checks `session().is_dirty()` (`:573`).
- **Correction to the original:** For UI-originated changes the window writes a full snapshot checkpoint (`record_ui_mutation_checkpoint` → `checkpoint_recovery_journal`, `:695`–`:698`), not a WAL append. What's missing is that snapshot plus the `dirty_commands` entry.
- **Suggested fix:** Record `undo_depth`, `redo_depth` and `is_dirty` before `dispatch_ui_command`, then call `record_ui_mutation_checkpoint(Some(trimmed), …)` after it, as `dispatch_ui_event_on_window` does. Add a test that runs `MenuAction { command_id: "ui.grid.excludeWithReason" }` on a window with a journal and checks that a checkpoint was written.

### B4 · Med–High · An unmodified vertical wheel or trackpad scroll zooms the canvas instead of panning

- **Location:** `apps/lumencraft/src/desktop_host.rs:1207`–`1211`. Input comes from `apps/lumencraft/src/gui_window.rs:450`–`462`.
- **Category:** wrong-result (input handling)
- **Defect:** A wheel event is treated as a pan only when `delta_x != 0` and neither Ctrl nor Meta is held. Every other event, including a purely vertical scroll with no modifier, becomes `WheelZoom` → `handle_pinch_zoom` (`interaction.rs:1480`–`1485`).
- **Failure scenario:** Scrolling a mouse wheel over the canvas zooms it. `gui_window.rs` forwards every egui `MouseWheel` event to this handler, including macOS two-finger trackpad scrolls (unit `Point`), so a vertical swipe zooms too. A swipe with a slight horizontal drift switches between zoom and pan partway through the gesture.
- **Correction to the original:** Drop the first half of the audit's explanation. With Ctrl or Meta held the code always zooms, which is correct, so horizontal jitter while Ctrl is held does nothing wrong.
- **Suggested fix:** With no modifier, send `InertialPan { delta_x, delta_y, … }` for both axes. With Ctrl or Meta held, send `WheelZoom`. Keep pinch-to-zoom on `TrackpadMagnify`. No test in `apps/lumencraft/tests` sends `MouseWheel`, so add one for each branch.

### U7 · Med · Paste reads the in-app clipboard cache before the OS clipboard, so external copies are ignored and a copied graph pastes as SVG text

- **Location:** `apps/lumencraft/src/desktop_host.rs:1511`–`1515`, with `:1493`, `:1505`
- **Category:** wrong-result
- **Defect:** When there's no `text_override`, paste uses `self.clipboard_text` whenever it's set, and reads the OS clipboard only if the cache is empty. Nothing ever clears or invalidates the cache.
- **Failure scenario:**
  - After any in-app copy, Cmd+V keeps pasting that old text even when the user has since copied something in another app.
  - Copying a graph or layout stores its SVG or HTML as `clipboard_text` (`:1493`, `:1505`). Selecting a data sheet and pressing Cmd+V then sends that markup to `table.paste`.
- **Suggested fix:** Read the OS clipboard first with `read_os_pasteboard()`. It already reports "unavailable" cleanly (`os_integration.rs:835`–`839`). Fall back to the cache only when the OS clipboard is unavailable, and never paste the SVG or HTML string from the graph branch into a table.

### U8 · Low · Failed or unavailable OS clipboard writes are ignored

- **Location:** `apps/lumencraft/src/desktop_host.rs:1485` (`write_os_pasteboard`), `:1492` (`write_os_clipboard_bundle`)
- **Category:** contract-drift
- **Defect:** Both results are discarded with `let _ =`, and the outcome still reports `clipboard.copied_*` as handled.
- **Correction to the original:** Checking for `Err` alone won't help. When there's no clipboard session, `write_os_pasteboard` returns `Ok(OsAdapterStatus::unavailable(..))` (`os_integration.rs:181`–`186`). The fix has to inspect the returned status. The second call site at `:1492` has the same problem.
- **Suggested fix:** Include the adapter status in `DesktopEventOutcome` (for example `command_result: {"os_clipboard": status}`) and set a distinct action string when the write failed or the clipboard was unavailable.

### U9 · Low–Med · `close_all_force` stops at the first journal-close error

- **Location:** `apps/lumencraft/src/desktop_host.rs:978`–`984`, with `:1079`–`1080`
- **Category:** resources
- **Defect:** `?` inside the loop returns on the first failure, and the remaining windows are never closed.
- **Correction to the original:** The failing window is removed from `self.windows` before `close_journal_cleanly` fails, so its journal is dropped without a clean close. The windows still in the loop keep their leases. Those journals do get picked up later: when the leases go stale (5-minute TTL), the next launch treats them as crashes and recovers them unprompted. The audit's claim that they "won't be cleaned up" is wrong. `main.rs:315` propagates the error, and `gui_window.rs:663` ignores it.
- **Suggested fix:** Try to close every window, collect the errors, and return the first one at the end.

### M2 · Low · Any folder is routed to the `.lumencraft.d` importer

- **Location:** `apps/lumencraft/src/desktop_host.rs:313`–`315`
- **Category:** contract-drift
- **Defect:** The trailing `|| path.is_dir()` makes every folder an unpacked-project target. It also makes the `is_unpacked_project_dir` check before it redundant.
- **Correction to the original:** Nothing gets imported. `open_project_unpacked_dir` fails on the missing `manifest.json` (`archive.rs:211`), so dropping a plain folder produces `failed to open '<dir>': …manifest.json…` instead of the "unsupported format" message. This is a misleading error, not wrong behaviour, so Low rather than Med.
- **Suggested fix:** Remove `|| path.is_dir()` so that plain folders get the extension-based "unsupported" error.

### M3 · Low–Med · Every redraw clones the whole framebuffer into a cache nothing reads

- **Location:** `apps/lumencraft/src/desktop_host.rs:748`
- **Category:** resources
- **Correction to the original:** The interactive default viewport is 1280×800 logical at scale 2.0 (`main.rs:211`). That's 2560×1600×4 ≈ **16.4 MB** per clone, not 6.5 MB. No code under `apps/` or `crates/`, tests included, calls `DesktopWindowSession::last_frame()`, so the copy serves no purpose.
- **Suggested fix:** Drop the `last_frame` cache, or store only the metadata. If a cached frame is really needed, keep the bytes behind an `Arc<[u8]>`.

---

## New findings (missed by the original report)

### NEW-1 · High · Default single-instance mode never journals edits, so crash recovery restores only the startup state

- **Location:** `apps/lumencraft/src/main.rs:212`–`216`, `:239`–`244`, `:348`–`350`. Effect shows in `desktop_host.rs:654`–`667` and `:685`–`699`.
- **Category:** data-loss
- **Defect:** Without `--new-instance`, `recovery_root` is `None` (`main.rs:212`), and `main.rs` opens the recovery journal itself and keeps it in a local `single_instance_journal`. The window is opened with `attach_recovery = false` (`main.rs:243`), so `DesktopWindowSession::recovery_journal` is `None`. As a result `execute_and_journal` records no WAL entries and `record_ui_mutation_checkpoint` writes no snapshots. When files are passed on the command line, no journal is opened at all (`main.rs:348`–`350`).
- **Failure scenario:** Launch `lumencraft` with no flags, make edits, and kill the process. On the next launch, `open_dir_with_recovery` finds a stale lease but none of the edits from that session.
- **Why it matters for B3:** This is the real problem behind B3's concern that the first document has no crash protection. It lives in `main.rs`, not in the audited branch.
- **Suggested fix:** Hand the startup journal to the window session instead of holding it in `main.rs`. That needs a `DesktopHostRuntime` entry point that takes a session together with an already-open journal, like `install_window_session`'s `recovery` tuple. Then close it through `close_all_force`. Confirm with a kill-and-relaunch integration test.
- **Note:** Check this against [Known defects](../gaps.md#known-defects) before filing. No match was found when searching for `desktop_host`, `recovery journal` and `single-instance`.

### NEW-2 · Med (latent) · Opening a window with a document overwrites any crash journal already in its recovery directory

- **Location:** `apps/lumencraft/src/desktop_host.rs:899`–`913`, with `:935`–`942`
- **Category:** data-loss
- **Defect:** If the supplied session has any sheets or a file path, `open_window_internal` throws away the session recovered from the journal and runs `session.checkpoint_journal(&mut journal)`, which replaces the recovered snapshot and WAL. `install_window_session` still sets `recovered_from_crash = true` and fills `dirty_commands` with the recovered command IDs, even though those commands aren't in the document now shown.
- **Failure scenario:** This happens whenever `recovery_root` is set and a document-bearing window lands in a directory that holds a crashed journal. For example, a file dropped on a dirty window opens a new window via `open_window_with_session(…, recovery_root.is_some())` at `:1368`. The shipped app sets `recovery_root` only together with `new_instance` and per-PID directory names (`:482`–`483`), so today this needs a reused PID or an embedding or test configuration.
- **Why it matters for B3:** The original report proposes to "always attach when `recovery_root` is `Some`". With `new_instance = false`, window 1 uses the root directory itself (`:484`–`485`), so that fix would overwrite the root's crash journal as soon as the first window opens a file. The `files.is_empty()` exclusion in `:851` looks like a deliberate guard against exactly this.
- **Suggested fix:** When the recovery report shows recovered content, don't overwrite it. Open the document in its own new journal directory, or offer the recovered session in a separate window.

### NEW-3 · Low · Journals from crashed `--new-instance` processes are never offered for recovery

- **Location:** `apps/lumencraft/src/desktop_host.rs:482`–`483`, `:872`–`884`
- **Category:** data-loss (recovery UX)
- **Defect:** `--new-instance` journal directories are named `instance-<pid>-win-<n>`. A new process has a different PID, so it never reopens a crashed process's directory. The only way to recover one is `recover_window_from_dir`, and only tests call it.
- **Suggested fix:** At startup, scan `recovery_root` for directories whose leases have gone stale and offer to recover them through `recover_window_from_dir`.

---

## Items to drop or downgrade, with reasons

- **B3. Incorrect as stated.** The code reading is right: with `recovery_root = Some` and `new_instance = false`, the first window opened with a file gets no journal. But the shipped app never builds that configuration. `main.rs:212` sets `recovery_root` only when `--new-instance` is given, and then `attach_recovery` is always true. The proposed fix would cause NEW-2. Replace B3 with NEW-1 and NEW-2.
- **U1. Info.** `hint_idx` is the index of the file in `files` (`:417`), so `usize::MAX` can't be reached. A bounded loop that returns an error would be tidier but fixes nothing real.
- **U2. Info.** The loop runs only in `--new-instance` mode when the base directory is leased. Each iteration builds a new path that usually doesn't exist, so the first iteration ends the loop. `dir_has_active_lease` returns `false` on any error (`:475`–`477`), which also ends it. Wrapping a `u64` counter isn't a realistic case.
- **U3. Invalid.** The app creates one `DesktopHostRuntime` per process (`main.rs:227`; `gui_window.rs:813` is a test helper). Nothing indexes window IDs across runtimes, and the project has zero telemetry by policy, so the "analytics" case doesn't apply.
- **U4. Invalid.** The ID is used only if the window is installed. Reusing it after a failure is correct, and nothing logs the failed attempt's ID. The same pattern in `open_window_internal` is fine for the same reason.
- **U5. Invalid.** `ViewportMetrics::from_physical` (`interaction.rs:55`–`62`) divides without rounding and is recomputed from the physical size on every event, so no drift can build up.
- **U6. Info.** `UiInputEvent` has no `TripleClick` variant (`interaction.rs:156`). This is a note for whoever adds one later, not a defect.
- **U10. Info.** `checkpoint_recovery_journal` writes a snapshot, not WAL entries. The double checkpoint happens only if a save changes undo or redo depth or flips the dirty flag from false to true, and a save doesn't normally do either. A related point that is worth tracking: every UI-originated change writes a full snapshot (`:698`) instead of appending to the WAL, which costs I/O proportional to document size on large projects.
- **M1. Info.** `UiApp::set_locale` returns `&str` (`lib.rs:147`). It normalizes unknown locales to a fallback and can't fail, so no error is being thrown away. The remaining inconsistency is that an unknown theme is a hard error while an unknown locale falls back silently. `main.rs` always passes `locale_override: None`, so this can't be reached from the shipped CLI.
- **M4, M5. Info.** M4 is accurate; using `to_string_lossy()` would keep a recognisable title. For M5, the outcome already reports `ticked_windows` (`:1605`).

## Suggested fix order

1. NEW-1 and B1 (data loss in the default launch path)
2. B4 and U7 (wrong input behaviour users hit every day)
3. B2, then NEW-2 before anyone changes the recovery-attach logic in B3
4. U9, U8, M3, M2
5. Info items as cleanup
