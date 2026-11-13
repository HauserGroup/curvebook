//! Scientific analysis, graph/layout styling, scene compilation, and vector/raster export commands.

use lumencraft_chart::{ChartScene, compile_graph, compile_layout};
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, AxisSegment, AxisSpec, AxisThreshold, BaselineOperation, BracketLabelMode, BracketStyle,
    BreakOrnament, CategoricalTickSource, ColorChannelMapping, ConnectingLineStyle, ContingencyMethod, ContingencySpec, CoxTieMethod,
    DescriptiveSpec, DiagnosticPlotKind, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, ErrorPropagationMode, FactorialAnovaSpec, FitMethod,
    FramePreset, GlmFamily, GlmSpec, GraphAnnotation, GraphAxisScale, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, JournalWidthPreset,
    LayoutItem, LayoutSheet, LayoutSpec, LineConnectionKind, LinearRegressionSpec, MathTransformPreset, MultiplicityMethod, MultiplicitySpec,
    MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec,
    OneWayMethod, OutlierSpec, PageOrientation, PairwiseBracketSpec, PanelBadgeStyle, PartsOfWholeLayout, PhysicalUnit, ProjectDocument, RectPt,
    RegressionOverlaySpec, RepeatedMeasuresDesign, ResultSheet, ResultStatus, RocCiMethod, RocSpec, SeriesSpec, SheetId, SimulationNoise,
    SizeChannelMapping, SumOfSquaresType, SummaryMeasure, SurvivalAnalysisSpec, TableArchetype, TableKind, TickLabelFormat, TransformAnalysisSpec,
    TwoGroupMethod, TwoGroupSpec, VariableRole, WeightingScheme,
};
use lumencraft_fit::get_builtin_model;
use lumencraft_format::{CsvOptions, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
use lumencraft_math::{LogitDomain, PValueFormat};
use lumencraft_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use lumencraft_pdf::{PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_render::{RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::dag;
use crate::{CommandOutcome, EngineError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisDomain {
    Any,
    TransformOnly,
    StatsOnly,
    FitOnly,
}

/// Compile a graph or layout sheet into a device-independent [`ChartScene`].
pub fn compile_sheet_scene(doc: &ProjectDocument, sheet_id: &SheetId) -> Result<ChartScene, EngineError> {
    if let Some(graph) = doc.graph_sheet(sheet_id) {
        return Ok(compile_graph(graph, doc)?);
    }
    if let Some(layout) = doc.layout_sheet(sheet_id) {
        return Ok(compile_layout(layout, doc)?);
    }
    Err(EngineError::new(format!("sheet `{sheet_id}` is not a graph or layout sheet")))
}

/// Try dispatching scientific analysis, graph/layout configuration, or figure export commands.
pub fn dispatch_sci_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    if let Some(outcome) = dispatch_analysis_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    if let Some(outcome) = dispatch_graph_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    if let Some(outcome) = dispatch_layout_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    dispatch_export_command(doc, command_id, params)
}

fn dispatch_analysis_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "analyze.run" => {
            let recalculated = dag::recalculate_project(doc)?;
            Ok(Some(CommandOutcome::mutated(command_id, "ran analysis pipeline", json!({ "recalculated_sheets": recalculated }))))
        }
        "transform.run" => cmd_run_analysis(doc, command_id, params, AnalysisDomain::TransformOnly).map(Some),
        "stats.run" => cmd_run_analysis(doc, command_id, params, AnalysisDomain::StatsOnly).map(Some),
        "fit.run" => cmd_run_analysis(doc, command_id, params, AnalysisDomain::FitOnly).map(Some),
        "analysis.run" => cmd_run_analysis(doc, command_id, params, AnalysisDomain::Any).map(Some),
        "analysis.updateParams" => cmd_analysis_update_params(doc, command_id, params).map(Some),
        "analysis.retargetData" => cmd_analysis_retarget_data(doc, command_id, params).map(Some),
        "analysis.exportJson" => cmd_analysis_export_json(doc, command_id, params).map(Some),
        "analysis.exportCsv" => cmd_analysis_export_csv(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_graph_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "graph.create" => cmd_graph_create(doc, command_id, params).map(Some),
        "graph.initialize" => cmd_graph_initialize(doc, command_id, params).map(Some),
        "graph.setKind" => cmd_graph_set_kind(doc, command_id, params).map(Some),
        "graph.setSpec" => cmd_graph_set_spec(doc, command_id, params).map(Some),
        "graph.setAxes" => cmd_graph_set_axes(doc, command_id, params).map(Some),
        "graph.setSeries" => cmd_graph_set_series(doc, command_id, params).map(Some),
        "graph.setRegressionOverlay" => cmd_graph_set_regression_overlay(doc, command_id, params).map(Some),
        "graph.addComparisonBrackets" => cmd_graph_add_comparison_brackets(doc, command_id, params).map(Some),
        "graph.addAnnotation" => cmd_graph_add_annotation(doc, command_id, params).map(Some),
        "graph.applyStyleFrom" => cmd_graph_apply_style_from(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_layout_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "layout.create" => cmd_layout_create(doc, command_id, params).map(Some),
        "layout.setSpec" => cmd_layout_set_spec(doc, command_id, params).map(Some),
        "layout.addPanel" => cmd_layout_add_panel(doc, command_id, params).map(Some),
        "layout.alignOrigins" => cmd_layout_align_origins(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_export_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "export.svg" | "graph.exportSvg" => cmd_export_svg(doc, command_id, params).map(Some),
        "export.pdf" | "graph.exportPdf" => cmd_export_pdf(doc, command_id, params).map(Some),
        "export.png" | "graph.exportPng" => cmd_export_png(doc, command_id, params).map(Some),
        "export.tiff" => cmd_export_tiff(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Analysis, Transform, Stats & Fit Commands
// ---------------------------------------------------------------------------

fn cmd_run_analysis(doc: &mut ProjectDocument, command_id: &str, params: &Value, domain: AnalysisDomain) -> Result<CommandOutcome, EngineError> {
    let has_creation_fields = has_any_key(params, &["spec", "source_sheet", "source_sheet_id", "data_sheet_id", "kind", "equation_id", "custom_dsl"]);
    if command_id == "analysis.run" && !has_creation_fields {
        return run_existing_or_project_analysis(doc, command_id, params);
    }

    let spec = build_analysis_spec(doc, params, domain)?;
    let sheet_id = spec.id.clone();
    let overwrite = params.get("overwrite").and_then(Value::as_bool).unwrap_or(false);

    if doc.sheet_section(&sheet_id).is_some() {
        if overwrite && let Some(existing) = doc.result_sheet_mut(&sheet_id) {
            existing.analysis = spec;
        } else {
            return Err(EngineError::new(format!("sheet `{sheet_id}` already exists; pass `overwrite: true` to replace an existing result sheet")));
        }
    } else {
        doc.add_result_sheet(ResultSheet::new(spec)?)?;
    }

    let recalculated = dag::recalculate_project(doc)?;
    let res = doc.result_sheet(&sheet_id).ok_or_else(|| EngineError::new(format!("result sheet `{sheet_id}` missing after execution")))?;
    ensure_result_succeeded(res)?;

    Ok(CommandOutcome::mutated(command_id, format!("ran {}", res.analysis.effective_title()), result_summary_payload(res, recalculated)))
}

fn run_existing_or_project_analysis(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    if let Some(raw_id) = opt_str(params, &["sheet_id", "id"]) {
        let id = SheetId::try_new(raw_id)?;
        if doc.result_sheet(&id).is_none() {
            return Err(EngineError::new(format!("result sheet `{id}` not found; supply `source_sheet` and `kind` to create a new analysis")));
        }
        let recalculated = dag::recalculate_project(doc)?;
        let res = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?;
        ensure_result_succeeded(res)?;
        return Ok(CommandOutcome::mutated(command_id, "recalculated result sheet", result_summary_payload(res, recalculated)));
    }
    let recalculated = dag::recalculate_project(doc)?;
    Ok(CommandOutcome::mutated(command_id, "recalculated analyses", json!({ "recalculated_sheets": recalculated })))
}

fn cmd_analysis_update_params(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let mut updated = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?.analysis.clone();
    let source_kind = doc
        .data_sheet(&updated.source_sheet_id)
        .ok_or_else(|| EngineError::new(format!("source data sheet `{}` not found", updated.source_sheet_id)))?
        .kind();

    apply_spec_param_updates(&mut updated, params)?;
    if !updated.kind.is_compatible_with(source_kind) {
        return Err(EngineError::new(format!("updated analysis kind is incompatible with table type `{}`", source_kind.label())));
    }

    if let Some(res) = doc.result_sheet_mut(&id) {
        res.analysis = updated;
    }
    let recalculated = dag::recalculate_project(doc)?;
    let res = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` missing after update")))?;
    ensure_result_succeeded(res)?;

    Ok(CommandOutcome::mutated(command_id, format!("updated analysis parameters on `{id}`"), result_summary_payload(res, recalculated)))
}

fn cmd_analysis_retarget_data(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let new_source = required_sheet_id_any(params, &["source_sheet_id", "source_sheet", "data_sheet_id"])?;
    let new_table_kind = doc.data_sheet(&new_source).ok_or_else(|| EngineError::new(format!("target data sheet `{new_source}` not found")))?.kind();

    let mut updated = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?.analysis.clone();
    let next_selected = if params.get("clear_selected_data_sets").and_then(Value::as_bool) == Some(true) {
        Vec::new()
    } else if let Some(indices) = parse_usize_list(params.get("selected_data_sets"))? {
        indices
    } else {
        updated.selected_data_sets.clone()
    };
    updated.retarget_source(new_source.clone(), new_table_kind, next_selected)?;

    if let Some(res) = doc.result_sheet_mut(&id) {
        res.analysis = updated;
    }
    let recalculated = dag::recalculate_project(doc)?;
    let res = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` missing after retarget")))?;
    ensure_result_succeeded(res)?;

    Ok(CommandOutcome::mutated(command_id, format!("retargeted analysis `{id}` to `{new_source}`"), result_summary_payload(res, recalculated)))
}

fn cmd_analysis_export_json(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let res = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?;
    let pretty = params.get("pretty").and_then(Value::as_bool).unwrap_or(true);
    let json_text = export_result_sheet_json(res, pretty)?;
    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, json_text.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(
        command_id,
        format!("exported analysis `{id}` to JSON"),
        json!({ "sheet_id": id.as_str(), "byte_len": json_text.len(), "path": path, "json": json_text }),
    ))
}

fn cmd_analysis_export_csv(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let res = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?;
    let opts = parse_csv_options(params)?;
    let csv = if let Some(tab_id) = opt_str(params, &["tab_id"]) {
        let tab = res.tabs.iter().find(|t| t.id == tab_id).ok_or_else(|| EngineError::new(format!("result tab `{tab_id}` not found")))?;
        export_result_tab_csv(tab, &opts)
    } else if let Some(tab_index) = params.get("tab_index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        let tab = res.tabs.get(tab_index).ok_or_else(|| EngineError::new(format!("result tab index {tab_index} out of bounds")))?;
        export_result_tab_csv(tab, &opts)
    } else {
        export_result_sheet_tidy_csv(res, &opts)
    };
    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, csv.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(
        command_id,
        format!("exported analysis `{id}` to CSV"),
        json!({ "sheet_id": id.as_str(), "byte_len": csv.len(), "path": path, "csv": csv }),
    ))
}

fn ensure_result_succeeded(res: &ResultSheet) -> Result<(), EngineError> {
    match &res.status {
        ResultStatus::Failed { code, message } => Err(EngineError::new(format!("analysis `{}` failed ({code}): {message}", res.id))),
        ResultStatus::Dirty | ResultStatus::UpToDate => Ok(()),
    }
}

fn result_summary_payload(res: &ResultSheet, recalculated: usize) -> Value {
    let scalars: Map<String, Value> =
        res.scalars.iter().map(|s| (s.key.clone(), json!({ "data_set_index": s.data_set_index, "value": s.value, "units": s.units }))).collect();
    json!({
        "sheet_id": res.id.as_str(),
        "source_sheet_id": res.analysis.source_sheet_id.as_str(),
        "title": res.analysis.effective_title(),
        "status": res.status,
        "recalculated_sheets": recalculated,
        "tabs_count": res.tabs.len(),
        "scalars_count": res.scalars.len(),
        "scalars": scalars,
        "pairwise_comparisons_count": res.pairwise_comparisons.len(),
        "fitted_curves_count": res.fitted_curves.len(),
    })
}

fn build_analysis_spec(doc: &ProjectDocument, params: &Value, domain: AnalysisDomain) -> Result<AnalysisSpec, EngineError> {
    if let Some(spec_val) = params.get("spec") {
        let spec: AnalysisSpec = serde_json::from_value(spec_val.clone()).map_err(|e| EngineError::new(format!("invalid `spec` object: {e}")))?;
        let table_kind = require_data_table_kind(doc, &spec.source_sheet_id)?;
        validate_domain_and_compat(&spec.kind, domain, table_kind)?;
        return Ok(spec);
    }

    if params.get("id").is_some()
        && params.get("source_sheet_id").is_some()
        && params.get("kind").is_some_and(Value::is_object)
        && let Ok(spec) = serde_json::from_value::<AnalysisSpec>(params.clone())
    {
        let table_kind = require_data_table_kind(doc, &spec.source_sheet_id)?;
        validate_domain_and_compat(&spec.kind, domain, table_kind)?;
        return Ok(spec);
    }

    let source_sheet_id = required_sheet_id_any(params, &["source_sheet_id", "source_sheet", "data_sheet_id"])?;
    let table_kind = require_data_table_kind(doc, &source_sheet_id)?;
    let id = match opt_str(params, &["sheet_id", "id"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, domain_prefix(domain))?,
    };

    let kind = parse_analysis_kind(params, domain, table_kind)?;
    validate_domain_and_compat(&kind, domain, table_kind)?;

    let mut spec = AnalysisSpec::new(id, source_sheet_id, kind)?;
    apply_metadata_updates(&mut spec, params)?;
    if matches!(spec.kind, AnalysisKind::Multivariate(_)) && spec.selected_data_sets.is_empty() {
        spec.selected_data_sets = default_multivariate_numeric_columns(doc, &spec.source_sheet_id);
    }
    Ok(spec)
}

fn default_multivariate_numeric_columns(doc: &ProjectDocument, sheet_id: &SheetId) -> Vec<usize> {
    let Some(sheet) = doc.data_sheet(sheet_id) else {
        return Vec::new();
    };
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Vec::new();
    };
    let continuous: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| v.role == VariableRole::Continuous && v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite)))
        .map(|(i, _)| i)
        .collect();
    if continuous.len() >= 2 && continuous.len() < mv.variables.len() {
        return continuous;
    }
    let numeric: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite)))
        .map(|(i, _)| i)
        .collect();
    if numeric.len() >= 2 && numeric.len() < mv.variables.len() {
        return numeric;
    }
    Vec::new()
}

fn require_data_table_kind(doc: &ProjectDocument, sheet_id: &SheetId) -> Result<TableKind, EngineError> {
    doc.data_sheet(sheet_id).map(lumencraft_doc::DataSheet::kind).ok_or_else(|| EngineError::new(format!("source data sheet `{sheet_id}` not found")))
}

fn domain_prefix(domain: AnalysisDomain) -> &'static str {
    match domain {
        AnalysisDomain::TransformOnly => "tr",
        AnalysisDomain::StatsOnly => "stat",
        AnalysisDomain::FitOnly => "fit",
        AnalysisDomain::Any => "res",
    }
}

fn validate_domain_and_compat(kind: &AnalysisKind, domain: AnalysisDomain, table_kind: TableKind) -> Result<(), EngineError> {
    let domain_ok = match domain {
        AnalysisDomain::Any => true,
        AnalysisDomain::TransformOnly => matches!(kind, AnalysisKind::Transform(_)),
        AnalysisDomain::FitOnly => matches!(kind, AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_)),
        AnalysisDomain::StatsOnly => {
            !matches!(kind, AnalysisKind::Transform(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_))
        }
    };
    if !domain_ok {
        return Err(EngineError::new("analysis kind does not match the requested command family"));
    }
    if !kind.is_compatible_with(table_kind) {
        return Err(EngineError::new(format!("analysis `{}` is incompatible with `{}` tables", kind.derive_auto_title(), table_kind.label())));
    }
    Ok(())
}

fn apply_metadata_updates(spec: &mut AnalysisSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(title_val) = params.get("custom_title").or_else(|| params.get("title")) {
        if title_val.is_null() {
            spec.set_custom_title(None)?;
        } else if let Some(s) = title_val.as_str() {
            let custom = if s.trim().is_empty() { None } else { Some(s.to_owned()) };
            spec.set_custom_title(custom)?;
        }
    }
    if let Some(pct) = params.get("confidence_percent").or_else(|| params.get("confidence_level")).and_then(Value::as_f64) {
        spec.set_confidence_percent(pct)?;
    }
    if let Some(indices) =
        parse_usize_list(params.get("selected_data_sets").or_else(|| params.get("selected_columns")).or_else(|| params.get("columns")))?
    {
        spec.selected_data_sets = indices;
    }
    if let Some(pv_val) = params.get("p_value_format") {
        spec.p_value_format = parse_p_value_format(pv_val)?;
    }
    Ok(())
}

fn apply_spec_param_updates(spec: &mut AnalysisSpec, params: &Value) -> Result<(), EngineError> {
    apply_metadata_updates(spec, params)?;

    if let Some(kind_val) = params.get("kind") {
        let mut new_kind = if kind_val.is_object() {
            serde_json::from_value(kind_val.clone()).map_err(|e| EngineError::new(format!("invalid typed `kind` object: {e}")))?
        } else {
            let name = kind_val.as_str().ok_or_else(|| EngineError::new("`kind` must be string or object"))?.trim().to_ascii_lowercase();
            resolve_kind_shorthand(&name, params)?
        };
        patch_analysis_kind_fields(&mut new_kind, params)?;
        apply_shorthand_kind_overrides(&mut new_kind, params)?;
        spec.update_kind(new_kind);
    } else if let Some(patch_obj) = params.get("params").or_else(|| params.get("patch")) {
        let mut kind_copy = spec.kind.clone();
        patch_analysis_kind_fields(&mut kind_copy, patch_obj)?;
        apply_shorthand_kind_overrides(&mut kind_copy, patch_obj)?;
        spec.update_kind(kind_copy);
    } else {
        let mut kind_copy = spec.kind.clone();
        patch_analysis_kind_fields(&mut kind_copy, params)?;
        apply_shorthand_kind_overrides(&mut kind_copy, params)?;
        spec.update_kind(kind_copy);
    }
    Ok(())
}

fn parse_analysis_kind(params: &Value, domain: AnalysisDomain, table_kind: TableKind) -> Result<AnalysisKind, EngineError> {
    if let Some(kind_val) = params.get("kind")
        && kind_val.is_object()
    {
        let mut kind: AnalysisKind = serde_json::from_value(kind_val.clone()).map_err(|e| EngineError::new(format!("invalid `kind` object: {e}")))?;
        apply_shorthand_kind_overrides(&mut kind, params)?;
        return Ok(kind);
    }

    let raw_kind = opt_str(params, &["kind"]).map(|s| s.trim().to_ascii_lowercase());
    let mut kind = match raw_kind.as_deref() {
        None => infer_default_kind(params, domain, table_kind)?,
        Some(name) => resolve_kind_shorthand(name, params)?,
    };
    patch_analysis_kind_fields(&mut kind, params)?;
    apply_shorthand_kind_overrides(&mut kind, params)?;
    Ok(kind)
}

fn infer_default_kind(params: &Value, domain: AnalysisDomain, table_kind: TableKind) -> Result<AnalysisKind, EngineError> {
    if params.get("equation_id").is_some() || params.get("custom_dsl").is_some() {
        return resolve_fit_shorthand("nonlinear", params);
    }
    match domain {
        AnalysisDomain::TransformOnly => resolve_transform_math(params),
        AnalysisDomain::FitOnly => resolve_fit_shorthand("four_parameter_logistic", params),
        AnalysisDomain::StatsOnly | AnalysisDomain::Any => match table_kind {
            TableKind::Xy => resolve_fit_shorthand("four_parameter_logistic", params),
            TableKind::Column => Ok(AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::OrdinaryAnova))),
            TableKind::Grouped => Ok(AnalysisKind::FactorialAnova(default_factorial_anova(false))),
            TableKind::Contingency => Ok(AnalysisKind::Contingency(ContingencySpec::default())),
            TableKind::Survival => Ok(AnalysisKind::Survival(default_km_survival())),
            TableKind::MultipleVariables => Ok(AnalysisKind::Multivariate(default_pca())),
            TableKind::Nested => {
                Ok(AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: false, multiplicity: MultiplicitySpec::default() }))
            }
            TableKind::PartsOfWhole => Ok(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        },
    }
}

fn resolve_kind_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if let Some(kind) = resolve_transform_shorthand(name, params)? {
        return Ok(kind);
    }
    if let Some(kind) = resolve_two_group_or_anova_shorthand(name, params)? {
        return Ok(kind);
    }
    if let Some(kind) = resolve_other_stats_shorthand(name, params)? {
        return Ok(kind);
    }
    resolve_fit_shorthand(name, params)
}

fn resolve_transform_math(params: &Value) -> Result<AnalysisKind, EngineError> {
    let y_transform = Some(parse_math_preset(opt_str(params, &["y_transform", "preset", "operation"]))?);
    let x_transform = opt_str(params, &["x_transform"]).map(|s| parse_math_preset(Some(s))).transpose()?;
    let error_propagation = parse_error_prop(opt_str(params, &["error_propagation", "propagate_error"]))?;
    Ok(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }))
}

fn resolve_transform_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let tr = match name {
        "math" | "transform.math" | "math_transform" => return resolve_transform_math(params).map(Some),
        "normalize" | "transform.normalize" => TransformAnalysisSpec::Normalize {
            zero_anchor: parse_normalize_anchor(opt_str(params, &["zero_anchor", "zero"]).unwrap_or("smallest")),
            hundred_anchor: parse_normalize_anchor(opt_str(params, &["hundred_anchor", "hundred"]).unwrap_or("largest")),
            output_percent: !params.get("as_fraction").and_then(Value::as_bool).unwrap_or(false),
            per_data_set: params.get("per_data_set").and_then(Value::as_bool).unwrap_or(true),
        },
        "remove_baseline" | "baseline" | "transform.remove_baseline" => TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: parse_usize_list(params.get("baseline_columns"))?.unwrap_or_else(|| vec![0]),
            operation: parse_baseline_op(opt_str(params, &["operation"]).unwrap_or("difference")),
        },
        "transpose" | "transform.transpose" => TransformAnalysisSpec::Transpose {
            swap_row_and_column_titles: params.get("swap_row_and_column_titles").and_then(Value::as_bool).unwrap_or(true),
        },
        "prune_rows" | "prune" | "transform.prune_rows" => TransformAnalysisSpec::PruneRows {
            every_k_rows: usize_opt(params, "every_k_rows").or_else(|| usize_opt(params, "step")).unwrap_or(2).max(1),
            average_window: params.get("average_window").or_else(|| params.get("average_dropped")).and_then(Value::as_bool).unwrap_or(false),
        },
        "select_and_transform" | "select" | "transform.select_and_transform" => TransformAnalysisSpec::SelectAndTransform {
            filter_formula: opt_str(params, &["filter_formula"]).map(str::to_owned),
            target_archetype: TableKind::Xy,
        },
        "simulate" | "simulate_xy" | "transform.simulate" => TransformAnalysisSpec::Simulate {
            seed: params.get("seed").and_then(Value::as_u64).unwrap_or(42),
            row_count: usize_opt(params, "row_count").or_else(|| usize_opt(params, "n_points")).unwrap_or(13).max(2),
            replicates: usize_opt(params, "replicates").unwrap_or(3).max(1),
            x_min: params.get("x_min").and_then(Value::as_f64).unwrap_or(-9.0),
            x_max: params.get("x_max").and_then(Value::as_f64).unwrap_or(-3.0),
            equation_id: opt_str(params, &["equation_id"]).unwrap_or("log_agonist_vs_response_4pl").to_owned(),
            parameters: parse_named_params_vec(params.get("params"))?,
            noise: SimulationNoise::None,
        },
        _ => return Ok(None),
    };
    Ok(Some(AnalysisKind::Transform(tr)))
}

fn resolve_two_group_or_anova_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let hyp = params.get("hypothetical").or_else(|| params.get("hypothetical_mean")).and_then(Value::as_f64).unwrap_or(0.0);
    let kind = match name {
        "unpaired_t" | "t_test" | "unpaired_t_test" => two_group_kind(TwoGroupMethod::UnpairedT),
        "welch_t" | "welch_t_test" => two_group_kind(TwoGroupMethod::WelchT),
        "paired_t" | "paired_t_test" => two_group_kind(TwoGroupMethod::PairedT),
        "ratio_paired_t" => two_group_kind(TwoGroupMethod::RatioPairedT),
        "lognormal_t" | "lognormal_unpaired_t" => two_group_kind(TwoGroupMethod::LognormalUnpairedT),
        "mann_whitney" | "mann_whitney_u" => two_group_kind(TwoGroupMethod::MannWhitneyU),
        "wilcoxon" | "wilcoxon_matched_pairs" | "wilcoxon_signed_rank" => two_group_kind(TwoGroupMethod::WilcoxonSignedRank),
        "kolmogorov_smirnov" | "ks" | "ks_two_sample" => two_group_kind(TwoGroupMethod::KolmogorovSmirnov),
        "one_sample_t" => two_group_kind(TwoGroupMethod::OneSampleT { hypothetical: hyp }),
        "one_sample_wilcoxon" => two_group_kind(TwoGroupMethod::OneSampleWilcoxon { hypothetical: hyp }),
        "one_way_anova" | "anova" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::OrdinaryAnova)),
        "welch_anova" | "brown_forsythe" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::BrownForsytheAndWelch)),
        "rm_one_way_anova" | "repeated_measures_anova" => {
            let gg = params.get("geisser_greenhouse").and_then(Value::as_bool).unwrap_or(true);
            AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg }))
        }
        "kruskal_wallis" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::KruskalWallis)),
        "friedman" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::Friedman)),
        "two_way_anova" | "factorial_anova" => AnalysisKind::FactorialAnova(default_factorial_anova(false)),
        "three_way_anova" => AnalysisKind::FactorialAnova(default_factorial_anova(true)),
        _ => return Ok(None),
    };
    Ok(Some(kind))
}

fn resolve_other_stats_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let kind = match name {
        "descriptive" | "descriptive_stats" | "column_stats" => AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        "outliers" | "outlier" | "rout" => {
            AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: params.get("q_percent").and_then(Value::as_f64).unwrap_or(1.0) })
        }
        "grubbs" => AnalysisKind::OutlierDetection(OutlierSpec::Grubbs {
            alpha: params.get("alpha").and_then(Value::as_f64).unwrap_or(0.05),
            iterative: params.get("iterative").and_then(Value::as_bool).unwrap_or(false),
        }),
        "nested_t" | "nested_anova" | "nested" => {
            AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: name == "nested_t", multiplicity: MultiplicitySpec::default() })
        }
        "contingency" | "chi_square" | "fisher_exact" => {
            let method =
                if name == "fisher_exact" { ContingencyMethod::FisherExact } else { ContingencyMethod::PearsonChiSquare { yates_continuity: false } };
            AnalysisKind::Contingency(ContingencySpec { method, ..ContingencySpec::default() })
        }
        "kaplan_meier" | "survival" | "log_rank" => AnalysisKind::Survival(default_km_survival()),
        "cox_ph" | "cox" => AnalysisKind::Survival(default_cox_survival(params)?),
        "roc" => AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        "glm" | "linear_glm" | "multiple_linear" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Linear)),
        "logistic_glm" | "multiple_logistic" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Logistic)),
        "poisson_glm" | "multiple_poisson" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Poisson)),
        "pca" | "principal_components" => AnalysisKind::Multivariate(default_pca()),
        "correlation" | "correlation_matrix" => AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix {
            spearman_rank: params.get("spearman").and_then(Value::as_bool).unwrap_or(false),
        }),
        "clustering" | "kmeans" => {
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(usize_opt(params, "k_clusters").unwrap_or(3).max(2)) })
        }
        _ => return Ok(None),
    };
    Ok(Some(kind))
}

fn resolve_fit_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if matches!(name, "linear" | "simple_linear" | "linear_regression" | "deming" | "deming_regression") {
        let deming_model_ii = matches!(name, "deming" | "deming_regression") || params.get("deming").and_then(Value::as_bool).unwrap_or(false);
        return Ok(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii, ..LinearRegressionSpec::default() }));
    }

    let eq_id = match name {
        "four_parameter_logistic" | "4pl" | "4pl_agonist" => "log_agonist_vs_response_4pl",
        "4pl_inhibitor" | "four_parameter_logistic_inhibitor" | "ic50" => "log_inhibitor_vs_response_4pl",
        "agonist_4pl" => "agonist_vs_response_4pl",
        "inhibitor_4pl" => "inhibitor_vs_response_4pl",
        "3pl" | "three_parameter_logistic" => "log_agonist_vs_response_3pl",
        "3pl_inhibitor" => "log_inhibitor_vs_response_3pl",
        "one_site_binding" | "saturation_binding" => "one_site_specific_binding",
        "nonlinear" | "nonlinear_regression" => opt_str(params, &["equation_id"]).unwrap_or("log_agonist_vs_response_4pl"),
        other => {
            if get_builtin_model(other).is_some() {
                other
            } else if params.get("custom_dsl").and_then(Value::as_str).is_some() {
                "custom_dsl"
            } else {
                return Err(EngineError::new(format!("unknown analysis kind or equation `{other}`")));
            }
        }
    };
    let explicit_eq = opt_str(params, &["equation_id"]).unwrap_or(eq_id);
    let mut fit = NonlinearFitSpec::new(explicit_eq);
    if let Some(dsl) = opt_str(params, &["custom_dsl"]) {
        fit.custom_dsl = Some(dsl.to_owned());
    }
    Ok(AnalysisKind::NonlinearRegression(fit))
}

fn apply_shorthand_kind_overrides(kind: &mut AnalysisKind, params: &Value) -> Result<(), EngineError> {
    match kind {
        AnalysisKind::OneWayAnova(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::FactorialAnova(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::NestedComparison(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::TwoGroupComparison(spec) => {
            if let Some(two_tailed) = params.get("two_tailed").and_then(Value::as_bool) {
                spec.two_tailed = two_tailed;
            } else if let Some(one_tailed) = params.get("one_tailed").and_then(Value::as_bool) {
                spec.two_tailed = !one_tailed;
            }
        }
        AnalysisKind::Survival(spec) => apply_survival_overrides(spec, params)?,
        AnalysisKind::Multivariate(spec) => apply_multivariate_overrides(spec, params)?,
        AnalysisKind::NonlinearRegression(spec) => apply_nonlinear_overrides(spec, params)?,
        AnalysisKind::LinearRegression(spec) => {
            if let Some(d) = params.get("deming").or_else(|| params.get("deming_model_ii")).and_then(Value::as_bool) {
                spec.deming_model_ii = d;
            }
            if let Some(o) = params.get("force_through_origin").and_then(Value::as_bool) {
                spec.force_through_origin = o;
            }
        }
        AnalysisKind::GeneralizedLinearModel(spec) => {
            if let Some(resp) = usize_opt(params, "response_column") {
                spec.response_column = resp;
            }
            if let Some(preds) = parse_usize_list(params.get("predictor_columns"))? {
                spec.predictor_columns = preds;
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_multiplicity_override(mult: &mut MultiplicitySpec, params: &Value) -> Result<(), EngineError> {
    if let Some(name) = opt_str(params, &["multiplicity", "post_hoc"]) {
        *mult = parse_multiplicity_method(name)?;
    }
    if let Some(t) = params.get("alpha").or_else(|| params.get("threshold")).and_then(Value::as_f64) {
        mult.threshold = t;
    }
    Ok(())
}

fn apply_survival_overrides(spec: &mut SurvivalAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. } => apply_multiplicity_override(multiplicity, params),
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. } => {
            if let Some(tc) = usize_opt(params, "time_column") {
                *time_column = tc;
            }
            if let Some(sc) = usize_opt(params, "status_column") {
                *status_column = sc;
            }
            if let Some(preds) = parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))? {
                *predictor_columns = preds;
            }
            Ok(())
        }
    }
}

fn apply_multivariate_overrides(spec: &mut MultivariateSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        MultivariateSpec::Pca { standardize, .. } => {
            if let Some(std_flag) = params.get("standardize").and_then(Value::as_bool) {
                *standardize = std_flag;
            }
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            if let Some(sp) = params.get("spearman").or_else(|| params.get("spearman_rank")).and_then(Value::as_bool) {
                *spearman_rank = sp;
            }
        }
        MultivariateSpec::Clustering { k_means_k } => {
            if let Some(k) = usize_opt(params, "k_clusters").or_else(|| usize_opt(params, "k_means_k")) {
                *k_means_k = Some(k.max(2));
            }
        }
    }
    Ok(())
}

fn apply_nonlinear_overrides(spec: &mut NonlinearFitSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(eq) = opt_str(params, &["equation_id"]) {
        spec.equation_id = eq.to_owned();
    }
    if let Some(w) = opt_str(params, &["weighting"]) {
        spec.weighting = parse_weighting_scheme(w)?;
    }
    if let Some(m) = opt_str(params, &["method", "fit_method"]) {
        spec.method = parse_fit_method(m)?;
    }
    if let Some(cb) = params.get("confidence_bands").and_then(Value::as_bool) {
        spec.confidence_bands = cb;
    }
    if let Some(pb) = params.get("prediction_bands").and_then(Value::as_bool) {
        spec.prediction_bands = pb;
    }
    if let Some(interp) = params.get("interpolate_unknowns").and_then(Value::as_bool) {
        spec.interpolate_unknowns = interp;
    }
    Ok(())
}

fn patch_analysis_kind_fields(kind: &mut AnalysisKind, patch: &Value) -> Result<(), EngineError> {
    let Value::Object(patch_map) = patch else {
        return Ok(());
    };
    let mut current_json = serde_json::to_value(&*kind).map_err(|e| EngineError::new(format!("failed serializing analysis kind: {e}")))?;
    if let Value::Object(ref mut variant_map) = current_json
        && variant_map.len() == 1
        && let Some((_key, inner)) = variant_map.iter_mut().next()
        && let Value::Object(inner_map) = inner
    {
        for (k, v) in patch_map {
            if inner_map.contains_key(k) {
                inner_map.insert(k.clone(), v.clone());
            }
        }
    }
    if let Ok(updated) = serde_json::from_value::<AnalysisKind>(current_json) {
        *kind = updated;
    }
    Ok(())
}

fn two_group_kind(method: TwoGroupMethod) -> AnalysisKind {
    AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..TwoGroupSpec::default() })
}

fn default_one_way_anova(method: OneWayMethod) -> OneWayAnovaSpec {
    OneWayAnovaSpec { method, ..OneWayAnovaSpec::default() }
}

fn default_factorial_anova(three_way: bool) -> FactorialAnovaSpec {
    FactorialAnovaSpec {
        three_way,
        ss_type: SumOfSquaresType::TypeIII,
        repeated_measures: RepeatedMeasuresDesign::None,
        ..FactorialAnovaSpec::default()
    }
}

fn default_km_survival() -> SurvivalAnalysisSpec {
    SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: false,
        trend_test: false,
        asymmetrical_greenwood_ci: true,
        multiplicity: MultiplicitySpec::default(),
    }
}

fn default_cox_survival(params: &Value) -> Result<SurvivalAnalysisSpec, EngineError> {
    let time_column = usize_opt(params, "time_column").unwrap_or(0);
    let status_column = usize_opt(params, "status_column").unwrap_or(1);
    let predictor_columns =
        parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))?.unwrap_or_else(|| vec![2]);
    Ok(SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column,
        status_column,
        predictor_columns,
        reference_levels: Vec::new(),
        tie_method: CoxTieMethod::Efron,
    })
}

fn default_glm(family: GlmFamily) -> GlmSpec {
    GlmSpec { family, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: true }
}

fn default_pca() -> MultivariateSpec {
    MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42) }
}

fn parse_multiplicity_method(raw: &str) -> Result<MultiplicitySpec, EngineError> {
    let method = match raw.trim().to_ascii_lowercase().as_str() {
        "none" => MultiplicityMethod::None,
        "tukey" | "tukey_hsd" => MultiplicityMethod::TukeyHsd,
        "dunnett" => MultiplicityMethod::Dunnett { control_column: 0 },
        "sidak" => MultiplicityMethod::Sidak,
        "bonferroni" => MultiplicityMethod::Bonferroni,
        "holm_sidak" => MultiplicityMethod::HolmSidak,
        "dunnett_t3" => MultiplicityMethod::DunnettT3,
        "games_howell" => MultiplicityMethod::GamesHowell,
        "tamhane_t2" => MultiplicityMethod::TamhaneT2,
        "dunn" | "dunn_rank_sum" => MultiplicityMethod::DunnRankSum,
        "linear_trend" => MultiplicityMethod::LinearTrend,
        "benjamini_hochberg" | "bh" | "fdr" => MultiplicityMethod::BenjaminiHochbergFdr,
        "benjamini_yekutieli" | "by" => MultiplicityMethod::BenjaminiYekutieliFdr,
        "benjamini_krieger_yekutieli" | "bky" => MultiplicityMethod::TwoStageBkyFdr,
        other => return Err(EngineError::new(format!("unknown multiplicity method `{other}`"))),
    };
    Ok(MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true })
}

fn parse_weighting_scheme(raw: &str) -> Result<WeightingScheme, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "unweighted" => Ok(WeightingScheme::None),
        "relative" | "inverse_y" | "1/y" => Ok(WeightingScheme::Relative),
        "relative_square" | "inverse_y2" | "1/y2" | "relative_1_y2" => Ok(WeightingScheme::RelativeSquare),
        "inverse_x" | "1/x" => Ok(WeightingScheme::InverseX),
        "inverse_x2" | "1/x2" => Ok(WeightingScheme::InverseX2),
        other => Err(EngineError::new(format!("unknown weighting scheme `{other}`"))),
    }
}

fn parse_fit_method(raw: &str) -> Result<FitMethod, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ordinary" | "ols" | "least_squares" => Ok(FitMethod::LeastSquares),
        "poisson" | "poisson_deviance" => Ok(FitMethod::PoissonDeviance),
        "robust" | "cauchy" => Ok(FitMethod::RobustCauchy),
        "rout" => Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }),
        other => Err(EngineError::new(format!("unknown fit method `{other}`"))),
    }
}

fn parse_math_preset(raw: Option<&str>) -> Result<MathTransformPreset, EngineError> {
    match raw.unwrap_or("log10").trim().to_ascii_lowercase().as_str() {
        "log10_y" | "log10" => Ok(MathTransformPreset::Log10),
        "ln_y" | "ln" => Ok(MathTransformPreset::Ln),
        "log2_y" | "log2" => Ok(MathTransformPreset::Log2),
        "exp10_y" | "exp10" | "10_pow_y" => Ok(MathTransformPreset::Exp10),
        "exp_y" | "exp" | "e_pow_y" => Ok(MathTransformPreset::Exp),
        "reciprocal_y" | "reciprocal" | "1/y" => Ok(MathTransformPreset::Reciprocal),
        "sqrt" => Ok(MathTransformPreset::Sqrt),
        "square" => Ok(MathTransformPreset::Square),
        "logit" => Ok(MathTransformPreset::Logit),
        "probit" => Ok(MathTransformPreset::Probit),
        "arcsin_sqrt" => Ok(MathTransformPreset::ArcsinSqrt),
        other => Err(EngineError::new(format!("unknown math transform preset `{other}`"))),
    }
}

fn parse_error_prop(raw: Option<&str>) -> Result<ErrorPropagationMode, EngineError> {
    match raw.unwrap_or("asymmetric").trim().to_ascii_lowercase().as_str() {
        "asymmetric" | "replicate_endpoints" => Ok(ErrorPropagationMode::ReplicateEndpoints),
        "delta" | "delta_method" | "first_order_delta" => Ok(ErrorPropagationMode::FirstOrderDeltaMethod),
        other => Err(EngineError::new(format!("unknown error propagation mode `{other}`"))),
    }
}

fn parse_normalize_anchor(raw: &str) -> NormalizeAnchor {
    match raw.trim().to_ascii_lowercase().as_str() {
        "first" | "first_row" => NormalizeAnchor::FirstRow,
        "last" | "last_row" => NormalizeAnchor::LastRow,
        "sum" | "column_sum" => NormalizeAnchor::ColumnSum,
        "largest" | "max" | "column_max" => NormalizeAnchor::ColumnMax,
        _ => NormalizeAnchor::ColumnMin,
    }
}

fn parse_baseline_op(raw: &str) -> BaselineOperation {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ratio" | "divide" => BaselineOperation::Ratio,
        "percent" | "percent_change" => BaselineOperation::PercentChange,
        "fraction" | "fractional_change" => BaselineOperation::FractionalChange,
        _ => BaselineOperation::Difference,
    }
}

fn parse_p_value_format(val: &Value) -> Result<PValueFormat, EngineError> {
    if let Ok(pv) = serde_json::from_value::<PValueFormat>(val.clone()) {
        return Ok(pv);
    }
    match val.as_str().map(str::to_ascii_lowercase).as_deref() {
        Some("four_decimal" | "numeric" | "digits" | "decimal") => Ok(PValueFormat::FourDecimal),
        Some("apa") => Ok(PValueFormat::Apa),
        Some("nejm") => Ok(PValueFormat::Nejm),
        Some("scientific") => Ok(PValueFormat::new_custom(4, Some(1.0))),
        _ => Err(EngineError::new("invalid `p_value_format`")),
    }
}

fn parse_csv_options(params: &Value) -> Result<CsvOptions, EngineError> {
    let mut opts = CsvOptions::csv();
    if let Some(delim) = opt_str(params, &["delimiter"]) {
        opts.delimiter = single_char(delim, "delimiter")?;
    }
    if let Some(dec) = opt_str(params, &["decimal_separator"]) {
        opts.decimal_separator = single_char(dec, "decimal_separator")?;
    }
    Ok(opts)
}

fn single_char(raw: &str, name: &str) -> Result<char, EngineError> {
    let mut chars = raw.chars();
    let ch = chars.next().ok_or_else(|| EngineError::new(format!("`{name}` cannot be empty")))?;
    if chars.next().is_some() {
        return Err(EngineError::new(format!("`{name}` must be a single character")));
    }
    Ok(ch)
}

// ---------------------------------------------------------------------------
// Graph Commands
// ---------------------------------------------------------------------------

fn cmd_graph_create(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let data_id = required_sheet_id_any(params, &["data_sheet_id", "source_sheet_id", "source_sheet"])?;
    let ds = doc.data_sheet(&data_id).ok_or_else(|| EngineError::new(format!("data sheet `{data_id}` not found")))?.clone();

    let id = match opt_str(params, &["sheet_id", "id"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, "graph")?,
    };

    let mut graph = GraphSheet::new_uninitialized_for_data(id.clone(), &ds)?;
    if let Some(title) = opt_str(params, &["title"]) {
        graph.title = title.to_owned();
    }
    if let Some(kind_val) = params.get("kind") {
        let kind = parse_graph_kind(kind_val)?;
        apply_graph_kind(&mut graph.spec, kind, true);
        graph.initialized = params.get("initialized").and_then(Value::as_bool).unwrap_or(true);
    } else if let Some(init) = params.get("initialized").and_then(Value::as_bool) {
        graph.initialized = init;
    }
    graph.spec.validate()?;

    let title = graph.title.clone();
    let initialized = graph.initialized;
    let kind = graph.spec.kind;
    doc.add_graph_sheet(graph)?;

    Ok(CommandOutcome::mutated(
        command_id,
        format!("created graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "data_sheet_id": data_id.as_str(), "title": title, "initialized": initialized, "kind": kind }),
    ))
}

fn cmd_graph_initialize(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let kind_opt = params.get("kind").map(parse_graph_kind).transpose()?;
    let title_opt = opt_str(params, &["title"]).map(str::to_owned);

    let graph = doc.graph_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?;
    graph.initialized = true;
    if let Some(title) = title_opt {
        graph.title = title;
    }
    if let Some(kind) = kind_opt {
        apply_graph_kind(&mut graph.spec, kind, true);
    }
    graph.spec.validate()?;

    Ok(CommandOutcome::mutated(
        command_id,
        format!("initialized graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "kind": graph.spec.kind, "initialized": true }),
    ))
}

fn cmd_graph_set_kind(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let kind_val = params.get("kind").ok_or_else(|| EngineError::new("missing `kind` parameter"))?;
    let kind = parse_graph_kind(kind_val)?;
    let reset_series = params.get("reset_series_style").and_then(Value::as_bool).unwrap_or(true);

    let graph = doc.graph_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?;
    graph.initialized = true;
    apply_graph_kind(&mut graph.spec, kind, reset_series);
    graph.spec.validate()?;

    Ok(CommandOutcome::mutated(command_id, format!("set graph `{id}` kind"), json!({ "sheet_id": id.as_str(), "kind": kind })))
}

fn cmd_graph_set_spec(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let current_spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();

    let patch = params.get("spec").cloned().unwrap_or_else(|| strip_keys(params, &["sheet_id", "id", "title", "initialized"]));
    let patch_clean = strip_keys(&patch, &["multivariate"]);
    let merged = merge_json_struct(&current_spec, &patch_clean)?;
    let mut new_spec: GraphSpec = serde_json::from_value(merged).map_err(|e| EngineError::new(format!("invalid graph spec on `{id}`: {e}")))?;

    if let Some(mv_val) = patch.get("multivariate").or_else(|| params.get("multivariate")) {
        new_spec.multivariate = Some(parse_multivariate_encoding(mv_val, current_spec.multivariate.as_ref())?);
    }
    if let Some(nar) = params.get("survival_number_at_risk").and_then(Value::as_bool) {
        new_spec.survival_number_at_risk = nar;
    }
    new_spec.validate()?;

    let title_opt = opt_str(params, &["title"]).map(str::to_owned);
    let init_opt = params.get("initialized").and_then(Value::as_bool);
    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = new_spec;
        if let Some(title) = title_opt {
            graph.title = title;
        }
        graph.initialized = init_opt.unwrap_or(true);
    }

    let kind = doc.graph_sheet(&id).map(|g| g.spec.kind);
    let series_count = doc.graph_sheet(&id).map_or(0, |g| g.spec.series.len());
    Ok(CommandOutcome::mutated(
        command_id,
        format!("updated graph spec on `{id}`"),
        json!({ "sheet_id": id.as_str(), "kind": kind, "series_count": series_count }),
    ))
}

fn cmd_graph_set_axes(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let mut spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();

    if let Some(fp) = params.get("frame_preset") {
        spec.frame_preset = parse_frame_preset(fp)?;
    }
    if let Some(x_val) = params.get("x_axis") {
        apply_axis_updates(&mut spec.x_axis, x_val)?;
    }
    if let Some(y_val) = params.get("y_axis") {
        apply_axis_updates(&mut spec.y_axis, y_val)?;
    }
    if let Some(y2_val) = params.get("y2_axis") {
        if y2_val.is_null() {
            spec.y2_axis = None;
        } else {
            let mut y2 = spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Right Y"));
            apply_axis_updates(&mut y2, y2_val)?;
            spec.y2_axis = Some(y2);
        }
    }

    if let Some(which) = opt_str(params, &["axis", "which"]) {
        match which.trim().to_ascii_lowercase().as_str() {
            "x" | "x_axis" => apply_axis_updates(&mut spec.x_axis, params)?,
            "y" | "y_axis" => apply_axis_updates(&mut spec.y_axis, params)?,
            "y2" | "y2_axis" | "right_y" => {
                let mut y2 = spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Right Y"));
                apply_axis_updates(&mut y2, params)?;
                spec.y2_axis = Some(y2);
            }
            other => return Err(EngineError::new(format!("unknown axis target `{other}`; expected `x`, `y`, or `y2`"))),
        }
    }

    spec.validate()?;
    let x_segs = spec.x_axis.segments.len();
    let y_segs = spec.y_axis.segments.len();
    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = spec;
        graph.initialized = true;
    }

    Ok(CommandOutcome::mutated(
        command_id,
        format!("updated axes on graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "x_segments": x_segs, "y_segments": y_segs }),
    ))
}

fn cmd_graph_set_series(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let mut spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();

    if let Some(series_list) = params.get("series").and_then(Value::as_array) {
        let mut updated_vec = Vec::with_capacity(series_list.len());
        for (idx, item) in series_list.iter().enumerate() {
            let mut base = spec.series.get(idx).cloned().unwrap_or_else(|| default_series_like(&spec, idx));
            apply_single_series_patch(&mut base, item)?;
            updated_vec.push(base);
        }
        spec.series = updated_vec;
    } else if has_series_patch_keys(params) {
        let target_idx = resolve_series_target_index(&spec, params)?;
        if let Some(s) = spec.series.get_mut(target_idx) {
            apply_single_series_patch(s, params)?;
        }
    }

    if let Some(mv_val) = params.get("multivariate") {
        spec.multivariate = Some(parse_multivariate_encoding(mv_val, spec.multivariate.as_ref())?);
    }

    spec.validate()?;
    let count = spec.series.len();
    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = spec;
        graph.initialized = true;
    }

    Ok(CommandOutcome::mutated(command_id, format!("updated series styling on `{id}`"), json!({ "sheet_id": id.as_str(), "series_count": count })))
}

fn cmd_graph_set_regression_overlay(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let clear = params.get("clear").and_then(Value::as_bool).unwrap_or(false);

    let overlay = if clear {
        None
    } else {
        let res_id = required_sheet_id_any(params, &["result_sheet_id", "result_sheet"])?;
        if doc.result_sheet(&res_id).is_none() {
            return Err(EngineError::new(format!("result sheet `{res_id}` not found")));
        }
        let sample_segments =
            params.get("sample_segments").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok()).unwrap_or(256).clamp(256, 1024);
        let show_confidence_band = params.get("show_confidence_band").and_then(Value::as_bool).unwrap_or(false);
        let show_prediction_band = params.get("show_prediction_band").and_then(Value::as_bool).unwrap_or(false);
        let shaded_ribbon = params.get("shaded_ribbon").and_then(Value::as_bool).unwrap_or(false);
        Some(RegressionOverlaySpec { result_sheet_id: res_id, sample_segments, show_confidence_band, show_prediction_band, shaded_ribbon })
    };

    let mut spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();
    if spec.series.is_empty() {
        return Err(EngineError::new(format!("graph `{id}` has no series to bind a regression overlay")));
    }

    let all_series = params.get("all_series").and_then(Value::as_bool).unwrap_or(false);
    if all_series {
        for s in &mut spec.series {
            s.regression_overlay = overlay.clone();
        }
    } else {
        let idx = resolve_series_target_index(&spec, params)?;
        if let Some(s) = spec.series.get_mut(idx) {
            s.regression_overlay = overlay;
        }
    }

    spec.validate()?;
    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = spec;
        graph.initialized = true;
    }

    Ok(CommandOutcome::mutated(command_id, format!("updated regression overlay on `{id}`"), json!({ "sheet_id": id.as_str(), "cleared": clear })))
}

fn cmd_graph_add_comparison_brackets(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let res_id = required_sheet_id_any(params, &["result_sheet_id", "result_sheet"])?;
    let comparisons =
        doc.result_sheet(&res_id).ok_or_else(|| EngineError::new(format!("result sheet `{res_id}` not found")))?.pairwise_comparisons.clone();

    let mut spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();
    if params.get("clear_existing").and_then(Value::as_bool).unwrap_or(false) {
        spec.comparisons.clear();
    }

    let style = parse_bracket_style(opt_str(params, &["style"]))?;
    let label_mode = parse_bracket_label_mode(params.get("label_mode"))?;
    let hide_if_non_significant = params.get("hide_if_non_significant").and_then(Value::as_bool).unwrap_or(false);
    let stroke_width_pt = params.get("stroke_width_pt").and_then(Value::as_f64).unwrap_or(0.9);
    let font_size_pt = params.get("font_size_pt").and_then(Value::as_f64).unwrap_or(8.5);
    let bracket_step_pt = params.get("bracket_step_pt").and_then(Value::as_f64).unwrap_or(12.0);

    let added = if params.get("group_a").is_some() && params.get("group_b").is_some() {
        let ga = resolve_group_index(params.get("group_a"), &comparisons)?;
        let gb = resolve_group_index(params.get("group_b"), &comparisons)?;
        spec.comparisons.push(PairwiseBracketSpec {
            result_sheet_id: res_id,
            group_a: ga,
            group_b: gb,
            style,
            label_mode,
            hide_if_non_significant,
            stroke_width_pt,
            font_size_pt,
            bracket_step_pt,
        });
        1_usize
    } else {
        let significant_only = params.get("significant_only").and_then(Value::as_bool).unwrap_or(false);
        let mut count = 0_usize;
        for cmp in comparisons {
            if significant_only && !cmp.significant {
                continue;
            }
            spec.comparisons.push(PairwiseBracketSpec {
                result_sheet_id: res_id.clone(),
                group_a: cmp.group_a,
                group_b: cmp.group_b,
                style,
                label_mode: label_mode.clone(),
                hide_if_non_significant,
                stroke_width_pt,
                font_size_pt,
                bracket_step_pt,
            });
            count += 1;
        }
        count
    };

    spec.validate()?;
    let total = spec.comparisons.len();
    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = spec;
        graph.initialized = true;
    }

    Ok(CommandOutcome::mutated(
        command_id,
        format!("added {added} comparison bracket(s) to `{id}`"),
        json!({ "sheet_id": id.as_str(), "added": added, "total_brackets": total }),
    ))
}

fn resolve_group_index(val: Option<&Value>, comparisons: &[lumencraft_doc::PairwiseComparisonRecord]) -> Result<usize, EngineError> {
    let v = val.ok_or_else(|| EngineError::new("missing group index or label"))?;
    if let Some(raw) = v.as_u64() {
        return usize::try_from(raw).map_err(|_| EngineError::new("group index out of bounds"));
    }
    if let Some(label) = v.as_str() {
        for cmp in comparisons {
            if cmp.label_a == label {
                return Ok(cmp.group_a);
            }
            if cmp.label_b == label {
                return Ok(cmp.group_b);
            }
        }
        return Err(EngineError::new(format!("unknown comparison group label `{label}`")));
    }
    Err(EngineError::new("group specifier must be integer index or group label"))
}

fn cmd_graph_add_annotation(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let annotation = parse_graph_annotation(doc, params)?;

    let mut spec = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.spec.clone();
    spec.annotations.push(annotation);
    spec.validate()?;
    let total = spec.annotations.len();

    if let Some(graph) = doc.graph_sheet_mut(&id) {
        graph.spec = spec;
        graph.initialized = true;
    }

    Ok(CommandOutcome::mutated(command_id, format!("added annotation to `{id}`"), json!({ "sheet_id": id.as_str(), "total_annotations": total })))
}

fn cmd_graph_apply_style_from(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let target_id = required_sheet_id_any(params, &["sheet_id", "target_sheet_id", "id"])?;
    let donor_id = required_sheet_id_any(params, &["donor_sheet_id", "from_sheet_id", "source_graph_id"])?;
    let donor_spec = doc.graph_sheet(&donor_id).ok_or_else(|| EngineError::new(format!("donor graph `{donor_id}` not found")))?.spec.clone();

    let target = doc.graph_sheet_mut(&target_id).ok_or_else(|| EngineError::new(format!("target graph `{target_id}` not found")))?;
    let mut updated = target.spec.clone();
    updated.apply_style_from(&donor_spec);
    updated.validate()?;
    target.spec = updated;
    target.initialized = true;

    Ok(CommandOutcome::mutated(
        command_id,
        format!("applied style from `{donor_id}` to `{target_id}`"),
        json!({ "sheet_id": target_id.as_str(), "donor_sheet_id": donor_id.as_str(), "primary_source_sheet_id": target.spec.primary_source_sheet_id.as_str() }),
    ))
}

fn okabe_swatch(idx: usize) -> Rgba8 {
    let swatches = PaletteId::OkabeIto.swatches();
    swatches[idx % swatches.len()]
}

fn apply_graph_kind(spec: &mut GraphSpec, kind: GraphKind, reset_series: bool) {
    spec.kind = kind;
    if !reset_series {
        return;
    }
    let swatch = okabe_swatch(0);
    for (idx, s) in spec.series.iter_mut().enumerate() {
        let color = okabe_swatch(idx);
        s.gdo = default_gdo_for_kind(kind, if idx == 0 { swatch } else { color });
    }
}

fn default_gdo_for_kind(kind: GraphKind, color: Rgba8) -> GraphicalDataObject {
    match kind {
        GraphKind::Xy | GraphKind::MultipleVariables | GraphKind::Diagnostic(_) => GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle { fill_color: color, stroke_color: color, ..MarkerStyle::default() }),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
        },
        GraphKind::Column | GraphKind::Grouped | GraphKind::Contingency => GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: color,
            border_color: Rgba8::BLACK,
            border_width_pt: 0.75,
            replicate_overlay: None,
        },
        GraphKind::Survival => GraphicalDataObject::KaplanMeierStep {
            line_color: color,
            stroke_width_pt: 1.5,
            show_censored_ticks: true,
            show_greenwood_ci: false,
            ci_dashed: true,
        },
        GraphKind::PartsOfWhole => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
    }
}

fn apply_axis_updates(axis: &mut AxisSpec, val: &Value) -> Result<(), EngineError> {
    if let Some(title) = opt_str(val, &["title"]) {
        axis.title = title.to_owned();
    }
    if let Some(gap) = val.get("gap_width_pt").and_then(Value::as_f64) {
        axis.gap_width_pt = gap;
    }
    if let Some(orn_val) = val.get("break_ornament") {
        axis.break_ornament = parse_break_ornament(orn_val)?;
    }
    if let Some(sw) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        axis.stroke_width_pt = sw;
    }
    if let Some(tick_angle) = val.get("tick_angle_deg").and_then(Value::as_f64) {
        axis.tick_angle_deg = tick_angle;
    }
    if let Some(tf_val) = val.get("tick_format") {
        axis.tick_format = parse_tick_format(tf_val)?;
    }
    if let Some(segs_val) = val.get("segments").and_then(Value::as_array) {
        if !(1..=3).contains(&segs_val.len()) {
            return Err(EngineError::new("axis `segments` must contain 1 to 3 segments"));
        }
        let template = axis.segments.first().cloned().unwrap_or_default();
        let mut parsed = Vec::with_capacity(segs_val.len());
        for s in segs_val {
            let mut seg = template.clone();
            apply_segment_updates(&mut seg, s)?;
            parsed.push(seg);
        }
        axis.segments = parsed;
    } else if let Some(first_seg) = axis.segments.first_mut() {
        apply_segment_updates(first_seg, val)?;
    }
    if let Some(thresh_val) = val.get("thresholds").and_then(Value::as_array) {
        let mut out = Vec::with_capacity(thresh_val.len());
        for item in thresh_val {
            out.push(parse_axis_threshold(item)?);
        }
        axis.thresholds = out;
    }
    axis.validate()?;
    Ok(())
}

fn apply_segment_updates(seg: &mut AxisSegment, val: &Value) -> Result<(), EngineError> {
    if let Some(scale_val) = val.get("scale") {
        seg.scale = parse_axis_scale(scale_val)?;
    }
    if let Some(min_v) = val.get("min") {
        seg.min = if min_v.is_null() { None } else { Some(min_v.as_f64().ok_or_else(|| EngineError::new("`min` must be numeric or null"))?) };
    }
    if let Some(max_v) = val.get("max") {
        seg.max = if max_v.is_null() { None } else { Some(max_v.as_f64().ok_or_else(|| EngineError::new("`max` must be numeric or null"))?) };
    }
    if let Some(share) = val.get("length_share").and_then(Value::as_f64) {
        seg.length_share = share;
    }
    if let Some(major) = val.get("major_tick_interval") {
        seg.major_tick_interval = if major.is_null() {
            None
        } else {
            Some(major.as_f64().ok_or_else(|| EngineError::new("`major_tick_interval` must be numeric or null"))?)
        };
    }
    if let Some(minor) = val.get("minor_ticks_per_interval").and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()) {
        seg.minor_ticks_per_interval = minor;
    }
    Ok(())
}

fn apply_single_series_patch(series: &mut SeriesSpec, val: &Value) -> Result<(), EngineError> {
    if let Some(label) = opt_str(val, &["legend_label", "label"]) {
        series.legend_label = Some(label.to_owned());
    }
    if let Some(y2) = val.get("bind_to_right_y2").and_then(Value::as_bool) {
        series.bind_to_right_y2 = y2;
    }
    if let Some(gdo_val) = val.get("gdo").or_else(|| val.get("graphical_object")) {
        series.gdo = serde_json::from_value(gdo_val.clone()).map_err(|e| EngineError::new(format!("invalid `gdo`: {e}")))?;
    }
    if let Some(color_val) = val.get("color").or_else(|| val.get("fill_color")) {
        let rgba = parse_rgba8(color_val)?;
        recolor_gdo(&mut series.gdo, rgba);
    }
    if let Some(m_val) = val.get("marker").or_else(|| val.get("markers")) {
        apply_marker_to_gdo(&mut series.gdo, m_val)?;
    }
    if let Some(l_val) = val.get("connecting_line").or_else(|| val.get("line")) {
        apply_line_to_gdo(&mut series.gdo, l_val)?;
    }
    if let Some(eb_val) = val.get("y_error_bars").or_else(|| val.get("error_bars")) {
        series.y_error_bars = patch_error_bars(series.y_error_bars, eb_val)?;
    }
    if let Some(xeb_val) = val.get("x_error_bars") {
        series.x_error_bars = patch_error_bars(series.x_error_bars, xeb_val)?;
    }
    Ok(())
}

fn recolor_gdo(gdo: &mut GraphicalDataObject, color: Rgba8) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
            if let Some(l) = line {
                l.color = color;
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => *fill_color = color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = color;
            marker.stroke_color = color;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = color;
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color = color,
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn apply_marker_to_gdo(gdo: &mut GraphicalDataObject, val: &Value) -> Result<(), EngineError> {
    if val.is_null() {
        if let GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } = gdo {
            *marker = None;
        }
        return Ok(());
    }
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } => {
            let mut style = marker.unwrap_or_default();
            apply_marker_patch(&mut style, val)?;
            *marker = Some(style);
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => apply_marker_patch(marker, val)?,
        _ => {}
    }
    Ok(())
}

fn apply_line_to_gdo(gdo: &mut GraphicalDataObject, val: &Value) -> Result<(), EngineError> {
    if val.is_null() {
        if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
            *line = None;
        }
        return Ok(());
    }
    if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
        if opt_str(val, &["kind"]).is_some_and(|k| k.eq_ignore_ascii_case("none")) {
            *line = None;
            return Ok(());
        }
        let mut style = line.unwrap_or(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.5,
            color: Rgba8::BLACK,
        });
        if let Some(k) = val.get("kind") {
            style.kind = parse_line_connection_kind(k)?;
        }
        if let Some(d) = val.get("dash_style").or_else(|| val.get("dash")) {
            style.dash_style = parse_line_dash_style(d)?;
        }
        if let Some(w) = val.get("stroke_width_pt").or_else(|| val.get("width_pt")).and_then(Value::as_f64) {
            style.stroke_width_pt = w;
        }
        if let Some(c) = val.get("color") {
            style.color = parse_rgba8(c)?;
        }
        *line = Some(style);
    } else if let GraphicalDataObject::KaplanMeierStep { line_color, stroke_width_pt, .. } = gdo {
        if let Some(c) = val.get("color") {
            *line_color = parse_rgba8(c)?;
        }
        if let Some(w) = val.get("stroke_width_pt").and_then(Value::as_f64) {
            *stroke_width_pt = w;
        }
    }
    Ok(())
}

fn apply_marker_patch(marker: &mut MarkerStyle, val: &Value) -> Result<(), EngineError> {
    if let Some(shape) = val.get("shape") {
        marker.shape = parse_marker_shape(shape)?;
    }
    if let Some(fill_mode) = val.get("fill_mode") {
        marker.fill_mode = parse_marker_fill_mode(fill_mode)?;
    }
    if let Some(size) = val.get("size_pt").and_then(Value::as_f64) {
        marker.size_pt = size;
    }
    if let Some(fc) = val.get("fill_color").or_else(|| val.get("color")) {
        marker.fill_color = parse_rgba8(fc)?;
    }
    if let Some(sc) = val.get("stroke_color") {
        marker.stroke_color = parse_rgba8(sc)?;
    }
    if let Some(sw) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        marker.stroke_width_pt = sw;
    }
    Ok(())
}

fn patch_error_bars(current: Option<ErrorBarSpec>, val: &Value) -> Result<Option<ErrorBarSpec>, EngineError> {
    if val.is_null() || opt_str(val, &["metric"]).is_some_and(|m| m.eq_ignore_ascii_case("none")) {
        return Ok(None);
    }
    let mut eb = current.unwrap_or_default();
    if let Some(m) = val.get("metric") {
        eb.metric = parse_error_bar_metric(m)?;
    }
    if let Some(d) = val.get("direction") {
        eb.direction = parse_error_bar_direction(d)?;
    }
    if let Some(dash) = val.get("dash_style") {
        eb.dash_style = parse_line_dash_style(dash)?;
    }
    if let Some(w) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        eb.stroke_width_pt = w;
    }
    if let Some(cw) = val.get("cap_width_pt").and_then(Value::as_f64) {
        eb.cap_width_pt = cw;
    }
    if let Some(c) = val.get("color") {
        eb.color = if c.is_null() { None } else { Some(parse_rgba8(c)?) };
    }
    Ok(Some(eb))
}

fn default_series_like(spec: &GraphSpec, data_set_index: usize) -> SeriesSpec {
    SeriesSpec {
        source_sheet_id: spec.primary_source_sheet_id.clone(),
        data_set_index,
        legend_label: None,
        bind_to_right_y2: false,
        gdo: default_gdo_for_kind(spec.kind, okabe_swatch(data_set_index)),
        y_error_bars: None,
        x_error_bars: None,
        regression_overlay: None,
    }
}

fn resolve_series_target_index(spec: &GraphSpec, params: &Value) -> Result<usize, EngineError> {
    if let Some(idx) = usize_opt(params, "series_index") {
        if idx >= spec.series.len() {
            return Err(EngineError::new(format!("`series_index` {idx} out of bounds ({} series)", spec.series.len())));
        }
        return Ok(idx);
    }
    if let Some(ds_idx) = usize_opt(params, "data_set_index") {
        return spec
            .series
            .iter()
            .position(|s| s.data_set_index == ds_idx)
            .ok_or_else(|| EngineError::new(format!("no series bound to `data_set_index` {ds_idx}")));
    }
    if spec.series.is_empty() {
        return Err(EngineError::new("graph has no series configured"));
    }
    Ok(0)
}

fn has_series_patch_keys(params: &Value) -> bool {
    has_any_key(
        params,
        &[
            "series_index",
            "data_set_index",
            "legend_label",
            "label",
            "bind_to_right_y2",
            "color",
            "fill_color",
            "marker",
            "markers",
            "connecting_line",
            "line",
            "error_bars",
            "y_error_bars",
            "x_error_bars",
            "gdo",
            "graphical_object",
        ],
    )
}

fn parse_multivariate_encoding(val: &Value, current: Option<&MultivariateGraphEncoding>) -> Result<MultivariateGraphEncoding, EngineError> {
    if let Ok(full) = serde_json::from_value::<MultivariateGraphEncoding>(val.clone()) {
        return Ok(full);
    }
    let mut base = current.cloned().unwrap_or(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(okabe_swatch(0)),
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    if let Some(xc) = usize_opt(val, "x_column") {
        base.x_column = xc;
    }
    if let Some(yc) = usize_opt(val, "y_column") {
        base.y_column = yc;
    }
    if let Some(lc) = usize_opt(val, "label_column") {
        base.label_column = Some(lc);
    }
    if let Some(hull) = val.get("show_convex_hulls").and_then(Value::as_bool) {
        base.show_convex_hulls = hull;
    }
    if let Some(ell) = val.get("show_confidence_ellipses").and_then(Value::as_bool) {
        base.show_confidence_ellipses = ell;
    }
    if let Some(color_v) = val.get("color") {
        base.color = parse_color_channel(color_v)?;
    }
    if let Some(size_v) = val.get("size") {
        base.size = parse_size_channel(size_v)?;
    }
    Ok(base)
}

fn parse_color_channel(val: &Value) -> Result<ColorChannelMapping, EngineError> {
    if let Ok(typed) = serde_json::from_value::<ColorChannelMapping>(val.clone()) {
        return Ok(typed);
    }
    if let Some(col) = val.as_u64().and_then(|v| usize::try_from(v).ok()) {
        return Ok(ColorChannelMapping::Continuous {
            column: col,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    if let Some(col) = usize_opt(val, "column") {
        if val.get("categorical").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(ColorChannelMapping::Categorical { column: col, palette: PaletteId::OkabeIto, level_order_override: None });
        }
        return Ok(ColorChannelMapping::Continuous {
            column: col,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    Ok(ColorChannelMapping::Constant(parse_rgba8(val)?))
}

fn parse_size_channel(val: &Value) -> Result<SizeChannelMapping, EngineError> {
    if let Ok(typed) = serde_json::from_value::<SizeChannelMapping>(val.clone()) {
        return Ok(typed);
    }
    if let Some(col) = usize_opt(val, "column") {
        let min_r = val.get("min_radius_pt").and_then(Value::as_f64).unwrap_or(3.0);
        let max_r = val.get("max_radius_pt").and_then(Value::as_f64).unwrap_or(14.0);
        return Ok(SizeChannelMapping::Variable { column: col, min_radius_pt: min_r, max_radius_pt: max_r });
    }
    if let Some(r) = val.get("radius_pt").and_then(Value::as_f64).or_else(|| val.as_f64()) {
        return Ok(SizeChannelMapping::Constant { radius_pt: r });
    }
    Err(EngineError::new("invalid multivariate `size` mapping"))
}

fn parse_graph_annotation(doc: &ProjectDocument, params: &Value) -> Result<GraphAnnotation, EngineError> {
    if let Some(ann_val) = params.get("annotation") {
        let ann: GraphAnnotation = serde_json::from_value(ann_val.clone()).map_err(|e| EngineError::new(format!("invalid `annotation`: {e}")))?;
        if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } = &ann
            && doc.result_sheet(result_sheet_id).is_none()
        {
            return Err(EngineError::new(format!("result sheet `{result_sheet_id}` not found")));
        }
        return Ok(ann);
    }

    let kind = opt_str(params, &["kind", "type"]).unwrap_or("text").trim().to_ascii_lowercase();
    match kind.as_str() {
        "text" | "text_box" | "textbox" => {
            let markdown = opt_str(params, &["markdown", "text"]).ok_or_else(|| EngineError::new("missing `text` or `markdown`"))?.to_owned();
            let bounds_pt = parse_rect_pt(
                params.get("bounds_pt"),
                RectPt::new(
                    params.get("x_pt").and_then(Value::as_f64).unwrap_or(100.0),
                    params.get("y_pt").and_then(Value::as_f64).unwrap_or(80.0),
                    params.get("width_pt").and_then(Value::as_f64).unwrap_or(120.0),
                    params.get("height_pt").and_then(Value::as_f64).unwrap_or(28.0),
                ),
            )?;
            let bordered = params.get("bordered").or_else(|| params.get("boxed")).and_then(Value::as_bool).unwrap_or(false);
            Ok(GraphAnnotation::TextBox { bounds_pt, markdown, bordered })
        }
        "line" | "arrow" => {
            let start_pt = parse_pair_f64(params.get("start_pt"), (40.0, 40.0))?;
            let end_pt = parse_pair_f64(params.get("end_pt"), (140.0, 40.0))?;
            let arrowhead = params.get("arrowhead").and_then(Value::as_bool).unwrap_or(kind == "arrow");
            let dash =
                params.get("dash").or_else(|| params.get("dash_style")).map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            let color = params.get("color").map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            Ok(GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color })
        }
        "shape" | "rect" | "ellipse" => {
            let shape_kind =
                match opt_str(params, &["shape"]).unwrap_or(if kind == "ellipse" { "ellipse" } else { "rectangle" }).to_ascii_lowercase().as_str() {
                    "ellipse" | "circle" => AnnotationShapeKind::Ellipse,
                    "rounded_rectangle" | "rounded" => AnnotationShapeKind::RoundedRectangle,
                    _ => AnnotationShapeKind::Rectangle,
                };
            let bounds_pt = parse_rect_pt(params.get("bounds_pt").or_else(|| params.get("rect_pt")), RectPt::new(60.0, 60.0, 80.0, 40.0))?;
            let stroke_color = params.get("stroke_color").or_else(|| params.get("color")).map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            let fill_color = params.get("fill_color").filter(|v| !v.is_null()).map(parse_rgba8).transpose()?;
            let stroke_width_pt = params.get("stroke_width_pt").and_then(Value::as_f64).unwrap_or(1.0);
            let dash = params.get("dash").map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            Ok(GraphAnnotation::Shape { kind: shape_kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color })
        }
        "table" | "embedded_result_table" | "result_table" => {
            let res_id = required_sheet_id_any(params, &["result_sheet_id", "result_sheet"])?;
            if doc.result_sheet(&res_id).is_none() {
                return Err(EngineError::new(format!("result sheet `{res_id}` not found")));
            }
            let bounds_pt = parse_rect_pt(params.get("bounds_pt"), RectPt::new(180.0, 40.0, 180.0, 90.0))?;
            let tab_id = opt_str(params, &["tab_id"]).map(str::to_owned);
            Ok(GraphAnnotation::EmbeddedResultTable { result_sheet_id: res_id, tab_id, bounds_pt })
        }
        other => Err(EngineError::new(format!("unknown annotation kind `{other}`"))),
    }
}

// ---------------------------------------------------------------------------
// Layout Commands
// ---------------------------------------------------------------------------

fn cmd_layout_create(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = match opt_str(params, &["sheet_id", "id"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, "layout")?,
    };
    let title = opt_str(params, &["title"]).unwrap_or("Layout 1").to_owned();
    let mut spec =
        if let Some(preset_val) = params.get("preset") { LayoutSpec::from_preset(parse_journal_preset(preset_val)?) } else { LayoutSpec::default() };
    apply_layout_spec_fields(&mut spec, params)?;

    let layout = LayoutSheet::new(id.clone(), title.clone(), spec)?;
    let page_pt = layout.spec.page_size_pt();
    doc.add_layout_sheet(layout)?;

    Ok(CommandOutcome::mutated(
        command_id,
        format!("created layout `{id}`"),
        json!({ "sheet_id": id.as_str(), "title": title, "width_pt": page_pt.width, "height_pt": page_pt.height }),
    ))
}

fn cmd_layout_set_spec(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let current = doc.layout_sheet(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?;
    let mut spec = current.spec.clone();

    if let Some(spec_val) = params.get("spec") {
        let merged = merge_json_struct(&spec, spec_val)?;
        spec = serde_json::from_value(merged).map_err(|e| EngineError::new(format!("invalid layout `spec`: {e}")))?;
    } else {
        apply_layout_spec_fields(&mut spec, params)?;
    }

    let title_opt = opt_str(params, &["title"]).map(str::to_owned);
    let page_pt = spec.page_size_pt();
    let items_len = spec.items.len();
    if let Some(layout) = doc.layout_sheet_mut(&id) {
        layout.spec = spec;
        if let Some(title) = title_opt {
            layout.title = title;
        }
    }

    Ok(CommandOutcome::mutated(
        command_id,
        format!("updated layout spec on `{id}`"),
        json!({ "sheet_id": id.as_str(), "items_count": items_len, "width_pt": page_pt.width, "height_pt": page_pt.height }),
    ))
}

fn cmd_layout_add_panel(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let layout = doc.layout_sheet(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?;
    let next_idx = layout.spec.items.len();
    let bounds_pt = resolve_panel_bounds(&layout.spec, next_idx, params)?;
    let panel_id = opt_str(params, &["panel_id"]).map(str::to_owned).unwrap_or_else(|| format!("panel-{}", next_idx + 1));

    let panel_kind = opt_str(params, &["kind", "type"]).unwrap_or_else(|| {
        if params.get("graph_sheet_id").is_some() {
            "graph"
        } else if params.get("source_sheet_id").is_some() {
            "table"
        } else {
            "text"
        }
    });

    let item = match panel_kind.trim().to_ascii_lowercase().as_str() {
        "graph" | "live_graph" => {
            let graph_id = required_sheet_id_any(params, &["graph_sheet_id", "graph_sheet"])?;
            if doc.graph_sheet(&graph_id).is_none() {
                return Err(EngineError::new(format!("graph sheet `{graph_id}` not found")));
            }
            LayoutItem::LiveGraph {
                id: panel_id,
                graph_sheet_id: graph_id,
                bounds_pt,
                scale_percent: params.get("scale_percent").and_then(Value::as_f64).unwrap_or(100.0),
                badge_override: opt_str(params, &["badge_override", "badge"]).map(str::to_owned),
            }
        }
        "table" | "live_table" => {
            let source_id = required_sheet_id_any(params, &["source_sheet_id", "source_sheet"])?;
            if doc.sheet_section(&source_id).is_none() {
                return Err(EngineError::new(format!("table source sheet `{source_id}` not found")));
            }
            LayoutItem::LiveTable { id: panel_id, source_sheet_id: source_id, tab_id: opt_str(params, &["tab_id"]).map(str::to_owned), bounds_pt }
        }
        "text" | "callout" | "text_callout" => LayoutItem::TextCallout {
            id: panel_id,
            markdown: opt_str(params, &["markdown", "text"]).ok_or_else(|| EngineError::new("missing `markdown` or `text`"))?.to_owned(),
            bounds_pt,
            font_size_pt: params.get("font_size_pt").and_then(Value::as_f64).unwrap_or(10.0),
        },
        other => return Err(EngineError::new(format!("unknown layout panel kind `{other}`"))),
    };

    let layout_mut = doc.layout_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?;
    layout_mut.spec.items.push(item);
    let panel_count = layout_mut.spec.items.len();

    Ok(CommandOutcome::mutated(
        command_id,
        format!("added panel to layout `{id}`"),
        json!({ "sheet_id": id.as_str(), "panel_index": panel_count - 1, "panel_count": panel_count, "bounds_pt": bounds_pt }),
    ))
}

fn cmd_layout_align_origins(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let align = params.get("align").or_else(|| params.get("align_plot_origins")).and_then(Value::as_bool).unwrap_or(true);
    let normalize_bounds = params.get("normalize_bounds").and_then(Value::as_bool).unwrap_or(true);

    let layout = doc.layout_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?;
    layout.spec.align_plot_origins = align;
    if align && normalize_bounds {
        align_live_graph_bounds(&mut layout.spec.items);
    }

    let mut aligned_rects = Vec::new();
    for (idx, item) in layout.spec.items.iter().enumerate() {
        if let LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, .. } = item {
            aligned_rects.push(json!({
                "panel_index": idx,
                "graph_sheet_id": graph_sheet_id.as_str(),
                "x": bounds_pt.x,
                "y": bounds_pt.y,
                "width": bounds_pt.width,
                "height": bounds_pt.height,
            }));
        }
    }

    Ok(CommandOutcome::mutated(
        command_id,
        format!("aligned plot origins on layout `{id}`"),
        json!({ "sheet_id": id.as_str(), "align_plot_origins": align, "aligned_panels": aligned_rects.len(), "panels": aligned_rects }),
    ))
}

fn align_live_graph_bounds(items: &mut [LayoutItem]) {
    let mut rects: Vec<(usize, RectPt)> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, it)| match it {
            LayoutItem::LiveGraph { bounds_pt, .. } => Some((idx, *bounds_pt)),
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } => None,
        })
        .collect();
    if rects.len() < 2 {
        return;
    }
    align_axis_bands(&mut rects, true);
    align_axis_bands(&mut rects, false);
    for (item_idx, new_rect) in rects {
        if let Some(LayoutItem::LiveGraph { bounds_pt, .. }) = items.get_mut(item_idx) {
            *bounds_pt = new_rect;
        }
    }
}

fn align_axis_bands(rects: &mut [(usize, RectPt)], vertical_col: bool) {
    let mut done = vec![false; rects.len()];
    for i in 0..rects.len() {
        if done[i] {
            continue;
        }
        let ref_r = rects[i].1;
        let span = if vertical_col { ref_r.width } else { ref_r.height }.max(40.0) * 0.35;
        let ref_coord = if vertical_col { ref_r.x } else { ref_r.y };
        let mut target = ref_coord;
        let mut group = vec![i];
        for (j, (_, rj)) in rects.iter().enumerate().skip(i + 1) {
            let coord_j = if vertical_col { rj.x } else { rj.y };
            if !done[j] && (coord_j - ref_coord).abs() <= span {
                target = target.max(coord_j);
                group.push(j);
            }
        }
        for idx in group {
            done[idx] = true;
            if vertical_col {
                rects[idx].1.x = target;
            } else {
                rects[idx].1.y = target;
            }
        }
    }
}

fn apply_layout_spec_fields(spec: &mut LayoutSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(preset_val) = params.get("preset") {
        let fresh = LayoutSpec::from_preset(parse_journal_preset(preset_val)?);
        spec.width = fresh.width;
        spec.height = fresh.height;
        spec.unit = fresh.unit;
    }
    if let Some(unit_val) = params.get("unit") {
        spec.unit = parse_physical_unit(unit_val)?;
    }
    if let Some(w) = params.get("width").and_then(Value::as_f64) {
        spec.width = w;
    }
    if let Some(h) = params.get("height").and_then(Value::as_f64) {
        spec.height = h;
    }
    if let Some(ori) = params.get("orientation") {
        spec.orientation = parse_page_orientation(ori)?;
    }
    if let Some(grid_val) = params.get("grid_template") {
        if grid_val.is_null() {
            spec.grid_template = None;
        } else if let Some(arr) = grid_val.as_array()
            && arr.len() == 2
        {
            let rows = arr[0].as_u64().and_then(|v| u8::try_from(v).ok()).ok_or_else(|| EngineError::new("`grid_template[0]` must be u8"))?;
            let cols = arr[1].as_u64().and_then(|v| u8::try_from(v).ok()).ok_or_else(|| EngineError::new("`grid_template[1]` must be u8"))?;
            spec.grid_template = Some((rows.max(1), cols.max(1)));
        }
    }
    if let Some(align) = params.get("align_plot_origins").and_then(Value::as_bool) {
        spec.align_plot_origins = align;
    }
    if let Some(badges) = params.get("panel_badges") {
        spec.panel_badges = parse_panel_badge_style(badges)?;
    }
    if let Some(bg) = params.get("background_fill") {
        spec.background_fill = if bg.is_null() { None } else { Some(parse_rgba8(bg)?) };
    }
    Ok(())
}

fn resolve_panel_bounds(spec: &LayoutSpec, panel_index: usize, params: &Value) -> Result<RectPt, EngineError> {
    if let Some(b) = params.get("bounds_pt") {
        return parse_rect_pt(Some(b), RectPt::new(18.0, 18.0, 200.0, 160.0));
    }
    let page = spec.page_size_pt();
    let margin = params.get("margin_pt").and_then(Value::as_f64).unwrap_or(18.0);
    let gutter = params.get("gutter_pt").and_then(Value::as_f64).unwrap_or(12.0);
    let (rows_u8, cols_u8) = spec.grid_template.unwrap_or((2, 2));
    let rows = usize::from(rows_u8.max(1));
    let cols = usize::from(cols_u8.max(1));
    let col = usize_opt(params, "col").unwrap_or(panel_index % cols);
    let row = usize_opt(params, "row").unwrap_or(panel_index / cols);

    let usable_w = (page.width - 2.0 * margin - gutter * ((cols - 1) as f64)).max(40.0);
    let usable_h = (page.height - 2.0 * margin - gutter * ((rows - 1) as f64)).max(40.0);
    let cell_w = usable_w / (cols as f64);
    let cell_h = usable_h / (rows as f64);
    Ok(RectPt::new(margin + (col as f64) * (cell_w + gutter), margin + (row as f64) * (cell_h + gutter), cell_w, cell_h))
}

// ---------------------------------------------------------------------------
// Export Commands
// ---------------------------------------------------------------------------

fn cmd_export_svg(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let scene = compile_sheet_scene(doc, &id)?;
    let mut opts = SvgExportOptions::default();
    if let Some(p) = params.get("pretty").and_then(Value::as_bool) {
        opts.pretty = p;
    }
    if let Some(xml) = params.get("include_xml_declaration").and_then(Value::as_bool) {
        opts.include_xml_declaration = xml;
    }
    if let Some(meta) = params.get("include_metadata").and_then(Value::as_bool) {
        opts.include_metadata = meta;
    }
    if let Some(emb) = params.get("embed_fonts").and_then(Value::as_bool) {
        opts.embed_fonts = emb;
    }
    if let Some(prec) = params.get("precision").and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()) {
        opts.precision = prec;
    }
    if let Some(ot) = params.get("outline_text").and_then(Value::as_bool) {
        opts.outline_text = ot;
    }

    let svg = render_svg(&scene, &opts)?;
    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, svg.as_bytes())?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported SVG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": svg.len(),
            "width_pt": scene.width_pt,
            "height_pt": scene.height_pt,
            "warnings_count": scene.warnings.len(),
            "path": path,
            "svg": svg,
        }),
    ))
}

fn cmd_export_pdf(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let mut opts = PdfExportOptions::default();
    if let Some(cs) = opt_str(params, &["color_space"]) {
        opts.color_space = match cs.trim().to_ascii_lowercase().as_str() {
            "cmyk" => PdfColorSpace::Cmyk,
            _ => PdfColorSpace::Srgb,
        };
    }
    if let Some(comp) = params.get("compress_streams").and_then(Value::as_bool) {
        opts.compress_streams = comp;
    }
    if let Some(title) = opt_str(params, &["title"]) {
        opts.title = Some(title.to_owned());
    }
    if let Some(outline) = params.get("outline_text").or_else(|| params.get("vector_text")).and_then(Value::as_bool) {
        opts.outline_text = outline;
    }

    let (pdf, sheet_label, width_pt, height_pt, page_count) = if let Some(ids_val) = params.get("sheet_ids").and_then(Value::as_array) {
        let mut scenes = Vec::with_capacity(ids_val.len());
        for v in ids_val {
            let raw = v.as_str().ok_or_else(|| EngineError::new("`sheet_ids` entries must be strings"))?;
            let sid = SheetId::try_new(raw)?;
            scenes.push(compile_sheet_scene(doc, &sid)?);
        }
        let w = scenes.first().map_or(0.0, |s| s.width_pt);
        let h = scenes.first().map_or(0.0, |s| s.height_pt);
        let pages = scenes.len();
        let bytes = render_pdf_pages(&scenes, &opts)?;
        (bytes, String::from("multi-page"), w, h, pages)
    } else {
        let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
        let scene = compile_sheet_scene(doc, &id)?;
        let bytes = render_pdf(&scene, &opts)?;
        (bytes, id.as_str().to_owned(), scene.width_pt, scene.height_pt, 1_usize)
    };

    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, &pdf)?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported PDF figure",
        json!({
            "sheet_id": sheet_label,
            "byte_len": pdf.len(),
            "page_count": page_count,
            "width_pt": width_pt,
            "height_pt": height_pt,
            "color_space": opts.color_space,
            "path": path,
            "base64": encode_base64(&pdf),
        }),
    ))
}

fn cmd_export_png(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let scene = compile_sheet_scene(doc, &id)?;
    let target = resolve_raster_target(&scene, params);
    let img = render_scene(&scene, target)?;
    let png = encode_png(&img)?;

    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, &png)?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported PNG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": png.len(),
            "width_px": img.width_px,
            "height_px": img.height_px,
            "dpi": target.dpi,
            "path": path,
            "base64": encode_base64(&png),
        }),
    ))
}

fn cmd_export_tiff(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = required_sheet_id_any(params, &["sheet_id", "id"])?;
    let scene = compile_sheet_scene(doc, &id)?;
    let target = resolve_raster_target(&scene, params);
    let compression = match opt_str(params, &["compression"]).unwrap_or("deflate").trim().to_ascii_lowercase().as_str() {
        "none" => TiffCompression::None,
        "lzw" => TiffCompression::Lzw,
        "deflate" => TiffCompression::Deflate,
        other => return Err(EngineError::new(format!("unknown TIFF compression `{other}`"))),
    };

    let img = render_scene(&scene, target)?;
    let tiff = encode_tiff(&img, compression)?;
    let path = opt_str(params, &["path"]).map(str::to_owned);
    if let Some(ref p) = path {
        write_output_file(p, &tiff)?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported TIFF figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": tiff.len(),
            "width_px": img.width_px,
            "height_px": img.height_px,
            "dpi": target.dpi,
            "compression": opt_str(params, &["compression"]).unwrap_or("deflate"),
            "path": path,
            "base64": encode_base64(&tiff),
        }),
    ))
}

fn resolve_raster_target(scene: &ChartScene, params: &Value) -> RenderTargetSpec {
    let dpi = params.get("dpi").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok()).unwrap_or(300).max(1);
    let default_w = ((scene.width_pt * f64::from(dpi) / 72.0).round().clamp(16.0, 8192.0)) as u32;
    let default_h = ((scene.height_pt * f64::from(dpi) / 72.0).round().clamp(16.0, 8192.0)) as u32;
    let width_px = params.get("width_px").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(default_w);
    let height_px = params.get("height_px").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(default_h);
    RenderTargetSpec { width_px, height_px, dpi }
}

fn write_output_file(path: &str, bytes: &[u8]) -> Result<(), EngineError> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::new(format!("failed creating parent directory `{}`: {e}", parent.display())))?;
    }
    std::fs::write(target, bytes).map_err(|e| EngineError::new(format!("failed writing `{path}`: {e}")))?;
    Ok(())
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((triple >> 18) & 0x3F) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3F) as usize]));
        out.push(if chunk.len() > 1 { char::from(TABLE[((triple >> 6) & 0x3F) as usize]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[(triple & 0x3F) as usize]) } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// JSON & Enum Parsing Helpers
// ---------------------------------------------------------------------------

fn required_sheet_id_any(params: &Value, keys: &[&str]) -> Result<SheetId, EngineError> {
    let raw = opt_str(params, keys).ok_or_else(|| EngineError::new(format!("missing required string parameter `{}`", keys[0])))?;
    Ok(SheetId::try_new(raw)?)
}

fn opt_str<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| params.get(*k).and_then(Value::as_str))
}

fn usize_opt(params: &Value, key: &str) -> Option<usize> {
    params.get(key).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok())
}

fn has_any_key(params: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|k| params.get(*k).is_some())
}

fn next_unique_sheet_id(doc: &ProjectDocument, prefix: &str) -> Result<SheetId, EngineError> {
    for idx in 1..10_000_usize {
        let candidate = SheetId::try_new(format!("{prefix}-{idx}"))?;
        if doc.sheet_section(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(EngineError::new("exhausted generated sheet IDs"))
}

fn parse_usize_list(val: Option<&Value>) -> Result<Option<Vec<usize>>, EngineError> {
    let Some(v) = val else {
        return Ok(None);
    };
    let arr = v.as_array().ok_or_else(|| EngineError::new("expected array of non-negative integers"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let raw = item.as_u64().ok_or_else(|| EngineError::new("array item must be a non-negative integer"))?;
        let idx = usize::try_from(raw).map_err(|_| EngineError::new("integer exceeds platform index bounds"))?;
        out.push(idx);
    }
    Ok(Some(out))
}

fn parse_named_params_vec(val: Option<&Value>) -> Result<Vec<(String, f64)>, EngineError> {
    let mut vec = Vec::new();
    let Some(obj) = val.and_then(Value::as_object) else {
        return Ok(vec);
    };
    for (k, v) in obj {
        let num = v.as_f64().ok_or_else(|| EngineError::new(format!("parameter `{k}` must be numeric")))?;
        vec.push((k.clone(), num));
    }
    Ok(vec)
}

fn merge_json_struct<T: Serialize>(base: &T, overlay: &Value) -> Result<Value, EngineError> {
    let mut base_val = serde_json::to_value(base).map_err(|e| EngineError::new(format!("serialization error: {e}")))?;
    merge_json_values(&mut base_val, overlay);
    Ok(base_val)
}

fn merge_json_values(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (k, v) in over_map {
                match base_map.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge_json_values(existing, v),
                    _ => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (target, source) => {
            *target = source.clone();
        }
    }
}

fn strip_keys(val: &Value, ignored: &[&str]) -> Value {
    let Some(map) = val.as_object() else {
        return val.clone();
    };
    let mut out = Map::new();
    for (k, v) in map {
        if !ignored.contains(&k.as_str()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn parse_rgba8(val: &Value) -> Result<Rgba8, EngineError> {
    if let Some(hex) = val.as_str() {
        return parse_color_string(hex);
    }
    if let Some(arr) = val.as_array()
        && (arr.len() == 3 || arr.len() == 4)
    {
        let r = byte_at(arr, 0)?;
        let g = byte_at(arr, 1)?;
        let b = byte_at(arr, 2)?;
        let a = if arr.len() == 4 { byte_at(arr, 3)? } else { 255 };
        return Ok(Rgba8::rgba(r, g, b, a));
    }
    serde_json::from_value::<Rgba8>(val.clone()).map_err(|e| EngineError::new(format!("invalid color `{val}`: {e}")))
}

fn byte_at(arr: &[Value], idx: usize) -> Result<u8, EngineError> {
    let raw = arr.get(idx).and_then(Value::as_u64).ok_or_else(|| EngineError::new("color channel must be 0..=255"))?;
    u8::try_from(raw).map_err(|_| EngineError::new("color channel out of u8 range"))
}

fn parse_color_string(raw: &str) -> Result<Rgba8, EngineError> {
    let s = raw.trim();
    match s.to_ascii_lowercase().as_str() {
        "black" => return Ok(Rgba8::BLACK),
        "white" => return Ok(Rgba8::WHITE),
        "transparent" => return Ok(Rgba8::TRANSPARENT),
        "red" => return Ok(Rgba8::rgb(214, 39, 40)),
        "blue" => return Ok(Rgba8::rgb(31, 119, 180)),
        "green" => return Ok(Rgba8::rgb(44, 160, 44)),
        "orange" => return Ok(Rgba8::rgb(255, 127, 14)),
        "purple" => return Ok(Rgba8::rgb(148, 103, 189)),
        "teal" => return Ok(Rgba8::rgb(23, 190, 207)),
        "gray" | "grey" => return Ok(Rgba8::rgb(127, 127, 127)),
        _ => {}
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !hex.is_ascii() {
        return Err(EngineError::new(format!("invalid hex color `{raw}`")));
    }
    match hex.len() {
        6 => {
            let (Some(r), Some(g), Some(b)) = (hex.get(0..2), hex.get(2..4), hex.get(4..6)) else {
                return Err(EngineError::new(format!("invalid hex color `{raw}`")));
            };
            Ok(Rgba8::rgb(parse_hex_u8(r)?, parse_hex_u8(g)?, parse_hex_u8(b)?))
        }
        8 => {
            let (Some(r), Some(g), Some(b), Some(a)) = (hex.get(0..2), hex.get(2..4), hex.get(4..6), hex.get(6..8)) else {
                return Err(EngineError::new(format!("invalid hex color `{raw}`")));
            };
            Ok(Rgba8::rgba(parse_hex_u8(r)?, parse_hex_u8(g)?, parse_hex_u8(b)?, parse_hex_u8(a)?))
        }
        _ => Err(EngineError::new(format!("invalid hex color `{raw}`"))),
    }
}

fn parse_hex_u8(pair: &str) -> Result<u8, EngineError> {
    u8::from_str_radix(pair, 16).map_err(|_| EngineError::new(format!("invalid hex byte `{pair}`")))
}

fn parse_pair_f64(val: Option<&Value>, def: (f64, f64)) -> Result<(f64, f64), EngineError> {
    let Some(v) = val else {
        return Ok(def);
    };
    if let Some(arr) = v.as_array()
        && arr.len() == 2
    {
        let x = arr[0].as_f64().ok_or_else(|| EngineError::new("point x must be f64"))?;
        let y = arr[1].as_f64().ok_or_else(|| EngineError::new("point y must be f64"))?;
        return Ok((x, y));
    }
    if let Some(obj) = v.as_object()
        && let (Some(x), Some(y)) = (obj.get("x").and_then(Value::as_f64), obj.get("y").and_then(Value::as_f64))
    {
        return Ok((x, y));
    }
    Err(EngineError::new("invalid `(x, y)` point"))
}

fn parse_rect_pt(val: Option<&Value>, default: RectPt) -> Result<RectPt, EngineError> {
    let Some(v) = val else {
        return Ok(default);
    };
    if let Some(arr) = v.as_array()
        && arr.len() == 4
    {
        let x = arr[0].as_f64().ok_or_else(|| EngineError::new("rect x must be f64"))?;
        let y = arr[1].as_f64().ok_or_else(|| EngineError::new("rect y must be f64"))?;
        let w = arr[2].as_f64().ok_or_else(|| EngineError::new("rect width must be f64"))?;
        let h = arr[3].as_f64().ok_or_else(|| EngineError::new("rect height must be f64"))?;
        return Ok(RectPt::new(x, y, w, h));
    }
    serde_json::from_value(v.clone()).map_err(|e| EngineError::new(format!("invalid `bounds_pt`: {e}")))
}

fn parse_graph_kind(val: &Value) -> Result<GraphKind, EngineError> {
    if let Ok(kind) = serde_json::from_value::<GraphKind>(val.clone()) {
        return Ok(kind);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("xy" | "xy_scatter" | "scatter" | "xy_line" | "line" | "xy_mean_error") => Ok(GraphKind::Xy),
        Some("column" | "column_bar" | "bar" | "column_box" | "column_violin" | "box" | "violin") => Ok(GraphKind::Column),
        Some("grouped" | "grouped_bar" | "grouped_mean_error") => Ok(GraphKind::Grouped),
        Some("contingency" | "contingency_stacked" | "contingency_grouped") => Ok(GraphKind::Contingency),
        Some("survival" | "survival_kaplan_meier" | "kaplan_meier") => Ok(GraphKind::Survival),
        Some("parts_of_whole" | "pie" | "donut") => Ok(GraphKind::PartsOfWhole),
        Some("multiple_variables" | "multivariate" | "bubble") => Ok(GraphKind::MultipleVariables),
        Some("heatmap") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)),
        Some("volcano") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::Volcano)),
        Some("bland_altman") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman)),
        Some("roc") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve)),
        Some("forest") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot)),
        Some("residuals" | "residual_plot") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot)),
        Some("qq" | "normal_qq") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot)),
        Some(other) => Err(EngineError::new(format!("unknown graph kind `{other}`"))),
        None => Err(EngineError::new("graph `kind` must be a string or typed `GraphKind` object")),
    }
}

fn parse_frame_preset(val: &Value) -> Result<FramePreset, EngineError> {
    if let Ok(fp) = serde_json::from_value::<FramePreset>(val.clone()) {
        return Ok(fp);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("open_l" | "bottom_left" | "classic" | "plain") => Ok(FramePreset::OpenL),
        Some("offset" | "offset_range" | "tufte") => Ok(FramePreset::OffsetRangeTufte),
        Some("plain_boxed" | "box" | "plain_box") => Ok(FramePreset::PlainBoxed),
        Some("dual_y" | "dual_y_secondary_right") => Ok(FramePreset::DualYSecondaryRight),
        Some("origin_crossing" | "origin_cross" | "cross") => Ok(FramePreset::OriginCrossing),
        _ => Err(EngineError::new(format!("invalid `frame_preset`: {val}"))),
    }
}

fn parse_axis_scale(val: &Value) -> Result<GraphAxisScale, EngineError> {
    if let Ok(scale) = serde_json::from_value::<GraphAxisScale>(val.clone()) {
        return Ok(scale);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("linear") => Ok(GraphAxisScale::Linear),
        Some("log10" | "log") => Ok(GraphAxisScale::Log10),
        Some("log2") => Ok(GraphAxisScale::Log2),
        Some("ln") => Ok(GraphAxisScale::Ln),
        Some("probit" | "probability") => Ok(GraphAxisScale::Probit),
        Some("logit" | "logit_percent") => Ok(GraphAxisScale::Logit(LogitDomain::Percent)),
        Some("logit_proportion") => Ok(GraphAxisScale::Logit(LogitDomain::Proportion)),
        Some("reciprocal") => Ok(GraphAxisScale::Reciprocal),
        _ => Err(EngineError::new(format!("invalid axis `scale`: {val}"))),
    }
}

fn parse_tick_format(val: &Value) -> Result<TickLabelFormat, EngineError> {
    if let Ok(tf) = serde_json::from_value::<TickLabelFormat>(val.clone()) {
        return Ok(tf);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("decimal") => Ok(TickLabelFormat::Decimal),
        Some("power_of_ten" | "power_of_10" | "powers_of_10" | "superscript") => Ok(TickLabelFormat::PowerOfTenSuperscript),
        Some("scientific" | "scientific_e") => Ok(TickLabelFormat::ScientificE),
        Some("antilog") => Ok(TickLabelFormat::Antilog),
        Some("engineering" | "si" | "engineering_si") => Ok(TickLabelFormat::EngineeringSi),
        Some("categorical" | "row_titles") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::RowTitles)),
        Some("column_titles") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles)),
        Some("row_numbers") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers)),
        _ => Err(EngineError::new(format!("invalid `tick_format`: {val}"))),
    }
}

fn parse_break_ornament(val: &Value) -> Result<BreakOrnament, EngineError> {
    if let Ok(b) = serde_json::from_value::<BreakOrnament>(val.clone()) {
        return Ok(b);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("double_slash" | "double") => Ok(BreakOrnament::DoubleSlash),
        Some("zigzag") => Ok(BreakOrnament::Zigzag),
        Some("blank" | "blank_gap" | "none") => Ok(BreakOrnament::BlankGap),
        _ => Err(EngineError::new(format!("invalid `break_ornament`: {val}"))),
    }
}

fn parse_axis_threshold(val: &Value) -> Result<AxisThreshold, EngineError> {
    if let Ok(t) = serde_json::from_value::<AxisThreshold>(val.clone()) {
        return Ok(t);
    }
    let value = val.get("value").and_then(Value::as_f64).ok_or_else(|| EngineError::new("threshold missing numeric `value`"))?;
    let shade_to = val.get("shade_to").and_then(Value::as_f64);
    let label = opt_str(val, &["label"]).map(str::to_owned);
    let color = val.get("color").map(parse_rgba8).transpose()?.unwrap_or(Rgba8::rgb(120, 120, 120));
    let dash_style = val.get("dash_style").map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Dashed);
    let stroke_width_pt = val.get("stroke_width_pt").and_then(Value::as_f64).unwrap_or(1.0);
    Ok(AxisThreshold { value, shade_to, color, dash_style, stroke_width_pt, label })
}

fn parse_marker_shape(val: &Value) -> Result<MarkerShape, EngineError> {
    if let Ok(s) = serde_json::from_value::<MarkerShape>(val.clone()) {
        return Ok(s);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("circle") => Ok(MarkerShape::Circle),
        Some("square") => Ok(MarkerShape::Square),
        Some("triangle" | "triangle_up") => Ok(MarkerShape::TriangleUp),
        Some("triangle_down") => Ok(MarkerShape::TriangleDown),
        Some("diamond") => Ok(MarkerShape::Diamond),
        Some("cross" | "x") => Ok(MarkerShape::Cross),
        Some("plus") => Ok(MarkerShape::Plus),
        Some("star") => Ok(MarkerShape::Star),
        Some("hexagon") => Ok(MarkerShape::Hexagon),
        _ => Err(EngineError::new(format!("invalid marker `shape`: {val}"))),
    }
}

fn parse_marker_fill_mode(val: &Value) -> Result<MarkerFillMode, EngineError> {
    if let Ok(f) = serde_json::from_value::<MarkerFillMode>(val.clone()) {
        return Ok(f);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("filled" | "solid") => Ok(MarkerFillMode::Filled),
        Some("open" | "hollow") => Ok(MarkerFillMode::Open),
        Some("semi_top") => Ok(MarkerFillMode::SemiFilledTop),
        Some("semi_bottom") => Ok(MarkerFillMode::SemiFilledBottom),
        Some("semi_left") => Ok(MarkerFillMode::SemiFilledLeft),
        Some("semi_right") => Ok(MarkerFillMode::SemiFilledRight),
        _ => Err(EngineError::new(format!("invalid marker `fill_mode`: {val}"))),
    }
}

fn parse_line_connection_kind(val: &Value) -> Result<LineConnectionKind, EngineError> {
    if let Ok(k) = serde_json::from_value::<LineConnectionKind>(val.clone()) {
        return Ok(k);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("point_to_point" | "straight") => Ok(LineConnectionKind::PointToPoint),
        Some("staircase" | "step" | "stepped") => Ok(LineConnectionKind::Staircase),
        Some("spline" | "smooth" | "monotonic_cubic") => Ok(LineConnectionKind::MonotonicCubicSpline),
        Some("akima") => Ok(LineConnectionKind::AkimaSpline),
        _ => Err(EngineError::new(format!("invalid connecting line `kind`: {val}"))),
    }
}

fn parse_line_dash_style(val: &Value) -> Result<LineDashStyle, EngineError> {
    if let Ok(d) = serde_json::from_value::<LineDashStyle>(val.clone()) {
        return Ok(d);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("solid") => Ok(LineDashStyle::Solid),
        Some("dashed" | "dash") => Ok(LineDashStyle::Dashed),
        Some("dotted" | "dot") => Ok(LineDashStyle::Dotted),
        Some("dash_dot" | "dashdot") => Ok(LineDashStyle::DashDot),
        Some("dash_dot_dot") => Ok(LineDashStyle::DashDotDot),
        Some("long_dash") => Ok(LineDashStyle::LongDash),
        _ => Err(EngineError::new(format!("invalid `dash_style`: {val}"))),
    }
}

fn parse_error_bar_metric(val: &Value) -> Result<ErrorBarMetric, EngineError> {
    if let Ok(m) = serde_json::from_value::<ErrorBarMetric>(val.clone()) {
        return Ok(m);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sd") => Ok(ErrorBarMetric::Sd),
        Some("sem") => Ok(ErrorBarMetric::Sem),
        Some("ci95" | "ci_95" | "95ci") => Ok(ErrorBarMetric::ConfidenceInterval95),
        Some("iqr") => Ok(ErrorBarMetric::InterquartileRange),
        Some("range") => Ok(ErrorBarMetric::Range),
        Some("explicit") => Ok(ErrorBarMetric::ExplicitSubcolumns),
        _ => Err(EngineError::new(format!("invalid error bar `metric`: {val}"))),
    }
}

fn parse_error_bar_direction(val: &Value) -> Result<ErrorBarDirection, EngineError> {
    if let Ok(d) = serde_json::from_value::<ErrorBarDirection>(val.clone()) {
        return Ok(d);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("both") => Ok(ErrorBarDirection::Both),
        Some("above" | "upper" | "upper_only") => Ok(ErrorBarDirection::UpperOnly),
        Some("below" | "lower" | "lower_only") => Ok(ErrorBarDirection::LowerOnly),
        _ => Err(EngineError::new(format!("invalid error bar `direction`: {val}"))),
    }
}

fn parse_bracket_style(raw: Option<&str>) -> Result<BracketStyle, EngineError> {
    match raw.unwrap_or("square").trim().to_ascii_lowercase().as_str() {
        "square" | "capped" | "ticks" => Ok(BracketStyle::Square),
        "half_tick" => Ok(BracketStyle::HalfTick),
        "flat" | "flat_line" | "plain" => Ok(BracketStyle::FlatLine),
        "curly" | "curly_brace" => Ok(BracketStyle::CurlyBrace),
        other => Err(EngineError::new(format!("invalid bracket `style` `{other}`"))),
    }
}

fn parse_bracket_label_mode(val: Option<&Value>) -> Result<BracketLabelMode, EngineError> {
    let Some(v) = val else {
        return Ok(BracketLabelMode::SummaryStars);
    };
    if let Ok(mode) = serde_json::from_value::<BracketLabelMode>(v.clone()) {
        return Ok(mode);
    }
    match v.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("stars" | "summary_stars" | "stars_only") => Ok(BracketLabelMode::SummaryStars),
        Some("numeric" | "p_value" | "p_only" | "stars_and_p") => Ok(BracketLabelMode::NumericPOrQ),
        Some(custom) => Ok(BracketLabelMode::Custom(custom.to_owned())),
        None => Err(EngineError::new("invalid `label_mode`")),
    }
}

fn parse_journal_preset(val: &Value) -> Result<JournalWidthPreset, EngineError> {
    if let Ok(p) = serde_json::from_value::<JournalWidthPreset>(val.clone()) {
        return Ok(p);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("single_column" | "single" | "85mm") => Ok(JournalWidthPreset::SingleColumn85Mm),
        Some("89mm") => Ok(JournalWidthPreset::SingleColumn89Mm),
        Some("column_1_5" | "one_and_half" | "120mm") => Ok(JournalWidthPreset::OneAndHalfColumn120Mm),
        Some("136mm") => Ok(JournalWidthPreset::OneAndHalfColumn136Mm),
        Some("double_column" | "double" | "174mm") => Ok(JournalWidthPreset::DoubleColumn174Mm),
        Some("183mm") => Ok(JournalWidthPreset::DoubleColumn183Mm),
        Some("us_letter" | "letter" | "letter_portrait") => Ok(JournalWidthPreset::LetterPortrait),
        Some("letter_landscape") => Ok(JournalWidthPreset::LetterLandscape),
        Some("full_page" | "a4" | "a4_portrait") => Ok(JournalWidthPreset::A4Portrait),
        Some("a4_landscape") => Ok(JournalWidthPreset::A4Landscape),
        _ => Err(EngineError::new(format!("invalid layout `preset`: {val}"))),
    }
}

fn parse_physical_unit(val: &Value) -> Result<PhysicalUnit, EngineError> {
    if let Ok(u) = serde_json::from_value::<PhysicalUnit>(val.clone()) {
        return Ok(u);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("mm" | "millimeters") => Ok(PhysicalUnit::Millimeters),
        Some("cm" | "centimeters") => Ok(PhysicalUnit::Centimeters),
        Some("in" | "inch" | "inches") => Ok(PhysicalUnit::Inches),
        Some("pt" | "points") => Ok(PhysicalUnit::Points),
        _ => Err(EngineError::new(format!("invalid physical `unit`: {val}"))),
    }
}

fn parse_page_orientation(val: &Value) -> Result<PageOrientation, EngineError> {
    if let Ok(o) = serde_json::from_value::<PageOrientation>(val.clone()) {
        return Ok(o);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("portrait") => Ok(PageOrientation::Portrait),
        Some("landscape") => Ok(PageOrientation::Landscape),
        _ => Err(EngineError::new(format!("invalid `orientation`: {val}"))),
    }
}

fn parse_panel_badge_style(val: &Value) -> Result<PanelBadgeStyle, EngineError> {
    if let Ok(b) = serde_json::from_value::<PanelBadgeStyle>(val.clone()) {
        return Ok(b);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("none") => Ok(PanelBadgeStyle::None),
        Some("uppercase" | "abc") => Ok(PanelBadgeStyle::UpperCaseLetters),
        Some("lowercase" | "abc_lower") => Ok(PanelBadgeStyle::LowerCaseLetters),
        _ => Err(EngineError::new(format!("invalid `panel_badges`: {val}"))),
    }
}
