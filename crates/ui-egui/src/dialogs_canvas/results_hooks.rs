//! `Change Data Analyzed`, multi-tab `ResultSheet` view state, narrative checklist, and Constant Hook Picker.

use curvebook_doc::{
    AnalysisKind, ChecklistItem, ChecklistStatus, CiBound, DataSheet, HookSource, PValueFormat, ParameterRule, ParameterScope, ProjectDocument,
    ResultCell, ResultSheet, ResultTab, SheetId, TableArchetype, TableKind,
};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Candidate table listed in `Change Data Analyzed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetargetCandidate {
    pub sheet_id: SheetId,
    pub title: String,
    pub table_kind: TableKind,
    pub compatible: bool,
    pub incompatible_reason: Option<String>,
}

/// Cross-table source retargeting state (`analysis.retargetData`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeDataAnalyzedState {
    pub result_sheet_id: Option<SheetId>,
    pub current_source_id: Option<SheetId>,
    pub selected_target_id: Option<SheetId>,
    pub selected_data_sets: Vec<usize>,
    pub candidates: Vec<RetargetCandidate>,
}

impl ChangeDataAnalyzedState {
    /// Populate compatible and incompatible `DataSheet` candidates for `result`.
    pub fn open_for_result(&mut self, result: &ResultSheet, doc: &ProjectDocument) {
        self.result_sheet_id = Some(result.id.clone());
        self.current_source_id = Some(result.analysis.source_sheet_id.clone());
        self.selected_data_sets = result.analysis.selected_data_sets.clone();
        self.candidates = doc
            .data_sheets
            .iter()
            .map(|ds| {
                let compatible = result.analysis.kind.is_compatible_with(ds.kind());
                let incompatible_reason =
                    (!compatible).then(|| format!("{} cannot run on {} tables", result.analysis.effective_title(), ds.kind().label()));
                RetargetCandidate { sheet_id: ds.id.clone(), title: ds.title.clone(), table_kind: ds.kind(), compatible, incompatible_reason }
            })
            .collect();
        self.selected_target_id = self
            .candidates
            .iter()
            .find(|c| c.compatible && Some(&c.sheet_id) != self.current_source_id.as_ref())
            .or_else(|| self.candidates.iter().find(|c| c.compatible))
            .map(|c| c.sheet_id.clone());
    }

    /// Select a candidate target sheet id. Returns `Err` if incompatible.
    pub fn select_target(&mut self, target_sheet_id: &SheetId) -> Result<(), String> {
        let Some(candidate) = self.candidates.iter().find(|c| &c.sheet_id == target_sheet_id) else {
            return Err(format!("unknown target sheet {}", target_sheet_id.as_str()));
        };
        if !candidate.compatible {
            return Err(candidate.incompatible_reason.clone().unwrap_or_else(|| String::from("incompatible table archetype")));
        }
        self.selected_target_id = Some(candidate.sheet_id.clone());
        Ok(())
    }

    /// Emit `analysis.retargetData` only when the selected target sheet is compatible.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let res_id = self.result_sheet_id.as_ref()?;
        let target = self.selected_target_id.as_ref()?;
        let compatible = self.candidates.iter().any(|c| &c.sheet_id == target && c.compatible);
        if !compatible {
            return None;
        }
        Some(ToolCommandIntent::new(
            "analysis.retargetData",
            json!({
                "sheet_id": res_id.as_str(),
                "source_sheet_id": target.as_str(),
                "selected_data_sets": self.selected_data_sets,
            }),
        ))
    }
}

// ============================================================================
// Multi-tab ResultSheet, Narrative Checklist, and Constant Hook Picker
// ============================================================================

/// Filter applied to plain-language assumption [`ChecklistItem`] diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChecklistSeverityFilter {
    #[default]
    All,
    WarningsAndViolations,
}

/// Interactive view state for a multi-tab [`ResultSheet`] and its narrative [`ChecklistItem`] pane.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultSheetViewState {
    pub result_sheet_id: Option<SheetId>,
    pub active_tab_index: usize,
    pub checklist_expanded: bool,
    pub checklist_filter: ChecklistSeverityFilter,
    #[serde(default = "default_report_format")]
    pub report_format: String,
    #[serde(default)]
    pub last_methods_report: Option<String>,
    #[serde(default)]
    pub bound_sheet_id: Option<String>,
    #[serde(default)]
    pub bound_tab_idx: Option<usize>,
    #[serde(default)]
    pub selected_anchor: Option<(usize, usize)>,
    #[serde(default)]
    pub selected_focus: Option<(usize, usize)>,
    #[serde(default)]
    pub last_copied_tsv: Option<String>,
}

fn default_report_format() -> String {
    String::from("markdown")
}

impl ResultSheetViewState {
    /// Synchronize active tab bounds against `result`.
    pub fn sync_with_result(&mut self, result: &ResultSheet) {
        if self.result_sheet_id.as_ref() != Some(&result.id) {
            self.result_sheet_id = Some(result.id.clone());
            self.active_tab_index = 0;
            self.checklist_expanded = !result.checklist.is_empty();
        }
        if result.tabs.is_empty() {
            self.active_tab_index = 0;
        } else if self.active_tab_index >= result.tabs.len() {
            self.active_tab_index = result.tabs.len() - 1;
        }
    }

    /// Select tab by zero-based index, clamped within `tab_count`.
    pub fn select_tab(&mut self, index: usize, tab_count: usize) {
        self.active_tab_index = if tab_count == 0 { 0 } else { index.min(tab_count - 1) };
    }

    /// Select a [`ResultTab`] by its stable `id`.
    pub fn select_tab_by_id(&mut self, result: &ResultSheet, tab_id: &str) -> bool {
        if let Some(pos) = result.tabs.iter().position(|t| t.id == tab_id) {
            self.active_tab_index = pos;
            true
        } else {
            false
        }
    }

    /// Active rectangular selection `(min_row, max_row, min_col, max_col)` if bound to `(sheet_id, tab_idx)`.
    #[must_use]
    pub fn selection_bounds(&self, sheet_id: &str, tab_idx: usize) -> Option<(usize, usize, usize, usize)> {
        if self.bound_sheet_id.as_deref() != Some(sheet_id) || self.bound_tab_idx != Some(tab_idx) {
            return None;
        }
        let a = self.selected_anchor?;
        let b = self.selected_focus.unwrap_or(a);
        Some((a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1)))
    }

    /// Currently active [`ResultTab`], if any.
    #[must_use]
    pub fn active_tab<'a>(&self, result: &'a ResultSheet) -> Option<&'a ResultTab> {
        result.tabs.get(self.active_tab_index).or_else(|| result.tabs.first())
    }

    /// Assumption diagnostics filtered by [`Self::checklist_filter`].
    #[must_use]
    pub fn visible_checklist<'a>(&self, result: &'a ResultSheet) -> Vec<&'a ChecklistItem> {
        result
            .checklist
            .iter()
            .filter(|item| match self.checklist_filter {
                ChecklistSeverityFilter::All => true,
                ChecklistSeverityFilter::WarningsAndViolations => {
                    matches!(item.status, ChecklistStatus::Warn | ChecklistStatus::Violated)
                }
            })
            .collect()
    }

    /// Return `(pass, warn, violated, info)` counts across `result.checklist`.
    #[must_use]
    pub fn checklist_summary_counts(result: &ResultSheet) -> (usize, usize, usize, usize) {
        let mut pass = 0usize;
        let mut warn = 0usize;
        let mut violated = 0usize;
        let mut info = 0usize;
        for item in &result.checklist {
            match item.status {
                ChecklistStatus::Pass => pass += 1,
                ChecklistStatus::Warn => warn += 1,
                ChecklistStatus::Violated => violated += 1,
                ChecklistStatus::Info => info += 1,
            }
        }
        (pass, warn, violated, info)
    }

    /// Build a `result.generateReport` intent for `result_sheet_id` (`QOL-6`).
    #[must_use]
    pub fn generate_report_intent(&self, result_sheet_id: &SheetId) -> ToolCommandIntent {
        let fmt = if self.report_format.trim().is_empty() { "markdown" } else { self.report_format.trim() };
        ToolCommandIntent::new("result.generateReport", json!({ "result_sheet_id": result_sheet_id.as_str(), "format": fmt }))
    }

    /// Format a typed [`ResultCell`] cleanly without panicking on `CiBound::Open` or non-finite values.
    #[must_use]
    pub fn format_result_cell(cell: &ResultCell, p_format: PValueFormat) -> String {
        match cell {
            ResultCell::Empty => String::new(),
            ResultCell::Number { value, prefix, units } => {
                let num = if value.is_finite() { format_compact_f64(*value) } else { String::from("—") };
                let pfx = prefix.as_deref().unwrap_or("");
                match units.as_deref() {
                    Some(u) if !u.is_empty() => format!("{pfx}{num} {u}"),
                    _ => format!("{pfx}{num}"),
                }
            }
            ResultCell::ConfidenceInterval { lower, upper } => {
                let lo = format_ci_bound(*lower);
                let hi = format_ci_bound(*upper);
                format!("[{lo}, {hi}]")
            }
            ResultCell::PValue { raw, formatted, stars } => {
                let p_text = if raw.is_finite() { p_format.format_p(*raw) } else { formatted.clone() };
                if stars.is_empty() { p_text } else { format!("{p_text} {stars}") }
            }
            ResultCell::Text(s) => s.clone(),
            ResultCell::Boolean(b) => String::from(if *b { "Yes" } else { "No" }),
        }
    }
}

fn format_ci_bound(bound: CiBound) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => format_compact_f64(v),
        CiBound::Finite(_) | CiBound::Open => String::from("Unbounded"),
    }
}

pub(crate) fn format_compact_f64(v: f64) -> String {
    if !v.is_finite() {
        return String::from("NaN");
    }
    let abs = v.abs();
    if abs != 0.0 && !(1e-3..1e5).contains(&abs) {
        format!("{v:.3e}")
    } else if (v - v.round()).abs() < 1e-9 && abs < 1e12 {
        format!("{:.0}", v.round())
    } else {
        format!("{v:.4}")
    }
}

/// Hookable upstream value surfaced by [`ConstantHookPickerState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableHookOption {
    pub label: String,
    pub preview_value: String,
    pub template_token: String,
    pub source: HookSource,
}

/// Target sheet category for [`ConstantHookPickerState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HookTargetKind {
    #[default]
    AnalysisParameter,
    GraphTextBox,
    LayoutTextCallout,
}

/// Dialog picker inserting live `InfoConstant`, `ResultScalar`, and `ColumnTitleValue` hooks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConstantHookPickerState {
    pub target_sheet_id: Option<SheetId>,
    pub target_parameter_name: Option<String>,
    #[serde(default)]
    pub target_kind: HookTargetKind,
    #[serde(default)]
    pub existing_parameter_rules: Vec<ParameterRule>,
    pub search_query: String,
    pub options: Vec<AvailableHookOption>,
    pub selected_index: Option<usize>,
}

impl ConstantHookPickerState {
    /// Scan all `InfoSheet`, `ResultSheet`, and numeric data-set column titles in `doc`.
    pub fn open_for_document(&mut self, target_sheet_id: SheetId, parameter_name: Option<String>, doc: &ProjectDocument) {
        self.existing_parameter_rules.clear();
        if let Some(res) = doc.result_sheet(&target_sheet_id) {
            self.target_kind = HookTargetKind::AnalysisParameter;
            if let AnalysisKind::NonlinearRegression(nl) = &res.analysis.kind {
                self.existing_parameter_rules = nl.parameter_rules.clone();
            }
        } else if doc.graph_sheet(&target_sheet_id).is_some() {
            self.target_kind = HookTargetKind::GraphTextBox;
        } else if doc.layout_sheet(&target_sheet_id).is_some() {
            self.target_kind = HookTargetKind::LayoutTextCallout;
        } else {
            self.target_kind = HookTargetKind::AnalysisParameter;
        }
        self.target_sheet_id = Some(target_sheet_id.clone());
        self.target_parameter_name = parameter_name;
        self.search_query.clear();
        self.options.clear();
        let numeric_only = self.target_kind == HookTargetKind::AnalysisParameter;
        collect_info_hooks(doc, numeric_only, &mut self.options);
        collect_result_hooks(doc, Some(&target_sheet_id), &mut self.options);
        collect_column_title_hooks(doc, &mut self.options);
        self.selected_index = (!self.options.is_empty()).then_some(0);
    }

    /// Options matching `search_query`.
    #[must_use]
    pub fn filtered_options(&self) -> Vec<&AvailableHookOption> {
        let q = self.search_query.trim().to_ascii_lowercase();
        self.options
            .iter()
            .filter(|opt| q.is_empty() || opt.label.to_ascii_lowercase().contains(&q) || opt.template_token.to_ascii_lowercase().contains(&q))
            .collect()
    }

    /// Return the currently selected [`AvailableHookOption`].
    #[must_use]
    pub fn selected_option(&self) -> Option<&AvailableHookOption> {
        self.selected_index.and_then(|idx| self.options.get(idx))
    }

    /// Emit `analysis.updateParams`, `graph.addAnnotation`, or `layout.addPanel` binding the chosen
    /// [`HookSource`] or template token onto `target_sheet_id`.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let target = self.target_sheet_id.as_ref()?;
        let opt = self.selected_option()?;
        match self.target_kind {
            HookTargetKind::AnalysisParameter => {
                let param_name = self.target_parameter_name.clone().unwrap_or_else(|| String::from("Bottom"));
                let mut rules: Vec<ParameterRule> = self.existing_parameter_rules.iter().filter(|r| r.name != param_name).cloned().collect();
                rules.push(ParameterRule {
                    name: param_name,
                    scope: ParameterScope::HookedConstant(opt.source.clone()),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                });
                Some(ToolCommandIntent::new(
                    "analysis.updateParams",
                    json!({
                        "sheet_id": target.as_str(),
                        "parameter_rules": rules,
                    }),
                ))
            }
            HookTargetKind::GraphTextBox => Some(ToolCommandIntent::new(
                "graph.addAnnotation",
                json!({
                    "sheet_id": target.as_str(),
                    "kind": "text",
                    "text": opt.template_token,
                }),
            )),
            HookTargetKind::LayoutTextCallout => Some(ToolCommandIntent::new(
                "layout.addPanel",
                json!({
                    "sheet_id": target.as_str(),
                    "kind": "text",
                    "markdown": opt.template_token,
                }),
            )),
        }
    }
}

fn collect_info_hooks(doc: &ProjectDocument, numeric_only: bool, out: &mut Vec<AvailableHookOption>) {
    let mut deferred_empty = Vec::new();
    for info in &doc.info_sheets {
        for c in &info.constants {
            if numeric_only && c.value.as_finite_f64().is_none() {
                continue;
            }
            let preview = c.value.display_text();
            let is_empty = preview.is_empty();
            let shown_preview = if is_empty { String::from("<empty>") } else { preview };
            let opt = AvailableHookOption {
                label: format!("Info [{}]: {}", info.title, c.name),
                preview_value: shown_preview,
                template_token: format!("{{{{info:{}:{}}}}}", info.id.as_str(), c.name),
                source: HookSource::InfoConstant { info_sheet_id: Some(info.id.clone()), constant_name: c.name.clone() },
            };
            if is_empty {
                deferred_empty.push(opt);
            } else {
                out.push(opt);
            }
        }
    }
    out.extend(deferred_empty);
}

fn collect_result_hooks(doc: &ProjectDocument, exclude_sheet_id: Option<&SheetId>, out: &mut Vec<AvailableHookOption>) {
    for res in &doc.result_sheets {
        if exclude_sheet_id.is_some_and(|ex| ex == &res.id) {
            continue;
        }
        for scalar in &res.scalars {
            if !scalar.value.is_finite() {
                continue;
            }
            let ds_suffix = scalar.data_set_index.map_or(String::new(), |ds| format!(" [ds {ds}]"));
            let token = match scalar.data_set_index {
                Some(ds) => format!("{{{{result:{}:{ds}:{}}}}}", res.id.as_str(), scalar.key),
                None => format!("{{{{result:{}:{}}}}}", res.id.as_str(), scalar.key),
            };
            out.push(AvailableHookOption {
                label: format!("Result [{}]: {}{ds_suffix}", res.title(), scalar.key),
                preview_value: format_compact_f64(scalar.value),
                template_token: token,
                source: HookSource::ResultScalar {
                    result_sheet_id: res.id.clone(),
                    parameter_key: scalar.key.clone(),
                    data_set_index: scalar.data_set_index,
                },
            });
        }
    }
}

fn collect_column_title_hooks(doc: &ProjectDocument, out: &mut Vec<AvailableHookOption>) {
    for ds in &doc.data_sheets {
        for col_idx in 0..ds.archetype.column_count() {
            let Some(title) = data_column_title(ds, col_idx) else {
                continue;
            };
            if title.trim().parse::<f64>().ok().is_some_and(f64::is_finite) {
                out.push(AvailableHookOption {
                    label: format!("Column Title [{}]: Col {} = {title}", ds.title, curvebook_doc::default_column_label(col_idx)),
                    preview_value: title.to_owned(),
                    template_token: format!("{{{{col:{}:{col_idx}}}}}", ds.id.as_str()),
                    source: HookSource::ColumnTitleValue { data_sheet_id: ds.id.clone(), data_set_index: col_idx },
                });
            }
        }
    }
}

pub(crate) fn data_column_title(sheet: &DataSheet, col_idx: usize) -> Option<&str> {
    match &sheet.archetype {
        TableArchetype::Xy(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col_idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col_idx).map(|g| g.title.as_str()),
    }
}

/// Serialize a [`ResultSheet`] selection (or full active tab when `force_full` is true) into TSV
/// and publish it to `app`'s internal and OS clipboard state.
pub fn copy_result_table_to_clipboard(app: &mut crate::UiApp, sheet_id: &str, force_full: bool) -> crate::interaction::UiEventOutcome {
    let (source_sheet_id, tab_id, tsv, sel) = {
        let Some(rs) = app.document().result_sheets.iter().find(|r| r.id.as_str() == sheet_id) else {
            return crate::interaction::UiEventOutcome::default();
        };
        let rv = &app.canvas_dialogs.result_view;
        let requested_idx = if !force_full && rv.bound_sheet_id.as_deref() == Some(sheet_id) {
            rv.bound_tab_idx.unwrap_or(rv.active_tab_index)
        } else {
            rv.active_tab_index
        };
        let fallback_tab =
            rs.tabs.is_empty().then(|| rs.analyzable_data.as_ref().map(|ds| datasheet_to_preview_tab(ds, &app.locale_preset))).flatten();
        let (resolved_idx, tab_ref) = if let Some(tab) = rs.tabs.get(requested_idx) {
            (requested_idx, Some(tab))
        } else if let Some(tab) = rs.tabs.first() {
            (0, Some(tab))
        } else {
            (0, fallback_tab.as_ref())
        };
        let Some(tab) = tab_ref else {
            return crate::interaction::UiEventOutcome::default();
        };
        let sel = if force_full { None } else { rv.selection_bounds(sheet_id, resolved_idx) };
        let tsv = format_tab_selection_tsv(tab, rs.analysis.p_value_format, sel);
        (rs.id.clone(), tab.id.clone(), tsv, sel)
    };
    app.canvas_dialogs.result_view.last_copied_tsv = Some(tsv.clone());
    app.interaction.last_clipboard_bundle = None;
    app.interaction.last_clipboard_image_png = None;
    app.interaction.last_clipboard_text = Some(tsv.clone());
    app.interaction.last_copied_result_table = Some(crate::interaction::ResultTableClipboard {
        source_sheet_id,
        tab_id: Some(tab_id),
        selected_rows: sel.map(|(r0, r1, _, _)| (r0..=r1).collect()),
        selected_cols: sel.map(|(_, _, c0, c1)| (c0..=c1).collect()),
    });
    let _ = app.write_clipboard_text(&tsv);

    crate::interaction::UiEventOutcome {
        handled: true,
        action: if force_full { String::from("results.copy_table") } else { String::from("results.copy_selection") },
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "copied_text": tsv, "chars": tsv.len() })),
    }
}

fn format_tab_selection_tsv(tab: &ResultTab, p_fmt: curvebook_doc::PValueFormat, sel: Option<(usize, usize, usize, usize)>) -> String {
    let num_cols = tab_column_count(tab);
    if tab.rows.is_empty() || num_cols == 0 {
        return String::new();
    }
    let max_r = tab.rows.len().saturating_sub(1);
    let max_c = num_cols.saturating_sub(1);
    let (r0, r1, c0, c1) = sel.map_or((0, max_r, 0, max_c), |(a, b, c, d)| (a.min(max_r), b.min(max_r), c.min(max_c), d.min(max_c)));

    if r0 == r1 && c0 == c1 {
        let cell = tab.rows.get(r0).and_then(|row| row.get(c0)).unwrap_or(&ResultCell::Empty);
        return ResultSheetViewState::format_result_cell(cell, p_fmt);
    }

    let include_headers = r0 == 0 && r1 == max_r && c1 > c0;
    let include_row_labels = c0 == 0 && c1 == max_c;
    let mut lines = Vec::new();
    if include_headers {
        let mut hdr_parts = Vec::new();
        if include_row_labels {
            hdr_parts.push(String::from("Row"));
        }
        for c in c0..=c1 {
            hdr_parts.push(tab_column_header(tab, c));
        }
        lines.push(hdr_parts.join("\t"));
    } else if r1 > r0 && c0 == c1 {
        lines.push(tab_column_header(tab, c0));
    }

    for r in r0..=r1 {
        let mut parts = Vec::new();
        if include_row_labels {
            parts.push(tab.row_labels.get(r).cloned().unwrap_or_else(|| format!("Row {}", r + 1)));
        }
        for c in c0..=c1 {
            let cell = tab.rows.get(r).and_then(|row| row.get(c)).unwrap_or(&ResultCell::Empty);
            parts.push(ResultSheetViewState::format_result_cell(cell, p_fmt));
        }
        lines.push(parts.join("\t"));
    }
    lines.join("\n")
}

fn datasheet_to_preview_tab(ds: &DataSheet, preset: &crate::i18n::LocalePreset) -> ResultTab {
    let (row_total, col_total) = crate::inspect::table_dimensions_summary(&ds.archetype);
    let col_count = col_total.max(1);
    let headers: Vec<String> = (0..col_count).map(|c| data_column_title(ds, c).map_or_else(|| format!("Col {}", c + 1), str::to_owned)).collect();
    let row_count = row_total.min(200);
    let mut row_labels = Vec::with_capacity(row_count);
    let mut rows = Vec::with_capacity(row_count);
    for r in 0..row_count {
        row_labels.push(format!("Row {}", r + 1));
        let mut row = Vec::with_capacity(col_count);
        for c in 0..col_count {
            let txt =
                ds.cell(curvebook_doc::CellCoord::new(r, c, 0)).map(|cell| crate::grid_view::format_cell_display(cell, preset)).unwrap_or_default();
            row.push(if txt.is_empty() { ResultCell::Empty } else { ResultCell::Text(txt) });
        }
        rows.push(row);
    }
    ResultTab {
        id: String::from("analyzable-data"),
        title: ds.title.clone(),
        kind: curvebook_doc::ResultTabKind::MainSummary,
        column_headers: headers,
        row_labels,
        rows,
        footnotes: Vec::new(),
    }
}

fn tab_column_count(tab: &ResultTab) -> usize {
    tab.column_headers.len().max(tab.rows.iter().map(Vec::len).max().unwrap_or(0))
}

fn tab_column_header(tab: &ResultTab, c_idx: usize) -> String {
    tab.column_headers.get(c_idx).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Col {}", c_idx + 1))
}
