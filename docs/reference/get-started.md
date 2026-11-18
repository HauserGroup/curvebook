# Get Started & Installation

CurveBook ships standalone desktop (`curvebook`) and headless CLI/MCP (`curvebook-cli`) binaries built in pure Rust with zero telemetry and zero runtime network egress.

---

## System Requirements & Zero-Egress Design {#system-requirements}

| Platform | Beta Release Asset(s) | Notes |
|---|---|---|
| **macOS** | `curvebook-<version>-macos-universal.dmg` | macOS 11+ (`arm64` Apple Silicon and `x86_64` Intel universal binary); Developer ID signed & notarized (`R8.10`). |
| **Windows** | `curvebook-<version>-windows-x64.msi`<br>`curvebook-<version>-windows-arm64.msi` | Windows 10/11 (`x64` / `arm64`). Currently **unsigned** pending code-signing rollout (`R8.11`). Windows `x86` and `-portable.zip` are not published for Beta. |
| **Linux** | `curvebook-<version>-linux-x86_64.AppImage`<br>`curvebook-<version>-linux-aarch64.AppImage` | Wayland / X11 desktop + CLI inside AppImage. Requires FUSE v2 (`libfuse2`) or `--appimage-extract`. `.deb`, `.rpm`, `.tar.gz`, and Flatpak bundles are omitted for Beta. |
| **FreeBSD (Preview)** | `curvebook-<version>-freebsd-x86_64.tar.gz` | Desktop GUI & CLI preview tarball for `x86_64`. |
| **Shared Verification & Attribution** | `SHA256SUMS.txt`<br>`THIRD-PARTY-LICENSES.html` | Published alongside every GitHub Release for checksum verification and dependency licence disclosure. |

> [!NOTE]
> The browser WebAssembly bundle (`curvebook-web`) is deferred until CurveBook `1.0` (`R0.28`) and is not published in Beta releases.

---

## Installing Beta Release Packages {#installing-packages}

Download release assets from the [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) page (never the GitHub automatic "Source code" archive unless compiling from source).

### macOS (`universal.dmg`) {#install-macos}

1. Open `curvebook-<version>-macos-universal.dmg`.
2. Drag **CurveBook.app** into **Applications** (`/Applications/CurveBook.app`).
3. Release `.dmg` packages are Developer ID signed and Apple-notarized (`R8.10`).
4. **Adding the bundled CLI (`curvebook-cli`) to `PATH`:** Beta macOS releases bundle `curvebook-cli` directly inside the app bundle at `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` (no separate CLI `.zip` archive is published). Either invoke that full path in scripts/MCP configs or add the `MacOS` directory (or a symlink) to `PATH` in `~/.zshrc`:

```zsh
export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"
```

Verify installation with `curvebook-cli --version`.

### Windows (`x64` / `arm64` `.msi`) {#install-windows}

1. Run `curvebook-<version>-windows-x64.msi` (or `curvebook-<version>-windows-arm64.msi` on Windows on ARM).
2. **Unsigned Beta SmartScreen prompt:** Windows Beta installers are currently unsigned (`R8.11` pending). Windows Defender SmartScreen displays *"Windows protected your PC"*; click **More info** -> **Run anyway** after verifying the file's SHA-256 checksum and Sigstore attestation below.
   - Note: Systems with **Windows 11 Smart App Control** enabled in enforcement mode block unsigned binaries and installers at the OS level until signed releases ship.
3. By default, the MSI installs for the current account under `%LocalAppData%\Programs\CurveBook` (and exposes `curvebook.exe` and `curvebook-cli.exe`). Administrators performing machine-wide silent deployment pass `ALLUSERS=1` to `msiexec`. Cross-context user vs. machine duplicate installs are rejected until the opposite scope is uninstalled.

### Linux (`x86_64` / `aarch64` `.AppImage`) {#install-linux}

1. Mark the downloaded AppImage executable and run it:

```sh
chmod +x curvebook-<version>-linux-x86_64.AppImage
./curvebook-<version>-linux-x86_64.AppImage
```

2. Run the bundled headless CLI directly via `--appimage-extract` or extract into a stable folder on systems without `libfuse2`:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/usr/bin/curvebook-cli --version
```

See [Troubleshooting](troubleshooting.md#linux-fuse) if FUSE v2 is unavailable.

### FreeBSD Preview (`x86_64` `.tar.gz`) {#install-freebsd}

Extract the preview archive and run `bin/curvebook` or `bin/curvebook-cli`:

```sh
tar -xzf curvebook-<version>-freebsd-x86_64.tar.gz
```

---

## Verifying Checksums & Build Attestations {#verifying-downloads}

Download `SHA256SUMS.txt` alongside your installer from the same release tag and verify integrity before running the installer. For complete step-by-step instructions across platforms, see [Verifying Downloads](../release/verifying-downloads.md).

1. **Verify SHA-256 Checksums:**
   - **macOS:** `shasum -a 256 -c SHA256SUMS.txt --ignore-missing`
   - **Linux:** `sha256sum -c SHA256SUMS.txt --ignore-missing`
   - **Windows (PowerShell):** compare `(Get-FileHash .\curvebook-<version>-windows-x64.msi -Algorithm SHA256).Hash` against the entry in `SHA256SUMS.txt`.
2. **Verify GitHub Sigstore Build Provenance Attestation** (requires GitHub CLI `gh`):

```sh
gh attestation verify curvebook-<version>-macos-universal.dmg \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

Every Beta release artifact and `SHA256SUMS.txt` itself receives a keyless GitHub Sigstore provenance attestation during CI publishing.

---

## Fonts & `CURVEBOOK_FONTS_DIR` {#fonts}

CurveBook contains **zero bundled binary font files** in its source tree (`cargo xtask assets`) and relies by default on standard OS TrueType/OpenType system fonts (e.g., `Arial Unicode.ttf` / `Arial.ttf` / `AppleGothic.ttf` / `Monaco.ttf` on macOS, `NotoSansCJK` / `DejaVuSans` / `LiberationSans` on Linux/FreeBSD, `arial.ttf` / `consola.ttf` on Windows) paired with a minimal clean-room fallback vector font metrics engine and Core-14 PDF metrics.

- **Custom font directory (`CURVEBOOK_FONTS_DIR`):** Set `CURVEBOOK_FONTS_DIR` to a directory containing `.ttf` / `.otf` files (loaded lexicographically ahead of OS fallbacks for desktop canvas measurement, SVG embedding, and TrueType PDF embedding).
- **Explicit single font (`--ttf-font <path>`):** Pass `--ttf-font <path>` to `curvebook` to validate and load a specific TrueType/SFNT font at startup.

---

## Updating & Uninstalling {#updating-and-uninstalling}

### Updating Without an Auto-Updater {#updating}

Because CurveBook enforces **zero background network egress** ([Privacy & Security](privacy-security.md#zero-egress)), it never checks for updates automatically or phones home. To update:

1. Download and verify the new release package from [GitHub Releases](https://github.com/HauserGroup/curvebook/releases).
2. Replace `CurveBook.app` on macOS, replace the `.AppImage` on Linux (or `.tar.gz` directory on FreeBSD), or run the new `.msi` on Windows (which preserves your remembered install folder and scope).
3. Your saved preferences, keyboard shortcuts, and recovery directory stay preserved in your OS configuration folder (`settings.md#storage-paths`).

### Uninstalling Cleanly {#uninstalling}

- **macOS:** Move `/Applications/CurveBook.app` to Trash and remove any `PATH` export line added to `~/.zshrc`; optionally remove `~/Library/Application Support/CurveBook`.
- **Windows:** Uninstall **CurveBook** from **Settings -> Apps -> Installed apps** (or `msiexec /x`); optionally delete `%APPDATA%\CurveBook` and `%LOCALAPPDATA%\CurveBook`.
- **Linux / FreeBSD:** Delete the `.AppImage` (and optional extracted `squashfs-root/`) or FreeBSD tarball folder; optionally remove `~/.config/curvebook` and `~/.local/share/curvebook`.

See [Desktop GUI Quick Start](desktop-gui.md) or [Automation: MCP](automation/mcp.md) to launch your first workspace.
