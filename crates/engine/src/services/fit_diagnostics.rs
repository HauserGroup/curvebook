//! Diagnostics-only nonlinear curve fit inspection services (`R19.7`).

use std::collections::HashSet;

use curvebook_doc::{AnalysisKind, ErrorCode, HookSource, ProjectDocument, SheetId};
use curvebook_fit::{HookContext, HorizontalLineComparisonReport, compare_nonlinear_analysis_vs_horizontal_line};

use crate::EngineError;
use crate::dag::{resolve_origin_info_sheet_id, resolve_source_table, validate_info_constant_hooks};

/// Compare an existing nonlinear regression [`curvebook_doc::ResultSheet`] against a horizontal
/// line (`Y = Mean`) null model without mutating the document or excluding any datasets (`R19.7`).
///
/// Reuses the exact upstream source table resolution and [`HookContext`] wiring of DAG recalculation
/// and runs extra-SS *F* (or Poisson deviance LRT) plus $\Delta\text{AIC}_c$ per dataset (for
/// independent fits) or globally (for shared-parameter fits).
///
/// # Errors
/// Returns [`EngineError`] if `result_sheet_id` does not exist, does not represent a
/// `NonlinearRegression`, its upstream source table or constant hooks are missing/invalid, or the
/// null-model fit fails.
pub fn compare_result_vs_horizontal_line(doc: &ProjectDocument, result_sheet_id: &SheetId) -> Result<HorizontalLineComparisonReport, EngineError> {
    let Some(res) = doc.result_sheet(result_sheet_id) else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{result_sheet_id}` not found")));
    };
    if !matches!(res.analysis.kind, AnalysisKind::NonlinearRegression(_)) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("horizontal-line comparison requires a NonlinearRegression result sheet, got `{}`", res.title()),
        ));
    }
    let Some(source_id) = res.analysis.source_sheet_id.as_ref() else {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("horizontal-line comparison requires a linked source data sheet for `{}`", res.title()),
        ));
    };
    let source_ds = resolve_source_table(doc, source_id).map_err(|(code, msg)| EngineError::with_code(code, msg))?;
    let primary_info_id = resolve_origin_info_sheet_id(doc, &source_ds, &res.analysis);
    validate_info_constant_hooks(doc, &res.analysis, primary_info_id).map_err(|msg| EngineError::with_code(ErrorCode::BadInput, msg))?;

    let has_explicit_info_hook =
        res.analysis.kind.hooked_sources().iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: Some(_), .. }));
    let hooks = HookContext {
        info_sheets: if primary_info_id.is_some() || has_explicit_info_hook { &doc.info_sheets } else { &[] },
        result_sheets: &doc.result_sheets,
        data_sheets: &doc.data_sheets,
        primary_info_sheet_id: primary_info_id,
        visited_sheets: HashSet::new(),
    };

    compare_nonlinear_analysis_vs_horizontal_line(&res.analysis, &source_ds, &hooks)
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, err.to_string()))
}
