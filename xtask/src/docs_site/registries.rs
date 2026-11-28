//! Typed registry reference generators for the CurveBook static manual (`R12.1`, `R12.4`).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use curvebook_doc::analysis::ALL_ANALYSIS_DESCRIPTORS;
use curvebook_doc::{
    AxisScale, DiagnosticPlotKind, ErrorCode, FitMethod, FramePreset, GraphKind, LogitDomain, ParameterScope, STANDARD_INFO_KEYS, SubcolumnMode,
    TableKind, WeightingScheme,
};
use curvebook_fit::{DataPoint, EquationCategory, format_model_equation_tex, get_builtin_model, list_builtin_equations};
use curvebook_format::registry::{DESC_PLATE_MAP, FormatDescriptor, FormatKind, FormatRegistry};
use curvebook_mcp::{ResourceDescriptor, ToolDefinition, list_gui_tools, list_prompts, list_resources, list_tools, list_tools_for_group};
use curvebook_palette::{ColormapId, PaletteId};
use curvebook_stats::power::PowerTestFamily;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, GRAPH_TYPE_OPTIONS};
use curvebook_ui_egui::i18n::LOCALES;
use curvebook_ui_egui::menus::UI_COMMANDS;

use super::markdown::{TocHeading, escape_html as eh, github_slug, render_math_html};

#[path = "../../../apps/curvebook-cli/src/cli_catalog.rs"]
#[allow(dead_code)]
mod cli_catalog;

#[derive(Debug, Clone)]
pub struct RegistryPageSpec {
    pub title: String,
    pub description: String,
    pub rel_path: String,
    pub h1: String,
    pub toc_headings: Vec<TocHeading>,
    pub body_html: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzEntry {
    pub term: String,
    pub category: String,
    pub rel_href: String,
}

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

pub struct RegistryBundle {
    pub pages: Vec<RegistryPageSpec>,
    pub az_entries: Vec<AzEntry>,
    pub expected: ExpectedCoverage,
}

pub fn generate_registry_pages(root: &Path, defined_analysis_anchors: &BTreeSet<String>) -> Result<RegistryBundle, String> {
    let (mut az, mut expected) = (Vec::new(), ExpectedCoverage::default());
    let pages = vec![
        build_cli_page(&mut az, &mut expected),
        build_commands_page(&mut az, &mut expected),
        build_equations_page(&mut az, &mut expected),
        build_formats_page(root, &mut az, &mut expected)?,
        build_graphs_page(&mut az, &mut expected),
        build_mcp_page(&mut az, &mut expected),
        build_tables_page(defined_analysis_anchors, &mut az, &mut expected),
    ];
    Ok(RegistryBundle { pages, az_entries: az, expected })
}

#[must_use]
pub fn all_analysis_kinds_meta() -> Vec<(&'static str, &'static str, &'static str)> {
    ALL_ANALYSIS_DESCRIPTORS.iter().map(|d| (d.slug, d.title, d.description)).collect()
}

fn kebab_id(raw: &str) -> String {
    let (mut out, mut prev_dash) = (String::with_capacity(raw.len()), false);
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
    let _ = writeln!(out, "<h{level} id=\"{}\" tabindex=\"-1\">{}</h{level}>", eh(id), eh(text));
}

fn write_table(out: &mut String, headers: &[&str], rows: impl FnOnce(&mut String)) {
    out.push_str("<div class=\"table-wrap\"><table>\n<thead><tr>");
    headers.iter().for_each(|h| {
        let _ = write!(out, "<th>{h}</th>");
    });
    out.push_str("</tr></thead>\n<tbody>\n");
    rows(out);
    out.push_str("</tbody>\n</table></div>\n");
}

fn write_tr(out: &mut String, id: Option<&str>, cells_html: &[&str]) {
    match id {
        Some(i) => {
            let _ = write!(out, "<tr id=\"{i}\" tabindex=\"-1\">");
        }
        None => out.push_str("<tr>"),
    }
    for c in cells_html {
        let _ = write!(out, "<td>{c}</td>");
    }
    out.push_str("</tr>\n");
}

fn code(s: &str) -> String {
    format!("<code>{}</code>", eh(s))
}

fn push_az(az: &mut Vec<AzEntry>, term: impl Into<String>, category: impl Into<String>, rel_href: String) {
    az.push(AzEntry { term: term.into(), category: category.into(), rel_href });
}

fn reg_page(title: &str, description: &str, rel_path: &str, toc_headings: Vec<TocHeading>, body_html: String) -> RegistryPageSpec {
    RegistryPageSpec {
        title: title.to_owned(),
        description: description.to_owned(),
        rel_path: rel_path.to_owned(),
        h1: title.to_owned(),
        toc_headings,
        body_html,
    }
}

fn build_commands_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let specs: Vec<_> = curvebook_engine::cmd::registry().specs().collect();
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(32_768));
    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete catalog of <strong>{}</strong> engine commands compiled from <code>curvebook_engine::cmd::registry()</code> and <strong>{}</strong> desktop shell commands from <code>UI_COMMANDS</code>.</p>\n<p>For wire execution syntax across MCP, CLI, and loopback <code>--control</code>, see <a href=\"../automation/mcp.html\">MCP Server Reference</a>, <a href=\"../automation/cli.html\">CLI Reference</a>, and <a href=\"mcp-tools.html\">MCP Tools Registry</a>.</p>",
        specs.len(),
        UI_COMMANDS.len()
    );
    push_heading(&mut toc, &mut body, 2, "engine-commands", "Engine Commands (cmd::registry)");
    for group in curvebook_engine::cmd::CommandGroup::all() {
        let group_specs: Vec<_> = specs.iter().copied().filter(|s| s.group == *group).collect();
        push_heading(&mut toc, &mut body, 3, &format!("group-{}", group.as_str()), &format!("{} ({})", group.title(), group_specs.len()));
        for s in group_specs {
            let anchor = format!("cmd-{}", kebab_id(s.id).replace('_', "-"));
            expected.engine_commands.push(anchor.clone());
            push_az(az, s.id, format!("Command ({})", group.as_str()), format!("registry/commands.html#{anchor}"));
            let (mcp, eff) = (curvebook_mcp::toolsets::command_id_to_mcp_tool_name(s.id), s.effects.flag_names());
            let eff_s = if eff.is_empty() { String::from("none") } else { eff.join(", ") };
            let schema = serde_json::to_string_pretty(&(s.input_schema)()).unwrap_or_else(|_| String::from("{}"));
            let (menu, sc) = (if s.menu_path.is_empty() { "—" } else { s.menu_path }, if s.shortcut.is_empty() { "—" } else { s.shortcut });
            let _ = writeln!(
                body,
                "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">\n  <h4><code>{}</code> — {}</h4>\n  <p>{}</p>\n  <div class=\"badge-row\">\n    <span class=\"badge\">group: {}</span>\n    <span class=\"badge\">effects: {}</span>\n    <span class=\"badge\">menu: {}</span>\n    <span class=\"badge\">shortcut: {}</span>\n  </div>\n  <p>MCP tool: <a href=\"mcp-tools.html#mcp-tool-{}\"><code>{}</code></a> · CLI: <code>curvebook-cli {} [flags]</code></p>\n  <pre><code class=\"language-json\">{}</code></pre>\n</div>",
                eh(s.id),
                eh(s.label),
                eh(s.summary),
                eh(group.as_str()),
                eh(&eff_s),
                eh(menu),
                eh(sc),
                eh(&mcp),
                eh(&mcp),
                eh(s.id),
                eh(&schema)
            );
        }
    }
    push_heading(&mut toc, &mut body, 2, "ui-commands", "Desktop UI Commands (UI_COMMANDS)");
    write_table(&mut body, &["ID", "Label", "Shortcut", "Params", "Summary"], |body| {
        for ui in UI_COMMANDS {
            let anchor = format!("ui-cmd-{}", kebab_id(ui.id).replace('_', "-"));
            expected.ui_commands.push(anchor.clone());
            push_az(az, ui.id, "UI Command", format!("registry/commands.html#{anchor}"));
            let sc = if ui.shortcut.is_empty() { "—" } else { ui.shortcut };
            write_tr(body, Some(&anchor), &[&code(ui.id), &eh(ui.label), &eh(sc), &code(ui.params_doc), &eh(ui.summary)]);
        }
    });
    reg_page(
        "Engine & UI Command Registry",
        "Complete registry of CurveBook engine commands, JSON input schemas, effect masks, and UI commands.",
        "registry/commands.html",
        toc,
        body,
    )
}

fn build_mcp_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(65_536));
    let (core_tools, gui_tools, groups) = (list_tools(), list_gui_tools(), curvebook_engine::cmd::CommandGroup::all());
    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete MCP JSON-RPC 2.0 surface generated from <code>curvebook-mcp</code>: <strong>{}</strong> core tools, <strong>{}</strong> interactive <code>gui</code> tools, <strong>{}</strong> dynamic <code>CommandGroup</code> toolsets, resources, and prompts.</p>\n<p>See <a href=\"../automation/mcp.html\">MCP Server Reference</a> for transport framing and live attachment details.</p>",
        core_tools.len(),
        gui_tools.len(),
        groups.len()
    );
    push_heading(&mut toc, &mut body, 2, "core-tools", "Core MCP Tools");
    core_tools.iter().for_each(|t| render_mcp_tool_card(t, "MCP Core Tool", az, expected, &mut body));
    push_heading(&mut toc, &mut body, 2, "mcp-toolsets", "Dynamic CommandGroup Toolsets");
    for group in groups {
        let group_tools = list_tools_for_group(*group);
        push_heading(
            &mut toc,
            &mut body,
            3,
            &format!("mcp-toolset-{}", group.as_str()),
            &format!("Toolset: {} ({})", group.as_str(), group_tools.len()),
        );
        group_tools.iter().for_each(|t| render_mcp_tool_card(t, &format!("MCP Toolset ({})", group.as_str()), az, expected, &mut body));
    }
    push_heading(&mut toc, &mut body, 2, "gui-toolset", "Interactive GUI Toolset (gui)");
    gui_tools.iter().for_each(|t| render_mcp_tool_card(t, "MCP GUI Tool", az, expected, &mut body));

    push_heading(&mut toc, &mut body, 2, "mcp-resources", "Resource URI Schemes");
    let mut sample_resources: Vec<ResourceDescriptor> = Vec::new();
    if let Ok(mut doc) = curvebook_doc::ProjectDocument::new("Sample") {
        if let Ok(xy) = curvebook_doc::XyTable::new(SubcolumnMode::Single, 1, 2)
            && let Ok(ds) = curvebook_doc::DataSheet::new("data-1", "Data 1", curvebook_doc::TableArchetype::Xy(xy))
        {
            let _ = doc.add_data_sheet(ds);
        }
        sample_resources = list_resources(&doc);
    }
    write_table(&mut body, &["URI Scheme", "MIME Type", "Sample Descriptor"], |body| {
        for r in &sample_resources {
            write_tr(body, None, &[&code(&r.uri), &code(&r.mime_type), &format!("{} — {}", eh(&r.name), eh(&r.description))]);
        }
        body.push_str("<tr><td><code>result://&lt;id&gt;</code></td><td><code>application/json</code></td><td>Serialized computed <code>ResultSheet</code> tabs and diagnostics</td></tr>\n");
    });

    push_heading(&mut toc, &mut body, 2, "mcp-prompts", "Built-in Workflow Prompts");
    for p in list_prompts() {
        let anchor = format!("mcp-prompt-{}", p.name);
        expected.mcp_prompts.push(anchor.clone());
        push_az(az, p.name.clone(), "MCP Prompt", format!("registry/mcp-tools.html#{anchor}"));
        let _ = writeln!(
            body,
            "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">\n  <h4><code>{}</code></h4>\n  <p>{}</p>",
            eh(&p.name),
            eh(&p.description)
        );
        if !p.arguments.is_empty() {
            body.push_str("  <ul>\n");
            for a in &p.arguments {
                let _ = writeln!(
                    body,
                    "    <li><code>{}</code> ({}): {}</li>",
                    eh(&a.name),
                    if a.required { "required" } else { "optional" },
                    eh(&a.description)
                );
            }
            body.push_str("  </ul>\n");
        }
        let req = serde_json::to_string(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/get", "params": {"name": p.name}}))
            .unwrap_or_else(|_| String::from("{}"));
        let _ = writeln!(body, "  <pre><code class=\"language-json\">{}</code></pre>\n</div>", eh(&req));
    }
    reg_page(
        "MCP Tools, Toolsets, Resources & Prompts",
        "Machine-generated schemas, tool annotations, single-call JSON examples, resources, and prompts for CurveBook MCP.",
        "registry/mcp-tools.html",
        toc,
        body,
    )
}

fn render_mcp_tool_card(tool: &ToolDefinition, az_category: &str, az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage, body: &mut String) {
    let anchor = format!("mcp-tool-{}", tool.name);
    expected.mcp_tools.push(anchor.clone());
    push_az(az, tool.name.clone(), az_category, format!("registry/mcp-tools.html#{anchor}"));
    let ann = &tool.annotations;
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool.name, "arguments": minimal_example_args(&tool.input_schema)}});
    let content = if matches!(tool.name.as_str(), "render_preview" | "ui_screenshot") {
        serde_json::json!([{"type": "image", "data": "<base64-png>", "mimeType": "image/png"}, {"type": "text", "text": "{...}"}])
    } else {
        serde_json::json!([{"type": "text", "text": "{...}"}])
    };
    let resp = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"content": content, "isError": false, "structuredContent": {}}});
    let _ = writeln!(
        body,
        "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">\n  <h4><code>{}</code></h4>\n  <p>{}</p>\n  <div class=\"badge-row\">\n    <span class=\"badge\">readOnlyHint: {}</span>\n    <span class=\"badge\">destructiveHint: {}</span>\n    <span class=\"badge\">idempotentHint: {}</span>\n    <span class=\"badge\">openWorldHint: {}</span>\n  </div>\n  <p><strong>Input schema:</strong></p>\n  <pre><code class=\"language-json\">{}</code></pre>\n  <p><strong>Single-call request / response syntax:</strong></p>\n  <pre><code class=\"language-json\">{}</code></pre>\n  <pre><code class=\"language-json\">{}</code></pre>\n</div>",
        eh(&tool.name),
        eh(&tool.description),
        ann.read_only_hint,
        ann.destructive_hint,
        ann.idempotent_hint,
        ann.open_world_hint,
        eh(&serde_json::to_string_pretty(&tool.input_schema).unwrap_or_else(|_| String::from("{}"))),
        eh(&serde_json::to_string(&req).unwrap_or_else(|_| String::from("{}"))),
        eh(&serde_json::to_string(&resp).unwrap_or_else(|_| String::from("{}"))),
    );
}

fn minimal_example_args(schema: &serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let reqs = schema.get("required").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(serde_json::Value::as_str);
    if let Some(props) = schema.get("properties").and_then(serde_json::Value::as_object) {
        for req in reqs {
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
    let (mut out, mut rest) = (String::with_capacity(raw.len() + 16), raw);
    while let Some((before, after)) = rest.split_once('`') {
        out.push_str(&eh(before));
        if let Some((c, tail)) = after.split_once('`') {
            let _ = write!(out, "<code>{}</code>", eh(c));
            rest = tail;
        } else {
            out.push('`');
            rest = after;
        }
    }
    out.push_str(&eh(rest));
    out
}

fn build_cli_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(16_384));
    let _ = writeln!(
        body,
        "<p class=\"lead\">Typed command and flag catalog shared between <code>curvebook-cli --help</code> and this manual page (<strong>{}</strong> subcommands, <strong>{}</strong> exit codes).</p>\n<p>See <a href=\"../automation/cli.html\">CLI Reference</a> for direct command flag mapping rules and error envelopes.</p>",
        cli_catalog::CLI_COMMANDS.len(),
        cli_catalog::CLI_EXIT_CODES.len()
    );
    push_heading(&mut toc, &mut body, 2, "cli-global-flags", "Global Options");
    write_table(&mut body, &["Flags", "Default", "Description"], |body| {
        for opt in cli_catalog::CLI_GLOBAL_FLAGS {
            let label = opt.short.map_or_else(|| opt.long.to_owned(), |s| format!("{s}, {}", opt.long));
            write_tr(body, None, &[&code(&label), &escape_inline_backticks(opt.default_value), &escape_inline_backticks(opt.description)]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "cli-subcommands", "Subcommands");
    for cmd in cli_catalog::CLI_COMMANDS {
        let anchor = format!("cli-{}", cmd.id);
        expected.cli_commands.push(anchor.clone());
        push_az(az, format!("curvebook-cli {}", cmd.id), "CLI Subcommand", format!("registry/cli.html#{anchor}"));
        let _ = writeln!(
            body,
            "<div class=\"entry-card\" id=\"{anchor}\" tabindex=\"-1\">\n  <h3><code>{}</code></h3>\n  <p>{}</p>\n  <pre><code class=\"language-sh\">curvebook-cli {}</code></pre>",
            eh(cmd.id),
            escape_inline_backticks(cmd.summary),
            eh(cmd.usage_line)
        );
        if !cmd.positionals.is_empty() {
            write_table(&mut body, &["Argument", "Required", "Description"], |body| {
                for p in cmd.positionals {
                    write_tr(
                        body,
                        None,
                        &[&code(p.syntax), if p.required { "required" } else { "optional" }, &escape_inline_backticks(p.description)],
                    );
                }
            });
        }
        if !cmd.flags.is_empty() {
            write_table(&mut body, &["Flag", "Default", "Description"], |body| {
                for f in cmd.flags {
                    let mut label = f.short.map_or_else(|| f.long.to_owned(), |s| format!("{s}, {}", f.long));
                    if let Some(ph) = f.value_placeholder {
                        label.push(' ');
                        label.push_str(ph);
                    }
                    write_tr(body, None, &[&code(&label), &escape_inline_backticks(f.default_value), &escape_inline_backticks(f.description)]);
                }
            });
        }
        body.push_str("</div>\n");
    }
    push_heading(&mut toc, &mut body, 2, "cli-exit-codes", "Exit Codes");
    write_table(&mut body, &["Code", "Constant", "Condition"], |body| {
        for ec in cli_catalog::CLI_EXIT_CODES {
            write_tr(body, None, &[&format!("<code>{}</code>", ec.code), &code(ec.name), &escape_inline_backticks(ec.meaning)]);
        }
    });
    reg_page(
        "CLI Subcommands & Flags Registry",
        "Generated subcommand usage, flags, defaults, and exit codes for curvebook-cli.",
        "registry/cli.html",
        toc,
        body,
    )
}

fn build_equations_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    const CATS: &[(EquationCategory, &str, &str)] = &[
        (EquationCategory::DoseResponseStimulation, "cat-dose-response-stimulation", "Dose-Response — Stimulation"),
        (EquationCategory::DoseResponseInhibition, "cat-dose-response-inhibition", "Dose-Response — Inhibition"),
        (EquationCategory::ReceptorBinding, "cat-receptor-binding", "Receptor Binding"),
        (EquationCategory::EnzymeKinetics, "cat-enzyme-kinetics", "Enzyme Kinetics"),
        (EquationCategory::Exponential, "cat-exponential", "Exponential Decay & Association"),
        (EquationCategory::Polynomial, "cat-polynomial", "Polynomials"),
        (EquationCategory::Gaussian, "cat-gaussian", "Gaussian Distributions"),
        (EquationCategory::SineWave, "cat-sine-wave", "Sine & Damped Waves"),
        (EquationCategory::Growth, "cat-growth", "Growth Models"),
        (EquationCategory::Peak, "cat-peak", "Peak & Chromatographic Shapes"),
        (EquationCategory::StandardCurve, "cat-standard-curve", "Standard Curves & Power Laws"),
        (EquationCategory::Radiobiology, "cat-radiobiology", "Radiobiology"),
        (EquationCategory::NistBenchmark, "cat-nist-benchmark", "NIST StRD Regression Benchmarks"),
        (EquationCategory::UserDefined, "cat-user-defined", "User-Defined Equation DSL"),
    ];
    let entries = list_builtin_equations();
    let sample_pts: Vec<_> =
        [(0.1, 1.0), (1.0, 2.5), (10.0, 6.0), (100.0, 9.0), (1000.0, 9.8), (10_000.0, 10.0)].into_iter().map(|(x, y)| DataPoint::xy(x, y)).collect();
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(49_152));
    let _ = writeln!(
        body,
        "<p class=\"lead\">Complete catalog of <strong>{}</strong> built-in parametric models compiled from <code>curvebook_fit::list_builtin_equations()</code>, together with fitting methods, weighting schemes, and solver defaults.</p>\n<p>See <a href=\"../fitting.html\">Curve Fitting Reference</a> for statistical theory, custom DSL equations, ODE models, and outlier handling.</p>",
        entries.len()
    );
    for &(cat, cat_id, cat_title) in CATS {
        let cat_items: Vec<_> = entries.iter().filter(|e| e.category == cat).collect();
        if cat_items.is_empty() {
            continue;
        }
        push_heading(&mut toc, &mut body, 2, cat_id, &format!("{cat_title} ({})", cat_items.len()));
        write_table(
            &mut body,
            &["Equation ID", "Name", "Formula", "TeX", "Parameters &amp; Default Bounds", "Analytical Gradient &amp; Initial Guess"],
            |body| {
                for entry in cat_items {
                    let anchor = format!("eq-{}", entry.id);
                    expected.fit_equations.push(anchor.clone());
                    push_az(az, entry.name.clone(), format!("Fit Equation ({cat_title})"), format!("registry/equations.html#{anchor}"));
                    push_az(az, entry.id.clone(), format!("Fit Equation ID ({cat_title})"), format!("registry/equations.html#{anchor}"));
                    let tex = format_model_equation_tex(&entry.id, None, &[], false, 4);
                    let (param_summary, grad_initial) = if let Some(model) = get_builtin_model(&entry.id) {
                        let (bounds, guesses) = (model.default_bounds(), model.initial_guess(&sample_pts));
                        let has_grad = model.grad(&[1.0, 1.0, 1.0, 1.0], &guesses, 0, &mut vec![0.0; model.param_count()]);
                        let init_ok = guesses.len() == model.param_count() && guesses.iter().all(|v| v.is_finite());
                        let params = model
                            .param_names()
                            .iter()
                            .enumerate()
                            .map(|(i, p)| match bounds.get(i).copied().unwrap_or((None, None)) {
                                (None, None) => (*p).to_owned(),
                                (Some(lo), None) => format!("{p} (>= {})", format_bound_f64(lo)),
                                (None, Some(hi)) => format!("{p} (<= {})", format_bound_f64(hi)),
                                (Some(lo), Some(hi)) => format!("{p} ([{}, {}])", format_bound_f64(lo), format_bound_f64(hi)),
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        (
                            params,
                            format!(
                                "{}; initial heuristic: {}",
                                if has_grad { "analytical Jacobian" } else { "central finite differences" },
                                if init_ok { "finite sample estimate" } else { "domain-dependent estimate" }
                            ),
                        )
                    } else {
                        (entry.param_names.join(", "), String::from("central finite differences"))
                    };
                    write_tr(
                        body,
                        Some(&anchor),
                        &[&code(&entry.id), &eh(&entry.name), &render_math_html(&tex), &code(&tex), &eh(&param_summary), &eh(&grad_initial)],
                    );
                }
            },
        );
    }
    push_heading(&mut toc, &mut body, 2, "fitting-methods", "Fitting Methods, Weighting & Solver Defaults");
    let cfg = curvebook_fit::SolverConfig::default();
    let _ = writeln!(
        body,
        "<p>Default <code>SolverConfig</code>: method <code>least_squares</code>, weighting <code>none</code>, confidence <code>95%</code>, max iterations <code>{}</code>, profile CIs enabled ({}), Hougaard skewness enabled ({}), confidence bands enabled ({}), simplex warm start enabled ({}), curve overlay segments <code>{}</code>.</p>",
        cfg.max_iters, cfg.compute_profile_ci, cfg.compute_hougaard, cfg.compute_bands, cfg.use_simplex_warm_start, cfg.curve_overlay_segments
    );
    write_table(&mut body, &["FitMethod", "Key", "Description"], |body| {
        for m in
            [FitMethod::LeastSquares, FitMethod::PoissonDeviance, FitMethod::RobustCauchy, FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }]
        {
            let (k, d) = match m {
                FitMethod::LeastSquares => ("least_squares", "Trust-region Levenberg–Marquardt sum-of-squares minimization."),
                FitMethod::PoissonDeviance => ("poisson_deviance", "Poisson log-likelihood deviance IRLS for non-negative count responses."),
                FitMethod::RobustCauchy => ("robust_cauchy", "Cauchy/Lorentzian M-estimator IRLS minimizing outlier leverage without deletion."),
                FitMethod::RoutOutlierElimination { .. } => (
                    "rout_outlier_elimination",
                    "Two-stage Cauchy robust fit + Benjamini–Hochberg FDR outlier masking followed by OLS/weighted refit.",
                ),
            };
            let _ = writeln!(body, "<tr><td><code>{k}</code></td><td><code>FitMethod::{k}</code></td><td>{}</td></tr>", eh(d));
        }
    });
    push_heading(&mut toc, &mut body, 2, "weighting-schemes", "Weighting Schemes & Parameter Scopes");
    write_table(&mut body, &["WeightingScheme", "Formula / Rule"], |body| {
        for w in [
            WeightingScheme::None,
            WeightingScheme::RelativeSquare,
            WeightingScheme::Relative,
            WeightingScheme::GeneralPowerY(2.0),
            WeightingScheme::InverseX,
            WeightingScheme::InverseX2,
            WeightingScheme::ObservedSd,
        ] {
            let (k, d) = match w {
                WeightingScheme::None => ("none", "Uniform weight w = 1 (homoscedastic constant-SD assumption)."),
                WeightingScheme::RelativeSquare => ("relative_square", "1 / Y^2 weighting (constant %CV proportional scatter)."),
                WeightingScheme::Relative => ("relative", "1 / |Y| weighting (Poisson-like variance proportional to mean)."),
                WeightingScheme::GeneralPowerY(_) => ("general_power_y", "General power-of-Y weighting 1 / |Y|^k."),
                WeightingScheme::InverseX => ("inverse_x", "1 / |X| inverse predictor weighting."),
                WeightingScheme::InverseX2 => ("inverse_x2", "1 / X^2 inverse squared predictor weighting."),
                WeightingScheme::ObservedSd => ("observed_sd", "Empirical replicate variance weighting 1 / SD_i^2."),
            };
            let _ = writeln!(body, "<tr><td><code>{k}</code></td><td>{}</td></tr>", eh(d));
        }
    });
    write_table(&mut body, &["ParameterScope", "Behavior"], |body| {
        let hook =
            curvebook_doc::HookSource::InfoConstant { info_sheet_id: Some(curvebook_doc::SheetId::new("info-1")), constant_name: String::from("K") };
        for s in [
            ParameterScope::Local,
            ParameterScope::SharedGlobal,
            ParameterScope::Constant(0.0),
            ParameterScope::HookedConstant(hook),
            ParameterScope::PerDatasetConstant(Vec::new()),
            ParameterScope::ColumnTitleConstant,
        ] {
            let (k, d) = match s {
                ParameterScope::Local => ("local", "Estimate independently per dataset."),
                ParameterScope::SharedGlobal => ("shared_global", "Jointly optimize a single shared value across all datasets."),
                ParameterScope::Constant(_) => ("constant", "Fix to a constant scalar value (0 free degrees of freedom)."),
                ParameterScope::HookedConstant(_) => ("hooked_constant", "Fix dynamically to an upstream InfoSheet or ResultSheet scalar hook."),
                ParameterScope::PerDatasetConstant(_) => ("per_dataset_constant", "Fix per-dataset constants while leaving unlisted datasets free."),
                ParameterScope::ColumnTitleConstant => ("column_title_constant", "Fix per dataset from the numeric token in each column header."),
            };
            let _ = writeln!(body, "<tr><td><code>{k}</code></td><td>{}</td></tr>", eh(d));
        }
    });
    reg_page(
        "Parametric Equations & Fitting Registry",
        "Complete built-in curve fitting equations, formulas, parameter bounds, fitting methods, and weighting schemes.",
        "registry/equations.html",
        toc,
        body,
    )
}

fn build_tables_page(defined_analysis_anchors: &BTreeSet<String>, az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let (mut toc, mut body, analysis_meta) = (Vec::new(), String::with_capacity(24_576), all_analysis_kinds_meta());
    let submodes = [
        (SubcolumnMode::Single, "single"),
        (SubcolumnMode::RawReplicates(3), "raw_replicates"),
        (SubcolumnMode::MeanSd, "mean_sd"),
        (SubcolumnMode::MeanSem, "mean_sem"),
        (SubcolumnMode::MeanSdN, "mean_sd_n"),
        (SubcolumnMode::MeanSemN, "mean_sem_n"),
        (SubcolumnMode::MeanCvN, "mean_cv_n"),
        (SubcolumnMode::MeanLowHigh, "mean_low_high"),
        (SubcolumnMode::UpperLowerLimits, "upper_lower_limits"),
        (SubcolumnMode::MedianIqr, "median_iqr"),
    ];
    let _ = writeln!(
        body,
        "<p class=\"lead\">Typed definitions from <code>curvebook-doc</code> and <code>curvebook-stats</code>: <strong>{}</strong> table archetypes, <strong>{}</strong> subcolumn modes, <strong>{}</strong> analysis kinds, <strong>{}</strong> info keys, and <strong>{}</strong> canonical engine error codes.</p>",
        TableKind::ALL.len(),
        submodes.len(),
        analysis_meta.len(),
        STANDARD_INFO_KEYS.len(),
        ErrorCode::ALL.len()
    );
    push_heading(&mut toc, &mut body, 2, "table-archetypes", "Table Archetypes (TableKind)");
    write_table(&mut body, &["Kind ID", "Display Label", "Supported Subcolumn Modes"], |body| {
        for kind in TableKind::ALL {
            let slug = match kind {
                TableKind::Xy => "xy",
                TableKind::Column => "column",
                TableKind::Grouped => "grouped",
                TableKind::Contingency => "contingency",
                TableKind::Survival => "survival",
                TableKind::PartsOfWhole => "parts_of_whole",
                TableKind::MultipleVariables => "multiple_variables",
                TableKind::Nested => "nested",
            };
            let anchor = format!("table-{slug}");
            expected.table_kinds.push(anchor.clone());
            push_az(az, format!("{} table", kind.label()), "Table Archetype", format!("registry/tables.html#{anchor}"));
            let allowed: Vec<&str> = submodes.iter().filter(|(m, _)| kind.allows_subcolumn_mode(*m)).map(|(_, id)| *id).collect();
            write_tr(body, Some(&anchor), &[&format!("<code>{slug}</code>"), &eh(kind.label()), &eh(&allowed.join(", "))]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "subcolumn-modes", "Subcolumn Modes (SubcolumnMode)");
    write_table(&mut body, &["Mode", "Subcolumns", "Pre-summarized", "Parametric Moments", "Headers"], |body| {
        for (mode, id) in submodes {
            let (n, pre, pm) =
                (mode.subcolumn_count().to_string(), mode.is_pre_summarized().to_string(), mode.supports_parametric_moments().to_string());
            write_tr(
                body,
                Some(&format!("subcolumn-{id}")),
                &[&format!("<code>{id}</code>"), &n, &pre, &pm, &eh(&mode.subcolumn_labels().join(", "))],
            );
        }
    });
    push_heading(&mut toc, &mut body, 2, "analysis-kinds", "Analysis Kinds (AnalysisKind)");
    write_table(&mut body, &["Analysis Kind", "Reference Link", "Scope Summary"], |body| {
        for (slug, title, summary) in analysis_meta {
            let (reg_a, doc_a) = (format!("analysis-kind-{slug}"), format!("analysis-{slug}"));
            let has_doc = defined_analysis_anchors.contains(&doc_a);
            expected.analysis_kinds.push((*slug).to_owned());
            push_az(az, title, "Analysis Kind", if has_doc { format!("analyses.html#{doc_a}") } else { format!("registry/tables.html#{reg_a}") });
            let cell_href = if has_doc { format!("../analyses.html#{doc_a}") } else { format!("#{reg_a}") };
            write_tr(body, Some(&reg_a), &[&format!("<code>{slug}</code>"), &format!("<a href=\"{cell_href}\">{}</a>", eh(title)), &eh(summary)]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "analysis-catalog", "Interactive Analysis Catalog Entries");
    write_table(&mut body, &["Catalog ID", "Label", "Category", "Summary"], |body| {
        for item in ANALYSIS_CATALOG {
            write_tr(body, None, &[&code(item.id), &eh(item.label), &eh(item.category.label()), &eh(item.summary)]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "power-families", "Statistical Power Test Families");
    write_table(&mut body, &["Family", "Default Standardized Effect Size"], |body| {
        for fam in PowerTestFamily::ALL {
            write_tr(body, None, &[&eh(fam.label()), &code(fam.default_effect_label())]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "info-keys", "Standard Info Sheet Keys");
    body.push_str("<ul>\n");
    STANDARD_INFO_KEYS.iter().for_each(|k| {
        let _ = writeln!(body, "  <li><code>{}</code></li>", eh(k));
    });
    body.push_str("</ul>\n");
    push_heading(&mut toc, &mut body, 2, "error-codes", "Engine Error Codes (ErrorCode)");
    write_table(&mut body, &["Code", "Category"], |body| {
        for code_v in ErrorCode::ALL {
            let id = format!("error-{}", code_v.as_str());
            push_az(az, code_v.as_str(), format!("ErrorCode ({})", code_v.category().as_str()), format!("registry/tables.html#{id}"));
            write_tr(body, Some(&id), &[&code(code_v.as_str()), &code(code_v.category().as_str())]);
        }
    });
    reg_page(
        "Tables, Analyses & Error Codes Registry",
        "Registry of CurveBook table archetypes, subcolumn modes, analysis kinds, info keys, and error codes.",
        "registry/tables.html",
        toc,
        body,
    )
}

fn build_graphs_page(az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> RegistryPageSpec {
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(20_480));
    let kinds: Vec<(&str, &str, &str)> = [
        GraphKind::Xy,
        GraphKind::Column,
        GraphKind::Grouped,
        GraphKind::Contingency,
        GraphKind::PartsOfWhole,
        GraphKind::Survival,
        GraphKind::MultipleVariables,
        GraphKind::Histogram,
        GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
        GraphKind::Diagnostic(DiagnosticPlotKind::Volcano),
        GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman),
        GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve),
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::HomoscedasticityPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::ActualVsPredictedPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualVsRowPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::GapStatisticPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::ParameterCorrelationHeatmap),
        GraphKind::Diagnostic(DiagnosticPlotKind::LognormalQqPlot),
        GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram),
        GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot),
        GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot),
    ]
    .into_iter()
    .map(|k| match k {
        GraphKind::Xy => ("xy", "XY Scatter / Line / Fitted Curve", "Continuous X vs Y scatter, connected lines, splines, area fills, spaghetti replicate trajectories, composite XY bars with series nudging, and decoupled subset-fit overlays."),
        GraphKind::Column => ("column", "Column Bar / Box / Violin / Dot", "One-way vertical or horizontal categorical comparisons with bar, box, violin, raincloud, beeswarm, or paired before-after lines."),
        GraphKind::Grouped => ("grouped", "Grouped Multi-Factor Figure", "Clustered, separated, stacked, or superimposed multi-factor vertical or horizontal (population pyramid) bar and point figures."),
        GraphKind::Contingency => ("contingency", "Contingency Bar / Stacked Proportions", "Categorical count or percentage bars from contingency tables."),
        GraphKind::PartsOfWhole => ("parts-of-whole", "Parts of Whole (Pie / Donut / Waffle)", "Pie, donut, stacked fraction bar, or 10x10 waffle chart."),
        GraphKind::Survival => ("survival", "Kaplan–Meier Survival Curve", "Kaplan–Meier staircase or direct trajectories (survival % / cumulative event % / fraction) with censoring ticks, Greenwood SEM/95% CI bands, and risk tables."),
        GraphKind::MultipleVariables => ("multiple-variables", "Multiple Variables Bubble / Matrix", "Multivariate bubble scatter or categorical summary mapping continuous/categorical axes, colour, size, hulls, and confidence ellipses."),
        GraphKind::Histogram => ("histogram", "Histogram & Frequency Distribution", "Contiguous frequency bars, XY frequency polygons, stepped outlines, cumulative distribution curves, and optional fitted Gaussian density/CDF overlays."),
        GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap) => ("diagnostic-heatmap", "Diagnostic: Heatmap", "Color-mapped 2D matrix with configurable row/column label sources, 0°/45°/90° rotated column headers, cell annotations, and tile gaps."),
        GraphKind::Diagnostic(DiagnosticPlotKind::Volcano) => ("diagnostic-volcano", "Diagnostic: Volcano Plot", "Effect fold-change or difference vs -log10(p) / -log10(q) significance."),
        GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman) => ("diagnostic-bland-altman", "Diagnostic: Bland–Altman Plot", "Difference/ratio vs average method-comparison plot with bias and 95% LoA."),
        GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve) => ("diagnostic-roc-curve", "Diagnostic: ROC Curve", "Sensitivity vs 1 - Specificity receiver operating characteristic curve."),
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot) => ("diagnostic-forest-plot", "Diagnostic: Forest Plot", "Horizontal effect-size, odds-ratio, or hazard-ratio plot from summary or study bounds with optional pooled summary diamond."),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot) => ("diagnostic-residual-plot", "Diagnostic: Residual Plot", "Regression residuals vs predicted Y or X scatter diagnostic."),
        GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot) => ("diagnostic-normal-qq-plot", "Diagnostic: Normal Q–Q Plot", "Theoretical normal quantiles vs observed/residual quantiles."),
        GraphKind::Diagnostic(DiagnosticPlotKind::HomoscedasticityPlot) => ("diagnostic-homoscedasticity-plot", "Diagnostic: Homoscedasticity Plot", "Absolute residuals versus predicted values for assessing variance homogeneity."),
        GraphKind::Diagnostic(DiagnosticPlotKind::ActualVsPredictedPlot) => ("diagnostic-actual-vs-predicted-plot", "Diagnostic: Actual vs Predicted Plot", "Observed response values plotted against model predictions with identity reference line."),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualVsRowPlot) => ("diagnostic-residual-vs-row-plot", "Diagnostic: Residual vs Row Plot", "Regression residuals plotted against observation row order."),
        GraphKind::Diagnostic(DiagnosticPlotKind::GapStatisticPlot) => ("diagnostic-gap-statistic-plot", "Diagnostic: Gap Statistic Plot", "Gap statistic with Monte Carlo reference error bars across candidate cluster counts."),
        GraphKind::Diagnostic(DiagnosticPlotKind::ParameterCorrelationHeatmap) => ("diagnostic-parameter-correlation-heatmap", "Diagnostic: Parameter Correlation Heatmap", "Symmetric heatmap of fitted parameter correlations from regression models."),
        GraphKind::Diagnostic(DiagnosticPlotKind::LognormalQqPlot) => ("diagnostic-lognormal-qq-plot", "Diagnostic: Lognormal Q–Q Plot", "Theoretical lognormal quantiles versus observed positive-value quantiles."),
        GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram) => ("diagnostic-dendrogram", "Diagnostic: Hierarchical Dendrogram", "Standalone hierarchical clustering merge tree ordered by leaf cluster topology with optional cut-height line."),
        GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot) => ("diagnostic-pca-biplot", "Diagnostic: PCA Biplot", "PC1 versus PC2 observation scores overlaid with scaled variable loading vectors and variance-explained axes."),
        GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot) => ("diagnostic-estimation-plot", "Diagnostic: Estimation Plot (Gardner–Altman)", "Dual-axis two-group estimation graphic aligning Group A's mean to zero on a floating mean-difference axis with 95% confidence interval and optional paired slope lines."),
    })
    .collect();
    let _ = writeln!(
        body,
        "<p class=\"lead\">Registry of <strong>{}</strong> primary and diagnostic graph kinds, <strong>{}</strong> graph preset options, axis scales, frame presets, qualitative colour palettes, and continuous colormaps.</p>",
        kinds.len(),
        GRAPH_TYPE_OPTIONS.len()
    );
    push_heading(&mut toc, &mut body, 2, "graph-kinds", "Graph Kinds & Diagnostic Plots");
    write_table(&mut body, &["Slug", "Graph Kind", "Description"], |body| {
        for (slug, label, desc) in kinds {
            let anchor = format!("graph-{slug}");
            expected.graph_kinds.push(anchor.clone());
            push_az(az, label, "Graph Kind", format!("registry/graphs.html#{anchor}"));
            write_tr(body, Some(&anchor), &[&format!("<code>{slug}</code>"), &eh(label), &eh(desc)]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "graph-type-options", "Graph Type Chooser Options (GRAPH_TYPE_OPTIONS)");
    write_table(&mut body, &["Option ID", "Label", "Compatible Tables", "Summary"], |body| {
        for opt in GRAPH_TYPE_OPTIONS {
            let anchor = format!("graph-option-{}", kebab_id(opt.id));
            expected.graph_options.push(anchor.clone());
            push_az(az, opt.label, "Graph Style Option", format!("registry/graphs.html#{anchor}"));
            let tables: Vec<&str> = opt.compatible_tables.iter().map(|t| t.label()).collect();
            write_tr(body, Some(&anchor), &[&code(opt.id), &eh(opt.label), &eh(&tables.join(", ")), &eh(opt.subtitle)]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "frame-presets", "Axis Frame Presets (FramePreset)");
    write_table(&mut body, &["Preset", "Geometry"], |body| {
        for f in [
            FramePreset::OpenL,
            FramePreset::OffsetRangeTufte,
            FramePreset::PlainBoxed,
            FramePreset::DualYSecondaryRight,
            FramePreset::OriginCrossing,
        ] {
            let (id, desc) = match f {
                FramePreset::OpenL => ("open-l", "Open L-shaped bottom X and left Y spines."),
                FramePreset::OffsetRangeTufte => ("offset-range-tufte", "Offset spines trimmed to the plotted data range."),
                FramePreset::PlainBoxed => ("plain-boxed", "Enclosed four-sided frame around the plot area."),
                FramePreset::DualYSecondaryRight => ("dual-y-secondary-right", "Left primary Y axis paired with an independent right Y2 axis."),
                FramePreset::OriginCrossing => ("origin-crossing", "Axes intersecting directly at data coordinate (0, 0)."),
            };
            let _ = writeln!(body, "<tr id=\"frame-{id}\" tabindex=\"-1\"><td><code>{id}</code></td><td>{}</td></tr>", eh(desc));
        }
    });
    push_heading(&mut toc, &mut body, 2, "axis-scales", "Axis Scale Bijections (AxisScale)");
    write_table(&mut body, &["Scale", "Domain &amp; Transform"], |body| {
        for s in [
            AxisScale::Linear,
            AxisScale::Log10,
            AxisScale::Log2,
            AxisScale::Ln,
            AxisScale::Logit(LogitDomain::Percent),
            AxisScale::Logit(LogitDomain::Proportion),
            AxisScale::Probit,
            AxisScale::Reciprocal,
        ] {
            let (id, desc) = match s {
                AxisScale::Linear => ("linear", "Identity g(u) = u over finite reals."),
                AxisScale::Log10 => ("log10", "Base-10 logarithm over u > 0."),
                AxisScale::Log2 => ("log2", "Base-2 logarithm over u > 0."),
                AxisScale::Ln => ("ln", "Natural logarithm over u > 0."),
                AxisScale::Logit(LogitDomain::Percent) => ("logit-percent", "Logit ln(p / (1 - p)) over u in (0, 100)."),
                AxisScale::Logit(LogitDomain::Proportion) => ("logit-proportion", "Logit ln(p / (1 - p)) over u in (0, 1)."),
                AxisScale::Probit => ("probit", "Normal probability quantile transform over u in (0, 100)."),
                AxisScale::Reciprocal => ("reciprocal", "Reciprocal 1 / u over nonzero finite u."),
            };
            let _ = writeln!(body, "<tr id=\"scale-{id}\" tabindex=\"-1\"><td><code>{id}</code></td><td>{}</td></tr>", eh(desc));
        }
    });
    push_heading(&mut toc, &mut body, 2, "palettes", "Qualitative Colour Palettes (PaletteId)");
    write_table(&mut body, &["Palette", "Swatches (Hex)"], |body| {
        for pal in PaletteId::ALL {
            let hexes: Vec<String> = pal.swatches().iter().map(|c| format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)).collect();
            write_tr(body, Some(&format!("palette-{}", kebab_id(pal.label()))), &[&eh(pal.label()), &code(&hexes.join(" "))]);
        }
    });
    push_heading(&mut toc, &mut body, 2, "colormaps", "Continuous Perceptual Colormaps (ColormapId)");
    write_table(&mut body, &["Colormap", "Control Stops"], |body| {
        for cm in ColormapId::ALL {
            write_tr(
                body,
                Some(&format!("colormap-{}", kebab_id(cm.label()))),
                &[&eh(cm.label()), &format!("{} uniform Oklab stops", cm.stops().len())],
            );
        }
    });
    push_heading(&mut toc, &mut body, 2, "graph-layout-features", "Graphing, Annotation & Page Layout Capabilities");
    body.push_str("<p>See <a href=\"../graphs.html\">Graphs &amp; Page Layouts</a> for full engine command and workflow guidance across the features below:</p>\n<ul>\n");
    for (title, detail) in [
        (
            "Composite XY, series nudging &amp; spaghetti trajectories",
            "Mix <code>ScatterAndLine</code> and <code>BarWithReplicates</code> series on <code>Xy</code> graphs, shift series via <code>SeriesOffsetSpec</code> (<code>x_data</code>, <code>x_pt</code>, <code>y_pt</code>), trace longitudinal <code>IndividualReplicateTrajectories</code>, or set <code>show_raw_data: false</code> for decoupled subset-fit curve overlays.",
        ),
        (
            "Horizontal orientation &amp; population pyramids",
            "Set <code>CartesianOrientation::Horizontal</code> on <code>Column</code> and <code>Grouped</code> plots paired with <code>TickLabelFormat::AbsoluteDecimal</code> (<code>|x|</code>) for symmetric diverging pyramids.",
        ),
        (
            "Kaplan–Meier modes &amp; Greenwood intervals",
            "Select <code>SurvivalDisplayMetric</code> (<code>PercentSurvival</code>, <code>PercentCumulativeEvent</code>, <code>FractionSurvival</code>), toggle staircase vs. direct segments, and choose Greenwood <code>Sem</code> or <code>ConfidenceInterval95</code> error displays.",
        ),
        (
            "Forest study bounds &amp; vertical-label heatmaps",
            "Plot Forest study confidence intervals with optional pooled summary diamonds (<code>forest_show_pooled_diamond</code>), and configure <code>HeatmapLabelSpec</code> row/column label sources, <code>Horizontal0</code> / <code>Angled45</code> / <code>Vertical90</code> column rotation, cell annotations, and <code>cell_gap_pt</code>.",
        ),
        (
            "Batch per-dataset creation &amp; selective style cloning",
            "Create one graph sheet per dataset in one undo step (<code>one_per_data_set</code> on <code>graph.create</code>), and selectively clone palette, frame/axes, or error-bar/legend settings via <code>graph.applyStyleFrom</code>.",
        ),
        (
            "Live rectangular tables, TeX equations &amp; embedded raster images",
            "Place transposable row/column slices via <code>EmbeddedResultTable</code> and <code>LiveTable</code> (<code>selected_rows</code>, <code>selected_cols</code>, <code>EmbeddedTableStyle</code>), live <code>ModelEquation</code> TeX boxes, and content-addressed <code>EmbeddedImage</code> assets (<code>graph.insertImage</code>, <code>layout.insertImage</code>, <code>edit.pasteImage</code>).",
        ),
        (
            "Page Layout scale equalization, Z-order &amp; inset layers",
            "Equalize inner plot scaling across panels with <code>layout.equalizeScaling</code> (<code>MatchSmallest</code>, <code>MatchLargest</code>, <code>ExplicitPercent</code>), adjust panel depth with <code>layout.reorderPanel</code>, and layer transparent axis-free inset graphs.",
        ),
    ] {
        let _ = writeln!(body, "  <li><strong>{title}:</strong> {detail}</li>");
    }
    body.push_str("</ul>\n");
    reg_page(
        "Graphs, Scales & Palettes Registry",
        "Registry of CurveBook graph kinds, style presets, axis scales, qualitative palettes, and colormaps.",
        "registry/graphs.html",
        toc,
        body,
    )
}

fn format_bound_f64(val: f64) -> String {
    if val == 0.0 {
        String::from("0")
    } else if val.is_finite() && !(1e-4..1e6).contains(&val.abs()) {
        format!("{val:e}")
    } else {
        val.to_string()
    }
}

fn build_formats_page(root: &Path, az: &mut Vec<AzEntry>, expected: &mut ExpectedCoverage) -> Result<RegistryPageSpec, String> {
    let reg = FormatRegistry::default_registry();
    let (mut import_ids, mut export_ids, mut seen, mut merged) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), Vec::<FormatDescriptor>::new());
    for (list, is_imp) in [(reg.supported_import_formats(), true), (reg.supported_export_formats(), false)] {
        for d in list {
            (if is_imp { &mut import_ids } else { &mut export_ids }).insert(d.id);
            if seen.insert(d.id) {
                merged.push(d);
            }
        }
    }
    if seen.insert(DESC_PLATE_MAP.id) {
        import_ids.insert(DESC_PLATE_MAP.id);
        export_ids.insert(DESC_PLATE_MAP.id);
        merged.push(DESC_PLATE_MAP);
    }
    let (mut toc, mut body) = (Vec::new(), String::with_capacity(24_576));
    let _ = writeln!(
        body,
        "<p class=\"lead\">Import/export format descriptors compiled from <code>FormatRegistry::default_registry()</code> and <code>DESC_PLATE_MAP</code> (<strong>{}</strong> formats) alongside persisted JSON Schemas and supported UI locales.</p>",
        merged.len()
    );
    push_heading(&mut toc, &mut body, 2, "formats", "Registered Import & Export Formats");
    write_table(&mut body, &["Format ID", "Label", "Extensions", "Category", "Import", "Export"], |body| {
        for d in &merged {
            let anchor = format!("format-{}", kebab_id(d.id));
            expected.formats.push(anchor.clone());
            push_az(az, d.label, "File Format", format!("registry/formats.html#{anchor}"));
            let exts: Vec<String> = d.extensions.iter().map(|e| format!(".{e}")).collect();
            let kind_label = if d.id == DESC_PLATE_MAP.id {
                "Plate Map Layout"
            } else {
                match d.kind {
                    FormatKind::Archive => "Project Archive",
                    FormatKind::Template => "Project Template",
                    FormatKind::ProjectDir => "Unpacked Project Directory",
                    FormatKind::DelimitedText => "Delimited Text",
                    FormatKind::ForeignProject => "External Project Interchange",
                    FormatKind::ResultTable => "Structured Result Export",
                    FormatKind::FigureVector => "Vector Publication Figure",
                    FormatKind::FigureRaster => "Raster Publication Figure",
                }
            };
            write_tr(
                body,
                Some(&anchor),
                &[
                    &code(d.id),
                    &eh(d.label),
                    &code(&exts.join(", ")),
                    &eh(kind_label),
                    if import_ids.contains(d.id) { "yes" } else { "no" },
                    if export_ids.contains(d.id) { "yes" } else { "no" },
                ],
            );
        }
    });
    for (file, anchor, title, prefix) in [
        ("preferences.schema.json", "schema-preferences", "Preferences File Schema", Some("pref-")),
        ("batch-v1.schema.json", "schema-batch-v1", "Batch Manifest Schema (curvebook.batch.v1)", None),
        ("document.schema.json", "schema-document", "Project Document Schema", None),
        ("archive-manifest.schema.json", "schema-archive-manifest", "Archive Manifest Schema", None),
        ("journal-record.schema.json", "schema-journal-record", "Autosave Journal Record Schema", None),
        ("plate-map.schema.json", "schema-plate-map", "Plate Map Layout Schema (.cbplate)", None),
    ] {
        let raw = std::fs::read_to_string(root.join("docs/schemas").join(file)).map_err(|e| format!("read docs/schemas/{file}: {e}"))?;
        let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse docs/schemas/{file}: {e}"))?;
        push_heading(&mut toc, &mut body, 2, anchor, title);
        if let Some(desc) = val.get("description").and_then(serde_json::Value::as_str) {
            let _ = writeln!(body, "<p>{}</p>", eh(desc));
        }
        if let Some(props) = val.get("properties").and_then(serde_json::Value::as_object) {
            write_table(&mut body, &["Property Key", "Schema Type / Details"], |body| {
                for (key, pv) in props {
                    let row_id = prefix.map(|pfx| {
                        let pid = format!("{pfx}{}", github_slug(key));
                        if file == "preferences.schema.json" {
                            expected.preference_keys.push(pid.clone());
                            push_az(az, format!("preference: {key}"), "Setting Key", format!("registry/formats.html#{pid}"));
                        }
                        eh(&pid)
                    });
                    write_tr(body, row_id.as_deref(), &[&code(key), &code(&serde_json::to_string(pv).unwrap_or_else(|_| String::from("{}")))]);
                }
            });
        }
        let _ = writeln!(body, "<pre><code class=\"language-json\">{}</code></pre>", eh(&serde_json::to_string_pretty(&val).unwrap_or(raw)));
    }
    push_heading(&mut toc, &mut body, 2, "locales", "Supported Interface Locales (LOCALES)");
    let locales_html = LOCALES.iter().map(|t| code(t)).collect::<Vec<_>>().join(", ");
    let _ = writeln!(
        body,
        "<p>Registered locale tags compiled in <code>curvebook_ui_egui::i18n::LOCALES</code> (<strong>{}</strong> entries):</p>\n<p>{locales_html}</p>",
        LOCALES.len()
    );
    Ok(reg_page(
        "Formats, Schemas & Locales Registry",
        "Registered import/export file formats, JSON Schemas (preferences, batch-v1, document, archive, journal, plate-map), and supported locales.",
        "registry/formats.html",
        toc,
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn tables_escape_categories_and_equations_format_extreme_bounds() {
        let (mut az, mut exp) = (Vec::new(), ExpectedCoverage::default());
        let tables = build_tables_page(&BTreeSet::new(), &mut az, &mut exp);
        assert!(
            tables.body_html.contains("<td>Transform &amp; Derive</td>")
                && tables.body_html.contains("<td>XY &amp; Regression</td>")
                && !tables.body_html.contains("<td>Transform & Derive</td>")
        );
        assert_eq!(
            (format_bound_f64(0.0), format_bound_f64(1e-30), format_bound_f64(0.05)),
            (String::from("0"), String::from("1e-30"), String::from("0.05"))
        );
        let eq = build_equations_page(&mut az, &mut exp);
        assert!(eq.body_html.contains("1e-30") && !eq.body_html.contains("0.000000000000000000000000000001"));
    }
}
