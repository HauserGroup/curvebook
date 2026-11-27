//! Tidy RFC-4180 CSV and structured JSON exporters for [`ResultSheet`] outputs,
//! designed for downstream R, Python, Snakemake, and Nextflow analysis pipelines.

use curvebook_doc::{
    AnalysisSpec, ChecklistItem, CiBound, DataSheet, FittedCurveOverlay, PairwiseComparisonRecord, ResultCell, ResultCellDetail, ResultProvenance,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, SheetId, SourceCellId,
};

use crate::archive::validate_data_sheet_floats;
use crate::csv::{CsvOptions, neutralize_csv_formula_text, write_delimited_records};
use crate::error::FormatError;

/// Structured JSON representation of a [`ResultSheet`] for external pipeline consumption.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ResultSheetExportJson<'a> {
    pub id: &'a SheetId,
    pub title: &'a str,
    pub status: &'a ResultStatus,
    pub provenance: &'a ResultProvenance,
    pub analysis: &'a AnalysisSpec,
    pub scalars: &'a [ResultScalarEntry],
    pub tabs: &'a [ResultTab],
    pub pairwise_comparisons: &'a [PairwiseComparisonRecord],
    pub fitted_curves: &'a [FittedCurveOverlay],
    pub eliminated_outliers: &'a [SourceCellId],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analyzable_data: Option<&'a DataSheet>,
    pub checklist: &'a [ChecklistItem],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub cell_details: &'a [ResultCellDetail],
}

/// Return a human-readable provenance or staleness warning when exporting an imported [`ResultSheet`],
/// or `None` for natively computed results.
#[must_use]
pub fn result_sheet_export_warning(sheet: &ResultSheet) -> Option<String> {
    match &sheet.provenance {
        ResultProvenance::Computed => None,
        ResultProvenance::Imported { source_format, original_kind_label, .. } => {
            let kind_suffix = if original_kind_label.trim().is_empty() { String::new() } else { format!(" ({original_kind_label})") };
            if matches!(sheet.status, ResultStatus::ImportedStale) {
                Some(format!(
                    "Result `{}` is a stale imported snapshot from `{source_format}`{kind_suffix}; upstream data changed after import and values were not computed by CurveBook.",
                    sheet.id
                ))
            } else {
                Some(format!("Result `{}` is an imported snapshot from `{source_format}`{kind_suffix} and was not computed by CurveBook.", sheet.id))
            }
        }
    }
}

/// Validate that `sheet` has not recorded [`ResultStatus::Failed`] before CSV export.
///
/// Failed analyses produce empty tabs and must not be silently written as header-only CSV files.
pub fn ensure_result_sheet_csv_exportable(sheet: &ResultSheet) -> Result<(), FormatError> {
    if let ResultStatus::Failed { code, message } = &sheet.status {
        return Err(FormatError::InvalidPipeline {
            reason: format!("cannot export failed result sheet `{}` ({}): {message}", sheet.id, code.as_str()),
        });
    }
    Ok(())
}

/// Export a single [`ResultTab`] as a wide RFC-4180 CSV/TSV table.
#[must_use]
pub fn export_result_tab_csv(tab: &ResultTab, opts: &CsvOptions) -> String {
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(tab.rows.len() + 1);
    let include_row_labels = !tab.row_labels.is_empty();
    let mut header = Vec::new();
    if include_row_labels {
        header.push(String::from("Row"));
    }
    header.extend(tab.column_headers.iter().map(|h| neutralize_csv_formula_text(h)));
    rows.push(header);

    for (r_idx, row) in tab.rows.iter().enumerate() {
        let mut out_row = Vec::new();
        if include_row_labels {
            let label = tab.row_labels.get(r_idx).map(String::as_str).unwrap_or("");
            out_row.push(neutralize_csv_formula_text(label));
        }
        for cell in row {
            out_row.push(format_result_cell_display(cell, opts));
        }
        rows.push(out_row);
    }

    write_delimited_records(&rows, opts)
}

/// Export a single [`ResultTab`] belonging to `sheet` as CSV/TSV, refusing failed result sheets.
pub fn try_export_result_tab_csv(sheet: &ResultSheet, tab: &ResultTab, opts: &CsvOptions) -> Result<String, FormatError> {
    try_export_result_tab_csv_with_warnings(sheet, tab, opts).map(|(csv, _)| csv)
}

/// Export a single [`ResultTab`] belonging to `sheet` as CSV/TSV, refusing failed result sheets
/// and surfacing any imported-result provenance or staleness warning.
pub fn try_export_result_tab_csv_with_warnings(
    sheet: &ResultSheet,
    tab: &ResultTab,
    opts: &CsvOptions,
) -> Result<(String, Vec<String>), FormatError> {
    ensure_result_sheet_csv_exportable(sheet)?;
    let warnings = result_sheet_export_warning(sheet).into_iter().collect();
    Ok((export_result_tab_csv(tab, opts), warnings))
}

/// Export all tabs, hooked scalars, pairwise comparisons, fitted curve coordinates, and
/// assumption checklist items from a [`ResultSheet`] into a unified tidy RFC-4180 CSV dataframe.
#[must_use]
pub fn export_result_sheet_tidy_csv(sheet: &ResultSheet, opts: &CsvOptions) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    rows.push(tidy_header());

    let res_id = neutralize_csv_formula_text(sheet.id.as_str());
    let title = neutralize_csv_formula_text(sheet.title());
    let provenance = tidy_provenance_value(sheet);

    append_tab_tidy_rows(&mut rows, &res_id, &title, &sheet.tabs, opts);
    append_scalar_tidy_rows(&mut rows, &res_id, &title, &sheet.scalars, opts);
    append_comparison_tidy_rows(&mut rows, &res_id, &title, &sheet.pairwise_comparisons, opts);
    append_curve_tidy_rows(&mut rows, &res_id, &title, &sheet.fitted_curves, opts);
    append_eliminated_outlier_tidy_rows(&mut rows, &res_id, &title, &sheet.eliminated_outliers);
    append_checklist_tidy_rows(&mut rows, &res_id, &title, &sheet.checklist);

    for row in rows.iter_mut().skip(1) {
        row.push(provenance.clone());
    }

    write_delimited_records(&rows, opts)
}

/// Export a [`ResultSheet`] as a tidy RFC-4180 CSV dataframe, refusing failed result sheets.
pub fn try_export_result_sheet_tidy_csv(sheet: &ResultSheet, opts: &CsvOptions) -> Result<String, FormatError> {
    try_export_result_sheet_tidy_csv_with_warnings(sheet, opts).map(|(csv, _)| csv)
}

/// Export a [`ResultSheet`] as a tidy RFC-4180 CSV dataframe, refusing failed result sheets
/// and surfacing any imported-result provenance or staleness warning.
pub fn try_export_result_sheet_tidy_csv_with_warnings(sheet: &ResultSheet, opts: &CsvOptions) -> Result<(String, Vec<String>), FormatError> {
    ensure_result_sheet_csv_exportable(sheet)?;
    let warnings = result_sheet_export_warning(sheet).into_iter().collect();
    Ok((export_result_sheet_tidy_csv(sheet, opts), warnings))
}

/// Export a [`ResultSheet`] to structured JSON (`serde_json`), validating that no non-finite
/// floats are silently lost.
pub fn export_result_sheet_json(sheet: &ResultSheet, pretty: bool) -> Result<String, FormatError> {
    validate_result_floats(sheet)?;
    let payload = ResultSheetExportJson {
        id: &sheet.id,
        title: sheet.title(),
        status: &sheet.status,
        provenance: &sheet.provenance,
        analysis: &sheet.analysis,
        scalars: &sheet.scalars,
        tabs: &sheet.tabs,
        pairwise_comparisons: &sheet.pairwise_comparisons,
        fitted_curves: &sheet.fitted_curves,
        eliminated_outliers: &sheet.eliminated_outliers,
        analyzable_data: sheet.analyzable_data.as_ref(),
        checklist: &sheet.checklist,
        cell_details: &sheet.cell_details,
    };
    let json = if pretty { serde_json::to_string_pretty(&payload)? } else { serde_json::to_string(&payload)? };
    Ok(json)
}

fn tidy_provenance_value(sheet: &ResultSheet) -> String {
    match (&sheet.provenance, &sheet.status) {
        (ResultProvenance::Computed, _) => String::from("computed"),
        (ResultProvenance::Imported { source_format, .. }, ResultStatus::ImportedStale) => {
            neutralize_csv_formula_text(&format!("imported_stale:{source_format}"))
        }
        (ResultProvenance::Imported { source_format, .. }, _) => neutralize_csv_formula_text(&format!("imported:{source_format}")),
    }
}

fn tidy_header() -> Vec<String> {
    vec![
        String::from("result_id"),
        String::from("analysis_title"),
        String::from("record_type"),
        String::from("tab_id"),
        String::from("tab_title"),
        String::from("tab_kind"),
        String::from("row_label"),
        String::from("col_header"),
        String::from("value_text"),
        String::from("numeric_value"),
        String::from("prefix"),
        String::from("units"),
        String::from("raw_p_or_q"),
        String::from("stars"),
        String::from("ci_lower"),
        String::from("ci_upper"),
        String::from("significant"),
        String::from("status"),
        String::from("diagnostic_metric"),
        String::from("pred_lower"),
        String::from("pred_upper"),
        String::from("provenance"),
    ]
}

fn append_tab_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, tabs: &[ResultTab], opts: &CsvOptions) {
    for tab in tabs {
        let kind_str = format!("{:?}", tab.kind);
        let safe_tab_id = neutralize_csv_formula_text(&tab.id);
        let safe_tab_title = neutralize_csv_formula_text(&tab.title);
        for (r_idx, row) in tab.rows.iter().enumerate() {
            let row_label = tab.row_labels.get(r_idx).map_or_else(|| format!("row_{r_idx}"), |s| neutralize_csv_formula_text(s));
            for (c_idx, cell) in row.iter().enumerate() {
                let col_header = tab.column_headers.get(c_idx).map_or_else(|| format!("col_{c_idx}"), |s| neutralize_csv_formula_text(s));
                let mut rec = vec![
                    res_id.to_owned(),
                    title.to_owned(),
                    String::from("tab_cell"),
                    safe_tab_id.clone(),
                    safe_tab_title.clone(),
                    kind_str.clone(),
                    row_label.clone(),
                    col_header,
                ];
                append_cell_columns(&mut rec, cell, opts);
                rec.push(String::new()); // significant
                rec.push(String::new()); // status
                rec.push(String::new()); // diagnostic_metric
                rec.push(String::new()); // pred_lower
                rec.push(String::new()); // pred_upper
                rows.push(rec);
            }
        }
        for (f_idx, footnote) in tab.footnotes.iter().enumerate() {
            rows.push(vec![
                res_id.to_owned(),
                title.to_owned(),
                String::from("tab_footnote"),
                safe_tab_id.clone(),
                safe_tab_title.clone(),
                kind_str.clone(),
                format!("footnote_{f_idx}"),
                String::from("footnote"),
                neutralize_csv_formula_text(footnote),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
        }
    }
}

fn append_cell_columns(rec: &mut Vec<String>, cell: &ResultCell, opts: &CsvOptions) {
    let display = format_result_cell_display(cell, opts);
    match cell {
        ResultCell::Empty => {
            rec.extend([display, String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new()])
        }
        ResultCell::Number { value, prefix, units } => {
            rec.extend([
                display,
                format_float_opt(Some(*value), opts),
                prefix.as_deref().map_or_else(String::new, neutralize_csv_formula_text),
                units.as_deref().map_or_else(String::new, neutralize_csv_formula_text),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            rec.extend([
                display,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                format_ci_bound(*lower, opts),
                format_ci_bound(*upper, opts),
            ]);
        }
        ResultCell::PValue { raw, stars, .. } => {
            rec.extend([
                display,
                format_float_opt(Some(*raw), opts),
                String::new(),
                String::new(),
                format_float_opt(Some(*raw), opts),
                neutralize_csv_formula_text(stars),
                String::new(),
                String::new(),
            ]);
        }
        ResultCell::Text(_) => {
            rec.extend([display, String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new()]);
        }
        ResultCell::Boolean(b) => {
            rec.extend([b.to_string(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new()]);
        }
    }
}

fn append_scalar_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, scalars: &[ResultScalarEntry], opts: &CsvOptions) {
    for entry in scalars {
        let ds_label = entry.data_set_index.map_or_else(|| String::from("global"), |i| format!("data_set_{i}"));
        let val_str = format_float_opt(Some(entry.value), opts);
        rows.push(vec![
            res_id.to_owned(),
            title.to_owned(),
            String::from("scalar"),
            String::new(),
            String::new(),
            String::new(),
            neutralize_csv_formula_text(&entry.key),
            ds_label,
            val_str.clone(),
            val_str,
            String::new(),
            entry.units.as_deref().map_or_else(String::new, neutralize_csv_formula_text),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]);
    }
}

fn append_comparison_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, comparisons: &[PairwiseComparisonRecord], opts: &CsvOptions) {
    for cmp in comparisons {
        let pair_label = neutralize_csv_formula_text(&format!("{} vs {}", cmp.label_a, cmp.label_b));
        let metric_kind = if cmp.is_fdr_q { "FDR q" } else { "FWER p" };
        let diff_str = format_float_opt(cmp.estimate_diff, opts);
        let p_str = format_float_opt(Some(cmp.raw_p_or_q), opts);
        rows.push(vec![
            res_id.to_owned(),
            title.to_owned(),
            String::from("pairwise_comparison"),
            String::new(),
            String::new(),
            String::from("MultipleComparisons"),
            pair_label,
            String::from(metric_kind),
            diff_str.clone(),
            diff_str,
            String::new(),
            String::new(),
            p_str,
            String::new(),
            String::new(),
            String::new(),
            cmp.significant.to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]);
    }
}

fn append_curve_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, curves: &[FittedCurveOverlay], opts: &CsvOptions) {
    for curve in curves {
        let diag = format!("converged={},ambiguous={}", curve.converged, curve.ambiguous);
        let safe_eq_id = neutralize_csv_formula_text(&curve.equation_id);
        for pt in &curve.points {
            let x_str = format_float_opt(Some(pt.x), opts);
            let y_str = format_float_opt(Some(pt.y), opts);
            rows.push(vec![
                res_id.to_owned(),
                title.to_owned(),
                String::from("curve_point"),
                safe_eq_id.clone(),
                format!("data_set_{}", curve.data_set_index),
                String::from("CurveCoordinates"),
                x_str,
                String::from("y"),
                y_str.clone(),
                y_str,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                format_float_opt(pt.ci_lower, opts),
                format_float_opt(pt.ci_upper, opts),
                String::new(),
                String::new(),
                diag.clone(),
                format_float_opt(pt.pred_lower, opts),
                format_float_opt(pt.pred_upper, opts),
            ]);
        }
    }
}

fn append_eliminated_outlier_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, outliers: &[SourceCellId]) {
    for outlier in outliers {
        let safe_sheet_id = neutralize_csv_formula_text(outlier.sheet_id.as_str());
        let coord = format!("sheet={},row={},col={},subcol={}", outlier.sheet_id.as_str(), outlier.row, outlier.col, outlier.subcol);
        rows.push(vec![
            res_id.to_owned(),
            title.to_owned(),
            String::from("eliminated_outlier"),
            safe_sheet_id,
            format!("data_set_{}", outlier.col),
            String::from("EliminatedOutliers"),
            format!("row_{}", outlier.row),
            format!("col_{}_sub_{}", outlier.col, outlier.subcol),
            neutralize_csv_formula_text(&coord),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]);
    }
}

fn append_checklist_tidy_rows(rows: &mut Vec<Vec<String>>, res_id: &str, title: &str, checklist: &[ChecklistItem]) {
    for item in checklist {
        rows.push(vec![
            res_id.to_owned(),
            title.to_owned(),
            String::from("checklist"),
            String::new(),
            String::new(),
            String::from("Diagnostics"),
            neutralize_csv_formula_text(&item.assumption),
            String::from("explanation"),
            neutralize_csv_formula_text(&item.explanation),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            format!("{:?}", item.status),
            item.diagnostic_metric.as_deref().map_or_else(String::new, neutralize_csv_formula_text),
            String::new(),
            String::new(),
        ]);
    }
}

fn format_result_cell_display(cell: &ResultCell, opts: &CsvOptions) -> String {
    match cell {
        ResultCell::Empty => String::new(),
        ResultCell::Number { value, prefix, units } => {
            let num = format_float_opt(Some(*value), opts);
            let has_prefix = prefix.as_deref().is_some_and(|p| !p.is_empty());
            let has_units = units.as_deref().is_some_and(|u| !u.is_empty());
            if !has_prefix && !has_units {
                return num;
            }
            let mut out = String::new();
            if let Some(p) = prefix {
                out.push_str(p);
            }
            out.push_str(&num);
            if let Some(u) = units
                && !u.is_empty()
            {
                out.push(' ');
                out.push_str(u);
            }
            neutralize_csv_formula_text(&out)
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            format!("[{}, {}]", format_ci_bound(*lower, opts), format_ci_bound(*upper, opts))
        }
        ResultCell::PValue { formatted, stars, .. } => {
            let raw = if stars.is_empty() { formatted.clone() } else { format!("{formatted} ({stars})") };
            neutralize_csv_formula_text(&raw)
        }
        ResultCell::Text(s) => neutralize_csv_formula_text(s),
        ResultCell::Boolean(b) => b.to_string(),
    }
}

fn format_ci_bound(bound: CiBound, opts: &CsvOptions) -> String {
    match bound {
        CiBound::Finite(v) => format_float_opt(Some(v), opts),
        CiBound::Open => String::from("Open"),
        CiBound::Unavailable => String::from("Unavailable"),
    }
}

fn format_float_opt(val: Option<f64>, opts: &CsvOptions) -> String {
    let Some(v) = val.filter(|x| x.is_finite()) else {
        return String::new();
    };
    let clean = if v == 0.0 { 0.0 } else { v };
    let s = format!("{clean}");
    if opts.decimal_separator == ',' { s.replace('.', ",") } else { s }
}

fn validate_result_floats(sheet: &ResultSheet) -> Result<(), FormatError> {
    if let Some(field) = sheet.analysis.first_non_finite_float() {
        return Err(FormatError::NonFiniteFloat { location: format!("{}.{field}", sheet.id) });
    }
    for (idx, s) in sheet.scalars.iter().enumerate() {
        if !s.value.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.scalars[{idx}]", sheet.id) });
        }
    }
    for tab in &sheet.tabs {
        for (r, row) in tab.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                validate_result_cell_float(cell, &tab.id, r, c)?;
            }
        }
    }
    for (idx, cmp) in sheet.pairwise_comparisons.iter().enumerate() {
        if !cmp.raw_p_or_q.is_finite() || cmp.estimate_diff.is_some_and(|v| !v.is_finite()) {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.pairwise_comparisons[{idx}]", sheet.id) });
        }
    }
    for (c_idx, curve) in sheet.fitted_curves.iter().enumerate() {
        for (p_idx, pt) in curve.points.iter().enumerate() {
            if !pt.x.is_finite()
                || !pt.y.is_finite()
                || pt.ci_lower.is_some_and(|v| !v.is_finite())
                || pt.ci_upper.is_some_and(|v| !v.is_finite())
                || pt.pred_lower.is_some_and(|v| !v.is_finite())
                || pt.pred_upper.is_some_and(|v| !v.is_finite())
            {
                return Err(FormatError::NonFiniteFloat { location: format!("{}.fitted_curves[{c_idx}].points[{p_idx}]", sheet.id) });
            }
        }
    }
    for (idx, detail) in sheet.cell_details.iter().enumerate() {
        if detail.raw_value.is_some_and(|v| !v.is_finite()) {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.cell_details[{idx}].raw_value", sheet.id) });
        }
    }
    if let Some(ds) = sheet.analyzable_data.as_ref() {
        validate_data_sheet_floats(ds)?;
    }
    Ok(())
}

fn validate_result_cell_float(cell: &ResultCell, tab_id: &str, r: usize, c: usize) -> Result<(), FormatError> {
    match cell {
        ResultCell::Number { value, .. } if !value.is_finite() => Err(FormatError::NonFiniteFloat { location: format!("tab[{tab_id}][{r}][{c}]") }),
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(v), .. } | ResultCell::ConfidenceInterval { upper: CiBound::Finite(v), .. }
            if !v.is_finite() =>
        {
            Err(FormatError::NonFiniteFloat { location: format!("tab[{tab_id}][{r}][{c}]") })
        }
        ResultCell::PValue { raw, .. } if !raw.is_finite() => Err(FormatError::NonFiniteFloat { location: format!("tab[{tab_id}][{r}][{c}]") }),
        _ => Ok(()),
    }
}
