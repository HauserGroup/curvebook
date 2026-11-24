# Troubleshooting, Error Codes & Known Issues

Use this reference to diagnose installation warnings, font discovery issues, MCP/CLI confinement errors, WAL crash recovery messages, structured `ErrorCode` diagnostics, and Beta known issues.

---

## Installation & OS Security Prompts {#install-issues}

### macOS Gatekeeper / Quarantine {#macos-gatekeeper}

- Beta universal `.dmg` releases are Developer ID signed and Apple-notarized (`R8.10`). If Gatekeeper flags a file copied across unsigned local media, confirm checksum and provenance with [Get Started: Verifying Checksums & Build Attestations](get-started.md#verifying-downloads), and ensure `CurveBook.app` lives inside `/Applications` with clean extended attributes.
- If `curvebook-cli` reports `command not found` in Terminal, verify `/Applications/CurveBook.app/Contents/MacOS` is on `PATH` ([Get Started: macOS](get-started.md#install-macos)).

### Windows SmartScreen & Smart App Control {#windows-smartscreen}

- **Defender SmartScreen ("Windows protected your PC"):** Beta Windows `.msi` installers (`x64`, `arm64`) are **unsigned** until `R8.11`. After verifying `SHA256SUMS.txt` and `gh attestation verify` against `HauserGroup/curvebook` ([Get Started: Verification](get-started.md#verifying-downloads)), select **More info** -> **Run anyway**.
- **Windows 11 Smart App Control (Enforcement Mode):** Smart App Control blocks unsigned installers and executables automatically without a bypass prompt; signed Windows releases (`R8.11`) or a signed Microsoft Store MSIX distribution (`R8.9`) will address this after Beta.
- **Cross-context duplicate MSI error:** Upgrading or switching between per-user (`Programs\CurveBook`) and machine-wide (`ALLUSERS=1`) installs requires uninstalling the opposite-scope installation first ([Windows installation policy](get-started.md#install-windows)).

### Linux AppImage & Missing FUSE v2 (`libfuse2`) {#linux-fuse}

If running `.AppImage` fails with `dlopen(): error loading libfuse.so.2` or `AppImages require FUSE to run`:

1. Extract the AppImage without FUSE and execute directly:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/AppRun
```

2. Headless CLI/MCP binaries sit inside `squashfs-root/usr/bin/curvebook-cli` and require no FUSE or display server once extracted. Ensure the AppImage has executable permissions (`chmod +x`).

---

## Fonts & Rendering Issues {#fonts-and-rendering}

- **Missing CJK / RTL / Special Mathematical Glyphs:** Because CurveBook ships zero bundled font binaries in the repository, it relies on host OS TrueType/OpenType system fonts. On minimal Linux/FreeBSD desktops or headless servers lacking `NotoSansCJK` or `DejaVuSans`, install system fonts or point `CURVEBOOK_FONTS_DIR` to a folder of `.ttf`/`.otf` faces (or pass `--ttf-font <path>` to `curvebook`).
- **Preflight warnings (`min_font_size`, `hairline_stroke`, `text_outside_canvas`):** Run `graph.preflightCheck` or switch [`CanvasResizeMode`](graphs.md#page-layouts) to `KeepTypographyPt` when shrinking canvases for narrow journal columns so labels stay $\ge 5.0\text{ pt}$ and strokes stay $\ge 0.25\text{ pt}$.

---

## MCP, CLI & `--control` Automation {#mcp-and-cli}

- **`path_escape` under `--workdir`:** All path parameters (`path`, `out`, `image_path`, stored `ImportLinkSpec` targets) must resolve strictly inside `--workdir` without `..` traversal or external symlinks. Pass paths relative to `--workdir` (e.g., `"study.curvebook"`, `"exports/figure.pdf"`).
- **MCP client connection failures:** `curvebook-cli mcp` reserves `stdout` strictly for JSON-RPC 2.0 frames and prints human diagnostics to `stderr`. Never wrap `curvebook-cli mcp` in shell profiles that print banners to `stdout`. See [Automation: MCP](automation/mcp.md).
- **`--control` binding errors:** `--control <port>` accepts `1..=65535` (port `0` is rejected) and binds strictly to `127.0.0.1`. Set `CURVEBOOK_CONTROL_TOKEN` in the environment instead of `--control-token` CLI flags.
- **`gui_only_violation`:** When `--gui-only` is active, direct engine tools (`analyze.run`, `table.setCell`, `project.save`, etc.) are blocked by policy; drive interactions via `ui.*` / GUI action primitives.

### MCP client cannot connect {#mcp-connection}

Start by checking whether the MCP client can launch the configured server process, then check the protocol handshake.

1. **Use absolute paths for the binary and for `--workdir`.** Set `"command"` to the full path of `curvebook-cli` (for example `./target/release/curvebook-cli` after running `cargo build --release -p curvebook-cli`, or `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` on macOS) and pass `--workdir` as a full path to a directory that already exists (`mkdir -p /path/to/your/data-folder`). Relative paths resolve against whatever working directory the client happens to start the process in, which is usually not your project folder. A missing or unreadable `--workdir` makes the server exit before the handshake with `usage error: --workdir ... is inaccessible` (exit status `2`).
2. **Do not rely on `PATH` (fixes `command not found` / `ENOENT`).** Applications started from the macOS Dock or Finder, or from a desktop launcher on Linux or Windows, do not inherit the `PATH` of your interactive shell, so a bare `curvebook-cli` that works in a terminal can fail to start from the client. Use the full path (for example the output of `command -v curvebook-cli`, or the absolute path to `target/release/curvebook-cli` from a checkout build). The same applies to wrappers such as `cargo run`; prefer the built binary.
3. **Add `"type": "stdio"` if your client asks for it.** Some clients require an explicit transport on each server entry and ignore or reject entries without one:

   ```json
   {
     "mcpServers": {
       "curvebook": {
         "type": "stdio",
         "command": "/absolute/path/to/curvebook-cli",
         "args": ["mcp", "--workdir", "/path/to/workspace"]
       }
     }
   }
   ```

4. **"No tools available" or a missing server usually means the process did not start.** Run the exact configured command by hand in a terminal, with the same arguments, and read its standard error. A usage or `--workdir` error prints one line to stderr and exits non-zero; a healthy server prints nothing and waits for input. Then confirm the handshake by sending the requests below (output abbreviated):

   ```sh
   printf '%s\n' \
     '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
     '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
     '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
     | /absolute/path/to/curvebook-cli mcp --workdir /path/to/workspace
   ```

   Expect two JSON lines: the `initialize` result (`serverInfo.name` is `"curvebook"`) and a `tools/list` result containing the tool list. These responses confirm that the configured binary starts and handles basic discovery. Next check the client configuration, its protocol logs, and whether it needs restarting after a configuration change.
5. **Keep stdout clean.** Stdout carries the protocol. Wrappers must send banners and diagnostics to stderr. Normal Cargo progress messages already use stderr; `cargo run -q` reduces that noise but cannot repair a wrapper that writes to stdout. Prefer the built binary for a stable launch command.
6. **Source build fails on older Rust versions.** Run `rustc --version` and `cargo --version` inside the repository to verify both report `1.99.1` or newer ([development prerequisites](../development.md#prerequisites)).

Errors that appear after a successful connection:

| Symptom | Cause and fix |
|---|---|
| Tool result with `code: "path_escape"` (`category: "policy"`) | `--workdir` is set and the requested path is outside it (absolute path elsewhere, `..`, or a symlink out). Move the file inside the workdir or widen `--workdir`. |
| Tool result with `code: "gui_only_violation"` | The server was started with `--gui-only`, which exposes only the `gui` tools (`ui_*`); every other tool, and `resources/read`, is rejected. Remove `--gui-only` for engine, file, and export tools ([strict `--gui-only` policy](automation/mcp.md#gui-only)). |
| Tool result with `code: "conflict"` when calling a `ui_*` tool | Default headless mode has no live window. Start with `--gui-only`, or attach with `--control-port` ([attachment modes](automation/mcp.md#attachment-modes-and-filesystem-policy)). |
| Tool result with `code: "io_error"` and `failed to connect to desktop control port` | `--control-port` (or `CURVEBOOK_CONTROL_PORT`) is set but no `curvebook --control <port>` instance is listening on `127.0.0.1:<port>`. Start the desktop side first, or drop the flag. The server itself still starts; the error appears on each tool call. |
| Tool result with `code: "bad_input"` and `invalid or missing control authentication token` | The attached desktop rejected the control authentication token (the raw control error `unauthorized` is mapped to MCP `bad_input`). Check `--control-token`, `CURVEBOOK_CONTROL_TOKEN`, and the per-user token file described in the [control protocol reference](automation/control-and-batch.md#loopback-desktop-control-protocol---control). Use the token for the running desktop instance; do not disable authentication. |
| `usage error: invalid --control-port` / `invalid CURVEBOOK_CONTROL_PORT` | The port must be an integer in `1..=65535`. |

---

## Crash Recovery & File Loading {#recovery-and-files}

- **Recovery journal replay warnings:** On launch after an unexpected crash, CurveBook replays `snapshot.curvebook` + valid lines of `ops.journal` cleanly up to the last intact CRC-32 record (`RecoveryReport`), logging `JournalRecoveryWarning` diagnostics for any torn final line.
- **Parallel desktop instances & `curvebook.lock`:** Active locks isolate additional windows into fresh per-instance recovery directories automatically; expired leases from dead local processes are reclaimed safely (`is_dead_same_host_owner`). Set `CURVEBOOK_NO_PREFS=1` to launch cleanly without touching preferences or recovery state.
- **`unsupported_version` on open or save:** Occurs when opening or targeting a file written with `schema_version > CURRENT_SCHEMA_VERSION` (`project.save` / `save_project_unpacked_dir` refuses to overwrite newer files to prevent data loss). Open and edit with the newer release, or save to a new destination path.
- **`corrupt_data` / CRC mismatch:** Indicates truncated ZIP bytes or manual edits to `.curvebook.d/document.json` without updating `manifest.json` CRCs.

---

## Structured Error Codes (`ErrorCode`) {#error-codes}

Every validation, format, policy, solver, and system error maps deterministically to one of the canonical `ErrorCode` strings (`crates/doc/src/result.rs`):

| Canonical Code (`snake_case`) | Category | Typical Cause & Resolution |
|---|---|---|
| `bad_input` | `Input` | Invalid command argument, negative count, malformed formula/DSL syntax, out-of-range coordinate, or invalid parameter/confidence bounds. |
| `not_found` | `State` | Unknown `sheet_id`, checkpoint name/ID, column index, or built-in `equation_id`. |
| `already_exists` | `State` | Duplicate `sheet_id` or checkpoint identifier created without `overwrite: true`. |
| `conflict` | `State` | Incompatible state or policy transition (e.g., deprecated remediation requests blocked by `QOL-7` `diagnostics_only_policy`, or cross-scope installer clash). |
| `frozen_sheet` | `State` | Mutation attempted on a `DataSheet` locked with `frozen == true` (`sheet.unfreeze` first). |
| `cycle_detected` | `State` | Circular dependency in `=` Calculated Variables, `HookSource` parameters, or `LinkedDataRegion`s. |
| `unsupported_format` | `Format` | Unrecognized file header, unsupported archive kind, or incompatible legacy container. |
| `unsupported_version` | `Format` | File/preferences schema version exceeds this release's supported maximum (`CURRENT_SCHEMA_VERSION = 7`, `CURRENT_PREFERENCES_VERSION = 1`). |
| `corrupt_data` | `Format` | Malformed UTF-8/JSON/XML/ZIP structure or CRC-32 mismatch in archives or WAL journals. |
| `io_error` | `Io` | OS file read/write/rename failure, missing input file, or permission denied. |
| `path_escape` | `Policy` | Path traversal (`..`), absolute external path, or escaping symlink rejected under `--workdir` or ZIP extraction. |
| `resource_limit` | `Policy` | Exceeded `MAX_ARCHIVE_FILE_BYTES` (`128 MiB`), `MAX_CSV_FILE_BYTES` (`64 MiB`), `MAX_SHEET_CELLS` (`30M` / `30,000,000`), `MAX_RASTER_DIM_PX` (`16,384`), `MAX_RASTER_PIXELS` (`67,108,864`), `MAX_PDF_PAGES` (`64`), `MAX_PDF_OUTPUT_BYTES` (`32 MiB`), or DSL/ODE step budgets. |
| `solver_failed` | `Compute` | Non-convergence (`HitMaxIters`), `ZeroVariance`, singular design/covariance matrix, unbracketed implicit root, ODE divergence, or `WeightingScheme` zero denominator (`ImpossibleWeights`). |
| `insufficient_data` | `Compute` | Empty input, too few non-excluded observations/events for `df`, or missing raw replicates (`RequiresRawReplicates`) when `MeanSdN`/`MeanSemN` subcolumns are supplied. |
| `cancelled` | `Compute` | Operation aborted via MCP/engine cancellation token. |
| `gui_only_violation` | `Policy` | Direct non-GUI mutation or filesystem bypass attempted while `--gui-only` policy is active. |
| `internal_panic` | `Internal` | Unexpected invariant fault trapped cleanly at the `guard.rs` safety boundary without aborting the host process. |
| `internal_error` | `Internal` | Unclassified internal dispatch or serialization error. |

---

## Known Issues & Beta Scope {#known-issues}

- **Supported Interfaces & Verification Boundary:** The native `egui` desktop GUI (`curvebook`) is a supported CurveBook interface alongside `curvebook-cli` and `curvebook-cli mcp` (the Alpha *"GUI is pre-alpha"* warning no longer applies). Automated `R15` and `R13` `EguiHumanPathDriver` / `egui_kittest` desktop suites verify interactive widget, menu, dialog, and canvas parity across the automated desktop workflows (`R10.25` Beta gate), while recorded interactive OS walkthroughs (`R10.13` macOS/browser walkthroughs and `R10.24` cross-OS Windows/Linux smoke QA) are **1.0** release-gate verification. The FreeBSD `x86_64` package (`curvebook-<version>-freebsd-x86_64.tar.gz`) is a **Tier 2 CLI-only** archive (`curvebook-cli` and local MCP; no desktop GUI).
- **Windows `x86` Dropped & Unsigned Beta `.msi` Installers:** Beta omits 32-bit Windows (`x86`) and `-portable.zip` archives (`R0.28`). `curvebook-<version>-windows-x64.msi` and `curvebook-<version>-windows-arm64.msi` are currently **unsigned** until `R8.11` (Windows Defender SmartScreen prompts **More info** -> **Run anyway**, and Windows 11 Smart App Control in enforcement mode blocks unsigned installers until Authenticode signing or Microsoft Store MSIX distribution under `R8.9` arrives). Direct `x64` `.msi` upgrade over an Alpha `x86` `.msi` has not yet been runner-verified; manually uninstall any Alpha `x86` copy before installing `x64`.
- **Draft Non-English Translations (`[Draft]`):** All Beta-active non-English UI locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`, `it`, `nl`, `pl`, `ru`, `tr`) carry `[Draft]` suffix labels in **View › Language** (`BETA_ACTIVE_LOCALES`) pending native-speaker scientific review (`R10.23`).
- **Right-to-Left (`ar`, `fa`, `he`, `ur`) GUI Locales Deferred to `1.0`:** While the underlying localization catalog (`LOCALES`) and text-shaping modules recognize `ar` and `he`, full right-to-left desktop GUI mirroring (`ar`, `fa`, `he`, `ur`, and legacy `iw`) is deferred to CurveBook **`1.0`**. In Beta desktop builds, the GUI language picker exposes only active LTR/CJK locales (`BETA_ACTIVE_LOCALES`: `en` plus `[Draft]` translations) and hides RTL options; any existing `preferences.json` specifying an RTL locale falls back cleanly to `"en"` on startup (`normalize_beta_gui_locale`) while mathematical coordinate axes and formulas stay left-to-right.
- **Browser App (`curvebook-web`) Deferred to `1.0`:** The browser WebAssembly bundle (`curvebook-web`) is deferred until CurveBook `1.0` (`R0.28`, `R11`) and is not published in Beta releases.
- **Zero Bundled Binary Fonts:** International CJK, RTL, and special technical glyph rendering depends on OS system TrueType/OpenType fonts or user-provided `.ttf`/`.otf` faces via `CURVEBOOK_FONTS_DIR` / `--ttf-font` ([Get Started: Fonts](get-started.md#fonts)).
- **Pre-`R0.20` Legacy Archive Rejection:** Pre-`R0.20` containers stamped with the legacy `lumencraft-archive` magic are rejected with `unsupported_format`; only current `.curvebook` / `.curvebook.d/` containers (`schema_version <= CURRENT_SCHEMA_VERSION`, currently `7`) and documented import formats (`.csv`, `.tsv`, `.pzfx`, `.prism`) are supported.
- **Formula Evaluator & `1.0`-Deferred Capabilities:** `=` Calculated Variables support numeric, boolean/3-valued logic, conditional aggregates, and window functions, and omit text-manipulation and date-arithmetic functions in Beta. Post-Beta `1.0` scope (`QOL-10` browser collaborator viewer extensions, `QOL-11` additional unpacked-directory QoL extensions, `QOL-13` instant statistical/Bayesian calculators, and `QOL-15` bench molarity/dilution/radiochemical calculators) is deferred beyond Beta.
- **Release Notes & Downloads:** Inspect [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) for per-tag Beta release notes, checksums, and build provenance attestations.
