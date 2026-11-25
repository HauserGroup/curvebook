//! MCP `prompts/list` and `prompts/get` workflow templates for scientific analysis and publication graphics.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Metadata describing an optional or required argument on an MCP prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Prompt entry exposed in MCP `prompts/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDescriptor {
    pub name: String,
    pub description: String,
    pub arguments: Vec<PromptArgument>,
}

/// Content body inside a prompt message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTextContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

/// Single turn returned by MCP `prompts/get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: PromptTextContent,
}

/// Complete response payload for MCP `prompts/get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptGetResult {
    pub description: String,
    pub messages: Vec<PromptMessage>,
}

/// Return all registered guided workflow prompts.
#[must_use]
pub fn list_prompts() -> Vec<PromptDescriptor> {
    vec![
        PromptDescriptor {
            name: String::from("dose_response_curve_fit"),
            description: String::from(
                "Step-by-step workflow to fit a 4-parameter agonist/antagonist dose-response curve, overlay it on a graph, and inspect EC50/IC50 scalars.",
            ),
            arguments: vec![
                PromptArgument {
                    name: String::from("sheet_id"),
                    description: String::from("Target XY data sheet ID (defaults to xy-1)."),
                    required: false,
                },
                PromptArgument {
                    name: String::from("response_type"),
                    description: String::from("Response direction: agonist or antagonist (defaults to agonist)."),
                    required: false,
                },
            ],
        },
        PromptDescriptor {
            name: String::from("choose_two_group_test"),
            description: String::from(
                "Decision checklist and CurveBook command sequence for choosing between parametric and non-parametric two-group comparisons.",
            ),
            arguments: vec![
                PromptArgument { name: String::from("sheet_id"), description: String::from("Column or grouped data sheet ID."), required: false },
                PromptArgument {
                    name: String::from("paired"),
                    description: String::from(
                        "String `'true'` or `'false'` indicating whether observations are matched/paired (MCP prompt arguments must be strings).",
                    ),
                    required: false,
                },
                PromptArgument {
                    name: String::from("parametric"),
                    description: String::from(
                        "String `'true'`, `'false'`, or `'unknown'` indicating whether normality and variance assumptions hold (MCP prompt arguments must be strings).",
                    ),
                    required: false,
                },
            ],
        },
        PromptDescriptor {
            name: String::from("publication_figure_checklist"),
            description: String::from(
                "Preflight and export workflow for producing vector (SVG/PDF) or high-DPI raster (TIFF/PNG) publication figures.",
            ),
            arguments: vec![
                PromptArgument { name: String::from("graph_sheet_id"), description: String::from("GraphSheet or LayoutSheet ID."), required: false },
                PromptArgument {
                    name: String::from("target_format"),
                    description: String::from("Desired export format (pdf, svg, tiff, png)."),
                    required: false,
                },
            ],
        },
    ]
}

/// Render a prompt by name and argument map, returning `Err(msg)` for unknown names or invalid
/// argument types.
pub fn get_prompt(name: &str, args: &Map<String, Value>) -> Result<PromptGetResult, String> {
    match name {
        "dose_response_curve_fit" => build_dose_response_prompt(args),
        "choose_two_group_test" => build_two_group_prompt(args),
        "publication_figure_checklist" => build_publication_figure_prompt(args),
        _ => Err(format!("unknown prompt: {name}")),
    }
}

fn read_opt_str_arg(args: &Map<String, Value>, key: &str, default: &str) -> Result<String, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default.to_owned()),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() { Ok(default.to_owned()) } else { Ok(trimmed.to_owned()) }
        }
        Some(_) => Err(format!("prompt argument \"{key}\" must be a string")),
    }
}

fn build_dose_response_prompt(args: &Map<String, Value>) -> Result<PromptGetResult, String> {
    let sheet_id = read_opt_str_arg(args, "sheet_id", "xy-1")?;
    let response_type = read_opt_str_arg(args, "response_type", "agonist")?;
    let equation = if response_type.eq_ignore_ascii_case("antagonist") || response_type.eq_ignore_ascii_case("inhibitor") {
        "log_inhibitor_vs_response_4pl"
    } else {
        "log_agonist_vs_response_4pl"
    };

    let text = format!(
        "Run a reproducible 4PL dose-response workflow on XY sheet '{sheet_id}':\n\
         1. Inspect the sheet via `doc_inspect` (`{{\"sheet_id\":\"{sheet_id}\",\"include_cells\":true,\"max_rows\":20}}`) or read `sheet://{sheet_id}` to confirm X concentrations are log10-transformed.\n\
         2. If concentrations are linear, run `transform.run` with log10 transform on X before fitting.\n\
         3. Run `fit.run` via `command_run` (or `enable_toolset` group `'analysis'`) with `{{\"sheet_id\":\"fit-1\",\"source_sheet_id\":\"{sheet_id}\",\"equation_id\":\"{equation}\",\"method\":\"rout\",\"identify_unstable_parameters\":true}}` (use `\"method\":\"least_squares\"` for standard OLS or `\"method\":\"rout\"` for automatic ROUT outlier exclusion + OLS refit).\n\
         4. Inspect compact diagnostics via `doc_inspect` (`{{\"sheet_id\":\"fit-1\",\"include_cells\":true,\"tab_id\":\"nonlinear_parameters\"}}`) or read `result://fit-1?include_curves=false` (omit dense curve sample arrays) to check `LogEC50`/`LogIC50`, HillSlope, R2, outlier flags, and diagnostic `checklist` warnings.\n\
         5. Overlay the fitted curve on the linked graph via `graph.setRegressionOverlay` and visually verify with `render_preview`."
    );

    Ok(PromptGetResult {
        description: String::from("4-parameter dose-response nonlinear regression and figure verification workflow."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}

fn build_two_group_prompt(args: &Map<String, Value>) -> Result<PromptGetResult, String> {
    let sheet_id = read_opt_str_arg(args, "sheet_id", "col-1")?;
    let paired = read_opt_str_arg(args, "paired", "false")?;
    let parametric = read_opt_str_arg(args, "parametric", "unknown")?;

    let recommendation: String = match (paired.eq_ignore_ascii_case("true"), parametric.to_ascii_lowercase().as_str()) {
        (true, "true") => String::from("Paired Student's t-test (assume Gaussian distribution of differences)."),
        (true, "false") => String::from("Wilcoxon matched-pairs signed-rank test (non-parametric paired)."),
        (true, _) => String::from(
            "Start by evaluating paired differences for normality (e.g. Shapiro-Wilk); use Paired t-test if Gaussian or Wilcoxon matched-pairs otherwise.",
        ),
        (false, "true") => String::from("Unpaired Welch's t-test (robust when variances may differ) or Student's t-test if equal SDs are verified."),
        (false, "false") => String::from("Mann-Whitney U test (non-parametric rank test for independent groups)."),
        (false, _) => format!(
            "Inspect `sheet://{sheet_id}` sample size and distribution first; prefer Welch's t-test for roughly Gaussian continuous data or Mann-Whitney for skewed/ordinal data."
        ),
    };

    let text = format!(
        "Selecting a two-group comparison for sheet '{sheet_id}' (paired={paired}, parametric={parametric}):\n\
         - Guidance: {recommendation}\n\
         - Steps in CurveBook:\n\
           1. Read `sheet://{sheet_id}?max_rows=20` (or call `doc_inspect` `{{\"sheet_id\":\"{sheet_id}\",\"include_cells\":true,\"max_rows\":20}}`) to check group counts, missing values, and excluded cells.\n\
           2. Execute `stats.run` via `command_run` (`{{\"command_id\":\"stats.run\",\"params\":{{\"sheet_id\":\"stats-1\",\"source_sheet_id\":\"{sheet_id}\",\"kind\":\"welch_t\"}}}}` or `unpaired_t`/`paired_t`/`mann_whitney`/`wilcoxon`; for 3+ groups use `one_way_anova`/`welch_anova`/`kruskal_wallis`).\n\
           3. Read `doc_inspect` (`{{\"sheet_id\":\"stats-1\",\"include_cells\":true}}`) or `result://stats-1` to verify assumption diagnostics before plotting comparison brackets via `graph.addComparisonBrackets`."
    );

    Ok(PromptGetResult {
        description: String::from("Statistical decision guide and execution steps for two-group comparisons."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}

fn build_publication_figure_prompt(args: &Map<String, Value>) -> Result<PromptGetResult, String> {
    let graph_id = read_opt_str_arg(args, "graph_sheet_id", "graph-1")?;
    let target_format = read_opt_str_arg(args, "target_format", "pdf")?;

    let text = format!(
        "Prepare graph/layout '{graph_id}' for '{target_format}' publication export:\n\
         1. Run `graph.preflightCheck` on `{{\"sheet_id\":\"{graph_id}\"}}` to audit font sizes, clipping, overlapping labels, and legend legibility.\n\
         2. Inspect the rendered image via `render_preview` (`dpi: 144` or higher) and check `primitives_count` and `warnings`.\n\
         3. Export via the `export` tool (`format: \"{target_format}\"`, `sheet_id: \"{graph_id}\"`, `output_path: \"...\"`) choosing 300-600 DPI for TIFF/PNG rasters or vector PDF/SVG for lossless print."
    );

    Ok(PromptGetResult {
        description: String::from("Publication figure preflight and high-DPI/vector export checklist."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}
