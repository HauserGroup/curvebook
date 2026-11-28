# Getting started and installation

<!-- Audience: Tier 1 Scientist Guide — keep sentences short and plain; put exhaustive format or troubleshooting details in files.md or troubleshooting.md. -->

CurveBook runs offline on your computer. It sends no telemetry and makes no background network connections. Every desktop installer includes both the desktop app and the command-line tool (`curvebook-cli`).

> [!NOTE]
> **Available downloads:** The steps below describe the Beta packages. For the current downloads, check the **Assets** list on [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases).

---

## Install CurveBook {#installing-packages}

Download the installer for your computer from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases). Skip GitHub's automatic **Source code** archive unless you plan to build from source.

### macOS {#install-macos}

1. Download `curvebook-<version>-macos-universal.dmg` (macOS 11 or later, Apple Silicon and Intel).
2. Open the disk image and drag **CurveBook.app** into **Applications**.
3. Open **CurveBook** from **Applications**. Beta macOS installers are signed and notarized.

To use the command-line tool from Terminal or an AI assistant, run `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` directly, or add its folder to your shell PATH in `~/.zshrc`:

```zsh
export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"
```

Run `curvebook-cli --version` to check that it works.

### Windows {#install-windows}

1. Download `curvebook-<version>-windows-x64.msi` (or the ARM64 installer on Windows on ARM) and `SHA256SUMS.txt` into the same folder.
2. **Verify your download before you run it.** Beta Windows installers are not yet signed. First check `SHA256SUMS.txt` with the [GitHub CLI](https://cli.github.com/), then check the installer's hash in PowerShell:

```powershell
gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
$f = "curvebook-<version>-windows-x64.msi"
$expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
(Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
```

   If you downloaded the ARM64 installer, change the filename. The hash check prints **True** when the file matches. If either check fails, stop and see [If a check fails](#verification-fails).
3. Double-click the verified installer. If Windows Defender SmartScreen shows *"Windows protected your PC"*, click **More info**, then **Run anyway**.
4. Finish the wizard. By default, the desktop app and `curvebook-cli.exe` install for your user account in `%LocalAppData%\Programs\CurveBook`.

Important Windows notes:

- **Windows 11 Smart App Control:** In enforcement mode, it blocks unsigned installers until Windows code signing or the signed Microsoft Store package ships.
- **Machine-wide installs:** Administrators can install CurveBook for all users with `msiexec` (`ALLUSERS=1`). Uninstall any existing copy before you switch install scopes.
- **Upgrading from the 32-bit Alpha:** The Beta has 64-bit Windows installers only. Uninstall any 32-bit Alpha copy by hand first.

### Linux {#install-linux}

1. Download `curvebook-<version>-linux-x86_64.AppImage` (or the ARM64 build for aarch64 Linux).
2. Make the file executable and start CurveBook:

```sh
chmod +x curvebook-<version>-linux-x86_64.AppImage
./curvebook-<version>-linux-x86_64.AppImage
```

3. To run the bundled command-line tool directly, or if your system lacks FUSE v2, extract the AppImage once:

```sh
./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
./squashfs-root/usr/bin/curvebook-cli --version
```

If CurveBook reports a missing FUSE library, see [Troubleshooting: Linux FUSE](troubleshooting.md#linux-fuse).

### FreeBSD {#install-freebsd}

On FreeBSD, CurveBook is a command-line-only preview for x86_64, with the local MCP server but no desktop app. Extract the archive, then run `bin/curvebook-cli`:

```sh
tar -xzf curvebook-<version>-freebsd-x86_64.tar.gz
```

---

## System requirements and release files {#system-requirements}

Beta release files follow the naming pattern `curvebook-<version>-<os>-<arch>.<ext>`:

| Platform | Beta release files | Included | Status |
|---|---|---|---|
| **macOS** (11+, Apple Silicon & Intel) | `curvebook-<version>-macos-universal.dmg` | Desktop app, CLI | Signed and notarized |
| **Windows** (10/11, x64 & ARM64) | `curvebook-<version>-windows-x64.msi`<br>`curvebook-<version>-windows-arm64.msi` | Desktop app, CLI | Unsigned in Beta |
| **Linux** (Wayland/X11, x86_64 & ARM64) | `curvebook-<version>-linux-x86_64.AppImage`<br>`curvebook-<version>-linux-aarch64.AppImage` | Desktop app, CLI | Needs FUSE v2 or AppImage extract |
| **FreeBSD** (x86_64) | `curvebook-<version>-freebsd-x86_64.tar.gz` | CLI and MCP only | Preview, no desktop app |
| **Shared release metadata** | `SHA256SUMS.txt`<br>`curvebook-<version>.cdx.json`<br>`THIRD-PARTY-LICENSES.html` | Checksums, SBOM, licenses | Published with every release |

Packaging notes:

- **One format per OS:** The Beta does not build macOS CLI zips, 32-bit or portable Windows zips, or Linux .deb, .rpm, Flatpak, or zsync packages.
- A signed Microsoft Store (MSIX) package is planned separately. A browser version is unscheduled.
- **Licenses and software inventory:** Every release includes a CycloneDX software bill of materials (SBOM) and third-party license notices.

---

## Verifying downloads {#verifying-downloads}

Download your installer and `SHA256SUMS.txt` from the same release into one folder. There are two checks:

- **Checksums** confirm that your file was not corrupted during download.
- **Build provenance** confirms that GitHub Actions built `SHA256SUMS.txt` and each package from CurveBook's repository. This check needs the [GitHub CLI](https://cli.github.com/), signed in with `gh auth login`.

1. **Verify the build provenance of `SHA256SUMS.txt`:**

```sh
gh attestation verify SHA256SUMS.txt \
  --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

   For a tagged release, also pin the tag by adding `--source-ref refs/tags/v<version>`. Always keep `--signer-workflow` in the command.

2. **Check your file's SHA-256 hash against `SHA256SUMS.txt`:**

   - **macOS** (prints **OK** next to the matching file):

     ```sh
     shasum -a 256 --check --ignore-missing SHA256SUMS.txt
     ```

   - **Linux** (prints **OK** next to the matching file):

     ```sh
     sha256sum --check --ignore-missing SHA256SUMS.txt
     ```

   - **FreeBSD** (finishes without error on a match and reports any mismatch):

     ```sh
     f=curvebook-<version>-freebsd-x86_64.tar.gz
     sha256 -c "$(awk -v f="$f" '$2 == f { print $1 }' SHA256SUMS.txt)" "$f"
     ```

   - **Windows PowerShell** (prints **True** on a match; change the filename for ARM64 if needed):

     ```powershell
     $f = "curvebook-<version>-windows-x64.msi"
     $expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
     (Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
     ```

You can also run `gh attestation verify` directly on any downloaded installer. For CycloneDX SBOM attestation commands and other release security details, see [Privacy and Security](privacy-security.md#pages-and-releases).

> [!NOTE]
> Linux AppImages and FreeBSD archives use public Sigstore build attestations, not separate GPG or Minisign signature files.

### What a passing check means {#verification-meaning}

- Your file's hash matches an attestation signed by CurveBook's release workflow.
- The attestation records the exact commit and tag that the release was built from.
- Build provenance is not the same as operating-system code signing. Beta macOS installers are signed and notarized, but Beta Windows installers are not signed.

### If a check fails {#verification-fails}

**Do not open or install the file.** Delete it, download a fresh copy from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases), and check again. If the mismatch persists, report it through [Privacy and Security](privacy-security.md#security-reporting) or [SECURITY.md](../../SECURITY.md).

---

## Fonts and custom font folders {#fonts}

CurveBook does not bundle any fonts. It uses standard TrueType or OpenType fonts installed on your system, such as Arial, Noto Sans, DejaVu Sans, or Liberation Sans. If a font is missing, built-in fallback metrics keep canvas text and PDF export working.

- **Custom font folder:** Set `CURVEBOOK_FONTS_DIR` to a folder of TrueType or OpenType font files. CurveBook loads them before your system fonts.
- **Single font file:** To load one TrueType font, start the desktop app from a terminal with `--ttf-font <path>`.

---

## Updating and uninstalling {#updating-and-uninstalling}

### Updating without an auto-updater {#updating}

CurveBook makes no background network connections ([Privacy and Security](privacy-security.md#zero-egress)), so it never checks for updates by itself. To update:

1. Download and verify the new release from [CurveBook Releases](https://github.com/HauserGroup/curvebook/releases).
2. On macOS, replace **CurveBook.app**. On Linux, replace the AppImage; on FreeBSD, replace the extracted folder. On Windows, run the new installer, which remembers your previous install folder and scope.
3. Your preferences, keyboard shortcuts, and recovery files stay intact in your system's configuration folder ([settings and storage paths](settings.md#storage-paths)).

### Uninstalling cleanly {#uninstalling}

- **macOS:** Move **CurveBook.app** from Applications to the Trash, and remove any PATH line you added to `~/.zshrc`. You may also delete `~/Library/Application Support/CurveBook`.
- **Windows:** Open **Settings › Apps › Installed apps** and uninstall **CurveBook**. You may also delete `%APPDATA%\CurveBook` and `%LOCALAPPDATA%\CurveBook`.
- **Linux and FreeBSD:** Delete the AppImage and any extracted `squashfs-root/` folder, or delete the extracted FreeBSD folder. You may also remove `~/.config/curvebook` and `~/.local/share/curvebook`.

---

## How to cite CurveBook {#how-to-cite}

When you report analyses or figures made with CurveBook, include the exact version you used. To copy a ready-to-paste reference or BibTeX entry, use **Help › Cite CurveBook**, **About CurveBook**, or **Cite** in a result's **Methods** panel. `curvebook-cli --version` also prints the version.

Plain-text reference (add the version number you used):

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

Machine-readable citation metadata is in [`CITATION.cff`](https://github.com/HauserGroup/curvebook/blob/main/CITATION.cff). A permanent Zenodo concept DOI will be added once the first archived release receives its DOI.

Next, explore the [desktop app overview](desktop-gui.md), connect an AI assistant with [MCP setup for beginners](../mcp-for-beginners.md), or read the [MCP automation reference](automation/mcp.md).
