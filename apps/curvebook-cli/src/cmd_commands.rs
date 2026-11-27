//! Engine command discovery (`commands`) and direct schema-typed command execution (`<command-id>`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use curvebook_engine::cmd::{Caller, CommandGroup, CommandSpec, registry};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::registry::{ExportOptions, ExportSource, FormatKind, FormatRegistry};
use serde_json::{Map, Value, json};

use crate::cmd_convert::{
    ExportFormat, ExportRequest, detect_export_format_from_path, export_from_document, is_unpacked_project_path, load_project_from_path,
    run_guarded_cli,
};
use crate::error::CliError;
use crate::sandbox::{PathResolver, WorkdirGuard};

/// Parsed flags for `curvebook-cli commands [--group <g>] [--json]`.
struct ListCommandsArgs {
    group: Option<CommandGroup>,
    json: bool,
    help: bool,
}

/// Parsed wrapper + parameter flags for direct `curvebook-cli <command-id> ...` invocation.
struct DirectCommandArgs {
    in_path: Option<String>,
    out_path: Option<String>,
    workdir: Option<String>,
    json: bool,
    help: bool,
    params: Value,
    warnings: Vec<String>,
}

/// Schema type classification extracted from a JSON Schema property object.
#[derive(Debug, Clone)]
enum PropKind {
    Bool,
    Integer,
    Number,
    String,
    Array { item: Option<Box<PropKind>> },
    Object,
    AnyJson,
    Multi { options: Vec<PropKind>, allow_null: bool },
}

#[derive(Debug, Clone)]
struct PropertySchema {
    canonical: String,
    nullable: bool,
    kind: PropKind,
}

/// Execute `curvebook-cli commands [--group <g>] [--json]`.
pub fn run_commands(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_list_commands_args(args)?;
    if parsed.help {
        print_commands_usage();
        return Ok(());
    }

    let reg = registry();
    let specs: Vec<&CommandSpec> = match parsed.group {
        Some(group) => reg.specs_by_group(group),
        None => reg.specs().collect(),
    };

    if parsed.json {
        let items: Vec<Value> = specs
            .iter()
            .map(|spec| {
                json!({
                    "group": spec.group.as_str(),
                    "id": spec.id,
                    "label": spec.label,
                    "shortcut": spec.shortcut,
                    "effects": spec.effects.flag_names(),
                    "summary": spec.summary,
                    "beginner": spec.beginner,
                    "input_schema": (spec.input_schema)(),
                })
            })
            .collect();
        let payload = json!({
            "ok": true,
            "group": parsed.group.map(CommandGroup::as_str),
            "count": items.len(),
            "commands": items,
        });
        let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
        println!("{text}");
    } else {
        for spec in &specs {
            let effects = spec.effects.flag_names();
            let effects_str = if effects.is_empty() { String::from("NONE") } else { effects.join("|") };
            let shortcut_part = if spec.shortcut.is_empty() { String::new() } else { format!(" [{}]", spec.shortcut) };
            println!("[{}] {} — {}{} ({effects_str})", spec.group.as_str(), spec.id, spec.label, shortcut_part);
            println!("  summary: {}", spec.summary);
            if !spec.beginner.is_empty() {
                println!("  guide:   {}", spec.beginner);
            }
        }
        println!("total: {} command(s)", specs.len());
    }
    Ok(())
}

/// Execute a registered engine command `<command-id>` directly with schema-typed CLI flags.
pub fn run_direct_command(spec: &CommandSpec, args: &[String]) -> Result<(), CliError> {
    let cancelled = AtomicBool::new(false);
    run_direct_command_with_cancel(spec, args, &cancelled)
}

/// Execute a registered engine command with an explicit cancellation token.
pub fn run_direct_command_with_cancel(spec: &CommandSpec, args: &[String], cancelled: &AtomicBool) -> Result<(), CliError> {
    let schema = (spec.input_schema)();
    let mut parsed = parse_direct_command_args(spec, &schema, args)?;
    if parsed.help {
        print_direct_command_help(spec, &schema);
        return Ok(());
    }

    let resolver = match parsed.workdir.as_deref() {
        Some(dir) => PathResolver::Sandboxed(WorkdirGuard::new(Path::new(dir))?),
        None => PathResolver::Unconstrained { base_dir: PathBuf::from(".") },
    };

    let resolved_in = parsed.in_path.as_deref().map(|raw| resolver.resolve(raw)).transpose()?;
    let out_target = parsed.out_path.as_deref().map(|raw| resolve_and_validate_out_target(&resolver, raw)).transpose()?;

    resolver.sanitize_params(&mut parsed.params)?;

    let mut session = match resolved_in {
        Some(ref path) => {
            let (loaded, _) = load_project_from_path(path, true)?;
            Session::from_document(loaded.doc)
        }
        None => Session::new(),
    };

    if let Some(dir) = parsed.workdir.as_deref() {
        session.set_path_policy(ExternalPathPolicy::workdir(Path::new(dir))?);
    }

    let outcome = session.execute_with_caller(spec.id, parsed.params, Caller::Cli, cancelled)?;
    let mut warnings = parsed.warnings;
    for w in outcome.warnings {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    for w in &warnings {
        eprintln!("warning: {w}");
    }

    let saved_out = if let Some((ref out_path, format)) = out_target {
        run_guarded_cli("cli.direct.export_out", || persist_session_output(&session, out_path, format))?;
        Some(out_path.to_string_lossy().into_owned())
    } else {
        None
    };

    if parsed.json {
        let mut payload = json!({
            "ok": true,
            "command_id": spec.id,
            "mutated": outcome.mutated,
            "message": outcome.message,
            "result": outcome.payload,
            "out": saved_out,
        });
        if !warnings.is_empty()
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert(String::from("warnings"), json!(warnings));
        }
        let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
        println!("{text}");
    } else if let Some(path) = saved_out {
        println!("{} (saved to {path})", outcome.message);
    } else {
        println!("{}", outcome.message);
    }
    Ok(())
}

fn print_commands_usage() {
    println!("usage: curvebook-cli commands [--group <group>] [--json]");
    let groups: Vec<&str> = CommandGroup::all().iter().map(|g| g.as_str()).collect();
    println!("groups: {}", groups.join(", "));
    println!("docs: {}#cli-commands", crate::cli_catalog::CLI_REGISTRY_DOCS_URL);
}

fn parse_list_commands_args(args: &[String]) -> Result<ListCommandsArgs, CliError> {
    let mut group = None;
    let mut json = false;
    let mut help = false;
    let mut idx = 0usize;

    while idx < args.len() {
        let Some(arg) = args.get(idx).map(String::as_str) else {
            break;
        };
        if arg == "--json" {
            json = true;
        } else if arg == "--help" || arg == "-h" {
            help = true;
        } else if let Some(val) = arg.strip_prefix("--group=") {
            group = Some(parse_command_group(val)?);
        } else if arg == "--group" {
            idx += 1;
            let val = args.get(idx).ok_or_else(|| CliError::Usage(String::from("--group requires a group name")))?;
            group = Some(parse_command_group(val)?);
        } else {
            return Err(CliError::Usage(format!("unexpected argument `{arg}` for `commands`")));
        }
        idx += 1;
    }

    Ok(ListCommandsArgs { group, json, help })
}

fn parse_command_group(raw: &str) -> Result<CommandGroup, CliError> {
    let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
    for &group in CommandGroup::all() {
        if group.as_str() == normalized {
            return Ok(group);
        }
    }
    let valid: Vec<&str> = CommandGroup::all().iter().map(|g| g.as_str()).collect();
    Err(CliError::Usage(format!("unknown command group `{raw}` (expected one of: {})", valid.join(", "))))
}

fn print_direct_command_help(spec: &CommandSpec, schema: &Value) {
    println!("{} — {} [{}]", spec.id, spec.label, spec.group.as_str());
    println!("  {}", spec.summary);
    if !spec.beginner.is_empty() {
        println!("  {}", spec.beginner);
    }
    println!("\nWrapper flags: --in <file>  --out <file>  --workdir <dir>  --json");
    if let Some(props) = schema.get("properties").and_then(Value::as_object)
        && !props.is_empty()
    {
        println!("Command flags:");
        for (name, prop_schema) in props {
            let flag = format!("--{}", name.replace('_', "-"));
            let desc = prop_schema
                .get("description")
                .and_then(Value::as_str)
                .or_else(|| {
                    prop_schema
                        .get("anyOf")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.iter().find_map(|s| s.get("description").and_then(Value::as_str)))
                })
                .unwrap_or("");
            println!("  {flag:<24} {desc}");
        }
    }
}

fn parse_direct_command_args(spec: &CommandSpec, schema: &Value, args: &[String]) -> Result<DirectCommandArgs, CliError> {
    let prop_table = build_property_lookup(schema);
    let mut in_path = None;
    let mut out_path = None;
    let mut workdir = None;
    let mut json = false;
    let mut help = false;
    let mut params = Map::new();
    let mut warnings = Vec::new();

    let mut idx = 0usize;
    while idx < args.len() {
        let Some(arg) = args.get(idx).map(String::as_str) else {
            break;
        };
        if handle_wrapper_flag(args, &mut idx, &mut in_path, &mut out_path, &mut workdir, &mut json, &mut help)? {
            idx += 1;
            continue;
        }
        let Some(flag_body) = arg.strip_prefix("--") else {
            return Err(CliError::Usage(format!("unexpected positional argument `{arg}` for `{}`", spec.id)));
        };
        let (flag_name, inline_val) = match flag_body.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (flag_body, None),
        };
        if flag_name.is_empty() {
            return Err(CliError::Usage(format!("empty flag `--` for `{}`", spec.id)));
        }

        if let Some((prop, negated, deprecated_alias)) = resolve_flag_property(&prop_table, flag_name) {
            if let Some(alias) = deprecated_alias {
                let msg = format!("parameter `{alias}` is deprecated for `{}`; use canonical `{}` instead", spec.id, prop.canonical);
                if !warnings.contains(&msg) {
                    warnings.push(msg);
                }
            }
            let parsed_val = parse_flag_value(prop, negated, inline_val, args, &mut idx, flag_name)?;
            merge_param_value(&mut params, prop, parsed_val)?;
        } else {
            return Err(CliError::Usage(format!("unknown flag `--{flag_name}` for `{}`", spec.id)));
        }
        idx += 1;
    }

    Ok(DirectCommandArgs { in_path, out_path, workdir, json, help, params: Value::Object(params), warnings })
}

fn handle_wrapper_flag(
    args: &[String],
    idx: &mut usize,
    in_path: &mut Option<String>,
    out_path: &mut Option<String>,
    workdir: &mut Option<String>,
    json: &mut bool,
    help: &mut bool,
) -> Result<bool, CliError> {
    let Some(arg) = args.get(*idx).map(String::as_str) else {
        return Ok(false);
    };
    match arg {
        "--json" => *json = true,
        "--help" | "-h" => *help = true,
        "--in" => *in_path = Some(next_flag_value(args, idx, "--in requires a file path")?),
        "--out" => *out_path = Some(next_flag_value(args, idx, "--out requires a file path")?),
        "--workdir" => *workdir = Some(next_flag_value(args, idx, "--workdir requires a directory path")?),
        _ if arg.starts_with("--in=") => *in_path = Some(non_empty_inline(arg, "--in=")?),
        _ if arg.starts_with("--out=") => *out_path = Some(non_empty_inline(arg, "--out=")?),
        _ if arg.starts_with("--workdir=") => *workdir = Some(non_empty_inline(arg, "--workdir=")?),
        _ => return Ok(false),
    }
    Ok(true)
}

fn next_flag_value(args: &[String], idx: &mut usize, missing_msg: &str) -> Result<String, CliError> {
    *idx += 1;
    let val = args.get(*idx).ok_or_else(|| CliError::Usage(String::from(missing_msg)))?;
    if val.trim().is_empty() {
        return Err(CliError::Usage(String::from(missing_msg)));
    }
    Ok(val.clone())
}

fn non_empty_inline(arg: &str, prefix: &str) -> Result<String, CliError> {
    let val = arg.strip_prefix(prefix).unwrap_or("");
    if val.trim().is_empty() {
        let flag = prefix.trim_end_matches('=');
        return Err(CliError::Usage(format!("{flag} requires a non-empty value")));
    }
    Ok(val.to_owned())
}

#[derive(Debug, Clone)]
struct FlagTarget {
    prop_idx: usize,
    deprecated_alias: Option<String>,
}

struct PropertyLookup {
    props: Vec<PropertySchema>,
    by_token: BTreeMap<String, FlagTarget>,
}

fn build_property_lookup(schema: &Value) -> PropertyLookup {
    let mut props = Vec::new();
    let mut by_token = BTreeMap::new();

    if let Some(prop_map) = schema.get("properties").and_then(Value::as_object) {
        for (name, prop_schema) in prop_map {
            let (kind, nullable) = inspect_schema_kind(prop_schema);
            let idx = props.len();
            props.push(PropertySchema { canonical: name.clone(), nullable, kind });
            register_flag_tokens(&mut by_token, name, idx, None);
        }
    }

    for alias_key in ["x-deprecated-aliases", "x-aliases"] {
        if let Some(alias_map) = schema.get(alias_key).and_then(Value::as_object) {
            for (canonical, aliases) in alias_map {
                let Some(idx) = by_token.get(canonical).or_else(|| by_token.get(&normalize_token(canonical))).map(|t| t.prop_idx) else {
                    continue;
                };
                if let Some(list) = aliases.as_array() {
                    for alias in list.iter().filter_map(Value::as_str) {
                        if alias != canonical {
                            register_flag_tokens(&mut by_token, alias, idx, Some(alias));
                        }
                    }
                }
            }
        }
    }

    PropertyLookup { props, by_token }
}

fn register_flag_tokens(map: &mut BTreeMap<String, FlagTarget>, raw_name: &str, prop_idx: usize, deprecated: Option<&str>) {
    let make_target = || FlagTarget { prop_idx, deprecated_alias: deprecated.map(str::to_owned) };
    map.entry(raw_name.to_owned()).or_insert_with(make_target);
    map.entry(normalize_token(raw_name)).or_insert_with(make_target);
    map.entry(camel_to_snake_lower(raw_name)).or_insert_with(make_target);
}

fn normalize_token(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('-', "_")
}

fn camel_to_snake_lower(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (pos, ch) in raw.chars().enumerate() {
        if ch == '-' {
            out.push('_');
        } else if ch.is_ascii_uppercase() {
            if pos > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

fn resolve_flag_property<'a>(lookup: &'a PropertyLookup, flag_name: &str) -> Option<(&'a PropertySchema, bool, Option<&'a str>)> {
    if let Some(target) = lookup.by_token.get(flag_name).or_else(|| lookup.by_token.get(&normalize_token(flag_name))) {
        return lookup.props.get(target.prop_idx).map(|p| (p, false, target.deprecated_alias.as_deref()));
    }
    if let Some(pos_flag) = flag_name.strip_prefix("no-")
        && let Some(target) = lookup.by_token.get(pos_flag).or_else(|| lookup.by_token.get(&normalize_token(pos_flag)))
        && let Some(prop) = lookup.props.get(target.prop_idx)
        && kind_accepts_bool(&prop.kind)
    {
        return Some((prop, true, target.deprecated_alias.as_deref()));
    }
    None
}

fn kind_accepts_bool(kind: &PropKind) -> bool {
    match kind {
        PropKind::Bool | PropKind::AnyJson => true,
        PropKind::Multi { options, .. } => options.iter().any(kind_accepts_bool),
        _ => false,
    }
}

fn inspect_schema_kind(schema: &Value) -> (PropKind, bool) {
    if let Some(branches) = schema.get("anyOf").or_else(|| schema.get("oneOf")).and_then(Value::as_array) {
        return inspect_union_branches(branches);
    }

    if let Some(types) = schema.get("type").and_then(Value::as_array) {
        let mut nullable = false;
        let mut options = Vec::new();
        for t in types.iter().filter_map(Value::as_str) {
            if t == "null" {
                nullable = true;
            } else {
                options.push(kind_for_type_name(t, schema));
            }
        }
        return collapse_options(options, nullable);
    }

    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if ty == "null" {
            return (PropKind::AnyJson, true);
        }
        return (kind_for_type_name(ty, schema), false);
    }

    (PropKind::AnyJson, false)
}

fn inspect_union_branches(branches: &[Value]) -> (PropKind, bool) {
    let mut nullable = false;
    let mut options = Vec::new();
    for branch in branches {
        if branch.get("type").and_then(Value::as_str) == Some("null") {
            nullable = true;
            continue;
        }
        let (sub_kind, sub_null) = inspect_schema_kind(branch);
        nullable |= sub_null;
        options.push(sub_kind);
    }
    collapse_options(options, nullable)
}

fn collapse_options(mut options: Vec<PropKind>, nullable: bool) -> (PropKind, bool) {
    match options.len() {
        0 => (PropKind::AnyJson, nullable),
        1 => (options.remove(0), nullable),
        _ => (PropKind::Multi { options, allow_null: nullable }, nullable),
    }
}

fn kind_for_type_name(ty: &str, schema: &Value) -> PropKind {
    match ty {
        "boolean" => PropKind::Bool,
        "integer" => PropKind::Integer,
        "number" => PropKind::Number,
        "string" => PropKind::String,
        "object" => PropKind::Object,
        "array" => {
            let item = schema.get("items").filter(|v| !v.is_null()).map(|v| Box::new(inspect_schema_kind(v).0));
            PropKind::Array { item }
        }
        _ => PropKind::AnyJson,
    }
}

fn parse_flag_value(
    prop: &PropertySchema,
    negated: bool,
    inline_val: Option<&str>,
    args: &[String],
    idx: &mut usize,
    flag_name: &str,
) -> Result<Value, CliError> {
    if negated {
        if let Some(raw) = inline_val {
            let parsed = parse_bool_literal(raw, flag_name)?;
            return Ok(Value::Bool(!parsed));
        }
        return Ok(Value::Bool(false));
    }

    if let PropKind::Bool = prop.kind {
        return parse_bool_flag(inline_val, args, idx, flag_name);
    }

    let raw = match inline_val {
        Some(v) => v.to_owned(),
        None => {
            *idx += 1;
            args.get(*idx).cloned().ok_or_else(|| CliError::Usage(format!("flag `--{flag_name}` requires a value")))?
        }
    };

    parse_typed_scalar_or_composite(&prop.kind, prop.nullable, &raw, flag_name)
}

fn parse_bool_flag(inline_val: Option<&str>, args: &[String], idx: &mut usize, flag_name: &str) -> Result<Value, CliError> {
    if let Some(raw) = inline_val {
        return Ok(Value::Bool(parse_bool_literal(raw, flag_name)?));
    }
    if let Some(next) = args.get(*idx + 1)
        && !next.starts_with('-')
    {
        *idx += 1;
        return Ok(Value::Bool(parse_bool_literal(next, flag_name)?));
    }
    Ok(Value::Bool(true))
}

fn parse_bool_literal(raw: &str, flag_name: &str) -> Result<bool, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(CliError::Usage(format!("invalid boolean `{raw}` for `--{flag_name}` (expected true/false)"))),
    }
}

fn parse_typed_scalar_or_composite(kind: &PropKind, nullable: bool, raw: &str, flag_name: &str) -> Result<Value, CliError> {
    if nullable && raw == "null" && !matches!(kind, PropKind::String) {
        return Ok(Value::Null);
    }
    match kind {
        PropKind::Bool => Ok(Value::Bool(parse_bool_literal(raw, flag_name)?)),
        PropKind::Integer => {
            let trimmed = raw.trim();
            if let Ok(n) = trimmed.parse::<i64>() {
                Ok(Value::Number(serde_json::Number::from(n)))
            } else if let Ok(n) = trimmed.parse::<u64>() {
                Ok(Value::Number(serde_json::Number::from(n)))
            } else {
                Err(CliError::Usage(format!("invalid integer `{raw}` for `--{flag_name}`")))
            }
        }
        PropKind::Number => {
            let f = raw.trim().parse::<f64>().map_err(|_| CliError::Usage(format!("invalid number `{raw}` for `--{flag_name}`")))?;
            let num = serde_json::Number::from_f64(f).ok_or_else(|| CliError::Usage(format!("non-finite number `{raw}` for `--{flag_name}`")))?;
            Ok(Value::Number(num))
        }
        PropKind::String => Ok(Value::String(raw.to_owned())),
        PropKind::Object => {
            let val: Value = serde_json::from_str(raw).map_err(|err| CliError::Usage(format!("invalid JSON object for `--{flag_name}`: {err}")))?;
            if !val.is_object() {
                return Err(CliError::Usage(format!("flag `--{flag_name}` expects a JSON object")));
            }
            Ok(val)
        }
        PropKind::AnyJson => Ok(parse_flexible_json(raw)),
        PropKind::Array { item, .. } => parse_array_value(item.as_deref(), raw, flag_name),
        PropKind::Multi { options, allow_null } => parse_multi_branch(options, *allow_null, raw, flag_name),
    }
}

fn parse_flexible_json(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        return parsed;
    }
    Value::String(raw.to_owned())
}

fn parse_array_value(item_kind: Option<&PropKind>, raw: &str, flag_name: &str) -> Result<Value, CliError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        let arr: Value = serde_json::from_str(trimmed).map_err(|err| CliError::Usage(format!("invalid JSON array for `--{flag_name}`: {err}")))?;
        if !arr.is_array() {
            return Err(CliError::Usage(format!("flag `--{flag_name}` expects a JSON array")));
        }
        return Ok(arr);
    }

    let item_kind_ref = item_kind.unwrap_or(&PropKind::AnyJson);
    let can_split_csv = !trimmed.starts_with('{')
        && trimmed.contains(',')
        && match item_kind_ref {
            PropKind::Bool | PropKind::Integer | PropKind::Number | PropKind::String | PropKind::AnyJson => true,
            PropKind::Multi { options, .. } => {
                options.iter().any(|opt| matches!(opt, PropKind::Bool | PropKind::Integer | PropKind::Number | PropKind::String | PropKind::AnyJson))
            }
            PropKind::Object | PropKind::Array { .. } => false,
        };

    if can_split_csv {
        let mut items = Vec::new();
        for part in trimmed.split(',') {
            items.push(parse_typed_scalar_or_composite(item_kind_ref, false, part.trim(), flag_name)?);
        }
        return Ok(Value::Array(items));
    }

    let single = parse_typed_scalar_or_composite(item_kind_ref, false, raw, flag_name)?;
    Ok(Value::Array(vec![single]))
}

fn parse_multi_branch(options: &[PropKind], allow_null: bool, raw: &str, flag_name: &str) -> Result<Value, CliError> {
    if allow_null && raw == "null" {
        return Ok(Value::Null);
    }
    let trimmed = raw.trim();
    if trimmed.starts_with('[')
        && options.iter().any(|opt| matches!(opt, PropKind::Array { .. }))
        && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        && val.is_array()
    {
        return Ok(val);
    }
    if trimmed.starts_with('{')
        && options.iter().any(|opt| matches!(opt, PropKind::Object | PropKind::AnyJson))
        && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        && val.is_object()
    {
        return Ok(val);
    }
    for opt in options {
        if !matches!(opt, PropKind::String | PropKind::AnyJson)
            && let Ok(val) = parse_typed_scalar_or_composite(opt, false, raw, flag_name)
        {
            return Ok(val);
        }
    }
    for opt in options {
        if let Ok(val) = parse_typed_scalar_or_composite(opt, false, raw, flag_name) {
            return Ok(val);
        }
    }
    Err(CliError::Usage(format!("invalid value `{raw}` for `--{flag_name}`")))
}

fn merge_param_value(params: &mut Map<String, Value>, prop: &PropertySchema, incoming: Value) -> Result<(), CliError> {
    if matches!(prop.kind, PropKind::Array { .. })
        && let Some(Value::Array(existing)) = params.get_mut(&prop.canonical)
        && let Value::Array(extra) = incoming
    {
        existing.extend(extra);
        return Ok(());
    }
    params.insert(prop.canonical.clone(), incoming);
    Ok(())
}

fn resolve_and_validate_out_target(resolver: &PathResolver, raw_out: &str) -> Result<(PathBuf, Option<ExportFormat>), CliError> {
    let out_path = resolver.resolve(raw_out)?;
    if out_path.is_dir() && !is_unpacked_project_path(&out_path) {
        return Err(CliError::Usage(format!("output path `{}` is a directory without a `.curvebook.d` suffix", out_path.display())));
    }
    if let Some(exporter) = FormatRegistry::default_registry().exporter_for_path(&out_path)
        && exporter.descriptor().kind == FormatKind::Template
    {
        return Ok((out_path, None));
    }
    let fmt = detect_export_format_from_path(&out_path)?;
    Ok((out_path, Some(fmt)))
}

fn persist_session_output(session: &Session, out_path: &Path, format: Option<ExportFormat>) -> Result<(), CliError> {
    if is_unpacked_project_path(out_path) || format == Some(ExportFormat::CurveBookDir) {
        let reg = FormatRegistry::default_registry();
        let exporter = reg
            .exporter_by_format("curvebook.d")
            .ok_or_else(|| CliError::Runtime(String::from("missing `curvebook.d` exporter in FormatRegistry")))?;
        exporter.export_path(ExportSource::Document(session.document()), out_path, &ExportOptions::default())?;
        return Ok(());
    }

    let reg = FormatRegistry::default_registry();
    if let Some(exporter) = reg.exporter_for_path(out_path)
        && exporter.descriptor().id != "curvebook.d"
        && matches!(exporter.descriptor().kind, FormatKind::Archive | FormatKind::Template | FormatKind::DelimitedText | FormatKind::ResultTable)
    {
        exporter.export_path(ExportSource::Document(session.document()), out_path, &ExportOptions { pretty: true, ..ExportOptions::default() })?;
        return Ok(());
    }

    let fmt = format.ok_or_else(|| CliError::Usage(format!("unsupported output format for `{}`", out_path.display())))?;
    export_from_document(&ExportRequest {
        doc: session.document(),
        out_path,
        format: fmt,
        sheet: None,
        dpi: None,
        pretty: true,
        tab_index: None,
        pdf_version: curvebook_pdf::PdfVersion::default(),
        embedded_truetype_font: None,
        experimental: false,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use curvebook_engine::cmd::registry;
    use serde_json::{Value, json};

    use super::{parse_direct_command_args, run_commands, run_direct_command_with_cancel};
    use crate::error::{CliError, EXIT_INTERRUPTED, EXIT_USAGE_OR_POLICY};

    #[test]
    fn commands_list_accepts_group_filter_and_json() {
        assert!(run_commands(&[String::from("--group"), String::from("sheet"), String::from("--json")]).is_ok());
        assert!(run_commands(&[String::from("--group=project")]).is_ok());
        let bad = run_commands(&[String::from("--group"), String::from("nonexistent")]).unwrap_err();
        assert!(matches!(bad, CliError::Usage(_)));
        assert_eq!(bad.exit_code(), EXIT_USAGE_OR_POLICY);
    }

    #[test]
    fn schema_typed_flag_parsing_handles_kebab_snake_bool_int_array_json() {
        let spec = registry().get("sheet.createData").unwrap();
        let schema = (spec.input_schema)();
        let args: Vec<String> = vec![
            "--sheet-id",
            "dose_1",
            "--title",
            "Dose Curve",
            "--kind",
            "XY",
            "--rows",
            "12",
            "--cols",
            "3",
            "--auto-graph",
            "--auto-info",
            "false",
            "--variables",
            "conc,resp",
            "--x-mode",
            "numbers",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let parsed = parse_direct_command_args(spec, &schema, &args).unwrap();
        assert_eq!(parsed.params["sheet_id"], json!("dose_1"));
        assert_eq!(parsed.params["title"], json!("Dose Curve"));
        assert_eq!(parsed.params["kind"], json!("XY"));
        assert_eq!(parsed.params["rows"], json!(12));
        assert_eq!(parsed.params["cols"], json!(3));
        assert_eq!(parsed.params["auto_graph"], json!(true));
        assert_eq!(parsed.params["auto_info"], json!(false));
        assert_eq!(parsed.params["variables"], json!(["conc", "resp"]));
        assert_eq!(parsed.params["x_mode"], json!("numbers"));

        let bad_int = parse_direct_command_args(spec, &schema, &[String::from("--rows"), String::from("not_a_num")]).err();
        assert!(matches!(bad_int, Some(CliError::Usage(_))));
    }

    #[test]
    fn build_property_lookup_resolves_both_x_aliases_and_x_deprecated_aliases() {
        let spec = registry().get("sheet.createData").unwrap();
        let legacy_schema = json!({
            "type": "object",
            "properties": {
                "sheet_id": { "type": "string" },
                "kind": { "type": "string" }
            },
            "x-aliases": {
                "kind": ["archetype"]
            }
        });
        let deprecated_schema = json!({
            "type": "object",
            "properties": {
                "sheet_id": { "type": "string" },
                "kind": { "type": "string" }
            },
            "x-deprecated-aliases": {
                "kind": ["archetype"]
            }
        });
        let args = vec![String::from("--archetype"), String::from("xy")];
        let parsed_legacy = parse_direct_command_args(spec, &legacy_schema, &args).unwrap();
        let parsed_deprecated = parse_direct_command_args(spec, &deprecated_schema, &args).unwrap();
        assert_eq!(parsed_legacy.params["kind"], json!("xy"));
        assert_eq!(parsed_deprecated.params["kind"], json!("xy"));
        assert_eq!(
            parsed_legacy.warnings,
            vec![String::from("parameter `archetype` is deprecated for `sheet.createData`; use canonical `kind` instead")]
        );
        assert_eq!(parsed_deprecated.warnings, parsed_legacy.warnings);

        let canonical_args = vec![String::from("--sheet-id"), String::from("d1"), String::from("--kind"), String::from("xy")];
        let parsed_canonical = parse_direct_command_args(spec, &deprecated_schema, &canonical_args).unwrap();
        assert!(parsed_canonical.warnings.is_empty(), "canonical flags must emit zero warnings: {:?}", parsed_canonical.warnings);
    }

    #[test]
    fn pre_cancelled_direct_command_maps_to_exit_130() {
        let spec = registry().get("project.inspect").unwrap();
        let cancelled = AtomicBool::new(true);
        let err = run_direct_command_with_cancel(spec, &[String::from("--json")], &cancelled).unwrap_err();
        assert!(matches!(err, CliError::Interrupted(_)));
        assert_eq!(err.exit_code(), EXIT_INTERRUPTED);
        let payload: Value = serde_json::from_str(&err.to_json_string()).unwrap();
        assert_eq!(payload["error"]["code"], json!("cancelled"));
        assert_eq!(payload["error"]["category"], json!("compute"));
        assert_eq!(payload["error"]["retryable"], json!(true));
    }
}
