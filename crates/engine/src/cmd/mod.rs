//! Typed `CommandSpec` registry, side-effect metadata, execution context, and markdown docs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CommandOutcome, EngineError, ErrorCode, Session};

pub mod effects;
pub mod helpers;
pub mod params;

pub mod analysis;
pub mod edit;
pub mod export;
pub mod graph;
pub mod image;
pub mod info;
pub mod layout;
pub mod project;
pub mod sheet;
pub mod table;

pub use effects::Effects;
pub use params::parse_params;

/// Logical area grouping for registered engine commands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandGroup {
    Project,
    Edit,
    Sheet,
    Table,
    Info,
    Analysis,
    Graph,
    Layout,
    Export,
}

impl CommandGroup {
    /// Canonical lowercase identifier for this command group.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Edit => "edit",
            Self::Sheet => "sheet",
            Self::Table => "table",
            Self::Info => "info",
            Self::Analysis => "analysis",
            Self::Graph => "graph",
            Self::Layout => "layout",
            Self::Export => "export",
        }
    }

    /// Human-readable section heading for documentation and menus.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Project => "Project & Files",
            Self::Edit => "Edit & History",
            Self::Sheet => "Sheets & Pipelines",
            Self::Table => "Data Tables",
            Self::Info => "Info & Constants",
            Self::Analysis => "Analysis & Reports",
            Self::Graph => "Graphs & Styling",
            Self::Layout => "Page Layouts",
            Self::Export => "Vector & Raster Export",
        }
    }

    /// All command groups in canonical documentation order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Project, Self::Edit, Self::Sheet, Self::Table, Self::Info, Self::Analysis, Self::Graph, Self::Layout, Self::Export]
    }
}

/// Originating frontend or automation channel invoking a command.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Caller {
    Ui,
    Cli,
    Mcp,
    Control,
    Replay,
}

/// Mutable execution context passed to every registered command handler.
pub struct Ctx<'a> {
    pub session: &'a mut Session,
    pub caller: Caller,
    pub cancelled: &'a AtomicBool,
}

impl Ctx<'_> {
    /// Return `true` when the caller requested cancellation.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

macro_rules! command_specs {
    (
        $group:expr;
        $(
            {
                id: $id:expr,
                label: $label:expr,
                menu_path: $menu_path:expr,
                $(shortcut: $shortcut:expr,)?
                summary: $summary:expr,
                beginner: $beginner:expr,
                effects: $effects:expr,
                input_schema: $input_schema:expr,
                enabled: $enabled:expr,
                run: $run:expr $(,)?
            }
        ),* $(,)?
    ) => {
        [
            $(
                $crate::cmd::CommandSpec {
                    id: $id,
                    label: $label,
                    menu_path: $menu_path,
                    shortcut: "",
                    summary: $summary,
                    beginner: $beginner,
                    group: $group,
                    effects: $effects,
                    input_schema: $input_schema,
                    enabled: $enabled,
                    run: $run,
                }
                $(.with_shortcut($shortcut))?
            ),*
        ]
    };
}
pub(super) use command_specs;

/// Declarative, metadata-rich definition of one engine command.
#[derive(Copy, Clone)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub menu_path: &'static str,
    pub shortcut: &'static str,
    pub summary: &'static str,
    pub beginner: &'static str,
    pub group: CommandGroup,
    pub effects: Effects,
    pub input_schema: fn() -> Value,
    pub enabled: fn(&Session) -> bool,
    pub run: fn(&mut Ctx<'_>, Value) -> Result<CommandOutcome, EngineError>,
}

impl CommandSpec {
    /// Attach a keyboard shortcut to this command specification.
    #[must_use]
    pub const fn with_shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = shortcut;
        self
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandSpec")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("group", &self.group)
            .field("effects", &self.effects)
            .finish_non_exhaustive()
    }
}

/// Immutable lookup registry of all compiled [`CommandSpec`] definitions.
#[derive(Debug, Clone)]
pub struct Registry {
    specs: BTreeMap<&'static str, CommandSpec>,
}

impl Registry {
    /// Construct an empty command registry.
    #[must_use]
    pub const fn empty() -> Self {
        Self { specs: BTreeMap::new() }
    }

    /// Build a registry from `specs`, rejecting empty or duplicate command identifiers.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::BadInput`] for empty identifiers or
    /// [`ErrorCode::AlreadyExists`] if two entries share an identifier.
    pub fn from_specs<'a>(specs: impl IntoIterator<Item = &'a CommandSpec>) -> Result<Self, EngineError> {
        let mut map = BTreeMap::new();
        for spec in specs {
            if spec.id.trim().is_empty() {
                return Err(EngineError::with_code(ErrorCode::BadInput, "command specification id must not be empty"));
            }
            if map.insert(spec.id, *spec).is_some() {
                return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("duplicate command id `{}` in registry", spec.id))
                    .with_details(json!({ "command_id": spec.id })));
            }
        }
        Ok(Self { specs: map })
    }

    /// Look up a command specification by its canonical dot-delimited ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.specs.get(id)
    }

    /// Iterate over all registered command specifications sorted by ID.
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.specs.values()
    }

    /// Return all command specifications belonging to `group` sorted by ID.
    #[must_use]
    pub fn specs_by_group(&self, group: CommandGroup) -> Vec<&CommandSpec> {
        self.specs.values().filter(|spec| spec.group == group).collect()
    }

    /// Total number of registered command specifications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Return `true` when zero command specifications are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

/// Assemble the workspace engine command registry across all nine area modules.
///
/// # Errors
/// Returns [`EngineError`] if any command ID is duplicated across modules.
pub fn build_registry() -> Result<Registry, EngineError> {
    let groups: [&[CommandSpec]; 9] = [
        project::specs(),
        edit::specs(),
        sheet::specs(),
        table::specs(),
        info::specs(),
        analysis::specs(),
        graph::specs(),
        layout::specs(),
        export::specs(),
    ];
    Registry::from_specs(groups.into_iter().flatten())
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Return the process-wide initialized command registry.
#[must_use]
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| match build_registry() {
        Ok(reg) => reg,
        Err(_) => Registry::empty(),
    })
}

/// Render human- and LLM-readable Markdown documentation for all commands in [`registry()`].
#[must_use]
pub fn render_commands_markdown() -> String {
    render_registry_markdown(registry())
}

/// Render grouped Markdown documentation for `reg`.
#[must_use]
pub fn render_registry_markdown(reg: &Registry) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# CurveBook Command Reference\n");
    let _ = writeln!(out, "Registered commands: {}\n", reg.len());
    for &group in CommandGroup::all() {
        let items = reg.specs_by_group(group);
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(out, "## {} (`{}`)\n", group.title(), group.as_str());
        for spec in items {
            append_spec_markdown(&mut out, spec);
        }
    }
    out
}

fn append_spec_markdown(out: &mut String, spec: &CommandSpec) {
    let _ = writeln!(out, "### `{}` — {}\n", spec.id, spec.label);
    let flags = spec.effects.flag_names();
    let effects_str = if flags.is_empty() { String::from("NONE") } else { flags.join(" | ") };
    let _ = writeln!(out, "- **Effects:** `{effects_str}`");
    if !spec.menu_path.is_empty() {
        let _ = writeln!(out, "- **Menu:** `{}`", spec.menu_path);
    }
    if !spec.shortcut.is_empty() {
        let _ = writeln!(out, "- **Shortcut:** `{}`", spec.shortcut);
    }
    let _ = writeln!(out, "- **Summary:** {}", spec.summary);
    if !spec.beginner.is_empty() {
        let _ = writeln!(out, "- **Beginner note:** {}", spec.beginner);
    }
    let schema = (spec.input_schema)();
    let params_line = summarize_schema_properties(&schema);
    let _ = writeln!(out, "- **Parameters:** {params_line}\n");
}

fn summarize_schema_properties(schema: &Value) -> String {
    let required: BTreeSet<&str> =
        schema.get("required").and_then(Value::as_array).map(|arr| arr.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return String::from("(dynamic)");
    };
    if props.is_empty() {
        return String::from("(none)");
    }
    let parts: Vec<String> = props
        .iter()
        .map(|(name, prop_schema)| {
            let ty = prop_schema
                .get("type")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.clone()),
                    Value::Array(arr) => {
                        let joined: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
                        Some(joined.join("|"))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| String::from("any"));
            let req = if required.contains(name.as_str()) { "*" } else { "" };
            format!("`{name}{req}` ({ty})")
        })
        .collect();
    parts.join(", ")
}

/// Rank up to `limit` unique candidate identifiers closest to `query` using token overlap + edit distance.
#[must_use]
pub fn fuzzy_suggest<'a>(query: &str, token_sep: char, candidates: impl IntoIterator<Item = &'a str>, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let q_norm = query.trim().to_ascii_lowercase().replace('-', &token_sep.to_string());
    let q_tokens: Vec<&str> = q_norm.split(token_sep).filter(|t| !t.is_empty()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for cand in candidates {
        let trimmed = cand.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
            continue;
        }
        let c_lower = trimmed.to_ascii_lowercase();
        let c_tokens: Vec<&str> = c_lower.split(token_sep).filter(|t| !t.is_empty()).collect();
        let shared = q_tokens.iter().filter(|tok| c_tokens.contains(tok)).count();
        let contains = usize::from(c_lower.contains(&q_norm) || q_norm.contains(&c_lower)) * 4;
        let bonus = shared * 6 + contains;
        let score = levenshtein_distance(&q_norm, &c_lower).saturating_sub(bonus);
        scored.push((score, trimmed.to_owned()));
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, name)| name).collect()
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = Vec::with_capacity(b_chars.len() + 1);
    for (i, ca) in a.chars().enumerate() {
        curr.clear();
        curr.push(i + 1);
        let mut left = i + 1;
        for (j, cb) in b_chars.iter().copied().enumerate() {
            let diag = prev.get(j).copied().unwrap_or(usize::MAX);
            let up = prev.get(j + 1).copied().unwrap_or(usize::MAX);
            let cost = usize::from(ca != cb);
            let cell = diag.saturating_add(cost).min(up.saturating_add(1)).min(left.saturating_add(1));
            curr.push(cell);
            left = cell;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::params::{empty_object_schema, non_empty_str_schema, obj_schema};

    fn noop_run(ctx: &mut Ctx<'_>, _params: Value) -> Result<CommandOutcome, EngineError> {
        Ok(CommandOutcome {
            command_id: String::from("test.noop"),
            mutated: false,
            message: format!("caller={:?}", ctx.caller),
            payload: json!({}),
            warnings: Vec::new(),
        })
    }

    fn always_enabled(_session: &Session) -> bool {
        true
    }

    #[test]
    fn build_registry_succeeds_without_duplicates() {
        let reg = build_registry().expect("built-in registry should not have duplicate IDs");
        assert_eq!(reg.len(), registry().len());
    }

    #[test]
    fn from_specs_rejects_duplicate_and_empty_ids() {
        let spec_a = CommandSpec {
            id: "project.inspect",
            label: "Inspect",
            menu_path: "File > Inspect",
            shortcut: "",
            summary: "Inspect document",
            beginner: "Returns summary metadata.",
            group: CommandGroup::Project,
            effects: Effects::NONE | Effects::IDEMPOTENT,
            input_schema: empty_object_schema,
            enabled: always_enabled,
            run: noop_run,
        };
        let dup = Registry::from_specs([&spec_a, &spec_a]).expect_err("duplicate IDs must fail");
        assert_eq!(dup.code, ErrorCode::AlreadyExists);

        let empty_id = CommandSpec { id: "  ", ..spec_a };
        let blank = Registry::from_specs([&empty_id]).expect_err("empty ID must fail");
        assert_eq!(blank.code, ErrorCode::BadInput);
    }

    #[test]
    fn renders_markdown_grouped_with_schema_and_effects() {
        let schema_fn = || obj_schema(vec![("sheet_id", non_empty_str_schema("Sheet identifier"))], &["sheet_id"], false);
        let spec_a = CommandSpec {
            id: "sheet.freeze",
            label: "Freeze Sheet",
            menu_path: "Sheet > Freeze",
            shortcut: "Cmd+L",
            summary: "Lock cells against accidental edits.",
            beginner: "Frozen tables keep analyses active while preventing edits.",
            group: CommandGroup::Sheet,
            effects: Effects::MUTATES_DOCUMENT | Effects::IDEMPOTENT,
            input_schema: schema_fn,
            enabled: always_enabled,
            run: noop_run,
        };
        let reg = Registry::from_specs([&spec_a]).expect("unique spec");
        let md = render_registry_markdown(&reg);
        assert!(md.contains("## Sheets & Pipelines (`sheet`)"));
        assert!(md.contains("`sheet.freeze`"));
        assert!(md.contains("MUTATES_DOCUMENT | IDEMPOTENT"));
        assert!(md.contains("`sheet_id*` (string)"));
        assert!(md.contains("Beginner note:"));
    }
}
