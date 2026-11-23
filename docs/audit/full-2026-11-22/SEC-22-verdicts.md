# Verification Report: SEC-22 — Xtask Policy Gates & CI (`xtask`)

- **Section:** `SEC-22` (`16` files: `xtask/Cargo.toml` [`51` LOC] and `15` modules in `xtask/src/` [`15,840` LOC]; `15,891` total LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-22.md`](../../../docs/audit/full-2026-11-22/SEC-22.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

| Verdict | Count | Findings |
|---|---:|---|
| `CONFIRMED` | 19 | `FA-SEC22-01` .. `FA-SEC22-19` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity (Unique) | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 15 | `FA-SEC22-01` .. `FA-SEC22-15` |
| `S3` | 3 | `FA-SEC22-16`, `FA-SEC22-17`, `FA-SEC22-18` |
| `S4` | 1 | `FA-SEC22-19` |

## File Coverage Completeness Check

Verified `16 / 16` files (`15,891` total LOC, verified directly via `wc -l` at `7c85a8e516165ddadb101236a88eec937bf1f095`; remaining `22` `xtask/src` files belong to `SEC-23` scope) appear in [`SEC-22.md`](../../../docs/audit/full-2026-11-22/SEC-22.md#L10-L30) with exact line counts and `1..=N` inspection ranges (`PASS`):
- [`xtask/Cargo.toml`](../../../xtask/Cargo.toml) (`51`)
- `xtask/src/` (`15` files, `15,840` LOC): `main.rs` (`494`), `assets.rs` (`1,039`), `brands.rs` (`5,711`), `ci.rs` (`223`), `ci_cache.rs` (`1,199`), `ci_quick.rs` (`324`), `cleanroom.rs` (`711`), `layers.rs` (`1,389`), `licenses.rs` (`1,177`), `pending.rs` (`126`), `secrets.rs` (`721`), `telemetry.rs` (`2,219`), `text_decode.rs` (`70`), `version.rs` (`116`), `workflows.rs` (`321`).

---

## Per-Finding Verdicts

### Verdict `FA-SEC22-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Active CI cache invalidation on every [`ci.yml`](../../../.github/workflows/ci.yml#L46-L61) run for all three top-level workspace binaries.
- **Evidence:**
  - Confirmed via `cargo metadata --manifest-path curvebook/Cargo.toml --format-version 1 --no-deps --offline`: Cargo emits `@`-less IDs `path+file:///.../apps/curvebook#0.1.0-alpha.2`, `path+file:///.../apps/curvebook-cli#0.1.0-alpha.2`, and `path+file:///.../xtask#0.1.0-alpha.2`.
  - [`xtask/src/ci_cache.rs:495-503`](../../../xtask/src/ci_cache.rs#L495-L503): `extract_pkg_name_from_id` splits after `#` on `@`, returning `"0.1.0-alpha.2"` when `@` is absent.
  - [`xtask/src/ci_cache.rs:409-413, 454-471`](../../../xtask/src/ci_cache.rs#L409-L471): `find_matching_top_fingerprint` rejects every `.fingerprint/` directory not starting with `"0.1.0-alpha.2-"`, omitting all three top-level binaries from `ActiveUnits`; additionally `dep_artifact_for_top` looks up `deps/{stem}-{hash}` without normalizing `-` to `_` (`curvebook-cli` vs `curvebook_cli`).

---

### Verdict `FA-SEC22-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Policy gate bypass in `cargo xtask brands` on BOM-less UTF-16 ASCII files and non-ASCII-heavy UTF-16 files.
- **Evidence:**
  - [`xtask/src/brands.rs:334-339`](../../../xtask/src/brands.rs#L334-L339): `decode_text_file` runs `std::str::from_utf8(bytes)` before `detect_bomless_utf16(bytes)`. Since BOM-less UTF-16 ASCII consists solely of `0x00..=0x7F` bytes (valid UTF-8 including U+0000), `from_utf8` always succeeds and returns a NUL-interleaved string, leaving `detect_bomless_utf16` unreachable for ASCII text.
  - [`xtask/src/text_decode.rs:7-29`](../../../xtask/src/text_decode.rs#L7-L29): `detect_bomless_utf16` additionally requires `le_ascii * 4 >= total_pairs` / `be_ascii * 4 >= total_pairs` (`>= 25%` ASCII pairs), rejecting UTF-16 files dominated by non-ASCII script code units.

---

### Verdict `FA-SEC22-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `telemetry` Check 1 misses banned browser networking `web-sys` features and aliased inline deps formatted across lines.
- **Evidence:**
  - [`xtask/src/telemetry.rs:156-215, 315-337`](../../../xtask/src/telemetry.rs#L156-L337): `scan_manifest_for_banned_crates` splits each line independently (`line.split_once('=')`) and `extract_inline_package_name` requires `rhs.starts_with('{')` on that single line. Multi-line `features = [\n  "WebSocket",\n]` under `[dependencies.web-sys]` (`section_dependency_name` at `:290-302`) and multi-line inline tables evade Check 1, while `Cargo.lock` records no crate features.

---

### Verdict `FA-SEC22-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `telemetry` Check 2 network/loopback scanner bypasses in Rust production sources and allowlisted loopback files.
- **Evidence:**
  - [`xtask/src/telemetry.rs:18-24, 359-403`](../../../xtask/src/telemetry.rs#L18-L403): Non-allowlisted scanning matches exact `web_sys::WebSocket` etc. strings (missed by `use web_sys::{WebSocket};` or `use web_sys as ws;`) and exact single-word `BANNED_JS_LITERALS` equality (`lit == "fetch"`), missing `fetch_with_str` or `js_sys::eval("fetch(...)")`.
  - [`telemetry.rs:405-557`](../../../xtask/src/telemetry.rs#L405-L557): In allowlisted files, `check_net_alias_imports` (`:449-461`) checks per line (`compact.contains("std::net") && line.contains(" as ")`), missing multi-line `use std::net::{\n TcpStream as S\n};`; `slice_has_loopback_guard` (`:513-535`) returns `true` for any `LOCALHOST`/`is_loopback` mention in the function even if unused; `find_non_loopback_ip_literal` (`:537-557`) only checks dotted IPv4 quads, `0.0.0.0`, and `[::]` (missing non-loopback hostnames and other IPv6 literals); and `collect_rust_fn_spans` (`:483-489`) stops at the first `{` after `fn`, truncating functions with const-generic `{ ... }` expressions in their signatures.

---

### Verdict `FA-SEC22-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `telemetry` Check 4 workflow gate omits `.github/actions/**` and misses quoted `permissions:` and multi-line/line-continued `run:` patterns.
- **Evidence:**
  - [`xtask/src/telemetry.rs:1402-1420`](../../../xtask/src/telemetry.rs#L1402-L1420) and [`xtask/src/workflows.rs:96-117`](../../../xtask/src/workflows.rs#L96-L117): Both `collect_workflow_files` implementations read only `.github/workflows/*.{yml,yaml}`, omitting composite actions under `.github/actions/**` (such as `.github/actions/ci-setup/action.yml`).
  - [`telemetry.rs:851-876, 923-955, 1030-1159`](../../../xtask/src/telemetry.rs#L851-L1159): `handle_permissions_line` matches unquoted `trimmed.strip_prefix("permissions:")`; `check_run_directive` only enters `run_block_col` when `is_yaml_block_scalar` (`|`/`>`) matches, bypassing quoted/plain multi-line `run:` scalars; and both `flag_untrusted_event_expr` and `flag_remote_script_execution` evaluate each line independently, missing `\`-continued pipes and multi-line `${{ ... }}`.

---

### Verdict `FA-SEC22-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `telemetry` Checks 3 and 5 bypasses across web asset extensions, NUL/UTF-16 files, JS/HTML syntax variants, CSP scheme sources, and Netlify per-path `_headers` blocks.
- **Evidence:**
  - [`xtask/src/telemetry.rs:626-628, 653-668, 764-815`](../../../xtask/src/telemetry.rs#L626-L815): Check 3 skips all NUL-byte files (`:626`); `is_web_asset_file` (`:653-662`) excludes `.svg`, `.json`, `.jsx`, `.tsx`, and `.cjs` (rendering `"jsx" | "tsx"` at `:667` unreached by Check 3's URL/API scan); and URL/API scans miss bracketed `window['fetch']`, spaced `new Worker (`, and HTML-entity URLs.
  - [`telemetry.rs:1200-1217, 1388-1399`](../../../xtask/src/telemetry.rs#L1200-L1399): `parse_netlify_global_headers` checks `trimmed == "/*"` and discards subsequent path-specific header blocks in `_headers`; `validate_csp_directives` checks `tok.contains("://") || tok.starts_with("//") || tok == "*"`, allowing scheme sources such as `https:` / `http:` / `wss:`.

---

### Verdict `FA-SEC22-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Coverage gaps across `brands` and `assets` scanners for repo text extensions, `.pdf`/ZIP repo files, `.svgz`, and case-folded suffixes/prefixes.
- **Evidence:**
  - [`xtask/src/brands.rs:117-154, 270-288, 356-389`](../../../xtask/src/brands.rs#L117-L389): `.pdf`, `.svgz`, `.zip`, `.curvebook`, `.cbtemplate` and vendor archive extensions sit in `BINARY_ASSET_EXT` and return `DecodedFile::BinaryAsset` in `scan_repo_checked` (`:285`), bypassing PDF metadata and ZIP scanning. In `scan_file` (`:367-389`), `.plist`/`.plist.in`/`.desktop` require lowercase suffixes, and files with extensions outside `is_scanned_non_rs_text_file` (e.g., `.wxi`, `.hbs`, `.py`, `.r`, `.githooks/*`, `CODEOWNERS`) skip `BRANDS` checks (running only `scan_suite_lines`).
  - [`xtask/src/assets.rs:161-171, 444-447, 512-539`](../../../xtask/src/assets.rs#L161-L539): `verify_asset_binaries` only inspects `png`, `ico`, `icns`, `svg`, `gif` (skipping `.svgz`, `.pdf`); `verify_assets` checks committed repo filenames rather than ZIP-contained fonts; and `is_original_only_visual_asset` checks case-sensitive `path.starts_with(prefix)`.

---

### Verdict `FA-SEC22-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Parser/decoding bypasses in `brands.rs` for Rust string escapes, SVG comments/entities/tags, and embedded data URIs.
- **Evidence:**
  - [`xtask/src/brands.rs:1538-1570`](../../../xtask/src/brands.rs#L1538-L1570): `RustLexer::read_string_escape` handles `\n`, `\r`, `\t`, `\0`, `\\`, `\"`, `\'`, and maps any other character via `other => char::from(other)` without decoding `\xHH` or `\u{...}` escapes.
  - [`brands.rs:726-735, 767-787, 810-860, 2538-2598`](../../../xtask/src/brands.rs#L726-L2598): Both `SvgState::split_line_chunks` (`:726-735, 748-754`) and `check_svg_metadata` (`:2540-2545`) skip XML `<!-- ... -->` comments; `scan_svg` scans raw chunks without decoding XML character entities, while `check_svg_metadata` ignores tag attributes (`:2559-2561`) and `<textPath>` (`is_text_tag` checks only `"text" | "tspan"` at `:2564`); and `check_and_strip_svg_data_uri` (`:850-856`) only inspects exact `subtype.eq_ignore_ascii_case("png")` before replacing all `data:image/*;base64,...` URIs with `"data:image/stripped"`.

---

### Verdict `FA-SEC22-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `cargo xtask secrets` silently skips NUL/UTF-16 files and misclassifies added `++ ` diff lines in `--history`.
- **Evidence:**
  - [`xtask/src/secrets.rs:124-126, 156-158`](../../../xtask/src/secrets.rs#L124-L158): Both `run_history` (`if buf.contains(&0) { continue; }`) and `scan_repo` (`if bytes.contains(&0) { continue; }`) silently skip UTF-16 and NUL-containing files without attempting UTF-16 decoding.
  - [`secrets.rs:195-214`](../../../xtask/src/secrets.rs#L195-L214): `scan_git_diff_line` checks `raw_line.starts_with("+++ ")` before `raw_line.strip_prefix('+')`, so any added diff line starting with `++ ` (e.g., `++ foo` -> `+++ foo` in `git log -p`) resets `state.current_file` instead of being scanned. Also verified [`secrets.rs:449, 496-498`](../../../xtask/src/secrets.rs#L449-L498) skip `#`/`//` comments and `b"..."`/`c"..."` literals.

---

### Verdict `FA-SEC22-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `ci quick` omits non-crate embedded/tested files from package selection, and `cargo_metadata` can silently rewrite `Cargo.lock`.
- **Evidence:**
  - [`xtask/src/ci_quick.rs:24-26, 178-210`](../../../xtask/src/ci_quick.rs#L24-L210): `WORKSPACE_WIDE_FILES` and `WORKSPACE_WIDE_DIRS` omit `.config/nextest.toml`, `deny.toml` (`licenses.rs:1029`), `assets/app-icon/` (embedded via `include_bytes!` at [`crates/ui-egui/src/egui_shell/brand.rs:9`](../../../crates/ui-egui/src/egui_shell/brand.rs#L9)), `docs/control-protocol.md` (read in [`crates/ui-egui/tests/audit_r1_s4c_s10_09.rs:107`](../../../crates/ui-egui/tests/audit_r1_s4c_s10_09.rs#L107)), `.github/` (embedded at [`xtask/src/ci.rs:214-215`](../../../xtask/src/ci.rs#L214-L215)), and `packaging/windows/` (embedded at [`xtask/src/msix.rs:11-12`](../../../xtask/src/msix.rs#L11-L12)), returning `Selection::Packages(empty)`.
  - [`xtask/src/main.rs:208-222`](../../../xtask/src/main.rs#L208-L222): `cargo_metadata` and `cargo_metadata_resolved` run `cargo metadata --format-version 1` without `--locked`.

---

### Verdict `FA-SEC22-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `layers` source hygiene (`raw-process` and `ui-raw-io`) bypassed by parent module namespace aliases, multi-line `::`, and omitted `std::fs` mutators.
- **Evidence:**
  - [`xtask/src/layers.rs:452-474, 509-538, 580-618, 996-1045`](../../../xtask/src/layers.rs#L452-L1045): `expand_use_tree` + `strip_use_alias` resolves `use std::process as proc;` to `"std::process"` and `use std::fs as disk;` to `"std::fs"`, neither of which appears in the leaf-only `scan_use_statements` allowlists (`:510, 518-535`). `scan_patterns` checks single lines verbatim without collapsing newlines around `::`, and `UI_IO_PATTERNS` omits `create_dir*`, `remove_dir*`, `set_permissions`, `hard_link`, and `symlink`.

---

### Verdict `FA-SEC22-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Fail-open release/archive scanning across nested depth >= 2, malformed non-vendor ZIPs, non-UTF-8 dist files, and nested reference directory paths.
- **Evidence:**
  - [`xtask/src/cleanroom.rs:155, 219-242`](../../../xtask/src/cleanroom.rs#L155-L242): `depth >= MAX_ARCHIVE_DEPTH` (`2`) only records a hit when `is_vendor_archive` is true (silently skipping non-vendor nested ZIPs at depth >= 2), and failed `ZipArchive::from_bytes_with_limits` on non-vendor ZIPs falls back to raw-compressed-byte text scanning without an error; line `:155` checks `starts_with` on the forbidden reference directory prefix rather than `contains`.
  - [`xtask/src/brands.rs:407-428, 4511, 4540-4560`](../../../xtask/src/brands.rs#L407-L4560) and [`xtask/src/assets.rs:722`](../../../xtask/src/assets.rs#L722): Both `brands::scan_dist_bytes` and `assets::scan_dist_asset_bytes` silently stop ZIP recursion at `depth >= 2`; `scan_dist_bytes` silently ignores `decode_text_file` errors (`let Ok(...) else { return Ok(()); }` at `:4540`); and `scan_suite_lines_with_scope` (`:409, :4558`) only enables `ADAPTED_UPSTREAM_BRAND` checking when `is_crates_or_apps_path(path)` is true (`crates/` or `apps/`), skipping it for `dist/` files.

---

### Verdict `FA-SEC22-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Deterministic slice-bounds panic on `"|"` in attribution markdown parsed by `cargo xtask assets` / `release-scan`.
- **Evidence:**
  - [`xtask/src/assets.rs:188-195, 449-450, 756-766`](../../../xtask/src/assets.rs#L188-L766): `parse_table_cells` checks `trimmed.starts_with('|') && trimmed.ends_with('|')` and directly slices `&trimmed[1..trimmed.len() - 1]` without verifying `trimmed.len() >= 2`. A lone `"|"` line produces `&trimmed[1..0]` and panics.

---

### Verdict `FA-SEC22-14` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Policy parity gap in `xtask licenses` (`--all-features` omitted vs `deny.toml`), target dependency wildcard gap, and swallowed `ATTRIBUTION.md`/`NOTICE` read failures.
- **Evidence:**
  - [`xtask/src/licenses.rs:243-253`](../../../xtask/src/licenses.rs#L243-L253) vs [`deny.toml:6-7`](../../../deny.toml#L6-L7): `run_locked_offline_metadata` passes `["metadata", "--format-version", "1", "--locked", "--offline"]` without `--all-features`, whereas `deny.toml` sets `[graph] all-features = true`.
  - [`licenses.rs:592-603, 750-753`](../../../xtask/src/licenses.rs#L592-L753): `is_dependency_section` omits `.dependencies.` / `.dev-dependencies.` / `.build-dependencies.` substring checks needed for `[target.<cfg>.dependencies.<crate>]` tables; `generate_licenses_html` fallback discards read/UTF-8 errors on `ATTRIBUTION.md` and `NOTICE` with `.unwrap_or_default()` and writes directly via `std::fs::write`.

---

### Verdict `FA-SEC22-15` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — False-green local `workflows` gate when only one external linter is installed, swallowed `repo_files` error in `collect_shell_scripts`, and YAML offline check gaps.
- **Evidence:**
  - [`xtask/src/workflows.rs:47-57, 72-86, 120-165, 184-215`](../../../xtask/src/workflows.rs#L47-L215): When `CI` is unset, `run_actionlint_if_installed` and `run_shellcheck_if_installed` warn and return `Ok(false)`, and `workflows::run` succeeds if either `actionlint_ran || shellcheck_ran`; `collect_shell_scripts` discards `crate::repo_files(root)` errors inside `if ... && let Ok(tracked)` and falls back to `walk_shell_targets` which skips `.github/` (`name_str.starts_with('.') && name_str != ".githooks"`); and `check_workflows_offline_yaml` only flags `line.starts_with('\t')` (`:72`) and checks `!line[open + 3..].contains("}}")` (`:82-83`) which misses a second unclosed `${{`.

---

### Verdict `FA-SEC22-16` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Relative `CARGO_TARGET_DIR` mismatch in `cmd_wasm` (`trunk`) and `ci_cache`, with absolute path workaround.
- **Evidence:**
  - [`xtask/src/main.rs:327-335`](../../../xtask/src/main.rs#L327-L335) and [`xtask/src/ci_cache.rs:178-183`](../../../xtask/src/ci_cache.rs#L178-L183): Both wrap relative `CARGO_TARGET_DIR` values directly via `PathBuf::from` rather than anchoring relative paths to `root` (contrast [`xtask/src/bundle.rs:82-90`](../../../xtask/src/bundle.rs#L82-L90)), causing `trunk build --dist` (with `current_dir = root/apps/curvebook-web`) and subdirectory `ci-cache` runs to read/write different directories.

---

### Verdict `FA-SEC22-17` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `version` key-prefix matching / non-atomic update without lockfile rollback and `pending` strict `R<m>.<n>` token splitting.
- **Evidence:**
  - [`xtask/src/version.rs:18-22, 34-46, 65-78`](../../../xtask/src/version.rs#L18-L78): `read_workspace_version` and `replace_workspace_version` check `t == "[workspace.package]"` and `t.strip_prefix("version")` / `t.starts_with("version") && t.contains('=')` (matching `version_*` keys), write `Cargo.toml.tmp` and rename before calling `refresh_lockfile(root)?` with no rollback of `Cargo.toml` on `cargo update` failure.
  - [`xtask/src/pending.rs:85-94`](../../../xtask/src/pending.rs#L85-L94): `extract_roadmap_id` checks `minor.bytes().all(|b| b.is_ascii_digit())` without stripping trailing punctuation or backticks (e.g., `R10.1:`).

---

### Verdict `FA-SEC22-18` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Symlink-following directory recursion (`path.is_dir()`) and unbounded `std::fs::read` on unpacked `dist/` artifacts.
- **Evidence:**
  - [`xtask/src/telemetry.rs:592, 1429, 1448`](../../../xtask/src/telemetry.rs#L592-L1455), [`xtask/src/layers.rs:555`](../../../xtask/src/layers.rs#L555), and [`xtask/src/licenses.rs:551`](../../../xtask/src/licenses.rs#L551): All five walkers recurse on `path.is_dir()` (following symlinks).
  - [`xtask/src/assets.rs:651`](../../../xtask/src/assets.rs#L651), [`xtask/src/brands.rs:4504`](../../../xtask/src/brands.rs#L4504), and [`xtask/src/cleanroom.rs:159`](../../../xtask/src/cleanroom.rs#L159): All three `dist/` scanners call unbounded `std::fs::read(...)` on every file before checking `ZipLimits` or extension limits.

---

### Verdict `FA-SEC22-19` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Cleanup / latent edge cases in `layers::classify`, `[patch.<reg>.<crate>]`, and unreachable `brands.rs` branches.
- **Evidence:**
  - [`xtask/src/layers.rs:96-99, 356-363, 406-431`](../../../xtask/src/layers.rs#L96-L431): `collect_resolved_transitive_deps` invokes `classify(pname)` on non-workspace crates (`:356, 361`), stopping traversal on any external crate matching `Class::Testkit | Class::Exempt` (`testkit`, `curvebook`, `cli`, `web`, `xtask`), and `scan_patch_sections` inspects only `lhs` keys without extracting `<crate>` from `[patch.<registry>.<crate>]` section headers.
  - [`xtask/src/brands.rs:373-379, 598, 4559`](../../../xtask/src/brands.rs#L373-L598): In `scan_file`, `path == "docs/development.md"` (`:378`) is dead after `else if ext == "md"` (`:373`), and repository `ASSETS.md`/`ATTRIBUTION.md` route through `scan_markdown` (`:373`) rather than `scan_plain_lines_no_waiver` (`:598`). (Verification precision note: `is_asset_doc` at `:598` remains reachable when scanning packaged `ASSETS.md`/`ATTRIBUTION.md` inside `dist/` via `scan_dist_bytes` at `:4559`, which has no `.md` branch.)
