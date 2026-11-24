//! UI tests for R18.1 nonlinear modal controls, `edit_params` patching, and 16-locale strings.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConvergenceStrictness, NonlinearFitSpec, OutlierHandling, ReplicateFitMode, SheetId, WeightingSource,
};
use curvebook_ui_egui::dialogs_canvas::EditAnalysisParametersState;
use curvebook_ui_egui::i18n::{LOCALES, tr};
use serde_json::json;

#[test]
fn r18_1_edit_params_patches_all_new_nonlinear_fields() {
    let spec = AnalysisSpec::new("res-1", SheetId::new("ds-1"), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();
    let mut state = EditAnalysisParametersState { draft: Some(spec), ..EditAnalysisParametersState::default() };

    let patch = json!({
        "weighting_source": "PredictedY",
        "outlier_handling": "IdentifyAndCountOnly",
        "emit_cleaned_data_tab": true,
        "replicate_fit_mode": "RowMeansOnly",
        "convergence_strictness": "Strict",
        "mv_y_columns": [2, 3]
    });
    state.apply_field_patch(patch.as_object().unwrap()).unwrap();

    assert!(
        matches!(
            &state.draft.as_ref().unwrap().kind,
            AnalysisKind::NonlinearRegression(nl)
                if nl.weighting_source == WeightingSource::PredictedY
                    && nl.outlier_handling == OutlierHandling::IdentifyAndCountOnly
                    && nl.emit_cleaned_data_tab
                    && nl.replicate_fit_mode == ReplicateFitMode::RowMeansOnly
                    && nl.convergence_strictness == ConvergenceStrictness::Strict
                    && nl.mv_y_columns == vec![2, 3]
        ),
        "unexpected NonlinearFitSpec: {:?}",
        state.draft
    );
}

#[test]
fn r18_1_all_new_fit_modal_keys_localized_across_16_locales() {
    let keys = [
        "fit.conv.default",
        "fit.conv.medium",
        "fit.conv.quick",
        "fit.conv.strict",
        "fit.conv.title",
        "fit.outlier.cleaned_tab",
        "fit.outlier.count_only",
        "fit.outlier.eliminate",
        "fit.outlier.none",
        "fit.outlier.title",
        "fit.rep.individual",
        "fit.rep.row_means",
        "fit.rep.title",
        "fit.wt.src.observed",
        "fit.wt.src.predicted",
        "fit.wt.src.title",
    ];
    assert_eq!(LOCALES.len(), 16);
    for locale in LOCALES {
        for key in keys {
            let val = tr(locale, key);
            assert!(!val.is_empty() && val != key, "missing translation for key `{key}` in locale `{locale}`");
        }
    }
}
