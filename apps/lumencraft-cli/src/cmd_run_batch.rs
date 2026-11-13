//! `lumencraft-cli run <script>` and `lumencraft-cli batch <manifest>` execution runners.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use lumencraft_doc::ProjectDocument;
use lumencraft_engine::Session;
use lumencraft_format::instantiate_template_bytes;
use serde_json::{Map, Value, json};

use crate::cmd_convert::{
    ExportFormat, ExportRequest, detect_export_format_from_path, export_from_document, load_project_from_bytes, parse_export_format,
};
use crate::error::CliError;
use crate::sandbox::{PathResolver, WorkdirGuard};

const BATCH_SCHEMA_V1: &str = "lumencraft.batch.v1";

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
}

struct ParsedManifest {
    template_path: Option<PathBuf>,
    project_path: Option<PathBuf>,
    inputs: Vec<BatchInputSpec>,
    commands: Vec<ParsedCommand>,
    exports: Vec<BatchExportSpec>,
}

/// Execute `lumencraft-cli run <script> [--workdir <dir>] [--json]`.
pub fn run_script(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_runner_args(args, "usage: lumencraft-cli run <script> [--workdir <dir>] [--json]")?;
    let resolver = build_resolver(parsed.workdir.as_deref(), Path::new("."))?;
    let raw_text = read_text_source(&parsed.source_path)?;
    let commands = parse_script_commands(&raw_text, &resolver)?;

    let mut session = Session::new();
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
}

/// Execute `lumencraft-cli batch <manifest> [--workdir <dir>] [--json]`.
pub fn run_batch(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_runner_args(args, "usage: lumencraft-cli batch <manifest> [--workdir <dir>] [--json]")?;
    let manifest_base = if parsed.source_path == "-" {
        Path::new(".").to_path_buf()
    } else {
        Path::new(&parsed.source_path).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new(".")).to_path_buf()
    };
    let resolver = build_resolver(parsed.workdir.as_deref(), &manifest_base)?;
    let raw_text = read_text_source(&parsed.source_path)?;
    let manifest = parse_batch_manifest(&raw_text, &resolver)?;

    let mut session = open_manifest_base_session(&manifest)?;
    load_manifest_inputs(&mut session, &manifest.inputs)?;
    let results = execute_commands(&mut session, &manifest.commands, parsed.workdir.is_some())?;
    session.recalculate_all()?;

    let mut export_summaries = Vec::with_capacity(manifest.exports.len());
    for exp in &manifest.exports {
        let summary = export_from_document(&ExportRequest {
            doc: session.document(),
            out_path: &exp.out_path,
            format: exp.format,
            sheet: exp.sheet.as_deref(),
            dpi: exp.dpi,
            pretty: exp.pretty,
            tab_index: exp.tab_index,
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
}

fn build_resolver(workdir: Option<&str>, fallback_base: &Path) -> Result<PathResolver, CliError> {
    match workdir {
        Some(dir) => Ok(PathResolver::Sandboxed(WorkdirGuard::new(Path::new(dir))?)),
        None => Ok(PathResolver::Unconstrained { base_dir: fallback_base.to_path_buf() }),
    }
}

fn read_text_source(source: &str) -> Result<String, CliError> {
    if source == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|err| CliError::Runtime(format!("failed to read stdin: {err}")))?;
        return Ok(buf);
    }
    fs::read_to_string(source).map_err(|err| CliError::Runtime(format!("failed to read `{source}`: {err}")))
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
        let Some(obj) = item.as_object() else {
            return Err(CliError::Schema(format!("manifest `inputs[{idx}]` must be an object")));
        };
        let raw_path = opt_obj_str(obj, &["csv", "tsv", "path", "file"])
            .ok_or_else(|| CliError::Schema(format!("manifest `inputs[{idx}]` requires a string `csv` (or `path`) field")))?;
        let csv_path = resolver.resolve(raw_path)?;
        out.push(BatchInputSpec {
            csv_path,
            sheet: opt_obj_str(obj, &["sheet", "sheet_id", "sheetId"]).map(str::to_owned),
            kind: opt_obj_str(obj, &["kind", "table_kind", "tableKind"]).map(str::to_owned),
            subcolumn_mode: opt_obj_str(obj, &["subcolumn_mode", "subcolumnMode"]).map(str::to_owned),
            delimiter: opt_obj_str(obj, &["delimiter"]).map(str::to_owned),
            decimal_separator: opt_obj_str(obj, &["decimal_separator", "decimalSeparator"]).map(str::to_owned),
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
    arr.iter().enumerate().map(|(idx, item)| parse_command_value(idx, item.clone(), resolver)).collect()
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
        let Some(obj) = item.as_object() else {
            return Err(CliError::Schema(format!("manifest `exports[{idx}]` must be an object")));
        };
        let raw_out = opt_obj_str(obj, &["out", "path"])
            .ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}]` requires string `out` (or `path`)")))?;
        let out_path = resolver.resolve(raw_out)?;
        let format = match obj.get("format") {
            Some(Value::String(s)) => parse_export_format(s).map_err(|e| CliError::Schema(e.to_string()))?,
            None | Some(Value::Null) => detect_export_format_from_path(&out_path).map_err(|e| CliError::Schema(e.to_string()))?,
            Some(_) => return Err(CliError::Schema(format!("manifest `exports[{idx}].format` must be a string"))),
        };
        let dpi = match obj.get("dpi") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => {
                let raw_u64 = n.as_u64().ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].dpi` must be a positive integer")))?;
                let val_u16 = u16::try_from(raw_u64)
                    .ok()
                    .filter(|&d| d > 0)
                    .ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].dpi` out of range")))?;
                Some(val_u16)
            }
            Some(_) => return Err(CliError::Schema(format!("manifest `exports[{idx}].dpi` must be an integer"))),
        };
        let pretty = obj.get("pretty").and_then(Value::as_bool).unwrap_or(true);
        let tab_index = match obj.get("tab_index").or_else(|| obj.get("tabIndex")) {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => {
                let raw_u64 = n.as_u64().ok_or_else(|| CliError::Schema(format!("manifest `exports[{idx}].tab_index` must be an integer")))?;
                let idx_val = usize::try_from(raw_u64).map_err(|_| CliError::Schema(format!("manifest `exports[{idx}].tab_index` out of bounds")))?;
                Some(idx_val)
            }
            Some(_) => return Err(CliError::Schema(format!("manifest `exports[{idx}].tab_index` must be an integer"))),
        };
        out.push(BatchExportSpec {
            out_path,
            format,
            sheet: opt_obj_str(obj, &["sheet", "sheet_id", "sheetId"]).map(str::to_owned),
            dpi,
            pretty,
            tab_index,
        });
    }
    Ok(out)
}

fn open_manifest_base_session(manifest: &ParsedManifest) -> Result<Session, CliError> {
    if let Some(tpl_path) = &manifest.template_path {
        let bytes = fs::read(tpl_path).map_err(|err| CliError::Runtime(format!("failed to read template `{}`: {err}", tpl_path.display())))?;
        let doc = instantiate_template_bytes(&bytes, None)?;
        return Ok(Session::from_document(doc));
    }
    if let Some(proj_path) = &manifest.project_path {
        let bytes = fs::read(proj_path).map_err(|err| CliError::Runtime(format!("failed to read project `{}`: {err}", proj_path.display())))?;
        let loaded = load_project_from_bytes(proj_path, &bytes, false)?;
        return Ok(Session::from_document(loaded.doc));
    }
    Ok(Session::new())
}

fn load_manifest_inputs(session: &mut Session, inputs: &[BatchInputSpec]) -> Result<(), CliError> {
    for (idx, input) in inputs.iter().enumerate() {
        let csv_text = fs::read_to_string(&input.csv_path)
            .map_err(|err| CliError::Runtime(format!("failed to read input `{}`: {err}", input.csv_path.display())))?;

        let matched_sheet_id = resolve_existing_data_sheet_id(session.document(), input.sheet.as_deref());
        let target_sheet_id = matched_sheet_id.clone().or_else(|| input.sheet.clone()).unwrap_or_else(|| format!("data_{}", idx + 1));

        let mut params = Map::new();
        params.insert(String::from("csv"), Value::String(csv_text));
        params.insert(String::from("sheet_id"), Value::String(target_sheet_id));
        params.insert(String::from("replace"), Value::Bool(matched_sheet_id.is_some()));
        if let Some(kind) = &input.kind {
            params.insert(String::from("kind"), Value::String(kind.clone()));
        } else if matched_sheet_id.is_none() {
            params.insert(String::from("kind"), Value::String(String::from("xy")));
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
        let outcome = session
            .execute(&entry.cmd, params)
            .map_err(|err| CliError::Runtime(format!("command #{} (`{}`) failed: {err}", entry.index + 1, entry.cmd)))?;
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
            | "export.png"
            | "export.tiff"
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

fn opt_obj_str<'a>(obj: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(Value::as_str) {
            return Some(s);
        }
    }
    None
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
