//! Deterministic data-only `.pzfx` XML exporter, shared preflight summary, tracked §6.3
//! corpus feature-coverage table, and data-layer fidelity comparison helper (§4.2, §4.3, §6.3).
//!
//! Scope:
//! - Always exports data tables, info sheets, notes, sheet order & active selection, exclusions,
//!   display decimals, subcolumn titles, and table/note colour tags.
//! - Never silently drops non-tabular content: [`preflight_pzfx_export`] computes a pure grouped
//!   summary shared verbatim with [`export_pzfx_bytes`].
//! - Byte-identical output via [`XmlWriter`] with `PrismXMLVersion="5.00"`, fixed `<Created>`
//!   derived solely from CurveBook's crate version (no timestamps, user/host paths, or external
//!   creator notices), and unanimous corpus element ordering:
//!   `Created -> InfoSequence -> Info* -> TableSequence -> (Table | HugeTable | Table1024)+`.
//! - Every emitted feature checked against [`PZFX_FEATURE_CORPUS_COVERAGE`] (`k >= 3` corpus files
//!   across `>= 2` distinct sources); any `InsufficientCoverage` feature emits a runtime
//!   [`PzfxExportWarningKind::InsufficientCorpusCoverage`] warning in [`PzfxExportReport::warnings`].

#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, ConstantValue, DataSetColumn, DataSheet, FloatingNote, InfoSheet, LevelOrderKind, MAX_DISPLAY_DECIMALS,
    ProjectDocument, ReplicateStacking, SheetId, SubcolumnMode, TableArchetype, VariableRole, VariableSource, XColumnMode,
};

use super::richtext::{color_to_pzfx_floating_note_name, color_to_pzfx_highlighted, write_pzfx_floating_note_body, write_pzfx_markup_element};
use crate::archive::validate_project_document;
use crate::error::FormatError;
use crate::xml::XmlWriter;

/// Bare `.pzfx` root XML element name observed in the corpus (`AGENTS.md` §1, decided 2026-11-26).
///
/// Stored without a leading `<` and passed directly to [`XmlWriter::start_element`]. Importer and
/// tests reference this constant rather than re-spelling it.
pub const PZFX_ROOT_ELEMENT: &str = "GraphPadPrismFile";

/// Target `.pzfx` XML dialect version observed across all corpus XML files.
const PZFX_XML_VERSION: &str = "5.00";

/// Deterministic `<Created>` body derived solely from the CurveBook crate version.
const CURVEBOOK_CREATED_LABEL: &str = concat!("CurveBook ", env!("CARGO_PKG_VERSION"));

/// Default column width attribute emitted on `<XColumn>`, `<YColumn>`, and `<RowTitlesColumn>`.
const DEFAULT_COLUMN_WIDTH: &str = "81";

/// Maximum Y column count written as `<Table>` before promoting to `<HugeTable>`.
const REGULAR_TABLE_MAX_Y_COLS: usize = 104;

/// Maximum subcolumn count per Y column written as `<Table>` before promoting to `<HugeTable>`.
const REGULAR_TABLE_MAX_SUBCOLS: usize = 52;

/// Maximum Y column count written as `<HugeTable>` before promoting to `<Table1024>`.
const HUGE_TABLE_MAX_Y_COLS: usize = 256;

// ---------------------------------------------------------------------------
// §6.3 Tracked Static Corpus Feature-Coverage Table
// ---------------------------------------------------------------------------

/// Corpus coverage status for a single `.pzfx` structural feature against the §6.3 gate
/// (`k >= 3` corpus files across `>= 2` distinct sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PzfxFeatureCoverageStatus {
    /// Observed in at least 3 distinct corpus files across at least 2 distinct source records.
    Conformant,
    /// Observed in fewer than 3 corpus files or fewer than 2 sources; still emitted when needed
    /// for lossless values, accompanied by a runtime [`PzfxExportWarningKind::InsufficientCorpusCoverage`] warning.
    InsufficientCoverage,
}

/// Single row of [`PZFX_FEATURE_CORPUS_COVERAGE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PzfxFeatureCoverageRow {
    /// Structural feature key matching `pzfx-dialect-profile.json` (`elem:...`, `attr:...`, `table_type:...`, `y_format:...`, `x_format:...`).
    pub feature: &'static str,
    /// Declared §6.3 conformance coverage status.
    pub status: PzfxFeatureCoverageStatus,
    /// Observed corpus file count when seeded (structural count only).
    pub corpus_files: usize,
    /// Observed corpus distinct source count when seeded (structural count only).
    pub corpus_sources: usize,
}

/// Tracked, neutral static feature-coverage table seeded from the `.pzfx` corpus dialect profile
/// (`2,178` direct XML files across `433` sources; §6.3 threshold `k >= 3` files across `>= 2` sources).
pub const PZFX_FEATURE_CORPUS_COVERAGE: &[PzfxFeatureCoverageRow] = &[
    // Root & sequences
    PzfxFeatureCoverageRow {
        feature: "attr:<Root>@PrismXMLVersion",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow { feature: "elem:<Root>/Created", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2178, corpus_sources: 433 },
    PzfxFeatureCoverageRow {
        feature: "elem:<Root>/InfoSequence",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow { feature: "elem:<Root>/Info", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2004, corpus_sources: 412 },
    PzfxFeatureCoverageRow {
        feature: "elem:<Root>/TableSequence",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow { feature: "elem:<Root>/Table", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2170, corpus_sources: 433 },
    PzfxFeatureCoverageRow { feature: "elem:<Root>/HugeTable", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 28, corpus_sources: 15 },
    PzfxFeatureCoverageRow { feature: "elem:<Root>/Table1024", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 8, corpus_sources: 6 },
    PzfxFeatureCoverageRow {
        feature: "elem:InfoSequence/Ref",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2010,
        corpus_sources: 414,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:TableSequence/Ref",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:InfoSequence@Selected",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 30,
        corpus_sources: 9,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:TableSequence@Selected",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 478,
        corpus_sources: 182,
    },
    PzfxFeatureCoverageRow { feature: "attr:Ref@ID", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2178, corpus_sources: 433 },
    PzfxFeatureCoverageRow { feature: "attr:Ref@Selected", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2178, corpus_sources: 433 },
    // Table archetypes
    PzfxFeatureCoverageRow { feature: "table_type:XY", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 823, corpus_sources: 260 },
    PzfxFeatureCoverageRow { feature: "table_type:OneWay", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 992, corpus_sources: 280 },
    PzfxFeatureCoverageRow { feature: "table_type:TwoWay", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 756, corpus_sources: 234 },
    PzfxFeatureCoverageRow { feature: "table_type:Contingency", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 20, corpus_sources: 13 },
    PzfxFeatureCoverageRow { feature: "table_type:Survival", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 33, corpus_sources: 18 },
    PzfxFeatureCoverageRow {
        feature: "table_type:PartsOfWhole",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 26,
        corpus_sources: 12,
    },
    PzfxFeatureCoverageRow {
        feature: "table_type:TwoWay|Ext=Nested",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 17,
        corpus_sources: 8,
    },
    PzfxFeatureCoverageRow {
        feature: "table_type:OneWay|Ext=MultipleVariables",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 4,
        corpus_sources: 3,
    },
    // Y formats
    PzfxFeatureCoverageRow {
        feature: "y_format:<missing>|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 1022,
        corpus_sources: 289,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=1",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 437,
        corpus_sources: 173,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=2",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 104,
        corpus_sources: 52,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=3",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 383,
        corpus_sources: 131,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=4",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 119,
        corpus_sources: 72,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=5",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 109,
        corpus_sources: 65,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:replicates|Subcolumns=<missing>|Replicates=6",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 133,
        corpus_sources: 75,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:SD|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 43,
        corpus_sources: 17,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:SE|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 36,
        corpus_sources: 15,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:SDN|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 80,
        corpus_sources: 46,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:SEN|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 16,
        corpus_sources: 12,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:upper-lower-limits|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 12,
        corpus_sources: 7,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:low-high|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 2,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:CV|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 2,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:CVN|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 0,
        corpus_sources: 0,
    },
    PzfxFeatureCoverageRow {
        feature: "y_format:median-iqr|Subcolumns=<missing>|Replicates=<missing>",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 0,
        corpus_sources: 0,
    },
    // Table attributes & children
    PzfxFeatureCoverageRow { feature: "attr:Table@EVFormat", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2166, corpus_sources: 430 },
    PzfxFeatureCoverageRow {
        feature: "attr:Table@ExtTableType",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 21,
        corpus_sources: 11,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:Table@Highlighted",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 100,
        corpus_sources: 30,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:HugeTable@Highlighted",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 4,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow { feature: "elem:Table/Info", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 12, corpus_sources: 10 },
    PzfxFeatureCoverageRow {
        feature: "elem:Table/FloatingNote",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 156,
        corpus_sources: 86,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:HugeTable/FloatingNote",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 2,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:Table1024/FloatingNote",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 2,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:Table/RowTitlesColumn",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 1181,
        corpus_sources: 316,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:HugeTable/RowTitlesColumn",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 14,
        corpus_sources: 8,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:Table1024/RowTitlesColumn",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 1,
        corpus_sources: 1,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:Table/SubColumnTitles",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 97,
        corpus_sources: 55,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:HugeTable/SubColumnTitles",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 1,
        corpus_sources: 1,
    },
    PzfxFeatureCoverageRow {
        feature: "elem:Table1024/SubColumnTitles",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 2,
        corpus_sources: 2,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:SubColumnTitles@OwnSet",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 98,
        corpus_sources: 56,
    },
    PzfxFeatureCoverageRow { feature: "elem:Table/XColumn", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 842, corpus_sources: 267 },
    PzfxFeatureCoverageRow { feature: "elem:HugeTable/XColumn", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 18, corpus_sources: 10 },
    PzfxFeatureCoverageRow { feature: "elem:Table1024/XColumn", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 6, corpus_sources: 4 },
    PzfxFeatureCoverageRow {
        feature: "attr:XColumn@Decimals",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 849,
        corpus_sources: 268,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:XColumn@Subcolumns",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 849,
        corpus_sources: 268,
    },
    PzfxFeatureCoverageRow { feature: "attr:XColumn@Width", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 849, corpus_sources: 268 },
    PzfxFeatureCoverageRow {
        feature: "attr:YColumn@Decimals",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:YColumn@Subcolumns",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 2178,
        corpus_sources: 433,
    },
    PzfxFeatureCoverageRow { feature: "attr:YColumn@Width", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 2178, corpus_sources: 433 },
    PzfxFeatureCoverageRow {
        feature: "attr:YColumn@MVType",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 1,
        corpus_sources: 1,
    },
    PzfxFeatureCoverageRow {
        feature: "attr:Table@XEVFormat",
        status: PzfxFeatureCoverageStatus::InsufficientCoverage,
        corpus_files: 0,
        corpus_sources: 0,
    },
    PzfxFeatureCoverageRow { feature: "attr:d@Excluded", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 240, corpus_sources: 118 },
    // Rich text & floating-note formatting
    PzfxFeatureCoverageRow { feature: "elem:Title/B", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 87, corpus_sources: 37 },
    PzfxFeatureCoverageRow { feature: "elem:Title/I", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 129, corpus_sources: 28 },
    PzfxFeatureCoverageRow { feature: "elem:Title/SUP", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 88, corpus_sources: 28 },
    PzfxFeatureCoverageRow { feature: "elem:Title/SUB", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 44, corpus_sources: 29 },
    PzfxFeatureCoverageRow { feature: "elem:d/B", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 8, corpus_sources: 9 },
    PzfxFeatureCoverageRow { feature: "elem:d/I", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 25, corpus_sources: 16 },
    PzfxFeatureCoverageRow { feature: "elem:d/SUP", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 16, corpus_sources: 11 },
    PzfxFeatureCoverageRow { feature: "elem:d/SUB", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 7, corpus_sources: 6 },
    PzfxFeatureCoverageRow {
        feature: "elem:FloatingNote/Font",
        status: PzfxFeatureCoverageStatus::Conformant,
        corpus_files: 157,
        corpus_sources: 86,
    },
    PzfxFeatureCoverageRow { feature: "elem:FloatingNote/B", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 96, corpus_sources: 63 },
    PzfxFeatureCoverageRow { feature: "elem:FloatingNote/I", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 4, corpus_sources: 4 },
    PzfxFeatureCoverageRow { feature: "elem:Font/BR", status: PzfxFeatureCoverageStatus::Conformant, corpus_files: 153, corpus_sources: 87 },
];

/// Look up a feature key in [`PZFX_FEATURE_CORPUS_COVERAGE`].
#[must_use]
pub fn lookup_pzfx_feature_coverage(feature: &str) -> Option<&'static PzfxFeatureCoverageRow> {
    PZFX_FEATURE_CORPUS_COVERAGE.iter().find(|row| row.feature == feature)
}

// ---------------------------------------------------------------------------
// §4.2 Shared Preflight & Export Report Types
// ---------------------------------------------------------------------------

/// Category of non-tabular content or metadata dropped or flattened during `.pzfx` export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum PzfxDroppedKind {
    /// Analysis and result sheets (`ProjectDocument::result_sheets`).
    AnalysesAndResults,
    /// Graph sheets (`ProjectDocument::graph_sheets`).
    Graphs,
    /// Multi-panel layout sheets (`ProjectDocument::layout_sheets`).
    Layouts,
    /// Live linked regions inside data sheets (`DataSheet::linked_regions`).
    LiveLinks,
    /// Embedded raster image assets (`ProjectDocument::image_assets`).
    Images,
    /// Named project milestone checkpoints (`ProjectDocument::checkpoints`).
    Checkpoints,
    /// Calculated-variable formulas written as their current evaluated values.
    FormulasWrittenAsValues,
    /// Table or sheet features with no exact `.pzfx` equivalent (losslessly materialized or omitted).
    UnrepresentableTableFeatures,
}

impl PzfxDroppedKind {
    /// Neutral human-readable label for dialogs and reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AnalysesAndResults => "Analyses and results",
            Self::Graphs => "Graphs",
            Self::Layouts => "Layouts",
            Self::LiveLinks => "Live linked regions",
            Self::Images => "Embedded images",
            Self::Checkpoints => "Project checkpoints",
            Self::FormulasWrittenAsValues => "Formulas (written as current values)",
            Self::UnrepresentableTableFeatures => "Unrepresentable table features",
        }
    }
}

/// Summary of items dropped or flattened under one [`PzfxDroppedKind`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PzfxDroppedGroup {
    pub kind: PzfxDroppedKind,
    pub count: usize,
    pub items: Vec<String>,
    pub summary: String,
}

/// Pure preflight report shared between GUI confirmation prompts and [`export_pzfx_bytes`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PzfxExportPreflight {
    pub dropped: Vec<PzfxDroppedGroup>,
    pub data_sheet_count: usize,
    pub info_sheet_count: usize,
}

impl PzfxExportPreflight {
    /// `true` when at least one non-tabular item or table feature will be dropped or flattened.
    #[must_use]
    pub fn has_dropped_items(&self) -> bool {
        !self.dropped.is_empty()
    }

    /// Total count of dropped or flattened items summed across all groups.
    #[must_use]
    pub fn total_dropped_count(&self) -> usize {
        self.dropped.iter().map(|g| g.count).fold(0_usize, usize::saturating_add)
    }

    /// Look up the dropped-item group for `kind`, if present.
    #[must_use]
    pub fn group(&self, kind: PzfxDroppedKind) -> Option<&PzfxDroppedGroup> {
        self.dropped.iter().find(|g| g.kind == kind)
    }
}

/// Classification of a warning emitted by [`export_pzfx_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PzfxExportWarningKind {
    /// Non-tabular project content or unrepresentable feature dropped or flattened (mirrors preflight).
    DroppedContent,
    /// Emitted `.pzfx` construct lacks sufficient corpus coverage (`< 3` files or `< 2` sources; §6.3).
    InsufficientCorpusCoverage,
}

/// Structured non-fatal warning returned in [`PzfxExportReport::warnings`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PzfxExportWarning {
    pub kind: PzfxExportWarningKind,
    pub source: String,
    pub message: String,
}

impl PzfxExportWarning {
    #[must_use]
    pub fn new(kind: PzfxExportWarningKind, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind, source: source.into(), message: message.into() }
    }
}

/// Complete output of [`export_pzfx_bytes`] (and [`export_pzfx_file`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PzfxExportReport {
    /// Deterministic UTF-8 `.pzfx` XML payload.
    pub bytes: Vec<u8>,
    /// Shared preflight summary of dropped non-tabular content and unrepresentable features.
    pub preflight: PzfxExportPreflight,
    /// Structured warnings covering every dropped group and any emitted `InsufficientCoverage` feature.
    pub warnings: Vec<PzfxExportWarning>,
}

/// Pure preflight audit computing every item dropped or flattened when exporting `doc` to `.pzfx`.
///
/// Shared verbatim by GUI confirmation modals and [`export_pzfx_bytes`] so preview counts and
/// sheet names never diverge from the final [`PzfxExportReport`].
#[must_use]
pub fn preflight_pzfx_export(doc: &ProjectDocument) -> PzfxExportPreflight {
    let mut dropped = Vec::new();

    if !doc.result_sheets.is_empty() {
        let items: Vec<String> = doc.result_sheets.iter().map(|s| s.title().to_owned()).collect();
        let count = items.len();
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::AnalysesAndResults,
            count,
            items,
            summary: format!("dropped {count} analysis/result sheet(s) (`.pzfx` stores data tables and info sheets only)"),
        });
    }
    if !doc.graph_sheets.is_empty() {
        let items: Vec<String> = doc.graph_sheets.iter().map(|s| s.title.clone()).collect();
        let count = items.len();
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::Graphs,
            count,
            items,
            summary: format!("dropped {count} graph sheet(s) (`.pzfx` export is data-only)"),
        });
    }
    if !doc.layout_sheets.is_empty() {
        let items: Vec<String> = doc.layout_sheets.iter().map(|s| s.title.clone()).collect();
        let count = items.len();
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::Layouts,
            count,
            items,
            summary: format!("dropped {count} layout sheet(s) (`.pzfx` export is data-only)"),
        });
    }

    let mut link_count = 0_usize;
    let mut link_sheets = Vec::new();
    let mut formula_count = 0_usize;
    let mut formula_items = Vec::new();
    let mut unrep_items = Vec::new();

    for ds in &doc.data_sheets {
        if !ds.linked_regions.is_empty() {
            link_count = link_count.saturating_add(ds.linked_regions.len());
            link_sheets.push(ds.title.clone());
        }
        collect_data_sheet_preflight_notes(ds, &mut formula_count, &mut formula_items, &mut unrep_items);
    }
    for info in &doc.info_sheets {
        collect_info_sheet_preflight_notes(info, &mut unrep_items);
    }

    if link_count > 0 {
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::LiveLinks,
            count: link_count,
            items: link_sheets,
            summary: format!("dropped {link_count} live linked region(s) (current cell values exported statically)"),
        });
    }
    if !doc.image_assets.is_empty() {
        let items: Vec<String> = doc.image_assets.iter().map(|a| a.asset_id.clone()).collect();
        let count = items.len();
        dropped.push(PzfxDroppedGroup { kind: PzfxDroppedKind::Images, count, items, summary: format!("dropped {count} embedded image asset(s)") });
    }
    if !doc.checkpoints.is_empty() {
        let items: Vec<String> = doc.checkpoints.iter().map(|cp| cp.name.clone()).collect();
        let count = items.len();
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::Checkpoints,
            count,
            items,
            summary: format!("dropped {count} project checkpoint(s)"),
        });
    }
    if formula_count > 0 {
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::FormulasWrittenAsValues,
            count: formula_count,
            items: formula_items,
            summary: format!("exported {formula_count} calculated-variable formula column(s) as their current evaluated values"),
        });
    }
    if !unrep_items.is_empty() {
        let count = unrep_items.len();
        dropped.push(PzfxDroppedGroup {
            kind: PzfxDroppedKind::UnrepresentableTableFeatures,
            count,
            items: unrep_items,
            summary: format!("{count} table/sheet feature(s) have no exact `.pzfx` equivalent and were materialized or omitted"),
        });
    }

    PzfxExportPreflight { dropped, data_sheet_count: doc.data_sheets.len(), info_sheet_count: doc.info_sheets.len() }
}

fn collect_data_sheet_preflight_notes(ds: &DataSheet, formula_count: &mut usize, formula_items: &mut Vec<String>, unrep_items: &mut Vec<String>) {
    collect_sheet_metadata_preflight_notes(ds, unrep_items);
    collect_archetype_preflight_notes(&ds.title, &ds.archetype, formula_count, formula_items, unrep_items);
}

fn collect_sheet_metadata_preflight_notes(ds: &DataSheet, unrep_items: &mut Vec<String>) {
    let label = &ds.title;
    if ds.frozen {
        unrep_items.push(format!("{label}: sheet freeze state"));
    }
    if !ds.exclusion_notes.is_empty() {
        unrep_items.push(format!("{label}: {} exclusion audit note(s)", ds.exclusion_notes.len()));
    }
    if !ds.plate_well_origins.is_empty() {
        unrep_items.push(format!("{label}: {} microplate well origin(s)", ds.plate_well_origins.len()));
    }
    if ds.origin.is_some() {
        unrep_items.push(format!("{label}: navigator origin badge metadata"));
    }
    if ds.floating_notes.iter().any(|n| n.minimized) {
        unrep_items.push(format!("{label}: minimized floating-note state"));
    }
    if let Some(tag) = ds.color_tag {
        let (_, _, tag_name) = table_element_dimensions(&ds.archetype);
        if tag_name == "Table1024" {
            unrep_items.push(format!("{label}: colour tag on Table1024"));
        } else if !color_to_pzfx_highlighted(tag).1 {
            unrep_items.push(format!("{label}: colour tag mapped to nearest 1..7 highlight index"));
        }
    }
}

fn collect_archetype_preflight_notes(
    label: &str,
    arch: &TableArchetype,
    formula_count: &mut usize,
    formula_items: &mut Vec<String>,
    unrep_items: &mut Vec<String>,
) {
    match arch {
        TableArchetype::Xy(xy) => {
            if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
                unrep_items.push(format!("{label}: generated X sequence materialized as explicit numeric X values"));
            }
            if let XColumnMode::DateOrElapsedTime { unit_label } = &xy.x_mode
                && unit_label != &xy.x_title
            {
                unrep_items.push(format!("{label}: distinct X unit label `{unit_label}` folded into X column title"));
            }
            if !xy.x_subcolumn_titles.is_empty() {
                unrep_items.push(format!("{label}: X-column subcolumn titles"));
            }
        }
        TableArchetype::Column(col) => {
            if matches!(col.design_hint, ColumnDesignHint::PairedRepeatedMeasures) {
                unrep_items.push(format!("{label}: paired/repeated-measures column design hint"));
            }
        }
        TableArchetype::Grouped(grp) => {
            if matches!(grp.replicate_stacking, ReplicateStacking::Stacked) {
                unrep_items.push(format!("{label}: stacked replicate layout"));
            }
            if grp.row_numeric_spacing.is_some() {
                unrep_items.push(format!("{label}: numeric row spacing"));
            }
            if grp.three_way_grouping.is_some() {
                unrep_items.push(format!("{label}: three-way column grouping metadata"));
            }
            if !grp.row_factor_title.is_empty() && grp.row_factor_title != "Row Factor" {
                unrep_items.push(format!("{label}: row factor title `{}`", grp.row_factor_title));
            }
            if !grp.column_factor_title.is_empty() && grp.column_factor_title != "Column Factor" {
                unrep_items.push(format!("{label}: column factor title `{}`", grp.column_factor_title));
            }
        }
        TableArchetype::Contingency(cont) => {
            if !cont.row_factor_title.is_empty() && cont.row_factor_title != "Outcome / Exposure" {
                unrep_items.push(format!("{label}: row factor title `{}`", cont.row_factor_title));
            }
            if !cont.column_factor_title.is_empty() && cont.column_factor_title != "Group" {
                unrep_items.push(format!("{label}: column factor title `{}`", cont.column_factor_title));
            }
        }
        TableArchetype::Survival(surv) => {
            if surv.event_code != 1 || surv.censored_code != 0 {
                unrep_items.push(format!("{label}: custom survival codes (event={}, censored={})", surv.event_code, surv.censored_code));
            }
        }
        TableArchetype::PartsOfWhole(_) | TableArchetype::Nested(_) => {}
        TableArchetype::MultipleVariables(mv) => {
            for var in &mv.variables {
                if matches!(var.source, VariableSource::Calculated { .. }) {
                    *formula_count = formula_count.saturating_add(1);
                    formula_items.push(format!("{label}: {}", var.title));
                }
                if matches!(var.role, VariableRole::Label) {
                    unrep_items.push(format!("{label}: `{}` Label variable role", var.title));
                }
                if var.level_order.order != LevelOrderKind::EntryOrder || var.level_order.reversed {
                    unrep_items.push(format!("{label}: `{}` custom categorical level order", var.title));
                }
            }
        }
    }
}

fn collect_info_sheet_preflight_notes(info: &InfoSheet, unrep_items: &mut Vec<String>) {
    let label = &info.title;
    let units_count = info.constants.iter().filter(|c| c.units.is_some()).count();
    if units_count > 0 {
        unrep_items.push(format!("{label}: {units_count} info constant unit label(s)"));
    }
    if !info.floating_notes.is_empty() {
        unrep_items.push(format!("{label}: {} floating note(s) on info sheet", info.floating_notes.len()));
    }
    if info.color_tag.is_some() {
        unrep_items.push(format!("{label}: info sheet colour tag"));
    }
    if info.origin.is_some() {
        unrep_items.push(format!("{label}: navigator origin badge metadata"));
    }
}

// ---------------------------------------------------------------------------
// §4.2 Deterministic Data-Only `.pzfx` Writer
// ---------------------------------------------------------------------------

/// Export `doc` to deterministic UTF-8 `.pzfx` XML bytes alongside its [`PzfxExportPreflight`]
/// and structured [`PzfxExportWarning`] list.
pub fn export_pzfx_bytes(doc: &ProjectDocument) -> Result<PzfxExportReport, FormatError> {
    validate_project_document(doc)?;

    let preflight = preflight_pzfx_export(doc);
    let mut warnings: Vec<PzfxExportWarning> = preflight
        .dropped
        .iter()
        .map(|group| PzfxExportWarning::new(PzfxExportWarningKind::DroppedContent, group.kind.label(), group.summary.clone()))
        .collect();
    let mut emitted_coverage_warnings: BTreeSet<(String, String)> = BTreeSet::new();

    let (embedded_info_by_table, embedded_info_ids) = plan_per_table_embedded_infos(doc);
    let (active_info_id, active_table_id, info_seq_selected, table_seq_selected) = resolve_export_selection(doc);

    let mut w = XmlWriter::with_indent("  ")?;
    w.write_xml_declaration()?;
    w.start_element(PZFX_ROOT_ELEMENT, &[("PrismXMLVersion", PZFX_XML_VERSION)])?;

    // 1. <Created> (fixed CurveBook version only; zero timestamps/user paths/external notices)
    w.text_element("Created", &[], CURVEBOOK_CREATED_LABEL)?;

    // 2. <InfoSequence>
    let info_seq_attrs: &[(&str, &str)] = if info_seq_selected { &[("Selected", "1")] } else { &[] };
    if doc.info_sheets.is_empty() {
        w.empty_element("InfoSequence", info_seq_attrs)?;
    } else {
        w.start_element("InfoSequence", info_seq_attrs)?;
        for info in &doc.info_sheets {
            let is_sel = active_info_id.as_ref() == Some(&info.id);
            if is_sel {
                w.empty_element("Ref", &[("ID", info.id.as_str()), ("Selected", "1")])?;
            } else {
                w.empty_element("Ref", &[("ID", info.id.as_str())])?;
            }
        }
        w.end_element_named("InfoSequence")?;
    }

    // 3. Top-level <Info>*
    for info in &doc.info_sheets {
        if !embedded_info_ids.contains(&info.id) {
            write_info_element(&mut w, info)?;
        }
    }

    // 4. <TableSequence>
    let table_seq_attrs: &[(&str, &str)] = if table_seq_selected { &[("Selected", "1")] } else { &[] };
    if doc.data_sheets.is_empty() {
        w.empty_element("TableSequence", table_seq_attrs)?;
    } else {
        w.start_element("TableSequence", table_seq_attrs)?;
        for ds in &doc.data_sheets {
            let is_sel = active_table_id.as_ref() == Some(&ds.id);
            if is_sel {
                w.empty_element("Ref", &[("ID", ds.id.as_str()), ("Selected", "1")])?;
            } else {
                w.empty_element("Ref", &[("ID", ds.id.as_str())])?;
            }
        }
        w.end_element_named("TableSequence")?;
    }

    // 5. (<Table> | <HugeTable> | <Table1024>)+
    for ds in &doc.data_sheets {
        let embedded_info = embedded_info_by_table.get(&ds.id).and_then(|info_id| doc.info_sheet(info_id));
        write_table_element(&mut w, ds, embedded_info, &mut warnings, &mut emitted_coverage_warnings)?;
    }

    w.end_element_named(PZFX_ROOT_ELEMENT)?;
    let bytes = w.finish_bytes()?;
    Ok(PzfxExportReport { bytes, preflight, warnings })
}

/// Export `doc` to a `.pzfx` file at `path` via atomic write (`curvebook_platform::fs::write_atomic`).
#[cfg(not(target_arch = "wasm32"))]
pub fn export_pzfx_file(path: &Path, doc: &ProjectDocument) -> Result<PzfxExportReport, FormatError> {
    let report = export_pzfx_bytes(doc)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    curvebook_platform::fs::write_atomic(path, &report.bytes)?;
    Ok(report)
}

fn plan_per_table_embedded_infos(doc: &ProjectDocument) -> (HashMap<SheetId, SheetId>, HashSet<SheetId>) {
    let mut embedded_by_table = HashMap::new();
    let mut embedded_infos = HashSet::new();
    if doc.info_sheets.len() <= 1 {
        return (embedded_by_table, embedded_infos);
    }

    let primary_id = doc.info_sheets.first().map(|s| &s.id);
    let mut table_refs_per_info: HashMap<&SheetId, Vec<&DataSheet>> = HashMap::new();
    for ds in &doc.data_sheets {
        if let Some(info_id) = &ds.info_sheet_id {
            table_refs_per_info.entry(info_id).or_default().push(ds);
        }
    }

    for info in &doc.info_sheets {
        if primary_id == Some(&info.id) {
            continue;
        }
        if let Some(owners) = table_refs_per_info.get(&info.id)
            && owners.len() == 1
            && let Some(owner_ds) = owners.first()
        {
            let (_, _, tag_name) = table_element_dimensions(&owner_ds.archetype);
            if tag_name == "Table" && !embedded_by_table.contains_key(&owner_ds.id) {
                embedded_by_table.insert(owner_ds.id.clone(), info.id.clone());
                embedded_infos.insert(info.id.clone());
            }
        }
    }
    (embedded_by_table, embedded_infos)
}

fn resolve_export_selection(doc: &ProjectDocument) -> (Option<SheetId>, Option<SheetId>, bool, bool) {
    let active_is_info = doc.active_sheet_id.as_ref().is_some_and(|id| doc.info_sheet(id).is_some());
    let active_is_data = doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some());

    let selected_info = if active_is_info { doc.active_sheet_id.clone() } else { doc.info_sheets.first().map(|s| s.id.clone()) };
    let selected_table = if active_is_data { doc.active_sheet_id.clone() } else { doc.data_sheets.first().map(|s| s.id.clone()) };
    let info_seq_selected = active_is_info;
    let table_seq_selected = active_is_data || (!active_is_info && !doc.data_sheets.is_empty());
    (selected_info, selected_table, info_seq_selected, table_seq_selected)
}

fn write_info_element(w: &mut XmlWriter, info: &InfoSheet) -> Result<(), FormatError> {
    w.start_element("Info", &[("ID", info.id.as_str())])?;
    write_pzfx_markup_element(w, "Title", &[], &info.title)?;
    if info.notes_markdown.is_empty() {
        w.empty_element("Notes", &[])?;
    } else {
        w.text_element("Notes", &[], &info.notes_markdown)?;
    }
    for constant in &info.constants {
        w.start_element("Constant", &[])?;
        w.text_element("Name", &[], &constant.name)?;
        match &constant.value {
            ConstantValue::Empty => w.empty_element("Value", &[])?,
            ConstantValue::Number(v) => w.text_element("Value", &[], &format_pzfx_f64(*v))?,
            ConstantValue::Date(s) => w.text_element("Value", &[], s)?,
            ConstantValue::Text(s) => write_pzfx_markup_element(w, "Value", &[], s)?,
        }
        w.end_element_named("Constant")?;
    }
    w.end_element_named("Info")
}

fn table_element_dimensions(arch: &TableArchetype) -> (usize, usize, &'static str) {
    let (y_cols, max_subs) = match arch {
        TableArchetype::Xy(t) => (t.data_sets.len(), t.y_subcolumn_mode.subcolumn_count()),
        TableArchetype::Column(t) => (t.data_sets.len(), t.subcolumn_mode.subcolumn_count()),
        TableArchetype::Grouped(t) => (t.data_sets.len(), t.subcolumn_mode.subcolumn_count()),
        TableArchetype::Contingency(t) => (t.data_sets.len(), 1),
        TableArchetype::Survival(t) => (t.cohorts.len(), 1),
        TableArchetype::PartsOfWhole(t) => (t.data_sets.len(), 1),
        TableArchetype::MultipleVariables(t) => (t.variables.len(), 1),
        TableArchetype::Nested(t) => {
            (t.groups.len(), t.groups.iter().map(curvebook_doc::NestedGroupColumn::biological_unit_count).max().unwrap_or(1))
        }
    };
    let tag = if y_cols > HUGE_TABLE_MAX_Y_COLS {
        "Table1024"
    } else if y_cols > REGULAR_TABLE_MAX_Y_COLS || max_subs > REGULAR_TABLE_MAX_SUBCOLS {
        "HugeTable"
    } else {
        "Table"
    };
    (y_cols, max_subs, tag)
}

fn record_feature_coverage(table_id: &str, feature: &str, warnings: &mut Vec<PzfxExportWarning>, emitted: &mut BTreeSet<(String, String)>) {
    let needs_warn = match lookup_pzfx_feature_coverage(feature) {
        Some(row) => row.status == PzfxFeatureCoverageStatus::InsufficientCoverage,
        None => true,
    };
    if needs_warn && emitted.insert((table_id.to_owned(), feature.to_owned())) {
        let detail = match lookup_pzfx_feature_coverage(feature) {
            Some(row) => format!(
                "feature `{feature}` has insufficient corpus coverage ({} file(s) across {} source(s); §6.3 requires >= 3 files across >= 2 sources)",
                row.corpus_files, row.corpus_sources
            ),
            None => format!("feature `{feature}` was not observed with sufficient corpus coverage (>= 3 files across >= 2 sources)"),
        };
        warnings.push(PzfxExportWarning::new(PzfxExportWarningKind::InsufficientCorpusCoverage, table_id, detail));
    }
}

struct TableHeaderAttrs {
    table_type: &'static str,
    ext_table_type: Option<&'static str>,
    x_format: &'static str,
    x_ev_format: Option<&'static str>,
    y_format: Option<&'static str>,
    replicates: Option<String>,
}

fn resolve_table_header_attrs(
    arch: &TableArchetype,
    table_id: &str,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> TableHeaderAttrs {
    match arch {
        TableArchetype::Xy(xy) => {
            let (x_format, x_ev_format) = resolve_xy_x_format_attrs(&xy.x_mode, table_id, warnings, emitted);
            let (y_format, replicates) = resolve_y_format_attrs(xy.y_subcolumn_mode, table_id, warnings, emitted);
            TableHeaderAttrs { table_type: "XY", ext_table_type: None, x_format, x_ev_format, y_format, replicates }
        }
        TableArchetype::Column(col) => {
            let (y_format, replicates) = resolve_y_format_attrs(col.subcolumn_mode, table_id, warnings, emitted);
            TableHeaderAttrs { table_type: "OneWay", ext_table_type: None, x_format: "none", x_ev_format: None, y_format, replicates }
        }
        TableArchetype::Grouped(grp) => {
            let (y_format, replicates) = resolve_y_format_attrs(grp.subcolumn_mode, table_id, warnings, emitted);
            TableHeaderAttrs { table_type: "TwoWay", ext_table_type: None, x_format: "none", x_ev_format: None, y_format, replicates }
        }
        TableArchetype::Contingency(_) => TableHeaderAttrs {
            table_type: "Contingency",
            ext_table_type: None,
            x_format: "none",
            x_ev_format: None,
            y_format: None,
            replicates: None,
        },
        TableArchetype::Survival(_) => TableHeaderAttrs {
            table_type: "Survival",
            ext_table_type: None,
            x_format: "numbers",
            x_ev_format: None,
            y_format: None,
            replicates: None,
        },
        TableArchetype::PartsOfWhole(_) => TableHeaderAttrs {
            table_type: "PartsOfWhole",
            ext_table_type: None,
            x_format: "none",
            x_ev_format: None,
            y_format: None,
            replicates: None,
        },
        TableArchetype::MultipleVariables(_) => TableHeaderAttrs {
            table_type: "OneWay",
            ext_table_type: Some("MultipleVariables"),
            x_format: "none",
            x_ev_format: None,
            y_format: None,
            replicates: None,
        },
        TableArchetype::Nested(nested) => {
            let max_units = nested.groups.iter().map(curvebook_doc::NestedGroupColumn::biological_unit_count).max().unwrap_or(1);
            TableHeaderAttrs {
                table_type: "TwoWay",
                ext_table_type: Some("Nested"),
                x_format: "none",
                x_ev_format: None,
                y_format: Some("replicates"),
                replicates: Some(max_units.to_string()),
            }
        }
    }
}

fn resolve_xy_x_format_attrs(
    x_mode: &XColumnMode,
    table_id: &str,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> (&'static str, Option<&'static str>) {
    match x_mode {
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single } | XColumnMode::GeneratedSequence { .. } => ("numbers", None),
        XColumnMode::DateOrElapsedTime { .. } => ("time", None),
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSd } => ("error", None),
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(_) } => {
            record_feature_coverage(table_id, "x_format:replicates", warnings, emitted);
            ("replicates", None)
        }
        XColumnMode::Numbers { subcolumn_mode } => {
            let ev = summary_mode_wire_name(*subcolumn_mode);
            record_feature_coverage(table_id, "attr:Table@XEVFormat", warnings, emitted);
            ("error", Some(ev))
        }
    }
}

fn summary_mode_wire_name(mode: SubcolumnMode) -> &'static str {
    match mode {
        SubcolumnMode::MeanSd => "SD",
        SubcolumnMode::MeanSem => "SE",
        SubcolumnMode::MeanSdN => "SDN",
        SubcolumnMode::MeanSemN => "SEN",
        SubcolumnMode::MeanCvN => "CVN",
        SubcolumnMode::MeanLowHigh => "low-high",
        SubcolumnMode::UpperLowerLimits => "upper-lower-limits",
        SubcolumnMode::MedianIqr => "median-iqr",
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => "replicates",
    }
}

fn resolve_y_format_attrs(
    mode: SubcolumnMode,
    table_id: &str,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> (Option<&'static str>, Option<String>) {
    match mode {
        SubcolumnMode::Single => (None, None),
        SubcolumnMode::RawReplicates(n) => (Some("replicates"), Some(n.to_string())),
        SubcolumnMode::MeanSd => {
            record_feature_coverage(table_id, "y_format:SD|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("SD"), None)
        }
        SubcolumnMode::MeanSem => {
            record_feature_coverage(table_id, "y_format:SE|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("SE"), None)
        }
        SubcolumnMode::MeanSdN => {
            record_feature_coverage(table_id, "y_format:SDN|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("SDN"), None)
        }
        SubcolumnMode::MeanSemN => {
            record_feature_coverage(table_id, "y_format:SEN|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("SEN"), None)
        }
        SubcolumnMode::MeanCvN => {
            record_feature_coverage(table_id, "y_format:CVN|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("CVN"), None)
        }
        SubcolumnMode::MeanLowHigh => {
            record_feature_coverage(table_id, "y_format:low-high|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("low-high"), None)
        }
        SubcolumnMode::UpperLowerLimits => {
            record_feature_coverage(table_id, "y_format:upper-lower-limits|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("upper-lower-limits"), None)
        }
        SubcolumnMode::MedianIqr => {
            record_feature_coverage(table_id, "y_format:median-iqr|Subcolumns=<missing>|Replicates=<missing>", warnings, emitted);
            (Some("median-iqr"), None)
        }
    }
}

fn write_table_element(
    w: &mut XmlWriter,
    ds: &DataSheet,
    embedded_info: Option<&InfoSheet>,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> Result<(), FormatError> {
    let (_, _, tag_name) = table_element_dimensions(&ds.archetype);
    let header = resolve_table_header_attrs(&ds.archetype, ds.id.as_str(), warnings, emitted);

    let mut attrs: Vec<(&str, &str)> = Vec::with_capacity(8);
    attrs.push(("ID", ds.id.as_str()));
    attrs.push(("XFormat", header.x_format));
    if let Some(x_ev) = header.x_ev_format {
        attrs.push(("XEVFormat", x_ev));
    }
    if let Some(y_fmt) = header.y_format {
        attrs.push(("YFormat", y_fmt));
    }
    if let Some(ref reps) = header.replicates {
        attrs.push(("Replicates", reps.as_str()));
    }
    attrs.push(("TableType", header.table_type));
    if let Some(ext) = header.ext_table_type {
        attrs.push(("ExtTableType", ext));
    }
    attrs.push(("EVFormat", "AsteriskAfterNumber"));
    if let Some(color) = ds.color_tag
        && tag_name != "Table1024"
    {
        let (hl_idx, _) = color_to_pzfx_highlighted(color);
        attrs.push(("Highlighted", hl_idx));
    }

    w.start_element(tag_name, &attrs)?;

    // Unanimous corpus child order inside <Table>/<HugeTable>/<Table1024>:
    // Title -> Info? -> FloatingNote* -> RowTitlesColumn? -> SubColumnTitles? -> XColumn? -> YColumn+
    write_pzfx_markup_element(w, "Title", &[], &ds.title)?;

    if let Some(info) = embedded_info {
        write_info_element(w, info)?;
    }

    if !ds.floating_notes.is_empty() {
        let feat = format!("elem:{tag_name}/FloatingNote");
        record_feature_coverage(ds.id.as_str(), &feat, warnings, emitted);
        for note in &ds.floating_notes {
            write_floating_note_element(w, note)?;
        }
    }

    write_row_titles_if_present(w, ds, tag_name, warnings, emitted)?;
    write_subcolumn_titles_if_present(w, ds, tag_name, warnings, emitted)?;
    write_x_column_if_present(w, &ds.archetype)?;
    write_y_columns(w, ds, warnings, emitted)?;

    w.end_element_named(tag_name)
}

fn write_floating_note_element(w: &mut XmlWriter, note: &FloatingNote) -> Result<(), FormatError> {
    let color_name = color_to_pzfx_floating_note_name(note.fill_color);
    let left = format!("{}", note.bounds_pt.x.round() as i64);
    let top = format!("{}", note.bounds_pt.y.round() as i64);
    let width = format!("{}", note.bounds_pt.width.round().max(0.0) as i64);
    let height = format!("{}", note.bounds_pt.height.round().max(0.0) as i64);
    w.start_element(
        "FloatingNote",
        &[("ID", note.id.as_str()), ("Color", color_name), ("Left", &left), ("Top", &top), ("Width", &width), ("Height", &height)],
    )?;
    write_pzfx_floating_note_body(w, &note.body_markdown)?;
    w.end_element_named("FloatingNote")
}

fn write_row_titles_if_present(
    w: &mut XmlWriter,
    ds: &DataSheet,
    tag_name: &str,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> Result<(), FormatError> {
    let titles: &[String] = match &ds.archetype {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    let last_non_empty = titles.iter().rposition(|s| !s.is_empty()).map_or(0, |idx| idx + 1);
    if last_non_empty == 0 {
        return Ok(());
    }
    let feat = format!("elem:{tag_name}/RowTitlesColumn");
    record_feature_coverage(ds.id.as_str(), &feat, warnings, emitted);

    w.start_element("RowTitlesColumn", &[("Width", DEFAULT_COLUMN_WIDTH)])?;
    write_compact_subcolumn(w, |w| {
        for label in titles.get(..last_non_empty).unwrap_or_default() {
            if label.is_empty() {
                w.empty_element("d", &[])?;
            } else {
                write_pzfx_markup_element(w, "d", &[], label)?;
            }
        }
        Ok(())
    })?;
    w.end_element_named("RowTitlesColumn")
}

fn write_subcolumn_titles_if_present(
    w: &mut XmlWriter,
    ds: &DataSheet,
    tag_name: &str,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> Result<(), FormatError> {
    let (own_set, flat): (bool, Vec<String>) = match &ds.archetype {
        TableArchetype::Xy(t) => collect_dataset_subcolumn_titles(&t.data_sets, t.y_subcolumn_mode.subcolumn_count()),
        TableArchetype::Column(t) => collect_dataset_subcolumn_titles(&t.data_sets, t.subcolumn_mode.subcolumn_count()),
        TableArchetype::Grouped(t) => collect_dataset_subcolumn_titles(&t.data_sets, t.subcolumn_mode.subcolumn_count()),
        TableArchetype::Contingency(t) => collect_dataset_subcolumn_titles(&t.data_sets, 1),
        TableArchetype::Survival(t) => collect_dataset_subcolumn_titles(&t.cohorts, 1),
        TableArchetype::PartsOfWhole(t) => collect_dataset_subcolumn_titles(&t.data_sets, 1),
        TableArchetype::MultipleVariables(_) => return Ok(()),
        TableArchetype::Nested(nested) => collect_nested_subcolumn_titles(nested),
    };
    if flat.is_empty() {
        return Ok(());
    }
    let feat = format!("elem:{tag_name}/SubColumnTitles");
    record_feature_coverage(ds.id.as_str(), &feat, warnings, emitted);

    w.start_element("SubColumnTitles", &[("OwnSet", if own_set { "1" } else { "0" })])?;
    for title in &flat {
        if title.is_empty() {
            w.empty_element("Subcolumn", &[])?;
        } else {
            write_compact_subcolumn(w, |w| write_pzfx_markup_element(w, "d", &[], title))?;
        }
    }
    w.end_element_named("SubColumnTitles")
}

fn collect_dataset_subcolumn_titles(cols: &[DataSetColumn], subcol_count: usize) -> (bool, Vec<String>) {
    if cols.is_empty() || subcol_count == 0 {
        return (false, Vec::new());
    }
    let any_custom = cols.iter().any(|c| c.subcolumn_titles.iter().any(|t| !t.is_empty()));
    if !any_custom {
        return (false, Vec::new());
    }
    let normalized: Vec<Vec<String>> = cols
        .iter()
        .map(|c| if c.subcolumn_titles.len() == subcol_count { c.subcolumn_titles.clone() } else { vec![String::new(); subcol_count] })
        .collect();
    if let Some(first) = normalized.first()
        && normalized.iter().all(|row| row == first)
    {
        return (false, first.clone());
    }
    (true, normalized.into_iter().flatten().collect())
}

fn collect_nested_subcolumn_titles(nested: &curvebook_doc::NestedTable) -> (bool, Vec<String>) {
    let any_custom = nested.groups.iter().any(|grp| {
        grp.subcolumn_titles.iter().enumerate().any(|(idx, title)| {
            let default_label = format!("{}:{}", grp.title, idx + 1);
            title != &default_label
        })
    });
    if !any_custom {
        return (false, Vec::new());
    }
    if let Some(first) = nested.groups.first()
        && nested.groups.iter().all(|g| g.subcolumn_titles == first.subcolumn_titles)
    {
        return (false, first.subcolumn_titles.clone());
    }
    (true, nested.groups.iter().flat_map(|g| g.subcolumn_titles.iter().cloned()).collect())
}

fn write_x_column_if_present(w: &mut XmlWriter, arch: &TableArchetype) -> Result<(), FormatError> {
    match arch {
        TableArchetype::Xy(xy) => {
            let row_count = xy.row_count();
            let subcols = match &xy.x_mode {
                XColumnMode::GeneratedSequence { .. } => 1,
                other => other.subcolumn_count().max(1),
            };
            let decimals_str = clamp_decimals_str(xy.x_display_decimals.unwrap_or(2));
            let subcols_str = subcols.to_string();
            w.start_element("XColumn", &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", &decimals_str), ("Subcolumns", &subcols_str)])?;
            write_pzfx_markup_element(w, "Title", &[], &xy.x_title)?;
            match &xy.x_mode {
                XColumnMode::GeneratedSequence { .. } => {
                    let populated = (0..row_count).rev().find(|&r| !is_unexcluded_empty_cell(&xy.materialized_x_cell_at(r))).map_or(0, |r| r + 1);
                    if populated == 0 {
                        w.empty_element("Subcolumn", &[])?;
                    } else {
                        write_compact_subcolumn(w, |w| {
                            for r in 0..populated {
                                let cell = xy.materialized_x_cell_at(r);
                                write_d_cell(w, &cell)?;
                            }
                            Ok(())
                        })?;
                    }
                }
                _ => {
                    for s in 0..subcols {
                        let populated = (0..row_count)
                            .rev()
                            .find(|&r| xy.x_cells.get(r).and_then(|row| row.get(s)).is_some_and(|c| !is_unexcluded_empty_cell(c)))
                            .map_or(0, |r| r + 1);
                        if populated == 0 {
                            w.empty_element("Subcolumn", &[])?;
                        } else {
                            write_compact_subcolumn(w, |w| {
                                for r in 0..populated {
                                    let cell = xy.x_cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                                    write_d_cell(w, &cell)?;
                                }
                                Ok(())
                            })?;
                        }
                    }
                }
            }
            w.end_element_named("XColumn")
        }
        TableArchetype::Survival(surv) => {
            w.start_element("XColumn", &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", "2"), ("Subcolumns", "1")])?;
            write_pzfx_markup_element(w, "Title", &[], &surv.time_title)?;
            let populated = trimmed_cell_slice_len(&surv.times);
            if populated == 0 {
                w.empty_element("Subcolumn", &[])?;
            } else {
                write_compact_subcolumn(w, |w| {
                    for cell in surv.times.get(..populated).unwrap_or_default() {
                        write_d_cell(w, cell)?;
                    }
                    Ok(())
                })?;
            }
            w.end_element_named("XColumn")
        }
        _ => Ok(()),
    }
}

fn write_y_columns(
    w: &mut XmlWriter,
    ds: &DataSheet,
    warnings: &mut Vec<PzfxExportWarning>,
    emitted: &mut BTreeSet<(String, String)>,
) -> Result<(), FormatError> {
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            let need_tail_anchor = t.row_count() > 0 && !xy_reaches_row_count(t);
            write_dataset_y_columns(w, &t.data_sets, t.y_subcolumn_mode.subcolumn_count(), 2, need_tail_anchor)
        }
        TableArchetype::Column(t) => {
            let need_tail_anchor = t.row_count() > 0 && !dataset_table_reaches_row_count(&t.row_titles, &t.data_sets, t.row_count());
            write_dataset_y_columns(w, &t.data_sets, t.subcolumn_mode.subcolumn_count(), 2, need_tail_anchor)
        }
        TableArchetype::Grouped(t) => {
            let need_tail_anchor = t.row_count() > 0 && !dataset_table_reaches_row_count(&t.row_titles, &t.data_sets, t.row_count());
            write_dataset_y_columns(w, &t.data_sets, t.subcolumn_mode.subcolumn_count(), 2, need_tail_anchor)
        }
        TableArchetype::Contingency(t) => {
            let need_tail_anchor = t.row_count() > 0 && !dataset_table_reaches_row_count(&t.row_titles, &t.data_sets, t.row_count());
            write_dataset_y_columns(w, &t.data_sets, 1, 0, need_tail_anchor)
        }
        TableArchetype::Survival(t) => {
            let rows = t.times.len().max(t.cohorts.iter().map(DataSetColumn::row_count).max().unwrap_or(0)).max(t.row_titles.len());
            let x_reaches = trimmed_cell_slice_len(&t.times) >= rows;
            let need_tail_anchor = rows > 0 && !x_reaches && !dataset_table_reaches_row_count(&t.row_titles, &t.cohorts, rows);
            write_dataset_y_columns(w, &t.cohorts, 1, 0, need_tail_anchor)
        }
        TableArchetype::PartsOfWhole(t) => {
            let need_tail_anchor = t.row_count() > 0 && !dataset_table_reaches_row_count(&t.slice_labels, &t.data_sets, t.row_count());
            write_dataset_y_columns(w, &t.data_sets, 1, 2, need_tail_anchor)
        }
        TableArchetype::MultipleVariables(mv) => {
            let rows = mv.row_count();
            let rt_reaches = mv.row_titles.iter().rposition(|s| !s.is_empty()).map_or(0, |i| i + 1) >= rows;
            let any_var_reaches = mv.variables.iter().any(|v| trimmed_cell_slice_len(&v.cells) >= rows);
            let need_tail_anchor = rows > 0 && !rt_reaches && !any_var_reaches;
            for (v_idx, var) in mv.variables.iter().enumerate() {
                let decimals_str = clamp_decimals_str(var.display_decimals.unwrap_or(2));
                let mv_type = if !var.role_inferred {
                    match var.role {
                        VariableRole::Categorical => Some("Categorical"),
                        VariableRole::Continuous => Some("Continues"),
                        VariableRole::Label | VariableRole::Calculated => None,
                    }
                } else {
                    None
                };
                if let Some(mv_attr) = mv_type {
                    record_feature_coverage(ds.id.as_str(), "attr:YColumn@MVType", warnings, emitted);
                    w.start_element(
                        "YColumn",
                        &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", &decimals_str), ("Subcolumns", "1"), ("MVType", mv_attr)],
                    )?;
                } else {
                    w.start_element("YColumn", &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", &decimals_str), ("Subcolumns", "1")])?;
                }
                write_pzfx_markup_element(w, "Title", &[], &var.title)?;
                let populated = if need_tail_anchor && v_idx == 0 { var.cells.len() } else { trimmed_cell_slice_len(&var.cells) };
                if populated == 0 {
                    w.empty_element("Subcolumn", &[])?;
                } else {
                    write_compact_subcolumn(w, |w| {
                        for cell in var.cells.get(..populated).unwrap_or_default() {
                            write_d_cell(w, cell)?;
                        }
                        Ok(())
                    })?;
                }
                w.end_element_named("YColumn")?;
            }
            Ok(())
        }
        TableArchetype::Nested(nested) => {
            let rows = nested.row_count();
            let rt_reaches = nested.row_titles.iter().rposition(|s| !s.is_empty()).map_or(0, |i| i + 1) >= rows;
            let any_grp_reaches = nested.groups.iter().any(|g| {
                (0..g.biological_unit_count()).any(|s| {
                    (0..g.cells.len())
                        .rev()
                        .find(|&r| g.cells.get(r).and_then(|row| row.get(s)).is_some_and(|c| !is_unexcluded_empty_cell(c)))
                        .map_or(0, |r| r + 1)
                        >= rows
                })
            });
            let need_tail_anchor = rows > 0 && !rt_reaches && !any_grp_reaches;
            for (g_idx, grp) in nested.groups.iter().enumerate() {
                let units = grp.biological_unit_count().max(1);
                let units_str = units.to_string();
                w.start_element("YColumn", &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", "2"), ("Subcolumns", &units_str)])?;
                write_pzfx_markup_element(w, "Title", &[], &grp.title)?;
                let row_count = grp.cells.len();
                for s in 0..units {
                    let populated = if need_tail_anchor && g_idx == 0 && s == 0 {
                        row_count
                    } else {
                        (0..row_count)
                            .rev()
                            .find(|&r| grp.cells.get(r).and_then(|row| row.get(s)).is_some_and(|c| !is_unexcluded_empty_cell(c)))
                            .map_or(0, |r| r + 1)
                    };
                    if populated == 0 {
                        w.empty_element("Subcolumn", &[])?;
                    } else {
                        write_compact_subcolumn(w, |w| {
                            for r in 0..populated {
                                let cell = grp.cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                                write_d_cell(w, &cell)?;
                            }
                            Ok(())
                        })?;
                    }
                }
                w.end_element_named("YColumn")?;
            }
            Ok(())
        }
    }
}

fn xy_reaches_row_count(xy: &curvebook_doc::XyTable) -> bool {
    let rows = xy.row_count();
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        if rows > 0 && !is_unexcluded_empty_cell(&xy.materialized_x_cell_at(rows - 1)) {
            return true;
        }
    } else if xy.x_cells.get(rows.saturating_sub(1)).is_some_and(|row| row.iter().any(|c| !is_unexcluded_empty_cell(c))) {
        return true;
    }
    dataset_table_reaches_row_count(&xy.row_titles, &xy.data_sets, rows)
}

fn dataset_table_reaches_row_count(row_titles: &[String], cols: &[DataSetColumn], rows: usize) -> bool {
    if rows == 0 {
        return true;
    }
    if row_titles.get(rows - 1).is_some_and(|s| !s.is_empty()) {
        return true;
    }
    cols.iter().any(|col| (0..col.subcolumn_count()).any(|s| col.cell(rows - 1, s).is_some_and(|c| !is_unexcluded_empty_cell(c))))
}

fn write_dataset_y_columns(
    w: &mut XmlWriter,
    cols: &[DataSetColumn],
    subcol_count: usize,
    default_decimals: u8,
    preserve_tail_rows: bool,
) -> Result<(), FormatError> {
    let subs_str = subcol_count.to_string();
    for (c_idx, col) in cols.iter().enumerate() {
        let dec_str = clamp_decimals_str(col.display_decimals.unwrap_or(default_decimals));
        w.start_element("YColumn", &[("Width", DEFAULT_COLUMN_WIDTH), ("Decimals", &dec_str), ("Subcolumns", &subs_str)])?;
        write_pzfx_markup_element(w, "Title", &[], &col.title)?;
        let row_count = col.row_count();
        for s in 0..subcol_count {
            let populated = if preserve_tail_rows && c_idx == 0 && s == 0 {
                row_count
            } else {
                (0..row_count).rev().find(|&r| col.cell(r, s).is_some_and(|c| !is_unexcluded_empty_cell(c))).map_or(0, |r| r + 1)
            };
            if populated == 0 {
                w.empty_element("Subcolumn", &[])?;
            } else {
                write_compact_subcolumn(w, |w| {
                    for r in 0..populated {
                        let cell = col.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                        write_d_cell(w, &cell)?;
                    }
                    Ok(())
                })?;
            }
        }
        w.end_element_named("YColumn")?;
    }
    Ok(())
}

/// Emit `<Subcolumn>` indented on its own line while keeping child `<d>` elements compact
/// (zero per-cell indentation bloat on huge tables, deterministic, and §6.3 conformant).
fn write_compact_subcolumn<F>(w: &mut XmlWriter, write_cells: F) -> Result<(), FormatError>
where
    F: FnOnce(&mut XmlWriter) -> Result<(), FormatError>,
{
    w.start_element("Subcolumn", &[])?;
    w.text("")?;
    write_cells(w)?;
    w.end_element_named("Subcolumn")
}

const fn is_unexcluded_empty_cell(cell: &Cell) -> bool {
    !cell.excluded && matches!(cell.value, CellValue::Empty)
}

fn trimmed_cell_slice_len(cells: &[Cell]) -> usize {
    cells.iter().rposition(|c| !is_unexcluded_empty_cell(c)).map_or(0, |idx| idx + 1)
}

fn write_d_cell(w: &mut XmlWriter, cell: &Cell) -> Result<(), FormatError> {
    let attrs: &[(&str, &str)] = if cell.excluded { &[("Excluded", "1")] } else { &[] };
    match &cell.value {
        CellValue::Empty => w.empty_element("d", attrs),
        CellValue::Number(v) => {
            if !v.is_finite() {
                return Err(FormatError::NonFiniteFloat { location: String::from("pzfx.d") });
            }
            w.text_element("d", attrs, &format_pzfx_f64(*v))
        }
        CellValue::Text(s) => {
            if s.is_empty() {
                w.empty_element("d", attrs)
            } else {
                write_pzfx_markup_element(w, "d", attrs, s)
            }
        }
    }
}

fn clamp_decimals_str(decimals: u8) -> String {
    decimals.min(MAX_DISPLAY_DECIMALS).to_string()
}

fn format_pzfx_f64(value: f64) -> String {
    let normalized = if value == 0.0 { 0.0 } else { value };
    normalized.to_string()
}

// ---------------------------------------------------------------------------
// §4.3 / §6.2 Reusable Data-Layer Comparison Helper
// ---------------------------------------------------------------------------

/// Single discrepancy recorded by [`compare_pzfx_data_layer`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PzfxDataLayerDifference {
    pub location: String,
    pub expected: String,
    pub actual: String,
}

/// Normalized tabular and info data-layer comparison result for `.pzfx` round-trip and re-export
/// fidelity verification (§4.3, §6.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PzfxDataLayerDiff {
    pub tables_compared: usize,
    pub cells_compared: usize,
    pub info_sheets_compared: usize,
    pub differences: Vec<PzfxDataLayerDifference>,
}

impl PzfxDataLayerDiff {
    /// `true` when zero data-layer discrepancies were detected.
    #[must_use]
    pub fn is_identical(&self) -> bool {
        self.differences.is_empty()
    }

    fn push(&mut self, location: impl Into<String>, expected: impl Into<String>, actual: impl Into<String>) {
        self.differences.push(PzfxDataLayerDifference { location: location.into(), expected: expected.into(), actual: actual.into() });
    }
}

/// Compare the representable tabular data layer, exclusions, display metadata, floating notes,
/// info sheets, and active selection between `expected` and `actual` (§4.3 / §6.2).
#[must_use]
pub fn compare_pzfx_data_layer(expected: &ProjectDocument, actual: &ProjectDocument) -> PzfxDataLayerDiff {
    let mut diff = PzfxDataLayerDiff::default();

    if expected.data_sheets.len() != actual.data_sheets.len() {
        diff.push("data_sheets.len", expected.data_sheets.len().to_string(), actual.data_sheets.len().to_string());
    }
    for (idx, (exp_ds, act_ds)) in expected.data_sheets.iter().zip(actual.data_sheets.iter()).enumerate() {
        diff.tables_compared += 1;
        compare_data_sheet(idx, exp_ds, act_ds, &mut diff);
    }

    if expected.info_sheets.len() != actual.info_sheets.len() {
        diff.push("info_sheets.len", expected.info_sheets.len().to_string(), actual.info_sheets.len().to_string());
    }
    for (idx, (exp_info, act_info)) in expected.info_sheets.iter().zip(actual.info_sheets.iter()).enumerate() {
        diff.info_sheets_compared += 1;
        compare_info_sheet(idx, exp_info, act_info, &mut diff);
    }

    if let Some(exp_active) = &expected.active_sheet_id
        && (expected.data_sheet(exp_active).is_some() || expected.info_sheet(exp_active).is_some())
        && actual.active_sheet_id.as_ref() != Some(exp_active)
    {
        diff.push("active_sheet_id", exp_active.as_str(), actual.active_sheet_id.as_ref().map_or("<none>", SheetId::as_str));
    }

    diff
}

fn compare_data_sheet(idx: usize, exp: &DataSheet, act: &DataSheet, diff: &mut PzfxDataLayerDiff) {
    let prefix = format!("data[{idx}:{}]", exp.id);
    if exp.id != act.id {
        diff.push(format!("{prefix}.id"), exp.id.as_str(), act.id.as_str());
    }
    if exp.title != act.title {
        diff.push(format!("{prefix}.title"), &exp.title, &act.title);
    }
    if exp.kind() != act.kind() {
        diff.push(format!("{prefix}.kind"), format!("{:?}", exp.kind()), format!("{:?}", act.kind()));
        return;
    }

    compare_floating_notes(&prefix, &exp.floating_notes, &act.floating_notes, diff);
    if let Some(exp_color) = exp.color_tag {
        let (exp_hl, _) = color_to_pzfx_highlighted(exp_color);
        let act_hl = act.color_tag.map(|c| color_to_pzfx_highlighted(c).0);
        if act_hl != Some(exp_hl) {
            diff.push(format!("{prefix}.color_tag"), exp_hl, act_hl.unwrap_or("<none>"));
        }
    }

    match (&exp.archetype, &act.archetype) {
        (TableArchetype::Xy(e), TableArchetype::Xy(a)) => compare_xy_table(&prefix, e, a, diff),
        (TableArchetype::Column(e), TableArchetype::Column(a)) => {
            if e.subcolumn_mode != a.subcolumn_mode {
                diff.push(format!("{prefix}.subcolumn_mode"), format!("{:?}", e.subcolumn_mode), format!("{:?}", a.subcolumn_mode));
            }
            compare_row_titles(&prefix, &e.row_titles, &a.row_titles, diff);
            compare_dataset_columns(&prefix, &e.data_sets, &a.data_sets, diff);
        }
        (TableArchetype::Grouped(e), TableArchetype::Grouped(a)) => {
            if e.subcolumn_mode != a.subcolumn_mode {
                diff.push(format!("{prefix}.subcolumn_mode"), format!("{:?}", e.subcolumn_mode), format!("{:?}", a.subcolumn_mode));
            }
            compare_row_titles(&prefix, &e.row_titles, &a.row_titles, diff);
            compare_dataset_columns(&prefix, &e.data_sets, &a.data_sets, diff);
        }
        (TableArchetype::Contingency(e), TableArchetype::Contingency(a)) => {
            compare_row_titles(&prefix, &e.row_titles, &a.row_titles, diff);
            compare_dataset_columns(&prefix, &e.data_sets, &a.data_sets, diff);
        }
        (TableArchetype::Survival(e), TableArchetype::Survival(a)) => compare_survival_table(&prefix, e, a, diff),
        (TableArchetype::PartsOfWhole(e), TableArchetype::PartsOfWhole(a)) => {
            compare_row_titles(&prefix, &e.slice_labels, &a.slice_labels, diff);
            compare_dataset_columns(&prefix, &e.data_sets, &a.data_sets, diff);
        }
        (TableArchetype::MultipleVariables(e), TableArchetype::MultipleVariables(a)) => compare_mv_table(&prefix, e, a, diff),
        (TableArchetype::Nested(e), TableArchetype::Nested(a)) => compare_nested_table(&prefix, e, a, diff),
        _ => {}
    }
}

fn compare_xy_table(prefix: &str, e: &curvebook_doc::XyTable, a: &curvebook_doc::XyTable, diff: &mut PzfxDataLayerDiff) {
    if e.x_title != a.x_title {
        diff.push(format!("{prefix}.x_title"), &e.x_title, &a.x_title);
    }
    if let Some(dec) = e.x_display_decimals
        && a.x_display_decimals != Some(dec)
    {
        diff.push(format!("{prefix}.x_display_decimals"), dec.to_string(), format!("{:?}", a.x_display_decimals));
    }
    if e.y_subcolumn_mode != a.y_subcolumn_mode {
        diff.push(format!("{prefix}.y_subcolumn_mode"), format!("{:?}", e.y_subcolumn_mode), format!("{:?}", a.y_subcolumn_mode));
    }
    let x_mode_matches = match (&e.x_mode, &a.x_mode) {
        (XColumnMode::GeneratedSequence { .. }, _) => true,
        (XColumnMode::DateOrElapsedTime { unit_label: e_unit }, XColumnMode::DateOrElapsedTime { unit_label: a_unit }) => {
            e_unit == a_unit || (e_unit != &e.x_title && a_unit == &e.x_title)
        }
        _ => e.x_mode == a.x_mode,
    };
    if !x_mode_matches {
        diff.push(format!("{prefix}.x_mode"), format!("{:?}", e.x_mode), format!("{:?}", a.x_mode));
    }
    compare_row_titles(prefix, &e.row_titles, &a.row_titles, diff);
    let rows = e.row_count().max(a.row_count());
    let x_subs = match &e.x_mode {
        XColumnMode::GeneratedSequence { .. } => 1,
        other => other.subcolumn_count(),
    };
    for r in 0..rows {
        for s in 0..x_subs {
            let e_cell = match &e.x_mode {
                XColumnMode::GeneratedSequence { .. } => e.materialized_x_cell_at(r),
                _ => e.x_cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty),
            };
            let a_cell = a.x_cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
            compare_cell(&format!("{prefix}.x[{r}][{s}]"), &e_cell, &a_cell, diff);
        }
    }
    compare_dataset_columns(prefix, &e.data_sets, &a.data_sets, diff);
}

fn compare_survival_table(prefix: &str, e: &curvebook_doc::SurvivalTable, a: &curvebook_doc::SurvivalTable, diff: &mut PzfxDataLayerDiff) {
    if e.time_title != a.time_title {
        diff.push(format!("{prefix}.time_title"), &e.time_title, &a.time_title);
    }
    compare_row_titles(prefix, &e.row_titles, &a.row_titles, diff);
    let rows = e.times.len().max(a.times.len());
    for r in 0..rows {
        let e_cell = e.times.get(r).cloned().unwrap_or_else(Cell::empty);
        let a_cell = a.times.get(r).cloned().unwrap_or_else(Cell::empty);
        compare_cell(&format!("{prefix}.time[{r}]"), &e_cell, &a_cell, diff);
    }
    compare_dataset_columns(prefix, &e.cohorts, &a.cohorts, diff);
}

fn compare_mv_table(
    prefix: &str,
    e: &curvebook_doc::MultipleVariablesTable,
    a: &curvebook_doc::MultipleVariablesTable,
    diff: &mut PzfxDataLayerDiff,
) {
    compare_row_titles(prefix, &e.row_titles, &a.row_titles, diff);
    if e.variables.len() != a.variables.len() {
        diff.push(format!("{prefix}.variables.len"), e.variables.len().to_string(), a.variables.len().to_string());
    }
    for (c, (ev, av)) in e.variables.iter().zip(a.variables.iter()).enumerate() {
        if ev.title != av.title {
            diff.push(format!("{prefix}.var[{c}].title"), &ev.title, &av.title);
        }
        if let Some(dec) = ev.display_decimals
            && av.display_decimals != Some(dec)
        {
            diff.push(format!("{prefix}.var[{c}].display_decimals"), dec.to_string(), format!("{:?}", av.display_decimals));
        }
        if matches!(ev.role, VariableRole::Continuous | VariableRole::Categorical) && ev.role != av.role {
            diff.push(format!("{prefix}.var[{c}].role"), format!("{:?}", ev.role), format!("{:?}", av.role));
        }
        let rows = ev.cells.len().max(av.cells.len());
        for r in 0..rows {
            let e_cell = ev.cells.get(r).cloned().unwrap_or_else(Cell::empty);
            let a_cell = av.cells.get(r).cloned().unwrap_or_else(Cell::empty);
            compare_cell(&format!("{prefix}.var[{c}][{r}]"), &e_cell, &a_cell, diff);
        }
    }
}

fn compare_nested_table(prefix: &str, e: &curvebook_doc::NestedTable, a: &curvebook_doc::NestedTable, diff: &mut PzfxDataLayerDiff) {
    compare_row_titles(prefix, &e.row_titles, &a.row_titles, diff);
    if e.groups.len() != a.groups.len() {
        diff.push(format!("{prefix}.groups.len"), e.groups.len().to_string(), a.groups.len().to_string());
    }
    for (g, (eg, ag)) in e.groups.iter().zip(a.groups.iter()).enumerate() {
        if eg.title != ag.title {
            diff.push(format!("{prefix}.group[{g}].title"), &eg.title, &ag.title);
        }
        if eg.subcolumn_titles != ag.subcolumn_titles {
            diff.push(format!("{prefix}.group[{g}].subcolumn_titles"), format!("{:?}", eg.subcolumn_titles), format!("{:?}", ag.subcolumn_titles));
        }
        let rows = eg.cells.len().max(ag.cells.len());
        let subs = eg.biological_unit_count().max(ag.biological_unit_count());
        for r in 0..rows {
            for s in 0..subs {
                let e_cell = eg.cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                let a_cell = ag.cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                compare_cell(&format!("{prefix}.group[{g}][{r}][{s}]"), &e_cell, &a_cell, diff);
            }
        }
    }
}

fn compare_dataset_columns(prefix: &str, exp_cols: &[DataSetColumn], act_cols: &[DataSetColumn], diff: &mut PzfxDataLayerDiff) {
    if exp_cols.len() != act_cols.len() {
        diff.push(format!("{prefix}.cols.len"), exp_cols.len().to_string(), act_cols.len().to_string());
    }
    for (c, (ec, ac)) in exp_cols.iter().zip(act_cols.iter()).enumerate() {
        if ec.title != ac.title {
            diff.push(format!("{prefix}.col[{c}].title"), &ec.title, &ac.title);
        }
        if !ec.subcolumn_titles.is_empty() && ec.subcolumn_titles != ac.subcolumn_titles {
            diff.push(format!("{prefix}.col[{c}].subcolumn_titles"), format!("{:?}", ec.subcolumn_titles), format!("{:?}", ac.subcolumn_titles));
        }
        if let Some(dec) = ec.display_decimals
            && ac.display_decimals != Some(dec)
        {
            diff.push(format!("{prefix}.col[{c}].display_decimals"), dec.to_string(), format!("{:?}", ac.display_decimals));
        }
        let rows = ec.row_count().max(ac.row_count());
        let subs = ec.subcolumn_count().max(ac.subcolumn_count());
        for r in 0..rows {
            for s in 0..subs {
                let e_cell = ec.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                let a_cell = ac.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                compare_cell(&format!("{prefix}.col[{c}][{r}][{s}]"), &e_cell, &a_cell, diff);
            }
        }
    }
}

fn compare_row_titles(prefix: &str, exp: &[String], act: &[String], diff: &mut PzfxDataLayerDiff) {
    let rows = exp.len().max(act.len());
    for r in 0..rows {
        let e = exp.get(r).map_or("", String::as_str);
        let a = act.get(r).map_or("", String::as_str);
        if e != a {
            diff.push(format!("{prefix}.row_title[{r}]"), e, a);
        }
    }
}

fn compare_floating_notes(prefix: &str, exp: &[FloatingNote], act: &[FloatingNote], diff: &mut PzfxDataLayerDiff) {
    let exp_non_empty: Vec<&FloatingNote> = exp.iter().filter(|n| !n.body_markdown.trim().is_empty()).collect();
    let act_non_empty: Vec<&FloatingNote> = act.iter().filter(|n| !n.body_markdown.trim().is_empty()).collect();
    if exp_non_empty.len() != act_non_empty.len() {
        diff.push(format!("{prefix}.floating_notes.len"), exp_non_empty.len().to_string(), act_non_empty.len().to_string());
        return;
    }
    for (idx, (en, an)) in exp_non_empty.iter().zip(act_non_empty.iter()).enumerate() {
        if en.body_markdown.trim() != an.body_markdown.trim() {
            diff.push(format!("{prefix}.floating_notes[{idx}].body"), en.body_markdown.trim(), an.body_markdown.trim());
        }
    }
}

fn compare_info_sheet(idx: usize, exp: &InfoSheet, act: &InfoSheet, diff: &mut PzfxDataLayerDiff) {
    let prefix = format!("info[{idx}:{}]", exp.id);
    if exp.id != act.id {
        diff.push(format!("{prefix}.id"), exp.id.as_str(), act.id.as_str());
    }
    if exp.title != act.title {
        diff.push(format!("{prefix}.title"), &exp.title, &act.title);
    }
    if exp.notes_markdown.trim() != act.notes_markdown.trim() {
        diff.push(format!("{prefix}.notes_markdown"), exp.notes_markdown.trim(), act.notes_markdown.trim());
    }
    if exp.constants.len() != act.constants.len() {
        diff.push(format!("{prefix}.constants.len"), exp.constants.len().to_string(), act.constants.len().to_string());
    }
    for (c_idx, (ec, ac)) in exp.constants.iter().zip(act.constants.iter()).enumerate() {
        if ec.name != ac.name {
            diff.push(format!("{prefix}.constant[{c_idx}].name"), &ec.name, &ac.name);
        }
        if !constant_values_equal(&ec.value, &ac.value) {
            diff.push(format!("{prefix}.constant[{c_idx}].value"), format!("{:?}", ec.value), format!("{:?}", ac.value));
        }
    }
}

fn compare_cell(location: &str, exp: &Cell, act: &Cell, diff: &mut PzfxDataLayerDiff) {
    diff.cells_compared += 1;
    if exp.excluded != act.excluded {
        diff.push(format!("{location}.excluded"), exp.excluded.to_string(), act.excluded.to_string());
    }
    if !cell_values_equal(&exp.value, &act.value) {
        diff.push(location, format!("{:?}", exp.value), format!("{:?}", act.value));
    }
}

fn cell_values_equal(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Empty, CellValue::Empty) => true,
        (CellValue::Number(x), CellValue::Number(y)) => floats_close(*x, *y),
        (CellValue::Text(x), CellValue::Text(y)) => x == y,
        _ => false,
    }
}

fn constant_values_equal(a: &ConstantValue, b: &ConstantValue) -> bool {
    match (a, b) {
        (ConstantValue::Empty, ConstantValue::Empty) => true,
        (ConstantValue::Number(x), ConstantValue::Number(y)) => floats_close(*x, *y),
        (ConstantValue::Date(x), ConstantValue::Date(y)) => x == y,
        (ConstantValue::Text(x), ConstantValue::Text(y)) => x.trim() == y.trim(),
        _ => false,
    }
}

fn floats_close(a: f64, b: f64) -> bool {
    let a = if a == 0.0 { 0.0 } else { a };
    let b = if b == 0.0 { 0.0 } else { b };
    let tol = 1e-12 * a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= tol
}

/// Inspect the structural features emitted by a `.pzfx` XML payload for conformance auditing
/// (useful for synthetic verification tests).
#[cfg(test)]
pub(crate) fn collect_emitted_pzfx_features(xml: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let Ok(root) = crate::xml::parse_xml(xml) else {
        return counts;
    };
    walk_feature_counts(&root, true, &mut counts);
    counts
}

#[cfg(test)]
fn walk_feature_counts(el: &crate::xml::XmlElement, is_root: bool, counts: &mut BTreeMap<String, usize>) {
    let elem_name = if is_root { "<Root>" } else { el.local_name() };
    for (k, _) in &el.attrs {
        let local_k = k.rsplit_once(':').map_or(k.as_str(), |(_, l)| l);
        *counts.entry(format!("attr:{elem_name}@{local_k}")).or_insert(0) += 1;
    }
    for child in el.child_elements() {
        *counts.entry(format!("elem:{elem_name}/{}", child.local_name())).or_insert(0) += 1;
        walk_feature_counts(child, false, counts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pzfx::import::import_pzfx_bytes;

    #[test]
    fn coverage_table_never_overclaims_conformant_below_threshold() {
        let mut seen = BTreeSet::new();
        for row in PZFX_FEATURE_CORPUS_COVERAGE {
            assert!(seen.insert(row.feature), "duplicate coverage row `{}`", row.feature);
            if row.status == PzfxFeatureCoverageStatus::Conformant {
                assert!(
                    row.corpus_files >= 3 && row.corpus_sources >= 2,
                    "row `{}` marked Conformant with ({}, {})",
                    row.feature,
                    row.corpus_files,
                    row.corpus_sources
                );
            }
        }
    }

    #[test]
    fn export_emits_required_features_and_zero_excluded_creator_attributes() {
        let mut doc = ProjectDocument::new("Deterministic Study").unwrap();
        let mut xy = curvebook_doc::XyTable::new(SubcolumnMode::MeanSd, 1, 2).unwrap();
        xy.x_title = String::from("Dose");
        xy.x_display_decimals = Some(1);
        xy.x_cells[0][0] = Cell::number(1.0);
        xy.x_cells[1][0] = Cell::number(10.0);
        xy.data_sets[0].title = String::from("Response");
        xy.data_sets[0].display_decimals = Some(3);
        xy.data_sets[0].cells[0][0] = Cell::number(12.5);
        xy.data_sets[0].cells[0][1] = Cell::number(0.8);
        xy.data_sets[0].cells[1][0] = Cell::excluded_number(99.0);
        xy.data_sets[0].cells[1][1] = Cell::number(1.2);
        doc.add_data_sheet(DataSheet::new("data-1", "Curve", TableArchetype::Xy(xy)).unwrap()).unwrap();

        let rep1 = export_pzfx_bytes(&doc).unwrap();
        let rep2 = export_pzfx_bytes(&doc).unwrap();
        assert_eq!(rep1.bytes, rep2.bytes, "export must be byte-identical");
        assert!(rep1.warnings.is_empty(), "expected zero warnings for conformant subset: {:?}", rep1.warnings);

        let xml = std::str::from_utf8(&rep1.bytes).unwrap();
        let feats = collect_emitted_pzfx_features(xml);
        for required in [
            "attr:<Root>@PrismXMLVersion",
            "elem:<Root>/Created",
            "elem:<Root>/InfoSequence",
            "elem:<Root>/TableSequence",
            "elem:TableSequence/Ref",
            "attr:Ref@ID",
            "attr:Ref@Selected",
            "attr:YColumn@Width",
            "attr:YColumn@Decimals",
            "attr:YColumn@Subcolumns",
            "elem:YColumn/Title",
            "elem:YColumn/Subcolumn",
            "elem:Subcolumn/d",
        ] {
            assert!(feats.contains_key(required), "missing required feature `{required}` in {feats:?}");
        }
        for excluded in [
            "attr:OriginalVersion@CreatedByProgram",
            "attr:OriginalVersion@Login",
            "attr:OriginalVersion@DateTime",
            "elem:<Root>/Template",
            "elem:<Root>/Title",
        ] {
            assert!(!feats.contains_key(excluded), "emitted forbidden feature `{excluded}`");
        }

        let reimported = import_pzfx_bytes(&rep1.bytes).unwrap();
        assert!(reimported.warnings.is_empty(), "reimport warnings: {:?}", reimported.warnings);
        let diff = compare_pzfx_data_layer(&doc, &reimported.document);
        assert!(diff.is_identical(), "round-trip diff: {diff:?}");
    }
}
