//! `cargo xtask web-fonts`: stage font faces listed in `crates/text/web-fonts.txt`, stamp exact
//! SHA-256 CSP script hashes into built `index.html`, and validate `R10.1`/`R10.9`/`R10.22` web
//! release bundle budget and packaging readiness (`<= 6_243_591` B compressed).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use curvebook_platform::{read_regular_file_bytes, write_atomic};

use crate::scan_util::{
    extract_html_tag_name, find_html_comment_end, find_html_tag_close, is_html_tag_start, parse_html_tag_attr_spans, parse_html_tag_attributes,
    read_dir_paths_sorted, skip_html_raw_or_rcdata_element, strip_toml_comment, unquote_scalar,
};
use crate::text_decode::unescape_html_attr;

const MANIFEST_REL: &str = "crates/text/web-fonts.txt";
const TRUNK_TOML_REL: &str = "apps/curvebook-web/Trunk.toml";
const WEB_INDEX_REL: &str = "apps/curvebook-web/index.html";
const PACKAGE_SH_REL: &str = "packaging/web/package.sh";
const MAX_BUILT_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_WEB_FONT_BYTES: usize = 32 * 1024 * 1024;

/// `R10.22` web release zip budget (`1.5 * 4,162,394 B` pre-`R0.20` baseline).
pub const MAX_WEB_RELEASE_ZIP_BYTES: u64 = 6_243_591;

pub fn run(root: &Path) -> Result<(), String> {
    let staging_root = std::env::var_os("TRUNK_STAGING_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map_or_else(|| root.join("dist/web"), |p| if p.is_absolute() { p } else { root.join(p) });
    if std::fs::symlink_metadata(&staging_root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("staging directory {} must not be a symlink", staging_root.display()));
    }
    let built_index = staging_root.join("index.html");
    if let Ok(meta) = std::fs::symlink_metadata(&built_index) {
        if meta.file_type().is_symlink() {
            return Err(format!("built index {} must not be a symlink", built_index.display()));
        }
        if meta.file_type().is_file() {
            let bytes = read_regular_file_bytes(&built_index, MAX_BUILT_INDEX_BYTES).map_err(|e| format!("read {}: {e}", built_index.display()))?;
            let html = String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", built_index.display()))?;
            let (rewritten, hashes) = rewrite_built_index_csp_hashes(&html)?;
            if rewritten != html {
                write_atomic(&built_index, rewritten.as_bytes()).map_err(|e| format!("write {}: {e}", built_index.display()))?;
            }
            println!("web-fonts: stamped {} SHA-256 script CSP hash(es) into {}", hashes.len(), built_index.display());
        }
    }
    let Some(fonts_root) = std::env::var_os("CURVEBOOK_FONTS_DIR").filter(|v| !v.is_empty()) else {
        println!("web-fonts: CURVEBOOK_FONTS_DIR not set; skipping optional font staging");
        return Ok(());
    };
    let faces = parse_manifest(&std::fs::read_to_string(root.join(MANIFEST_REL)).map_err(|e| format!("{MANIFEST_REL}: {e}"))?);
    validate_manifest_entries(&faces)?;
    let staging = staging_root.join("fonts");
    let copied = stage_font_files(&PathBuf::from(fonts_root).join("fonts"), &staging, &faces)?;
    println!("web-fonts: staged {copied} font face(s) into {}", staging.display());
    Ok(())
}

fn reject_symlink_rel_components(base: &Path, rel: &str) -> Result<(), String> {
    let mut cur = base.to_path_buf();
    for seg in rel.split('/') {
        cur.push(seg);
        if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{}: symlink forbidden during font staging", cur.display()));
        }
    }
    Ok(())
}

fn stage_font_files(src_base: &Path, staging: &Path, faces: &[String]) -> Result<usize, String> {
    for (dir, label) in [(src_base, "font source"), (staging, "font staging")] {
        if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink() || !m.file_type().is_dir()) {
            return Err(format!("{label} directory {} must be a regular non-symlink directory", dir.display()));
        }
    }
    std::fs::create_dir_all(staging).map_err(|e| format!("mkdir {}: {e}", staging.display()))?;
    for rel in faces {
        reject_symlink_rel_components(src_base, rel)?;
        reject_symlink_rel_components(staging, rel)?;
        let (src, dst) = (src_base.join(rel), staging.join(rel));
        let src_meta = std::fs::symlink_metadata(&src).map_err(|e| format!("stat {}: {e}", src.display()))?;
        if src_meta.file_type().is_symlink() || !src_meta.file_type().is_file() {
            return Err(format!("{}: font source must be a regular non-symlink file", src.display()));
        }
        let bytes = read_regular_file_bytes(&src, MAX_WEB_FONT_BYTES).map_err(|e| format!("read {}: {e}", src.display()))?;
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        if std::fs::symlink_metadata(&dst).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{}: font destination must not be a symlink", dst.display()));
        }
        write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    Ok(faces.len())
}

struct CspMetaAttr {
    tok_start: usize,
    tok_end: usize,
    policy: String,
}

struct IndexScan {
    script_hashes: Vec<String>,
    csp_meta: Option<CspMetaAttr>,
    comment_stripped: String,
}

fn scan_index_html(html: &str) -> Result<IndexScan, String> {
    let (mut hashes, mut csp_meta, mut stripped, mut pos) = (Vec::new(), None, String::with_capacity(html.len()), 0usize);
    while let Some(rel_lt) = html.get(pos..).and_then(|s| s.find('<')) {
        let lt = pos + rel_lt;
        stripped.push_str(html.get(pos..lt).unwrap_or(""));
        let after_lt = html.get(lt + 1..).unwrap_or("");
        if !is_html_tag_start(after_lt) {
            stripped.push('<');
            pos = lt + 1;
            continue;
        }
        if let Some(after_c) = after_lt.strip_prefix("!--") {
            pos = lt + 4 + find_html_comment_end(after_c).ok_or("unclosed HTML comment in index.html")?;
            continue;
        }
        let close_rel = find_html_tag_close(after_lt).ok_or("unclosed HTML tag or attribute quote in index.html")?;
        let tag_body = after_lt.get(..close_rel).ok_or("unclosed HTML tag in index.html")?;
        let (after_tag, trimmed) = (lt + 2 + close_rel, tag_body.trim());
        let (is_closing, tag_lower) = (trimmed.starts_with('/'), extract_html_tag_name(trimmed).to_ascii_lowercase());
        if !is_closing && matches!(tag_lower.as_str(), "script" | "style" | "title" | "textarea") {
            let (body_end, next_pos) = skip_html_raw_or_rcdata_element(html, after_tag, &tag_lower).map_err(|e| format!("{e} in index.html"))?;
            if tag_lower == "script" && !parse_html_tag_attributes(trimmed).iter().any(|(k, _)| k.eq_ignore_ascii_case("src")) {
                let body = html.get(after_tag..body_end).ok_or("unclosed `<script>` element")?;
                let tok = format!("'sha256-{}'", encode_base64_std(&crate::release_scan::sha256_bytes(body.as_bytes())));
                if !hashes.contains(&tok) {
                    hashes.push(tok);
                }
            }
            stripped.push_str(html.get(lt..next_pos).unwrap_or(""));
            pos = next_pos;
            continue;
        }
        if !is_closing
            && tag_lower == "meta"
            && let Some(m) = parse_meta_csp_attr(tag_body)?
            && csp_meta.replace(CspMetaAttr { tok_start: lt + 1 + m.tok_start, tok_end: lt + 1 + m.tok_end, policy: m.policy }).is_some()
        {
            return Err(String::from("multiple active Content-Security-Policy `<meta>` tags found"));
        }
        stripped.push_str(html.get(lt..after_tag).unwrap_or(""));
        pos = after_tag;
    }
    stripped.push_str(html.get(pos..).unwrap_or(""));
    Ok(IndexScan { script_hashes: hashes, csp_meta, comment_stripped: stripped })
}

/// Compute exact CSP `'sha256-...'` hashes over every inline `<script>` block in built `html`
/// and rewrite `<meta http-equiv="Content-Security-Policy" ...>` removing `'unsafe-inline'`.
pub fn rewrite_built_index_csp_hashes(html: &str) -> Result<(String, Vec<String>), String> {
    let scan = scan_index_html(html)?;
    let csp = scan.csp_meta.ok_or("built index.html missing `<meta http-equiv=\"Content-Security-Policy\">`")?;
    let (mut saw_script_src, mut rebuilt) = (false, Vec::new());
    for (name, tokens) in parse_csp_directives(&csp.policy)? {
        if name == "script-src" {
            saw_script_src = true;
            let mut parts = vec![String::from("script-src"), String::from("'self'")];
            parts.extend(scan.script_hashes.iter().cloned());
            parts.push(String::from("'wasm-unsafe-eval'"));
            rebuilt.push(parts.join(" "));
        } else {
            if tokens.iter().any(|t| t.eq_ignore_ascii_case("'unsafe-inline'")) {
                return Err(format!("CSP directive `{name}` must not allow `'unsafe-inline'`"));
            }
            rebuilt.push(if tokens.is_empty() { name } else { format!("{name} {}", tokens.join(" ")) });
        }
    }
    if !saw_script_src {
        return Err(String::from("Content-Security-Policy `<meta>` is missing a `script-src` directive"));
    }
    let rewritten = format!(
        "{}\"{}\"{}",
        html.get(..csp.tok_start).ok_or("invalid CSP meta tag boundary")?,
        rebuilt.join("; "),
        html.get(csp.tok_end..).ok_or("invalid CSP meta tag boundary")?
    );
    let active = scan_index_html(&rewritten)?.comment_stripped;
    if active.to_ascii_lowercase().contains("'unsafe-inline'") || unescape_html_attr(&active).to_ascii_lowercase().contains("'unsafe-inline'") {
        return Err(String::from("built index.html still contains `'unsafe-inline'` after CSP hash rewrite"));
    }
    Ok((rewritten, scan.script_hashes))
}

fn parse_csp_directives(policy: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let (mut out, mut seen) = (Vec::new(), BTreeSet::new());
    for raw in policy.split(';') {
        let mut parts = raw.split_whitespace();
        let Some(name_raw) = parts.next() else { continue };
        let name = name_raw.to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate CSP directive `{name}`"));
        }
        out.push((name, parts.map(str::to_owned).collect()));
    }
    Ok(out)
}

fn parse_meta_csp_attr(tag_body: &str) -> Result<Option<CspMetaAttr>, String> {
    let (mut is_csp, mut he_n, mut ct_n, mut content) = (false, 0usize, 0usize, None);
    for attr in parse_html_tag_attr_spans(tag_body) {
        let Some((ts, te)) = attr.value_span else { continue };
        if attr.key.eq_ignore_ascii_case("http-equiv") {
            he_n += 1;
            is_csp |= attr.value.trim().eq_ignore_ascii_case("content-security-policy");
        } else if attr.key.eq_ignore_ascii_case("content") {
            ct_n += 1;
            content.get_or_insert(CspMetaAttr { tok_start: ts, tok_end: te, policy: attr.value });
        }
    }
    if !is_csp {
        return Ok(None);
    }
    if he_n > 1 || ct_n > 1 {
        return Err(String::from("duplicate `http-equiv` or `content` attribute on Content-Security-Policy `<meta>`"));
    }
    content.ok_or_else(|| String::from("Content-Security-Policy `<meta>` lacks `content=\"...\"`")).map(Some)
}

fn encode_base64_std(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let t = (u32::from(c[0]) << 16) | (u32::from(c.get(1).copied().unwrap_or(0)) << 8) | u32::from(c.get(2).copied().unwrap_or(0));
        out.push(char::from(TABLE[((t >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((t >> 12) & 0x3f) as usize]));
        out.push(if c.len() > 1 { char::from(TABLE[((t >> 6) & 0x3f) as usize]) } else { '=' });
        out.push(if c.len() > 2 { char::from(TABLE[(t & 0x3f) as usize]) } else { '=' });
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
            || rel.split('/').any(|s| s.is_empty() || s == "." || s == "..")
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
    if size == 0 || size > MAX_WEB_RELEASE_ZIP_BYTES {
        return Err(format!("web release bundle {} invalid size {size} B (budget: 1..={MAX_WEB_RELEASE_ZIP_BYTES} B)", zip_path.display()));
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(zip_path)
        .map_err(|e| format!("open {}: {e}", zip_path.display()))?
        .read_exact(&mut magic)
        .map_err(|e| format!("read header {}: {e}", zip_path.display()))?;
    if &magic != b"PK\x03\x04" {
        return Err(format!("{} does not start with ZIP local file signature", zip_path.display()));
    }
    Ok(size)
}

#[must_use]
pub fn parse_require_web_bundle_env(raw: Option<&std::ffi::OsStr>) -> bool {
    crate::workflows::parse_ci_env_value(raw)
}

pub fn run_bundle_check(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask web-bundle (unexpected args: `{}`)", args.join(" ")));
    }
    check_web_bundle_readiness(root)
}

pub fn check_web_bundle_readiness(root: &Path) -> Result<(), String> {
    check_web_bundle_readiness_with_env(root, std::env::var_os("CURVEBOOK_REQUIRE_WEB_BUNDLE").as_deref())
}

pub fn check_web_bundle_readiness_with_env(root: &Path, require_bundle_env: Option<&std::ffi::OsStr>) -> Result<(), String> {
    let read_rel = |rel: &str| std::fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"));
    let strip_c = |s: &str| s.lines().map(strip_toml_comment).collect::<Vec<_>>().join("\n");
    let faces = parse_manifest(&read_rel(MANIFEST_REL)?);
    validate_manifest_entries(&faces)?;
    let trunk_toml = strip_c(&read_rel(TRUNK_TOML_REL)?);
    for required in ["public_url = \"./\"", "filehash = true", "web-fonts"] {
        if !trunk_toml.contains(required) {
            return Err(format!("{TRUNK_TOML_REL} missing required setting `{required}`"));
        }
    }
    validate_source_index_html(&read_rel(WEB_INDEX_REL)?)?;
    if !strip_c(&read_rel(PACKAGE_SH_REL)?).contains("--cargo-profile web-release") {
        return Err(format!("{PACKAGE_SH_REL} missing `--cargo-profile web-release`"));
    }
    check_cargo_web_release_profile(&read_rel("Cargo.toml")?)?;

    let (release_dir, mut checked_zips) = (root.join("dist/release"), 0usize);
    if std::fs::symlink_metadata(&release_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
        for zip_path in read_dir_paths_sorted(&release_dir)? {
            if zip_path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("curvebook-web-") && n.ends_with(".zip")) {
                let size = verify_web_bundle_zip_bytes(&zip_path)?;
                println!("web-bundle: {} = {size} B (budget <= {MAX_WEB_RELEASE_ZIP_BYTES} B)", zip_path.display());
                checked_zips += 1;
            }
        }
    }
    if checked_zips == 0 {
        if parse_require_web_bundle_env(require_bundle_env) {
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
    let scan = scan_index_html(index_html_raw).map_err(|e| format!("{WEB_INDEX_REL}: {e}"))?;
    if !scan.comment_stripped.contains("data-wasm-opt") {
        return Err(format!("{WEB_INDEX_REL} missing required attribute `data-wasm-opt`"));
    }
    let csp = scan.csp_meta.ok_or_else(|| format!("{WEB_INDEX_REL}: missing Content-Security-Policy `<meta>`"))?;
    let dirs = parse_csp_directives(&csp.policy).map_err(|e| format!("{WEB_INDEX_REL}: {e}"))?;
    let find_dir = |name: &str| dirs.iter().find(|(n, _)| n == name).map(|(_, t)| t);
    for req in ["default-src", "style-src"] {
        if !find_dir(req).is_some_and(|t| t.iter().any(|x| x.eq_ignore_ascii_case("'self'"))) {
            return Err(format!("{WEB_INDEX_REL}: `{req}` must include `'self'`"));
        }
    }
    for no_unsafe in ["default-src", "style-src", "style-src-elem", "style-src-attr"] {
        if find_dir(no_unsafe).is_some_and(|t| t.iter().any(|x| x.eq_ignore_ascii_case("'unsafe-inline'"))) {
            return Err(format!("{WEB_INDEX_REL} must not allow `'unsafe-inline'` in `{no_unsafe}`"));
        }
    }
    Ok(())
}

fn check_cargo_web_release_profile(cargo_toml_raw: &str) -> Result<(), String> {
    let (mut in_sec, mut saw_sec, mut lto_fat, mut strip_sym) = (false, false, false, false);
    for raw in cargo_toml_raw.lines() {
        let line = strip_toml_comment(raw).trim();
        if let Some(hdr) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_sec = hdr.trim() == "profile.web-release";
            saw_sec |= in_sec;
        } else if in_sec && let Some((k, v)) = line.split_once('=') {
            let val = unquote_scalar(v);
            lto_fat |= k.trim() == "lto" && val == "fat";
            strip_sym |= k.trim() == "strip" && val == "symbols";
        }
    }
    if !saw_sec || !lto_fat || !strip_sym {
        return Err(String::from("Cargo.toml `[profile.web-release]` must define `lto = \"fat\"` and `strip = \"symbols\"`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn manifest_zip_budget_csp_and_symlink_guards() {
        let parsed = parse_manifest("# c\n\nInter/Inter-Regular.ttf # body\nJetBrainsMono/Mono.ttf\n");
        assert_eq!(parsed, vec!["Inter/Inter-Regular.ttf", "JetBrainsMono/Mono.ttf"]);
        assert!(validate_manifest_entries(&parsed).is_ok());
        for bad in ["C:Inter.ttf", "foo\\bar.ttf", "./Inter.ttf", "Inter//Regular.ttf", "../a.ttf", ""] {
            assert!(validate_manifest_entries(&[String::from(bad)]).is_err(), "{bad}");
        }
        let dir = TestDir::new("webfonts-budget");
        for (rel, body) in [
            (MANIFEST_REL, "Inter/Inter-Regular.ttf\n"),
            (TRUNK_TOML_REL, "[build]\npublic_url = \"./\"\nfilehash = true\ncommand_arguments = [\"xtask\", \"web-fonts\"]\n"),
            (
                WEB_INDEX_REL,
                "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            ),
            (PACKAGE_SH_REL, "trunk build --release --cargo-profile web-release\n"),
            ("Cargo.toml", "[profile.web-release]\ninherits = \"release\"\nlto = \"fat\"\nstrip = \"symbols\"\n"),
        ] {
            dir.write(rel, body);
        }
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_ok());
        assert!(!parse_require_web_bundle_env(Some(OsStr::new("0"))) && parse_require_web_bundle_env(Some(OsStr::new("1"))));
        assert!(check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("1"))).unwrap_err().contains("CURVEBOOK_REQUIRE_WEB_BUNDLE"));
        let mut zip = b"PK\x03\x04".to_vec();
        zip.resize(2048, 0);
        dir.write_bytes("dist/release/curvebook-web-0.1.0.zip", &zip);
        assert!(check_web_bundle_readiness_with_env(dir.path(), Some(OsStr::new("1"))).is_ok());
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("dist/release/curvebook-web-0.1.0.zip"))
            .unwrap()
            .set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1)
            .unwrap();
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err());
        dir.write("Cargo.toml", "[profile.release]\nlto = \"fat\"\nstrip = \"symbols\"\n[profile.web-release]\ninherits = \"release\"\n");
        assert!(check_web_bundle_readiness_with_env(dir.path(), None).is_err());
        assert!(check_web_bundle_readiness(&crate::workspace_root().unwrap()).is_ok());

        let html = "<html><head><!-- <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\"> --><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; script-src-attr 'none'; style-src 'self'\"></head><body><script src=\"app.js\"></script><!-- <script>ignored();</script> --><script>const s = \"</script_tag\";</script><script/>a()</script></body></html>";
        let (rewritten, hashes) = rewrite_built_index_csp_hashes(html).unwrap();
        assert!(
            hashes.len() == 2
                && !rewritten.to_ascii_lowercase().contains("'unsafe-inline'")
                && rewritten.contains("script-src-attr 'none'")
                && hashes.iter().all(|h| h.starts_with("'sha256-") && rewritten.contains(h))
        );
        for bad_html in [
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self' 'UNSAFE-INLINE'\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self' &#39;unsafe-inline&#39;\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"&#x43;ontent-Security-Policy\" content=\"default-src *\"><link data-wasm-opt=\"z\">",
            "<meta =\" http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; script-src 'self'; style-src 'self'\"><link data-wasm-opt=\"z\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; style-src 'self'\">",
        ] {
            assert!(rewrite_built_index_csp_hashes(bad_html).is_err(), "{bad_html}");
        }
        #[cfg(unix)]
        {
            let d = TestDir::new("webfonts-sym");
            let secret = d.write("outside.ttf", "secret");
            let (src, stg) = (d.join("src"), d.join("stg"));
            std::fs::create_dir_all(&src).unwrap();
            std::os::unix::fs::symlink(&secret, src.join("Inter.ttf")).unwrap();
            assert!(stage_font_files(&src, &stg, &[String::from("Inter.ttf")]).is_err());
        }
    }
}
