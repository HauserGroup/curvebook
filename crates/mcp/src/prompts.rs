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

/// Return all registered guided workflow prompts for headless/normal MCP mode.
#[must_use]
pub fn list_prompts() -> Vec<PromptDescriptor> {
    list_prompts_for_mode(false)
}

/// Return guided workflow prompts tailored to `--gui-only` (`gui_only == true`) or normal MCP mode.
#[must_use]
pub fn list_prompts_for_mode(gui_only: bool) -> Vec<PromptDescriptor> {
    let descriptions = if gui_only {
        [
            "GUI-only step-by-step workflow to inspect an XY sheet, fit a 4-parameter dose-response curve via menus/dialogs, and verify via screenshots.",
            "GUI-only decision checklist and interactive dialog sequence for choosing between parametric and non-parametric two-group comparisons.",
            "GUI-only figure preflight, visual inspection, and interactive export workflow using menus, dialogs, and inline screenshots.",
        ]
    } else {
        [
            "Step-by-step workflow to fit a 4-parameter agonist/antagonist dose-response curve, overlay it on a graph, and inspect EC50/IC50 scalars.",
            "Decision checklist and LumenCraft command sequence for choosing between parametric and non-parametric two-group comparisons.",
            "Preflight and export workflow for producing vector (SVG/PDF) or high-DPI raster (TIFF/PNG) publication figures.",
        ]
    };

    vec![
        PromptDescriptor {
            name: String::from("dose_response_curve_fit"),
            description: String::from(descriptions[0]),
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
            description: String::from(descriptions[1]),
            arguments: vec![
                PromptArgument { name: String::from("sheet_id"), description: String::from("Column or grouped data sheet ID."), required: false },
                PromptArgument {
                    name: String::from("paired"),
                    description: String::from("Whether observations are matched/paired (true/false)."),
                    required: false,
                },
                PromptArgument {
                    name: String::from("parametric"),
                    description: String::from("Whether normality and variance assumptions hold (true/false/unknown)."),
                    required: false,
                },
            ],
        },
        PromptDescriptor {
            name: String::from("publication_figure_checklist"),
            description: String::from(descriptions[2]),
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

/// Render a prompt by name and argument map for headless/normal MCP mode, returning `Err(msg)` for
/// unknown names or invalid argument types.
pub fn get_prompt(name: &str, args: &Map<String, Value>) -> Result<PromptGetResult, String> {
    get_prompt_for_mode(name, args, false)
}

/// Render a prompt by name and argument map tailored to `--gui-only` (`gui_only == true`) or normal MCP mode.
pub fn get_prompt_for_mode(name: &str, args: &Map<String, Value>, gui_only: bool) -> Result<PromptGetResult, String> {
    match name {
        "dose_response_curve_fit" => build_dose_response_prompt(args, gui_only),
        "choose_two_group_test" => build_two_group_prompt(args, gui_only),
        "publication_figure_checklist" => build_publication_figure_prompt(args, gui_only),
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

fn build_dose_response_prompt(args: &Map<String, Value>, gui_only: bool) -> Result<PromptGetResult, String> {
    let sheet_id = read_opt_str_arg(args, "sheet_id", "xy-1")?;
    let response_type = read_opt_str_arg(args, "response_type", "agonist")?;
    let equation = if response_type.eq_ignore_ascii_case("antagonist") || response_type.eq_ignore_ascii_case("inhibitor") {
        "log_inhibitor_vs_response_4pl"
    } else {
        "log_agonist_vs_response_4pl"
    };

    let text = if gui_only {
        format!(
            "Run a reproducible 4PL dose-response workflow on XY sheet '{sheet_id}' using only GUI tools:\n\
             1. Call `ui_switch_tab` (`{{\"sheet_id\":\"{sheet_id}\"}}`) and `ui_inspect` (`{{\"scope\":\"grid\"}}`) to review the visible XY concentrations.\n\
             2. Open the Transform or Analyze dialog via `ui_invoke_menu` (for example `Analyze/Analyze...`) and inspect dialog fields with `ui_inspect` (`{{\"scope\":\"dialog\"}}`).\n\
             3. Configure the 4-parameter nonlinear regression equation (`{equation}`) with `ui_answer_dialog`, then confirm (`action: \"ok\"`) to generate the result sheet.\n\
             4. Switch to the result sheet with `ui_switch_tab`, inspect diagnostics with `ui_inspect` (`{{\"scope\":\"all\"}}`), and verify the fitted curve visually via `ui_screenshot`."
        )
    } else {
        format!(
            "Run a reproducible 4PL dose-response workflow on XY sheet '{sheet_id}':\n\
             1. Inspect the sheet via `doc_inspect` (or read resource `sheet://{sheet_id}`) to confirm X concentrations are log10-transformed.\n\
             2. If concentrations are linear, run `transform.run` with log10 transform on X before fitting.\n\
             3. Enable the `analysis` toolset (`enable_toolset` with `{{\"group\":\"analysis\"}}`) or use `command_run` to execute `fit.run` with `{{\"sheet_id\":\"fit-1\",\"source_sheet_id\":\"{sheet_id}\",\"equation_id\":\"{equation}\"}}`.\n\
             4. Read resource `result://fit-1` to inspect `LogEC50`/`LogIC50`, HillSlope, R2, and diagnostic `checklist` flags (e.g. wide confidence intervals or insufficient dose range).\n\
             5. Overlay the fitted curve on the linked graph via `graph.setRegressionOverlay` and visually verify with `render_preview`."
        )
    };

    Ok(PromptGetResult {
        description: String::from("4-parameter dose-response nonlinear regression and figure verification workflow."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}

fn build_two_group_prompt(args: &Map<String, Value>, gui_only: bool) -> Result<PromptGetResult, String> {
    let sheet_id = read_opt_str_arg(args, "sheet_id", "col-1")?;
    let paired = read_opt_str_arg(args, "paired", "false")?;
    let parametric = read_opt_str_arg(args, "parametric", "unknown")?;

    let recommendation = match (paired.eq_ignore_ascii_case("true"), parametric.to_ascii_lowercase().as_str()) {
        (true, "true") => "Paired Student's t-test (assume Gaussian distribution of differences).",
        (true, "false") => "Wilcoxon matched-pairs signed-rank test (non-parametric paired).",
        (true, _) => {
            "Start by evaluating paired differences for normality (e.g. Shapiro-Wilk); use Paired t-test if Gaussian or Wilcoxon matched-pairs otherwise."
        }
        (false, "true") => "Unpaired Welch's t-test (robust when variances may differ) or Student's t-test if equal SDs are verified.",
        (false, "false") => "Mann-Whitney U test (non-parametric rank test for independent groups).",
        (false, _) if gui_only => {
            "Inspect the visible grid values via `ui_inspect` first; prefer Welch's t-test for roughly Gaussian continuous data or Mann-Whitney for skewed/ordinal data."
        }
        (false, _) => {
            "Inspect `sheet://{sheet_id}` sample size and distribution first; prefer Welch's t-test for roughly Gaussian continuous data or Mann-Whitney for skewed/ordinal data."
        }
    };

    let text = if gui_only {
        format!(
            "Selecting a two-group comparison for sheet '{sheet_id}' in `--gui-only` mode (paired={paired}, parametric={parametric}):\n\
             - Guidance: {recommendation}\n\
             - Steps in LumenCraft GUI:\n\
               1. Focus '{sheet_id}' via `ui_switch_tab` and review visible columns/exclusions via `ui_inspect` (`{{\"scope\":\"grid\"}}`).\n\
               2. Open `Analyze/Analyze...` via `ui_invoke_menu`, choose the matching test family in `ui_answer_dialog`, and confirm (`action: \"ok\"`).\n\
               3. Review the visible result summary and diagnostics via `ui_inspect`, then format comparison brackets via `Format/Pairwise Comparison Brackets...` and `ui_screenshot`."
        )
    } else {
        format!(
            "Selecting a two-group comparison for sheet '{sheet_id}' (paired={paired}, parametric={parametric}):\n\
             - Guidance: {recommendation}\n\
             - Steps in LumenCraft:\n\
               1. Read `sheet://{sheet_id}` (or call `doc_inspect`) to check group counts, missing values, and excluded cells.\n\
               2. Execute `stats.run` via `command_run` (or enable the `analysis` toolset) to compute test statistics, unrounded p-values, effect-size CI bounds, and assumption `checklist` checks.\n\
               3. Read `result://<result_id>` to verify assumption diagnostics before plotting comparison brackets via `graph.addComparisonBrackets`."
        )
    };

    Ok(PromptGetResult {
        description: String::from("Statistical decision guide and execution steps for two-group comparisons."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}

fn build_publication_figure_prompt(args: &Map<String, Value>, gui_only: bool) -> Result<PromptGetResult, String> {
    let graph_id = read_opt_str_arg(args, "graph_sheet_id", "graph-1")?;
    let target_format = read_opt_str_arg(args, "target_format", "pdf")?;

    let text = if gui_only {
        format!(
            "Prepare graph/layout '{graph_id}' for '{target_format}' review using only GUI tools:\n\
             1. Switch to '{graph_id}' via `ui_switch_tab` (`{{\"sheet_id\":\"{graph_id}\"}}`) and inspect visible canvas hit targets via `ui_inspect` (`{{\"scope\":\"canvas\",\"include_bounds\":true}}`).\n\
             2. Run preflight via `ui_invoke_menu` (`Graph/Preflight Check`) and adjust axes or styling via `Format/Format Axes...` or `Format/Format Graph...` + `ui_answer_dialog`.\n\
             3. Capture an inline visual check via `ui_screenshot` (`{{\"target\":\"canvas\"}}`) and open `File/Export Figure...` via `ui_invoke_menu` + `ui_answer_dialog` if interactive export dialog settings are needed."
        )
    } else {
        format!(
            "Prepare graph/layout '{graph_id}' for '{target_format}' publication export:\n\
             1. Run `graph.preflightCheck` on `{{\"sheet_id\":\"{graph_id}\"}}` to audit font sizes, clipping, overlapping labels, and legend legibility.\n\
             2. Inspect the rendered image via `render_preview` (`dpi: 144` or higher) and check `primitives_count` and `warnings`.\n\
             3. Export via the `export` tool (`format: \"{target_format}\"`, `sheet_id: \"{graph_id}\"`, `output_path: \"...\"`) choosing 300-600 DPI for TIFF/PNG rasters or vector PDF/SVG for lossless print."
        )
    };

    Ok(PromptGetResult {
        description: String::from("Publication figure preflight and high-DPI/vector export checklist."),
        messages: vec![PromptMessage { role: String::from("user"), content: PromptTextContent { kind: String::from("text"), text } }],
    })
}
