# Releasing LumenCraft

Releases are built, signed, notarized, and drafted automatically by [`.github/workflows/release.yml`](../.github/workflows/release.yml) when commits are pushed to the `release` branch (with a nightly unsigned dry run on `main`).

## Version Source of Truth

The workspace version lives in one place: `[workspace.package] version` in the root [`Cargo.toml`](../Cargo.toml).

```sh
# Print current version
cargo xtask version

# Bump version atomically and update Cargo.lock
cargo xtask version set 0.2.0
```

## Release Artifacts

| Platform | Packaging Script | Artifacts |
|---|---|---|
| macOS (`universal`) | `packaging/macos/package.sh` | `lumencraft-<ver>-macos-universal.dmg`, `lumencraft-cli-<ver>-macos-universal.zip` |
| Linux (`x86_64`, `aarch64`, `riscv64`) | `packaging/linux/package.sh`, `flatpak-bundle.sh` | `.AppImage` (+ `.zsync`), `.deb`, `.rpm`, `.tar.gz`, `.flatpak`, GHCR OCI |
| Windows (`x64`, `x86`, `arm64`) | `packaging/windows/package.ps1` | `.msi` (WiX v5 dual-scope) + `-portable.zip` |
| FreeBSD (`x86_64`, `aarch64`) | `packaging/freebsd/package.sh` | `lumencraft-<ver>-freebsd-<arch>.tar.gz` |
| Web (WASM) | `packaging/web/package.sh` | `lumencraft-web-<ver>.zip` |

## Pre-Release Checklist

1. `cargo xtask ci` passes cleanly.
2. `packaging/check-app-specific.sh` passes.
3. All new crates/dependencies satisfy [`ATTRIBUTION.md`](../ATTRIBUTION.md) license policy (`MIT`, `Apache-2.0`, `BSD`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`) and [`NOTICE`](../NOTICE) is up to date.
4. Bump version via `cargo xtask version set X.Y.Z`, merge to `main`, and fast-forward `release` to trigger the signed release workflow.
