# CurveBook Alpha Known Issues & Release Scope (`v0.1.0-alpha.2`)

> **Release tier:** MCP Alpha (`v0.1.0-alpha.2`, prerelease) · **Primary surface:** `curvebook-cli` & `curvebook-cli mcp` (stdio JSON-RPC 2.0) · **Onboarding:** [`mcp-for-beginners.md`](../mcp-for-beginners.md) · **Branding & provenance clearance:** [`alpha-branding-clearance.md`](alpha-branding-clearance.md)

This document records the supported functional scope, known limitations, platform signing caveats, and GitHub Release notes template for CurveBook's public Alpha launch ([`R0.20`](../planning/roadmap-history/r0.md#r020-rename-lumencraft-to-curvebook), [`R0.21`](../planning/roadmap-history/r0.md#r021-strict-alpha-branding-and-asset-provenance-gate), [`R0.14`](../ROADMAP.md#r014-release-verification-scripts--readme-scaffold)).

---

## 1. What Works in Alpha (`curvebook-cli` & `curvebook-cli mcp`)

Alpha delivers full Model Context Protocol (MCP) and command-line access to CurveBook's implemented statistical, curve-fitting, graphing, and persistence engine:

- **Stdio MCP server (`curvebook-cli mcp`):** Newline-delimited JSON-RPC 2.0 and `Content-Length` framing over stdin/stdout, `--workdir <dir>` path containment (`WorkdirGuard`), `10` core tools + progressive toolsets (`97` typed `CommandSpec` commands across `9` groups + `45` `UI_COMMANDS`), `sheet://` and `result://` resources, workflow prompts, structured `isError` diagnostics, and opt-in `curvebook --control <port> --window` live session sharing ([`mcp.md`](../mcp.md)).
- **Biostatistics, nonlinear regression & graphing engine:** `8` structured table archetypes, `=` Calculated Variables, parametric/nonparametric group comparisons, 1/2/3-way & Multifactor ANOVA, REML mixed models, multiplicity & Piepho (2004) Compact Letter Display, Kaplan–Meier & Cox PH survival, contingency tables, ROC, GLMs, PCA/clustering, MICE, `8`-family Power & Sample Size, `117` built-in nonlinear regression equations + custom algebraic/implicit/ODE DSL, and publication exports (`SVG`, `%PDF-1.7`, `PNG`, `TIFF`, `.emf`).
- **Persistence & interchange:** Versioned `.curvebook` archives, `.curvebook.d/` unpacked directories, `.cbtemplate` templates, crash-recovery WAL journals (`ops.journal`), `curvebook.batch.v1` headless batch manifests, `CSV`/`TSV`, and bounded clean-room `.pzfx`/`.prism` project import ([`file-formats.md`](../file-formats.md)).
- **Zero telemetry:** Strictly offline execution with zero analytics, crash uploaders, update pings, or runtime network egress (`cargo xtask telemetry`).

---

## 2. Known Limitations & Caveats in `v0.1.0-alpha.2`

1. **Point-and-click desktop & browser GUI is pre-alpha (`R10`):**
   - Human-operable GUI completion (`R10`, accepted through [`R10.13`](../ROADMAP.md#r1013-human-operable-50-workflow-acceptance--manual-qa)) is not required or claimed for Alpha. As of 2026-11-18 the desktop GUI is a Beta requirement (`R10.13` desktop walkthrough, `R10.23`, `R10.24`); the browser app remains a `1.0` requirement. Treat the desktop window (`curvebook`) and WebAssembly bundle (`curvebook-web`) as experimental previews; use `curvebook-cli mcp` or `curvebook-cli` for Alpha workflows.
   - **Browser bundle limits (from a code review, not yet confirmed in a browser):** IME and on-screen keyboard text entry are not expected to work, **Save** always downloads a new file (or asks "Save as") instead of saving in place, opened `.curvebook.d/` folders are not written back, and all projects share one autosave slot that restores without a prompt. Long analyses block the page while they run. These are tracked in [`R11`](../ROADMAP.md#r11-usable-browser-app).
2. **Unsigned Windows & ad-hoc signed macOS packages (macOS signing is the Beta gate [`R8.10`](../ROADMAP.md#r810-signed-macos-installers--download-verification-beta-gate); Windows signing follows Beta in [`R8.11`](../ROADMAP.md#r811-windows-authenticode-signing-after-beta); Store packaging in `R8.9`):**
   - **macOS (`.dmg`, CLI `.zip`):** Built with ad-hoc code signatures (`MACOS_SIGN_IDENTITY="-"`) and no Apple Developer ID notarization. macOS Gatekeeper will prompt on first launch; use **System Settings → Privacy & Security → Open Anyway** (or clear quarantine on an extracted CLI archive after verifying `SHA256SUMS.txt`).
   - **Windows (`.msi`, `-portable.zip`):** Built without Authenticode/Azure code signing in Alpha mode. Windows SmartScreen or enterprise policies may display an unknown-publisher prompt (see [`windows-signing.md`](windows-signing.md)). Microsoft Store MSIX packaging remains planned under [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).
   - **Linux (`.deb` / `.rpm` maintainer variable):** `.AppImage`, `.tar.gz`, and `.flatpak` build unconditionally in CI; `.deb` and `.rpm` packages are emitted only when the GitHub Actions repository variable `CURVEBOOK_MAINTAINER` (`Name <email>`) is configured on `HauserGroup/curvebook`.
   - **Web (`curvebook-web-*.zip`):** Static bundle requiring an HTTP(S) server with the bundled `_headers` / `.htaccess` CSP/COOP/COEP headers (`HOSTING.md`); opening `index.html` over `file://` is unsupported by browsers.
3. **Full breaking format cutover (`R0.20`):**
   - Pre-cutover `.lumencraft`, `.lumencraft.d/`, `.lctemplate`, `lumencraft-archive`, and `lumencraft.batch.v1` files are intentionally rejected with no compatibility reader or migration tool. Renaming file extensions does not convert old archives ([`file-formats.md` §4](../file-formats.md#4-breaking-cutover-legacy-rejection--residual-name-allowlist)).
4. **Evolving interfaces & pre-`1.0` audit/QA tracks:**
   - Command parameter schemas, MCP toolset layouts, and persisted schema versions may change between Alpha builds (newer file versions are refused cleanly rather than overwritten).
   - Cross-OS hardware VM walkthroughs (Windows, Linux X11/Wayland, FreeBSD, live browser QA in `R8.5`) and hosted GitHub Actions CI matrix runs (`R0.6` / `R0.24`) remain open, alongside the deferred `R9.10`/`R9.11`/`R9.13`/`R9.15` quality-of-life items. The `R3.6`, `R3.7`, and `R6.3` extensions and all `R0.25` review fixes are included from `v0.1.0-alpha.2`. See [`STATUS.md`](../STATUS.md) and [`ROADMAP.md`](../ROADMAP.md) for current verification records.

---

## 3. GitHub Draft Prerelease Notes Template

Copy and adapt the summary below when publishing the reviewed **`CurveBook Alpha v0.1.0-alpha.2`** draft prerelease created by [`.github/workflows/release.yml`](../../.github/workflows/release.yml) (keep **Set as a pre-release** checked and **Set as the latest release** unchecked):

```markdown
## CurveBook Alpha v0.1.0-alpha.2 (MCP & CLI Prerelease)

**New to CurveBook?** Start with the **[Beginner's Guide to using CurveBook with your AI assistant](https://github.com/HauserGroup/curvebook/blob/main/docs/mcp-for-beginners.md)**.

CurveBook is an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing engine written in Rust (`MIT OR Apache-2.0`), with **zero telemetry** and **100% local calculation**.

### Which file should I download?
For connecting Claude Desktop or another local MCP client, download the package containing `curvebook-cli` (`curvebook-cli.exe` on Windows):
- **macOS (Apple Silicon & Intel):** `curvebook-cli-0.1.0-alpha.2-macos-universal.zip` *(or `curvebook-0.1.0-alpha.2-macos-universal.dmg`)*
- **Windows (`x64`, `arm64`, `x86`):** `curvebook-0.1.0-alpha.2-windows-x64-portable.zip` *(or `.msi` installer)*
- **Linux (`x86_64`, `aarch64`):** `curvebook-0.1.0-alpha.2-linux-x86_64.tar.gz` *(plus `.AppImage`, `.flatpak`, and `.deb`/`.rpm` when configured)*
- **FreeBSD (`x86_64`):** `curvebook-0.1.0-alpha.2-freebsd-x86_64.tar.gz`

### What's new since v0.1.0-alpha.1
- **Statistics:** mixed-effects REML for one-way and three-way repeated measures with selectable Kenward–Roger or Satterthwaite *df* (`R3.6`); exact Cox tie handling, per-risk-set stabilisation, and Baptista–Pike odds-ratio intervals for large tables (`R3.7`).
- **Formulas:** window clauses (`OVER`, `WHERE`, `ORDER BY`, `ROWS`), more conditional aggregates, and derived formulas in Select & Transform (`R6.3`).
- **Fixes:** the remaining milestone-review findings across the engine, statistics, fitting, MCP, and web host (`R0.25`).
- **Releases:** candidate and Alpha tags are always published as prereleases (`R0.14`).

### Early-release notes & OS security prompts
- **MCP Alpha scope:** The stdio MCP server (`curvebook-cli mcp`) and CLI runner are ready for testing. The point-and-click desktop and browser GUI (`R10`) is still under active development and is **not** complete in this Alpha.
- **macOS Gatekeeper:** Alpha builds use ad-hoc signing without Apple notarization. You may need **System Settings → Privacy & Security → Open Anyway** on first run.
- **Windows SmartScreen:** Alpha installers and binaries are unsigned and may show an unknown-publisher warning.
- **Full known-issues list & status:** [`docs/release/alpha-known-issues.md`](https://github.com/HauserGroup/curvebook/blob/main/docs/release/alpha-known-issues.md) · [`docs/STATUS.md`](https://github.com/HauserGroup/curvebook/blob/main/docs/STATUS.md)

### Verify your download
1. **SHA-256 checksum:** Download `SHA256SUMS.txt` alongside your package and run `shasum -a 256 -c --ignore-missing SHA256SUMS.txt` (macOS) or `sha256sum -c --ignore-missing SHA256SUMS.txt` (Linux).
2. **GitHub Sigstore build provenance:**
   `gh attestation verify <downloaded-file> --repo HauserGroup/curvebook`
```
