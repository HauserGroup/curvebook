//! `serde` JSON patch normalizers, mergers, and post-merge hooks for R9, workspace, import, print, and power wizard dialogs.

use curvebook_doc::SheetId;
use curvebook_stats::power::{PowerSolveTarget, PowerTestFamily};
use serde_json::{Map, Value, json};

use super::engine::{insert_nested_field, merge_struct_json};
use super::graph_layout::convert_target_kind_from_val;
use super::value_parsers::{
    norm_key, parse_bool_field, parse_equation_display_mode_str, parse_f64_list_with_locale, parse_finite_f64_with_locale,
    parse_opt_finite_f64_with_locale, parse_opt_nonempty_string, parse_opt_usize_min, parse_plate_values_json, parse_plate_values_text,
    parse_positive_usize_arg, parse_str_field, parse_table_kind_str, parse_u64_val, parse_usize_list, parse_usize_min,
};
use crate::dialogs_canvas::{
    CheckpointsDialogState, CommandPaletteState, ConstantHookPickerState, ConvertArchetypeDialogState, ExcludeWithReasonDialogState, PrintDialogState,
};
use crate::dialogs_r9::{
    InsertModelEquationDialogState, RandomSampleRowsDialogState, RandomizationSchemeKind, RandomizeGroupsDialogState, SimDistKind,
    SimulateDistributionsDialogState, SimulatedDistColumnDraft,
};
use crate::grid_view::{ImportReshapeMode, ImportReshapeState};
use crate::i18n::LocalePreset;
use crate::power_wizard::{PowerEffectInputMode, PowerWizardStage, PowerWizardState};

pub fn patch_print(state: &mut PrintDialogState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut patch = Map::new();
    let mut next_copies: Option<u16> = None;
    let mut next_scale: Option<f64> = None;
    let mut next_printer: Option<Option<String>> = None;

    for (key, val) in fields {
        match key.as_str() {
            "crop_marks" | "print.crop_marks" => {
                insert_nested_field(&mut patch, "job", "crop_marks", json!(parse_bool_field(val, "crop_marks")?));
            }
            "bleed_pt" | "print.bleed_pt" => {
                let bleed = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0..=144.0).contains(&bleed) {
                    return Err(format!("`bleed_pt` ({bleed}) must be within 0.0..=144.0"));
                }
                insert_nested_field(&mut patch, "job", "bleed_pt", json!(bleed));
            }
            "copies" | "print.copies" => {
                let raw = parse_finite_f64_with_locale(val, key, preset)?;
                if raw < 1.0 || raw > 999.0 || (raw - raw.round()).abs() > 1e-9 {
                    return Err(format!("`copies` ({raw}) must be an integer within 1..=999"));
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let count = raw.round() as u16;
                next_copies = Some(count);
            }
            "scale_percent" | "print.scale_percent" => {
                let scale = parse_finite_f64_with_locale(val, key, preset)?;
                if !(10.0..=400.0).contains(&scale) {
                    return Err(format!("`scale_percent` ({scale}) must be within 10.0..=400.0"));
                }
                next_scale = Some(scale);
            }
            "selected_printer" | "printer" | "print.printer" | "print.selected_printer" => {
                next_printer = Some(if val.is_null() { None } else { Some(parse_str_field(val, "printer")?.to_owned()) });
            }
            "title" | "print.title" => {
                let title = if val.is_null() {
                    None
                } else {
                    let s = parse_str_field(val, "title")?.trim();
                    (!s.is_empty()).then(|| s.to_owned())
                };
                insert_nested_field(&mut patch, "job", "title", json!(title));
            }
            "submit_to_os" | "print.submit_to_os" => {
                patch.insert(String::from("submit_to_os"), json!(parse_bool_field(val, "submit_to_os")?));
            }
            other => return Err(format!("unknown Print field `{other}`")),
        }
    }

    merge_struct_json(state, &patch, &["job"])?;
    if let Some(count) = next_copies {
        state.update_copies(count);
    }
    if let Some(scale) = next_scale {
        state.set_scale_percent(scale);
    }
    if let Some(printer) = next_printer {
        state.select_printer(printer);
    }
    Ok(())
}

pub fn patch_convert_archetype(state: &mut ConvertArchetypeDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    let mut next_target_kind = None;
    for (key, val) in fields {
        match key.as_str() {
            "target_kind" | "kind" | "convert.target_kind" => {
                let parsed = convert_target_kind_from_val(val)?;
                if !ConvertArchetypeDialogState::supported_targets().contains(&parsed) {
                    return Err(format!("unsupported conversion target `{}`", val.as_str().unwrap_or("")));
                }
                next_target_kind = Some(parsed);
            }
            "allow_lossy" | "convert.allow_lossy" => {
                patch.insert(String::from("allow_lossy"), json!(parse_bool_field(val, "allow_lossy")?));
            }
            "preview_only" | "convert.preview_only" => {
                patch.insert(String::from("preview_only"), json!(parse_bool_field(val, "preview_only")?));
            }
            other => return Err(format!("unknown Convert Table Archetype field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])?;
    if let Some(target) = next_target_kind {
        state.set_target_kind(target);
    }
    Ok(())
}

pub fn patch_exclude_with_reason(state: &mut ExcludeWithReasonDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "reason_tag" | "exclude.reason_tag" => {
                let tag = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("reason_tag"), json!(tag));
            }
            "note" | "exclude.note" => {
                let note = if val.is_null() { "" } else { val.as_str().ok_or_else(|| format!("`{key}` must be a string"))? };
                patch.insert(String::from("note"), json!(note));
            }
            other => return Err(format!("unknown ExcludeWithReason field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_command_palette(state: &mut CommandPaletteState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "query" | "palette.query" => {
                let q = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("query"), json!(q));
            }
            "selected_index" | "palette.selected_index" => {
                let raw = val.as_u64().ok_or_else(|| format!("`{key}` must be a non-negative integer"))?;
                let idx = usize::try_from(raw).map_err(|_| format!("`{key}` exceeds usize"))?;
                patch.insert(String::from("selected_index"), json!(idx));
            }
            other => return Err(format!("unknown CommandPalette field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_hook_picker(state: &mut ConstantHookPickerState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "search_query" | "hook.search_query" | "hook.search_input" => {
                let q = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("search_query"), json!(q));
            }
            "selected_index" | "hook.selected_index" => {
                let idx = if val.is_null() {
                    None
                } else {
                    let raw = val.as_u64().ok_or_else(|| format!("`{key}` must be a non-negative integer"))?;
                    Some(usize::try_from(raw).map_err(|_| format!("`{key}` exceeds usize"))?)
                };
                patch.insert(String::from("selected_index"), json!(idx));
            }
            other => return Err(format!("unknown ConstantHookPicker field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_checkpoints(state: &mut CheckpointsDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "name" | "checkpoints.name" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("name_draft"), json!(s));
            }
            "note" | "checkpoints.note" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("note_draft"), json!(s));
            }
            other => return Err(format!("unknown checkpoints field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_randomize_groups(state: &mut RandomizeGroupsDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "groups" | "randomize.groups" => {
                let parsed = parse_randomize_groups_val(val)?;
                patch.insert(String::from("groups"), serde_json::to_value(parsed).map_err(|e| e.to_string())?);
            }
            "subject_count" | "n_subjects" | "randomize.subject_count" => {
                patch.insert(String::from("subject_count"), json!(parse_usize_min(val, key, 1)?));
            }
            "subject_ids" | "randomize.subject_ids" => {
                let ids: Vec<String> = if val.is_null() {
                    Vec::new()
                } else if let Some(arr) = val.as_array() {
                    let mut out = Vec::with_capacity(arr.len());
                    for (idx, item) in arr.iter().enumerate() {
                        let raw = item.as_str().ok_or_else(|| format!("`{key}[{idx}]` must be a string"))?.trim();
                        if !raw.is_empty() {
                            out.push(raw.to_owned());
                        }
                    }
                    out
                } else if let Some(s) = val.as_str() {
                    s.split([',', ';', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(ToOwned::to_owned).collect()
                } else {
                    return Err(String::from("`subject_ids` must be an array, string, or null"));
                };
                patch.insert(String::from("subject_ids"), json!(ids));
            }
            "scheme" | "randomize.scheme" => {
                let raw = parse_str_field(val, "scheme")?;
                let scheme = match norm_key(raw).as_str() {
                    "completebalanced" | "complete" | "balanced" => RandomizationSchemeKind::CompleteBalanced,
                    "permutedblock" | "block" => RandomizationSchemeKind::PermutedBlock,
                    _ => return Err(format!("unknown randomization scheme `{raw}`")),
                };
                patch.insert(String::from("scheme"), serde_json::to_value(scheme).map_err(|e| e.to_string())?);
            }
            "block_sizes" | "block_size" | "randomize.block_sizes" => {
                let blocks = if val.is_u64() { vec![parse_usize_min(val, key, 1)?] } else { parse_usize_list(val, key)? };
                if blocks.is_empty() || blocks.contains(&0) {
                    return Err(String::from("`block_sizes` entries must be positive integers"));
                }
                patch.insert(String::from("block_sizes"), json!(blocks));
            }
            "seed" | "randomize.seed" | "rand_groups.seed.val" => {
                patch.insert(String::from("seed"), json!(parse_u64_val(val, key)?));
            }
            "title" | "randomize.title" => {
                let raw = parse_str_field(val, "title")?.trim();
                if !raw.is_empty() {
                    patch.insert(String::from("title"), json!(raw));
                }
            }
            "info_sheet_id" | "target_info_sheet_id" | "randomize.info_sheet_id" => {
                let tid = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(SheetId::from);
                patch.insert(String::from("target_info_sheet_id"), serde_json::to_value(tid).map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown Randomize Groups field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

fn parse_randomize_groups_val(val: &Value) -> Result<Vec<(String, usize)>, String> {
    if let Some(s) = val.as_str() {
        let mut tmp = RandomizeGroupsDialogState::default();
        tmp.set_groups_from_str(s)?;
        return Ok(tmp.groups);
    }
    let Some(arr) = val.as_array() else {
        return Err(String::from("`groups` must be an array or comma-separated string"));
    };
    let mut next = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(label) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            next.push((label.to_owned(), 1));
        } else if let Some(obj) = item.as_object() {
            let label = obj
                .get("label")
                .or_else(|| obj.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| String::from("group object missing non-empty `label`"))?;
            let weight = match obj.get("weight") {
                Some(w) => parse_usize_min(w, "weight", 1)?,
                None => 1,
            };
            next.push((label.to_owned(), weight));
        } else {
            return Err(String::from("`groups` items must be strings or `{label, weight}` objects"));
        }
    }
    if next.is_empty() {
        return Err(String::from("`groups` must contain at least one group"));
    }
    Ok(next)
}

pub fn patch_random_sample_rows(state: &mut RandomSampleRowsDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "source_sheet_id" | "sample.source_sheet_id" => {
                if let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    patch.insert(String::from("source_sheet_id"), json!(s));
                }
            }
            "sample_size" | "sample.sample_size" | "rand_sample.n.val" => {
                patch.insert(String::from("sample_size"), json!(parse_usize_min(val, key, 1)?));
            }
            "seed" | "sample.seed" => {
                patch.insert(String::from("seed"), json!(parse_u64_val(val, key)?));
            }
            "stratify_column" | "sample.stratify_column" => {
                patch.insert(String::from("stratify_column"), json!(parse_opt_usize_min(val, key, 0)?));
            }
            "proportional_allocation" | "sample.proportional_allocation" => {
                patch.insert(String::from("proportional_allocation"), json!(parse_bool_field(val, "proportional_allocation")?));
            }
            "title" | "sample.title" => {
                let raw = parse_str_field(val, "title")?.trim();
                if !raw.is_empty() {
                    patch.insert(String::from("title"), json!(raw));
                }
            }
            "info_sheet_id" | "target_info_sheet_id" | "sample.info_sheet_id" => {
                let tid = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(SheetId::from);
                patch.insert(String::from("target_info_sheet_id"), serde_json::to_value(tid).map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown Random Sample Rows field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_simulate_distributions(
    state: &mut SimulateDistributionsDialogState,
    fields: &Map<String, Value>,
    preset: &LocalePreset,
) -> Result<(), String> {
    let mut working = state.clone();
    if working.columns.is_empty() {
        working.columns.push(SimulatedDistColumnDraft::default());
    }
    let mut patch = Map::new();
    let mut col0_patch = Map::new();

    for (key, val) in fields {
        match key.as_str() {
            "n_rows" | "row_count" | "rows" | "simulate.n_rows" => {
                patch.insert(String::from("n_rows"), json!(parse_usize_min(val, key, 1)?));
            }
            "seed" | "simulate.seed" => {
                patch.insert(String::from("seed"), json!(parse_u64_val(val, key)?));
            }
            "use_multiple_variables" | "multiple_variables" | "simulate.use_multiple_variables" => {
                patch.insert(String::from("use_multiple_variables"), json!(parse_bool_field(val, "use_multiple_variables")?));
            }
            "title" | "simulate.title" => {
                let raw = parse_str_field(val, "title")?.trim();
                if !raw.is_empty() {
                    patch.insert(String::from("title"), json!(raw));
                }
            }
            "columns" | "simulate.columns" => {
                let arr = val.as_array().ok_or_else(|| String::from("`columns` must be an array"))?;
                if arr.is_empty() {
                    return Err(String::from("`columns` cannot be empty"));
                }
                let mut parsed = Vec::with_capacity(arr.len());
                for item in arr {
                    let obj = item.as_object().ok_or_else(|| String::from("each column item must be an object"))?;
                    let mut col = SimulatedDistColumnDraft::default();
                    let mut col_map = normalize_sim_column_object(obj, preset, true)?;
                    if let Some(params_val) = obj.get("params") {
                        let params = params_val.as_object().ok_or_else(|| String::from("`params` must be an object"))?;
                        let sub = normalize_sim_column_object(params, preset, false)?;
                        col_map.extend(sub);
                    }
                    if let Some(params) = obj.get("distribution").and_then(Value::as_object) {
                        let sub = normalize_sim_column_object(params, preset, false)?;
                        col_map.extend(sub);
                    }
                    merge_struct_json(&mut col, &col_map, &[])?;
                    parsed.push(col);
                }
                patch.insert(String::from("columns"), serde_json::to_value(parsed).map_err(|e| e.to_string())?);
            }
            "column_name" | "simulate.column_name" => {
                let s = parse_str_field(val, "column_name")?.trim();
                if !s.is_empty() {
                    col0_patch.insert(String::from("name"), json!(s));
                }
            }
            "distribution" | "simulate.distribution" => {
                let d = SimDistKind::from_str(parse_str_field(val, "distribution")?)?;
                col0_patch.insert(String::from("distribution"), serde_json::to_value(d).map_err(|e| e.to_string())?);
            }
            "min" | "simulate.min" => {
                col0_patch.insert(String::from("min"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "max" | "simulate.max" => {
                col0_patch.insert(String::from("max"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "mean" | "simulate.mean" => {
                col0_patch.insert(String::from("mean"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sd" | "simulate.sd" => {
                col0_patch.insert(String::from("sd"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "mean_log" | "simulate.mean_log" => {
                col0_patch.insert(String::from("mean_log"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sd_log" | "simulate.sd_log" => {
                col0_patch.insert(String::from("sd_log"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "trials" | "simulate.trials" => {
                col0_patch.insert(String::from("trials"), json!(parse_u64_val(val, key)?));
            }
            "p" | "simulate.p" => {
                col0_patch.insert(String::from("p"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "lambda" | "simulate.lambda" => {
                col0_patch.insert(String::from("lambda"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "rate" | "simulate.rate" => {
                col0_patch.insert(String::from("rate"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            other => return Err(format!("unknown Simulate Distributions field `{other}`")),
        }
    }

    merge_struct_json(&mut working, &patch, &[])?;
    if !col0_patch.is_empty()
        && let Some(first_col) = working.columns.first_mut()
    {
        merge_struct_json(first_col, &col0_patch, &[])?;
    }
    *state = working;
    Ok(())
}

fn normalize_sim_column_object(obj: &Map<String, Value>, preset: &LocalePreset, allow_params: bool) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    if let Some(name) = obj.get("name").or_else(|| obj.get("title")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        out.insert(String::from("name"), json!(name));
    }
    if let Some(dist_raw) =
        obj.get("distribution").and_then(|d| d.as_str().or_else(|| d.as_object().and_then(|o| o.get("kind").and_then(Value::as_str))))
    {
        let kind = SimDistKind::from_str(dist_raw)?;
        out.insert(String::from("distribution"), serde_json::to_value(kind).map_err(|e| e.to_string())?);
    }
    for (field, val) in obj {
        match field.as_str() {
            "name" | "title" | "distribution" | "kind" => {}
            "params" if allow_params => {
                if !val.is_object() {
                    return Err(String::from("`params` must be an object"));
                }
            }
            "min" => {
                out.insert(String::from("min"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "max" => {
                out.insert(String::from("max"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "mean" | "mu" => {
                out.insert(String::from("mean"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "sd" | "std" | "sigma" => {
                out.insert(String::from("sd"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "mean_log" | "meanlog" => {
                out.insert(String::from("mean_log"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "sd_log" | "sdlog" => {
                out.insert(String::from("sd_log"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "trials" | "n" => {
                out.insert(String::from("trials"), json!(parse_u64_val(val, field)?));
            }
            "p" | "prob" | "probability" => {
                out.insert(String::from("p"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "lambda" => {
                out.insert(String::from("lambda"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            "rate" => {
                out.insert(String::from("rate"), json!(parse_finite_f64_with_locale(val, field, preset)?));
            }
            other => return Err(format!("unknown simulated column field `{other}`")),
        }
    }
    Ok(out)
}

pub fn patch_insert_model_equation(
    state: &mut InsertModelEquationDialogState,
    fields: &Map<String, Value>,
    preset: &LocalePreset,
) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "result_sheet_id" | "equation.result_sheet_id" => {
                let raw = parse_str_field(val, "result_sheet_id")?.trim();
                if raw.is_empty() {
                    return Err(String::from("`result_sheet_id` cannot be empty"));
                }
                patch.insert(String::from("result_sheet_id"), json!(raw));
            }
            "data_set_index" | "dataset_index" | "equation.data_set_index" => {
                patch.insert(String::from("data_set_index"), json!(parse_usize_min(val, key, 0)?));
            }
            "mode" | "display_mode" | "equation.mode" => {
                let raw = val.as_str().ok_or_else(|| String::from("`mode` must be `substituted` or `symbolic`"))?;
                let mode = parse_equation_display_mode_str(raw)?;
                patch.insert(String::from("mode"), serde_json::to_value(mode).map_err(|e| e.to_string())?);
            }
            "font_size_pt" | "equation.font_size_pt" => {
                let pt = parse_finite_f64_with_locale(val, key, preset)?;
                if !(4.0..=72.0).contains(&pt) {
                    return Err(format!("`font_size_pt` ({pt}) must be within 4.0..=72.0"));
                }
                patch.insert(String::from("font_size_pt"), json!(pt));
            }
            "bordered" | "equation.bordered" | "eq.bordered" => {
                patch.insert(String::from("bordered"), json!(parse_bool_field(val, "bordered")?));
            }
            "update_existing" | "equation.update_existing" => {
                patch.insert(String::from("update_existing"), json!(parse_bool_field(val, "update_existing")?));
            }
            "x_pt" | "equation.x_pt" => {
                insert_nested_field(&mut patch, "bounds_pt", "x", json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "y_pt" | "equation.y_pt" => {
                insert_nested_field(&mut patch, "bounds_pt", "y", json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "width_pt" | "equation.width_pt" => {
                insert_nested_field(&mut patch, "bounds_pt", "width", json!(parse_finite_f64_with_locale(val, key, preset)?.max(24.0)));
            }
            "height_pt" | "equation.height_pt" => {
                insert_nested_field(&mut patch, "bounds_pt", "height", json!(parse_finite_f64_with_locale(val, key, preset)?.max(12.0)));
            }
            other => return Err(format!("unknown Insert Model Equation field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &["bounds_pt"])
}

pub fn patch_import_reshape(state: &mut ImportReshapeState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut patch = Map::new();
    let dec_raw = fields.get("decimal_separator").or_else(|| fields.get("decimal_sep")).or_else(|| fields.get("import.decimal_separator"));
    let mut effective_preset = preset.clone();
    if let Some(dec_val) = dec_raw {
        let ds = parse_str_field(dec_val, "decimal_separator")?;
        let sep = match ds.trim() {
            "," => ',',
            "." => '.',
            _ => return Err(format!("unsupported decimal_separator `{ds}`")),
        };
        effective_preset.decimal_separator = sep;
        patch.insert(String::from("decimal_separator"), json!(sep));
    } else if state.decimal_separator == ',' || state.decimal_separator == '.' {
        effective_preset.decimal_separator = state.decimal_separator;
    }

    let mut effective_mode = state.mode;
    for key in ["mode", "import.mode", "reshape_mode"] {
        if let Some(val) = fields.get(key) {
            let raw = parse_str_field(val, "mode")?;
            effective_mode = match norm_key(raw).as_str() {
                "plate96" | "plates96" | "wells96" | "96" => ImportReshapeMode::Plate96,
                "plate384" | "plates384" | "wells384" | "384" => ImportReshapeMode::Plate384,
                "tidylongpivot" | "tidylong" | "tidy" => ImportReshapeMode::TidyLongPivot,
                _ => return Err(format!("unsupported ImportReshape mode `{raw}`")),
            };
            patch.insert(String::from("mode"), serde_json::to_value(effective_mode).map_err(|e| e.to_string())?);
        }
    }

    for (key, val) in fields {
        match key.as_str() {
            "mode" | "import.mode" | "reshape_mode" | "decimal_separator" | "decimal_sep" | "import.decimal_separator" => {}
            "title" | "import.title" | "import_paste.input.title" | "sheet_title" => {
                patch.insert(String::from("title"), json!(parse_str_field(val, "title")?));
            }
            "target_kind" | "import.target_kind" => {
                let tk = parse_table_kind_str(parse_str_field(val, "target_kind")?)?;
                patch.insert(String::from("target_kind"), serde_json::to_value(tk).map_err(|e| e.to_string())?);
            }
            "x_title" | "import.x_title" => {
                patch.insert(String::from("x_title"), json!(parse_str_field(val, "x_title")?));
            }
            "subtract_blank_mean" | "import.subtract_blank_mean" => {
                patch.insert(String::from("subtract_blank_mean"), json!(parse_bool_field(val, "subtract_blank_mean")?));
            }
            "tidy_csv" | "csv" | "import.tidy_csv" | "import.csv" => {
                patch.insert(String::from("tidy_csv"), json!(parse_str_field(val, "tidy_csv")?));
            }
            "data" | "text" | "import.data" => {
                let s = parse_str_field(val, "data")?;
                if effective_mode == ImportReshapeMode::TidyLongPivot {
                    patch.insert(String::from("tidy_csv"), json!(s));
                } else {
                    let pv = parse_plate_values_text(s, &effective_preset)?;
                    patch.insert(String::from("plate_values"), json!(pv));
                }
            }
            "plate_values" | "values" | "import.plate_values" => {
                patch.insert(String::from("plate_values"), json!(parse_plate_values_json(val, &effective_preset)?));
            }
            "plate_roles" | "roles" | "import.plate_roles" => {
                let arr = val.as_array().ok_or_else(|| String::from("`plate_roles` must be an array"))?;
                patch.insert(String::from("plate_roles"), Value::Array(arr.clone()));
            }
            "tidy_delimiter" | "delimiter" | "import.delimiter" => {
                let s = parse_str_field(val, "delimiter")?;
                let delim = match s.trim().to_ascii_lowercase().as_str() {
                    "tab" | "\\t" | "\t" => '\t',
                    "comma" | "," => ',',
                    "semicolon" | ";" => ';',
                    "pipe" | "|" => '|',
                    _ => s.chars().next().ok_or_else(|| String::from("`delimiter` cannot be empty"))?,
                };
                patch.insert(String::from("tidy_delimiter"), json!(delim));
            }
            "x_col" | "import.x_col" => {
                patch.insert(String::from("x_col"), json!(parse_opt_nonempty_string(val, key)?));
            }
            "group_col" | "import.group_col" => {
                patch.insert(String::from("group_col"), json!(parse_str_field(val, "group_col")?));
            }
            "replicate_col" | "import.replicate_col" => {
                patch.insert(String::from("replicate_col"), json!(parse_opt_nonempty_string(val, key)?));
            }
            "value_col" | "import.value_col" => {
                patch.insert(String::from("value_col"), json!(parse_str_field(val, "value_col")?));
            }
            "excluded_col" | "import.excluded_col" => {
                patch.insert(String::from("excluded_col"), json!(parse_opt_nonempty_string(val, key)?));
            }
            other => return Err(format!("unknown Import Reshape field `{other}`")),
        }
    }

    merge_struct_json(state, &patch, &[])
}

fn parse_wizard_power_family_str(raw: &str) -> Result<PowerTestFamily, String> {
    match norm_key(raw).as_str() {
        "unpairedt" | "twosampleunpairedt" | "independentt" | "twosamplet" => Ok(PowerTestFamily::TwoSampleUnpairedT),
        "pairedt" | "twosamplepairedt" | "dependentt" => Ok(PowerTestFamily::TwoSamplePairedT),
        "onesamplet" | "singlesamplet" => Ok(PowerTestFamily::OneSampleT),
        "onewayanova" | "anova" => Ok(PowerTestFamily::OneWayAnova),
        "twoproportions" | "proportions" => Ok(PowerTestFamily::TwoProportions),
        "correlation" | "pearsoncorrelation" => Ok(PowerTestFamily::Correlation),
        "chisquare" | "chi2" => Ok(PowerTestFamily::ChiSquare),
        "simplelinearregression" | "linearregression" | "regression" | "simplelinear" => Ok(PowerTestFamily::SimpleLinearRegression),
        _ => Err(format!("unknown power test family `{raw}`")),
    }
}

fn parse_wizard_power_solve_target_str(raw: &str) -> Result<PowerSolveTarget, String> {
    match norm_key(raw).as_str() {
        "samplesize" | "n" => Ok(PowerSolveTarget::SampleSize),
        "power" => Ok(PowerSolveTarget::Power),
        "detectableeffect" | "effect" | "mde" => Ok(PowerSolveTarget::DetectableEffect),
        _ => Err(format!("unknown power solve target `{raw}`")),
    }
}

pub fn patch_power_wizard(state: &mut PowerWizardState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut working = state.clone();
    if let Some(fam_val) = fields.get("family").or_else(|| fields.get("power.family")) {
        let fam = parse_wizard_power_family_str(parse_str_field(fam_val, "family")?)?;
        working.set_family(fam);
    }

    let mut patch = Map::new();
    let mut target_stage: Option<PowerWizardStage> = None;
    let mut hover_actions: Vec<PowerHoverAction> = Vec::new();
    let active_family = working.family;

    for (key, val) in fields {
        match key.as_str() {
            "family" | "power.family" => {}
            "stage" | "power.stage" => target_stage = Some(parse_power_stage(val)?),
            "solve_target" | "power.solve_target" => {
                let st = parse_wizard_power_solve_target_str(parse_str_field(val, "solve_target")?)?;
                patch.insert(String::from("solve_target"), serde_json::to_value(st).map_err(|e| e.to_string())?);
            }
            "effect_mode" | "power.effect_mode" => {
                let em = parse_power_effect_mode(parse_str_field(val, "effect_mode")?)?;
                patch.insert(String::from("effect_mode"), serde_json::to_value(em).map_err(|e| e.to_string())?);
            }
            "standardized_effect" | "power.standardized_effect" | "effect_size" | "d" => {
                patch.insert(String::from("standardized_effect"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "mean_diff" | "power.mean_diff" => {
                patch.insert(String::from("mean_diff"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sd" | "sigma" | "power.sd" => {
                patch.insert(String::from("sd"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sd_group2" | "power.sd_group2" => {
                patch.insert(String::from("sd_group2"), json!(parse_opt_finite_f64_with_locale(val, key, preset)?));
            }
            "within_pair_correlation" | "r" | "power.within_pair_correlation" => {
                patch.insert(String::from("within_pair_correlation"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "paired_sd_diff" | "sigma_d" | "power.paired_sd_diff" => {
                patch.insert(String::from("paired_sd_diff"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "anova_groups" | "num_groups" | "k" | "power.anova_groups" => {
                let k = parse_positive_usize_arg(val, key, preset)?;
                patch.insert(String::from("anova_groups"), json!(k.max(2)));
            }
            "group_means" | "power.group_means" => {
                let parsed = parse_f64_list_with_locale(val, "group_means", preset)?;
                if parsed.len() < 2 {
                    return Err(String::from("`group_means` must contain at least 2 group means"));
                }
                patch.insert(String::from("anova_groups"), json!(parsed.len()));
                patch.insert(String::from("group_means"), json!(parsed));
            }
            "p1" | "power.p1" => {
                patch.insert(String::from("p1"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "p2" | "power.p2" => {
                patch.insert(String::from("p2"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "r_squared" | "power.r_squared" => {
                patch.insert(String::from("r_squared"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "beta1" | "power.beta1" => {
                patch.insert(String::from("beta1"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sigma_x" | "power.sigma_x" => {
                patch.insert(String::from("sigma_x"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sigma_eps" | "power.sigma_eps" => {
                patch.insert(String::from("sigma_eps"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "two_tailed" | "power.two_tailed" => {
                let two = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                if !two && !active_family.supports_one_sided() {
                    return Err(format!("{} only supports two-tailed tests", active_family.label()));
                }
                patch.insert(String::from("two_tailed"), json!(two));
            }
            "one_sided" | "power.one_sided" => {
                let one = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                if one && !active_family.supports_one_sided() {
                    return Err(format!("{} only supports two-tailed tests", active_family.label()));
                }
                patch.insert(String::from("two_tailed"), json!(!one));
            }
            "one_sided_greater" | "power.one_sided_greater" => {
                patch.insert(String::from("one_sided_greater"), json!(val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?));
            }
            "tail_mode" | "power.tail_mode" => {
                let raw = parse_str_field(val, "tail_mode")?;
                match norm_key(raw).as_str() {
                    "twosided" | "twotailed" | "two" => {
                        patch.insert(String::from("two_tailed"), json!(true));
                    }
                    "onesidedgreater" | "onetailedgreater" | "onesided" | "onetailed" | "greater" => {
                        if !active_family.supports_one_sided() {
                            return Err(format!("{} only supports two-tailed tests", active_family.label()));
                        }
                        patch.insert(String::from("two_tailed"), json!(false));
                        patch.insert(String::from("one_sided_greater"), json!(true));
                    }
                    "onesidedless" | "onetailedless" | "less" => {
                        if !active_family.supports_one_sided() {
                            return Err(format!("{} only supports two-tailed tests", active_family.label()));
                        }
                        patch.insert(String::from("two_tailed"), json!(false));
                        patch.insert(String::from("one_sided_greater"), json!(false));
                    }
                    _ => return Err(format!("unknown tail mode `{raw}`")),
                }
            }
            "alpha" | "power.alpha" | "power.sidebar.alpha" => {
                patch.insert(String::from("alpha"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "target_power" | "power" | "power.target_power" => {
                patch.insert(String::from("target_power"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sample_size" | "n" | "power.sample_size" => {
                patch.insert(String::from("sample_size"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "sample_size_group2" | "n2" | "power.sample_size_group2" => {
                patch.insert(String::from("sample_size_group2"), json!(parse_opt_finite_f64_with_locale(val, key, preset)?));
            }
            "allocation_ratio" | "power.allocation_ratio" => {
                patch.insert(String::from("allocation_ratio"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "chi_square_df" | "df" | "power.chi_square_df" => {
                patch.insert(String::from("chi_square_df"), json!(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "continuity_correction" | "power.continuity_correction" => {
                patch.insert(String::from("continuity_correction"), json!(val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?));
            }
            "hover_n" | "power.hover_n" => {
                hover_actions.push(PowerHoverAction::AtN(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "hover_fraction_x" | "power.hover_fraction_x" => {
                hover_actions.push(PowerHoverAction::AtFraction(parse_finite_f64_with_locale(val, key, preset)?));
            }
            "power.curve_hover" | "curve_hover" => {
                if let Some(obj) = val.as_object() {
                    if let Some(hn_v) = obj.get("hover_n") {
                        hover_actions.push(PowerHoverAction::AtN(parse_finite_f64_with_locale(hn_v, "power.curve_hover.hover_n", preset)?));
                    } else if let Some(fx_v) = obj.get("hover_fraction_x").or_else(|| obj.get("fraction_x")) {
                        hover_actions.push(PowerHoverAction::AtFraction(parse_finite_f64_with_locale(
                            fx_v,
                            "power.curve_hover.hover_fraction_x",
                            preset,
                        )?));
                    } else {
                        return Err(String::from("`power.curve_hover` object must specify `hover_n` or `hover_fraction_x`"));
                    }
                } else {
                    hover_actions.push(PowerHoverAction::AtN(parse_finite_f64_with_locale(val, key, preset)?));
                }
            }
            other => return Err(format!("unknown power wizard field `{other}`")),
        }
    }

    merge_struct_json(&mut working, &patch, &[])?;
    if let Some(stage) = target_stage {
        working.step_to_stage(stage)?;
    }
    if matches!(working.stage, PowerWizardStage::SignificanceAndSolve | PowerWizardStage::Results) || working.result.is_some() {
        let _ = working.recalculate()?;
    } else {
        let _ = working.recalculate();
    }
    for act in hover_actions {
        if working.result.is_none() {
            let _ = working.recalculate()?;
        }
        match act {
            PowerHoverAction::AtN(n) => {
                let _ = working.hover_at_n(n);
            }
            PowerHoverAction::AtFraction(f) => {
                let _ = working.hover_at_fraction(f);
            }
        }
    }
    *state = working;
    Ok(())
}

enum PowerHoverAction {
    AtN(f64),
    AtFraction(f64),
}

fn parse_power_stage(val: &Value) -> Result<PowerWizardStage, String> {
    if let Some(idx) = val.as_u64() {
        let idx_usize = usize::try_from(idx).map_err(|_| format!("invalid power wizard stage index `{idx}`"))?;
        return PowerWizardStage::from_index(idx_usize).ok_or_else(|| format!("invalid power wizard stage index `{idx}`"));
    }
    let raw = val.as_str().ok_or_else(|| String::from("`stage` must be a string or stage index"))?;
    match norm_key(raw).as_str() {
        "1" | "experimentdesign" | "design" => Ok(PowerWizardStage::ExperimentDesign),
        "2" | "effectsize" | "effect" => Ok(PowerWizardStage::EffectSize),
        "3" | "significanceandsolve" | "significance" => Ok(PowerWizardStage::SignificanceAndSolve),
        "4" | "results" | "resultssensitivity" => Ok(PowerWizardStage::Results),
        _ => Err(format!("unknown power wizard stage `{raw}`")),
    }
}

fn parse_power_effect_mode(raw: &str) -> Result<PowerEffectInputMode, String> {
    match norm_key(raw).as_str() {
        "standardized" | "direct" => Ok(PowerEffectInputMode::Standardized),
        "meansandsd" | "rawmeans" | "means" => Ok(PowerEffectInputMode::MeansAndSd),
        "paireddiffdirect" | "paireddiff" | "sigmad" => Ok(PowerEffectInputMode::PairedDiffDirect),
        "proportions" | "twoproportions" => Ok(PowerEffectInputMode::Proportions),
        "regressionrsquared" | "rsquared" | "r2" => Ok(PowerEffectInputMode::RegressionRSquared),
        "regressionslope" | "slope" => Ok(PowerEffectInputMode::RegressionSlope),
        _ => Err(format!("unknown power effect mode `{raw}`")),
    }
}

pub(super) fn power_tail_mode_from_bool(two_tailed: bool) -> curvebook_doc::PowerTailMode {
    if two_tailed {
        curvebook_doc::PowerTailMode::TwoSided
    } else {
        curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater)
    }
}

pub(super) fn parse_power_solve_target_for_spec(raw: &str) -> Result<curvebook_doc::PowerSolveTarget, String> {
    super::value_parsers::parse_power_solve_target_str(raw)
}

pub(super) fn parse_power_family_for_spec(raw: &str) -> Result<curvebook_doc::PowerTestFamily, String> {
    super::value_parsers::parse_power_family_str(raw)
}
