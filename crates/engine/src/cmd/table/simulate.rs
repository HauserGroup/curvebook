//! Simulation, randomization, and random sampling table commands (`table.simulateXy`,
//! `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`).

use std::collections::BTreeMap;

use curvebook_doc::{ConstantValue, GraphSheet, InfoSheet, ProjectDocument, SheetId, SimulationNoise, validate_rows};
use curvebook_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution,
    SubjectRandomizationConfig, random_sample_rows, randomize_subjects_to_groups, simulate_distribution_columns, simulate_xy_sheet,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::Ctx;
use super::super::helpers::{
    bad_input, link_data_sheet_to_info, next_unique_sheet_id, parse_sheet_id, require_analyzable_data_sheet, require_info_sheet_mut,
    require_unfrozen_data_sheet_mut, sheet_id_exists,
};
use super::super::params::{command_schema, parse_params};
use super::super::project::recalculate_dirty_with_links;
use super::validate_project_cell_budget_after_change;
use crate::{CommandOutcome, EngineError, ErrorCode};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SimParamObj {
    name: String,
    value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum SimParamArrayItem {
    Scalar(f64),
    Pair(String, f64),
    Obj(SimParamObj),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum SimParametersInput {
    Map(BTreeMap<String, f64>),
    List(Vec<SimParamArrayItem>),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SimulateXyParams {
    /// Optional destination data sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Simulated sheet title (default `Simulated XY`).
    title: Option<String>,
    /// PRNG seed (default 1).
    seed: Option<u64>,
    /// Number of simulated X rows (default 12).
    row_count: Option<usize>,
    /// Number of replicate Y subcolumns (default 3).
    replicates: Option<usize>,
    /// Minimum X value (default 0.0).
    x_min: Option<f64>,
    /// Maximum X value (default 10.0).
    x_max: Option<f64>,
    /// Model equation ID (default `hill_4pl`).
    equation_id: Option<String>,
    /// Simulation equation parameters as `{ Name: value }` map or array.
    parameters: Option<SimParametersInput>,
    /// Noise model: none, gaussian, relative_percent/cv, or poisson (default gaussian).
    noise_kind: Option<String>,
    /// Gaussian SD noise level (default 0.0).
    noise_sd: Option<f64>,
    /// Relative CV% noise level (default 0.0).
    cv_percent: Option<f64>,
    /// Auto-create linked InfoSheet for new simulated table.
    auto_info: Option<bool>,
    /// Optional GraphSheet ID to create for new simulated table.
    #[schemars(length(min = 1))]
    graph_sheet_id: Option<String>,
}

fn resolve_simulation_parameters(raw: Option<SimParametersInput>) -> Result<Vec<(String, f64)>, EngineError> {
    let Some(input) = raw else {
        return Ok(Vec::new());
    };
    let default_names = ["Bottom", "Top", "LogEC50", "HillSlope", "S", "K", "Span", "Rate", "Slope", "Intercept"];
    let pairs: Vec<(String, f64)> = match input {
        SimParametersInput::Map(map) => map.into_iter().collect(),
        SimParametersInput::List(items) => items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| match item {
                SimParamArrayItem::Scalar(num) => (default_names.get(idx).copied().unwrap_or("Param").to_owned(), num),
                SimParamArrayItem::Pair(name, num) => (name, num),
                SimParamArrayItem::Obj(o) => (o.name, o.value),
            })
            .collect(),
    };
    for (_, val) in &pairs {
        if !val.is_finite() {
            return Err(bad_input("simulation parameter value must be finite"));
        }
    }
    Ok(pairs)
}

fn resolve_simulation_noise(noise_kind: Option<&str>, noise_sd: Option<f64>, cv_percent: Option<f64>) -> Result<SimulationNoise, EngineError> {
    let kind = noise_kind.unwrap_or("gaussian");
    let norm: String = kind.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "none" | "zero" => Ok(SimulationNoise::None),
        "gaussian" | "normal" => Ok(SimulationNoise::Gaussian { sd: noise_sd.unwrap_or(0.0) }),
        "relative" | "relativepercent" | "cv" => Ok(SimulationNoise::RelativePercent { cv_percent: cv_percent.or(noise_sd).unwrap_or(0.0) }),
        "poisson" => Ok(SimulationNoise::Poisson),
        _ => {
            Err(bad_input(format!("unknown simulation noise model `{kind}`"))
                .with_remediation("Use none, gaussian, relative_percent/cv, or poisson."))
        }
    }
}

pub(super) fn run_simulate_xy(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SimulateXyParams = parse_params(&command_schema::<SimulateXyParams>(), raw)?;
    let sim_params = resolve_simulation_parameters(p.parameters)?;
    let noise = resolve_simulation_noise(p.noise_kind.as_deref(), p.noise_sd, p.cv_percent)?;

    let title = p.title.as_deref().unwrap_or("Simulated XY");
    let seed = p.seed.unwrap_or(1);
    let row_count = p.row_count.unwrap_or(12);
    let replicates = p.replicates.unwrap_or(3);
    let x_min = p.x_min.unwrap_or(0.0);
    let x_max = p.x_max.unwrap_or(10.0);
    let equation_id = p.equation_id.as_deref().unwrap_or("hill_4pl");

    let doc = &mut ctx.session.doc;
    let id = match p.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "data")?,
    };

    let mut simulated = simulate_xy_sheet(id.as_str(), title, seed, row_count, replicates, x_min, x_max, equation_id, &sim_params, noise)?;

    if doc.data_sheet(&id).is_some() {
        validate_project_cell_budget_after_change(doc, Some(&id), simulated.archetype.allocated_cell_slots())?;
        let existing_info = doc.data_sheet(&id).and_then(|d| d.info_sheet_id.clone());
        let slot = require_unfrozen_data_sheet_mut(doc, &id)?;
        if let Some(info_id) = existing_info {
            simulated.info_sheet_id = Some(info_id);
        }
        *slot = simulated;
    } else {
        validate_project_cell_budget_after_change(doc, None, simulated.archetype.allocated_cell_slots())?;
        doc.add_data_sheet(simulated)?;
        if p.auto_info.unwrap_or(false) {
            let iid = next_unique_sheet_id(doc, "info")?;
            if let Some(ds) = doc.data_sheet_mut(&id) {
                ds.info_sheet_id = Some(iid.clone());
            }
            doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), format!("{title} Info"))?)?;
            link_data_sheet_to_info(doc, &iid, &id);
        }
        if let Some(graph_id) = p.graph_sheet_id.as_deref()
            && let Some(ds) = doc.data_sheet(&id).cloned()
        {
            doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data(parse_sheet_id(graph_id)?, &ds)?)?;
        }
        doc.select_in_navigator(&id)?;
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), None, Some(ctx.cancelled))?;
    Ok(CommandOutcome::mutated(
        "table.simulateXy",
        "simulated XY data sheet",
        json!({
            "sheet_id": id.as_str(),
            "equation_id": equation_id,
            "row_count": row_count,
            "replicates": replicates,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

fn resolve_new_data_sheet_id(doc: &ProjectDocument, raw_id: Option<&str>, prefix: &str) -> Result<SheetId, EngineError> {
    if let Some(raw) = raw_id {
        let id = parse_sheet_id(raw)?;
        if sheet_id_exists(doc, &id) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("sheet `{id}` already exists")));
        }
        Ok(id)
    } else {
        next_unique_sheet_id(doc, prefix)
    }
}

fn upsert_info_constants(
    doc: &mut ProjectDocument,
    explicit_info_id: Option<&str>,
    fallback_info_id: Option<&SheetId>,
    create_if_missing: bool,
    default_title: &str,
    constants: &[(String, String)],
) -> Result<Option<SheetId>, EngineError> {
    let target_id = if let Some(raw) = explicit_info_id {
        let iid = parse_sheet_id(raw)?;
        if doc.info_sheet(&iid).is_some() {
            iid
        } else if sheet_id_exists(doc, &iid) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("sheet `{iid}` already exists and is not an InfoSheet")));
        } else {
            doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), default_title)?)?;
            iid
        }
    } else if let Some(fid) = fallback_info_id.filter(|id| doc.info_sheet(id).is_some()) {
        fid.clone()
    } else if create_if_missing {
        let iid = next_unique_sheet_id(doc, "info")?;
        doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), default_title)?)?;
        iid
    } else {
        return Ok(None);
    };

    let info = require_info_sheet_mut(doc, &target_id)?;
    for (name, val) in constants {
        info.set_constant(name.clone(), ConstantValue::Text(val.clone()), None)?;
    }
    Ok(Some(target_id))
}

const fn default_weight_one() -> usize {
    1
}

fn subject_ids_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            { "type": "array", "items": { "type": "string", "minLength": 1 }, "minItems": 1 },
            { "type": "null" }
        ]
    })
}

fn randomize_scheme_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            { "type": "string", "enum": ["complete_balanced", "permuted_block"] },
            { "type": "null" }
        ]
    })
}

fn block_sizes_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            { "type": "array", "items": { "type": "integer", "minimum": 1 }, "minItems": 1 },
            { "type": "null" }
        ]
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum GroupSpecInput {
    Name(#[schemars(length(min = 1))] String),
    Weighted {
        /// Treatment arm label.
        #[schemars(length(min = 1))]
        label: String,
        /// Allocation weight.
        #[serde(default = "default_weight_one")]
        #[schemars(range(min = 1))]
        weight: usize,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RandomizeGroupsParams {
    /// Treatment arm names or `{label, weight}` allocation objects.
    #[schemars(length(min = 1))]
    groups: Vec<GroupSpecInput>,
    /// Explicit subject ID list.
    #[serde(default)]
    #[schemars(schema_with = "subject_ids_schema")]
    subject_ids: Option<Vec<String>>,
    /// Number of subjects (auto-numbered 1..=N when `subject_ids` is omitted).
    #[serde(default)]
    #[schemars(range(min = 1, max = 1_000_000))]
    subject_count: Option<usize>,
    /// Allocation scheme (`complete_balanced` or `permuted_block`, default `complete_balanced`).
    #[serde(default)]
    #[schemars(schema_with = "randomize_scheme_schema")]
    scheme: Option<String>,
    /// Block sizes (each a positive multiple of sum of weights) for `permuted_block`.
    #[serde(default)]
    #[schemars(schema_with = "block_sizes_schema")]
    block_sizes: Option<Vec<usize>>,
    /// Deterministic 64-bit PRNG seed (default 1).
    #[serde(default)]
    seed: Option<u64>,
    /// Optional destination DataSheet ID.
    #[serde(default)]
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional destination sheet title (default `Randomization Schedule`).
    #[serde(default)]
    title: Option<String>,
    /// Optional target InfoSheet ID for ARRIVE randomization constants.
    #[serde(default)]
    #[schemars(length(min = 1))]
    info_sheet_id: Option<String>,
}

pub(super) fn run_randomize_groups(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RandomizeGroupsParams = parse_params(&command_schema::<RandomizeGroupsParams>(), raw)?;
    let groups: Vec<GroupAllocationWeight> = p
        .groups
        .into_iter()
        .map(|g| match g {
            GroupSpecInput::Name(label) => GroupAllocationWeight::new(label),
            GroupSpecInput::Weighted { label, weight } => GroupAllocationWeight::with_weight(label, weight),
        })
        .collect();
    let sum_weights: usize = groups.iter().fold(0usize, |acc, g| acc.saturating_add(g.weight));

    let norm_scheme =
        p.scheme.as_deref().unwrap_or(if p.block_sizes.as_ref().is_some_and(|v| !v.is_empty()) { "permuted_block" } else { "complete_balanced" });
    let scheme_key: String = norm_scheme.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let scheme = match scheme_key.as_str() {
        "completebalanced" | "complete" | "balanced" => AllocationScheme::CompleteBalanced,
        "permutedblock" | "block" => {
            let block_sizes = match p.block_sizes {
                Some(bs) => bs,
                None => vec![sum_weights.saturating_mul(2).max(2)],
            };
            AllocationScheme::PermutedBlock { block_sizes }
        }
        other => {
            return Err(bad_input(format!("unknown randomization scheme `{other}`; use `complete_balanced` or `permuted_block`")));
        }
    };

    let doc = &mut ctx.session.doc;
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let title = p.title.unwrap_or_else(|| String::from("Randomization Schedule"));
    let seed = p.seed.unwrap_or(1);

    let config = match (p.subject_ids, p.subject_count) {
        (Some(ids), Some(count)) => {
            if ids.len() != count {
                return Err(bad_input(format!("`subject_ids` length ({}) does not match `subject_count` ({count})", ids.len())));
            }
            validate_rows(ids.len())?;
            validate_project_cell_budget_after_change(doc, None, ids.len().saturating_mul(4))?;
            SubjectRandomizationConfig { sheet_id: id.to_string(), title: title.clone(), subject_ids: ids, groups, scheme, seed }
        }
        (Some(ids), None) => {
            validate_rows(ids.len())?;
            validate_project_cell_budget_after_change(doc, None, ids.len().saturating_mul(4))?;
            SubjectRandomizationConfig { sheet_id: id.to_string(), title: title.clone(), subject_ids: ids, groups, scheme, seed }
        }
        (None, Some(count)) => {
            validate_rows(count)?;
            validate_project_cell_budget_after_change(doc, None, count.saturating_mul(4))?;
            SubjectRandomizationConfig::try_from_subject_count(id.to_string(), title.clone(), count, groups, scheme, seed)?
        }
        (None, None) => {
            return Err(bad_input("table.randomizeGroups requires `subject_ids` or `subject_count`"));
        }
    };

    let subject_count = config.subject_ids.len();
    let mut rand_out = randomize_subjects_to_groups(&config)?;
    validate_project_cell_budget_after_change(doc, None, rand_out.data_sheet.archetype.allocated_cell_slots())?;
    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), None, true, &format!("{title} Info"), &rand_out.arrive_constants)?
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "failed resolving InfoSheet for ARRIVE constants"))?;
    rand_out.data_sheet.info_sheet_id = Some(info_id.clone());
    doc.add_data_sheet(rand_out.data_sheet)?;
    link_data_sheet_to_info(doc, &info_id, &id);
    let recalc = recalculate_dirty_with_links(doc, &[id.clone(), info_id.clone()], None, Some(ctx.cancelled))?;

    let constants_map: serde_json::Map<String, Value> = rand_out.arrive_constants.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
    Ok(CommandOutcome::mutated(
        "table.randomizeGroups",
        format!("randomized {subject_count} subjects into `{}`", id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "info_sheet_id": info_id.as_str(),
            "title": title,
            "subject_count": subject_count,
            "seed": seed,
            "arrive_constants": constants_map,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RandomSampleRowsParams {
    /// Source DataSheet or analyzable ResultSheet ID.
    #[schemars(length(min = 1))]
    source_sheet_id: String,
    /// Number of rows to draw without replacement.
    #[schemars(range(min = 1))]
    sample_size: usize,
    /// Deterministic 64-bit PRNG seed (default 42).
    #[serde(default)]
    seed: Option<u64>,
    /// Optional zero-based stratification column index.
    #[serde(default)]
    stratify_column: Option<usize>,
    /// Use proportional stratum allocation when stratified (default true).
    #[serde(default)]
    proportional_allocation: Option<bool>,
    /// Optional destination DataSheet ID.
    #[serde(default)]
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional destination sheet title.
    #[serde(default)]
    title: Option<String>,
    /// Optional InfoSheet ID for recording sampling provenance constants.
    #[serde(default)]
    #[schemars(length(min = 1))]
    info_sheet_id: Option<String>,
}

pub(super) fn run_random_sample_rows(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RandomSampleRowsParams = parse_params(&command_schema::<RandomSampleRowsParams>(), raw)?;
    let src_id = parse_sheet_id(&p.source_sheet_id)?;
    let seed = p.seed.unwrap_or(42);
    let proportional = p.proportional_allocation.unwrap_or(true);

    let doc = &mut ctx.session.doc;
    let src = require_analyzable_data_sheet(doc, &src_id)?.clone();
    let src_info_id = src.info_sheet_id.clone();
    let population_size = src.archetype.row_count();
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let title = p.title.unwrap_or_else(|| format!("{} (Sample n={})", src.title, p.sample_size));

    let mut sampled = random_sample_rows(
        &src,
        &RowSampleConfig {
            seed,
            sample_size: p.sample_size,
            stratify_column: p.stratify_column,
            proportional_allocation: proportional,
            output_id: Some(id.to_string()),
            output_title: Some(title.clone()),
        },
    )?;

    let out_rows = sampled.archetype.row_count();
    let out_cols = sampled.archetype.column_count();
    validate_project_cell_budget_after_change(doc, None, sampled.archetype.allocated_cell_slots())?;
    let mut provenance = vec![
        (String::from("sample.source_sheet_id"), src_id.to_string()),
        (String::from("sample.seed"), seed.to_string()),
        (String::from("sample.sample_size"), out_rows.to_string()),
        (String::from("sample.population_size"), population_size.to_string()),
    ];
    if let Some(col) = p.stratify_column {
        provenance.push((String::from("sample.stratify_column"), col.to_string()));
        provenance.push((String::from("sample.allocation"), String::from(if proportional { "proportional" } else { "equal" })));
    }

    let reusable_src_info_id = src_info_id.filter(|iid| {
        doc.info_sheet(iid).is_some_and(|info| !info.constants.iter().any(|c| c.name.starts_with("sample.") || c.name.starts_with("randomization.")))
    });
    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), reusable_src_info_id.as_ref(), true, &format!("{title} Info"), &provenance)?;
    if let Some(ref iid) = info_id {
        sampled.info_sheet_id = Some(iid.clone());
    }
    doc.add_data_sheet(sampled)?;
    if let Some(ref iid) = info_id {
        link_data_sheet_to_info(doc, iid, &id);
    }
    let mut dirty_seeds = vec![id.clone()];
    if let Some(ref iid) = info_id {
        dirty_seeds.push(iid.clone());
    }
    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, None, Some(ctx.cancelled))?;

    Ok(CommandOutcome::mutated(
        "table.randomSampleRows",
        format!("sampled {out_rows} row(s) from `{}` into `{}`", src_id.as_str(), id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "source_sheet_id": src_id.as_str(),
            "info_sheet_id": info_id.as_ref().map(SheetId::as_str),
            "title": title,
            "row_count": out_rows,
            "col_count": out_cols,
            "seed": seed,
            "sample_size": p.sample_size,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

fn sim_dist_columns_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "minItems": 1,
        "items": {
            "type": "object",
            "description": "Column distribution specification `{ name, distribution, ... }` supporting uniform (`min`, `max`), gaussian (`mean`, `sd`), log_normal (`mean_log`, `sd_log`), binomial (`trials`, `p`), poisson (`lambda`), exponential (`rate`)",
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "distribution": {
                    "anyOf": [
                        { "type": "string", "enum": ["uniform", "gaussian", "log_normal", "binomial", "poisson", "exponential"] },
                        { "type": "object" }
                    ]
                },
                "params": { "type": "object" },
                "min": { "type": "number", "description": "Minimum bound for uniform distribution" },
                "max": { "type": "number", "description": "Maximum bound for uniform distribution" },
                "mean": { "type": "number", "description": "Mean for gaussian distribution" },
                "sd": { "type": "number", "description": "Standard deviation for gaussian distribution" },
                "mean_log": { "type": "number", "description": "Log-space mean for log_normal distribution" },
                "sd_log": { "type": "number", "description": "Log-space standard deviation for log_normal distribution" },
                "trials": { "type": "integer", "minimum": 0, "description": "Number of trials for binomial distribution" },
                "p": { "type": "number", "description": "Success probability for binomial distribution" },
                "lambda": { "type": "number", "description": "Rate/mean parameter for poisson distribution" },
                "rate": { "type": "number", "description": "Rate parameter for exponential distribution" }
            },
            "required": ["name", "distribution"],
            "additionalProperties": false
        }
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SimulateDistributionsParams {
    /// Simulated column distribution specifications.
    #[schemars(schema_with = "sim_dist_columns_schema")]
    columns: Vec<Value>,
    /// Number of rows to simulate per column.
    #[schemars(range(min = 1))]
    n_rows: usize,
    /// Deterministic 64-bit PRNG seed (default 1).
    #[serde(default)]
    seed: Option<u64>,
    /// Create MultipleVariables table instead of Column table (default false).
    #[serde(default)]
    use_multiple_variables: Option<bool>,
    /// Optional destination DataSheet ID.
    #[serde(default)]
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional destination sheet title (default `Simulated Distributions`).
    #[serde(default)]
    title: Option<String>,
}

fn find_dist_f64(sources: &[&serde_json::Map<String, Value>], field_name: &str, idx: usize) -> Result<f64, EngineError> {
    for src in sources {
        if let Some(val) = src.get(field_name) {
            return val.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad_input(format!("columns[{idx}].{field_name} must be a finite number")));
        }
    }
    Err(bad_input(format!("columns[{idx}] missing numeric parameter `{field_name}`")))
}

fn find_dist_u64(sources: &[&serde_json::Map<String, Value>], field_name: &str, idx: usize) -> Result<u64, EngineError> {
    for src in sources {
        if let Some(val) = src.get(field_name) {
            return val.as_u64().ok_or_else(|| bad_input(format!("columns[{idx}].{field_name} must be a non-negative integer")));
        }
    }
    Err(bad_input(format!("columns[{idx}] missing integer parameter `{field_name}`")))
}

fn parse_simulated_column_spec(item: &Value, idx: usize) -> Result<SimulatedColumnSpec, EngineError> {
    if let Ok(direct) = serde_json::from_value::<SimulatedColumnSpec>(item.clone()) {
        return Ok(direct);
    }
    let obj = item.as_object().ok_or_else(|| bad_input(format!("columns[{idx}] must be an object")))?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_input(format!("columns[{idx}] missing non-empty `name`")))?
        .to_owned();

    let dist_val = obj.get("distribution").ok_or_else(|| bad_input(format!("columns[{idx}] missing `distribution`")))?;

    if let Ok(dist) = serde_json::from_value::<SimulatedDistribution>(dist_val.clone()) {
        return Ok(SimulatedColumnSpec { name, distribution: dist });
    }

    let mut sources: Vec<&serde_json::Map<String, Value>> = Vec::new();
    if let Some(params_obj) = obj.get("params").and_then(Value::as_object) {
        sources.push(params_obj);
    }
    let kind_raw = if let Some(s) = dist_val.as_str() {
        s
    } else if let Some(dist_obj) = dist_val.as_object() {
        sources.push(dist_obj);
        if let Some(inner_params) = dist_obj.get("params").and_then(Value::as_object) {
            sources.push(inner_params);
        }
        dist_obj.get("kind").and_then(Value::as_str).ok_or_else(|| bad_input(format!("columns[{idx}].distribution object must specify `kind`")))?
    } else {
        return Err(bad_input(format!("columns[{idx}].distribution must be a distribution name or object")));
    };
    sources.push(obj);

    let kind_key: String = kind_raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let distribution = match kind_key.as_str() {
        "uniform" => SimulatedDistribution::Uniform { min: find_dist_f64(&sources, "min", idx)?, max: find_dist_f64(&sources, "max", idx)? },
        "gaussian" | "normal" => {
            SimulatedDistribution::Gaussian { mean: find_dist_f64(&sources, "mean", idx)?, sd: find_dist_f64(&sources, "sd", idx)? }
        }
        "lognormal" => {
            SimulatedDistribution::LogNormal { mean_log: find_dist_f64(&sources, "mean_log", idx)?, sd_log: find_dist_f64(&sources, "sd_log", idx)? }
        }
        "binomial" => SimulatedDistribution::Binomial { trials: find_dist_u64(&sources, "trials", idx)?, p: find_dist_f64(&sources, "p", idx)? },
        "poisson" => SimulatedDistribution::Poisson { lambda: find_dist_f64(&sources, "lambda", idx)? },
        "exponential" | "exp" => SimulatedDistribution::Exponential { rate: find_dist_f64(&sources, "rate", idx)? },
        other => {
            return Err(bad_input(format!(
                "columns[{idx}] unknown distribution `{other}`; use uniform, gaussian, log_normal, binomial, poisson, or exponential"
            )));
        }
    };

    Ok(SimulatedColumnSpec { name, distribution })
}

pub(super) fn run_simulate_distributions(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SimulateDistributionsParams = parse_params(&command_schema::<SimulateDistributionsParams>(), raw)?;
    let columns: Vec<SimulatedColumnSpec> =
        p.columns.iter().enumerate().map(|(i, item)| parse_simulated_column_spec(item, i)).collect::<Result<_, _>>()?;
    let seed = p.seed.unwrap_or(1);
    let use_mv = p.use_multiple_variables.unwrap_or(false);
    let title = p.title.unwrap_or_else(|| String::from("Simulated Distributions"));

    let doc = &mut ctx.session.doc;
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let col_count = columns.len();
    let cfg = DistributionSimulationConfig {
        sheet_id: id.to_string(),
        title: title.clone(),
        n_rows: p.n_rows,
        seed,
        columns,
        use_multiple_variables: use_mv,
    };
    let sheet = simulate_distribution_columns(&cfg)?;
    let kind = sheet.archetype.kind();
    validate_project_cell_budget_after_change(doc, None, sheet.archetype.allocated_cell_slots())?;
    doc.add_data_sheet(sheet)?;
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), None, Some(ctx.cancelled))?;

    Ok(CommandOutcome::mutated(
        "table.simulateDistributions",
        format!("simulated {col_count} column(s) x {} rows into `{}`", p.n_rows, id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "title": title,
            "kind": kind,
            "n_rows": p.n_rows,
            "n_cols": col_count,
            "seed": seed,
            "use_multiple_variables": use_mv,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}
