//! Generated AI-discoverability plain-Markdown maps (`R12.10`): `docs/llms.txt` and `docs/llms-full.txt`.
//!
//! Both files are pure, deterministic functions of the repository's published metadata and
//! hand-written user reference guides (`docs/reference/*.md`, `docs/reference/automation/*.md`,
//! `docs/mcp-for-beginners.md`). They contain zero timestamps and never include internal
//! contributor/planning documents (`ROADMAP.md`, `STATUS.md`, `audit/`, `planning/`, etc.).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use curvebook_platform::write_atomic;

use super::seo::{SITE_BASE_URL, manual_canonical_url, published_url};
use super::sitemap::MANUAL_GUIDES;
use crate::scan_util::{is_regular_file, normalize_newlines};

/// Primary one-paragraph summary matching `docs/index.html`.
pub const SITE_SUMMARY: &str = "CurveBook is a free, open-source biostatistics, nonlinear regression, and scientific graphing application for macOS, Windows, and Linux. It runs offline with zero telemetry and opens .pzfx and .prism data files.";

const GITHUB_REPO_URL: &str = "https://github.com/HauserGroup/curvebook";

const KEY_PAGES: &[(&str, &str, &str)] = &[
    ("CurveBook Home", "index.html", "Overview, workspace screenshots, core pillars, and Beta release downloads."),
    (
        "Open .pzfx and .prism Files",
        "open-pzfx-files.html",
        "Open .pzfx and .prism data files offline on macOS, Windows, and Linux and export tables to CSV or CurveBook projects.",
    ),
    (
        "CurveBook on Linux",
        "linux.html",
        "Native pure-Rust biostatistics and nonlinear curve fitting AppImage for Linux (x86_64 and aarch64), Wayland/X11, headless CLI, and local MCP.",
    ),
    (
        "Frequently Asked Questions",
        "faq.html",
        "Quick facts and direct answers on licensing, file formats, offline privacy, platforms, statistical methods, and MCP automation.",
    ),
    ("Privacy Policy", "privacy.html", "Local-first architecture, zero telemetry, no outbound network connections, and no auto-updater."),
    (
        "Full Reference Compilation (llms-full.txt)",
        "llms-full.txt",
        "Single-file plain-Markdown compilation of the CurveBook summary, quick facts, and all hand-written reference guides.",
    ),
];

/// Generated typed registry pages in the documentation manual (mirroring `sitemap.rs`).
const MANUAL_REGISTRY_PAGES: &[(&str, &str, &str)] = &[
    (
        "registry/tables.html",
        "Tables, Analyses & Error Codes Registry",
        "Live typed registry of table archetypes, subcolumn modes, analysis kinds, and engine error codes.",
    ),
    (
        "registry/equations.html",
        "Parametric Equations & Fitting Registry",
        "Live typed registry of built-in nonlinear regression equations, parameters, and constraints.",
    ),
    (
        "registry/graphs.html",
        "Graphs, Scales & Palettes Registry",
        "Live typed registry of graph kinds, axis scales, error bar options, palettes, and layout features.",
    ),
    (
        "registry/formats.html",
        "Formats & Schemas Registry",
        "Live typed registry of supported import/export formats, archive schemas, and user preference keys.",
    ),
    ("registry/commands.html", "Engine & UI Commands Registry", "Live typed registry of CurveBook engine commands and desktop UI actions."),
    (
        "registry/mcp-tools.html",
        "MCP Tools, Resources & Prompts Registry",
        "Live typed registry of CurveBook Model Context Protocol tools, resource URI schemes, and prompts.",
    ),
    ("registry/cli.html", "CLI Subcommands Registry", "Live typed registry of headless curvebook-cli subcommands and flags."),
];

const QUICK_FACTS: &[(&str, &str)] = &[
    ("License", "Dual-licensed under MIT OR Apache-2.0"),
    ("Platforms", "macOS, Windows, Linux (plus FreeBSD preview)"),
    ("Implementation", "Pure Rust, independently implemented from peer-reviewed scientific literature"),
    ("Privacy & network", "Offline-first, zero telemetry, no outbound network connections, no auto-updater"),
    ("Formats read", "`.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.pzfx`, `.prism`, `.csv`/`.tsv`, `.xlsx`"),
    ("Formats written", "`.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.csv`/`.tsv`, `SVG`, `PDF`, `PNG`, `TIFF`, `EMF`"),
    ("Interfaces", "Desktop GUI (`curvebook`), headless CLI (`curvebook-cli`), local stdio MCP server (`curvebook-cli mcp`)"),
    ("Source code", GITHUB_REPO_URL),
];

/// Build the concise `docs/llms.txt` Markdown map following the `llms.txt` convention.
pub fn build_llms_txt(_root: &Path) -> Result<String, String> {
    let mut out = String::with_capacity(4096);
    let _ = writeln!(out, "# CurveBook\n\n> {SITE_SUMMARY}\n");
    out.push_str("CurveBook is written in pure Rust and dual-licensed under MIT OR Apache-2.0. It provides an offline desktop GUI, deterministic headless CLI (`curvebook-cli`), and local stdio Model Context Protocol (MCP) server (`curvebook-cli mcp`).\n\n## Key Pages\n\n");
    for &(label, rel, desc) in KEY_PAGES {
        let _ = writeln!(out, "- [{label}]({}): {desc}", published_url(rel));
    }
    let _ = writeln!(out, "- [Source Code & Releases]({GITHUB_REPO_URL}): GitHub repository, issue tracker, and release packages.");

    out.push_str("\n## Documentation Manual\n\n");
    let _ = writeln!(
        out,
        "- [CurveBook Reference Manual]({}): Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
        manual_canonical_url("index.html")
    );
    for g in MANUAL_GUIDES {
        let _ = writeln!(out, "- [{}]({}): {}", g.title, manual_canonical_url(g.rel_path), g.description);
    }
    for &(rel_path, title, desc) in MANUAL_REGISTRY_PAGES {
        let _ = writeln!(out, "- [{title}]({}): {desc}", manual_canonical_url(rel_path));
    }
    let _ = writeln!(
        out,
        "- [A–Z Reference Index]({}): Alphabetical index of CurveBook analyses, equations, graph kinds, tables, commands, MCP tools, CLI flags, formats, and preferences.",
        manual_canonical_url("az-index.html")
    );
    Ok(out)
}

/// Build the deterministic, chrome-free plain-Markdown `docs/llms-full.txt` reference bundle.
pub fn build_llms_full_txt(root: &Path) -> Result<String, String> {
    let mut out = String::with_capacity(256 * 1024);
    let _ = writeln!(out, "# CurveBook — Full Reference Documentation\n\n> {SITE_SUMMARY}\n");
    let _ = writeln!(out, "- Website: {SITE_BASE_URL}");
    let _ = writeln!(out, "- Online Manual: {}", manual_canonical_url("index.html"));
    let _ = writeln!(out, "- Concise Index: {}", published_url("llms.txt"));
    let _ = writeln!(out, "- Source Code: {GITHUB_REPO_URL}\n");

    out.push_str("## Quick Facts\n\n| Property | Details |\n| --- | --- |\n");
    for &(key, val) in QUICK_FACTS {
        let _ = writeln!(out, "| {key} | {val} |");
    }

    for g in MANUAL_GUIDES {
        let full_path = root.join(g.repo_path);
        if !is_regular_file(&full_path) {
            continue;
        }
        let raw = std::fs::read_to_string(&full_path).map_err(|e| format!("read {}: {e}", g.repo_path))?;
        let normalized = strip_internal_doc_links(&normalize_newlines(&raw));
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n---\n\nCanonical URL: {}\n\n{trimmed}", manual_canonical_url(g.rel_path));
    }
    Ok(out)
}

const INTERNAL_TARGETS: &[&str] = &["roadmap.md", "status.md", "audit/", "planning/", "evidence/", "history/"];

fn is_internal_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    INTERNAL_TARGETS.iter().any(|pat| lower.contains(pat))
}

fn normalize_ref_label(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

fn parse_fence_delim(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|&b| b == first as u8).count();
    (count >= 3).then_some((first, count))
}

fn parse_ref_def_line(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    let after_open = trimmed.strip_prefix('[')?;
    let (label, after_colon) = after_open.split_once("]:")?;
    if label.trim().is_empty() || label.contains(['[', ']']) {
        return None;
    }
    let rest = after_colon.trim_start();
    let dest = if let Some(after_lt) = rest.strip_prefix('<') { after_lt.split_once('>')?.0 } else { rest.split_whitespace().next()? };
    Some((normalize_ref_label(label), dest))
}

fn strip_internal_ref_defs(md: &str) -> (String, BTreeSet<String>) {
    let mut kept = String::with_capacity(md.len());
    let mut internal_refs = BTreeSet::new();
    let mut fence: Option<(char, usize)> = None;

    for line in md.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        let trimmed = bare.trim();
        if let Some((ch, min_len)) = fence {
            kept.push_str(line);
            let count = trimmed.bytes().take_while(|&b| b == ch as u8).count();
            if count >= min_len && trimmed.get(count..).is_some_and(|r| r.trim().is_empty()) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = parse_fence_delim(trimmed) {
            fence = Some(open);
            kept.push_str(line);
            continue;
        }
        if let Some((key, dest)) = parse_ref_def_line(bare)
            && is_internal_target(dest)
        {
            internal_refs.insert(key);
            continue;
        }
        kept.push_str(line);
    }
    (kept, internal_refs)
}

fn parse_balanced_brackets(slice: &str) -> Option<(&str, usize)> {
    const MAX_LABEL_BYTES: usize = 2048;
    let after = slice.strip_prefix('[')?;
    let mut depth = 1usize;
    let mut i = 0usize;
    while let Some(rest) = after.get(i..) {
        if i >= MAX_LABEL_BYTES || rest.starts_with("\n\n") {
            return None;
        }
        let ch = rest.chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(nc) = after.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        if ch == '`' {
            let ticks = rest.bytes().take_while(|&b| b == b'`').count();
            i += ticks;
            continue;
        }
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((after.get(..i)?, i + 2));
                }
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn parse_inline_link_dest(slice: &str) -> Option<(&str, usize)> {
    const MAX_DEST_BYTES: usize = 2048;
    let after_open = slice.strip_prefix('(')?;
    let ws = after_open.bytes().take_while(u8::is_ascii_whitespace).count();
    let rest = after_open.get(ws..)?;
    let (dest, after_dest_offset) = if let Some(after_lt) = rest.strip_prefix('<') {
        let close = after_lt.find('>')?;
        if close >= MAX_DEST_BYTES || after_lt.get(..close)?.contains('\n') {
            return None;
        }
        (after_lt.get(..close)?, ws + 1 + close + 1)
    } else {
        let mut paren_depth = 0usize;
        let mut i = 0usize;
        while let Some(cur) = rest.get(i..) {
            if i >= MAX_DEST_BYTES {
                return None;
            }
            let ch = cur.chars().next()?;
            if ch == '\\' {
                i += 1;
                if let Some(nc) = rest.get(i..).and_then(|s| s.chars().next()) {
                    i += nc.len_utf8();
                }
                continue;
            }
            if ch.is_ascii_whitespace() {
                break;
            }
            if ch == '(' {
                paren_depth += 1;
            } else if ch == ')' {
                if paren_depth == 0 {
                    break;
                }
                paren_depth -= 1;
            }
            i += ch.len_utf8();
        }
        if paren_depth != 0 {
            return None;
        }
        (rest.get(..i)?, ws + i)
    };

    let tail = after_open.get(after_dest_offset..)?;
    let mut j = tail.bytes().take_while(u8::is_ascii_whitespace).count();
    if let Some(rem) = tail.get(j..) {
        if let Some(q) = rem.bytes().next().filter(|&b| b == b'"' || b == b'\'') {
            j += 1;
            let mut escaped = false;
            while let Some(&b) = tail.as_bytes().get(j) {
                j += 1;
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    break;
                }
            }
            j += tail.get(j..).map_or(0, |s| s.bytes().take_while(u8::is_ascii_whitespace).count());
        }
        if tail.as_bytes().get(j) == Some(&b')') {
            return Some((dest, 1 + after_dest_offset + j + 1));
        }
    }
    None
}

fn parse_code_span_len(slice: &str) -> Option<usize> {
    let ticks = slice.bytes().take_while(|&b| b == b'`').count();
    if ticks == 0 {
        return None;
    }
    let after = slice.get(ticks..)?;
    let mut i = 0usize;
    while let Some(cur) = after.get(i..) {
        if cur.is_empty() {
            break;
        }
        if cur.starts_with('`') {
            let run = cur.bytes().take_while(|&b| b == b'`').count();
            if run == ticks {
                return Some(ticks + i + ticks);
            }
            i += run;
            continue;
        }
        i += cur.chars().next()?.len_utf8();
    }
    None
}

fn strip_internal_links_segment(segment: &str, internal_refs: &BTreeSet<String>) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut i = 0usize;
    while let Some(rest) = segment.get(i..) {
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '`' {
            let run = rest.bytes().take_while(|&b| b == b'`').count();
            let consumed = parse_code_span_len(rest).unwrap_or(run);
            if let Some(raw) = rest.get(..consumed) {
                out.push_str(raw);
            }
            i += consumed;
            continue;
        }
        if ch == '<'
            && let Some((inner, _)) = rest.get(1..).and_then(|s| s.split_once('>'))
            && !inner.is_empty()
            && !inner.chars().any(char::is_whitespace)
            && !inner.contains('<')
            && is_internal_target(inner)
        {
            i += inner.len() + 2;
            continue;
        }
        if ch == '['
            && let Some((label, label_consumed)) = parse_balanced_brackets(rest)
        {
            let after_label = rest.get(label_consumed..).unwrap_or("");
            if let Some((dest, dest_consumed)) = parse_inline_link_dest(after_label) {
                if is_internal_target(dest) {
                    out.push_str(&strip_internal_links_segment(label, internal_refs));
                    i += label_consumed + dest_consumed;
                    continue;
                }
            } else if let Some((ref_label, ref_consumed)) = parse_balanced_brackets(after_label) {
                let key = if ref_label.trim().is_empty() { normalize_ref_label(label) } else { normalize_ref_label(ref_label) };
                if internal_refs.contains(&key) {
                    out.push_str(&strip_internal_links_segment(label, internal_refs));
                    i += label_consumed + ref_consumed;
                    continue;
                }
            } else if internal_refs.contains(&normalize_ref_label(label)) {
                out.push_str(&strip_internal_links_segment(label, internal_refs));
                i += label_consumed;
                continue;
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Replace Markdown links (inline, reference-style, and autolinks) pointing at internal
/// contributor/planning docs (`ROADMAP.md`, `STATUS.md`, `audit/`, `planning/`, `evidence/`,
/// `history/`) with their plain link text.
fn strip_internal_doc_links(md: &str) -> String {
    let (without_defs, internal_refs) = strip_internal_ref_defs(md);
    let mut out = String::with_capacity(without_defs.len());
    let mut pending = String::new();
    let mut fence: Option<(char, usize)> = None;

    for line in without_defs.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim();
        if let Some((ch, min_len)) = fence {
            out.push_str(line);
            let count = trimmed.bytes().take_while(|&b| b == ch as u8).count();
            if count >= min_len && trimmed.get(count..).is_some_and(|r| r.trim().is_empty()) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = parse_fence_delim(trimmed) {
            if !pending.is_empty() {
                out.push_str(&strip_internal_links_segment(&pending, &internal_refs));
                pending.clear();
            }
            fence = Some(open);
            out.push_str(line);
            continue;
        }
        pending.push_str(line);
    }
    if !pending.is_empty() {
        out.push_str(&strip_internal_links_segment(&pending, &internal_refs));
    }
    out
}

/// Regenerate both `llms.txt` and `llms-full.txt` atomically inside `out_dir` (e.g. `<manual_dir>/_root/`).
pub fn write_llms_files_into(root: &Path, out_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    for (name, content) in [("llms.txt", build_llms_txt(root)?), ("llms-full.txt", build_llms_full_txt(root)?)] {
        let path = out_dir.join(name);
        write_atomic(&path, content.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn llms_txt_contains_summary_key_pages_and_latest_manual_urls() {
        let root = TestDir::new("docs-llms-index");
        let text = build_llms_txt(root.path()).unwrap();
        assert!(text.starts_with("# CurveBook\n\n> "));
        for expected in [
            SITE_SUMMARY,
            "## Key Pages",
            "## Documentation Manual",
            "https://curvebook.org/faq.html",
            "https://curvebook.org/open-pzfx-files.html",
            "https://curvebook.org/llms-full.txt",
            "https://curvebook.org/manual/latest/index.html",
            "https://curvebook.org/manual/latest/analyses.html",
            "https://curvebook.org/manual/latest/automation/mcp.html",
        ] {
            assert!(text.contains(expected), "missing `{expected}`");
        }
        for forbidden in ["/manual/v0.", "ROADMAP.md", "STATUS.md"] {
            assert!(!text.contains(forbidden), "unexpected `{forbidden}`");
        }
    }

    #[test]
    fn llms_full_txt_is_deterministic_and_excludes_internal_docs() {
        let root = TestDir::new("docs-llms-full");
        root.write(
            "docs/reference/get-started.md",
            "# Get Started\r\n\r\nInstall instructions (see [roadmap](../ROADMAP.md), [status](../STATUS.md), and [audit report](../audit/r1.md)).\r\n",
        );
        root.write("docs/mcp-for-beginners.md", "# MCP for Beginners\n\nConfigure stdio MCP.\n");
        root.write("docs/reference/automation/cli.md", "# CLI Reference\n\nRun `curvebook-cli`.\n");
        root.write("docs/ROADMAP.md", "# Internal Roadmap SECRET_ROADMAP_MARKER\n");
        root.write("docs/STATUS.md", "# Internal Status SECRET_STATUS_MARKER\n");
        root.write("docs/audit/report.md", "# Audit SECRET_AUDIT_MARKER\n");

        let pass_a = build_llms_full_txt(root.path()).unwrap();
        assert_eq!(pass_a, build_llms_full_txt(root.path()).unwrap(), "llms-full.txt must be deterministic");
        assert!(!pass_a.contains('\r'), "CRLF must be normalized to LF");
        for expected in [
            "## Quick Facts",
            "Canonical URL: https://curvebook.org/manual/latest/get-started.html",
            "Install instructions (see roadmap, status, and audit report).",
            "Configure stdio MCP.",
            "Run `curvebook-cli`.",
        ] {
            assert!(pass_a.contains(expected), "missing `{expected}`");
        }
        for forbidden in ["ROADMAP.md", "STATUS.md", "audit/", "SECRET_ROADMAP_MARKER", "SECRET_STATUS_MARKER", "SECRET_AUDIT_MARKER"] {
            assert!(!pass_a.contains(forbidden), "unexpected `{forbidden}`");
        }
    }

    #[test]
    fn aud_12_44_strip_internal_doc_links_handles_brackets_parens_multiline_refs_and_autolinks() {
        let md = concat!(
            "Nested: [see arr[0]](../ROADMAP.md).\n",
            "Parens: [doc](../audit/report(1).md).\n",
            "Multiline: [see\nroadmap](../ROADMAP.md).\n",
            "Ref: [roadmap][r] and [status][] and [hist].\n",
            "Auto: <https://github.com/HauserGroup/curvebook/blob/main/docs/ROADMAP.md>.\n",
            "Keep: [Get Started](get-started.md) and `arr[0]`.\n\n",
            "[r]: ../ROADMAP.md\n",
            "[status]: ../STATUS.md \"Status\"\n",
            "[hist]: ../history/r1.md\n",
        );
        let cleaned = strip_internal_doc_links(md);
        assert!(cleaned.contains("Nested: see arr[0]."), "{cleaned}");
        assert!(cleaned.contains("Parens: doc."), "{cleaned}");
        assert!(cleaned.contains("Multiline: see\nroadmap."), "{cleaned}");
        assert!(cleaned.contains("Ref: roadmap and status and hist."), "{cleaned}");
        assert!(cleaned.contains("Keep: [Get Started](get-started.md) and `arr[0]`."), "{cleaned}");
        for forbidden in ["ROADMAP.md", "STATUS.md", "audit/", "history/"] {
            assert!(!cleaned.contains(forbidden), "leaked `{forbidden}` in:\n{cleaned}");
        }
    }
}
