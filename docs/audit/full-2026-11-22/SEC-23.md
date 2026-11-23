# Security & Engineering Audit — SEC-23 (Xtask Docs/Release, Testkit, Packaging, CI Workflows & Root Configs)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-23` — remaining `16` `xtask/src/` modules, `6` `xtask/src/docs_site/` modules, `11` `crates/testkit/` files, `32` `packaging/` files (including `.shellcheckrc`, `.htaccess`, `_headers`), `17` `.github/` files, and `14` root configs/docs (`96` files, `22,036` LOC)
- **Date:** `2026-11-22`
- **Summary:** `S1=0`, `S2=8`, `S3=6`, `S4=2` (Total: `16` findings: `FA-SEC23-01` .. `FA-SEC23-16`)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings / Status |
| --- | ---: | --- | --- | --- |
| `xtask/src/bundle.rs` | 240 | `1..=240` | macOS `.app` layout, `Info.plist` rendering & `plutil -lint`, ad-hoc `codesign --verify --strict --deep`, doc bundling, relative `CARGO_TARGET_DIR`, bounded subprocesses | [`FA-SEC23-06`](#fa-sec23-06), [`FA-SEC23-10`](#fa-sec23-10), [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/citations.rs` | 1,139 | `1..=1139` | `D1` rule-1 literature citations, `ALLOWLIST` validation, Rust lexer (`#[cfg(test)]`, `impl`, `mod`, char/raw strings), author-year/DOI/ISBN matching, symlink walks | [`FA-SEC23-05`](#fa-sec23-05) |
| `xtask/src/commands.rs` | 108 | `1..=108` | `--check` flag parsing, `\r\n` normalization, `curvebook_engine::cmd::render_commands_markdown()` parity, file writes | [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/contributors.rs` | 343 | `1..=343` | `people.toml` parser, `git log` identity resolution, JSON string escaping, repo slug extraction, atomic writes & bounded processes | [`FA-SEC23-14`](#fa-sec23-14), [`FA-SEC23-15`](#fa-sec23-15) |
| `xtask/src/corpus.rs` | 746 | `1..=746` | Offline vs `--fetch` separation, HTTPS-only `curl`, `PINNED_NIST_STRD_CORPUS` enforcement, manifest path/size guards, `.dat` validation | [`FA-SEC23-02`](#fa-sec23-02), [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/coverage.rs` | 291 | `1..=291` | `-p`/`--` CLI parsing, `xtask`/`curvebook-testkit` exclusions, `cargo-llvm-cov` plan, JSON summary validation, `CARGO_TARGET_DIR` isolation | [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/cursors.rs` | 446 | `1..=446` | `pub enum Cursor` scanner, comment/string/attribute skipping, duplicate/missing/stale mismatch reporting, `cursors.toml` parsing | [`FA-SEC23-15`](#fa-sec23-15) |
| `xtask/src/ico.rs` | 108 | `1..=108` | PNG signature + `IHDR` validation, `1..=256` bounds, `256` -> `0` byte width/height encoding, checked offsets, output writes | [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/msix.rs` | 273 | `1..=273` | SemVer 4-part Store version (`A.B.C.0`), XML escaping, PE machine validation (`x64`/`x86`/`arm64`), atomic writes, bounded `MakeAppx.exe`, full doc/license staging | Clean — strict version, PE arch, XML escaping, `write_atomic`, `ProcessCommand`, and complete doc/attribution staging |
| `xtask/src/release_scan.rs` | 2,184 | `1..=2184` | `SHA256SUMS.txt` parity, Beta 8-file completeness, cross-OS report verification, archive path traversal/symlinks/size caps, AppImage execution fallback, macOS DMG mount cleanup | [`FA-SEC23-01`](#fa-sec23-01) |
| `xtask/src/schemas.rs` | 168 | `1..=168` | `--check` flag parsing, JSON pretty formatting, stale `.schema.json` file detection, `curvebook_format::schemas` parity | [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/screenshots.rs` | 329 | `1..=329` | Nested target dir resolution, PNG magic + chunk CRC-32 + `IHDR`/`IEND` bounds, `ASSETS.md` lookup, subprocess/file safety | [`FA-SEC23-13`](#fa-sec23-13), [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/stats.rs` | 109 | `1..=109` | `--exact` CLI handling, workspace package walk, symlink handling, line/test/proptest counting | [`FA-SEC23-12`](#fa-sec23-12) |
| `xtask/src/status.rs` | 2,355 | `1..=2355` | Markdown duplicate-key check, `WorkspaceTests` lexer accuracy, `WorkflowRun` static bypass lints, `UI_COMMANDS` counting, metric parity | [`FA-SEC23-04`](#fa-sec23-04) |
| `xtask/src/ui_commands.rs` | 118 | `1..=118` | Inner doc stripping, `rustfmt` pipe handling, `--check` newline normalization, sync banner emission | [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/web_fonts.rs` | 273 | `1..=273` | Font manifest traversal checks, `TRUNK_STAGING_DIR` handling, `MAX_WEB_RELEASE_ZIP_BYTES` + ZIP signature verification, static readiness checks | [`FA-SEC23-13`](#fa-sec23-13) |
| `xtask/src/docs_site/markdown.rs` | 743 | `1..=743` | HTML entity escaping across headings/tables/code/links/images, link & table cell parsing, GitHub slug uniqueness | Clean — all text and attributes HTML-escaped; safe slice indexing on UTF-8 boundaries |
| `xtask/src/docs_site/mod.rs` | 292 | `1..=292` | Deterministic preview check, `--release` snapshot manifest & redirect generation, file cleanup walks, bounded `git` calls | [`FA-SEC23-12`](#fa-sec23-12), [`FA-SEC23-14`](#fa-sec23-14) |
| `xtask/src/docs_site/registries.rs` | 1,235 | `1..=1235` | Live HTML generation for all 7 registry pages (`cmd`, `UI_COMMANDS`, `mcp`, `cli`, `fit`, `doc`, `graphs`, `formats`, schemas), full `escape_html` usage | Clean — live typed metadata serialization with consistent `escape_html` |
| `xtask/src/docs_site/sitemap.rs` | 923 | `1..=923` | Section ordering, internal/GitHub link rewriting, broken link/image handling, A–Z deduplication | [`FA-SEC23-08`](#fa-sec23-08) |
| `xtask/src/docs_site/template.rs` | 320 | `1..=320` | `STRICT_CSP` (`default-src 'none'`), `no-referrer`, breadcrumbs/pager/TOC escaping, snapshot relative asset prefixes | Clean — strict CSP, `no-referrer`, zero scripts/forms, and full HTML escaping |
| `xtask/src/docs_site/verify.rs` | 633 | `1..=633` | Forbidden page exclusions, `<script>`/`<form>`/`style=`/`on*=`/external-URL tag audit, fragment checks, registry coverage, committed snapshots, secrets/brands/cleanroom/assets | [`FA-SEC23-08`](#fa-sec23-08), [`FA-SEC23-12`](#fa-sec23-12), [`FA-SEC23-14`](#fa-sec23-14) |
| `crates/testkit/Cargo.toml` | 16 | `1..=16` | `[lints] workspace = true`, workspace version/edition inheritance, minimal internal-only dependencies (`math`, `palette`, `doc`) | Clean — zero network/HTTP/`chrono` deps; inherits workspace lints |
| `crates/testkit/src/dd.rs` | 201 | `1..=201` | Pure-Rust 106-bit `DoubleDouble` arithmetic (`two_sum`, `two_prod`, `sqrt`, `powi`, `oracle_univariate_f64`) | Clean — verified compensated double-double operators and finite/empty edge guards |
| `crates/testkit/src/lib.rs` | 47 | `1..=47` | `#![forbid(unsafe_code)]`, test-only helper re-exports | Clean — `#![forbid(unsafe_code)]` and public test oracle API |
| `crates/testkit/src/metrics.rs` | 96 | `1..=96` | `approx_eq`, `ulp_distance` (`+0.0`/`-0.0`), NIST `LRE` clamping, `NaN`/`Inf` handling | Clean — non-finite values safely rejected before ULP/LRE comparisons |
| `crates/testkit/src/r13_datasets.rs` | 611 | `1..=611` | `#![forbid(unsafe_code)]`, synthetic `WF-51`..`WF-65` fixtures, clean-room identifiers & finite dimensions | Clean — original synthetic fixtures only, `#![forbid(unsafe_code)]` |
| `crates/testkit/src/sha256.rs` | 257 | `1..=257` | FIPS 180-4 SHA-256 implementation, test vectors, size/digest verification, pre-read sizing in `load_verified_corpus_file` | [`FA-SEC23-02`](#fa-sec23-02) |
| `crates/testkit/src/wf_datasets.rs` | 980 | `1..=980` | `#![forbid(unsafe_code)]`, synthetic `WF-01`..`WF-65` builders & TSV/sheet converters, clean-room fixtures | Clean — original well-conditioned numeric fixtures, `#![forbid(unsafe_code)]` |
| `crates/testkit/src/nist_strd/corpus.rs` | 558 | `1..=558` | 58 pinned SHA-256 + exact byte-size entries, `CURVEBOOK_REQUIRE_CORPUS` fail-closed guard, dataset count invariants | Clean — all 58 NIST StRD files pinned by exact SHA-256 and byte count |
| `crates/testkit/src/nist_strd/datasets.rs` | 459 | `1..=459` | Offline certified univariate/linear/ANOVA generators (`DoubleDouble` Householder QR oracles) | Clean — zero network I/O; 106-bit DD-certified offline regression & ANOVA benchmarks |
| `crates/testkit/src/nist_strd/mod.rs` | 23 | `1..=23` | Module re-exports for NIST StRD corpus, datasets, and parsers | Clean — re-export facade only |
| `crates/testkit/src/nist_strd/parser.rs` | 939 | `1..=939` | `MAX_STRD_BYTES` (`16 MiB`) / `MAX_STRD_ROWS` (`1e6`) caps, `checked_add`, Fortran `D` floats, TOC vs `Data:` disambiguation, `df_between` pre-alloc bounds | Clean — bounded memory/row caps, overflow-checked group validation, finite float checks |
| `packaging/.shellcheckrc` | 7 | `1..=7` | `external-sources=true`, `source-path=SCRIPTDIR`, `avoid-nullary-conditions` | Clean — configures `shellcheck -x` cross-file source resolution |
| `packaging/check-app-specific.sh` | 69 | `1..=69` | Forbidden legacy/placeholder brand sweep, Linux desktop/metainfo/MIME, macOS plist, and Windows `UpgradeCode`/GUID checks | Clean — verifies CurveBook bundle IDs, GUIDs, and neutral naming |
| `packaging/env.sh` | 68 | `1..=68` | `[workspace.package]` version extraction, `DIST`/`CARGO_TARGET_DIR` setup, `copy_docs()` file set, `sha256` portability | [`FA-SEC23-06`](#fa-sec23-06), [`FA-SEC23-15`](#fa-sec23-15) |
| `packaging/icons.sh` | 65 | `1..=65` | SVG viewBox macOS inset rewrite, workspace-local temp dir (`$ROOT/target`), `cargo xtask ico` invocation | [`FA-SEC23-10`](#fa-sec23-10) |
| `packaging/release-channel.sh` | 63 | `1..=63` | SemVer validation, prerelease detection (`alpha`/`beta`/`rc` -> `prerelease=true`), `ALPHA_RELEASE` handling | Clean — rejects invalid SemVer and prevents prerelease tags from marking Latest |
| `packaging/freebsd/package.sh` | 63 | `1..=63` | `--locked` Cargo build, `tar` bundling of desktop/MIME/metainfo/icons/docs, `copy_docs` usage | [`FA-SEC23-06`](#fa-sec23-06) |
| `packaging/linux/apprun-test.sh` | 47 | `1..=47` | Quoted `--appimage-extract` smoke execution of `AppRun` and `curvebook-cli` | Clean — clean temp workdir cleanup (`trap`), quoted paths, CLI verification |
| `packaging/linux/org.curvebook.CurveBook.desktop` | 20 | `1..=20` | Desktop Entry 1.5 metadata, MIME types, `Exec=curvebook %F`, `Icon=org.curvebook.CurveBook` | Clean — matches AppStream and MIME specifications |
| `packaging/linux/org.curvebook.CurveBook.metainfo.xml.in` | 61 | `1..=61` | CC0-1.0 metadata license, `MIT OR Apache-2.0` project license, `@VERSION@`/`@DATE@` placeholders | Clean — matches desktop ID, project URLs, and binary metadata |
| `packaging/linux/org.curvebook.CurveBook.mime.xml` | 25 | `1..=25` | `*.curvebook`, `*.cbtemplate`, and external interchange MIME globs | Clean — valid Shared MIME-info declarations |
| `packaging/linux/package.sh` | 121 | `1..=121` | `--locked` build, SHA-256 pinning for downloaded `appimagetool` & `runtime`, `desktop-file-validate`/`appstreamcli`, preinstalled tool trust | [`FA-SEC23-07`](#fa-sec23-07) |
| `packaging/macos/Info.plist.in` | 144 | `1..=144` | Bundle ID, macOS min `11.0`, exported UTIs (`.curvebook`, `.cbtemplate`), document viewer roles | Clean — complete bundle & UTI declarations |
| `packaging/macos/entitlements.plist` | 9 | `1..=9` | Hardened Runtime entitlements vs zero-telemetry / zero-egress policy | [`FA-SEC23-11`](#fa-sec23-11) |
| `packaging/macos/import-cert.sh` | 34 | `1..=34` | Ephemeral keychain creation, `.p12` decoding & cleanup trap, partition list & `Developer ID Application` lookup | [`FA-SEC23-07`](#fa-sec23-07) |
| `packaging/macos/package.sh` | 171 | `1..=171` | Universal `lipo`, `REQUIRE_SIGNING` enforcement in CI, Hardened Runtime signing, `notarytool` + `stapler` retries, `copy_docs`, CLI parsing | [`FA-SEC23-06`](#fa-sec23-06), [`FA-SEC23-16`](#fa-sec23-16) |
| `packaging/macos/verify.sh` | 219 | `1..=219` | Strict `--verify --deep`, `--require-notarized` fail-closed checks, bounded notarization retries, DMG mount cleanup trap | Clean — verifies bundle, DMG, and bundled `curvebook-cli` with strict cleanup |
| `packaging/web/.htaccess` | 24 | `1..=24` | MIME types (`.wasm`, `.ttf`, `.js`), CSP/COOP/COEP/`no-referrer`/`nosniff`/`Permissions-Policy`, cache headers | [`FA-SEC23-11`](#fa-sec23-11) |
| `packaging/web/README.md` | 25 | `1..=25` | Web hosting guide, MIME/CORS/HTTPS/WASM-SIMD browser floor docs | Clean — matches `.cargo/config.toml` `+simd128` and hosting headers |
| `packaging/web/_headers` | 42 | `1..=42` | Global `/*` CSP/COOP/COEP/`Referrer-Policy`/`Permissions-Policy`, root & subpath `/curvebook/` asset rules | [`FA-SEC23-11`](#fa-sec23-11) |
| `packaging/web/package.sh` | 33 | `1..=33` | `trunk build` lockfile parity, root-absolute URL guard, header/doc staging (`copy_docs`) | [`FA-SEC23-06`](#fa-sec23-06), [`FA-SEC23-10`](#fa-sec23-10) |
| `packaging/windows/AppxManifest.xml.in` | 44 | `1..=44` | MSIX identity tokens, `Windows.Desktop` versions, `.curvebook`/`.cbtemplate` association, `curvebook-cli.exe` alias | Clean — consistent with `xtask/src/msix.rs` and `store-identity.json` |
| `packaging/windows/INSTALLATION.md` | 27 | `1..=27` | Per-user/machine scope documentation and standalone policy check instructions | Clean — matches `install-policy.wxi` and `test-install-policy.ps1` |
| `packaging/windows/check-icons.ps1` | 131 | `1..=131` | ICO `ICONDIR`/`ICONDIRENTRY` bounds checking, PNG `IHDR` & DIB header dimension validation | Clean — bounds-checks every directory offset, reserved field, and frame header |
| `packaging/windows/check-install-policy.ps1` | 171 | `1..=171` | Safe COM read-only MSI validation (`OpenPackage(..., 1)`), `SecureCustomProperties`, 24 scope/install matrix cases, registry & GUID contract | Clean — read-only MSI verification covering scope, upgrade/component GUIDs, and sequencing |
| `packaging/windows/curvebook.en-us.wxl` | 11 | `1..=11` | WiX en-US localization strings for setup wizard and shortcut dialog | Clean — properly XML-escaped localization strings |
| `packaging/windows/curvebook.wxs` | 103 | `1..=103` | WiX v4/v5 package definition, dual-scope, `install-policy.wxi` include, shortcuts, file associations | Clean — matches `install-policy.json` UpgradeCode, GUIDs, and registry keys |
| `packaging/windows/install-policy.json` | 45 | `1..=45` | Pinned `upgradeCode`, component GUIDs, extensions, payloads, and `HKMU` registry entries | Clean — matches `curvebook.wxs` and `check-install-policy.ps1` |
| `packaging/windows/install-policy.wxi` | 58 | `1..=58` | `SecureCustomProperties` flags, user/machine AppSearch, duplicate-scope LaunchConditions, UI dialog sequencing | Clean — exhaustively verified by `check-install-policy.ps1` |
| `packaging/windows/package.ps1` | 138 | `1..=138` | `+crt-static` `--locked` build, PE validation, doc staging (`ATTRIBUTION.md`), WiX build, policy check & signing invocation | [`FA-SEC23-06`](#fa-sec23-06), [`FA-SEC23-09`](#fa-sec23-09) |
| `packaging/windows/sign.ps1` | 85 | `1..=85` | Authenticode signing via `WINDOWS_CERTIFICATE` or SHA-256-pinned `Microsoft.Trusted.Signing.Client`, missing-secret handling | [`FA-SEC23-09`](#fa-sec23-09) |
| `packaging/windows/store-identity.json` | 5 | `1..=5` | Microsoft Store publisher ID and package name matching `xtask/src/msix.rs` tests | Clean — matches `AppxManifest.xml.in` and `msix.rs` assertions |
| `packaging/windows/test-install-policy.ps1` | 55 | `1..=55` | Non-executable `x64`/`arm64` MSI fixture build + `check-install-policy.ps1` runner | Clean — validates `install-policy.json` invariants without installing |
| `.github/CODEOWNERS` | 7 | `1..=7` | Default code owner mapping | Clean — standard `* @jsture` ownership |
| `.github/actionlint.yaml` | 8 | `1..=8` | Custom GitHub-hosted ARM runner labels (`ubuntu-24.04-arm`, `windows-11-arm`) | Clean — eliminates false positives for ARM hosted labels |
| `.github/dependabot.yml` | 46 | `1..=46` | Daily Cargo & GitHub Actions update grouping and PR cap (`10`) | Clean — valid Dependabot v2 configuration |
| `.github/pull_request_template.md` | 27 | `1..=27` | Clean-room, no-panic, oracle, asset, and `ci quick` PR checklist | Clean — aligns with `AGENTS.md` gates |
| `.github/ISSUE_TEMPLATE/bug_report.yml` | 76 | `1..=76` | Bug report schema and private security redirection | Clean — neutral prompts redirecting vulnerabilities to `SECURITY.md` |
| `.github/ISSUE_TEMPLATE/config.yml` | 8 | `1..=8` | Blank issues disabled; links to security policy & roadmap | Clean — routes security reports privately |
| `.github/ISSUE_TEMPLATE/feature_request.yml` | 49 | `1..=49` | Clean-room public-spec enforcement on feature requests | Clean — explicitly bars proprietary inputs |
| `.github/ISSUE_TEMPLATE/numerical_result.yml` | 89 | `1..=89` | Clean-room D1 reference check (NIST/published/open-source) | Clean — explicitly forbids proprietary app reference values |
| `.github/ISSUE_TEMPLATE/roadmap_task.yml` | 60 | `1..=60` | Roadmap task claim & exit-criterion template | Clean — matches `(R<m>.<n>)` PR/roadmap workflow |
| `.github/actions/ci-setup/action.yml` | 156 | `1..=156` | SHA-pinned checkout/cache/toolchain, `restore-mtimes`, SHA-256 verification of `nextest`/`actionlint`, curl proto flags | [`FA-SEC23-10`](#fa-sec23-10) |
| `.github/workflows/ci.yml` | 211 | `1..=211` | Top-level `contents: read`, `persist-credentials: false`, SHA-pinned actions, `build` + 6 `checks` + `gate`, env-scoped `prune-caches` | Clean — least-privilege permissions, zero expression injection, complete CI matrix parity |
| `.github/workflows/coverage.yml` | 89 | `1..=89` | Diagnostic-only local artifact coverage, pinned nextest checksum, `cargo-llvm-cov` version pinning | [`FA-SEC23-10`](#fa-sec23-10) |
| `.github/workflows/freebsd.yml` | 45 | `1..=45` | `contents: read`, SHA-pinned `freebsd-vm`, pinned Rust `1.99.1`, `packaging/freebsd/package.sh` | Clean — least-privilege read-only FreeBSD build validation |
| `.github/workflows/packaging-lint.yml` | 88 | `1..=88` | `contents: read`, `shellcheck -x`, `desktop-file-validate`, `appstreamcli`, `check-app-specific.sh`, WiX `test-install-policy.ps1` | Clean — validates packaging shell/XML/WiX metadata under read-only token |
| `.github/workflows/release.yml` | 685 | `1..=685` | `contents: read` root, `publish` narrow write/OIDC perms, SHA-pinned actions, version/`Cargo.lock` source audit, cross-OS reports, `attest-build-provenance`, Windows signing enforcement | [`FA-SEC23-09`](#fa-sec23-09), [`FA-SEC23-10`](#fa-sec23-10), [`FA-SEC23-16`](#fa-sec23-16) |
| `.github/workflows/windows-arm64.yml` | 50 | `1..=50` | `contents: read`, SHA-pinned actions, pinned WiX `5.0.2`, `package.ps1 -Arch arm64` | Clean — read-only Windows ARM64 MSI smoke build |
| `.github/workflows/windows-store.yml` | 129 | `1..=129` | `contents: read`, `--locked` tools & build, static CRT, `xtask msix`, SHA-256 checksums, non-injected `env:` dispatch version | Clean — builds unsigned Partner Center MSIX cleanly with license HTML bundled |
| `Cargo.toml` | 101 | `1..=101` | Workspace resolver/members, minimal non-network dependencies, `unsafe_code = "forbid"`, no-panic clippy lints vs `indexing_slicing` | [`FA-SEC23-03`](#fa-sec23-03), [`FA-SEC23-16`](#fa-sec23-16) |
| `clippy.toml` | 15 | `1..=15` | Disallowed `std::sync` & `egui::ComboBox`, cognitive complexity `30`, test exemptions matching workspace lints | [`FA-SEC23-03`](#fa-sec23-03) |
| `deny.toml` | 45 | `1..=45` | `all-features = true`, `yanked = "deny"`, narrow 3-ID Linux accesskit unmaintained review list (`2027-02-16`), 8-license allowlist, `allow-git = []` | Clean — aligns with `AGENTS.md` and `about.toml`; registry-only crates enforced |
| `about.toml` | 15 | `1..=15` | Accepted SPDX allowlist parity with `deny.toml`, build deps included | Clean — exact 8-license match with `deny.toml` and `AGENTS.md` |
| `about.hbs` | 102 | `1..=102` | Script-free HTML template with Handlebars `{{...}}` escaping for `cargo-about` | Clean — zero `<script>` tags and escaped template variables |
| `rust-toolchain.toml` | 4 | `1..=4` | `channel = "1.99.1"`, `minimal` profile, `rustfmt`/`clippy` components | Clean — matches `Cargo.toml` MSRV and CI workflow toolchain pins |
| `rustfmt.toml` | 2 | `1..=2` | `max_width = 150`, `use_small_heuristics = "Max"` | Clean — standard workspace formatting policy |
| `.cargo/config.toml` | 8 | `1..=8` | `xtask` alias and `wasm32-unknown-unknown` `+simd128` flag | Clean — zero `[source]`/`[patch]`/`[paths]` overrides |
| `.config/nextest.toml` | 19 | `1..=19` | Min `nextest-version = "0.9.150"` (GHSA fix), `retries = 0`, bounded `slow-timeout`, clipboard serialization override | Clean — enforces no retries and serializes OS clipboard test |
| `.githooks/pre-commit` | 3 | `1..=3` | POSIX `sh` `set -eu` running `exec cargo xtask secrets` | Clean — runs secrets check pre-commit |
| `.coderabbit.yaml` | 17 | `1..=17` | Auto-review rules reinforcing `AGENTS.md` no-panic/safe-indexing/layering invariants | Clean — consistent with repository policies |
| `ASSETS.md` | 63 | `1..=63` | Asset manifest mapping icons, screenshots, and UI SVGs with permissive licenses & zero committed fonts | Clean — verified with `xtask/src/assets.rs` |
| `ATTRIBUTION.md` | 24 | `1..=24` | Asset attributions + permitted upstream `MIT OR Apache-2.0` GUI source reuse record | Clean — conforms to `AGENTS.md` & `docs/licences.md` (note omission from bundles in `FA-SEC23-06`) |
| `NOTICE` | 49 | `1..=49` | Dual MIT/Apache-2.0 copyright notice, bundled font notice policy, upstream permission & MIT text | Clean — preserves required copyright and permission texts |

---

## Findings

### `FA-SEC23-01` · `S2` · `release-scan` executes untrusted AppImage binaries on fallback, misses archive symlink/hardlink & `.deb` `ar` traversal guards, and reads whole containers unboundedly

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/release_scan.rs:768, 782, 946`, `xtask/src/release_scan.rs:974-988`, `xtask/src/release_scan.rs:1033-1061`, `xtask/src/release_scan.rs:1092-1140`, `xtask/src/release_scan.rs:1174, 1205-1220`, `xtask/src/release_scan.rs:1347`, `xtask/src/release_scan.rs:1433-1475`
- **Category:** `security` / `resources`
- **Defect:**
  1. When `unsquashfs` fails or is absent on Linux (`tools.allow_appimage_exec`), `unpack_appimage` marks the scanned `.AppImage` executable (`0o755`) and **executes the untrusted binary directly** (`runner.AppImage --appimage-extract`).
  2. `unpack_tarball` and `unpack_rpm` run `tar -tzf` / `bsdtar -tf` / `cpio -it` (which print only filenames without `-v` entry-type indicators for symlinks/hardlinks or link targets), and `extract_deb_ar_members` runs `ar -x` / `bsdtar -xf` without listing/validating member names prior to extraction.
  3. `unpack_zip`, `unpack_appimage`, `scan_single_asset` (for HTML/zsync), and `copy_mounted_tree_no_symlinks` read entire containers or DMG files into RAM with unbounded `std::fs::read` (prior to `RELEASE_ZIP_LIMITS` or size bounds).
- **Failure scenario:** Running `cargo xtask release-scan` against a hostile or corrupted `.AppImage` without `unsquashfs` (or with an intentionally offset/malformed SquashFS header so `unsquashfs` fails) triggers native code execution inside the host or CI runner. On `.tar.gz`/`.deb`/`.rpm`, a crafted symlink or hardlink entry named `./safe/link` passes `validate_archive_listing` because `tar -tf`/`cpio -it` omit `l`/`h` type flags and targets, and multi-gigabyte release blobs trigger OOM during `std::fs::read`.
- **Evidence:**
  ```rust
  // xtask/src/release_scan.rs:1174, 1205-1212
  let bytes = std::fs::read(appimage_path).map_err(|e| format!("read `{asset_name}`: {e}"))?;
  ...
  if tools.allow_appimage_exec && bytes.starts_with(b"\x7fELF") {
      ...
      set_owner_executable(&runner_copy)?;
      match build_cmd(tools, runner_str.as_ref()).current_dir(&exec_work).env("APPIMAGE_EXTRACT_AND_RUN", "1").arg("--appimage-extract").run() {

  // xtask/src/release_scan.rs:946, 974-979, 1051
  let bytes = std::fs::read(path).map_err(|e| format!("read `{asset_name}`: {e}"))?;
  ...
  let (chosen_tar, list_out) = run_tar_or_bsdtar(tools, &[list_flag, archive_str.as_ref()], asset_name, "list entries")?;
  let listing = String::from_utf8_lossy(&list_out);
  validate_archive_listing(asset_name, &listing)?;
  ...
  match build_cmd(tools, &tools.ar_bin).current_dir(ar_dir).args(["-x", deb_str.as_ref()]).run() {
  ```
- **Suggested fix:** Remove `APPIMAGE_EXTRACT_AND_RUN=1` / direct `.AppImage` binary execution from `release_scan.rs` (require `unsquashfs` cleanly rather than running untrusted ELFs); list tar/cpio/`ar` entries with verbose mode (`tar -tv` / `ar -t` before `ar -x`) and fail closed on symlinks/hardlinks or unsafe targets outside `stage_dir`; and stream/bound `std::fs::metadata` file sizes before allocating in `unpack_zip`, `unpack_appimage`, and `copy_mounted_tree_no_symlinks`.
- **Related:** `FA-SEC22-12`, `FA-SEC22-18`

---

### `FA-SEC23-02` · `S2` · `corpus` permits path traversal and unbounded reads on two-column manifests, and `testkit::sha256` reads unverified files before size checks

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/corpus.rs:290-299`, `xtask/src/corpus.rs:380`, `crates/testkit/src/sha256.rs:230-235`
- **Category:** `security` / `resources` / `wrong-result`
- **Defect:** In `xtask/src/corpus.rs::verify_manifest_file`, the two-column `[sha_hex, rel]` branch checks `pinned_spec_for_rel(clean_rel)` only when `enforce_pinned && clean_rel.ends_with(".dat")`, joins `nist_root.join(clean_rel)` without rejecting leading `/` or `..` traversal components, omits `MAX_STRD_BYTES`, and passes `bytes.len()` as `expected_size` to `verify_blob`. Additionally, both `corpus.rs:380` and `crates/testkit/src/sha256.rs:230-235` read files into memory via `std::fs::read` before inspecting file size against `expected_bytes` / `MAX_STRD_BYTES`.
- **Failure scenario:** A crafted `sha256sums.txt` in a corpus directory with a two-column non-`.dat` entry (or inside a directory not named `nist-strd`) pointing to `/dev/zero`, `../../../etc/passwd`, or a multi-gigabyte file bypasses path containment and size pinning and triggers unbounded memory allocation (`std::fs::read`); similarly, `load_verified_corpus_file` allocates an arbitrarily large replaced `.dat` file in RAM before `verify_blob` detects `bytes.len() != expected_size`.
- **Evidence:**
  ```rust
  // xtask/src/corpus.rs:290-298
  [sha_hex, rel] => {
      let clean_rel = rel.strip_prefix('*').unwrap_or(rel);
      if enforce_pinned && clean_rel.ends_with(".dat") && pinned_spec_for_rel(clean_rel).is_none() {
          return Err(format!("{clean_rel}: unexpected `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"));
      }
      let bytes = fs::read(nist_root.join(clean_rel)).map_err(|e| format!("read {clean_rel}: {e}"))?;
      verify_blob(&bytes, bytes.len(), sha_hex).map_err(|e| format!("{}:{line_no} ({clean_rel}): {e}", manifest_path.display()))?;

  // crates/testkit/src/sha256.rs:230-234
  pub fn load_verified_corpus_file(corpus_root: &Path, entry: CorpusManifestEntry<'_>) -> Result<String, CorpusError> {
      let full_path = corpus_root.join(entry.relative_path);
      let bytes = std::fs::read(full_path).map_err(CorpusError::Io)?;
      verify_blob(&bytes, entry.expected_bytes, entry.sha256_hex)?;
  ```
- **Suggested fix:** Validate relative manifest paths strictly (`Component::Normal` only, no leading `/` / `\` / `..`, regular non-symlink files via `symlink_metadata`), check `meta.len()` against `MAX_STRD_BYTES` / `entry.expected_bytes` *before* `fs::read` in both `corpus.rs` and `sha256.rs`, and reject unknown non-`.dat` two-column entries in pinned manifests.
- **Related:** `none`

---

### `FA-SEC23-03` · `S2` · `Cargo.toml` omits `clippy::indexing_slicing = "deny"` configured by `clippy.toml`, leaving unchecked slice indexing unenforced

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `Cargo.toml:66-80`, `clippy.toml:1-6` (contract `AGENTS.md:45-49`)
- **Category:** `no-panic` / `contract-drift`
- **Defect:** `clippy.toml` configures `allow-indexing-slicing-in-tests = true` alongside `allow-unwrap-in-tests`, `allow-expect-in-tests`, and `allow-panic-in-tests`, and `AGENTS.md` mandates checked `.get(...)` access instead of unchecked slice indexing across shipped code, yet `[workspace.lints.clippy]` in `Cargo.toml` omits `indexing_slicing = "deny"` (and `string_slice = "deny"`).
- **Failure scenario:** Any `slice[idx]` or `&text[a..b]` expression introduced into production crates or `xtask` passes `cargo clippy --workspace --all-targets -- -D warnings` and CI's `lint` stage with zero diagnostics, allowing out-of-bounds/UTF-8 slicing panics (such as `assets::parse_table_cells` at `xtask/src/assets.rs:193`) to ship undetected.
- **Evidence:**
  ```toml
  # clippy.toml:1-6
  # Tests may unwrap, expect, panic and index/slice directly: a failing assertion is the point of a
  # test. Shipped code may not (workspace lints in Cargo.toml; AGENTS.md › Non-negotiables).
  allow-unwrap-in-tests = true
  allow-expect-in-tests = true
  allow-panic-in-tests = true
  allow-indexing-slicing-in-tests = true

  # Cargo.toml:71-80
  # CurveBook never crashes: shipped code returns `Result`/`Option` instead of panicking (see
  # AGENTS.md › Non-negotiables). Tests may unwrap (clippy.toml).
  unwrap_used = "deny"
  expect_used = "deny"
  panic = "deny"
  todo = "deny"
  unimplemented = "deny"
  unreachable = "deny"
  ```
- **Suggested fix:** Add `indexing_slicing = "deny"` (and consider `string_slice = "deny"`) to `[workspace.lints.clippy]` in `Cargo.toml` and replace any remaining non-test indexing in shipped crates (marking helper-only test crates like `curvebook-testkit` appropriately if dev-only).
- **Related:** `FA-SEC22-13`

---

### `FA-SEC23-04` · `S2` · `status` misses `mod tests` / helper `WorkflowRun` bypasses, indexes commented `#[test]` fixtures, and truncates `UI_COMMANDS` on `"];"`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/status.rs:194-244`, `xtask/src/status.rs:908-958, 1165`, `xtask/src/status.rs:1302-1363`, `xtask/src/status.rs:2133-2140`
- **Category:** `wrong-result` / `security`
- **Defect:**
  1. `collect_workspace_tests` (`scan_tests_in_rust_source`) scans lines without lexing `/* ... */` block comments or `r#"..."#` multi-line raw strings.
  2. `scan_human_wf_completion` scans only top-level `crates/ui-egui/tests/human_wf_*.rs` files, `extract_rust_functions` skips functions inside `mod tests { ... }` (`brace_depth != 0`), and `analyze_human_wf_sources` skips `check_workflow_run_lint` for any function whose source does not literally contain `"WorkflowRun"`. Meanwhile `has_finish` matches any `.finish()`, and `has_guarded_find_by_widget_id` checks only `if`/`if let` conditions.
  3. `count_live_ui_commands` finds the array end using naive `slice_tail.find("];")` without skipping string literals.
- **Failure scenario:**
  - Any `/* #[test] fn wf01_human_stub() {} */` comment or multi-line raw string test fixture in `crates/`/`apps/` is counted by `status` as a backed workspace human-path test.
  - A `#[test] fn wf01_human_...` inside `mod tests { ... }`, or in a subdirectory under `tests/human_wf_*/`, or a top-level test delegating `app_mut()`, `state_mut()`, `execute_command`, `dispatch_gui_action`, `Session::from_document`, or `find_by_widget_id` into a helper function that omits the `WorkflowRun` identifier (e.g., via a type alias or `&mut UiApp`), is counted or bypasses `check_workflow_run_lint` undetected.
  - Adding a `UiCommandSpec` whose `params_doc`/`summary` string contains `"];"` (such as `"[]];"` tested in `xtask/src/ui_commands.rs:98`) causes `count_live_ui_commands` to truncate the slice early and fail `status` with a false count mismatch.
- **Evidence:**
  ```rust
  // xtask/src/status.rs:944-954, 1165, 2136-2140
  if func.header_and_body_code.contains("WorkflowRun") {
      check_workflow_run_lint(path, &func, &mut violations);
  }
  ...
  if brace_depth == 0 {
  ...
  let slice_start = src.find("pub const UI_COMMANDS:")?;
  let slice_tail = &src[slice_start..];
  let slice_end = slice_tail.find("];")?;
  Some(slice_tail[..slice_end].matches("UiCommandSpec {").count())
  ```
- **Suggested fix:** Reuse `build_stripped_views` inside `scan_tests_in_rust_source` so comments and string literals cannot register tests; recurse into `human_wf_*` subdirectories and descend into `mod tests { ... }` blocks in `extract_rust_functions`; run `check_workflow_run_lint` across all functions in `human_wf_*.rs` regardless of whether `"WorkflowRun"` appears in each helper; and count `UI_COMMANDS` entries via `build_stripped_views` (ignoring string contents) or bracket matching.
- **Related:** `FA-SEC23-12`

---

### `FA-SEC23-05` · `S2` · `citations` treats inherent `impl` blocks with `for<'a>` bounds as trait impls and skips all their `pub fn` items

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/citations.rs:469-471`, `xtask/src/citations.rs:530-557`, `xtask/src/citations.rs:756-757, 917-930`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `enter_or_skip_container` sets `saw_for = true` whenever any `Token::Ident { text == "for" }` appears anywhere between `impl` and `{`. Additionally, `skip_char_or_lifetime` checks only single-byte distance (`bytes.get(start + 2) == Some(&b'\'')`) for non-escaped character literals.
- **Failure scenario:** Any inherent `impl` in `crates/{math,stats,fit,transform}/src/` that uses a Higher-Rank Trait Bound in its `where` clause or generic parameter (e.g., `impl<F> Solver<F> where F: for<'a> Fn(&'a [f64]) -> f64 { pub fn solve(...) }`) is misclassified as a trait `impl Trait for Type`, causing `citations` to skip the entire block and never check or count any `pub fn` methods inside it. Separately, a multi-byte UTF-8 char literal (e.g. `'α'`, `'—'`) fails `skip_char_or_lifetime`, leaving trailing `'` tokens.
- **Evidence:**
  ```rust
  // xtask/src/citations.rs:469-471, 538-551
  if kw == "impl" {
      return enter_or_skip_container(tokens, kw_idx + 1, end, true, rel_path, report);
  }
  ...
  let mut saw_for = false;
  while idx < end {
      match &tokens[idx] {
          Token::Semi => return idx + 1,
          Token::Ident { text, .. } if reject_trait_for && text == "for" => {
              saw_for = true;
              idx += 1;
          }
          Token::LBrace => {
              let close = find_matching_rbrace(tokens, idx, end);
              if !saw_for {
                  parse_container_items(tokens, idx + 1, close, rel_path, report);
              }
  ```
- **Suggested fix:** Ignore `for` inside `where` clauses (stop checking for trait `for` once the `where` keyword or `<...>` generic/HRTB depth > 0 is entered) so only top-level `impl ... Trait for Target {` is treated as a trait implementation; and use `src[start + 1..].chars().next()` length in `skip_char_or_lifetime` to handle multi-byte UTF-8 char literals properly.
- **Related:** `none`

---

### `FA-SEC23-06` · `S2` · `packaging/env.sh`, `packaging/windows/package.ps1`, and `xtask/src/bundle.rs` omit `ATTRIBUTION.md` from release bundles

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `packaging/env.sh:44-54`, `packaging/windows/package.ps1:71-84`, `xtask/src/bundle.rs:71-76` (cross-check `docs/licences.md:49`, `xtask/src/msix.rs:121`, `packaging/freebsd/package.sh:56`, `packaging/linux/package.sh:57`, `packaging/macos/package.sh:124`, `packaging/web/package.sh:29`)
- **Category:** `contract-drift` / `security`
- **Defect:** Every platform packager except `xtask/src/msix.rs` copies documentation via a hardcoded filename list (`README.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE ASSETS.md`) that omits `ATTRIBUTION.md` (`xtask/src/bundle.rs` also omits `licenses/THIRD-PARTY-LICENSES.html`).
- **Failure scenario:** Despite `docs/licences.md:49` documenting `ATTRIBUTION.md` as "shipped alongside releases" and `AGENTS.md` requiring per-file asset and adapted-source attribution records alongside `NOTICE`, every built macOS `.dmg` / `CurveBook.app`, Linux `.AppImage`, FreeBSD `.tar.gz`, Web `.zip`, and Windows `.msi` ships without `ATTRIBUTION.md`.
- **Evidence:**
  ```sh
  # packaging/env.sh:44-48
  copy_docs() {
    local dest="$1" f
    for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE ASSETS.md; do
      if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
    done
  ```
  ```powershell
  # packaging/windows/package.ps1:73
  foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ASSETS.md') {
  ```
  ```rust
  // xtask/src/bundle.rs:71
  for doc in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md"] {
  ```
- **Suggested fix:** Add `ATTRIBUTION.md` to `copy_docs()` in `packaging/env.sh`, the doc copy list in `packaging/windows/package.ps1`, and `bundle.rs` (also staging `licenses/THIRD-PARTY-LICENSES.html` if present in `bundle.rs`), aligning all installers with `msix.rs:121-124`.
- **Related:** `FA-SEC23-10`

---

### `FA-SEC23-07` · `S2` · `packaging/linux/package.sh` skips `APPIMAGETOOL_SHA256` on preinstalled tools, and `packaging/macos/import-cert.sh` lacks `.p12` cleanup trap

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `packaging/linux/package.sh:84-96, 112`, `packaging/macos/import-cert.sh:10-22`
- **Category:** `security` / `cleanup`
- **Defect:**
  1. In `packaging/linux/package.sh`, `TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"` resolves an existing `appimagetool` on `PATH` or `$APPIMAGETOOL` and skips the `if [[ -z "$TOOL" ]]; then ... verify_sha256 ... fi` block entirely before executing `$TOOL` with `APPIMAGE_EXTRACT_AND_RUN=1` (unlike `RUNTIME`, which calls `verify_sha256 "$APPIMAGE_RUNTIME_SHA256" "$RUNTIME"` unconditionally).
  2. In `packaging/macos/import-cert.sh`, the base64-decoded signing certificate `.p12` written to `$RUNNER_TEMP` or `/tmp` is removed only on success at line 21 without a `trap ... EXIT` handler (unlike the inline copy in `release.yml:306`).
- **Failure scenario:** On a CI or developer environment with an unverified or trojanized `appimagetool` earlier on `PATH`, `packaging/linux/package.sh` executes it unchecked while building release `.AppImage`s; on macOS signing errors (`security import` / password mismatch / partition failure under `set -e`), `import-cert.sh` aborts leaving the unencrypted `.p12` signing private key in `/tmp` or `$RUNNER_TEMP`.
- **Evidence:**
  ```bash
  # packaging/linux/package.sh:84-96
  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [[ -z "$TOOL" ]]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage"
    ...
    verify_sha256 "$APPIMAGETOOL_SHA256" "$TOOL"
  fi
  ```
  ```bash
  # packaging/macos/import-cert.sh:14-21
  printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >"$CERT_P12"
  security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  ...
  rm -f "$CERT_P12"
  ```
- **Suggested fix:** Verify `APPIMAGETOOL_SHA256` against `$TOOL` whenever using a binary path/download (or only trust pinned `appimagetool` under `$CARGO_TARGET_DIR` unless explicitly opted out), and register `trap 'rm -f "$CERT_P12"' EXIT` immediately before decoding `$APPLE_CERTIFICATE` in `packaging/macos/import-cert.sh`.
- **Related:** `FA-SEC23-09`, `FA-SEC23-10`

---

### `FA-SEC23-08` · `S2` · `docs-site` silently falls back on missing images, stops HTML audits on unclosed quotes, and skips non-`docs/reference/` relative links & older snapshots

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/docs_site/sitemap.rs:244-250`, `xtask/src/docs_site/sitemap.rs:256-272`, `xtask/src/docs_site/verify.rs:94-96`, `xtask/src/docs_site/verify.rs:153-168`, `xtask/src/docs_site/verify.rs:408-459`
- **Category:** `wrong-result` / `security`
- **Defect:**
  1. `RepoLinkResolver::resolve_image` silently returns `images/curvebook.png` for any local Markdown image path not found in `docs/images/`.
  2. `RepoLinkResolver::resolve_href` checks broken repo links only when `source_repo_rel.starts_with("docs/reference/")`, omitting `docs/mcp-for-beginners.md` (`sitemap.rs:499`).
  3. `verify::scan_html_tags` breaks out of the HTML tag loop if `find_tag_close` returns `None` (unclosed quote inside `<...>`), leaving the remainder of the HTML file unverified against `<script>`, `on*=`, `style=`, and external URLs.
  4. `check_committed_snapshots` inspects only the single highest-versioned snapshot directory (`max_by_key`), ignoring older `docs/manual/v*/manifest.json` snapshots.
- **Failure scenario:** A broken local diagram path in any reference page silently compiles into the `curvebook.png` favicon/logo and passes `cargo xtask docs-site --check`; a broken relative repo link in `docs/mcp-for-beginners.md` compiles into a dead GitHub URL unchecked; an unclosed quote inside a malformed HTML tag causes `verify_site` to fail open on the rest of that page; and tampering with an older committed manual snapshot under `docs/manual/v0.*/` goes unnoticed once a newer minor directory exists.
- **Evidence:**
  ```rust
  // xtask/src/docs_site/sitemap.rs:244, 266-271
  if self.source_repo_rel.starts_with("docs/reference/")
      && (trimmed.ends_with(".html") || (trimmed.starts_with("docs/reference/") && !self.repo_root.join(trimmed).exists()))
  { ... }
  ...
  if let Some(file_name) = Path::new(&normalized).file_name().and_then(|n| n.to_str())
      && self.repo_root.join("docs/images").join(file_name).is_file()
  {
      return format!("{assets}images/{file_name}");
  }
  format!("{assets}images/curvebook.png")

  // xtask/src/docs_site/verify.rs:94-96
  let Some(tag_end_rel) = find_tag_close(&html[i + 1..]) else {
      break;
  };
  ```
- **Suggested fix:** Return the resolved non-existent image path from `resolve_image` (recording a broken-image error or letting `verify_target` fail), check relative repo file existence for all ingested Markdown sources including `docs/mcp-for-beginners.md`, return a parse error when `find_tag_close` encounters an unclosed `<...>` tag, and verify every snapshot directory in `check_committed_snapshots`.
- **Related:** `FA-SEC23-12`

---

### `FA-SEC23-09` · `S3` · `packaging/windows/sign.ps1` lacks fail-closed release signing enforcement and exposes PFX password in argv; `package.ps1` parses PE bytes without header/size guards

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `packaging/windows/sign.ps1:37-40`, `packaging/windows/sign.ps1:54`, `packaging/windows/package.ps1:31, 52-56`, `.github/workflows/release.yml:411-432`
- **Category:** `security` / `contract-drift` / `resources`
- **Defect:** Unlike macOS `package.sh --require-signing` (`R8.10`), `packaging/windows/sign.ps1` always exits `0` with only a warning when signing credentials are unset (`exit 0`), passes `$env:WINDOWS_CERTIFICATE_PASSWORD` on the `signtool.exe` command line (`/p`), and `packaging/windows/package.ps1` reads whole executables into RAM without validating `MZ`/`PE\0\0` signatures or array bounds in `Get-PeHeader` (and fails to strip `+build` metadata from `$MsiVersion` when no `-` prerelease exists).
- **Failure scenario:** While `docs/release/releasing.md` notes Windows Beta ships unsigned until `R8.11`, there is no opt-in/env fail-closed gate once signing is required or when non-Alpha tag releases misconfigure `WINDOWS_CERTIFICATE`/`AZURE_*` secrets—producing silently unsigned Windows `.exe`/`.msi` release assets with exit code `0`. Additionally, a `.pfx` password passed via `/p` is visible in process command lines during signing, and a truncated binary in `Get-PeHeader` throws a raw offset exception instead of a clear validation error.
- **Evidence:**
  ```powershell
  # packaging/windows/sign.ps1:37-40, 54
  if (-not $haveCert -and -not $haveAzure) {
    Write-Warn "Windows signing secrets not set (WINDOWS_CERTIFICATE or AZURE_*): leaving unsigned: $($Files -join ', ')"
    exit 0
  }
  ...
  if ($env:WINDOWS_CERTIFICATE_PASSWORD) { $signArgs += @('/p', $env:WINDOWS_CERTIFICATE_PASSWORD) }

  # packaging/windows/package.ps1:31, 52-56
  $MsiVersion = ($Version -split '-')[0]
  ...
  function Get-PeHeader([string] $Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
  }
  ```
- **Suggested fix:** Add a `-RequireSigning` switch (and `CURVEBOOK_REQUIRE_WINDOWS_SIGNING` / non-Alpha release enforcement after `R8.11`), import `.pfx` into an ephemeral user cert store (`/sha1` thumbprint instead of `/p` on argv), strip `+build` in `$MsiVersion = (($Version -split '-')[0] -split '\+')[0]`, and read only the bounded PE header while verifying `MZ`/`PE\0\0` signatures (matching `xtask/src/msix.rs:92-100`).
- **Related:** `FA-SEC23-07`, `FA-SEC23-10`

---

### `FA-SEC23-10` · `S3` · Unpinned `cargo-llvm-cov` in `coverage.yml`, unlocked builds in `release.yml`/`web`/`icons`/`bundle`, and missing `curl --proto '=https'` in `ci-setup`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `.github/workflows/coverage.yml:66, 71-73`, `.github/workflows/release.yml:129`, `.github/actions/ci-setup/action.yml:141, 148`, `packaging/web/package.sh:12`, `packaging/icons.sh:47`, `xtask/src/bundle.rs:35-41`
- **Category:** `security` / `contract-drift`
- **Defect:**
  1. `.github/workflows/coverage.yml` installs `cargo install --locked cargo-llvm-cov` without a pinned `--version`, whereas every other CI tool (`cargo-deny 0.20.2`, `cargo-about 0.9.3`, `cargo-nextest 0.9.152`, `actionlint 1.7.12`, `wix 5.0.2`) is pinned.
  2. `.github/workflows/release.yml` runs `cargo xtask contributors` without `--locked`, `packaging/web/package.sh` invokes `trunk build` without locked/offline cargo protection, `packaging/icons.sh` invokes `cargo xtask ico` without `--locked`, and `xtask/src/bundle.rs` invokes `cargo build --release` without `--locked`.
  3. `.github/actions/ci-setup/action.yml` and `.github/workflows/coverage.yml` invoke `curl -fsSL` without `--proto '=https'` / `--proto-redir '=https'`, and `ci-setup` hardcodes `x86_64-unknown-linux-gnu` / `linux_amd64` URLs.
- **Failure scenario:** An unpinned `cargo install --locked cargo-llvm-cov` downloads arbitrary latest crates.io releases on cache miss inside `coverage.yml`; unlocked release/packaging invocations can silently resolve or modify `Cargo.lock` locally; and `curl` without `--proto '=https'` permits HTTP redirects if a download endpoint returns non-HTTPS redirects.
- **Evidence:**
  ```yaml
  # .github/workflows/coverage.yml:71-73
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    cargo install --locked cargo-llvm-cov
  fi

  # .github/workflows/release.yml:129
  - name: Run cargo xtask contributors
    run: cargo xtask contributors

  # .github/actions/ci-setup/action.yml:141
  curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 60 'https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.152/cargo-nextest-0.9.152-x86_64-unknown-linux-gnu.tar.gz' -o "$tarball"
  ```
- **Suggested fix:** Pin `cargo-llvm-cov --version <exact>` in `coverage.yml`, add `--locked` to `release.yml`'s `contributors` invocation, `packaging/icons.sh`, `bundle.rs`, and `packaging/web/package.sh` (e.g. `CARGO_NET_LOCKED=true` or `--locked`), and add `--proto '=https' --proto-redir '=https'` to all CI `curl` downloads (parameterizing OS/arch if `ci-setup` is ever used on ARM64).
- **Related:** `FA-SEC23-06`, `FA-SEC23-07`, `FA-SEC23-09`

---

### `FA-SEC23-11` · `S3` · Web CSP includes `'unsafe-inline'` in `script-src`/`style-src`, and `entitlements.plist` grants `network.client` on an offline app

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `packaging/web/_headers:3`, `packaging/web/.htaccess:11`, `apps/curvebook-web/index.html:5`, `packaging/macos/entitlements.plist:6-7`
- **Category:** `security` / `contract-drift`
- **Defect:**
  1. `packaging/web/_headers`, `packaging/web/.htaccess`, and `apps/curvebook-web/index.html` permit `'unsafe-inline'` in both `script-src` (`'self' 'unsafe-inline' 'wasm-unsafe-eval'`) and `style-src` (`'self' 'unsafe-inline'`) plus `data:`/`blob:` in `img-src`.
  2. `packaging/macos/entitlements.plist` declares `com.apple.security.network.client = true` for macOS Hardened Runtime signing despite CurveBook enforcing strict zero-telemetry / zero-outbound-network operation (`AGENTS.md:65-68`).
- **Failure scenario:** `'unsafe-inline'` weakens CSP XSS defense-in-depth if any DOM injection ever occurs in the web shell (while static `docs-site` achieves `default-src 'none'` with zero `'unsafe-inline'`). On macOS, `com.apple.security.network.client = true` needlessly asserts outbound network client capability in signed app entitlements (and would permit outbound sockets if App Sandbox is ever enabled), conflicting with the documented zero-egress guarantee.
- **Evidence:**
  ```text
  # packaging/web/_headers:3
  Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'
  ```
  ```xml
  <!-- packaging/macos/entitlements.plist:5-8 -->
  <dict>
    <key>com.apple.security.network.client</key>
    <true/>
  </dict>
  ```
- **Suggested fix:** Remove `com.apple.security.network.client` from `packaging/macos/entitlements.plist` (`<dict/>` if no Hardened Runtime exemptions are needed, since loopback IPC uses no special outbound entitlement outside App Sandbox), and plan CSP tightening on web builds by externalizing Trunk's bootstrap/CSS or hashing inline blocks so `'unsafe-inline'` can be removed from `script-src`/`style-src`.
- **Related:** `FA-SEC22-06`

---

### `FA-SEC23-12` · `S3` · `xtask/src/stats.rs` ignores `--exact`, miscounts tests in comments/strings, and walks symlinks alongside `docs_site`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/stats.rs:20, 49, 62-76`, `xtask/src/docs_site/mod.rs:265`, `xtask/src/docs_site/verify.rs:512`
- **Category:** `wrong-result` / `resources`
- **Defect:**
  1. `stats::run` accepts `exact: bool` (`cargo xtask stats --exact`), passes it to `print_table(rows, _exact)`, and ignores it completely; `accumulate_source` naively matches `line.trim().starts_with("#[test]")` / `"proptest!"` inside block comments and raw strings.
  2. `stats::walk_rs`, `docs_site::collect_rel_files`, `docs_site::verify::scan_dir_secrets`, and `status::visit_rust_dir` (`status.rs:176`) follow directory symlinks via `path.is_dir()` instead of `symlink_metadata`.
- **Failure scenario:** Running `cargo xtask stats --exact` produces the identical approximate line-heuristic output as `cargo xtask stats`; raw strings or doc examples containing `"#[test]"` inflate workspace test counts; and a directory symlink cycle inside a workspace or generated `docs-site` output directory causes infinite recursion and stack/FD exhaustion.
- **Evidence:**
  ```rust
  // xtask/src/stats.rs:49, 64-76
  if path.is_dir() {
      ...
      walk_rs(&path, st)?;
  ...
  fn print_table(rows: &[PkgStats], _exact: bool) {
  ```
- **Suggested fix:** Replace `path.is_dir()` with `std::fs::symlink_metadata` / `entry.file_type()` in `stats.rs`, `docs_site/{mod.rs,verify.rs}`, and `status.rs`, and wire `--exact` in `stats.rs` to either strip comments/strings (or query `cargo test -- --list`) or reject `--exact` until implemented.
- **Related:** `FA-SEC22-18`, `FA-SEC23-04`, `FA-SEC23-08`

---

### `FA-SEC23-13` · `S3` · `screenshots` & `web_fonts` leave relative env target paths unanchored, `web_fonts` accepts Windows drive paths & brittle HTML/TOML substrings

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/screenshots.rs:51-56, 85, 102, 119-122`, `xtask/src/web_fonts.rs:29, 52-59, 118-135`
- **Category:** `wrong-result` / `security`
- **Defect:**
  1. `screenshots::nested_target_dir` and `web_fonts::run` pass relative `CARGO_TARGET_DIR` / `TRUNK_STAGING_DIR` via `PathBuf::from` without anchoring to `root` (and `screenshots.rs:85, 102` reads PNG files via unbounded `fs::read`, while `ensure_asset_listed` checks `assets_md.contains(...)` anywhere in prose rather than table rows).
  2. `web_fonts::validate_manifest_entries` rejects only `/`, `\`, and `..`, permitting drive-relative Windows paths (`C:foo.ttf`), while `check_web_bundle_readiness_with_env` checks `default-src 'self'` in `index.html` and `[profile.web-release]` in `Cargo.toml` via naive `.contains(...)` (which match HTML/TOML comments).
- **Failure scenario:** With a relative `TRUNK_STAGING_DIR` or `CARGO_TARGET_DIR` invoked outside `root`, staged fonts or nested target builds write to CWD-relative paths rather than workspace/trunk dirs; a commented `# default-src 'self'` or `# [profile.web-release]` satisfies `web-bundle` readiness checks; and an unattributed `docs/images/*.png` mentioned in prose passes `screenshots::ensure_asset_listed`.
- **Evidence:**
  ```rust
  // xtask/src/screenshots.rs:51-56, 119-121
  fn nested_target_dir(root: &Path) -> PathBuf {
      match std::env::var_os("CARGO_TARGET_DIR") {
          Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("doc-screenshots-nested"),
  ...
  fn ensure_asset_listed(rel: &str, assets_md: &str) -> Result<(), String> {
      let needle = format!("`{rel}`");
      if assets_md.contains(&needle) { Ok(()) } else { ... }

  // xtask/src/web_fonts.rs:29, 54
  let staging = std::env::var_os("TRUNK_STAGING_DIR").map_or_else(|| root.join("dist/web"), PathBuf::from).join("fonts");
  ...
  if rel.starts_with('/') || rel.starts_with('\\') || rel.contains("..") {
  ```
- **Suggested fix:** Anchor relative `CARGO_TARGET_DIR` / `TRUNK_STAGING_DIR` paths to `root` (and reject `Component::Prefix` / non-`Normal` components in `web_fonts::validate_manifest_entries`), validate non-commented CSP/profile lines in `web_fonts`, and verify `ASSETS.md` table rows (matching `assets.rs`) plus file size bounds in `screenshots.rs`.
- **Related:** `FA-SEC22-16`

---

### `FA-SEC23-14` · `S3` · Non-atomic `std::fs::write` and unbounded `std::process::Command` across non-core `xtask` subcommands violate `curvebook-platform` discipline

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/bundle.rs:35, 58, 121, 138`, `xtask/src/commands.rs:41`, `xtask/src/contributors.rs:53, 100`, `xtask/src/corpus.rs:217, 437, 524`, `xtask/src/coverage.rs:170, 201`, `xtask/src/ico.rs:41`, `xtask/src/schemas.rs:50`, `xtask/src/screenshots.rs:66`, `xtask/src/ui_commands.rs:35, 72`, `xtask/src/docs_site/mod.rs:171, 176, 256, 277, 286`, `xtask/src/docs_site/verify.rs:463`
- **Category:** `architecture` / `data-loss` / `resources`
- **Defect:** While `msix.rs` and `release_scan.rs` consistently use `curvebook_platform::write_atomic` and `ProcessCommand`, the remaining `xtask` subcommands write tracked/generated repo files (`docs/commands.md`, `contributors/contributors.json`, `docs/schemas/*.schema.json`, `crates/mcp/src/ui_catalog.rs`, `docs/manual/**`, `.ico`, `Info.plist`) directly via bare `std::fs::write` and spawn `cargo`, `git`, `curl`, `rustfmt`, `rustc`, `plutil`, and `codesign` via raw `std::process::Command` without timeouts or output caps.
- **Failure scenario:** An interrupted write or crash during `xtask ui-commands`, `commands`, `schemas`, `contributors`, or `docs-site --release` can truncate tracked repository files in place (`AGENTS.md:122`); and a stalled `rustfmt`, `git`, or `curl` invocation can hang CI or local `xtask` automation indefinitely without `ProcessCommand` wall/idle timeouts.
- **Evidence:**
  ```rust
  // xtask/src/ui_commands.rs:35, 72
  std::fs::write(&dst_path, formatted).map_err(|e| format!("write {DST_REL}: {e}"))?;
  ...
  let mut cmd = Command::new("rustfmt");

  // xtask/src/docs_site/mod.rs:171, 256, 277
  std::fs::write(snap_dir.join("manifest.json"), format!("{manifest_str}\n"))...
  ...
  let out = Command::new("git").current_dir(root).args(["rev-parse", "HEAD"]).output().ok()?;
  ```
- **Suggested fix:** Route generated-file writes through `curvebook_platform::write_atomic` and replace raw `std::process::Command` helper calls in `xtask` with bounded `curvebook_platform::process::ProcessCommand`.
- **Related:** `FA-SEC22-17`

---

### `FA-SEC23-15` · `S4` · Hand-rolled TOML parsers in `packaging/env.sh`, `contributors.rs`, and `cursors.rs` misread comments, single quotes, or non-cursor tables

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `packaging/env.sh:15-20`, `xtask/src/contributors.rs:109-178`, `xtask/src/cursors.rs:357-371`
- **Category:** `wrong-result`
- **Defect:**
  1. `packaging/env.sh::workspace_version` matches `in_pkg && $1 == "version"` and cleans `$3` via whitespace split, failing if `[workspace.package]` has no spaces around `=` (`version="..."`), trailing `# comment`, or `[workspace.package] # comment`.
  2. `contributors::parse_people_toml` splits on `=`, handles only `"..."` double-quoted strings, and breaks if a key/table header uses single quotes or `[people.'login']` with inline comments.
  3. `cursors::parse_manifest_names` collects `name = "..."` from *any* TOML table in `crates/tools/src/cursors.toml` rather than restricting to `[[cursor]]` arrays-of-tables (and ignores single-quoted strings).
- **Failure scenario:** Adding a metadata `[meta] name = "..."` section to `cursors.toml` triggers a false stale-cursor failure in `cargo xtask cursors`; formatting `Cargo.toml` with inline comments or compact `=` silently yields empty/corrupted `$VERSION` in packaging scripts (`workspace_version` fails).
- **Evidence:**
  ```sh
  # packaging/env.sh:15-20
  workspace_version() {
    awk '
      /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
      in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
    ' "$ROOT/Cargo.toml"
  }
  ```
  ```rust
  // xtask/src/cursors.rs:357-368
  fn parse_manifest_names(toml: &str) -> Vec<String> {
      let mut out = Vec::new();
      for line in toml.lines() {
          let stripped = strip_toml_comment(line).trim();
          if let Some((key, val)) = stripped.split_once('=') && key.trim() == "name" {
  ```
- **Suggested fix:** Match `cargo xtask version` in `packaging/env.sh` (`cargo run --locked -q --no-default-features -p xtask -- version` or strip `#` comments and parse `key=val` cleanly), track `in_cursor_table` in `cursors.rs`, and handle literal `'...'` strings in `contributors.rs` and `cursors.rs`.
- **Related:** `FA-SEC22-17`

---

### `FA-SEC23-16` · `S4` · Minor packaging/manifest cleanup: redundant workspace member, `--arch` missing-value abort, and unpruned draft assets in `release.yml`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `Cargo.toml:3`, `packaging/macos/package.sh:23`, `.github/workflows/release.yml:680`
- **Category:** `cleanup`
- **Defect:**
  1. `Cargo.toml:3` lists both `"crates/*"` and `"crates/platform"` in `members`.
  2. `packaging/macos/package.sh:23` reads `--arch) ARCH="$2"` without checking `$# -ge 2` (unlike `packaging/macos/verify.sh:23`), causing an `unbound variable` bash abort under `set -u` instead of a clear CLI error.
  3. `.github/workflows/release.yml:680` updates an existing draft release via `gh release upload "$tag" dist/release/* --clobber`, overwriting matching names without removing stale assets from an earlier failed/changed run on the same draft tag.
- **Failure scenario:** Invoking `packaging/macos/package.sh --arch` prints a confusing shell `unbound variable` message rather than a usage error; re-running a draft release after dropping or renaming an artifact leaves stale assets attached alongside the newly attested set.
- **Evidence:**
  ```toml
  # Cargo.toml:3
  members = ["crates/*", "crates/platform", "apps/*", "xtask"]
  ```
  ```bash
  # packaging/macos/package.sh:23
  --arch) ARCH="$2"; shift 2 ;;
  ```
- **Suggested fix:** Remove redundant `"crates/platform"` from `Cargo.toml:3` (since `crates/*` already covers it), add `[[ $# -ge 2 ]]` validation to `--arch` in `packaging/macos/package.sh`, and prune stale non-matching assets when refreshing an existing draft release in `release.yml`.
- **Related:** `none`
