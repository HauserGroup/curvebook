//! Document model for CurveBook projects: sheet families (`Data`, `Info`, `Analyses`, `Graphs`,
//! `Layouts`), the eight typed scientific data-table schemas, replicate subcolumns, cell-exclusion
//! masks, hookable `InfoSheet` constants, `AnalysisSpec` / `ResultSheet`, `GraphSpec` / `LayoutSpec`,
//! and reactive dependency links between tables, analyses, and figures.
//!
//! Layer L1 crate: pure data structures and structural validation, free of statistical solvers or
//! rendering backends. Governing spec: `docs/implementation/data-tables-and-transforms.md`.
//!
//! ```
//! use curvebook_doc::{Cell, CellCoord, DataSheet, SubcolumnMode, TableArchetype, TableKind, XyTable};
//!
//! let xy = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap();
//! let mut sheet = DataSheet::new("data-1", "Dose Response", TableArchetype::Xy(xy)).unwrap();
//! assert_eq!(sheet.kind(), TableKind::Xy);
//! sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(12.5)).unwrap();
//! sheet.set_excluded(CellCoord::new(0, 0, 0), true).unwrap();
//! assert_eq!(sheet.cell(CellCoord::new(0, 0, 0)).unwrap().active_number(), None);
//! ```
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]
#![deny(missing_docs)]

pub mod analysis;
pub mod archetypes;
pub mod cell;
pub mod delimited;
pub mod error;
pub mod graph;
pub mod ids;
pub mod image;
pub mod info;
pub mod layout;
pub mod links;
pub mod param_spellings;
pub mod project;
pub mod result;
pub mod result_builder;
pub mod schema;

pub use analysis::{
    ALL_ANALYSIS_DESCRIPTORS, AnalysisDomainKind, AnalysisKind, AnalysisKindDescriptor, AnalysisKindId, AnalysisRecomputeStrategy, AnalysisSpec,
    AnalysisUiCategory, BaselineOperation, BlandAltmanSpec, ClusterDistance, ClusterLinkage, ColumnTitleHookError, ContingencyMethod,
    ContingencySpec, ConvergenceStrictness, CorrelationPowerMethod, CoxTieMethod, DemingErrorSpec, DerivedColumnFormula, DescriptiveSpec,
    ErrorPropagationMode, FactorialAnovaSpec, FitFlagRules, FitMethod, GlmFamily, GlmSpec, IdentifiabilityMode, KappaWeighting, LinearRegressionSpec,
    LowessPreset, MathTransformPreset, MixedModelDfMethod, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec, MultipleRowTestsSpec,
    MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OneWayMethod,
    OutlierHandling, OutlierSpec, PValueFormat, PValueStackMethod, PValueStackSpec, ParameterRule, ParameterScope, PassingBablokSpec,
    PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerSolveTarget, PowerTailMode, PowerTestFamily, RepeatedMeasuresDesign,
    ReplicateFitMode, ResidualCovariance, RocCiMethod, RocSpec, SimpleLogisticSpec, SimulationNoise, SplineLowessMethod, SplineLowessSpec,
    SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, TwoProportionsMethod,
    WeightingScheme, WeightingSource, all_analysis_descriptors, default_analysis_kind_id_for_table, descriptor_by_serde_variant,
    descriptor_by_shorthand, descriptor_by_slug, descriptor_by_ui_catalog_id, descriptor_for_id, descriptor_for_kind,
    parse_column_title_numeric_hook, resolve_analysis_descriptor,
};
pub use archetypes::{
    CategoricalLevelSummary, CellCoord, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, GroupedTable, LevelOrderKind, LevelOrderSpec,
    MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable, SequenceProgression, SliceFraction, SurvivalObservation,
    SurvivalStatus, SurvivalTable, TableArchetype, ThreeWayColumnGrouping, VariableColumn, VariableRole, VariableSource, XColumnMode, XyTable,
    default_column_label,
};
pub use cell::{
    Cell, CellValue, ExplicitErrorBounds, ReplicateStacking, SubcolumnMode, SummarizedPoint, infer_number_separators, parse_import_number,
    strip_thousands_grouping,
};
pub use delimited::{DelimitedField, DelimitedScanError, DelimitedScanOptions, DelimitedVisitorError, format_linked_paste_tsv_cell, scan_delimited};
pub use error::{
    CURRENT_SCHEMA_VERSION, ConfidenceLevel, DocError, MAX_DATA_SETS, MAX_PROJECT_CELLS, MAX_ROWS, MAX_SHEET_CELLS, MAX_SHEETS_PER_PROJECT,
    MAX_SUBCOLUMNS, MAX_TITLE_BYTES, SummaryPointError, validate_cell_grid, validate_confidence_level, validate_data_sets, validate_project_cells,
    validate_rows, validate_sheet_cells, validate_subcolumns, validate_title,
};
pub use graph::{
    AdditionalTick, AnnotationShapeKind, AreaFillSpec, AxisScale, AxisSegment, AxisSpec, AxisThreshold, BracketLabelMode, BracketStyle,
    BreakOrnament, CartesianOrientation, CategoricalTickSource, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec, ConnectingLineStyle,
    DiagnosticPlotKind, EmbeddedTableStyle, EquationDisplayMode, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphAnnotation,
    GraphAxisScale, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, HeatmapColumnLabelAngle, HeatmapColumnLabelMode,
    HeatmapLabelSpec, HeatmapRowLabelMode, LegendSpec, LineConnectionKind, LogitDomain, MultivariateGraphEncoding, MultivariatePlotMode,
    PairwiseBracketSpec, PairwiseDisplayMode, PartsOfWholeLayout, PointStyleOverride, RegressionOverlaySpec, ReplicateLineMode,
    ReplicateOverlayStyle, SeriesOffsetSpec, SeriesSpec, SizeChannelMapping, SummaryMeasure, SurvivalDisplayMetric, TickDirection, TickLabelFormat,
    WhiskerMethod,
};
pub use ids::{FloatingNote, HookSource, NoteId, RectPt, SheetId, SizePt, SourceCellId, extract_template_hooks};
pub use image::{
    CropInsetsPt, ImageBorderSpec, ImageMimeKind, MAX_IMAGE_ASSET_BYTES, MAX_IMAGE_DIM_PX, MAX_IMAGE_PIXELS, MAX_PROJECT_IMAGE_ASSETS,
    ProjectImageAsset, content_addressed_asset_id, is_valid_asset_id, sha256_bytes,
};
pub use info::{ConstantValue, InfoConstant, InfoSheet, STANDARD_INFO_KEYS};
pub use layout::{
    CanvasResizeMode, FigureCanvasPreset, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MAX_LAYOUT_ITEMS, PageOrientation,
    PanelBadgeStyle, PhysicalUnit,
};
pub use links::{
    CellExclusionNote, CloneDataSheetOptions, DataSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkBorderColor,
    LinkSource, LinkedDataRegion, PlacementMode, SheetCellRange, transpose_cell_grid,
};
pub use project::{NamedCheckpoint, ProjectDocument, SheetFolderKind};
pub use result::{
    ChecklistItem, ChecklistStatus, CiBound, CurveSamplePoint, ErrorCategory, ErrorCode, FittedCurveOverlay, PairwiseComparisonRecord,
    RemediationKind, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, format_significant_digits,
};
pub use result_builder::{
    KvTabBuilder, TabBuilder, bool_cell, ci_cell, finite_ci_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell, p_cell_at_alpha, text_cell,
};
pub use schema::{
    AdditionalProperties, DRAFT_2020_12_SCHEMA_URI, JsonSchema, PROJECT_DOCUMENT_SCHEMA_ID, SchemaLiteral, SchemaNode, SchemaObject, SchemaType,
    SchemaTypeSpec, any_of_schema, array_schema, bool_schema, const_integer_schema, const_string_schema, enum_schema, integer_schema, map_schema,
    null_schema, nullable_schema, number_schema, object_schema, one_of_schema, project_document_schema, ref_schema, string_schema,
};

/// The eight typed scientific data-table families in a CurveBook project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TableKind {
    /// Continuous or staggered independent variable *X* paired with one or more response series *Y* (with optional *X*/*Y* replicate or summary subcolumns).
    Xy,
    /// One-way categorical grouping where each column represents an independent or matched treatment group.
    Column,
    /// Two-way (or three-way column-grouped) factorial table cross-classifying row factor levels against column data sets with replicate or summary subcolumns.
    Grouped,
    /// Cross-tabulated non-negative integer event counts (`r x c`) for categorical association, relative risk, odds ratio, and kappa agreement analyses.
    Contingency,
    /// Time-to-event observations pairing elapsed follow-up time *X* with per-arm censoring/event indicator codes (`1` = event, `0` = censored).
    Survival,
    /// Single-column non-negative slice magnitudes and labels representing proportions or totals of a whole.
    PartsOfWhole,
    /// Multivariate tidy observation matrix where rows are subjects/cases and columns carry typed [`VariableRole`] assignments.
    MultipleVariables,
    /// Hierarchical two-level nested design where each treatment column holds subcolumns corresponding to experimental sub-units (for example litters or cages) and rows hold technical replicates.
    Nested,
}

impl TableKind {
    /// Complete ordered list of all eight [`TableKind`] table archetypes.
    pub const ALL: [Self; 8] =
        [Self::Xy, Self::Column, Self::Grouped, Self::Contingency, Self::Survival, Self::PartsOfWhole, Self::MultipleVariables, Self::Nested];

    /// Neutral human-readable label for dialogs and inspectors.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Column => "Column",
            Self::Grouped => "Grouped",
            Self::Contingency => "Contingency",
            Self::Survival => "Survival",
            Self::PartsOfWhole => "Parts of Whole",
            Self::MultipleVariables => "Multiple Variables",
            Self::Nested => "Nested",
        }
    }

    /// Check whether `mode` is permitted as a primary subcolumn layout on this table family
    /// (`docs/implementation/data-tables-and-transforms.md` §2).
    #[must_use]
    pub const fn allows_subcolumn_mode(self, mode: SubcolumnMode) -> bool {
        match self {
            Self::Xy => true,
            Self::Column => {
                matches!(mode, SubcolumnMode::Single | SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN)
            }
            Self::Grouped => !matches!(mode, SubcolumnMode::Single),
            Self::Contingency | Self::Survival | Self::PartsOfWhole | Self::MultipleVariables => matches!(mode, SubcolumnMode::Single),
            Self::Nested => matches!(mode, SubcolumnMode::RawReplicates(_)),
        }
    }
}

/// Top-level sheet sections in a CurveBook project navigator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SheetSection {
    /// Primary and linked [`DataSheet`] tables across all eight [`TableKind`] archetypes.
    Data,
    /// Structured [`InfoSheet`] laboratory metadata constants and Markdown experimental notes.
    Info,
    /// Reactive [`ResultSheet`] outputs produced by statistical tests, curve fits, and data transforms.
    Analyses,
    /// Scientific [`GraphSheet`] figures bound to upstream data and result sheets.
    Graphs,
    /// Multi-panel publication [`LayoutSheet`] canvases arranging live graphs, tables, and callouts.
    Layouts,
}

#[cfg(test)]
mod tests {
    use curvebook_palette::{LineDashStyle, PaletteId, Rgba8};

    use super::{
        AnalysisKind, AnalysisSpec, AxisSegment, Cell, CellCoord, CiBound, ColumnDesignHint, ColumnTable, ConfidenceLevel, ConstantValue,
        ContingencyTable, DataSheet, DocError, FloatingNote, GraphSheet, GraphSpec, GraphicalDataObject, GroupedTable, HookSource, ImportFilterSpec,
        ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LevelOrderKind,
        LevelOrderSpec, LinkBorderColor, LinkSource, LinkedDataRegion, MAX_ROWS, MAX_SUBCOLUMNS, MultipleVariablesTable, NestedTable,
        NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, PanelBadgeStyle, ParameterRule, ParameterScope, PartsOfWholeTable,
        PhysicalUnit, ProjectDocument, RectPt, ResultCell, ResultScalarEntry, ResultSheet, SequenceProgression, SheetCellRange, SheetId,
        SheetSection, SubcolumnMode, SummaryPointError, SurvivalStatus, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
        extract_template_hooks,
    };

    #[test]
    fn eight_table_families_defined_and_constructible() {
        assert_eq!(TableKind::ALL.len(), 8);
        assert_ne!(SheetSection::Data, SheetSection::Graphs);

        let archetypes = [
            TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(3), 2, 5).unwrap()),
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).unwrap()),
            TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 3).unwrap()),
            TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap()),
            TableArchetype::Survival(SurvivalTable::new(2, 8).unwrap()),
            TableArchetype::PartsOfWhole(PartsOfWholeTable::new(1, 4).unwrap()),
            TableArchetype::MultipleVariables(MultipleVariablesTable::new(5, 10).unwrap()),
            TableArchetype::Nested(NestedTable::new(2, 3, 4).unwrap()),
        ];

        for (expected, arch) in TableKind::ALL.iter().zip(archetypes.iter()) {
            assert_eq!(arch.kind(), *expected);
            assert!(arch.row_count() >= 2);
            assert!(arch.column_count() >= 1);
        }
    }

    #[test]
    fn dimension_caps_reject_oversized_tables_cleanly() {
        assert!(matches!(XyTable::new(SubcolumnMode::Single, 1, MAX_ROWS + 1), Err(DocError::TooManyRows { .. })));
        assert!(matches!(XyTable::new(SubcolumnMode::RawReplicates(0), 1, 4), Err(DocError::InvalidSubcolumnCount { count: 0 })));
        assert!(matches!(XyTable::new(SubcolumnMode::RawReplicates(MAX_SUBCOLUMNS + 1), 1, 4), Err(DocError::TooManySubcolumns { .. })));
        assert!(matches!(GroupedTable::new(SubcolumnMode::Single, 2, 2), Err(DocError::IncompatibleSubcolumnMode { .. })));
    }

    #[test]
    fn non_destructive_exclusions_asterisk_tokens_and_freeze_state() {
        let starred = Cell::parse_import_token("45.6*", '.', true);
        assert!(starred.excluded);
        assert_eq!(starred.active_number(), None);
        assert_eq!(starred.value.finite_f64(), Some(45.6));

        let comma_dec = Cell::parse_import_token("*12,5", ',', true);
        assert!(comma_dec.excluded);
        assert_eq!(comma_dec.value.finite_f64(), Some(12.5));

        let col_table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
        let mut sheet = DataSheet::new("data-1", "Dose Study", TableArchetype::Column(col_table)).unwrap();
        sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0)).unwrap();
        sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(99.0)).unwrap();
        sheet.set_excluded(CellCoord::new(1, 0, 0), true).unwrap();

        if let TableArchetype::Column(t) = &sheet.archetype {
            assert_eq!(t.active_column_values(0), vec![10.0]);
            // Raw value remains stored intact in cell (1, 0, 0).
            assert_eq!(t.data_sets[0].cell(1, 0).unwrap().value.finite_f64(), Some(99.0));
        }

        sheet.frozen = true;
        assert!(matches!(sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(5.0)), Err(DocError::SheetFrozen { .. })));
        assert!(matches!(sheet.set_excluded(CellCoord::new(0, 0, 0), false), Err(DocError::SheetFrozen { .. })));
    }

    #[test]
    fn pre_summarized_moment_reconstruction_matches_spec() {
        let mut xy = XyTable::new(SubcolumnMode::MeanSemN, 1, 3).unwrap();
        xy.x_mode = XColumnMode::GeneratedSequence { start: 0.0, step: 10.0, progression: SequenceProgression::Arithmetic };
        // Mean = 20, SEM = 2.0, N = 4 -> SD = 2 * sqrt(4) = 4.0, SS_within = (4 - 1) * 16 = 48.0.
        xy.data_sets[0].cells[0] = vec![Cell::number(20.0), Cell::number(2.0), Cell::number(4.0)];
        let pt = xy.summarized_y_point(0, 0).unwrap().unwrap();
        assert!((pt.sd - 4.0).abs() < 1e-12);
        assert!((pt.ss_within - 48.0).abs() < 1e-12);
        assert_eq!(xy.x_value_at(2), Some(20.0));

        // Missing N or N < 1 is flagged rather than silently treated as N = 1.
        xy.data_sets[0].cells[1] = vec![Cell::number(15.0), Cell::number(1.0), Cell::empty()];
        assert_eq!(xy.summarized_y_point(1, 0), Err(SummaryPointError::MissingSampleSize));
        xy.data_sets[0].cells[2] = vec![Cell::number(15.0), Cell::number(1.0), Cell::number(0.0)];
        assert_eq!(xy.summarized_y_point(2, 0), Err(SummaryPointError::SampleSizeBelowOne { n: 0.0 }));
    }

    #[test]
    fn contingency_survival_and_parts_of_whole_domain_validation() {
        let mut cont = ContingencyTable::new(2, 2).unwrap();
        cont.data_sets[0].cells[0][0] = Cell::number(14.0);
        cont.data_sets[0].cells[1][0] = Cell::number(3.5);
        assert_eq!(cont.count_at(0, 0).unwrap(), Some(14));
        assert!(matches!(cont.count_at(1, 0), Err(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. })));

        let mut surv = SurvivalTable::new(1, 3).unwrap();
        surv.times = vec![Cell::number(5.0), Cell::number(12.0), Cell::excluded_number(18.0)];
        surv.cohorts[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(0.0)], vec![Cell::number(1.0)]];
        let obs = surv.cohort_observations(0).unwrap();
        assert_eq!(obs.len(), 2);
        assert_eq!(obs[0].status, SurvivalStatus::Event);
        assert_eq!(obs[1].status, SurvivalStatus::Censored);

        let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
        pow.slice_labels = vec![String::from("Alpha"), String::from("Beta")];
        pow.data_sets[0].cells = vec![vec![Cell::number(25.0)], vec![Cell::number(75.0)]];
        let fracs = pow.fractions_for_series(0).unwrap();
        assert!((fracs[0].fraction - 0.25).abs() < 1e-12);
        assert!((fracs[1].fraction - 0.75).abs() < 1e-12);
    }

    #[test]
    fn multiple_variables_role_inference_recoding_and_level_ordering() {
        let mv = MultipleVariablesTable::new(2, 5).unwrap();
        let mut sheet = DataSheet::new("mv-1", "Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
        for (r, tok) in ["Dose 10", "Dose 2", "Hzel", "Dose 2", "Hazel"].iter().enumerate() {
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::text(*tok)).unwrap();
        }
        sheet.set_excluded(CellCoord::new(4, 0, 0), true).unwrap();

        let TableArchetype::MultipleVariables(mv_ref) = &mut sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        let col = &mut mv_ref.variables[0];
        assert_eq!(col.role, super::VariableRole::Categorical);
        assert!(col.role_inferred);

        // Bulk typo merge "Hzel" -> "Hazel" updates cells and merges frequency counts.
        let edited = col.recode_levels(&[(String::from("Hzel"), String::from("Hazel"))]);
        assert_eq!(edited, 1);

        col.level_order = LevelOrderSpec { order: LevelOrderKind::Alphabetical, reversed: false };
        let alpha = col.ordered_levels();
        let names: Vec<&str> = alpha.iter().map(|s| s.level.as_str()).collect();
        // Natural sort puts "Dose 2" before "Dose 10".
        assert_eq!(names, vec!["Dose 2", "Dose 10", "Hazel"]);
        let hazel = alpha.iter().find(|s| s.level == "Hazel").unwrap();
        assert_eq!(hazel.included_count, 1);
        assert_eq!(hazel.excluded_count, 1);

        let explicit = col.ordered_levels_with(&LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Hazel")]), reversed: true });
        assert_eq!(explicit.last().unwrap().level, "Hazel");

        mv_ref.variables[1].set_calculated_formula("= [Body Mass] / ([Height_m]^2)");
        assert_eq!(mv_ref.variables[1].source, super::VariableSource::Calculated { formula: String::from("[Body Mass] / ([Height_m]^2)") });
    }

    #[test]
    fn info_sheet_constants_and_template_hooks() {
        let mut info = InfoSheet::new_preseeded("info-1", "Project Info 1").unwrap();
        assert_eq!(info.constants.len(), 6);
        info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM"))).unwrap();
        assert_eq!(info.get_constant("hotnm").unwrap().value.as_finite_f64(), Some(2.5));

        let hooks = extract_template_hooks("Ligand {{info:HotNM}}, EC50 = {{result:fit-1:EC50}} ({{result:fit-1:1:HillSlope}})");
        assert_eq!(
            hooks,
            vec![
                HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("HotNM") },
                HookSource::ResultScalar { result_sheet_id: SheetId::new("fit-1"), parameter_key: String::from("EC50"), data_set_index: None },
                HookSource::ResultScalar {
                    result_sheet_id: SheetId::new("fit-1"),
                    parameter_key: String::from("HillSlope"),
                    data_set_index: Some(1),
                },
            ]
        );
    }

    #[test]
    fn analysis_spec_auto_title_retargeting_and_pvalue_formats() {
        assert_eq!(PValueFormat::FourDecimal.format_p(0.000_04), "< 0.0001");
        assert_eq!(PValueFormat::FourDecimal.summary_stars(0.000_04), "****");
        assert_eq!(PValueFormat::Apa.format_p(0.042), ".042");
        assert_eq!(PValueFormat::Apa.summary_stars(0.000_01), "***");
        assert_eq!(PValueFormat::Nejm.format_p(0.042), "0.042");
        assert!((ConfidenceLevel::new(99.9).unwrap().alpha() - 0.001).abs() < 1e-12);

        let mut spec = AnalysisSpec::new("res-1", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
        assert_eq!(spec.effective_title(), "Ordinary one-way ANOVA");
        spec.set_confidence_percent(99.0).unwrap();
        assert_eq!(spec.confidence_level, curvebook_math::ConfidenceLevel::PERCENT_99);
        assert!(matches!(spec.set_confidence_percent(105.0), Err(DocError::InvalidConfidenceLevel { percent }) if percent == 105.0));
        assert_eq!(super::GraphAxisScale::Logit(super::LogitDomain::Proportion), curvebook_math::AxisScale::LOGIT_PROPORTION);
        assert_eq!(super::GraphAxisScale::Logit(super::LogitDomain::Proportion).transform(0.5), Some(0.0));

        // In-place parameter update dynamically re-derives the default sheet title.
        spec.update_kind(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            ..OneWayAnovaSpec::default()
        }));
        assert_eq!(spec.effective_title(), "RM one-way ANOVA");

        // Retargeting onto an incompatible Survival table is rejected cleanly.
        assert!(matches!(spec.retarget_source("surv-1", TableKind::Survival, vec![]), Err(DocError::IncompatibleRetargetTable { .. })));
        // Retargeting onto another Column or MultipleVariables table succeeds.
        spec.retarget_source("mv-1", TableKind::MultipleVariables, vec![0, 1, 2]).unwrap();
        assert_eq!(spec.source_sheet_id.as_str(), "mv-1");
    }

    #[test]
    fn graph_layout_style_cloning_and_project_dag_dependencies() {
        let mut proj = ProjectDocument::new("Pharmacology Study").unwrap();
        let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata").unwrap();
        info.set_constant("HotNM", ConstantValue::Number(1.5), Some(String::from("nM"))).unwrap();
        proj.add_info_sheet(info).unwrap();

        let paired_col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 4).unwrap();
        let mut data_paired = DataSheet::new("data-paired", "Paired Subjects", TableArchetype::Column(paired_col)).unwrap();
        data_paired.info_sheet_id = Some(SheetId::new("info-1"));
        data_paired.floating_notes.push(FloatingNote::new("note-1", RectPt::new(10.0, 10.0, 120.0, 60.0), "Subject 3 hemolyzed"));

        // Paired Column design defaults to before-after lines with expanded row titles.
        let paired_graph = GraphSpec::default_for_data_sheet(&data_paired);
        assert!(matches!(paired_graph.series[0].gdo, GraphicalDataObject::BeforeAfterLines { .. }));
        proj.add_data_sheet(data_paired).unwrap();

        let mut xy_sheet =
            DataSheet::new("data-xy", "Radioligand Curve", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 8).unwrap())).unwrap();
        xy_sheet
            .add_linked_region(LinkedDataRegion {
                id: String::from("lnk-1"),
                spec: ImportLinkSpec {
                    source: LinkSource::InternalSheet {
                        sheet_id: SheetId::new("data-paired"),
                        kind: InternalLinkKind::DataSheet,
                        tab_id: None,
                        range: SheetCellRange::default(),
                    },
                    filter: ImportFilterSpec::default(),
                    placement: ImportPlacementSpec { transpose: true, ..ImportPlacementSpec::default() },
                },
                target_range: SheetCellRange::default(),
                provenance_summary: String::from("Linked transposed from Paired Subjects"),
            })
            .unwrap();
        assert_eq!(xy_sheet.linked_regions[0].border_color(), Some(LinkBorderColor::Blue));
        proj.add_data_sheet(xy_sheet.clone()).unwrap();

        let mut fit_spec = NonlinearFitSpec::new("one_site_fit_ki");
        fit_spec.parameter_rules.push(ParameterRule {
            name: String::from("HotNM"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("HotNM"),
            }),
            initial_override: None,
            lower_bound: Some(0.0),
            upper_bound: None,
        });
        let analysis = AnalysisSpec::new("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit_spec)).unwrap();
        let mut res = ResultSheet::new(analysis).unwrap();
        res.status = super::ResultStatus::UpToDate;
        res.scalars.push(ResultScalarEntry { key: String::from("Ki"), data_set_index: Some(0), value: 4.2e-9, units: Some(String::from("M")) });
        let _ci_cell = ResultCell::ConfidenceInterval { lower: CiBound::Finite(2.1e-9), upper: CiBound::Open };
        assert_eq!(res.lookup_scalar("ki", Some(0)), Some(4.2e-9));
        proj.add_result_sheet(res).unwrap();

        let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
        gsheet.spec.title = String::from("Competition Ki = {{result:fit-1:Ki}}");
        // Discontinuous 4-segment axis is rejected; 2 segments pass.
        gsheet.spec.x_axis.segments = vec![AxisSegment::default(); 4];
        assert!(matches!(gsheet.spec.validate(), Err(DocError::InvalidAxisSegments { count: 4 })));
        gsheet.spec.x_axis.segments = vec![AxisSegment::default(); 2];
        gsheet.spec.validate().unwrap();

        // Style cloning updates palette and dashed error-bar styling without changing source_sheet_id.
        let mut donor = gsheet.spec.clone();
        donor.palette = PaletteId::PaulTolBright;
        if let Some(eb) = &mut donor.series[0].y_error_bars {
            eb.dash_style = LineDashStyle::Dashed;
        }
        gsheet.spec.apply_style_from(&donor);
        assert_eq!(gsheet.spec.palette, PaletteId::PaulTolBright);
        assert_eq!(gsheet.spec.series[0].y_error_bars.unwrap().dash_style, LineDashStyle::Dashed);
        assert_eq!(gsheet.spec.primary_source_sheet_id.as_str(), "data-xy");
        proj.add_graph_sheet(gsheet).unwrap();

        let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn183Mm);
        assert!((lspec.page_size_pt().width - PhysicalUnit::Millimeters.to_points(183.0)).abs() < 1e-9);
        assert_eq!(PanelBadgeStyle::UpperCaseLetters.badge_for(0).as_deref(), Some("A"));
        lspec.items.push(LayoutItem::LiveGraph {
            id: String::from("panel-a"),
            graph_sheet_id: SheetId::new("graph-1"),
            bounds_pt: RectPt::new(18.0, 18.0, 220.0, 160.0),
            scale_percent: 100.0,
            badge_override: None,
        });
        proj.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).unwrap()).unwrap();

        // Verify Family anchor stability vs Navigator re-anchoring.
        proj.select_in_navigator(&SheetId::new("data-xy")).unwrap();
        proj.select_in_family_pane(&SheetId::new("graph-1")).unwrap();
        assert_eq!(proj.active_sheet_id, Some(SheetId::new("graph-1")));
        assert_eq!(proj.family_anchor_sheet_id, Some(SheetId::new("data-xy")));

        // Verify canonical upstream DAG edge extraction.
        assert_eq!(proj.upstream_dependencies(&SheetId::new("data-xy")), vec![SheetId::new("data-paired")]);
        assert_eq!(proj.upstream_dependencies(&SheetId::new("fit-1")), vec![SheetId::new("data-xy"), SheetId::new("info-1")]);
        assert_eq!(proj.upstream_dependencies(&SheetId::new("graph-1")), vec![SheetId::new("data-xy"), SheetId::new("fit-1")]);
        assert_eq!(proj.upstream_dependencies(&SheetId::new("layout-1")), vec![SheetId::new("graph-1")]);
        let _ = Rgba8::WHITE;
    }

    #[test]
    fn reactive_transposition_clone_pipeline_and_explicit_error_bounds() {
        let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
        grouped.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::excluded_number(2.0), Cell::number(3.0)];
        let sheet = DataSheet::new("grp-1", "Two-Way Assay", TableArchetype::Grouped(grouped)).unwrap();

        // Extract 1x3 horizontal replicate row and transpose into 3x1 vertical column preserving exclusions.
        let horizontal = sheet.extract_cell_grid(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 2)));
        assert_eq!(horizontal.len(), 1);
        assert_eq!(horizontal[0].len(), 3);
        let vertical = super::transpose_cell_grid(&horizontal);
        assert_eq!(vertical.len(), 3);
        assert!(vertical[1][0].excluded);
        assert_eq!(vertical[2][0].active_number(), Some(3.0));

        // Clone pipeline overrides replicate count and clears data values cleanly.
        let cloned = sheet
            .clone_structure(
                "grp-2",
                &super::CloneDataSheetOptions {
                    title: String::from("Repeat Plate"),
                    keep_x_values: true,
                    clear_y_values: true,
                    clear_column_titles: false,
                    clear_row_titles: false,
                    override_replicates: Some(5),
                },
            )
            .unwrap();
        let TableArchetype::Grouped(g2) = &cloned.archetype else {
            panic!("expected Grouped");
        };
        assert_eq!(g2.subcolumn_mode, SubcolumnMode::RawReplicates(5));
        assert_eq!(g2.data_sets[0].subcolumn_count(), 5);
        assert!(g2.data_sets[0].cell(0, 0).unwrap().is_empty());

        // Explicit bounds for MeanLowHigh and UpperLowerLimits.
        let mlh = SubcolumnMode::MeanLowHigh.explicit_bounds(&[Cell::number(10.0), Cell::number(2.0), Cell::number(3.0)]).unwrap();
        assert_eq!((mlh.center, mlh.lower, mlh.upper), (10.0, 8.0, 13.0));
        let ull = SubcolumnMode::UpperLowerLimits.explicit_bounds(&[Cell::number(10.0), Cell::number(7.5), Cell::number(14.5)]).unwrap();
        assert_eq!((ull.center, ull.lower, ull.upper), (10.0, 7.5, 14.5));
    }

    #[test]
    fn extract_cell_grid_clamps_unbounded_ranges_and_handles_empty_sheets() {
        let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
        grouped.data_sets[0].cells[0] = vec![Cell::number(1.5), Cell::excluded_number(2.5)];
        grouped.data_sets[1].cells[2] = vec![Cell::number(8.0), Cell::number(9.0)];
        let sheet = DataSheet::new("grp-bounded", "Small Sheet", TableArchetype::Grouped(grouped)).unwrap();

        let unbounded = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(usize::MAX, usize::MAX, usize::MAX));
        let grid = sheet.extract_cell_grid(unbounded);
        assert_eq!(grid.len(), 3);
        assert!(grid.iter().all(|row| row.len() == 4));
        assert_eq!(grid[0][0].active_number(), Some(1.5));
        assert!(grid[0][1].excluded);
        assert_eq!(grid[2][3].active_number(), Some(9.0));

        // Out-of-bounds start coordinates and inverted ranges return empty without panicking.
        assert!(sheet.extract_cell_grid(SheetCellRange::new(CellCoord::new(3, 0, 0), CellCoord::new(usize::MAX, 0, 0))).is_empty());
        assert!(sheet.extract_cell_grid(SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(0, usize::MAX, 0))).is_empty());
        assert!(sheet.extract_cell_grid(SheetCellRange::new(CellCoord::new(2, 0, 0), CellCoord::new(1, 0, 0))).is_empty());
        assert!(sheet.extract_cell_grid(SheetCellRange::new(CellCoord::new(0, 0, 5), CellCoord::new(0, 0, usize::MAX))).is_empty());

        // Zero-row sheet returns empty cleanly on `usize::MAX` bounds.
        let empty_xy = XyTable::new(SubcolumnMode::Single, 2, 0).unwrap();
        let empty_sheet = DataSheet::new("xy-empty", "Zero Rows", TableArchetype::Xy(empty_xy)).unwrap();
        assert!(empty_sheet.extract_cell_grid(unbounded).is_empty());
    }

    #[test]
    fn freeze_guarded_x_and_survival_time_accessors() {
        let xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::Single, 1, 2).unwrap();
        let mut xy_sheet = DataSheet::new("xy-x", "Calibrated X", TableArchetype::Xy(xy)).unwrap();
        xy_sheet.set_x_or_time_cell(0, 0, Cell::number(5.0)).unwrap();
        xy_sheet.set_x_or_time_cell(0, 1, Cell::number(0.5)).unwrap();
        xy_sheet.set_x_or_time_cell(0, 2, Cell::number(4.0)).unwrap();
        assert_eq!(xy_sheet.x_or_time_cell(0, 0).unwrap().active_number(), Some(5.0));
        xy_sheet.set_x_or_time_excluded(0, 0, true).unwrap();
        assert_eq!(xy_sheet.x_or_time_cell(0, 0).unwrap().active_number(), None);
        assert_eq!(xy_sheet.x_or_time_cell(0, 0).unwrap().value.finite_f64(), Some(5.0));
        assert!(matches!(xy_sheet.set_x_or_time_cell(0, 3, Cell::number(1.0)), Err(DocError::CellOutOfBounds { row: 0, col: 0, subcol: 3 })));

        xy_sheet.frozen = true;
        assert!(matches!(xy_sheet.set_x_or_time_cell(0, 0, Cell::number(7.0)), Err(DocError::SheetFrozen { .. })));
        assert!(matches!(xy_sheet.set_x_or_time_excluded(0, 0, false), Err(DocError::SheetFrozen { .. })));

        let surv = SurvivalTable::new(1, 2).unwrap();
        let mut surv_sheet = DataSheet::new("surv-t", "Cohort Survival", TableArchetype::Survival(surv)).unwrap();
        surv_sheet.set_x_or_time_cell(0, 0, Cell::number(14.0)).unwrap();
        surv_sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(1.0)).unwrap();
        assert!(matches!(surv_sheet.set_x_or_time_cell(0, 1, Cell::number(14.0)), Err(DocError::CellOutOfBounds { row: 0, col: 0, subcol: 1 })));
        surv_sheet.set_x_or_time_excluded(0, 0, true).unwrap();
        if let TableArchetype::Survival(t) = &surv_sheet.archetype {
            assert!(t.cohort_observations(0).unwrap().is_empty());
        }
        surv_sheet.frozen = true;
        assert!(matches!(surv_sheet.set_x_or_time_cell(0, 0, Cell::number(21.0)), Err(DocError::SheetFrozen { .. })));
        assert!(matches!(surv_sheet.set_x_or_time_excluded(0, 0, false), Err(DocError::SheetFrozen { .. })));

        let col_sheet =
            DataSheet::new("col-no-x", "Groups", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap()))
                .unwrap();
        assert_eq!(col_sheet.x_or_time_cell(0, 0), None);
    }

    #[test]
    fn column_table_pre_summarized_subcolumns_and_moment_reconstruction() {
        let mut sem_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSemN, 2, 2).unwrap();
        sem_col.data_sets[0].cells[0] = vec![Cell::number(30.0), Cell::number(3.0), Cell::number(9.0)];
        let pt_sem = sem_col.summarized_point(0, 0).unwrap().unwrap();
        assert!((pt_sem.sd - 9.0).abs() < 1e-12);
        assert!((pt_sem.ss_within - 648.0).abs() < 1e-12);

        let mut cv_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanCvN, 1, 2).unwrap();
        cv_col.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(10.0), Cell::number(5.0)];
        let pt_cv = cv_col.summarized_point(0, 0).unwrap().unwrap();
        assert!((pt_cv.sd - 5.0).abs() < 1e-12);
        assert!((pt_cv.ss_within - 100.0).abs() < 1e-12);
        assert_eq!(cv_col.summarized_point(1, 0).unwrap(), None);

        assert!(matches!(
            ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::RawReplicates(3), 2, 4),
            Err(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, mode: SubcolumnMode::RawReplicates(3) })
        ));
    }

    #[test]
    fn multivariate_categorical_summary_floating_bar_and_shape_annotations() {
        let mv = MultipleVariablesTable::new(3, 6).unwrap();
        let mv_sheet = DataSheet::new("mv-cat", "Biomarker Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
        let mut spec = GraphSpec::default_for_data_sheet(&mv_sheet);
        assert_eq!(spec.multivariate.as_ref().unwrap().plot_mode, super::MultivariatePlotMode::BubbleScatter);

        if let Some(enc) = &mut spec.multivariate {
            enc.plot_mode = super::MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::FloatingBar {
                    summary: Some(super::SummaryMeasure::Median),
                    fill_color: Rgba8::rgba(10, 20, 30, 180),
                    border_color: Rgba8::BLACK,
                    border_width_pt: 1.25,
                },
                y_error_bars: Some(super::ErrorBarSpec::default()),
            };
        }
        spec.series[0].gdo = GraphicalDataObject::FloatingBar {
            summary: None,
            fill_color: Rgba8::rgba(1, 2, 3, 200),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
        };
        for kind in [super::AnnotationShapeKind::Rectangle, super::AnnotationShapeKind::RoundedRectangle, super::AnnotationShapeKind::Ellipse] {
            spec.annotations.push(super::GraphAnnotation::Shape {
                kind,
                bounds_pt: RectPt::new(12.0, 16.0, 80.0, 40.0),
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 1.0,
                dash: LineDashStyle::Dotted,
                fill_color: Some(Rgba8::rgba(240, 240, 240, 128)),
            });
        }
        assert_eq!(spec.annotations.len(), 3);

        let mut donor = spec.clone();
        donor.palette = PaletteId::PaulTolVibrant;
        donor.series[0].y_error_bars = Some(super::ErrorBarSpec { dash_style: LineDashStyle::Dashed, ..super::ErrorBarSpec::default() });
        spec.apply_style_from(&donor);

        let expected_primary = PaletteId::PaulTolVibrant.color_at(0);
        assert_eq!(
            spec.series[0].gdo,
            GraphicalDataObject::FloatingBar {
                summary: None,
                fill_color: expected_primary.with_alpha(200),
                border_color: expected_primary,
                border_width_pt: 1.0,
            }
        );
        let Some(super::MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars }) = spec.multivariate.as_ref().map(|m| &m.plot_mode) else {
            panic!("expected CategoricalSummary");
        };
        assert_eq!(
            *gdo,
            GraphicalDataObject::FloatingBar {
                summary: Some(super::SummaryMeasure::Median),
                fill_color: expected_primary.with_alpha(180),
                border_color: expected_primary,
                border_width_pt: 1.25,
            }
        );
        assert_eq!(y_error_bars.unwrap().dash_style, LineDashStyle::Dashed);
    }

    #[test]
    fn project_document_full_serde_json_round_trip() {
        let mut proj = ProjectDocument::new("Serde Round-Trip Study").unwrap();

        let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata").unwrap();
        info.set_constant("HotNM", ConstantValue::Number(1e-300), Some(String::from("M"))).unwrap();
        info.set_constant("MinPos", ConstantValue::Number(f64::MIN_POSITIVE), None).unwrap();
        info.set_constant("NegHi", ConstantValue::Number(-1.234_567_890_123_456_7e12), None).unwrap();
        info.set_constant("Date", ConstantValue::Date(String::from("2026-11-13")), None).unwrap();
        proj.add_info_sheet(info).unwrap();

        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
        xy.x_mode = XColumnMode::GeneratedSequence { start: 1e-300, step: 10.0, progression: SequenceProgression::Geometric };
        xy.data_sets[0].cells[0] = vec![Cell::number(f64::MIN_POSITIVE), Cell::excluded_number(-1.234_567_890_123_456_7e12)];
        let mut xy_sheet = DataSheet::new("data-xy", "XY Sheet", TableArchetype::Xy(xy)).unwrap();
        xy_sheet.info_sheet_id = Some(SheetId::new("info-1"));
        xy_sheet
            .add_linked_region(LinkedDataRegion {
                id: String::from("lnk-1"),
                spec: ImportLinkSpec {
                    source: LinkSource::InternalSheet {
                        sheet_id: SheetId::new("data-col"),
                        kind: InternalLinkKind::DataSheet,
                        tab_id: None,
                        range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
                    },
                    filter: ImportFilterSpec::default(),
                    placement: ImportPlacementSpec::default(),
                },
                target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
                provenance_summary: String::from("Linked column"),
            })
            .unwrap();
        xy_sheet.floating_notes.push(FloatingNote::new("note-1", RectPt::new(4.0, 8.0, 120.0, 50.0), "Check dilution"));
        proj.add_data_sheet(xy_sheet.clone()).unwrap();

        let col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSemN, 2, 2).unwrap();
        proj.add_data_sheet(DataSheet::new("data-col", "Column", TableArchetype::Column(col)).unwrap()).unwrap();

        let sim_spec = AnalysisSpec::new(
            "res-sim",
            "data-xy",
            AnalysisKind::Transform(super::TransformAnalysisSpec::Simulate {
                seed: 42,
                row_count: 16,
                replicates: 3,
                x_min: -9.0,
                x_max: -3.0,
                equation_id: String::from("four_pl"),
                parameters: vec![
                    (String::from("Bottom"), 0.0),
                    (String::from("Top"), 100.0),
                    (String::from("LogIC50"), -6.5),
                    (String::from("HillSlope"), 1.25),
                ],
                noise: super::SimulationNoise::RelativePercent { cv_percent: 5.0 },
            }),
        )
        .unwrap();
        let mut res_sheet = ResultSheet::new(sim_spec).unwrap();
        res_sheet.status = super::ResultStatus::UpToDate;
        res_sheet.analyzable_data = Some(xy_sheet.clone());
        res_sheet.scalars.push(ResultScalarEntry {
            key: String::from("EC50"),
            data_set_index: Some(0),
            value: 3.162_277_660_168_379_5e-7,
            units: Some(String::from("M")),
        });
        proj.add_result_sheet(res_sheet).unwrap();

        let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
        gsheet.spec.x_axis.segments[0].scale = super::GraphAxisScale::Logit(super::LogitDomain::Proportion);
        proj.add_graph_sheet(gsheet).unwrap();

        let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
        lspec.items.push(LayoutItem::LiveGraph {
            id: String::from("panel-a"),
            graph_sheet_id: SheetId::new("graph-1"),
            bounds_pt: RectPt::new(0.0, 0.0, 200.0, 150.0),
            scale_percent: 100.0,
            badge_override: None,
        });
        proj.add_layout_sheet(LayoutSheet::new("layout-1", "Fig 1", lspec).unwrap()).unwrap();

        let json = serde_json::to_string(&proj).unwrap();
        let decoded: ProjectDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, proj);

        // Verify transparent SheetId / NoteId JSON representation and ConfidenceLevel validation.
        assert_eq!(serde_json::to_string(&SheetId::new("data-xy")).unwrap(), "\"data-xy\"");
        assert!(serde_json::from_str::<ConfidenceLevel>("0.0").is_err());
        assert!(serde_json::from_str::<ConfidenceLevel>("100.0").is_err());
    }
}
