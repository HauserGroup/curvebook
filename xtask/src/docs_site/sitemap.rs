//! Beta site map assembly, Markdown page ingestion, link rewriting, A–Z index, and fallback bridges.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use super::markdown::{LinkResolver, TocHeading, escape_html, github_slug, render_markdown};
use super::registries::{AzEntry, ExpectedCoverage, RegistryBundle, RegistryPageSpec, all_analysis_kinds_meta, generate_registry_pages};
use super::template::{NavLink, PageSpec, SiteMeta, rel_href, render_page};

const GITHUB_REPO_BASE: &str = "https://github.com/HauserGroup/curvebook";

/// Complete generated static site in memory before writing to disk or checking invariants.
pub struct GeneratedSite {
    /// Deterministic ordered map of site-relative paths (e.g., `"index.html"`, `"registry/commands.html"`)
    /// to complete UTF-8 HTML documents.
    pub html_pages: BTreeMap<String, String>,
    /// Expected live registry identifiers for `--check` coverage validation.
    pub expected_coverage: ExpectedCoverage,
    /// Release / preview metadata used to render this site.
    pub meta: SiteMeta,
}

#[derive(Clone)]
struct SectionDef {
    id: &'static str,
    title: &'static str,
    index_rel_path: &'static str,
    summary: &'static str,
}

const BETA_SECTIONS: &[SectionDef] = &[
    SectionDef {
        id: "get-started",
        title: "Get started",
        index_rel_path: "section-get-started.html",
        summary: "Installation, checksum/attestation verification, font configuration, desktop onboarding, and MCP client setup.",
    },
    SectionDef {
        id: "concepts",
        title: "Concepts",
        index_rel_path: "section-concepts.html",
        summary: "Projects, five sheet families, reactive recalculation pipeline, non-destructive exclusions, checkpoints, and undo.",
    },
    SectionDef {
        id: "tables",
        title: "Data tables & formulas",
        index_rel_path: "section-tables.html",
        summary: "Table archetypes, subcolumn and replicate modes, Calculated Variables formulas, and linked data transforms.",
    },
    SectionDef {
        id: "analyses",
        title: "Analyses",
        index_rel_path: "section-analyses.html",
        summary: "Statistical tests, assumptions, diagnostics, edge-case behavior, peer-reviewed citations, and oracle verification.",
    },
    SectionDef {
        id: "fitting",
        title: "Curve fitting",
        index_rel_path: "section-fitting.html",
        summary: "Built-in equation library, nonlinear solvers, weighting schemes, robust/ROUT fitting, bands, and custom DSL/ODE models.",
    },
    SectionDef {
        id: "graphs",
        title: "Graphs & Page Layouts",
        index_rel_path: "section-graphs.html",
        summary: "Plot families, scales, frames, palettes, multi-panel Page Layouts, and publication vector/raster rendering.",
    },
    SectionDef {
        id: "qol",
        title: "Calculators & quality-of-life tools",
        index_rel_path: "section-qol.html",
        summary: "Quick statistics, scientific calculators, command palette, cloning pipelines, and quality-of-life capabilities.",
    },
    SectionDef {
        id: "files",
        title: "Files, import & export",
        index_rel_path: "section-files.html",
        summary: "Archive/directory/template formats, interchange importers, export formats, autosave recovery, and JSON Schemas.",
    },
    SectionDef {
        id: "automation",
        title: "Automation",
        index_rel_path: "section-automation.html",
        summary: "MCP server, curvebook-cli subcommands, loopback --control protocol, curvebook.batch.v1 manifests, and command catalogs.",
    },
    SectionDef {
        id: "settings",
        title: "Settings & languages",
        index_rel_path: "section-settings.html",
        summary: "Preference schema keys, shortcut customization, supported interface locales, and locale formatting.",
    },
    SectionDef {
        id: "privacy-security",
        title: "Privacy, licences & security",
        index_rel_path: "section-privacy-security.html",
        summary: "Offline/zero-telemetry operation, loopback socket policy, dual licensing, clean-room policy, and security disclosures.",
    },
    SectionDef {
        id: "troubleshooting",
        title: "Troubleshooting & known issues",
        index_rel_path: "section-troubleshooting.html",
        summary: "Diagnosing font, MCP, archive recovery, and solver diagnostics alongside documented release limits.",
    },
    SectionDef {
        id: "glossary",
        title: "Glossary & A–Z index",
        index_rel_path: "section-glossary.html",
        summary: "Defined terminology across statistical and software workflows plus the complete A–Z index of symbols.",
    },
    SectionDef {
        id: "contribute",
        title: "Contribute",
        index_rel_path: "section-contribute.html",
        summary: "Architecture, clean-room rules, CI stages, extending analyses/commands, localization, and releasing snapshots.",
    },
];

const CONTRIBUTOR_GITHUB_GUIDES: &[(&str, &str, &str)] = &[
    ("AGENTS Rules & Workspace Policy", "AGENTS.md", "Repository contributor rules, clean-room engineering policy, and module boundaries."),
    ("Workspace Architecture", "docs/architecture.md", "Crate dependency layers (L0–L7), rendering pipeline, and headless/GUI separation."),
    ("Developer Setup & Commands", "docs/development.md", "Building, testing, coverage reporting, font env vars, and verification workflows."),
    ("Extending CurveBook", "docs/extending.md", "How to add new engine commands, statistical analyses, equations, and UI menus."),
    ("Engineering Pitfalls", "docs/pitfalls.md", "Recurring pitfalls prevented by repository gates and review checklists."),
    ("General Engineering Pitfalls", "docs/pitfalls-engineering.md", "General Rust, tooling, persistence, and process rules adopted for CurveBook."),
    (
        "Command ID Stability Contract",
        "docs/command-ids.md",
        "Naming conventions, area prefixes, and backward-compatibility guarantees for engine command IDs.",
    ),
    (
        "Archive & Format Specifications",
        "docs/file-formats.md",
        "Technical format boundaries for .curvebook archives, unpacked directories, templates, and foreign imports.",
    ),
    ("Licences & Third-Party Policy", "docs/licences.md", "Dual-licence policy, cargo-deny rules, and asset attribution gates."),
    ("Localization Architecture", "docs/localization.md", "Translation catalog conventions, ICU/number formatting, and locale testing."),
    ("CI Stages & Gates", "docs/ci.md", "Repository cargo xtask ci stage matrix, budgets, and compliance scanners."),
    (
        "Releasing & Snapshot Publication",
        "docs/release/releasing.md",
        "Cutting releases, artifact verification, checksums, and publishing versioned docs snapshots.",
    ),
];

enum PageSource {
    Markdown { repo_path: &'static str, fallback_kind: FallbackKind },
    PreRendered { h1: String, toc: Vec<TocHeading>, body_html: String },
}

#[derive(Clone, Copy)]
enum FallbackKind {
    Generic { lead: &'static str, target_registry: Option<(&'static str, &'static str)> },
    Analyses,
    Glossary,
    MustExist,
}

struct PageItem {
    rel_path: String,
    title: String,
    description: String,
    section_id: &'static str,
    source: PageSource,
}

struct RepoLinkResolver<'a> {
    source_repo_rel: &'a str,
    current_site_rel: &'a str,
    repo_root: &'a Path,
    meta: &'a SiteMeta,
    known_site_targets: &'a BTreeMap<&'static str, &'static str>,
    broken_links: std::cell::RefCell<Vec<String>>,
}

impl LinkResolver for RepoLinkResolver<'_> {
    fn resolve_href(&self, raw_href: &str) -> String {
        if raw_href.starts_with('#') || raw_href.starts_with("http://") || raw_href.starts_with("https://") || raw_href.starts_with("mailto:") {
            return raw_href.to_owned();
        }
        let (target_part, frag_part) = match raw_href.split_once('#') {
            Some((t, f)) => (t, Some(f)),
            None => (raw_href, None),
        };
        if target_part.is_empty() {
            return frag_part.map(|f| format!("#{f}")).unwrap_or_default();
        }

        let clean_target = target_part.trim_end_matches('/');
        if clean_target.starts_with("registry/") && clean_target.ends_with(".html") {
            let mut href = rel_href(self.current_site_rel, clean_target);
            if let Some(f) = frag_part {
                href.push('#');
                href.push_str(f);
            }
            return href;
        }
        if clean_target.starts_with("../registry/") && clean_target.ends_with(".html") {
            let suffix = clean_target.trim_start_matches("../");
            let mut href = rel_href(self.current_site_rel, suffix);
            if let Some(f) = frag_part {
                href.push('#');
                href.push_str(f);
            }
            return href;
        }

        let source_dir = Path::new(self.source_repo_rel).parent().unwrap_or_else(|| Path::new(""));
        let normalized = normalize_repo_rel(source_dir, target_part);

        if let Some(mapped_site_rel) = self.known_site_targets.get(normalized.as_str()) {
            let mut href = rel_href(self.current_site_rel, mapped_site_rel);
            if let Some(f) = frag_part {
                href.push('#');
                href.push_str(f);
            } else if let Some(default_frag) = default_schema_fragment(&normalized) {
                href.push_str(default_frag);
            }
            return href;
        }

        if normalized == "docs/index.html" {
            return if self.meta.is_release_snapshot {
                let assets = super::template::asset_prefix(self.current_site_rel, true);
                format!("{assets}index.html{}", frag_suffix(frag_part))
            } else {
                format!("{}{}", rel_href(self.current_site_rel, "index.html"), frag_suffix(frag_part))
            };
        }
        if normalized == "docs/privacy.html" {
            return if self.meta.is_release_snapshot {
                let assets = super::template::asset_prefix(self.current_site_rel, true);
                format!("{assets}privacy.html{}", frag_suffix(frag_part))
            } else {
                format!("{}{}", rel_href(self.current_site_rel, "privacy-security.html"), frag_suffix(frag_part))
            };
        }
        if let Some(img_name) = normalized.strip_prefix("docs/images/") {
            let assets = super::template::asset_prefix(self.current_site_rel, self.meta.is_release_snapshot);
            return format!("{assets}images/{img_name}");
        }

        let trimmed = normalized.trim_end_matches('/');
        let target_path = self.repo_root.join(trimmed);
        if trimmed.is_empty() || trimmed.ends_with(".html") || !target_path.exists() {
            self.broken_links
                .borrow_mut()
                .push(format!("{}: broken relative link `{raw_href}` (resolved repo path `{trimmed}` does not exist)", self.source_repo_rel));
        }
        let is_dir = target_part.ends_with('/')
            || std::fs::symlink_metadata(&target_path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir());
        let kind = if is_dir { "tree" } else { "blob" };
        format!("{GITHUB_REPO_BASE}/{kind}/main/{trimmed}{}", frag_suffix(frag_part))
    }

    fn resolve_image(&self, raw_src: &str) -> String {
        if raw_src.starts_with("http://") || raw_src.starts_with("https://") {
            return raw_src.to_owned();
        }
        let source_dir = Path::new(self.source_repo_rel).parent().unwrap_or_else(|| Path::new(""));
        let normalized = normalize_repo_rel(source_dir, raw_src);
        let assets = super::template::asset_prefix(self.current_site_rel, self.meta.is_release_snapshot);
        if let Some(img_name) = normalized.strip_prefix("docs/images/") {
            let candidate = self.repo_root.join("docs/images").join(img_name);
            if std::fs::symlink_metadata(&candidate).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
                return format!("{assets}images/{img_name}");
            }
            self.broken_links
                .borrow_mut()
                .push(format!("{}: missing image asset `{raw_src}` (resolved repo path `{normalized}` does not exist)", self.source_repo_rel));
            return format!("{assets}images/{img_name}");
        }
        if let Some(file_name) = Path::new(&normalized).file_name().and_then(|n| n.to_str()) {
            let candidate = self.repo_root.join("docs/images").join(file_name);
            if std::fs::symlink_metadata(&candidate).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
                return format!("{assets}images/{file_name}");
            }
        }
        self.broken_links
            .borrow_mut()
            .push(format!("{}: missing image asset `{raw_src}` (resolved repo path `{normalized}` does not exist)", self.source_repo_rel));
        format!("{assets}images/{normalized}")
    }
}

fn frag_suffix(frag: Option<&str>) -> String {
    frag.map(|f| format!("#{f}")).unwrap_or_default()
}

fn default_schema_fragment(normalized: &str) -> Option<&'static str> {
    match normalized {
        "docs/schemas/preferences.schema.json" => Some("#schema-preferences"),
        "docs/schemas/batch-v1.schema.json" => Some("#schema-batch-v1"),
        "docs/schemas/document.schema.json" => Some("#schema-document"),
        "docs/schemas/archive-manifest.schema.json" => Some("#schema-archive-manifest"),
        "docs/schemas/journal-record.schema.json" => Some("#schema-journal-record"),
        _ => None,
    }
}

fn normalize_repo_rel(base_dir: &Path, raw: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for comp in base_dir.components() {
        if let std::path::Component::Normal(s) = comp
            && let Some(txt) = s.to_str()
            && !txt.is_empty()
        {
            parts.push(txt.to_owned());
        }
    }
    let clean = raw.replace('\\', "/");
    let stripped = clean.trim_start_matches("./");
    if let Some(abs) = stripped.strip_prefix('/') {
        parts.clear();
        push_segments(abs, &mut parts);
    } else {
        push_segments(stripped, &mut parts);
    }
    parts.join("/")
}

fn push_segments(path: &str, parts: &mut Vec<String>) {
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                let _ = parts.pop();
            }
            other => parts.push(other.to_owned()),
        }
    }
}

fn build_known_site_targets() -> BTreeMap<&'static str, &'static str> {
    let mut map = BTreeMap::new();
    map.insert("docs/reference/get-started.md", "get-started.html");
    map.insert("docs/reference/desktop-gui.md", "desktop-gui.html");
    map.insert("docs/reference/concepts.md", "concepts.html");
    map.insert("docs/reference/tables.md", "tables.html");
    map.insert("docs/reference/analyses.md", "analyses.html");
    map.insert("docs/reference/fitting.md", "fitting.html");
    map.insert("docs/reference/graphs.md", "graphs.html");
    map.insert("docs/reference/qol.md", "qol.html");
    map.insert("docs/reference/files.md", "files.html");
    map.insert("docs/reference/settings.md", "settings.html");
    map.insert("docs/reference/privacy-security.md", "privacy-security.html");
    map.insert("docs/reference/troubleshooting.md", "troubleshooting.html");
    map.insert("docs/reference/glossary.md", "glossary.html");
    map.insert("docs/reference/automation/mcp.md", "automation/mcp.html");
    map.insert("docs/reference/automation/cli.md", "automation/cli.html");
    map.insert("docs/reference/automation/control-and-batch.md", "automation/control-and-batch.html");
    map.insert("docs/mcp-for-beginners.md", "mcp-for-beginners.html");
    map.insert("docs/mcp.md", "automation/mcp.html");
    map.insert("docs/control-protocol.md", "automation/control-and-batch.html");
    map.insert("docs/commands.md", "registry/commands.html");
    map.insert("docs/schemas/preferences.schema.json", "registry/formats.html");
    map.insert("docs/schemas/batch-v1.schema.json", "registry/formats.html");
    map.insert("docs/schemas/document.schema.json", "registry/formats.html");
    map.insert("docs/schemas/archive-manifest.schema.json", "registry/formats.html");
    map.insert("docs/schemas/journal-record.schema.json", "registry/formats.html");
    map
}

fn collect_defined_analysis_anchors(root: &Path) -> BTreeSet<String> {
    let analyses_md = root.join("docs/reference/analyses.md");
    let Ok(md_src) = std::fs::read_to_string(analyses_md) else {
        return BTreeSet::new();
    };
    let rendered = render_markdown(&md_src, &super::markdown::IdentityResolver);
    rendered.headings.into_iter().map(|h| h.id).collect()
}

/// Assemble the entire deterministic documentation site from repository Markdown and registries.
pub fn assemble_site(root: &Path, meta: SiteMeta) -> Result<GeneratedSite, String> {
    let defined_analysis_anchors = collect_defined_analysis_anchors(root);
    let registries: RegistryBundle = generate_registry_pages(root, &defined_analysis_anchors)?;
    let known_targets = build_known_site_targets();
    let mut az_entries = registries.az_entries;

    let mut pages = ordered_content_pages(registries.pages);
    let glossary_extra = collect_glossary_az_entries(root);
    az_entries.extend(glossary_extra);

    // Build deterministic flat reading sequence: home, then per section (section index -> section pages).
    let mut ordered: Vec<PageItem> = Vec::with_capacity(pages.len() + BETA_SECTIONS.len() + 2);
    ordered.push(build_home_page(&pages));

    let mut by_section: BTreeMap<&'static str, Vec<PageItem>> = BTreeMap::new();
    for p in pages.drain(..) {
        by_section.entry(p.section_id).or_default().push(p);
    }

    for sec in BETA_SECTIONS {
        let children = by_section.remove(sec.id).unwrap_or_default();
        let child_refs: Vec<&PageItem> = children.iter().collect();
        ordered.push(build_section_index_page(sec, &child_refs));
        if sec.id == "glossary" {
            for p in children {
                ordered.push(p);
            }
            ordered.push(build_az_index_page(&az_entries));
        } else {
            for p in children {
                ordered.push(p);
            }
        }
    }

    let nav_links: Vec<NavLink> = ordered.iter().map(|p| NavLink { label: p.title.clone(), rel_path: p.rel_path.clone() }).collect();
    let mut html_pages = BTreeMap::new();

    for (idx, item) in ordered.iter().enumerate() {
        let sec = section_by_id(item.section_id);
        let prev = idx.checked_sub(1).and_then(|i| nav_links.get(i));
        let next = nav_links.get(idx + 1);

        let html = match &item.source {
            PageSource::PreRendered { h1, toc, body_html } => render_page(&PageSpec {
                title: &item.title,
                description: &item.description,
                rel_path: &item.rel_path,
                section_title: sec.map_or("Manual", |s| s.title),
                section_rel_path: sec.map(|s| s.index_rel_path),
                h1,
                toc_headings: toc,
                body_html,
                body_has_h1: false,
                prev,
                next,
                meta: &meta,
            }),
            PageSource::Markdown { repo_path, fallback_kind } => {
                let full_path = root.join(repo_path);
                let rendered = if full_path.is_file() {
                    let md_src = std::fs::read_to_string(&full_path).map_err(|e| format!("read {repo_path}: {e}"))?;
                    let resolver = RepoLinkResolver {
                        source_repo_rel: repo_path,
                        current_site_rel: &item.rel_path,
                        repo_root: root,
                        meta: &meta,
                        known_site_targets: &known_targets,
                        broken_links: std::cell::RefCell::new(Vec::new()),
                    };
                    let out = render_markdown(&md_src, &resolver);
                    let broken = resolver.broken_links.into_inner();
                    if let Some(first) = broken.first() {
                        return Err(first.clone());
                    }
                    out
                } else {
                    render_fallback_page(*fallback_kind, &item.rel_path)?
                };
                let has_h1 = rendered.title.is_some();
                let page_title = rendered.title.as_deref().unwrap_or(&item.title);
                render_page(&PageSpec {
                    title: page_title,
                    description: &item.description,
                    rel_path: &item.rel_path,
                    section_title: sec.map_or("Manual", |s| s.title),
                    section_rel_path: sec.map(|s| s.index_rel_path),
                    h1: page_title,
                    toc_headings: &rendered.headings,
                    body_html: &rendered.html,
                    body_has_h1: has_h1,
                    prev,
                    next,
                    meta: &meta,
                })
            }
        };
        html_pages.insert(item.rel_path.clone(), html);
    }

    Ok(GeneratedSite { html_pages, expected_coverage: registries.expected, meta })
}

fn section_by_id(id: &str) -> Option<&'static SectionDef> {
    BETA_SECTIONS.iter().find(|s| s.id == id)
}

fn ordered_content_pages(registry_pages: Vec<RegistryPageSpec>) -> Vec<PageItem> {
    let mut reg_map: BTreeMap<String, RegistryPageSpec> = registry_pages.into_iter().map(|p| (p.rel_path.clone(), p)).collect();
    let mut items = vec![
        md_page(
            "get-started.html",
            "Get Started",
            "System requirements, installing CurveBook, verifying checksums/attestations, and font configuration.",
            "get-started",
            "docs/reference/get-started.md",
            FallbackKind::Generic {
                lead: "Interface-neutral installation, verification, font discovery (`CURVEBOOK_FONTS_DIR`), and initial setup reference.",
                target_registry: Some(("Format & Package Reference", "registry/formats.html")),
            },
        ),
        md_page(
            "desktop-gui.html",
            "Desktop GUI Overview",
            "Overview of desktop welcome workflow, sheets, analyses, graphs, saving, and exporting.",
            "get-started",
            "docs/reference/desktop-gui.md",
            FallbackKind::Generic {
                lead: "Overview of the desktop application layout and entry points paired with the typed UI command catalog.",
                target_registry: Some(("UI Command Catalog", "registry/commands.html#ui-commands")),
            },
        ),
        md_page(
            "mcp-for-beginners.html",
            "MCP Setup for Beginners",
            "Configure an MCP client to connect to CurveBook and verify the connection.",
            "get-started",
            "docs/mcp-for-beginners.md",
            FallbackKind::MustExist,
        ),
        md_page(
            "concepts.html",
            "Concepts & Reactive Pipeline",
            "Project structure, five sheet types, reactive recalculation pipeline, exclusions, checkpoints, and undo.",
            "concepts",
            "docs/reference/concepts.md",
            FallbackKind::Generic {
                lead: "Core CurveBook workspace concepts: projects, linked Data/Info/Result/Graph/Layout sheets, exclusions, hooks, and undo.",
                target_registry: Some(("Table & Analysis Registry", "registry/tables.html")),
            },
        ),
        md_page(
            "tables.html",
            "Data Tables, Formulas & Transforms",
            "Table archetypes, replicate subcolumn modes, Calculated Variables formula language, and data transforms.",
            "tables",
            "docs/reference/tables.md",
            FallbackKind::Generic {
                lead: "Reference for all eight TableKind archetypes, SubcolumnMode layouts, formula operators, and linked data transforms.",
                target_registry: Some(("Table Archetypes & Subcolumn Registry", "registry/tables.html#table-archetypes")),
            },
        ),
        md_page(
            "analyses.html",
            "Statistical Analyses Reference",
            "Complete interface-neutral reference for every CurveBook statistical analysis kind, options, diagnostics, and citations.",
            "analyses",
            "docs/reference/analyses.md",
            FallbackKind::Analyses,
        ),
        md_page(
            "fitting.html",
            "Curve Fitting Reference",
            "Nonlinear regression methods, equation library, weighting, robust/ROUT fitting, bands, custom DSL, and ODE models.",
            "fitting",
            "docs/reference/fitting.md",
            FallbackKind::Generic {
                lead: "Interface-neutral reference for trust-region LM fitting, Poisson deviance, Cauchy/ROUT regression, global sharing, and custom equation DSL.",
                target_registry: Some(("Parametric Equations & Fitting Registry", "registry/equations.html")),
            },
        ),
        md_page(
            "graphs.html",
            "Graphs & Page Layouts",
            "Graph types, compatible table archetypes, axis scales, error bars, colour palettes, and Page Layout rendering.",
            "graphs",
            "docs/reference/graphs.md",
            FallbackKind::Generic {
                lead: "Reference for scientific graph families, axis scale bijections, frame presets, perceptual colormaps, and Page Layouts.",
                target_registry: Some(("Graphs, Scales & Palettes Registry", "registry/graphs.html")),
            },
        ),
        md_page(
            "qol.html",
            "Calculators & Quality-of-Life Tools",
            "Scientific calculators, quick statistics, and workspace quality-of-life capabilities.",
            "qol",
            "docs/reference/qol.md",
            FallbackKind::Generic {
                lead: "Capability reference for built-in scientific calculators, batch inspection, cloning pipelines, and workspace tools.",
                target_registry: Some(("UI & Engine Commands", "registry/commands.html")),
            },
        ),
        md_page(
            "files.html",
            "Files, Import & Export",
            "CurveBook archives, unpacked .curvebook.d directories, templates, autosave recovery, and interchange formats.",
            "files",
            "docs/reference/files.md",
            FallbackKind::Generic {
                lead: "Reference for .curvebook archives, unpacked .curvebook.d directories, .cbtemplate templates, and registered import/export formats.",
                target_registry: Some(("Formats & Schemas Registry", "registry/formats.html")),
            },
        ),
        md_page(
            "automation/mcp.html",
            "MCP Server Reference",
            "CurveBook Model Context Protocol server transports, core tools, toolsets, resources, prompts, and --gui-only policy.",
            "automation",
            "docs/reference/automation/mcp.md",
            FallbackKind::MustExist,
        ),
        md_page(
            "automation/cli.html",
            "CLI Reference (curvebook-cli)",
            "Headless runner subcommands, schema-typed direct command flags, exit codes, and PDF/format flags.",
            "automation",
            "docs/reference/automation/cli.md",
            FallbackKind::MustExist,
        ),
        md_page(
            "automation/control-and-batch.html",
            "Control Protocol, Batch Format & Errors",
            "Loopback --control JSON-lines protocol, curvebook.batch.v1 manifests, run scripts, and engine error codes.",
            "automation",
            "docs/reference/automation/control-and-batch.md",
            FallbackKind::MustExist,
        ),
        md_page(
            "settings.html",
            "Settings & Languages",
            "Preferences JSON Schema keys, keyboard shortcut customization, interface locales, and locale formatting.",
            "settings",
            "docs/reference/settings.md",
            FallbackKind::Generic {
                lead: "Reference for user preferences keys, keybinding overrides, and compiled interface locale tags.",
                target_registry: Some(("Preferences Schema & Locales Registry", "registry/formats.html#schema-preferences")),
            },
        ),
        md_page(
            "privacy-security.html",
            "Privacy, Licences & Security",
            "Zero-telemetry local execution, loopback control rules, dual licensing, and vulnerability reporting.",
            "privacy-security",
            "docs/reference/privacy-security.md",
            FallbackKind::Generic {
                lead: "CurveBook runs strictly locally with zero outbound telemetry, loopback-only control sockets, and dual MIT OR Apache-2.0 licensing.",
                target_registry: None,
            },
        ),
        md_page(
            "troubleshooting.html",
            "Troubleshooting & Known Issues",
            "Common install, font discovery, MCP transport, archive recovery, and solver diagnostics.",
            "troubleshooting",
            "docs/reference/troubleshooting.md",
            FallbackKind::Generic {
                lead: "Diagnostic guide for font setup, MCP stdio/control attachment, archive version guards, and numerical error codes.",
                target_registry: Some(("Engine Error Codes", "registry/tables.html#error-codes")),
            },
        ),
        md_page(
            "glossary.html",
            "Glossary",
            "Definitions of core CurveBook scientific, data-model, and automation terms.",
            "glossary",
            "docs/reference/glossary.md",
            FallbackKind::Glossary,
        ),
    ];

    for (reg_path, section_id) in [
        ("registry/tables.html", "tables"),
        ("registry/equations.html", "fitting"),
        ("registry/graphs.html", "graphs"),
        ("registry/formats.html", "files"),
        ("registry/commands.html", "automation"),
        ("registry/mcp-tools.html", "automation"),
        ("registry/cli.html", "automation"),
    ] {
        if let Some(reg) = reg_map.remove(reg_path) {
            items.push(PageItem {
                rel_path: reg.rel_path,
                title: reg.title,
                description: reg.description,
                section_id,
                source: PageSource::PreRendered { h1: reg.h1, toc: reg.toc_headings, body_html: reg.body_html },
            });
        }
    }
    items
}

fn md_page(
    rel_path: &str,
    title: &str,
    description: &str,
    section_id: &'static str,
    repo_path: &'static str,
    fallback_kind: FallbackKind,
) -> PageItem {
    PageItem {
        rel_path: rel_path.to_owned(),
        title: title.to_owned(),
        description: description.to_owned(),
        section_id,
        source: PageSource::Markdown { repo_path, fallback_kind },
    }
}

fn render_fallback_page(kind: FallbackKind, current_rel: &str) -> Result<super::markdown::RenderedMarkdown, String> {
    let mut html = String::new();
    let mut headings = Vec::new();
    match kind {
        FallbackKind::MustExist => {
            return Err(format!("required Markdown source missing for `{current_rel}`"));
        }
        FallbackKind::Generic { lead, target_registry } => {
            let _ = writeln!(html, "<p class=\"lead\">{}</p>", escape_html(lead));
            html.push_str(
                "<div class=\"callout\"><p>This reference topic mirrors typed metadata from the CurveBook registry generators below.</p></div>\n",
            );
            if let Some((reg_label, reg_target)) = target_registry {
                let _ = writeln!(
                    html,
                    "<p>Open the generated registry page: <a href=\"{}\">{}</a>.</p>",
                    escape_html(&rel_href(current_rel, reg_target)),
                    escape_html(reg_label),
                );
            }
        }
        FallbackKind::Analyses => {
            html.push_str("<p class=\"lead\">Complete statistical analysis catalog covering every CurveBook <code>AnalysisKind</code>.</p>\n");
            let _ = writeln!(
                html,
                "<p>See the generated <a href=\"{}\">Tables, Analyses &amp; Error Codes Registry</a> for tabular summaries.</p>",
                escape_html(&rel_href(current_rel, "registry/tables.html#analysis-kinds")),
            );
            for (slug, name, desc) in all_analysis_kinds_meta() {
                let id = format!("analysis-{slug}");
                headings.push(TocHeading { level: 2, id: id.clone(), text: name.to_owned() });
                let _ = writeln!(html, "<h2 id=\"{id}\" tabindex=\"-1\">{}</h2>", escape_html(name));
                let _ = writeln!(html, "<p>{}</p>", escape_html(desc));
            }
        }
        FallbackKind::Glossary => {
            html.push_str("<p class=\"lead\">Core CurveBook scientific, workspace, and automation terminology.</p>\n");
            let terms = glossary_seed_terms();
            html.push_str("<dl>\n");
            for (term, def) in terms {
                let id = format!("term-{}", github_slug(term));
                headings.push(TocHeading { level: 2, id: id.clone(), text: (*term).to_owned() });
                let _ = writeln!(html, "  <dt id=\"{id}\" tabindex=\"-1\">{}</dt>", escape_html(term));
                let _ = writeln!(html, "  <dd>{}</dd>", escape_html(def));
            }
            html.push_str("</dl>\n");
        }
    }
    Ok(super::markdown::RenderedMarkdown { html, headings, title: None })
}

fn glossary_seed_terms() -> &'static [(&'static str, &'static str)] {
    &[
        ("Checkpoint", "Named immutable snapshot recorded inside a ProjectDocument for restoring earlier project states."),
        (
            "Clean-room implementation",
            "Independent implementation derived solely from published literature and standards without inspecting or reusing proprietary software or fixtures.",
        ),
        ("Control Protocol", "Loopback-only 127.0.0.1 JSON-lines channel authenticated with a bearer token for live desktop automation."),
        ("DataSheet", "Typed scientific grid belonging to one of the eight TableKind archetypes with optional non-destructive cell exclusions."),
        ("Exclusion", "Non-destructive cell mask retaining raw observations visually while omitting them from analyses and plotted summaries."),
        ("GraphSheet", "Scientific figure specification referencing upstream DataSheets and ResultSheets with axis scales and palettes."),
        (
            "GUI-only policy",
            "Strict --gui-only enforcement restricting MCP and control clients exclusively to human-equivalent interactive GUI tools.",
        ),
        ("Hook", "Live reference linking a parameter, threshold, baseline, or text placeholder to an upstream InfoSheet or ResultSheet value."),
        ("InfoSheet", "Structured experimental constants table paired with freeform Markdown laboratory notes."),
        ("LayoutSheet", "Multi-panel publication composition placing graphs, tables, and text annotations onto physical paper coordinates."),
        ("MCP (Model Context Protocol)", "JSON-RPC 2.0 protocol over stdio exposing CurveBook tools, resources, and workflow prompts."),
        ("Reactive DAG", "Dependency graph automatically recomputing downstream transforms, analyses, graphs, and layouts when inputs change."),
        ("ResultSheet", "Read-only tabbed computational output produced by an analysis spec and tagged with its upstream parameters and status."),
        ("Workdir guard", "Canonicalized filesystem containment policy confining CLI and MCP file reads and writes within --workdir."),
    ]
}

fn collect_glossary_az_entries(root: &Path) -> Vec<AzEntry> {
    let glossary_md = root.join("docs/reference/glossary.md");
    if let Ok(text) = std::fs::read_to_string(&glossary_md) {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let rendered = render_markdown(&text, &super::markdown::IdentityResolver);
        for h in rendered.headings {
            if h.level >= 2 && seen.insert(h.id.clone()) {
                out.push(AzEntry { term: h.text, category: String::from("Glossary"), rel_href: format!("glossary.html#{}", h.id) });
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    glossary_seed_terms()
        .iter()
        .map(|(term, _)| AzEntry {
            term: (*term).to_owned(),
            category: String::from("Glossary"),
            rel_href: format!("glossary.html#term-{}", github_slug(term)),
        })
        .collect()
}

fn build_home_page(pages: &[PageItem]) -> PageItem {
    let mut body = String::with_capacity(8192);
    body.push_str("<p class=\"lead\">Complete static reference manual for CurveBook scientific tables, statistical methods, nonlinear regression, publication graphics, MCP/CLI automation, and contributor guides on GitHub.</p>\n");
    body.push_str("<div class=\"section-grid\">\n");
    for sec in BETA_SECTIONS {
        let badge = if sec.id == "contribute" {
            format!("{} GitHub guides", CONTRIBUTOR_GITHUB_GUIDES.len())
        } else {
            let count = pages.iter().filter(|p| p.section_id == sec.id).count();
            format!("{count} pages")
        };
        let _ = writeln!(body, "  <section class=\"section-card\">");
        let _ = writeln!(body, "    <h2><a href=\"{}\">{}</a></h2>", escape_html(sec.index_rel_path), escape_html(sec.title));
        let _ = writeln!(body, "    <p>{}</p>", escape_html(sec.summary));
        let _ = writeln!(body, "    <p><span class=\"badge\">{badge}</span></p>");
        let _ = writeln!(body, "  </section>");
    }
    body.push_str("</div>\n");

    PageItem {
        rel_path: String::from("index.html"),
        title: String::from("CurveBook Reference Manual"),
        description: String::from(
            "Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
        ),
        section_id: "get-started",
        source: PageSource::PreRendered { h1: String::from("CurveBook Reference Manual"), toc: Vec::new(), body_html: body },
    }
}

fn build_section_index_page(sec: &SectionDef, children: &[&PageItem]) -> PageItem {
    let mut body = String::with_capacity(4096);
    let _ = writeln!(body, "<p class=\"lead\">{}</p>", escape_html(sec.summary));
    if sec.id == "contribute" {
        body.push_str("<div class=\"callout\"><p>Contributor specifications are maintained continuously against the live <code>main</code> branch on GitHub.</p></div>\n");
    }
    body.push_str("<div class=\"section-grid\">\n");
    if sec.id == "contribute" {
        for &(title, repo_path, summary) in CONTRIBUTOR_GITHUB_GUIDES {
            let _ = writeln!(body, "  <section class=\"section-card\">");
            let _ = writeln!(
                body,
                "    <h2><a href=\"{GITHUB_REPO_BASE}/blob/main/{repo_path}\">{} <span aria-hidden=\"true\">↗</span></a></h2>",
                escape_html(title),
            );
            let _ = writeln!(body, "    <p>{}</p>", escape_html(summary));
            let _ = writeln!(body, "  </section>");
        }
    }
    for child in children {
        let _ = writeln!(body, "  <section class=\"section-card\">");
        let _ = writeln!(
            body,
            "    <h2><a href=\"{}\">{}</a></h2>",
            escape_html(&rel_href(sec.index_rel_path, &child.rel_path)),
            escape_html(&child.title),
        );
        let _ = writeln!(body, "    <p>{}</p>", escape_html(&child.description));
        let _ = writeln!(body, "  </section>");
    }
    if sec.id == "glossary" {
        let _ = writeln!(body, "  <section class=\"section-card\">");
        let _ = writeln!(body, "    <h2><a href=\"az-index.html\">Complete A–Z Symbol Index</a></h2>");
        let _ = writeln!(
            body,
            "    <p>Alphabetical index across every analysis kind, fit equation, graph type, table type, engine/UI command, MCP tool, CLI command, format, and setting key.</p>"
        );
        let _ = writeln!(body, "  </section>");
    }
    body.push_str("</div>\n");

    PageItem {
        rel_path: sec.index_rel_path.to_owned(),
        title: sec.title.to_owned(),
        description: sec.summary.to_owned(),
        section_id: sec.id,
        source: PageSource::PreRendered { h1: sec.title.to_owned(), toc: Vec::new(), body_html: body },
    }
}

fn build_az_index_page(entries: &[AzEntry]) -> PageItem {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.term.to_ascii_lowercase().cmp(&b.term.to_ascii_lowercase()).then_with(|| a.category.cmp(&b.category)));
    sorted.dedup();

    let mut grouped: BTreeMap<char, Vec<&AzEntry>> = BTreeMap::new();
    for entry in &sorted {
        let ch = entry.term.chars().find(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).unwrap_or('#');
        let bucket = if ch.is_ascii_alphabetic() { ch } else { '#' };
        grouped.entry(bucket).or_default().push(entry);
    }

    let mut body = String::with_capacity(65_536);
    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete script-free alphabetical index covering <strong>{}</strong> documented entries across analyses, fit equations, graph types, table types, engine/UI commands, MCP tools, CLI commands, formats, settings, and glossary terms.</p>",
        sorted.len(),
    );

    let mut toc = Vec::new();
    body.push_str("<ul class=\"az-letters\" aria-label=\"Alphabetical jump links\">\n");
    for letter in 'A'..='Z' {
        if grouped.contains_key(&letter) {
            let _ = writeln!(body, "  <li><a href=\"#letter-{letter}\">{letter}</a></li>");
        } else {
            let _ = writeln!(body, "  <li><span>{letter}</span></li>");
        }
    }
    if grouped.contains_key(&'#') {
        body.push_str("  <li><a href=\"#letter-other\">#</a></li>\n");
    }
    body.push_str("</ul>\n");

    for (bucket, items) in grouped {
        let (id, heading) =
            if bucket == '#' { (String::from("letter-other"), String::from("#")) } else { (format!("letter-{bucket}"), bucket.to_string()) };
        toc.push(TocHeading { level: 2, id: id.clone(), text: heading.clone() });
        let _ = writeln!(body, "<h2 id=\"{id}\" tabindex=\"-1\">{heading}</h2>\n<ul>");
        for item in items {
            let _ = writeln!(
                body,
                "  <li><a href=\"{}\">{}</a> <span class=\"badge\">{}</span></li>",
                escape_html(&item.rel_href),
                escape_html(&item.term),
                escape_html(&item.category),
            );
        }
        body.push_str("</ul>\n");
    }

    PageItem {
        rel_path: String::from("az-index.html"),
        title: String::from("A–Z Reference Index"),
        description: String::from(
            "Alphabetical index of CurveBook analyses, equations, graph kinds, tables, commands, MCP tools, CLI flags, formats, and preferences.",
        ),
        section_id: "glossary",
        source: PageSource::PreRendered { h1: String::from("A–Z Reference Index"), toc, body_html: body },
    }
}
