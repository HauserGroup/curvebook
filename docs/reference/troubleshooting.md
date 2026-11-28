# Troubleshooting, error codes, and known issues

<!-- Audience: Tier 2 Technical Reference — OS security workarounds, GPU/MCP diagnostics, error code tables, and Beta known issues belong here; keep get-started.md and faq.html concise for scientists. -->

This page explains how to get past installer security prompts, missing fonts, GPU startup errors, MCP connection failures, and crash-recovery warnings. It also lists the structured error codes and the known issues in the Beta release. For a standard installation or a first-time MCP setup, start with [Install CurveBook](get-started.md) or [MCP setup for beginners](../mcp-for-beginners.md).

---

## Installation and OS security prompts {#install-issues}

### macOS Gatekeeper and quarantine {#macos-gatekeeper}

- Beta universal `.dmg` releases are signed with an Apple Developer ID and notarized.
- If Gatekeeper warns about a copy that you moved from unsigned local media, first verify its checksum and build attestation ([Verifying downloads](get-started.md#verifying-downloads)). Then move **CurveBook.app** into `/Applications`.
- If Terminal reports `command not found` for `curvebook-cli`, add `/Applications/CurveBook.app/Contents/MacOS` to your `PATH` ([macOS install steps](get-started.md#install-macos)).

### Windows SmartScreen and Smart App Control {#windows-smartscreen}

- Defender SmartScreen ("Windows protected your PC"): The Beta Windows `.msi` installers (x64 and ARM64) are currently unsigned. Before you continue, verify the installer against `SHA256SUMS.txt` and with `gh attestation verify` ([Verifying downloads](get-started.md#verifying-downloads)). Then click **More info** and **Run anyway**.
- Windows 11 Smart App Control: In enforcement mode, Smart App Control blocks unsigned installers and offers no override. Signed Windows installers or a signed Microsoft Store MSIX package will address this after Beta.
- Duplicate installer scope error: You cannot mix a per-user install with a machine-wide (`ALLUSERS=1`) install. Uninstall the existing copy before you switch scopes ([Windows install steps](get-started.md#install-windows)).

### Linux AppImage and missing FUSE v2 (`libfuse2`) {#linux-fuse}

Running the `.AppImage` can fail with `dlopen(): error loading libfuse.so.2` or `AppImages require FUSE to run`. First make sure the `.AppImage` is marked executable (`chmod +x`). Then use one of these workarounds:

1. Extract the AppImage once and run the extracted copy, which does not need FUSE:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/AppRun
```

2. Run the headless CLI directly from `squashfs-root/usr/bin/curvebook-cli`. The CLI needs neither FUSE nor a display server.

---

## Fonts and rendering issues {#fonts-and-rendering}

- Missing CJK, RTL, or mathematical glyphs: CurveBook bundles no binary fonts and uses the TrueType and OpenType fonts installed on your system. On minimal Linux or FreeBSD systems without Noto Sans CJK or DejaVu Sans, install system fonts. Alternatively, set `CURVEBOOK_FONTS_DIR` to a folder of `.ttf` or `.otf` files, or pass `--ttf-font <path>` to `curvebook`.
- Preflight warnings (`min_font_size`, `hairline_stroke`, `text_outside_canvas`): When you shrink figures for narrow journal columns, run `graph.preflightCheck`, or set the [page-layout resize mode](graphs.md#page-layouts) to `KeepTypographyPt`. That mode keeps labels at 5.0 pt or larger and strokes at 0.25 pt or thicker.

### GPU and graphics initialization {#gpu-graphics}

The CurveBook desktop window uses native GPU acceleration. It uses Metal only on macOS; DirectX 12, Vulkan, or OpenGL on Windows; and Vulkan or OpenGL on Linux. Older Macs without full Metal support, virtualized desktops, and outdated GPU drivers can cause errors at startup or when you resize the window. Typical messages are `RequestAdapterError`, `RequestDeviceError`, `CreateSurfaceError`, `NoSurfaceFormatsAvailable`, and `GPU device lost`.

Try these steps in order:

1. Update your operating system or graphics drivers. On macOS, use a supported macOS release with native Metal acceleration; in a virtual machine, enable host 3D acceleration in the VM settings. On Windows and Linux, install current Vulkan, DirectX 12, or OpenGL drivers.
2. From a terminal, try another graphics backend that your platform supports, or override the adapter check:

   - macOS: CurveBook builds `wgpu` with the native Metal backend only. `WGPU_BACKEND=gl` is therefore unavailable on macOS, because `wgpu/angle` is not enabled. If your GPU or virtualized adapter is flagged as non-compliant, try:

     ```sh
     WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1 curvebook
     ```

   - Linux and Windows: Select another compiled backend (`vulkan` or `gl`, or `dx12` on Windows), or allow non-compliant adapters:

     ```sh
     WGPU_BACKEND=gl curvebook
     ```

     You can also use `WGPU_BACKEND=vulkan`, `WGPU_BACKEND=dx12` on Windows, or `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1`.

3. Fall back to the CPU-only CLI or MCP mode. Both `curvebook-cli` and `curvebook-cli mcp` run every statistical test, curve fit, and SVG/PDF/PNG/TIFF export on the CPU, without a GPU.

Note that `curvebook --headless-once` is a startup diagnostic, not a general headless export mode. It renders a single 800×520 software smoke-test frame, discards it, and exits.

---

## MCP, CLI, and `--control` automation {#mcp-and-cli}

- Paths blocked by `--workdir` (`path_escape`): Every file argument (`path`, `out`, `image_path`, and linked import paths) must stay inside `--workdir`. Arguments may not use `..` traversal or follow symlinks to locations outside it. Use paths relative to `--workdir`, such as `study.curvebook` or `exports/figure.pdf`.
- Noisy shell wrappers break MCP: `curvebook-cli mcp` reserves `stdout` strictly for JSON-RPC 2.0 messages and writes diagnostics to `stderr`. Never launch it through a shell profile that prints banners to `stdout` ([MCP reference](automation/mcp.md)).
- `--control` binding errors: `--control <port>` accepts ports `1` through `65535` and rejects port `0`. It listens only on `127.0.0.1`. Pass the authentication token in `CURVEBOOK_CONTROL_TOKEN` rather than on the command line.

### MCP client cannot connect {#mcp-connection}

First check that your AI client can launch `curvebook-cli`. Then verify the JSON-RPC handshake.

1. Use absolute paths for both `"command"` and `--workdir`. Set `"command"` to the full path of `curvebook-cli`, such as `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` on macOS. Make sure the `--workdir` folder already exists (`mkdir -p /path/to/your/data-folder`). If the folder is missing or unreadable, the server exits immediately with status `2` and prints `usage error: --workdir ... is inaccessible`.
2. Do not rely on `PATH`. This fixes `command not found` and `ENOENT` errors. Desktop apps launched from the macOS Dock, the Finder, or a desktop menu rarely inherit the `PATH` from your terminal. Always write the full binary path rather than a bare `curvebook-cli` or a wrapper such as `cargo run`.
3. Add `"type": "stdio"` if your client requires it. Some clients ignore server entries that do not name a transport explicitly:

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

4. Test the server directly in a terminal. Run the exact command from your configuration with the same arguments. If a flag is wrong or `--workdir` is unreadable, the server prints one line to `stderr` and exits. A healthy server prints nothing and waits for input. To confirm the handshake, run:

   ```sh
   printf '%s\n' \
     '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
     '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
     '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
     | /absolute/path/to/curvebook-cli mcp --workdir /path/to/workspace
   ```

   You should see two JSON lines: an `initialize` result, in which `serverInfo.name` is `"curvebook"`, and a `tools/list` result. If this works, check your client's configuration file and protocol logs, and whether the client needs a full restart.
5. Keep `stdout` clean. Any wrapper script must send startup banners and logs to `stderr`. Where possible, run the compiled binary directly.
6. A source build fails on older Rust versions. Inside the repository, check `rustc --version` and `cargo --version`. Both must report `1.99.1` or newer ([development prerequisites](../development.md#prerequisites)).

Common errors after the client connects:

| Symptom | Cause | Fix |
|---|---|---|
| `code: "path_escape"` (`policy`) | The requested path is outside `--workdir` | Move the file inside `--workdir`, or choose a wider `--workdir` |
| `code: "conflict"` on a `ui_*` tool | Headless mode has no desktop window | Attach to a desktop window with `--control-port` |
| `code: "io_error"` (`failed to connect to desktop control port`) | No desktop control listener is running on that port | Start `curvebook --control <port>` first, or omit `--control-port` |
| `code: "bad_input"` (`invalid or missing control authentication token`) | The desktop app rejected the control token | Pass the running instance's token in `CURVEBOOK_CONTROL_TOKEN` or the token file |
| `usage error: invalid --control-port` | The port is out of range | Choose an integer from `1` to `65535` |

Connection notes:

- In headless mode, the MCP server starts even when `--control-port` points to a port with no listener. The `io_error` appears only when you call a tool. See [attachment modes](automation/mcp.md#attachment-modes-and-filesystem-policy) for setup details.
- Never disable control authentication. Use the per-user token described in the [control protocol reference](automation/control-and-batch.md#loopback-desktop-control-protocol---control). The MCP server reports a raw `unauthorized` response from the control channel as the MCP error `bad_input`.

---

## Crash recovery and file loading {#recovery-and-files}

- Recovery journal warnings: After an unexpected exit, CurveBook replays `snapshot.curvebook` and the valid lines of `ops.journal`, up to the last intact CRC-32 record. If the final line is incomplete, CurveBook skips it and shows a recovery warning.
- Multiple windows and lock files: When a `curvebook.lock` lease is already active, CurveBook automatically gives each new window its own recovery folder. It safely reclaims stale leases left by local processes that are no longer running. To start CurveBook without reading or modifying preferences or recovery state, set `CURVEBOOK_NO_PREFS=1`.
- `unsupported_version` on open or save: A newer CurveBook schema version saved the file. To protect your data, CurveBook refuses to overwrite newer files. Open the file with the newer release, or save to a different path.
- `corrupt_data` or a CRC mismatch: Either the `.curvebook` ZIP archive is truncated, or someone edited `.curvebook.d/document.json` by hand without updating the checksums in `manifest.json`.

---

## Structured error codes {#error-codes}

Every validation, format, policy, solver, and I/O failure reports one of these `snake_case` error codes:

| Code | Category | Typical cause |
|---|---|---|
| `bad_input` | Input | Invalid argument, syntax error, or out-of-range option |
| `not_found` | State | Unknown sheet ID, checkpoint, column index, or equation ID |
| `already_exists` | State | Duplicate sheet ID or checkpoint without `overwrite: true` |
| `conflict` | State | The command is disabled in the current session, the request conflicts with the current state (for example, a result sheet that is no longer linked to its data), or a diagnostics-only policy refuses it |
| `frozen_sheet` | State | Attempt to edit a locked data sheet |
| `cycle_detected` | State | Circular reference in formulas, hooks, or linked regions |
| `unsupported_format` | Format | Unrecognized file header or unsupported container format |
| `unsupported_version` | Format | File, preferences, or plate-map version is newer than this release |
| `corrupt_data` | Format | Malformed JSON/XML/ZIP data or failed CRC-32 check |
| `io_error` | IO | Missing file, permission denied, or filesystem failure |
| `path_escape` | Policy | Path traversal (`..`), external absolute path, or symlink that points outside the allowed folder |
| `resource_limit` | Policy | Input, export, or formula exceeded a safety limit |
| `solver_failed` | Compute | Non-convergence, zero variance, singular matrix, or invalid weights |
| `insufficient_data` | Compute | Empty input, too few observations for `df`, or missing raw replicates |
| `cancelled` | Compute | Operation cancelled through MCP or an engine cancellation token |
| `internal_panic` | Internal | Unexpected internal fault, caught before it could crash CurveBook |
| `internal_error` | Internal | Unclassified internal dispatch or serialization error |

Error details and limits:

- Unlocking sheets (`frozen_sheet`): Run `sheet.unfreeze` before you modify a locked data sheet.
- Resource limits (`resource_limit`): Safety limits prevent runaway memory or disk use.
  - Project archives are limited to 128 MiB, CSV files to 64 MiB, and sheets to 30 million cells.
  - Raster exports are limited to 16,384 px per edge (67 million pixels in total).
  - PDF exports are limited to 64 pages or 32 MiB.
  - Formulas run with bounded step limits.
- Missing replicates (`insufficient_data`): Some tests need raw replicate values. They cannot run when you have entered only summary columns (mean, SD/SEM, and N).
- Version limits (`unsupported_version`): CurveBook supports project archives up to schema version 9, plate-map files up to version 1, and preferences files up to version 1.
- Diagnostics-only policy (`conflict`): Assumption banners only report diagnostics. They reject requests for automatic remediation.

---

## Known issues and Beta scope {#known-issues}

- Supported interfaces and verification: CurveBook supports the native desktop GUI (`curvebook`) alongside `curvebook-cli` and `curvebook-cli mcp`. For Beta, automated desktop interface suites verify workflows that use widgets, menus, dialogs, and the canvas. Recorded manual walkthroughs across operating systems are part of release verification for 1.0. FreeBSD (x86_64) is a Tier 2, CLI-only package with no desktop GUI.
- Windows packages and signing: Beta does not include 32-bit Windows (x86) builds or portable `.zip` archives. The Windows x64 and ARM64 `.msi` installers are unsigned during Beta. Verify the checksums and build attestations before you click **More info** and **Run anyway** in SmartScreen. In enforcement mode, Windows 11 Smart App Control blocks unsigned installers until Windows code signing or Microsoft Store MSIX distribution is available. If you have an Alpha 32-bit copy, uninstall it manually before you install the 64-bit `.msi`.
- Draft translations: Until native-speaker scientific review is finished, the non-English Beta interface languages show a `[Draft]` label in **View › Language**. These languages are German, Spanish, French, Italian, Dutch, Polish, Portuguese, Russian, Turkish, Japanese, Korean, and Simplified and Traditional Chinese.
- Right-to-left interface languages: Full right-to-left mirroring of the desktop layout (for Arabic, Hebrew, Persian, and Urdu) is unscheduled and unlikely. CurveBook hides right-to-left languages in the language picker, and falls back to English if an existing preferences file requests one. Mathematical axes and formulas stay left-to-right.
- A browser version of CurveBook is unscheduled.
- No bundled binary fonts: CJK, RTL, and special scientific symbols depend on system TrueType and OpenType fonts, or on fonts you supply through `CURVEBOOK_FONTS_DIR` or `--ttf-font` ([Fonts](get-started.md#fonts)).
- Legacy archives are rejected: CurveBook rejects early pre-CurveBook archive files with `unsupported_format`. It opens current `.curvebook` and `.curvebook.d/` projects, `.cbtemplate` templates, `.cbplate` plate maps, and `.csv`, `.tsv`, `.xlsx`, `.pzfx`, and `.prism` files.
- Calculated Variables support numeric math, Boolean and three-valued logic, conditional aggregates, and window functions. Text manipulation and date arithmetic are outside the Beta scope.
- Release notes: See [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) for the notes, checksums, and build attestations of each release.
