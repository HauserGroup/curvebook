# Verification Report: SEC-23 — Xtask Docs/Release, Testkit, Packaging, CI Workflows & Root Configs

- **Section:** `SEC-23` (`96` files: `16` remaining `xtask/src/` modules, `6` `xtask/src/docs_site/` modules, `11` `crates/testkit/` files, `32` `packaging/**` files including `.shellcheckrc` & `.htaccess`, `17` `.github/**` files, and `14` root configs/docs; `22,036` total LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-23.md`](../../../docs/audit/full-2026-11-22/SEC-23.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-3`
- **Date:** `2026-11-22`

## Summary Table

| Verdict | Count | Findings |
|---|---:|---|
| `CONFIRMED` | 16 | `FA-SEC23-01` .. `FA-SEC23-16` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity (Unique) | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 8 | `FA-SEC23-01` .. `FA-SEC23-08` |
| `S3` | 6 | `FA-SEC23-09` .. `FA-SEC23-14` |
| `S4` | 2 | `FA-SEC23-15`, `FA-SEC23-16` |

## File Coverage Completeness Check

Verified `96 / 96` scoped files (`22,036` total LOC, matching `wc -l` line-for-line at `7c85a8e516165ddadb101236a88eec937bf1f095`; remaining `15` `xtask/src/` modules and `xtask/Cargo.toml` are verified in [`SEC-22-verdicts.md`](../../../docs/audit/full-2026-11-22/SEC-22-verdicts.md)) appear in [`SEC-23.md`](../../../docs/audit/full-2026-11-22/SEC-23.md#L10-L110) with exact line counts and `1..=N` inspection ranges (`PASS`):

- **Remaining `xtask/src/` (`16` files, `9,787` LOC):** `bundle.rs` (`240`), `citations.rs` (`1,139`), `commands.rs` (`108`), `contributors.rs` (`343`), `corpus.rs` (`746`), `coverage.rs` (`291`), `cursors.rs` (`446`), `ico.rs` (`108`), `msix.rs` (`273`), `release_scan.rs` (`2,184`), `schemas.rs` (`168`), `screenshots.rs` (`329`), `stats.rs` (`109`), `status.rs` (`2,355`), `ui_commands.rs` (`118`), `web_fonts.rs` (`273`).
- **`xtask/src/docs_site/` (`6` files, `4,146` LOC):** `markdown.rs` (`743`), `mod.rs` (`292`), `registries.rs` (`1,235`), `sitemap.rs` (`923`), `template.rs` (`320`), `verify.rs` (`633`).
- **`crates/testkit/` (`11` files, `3,630` LOC):** `Cargo.toml` (`16`), `src/dd.rs` (`201`), `src/lib.rs` (`47`), `src/metrics.rs` (`96`), `src/r13_datasets.rs` (`611`), `src/sha256.rs` (`257`), `src/wf_datasets.rs` (`980`), `src/nist_strd/corpus.rs` (`558`), `src/nist_strd/datasets.rs` (`459`), `src/nist_strd/mod.rs` (`23`), `src/nist_strd/parser.rs` (`939`).
- **`packaging/**` (`32` files, `2,183` LOC):** `.shellcheckrc` (`7`), `check-app-specific.sh` (`69`), `env.sh` (`68`), `icons.sh` (`65`), `release-channel.sh` (`63`), `freebsd/package.sh` (`63`), `linux/apprun-test.sh` (`47`), `linux/org.curvebook.CurveBook.desktop` (`20`), `linux/org.curvebook.CurveBook.metainfo.xml.in` (`61`), `linux/org.curvebook.CurveBook.mime.xml` (`25`), `linux/package.sh` (`121`), `macos/Info.plist.in` (`144`), `macos/entitlements.plist` (`9`), `macos/import-cert.sh` (`34`), `macos/package.sh` (`171`), `macos/verify.sh` (`219`), `web/.htaccess` (`24`), `web/README.md` (`25`), `web/_headers` (`42`), `web/package.sh` (`33`), `windows/AppxManifest.xml.in` (`44`), `windows/INSTALLATION.md` (`27`), `windows/check-icons.ps1` (`131`), `windows/check-install-policy.ps1` (`171`), `windows/curvebook.en-us.wxl` (`11`), `windows/curvebook.wxs` (`103`), `windows/install-policy.json` (`45`), `windows/install-policy.wxi` (`58`), `windows/package.ps1` (`138`), `windows/sign.ps1` (`85`), `windows/store-identity.json` (`5`), `windows/test-install-policy.ps1` (`55`).
- **`.github/**` (`17` files, `1,769` LOC):** `CODEOWNERS` (`7`), `actionlint.yaml` (`8`), `dependabot.yml` (`46`), `pull_request_template.md` (`27`), `ISSUE_TEMPLATE/bug_report.yml` (`76`), `ISSUE_TEMPLATE/config.yml` (`8`), `ISSUE_TEMPLATE/feature_request.yml` (`49`), `ISSUE_TEMPLATE/numerical_result.yml` (`89`), `ISSUE_TEMPLATE/roadmap_task.yml` (`60`), `actions/ci-setup/action.yml` (`156`), `workflows/ci.yml` (`211`), `workflows/coverage.yml` (`89`), `workflows/freebsd.yml` (`45`), `workflows/packaging-lint.yml` (`88`), `workflows/release.yml` (`685`), `workflows/windows-arm64.yml` (`50`), `workflows/windows-store.yml` (`129`).
- **Root configs & docs (`14` files, `521` LOC):** `Cargo.toml` (`101`), `clippy.toml` (`15`), `deny.toml` (`45`), `about.toml` (`15`), `about.hbs` (`102`), `rust-toolchain.toml` (`4`), `rustfmt.toml` (`2`), `.cargo/config.toml` (`8`), `.config/nextest.toml` (`19`), `.githooks/pre-commit` (`3`), `.coderabbit.yaml` (`17`), `ASSETS.md` (`63`), `ATTRIBUTION.md` (`24`), `NOTICE` (`49`).

---

## Per-Finding Verdicts

### Verdict `FA-SEC23-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Untrusted `.AppImage` binary execution fallback, pre-extraction archive link/traversal gaps, and unbounded reads inside the release container scanner.
- **Evidence:**
  - [`xtask/src/release_scan.rs:177, 1185-1220`](../../../xtask/src/release_scan.rs#L1185-L1220): `ToolConfig::default()` sets `allow_appimage_exec: cfg!(target_os = "linux")` (`:177`), and `ToolConfig::from_env()` (`:185-192`) never clears it. When `find_squashfs_offset` returns `None` or `unsquashfs` fails, `unpack_appimage` writes `runner.AppImage`, marks it `0o755` via `set_owner_executable` (`:1210, 1229-1234`), and runs `APPIMAGE_EXTRACT_AND_RUN=1` `runner.AppImage --appimage-extract` (`:1212`).
  - [`xtask/src/release_scan.rs:971-981, 1047-1071, 1092-1135, 1433-1475`](../../../xtask/src/release_scan.rs#L971-L1475): `unpack_tarball` passes `"-tzf"`/`"-tf"` (`:971`), and `unpack_rpm` passes `bsdtar -tf` (`:1092`) and `cpio -it` (`:1121`), none of which emit entry type indicators (`l`/`h`) or link targets for `validate_archive_listing` (`:1433-1475`); `scan_tree_symlinks` (`:918-943`) runs only after extraction (`:886`) and checks brand names rather than symlink confinement or hardlinks. `extract_deb_ar_members` (`:1051, 1060`) invokes `ar -x` and `bsdtar -xf` before inspecting member names.
  - [`xtask/src/release_scan.rs:768, 782, 946, 1174, 1347`](../../../xtask/src/release_scan.rs#L768-L1347): `scan_single_asset` (`Checksums`/`HtmlCompliance` at `:768`, `Zsync` at `:782`), `unpack_zip` (`:946`, prior to `ZipArchive::from_bytes_with_limits` at `:947`), `unpack_appimage` (`:1174`), and `copy_mounted_tree_no_symlinks` (`:1347`) call `std::fs::read` without checking `metadata().len()` against any size cap first. (Distinct from `FA-SEC22-12` and `FA-SEC22-18`, which cover `brands.rs`, `cleanroom.rs`, `assets.rs`, `telemetry.rs`, `layers.rs`, and `licenses.rs`.)

---

### Verdict `FA-SEC23-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Manifest path traversal, tautological size validation, and unbounded pre-verification `std::fs::read` in corpus verification helpers.
- **Evidence:**
  - [`xtask/src/corpus.rs:290-299, 362, 390-397`](../../../xtask/src/corpus.rs#L290-L397): In `verify_manifest_file`, the `[sha_hex, rel]` branch only checks `pinned_spec_for_rel(clean_rel)` when `enforce_pinned && clean_rel.ends_with(".dat")`, passes `clean_rel` straight to `nist_root.join(clean_rel)` without checking for leading `/`, `..`, or symlinks, reads the target unboundedly via `fs::read` without checking `MAX_STRD_BYTES`, and calls `verify_blob(&bytes, bytes.len(), sha_hex)` (`:296`), passing `bytes.len()` as `expected_size`.
  - [`xtask/src/corpus.rs:380-384`](../../../xtask/src/corpus.rs#L380-L384) and [`crates/testkit/src/sha256.rs:230-235`](../../../crates/testkit/src/sha256.rs#L230-L235): Both unmanifested `.dat` verification (`corpus.rs:380`, checking `bytes.len() > MAX_STRD_BYTES` at `:381`) and `load_verified_corpus_file` (`sha256.rs:232`, calling `verify_blob` at `:233`, invoked directly by `corpus.rs:287, 346` and [`crates/testkit/src/nist_strd/corpus.rs`](../../../crates/testkit/src/nist_strd/corpus.rs#L13)) read the entire file via `std::fs::read` before inspecting file size.

---

### Verdict `FA-SEC23-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Workspace Clippy policy gap leaving unchecked slice/string indexing undetected across shipped crates.
- **Evidence:**
  - [`clippy.toml:1-6`](../../../clippy.toml#L1-L6) explicitly configures `allow-indexing-slicing-in-tests = true` with the comment `"Tests may unwrap, expect, panic and index/slice directly... Shipped code may not (workspace lints in Cargo.toml; AGENTS.md › Non-negotiables)"`, and [`AGENTS.md:45-49`](../../../AGENTS.md#L45-L49) requires checked slice access via `.get(...)`.
  - [`Cargo.toml:66-80`](../../../Cargo.toml#L66-L80) denies `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `unreachable`, but omits `indexing_slicing = "deny"` (and `string_slice = "deny"`), permitting unchecked indexing such as `&trimmed[1..trimmed.len() - 1]` in [`xtask/src/assets.rs:193`](../../../xtask/src/assets.rs#L193) (`FA-SEC22-13`) to pass `cargo clippy --workspace --all-targets -- -D warnings`.

---

### Verdict `FA-SEC23-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Multiple evidence-verification and completion-lint bypasses in the `cargo xtask status` policy gate.
- **Evidence:**
  - [`xtask/src/status.rs:194-244`](../../../xtask/src/status.rs#L194-L244): `scan_tests_in_rust_source` skips only `//` line comments (`:201`) without lexing `/* ... */` block comments or raw strings, so commented or raw-string `#[test] fn ...` fixtures populate `WorkspaceTests`.
  - [`xtask/src/status.rs:908-958, 1165, 1302-1363`](../../../xtask/src/status.rs#L908-L1363): `scan_human_wf_completion` reads only top-level `crates/ui-egui/tests/human_wf_*.rs` files (`:914-924`, skipping subdirectories); `extract_rust_functions` inspects functions only at `brace_depth == 0` (`:1165`, skipping `mod tests { ... }`); `analyze_human_wf_sources` skips `check_workflow_run_lint` whenever `"WorkflowRun"` does not appear literally in `func.header_and_body_code` (`:944`); `has_finish` (`:1302-1305`) accepts any `.finish()`; and `has_guarded_find_by_widget_id` (`:1319-1363`) checks only `if`/`if let` conditions.
  - [`xtask/src/status.rs:2133-2140`](../../../xtask/src/status.rs#L2133-L2140): `count_live_ui_commands` locates the end of `UI_COMMANDS` using `slice_tail.find("];")` (`:2138`), which matches inside string literals (such as `"[]];"` tested in [`xtask/src/ui_commands.rs:98`](../../../xtask/src/ui_commands.rs#L98)) and truncates the count.

---

### Verdict `FA-SEC23-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `D1` rule-1 citation gate silently skips inherent `impl` blocks carrying `for<'a>` bounds and mis-lexes multi-byte UTF-8 char literals.
- **Evidence:**
  - [`xtask/src/citations.rs:469-471, 530-557`](../../../xtask/src/citations.rs#L469-L557): `inspect_item` calls `enter_or_skip_container(..., reject_trait_for = true)` on `"impl"`, and line `:542` sets `saw_for = true` for any `Token::Ident { text == "for" }` between `impl` and `{` without distinguishing `where`/HRTB `for<'a>` bounds from `impl Trait for Type`. Line `:548` then skips `parse_container_items`, ignoring all `pub fn` methods inside that `impl`.
  - [`xtask/src/citations.rs:756-757, 917-930`](../../../xtask/src/citations.rs#L917-L930): For non-escaped char literals, `skip_char_or_lifetime` checks only `bytes.get(start + 2) == Some(&b'\'')` (`:926`), returning `start + 1` on multi-byte UTF-8 char literals and leaving the closing `'` unpaired in the token stream.

---

### Verdict `FA-SEC23-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Legal/attribution policy drift omitting `ATTRIBUTION.md` from all non-MSIX release packages and local macOS app bundles.
- **Evidence:**
  - [`packaging/env.sh:44-54`](../../../packaging/env.sh#L44-L54) (`copy_docs()`, called by `freebsd/package.sh:56`, `linux/package.sh:57`, `macos/package.sh:124`, `web/package.sh:29`), [`packaging/windows/package.ps1:71-84`](../../../packaging/windows/package.ps1#L71-L84) (copied into `$(var.DocsDir)` harvested at [`packaging/windows/curvebook.wxs:70`](../../../packaging/windows/curvebook.wxs#L70)), and [`xtask/src/bundle.rs:71-76`](../../../xtask/src/bundle.rs#L71-L76) all stage `README.md`, `LICENSE*`, `NOTICE`, and `ASSETS.md` while omitting `ATTRIBUTION.md` (`bundle.rs` also omits `licenses/THIRD-PARTY-LICENSES.html`).
  - Cross-checked against [`docs/licences.md:49`](../../../docs/licences.md#L49) (`ATTRIBUTION.md` documented as `"shipped alongside releases"`), [`AGENTS.md:25`](../../../AGENTS.md#L25), and [`xtask/src/msix.rs:121-124`](../../../xtask/src/msix.rs#L121-L124) (which explicitly stages `ATTRIBUTION.md` and `THIRD-PARTY-LICENSES.html`).

---

### Verdict `FA-SEC23-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Unverified `appimagetool` execution when preinstalled/`$APPIMAGETOOL` is set, and private `.p12` key left on disk upon certificate-import failure.
- **Evidence:**
  - [`packaging/linux/package.sh:84-96, 112`](../../../packaging/linux/package.sh#L84-L112): `TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"` skips the entire `if [[ -z "$TOOL" ]]; then ... verify_sha256 "$APPIMAGETOOL_SHA256" "$TOOL"; fi` block whenever `$APPIMAGETOOL` or a `PATH` `appimagetool` exists, then executes `APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL"` at `:112` without checking `$APPIMAGETOOL_SHA256` (contrast `RUNTIME`, verified unconditionally at `:105`).
  - [`packaging/macos/import-cert.sh:10-22`](../../../packaging/macos/import-cert.sh#L10-L22): Decodes `$APPLE_CERTIFICATE` into `$CERT_P12` (`:14`) under `set -euo pipefail` (`:4`) and deletes it only at `:21` with no `trap 'rm -f "$CERT_P12"' EXIT` handler (contrast the inline step in [`.github/workflows/release.yml:306`](../../../.github/workflows/release.yml#L306)).

---

### Verdict `FA-SEC23-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Fail-open verification and silent fallbacks across the `docs-site` link, image, HTML/CSP, and snapshot gates.
- **Evidence:**
  - [`xtask/src/docs_site/sitemap.rs:256-272`](../../../xtask/src/docs_site/sitemap.rs#L256-L272): `RepoLinkResolver::resolve_image` silently returns `format!("{assets}images/curvebook.png")` (`:271`) whenever a local Markdown image path is outside `docs/images/` and its basename is missing from `docs/images/`.
  - [`xtask/src/docs_site/sitemap.rs:244-250, 500`](../../../xtask/src/docs_site/sitemap.rs#L244-L250): `RepoLinkResolver::resolve_href` checks broken relative repo links only when `self.source_repo_rel.starts_with("docs/reference/")` (`:244`), omitting `docs/mcp-for-beginners.md` (`:500`) and externalizing unresolved relative paths into GitHub URLs (`:253`) skipped by `verify.rs:235`.
  - [`xtask/src/docs_site/verify.rs:94-96, 153-168`](../../../xtask/src/docs_site/verify.rs#L94-L168): `scan_html_tags` executes `break` when `find_tag_close` returns `None` (unclosed quote inside `<...>`), leaving the remainder of the HTML file unchecked for `<script>`, `on*=`, `style=`, and external URLs.
  - [`xtask/src/docs_site/verify.rs:408-459`](../../../xtask/src/docs_site/verify.rs#L408-L459): `check_committed_snapshots` selects only `snapshots.into_iter().max_by_key(|(v, _)| *v)` (`:425`), skipping verification of all older `docs/manual/v*/manifest.json` snapshots.

---

### Verdict `FA-SEC23-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Windows signing script lacks opt-in fail-closed enforcement and exposes PFX passwords on argv; `package.ps1` parses PE bytes and MSI versions without bounds/signature or `+build` guards.
- **Evidence:**
  - [`packaging/windows/sign.ps1:37-40, 54`](../../../packaging/windows/sign.ps1#L37-L54): Exits `0` unconditionally with a warning when `WINDOWS_CERTIFICATE` and `AZURE_*` are unset (`:38-39`; Beta intentionally ships unsigned per [`docs/release/releasing.md:15`](../../../docs/release/releasing.md#L15), with no `-RequireSigning`/env fail-closed switch for post-`R8.11` or tagged verification) and passes `$env:WINDOWS_CERTIFICATE_PASSWORD` in `signtool.exe` argv via `/p` (`:54`).
  - [`packaging/windows/package.ps1:31, 52-56`](../../../packaging/windows/package.ps1#L31-L56): `$MsiVersion = ($Version -split '-')[0]` (`:31`) preserves `+build` metadata when no `-` prerelease exists, and `Get-PeHeader` (`:52-56`) reads the entire executable via `ReadAllBytes` and indexes `0x3C` / `$pe + 4` / `$pe + 0x5C` without checking `MZ`, `PE\0\0`, or file length bounds (contrast [`xtask/src/msix.rs:92-100`](../../../xtask/src/msix.rs#L92-L100)).

---

### Verdict `FA-SEC23-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Unpinned `cargo-llvm-cov`, unlocked Cargo/Trunk invocations across release/packaging scripts, and `curl` calls omitting `--proto '=https'` (mitigated by SHA-256 verification on downloaded tarballs).
- **Evidence:**
  - [`.github/workflows/coverage.yml:66, 71-73`](../../../.github/workflows/coverage.yml#L66-L73): `cargo install --locked cargo-llvm-cov` (`:72`) omits `--version <exact>`, and `curl -fsSL` (`:66`) omits `--proto '=https' --proto-redir '=https'`.
  - [`.github/actions/ci-setup/action.yml:141, 148`](../../../.github/actions/ci-setup/action.yml#L141-L148): `curl -fsSL` omits `--proto '=https' --proto-redir '=https'` and hardcodes `x86_64-unknown-linux-gnu` / `linux_amd64` URLs (both tarballs are SHA-256 checked immediately at `:142, 149`).
  - [`.github/workflows/release.yml:129`](../../../.github/workflows/release.yml#L129) (`cargo xtask contributors`), [`packaging/web/package.sh:12`](../../../packaging/web/package.sh#L12) (`trunk build` without locked/offline cargo enforcement), [`packaging/icons.sh:47`](../../../packaging/icons.sh#L47) (`cargo xtask ico`), and [`xtask/src/bundle.rs:35-41`](../../../xtask/src/bundle.rs#L35-L41) (`cargo build --release`) omit `--locked`.

---

### Verdict `FA-SEC23-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Defense-in-depth and policy-posture drift in web CSP (`'unsafe-inline'`) and macOS Hardened Runtime entitlements (`network.client`).
- **Evidence:**
  - [`packaging/web/_headers:3`](../../../packaging/web/_headers#L3), [`packaging/web/.htaccess:11`](../../../packaging/web/.htaccess#L11), and [`apps/curvebook-web/index.html:5`](../../../apps/curvebook-web/index.html#L5) specify `script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'`, `style-src 'self' 'unsafe-inline'`, and `img-src 'self' data: blob:` (note [`xtask/src/telemetry.rs:73-74`](../../../xtask/src/telemetry.rs#L73-L74) does not forbid `'unsafe-inline'`).
  - [`packaging/macos/entitlements.plist:5-8`](../../../packaging/macos/entitlements.plist#L5-L8) grants `<key>com.apple.security.network.client</key><true/>` despite CurveBook's strict zero-egress architecture ([`AGENTS.md:65-68`](../../../AGENTS.md#L65-L68)) and loopback-only `--control` IPC; distinct from `SEC-20` findings.

---

### Verdict `FA-SEC23-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `xtask stats --exact` ignores `--exact`, miscounts tests in comments/raw strings, and four walkers follow directory symlinks via `path.is_dir()`.
- **Evidence:**
  - [`xtask/src/stats.rs:20-24, 49, 62-76`](../../../xtask/src/stats.rs#L20-L76): `stats::run` forwards `exact` into `print_table(&rows, exact)` whose parameter is `_exact: bool` (`:76`); `accumulate_source` matches `line.trim().starts_with("#[test]")` / `"proptest!"` inside block comments and raw strings.
  - [`xtask/src/stats.rs:49`](../../../xtask/src/stats.rs#L49), [`xtask/src/docs_site/mod.rs:265`](../../../xtask/src/docs_site/mod.rs#L265), [`xtask/src/docs_site/verify.rs:512`](../../../xtask/src/docs_site/verify.rs#L512), and [`xtask/src/status.rs:176`](../../../xtask/src/status.rs#L176) all recurse on `path.is_dir()` (which follows directory symlinks) instead of checking `symlink_metadata` / `file_type()`.

---

### Verdict `FA-SEC23-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Unanchored relative env target paths in `screenshots`/`web_fonts`, Windows drive-prefix path acceptance in `web_fonts`, and substring readiness/attribution checks.
- **Evidence:**
  - [`xtask/src/screenshots.rs:51-56, 85, 102, 119-122`](../../../xtask/src/screenshots.rs#L51-L122): `nested_target_dir` wraps relative `CARGO_TARGET_DIR` via `PathBuf::from(dir)` without anchoring to `root`; `:85` and `:102` call unbounded `fs::read(&abs)`; `ensure_asset_listed` (`:119-122`) checks `assets_md.contains(&format!("`{rel}`"))` anywhere in prose rather than table rows.
  - [`xtask/src/web_fonts.rs:29, 52-59, 111-135`](../../../xtask/src/web_fonts.rs#L29-L135): `web_fonts::run` uses `PathBuf::from` on relative `TRUNK_STAGING_DIR` without anchoring to `root`; `validate_manifest_entries` (`:52-59`) checks only `/`, `\`, and `..` (accepting Windows drive-relative paths like `C:foo.ttf`); and `check_web_bundle_readiness_with_env` (`:111-135`) uses `.contains(...)` for `Trunk.toml`, `index.html`, and `Cargo.toml`, matching commented lines.

---

### Verdict `FA-SEC23-14` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Non-atomic `std::fs::write` and unbounded `std::process::Command` across non-core `xtask` subcommands (`AGENTS.md:122`).
- **Evidence:**
  - Verified bare `std::fs::write` and raw `std::process::Command` (instead of `curvebook_platform::{write_atomic, process::ProcessCommand}`) at every cited location:
    - [`xtask/src/bundle.rs:35, 58, 121, 138`](../../../xtask/src/bundle.rs#L35-L138) (`Command::new` for `cargo`/macOS tool/`git`, `std::fs::write` for `Info.plist`)
    - [`xtask/src/commands.rs:41`](../../../xtask/src/commands.rs#L41) (`fs::write` on `docs/commands.md`)
    - [`xtask/src/contributors.rs:53, 100`](../../../xtask/src/contributors.rs#L53-L100) (`std::fs::write` on `contributors/contributors.json`, `Command::new("git")`)
    - [`xtask/src/corpus.rs:217, 437, 524`](../../../xtask/src/corpus.rs#L217-L525) (`Command::new("cargo")` / `"curl"`, `fs::write` + `fs::rename` without `sync_all`)
    - [`xtask/src/coverage.rs:170, 201`](../../../xtask/src/coverage.rs#L170-L201) (`Command::new("cargo")` / `"rustc"`)
    - [`xtask/src/ico.rs:41`](../../../xtask/src/ico.rs#L41) (`std::fs::write`)
    - [`xtask/src/schemas.rs:50`](../../../xtask/src/schemas.rs#L50) (`fs::write` on `docs/schemas/*.schema.json`)
    - [`xtask/src/screenshots.rs:66`](../../../xtask/src/screenshots.rs#L66) (`Command::new("cargo")`)
    - [`xtask/src/ui_commands.rs:35, 72`](../../../xtask/src/ui_commands.rs#L35-L72) (`std::fs::write` on `crates/mcp/src/ui_catalog.rs`, `Command::new("rustfmt")`)
    - [`xtask/src/docs_site/mod.rs:171, 176, 256, 277, 286`](../../../xtask/src/docs_site/mod.rs#L171-L286) (`std::fs::write` on `manifest.json`/`index.html`/pages, `Command::new("git")`)
    - [`xtask/src/docs_site/verify.rs:463`](../../../xtask/src/docs_site/verify.rs#L463) (`Command::new("git")`)

---

### Verdict `FA-SEC23-15` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Brittle hand-rolled TOML parsing in `packaging/env.sh`, `contributors.rs`, and `cursors.rs`.
- **Evidence:**
  - [`packaging/env.sh:15-20`](../../../packaging/env.sh#L15-L20): `workspace_version` checks `$0 == "[workspace.package]"` and `$1 == "version"` then prints `$3`, failing on compact `version="..."` or inline `# comment` on `[workspace.package]`.
  - [`xtask/src/contributors.rs:109-178`](../../../xtask/src/contributors.rs#L109-L178): `parse_people_toml`, `strip_toml_comment`, and `parse_quoted_value` handle only double-quoted `"..."` keys/values, ignoring `'...'` single-quoted TOML strings/keys.
  - [`xtask/src/cursors.rs:357-371`](../../../xtask/src/cursors.rs#L357-L371): `parse_manifest_names` collects `name = "..."` from any TOML table without tracking `[[cursor]]` headers and ignores single-quoted strings.

---

### Verdict `FA-SEC23-16` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Minor manifest, CLI argument guard, and draft release upload cleanup items.
- **Evidence:**
  - [`Cargo.toml:3`](../../../Cargo.toml#L3): `members = ["crates/*", "crates/platform", "apps/*", "xtask"]` redundantly lists `"crates/platform"` alongside `"crates/*"`.
  - [`packaging/macos/package.sh:23`](../../../packaging/macos/package.sh#L23): `--arch) ARCH="$2"; shift 2 ;;` lacks a `$# -ge 2` check under `set -u` (`:7`), triggering a raw `unbound variable` error when `--arch` is passed without a value (contrast [`packaging/macos/verify.sh:23`](../../../packaging/macos/verify.sh#L23)).
  - [`.github/workflows/release.yml:680`](../../../.github/workflows/release.yml#L680): `gh release upload "$tag" dist/release/* --clobber` overwrites matching filenames on an existing draft release without deleting stale assets whose filenames are absent from the refreshed `dist/release/*` set.
