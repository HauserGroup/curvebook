//! Single source of truth for CurveBook software citation metadata (`CITATION.cff`, generated
//! methods reports, desktop Cite dialog, and documentation drift verification).
//!
//! Pure data and deterministic string formatters only: zero OS dependencies, zero network access,
//! and fully compatible with `wasm32-unknown-unknown`.

use std::fmt::Write as _;

/// Collective or individual author entry recorded in `CITATION.cff` and software references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoftwareAuthor {
    /// Collective entity or display name.
    pub name: &'static str,
    /// Optional public website URL associated with the entity or author.
    pub website: Option<&'static str>,
}

/// Canonical software citation metadata shared across `CITATION.cff`, generated methods reports,
/// the desktop Cite dialog, and documentation drift checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoftwareCitation {
    /// Software title (`"CurveBook"`).
    pub title: &'static str,
    /// Prompt message recorded in `CITATION.cff`.
    pub cff_message: &'static str,
    /// Ordered list of authors / collective entities credited in software citations.
    pub authors: &'static [SoftwareAuthor],
    /// Canonical public source-repository URL.
    pub repository_url: &'static str,
    /// Canonical public website URL.
    pub website_url: &'static str,
    /// Individual SPDX licence identifiers listed in CFF 1.2.0 `license`.
    pub spdx_licenses: &'static [&'static str],
    /// Human-readable SPDX licence expression for BibTeX and documentation.
    pub license_expression: &'static str,
    /// Zenodo concept DOI (`10.5281/zenodo.<id>`) once minted; `None` until the first Zenodo
    /// archive exists.
    pub concept_doi: Option<&'static str>,
    /// ISO-8601 `YYYY-MM-DD` release date mirrored into `CITATION.cff`.
    pub release_date: &'static str,
    /// Publication year emitted in plain-text and BibTeX references.
    pub release_year: u16,
}

/// Canonical CurveBook citation record shared across the workspace.
pub const CURVEBOOK_CITATION: SoftwareCitation = SoftwareCitation {
    title: "CurveBook",
    cff_message: "If you use CurveBook in your research, please cite it using the metadata from this file.",
    authors: &[SoftwareAuthor { name: "CurveBook Contributors", website: Some("https://hausergroup.org") }],
    repository_url: "https://github.com/HauserGroup/curvebook",
    website_url: "https://curvebook.org/",
    spdx_licenses: &["MIT", "Apache-2.0"],
    license_expression: "MIT OR Apache-2.0",
    concept_doi: None,
    release_date: "2026-11-27",
    release_year: 2026,
};

impl SoftwareCitation {
    /// Build a canonical HTTPS DOI resolver URL (`https://doi.org/<doi>`) for a non-empty DOI.
    #[must_use]
    pub fn doi_url_for(doi: &str) -> String {
        format!("https://doi.org/{}", doi.trim())
    }

    /// Return the resolved HTTPS concept DOI URL when [`Self::concept_doi`] is configured.
    #[must_use]
    pub fn doi_url(&self) -> Option<String> {
        self.concept_doi.map(str::trim).filter(|doi| !doi.is_empty()).map(Self::doi_url_for)
    }

    /// Return the primary citation URL: the concept DOI URL when minted, otherwise the repository URL.
    #[must_use]
    pub fn primary_citation_url(&self) -> String {
        self.primary_citation_url_with_doi(self.concept_doi)
    }

    /// Return the primary citation URL using an explicit optional concept DOI override.
    #[must_use]
    pub fn primary_citation_url_with_doi(&self, concept_doi: Option<&str>) -> String {
        concept_doi.map(str::trim).filter(|doi| !doi.is_empty()).map_or_else(|| self.repository_url.to_owned(), Self::doi_url_for)
    }

    /// Format the author list for plain-text references.
    #[must_use]
    pub fn author_list_text(&self) -> String {
        let mut out = String::new();
        for (idx, author) in self.authors.iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(author.name);
        }
        out
    }

    /// Format the inline software attribution used inside generated methods paragraphs, e.g.
    /// `"CurveBook 1.4.0"` or `"CurveBook 1.4.0 (doi:10.5281/zenodo.NNNN)"`.
    #[must_use]
    pub fn methods_software_tag(&self, version: &str) -> String {
        self.methods_software_tag_with_doi(version, self.concept_doi)
    }

    /// Format the inline methods-paragraph software attribution with an explicit optional DOI.
    #[must_use]
    pub fn methods_software_tag_with_doi(&self, version: &str, concept_doi: Option<&str>) -> String {
        let ver = version.trim();
        let doi = concept_doi.map(str::trim).filter(|d| !d.is_empty());
        match (ver.is_empty(), doi) {
            (false, Some(d)) => format!("{} {ver} (doi:{d})", self.title),
            (false, None) => format!("{} {ver}", self.title),
            (true, Some(d)) => format!("{} (doi:{d})", self.title),
            (true, None) => self.title.to_owned(),
        }
    }

    /// Format the ready-to-paste plain-text software reference for `version` (when supplied).
    #[must_use]
    pub fn plain_text_reference(&self, version: Option<&str>) -> String {
        self.plain_text_reference_with_doi(version, self.concept_doi)
    }

    /// Format the plain-text software reference with an explicit optional concept DOI.
    #[must_use]
    pub fn plain_text_reference_with_doi(&self, version: Option<&str>, concept_doi: Option<&str>) -> String {
        let authors = self.author_list_text();
        let url = self.primary_citation_url_with_doi(concept_doi);
        match version.map(str::trim).filter(|v| !v.is_empty()) {
            Some(ver) => format!("{authors} ({}). {} (Version {ver}) [Computer software]. {url}", self.release_year, self.title),
            None => format!("{authors} ({}). {} [Computer software]. {url}", self.release_year, self.title),
        }
    }

    /// Format a `@software` BibTeX entry for `version` (when supplied).
    #[must_use]
    pub fn bibtex(&self, version: Option<&str>) -> String {
        self.bibtex_with_doi(version, self.concept_doi)
    }

    /// Format a `@software` BibTeX entry with an explicit optional concept DOI.
    #[must_use]
    pub fn bibtex_with_doi(&self, version: Option<&str>, concept_doi: Option<&str>) -> String {
        let mut author_field = String::new();
        for (idx, author) in self.authors.iter().enumerate() {
            if idx > 0 {
                author_field.push_str(" and ");
            }
            let _ = write!(author_field, "{{{}}}", author.name);
        }

        let mut out = String::new();
        let _ = writeln!(out, "@software{{curvebook,");
        let _ = writeln!(out, "  author  = {{{author_field}}},");
        let _ = writeln!(out, "  title   = {{{{{}}}}},", self.title);
        if let Some(ver) = version.map(str::trim).filter(|v| !v.is_empty()) {
            let _ = writeln!(out, "  version = {{{ver}}},");
        }
        let _ = writeln!(out, "  year    = {{{}}},", self.release_year);
        let _ = writeln!(out, "  license = {{{}}},", self.license_expression);
        if let Some(doi) = concept_doi.map(str::trim).filter(|d| !d.is_empty()) {
            let _ = writeln!(out, "  doi     = {{{doi}}},");
        }
        let _ = writeln!(out, "  url     = {{{}}}", self.repository_url);
        out.push('}');
        out
    }

    /// Render the canonical root `CITATION.cff` document (CFF 1.2.0) for `version`.
    #[must_use]
    pub fn render_citation_cff(&self, version: &str) -> String {
        self.render_citation_cff_with_doi(version, self.concept_doi)
    }

    /// Render the canonical root `CITATION.cff` document (CFF 1.2.0) with an explicit optional DOI.
    #[must_use]
    pub fn render_citation_cff_with_doi(&self, version: &str, concept_doi: Option<&str>) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "cff-version: 1.2.0");
        let _ = writeln!(out, "title: {}", self.title);
        let _ = writeln!(out, "message: \"{}\"", self.cff_message);
        let _ = writeln!(out, "type: software");
        let _ = writeln!(out, "authors:");
        for author in self.authors {
            let _ = writeln!(out, "  - name: {}", author.name);
            if let Some(site) = author.website {
                let _ = writeln!(out, "    website: {site}");
            }
        }
        let _ = writeln!(out, "repository-code: {}", self.repository_url);
        let _ = writeln!(out, "url: {}", self.website_url);
        let _ = writeln!(out, "license:");
        for lic in self.spdx_licenses {
            let _ = writeln!(out, "  - {lic}");
        }
        let _ = writeln!(out, "version: {}", version.trim());
        let _ = writeln!(out, "date-released: {}", self.release_date);
        if let Some(doi) = concept_doi.map(str::trim).filter(|d| !d.is_empty()) {
            let _ = writeln!(out, "identifiers:");
            let _ = writeln!(out, "  - type: doi");
            let _ = writeln!(out, "    value: {doi}");
            let _ = writeln!(out, "    description: Concept DOI representing all versions of {} on Zenodo", self.title);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::CURVEBOOK_CITATION;

    #[test]
    fn methods_tag_plain_text_bibtex_and_cff_support_optional_doi() {
        assert_eq!(CURVEBOOK_CITATION.concept_doi, None);
        assert_eq!(CURVEBOOK_CITATION.doi_url(), None);
        assert_eq!(CURVEBOOK_CITATION.primary_citation_url(), "https://github.com/HauserGroup/curvebook");

        assert_eq!(CURVEBOOK_CITATION.methods_software_tag_with_doi("1.4.0", None), "CurveBook 1.4.0");
        assert_eq!(
            CURVEBOOK_CITATION.methods_software_tag_with_doi("1.4.0", Some("10.5281/zenodo.1234567")),
            "CurveBook 1.4.0 (doi:10.5281/zenodo.1234567)"
        );

        assert_eq!(
            CURVEBOOK_CITATION.plain_text_reference_with_doi(Some("0.1.0-alpha.2"), None),
            "CurveBook Contributors (2026). CurveBook (Version 0.1.0-alpha.2) [Computer software]. https://github.com/HauserGroup/curvebook"
        );
        assert_eq!(
            CURVEBOOK_CITATION.plain_text_reference_with_doi(None, None),
            "CurveBook Contributors (2026). CurveBook [Computer software]. https://github.com/HauserGroup/curvebook"
        );
        assert_eq!(
            CURVEBOOK_CITATION.plain_text_reference_with_doi(Some("1.4.0"), Some("10.5281/zenodo.1234567")),
            "CurveBook Contributors (2026). CurveBook (Version 1.4.0) [Computer software]. https://doi.org/10.5281/zenodo.1234567"
        );

        let bib_no_doi = CURVEBOOK_CITATION.bibtex_with_doi(Some("0.1.0-alpha.2"), None);
        assert!(bib_no_doi.contains("author  = {{CurveBook Contributors}},"));
        assert!(bib_no_doi.contains("version = {0.1.0-alpha.2},"));
        assert!(!bib_no_doi.contains("doi"));
        assert!(bib_no_doi.ends_with("  url     = {https://github.com/HauserGroup/curvebook}\n}"));

        let bib_with_doi = CURVEBOOK_CITATION.bibtex_with_doi(Some("1.4.0"), Some("10.5281/zenodo.1234567"));
        assert!(bib_with_doi.contains("doi     = {10.5281/zenodo.1234567},"));
        assert!(bib_with_doi.contains("url     = {https://github.com/HauserGroup/curvebook}"));

        let cff_no_doi = CURVEBOOK_CITATION.render_citation_cff_with_doi("0.1.0-alpha.2", None);
        assert!(cff_no_doi.starts_with("cff-version: 1.2.0\ntitle: CurveBook\n"));
        assert!(cff_no_doi.contains("license:\n  - MIT\n  - Apache-2.0\n"));
        assert!(cff_no_doi.contains("version: 0.1.0-alpha.2\n"));
        assert!(!cff_no_doi.contains("identifiers:"));

        let cff_with_doi = CURVEBOOK_CITATION.render_citation_cff_with_doi("1.4.0", Some("10.5281/zenodo.1234567"));
        assert!(cff_with_doi.contains("identifiers:\n  - type: doi\n    value: 10.5281/zenodo.1234567\n"));
    }
}
