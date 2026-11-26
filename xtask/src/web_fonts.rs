//! `cargo xtask web-fonts`: stage font faces listed in `crates/text/web-fonts.txt` from
//! `$CURVEBOOK_FONTS_DIR/fonts/` into the Trunk staging directory (`$TRUNK_STAGING_DIR/fonts` or
//! `dist/web/fonts`). No-op when `CURVEBOOK_FONTS_DIR` is unset.
//!
//! Also provides `R10.1` / `R10.9` / `R10.22` web release bundle budget and packaging readiness
//! validation (`<= 6_243_591` B compressed `curvebook-web-*.zip`).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use curvebook_platform::{read_regular_file_bytes, write_atomic};

const MANIFEST_REL: &str = "crates/text/web-fonts.txt";
const TRUNK_TOML_REL: &str = "apps/curvebook-web/Trunk.toml";
const WEB_INDEX_REL: &str = "apps/curvebook-web/index.html";
const PACKAGE_SH_REL: &str = "packaging/web/package.sh";
const MAX_BUILT_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_WEB_FONT_BYTES: usize = 32 * 1024 * 1024;

/// `R10.22` web release zip budget (`1.5 * 4,162,394 B` pre-`R0.20` baseline).
pub const MAX_WEB_RELEASE_ZIP_BYTES: u64 = 6_243_591;

fn resolve_staging_root(root: &Path) -> PathBuf {
    match std::env::var_os("TRUNK_STAGING_DIR").filter(|v| !v.is_empty()) {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() { path } else { root.join(path) }
        }
        None => root.join("dist/web"),
    }
}

pub fn run(root: &Path) -> Result<(), String> {
    let staging_root = resolve_staging_root(root);
    if let Ok(meta) = std::fs::symlink_metadata(&staging_root)
        && meta.file_type().is_symlink()
    {
        return Err(format!("staging directory {} must not be a symlink", staging_root.display()));
    }
    let built_index = staging_root.join("index.html");
    if let Ok(meta) = std::fs::symlink_metadata(&built_index) {
        if meta.file_type().is_symlink() {
            return Err(format!("built index {} must not be a symlink", built_index.display()));
        }
        if meta.file_type().is_file() {
            let hashes = rewrite_built_index_csp_file(&built_index)?;
            println!("web-fonts: stamped {} SHA-256 script CSP hash(es) into {}", hashes.len(), built_index.display());
        }
    }

    let Some(fonts_root) = std::env::var_os("CURVEBOOK_FONTS_DIR").filter(|v| !v.is_empty()) else {
        println!("web-fonts: CURVEBOOK_FONTS_DIR not set; skipping optional font staging");
        return Ok(());
    };
    let manifest_path = root.join(MANIFEST_REL);
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{MANIFEST_REL}: {e}"))?;
    let faces = parse_manifest(&manifest);
    validate_manifest_entries(&faces)?;

    let staging = staging_root.join("fonts");
    let src_base = PathBuf::from(fonts_root).join("fonts");
    let copied = stage_font_files(&src_base, &staging, &faces)?;
    println!("web-fonts: staged {copied} font face(s) into {}", staging.display());
    Ok(())
}

fn reject_symlink_rel_components(base: &Path, rel: &str) -> Result<(), String> {
    let mut cur = base.to_path_buf();
    for seg in rel.split('/') {
        cur.push(seg);
        if let Ok(meta) = std::fs::symlink_metadata(&cur)
            && meta.file_type().is_symlink()
        {
            return Err(format!("{}: symlink forbidden during font staging", cur.display()));
        }
    }
    Ok(())
}

fn stage_font_files(src_base: &Path, staging: &Path, faces: &[String]) -> Result<usize, String> {
    if let Ok(meta) = std::fs::symlink_metadata(src_base)
        && (meta.file_type().is_symlink() || !meta.file_type().is_dir())
    {
        return Err(format!("font source directory {} must be a regular non-symlink directory", src_base.display()));
    }
    if let Ok(meta) = std::fs::symlink_metadata(staging)
        && (meta.file_type().is_symlink() || !meta.file_type().is_dir())
    {
        return Err(format!("font staging directory {} must be a regular non-symlink directory", staging.display()));
    }
    std::fs::create_dir_all(staging).map_err(|e| format!("mkdir {}: {e}", staging.display()))?;

    let mut copied = 0usize;
    for rel in faces {
        reject_symlink_rel_components(src_base, rel)?;
        reject_symlink_rel_components(staging, rel)?;

        let src = src_base.join(rel);
        let src_meta = std::fs::symlink_metadata(&src).map_err(|e| format!("stat {}: {e}", src.display()))?;
        if src_meta.file_type().is_symlink() || !src_meta.file_type().is_file() {
            return Err(format!("{}: font source must be a regular non-symlink file", src.display()));
        }
        let bytes = read_regular_file_bytes(&src, MAX_WEB_FONT_BYTES).map_err(|e| format!("read {}: {e}", src.display()))?;

        let dst = staging.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        if let Ok(dst_meta) = std::fs::symlink_metadata(&dst)
            && dst_meta.file_type().is_symlink()
        {
            return Err(format!("{}: font destination must not be a symlink", dst.display()));
        }
        write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// Compute exact CSP `'sha256-...'` hashes over every inline `<script>` block in built `html`
/// and rewrite `<meta http-equiv="Content-Security-Policy" ...>` removing `'unsafe-inline'`.
pub fn rewrite_built_index_csp_hashes(html: &str) -> Result<(String, Vec<String>), String> {
    let hashes = extract_inline_script_hashes(html)?;
    let rewritten = rewrite_csp_meta_tag(html, &hashes)?;
    let active_html = strip_html_comments(&rewritten)?;
    if active_html.to_ascii_lowercase().contains("'unsafe-inline'")
        || crate::pages_site::unescape_html_attr(&active_html).to_ascii_lowercase().contains("'unsafe-inline'")
    {
        return Err(String::from("built index.html still contains `'unsafe-inline'` after CSP hash rewrite"));
    }
    Ok((rewritten, hashes))
}

fn rewrite_built_index_csp_file(index_path: &Path) -> Result<Vec<String>, String> {
    let bytes = read_regular_file_bytes(index_path, MAX_BUILT_INDEX_BYTES).map_err(|e| format!("read {}: {e}", index_path.display()))?;
    let html = String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", index_path.display()))?;
    let (rewritten, hashes) = rewrite_built_index_csp_hashes(&html)?;
    if rewritten != html {
        write_atomic(index_path, rewritten.as_bytes()).map_err(|e| format!("write {}: {e}", index_path.display()))?;
    }
    Ok(hashes)
}

fn skip_raw_text_element(html: &str, body_start: usize, tag_name: &str) -> Result<(usize, usize), String> {
    crate::pages_site::skip_html_raw_or_rcdata_element(html, body_start, tag_name).map_err(|e| format!("{e} in index.html"))
}

fn extract_inline_script_hashes(html: &str) -> Result<Vec<String>, String> {
    let mut hashes = Vec::new();
    let mut pos = 0usize;
    while let Some(rel_lt) = html.get(pos..).and_then(|s| s.find('<')) {
        let lt_pos = pos + rel_lt;
        let after_lt = html.get(lt_pos + 1..).unwrap_or("");
        if !crate::pages_site::is_html_tag_start(after_lt) {
            pos = lt_pos + 1;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let close_end =
                crate::pages_site::find_html_comment_end(after_comment).ok_or_else(|| String::from("unclosed HTML comment in index.html"))?;
            pos = lt_pos + 4 + close_end;
            continue;
        }
        let close_rel =
            crate::pages_site::find_html_tag_close(after_lt).ok_or_else(|| String::from("unclosed HTML tag or attribute quote in index.html"))?;
        let tag_body = after_lt.get(..close_rel).ok_or_else(|| String::from("unclosed HTML tag in index.html"))?;
        let body_start = lt_pos + 1 + close_rel + 1;
        let trimmed = tag_body.trim();
        let is_closing = trimmed.starts_with('/');
        let tag_lower = crate::pages_site::extract_html_tag_name(trimmed).to_ascii_lowercase();
        if !is_closing && tag_lower == "script" {
            // HTML5 `<script>` is never a void element (`<script/>...</script>` still executes its body).
            let (body_end, next_pos) = skip_raw_text_element(html, body_start, "script")?;
            let attrs = crate::pages_site::parse_html_tag_attributes(trimmed);
            let has_src = attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case("src"));
            if !has_src {
                let body = html.get(body_start..body_end).ok_or_else(|| String::from("unclosed `<script>` element"))?;
                let digest = crate::release_scan::sha256_bytes(body.as_bytes());
                let token = format!("'sha256-{}'", encode_base64_std(&digest));
                if !hashes.contains(&token) {
                    hashes.push(token);
                }
            }
            pos = next_pos;
        } else if !is_closing && matches!(tag_lower.as_str(), "style" | "title" | "textarea") {
            let (_, next_pos) = skip_raw_text_element(html, body_start, &tag_lower)?;
            pos = next_pos;
        } else {
            pos = body_start;
        }
    }
    Ok(hashes)
}

struct CspMetaMatch {
    abs_token_start: usize,
    abs_token_end: usize,
    decoded_policy: String,
}

fn find_single_csp_meta_span(html: &str) -> Result<CspMetaMatch, String> {
    let mut pos = 0usize;
    let mut found_match: Option<CspMetaMatch> = None;
    while let Some(rel_lt) = html.get(pos..).and_then(|s| s.find('<')) {
        let lt_pos = pos + rel_lt;
        let after_lt = html.get(lt_pos + 1..).unwrap_or("");
        if !crate::pages_site::is_html_tag_start(after_lt) {
            pos = lt_pos + 1;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let close_end =
                crate::pages_site::find_html_comment_end(after_comment).ok_or_else(|| String::from("unclosed HTML comment in index.html"))?;
            pos = lt_pos + 4 + close_end;
            continue;
        }
        let close_rel =
            crate::pages_site::find_html_tag_close(after_lt).ok_or_else(|| String::from("unclosed HTML tag or attribute quote in index.html"))?;
        let tag_body = after_lt.get(..close_rel).ok_or_else(|| String::from("unclosed HTML tag in index.html"))?;
        let next_after_tag = lt_pos + 1 + close_rel + 1;
        let trimmed = tag_body.trim();
        let is_closing = trimmed.starts_with('/');
        let tag_lower = crate::pages_site::extract_html_tag_name(trimmed).to_ascii_lowercase();
        if !is_closing && matches!(tag_lower.as_str(), "script" | "style" | "title" | "textarea") {
            let (_, next_pos) = skip_raw_text_element(html, next_after_tag, &tag_lower)?;
            pos = next_pos;
            continue;
        }
        if !is_closing && tag_lower == "meta" {
            let parsed = parse_meta_http_equiv_and_content(tag_body)?;
            if parsed.is_csp {
                let (tok_start, tok_end, decoded_policy) =
                    parsed.content.ok_or_else(|| String::from("Content-Security-Policy `<meta>` lacks `content=\"...\"`"))?;
                if found_match.is_some() {
                    return Err(String::from("multiple active Content-Security-Policy `<meta>` tags found"));
                }
                found_match = Some(CspMetaMatch { abs_token_start: lt_pos + 1 + tok_start, abs_token_end: lt_pos + 1 + tok_end, decoded_policy });
            }
        }
        pos = next_after_tag;
    }
    found_match.ok_or_else(|| String::from("built index.html missing `<meta http-equiv=\"Content-Security-Policy\">`"))
}

fn parse_csp_directives(policy: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut directives = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_dir in policy.split(';') {
        let trimmed = raw_dir.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(name_raw) = parts.next() else {
            continue;
        };
        let name = name_raw.to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate CSP directive `{name}`"));
        }
        let tokens: Vec<String> = parts.map(str::to_owned).collect();
        directives.push((name, tokens));
    }
    Ok(directives)
}

fn rewrite_csp_meta_tag(html: &str, script_hashes: &[String]) -> Result<String, String> {
    let csp = find_single_csp_meta_span(html)?;
    let parsed = parse_csp_directives(&csp.decoded_policy)?;
    let mut saw_script_src = false;
    let mut rebuilt = Vec::with_capacity(parsed.len());
    for (name, tokens) in parsed {
        if name == "script-src" {
            saw_script_src = true;
            let mut parts = vec![String::from("script-src"), String::from("'self'")];
            parts.extend(script_hashes.iter().cloned());
            parts.push(String::from("'wasm-unsafe-eval'"));
            rebuilt.push(parts.join(" "));
        } else {
            if tokens.iter().any(|t| t.eq_ignore_ascii_case("'unsafe-inline'")) {
                return Err(format!("CSP directive `{name}` must not allow `'unsafe-inline'`"));
            }
            if tokens.is_empty() {
                rebuilt.push(name);
            } else {
                rebuilt.push(format!("{name} {}", tokens.join(" ")));
            }
        }
    }
    if !saw_script_src {
        return Err(String::from("Content-Security-Policy `<meta>` is missing a `script-src` directive"));
    }
    let new_policy = rebuilt.join("; ");
    let prefix = html.get(..csp.abs_token_start).ok_or_else(|| String::from("invalid CSP meta tag boundary"))?;
    let suffix = html.get(csp.abs_token_end..).ok_or_else(|| String::from("invalid CSP meta tag boundary"))?;
    let mut out = String::with_capacity(html.len() + 256);
    out.push_str(prefix);
    out.push('"');
    out.push_str(&new_policy);
    out.push('"');
    out.push_str(suffix);
    Ok(out)
}

struct ParsedMetaCsp {
    is_csp: bool,
    content: Option<(usize, usize, String)>,
}

fn parse_meta_http_equiv_and_content(tag_body: &str) -> Result<ParsedMetaCsp, String> {
    let bytes = tag_body.as_bytes();
    let mut i = bytes.iter().position(|b| b.is_ascii_whitespace() || *b == b'/').unwrap_or(bytes.len());
    let mut is_csp = false;
    let mut http_equiv_count = 0usize;
    let mut content: Option<(usize, usize, String)> = None;
    let mut content_count = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let k_start = i;
        // HTML5 §13.2.5.32: the first non-whitespace, non-`/` byte starts the attribute name even if `=`.
        i += 1;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let key = tag_body.get(k_start..i).unwrap_or("");
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let (tok_start, tok_end, v_start, v_end) = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let q = bytes[i];
            let ts = i;
            i += 1;
            let vs = i;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            let ve = i;
            if i < bytes.len() {
                i += 1;
            }
            (ts, i, vs, ve)
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            (start, i, start, i)
        };
        if key.eq_ignore_ascii_case("http-equiv") {
            http_equiv_count += 1;
            let decoded = crate::pages_site::unescape_html_attr(tag_body.get(v_start..v_end).unwrap_or(""));
            if decoded.trim().eq_ignore_ascii_case("content-security-policy") {
                is_csp = true;
            }
        } else if key.eq_ignore_ascii_case("content") {
            content_count += 1;
            if content.is_none() {
                let decoded = crate::pages_site::unescape_html_attr(tag_body.get(v_start..v_end).unwrap_or(""));
                content = Some((tok_start, tok_end, decoded));
            }
        }
    }
    if is_csp && (http_equiv_count > 1 || content_count > 1) {
        return Err(String::from("duplicate `http-equiv` or `content` attribute on Content-Security-Policy `<meta>`"));
    }
    Ok(ParsedMetaCsp { is_csp, content })
}

fn encode_base64_std(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(char::from(TABLE[((triple >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3f) as usize]));
        out.push(if chunk.len() > 1 { char::from(TABLE[((triple >> 6) & 0x3f) as usize]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[(triple & 0x3f) as usize]) } else { '=' });
    }
    out
}

#[must_use]
pub fn parse_manifest(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split('#').next().unwrap_or_default().trim()).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

fn validate_manifest_entries(faces: &[String]) -> Result<(), String> {
    for rel in faces {
        if rel.is_empty()
            || rel.contains(['\\', '\0', ':'])
            || rel.starts_with('/')
            || rel.ends_with('/')
            || rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
            || Path::new(rel).components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(format!("{MANIFEST_REL}: unsafe font manifest entry `{rel}`"));
        }
    }
    Ok(())
}

/// Verify that a built `curvebook-web-*.zip` archive is a valid non-empty ZIP within `MAX_WEB_RELEASE_ZIP_BYTES`.
pub fn verify_web_bundle_zip_bytes(zip_path: &Path) -> Result<u64, String> {
    let meta = std::fs::symlink_metadata(zip_path).map_err(|e| format!("stat {}: {e}", zip_path.display()))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("{} is not a regular non-symlink file", zip_path.display()));
    }
    let size = meta.len();
    if size == 0 {
        return Err(format!("{} is empty (0 B)", zip_path.display()));
    }
    if size > MAX_WEB_RELEASE_ZIP_BYTES {
        return Err(format!("web release bundle {} exceeds R10.22 budget: {size} B > {MAX_WEB_RELEASE_ZIP_BYTES} B", zip_path.display()));
    }
    let mut file = std::fs::File::open(zip_path).map_err(|e| format!("open {}: {e}", zip_path.display()))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| format!("read header {}: {e}", zip_path.display()))?;
    if &magic != b"PK\x03\x04" {
        return Err(format!("{} does not start with ZIP local file signature", zip_path.display()));
    }
    Ok(size)
}

/// Parse whether `CURVEBOOK_REQUIRE_WEB_BUNDLE` requires a built `dist/release/curvebook-web-*.zip`.
///
/// Unset, empty, `0`, `false`, `no`, and `off` (case-insensitive) return `false`; any other value
/// (such as `1` or `true`) returns `true` so release/CI bundle checks fail closed.
#[must_use]
pub fn parse_require_web_bundle_env(raw: Option<&std::ffi::OsStr>) -> bool {
    crate::workflows::parse_ci_env_value(raw)
}

/// Execute `cargo xtask web-bundle`.
pub fn run_bundle_check(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask web-bundle (unexpected args: `{}`)", args.join(" ")));
    }
    check_web_bundle_readiness(root)
}

/// Verify web release packaging readiness and check any built `dist/release/curvebook-web-*.zip` against `MAX_WEB_RELEASE_ZIP_BYTES`.
pub fn check_web_bundle_readiness(root: &Path) -> Result<(), String> {
    check_web_bundle_readiness_with_env(root, std::env::var_os("CURVEBOOK_REQUIRE_WEB_BUNDLE").as_deref())
}

/// Testable core of [`check_web_bundle_readiness`] accepting an explicit `CURVEBOOK_REQUIRE_WEB_BUNDLE` value.
pub fn check_web_bundle_readiness_with_env(root: &Path, require_bundle_env: Option<&std::ffi::OsStr>) -> Result<(), String> {
    let manifest_raw = std::fs::read_to_string(root.join(MANIFEST_REL)).map_err(|e| format!("read {MANIFEST_REL}: {e}"))?;
    let faces = parse_manifest(&manifest_raw);
    validate_manifest_entries(&faces)?;

    let trunk_toml_raw = std::fs::read_to_string(root.join(TRUNK_TOML_REL)).map_err(|e| format!("read {TRUNK_TOML_REL}: {e}"))?;
    let trunk_toml = strip_hash_comments(&trunk_toml_raw);
    for required in ["public_url = \"./\"", "filehash = true", "web-fonts"] {
        if !trunk_toml.contains(required) {
            return Err(format!("{TRUNK_TOML_REL} missing required setting `{required}`"));
        }
    }

    let index_html_raw = std::fs::read_to_string(root.join(WEB_INDEX_REL)).map_err(|e| format!("read {WEB_INDEX_REL}: {e}"))?;
    validate_source_index_html(&index_html_raw)?;

    let pkg_sh_raw = std::fs::read_to_string(root.join(PACKAGE_SH_REL)).map_err(|e| format!("read {PACKAGE_SH_REL}: {e}"))?;
    let pkg_sh = strip_hash_comments(&pkg_sh_raw);
    if !pkg_sh.contains("--cargo-profile web-release") {
        return Err(format!("{PACKAGE_SH_REL} missing `--cargo-profile web-release`"));
    }

    let cargo_toml_raw = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("read Cargo.toml: {e}"))?;
    check_cargo_web_release_profile(&cargo_toml_raw)?;

    let release_dir = root.join("dist/release");
    let mut checked_zips = 0usize;
    if std::fs::symlink_metadata(&release_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&release_dir).map_err(|e| format!("read {}: {e}", release_dir.display()))? {
            let path = entry.map_err(|e| format!("read entry in {}: {e}", release_dir.display()))?.path();
            if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("curvebook-web-") && n.ends_with(".zip")) {
                entries.push(path);
            }
        }
        entries.sort();
        for zip_path in entries {
            let size = verify_web_bundle_zip_bytes(&zip_path)?;
            println!("web-bundle: {} = {size} B (budget <= {MAX_WEB_RELEASE_ZIP_BYTES} B)", zip_path.display());
            checked_zips += 1;
        }
    }

    let require_bundle = parse_require_web_bundle_env(require_bundle_env);
    if checked_zips == 0 {
        if require_bundle {
            return Err(String::from("CURVEBOOK_REQUIRE_WEB_BUNDLE is set, but no dist/release/curvebook-web-*.zip archive exists"));
        }
        println!(
            "web-bundle: CI readiness OK ({} staged manifest entry/ies, budget <= {MAX_WEB_RELEASE_ZIP_BYTES} B; no local zip in dist/release/)",
            faces.len()
        );
    }
    Ok(())
}

fn validate_source_index_html(index_html_raw: &str) -> Result<(), String> {
    let index_html = strip_html_comments(index_html_raw)?;
    if !index_html.contains("data-wasm-opt") {
        return Err(format!("{WEB_INDEX_REL} missing required attribute `data-wasm-opt`"));
    }
    let csp = find_single_csp_meta_span(index_html_raw).map_err(|e| format!("{WEB_INDEX_REL}: {e}"))?;
    let directives = parse_csp_directives(&csp.decoded_policy).map_err(|e| format!("{WEB_INDEX_REL}: {e}"))?;

    let find_dir = |name: &str| directives.iter().find(|(n, _)| n == name).map(|(_, toks)| toks);
    let default_src = find_dir("default-src").ok_or_else(|| format!("{WEB_INDEX_REL} missing required CSP directive `default-src 'self'`"))?;
    if !default_src.iter().any(|t| t.eq_ignore_ascii_case("'self'")) {
        return Err(format!("{WEB_INDEX_REL}: `default-src` must include `'self'`"));
    }
    if default_src.iter().any(|t| t.eq_ignore_ascii_case("'unsafe-inline'")) {
        return Err(format!("{WEB_INDEX_REL} must not allow `'unsafe-inline'` in `default-src`"));
    }

    let style_src = find_dir("style-src").ok_or_else(|| format!("{WEB_INDEX_REL} missing required CSP directive `style-src 'self'`"))?;
    if !style_src.iter().any(|t| t.eq_ignore_ascii_case("'self'")) {
        return Err(format!("{WEB_INDEX_REL}: `style-src` must include `'self'`"));
    }
    for style_dir in ["style-src", "style-src-elem", "style-src-attr"] {
        if let Some(tokens) = find_dir(style_dir)
            && tokens.iter().any(|t| t.eq_ignore_ascii_case("'unsafe-inline'"))
        {
            return Err(format!("{WEB_INDEX_REL} must not allow `'unsafe-inline'` in `{style_dir}`"));
        }
    }
    Ok(())
}

fn check_cargo_web_release_profile(cargo_toml_raw: &str) -> Result<(), String> {
    let mut in_web_release = false;
    let mut saw_section = false;
    let mut has_lto_fat = false;
    let mut has_strip_symbols = false;
    for raw_line in cargo_toml_raw.lines() {
        let line = crate::scan_util::strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let section = header.trim();
            in_web_release = section == "profile.web-release";
            if in_web_release {
                saw_section = true;
            }
            continue;
        }
        if in_web_release && let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = crate::scan_util::unquote_scalar(v);
            if key == "lto" && val == "fat" {
                has_lto_fat = true;
            } else if key == "strip" && val == "symbols" {
                has_strip_symbols = true;
            }
        }
    }
    if !saw_section {
        return Err(String::from("Cargo.toml missing `[profile.web-release]` for web release size budget"));
    }
    if !has_lto_fat {
        return Err(String::from("Cargo.toml `[profile.web-release]` missing `lto = \"fat\"` for web release size budget"));
    }
    if !has_strip_symbols {
        return Err(String::from("Cargo.toml `[profile.web-release]` missing `strip = \"symbols\"` for web release size budget"));
    }
    Ok(())
}

fn strip_hash_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut in_double = false;
        let mut in_single = false;
        let mut escaped = false;
        let mut cut = line.len();
        for (idx, ch) in line.char_indices() {
            if in_double {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_double = false;
                }
            } else if in_single {
                if ch == '\'' {
                    in_single = false;
                }
            } else {
                match ch {
                    '"' => in_double = true,
                    '\'' => in_single = true,
                    '#' => {
                        cut = idx;
                        break;
                    }
                    _ => {}
                }
            }
        }
        out.push_str(line.get(..cut).unwrap_or(line));
        out.push('\n');
    }
    out
}

fn strip_html_comments(html: &str) -> Result<String, String> {
    let mut out = String::with_capacity(html.len());
    let mut pos = 0usize;
    while let Some(rel_lt) = html.get(pos..).and_then(|s| s.find('<')) {
        let lt_pos = pos + rel_lt;
        out.push_str(html.get(pos..lt_pos).unwrap_or(""));
        let after_lt = html.get(lt_pos + 1..).unwrap_or("");
        if !crate::pages_site::is_html_tag_start(after_lt) {
            out.push('<');
            pos = lt_pos + 1;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let close_end =
                crate::pages_site::find_html_comment_end(after_comment).ok_or_else(|| String::from("unclosed HTML comment in index.html"))?;
            pos = lt_pos + 4 + close_end;
            continue;
        }
        let close_rel =
            crate::pages_site::find_html_tag_close(after_lt).ok_or_else(|| String::from("unclosed HTML tag or attribute quote in index.html"))?;
        let tag_body = after_lt.get(..close_rel).ok_or_else(|| String::from("unclosed HTML tag in index.html"))?;
        let next_after_tag = lt_pos + 1 + close_rel + 1;
        let trimmed = tag_body.trim();
        let is_closing = trimmed.starts_with('/');
        let tag_lower = crate::pages_site::extract_html_tag_name(trimmed).to_ascii_lowercase();
        if !is_closing && matches!(tag_lower.as_str(), "script" | "style" | "title" | "textarea") {
            let (_, next_pos) = skip_raw_text_element(html, next_after_tag, &tag_lower)?;
            out.push_str(html.get(lt_pos..next_pos).unwrap_or(""));
            pos = next_pos;
        } else {
            out.push_str(html.get(lt_pos..next_after_tag).unwrap_or(""));
            pos = next_after_tag;
        }
    }
    out.push_str(html.get(pos..).unwrap_or(""));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{
        MAX_WEB_RELEASE_ZIP_BYTES, check_web_bundle_readiness, check_web_bundle_readiness_with_env, parse_manifest, parse_require_web_bundle_env,
        validate_manifest_entries, verify_web_bundle_zip_bytes,
    };
    use crate::scan_util::test_support::TestDir;

    fn write_minimal_web_workspace(root: &TestDir) {
        for (rel, body) in [
            ("crates/text/web-fonts.txt", "Inter/Inter-Regular.ttf\n"),
            ("apps/curvebook-web/Trunk.toml", "[build]\npublic_url = \"./\"\nfilehash = true\ncommand_arguments = [\"xtask\", \"web-fonts\"]\n"),
            (
                "apps/curvebook-web/index.html",
                "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            ),
            ("packaging/web/package.sh", "trunk build --release --cargo-profile web-release\n"),
            ("Cargo.toml", "[profile.web-release]\ninherits = \"release\"\nlto = \"fat\"\nstrip = \"symbols\"\n"),
        ] {
            root.write(rel, body);
        }
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let m = "# comment\n\nInter/Inter-Regular.ttf # body\nJetBrainsMono/JetBrainsMono-Regular.ttf\n";
        let parsed = parse_manifest(m);
        assert_eq!(parsed, vec![String::from("Inter/Inter-Regular.ttf"), String::from("JetBrainsMono/JetBrainsMono-Regular.ttf")]);
        assert!(validate_manifest_entries(&parsed).is_ok());
        assert!(validate_manifest_entries(&[String::from("../escape.ttf")]).is_err());
    }

    #[test]
    fn verifies_web_bundle_zip_budget_bounds() {
        let dir = TestDir::new("webfonts-budget");
        let mut ok_bytes = b"PK\x03\x04".to_vec();
        ok_bytes.resize(4096, 0);
        dir.write_bytes("curvebook-web-ok.zip", &ok_bytes);
        assert_eq!(verify_web_bundle_zip_bytes(&dir.join("curvebook-web-ok.zip")), Ok(4096));

        dir.write_bytes("bad.zip", b"NOTZIP");
        assert!(verify_web_bundle_zip_bytes(&dir.join("bad.zip")).is_err());

        let over_budget = dir.join("over.zip");
        std::fs::File::create(&over_budget).unwrap().set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(verify_web_bundle_zip_bytes(&over_budget).is_err());
    }

    #[test]
    fn require_web_bundle_env_allows_missing_zip_when_unset_and_fails_closed_when_enabled() {
        assert!(!parse_require_web_bundle_env(None));
        for falsy in ["", "0", "false"] {
            assert!(!parse_require_web_bundle_env(Some(OsStr::new(falsy))));
        }
        for truthy in ["1", "true"] {
            assert!(parse_require_web_bundle_env(Some(OsStr::new(truthy))));
        }

        let dir = TestDir::new("webfonts-require");
        write_minimal_web_workspace(&dir);

        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_ok());
        assert!(check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("0"))).is_ok());

        let missing_err = check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("1"))).unwrap_err();
        assert!(missing_err.contains("CURVEBOOK_REQUIRE_WEB_BUNDLE"), "unexpected error: {missing_err}");

        let mut zip_bytes = b"PK\x03\x04".to_vec();
        zip_bytes.resize(2048, 0);
        dir.write_bytes("dist/release/curvebook-web-0.1.0-alpha.2.zip", &zip_bytes);
        assert!(check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("1"))).is_ok());

        let zip_path = dir.join("dist/release/curvebook-web-0.1.0-alpha.2.zip");
        std::fs::OpenOptions::new().write(true).open(&zip_path).unwrap().set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err());
        assert!(check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("1"))).is_err());
    }

    #[test]
    fn workspace_web_bundle_readiness_passes() {
        let root = crate::workspace_root().unwrap();
        assert!(check_web_bundle_readiness_with_env(&root, None).is_ok());
        assert!(check_web_bundle_readiness(&root).is_ok());
    }

    #[test]
    fn fa_sec23_11_13_manifest_guards_comment_stripping_and_script_csp_hashes() {
        for bad in ["C:Inter.ttf", "foo\\bar.ttf", "./Inter.ttf", "Inter//Regular.ttf", "../a.ttf", ""] {
            assert!(validate_manifest_entries(&[String::from(bad)]).is_err(), "expected rejection for {bad}");
        }

        let dir = TestDir::new("webfonts-csp");
        write_minimal_web_workspace(&dir);
        dir.write("apps/curvebook-web/Trunk.toml", "# public_url = \"./\"\n# filehash = true\n# web-fonts\n[build]\n");
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err());

        let html = concat!(
            "<!DOCTYPE html><html><head>",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self'\">",
            "</head><body><script src=\"app.js\"></script><script>console.log(1);</script><script type=\"module\">console.log(2);</script></body></html>"
        );
        let (rewritten, hashes) = super::rewrite_built_index_csp_hashes(html).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(!rewritten.contains("'unsafe-inline'"));
        for h in &hashes {
            assert!(h.starts_with("'sha256-") && h.ends_with('\'') && rewritten.contains(h));
        }
    }

    #[test]
    fn aud_12_10_csp_parsing_case_sensitivity_script_src_attr_comments_and_script_close() {
        // 1. Source index.html style-src 'unsafe-inline' bypasses must all be rejected.
        for bad_policy in [
            "default-src 'self'; style-src 'self'  'unsafe-inline'",
            "default-src 'self'; style-src 'self' data: 'unsafe-inline'",
            "default-src 'self'; style-src 'self' 'UNSAFE-INLINE'",
            "default-src 'self'; style-src 'unsafe-inline' 'self'; style-src 'self'",
            "default-src 'self' 'UNSAFE-INLINE'; style-src 'self'",
        ] {
            let dir = TestDir::new("webfonts-style-bypass");
            write_minimal_web_workspace(&dir);
            dir.write(
                "apps/curvebook-web/index.html",
                &format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{bad_policy}\"><link data-wasm-opt=\"z\">"),
            );
            assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err(), "expected rejection for `{bad_policy}`");
        }

        // 2. script-src-attr / script-src-elem must NOT be corrupted; missing script-src and uppercase 'UNSAFE-INLINE' fail.
        let with_attr = concat!(
            "<html><head>",
            "<!-- <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\"> -->",
            "<meta name=\"description\" content=\"Uses Content-Security-Policy\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; script-src-attr 'none'; style-src 'self'\">",
            "</head><body>",
            "<!-- <script>evilCommented();</script> -->",
            "<script>const s = \"</script_tag\"; console.log(s);</script>",
            "</body></html>"
        );
        let (rewritten, hashes) = super::rewrite_built_index_csp_hashes(with_attr).unwrap();
        assert_eq!(hashes.len(), 1, "commented script must not be hashed; embedded </script_tag must not split script");
        assert!(rewritten.contains("script-src-attr 'none'"), "script-src-attr must be preserved: {rewritten}");
        assert!(!rewritten.to_ascii_lowercase().contains("'unsafe-inline'"));

        // Self-closing <script/> is NOT void in HTML5; its body must still be hashed.
        let slash_script = concat!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self'\">",
            "<script/>alert(1)</script>"
        );
        let (_, slash_hashes) = super::rewrite_built_index_csp_hashes(slash_script).unwrap();
        assert_eq!(slash_hashes.len(), 1);

        // Missing script-src fails closed.
        let no_script_src = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\">";
        assert!(super::rewrite_built_index_csp_hashes(no_script_src).unwrap_err().contains("script-src"));

        // Uppercase 'UNSAFE-INLINE' in another directive fails closed.
        let upper_unsafe =
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self'; style-src 'self' 'UNSAFE-INLINE'\">";
        assert!(super::rewrite_built_index_csp_hashes(upper_unsafe).is_err());

        // 3. Entity-encoded CSP directives/http-equiv, quote-desync, attribute-wrapped `<!--`, and abrupt/bang comments.
        for bad_html in [
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self' &#39;unsafe-inline&#39;\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self' &apos;unsafe-inline&apos;\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"&#x43;ontent-Security-Policy\" content=\"default-src *\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><meta http-equiv=\"&#x43;ontent-Security-Policy\" content=\"default-src *\"><link data-wasm-opt=\"z\">",
            "<div title=\"<!--\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><span title=\"-->\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            "<p title=x\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><b z=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            "<!--><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><!-- --><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            "<!-- c --!><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><!-- --><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
        ] {
            let dir = TestDir::new("webfonts-entity-bypass");
            write_minimal_web_workspace(&dir);
            dir.write("apps/curvebook-web/index.html", bad_html);
            assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err(), "source check must reject `{bad_html}`");
            assert!(super::rewrite_built_index_csp_hashes(bad_html).is_err(), "rewrite check must reject `{bad_html}`");
        }

        // Entity-encoded `'unsafe-inline'` in style-src must also fail rewrite when script-src is present.
        let entity_style_unsafe = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' &#39;unsafe-inline&#39;\">";
        assert!(super::rewrite_built_index_csp_hashes(entity_style_unsafe).is_err());

        // Quote-desync / attribute-wrapped `<!--` / abrupt comment / non-tag `<` / RCDATA `<title>` / form-feed close must not hide scripts.
        let hidden_scripts = concat!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self'\">",
            "<p title=x\"><script>a();</script><b z=1\">",
            "<div title=\"<!--\"><script>b();</script><span title=\"-->\">",
            "<!--><script>c();</script><!-- -->",
            "<!-- x --!><script>d();</script><!-- -->",
            "< a=\"><script>e();</script>",
            "<<script>f();</script\x0c>",
            "<title><!--</title><script>g();</script><!-- -->"
        );
        let (_, hidden_hashes) = super::rewrite_built_index_csp_hashes(hidden_scripts).unwrap();
        assert_eq!(hidden_hashes.len(), 7, "all seven inline scripts must be hashed");

        // Leading `=` in `<meta =" http-equiv="Content-Security-Policy" ...>` must NOT hide a duplicate/permissive CSP meta tag.
        let leading_eq_csp = "<meta =\" http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self'; style-src 'self'\">";
        assert!(super::rewrite_built_index_csp_hashes(leading_eq_csp).is_err());
    }

    #[test]
    fn aud_12_43_web_fonts_rejects_symlinks_and_scopes_cargo_profile() {
        let dir = TestDir::new("webfonts-profile-scope");
        write_minimal_web_workspace(&dir);
        // Put lto = "fat" and strip = "symbols" under [profile.release] instead of [profile.web-release].
        dir.write("Cargo.toml", "[profile.release]\nlto = \"fat\"\nstrip = \"symbols\"\n\n[profile.web-release]\ninherits = \"release\"\n");
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err());

        #[cfg(unix)]
        {
            let font_dir = TestDir::new("webfonts-symlink");
            let secret = font_dir.write("outside.ttf", "secret");
            let src_fonts = font_dir.join("src/fonts");
            let staging = font_dir.join("staging/fonts");
            std::fs::create_dir_all(&src_fonts).unwrap();
            std::os::unix::fs::symlink(&secret, src_fonts.join("Inter.ttf")).unwrap();
            assert!(super::stage_font_files(&src_fonts, &staging, &[String::from("Inter.ttf")]).is_err());

            let clean_src = font_dir.join("clean/fonts");
            std::fs::create_dir_all(&clean_src).unwrap();
            std::fs::write(clean_src.join("Inter.ttf"), b"font-bytes").unwrap();
            std::fs::create_dir_all(&staging).unwrap();
            std::os::unix::fs::symlink(&secret, staging.join("Inter.ttf")).unwrap();
            assert!(super::stage_font_files(&clean_src, &staging, &[String::from("Inter.ttf")]).is_err());
        }
    }
}
