# Security & Engineering Audit — SEC-22 (Xtask Policy Gates & CI)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-22` — `xtask/Cargo.toml` (`1` manifest) and `15` modules in `xtask/src/` (`main.rs`, `assets.rs`, `brands.rs`, `ci.rs`, `ci_cache.rs`, `ci_quick.rs`, `cleanroom.rs`, `layers.rs`, `licenses.rs`, `pending.rs`, `secrets.rs`, `telemetry.rs`, `text_decode.rs`, `version.rs`, `workflows.rs`) (`16` files, `15,891` LOC)
- **Date:** `2026-11-22`
- **Summary:** `S1=0`, `S2=15`, `S3=3`, `S4=1` (Total: `19` findings: `FA-SEC22-01` .. `FA-SEC22-19`)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings / Status |
| --- | ---: | --- | --- | --- |
| `xtask/Cargo.toml` | 51 | `1..=51` | `[lints] workspace = true`, workspace edition/MSRV/license inheritance, `full-gates` optional dependency isolation, zero HTTP/telemetry/`chrono` crates | Clean — workspace lints enforced and zero network/telemetry/`chrono` dependencies |
| `xtask/src/main.rs` | 494 | `1..=494` | `#![forbid(unsafe_code)]`, `repo_files` non-ASCII/broken-symlink/deleted-file handling, `cmd_i18n_report` exact single-test guard, `cargo_metadata` locking, `CARGO_TARGET_DIR` resolution in `cmd_wasm` | [`FA-SEC22-10`](#fa-sec22-10), [`FA-SEC22-16`](#fa-sec22-16) |
| `xtask/src/assets.rs` | 1,039 | `1..=1039` | Zero-bundled-font ban, `ASSETS.md`/`ATTRIBUTION.md`/`NOTICE` parser & SPDX allowlist, provenance cycle/URL checks, original-visual-asset rules, binary/SVG metadata dispatch, `dist/` archive recursion & Flatpak icon cache check | [`FA-SEC22-07`](#fa-sec22-07), [`FA-SEC22-12`](#fa-sec22-12), [`FA-SEC22-13`](#fa-sec22-13), [`FA-SEC22-18`](#fa-sec22-18) |
| `xtask/src/brands.rs` | 5,711 | `1..=5711` | UTF-8/UTF-16 decoding order, tracked path & extension coverage, Rust lexer comments/strings/escapes/idents, TOML scanner, SVG/HTML/PDF/PNG/ICO/ICNS/GIF metadata, MCP/UI/i18n sweeps, `dist/` scanning | [`FA-SEC22-02`](#fa-sec22-02), [`FA-SEC22-07`](#fa-sec22-07), [`FA-SEC22-08`](#fa-sec22-08), [`FA-SEC22-12`](#fa-sec22-12), [`FA-SEC22-18`](#fa-sec22-18), [`FA-SEC22-19`](#fa-sec22-19) |
| `xtask/src/ci.rs` | 223 | `1..=223` | `HOSTED_STAGES` vs `.github/workflows/ci.yml` parity, `PERFORMANCE_FILTER` exact complement split, `RUSTDOCFLAGS="-D warnings"`, archived nextest `--workspace-remap`/`--extract-to`, `--no-default-features` stage | Clean — complementary native/performance filters, strict rustdoc warnings, archive execution parity, and no-default-features coverage |
| `xtask/src/ci_cache.rs` | 1,199 | `1..=1199` | Cargo JSONL active-unit extraction, top-level binary fingerprint matching, `deps/`/`build/`/`.fingerprint/` pruning, `restore_mtimes` shallow-clone tree-hash fallback, `write_atomic`, `CARGO_TARGET_DIR` resolution | [`FA-SEC22-01`](#fa-sec22-01), [`FA-SEC22-16`](#fa-sec22-16) |
| `xtask/src/ci_quick.rs` | 324 | `1..=324` | `merge-base` diff + untracked discovery, `WORKSPACE_WIDE_FILES`/`WORKSPACE_WIDE_DIRS` completeness vs cross-crate `include_bytes!`/`include_str!`/test inputs, reverse-dependency closure, `package_dirs` workspace filtering | [`FA-SEC22-10`](#fa-sec22-10) |
| `xtask/src/cleanroom.rs` | 711 | `1..=711` | Vendor notice/creator proximity matching, UTF-16 BOM/BOM-less decoding, ZIP recursion & `CLEANROOM_ZIP_LIMITS` handling, `docs/audit/` & `docs/history/audit/` skip rules, upstream-attribution scoping, `dist/` scanning | [`FA-SEC22-12`](#fa-sec22-12), [`FA-SEC22-18`](#fa-sec22-18) |
| `xtask/src/layers.rs` | 1,389 | `1..=1389` | `L0`–`L7` + `INTRA_LAYER_ORDER` + `Testkit`/`Exempt` graph rules, UI-crate ban below `L6`, `#[cfg(test)]` predicate evaluation, `use` tree expansion, `raw-process` & `ui-raw-io` hygiene, `[patch]`/`[replace]` checks | [`FA-SEC22-11`](#fa-sec22-11), [`FA-SEC22-18`](#fa-sec22-18), [`FA-SEC22-19`](#fa-sec22-19) |
| `xtask/src/licenses.rs` | 1,177 | `1..=1177` | `deny.toml`/`about.toml` parity, SPDX `AND`/`OR`/`WITH` recursive-descent evaluation, `cargo metadata --locked --offline` resolution vs `--all-features`, wildcard dep detection, `Cargo.lock` sources, HTML generation | [`FA-SEC22-14`](#fa-sec22-14), [`FA-SEC22-18`](#fa-sec22-18) |
| `xtask/src/pending.rs` | 126 | `1..=126` | Fenced code-block skipping, `### R<m>.<n>` / `##` scoping, `- [ ]` task collection and distinct item counting | [`FA-SEC22-17`](#fa-sec22-17) |
| `xtask/src/secrets.rs` | 721 | `1..=721` | API token, PEM header, inline env/Rust secret assignment, and home-path detectors; placeholder filtering; UTF-16/NUL handling; `--history` `git log -p` diff state machine | [`FA-SEC22-09`](#fa-sec22-09) |
| `xtask/src/telemetry.rs` | 2,219 | `1..=2219` | Checks 1–5: manifest/`Cargo.lock` banned crates & `web-sys` features, non-test Rust socket/WASM primitives & loopback guards, web/workflow telemetry & URLs, workflow permissions/SHA/script injection, CSP/web headers | [`FA-SEC22-03`](#fa-sec22-03), [`FA-SEC22-04`](#fa-sec22-04), [`FA-SEC22-05`](#fa-sec22-05), [`FA-SEC22-06`](#fa-sec22-06), [`FA-SEC22-18`](#fa-sec22-18) |
| `xtask/src/text_decode.rs` | 70 | `1..=70` | BOM-less UTF-16 LE/BE NUL-pair ratio & dominance heuristics, `decode_utf16` unpaired surrogate & odd tail byte replacement (`U+FFFD`) without panics | Clean — safe UTF-16 decoding and non-panicking `as_chunks::<2>()` handling |
| `xtask/src/version.rs` | 116 | `1..=116` | `[workspace.package] version` parsing/replacement, `semver` validation (rejecting build metadata), lockfile refresh, atomic manifest write discipline | [`FA-SEC22-17`](#fa-sec22-17) |
| `xtask/src/workflows.rs` | 321 | `1..=321` | `parse_ci_env_value` truthiness, `.github/workflows` discovery, offline YAML tab/`${{`/`on:`/`jobs:` checks, shell/hook discovery, `actionlint`/`shellcheck` gating | [`FA-SEC22-05`](#fa-sec22-05), [`FA-SEC22-15`](#fa-sec22-15) |

---

## Findings

### `FA-SEC22-01` · `S2` · `ci-cache trim` misparses Cargo 1.77+ `package_id`s and prunes active `xtask`, `curvebook`, and `curvebook-cli` units on every CI run

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/ci_cache.rs:409-413`, `xtask/src/ci_cache.rs:454-471`, `xtask/src/ci_cache.rs:495-503` (caller `.github/workflows/ci.yml:46-60, 125`)
- **Category:** `wrong-result` / `resources`
- **Defect:** `extract_pkg_name_from_id` assumes every Cargo `package_id` with `#` has the form `...#<name>@<version>`, so for Cargo 1.77+ workspace packages whose directory basename equals their package name (`.../<name>#<version>`), it returns the version string (e.g. `"0.1.0-alpha.2"`) as `pkg_hint`, causing `find_matching_top_fingerprint` to skip all `.fingerprint/` directories; additionally, `dep_artifact_for_top` does not normalize `-` to `_` for hyphenated binary names in `deps/`.
- **Failure scenario:** In `cargo metadata` / `cargo build --message-format=json` on this repository, all top-level binary crates (`xtask`, `curvebook`, `curvebook-cli`) emit `package_id` values without `@` (e.g. `path+file:///.../xtask#0.1.0-alpha.2` and `path+file:///.../apps/curvebook-cli#0.1.0-alpha.2`). During `ci.yml`'s `build` job, `ci-cache trim` sets `pkg_hint = Some("0.1.0-alpha.2")`, rejects every `<pkg>-<16hex>` directory in `target/debug/.fingerprint/`, adds zero fingerprint dirs or unit hashes for `xtask`, `curvebook`, or `curvebook-cli` to `ActiveUnits`, and deletes their `.fingerprint/` entries and `deps/` binaries before saving the CI cache. Every downstream `checks` job then rebuilds `xtask` (and `lint`/`doctests` rebuild `curvebook`/`curvebook-cli`). Even with a fixed `pkg_hint`, `dep_artifact_for_top` checks `deps/curvebook-cli-<hash>` rather than `deps/curvebook_cli-<hash>`, missing the exact inode match for hyphenated binaries.
- **Evidence:**
  ```rust
  // xtask/src/ci_cache.rs:409-413, 469, 495-503
  if let Some(pkg) = pkg_hint
      && !fp_name.starts_with(&format!("{pkg}-"))
  {
      continue;
  }
  ...
  let p = deps_dir.join(format!("{stem}-{hash}"));
  ...
  fn extract_pkg_name_from_id(pkg_id: &str) -> Option<String> {
      if let Some((_, tail)) = pkg_id.rsplit_once('#') {
          let name_part = tail.split('@').next().unwrap_or(tail);
          if !name_part.is_empty() {
              return Some(name_part.to_owned());
          }
      }
      pkg_id.split_whitespace().next().filter(|s| !s.is_empty()).map(str::to_owned)
  }
  ```
  Observed `cargo metadata --manifest-path Cargo.toml --format-version 1 --no-deps --offline` IDs:
  `curvebook -> path+file:///.../apps/curvebook#0.1.0-alpha.2`
  `curvebook-cli -> path+file:///.../apps/curvebook-cli#0.1.0-alpha.2`
  `xtask -> path+file:///.../xtask#0.1.0-alpha.2`
- **Suggested fix:** In `extract_pkg_name_from_id`, extract `name_part` before `@` only when `tail.contains('@')`, and otherwise extract the final path segment before `#` (e.g. `xtask` from `...xtask#0.1.0-alpha.2`). In `dep_artifact_for_top`, check both `stem` and `stem.replace('-', "_")` (and normalize `.pdb` names in `record_top_level_file`), and add a unit test with `@`-less Cargo `package_id`s.
- **Related:** `FA-SEC22-16`

---

### `FA-SEC22-02` · `S2` · `brands::decode_text_file` checks UTF-8 before BOM-less UTF-16, making BOM-less UTF-16 detection unreachable for ASCII text

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/brands.rs:334-339`, `xtask/src/text_decode.rs:7-29`
- **Category:** `security` / `wrong-result`
- **Defect:** `decode_text_file` attempts `std::str::from_utf8(bytes)` before `detect_bomless_utf16(bytes)`, and `detect_bomless_utf16` also requires at least 25% of all 2-byte pairs to be NUL-paired ASCII.
- **Failure scenario:** Any BOM-less UTF-16LE or UTF-16BE file containing ASCII text (e.g. Windows `.rc`, `.wxl`, `.txt`, `.xml`, `.json`, `.html`, `.desktop`, or `.svg`) consists exclusively of bytes in `0x00..=0x7F`, which is valid 1-byte UTF-8. `std::str::from_utf8(bytes)` therefore succeeds at `brands.rs:334` and returns a NUL-interleaved `String`, skipping `detect_bomless_utf16` completely. Because `\0` breaks word runs, snake/kebab compound matching, and literal substring matching, every competitor brand, suite name, and pre-rename name in a BOM-less UTF-16 ASCII file passes `cargo xtask brands` undetected. (Additionally, a BOM-less UTF-16 file dominated by non-ASCII script characters such as CJK with an embedded ASCII brand fails the `le_ascii * 4 >= total_pairs` ratio check in `text_decode.rs:22-24`.)
- **Evidence:**
  ```rust
  // xtask/src/brands.rs:334-339
  if let Ok(s) = std::str::from_utf8(bytes) {
      return Ok(DecodedFile::Text(s.to_owned()));
  }
  if let Some(little_endian) = detect_bomless_utf16(bytes) {
      return Ok(DecodedFile::Text(decode_utf16(bytes, little_endian)));
  }
  ```
- **Suggested fix:** Run `detect_bomless_utf16(bytes)` before `std::str::from_utf8(bytes)` (matching `cleanroom.rs:264-270`), reject unexpected NUL bytes in plain-text files unless decoded as UTF-16, and test `brands::scan_repo_checked` with `include_bom = false` UTF-16LE/BE fixtures.
- **Related:** `FA-SEC22-07`, `FA-SEC22-09`

---

### `FA-SEC22-03` · `S2` · `telemetry` Check 1 misses banned `web-sys` network features and aliased crates in multi-line `Cargo.toml` declarations

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/telemetry.rs:156-215`, `xtask/src/telemetry.rs:315-337`
- **Category:** `security` / `wrong-result`
- **Defect:** `scan_manifest_for_banned_crates` evaluates each line independently via `line.split_once('=')` and requires inline `{ ... }` tables to begin on the same line, so multi-line `web-sys` `features = [...]` arrays and multi-line inline dependency tables bypass Check 1.
- **Failure scenario:** Adding a multi-line `web-sys` dependency such as `[dependencies.web-sys]\nversion = "0.3"\nfeatures = [\n  "Window",\n  "WebSocket",\n]` (or `web-sys = {\n  features = ["WebSocket"],\n}`) causes `features = [` to have `rhs == "["` and `"WebSocket",` to fail `split_once('=')`. Because `Cargo.lock` never records crate feature flags, banned browser networking features (`WebSocket`, `XmlHttpRequest`, `Request`, `Navigator`, `Beacon`, `ServiceWorker`) pass `cargo xtask telemetry` cleanly. Multi-line `client = {\n  package = "reqwest",\n}` similarly evades `extract_inline_package_name` (though `Cargo.lock` still catches `reqwest` after lockfile update).
- **Evidence:**
  ```rust
  // xtask/src/telemetry.rs:180-185, 203-213, 316
  let Some((lhs, rhs)) = line.split_once('=') else {
      continue;
  };
  let key = unquote_toml_str(lhs.trim());
  let rhs = rhs.trim();
  if key != "package" && is_banned_crate_name(&key) { ... }
  ...
  let is_web_sys_entry = key == "web-sys" || section_dependency_name(&section) == Some("web-sys") && key == "features";
  if is_web_sys_entry {
      for feat in extract_banned_web_sys_features(rhs) { ... }
  }
  ...
  fn extract_inline_package_name(rhs: &str) -> Option<String> {
      if !rhs.starts_with('{') { return None; }
  ```
- **Suggested fix:** Reuse a state-tracking multi-line TOML scanner (like `TomlScanner` in `brands.rs`) across bracket/brace depth inside dependency sections so multi-line `features = [...]` arrays and inline tables on `web-sys` / `package = ...` are fully checked.
- **Related:** `FA-SEC22-04`, `FA-SEC22-14`

---

### `FA-SEC22-04` · `S2` · `telemetry` Check 2 Rust network scanner bypassed via braced/aliased `web_sys` or `std::net` imports, non-exact JS strings, and unchecked allowlist targets

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/telemetry.rs:18-24`, `xtask/src/telemetry.rs:360-557`
- **Category:** `security` / `wrong-result`
- **Defect:** Check 2 matches only literal `"web_sys::<Type>"` strings and exact single-word JS strings in non-allowlisted files, while in allowlisted loopback files it accepts multi-line `use std::net::{... as ...}` aliases, arbitrary hostname/IPv6 literals, and uncoupled `LOCALHOST`/`is_loopback` mentions anywhere in the enclosing function.
- **Failure scenario:**
  1. In any crate or `build.rs`, `use web_sys::{WebSocket, XmlHttpRequest};`, `use web_sys as ws; ws::WebSocket::new(...)`, `web_sys::window().fetch_with_str("https://...")`, or `js_sys::eval("fetch('https://...')")` contains none of `ALWAYS_BANNED_NET_PATTERNS` (`web_sys::WebSocket`, etc.) and fails exact equality (`lit == "fetch"`) in `check_js_sys_network_strings`.
  2. Inside the three allowlisted files (`control_io.rs`, `tools.rs`, `apps/curvebook/src/main.rs`), `use std::net::{\n  TcpStream as S\n};` evades `check_net_alias_imports` (`compact.contains("std::net") && line.contains(" as ")` is per-line) and `fn_opens_socket`. Even without aliasing, `TcpStream::connect("telemetry.example:443")` or `"[2001:db8::1]:443"` alongside an unused `Ipv4Addr::LOCALHOST` passes `slice_has_loopback_guard` and `find_non_loopback_ip_literal` (which only checks dotted IPv4 quads, `0.0.0.0`, and `[::]`). Furthermore, a const-generic `{}` in a function signature (`fn f(x: [u8; { 1 }]) { ... }`) prematurely terminates `collect_rust_fn_spans` before the function body.
- **Evidence:**
  ```rust
  // xtask/src/telemetry.rs:388-390, 450-453, 515-552
  let lit = line[pos + 1..pos + 1 + end_rel].trim().to_ascii_lowercase();
  if BANNED_JS_LITERALS.contains(&lit.as_str()) { ... }
  ...
  if compact.contains("std::net::*") || compact.contains("std::netas") || (compact.contains("std::net") && line.contains(" as ")) { ... }
  ...
  if prod_code.contains("is_loopback") || ... || prod_code.contains("LOCALHOST") { return true; }
  ...
  if is_ipv4_dotted_quad(host) && !host.starts_with("127.") { ... }
  ```
- **Suggested fix:** Expand `use` statements (via `layers.rs`'s multi-line `collect_use_statements`/`expand_use_tree`) to ban `web_sys`/`std::net`/`core::net` aliases and grouped imports of banned types, flag `fetch_with_str`/`fetch_with_request` and substring network API calls inside `js_sys` string literals, reject all string-literal socket targets not explicitly matching `127.0.0.1` / `localhost` / `[::1]` in allowlisted files, and track parenthesis/angle-bracket nesting before matching a `fn` body's `{`.
- **Related:** `FA-SEC22-03`, `FA-SEC22-11`

---

### `FA-SEC22-05` · `S2` · `telemetry` Check 4 workflow security gate skips `.github/actions/**` and can be bypassed by quoted `permissions:`, multi-line scalars, and line-continued pipes

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/telemetry.rs:851-876`, `xtask/src/telemetry.rs:923-955`, `xtask/src/telemetry.rs:1030-1159`, `xtask/src/telemetry.rs:1402-1420` (and `xtask/src/workflows.rs:96-117`)
- **Category:** `security` / `wrong-result`
- **Defect:** `collect_workflow_files` inspects only `.github/workflows/*.{yml,yaml}`, and `audit_workflow_yaml`'s line-by-line parser misses quoted `permissions` keys, multi-line plain/quoted `run:` scalars, multi-line `${{ ... }}` expressions, and `curl`/`wget` pipes split across lines.
- **Failure scenario:**
  - Unpinned `uses:` tags, `curl | bash` downloads, or telemetry commands placed in composite actions under `.github/actions/**/action.yml` (such as `.github/actions/ci-setup/action.yml`) are never checked by `telemetry` Check 3/4 or `workflows`.
  - Inside `.github/workflows/*.yml`, job-level `'permissions': { contents: write }` escapes `trimmed.strip_prefix("permissions:")`; a `run:` value written as a quoted multi-line string (`run: "\n  curl ... | bash\n"`) or unblocked multi-line scalar without `|`/`>` fails `is_yaml_block_scalar`, skipping both `flag_untrusted_event_expr` and `flag_remote_script_execution`; and `curl -fsSL https://example.com/x.sh \\\n  | bash` or `${{\n  github.event.issue.title\n}}` inside `run: |` bypasses single-line checks when `telemetry` runs standalone.
- **Evidence:**
  ```rust
  // xtask/src/telemetry.rs:923, 1035-1049, 1084-1098, 1146-1151, 1403
  if let Some(rest) = trimmed.strip_prefix("permissions:") { ... }
  ...
  if is_yaml_block_scalar(trimmed_rhs) {
      state.run_block_col = Some(key_col);
  } ... // plain/quoted multi-line `run:` scalars skip block mode
  ...
  if has_fetcher
      && let Some((_, pipe_rhs)) = lower.split_once('|') ...
  ```
- **Suggested fix:** Include `.github/actions/**/*.{yml,yaml}` in `collect_workflow_files` (applying action `uses:`/`run:`/telemetry checks without requiring workflow `on:`/`jobs:`/`permissions:` headers on composite actions), handle quoted YAML keys, join `\` line continuations and multi-line `${{ ... }}` spans across `run:` blocks, and track indented `run:` plain/quoted scalar blocks.
- **Related:** `FA-SEC22-06`, `FA-SEC22-15`

---

### `FA-SEC22-06` · `S2` · `telemetry` Checks 3 and 5 miss UTF-16/SVG/JSON web files, bracketed `fetch`/spaced `Worker` calls, scheme-source CSP directives, and per-path `_headers` overrides

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/telemetry.rs:626-628`, `xtask/src/telemetry.rs:653-816`, `xtask/src/telemetry.rs:1200-1217`, `xtask/src/telemetry.rs:1388-1399`
- **Category:** `security` / `wrong-result`
- **Defect:** Check 3 skips NUL-containing files, skips `.svg`/`.json`/`.jsx`/`.tsx`/`.cjs` web assets in `is_web_asset_file`, and relies on rigid JS/URL syntax matching; Check 5 permits CSP scheme sources without `://` and ignores Netlify `_headers` blocks outside `/*`.
- **Failure scenario:**
  - Under `apps/curvebook-web/` or `packaging/web/`, an SVG with `<script href="https://...">` or a web manifest `.json` with an external URL is skipped by `is_web_asset_file` (`653-662`, making `"jsx" | "tsx"` at line 667 dead), a UTF-16 file is skipped by `if bytes.contains(&0) { continue; }`, and `window['fetch']('https://...')`, `new Worker ('w.js')` (space before `(`), `new  Worker('w.js')`, or HTML-entity URLs (`&#104;ttps://...`) pass Check 3.
  - In `packaging/web/_headers` or `.htaccess`, adding `script-src https:; img-src https:; frame-src https:` alongside the required exact directives passes `validate_csp_directives` because `tok.contains("://")` does not flag scheme sources (`https:`, `http:`, `wss:`), and a Netlify `/index.html` path block after `/*` relaxing `Content-Security-Policy` or `Referrer-Policy` is ignored by `parse_netlify_global_headers`.
- **Evidence:**
  ```rust
  // xtask/src/telemetry.rs:626-628, 653-662, 1208-1210, 1388-1391
  if bytes.contains(&0) { continue; }
  ...
  fn is_web_asset_file(rel: &str) -> bool {
      ...
      matches!(..., Some("html" | "htm" | "js" | "mjs" | "ts" | "css"))
  }
  ...
  if !raw.starts_with(char::is_whitespace) {
      in_global = trimmed == "/*";
      continue;
  }
  ...
  if tok.contains("://") || tok.starts_with("//") || tok == "*" { ... }
  ```
- **Suggested fix:** Decode UTF-16 web/workflow files, include `.svg`/`.json`/`.jsx`/`.tsx`/`.cjs` in `is_web_asset_file`, decode HTML entities and normalize whitespace before JS/URL scanning, reject `https:`/`http:`/`wss:`/`ws:` scheme sources and `'unsafe-*'`/`data:`/`blob:` additions in `validate_csp_directives`, and reject weaker per-path blocks in `_headers`/`.htaccess`.
- **Related:** `FA-SEC22-05`

---

### `FA-SEC22-07` · `S2` · `brands` and `assets` skip several tracked text extensions, `.pdf`/ZIP repo files, `.svgz` binaries, and mixed-case asset prefixes

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/brands.rs:270-288`, `xtask/src/brands.rs:356-389`, `xtask/src/assets.rs:161-171`, `xtask/src/assets.rs:444-447`, `xtask/src/assets.rs:512-539`
- **Category:** `security` / `wrong-result`
- **Defect:** `brands::scan_file` applies competitor-brand checks only to a fixed extension list and case-sensitive suffixes, `brands::scan_repo_checked` skips `.pdf` and ZIP containers (`BINARY_ASSET_EXT`) completely, and `assets` skips `.svgz`/`.pdf` metadata, ZIP-embedded fonts in repo files, and case-folded asset directory prefixes.
- **Failure scenario:**
  - Competitor trademarks placed in tracked `.wxi` (`packaging/windows/install-policy.wxi`), `.hbs` (`about.hbs`), `.py`, `.R`, extensionless scripts/files (`.githooks/pre-commit`, `.github/CODEOWNERS`, `LICENSE-*`), or uppercase `.PLIST`/`.DESKTOP` files only hit `scan_suite_lines` and completely bypass `BRANDS` checks in `cargo xtask brands`.
  - A tracked `.pdf`, `.zip`, `.curvebook`, or `.cbtemplate` in `examples/` or `docs/` returns `DecodedFile::BinaryAsset` in `brands::scan_repo_checked` (skipping `check_pdf_metadata` and archive entry checks that `scan_dist_bytes` performs), while `assets::verify_asset_binaries` skips `.svgz` and `.pdf` metadata, `assets::verify_assets` skips font detection inside tracked ZIPs, and `Assets/ui/unkeyworded.png` bypasses `is_original_only_visual_asset`'s case-sensitive `path.starts_with(prefix)` check on case-insensitive filesystems.
- **Evidence:**
  ```rust
  // xtask/src/brands.rs:367-380
  } else if path.ends_with(".plist") || path.ends_with(".plist.in") {
      scan_plist(path, text, out);
  } else if is_mime_package_xml(path) {
      scan_mime_xml(path, text, out);
  } else if path.ends_with(".desktop") {
      scan_desktop_entry(path, text, out);
  ...
  } else if path == "docs/development.md" || is_scanned_non_rs_text_file(path, &ext) {
      scan_plain_lines_no_waiver(path, text, out);
  }
  ```
- **Suggested fix:** Use lowercase path suffixes for `.plist`/`.plist.in`/`.desktop`, default any valid decoded UTF-8/UTF-16 tracked file without a dedicated parser to `scan_plain_lines_no_waiver`, run `check_pdf_metadata` and bounded `ZipArchive` inspection in `scan_repo_checked`, decompress/scan `.svgz` and `.pdf` in `assets::verify_asset_binaries`, and normalize asset paths to lowercase ASCII before prefix checks.
- **Related:** `FA-SEC22-02`, `FA-SEC22-08`, `FA-SEC22-12`

---

### `FA-SEC22-08` · `S2` · `brands` misses `\xHH`/`\u{...}` escapes in Rust literals, SVG XML comments/entities, and non-PNG `data:image/` URIs

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/brands.rs:726-754`, `xtask/src/brands.rs:810-860`, `xtask/src/brands.rs:1538-1570`, `xtask/src/brands.rs:2540-2565`
- **Category:** `security` / `wrong-result`
- **Defect:** `RustLexer::read_string_escape` does not decode `\xHH` or `\u{...}` escapes; both `SvgState` and `check_svg_metadata` discard `<!-- ... -->` comments; `scan_svg` does not decode XML entities while `check_svg_metadata` ignores tag attributes and `<textPath>`; and `check_and_strip_svg_data_uri` strips all non-exact-`"png"` `data:image/` payloads without inspecting them.
- **Failure scenario:**
  - A Rust literal written with byte/Unicode escapes (`"\x41\x42..."`, `"\u{50}\u{72}..."`) decodes in `RustLexer` with the literal `'x'`/`'u'` prefix intact (`"x41..."`, `"u{50}..."`), bypassing `BRANDS`, `SUITE_BRANDS`, and `ADAPTED_UPSTREAM_BRAND` checks.
  - Inside an `.svg` file (in repo or `dist/`), `<!-- [COMPETITOR] / [PRE_RENAME] -->` is ignored by both `scan_svg` and `check_svg_metadata`; `<g id="&#71;...">` or `<textPath>&#80;...</textPath>` bypasses both (one skips entities, the other ignores attributes and `<textPath>`); and `<image href="data:image/gif;base64,..."/>`, `data:image/svg+xml;base64,...`, or `data:image/png;charset=utf-8;base64,...` is replaced with `"data:image/stripped"` without checking embedded GIF comments, nested SVGs, or PNG chunks.
- **Evidence:**
  ```rust
  // xtask/src/brands.rs:850-856, 1559-1568, 2540-2544
  if subtype.eq_ignore_ascii_case("png")
      && let Some(decoded) = decode_base64_ascii(b64_payload)
  {
      out.extend(check_png_metadata(path, &decoded));
  }
  out_str.push_str("data:image/stripped");
  ...
  // RustLexer::read_string_escape
  other => char::from(other),
  ...
  // check_svg_metadata
  if rest.starts_with("<!--") {
      let end = rest.find("-->").map_or(svg.len(), |rel| pos + rel + 3);
      line += count_newlines(&svg[pos..end]);
      pos = end;
      continue;
  }
  ```
- **Suggested fix:** Decode Rust `\xHH` and `\u{...}` escapes in `RustLexer::read_string_escape` (and run `SUITE_BRANDS`/`ADAPTED_UPSTREAM_BRAND` on decoded literals too), scan SVG XML comments and entity-decoded attributes/data URIs (`png` with `;params`, `gif`, `svg+xml`), and include `<textPath>` in `check_svg_metadata`.
- **Related:** `FA-SEC22-07`, `FA-SEC22-12`

---

### `FA-SEC22-09` · `S2` · `secrets` silently skips all UTF-16/NUL-byte files and misparses added `+++ ` diff lines in `--history`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/secrets.rs:124-126`, `xtask/src/secrets.rs:156-158`, `xtask/src/secrets.rs:195-214`, `xtask/src/secrets.rs:449-498`
- **Category:** `security` / `wrong-result`
- **Defect:** `scan_repo` skips every file containing any `0x00` byte, `run_history` skips `0x00`-containing diff lines without advancing `added_line_no`, and `scan_git_diff_line` matches `+++ ` before `+`, misinterpreting added lines starting with `++ ` as diff file headers.
- **Failure scenario:** Any UTF-16LE/BE file (`.rc`, `.wxl`, `.ps1`, `.txt`) or any text/config file containing a single embedded `\0` byte carrying an API token, PEM private key, `API_KEY=...`, or `/Users/<name>/` path is skipped silently by `cargo xtask secrets`. In `cargo xtask secrets --history`, an added line beginning with `++ ` (emitted by `git log -p` as `+++ ...`) triggers `raw_line.starts_with("+++ ")`, skipping secret detection on that line and overwriting `state.current_file`. Additionally, `detect_inline_secret_assignment` skips `#`/`//` comment lines (`# PASSWORD=...`) and `b"..."`/`c"..."` Rust literals.
- **Evidence:**
  ```rust
  // xtask/src/secrets.rs:156-158, 195-209
  if bytes.contains(&0) {
      continue;
  }
  ...
  if raw_line.starts_with("+++ ") {
      if let Some(path) = parse_diff_plus_path(raw_line) {
          state.current_file = path;
      }
      return;
  }
  ...
  if let Some(added) = raw_line.strip_prefix('+') {
  ```
- **Suggested fix:** Decode UTF-16 files via `text_decode` in `secrets::scan_repo` before treating binary blobs with NUL bytes as non-text (or scan ASCII/UTF-8 runs inside non-binary-header files), and in `scan_git_diff_line` track hunk line counts (`@@ -a,b +c,d @@`) so `+` content lines inside a hunk cannot be mistaken for `+++ ` file headers.
- **Related:** `FA-SEC22-02`

---

### `FA-SEC22-10` · `S2` · `ci quick` skips affected crates when shared embedded assets/configs/workflows change, and `cargo_metadata` runs without `--locked`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/ci_quick.rs:24-26`, `xtask/src/ci_quick.rs:178-210`, `xtask/src/main.rs:208-222` (and `xtask/src/ci.rs:168-170`)
- **Category:** `wrong-result`
- **Defect:** `ci_quick::select` only treats `Cargo.toml`, `Cargo.lock`, toolchain, clippy config, and `.cargo/` as workspace-wide inputs, ignoring shared non-crate files embedded at compile time or tested by workspace crates; meanwhile `main.rs::run_cargo_metadata` invokes `cargo metadata` without `--locked`.
- **Failure scenario:**
  1. Editing `assets/app-icon/hicolor/128x128/apps/org.curvebook.CurveBook.png` (embedded via `include_bytes!` in `crates/ui-egui/src/egui_shell/brand.rs:9`), `assets/app-icon/curvebook-macos-512.png` (embedded in `apps/curvebook/src/gui_window.rs:1326`), `docs/control-protocol.md` (read by `crates/ui-egui/tests/audit_r1_s4c_s10_09.rs:107`), `.github/workflows/ci.yml` / `.github/actions/ci-setup/action.yml` (embedded in `xtask/src/ci.rs:214-215`), `packaging/windows/{AppxManifest.xml.in,store-identity.json}` (embedded in `xtask/src/msix.rs:11-12`), `deny.toml` (embedded in `xtask/src/licenses.rs:1029`), or `.config/nextest.toml` yields `Selection::Packages(empty)` in `ci_quick::select`, so `cargo xtask ci quick` skips Clippy and tests for all affected crates.
  2. When a crate `Cargo.toml` is modified without updating `Cargo.lock`, running `cargo xtask layers`, `cargo xtask ci policy` (where `cmd_layers` runs before `licenses`), or `cargo xtask ci quick` invokes `cargo metadata --format-version 1` without `--locked`, silently mutating `Cargo.lock` instead of failing closed.
- **Evidence:**
  ```rust
  // xtask/src/ci_quick.rs:24-26, 178-194
  const WORKSPACE_WIDE_FILES: &[&str] = &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain", "clippy.toml", ".clippy.toml"];
  const WORKSPACE_WIDE_DIRS: &[&str] = &[".cargo/"];
  ...
  let mut selected: BTreeSet<String> = changed.iter().filter_map(|path| owning_package(path, dirs)).map(str::to_owned).collect();

  // xtask/src/main.rs:216-222
  pub(crate) fn cargo_metadata_resolved(root: &Path) -> Result<serde_json::Value, String> {
      run_cargo_metadata(root, &["metadata", "--format-version", "1"])
  }
  ```
- **Suggested fix:** Add `.config/nextest.toml` to `WORKSPACE_WIDE_FILES`, map shared non-crate inputs (`assets/app-icon/`, `docs/control-protocol.md`, `.github/`, `packaging/`, `deny.toml`, `ASSETS.md`, `ATTRIBUTION.md`, `NOTICE`) to the packages that embed or test them (`curvebook-ui-egui`, `curvebook`, `xtask`), and pass `--locked` (when `Cargo.lock` exists) to `run_cargo_metadata`.
- **Related:** `FA-SEC22-01`

---

### `FA-SEC22-11` · `S2` · `layers` source hygiene (`raw-process` and `ui-raw-io`) bypassed by module namespace aliases, multi-line `::` paths, and unbanned `std::fs` APIs

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/layers.rs:452-539`, `xtask/src/layers.rs:580-618`, `xtask/src/layers.rs:941-1064`
- **Category:** `security` / `wrong-result`
- **Defect:** `check_source_hygiene` checks expanded `use` paths only against leaf-level items (`std::process::Command`, `std::fs::write`, etc.), performs single-line literal pattern matching without normalizing whitespace around `::`, and omits several destructive `std::fs` functions from `UI_IO_PATTERNS`.
- **Failure scenario:**
  - In any non-exempt production crate, `use std::process as proc; proc::Command::new("sh")` expands via `expand_use_tree`/`strip_use_alias` to `"std::process"` (which is not in the banned list), and `proc::Command::new` matches none of `RAW_PROCESS_PATTERNS`. Multi-line `std::process::\n  Command::new("sh")` likewise fails single-line `scan_patterns`.
  - In `crates/ui-egui/src/`, `use std::fs as disk; disk::write(...)` or `disk::File::create(...)` bypasses `ui-raw-io` identically, and direct calls to `std::fs::create_dir_all`, `std::fs::remove_dir_all`, `std::fs::set_permissions`, `std::fs::hard_link`, or `std::os::unix::fs::symlink` are not in `UI_IO_PATTERNS` or `scan_use_statements`.
- **Evidence:**
  ```rust
  // xtask/src/layers.rs:510, 581-591, 1012-1020
  scan_use_statements(&rel, &prod, "raw-process", &["std::process::Command", "process::Command", "std::process::*"], &mut violations);
  ...
  for (idx, line) in prod_src.lines().enumerate() {
      if let Some((hit_pos, pat)) = next_pattern_match(line, pos, patterns) { ... }
  }
  ...
  let without_alias = strip_use_alias(trimmed); // `use std::process as proc;` -> `"std::process"`
  ```
- **Suggested fix:** Ban parent namespace imports (`std::process`, `process`, `std::fs`, `fs`, `std::os::unix::fs`) in `scan_use_statements` (requiring `curvebook_platform` exclusively), collapse whitespace around `::` before running `scan_patterns`, and add `create_dir*`, `remove_dir*`, `set_permissions`, and `hard_link`/`symlink` to `UI_IO_PATTERNS`.
- **Related:** `FA-SEC22-04`, `FA-SEC22-19`

---

### `FA-SEC22-12` · `S2` · Distribution and archive scanning in `brands`, `cleanroom`, and `assets` fails open on nested depth >= 2, malformed ZIPs, non-UTF-8 dist files, and nested reference dirs

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/brands.rs:4511`, `xtask/src/brands.rs:4540-4560`, `xtask/src/cleanroom.rs:155`, `xtask/src/cleanroom.rs:219-242`, `xtask/src/assets.rs:722`
- **Category:** `security` / `wrong-result`
- **Defect:** Release-container scanners silently stop recursing into non-vendor ZIP archives at `depth >= 2`, silently skip malformed/over-limit non-vendor ZIPs in `cleanroom`, silently ignore non-UTF-8 text files and adapted-upstream identifiers in `brands::scan_dist_bytes`, and miss nested reference directories in `cleanroom::scan_dist_artefacts_with_stats`.
- **Failure scenario:**
  - Placing a forbidden asset, font, vendor copyright notice, or trademark inside a 3-deep archive (`pkg.zip!inner.zip!nested.zip`) hits `depth >= 2` (`cleanroom.rs:220`, `brands.rs:4511`, `assets.rs:722`) and silently returns 0 violations; a non-vendor `.zip`/`.curvebook` exceeding `CLEANROOM_ZIP_LIMITS` or with corrupt central directory falls back in `cleanroom.rs:237` to scanning compressed bytes as text and exits clean.
  - Inside `dist/` or `packaging/web/`, a Latin-1 `.html`/`.js`/`.desktop`/`.plist` hits `let Ok(DecodedFile::Text(text)) = decode_text_file(...) else { return Ok(()); };` (`brands.rs:4540`) and is skipped (unlike `scan_repo_checked` which fails closed); non-HTML/JSON dist text files call `scan_suite_lines_with_scope` which skips `ADAPTED_UPSTREAM_BRAND` because `is_crates_or_apps_path` is false; and `cleanroom.rs:155` checks `starts_with` rather than `contains("/.../...")`, missing a nested reference directory inside a `.app` bundle.
- **Evidence:**
  ```rust
  // xtask/src/cleanroom.rs:220-241
  if depth >= MAX_ARCHIVE_DEPTH {
      if is_vendor_archive { hits.push(...); }
      return;
  }
  if let Ok(archive) = ZipArchive::from_bytes_with_limits(bytes, CLEANROOM_ZIP_LIMITS) { ... return; }
  ...
  if is_vendor_archive && hits.len() == before { hits.push(...); }

  // xtask/src/brands.rs:4540-4542
  let Ok(DecodedFile::Text(text)) = decode_text_file(display_path, bytes) else {
      return Ok(());
  };
  ```
- **Suggested fix:** Fail closed (emit a violation) whenever any ZIP archive exceeds `MAX_ARCHIVE_DEPTH` or fails `ZipArchive::from_bytes_with_limits`, return an error on undecodable non-binary text extensions in `brands::scan_dist_bytes`, enforce `ADAPTED_UPSTREAM_BRAND` on all non-attribution dist text files, and check `contains` for nested forbidden reference paths in `cleanroom::scan_dist_artefacts_with_stats`.
- **Related:** `FA-SEC22-07`, `FA-SEC22-08`

---

### `FA-SEC22-13` · `S2` · `assets::parse_table_cells` panics on a bare `"|"` line in attribution markdown

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/assets.rs:188-195` (reachable from `assets.rs:449-450, 756-766`)
- **Category:** `no-panic`
- **Defect:** `parse_table_cells` checks `trimmed.starts_with('|') && trimmed.ends_with('|')` and unconditionally slices `&trimmed[1..trimmed.len() - 1]` without checking `trimmed.len() >= 2`.
- **Failure scenario:** If `ASSETS.md`, `ATTRIBUTION.md`, or a bundled `ASSETS.md`/`ATTRIBUTION.md` inside a scanned release archive (`scan_dist_artefacts`) contains a line whose trimmed content is a single `"|"`, `trimmed.len()` is `1` and `&trimmed[1..0]` panics at runtime, crashing `cargo xtask assets` and `cargo xtask release-scan` instead of returning a structured parse error.
- **Evidence:**
  ```rust
  // xtask/src/assets.rs:188-195
  fn parse_table_cells(line: &str) -> Option<Vec<&str>> {
      let trimmed = line.trim();
      if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
          return None;
      }
      let inner = &trimmed[1..trimmed.len() - 1];
      Some(inner.split('|').map(str::trim).collect())
  }
  ```
- **Suggested fix:** Guard `trimmed.len() < 2` (or use `trimmed.strip_prefix('|').and_then(|s| s.strip_suffix('|'))`) in `parse_table_cells` so a lone `"|"` returns `None` or a clear parse error without panicking.
- **Related:** `none`

---

### `FA-SEC22-14` · `S2` · `licenses` omits `--all-features` parity with `deny.toml`, skips target-specific dependency subtables, and swallows `ATTRIBUTION.md`/`NOTICE` read errors

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/licenses.rs:243-253`, `xtask/src/licenses.rs:592-603`, `xtask/src/licenses.rs:750-753` (cross-check `deny.toml:6-7`)
- **Category:** `contract-drift` / `security`
- **Defect:** `run_locked_offline_metadata` invokes `cargo metadata` without `--all-features`, `is_dependency_section` omits `[target.<cfg>.dependencies.<crate>]` subtables for wildcard bans, and `generate_licenses_html` discards `ATTRIBUTION.md`/`NOTICE` read failures with `.unwrap_or_default()` and uses non-atomic `std::fs::write`.
- **Failure scenario:**
  - `deny.toml` sets `[graph] all-features = true` so optional dependencies behind non-default Cargo features are audited; when `cargo-deny` is absent locally, `cargo xtask licenses` omits `--all-features` and passes even if an optional non-default feature enables a copyleft or unlicensed dependency.
  - A wildcard `"*"` version under `[target.'cfg(unix)'.dependencies.bad]` bypasses `is_dependency_section` (which checks `.dependencies` suffix and `dependencies.` prefix, unlike `telemetry.rs:296`).
  - During `cargo xtask licenses --html` without `cargo-about`, unreadable or non-UTF-8 `ATTRIBUTION.md` or `NOTICE` files silently disappear from the generated `THIRD-PARTY-LICENSES.html` via `.unwrap_or_default()`.
- **Evidence:**
  ```rust
  // xtask/src/licenses.rs:246, 592-603, 750-753
  .args(["metadata", "--format-version", "1", "--locked", "--offline"])
  ...
  fn is_dependency_section(section: &str) -> bool {
      section == "dependencies" ... || section.ends_with(".dependencies") ... || section.starts_with("dependencies.") ...
  }
  ...
  let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap_or_default();
  let notice = std::fs::read_to_string(root.join("NOTICE")).unwrap_or_default();
  ...
  std::fs::write(out_path, html)
  ```
- **Suggested fix:** Add `--all-features` to `run_locked_offline_metadata`, add `.dependencies.` / `.dev-dependencies.` / `.build-dependencies.` substring checks to `is_dependency_section`, propagate read/UTF-8 errors when `ATTRIBUTION.md` or `NOTICE` exists, and write the HTML report via `curvebook_platform::fs::write_atomic`.
- **Related:** `FA-SEC22-03`

---

### `FA-SEC22-15` · `S2` · `workflows` passes locally when either `actionlint` or `shellcheck` is missing and swallows `repo_files` errors in `collect_shell_scripts`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/workflows.rs:47-57`, `xtask/src/workflows.rs:72-86`, `xtask/src/workflows.rs:120-165`
- **Category:** `security` / `wrong-result`
- **Defect:** `workflows::run` fails locally only if both `actionlint` and `shellcheck` are missing (passing if either one is absent), `collect_shell_scripts` silently falls back on `repo_files` error to `walk_shell_targets` which skips `.github/`, and `check_workflows_offline_yaml` misses space-prefixed tabs and a second unclosed `${{` on a line.
- **Failure scenario:** A contributor with `shellcheck` installed but without `actionlint` runs `cargo xtask workflows` (or `cargo xtask ci quick`) and sees a green pass on broken GitHub Actions workflows. If `crate::repo_files(root)` fails in a git checkout (e.g., broken symlink), `if root.join(".git").exists() && let Ok(tracked) = crate::repo_files(root)` ignores the error and runs `walk_shell_targets`, which skips `.github/` scripts. Additionally, `"  \tbad: true"` evades `line.starts_with('\t')`, and `"${{ ok }} ${{ unclosed"` passes `line.find("${{")` because `]]` from the first expression satisfies `contains("}}")`.
- **Evidence:**
  ```rust
  // xtask/src/workflows.rs:47-57, 72-86, 122-124, 162
  } else if actionlint_ran {
      println!("workflows: actionlint ({} workflow files) passed", workflows.len());
  } else if shellcheck_ran {
      println!("workflows: shellcheck ({} shell scripts) passed", shell_scripts.len());
  }
  ...
  if root.join(".git").exists()
      && let Ok(tracked) = crate::repo_files(root)
  ```
- **Suggested fix:** Require both `actionlint` and `shellcheck` when not explicitly skipped (or fail when `workflows` is invoked directly and either required linter is missing), propagate `crate::repo_files(root)?` in git repos, include `.github/` in `walk_shell_targets`, and check `line.contains('\t')` plus balanced `${{`..`}}` scanning across each line.
- **Related:** `FA-SEC22-05`

---

### `FA-SEC22-16` · `S3` · Relative `CARGO_TARGET_DIR` diverges from workspace `root` in `cmd_wasm` (`trunk`) and `ci-cache`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/main.rs:327-335`, `xtask/src/ci_cache.rs:178-183`
- **Category:** `wrong-result`
- **Defect:** `run_trunk_web_release_if_available` and `ci_cache::resolve_target_dir` wrap `CARGO_TARGET_DIR` with `PathBuf::from` without anchoring relative paths to `root` (unlike `xtask/src/bundle.rs:86`).
- **Failure scenario:** Setting `CARGO_TARGET_DIR=target-worker` causes `run_trunk_web_release_if_available` to create `cwd/target-worker/wasm-trunk-dist` and invoke `trunk build --dist target-worker/wasm-trunk-dist` with `current_dir = root/apps/curvebook-web`, so `trunk` writes to `apps/curvebook-web/target-worker/wasm-trunk-dist` while `xtask` verifies `cwd/target-worker/wasm-trunk-dist/index.html` and fails. Likewise, running `cargo xtask ci-cache` from a subdirectory with relative `CARGO_TARGET_DIR` makes `cargo` build into `root/<target>` while `trim`/`restore-mtimes` inspect `cwd/<target>`.
- **Evidence:**
  ```rust
  // xtask/src/main.rs:327-335
  let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
  let dist_dir = target_dir.join("wasm-trunk-dist");
  ...
  let status = Command::new("trunk").current_dir(&web_dir).args(["build", "--dist", &dist_arg, ...])

  // xtask/src/ci_cache.rs:178-183
  fn resolve_target_dir(root: &Path, explicit: Option<&Path>) -> PathBuf {
      if let Some(dir) = explicit { return dir.to_path_buf(); }
      std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
  }
  ```
- **Suggested fix:** Resolve `CARGO_TARGET_DIR` through a shared helper (`if p.is_absolute() { p } else { root.join(p) }`, matching `bundle.rs:84-87`).
- **Related:** `FA-SEC22-01`

---

### `FA-SEC22-17` · `S3` · `version` matches `version*` prefix keys and writes `Cargo.toml` non-atomically without lockfile rollback; `pending` drops punctuated `R<m>.<n>` headings

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/version.rs:19-22`, `xtask/src/version.rs:30-85`, `xtask/src/pending.rs:85-94`
- **Category:** `wrong-result` / `data-loss` / `architecture`
- **Defect:** `read_workspace_version` and `replace_workspace_version` match any key in `[workspace.package]` starting with `"version"`, write `Cargo.toml.tmp` without `curvebook_platform::fs::write_atomic` or rollback on `cargo update` failure, and `pending::extract_roadmap_id` rejects `R<m>.<n>` tokens with trailing punctuation or backticks.
- **Failure scenario:** A key such as `version_code = 1` placed before `version = "..."` in `[workspace.package]` (or a `[workspace.package] # comment` header) causes `cargo xtask version` to return/overwrite the wrong field, while a failed `refresh_lockfile` leaves `Cargo.toml` modified and `Cargo.lock` stale (and a failed rename leaves `Cargo.toml.tmp` behind). In `docs/ROADMAP.md`, headings formatted as `### R10.1: Title` or ``### `R10.1` Title`` are classified as `"unscoped"` by `pending`.
- **Evidence:**
  ```rust
  // xtask/src/version.rs:19-22, 39-40, 67
  let tmp = root.join("Cargo.toml.tmp");
  std::fs::write(&tmp, &updated)...;
  std::fs::rename(&tmp, &manifest)...;
  refresh_lockfile(root)?;
  ...
  && let Some(rest) = t.strip_prefix("version")
  && let Some((_, rhs)) = rest.split_once('=')
  ```
- **Suggested fix:** Check exact key equality (`lhs.trim() == "version"`) after `split_once('=')`, strip `#` comments before matching `[workspace.package]`, use `curvebook_platform::fs::write_atomic` (restoring previous `Cargo.toml` bytes if `refresh_lockfile` fails), and trim trailing `:` / `` ` `` delimiters in `pending::extract_roadmap_id`.
- **Related:** `none`

---

### `FA-SEC22-18` · `S3` · Symlink-following directory walks in `telemetry`/`layers`/`licenses` and unbounded `std::fs::read` on `dist/` artifacts

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/telemetry.rs:592, 1429, 1448`, `xtask/src/layers.rs:555`, `xtask/src/licenses.rs:551`, `xtask/src/assets.rs:651`, `xtask/src/brands.rs:4504`, `xtask/src/cleanroom.rs:159`
- **Category:** `resources`
- **Defect:** Directory walkers in `telemetry.rs`, `layers.rs`, and `licenses.rs` call `path.is_dir()` (which follows symlinks) instead of checking `symlink_metadata`, and `dist/` scanners in `assets.rs`, `brands.rs`, and `cleanroom.rs` load entire unpacked files via unbounded `std::fs::read` prior to inspecting `ZipLimits`.
- **Failure scenario:** An untracked or generated symlink inside the workspace pointing to an ancestor directory causes infinite recursion (`ELOOP` / `PATH_MAX` / stack exhaustion) during `telemetry`, `layers`, or `licenses`; reading a multi-gigabyte binary or crafted release artifact in `dist/` via `std::fs::read` risks OOM before `ZipArchive::from_bytes_with_limits` is ever reached.
- **Evidence:**
  ```rust
  // xtask/src/telemetry.rs:1448, xtask/src/layers.rs:555, xtask/src/licenses.rs:551
  if path.is_dir() { ... }

  // xtask/src/assets.rs:651, xtask/src/brands.rs:4504, xtask/src/cleanroom.rs:159
  let bytes = std::fs::read(dist_dir.join(&rel))...
  ```
- **Suggested fix:** Replace `path.is_dir()` with `entry.file_type()` / `symlink_metadata` (matching `workflows.rs:160` and `collect_dist_files`), and read unpacked `dist/` files via `curvebook_platform::fs::read_regular_file_bytes` with an explicit upper bound (or stream archive headers before buffering).
- **Related:** `FA-SEC22-12`

---

### `FA-SEC22-19` · `S4` · `layers::classify` name collisions, `[patch.<reg>.<crate>]` subtables, and unreachable branches in `brands.rs`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/layers.rs:96-99, 356-361, 406-431`, `xtask/src/brands.rs:378, 598`
- **Category:** `cleanup` / `wrong-result`
- **Defect:** `layers::classify` matches `short_name(pkg)` on non-workspace crates.io dependencies too (stopping transitive dependency checks if any external crate is named `cli`, `web`, `curvebook`, `xtask`, or `testkit`); `scan_patch_sections` ignores `[patch.<registry>.<crate>]` subtable headers; and `brands.rs` retains two dead branches (`path == "docs/development.md"` after `ext == "md"`, and `is_asset_doc` matching `.md` files inside `scan_plain_lines_no_waiver`).
- **Failure scenario:** Adding an external crates.io dependency named `web` or `testkit` that itself depends on a UI toolkit crate causes `collect_resolved_transitive_deps` to hit `matches!(classify(pname), Some(Class::Testkit | Class::Exempt))` and silently skip checking its transitive dependencies (or misreport an external crate named `math`/`doc` as a workspace crate). Meanwhile in `brands.rs`, `ASSETS.md` and `ATTRIBUTION.md` are routed to `scan_markdown` instead of reaching `is_asset_doc` inside `scan_plain_lines_no_waiver`.
- **Evidence:**
  ```rust
  // xtask/src/layers.rs:356-363
  let is_ws = member_id_set.contains(pid) || classify(pname).is_some();
  out.insert((pname.to_owned(), root_kind, is_ws));
  if matches!(classify(pname), Some(Class::Testkit | Class::Exempt)) {
      continue;
  }

  // xtask/src/brands.rs:373-378, 598
  } else if ext == "md" {
      scan_markdown(path, text, out);
  ...
  } else if path == "docs/development.md" || is_scanned_non_rs_text_file(path, &ext) {
  ```
- **Suggested fix:** Call `classify` only on workspace-member `pkg_id`s (`member_id_set.contains(pid)`), extract the crate name segment from `[patch.<registry>.<crate>]` headers in `scan_patch_sections`, remove the dead `docs/development.md` branch in `brands.rs`, and route `ASSETS.md`/`ATTRIBUTION.md` explicitly through strict brand-policy scanning (without waivers) in `brands::scan_file`.
- **Related:** `FA-SEC22-07`, `FA-SEC22-11`
