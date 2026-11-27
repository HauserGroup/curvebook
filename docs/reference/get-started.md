# Get started & installation

<!-- Audience: Tier 1 Scientist Guide — keep sentences short and plain; put exhaustive format or troubleshooting details in files.md or troubleshooting.md. -->

CurveBook runs offline on your computer with zero telemetry and no background network connections. Every desktop installer includes both the desktop app and the command-line tool (`curvebook-cli`).

> [!NOTE]
> **Release assets note:** The steps below describe CurveBook's Beta packages. Check the **Assets** list on [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases) to see the downloads currently available.

---

## Install CurveBook {#installing-packages}

Download the installer for your platform from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases). Do not download GitHub's automatic **Source code** archive unless you plan to build from source.

### macOS {#install-macos}

1. Download `curvebook-<version>-macos-universal.dmg` (macOS 11 or later, Apple Silicon and Intel).
2. Open the disk image and drag **CurveBook.app** into **Applications**.
3. Open **CurveBook** from **Applications**. Beta macOS installers are signed and notarized.

To run the command-line tool from Terminal or an AI assistant, call `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` directly, or add it to your shell PATH in `~/.zshrc`:

```zsh
export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"
```

Run `curvebook-cli --version` to check that it works.

### Windows {#install-windows}

1. Download `curvebook-<version>-windows-x64.msi` (or the ARM64 installer on Windows on ARM) and `SHA256SUMS.txt` into one folder.
2. **Verify your download before running it.** Beta Windows installers are currently unsigned. Check `SHA256SUMS.txt` with the [GitHub CLI](https://cli.github.com/) and verify the installer hash in PowerShell first:

```powershell
gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
$f = "curvebook-<version>-windows-x64.msi"
$expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
(Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
```

   Change the filename if you downloaded the ARM64 installer. The hash check prints **True** when the file matches. If either check fails, stop and see [If a check fails](#verification-fails).
3. Double-click the verified installer. When Windows Defender SmartScreen shows *"Windows protected your PC"*, click **More info** and then **Run anyway**.
4. Finish the wizard. By default, CurveBook installs for your user account in `%LocalAppData%\Programs\CurveBook` with both the desktop app and `curvebook-cli.exe`.

Important Windows notes:

- **Windows 11 Smart App Control:** Systems in enforcement mode block unsigned installers until Windows code signing or the signed Microsoft Store package ships.
- **Machine-wide installs:** Administrators can deploy for all users with `msiexec` (`ALLUSERS=1`). Uninstall any existing copy before switching install scopes.
- **Upgrading from Alpha 32-bit:** Beta publishes 64-bit Windows installers only. Uninstall any 32-bit Alpha copy manually first.

### Linux {#install-linux}

1. Download `curvebook-<version>-linux-x86_64.AppImage` (or the ARM64 build on aarch64 Linux).
2. Make the file executable and launch CurveBook:

```sh
chmod +x curvebook-<version>-linux-x86_64.AppImage
./curvebook-<version>-linux-x86_64.AppImage
```

3. To run the bundled CLI directly—or on systems without FUSE v2—extract the AppImage once:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/usr/bin/curvebook-cli --version
```

If launching reports a missing FUSE library, see [Troubleshooting: Linux FUSE](troubleshooting.md#linux-fuse).

### FreeBSD {#install-freebsd}

FreeBSD ships as a CLI-only preview archive for x86_64 (command-line tool and local MCP, no desktop app). Extract the archive and run `bin/curvebook-cli`:

```sh
tar -xzf curvebook-<version>-freebsd-x86_64.tar.gz
```

---

## System requirements and release files {#system-requirements}

Every Beta release uses the naming pattern `curvebook-<version>-<os>-<arch>.<ext>`:

| Platform | Beta release files | Included | Status |
|---|---|---|---|
| **macOS** (11+, Apple Silicon & Intel) | `curvebook-<version>-macos-universal.dmg` | Desktop app, CLI | Signed and notarized |
| **Windows** (10/11, x64 & ARM64) | `curvebook-<version>-windows-x64.msi`<br>`curvebook-<version>-windows-arm64.msi` | Desktop app, CLI | Unsigned in Beta |
| **Linux** (Wayland/X11, x86_64 & ARM64) | `curvebook-<version>-linux-x86_64.AppImage`<br>`curvebook-<version>-linux-aarch64.AppImage` | Desktop app, CLI | Needs FUSE v2 or AppImage extract |
| **FreeBSD** (x86_64) | `curvebook-<version>-freebsd-x86_64.tar.gz` | CLI and MCP only | Preview, no desktop app |
| **Shared release metadata** | `SHA256SUMS.txt`<br>`curvebook-<version>.cdx.json`<br>`THIRD-PARTY-LICENSES.html` | Checksums, SBOM, licences | Published with every release |

Packaging notes:

- **One format per OS:** Beta does not build macOS CLI zips, 32-bit or portable Windows zips, or Linux .deb, .rpm, Flatpak, or zsync packages.
- **Microsoft Store and browser app:** A signed Microsoft Store MSIX package is planned separately. The browser app is deferred until CurveBook 1.0.
- **Licenses and software inventory:** Every release ships a CycloneDX software bill of materials and third-party license notices.

---

## Verifying downloads {#verifying-downloads}

Download both your installer and `SHA256SUMS.txt` from the same release into one folder:

- **Checksums:** Confirm your file was not corrupted during download.
- **Build provenance:** Confirms GitHub Actions built `SHA256SUMS.txt` and each package from CurveBook's repository. This step requires the [GitHub CLI](https://cli.github.com/) signed in with `gh auth login`.

1. **Verify `SHA256SUMS.txt` build provenance:**

```sh
gh attestation verify SHA256SUMS.txt \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

   For tagged releases, also pin the tag with `--source-ref refs/tags/v<version>`. Always keep `--signer-workflow` in the command.

2. **Check your file's SHA-256 hash against `SHA256SUMS.txt`:**

   - **macOS** (prints **OK** next to the matched file):

     ```sh
     shasum -a 256 --check --ignore-missing SHA256SUMS.txt
     ```

   - **Linux** (prints **OK** next to the matched file):

     ```sh
     sha256sum --check --ignore-missing SHA256SUMS.txt
     ```

   - **FreeBSD** (exits cleanly on match and reports mismatches):

     ```sh
     f=curvebook-<version>-freebsd-x86_64.tar.gz
     sha256 -c "$(awk -v f="$f" '$2 == f { print $1 }' SHA256SUMS.txt)" "$f"
     ```

   - **Windows PowerShell** (replace the filename for ARM64 if needed; prints **True** on match):

     ```powershell
     $f = "curvebook-<version>-windows-x64.msi"
     $expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
     (Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
     ```

You can also run `gh attestation verify` directly on any downloaded installer. For CycloneDX SBOM attestation commands and release security details, see [Privacy & Security](privacy-security.md#pages-and-releases).

> [!NOTE]
> Linux AppImage and FreeBSD archives use public Sigstore build attestations rather than separate GPG or Minisign signature files.

### What a passing check means {#verification-meaning}

- Your file's hash matches an attestation signed by CurveBook's release workflow.
- The attestation records the exact commit and tag used to build the release.
- Build provenance is separate from OS code signing. Beta macOS installers are signed and notarized, while Beta Windows installers ship unsigned.

### If a check fails {#verification-fails}

**Do not open or install the file.** Delete it, download a fresh copy from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases), and check again. If the mismatch persists, report it via [Privacy & Security](privacy-security.md#security-reporting) or [SECURITY.md](../../SECURITY.md).

---

## Fonts and custom font folders {#fonts}

CurveBook bundles no fonts and uses standard TrueType or OpenType fonts installed on your system (such as Arial, Noto Sans, DejaVu Sans, or Liberation Sans). Built-in fallback metrics keep canvas text and PDF export working when a face is unavailable.

- **Custom font folder:** Set `CURVEBOOK_FONTS_DIR` to a folder of TrueType or OpenType font files to load them ahead of system fonts.
- **Single font file:** Pass `--ttf-font <path>` when launching the desktop app from a terminal to load one TrueType font.

---

## Updating and uninstalling {#updating-and-uninstalling}

### Updating without an auto-updater {#updating}

CurveBook makes no background network connections ([Privacy & Security](privacy-security.md#zero-egress)), so it never checks for updates automatically. To update:

1. Download and verify the new release from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases).
2. Replace **CurveBook.app** on macOS, replace the AppImage on Linux (or the extracted folder on FreeBSD), or run the new Windows installer. The Windows installer remembers your previous folder and scope.
3. Your preferences, keyboard shortcuts, and recovery files stay intact in your OS configuration folder ([Settings & storage paths](settings.md#storage-paths)).

### Uninstalling cleanly {#uninstalling}

- **macOS:** Move **CurveBook.app** from Applications to Trash and remove any PATH line you added to `~/.zshrc`. Optionally delete `~/Library/Application Support/CurveBook`.
- **Windows:** Open **Settings › Apps › Installed apps** and uninstall **CurveBook**. Optionally delete `%APPDATA%\CurveBook` and `%LOCALAPPDATA%\CurveBook`.
- **Linux / FreeBSD:** Delete the AppImage (and any extracted `squashfs-root/` folder) or the extracted FreeBSD folder. Optionally remove `~/.config/curvebook` and `~/.local/share/curvebook`.

---

## How to cite CurveBook {#how-to-cite}

When reporting analyses or figures made with CurveBook, include the exact version you ran. Copy a ready-to-paste reference or BibTeX entry from **Help › Cite CurveBook**, **About CurveBook**, or **Cite** in the result **Methods** panel. You can also check the CLI version with `curvebook-cli --version`.

Plain-text reference (insert your running version number):

```text
CurveBook Contributors (2026). CurveBook [Computer software]. https://github.com/HauserGroup/curvebook
```

BibTeX:

```bibtex
@software{curvebook,
  author  = {{CurveBook Contributors}},
  title   = {{CurveBook}},
  year    = {2026},
  license = {MIT OR Apache-2.0},
  url     = {https://github.com/HauserGroup/curvebook}
}
```

Machine-readable citation metadata lives in [`CITATION.cff`](https://github.com/HauserGroup/curvebook/blob/main/CITATION.cff). A permanent Zenodo concept DOI will be added once the first archived release is minted.

Next steps: explore the [Desktop GUI overview](desktop-gui.md), connect an AI assistant with [MCP setup for beginners](../mcp-for-beginners.md), or read the [MCP automation reference](automation/mcp.md).
