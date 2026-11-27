//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;

use crate::rust_lex::{ExtractedLiteral, RustLexer, count_newlines};
use crate::scan_util::{
    TomlEntry, ZIP_CONTAINER_EXTS, collect_dist_files, extract_html_tag_name, find_html_comment_end, find_html_raw_close_tag_start,
    find_html_tag_close, looks_like_zip, lower_ext, parse_html_tag_attributes, parse_toml_entries,
};
use crate::text_decode::{decode_utf8_or_utf16, decode_utf16, decode_xml_html_entities};
use crate::zip_read::{ZipArchive, ZipLimits};

pub(crate) const BRANDS: &str = "graphpad dotmatics prism instat statmate jmp spss minitab originlab originpro sigmaplot systat statistica xlstat";
pub(crate) const PRE_RENAME_ASSET_BRANDS: &str = "lumencraft";
const SUITE_BRANDS: &str = "artcraft getartcraft storytold storyteller craft-fonts craft_fonts craft_fonts_dir craft-common-actions craft-contrib craft-house-rules craft-ui craft_ui craft-keymap craft-update craft-window";
pub(crate) const ADAPTED_UPSTREAM_REPO_URL: &str = "https://github.com/storytold/vectorcraft";
pub(crate) const ADAPTED_UPSTREAM_BRAND: &str = "vectorcraft";

const OPTICAL_PRISM_MODIFIERS: &str = "optical refractive triangular glass crystal dispersive dispersion dichroic achromatic";
const ATTRIBUTED_HEADING: &str = "## Attributed Source Code";
const ALLOW_MARKER: &str = "brand-ok";
const SELF_SCANNER_PATHS: &str = "xtask/src/brands.rs xtask/src/cleanroom.rs xtask/src/assets.rs xtask/src/bundle.rs xtask/src/layers.rs";
const ROADMAP_R24_ANCHOR: &str = "r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget";
const PLAN_PRISM_SCOPE: &str = "plan/prism";
const INTEROP_CORPUS_ALLOWLIST_FILE: &str = "xtask/src/interop_corpus.rs";
const INTEROP_CORPUS_DEFAULT_DIR: &str = "corpus/third-party-prism";
const PZFX_EXPORT_ALLOWLIST_FILE: &str = "crates/format/src/pzfx/export.rs";
const PZFX_VENDOR_ROOT_ELEMENT: &str = "GraphPadPrismFile";
const HOMEPAGE_HTML_PATH: &str = "docs/index.html";
const HOMEPAGE_ALLOWED_PHRASES: &[&str] = &[
    "Coming from GraphPad Prism",
    "A GraphPad Prism Pro license costs $720 per year",
    "*GraphPad Prism Pro annual pricing as listed in the EU on November 27, 2026.",
    "not affiliated with GraphPad Prism, GraphPad Software, or Dotmatics",
];
const BINARY_ASSET_EXT: &str =
    "png jpg jpeg gif webp bmp tif tiff ico icns avif svgz pdf eps icc icm cur ani ase aco ttf otf ttc woff woff2 eot pfb mp4 wav mp3";
const MAX_DIST_FILE_BYTES: usize = 256 * 1024 * 1024;
const MAX_GZIP_INFLATE_BYTES: usize = 1_048_576;

fn words_has(list: &str, needle: &str) -> bool {
    !needle.is_empty() && list.split_ascii_whitespace().any(|w| w == needle)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DistScanStats {
    pub files: usize,
    pub archive_entries: usize,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let mut hits = scan_repo_checked(root, &files)?;
    if root.join("packaging/web").exists() {
        hits.extend(scan_dist_artefacts(&root.join("packaging/web"))?);
    }
    let dist_stats = root.join("dist").exists().then(|| scan_dist_artefacts_with_stats(&root.join("dist"))).transpose()?.map(|(s, dh)| {
        hits.extend(dh);
        s
    });
    if hits.is_empty() {
        match dist_stats {
            Some(s) => println!(
                "brands: 0 unapproved brand or suite mentions across {} tracked files and {} dist/ files ({} archive entries)",
                files.len(),
                s.files,
                s.archive_entries
            ),
            None => println!("brands: 0 unapproved brand or suite mentions across {} tracked files", files.len()),
        }
        return Ok(());
    }
    let lines: Vec<String> = hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
    Err(format!("{} unapproved brand/suite mention(s):\n  {}", hits.len(), lines.join("\n  ")))
}

pub fn scan_repo_checked(root: &Path, files: &[String]) -> Result<Vec<BrandHit>, String> {
    let (mut hits, mut errs) = (Vec::new(), Vec::new());
    for path in files.iter().filter(|p| !skip_path(p)) {
        scan_tracked_path_name(path, &mut hits);
        let Ok(bytes) = curvebook_platform::fs::read_regular_file_bytes(&root.join(path), MAX_DIST_FILE_BYTES)
            .map_err(|e| errs.push(format!("cannot scan {path}: {e}")))
        else {
            continue;
        };
        let ext = lower_ext(path);
        let handled = try_scan_doc_binary_metadata(path, &ext, &bytes, false, &mut hits).unwrap_or_else(|e| (errs.push(e), true).1);
        if looks_like_zip(&bytes) || ZIP_CONTAINER_EXTS.contains(&ext.as_str()) {
            if let Err(e) = scan_zip_container_bytes(path, &bytes, 0, &mut DistScanStats::default(), &mut hits) {
                errs.push(e);
            }
        } else if !handled {
            match decode_text_file(path, &bytes) {
                Ok(Some(text)) => scan_file(path, &text, &mut hits),
                Ok(None) => {}
                Err(e) => errs.push(e),
            }
        }
    }
    if !errs.is_empty() {
        errs.extend(hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)));
        return Err(errs.join("\n  "));
    }
    Ok(hits)
}

fn norm_rel(path: &str) -> String {
    let n = path.replace('\\', "/");
    n.strip_prefix("./").unwrap_or(&n).to_owned()
}
fn path_contains_dir(t: &str, dir: &str) -> bool {
    t == dir || t.starts_with(&format!("{dir}/")) || t.contains(&format!("/{dir}/")) || t.ends_with(&format!("/{dir}"))
}
pub(crate) fn is_plan_prism_path(path: &str) -> bool {
    path_contains_dir(&norm_rel(path), PLAN_PRISM_SCOPE)
}
pub(crate) fn is_corpus_path(path: &str) -> bool {
    path_contains_dir(&norm_rel(path), "corpus")
}
pub(crate) fn contains_vendor_brand_word(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    BRANDS.split_ascii_whitespace().any(|brand| {
        let mut found = false;
        collect_word_matches(text, &lower, brand, PrismPolicy::Strict, |_| found = true);
        found
    })
}
pub(crate) fn is_forbidden_bundled_dir(rel: &str) -> bool {
    let t = norm_rel(rel);
    ["docs/audit", "docs/history/audit", PLAN_PRISM_SCOPE, "corpus"].iter().any(|d| path_contains_dir(&t, d))
}
pub(crate) fn is_crates_or_apps_path(path: &str) -> bool {
    let t = norm_rel(path);
    t.starts_with("crates/") || t.starts_with("apps/")
}
pub(crate) fn is_legal_policy_doc_path(path: &str) -> bool {
    let t = norm_rel(path);
    matches!(t.as_str(), "AGENTS.md" | "docs/ROADMAP.md" | "docs/planning/gui-layout-spec.md" | "plan/curvebook-rename.md" | "contribute/agents.html")
        || t.ends_with("/contribute/agents.html")
}
fn matches_root_or_packaged(path: &str, allow_packaged: bool, names: &[&str]) -> bool {
    let t = norm_rel(path);
    names.contains(&t.as_str())
        || (allow_packaged
            && !is_generated_docs_html_path(&t)
            && !t.split(['/', '!', ':']).any(|s| matches!(s, "crates" | "apps"))
            && t.rsplit(['/', ':', '!']).next().is_some_and(|n| names.contains(&n)))
}
pub(crate) fn is_attribution_or_notice_path(path: &str, allow_packaged: bool) -> bool {
    matches_root_or_packaged(path, allow_packaged, &["ATTRIBUTION.md", "NOTICE"])
}
pub(crate) fn is_attributed_source_section_heading(line: &str) -> bool {
    line.trim() == ATTRIBUTED_HEADING
}
pub(crate) fn closes_attributed_source_section(line: &str) -> bool {
    let t = line.trim();
    t != ATTRIBUTED_HEADING && ["# ", "#\t", "## ", "##\t"].iter().any(|p| t.starts_with(p))
}

pub(crate) fn mask_attributed_upstream_url<'a>(path: &str, text: &'a str, allow_packaged: bool) -> Cow<'a, str> {
    if !is_attribution_or_notice_path(path, allow_packaged) || !text.contains(ADAPTED_UPSTREAM_REPO_URL) {
        return Cow::Borrowed(text);
    }
    let is_end = |c: char| c.is_whitespace() || matches!(c, ')' | ']' | '}' | '>' | '`' | '"' | '\'' | ',' | ';');
    let is_pre = |c: char| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '<' | '`' | '"' | '\'' | ':' | ',' | ';');
    let (mut out, mut in_sec) = (String::with_capacity(text.len()), false);
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        in_sec = is_attributed_source_section_heading(bare) || (in_sec && !closes_attributed_source_section(bare));
        if !in_sec || !line.contains(ADAPTED_UPSTREAM_REPO_URL) {
            out.push_str(line);
            continue;
        }
        let mut cur = 0usize;
        while let Some(rel) = line.get(cur..).and_then(|s| s.find(ADAPTED_UPSTREAM_REPO_URL)) {
            let (pos, end) = (cur + rel, cur + rel + ADAPTED_UPSTREAM_REPO_URL.len());
            let mut after = line.get(end..).unwrap_or("").chars();
            let suf_ok = match after.next() {
                None => true,
                Some('.') => after.next().is_none_or(is_end),
                Some(c) => is_end(c),
            };
            if line.get(..pos).and_then(|s| s.chars().next_back()).is_none_or(is_pre) && suf_ok {
                out.push_str(line.get(cur..pos).unwrap_or(""));
                out.push_str("upstream-attributed-repo");
                cur = end;
            } else {
                out.push_str(line.get(cur..pos + 1).unwrap_or(""));
                cur = pos + 1;
            }
        }
        out.push_str(line.get(cur..).unwrap_or(""));
    }
    Cow::Owned(out)
}

pub(crate) fn inflate_gzip_bounded(gz: &[u8]) -> Result<Vec<u8>, ()> {
    let mut out = Vec::new();
    GzDecoder::new(gz).take((MAX_GZIP_INFLATE_BYTES + 1) as u64).read_to_end(&mut out).map_err(|_| ())?;
    (out.len() <= MAX_GZIP_INFLATE_BYTES).then_some(out).ok_or(())
}

fn try_scan_doc_binary_metadata(path: &str, ext: &str, bytes: &[u8], allow_packaged: bool, hits: &mut Vec<BrandHit>) -> Result<bool, String> {
    if ext == "pdf" || bytes.starts_with(b"%PDF-") {
        hits.extend(check_pdf_metadata_checked(path, bytes)?);
        return Ok(true);
    }
    if ext == "svgz" || (bytes.starts_with(&[0x1F, 0x8B]) && is_binary_asset_ext(path)) {
        let raw = inflate_gzip_bounded(bytes).map_err(|()| format!("cannot scan {path}: corrupt or oversized .svgz gzip payload"))?;
        let svg = decode_text_file(path, &raw)?.ok_or_else(|| format!("cannot scan {path}: .svgz payload is not valid UTF-8/UTF-16 SVG text"))?;
        let from = hits.len();
        scan_suite_lines_with_scope(path, &svg, allow_packaged, hits);
        hits.extend(check_svg_metadata(path, &svg));
        dedup_tail_hits(hits, from);
        return Ok(true);
    }
    Ok(false)
}

fn is_prism_format_module_path(path: &str) -> bool {
    let t = norm_rel(path);
    t == "crates/format/src/prism.rs"
        || t.strip_prefix("crates/format/src/prism/")
            .is_some_and(|leaf| !leaf.is_empty() && !leaf.contains('/') && leaf.ends_with(".rs") && leaf != ".rs")
}

fn scan_tracked_path_name(path: &str, out: &mut Vec<BrandHit>) {
    let norm = path.replace('\\', "/");
    if norm == PLAN_PRISM_SCOPE || norm.starts_with("plan/prism/") {
        return out.push(BrandHit { path: path.to_owned(), line: 1, brand: "prism" });
    }
    let from = out.len();
    collect_line_words(path, 1, &norm, SUITE_BRANDS, PrismPolicy::Strict, out);
    if is_crates_or_apps_path(&norm) {
        collect_line_words(path, 1, &norm, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict, out);
    }
    if let Some(leaf) = norm.strip_prefix("crates/format/src/prism/")
        && !leaf.is_empty()
        && !leaf.contains('/')
        && leaf.ends_with(".rs")
        && leaf != ".rs"
    {
        collect_line_words(path, 1, leaf, BRANDS, PrismPolicy::Strict, out);
    } else {
        let pol = if norm == "crates/format/src/prism.rs" { PrismPolicy::AllowExactBarePrism } else { PrismPolicy::Strict };
        collect_line_words(path, 1, &norm, BRANDS, pol, out);
    }
    dedup_tail_hits(out, from);
}

fn is_binary_asset_ext(path: &str) -> bool {
    words_has(BINARY_ASSET_EXT, &lower_ext(path))
}
fn decode_text_file(path: &str, bytes: &[u8]) -> Result<Option<String>, String> {
    match decode_utf8_or_utf16(bytes) {
        Ok(Some(text)) => Ok(Some(text)),
        Err(err) => Err(format!("cannot scan {path}: {err}")),
        Ok(None) if is_binary_asset_ext(path) => Ok(None),
        Ok(None) => Err(format!("cannot scan {path}: file is not valid UTF-8 or UTF-16")),
    }
}
fn skip_path(path: &str) -> bool {
    words_has(SELF_SCANNER_PATHS, &norm_rel(path))
}

fn scan_common_surface_text(path: &str, ext: &str, text: &str, allow_packaged: bool, out: &mut Vec<BrandHit>) -> bool {
    let lp = path.to_ascii_lowercase();
    if ext == "svg" {
        out.extend(check_svg_metadata(path, text));
    } else if matches!(ext, "html" | "htm") {
        out.extend(check_html_metadata(path, text));
    } else if ext == "wxs" {
        scan_wxs(path, text, out);
    } else if lp.ends_with(".plist")
        || lp.ends_with(".plist.in")
        || lp.ends_with(".mime.xml")
        || lp.ends_with(".desktop")
        || (ext == "xml" && path_contains_dir(&lp, "share/mime/packages"))
    {
        scan_packaging_descriptor(path, &lp, text, out);
    } else if allow_packaged && ext == "json" {
        out.extend(check_manifest_metadata(path, text));
    } else {
        return false;
    }
    true
}

fn scan_wxs(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        let (t, ln) = (line.trim(), idx + 1);
        collect_line_words(path, ln, line, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, out);
        if t.starts_with("<Extension ") && t.ends_with('>') {
            let mut work = t.to_owned();
            for exact_attr in [r#"Id="prism""#, r#"ContentType="application/x-prism-project""#, r#"ContentType="application/x-prism-xml""#] {
                if work.contains(exact_attr) {
                    work = work.replace(exact_attr, r#"Attr="format_ext""#);
                }
            }
            collect_line_words(path, ln, &work, BRANDS, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, ln, line, BRANDS, PrismPolicy::Strict, out);
        }
    }
}

fn scan_packaging_descriptor(path: &str, lp: &str, text: &str, out: &mut Vec<BrandHit>) {
    let (is_plist, is_desktop, mut in_key, mut in_arr) =
        (lp.ends_with(".plist") || lp.ends_with(".plist.in"), lp.ends_with(".desktop"), false, false);
    for (idx, line) in text.lines().enumerate() {
        let (t, ln) = (line.trim(), idx + 1);
        collect_line_words(path, ln, line, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, out);
        if is_plist {
            match t {
                "<key>CFBundleTypeExtensions</key>" => (in_key, in_arr) = (true, false),
                "<array>" if in_key => (in_key, in_arr) = (false, true),
                "</array>" if in_arr => in_arr = false,
                "<string>prism</string>" if in_arr => {}
                _ => (in_key, ()) = (false, collect_line_words(path, ln, line, BRANDS, PrismPolicy::Strict, out)),
            }
        } else if is_desktop && let Some(vals) = t.strip_prefix("MimeType=") {
            vals.split(';')
                .filter(|x| !matches!(x.trim(), "application/x-prism-project" | "application/x-prism-xml"))
                .for_each(|tok| collect_line_words(path, ln, tok, BRANDS, PrismPolicy::Strict, out));
        } else if is_desktop
            || !matches!(
                t,
                "<mime-type type=\"application/x-prism-project\">" | "<mime-type type=\"application/x-prism-xml\">" | "<glob pattern=\"*.prism\"/>"
            )
        {
            collect_line_words(path, ln, line, BRANDS, PrismPolicy::Strict, out);
        }
    }
}

fn scan_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let (from, ext) = (out.len(), lower_ext(path));
    scan_suite_lines_with_scope(path, text, false, out);
    let is_prov = matches!(Path::new(path).file_name().and_then(|n| n.to_str()), Some("ASSETS.md" | "ATTRIBUTION.md" | "NOTICE"));
    if ext == "rs" {
        scan_rust(path, text, out);
    } else if path == "Cargo.toml" || path.ends_with("/Cargo.toml") {
        scan_cargo_toml(path, text, out);
    } else if path == "Cargo.lock" || scan_common_surface_text(path, &ext, text, false, out) {
    } else if path == "crates/ui-egui/locales/catalog.tsv" {
        scan_locale_catalog_tsv(path, text, out);
    } else {
        scan_text_lines(path, text, false, !is_prov && ext == "md", out);
    }
    dedup_tail_hits(out, from);
}

fn scan_locale_catalog_tsv(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let (mut it_col, mut saw_header) = (None, false);
    for (idx, line) in text.lines().enumerate() {
        let (t, ln) = (line.trim(), idx + 1);
        if t.is_empty() || t.starts_with('#') || !saw_header {
            if !t.is_empty() && !t.starts_with('#') {
                (saw_header, it_col) = (true, line.split('\t').position(|h| h.trim() == "it"));
            }
            collect_line_words(path, ln, line, BRANDS, PrismPolicy::Strict, out);
            continue;
        }
        for (col, cell) in line.split('\t').enumerate() {
            let pol = if col == 0 || it_col == Some(col) {
                PrismPolicy::UiTextRun { it_stat: it_col == Some(col), cmd_id: col == 0 }
            } else {
                PrismPolicy::Strict
            };
            collect_line_words(path, ln, cell, BRANDS, pol, out);
        }
    }
}

fn dedup_tail_hits(hits: &mut Vec<BrandHit>, from: usize) {
    let (mut seen, mut i) = (BTreeSet::new(), from);
    while i < hits.len() {
        if seen.insert((hits[i].line, hits[i].brand)) {
            i += 1;
        } else {
            hits.remove(i);
        }
    }
}

fn scan_suite_lines_with_scope(path: &str, text: &str, allow_packaged: bool, out: &mut Vec<BrandHit>) {
    let masked = mask_attributed_upstream_url(path, text, allow_packaged);
    let check_adapted =
        is_crates_or_apps_path(path) || (allow_packaged && !is_attribution_or_notice_path(path, true) && !is_legal_policy_doc_path(path));
    let is_readme = matches_root_or_packaged(path, allow_packaged, &["README.md"]);
    let strict_suite = SUITE_BRANDS.strip_prefix("artcraft ").unwrap_or(SUITE_BRANDS);
    for (idx, line) in masked.lines().enumerate() {
        let words = if is_readme && line.contains(ALLOW_MARKER) { strict_suite } else { SUITE_BRANDS };
        collect_line_words(path, idx + 1, line, words, PrismPolicy::Strict, out);
        if check_adapted {
            collect_line_words(path, idx + 1, line, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict, out);
        }
    }
}

fn scan_cargo_toml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let mut cur_sec_len = 0usize;
    for entry in parse_toml_entries(text) {
        let (line, segs, skip, val) = match &entry {
            TomlEntry::Section { line, path } => {
                cur_sec_len = path.len();
                let skip = if path.first().map(String::as_str) == Some("workspace") { 2 } else { 1 };
                (*line, path, skip, None)
            }
            TomlEntry::Leaf { line, path, value } => (*line, path, cur_sec_len, Some(value)),
        };
        let first = segs.first().map(String::as_str);
        if first != Some("package") && !(first == Some("workspace") && matches!(segs.get(1).map(String::as_str), Some("package" | "metadata"))) {
            continue;
        }
        segs.iter().skip(skip).for_each(|seg| collect_line_words(path, line, seg, BRANDS, PrismPolicy::Strict, out));
        if let Some(value) = val {
            let mut vals = Vec::new();
            value.collect_strings(&mut vals);
            vals.into_iter()
                .for_each(|s| s.lines().enumerate().for_each(|(off, l)| collect_line_words(path, line + off, l, BRANDS, PrismPolicy::Strict, out)));
        }
    }
}

fn is_generated_docs_html_path(path: &str) -> bool {
    let t = norm_rel(path);
    (t.starts_with("docs-site/") && t != "docs-site/manual/index.html")
        || t.starts_with("target/docs-site/")
        || (t.starts_with("target/docs-manual/") && t != "target/docs-manual/index.html")
}

fn mask_generated_docs_html_tokens(path: &str, line: &str) -> Option<String> {
    const TOKENS: &str = "table.importPrism table_import_prism #cmd-table-importprism #mcp-tool-table_import_prism project.exportPrism project_export_prism #cmd-project-exportprism #mcp-tool-project_export_prism #format-prism id=\"format-prism\" import_prism_bytes export_prism_bytes prism.rs {pzfx,prism, {pzfx,prism} &quot;prism&quot; \"prism\" <code>prism</code> `prism` https://github.com/HauserGroup/curvebook/ http://github.com/HauserGroup/curvebook/";
    let norm = path.replace('\\', "/");
    if norm.ends_with("/registry/formats.html") && line.trim() == "prism" {
        return Some(String::from("format_token"));
    }
    let (mut work, mut changed) = (line.to_owned(), false);
    let mut rep = |needle: &str, to: &str| {
        if work.contains(needle) {
            (work, changed) = (work.replace(needle, to), true);
        }
    };
    rep(ROADMAP_R24_ANCHOR, "r24-clean-room-interchange");
    if norm.ends_with("/contribute/agents.html") && ((line.contains("Scope:") && line.contains(PLAN_PRISM_SCOPE)) || line.trim() == "plan/prism/") {
        rep(PLAN_PRISM_SCOPE, "plan/ref_frames");
    }
    rep("crates/format/src/prism/", "crates/format/src/format_mod/");
    TOKENS.split_ascii_whitespace().for_each(|tok| rep(tok, "format_token"));
    changed.then_some(work)
}

fn mask_homepage_html_phrases(source: &str, line: &str) -> Option<String> {
    if norm_rel(source) != HOMEPAGE_HTML_PATH {
        return None;
    }
    let (mut work, mut changed) = (line.to_owned(), false);
    for &phrase in HOMEPAGE_ALLOWED_PHRASES {
        if work.contains(phrase) {
            (work, changed) = (work.replace(phrase, "homepage_allowed_phrase"), true);
        }
    }
    changed.then_some(work)
}

fn scan_text_lines(path: &str, text: &str, allow_packaged: bool, is_md: bool, out: &mut Vec<BrandHit>) {
    let (is_readme, is_gen_docs) = (matches_root_or_packaged(path, allow_packaged, &["README.md"]), !is_md && is_generated_docs_html_path(path));
    let is_dev = is_md && matches!(path, "docs/commands.md" | "docs/development.md" | "docs/extending.md");
    let strict_disc = BRANDS.strip_prefix("graphpad dotmatics ").unwrap_or(BRANDS);
    for (idx, line) in text.lines().enumerate() {
        let brands = if is_readme && line.contains(ALLOW_MARKER) { strict_disc } else { BRANDS };
        let mut work = Cow::Borrowed(line);
        if is_gen_docs && let Some(m) = mask_generated_docs_html_tokens(path, line) {
            work = Cow::Owned(m);
        } else if is_md {
            if work.contains(ROADMAP_R24_ANCHOR) {
                work = Cow::Owned(work.replace(ROADMAP_R24_ANCHOR, "r24-clean-room-interchange"));
            }
            if path == "AGENTS.md" && line.contains("Scope:") && line.contains(PLAN_PRISM_SCOPE) {
                work = Cow::Owned(work.replace(PLAN_PRISM_SCOPE, "plan/ref_frames"));
            }
            if is_dev && work.contains("crates/format/src/prism/") {
                work = Cow::Owned(work.replace("crates/format/src/prism/", "crates/format/src/format_mod/"));
            }
            for tok in
                ["table.importPrism", "project.exportPrism", "import_prism_bytes", "export_prism_bytes", "prism.rs", "{pzfx,prism,", "{pzfx,prism}"]
            {
                if work.contains(tok) && (is_dev || matches!(tok, "table.importPrism" | "project.exportPrism")) {
                    work = Cow::Owned(work.replace(tok, "format_importer"));
                }
            }
        }
        collect_line_words(path, idx + 1, &work, brands, PrismPolicy::Strict, out);
    }
}

fn is_negative_brand_guard_literal(path: &str, src: &str, lit: &ExtractedLiteral) -> bool {
    if !(path.starts_with("tests/") || path.contains("/tests/")) || lit.start_line != lit.end_line {
        return false;
    }
    let ls = src.get(..lit.quote_start).and_then(|s| s.rfind('\n')).map_or(0, |p| p + 1);
    let le = src.get(lit.quote_end..).and_then(|s| s.find('\n')).map_or(src.len(), |r| lit.quote_end + r);
    let blanked = crate::rust_lex::blank_comments_and_literals(src.get(ls..le).unwrap_or(""));
    blanked
        .get(..lit.quote_start.saturating_sub(ls))
        .unwrap_or("")
        .rsplit_once("for forbidden in [")
        .is_some_and(|(_, tail)| !tail.contains(']') && blanked.get(lit.quote_end.saturating_sub(ls)..).unwrap_or("").contains(']'))
}

fn scan_rust(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    const FMT_TOKS: &str = "table.importPrism importPrism import_prism_bytes project.exportPrism exportPrism export_prism_bytes project_export_prism ExportPrismParams run_project_export_prism <PrismFile </PrismFile> PrismXMLVersion PrismTableMeta";
    let (lines, mut lexer, mut seen) = (text.lines().collect::<Vec<_>>(), RustLexer::new(text), BTreeSet::new());
    let (strict_ui, is_fmt) = (
        path.starts_with("crates/ui-egui/src/") || path.starts_with("apps/"),
        ["crates/format/", "crates/engine/", "crates/mcp/", "apps/"].iter().any(|p| path.starts_with(p)),
    );
    while !lexer.is_eof() {
        if let Some((start_line, body)) = lexer.try_read_comment() {
            let allow_it = path == "crates/stats/src/pvalue_stack.rs" && body.contains("Teoria statistica delle classi");
            for (off, line_text) in body.lines().enumerate() {
                let mut work = Cow::Borrowed(line_text);
                if is_fmt {
                    for tok in FMT_TOKS.split_ascii_whitespace() {
                        if work.contains(tok) {
                            work = Cow::Owned(work.replace(tok, "format_tok"));
                        }
                    }
                    if (work.contains("`pzfx`") || work.contains("`curvebook`")) && work.contains("`prism`") {
                        work = Cow::Owned(work.replace("`prism`", "`fmt`"));
                    }
                }
                emit_words(path, &work, BRANDS, PrismPolicy::RustComment(allow_it), |_| start_line + off, &mut seen, out);
            }
        } else if lexer.try_skip_char_literal() {
        } else if let Some(lit) = lexer.try_read_raw_string().or_else(|| lexer.try_read_quoted_string()) {
            let rel_path = norm_rel(path);
            if is_negative_brand_guard_literal(path, text, &lit)
                || (rel_path == INTEROP_CORPUS_ALLOWLIST_FILE && lit.text == INTEROP_CORPUS_DEFAULT_DIR)
                || (rel_path == PZFX_EXPORT_ALLOWLIST_FILE && lit.text == PZFX_VENDOR_ROOT_ELEMENT)
            {
                continue;
            }
            let pol = PrismPolicy::RustLiteral {
                bare: is_import_export_prism_literal(path, text, &lit),
                cmd: lit.text.contains("table.importPrism")
                    || lit.text == "cmd.table.importPrism"
                    || lit.text == "importPrism"
                    || lit.text.contains("project.exportPrism")
                    || lit.text == "cmd.project.exportPrism"
                    || lit.text == "exportPrism",
                xml: ["<PrismFile", "</PrismFile>", "PrismXMLVersion"].iter().any(|t| lit.text.contains(t)) || lit.text.trim() == "PrismFile",
                it_stat: strict_ui
                    && path == "crates/ui-egui/src/i18n.rs"
                    && lines.get(lit.start_line.saturating_sub(1)).is_some_and(|l| l.trim_start().starts_with("it:")),
                test_op: is_format_test_container_literal(path, &lit.text),
            };
            let line_of = |pos: usize| lit.byte_lines.get(pos).copied().unwrap_or(lit.start_line);
            emit_words(path, &lit.text, BRANDS, pol, line_of, &mut seen, out);
            emit_words(path, &lit.text, SUITE_BRANDS, PrismPolicy::Strict, line_of, &mut seen, out);
            if is_crates_or_apps_path(path) {
                emit_words(path, &lit.text, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict, line_of, &mut seen, out);
            }
        } else if let Some((line, ident)) = lexer.try_read_identifier() {
            let pol = PrismPolicy::RustIdent {
                dsl_jmp: path == "crates/fit/src/dsl.rs" && ident == "jmp_end",
                canonical_prism: is_canonical_format_rust_ident(path, ident),
            };
            emit_words(path, ident, BRANDS, pol, |_| line, &mut seen, out);
        } else {
            lexer.advance_one();
        }
    }
}

fn is_format_test_container_literal(path: &str, text: &str) -> bool {
    const WORDS: &str = "zip|pzfx|bytes|open |open:|write |save |to prism|finish |import |export |expected |prism json|prism:";
    let is_test = path.starts_with("tests/") || path.contains("/tests/");
    ((text.contains("import_prism_bytes") || text.contains("export_prism_bytes")) && (path.starts_with("crates/format/") || is_test))
        || ((is_test || path == "apps/curvebook-web/src/lib.rs" || is_prism_format_module_path(path))
            && !text.contains("Prism")
            && text.contains("prism")
            && text.chars().all(|b| b.is_ascii_alphanumeric() || matches!(b, '_' | '-' | '.' | '/' | ' ' | ':' | '{' | '}'))
            && WORDS.split('|').any(|w| text.contains(w)))
}

fn is_canonical_format_rust_ident(path: &str, ident: &str) -> bool {
    const EXACT: &str =
        "prism DESC_PRISM PrismAdapter PrismManifest PrismTableMeta PrismInfoMeta ImportPrismParams ExportPrismParams PrismFile PrismXMLVersion";
    const PFX: &str = "import_prism export_prism project_export_prism open_prism run_import_prism run_project_export_prism schema_import_prism schema_export_prism schema_project_export_prism parse_prism_constant";
    const FORBIDDEN: &str = "style theme palette logo icon ui widget dialog like clone copy competitor vendor brand title_bar";
    const REQUIRED: &str = "zip pzfx bytes import export open report manifest file path sess host res sum opened inspect container archive cleanroom synthetic merge budget json csv bad nan legacy renamed tpl batch huge test c_prism";
    if path.starts_with("crates/ui-egui/src/") {
        return false;
    }
    if words_has(EXACT, ident) || PFX.split_ascii_whitespace().any(|p| ident.starts_with(p)) {
        return true;
    }
    if !["crates/format/", "crates/engine/", "crates/mcp/", "apps/"].iter().any(|p| path.starts_with(p))
        || ident.bytes().any(|b| b.is_ascii_uppercase())
    {
        return false;
    }
    let toks: Vec<&str> = ident.split('_').filter(|t| !t.is_empty()).collect();
    !toks.iter().any(|t| words_has(FORBIDDEN, t)) && (ident == "c_prism" || toks.iter().any(|t| words_has(REQUIRED, t)))
}

#[derive(Clone, Copy)]
enum PrismPolicy<'a> {
    Strict,
    AllowExactBarePrism,
    RustComment(bool),
    RustIdent { dsl_jmp: bool, canonical_prism: bool },
    RustLiteral { bare: bool, cmd: bool, xml: bool, it_stat: bool, test_op: bool },
    UiTextRun { it_stat: bool, cmd_id: bool },
    SvgTitleOrDesc(Option<&'a str>),
}

fn emit_words(
    path: &str,
    raw: &str,
    words: &'static str,
    policy: PrismPolicy<'_>,
    line_of: impl Fn(usize) -> usize,
    seen: &mut BTreeSet<(usize, &'static str)>,
    out: &mut Vec<BrandHit>,
) {
    let lower = raw.to_ascii_lowercase();
    for brand in words.split_ascii_whitespace() {
        collect_word_matches(raw, &lower, brand, policy, |pos| {
            let line = line_of(pos);
            if seen.insert((line, brand)) {
                out.push(BrandHit { path: path.to_owned(), line, brand });
            }
        });
    }
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &'static str, policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    emit_words(path, haystack, words, policy, |_| line_no, &mut BTreeSet::new(), out);
}

fn collect_word_matches(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>, mut on_match: impl FnMut(usize)) {
    let bytes = raw.as_bytes();
    if word.contains(['-', '_']) {
        let (lb, mut start) = (lower.as_bytes(), 0usize);
        while let Some(rel) = lower.get(start..).and_then(|s| s.find(word)) {
            let (pos, end) = (start + rel, start + rel + word.len());
            if (pos == 0 || !lb[pos - 1].is_ascii_alphanumeric()) && (end == lb.len() || !lb[end].is_ascii_alphanumeric()) {
                on_match(pos);
            }
            start = pos + 1;
        }
        return;
    }
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_alphanumeric() {
            idx += 1;
            continue;
        }
        let mut parts = Vec::new();
        while idx < bytes.len() {
            let s = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_alphanumeric() {
                idx += 1;
            }
            parts.push((s, idx));
            if idx < bytes.len() && matches!(bytes[idx], b'_' | b'-') && bytes.get(idx + 1).is_some_and(u8::is_ascii_alphanumeric) {
                idx += 1;
            } else {
                break;
            }
        }
        for &(rs, re) in &parts {
            let (mut subs, mut sub_s) = (Vec::new(), rs);
            for i in (rs + 1)..re {
                let (p, c, n) = (bytes[i - 1], bytes[i], bytes.get(i + 1).copied());
                if ((p.is_ascii_lowercase() || p.is_ascii_digit()) && c.is_ascii_uppercase())
                    || (p.is_ascii_uppercase() && c.is_ascii_uppercase() && n.is_some_and(|x| x.is_ascii_lowercase()))
                    || (p.is_ascii_digit() != c.is_ascii_digit())
                {
                    subs.push((sub_s, i));
                    sub_s = i;
                }
            }
            subs.push((sub_s, re));
            for s in 0..subs.len() {
                for count in 1..=3.min(subs.len() - s) {
                    let (a, b) = (subs[s].0, subs[s + count - 1].1);
                    if lower.get(a..b) == Some(word) && !is_token_match_exempt(raw, lower, word, a, b, policy) {
                        on_match(a);
                    }
                }
            }
        }
        if word.len() >= 4 && parts.len() > 1 {
            for i in 0..parts.len() {
                let mut joined = String::new();
                for &(ps, pe) in &parts[i..(i + 3).min(parts.len())] {
                    let prev = joined.len();
                    joined.push_str(lower.get(ps..pe).unwrap_or(""));
                    if prev > 0 && joined == word && !is_token_match_exempt(raw, lower, word, parts[i].0, pe, policy) {
                        on_match(parts[i].0);
                    }
                }
            }
        }
    }
}

fn is_token_match_exempt(raw: &str, lower: &str, word: &'static str, pos: usize, end: usize, policy: PrismPolicy<'_>) -> bool {
    match word {
        "prism" => {
            if raw.as_bytes().get(pos..end) == Some(b"prism") && is_prism_extension_token(lower.as_bytes(), pos, end) {
                return true;
            }
            let is_cmd = raw.get(pos.saturating_sub(12)..end) == Some("table.importPrism")
                || raw.get(pos.saturating_sub(6)..end) == Some("importPrism")
                || raw.get(pos.saturating_sub(14)..end) == Some("project.exportPrism")
                || raw.get(pos.saturating_sub(6)..end) == Some("exportPrism");
            let is_xml = raw.get(pos..pos + 9) == Some("PrismFile") || raw.get(pos..pos + 15) == Some("PrismXMLVersion");
            match policy {
                PrismPolicy::Strict | PrismPolicy::RustComment(_) => false,
                PrismPolicy::AllowExactBarePrism => raw.as_bytes().get(pos..end) == Some(b"prism"),
                PrismPolicy::RustIdent { canonical_prism, .. } => canonical_prism,
                PrismPolicy::RustLiteral { bare, cmd, xml, test_op, .. } => {
                    (bare && raw == "prism") || (cmd && is_cmd) || (xml && is_xml) || (test_op && raw.as_bytes().get(pos..end) == Some(b"prism"))
                }
                PrismPolicy::UiTextRun { cmd_id, .. } => cmd_id && is_cmd,
                PrismPolicy::SvgTitleOrDesc(prev) => {
                    let t = lower.get(..pos).unwrap_or("").trim_end();
                    words_has(OPTICAL_PRISM_MODIFIERS, if t.is_empty() { prev.unwrap_or("") } else { trailing_ascii_word(t) })
                }
            }
        }
        "statistica" => {
            raw.get(pos..end) == Some("statistica")
                && matches!(
                    policy,
                    PrismPolicy::RustComment(true) | PrismPolicy::RustLiteral { it_stat: true, .. } | PrismPolicy::UiTextRun { it_stat: true, .. }
                )
        }
        "jmp" => matches!(policy, PrismPolicy::RustIdent { dsl_jmp: true, .. }),
        _ => false,
    }
}

fn is_import_export_prism_literal(path: &str, src: &str, lit: &ExtractedLiteral) -> bool {
    const CALLS: &str = "apply_import_report load_import_report importer_by_format exporter_by_format add_filter open_prism import_prism export_prism project_export_prism handle_native_import_file formatdescriptor file_extensions import_extensions export_extensions format_tag format_id format_opt detect_ext";
    const UI_FNS: &str = "label title text button tooltip heading window_title set_title";
    const UI_FLDS: &str = "label title text tooltip summary beginner message placeholder";
    if lit.text != "prism" {
        return false;
    }
    let (before, after) = (src.get(..lit.quote_start).unwrap_or(""), src.get(lit.quote_end..).unwrap_or(""));
    let (bt, at) = (before.trim_end(), after.trim_start());
    let (stmt, tail) = (before.rsplit_once([';', '{', '}']).map_or(before, |(_, a)| a), after.split_once([';', '{', '}']).map_or(after, |(a, _)| a));
    let stmt_lc = stmt.to_ascii_lowercase();
    let in_ext_arr = stmt
        .rsplit_once('[')
        .filter(|(_, ib)| !ib.contains(']'))
        .zip(tail.split_once(']').filter(|(ia, _)| !ia.contains('[')))
        .is_some_and(|((pre, ib), (ia, _))| {
            ["\"pzfx\"", "\"cbtemplate\"", "\"curvebook\"", "\"curvebook.d\"", "\"csv\"", "\"tsv\""].iter().any(|p| ib.contains(p) || ia.contains(p))
                || ["add_filter", "extension", "file_filter", "import", "export", "accept", "format_"]
                    .iter()
                    .any(|m| pre.to_ascii_lowercase().contains(m))
        });
    let ui_ctx = bt.rsplit_once('(').is_some_and(|(bp, _)| words_has(UI_FNS, &trailing_ascii_word(bp.trim_end()).to_ascii_lowercase()))
        || bt.rsplit_once(':').is_some_and(|(bc, _)| words_has(UI_FLDS, &trailing_ascii_word(bc.trim_end()).to_ascii_lowercase()));
    (bt.ends_with("Some(") && at.strip_prefix(')').is_some_and(|r| ["=>", "|", "if "].iter().any(|p| r.trim_start().starts_with(p))))
        || ((bt.ends_with('|') || at.starts_with('|'))
            && (tail.contains("=>")
                || stmt.contains("matches!")
                || ["\"pzfx\"", "\"curvebook\""].iter().any(|p| tail.contains(p) || stmt.contains(p))))
        || ((bt.ends_with("==") || bt.ends_with("!=")) && {
            let part = bt.trim_end_matches(['=', '!']).trim_end();
            let ident = trailing_ascii_word(part).to_ascii_lowercase();
            ["ext", "format", "fmt", "filter", "kind", "importer"].iter().any(|k| ident.contains(k))
                || part.ends_with("[\"format\"]")
                || part.ends_with("[\"id\"]")
        })
        || in_ext_arr
        || (!ui_ctx
            && (CALLS.split_ascii_whitespace().any(|m| stmt_lc.contains(m))
                || (stmt_lc.contains("[\"format\"]") && (bt.ends_with("json!(") || bt.ends_with("Some(") || bt.ends_with("==")))))
        || (path == "crates/format/src/registry.rs" && bt.ends_with("id:") && before.contains("FormatDescriptor"))
        || ((path.starts_with("tests/") || path.contains("/tests/"))
            && (bt.ends_with(".id,")
                || stmt.contains("unique_temp_path")
                || stmt.contains("TempDir::new")
                || (bt.ends_with('(') && tail.contains(".prism\""))
                || (stmt.contains("assert_eq!") && stmt.contains("[\"format\"]"))))
}

fn is_prism_extension_token(bytes: &[u8], pos: usize, end: usize) -> bool {
    if pos == 0 || bytes[pos - 1] != b'.' || (bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric)) {
        return false;
    }
    let mut i = pos - 1;
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'-' | b'.')) {
        i -= 1;
    }
    !(i >= 3 && bytes.get(i - 3..i) == Some(b"://"))
}

fn trailing_ascii_word(s: &str) -> &str {
    let b = s.as_bytes();
    let mut start = b.len();
    while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'-') {
        start -= 1;
    }
    s.get(start..).unwrap_or("")
}

fn check_single_string_with_extra(
    path: &str,
    line: usize,
    text: &str,
    prism_policy: PrismPolicy<'_>,
    include_pre_rename: bool,
    out: &mut Vec<BrandHit>,
) {
    let mut seen = BTreeSet::new();
    emit_words(path, text, SUITE_BRANDS, PrismPolicy::Strict, |_| line, &mut seen, out);
    if !is_legal_policy_doc_path(path) && !is_attribution_or_notice_path(path, true) {
        emit_words(path, text, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict, |_| line, &mut seen, out);
    }
    if include_pre_rename {
        emit_words(path, text, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, |_| line, &mut seen, out);
    }
    emit_words(path, text, BRANDS, prism_policy, |_| line, &mut seen, out);
}

pub fn check_ui_text_run(path: &str, line: usize, text: &str, it_stat: bool, cmd_id: bool, out: &mut Vec<BrandHit>) {
    check_single_string_with_extra(path, line, text, PrismPolicy::UiTextRun { it_stat, cmd_id }, false, out);
}
#[must_use]
pub fn check_text_runs<I: IntoIterator<Item = S>, S: AsRef<str>>(source: &str, runs: I) -> Vec<BrandHit> {
    let (mut out, pol) = (Vec::new(), PrismPolicy::UiTextRun { it_stat: false, cmd_id: false });
    runs.into_iter().enumerate().for_each(|(i, r)| check_single_string_with_extra(source, i + 1, r.as_ref(), pol, false, &mut out));
    out
}
#[must_use]
pub fn check_text_runs_strict<I: IntoIterator<Item = S>, S: AsRef<str>>(source: &str, runs: I) -> Vec<BrandHit> {
    let mut out = Vec::new();
    runs.into_iter().enumerate().for_each(|(i, r)| check_single_string_with_extra(source, i + 1, r.as_ref(), PrismPolicy::Strict, false, &mut out));
    out
}
#[must_use]
pub fn check_labeled_text_runs<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(runs: I) -> Vec<BrandHit> {
    let (mut out, pol) = (Vec::new(), PrismPolicy::UiTextRun { it_stat: false, cmd_id: false });
    runs.into_iter().enumerate().for_each(|(i, (l, t))| check_single_string_with_extra(l, i + 1, t, pol, false, &mut out));
    out
}
#[must_use]
pub fn check_i18n_catalog_entries<'a, I: IntoIterator<Item = (&'a str, &'a str, &'a str)>>(entries: I) -> Vec<BrandHit> {
    let mut out = Vec::new();
    for (key, locale, text) in entries {
        let pol = PrismPolicy::UiTextRun { it_stat: locale == "it", cmd_id: false };
        check_single_string_with_extra(&format!("i18n:{locale}:{key}"), 1, text, pol, false, &mut out);
    }
    out
}

fn scan_xml_text_fragment(source: &str, start_line: usize, raw: &str, svg_title: bool, out: &mut Vec<BrandHit>) {
    let (decoded, mut prev) = (decode_xml_html_entities(raw), None::<String>);
    for (off, line_text) in decoded.lines().enumerate() {
        let pol = if svg_title { PrismPolicy::SvgTitleOrDesc(prev.as_deref()) } else { PrismPolicy::Strict };
        check_single_string_with_extra(source, start_line + off, line_text, pol, true, out);
        if svg_title {
            let last = trailing_ascii_word(line_text.trim_end());
            if !last.is_empty() {
                prev = Some(last.to_ascii_lowercase());
            }
        }
    }
}

fn scan_markup_tag_attrs(source: &str, line: usize, trimmed: &str, is_html: bool, is_gen_docs: bool, out: &mut Vec<BrandHit>) -> String {
    let raw_name = extract_html_tag_name(trimmed);
    let local = raw_name.rsplit_once(':').map_or(raw_name, |(_, l)| l).to_ascii_lowercase();
    if trimmed.starts_with('/') {
        return local;
    }
    if !is_gen_docs {
        check_single_string_with_extra(source, line, raw_name, PrismPolicy::Strict, true, out);
    }
    for (k, raw_val) in parse_html_tag_attributes(trimmed) {
        let val = match raw_val.split_once(',') {
            Some((m, _)) if m.trim_start().to_ascii_lowercase().starts_with("data:image/") && m.to_ascii_lowercase().contains(";base64") => m,
            _ => raw_val.as_str(),
        };
        if !is_html {
            check_single_string_with_extra(source, line, &k, PrismPolicy::Strict, true, out);
            scan_xml_text_fragment(source, line, val, false, out);
        } else {
            let al = k.to_ascii_lowercase();
            let strict = matches!(al.as_str(), "aria-label" | "alt" | "title") || (local == "meta" && al == "content");
            if !strict && is_gen_docs && al == "id" && val == "format-prism" {
                continue;
            }
            let masked = (!strict && is_gen_docs).then(|| mask_generated_docs_html_tokens(source, val)).flatten();
            check_single_string_with_extra(source, line, masked.as_deref().unwrap_or(val), PrismPolicy::Strict, true, out);
        }
    }
    local
}

fn scan_markup_metadata(source: &str, doc: &str, is_html: bool) -> Vec<BrandHit> {
    let (mut out, mut line, mut pos, mut depth, mut text_start) = (Vec::new(), 1usize, 0usize, 0usize, 1usize);
    let (mut raw_close, mut buf, is_gen_docs) = (None::<&'static str>, String::new(), is_html && is_generated_docs_html_path(source));
    let flush = |sl: usize, raw: &str, strict: bool, out: &mut Vec<BrandHit>| {
        if raw.trim().is_empty() {
            return;
        }
        if !is_html {
            return scan_xml_text_fragment(source, sl, raw, strict, out);
        }
        for (off, l) in decode_xml_html_entities(raw).lines().enumerate() {
            let masked = (!strict)
                .then(|| if is_gen_docs { mask_generated_docs_html_tokens(source, l) } else { mask_homepage_html_phrases(source, l) })
                .flatten();
            check_single_string_with_extra(source, sl + off, masked.as_deref().unwrap_or(l), PrismPolicy::Strict, strict, out);
        }
    };
    while let Some(rest) = doc.get(pos..).filter(|s| !s.is_empty()) {
        if let Some(tag) = raw_close {
            let cp = find_html_raw_close_tag_start(doc, pos, tag);
            flush(line, doc.get(pos..cp.unwrap_or(doc.len())).unwrap_or(""), false, &mut out);
            let end = cp.map_or(doc.len(), |ts| doc.get(ts..).and_then(|s| s.find('>')).map_or(doc.len(), |g| ts + g + 1));
            (line, pos, raw_close) = (line + count_newlines(doc.get(pos..end).unwrap_or("").as_bytes()), end, None);
        } else if let Some(after) = rest.strip_prefix("<!--") {
            let end_rel = find_html_comment_end(after).unwrap_or(after.len());
            flush(line, after.get(..end_rel).unwrap_or(after).trim_end_matches("-->"), is_html, &mut out);
            let end = pos + 4 + end_rel;
            (line, pos) = (line + count_newlines(doc.get(pos..end).unwrap_or(rest).as_bytes()), end);
        } else if !is_html && let Some(after) = rest.strip_prefix("<![CDATA[") {
            let (body, end) = after.find("]]>").map_or((after, doc.len()), |r| (after.get(..r).unwrap_or(""), pos + 12 + r));
            if buf.is_empty() {
                text_start = line;
            }
            buf.push_str(body);
            (line, pos) = (line + count_newlines(doc.get(pos..end).unwrap_or(rest).as_bytes()), end);
        } else if rest.starts_with('<')
            && let Some(gt_rel) = find_html_tag_close(rest.get(1..).unwrap_or(""))
        {
            flush(text_start, &buf, depth > 0, &mut out);
            buf.clear();
            let (tag_end, trimmed) = (pos + gt_rel + 2, rest.get(1..1 + gt_rel).unwrap_or("").trim());
            let (is_close, self_closing) = (trimmed.starts_with('/'), !trimmed.starts_with('/') && trimmed.ends_with('/'));
            let local = scan_markup_tag_attrs(source, line, trimmed, is_html, is_gen_docs, &mut out);
            if is_html && !is_close && !self_closing && matches!(local.as_str(), "script" | "style") {
                raw_close = Some(if local == "script" { "script" } else { "style" });
            } else if matches!(local.as_str(), "title" | "desc") || (is_html && local == "metadata") {
                depth = if is_close { depth.saturating_sub(1) } else { depth + usize::from(!self_closing || is_html) };
            }
            (line, pos) = (line + count_newlines(doc.get(pos..tag_end).unwrap_or("").as_bytes()), tag_end);
        } else {
            let next_lt = rest.find('<').map_or(doc.len(), |r| pos + if r == 0 { rest.chars().next().map_or(1, char::len_utf8) } else { r });
            if buf.is_empty() {
                text_start = line;
            }
            let chunk = doc.get(pos..next_lt).unwrap_or("");
            buf.push_str(chunk);
            (line, pos) = (line + count_newlines(chunk.as_bytes()), next_lt);
        }
    }
    flush(text_start, &buf, depth > 0, &mut out);
    dedup_tail_hits(&mut out, 0);
    out
}

#[must_use]
pub fn check_svg_metadata(source: &str, svg: &str) -> Vec<BrandHit> {
    scan_markup_metadata(source, svg, false)
}
#[must_use]
pub fn check_html_metadata(source: &str, html: &str) -> Vec<BrandHit> {
    scan_markup_metadata(source, html, true)
}

fn decode_pdf_string(bytes: &[u8], pos: &mut usize, line: &mut usize) -> Option<String> {
    let raw: Vec<u8> = match *bytes.get(*pos)? {
        b'(' => {
            *pos += 1;
            let (mut depth, mut raw) = (1usize, Vec::new());
            while *pos < bytes.len() && depth > 0 {
                let b = bytes[*pos];
                *pos += 1;
                match b {
                    b'(' => {
                        depth += 1;
                        raw.push(b);
                    }
                    b')' => {
                        depth -= 1;
                        if depth > 0 {
                            raw.push(b);
                        }
                    }
                    b'\\' if *pos < bytes.len() => {
                        let e = bytes[*pos];
                        *pos += 1;
                        if e == b'\n' {
                            *line += 1;
                        } else {
                            raw.push(match e {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                o => o,
                            });
                        }
                    }
                    b'\n' => {
                        *line += 1;
                        raw.push(b);
                    }
                    o => raw.push(o),
                }
            }
            raw
        }
        b'<' if bytes.get(*pos + 1) != Some(&b'<') => {
            *pos += 1;
            let mut nibs = Vec::new();
            while *pos < bytes.len() && bytes[*pos] != b'>' {
                let b = bytes[*pos];
                (*pos, *line) = (*pos + 1, *line + usize::from(b == b'\n'));
                nibs.extend((b as char).to_digit(16).and_then(|n| u8::try_from(n).ok()));
            }
            *pos += usize::from(bytes.get(*pos) == Some(&b'>'));
            if nibs.len() % 2 == 1 {
                nibs.push(0);
            }
            nibs.as_chunks::<2>().0.iter().map(|&[h, l]| (h << 4) | l).collect()
        }
        _ => return None,
    };
    Some(raw.strip_prefix(&[0xFE, 0xFF]).map_or_else(|| String::from_utf8_lossy(&raw).into_owned(), |r| decode_utf16(r, false)))
}

pub(crate) fn check_pdf_metadata_checked(source: &str, pdf_bytes: &[u8]) -> Result<Vec<BrandHit>, String> {
    const KEYS: &str = "Creator Producer Title Author Subject Keywords";
    if !pdf_bytes[..pdf_bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        return Err(format!("cannot scan {source}: invalid PDF magic header (missing `%PDF-`)"));
    }
    let (mut out, mut line, mut pos) = (Vec::new(), 1usize, 0usize);
    while pos < pdf_bytes.len() {
        if pdf_bytes[pos] == b'\n' {
            (line, pos) = (line + 1, pos + 1);
        } else if pdf_bytes[pos] == b'/' {
            let start = pos + 1;
            pos = start;
            while pos < pdf_bytes.len()
                && !pdf_bytes[pos].is_ascii_whitespace()
                && !matches!(pdf_bytes[pos], b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
            {
                pos += 1;
            }
            if std::str::from_utf8(&pdf_bytes[start..pos]).is_ok_and(|k| words_has(KEYS, k)) {
                while pos < pdf_bytes.len() && matches!(pdf_bytes[pos], b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0C) {
                    (line, pos) = (line + usize::from(pdf_bytes[pos] == b'\n'), pos + 1);
                }
                let val_line = line;
                if let Some(val) = decode_pdf_string(pdf_bytes, &mut pos, &mut line) {
                    check_single_string_with_extra(source, val_line, &val, PrismPolicy::Strict, true, &mut out);
                }
            }
        } else if pdf_bytes.get(pos..pos + 10) == Some(b"<x:xmpmeta") || pdf_bytes.get(pos..pos + 9) == Some(b"<?xpacket") {
            let end = pdf_bytes[pos..].windows(9).position(|w| w == b"endstream").map_or(pdf_bytes.len(), |r| pos + r);
            scan_xml_text_fragment(source, line, &String::from_utf8_lossy(&pdf_bytes[pos..end]), false, &mut out);
            (line, pos) = (line + count_newlines(&pdf_bytes[pos..end]), end);
        } else {
            pos += 1;
        }
    }
    Ok(out)
}

#[must_use]
pub fn check_pdf_metadata(source: &str, pdf_bytes: &[u8]) -> Vec<BrandHit> {
    check_pdf_metadata_checked(source, pdf_bytes).unwrap_or_default()
}
#[must_use]
pub fn check_manifest_metadata(source: &str, manifest_json: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    match serde_json::from_str::<serde_json::Value>(manifest_json) {
        Ok(val) => visit_json_strings(source, &val, 1, &mut out),
        Err(_) => {
            manifest_json.lines().enumerate().for_each(|(i, l)| check_single_string_with_extra(source, i + 1, l, PrismPolicy::Strict, true, &mut out))
        }
    }
    out
}

fn visit_json_strings(source: &str, val: &serde_json::Value, idx_hint: usize, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::String(s) => check_single_string_with_extra(source, idx_hint, s, PrismPolicy::Strict, true, out),
        serde_json::Value::Array(items) => items.iter().enumerate().for_each(|(i, x)| visit_json_strings(source, x, i + 1, out)),
        serde_json::Value::Object(map) => map.iter().enumerate().for_each(|(i, (k, v))| {
            check_single_string_with_extra(source, i + 1, k, PrismPolicy::Strict, true, out);
            visit_json_strings(source, v, i + 1, out);
        }),
        _ => {}
    }
}

#[must_use]
pub fn check_window_title(source: &str, title: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    check_single_string_with_extra(source, 1, title, PrismPolicy::Strict, false, &mut out);
    out
}

#[derive(Clone, Copy)]
struct McpScanCtx<'a> {
    source: &'a str,
    method: &'a str,
    called_tool: Option<&'a str>,
    entry_tool: Option<&'a str>,
}

#[must_use]
pub fn check_mcp_rpc_response(source: &str, method: &str, called_tool: Option<&str>, response: &serde_json::Value) -> Vec<BrandHit> {
    let mut out = Vec::new();
    scan_mcp_json_value(McpScanCtx { source, method, called_tool, entry_tool: None }, "", response, &mut out);
    out
}

fn scan_mcp_json_value(ctx: McpScanCtx<'_>, ptr: &str, val: &serde_json::Value, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::Object(map) => {
            let src = format!("{}:{ptr}", ctx.source);
            let is_png = ctx.method == "tools/call"
                && map.get("type").and_then(serde_json::Value::as_str) == Some("image")
                && map.get("mimeType").and_then(serde_json::Value::as_str) == Some("image/png");
            let next_tool = (ctx.method == "tools/list" && ptr.starts_with("/result/tools/") && ctx.entry_tool.is_none())
                .then(|| map.get("name").and_then(serde_json::Value::as_str))
                .flatten()
                .or(ctx.entry_tool);
            let child = McpScanCtx { entry_tool: next_tool, ..ctx };
            for (i, (k, v)) in map.iter().enumerate() {
                check_single_string_with_extra(&src, i + 1, k, PrismPolicy::Strict, true, out);
                if !is_png {
                    scan_mcp_json_value(child, &format!("{ptr}/{k}"), v, out);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().enumerate().for_each(|(i, x)| scan_mcp_json_value(ctx, &format!("{ptr}/{i}"), x, out)),
        serde_json::Value::String(s) => scan_mcp_leaf_string(ctx, ptr, s, out),
        _ => {}
    }
}

fn scan_mcp_leaf_string(ctx: McpScanCtx<'_>, ptr: &str, text: &str, out: &mut Vec<BrandHit>) {
    if matches!(ctx.method, "tools/call" | "resources/read") && ptr.ends_with("/text") {
        if text.trim_start().starts_with(['{', '['])
            && let Ok(inner) = serde_json::from_str::<serde_json::Value>(text)
        {
            return scan_mcp_json_value(ctx, &format!("{ptr}#json"), &inner, out);
        }
        if ctx.called_tool == Some("export") && text.contains("<svg") {
            return out.extend(check_svg_metadata(&format!("{}:{ptr}#svg", ctx.source), text));
        }
    }
    let top_field = |f: &str| {
        ptr.strip_prefix("/result/tools/")
            .and_then(|r| r.split_once('/'))
            .is_some_and(|(i, t)| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()) && t == f)
    };
    if matches!(text, "table_import_prism" | "project_export_prism")
        && ((ctx.method == "tools/list" && ctx.entry_tool == Some(text) && top_field("name"))
            || (ctx.method == "tools/call"
                && matches!(ctx.called_tool, Some("list_toolsets" | "command_list"))
                && (ptr.ends_with("/name") || ptr.ends_with("/mcp_tool"))))
    {
        return;
    }
    let pol = if ctx.method == "tools/list"
        && matches!(ctx.entry_tool, Some("open_file" | "save_file" | "export"))
        && ptr.contains("/inputSchema/properties/format/enum/")
        && text == "prism"
    {
        PrismPolicy::AllowExactBarePrism
    } else if (ctx.method == "tools/list"
        && matches!(ctx.entry_tool, Some("table_import_prism" | "project_export_prism"))
        && top_field("description"))
        || (ctx.method == "tools/call"
            && matches!(ctx.called_tool, Some("list_toolsets" | "command_list" | "ui_inspect"))
            && (ptr.ends_with("/command_id") || ptr.ends_with("/id") || ptr.ends_with("/widget_id"))
            && (matches!(text, "table.importPrism" | "project.exportPrism")
                || text.ends_with(".table.importPrism")
                || text.ends_with(".project.exportPrism")))
    {
        PrismPolicy::UiTextRun { it_stat: false, cmd_id: true }
    } else {
        PrismPolicy::Strict
    };
    check_single_string_with_extra(&format!("{}:{ptr}", ctx.source), 1, text, pol, true, out);
}

pub fn scan_dist_artefacts(dist_dir: &Path) -> Result<Vec<BrandHit>, String> {
    Ok(scan_dist_artefacts_with_stats(dist_dir)?.1)
}
pub fn scan_dist_artefacts_with_stats(dist_dir: &Path) -> Result<(DistScanStats, Vec<BrandHit>), String> {
    scan_dir_files(dist_dir, None, true, true)
}
#[cfg(feature = "docs-site")]
fn scan_docs_dir_dedup(dir: &Path, prefix: &str) -> Result<Vec<BrandHit>, String> {
    let mut hits = scan_dir_files(dir, Some(prefix), false, false)?.1;
    hits.sort();
    hits.dedup();
    Ok(hits)
}
#[cfg(feature = "docs-site")]
pub fn check_docs_site_dir(dir: &Path) -> Result<Vec<BrandHit>, String> {
    scan_docs_dir_dedup(dir, "docs-site")
}
#[cfg(feature = "docs-site")]
pub fn check_manual_build_dir(dir: &Path) -> Result<Vec<BrandHit>, String> {
    scan_docs_dir_dedup(dir, "docs-site/manual")
}

fn scan_dir_files(
    dir: &Path,
    display_prefix: Option<&str>,
    include_adapted: bool,
    check_forbidden: bool,
) -> Result<(DistScanStats, Vec<BrandHit>), String> {
    if !dir.exists() {
        return Err(format!("directory `{}` does not exist", dir.display()));
    }
    let mut files = Vec::new();
    collect_dist_files(dir, dir, &mut files)?;
    files.sort();
    let (mut stats, mut hits) = (DistScanStats { files: files.len(), archive_entries: 0 }, Vec::new());
    for rel in files {
        let display = display_prefix.map_or_else(|| rel.clone(), |p| format!("{p}/{rel}"));
        if check_forbidden && is_forbidden_bundled_dir(&rel) {
            let brand = if is_corpus_path(&rel) { "corpus" } else { "prism" };
            hits.push(BrandHit { path: display, line: 1, brand });
            continue;
        }
        scan_strict_label_words(&display, &rel, include_adapted, &mut hits);
        let bytes =
            curvebook_platform::fs::read_regular_file_bytes(&dir.join(&rel), MAX_DIST_FILE_BYTES).map_err(|e| format!("read file {rel}: {e}"))?;
        scan_dist_bytes(&display, &bytes, 0, &mut stats, &mut hits)?;
    }
    Ok((stats, hits))
}

fn scan_strict_label_words(source: &str, text: &str, include_adapted: bool, hits: &mut Vec<BrandHit>) {
    let mut seen = BTreeSet::new();
    for words in [SUITE_BRANDS, BRANDS, PRE_RENAME_ASSET_BRANDS] {
        emit_words(source, text, words, PrismPolicy::Strict, |_| 1, &mut seen, hits);
    }
    if include_adapted {
        emit_words(source, text, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict, |_| 1, &mut seen, hits);
    }
}

fn is_native_binary_blob(display_path: &str, bytes: &[u8]) -> bool {
    const EXTS: &str = "exe dll so dylib wasm dmg msi appimage a lib";
    let is_magic = matches!(
        bytes.get(..4),
        Some(
            b"\x7FELF"
                | b"\x00asm"
                | b"\xFE\xED\xFA\xCE"
                | b"\xFE\xED\xFA\xCF"
                | b"\xCE\xFA\xED\xFE"
                | b"\xCF\xFA\xED\xFE"
                | b"\xCA\xFE\xBA\xBE"
                | b"\xBE\xBA\xFE\xCA"
        )
    ) || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"!<arch>\n");
    let is_ticket = bytes.starts_with(b"s8ch") && display_path.replace('\\', "/").ends_with(".app/Contents/CodeResources");
    words_has(EXTS, &lower_ext(display_path)) || is_ticket || is_magic
}

fn scan_zip_container_bytes(
    display_path: &str,
    bytes: &[u8],
    depth: usize,
    stats: &mut DistScanStats,
    hits: &mut Vec<BrandHit>,
) -> Result<(), String> {
    if depth >= 2 {
        return Err(format!("{display_path}: nested archive depth exceeded (max depth 2)"));
    }
    let zip = ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()).map_err(|e| format!("cannot inspect archive `{display_path}`: {e}"))?;
    if !zip.archive_comment().is_empty() {
        scan_strict_label_words(&format!("{display_path}#comment"), &String::from_utf8_lossy(zip.archive_comment()), true, hits);
    }
    for (name, comment) in zip.entry_comments() {
        scan_strict_label_words(&format!("{display_path}!{name}#comment"), &String::from_utf8_lossy(comment), true, hits);
    }
    for (name, entry_bytes) in zip.iter() {
        stats.archive_entries += 1;
        let entry_path = format!("{display_path}!{name}");
        if is_forbidden_bundled_dir(name) {
            let brand = if is_corpus_path(name) { "corpus" } else { "prism" };
            hits.push(BrandHit { path: entry_path, line: 1, brand });
            continue;
        }
        scan_strict_label_words(&entry_path, name, true, hits);
        scan_dist_bytes(&entry_path, entry_bytes, depth + 1, stats, hits)?;
    }
    Ok(())
}

fn scan_dist_bytes(display_path: &str, bytes: &[u8], depth: usize, stats: &mut DistScanStats, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let ext = lower_ext(display_path);
    let scanned_doc = try_scan_doc_binary_metadata(display_path, &ext, bytes, true, hits)?;
    let is_zip = looks_like_zip(bytes) || ZIP_CONTAINER_EXTS.contains(&ext.as_str());
    if is_zip {
        scan_zip_container_bytes(display_path, bytes, depth, stats, hits)?;
    }
    if scanned_doc || is_zip {
        return Ok(());
    }
    let text = match decode_text_file(display_path, bytes) {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(()),
        Err(err) => return if is_native_binary_blob(display_path, bytes) { Ok(()) } else { Err(err) },
    };
    let from = hits.len();
    scan_suite_lines_with_scope(display_path, &text, true, hits);
    if !scan_common_surface_text(display_path, &ext, &text, true, hits) {
        scan_text_lines(display_path, &text, true, false, hits);
    }
    dedup_tail_hits(hits, from);
    Ok(())
}

#[must_use]
pub(crate) fn scan_dist_label(source: &str, label: &str) -> Vec<BrandHit> {
    let mut hits = Vec::new();
    scan_strict_label_words(source, label, true, &mut hits);
    hits
}

#[must_use]
pub(crate) fn scan_compliance_text(display_path: &str, text: &str) -> Vec<BrandHit> {
    let mut hits = scan_dist_label(display_path, display_path);
    let from = hits.len();
    scan_suite_lines_with_scope(display_path, text, true, &mut hits);
    scan_text_lines(display_path, text, false, false, &mut hits);
    if matches!(lower_ext(display_path).as_str(), "html" | "htm") {
        hits.extend(check_html_metadata(display_path, text));
    }
    dedup_tail_hits(&mut hits, from);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::{TestDir, to_utf16_bytes};

    type RustCase = (&'static str, &'static str, &'static [(usize, &'static str)]);
    fn scan(path: &str, text: &str) -> Vec<BrandHit> {
        let mut hits = Vec::new();
        scan_file(path, text, &mut hits);
        hits
    }
    fn hit(path: &str, line: usize, brand: &'static str) -> BrandHit {
        BrandHit { path: path.to_owned(), line, brand }
    }

    #[test]
    fn upstream_provenance_and_rust_packaging_rules() {
        let (story, vc, vc_title) = (["sto", "ry", "told"].concat(), ["vec", "tor", "craft"].concat(), ["Vec", "tor", "Cra", "ft"].concat());
        let url = format!("https://github.com/{story}/{vc}");
        for (p, b) in [
            ("ATTRIBUTION.md", format!("## Attributed Source Code\nFrom {vc_title} (`{url}`).\n")),
            ("NOTICE", format!("## Attributed Source Code\nCopyright {vc_title} ({url}).\n")),
            ("AGENTS.md", format!("Attributed {vc_title}.\n")),
        ] {
            assert!(scan(p, &b).is_empty(), "{p}");
        }
        for (p, b) in [
            ("ATTRIBUTION.md", format!("## Other\n{url}\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\n{url}/blob/main/lib.rs\n")),
            ("NOTICE", format!("Preamble {url}\n")),
            ("README.md", format!("Upstream {url} <!-- brand-ok -->\n")),
        ] {
            assert!(scan(p, &b).iter().any(|h| h.brand == "storytold"), "{p}");
        }
        assert!(scan("crates/ui-egui/src/w.rs", &format!("pub fn {vc}_helper() {{}}\n")).iter().any(|h| h.brand == "vectorcraft"));
        let rust_cases: &[RustCase] = &[
            (
                "crates/ui-egui/src/lib.rs",
                "// GraphPad\npub const L: &str = \"OriginPro\"; // brand-ok\npub struct PrismStyle;\n#[cfg(test)] mod t { const T: &str = \"SPSS\"; }\n",
                &[(1, "graphpad"), (2, "originpro"), (3, "prism"), (4, "spss")],
            ),
            (
                "crates/format/src/registry.rs",
                "pub fn ext(e: &str) -> &'static str { match e { \"prism\" | \"pzfx\" => \"x.prism\", _ => \"\" } }\npub const BAD: &str = \"Prism Archive\";\n",
                &[(2, "prism")],
            ),
            ("crates/ui-egui/src/i18n.rs", "it: \"statistica\",\nde: \"Statistica\",\n", &[(2, "statistica")]),
            ("crates/fit/src/dsl.rs", "struct C { jmp_end: usize }\nconst BAD: &str = \"jmp\";\n", &[(2, "jmp")]),
        ];
        for &(p, src, exp) in rust_cases {
            assert_eq!(scan(p, src), exp.iter().map(|&(l, b)| hit(p, l, b)).collect::<Vec<_>>(), "{p}");
        }
        assert!(scan("README.md", "Not affiliated with ArtCraft, Dotmatics or GraphPad. <!-- brand-ok -->\n").is_empty());
        assert!(!scan("README.md", "Not affiliated with OriginPro. <!-- brand-ok -->\n").is_empty());
        assert_eq!(
            scan(
                "packaging/linux/cb.desktop",
                "[Desktop Entry]\nMimeType=application/x-prism-project;application/x-prism-xml;\nKeywords=GraphPad;prism;\n"
            ),
            vec![hit("packaging/linux/cb.desktop", 3, "graphpad"), hit("packaging/linux/cb.desktop", 3, "prism")]
        );
        let th = scan(
            "crates/demo/Cargo.toml",
            "[package]\ndescription = \"Unlike GraphPad\"\n[package.metadata.prism]\nenabled = true\n[dependencies]\n# GraphPad\nserde = \"1\"\n",
        );
        assert!(th.len() == 2 && th.iter().any(|h| h.brand == "graphpad") && th.iter().any(|h| h.brand == "prism"));
    }

    #[test]
    fn surface_metadata_mcp_and_dist_scanners() {
        assert_eq!(
            check_i18n_catalog_entries([("k1", "it", "statistica"), ("k2", "fr", "GraphPad"), ("k3", "es", "Prism (*.prism)")]),
            vec![hit("i18n:fr:k2", 1, "graphpad"), hit("i18n:es:k3", 1, "prism")]
        );
        assert_eq!(check_text_runs("ui", ["Import .prism", "Open Prism"]), vec![hit("ui", 2, "prism")]);
        assert_eq!(check_text_runs_strict("ui", ["Import .prism", "Import Prism"]), vec![hit("ui", 2, "prism")]);
        assert_eq!(check_labeled_text_runs([("n1", "*.prism"), ("n2", "OriginPro")]), vec![hit("n2", 2, "originpro")]);
        assert_eq!(check_window_title("win", "CurveBook (prism)"), vec![hit("win", 1, "prism")]);
        assert!(
            check_svg_metadata("ok.svg", "<svg><title>optical prism</title></svg>").is_empty()
                && !check_svg_metadata("bad.svg", "<svg><title>Prism Studio</title></svg>").is_empty()
        );
        assert!(check_pdf_metadata("ok.pdf", b"%PDF-1.4\n<< /Creator (CurveBook) >>\n").is_empty());
        assert_eq!(
            check_pdf_metadata("bad.pdf", b"%PDF-1.4\n<< /Creator (Prism) /Title <FEFF00470072006100700068005000610064> >>\n"),
            vec![hit("bad.pdf", 2, "prism"), hit("bad.pdf", 2, "graphpad")]
        );
        assert!(
            check_html_metadata("ok.html", "<html><head><title>Open .prism</title></head></html>").is_empty()
                && !check_html_metadata("bad.html", "<html><head><title>Prism</title><meta name=author content=OriginLab></head></html>").is_empty()
        );
        assert!(!check_manifest_metadata("m.json", r#"{"generator":"GraphPad"}"#).is_empty());
        let clean = serde_json::json!({"result":{"tools":[{"name":"open_file","description":"Open .prism","inputSchema":{"properties":{"format":{"enum":["curvebook","prism"]}}}},{"name":"table_import_prism","description":"[table.importPrism] Import"}]}});
        let dirty = serde_json::json!({"result":{"tools":[{"name":"doc_inspect","description":"GraphPad Prism"}]}});
        assert!(
            check_mcp_rpc_response("tools", "tools/list", None, &clean).is_empty()
                && check_mcp_rpc_response("tools", "tools/list", None, &dirty).len() == 2
        );

        let dir = TestDir::new("brands-repo-dist");
        dir.write_bytes("packaging/windows/app.rc", &to_utf16_bytes("VALUE \"Product\", \"GraphPad\"\n", true, true));
        dir.write("crates/format/src/prism.rs", "// .prism\n");
        dir.write("crates/ui-egui/src/PrismStyle.rs", "pub struct X;\n");
        let files: Vec<_> = ["packaging/windows/app.rc", "crates/format/src/prism.rs", "crates/ui-egui/src/PrismStyle.rs"].map(String::from).into();
        assert_eq!(
            scan_repo_checked(dir.path(), &files).unwrap(),
            vec![hit("packaging/windows/app.rc", 1, "graphpad"), hit("crates/ui-egui/src/PrismStyle.rs", 1, "prism")]
        );
        let mut zw = crate::zip_read::TestZipWriter::new();
        zw.add_file("meta.txt", b"OriginPro".to_vec()).unwrap();
        dir.write_bytes("dist/book.zip", &zw.finish().unwrap());
        dir.write("dist/docs/audit/leak.md", "x");
        let dh = scan_dist_artefacts(&dir.join("dist")).unwrap();
        assert!(dh.iter().any(|h| h.brand == "originpro") && dh.iter().any(|h| h.path == "docs/audit/leak.md"));
        assert!(skip_path("xtask/src/brands.rs") && !skip_path("docs/audit/README.md"));
    }

    #[test]
    fn interop_corpus_allowlist_and_corpus_bundle_gate() {
        let mut ok_hits = Vec::new();
        super::scan_file("xtask/src/interop_corpus.rs", "pub(crate) const DEFAULT_CORPUS_DIR: &str = \"corpus/third-party-prism\";\n", &mut ok_hits);
        assert!(ok_hits.is_empty(), "exact default corpus dir constant must be allowed in interop_corpus.rs: {ok_hits:?}");

        let mut other_file_hits = Vec::new();
        super::scan_file("xtask/src/other.rs", "const DIR: &str = \"corpus/third-party-prism\";\n", &mut other_file_hits);
        assert!(other_file_hits.iter().any(|h| h.brand == "prism"), "other file must reject third-party-prism: {other_file_hits:?}");

        let mut extra_hits = Vec::new();
        super::scan_file(
            "xtask/src/interop_corpus.rs",
            "const BAD1: &str = \"corpus/third-party-prism/sub\";\nconst BAD2: &str = \"Prism\";\n",
            &mut extra_hits,
        );
        assert_eq!(extra_hits.len(), 2, "non-exact tokens in interop_corpus.rs must still fail: {extra_hits:?}");

        assert!(super::is_corpus_path("corpus"));
        assert!(super::is_corpus_path("corpus/sample.pzfx"));
        assert!(super::is_corpus_path("bundle/corpus/sample.pzfx"));
        assert!(!super::is_corpus_path("xtask/src/corpus.rs"));
        assert!(!super::is_corpus_path("crates/testkit/src/nist_strd/corpus.rs"));
        assert!(super::is_forbidden_bundled_dir("dist/app/corpus/sample.pzfx"));

        let dist_dir = TestDir::new("brands-corpus-dist");
        dist_dir.write("app/corpus/data.txt", "clean");
        let mut zw = crate::zip_read::TestZipWriter::new();
        zw.add_file("corpus/data.txt", b"clean".to_vec()).unwrap();
        dist_dir.write_bytes("pkg.zip", &zw.finish().unwrap());
        let dist_hits = super::scan_dist_artefacts(dist_dir.path()).unwrap();
        assert!(dist_hits.iter().any(|h| h.path == "app/corpus/data.txt" && h.brand == "corpus"), "{dist_hits:?}");
        assert!(dist_hits.iter().any(|h| h.path == "pkg.zip!corpus/data.txt" && h.brand == "corpus"), "{dist_hits:?}");

        let repo = TestDir::git_repo("brands-corpus-untracked");
        repo.write(".gitignore", "/corpus\n");
        repo.write("docs/clean.md", "# Clean CurveBook note\n");
        repo.git_add_all();
        repo.write("corpus/third-party-prism/vendor.pzfx", "GraphPad Prism");
        assert!(super::run(repo.path()).is_ok(), "untracked local corpus/ files must not fail brands gate");
    }

    #[test]
    fn pzfx_export_bare_root_element_allowlist_stays_strict_everywhere_else() {
        let root = super::PZFX_VENDOR_ROOT_ELEMENT;
        let gp = ["Graph", "Pad"].concat();
        let copy = ["Copy", "right (", "c)"].concat();

        let ok_src = format!("pub(crate) const PZFX_ROOT_ELEMENT: &str = \"{root}\";\n");
        assert!(scan("crates/format/src/pzfx/export.rs", &ok_src).is_empty());

        let doc_md = format!("Root element: `{root}`\n");
        let fixture_xml = format!("<{root} PrismXMLVersion=\"5.00\"/>\n");
        for (other_path, body) in [
            ("crates/format/src/pzfx/import.rs", ok_src.as_str()),
            ("crates/format/src/pzfx/mod.rs", ok_src.as_str()),
            ("crates/format/tests/format_suite/pzfx.rs", ok_src.as_str()),
            ("docs/file-formats.md", doc_md.as_str()),
            ("fixtures/sample.pzfx", fixture_xml.as_str()),
        ] {
            let hits = scan(other_path, body);
            assert!(hits.iter().any(|h| h.brand == "graphpad"), "expected vendor root-element rejection in {other_path}, got {hits:?}");
        }

        let bad_export_src = format!(
            "const OPEN_TAG: &str = \"<{root}>\";\n\
             const NON_EXACT: &str = \"{root}Extra\";\n\
             const OTHER_VENDOR: &str = \"Dotmatics\";\n\
             const NOTICE_LINE: &str = \"{copy} 2026 {gp} Software\";\n\
             const GRAPHPAD_ROOT: &str = \"ok\";\n\
             // {root} comment\n"
        );
        let export_hits = scan("crates/format/src/pzfx/export.rs", &bad_export_src);
        for line in 1..=6 {
            assert!(export_hits.iter().any(|h| h.line == line), "expected rejection on line {line} in pzfx/export.rs, got {export_hits:?}");
        }

        let dist = TestDir::new("brands-pzfx-root-dist");
        dist.write("docs/readme.txt", &format!("{root}\n"));
        dist.write("docs-site/guide.html", &format!("<p>{root}</p>\n"));
        let mut zw = crate::zip_read::TestZipWriter::new();
        zw.add_file("meta.txt", root.as_bytes().to_vec()).unwrap();
        dist.write_bytes("bundle.zip", &zw.finish().unwrap());
        let mut elf = b"\x7FELF\x00\x00".to_vec();
        elf.extend_from_slice(root.as_bytes());
        elf.push(0);
        dist.write_bytes("bin/curvebook", &elf);
        let dist_hits = super::scan_dist_artefacts(dist.path()).unwrap();
        assert!(dist_hits.iter().any(|h| h.path == "docs/readme.txt" && h.brand == "graphpad"), "{dist_hits:?}");
        assert!(dist_hits.iter().any(|h| h.path == "docs-site/guide.html" && h.brand == "graphpad"), "{dist_hits:?}");
        assert!(dist_hits.iter().any(|h| h.path == "bundle.zip!meta.txt" && h.brand == "graphpad"), "{dist_hits:?}");
        assert!(!dist_hits.iter().any(|h| h.path == "bin/curvebook"), "native binary should not fail brands: {dist_hits:?}");
    }

    #[test]
    fn prism_module_split_export_prism_allowlists_and_wxs_scanner() {
        for ok_path in [
            "crates/format/src/prism.rs",
            "crates/format/src/prism/mod.rs",
            "crates/format/src/prism/import.rs",
            "crates/format/src/prism/export.rs",
            "crates/format/src/prism/schema.rs",
        ] {
            let mut hits = Vec::new();
            super::scan_tracked_path_name(ok_path, &mut hits);
            assert!(hits.is_empty(), "expected clean module path {ok_path}, got {hits:?}");
        }
        for bad_path in [
            "crates/format/src/prism/prism_style.rs",
            "crates/format/src/prism/graphpad.rs",
            "crates/format/src/prism/sub/mod.rs",
            "crates/ui-egui/src/prism/mod.rs",
        ] {
            let mut hits = Vec::new();
            super::scan_tracked_path_name(bad_path, &mut hits);
            assert!(!hits.is_empty(), "expected rejection for {bad_path}");
        }

        let export_rust = "\
            // Routes project.exportPrism through export_prism_bytes and ExportPrismParams via run_project_export_prism.\n\
            pub struct ExportPrismParams;\n\
            pub fn run_project_export_prism() -> &'static str { \"project.exportPrism\" }\n\
            pub fn export_prism_bytes() -> &'static str { exporter_by_format(\"prism\") }\n\
            pub const CMD_KEY: &str = \"cmd.project.exportPrism\";\n\
            pub const TEST_OP: &str = \"export_prism_bytes failed\";\n";
        assert!(scan("crates/format/src/prism/export.rs", export_rust).is_empty(), "canonical exportPrism Rust code must pass");

        let bad_export_rust = "\
            pub struct PrismStyle;\n\
            pub const BAD_CMD: &str = \"bad.exportPrism\";\n\
            pub const BAD_PROSE: &str = \"Export to Prism format\";\n";
        let bad_rust_hits = scan("crates/format/src/prism/export.rs", bad_export_rust);
        assert_eq!(bad_rust_hits.len(), 3, "non-canonical prism tokens in prism/export.rs must fail: {bad_rust_hits:?}");

        let tsv = "key\ten\tde\ncmd.project.exportPrism\tExport .prism file\t.prism-Datei exportieren\ncmd.bad\tExport Prism file\tPrism Datei\n";
        let tsv_hits = scan("crates/ui-egui/locales/catalog.tsv", tsv);
        assert_eq!(tsv_hits, vec![hit("crates/ui-egui/locales/catalog.tsv", 3, "prism")]);

        assert!(
            scan("docs/commands.md", "### `project.exportPrism`\nImplemented in `crates/format/src/prism/export.rs` via `export_prism_bytes`.\n")
                .is_empty()
        );
        assert!(
            !scan("docs/commands.md", "Implemented in `crates/format/src/prism/prism_style.rs`.\n").is_empty(),
            "markdown must still reject contaminated leaf inside crates/format/src/prism/"
        );
        let ok_html = "<!doctype html><html><body><p><a href=\"#cmd-project-exportprism\">project.exportPrism</a> <a href=\"#mcp-tool-project_export_prism\">project_export_prism</a> in <a href=\"https://github.com/HauserGroup/curvebook/blob/main/crates/format/src/prism/export.rs\">export</a> via <code>export_prism_bytes</code>.</p></body></html>\n";
        assert!(scan("docs-site/commands.html", ok_html).is_empty());
        assert!(!scan("docs-site/commands.html", "<p>Export Prism file from crates/format/src/prism/prism_style.rs</p>\n").is_empty());

        let mcp_ok_list: serde_json::Value = serde_json::json!({
            "result": {
                "tools": [
                    {
                        "name": "save_file",
                        "description": "Save or export a .curvebook, .pzfx, or .prism file.",
                        "inputSchema": {"type": "object", "properties": {"format": {"type": "string", "enum": ["curvebook", "pzfx", "prism"]}}}
                    },
                    {
                        "name": "export",
                        "description": "Export data as .pzfx or .prism.",
                        "inputSchema": {"type": "object", "properties": {"format": {"type": "string", "enum": ["pzfx", "prism"]}}}
                    },
                    {
                        "name": "project_export_prism",
                        "description": "[project.exportPrism] Export .prism file.",
                        "inputSchema": {"type": "object"}
                    }
                ]
            }
        });
        let mut mcp_ok_hits = Vec::new();
        super::scan_mcp_json_value(
            super::McpScanCtx { source: "mcp:tools/list", method: "tools/list", called_tool: None, entry_tool: None },
            "",
            &mcp_ok_list,
            &mut mcp_ok_hits,
        );
        assert!(mcp_ok_hits.is_empty(), "expected clean MCP export surfaces, got {mcp_ok_hits:?}");

        let mcp_ok_call: serde_json::Value = serde_json::json!({
            "result": {
                "commands": [
                    {"command_id": "project.exportPrism", "mcp_tool": "project_export_prism", "widget_id": "cmd.project.exportPrism"}
                ]
            }
        });
        let mut mcp_call_hits = Vec::new();
        super::scan_mcp_json_value(
            super::McpScanCtx { source: "mcp:command_list", method: "tools/call", called_tool: Some("command_list"), entry_tool: None },
            "",
            &mcp_ok_call,
            &mut mcp_call_hits,
        );
        assert!(mcp_call_hits.is_empty(), "expected clean MCP command_list exportPrism entry, got {mcp_call_hits:?}");

        let mcp_bad_list: serde_json::Value = serde_json::json!({
            "result": {
                "tools": [
                    {
                        "name": "export",
                        "description": "Export a Prism project file.",
                        "inputSchema": {"type": "object", "properties": {"format": {"type": "string", "enum": ["prism"]}}}
                    }
                ]
            }
        });
        let mut mcp_bad_hits = Vec::new();
        super::scan_mcp_json_value(
            super::McpScanCtx { source: "mcp:tools/list", method: "tools/list", called_tool: None, entry_tool: None },
            "",
            &mcp_bad_list,
            &mut mcp_bad_hits,
        );
        assert!(mcp_bad_hits.iter().any(|h| h.brand == "prism"), "MCP export description prose must stay strict: {mcp_bad_hits:?}");

        let ok_wxs = "\
            <Wix>\n\
              <ProgId Id=\"CurveBook.Pzfx\" Description=\"CurveBook .pzfx file\">\n\
                <Extension Id=\"pzfx\" ContentType=\"application/x-prism-xml\">\n\
                  <Verb Id=\"open\" Command=\"Open\" TargetFile=\"CurveBookExe\" Argument=\"&quot;%1&quot;\" />\n\
                </Extension>\n\
              </ProgId>\n\
              <ProgId Id=\"CurveBook.Archive\" Description=\"CurveBook .prism file\">\n\
                <Extension Id=\"prism\" ContentType=\"application/x-prism-project\">\n\
                  <Verb Id=\"open\" Command=\"Open\" TargetFile=\"CurveBookExe\" Argument=\"&quot;%1&quot;\" />\n\
                </Extension>\n\
              </ProgId>\n\
            </Wix>\n";
        assert!(scan("packaging/windows/curvebook.wxs", ok_wxs).is_empty(), "valid .wxs extension registration must pass");

        let mut wxs_dist_hits = Vec::new();
        super::scan_dist_bytes("installer/curvebook.wxs", ok_wxs.as_bytes(), 0, &mut super::DistScanStats::default(), &mut wxs_dist_hits).unwrap();
        assert!(wxs_dist_hits.is_empty(), "valid dist .wxs must pass: {wxs_dist_hits:?}");

        let bad_wxs = "\
            <Wix>\n\
              <ProgId Id=\"CurveBook.Prism\" Description=\"Prism Project\">\n\
                <Extension Id=\"Prism\">\n\
                <Extension Id=\"prism\" Description=\"GraphPad Prism\">\n\
                <!-- <Extension Id=\"prism\"> -->\n\
                <SummaryInformation Description=\"LumenCraft\" />\n\
              </ProgId>\n\
            </Wix>\n";
        let bad_wxs_hits = scan("packaging/windows/curvebook.wxs", bad_wxs);
        for line in 2..=6 {
            assert!(bad_wxs_hits.iter().any(|h| h.line == line), "expected .wxs rejection on line {line}, got {bad_wxs_hits:?}");
        }
    }

    #[test]
    fn homepage_migration_heading_and_disclaimer_allowlist() {
        let ok_index = "\
            <!doctype html><html><head><title>CurveBook</title></head><body>\n\
            <main><h2>Coming from GraphPad Prism?</h2><ul><li><strong>A GraphPad Prism Pro license costs $720 per year<a href=\"#trademark-disclaimer\">*</a>.</strong></li></ul></main>\n\
            <footer><p id=\"trademark-disclaimer\">*GraphPad Prism Pro annual pricing as listed in the EU on November 27, 2026. CurveBook is not affiliated with GraphPad Prism, GraphPad Software, or Dotmatics.</p></footer>\n\
            </body></html>\n";
        assert!(scan("docs/index.html", ok_index).is_empty(), "allowed homepage phrases must pass");

        for (path, bad_html) in [
            ("docs/index.html", "<html><head><title>Coming from GraphPad Prism</title></head></html>\n"),
            ("docs/index.html", "<html><head><meta name=\"description\" content=\"Coming from GraphPad Prism\"></head></html>\n"),
            ("docs/index.html", "<html><body><!-- Coming from GraphPad Prism --></body></html>\n"),
            ("docs/index.html", "<html><body><p>Better than GraphPad Prism</p></body></html>\n"),
            ("docs/index.html", "<html><body><p>Coming from GraphPad Prism and OriginPro</p></body></html>\n"),
            ("docs/faq.html", "<html><body><h2>Coming from GraphPad Prism?</h2></body></html>\n"),
            ("docs/open-pzfx-files.html", "<html><body><h2>Coming from GraphPad Prism?</h2></body></html>\n"),
            ("docs-site/index.html", "<html><body><h2>Coming from GraphPad Prism?</h2></body></html>\n"),
        ] {
            let hits = scan(path, bad_html);
            assert!(!hits.is_empty(), "expected rejection for {path}: {bad_html}");
        }
    }
}
