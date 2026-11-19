# Get Started & Installation

CurveBook ships standalone desktop (`curvebook`) and headless CLI/MCP (`curvebook-cli`) binaries built in pure Rust with zero telemetry and zero runtime network egress.

---

## Which File Do I Need? (System Requirements & Beta Assets) {#system-requirements}

Every Beta release attaches one installer package per supported desktop platform/architecture under the naming scheme `curvebook-<version>-<os>-<arch>.<ext>`. Every package bundles both the desktop GUI (`curvebook`) and the headless CLI/MCP executable (`curvebook-cli`):

| Platform | Beta Release Asset(s) | Notes & Omitted Formats (`R0.28`) |
|---|---|---|
| **macOS** | `curvebook-<version>-macos-universal.dmg` | macOS 11+ (`arm64` Apple Silicon and `x86_64` Intel universal binary); Developer ID signed & Apple-notarized (`R8.10`). Bundles `CurveBook.app` and `curvebook-cli` (no separate macOS CLI `.zip`). |
| **Windows** | `curvebook-<version>-windows-x64.msi`<br>`curvebook-<version>-windows-arm64.msi` | Windows 10/11 (`x64` / `arm64`). Currently **unsigned** pending code signing (`R8.11`). Windows `x86` (32-bit) and `-portable.zip` are dropped; Microsoft Store MSIX (`R8.9`) is a separate Store channel alternative. |
| **Linux** | `curvebook-<version>-linux-x86_64.AppImage`<br>`curvebook-<version>-linux-aarch64.AppImage` | Wayland / X11 desktop + bundled `curvebook-cli`. Requires FUSE v2 (`libfuse2`) or `--appimage-extract`. `.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz`, and Flatpak packages are omitted. |
| **FreeBSD (Preview)** | `curvebook-<version>-freebsd-x86_64.tar.gz` | Desktop GUI & CLI **preview** tarball for `x86_64`. |
| **Shared Verification & Attribution** | `SHA256SUMS.txt`<br>`THIRD-PARTY-LICENSES.html` | Published alongside every GitHub Release for checksum verification and dependency licence disclosure. |

> [!NOTE]
> The browser WebAssembly bundle (`curvebook-web`) is deferred until CurveBook `1.0` (`R0.28`, `R11`) and is not published in Beta releases.

---

## Installing Beta Release Packages {#installing-packages}

Download release assets from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) (never the automatic GitHub "Source code" archive unless compiling from source).

### macOS (`universal.dmg`) {#install-macos}

1. Open `curvebook-<version>-macos-universal.dmg`.
2. Drag **CurveBook.app** into **Applications** (`/Applications/CurveBook.app`).
3. Beta `.dmg` releases are Developer ID signed and Apple-notarized (`R8.10`).
4. **Adding the bundled CLI (`curvebook-cli`) to `PATH`:** Beta macOS releases bundle `curvebook-cli` directly inside `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli`. Either pass that full path in scripts and MCP client configs or add the `MacOS` directory to `PATH` in `~/.zshrc`:

```zsh
export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"
```

Verify installation with `curvebook-cli --version`.

### Windows (`x64` / `arm64` `.msi`) {#install-windows}

1. Run `curvebook-<version>-windows-x64.msi` (or `curvebook-<version>-windows-arm64.msi` on Windows on ARM).
2. **Unsigned Beta SmartScreen & Store note:** Beta Windows `.msi` installers are currently **unsigned** (`R8.11` pending). After verifying `SHA256SUMS.txt` and `gh attestation verify` below, click **More info** -> **Run anyway** when Windows Defender SmartScreen shows *"Windows protected your PC"*.
   - **Windows 11 Smart App Control:** Systems running Smart App Control in enforcement mode block unsigned installers at the OS level until Authenticode signing lands (`R8.11`) or a signed Microsoft Store **MSIX** distribution ships under `R8.9` as a Store channel alternative.
3. By default, the MSI installs per-user under `%LocalAppData%\Programs\CurveBook` (exposing `curvebook.exe` and `curvebook-cli.exe`). Administrators performing machine-wide silent deployment pass `ALLUSERS=1` to `msiexec`. Cross-context user vs. machine duplicate installs are rejected until the opposite scope is uninstalled.
4. **Upgrading from Alpha `x86`:** Windows `x86` is no longer published for Beta, and upgrading directly from an Alpha `x86` `.msi` to Beta `x64` has not yet been runner-verified; uninstall any Alpha `x86` copy manually before installing `x64`.

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

This section is the complete user guide for verifying Beta downloads. Download both your installer package and `SHA256SUMS.txt` from the same release tag into the same working directory before running the checks below:

- **Checksum (`SHA256SUMS.txt`):** Confirms the downloaded archive was not corrupted in transit.
- **Sigstore Build Provenance (`gh attestation verify`):** Confirms `SHA256SUMS.txt` (and every individual release file) was built by CurveBook's GitHub Actions release workflow from a recorded commit (requires the [GitHub CLI](https://cli.github.com/) `gh`, signed in via `gh auth login`). Verify provenance at least once, because a checksum alone cannot protect against a tampered file paired with a tampered `SHA256SUMS.txt`.

1. **Verify `SHA256SUMS.txt` build provenance:**

```sh
gh attestation verify SHA256SUMS.txt \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

   For tag-built releases, also pin the release tag by appending `--source-ref refs/tags/v<version>`. Keep `--signer-workflow` included so `gh` accepts attestations only from `release.yml`.

2. **Verify your downloaded file's SHA-256 checksum against `SHA256SUMS.txt`:**

   - **macOS** (prints `OK` next to the matched file):

     ```sh
     shasum -a 256 --check --ignore-missing SHA256SUMS.txt
     ```

   - **Linux** (prints `OK` next to the matched file):

     ```sh
     sha256sum --check --ignore-missing SHA256SUMS.txt
     ```

   - **FreeBSD** (replace `<version>` if needed; exits with status `0` silently on match and reports mismatches):

     ```sh
     f=curvebook-<version>-freebsd-x86_64.tar.gz
     sha256 -c "$(awk -v f="$f" '$2 == f { print $1 }' SHA256SUMS.txt)" "$f"
     ```

   - **Windows PowerShell** (replace the filename for `arm64` if needed; prints `True` on match):

     ```powershell
     $f = "curvebook-<version>-windows-x64.msi"
     $expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
     (Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
     ```

3. **Alternative — verify a downloaded installer directly via Sigstore:**

```sh
gh attestation verify <downloaded-file> \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

> [!NOTE]
> Linux `.AppImage` and FreeBSD `.tar.gz` assets ship without separate GPG/Minisign signature files because every release asset (and `SHA256SUMS.txt`) carries a public Sigstore provenance attestation (`R8.10`). If any verification step fails, do **not** run the installer — delete the file, re-download from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases), and report persistent mismatches via [Privacy & Security](privacy-security.md#security-reporting).


### What a Passing Check Means {#verification-meaning}

- The file's SHA-256 hash matches an attestation signed by `.github/workflows/release.yml` in `HauserGroup/curvebook`. The attestation names the commit and ref the release was built from.
- Without `--signer-workflow`, `gh` accepts an attestation from *any* workflow in the repository, so keep that option.
- It does not mean an operating system trusts the file. OS trust signing is separate (`R8.10` and `R8.11` in the [roadmap](../ROADMAP.md)): from Beta onward, macOS `.dmg` downloads are Developer ID signed and notarized, whereas Windows `.msi` (`x64` and `arm64`, following `R0.28` retirement of `x86` and `-portable.zip`) ships unsigned until `R8.11`.

### If a Check Fails {#verification-fails}

Do not open or install the file. Download it again from the release page and repeat the check. If it still fails, report it as described in [SECURITY.md](../../SECURITY.md).

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
3. Your saved preferences, keyboard shortcuts, and recovery directory stay preserved in your OS configuration folder ([Settings, Shortcuts & Storage Paths](settings.md#storage-paths)).

### Uninstalling Cleanly {#uninstalling}

- **macOS:** Move `/Applications/CurveBook.app` to Trash and remove any `PATH` export line added to `~/.zshrc`; optionally remove `~/Library/Application Support/CurveBook`.
- **Windows:** Uninstall **CurveBook** from **Settings -> Apps -> Installed apps** (or `msiexec /x`); optionally delete `%APPDATA%\CurveBook` and `%LOCALAPPDATA%\CurveBook`.
- **Linux / FreeBSD:** Delete the `.AppImage` (and optional extracted `squashfs-root/`) or FreeBSD tarball folder; optionally remove `~/.config/curvebook` and `~/.local/share/curvebook`.

See [Desktop GUI Quick Start](desktop-gui.md) or [Automation: MCP](automation/mcp.md) to launch your first workspace.
