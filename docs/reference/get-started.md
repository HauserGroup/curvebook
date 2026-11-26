# Get started & installation

CurveBook runs offline on your computer with zero telemetry and no background network connections. Each desktop installer bundles both the desktop app (`curvebook`) and the command-line and MCP server tool (`curvebook-cli`).

> [!NOTE]
> **Release assets note:** The steps and file list below describe CurveBook's Beta packages, which ship once Beta is tagged. Check the **Assets** list on the latest release at [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) to see the packages available for download today.

---

## Install CurveBook {#installing-packages}

Download the installer for your platform from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases). Do not download GitHub's automatic **Source code** archive unless you want to build CurveBook from source.

### macOS {#install-macos}

1. Download `curvebook-<version>-macos-universal.dmg` (macOS 11 or later, Apple Silicon and Intel).
2. Open the `.dmg` file and drag **CurveBook.app** into **Applications**.
3. Open **CurveBook** from **Applications**. Beta macOS installers are signed with an Apple Developer ID and notarized.

To run the bundled CLI from Terminal or an AI assistant, use `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` directly, or add it to `PATH` in `~/.zshrc`:

```zsh
export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"
```

Run `curvebook-cli --version` to confirm the CLI works.

### Windows {#install-windows}

1. Download `curvebook-<version>-windows-x64.msi` (or `curvebook-<version>-windows-arm64.msi` on Windows on ARM) and `SHA256SUMS.txt` from the same release into one folder.
2. **Verify your download before running it.** Beta Windows installers are currently unsigned. Verify `SHA256SUMS.txt` with the [GitHub CLI](https://cli.github.com/) and check the installer hash in PowerShell before approving any security prompt:

```powershell
gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
$f = "curvebook-<version>-windows-x64.msi"
$expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
(Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
```

   Replace the filename if you downloaded the ARM64 installer. The hash check prints `True` when the file matches; if either command fails, stop and see [If a check fails](#verification-fails).
3. Double-click the verified `.msi` file. When Windows Defender SmartScreen shows *"Windows protected your PC"*, click **More info** and then **Run anyway**.
4. Finish the installer wizard. By default, CurveBook installs for your user account in `%LocalAppData%\Programs\CurveBook` with both `curvebook.exe` and `curvebook-cli.exe`.

Important Windows notes:

- **Windows 11 Smart App Control:** Systems running Smart App Control in enforcement mode block unsigned installers at the OS level until Windows code signing or the signed Microsoft Store MSIX package becomes available.
- **Machine-wide installs:** Administrators can deploy for all users by passing `ALLUSERS=1` to `msiexec`. Uninstall an existing per-user or machine-wide installation before switching scopes.
- **Upgrading from Alpha 32-bit (x86):** Beta no longer publishes 32-bit Windows packages. Uninstall any Alpha 32-bit copy manually before installing the 64-bit `.msi`.

### Linux {#install-linux}

1. Download `curvebook-<version>-linux-x86_64.AppImage` (or the `aarch64` build on ARM64 Linux).
2. Make the file executable and launch CurveBook:

```sh
chmod +x curvebook-<version>-linux-x86_64.AppImage
./curvebook-<version>-linux-x86_64.AppImage
```

3. To run the bundled CLI directly—or to run on a system without FUSE v2 (`libfuse2`)—extract the AppImage once:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/usr/bin/curvebook-cli --version
```

If launching the AppImage reports a missing FUSE library, see [Troubleshooting: Linux FUSE](troubleshooting.md#linux-fuse).

### FreeBSD {#install-freebsd}

FreeBSD ships as a Tier 2 CLI-only archive for x86_64 (`curvebook-cli` and local MCP, with no desktop GUI). Extract the tarball and run `bin/curvebook-cli`:

```sh
tar -xzf curvebook-<version>-freebsd-x86_64.tar.gz
```

---

## System requirements and release files {#system-requirements}

Every Beta release follows the naming scheme `curvebook-<version>-<os>-<arch>.<ext>`:

| Platform | Beta release files | Included | Status |
|---|---|---|---|
| **macOS** (11+, Apple Silicon & Intel) | `curvebook-<version>-macos-universal.dmg` | Desktop app, CLI | Signed and notarized |
| **Windows** (10/11, x64 & ARM64) | `curvebook-<version>-windows-x64.msi`<br>`curvebook-<version>-windows-arm64.msi` | Desktop app, CLI | Unsigned in Beta |
| **Linux** (Wayland/X11, x86_64 & ARM64) | `curvebook-<version>-linux-x86_64.AppImage`<br>`curvebook-<version>-linux-aarch64.AppImage` | Desktop app, CLI | Needs `libfuse2` or `--appimage-extract` |
| **FreeBSD** (x86_64) | `curvebook-<version>-freebsd-x86_64.tar.gz` | CLI and MCP only | Tier 2, no desktop GUI |
| **Shared release metadata** | `SHA256SUMS.txt`<br>`curvebook-<version>.cdx.json`<br>`THIRD-PARTY-LICENSES.html` | Checksums, SBOM, licences | Published with every release |

Packaging notes:

- **One format per OS:** Separate macOS CLI `.zip` archives, Windows 32-bit (x86) and portable `.zip` packages, and Linux `.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz`, and Flatpak bundles are not built.
- **Microsoft Store and browser app:** A signed Microsoft Store MSIX package is planned as a separate Store channel. The browser WebAssembly app (`curvebook-web`) is deferred until CurveBook 1.0 and is not published in Beta releases.
- **SBOM and licences:** `curvebook-<version>.cdx.json` provides a CycloneDX 1.5 dependency inventory, and `THIRD-PARTY-LICENSES.html` lists third-party attributions.

---

## Verifying downloads {#verifying-downloads}

Download both your release file and `SHA256SUMS.txt` from the same release tag into one folder before running these checks:

- **Checksums (`SHA256SUMS.txt`):** Confirm the file was not corrupted during download.
- **Build provenance (`gh attestation verify`):** Confirms GitHub Actions built `SHA256SUMS.txt` and each package from CurveBook's repository. This check requires the [GitHub CLI](https://cli.github.com/) signed in with `gh auth login`. Verify provenance at least once so a tampered file cannot pass with a matching tampered checksum list.

1. **Verify `SHA256SUMS.txt` build provenance:**

```sh
gh attestation verify SHA256SUMS.txt \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

   For tag-built releases, also pin the tag by adding `--source-ref refs/tags/v<version>`. Always keep `--signer-workflow` so `gh` accepts attestations only from `release.yml`.

2. **Verify your downloaded file's SHA-256 hash against `SHA256SUMS.txt`:**

   - **macOS** (prints `OK` next to the matched file):

     ```sh
     shasum -a 256 --check --ignore-missing SHA256SUMS.txt
     ```

   - **Linux** (prints `OK` next to the matched file):

     ```sh
     sha256sum --check --ignore-missing SHA256SUMS.txt
     ```

   - **FreeBSD** (exits with status `0` silently on match and reports mismatches):

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

3. **Optional — verify an installer and its CycloneDX SBOM attestation directly:**

```sh
gh attestation verify <downloaded-file> \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

To verify the signed CycloneDX 1.5 SBOM attestation attached to an installer or FreeBSD archive:

```sh
gh attestation verify <downloaded-file> \
  --repo HauserGroup/curvebook \
  --predicate-type https://cyclonedx.org/bom \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

> [!NOTE]
> Linux `.AppImage` and FreeBSD `.tar.gz` packages do not ship separate GPG or Minisign signature files. Every release file carries a public Sigstore build provenance attestation instead.

### What a passing check means {#verification-meaning}

- Your file's SHA-256 hash matches an attestation signed by `.github/workflows/release.yml` in `HauserGroup/curvebook`.
- The attestation records the exact commit and Git ref used to build the release.
- Keep `--signer-workflow` in the command; without it, `gh` accepts attestations from any workflow in the repository.
- Build provenance is separate from operating-system code signing. Beta macOS `.dmg` packages are Developer ID signed and notarized, while Beta Windows `.msi` packages ship unsigned until Windows code signing is added.

### If a check fails {#verification-fails}

**Do not open or install the file.** Delete it, download a fresh copy from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases), and repeat the check. If the mismatch persists, report it through [Privacy & Security](privacy-security.md#security-reporting) or [SECURITY.md](../../SECURITY.md).

---

## Fonts and `CURVEBOOK_FONTS_DIR` {#fonts}

CurveBook bundles no binary font files. It discovers standard TrueType and OpenType fonts installed on your system, such as Arial, Noto Sans CJK, DejaVu Sans, or Liberation Sans. A built-in fallback metrics engine covers canvas drawing and PDF export when a face is unavailable.

- **Custom font folder:** Set `CURVEBOOK_FONTS_DIR` to a folder containing `.ttf` or `.otf` files. CurveBook loads those fonts in alphabetical order ahead of system fallbacks for canvas text, SVG output, and embedded PDF fonts.
- **Single font file:** Pass `--ttf-font <path>` when launching `curvebook` to load one specific TrueType font at startup.

---

## Updating and uninstalling {#updating-and-uninstalling}

### Updating without an auto-updater {#updating}

CurveBook makes no background network connections ([Privacy & Security](privacy-security.md#zero-egress)), so it never checks for updates automatically. To update:

1. Download and verify the new release from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases).
2. Replace **CurveBook.app** on macOS, replace the `.AppImage` on Linux (or the extracted folder on FreeBSD), or run the new `.msi` on Windows. The Windows installer remembers your previous folder and scope.
3. Your saved preferences, keyboard shortcuts, and recovery files stay intact in your OS configuration directory ([Settings, shortcuts & storage paths](settings.md#storage-paths)).

### Uninstalling cleanly {#uninstalling}

- **macOS:** Move `/Applications/CurveBook.app` to Trash and remove any `PATH` line you added to `~/.zshrc`. Optionally delete `~/Library/Application Support/CurveBook`.
- **Windows:** Open **Settings › Apps › Installed apps** and uninstall **CurveBook** (or run `msiexec /x`). Optionally delete `%APPDATA%\CurveBook` and `%LOCALAPPDATA%\CurveBook`.
- **Linux / FreeBSD:** Delete the `.AppImage` (and any extracted `squashfs-root/` folder) or the extracted FreeBSD folder. Optionally remove `~/.config/curvebook` and `~/.local/share/curvebook`.

Next steps: open your first workspace in the [Desktop GUI overview](desktop-gui.md), connect an AI assistant with [MCP setup for beginners](../mcp-for-beginners.md), or read the [MCP automation reference](automation/mcp.md).
