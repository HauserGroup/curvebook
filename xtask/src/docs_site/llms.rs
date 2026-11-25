//! Generated AI-discoverability plain-Markdown maps (`R12.10`): `docs/llms.txt` and `docs/llms-full.txt`.
//!
//! Both files are pure, deterministic functions of the repository's published metadata and
//! hand-written user reference guides (`docs/reference/*.md`, `docs/reference/automation/*.md`,
//! `docs/mcp-for-beginners.md`). They contain zero timestamps and never include internal
//! contributor/planning documents (`ROADMAP.md`, `STATUS.md`, `audit/`, `planning/`, etc.).

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

/// Replace Markdown links pointing at internal contributor/planning docs (`ROADMAP.md`,
/// `STATUS.md`, `audit/`, `planning/`, `evidence/`, `history/`) with their plain link text.
fn strip_internal_doc_links(md: &str) -> String {
    const INTERNAL_TARGETS: &[&str] = &["roadmap.md", "status.md", "audit/", "planning/", "evidence/", "history/"];
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some((before, after_open)) = rest.split_once('[') {
        out.push_str(before);
        if let Some((label, after_mid)) = after_open.split_once("](")
            && !label.contains(['\n', '['])
            && let Some((target, tail)) = after_mid.split_once(')')
            && !target.contains('\n')
        {
            let lower_target = target.to_ascii_lowercase();
            if INTERNAL_TARGETS.iter().any(|pat| lower_target.contains(pat)) {
                out.push_str(label);
                rest = tail;
                continue;
            }
        }
        out.push('[');
        rest = after_open;
    }
    out.push_str(rest);
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
            "https://hausergroup.github.io/curvebook/faq.html",
            "https://hausergroup.github.io/curvebook/open-pzfx-files.html",
            "https://hausergroup.github.io/curvebook/llms-full.txt",
            "https://hausergroup.github.io/curvebook/manual/latest/index.html",
            "https://hausergroup.github.io/curvebook/manual/latest/analyses.html",
            "https://hausergroup.github.io/curvebook/manual/latest/automation/mcp.html",
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
            "Canonical URL: https://hausergroup.github.io/curvebook/manual/latest/get-started.html",
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
}
