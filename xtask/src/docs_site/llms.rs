//! Generated AI-discoverability plain-Markdown maps (`R12.10`): `llms.txt` and `llms-full.txt`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use curvebook_platform::write_atomic;

use super::markdown::{parse_code_span_len, parse_fence_open};
use super::seo::{SITE_BASE_URL, manual_canonical_url, published_url};
use super::sitemap::MANUAL_GUIDES;
use crate::scan_util::{is_regular_file, normalize_newlines};

pub const SITE_SUMMARY: &str = "CurveBook is a free, open-source biostatistics, nonlinear regression, and scientific graphing application for macOS, Windows, and Linux. It runs offline with zero telemetry and opens .pzfx and .prism data files.";
const GITHUB_REPO_URL: &str = "https://github.com/HauserGroup/curvebook";
const INTERNAL_TARGETS: &[&str] = &["roadmap.md", "status.md", "audit/", "planning/", "evidence/", "history/"];

const KEY_PAGES: &[(&str, &str, &str)] = &[
    ("CurveBook Home", "index.html", "Overview, feature summary with links into the manual, and Beta release downloads."),
    (
        "Open .pzfx and .prism Files",
        "open-pzfx-files.html",
        "Open .pzfx and .prism data files offline on macOS, Windows, and Linux, inspect read-only .prism analysis snapshots, and export tables to CSV, data-only .pzfx, or CurveBook projects.",
    ),
    (
        "Empirical .prism and .pzfx Specification",
        "pzfx-format-specification.html",
        "Clean-room empirical specification of .prism (ZIP/JSON/CSV, .dt digests, and proprietary vendor blobs) and .pzfx (XML and legacy PCFF) files observed across 3,719 public Zenodo files.",
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

const MANUAL_HUBS_AND_REGISTRIES: &[(&str, &str, &str)] = &[
    (
        "index.html",
        "CurveBook Reference Manual",
        "Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
    ),
    (
        "reference.html",
        "Reference",
        "Grouped reference directory for CurveBook concepts, data tables, statistical analyses, curve fitting, graphs, calculators, files, settings, and glossary.",
    ),
    (
        "support.html",
        "Support",
        "Troubleshooting, known issues, offline privacy guarantees, dual licensing, security reporting, and contributor guides.",
    ),
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
    (
        "az-index.html",
        "A–Z Reference Index",
        "Alphabetical index of CurveBook analyses, equations, graph kinds, tables, commands, MCP tools, CLI flags, formats, and preferences.",
    ),
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

pub fn build_llms_txt(_root: &Path) -> Result<String, String> {
    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "# CurveBook\n\n> {SITE_SUMMARY}\n\nCurveBook is written in pure Rust and dual-licensed under MIT OR Apache-2.0. It provides an offline desktop GUI, deterministic headless CLI (`curvebook-cli`), and local stdio Model Context Protocol (MCP) server (`curvebook-cli mcp`).\n\n## Key Pages\n"
    );
    for &(label, rel, desc) in KEY_PAGES {
        let _ = writeln!(out, "- [{label}]({}): {desc}", published_url(rel));
    }
    let _ = writeln!(
        out,
        "- [Source Code & Releases]({GITHUB_REPO_URL}): GitHub repository, issue tracker, and release packages.\n\n## Documentation Manual\n"
    );
    for &(rel, title, desc) in &MANUAL_HUBS_AND_REGISTRIES[..3] {
        let _ = writeln!(out, "- [{title}]({}): {desc}", manual_canonical_url(rel));
    }
    for g in MANUAL_GUIDES {
        let _ = writeln!(out, "- [{}]({}): {}", g.title, manual_canonical_url(g.rel_path), g.description);
    }
    for &(rel, title, desc) in &MANUAL_HUBS_AND_REGISTRIES[3..] {
        let _ = writeln!(out, "- [{title}]({}): {desc}", manual_canonical_url(rel));
    }
    Ok(out)
}

pub fn build_llms_full_txt(root: &Path) -> Result<String, String> {
    let mut out = String::with_capacity(256 * 1024);
    let _ = writeln!(
        out,
        "# CurveBook — Full Reference Documentation\n\n> {SITE_SUMMARY}\n\n- Website: {SITE_BASE_URL}\n- Online Manual: {}\n- Concise Index: {}\n- Source Code: {GITHUB_REPO_URL}\n\n## Quick Facts\n\n| Property | Details |\n| --- | --- |",
        manual_canonical_url("index.html"),
        published_url("llms.txt")
    );
    for &(k, v) in QUICK_FACTS {
        let _ = writeln!(out, "| {k} | {v} |");
    }
    for g in MANUAL_GUIDES {
        let full = root.join(g.repo_path);
        if is_regular_file(&full) {
            let raw = std::fs::read_to_string(&full).map_err(|e| format!("read {}: {e}", g.repo_path))?;
            let cleaned = strip_internal_doc_links(&normalize_newlines(&raw));
            if !cleaned.trim().is_empty() {
                let _ = writeln!(out, "\n---\n\nCanonical URL: {}\n\n{}", manual_canonical_url(g.rel_path), cleaned.trim());
            }
        }
    }
    Ok(out)
}

fn is_internal_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    INTERNAL_TARGETS.iter().any(|p| lower.contains(p))
}
fn norm_ref(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

fn parse_balanced_brackets(slice: &str) -> Option<(&str, usize)> {
    let (after, mut depth, mut i) = (slice.strip_prefix('[')?, 1usize, 0usize);
    while let Some(rest) = after.get(i..).filter(|r| i < 2048 && !r.starts_with("\n\n")) {
        let ch = rest.chars().next()?;
        if ch == '\\' {
            i += 1 + after.get(i + 1..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
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
    let after = slice.strip_prefix('(')?;
    let ws = after.bytes().take_while(u8::is_ascii_whitespace).count();
    let rest = after.get(ws..)?;
    let (dest, after_dest) = if let Some(lt) = rest.strip_prefix('<') {
        let close = lt.find('>')?;
        if close >= 2048 || lt.get(..close)?.contains('\n') {
            return None;
        }
        (lt.get(..close)?, ws + close + 2)
    } else {
        let (mut depth, mut i) = (0usize, 0usize);
        while let Some(cur) = rest.get(i..) {
            let ch = (i < 2048).then(|| cur.chars().next()).flatten()?;
            if ch == '\\' {
                i += 1 + rest.get(i + 1..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
                continue;
            }
            if ch.is_ascii_whitespace() || (ch == ')' && depth == 0) {
                break;
            }
            depth = if ch == '(' {
                depth + 1
            } else if ch == ')' {
                depth - 1
            } else {
                depth
            };
            i += ch.len_utf8();
        }
        (depth == 0).then(|| (rest.get(..i), ws + i)).and_then(|(d, p)| Some((d?, p)))?
    };
    let tail = after.get(after_dest..)?;
    let mut j = tail.bytes().take_while(u8::is_ascii_whitespace).count();
    if let Some(&q) = tail.as_bytes().get(j).filter(|&&b| b == b'"' || b == b'\'') {
        let (mut k, mut esc) = (j + 1, false);
        while let Some(&b) = tail.as_bytes().get(k) {
            k += 1;
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == q {
                break;
            }
        }
        j = k + tail.get(k..).map_or(0, |s| s.bytes().take_while(u8::is_ascii_whitespace).count());
    }
    (tail.as_bytes().get(j) == Some(&b')')).then_some((dest, 1 + after_dest + j + 1))
}

fn strip_links_segment(seg: &str, internal_refs: &BTreeSet<String>) -> String {
    let (mut out, mut i) = (String::with_capacity(seg.len()), 0usize);
    while let Some(rest) = seg.get(i..) {
        let Some(ch) = rest.chars().next() else { break };
        if ch == '`' {
            let n = parse_code_span_len(rest).unwrap_or_else(|| rest.bytes().take_while(|&b| b == b'`').count());
            out.push_str(rest.get(..n).unwrap_or(""));
            i += n;
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
            && let Some((label, l_len)) = parse_balanced_brackets(rest)
        {
            let after = rest.get(l_len..).unwrap_or("");
            let drop_len = if let Some((dest, d_len)) = parse_inline_link_dest(after) {
                is_internal_target(dest).then_some(l_len + d_len)
            } else if let Some((r_lbl, r_len)) = parse_balanced_brackets(after) {
                let key = if r_lbl.trim().is_empty() { norm_ref(label) } else { norm_ref(r_lbl) };
                internal_refs.contains(&key).then_some(l_len + r_len)
            } else {
                internal_refs.contains(&norm_ref(label)).then_some(l_len)
            };
            if let Some(skip) = drop_len {
                out.push_str(&strip_links_segment(label, internal_refs));
                i += skip;
                continue;
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn for_each_fenced_line<'a>(md: &'a str, mut on_line: impl FnMut(&'a str, bool)) {
    let mut fence = None::<(u8, usize)>;
    for line in md.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim();
        if let Some((ch, min_len)) = fence {
            on_line(line, true);
            let n = trimmed.bytes().take_while(|&b| b == ch).count();
            if n >= min_len && trimmed.get(n..).is_some_and(|r| r.trim().is_empty()) {
                fence = None;
            }
        } else if let Some(open) = parse_fence_open(trimmed) {
            fence = Some(open);
            on_line(line, true);
        } else {
            on_line(line, false);
        }
    }
}

fn strip_internal_doc_links(md: &str) -> String {
    let (mut kept, mut internal_refs) = (String::with_capacity(md.len()), BTreeSet::new());
    for_each_fenced_line(md, |line, in_fence| {
        let def = (!in_fence)
            .then(|| {
                let (l, r) = line.trim_end_matches('\n').trim_start().strip_prefix('[')?.split_once("]:")?;
                let r = (!l.trim().is_empty() && !l.contains(['[', ']'])).then_some(r.trim_start())?;
                let dest = if let Some(lt) = r.strip_prefix('<') { lt.split_once('>')?.0 } else { r.split_whitespace().next()? };
                is_internal_target(dest).then(|| norm_ref(l))
            })
            .flatten();
        if let Some(key) = def {
            internal_refs.insert(key);
        } else {
            kept.push_str(line);
        }
    });
    let (mut out, mut pending) = (String::with_capacity(kept.len()), String::new());
    for_each_fenced_line(&kept, |line, in_fence| {
        if in_fence {
            if !pending.is_empty() {
                out.push_str(&strip_links_segment(&pending, &internal_refs));
                pending.clear();
            }
            out.push_str(line);
        } else {
            pending.push_str(line);
        }
    });
    if !pending.is_empty() {
        out.push_str(&strip_links_segment(&pending, &internal_refs));
    }
    out
}

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
    fn llms_files_are_deterministic_and_strip_internal_links() {
        let root = TestDir::new("docs-llms");
        root.write("docs/reference/get-started.md", "# Get Started\r\n\r\nSee [arr[0]](../ROADMAP.md), [doc](../audit/r(1).md), [r][r], [st][], <https://github.com/HauserGroup/curvebook/blob/main/docs/ROADMAP.md>, and [Keep](get-started.md).\r\n\r\n[r]: ../ROADMAP.md\r\n[st]: ../STATUS.md\r\n");
        root.write("docs/ROADMAP.md", "SECRET_ROADMAP");
        let txt = build_llms_txt(root.path()).unwrap();
        assert!(txt.contains(SITE_SUMMARY) && txt.contains("https://curvebook.org/manual/latest/index.html") && !txt.contains("ROADMAP.md"));
        let full = build_llms_full_txt(root.path()).unwrap();
        assert_eq!(full, build_llms_full_txt(root.path()).unwrap());
        assert!(!full.contains('\r') && full.contains("See arr[0], doc, r, st, , and [Keep](get-started.md)."));
        for bad in ["ROADMAP.md", "STATUS.md", "audit/", "SECRET_ROADMAP"] {
            assert!(!full.contains(bad), "leaked `{bad}`");
        }
    }
}
