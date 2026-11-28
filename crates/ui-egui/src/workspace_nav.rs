//! Project Navigator tree state, anchored Family dependency pane model, and Welcome / New Table
//! + Clone Pipeline wizard state (`docs/evidence/user-interface.md`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use curvebook_doc::{
    Cell, ChecklistStatus, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, DiagnosticPlotKind,
    DocError, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedTable, LayoutItem, LinkBorderColor, LinkSource, MultipleVariablesTable,
    NestedTable, PartsOfWholeTable, ProjectDocument, ResultSheet, ResultStatus, SheetFolderKind, SheetId, SheetOrigin, SheetSection, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_tools::{NavigatorDragState, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Display mode for the five-folder Project Navigator tree (`docs/evidence/user-interface.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NavigatorNestingMode {
    /// Five independent top-level folders (`Data Tables`, `Info`, `Results`, `Graphs`, `Layouts`).
    #[default]
    SeparateFolders,
    /// Nests derived `Results` sheets directly underneath their upstream `Data Tables` entry.
    NestedResultsUnderData,
}

/// Logical badge hue rendered in the Project Navigator (`Blue`, `Green`, `Red`, `Charcoal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NavigatorBadgeColor {
    Blue,
    Green,
    Red,
    Charcoal,
}

impl NavigatorBadgeColor {
    /// Convert to a high-contrast sRGB color swatch.
    #[must_use]
    pub const fn rgba(self) -> Rgba8 {
        match self {
            Self::Blue => Rgba8::rgb(59, 130, 246),
            Self::Green => Rgba8::rgb(22, 163, 74),
            Self::Red => Rgba8::rgb(220, 38, 38),
            Self::Charcoal => Rgba8::rgb(55, 65, 81),
        }
    }
}

/// Semantic category for a navigator status or linked-region pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NavigatorBadgeKind {
    /// Live link to an external delimited (`CSV` / `TSV`) file.
    LiveFileLink,
    /// Live link to an internal `DataSheet` or `ResultSheet`.
    InternalTableLink,
    /// Live link to an analyzable `ResultSheet` (`Transform`, `Normalize`, `Transpose`).
    AnalyzableResultLink,
    /// Live link to a terminal statistical or regression `ResultSheet`.
    TerminalResultLink,
    /// Sheet requires recalculation in the reactive DAG.
    Dirty,
    /// Analysis or calculation failed with a diagnostic error.
    Failed,
    /// Data table is locked against edits (`DataSheet::frozen == true`).
    Frozen,
}

/// Visual status or provenance badge attached to a [`NavigatorEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigatorBadge {
    pub kind: NavigatorBadgeKind,
    pub color: NavigatorBadgeColor,
    pub rgba: Rgba8,
    pub label: String,
}

impl NavigatorBadge {
    #[must_use]
    fn new(kind: NavigatorBadgeKind, color: NavigatorBadgeColor, label: impl Into<String>) -> Self {
        Self { kind, color, rgba: color.rgba(), label: label.into() }
    }
}

/// Disambiguation metadata for one sheet in Navigator, Family lineage, and Split View selectors (`R20.12`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SheetDisambiguation {
    pub subtitle: Option<String>,
    pub has_duplicate_title: bool,
}

/// Navigator entry summarizing one sheet in one of the five project sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigatorEntry {
    pub sheet_id: SheetId,
    pub section: SheetSection,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub has_duplicate_title: bool,
    pub is_active: bool,
    pub is_bold: bool,
    pub is_upstream_dependency: bool,
    pub is_downstream_dependent: bool,
    pub is_frozen: bool,
    pub is_dirty: bool,
    pub archetype_label: Option<String>,
    pub parent_sheet_id: Option<SheetId>,
    pub nesting_depth: usize,
    pub color_tag: Option<Rgba8>,
    pub link_colors: Vec<LinkBorderColor>,
    pub badges: Vec<NavigatorBadge>,
    pub floating_note_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SheetOrigin>,
}

/// One of the five canonical folders in the Project Navigator (`Data Tables`, `Info`, `Results`,
/// `Graphs`, `Layouts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigatorFolder {
    pub section: SheetSection,
    pub label: String,
    pub collapsed: bool,
    pub entries: Vec<NavigatorEntry>,
}

/// Relationship of a sheet inside the anchored Family pane relative to `family_anchor_sheet_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FamilyRelation {
    Anchor,
    Upstream,
    Downstream,
}

/// Node displayed in the live Family dependency pane (`Data -> Transform -> Stats/Fit -> Graph -> Layout`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyPaneNode {
    pub sheet_id: SheetId,
    pub section: SheetSection,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub has_duplicate_title: bool,
    pub relation: FamilyRelation,
    pub hop_distance: usize,
    pub is_active: bool,
    pub is_anchor: bool,
    pub is_frozen: bool,
    pub is_dirty: bool,
}

/// Active tab inside the Welcome / Project Initialization dialog (`docs/evidence/user-interface.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WelcomeMode {
    #[default]
    EmptyTypedTable,
    TutorialSample,
    FromTemplate,
    ClonePipeline,
}

/// Draft configuration for creating a new typed data table across the 8 archetypes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableCreationDraft {
    pub kind: TableKind,
    pub sheet_id: String,
    pub title: String,
    pub data_set_count: usize,
    pub row_count: usize,
    pub replicates: usize,
    pub subcolumn_mode: SubcolumnMode,
    pub column_design: ColumnDesignHint,
    pub x_mode: XColumnMode,
    pub create_default_graph: bool,
    pub create_paired_info: bool,
}

impl Default for TableCreationDraft {
    fn default() -> Self {
        Self::for_kind(TableKind::Xy)
    }
}

impl TableCreationDraft {
    /// Construct sensible scientific defaults for one of the eight [`TableKind`] archetypes.
    #[must_use]
    pub fn for_kind(kind: TableKind) -> Self {
        let (replicates, subcolumn_mode, data_set_count, row_count) = match kind {
            TableKind::Xy => (1, SubcolumnMode::Single, 2, 12),
            TableKind::Column => (1, SubcolumnMode::Single, 3, 12),
            TableKind::Grouped => (3, SubcolumnMode::RawReplicates(3), 2, 6),
            TableKind::Contingency => (1, SubcolumnMode::Single, 2, 2),
            TableKind::Survival => (1, SubcolumnMode::Single, 2, 16),
            TableKind::PartsOfWhole => (1, SubcolumnMode::Single, 1, 5),
            TableKind::MultipleVariables => (1, SubcolumnMode::Single, 4, 20),
            TableKind::Nested => (3, SubcolumnMode::RawReplicates(3), 2, 6),
        };
        Self {
            kind,
            sheet_id: String::from("data-1"),
            title: format!("{} Data", kind.label()),
            data_set_count,
            row_count,
            replicates,
            subcolumn_mode,
            column_design: ColumnDesignHint::UnpairedReplicates,
            x_mode: XColumnMode::default(),
            create_default_graph: true,
            create_paired_info: true,
        }
    }

    /// Update replicate count `k` (`1..=MAX_SUBCOLUMNS`) and keep `subcolumn_mode` synchronized
    /// for archetypes that support raw replicate subcolumns (`Xy`, `Grouped`, `Nested`).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_replicates(mut self, k: usize) -> Self {
        let clamped = k.max(1);
        self.replicates = clamped;
        match self.kind {
            TableKind::Xy | TableKind::Grouped | TableKind::Nested => {
                self.subcolumn_mode =
                    if clamped == 1 && matches!(self.kind, TableKind::Xy) { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(clamped) };
            }
            TableKind::Column | TableKind::Contingency | TableKind::Survival | TableKind::PartsOfWhole | TableKind::MultipleVariables => {}
        }
        self
    }

    /// Update the `Column` design hint (`UnpairedReplicates`, `PairedRepeatedMeasures`, `PrecomputedError`).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_column_design(mut self, design: ColumnDesignHint) -> Self {
        self.column_design = design;
        if self.kind == TableKind::Column {
            self.subcolumn_mode = match design {
                ColumnDesignHint::UnpairedReplicates | ColumnDesignHint::PairedRepeatedMeasures => SubcolumnMode::Single,
                ColumnDesignHint::PrecomputedError => SubcolumnMode::MeanSdN,
            };
        }
        self
    }

    /// Build a validated [`TableArchetype`] matching the draft settings.
    pub fn build_archetype(&self) -> Result<TableArchetype, DocError> {
        match self.kind {
            TableKind::Xy => {
                let mode = if self.replicates > 1 && matches!(self.subcolumn_mode, SubcolumnMode::Single) {
                    SubcolumnMode::RawReplicates(self.replicates)
                } else {
                    self.subcolumn_mode
                };
                Ok(TableArchetype::Xy(XyTable::new_with_x_mode(self.x_mode.clone(), mode, self.data_set_count, self.row_count)?))
            }
            TableKind::Column => Ok(TableArchetype::Column(ColumnTable::new_with_subcolumns(
                self.column_design,
                self.subcolumn_mode,
                self.data_set_count,
                self.row_count,
            )?)),
            TableKind::Grouped => {
                let mode = if matches!(self.subcolumn_mode, SubcolumnMode::Single) {
                    SubcolumnMode::RawReplicates(self.replicates.max(1))
                } else {
                    self.subcolumn_mode
                };
                Ok(TableArchetype::Grouped(GroupedTable::new(mode, self.data_set_count, self.row_count)?))
            }
            TableKind::Contingency => Ok(TableArchetype::Contingency(ContingencyTable::new(self.data_set_count, self.row_count)?)),
            TableKind::Survival => Ok(TableArchetype::Survival(SurvivalTable::new(self.data_set_count, self.row_count)?)),
            TableKind::PartsOfWhole => Ok(TableArchetype::PartsOfWhole(PartsOfWholeTable::new(self.data_set_count, self.row_count)?)),
            TableKind::MultipleVariables => Ok(TableArchetype::MultipleVariables(MultipleVariablesTable::new(self.data_set_count, self.row_count)?)),
            TableKind::Nested => Ok(TableArchetype::Nested(NestedTable::new(self.data_set_count, self.replicates, self.row_count)?)),
        }
    }

    /// Emit a canonical `"sheet.createData"` command intent for the headless engine.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn to_command_intent(&self) -> ToolCommandIntent {
        ToolCommandIntent {
            command_id: String::from("sheet.createData"),
            params: json!({
                "sheet_id": self.sheet_id,
                "title": self.title,
                "kind": self.kind,
                "cols": self.data_set_count,
                "rows": self.row_count,
                "replicates": self.replicates,
                "subcolumn_mode": self.subcolumn_mode,
                "design_hint": self.column_design,
                "x_mode": self.x_mode,
                "auto_graph": self.create_default_graph,
                "auto_info": self.create_paired_info,
            }),
        }
    }
}

/// Curated self-guided tutorial / sample dataset descriptor (`docs/evidence/user-interface.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleFixtureDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: TableKind,
    pub summary: &'static str,
    pub row_count: usize,
    pub column_count: usize,
    pub replicates: usize,
}

/// Built-in clean-room tutorial datasets spanning all eight scientific table archetypes.
pub const SAMPLE_FIXTURES: &[SampleFixtureDescriptor] = &[
    SampleFixtureDescriptor {
        id: "dose-response-4pl",
        title: "Ligand Dose-Response Inhibition",
        kind: TableKind::Xy,
        summary: "Log concentration vs. normalized fluorescence with 3 replicates per point.",
        row_count: 6,
        column_count: 2,
        replicates: 3,
    },
    SampleFixtureDescriptor {
        id: "unpaired-treatment-groups",
        title: "Three-Arm Treatment Screen",
        kind: TableKind::Column,
        summary: "Vehicle, Compound A, and Compound B unpaired replicate measurements.",
        row_count: 5,
        column_count: 3,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "paired-baseline-treatment",
        title: "Before vs. After Subject Cohort",
        kind: TableKind::Column,
        summary: "Matched subject measurements before and after dosing with expanded row labels.",
        row_count: 5,
        column_count: 2,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "two-way-time-treatment",
        title: "Time-Course Two-Factor Assay",
        kind: TableKind::Grouped,
        summary: "Wildtype vs. Mutant measured across 3 time points with 3 replicates.",
        row_count: 3,
        column_count: 2,
        replicates: 3,
    },
    SampleFixtureDescriptor {
        id: "contingency-2x2",
        title: "Responder Cross-Tabulation",
        kind: TableKind::Contingency,
        summary: "2x2 integer event counts comparing Placebo vs. Treated cohorts.",
        row_count: 2,
        column_count: 2,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "kaplan-meier-cohorts",
        title: "Time-to-Event Cohort Survival",
        kind: TableKind::Survival,
        summary: "Elapsed days with event (1) and censored (0) indicators for two cohorts.",
        row_count: 6,
        column_count: 2,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "cell-cycle-fractions",
        title: "Cell-Cycle Phase Distribution",
        kind: TableKind::PartsOfWhole,
        summary: "Percentage breakdown of G0/G1, S, G2/M, and Sub-G1 populations.",
        row_count: 4,
        column_count: 1,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "clinical-biomarker-cohort",
        title: "Multivariate Biomarker Cohort",
        kind: TableKind::MultipleVariables,
        summary: "Tidy table mixing Continuous, Categorical, and Calculated BMI variables.",
        row_count: 6,
        column_count: 4,
        replicates: 1,
    },
    SampleFixtureDescriptor {
        id: "nested-animal-replicates",
        title: "Nested Animal & Technical Replicates",
        kind: TableKind::Nested,
        summary: "Control vs. Treated groups with 3 biological subjects and 4 technical reads each.",
        row_count: 4,
        column_count: 2,
        replicates: 3,
    },
];

/// Instantiate one of [`SAMPLE_FIXTURES`] as a populated clean-room [`DataSheet`].
pub fn instantiate_sample_fixture(fixture_id: &str, sheet_id: impl Into<SheetId>) -> Result<DataSheet, DocError> {
    let sheet_id = sheet_id.into();
    let Some(meta) = SAMPLE_FIXTURES.iter().find(|f| f.id == fixture_id) else {
        return Err(DocError::SheetNotFound { sheet_id: SheetId::new(fixture_id) });
    };
    let mut sheet = match meta.id {
        "dose-response-4pl" => build_dose_response_fixture(sheet_id, meta.title)?,
        "unpaired-treatment-groups" => build_unpaired_column_fixture(sheet_id, meta.title)?,
        "paired-baseline-treatment" => build_paired_column_fixture(sheet_id, meta.title)?,
        "two-way-time-treatment" => build_grouped_fixture(sheet_id, meta.title)?,
        "contingency-2x2" => build_contingency_fixture(sheet_id, meta.title)?,
        "kaplan-meier-cohorts" => build_survival_fixture(sheet_id, meta.title)?,
        "cell-cycle-fractions" => build_parts_of_whole_fixture(sheet_id, meta.title)?,
        "clinical-biomarker-cohort" => build_multivar_fixture(sheet_id, meta.title)?,
        "nested-animal-replicates" => build_nested_fixture(sheet_id, meta.title)?,
        _ => DataSheet::new(sheet_id, meta.title, TableCreationDraft::for_kind(meta.kind).build_archetype()?)?,
    };
    sheet.floating_notes.clear();
    Ok(sheet)
}

/// Configuration draft for `sheet.clonePipeline` ("Clone a Graph / Pipeline"; `docs/evidence/user-interface.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClonePipelineDraft {
    pub source_sheet_id: Option<SheetId>,
    pub new_sheet_id: String,
    pub options: CloneDataSheetOptions,
    pub clone_downstream_analyses: bool,
    pub clone_downstream_graphs: bool,
    pub clone_downstream_layouts: bool,
}

impl Default for ClonePipelineDraft {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            new_sheet_id: String::from("data-clone-1"),
            options: CloneDataSheetOptions {
                title: String::from("Cloned Pipeline"),
                keep_x_values: true,
                clear_y_values: true,
                clear_column_titles: false,
                clear_row_titles: false,
                override_replicates: None,
            },
            clone_downstream_analyses: true,
            clone_downstream_graphs: true,
            clone_downstream_layouts: false,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ClonePipelineDraft {
    /// Emit a `"sheet.clonePipeline"` command intent when `source_sheet_id` is configured.
    #[must_use]
    pub fn to_command_intent(&self) -> Option<ToolCommandIntent> {
        let source = self.source_sheet_id.as_ref()?;
        Some(ToolCommandIntent {
            command_id: String::from("sheet.clonePipeline"),
            params: json!({
                "sheet_id": source.as_str(),
                "new_sheet_id": self.new_sheet_id,
                "title": self.options.title,
                "keep_x_values": self.options.keep_x_values,
                "clear_y_values": self.options.clear_y_values,
                "clear_column_titles": self.options.clear_column_titles,
                "clear_row_titles": self.options.clear_row_titles,
                "override_replicates": self.options.override_replicates,
                "clone_analyses": self.clone_downstream_analyses,
                "clone_graphs": self.clone_downstream_graphs,
                "clone_layouts": self.clone_downstream_layouts,
            }),
        })
    }
}

/// Keyboard-navigable Welcome / New Table / Clone Pipeline initialization wizard model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WelcomeWizardState {
    pub open: bool,
    pub mode: WelcomeMode,
    pub focused_archetype_index: usize,
    pub selected_fixture_id: Option<String>,
    pub template_path: Option<String>,
    pub table_draft: TableCreationDraft,
    pub clone_draft: ClonePipelineDraft,
}

impl Default for WelcomeWizardState {
    fn default() -> Self {
        Self {
            open: false,
            mode: WelcomeMode::EmptyTypedTable,
            focused_archetype_index: 0,
            selected_fixture_id: SAMPLE_FIXTURES.first().map(|f| String::from(f.id)),
            template_path: None,
            table_draft: TableCreationDraft::default(),
            clone_draft: ClonePipelineDraft::default(),
        }
    }
}

impl WelcomeWizardState {
    /// Move keyboard focus ring forward across the 8 table archetypes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn focus_next_archetype(&mut self) {
        let idx = (self.focused_archetype_index + 1) % TableKind::ALL.len();
        self.select_archetype(TableKind::ALL[idx]);
    }

    /// Select `kind` and update `focused_archetype_index` + `table_draft`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn select_archetype(&mut self, kind: TableKind) {
        if let Some(pos) = TableKind::ALL.iter().position(|&k| k == kind) {
            self.focused_archetype_index = pos;
        }
        let id = self.table_draft.sheet_id.clone();
        self.table_draft = TableCreationDraft::for_kind(kind);
        self.table_draft.sheet_id = id;
    }

    /// Update replicate count `k` on the current table creation draft.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_replicates(&mut self, k: usize) {
        self.table_draft = self.table_draft.clone().with_replicates(k);
    }

    /// Open the wizard directly into `ClonePipeline` mode anchored on `source_sheet_id`.
    pub fn trigger_clone_pipeline(&mut self, source_sheet_id: SheetId, title: impl Into<String>) {
        self.open = true;
        self.mode = WelcomeMode::ClonePipeline;
        self.clone_draft.source_sheet_id = Some(source_sheet_id);
        self.clone_draft.options.title = title.into();
    }
}

/// Active keyboard focus pane cycled with `F6` / `Shift+F6`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ActiveWorkspacePane {
    Navigator,
    FamilyStrip,
    #[default]
    MainCanvasOrGrid,
    Inspector,
    FloatingNotes,
}

impl ActiveWorkspacePane {
    /// Canonical pane focus cycle order (`F6` forward, `Shift+F6` reverse).
    pub const CYCLE_ORDER: [Self; 5] = [Self::Navigator, Self::FamilyStrip, Self::MainCanvasOrGrid, Self::Inspector, Self::FloatingNotes];

    /// Step one pane forward (`reverse = false`) or backward (`reverse = true`).
    #[must_use]
    pub const fn cycle(self, reverse: bool) -> Self {
        let idx = match self {
            Self::Navigator => 0,
            Self::FamilyStrip => 1,
            Self::MainCanvasOrGrid => 2,
            Self::Inspector => 3,
            Self::FloatingNotes => 4,
        };
        let len = Self::CYCLE_ORDER.len();
        let next = if reverse { (idx + len - 1) % len } else { (idx + 1) % len };
        Self::CYCLE_ORDER[next]
    }
}

/// Sort order options inside Portfolio Gallery (`R10.35`, Locked Decision #1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum GallerySortOrder {
    /// Follow Navigator folder sequence (`Data -> Info -> Results -> Graphs -> Layouts`) with collapsible sections in `All`.
    #[default]
    NavigatorOrder,
    /// Alphabetical by sheet title (`A–Z`), flattened across folders.
    NameAsc,
    /// Grouped by sheet type (`Graphs -> Layouts -> Data -> Results -> Info`) with collapsible sections in `All`.
    Type,
    /// Descending by series, panels, rows, tabs, or constants count, flattened across folders.
    SeriesOrRows,
}

impl GallerySortOrder {
    /// Ordered list of all supported Portfolio Gallery sort options.
    pub const ALL: [Self; 4] = [Self::NavigatorOrder, Self::NameAsc, Self::Type, Self::SeriesOrRows];

    /// Stable machine slug for inspection metadata and widget IDs.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::NavigatorOrder => "navigator_order",
            Self::NameAsc => "name_az",
            Self::Type => "type",
            Self::SeriesOrRows => "series_or_rows",
        }
    }

    /// Localization catalog key for this sort option.
    #[must_use]
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::NavigatorOrder => "gallery.sort.navigator",
            Self::NameAsc => "gallery.sort.name_az",
            Self::Type => "gallery.sort.type",
            Self::SeriesOrRows => "gallery.sort.series_rows",
        }
    }

    /// Locked Decision #1: collapsible section headers appear in `All` view only for `NavigatorOrder` and `Type`.
    #[must_use]
    pub const fn uses_collapsible_sections(self) -> bool {
        matches!(self, Self::NavigatorOrder | Self::Type)
    }
}

/// Per-folder sheet counts displayed on Portfolio Gallery filter pills (`R10.35`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GalleryFolderCounts {
    pub total: usize,
    pub data: usize,
    pub info: usize,
    pub results: usize,
    pub graphs: usize,
    pub layouts: usize,
}

impl GalleryFolderCounts {
    /// Compute sheet counts per folder from `doc`.
    #[must_use]
    pub fn from_document(doc: &ProjectDocument) -> Self {
        let data = doc.data_sheets.len();
        let info = doc.info_sheets.len();
        let results = doc.result_sheets.len();
        let graphs = doc.graph_sheets.len();
        let layouts = doc.layout_sheets.len();
        Self { total: data + info + results + graphs + layouts, data, info, results, graphs, layouts }
    }

    /// Count matching an optional folder filter (`None` = `All`).
    #[must_use]
    pub const fn for_filter(self, filter: Option<SheetFolderKind>) -> usize {
        match filter {
            None => self.total,
            Some(SheetFolderKind::Data) => self.data,
            Some(SheetFolderKind::Info) => self.info,
            Some(SheetFolderKind::Results) => self.results,
            Some(SheetFolderKind::Graphs) => self.graphs,
            Some(SheetFolderKind::Layouts) => self.layouts,
        }
    }
}

/// Visual summary card rendered in the Portfolio Gallery overview and batch-export view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioCard {
    pub sheet_id: SheetId,
    pub title: String,
    pub folder: SheetFolderKind,
    pub badge_label: String,
    pub subtitle: String,
    pub color_tag: Option<Rgba8>,
    pub active: bool,
    pub selected: bool,
    #[serde(default)]
    pub is_frozen: bool,
    #[serde(default)]
    pub is_uninitialized: bool,
    #[serde(default)]
    pub is_warning_or_stale: bool,
    #[serde(default)]
    pub metric_count: usize,
    #[serde(default)]
    pub navigator_index: usize,
}

/// State for the Project Portfolio Gallery overlay supporting folder filters, sorting, and multi-select batch workflows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PortfolioGalleryState {
    pub open: bool,
    pub filter_folder: Option<SheetFolderKind>,
    pub selected_ids: BTreeSet<SheetId>,
    pub cards: Vec<PortfolioCard>,
    #[serde(default)]
    pub search_query: String,
    #[serde(default)]
    pub sort_order: GallerySortOrder,
    #[serde(default)]
    pub current_family_only: bool,
}

/// Build an `"info.setConstant"` [`ToolCommandIntent`] for updating or creating an [`curvebook_doc::InfoSheet`] constant.
#[must_use]
pub fn build_set_info_constant_command(sheet_id: &SheetId, name: &str, value: &ConstantValue, units: Option<&str>) -> ToolCommandIntent {
    let mut obj = serde_json::Map::new();
    obj.insert(String::from("sheet_id"), json!(sheet_id.as_str()));
    obj.insert(String::from("name"), json!(name));
    match value {
        ConstantValue::Empty => {
            obj.insert(String::from("value"), Value::Null);
        }
        ConstantValue::Number(v) => {
            obj.insert(String::from("value"), if v.is_finite() { json!(v) } else { Value::Null });
        }
        ConstantValue::Text(s) => {
            obj.insert(String::from("value"), json!(s));
        }
        ConstantValue::Date(s) => {
            obj.insert(String::from("value"), json!(s));
            obj.insert(String::from("kind"), json!("date"));
        }
    }
    if let Some(u) = units {
        obj.insert(String::from("units"), json!(u));
    }
    ToolCommandIntent::new("info.setConstant", Value::Object(obj))
}

/// Build an `"info.setNote"` [`ToolCommandIntent`] for updating or appending structured notes on an [`curvebook_doc::InfoSheet`].
#[must_use]
pub fn build_set_info_notes_command(sheet_id: &SheetId, markdown: &str, append: bool) -> ToolCommandIntent {
    ToolCommandIntent::new(
        "info.setNote",
        json!({
            "sheet_id": sheet_id.as_str(),
            "text": markdown,
            "append": append,
        }),
    )
}

/// Front-end drawer state for the Project Navigator, Family dependency pane, and Welcome wizard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceNavState {
    pub navigator_open: bool,
    pub inspector_open: bool,
    pub active_pane: ActiveWorkspacePane,
    pub navigator_drag: NavigatorDragState,
    pub gallery: PortfolioGalleryState,
    pub floating_notes_collapsed: BTreeMap<String, bool>,
    pub entries: Vec<NavigatorEntry>,
    pub family_related_ids: Vec<SheetId>,
    pub nesting_mode: NavigatorNestingMode,
    pub search_query: String,
    pub folders: Vec<NavigatorFolder>,
    pub all_entries: Vec<NavigatorEntry>,
    pub collapsed_sections: Vec<SheetSection>,
    pub family_anchor_sheet_id: Option<SheetId>,
    pub family_nodes: Vec<FamilyPaneNode>,
    pub transitive_upstream_ids: Vec<SheetId>,
    pub transitive_downstream_ids: Vec<SheetId>,
    pub welcome: WelcomeWizardState,
    pub pending_intents: Vec<ToolCommandIntent>,
}

impl Default for WorkspaceNavState {
    fn default() -> Self {
        Self {
            navigator_open: true,
            inspector_open: true,
            active_pane: ActiveWorkspacePane::default(),
            navigator_drag: NavigatorDragState::default(),
            gallery: PortfolioGalleryState::default(),
            floating_notes_collapsed: BTreeMap::new(),
            entries: Vec::new(),
            family_related_ids: Vec::new(),
            nesting_mode: NavigatorNestingMode::SeparateFolders,
            search_query: String::new(),
            folders: default_empty_folders(),
            all_entries: Vec::new(),
            collapsed_sections: Vec::new(),
            family_anchor_sheet_id: None,
            family_nodes: Vec::new(),
            transitive_upstream_ids: Vec::new(),
            transitive_downstream_ids: Vec::new(),
            welcome: WelcomeWizardState::default(),
            pending_intents: Vec::new(),
        }
    }
}

/// Resolve the live zero-based index of `sheet_id` inside `folder` of `doc`, if still present.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn live_sheet_index_in_folder(doc: &ProjectDocument, folder: SheetFolderKind, sheet_id: &str) -> Option<usize> {
    match folder {
        SheetFolderKind::Data => doc.data_sheets.iter().position(|s| s.id.as_str() == sheet_id),
        SheetFolderKind::Info => doc.info_sheets.iter().position(|s| s.id.as_str() == sheet_id),
        SheetFolderKind::Results => doc.result_sheets.iter().position(|s| s.id.as_str() == sheet_id),
        SheetFolderKind::Graphs => doc.graph_sheets.iter().position(|s| s.id.as_str() == sheet_id),
        SheetFolderKind::Layouts => doc.layout_sheets.iter().position(|s| s.id.as_str() == sheet_id),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn folder_len(doc: &ProjectDocument, folder: SheetFolderKind) -> usize {
    match folder {
        SheetFolderKind::Data => doc.data_sheets.len(),
        SheetFolderKind::Info => doc.info_sheets.len(),
        SheetFolderKind::Results => doc.result_sheets.len(),
        SheetFolderKind::Graphs => doc.graph_sheets.len(),
        SheetFolderKind::Layouts => doc.layout_sheets.len(),
    }
}

impl WorkspaceNavState {
    /// Cycle active workspace pane focus forward (`reverse = false`, `F6`) or backward (`reverse = true`, `Shift+F6`).
    pub fn cycle_focus_pane(&mut self, reverse: bool) -> ActiveWorkspacePane {
        self.active_pane = self.active_pane.cycle(reverse);
        self.active_pane
    }

    /// Begin dragging a navigator sheet within `folder` starting at `from_index`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn begin_sheet_drag(&mut self, folder: SheetFolderKind, sheet_id: impl Into<String>, from_index: usize) {
        self.navigator_drag.begin_drag(folder, sheet_id, from_index);
    }

    /// Update the live drop slot index during an active navigator sheet drag.
    #[cfg(any(test, feature = "test-support"))]
    pub fn update_sheet_hover(&mut self, to_index: usize) {
        self.navigator_drag.update_hover(to_index);
    }

    /// Update the live drop slot index only when `folder` matches the dragged sheet's folder.
    #[cfg(any(test, feature = "test-support"))]
    pub fn update_sheet_hover_in_folder(&mut self, folder: SheetFolderKind, to_index: usize) {
        self.navigator_drag.update_hover_in_folder(folder, to_index);
    }

    /// Cancel an in-progress navigator sheet drag without emitting a reorder command.
    pub fn cancel_sheet_drag(&mut self) {
        self.navigator_drag.cancel();
    }

    /// Commit an active navigator drag-reorder as an engine `("sheet.reorder", payload)` command pair.
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_sheet_drop_command(&mut self) -> Option<(&'static str, Value)> {
        let (folder, from_index, to_index) = self.navigator_drag.commit_drop()?;
        if from_index == to_index {
            return None;
        }
        Some(("sheet.reorder", json!({ "folder": folder, "from_index": from_index, "to_index": to_index })))
    }

    /// Commit an active navigator drag-reorder by resolving the dragged `sheet_id` against the
    /// live `ProjectDocument` (`R1-S11-11`) rather than relying on the start index captured before
    /// mid-drag edits.
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_sheet_drop_command_for_doc(&mut self, doc: &ProjectDocument) -> Option<(&'static str, Value)> {
        let (folder, sheet_id, _captured_from_index, raw_to_index) = self.navigator_drag.commit_drop_with_sheet()?;
        let live_from_index = live_sheet_index_in_folder(doc, folder, &sheet_id)?;
        let count = folder_len(doc, folder);
        let to_index = raw_to_index.min(count.saturating_sub(1));
        if live_from_index == to_index {
            return None;
        }
        Some(("sheet.reorder", json!({ "sheet_id": sheet_id, "folder": folder, "index": to_index, "to_index": to_index })))
    }

    /// Toggle visibility of the Project Portfolio Gallery overview.
    pub fn toggle_gallery(&mut self) -> bool {
        self.gallery.open = !self.gallery.open;
        self.gallery.open
    }

    /// Update the active folder filter and refresh gallery cards from `doc`.
    pub fn set_gallery_filter(&mut self, folder: Option<SheetFolderKind>, doc: &ProjectDocument) {
        self.gallery.filter_folder = folder;
        self.sync_gallery(doc);
    }

    /// Select a gallery card (single-select when `additive = false`, toggle membership when `additive = true`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn select_gallery_card(&mut self, sheet_id: &SheetId, additive: bool) {
        if additive {
            if !self.gallery.selected_ids.remove(sheet_id) {
                self.gallery.selected_ids.insert(sheet_id.clone());
            }
        } else {
            self.gallery.selected_ids.clear();
            self.gallery.selected_ids.insert(sheet_id.clone());
        }
        for card in &mut self.gallery.cards {
            card.selected = self.gallery.selected_ids.contains(&card.sheet_id);
        }
    }

    /// Clear all multi-selected Portfolio Gallery sheet IDs.
    #[cfg(any(test, feature = "test-support"))]
    pub fn clear_gallery_selection(&mut self) {
        self.gallery.selected_ids.clear();
        for card in &mut self.gallery.cards {
            card.selected = false;
        }
    }

    /// Return selected sheet IDs ordered consistently with visible gallery cards (for batch figure export).
    #[must_use]
    pub fn selected_gallery_ids(&self) -> Vec<SheetId> {
        let mut out: Vec<SheetId> = self.gallery.cards.iter().filter(|c| c.selected).map(|c| c.sheet_id.clone()).collect();
        for id in &self.gallery.selected_ids {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
        out
    }

    /// Collect every sheet ID belonging to the active Navigator/Family lineage (`R10.35`).
    #[must_use]
    pub fn collect_current_family_ids(&self, doc: &ProjectDocument) -> BTreeSet<SheetId> {
        let mut out = BTreeSet::new();
        for node in &self.family_nodes {
            out.insert(node.sheet_id.clone());
        }
        for id in &self.family_related_ids {
            out.insert(id.clone());
        }
        for id in &self.transitive_upstream_ids {
            out.insert(id.clone());
        }
        for id in &self.transitive_downstream_ids {
            out.insert(id.clone());
        }
        if let Some(anchor) = self.family_anchor_sheet_id.as_ref().or(doc.family_anchor_sheet_id.as_ref()).or(doc.active_sheet_id.as_ref()) {
            out.insert(anchor.clone());
            if self.family_nodes.is_empty() {
                for dep in collect_downstream_dependents(doc, anchor) {
                    out.insert(dep);
                }
            }
        }
        out
    }

    /// Update Portfolio Gallery view options (search, sort order, Current Family filter) and resync cards.
    pub fn set_gallery_view_options(
        &mut self,
        doc: &ProjectDocument,
        search_query: Option<String>,
        sort_order: Option<GallerySortOrder>,
        current_family_only: Option<bool>,
    ) {
        if let Some(q) = search_query {
            self.gallery.search_query = q;
        }
        if let Some(s) = sort_order {
            self.gallery.sort_order = s;
        }
        if let Some(f) = current_family_only {
            self.gallery.current_family_only = f;
        }
        self.sync_gallery(doc);
    }

    /// Build ordered, filtered [`PortfolioCard`] entries using explicit search/sort/family overrides so
    /// [`Self::sync_gallery`] and `center_stage` always share the exact same ordering (`R10.35`).
    #[must_use]
    pub fn build_gallery_cards_with(
        &self,
        doc: &ProjectDocument,
        search_query: &str,
        sort_order: GallerySortOrder,
        current_family_only: bool,
    ) -> Vec<PortfolioCard> {
        let active = doc.active_sheet_id.as_ref();
        let selected = &self.gallery.selected_ids;
        let filter = self.gallery.filter_folder;
        let needle = search_query.trim().to_ascii_lowercase();
        let family_ids = current_family_only.then(|| self.collect_current_family_ids(doc));
        let mut cards = Vec::new();
        let mut nav_idx = 0usize;

        let matches_query = |card: &PortfolioCard| {
            if let Some(ref fam) = family_ids
                && !fam.contains(&card.sheet_id)
            {
                return false;
            }
            if needle.is_empty() {
                return true;
            }
            card.title.to_ascii_lowercase().contains(&needle)
                || card.badge_label.to_ascii_lowercase().contains(&needle)
                || card.subtitle.to_ascii_lowercase().contains(&needle)
        };

        for s in &doc.data_sheets {
            let idx = nav_idx;
            nav_idx += 1;
            if !filter.is_none_or(|f| f == SheetFolderKind::Data) {
                continue;
            }
            let rows = s.archetype.row_count();
            let cols = s.archetype.column_count();
            let card = PortfolioCard {
                sheet_id: s.id.clone(),
                title: s.title.clone(),
                folder: SheetFolderKind::Data,
                badge_label: s.kind().label().to_owned(),
                subtitle: format!("{rows} × {cols}"),
                color_tag: s.color_tag,
                active: active == Some(&s.id),
                selected: selected.contains(&s.id),
                is_frozen: s.frozen,
                is_uninitialized: false,
                is_warning_or_stale: false,
                metric_count: rows,
                navigator_index: idx,
            };
            if matches_query(&card) {
                cards.push(card);
            }
        }
        for s in &doc.info_sheets {
            let idx = nav_idx;
            nav_idx += 1;
            if !filter.is_none_or(|f| f == SheetFolderKind::Info) {
                continue;
            }
            let constants_len = s.constants.len();
            let card = PortfolioCard {
                sheet_id: s.id.clone(),
                title: s.title.clone(),
                folder: SheetFolderKind::Info,
                badge_label: String::from("Info"),
                subtitle: format!("{constants_len} constants"),
                color_tag: s.color_tag,
                active: active == Some(&s.id),
                selected: selected.contains(&s.id),
                is_frozen: false,
                is_uninitialized: false,
                is_warning_or_stale: false,
                metric_count: constants_len,
                navigator_index: idx,
            };
            if matches_query(&card) {
                cards.push(card);
            }
        }
        for s in &doc.result_sheets {
            let idx = nav_idx;
            nav_idx += 1;
            if !filter.is_none_or(|f| f == SheetFolderKind::Results) {
                continue;
            }
            let tabs_len = s.tabs.len();
            let card = PortfolioCard {
                sheet_id: s.id.clone(),
                title: s.title().to_owned(),
                folder: SheetFolderKind::Results,
                badge_label: String::from("Result"),
                subtitle: format!("{tabs_len} tabs"),
                color_tag: s.color_tag,
                active: active == Some(&s.id),
                selected: selected.contains(&s.id),
                is_frozen: s.analyzable_data.as_ref().is_some_and(|ds| ds.frozen),
                is_uninitialized: false,
                is_warning_or_stale: result_is_warning_or_stale(s),
                metric_count: tabs_len,
                navigator_index: idx,
            };
            if matches_query(&card) {
                cards.push(card);
            }
        }
        for s in &doc.graph_sheets {
            let idx = nav_idx;
            nav_idx += 1;
            if !filter.is_none_or(|f| f == SheetFolderKind::Graphs) {
                continue;
            }
            let series_len = s.spec.series.len();
            let card = PortfolioCard {
                sheet_id: s.id.clone(),
                title: s.title.clone(),
                folder: SheetFolderKind::Graphs,
                badge_label: graph_kind_label(&s.spec).to_owned(),
                subtitle: if s.initialized { format!("{series_len} series") } else { String::from("Uninitialized") },
                color_tag: s.color_tag,
                active: active == Some(&s.id),
                selected: selected.contains(&s.id),
                is_frozen: false,
                is_uninitialized: !s.initialized,
                is_warning_or_stale: graph_is_warning_or_stale(doc, s),
                metric_count: series_len,
                navigator_index: idx,
            };
            if matches_query(&card) {
                cards.push(card);
            }
        }
        for s in &doc.layout_sheets {
            let idx = nav_idx;
            nav_idx += 1;
            if !filter.is_none_or(|f| f == SheetFolderKind::Layouts) {
                continue;
            }
            let items_len = s.spec.items.len();
            let card = PortfolioCard {
                sheet_id: s.id.clone(),
                title: s.title.clone(),
                folder: SheetFolderKind::Layouts,
                badge_label: String::from("Layout"),
                subtitle: format!("{items_len} items"),
                color_tag: s.color_tag,
                active: active == Some(&s.id),
                selected: selected.contains(&s.id),
                is_frozen: false,
                is_uninitialized: false,
                is_warning_or_stale: layout_is_warning_or_stale(doc, s),
                metric_count: items_len,
                navigator_index: idx,
            };
            if matches_query(&card) {
                cards.push(card);
            }
        }

        sort_portfolio_cards(&mut cards, sort_order);
        cards
    }

    /// Rebuild Portfolio Gallery cards from `doc`, pruning deleted sheet IDs from `selected_ids`.
    pub fn sync_gallery(&mut self, doc: &ProjectDocument) {
        let valid_ids: BTreeSet<SheetId> = doc
            .data_sheets
            .iter()
            .map(|s| s.id.clone())
            .chain(doc.info_sheets.iter().map(|s| s.id.clone()))
            .chain(doc.result_sheets.iter().map(|s| s.id.clone()))
            .chain(doc.graph_sheets.iter().map(|s| s.id.clone()))
            .chain(doc.layout_sheets.iter().map(|s| s.id.clone()))
            .collect();
        self.gallery.selected_ids.retain(|id| valid_ids.contains(id));

        self.gallery.cards =
            self.build_gallery_cards_with(doc, &self.gallery.search_query, self.gallery.sort_order, self.gallery.current_family_only);
    }

    /// Toggle collapsed state for a [`curvebook_doc::FloatingNote`] identifier and return the new collapsed flag.
    #[cfg(any(test, feature = "test-support"))]
    pub fn toggle_floating_note(&mut self, note_id: &str) -> bool {
        let collapsed = self.floating_notes_collapsed.entry(note_id.to_owned()).or_insert(false);
        *collapsed = !*collapsed;
        *collapsed
    }

    /// Return whether a [`curvebook_doc::FloatingNote`] is currently collapsed in the UI.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn is_floating_note_collapsed(&self, note_id: &str) -> bool {
        self.floating_notes_collapsed.get(note_id).copied().unwrap_or(false)
    }
}

impl WorkspaceNavState {
    /// Switch between [`NavigatorNestingMode::SeparateFolders`] and
    /// [`NavigatorNestingMode::NestedResultsUnderData`] and refresh from `doc`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_nesting_mode(&mut self, mode: NavigatorNestingMode, doc: &ProjectDocument) {
        self.nesting_mode = mode;
        self.sync_from_document(doc);
    }

    /// Update the live navigator search filter and refresh entries from `doc`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_search_query(&mut self, query: impl Into<String>, doc: &ProjectDocument) {
        self.search_query = query.into();
        self.sync_from_document(doc);
    }

    /// Enqueue a `sheet.select` intent for a click in the main Project Navigator without mutating `doc`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn click_navigator_entry(&mut self, doc: &ProjectDocument, sheet_id: &SheetId) -> Result<(), DocError> {
        if doc.sheet_section(sheet_id).is_none() {
            return Err(DocError::SheetNotFound { sheet_id: sheet_id.clone() });
        }
        self.pending_intents.push(ToolCommandIntent {
            command_id: String::from("sheet.select"),
            params: json!({ "sheet_id": sheet_id.as_str(), "mode": "navigator" }),
        });
        Ok(())
    }

    /// Enqueue a `sheet.select` intent for a click or double-click inside the Family pane without mutating `doc`:
    /// - Single-clicking a `Result`, `Graph`, `Layout`, or `Info` keeps `family_anchor_sheet_id` stable.
    /// - Double-clicking OR choosing a `Data` sheet shifts `family_anchor_sheet_id`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn click_family_entry(&mut self, doc: &ProjectDocument, sheet_id: &SheetId, double_click: bool) -> Result<(), DocError> {
        let Some(section) = doc.sheet_section(sheet_id) else {
            return Err(DocError::SheetNotFound { sheet_id: sheet_id.clone() });
        };
        let is_data = section == SheetSection::Data;
        let mode = if double_click || is_data { "navigator" } else { "family" };
        self.pending_intents
            .push(ToolCommandIntent { command_id: String::from("sheet.select"), params: json!({ "sheet_id": sheet_id.as_str(), "mode": mode }) });
        Ok(())
    }

    /// Borrow all matching entries for the active search filter.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn filtered_entries(&self) -> Vec<&NavigatorEntry> {
        self.entries.iter().collect()
    }

    /// Synchronize navigator folders, search results, transitive dependency highlighting, badges,
    /// and anchored Family-pane nodes from `doc`.
    pub fn sync_from_document(&mut self, doc: &ProjectDocument) {
        let upstream_map = build_upstream_map(doc);
        let downstream_map = build_downstream_map(&upstream_map);

        let (up_set, down_set) = match &doc.active_sheet_id {
            Some(active_id) => (collect_transitive(active_id, &upstream_map), collect_transitive(active_id, &downstream_map)),
            None => (BTreeSet::new(), BTreeSet::new()),
        };

        self.transitive_upstream_ids = up_set.iter().cloned().collect();
        self.transitive_downstream_ids = down_set.iter().cloned().collect();

        let raw_entries = build_raw_entries(doc, &up_set, &down_set);
        let ordered_all = order_entries_by_nesting(doc, raw_entries, self.nesting_mode);

        let needle = self.search_query.trim().to_lowercase();
        self.all_entries = ordered_all;
        self.entries = self.all_entries.iter().filter(|e| matches_search(e, &needle)).cloned().collect();
        self.folders = build_folders(&self.entries, self.nesting_mode, &self.collapsed_sections);

        self.family_anchor_sheet_id = doc.family_anchor_sheet_id.clone();
        self.sync_family_pane(doc, &upstream_map, &downstream_map);
        self.sync_gallery(doc);
    }

    fn sync_family_pane(
        &mut self,
        doc: &ProjectDocument,
        upstream_map: &BTreeMap<SheetId, Vec<SheetId>>,
        downstream_map: &BTreeMap<SheetId, Vec<SheetId>>,
    ) {
        self.family_nodes.clear();
        self.family_related_ids.clear();

        let Some(anchor) = &doc.family_anchor_sheet_id else {
            return;
        };
        if doc.sheet_section(anchor).is_none() {
            return;
        }

        let up_dist = bfs_distances(anchor, upstream_map);
        let down_dist = bfs_distances(anchor, downstream_map);

        for entry in &self.all_entries {
            let relation_and_dist = if &entry.sheet_id == anchor {
                Some((FamilyRelation::Anchor, 0))
            } else if let Some(&d) = up_dist.get(&entry.sheet_id) {
                Some((FamilyRelation::Upstream, d))
            } else {
                down_dist.get(&entry.sheet_id).map(|&d| (FamilyRelation::Downstream, d))
            };

            if let Some((relation, hop_distance)) = relation_and_dist {
                if relation != FamilyRelation::Anchor {
                    self.family_related_ids.push(entry.sheet_id.clone());
                }
                self.family_nodes.push(FamilyPaneNode {
                    sheet_id: entry.sheet_id.clone(),
                    section: entry.section,
                    title: entry.title.clone(),
                    subtitle: entry.subtitle.clone(),
                    has_duplicate_title: entry.has_duplicate_title,
                    relation,
                    hop_distance,
                    is_active: entry.is_active,
                    is_anchor: relation == FamilyRelation::Anchor,
                    is_frozen: entry.is_frozen,
                    is_dirty: entry.is_dirty,
                });
            }
        }
    }
}

fn default_empty_folders() -> Vec<NavigatorFolder> {
    [
        (SheetSection::Data, "Data Tables"),
        (SheetSection::Info, "Info"),
        (SheetSection::Analyses, "Results"),
        (SheetSection::Graphs, "Graphs"),
        (SheetSection::Layouts, "Layouts"),
    ]
    .into_iter()
    .map(|(section, label)| NavigatorFolder { section, label: String::from(label), collapsed: false, entries: Vec::new() })
    .collect()
}

fn section_label(section: SheetSection) -> &'static str {
    match section {
        SheetSection::Data => "Data Tables",
        SheetSection::Info => "Info",
        SheetSection::Analyses => "Results",
        SheetSection::Graphs => "Graphs",
        SheetSection::Layouts => "Layouts",
    }
}

fn result_is_warning_or_stale(rs: &ResultSheet) -> bool {
    matches!(rs.status, ResultStatus::Dirty | ResultStatus::Failed { .. })
        || rs.checklist.iter().any(|c| matches!(c.status, ChecklistStatus::Warn | ChecklistStatus::Violated))
}

fn graph_is_warning_or_stale(doc: &ProjectDocument, gs: &GraphSheet) -> bool {
    if !gs.initialized {
        return false;
    }
    let missing_source = doc.data_sheet(&gs.spec.primary_source_sheet_id).is_none() && doc.result_sheet(&gs.spec.primary_source_sheet_id).is_none();
    let stale_overlay = gs.spec.series.iter().any(|s| {
        s.regression_overlay.as_ref().is_some_and(|ro| {
            doc.result_sheet(&ro.result_sheet_id).is_none_or(|r| matches!(r.status, ResultStatus::Dirty | ResultStatus::Failed { .. }))
        })
    });
    missing_source
        || stale_overlay
        || (gs.spec.series.is_empty()
            && gs.spec.annotations.is_empty()
            && !matches!(gs.spec.kind, GraphKind::Diagnostic(_) | GraphKind::MultipleVariables))
}

fn layout_is_warning_or_stale(doc: &ProjectDocument, ls: &curvebook_doc::LayoutSheet) -> bool {
    ls.spec.items.iter().any(|item| match item {
        LayoutItem::LiveGraph { graph_sheet_id, .. } => {
            doc.graph_sheet(graph_sheet_id).is_none_or(|gs| !gs.initialized || graph_is_warning_or_stale(doc, gs))
        }
        LayoutItem::LiveTable { source_sheet_id, .. } => {
            doc.data_sheet(source_sheet_id).is_none()
                && doc.info_sheet(source_sheet_id).is_none()
                && doc.result_sheet(source_sheet_id).is_none_or(result_is_warning_or_stale)
        }
        LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => false,
    })
}

const fn type_folder_rank(folder: SheetFolderKind) -> usize {
    match folder {
        SheetFolderKind::Graphs => 0,
        SheetFolderKind::Layouts => 1,
        SheetFolderKind::Data => 2,
        SheetFolderKind::Results => 3,
        SheetFolderKind::Info => 4,
    }
}

fn sort_portfolio_cards(cards: &mut [PortfolioCard], sort_order: GallerySortOrder) {
    match sort_order {
        GallerySortOrder::NavigatorOrder => cards.sort_by_key(|c| c.navigator_index),
        GallerySortOrder::NameAsc => {
            cards.sort_by(|a, b| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()).then(a.navigator_index.cmp(&b.navigator_index)))
        }
        GallerySortOrder::Type => cards.sort_by(|a, b| {
            type_folder_rank(a.folder)
                .cmp(&type_folder_rank(b.folder))
                .then_with(|| a.badge_label.to_ascii_lowercase().cmp(&b.badge_label.to_ascii_lowercase()))
                .then_with(|| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()))
                .then(a.navigator_index.cmp(&b.navigator_index))
        }),
        GallerySortOrder::SeriesOrRows => {
            cards.sort_by(|a, b| {
                b.metric_count
                    .cmp(&a.metric_count)
                    .then_with(|| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()))
                    .then(a.navigator_index.cmp(&b.navigator_index))
            });
        }
    }
}

fn build_upstream_map(doc: &ProjectDocument) -> BTreeMap<SheetId, Vec<SheetId>> {
    let mut map = BTreeMap::new();
    for s in &doc.data_sheets {
        map.insert(s.id.clone(), doc.upstream_dependencies(&s.id));
    }
    for s in &doc.info_sheets {
        map.insert(s.id.clone(), Vec::new());
    }
    for s in &doc.result_sheets {
        map.insert(s.id.clone(), doc.upstream_dependencies(&s.id));
    }
    for s in &doc.graph_sheets {
        map.insert(s.id.clone(), doc.upstream_dependencies(&s.id));
    }
    for s in &doc.layout_sheets {
        map.insert(s.id.clone(), doc.upstream_dependencies(&s.id));
    }
    map
}

fn build_downstream_map(upstream_map: &BTreeMap<SheetId, Vec<SheetId>>) -> BTreeMap<SheetId, Vec<SheetId>> {
    let mut downstream: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    for (sheet_id, deps) in upstream_map {
        for dep in deps {
            downstream.entry(dep.clone()).or_default().push(sheet_id.clone());
        }
    }
    for list in downstream.values_mut() {
        list.sort();
        list.dedup();
    }
    downstream
}

fn collect_transitive(start: &SheetId, adj: &BTreeMap<SheetId, Vec<SheetId>>) -> BTreeSet<SheetId> {
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(start.clone());
    while let Some(curr) = queue.pop_front() {
        if let Some(neighbors) = adj.get(&curr) {
            for next in neighbors {
                if next != start && visited.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
    }
    visited
}

/// Collect all downstream dependent sheet IDs transitively relying on `root_id`.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub(crate) fn collect_downstream_dependents(doc: &ProjectDocument, root_id: &SheetId) -> Vec<SheetId> {
    let upstream = build_upstream_map(doc);
    let downstream = build_downstream_map(&upstream);
    collect_transitive(root_id, &downstream).into_iter().collect()
}

fn bfs_distances(start: &SheetId, adj: &BTreeMap<SheetId, Vec<SheetId>>) -> BTreeMap<SheetId, usize> {
    let mut dist = BTreeMap::new();
    let mut queue = VecDeque::new();
    queue.push_back((start.clone(), 0usize));
    while let Some((curr, d)) = queue.pop_front() {
        if let Some(neighbors) = adj.get(&curr) {
            for next in neighbors {
                if next != start && !dist.contains_key(next) {
                    dist.insert(next.clone(), d + 1);
                    queue.push_back((next.clone(), d + 1));
                }
            }
        }
    }
    dist
}

fn lookup_doc_sheet_title<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Option<&'a str> {
    if let Some(ds) = doc.data_sheet(id) {
        Some(&ds.title)
    } else if let Some(info) = doc.info_sheet(id) {
        Some(&info.title)
    } else if let Some(rs) = doc.result_sheet(id) {
        Some(rs.title())
    } else if let Some(gs) = doc.graph_sheet(id) {
        Some(&gs.title)
    } else if let Some(ls) = doc.layout_sheet(id) {
        Some(&ls.title)
    } else {
        None
    }
}

fn graph_kind_label(spec: &GraphSpec) -> &'static str {
    match spec.kind {
        GraphKind::Xy => "XY",
        GraphKind::Column => match spec.series.first().map(|s| &s.gdo) {
            Some(GraphicalDataObject::Violin { .. }) => "Violin",
            Some(GraphicalDataObject::BoxAndWhisker { .. }) => "Box Plot",
            Some(GraphicalDataObject::AlignedDotPlot { .. }) => "Dot Plot",
            Some(GraphicalDataObject::FloatingBar { .. }) => "Floating Bar",
            Some(GraphicalDataObject::BeforeAfterLines { .. }) => "Before\u{2013}After",
            Some(GraphicalDataObject::BarWithReplicates { .. }) => "Bar",
            _ => "Column",
        },
        GraphKind::Grouped => "Grouped",
        GraphKind::Contingency => "Contingency",
        GraphKind::PartsOfWhole => "Parts of Whole",
        GraphKind::Survival => "Survival",
        GraphKind::MultipleVariables => "Multivariate",
        GraphKind::Histogram => "Histogram",
        GraphKind::Diagnostic(diag) => match diag {
            DiagnosticPlotKind::Heatmap => "Heatmap",
            DiagnosticPlotKind::Volcano => "Volcano",
            DiagnosticPlotKind::BlandAltman => "Bland\u{2013}Altman",
            DiagnosticPlotKind::RocCurve => "ROC Curve",
            DiagnosticPlotKind::ForestPlot => "Forest Plot",
            DiagnosticPlotKind::ResidualPlot => "Residual Plot",
            DiagnosticPlotKind::NormalQqPlot => "Normal Q\u{2013}Q",
            DiagnosticPlotKind::HomoscedasticityPlot => "Homoscedasticity",
            DiagnosticPlotKind::ActualVsPredictedPlot => "Actual vs. Predicted",
            DiagnosticPlotKind::ResidualVsRowPlot => "Residual vs. Row",
            DiagnosticPlotKind::GapStatisticPlot => "Gap Statistic",
            DiagnosticPlotKind::ParameterCorrelationHeatmap => "Parameter Correlation",
            DiagnosticPlotKind::LognormalQqPlot => "Lognormal Q\u{2013}Q",
            DiagnosticPlotKind::Dendrogram => "Dendrogram",
            DiagnosticPlotKind::PcaBiplot => "PCA Biplot",
        },
    }
}

fn derive_data_subtitle(doc: &ProjectDocument, s: &DataSheet) -> Option<String> {
    for region in &s.linked_regions {
        match &region.spec.source {
            LinkSource::InternalSheet { sheet_id, .. } => {
                let src_title = lookup_doc_sheet_title(doc, sheet_id).unwrap_or(sheet_id.as_str());
                return Some(format!("Linked \u{2190} {src_title}"));
            }
            LinkSource::ExternalDelimitedFile { path, .. } => {
                let file_name = path.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or(path.as_str());
                return Some(format!("Linked \u{2190} {file_name}"));
            }
            LinkSource::Clipboard => {}
        }
    }
    Some(format!("{} \u{00B7} {}\u{00D7}{}", s.kind().label(), s.archetype.row_count(), s.archetype.column_count()))
}

fn derive_info_subtitle(doc: &ProjectDocument, s: &curvebook_doc::InfoSheet) -> Option<String> {
    let n_const = s.constants.len();
    let const_lbl = if n_const == 1 { String::from("1 constant") } else { format!("{n_const} constants") };
    let paired: Vec<&str> = s.linked_data_sheets.iter().filter_map(|id| lookup_doc_sheet_title(doc, id)).collect();
    if paired.is_empty() { Some(const_lbl) } else { Some(format!("Paired: {} \u{00B7} {const_lbl}", paired.join(", "))) }
}

fn derive_result_subtitle(doc: &ProjectDocument, s: &ResultSheet) -> Option<String> {
    let short_kind = s.analysis.kind.descriptor().short_label;
    let Some(src_id) = s.analysis.source_sheet_id.as_ref() else {
        return if s.is_imported() { Some(format!("Imported \u{00B7} {short_kind}")) } else { Some(String::from(short_kind)) };
    };
    let src_title = lookup_doc_sheet_title(doc, src_id).unwrap_or(src_id.as_str());
    if s.analysis.custom_title.is_some() { Some(format!("From {src_title} \u{00B7} {short_kind}")) } else { Some(format!("From {src_title}")) }
}

fn derive_graph_subtitle(doc: &ProjectDocument, gs: &GraphSheet) -> Option<String> {
    let base_kind = graph_kind_label(&gs.spec);
    let linked_result_id = gs
        .spec
        .series
        .iter()
        .find_map(|ser| ser.regression_overlay.as_ref().map(|ov| &ov.result_sheet_id))
        .or_else(|| gs.spec.comparisons.first().map(|c| &c.result_sheet_id))
        .or_else(|| gs.spec.compact_letter_display.as_ref().map(|c| &c.result_sheet_id));
    let kind_part = match linked_result_id.and_then(|rid| doc.result_sheet(rid)) {
        Some(rs) => format!("{base_kind} + {}", rs.analysis.kind.descriptor().short_label),
        None => String::from(base_kind),
    };
    let src_title = lookup_doc_sheet_title(doc, &gs.spec.primary_source_sheet_id).map(str::trim).filter(|t| !t.is_empty());
    let norm_title = gs.title.trim().to_lowercase();
    let norm_src = src_title.map(str::to_lowercase);
    let norm_kind = kind_part.to_lowercase();

    match (src_title, norm_src.as_deref() == Some(norm_title.as_str()), norm_kind == norm_title) {
        (Some(src), false, false) => Some(format!("{src} \u{00B7} {kind_part}")),
        (Some(src), false, true) => Some(format!("From {src}")),
        (_, true, false) | (None, _, false) => Some(kind_part),
        (Some(src), true, true) => Some(format!("From {src}")),
        (None, _, true) => None,
    }
}

fn derive_layout_subtitle(ls: &curvebook_doc::LayoutSheet) -> Option<String> {
    let n = ls.spec.items.len();
    Some(if n == 1 { String::from("1 panel") } else { format!("{n} panels") })
}

fn collision_hint_suffix(doc: &ProjectDocument, section: SheetSection, id: &SheetId) -> Option<String> {
    match section {
        SheetSection::Analyses => doc.result_sheet(id).and_then(|rs| {
            (!rs.analysis.selected_data_sets.is_empty()).then(|| {
                let cols: Vec<String> = rs.analysis.selected_data_sets.iter().map(|i| format!("{}", i + 1)).collect();
                format!("cols {}", cols.join(", "))
            })
        }),
        SheetSection::Graphs => doc.graph_sheet(id).map(|gs| {
            let n = gs.spec.series.len();
            if n == 1 { String::from("1 series") } else { format!("{n} series") }
        }),
        SheetSection::Layouts => doc.layout_sheet(id).map(|ls| format!("{}x{}", ls.spec.width, ls.spec.height)),
        SheetSection::Data | SheetSection::Info => None,
    }
}

const fn section_ord(section: SheetSection) -> u8 {
    match section {
        SheetSection::Data => 0,
        SheetSection::Info => 1,
        SheetSection::Analyses => 2,
        SheetSection::Graphs => 3,
        SheetSection::Layouts => 4,
    }
}

/// Compute per-sheet disambiguation metadata (`subtitle` and `has_duplicate_title`) across all sections (`R20.12`).
#[must_use]
pub fn compute_sheet_disambiguation(doc: &ProjectDocument) -> BTreeMap<SheetId, SheetDisambiguation> {
    let mut title_counts: BTreeMap<(u8, String), usize> = BTreeMap::new();
    let mut rows: Vec<(SheetId, SheetSection, String, Option<String>)> = Vec::with_capacity(doc.total_sheet_count());

    let mut push_row = |id: &SheetId, section: SheetSection, title: &str, subtitle: Option<String>| {
        let norm_title = title.trim().to_lowercase();
        *title_counts.entry((section_ord(section), norm_title.clone())).or_insert(0) += 1;
        rows.push((id.clone(), section, norm_title, subtitle));
    };

    for s in &doc.data_sheets {
        push_row(&s.id, SheetSection::Data, &s.title, derive_data_subtitle(doc, s));
    }
    for s in &doc.info_sheets {
        push_row(&s.id, SheetSection::Info, &s.title, derive_info_subtitle(doc, s));
    }
    for s in &doc.result_sheets {
        push_row(&s.id, SheetSection::Analyses, s.title(), derive_result_subtitle(doc, s));
    }
    for s in &doc.graph_sheets {
        push_row(&s.id, SheetSection::Graphs, &s.title, derive_graph_subtitle(doc, s));
    }
    for s in &doc.layout_sheets {
        push_row(&s.id, SheetSection::Layouts, &s.title, derive_layout_subtitle(s));
    }

    let mut out: BTreeMap<SheetId, SheetDisambiguation> = BTreeMap::new();
    let mut collision_groups: BTreeMap<(u8, String, String), (SheetSection, Vec<SheetId>)> = BTreeMap::new();
    for (id, section, norm_title, subtitle) in rows {
        let sec_k = section_ord(section);
        let has_dup = title_counts.get(&(sec_k, norm_title.clone())).copied().unwrap_or(0) >= 2;
        if has_dup {
            let sub_key = subtitle.as_deref().unwrap_or("").to_lowercase();
            collision_groups.entry((sec_k, norm_title, sub_key)).or_insert_with(|| (section, Vec::new())).1.push(id.clone());
        }
        out.insert(id, SheetDisambiguation { subtitle, has_duplicate_title: has_dup });
    }

    // Break residual (section, title, subtitle) collisions deterministically without showing raw SheetIds.
    for (_, (section, group)) in collision_groups {
        if group.len() < 2 {
            continue;
        }
        let hints: Vec<Option<String>> = group.iter().map(|id| collision_hint_suffix(doc, section, id)).collect();
        let unique_hints: BTreeSet<&str> = hints.iter().filter_map(|h| h.as_deref()).collect();
        let hints_all_distinct = hints.iter().all(Option::is_some) && unique_hints.len() == group.len();
        for (idx, id) in group.iter().enumerate() {
            if let Some(entry) = out.get_mut(id) {
                let suffix = if hints_all_distinct { hints[idx].clone().unwrap_or_default() } else { format!("#{}", idx + 1) };
                entry.subtitle = Some(match &entry.subtitle {
                    Some(base) if !base.is_empty() => format!("{base} \u{00B7} {suffix}"),
                    _ => suffix,
                });
            }
        }
    }
    out
}

fn build_raw_entries(doc: &ProjectDocument, up_set: &BTreeSet<SheetId>, down_set: &BTreeSet<SheetId>) -> Vec<NavigatorEntry> {
    let mut entries = Vec::with_capacity(doc.total_sheet_count());
    let active = doc.active_sheet_id.as_ref();
    let ctx = FamilyHighlightCtx { active, up_set, down_set };
    let disambig = compute_sheet_disambiguation(doc);

    for s in &doc.data_sheets {
        entries.push(build_data_entry(s, active, up_set, down_set, disambig.get(&s.id)));
    }
    for s in &doc.info_sheets {
        entries.push(build_simple_entry(
            SimpleSheetSummary {
                id: &s.id,
                section: SheetSection::Info,
                title: &s.title,
                disambig: disambig.get(&s.id),
                color_tag: s.color_tag,
                floating_note_count: s.floating_notes.len(),
                origin: s.origin.as_ref(),
            },
            &ctx,
        ));
    }
    for s in &doc.result_sheets {
        entries.push(build_result_entry(s, active, up_set, down_set, disambig.get(&s.id)));
    }
    for s in &doc.graph_sheets {
        entries.push(build_simple_entry(
            SimpleSheetSummary {
                id: &s.id,
                section: SheetSection::Graphs,
                title: &s.title,
                disambig: disambig.get(&s.id),
                color_tag: s.color_tag,
                floating_note_count: s.floating_notes.len(),
                origin: s.origin.as_ref(),
            },
            &ctx,
        ));
    }
    for s in &doc.layout_sheets {
        entries.push(build_simple_entry(
            SimpleSheetSummary {
                id: &s.id,
                section: SheetSection::Layouts,
                title: &s.title,
                disambig: disambig.get(&s.id),
                color_tag: s.color_tag,
                floating_note_count: s.floating_notes.len(),
                origin: s.origin.as_ref(),
            },
            &ctx,
        ));
    }
    entries
}

struct FamilyHighlightCtx<'a> {
    active: Option<&'a SheetId>,
    up_set: &'a BTreeSet<SheetId>,
    down_set: &'a BTreeSet<SheetId>,
}

struct SimpleSheetSummary<'a> {
    id: &'a SheetId,
    section: SheetSection,
    title: &'a str,
    disambig: Option<&'a SheetDisambiguation>,
    color_tag: Option<Rgba8>,
    floating_note_count: usize,
    origin: Option<&'a SheetOrigin>,
}

fn build_data_entry(
    s: &DataSheet,
    active: Option<&SheetId>,
    up_set: &BTreeSet<SheetId>,
    down_set: &BTreeSet<SheetId>,
    disambig: Option<&SheetDisambiguation>,
) -> NavigatorEntry {
    let is_up = up_set.contains(&s.id);
    let is_down = down_set.contains(&s.id);
    let mut link_colors = Vec::new();
    let mut badges = Vec::new();

    if s.frozen {
        badges.push(NavigatorBadge::new(NavigatorBadgeKind::Frozen, NavigatorBadgeColor::Charcoal, "Frozen"));
    }

    for region in &s.linked_regions {
        if let Some(color) = region.border_color() {
            if !link_colors.contains(&color) {
                link_colors.push(color);
            }
            let badge = match color {
                LinkBorderColor::Blue => NavigatorBadge::new(NavigatorBadgeKind::InternalTableLink, NavigatorBadgeColor::Blue, "Live Table Link"),
                LinkBorderColor::Green => {
                    NavigatorBadge::new(NavigatorBadgeKind::AnalyzableResultLink, NavigatorBadgeColor::Green, "Live Result Link")
                }
                LinkBorderColor::Red => NavigatorBadge::new(NavigatorBadgeKind::TerminalResultLink, NavigatorBadgeColor::Red, "Terminal Result Link"),
                LinkBorderColor::Charcoal => NavigatorBadge::new(NavigatorBadgeKind::LiveFileLink, NavigatorBadgeColor::Charcoal, "Live File Link"),
            };
            if !badges.iter().any(|b| b.kind == badge.kind) {
                badges.push(badge);
            }
        }
    }

    NavigatorEntry {
        sheet_id: s.id.clone(),
        section: SheetSection::Data,
        title: s.title.clone(),
        subtitle: disambig.and_then(|d| d.subtitle.clone()),
        has_duplicate_title: disambig.is_some_and(|d| d.has_duplicate_title),
        is_active: active == Some(&s.id),
        is_bold: active == Some(&s.id) || is_up || is_down,
        is_upstream_dependency: is_up,
        is_downstream_dependent: is_down,
        is_frozen: s.frozen,
        is_dirty: false,
        archetype_label: Some(String::from(s.kind().label())),
        parent_sheet_id: None,
        nesting_depth: 0,
        color_tag: s.color_tag,
        link_colors,
        badges,
        floating_note_count: s.floating_notes.len(),
        origin: s.origin.clone(),
    }
}

fn build_result_entry(
    s: &ResultSheet,
    active: Option<&SheetId>,
    up_set: &BTreeSet<SheetId>,
    down_set: &BTreeSet<SheetId>,
    disambig: Option<&SheetDisambiguation>,
) -> NavigatorEntry {
    let is_up = up_set.contains(&s.id);
    let is_down = down_set.contains(&s.id);
    let is_dirty = matches!(s.status, ResultStatus::Dirty | ResultStatus::ImportedStale | ResultStatus::Failed { .. });
    let mut badges = Vec::new();
    match &s.status {
        ResultStatus::Dirty => {
            badges.push(NavigatorBadge::new(NavigatorBadgeKind::Dirty, NavigatorBadgeColor::Red, "Dirty"));
        }
        ResultStatus::Failed { .. } => {
            badges.push(NavigatorBadge::new(NavigatorBadgeKind::Failed, NavigatorBadgeColor::Red, "Failed"));
        }
        ResultStatus::Imported => {
            badges.push(NavigatorBadge::new(NavigatorBadgeKind::Dirty, NavigatorBadgeColor::Blue, "Imported"));
        }
        ResultStatus::ImportedStale => {
            badges.push(NavigatorBadge::new(NavigatorBadgeKind::Dirty, NavigatorBadgeColor::Red, "Imported (stale)"));
        }
        ResultStatus::UpToDate => {}
    }

    NavigatorEntry {
        sheet_id: s.id.clone(),
        section: SheetSection::Analyses,
        title: s.title().to_owned(),
        subtitle: disambig.and_then(|d| d.subtitle.clone()),
        has_duplicate_title: disambig.is_some_and(|d| d.has_duplicate_title),
        is_active: active == Some(&s.id),
        is_bold: active == Some(&s.id) || is_up || is_down,
        is_upstream_dependency: is_up,
        is_downstream_dependent: is_down,
        is_frozen: false,
        is_dirty,
        archetype_label: None,
        parent_sheet_id: None,
        nesting_depth: 0,
        color_tag: s.color_tag,
        link_colors: Vec::new(),
        badges,
        floating_note_count: s.floating_notes.len(),
        origin: s.origin.clone(),
    }
}

fn build_simple_entry(summary: SimpleSheetSummary<'_>, ctx: &FamilyHighlightCtx<'_>) -> NavigatorEntry {
    let is_up = ctx.up_set.contains(summary.id);
    let is_down = ctx.down_set.contains(summary.id);
    let is_active = ctx.active == Some(summary.id);
    NavigatorEntry {
        sheet_id: summary.id.clone(),
        section: summary.section,
        title: summary.title.to_owned(),
        subtitle: summary.disambig.and_then(|d| d.subtitle.clone()),
        has_duplicate_title: summary.disambig.is_some_and(|d| d.has_duplicate_title),
        is_active,
        is_bold: is_active || is_up || is_down,
        is_upstream_dependency: is_up,
        is_downstream_dependent: is_down,
        is_frozen: false,
        is_dirty: false,
        archetype_label: None,
        parent_sheet_id: None,
        nesting_depth: 0,
        color_tag: summary.color_tag,
        link_colors: Vec::new(),
        badges: Vec::new(),
        floating_note_count: summary.floating_note_count,
        origin: summary.origin.cloned(),
    }
}

fn order_entries_by_nesting(doc: &ProjectDocument, raw: Vec<NavigatorEntry>, mode: NavigatorNestingMode) -> Vec<NavigatorEntry> {
    if mode == NavigatorNestingMode::SeparateFolders {
        return raw;
    }

    let mut result_entries: BTreeMap<SheetId, NavigatorEntry> = BTreeMap::new();
    let mut non_results = Vec::with_capacity(raw.len());
    for entry in raw {
        if entry.section == SheetSection::Analyses {
            result_entries.insert(entry.sheet_id.clone(), entry);
        } else {
            non_results.push(entry);
        }
    }

    let mut children_by_parent: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    for rs in &doc.result_sheets {
        if let Some(src_id) = &rs.analysis.source_sheet_id {
            children_by_parent.entry(src_id.clone()).or_default().push(rs.id.clone());
        }
    }

    let mut placed = BTreeSet::new();
    let mut ordered = Vec::with_capacity(non_results.len() + result_entries.len());

    for entry in non_results {
        let is_data = entry.section == SheetSection::Data;
        let root_id = entry.sheet_id.clone();
        ordered.push(entry);
        if is_data {
            append_nested_results(&root_id, 1, &children_by_parent, &result_entries, &mut placed, &mut ordered);
        }
    }

    for rs in &doc.result_sheets {
        if !placed.contains(&rs.id)
            && let Some(entry) = result_entries.get(&rs.id)
        {
            ordered.push(entry.clone());
        }
    }

    ordered
}

fn append_nested_results(
    parent_id: &SheetId,
    depth: usize,
    children_by_parent: &BTreeMap<SheetId, Vec<SheetId>>,
    result_entries: &BTreeMap<SheetId, NavigatorEntry>,
    placed: &mut BTreeSet<SheetId>,
    ordered: &mut Vec<NavigatorEntry>,
) {
    if depth > result_entries.len() + 1 {
        return;
    }
    let Some(children) = children_by_parent.get(parent_id) else {
        return;
    };
    for child_id in children {
        if placed.contains(child_id) {
            continue;
        }
        if let Some(entry) = result_entries.get(child_id) {
            placed.insert(child_id.clone());
            let mut nested = entry.clone();
            nested.parent_sheet_id = Some(parent_id.clone());
            nested.nesting_depth = depth;
            ordered.push(nested);
            append_nested_results(child_id, depth + 1, children_by_parent, result_entries, placed, ordered);
        }
    }
}

fn matches_search(entry: &NavigatorEntry, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    entry.title.to_lowercase().contains(needle_lower)
        || entry.sheet_id.as_str().to_lowercase().contains(needle_lower)
        || entry.archetype_label.as_deref().is_some_and(|lbl| lbl.to_lowercase().contains(needle_lower))
        || entry.subtitle.as_deref().is_some_and(|sub| sub.to_lowercase().contains(needle_lower))
}

fn build_folders(entries: &[NavigatorEntry], mode: NavigatorNestingMode, collapsed: &[SheetSection]) -> Vec<NavigatorFolder> {
    let sections = [SheetSection::Data, SheetSection::Info, SheetSection::Analyses, SheetSection::Graphs, SheetSection::Layouts];
    let mut folders: Vec<NavigatorFolder> = sections
        .into_iter()
        .map(|section| NavigatorFolder {
            section,
            label: String::from(section_label(section)),
            collapsed: collapsed.contains(&section),
            entries: Vec::new(),
        })
        .collect();

    for entry in entries {
        let target_section =
            if mode == NavigatorNestingMode::NestedResultsUnderData && entry.section == SheetSection::Analyses && entry.parent_sheet_id.is_some() {
                SheetSection::Data
            } else {
                entry.section
            };
        if let Some(folder) = folders.iter_mut().find(|f| f.section == target_section) {
            folder.entries.push(entry.clone());
        }
    }
    folders
}

// ============================================================================
// Clean-room built-in tutorial dataset builders
// ============================================================================

fn build_dose_response_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 6)?;
    xy.x_title = String::from("Log [Inhibitor] (M)");
    xy.data_sets[0].title = String::from("Control");
    xy.data_sets[1].title = String::from("Mutant");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ys_ctrl = [[99.0, 97.5, 101.2], [93.0, 95.2, 91.8], [72.0, 74.5, 70.1], [31.0, 29.4, 33.2], [8.5, 9.2, 7.8], [2.1, 1.9, 2.5]];
    let ys_mut = [[100.0, 98.0, 101.0], [97.0, 96.5, 98.1], [86.0, 88.2, 84.5], [58.0, 55.3, 60.4], [24.0, 22.8, 25.1], [6.0, 5.8, 6.4]];
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        for s in 0..3 {
            xy.data_sets[0].cells[r][s] = Cell::number(ys_ctrl[r][s]);
            xy.data_sets[1].cells[r][s] = Cell::number(ys_mut[r][s]);
        }
    }
    DataSheet::new(sheet_id, title, TableArchetype::Xy(xy))
}

fn build_unpaired_column_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5)?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Compound A");
    col.data_sets[2].title = String::from("Compound B");
    let vals = [[12.1, 13.4, 11.8, 12.9, 12.5], [18.2, 19.1, 17.5, 18.8, 19.4], [24.0, 25.3, 23.6, 24.8, 25.1]];
    for (c, series) in vals.iter().enumerate() {
        for (r, &v) in series.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(sheet_id, title, TableArchetype::Column(col))
}

fn build_paired_column_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 5)?;
    col.data_sets[0].title = String::from("Baseline");
    col.data_sets[1].title = String::from("Post-Dose");
    let before = [102.0, 98.5, 110.2, 95.0, 104.3];
    let after = [88.4, 84.0, 92.1, 82.5, 89.0];
    for r in 0..5 {
        col.row_titles[r] = format!("Subject {}", r + 1);
        col.data_sets[0].cells[r][0] = Cell::number(before[r]);
        col.data_sets[1].cells[r][0] = Cell::number(after[r]);
    }
    DataSheet::new(sheet_id, title, TableArchetype::Column(col))
}

fn build_grouped_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 3)?;
    grp.row_factor_title = String::from("Time (h)");
    grp.column_factor_title = String::from("Genotype");
    grp.row_titles = vec![String::from("0 h"), String::from("4 h"), String::from("24 h")];
    grp.data_sets[0].title = String::from("Wildtype");
    grp.data_sets[1].title = String::from("Knockout");
    for r in 0..3 {
        for s in 0..3 {
            grp.data_sets[0].cells[r][s] = Cell::number(10.0 + (r as f64) * 5.0 + (s as f64) * 0.4);
            grp.data_sets[1].cells[r][s] = Cell::number(10.0 + (r as f64) * 2.5 + (s as f64) * 0.5);
        }
    }
    DataSheet::new(sheet_id, title, TableArchetype::Grouped(grp))
}

fn build_contingency_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut cont = ContingencyTable::new(2, 2)?;
    cont.row_titles = vec![String::from("Responder"), String::from("Non-Responder")];
    cont.data_sets[0].title = String::from("Placebo");
    cont.data_sets[1].title = String::from("Active");
    cont.data_sets[0].cells[0][0] = Cell::number(14.0);
    cont.data_sets[0].cells[1][0] = Cell::number(36.0);
    cont.data_sets[1].cells[0][0] = Cell::number(31.0);
    cont.data_sets[1].cells[1][0] = Cell::number(19.0);
    DataSheet::new(sheet_id, title, TableArchetype::Contingency(cont))
}

fn build_survival_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut surv = SurvivalTable::new(2, 6)?;
    surv.cohorts[0].title = String::from("Standard Care");
    surv.cohorts[1].title = String::from("Experimental");
    let times = [12.0, 18.0, 24.0, 30.0, 36.0, 48.0];
    let c0 = [1.0, 1.0, 0.0, 1.0, 1.0, 0.0];
    let c1 = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    for r in 0..6 {
        surv.times[r] = Cell::number(times[r]);
        surv.cohorts[0].cells[r][0] = Cell::number(c0[r]);
        surv.cohorts[1].cells[r][0] = Cell::number(c1[r]);
    }
    DataSheet::new(sheet_id, title, TableArchetype::Survival(surv))
}

fn build_parts_of_whole_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut pow = PartsOfWholeTable::new(1, 4)?;
    pow.slice_labels = vec![String::from("G0/G1"), String::from("S"), String::from("G2/M"), String::from("Sub-G1")];
    pow.data_sets[0].title = String::from("Population %");
    for (r, &v) in [54.2, 24.8, 17.5, 3.5].iter().enumerate() {
        pow.data_sets[0].cells[r][0] = Cell::number(v);
    }
    DataSheet::new(sheet_id, title, TableArchetype::PartsOfWhole(pow))
}

fn build_multivar_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut mv = MultipleVariablesTable::new(4, 6)?;
    mv.variables[0].title = String::from("Cohort");
    mv.variables[1].title = String::from("Body Mass");
    mv.variables[2].title = String::from("Height_m");
    mv.variables[3].title = String::from("BMI");
    let cohorts = ["Control", "Control", "Treated", "Treated", "Control", "Treated"];
    let masses = [68.0, 74.5, 81.2, 65.0, 72.0, 79.4];
    let heights = [1.72, 1.78, 1.80, 1.66, 1.75, 1.82];
    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::text(cohorts[r]);
        mv.variables[1].cells[r] = Cell::number(masses[r]);
        mv.variables[2].cells[r] = Cell::number(heights[r]);
        mv.variables[3].cells[r] = Cell::number(masses[r] / (heights[r] * heights[r]));
    }
    mv.variables[0].refresh_inferred_role();
    mv.variables[1].refresh_inferred_role();
    mv.variables[2].refresh_inferred_role();
    mv.variables[3].set_calculated_formula("= [Body Mass] / ([Height_m]^2)");
    DataSheet::new(sheet_id, title, TableArchetype::MultipleVariables(mv))
}

fn build_nested_fixture(sheet_id: SheetId, title: &str) -> Result<DataSheet, DocError> {
    let mut nested = NestedTable::new(2, 3, 4)?;
    nested.groups[0].title = String::from("Control");
    nested.groups[0].subcolumn_titles = vec![String::from("Ctrl:1"), String::from("Ctrl:2"), String::from("Ctrl:3")];
    nested.groups[1].title = String::from("Treated");
    nested.groups[1].subcolumn_titles = vec![String::from("Trt:1"), String::from("Trt:2"), String::from("Trt:3")];
    for r in 0..4 {
        for u in 0..3 {
            nested.groups[0].cells[r][u] = Cell::number(20.0 + (u as f64) * 1.5 + (r as f64) * 0.3);
            nested.groups[1].cells[r][u] = Cell::number(31.0 + (u as f64) * 1.2 + (r as f64) * 0.4);
        }
    }
    DataSheet::new(sheet_id, title, TableArchetype::Nested(nested))
}

/// Return all [`FloatingNote`](curvebook_doc::FloatingNote) overlays attached to sheet `id`.
#[must_use]
pub(crate) fn sheet_floating_notes<'a>(doc: &'a ProjectDocument, id: &SheetId) -> &'a [curvebook_doc::FloatingNote] {
    if let Some(ds) = doc.data_sheet(id) {
        &ds.floating_notes
    } else if let Some(info) = doc.info_sheet(id) {
        &info.floating_notes
    } else if let Some(rs) = doc.result_sheet(id) {
        &rs.floating_notes
    } else if let Some(gs) = doc.graph_sheet(id) {
        &gs.floating_notes
    } else if let Some(ls) = doc.layout_sheet(id) {
        &ls.floating_notes
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{AnalysisKind, AnalysisSpec, GraphSpec, InfoSheet, LayoutSheet, LayoutSpec};

    #[test]
    fn duplicate_titles_get_disambiguation_subtitles_and_family_propagation() {
        let mut doc = ProjectDocument::new("dup-test").unwrap();
        let mut d1 = ColumnTable::new(ColumnDesignHint::default(), 2, 3).unwrap();
        d1.data_sets[0].title = String::from("Ctrl");
        d1.data_sets[1].title = String::from("Drug");
        let ds1 = DataSheet::new(SheetId::from("d1"), "Cohort A", TableArchetype::Column(d1)).unwrap();

        let d2 = ColumnTable::new(ColumnDesignHint::default(), 3, 5).unwrap();
        let ds2 = DataSheet::new(SheetId::from("d2"), "Cohort B", TableArchetype::Column(d2)).unwrap();

        let g1_spec = GraphSpec::default_for_data_sheet(&ds1);
        let gs1 = GraphSheet {
            id: SheetId::from("g1"),
            title: String::from("Summary"),
            initialized: true,
            spec: g1_spec,
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };
        let mut g2_spec = GraphSpec::default_for_data_sheet(&ds2);
        g2_spec.series[0].gdo = GraphicalDataObject::Violin {
            fill_color: Rgba8::rgb(100, 120, 200),
            stroke_color: Rgba8::BLACK,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        };
        let gs2 = GraphSheet {
            id: SheetId::from("g2"),
            title: String::from("Summary"),
            initialized: true,
            spec: g2_spec,
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };

        let mut rs1 = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::OneWayAnova(Default::default())).unwrap()).unwrap();
        rs1.analysis.custom_title = Some(String::from("Comparison"));
        let mut rs2 = ResultSheet::new(AnalysisSpec::new("r2", "d2", AnalysisKind::OneWayAnova(Default::default())).unwrap()).unwrap();
        rs2.analysis.custom_title = Some(String::from("comparison "));

        let info1 = InfoSheet::new_preseeded("i1", "Cohort A").unwrap();
        let mut ls1 = LayoutSheet::new("l1", "Panel", LayoutSpec::default()).unwrap();
        ls1.spec.grid_template = None;

        doc.data_sheets.extend([ds1, ds2]);
        doc.graph_sheets.extend([gs1, gs2]);
        doc.result_sheets.extend([rs1, rs2]);
        doc.info_sheets.push(info1);
        doc.layout_sheets.push(ls1);
        doc.active_sheet_id = Some(SheetId::from("d1"));
        doc.family_anchor_sheet_id = Some(SheetId::from("d1"));

        let mut nav = WorkspaceNavState::default();
        nav.sync_from_document(&doc);

        let g1_entry = nav.all_entries.iter().find(|e| e.sheet_id.as_str() == "g1").unwrap();
        let g2_entry = nav.all_entries.iter().find(|e| e.sheet_id.as_str() == "g2").unwrap();
        assert!(g1_entry.has_duplicate_title);
        assert!(g2_entry.has_duplicate_title);
        assert_ne!(g1_entry.subtitle, g2_entry.subtitle);
        assert!(g1_entry.subtitle.as_deref().unwrap().contains("Cohort A"));
        assert!(g2_entry.subtitle.as_deref().unwrap().contains("Cohort B"));
        assert!(g2_entry.subtitle.as_deref().unwrap().contains("Violin"));

        let r1_entry = nav.all_entries.iter().find(|e| e.sheet_id.as_str() == "r1").unwrap();
        let r2_entry = nav.all_entries.iter().find(|e| e.sheet_id.as_str() == "r2").unwrap();
        assert!(r1_entry.has_duplicate_title);
        assert!(r2_entry.has_duplicate_title);
        assert!(r1_entry.subtitle.as_deref().unwrap().starts_with("From Cohort A"));
        assert!(r2_entry.subtitle.as_deref().unwrap().starts_with("From Cohort B"));

        // Family nodes inherit subtitle + has_duplicate_title
        let fam_g1 = nav.family_nodes.iter().find(|n| n.sheet_id.as_str() == "g1").unwrap();
        assert!(fam_g1.has_duplicate_title);
        assert_eq!(fam_g1.subtitle, g1_entry.subtitle);

        // Search query matches subtitles cleanly
        nav.set_search_query("Violin", &doc);
        assert!(nav.entries.iter().any(|e| e.sheet_id.as_str() == "g2"));
        assert!(!nav.entries.iter().any(|e| e.sheet_id.as_str() == "g1"));
    }

    #[test]
    fn graph_subtitle_omits_redundant_title_and_collision_suffixes_stay_distinct() {
        let mut doc = ProjectDocument::new("collision-test").unwrap();
        let ds1 = DataSheet::new(SheetId::from("d1"), "Data 1", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 3, 2).unwrap())).unwrap();
        let ds2 = DataSheet::new(SheetId::from("d2"), "Data 1", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 3, 2).unwrap())).unwrap();
        let gs_same = GraphSheet {
            id: SheetId::from("g_same"),
            title: String::from("Data 1"),
            initialized: true,
            spec: GraphSpec::default_for_data_sheet(&ds1),
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };
        doc.data_sheets.extend([ds1, ds2]);
        doc.graph_sheets.push(gs_same);

        let map = compute_sheet_disambiguation(&doc);
        let d1 = &map[&SheetId::from("d1")];
        let d2 = &map[&SheetId::from("d2")];
        assert!(d1.has_duplicate_title);
        assert!(d2.has_duplicate_title);
        assert_ne!(d1.subtitle, d2.subtitle);
        assert_eq!(map[&SheetId::from("g_same")].subtitle.as_deref(), Some("XY"));
    }
}
