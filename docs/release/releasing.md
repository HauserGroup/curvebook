# Releasing CurveBook

Releases are built and published by [.github/workflows/release.yml](../../.github/workflows/release.yml) when a version tag (`vX.Y.Z`) is pushed, with packaging metadata linted continuously by [.github/workflows/packaging-lint.yml](../../.github/workflows/packaging-lint.yml).

Single source of truth for the release version: `[workspace.package] version` in [Cargo.toml](../../Cargo.toml), updated via `cargo xtask version`.

## Alpha and Beta releases with the release workflow

Use the **Release** GitHub Actions workflow (`release.yml`); Alpha and Beta are both built by the same pipeline and classified by SemVer pre-release identifiers (`packaging/release-channel.sh`). Store MSIX uses the separate **Windows Store package** workflow under [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).

1. Open the repository's **Actions → Release → Run workflow** and choose the branch to build (or push a matching `v<version>` tag for Beta/RC/stable releases).
2. Leave **version** empty to use `Cargo.toml`. If supplied, it must exactly match the workspace version.
3. Choose **alpha**:
   - **Checked (`true`, manual default):** builds an Alpha draft prerelease (`CurveBook Alpha v<version>`) using ad-hoc macOS signing without notarization and unsigned Windows `.msi` installers.
   - **Unchecked (`false`, or tag/branch push):** enforces **fail-closed Apple Developer ID signing and notarization** on macOS (`R8.10`). If any of `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, or `APPLE_TEAM_ID` is missing, the `macos` job stops with an error instead of falling back to ad-hoc output. Windows `.msi` installers remain **unsigned** for Beta until [`R8.11`](../ROADMAP.md#r811-windows-authenticode-signing-after-beta).
4. **Nightly schedule & single rehearsal run (`R8.11`):** scheduled dry runs skip the `macos` and `publish` jobs, guard all Windows signing secrets (`github.event_name != 'schedule'`), and upload Actions artifacts only. Run a single integrated rehearsal dispatch (e.g. `v0.1.0-alpha.2` after changes land on `main`) rather than separate per-branch hosted dry runs.
5. The `licenses` job verifies crate licences (`cargo deny`, `cargo about`, `cargo xtask licenses`), generates `THIRD-PARTY-LICENSES.html`, and builds a deterministic CycloneDX 1.5 JSON SBOM (`curvebook-<version>.cdx.json`) via CI-only `cargo-cyclonedx` (`packaging/sbom.sh`, validated immediately with `cargo xtask release-scan --no-checksums sbom`). Every platform build job (`linux`, `macos`, `windows`, and `freebsd`) runs `cargo xtask release-scan` and uploads a `scan-report-*` artifact. The `publish` job depends on all build jobs passing, combines the installers, `THIRD-PARTY-LICENSES.html`, and `curvebook-<version>.cdx.json`, generates `SHA256SUMS.txt`, runs `release-scan --expect-beta-set "$ver" --cross-os-reports dist/scan-reports` to verify exact Beta asset parity and native scan records, creates keyless Sigstore build provenance attestations (`actions/attest-build-provenance`) for all files plus CycloneDX SBOM attestations (`actions/attest-sbom`) bound to the platform packages, verifies every file's attestations, and drafts the GitHub Release.
6. After all jobs pass, review the draft release under **Releases**, paste the [Beta Release Notes Template](#beta-release-notes-template) for Beta cuts, and publish when ready. Draft assets require repository access; public distribution begins only when a maintainer publishes the reviewed release.

Rerunning a version updates its existing draft and replaces matching assets. The workflow refuses to modify a release that is already published before creating any attestations.

**Release channel.** [`packaging/release-channel.sh`](../../packaging/release-channel.sh) classifies the resolved version for every draft. Any SemVer pre-release version is drafted as a GitHub **prerelease** that never becomes **Latest**, whatever the trigger, and is named by its first pre-release identifier: `-alpha.N` → **CurveBook Alpha v…**, `-beta.N` → **CurveBook Beta v…**, `-rc.N` → **CurveBook Release Candidate v…**, anything else → **CurveBook Prerelease v…**. A stable version (`X.Y.Z`) is a normal release unless **alpha** is checked. To cut a release, first set the workspace version (`cargo xtask version set 0.1.0-beta.1`) and commit it; a pushed tag `v0.1.0-beta.1` must match it exactly, or the version-parity check fails. Regression test: `xtask/tests/r014_release_channel.rs`.

**Signing posture & user documentation (`R8.10` / `R8.11`):**
- **macOS (Beta+):** signed with Developer ID Application and notarized with stapled tickets on both `CurveBook.app` and the `.dmg`; historical Alpha prereleases were ad-hoc signed.
- **Windows (Beta):** `x64` and `arm64` `.msi` installers ship **unsigned** until `R8.11` (`x86` and `-portable.zip` were retired in `R0.28`; see [`windows-signing.md`](windows-signing.md) and [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution)).
- User-facing install steps, `PATH` setup for bundled CLI binaries, *"which file do I need?"*, and OS trust prompts live in [Get Started](https://curvebook.org/manual/latest/get-started.html) and [Troubleshooting](https://curvebook.org/manual/latest/troubleshooting.html#known-issues).

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
   - Verify (or refresh on UI changes) clean-room workspace screenshots & `ASSETS.md` attribution:
     ```sh
     cargo xtask screenshots --check
     # or regenerate all 6 PNGs on demand:
     cargo xtask screenshots
     ```
   - Run licence compliance & regenerate licence compendium:
     ```sh
     cargo xtask licenses
     ```
     This verifies crates offline against [docs/licences.md](../licences.md) (`deny.toml`), and runs `cargo deny check licenses` when installed (mandatory when `CI=true`); add `--html` to write `THIRD-PARTY-LICENSES.html`.
   - Verify the offline documentation site (`docs-site`), and when cutting a release, refresh screenshots, root redirect shims, and `404.html`, and verify the generated manual plus `_root/{sitemap.xml,llms.txt,llms-full.txt}` build in `target/docs-manual/` (manual HTML and SEO/LLM outputs are built purely in CI by `.github/workflows/pages.yml` and are not committed under `docs/`):
     ```sh
     cargo run -p xtask --features docs-site -- docs-site --check
     cargo run -p xtask --features docs-site -- docs-site --release <version> --refresh-screenshots --check
     ```
   - Run the default CI gate suite:
     ```sh
     cargo xtask ci
     ```
2. **Mandatory `release-scan` asset verification (`R0.26` / `R0.28`):**
   - Every release build **must** pass `cargo xtask release-scan` before drafting a GitHub Release:
     ```sh
     # Per-platform build check (no combined SHA256SUMS.txt yet):
     cargo run --locked -p xtask -- release-scan --no-checksums --report dist/scan-report-<platform>.json dist/release

     # Full Beta release-directory check in publish (verifies SHA256SUMS.txt, exact Beta set,
     # native Linux/FreeBSD/license containers, and matching macOS/Windows native scan reports):
     cargo run --locked -p xtask -- release-scan \
       --expect-beta-set "$ver" \
       --cross-os-reports dist/scan-reports \
       --report dist/release-scan-publish.json \
       dist/release
     ```
   - `release-scan` unpacks containers through bounded `curvebook-platform` subprocesses (`.zip`, `.tar.gz`/`.tgz`, `.AppImage`, `.dmg` on macOS, `.msi` on Windows, plus historical `.deb`/`.rpm`/`.flatpak`/`.zsync` for Alpha backfill), validates CycloneDX SBOM JSON (`.cdx.json` with `"bomFormat": "CycloneDX"`, non-empty `specVersion` and `components`, plus `brands`, `cleanroom`, and `secrets` scans), enforces zip-slip/tar-slip guards, fails closed on unknown or unextractable formats, checks `SHA256SUMS.txt`, and runs `brands`, `cleanroom`, and `assets` over every unpacked entry.
   - Backfilling `v0.1.0-alpha.1` requires downloading its published assets from GitHub (see [alpha-branding-clearance.md](../history/release/alpha-branding-clearance.md#6-hosted-release-asset-scan-r026)).
3. **Cross-platform packaging integrity scripts:**
   - Shared packaging metadata / brand hygiene check:
     ```sh
     sh packaging/check-app-specific.sh
     ```
   - **macOS:** Build and verify the universal `.dmg` (`packaging/macos/package.sh --arch universal --require-signing` for non-Alpha releases) and run:
     ```sh
     sh packaging/macos/verify.sh --arch universal --require-notarized
     ```
     On local developer builds without Apple credentials, omit `--require-signing` and `--require-notarized` for ad-hoc verification (`cargo xtask bundle` also checks `Info.plist` + ad-hoc codesign).
   - **Linux:** Verify `.AppImage` startup / AppRun execution before upload:
     ```sh
     sh packaging/linux/apprun-test.sh
     ```
   - **Windows:** Verify Windows `.ico` completeness (`cargo xtask ico` if regenerating) and WiX v5 installer policy (`x64` and `arm64`):
     ```powershell
     pwsh packaging/windows/check-icons.ps1
     pwsh packaging/windows/test-install-policy.ps1 -OutputDirectory ./target/install-policy-tests
     ```

---

## 2. Shipped Beta Release Artefacts (`R0.28`)

Beta releases ship one installer format per OS and one file per architecture (desktop installers bundle both `curvebook` and `curvebook-cli`; FreeBSD ships a Tier 2 CLI-only archive), plus compliance notices and checksums:

| Platform | Asset Filename | Builder / Config | Notes |
| :--- | :--- | :--- | :--- |
| **macOS** | `curvebook-<version>-macos-universal.dmg` | `packaging/macos/package.sh`, `Info.plist.in`, `entitlements.plist` | Universal `arm64` + `x86_64` DMG containing `CurveBook.app` (with `curvebook-cli` inside `Contents/MacOS/`). Signed with Developer ID + hardened runtime, notarized, and stapled (`R8.10`; non-Alpha runs fail closed if credentials are missing). Separate CLI `.zip` dropped (`R0.28`). |
| **Windows (`x64`, `arm64`)** | `curvebook-<version>-windows-x64.msi`, `curvebook-<version>-windows-arm64.msi` | `packaging/windows/package.ps1`, `curvebook.wxs`, `install-policy.wxi`, [`INSTALLATION.md`](../../packaging/windows/INSTALLATION.md) | Dual-scope WiX v5 MSI (`PerUser` default, optional `PerMachine`) with static MSVC CRT bundling `curvebook.exe` + `curvebook-cli.exe`. Ships **unsigned** for Beta until `R8.11`. `x86` and `-portable.zip` dropped (`R0.28`). |
| **Linux (`x86_64`, `aarch64`)** | `curvebook-<version>-linux-x86_64.AppImage`, `curvebook-<version>-linux-aarch64.AppImage` | `packaging/linux/package.sh`, `apprun-test.sh` | Self-contained AppImage bundling `curvebook`, `curvebook-cli`, `.desktop`, AppStream metadata, MIME associations, and icons. `.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz`, and Flatpak/OCI dropped (`R0.28`). |
| **FreeBSD (`x86_64`)** | `curvebook-<version>-freebsd-x86_64.tar.gz` | `packaging/freebsd/package.sh` | Tier 2 CLI-only tarball (`curvebook-cli` + MCP; no desktop GUI) built on FreeBSD VM and scanned on Ubuntu host. |
| **Compliance, SBOM & Checksums** | `curvebook-<version>.cdx.json`, `THIRD-PARTY-LICENSES.html`, `SHA256SUMS.txt` | `packaging/sbom.sh` (`cargo-cyclonedx`), `cargo xtask licenses`, `sha256sum`, `actions/attest-build-provenance`, `actions/attest-sbom` | `curvebook-<version>.cdx.json` is a deterministic, path-normalized CycloneDX 1.5 JSON SBOM covering `curvebook` and `curvebook-cli` across all Cargo targets (`R14.2`, CI-only tool, never a workspace dependency). Every asset in the Beta set receives a keyless GitHub Sigstore build provenance attestation, the six platform packages also receive a CycloneDX SBOM attestation (`https://cyclonedx.org/bom`), and all files are verified by `release-scan --expect-beta-set`. |

**Browser bundle (`curvebook-web`, deferred to `1.0`).** `curvebook-web-<version>.zip` is omitted from Beta GitHub Release assets (`R0.28`, [`R11`](../ROADMAP.md#r11-usable-browser-app)). Automatic WASM verification is paused in both CI and the release workflow. To verify a browser bundle on demand, build it with `./packaging/web/package.sh` and run `CURVEBOOK_REQUIRE_WEB_BUNDLE=1 cargo xtask web-bundle` to check the `R10.22` size budget.

### Beta Release Notes Template

Use this template when editing a drafted Beta GitHub Release (`v<version>`) before publishing:

```markdown
## CurveBook v<version> (Beta)

**Beta prerelease:** Intended for evaluation and workflow testing; back up important work before opening files.

- **Install, "which file do I need?", and download verification:** [Get Started](https://curvebook.org/manual/latest/get-started.html)
- **Known issues and platform status:** [Troubleshooting & Known Issues](https://curvebook.org/manual/latest/troubleshooting.html#known-issues)

### Highlights

- <Verified changes in this Beta cut>

### Release Assets & Signatures

- **macOS (`curvebook-<version>-macos-universal.dmg`):** Developer ID signed and notarized (`CurveBook.app` bundles `curvebook-cli` inside `Contents/MacOS/`).
- **Windows (`curvebook-<version>-windows-x64.msi`, `curvebook-<version>-windows-arm64.msi`):** Ships **unsigned** until `R8.11`; verify `SHA256SUMS.txt` and the GitHub Sigstore attestation before running.
- **Linux (`curvebook-<version>-linux-x86_64.AppImage`, `curvebook-<version>-linux-aarch64.AppImage`):** Self-contained desktop + CLI downloads; verify via `SHA256SUMS.txt` and `gh attestation verify`.
- **FreeBSD (`curvebook-<version>-freebsd-x86_64.tar.gz`):** Tier 2 CLI-only archive (`curvebook-cli` + MCP; no desktop GUI); verify via `SHA256SUMS.txt` and `gh attestation verify`.
- **Notices, SBOM & Checksums:** `curvebook-<version>.cdx.json` (CycloneDX 1.5 SBOM), `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt`.
```

### Verifying Downloaded Release Artefacts

User-facing verification steps live only in the [Get Started verification section](../reference/get-started.md#verifying-downloads), published as [Get Started](https://curvebook.org/manual/latest/get-started.html). The `publish` job runs `gh attestation verify` on every file in the `R0.28` Beta asset set before upload and stops the release if any file fails.

1. **Verify Sigstore build provenance (and CycloneDX SBOM attestation)** with the GitHub CLI (`gh`):
   ```sh
   gh attestation verify <file> --repo HauserGroup/curvebook \
     --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
   ```
   To verify the CycloneDX SBOM attestation bound to a release package (`.dmg`, `.msi`, `.AppImage`, `.tar.gz`):
   ```sh
   gh attestation verify <package-file> --repo HauserGroup/curvebook \
     --predicate-type https://cyclonedx.org/bom \
     --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
   ```
   Without `--signer-workflow`, `gh` accepts an attestation signed by any workflow in the repository. A release built from a tag push also records the tag; check it with `--source-ref refs/tags/v<version>`. A manually dispatched run records the branch it ran from instead; for example, `v0.1.0-alpha.1` was built from `refs/heads/main`.

   The `publish` job refuses to modify an already-published release before it attests anything, so a rerun cannot create attestations for rebuilt files that are never uploaded. To regenerate the CycloneDX SBOM locally (with `cargo-cyclonedx` installed out-of-tree), run `./packaging/sbom.sh sbom`.
2. **Verify SHA-256 checksums** after downloading `SHA256SUMS.txt` alongside your release files:
   ```sh
   sha256sum -c SHA256SUMS.txt
   ```
   *(If you downloaded only one platform's files into the current directory, pass `sha256sum -c --ignore-missing SHA256SUMS.txt`; on macOS without GNU coreutils, use `shasum -a 256 -c SHA256SUMS.txt`.)*

**CI-only provenance & zero binary egress:** Keyless OIDC/Sigstore attestation calls (`id-token: write` and `attestations: write`, scoped strictly to the `publish` job in `.github/workflows/release.yml`) run exclusively in GitHub Actions during release publishing. Shipped CurveBook binaries (`curvebook`, `curvebook-cli`) and the WASM web bundle gain **zero** network access, update checks, or telemetry (`cargo xtask telemetry`).

### Why Linux and FreeBSD Downloads Have No Separate Signature File

Neither the AppImage runtime nor FreeBSD checks a signature when you run a downloaded file, so a signature only helps people who check it by hand. The release attestation already gives that check. It ties each file to the release workflow, uses no long-lived signing key that could leak, and records every signature in a public log.

A separate GPG or Minisign signature would only add protection if its key were kept off GitHub. Kept as a GitHub secret, it can be used by anyone who can run the release workflow, which is the same group that can produce an attestation. An offline key would need manual signing on every release and its own key management, and would add one more file per download. We will revisit this if a downstream packager needs a signing key, if an update channel that checks signatures is added, or if releases move off GitHub. This decision is recorded under `R8.10` in the [roadmap](../ROADMAP.md).

### Repository Settings That Protect Release Provenance

An attestation names the source commit and ref a file was built from. If someone moves or deletes a release tag afterwards, the tag no longer matches what users can verify. These settings belong to the repository, not to the workflow, so maintainers configure them in GitHub. **Settings → Rules → Rulesets → New tag ruleset**:

| Field | Value | Why |
|---|---|---|
| Name | `Release tags` | |
| Enforcement status | **Active** | A disabled ruleset applies no rules. |
| Bypass list | Empty | Admins can still edit or turn off the ruleset, and the change appears in the audit log. |
| Target tags | Include by pattern `v*` | Covers every `v<version>` release tag. |
| Restrict updates | On | Nobody can repoint a published tag to a different commit. |
| Restrict deletions | On | Nobody can delete a published tag. |
| Block force pushes | On | Closes the force-push path to moving a tag. |
| Restrict creations | **Off** | Releases create tags through `git push` and `gh release create --target` (manual Alpha runs). Restricting creation would block both for anyone not on the bypass list. |

Leave the other rules off; they apply to branches, not release tags. Also turn on **Settings → General → Releases → immutable releases**, so a published release's assets cannot be replaced.

To check the ruleset from a terminal:

```sh
gh api repos/HauserGroup/curvebook/rulesets --jq '.[] | select(.target == "tag") | [.name, .enforcement] | @tsv'
```

---

## 3. App-Specific Packaging Notes

- **Canonical identity (`packaging/env.sh`):** All installers share App ID `org.curvebook.CurveBook`, repository `HauserGroup/curvebook`, binary/CLI slug `curvebook` / `curvebook-cli`, and project/template/interchange file associations (`.curvebook`, `.cbtemplate`, `.pzfx`, `.prism`). `packaging/check-app-specific.sh` must pass cleanly without template leftover strings.
- **Full breaking cutover & separate install identity (`R0.20`):** CurveBook is a full breaking cutover with zero legacy file readers, executable/env aliases, settings/recovery migrations, or browser-storage migrations. Fresh CurveBook installers use a fresh Windows installer identity and never claim legacy associations or uninstall/upgrade earlier pre-cutover software. See [`file-formats.md` §4](../file-formats.md#4-breaking-cutover-legacy-rejection--residual-name-allowlist) for the narrow residual-name allowlist.
- **Zero Telemetry & Web Privacy Headers:** CurveBook ships zero telemetry, analytics, crash uploaders, update-check pings, or external third-party cloud scanners (`cargo xtask telemetry`). Web deployments bundle `packaging/web/_headers` and `packaging/web/.htaccess` enforcing `default-src 'self'`, `connect-src 'self'` (same-origin `.wasm` streaming only, zero external origins), `Referrer-Policy: no-referrer`, `Permissions-Policy`, `Cross-Origin-Opener-Policy: same-origin`, and `Cross-Origin-Embedder-Policy: require-corp`.
- **Zero bundled fonts:** Packages do **not** embed binary fonts by default (`cargo xtask assets` rejects tracked `.ttf`/`.otf`/`.woff`/`.woff2`). Desktop releases probe OS font paths at runtime (`--ttf-font <path>` adds a face); `CURVEBOOK_FONTS_DIR` is read at build time (and for runtime overrides), and web tests using `bundled-fonts` require explicit local font staging (`cargo xtask web-fonts`) with matching licence notice rules ([docs/licences.md](../licences.md)).
- **macOS specifics:** `packaging/macos/Info.plist.in` registers UTIs/document handlers for `.curvebook`, `.cbtemplate`, `.pzfx`, and `.prism`; hardened runtime entitlements live in `packaging/macos/entitlements.plist`; [Get Started](https://curvebook.org/manual/latest/get-started.html) (`docs/reference/get-started.md`) covers installation and adding `CurveBook.app/Contents/MacOS/curvebook-cli` to `PATH`.
- **Linux specifics:** `packaging/linux/package.sh` builds `.AppImage` (`x86_64`, `aarch64`) embedding `.desktop`, AppStream metainfo, hicolor icons, and MIME definitions (`org.curvebook.CurveBook`), verified via `desktop-file-validate`, `appstreamcli validate`, and `packaging/linux/apprun-test.sh`.
- **Windows specifics:** WiX v5 dual-scope MSI supports unprivileged per-user installation by default and elevated per-machine deployment ([packaging/windows/INSTALLATION.md](../../packaging/windows/INSTALLATION.md)) on `x64` and `arm64`, linking the static CRT to avoid external redistributable prompts.
- **Windows signing and Store distribution:** [`windows-signing.md`](windows-signing.md) explains why Beta `.msi` downloads ship unsigned until `R8.11`, self-signing for trusted test machines without Azure, and the unsigned x64 MSIX route built by the separate **Windows Store package** workflow (`R8.9`).
- **Web specifics:** Built via Trunk under `web-release` with relative root paths (`./`) and strict no-egress security headers (`packaging/web/{_headers,.htaccess}`), size-checked in CI (`CURVEBOOK_REQUIRE_WEB_BUNDLE=1`), and deferred from GitHub Release asset attachment until `1.0`.

---

## 4. Closing Roadmap Items After Release

Once the release build passes verification and is published:

1. Confirm the release-gate table in [docs/ROADMAP.md](../ROADMAP.md) (`Legacy Milestone & Gate Mapping`).
2. Mark all shipped `R<m>.<n>` items **Done** in [docs/ROADMAP.md](../ROADMAP.md) (moving milestone archival notes to `docs/history/roadmap.md` if retiring a milestone phase).
3. Refresh and verify [docs/STATUS.md](../STATUS.md) and open-item summaries:
   ```sh
   cargo xtask status
   cargo xtask pending
   ```
4. Confirm release notes link only verified changes, `curvebook-<version>.cdx.json`, `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt` are published cleanly alongside installers, and open issues closed by `(R<m>.<n>)` PRs are updated.
5. Tell reporters their fix shipped. List the issues completed since the previous release tag and add a short comment to each naming the new release (for example *"Included in `v0.1.0-alpha.2`."*):
   ```sh
   gh issue list --state closed --search "reason:completed closed:>=<previous tag date, YYYY-MM-DD>" --limit 200
   ```
   Skip issues whose fixing commit is not in the release (check with `git merge-base --is-ancestor <sha> <new tag>`).
