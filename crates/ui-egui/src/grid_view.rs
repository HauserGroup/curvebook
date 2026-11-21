//! Virtualized two-tier `(Group x Subcolumn)` spreadsheet grid view state, cell formatting,
//! `MultipleVariables` role header strip & `Variables...` recoding/ordering model, `=` Calculated
//! Variable formula bar with autocomplete, 4-stage `Import & Paste Special` / `Paste Link`
//! pipeline model, and selection quick-stats footer (`docs/evidence/user-interface.md`,
//! `docs/implementation/data-tables-and-transforms.md`).

use std::borrow::Borrow;
use std::collections::{BTreeSet, HashSet};

use crate::i18n::LocalePreset;
use crate::os_integration::ClipboardBundle;
use curvebook_doc::{
    CategoricalLevelSummary, Cell, CellCoord, CellValue, DataSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind,
    LevelOrderSpec, LinkBorderColor, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId, SubcolumnMode, TableArchetype,
    TableKind, VariableRole, VariableSource, XColumnMode, default_column_label,
};
use curvebook_format::{CsvOptions, format_cell_token, write_delimited_records};
use curvebook_palette::Rgba8;
use curvebook_tools::{GridCellCoord, GridToolState, GridVisibleWindow, SelectionQuickStats, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_QUICK_STATS_CELLS_FOR_CLONE_PASS: usize = 50_000;
const MAX_MATERIALIZED_SELECTION_COORDS: usize = 4_096;

/// Cardinal arrow navigation direction inside the spreadsheet grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MoveDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Visual styling metadata computed for one visible cell (`blue italic strikethrough` exclusions,
/// provenance borders, and selection highlight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellVisualStyle {
    pub text_color: Rgba8,
    pub italic: bool,
    pub strikethrough: bool,
    pub excluded_badge: Option<String>,
    pub provenance_border: Option<LinkBorderColor>,
    pub provenance_border_rgba: Option<Rgba8>,
    pub link_id: Option<String>,
    pub validation_error: Option<String>,
    pub is_selected: bool,
    pub is_active: bool,
    pub is_calculated: bool,
}

impl Default for CellVisualStyle {
    fn default() -> Self {
        Self {
            text_color: Rgba8::rgb(24, 28, 36),
            italic: false,
            strikethrough: false,
            excluded_badge: None,
            provenance_border: None,
            provenance_border_rgba: None,
            link_id: None,
            validation_error: None,
            is_selected: false,
            is_active: false,
            is_calculated: false,
        }
    }
}

/// Formatted cell payload materialized strictly for cells inside the active viewport window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattedGridCell {
    pub coord: GridCellCoord,
    pub flat_col: usize,
    pub display_text: String,
    pub excluded: bool,
    pub visual: CellVisualStyle,
}

/// Role/type pill rendered in the `MultipleVariables` column header strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableRoleHeader {
    pub col_index: usize,
    pub title: String,
    pub role: VariableRole,
    pub role_inferred: bool,
    pub is_calculated: bool,
    pub formula: Option<String>,
    pub badge_text: String,
    pub badge_color: Rgba8,
}

/// Tier-1 group header span covering one or more replicate subcolumns (`Group A`, `Group B`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier1GroupHeader {
    pub col_index: usize,
    pub letter: String,
    pub title: String,
    pub start_flat_col: usize,
    pub span: usize,
    pub variable_role: Option<VariableRoleHeader>,
}

/// Tier-2 subcolumn header (`Y`, `Y1..Yn`, `Mean`, `SD`, `N`, `A:1`, or variable role tag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier2SubcolumnHeader {
    pub flat_col: usize,
    pub col: usize,
    pub subcol: usize,
    pub label: String,
    pub is_calculated: bool,
}

/// Pinned leading column cell for Row Titles or independent `X` / `Elapsed Time` / `Slice`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadingRowData {
    pub row: usize,
    pub row_title: String,
    pub x_or_time_display: Option<String>,
    pub x_excluded: bool,
}

/// One materialized visible row slice inside [`GridVisibleFrame`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleGridRow {
    pub row: usize,
    pub leading: LeadingRowData,
    pub cells: Vec<FormattedGridCell>,
}

/// Complete O(visible_rows * visible_subcols) virtualized frame slice for rendering the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridVisibleFrame {
    pub sheet_id: SheetId,
    pub kind: TableKind,
    pub frozen: bool,
    pub row_titles_expanded: bool,
    pub x_column_title: Option<String>,
    pub total_rows: usize,
    pub total_flat_cols: usize,
    pub window: GridVisibleWindow,
    pub tier1_groups: Vec<Tier1GroupHeader>,
    pub tier2_subcolumns: Vec<Tier2SubcolumnHeader>,
    pub rows: Vec<VisibleGridRow>,
}

/// Selection summary statistics footer (`n`, `excluded`, `mean`, `SD`, `SEM`, `median`, `sum`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct GridQuickStats {
    pub selected_cells: usize,
    pub n: usize,
    pub excluded: usize,
    pub sum: Option<f64>,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
    pub sem: Option<f64>,
    pub median: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl GridQuickStats {
    /// Compute quick summary statistics across `[a, b]` directly from `sheet` without cloning cells.
    #[must_use]
    pub fn from_data_sheet(sheet: &DataSheet, a: GridCellCoord, b: GridCellCoord) -> Self {
        let row_count = sheet.archetype.row_count();
        let col_count = sheet.archetype.column_count();
        if row_count == 0 || col_count == 0 {
            return Self::default();
        }
        let r0 = a.row.min(b.row);
        let r1 = a.row.max(b.row);
        if r0 >= row_count {
            return Self::default();
        }
        let r1 = r1.min(row_count - 1);

        let f0 = flat_index_for_coord(&sheet.archetype, a.col, a.subcol);
        let f1 = flat_index_for_coord(&sheet.archetype, b.col, b.subcol);
        let start_flat = f0.min(f1);
        let end_flat = f0.max(f1);
        let total_flat = total_flat_data_subcolumns(&sheet.archetype);
        if total_flat == 0 || start_flat >= total_flat {
            return Self::default();
        }
        let end_flat = end_flat.min(total_flat - 1);

        let mut coords = Vec::with_capacity(end_flat - start_flat + 1);
        for flat in start_flat..=end_flat {
            if let Some((c, s)) = resolve_flat_coord(&sheet.archetype, flat) {
                coords.push((c, s));
            }
        }

        let mut selected_cells = 0usize;
        let mut excluded = 0usize;
        let mut values = Vec::new();
        for r in r0..=r1 {
            for &(c, s) in &coords {
                selected_cells += 1;
                if let Some(cell) = sheet.cell(CellCoord::new(r, c, s)) {
                    if cell.excluded {
                        excluded += 1;
                        continue;
                    }
                    if let CellValue::Number(v) = cell.value
                        && v.is_finite()
                    {
                        values.push(v);
                    }
                }
            }
        }

        Self::from_values(selected_cells, excluded, &mut values)
    }

    /// Compute descriptive quick stats across an arbitrary (possibly discontiguous) slice of `coords`.
    #[must_use]
    pub fn from_coords(sheet: &DataSheet, coords: &[GridCellCoord]) -> Self {
        let mut selected_cells = 0usize;
        let mut excluded = 0usize;
        let mut values = Vec::new();
        for &coord in coords {
            selected_cells += 1;
            if let Some(cell) = sheet.cell(CellCoord::from(coord)) {
                if cell.excluded {
                    excluded += 1;
                    continue;
                }
                if let CellValue::Number(v) = cell.value
                    && v.is_finite()
                {
                    values.push(v);
                }
            }
        }
        Self::from_values(selected_cells, excluded, &mut values)
    }

    #[must_use]
    fn from_values(selected_cells: usize, excluded: usize, values: &mut [f64]) -> Self {
        if values.is_empty() {
            return Self { selected_cells, n: 0, excluded, ..Self::default() };
        }
        let n = values.len();
        let mut sum = 0.0;
        let mut comp = 0.0;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &v in values.iter() {
            let t = sum + v;
            comp += if sum.abs() >= v.abs() { (sum - t) + v } else { (v - t) + sum };
            sum = t;
            min = min.min(v);
            max = max.max(v);
        }
        sum += comp;
        let mean = sum / (n as f64);

        let (sd, sem) = if n > 1 {
            let mut sq_sum = 0.0;
            for &v in values.iter() {
                let d = v - mean;
                sq_sum += d * d;
            }
            let sample_sd = (sq_sum / ((n - 1) as f64)).sqrt();
            (Some(sample_sd), Some(sample_sd / (n as f64).sqrt()))
        } else {
            (None, None)
        };

        values.sort_by(f64::total_cmp);
        let median = if n % 2 == 1 { Some(values[n / 2]) } else { Some(0.5 * (values[n / 2 - 1] + values[n / 2])) };

        Self { selected_cells, n, excluded, sum: Some(sum), mean: Some(mean), sd, sem, median, min: Some(min), max: Some(max) }
    }

    /// Format a concise status-bar summary (`n`, `excluded`, `mean`, `SD`, `SEM`, `median`, `sum`).
    #[must_use]
    pub fn format_footer(&self) -> String {
        let mut parts = vec![format!("n={}", self.n), format!("excluded={}", self.excluded)];
        if let Some(mean) = self.mean {
            parts.push(format!("mean={}", format_compact_f64(mean, '.')));
        }
        if let Some(sd) = self.sd {
            parts.push(format!("SD={}", format_compact_f64(sd, '.')));
        }
        if let Some(sem) = self.sem {
            parts.push(format!("SEM={}", format_compact_f64(sem, '.')));
        }
        if let Some(median) = self.median {
            parts.push(format!("median={}", format_compact_f64(median, '.')));
        }
        if let Some(sum) = self.sum {
            parts.push(format!("sum={}", format_compact_f64(sum, '.')));
        }
        parts.join(" | ")
    }
}

/// State model for the `Variables...` column-role, level-recoding, and level-ordering editor
/// (`MultipleVariables` tables).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VariablesDialogState {
    pub open: bool,
    pub selected_column: Option<usize>,
    pub pending_role: Option<VariableRole>,
    pub recode_mappings: Vec<(String, String)>,
    pub level_order: LevelOrderSpec,
    pub ordered_levels: Vec<CategoricalLevelSummary>,
}

impl VariablesDialogState {
    /// Open and populate the inspector for `col_idx` on `sheet`.
    pub fn open_for_column(&mut self, sheet: &DataSheet, col_idx: usize) {
        self.open = true;
        self.selected_column = Some(col_idx);
        self.recode_mappings.clear();
        self.sync_from_sheet(sheet);
    }

    /// Refresh level counts and role metadata from `sheet`.
    pub fn sync_from_sheet(&mut self, sheet: &DataSheet) {
        let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
            self.ordered_levels.clear();
            return;
        };
        let Some(col_idx) = self.selected_column else {
            self.ordered_levels.clear();
            return;
        };
        let Some(col) = mv.variables.get(col_idx) else {
            self.ordered_levels.clear();
            return;
        };
        self.pending_role = Some(col.role);
        self.level_order = col.level_order.clone();
        self.ordered_levels = col.ordered_levels_with(&self.level_order);
    }

    /// Stage a bulk level recoding replacement (`from -> to`), deduplicating existing entries.
    pub fn add_recode_mapping(&mut self, from: impl Into<String>, to: impl Into<String>) {
        let from = from.into();
        let to = to.into();
        if let Some(existing) = self.recode_mappings.iter_mut().find(|(src, _)| src == &from) {
            existing.1 = to;
        } else {
            self.recode_mappings.push((from, to));
        }
    }

    /// Merge multiple typo/synonym levels into one canonical `target` label.
    pub fn merge_levels_into(&mut self, sources: &[&str], target: &str) {
        for &src in sources {
            if src != target {
                self.add_recode_mapping(src, target);
            }
        }
    }

    /// Update the pending [`LevelOrderSpec`] and refresh live preview ordering when `sheet` is given.
    pub fn set_level_order(&mut self, spec: LevelOrderSpec, sheet: Option<&DataSheet>) {
        self.level_order = spec;
        if let Some(s) = sheet
            && let TableArchetype::MultipleVariables(mv) = &s.archetype
            && let Some(col) = self.selected_column.and_then(|idx| mv.variables.get(idx))
        {
            self.ordered_levels = col.ordered_levels_with(&self.level_order);
        }
    }

    /// Preview recoding impact without mutating `sheet`, returning `(changed_cells, resulting_levels)`.
    #[must_use]
    pub fn preview_recode(&self, sheet: &DataSheet) -> Option<(usize, Vec<CategoricalLevelSummary>)> {
        let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
            return None;
        };
        let col_idx = self.selected_column?;
        let mut preview_col = mv.variables.get(col_idx)?.clone();
        let edited = preview_col.recode_levels(&self.recode_mappings);
        let levels = preview_col.ordered_levels_with(&self.level_order);
        Some((edited, levels))
    }

    /// Emit a `"table.setVariableRole"` command intent for the selected column.
    #[must_use]
    pub fn build_role_command(&self, sheet_id: &SheetId) -> Option<ToolCommandIntent> {
        let col = self.selected_column?;
        let role = self.pending_role?;
        Some(ToolCommandIntent {
            command_id: String::from("table.setVariableRole"),
            params: json!({ "sheet_id": sheet_id.as_str(), "col": col, "role": role }),
        })
    }

    /// Emit a `"table.recodeLevels"` command intent for the staged recode mappings.
    #[must_use]
    pub fn build_recode_command(&self, sheet_id: &SheetId) -> Option<ToolCommandIntent> {
        let col = self.selected_column?;
        if self.recode_mappings.is_empty() {
            return None;
        }
        let mappings: Vec<serde_json::Value> = self.recode_mappings.iter().map(|(from, to)| json!({ "from": from, "to": to })).collect();
        Some(ToolCommandIntent {
            command_id: String::from("table.recodeLevels"),
            params: json!({ "sheet_id": sheet_id.as_str(), "col": col, "mappings": mappings }),
        })
    }

    /// Emit a `"table.setCategoricalOrder"` command intent for `level_order`.
    #[must_use]
    pub fn build_order_command(&self, sheet_id: &SheetId) -> Option<ToolCommandIntent> {
        let col = self.selected_column?;
        Some(ToolCommandIntent {
            command_id: String::from("table.setCategoricalOrder"),
            params: json!({ "sheet_id": sheet_id.as_str(), "col": col, "order_spec": self.level_order }),
        })
    }
}

/// Category of an item suggested by the `=` Calculated Variable formula bar autocomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormulaAutocompleteKind {
    Column,
    ScalarFunction,
    AggregateFunction,
    WindowFunction,
    Constant,
    InfoConstant,
}

/// One autocomplete suggestion produced by [`FormulaBarState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaAutocompleteItem {
    pub label: String,
    pub insert_text: String,
    pub kind: FormulaAutocompleteKind,
    pub detail: String,
}

const FORMULA_BUILTINS: &[(&str, &str, FormulaAutocompleteKind, &str)] = &[
    ("abs(x)", "abs(", FormulaAutocompleteKind::ScalarFunction, "Absolute value"),
    ("sqrt(x)", "sqrt(", FormulaAutocompleteKind::ScalarFunction, "Square root"),
    ("sqr(x)", "sqr(", FormulaAutocompleteKind::ScalarFunction, "Square (x^2)"),
    ("ln(x)", "ln(", FormulaAutocompleteKind::ScalarFunction, "Natural logarithm"),
    ("log10(x)", "log10(", FormulaAutocompleteKind::ScalarFunction, "Base-10 logarithm"),
    ("log2(x)", "log2(", FormulaAutocompleteKind::ScalarFunction, "Base-2 logarithm"),
    ("exp(x)", "exp(", FormulaAutocompleteKind::ScalarFunction, "Exponential (e^x)"),
    ("logit(p)", "logit(", FormulaAutocompleteKind::ScalarFunction, "Logit transform ln(p/(1-p))"),
    ("probit(p)", "probit(", FormulaAutocompleteKind::ScalarFunction, "Probit transform 5 + inv_norm(p)"),
    ("round(x)", "round(", FormulaAutocompleteKind::ScalarFunction, "Round to nearest integer"),
    ("floor(x)", "floor(", FormulaAutocompleteKind::ScalarFunction, "Floor integer"),
    ("ceil(x)", "ceil(", FormulaAutocompleteKind::ScalarFunction, "Ceiling integer"),
    ("if(cond, a, b)", "if(", FormulaAutocompleteKind::ScalarFunction, "Conditional branch"),
    ("clamp(x, lo, hi)", "clamp(", FormulaAutocompleteKind::ScalarFunction, "Clamp value to [lo, hi]"),
    ("is_defined(x)", "is_defined(", FormulaAutocompleteKind::ScalarFunction, "True when cell is non-missing"),
    ("count(var)", "count(", FormulaAutocompleteKind::AggregateFunction, "Active non-missing count"),
    ("sum(var)", "sum(", FormulaAutocompleteKind::AggregateFunction, "Column sum"),
    ("mean(var)", "mean(", FormulaAutocompleteKind::AggregateFunction, "Column arithmetic mean"),
    ("sd(var)", "sd(", FormulaAutocompleteKind::AggregateFunction, "Sample standard deviation"),
    ("sem(var)", "sem(", FormulaAutocompleteKind::AggregateFunction, "Standard error of the mean"),
    ("median(var)", "median(", FormulaAutocompleteKind::AggregateFunction, "Column median"),
    ("iqr(var)", "iqr(", FormulaAutocompleteKind::AggregateFunction, "Interquartile range"),
    ("percentile(var, p)", "percentile(", FormulaAutocompleteKind::AggregateFunction, "p-th percentile (0..100)"),
    ("zscore(var)", "zscore(", FormulaAutocompleteKind::AggregateFunction, "Standardized z-score"),
    ("countif(cond)", "countif(", FormulaAutocompleteKind::AggregateFunction, "Conditional row count"),
    ("sumif(var, cond)", "sumif(", FormulaAutocompleteKind::AggregateFunction, "Conditional column sum"),
    ("meanif(var, cond)", "meanif(", FormulaAutocompleteKind::AggregateFunction, "Conditional column mean"),
    ("row_number()", "row_number()", FormulaAutocompleteKind::WindowFunction, "1-based row number"),
    ("rank(var)", "rank(", FormulaAutocompleteKind::WindowFunction, "Competition rank"),
    ("dense_rank(var)", "dense_rank(", FormulaAutocompleteKind::WindowFunction, "Dense rank without gaps"),
    ("lag(var, k)", "lag(", FormulaAutocompleteKind::WindowFunction, "Previous row offset k"),
    ("lead(var, k)", "lead(", FormulaAutocompleteKind::WindowFunction, "Next row offset k"),
    ("cumsum(var)", "cumsum(", FormulaAutocompleteKind::WindowFunction, "Cumulative sum"),
    ("cummean(var)", "cummean(", FormulaAutocompleteKind::WindowFunction, "Cumulative running mean"),
    ("diff(var, k)", "diff(", FormulaAutocompleteKind::WindowFunction, "Difference over lag k"),
    ("moving_avg(var, w)", "moving_avg(", FormulaAutocompleteKind::WindowFunction, "Trailing moving average"),
    ("pi", "pi", FormulaAutocompleteKind::Constant, "Circle constant 3.14159..."),
    ("e", "e", FormulaAutocompleteKind::Constant, "Euler constant 2.71828..."),
    ("$row", "$row", FormulaAutocompleteKind::Constant, "1-based row index"),
];

/// In-table `=` Calculated Variable formula bar with live syntax validation and column/function
/// autocomplete (`docs/implementation/data-tables-and-transforms.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FormulaBarState {
    pub active: bool,
    #[serde(default)]
    pub target_sheet_id: Option<SheetId>,
    pub target_column: Option<usize>,
    pub target_title: String,
    pub input: String,
    pub cursor_byte: usize,
    pub completions: Vec<FormulaAutocompleteItem>,
    pub selected_completion: usize,
    pub parse_error: Option<String>,
    pub is_valid: bool,
}

impl FormulaBarState {
    /// Focus the formula bar on column `col_idx` of `sheet`.
    pub fn open_for_column(&mut self, sheet: &DataSheet, col_idx: usize, doc: Option<&ProjectDocument>) {
        self.active = true;
        self.target_sheet_id = Some(sheet.id.clone());
        self.target_column = Some(col_idx);
        self.selected_completion = 0;

        if let TableArchetype::MultipleVariables(mv) = &sheet.archetype
            && let Some(var) = mv.variables.get(col_idx)
        {
            self.target_title = var.title.clone();
            self.input = match &var.source {
                VariableSource::Calculated { formula } => format!("= {formula}"),
                VariableSource::Entered => String::from("="),
            };
        } else {
            self.target_title = default_column_label(col_idx);
            self.input = String::from("=");
        }
        self.cursor_byte = self.input.len();
        self.refresh_validation_and_completions(doc, Some(sheet));
    }

    /// Open the formula bar targeting a new calculated variable named `title` on `sheet`.
    pub fn open_for_new_column(&mut self, sheet: &DataSheet, title: impl Into<String>, doc: Option<&ProjectDocument>) {
        self.active = true;
        self.target_sheet_id = Some(sheet.id.clone());
        self.target_column = None;
        self.selected_completion = 0;
        self.target_title = title.into();
        self.input = String::from("=");
        self.cursor_byte = self.input.len();
        self.refresh_validation_and_completions(doc, Some(sheet));
    }

    /// Update the edited formula buffer and cursor offset (safe for arbitrary UTF-8 positions).
    pub fn set_input(&mut self, text: impl Into<String>, cursor_byte: Option<usize>, doc: &ProjectDocument) {
        self.input = text.into();
        let raw_pos = cursor_byte.unwrap_or(self.input.len()).min(self.input.len());
        self.cursor_byte = clamp_char_boundary(&self.input, raw_pos);
        self.selected_completion = 0;
        let target_sheet = self
            .target_sheet_id
            .as_ref()
            .and_then(|id| doc.data_sheet(id))
            .or_else(|| doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)));
        self.refresh_validation_and_completions(Some(doc), target_sheet);
    }

    /// Accept the currently highlighted autocomplete item into `self.input`.
    pub fn accept_completion(&mut self, doc: &ProjectDocument) -> bool {
        let Some(item) = self.completions.get(self.selected_completion).cloned() else {
            return false;
        };
        let (prefix_start, prefix_end) = active_token_span(&self.input, self.cursor_byte);
        let mut updated = String::with_capacity(self.input.len() + item.insert_text.len());
        updated.push_str(&self.input[..prefix_start]);
        updated.push_str(&item.insert_text);
        let new_cursor = updated.len();
        updated.push_str(&self.input[prefix_end..]);
        self.set_input(updated, Some(new_cursor), doc);
        true
    }

    /// Emit a canonical `"table.setCalculatedVariable"` command intent if the formula parses cleanly.
    pub fn commit_intent(&self, sheet_id: &SheetId) -> Result<ToolCommandIntent, String> {
        if self.target_column.is_none() && self.target_title.trim().is_empty() {
            return Err(String::from("no target variable column selected"));
        }
        if let Some(err) = &self.parse_error {
            return Err(err.clone());
        }
        let normalized = self.input.trim().strip_prefix('=').unwrap_or(self.input.trim()).trim().to_owned();
        if normalized.is_empty() {
            return Err(String::from("calculated variable formula cannot be empty"));
        }
        let mut params = serde_json::Map::new();
        params.insert(String::from("sheet_id"), Value::String(sheet_id.as_str().to_owned()));
        if let Some(col) = self.target_column {
            params.insert(String::from("col"), json!(col));
        }
        if !self.target_title.trim().is_empty() {
            params.insert(String::from("title"), json!(self.target_title));
        }
        params.insert(String::from("formula"), json!(normalized));
        Ok(ToolCommandIntent { command_id: String::from("table.setCalculatedVariable"), params: Value::Object(params) })
    }

    fn refresh_validation_and_completions(&mut self, doc: Option<&ProjectDocument>, sheet: Option<&DataSheet>) {
        let trimmed = self.input.trim();
        let body = trimmed.strip_prefix('=').unwrap_or(trimmed).trim();
        if body.is_empty() {
            self.parse_error = None;
            self.is_valid = false;
        } else {
            match validate_formula_syntax(body) {
                Ok(()) => {
                    if let (Some(TableArchetype::MultipleVariables(mv)), Some(col)) = (sheet.map(|s| &s.archetype), self.target_column)
                        && let Err(err) = curvebook_transform::validate_no_formula_cycle(mv, col, body)
                    {
                        self.parse_error = Some(err.to_string());
                        self.is_valid = false;
                    } else {
                        self.parse_error = None;
                        self.is_valid = true;
                    }
                }
                Err(err) => {
                    self.parse_error = Some(err);
                    self.is_valid = false;
                }
            }
        }

        self.completions = build_autocomplete_items(&self.input, self.cursor_byte, self.target_column, doc, sheet);
        if self.selected_completion >= self.completions.len() {
            self.selected_completion = 0;
        }
    }

    /// Close and reset the formula bar without committing changes.
    pub fn cancel(&mut self) {
        self.active = false;
        self.target_sheet_id = None;
        self.target_column = None;
        self.target_title.clear();
        self.input.clear();
        self.cursor_byte = 0;
        self.completions.clear();
        self.selected_completion = 0;
        self.parse_error = None;
        self.is_valid = false;
    }
}

/// Active stage of the four-stage `Import & Paste Special` / `Paste Link` pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ImportWizardStage {
    #[default]
    Source,
    Filter,
    Placement,
    Provenance,
}

/// Stage 1 origin type in [`ImportPasteWizardState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ImportOriginKind {
    #[default]
    Clipboard,
    InternalDataSheet,
    InternalResultSheet,
    ExternalDelimitedFile,
}

/// Guided structural import reshaping preset (`QOL-2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ImportReshapeMode {
    #[default]
    Plate96,
    Plate384,
    TidyLongPivot,
}

/// Interactive state for Plate Map (`96` / `384`-well) and Tidy Long Pivot import reshaping (`QOL-2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportReshapeState {
    pub open: bool,
    pub mode: ImportReshapeMode,
    pub title: String,
    pub target_kind: TableKind,
    pub x_title: String,
    pub subtract_blank_mean: bool,
    pub plate_values: Vec<Option<f64>>,
    pub plate_roles: Vec<Value>,
    pub tidy_csv: String,
    pub tidy_delimiter: char,
    pub decimal_separator: char,
    pub x_col: Option<String>,
    pub group_col: String,
    pub replicate_col: Option<String>,
    pub value_col: String,
    pub excluded_col: Option<String>,
}

impl Default for ImportReshapeState {
    fn default() -> Self {
        Self {
            open: false,
            mode: ImportReshapeMode::Plate96,
            title: String::from("Reshaped Import"),
            target_kind: TableKind::Grouped,
            x_title: String::from("Concentration"),
            subtract_blank_mean: true,
            plate_values: Vec::new(),
            plate_roles: Vec::new(),
            tidy_csv: String::new(),
            tidy_delimiter: ',',
            decimal_separator: '.',
            x_col: None,
            group_col: String::from("Group"),
            replicate_col: None,
            value_col: String::from("Value"),
            excluded_col: None,
        }
    }
}

impl ImportReshapeState {
    /// Target engine command identifier for the currently selected reshape mode.
    #[must_use]
    pub const fn command_id(&self) -> &'static str {
        match self.mode {
            ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384 => "table.importPlateMap",
            ImportReshapeMode::TidyLongPivot => "table.importTidyLong",
        }
    }

    /// Build the engine [`ToolCommandIntent`] (`table.importPlateMap` or `table.importTidyLong`).
    ///
    /// # Errors
    /// Returns a descriptive message when plate geometry dimensions or tidy column mappings are incomplete.
    pub fn build_import_intent(&self) -> Result<ToolCommandIntent, String> {
        let title = if self.title.trim().is_empty() { "Reshaped Import" } else { self.title.trim() };
        let target_kind = target_kind_wire_name(self.target_kind);
        match self.mode {
            ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384 => {
                let (geom_label, expected_wells) = match self.mode {
                    ImportReshapeMode::Plate96 => ("Wells96", 96usize),
                    ImportReshapeMode::Plate384 => ("Wells384", 384usize),
                    ImportReshapeMode::TidyLongPivot => ("Wells96", 96usize),
                };
                if self.plate_values.len() != expected_wells {
                    return Err(format!("expected {expected_wells} well values for {geom_label}, got {}", self.plate_values.len()));
                }
                if self.plate_roles.len() != expected_wells {
                    return Err(format!("expected {expected_wells} well roles for {geom_label}, got {}", self.plate_roles.len()));
                }
                Ok(ToolCommandIntent::new(
                    "table.importPlateMap",
                    json!({
                        "title": title,
                        "geometry": geom_label,
                        "values": self.plate_values,
                        "roles": self.plate_roles,
                        "x_title": self.x_title,
                        "target_kind": target_kind,
                        "subtract_blank_mean": self.subtract_blank_mean,
                    }),
                ))
            }
            ImportReshapeMode::TidyLongPivot => {
                if self.tidy_csv.trim().is_empty() {
                    return Err(String::from("tidy CSV payload cannot be empty"));
                }
                if self.group_col.trim().is_empty() || self.value_col.trim().is_empty() {
                    return Err(String::from("tidy long mapping requires both `group_col` and `value_col`"));
                }
                let mut mapping = serde_json::Map::new();
                mapping.insert(String::from("group_col"), Value::String(self.group_col.clone()));
                mapping.insert(String::from("value_col"), Value::String(self.value_col.clone()));
                if let Some(x) = &self.x_col
                    && !x.trim().is_empty()
                {
                    mapping.insert(String::from("x_col"), Value::String(x.clone()));
                }
                if let Some(rep) = &self.replicate_col
                    && !rep.trim().is_empty()
                {
                    mapping.insert(String::from("replicate_col"), Value::String(rep.clone()));
                }
                if let Some(excl) = &self.excluded_col
                    && !excl.trim().is_empty()
                {
                    mapping.insert(String::from("excluded_col"), Value::String(excl.clone()));
                }
                Ok(ToolCommandIntent::new(
                    "table.importTidyLong",
                    json!({
                        "title": title,
                        "csv": self.tidy_csv,
                        "delimiter": self.tidy_delimiter.to_string(),
                        "decimal_sep": if self.decimal_separator == ',' { "," } else { "." },
                        "mapping": Value::Object(mapping),
                        "target_kind": target_kind,
                    }),
                ))
            }
        }
    }

    /// Patch `ImportReshape` dialog state from scripted GUI actions / text fields.
    ///
    /// # Errors
    /// Returns an error if `mode`, `target_kind`, `delimiter`, `plate_values`, or `plate_roles` is invalid.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Patch `ImportReshape` dialog state using regional numeric parsing (`preset`).
    ///
    /// # Errors
    /// Returns an error if any field name or value is invalid.
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        let active_preset = if fields.contains_key("decimal_separator") || fields.contains_key("import.decimal_separator") {
            preset.clone()
        } else {
            let mut p = preset.clone();
            if self.decimal_separator == ',' || self.decimal_separator == '.' {
                p.decimal_separator = self.decimal_separator;
            }
            p
        };
        let effective_preset =
            if let Some(ds) = fields.get("decimal_separator").or_else(|| fields.get("import.decimal_separator")).and_then(Value::as_str) {
                let mut p = active_preset;
                p.decimal_separator = if ds.trim() == "," { ',' } else { '.' };
                p
            } else {
                active_preset
            };
        for key in ["mode", "import.mode", "reshape_mode"] {
            if let Some(val) = fields.get(key) {
                let raw = val.as_str().ok_or_else(|| String::from("`mode` must be a string"))?;
                let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
                self.mode = match norm.as_str() {
                    "plate96" | "plates96" | "wells96" | "96" => ImportReshapeMode::Plate96,
                    "plate384" | "plates384" | "wells384" | "384" => ImportReshapeMode::Plate384,
                    "tidylongpivot" | "tidylong" | "tidy" => ImportReshapeMode::TidyLongPivot,
                    _ => return Err(format!("unsupported ImportReshape mode `{raw}`")),
                };
            }
        }
        for (key, val) in fields {
            match key.as_str() {
                "mode" | "import.mode" | "reshape_mode" => {}
                "title" | "import.title" | "sheet_title" => {
                    self.title = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.to_owned();
                }
                "target_kind" | "import.target_kind" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`target_kind` must be a string"))?;
                    self.target_kind = crate::dialogs_canvas::parse_table_kind_str(raw)?;
                }
                "x_title" | "import.x_title" => {
                    self.x_title = val.as_str().ok_or_else(|| String::from("`x_title` must be a string"))?.to_owned();
                }
                "subtract_blank_mean" | "import.subtract_blank_mean" => {
                    self.subtract_blank_mean = val.as_bool().ok_or_else(|| String::from("`subtract_blank_mean` must be boolean"))?;
                }
                "tidy_csv" | "csv" | "import.tidy_csv" | "import.csv" => {
                    self.tidy_csv = val.as_str().ok_or_else(|| String::from("`tidy_csv` must be a string"))?.to_owned();
                }
                "data" | "text" | "import.data" => {
                    let s = val.as_str().ok_or_else(|| String::from("`data` must be a string"))?;
                    if self.mode == ImportReshapeMode::TidyLongPivot {
                        self.tidy_csv = s.to_owned();
                    } else {
                        self.plate_values = parse_plate_values_text(s, &effective_preset)?;
                    }
                }
                "plate_values" | "values" | "import.plate_values" => {
                    self.plate_values = parse_plate_values_json(val, &effective_preset)?;
                }
                "plate_roles" | "roles" | "import.plate_roles" => {
                    let arr = val.as_array().ok_or_else(|| String::from("`plate_roles` must be an array"))?;
                    self.plate_roles = arr.clone();
                }
                "tidy_delimiter" | "delimiter" | "import.delimiter" => {
                    let s = val.as_str().ok_or_else(|| String::from("`delimiter` must be a string"))?;
                    self.tidy_delimiter = match s.trim().to_ascii_lowercase().as_str() {
                        "tab" | "\\t" | "\t" => '\t',
                        "comma" | "," => ',',
                        "semicolon" | ";" => ';',
                        "pipe" | "|" => '|',
                        _ => s.chars().next().ok_or_else(|| String::from("`delimiter` cannot be empty"))?,
                    };
                }
                "decimal_separator" | "decimal_sep" | "import.decimal_separator" => {
                    let s = val.as_str().ok_or_else(|| String::from("`decimal_separator` must be a string"))?;
                    self.decimal_separator = match s.trim() {
                        "," => ',',
                        "." => '.',
                        _ => return Err(format!("unsupported decimal_separator `{s}`")),
                    };
                }
                "x_col" | "import.x_col" => {
                    self.x_col = opt_nonempty_string(val, key)?;
                }
                "group_col" | "import.group_col" => {
                    self.group_col = val.as_str().ok_or_else(|| String::from("`group_col` must be a string"))?.to_owned();
                }
                "replicate_col" | "import.replicate_col" => {
                    self.replicate_col = opt_nonempty_string(val, key)?;
                }
                "value_col" | "import.value_col" => {
                    self.value_col = val.as_str().ok_or_else(|| String::from("`value_col` must be a string"))?.to_owned();
                }
                "excluded_col" | "import.excluded_col" => {
                    self.excluded_col = opt_nonempty_string(val, key)?;
                }
                other => return Err(format!("unknown Import Reshape field `{other}`")),
            }
        }
        Ok(())
    }
}

fn opt_nonempty_string(val: &Value, field: &str) -> Result<Option<String>, String> {
    if val.is_null() {
        return Ok(None);
    }
    let s = val.as_str().ok_or_else(|| format!("`{field}` must be a string or null"))?.trim();
    Ok((!s.is_empty()).then(|| s.to_owned()))
}

fn parse_plate_cell_token(tok: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    let t = tok.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("na") || t.eq_ignore_ascii_case("null") || t == "-" {
        return Ok(None);
    }
    parse_localized_number_input(t, preset).filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("invalid plate numeric token `{t}`"))
}

fn parse_plate_values_text(raw: &str, preset: &LocalePreset) -> Result<Vec<Option<f64>>, String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains('\t') || trimmed.contains(';') || (preset.decimal_separator != ',' && trimmed.contains(',')) {
            let split_iter: Box<dyn Iterator<Item = &str>> =
                if preset.decimal_separator == ',' { Box::new(trimmed.split(['\t', ';'])) } else { Box::new(trimmed.split([',', '\t', ';'])) };
            for cell in split_iter {
                out.push(parse_plate_cell_token(cell, preset)?);
            }
        } else {
            for cell in trimmed.split_whitespace() {
                out.push(parse_plate_cell_token(cell, preset)?);
            }
        }
    }
    Ok(out)
}

fn parse_plate_values_json(val: &Value, preset: &LocalePreset) -> Result<Vec<Option<f64>>, String> {
    if let Some(s) = val.as_str() {
        return parse_plate_values_text(s, preset);
    }
    let arr = val.as_array().ok_or_else(|| String::from("`plate_values` must be an array or delimited string"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(inner) = item.as_array() {
            for sub in inner {
                out.push(parse_plate_scalar(sub, preset)?);
            }
        } else {
            out.push(parse_plate_scalar(item, preset)?);
        }
    }
    Ok(out)
}

fn parse_plate_scalar(val: &Value, preset: &LocalePreset) -> Result<Option<f64>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(f) = val.as_f64() {
        return f.is_finite().then_some(Some(f)).ok_or_else(|| String::from("plate well value must be finite"));
    }
    if let Some(s) = val.as_str() {
        return parse_plate_cell_token(s, preset);
    }
    Err(String::from("plate well value must be a number, string, or null"))
}

fn target_kind_wire_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Column => "Column",
        TableKind::Grouped => "Grouped",
        TableKind::Xy => "Xy",
        TableKind::MultipleVariables => "MultipleVariables",
        _ => "Grouped",
    }
}

/// Complete 4-stage `Import & Paste Special` / `Paste Link` UI state model
/// (`docs/implementation/data-tables-and-transforms.md` §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPasteWizardState {
    pub open: bool,
    pub stage: ImportWizardStage,
    pub is_live_link: bool,
    pub origin_kind: ImportOriginKind,
    pub raw_clipboard_text: String,
    #[serde(default)]
    pub target_sheet_id: Option<SheetId>,
    pub source_sheet_id: Option<SheetId>,
    pub source_tab_id: Option<String>,
    pub source_range: SheetCellRange,
    pub external_file_path: String,
    pub delimiter: char,
    pub decimal_separator: char,
    pub filter: ImportFilterSpec,
    pub placement: ImportPlacementSpec,
    pub link_id: String,
    pub provenance_note: String,
    pub record_in_info_sheet: bool,
}

impl Default for ImportPasteWizardState {
    fn default() -> Self {
        Self {
            open: false,
            stage: ImportWizardStage::Source,
            is_live_link: false,
            origin_kind: ImportOriginKind::Clipboard,
            raw_clipboard_text: String::new(),
            target_sheet_id: None,
            source_sheet_id: None,
            source_tab_id: None,
            source_range: SheetCellRange::default(),
            external_file_path: String::new(),
            delimiter: '\t',
            decimal_separator: '.',
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
            link_id: String::from("link-1"),
            provenance_note: String::new(),
            record_in_info_sheet: true,
        }
    }
}

impl ImportPasteWizardState {
    /// Advance to the next stage (`Source -> Filter -> Placement -> Provenance`).
    pub fn next_stage(&mut self) {
        self.stage = match self.stage {
            ImportWizardStage::Source => ImportWizardStage::Filter,
            ImportWizardStage::Filter => ImportWizardStage::Placement,
            ImportWizardStage::Placement | ImportWizardStage::Provenance => ImportWizardStage::Provenance,
        };
    }

    /// Step back to the previous stage.
    pub fn prev_stage(&mut self) {
        self.stage = match self.stage {
            ImportWizardStage::Source | ImportWizardStage::Filter => ImportWizardStage::Source,
            ImportWizardStage::Placement => ImportWizardStage::Filter,
            ImportWizardStage::Provenance => ImportWizardStage::Placement,
        };
    }

    /// Build the [`LinkSource`] corresponding to Stage 1 settings.
    #[must_use]
    pub fn build_link_source(&self, doc: Option<&ProjectDocument>) -> LinkSource {
        match self.origin_kind {
            ImportOriginKind::Clipboard => LinkSource::Clipboard,
            ImportOriginKind::InternalDataSheet => LinkSource::InternalSheet {
                sheet_id: self.source_sheet_id.clone().unwrap_or_else(|| SheetId::new("data-1")),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: self.source_range,
            },
            ImportOriginKind::InternalResultSheet => {
                let sheet_id = self.source_sheet_id.clone().unwrap_or_else(|| SheetId::new("res-1"));
                let kind = doc.and_then(|d| d.result_sheet(&sheet_id)).map_or(InternalLinkKind::TerminalResultSheet, |rs| {
                    if rs.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet }
                });
                LinkSource::InternalSheet { sheet_id, kind, tab_id: self.source_tab_id.clone(), range: self.source_range }
            }
            ImportOriginKind::ExternalDelimitedFile => LinkSource::ExternalDelimitedFile {
                path: self.external_file_path.clone(),
                delimiter: self.delimiter,
                decimal_separator: self.decimal_separator,
            },
        }
    }

    /// Assemble the full [`ImportLinkSpec`] across Stages 1–3.
    #[must_use]
    pub fn to_link_spec(&self, doc: Option<&ProjectDocument>) -> ImportLinkSpec {
        ImportLinkSpec { source: self.build_link_source(doc), filter: self.filter.clone(), placement: self.placement.clone() }
    }

    /// Determine the provenance border badge color that will surround the placed region.
    #[must_use]
    pub fn preview_border_color(&self, doc: Option<&ProjectDocument>) -> Option<LinkBorderColor> {
        if !self.is_live_link {
            return None;
        }
        self.build_link_source(doc).border_color()
    }

    /// Preview Stage 1–3 cells cleanly (honoring delimiter, decimal separator, `*` exclusion, and
    /// `transpose`) without mutating the document.
    #[must_use]
    pub fn preview_grid(&self, doc: Option<&ProjectDocument>) -> Vec<Vec<Cell>> {
        let mut grid = match self.origin_kind {
            ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile => self
                .raw_clipboard_text
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| {
                    line.split(self.delimiter)
                        .map(|tok| Cell::parse_import_token(tok, self.decimal_separator, self.filter.parse_asterisk_exclusions))
                        .collect::<Vec<Cell>>()
                })
                .collect::<Vec<Vec<Cell>>>(),
            ImportOriginKind::InternalDataSheet => doc
                .and_then(|d| self.source_sheet_id.as_ref().and_then(|id| d.data_sheet(id)))
                .map(|ds| ds.extract_cell_grid(self.source_range))
                .unwrap_or_default(),
            ImportOriginKind::InternalResultSheet => Vec::new(),
        };

        if let Some((r0, r1)) = self.filter.row_window {
            if r0 <= r1 && r0 < grid.len() {
                let end = r1.min(grid.len() - 1);
                grid = grid[r0..=end].to_vec();
            } else {
                grid.clear();
            }
        }
        if let Some((c0, c1)) = self.filter.col_window {
            for row in &mut grid {
                if c0 <= c1 && c0 < row.len() {
                    let end = c1.min(row.len() - 1);
                    *row = row[c0..=end].to_vec();
                } else {
                    row.clear();
                }
            }
        }
        if let Some(k) = self.filter.decimate_every_k
            && k >= 1
        {
            grid = grid.into_iter().step_by(k).collect();
        }
        if self.placement.transpose {
            grid = curvebook_doc::transpose_cell_grid(&grid);
        }
        grid
    }

    /// Emit `"table.pasteLink"` (for live links) or `"table.paste"` (for static pastes).
    #[must_use]
    pub fn build_command_intent(&self, target_sheet_id: &SheetId, doc: Option<&ProjectDocument>) -> ToolCommandIntent {
        let spec = self.to_link_spec(doc);
        if self.is_live_link && !matches!(spec.source, LinkSource::Clipboard) {
            let link_id = (!self.link_id.trim().is_empty()).then_some(self.link_id.as_str());
            let provenance = (!self.provenance_note.trim().is_empty()).then_some(self.provenance_note.as_str());
            return ToolCommandIntent {
                command_id: String::from("table.pasteLink"),
                params: json!({
                    "target_sheet_id": target_sheet_id.as_str(),
                    "link_id": link_id,
                    "link_spec": spec,
                    "provenance": provenance,
                    "record_in_info_sheet": self.record_in_info_sheet,
                }),
            };
        }

        let delimiter = if self.delimiter == '\0' { String::from("\t") } else { self.delimiter.to_string() };
        let decimal_separator = if self.decimal_separator == ',' { "," } else { "." };
        ToolCommandIntent {
            command_id: String::from("table.paste"),
            params: json!({
                "sheet_id": target_sheet_id.as_str(),
                "text": self.raw_clipboard_text,
                "delimiter": delimiter,
                "decimal_separator": decimal_separator,
                "anchor": self.placement.anchor,
                "mode": self.placement.mode,
                "transpose": self.placement.transpose,
                "propagate_title_header": self.placement.propagate_title_header,
                "skip_contiguous_blanks": self.placement.skip_contiguous_blanks,
                "filter": self.filter,
            }),
        }
    }
}

/// Non-modal `Format Link` inspector opened by double-clicking a linked cell.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FormatLinkInspectorState {
    pub open: bool,
    pub region_id: String,
    pub border_color: Option<LinkBorderColor>,
    pub provenance_summary: String,
    pub spec: Option<ImportLinkSpec>,
}

/// View-layer state for the virtualized spreadsheet grid and selection status bar.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GridViewState {
    pub tool: GridToolState,
    pub quick_stats: Option<SelectionQuickStats>,
    pub extended_stats: Option<GridQuickStats>,
    pub decimal_separator: char,
    pub row_titles_collapsed_override: Option<bool>,
    pub variables_dialog: VariablesDialogState,
    pub formula_bar: FormulaBarState,
    pub import_wizard: ImportPasteWizardState,
    #[serde(default)]
    pub import_reshape: ImportReshapeState,
    pub format_link_inspector: FormatLinkInspectorState,
    #[serde(default)]
    pub hover_exclusion_tooltip: Option<String>,
    #[serde(default)]
    pub selected_rows: Vec<usize>,
    #[serde(default)]
    pub selected_coords: Vec<GridCellCoord>,
    /// Currently focused/active X (or Time) column cell `(row, subcol)` for XY / Survival sheets.
    #[serde(default)]
    pub active_x_cell: Option<(usize, usize)>,
}

/// Resolved selection target for grid clipboard copy/cut operations (`R15-GUI-19`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridClipboardTarget {
    /// Uncapped contiguous rectangular range clamped to valid sheet bounds.
    Range(SheetCellRange),
    /// Exact deduplicated discontiguous cells sorted in reading order.
    Cells(Vec<CellCoord>),
    /// No explicit cell selection: copy falls back to whole-sheet TSV, and cut is a no-op.
    WholeSheetFallback,
}

impl GridClipboardTarget {
    /// Build `table.clearCells` parameter payloads clearing only the exact target selection.
    ///
    /// - Contiguous ranges emit compact `range` payloads (uncapped in rows, never truncating at
    ///   `MAX_MATERIALIZED_SELECTION_COORDS`).
    /// - Discontiguous selections emit a single `cells` payload listing only selected coordinates
    ///   so unselected cells inside the enclosing bounding box remain untouched.
    #[must_use]
    pub fn clear_cells_requests(&self, sheet: &DataSheet) -> Vec<Value> {
        match self {
            Self::WholeSheetFallback => Vec::new(),
            Self::Cells(coords) => {
                if coords.is_empty() {
                    return Vec::new();
                }
                let cells_json: Vec<Value> = coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect();
                vec![json!({ "sheet_id": sheet.id.as_str(), "cells": cells_json })]
            }
            Self::Range(range) => range_clear_cells_requests(sheet, *range),
        }
    }
}

/// Prepared clipboard TSV text and exact target used by both `egui` shell and desktop host (`R15-GUI-19`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridClipboardSelectionPlan {
    pub tsv: String,
    pub target: GridClipboardTarget,
}

impl GridClipboardSelectionPlan {
    /// Build `table.clearCells` parameter payloads for cutting this selection.
    #[must_use]
    pub fn clear_cells_requests(&self, sheet: &DataSheet) -> Vec<Value> {
        self.target.clear_cells_requests(sheet)
    }
}

fn range_clear_cells_requests(sheet: &DataSheet, range: SheetCellRange) -> Vec<Value> {
    let mut groups: Vec<(usize, usize, usize, usize)> = Vec::new();
    for col in range.start.col..=range.end.col {
        let max_sub = data_set_subcolumn_count(&sheet.archetype, col).saturating_sub(1);
        let s_min = if col == range.start.col { range.start.subcol.min(max_sub) } else { 0 };
        let s_max = if col == range.end.col { range.end.subcol.min(max_sub) } else { max_sub };
        if s_min > s_max {
            continue;
        }
        if let Some(last) = groups.last_mut()
            && last.1 + 1 == col
            && last.2 == s_min
            && last.3 == s_max
        {
            last.1 = col;
        } else {
            groups.push((col, col, s_min, s_max));
        }
    }
    groups
        .into_iter()
        .map(|(col_start, col_end, sub_start, sub_end)| {
            let sub_range =
                SheetCellRange::new(CellCoord::new(range.start.row, col_start, sub_start), CellCoord::new(range.end.row, col_end, sub_end));
            json!({ "sheet_id": sheet.id.as_str(), "range": sub_range })
        })
        .collect()
}

fn format_discontiguous_cells_tsv(sheet: &DataSheet, coords: &[CellCoord], decimal_separator: char) -> String {
    if coords.is_empty() {
        return String::new();
    }
    let mut rows: BTreeSet<usize> = BTreeSet::new();
    let mut cols: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut selected: HashSet<(usize, usize, usize)> = HashSet::with_capacity(coords.len());
    for c in coords {
        rows.insert(c.row);
        cols.insert((c.col, c.subcol));
        selected.insert((c.row, c.col, c.subcol));
    }
    let opts = CsvOptions { decimal_separator, ..CsvOptions::tsv() };
    let records: Vec<Vec<String>> = rows
        .into_iter()
        .map(|row| {
            cols.iter()
                .map(|&(col, subcol)| {
                    if selected.contains(&(row, col, subcol)) {
                        sheet.cell(CellCoord::new(row, col, subcol)).map_or_else(String::new, |cell| format_cell_token(cell, &opts))
                    } else {
                        String::new()
                    }
                })
                .collect()
        })
        .collect();
    write_delimited_records(&records, &opts)
}

impl GridViewState {
    /// Format the exclusion tooltip string for `coord` on `sheet`, including auditable reason tag
    /// and note when present (`QOL-3`).
    #[must_use]
    pub fn cell_exclusion_tooltip(sheet: &DataSheet, coord: GridCellCoord) -> Option<String> {
        let cell_coord = CellCoord::from(coord);
        let is_excluded = sheet.archetype.data_cell(cell_coord).is_some_and(|cell| cell.excluded);
        let note = sheet.exclusion_note(cell_coord);
        match (is_excluded, note) {
            (_, Some(entry)) => {
                if entry.note.trim().is_empty() {
                    Some(format!("Excluded [{}] ({})", entry.reason_tag, entry.timestamp_utc))
                } else {
                    Some(format!("Excluded [{}]: {} ({})", entry.reason_tag, entry.note, entry.timestamp_utc))
                }
            }
            (true, None) => Some(String::from("Excluded from analyses")),
            (false, None) => None,
        }
    }

    /// Refresh [`Self::hover_exclusion_tooltip`] from the cell under pointer `coord`.
    pub fn update_hover_exclusion_tooltip(&mut self, sheet: &DataSheet, coord: Option<GridCellCoord>) -> Option<&str> {
        self.hover_exclusion_tooltip = coord.and_then(|c| Self::cell_exclusion_tooltip(sheet, c));
        self.hover_exclusion_tooltip.as_deref()
    }

    /// Clear multi-selection and select a single coordinate (`coord`).
    pub fn select_single_sync(&mut self, coord: GridCellCoord) {
        self.active_x_cell = None;
        self.tool.select_single(coord);
        self.selected_rows = vec![coord.row];
        self.selected_coords = vec![coord];
    }

    /// Focus/select a single X-column (or Survival Time-column) cell at `(row, subcol)`.
    pub fn select_x_cell_sync(&mut self, row: usize, subcol: usize) {
        self.active_x_cell = Some((row, subcol));
        self.tool.active = None;
        self.tool.anchor = None;
        self.selected_rows = vec![row];
        self.selected_coords.clear();
    }

    /// Extend a contiguous rectangular / multi-row selection (`Shift+Click` or pointer drag) in `O(1)`
    /// range bounds without materializing every cell coordinate in multi-million-cell ranges (`RV-50`).
    pub fn extend_shift_range(&mut self, sheet: &DataSheet, coord: GridCellCoord) {
        self.active_x_cell = None;
        self.tool.extend_selection(coord);
        self.selected_coords.clear();
        self.selected_rows = self.bounded_selected_rows(sheet);
    }

    /// Toggle a cell coordinate in a discontiguous multi-selection (`Cmd/Ctrl+Click`).
    pub fn toggle_cmd_selection(&mut self, sheet: &DataSheet, coord: GridCellCoord) {
        use std::collections::BTreeSet;

        self.active_x_cell = None;
        let mut set: BTreeSet<GridCellCoord> = if !self.selected_coords.is_empty() {
            self.selected_coords.iter().copied().collect()
        } else {
            self.selection_coords(sheet).into_iter().collect()
        };
        if set.contains(&coord) {
            if set.len() > 1 {
                set.remove(&coord);
            }
        } else if set.len() < MAX_MATERIALIZED_SELECTION_COORDS {
            set.insert(coord);
        }
        self.tool.active = Some(coord);
        if self.tool.anchor.is_none() {
            self.tool.anchor = Some(coord);
        }
        self.selected_rows = set.iter().map(|c| c.row).collect::<BTreeSet<_>>().into_iter().collect();
        self.selected_coords = set.into_iter().collect();
    }

    /// Select or toggle an entire row (across all data columns of `sheet`), supporting `Shift+Click`
    /// range and `Cmd/Ctrl+Click` discontiguous multi-row selection (`WF-22`).
    pub fn select_row_click(&mut self, sheet: &DataSheet, row: usize, shift: bool, command: bool) {
        self.active_x_cell = None;
        let total_rows = sheet.archetype.row_count();
        if total_rows == 0 {
            self.selected_rows.clear();
            self.selected_coords.clear();
            return;
        }
        let max_row = total_rows - 1;
        let clamped_row = row.min(max_row);
        let col_count = sheet.archetype.column_count().max(1);
        let last_col = col_count - 1;
        let last_subcol = data_set_subcolumn_count(&sheet.archetype, last_col).saturating_sub(1);
        let first_coord = GridCellCoord::new(clamped_row, 0, 0);
        let last_coord = GridCellCoord::new(clamped_row, last_col, last_subcol);

        if shift {
            let anchor_row = self.tool.anchor.map_or(clamped_row, |a| a.row.min(max_row));
            self.tool.select_single(GridCellCoord::new(anchor_row, 0, 0));
            self.tool.extend_selection(last_coord);
            self.selected_coords.clear();
            self.selected_rows = bounded_row_range_vec(sheet, anchor_row.min(clamped_row), anchor_row.max(clamped_row));
        } else if command {
            self.toggle_command_row_selection(sheet, clamped_row, col_count, first_coord);
        } else {
            self.tool.select_single(first_coord);
            self.tool.extend_selection(last_coord);
            self.selected_rows = vec![clamped_row];
            self.selected_coords.clear();
        }
        self.refresh_selection_stats_for_sheet(sheet);
    }

    fn toggle_command_row_selection(&mut self, sheet: &DataSheet, clamped_row: usize, col_count: usize, first_coord: GridCellCoord) {
        use std::collections::BTreeSet;

        let mut row_set: BTreeSet<usize> = self.selected_rows.iter().copied().collect();
        let mut coord_set: BTreeSet<GridCellCoord> = if !self.selected_coords.is_empty() {
            self.selected_coords.iter().copied().collect()
        } else {
            self.selection_coords(sheet).into_iter().collect()
        };
        if row_set.contains(&clamped_row) {
            if row_set.len() > 1 {
                row_set.remove(&clamped_row);
                coord_set.retain(|c| c.row != clamped_row);
            }
        } else {
            if row_set.len() < MAX_MATERIALIZED_SELECTION_COORDS {
                row_set.insert(clamped_row);
            }
            append_row_coords_bounded(&sheet.archetype, clamped_row, col_count, &mut coord_set);
        }
        self.selected_rows = row_set.into_iter().collect();
        self.selected_coords = coord_set.into_iter().collect();
        self.tool.active = Some(first_coord);
        if self.tool.anchor.is_none() {
            self.tool.anchor = Some(first_coord);
        }
    }

    fn bounded_selected_rows(&self, sheet: &DataSheet) -> Vec<usize> {
        let total_rows = sheet.archetype.row_count();
        let Some((min_c, max_c)) = self.tool.selection_bounds() else {
            return Vec::new();
        };
        if total_rows == 0 || min_c.row >= total_rows {
            return Vec::new();
        }
        bounded_row_range_vec(sheet, min_c.row, max_c.row.min(total_rows - 1))
    }

    /// Check in `O(1)` time for rectangular ranges (or `O(log k)` for discontiguous selections)
    /// whether `coord` is currently selected on `sheet`.
    #[must_use]
    pub fn is_cell_selected(&self, sheet: &DataSheet, coord: GridCellCoord) -> bool {
        let total_rows = sheet.archetype.row_count();
        let total_flat = total_flat_data_subcolumns(&sheet.archetype);
        if coord.row >= total_rows || total_flat == 0 {
            return false;
        }
        if self.has_discontiguous_selection() {
            return self.selected_coords.binary_search(&coord).is_ok() || self.selected_coords.contains(&coord);
        }
        let Some((r0, r1, f0, f1)) = selection_flat_bounds(&sheet.archetype, self.tool.anchor, self.tool.active) else {
            return false;
        };
        let flat = flat_index_for_coord(&sheet.archetype, coord.col, coord.subcol);
        (r0..=r1).contains(&coord.row) && (f0..=f1).contains(&flat)
    }

    fn has_discontiguous_selection(&self) -> bool {
        self.selected_coords.len() > 1 || (!self.selected_coords.is_empty() && self.tool.anchor == self.tool.active)
    }

    /// Resolve the uncapped clipboard copy/cut target for `sheet` (`R15-GUI-19`):
    /// - discontiguous selections resolve to their exact deduplicated cell coordinates;
    /// - contiguous rectangular selections resolve to uncapped `selection_bounds` clamped to `sheet`;
    /// - empty selections fall back to whole-sheet copy (and cut is a no-op).
    #[must_use]
    pub fn clipboard_target(&self, sheet: &DataSheet) -> GridClipboardTarget {
        let total_rows = sheet.archetype.row_count();
        let total_cols = sheet.archetype.column_count();
        if total_rows == 0 || total_cols == 0 {
            return GridClipboardTarget::WholeSheetFallback;
        }
        if self.has_discontiguous_selection() {
            let mut unique: BTreeSet<(usize, usize, usize)> = BTreeSet::new();
            for &c in &self.selected_coords {
                if c.row < total_rows && c.col < total_cols && c.subcol < data_set_subcolumn_count(&sheet.archetype, c.col) {
                    unique.insert((c.row, c.col, c.subcol));
                }
            }
            if !unique.is_empty() {
                let coords = unique.into_iter().map(|(row, col, subcol)| CellCoord::new(row, col, subcol)).collect();
                return GridClipboardTarget::Cells(coords);
            }
        }
        if let Some((min_c, max_c)) = self.tool.selection_bounds()
            && min_c.row < total_rows
            && min_c.col < total_cols
        {
            let start_row = min_c.row;
            let end_row = max_c.row.min(total_rows - 1);
            let start_col = min_c.col;
            let end_col = max_c.col.min(total_cols - 1);
            let start_sub = min_c.subcol.min(data_set_subcolumn_count(&sheet.archetype, start_col).saturating_sub(1));
            let end_sub = max_c.subcol.min(data_set_subcolumn_count(&sheet.archetype, end_col).saturating_sub(1));
            return GridClipboardTarget::Range(SheetCellRange::new(
                CellCoord::new(start_row, start_col, start_sub),
                CellCoord::new(end_row, end_col, end_sub),
            ));
        }
        GridClipboardTarget::WholeSheetFallback
    }

    /// Format the current grid selection on `sheet` as TSV and capture the exact target used by
    /// both copy and cut across the `egui` shell and the desktop host (`R15-GUI-19`).
    ///
    /// # Errors
    /// Returns an error if whole-sheet fallback CSV/TSV export fails.
    pub fn prepare_clipboard_selection(&self, sheet: &DataSheet, decimal_separator: char) -> Result<GridClipboardSelectionPlan, String> {
        let target = self.clipboard_target(sheet);
        let tsv = match &target {
            GridClipboardTarget::Range(range) => ClipboardBundle::tsv_from_datasheet_range(sheet, *range, decimal_separator),
            GridClipboardTarget::Cells(coords) => format_discontiguous_cells_tsv(sheet, coords, decimal_separator),
            GridClipboardTarget::WholeSheetFallback => ClipboardBundle::tsv_from_datasheet(sheet, decimal_separator)?,
        };
        Ok(GridClipboardSelectionPlan { tsv, target })
    }

    fn refresh_selection_stats_slice(&mut self, sheet: &DataSheet) {
        let coords = self.all_selected_coords(sheet);
        let Some(_first) = coords.first().copied() else {
            return;
        };
        let total = coords.len();
        if total <= MAX_QUICK_STATS_CELLS_FOR_CLONE_PASS {
            let ext = GridQuickStats::from_coords(sheet, &coords);
            self.extended_stats = Some(ext);
            self.quick_stats = Some(SelectionQuickStats {
                selected_cells: ext.selected_cells,
                numeric_count: ext.n,
                excluded_count: ext.excluded,
                sum: ext.sum,
                mean: ext.mean,
                sd: ext.sd,
                sem: ext.sem,
                median: ext.median,
                min: ext.min,
                max: ext.max,
            });
        } else {
            self.extended_stats = Some(GridQuickStats { selected_cells: total, ..GridQuickStats::default() });
            self.quick_stats = None;
        }
    }

    fn refresh_selection_stats_for_sheet(&mut self, sheet: &DataSheet) {
        if !self.selected_coords.is_empty() {
            self.refresh_selection_stats_slice(sheet);
            return;
        }
        let (Some(a), Some(b)) = (self.tool.anchor, self.tool.active) else {
            self.quick_stats = None;
            self.extended_stats = None;
            return;
        };
        let selected_cells = selection_cell_count(sheet, a, b);
        if selected_cells <= MAX_QUICK_STATS_CELLS_FOR_CLONE_PASS {
            self.extended_stats = Some(GridQuickStats::from_data_sheet(sheet, a, b));
            self.quick_stats = Some(SelectionQuickStats::from_data_sheet(sheet, a, b));
        } else {
            self.extended_stats = Some(GridQuickStats { selected_cells, ..GridQuickStats::default() });
            self.quick_stats = None;
        }
    }

    /// Return all currently selected [`GridCellCoord`] entries (combining rectangular range and
    /// discontiguous `Cmd/Ctrl+Click` selections), bounded to populated sheet dimensions.
    #[must_use]
    pub fn all_selected_coords(&self, sheet: &DataSheet) -> Vec<GridCellCoord> {
        if !self.selected_coords.is_empty() {
            let total_rows = sheet.archetype.row_count();
            let cap = self.selected_coords.len().min(MAX_MATERIALIZED_SELECTION_COORDS);
            let mut seen = HashSet::with_capacity(cap);
            let mut out = Vec::with_capacity(cap);
            for &c in &self.selected_coords {
                if c.row < total_rows && seen.insert((c.row, c.col, c.subcol)) {
                    out.push(c);
                    if out.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                        break;
                    }
                }
            }
            return out;
        }
        self.selection_coords(sheet)
    }

    /// Map currently selected grid rows/cells into graph `(series_index, point_index)` targets.
    #[must_use]
    pub fn selected_graph_point_indices(&self, sheet: &DataSheet, max_series: usize) -> Vec<(usize, usize)> {
        let targets = self.selected_graph_point_targets(sheet, max_series);
        let mut seen = HashSet::with_capacity(targets.len());
        let mut out = Vec::with_capacity(targets.len());
        for (s, p, _) in targets {
            if seen.insert((s, p)) {
                out.push((s, p));
            }
        }
        out
    }

    /// Map currently selected grid rows/cells into `(series_index, point_index, subcol)` targets,
    /// intersecting with populated sheet rows and deduplicating in `O(n)` via `HashSet` (`RV-50`).
    #[must_use]
    pub fn selected_graph_point_targets(&self, sheet: &DataSheet, max_series: usize) -> Vec<(usize, usize, Option<usize>)> {
        if !self.selected_coords.is_empty() {
            return self.discontiguous_graph_point_targets(sheet, max_series);
        }
        let mut out = self.rectangular_graph_point_targets(sheet, max_series);
        if out.is_empty() && !self.selected_rows.is_empty() {
            let row_cap = populated_row_bound_for_sheet(sheet);
            let mut seen = HashSet::with_capacity(self.selected_rows.len().min(MAX_MATERIALIZED_SELECTION_COORDS));
            for &row in &self.selected_rows {
                if row < row_cap && seen.insert(row) {
                    out.push((0, row, None));
                    if out.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                        break;
                    }
                }
            }
        }
        out
    }

    fn discontiguous_graph_point_targets(&self, sheet: &DataSheet, max_series: usize) -> Vec<(usize, usize, Option<usize>)> {
        let total_rows = sheet.archetype.row_count();
        let cap = self.selected_coords.len().min(MAX_MATERIALIZED_SELECTION_COORDS);
        let mut seen = HashSet::with_capacity(cap);
        let mut out = Vec::with_capacity(cap);
        for &c in &self.selected_coords {
            if c.row >= total_rows {
                continue;
            }
            let target = graph_target_for_coord(&sheet.archetype, c.col, c.row, c.subcol, max_series);
            if seen.insert(target) {
                out.push(target);
                if out.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                    break;
                }
            }
        }
        out
    }

    fn rectangular_graph_point_targets(&self, sheet: &DataSheet, max_series: usize) -> Vec<(usize, usize, Option<usize>)> {
        let Some((r0, r1, lo, hi)) = self.materialized_selection_flat_rect(sheet, true) else {
            return Vec::new();
        };
        let mut col_targets = Vec::with_capacity(hi - lo + 1);
        let mut seen_cols = HashSet::with_capacity(hi - lo + 1);
        for flat in lo..=hi {
            if let Some((col, subcol)) = resolve_flat_coord(&sheet.archetype, flat) {
                let series = if max_series == 0 { col } else { col.min(max_series - 1) };
                let sub_opt = (data_set_subcolumn_count(&sheet.archetype, col) > 1).then_some(subcol);
                if seen_cols.insert((series, sub_opt)) {
                    col_targets.push((series, sub_opt));
                }
            }
        }
        if col_targets.is_empty() {
            return Vec::new();
        }
        let max_rows = (MAX_MATERIALIZED_SELECTION_COORDS / col_targets.len()).max(1);
        let eff_r1 = r1.min(r0.saturating_add(max_rows - 1));
        let mut out = Vec::with_capacity((eff_r1 - r0 + 1).saturating_mul(col_targets.len()).min(MAX_MATERIALIZED_SELECTION_COORDS));
        for row in r0..=eff_r1 {
            for &(series, sub_opt) in &col_targets {
                if out.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                    return out;
                }
                out.push((series, row, sub_opt));
            }
        }
        out
    }

    /// Expand the current rectangular selection into concrete [`GridCellCoord`] coordinates
    /// intersected with populated sheet dimensions and capped safely (`RV-50`).
    #[must_use]
    pub fn selection_coords(&self, sheet: &DataSheet) -> Vec<GridCellCoord> {
        let Some((r0, r1, lo, hi)) = self.materialized_selection_flat_rect(sheet, false) else {
            return Vec::new();
        };
        let flat_coords: Vec<(usize, usize)> = (lo..=hi).filter_map(|flat| resolve_flat_coord(&sheet.archetype, flat)).collect();
        if flat_coords.is_empty() {
            return Vec::new();
        }
        let max_rows = (MAX_MATERIALIZED_SELECTION_COORDS / flat_coords.len()).max(1);
        let eff_r1 = r1.min(r0.saturating_add(max_rows - 1));
        let mut coords = Vec::with_capacity((eff_r1 - r0 + 1).saturating_mul(flat_coords.len()).min(MAX_MATERIALIZED_SELECTION_COORDS));
        for row in r0..=eff_r1 {
            for &(col, subcol) in &flat_coords {
                if coords.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                    return coords;
                }
                coords.push(GridCellCoord::new(row, col, subcol));
            }
        }
        coords
    }

    fn materialized_selection_flat_rect(&self, sheet: &DataSheet, require_populated_on_small: bool) -> Option<(usize, usize, usize, usize)> {
        let total_rows = sheet.archetype.row_count();
        let total_flat = total_flat_data_subcolumns(&sheet.archetype);
        if total_rows == 0 || total_flat == 0 {
            return None;
        }
        let (min_c, max_c) = self.tool.selection_bounds()?;
        if min_c.row >= total_rows {
            return None;
        }
        let r0 = min_c.row;
        let raw_r1 = max_c.row.min(total_rows - 1);
        let f0 = flat_index_for_coord(&sheet.archetype, min_c.col, min_c.subcol);
        let f1 = flat_index_for_coord(&sheet.archetype, max_c.col, max_c.subcol);
        let lo = f0.min(f1);
        if lo >= total_flat {
            return None;
        }
        let hi = f0.max(f1).min(total_flat - 1);

        if min_c == max_c && !require_populated_on_small {
            return Some((r0, r0, lo, hi));
        }

        let max_rows_for_cols = (MAX_MATERIALIZED_SELECTION_COORDS / (hi - lo + 1).max(1)).max(1);
        let window_r1 = raw_r1.min(r0.saturating_add(max_rows_for_cols - 1));
        if let Some(last_pop) = last_populated_row_in_range(sheet, r0, window_r1) {
            return Some((r0, last_pop, lo, hi));
        }
        let span_cells = (raw_r1 - r0 + 1).saturating_mul(hi - lo + 1);
        if !require_populated_on_small && span_cells <= MAX_MATERIALIZED_SELECTION_COORDS && total_rows <= MAX_MATERIALIZED_SELECTION_COORDS {
            return Some((r0, window_r1, lo, hi));
        }
        None
    }

    /// Recompute quick summary statistics (`n`, `excluded`, `mean`, `SD`, `SEM`, `median`, `sum`)
    /// for the active data sheet selection.
    pub fn refresh_selection_stats(&mut self, doc: &ProjectDocument) {
        let Some(active_id) = &doc.active_sheet_id else {
            self.quick_stats = None;
            self.extended_stats = None;
            return;
        };
        let Some(sheet) = doc.data_sheet(active_id) else {
            self.quick_stats = None;
            self.extended_stats = None;
            return;
        };
        self.refresh_selection_stats_for_sheet(sheet);
    }

    /// Visible row/column window for the currently selected [`DataSheet`].
    #[must_use]
    pub fn visible_window(&self, doc: &ProjectDocument) -> Option<GridVisibleWindow> {
        let active_id = doc.active_sheet_id.as_ref()?;
        let sheet = doc.data_sheet(active_id)?;
        let total_rows = sheet.archetype.row_count();
        let total_flat_cols = total_flat_data_subcolumns(&sheet.archetype);
        Some(self.tool.visible_window(total_rows, total_flat_cols))
    }

    /// Materialize the two-tier `(Group x Subcolumn)` visible frame strictly for `[start_row..end_row)`
    /// and `[start_col..end_col)` in `O(visible_rows * visible_subcols)` time.
    #[must_use]
    pub fn visible_frame(&self, doc: &ProjectDocument) -> Option<GridVisibleFrame> {
        let active_id = doc.active_sheet_id.as_ref()?;
        let sheet = doc.data_sheet(active_id)?;
        Some(self.visible_frame_for_sheet(sheet))
    }

    /// Materialize the two-tier `(Group x Subcolumn)` visible frame for an explicit [`DataSheet`].
    #[must_use]
    pub fn visible_frame_for_sheet(&self, sheet: &DataSheet) -> GridVisibleFrame {
        let total_rows = sheet.archetype.row_count();
        let total_flat_cols = total_flat_data_subcolumns(&sheet.archetype);
        let window = self.tool.visible_window(total_rows, total_flat_cols);

        let decimal_sep = if self.decimal_separator == ',' { ',' } else { '.' };
        let (tier1_groups, tier2_subcolumns) = build_visible_headers(&sheet.archetype, window.start_col, window.end_col);

        let sel_bounds = selection_flat_bounds(&sheet.archetype, self.tool.anchor, self.tool.active);
        let selected_set: std::collections::HashSet<GridCellCoord> =
            if self.selected_coords.len() > 1 || (!self.selected_coords.is_empty() && self.tool.anchor == self.tool.active) {
                self.selected_coords.iter().copied().collect()
            } else {
                std::collections::HashSet::new()
            };
        let fmt_ctx = CellFormatContext { decimal_sep, sel_bounds, selected_set: &selected_set, active_coord: self.tool.active };
        let mut rows = Vec::with_capacity(window.end_row.saturating_sub(window.start_row));
        for r in window.start_row..window.end_row {
            let leading = build_leading_row(sheet, r, decimal_sep);
            let mut cells = Vec::with_capacity(tier2_subcolumns.len());
            for hdr in &tier2_subcolumns {
                let coord = GridCellCoord::new(r, hdr.col, hdr.subcol);
                let cell = sheet.cell(CellCoord::from(coord)).cloned().unwrap_or_default();
                let formatted = format_grid_cell(sheet, coord, hdr, &cell, &fmt_ctx);
                cells.push(formatted);
            }
            rows.push(VisibleGridRow { row: r, leading, cells });
        }

        GridVisibleFrame {
            sheet_id: sheet.id.clone(),
            kind: sheet.kind(),
            frozen: sheet.frozen,
            row_titles_expanded: effective_row_titles_expanded(sheet, self.row_titles_collapsed_override),
            x_column_title: x_or_time_header_title(&sheet.archetype),
            total_rows,
            total_flat_cols,
            window,
            tier1_groups,
            tier2_subcolumns,
            rows,
        }
    }

    /// Non-destructively toggle the `Row Titles` column expansion state without mutating `DataSheet`.
    pub fn toggle_row_titles(&mut self, sheet: &DataSheet) {
        let current = effective_row_titles_expanded(sheet, self.row_titles_collapsed_override);
        self.row_titles_collapsed_override = Some(!current);
    }

    /// Inspect any [`LinkedDataRegion`] containing `coord` (for double-click `Format Link`).
    pub fn inspect_link_at(&mut self, sheet: &DataSheet, coord: GridCellCoord) -> bool {
        if let Some(region) = find_linked_region_at(sheet, coord) {
            self.format_link_inspector = FormatLinkInspectorState {
                open: true,
                region_id: region.id.clone(),
                border_color: region.border_color(),
                provenance_summary: region.provenance_summary.clone(),
                spec: Some(region.spec.clone()),
            };
            true
        } else {
            self.format_link_inspector.open = false;
            false
        }
    }

    /// Boundary-aware arrow navigation (`Shift+Cmd/Ctrl+Arrow` stops cleanly at the first empty
    /// cell boundary). Updates `tool.anchor` and `tool.active`.
    pub fn navigate(&mut self, sheet: &DataSheet, dir: MoveDirection, extend_selection: bool, jump_to_boundary: bool) {
        let total_rows = sheet.archetype.row_count();
        let total_flat = total_flat_data_subcolumns(&sheet.archetype);
        if total_rows == 0 || total_flat == 0 {
            return;
        }
        let current = self.tool.active.unwrap_or_default();
        let curr_flat = flat_index_for_coord(&sheet.archetype, current.col, current.subcol).min(total_flat - 1);
        let curr_row = current.row.min(total_rows - 1);

        let (next_row, next_flat) = if jump_to_boundary {
            jump_boundary_step(sheet, curr_row, curr_flat, total_rows, total_flat, dir)
        } else {
            single_step(curr_row, curr_flat, total_rows, total_flat, dir)
        };

        let Some((col, subcol)) = resolve_flat_coord(&sheet.archetype, next_flat) else {
            return;
        };
        let next = GridCellCoord::new(next_row, col, subcol);
        if extend_selection {
            if self.tool.anchor.is_none() {
                self.tool.anchor = Some(GridCellCoord::new(curr_row, current.col, current.subcol));
            }
            self.tool.active = Some(next);
        } else {
            self.tool.select_single(next);
        }
    }

    /// Emit batched rectangular `"table.excludeCells"` range intents (`Cmd+E` / `Ctrl+E`) toggling
    /// exclusion across the current selection (`docs/implementation/data-tables-and-transforms.md` §1.2).
    /// Returns the number of `"range"` command intents queued into `tool.pending_intents`.
    pub fn emit_toggle_exclude_intents(&mut self, sheet: &DataSheet, excluded: bool) -> usize {
        let total_rows = sheet.archetype.row_count();
        let total_flat = total_flat_data_subcolumns(&sheet.archetype);
        if total_rows == 0 || total_flat == 0 {
            return 0;
        }
        let ranges = if !self.selected_coords.is_empty() {
            let mut coords: Vec<(usize, usize, usize)> =
                self.all_selected_coords(sheet).into_iter().filter(|c| c.row < total_rows).map(|c| (c.col, c.subcol, c.row)).collect();
            coords.sort_unstable();
            coords.dedup();
            let mut out = Vec::new();
            let mut i = 0;
            while i < coords.len() {
                let (col, subcol, start_row) = coords[i];
                let mut end_row = start_row;
                while i + 1 < coords.len() && coords[i + 1].0 == col && coords[i + 1].1 == subcol && coords[i + 1].2 == end_row + 1 {
                    i += 1;
                    end_row = coords[i].2;
                }
                out.push(SheetCellRange::new(CellCoord::new(start_row, col, subcol), CellCoord::new(end_row, col, subcol)));
                i += 1;
            }
            out
        } else {
            let (Some(a), Some(b)) = (self.tool.anchor, self.tool.active) else {
                return 0;
            };
            let r0 = a.row.min(b.row).min(total_rows - 1);
            let r1 = a.row.max(b.row).min(total_rows - 1);
            let f0 = flat_index_for_coord(&sheet.archetype, a.col, a.subcol);
            let f1 = flat_index_for_coord(&sheet.archetype, b.col, b.subcol);
            let c0 = f0.min(f1).min(total_flat - 1);
            let c1 = f0.max(f1).min(total_flat - 1);
            build_exclusion_ranges(&sheet.archetype, r0, r1, c0, c1)
        };
        let emitted = ranges.len();
        for range in ranges {
            self.tool.pending_intents.push(ToolCommandIntent {
                command_id: String::from("table.excludeCells"),
                params: json!({
                    "sheet_id": sheet.id.as_str(),
                    "range": range,
                    "excluded": excluded,
                }),
            });
        }
        emitted
    }

    /// Build the `MultipleVariables` role/type column header strip across all variables in `sheet`.
    #[must_use]
    pub fn variable_role_strip(&self, sheet: &DataSheet) -> Vec<VariableRoleHeader> {
        let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
            return Vec::new();
        };
        mv.variables.iter().enumerate().map(|(idx, var)| build_variable_role_header(idx, var)).collect()
    }
}

/// Map a [`LinkBorderColor`] to its canonical UI border RGB color (`docs/implementation/data-tables-and-transforms.md` §6.2).
#[must_use]
pub const fn provenance_border_rgba(color: LinkBorderColor) -> Rgba8 {
    match color {
        LinkBorderColor::Blue => Rgba8::rgb(59, 130, 246),
        LinkBorderColor::Green => Rgba8::rgb(22, 163, 74),
        LinkBorderColor::Red => Rgba8::rgb(220, 38, 38),
        LinkBorderColor::Charcoal => Rgba8::rgb(55, 65, 81),
    }
}

// ============================================================================
// Virtualized Flat-Subcolumn Mapping Helpers (O(1) for uniform subcolumns)
// ============================================================================

#[must_use]
pub fn total_flat_data_subcolumns(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.len().saturating_mul(t.y_subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Column(t) => t.data_sets.len().saturating_mul(t.subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Grouped(t) => t.data_sets.len().saturating_mul(t.subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Contingency(t) => t.data_sets.len(),
        TableArchetype::Survival(t) => t.cohorts.len(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.len(),
        TableArchetype::MultipleVariables(t) => t.variables.len(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.biological_unit_count().max(1)).sum(),
    }
}

#[must_use]
pub(crate) fn data_set_subcolumn_count(arch: &TableArchetype, col: usize) -> usize {
    if let Some(subs) = uniform_subcolumn_count(arch) {
        return subs;
    }
    if let TableArchetype::Nested(t) = arch {
        return t.groups.get(col).map_or(1, |g| g.biological_unit_count().max(1));
    }
    1
}

#[must_use]
fn uniform_subcolumn_count(arch: &TableArchetype) -> Option<usize> {
    match arch {
        TableArchetype::Xy(t) => Some(t.y_subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Column(t) => Some(t.subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Grouped(t) => Some(t.subcolumn_mode.subcolumn_count().max(1)),
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => {
            Some(1)
        }
        TableArchetype::Nested(_) => None,
    }
}

#[must_use]
fn flat_index_for_coord(arch: &TableArchetype, col: usize, subcol: usize) -> usize {
    if let Some(subs) = uniform_subcolumn_count(arch) {
        return col.saturating_mul(subs).saturating_add(subcol.min(subs - 1));
    }
    if let TableArchetype::Nested(t) = arch {
        let mut offset = 0usize;
        for (idx, grp) in t.groups.iter().enumerate() {
            let units = grp.biological_unit_count().max(1);
            if idx == col {
                return offset.saturating_add(subcol.min(units - 1));
            }
            offset = offset.saturating_add(units);
        }
        return offset;
    }
    col
}

#[must_use]
pub(crate) fn resolve_flat_coord(arch: &TableArchetype, flat_col: usize) -> Option<(usize, usize)> {
    if let Some(subs) = uniform_subcolumn_count(arch) {
        let col = flat_col / subs;
        if col >= arch.column_count() {
            return None;
        }
        return Some((col, flat_col % subs));
    }
    if let TableArchetype::Nested(t) = arch {
        let mut offset = 0usize;
        for (col_idx, grp) in t.groups.iter().enumerate() {
            let units = grp.biological_unit_count().max(1);
            if flat_col < offset + units {
                return Some((col_idx, flat_col - offset));
            }
            offset += units;
        }
    }
    None
}

#[must_use]
fn selection_cell_count(sheet: &DataSheet, a: GridCellCoord, b: GridCellCoord) -> usize {
    let row_count = sheet.archetype.row_count();
    let total_flat = total_flat_data_subcolumns(&sheet.archetype);
    if row_count == 0 || total_flat == 0 {
        return 0;
    }
    let r0 = a.row.min(b.row);
    if r0 >= row_count {
        return 0;
    }
    let r1 = a.row.max(b.row).min(row_count - 1);

    let f0 = flat_index_for_coord(&sheet.archetype, a.col, a.subcol);
    let f1 = flat_index_for_coord(&sheet.archetype, b.col, b.subcol);
    let start_flat = f0.min(f1);
    if start_flat >= total_flat {
        return 0;
    }
    let end_flat = f0.max(f1).min(total_flat - 1);

    r1.saturating_sub(r0).saturating_add(1).saturating_mul(end_flat.saturating_sub(start_flat).saturating_add(1))
}

fn graph_target_for_coord(arch: &TableArchetype, col: usize, row: usize, subcol: usize, max_series: usize) -> (usize, usize, Option<usize>) {
    let series = if max_series == 0 { col } else { col.min(max_series - 1) };
    let sub_opt = (data_set_subcolumn_count(arch, col) > 1).then_some(subcol);
    (series, row, sub_opt)
}

fn append_row_coords_bounded(arch: &TableArchetype, row: usize, col_count: usize, coord_set: &mut std::collections::BTreeSet<GridCellCoord>) {
    for col in 0..col_count {
        let subs = data_set_subcolumn_count(arch, col);
        for subcol in 0..subs {
            if coord_set.len() >= MAX_MATERIALIZED_SELECTION_COORDS {
                return;
            }
            coord_set.insert(GridCellCoord::new(row, col, subcol));
        }
    }
}

fn bounded_row_range_vec(sheet: &DataSheet, r0: usize, r1: usize) -> Vec<usize> {
    let total_rows = sheet.archetype.row_count();
    if total_rows == 0 || r0 > r1 || r0 >= total_rows {
        return Vec::new();
    }
    let clamped_r1 = r1.min(total_rows - 1);
    if clamped_r1 - r0 < MAX_MATERIALIZED_SELECTION_COORDS && total_rows <= MAX_MATERIALIZED_SELECTION_COORDS {
        let end = last_populated_row_in_range(sheet, r0, clamped_r1).unwrap_or(clamped_r1);
        return (r0..=end).collect();
    }
    let window_r1 = clamped_r1.min(r0.saturating_add(MAX_MATERIALIZED_SELECTION_COORDS - 1));
    match last_populated_row_in_range(sheet, r0, window_r1) {
        Some(last_pop) => (r0..=last_pop).collect(),
        None if r0 == clamped_r1 => vec![r0],
        None => Vec::new(),
    }
}

fn populated_row_bound_for_sheet(sheet: &DataSheet) -> usize {
    let total_rows = sheet.archetype.row_count();
    if total_rows == 0 {
        return 0;
    }
    let window_end = (total_rows - 1).min(MAX_MATERIALIZED_SELECTION_COORDS - 1);
    if let Some(last_pop) = last_populated_row_in_range(sheet, 0, window_end) {
        if window_end == total_rows - 1 || last_pop < window_end {
            return last_pop + 1;
        }
        return total_rows;
    }
    0
}

fn is_cell_populated(cell: &Cell) -> bool {
    !cell.is_empty() || cell.excluded
}

fn is_subrow_populated(subs: &[Cell]) -> bool {
    subs.iter().any(is_cell_populated)
}

fn rfind_in_window<T, F>(slice: &[T], r0: usize, r1: usize, best: Option<usize>, is_pop: F) -> Option<usize>
where
    F: Fn(&T) -> bool,
{
    if r0 > r1 || r0 >= slice.len() {
        return best;
    }
    let hi = r1.min(slice.len() - 1);
    let lo = best.map_or(r0, |b| (b + 1).max(r0));
    if lo > hi {
        return best;
    }
    slice[lo..=hi].iter().rposition(is_pop).map_or(best, |rel| Some(lo + rel))
}

fn last_populated_row_in_range(sheet: &DataSheet, r0: usize, r1: usize) -> Option<usize> {
    let mut best = None;
    match &sheet.archetype {
        TableArchetype::Xy(t) => {
            if matches!(t.x_mode, XColumnMode::GeneratedSequence { .. }) {
                return (r0 <= r1 && r0 < t.row_count()).then(|| r1.min(t.row_count() - 1));
            }
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            best = rfind_in_window(&t.x_cells, r0, r1, best, |subs| is_subrow_populated(subs));
            for col in &t.data_sets {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::Column(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            for col in &t.data_sets {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::Grouped(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            for col in &t.data_sets {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::Contingency(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            for col in &t.data_sets {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::Survival(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            best = rfind_in_window(&t.times, r0, r1, best, is_cell_populated);
            for col in &t.cohorts {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            best = rfind_in_window(&t.slice_labels, r0, r1, best, |s| !s.is_empty());
            for col in &t.data_sets {
                best = rfind_in_window(&col.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
        TableArchetype::MultipleVariables(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            for var in &t.variables {
                best = rfind_in_window(&var.cells, r0, r1, best, is_cell_populated);
            }
        }
        TableArchetype::Nested(t) => {
            best = rfind_in_window(&t.row_titles, r0, r1, best, |s| !s.is_empty());
            for grp in &t.groups {
                best = rfind_in_window(&grp.cells, r0, r1, best, |subs| is_subrow_populated(subs));
            }
        }
    }
    best
}

fn build_exclusion_ranges(arch: &TableArchetype, r0: usize, r1: usize, c0: usize, c1: usize) -> Vec<SheetCellRange> {
    if let Some(subs) = uniform_subcolumn_count(arch) {
        let col0 = c0 / subs;
        let sub0 = c0 % subs;
        let col1 = c1 / subs;
        let sub1 = c1 % subs;
        if col0 == col1 {
            return vec![SheetCellRange::new(CellCoord::new(r0, col0, sub0), CellCoord::new(r1, col0, sub1))];
        }
        let mut ranges = Vec::with_capacity(3);
        let mid_start = if sub0 > 0 {
            ranges.push(SheetCellRange::new(CellCoord::new(r0, col0, sub0), CellCoord::new(r1, col0, subs - 1)));
            col0 + 1
        } else {
            col0
        };
        let has_trailing = sub1 + 1 < subs;
        if let Some(mid_end) = if has_trailing { col1.checked_sub(1) } else { Some(col1) }
            && mid_start <= mid_end
        {
            ranges.push(SheetCellRange::new(CellCoord::new(r0, mid_start, 0), CellCoord::new(r1, mid_end, subs - 1)));
        }
        if has_trailing {
            ranges.push(SheetCellRange::new(CellCoord::new(r0, col1, 0), CellCoord::new(r1, col1, sub1)));
        }
        return ranges;
    }

    let mut ranges = Vec::new();
    if let TableArchetype::Nested(t) = arch {
        let mut offset = 0usize;
        for (col_idx, grp) in t.groups.iter().enumerate() {
            let units = grp.biological_unit_count().max(1);
            let lo = c0.max(offset);
            let hi = c1.min(offset + units - 1);
            if lo <= hi {
                ranges.push(SheetCellRange::new(CellCoord::new(r0, col_idx, lo - offset), CellCoord::new(r1, col_idx, hi - offset)));
            }
            offset += units;
        }
    }
    ranges
}

impl GridViewState {
    /// Synchronize active decimal separators across the grid view, underlying tool, and import wizards.
    pub fn apply_locale_preset(&mut self, preset: &crate::i18n::LocalePreset) {
        self.decimal_separator = preset.decimal_separator;
        self.tool.decimal_separator = preset.decimal_separator;
        self.import_wizard.decimal_separator = preset.decimal_separator;
        self.import_reshape.decimal_separator = preset.decimal_separator;
    }
}

fn build_visible_headers(arch: &TableArchetype, start_flat: usize, end_flat: usize) -> (Vec<Tier1GroupHeader>, Vec<Tier2SubcolumnHeader>) {
    if start_flat >= end_flat {
        return (Vec::new(), Vec::new());
    }
    let mut tier1: Vec<Tier1GroupHeader> = Vec::new();
    let mut tier2 = Vec::with_capacity(end_flat - start_flat);

    let sub_labels_cache: Option<Vec<String>> = match arch {
        TableArchetype::Xy(t) => Some(t.y_subcolumn_mode.subcolumn_labels()),
        TableArchetype::Column(t) => Some(t.subcolumn_mode.subcolumn_labels()),
        TableArchetype::Grouped(t) => Some(t.subcolumn_mode.subcolumn_labels()),
        _ => None,
    };

    for flat in start_flat..end_flat {
        let Some((col, subcol)) = resolve_flat_coord(arch, flat) else {
            break;
        };
        let is_calc = matches!(arch, TableArchetype::MultipleVariables(mv) if mv.variables.get(col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. })));
        let sub_label = subcolumn_header_label(arch, col, subcol, sub_labels_cache.as_deref());
        tier2.push(Tier2SubcolumnHeader { flat_col: flat, col, subcol, label: sub_label, is_calculated: is_calc });

        if let Some(last) = tier1.last_mut()
            && last.col_index == col
        {
            last.span += 1;
        } else {
            let (title, var_role) = group_header_meta(arch, col);
            tier1.push(Tier1GroupHeader {
                col_index: col,
                letter: default_column_label(col),
                title,
                start_flat_col: flat,
                span: 1,
                variable_role: var_role,
            });
        }
    }

    (tier1, tier2)
}

fn group_header_meta(arch: &TableArchetype, col: usize) -> (String, Option<VariableRoleHeader>) {
    if let TableArchetype::MultipleVariables(mv) = arch {
        match mv.variables.get(col) {
            Some(var) => (var.title.clone(), Some(build_variable_role_header(col, var))),
            None => (default_column_label(col), None),
        }
    } else {
        archetype_column_title(arch, col)
    }
}

fn archetype_column_title(arch: &TableArchetype, col: usize) -> (String, Option<VariableRoleHeader>) {
    let title = match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Column(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map(|d| d.title.clone()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map(|v| v.title.clone()),
        TableArchetype::Nested(t) => t.groups.get(col).map(|g| g.title.clone()),
    };
    (title.unwrap_or_else(|| default_column_label(col)), None)
}

/// Return Tier-2 subcolumn header labels for group/data-set column `col` (`A:1..A:k` for `Nested`,
/// `Y1..Yn` / `Mean, SD, N` for `Xy`/`Column`/`Grouped`, or single role/kind headers).
#[must_use]
pub fn subcolumn_header_labels(arch: &TableArchetype, col: usize) -> Vec<String> {
    match arch {
        TableArchetype::Xy(t) if col < t.data_sets.len() => t.y_subcolumn_mode.subcolumn_labels(),
        TableArchetype::Column(t) if col < t.data_sets.len() => t.subcolumn_mode.subcolumn_labels(),
        TableArchetype::Grouped(t) if col < t.data_sets.len() => t.subcolumn_mode.subcolumn_labels(),
        TableArchetype::Nested(t) => t
            .groups
            .get(col)
            .map(|g| {
                let units = g.biological_unit_count().max(1);
                (0..units)
                    .map(|subcol| g.subcolumn_titles.get(subcol).cloned().unwrap_or_else(|| format!("{}:{}", default_column_label(col), subcol + 1)))
                    .collect()
            })
            .unwrap_or_default(),
        TableArchetype::MultipleVariables(mv) => mv.variables.get(col).map(|v| vec![role_short_label(v.role, v.role_inferred)]).unwrap_or_default(),
        TableArchetype::Contingency(t) if col < t.data_sets.len() => vec![String::from("Count")],
        TableArchetype::Survival(t) if col < t.cohorts.len() => vec![String::from("Status")],
        TableArchetype::PartsOfWhole(t) if col < t.data_sets.len() => vec![String::from("Value")],
        _ => Vec::new(),
    }
}

/// Return a compact group-level badge summarizing biological units (`Nested`) or replicate
/// subcolumn modes (`Xy`, `Column`, `Grouped`) for column `col`, or `None` for single columns.
#[must_use]
pub fn biological_unit_label(arch: &TableArchetype, col: usize) -> Option<String> {
    match arch {
        TableArchetype::Nested(t) => t.groups.get(col).map(|grp| {
            let count = grp.biological_unit_count();
            format!("{count} bio unit{}", if count == 1 { "" } else { "s" })
        }),
        TableArchetype::Xy(t) => t.data_sets.get(col).and_then(|_| replicate_mode_badge(t.y_subcolumn_mode)),
        TableArchetype::Column(t) => t.data_sets.get(col).and_then(|_| replicate_mode_badge(t.subcolumn_mode)),
        TableArchetype::Grouped(t) => t.data_sets.get(col).and_then(|_| replicate_mode_badge(t.subcolumn_mode)),
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => None,
    }
}

fn replicate_mode_badge(mode: SubcolumnMode) -> Option<String> {
    match mode {
        SubcolumnMode::Single => None,
        SubcolumnMode::RawReplicates(n) => Some(format!("{n} replicate{}", if n == 1 { "" } else { "s" })),
        SubcolumnMode::MeanSdN => Some(String::from("Mean / SD / N")),
        SubcolumnMode::MeanSemN => Some(String::from("Mean / SEM / N")),
        SubcolumnMode::MeanCvN => Some(String::from("Mean / %CV / N")),
        SubcolumnMode::MeanLowHigh => Some(String::from("Mean / -Err / +Err")),
        SubcolumnMode::UpperLowerLimits => Some(String::from("Mean / Lower / Upper")),
        SubcolumnMode::MedianIqr => Some(String::from("Median / Q1 / Q3")),
    }
}

fn subcolumn_header_label(arch: &TableArchetype, col: usize, subcol: usize, cached: Option<&[String]>) -> String {
    if let Some(labels) = cached {
        return labels.get(subcol).cloned().unwrap_or_else(|| format!("Y{}", subcol + 1));
    }
    subcolumn_header_labels(arch, col).get(subcol).cloned().unwrap_or_else(|| String::from("Y"))
}

fn role_short_label(role: VariableRole, inferred: bool) -> String {
    let base = match role {
        VariableRole::Continuous => "Continuous",
        VariableRole::Categorical => "Categorical",
        VariableRole::Label => "Label",
        VariableRole::Calculated => "Calculated",
    };
    if inferred && role != VariableRole::Calculated { format!("{base} (auto)") } else { String::from(base) }
}

fn build_variable_role_header(col_index: usize, var: &curvebook_doc::VariableColumn) -> VariableRoleHeader {
    let is_calculated = matches!(var.source, VariableSource::Calculated { .. });
    let formula = match &var.source {
        VariableSource::Calculated { formula } => Some(formula.clone()),
        VariableSource::Entered => None,
    };
    let badge_color = match (var.role, var.role_inferred) {
        (VariableRole::Continuous, false) => Rgba8::rgb(29, 78, 216),
        (VariableRole::Continuous, true) => Rgba8::rgb(100, 116, 139),
        (VariableRole::Categorical, false) => Rgba8::rgb(124, 58, 237),
        (VariableRole::Categorical, true) => Rgba8::rgb(139, 92, 246),
        (VariableRole::Label, _) => Rgba8::rgb(71, 85, 105),
        (VariableRole::Calculated, _) => Rgba8::rgb(5, 150, 105),
    };
    VariableRoleHeader {
        col_index,
        title: var.title.clone(),
        role: var.role,
        role_inferred: var.role_inferred,
        is_calculated,
        formula,
        badge_text: role_short_label(var.role, var.role_inferred),
        badge_color,
    }
}

// ============================================================================
// Cell Formatting, Exclusions, Provenance Borders & Typed Validation
// ============================================================================

fn effective_row_titles_expanded(sheet: &DataSheet, override_flag: Option<bool>) -> bool {
    if let Some(val) = override_flag {
        return val;
    }
    match &sheet.archetype {
        TableArchetype::Xy(t) => t.row_titles_expanded,
        TableArchetype::Column(t) => t.row_titles_expanded,
        TableArchetype::Survival(t) => t.row_titles_expanded,
        TableArchetype::MultipleVariables(t) => t.row_titles_expanded,
        TableArchetype::Nested(t) => t.row_titles_expanded,
        TableArchetype::Grouped(_) | TableArchetype::Contingency(_) | TableArchetype::PartsOfWhole(_) => true,
    }
}

fn x_or_time_header_title(arch: &TableArchetype) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => Some(t.x_title.clone()),
        TableArchetype::Survival(t) => Some(t.time_title.clone()),
        TableArchetype::PartsOfWhole(_) => Some(String::from("Slice")),
        _ => None,
    }
}

fn build_leading_row(sheet: &DataSheet, row: usize, decimal_sep: char) -> LeadingRowData {
    let row_title = match &sheet.archetype {
        TableArchetype::Xy(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::Column(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::Grouped(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::Contingency(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::Survival(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::PartsOfWhole(t) => t.slice_labels.get(row).cloned().unwrap_or_default(),
        TableArchetype::MultipleVariables(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
        TableArchetype::Nested(t) => t.row_titles.get(row).cloned().unwrap_or_default(),
    };

    let (x_or_time_display, x_excluded) = match &sheet.archetype {
        TableArchetype::Xy(t) => match &t.x_mode {
            XColumnMode::GeneratedSequence { .. } => (t.x_value_at(row).map(|v| format_compact_f64(v, decimal_sep)), false),
            _ => match t.x_cells.get(row).and_then(|subs| subs.first()) {
                Some(cell) => (Some(format_cell_value(&cell.value, decimal_sep)), cell.excluded),
                None => (None, false),
            },
        },
        TableArchetype::Survival(t) => match t.times.get(row) {
            Some(cell) => (Some(format_cell_value(&cell.value, decimal_sep)), cell.excluded),
            None => (None, false),
        },
        TableArchetype::PartsOfWhole(t) => (t.slice_labels.get(row).cloned(), false),
        _ => (None, false),
    };

    LeadingRowData { row, row_title, x_or_time_display, x_excluded }
}

struct CellFormatContext<'a> {
    decimal_sep: char,
    sel_bounds: Option<(usize, usize, usize, usize)>,
    selected_set: &'a std::collections::HashSet<GridCellCoord>,
    active_coord: Option<GridCellCoord>,
}

fn format_grid_cell(
    sheet: &DataSheet,
    coord: GridCellCoord,
    hdr: &Tier2SubcolumnHeader,
    cell: &Cell,
    ctx: &CellFormatContext<'_>,
) -> FormattedGridCell {
    let display_text = format_cell_value(&cell.value, ctx.decimal_sep);
    let region = find_linked_region_at(sheet, coord);
    let provenance_border = region.and_then(LinkedDataRegion::border_color);
    let provenance_border_rgba = provenance_border.map(provenance_border_rgba);
    let link_id = region.map(|r| r.id.clone());

    let is_selected = if !ctx.selected_set.is_empty() {
        ctx.selected_set.contains(&coord)
    } else {
        ctx.sel_bounds.is_some_and(|(r0, r1, f0, f1)| coord.row >= r0 && coord.row <= r1 && hdr.flat_col >= f0 && hdr.flat_col <= f1)
    };
    let is_active = ctx.active_coord == Some(coord);
    let validation_error = validate_cell_for_archetype(sheet, coord, cell);

    let text_color = if cell.excluded { Rgba8::rgb(37, 99, 235) } else { Rgba8::rgb(24, 28, 36) };
    let visual = CellVisualStyle {
        text_color,
        italic: cell.excluded,
        strikethrough: cell.excluded,
        excluded_badge: cell.excluded.then(|| String::from("*")),
        provenance_border,
        provenance_border_rgba,
        link_id,
        validation_error,
        is_selected,
        is_active,
        is_calculated: hdr.is_calculated,
    };

    FormattedGridCell { coord, flat_col: hdr.flat_col, display_text, excluded: cell.excluded, visual }
}

fn format_cell_value(val: &CellValue, decimal_sep: char) -> String {
    match val {
        CellValue::Empty => String::new(),
        CellValue::Number(v) if v.is_finite() => format_compact_f64(*v, decimal_sep),
        CellValue::Number(_) => String::new(),
        CellValue::Text(s) => s.clone(),
    }
}

fn format_compact_f64(value: f64, decimal_sep: char) -> String {
    let normalized = if value == 0.0 { 0.0 } else { value };
    let formatted = format!("{normalized}");
    if decimal_sep == ',' { formatted.replace('.', ",") } else { formatted }
}

/// Format a grid [`Cell`] for display under `preset`, applying regional decimal separators,
/// typography minus signs, and trailing `*` exclusion markers while keeping formulas/text intact.
#[must_use]
pub fn format_cell_display(cell: &Cell, preset: impl Borrow<LocalePreset>) -> String {
    let preset = preset.borrow();
    let base = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) if v.is_finite() => format_localized_f64(*v, preset),
        CellValue::Number(_) => String::new(),
        CellValue::Text(s) => s.clone(),
    };
    if cell.excluded && !base.is_empty() { format!("{base}*") } else { base }
}

/// Parse a user-entered localized numeric string into a finite `f64`.
///
/// Formulas starting with `=` return `None` so callers preserve formula expressions verbatim
/// without corrupting commas or numbers inside formulas.
#[must_use]
pub fn parse_localized_number_input(raw: &str, preset: impl Borrow<LocalePreset>) -> Option<f64> {
    let preset = preset.borrow();
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('=') {
        return None;
    }

    let (neg, body) = split_leading_sign(trimmed, &preset.minus_sign);
    if body.is_empty() {
        return None;
    }

    let (mantissa, exp_suffix) = match body.find(['e', 'E']) {
        Some(idx) => {
            let (m, exp) = body.split_at(idx);
            let normalized_exp = normalize_exponent_suffix(&exp[1..], &preset.minus_sign)?;
            (m, Some(normalized_exp))
        }
        None => (body, None),
    };

    let ascii_mantissa = normalize_mantissa(mantissa, preset)?;
    let mut normalized = String::new();
    if neg {
        normalized.push('-');
    }
    normalized.push_str(&ascii_mantissa);
    if let Some(exp) = exp_suffix {
        normalized.push('e');
        normalized.push_str(&exp);
    }

    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn format_localized_f64(value: f64, preset: &LocalePreset) -> String {
    let normalized = if value == 0.0 { 0.0 } else { value };
    let mut text = format!("{normalized}");
    if let Some(rest) = text.strip_prefix('-') {
        text = format!("{}{rest}", preset.minus_sign);
    }
    if preset.decimal_separator != '.' {
        text = text.replace('.', &preset.decimal_separator.to_string());
    }
    text
}

fn split_leading_sign<'a>(s: &'a str, preset_minus: &str) -> (bool, &'a str) {
    if !preset_minus.is_empty()
        && let Some(rest) = s.strip_prefix(preset_minus)
    {
        return (true, rest);
    }
    if let Some(rest) = s.strip_prefix('\u{2212}') {
        return (true, rest);
    }
    if let Some(rest) = s.strip_prefix('-') {
        return (true, rest);
    }
    if let Some(rest) = s.strip_prefix('+') {
        return (false, rest);
    }
    (false, s)
}

fn normalize_exponent_suffix(exp_body: &str, preset_minus: &str) -> Option<String> {
    if exp_body.is_empty() {
        return None;
    }
    let (exp_neg, digits) = split_leading_sign(exp_body, preset_minus);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(if exp_neg { format!("-{digits}") } else { digits.to_owned() })
}

fn normalize_mantissa(mantissa: &str, preset: &LocalePreset) -> Option<String> {
    let dec = preset.decimal_separator;
    let dec_count = mantissa.chars().filter(|&c| c == dec).count();
    if dec_count > 1 {
        return None;
    }

    if dec_count == 1 {
        let (int_raw, frac_raw) = mantissa.split_once(dec)?;
        if (int_raw.is_empty() && frac_raw.is_empty()) || !frac_raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let clean_int = if int_raw.is_empty() {
            String::new()
        } else {
            let mut seps = Vec::with_capacity(5);
            if let Some(t) = preset.thousands_separator
                && t != dec
            {
                seps.push(t);
            }
            for fallback in [' ', '\u{00a0}', '\u{202f}', if dec == ',' { '.' } else { ',' }] {
                if fallback != dec && !seps.contains(&fallback) {
                    seps.push(fallback);
                }
            }
            strip_thousands_grouping(int_raw, &seps)?
        };
        return Some(format!("{clean_int}.{frac_raw}"));
    }

    if mantissa.bytes().all(|b| b.is_ascii_digit()) {
        return (!mantissa.is_empty()).then(|| mantissa.to_owned());
    }

    let mut plain_seps = Vec::with_capacity(4);
    if let Some(t) = preset.thousands_separator
        && t != dec
    {
        plain_seps.push(t);
    }
    for space in [' ', '\u{00a0}', '\u{202f}'] {
        if !plain_seps.contains(&space) {
            plain_seps.push(space);
        }
    }
    if let Some(clean_int) = strip_thousands_grouping(mantissa, &plain_seps) {
        return Some(clean_int);
    }

    if dec == ',' && preset.thousands_separator != Some('.') && mantissa.chars().filter(|&c| c == '.').count() == 1 {
        let (int_raw, frac_raw) = mantissa.split_once('.')?;
        let valid_parts = !(int_raw.is_empty() && frac_raw.is_empty())
            && int_raw.bytes().all(|b| b.is_ascii_digit())
            && frac_raw.bytes().all(|b| b.is_ascii_digit());
        if valid_parts {
            return Some(mantissa.to_owned());
        }
    }

    None
}

fn strip_thousands_grouping(int_part: &str, allowed_seps: &[char]) -> Option<String> {
    if int_part.is_empty() {
        return None;
    }
    if int_part.bytes().all(|b| b.is_ascii_digit()) {
        return Some(int_part.to_owned());
    }
    for &sep in allowed_seps {
        if !int_part.contains(sep) {
            continue;
        }
        let mut parts = int_part.split(sep);
        let first = parts.next()?;
        if first.is_empty() || first.len() > 3 || !first.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut out = String::from(first);
        let mut group_count = 0usize;
        for part in parts {
            if part.len() != 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(part);
            group_count += 1;
        }
        if group_count > 0 {
            return Some(out);
        }
    }
    None
}

fn validate_cell_for_archetype(sheet: &DataSheet, coord: GridCellCoord, cell: &Cell) -> Option<String> {
    if cell.excluded || cell.is_empty() {
        return None;
    }
    match &sheet.archetype {
        TableArchetype::Contingency(cont) => {
            if let Err(err) = cont.count_at(coord.row, coord.col) {
                return Some(err.to_string());
            }
        }
        TableArchetype::Survival(surv) => {
            let Some(v) = cell.value.finite_f64() else {
                return Some(String::from("survival status must be numeric (event/censored code)"));
            };
            if v.fract() != 0.0 || (v as i64 != surv.event_code && v as i64 != surv.censored_code) {
                return Some(format!("status code must be {} or {}", surv.event_code, surv.censored_code));
            }
        }
        TableArchetype::PartsOfWhole(_) => {
            let Some(v) = cell.value.finite_f64() else {
                return Some(String::from("slice value must be numeric"));
            };
            if v < 0.0 {
                return Some(String::from("slice value must be >= 0"));
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            if let Some(var) = mv.variables.get(coord.col)
                && matches!(var.role, VariableRole::Continuous | VariableRole::Calculated)
                && matches!(cell.value, CellValue::Text(_))
            {
                return Some(String::from("continuous variable expects numeric value"));
            }
        }
        TableArchetype::Xy(_) | TableArchetype::Column(_) | TableArchetype::Grouped(_) | TableArchetype::Nested(_) => {
            if matches!(cell.value, CellValue::Text(_)) {
                return Some(String::from("numeric observation expected"));
            }
        }
    }
    None
}

fn selection_flat_bounds(
    arch: &TableArchetype,
    anchor: Option<GridCellCoord>,
    active: Option<GridCellCoord>,
) -> Option<(usize, usize, usize, usize)> {
    let (a, b) = (anchor?, active?);
    let fa = flat_index_for_coord(arch, a.col, a.subcol);
    let fb = flat_index_for_coord(arch, b.col, b.subcol);
    Some((a.row.min(b.row), a.row.max(b.row), fa.min(fb), fa.max(fb)))
}

fn find_linked_region_at(sheet: &DataSheet, coord: GridCellCoord) -> Option<&LinkedDataRegion> {
    sheet.linked_regions.iter().rev().find(|region| range_contains(&region.target_range, coord))
}

fn range_contains(range: &SheetCellRange, coord: GridCellCoord) -> bool {
    if coord.row < range.start.row || coord.row > range.end.row {
        return false;
    }
    if coord.col < range.start.col || coord.col > range.end.col {
        return false;
    }
    if coord.col == range.start.col && coord.subcol < range.start.subcol {
        return false;
    }
    if coord.col == range.end.col && coord.subcol > range.end.subcol {
        return false;
    }
    true
}

// ============================================================================
// Boundary-Aware Range Navigation Helpers
// ============================================================================

fn single_step(row: usize, flat_col: usize, total_rows: usize, total_flat: usize, dir: MoveDirection) -> (usize, usize) {
    match dir {
        MoveDirection::Up => (row.saturating_sub(1), flat_col),
        MoveDirection::Down => ((row + 1).min(total_rows - 1), flat_col),
        MoveDirection::Left => (row, flat_col.saturating_sub(1)),
        MoveDirection::Right => (row, (flat_col + 1).min(total_flat - 1)),
    }
}

fn jump_boundary_step(sheet: &DataSheet, row: usize, flat_col: usize, total_rows: usize, total_flat: usize, dir: MoveDirection) -> (usize, usize) {
    let step = |r: usize, f: usize| -> Option<(usize, usize)> {
        match dir {
            MoveDirection::Up => (r > 0).then(|| (r - 1, f)),
            MoveDirection::Down => (r + 1 < total_rows).then(|| (r + 1, f)),
            MoveDirection::Left => (f > 0).then(|| (r, f - 1)),
            MoveDirection::Right => (f + 1 < total_flat).then(|| (r, f + 1)),
        }
    };

    let is_filled = |r: usize, f: usize| -> bool {
        resolve_flat_coord(&sheet.archetype, f).and_then(|(c, s)| sheet.cell(CellCoord::new(r, c, s))).is_some_and(|cell| !cell.is_empty())
    };

    let Some((mut cur_r, mut cur_f)) = step(row, flat_col) else {
        return (row, flat_col);
    };

    if is_filled(row, flat_col) && is_filled(cur_r, cur_f) {
        while let Some((nr, nf)) = step(cur_r, cur_f) {
            if !is_filled(nr, nf) {
                break;
            }
            cur_r = nr;
            cur_f = nf;
        }
        return (cur_r, cur_f);
    }

    while !is_filled(cur_r, cur_f) {
        let Some((nr, nf)) = step(cur_r, cur_f) else {
            return (cur_r, cur_f);
        };
        cur_r = nr;
        cur_f = nf;
    }
    (cur_r, cur_f)
}

// ============================================================================
// Formula Bar UTF-8 Safe Autocomplete Helpers
// ============================================================================

fn clamp_char_boundary(s: &str, mut pos: usize) -> usize {
    pos = pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn active_token_span(input: &str, cursor_byte: usize) -> (usize, usize) {
    let cursor = clamp_char_boundary(input, cursor_byte);
    let prefix = &input[..cursor];

    if let Some(open_bracket) = prefix.rfind('[') {
        let after = &prefix[open_bracket + 1..];
        if !after.contains(']') {
            return (open_bracket, cursor);
        }
    }

    let mut start = cursor;
    for (idx, ch) in prefix.char_indices().rev() {
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            start = idx;
        } else {
            break;
        }
    }
    (start, cursor)
}

fn build_autocomplete_items(
    input: &str,
    cursor_byte: usize,
    target_col: Option<usize>,
    doc: Option<&ProjectDocument>,
    sheet: Option<&DataSheet>,
) -> Vec<FormulaAutocompleteItem> {
    let (start, end) = active_token_span(input, cursor_byte);
    let raw_token = &input[start..end];
    let bracket_mode = raw_token.starts_with('[');
    let query = raw_token.trim_start_matches('[').trim().to_ascii_lowercase();

    let mut items = Vec::new();
    collect_column_completions(sheet, target_col, bracket_mode, &query, &mut items);

    if !bracket_mode {
        collect_info_completions(doc, &query, &mut items);
        for &(label, insert_text, kind, detail) in FORMULA_BUILTINS {
            if query.is_empty() || label.to_ascii_lowercase().starts_with(&query) {
                items.push(FormulaAutocompleteItem {
                    label: String::from(label),
                    insert_text: String::from(insert_text),
                    kind,
                    detail: String::from(detail),
                });
            }
        }
    }

    items.sort_by(|a, b| score_completion(a, &query).cmp(&score_completion(b, &query)).then_with(|| a.label.cmp(&b.label)));
    items
}

fn collect_column_completions(
    sheet: Option<&DataSheet>,
    target_col: Option<usize>,
    bracket_mode: bool,
    query: &str,
    out: &mut Vec<FormulaAutocompleteItem>,
) {
    let Some(s) = sheet else {
        return;
    };
    let TableArchetype::MultipleVariables(mv) = &s.archetype else {
        return;
    };
    for (idx, var) in mv.variables.iter().enumerate() {
        if Some(idx) == target_col {
            continue;
        }
        let lower = var.title.to_ascii_lowercase();
        let matches = if query.is_empty() {
            true
        } else if bracket_mode {
            lower.contains(query)
        } else {
            lower.starts_with(query) || lower.contains(query)
        };
        if !matches {
            continue;
        }
        let ref_text = format_column_reference(&var.title);
        out.push(FormulaAutocompleteItem {
            label: ref_text.clone(),
            insert_text: ref_text,
            kind: FormulaAutocompleteKind::Column,
            detail: format!("{} variable", role_short_label(var.role, var.role_inferred)),
        });
    }
}

fn collect_info_completions(doc: Option<&ProjectDocument>, query: &str, out: &mut Vec<FormulaAutocompleteItem>) {
    let Some(d) = doc else {
        return;
    };
    for info in &d.info_sheets {
        for c in &info.constants {
            let lower = c.name.to_ascii_lowercase();
            if query.is_empty() || lower.contains(query) {
                out.push(FormulaAutocompleteItem {
                    label: format!("Info(\"{}\")", c.name),
                    insert_text: format!("Info(\"{}\")", c.name),
                    kind: FormulaAutocompleteKind::InfoConstant,
                    detail: format!("Constant from {}", info.title),
                });
            }
        }
    }
}

fn format_column_reference(title: &str) -> String {
    let trimmed = title.trim();
    let is_simple_ident = !trimmed.is_empty()
        && trimmed
            .chars()
            .enumerate()
            .all(|(i, c)| (i == 0 && (c.is_ascii_alphabetic() || c == '_')) || (i > 0 && (c.is_ascii_alphanumeric() || c == '_')));
    if is_simple_ident { String::from(trimmed) } else { format!("[{trimmed}]") }
}

fn score_completion(item: &FormulaAutocompleteItem, query: &str) -> (u8, usize) {
    let rank_kind = match item.kind {
        FormulaAutocompleteKind::Column => 0,
        FormulaAutocompleteKind::InfoConstant => 1,
        FormulaAutocompleteKind::ScalarFunction => 2,
        FormulaAutocompleteKind::AggregateFunction => 3,
        FormulaAutocompleteKind::WindowFunction => 4,
        FormulaAutocompleteKind::Constant => 5,
    };
    let exact_prefix = usize::from(!query.is_empty() && !item.label.trim_start_matches('[').to_ascii_lowercase().starts_with(query));
    (rank_kind, exact_prefix)
}

fn validate_formula_syntax(body: &str) -> Result<(), String> {
    let mut paren_depth: usize = 0;
    let mut in_bracket = false;
    let mut bracket_len: usize = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut last_non_ws: Option<char> = None;

    for ch in body.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            last_non_ws = Some(ch);
            continue;
        }
        if in_bracket {
            match ch {
                '[' => return Err(String::from("nested '[' inside column reference")),
                ']' if bracket_len == 0 => return Err(String::from("empty column reference '[]'")),
                ']' => in_bracket = false,
                _ => bracket_len += 1,
            }
            last_non_ws = Some(ch);
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' => {
                in_bracket = true;
                bracket_len = 0;
            }
            ']' => return Err(String::from("unmatched ']' in formula")),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.checked_sub(1).ok_or_else(|| String::from("unmatched ')' in formula"))?,
            _ => {}
        }
        if !ch.is_whitespace() {
            last_non_ws = Some(ch);
        }
    }

    if quote.is_some() {
        return Err(String::from("unclosed string literal in formula"));
    }
    if in_bracket {
        return Err(String::from("unclosed '[' column reference"));
    }
    if paren_depth > 0 {
        return Err(String::from("unclosed '(' in formula"));
    }
    if matches!(last_non_ws, Some('+' | '-' | '*' | '/' | '^' | ',' | '<' | '>' | '=' | '&' | '|')) {
        return Err(String::from("unexpected trailing operator in formula"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GridQuickStats, GridViewState, ImportOriginKind, MoveDirection};
    use curvebook_doc::{
        Cell, CellCoord, DataSheet, LinkBorderColor, MultipleVariablesTable, ProjectDocument, SubcolumnMode, TableArchetype, VariableRole, XyTable,
    };
    use curvebook_tools::GridCellCoord;

    #[test]
    fn quick_stats_computes_n_excluded_mean_sd_sem_median_sum() {
        let xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
        let mut sheet = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).unwrap();
        for (r, v) in [10.0, 12.0, 14.0, 16.0, 999.0].iter().enumerate() {
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(*v)).unwrap();
        }
        sheet.set_excluded(CellCoord::new(4, 0, 0), true).unwrap();

        let stats = GridQuickStats::from_data_sheet(&sheet, GridCellCoord::new(0, 0, 0), GridCellCoord::new(5, 0, 0));
        assert_eq!(stats.selected_cells, 6);
        assert_eq!(stats.n, 4);
        assert_eq!(stats.excluded, 1);
        assert!((stats.sum.unwrap() - 52.0).abs() < 1e-12);
        assert!((stats.mean.unwrap() - 13.0).abs() < 1e-12);
        assert!((stats.median.unwrap() - 13.0).abs() < 1e-12);
        let expected_sd = (20.0_f64 / 3.0).sqrt();
        assert!((stats.sd.unwrap() - expected_sd).abs() < 1e-12);
        assert!((stats.sem.unwrap() - expected_sd / 2.0).abs() < 1e-12);
        assert!(stats.format_footer().contains("SEM="));
    }

    #[test]
    fn formula_bar_autocompletes_columns_and_functions() {
        let mut doc = ProjectDocument::default();
        let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
        mv.variables[0].title = String::from("Body Mass");
        mv.variables[1].title = String::from("Height_m");
        mv.variables[2].title = String::from("BMI");
        let sheet = DataSheet::new("mv-1", "Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
        doc.add_data_sheet(sheet).unwrap();

        let mut grid = GridViewState::default();
        let ds = doc.data_sheet(&"mv-1".into()).unwrap();
        grid.formula_bar.open_for_column(ds, 2, Some(&doc));
        grid.formula_bar.set_input("= [Body", None, &doc);
        assert!(grid.formula_bar.completions.iter().any(|c| c.insert_text == "[Body Mass]"));
        assert!(grid.formula_bar.accept_completion(&doc));
        assert_eq!(grid.formula_bar.input, "= [Body Mass]");
        assert!(grid.formula_bar.is_valid);

        grid.formula_bar.set_input("= zsc", None, &doc);
        assert!(grid.formula_bar.completions.iter().any(|c| c.insert_text == "zscore("));
    }

    #[test]
    fn variables_dialog_and_import_wizard_and_boundary_navigation() {
        let mut doc = ProjectDocument::default();
        let mut mv = MultipleVariablesTable::new(2, 5).unwrap();
        mv.variables[0].title = String::from("EyeColor");
        for (r, s) in ["Hzel", "Blue", "Hazel", "Blue", ""].iter().enumerate() {
            if !s.is_empty() {
                mv.variables[0].cells[r] = Cell::text(*s);
            }
        }
        mv.variables[0].refresh_inferred_role();
        let sheet = DataSheet::new("mv-1", "Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
        doc.add_data_sheet(sheet).unwrap();

        let ds = doc.data_sheet(&"mv-1".into()).unwrap();
        let mut grid = GridViewState::default();
        let strip = grid.variable_role_strip(ds);
        assert_eq!(strip[0].role, VariableRole::Categorical);
        assert!(strip[0].role_inferred);

        grid.variables_dialog.open_for_column(ds, 0);
        grid.variables_dialog.merge_levels_into(&["Hzel"], "Hazel");
        let (edited, levels) = grid.variables_dialog.preview_recode(ds).unwrap();
        assert_eq!(edited, 1);
        assert_eq!(levels.len(), 2);

        // Boundary navigation stops at row 3 (last filled cell before blank row 4).
        grid.tool.select_single(GridCellCoord::new(0, 0, 0));
        grid.navigate(ds, MoveDirection::Down, true, true);
        assert_eq!(grid.tool.active, Some(GridCellCoord::new(3, 0, 0)));

        // 4-stage Import & Paste Special / Paste Link preview.
        grid.import_wizard.is_live_link = true;
        grid.import_wizard.origin_kind = ImportOriginKind::ExternalDelimitedFile;
        grid.import_wizard.delimiter = ',';
        grid.import_wizard.decimal_separator = ',';
        grid.import_wizard.raw_clipboard_text = String::from("12,5*;14,0");
        grid.import_wizard.delimiter = ';';
        assert_eq!(grid.import_wizard.preview_border_color(Some(&doc)), Some(LinkBorderColor::Charcoal));
        let preview = grid.import_wizard.preview_grid(Some(&doc));
        assert!(preview[0][0].excluded);
        assert_eq!(preview[0][0].value.finite_f64(), Some(12.5));
    }

    #[test]
    fn rv50_one_million_row_range_selection_and_brushing_completes_under_5ms_without_giant_alloc() {
        use std::time::Instant;

        use curvebook_doc::MAX_ROWS;

        // Allocate a 1,000,000-row sheet with 25 populated rows (outside timed selection/brushing region).
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, MAX_ROWS).expect("1M-row XyTable");
        for r in 0..25 {
            xy.x_cells[r][0] = Cell::number(r as f64);
            xy.data_sets[0].cells[r][0] = Cell::number((r as f64) * 2.0);
            xy.data_sets[1].cells[r][0] = Cell::number((r as f64) * 3.0);
        }
        let sheet = DataSheet::new("xy-rv50-1m", "1M Sheet", TableArchetype::Xy(xy)).expect("datasheet");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(sheet).expect("add sheet");
        doc.active_sheet_id = Some("xy-rv50-1m".into());
        let ds = doc.data_sheet(&"xy-rv50-1m".into()).expect("active ds");

        let mut grid = GridViewState::default();
        let t0 = Instant::now();
        grid.select_single_sync(GridCellCoord::new(0, 0, 0));
        grid.extend_shift_range(ds, GridCellCoord::new(MAX_ROWS - 1, 1, 0));
        grid.refresh_selection_stats(&doc);
        let coords = grid.all_selected_coords(ds);
        let targets = grid.selected_graph_point_targets(ds, 2);
        let frame = grid.visible_frame(&doc).expect("visible frame");
        let elapsed = t0.elapsed();

        let budget_ms = if cfg!(debug_assertions) { 25 } else { 5 };
        assert!(
            elapsed < std::time::Duration::from_millis(budget_ms),
            "1,000,000-row selection + brushing exceeded {budget_ms} ms budget: {elapsed:?}"
        );
        // Contiguous range selection stays in O(1) bounds (`selected_coords` unmaterialized) while `is_cell_selected` covers all 1M rows.
        assert!(grid.selected_coords.is_empty(), "rectangular range must not materialize millions of selected_coords");
        assert!(grid.is_cell_selected(ds, GridCellCoord::new(0, 0, 0)));
        assert!(grid.is_cell_selected(ds, GridCellCoord::new(MAX_ROWS - 1, 1, 0)));
        assert_eq!(grid.extended_stats.map(|s| s.selected_cells), Some(MAX_ROWS * 2));
        // Materialized brushing / coordinates intersect `0..populated_rows` (`25 * 2 = 50` cells, not 2,000,000).
        assert_eq!(coords.len(), 50);
        assert_eq!(targets.len(), 50);
        assert!(frame.rows.iter().all(|r| r.cells.iter().all(|c| c.visual.is_selected)));
    }

    #[test]
    fn audit_14_conditional_aggregate_hints_match_evaluator_arg_order() {
        use curvebook_doc::{VariableColumn, VariableSource};
        use curvebook_engine::Session;
        use serde_json::json;

        use super::FORMULA_BUILTINS;
        use crate::UiApp;
        use crate::egui_shell::trailing_studio::studio_formula_signature;
        use crate::views::mv_formula::formula_function_signature;

        let find_builtin = |name: &str| {
            FORMULA_BUILTINS.iter().find(|(label, _, _, _)| label.starts_with(&format!("{name}("))).map(|(label, insert, _, _)| (*label, *insert))
        };
        assert_eq!(find_builtin("countif"), Some(("countif(cond)", "countif(")));
        assert_eq!(find_builtin("sumif"), Some(("sumif(var, cond)", "sumif(")));
        assert_eq!(find_builtin("meanif"), Some(("meanif(var, cond)", "meanif(")));
        assert!(
            !FORMULA_BUILTINS.iter().any(|(label, _, _, _)| label.contains("(cond, var)")),
            "FORMULA_BUILTINS must not suggest reversed (cond, var) hints"
        );

        assert_eq!(formula_function_signature("countif"), Some("countif(cond)"));
        assert_eq!(formula_function_signature("sumif"), Some("sumif(var, cond)"));
        assert_eq!(formula_function_signature("meanif"), Some("meanif(var, cond)"));

        assert_eq!(studio_formula_signature("countif"), Some("countif(cond)"));
        assert_eq!(studio_formula_signature("sumif"), Some("sumif(var, cond)"));
        assert_eq!(studio_formula_signature("meanif"), Some("meanif(var, cond)"));

        let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
        mv.variables[0] = VariableColumn {
            title: String::from("Reading"),
            role: VariableRole::Continuous,
            role_inferred: false,
            source: VariableSource::Entered,
            level_order: Default::default(),
            cells: vec![Cell::number(1.0), Cell::number(10.0)],
        };
        mv.variables[1] = VariableColumn {
            title: String::from("Flag"),
            role: VariableRole::Continuous,
            role_inferred: false,
            source: VariableSource::Entered,
            level_order: Default::default(),
            cells: vec![Cell::number(1.0), Cell::number(0.0)],
        };
        let sheet = DataSheet::new("mv-14", "Conditional", TableArchetype::MultipleVariables(mv)).unwrap();
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(sheet).unwrap();
        doc.active_sheet_id = Some("mv-14".into());

        let mut grid = GridViewState::default();
        let ds = doc.data_sheet(&"mv-14".into()).unwrap();
        grid.formula_bar.open_for_new_column(ds, "CondSum", Some(&doc));
        grid.formula_bar.set_input("= sumi", None, &doc);
        assert!(grid.formula_bar.completions.iter().any(|c| c.label == "sumif(var, cond)" && c.insert_text == "sumif("));
        grid.formula_bar.set_input("= meani", None, &doc);
        assert!(grid.formula_bar.completions.iter().any(|c| c.label == "meanif(var, cond)" && c.insert_text == "meanif("));

        let mut app = UiApp::from_session(Session::from_document(doc));
        app.execute_command("table.setCalculatedVariable", json!({ "sheet_id": "mv-14", "title": "CondSum", "formula": "= sumif(Reading, Flag)" }))
            .expect("evaluate sumif(Reading, Flag)");
        let updated_ds = app.document().data_sheet(&"mv-14".into()).unwrap();
        let TableArchetype::MultipleVariables(updated_mv) = &updated_ds.archetype else {
            panic!("expected MultipleVariables table");
        };
        let cond_sum = updated_mv.variables.iter().find(|v| v.title == "CondSum").expect("CondSum variable");
        assert_eq!(cond_sum.cells[0].active_number(), Some(1.0));
        assert_eq!(cond_sum.cells[1].active_number(), Some(1.0));
    }
}
