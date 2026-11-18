# Verifying a CurveBook Download

Every file on a CurveBook GitHub Release can be checked in two ways:

- **Checksum** — confirms the file arrived intact. `SHA256SUMS.txt` on the release lists a SHA-256 hash for every other file.
- **Build provenance** — confirms the file was built by CurveBook's release workflow, from a recorded commit, on GitHub-hosted runners. Each release file, including `SHA256SUMS.txt`, carries a [Sigstore](https://www.sigstore.dev/) build attestation that GitHub stores and records in a public transparency log.

A checksum on its own does not prove where a file came from: anyone able to replace a release file could replace `SHA256SUMS.txt` too. Check provenance at least once, either on `SHA256SUMS.txt` or on the file itself.

## Beta release files (`R0.28`)

From Beta onward, each GitHub Release provides one installer format per OS (`curvebook-cli` is included in every package):

| Platform | File | OS signature status |
|---|---|---|
| macOS (`arm64` + `x86_64`) | `curvebook-<version>-macos-universal.dmg` | Signed with Apple Developer ID and notarized (`R8.10`; Alpha prereleases were ad-hoc signed). |
| Windows (`x64`, `arm64`) | `curvebook-<version>-windows-x64.msi`, `curvebook-<version>-windows-arm64.msi` | **Unsigned** at Beta until `R8.11`. SmartScreen shows **More info → Run anyway**; Windows 11 Smart App Control or managed PCs may block unsigned installers. (`x86` and `-portable.zip` are retired.) |
| Linux (`x86_64`, `aarch64`) | `curvebook-<version>-linux-x86_64.AppImage`, `curvebook-<version>-linux-aarch64.AppImage` | No detached GPG/Minisign file; verify via Sigstore attestation + `SHA256SUMS.txt`. |
| FreeBSD (`x86_64`, preview) | `curvebook-<version>-freebsd-x86_64.tar.gz` | No detached signature file; verify via Sigstore attestation + `SHA256SUMS.txt`. |
| Licences & Checksums | `THIRD-PARTY-LICENSES.html`, `SHA256SUMS.txt` | Covered by Sigstore attestation. |

*(The browser bundle `curvebook-web-<version>.zip` is deferred to the `1.0` release under `R0.28` / `R11`.)*

## What you need

- The file you downloaded and `SHA256SUMS.txt` from the same release, in the same folder. Run every command below from that folder. With `gh`, you can fetch the checksum file with `gh release download v<version> --repo HauserGroup/curvebook --pattern SHA256SUMS.txt`.
- For the provenance check: the [GitHub CLI](https://cli.github.com/) (`gh`), signed in with any GitHub account (`gh auth login`). `gh` will not look up attestations without a sign-in, and GitHub limits anonymous API requests.

## Recommended: check `SHA256SUMS.txt`, then your file

1. Check where `SHA256SUMS.txt` came from:

   ```sh
   gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook \
     --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
   ```

   For a release built from a version tag, also pin the tag by adding `--source-ref refs/tags/v<version>`. Manually started releases record the branch they ran from instead; `v0.1.0-alpha.1` was built from `refs/heads/main`.

2. Check your file against it.

   **Linux:**

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS.txt
   ```

   **macOS:**

   ```sh
   shasum -a 256 --check --ignore-missing SHA256SUMS.txt
   ```

   On macOS Beta and later releases, you can also inspect the Apple notarization and Gatekeeper assessment on the `.dmg` or installed `CurveBook.app`:

   ```sh
   spctl --assess --type open --context context:primary-signature -vvv curvebook-<version>-macos-universal.dmg
   codesign --verify --strict --deep --verbose=2 /Applications/CurveBook.app
   ```

   **FreeBSD** (replace the file name with the one you downloaded):

   ```sh
   f=curvebook-<version>-freebsd-x86_64.tar.gz
   sha256 -c "$(awk -v f="$f" '$2 == f { print $1 }' SHA256SUMS.txt)" "$f"
   ```

   **Windows** (PowerShell; replace the file name):

   ```powershell
   $f = "curvebook-<version>-windows-x64.msi"
   $expected = (Select-String -Path SHA256SUMS.txt -SimpleMatch "  $f").Line.Split(" ")[0]
   (Get-FileHash $f -Algorithm SHA256).Hash -eq $expected.ToUpper()
   ```

   Linux and macOS print `OK` next to each file that matches. FreeBSD exits silently on a match and prints a mismatch otherwise. PowerShell prints `True`.

## Alternative: check the file directly

```sh
gh attestation verify <downloaded-file> --repo HauserGroup/curvebook \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

## What a passing check means

- The file's SHA-256 hash matches an attestation signed by `.github/workflows/release.yml` in `HauserGroup/curvebook`. The attestation names the commit and ref the release was built from.
- Without `--signer-workflow`, `gh` accepts an attestation from *any* workflow in the repository, so keep that option.
- It does not mean an operating system trusts the file. macOS Developer ID signing and notarization (`R8.10`) ships for Beta; Windows Authenticode signing (`R8.11`) is deferred until after Beta, so Beta Windows `.msi` downloads are unsigned.

## If a check fails

Do not open or install the file. Download it again from the release page and repeat the check. If it still fails, report it as described in [SECURITY.md](../../SECURITY.md).

## Why Linux and FreeBSD downloads have no separate signature file

Neither the AppImage runtime nor FreeBSD checks a signature when you run a downloaded file, so a signature only helps people who check it by hand. The release attestation already gives that check. It ties each file to the release workflow, uses no long-lived signing key that could leak, and records every signature in a public log.

A separate GPG or Minisign signature would only add protection if its key were kept off GitHub. Kept as a GitHub secret, it can be used by anyone who can run the release workflow, which is the same group that can produce an attestation. An offline key would need manual signing on every release and its own key management, and would add one more file per download. We will revisit this if a downstream packager needs a signing key, if an update channel that checks signatures is added, or if releases move off GitHub. This decision is recorded under `R8.10` in the [roadmap](../ROADMAP.md).

## For maintainers

The release workflow runs the same `gh attestation verify` command on every file before uploading it, pinned to the run's own ref, and stops the release if any file fails. The repository settings that keep release tags from being moved are listed in [releasing.md](releasing.md#repository-settings-that-protect-release-provenance).
