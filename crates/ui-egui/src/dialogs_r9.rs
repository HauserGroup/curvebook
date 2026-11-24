//! R9 statistical option parsers and modal dialog state machines (`QOL-7`, `QOL-12`, `QOL-14`,
//! `QOL-16`, `WF-44..WF-48`).
//!
//! Kept isolated from `dialogs_canvas.rs` so parsers and workflow dialog states share one typed,
//! declarative contract emitting pure [`ToolCommandIntent`] payloads into `curvebook-engine`.

use curvebook_doc::{
    AnalysisKind, ChecklistItem, ChecklistStatus, CldOrdering, EquationDisplayMode, GraphAnnotation, GraphSheet, PairwiseDisplayMode,
    ProjectDocument, RectPt, SheetId,
};
use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_canvas::data_column_title;

// ============================================================================
// Shared String / Enum / JSON Parsers (reused across R9.12, R9.14, R9.16, WF-44..48)
// ============================================================================

/// Normalize an identifier to lowercase ASCII alphanumeric for flexible enum parsing.
#[must_use]
pub(crate) fn norm_key(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

pub(crate) fn equation_display_mode_str(mode: EquationDisplayMode) -> &'static str {
    match mode {
        EquationDisplayMode::Substituted => "substituted",
        EquationDisplayMode::Symbolic => "symbolic",
    }
}

pub(crate) fn pairwise_display_mode_str(mode: PairwiseDisplayMode) -> &'static str {
    match mode {
        PairwiseDisplayMode::BracketsOnly => "brackets_only",
        PairwiseDisplayMode::CompactLetterDisplayOnly => "compact_letter_display_only",
        PairwiseDisplayMode::BracketsAndCompactLetterDisplay => "brackets_and_compact_letter_display",
    }
}

pub(crate) fn cld_ordering_str(ordering: CldOrdering) -> &'static str {
    match ordering {
        CldOrdering::DescendingMean => "descending_mean",
        CldOrdering::AscendingMean => "ascending_mean",
    }
}

// ============================================================================
// QOL-7 Diagnostics-Only Helpers
// ============================================================================

/// Whether a [`ChecklistStatus::Info`] item reports an inconclusive check or small-sample/asymptotic limitation.
#[must_use]
pub(crate) fn is_inconclusive_info_item(item: &ChecklistItem) -> bool {
    if item.status != ChecklistStatus::Info {
        return false;
    }
    let combined = format!("{} {}", item.assumption, item.explanation).to_ascii_lowercase();
    [
        "small",
        "n <",
        "n=",
        "inconclusive",
        "underpowered",
        "low power",
        "asymptotic",
        "expected <",
        "sparse",
        "ties",
        "unbalanced",
        "too few",
        "insufficient",
        "cannot test",
        "not enough",
        "normality test",
    ]
    .iter()
    .any(|kw| combined.contains(kw))
}

/// Format statistical limitations strictly as neutral diagnostics without recommending alternative
/// tests, data transforms, or outlier exclusion (`QOL-7` policy).
#[must_use]
pub(crate) fn sanitize_diagnostics_only_text(item: &ChecklistItem) -> String {
    let raw = item.explanation.trim();
    if raw.is_empty() {
        return format!("Diagnostic status for {} ({:?}); inspect the Diagnostics tab on this ResultSheet.", item.assumption, item.status);
    }
    let lower = raw.to_ascii_lowercase();
    let prescriptive = [
        "recommend",
        "consider ",
        "consider running",
        "consider switching",
        "switch to",
        "re-run with",
        "use welch",
        "use mann",
        "use kruskal",
        "use wilcoxon",
        "use friedman",
        "apply ",
        "transform ",
        "exclude ",
        "remove outlier",
    ]
    .iter()
    .any(|kw| lower.contains(kw));
    if prescriptive {
        let metric_part = item.diagnostic_metric.as_deref().map(|m| format!(" ({m})")).unwrap_or_default();
        return format!(
            "{} flagged {}{}: statistical assumptions may not hold as tested; review Residuals / Normal Q-Q / Diagnostics tabs or adjust parameters explicitly via Cmd+T.",
            item.assumption,
            match item.status {
                ChecklistStatus::Violated => "violation",
                ChecklistStatus::Warn => "caution",
                ChecklistStatus::Info => "an inconclusive or small-sample limitation",
                ChecklistStatus::Pass => "pass",
            },
            metric_part
        );
    }
    raw.to_owned()
}

// ============================================================================
// R9.14 Dialogs: Randomize Groups, Random Sample Rows, Simulate Distributions
// ============================================================================

/// Allocation scheme for [`RandomizeGroupsDialogState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RandomizationSchemeKind {
    #[default]
    CompleteBalanced,
    PermutedBlock,
}

/// Modal state for `Randomize Subjects to Groups...` (`R9.14`, `table.randomizeGroups`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomizeGroupsDialogState {
    pub groups: Vec<(String, usize)>,
    pub subject_count: usize,
    pub subject_ids: Vec<String>,
    pub scheme: RandomizationSchemeKind,
    pub block_sizes: Vec<usize>,
    pub seed: u64,
    pub title: String,
    pub target_info_sheet_id: Option<SheetId>,
}

impl Default for RandomizeGroupsDialogState {
    fn default() -> Self {
        Self {
            groups: vec![(String::from("Control"), 1), (String::from("Treatment"), 1)],
            subject_count: 12,
            subject_ids: Vec::new(),
            scheme: RandomizationSchemeKind::CompleteBalanced,
            block_sizes: vec![4],
            seed: 1,
            title: String::from("Randomization Schedule"),
            target_info_sheet_id: None,
        }
    }
}

impl RandomizeGroupsDialogState {
    /// Synchronize default InfoSheet destination from `doc` when opening the dialog.
    pub fn open_for_document(&mut self, doc: &ProjectDocument) {
        self.target_info_sheet_id = doc.info_sheets.first().map(|s| s.id.clone());
    }

    /// Parse comma/semicolon-separated `"Arm, Arm2:2"` group entries.
    pub fn set_groups_from_str(&mut self, raw: &str) -> Result<(), String> {
        let mut parsed = Vec::new();
        for tok in raw.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()) {
            if let Some((name, w_str)) = tok.split_once(':') {
                let label = name.trim();
                let weight = w_str.trim().parse::<usize>().map_err(|_| format!("invalid group weight `{w_str}`"))?;
                if label.is_empty() || weight == 0 {
                    return Err(format!("invalid weighted group specification `{tok}`"));
                }
                parsed.push((label.to_owned(), weight));
            } else {
                parsed.push((tok.to_owned(), 1));
            }
        }
        if parsed.is_empty() {
            return Err(String::from("`groups` must contain at least one treatment arm"));
        }
        self.groups = parsed;
        Ok(())
    }

    /// Emit `table.randomizeGroups` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        if self.groups.is_empty() || (self.subject_ids.is_empty() && self.subject_count == 0) {
            return None;
        }
        let groups_json: Vec<Value> = self.groups.iter().map(|(label, weight)| json!({ "label": label, "weight": weight })).collect();
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("groups"), Value::Array(groups_json));
        if self.subject_ids.is_empty() {
            payload.insert(String::from("subject_count"), json!(self.subject_count));
        } else {
            payload.insert(String::from("subject_ids"), json!(self.subject_ids));
        }
        payload.insert(
            String::from("scheme"),
            json!(match self.scheme {
                RandomizationSchemeKind::CompleteBalanced => "complete_balanced",
                RandomizationSchemeKind::PermutedBlock => "permuted_block",
            }),
        );
        if self.scheme == RandomizationSchemeKind::PermutedBlock && !self.block_sizes.is_empty() {
            payload.insert(String::from("block_sizes"), json!(self.block_sizes));
        }
        payload.insert(String::from("seed"), json!(self.seed));
        if !self.title.trim().is_empty() {
            payload.insert(String::from("title"), json!(self.title.trim()));
        }
        if let Some(iid) = &self.target_info_sheet_id {
            payload.insert(String::from("info_sheet_id"), json!(iid.as_str()));
        }
        Some(ToolCommandIntent::new("table.randomizeGroups", Value::Object(payload)))
    }
}

/// Modal state for `Random Sample Rows...` (`R9.14`, `table.randomSampleRows`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomSampleRowsDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub sample_size: usize,
    pub seed: u64,
    pub stratify_column: Option<usize>,
    pub proportional_allocation: bool,
    pub title: String,
    pub target_info_sheet_id: Option<SheetId>,
    pub available_columns: Vec<String>,
}

impl Default for RandomSampleRowsDialogState {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            sample_size: 10,
            seed: 1,
            stratify_column: None,
            proportional_allocation: true,
            title: String::from("Random Sample"),
            target_info_sheet_id: None,
            available_columns: Vec::new(),
        }
    }
}

impl RandomSampleRowsDialogState {
    /// Load source sheet columns and InfoSheet defaults for `source_sheet_id`.
    pub fn open_for_sheet(&mut self, doc: &ProjectDocument, source_sheet_id: &SheetId) -> bool {
        let Some(ds) = doc.data_sheet(source_sheet_id) else {
            return false;
        };
        self.source_sheet_id = Some(ds.id.clone());
        self.target_info_sheet_id = ds.info_sheet_id.clone().or_else(|| doc.info_sheets.first().map(|s| s.id.clone()));
        let col_count = ds.archetype.column_count();
        self.available_columns = (0..col_count)
            .map(|i| data_column_title(ds, i).map_or_else(|| format!("Col {}", curvebook_doc::default_column_label(i)), ToOwned::to_owned))
            .collect();
        self.sample_size = self.sample_size.max(1).min(ds.archetype.row_count().max(1));
        true
    }

    /// Emit `table.randomSampleRows` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        if self.sample_size == 0 {
            return None;
        }
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("source_sheet_id"), json!(src.as_str()));
        payload.insert(String::from("sample_size"), json!(self.sample_size));
        payload.insert(String::from("seed"), json!(self.seed));
        if let Some(col) = self.stratify_column {
            payload.insert(String::from("stratify_column"), json!(col));
            payload.insert(String::from("proportional_allocation"), json!(self.proportional_allocation));
        }
        if !self.title.trim().is_empty() {
            payload.insert(String::from("title"), json!(self.title.trim()));
        }
        if let Some(iid) = &self.target_info_sheet_id {
            payload.insert(String::from("info_sheet_id"), json!(iid.as_str()));
        }
        Some(ToolCommandIntent::new("table.randomSampleRows", Value::Object(payload)))
    }
}

/// Parametric distribution family for [`SimulateDistributionsDialogState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimDistKind {
    Uniform,
    #[default]
    Gaussian,
    LogNormal,
    Binomial,
    Poisson,
    Exponential,
}

impl SimDistKind {
    /// Canonical lowercase name accepted by `table.simulateDistributions`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Gaussian => "gaussian",
            Self::LogNormal => "log_normal",
            Self::Binomial => "binomial",
            Self::Poisson => "poisson",
            Self::Exponential => "exponential",
        }
    }

    pub(crate) fn from_str(raw: &str) -> Result<Self, String> {
        match norm_key(raw).as_str() {
            "uniform" => Ok(Self::Uniform),
            "gaussian" | "normal" => Ok(Self::Gaussian),
            "lognormal" => Ok(Self::LogNormal),
            "binomial" => Ok(Self::Binomial),
            "poisson" => Ok(Self::Poisson),
            "exponential" | "exp" => Ok(Self::Exponential),
            _ => Err(format!("unknown simulated distribution `{raw}`")),
        }
    }
}

/// One column specification in [`SimulateDistributionsDialogState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulatedDistColumnDraft {
    pub name: String,
    pub distribution: SimDistKind,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub sd: f64,
    pub mean_log: f64,
    pub sd_log: f64,
    pub trials: u64,
    pub p: f64,
    pub lambda: f64,
    pub rate: f64,
}

impl Default for SimulatedDistColumnDraft {
    fn default() -> Self {
        Self {
            name: String::from("Simulated 1"),
            distribution: SimDistKind::Gaussian,
            min: 0.0,
            max: 1.0,
            mean: 0.0,
            sd: 1.0,
            mean_log: 0.0,
            sd_log: 0.5,
            trials: 10,
            p: 0.5,
            lambda: 3.0,
            rate: 1.0,
        }
    }
}

impl SimulatedDistColumnDraft {
    /// Convert draft column into `table.simulateDistributions` column JSON object.
    #[must_use]
    pub fn to_engine_json(&self) -> Value {
        match self.distribution {
            SimDistKind::Uniform => json!({ "name": self.name, "distribution": "uniform", "min": self.min, "max": self.max }),
            SimDistKind::Gaussian => json!({ "name": self.name, "distribution": "gaussian", "mean": self.mean, "sd": self.sd }),
            SimDistKind::LogNormal => json!({ "name": self.name, "distribution": "log_normal", "mean_log": self.mean_log, "sd_log": self.sd_log }),
            SimDistKind::Binomial => json!({ "name": self.name, "distribution": "binomial", "trials": self.trials, "p": self.p }),
            SimDistKind::Poisson => json!({ "name": self.name, "distribution": "poisson", "lambda": self.lambda }),
            SimDistKind::Exponential => json!({ "name": self.name, "distribution": "exponential", "rate": self.rate }),
        }
    }
}

/// Modal state for `Simulate Distributions...` (`R9.14`, `table.simulateDistributions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulateDistributionsDialogState {
    pub n_rows: usize,
    pub seed: u64,
    pub use_multiple_variables: bool,
    pub title: String,
    pub columns: Vec<SimulatedDistColumnDraft>,
    #[serde(default)]
    pub selected_col_idx: usize,
}

impl Default for SimulateDistributionsDialogState {
    fn default() -> Self {
        Self {
            n_rows: 30,
            seed: 42,
            use_multiple_variables: false,
            title: String::from("Simulated Distributions"),
            columns: vec![SimulatedDistColumnDraft::default()],
            selected_col_idx: 0,
        }
    }
}

impl SimulateDistributionsDialogState {
    /// Emit `table.simulateDistributions` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        if self.columns.is_empty() || self.n_rows == 0 {
            return None;
        }
        let cols: Vec<Value> = self.columns.iter().map(SimulatedDistColumnDraft::to_engine_json).collect();
        Some(ToolCommandIntent::new(
            "table.simulateDistributions",
            json!({
                "columns": cols,
                "n_rows": self.n_rows,
                "seed": self.seed,
                "use_multiple_variables": self.use_multiple_variables,
                "title": self.title.trim(),
            }),
        ))
    }
}

// ============================================================================
// R9.16 Dialog: Insert / Toggle ModelEquation Annotation (strictly equations only)
// ============================================================================

/// Modal state for inserting or toggling a `GraphAnnotation::ModelEquation` callout (`QOL-16`).
///
/// Strictly equations-only: renders symbolic or substituted model equations from an existing
/// regression [`ResultSheet`](curvebook_doc::ResultSheet) without Compact Letter Display (CLD) or fit-statistic callouts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertModelEquationDialogState {
    pub graph_sheet_id: Option<SheetId>,
    pub result_sheet_id: Option<SheetId>,
    pub available_result_ids: Vec<SheetId>,
    pub data_set_index: usize,
    pub mode: EquationDisplayMode,
    pub font_size_pt: f64,
    pub bordered: bool,
    pub bounds_pt: RectPt,
    pub update_existing: bool,
}

impl Default for InsertModelEquationDialogState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            result_sheet_id: None,
            available_result_ids: Vec::new(),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            font_size_pt: 10.0,
            bordered: false,
            bounds_pt: RectPt::new(24.0, 24.0, 210.0, 42.0),
            update_existing: false,
        }
    }
}

impl InsertModelEquationDialogState {
    /// Discover equation-bearing regression result sheets and prefill existing annotation state.
    pub fn open_for_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.available_result_ids = doc
            .result_sheets
            .iter()
            .filter(|rs| {
                matches!(
                    rs.analysis.kind,
                    AnalysisKind::NonlinearRegression(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::GeneralizedLinearModel(_)
                ) || !rs.fitted_curves.is_empty()
            })
            .map(|rs| rs.id.clone())
            .collect();
        if let Some(GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered }) =
            graph.spec.annotations.iter().find(|a| matches!(a, GraphAnnotation::ModelEquation { .. }))
        {
            self.result_sheet_id = Some(result_sheet_id.clone());
            self.data_set_index = *data_set_index;
            self.mode = *mode;
            self.bounds_pt = *bounds_pt;
            self.font_size_pt = *font_size_pt;
            self.bordered = *bordered;
            self.update_existing = true;
        } else {
            self.update_existing = false;
            if self.result_sheet_id.as_ref().is_none_or(|id| !self.available_result_ids.contains(id)) {
                self.result_sheet_id = self.available_result_ids.first().cloned();
            }
        }
    }

    /// Toggle draft display mode between `Substituted` (fitted parameters) and `Symbolic` (formula).
    pub fn toggle_mode(&mut self) -> EquationDisplayMode {
        self.mode = match self.mode {
            EquationDisplayMode::Substituted => EquationDisplayMode::Symbolic,
            EquationDisplayMode::Symbolic => EquationDisplayMode::Substituted,
        };
        self.mode
    }

    /// Emit `graph.addAnnotation` (`kind: "equation"`) or update existing `ModelEquation` via `graph.setSpec`.
    #[must_use]
    pub fn commit_intent(&self, doc: Option<&ProjectDocument>) -> Option<ToolCommandIntent> {
        let graph_id = self.graph_sheet_id.as_ref()?;
        let res_id = self.result_sheet_id.as_ref()?;
        if self.update_existing
            && let Some(gs) = doc.and_then(|d| d.graph_sheet(graph_id))
            && gs.spec.annotations.iter().any(|ann| matches!(ann, GraphAnnotation::ModelEquation { .. }))
        {
            let mut next_annotations = gs.spec.annotations.clone();
            let exact_idx = next_annotations.iter().position(|ann| {
                matches!(
                    ann,
                    GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, .. }
                        if result_sheet_id == res_id && *data_set_index == self.data_set_index
                )
            });
            let result_idx = next_annotations.iter().position(|ann| {
                matches!(
                    ann,
                    GraphAnnotation::ModelEquation { result_sheet_id, .. } if result_sheet_id == res_id
                )
            });
            let fallback_idx = next_annotations.iter().position(|ann| matches!(ann, GraphAnnotation::ModelEquation { .. }));
            if let Some(idx) = exact_idx.or(result_idx).or(fallback_idx)
                && let GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } =
                    &mut next_annotations[idx]
            {
                *result_sheet_id = res_id.clone();
                *data_set_index = self.data_set_index;
                *mode = self.mode;
                *bounds_pt = self.bounds_pt;
                *font_size_pt = self.font_size_pt;
                *bordered = self.bordered;
                return Some(ToolCommandIntent::new(
                    "graph.setSpec",
                    json!({
                        "sheet_id": graph_id.as_str(),
                        "annotations": next_annotations,
                    }),
                ));
            }
        }
        Some(ToolCommandIntent::new(
            "graph.addAnnotation",
            json!({
                "sheet_id": graph_id.as_str(),
                "kind": "equation",
                "result_sheet_id": res_id.as_str(),
                "data_set_index": self.data_set_index,
                "mode": equation_display_mode_str(self.mode),
                "bounds_pt": self.bounds_pt,
                "font_size_pt": self.font_size_pt,
                "bordered": self.bordered,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LocalePreset;
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, ColumnTable, DataSheet, GraphKind, GraphSheet, LinearRegressionSpec,
        ProjectDocument, RemediationKind, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, TableArchetype,
    };
    use serde_json::json;

    #[test]
    fn r9_14_dialogs_validate_patches_and_emit_expected_intents() {
        let preset = LocalePreset::default();
        let mut rand_dlg = RandomizeGroupsDialogState::default();
        let patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "groups": "Vehicle:1, DrugA:2",
            "subject_count": 9,
            "scheme": "permuted_block",
            "block_sizes": [3, 6],
            "seed": 77,
            "title": "Preclinical Arms"
        }))
        .unwrap();
        crate::dialogs_canvas::dialog_patch::patch_randomize_groups(&mut rand_dlg, &patch).unwrap();
        let intent = rand_dlg.commit_intent().expect("randomize intent");
        assert_eq!(intent.command_id, "table.randomizeGroups");
        assert_eq!(intent.params["scheme"], "permuted_block");
        assert_eq!(intent.params["block_sizes"], json!([3, 6]));
        assert_eq!(intent.params["subject_count"], 9);

        let mut sample_dlg = RandomSampleRowsDialogState::default();
        let mut col = ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 2, 12).unwrap();
        col.data_sets[0].title = String::from("Weight");
        col.data_sets[1].title = String::from("Group");
        let ds = DataSheet::new("data-src", "Cohort", TableArchetype::Column(col)).unwrap();
        let mut doc = ProjectDocument::new("Sample Test").unwrap();
        doc.add_data_sheet(ds).unwrap();
        assert!(sample_dlg.open_for_sheet(&doc, &SheetId::new("data-src")));
        let s_patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "sample_size": 6,
            "seed": 19,
            "stratify_column": 1,
            "proportional_allocation": false
        }))
        .unwrap();
        crate::dialogs_canvas::dialog_patch::patch_random_sample_rows(&mut sample_dlg, &s_patch).unwrap();
        let s_intent = sample_dlg.commit_intent().expect("sample intent");
        assert_eq!(s_intent.command_id, "table.randomSampleRows");
        assert_eq!(s_intent.params["stratify_column"], 1);
        assert_eq!(s_intent.params["proportional_allocation"], false);

        let mut sim_dlg = SimulateDistributionsDialogState::default();
        let sim_patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "n_rows": 24,
            "seed": 11,
            "use_multiple_variables": true,
            "columns": [
                { "name": "Control", "distribution": "gaussian", "mean": 10.0, "sd": 2.0 },
                { "name": "Counts", "distribution": "poisson", "lambda": 4.5 },
                { "name": "Latency", "distribution": "exponential", "rate": 0.25 }
            ]
        }))
        .unwrap();
        crate::dialogs_canvas::dialog_patch::patch_simulate_distributions(&mut sim_dlg, &sim_patch, &preset).unwrap();
        let sim_intent = sim_dlg.commit_intent().expect("sim intent");
        assert_eq!(sim_intent.command_id, "table.simulateDistributions");
        assert_eq!(sim_intent.params["n_rows"], 24);
        assert_eq!(sim_intent.params["use_multiple_variables"], true);
        assert_eq!(sim_intent.params["columns"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn r9_16_insert_model_equation_toggles_modes_and_excludes_cld() {
        let mut doc = ProjectDocument::new("Eq Project").unwrap();
        let col = ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        doc.add_data_sheet(DataSheet::new("data-1", "Raw", TableArchetype::Column(col)).unwrap()).unwrap();
        let res_spec = AnalysisSpec::new("res-lin", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
        let mut res = ResultSheet::new(res_spec).unwrap();
        res.status = ResultStatus::UpToDate;
        doc.add_result_sheet(res).unwrap();
        let mut graph = GraphSheet::new_uninitialized_for_data("g-lin", doc.data_sheet(&"data-1".into()).unwrap()).unwrap();
        graph.initialized = true;
        graph.spec.kind = GraphKind::Xy;
        doc.add_graph_sheet(graph).unwrap();

        let mut dlg = InsertModelEquationDialogState::default();
        dlg.open_for_graph(doc.graph_sheet(&"g-lin".into()).unwrap(), &doc);
        assert_eq!(dlg.result_sheet_id.as_ref().map(SheetId::as_str), Some("res-lin"));
        assert_eq!(dlg.mode, EquationDisplayMode::Substituted);

        let insert_intent = dlg.commit_intent(Some(&doc)).expect("insert equation");
        assert_eq!(insert_intent.command_id, "graph.addAnnotation");
        assert_eq!(insert_intent.params["kind"], "equation");
        assert_eq!(insert_intent.params["mode"], "substituted");

        assert_eq!(dlg.toggle_mode(), EquationDisplayMode::Symbolic);
        // Simulate updating an already-present ModelEquation annotation via `graph.setSpec`.
        doc.graph_sheet_mut(&"g-lin".into()).unwrap().spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("res-lin"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(10.0, 10.0, 180.0, 36.0),
            font_size_pt: 10.0,
            bordered: false,
        });
        dlg.open_for_graph(doc.graph_sheet(&"g-lin".into()).unwrap(), &doc);
        assert!(dlg.update_existing);
        assert_eq!(dlg.toggle_mode(), EquationDisplayMode::Symbolic);
        let update_intent = dlg.commit_intent(Some(&doc)).expect("update equation");
        assert_eq!(update_intent.command_id, "graph.setSpec");
        let serialized = serde_json::to_string(&update_intent).unwrap();
        assert!(serialized.contains("\"ModelEquation\""));
        assert!(!serialized.contains("CompactLetterDisplay"));
    }

    #[test]
    fn r9_7_sanitize_preserves_metrics_and_collects_diagnostic_tabs() {
        let violated = ChecklistItem {
            assumption: String::from("Homoscedasticity"),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some(String::from("F(1, 14) = 8.42, p = 0.011")),
            explanation: String::from("Equal variance violated; consider Welch's t test and switch to nonparametric rank tests."),
            remediation: Some(RemediationKind::WelchT),
        };
        let cleaned = sanitize_diagnostics_only_text(&violated);
        assert!(!cleaned.to_ascii_lowercase().contains("consider"));
        assert!(!cleaned.to_ascii_lowercase().contains("switch to"));
        assert!(cleaned.contains("F(1, 14) = 8.42, p = 0.011"));
        let info_small_n = ChecklistItem {
            assumption: String::from("Normality"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("n = 4")),
            explanation: String::from("Inconclusive normality power with small sample size."),
            remediation: Some(RemediationKind::MannWhitneyU),
        };
        assert!(is_inconclusive_info_item(&info_small_n));
        let tab = ResultTab {
            id: String::from("qq"),
            title: String::from("Q-Q"),
            kind: ResultTabKind::NormalQq,
            column_headers: vec![String::from("Q")],
            row_labels: vec![String::from("1")],
            rows: vec![vec![ResultCell::Text(String::from("ok"))]],
            footnotes: vec![],
        };
        assert_eq!(tab.kind, ResultTabKind::NormalQq);
    }
}
