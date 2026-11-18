# Releasing CurveBook

Releases are built and published by [.github/workflows/release.yml](../../.github/workflows/release.yml) when a version tag (`vX.Y.Z`) is pushed, with packaging metadata linted continuously by [.github/workflows/packaging-lint.yml](../../.github/workflows/packaging-lint.yml).

Single source of truth for the release version: `[workspace.package] version` in [Cargo.toml](../../Cargo.toml), updated via `cargo xtask version`.

## Alpha releases with the existing builder

Use the existing **Release** GitHub Actions workflow; Alpha is a manual option, not a separate build pipeline. The same jobs build macOS, Windows, Linux, WASM, and FreeBSD packages. MSIX remains deferred in [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).

Once the updated workflow is on GitHub's default branch:

1. Open the repository's **Actions → Release → Run workflow** and choose the branch to build.
2. Leave **version** empty to use `Cargo.toml`. If supplied, it must exactly match the workspace version; changing the release's display name does not change package versions or the `v<version>` tag.
3. Leave **alpha** checked (the manual default). macOS skips certificate import and uses the existing ad-hoc signing path without notarization. Windows receives no signing credentials and produces unsigned MSI/portable packages. Linux and WASM use their existing builders.
4. After all build jobs pass, follow the draft-release link in the run summary, or open **Releases**. The release is named **CurveBook Alpha v<version>**, marked **prerelease**, and kept as a **draft**, without becoming the latest stable release.
5. Review the files under the release's **Assets**: installers, native archives, Linux packages, the static WASM ZIP, `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt`. These are attached GitHub Release Assets, in addition to the temporary Actions artefacts. Draft assets require appropriate repository access; public distribution begins only when a maintainer publishes the reviewed prerelease.

Every package/disclosure asset is included in `SHA256SUMS.txt`, excluding the checksum file itself and GitHub's automatically generated source archives. The workflow verifies the manifest before attaching it. After downloading the listed files into one directory, verify with `sha256sum --check SHA256SUMS.txt` on Linux or `shasum -a 256 -c SHA256SUMS.txt` on macOS. On Windows, compare a download's `Get-FileHash -Algorithm SHA256 <path>` result with its manifest entry.

Rerunning a version updates its existing draft and replaces matching assets. The workflow refuses to modify a release that is already published. Alpha naming uses the same `v<version>` tag as the normal builder; use a new workspace version for a new publicly distributed build. When creating a new tag, the release targets the commit actually built. Uncheck **alpha** for the normal signing configuration; automatic branch/tag builds retain that normal mode. Scheduled builds still upload Actions artefacts only.

**Release channel.** [`packaging/release-channel.sh`](../../packaging/release-channel.sh) classifies the resolved version for every draft. Any SemVer pre-release version is drafted as a GitHub **prerelease** that never becomes **Latest**, whatever the trigger, and is named by its first pre-release identifier: `-alpha.N` → **CurveBook Alpha v…**, `-beta.N` → **CurveBook Beta v…**, `-rc.N` → **CurveBook Release Candidate v…**, anything else → **CurveBook Prerelease v…**. A stable version (`X.Y.Z`) is a normal release unless **alpha** is checked. To cut a release candidate, first set the workspace version (`cargo xtask version set 0.1.0-rc.1`) and commit it; the `v0.1.0-rc.1` tag must match it exactly, or the version-parity check fails. Regression test: `xtask/tests/r014_release_channel.rs`.

macOS testers may need **Privacy & Security → Open Anyway**; Windows testers may encounter unsigned-publisher warnings or organization restrictions. The web ZIP needs an HTTP(S) server and does not publish a live website. Package/signature checks are not evidence of full human GUI readiness. Before public Alpha launch, complete the identity and branding gates in [`R0.20`](../ROADMAP.md#r020-rename-lumencraft-to-curvebook) and [`R0.21`](../ROADMAP.md#r021-strict-alpha-branding-and-asset-provenance-gate) (see [Alpha branding clearance](alpha-branding-clearance.md) and [Alpha known issues & release notes](alpha-known-issues.md)).

GitHub documents [manual workflow runs](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow), [draft/prerelease creation with attached assets](https://cli.github.com/manual/gh_release_create), and [asset uploads](https://cli.github.com/manual/gh_release_upload).

The [prepared GitHub Pages site](../index.html) covers MCP onboarding, current status, and downloads. Follow its [publication checklist](github-pages.md) alongside the reviewed public Alpha; update its pending-launch wording only after the prerelease is published. GUI onboarding follows `R10` human acceptance, and a live browser demo requires browser QA separately.

---

## 1. Pre-Publish Package Verification

Before tagging or publishing a release, verify both source invariants and packaging outputs across target platforms:

1. **Workspace gates & licences:**
   - Bump version cleanly: `cargo xtask version set <X.Y.Z>` (check synced outputs with `git diff`).
    - Run the Zero-Telemetry & static security hygiene gate:
      ```sh
      cargo xtask telemetry
      ```
      This asserts zero telemetry/analytics/crash-uploader egress across workspace sources, strict GitHub workflow `permissions` / `${{ }}` hygiene, and no-egress web headers.
    - Verify clean-room workspace screenshots & `ASSETS.md` attribution:
      ```sh
      cargo xtask screenshots --check
      ```
      Use `cargo xtask screenshots` (without `--check`) to regenerate `docs/images/workspace-split-fit.png` and `docs/images/workspace-layout-dark.png`.
    - Run licence compliance & regenerate licence compendium:
      ```sh
      cargo xtask licenses
      ```
      This verifies crates offline against [docs/licences.md](../licences.md) (`deny.toml`), and runs `cargo deny check licenses` when installed (mandatory when `CI=true`); add `--html` to write `THIRD-PARTY-LICENSES.html` (`cargo about generate about.hbs` when `cargo-about` is installed, otherwise a built-in renderer). The release workflow's `licenses` job uses `cargo about` with the `cargo xtask licenses --html` fallback.
    - Run the full CI gate suite:
      ```sh
      cargo xtask ci
      ```
      Hosted CI currently fails on `main` at the `actionlint` install step of `.github/workflows/ci.yml` before any Rust gate runs (`R0.6`, open in [ROADMAP.md](../ROADMAP.md)), so a local `cargo xtask ci` pass is the only gate evidence until that is fixed.
2. **Cross-platform packaging integrity scripts:**
   - Shared packaging metadata / brand hygiene check (verifies `packaging/env.sh` values and rejects stale template placeholders):
     ```sh
     sh packaging/check-app-specific.sh
     ```
   - **macOS:** Build `.app`, `.dmg`, and `curvebook-cli-*.zip` (`packaging/macos/package.sh --arch aarch64` / `universal`), verify `Info.plist` + codesign (`cargo xtask bundle`), and run `verify.sh` on non-sandboxed macOS hosts / CI runners (`hdiutil create` requires disk-image creation outside restricted sandboxes):
     ```sh
     sh packaging/macos/verify.sh
     ```
   - **Linux:** Verify `.AppImage` startup / AppRun execution before upload:
     ```sh
     sh packaging/linux/apprun-test.sh
     ```
   - **Windows:** Verify Windows `.ico` completeness (`cargo xtask ico` if regenerating) and WiX installer policy:
     ```powershell
     pwsh packaging/windows/check-icons.ps1
     pwsh packaging/windows/check-install-policy.ps1
     ```

*(Notes: `packaging/macos/verify.sh`, `packaging/linux/apprun-test.sh`, and `packaging/windows/check-icons.ps1` live under `packaging/` as part of `R0.14`; run `packaging-lint.yml` cleanly before tagging, and `.github/workflows/release.yml` runs the `licenses` job via `cargo about` with `cargo xtask licenses --html` fallback to ship `THIRD-PARTY-LICENSES.html`.)*

---

## 2. Shipped Release Artefacts

Every tagged release distributes binaries and archives alongside licence disclosures and integrity checksums:

| Platform | Artefact / Format | Builder / Config | Notes |
| :--- | :--- | :--- | :--- |
| **macOS** | Signed/universal `.dmg` installer (`x86_64` + `aarch64` `.app`), CLI `.zip` | `packaging/macos/package.sh`, `Info.plist.in`, `entitlements.plist` | Verified via `cargo xtask bundle` / `package.sh` (`CurveBook.app`, `plutil -lint OK`, ad-hoc `codesign --verify --strict --deep`; pre-`R0.20` baseline `LumenCraft.app` arm64 build was `24M`) and `packaging/macos/verify.sh` on non-sandboxed macOS hosts (`hdiutil create` requires disk-image creation outside restricted sandboxes). |
| **Linux** | `.AppImage` + `.AppImage.zsync`, `.deb`, `.rpm`, portable `.tar.gz` | `packaging/linux/package.sh` | AppImage tested via `packaging/linux/apprun-test.sh`; includes desktop entry, AppStream metadata, and MIME associations. Requires `CURVEBOOK_MAINTAINER` for formats needing a maintainer contact (`.deb`/`.rpm`); `.tar.gz`/portable archives build without it. |
| **Linux (Flatpak / OCI)** | `.flatpak` bundle and GHCR OCI image (`org.curvebook.CurveBook`) | `packaging/linux/flatpak-bundle.sh`, `packaging/linux/flatpak-oci.sh`, `packaging/linux/flatpak/org.curvebook.CurveBook.yml` | Sandboxed desktop distribution matching `packaging/env.sh` App ID. |
| **Windows** | WiX v5 `.msi` installer and `-portable.zip` standalone archive | `packaging/windows/package.ps1`, `packaging/windows/INSTALLATION.md` | Dual-scope MSI (`PerUser` default, optional `PerMachine`); static MSVC CRT; checked via `check-icons.ps1` & `check-install-policy.ps1`. Fresh CurveBook upgrade/component identity leaves any existing LumenCraft install separate. |
| **FreeBSD** | `.tar.gz` binary archive | `packaging/freebsd/package.sh` | Desktop & headless CLI archive for FreeBSD builds. |
| **Web (WASM)** | Trunk `web-release` static `.zip` bundle | `packaging/web/package.sh`, `packaging/web/{_headers,.htaccess}` | Built targeting `wasm32-unknown-unknown` with relative (`./`) asset URLs and strict Zero-Telemetry CSP (`connect-src 'self'` for same-origin `.wasm` streaming only, zero external origins), COOP, and COEP headers; verified via `cargo xtask wasm` + `telemetry`. |
| **Compliance & Checksums** | **`THIRD-PARTY-LICENSES.html`**, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `ATTRIBUTION.md`, `SHA256SUMS.txt` | `cargo xtask licenses` (`about.toml`), `sha256sum`, & `actions/attest-build-provenance` | Bundled inside packages / release assets so every binary download includes complete third-party dependency & asset notices; `SHA256SUMS.txt` and all uploaded artefacts receive keyless GitHub Sigstore build provenance attestations in CI. |

### Verifying Downloaded Release Artefacts

Every file uploaded by the `publish` job in [`.github/workflows/release.yml`](../../.github/workflows/release.yml) — Linux (`.AppImage`, `.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz`, `.flatpak`), Windows (`.msi`, `-portable.zip`), macOS (`.dmg`, CLI `.zip`), FreeBSD (`.tar.gz`), web (`.zip`), `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt` itself — receives a keyless GitHub Sigstore build provenance attestation (`actions/attest-build-provenance`).

1. **Verify Sigstore build provenance** with the GitHub CLI (`gh`):
   ```sh
   gh attestation verify <file> --repo HauserGroup/curvebook
   ```
   This confirms that `<file>` (including `SHA256SUMS.txt` or any individual installer/archive) matches a build produced by `.github/workflows/release.yml` in `HauserGroup/curvebook`.
2. **Verify SHA-256 checksums** after downloading `SHA256SUMS.txt` alongside your release files:
   ```sh
   sha256sum -c SHA256SUMS.txt
   ```
   *(If you downloaded only one platform's files into the current directory, pass `sha256sum -c --ignore-missing SHA256SUMS.txt`; on macOS without GNU coreutils, use `shasum -a 256 -c SHA256SUMS.txt`.)*

**CI-only provenance & zero binary egress:** Keyless OIDC/Sigstore attestation calls (`id-token: write` and `attestations: write`, scoped strictly to the `publish` job in `.github/workflows/release.yml`) run exclusively in GitHub Actions during release publishing. Shipped CurveBook binaries (`curvebook`, `curvebook-cli`) and the WASM web bundle gain **zero** network access, update checks, or telemetry (`cargo xtask telemetry`).

---

## 3. App-Specific Packaging Notes

- **Canonical identity (`packaging/env.sh`):** All installers share App ID `org.curvebook.CurveBook`, repository `HauserGroup/curvebook`, binary/CLI slug `curvebook` / `curvebook-cli`, and project/template/interchange file associations (`.curvebook`, `.cbtemplate`, `.pzfx`, `.prism`). `packaging/check-app-specific.sh` must pass cleanly without template leftover strings.
- **Full breaking cutover & separate install identity (`R0.20`):** CurveBook is a full breaking cutover with zero LumenCraft file readers, executable/env aliases, settings/recovery migrations, or browser-storage migrations. Renaming `.lumencraft` to `.curvebook` does not convert old archives (`lumencraft-archive` is explicitly rejected). Fresh CurveBook installers use a fresh Windows installer identity and never claim old `.lumencraft` / `.lctemplate` associations or uninstall/upgrade an existing LumenCraft installation. See [`file-formats.md` §4](../file-formats.md#4-breaking-cutover-legacy-rejection--residual-name-allowlist) for the narrow residual-name allowlist.
- **Zero Telemetry & Web Privacy Headers:** CurveBook ships zero telemetry, analytics, crash uploaders, update-check pings, or external third-party cloud scanners (`cargo xtask telemetry`). Web deployments bundle `packaging/web/_headers` and `packaging/web/.htaccess` enforcing `default-src 'self'`, `connect-src 'self'` (same-origin `.wasm` streaming only, zero external origins), `Referrer-Policy: no-referrer`, `Permissions-Policy`, `Cross-Origin-Opener-Policy: same-origin`, and `Cross-Origin-Embedder-Policy: require-corp`.
- **Zero bundled fonts:** Packages do **not** embed binary fonts by default (`cargo xtask assets` rejects tracked `.ttf`/`.otf`/`.woff`/`.woff2`). Desktop releases probe OS font paths at runtime (`--ttf-font <path>` adds a face); `CURVEBOOK_FONTS_DIR` is read at build time (and for runtime overrides), and web tests using `bundled-fonts` require explicit local font staging (`cargo xtask web-fonts`) with matching licence notice rules ([docs/licences.md](../licences.md)).
- **macOS specifics:** `packaging/macos/Info.plist.in` registers UTIs/document handlers for `.curvebook`, `.cbtemplate`, `.pzfx`, and `.prism`; hardened runtime entitlements live in `packaging/macos/entitlements.plist`.
- **Linux specifics:** `packaging/linux/` installs desktop, AppStream, icon sizes, and MIME definitions consistently for `.AppImage`, `.deb`, `.rpm`, and Flatpak (`org.curvebook.CurveBook`). Package formats that require a maintainer contact address (such as `.deb` / `.rpm` via `nfpm`) require an explicit `CURVEBOOK_MAINTAINER` environment variable and fail early if it is unset—there is no fallback email address—while `.tar.gz`/portable archive builds remain usable without `CURVEBOOK_MAINTAINER`.
- **Windows specifics:** WiX v5 dual-scope MSI supports unprivileged per-user installation by default and elevated per-machine deployment ([packaging/windows/INSTALLATION.md](../../packaging/windows/INSTALLATION.md)). Both `.msi` and `-portable.zip` link the static CRT to avoid external redistributable prompts. CurveBook installs into a separate `CurveBook` directory (`INSTALLFOLDER`) with fresh upgrade/component GUIDs so it installs alongside any existing LumenCraft installation without uninstalling or upgrading it.
- **Windows signing and planned Store distribution:** [`windows-signing.md`](windows-signing.md) explains self-signing for trusted test machines without Azure, plus the Microsoft-signed MSIX Store route. MSIX packaging is planned in [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution); the current builder produces MSI and portable ZIP only. A self-signed MSI/EXE does not meet Store MSI/EXE signing requirements.
- **Web specifics:** Built via Trunk under `web-release` with relative root paths (`./`) and strict no-egress security headers (`packaging/web/{_headers,.htaccess}`) so bundles deploy cleanly on subpath hosts without rebuilding.

---

## 4. Closing Roadmap Items After Release

Once the release build passes verification and is published:

1. Confirm the release-gate table in [docs/ROADMAP.md](../ROADMAP.md) (`Legacy Milestone & Gate Mapping`): the **Agent-harness workflows** gate (formerly "Core workflows") covers the semantic `gui_only` agent suite only, and a `1.0` release additionally requires the **Human GUI workflows** gate (`R10`).
2. Mark all shipped `R<m>.<n>` items **Done** in [docs/ROADMAP.md](../ROADMAP.md) (moving milestone archival notes to `docs/planning/roadmap-history.md` if retiring a milestone phase).
3. Refresh and verify [docs/STATUS.md](../STATUS.md) and open-item summaries:
   ```sh
   cargo xtask status
   cargo xtask pending
   ```
4. Confirm release notes link only verified changes, `THIRD-PARTY-LICENSES.html` and `SHA256SUMS.txt` are published cleanly alongside installers, and open issues closed by `(R<m>.<n>)` PRs are updated.
