//! Exhaustive verification of the canonical summary-mode descriptor matrix (`SUMMARY_MODE_MATRIX`)
//! and widened `TableKind::Column` subcolumn rules.

use std::collections::BTreeSet;

use curvebook_doc::{
    ALL_ANALYSIS_DESCRIPTORS, AnalysisKind, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, DescriptiveSpec, FitMethod,
    GroupedTable, LinearRegressionSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, OutlierHandling, SUMMARY_CAPABLE_TABLE_KINDS,
    SUMMARY_MODE_MATRIX, SUMMARY_SUBCOLUMN_MODES, SubcolumnMode, SummaryModeProcedure, SummaryModeRefusalReason, SummaryModeSemantics,
    SummaryModeSupport, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, WeightingScheme, XyTable,
    summary_mode_matrix_rows, summary_mode_row_for_spec, summary_mode_support,
};

#[test]
fn column_table_allows_all_eight_summary_modes_and_refuses_raw_replicates() {
    assert!(TableKind::Column.allows_subcolumn_mode(SubcolumnMode::Single));
    assert!(!TableKind::Column.allows_subcolumn_mode(SubcolumnMode::RawReplicates(1)));
    assert!(!TableKind::Column.allows_subcolumn_mode(SubcolumnMode::RawReplicates(3)));

    for mode in SUMMARY_SUBCOLUMN_MODES {
        assert!(mode.is_pre_summarized(), "{mode:?} must be pre-summarized");
        for tk in SUMMARY_CAPABLE_TABLE_KINDS {
            assert!(tk.allows_subcolumn_mode(mode), "{tk:?} must allow {mode:?}");
        }
        let col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 2).unwrap();
        assert_eq!(col.subcolumn_mode, mode);
        assert_eq!(col.data_sets[0].subcolumn_count(), mode.subcolumn_count());
        assert!(col.validate().is_ok());
        assert_eq!(TableArchetype::Column(col).primary_subcolumn_mode(), Some(mode));

        let xy = XyTable::new(mode, 1, 2).unwrap();
        assert_eq!(TableArchetype::Xy(xy).primary_subcolumn_mode(), Some(mode));

        let grp = GroupedTable::new(mode, 2, 2).unwrap();
        assert_eq!(TableArchetype::Grouped(grp).primary_subcolumn_mode(), Some(mode));
    }
}

#[test]
fn summary_mode_matrix_is_complete_and_bijective_over_all_procedures_and_modes() {
    assert_eq!(summary_mode_matrix_rows().len(), SummaryModeProcedure::COUNT);
    assert_eq!(SUMMARY_MODE_MATRIX.len(), SummaryModeProcedure::ALL.len());

    let mut seen = BTreeSet::new();
    for (idx, (&proc_id, row)) in SummaryModeProcedure::ALL.iter().zip(SUMMARY_MODE_MATRIX.iter()).enumerate() {
        assert_eq!(proc_id as usize, idx);
        assert_eq!(row.procedure, proc_id);
        assert!(seen.insert(proc_id));
        assert!(!row.label.is_empty());
        assert!(!row.reference_citation.is_empty());
        assert!(row.support_for(SubcolumnMode::Single).is_none());
        assert!(row.support_for(SubcolumnMode::RawReplicates(3)).is_none());
        for mode in SUMMARY_SUBCOLUMN_MODES {
            assert!(row.support_for(mode).is_some(), "missing verdict for {:?} x {mode:?}", proc_id);
        }
    }

    // Every user-creatable analysis compatible with Column, Xy, or Grouped appears in the matrix.
    for desc in ALL_ANALYSIS_DESCRIPTORS {
        if !desc.user_creatable {
            continue;
        }
        for tk in SUMMARY_CAPABLE_TABLE_KINDS {
            if desc.supports_table_kind(tk) {
                let covered = SUMMARY_MODE_MATRIX.iter().any(|r| r.analysis_kind_id == desc.id && r.applies_to_table_kind(tk));
                assert!(covered, "descriptor {:?} on {tk:?} missing from SUMMARY_MODE_MATRIX", desc.id);
            }
        }
    }
}

#[test]
fn summary_mode_support_dispatch_classifies_key_procedures_accurately() {
    // Non-summary modes and incompatible archetypes resolve to None.
    let desc = AnalysisKind::DescriptiveStats(DescriptiveSpec::default());
    assert_eq!(summary_mode_support(&desc, TableKind::Column, SubcolumnMode::Single), None);
    assert_eq!(summary_mode_support(&desc, TableKind::Xy, SubcolumnMode::RawReplicates(3)), None);
    let lin = AnalysisKind::LinearRegression(LinearRegressionSpec::default());
    assert_eq!(summary_mode_support(&lin, TableKind::Column, SubcolumnMode::MeanSdN), None);

    // DescriptiveStats supports all 8 modes with distinct honest semantics.
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        assert_eq!(
            summary_mode_support(&desc, TableKind::Column, mode),
            Some(SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments))
        );
    }
    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem] {
        assert_eq!(
            summary_mode_support(&desc, TableKind::Column, mode),
            Some(SummaryModeSupport::Supported(SummaryModeSemantics::SpreadOnlySummary))
        );
    }
    for mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::UpperLowerLimits] {
        assert_eq!(
            summary_mode_support(&desc, TableKind::Column, mode),
            Some(SummaryModeSupport::Supported(SummaryModeSemantics::IntervalLimitSummary))
        );
    }
    assert_eq!(
        summary_mode_support(&desc, TableKind::Column, SubcolumnMode::MedianIqr),
        Some(SummaryModeSupport::Supported(SummaryModeSemantics::QuartileMedianSummary))
    );

    // Parametric t-tests & one-way ANOVA support MeanSdN/MeanSemN/MeanCvN and refuse the 5 no-N/nonparametric modes.
    let unpaired = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() });
    let mw = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() });
    let anova = AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() });
    let kw = AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..OneWayAnovaSpec::default() });
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        assert!(summary_mode_support(&unpaired, TableKind::Column, mode).unwrap().is_supported());
        assert!(summary_mode_support(&anova, TableKind::Column, mode).unwrap().is_supported());
        assert_eq!(
            summary_mode_support(&mw, TableKind::Column, mode),
            Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates))
        );
        assert_eq!(
            summary_mode_support(&kw, TableKind::Column, mode),
            Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates))
        );
    }
    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem, SubcolumnMode::MeanLowHigh, SubcolumnMode::UpperLowerLimits, SubcolumnMode::MedianIqr]
    {
        assert_eq!(
            summary_mode_support(&unpaired, TableKind::Column, mode),
            Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance))
        );
        assert_eq!(
            summary_mode_support(&anova, TableKind::Column, mode),
            Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance))
        );
    }

    // Nonlinear regression with ObservedSd weighting refuses non-SD interval modes; ROUT/Cauchy refuses all 8.
    let nl_obs_sd =
        AnalysisKind::NonlinearRegression(NonlinearFitSpec { weighting: WeightingScheme::ObservedSd, ..NonlinearFitSpec::new("straight_line") });
    assert_eq!(
        summary_mode_support(&nl_obs_sd, TableKind::Xy, SubcolumnMode::MeanLowHigh),
        Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSdForWeighting))
    );
    let nl_rout = AnalysisKind::NonlinearRegression(NonlinearFitSpec {
        method: FitMethod::LeastSquares,
        outlier_handling: OutlierHandling::Eliminate,
        ..NonlinearFitSpec::new("straight_line")
    });
    assert_eq!(
        summary_mode_support(&nl_rout, TableKind::Xy, SubcolumnMode::MeanSdN),
        Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates))
    );

    // Transpose is refused on pre-summarized Column and supported on XY/Grouped.
    let tx_tr = AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true });
    assert_eq!(
        summary_mode_support(&tx_tr, TableKind::Column, SubcolumnMode::MeanSdN),
        Some(SummaryModeSupport::Refused(SummaryModeRefusalReason::UnsupportedSummaryMode))
    );
    assert!(summary_mode_support(&tx_tr, TableKind::Xy, SubcolumnMode::MedianIqr).unwrap().is_supported());

    // Contingency ExactBinomial on Column maps cleanly; PearsonChiSquare on Column is incompatible (None).
    let binom = AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, ..ContingencySpec::default() });
    let chi2 = AnalysisKind::Contingency(ContingencySpec::default());
    assert_eq!(summary_mode_row_for_spec(&binom, TableKind::Column).map(|r| r.procedure), Some(SummaryModeProcedure::ContingencyColumnBinary));
    assert!(summary_mode_row_for_spec(&chi2, TableKind::Column).is_none());
}
