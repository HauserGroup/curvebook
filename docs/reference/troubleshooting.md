# Troubleshooting, Error Codes & Beta Known Limits

Use this reference to diagnose installation warnings, font discovery issues, MCP/CLI confinement errors, WAL crash recovery messages, structured `ErrorCode` diagnostics, and known Beta platform limits.

---

## Installation & OS Security Prompts {#install-issues}

### macOS Gatekeeper / Quarantine {#macos-gatekeeper}

- Beta universal `.dmg` releases are Developer ID signed and Apple-notarized (`R8.10`). If Gatekeeper flags a file copied across unsigned local media, confirm checksum and provenance with [`verifying-downloads.md`](../release/verifying-downloads.md), and ensure `CurveBook.app` lives inside `/Applications` with clean extended attributes.
- If `curvebook-cli` reports `command not found` in Terminal, verify `/Applications/CurveBook.app/Contents/MacOS` is on `PATH` ([Get Started: macOS](get-started.md#install-macos)).

### Windows SmartScreen & Smart App Control {#windows-smartscreen}

- **Defender SmartScreen ("Windows protected your PC"):** Beta Windows `.msi` installers (`x64`, `arm64`) are **unsigned** until `R8.11`. After verifying `SHA256SUMS.txt` and `gh attestation verify` against `HauserGroup/curvebook`, select **More info** -> **Run anyway**.
- **Windows 11 Smart App Control (Enforcement Mode):** Smart App Control blocks unsigned installers and executables automatically without a bypass prompt; signed Windows releases will resolve this in `R8.11`.
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

---

## Crash Recovery & File Loading {#recovery-and-files}

- **Recovery journal replay warnings:** On launch after an unexpected crash, CurveBook replays `snapshot.curvebook` + valid lines of `ops.journal` cleanly up to the last intact CRC-32 record (`RecoveryReport`), logging `JournalRecoveryWarning` diagnostics for any torn final line.
- **Parallel desktop instances & `curvebook.lock`:** Active locks isolate additional windows into fresh per-instance recovery directories automatically; expired leases from dead local processes are reclaimed safely (`is_dead_same_host_owner`). Set `CURVEBOOK_NO_PREFS=1` to launch cleanly without touching preferences or recovery state.
- **`unsupported_version` on open or save:** Occurs when opening or targeting a file written with `schema_version > 3` (`project.save` / `save_project_unpacked_dir` refuses to overwrite newer files to prevent data loss). Open and edit with the newer release, or save to a new destination path.
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
| `unsupported_version` | `Format` | File/preferences schema version exceeds this release's supported maximum (`CURRENT_SCHEMA_VERSION = 3`, `CURRENT_PREFERENCES_VERSION = 1`). |
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

## Beta Known Limits {#beta-known-limits}

- **Windows code signing:** Windows `x64` / `arm64` `.msi` installers are unsigned in Beta (`R8.11` pending); Windows `x86` and `-portable.zip` packages are omitted (`R0.28`).
- **Draft translations:** Non-English UI translations across all non-English locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`, `it`, `nl`, `pl`, `ru`, `tr`, `ar`, `he`) display a `[Draft]` suffix pending native-speaker review (`R10.23`).
- **FreeBSD package:** Shipped as an experimental desktop + CLI `.tar.gz` preview.
- **Browser bundle (`curvebook-web`):** Deferred to `1.0` (`R0.28`, `R11`).
- **Formula scope:** `=` Calculated Variables support numeric, boolean/3-valued logic, conditional aggregates, and window functions, and omit text-manipulation and date-arithmetic functions in Beta.
- **Local fonts:** International glyph rendering depends on OS-installed TrueType/OpenType fonts or `CURVEBOOK_FONTS_DIR`.
