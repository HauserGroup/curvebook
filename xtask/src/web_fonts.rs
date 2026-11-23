//! `cargo xtask web-fonts`: stage font faces listed in `crates/text/web-fonts.txt` from
//! `$CURVEBOOK_FONTS_DIR/fonts/` into the Trunk staging directory (`$TRUNK_STAGING_DIR/fonts` or
//! `dist/web/fonts`). No-op when `CURVEBOOK_FONTS_DIR` is unset.
//!
//! Also provides `R10.1` / `R10.9` / `R10.22` web release bundle budget and packaging readiness
//! validation (`<= 6_243_591` B compressed `curvebook-web-*.zip`).

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use curvebook_platform::write_atomic;

const MANIFEST_REL: &str = "crates/text/web-fonts.txt";
const TRUNK_TOML_REL: &str = "apps/curvebook-web/Trunk.toml";
const WEB_INDEX_REL: &str = "apps/curvebook-web/index.html";
const PACKAGE_SH_REL: &str = "packaging/web/package.sh";
const MAX_BUILT_INDEX_BYTES: usize = 8 * 1024 * 1024;

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
    let built_index = staging_root.join("index.html");
    if std::fs::symlink_metadata(&built_index).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
        let hashes = rewrite_built_index_csp_file(&built_index)?;
        println!("web-fonts: stamped {} SHA-256 script CSP hash(es) into {}", hashes.len(), built_index.display());
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
    std::fs::create_dir_all(&staging).map_err(|e| format!("mkdir {}: {e}", staging.display()))?;

    let src_base = PathBuf::from(fonts_root).join("fonts");
    let mut copied = 0usize;
    for rel in &faces {
        let src = src_base.join(rel);
        let dst = staging.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
        copied += 1;
    }
    println!("web-fonts: staged {copied} font face(s) into {}", staging.display());
    Ok(())
}

/// Compute exact CSP `'sha256-...'` hashes over every inline `<script>` block in built `html`
/// and rewrite `<meta http-equiv="Content-Security-Policy" ...>` removing `'unsafe-inline'`.
pub fn rewrite_built_index_csp_hashes(html: &str) -> Result<(String, Vec<String>), String> {
    let hashes = extract_inline_script_hashes(html)?;
    let rewritten = rewrite_csp_meta_tag(html, &hashes)?;
    if rewritten.contains("'unsafe-inline'") {
        return Err(String::from("built index.html still contains `'unsafe-inline'` after CSP hash rewrite"));
    }
    Ok((rewritten, hashes))
}

fn rewrite_built_index_csp_file(index_path: &Path) -> Result<Vec<String>, String> {
    let bytes =
        curvebook_platform::read_regular_file_bytes(index_path, MAX_BUILT_INDEX_BYTES).map_err(|e| format!("read {}: {e}", index_path.display()))?;
    let html = String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", index_path.display()))?;
    let (rewritten, hashes) = rewrite_built_index_csp_hashes(&html)?;
    if rewritten != html {
        write_atomic(index_path, rewritten.as_bytes()).map_err(|e| format!("write {}: {e}", index_path.display()))?;
    }
    Ok(hashes)
}

fn extract_inline_script_hashes(html: &str) -> Result<Vec<String>, String> {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut hashes = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = lower[pos..].find("<script") {
        let tag_start = pos + rel;
        let after_name = tag_start + "<script".len();
        let next_b = bytes.get(after_name).copied();
        if !matches!(next_b, Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n')) {
            pos = after_name;
            continue;
        }
        let close_rel = find_html_tag_close(&html[after_name..]).ok_or_else(|| String::from("unclosed `<script>` opening tag"))?;
        let open_end = after_name + close_rel;
        let attrs = &html[after_name..open_end];
        let body_start = open_end + 1;
        if attrs.trim_end().ends_with('/') {
            pos = body_start;
            continue;
        }
        let close_tag_rel = lower[body_start..].find("</script").ok_or_else(|| String::from("unclosed `<script>` element"))?;
        let body_end = body_start + close_tag_rel;
        let close_tag_gt = find_html_tag_close(&html[body_end..]).ok_or_else(|| String::from("unclosed `</script>` closing tag"))?;
        if !tag_has_src_attr(attrs) {
            let body = &html[body_start..body_end];
            let digest = crate::release_scan::sha256_bytes(body.as_bytes());
            let token = format!("'sha256-{}'", encode_base64_std(&digest));
            if !hashes.contains(&token) {
                hashes.push(token);
            }
        }
        pos = body_end + close_tag_gt + 1;
    }
    Ok(hashes)
}

fn find_html_tag_close(slice: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (idx, b) in slice.bytes().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(idx);
        }
    }
    None
}

fn tag_has_src_attr(attrs: &str) -> bool {
    let bytes = attrs.as_bytes();
    let lower = attrs.to_ascii_lowercase();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let k_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let key = &lower[k_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let has_eq = i < bytes.len() && bytes[i] == b'=';
        if key == "src" && has_eq {
            return true;
        }
        if has_eq {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
        }
    }
    false
}

fn rewrite_csp_meta_tag(html: &str, script_hashes: &[String]) -> Result<String, String> {
    let lower = html.to_ascii_lowercase();
    let mut pos = 0usize;
    while let Some(rel) = lower[pos..].find("<meta") {
        let tag_start = pos + rel;
        let after_name = tag_start + "<meta".len();
        if !matches!(lower.as_bytes().get(after_name), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            pos = after_name;
            continue;
        }
        let close_rel = find_html_tag_close(&html[after_name..]).ok_or_else(|| String::from("unclosed `<meta>` tag"))?;
        let tag_end = after_name + close_rel;
        let tag = &html[tag_start..=tag_end];
        if !tag.to_ascii_lowercase().contains("content-security-policy") {
            pos = tag_end + 1;
            continue;
        }
        let Some((val_start, val_end)) = find_meta_content_span(tag) else {
            return Err(String::from("Content-Security-Policy `<meta>` lacks `content=\"...\"`"));
        };
        let old_policy = &tag[val_start..val_end];
        let mut rebuilt = Vec::new();
        for dir in old_policy.split(';') {
            let trimmed = dir.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.to_ascii_lowercase().starts_with("script-src") {
                let mut parts = vec![String::from("script-src"), String::from("'self'")];
                parts.extend(script_hashes.iter().cloned());
                parts.push(String::from("'wasm-unsafe-eval'"));
                rebuilt.push(parts.join(" "));
            } else {
                rebuilt.push(trimmed.to_owned());
            }
        }
        let new_policy = rebuilt.join("; ");
        let mut out = String::with_capacity(html.len() + 256);
        out.push_str(&html[..tag_start + val_start]);
        out.push_str(&new_policy);
        out.push_str(&html[tag_start + val_end..]);
        return Ok(out);
    }
    Err(String::from("built index.html missing `<meta http-equiv=\"Content-Security-Policy\">`"))
}

fn find_meta_content_span(tag: &str) -> Option<(usize, usize)> {
    let lower = tag.to_ascii_lowercase();
    let bytes = tag.as_bytes();
    let mut i = 5usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/' || bytes[i] == b'>') {
            i += 1;
        }
        let k_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' && bytes[i] != b'>' {
            i += 1;
        }
        let key = &lower[k_start..i];
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
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let q = bytes[i];
            i += 1;
            let v_start = i;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            let v_end = i;
            if i < bytes.len() {
                i += 1;
            }
            if key == "content" {
                return Some((v_start, v_end));
            }
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
        }
    }
    None
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
    let index_html = strip_html_comments(&index_html_raw)?;
    for required in ["default-src 'self'", "data-wasm-opt", "style-src 'self'"] {
        if !index_html.contains(required) {
            return Err(format!("{WEB_INDEX_REL} missing required attribute `{required}`"));
        }
    }
    if index_html.contains("style-src 'self' 'unsafe-inline'") {
        return Err(format!("{WEB_INDEX_REL} must not allow `'unsafe-inline'` in `style-src`"));
    }

    let pkg_sh_raw = std::fs::read_to_string(root.join(PACKAGE_SH_REL)).map_err(|e| format!("read {PACKAGE_SH_REL}: {e}"))?;
    let pkg_sh = strip_hash_comments(&pkg_sh_raw);
    if !pkg_sh.contains("--cargo-profile web-release") {
        return Err(format!("{PACKAGE_SH_REL} missing `--cargo-profile web-release`"));
    }

    let cargo_toml_raw = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("read Cargo.toml: {e}"))?;
    let cargo_toml = strip_hash_comments(&cargo_toml_raw);
    for required in ["[profile.web-release]", "lto = \"fat\"", "strip = \"symbols\""] {
        if !cargo_toml.contains(required) {
            return Err(format!("Cargo.toml missing `{required}` for web release size budget"));
        }
    }

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
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

fn strip_html_comments(html: &str) -> Result<String, String> {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        let end = after.find("-->").ok_or_else(|| String::from("unclosed HTML comment in index.html"))?;
        rest = &after[end + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_WEB_RELEASE_ZIP_BYTES, check_web_bundle_readiness, check_web_bundle_readiness_with_env, parse_manifest, parse_require_web_bundle_env,
        validate_manifest_entries, verify_web_bundle_zip_bytes,
    };
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    fn temp_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-xtask-webfonts-{label}-{}-{nanos}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_minimal_web_workspace(root: &Path) {
        std::fs::create_dir_all(root.join("crates/text")).unwrap();
        std::fs::create_dir_all(root.join("apps/curvebook-web")).unwrap();
        std::fs::create_dir_all(root.join("packaging/web")).unwrap();
        std::fs::write(root.join("crates/text/web-fonts.txt"), "Inter/Inter-Regular.ttf\n").unwrap();
        std::fs::write(
            root.join("apps/curvebook-web/Trunk.toml"),
            "[build]\npublic_url = \"./\"\nfilehash = true\ncommand_arguments = [\"xtask\", \"web-fonts\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("apps/curvebook-web/index.html"),
            "<meta content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
        )
        .unwrap();
        std::fs::write(root.join("packaging/web/package.sh"), "trunk build --release --cargo-profile web-release\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[profile.web-release]\ninherits = \"release\"\nlto = \"fat\"\nstrip = \"symbols\"\n").unwrap();
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
        let dir = temp_test_dir("budget");
        let ok_zip = dir.join("curvebook-web-ok.zip");
        let mut ok_bytes = b"PK\x03\x04".to_vec();
        ok_bytes.resize(4096, 0);
        std::fs::write(&ok_zip, &ok_bytes).unwrap();
        assert_eq!(verify_web_bundle_zip_bytes(&ok_zip), Ok(4096));

        let bad_magic = dir.join("bad.zip");
        std::fs::write(&bad_magic, b"NOTZIP").unwrap();
        assert!(verify_web_bundle_zip_bytes(&bad_magic).is_err());

        let over_budget = dir.join("over.zip");
        let file = std::fs::File::create(&over_budget).unwrap();
        file.set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(verify_web_bundle_zip_bytes(&over_budget).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn require_web_bundle_env_allows_missing_zip_when_unset_and_fails_closed_when_enabled() {
        assert!(!parse_require_web_bundle_env(None));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new(""))));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new("0"))));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new("false"))));
        assert!(parse_require_web_bundle_env(Some(OsStr::new("1"))));
        assert!(parse_require_web_bundle_env(Some(OsStr::new("true"))));

        let dir = temp_test_dir("require-bundle");
        write_minimal_web_workspace(&dir);

        // Unset / falsy skips missing zip cleanly after checking static readiness.
        assert!(check_web_bundle_readiness_with_env(&dir, None).is_ok());
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("0"))).is_ok());

        // CURVEBOOK_REQUIRE_WEB_BUNDLE=1 fails closed when dist/release/ has no web zip.
        let missing_err = check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).unwrap_err();
        assert!(missing_err.contains("CURVEBOOK_REQUIRE_WEB_BUNDLE"), "unexpected error: {missing_err}");

        // Once a valid in-budget zip exists in dist/release/, CURVEBOOK_REQUIRE_WEB_BUNDLE=1 passes.
        let rel_dir = dir.join("dist/release");
        std::fs::create_dir_all(&rel_dir).unwrap();
        let zip_path = rel_dir.join("curvebook-web-0.1.0-alpha.2.zip");
        let mut zip_bytes = b"PK\x03\x04".to_vec();
        zip_bytes.resize(2048, 0);
        std::fs::write(&zip_path, &zip_bytes).unwrap();
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).is_ok());

        // Over-budget zip fails closed in both unset and CURVEBOOK_REQUIRE_WEB_BUNDLE=1 modes.
        let file = std::fs::OpenOptions::new().write(true).open(&zip_path).unwrap();
        file.set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(check_web_bundle_readiness_with_env(&dir, None).is_err());
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).is_err());

        let _ = std::fs::remove_dir_all(&dir);
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

        let dir = temp_test_dir("comments-csp");
        write_minimal_web_workspace(&dir);
        std::fs::write(dir.join("apps/curvebook-web/Trunk.toml"), "# public_url = \"./\"\n# filehash = true\n# web-fonts\n[build]\n").unwrap();
        assert!(check_web_bundle_readiness_with_env(&dir, None).is_err());

        let html = concat!(
            "<!DOCTYPE html><html><head>",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self'\">",
            "</head><body><script src=\"app.js\"></script><script>console.log(1);</script><script type=\"module\">console.log(2);</script></body></html>"
        );
        let (rewritten, hashes) = super::rewrite_built_index_csp_hashes(html).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(!rewritten.contains("'unsafe-inline'"));
        for h in &hashes {
            assert!(h.starts_with("'sha256-") && h.ends_with('\''));
            assert!(rewritten.contains(h));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
