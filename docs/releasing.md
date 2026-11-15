# Releasing LumenCraft

Releases are built and published by [.github/workflows/release.yml](../.github/workflows/release.yml) when a version tag (`vX.Y.Z`) is pushed, with packaging metadata linted continuously by [.github/workflows/packaging-lint.yml](../.github/workflows/packaging-lint.yml).

Single source of truth for the release version: `[workspace.package] version` in [Cargo.toml](../Cargo.toml), updated via `cargo xtask version`.

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
      This verifies crates offline against [docs/licences.md](licences.md) (`deny.toml`), and runs `cargo deny check licenses` when installed (mandatory when `CI=true`); add `--html` to write `THIRD-PARTY-LICENSES.html` (`cargo about generate about.hbs` when `cargo-about` is installed, otherwise a built-in renderer). The release workflow's `licenses` job uses `cargo about` with the `cargo xtask licenses --html` fallback.
    - Run the full CI gate suite:
      ```sh
      cargo xtask ci
      ```
      Hosted CI currently fails on `main` at the `actionlint` install step of `.github/workflows/ci.yml` before any Rust gate runs (`R0.6`, open in [ROADMAP.md](ROADMAP.md)), so a local `cargo xtask ci` pass is the only gate evidence until that is fixed.
2. **Cross-platform packaging integrity scripts:**
   - Shared packaging metadata / brand hygiene check (verifies `packaging/env.sh` values and rejects stale template placeholders):
     ```sh
     sh packaging/check-app-specific.sh
     ```
   - **macOS:** Build `.app`, `.dmg`, and `lumencraft-cli-*.zip` (`packaging/macos/package.sh --arch aarch64` / `universal`), verify `Info.plist` + codesign (`cargo xtask bundle`), and run `verify.sh` on non-sandboxed macOS hosts / CI runners (`hdiutil create` requires disk-image creation outside restricted sandboxes):
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
| **macOS** | Signed/universal `.dmg` installer (`x86_64` + `aarch64` `.app`), CLI `.zip` | `packaging/macos/package.sh`, `Info.plist.in`, `entitlements.plist` | Verified via `cargo xtask bundle` / `package.sh` (`LumenCraft.app` `24M` arm64 build, `plutil -lint OK`, ad-hoc `codesign --verify --strict --deep`) and `packaging/macos/verify.sh` on non-sandboxed macOS hosts (`hdiutil create` requires disk-image creation outside restricted sandboxes). |
| **Linux** | `.AppImage` + `.AppImage.zsync`, `.deb`, `.rpm`, portable `.tar.gz` | `packaging/linux/package.sh` | AppImage tested via `packaging/linux/apprun-test.sh`; includes desktop entry, AppStream metadata, and MIME associations. |
| **Linux (Flatpak / OCI)** | `.flatpak` bundle and GHCR OCI image (`org.lumencraft.LumenCraft`) | `packaging/linux/flatpak-bundle.sh`, `packaging/linux/flatpak-oci.sh`, `packaging/linux/flatpak/org.lumencraft.LumenCraft.yml` | Sandboxed desktop distribution matching `packaging/env.sh` App ID. |
| **Windows** | WiX v5 `.msi` installer and `-portable.zip` standalone archive | `packaging/windows/package.ps1`, `packaging/windows/INSTALLATION.md` | Dual-scope MSI (`PerUser` default, optional `PerMachine`); static MSVC CRT; checked via `check-icons.ps1` & `check-install-policy.ps1`. |
| **FreeBSD** | `.tar.gz` binary archive | `packaging/freebsd/package.sh` | Desktop & headless CLI archive for FreeBSD builds. |
| **Web (WASM)** | Trunk `web-release` static `.zip` bundle | `packaging/web/package.sh`, `packaging/web/{_headers,.htaccess}` | Built targeting `wasm32-unknown-unknown` with relative (`./`) asset URLs and strict Zero-Telemetry CSP (`connect-src 'none'`), COOP, and COEP headers; verified via `cargo xtask wasm` + `telemetry`. |
| **Compliance & Checksums** | **`THIRD-PARTY-LICENSES.html`**, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `ATTRIBUTION.md`, `SHA256SUMS.txt` | `cargo xtask licenses` (`about.toml`) & release workflow | Bundled inside packages / release assets so every binary download includes complete third-party dependency & asset notices. |

---

## 3. App-Specific Packaging Notes

- **Canonical identity (`packaging/env.sh`):** All installers share App ID `org.lumencraft.LumenCraft`, binary/CLI slug `lumencraft` / `lumencraft-cli`, and project/template/interchange file associations (`.lumencraft`, `.lctemplate`, `.pzfx`, `.prism`). `packaging/check-app-specific.sh` must pass cleanly without template leftover strings.
- **Zero Telemetry & Web Privacy Headers:** LumenCraft ships zero telemetry, analytics, crash uploaders, update-check pings, or external third-party cloud scanners (`cargo xtask telemetry`). Web deployments bundle `packaging/web/_headers` and `packaging/web/.htaccess` enforcing `default-src 'self'`, `connect-src 'none'`, `Referrer-Policy: no-referrer`, `Permissions-Policy`, `Cross-Origin-Opener-Policy: same-origin`, and `Cross-Origin-Embedder-Policy: require-corp`.
- **Zero bundled fonts:** Packages do **not** embed binary fonts by default (`cargo xtask assets` rejects tracked `.ttf`/`.otf`/`.woff`/`.woff2`). Desktop releases probe OS font paths at runtime (`--ttf-font <path>` adds a face); `LUMENCRAFT_FONTS_DIR` is read only at build time, and web tests using `bundled-fonts` require explicit local font staging (`cargo xtask web-fonts`) with matching licence notice rules ([docs/licences.md](licences.md)).
- **macOS specifics:** `packaging/macos/Info.plist.in` registers UTIs/document handlers for `.lumencraft`, `.lctemplate`, `.pzfx`, and `.prism`; hardened runtime entitlements live in `packaging/macos/entitlements.plist`.
- **Linux specifics:** `packaging/linux/` installs desktop, AppStream, icon sizes, and MIME definitions consistently for `.AppImage`, `.deb`, `.rpm`, and Flatpak (`org.lumencraft.LumenCraft`).
- **Windows specifics:** WiX v5 dual-scope MSI supports unprivileged per-user installation by default and elevated per-machine deployment ([packaging/windows/INSTALLATION.md](../packaging/windows/INSTALLATION.md)). Both `.msi` and `-portable.zip` link the static CRT to avoid external redistributable prompts.
- **Web specifics:** Built via Trunk under `web-release` with relative root paths (`./`) and strict no-egress security headers (`packaging/web/{_headers,.htaccess}`) so bundles deploy cleanly on subpath hosts without rebuilding.

---

## 4. Closing Roadmap Items After Release

Once the release build passes verification and is published:

1. Confirm the release-gate table in [docs/ROADMAP.md](ROADMAP.md) (`Legacy Milestone & Gate Mapping`): the **Agent-harness workflows** gate (formerly "Core workflows") covers the semantic `gui_only` agent suite only, and a `1.0` release additionally requires the **Human GUI workflows** gate (`R10`).
2. Mark all shipped `R<m>.<n>` items **Done** in [docs/ROADMAP.md](ROADMAP.md) (moving milestone archival notes to `docs/roadmap-history.md` if retiring a milestone phase).
3. Refresh and verify [docs/STATUS.md](STATUS.md) and open-item summaries:
   ```sh
   cargo xtask status
   cargo xtask pending
   ```
4. Confirm release notes link only verified changes, `THIRD-PARTY-LICENSES.html` and `SHA256SUMS.txt` are published cleanly alongside installers, and open issues closed by `(R<m>.<n>)` PRs are updated.
