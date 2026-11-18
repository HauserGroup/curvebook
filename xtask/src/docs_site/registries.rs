//! Typed registry reference generators for the CurveBook static manual (`R12.1`, `R12.4`).
//!
//! Generates complete HTML reference pages directly from compiled Rust metadata so names,
//! parameters, annotations, bounds, formats, and preference keys cannot drift:
//! - `registry/commands.html`: `cmd::registry()` + `UI_COMMANDS`
//! - `registry/mcp-tools.html`: core, dynamic `CommandGroup`, and `gui` MCP tools + resources + prompts
//! - `registry/cli.html`: `cli_catalog` subcommands, flags, defaults, and exit codes
//! - `registry/equations.html`: `curvebook-fit` built-in equations, formulas, params, bounds, methods, weights
//! - `registry/tables.html`: `TableKind`, `SubcolumnMode`, `AnalysisKind`, `PowerTestFamily`, `STANDARD_INFO_KEYS`, `ErrorCode`
//! - `registry/graphs.html`: `GraphKind`, `DiagnosticPlotKind`, `GRAPH_TYPE_OPTIONS`, `FramePreset`, `AxisScale`, palettes, colormaps
//! - `registry/formats.html`: `FormatRegistry` importers/exporters + `docs/schemas/*.schema.json` + `LOCALES`

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use curvebook_doc::{
    AnalysisKind, AxisScale, DiagnosticPlotKind, ErrorCode, FitMethod, FramePreset, GraphKind, LogitDomain, ParameterScope, STANDARD_INFO_KEYS,
    SubcolumnMode, TableKind, WeightingScheme,
};
use curvebook_fit::{DataPoint, EquationCategory, format_model_equation_tex, get_builtin_model, list_builtin_equations};
use curvebook_format::registry::{FormatDescriptor, FormatKind, FormatRegistry};
use curvebook_mcp::{
    PromptDescriptor, ResourceDescriptor, ToolDefinition, list_gui_tools, list_prompts_for_mode, list_resources, list_tools, list_tools_for_group,
};
use curvebook_palette::{ColormapId, PaletteId};
use curvebook_stats::power::PowerTestFamily;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, GRAPH_TYPE_OPTIONS};
use curvebook_ui_egui::i18n::LOCALES;
use curvebook_ui_egui::menus::UI_COMMANDS;

use super::markdown::{TocHeading, escape_html, github_slug};

#[path = "../../../apps/curvebook-cli/src/cli_catalog.rs"]
#[allow(dead_code)]
mod cli_catalog;

/// One generated registry HTML page prior to wrapping in the Classic coral template.
#[derive(Debug, Clone)]
pub struct RegistryPageSpec {
    pub title: String,
    pub description: String,
    pub rel_path: String,
    pub section_title: &'static str,
    pub section_rel_path: &'static str,
    pub h1: String,
    pub toc_headings: Vec<TocHeading>,
    pub body_html: String,
}

/// Entry contributed by registries or Markdown pages to `az-index.html`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzEntry {
    pub term: String,
    pub category: String,
    pub rel_href: String,
}

/// Expected canonical identifiers collected directly from live Rust registries for `--check`.
#[derive(Debug, Clone, Default)]
pub struct ExpectedCoverage {
    pub engine_commands: Vec<String>,
    pub ui_commands: Vec<String>,
    pub mcp_tools: Vec<String>,
    pub mcp_prompts: Vec<String>,
    pub cli_commands: Vec<String>,
    pub analysis_kinds: Vec<String>,
    pub fit_equations: Vec<String>,
    pub graph_kinds: Vec<String>,
    pub graph_options: Vec<String>,
    pub table_kinds: Vec<String>,
    pub formats: Vec<String>,
    pub preference_keys: Vec<String>,
}

/// Bundle returned by [`generate_registry_pages`].
pub struct RegistryBundle {
    pub pages: Vec<RegistryPageSpec>,
    pub az_entries: Vec<AzEntry>,
    pub expected: ExpectedCoverage,
}

/// Build all seven registry reference pages, A–Z entries, and coverage expectations from `root`.
pub fn generate_registry_pages(root: &Path) -> Result<RegistryBundle, String> {
    let mut az_entries = Vec::new();
    let mut expected = ExpectedCoverage::default();

    let commands_page = build_commands_page(&mut az_entries, &mut expected);
    let mcp_page = build_mcp_page(&mut az_entries, &mut expected);
    let cli_page = build_cli_page(&mut az_entries, &mut expected);
    let equations_page = build_equations_page(&mut az_entries, &mut expected);
    let tables_page = build_tables_page(&mut az_entries, &mut expected);
    let graphs_page = build_graphs_page(&mut az_entries, &mut expected);
    let formats_page = build_formats_page(root, &mut az_entries, &mut expected)?;

    Ok(RegistryBundle {
        pages: vec![cli_page, commands_page, equations_page, formats_page, graphs_page, mcp_page, tables_page],
        az_entries,
        expected,
    })
}

/// Canonical slug for any [`AnalysisKind`] matching `@r12-methods-ref`'s `{#analysis-<slug>}` contract.
#[must_use]
pub const fn analysis_kind_slug(kind: &AnalysisKind) -> &'static str {
    match kind {
        AnalysisKind::Transform(_) => "transform",
        AnalysisKind::DescriptiveStats(_) => "descriptive-stats",
        AnalysisKind::OutlierDetection(_) => "outlier-detection",
        AnalysisKind::TwoGroupComparison(_) => "two-group-comparison",
        AnalysisKind::BlandAltman(_) => "bland-altman",
        AnalysisKind::MultipleRowTTests(_) => "multiple-row-t-tests",
        AnalysisKind::PassingBablok(_) => "passing-bablok",
        AnalysisKind::OneWayAnova(_) => "one-way-anova",
        AnalysisKind::FactorialAnova(_) => "factorial-anova",
        AnalysisKind::NestedComparison(_) => "nested-comparison",
        AnalysisKind::Contingency(_) => "contingency",
        AnalysisKind::Survival(_) => "survival",
        AnalysisKind::RocCurve(_) => "roc-curve",
        AnalysisKind::GeneralizedLinearModel(_) => "generalized-linear-model",
        AnalysisKind::Multivariate(_) => "multivariate",
        AnalysisKind::LinearRegression(_) => "linear-regression",
        AnalysisKind::NonlinearRegression(_) => "nonlinear-regression",
        AnalysisKind::PValueStack(_) => "p-value-stack",
        AnalysisKind::PowerAnalysis(_) => "power-analysis",
    }
}

/// All 19 [`AnalysisKind`] metadata tuples `(slug, title, description)`.
#[must_use]
pub fn all_analysis_kinds_meta() -> Vec<(&'static str, &'static str, &'static str)> {
    // Compile-time exhaustive match guard referencing `analysis_kind_slug`: if a new variant is
    // added to `AnalysisKind`, `analysis_kind_slug` above will fail to compile until documented.
    let _guard: fn(&AnalysisKind) -> &'static str = analysis_kind_slug;
    vec![
        (
            "transform",
            "Transforms & Data Manipulation",
            "Mathematical transforms, normalization, baseline subtraction, transpose, row pruning, simulation, and random sampling.",
        ),
        (
            "descriptive-stats",
            "Descriptive Statistics & Normality",
            "Univariate location, spread, quantiles, normality battery, and one-sample location tests.",
        ),
        (
            "outlier-detection",
            "Outlier Detection (Grubbs / ROUT)",
            "Single-outlier Grubbs ESD and multi-outlier ROUT false-discovery-rate detection.",
        ),
        (
            "two-group-comparison",
            "Two-Group & One-Sample Comparisons",
            "Parametric t-tests (Student, Welch, paired, ratio, lognormal) and nonparametric rank/sign tests.",
        ),
        ("bland-altman", "Bland–Altman Method Comparison", "Method-comparison bias and 95% limits of agreement."),
        ("multiple-row-t-tests", "Multiple Row-by-Row Two-Group Tests", "Row-wise parametric or nonparametric comparisons with FWER/FDR correction."),
        ("passing-bablok", "Passing–Bablok Regression", "Nonparametric errors-in-variables linear method-comparison regression."),
        (
            "one-way-anova",
            "One-Way ANOVA & Nonparametric Equivalents",
            "Ordinary, Brown–Forsythe/Welch, repeated-measures/REML, Kruskal–Wallis, and Friedman ANOVA with post-hoc multiple comparisons.",
        ),
        (
            "factorial-anova",
            "Two-Way, Three-Way & Factorial ANOVA",
            "Multi-factor ANOVA / REML mixed models with main effects, interactions, and simple-effects slicing.",
        ),
        ("nested-comparison", "Nested t-Test & Nested One-Way ANOVA", "Hierarchical nested random-effects / mixed-model comparisons."),
        (
            "contingency",
            "Contingency & Proportion Tests",
            "Chi-square, Fisher's exact, chi-square for trend, McNemar, Cohen's kappa, and exact binomial tests.",
        ),
        (
            "survival",
            "Survival Analysis & Cox Regression",
            "Kaplan–Meier survival curves, log-rank/Gehan-Breslow-Wilcoxon tests, and Cox proportional hazards regression.",
        ),
        (
            "roc-curve",
            "ROC Curve Analysis",
            "Receiver operating characteristic curves, DeLong/Hanley–McNeil AUC confidence intervals, Youden cutoffs, and AUC comparison.",
        ),
        (
            "generalized-linear-model",
            "Generalized Linear Models (GLM)",
            "Multiple linear, logistic, and Poisson regression with link diagnostics and likelihood tests.",
        ),
        (
            "multivariate",
            "Multivariate Analysis (PCA, Correlation, Clustering, MI)",
            "Principal component analysis, Pearson/Spearman correlation matrices, k-means/hierarchical clustering, and Rubin multiple imputation.",
        ),
        ("linear-regression", "Simple & Deming Linear Regression", "Model I OLS, ANCOVA slope/intercept comparison, and Model II Deming regression."),
        (
            "nonlinear-regression",
            "Nonlinear Regression & Custom Equations",
            "Trust-region Levenberg–Marquardt, Poisson deviance, Cauchy robust, and ROUT curve fitting with DSL/ODE support.",
        ),
        ("p-value-stack", "P-Value Stack Multiplicity Adjustment", "Standalone FWER and FDR adjustment over a pre-computed column of P values."),
        ("power-analysis", "Statistical Power & Sample Size", "Prospective power, sample-size, and minimum-detectable-effect calculations."),
    ]
}

fn kebab_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

fn push_heading(toc: &mut Vec<TocHeading>, out: &mut String, level: u8, id: &str, text: &str) {
    toc.push(TocHeading { level, id: id.to_owned(), text: text.to_owned() });
    let _ = writeln!(out, "<h{level} id=\"{}\" tabindex=\"-1\">{}</h{level}>", escape_html(id), escape_html(text));
}

fn build_commands_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let reg = curvebook_engine::cmd::registry();
    let specs: Vec<_> = reg.specs().collect();
    let mut toc = Vec::new();
    let mut body = String::with_capacity(32_768);

    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete catalog of <strong>{}</strong> engine commands compiled from <code>curvebook_engine::cmd::registry()</code> and <strong>{}</strong> desktop shell commands from <code>UI_COMMANDS</code>.</p>",
        specs.len(),
        UI_COMMANDS.len(),
    );
    let _ = writeln!(
        body,
        "<p>For wire execution syntax across MCP, CLI, and loopback <code>--control</code>, see <a href=\"../automation/mcp.html\">MCP Server Reference</a>, <a href=\"../automation/cli.html\">CLI Reference</a>, and <a href=\"mcp-tools.html\">MCP Tools Registry</a>.</p>"
    );

    push_heading(&mut toc, &mut body, 2, "engine-commands", "Engine Commands (cmd::registry)");
    for group in curvebook_engine::cmd::CommandGroup::all() {
        let group_specs: Vec<_> = specs.iter().copied().filter(|s| s.group == *group).collect();
        let group_id = format!("group-{}", group.as_str());
        let group_heading = format!("{} ({})", group.title(), group_specs.len());
        push_heading(&mut toc, &mut body, 3, &group_id, &group_heading);

        for spec in group_specs {
            let cmd_anchor = format!("cmd-{}", kebab_id(spec.id).replace('_', "-"));
            expected.engine_commands.push(cmd_anchor.clone());
            az.push(AzEntry {
                term: spec.id.to_owned(),
                category: format!("Command ({})", group.as_str()),
                rel_href: format!("registry/commands.html#{cmd_anchor}"),
            });

            let mcp_name = curvebook_mcp::toolsets::command_id_to_mcp_tool_name(spec.id);
            let effects = spec.effects.flag_names();
            let effects_label = if effects.is_empty() { String::from("none") } else { effects.join(", ") };
            let shortcut = if spec.shortcut.is_empty() { "—" } else { spec.shortcut };
            let menu_path = if spec.menu_path.is_empty() { "—" } else { spec.menu_path };
            let schema = (spec.input_schema)();
            let schema_pretty = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| String::from("{}"));

            let _ = writeln!(body, "<div class=\"entry-card\" id=\"{cmd_anchor}\" tabindex=\"-1\">");
            let _ = writeln!(body, "  <h4><code>{}</code> — {}</h4>", escape_html(spec.id), escape_html(spec.label));
            let _ = writeln!(body, "  <p>{}</p>", escape_html(spec.summary));
            let _ = writeln!(body, "  <div class=\"badge-row\">");
            let _ = writeln!(body, "    <span class=\"badge\">group: {}</span>", escape_html(group.as_str()));
            let _ = writeln!(body, "    <span class=\"badge\">effects: {}</span>", escape_html(&effects_label));
            let _ = writeln!(body, "    <span class=\"badge\">menu: {}</span>", escape_html(menu_path));
            let _ = writeln!(body, "    <span class=\"badge\">shortcut: {}</span>", escape_html(shortcut));
            let _ = writeln!(body, "  </div>");
            let _ = writeln!(
                body,
                "  <p>MCP tool: <a href=\"mcp-tools.html#mcp-tool-{}\"><code>{}</code></a> · CLI: <code>curvebook-cli {} [flags]</code></p>",
                escape_html(&mcp_name),
                escape_html(&mcp_name),
                escape_html(spec.id),
            );
            let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>", escape_html(&schema_pretty));
            let _ = writeln!(body, "</div>");
        }
    }

    push_heading(&mut toc, &mut body, 2, "ui-commands", "Desktop UI Commands (UI_COMMANDS)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>ID</th><th>Label</th><th>Shortcut</th><th>Params</th><th>Summary</th></tr></thead>\n<tbody>\n");
    for ui in UI_COMMANDS {
        let ui_anchor = format!("ui-cmd-{}", kebab_id(ui.id).replace('_', "-"));
        expected.ui_commands.push(ui_anchor.clone());
        az.push(AzEntry { term: ui.id.to_owned(), category: String::from("UI Command"), rel_href: format!("registry/commands.html#{ui_anchor}") });
        let shortcut = if ui.shortcut.is_empty() { "—" } else { ui.shortcut };
        let _ = writeln!(
            body,
            "<tr id=\"{ui_anchor}\" tabindex=\"-1\"><td><code>{}</code></td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            escape_html(ui.id),
            escape_html(ui.label),
            escape_html(shortcut),
            escape_html(ui.params_doc),
            escape_html(ui.summary),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    RegistryPageSpec {
        title: String::from("Engine & UI Command Registry"),
        description: String::from("Complete registry of CurveBook engine commands, JSON input schemas, effect masks, and UI commands."),
        rel_path: String::from("registry/commands.html"),
        section_title: "Automation",
        section_rel_path: "section-automation.html",
        h1: String::from("Engine & UI Command Registry"),
        toc_headings: toc,
        body_html: body,
    }
}

fn build_mcp_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let mut toc = Vec::new();
    let mut body = String::with_capacity(65_536);

    let core_tools = list_tools();
    let gui_tools = list_gui_tools();
    let groups = curvebook_engine::cmd::CommandGroup::all();

    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete MCP JSON-RPC 2.0 surface generated from <code>curvebook-mcp</code>: <strong>{}</strong> core tools, <strong>{}</strong> interactive <code>gui</code> tools, <strong>{}</strong> dynamic <code>CommandGroup</code> toolsets, resources, and prompts.</p>",
        core_tools.len(),
        gui_tools.len(),
        groups.len(),
    );
    let _ = writeln!(
        body,
        "<p>See <a href=\"../automation/mcp.html\">MCP Server Reference</a> for transport framing and <code>--gui-only</code> policy details.</p>"
    );

    push_heading(&mut toc, &mut body, 2, "core-tools", "Core MCP Tools");
    for tool in &core_tools {
        render_mcp_tool_card(tool, "MCP Core Tool", az, expected, &mut body);
    }

    push_heading(&mut toc, &mut body, 2, "mcp-toolsets", "Dynamic CommandGroup Toolsets");
    for group in groups {
        let group_tools = list_tools_for_group(*group);
        let group_id = format!("mcp-toolset-{}", group.as_str());
        let heading = format!("Toolset: {} ({})", group.as_str(), group_tools.len());
        push_heading(&mut toc, &mut body, 3, &group_id, &heading);
        for tool in &group_tools {
            render_mcp_tool_card(tool, &format!("MCP Toolset ({})", group.as_str()), az, expected, &mut body);
        }
    }

    push_heading(&mut toc, &mut body, 2, "gui-toolset", "Interactive GUI Toolset (gui)");
    for tool in &gui_tools {
        render_mcp_tool_card(tool, "MCP GUI Tool", az, expected, &mut body);
    }

    push_heading(&mut toc, &mut body, 2, "mcp-resources", "Resource URI Schemes");
    let mut sample_resources: Vec<ResourceDescriptor> = Vec::new();
    if let Ok(mut sample_doc) = curvebook_doc::ProjectDocument::new("Sample") {
        if let Ok(xy) = curvebook_doc::XyTable::new(SubcolumnMode::Single, 1, 2)
            && let Ok(ds) = curvebook_doc::DataSheet::new("data-1", "Data 1", curvebook_doc::TableArchetype::Xy(xy))
        {
            let _ = sample_doc.add_data_sheet(ds);
        }
        sample_resources = list_resources(&sample_doc);
    }
    let _ = writeln!(
        body,
        "<div class=\"table-wrap\"><table>\n<thead><tr><th>URI Scheme</th><th>MIME Type</th><th>Sample Descriptor</th><th>Policy</th></tr></thead>\n<tbody>"
    );
    for res in &sample_resources {
        let _ = writeln!(
            body,
            "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{} — {}</td><td>Disabled under <code>--gui-only</code></td></tr>",
            escape_html(&res.uri),
            escape_html(&res.mime_type),
            escape_html(&res.name),
            escape_html(&res.description),
        );
    }
    let _ = writeln!(
        body,
        "<tr><td><code>result://&lt;id&gt;</code></td><td><code>application/json</code></td><td>Serialized computed <code>ResultSheet</code> tabs and diagnostics</td><td>Disabled under <code>--gui-only</code></td></tr>"
    );
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "mcp-prompts", "Built-in Workflow Prompts");
    let prompts_normal: Vec<PromptDescriptor> = list_prompts_for_mode(false);
    let prompts_gui: Vec<PromptDescriptor> = list_prompts_for_mode(true);
    for (idx, p) in prompts_normal.iter().enumerate() {
        let anchor = format!("mcp-prompt-{}", p.name);
        expected.mcp_prompts.push(anchor.clone());
        az.push(AzEntry { term: p.name.clone(), category: String::from("MCP Prompt"), rel_href: format!("registry/mcp-tools.html#{anchor}") });
        let gui_desc = prompts_gui.get(idx).map(|g| g.description.as_str()).unwrap_or("");
        let _ = writeln!(body, "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">");
        let _ = writeln!(body, "  <h4><code>{}</code></h4>", escape_html(&p.name));
        let _ = writeln!(body, "  <p><strong>Standard mode:</strong> {}</p>", escape_html(&p.description));
        if !gui_desc.is_empty() {
            let _ = writeln!(body, "  <p><strong>GUI-only mode:</strong> {}</p>", escape_html(gui_desc));
        }
        if !p.arguments.is_empty() {
            body.push_str("  <ul>\n");
            for arg in &p.arguments {
                let req = if arg.required { "required" } else { "optional" };
                let _ = writeln!(body, "    <li><code>{}</code> ({req}): {}</li>", escape_html(&arg.name), escape_html(&arg.description));
            }
            body.push_str("  </ul>\n");
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "prompts/get",
            "params": { "name": p.name }
        });
        let req_str = serde_json::to_string(&req).unwrap_or_else(|_| String::from("{}"));
        let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>", escape_html(&req_str));
        let _ = writeln!(body, "</div>");
    }

    RegistryPageSpec {
        title: String::from("MCP Tools, Toolsets, Resources & Prompts"),
        description: String::from(
            "Machine-generated schemas, tool annotations, single-call JSON examples, resources, and prompts for CurveBook MCP.",
        ),
        rel_path: String::from("registry/mcp-tools.html"),
        section_title: "Automation",
        section_rel_path: "section-automation.html",
        h1: String::from("MCP Tools, Toolsets, Resources & Prompts"),
        toc_headings: toc,
        body_html: body,
    }
}

fn render_mcp_tool_card(tool: &ToolDefinition, az_category: &str, az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage, body: &mut String) {
    let anchor = format!("mcp-tool-{}", tool.name);
    expected.mcp_tools.push(anchor.clone());
    az.push(AzEntry { term: tool.name.clone(), category: az_category.to_owned(), rel_href: format!("registry/mcp-tools.html#{anchor}") });

    let ann = &tool.annotations;
    let req_json = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool.name,
            "arguments": minimal_example_args(&tool.input_schema)
        }
    });
    let resp_json = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "content": [{ "type": "text", "text": "{...}" }],
            "isError": false
        }
    });
    let schema_pretty = serde_json::to_string_pretty(&tool.input_schema).unwrap_or_else(|_| String::from("{}"));
    let req_str = serde_json::to_string(&req_json).unwrap_or_else(|_| String::from("{}"));
    let resp_str = serde_json::to_string(&resp_json).unwrap_or_else(|_| String::from("{}"));

    let _ = writeln!(body, "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">");
    let _ = writeln!(body, "  <h4><code>{}</code></h4>", escape_html(&tool.name));
    let _ = writeln!(body, "  <p>{}</p>", escape_html(&tool.description));
    let _ = writeln!(body, "  <div class=\"badge-row\">");
    let _ = writeln!(body, "    <span class=\"badge\">readOnlyHint: {}</span>", ann.read_only_hint);
    let _ = writeln!(body, "    <span class=\"badge\">destructiveHint: {}</span>", ann.destructive_hint);
    let _ = writeln!(body, "    <span class=\"badge\">idempotentHint: {}</span>", ann.idempotent_hint);
    let _ = writeln!(body, "    <span class=\"badge\">openWorldHint: {}</span>", ann.open_world_hint);
    let _ = writeln!(body, "  </div>");
    let _ = writeln!(body, "  <p><strong>Input schema:</strong></p>");
    let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>", escape_html(&schema_pretty));
    let _ = writeln!(body, "  <p><strong>Single-call request / response syntax:</strong></p>");
    let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>", escape_html(&req_str));
    let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>", escape_html(&resp_str));
    let _ = writeln!(body, "</div>");
}

fn minimal_example_args(schema: &serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    if let Some(props) = schema.get("properties").and_then(serde_json::Value::as_object) {
        for req in required {
            if let Some(prop) = props.get(req) {
                let sample = match prop.get("enum").and_then(serde_json::Value::as_array).and_then(|a| a.first()) {
                    Some(v) if v.as_str() != Some("prism") => v.clone(),
                    _ => match prop.get("type").and_then(serde_json::Value::as_str) {
                        Some("boolean") => serde_json::Value::Bool(true),
                        Some("integer") => serde_json::json!(1),
                        Some("number") => serde_json::json!(1.0),
                        Some("array") => serde_json::json!([]),
                        Some("object") => serde_json::json!({}),
                        _ => serde_json::Value::String(format!("<{req}>")),
                    },
                };
                map.insert(req.to_owned(), sample);
            }
        }
    }
    serde_json::Value::Object(map)
}

fn escape_inline_backticks(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut rest = raw;
    while let Some(start) = rest.find('`') {
        out.push_str(&escape_html(&rest[..start]));
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            out.push_str("<code>");
            out.push_str(&escape_html(&after[..end]));
            out.push_str("</code>");
            rest = &after[end + 1..];
        } else {
            out.push('`');
            rest = after;
        }
    }
    out.push_str(&escape_html(rest));
    out
}

fn build_cli_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let mut toc = Vec::new();
    let mut body = String::with_capacity(16_384);

    let _ = writeln!(
        body,
        "<p class=\"lead\">Typed command and flag catalog shared between <code>curvebook-cli --help</code> and this manual page (<strong>{}</strong> subcommands, <strong>{}</strong> exit codes).</p>",
        cli_catalog::CLI_COMMANDS.len(),
        cli_catalog::CLI_EXIT_CODES.len(),
    );
    let _ =
        writeln!(body, "<p>See <a href=\"../automation/cli.html\">CLI Reference</a> for direct command flag mapping rules and error envelopes.</p>");

    push_heading(&mut toc, &mut body, 2, "cli-global-flags", "Global Options");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Flags</th><th>Default</th><th>Description</th></tr></thead>\n<tbody>\n");
    for opt in cli_catalog::CLI_GLOBAL_FLAGS {
        let label = match opt.short {
            Some(short) => format!("{short}, {}", opt.long),
            None => opt.long.to_owned(),
        };
        let _ = writeln!(
            body,
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            escape_html(&label),
            escape_inline_backticks(opt.default_value),
            escape_inline_backticks(opt.description),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "cli-subcommands", "Subcommands");
    for cmd in cli_catalog::CLI_COMMANDS {
        let anchor = format!("cli-{}", cmd.id);
        expected.cli_commands.push(anchor.clone());
        az.push(AzEntry {
            term: format!("curvebook-cli {}", cmd.id),
            category: String::from("CLI Subcommand"),
            rel_href: format!("registry/cli.html#{anchor}"),
        });
        let _ = writeln!(body, "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">");
        let _ = writeln!(body, "  <h3><code>{}</code></h3>", escape_html(cmd.id));
        let _ = writeln!(body, "  <p>{}</p>", escape_inline_backticks(cmd.summary));
        let _ = writeln!(body, "  <pre><code class=\"language-sh\">curvebook-cli {}</code></pre>", escape_html(cmd.usage_line));
        if !cmd.positionals.is_empty() {
            body.push_str(
                "<div class=\"table-wrap\"><table>\n<thead><tr><th>Argument</th><th>Required</th><th>Description</th></tr></thead>\n<tbody>\n",
            );
            for pos in cmd.positionals {
                let req = if pos.required { "required" } else { "optional" };
                let _ = writeln!(
                    body,
                    "<tr><td><code>{}</code></td><td>{req}</td><td>{}</td></tr>",
                    escape_html(pos.syntax),
                    escape_inline_backticks(pos.description),
                );
            }
            body.push_str("</tbody>\n</table></div>\n");
        }
        if !cmd.flags.is_empty() {
            body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Flag</th><th>Default</th><th>Description</th></tr></thead>\n<tbody>\n");
            for flag in cmd.flags {
                let mut label = match flag.short {
                    Some(short) => format!("{short}, {}", flag.long),
                    None => flag.long.to_owned(),
                };
                if let Some(ph) = flag.value_placeholder {
                    label.push(' ');
                    label.push_str(ph);
                }
                let _ = writeln!(
                    body,
                    "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                    escape_html(&label),
                    escape_inline_backticks(flag.default_value),
                    escape_inline_backticks(flag.description),
                );
            }
            body.push_str("</tbody>\n</table></div>\n");
        }
        let _ = writeln!(body, "</div>");
    }

    push_heading(&mut toc, &mut body, 2, "cli-exit-codes", "Exit Codes");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Code</th><th>Constant</th><th>Condition</th></tr></thead>\n<tbody>\n");
    for ec in cli_catalog::CLI_EXIT_CODES {
        let _ = writeln!(
            body,
            "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>",
            ec.code,
            escape_html(ec.name),
            escape_inline_backticks(ec.meaning),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    RegistryPageSpec {
        title: String::from("CLI Subcommands & Flags Registry"),
        description: String::from("Generated subcommand usage, flags, defaults, and exit codes for curvebook-cli."),
        rel_path: String::from("registry/cli.html"),
        section_title: "Automation",
        section_rel_path: "section-automation.html",
        h1: String::from("CLI Subcommands & Flags Registry"),
        toc_headings: toc,
        body_html: body,
    }
}

fn equation_category_label(cat: EquationCategory) -> (&'static str, &'static str) {
    match cat {
        EquationCategory::DoseResponseStimulation => ("cat-dose-response-stimulation", "Dose-Response — Stimulation"),
        EquationCategory::DoseResponseInhibition => ("cat-dose-response-inhibition", "Dose-Response — Inhibition"),
        EquationCategory::ReceptorBinding => ("cat-receptor-binding", "Receptor Binding"),
        EquationCategory::EnzymeKinetics => ("cat-enzyme-kinetics", "Enzyme Kinetics"),
        EquationCategory::Exponential => ("cat-exponential", "Exponential Decay & Association"),
        EquationCategory::Polynomial => ("cat-polynomial", "Polynomials"),
        EquationCategory::Gaussian => ("cat-gaussian", "Gaussian Distributions"),
        EquationCategory::SineWave => ("cat-sine-wave", "Sine & Damped Waves"),
        EquationCategory::Growth => ("cat-growth", "Growth Models"),
        EquationCategory::Peak => ("cat-peak", "Peak & Chromatographic Shapes"),
        EquationCategory::StandardCurve => ("cat-standard-curve", "Standard Curves & Power Laws"),
        EquationCategory::Radiobiology => ("cat-radiobiology", "Radiobiology"),
        EquationCategory::NistBenchmark => ("cat-nist-benchmark", "NIST StRD Regression Benchmarks"),
        EquationCategory::UserDefined => ("cat-user-defined", "User-Defined Equation DSL"),
    }
}

fn build_equations_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let entries = list_builtin_equations();
    let sample_pts = vec![
        DataPoint::xy(0.1, 1.0),
        DataPoint::xy(1.0, 2.5),
        DataPoint::xy(10.0, 6.0),
        DataPoint::xy(100.0, 9.0),
        DataPoint::xy(1000.0, 9.8),
        DataPoint::xy(10_000.0, 10.0),
    ];

    let mut toc = Vec::new();
    let mut body = String::with_capacity(49_152);
    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete catalog of <strong>{}</strong> built-in parametric models compiled from <code>curvebook_fit::list_builtin_equations()</code>, together with fitting methods, weighting schemes, and solver defaults.</p>",
        entries.len(),
    );
    let _ = writeln!(
        body,
        "<p>See <a href=\"../fitting.html\">Curve Fitting Reference</a> for statistical theory, custom DSL equations, ODE models, and outlier handling.</p>"
    );

    let ordered_cats = [
        EquationCategory::DoseResponseStimulation,
        EquationCategory::DoseResponseInhibition,
        EquationCategory::ReceptorBinding,
        EquationCategory::EnzymeKinetics,
        EquationCategory::Exponential,
        EquationCategory::Polynomial,
        EquationCategory::Gaussian,
        EquationCategory::SineWave,
        EquationCategory::Growth,
        EquationCategory::Peak,
        EquationCategory::StandardCurve,
        EquationCategory::Radiobiology,
        EquationCategory::NistBenchmark,
    ];

    for cat in ordered_cats {
        let cat_items: Vec<_> = entries.iter().filter(|e| e.category == cat).collect();
        if cat_items.is_empty() {
            continue;
        }
        let (cat_id, cat_title) = equation_category_label(cat);
        let heading = format!("{cat_title} ({})", cat_items.len());
        push_heading(&mut toc, &mut body, 2, cat_id, &heading);

        body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Equation ID</th><th>Name</th><th>Formula (TeX)</th><th>Parameters &amp; Default Bounds</th><th>Analytical Gradient &amp; Initial Guess</th></tr></thead>\n<tbody>\n");
        for entry in cat_items {
            let anchor = format!("eq-{}", entry.id);
            expected.fit_equations.push(anchor.clone());
            az.push(AzEntry {
                term: entry.name.clone(),
                category: format!("Fit Equation ({cat_title})"),
                rel_href: format!("registry/equations.html#{anchor}"),
            });
            az.push(AzEntry {
                term: entry.id.clone(),
                category: format!("Fit Equation ID ({cat_title})"),
                rel_href: format!("registry/equations.html#{anchor}"),
            });

            let tex = format_model_equation_tex(&entry.id, None, &[], false, 4);
            let mut param_summary = String::new();
            let mut grad_initial = String::from("central finite differences");

            if let Some(model) = get_builtin_model(&entry.id) {
                let bounds = model.default_bounds();
                let guesses = model.initial_guess(&sample_pts);
                let mut grad_out = vec![0.0; model.param_count()];
                let has_grad = model.grad(&[1.0, 1.0, 1.0, 1.0], &guesses, 0, &mut grad_out);
                let init_ok = guesses.len() == model.param_count() && guesses.iter().all(|v| v.is_finite());
                grad_initial = format!(
                    "{}; initial heuristic: {}",
                    if has_grad { "analytical Jacobian" } else { "central finite differences" },
                    if init_ok { "finite sample estimate" } else { "domain-dependent estimate" },
                );

                for (idx, pname) in model.param_names().iter().enumerate() {
                    if idx > 0 {
                        param_summary.push_str(", ");
                    }
                    let bound = bounds.get(idx).copied().unwrap_or((None, None));
                    match bound {
                        (None, None) => param_summary.push_str(pname),
                        (Some(lo), None) => {
                            let _ = write!(param_summary, "{pname} (>= {lo})");
                        }
                        (None, Some(hi)) => {
                            let _ = write!(param_summary, "{pname} (<= {hi})");
                        }
                        (Some(lo), Some(hi)) => {
                            let _ = write!(param_summary, "{pname} ([{lo}, {hi}])");
                        }
                    }
                }
            } else {
                param_summary = entry.param_names.join(", ");
            }

            let _ = writeln!(
                body,
                "<tr id=\"{anchor}\" tabindex=\"-1\"><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                escape_html(&entry.id),
                escape_html(&entry.name),
                escape_html(&tex),
                escape_html(&param_summary),
                escape_html(&grad_initial),
            );
        }
        body.push_str("</tbody>\n</table></div>\n");
    }

    push_heading(&mut toc, &mut body, 2, "fitting-methods", "Fitting Methods, Weighting & Solver Defaults");
    let cfg = curvebook_fit::SolverConfig::default();
    let _ = writeln!(
        body,
        "<p>Default <code>SolverConfig</code>: method <code>least_squares</code>, weighting <code>none</code>, confidence <code>95%</code>, max iterations <code>{}</code>, profile CIs enabled ({}), Hougaard skewness enabled ({}), confidence bands enabled ({}), simplex warm start enabled ({}), curve overlay segments <code>{}</code>.</p>",
        cfg.max_iters, cfg.compute_profile_ci, cfg.compute_hougaard, cfg.compute_bands, cfg.use_simplex_warm_start, cfg.curve_overlay_segments,
    );

    let methods = [
        (FitMethod::LeastSquares, "least_squares", "Trust-region Levenberg–Marquardt sum-of-squares minimization."),
        (FitMethod::PoissonDeviance, "poisson_deviance", "Poisson log-likelihood deviance IRLS for non-negative count responses."),
        (FitMethod::RobustCauchy, "robust_cauchy", "Cauchy/Lorentzian M-estimator IRLS minimizing outlier leverage without deletion."),
        (
            FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            "rout_outlier_elimination",
            "Two-stage Cauchy robust fit + Benjamini–Hochberg FDR outlier masking followed by OLS/weighted refit.",
        ),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>FitMethod</th><th>Key</th><th>Description</th></tr></thead>\n<tbody>\n");
    for (_, key, desc) in methods {
        let _ = writeln!(body, "<tr><td><code>{key}</code></td><td><code>FitMethod::{key}</code></td><td>{}</td></tr>", escape_html(desc));
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "weighting-schemes", "Weighting Schemes & Parameter Scopes");
    let weights = [
        (WeightingScheme::None, "none", "Uniform weight w = 1 (homoscedastic constant-SD assumption)."),
        (WeightingScheme::RelativeSquare, "relative_square", "1 / Y^2 weighting (constant %CV proportional scatter)."),
        (WeightingScheme::Relative, "relative", "1 / |Y| weighting (Poisson-like variance proportional to mean)."),
        (WeightingScheme::GeneralPowerY(2.0), "general_power_y", "General power-of-Y weighting 1 / |Y|^k."),
        (WeightingScheme::InverseX, "inverse_x", "1 / |X| inverse predictor weighting."),
        (WeightingScheme::InverseX2, "inverse_x2", "1 / X^2 inverse squared predictor weighting."),
        (WeightingScheme::ObservedSd, "observed_sd", "Empirical replicate variance weighting 1 / SD_i^2."),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>WeightingScheme</th><th>Formula / Rule</th></tr></thead>\n<tbody>\n");
    for (_, key, desc) in weights {
        let _ = writeln!(body, "<tr><td><code>{key}</code></td><td>{}</td></tr>", escape_html(desc));
    }
    body.push_str("</tbody>\n</table></div>\n");

    let scopes = [
        (ParameterScope::Local, "local", "Estimate independently per dataset."),
        (ParameterScope::SharedGlobal, "shared_global", "Jointly optimize a single shared value across all datasets."),
        (ParameterScope::Constant(0.0), "constant", "Fix to a constant scalar value (0 free degrees of freedom)."),
        (
            ParameterScope::HookedConstant(curvebook_doc::HookSource::InfoConstant {
                info_sheet_id: Some(curvebook_doc::SheetId::new("info-1")),
                constant_name: String::from("K"),
            }),
            "hooked_constant",
            "Fix dynamically to an upstream InfoSheet or ResultSheet scalar hook.",
        ),
        (ParameterScope::PerDatasetConstant(Vec::new()), "per_dataset_constant", "Fix per-dataset constants while leaving unlisted datasets free."),
        (ParameterScope::ColumnTitleConstant, "column_title_constant", "Fix per dataset from the numeric token in each column header."),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>ParameterScope</th><th>Behavior</th></tr></thead>\n<tbody>\n");
    for (_, key, desc) in scopes {
        let _ = writeln!(body, "<tr><td><code>{key}</code></td><td>{}</td></tr>", escape_html(desc));
    }
    body.push_str("</tbody>\n</table></div>\n");

    RegistryPageSpec {
        title: String::from("Parametric Equations & Fitting Registry"),
        description: String::from("Complete built-in curve fitting equations, formulas, parameter bounds, fitting methods, and weighting schemes."),
        rel_path: String::from("registry/equations.html"),
        section_title: "Curve fitting",
        section_rel_path: "section-fitting.html",
        h1: String::from("Parametric Equations & Fitting Registry"),
        toc_headings: toc,
        body_html: body,
    }
}

fn table_kind_slug(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Xy => "xy",
        TableKind::Column => "column",
        TableKind::Grouped => "grouped",
        TableKind::Contingency => "contingency",
        TableKind::Survival => "survival",
        TableKind::PartsOfWhole => "parts_of_whole",
        TableKind::MultipleVariables => "multiple_variables",
        TableKind::Nested => "nested",
    }
}

fn build_tables_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let mut toc = Vec::new();
    let mut body = String::with_capacity(24_576);
    let analysis_meta = all_analysis_kinds_meta();

    let _ = writeln!(
        body,
        "<p class=\"lead\">Typed definitions from <code>curvebook-doc</code> and <code>curvebook-stats</code>: <strong>{}</strong> table archetypes, <strong>{}</strong> subcolumn modes, <strong>{}</strong> analysis kinds, <strong>{}</strong> info keys, and <strong>{}</strong> canonical engine error codes.</p>",
        TableKind::ALL.len(),
        8,
        analysis_meta.len(),
        STANDARD_INFO_KEYS.len(),
        ErrorCode::ALL.len(),
    );

    push_heading(&mut toc, &mut body, 2, "table-archetypes", "Table Archetypes (TableKind)");
    let submodes = [
        (SubcolumnMode::Single, "single", "Single Y subcolumn per dataset"),
        (SubcolumnMode::RawReplicates(3), "raw_replicates", "N raw replicate subcolumns (Y1..Yn)"),
        (SubcolumnMode::MeanSdN, "mean_sd_n", "Pre-summarized Mean, SD, N"),
        (SubcolumnMode::MeanSemN, "mean_sem_n", "Pre-summarized Mean, SEM, N"),
        (SubcolumnMode::MeanCvN, "mean_cv_n", "Pre-summarized Mean, %CV, N"),
        (SubcolumnMode::MeanLowHigh, "mean_low_high", "Pre-summarized Mean, -Error, +Error"),
        (SubcolumnMode::UpperLowerLimits, "upper_lower_limits", "Pre-summarized Mean, LowerLimit, UpperLimit"),
        (SubcolumnMode::MedianIqr, "median_iqr", "Pre-summarized Median, Q1, Q3"),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Kind ID</th><th>Display Label</th><th>Supported Subcolumn Modes</th></tr></thead>\n<tbody>\n");
    for kind in TableKind::ALL {
        let slug = table_kind_slug(kind);
        let anchor = format!("table-{slug}");
        expected.table_kinds.push(anchor.clone());
        az.push(AzEntry {
            term: format!("{} table", kind.label()),
            category: String::from("Table Archetype"),
            rel_href: format!("registry/tables.html#{anchor}"),
        });
        let allowed: Vec<&str> = submodes.iter().filter(|(m, _, _)| kind.allows_subcolumn_mode(*m)).map(|(_, id, _)| *id).collect();
        let _ = writeln!(
            body,
            "<tr id=\"{anchor}\" tabindex=\"-1\"><td><code>{slug}</code></td><td>{}</td><td>{}</td></tr>",
            escape_html(kind.label()),
            escape_html(&allowed.join(", ")),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "subcolumn-modes", "Subcolumn Modes (SubcolumnMode)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Mode</th><th>Subcolumns</th><th>Pre-summarized</th><th>Parametric Moments</th><th>Headers</th></tr></thead>\n<tbody>\n");
    for (mode, id, _desc) in submodes {
        let _ = writeln!(
            body,
            "<tr id=\"subcolumn-{id}\" tabindex=\"-1\"><td><code>{id}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            mode.subcolumn_count(),
            mode.is_pre_summarized(),
            mode.supports_parametric_moments(),
            escape_html(&mode.subcolumn_labels().join(", ")),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "analysis-kinds", "Analysis Kinds (AnalysisKind)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Analysis Kind</th><th>Methods Page Link</th><th>Scope Summary</th></tr></thead>\n<tbody>\n");
    for (slug, title, summary) in analysis_meta {
        let reg_anchor = format!("analysis-kind-{slug}");
        let doc_anchor = format!("analysis-{slug}");
        expected.analysis_kinds.push(doc_anchor.clone());
        az.push(AzEntry { term: (*title).to_owned(), category: String::from("Analysis Kind"), rel_href: format!("analyses.html#{doc_anchor}") });
        let _ = writeln!(
            body,
            "<tr id=\"{reg_anchor}\" tabindex=\"-1\"><td><code>{slug}</code></td><td><a href=\"../analyses.html#{doc_anchor}\">{}</a></td><td>{}</td></tr>",
            escape_html(title),
            escape_html(summary),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "analysis-catalog", "Interactive Analysis Catalog Entries");
    body.push_str(
        "<div class=\"table-wrap\"><table>\n<thead><tr><th>Catalog ID</th><th>Label</th><th>Category</th><th>Summary</th></tr></thead>\n<tbody>\n",
    );
    for item in ANALYSIS_CATALOG {
        let cat_label = item.category.label();
        let _ = writeln!(
            body,
            "<tr><td><code>{}</code></td><td>{}</td><td>{cat_label}</td><td>{}</td></tr>",
            escape_html(item.id),
            escape_html(item.label),
            escape_html(item.summary),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "power-families", "Statistical Power Test Families");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Family</th><th>Default Standardized Effect Size</th></tr></thead>\n<tbody>\n");
    for fam in PowerTestFamily::ALL {
        let _ = writeln!(body, "<tr><td>{}</td><td><code>{}</code></td></tr>", escape_html(fam.label()), escape_html(fam.default_effect_label()),);
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "info-keys", "Standard Info Sheet Keys");
    body.push_str("<ul>\n");
    for key in STANDARD_INFO_KEYS {
        let _ = writeln!(body, "  <li><code>{}</code></li>", escape_html(key));
    }
    body.push_str("</ul>\n");

    push_heading(&mut toc, &mut body, 2, "error-codes", "Engine Error Codes (ErrorCode)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Code</th><th>Category</th></tr></thead>\n<tbody>\n");
    for code in ErrorCode::ALL {
        let code_id = format!("error-{}", code.as_str());
        az.push(AzEntry {
            term: code.as_str().to_owned(),
            category: format!("ErrorCode ({})", code.category().as_str()),
            rel_href: format!("registry/tables.html#{code_id}"),
        });
        let _ = writeln!(
            body,
            "<tr id=\"{code_id}\" tabindex=\"-1\"><td><code>{}</code></td><td><code>{}</code></td></tr>",
            escape_html(code.as_str()),
            escape_html(code.category().as_str()),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    RegistryPageSpec {
        title: String::from("Tables, Analyses & Error Codes Registry"),
        description: String::from("Registry of CurveBook table archetypes, subcolumn modes, analysis kinds, info keys, and error codes."),
        rel_path: String::from("registry/tables.html"),
        section_title: "Data tables & formulas",
        section_rel_path: "section-tables.html",
        h1: String::from("Tables, Analyses & Error Codes Registry"),
        toc_headings: toc,
        body_html: body,
    }
}

fn graph_kind_entries() -> Vec<(&'static str, &'static str, &'static str)> {
    // Exhaustive compile-time guard for every `GraphKind` and `DiagnosticPlotKind`.
    let kinds = [
        GraphKind::Xy,
        GraphKind::Column,
        GraphKind::Grouped,
        GraphKind::Contingency,
        GraphKind::PartsOfWhole,
        GraphKind::Survival,
        GraphKind::MultipleVariables,
        GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
        GraphKind::Diagnostic(DiagnosticPlotKind::Volcano),
        GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman),
        GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve),
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot),
    ];
    kinds
        .into_iter()
        .map(|k| match k {
            GraphKind::Xy => {
                ("xy", "XY Scatter / Line / Fitted Curve", "Continuous X vs Y scatter, connected line, spline, area, and fitted curves.")
            }
            GraphKind::Column => (
                "column",
                "Column Bar / Box / Violin / Dot",
                "One-way categorical comparisons with bar, box, violin, beeswarm, or paired before-after lines.",
            ),
            GraphKind::Grouped => {
                ("grouped", "Grouped Multi-Factor Figure", "Clustered, stacked, or superimposed two-factor/three-factor bar and point figures.")
            }
            GraphKind::Contingency => {
                ("contingency", "Contingency Bar / Stacked Proportions", "Categorical count or percentage bars from contingency tables.")
            }
            GraphKind::PartsOfWhole => {
                ("parts-of-whole", "Parts of Whole (Pie / Donut / Waffle)", "Pie, donut, stacked fraction bar, or 10x10 waffle chart.")
            }
            GraphKind::Survival => {
                ("survival", "Kaplan–Meier Survival Curve", "Staircase survival trajectories with censoring ticks, CI bands, and risk tables.")
            }
            GraphKind::MultipleVariables => (
                "multiple-variables",
                "Multiple Variables Bubble / Matrix",
                "Multivariate bubble scatter mapping continuous/categorical axes, colour, and size.",
            ),
            GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap) => {
                ("diagnostic-heatmap", "Diagnostic: Heatmap", "Color-mapped 2D matrix of correlations, expression values, or grouped means.")
            }
            GraphKind::Diagnostic(DiagnosticPlotKind::Volcano) => {
                ("diagnostic-volcano", "Diagnostic: Volcano Plot", "Effect fold-change or difference vs -log10(p) / -log10(q) significance.")
            }
            GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman) => (
                "diagnostic-bland-altman",
                "Diagnostic: Bland–Altman Plot",
                "Difference/ratio vs average method-comparison plot with bias and 95% LoA.",
            ),
            GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve) => {
                ("diagnostic-roc-curve", "Diagnostic: ROC Curve", "Sensitivity vs 1 - Specificity receiver operating characteristic curve.")
            }
            GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot) => {
                ("diagnostic-forest-plot", "Diagnostic: Forest Plot", "Horizontal confidence-interval effect-size, odds-ratio, or hazard-ratio plot.")
            }
            GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot) => {
                ("diagnostic-residual-plot", "Diagnostic: Residual Plot", "Regression residuals vs predicted Y or X scatter diagnostic.")
            }
            GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot) => {
                ("diagnostic-normal-qq-plot", "Diagnostic: Normal Q–Q Plot", "Theoretical normal quantiles vs observed/residual quantiles.")
            }
        })
        .collect()
}

fn build_graphs_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let mut toc = Vec::new();
    let mut body = String::with_capacity(20_480);
    let kinds = graph_kind_entries();

    let _ = writeln!(
        body,
        "<p class=\"lead\">Registry of <strong>{}</strong> primary and diagnostic graph kinds, <strong>{}</strong> graph preset options, axis scales, frame presets, qualitative colour palettes, and continuous colormaps.</p>",
        kinds.len(),
        GRAPH_TYPE_OPTIONS.len(),
    );

    push_heading(&mut toc, &mut body, 2, "graph-kinds", "Graph Kinds & Diagnostic Plots");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Slug</th><th>Graph Kind</th><th>Description</th></tr></thead>\n<tbody>\n");
    for (slug, label, desc) in kinds {
        let anchor = format!("graph-{slug}");
        expected.graph_kinds.push(anchor.clone());
        az.push(AzEntry { term: (*label).to_owned(), category: String::from("Graph Kind"), rel_href: format!("registry/graphs.html#{anchor}") });
        let _ = writeln!(
            body,
            "<tr id=\"{anchor}\" tabindex=\"-1\"><td><code>{slug}</code></td><td>{}</td><td>{}</td></tr>",
            escape_html(label),
            escape_html(desc),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "graph-type-options", "Graph Type Chooser Options (GRAPH_TYPE_OPTIONS)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Option ID</th><th>Label</th><th>Compatible Tables</th><th>Summary</th></tr></thead>\n<tbody>\n");
    for opt in GRAPH_TYPE_OPTIONS {
        let anchor = format!("graph-option-{}", kebab_id(opt.id));
        expected.graph_options.push(anchor.clone());
        az.push(AzEntry {
            term: opt.label.to_owned(),
            category: String::from("Graph Style Option"),
            rel_href: format!("registry/graphs.html#{anchor}"),
        });
        let tables: Vec<&str> = opt.compatible_tables.iter().map(|t| t.label()).collect();
        let _ = writeln!(
            body,
            "<tr id=\"{anchor}\" tabindex=\"-1\"><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(opt.id),
            escape_html(opt.label),
            escape_html(&tables.join(", ")),
            escape_html(opt.subtitle),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "frame-presets", "Axis Frame Presets (FramePreset)");
    let frames = [
        (FramePreset::OpenL, "open-l", "Open L-shaped bottom X and left Y spines."),
        (FramePreset::OffsetRangeTufte, "offset-range-tufte", "Offset spines trimmed to the plotted data range."),
        (FramePreset::PlainBoxed, "plain-boxed", "Enclosed four-sided frame around the plot area."),
        (FramePreset::DualYSecondaryRight, "dual-y-secondary-right", "Left primary Y axis paired with an independent right Y2 axis."),
        (FramePreset::OriginCrossing, "origin-crossing", "Axes intersecting directly at data coordinate (0, 0)."),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Preset</th><th>Geometry</th></tr></thead>\n<tbody>\n");
    for (_, id, desc) in frames {
        let _ = writeln!(body, "<tr id=\"frame-{id}\" tabindex=\"-1\"><td><code>{id}</code></td><td>{}</td></tr>", escape_html(desc));
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "axis-scales", "Axis Scale Bijections (AxisScale)");
    let scales = [
        (AxisScale::Linear, "linear", "Identity g(u) = u over finite reals."),
        (AxisScale::Log10, "log10", "Base-10 logarithm over u > 0."),
        (AxisScale::Log2, "log2", "Base-2 logarithm over u > 0."),
        (AxisScale::Ln, "ln", "Natural logarithm over u > 0."),
        (AxisScale::Logit(LogitDomain::Percent), "logit-percent", "Logit ln(p / (1 - p)) over u in (0, 100)."),
        (AxisScale::Logit(LogitDomain::Proportion), "logit-proportion", "Logit ln(p / (1 - p)) over u in (0, 1)."),
        (AxisScale::Probit, "probit", "Normal probability quantile transform over u in (0, 100)."),
        (AxisScale::Reciprocal, "reciprocal", "Reciprocal 1 / u over nonzero finite u."),
    ];
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Scale</th><th>Domain &amp; Transform</th></tr></thead>\n<tbody>\n");
    for (_, id, desc) in scales {
        let _ = writeln!(body, "<tr id=\"scale-{id}\" tabindex=\"-1\"><td><code>{id}</code></td><td>{}</td></tr>", escape_html(desc));
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "palettes", "Qualitative Colour Palettes (PaletteId)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Palette</th><th>Swatches (Hex)</th></tr></thead>\n<tbody>\n");
    for pal in PaletteId::ALL {
        let hexes: Vec<String> = pal.swatches().iter().map(|c| format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)).collect();
        let id = format!("palette-{}", kebab_id(pal.label()));
        let _ = writeln!(
            body,
            "<tr id=\"{id}\" tabindex=\"-1\"><td>{}</td><td><code>{}</code></td></tr>",
            escape_html(pal.label()),
            escape_html(&hexes.join(" ")),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    push_heading(&mut toc, &mut body, 2, "colormaps", "Continuous Perceptual Colormaps (ColormapId)");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Colormap</th><th>Control Stops</th></tr></thead>\n<tbody>\n");
    for cm in ColormapId::ALL {
        let id = format!("colormap-{}", kebab_id(cm.label()));
        let _ = writeln!(
            body,
            "<tr id=\"{id}\" tabindex=\"-1\"><td>{}</td><td>{} uniform Oklab stops</td></tr>",
            escape_html(cm.label()),
            cm.stops().len(),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    RegistryPageSpec {
        title: String::from("Graphs, Scales & Palettes Registry"),
        description: String::from("Registry of CurveBook graph kinds, style presets, axis scales, qualitative palettes, and colormaps."),
        rel_path: String::from("registry/graphs.html"),
        section_title: "Graphs & Page Layouts",
        section_rel_path: "section-graphs.html",
        h1: String::from("Graphs, Scales & Palettes Registry"),
        toc_headings: toc,
        body_html: body,
    }
}

fn format_kind_label(kind: FormatKind) -> &'static str {
    match kind {
        FormatKind::Archive => "Project Archive",
        FormatKind::Template => "Project Template",
        FormatKind::ProjectDir => "Unpacked Project Directory",
        FormatKind::DelimitedText => "Delimited Text",
        FormatKind::ForeignProject => "External Project Interchange",
        FormatKind::ResultTable => "Structured Result Export",
        FormatKind::FigureVector => "Vector Publication Figure",
        FormatKind::FigureRaster => "Raster Publication Figure",
    }
}

fn build_formats_page(root: &Path, az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> Result<RegistryPageSpec, String> {
    let reg = FormatRegistry::default_registry();
    let imports: Vec<FormatDescriptor> = reg.supported_import_formats();
    let exports: Vec<FormatDescriptor> = reg.supported_export_formats();

    let mut import_ids: BTreeSet<&str> = BTreeSet::new();
    let mut export_ids: BTreeSet<&str> = BTreeSet::new();
    let mut merged: Vec<FormatDescriptor> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for desc in imports {
        import_ids.insert(desc.id);
        if seen.insert(desc.id) {
            merged.push(desc);
        }
    }
    for desc in exports {
        export_ids.insert(desc.id);
        if seen.insert(desc.id) {
            merged.push(desc);
        }
    }

    let mut toc = Vec::new();
    let mut body = String::with_capacity(24_576);
    let _ = writeln!(
        body,
        "<p class=\"lead\">Import/export format descriptors compiled from <code>FormatRegistry::default_registry()</code> (<strong>{}</strong> formats) alongside persisted JSON Schemas and supported UI locales.</p>",
        merged.len(),
    );

    push_heading(&mut toc, &mut body, 2, "formats", "Registered Import & Export Formats");
    body.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>Format ID</th><th>Label</th><th>Extensions</th><th>Category</th><th>Import</th><th>Export</th></tr></thead>\n<tbody>\n");
    for desc in &merged {
        let anchor = format!("format-{}", kebab_id(desc.id));
        expected.formats.push(anchor.clone());
        az.push(AzEntry { term: desc.label.to_owned(), category: String::from("File Format"), rel_href: format!("registry/formats.html#{anchor}") });
        let exts: Vec<String> = desc.extensions.iter().map(|e| format!(".{e}")).collect();
        let can_in = if import_ids.contains(desc.id) { "yes" } else { "no" };
        let can_out = if export_ids.contains(desc.id) { "yes" } else { "no" };
        let _ = writeln!(
            body,
            "<tr id=\"{anchor}\" tabindex=\"-1\"><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{can_in}</td><td>{can_out}</td></tr>",
            escape_html(desc.id),
            escape_html(desc.label),
            escape_html(&exts.join(", ")),
            escape_html(format_kind_label(desc.kind)),
        );
    }
    body.push_str("</tbody>\n</table></div>\n");

    let schema_files = [
        ("preferences.schema.json", "schema-preferences", "Preferences File Schema", Some("pref-")),
        ("batch-v1.schema.json", "schema-batch-v1", "Batch Manifest Schema (curvebook.batch.v1)", None),
        ("document.schema.json", "schema-document", "Project Document Schema", None),
        ("archive-manifest.schema.json", "schema-archive-manifest", "Archive Manifest Schema", None),
        ("journal-record.schema.json", "schema-journal-record", "Autosave Journal Record Schema", None),
    ];

    for (file_name, anchor, title, key_prefix) in schema_files {
        let raw = std::fs::read_to_string(root.join("docs/schemas").join(file_name)).map_err(|e| format!("read docs/schemas/{file_name}: {e}"))?;
        let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse docs/schemas/{file_name}: {e}"))?;

        push_heading(&mut toc, &mut body, 2, anchor, title);
        if let Some(desc) = val.get("description").and_then(serde_json::Value::as_str) {
            let _ = writeln!(body, "<p>{}</p>", escape_html(desc));
        }

        if let Some(props) = val.get("properties").and_then(serde_json::Value::as_object) {
            body.push_str(
                "<div class=\"table-wrap\"><table>\n<thead><tr><th>Property Key</th><th>Schema Type / Details</th></tr></thead>\n<tbody>\n",
            );
            for (key, prop_val) in props {
                let row_id = if let Some(prefix) = key_prefix {
                    let pid = format!("{prefix}{}", github_slug(key));
                    if file_name == "preferences.schema.json" {
                        expected.preference_keys.push(pid.clone());
                        az.push(AzEntry {
                            term: format!("preference: {key}"),
                            category: String::from("Setting Key"),
                            rel_href: format!("registry/formats.html#{pid}"),
                        });
                    }
                    format!(" id=\"{}\" tabindex=\"-1\"", escape_html(&pid))
                } else {
                    String::new()
                };
                let compact = serde_json::to_string(prop_val).unwrap_or_else(|_| String::from("{}"));
                let _ = writeln!(body, "<tr{row_id}><td><code>{}</code></td><td><code>{}</code></td></tr>", escape_html(key), escape_html(&compact));
            }
            body.push_str("</tbody>\n</table></div>\n");
        }
        let pretty = serde_json::to_string_pretty(&val).unwrap_or(raw);
        let _ = writeln!(body, "<pre><code class=\"language-json\">{}</code></pre>", escape_html(&pretty));
    }

    push_heading(&mut toc, &mut body, 2, "locales", "Supported Interface Locales (LOCALES)");
    let _ = writeln!(
        body,
        "<p>Registered locale tags compiled in <code>curvebook_ui_egui::i18n::LOCALES</code> (<strong>{}</strong> entries):</p>",
        LOCALES.len()
    );
    body.push_str("<p>");
    for (idx, tag) in LOCALES.iter().enumerate() {
        if idx > 0 {
            body.push_str(", ");
        }
        let _ = write!(body, "<code>{}</code>", escape_html(tag));
    }
    body.push_str("</p>\n");

    Ok(RegistryPageSpec {
        title: String::from("Formats, Schemas & Locales Registry"),
        description: String::from(
            "Registered import/export file formats, JSON Schemas (preferences, batch-v1, document, archive, journal), and supported locales.",
        ),
        rel_path: String::from("registry/formats.html"),
        section_title: "Files, import & export",
        section_rel_path: "section-files.html",
        h1: String::from("Formats, Schemas & Locales Registry"),
        toc_headings: toc,
        body_html: body,
    })
}
