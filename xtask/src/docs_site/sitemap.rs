//! Beta site map assembly, Markdown page ingestion, link rewriting, A–Z index, and fallback bridges.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use super::markdown::{LinkResolver, TocHeading, escape_html as eh, github_slug, render_markdown};
use super::registries::{AzEntry, ExpectedCoverage, RegistryBundle, RegistryPageSpec, all_analysis_kinds_meta, generate_registry_pages};
use super::template::{NavLink, PageSpec, SidebarChild, SidebarGroup, SidebarSection, SiteMeta, rel_href, render_page};
use crate::scan_util::{is_real_dir, is_regular_file};

const GITHUB_REPO_BASE: &str = "https://github.com/HauserGroup/curvebook";

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

pub struct GeneratedSite {
    pub html_pages: BTreeMap<String, String>,
    pub expected_coverage: ExpectedCoverage,
    pub meta: SiteMeta,
}

#[derive(Clone, Copy)]
struct SectionDef {
    id: &'static str,
    title: &'static str,
    nav_group: NavGroup,
    summary: &'static str,
}

impl SectionDef {
    fn index_rel_path(self) -> String {
        format!("section-{}.html", self.id)
    }
}

const fn sec(id: &'static str, title: &'static str, nav_group: NavGroup, summary: &'static str) -> SectionDef {
    SectionDef { id, title, nav_group, summary }
}
const GS: NavGroup = NavGroup::GettingStarted;
const RF: NavGroup = NavGroup::Reference;
const AU: NavGroup = NavGroup::Automation;
const SP: NavGroup = NavGroup::Support;

const BETA_SECTIONS: &[SectionDef] = &[
    sec(
        "get-started",
        "Getting started",
        GS,
        "Installation, checksum/attestation verification, font configuration, desktop onboarding, and MCP client setup.",
    ),
    sec(
        "concepts",
        "Concepts",
        RF,
        "Projects, five sheet families, reactive recalculation pipeline, non-destructive exclusions, checkpoints, and undo.",
    ),
    sec(
        "tables",
        "Data tables & formulas",
        RF,
        "Table archetypes, subcolumn and replicate modes, Calculated Variables formulas, and linked data transforms.",
    ),
    sec(
        "analyses",
        "Analyses",
        RF,
        "Statistical tests, assumptions, diagnostics, edge-case behavior, peer-reviewed citations, and oracle verification.",
    ),
    sec(
        "fitting",
        "Curve fitting",
        RF,
        "Built-in equation library, nonlinear solvers, weighting schemes, robust/ROUT fitting, bands, and custom DSL/ODE models.",
    ),
    sec(
        "graphs",
        "Graphs & Page Layouts",
        RF,
        "Plot families, scales, frames, palettes, multi-panel Page Layouts, and publication vector/raster rendering.",
    ),
    sec(
        "qol",
        "Calculators & quality-of-life tools",
        RF,
        "Quick statistics, scientific calculators, command palette, cloning pipelines, and quality-of-life capabilities.",
    ),
    sec(
        "files",
        "Files, import & export",
        RF,
        "Archive/directory/template formats, interchange importers, export formats, autosave recovery, and JSON Schemas.",
    ),
    sec(
        "settings",
        "Settings & languages",
        RF,
        "Preference schema keys, shortcut customization, supported interface locales, and locale formatting.",
    ),
    sec("glossary", "Glossary", RF, "Defined terminology across statistical and software workflows plus the complete A–Z index of symbols."),
    sec(
        "automation",
        "Automation",
        AU,
        "MCP server, curvebook-cli subcommands, loopback --control protocol, curvebook.batch.v1 manifests, and command catalogs.",
    ),
    sec(
        "troubleshooting",
        "Troubleshooting & known issues",
        SP,
        "Diagnosing font, MCP, archive recovery, and solver diagnostics alongside documented release limits.",
    ),
    sec(
        "privacy-security",
        "Privacy, licences & security",
        SP,
        "Offline/zero-telemetry operation, loopback socket policy, dual licensing, clean-room policy, and security disclosures.",
    ),
    sec(
        "contribute",
        "Contribute",
        SP,
        "Architecture, clean-room rules, CI stages, extending analyses/commands, localization, and releasing snapshots.",
    ),
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

pub(crate) struct GuideDescriptor {
    pub(crate) rel_path: &'static str,
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
    pub(crate) section_id: &'static str,
    pub(crate) repo_path: &'static str,
}

const fn guide(
    rel_path: &'static str,
    title: &'static str,
    description: &'static str,
    section_id: &'static str,
    repo_path: &'static str,
) -> GuideDescriptor {
    GuideDescriptor { rel_path, title, description, section_id, repo_path }
}

pub(crate) const MANUAL_GUIDES: &[GuideDescriptor] = &[
    guide(
        "get-started.html",
        "Install CurveBook",
        "System requirements, installing CurveBook, verifying checksums/attestations, and font configuration.",
        "get-started",
        "docs/reference/get-started.md",
    ),
    guide(
        "desktop-gui.html",
        "Desktop GUI Overview",
        "Overview of desktop welcome workflow, sheets, analyses, graphs, saving, and exporting.",
        "get-started",
        "docs/reference/desktop-gui.md",
    ),
    guide(
        "mcp-for-beginners.html",
        "MCP Setup for Beginners",
        "Configure an MCP client to connect to CurveBook and verify the connection.",
        "get-started",
        "docs/mcp-for-beginners.md",
    ),
    guide(
        "concepts.html",
        "Concepts & Reactive Pipeline",
        "Project structure, five sheet types, reactive recalculation pipeline, exclusions, checkpoints, and undo.",
        "concepts",
        "docs/reference/concepts.md",
    ),
    guide(
        "tables.html",
        "Data Tables, Formulas & Transforms",
        "Table archetypes, replicate subcolumn modes, Calculated Variables formula language, and data transforms.",
        "tables",
        "docs/reference/tables.md",
    ),
    guide(
        "analyses.html",
        "Statistical Analyses Reference",
        "Complete interface-neutral reference for every CurveBook statistical analysis kind, options, diagnostics, and citations.",
        "analyses",
        "docs/reference/analyses.md",
    ),
    guide(
        "fitting.html",
        "Curve Fitting Reference",
        "Nonlinear regression methods, equation library, weighting, robust/ROUT fitting, bands, custom DSL, and ODE models.",
        "fitting",
        "docs/reference/fitting.md",
    ),
    guide(
        "graphs.html",
        "Graphs & Page Layouts",
        "Graph types, compatible table archetypes, axis scales, error bars, colour palettes, and Page Layout rendering.",
        "graphs",
        "docs/reference/graphs.md",
    ),
    guide(
        "qol.html",
        "Calculators & Quality-of-Life Tools",
        "Scientific calculators, quick statistics, and workspace quality-of-life capabilities.",
        "qol",
        "docs/reference/qol.md",
    ),
    guide(
        "files.html",
        "Files, Import & Export",
        "CurveBook archives, unpacked .curvebook.d directories, templates, autosave recovery, and interchange formats.",
        "files",
        "docs/reference/files.md",
    ),
    guide(
        "automation/mcp.html",
        "MCP Server Reference",
        "CurveBook Model Context Protocol server transports, core tools, toolsets, resources, and prompts.",
        "automation",
        "docs/reference/automation/mcp.md",
    ),
    guide(
        "automation/cli.html",
        "CLI Reference (curvebook-cli)",
        "Headless runner subcommands, schema-typed direct command flags, exit codes, and PDF/format flags.",
        "automation",
        "docs/reference/automation/cli.md",
    ),
    guide(
        "automation/control-and-batch.html",
        "Control Protocol, Batch Format & Errors",
        "Loopback --control JSON-lines protocol, curvebook.batch.v1 manifests, run scripts, and engine error codes.",
        "automation",
        "docs/reference/automation/control-and-batch.md",
    ),
    guide(
        "settings.html",
        "Settings & Languages",
        "Preferences JSON Schema keys, keyboard shortcut customization, interface locales, and locale formatting.",
        "settings",
        "docs/reference/settings.md",
    ),
    guide(
        "privacy-security.html",
        "Privacy, Licences & Security",
        "Zero-telemetry local execution, loopback control rules, dual licensing, and vulnerability reporting.",
        "privacy-security",
        "docs/reference/privacy-security.md",
    ),
    guide(
        "troubleshooting.html",
        "Troubleshooting & Known Issues",
        "Common install, font discovery, MCP transport, archive recovery, and solver diagnostics.",
        "troubleshooting",
        "docs/reference/troubleshooting.md",
    ),
    guide(
        "glossary.html",
        "Glossary",
        "Definitions of core CurveBook scientific, data-model, and automation terms.",
        "glossary",
        "docs/reference/glossary.md",
    ),
];

const FALLBACK_REGS: &[(&str, &str, &str)] = &[
    ("get-started.html", "Format & Package Reference", "registry/formats.html"),
    ("desktop-gui.html", "UI Command Catalog", "registry/commands.html#ui-commands"),
    ("concepts.html", "Table & Analysis Registry", "registry/tables.html"),
    ("tables.html", "Table Archetypes & Subcolumn Registry", "registry/tables.html#table-archetypes"),
    ("fitting.html", "Parametric Equations & Fitting Registry", "registry/equations.html"),
    ("graphs.html", "Graphs, Scales & Palettes Registry", "registry/graphs.html"),
    ("qol.html", "UI & Engine Commands", "registry/commands.html"),
    ("files.html", "Formats & Schemas Registry", "registry/formats.html"),
    ("settings.html", "Preferences Schema & Locales Registry", "registry/formats.html#schema-preferences"),
    ("troubleshooting.html", "Engine Error Codes", "registry/tables.html#error-codes"),
];

enum PageSource {
    Markdown { repo_path: &'static str },
    PreRendered { h1: String, toc: Vec<TocHeading>, body_html: String },
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
    schemes.iter().any(|s| raw.get(..s.len()).is_some_and(|p| p.eq_ignore_ascii_case(s)))
}

impl LinkResolver for RepoLinkResolver<'_> {
    fn resolve_href(&self, raw_href: &str) -> String {
        if raw_href.starts_with('#') || starts_with_scheme_ci(raw_href, &["http://", "https://", "mailto:"]) {
            return raw_href.to_owned();
        }
        let (target_part, frag_part) = raw_href.split_once('#').map_or((raw_href, None), |(t, f)| (t, Some(f)));
        let frag_s = frag_part.map(|f| format!("#{f}")).unwrap_or_default();
        if target_part.is_empty() {
            return frag_s;
        }
        let clean = target_part.trim_end_matches('/');
        if clean.ends_with(".html") && (clean.starts_with("registry/") || clean.starts_with("../registry/")) {
            return format!("{}{frag_s}", rel_href(self.current_site_rel, clean.trim_start_matches("../")));
        }
        let source_dir = Path::new(self.source_repo_rel).parent().unwrap_or_else(|| Path::new(""));
        let Some(normalized) = normalize_repo_rel(source_dir, target_part) else {
            self.broken_links.borrow_mut().push(format!("{}: relative link `{raw_href}` escapes repository root", self.source_repo_rel));
            return String::from("#");
        };
        if let Some(mapped) = self.known_site_targets.get(normalized.as_str()) {
            let frag = frag_part
                .map(|f| format!("#{f}"))
                .or_else(|| {
                    normalized.strip_prefix("docs/schemas/").and_then(|s| s.strip_suffix(".schema.json")).map(|stem| format!("#schema-{stem}"))
                })
                .unwrap_or_default();
            return format!("{}{frag}", rel_href(self.current_site_rel, mapped));
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
            return format!("{base}{frag_s}");
        }
        if let Some(img) = normalized.strip_prefix("docs/images/") {
            return format!("{}images/{img}", super::template::asset_prefix(self.current_site_rel, self.meta.is_release_snapshot));
        }
        let trimmed = normalized.trim_end_matches('/');
        let target_path = self.repo_root.join(trimmed);
        if trimmed.is_empty() || trimmed.ends_with(".html") || !target_path.exists() {
            self.broken_links
                .borrow_mut()
                .push(format!("{}: broken relative link `{raw_href}` (resolved repo path `{trimmed}` does not exist)", self.source_repo_rel));
        }
        let kind = if target_part.ends_with('/') || is_real_dir(&target_path) { "tree" } else { "blob" };
        format!("{GITHUB_REPO_BASE}/{kind}/main/{trimmed}{frag_s}")
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
        if let Some(img) = normalized.strip_prefix("docs/images/") {
            if !is_regular_file(&self.repo_root.join("docs/images").join(img)) {
                self.broken_links
                    .borrow_mut()
                    .push(format!("{}: missing image asset `{raw_src}` (resolved repo path `{normalized}` does not exist)", self.source_repo_rel));
            }
            return format!("{assets}images/{img}");
        }
        if let Some(fname) = Path::new(&normalized).file_name().and_then(|n| n.to_str())
            && is_regular_file(&self.repo_root.join("docs/images").join(fname))
        {
            return format!("{assets}images/{fname}");
        }
        self.broken_links
            .borrow_mut()
            .push(format!("{}: missing image asset `{raw_src}` (resolved repo path `{normalized}` does not exist)", self.source_repo_rel));
        format!("{assets}images/{normalized}")
    }
}

fn normalize_repo_rel(base_dir: &Path, raw: &str) -> Option<String> {
    let mut parts: Vec<String> = base_dir
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str().filter(|t| !t.is_empty()).map(str::to_owned),
            _ => None,
        })
        .collect();
    let clean = raw.replace('\\', "/");
    let stripped = clean.trim_start_matches("./");
    let target = stripped.strip_prefix('/').inspect(|_| parts.clear()).unwrap_or(stripped);
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other.to_owned()),
        }
    }
    Some(parts.join("/"))
}

pub fn assemble_site(root: &Path, meta: SiteMeta) -> Result<GeneratedSite, String> {
    let defined_anchors: BTreeSet<String> = std::fs::read_to_string(root.join("docs/reference/analyses.md"))
        .ok()
        .filter(|_| is_regular_file(&root.join("docs/reference/analyses.md")))
        .map(|s| render_markdown(&s, &super::markdown::IdentityResolver).headings.into_iter().map(|h| h.id).collect())
        .unwrap_or_default();
    let registries: RegistryBundle = generate_registry_pages(root, &defined_anchors)?;
    let mut known: BTreeMap<&'static str, &'static str> = MANUAL_GUIDES.iter().map(|g| (g.repo_path, g.rel_path)).collect();
    known.extend([
        ("docs/mcp.md", "automation/mcp.html"),
        ("docs/control-protocol.md", "automation/control-and-batch.html"),
        ("docs/commands.md", "registry/commands.html"),
        ("docs/schemas/preferences.schema.json", "registry/formats.html"),
        ("docs/schemas/batch-v1.schema.json", "registry/formats.html"),
        ("docs/schemas/document.schema.json", "registry/formats.html"),
        ("docs/schemas/archive-manifest.schema.json", "registry/formats.html"),
        ("docs/schemas/journal-record.schema.json", "registry/formats.html"),
    ]);
    let mut az_entries = registries.az_entries;
    az_entries.extend(collect_glossary_az_entries(root));
    let mut by_section: BTreeMap<&'static str, Vec<PageItem>> = BTreeMap::new();
    for p in ordered_content_pages(registries.pages) {
        if let Some(sec_id) = p.section_id {
            by_section.entry(sec_id).or_default().push(p);
        }
    }
    let (sidebar_groups, az_sorted) = (build_sidebar_groups(&by_section), sort_and_dedup_az(&az_entries));
    let mut ordered = vec![
        build_grouped_hub(
            "index.html",
            "CurveBook Reference Manual",
            "Section map and typed reference manual for CurveBook statistical analyses, curve fitting, graphics, MCP, and CLI automation.",
            None,
            "Complete static reference manual for CurveBook scientific tables, statistical methods, nonlinear regression, publication graphics, MCP/CLI automation, and contributor guides on GitHub.",
            &[
                ("group-start-here", "Start Here", &["get-started", "concepts"]),
                ("group-scientific-reference", "Scientific & Graphing Reference", &["tables", "analyses", "fitting", "graphs", "qol"]),
                ("group-files-automation-settings", "Files, Automation & Settings", &["files", "automation", "settings"]),
                ("group-support-contribute", "Support & Contribute", &["privacy-security", "troubleshooting", "glossary", "contribute"]),
            ],
            &by_section,
            None,
        ),
        build_grouped_hub(
            "reference.html",
            "Reference",
            "Grouped reference directory for CurveBook concepts, data tables, statistical analyses, curve fitting, graphs, calculators, files, settings, and glossary.",
            Some(RF),
            "Scientific and workspace reference across core concepts, data tables, statistical analyses, curve fitting, graphs, calculators, files, settings, and terminology.",
            &[
                ("group-core-concepts-data", "Core concepts & data", &["concepts", "tables"]),
                ("group-stats-fitting", "Statistics & curve fitting", &["analyses", "fitting"]),
                ("group-graphs-files-settings", "Graphs, files & settings", &["graphs", "qol", "files", "settings"]),
                ("group-terminology", "Terminology", &["glossary"]),
            ],
            &by_section,
            Some(("group-terminology", az_sorted.len())),
        ),
        build_support_page(&by_section),
    ];
    for &s in BETA_SECTIONS {
        let children = by_section.remove(s.id).unwrap_or_default();
        ordered.push(build_section_index_page(s, &children));
        ordered.extend(children);
        if s.id == "glossary" {
            ordered.push(build_az_index_page(&az_sorted));
        }
    }
    let pager_links: Vec<NavLink> =
        ordered.iter().filter(|p| !p.is_hub).map(|p| NavLink { label: p.title.clone(), rel_path: p.rel_path.clone() }).collect();
    let pager_pos: BTreeMap<&str, usize> = pager_links.iter().enumerate().map(|(i, l)| (l.rel_path.as_str(), i)).collect();
    let mut html_pages = BTreeMap::new();
    for item in &ordered {
        let sec = item.section_id.and_then(section_by_id);
        let sec_idx = sec.map(SectionDef::index_rel_path);
        let pos = (!item.is_hub).then(|| pager_pos.get(item.rel_path.as_str()).copied()).flatten();
        let (title, h1, toc, body_html, body_has_h1) = match &item.source {
            PageSource::PreRendered { h1, toc, body_html } => (item.title.clone(), h1.clone(), toc.clone(), body_html.clone(), false),
            PageSource::Markdown { repo_path } => {
                let full = root.join(repo_path);
                if std::fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(format!("{repo_path}: Markdown source must not be a symlink"));
                }
                let rendered = if is_regular_file(&full) {
                    let md_src = std::fs::read_to_string(&full).map_err(|e| format!("read {repo_path}: {e}"))?;
                    let resolver = RepoLinkResolver {
                        source_repo_rel: repo_path,
                        current_site_rel: &item.rel_path,
                        repo_root: root,
                        meta: &meta,
                        known_site_targets: &known,
                        broken_links: std::cell::RefCell::new(Vec::new()),
                    };
                    let out = render_markdown(&md_src, &resolver);
                    if let Some(first) = resolver.broken_links.into_inner().into_iter().next() {
                        return Err(first);
                    }
                    out
                } else {
                    render_fallback_page(&item.rel_path)?
                };
                let has_h1 = rendered.title.is_some();
                let t = rendered.title.unwrap_or_else(|| item.title.clone());
                (t.clone(), t, rendered.headings, rendered.html, has_h1)
            }
        };
        let html = render_page(&PageSpec {
            title: &title,
            description: &item.description,
            rel_path: &item.rel_path,
            section_title: sec.map_or("Manual", |s| s.title),
            section_rel_path: sec_idx.as_deref(),
            nav_group: item.nav_group.or_else(|| sec.map(|s| s.nav_group)),
            active_section_id: item.section_id,
            sidebar_groups: &sidebar_groups,
            h1: &h1,
            toc_headings: &toc,
            body_html: &body_html,
            body_has_h1,
            prev: pos.and_then(|i| i.checked_sub(1)).and_then(|i| pager_links.get(i)),
            next: pos.and_then(|i| pager_links.get(i + 1)),
            meta: &meta,
        });
        html_pages.insert(item.rel_path.clone(), html);
    }
    Ok(GeneratedSite { html_pages, expected_coverage: registries.expected, meta })
}

fn section_by_id(id: &str) -> Option<SectionDef> {
    BETA_SECTIONS.iter().copied().find(|s| s.id == id)
}

fn section_target_href(sec: SectionDef, by_sec: &BTreeMap<&'static str, Vec<PageItem>>) -> String {
    match by_sec.get(sec.id).map(Vec::as_slice) {
        Some([one]) => one.rel_path.clone(),
        _ => sec.index_rel_path(),
    }
}

fn build_sidebar_groups(by_sec: &BTreeMap<&'static str, Vec<PageItem>>) -> Vec<SidebarGroup> {
    NavGroup::ALL
        .into_iter()
        .map(|group| SidebarGroup {
            group,
            sections: BETA_SECTIONS
                .iter()
                .copied()
                .filter(|s| s.nav_group == group)
                .map(|s| {
                    let items = by_sec.get(s.id).map(Vec::as_slice).unwrap_or(&[]);
                    let children = if items.len() > 1 {
                        items.iter().map(|p| SidebarChild { title: p.title.clone(), rel_path: p.rel_path.clone() }).collect()
                    } else {
                        Vec::new()
                    };
                    SidebarSection { id: s.id, title: s.title.to_owned(), href: section_target_href(s, by_sec), children }
                })
                .collect(),
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
            source: PageSource::Markdown { repo_path: g.repo_path },
        })
        .collect();
    for (reg_path, sec_id) in [
        ("registry/tables.html", "tables"),
        ("registry/equations.html", "fitting"),
        ("registry/graphs.html", "graphs"),
        ("registry/formats.html", "files"),
        ("registry/commands.html", "automation"),
        ("registry/mcp-tools.html", "automation"),
        ("registry/cli.html", "automation"),
    ] {
        if let Some(r) = reg_map.remove(reg_path) {
            items.push(PageItem {
                rel_path: r.rel_path,
                title: r.title,
                description: r.description,
                section_id: Some(sec_id),
                nav_group: section_by_id(sec_id).map(|s| s.nav_group),
                is_hub: false,
                source: PageSource::PreRendered { h1: r.h1, toc: r.toc_headings, body_html: r.body_html },
            });
        }
    }
    items
}

fn render_fallback_page(current_rel: &str) -> Result<super::markdown::RenderedMarkdown, String> {
    let (mut html, mut headings) = (String::new(), Vec::new());
    match current_rel {
        "analyses.html" => {
            let _ = writeln!(
                html,
                "<p class=\"lead\">Complete statistical analysis catalog covering every CurveBook <code>AnalysisKind</code>.</p>\n<p>See the generated <a href=\"{}\">Tables, Analyses &amp; Error Codes Registry</a> for tabular summaries.</p>",
                eh(&rel_href(current_rel, "registry/tables.html#analysis-kinds"))
            );
            for (slug, name, desc) in all_analysis_kinds_meta() {
                let id = format!("analysis-{slug}");
                headings.push(TocHeading { level: 2, id: id.clone(), text: name.to_owned() });
                let _ = writeln!(html, "<h2 id=\"{id}\" tabindex=\"-1\">{}</h2>\n<p>{}</p>", eh(name), eh(desc));
            }
        }
        "glossary.html" => {
            html.push_str("<p class=\"lead\">Core CurveBook scientific, workspace, and automation terminology.</p>\n<dl>\n");
            for &(term, def) in GLOSSARY_SEED_TERMS {
                let id = format!("term-{}", github_slug(term));
                headings.push(TocHeading { level: 2, id: id.clone(), text: term.to_owned() });
                let _ = writeln!(html, "  <dt id=\"{id}\" tabindex=\"-1\">{}</dt>\n  <dd>{}</dd>", eh(term), eh(def));
            }
            html.push_str("</dl>\n");
        }
        "mcp-for-beginners.html" | "automation/mcp.html" | "automation/cli.html" | "automation/control-and-batch.html" => {
            return Err(format!("required Markdown source missing for `{current_rel}`"));
        }
        _ => {
            let lead = MANUAL_GUIDES.iter().find(|g| g.rel_path == current_rel).map_or(
                "CurveBook runs strictly locally with zero outbound telemetry, loopback-only control sockets, and dual MIT OR Apache-2.0 licensing.",
                |g| g.description,
            );
            let _ = writeln!(
                html,
                "<p class=\"lead\">{}</p>\n<div class=\"callout\"><p>This reference topic mirrors typed metadata from the CurveBook registry generators below.</p></div>",
                eh(lead)
            );
            if let Some(&(_, label, target)) = FALLBACK_REGS.iter().find(|&&(k, _, _)| k == current_rel) {
                let _ =
                    writeln!(html, "<p>Open the generated registry page: <a href=\"{}\">{}</a>.</p>", eh(&rel_href(current_rel, target)), eh(label));
            }
        }
    }
    Ok(super::markdown::RenderedMarkdown { html, headings, title: None })
}

const GLOSSARY_SEED_TERMS: &[(&str, &str)] = &[
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
];

fn collect_glossary_az_entries(root: &Path) -> Vec<AzEntry> {
    let md = root.join("docs/reference/glossary.md");
    if is_regular_file(&md)
        && let Ok(text) = std::fs::read_to_string(&md)
    {
        let mut seen = BTreeSet::new();
        let out: Vec<_> = render_markdown(&text, &super::markdown::IdentityResolver)
            .headings
            .into_iter()
            .filter(|h| h.level >= 2 && seen.insert(h.id.clone()))
            .map(|h| AzEntry { term: h.text, category: String::from("Glossary"), rel_href: format!("glossary.html#{}", h.id) })
            .collect();
        if !out.is_empty() {
            return out;
        }
    }
    GLOSSARY_SEED_TERMS
        .iter()
        .map(|&(t, _)| AzEntry { term: t.to_owned(), category: String::from("Glossary"), rel_href: format!("glossary.html#term-{}", github_slug(t)) })
        .collect()
}

fn format_page_count(n: usize) -> String {
    if n == 1 { String::from("1 page") } else { format!("{n} pages") }
}

fn section_badge(sec: SectionDef, by_sec: &BTreeMap<&'static str, Vec<PageItem>>) -> String {
    if sec.id == "contribute" {
        format!("{} GitHub guides", CONTRIBUTOR_GITHUB_GUIDES.len())
    } else {
        format_page_count(by_sec.get(sec.id).map_or(0, Vec::len))
    }
}

fn push_section_card(body: &mut String, indent: &str, htag: &str, href: &str, title_html: &str, desc: &str, badge: Option<&str>) {
    let _ = writeln!(
        body,
        "{indent}<section class=\"section-card\">\n{indent}  <{htag}><a href=\"{href}\">{title_html}</a></{htag}>\n{indent}  <p>{}</p>",
        eh(desc)
    );
    if let Some(b) = badge {
        let _ = writeln!(body, "{indent}  <p><span class=\"badge\">{b}</span></p>");
    }
    let _ = writeln!(body, "{indent}</section>");
}

fn hub_page(rel: &str, title: &str, desc: &str, nav_group: Option<NavGroup>, body_html: String) -> PageItem {
    PageItem {
        rel_path: rel.to_owned(),
        title: title.to_owned(),
        description: desc.to_owned(),
        section_id: None,
        nav_group,
        is_hub: true,
        source: PageSource::PreRendered { h1: title.to_owned(), toc: Vec::new(), body_html },
    }
}

#[allow(clippy::too_many_arguments)]
fn build_grouped_hub(
    rel: &str,
    title: &str,
    desc: &str,
    nav_group: Option<NavGroup>,
    lead: &str,
    groups: &[(&str, &str, &[&str])],
    by_sec: &BTreeMap<&'static str, Vec<PageItem>>,
    az_group: Option<(&str, usize)>,
) -> PageItem {
    let mut body = format!("<p class=\"lead\">{lead}</p>\n");
    for &(gid, gtitle, ids) in groups {
        let _ = writeln!(
            body,
            "<section class=\"section-group\" aria-labelledby=\"{gid}\">\n  <h2 id=\"{gid}\" tabindex=\"-1\">{}</h2>\n  <div class=\"section-grid\">",
            eh(gtitle)
        );
        for s in ids.iter().filter_map(|&id| section_by_id(id)) {
            push_section_card(
                &mut body,
                "    ",
                "h3",
                &eh(&section_target_href(s, by_sec)),
                &eh(s.title),
                s.summary,
                Some(&section_badge(s, by_sec)),
            );
        }
        if let Some((ag, n)) = az_group
            && ag == gid
        {
            push_section_card(
                &mut body,
                "    ",
                "h3",
                "az-index.html",
                "A–Z Reference Index",
                "Alphabetical symbol directory covering every analysis kind, fit equation, graph type, table type, command, MCP tool, CLI flag, format, and setting key.",
                Some(&format!("{n} entries")),
            );
        }
        body.push_str("  </div>\n</section>\n");
    }
    hub_page(rel, title, desc, nav_group, body)
}

fn build_support_page(by_sec: &BTreeMap<&'static str, Vec<PageItem>>) -> PageItem {
    let mut body = String::from(
        "<p class=\"lead\">Diagnostic guides, offline privacy and licensing commitments, security reporting, and contributor resources on GitHub.</p>\n<div class=\"section-grid\">\n",
    );
    for s in ["troubleshooting", "privacy-security", "contribute"].into_iter().filter_map(section_by_id) {
        push_section_card(&mut body, "  ", "h2", &eh(&section_target_href(s, by_sec)), &eh(s.title), s.summary, Some(&section_badge(s, by_sec)));
    }
    body.push_str("</div>\n");
    hub_page(
        "support.html",
        "Support",
        "Troubleshooting, known issues, offline privacy guarantees, dual licensing, security reporting, and contributor guides.",
        Some(SP),
        body,
    )
}

fn build_section_index_page(sec: SectionDef, children: &[PageItem]) -> PageItem {
    let idx_rel = sec.index_rel_path();
    let mut body = format!("<p class=\"lead\">{}</p>\n", eh(sec.summary));
    if sec.id == "contribute" {
        body.push_str("<div class=\"callout\"><p>Contributor specifications are maintained continuously against the live <code>main</code> branch on GitHub.</p></div>\n");
    }
    body.push_str("<div class=\"section-grid\">\n");
    if sec.id == "contribute" {
        for &(title, repo_path, summary) in CONTRIBUTOR_GITHUB_GUIDES {
            push_section_card(
                &mut body,
                "  ",
                "h2",
                &format!("{GITHUB_REPO_BASE}/blob/main/{repo_path}"),
                &format!("{} <span aria-hidden=\"true\">↗</span>", eh(title)),
                summary,
                None,
            );
        }
    }
    for c in children {
        push_section_card(&mut body, "  ", "h2", &eh(&rel_href(&idx_rel, &c.rel_path)), &eh(&c.title), &c.description, None);
    }
    if sec.id == "glossary" {
        push_section_card(
            &mut body,
            "  ",
            "h2",
            "az-index.html",
            "Complete A–Z Symbol Index",
            "Alphabetical index across every analysis kind, fit equation, graph type, table type, engine/UI command, MCP tool, CLI command, format, and setting key.",
            None,
        );
    }
    body.push_str("</div>\n");
    PageItem {
        rel_path: idx_rel,
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

fn build_az_index_page(sorted: &[AzEntry]) -> PageItem {
    let mut grouped: BTreeMap<char, Vec<&AzEntry>> = BTreeMap::new();
    for entry in sorted {
        let ch = entry.term.chars().find(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).unwrap_or('#');
        grouped.entry(if ch.is_ascii_alphabetic() { ch } else { '#' }).or_default().push(entry);
    }
    let mut body = format!(
        "<p class=\"lead\">Complete script-free alphabetical index covering <strong>{}</strong> documented entries across analyses, fit equations, graph types, table types, engine/UI commands, MCP tools, CLI commands, formats, settings, and glossary terms.</p>\n<ul class=\"az-letters\" aria-label=\"Alphabetical jump links\">\n",
        sorted.len()
    );
    for l in 'A'..='Z' {
        let _ = writeln!(
            body,
            "{}",
            if grouped.contains_key(&l) { format!("  <li><a href=\"#letter-{l}\">{l}</a></li>") } else { format!("  <li><span>{l}</span></li>") }
        );
    }
    if grouped.contains_key(&'#') {
        body.push_str("  <li><a href=\"#letter-other\">#</a></li>\n");
    }
    body.push_str("</ul>\n");
    for (bucket, items) in grouped {
        let (id, h) =
            if bucket == '#' { (String::from("letter-other"), String::from("#")) } else { (format!("letter-{bucket}"), bucket.to_string()) };
        let _ = writeln!(body, "<h2 id=\"{id}\" tabindex=\"-1\">{h}</h2>\n<ul>");
        for item in items {
            let _ = writeln!(
                body,
                "  <li><a href=\"{}\">{}</a> <span class=\"badge\">{}</span></li>",
                eh(&item.rel_href),
                eh(&item.term),
                eh(&item.category)
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
    use super::*;

    #[test]
    fn repo_links_az_dedup_and_section_metadata() {
        assert_eq!(
            (
                normalize_repo_rel(Path::new("docs/reference"), "../../README.md").as_deref(),
                normalize_repo_rel(Path::new("docs/reference"), "../../../../../../README.md")
            ),
            (Some("README.md"), None)
        );
        let (meta, known) = (
            SiteMeta { version_label: "v0.1.0".into(), commit_sha: None, release_date: None, is_release_snapshot: false, is_tagged_release: false },
            BTreeMap::new(),
        );
        let res = RepoLinkResolver {
            source_repo_rel: "docs/reference/get-started.md",
            current_site_rel: "get-started.html",
            repo_root: Path::new("."),
            meta: &meta,
            known_site_targets: &known,
            broken_links: std::cell::RefCell::new(Vec::new()),
        };
        assert_eq!(
            (res.resolve_href("HTTPS://example.com/a"), res.resolve_image("HTTP://example.com/a.png")),
            ("HTTPS://example.com/a".into(), "HTTP://example.com/a.png".into())
        );
        let _ = res.resolve_href("../../../../../../../../README.md");
        assert!(!res.broken_links.borrow().is_empty());
        let sorted = sort_and_dedup_az(&[
            AzEntry { term: "Alpha".into(), category: "Cat".into(), rel_href: "a.html".into() },
            AzEntry { term: "alpha".into(), category: "Cat".into(), rel_href: "b.html".into() },
            AzEntry { term: "Alpha".into(), category: "Cat".into(), rel_href: "a.html".into() },
        ]);
        let PageSource::PreRendered { body_html, toc, .. } = build_az_index_page(&sorted).source else { panic!("expected PreRendered") };
        assert!(body_html.contains("<strong>2</strong> documented entries") && body_html.matches("href=\"a.html\"").count() == 1 && toc.is_empty());
        assert_eq!((format_page_count(1), format_page_count(2)), (String::from("1 page"), String::from("2 pages")));
    }
}
