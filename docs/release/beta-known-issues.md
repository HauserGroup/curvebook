# CurveBook Beta Known Issues & Release Scope

> **Release tier:** Beta (`prerelease`) · **Supported surfaces:** Desktop GUI (`curvebook`), Command-Line Runner (`curvebook-cli`), and Stdio MCP Server (`curvebook-cli mcp`) · **Onboarding:** [README](../../README.md) · [`mcp-for-beginners.md`](../mcp-for-beginners.md) · **Historical Alpha record:** [`alpha-known-issues.md`](alpha-known-issues.md)

This document records the supported Beta functional scope, streamlined download set ([`R0.28`](../ROADMAP.md#r028-rein-in-the-number-of-release-install-files)), platform signing status ([`R8.10`](../ROADMAP.md#r810-signed-macos-installers--download-verification-beta-gate), [`R8.11`](../ROADMAP.md#r811-windows-authenticode-signing-after-beta)), known limitations, and GitHub Release notes template for CurveBook's Beta release.

---

## 1. What Works in Beta

Beta presents the **desktop graphical interface (`curvebook`)** as a supported way to use CurveBook alongside the command-line runner and MCP server:

- **Desktop GUI (`curvebook`):** Interactive `LayoutTree` workspace (`R10`), Welcome & New Data Table launcher (including clean-room tutorial samples, templates, and pipeline cloning), structured spreadsheet grids with inline editing and `=` Calculated Variables, searchable `Analyze...` picker and parameter dialogs, 4-stage Power & Sample Size wizard, live right-hand Format Inspectors, interactive graphs and multi-panel Page Layouts, vector clipboard copy (`Cmd/Ctrl+C`), print preview / PDF spooling, and native OS file open/save/export dialogs ([`ui-parity.md`](../ui-parity.md)).
- **Command-line runner (`curvebook-cli`) & stdio MCP server (`curvebook-cli mcp`):** Headless command execution, `curvebook.batch.v1` pipelines, JSON-RPC 2.0 over stdio (`stdin`/`stdout`), `--workdir <dir>` path confinement (`WorkdirGuard`), dynamic toolsets, `sheet://` and `result://` resources, workflow prompts, and optional loopback `--control <port>` (`--window` shared desktop session, `--gui-only` policy; [`mcp.md`](../mcp.md)).
- **Biostatistics, nonlinear regression & graphing engine:** Structured table archetypes (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`, `Nested`), parametric and nonparametric comparisons, 1/2/3-way & Multifactor ANOVA, REML mixed models with Kenward–Roger / Satterthwaite *df*, multiplicity adjustments & Piepho (2004) Compact Letter Display, Kaplan–Meier & Cox PH survival, contingency tables, ROC, GLMs, PCA/clustering, MICE, Power & Sample Size, built-in and custom algebraic/implicit/ODE nonlinear regression, and vector/raster export (`SVG`, `%PDF-1.7`, `PNG`, `TIFF`, `.emf`, `CSV`, `TSV`, `JSON`).
- **Persistence & interchange:** Versioned `.curvebook` archives, `.curvebook.d/` unpacked directories, `.cbtemplate` templates, crash-recovery WAL (`ops.journal`), `CSV`/`TSV`, and bounded clean-room `.pzfx`/`.prism` project import ([`file-formats.md`](../file-formats.md)).
- **Zero telemetry:** Strictly offline calculation with zero analytics, crash uploaders, update pings, or background network egress ([Privacy Policy](https://hausergroup.github.io/curvebook/privacy.html), `cargo xtask telemetry`).

---

## 2. Beta Download Set & Platform Status (`R0.28`, `R8.10`, `R8.11`)

Beta ships one installer format per desktop OS and one file per supported architecture. Every installer includes both `curvebook` and `curvebook-cli`:

| Operating System | Beta Release File | Status & Caveats |
|---|---|---|
| **macOS (universal)** | `curvebook-<version>-macos-universal.dmg` | Signed with Apple Developer ID and notarized (`R8.10`). `curvebook-cli` is bundled inside `CurveBook.app/Contents/MacOS/` (no separate CLI `.zip` is shipped). |
| **Windows (`x64`, `arm64`)** | `curvebook-<version>-windows-x64.msi`, `curvebook-<version>-windows-arm64.msi` | **Unsigned** until post-Beta Authenticode signing (`R8.11`). Windows `x86` (32-bit) and `-portable.zip` are dropped (`R0.28`). |
| **Linux (`x86_64`, `aarch64`)** | `curvebook-<version>-linux-x86_64.AppImage`, `curvebook-<version>-linux-aarch64.AppImage` | Single-file `.AppImage` only (`.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz`, and Flatpak dropped; `R0.28`). Verified with `SHA256SUMS.txt` and Sigstore attestations (`R8.10`). |
| **FreeBSD (`x86_64`)** | `curvebook-<version>-freebsd-x86_64.tar.gz` | **Preview** desktop GUI and CLI archive (`R10.24` / `R8.5`). |
| **Web (`curvebook-web`)** | *Not shipped in Beta* | **Deferred to `1.0`** (`R0.28`, `R11`). |
| **Compliance & checksums** | `SHA256SUMS.txt`, `THIRD-PARTY-LICENSES.html` | Every asset carries a GitHub Sigstore build attestation verified before upload ([`verifying-downloads.md`](verifying-downloads.md)). |

### Platform-Specific Install & Upgrade Notes

- **Windows unsigned `.msi` (SmartScreen & Smart App Control until `R8.11`):**
  - Windows Authenticode signing is deferred until after Beta ([`R8.11`](../ROADMAP.md#r811-windows-authenticode-signing-after-beta)). After checking `SHA256SUMS.txt` and `gh attestation verify`, Windows Defender SmartScreen displays an *Unknown publisher* prompt: choose **More info → Run anyway** to install.
  - On Windows 11 machines with **Smart App Control** turned on, or managed institutional/lab computers that disallow unsigned software, Windows blocks unsigned `.msi` installers with no override option. Microsoft-signed Microsoft Store packaging (`.msix`) is tracked separately under [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).
  - **Upgrading from Alpha `x86` to Beta `x64`:** Windows `x86` builds are discontinued (`R0.28`). The Beta `x64` `.msi` shares the `UpgradeCode` used by Alpha (`R0.28`), though uninstall/folder-restoration behavior across the 32-bit-to-64-bit upgrade boundary has not yet been verified on a Windows runner (the `x64` package defaults to the current account under `%LOCALAPPDATA%\Programs\CurveBook\` rather than reading the 32-bit machine registry view). If an older Alpha `x86` entry remains in **Settings → Apps**, uninstall it manually.
- **Linux AppImage & FUSE:**
  - AppImages require FUSE (`libfuse2` or `fuse3`). On systems without FUSE enabled, extract or run via `--appimage-extract-and-run` (or `--appimage-extract` to access `squashfs-root/usr/bin/curvebook-cli`).
- **FreeBSD desktop preview:**
  - The FreeBSD tarball provides both `curvebook` and `curvebook-cli`, and the desktop application is labelled a **preview** (`R10.24`) because interactive FreeBSD hardware QA remains open in [`R8.5`](../ROADMAP.md#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) for `1.0`.

---

## 3. Known Limitations & Caveats in Beta

1. **15 non-English UI locales are machine-drafted `[Draft]` translations (`R10.23`):**
   - English (`en`) is authored directly. All 15 non-English UI locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`, `it`, `nl`, `pl`, `ru`, `tr`, `ar`, `he`) are **machine-drafted translations** pending native-speaker linguistic review ([`R10.21`](../planning/roadmap-history/r10.md#r1021-translations-for-new-ui-strings), [`localization-parity.md`](../localization-parity.md)) and display `[Draft]` in **View › Language**.
2. **Zero bundled font binaries:**
   - CurveBook embeds no `.ttf`, `.otf`, `.woff`, or `.woff2` files in the repository or default release packages (`cargo xtask assets`). The desktop app and `%PDF-1.7` exports rely on fonts installed on your OS (or supplied via `CURVEBOOK_FONTS_DIR` / `--ttf-font <path>`). Scripts lacking an installed host font (such as CJK, Arabic, or Hebrew on minimal Linux/BSD installations) fall back to substitute glyphs or Core-14 metrics until a matching OS font is installed.
3. **Browser bundle (`curvebook-web`) deferred to `1.0` (`R0.28`, `R11`):**
   - No `curvebook-web` ZIP archive is attached to Beta releases. Browser-host text layout, IME/soft-keyboard entry, save-in-place, `.curvebook.d/` folder write-back, per-document autosave isolation, off-main-thread execution, and cross-browser QA are tracked under [`R11`](../ROADMAP.md#r11-usable-browser-app) for **`1.0`**.
4. **Pending live Beta walkthrough & `1.0`-deferred desktop capabilities:**
   - Automated `HumanPathDriver` desktop suites cover the baseline workflows (`WF-01`–`WF-50`), while live interactive verification across the signed macOS Beta build (`R10.13`), Windows 11 `x64`/`arm64` and Linux X11/Wayland AppImage smoke checks (`R10.24`), and FreeBSD / physical printer driver QA (`R8.5`) are separate release-gate checks documented in [`STATUS.md`](../STATUS.md) and [`ROADMAP.md`](../ROADMAP.md).
   - Desktop capabilities scoped to **`1.0`** (`R13`) rather than Beta include: multi-card `Shift`/`Cmd` selection in the Portfolio Gallery and batch `Apply Style From`, one-step batch per-dataset graph generation, embedded schematic images on graphs/layouts, rectangular-slice live embedded result tables, non-exported canvas grid overlays, layout plot-scale equalization and transparent axis-free graph overlays, restricted-domain linear fit `Range` tabs, and `WF-51`–`WF-65` workflows.
   - `QOL-10`, `QOL-11`, `QOL-13` (statistical/Bayesian calculators), and `QOL-15` (molarity/dilution calculators) remain deferred without scheduled milestones ([`qol-beyond-parity.md`](../planning/qol-beyond-parity.md)).
5. **Breaking cutover & schema version safety (`R0.20`):**
   - Pre-cutover `.lumencraft`, `.lumencraft.d/`, `.lctemplate`, and `lumencraft.batch.v1` files are intentionally rejected with no converter ([`file-formats.md`](../file-formats.md#4-breaking-cutover-legacy-rejection--residual-name-allowlist)).
   - Project and preferences files written by a newer CurveBook schema version fail cleanly with an unsupported-version error and are never overwritten.

---

## 4. Beta GitHub Release Notes Template

Copy and adapt the template below when publishing a reviewed **`CurveBook Beta`** prerelease:

````markdown
## CurveBook Beta v<version>

**New to CurveBook?** See the **[README](https://github.com/HauserGroup/curvebook/blob/main/README.md)** to install the desktop app, or the **[Beginner's Guide](https://github.com/HauserGroup/curvebook/blob/main/docs/mcp-for-beginners.md)** to connect CurveBook to Claude Desktop or another local MCP client.

CurveBook is an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing application written in pure Rust (`MIT OR Apache-2.0`), with **zero telemetry** and **100% local calculation**.

### Which file do I need?
Every installer includes both the desktop app (`curvebook`) and the CLI/MCP binary (`curvebook-cli`):

| Operating System | Architecture | Download | Notes |
|---|---|---|---|
| **macOS** | Apple Silicon & Intel (`universal`) | `curvebook-<version>-macos-universal.dmg` | Signed & notarized; `curvebook-cli` is inside `CurveBook.app/Contents/MacOS/`. |
| **Windows 10/11** | `x64` / `arm64` | `curvebook-<version>-windows-x64.msi` or `-arm64.msi` | **Unsigned** until `R8.11` (SmartScreen **More info → Run anyway**; blocked by Smart App Control). `x86` and `-portable.zip` are dropped. |
| **Linux** | `x86_64` / `aarch64` | `curvebook-<version>-linux-x86_64.AppImage` or `-aarch64.AppImage` | Single-file AppImage only (`.deb`, `.rpm`, `.tar.gz`, Flatpak, and `.zsync` dropped). |
| **FreeBSD** | `x86_64` | `curvebook-<version>-freebsd-x86_64.tar.gz` | **Preview** desktop GUI & CLI archive. |
| **Browser (`curvebook-web`)** | WebAssembly | *Deferred to 1.0* | Not attached to Beta releases. |

### Beta notes & caveats
- **Desktop GUI:** The interactive desktop workspace (`curvebook`) is supported alongside `curvebook-cli` and `curvebook-cli mcp`.
- **Translations:** All 15 non-English UI languages carry `[Draft]` labels in **View › Language** pending native-speaker review.
- **Windows unsigned installers:** Beta `.msi` files are not Authenticode-signed yet; verify `SHA256SUMS.txt` and Sigstore provenance before running.
- **FreeBSD preview & browser deferral:** FreeBSD desktop is a preview build, and `curvebook-web` is deferred to `1.0`.
- **Full known-issues list & documentation:** [`docs/release/beta-known-issues.md`](https://github.com/HauserGroup/curvebook/blob/main/docs/release/beta-known-issues.md) · [Documentation](https://hausergroup.github.io/curvebook/manual/) · [Privacy Policy](https://hausergroup.github.io/curvebook/privacy.html)

### Verify your download
1. **GitHub Sigstore build provenance** (with `gh` signed in):
   ```sh
   gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook \
     --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml \
     --source-ref refs/tags/v<version>
   ```
2. **SHA-256 checksum:**
   - macOS: `shasum -a 256 --check --ignore-missing SHA256SUMS.txt`
   - Linux: `sha256sum --check --ignore-missing SHA256SUMS.txt`
   - Windows & FreeBSD: see [`docs/release/verifying-downloads.md`](https://github.com/HauserGroup/curvebook/blob/main/docs/release/verifying-downloads.md)
````
