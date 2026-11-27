# Troubleshooting, error codes & known issues

Use this guide to resolve installer security prompts, missing fonts, GPU startup errors, MCP connection failures, crash recovery warnings, structured error codes, and Beta known issues.

---

## Installation and OS security prompts {#install-issues}

### macOS Gatekeeper and quarantine {#macos-gatekeeper}

- Beta universal `.dmg` releases are signed with an Apple Developer ID and notarized.
- If Gatekeeper warns about a copy moved from unsigned local media, verify its checksum and build attestation ([Verifying downloads](get-started.md#verifying-downloads)) and move **CurveBook.app** into `/Applications`.
- If Terminal reports `command not found` for `curvebook-cli`, add `/Applications/CurveBook.app/Contents/MacOS` to `PATH` ([macOS install steps](get-started.md#install-macos)).

### Windows SmartScreen and Smart App Control {#windows-smartscreen}

- **Defender SmartScreen ("Windows protected your PC"):** Beta Windows `.msi` installers (x64 and ARM64) are currently unsigned. **Verify `SHA256SUMS.txt` and `gh attestation verify` first** ([Verifying downloads](get-started.md#verifying-downloads)), then click **More info** and **Run anyway**.
- **Windows 11 Smart App Control:** In enforcement mode, Smart App Control blocks unsigned installers with no override prompt. Signed Windows installers or a signed Microsoft Store MSIX package will address this after Beta.
- **Duplicate installer scope error:** You cannot mix per-user and machine-wide (`ALLUSERS=1`) installs. Uninstall the existing copy before switching scopes ([Windows install steps](get-started.md#install-windows)).

### Linux AppImage and missing FUSE v2 (`libfuse2`) {#linux-fuse}

If running the `.AppImage` fails with `dlopen(): error loading libfuse.so.2` or `AppImages require FUSE to run`:

1. Extract the AppImage once and run it without FUSE:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/AppRun
```

2. Run the headless CLI directly from `squashfs-root/usr/bin/curvebook-cli` (no FUSE or display server required). Make sure the `.AppImage` is marked executable (`chmod +x`) before extracting.

---

## Fonts and rendering issues {#fonts-and-rendering}

- **Missing CJK, RTL, or mathematical glyphs:** CurveBook bundles no binary fonts and relies on system TrueType and OpenType fonts. On minimal Linux or FreeBSD systems without Noto Sans CJK or DejaVu Sans, install system fonts, set `CURVEBOOK_FONTS_DIR` to a folder of `.ttf` or `.otf` files, or pass `--ttf-font <path>` to `curvebook`.
- **Preflight warnings (`min_font_size`, `hairline_stroke`, `text_outside_canvas`):** Run `graph.preflightCheck` or set the [page-layout resize mode](graphs.md#page-layouts) to `KeepTypographyPt` when shrinking figures for narrow journal columns. This keeps labels at 5.0 pt or larger and strokes at 0.25 pt or thicker.

### GPU and graphics initialization {#gpu-graphics}

The CurveBook desktop window uses native GPU acceleration (Metal only on macOS, DirectX 12, Vulkan, or OpenGL on Windows, and Vulkan or OpenGL on Linux). On older Macs without full Metal support, virtualized desktops, or systems with outdated GPU drivers, startup or window resizing can fail with a graphics initialization error (`RequestAdapterError`, `RequestDeviceError`, `CreateSurfaceError`, `NoSurfaceFormatsAvailable`) or a `GPU device lost` message.

Try these steps in order:

1. **Update your OS or graphics drivers:** On macOS, use a supported macOS release with native Metal acceleration (or enable host 3D acceleration in VM settings). On Windows and Linux, install current Vulkan, DirectX 12, or OpenGL drivers.
2. **Try a platform-supported graphics backend or adapter override from a terminal:**

   - **macOS:** CurveBook compiles `wgpu` with the native Metal backend only (`WGPU_BACKEND=gl` is unavailable on macOS because `wgpu/angle` is not enabled). If your GPU or virtualized adapter is flagged as non-compliant, try:

     ```sh
     WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1 curvebook
     ```

   - **Linux / Windows:** Select an alternative compiled backend (`vulkan`, `gl`, or `dx12` on Windows), or allow non-compliant adapters:

     ```sh
     WGPU_BACKEND=gl curvebook
     ```

     (or `WGPU_BACKEND=vulkan`, `WGPU_BACKEND=dx12` on Windows, or `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1`).

3. **Fall back to CPU-only CLI or MCP mode:** Both `curvebook-cli` and `curvebook-cli mcp` run every statistical test, curve fit, and SVG/PDF/PNG/TIFF export on the CPU without a GPU. *(Note: `curvebook --headless-once` renders a single discarded 800×520 software smoke frame and exits; it is a startup diagnostic rather than a general headless export mode.)*

---

## MCP, CLI, and `--control` automation {#mcp-and-cli}

- **Paths blocked by `--workdir` (`path_escape`):** Every file argument (`path`, `out`, `image_path`, and linked import paths) must stay inside `--workdir` without `..` traversal or external symlinks. Use paths relative to `--workdir`, such as `study.curvebook` or `exports/figure.pdf`.
- **Noisy shell wrappers break MCP:** `curvebook-cli mcp` reserves `stdout` strictly for JSON-RPC 2.0 messages and sends diagnostics to `stderr`. Never launch it through a shell profile that prints banners to `stdout` ([MCP reference](automation/mcp.md)).
- **`--control` binding errors:** `--control <port>` accepts ports `1` through `65535` (port `0` is rejected) and listens only on `127.0.0.1`. Pass the auth token via `CURVEBOOK_CONTROL_TOKEN` rather than on the command line.

### MCP client cannot connect {#mcp-connection}

Check first that your AI client can launch `curvebook-cli`, then verify the JSON-RPC handshake.

1. **Use absolute paths for both `"command"` and `--workdir`.** Point `"command"` to the full path of `curvebook-cli` (such as `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` on macOS). Make sure the `--workdir` folder already exists (`mkdir -p /path/to/your/data-folder`). If the folder is missing or unreadable, the server exits immediately with status `2` and `usage error: --workdir ... is inaccessible`.
2. **Do not rely on `PATH` (fixes `command not found` and `ENOENT`).** Desktop apps launched from the macOS Dock, Finder, or a desktop menu rarely inherit your terminal `PATH`. Always write the full binary path instead of bare `curvebook-cli` or a wrapper like `cargo run`.
3. **Add `"type": "stdio"` if your client requires it.** Some clients ignore server entries that omit an explicit transport:

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

4. **Test the server directly in a terminal.** Run the exact configured command with the same arguments. A bad flag or unreadable `--workdir` prints one line to `stderr` and exits; a healthy server prints nothing and waits for input. Confirm the handshake with:

   ```sh
   printf '%s\n' \
     '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
     '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
     '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
     | /absolute/path/to/curvebook-cli mcp --workdir /path/to/workspace
   ```

   Expect two JSON lines: an `initialize` result (`serverInfo.name` is `"curvebook"`) and a `tools/list` result. If that succeeds, check your client's config file, protocol logs, and whether the client needs a full restart.
5. **Keep `stdout` clean.** Wrappers must send startup banners and logs to `stderr`. Prefer running the compiled binary directly.
6. **Source build fails on older Rust versions.** Check `rustc --version` and `cargo --version` inside the repository; both must report `1.99.1` or newer ([development prerequisites](../development.md#prerequisites)).

Common errors after connecting:

| Symptom | Cause | Fix |
|---|---|---|
| `code: "path_escape"` (`policy`) | Requested path leaves `--workdir` | Move the file inside `--workdir` or widen `--workdir` |
| `code: "conflict"` on a `ui_*` tool | Headless mode has no desktop window | Attach with `--control-port` |
| `code: "io_error"` (`failed to connect to desktop control port`) | No desktop control listener on that port | Start `curvebook --control <port>` first, or omit `--control-port` |
| `code: "bad_input"` (`invalid or missing control authentication token`) | Desktop rejected the control token | Pass the running instance's token via `CURVEBOOK_CONTROL_TOKEN` or token file |
| `usage error: invalid --control-port` | Port out of range | Choose an integer from `1` to `65535` |

Connection notes:

- In headless mode, the MCP server starts even when `--control-port` points to an inactive port; the `io_error` appears when you call a tool. See [attachment modes](automation/mcp.md#attachment-modes-and-filesystem-policy) for setup details.
- Never disable control authentication. Match the per-user token described in the [control protocol reference](automation/control-and-batch.md#loopback-desktop-control-protocol---control); the server maps raw `unauthorized` control responses to MCP `bad_input`.

---

## Crash recovery and file loading {#recovery-and-files}

- **Recovery journal warnings:** After an unexpected exit, CurveBook replays `snapshot.curvebook` and valid lines of `ops.journal` up to the last intact CRC-32 record. Any incomplete final line is skipped with a recovery warning.
- **Multiple windows and lock files:** Active `curvebook.lock` leases automatically give each new window its own recovery folder, and stale leases from dead local processes are reclaimed safely. Set `CURVEBOOK_NO_PREFS=1` to launch cleanly without reading or modifying preferences or recovery state.
- **`unsupported_version` on open or save:** A file was saved by a newer CurveBook schema version. CurveBook refuses to overwrite newer files to protect your data; open the file with the newer release, or save to a different path.
- **`corrupt_data` or CRC mismatch:** The `.curvebook` ZIP archive is truncated, or someone edited `.curvebook.d/document.json` manually without updating `manifest.json` checksums.

---

## Structured error codes {#error-codes}

Every validation, format, policy, solver, and I/O failure maps to one of these canonical `snake_case` error codes:

| Code | Category | Typical cause |
|---|---|---|
| `bad_input` | Input | Invalid argument, syntax error, or out-of-range option |
| `not_found` | State | Unknown sheet ID, checkpoint, column index, or equation ID |
| `already_exists` | State | Duplicate sheet ID or checkpoint without `overwrite: true` |
| `conflict` | State | Incompatible state, policy refusal, or duplicate installer scope |
| `frozen_sheet` | State | Edit attempted on a locked data sheet |
| `cycle_detected` | State | Circular reference in formulas, hooks, or linked regions |
| `unsupported_format` | Format | Unrecognized file header or unsupported container format |
| `unsupported_version` | Format | File, preferences, or plate-map version is newer than this release |
| `corrupt_data` | Format | Malformed JSON/XML/ZIP data or failed CRC-32 check |
| `io_error` | IO | Missing file, permission denied, or filesystem failure |
| `path_escape` | Policy | Path traversal (`..`), external absolute path, or escaping symlink |
| `resource_limit` | Policy | Input, export, or formula exceeded a safety budget |
| `solver_failed` | Compute | Non-convergence, zero variance, singular matrix, or invalid weights |
| `insufficient_data` | Compute | Empty input, too few observations for `df`, or missing raw replicates |
| `cancelled` | Compute | Operation cancelled via MCP or engine token |
| `internal_panic` | Internal | Unexpected internal fault caught cleanly without crashing |
| `internal_error` | Internal | Unclassified internal dispatch or serialization error |

Error details and limits:

- **Unlocking sheets (`frozen_sheet`):** Run `sheet.unfreeze` before modifying a locked data sheet.
- **Resource limits (`resource_limit`):** Safety caps prevent runaway memory or disk use. Project archives are capped at 128 MiB, CSV files at 64 MiB, and sheets at 30 million cells. Raster exports cap at 16,384 px per edge (67 million pixels total), PDF exports at 64 pages or 32 MiB, and formulas use bounded step limits.
- **Missing replicates (`insufficient_data`):** Some tests require raw replicate values and cannot run when only summary columns (mean, SD/SEM, and N) are entered.
- **Version limits (`unsupported_version`):** CurveBook supports project archives up to schema version 9, plate-map files up to version 1, and preferences files up to version 1.
- **Diagnostics-only policy (`conflict`):** Assumption banners report diagnostics only and reject automatic remediation requests.

---

## Known issues and Beta scope {#known-issues}

- **Supported interfaces and verification:** The native desktop GUI (`curvebook`) is supported alongside `curvebook-cli` and `curvebook-cli mcp`. Automated desktop interface suites verify widget, menu, dialog, and canvas workflows for Beta. Recorded manual cross-OS walkthroughs are part of 1.0 release verification. FreeBSD (x86_64) is a **Tier 2 CLI-only** package with no desktop GUI.
- **Windows packages and signing:** Beta omits 32-bit Windows (x86) and portable `.zip` archives. Windows x64 and ARM64 `.msi` installers ship **unsigned** during Beta. Verify checksums and build attestations before clicking **More info** and **Run anyway** in SmartScreen. Windows 11 Smart App Control in enforcement mode blocks unsigned installers until Windows code signing or Microsoft Store MSIX distribution arrives. Uninstall any Alpha 32-bit copy manually before installing the 64-bit `.msi`.
- **Draft translations:** Non-English Beta UI languages (German, Spanish, French, Italian, Dutch, Polish, Portuguese, Russian, Turkish, Japanese, Korean, and Simplified/Traditional Chinese) display a `[Draft]` label in **View › Language** until native-speaker scientific review finishes.
- **Right-to-left GUI locales:** Full right-to-left desktop layout mirroring (Arabic, Hebrew, Persian, and Urdu) is unscheduled and unlikely. CurveBook hides RTL locales in the language picker and falls back to English if an existing preferences file requests one. Mathematical axes and formulas stay left-to-right.
- **Browser app deferred to 1.0:** The browser WebAssembly app (`curvebook-web`) is deferred until CurveBook 1.0 and is not shipped in Beta releases.
- **Zero bundled binary fonts:** CJK, RTL, and special scientific symbols rely on system TrueType and OpenType fonts or fonts supplied via `CURVEBOOK_FONTS_DIR` or `--ttf-font` ([Fonts](get-started.md#fonts)).
- **Legacy archive rejection:** Early pre-CurveBook archive files are rejected with `unsupported_format`. CurveBook opens current `.curvebook` and `.curvebook.d/` projects, `.cbtemplate` templates, `.cbplate` plate maps, and `.csv`, `.tsv`, `.xlsx`, `.pzfx`, and `.prism` files.
- **Deferred formula and calculator features:** Calculated Variables support numeric math, boolean and 3-valued logic, conditional aggregates, and window functions; text-manipulation and date-arithmetic functions are omitted in Beta. Extended browser collaboration tools, additional unpacked-directory conveniences, and standalone statistical or wet-lab bench calculators are deferred beyond Beta.
- **Release notes:** Check [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) for release-specific notes, checksums, and build attestations.
