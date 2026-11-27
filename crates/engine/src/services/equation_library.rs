//! Versioned atomic User Equation Library persisted under `CURVEBOOK_CONFIG_DIR`.
//!
//! Provides load/save/list/get/delete/validate operations for user-defined curve-fitting DSL
//! equations (supporting `[Constraints]`, `[Transforms]`, `[InitialValues]`, `<A>` dataset
//! selectors, implicit equations, and 1st-order ODEs), plus a `"Clone built-in equation"` helper
//! generating valid editable DSL templates for every built-in equation in `curvebook-fit`.
//!
//! All disk writes use [`curvebook_platform::fs::write_atomic`] so the store file
//! (`user_equations.json`) is crash-safe and never partially written.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use curvebook_doc::ErrorCode;
use curvebook_fit::{CompiledDslModel, EquationCategory, ParametricModel, get_builtin_model};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::fs::{read_regular_file_bytes_if_exists, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::EngineError;

/// Current on-disk JSON schema version for the user equation library store.
pub const USER_EQUATION_LIBRARY_SCHEMA_VERSION: u32 = 1;

/// Canonical filename inside `CURVEBOOK_CONFIG_DIR`.
pub const USER_EQUATION_LIBRARY_FILE_NAME: &str = "user_equations.json";

/// Maximum file size permitted when reading `user_equations.json` (`4 MiB`).
pub const MAX_USER_EQUATION_LIBRARY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum number of saved user equations permitted in one store.
pub const MAX_USER_EQUATIONS: usize = 512;

/// Maximum length of a user equation identifier slug in bytes.
pub const MAX_USER_EQUATION_ID_LEN: usize = 96;

/// Maximum length of a user equation display name or category in bytes.
pub const MAX_USER_EQUATION_NAME_LEN: usize = 160;

/// Maximum length of a user equation description in bytes.
pub const MAX_USER_EQUATION_DESC_LEN: usize = 2_048;

/// One persisted user-defined equation entry in [`UserEquationStoreFile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEquationEntry {
    /// Canonical normalized identifier slug (lowercase ASCII alphanumerics and underscores).
    pub id: String,
    /// Human-readable equation title.
    pub name: String,
    /// User-defined or built-in equation category label.
    #[serde(default = "default_user_category")]
    pub category: String,
    /// Optional notes or citation text for this equation.
    #[serde(default)]
    pub description: String,
    /// Custom DSL source compiled by [`CompiledDslModel::compile`] (may include `[InitialValues]`,
    /// `[Constraints]`, and `[Transforms]` sections).
    pub dsl_source: String,
    /// Canonical built-in equation ID when created via [`clone_builtin_equation_template`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloned_from_builtin: Option<String>,
    /// Ordered fitted parameter names extracted from compiling `dsl_source`.
    #[serde(default)]
    pub param_names: Vec<String>,
    /// Parameter names declared as mandatory constants (`PARAM = ?`) in `[Constraints]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mandatory_constants: Vec<String>,
    /// Number of relational inequality constraints extracted from `[Constraints]`.
    #[serde(default)]
    pub relational_constraints_count: usize,
    /// Number of derived scalar rules extracted from `[Transforms]`.
    #[serde(default)]
    pub transforms_count: usize,
}

fn default_user_category() -> String {
    String::from("User Equations")
}

/// Versioned on-disk JSON envelope stored in `user_equations.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEquationStoreFile {
    /// Schema format version (`1`).
    pub schema_version: u32,
    /// Saved user equation definitions sorted deterministically by `id`.
    #[serde(default)]
    pub equations: Vec<UserEquationEntry>,
}

impl Default for UserEquationStoreFile {
    fn default() -> Self {
        Self { schema_version: USER_EQUATION_LIBRARY_SCHEMA_VERSION, equations: Vec::new() }
    }
}

/// Request payload for saving or updating a user equation in [`UserEquationLibrary`].
#[derive(Debug, Clone)]
pub struct SaveUserEquationRequest {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub dsl_source: String,
    pub cloned_from_builtin: Option<String>,
    pub overwrite: bool,
}

/// Validation summary returned by [`validate_user_equation_dsl`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedUserEquation {
    pub id: String,
    pub param_names: Vec<String>,
    pub mandatory_constants: Vec<String>,
    pub relational_constraints_count: usize,
    pub transforms_count: usize,
    pub warnings: Vec<String>,
}

/// Atomic file-backed store for user-defined curve-fitting DSL equations.
#[derive(Debug, Clone)]
pub struct UserEquationLibrary {
    config_dir: PathBuf,
}

impl UserEquationLibrary {
    /// Construct a library store rooted at an explicit configuration directory.
    #[must_use]
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self { config_dir: config_dir.into() }
    }

    /// Resolve the default configuration directory from `CURVEBOOK_CONFIG_DIR` or OS standard paths.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::IoError`] if no configuration directory can be
    /// determined from the environment.
    pub fn default_config_dir() -> Result<PathBuf, EngineError> {
        if let Some(raw) = std::env::var_os("CURVEBOOK_CONFIG_DIR")
            && !raw.is_empty()
        {
            return Ok(PathBuf::from(raw));
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
                return Ok(PathBuf::from(home).join("Library").join("Application Support").join("CurveBook"));
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(appdata) = std::env::var_os("APPDATA").filter(|p| !p.is_empty()) {
                return Ok(PathBuf::from(appdata).join("CurveBook"));
            }
            if let Some(profile) = std::env::var_os("USERPROFILE").filter(|p| !p.is_empty()) {
                return Ok(PathBuf::from(profile).join("AppData").join("Roaming").join("CurveBook"));
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
                return Ok(PathBuf::from(xdg).join("curvebook"));
            }
            if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
                return Ok(PathBuf::from(home).join(".config").join("curvebook"));
            }
        }
        Err(EngineError::with_code(ErrorCode::IoError, "cannot resolve CurveBook configuration directory (set `CURVEBOOK_CONFIG_DIR`)"))
    }

    /// Open the default user equation library under `CURVEBOOK_CONFIG_DIR` (or OS config dir).
    ///
    /// # Errors
    /// Returns [`EngineError`] if the configuration directory cannot be resolved.
    pub fn open_default() -> Result<Self, EngineError> {
        Ok(Self::new(Self::default_config_dir()?))
    }

    /// Configuration directory backing this library.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Full path to `user_equations.json`.
    #[must_use]
    pub fn store_path(&self) -> PathBuf {
        self.config_dir.join(USER_EQUATION_LIBRARY_FILE_NAME)
    }

    /// Load the versioned store file from disk, returning an empty v1 store when absent.
    ///
    /// # Errors
    /// Returns [`EngineError`] on I/O error, oversized file, invalid JSON, or unsupported version.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_store(&self) -> Result<UserEquationStoreFile, EngineError> {
        let path = self.store_path();
        let Some(bytes) = read_regular_file_bytes_if_exists(&path, MAX_USER_EQUATION_LIBRARY_BYTES)? else {
            return Ok(UserEquationStoreFile::default());
        };
        let mut store: UserEquationStoreFile = serde_json::from_slice(&bytes).map_err(|err| {
            EngineError::with_code(ErrorCode::BadInput, format!("corrupt user equation library `{}`: {err}", path.display()))
                .with_remediation("Repair or remove the corrupted `user_equations.json` store.")
        })?;
        if store.schema_version == 0 || store.schema_version > USER_EQUATION_LIBRARY_SCHEMA_VERSION {
            return Err(EngineError::with_code(
                ErrorCode::UnsupportedVersion,
                format!(
                    "unsupported user equation library schema version {} (max supported: {USER_EQUATION_LIBRARY_SCHEMA_VERSION})",
                    store.schema_version
                ),
            ));
        }
        store.equations.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(store)
    }

    /// Load store stub on `wasm32` targets without host filesystem access.
    #[cfg(target_arch = "wasm32")]
    pub fn load_store(&self) -> Result<UserEquationStoreFile, EngineError> {
        Ok(UserEquationStoreFile::default())
    }

    /// Persist `store` atomically to `user_equations.json`.
    #[cfg(not(target_arch = "wasm32"))]
    fn persist_store(&self, store: &mut UserEquationStoreFile) -> Result<(), EngineError> {
        store.schema_version = USER_EQUATION_LIBRARY_SCHEMA_VERSION;
        store.equations.sort_by(|a, b| a.id.cmp(&b.id));
        std::fs::create_dir_all(&self.config_dir).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed creating config dir `{}`: {err}", self.config_dir.display()))
        })?;
        let bytes = serde_json::to_vec_pretty(store)
            .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed serializing user equations: {err}")))?;
        write_atomic(&self.store_path(), &bytes)?;
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn persist_store(&self, _store: &mut UserEquationStoreFile) -> Result<(), EngineError> {
        Err(EngineError::with_code(ErrorCode::Unsupported, "host user equation library persistence is unsupported on wasm32"))
    }

    /// List all saved user equations in deterministic `id` order.
    ///
    /// # Errors
    /// Returns [`EngineError`] if loading the store fails.
    pub fn list(&self) -> Result<Vec<UserEquationEntry>, EngineError> {
        Ok(self.load_store()?.equations)
    }

    /// Look up one saved user equation by canonical or normalized identifier.
    ///
    /// # Errors
    /// Returns [`EngineError`] if loading the store fails.
    pub fn get(&self, raw_id: &str) -> Result<Option<UserEquationEntry>, EngineError> {
        let norm = normalize_user_equation_id(raw_id);
        if norm.is_empty() {
            return Ok(None);
        }
        Ok(self.load_store()?.equations.into_iter().find(|e| e.id == norm))
    }

    /// Validate and atomically save (or update when `overwrite == true`) a user equation.
    ///
    /// # Errors
    /// Returns [`EngineError`] on invalid ID/name/DSL, collision with a built-in equation ID,
    /// duplicate user ID when `overwrite == false`, capacity limit exceeded, or I/O failure.
    pub fn save(&self, req: SaveUserEquationRequest) -> Result<UserEquationEntry, EngineError> {
        let id = validate_user_equation_id(&req.id)?;
        if get_builtin_model(&id).is_some() {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("user equation id `{id}` collides with a built-in equation; choose a distinct id (for example `{id}_custom`)"),
            )
            .with_details(json!({ "id": id })));
        }
        let name = validate_non_empty_text("name", &req.name, MAX_USER_EQUATION_NAME_LEN)?;
        let category = match req.category {
            Some(raw) if !raw.trim().is_empty() => validate_non_empty_text("category", &raw, MAX_USER_EQUATION_NAME_LEN)?,
            _ => default_user_category(),
        };
        let description = match req.description {
            Some(raw) => {
                let trimmed = raw.trim();
                if trimmed.len() > MAX_USER_EQUATION_DESC_LEN {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("`description` exceeds maximum length ({MAX_USER_EQUATION_DESC_LEN} bytes)"),
                    ));
                }
                trimmed.to_owned()
            }
            None => String::new(),
        };

        let validated = validate_user_equation_dsl(&id, &req.dsl_source)?;
        let entry = UserEquationEntry {
            id: id.clone(),
            name,
            category,
            description,
            dsl_source: req.dsl_source.trim().to_owned(),
            cloned_from_builtin: req.cloned_from_builtin.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            param_names: validated.param_names,
            mandatory_constants: validated.mandatory_constants,
            relational_constraints_count: validated.relational_constraints_count,
            transforms_count: validated.transforms_count,
        };

        let mut store = self.load_store()?;
        if let Some(slot) = store.equations.iter_mut().find(|e| e.id == id) {
            if !req.overwrite {
                return Err(EngineError::with_code(
                    ErrorCode::AlreadyExists,
                    format!("user equation `{id}` already exists; pass `overwrite: true` to replace it"),
                )
                .with_remediation("Choose a unique `id` or pass `overwrite: true`.")
                .with_details(json!({ "id": id })));
            }
            *slot = entry.clone();
        } else {
            if store.equations.len() >= MAX_USER_EQUATIONS {
                return Err(EngineError::with_code(
                    ErrorCode::ResourceLimit,
                    format!("user equation library reached maximum capacity ({MAX_USER_EQUATIONS} equations)"),
                ));
            }
            store.equations.push(entry.clone());
        }
        self.persist_store(&mut store)?;
        Ok(entry)
    }

    /// Atomically remove a saved user equation by ID. Returns `true` if an entry was removed.
    ///
    /// # Errors
    /// Returns [`EngineError`] on invalid ID or I/O failure.
    pub fn delete(&self, raw_id: &str) -> Result<bool, EngineError> {
        let id = validate_user_equation_id(raw_id)?;
        let mut store = self.load_store()?;
        let before = store.equations.len();
        store.equations.retain(|e| e.id != id);
        let removed = store.equations.len() < before;
        if removed {
            self.persist_store(&mut store)?;
        }
        Ok(removed)
    }

    /// Clone any built-in equation into an editable DSL template and immediately persist it.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the built-in ID does not exist, template compilation fails, or
    /// saving fails.
    pub fn clone_builtin_and_save(
        &self,
        builtin_id: &str,
        new_id: Option<&str>,
        new_name: Option<&str>,
        overwrite: bool,
    ) -> Result<UserEquationEntry, EngineError> {
        let template = clone_builtin_equation_template(builtin_id, new_id, new_name)?;
        self.save(SaveUserEquationRequest {
            id: template.id,
            name: template.name,
            category: Some(template.category),
            description: Some(template.description),
            dsl_source: template.dsl_source,
            cloned_from_builtin: template.cloned_from_builtin,
            overwrite,
        })
    }
}

/// Normalize a user equation ID into canonical lowercase snake_case.
#[must_use]
pub fn normalize_user_equation_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

/// Validate a canonical user equation identifier slug.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if empty, oversized, or containing
/// disallowed characters.
pub fn validate_user_equation_id(raw: &str) -> Result<String, EngineError> {
    let norm = normalize_user_equation_id(raw);
    if norm.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "user equation `id` must not be empty"));
    }
    if norm.len() > MAX_USER_EQUATION_ID_LEN {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("user equation `id` exceeds maximum length ({MAX_USER_EQUATION_ID_LEN} bytes)"),
        ));
    }
    if !norm.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        || !norm.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("invalid user equation `id` `{raw}`: must start with an ASCII letter and contain only letters, digits, `_`, or `-`"),
        ));
    }
    Ok(norm)
}

fn validate_non_empty_text(field: &str, raw: &str, max_bytes: usize) -> Result<String, EngineError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{field}` must not be empty")));
    }
    if trimmed.len() > max_bytes {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{field}` exceeds maximum length ({max_bytes} bytes)")));
    }
    Ok(trimmed.to_owned())
}

/// Compile and validate a custom DSL source string (including `[InitialValues]`, `[Constraints]`,
/// and `[Transforms]` sections) via [`CompiledDslModel::compile`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] containing syntax/line/column diagnostics
/// if compilation fails.
pub fn validate_user_equation_dsl(id: &str, dsl_source: &str) -> Result<ValidatedUserEquation, EngineError> {
    let trimmed = dsl_source.trim();
    if trimmed.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`dsl_source` must not be empty"));
    }
    let compiled = CompiledDslModel::compile(id, trimmed).map_err(|err| {
        EngineError::with_code(ErrorCode::BadInput, format!("invalid custom equation DSL for `{id}`: {err}"))
            .with_remediation("Check equation syntax, `[Constraints]`, `[Transforms]`, and `[InitialValues]` sections.")
    })?;
    let transforms_count = count_dsl_transform_assignments(trimmed);
    let warnings = compiled.warnings().iter().map(|w| w.message.clone()).collect();
    Ok(ValidatedUserEquation {
        id: id.to_owned(),
        param_names: compiled.param_names().to_vec(),
        mandatory_constants: compiled.mandatory_constant_params().to_vec(),
        relational_constraints_count: compiled.default_relational_constraints().len(),
        transforms_count,
        warnings,
    })
}

/// Count valid assignment statements inside `[Transforms]` / `[Derived Scalars]` sections of a
/// DSL source string already validated by [`CompiledDslModel::compile`].
///
/// Matches `curvebook_fit::dsl` logical-line continuation (`\`), `#`/`;` comment stripping, and
/// section-header aliases without evaluating expressions at a probe point (avoiding undercounting
/// transforms like `1 / (KFast - KSlow)` or `ln(Top - Bottom)` that are non-finite at `[1.0; k]`).
fn count_dsl_transform_assignments(source: &str) -> usize {
    let mut logical_lines: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for raw_line in source.lines() {
        let trimmed_end = raw_line.trim_end();
        let is_cont = trimmed_end.strip_suffix('\\').filter(|prefix| !prefix.contains('#') && !prefix.contains(';'));
        if let Some(code_prefix) = is_cont {
            if let Some(ref mut buf) = pending {
                buf.push(' ');
                buf.push_str(code_prefix.trim());
            } else {
                pending = Some(code_prefix.trim_end().to_owned());
            }
        } else if let Some(mut buf) = pending.take() {
            buf.push(' ');
            buf.push_str(raw_line.trim_start());
            logical_lines.push(buf);
        } else {
            logical_lines.push(raw_line.to_owned());
        }
    }
    if let Some(buf) = pending.take() {
        logical_lines.push(buf);
    }

    let mut in_transforms = false;
    let mut count = 0_usize;
    for raw_line in &logical_lines {
        let trimmed = raw_line.trim();
        let without_comment = trimmed
            .split_once('#')
            .into_iter()
            .chain(trimmed.split_once(';'))
            .min_by_key(|(before, _)| before.len())
            .map_or(trimmed, |(before, _)| before.trim());
        if without_comment.is_empty() {
            continue;
        }
        if let Some(inner_raw) = without_comment.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let norm: String =
                inner_raw.chars().filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-').flat_map(char::to_lowercase).collect();
            in_transforms = matches!(norm.as_str(), "transforms" | "transform" | "derived" | "derivedscalars");
            continue;
        }
        if in_transforms {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Clone any built-in equation from `curvebook-fit` into a compilable, editable
/// [`UserEquationEntry`] template without modifying the disk store.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `builtin_id` is unknown, or
/// [`ErrorCode::BadInput`] if `new_id` / `new_name` is invalid.
pub fn clone_builtin_equation_template(builtin_id: &str, new_id: Option<&str>, new_name: Option<&str>) -> Result<UserEquationEntry, EngineError> {
    let model = get_builtin_model(builtin_id).ok_or_else(|| {
        EngineError::with_code(ErrorCode::NotFound, format!("built-in equation `{builtin_id}` not found"))
            .with_remediation("Pass a canonical built-in equation ID from the equation library.")
    })?;
    let canonical = model.id().to_owned();
    let default_id = format!("{canonical}_custom");
    let id = validate_user_equation_id(new_id.unwrap_or(&default_id))?;
    if get_builtin_model(&id).is_some() {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cloned equation id `{id}` collides with a built-in equation; choose a distinct id"),
        ));
    }
    let default_name = format!("{} (Custom)", model.name());
    let name = validate_non_empty_text("name", new_name.unwrap_or(&default_name), MAX_USER_EQUATION_NAME_LEN)?;
    let dsl_source = builtin_model_to_dsl_template(&*model);
    let validated = validate_user_equation_dsl(&id, &dsl_source)?;

    Ok(UserEquationEntry {
        id,
        name,
        category: equation_category_label(model.category()).to_owned(),
        description: format!("Editable custom DSL template cloned from built-in equation `{canonical}` ({}).", model.name()),
        dsl_source,
        cloned_from_builtin: Some(canonical),
        param_names: validated.param_names,
        mandatory_constants: validated.mandatory_constants,
        relational_constraints_count: validated.relational_constraints_count,
        transforms_count: validated.transforms_count,
    })
}

fn equation_category_label(cat: EquationCategory) -> &'static str {
    match cat {
        EquationCategory::DoseResponseStimulation => "Dose-Response - Stimulation",
        EquationCategory::DoseResponseInhibition => "Dose-Response - Inhibition",
        EquationCategory::ReceptorBinding => "Receptor Binding",
        EquationCategory::EnzymeKinetics => "Enzyme Kinetics",
        EquationCategory::Exponential => "Exponential",
        EquationCategory::Growth => "Growth",
        EquationCategory::Gaussian => "Gaussian",
        EquationCategory::Peak => "Peak",
        EquationCategory::SineWave => "Sine Wave",
        EquationCategory::StandardCurve => "Standard Curve",
        EquationCategory::Polynomial => "Polynomial",
        EquationCategory::Radiobiology => "Radiobiology",
        EquationCategory::NistBenchmark => "NIST Benchmark",
        EquationCategory::UserDefined => "User Equations",
    }
}

/// Generate a valid, editable DSL template matching a built-in [`ParametricModel`].
fn builtin_model_to_dsl_template(model: &dyn ParametricModel) -> String {
    let body = builtin_dsl_body(model.id()).unwrap_or_else(|| generic_fallback_dsl(model));
    let mut out = String::new();
    let _ = writeln!(out, "# Cloned from built-in equation: {} (`{}`)", model.name(), model.id());
    let _ = writeln!(out, "{body}");

    let bounds = model.default_bounds();
    let rels = model.default_relational_constraints();
    let mandatory = model.mandatory_constant_params();
    let mut constraint_lines: Vec<String> = Vec::new();
    for (pname, (lo, hi)) in model.param_names().iter().zip(bounds.iter()) {
        match (*lo, *hi) {
            (Some(l), Some(h)) if l.is_finite() && h.is_finite() => {
                constraint_lines.push(format!("{l} <= {pname} <= {h}"));
            }
            (Some(l), None) if l.is_finite() => {
                constraint_lines.push(format!("{pname} >= {l}"));
            }
            (None, Some(h)) if h.is_finite() => {
                constraint_lines.push(format!("{pname} <= {h}"));
            }
            _ => {}
        }
    }
    for rel in &rels {
        let op = match rel.op {
            curvebook_doc::RelationalConstraintOp::GreaterThan => ">",
            curvebook_doc::RelationalConstraintOp::GreaterOrEqual => ">=",
            curvebook_doc::RelationalConstraintOp::LessThan => "<",
            curvebook_doc::RelationalConstraintOp::LessOrEqual => "<=",
        };
        if rel.margin == 0.0 {
            constraint_lines.push(format!("{} {op} {}", rel.lhs_param, rel.rhs_param));
        } else {
            constraint_lines.push(format!("{} - {} {op} {}", rel.lhs_param, rel.rhs_param, rel.margin));
        }
    }
    for m in mandatory {
        constraint_lines.push(format!("{m} = ?"));
    }
    if !constraint_lines.is_empty() {
        let _ = writeln!(out, "\n[Constraints]");
        for line in constraint_lines {
            let _ = writeln!(out, "{line}");
        }
    }

    if let Some(transforms) = builtin_dsl_transforms(model.id()) {
        let _ = writeln!(out, "\n[Transforms]\n{transforms}");
    }
    out.trim_end().to_owned()
}

fn generic_fallback_dsl(model: &dyn ParametricModel) -> String {
    let terms: Vec<String> = model.param_names().iter().enumerate().map(|(i, p)| if i == 0 { p.clone() } else { format!("{p} * X^{i}") }).collect();
    if terms.is_empty() { String::from("Y = X") } else { format!("Y = {}", terms.join(" + ")) }
}

fn builtin_dsl_transforms(id: &str) -> Option<&'static str> {
    match id {
        "log_agonist_vs_response_3pl"
        | "log_agonist_vs_response_4pl"
        | "classic_sigmoidal_dose_response"
        | "classic_sigmoidal_dose_response_variable_slope" => Some("EC50 = 10^LogEC50\nSpan = Top - Bottom"),
        "log_inhibitor_vs_response_3pl" | "log_inhibitor_vs_response_4pl" | "one_site_fit_logic50" => Some("IC50 = 10^LogIC50\nSpan = Top - Bottom"),
        "one_phase_decay" => Some("HalfLife = ln(2) / K\nTau = 1 / K\nSpan = Y0 - Plateau"),
        "one_phase_association" => Some("HalfTime = ln(2) / K\nTau = 1 / K\nSpan = Plateau - Y0"),
        "two_phase_decay" => Some("HalfLifeFast = ln(2) / KFast\nHalfLifeSlow = ln(2) / KSlow\nSpan = Y0 - Plateau"),
        "exponential_growth" | "malthusian_growth" => Some("DoublingTime = ln(2) / k"),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn builtin_dsl_body(id: &str) -> Option<String> {
    if let Some(s) = pharma_dsl_body(id) {
        return Some(s.to_owned());
    }
    if let Some(s) = curves_dsl_body(id) {
        return Some(s);
    }
    nist_dsl_body(id).map(str::to_owned)
}

#[allow(clippy::too_many_lines)]
fn pharma_dsl_body(id: &str) -> Option<&'static str> {
    let s = match id {
        "log_agonist_vs_response_3pl" | "classic_sigmoidal_dose_response" => "Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))",
        "log_agonist_vs_response_4pl" | "classic_sigmoidal_dose_response_variable_slope" => {
            "Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))"
        }
        "log_agonist_vs_response_normalized" => "Y = 100 / (1 + 10^(LogEC50 - X))",
        "log_agonist_vs_response_normalized_var" => "Y = 100 / (1 + 10^((LogEC50 - X) * HillSlope))",
        "agonist_vs_response_3pl" => "Y = Bottom + (Top - Bottom) * X / (EC50 + X)",
        "agonist_vs_response_4pl" => "Y = Bottom + (Top - Bottom) * X^HillSlope / (EC50^HillSlope + X^HillSlope)",
        "agonist_vs_response_normalized" => "Y = 100 * X / (EC50 + X)",
        "agonist_vs_response_normalized_var" => "Y = 100 / (1 + (EC50 / X)^HillSlope)",
        "log_agonist_vs_response_5pl" => "Y = Bottom + (Top - Bottom) / (1 + (2^(1 / S) - 1) * 10^((LogEC50 - X) * HillSlope))^S",
        "agonist_vs_response_5pl" => "Y = Bottom + (Top - Bottom) / (1 + (2^(1 / S) - 1) * (EC50 / X)^HillSlope)^S",
        "bell_shaped_dose_response" => {
            "Y = Dip + (Plateau1 - Dip) / (1 + 10^((X - LogEC50_1) * nH1)) + (Plateau2 - Dip) / (1 + 10^((LogEC50_2 - X) * nH2))"
        }
        "bell_shaped_dose_response_conc" => "Y = Dip + (Plateau1 - Dip) / (1 + (X / EC50_1)^nH1) + (Plateau2 - Dip) / (1 + (EC50_2 / X)^nH2)",
        "biphasic_dose_response" => {
            "Y = Bottom + (Top - Bottom) * (Frac / (1 + 10^((LogEC50_1 - X) * nH1)) + (1 - Frac) / (1 + 10^((LogEC50_2 - X) * nH2)))"
        }
        "biphasic_dose_response_conc" => "Y = Bottom + (Top - Bottom) * (Frac / (1 + (EC50_1 / X)^nH1) + (1 - Frac) / (1 + (EC50_2 / X)^nH2))",
        "log_agonist_vs_response_ecf" => "Y = Bottom + (Top - Bottom) / (1 + ((100 - F) / F) * 10^((LogECF - X) * HillSlope))",
        "agonist_vs_response_ecf" => "Y = Bottom + (Top - Bottom) / (1 + ((100 - F) / F) * (ECF / X)^HillSlope)",
        "ec50_shift_gaddum_schild" => {
            "LogEC50Shift = LogEC50 + log10(1 + (AntagConc * 10^pA2)^SchildSlope)\nY = Bottom + (Top - Bottom) / (1 + 10^((LogEC50Shift - X) * HillSlope))"
        }
        "ec50_shift_gaddum_schild_conc" => {
            "EC50Shift = EC50 * (1 + (AntagConc * 10^pA2)^SchildSlope)\nY = Bottom + (Top - Bottom) / (1 + (EC50Shift / X)^HillSlope)"
        }
        "ec50_shift_gaddum_schild_unity_log" => {
            "LogEC50Shift = LogEC50 + log10(1 + AntagConc * 10^pKB)\nY = Bottom + (Top - Bottom) / (1 + 10^((LogEC50Shift - X) * HillSlope))"
        }
        "ec50_shift_gaddum_schild_unity_conc" => {
            "EC50Shift = EC50 * (1 + AntagConc * 10^pKB)\nY = Bottom + (Top - Bottom) / (1 + (EC50Shift / X)^HillSlope)"
        }
        "ec50_shift_ratio_log" => {
            "LogEC50Shift = LogEC50Control + log10(EC50Ratio)\nY = Bottom + (Top - Bottom) / (1 + 10^((LogEC50Shift - X) * HillSlope))"
        }
        "ec50_shift_ratio_conc" => "Y = Bottom + (Top - Bottom) / (1 + (EC50Control * EC50Ratio / X)^HillSlope)",
        "allosteric_ec50_shift" => {
            "Ratio = (1 + ModConc / 10^LogKB) / (1 + 10^Logalpha * ModConc / 10^LogKB)\nY = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(Ratio) - X) * HillSlope))"
        }
        "allosteric_ec50_shift_conc" => {
            "Ratio = (1 + ModConc / Kb) / (1 + alpha * ModConc / Kb)\nY = Bottom + (Top - Bottom) / (1 + (EC50 * Ratio / X)^HillSlope)"
        }
        "operational_model_agonist" => {
            "<A> Y = Basal + (EffectMax - Basal) / (1 + 10^((LogEC50 - X) * n))\n<~A> Y = Basal + (EffectMax - Basal) / (1 + ((10^LogKA + 10^X) / 10^(LogTau + X))^n)"
        }
        "operational_model_agonist_conc" => {
            "<A> Y = Basal + (EffectMax - Basal) / (1 + (EC50 / X)^n)\n<~A> Y = Basal + (EffectMax - Basal) / (1 + ((KA + X) / (Tau * X))^n)"
        }
        "operational_model_depletion_log" => {
            "A = 10^X\nKA = 10^LogKA\nTau = 10^LogTau\nY = Basal + (EffectMax - Basal) / (1 + ((KA + A) / (Tau * A))^n)"
        }
        "operational_model_depletion_conc" => "Y = Basal + (EffectMax - Basal) / (1 + ((KA + X) / (Tau * X))^n)",
        "log_inhibitor_vs_response_3pl" | "one_site_fit_logic50" => "Y = Bottom + (Top - Bottom) / (1 + 10^(X - LogIC50))",
        "log_inhibitor_vs_response_4pl" => "Y = Bottom + (Top - Bottom) / (1 + 10^((X - LogIC50) * HillSlope))",
        "log_inhibitor_vs_response_normalized" => "Y = 100 / (1 + 10^(X - LogIC50))",
        "log_inhibitor_vs_response_normalized_var" => "Y = 100 / (1 + 10^((X - LogIC50) * HillSlope))",
        "inhibitor_vs_response_3pl" => "Y = Bottom + (Top - Bottom) / (1 + X / IC50)",
        "inhibitor_vs_response_4pl" => "Y = Bottom + (Top - Bottom) / (1 + (X / IC50)^HillSlope)",
        "inhibitor_vs_response_normalized" => "Y = 100 / (1 + X / IC50)",
        "inhibitor_vs_response_normalized_var" => "Y = 100 / (1 + (X / IC50)^HillSlope)",
        "absolute_ic50_4pl" => {
            "Mid = 0.5 * (Control0 + Control100)\nY = Bottom + (Top - Bottom) / (1 + ((Top - Mid) / (Mid - Bottom)) * 10^((X - LogIC50) * HillSlope))"
        }
        "absolute_ic50_4pl_conc" => {
            "Mid = 0.5 * (Control0 + Control100)\nY = Bottom + (Top - Bottom) / (1 + ((Top - Mid) / (Mid - Bottom)) * (X / IC50)^HillSlope)"
        }
        "one_site_specific_binding" | "classic_one_site_binding" => "Y = Bmax * X / (Kd + X)",
        "classic_two_site_binding" => "Y = Bmax1 * X / (Kd1 + X) + Bmax2 * X / (Kd2 + X)",
        "classic_one_site_competition" => "Y = Bottom + (Top - Bottom) / (1 + 10^(X - LogEC50))",
        "classic_two_site_competition" => {
            "Y = Bottom + (Top - Bottom) * (Fraction_1 / (1 + 10^(X - LogEC50_1)) + (1 - Fraction_1) / (1 + 10^(X - LogEC50_2)))"
        }
        "one_site_specific_hill" => "Y = Bmax * X^h / (Kd^h + X^h)",
        "one_site_total_and_ns" => "Y = Bmax * X / (Kd + X) + NS * X + Background",
        "one_site_total_and_ns_global" => "<A> Y = Bmax * X / (Kd + X) + NS * X + Background\n<~A> Y = NS * X + Background",
        "two_site_total_and_ns_global" => {
            "<A> Y = BmaxHi * X / (KdHi + X) + BmaxLo * X / (KdLo + X) + NS * X + Background\n<~A> Y = NS * X + Background"
        }
        "one_site_specific_ligand_depletion" => "KdEff = Kd * (1 + NS)\nY = 0.5 * (Rt + X + KdEff - sqrt((KdEff + Rt - X)^2 + 4 * KdEff * X))",
        "one_site_total_ligand_depletion" => {
            "KdCpm = Kd * Vol * 1000 * SpecAct\nB = KdCpm * (1 + NS) + X * (1 + 2 * NS) + Bmax\nC = X * (NS * KdCpm + NS * X + Bmax)\nY = (2 * C) / (B + sqrt(B^2 - 4 * (1 + NS) * C))"
        }
        "one_site_competition_ligand_depletion" => {
            "KdEff = KdNM * (1 + NS)\nR = KdNM * (Y * (1 + NS) - NS * HotNM) / (HotNM - Y)\nI_nM = 10^(X + 9)\nKi_nM = 10^(LogKi + 9)\nR + R * HotNM / (KdEff + R) + R * I_nM / (Ki_nM + R) = Rt"
        }
        "allosteric_modulator_saturation" => {
            "Num = Bmax * X * (1 + alpha * ModConc / Kb)\nDen = Kd * (1 + ModConc / Kb) + X * (1 + alpha * ModConc / Kb)\nY = Num / Den"
        }
        "allosteric_modulator_competition" => {
            "Factor = (1 + 10^(X - LogKB)) / (1 + 10^(X - LogKB + LogAlpha))\nY = Y0 * (RadioligandNM + HotKdNM) / (RadioligandNM + HotKdNM * Factor)"
        }
        "homologous_one_site_competition" => "Y = Bmax * HotConc / (HotConc + 10^X + Kd) + NS",
        "two_sites_specific_binding" => "Y = BmaxHi * X / (KdHi + X) + BmaxLo * X / (KdLo + X)",
        "one_site_fit_ki" => "LogIC50 = LogKi + log10(1 + HotConc_nM / HotKd_nM)\nY = Bottom + (Top - Bottom) / (1 + 10^(X - LogIC50))",
        "two_sites_fit_logic50" => {
            "Y = Bottom + (Top - Bottom) * (FractionHi / (1 + 10^(X - LogIC50_Hi)) + (1 - FractionHi) / (1 + 10^(X - LogIC50_Lo)))"
        }
        "two_sites_fit_ki" => {
            "LogIC50Hi = LogKiHi + log10(1 + HotNM / HotKdNMHi)\nLogIC50Lo = LogKiLo + log10(1 + HotNM / HotKdNMLo)\nY = Bottom + (Top - Bottom) * (FractionHi / (1 + 10^(X - LogIC50Hi)) + (1 - FractionHi) / (1 + 10^(X - LogIC50Lo)))"
        }
        "dissociation_one_phase" => "Y = (Y0 - NS) * exp(-Koff * X) + NS",
        "association_kinetics_one_conc" => "Lig = Hotnm * 1e-9\nKob = Kon * Lig + Koff\nY = (Bmax * Kon * Lig / Kob) * (1 - exp(-Kob * X))",
        "association_kinetics_two_conc" => "Kob = Kon * LigandConc + Koff\nY = (Bmax * Kon * LigandConc / Kob) * (1 - exp(-Kob * X))",
        "assoc_then_dissoc" => {
            "Kob = Kon * LigandConc + Koff\nEq = Bmax * Kon * LigandConc / Kob\nY = Eq * (1 - exp(-Kob * min(X, Time0))) * exp(-Koff * max(0, X - Time0))"
        }
        "motulsky_mahan_kinetics" => {
            "KA = kon1 * L + koff1\nKB = kon2 * I + koff2\nS = sqrt((KA - KB)^2 + 4 * kon1 * kon2 * L * I)\nKF = 0.5 * (KA + KB + S)\nKS = 0.5 * (KA + KB - S)\nY = (Bmax * kon1 * L / (KF - KS)) * (koff2 * (KF - KS) / (KF * KS) + (koff2 - KF) / KF * exp(-KF * X) - (koff2 - KS) / KS * exp(-KS * X))"
        }
        "michaelis_menten" => "Y = Vmax * X / (Km + X)",
        "kcat_enzyme_kinetics" => "Y = Et * kcat * X / (Km + X)",
        "allosteric_sigmoidal_kinetics" => "Y = Vmax * X^h / (Khalf^h + X^h)",
        "substrate_inhibition" => "Y = Vmax * X / (Km + X * (1 + X / Ki))",
        "competitive_inhibition" => "Y = Vmax * X / (Km * (1 + I / Ki) + X)",
        "noncompetitive_inhibition" => "Y = Vmax * X / ((1 + I / Ki) * (Km + X))",
        "uncompetitive_inhibition" => "Y = Vmax * X / (Km + X * (1 + I / alphaKi))",
        "mixed_model_inhibition" => "Y = Vmax * X / (Km * (1 + I / Ki) + X * (1 + I / (Alpha * Ki)))",
        "morrison_tight_binding_ki" => "KiApp = Ki * (1 + S / Km)\nB = Et + X + KiApp\nY = Vo * (1 - (B - sqrt(B^2 - 4 * Et * X)) / (2 * Et))",
        _ => return None,
    };
    Some(s)
}

#[allow(clippy::too_many_lines)]
fn curves_dsl_body(id: &str) -> Option<String> {
    if let Some(deg_s) = id.strip_prefix("polynomial_")
        && let Ok(deg) = deg_s.parse::<usize>()
        && (2..=6).contains(&deg)
    {
        let mut terms = vec![String::from("B0"), String::from("B1 * X")];
        for p in 2..=deg {
            terms.push(format!("B{p} * X^{p}"));
        }
        return Some(format!("Y = {}", terms.join(" + ")));
    }
    if let Some(deg_s) = id.strip_prefix("centered_poly_")
        && let Ok(deg) = deg_s.parse::<usize>()
        && (1..=6).contains(&deg)
    {
        let mut terms = vec![String::from("B0")];
        for p in 1..=deg {
            if p == 1 {
                terms.push(String::from("B1 * (X - XCenter)"));
            } else {
                terms.push(format!("B{p} * (X - XCenter)^{p}"));
            }
        }
        return Some(format!("Y = {}", terms.join(" + ")));
    }

    let s = match id {
        "one_phase_decay" => "Y = (Y0 - Plateau) * exp(-K * X) + Plateau",
        "two_phase_decay" => {
            "SpanFast = (Y0 - Plateau) * PercentFast / 100\nSpanSlow = (Y0 - Plateau) * (100 - PercentFast) / 100\nY = Plateau + SpanFast * exp(-KFast * X) + SpanSlow * exp(-KSlow * X)"
        }
        "three_phase_decay" => {
            "Span = Y0 - Plateau\nY = Plateau + Span * ((PercentFast / 100) * exp(-KFast * X) + (PercentSlow / 100) * exp(-KSlow * X) + (1 - (PercentFast + PercentSlow) / 100) * exp(-KVerySlow * X))"
        }
        "plateau_followed_by_one_phase_decay" => "Y = (Y0 - Plateau) * exp(-K * max(0, X - X0)) + Plateau",
        "one_phase_association" => "Y = Y0 + (Plateau - Y0) * (1 - exp(-K * X))",
        "two_phase_association" => {
            "Span = Plateau - Y0\nY = Y0 + Span * ((PercentFast / 100) * (1 - exp(-KFast * X)) + (1 - PercentFast / 100) * (1 - exp(-KSlow * X)))"
        }
        "plateau_followed_by_one_phase_association" => "Y = Y0 + (Plateau - Y0) * (1 - exp(-K * max(0, X - X0)))",
        "exponential_growth" | "malthusian_growth" => "Y = Y0 * exp(k * X)",
        "log_exponential_growth" => "Y = LnY0 + k * X",
        "exponential_plateau" => "Y = YM - (YM - Y0) * exp(-k * X)",
        "logistic_growth" => "Y = YM * Y0 / ((YM - Y0) * exp(-k * X) + Y0)",
        "boltzmann_sigmoid" => "Y = Bottom + (Top - Bottom) / (1 + exp((V50 - X) / Slope))",
        "gompertz_growth" => "Y = YM * (Y0 / YM)^(exp(-k * X))",
        "beta_growth_then_decay" => "Y = IF(X <= te, Ym * (1 + (te - X) / (te - tm)) * (X / te)^(te / (te - tm)), Ym)",
        "linear_quadratic_survival" => "Y = exp(-(Alpha * X + Beta * X^2))",
        "linear_quadratic_percent_surviving" => "Y = 100 * exp(-(Alpha * X + Beta * X^2))",
        "linear_quadratic_number_surviving" => "Y = Y0 * exp(-(Alpha * X + Beta * X^2))",
        "linear_quadratic_fraction_dead" => "Y = 1 - exp(-(Alpha * X + Beta * X^2))",
        "linear_quadratic_percent_dead" => "Y = 100 * (1 - exp(-(Alpha * X + Beta * X^2)))",
        "linear_quadratic_number_dead" => "Y = Y0 * (1 - exp(-(Alpha * X + Beta * X^2)))",
        "gaussian" => "Y = Amplitude * exp(-0.5 * ((X - Mean) / SD)^2)",
        "gaussian_baseline" => "Y = Baseline + Amplitude * exp(-0.5 * ((X - Mean) / SD)^2)",
        "gaussian_area" => "Y = (Area / (SD * sqrt(2 * PI))) * exp(-0.5 * ((X - Mean) / SD)^2)",
        "sum_of_two_gaussians" => "Y = Amp1 * exp(-0.5 * ((X - Mean1) / SD1)^2) + Amp2 * exp(-0.5 * ((X - Mean2) / SD2)^2)",
        "log_gaussian" => "Y = Amplitude * exp(-0.5 * (ln(X / GeoMean) / ln(GeoSD))^2)",
        "lognormal_area" => "Y = (A / X) * exp(-0.5 * (ln(X / GeoMean) / ln(GeoSD))^2)",
        "cumulative_gaussian" => "Y = Bottom + (Top - Bottom) * 0.5 * (1 + erf((X - Mean) / (SD * SQRT2)))",
        "cumulative_gaussian_percent" => "Y = 100 * normcdf((X - Mean) / SD)",
        "cumulative_gaussian_fraction" => "Y = normcdf((X - Mean) / SD)",
        "cumulative_gaussian_counts" => "Y = N * normcdf((X - Mean) / SD)",
        "lorentzian" => "Y = Amplitude / (1 + ((X - Center) / Width)^2)",
        "sum_of_two_lorentzians" => "Y = Amplitude1 / (1 + ((X - Center1) / Width1)^2) + Amplitude2 / (1 + ((X - Center2) / Width2)^2)",
        "weibull_peak" => "Y = Amplitude * (Shape / Scale) * (X / Scale)^(Shape - 1) * exp(-(X / Scale)^Shape)",
        "sine_wave" => "Y = Amplitude * sin(2 * PI * X / Wavelength + PhaseShift) + Baseline",
        "standard_sine_wave" => "Y = Amplitude * sin(2 * PI * X / Wavelength + PhaseShift)",
        "damped_sine_wave" => "Y = Amplitude * exp(-K * X) * sin(2 * PI * X / Wavelength + PhaseShift)",
        "sinc_wave" => "U = 2 * PI * X / Wavelength\nY = IF(abs(U) < 1e-12, Amplitude, Amplitude * sin(U) / U)",
        "horizontal_line" => "Y = Mean",
        "straight_line" => "Y = Slope * X + YIntercept",
        "line_through_origin" => "Y = Slope * X",
        "pade_1_1" => "Y = (A0 + A1 * X) / (1 + B1 * X)",
        "two_lines_intersection" => "Y = YCross + Slope * (X - XCross)",
        "segmental_linear" => "Y = Intercept1 + Slope1 * min(X, X0) + Slope2 * max(0, X - X0)",
        "hinge_smooth_hyperbolic" => "D = X - X0\nY = Y0 + Slope1 * D + 0.5 * (Slope2 - Slope1) * (D + sqrt(D^2 + Delta^2) - abs(Delta))",
        "power_law" => "Y = a * X^b",
        "power_series_two_term" => "Y = A * X^B + C * X^D",
        "logarithmic_curve" => "Y = a * ln(X) + b",
        "semi_log_line" => "Y = Slope * log10(X) + YIntercept",
        "semilog_x_lin_y_log" => "Y = 10^(Slope * X + YIntercept)",
        "log_log_line" => "Y = 10^(Slope * log10(X) + LogYIntercept)",
        "probit_line" => "Y = 100 * normcdf(Slope * X + Intercept)",
        "classic_one_phase_decay" => "Y = Span * exp(-K * X) + Plateau",
        "classic_two_phase_decay" => "Y = Span1 * exp(-K1 * X) + Span2 * exp(-K2 * X) + Plateau",
        "classic_one_phase_association" => "Y = Ymax * (1 - exp(-K * X))",
        "classic_two_phase_association" => "Y = Ymax1 * (1 - exp(-K1 * X)) + Ymax2 * (1 - exp(-K2 * X))",
        "classic_exponential_growth" => "Y = Start * exp(K * X)",
        "classic_sine_wave" => "Y = Baseline + Amplitude * sin(2 * PI * Frequency * X + PhaseShift)",
        _ => return None,
    };
    Some(s.to_owned())
}

fn nist_dsl_body(id: &str) -> Option<&'static str> {
    let base = id.strip_prefix("nist_").unwrap_or(id);
    let s = match base {
        "misra1a" | "boxbod" => "Y = b1 * (1 - exp(-b2 * X))",
        "chwirut1" | "chwirut2" => "Y = exp(-b1 * X) / (b2 + b3 * X)",
        "lanczos1" | "lanczos2" | "lanczos3" => "Y = b1 * exp(-b2 * X) + b3 * exp(-b4 * X) + b5 * exp(-b6 * X)",
        "gauss1" | "gauss2" | "gauss3" => "Y = b1 * exp(-b2 * X) + b3 * exp(-((X - b4) / b5)^2) + b6 * exp(-((X - b7) / b8)^2)",
        "danwood" => "Y = b1 * X^b2",
        "misra1b" => "Y = b1 * (1 - (1 + b2 * X / 2)^(-2))",
        "kirby2" => "Y = (b1 + b2 * X + b3 * X^2) / (1 + b4 * X + b5 * X^2)",
        "hahn1" | "thurber" => "Y = (b1 + b2 * X + b3 * X^2 + b4 * X^3) / (1 + b5 * X + b6 * X^2 + b7 * X^3)",
        "nelson" => "Y = b1 - b2 * X1 * exp(-b3 * X2)",
        "mgh17" => "Y = b1 + b2 * exp(-b4 * X) + b3 * exp(-b5 * X)",
        "misra1c" => "Y = b1 * (1 - (1 + 2 * b2 * X)^(-0.5))",
        "misra1d" => "Y = b1 * b2 * X / (1 + b2 * X)",
        "roszman1" => "Y = b1 - b2 * X - atan(b3 / (X - b4)) / PI",
        "enso" => {
            "Y = b1 + b2 * cos(2 * PI * X / 12) + b3 * sin(2 * PI * X / 12) + b5 * cos(2 * PI * X / b4) + b6 * sin(2 * PI * X / b4) + b8 * cos(2 * PI * X / b7) + b9 * sin(2 * PI * X / b7)"
        }
        "mgh09" => "Y = b1 * (X^2 + b2 * X) / (X^2 + b3 * X + b4)",
        "rat42" => "Y = b1 / (1 + exp(b2 - b3 * X))",
        "mgh10" => "Y = b1 * exp(b2 / (X + b3))",
        "eckerle4" => "Y = (b1 / b2) * exp(-0.5 * ((X - b3) / b2)^2)",
        "rat43" => "Y = b1 / (1 + exp(b2 - b3 * X))^(1 / b4)",
        "bennett5" => "Y = b1 * (b2 + X)^(-1 / b3)",
        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use curvebook_fit::list_builtin_equations;

    use super::*;

    #[test]
    fn every_builtin_equation_clones_into_compilable_dsl_with_exact_parameters() {
        let builtins = list_builtin_equations();
        assert!(!builtins.is_empty());
        let mut errors = Vec::new();
        for eq in &builtins {
            match clone_builtin_equation_template(&eq.id, None, None) {
                Err(err) => errors.push(format!("{}: clone error: {err}", eq.id)),
                Ok(cloned) => match CompiledDslModel::compile(&cloned.id, &cloned.dsl_source) {
                    Err(err) => errors.push(format!("{}: compile error: {err}\n{}", eq.id, cloned.dsl_source)),
                    Ok(compiled) => {
                        let expected: BTreeSet<String> = eq.param_names.iter().cloned().collect();
                        let actual: BTreeSet<String> = compiled.param_names().iter().cloned().collect();
                        if actual != expected {
                            errors.push(format!("{}: expected {expected:?}, got {actual:?}", eq.id));
                        }
                    }
                },
            }
        }
        assert!(errors.is_empty(), "built-in clone mismatches:\n{}", errors.join("\n"));
    }

    #[test]
    fn one_phase_association_clone_uses_positive_span_and_halftime() {
        let cloned = clone_builtin_equation_template("one_phase_association", None, None).expect("clone one_phase_association");
        assert!(cloned.dsl_source.contains("HalfTime = ln(2) / K"), "missing HalfTime in:\n{}", cloned.dsl_source);
        assert!(cloned.dsl_source.contains("Span = Plateau - Y0"), "wrong Span sign in:\n{}", cloned.dsl_source);
        assert_eq!(cloned.transforms_count, 3);

        let decay = clone_builtin_equation_template("one_phase_decay", None, None).expect("clone one_phase_decay");
        assert!(decay.dsl_source.contains("HalfLife = ln(2) / K"));
        assert!(decay.dsl_source.contains("Span = Y0 - Plateau"));
        assert_eq!(decay.transforms_count, 3);
    }

    #[test]
    fn transforms_count_counts_all_valid_transforms_even_when_singular_at_unit_params() {
        let dsl = "\
Y = Bottom + (Top - Bottom) * (0.5 * exp(-KFast * X) + 0.5 * exp(-KSlow * X))
[Constraints]
KFast > KSlow
[Transforms]
# Comments and blank lines should not be counted
InvDiff = 1 / (KFast - KSlow)
LogSpan = ln(Top - Bottom)
Ratio = \\
    KFast / KSlow ; inline comment
";
        let validated = validate_user_equation_dsl("unit_singular_transforms", dsl).expect("valid DSL");
        assert_eq!(validated.transforms_count, 3);
        assert_eq!(validated.relational_constraints_count, 1);
    }
}
