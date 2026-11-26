//! Beta site map assembly, Markdown page ingestion, link rewriting, A–Z index, and fallback bridges.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use super::markdown::{LinkResolver, TocHeading, escape_html, github_slug, render_markdown};
use super::registries::{AzEntry, ExpectedCoverage, RegistryBundle, RegistryPageSpec, all_analysis_kinds_meta, generate_registry_pages};
use super::template::{NavLink, PageSpec, SidebarChild, SidebarGroup, SidebarSection, SiteMeta, rel_href, render_page};
use crate::scan_util::{is_real_dir, is_regular_file};

const GITHUB_REPO_BASE: &str = "https://github.com/HauserGroup/curvebook";

/// Top-level manual navigation group owning each section in [`BETA_SECTIONS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavGroup {
    GettingStarted,
    Reference,
    Automation,
    Support,
}

impl NavGroup {
    const ALL: [Self; 4] = [Self::GettingStarted, Self::Reference, Self::Automation, Self::Support];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GettingStarted => "Getting started",
            Self::Reference => "Reference",
            Self::Automation => "Automation",
            Self::Support => "Support",
        }
    }

    #[must_use]
    pub const fn nav_target(self) -> &'static str {
        match self {
            Self::GettingStarted => "section-get-started.html",
            Self::Reference => "reference.html",
            Self::Automation => "section-automation.html",
            Self::Support => "support.html",
        }
    }
}

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
    nav_group: NavGroup,
    summary: &'static str,
}

const BETA_SECTIONS: &[SectionDef] = &[
    SectionDef {
        id: "get-started",
        title: "Getting started",
        index_rel_path: "section-get-started.html",
        nav_group: NavGroup::GettingStarted,
        summary: "Installation, checksum/attestation verification, font configuration, desktop onboarding, and MCP client setup.",
    },
    SectionDef {
        id: "concepts",
        title: "Concepts",
        index_rel_path: "section-concepts.html",
        nav_group: NavGroup::Reference,
        summary: "Projects, five sheet families, reactive recalculation pipeline, non-destructive exclusions, checkpoints, and undo.",
    },
    SectionDef {
        id: "tables",
        title: "Data tables & formulas",
        index_rel_path: "section-tables.html",
        nav_group: NavGroup::Reference,
        summary: "Table archetypes, subcolumn and replicate modes, Calculated Variables formulas, and linked data transforms.",
    },
    SectionDef {
        id: "analyses",
        title: "Analyses",
        index_rel_path: "section-analyses.html",
        nav_group: NavGroup::Reference,
        summary: "Statistical tests, assumptions, diagnostics, edge-case behavior, peer-reviewed citations, and oracle verification.",
    },
    SectionDef {
        id: "fitting",
        title: "Curve fitting",
        index_rel_path: "section-fitting.html",
        nav_group: NavGroup::Reference,
        summary: "Built-in equation library, nonlinear solvers, weighting schemes, robust/ROUT fitting, bands, and custom DSL/ODE models.",
    },
    SectionDef {
        id: "graphs",
        title: "Graphs & Page Layouts",
        index_rel_path: "section-graphs.html",
        nav_group: NavGroup::Reference,
        summary: "Plot families, scales, frames, palettes, multi-panel Page Layouts, and publication vector/raster rendering.",
    },
    SectionDef {
        id: "qol",
        title: "Calculators & quality-of-life tools",
        index_rel_path: "section-qol.html",
        nav_group: NavGroup::Reference,
        summary: "Quick statistics, scientific calculators, command palette, cloning pipelines, and quality-of-life capabilities.",
    },
    SectionDef {
        id: "files",
        title: "Files, import & export",
        index_rel_path: "section-files.html",
        nav_group: NavGroup::Reference,
        summary: "Archive/directory/template formats, interchange importers, export formats, autosave recovery, and JSON Schemas.",
    },
    SectionDef {
        id: "settings",
        title: "Settings & languages",
        index_rel_path: "section-settings.html",
        nav_group: NavGroup::Reference,
        summary: "Preference schema keys, shortcut customization, supported interface locales, and locale formatting.",
    },
    SectionDef {
        id: "glossary",
        title: "Glossary",
        index_rel_path: "section-glossary.html",
        nav_group: NavGroup::Reference,
        summary: "Defined terminology across statistical and software workflows plus the complete A–Z index of symbols.",
    },
    SectionDef {
        id: "automation",
        title: "Automation",
        index_rel_path: "section-automation.html",
        nav_group: NavGroup::Automation,
        summary: "MCP server, curvebook-cli subcommands, loopback --control protocol, curvebook.batch.v1 manifests, and command catalogs.",
    },
    SectionDef {
        id: "troubleshooting",
        title: "Troubleshooting & known issues",
        index_rel_path: "section-troubleshooting.html",
        nav_group: NavGroup::Support,
        summary: "Diagnosing font, MCP, archive recovery, and solver diagnostics alongside documented release limits.",
    },
    SectionDef {
        id: "privacy-security",
        title: "Privacy, licences & security",
        index_rel_path: "section-privacy-security.html",
        nav_group: NavGroup::Support,
        summary: "Offline/zero-telemetry operation, loopback socket policy, dual licensing, clean-room policy, and security disclosures.",
    },
    SectionDef {
        id: "contribute",
        title: "Contribute",
        index_rel_path: "section-contribute.html",
        nav_group: NavGroup::Support,
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
    (
        "CurveBook Design Language",
        "docs/design-language.md",
        "OKLab/OKLCH perceptual colour tokens, fixed scales, ActionRank priority hierarchy, and UI gates.",
    ),
    ("CI Stages & Gates", "docs/ci.md", "Repository cargo xtask ci stage matrix, budgets, and compliance scanners."),
    (
        "Releasing & Snapshot Publication",
        "docs/release/releasing.md",
        "Cutting releases, artifact verification, checksums, and publishing versioned docs snapshots.",
    ),
];

/// Single source of truth for hand-written user reference guides shared by `sitemap.rs` and `llms.rs`.
pub(crate) struct GuideDescriptor {
    pub(crate) rel_path: &'static str,
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
    pub(crate) section_id: &'static str,
    pub(crate) repo_path: &'static str,
    fallback_kind: FallbackKind,
}

const fn guide(
    rel_path: &'static str,
    title: &'static str,
    description: &'static str,
    section_id: &'static str,
    repo_path: &'static str,
    fallback_kind: FallbackKind,
) -> GuideDescriptor {
    GuideDescriptor { rel_path, title, description, section_id, repo_path, fallback_kind }
}

pub(crate) const MANUAL_GUIDES: &[GuideDescriptor] = &[
    guide(
        "get-started.html",
        "Install CurveBook",
        "System requirements, installing CurveBook, verifying checksums/attestations, and font configuration.",
        "get-started",
        "docs/reference/get-started.md",
        FallbackKind::Generic {
            lead: "Interface-neutral installation, verification, font discovery (`CURVEBOOK_FONTS_DIR`), and initial setup reference.",
            target_registry: Some(("Format & Package Reference", "registry/formats.html")),
        },
    ),
    guide(
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
    guide(
        "mcp-for-beginners.html",
        "MCP Setup for Beginners",
        "Configure an MCP client to connect to CurveBook and verify the connection.",
        "get-started",
        "docs/mcp-for-beginners.md",
        FallbackKind::MustExist,
    ),
    guide(
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
    guide(
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
    guide(
        "analyses.html",
        "Statistical Analyses Reference",
        "Complete interface-neutral reference for every CurveBook statistical analysis kind, options, diagnostics, and citations.",
        "analyses",
        "docs/reference/analyses.md",
        FallbackKind::Analyses,
    ),
    guide(
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
    guide(
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
    guide(
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
    guide(
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
    guide(
        "automation/mcp.html",
        "MCP Server Reference",
        "CurveBook Model Context Protocol server transports, core tools, toolsets, resources, and prompts.",
        "automation",
        "docs/reference/automation/mcp.md",
        FallbackKind::MustExist,
    ),
    guide(
        "automation/cli.html",
        "CLI Reference (curvebook-cli)",
        "Headless runner subcommands, schema-typed direct command flags, exit codes, and PDF/format flags.",
        "automation",
        "docs/reference/automation/cli.md",
        FallbackKind::MustExist,
    ),
    guide(
        "automation/control-and-batch.html",
        "Control Protocol, Batch Format & Errors",
        "Loopback --control JSON-lines protocol, curvebook.batch.v1 manifests, run scripts, and engine error codes.",
        "automation",
        "docs/reference/automation/control-and-batch.md",
        FallbackKind::MustExist,
    ),
    guide(
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
    guide(
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
    guide(
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
    guide(
        "glossary.html",
        "Glossary",
        "Definitions of core CurveBook scientific, data-model, and automation terms.",
        "glossary",
        "docs/reference/glossary.md",
        FallbackKind::Glossary,
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
    section_id: Option<&'static str>,
    nav_group: Option<NavGroup>,
    is_hub: bool,
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

fn starts_with_scheme_ci(raw: &str, schemes: &[&str]) -> bool {
    schemes.iter().any(|scheme| raw.get(..scheme.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme)))
}

impl LinkResolver for RepoLinkResolver<'_> {
    fn resolve_href(&self, raw_href: &str) -> String {
        if raw_href.starts_with('#') || starts_with_scheme_ci(raw_href, &["http://", "https://", "mailto:"]) {
            return raw_href.to_owned();
        }
        let (target_part, frag_part) = raw_href.split_once('#').map_or((raw_href, None), |(t, f)| (t, Some(f)));
        if target_part.is_empty() {
            return frag_suffix(frag_part);
        }

        let clean_target = target_part.trim_end_matches('/');
        if clean_target.ends_with(".html") && (clean_target.starts_with("registry/") || clean_target.starts_with("../registry/")) {
            let suffix = clean_target.trim_start_matches("../");
            return format!("{}{}", rel_href(self.current_site_rel, suffix), frag_suffix(frag_part));
        }

        let source_dir = Path::new(self.source_repo_rel).parent().unwrap_or_else(|| Path::new(""));
        let Some(normalized) = normalize_repo_rel(source_dir, target_part) else {
            self.broken_links.borrow_mut().push(format!("{}: relative link `{raw_href}` escapes repository root", self.source_repo_rel));
            return String::from("#");
        };

        if let Some(mapped_site_rel) = self.known_site_targets.get(normalized.as_str()) {
            let frag = frag_part.map(|f| format!("#{f}")).or_else(|| default_schema_fragment(&normalized).map(str::to_owned)).unwrap_or_default();
            return format!("{}{frag}", rel_href(self.current_site_rel, mapped_site_rel));
        }

        if let Some((snap_rel, preview_rel)) = match normalized.as_str() {
            "docs/index.html" => Some(("index.html", "index.html")),
            "docs/privacy.html" => Some(("privacy.html", "privacy-security.html")),
            _ => None,
        } {
            let base = if self.meta.is_release_snapshot {
                format!("{}{snap_rel}", super::template::asset_prefix(self.current_site_rel, true))
            } else {
                rel_href(self.current_site_rel, preview_rel)
            };
            return format!("{base}{}", frag_suffix(frag_part));
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
        let kind = if target_part.ends_with('/') || is_real_dir(&target_path) { "tree" } else { "blob" };
        format!("{GITHUB_REPO_BASE}/{kind}/main/{trimmed}{}", frag_suffix(frag_part))
    }

    fn resolve_image(&self, raw_src: &str) -> String {
        if starts_with_scheme_ci(raw_src, &["http://", "https://"]) {
            return raw_src.to_owned();
        }
        let source_dir = Path::new(self.source_repo_rel).parent().unwrap_or_else(|| Path::new(""));
        let assets = super::template::asset_prefix(self.current_site_rel, self.meta.is_release_snapshot);
        let Some(normalized) = normalize_repo_rel(source_dir, raw_src) else {
            self.broken_links.borrow_mut().push(format!("{}: image path `{raw_src}` escapes repository root", self.source_repo_rel));
            return String::from("#");
        };
        if let Some(img_name) = normalized.strip_prefix("docs/images/") {
            if !is_regular_file(&self.repo_root.join("docs/images").join(img_name)) {
                self.broken_links
                    .borrow_mut()
                    .push(format!("{}: missing image asset `{raw_src}` (resolved repo path `{normalized}` does not exist)", self.source_repo_rel));
            }
            return format!("{assets}images/{img_name}");
        }
        if let Some(file_name) = Path::new(&normalized).file_name().and_then(|n| n.to_str())
            && is_regular_file(&self.repo_root.join("docs/images").join(file_name))
        {
            return format!("{assets}images/{file_name}");
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

fn normalize_repo_rel(base_dir: &Path, raw: &str) -> Option<String> {
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
        if !push_segments(abs, &mut parts) {
            return None;
        }
    } else if !push_segments(stripped, &mut parts) {
        return None;
    }
    Some(parts.join("/"))
}

fn push_segments(path: &str, parts: &mut Vec<String>) -> bool {
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return false;
                }
            }
            other => parts.push(other.to_owned()),
        }
    }
    true
}

fn build_known_site_targets() -> BTreeMap<&'static str, &'static str> {
    let mut map: BTreeMap<&'static str, &'static str> = MANUAL_GUIDES.iter().map(|g| (g.repo_path, g.rel_path)).collect();
    map.extend([
        ("docs/mcp.md", "automation/mcp.html"),
        ("docs/control-protocol.md", "automation/control-and-batch.html"),
        ("docs/commands.md", "registry/commands.html"),
        ("docs/schemas/preferences.schema.json", "registry/formats.html"),
        ("docs/schemas/batch-v1.schema.json", "registry/formats.html"),
        ("docs/schemas/document.schema.json", "registry/formats.html"),
        ("docs/schemas/archive-manifest.schema.json", "registry/formats.html"),
        ("docs/schemas/journal-record.schema.json", "registry/formats.html"),
    ]);
    map
}

fn collect_defined_analysis_anchors(root: &Path) -> BTreeSet<String> {
    let analyses_md = root.join("docs/reference/analyses.md");
    if !is_regular_file(&analyses_md) {
        return BTreeSet::new();
    }
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

    let mut by_section: BTreeMap<&'static str, Vec<PageItem>> = BTreeMap::new();
    for p in pages.drain(..) {
        if let Some(sec_id) = p.section_id {
            by_section.entry(sec_id).or_default().push(p);
        }
    }

    let sidebar_groups = build_sidebar_groups(&by_section);
    let az_count = count_unique_az_entries(&az_entries);

    let mut ordered: Vec<PageItem> = Vec::with_capacity(MANUAL_GUIDES.len() + BETA_SECTIONS.len() + 12);
    ordered.push(build_home_page(&by_section));
    ordered.push(build_reference_page(&by_section, az_count));
    ordered.push(build_support_page(&by_section));

    for sec in BETA_SECTIONS {
        let children = by_section.remove(sec.id).unwrap_or_default();
        let child_refs: Vec<&PageItem> = children.iter().collect();
        ordered.push(build_section_index_page(sec, &child_refs));
        ordered.extend(children);
        if sec.id == "glossary" {
            ordered.push(build_az_index_page(&az_entries));
        }
    }

    let pager_links: Vec<NavLink> =
        ordered.iter().filter(|p| !p.is_hub).map(|p| NavLink { label: p.title.clone(), rel_path: p.rel_path.clone() }).collect();
    let pager_pos: BTreeMap<&str, usize> = pager_links.iter().enumerate().map(|(idx, link)| (link.rel_path.as_str(), idx)).collect();
    let mut html_pages = BTreeMap::new();

    for item in &ordered {
        let sec = item.section_id.and_then(section_by_id);
        let pos = if item.is_hub { None } else { pager_pos.get(item.rel_path.as_str()).copied() };
        let prev = pos.and_then(|i| i.checked_sub(1)).and_then(|i| pager_links.get(i));
        let next = pos.and_then(|i| pager_links.get(i + 1));
        let (title, h1, toc, body_html, body_has_h1) = match &item.source {
            PageSource::PreRendered { h1, toc, body_html } => (item.title.clone(), h1.clone(), toc.clone(), body_html.clone(), false),
            PageSource::Markdown { repo_path, fallback_kind } => {
                let full_path = root.join(repo_path);
                if let Ok(meta) = std::fs::symlink_metadata(&full_path)
                    && meta.file_type().is_symlink()
                {
                    return Err(format!("{repo_path}: Markdown source must not be a symlink"));
                }
                let rendered = if is_regular_file(&full_path) {
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
                    if let Some(first) = resolver.broken_links.into_inner().into_iter().next() {
                        return Err(first);
                    }
                    out
                } else {
                    render_fallback_page(*fallback_kind, &item.rel_path)?
                };
                let has_h1 = rendered.title.is_some();
                let page_title = rendered.title.unwrap_or_else(|| item.title.clone());
                (page_title.clone(), page_title, rendered.headings, rendered.html, has_h1)
            }
        };
        let html = render_page(&PageSpec {
            title: &title,
            description: &item.description,
            rel_path: &item.rel_path,
            section_title: sec.map_or("Manual", |s| s.title),
            section_rel_path: sec.map(|s| s.index_rel_path),
            nav_group: item.nav_group.or_else(|| sec.map(|s| s.nav_group)),
            active_section_id: item.section_id,
            sidebar_groups: &sidebar_groups,
            h1: &h1,
            toc_headings: &toc,
            body_html: &body_html,
            body_has_h1,
            prev,
            next,
            meta: &meta,
        });
        html_pages.insert(item.rel_path.clone(), html);
    }

    Ok(GeneratedSite { html_pages, expected_coverage: registries.expected, meta })
}

fn section_by_id(id: &str) -> Option<&'static SectionDef> {
    BETA_SECTIONS.iter().find(|s| s.id == id)
}

fn section_target_href(sec: &SectionDef, by_section: &BTreeMap<&'static str, Vec<PageItem>>) -> String {
    match by_section.get(sec.id).map(Vec::as_slice) {
        Some([single]) => single.rel_path.clone(),
        _ => sec.index_rel_path.to_owned(),
    }
}

fn build_sidebar_groups(by_section: &BTreeMap<&'static str, Vec<PageItem>>) -> Vec<SidebarGroup> {
    NavGroup::ALL
        .into_iter()
        .map(|group| {
            let sections = BETA_SECTIONS
                .iter()
                .filter(|sec| sec.nav_group == group)
                .map(|sec| {
                    let items = by_section.get(sec.id).map(Vec::as_slice).unwrap_or(&[]);
                    let children = if items.len() > 1 {
                        items.iter().map(|p| SidebarChild { title: p.title.clone(), rel_path: p.rel_path.clone() }).collect()
                    } else {
                        Vec::new()
                    };
                    SidebarSection { id: sec.id, title: sec.title.to_owned(), href: section_target_href(sec, by_section), children }
                })
                .collect();
            SidebarGroup { group, sections }
        })
        .collect()
}

fn ordered_content_pages(registry_pages: Vec<RegistryPageSpec>) -> Vec<PageItem> {
    let mut reg_map: BTreeMap<String, RegistryPageSpec> = registry_pages.into_iter().map(|p| (p.rel_path.clone(), p)).collect();
    let mut items: Vec<PageItem> = MANUAL_GUIDES
        .iter()
        .map(|g| PageItem {
            rel_path: g.rel_path.to_owned(),
            title: g.title.to_owned(),
            description: g.description.to_owned(),
            section_id: Some(g.section_id),
            nav_group: section_by_id(g.section_id).map(|s| s.nav_group),
            is_hub: false,
            source: PageSource::Markdown { repo_path: g.repo_path, fallback_kind: g.fallback_kind },
        })
        .collect();

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
                section_id: Some(section_id),
                nav_group: section_by_id(section_id).map(|s| s.nav_group),
                is_hub: false,
                source: PageSource::PreRendered { h1: reg.h1, toc: reg.toc_headings, body_html: reg.body_html },
            });
        }
    }
    items
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
    if is_regular_file(&glossary_md)
        && let Ok(text) = std::fs::read_to_string(&glossary_md)
    {
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

const HOME_SECTION_GROUPS: &[(&str, &str, &[&str])] = &[
    ("group-start-here", "Start Here", &["get-started", "concepts"]),
    ("group-scientific-reference", "Scientific & Graphing Reference", &["tables", "analyses", "fitting", "graphs", "qol"]),
    ("group-files-automation-settings", "Files, Automation & Settings", &["files", "automation", "settings"]),
    ("group-support-contribute", "Support & Contribute", &["privacy-security", "troubleshooting", "glossary", "contribute"]),
];

const REFERENCE_SECTION_GROUPS: &[(&str, &str, &[&str])] = &[
    ("group-core-concepts-data", "Core concepts & data", &["concepts", "tables"]),
    ("group-stats-fitting", "Statistics & curve fitting", &["analyses", "fitting"]),
    ("group-graphs-files-settings", "Graphs, files & settings", &["graphs", "qol", "files", "settings"]),
    ("group-terminology", "Terminology", &["glossary"]),
];

fn format_page_count(count: usize) -> String {
    if count == 1 { String::from("1 page") } else { format!("{count} pages") }
}

fn section_badge(sec: &SectionDef, by_section: &BTreeMap<&'static str, Vec<PageItem>>) -> String {
    if sec.id == "contribute" {
        format!("{} GitHub guides", CONTRIBUTOR_GITHUB_GUIDES.len())
    } else {
        let count = by_section.get(sec.id).map_or(0, Vec::len);
        format_page_count(count)
    }
}

fn append_section_cards_group(
    body: &mut String,
    group_id: &str,
    group_title: &str,
    section_ids: &[&str],
    by_section: &BTreeMap<&'static str, Vec<PageItem>>,
    extra_terminology_az: Option<usize>,
) {
    let _ = writeln!(body, "<section class=\"section-group\" aria-labelledby=\"{group_id}\">");
    let _ = writeln!(body, "  <h2 id=\"{group_id}\" tabindex=\"-1\">{}</h2>", escape_html(group_title));
    body.push_str("  <div class=\"section-grid\">\n");
    for &sec_id in section_ids {
        let Some(sec) = section_by_id(sec_id) else {
            continue;
        };
        let href = section_target_href(sec, by_section);
        let badge = section_badge(sec, by_section);
        let _ = writeln!(body, "    <section class=\"section-card\">");
        let _ = writeln!(body, "      <h3><a href=\"{}\">{}</a></h3>", escape_html(&href), escape_html(sec.title));
        let _ = writeln!(body, "      <p>{}</p>", escape_html(sec.summary));
        let _ = writeln!(body, "      <p><span class=\"badge\">{badge}</span></p>");
        let _ = writeln!(body, "    </section>");
    }
    if let Some(az_count) = extra_terminology_az {
        body.push_str("    <section class=\"section-card\">\n");
        body.push_str("      <h3><a href=\"az-index.html\">A–Z Reference Index</a></h3>\n");
        body.push_str("      <p>Alphabetical symbol directory covering every analysis kind, fit equation, graph type, table type, command, MCP tool, CLI flag, format, and setting key.</p>\n");
        let _ = writeln!(body, "      <p><span class=\"badge\">{az_count} entries</span></p>");
        body.push_str("    </section>\n");
    }
    body.push_str("  </div>\n");
    body.push_str("</section>\n");
}

fn build_home_page(by_section: &BTreeMap<&'static str, Vec<PageItem>>) -> PageItem {
    let mut body = String::with_capacity(8192);
    body.push_str("<p class=\"lead\">Complete static reference manual for CurveBook scientific tables, statistical methods, nonlinear regression, publication graphics, MCP/CLI automation, and contributor guides on GitHub.</p>\n");
    for &(group_id, group_title, section_ids) in HOME_SECTION_GROUPS {
        append_section_cards_group(&mut body, group_id, group_title, section_ids, by_section, None);
    }

    PageItem {
        rel_path: String::from("index.html"),
        title: String::from("CurveBook Reference Manual"),
        description: String::from(
            "Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
        ),
        section_id: None,
        nav_group: None,
        is_hub: true,
        source: PageSource::PreRendered { h1: String::from("CurveBook Reference Manual"), toc: Vec::new(), body_html: body },
    }
}

fn build_reference_page(by_section: &BTreeMap<&'static str, Vec<PageItem>>, az_count: usize) -> PageItem {
    let mut body = String::with_capacity(6144);
    body.push_str("<p class=\"lead\">Scientific and workspace reference across core concepts, data tables, statistical analyses, curve fitting, graphs, calculators, files, settings, and terminology.</p>\n");
    for &(group_id, group_title, section_ids) in REFERENCE_SECTION_GROUPS {
        let extra = (group_id == "group-terminology").then_some(az_count);
        append_section_cards_group(&mut body, group_id, group_title, section_ids, by_section, extra);
    }

    PageItem {
        rel_path: String::from("reference.html"),
        title: String::from("Reference"),
        description: String::from(
            "Grouped reference directory for CurveBook concepts, data tables, statistical analyses, curve fitting, graphs, calculators, files, settings, and glossary.",
        ),
        section_id: None,
        nav_group: Some(NavGroup::Reference),
        is_hub: true,
        source: PageSource::PreRendered { h1: String::from("Reference"), toc: Vec::new(), body_html: body },
    }
}

fn build_support_page(by_section: &BTreeMap<&'static str, Vec<PageItem>>) -> PageItem {
    let mut body = String::with_capacity(3072);
    body.push_str("<p class=\"lead\">Diagnostic guides, offline privacy and licensing commitments, security reporting, and contributor resources on GitHub.</p>\n");
    body.push_str("<div class=\"section-grid\">\n");
    for sec_id in ["troubleshooting", "privacy-security", "contribute"] {
        if let Some(sec) = section_by_id(sec_id) {
            let href = section_target_href(sec, by_section);
            let badge = section_badge(sec, by_section);
            let _ = writeln!(body, "  <section class=\"section-card\">");
            let _ = writeln!(body, "    <h2><a href=\"{}\">{}</a></h2>", escape_html(&href), escape_html(sec.title));
            let _ = writeln!(body, "    <p>{}</p>", escape_html(sec.summary));
            let _ = writeln!(body, "    <p><span class=\"badge\">{badge}</span></p>");
            let _ = writeln!(body, "  </section>");
        }
    }
    body.push_str("</div>\n");

    PageItem {
        rel_path: String::from("support.html"),
        title: String::from("Support"),
        description: String::from(
            "Troubleshooting, known issues, offline privacy guarantees, dual licensing, security reporting, and contributor guides.",
        ),
        section_id: None,
        nav_group: Some(NavGroup::Support),
        is_hub: true,
        source: PageSource::PreRendered { h1: String::from("Support"), toc: Vec::new(), body_html: body },
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
        title: format!("{} (Overview)", sec.title),
        description: sec.summary.to_owned(),
        section_id: Some(sec.id),
        nav_group: Some(sec.nav_group),
        is_hub: true,
        source: PageSource::PreRendered { h1: sec.title.to_owned(), toc: Vec::new(), body_html: body },
    }
}

fn sort_and_dedup_az(entries: &[AzEntry]) -> Vec<AzEntry> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| {
        a.term
            .to_ascii_lowercase()
            .cmp(&b.term.to_ascii_lowercase())
            .then_with(|| a.category.cmp(&b.category))
            .then_with(|| a.term.cmp(&b.term))
            .then_with(|| a.rel_href.cmp(&b.rel_href))
    });
    sorted.dedup();
    sorted
}

fn count_unique_az_entries(entries: &[AzEntry]) -> usize {
    sort_and_dedup_az(entries).len()
}

fn build_az_index_page(entries: &[AzEntry]) -> PageItem {
    let sorted = sort_and_dedup_az(entries);

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
        section_id: None,
        nav_group: None,
        is_hub: false,
        source: PageSource::PreRendered { h1: String::from("A–Z Reference Index"), toc: Vec::new(), body_html: body },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{AzEntry, LinkResolver, PageSource, RepoLinkResolver, SiteMeta, build_az_index_page, normalize_repo_rel};

    #[test]
    fn aud_12_26_normalize_repo_rel_rejects_underflow_and_schemes_are_case_insensitive() {
        assert_eq!(normalize_repo_rel(Path::new("docs/reference"), "../../README.md").as_deref(), Some("README.md"));
        assert_eq!(normalize_repo_rel(Path::new("docs/reference"), "../../../../../../README.md"), None);

        let meta = SiteMeta {
            version_label: String::from("v0.1.0"),
            commit_sha: None,
            release_date: None,
            is_release_snapshot: false,
            is_tagged_release: false,
        };
        let known = BTreeMap::new();
        let resolver = RepoLinkResolver {
            source_repo_rel: "docs/reference/get-started.md",
            current_site_rel: "get-started.html",
            repo_root: Path::new("."),
            meta: &meta,
            known_site_targets: &known,
            broken_links: std::cell::RefCell::new(Vec::new()),
        };
        assert_eq!(resolver.resolve_href("HTTPS://example.com/a"), "HTTPS://example.com/a");
        assert_eq!(resolver.resolve_href("MAILTO:user@example.com"), "MAILTO:user@example.com");
        assert_eq!(resolver.resolve_image("HTTP://example.com/a.png"), "HTTP://example.com/a.png");
        assert!(resolver.broken_links.borrow().is_empty());

        let _ = resolver.resolve_href("../../../../../../../../README.md");
        assert!(resolver.broken_links.borrow().iter().any(|e| e.contains("escapes repository root")));
    }

    #[test]
    fn aud_12_44_az_index_total_order_deduplicates_non_adjacent_duplicates() {
        let entries = vec![
            AzEntry { term: String::from("Alpha"), category: String::from("Cat"), rel_href: String::from("a.html") },
            AzEntry { term: String::from("alpha"), category: String::from("Cat"), rel_href: String::from("b.html") },
            AzEntry { term: String::from("Alpha"), category: String::from("Cat"), rel_href: String::from("a.html") },
        ];
        let page = build_az_index_page(&entries);
        let PageSource::PreRendered { body_html, .. } = page.source else {
            panic!("expected PreRendered");
        };
        assert!(body_html.contains("<strong>2</strong> documented entries"), "{body_html}");
        assert_eq!(body_html.matches("href=\"a.html\"").count(), 1);
        let PageSource::PreRendered { toc, .. } = build_az_index_page(&entries).source else {
            panic!("expected PreRendered");
        };
        assert!(toc.is_empty(), "A-Z index must suppress duplicate vertical TOC");
    }

    #[test]
    fn r12_14_and_r12_13_pluralization_nav_groups_and_direct_single_page_links() {
        assert_eq!(super::format_page_count(1), "1 page");
        assert_eq!(super::format_page_count(2), "2 pages");

        for sec in super::BETA_SECTIONS {
            assert!(!sec.nav_group.label().is_empty());
            assert!(!sec.nav_group.nav_target().is_empty());
        }
    }
}
