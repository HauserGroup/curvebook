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

/// Repository-relative path of the concise LLM discovery index.
pub const LLMS_TXT_REL_PATH: &str = "docs/llms.txt";

/// Repository-relative path of the compiled plain-Markdown reference bundle.
pub const LLMS_FULL_TXT_REL_PATH: &str = "docs/llms-full.txt";

/// Primary one-paragraph summary matching `docs/index.html`.
pub const SITE_SUMMARY: &str = "CurveBook is a free, open-source biostatistics, nonlinear regression, and scientific graphing application for macOS, Windows, and Linux. It runs offline with zero telemetry and opens .pzfx and .prism data files.";

const GITHUB_REPO_URL: &str = "https://github.com/HauserGroup/curvebook";

/// Hand-written user reference guides compiled into `docs/llms-full.txt`, in deterministic manual reading order.
/// Each tuple is `(site_rel_path, title, description, repo_markdown_path)`.
pub const MANUAL_MARKDOWN_GUIDES: &[(&str, &str, &str, &str)] = &[
    (
        "get-started.html",
        "Get Started",
        "System requirements, installing CurveBook, verifying checksums/attestations, and font configuration.",
        "docs/reference/get-started.md",
    ),
    (
        "desktop-gui.html",
        "Desktop GUI Overview",
        "Overview of desktop welcome workflow, sheets, analyses, graphs, saving, and exporting.",
        "docs/reference/desktop-gui.md",
    ),
    (
        "mcp-for-beginners.html",
        "MCP Setup for Beginners",
        "Configure an MCP client to connect to CurveBook and verify the connection.",
        "docs/mcp-for-beginners.md",
    ),
    (
        "concepts.html",
        "Concepts & Reactive Pipeline",
        "Project structure, five sheet types, reactive recalculation pipeline, exclusions, checkpoints, and undo.",
        "docs/reference/concepts.md",
    ),
    (
        "tables.html",
        "Data Tables, Formulas & Transforms",
        "Table archetypes, replicate subcolumn modes, Calculated Variables formula language, and data transforms.",
        "docs/reference/tables.md",
    ),
    (
        "analyses.html",
        "Statistical Analyses Reference",
        "Complete interface-neutral reference for every CurveBook statistical analysis kind, options, diagnostics, and citations.",
        "docs/reference/analyses.md",
    ),
    (
        "fitting.html",
        "Curve Fitting Reference",
        "Nonlinear regression methods, equation library, weighting, robust/ROUT fitting, bands, custom DSL, and ODE models.",
        "docs/reference/fitting.md",
    ),
    (
        "graphs.html",
        "Graphs & Page Layouts",
        "Graph types, compatible table archetypes, axis scales, error bars, colour palettes, and Page Layout rendering.",
        "docs/reference/graphs.md",
    ),
    (
        "qol.html",
        "Calculators & Quality-of-Life Tools",
        "Scientific calculators, quick statistics, and workspace quality-of-life capabilities.",
        "docs/reference/qol.md",
    ),
    (
        "files.html",
        "Files, Import & Export",
        "CurveBook archives, unpacked .curvebook.d directories, templates, autosave recovery, and interchange formats.",
        "docs/reference/files.md",
    ),
    (
        "automation/mcp.html",
        "MCP Server Reference",
        "CurveBook Model Context Protocol server transports, core tools, toolsets, resources, and prompts.",
        "docs/reference/automation/mcp.md",
    ),
    (
        "automation/cli.html",
        "CLI Reference (curvebook-cli)",
        "Headless runner subcommands, schema-typed direct command flags, exit codes, and PDF/format flags.",
        "docs/reference/automation/cli.md",
    ),
    (
        "automation/control-and-batch.html",
        "Control Protocol, Batch Format & Errors",
        "Loopback --control JSON-lines protocol, curvebook.batch.v1 manifests, run scripts, and engine error codes.",
        "docs/reference/automation/control-and-batch.md",
    ),
    (
        "settings.html",
        "Settings & Languages",
        "Preferences JSON Schema keys, keyboard shortcut customization, interface locales, and locale formatting.",
        "docs/reference/settings.md",
    ),
    (
        "privacy-security.html",
        "Privacy, Licences & Security",
        "Zero-telemetry local execution, loopback control rules, dual licensing, and vulnerability reporting.",
        "docs/reference/privacy-security.md",
    ),
    (
        "troubleshooting.html",
        "Troubleshooting & Known Issues",
        "Common install, font discovery, MCP transport, archive recovery, and solver diagnostics.",
        "docs/reference/troubleshooting.md",
    ),
    ("glossary.html", "Glossary", "Definitions of core CurveBook scientific, data-model, and automation terms.", "docs/reference/glossary.md"),
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
    out.push_str("# CurveBook\n\n");
    let _ = writeln!(out, "> {SITE_SUMMARY}\n");
    out.push_str("CurveBook is written in pure Rust and dual-licensed under MIT OR Apache-2.0. It provides an offline desktop GUI, deterministic headless CLI (`curvebook-cli`), and local stdio Model Context Protocol (MCP) server (`curvebook-cli mcp`).\n\n");

    out.push_str("## Key Pages\n\n");
    let _ = writeln!(
        out,
        "- [CurveBook Home]({}): Overview, workspace screenshots, core pillars, and Beta release downloads.",
        published_url("index.html")
    );
    let _ = writeln!(
        out,
        "- [Open .pzfx and .prism Files]({}): Open .pzfx and .prism data files offline on macOS, Windows, and Linux and export tables to CSV or CurveBook projects.",
        published_url("open-pzfx-files.html")
    );
    let _ = writeln!(
        out,
        "- [CurveBook on Linux]({}): Native pure-Rust biostatistics and nonlinear curve fitting AppImage for Linux (x86_64 and aarch64), Wayland/X11, headless CLI, and local MCP.",
        published_url("linux.html")
    );
    let _ = writeln!(
        out,
        "- [Frequently Asked Questions]({}): Quick facts and direct answers on licensing, file formats, offline privacy, platforms, statistical methods, and MCP automation.",
        published_url("faq.html")
    );
    let _ = writeln!(
        out,
        "- [Privacy Policy]({}): Local-first architecture, zero telemetry, no outbound network connections, and no auto-updater.",
        published_url("privacy.html")
    );
    let _ = writeln!(
        out,
        "- [Full Reference Compilation (llms-full.txt)]({}): Single-file plain-Markdown compilation of the CurveBook summary, quick facts, and all hand-written reference guides.",
        published_url("llms-full.txt")
    );
    let _ = writeln!(out, "- [Source Code & Releases]({GITHUB_REPO_URL}): GitHub repository, issue tracker, and release packages.");

    out.push_str("\n## Documentation Manual\n\n");
    let _ = writeln!(
        out,
        "- [CurveBook Reference Manual]({}): Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
        manual_canonical_url("index.html")
    );
    for &(rel_path, title, desc, _) in MANUAL_MARKDOWN_GUIDES {
        let _ = writeln!(out, "- [{title}]({}): {desc}", manual_canonical_url(rel_path));
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
    out.push_str("# CurveBook — Full Reference Documentation\n\n");
    let _ = writeln!(out, "> {SITE_SUMMARY}\n");
    let _ = writeln!(out, "- Website: {SITE_BASE_URL}");
    let _ = writeln!(out, "- Online Manual: {}", manual_canonical_url("index.html"));
    let _ = writeln!(out, "- Concise Index: {}", published_url("llms.txt"));
    let _ = writeln!(out, "- Source Code: {GITHUB_REPO_URL}\n");

    out.push_str("## Quick Facts\n\n");
    out.push_str("| Property | Details |\n");
    out.push_str("| --- | --- |\n");
    for &(key, val) in QUICK_FACTS {
        let _ = writeln!(out, "| {key} | {val} |");
    }

    for &(site_rel, _title, _desc, repo_path) in MANUAL_MARKDOWN_GUIDES {
        let full_path = root.join(repo_path);
        if !std::fs::symlink_metadata(&full_path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
            continue;
        }
        let raw = std::fs::read_to_string(&full_path).map_err(|e| format!("read {repo_path}: {e}"))?;
        let normalized = strip_internal_doc_links(&raw.replace("\r\n", "\n"));
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push_str("\n---\n\n");
        let _ = writeln!(out, "Canonical URL: {}\n", manual_canonical_url(site_rel));
        out.push_str(trimmed);
        out.push('\n');
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
            && !label.contains('\n')
            && !label.contains('[')
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
    let llms = build_llms_txt(root)?;
    let llms_path = out_dir.join("llms.txt");
    write_atomic(&llms_path, llms.as_bytes()).map_err(|e| format!("write {}: {e}", llms_path.display()))?;
    let llms_full = build_llms_full_txt(root)?;
    let llms_full_path = out_dir.join("llms-full.txt");
    write_atomic(&llms_full_path, llms_full.as_bytes()).map_err(|e| format!("write {}: {e}", llms_full_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        std::env::temp_dir().join(format!("cb-docs-llms-{label}-{}-{nanos}", std::process::id()))
    }

    fn write_file(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn llms_txt_contains_summary_key_pages_and_latest_manual_urls() {
        let root = temp_root("index");
        let text = build_llms_txt(&root).unwrap();
        assert!(text.starts_with("# CurveBook\n\n> "));
        assert!(text.contains(SITE_SUMMARY));
        assert!(text.contains("## Key Pages"));
        assert!(text.contains("## Documentation Manual"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/faq.html"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/open-pzfx-files.html"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/llms-full.txt"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/manual/latest/index.html"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/manual/latest/analyses.html"));
        assert!(text.contains("https://hausergroup.github.io/curvebook/manual/latest/automation/mcp.html"));
        assert!(!text.contains("/manual/v0."), "llms.txt must link to /manual/latest/, not versioned snapshots");
        assert!(!text.contains("ROADMAP.md"));
        assert!(!text.contains("STATUS.md"));
    }

    #[test]
    fn llms_full_txt_is_deterministic_and_excludes_internal_docs() {
        let root = temp_root("full");
        write_file(
            &root,
            "docs/reference/get-started.md",
            "# Get Started\r\n\r\nInstall instructions (see [roadmap](../ROADMAP.md), [status](../STATUS.md), and [audit report](../audit/r1.md)).\r\n",
        );
        write_file(&root, "docs/mcp-for-beginners.md", "# MCP for Beginners\n\nConfigure stdio MCP.\n");
        write_file(&root, "docs/reference/automation/cli.md", "# CLI Reference\n\nRun `curvebook-cli`.\n");
        // Internal docs that must never appear in llms-full.txt.
        write_file(&root, "docs/ROADMAP.md", "# Internal Roadmap SECRET_ROADMAP_MARKER\n");
        write_file(&root, "docs/STATUS.md", "# Internal Status SECRET_STATUS_MARKER\n");
        write_file(&root, "docs/audit/report.md", "# Audit SECRET_AUDIT_MARKER\n");

        let pass_a = build_llms_full_txt(&root).unwrap();
        let pass_b = build_llms_full_txt(&root).unwrap();
        assert_eq!(pass_a, pass_b, "llms-full.txt must be deterministic");
        assert!(!pass_a.contains('\r'), "CRLF must be normalized to LF");
        assert!(pass_a.contains("## Quick Facts"));
        assert!(pass_a.contains("Canonical URL: https://hausergroup.github.io/curvebook/manual/latest/get-started.html"));
        assert!(pass_a.contains("Install instructions (see roadmap, status, and audit report)."));
        assert!(pass_a.contains("Configure stdio MCP."));
        assert!(pass_a.contains("Run `curvebook-cli`."));
        assert!(!pass_a.contains("ROADMAP.md"));
        assert!(!pass_a.contains("STATUS.md"));
        assert!(!pass_a.contains("audit/"));
        assert!(!pass_a.contains("SECRET_ROADMAP_MARKER"));
        assert!(!pass_a.contains("SECRET_STATUS_MARKER"));
        assert!(!pass_a.contains("SECRET_AUDIT_MARKER"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
