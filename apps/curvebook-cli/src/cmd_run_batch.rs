//! `curvebook-cli run <script>` and `curvebook-cli batch <manifest>` execution runners.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use curvebook_doc::{ErrorCode, ProjectDocument, ResultStatus};
use curvebook_engine::cmd::Caller;
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, instantiate_template_bytes};
use curvebook_pdf::PdfVersion;
use curvebook_platform::PlatformError;
use serde_json::{Map, Value, json};

use crate::cmd_convert::{
    ExportFormat, ExportRequest, detect_export_format_from_path, export_from_document, load_project_from_path, load_validated_ttf_font,
    parse_export_format, parse_pdf_version,
};
use crate::error::CliError;
use crate::sandbox::{PathResolver, WorkdirGuard};

const BATCH_SCHEMA_V1: &str = "curvebook.batch.v1";
const MAX_SCRIPT_FILE_BYTES: usize = 64 * 1024 * 1024;
const MANIFEST_ROOT_KEYS: &[&str] = &["schema", "template", "project", "inputs", "commands", "exports"];
const MANIFEST_INPUT_KEYS: &[&str] = &[
    "csv",
    "tsv",
    "path",
    "file",
    "sheet",
    "sheet_id",
    "sheetId",
    "kind",
    "table_kind",
    "tableKind",
    "subcolumn_mode",
    "subcolumnMode",
    "delimiter",
    "decimal_separator",
    "decimalSeparator",
];
const MANIFEST_COMMAND_KEYS: &[&str] = &["cmd", "command", "method", "id", "params"];
const MANIFEST_EXPORT_KEYS: &[&str] = &[
    "out",
    "path",
    "format",
    "sheet",
    "sheet_id",
    "sheetId",
    "dpi",
    "pretty",
    "tab_index",
    "tabIndex",
    "pdf_version",
    "pdfVersion",
    "ttf_font",
    "ttfFont",
];

struct RunnerArgs {
    source_path: String,
    workdir: Option<String>,
    json: bool,
}

#[derive(Debug, Clone)]
struct ParsedCommand {
    index: usize,
    id: Option<Value>,
    cmd: String,
    params: Value,
}

struct BatchInputSpec {
    csv_path: PathBuf,
    sheet: Option<String>,
    kind: Option<String>,
    subcolumn_mode: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
}

struct BatchExportSpec {
    out_path: PathBuf,
    format: ExportFormat,
    sheet: Option<String>,
    dpi: Option<u16>,
    pretty: bool,
    tab_index: Option<usize>,
    pdf_version: PdfVersion,
    ttf_font_path: Option<PathBuf>,
}

struct ParsedManifest {
    template_path: Option<PathBuf>,
    project_path: Option<PathBuf>,
    inputs: Vec<BatchInputSpec>,
    commands: Vec<ParsedCommand>,
    exports: Vec<BatchExportSpec>,
}

/// Execute `curvebook-cli run <script> [--workdir <dir>] [--json]`.
pub fn run_script(args: &[String]) -> Result<(), CliError> {
    crate::cmd_convert::run_guarded_cli("cli.run", || {
        let parsed = parse_runner_args(args, "usage: curvebook-cli run <script> [--workdir <dir>] [--json]")?;
        let resolver = build_resolver(parsed.workdir.as_deref(), Path::new("."))?;
        let raw_text = read_text_source(&parsed.source_path)?;
        let commands = parse_script_commands(&raw_text, &resolver)?;

        let mut session = Session::new();
        if let Some(dir) = parsed.workdir.as_deref() {
            session.set_path_policy(ExternalPathPolicy::workdir(Path::new(dir))?);
        }
        let results = execute_commands(&mut session, &commands, parsed.workdir.is_some())?;

        if parsed.json {
            let payload = json!({
                "ok": true,
                "script": parsed.source_path,
                "commands_executed": results.len(),
                "total_sheets": session.document().total_sheet_count(),
                "results": results,
            });
            let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            println!("{text}");
        } else {
            println!("executed {} command(s) cleanly (sheets: {})", results.len(), session.document().total_sheet_count());
        }
        Ok(())
    })
}

/// Execute `curvebook-cli batch <manifest> [--workdir <dir>] [--json]`.
pub fn run_batch(args: &[String]) -> Result<(), CliError> {
    crate::cmd_convert::run_guarded_cli("cli.batch", || {
        let parsed = parse_runner_args(args, "usage: curvebook-cli batch <manifest> [--workdir <dir>] [--json]")?;
        let manifest_base = if parsed.source_path == "-" {
            Path::new(".").to_path_buf()
        } else {
            Path::new(&parsed.source_path).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new(".")).to_path_buf()
        };
        let resolver = build_resolver(parsed.workdir.as_deref(), &manifest_base)?;
        let raw_text = read_text_source(&parsed.source_path)?;
        let manifest = parse_batch_manifest(&raw_text, &resolver)?;

        let mut session = open_manifest_base_session(&manifest)?;
        if let Some(dir) = parsed.workdir.as_deref() {
            session.set_path_policy(ExternalPathPolicy::workdir(Path::new(dir))?);
        }
        load_manifest_inputs(&mut session, &manifest.inputs)?;
        let results = execute_commands(&mut session, &manifest.commands, parsed.workdir.is_some())?;
        session.recalculate_all()?;
        ensure_no_failed_result_sheets(session.document())?;

        let mut export_summaries = Vec::with_capacity(manifest.exports.len());
        for exp in &manifest.exports {
            let ttf_font_bytes = exp.ttf_font_path.as_deref().map(load_validated_ttf_font).transpose()?;
            let summary = export_from_document(&ExportRequest {
                doc: session.document(),
                out_path: &exp.out_path,
                format: exp.format,
                sheet: exp.sheet.as_deref(),
                dpi: exp.dpi,
                pretty: exp.pretty,
                tab_index: exp.tab_index,
                pdf_version: exp.pdf_version,
                embedded_truetype_font: ttf_font_bytes.as_deref(),
            })?;
            export_summaries.push(json!({
                "out": summary.out_path,
                "format": summary.format,
                "sheet_id": summary.sheet_id,
                "bytes_written": summary.bytes_written,
            }));
        }

        if parsed.json {
            let payload = json!({
                "ok": true,
                "schema": BATCH_SCHEMA_V1,
                "manifest": parsed.source_path,
                "inputs_loaded": manifest.inputs.len(),
                "commands_executed": results.len(),
                "exports_written": export_summaries.len(),
                "exports": export_summaries,
                "results": results,
            });
            let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            println!("{text}");
        } else {
            println!("batch completed: {} input(s), {} command(s), {} export(s)", manifest.inputs.len(), results.len(), export_summaries.len());
        }
        Ok(())
    })
}

fn ensure_no_failed_result_sheets(doc: &ProjectDocument) -> Result<(), CliError> {
    let mut failed_entries = Vec::new();
    let mut descriptions = Vec::new();
    let mut first_code: Option<ErrorCode> = None;

    for rs in &doc.result_sheets {
        if let ResultStatus::Failed { code, message } = &rs.status {
            if first_code.is_none() {
                first_code = Some(*code);
            }
            let title = rs.title();
            let label = if title.is_empty() { rs.id.to_string() } else { format!("{title} ({})", rs.id) };
            descriptions.push(format!("{label}: [{}] {message}", code.as_str()));
            failed_entries.push(json!({
                "sheet_id": rs.id.to_string(),
                "title": title,
                "code": code.as_str(),
                "message": message,
            }));
        }
    }

    if descriptions.is_empty() {
        return Ok(());
    }

    // Batch failures must always map to exit code 1 (`EXIT_RUNTIME`); Cancelled/PathEscape would otherwise map to 130/2.
    let outer_code = match first_code {
        Some(ErrorCode::Cancelled | ErrorCode::PathEscape) | None => ErrorCode::SolverFailed,
        Some(code) => code,
    };
    let err =
        EngineError::with_code(outer_code, format!("batch failed with {} failed result sheet(s): {}", descriptions.len(), descriptions.join("; ")))
            .with_remediation("Fix the input data, column types, or analysis settings for the failed result sheet(s) before exporting.")
            .with_details(json!({
                "failed_count": failed_entries.len(),
                "failed_result_sheets": failed_entries,
            }));
    Err(CliError::Engine(err))
}

fn build_resolver(workdir: Option<&str>, fallback_base: &Path) -> Result<PathResolver, CliError> {
    match workdir {
        Some(dir) => Ok(PathResolver::Sandboxed(WorkdirGuard::new(Path::new(dir))?)),
        None => Ok(PathResolver::Unconstrained { base_dir: fallback_base.to_path_buf() }),
    }
}

fn read_text_source(source: &str) -> Result<String, CliError> {
    if source == "-" {
        let mut bytes = Vec::new();
        let cap = (MAX_SCRIPT_FILE_BYTES as u64).saturating_add(1);
        std::io::stdin().take(cap).read_to_end(&mut bytes).map_err(|err| CliError::Runtime(format!("failed to read stdin: {err}")))?;
        if bytes.len() > MAX_SCRIPT_FILE_BYTES {
            return Err(CliError::from(PlatformError::FileTooLarge {
                path: String::from("<stdin>"),
                actual_bytes: None,
                limit_bytes: MAX_SCRIPT_FILE_BYTES,
            }));
        }
        return String::from_utf8(bytes).map_err(|err| CliError::Runtime(format!("failed to read stdin as UTF-8: {err}")));
    }
    curvebook_platform::fs::read_regular_file_utf8(Path::new(source), MAX_SCRIPT_FILE_BYTES).map_err(|err| match err {
        PlatformError::FileTooLarge { .. } => CliError::from(err),
        other => CliError::Runtime(format!("failed to read `{source}`: {other}")),
    })
}

fn parse_script_commands(raw_text: &str, resolver: &PathResolver) -> Result<Vec<ParsedCommand>, CliError> {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        let arr: Vec<Value> = serde_json::from_str(trimmed).map_err(|err| CliError::Schema(format!("invalid JSON command array: {err}")))?;
        return arr.into_iter().enumerate().map(|(idx, val)| parse_command_value(idx, val, resolver)).collect();
    }

    let mut out = Vec::new();
    for (line_idx, raw_line) in raw_text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let val: Value = serde_json::from_str(line).map_err(|err| CliError::Schema(format!("line {}: invalid JSON command: {err}", line_idx + 1)))?;
        out.push(parse_command_value(line_idx, val, resolver)?);
    }
    Ok(out)
}

fn parse_command_value(index: usize, val: Value, resolver: &PathResolver) -> Result<ParsedCommand, CliError> {
    let line_label = index + 1;
    let Value::Object(mut map) = val else {
        return Err(CliError::Schema(format!("command #{line_label} must be a JSON object")));
    };

    let cmd = extract_cmd_name(&map, line_label)?;
    let id = map.remove("id");
    let mut params = match map.remove("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(obj @ Value::Object(_)) => obj,
        Some(_) => return Err(CliError::Schema(format!("command #{line_label} (`{cmd}`) `params` must be an object"))),
    };

    resolver.sanitize_params(&mut params)?;
    Ok(ParsedCommand { index, id, cmd, params })
}

fn extract_cmd_name(map: &Map<String, Value>, line_label: usize) -> Result<String, CliError> {
    for key in ["cmd", "command", "method"] {
        if let Some(val) = map.get(key) {
            let Some(name) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                return Err(CliError::Schema(format!("command #{line_label} `{key}` must be a non-empty string")));
            };
            return Ok(name.to_owned());
        }
    }
    Err(CliError::Schema(format!("command #{line_label} must contain `cmd`, `command`, or `method`")))
}

fn parse_batch_manifest(raw_text: &str, resolver: &PathResolver) -> Result<ParsedManifest, CliError> {
    let val: Value = serde_json::from_str(raw_text).map_err(|err| CliError::Schema(format!("invalid manifest JSON: {err}")))?;
    let Some(obj) = val.as_object() else {
        return Err(CliError::Schema(String::from("batch manifest root must be a JSON object")));
    };

    let schema = obj.get("schema").and_then(Value::as_str).unwrap_or("");
    if schema != BATCH_SCHEMA_V1 {
        return Err(CliError::Schema(format!("manifest `schema` must be `{BATCH_SCHEMA_V1}` (got `{schema}`)")));
    }
    reject_unknown_keys(obj, MANIFEST_ROOT_KEYS, "manifest")?;

    let template_path = parse_optional_resolved_path(obj, "template", resolver)?;
    let project_path = parse_optional_resolved_path(obj, "project", resolver)?;
    if template_path.is_some() && project_path.is_some() {
        return Err(CliError::Schema(String::from("manifest cannot specify both `template` and `project`")));
    }

    let inputs = parse_manifest_inputs(obj.get("inputs"), resolver)?;
    let commands = parse_manifest_commands(obj.get("commands"), resolver)?;
    let exports = parse_manifest_exports(obj.get("exports"), resolver)?;

    Ok(ParsedManifest { template_path, project_path, inputs, commands, exports })
}

fn parse_optional_resolved_path(obj: &Map<String, Value>, key: &str, resolver: &PathResolver) -> Result<Option<PathBuf>, CliError> {
    let Some(val) = obj.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(raw) = val.as_str() else {
        return Err(CliError::Schema(format!("manifest `{key}` must be a string path")));
    };
    Ok(Some(resolver.resolve(raw)?))
}

fn parse_manifest_inputs(val: Option<&Value>, resolver: &PathResolver) -> Result<Vec<BatchInputSpec>, CliError> {
    let Some(val) = val else {
        return Ok(Vec::new());
    };
    if val.is_null() {
        return Ok(Vec::new());
    }
    let Some(arr) = val.as_array() else {
        return Err(CliError::Schema(String::from("manifest `inputs` must be an array")));
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let context = format!("manifest `inputs[{idx}]`");
        let Some(obj) = item.as_object() else {
            return Err(CliError::Schema(format!("{context} must be an object")));
        };
        reject_unknown_keys(obj, MANIFEST_INPUT_KEYS, &context)?;
        let raw_path = opt_obj_str(obj, &["csv", "tsv", "path", "file"], &context)?
            .ok_or_else(|| CliError::Schema(format!("{context} requires a string `csv` (or `path`) field")))?;
        let csv_path = resolver.resolve(raw_path)?;
        out.push(BatchInputSpec {
            csv_path,
            sheet: opt_obj_str(obj, &["sheet", "sheet_id", "sheetId"], &context)?.map(str::to_owned),
            kind: opt_obj_str(obj, &["kind", "table_kind", "tableKind"], &context)?.map(str::to_owned),
            subcolumn_mode: opt_obj_str(obj, &["subcolumn_mode", "subcolumnMode"], &context)?.map(str::to_owned),
            delimiter: opt_obj_str(obj, &["delimiter"], &context)?.map(str::to_owned),
            decimal_separator: opt_obj_str(obj, &["decimal_separator", "decimalSeparator"], &context)?.map(str::to_owned),
        });
    }
    Ok(out)
}

fn parse_manifest_commands(val: Option<&Value>, resolver: &PathResolver) -> Result<Vec<ParsedCommand>, CliError> {
    let Some(val) = val else {
        return Ok(Vec::new());
    };
    if val.is_null() {
        return Ok(Vec::new());
    }
    let Some(arr) = val.as_array() else {
        return Err(CliError::Schema(String::from("manifest `commands` must be an array")));
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(CliError::Schema(format!("command #{} must be a JSON object", idx + 1)));
        };
        reject_unknown_keys(obj, MANIFEST_COMMAND_KEYS, &format!("manifest `commands[{idx}]`"))?;
        out.push(parse_command_value(idx, item.clone(), resolver)?);
    }
    Ok(out)
}

fn parse_manifest_exports(val: Option<&Value>, resolver: &PathResolver) -> Result<Vec<BatchExportSpec>, CliError> {
    let Some(val) = val else {
        return Ok(Vec::new());
    };
    if val.is_null() {
        return Ok(Vec::new());
    }
    let Some(arr) = val.as_array() else {
        return Err(CliError::Schema(String::from("manifest `exports` must be an array")));
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        out.push(parse_single_export_entry(idx, item, resolver)?);
    }
    Ok(out)
}

fn parse_single_export_entry(idx: usize, item: &Value, resolver: &PathResolver) -> Result<BatchExportSpec, CliError> {
    let context = format!("manifest `exports[{idx}]`");
    let Some(obj) = item.as_object() else {
        return Err(CliError::Schema(format!("{context} must be an object")));
    };
    reject_unknown_keys(obj, MANIFEST_EXPORT_KEYS, &context)?;
    let raw_out =
        opt_obj_str(obj, &["out", "path"], &context)?.ok_or_else(|| CliError::Schema(format!("{context} requires string `out` (or `path`)")))?;
    let out_path = resolver.resolve(raw_out)?;
    let format = match obj.get("format") {
        Some(Value::String(s)) => parse_export_format(s).map_err(cli_error_as_schema)?,
        None | Some(Value::Null) => detect_export_format_from_path(&out_path).map_err(cli_error_as_schema)?,
        Some(_) => return Err(CliError::Schema(format!("manifest `exports[{idx}].format` must be a string"))),
    };
    let dpi = parse_export_dpi(idx, obj.get("dpi"))?;
    let pretty = parse_export_pretty(idx, obj.get("pretty"))?;
    let tab_index = parse_export_tab_index(idx, obj.get("tab_index").or_else(|| obj.get("tabIndex")))?;
    let pdf_version = parse_export_pdf_version(idx, obj.get("pdf_version").or_else(|| obj.get("pdfVersion")))?;
    let ttf_font_path = parse_export_ttf_font(idx, obj.get("ttf_font").or_else(|| obj.get("ttfFont")), resolver)?;

    Ok(BatchExportSpec {
        out_path,
        format,
        sheet: opt_obj_str(obj, &["sheet", "sheet_id", "sheetId"], &context)?.map(str::to_owned),
        dpi,
        pretty,
        tab_index,
        pdf_version,
        ttf_font_path,
    })
}

fn parse_export_pretty(idx: usize, val: Option<&Value>) -> Result<bool, CliError> {
    match val {
        None | Some(Value::Null) => Ok(true),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(CliError::Schema(format!("manifest `exports[{idx}].pretty` must be a boolean"))),
    }
}

fn parse_export_dpi(idx: usize, val: Option<&Value>) -> Result<Option<u16>, CliError> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            let raw_u64 = n.as_u64().ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].dpi` must be a positive integer")))?;
            let val_u16 = u16::try_from(raw_u64)
                .ok()
                .filter(|&d| d > 0)
                .ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].dpi` out of range")))?;
            Ok(Some(val_u16))
        }
        Some(_) => Err(CliError::Schema(format!("manifest `exports[{idx}].dpi` must be an integer"))),
    }
}

fn parse_export_tab_index(idx: usize, val: Option<&Value>) -> Result<Option<usize>, CliError> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            let raw_u64 = n.as_u64().ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].tab_index` must be an integer")))?;
            let idx_val = usize::try_from(raw_u64).map_err(|_| CliError::Schema(format!("manifest `exports[{idx}].tab_index` out of bounds")))?;
            Ok(Some(idx_val))
        }
        Some(_) => Err(CliError::Schema(format!("manifest `exports[{idx}].tab_index` must be an integer"))),
    }
}

fn parse_export_pdf_version(idx: usize, val: Option<&Value>) -> Result<PdfVersion, CliError> {
    match val {
        None | Some(Value::Null) => Ok(PdfVersion::Pdf17),
        Some(Value::String(s)) => parse_pdf_version(s).map_err(cli_error_as_schema),
        Some(_) => Err(CliError::Schema(format!("manifest `exports[{idx}].pdf_version` must be a string (`1.4` or `1.7`)"))),
    }
}

fn parse_export_ttf_font(idx: usize, val: Option<&Value>, resolver: &PathResolver) -> Result<Option<PathBuf>, CliError> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => Ok(Some(resolver.resolve(raw)?)),
        Some(_) => Err(CliError::Schema(format!("manifest `exports[{idx}].ttf_font` must be a string path"))),
    }
}

fn open_manifest_base_session(manifest: &ParsedManifest) -> Result<Session, CliError> {
    if let Some(tpl_path) = &manifest.template_path {
        let bytes = curvebook_platform::fs::read_regular_file_bytes(tpl_path, MAX_ARCHIVE_FILE_BYTES).map_err(|err| match err {
            PlatformError::FileTooLarge { .. } => CliError::from(err),
            other => CliError::Runtime(format!("failed to read template `{}`: {other}", tpl_path.display())),
        })?;
        let doc = instantiate_template_bytes(&bytes, None)?;
        return Ok(Session::from_document(doc));
    }
    if let Some(proj_path) = &manifest.project_path {
        let (loaded, _bytes) = load_project_from_path(proj_path, false)?;
        return Ok(Session::from_document(loaded.doc));
    }
    Ok(Session::new())
}

fn load_manifest_inputs(session: &mut Session, inputs: &[BatchInputSpec]) -> Result<(), CliError> {
    for (idx, input) in inputs.iter().enumerate() {
        let csv_text = curvebook_platform::fs::read_regular_file_utf8(&input.csv_path, MAX_CSV_FILE_BYTES).map_err(|err| match err {
            PlatformError::FileTooLarge { .. } => CliError::from(err),
            other => CliError::Runtime(format!("failed to read input `{}`: {other}", input.csv_path.display())),
        })?;

        let matched_sheet_id = resolve_existing_data_sheet_id(session.document(), input.sheet.as_deref());
        let target_sheet_id = matched_sheet_id.clone().or_else(|| input.sheet.clone()).unwrap_or_else(|| format!("data_{}", idx + 1));

        let mut params = Map::new();
        params.insert(String::from("csv"), Value::String(csv_text));
        params.insert(String::from("sheet_id"), Value::String(target_sheet_id));
        params.insert(String::from("replace"), Value::Bool(matched_sheet_id.is_some()));
        if let Some(kind) = &input.kind {
            params.insert(String::from("archetype"), Value::String(kind.clone()));
        } else if matched_sheet_id.is_none() {
            params.insert(String::from("archetype"), Value::String(String::from("xy")));
        }
        if let Some(mode) = &input.subcolumn_mode {
            params.insert(String::from("subcolumn_mode"), Value::String(mode.clone()));
        }
        if let Some(delim) = &input.delimiter {
            params.insert(String::from("delimiter"), Value::String(delim.clone()));
        }
        if let Some(dec) = &input.decimal_separator {
            params.insert(String::from("decimal_separator"), Value::String(dec.clone()));
        }

        session.execute("table.importCsv", Value::Object(params))?;
    }
    Ok(())
}

fn resolve_existing_data_sheet_id(doc: &ProjectDocument, sheet: Option<&str>) -> Option<String> {
    let needle = sheet?;
    doc.data_sheets
        .iter()
        .find(|d| d.id.as_str() == needle)
        .or_else(|| doc.data_sheets.iter().find(|d| d.title == needle))
        .or_else(|| doc.data_sheets.iter().find(|d| d.title.eq_ignore_ascii_case(needle)))
        .map(|d| d.id.as_str().to_owned())
}

fn execute_commands(session: &mut Session, commands: &[ParsedCommand], ensure_output_parents: bool) -> Result<Vec<Value>, CliError> {
    let mut results = Vec::with_capacity(commands.len());
    for entry in commands {
        let mut params = entry.params.clone();
        apply_command_convenience_defaults(session.document(), &entry.cmd, &mut params);
        if ensure_output_parents && is_file_write_command(&entry.cmd) {
            ensure_parent_dir_exists(&params)?;
        }
        let cancelled = AtomicBool::new(false);
        let outcome = session.execute_with_caller(&entry.cmd, params, Caller::Cli, &cancelled)?;
        results.push(json!({
            "index": entry.index,
            "id": entry.id,
            "cmd": entry.cmd,
            "mutated": outcome.mutated,
            "message": outcome.message,
            "result": outcome.payload,
        }));
    }
    Ok(results)
}

fn apply_command_convenience_defaults(doc: &ProjectDocument, cmd: &str, params: &mut Value) {
    if (cmd == "info.setConstant" || cmd == "info.setNotes" || cmd == "info.removeConstant")
        && let Some(obj) = params.as_object_mut()
    {
        let has_sheet = obj.contains_key("sheet_id") || obj.contains_key("sheetId") || obj.contains_key("info_sheet_id") || obj.contains_key("id");
        if !has_sheet && doc.info_sheets.len() == 1 {
            obj.insert(String::from("sheet_id"), Value::String(doc.info_sheets[0].id.as_str().to_owned()));
        }
    }
}

fn is_file_write_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "project.save"
            | "project.saveTemplate"
            | "table.exportCsv"
            | "analysis.exportJson"
            | "analysis.exportCsv"
            | "export.svg"
            | "export.pdf"
            | "export.emf"
            | "export.png"
            | "export.tiff"
            | "graph.exportEmf"
            | "graph.exportSvg"
            | "graph.exportPdf"
            | "graph.exportPng"
            | "graph.exportTiff"
            | "layout.exportEmf"
            | "layout.exportSvg"
            | "layout.exportPdf"
            | "layout.exportPng"
            | "layout.exportTiff"
    )
}

fn ensure_parent_dir_exists(params: &Value) -> Result<(), CliError> {
    if let Some(raw_path) = params.get("path").and_then(Value::as_str) {
        let p = Path::new(raw_path);
        if let Some(parent) = p.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent).map_err(|err| CliError::Runtime(format!("failed to create directory `{}`: {err}", parent.display())))?;
        }
    }
    Ok(())
}

fn reject_unknown_keys(obj: &Map<String, Value>, allowed: &[&str], context: &str) -> Result<(), CliError> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            let hint = match (context, key.as_str()) {
                (ctx, "mode") if ctx.starts_with("manifest `inputs[") => {
                    " (sheet replacement is automatic when `sheet` matches an existing sheet by ID or title)"
                }
                (ctx, "kind") if ctx.starts_with("manifest `exports[") => {
                    " (use `format` with `curvebook`, `curvebook.d`, `svg`, `pdf`, `emf`, `png`, `tiff`, `csv`, `tsv`, or `json`)"
                }
                _ => "",
            };
            return Err(CliError::Schema(format!("{context} contains unknown field `{key}`{hint}")));
        }
    }
    Ok(())
}

fn cli_error_as_schema(err: CliError) -> CliError {
    match err {
        CliError::Usage(msg) | CliError::Schema(msg) => CliError::Schema(msg),
        other => other,
    }
}

fn opt_obj_str<'a>(obj: &'a Map<String, Value>, keys: &[&str], context: &str) -> Result<Option<&'a str>, CliError> {
    for key in keys {
        if let Some(val) = obj.get(*key) {
            match val {
                Value::String(s) => return Ok(Some(s.as_str())),
                Value::Null => continue,
                _ => return Err(CliError::Schema(format!("{context} field `{key}` must be a string"))),
            }
        }
    }
    Ok(None)
}

fn parse_runner_args(args: &[String], usage: &str) -> Result<RunnerArgs, CliError> {
    let mut source_path = None;
    let mut workdir = None;
    let mut json = false;
    let mut idx = 0;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--json" {
            json = true;
            idx += 1;
        } else if arg == "--workdir" {
            idx += 1;
            let dir = args.get(idx).ok_or_else(|| CliError::Usage(String::from("--workdir requires a directory path")))?;
            workdir = Some(dir.clone());
            idx += 1;
        } else if let Some(dir) = arg.strip_prefix("--workdir=") {
            workdir = Some(dir.to_owned());
            idx += 1;
        } else if arg == "-" {
            if source_path.is_none() {
                source_path = Some(arg.clone());
                idx += 1;
            } else {
                return Err(CliError::Usage(String::from(usage)));
            }
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}`\n\n{usage}")));
        } else if source_path.is_none() {
            source_path = Some(arg.clone());
            idx += 1;
        } else {
            return Err(CliError::Usage(String::from(usage)));
        }
    }

    let Some(source_path) = source_path else {
        return Err(CliError::Usage(String::from(usage)));
    };

    Ok(RunnerArgs { source_path, workdir, json })
}

#[cfg(test)]
mod tests {
    use curvebook_pdf::PdfVersion;
    use serde_json::json;

    use super::{parse_batch_manifest, parse_manifest_exports};
    use crate::error::CliError;
    use crate::sandbox::{PathResolver, WorkdirGuard};

    #[test]
    fn manifest_exports_parse_pdf_version_and_enforce_workdir_on_ttf_font() {
        let dir = std::env::temp_dir().join(format!("cb_batch_parse_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        let guard = WorkdirGuard::new(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(guard);

        let valid = json!([
            { "format": "pdf", "out": "out/a.pdf", "pdf_version": "1.4", "ttf_font": "fonts/custom.ttf" },
            { "format": "pdf", "out": "out/b.pdf" }
        ]);
        let parsed = parse_manifest_exports(Some(&valid), &resolver).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].pdf_version, PdfVersion::Pdf14);
        assert!(parsed[0].ttf_font_path.as_ref().is_some_and(|p| p.ends_with("fonts/custom.ttf")));
        assert_eq!(parsed[1].pdf_version, PdfVersion::Pdf17);
        assert!(parsed[1].ttf_font_path.is_none());

        let escape = json!([
            { "format": "pdf", "out": "out/esc.pdf", "ttf_font": "../outside.ttf" }
        ]);
        assert!(matches!(parse_manifest_exports(Some(&escape), &resolver), Err(CliError::PathEscape(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_manifest_rejects_unknown_fields_with_schema_error() {
        let dir = std::env::temp_dir().join(format!("cb_batch_strict_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(WorkdirGuard::new(&dir).unwrap());

        let stale_export_kind = json!({
            "schema": "curvebook.batch.v1",
            "exports": [{ "kind": "results_csv", "sheet": "4PL Fit", "out": "out/params.txt" }]
        });
        let err = parse_batch_manifest(&stale_export_kind.to_string(), &resolver).err().unwrap();
        assert!(matches!(err, CliError::Schema(ref msg) if msg.contains("`kind`") && msg.contains("`format`")));

        let stale_input_mode = json!({
            "schema": "curvebook.batch.v1",
            "inputs": [{ "sheet": "Dose Response", "csv": "plates/plate_01.csv", "mode": "replace_data" }]
        });
        let err = parse_batch_manifest(&stale_input_mode.to_string(), &resolver).err().unwrap();
        assert!(matches!(err, CliError::Schema(ref msg) if msg.contains("`mode`")));

        let unknown_root = json!({
            "schema": "curvebook.batch.v1",
            "unexpected": true
        });
        assert!(matches!(parse_batch_manifest(&unknown_root.to_string(), &resolver), Err(CliError::Schema(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_manifest_rejects_legacy_and_future_schema_versions() {
        let dir = std::env::temp_dir().join(format!("cb_batch_legacy_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(WorkdirGuard::new(&dir).unwrap());

        for bad_schema in ["lumencraft.batch.v1", "curvebook.batch.v2", "curvebook.batch.v0", ""] {
            let manifest = json!({
                "schema": bad_schema,
                "exports": []
            });
            let err = parse_batch_manifest(&manifest.to_string(), &resolver).err().unwrap();
            assert!(matches!(err, CliError::Schema(ref msg) if msg.contains("curvebook.batch.v1")), "unexpected error for `{bad_schema}`: {err:?}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
