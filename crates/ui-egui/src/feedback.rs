//! Prefilled GitHub Issue feedback builder for CurveBook (`app.feedback`).
//!
//! Zero-telemetry / zero-egress design: CurveBook never makes outbound network
//! requests. Instead, this module formats user-entered feedback plus strictly
//! sanitized, opt-in environment metadata into either:
//! 1. An allowlisted `https://github.com/HauserGroup/curvebook/issues/new?...`
//!    URL matching `.github/ISSUE_TEMPLATE/{bug_report,feature_request,numerical_result}.yml`, or
//! 2. A full copyable Markdown report for offline / air-gapped workflows or
//!    reports that exceed conservative browser URL length limits.

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::i18n::normalize_locale;

/// Canonical GitHub new-issue base URL for CurveBook.
pub const GITHUB_ISSUES_NEW_BASE: &str = "https://github.com/HauserGroup/curvebook/issues/new";

/// Conservative byte budget for constructed GitHub issue URLs so browser,
/// proxy, and OS URL launchers never silently reject or mangle long reports.
pub const MAX_GITHUB_ISSUE_URL_BYTES: usize = 4096;

/// Hard upper bound accepted by [`validate_github_issue_url`].
pub const MAX_ALLOWED_EXTERNAL_URL_BYTES: usize = 8192;

const TRUNCATION_NOTICE: &str = "\n\n[Truncated for URL length — paste full report from clipboard]";

/// Issue category mapped 1-to-1 to `.github/ISSUE_TEMPLATE/*.yml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    #[default]
    BugReport,
    FeatureRequest,
    NumericalResult,
}

impl FeedbackCategory {
    /// All supported feedback categories in display order.
    pub const ALL: [Self; 3] = [Self::BugReport, Self::FeatureRequest, Self::NumericalResult];

    /// Stable machine identifier used in control parameters and widget IDs.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::BugReport => "bug",
            Self::FeatureRequest => "feature",
            Self::NumericalResult => "numerical",
        }
    }

    /// Parse a category identifier from control/UI parameters (`"bug"`, `"feature"`, `"numerical"`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "bug" | "bug_report" | "bug-report" => Some(Self::BugReport),
            "feature" | "feature_request" | "feature-request" | "enhancement" => Some(Self::FeatureRequest),
            "numerical" | "numerical_result" | "numerical-result" | "wrong_number" => Some(Self::NumericalResult),
            _ => None,
        }
    }

    /// Matching YAML issue template filename under `.github/ISSUE_TEMPLATE/`.
    #[must_use]
    pub const fn template_filename(self) -> &'static str {
        match self {
            Self::BugReport => "bug_report.yml",
            Self::FeatureRequest => "feature_request.yml",
            Self::NumericalResult => "numerical_result.yml",
        }
    }

    /// Human-readable English label for the category selector and Markdown header.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BugReport => "Bug report",
            Self::FeatureRequest => "Feature request",
            Self::NumericalResult => "Wrong numerical result",
        }
    }

    /// Guidance note shown at the top of the feedback modal for this category.
    #[must_use]
    pub const fn guidance(self) -> &'static str {
        match self {
            Self::BugReport => {
                "Opens GitHub's Bug Report form in your browser. Nothing is sent until you submit on GitHub. Do not include confidential data or security vulnerabilities (follow SECURITY.md)."
            }
            Self::FeatureRequest => {
                "Opens GitHub's Feature Request form in your browser. Describe the scientific workflow and cite a public textbook, paper, or specification."
            }
            Self::NumericalResult => {
                "Opens GitHub's Wrong Numerical Result form in your browser. Cite a published oracle (NIST StRD, textbook, paper, or open-source software)."
            }
        }
    }

    /// Label for the primary multi-line input field.
    #[must_use]
    pub const fn summary_label(self) -> &'static str {
        match self {
            Self::BugReport => "What happened",
            Self::FeatureRequest => "What you are trying to do",
            Self::NumericalResult => "What CurveBook reports",
        }
    }

    /// Label for the secondary multi-line input field.
    #[must_use]
    pub const fn details_label(self) -> &'static str {
        match self {
            Self::BugReport => "Steps to reproduce",
            Self::FeatureRequest => "What is missing or incomplete",
            Self::NumericalResult => "What the reference gives",
        }
    }

    /// Label for the optional third input field.
    #[must_use]
    pub const fn extra_label(self) -> &'static str {
        match self {
            Self::BugReport => "Error text or logs (optional)",
            Self::FeatureRequest => "Public specification or citation (optional)",
            Self::NumericalResult => "Analysis or model (optional)",
        }
    }
}

/// Application runtime surface recorded in sanitized diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSurface {
    Desktop,
    Web,
}

impl FeedbackSurface {
    /// Detect the compiled target surface (`Web` on `wasm32`, `Desktop` otherwise).
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_arch = "wasm32") { Self::Web } else { Self::Desktop }
    }

    /// Exact option text matching `.github/ISSUE_TEMPLATE/bug_report.yml`'s `surface` dropdown.
    #[must_use]
    pub const fn github_dropdown_value(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop app",
            Self::Web => "Web app (WebAssembly)",
        }
    }
}

/// Strictly non-identifying environment diagnostics eligible for opt-in inclusion.
///
/// Never includes project names, file paths, dataset contents, or control tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackDiagnostics {
    pub version: String,
    pub surface: FeedbackSurface,
    pub os: String,
    pub arch: String,
    pub locale: String,
    pub theme: String,
}

impl FeedbackDiagnostics {
    /// Capture non-identifying diagnostics from the active [`UiApp`].
    #[must_use]
    pub fn from_app(app: &UiApp) -> Self {
        let build_sha = option_env!("CURVEBOOK_BUILD_SHA");
        let version = format_version_with_sha(env!("CARGO_PKG_VERSION"), build_sha);
        let os = if cfg!(target_arch = "wasm32") { String::from("web") } else { String::from(std::env::consts::OS) };
        let arch = String::from(std::env::consts::ARCH);
        let locale = normalize_locale(&app.locale).to_owned();
        let theme = format!("{:?} ({:.0}%)", app.theme.mode, app.theme.ui_scale * 100.0);
        Self { version, surface: FeedbackSurface::current(), os, arch, locale, theme }
    }

    /// Concise platform line suitable for GitHub's `platform` field.
    #[must_use]
    pub fn platform_line(&self) -> String {
        format!("{} {} · locale {} · theme {}", self.os, self.arch, self.locale, self.theme)
    }

    /// Multi-line preview displayed inside the feedback dialog when diagnostics are enabled.
    #[must_use]
    pub fn preview_text(&self) -> String {
        format!(
            "Version: {}\nSurface: {}\nPlatform: {} ({})\nLocale: {} · Theme: {}",
            self.version,
            self.surface.github_dropdown_value(),
            self.os,
            self.arch,
            self.locale,
            self.theme
        )
    }
}

/// Editable in-app feedback form state persisted inside `CanvasDialogState`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackDraft {
    pub category: FeedbackCategory,
    pub title: String,
    pub summary: String,
    pub details: String,
    pub extra: String,
    pub include_diagnostics: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default)]
    pub copied_markdown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_built_url: Option<String>,
    #[serde(default)]
    pub last_url_truncated: bool,
}

impl Default for FeedbackDraft {
    fn default() -> Self {
        Self {
            category: FeedbackCategory::BugReport,
            title: String::new(),
            summary: String::new(),
            details: String::new(),
            extra: String::new(),
            include_diagnostics: true,
            status_message: None,
            copied_markdown: false,
            last_built_url: None,
            last_url_truncated: false,
        }
    }
}

impl FeedbackDraft {
    /// Reset user-entered form fields while preserving the selected category and diagnostics toggle.
    pub fn clear_fields(&mut self) {
        self.title.clear();
        self.summary.clear();
        self.details.clear();
        self.extra.clear();
        self.status_message = None;
        self.copied_markdown = false;
        self.last_built_url = None;
        self.last_url_truncated = false;
    }
}

/// Result of constructing a prefilled GitHub new-issue URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltFeedbackUrl {
    pub url: String,
    pub truncated: bool,
    pub omitted_chars: usize,
}

/// Format `CurveBook <version>` with an optional truncated build SHA (max 12 chars).
#[must_use]
pub fn format_version_with_sha(version: &str, build_sha: Option<&str>) -> String {
    let short_sha = build_sha.map(str::trim).filter(|s| !s.is_empty()).map(|s| s.chars().take(12).collect::<String>());
    match short_sha {
        Some(sha) => format!("CurveBook {version} ({sha})"),
        None => format!("CurveBook {version}"),
    }
}

/// Percent-encode `input` as a URL query parameter value using RFC 3986 unreserved bytes.
#[must_use]
pub fn percent_encode_query_component(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let is_unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(char::from(*byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(*byte >> 4)]));
            out.push(char::from(HEX[usize::from(*byte & 0x0F)]));
        }
    }
    out
}

/// Validate that `url` is strictly an allowlisted CurveBook GitHub new-issue URL.
///
/// Rejects arbitrary origins, non-HTTPS schemes, whitespace, quotes, ASCII control
/// characters, and oversized strings so URL openers can never act as generic launchers.
///
/// # Errors
/// Returns a descriptive error string when `url` fails any allowlist or hygiene check.
pub fn validate_github_issue_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err(String::from("feedback URL must not be empty"));
    }
    if url.len() > MAX_ALLOWED_EXTERNAL_URL_BYTES {
        return Err(format!("feedback URL exceeds maximum allowed length of {MAX_ALLOWED_EXTERNAL_URL_BYTES} bytes"));
    }
    let Some(rest) = url.strip_prefix(GITHUB_ISSUES_NEW_BASE) else {
        return Err(format!("external URL is restricted to `{GITHUB_ISSUES_NEW_BASE}`"));
    };
    if !rest.is_empty() && !rest.starts_with('?') {
        return Err(format!("external URL path must match `{GITHUB_ISSUES_NEW_BASE}` exactly"));
    }
    if url.bytes().any(|b| b <= 0x20 || b == 0x7F || matches!(b, b'"' | b'\'' | b'<' | b'>' | b'`' | b'\\')) {
        return Err(String::from("feedback URL contains disallowed whitespace, control, or shell-quote characters"));
    }
    Ok(())
}

/// Build a prefilled GitHub issue URL capped cleanly at `max_bytes` (default [`MAX_GITHUB_ISSUE_URL_BYTES`]).
#[must_use]
pub fn build_github_issue_url_capped(draft: &FeedbackDraft, diag: Option<&FeedbackDiagnostics>, max_bytes: usize) -> BuiltFeedbackUrl {
    let budget = max_bytes.max(GITHUB_ISSUES_NEW_BASE.len() + 64);
    let pairs = collect_query_pairs(draft, diag);

    let mut url = String::from(GITHUB_ISSUES_NEW_BASE);
    let mut first = true;
    let mut truncated = false;
    let mut omitted_chars = 0usize;

    for (key, value) in pairs {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let sep = if first { '?' } else { '&' };
        let prefix_len = 1 + key.len() + 1; // sep + key + '='
        let encoded_full = percent_encode_query_component(trimmed);

        if url.len() + prefix_len + encoded_full.len() <= budget {
            url.push(sep);
            url.push_str(key);
            url.push('=');
            url.push_str(&encoded_full);
            first = false;
            continue;
        }

        truncated = true;
        let remaining = budget.saturating_sub(url.len() + prefix_len);
        let notice_enc = percent_encode_query_component(TRUNCATION_NOTICE);
        if remaining > notice_enc.len() + 12 {
            let value_budget = remaining - notice_enc.len();
            let (fit_enc, kept_chars) = encode_prefix_within_bytes(trimmed, value_budget);
            let total_chars = trimmed.chars().count();
            omitted_chars = omitted_chars.saturating_add(total_chars.saturating_sub(kept_chars));
            url.push(sep);
            url.push_str(key);
            url.push('=');
            url.push_str(&fit_enc);
            url.push_str(&notice_enc);
            first = false;
        } else {
            omitted_chars = omitted_chars.saturating_add(trimmed.chars().count());
        }
    }

    BuiltFeedbackUrl { url, truncated, omitted_chars }
}

/// Build a prefilled GitHub issue URL using [`MAX_GITHUB_ISSUE_URL_BYTES`].
#[must_use]
pub fn build_github_issue_url(draft: &FeedbackDraft, diag: Option<&FeedbackDiagnostics>) -> BuiltFeedbackUrl {
    build_github_issue_url_capped(draft, diag, MAX_GITHUB_ISSUE_URL_BYTES)
}

/// Format the complete, untruncated feedback report as Markdown suitable for clipboard copy.
#[must_use]
pub fn build_feedback_markdown(draft: &FeedbackDraft, diag: Option<&FeedbackDiagnostics>) -> String {
    let mut out = String::new();
    let title = formatted_issue_title(draft);
    out.push_str("# ");
    if title.is_empty() {
        out.push_str(draft.category.label());
    } else {
        out.push_str(&title);
    }
    out.push_str("\n\n");
    out.push_str(&format!("- **Category:** {}\n", draft.category.label()));
    out.push_str(&format!("- **Template:** `.github/ISSUE_TEMPLATE/{}`\n", draft.category.template_filename()));
    if let Some(d) = diag {
        out.push_str(&format!("- **Version:** {}\n", d.version));
        out.push_str(&format!("- **Surface:** {}\n", d.surface.github_dropdown_value()));
        out.push_str(&format!("- **Platform:** {}\n", d.platform_line()));
    }

    append_md_section(&mut out, draft.category.summary_label(), &draft.summary);
    append_md_section(&mut out, draft.category.details_label(), &draft.details);
    append_md_section(&mut out, draft.category.extra_label(), &draft.extra);
    out
}

fn append_md_section(out: &mut String, heading: &str, body: &str) {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return;
    }
    out.push_str("\n## ");
    out.push_str(heading);
    out.push_str("\n\n");
    out.push_str(trimmed);
    out.push('\n');
}

fn formatted_issue_title(draft: &FeedbackDraft) -> String {
    let raw = draft.title.trim();
    if raw.is_empty() {
        return String::new();
    }
    match draft.category {
        FeedbackCategory::NumericalResult => {
            if raw.to_ascii_lowercase().starts_with("numerical:") {
                raw.to_owned()
            } else {
                format!("Numerical: {raw}")
            }
        }
        FeedbackCategory::BugReport | FeedbackCategory::FeatureRequest => raw.to_owned(),
    }
}

fn collect_query_pairs(draft: &FeedbackDraft, diag: Option<&FeedbackDiagnostics>) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::with_capacity(8);
    pairs.push(("template", String::from(draft.category.template_filename())));

    let title = formatted_issue_title(draft);
    if !title.is_empty() {
        pairs.push(("title", title));
    }

    match draft.category {
        FeedbackCategory::BugReport => {
            if let Some(d) = diag {
                pairs.push(("surface", String::from(d.surface.github_dropdown_value())));
                pairs.push(("version", d.version.clone()));
                pairs.push(("platform", d.platform_line()));
            }
            pairs.push(("what-happened", draft.summary.clone()));
            pairs.push(("steps", draft.details.clone()));
            pairs.push(("logs", draft.extra.clone()));
        }
        FeedbackCategory::FeatureRequest => {
            let mut need = draft.summary.trim().to_owned();
            if let Some(d) = diag {
                let env_note = format!("Environment: {} ({}; {})", d.version, d.surface.github_dropdown_value(), d.platform_line());
                if need.is_empty() {
                    need = env_note;
                } else {
                    need.push_str("\n\n");
                    need.push_str(&env_note);
                }
            }
            pairs.push(("need", need));
            pairs.push(("missing", draft.details.clone()));
            pairs.push(("spec", draft.extra.clone()));
        }
        FeedbackCategory::NumericalResult => {
            if let Some(d) = diag {
                pairs.push(("version", d.version.clone()));
                pairs.push(("platform", d.platform_line()));
            }
            pairs.push(("analysis", draft.extra.clone()));
            pairs.push(("observed", draft.summary.clone()));
            pairs.push(("expected", draft.details.clone()));
        }
    }

    pairs
}

fn encode_prefix_within_bytes(input: &str, max_encoded_bytes: usize) -> (String, usize) {
    let mut out = String::new();
    let mut kept_chars = 0usize;
    let mut buf = [0u8; 4];
    for ch in input.chars() {
        let piece = percent_encode_query_component(ch.encode_utf8(&mut buf));
        if out.len() + piece.len() > max_encoded_bytes {
            break;
        }
        out.push_str(&piece);
        kept_chars += 1;
    }
    (out, kept_chars)
}

#[cfg(test)]
mod tests {
    use super::{
        FeedbackCategory, FeedbackDiagnostics, FeedbackDraft, FeedbackSurface, GITHUB_ISSUES_NEW_BASE, MAX_GITHUB_ISSUE_URL_BYTES,
        build_feedback_markdown, build_github_issue_url, build_github_issue_url_capped, percent_encode_query_component, validate_github_issue_url,
    };

    fn sample_diag() -> FeedbackDiagnostics {
        FeedbackDiagnostics {
            version: String::from("CurveBook 0.1.0-alpha.2 (abc123def456)"),
            surface: FeedbackSurface::Desktop,
            os: String::from("macos"),
            arch: String::from("aarch64"),
            locale: String::from("en"),
            theme: String::from("Light (100%)"),
        }
    }

    #[test]
    fn percent_encoding_handles_ascii_specials_newlines_and_utf8() {
        assert_eq!(percent_encode_query_component("abc-123_4.5~"), "abc-123_4.5~");
        assert_eq!(percent_encode_query_component("a + b = c & d\nline2"), "a%20%2B%20b%20%3D%20c%20%26%20d%0Aline2");
        assert_eq!(percent_encode_query_component("µM ± 0.5"), "%C2%B5M%20%C2%B1%200.5");
    }

    #[test]
    fn bug_report_url_maps_to_bug_template_fields() {
        let draft = FeedbackDraft {
            category: FeedbackCategory::BugReport,
            title: String::from("Axis tick overlap on log10 scale"),
            summary: String::from("Ticks overlap at 1e-3 and 1e-2"),
            details: String::from("1. Create XY table\n2. Enable Log10 X axis"),
            extra: String::from("WARN: tick collision"),
            include_diagnostics: true,
            ..FeedbackDraft::default()
        };
        let diag = sample_diag();
        let built = build_github_issue_url(&draft, Some(&diag));
        assert!(!built.truncated);
        assert_eq!(built.omitted_chars, 0);
        validate_github_issue_url(&built.url).expect("generated URL must pass allowlist");
        assert!(built.url.starts_with("https://github.com/HauserGroup/curvebook/issues/new?template=bug_report.yml"));
        assert!(built.url.contains("&title=Axis%20tick%20overlap%20on%20log10%20scale"));
        assert!(built.url.contains("&surface=Desktop%20app"));
        assert!(built.url.contains("&version=CurveBook%200.1.0-alpha.2%20%28abc123def456%29"));
        assert!(built.url.contains("&platform=macos%20aarch64"));
        assert!(built.url.contains("&what-happened=Ticks%20overlap"));
        assert!(built.url.contains("&steps=1.%20Create%20XY%20table%0A2.%20Enable%20Log10%20X%20axis"));
        assert!(built.url.contains("&logs=WARN%3A%20tick%20collision"));
    }

    #[test]
    fn feature_and_numerical_templates_map_cleanly_without_double_prefix() {
        let feature = FeedbackDraft {
            category: FeedbackCategory::FeatureRequest,
            title: String::from("Add Bland-Altman confidence bands"),
            summary: String::from("Compare two assay instruments"),
            details: String::from("Shaded CI bands around limits of agreement"),
            extra: String::from("Bland & Altman (1986), Lancet"),
            include_diagnostics: false,
            ..FeedbackDraft::default()
        };
        let feat_url = build_github_issue_url(&feature, None);
        validate_github_issue_url(&feat_url.url).expect("feature URL valid");
        assert!(feat_url.url.contains("template=feature_request.yml"));
        assert!(feat_url.url.contains("&need=Compare%20two%20assay%20instruments"));
        assert!(feat_url.url.contains("&missing=Shaded%20CI%20bands"));
        assert!(feat_url.url.contains("&spec=Bland%20%26%20Altman"));
        assert!(!feat_url.url.contains("macos"), "diagnostics must be omitted when disabled");

        let numerical = FeedbackDraft {
            category: FeedbackCategory::NumericalResult,
            title: String::from("Welch t-test df rounding"),
            summary: String::from("df = 14.21"),
            details: String::from("df = 14.22 in R stats::t.test"),
            extra: String::from("Welch t-test, two-sided"),
            include_diagnostics: true,
            ..FeedbackDraft::default()
        };
        let num_url = build_github_issue_url(&numerical, Some(&sample_diag()));
        validate_github_issue_url(&num_url.url).expect("numerical URL valid");
        assert!(num_url.url.contains("template=numerical_result.yml"));
        assert!(num_url.url.contains("&title=Numerical%3A%20Welch%20t-test%20df%20rounding"));
        assert!(num_url.url.contains("&analysis=Welch%20t-test"));
        assert!(num_url.url.contains("&observed=df%20%3D%2014.21"));
        assert!(num_url.url.contains("&expected=df%20%3D%2014.22"));

        let already_prefixed = FeedbackDraft {
            category: FeedbackCategory::NumericalResult,
            title: String::from("Numerical: Already prefixed"),
            ..FeedbackDraft::default()
        };
        let pref_url = build_github_issue_url(&already_prefixed, None);
        assert!(pref_url.url.contains("&title=Numerical%3A%20Already%20prefixed"));
        assert!(!pref_url.url.contains("Numerical%3A%20Numerical%3A"));
    }

    #[test]
    fn oversized_payload_truncates_honestly_within_budget_and_preserves_markdown() {
        let huge = "αβγ-log-line ".repeat(600);
        let draft = FeedbackDraft {
            category: FeedbackCategory::BugReport,
            title: String::from("Long log test"),
            summary: huge.clone(),
            details: String::from("Step 1"),
            extra: huge,
            include_diagnostics: true,
            ..FeedbackDraft::default()
        };
        let built = build_github_issue_url_capped(&draft, Some(&sample_diag()), MAX_GITHUB_ISSUE_URL_BYTES);
        assert!(built.truncated);
        assert!(built.omitted_chars > 0);
        assert!(built.url.len() <= MAX_GITHUB_ISSUE_URL_BYTES);
        validate_github_issue_url(&built.url).expect("truncated URL must remain valid");
        assert!(built.url.contains("Truncated%20for%20URL%20length"));

        let md = build_feedback_markdown(&draft, Some(&sample_diag()));
        assert!(md.contains("# Long log test"));
        assert!(md.contains("## What happened"));
        assert!(md.contains("αβγ-log-line"));
        assert!(md.len() > MAX_GITHUB_ISSUE_URL_BYTES, "Markdown copy must remain untruncated");
    }

    #[test]
    fn allowlist_rejects_non_curvebook_and_malformed_urls() {
        assert!(validate_github_issue_url(GITHUB_ISSUES_NEW_BASE).is_ok());
        assert!(validate_github_issue_url("https://github.com/HauserGroup/curvebook/issues/new?template=bug_report.yml").is_ok());

        for bad in [
            "",
            "http://github.com/HauserGroup/curvebook/issues/new",
            "https://github.com/HauserGroup/curvebook/issues/new.evil.example/?x=1",
            "https://github.com/other/repo/issues/new",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://github.com/HauserGroup/curvebook/issues/new?title=hello world",
            "https://github.com/HauserGroup/curvebook/issues/new?title=a\nb",
            "https://github.com/HauserGroup/curvebook/issues/new?title=\"quoted\"",
        ] {
            assert!(validate_github_issue_url(bad).is_err(), "expected `{bad}` to be rejected");
        }
    }
}
