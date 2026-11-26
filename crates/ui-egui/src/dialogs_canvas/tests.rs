use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
    JournalWidthPreset, LayoutSheet, LayoutSpec, NonlinearFitSpec, OneWayAnovaSpec, PValueFormat, ProjectDocument, RemediationKind, ResultCell,
    ResultSheet, ResultStatus, TableArchetype, TwoGroupSpec, XyTable,
};

use super::analyze_dialog::collect_variable_candidates;
use super::{
    ActiveDialog, AssumptionDiagnosticBanner, AxisTarget, CanvasDialogState, CommandPaletteState, ConstantHookPickerState, FormatAxesState,
    HookTargetKind, ResultSheetViewState,
};
use crate::theme::{UiThemeSpec, wcag_contrast_ratio};

#[test]
fn command_palette_includes_both_engine_and_ui_commands() {
    let mut palette = CommandPaletteState { query: String::from("nav"), ..Default::default() };
    let hits = palette.matches();
    assert!(hits.iter().any(|h| h.id == "ui.navigator.toggle"));

    palette.query = String::from("exportPng");
    let hits = palette.matches();
    assert_eq!(hits.first().map(|h| h.id), Some("graph.exportPng"));
}

#[test]
fn result_cell_formatting_handles_open_ci_bounds_and_pvalues() {
    let ci = ResultSheetViewState::format_result_cell(
        &ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.5), upper: CiBound::Open },
        PValueFormat::FourDecimal,
    );
    assert!(ci.contains("Unbounded"));

    let p = ResultSheetViewState::format_result_cell(
        &ResultCell::PValue { raw: 0.0021, formatted: String::from("0.0021"), stars: String::from("**") },
        PValueFormat::Apa,
    );
    assert!(p.contains(".002"));
    assert!(p.contains("**"));
}

#[test]
fn format_axes_enforces_one_to_three_segments() {
    let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
    let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
    let gs = GraphSheet::new_uninitialized_for_data("g1", &ds).expect("gs");
    let mut axes = FormatAxesState::default();
    axes.sync_from_graph(&gs, AxisTarget::Y);
    assert!(!axes.remove_segment(0));
    assert!(axes.add_segment());
    assert!(axes.add_segment());
    assert!(!axes.add_segment());
    assert_eq!(axes.axis_draft.segments.len(), 3);
}

#[test]
fn duplicate_prompt_and_checklist_counts_work() {
    let mut doc = ProjectDocument::default();
    let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
    let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
    doc.add_data_sheet(ds).expect("add data");

    let spec = AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(Default::default())).expect("spec");
    let mut res = ResultSheet::new(spec).expect("result");
    res.status = ResultStatus::UpToDate;
    res.checklist.push(ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(String::from("p=0.04")),
        explanation: String::from("Mild skew"),
        remediation: None,
    });
    assert_eq!(ResultSheetViewState::checklist_summary_counts(&res), (0, 1, 0, 0));
    doc.add_result_sheet(res).expect("add result");

    let mut dialogs = CanvasDialogState::default();
    assert!(dialogs.open_analyze_dialog(&doc, &"d1".into()));
    dialogs.analyze.selected_entry_id = Some(String::from("fit.linear"));
    assert!(dialogs.submit_analyze_or_prompt(&doc).is_none());
    assert!(matches!(dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
}

#[test]
fn theme_specs_meet_wcag_aa_across_all_themes() {
    for theme in [UiThemeSpec::light(), UiThemeSpec::dark(), UiThemeSpec::high_contrast()] {
        assert!(wcag_contrast_ratio(theme.border, theme.panel_bg) >= 3.0, "border vs panel_bg failed for {:?}", theme.mode);
        assert!(wcag_contrast_ratio(theme.border, theme.surface) >= 3.0, "border vs surface failed for {:?}", theme.mode);
        assert!(wcag_contrast_ratio(theme.text_primary, theme.surface) >= 4.5, "text_primary vs surface failed for {:?}", theme.mode);
    }
}

#[test]
fn constant_hook_picker_emits_engine_compatible_commands_by_target() {
    let mut doc = ProjectDocument::default();
    let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
    let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
    doc.add_data_sheet(ds).expect("add data");
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol").expect("info");
    info.constants.push(InfoConstant::new("Baseline_EC50", ConstantValue::Number(12.5), Some(String::from("nM"))).expect("const"));
    doc.add_info_sheet(info).expect("add info");

    let fit_spec =
        AnalysisSpec::new("r1", "d1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).expect("fit spec");
    doc.add_result_sheet(ResultSheet::new(fit_spec).expect("fit res")).expect("add fit");
    let gs = GraphSheet::new_uninitialized_for_data("g1", doc.data_sheet(&"d1".into()).expect("d1")).expect("gs");
    doc.add_graph_sheet(gs).expect("add gs");
    let layout = LayoutSheet::new("l1", "Fig 1", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).expect("layout");
    doc.add_layout_sheet(layout).expect("add layout");

    let mut picker = ConstantHookPickerState::default();
    picker.open_for_document("r1".into(), Some(String::from("Bottom")), &doc);
    assert_eq!(picker.target_kind, HookTargetKind::AnalysisParameter);
    let res_intent = picker.commit_intent().expect("res hook");
    assert_eq!(res_intent.command_id, "analysis.updateParams");
    assert_eq!(res_intent.params["sheet_id"], "r1");
    assert!(res_intent.params["parameter_rules"].is_array());

    picker.open_for_document("g1".into(), None, &doc);
    assert_eq!(picker.target_kind, HookTargetKind::GraphTextBox);
    let graph_intent = picker.commit_intent().expect("graph hook");
    assert_eq!(graph_intent.command_id, "graph.addAnnotation");
    assert_eq!(graph_intent.params["sheet_id"], "g1");

    picker.open_for_document("l1".into(), None, &doc);
    assert_eq!(picker.target_kind, HookTargetKind::LayoutTextCallout);
    let layout_intent = picker.commit_intent().expect("layout hook");
    assert_eq!(layout_intent.command_id, "layout.addPanel");
    assert_eq!(layout_intent.params["sheet_id"], "l1");
}

#[test]
fn assumption_banner_is_diagnostics_only_without_remediation_or_prescriptions() {
    let tg_spec = AnalysisSpec::new("res-t", "d1", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).expect("spec");
    let mut res = ResultSheet::new(tg_spec).expect("result");
    res.status = ResultStatus::UpToDate;
    res.tabs.push(curvebook_doc::ResultTab {
        id: String::from("diagnostics"),
        title: String::from("Diagnostics"),
        kind: curvebook_doc::ResultTabKind::Diagnostics,
        column_headers: vec![String::from("Check")],
        row_labels: Vec::new(),
        rows: Vec::new(),
        footnotes: Vec::new(),
    });
    res.checklist = vec![
        ChecklistItem {
            assumption: String::from("Variance homogeneity"),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some(String::from("F = 8.42, p = 0.008")),
            explanation: String::from("Sample variances differ materially across groups."),
            remediation: Some(RemediationKind::WelchT),
        },
        ChecklistItem {
            assumption: String::from("Normality"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(String::from("p = 0.03")),
            explanation: String::from("Moderate right skew observed."),
            remediation: Some(RemediationKind::MannWhitneyU),
        },
    ];
    let banner = AssumptionDiagnosticBanner::from_result(&res).expect("banner from Violated check");
    assert_eq!(banner.severity, ChecklistStatus::Violated);
    assert_eq!(banner.diagnostic_metric.as_deref(), Some("F = 8.42, p = 0.008"));
    assert!(banner.limitation_note.contains("Sample variances differ"));
    assert_eq!(banner.detail_tab_ids, vec![String::from("diagnostics")]);

    // Prescriptive phrases inside raw item explanations are neutralized (QOL-7).
    res.checklist = vec![ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(String::from("W = 0.81, p = 0.02")),
        explanation: String::from("Switch to Mann-Whitney U or transform data or exclude outlier."),
        remediation: Some(RemediationKind::MannWhitneyU),
    }];
    let neutralized = AssumptionDiagnosticBanner::from_result(&res).expect("neutralized banner");
    assert!(!neutralized.limitation_note.to_ascii_lowercase().contains("mann-whitney"));
    assert!(!neutralized.limitation_note.to_ascii_lowercase().contains("exclude outlier"));

    // Inconclusive / small-N `ChecklistStatus::Info` items also surface diagnostics-only banners,
    // whereas routine `Pass` items yield `None`.
    let anova_spec = AnalysisSpec::new("res-a", "d1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("anova spec");
    let mut anova_res = ResultSheet::new(anova_spec).expect("anova res");
    anova_res.checklist = vec![ChecklistItem {
        assumption: String::from("Normality test power"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("p = 0.42")),
        explanation: String::from("Passed Shapiro-Wilk."),
        remediation: None,
    }];
    assert!(AssumptionDiagnosticBanner::from_result(&anova_res).is_none());

    anova_res.checklist = vec![ChecklistItem {
        assumption: String::from("Normality check"),
        status: ChecklistStatus::Info,
        diagnostic_metric: Some(String::from("n = 4")),
        explanation: String::from("Sample size is too small (n < 5) for reliable normality testing."),
        remediation: None,
    }];
    let info_banner = AssumptionDiagnosticBanner::from_result(&anova_res).expect("small-N Info banner");
    assert_eq!(info_banner.severity, ChecklistStatus::Info);
    assert!(info_banner.limitation_note.contains("too small"));
}

#[test]
fn command_palette_surfaces_summary_beginner_and_session_enabled() {
    let palette = CommandPaletteState { query: String::from("analyze.run"), ..CommandPaletteState::default() };
    let session = super::Session::new();
    let rows = palette.preview_rows(Some(&session), 5);
    assert!(rows.len() >= 3);
    let hits = palette.matches_for_session(&session);
    let top = hits.first().expect("expected analyze.run match");
    assert_eq!(top.id, "analyze.run");
    assert!(!top.summary.is_empty());
    assert!(!top.beginner.is_empty());
    assert!(top.format_palette_row().contains(top.summary));
    assert!(top.format_palette_row().contains(top.beginner));
    assert!(rows.iter().any(|row| row.contains(top.summary)));
    assert!(rows.iter().any(|row| row.contains(top.beginner)));

    let axes_disabled = {
        let axes_palette = CommandPaletteState { query: String::from("graph.setAxes"), ..CommandPaletteState::default() };
        axes_palette.matches_for_session(&session)
    };
    assert!(!axes_disabled.first().expect("graph.setAxes entry").enabled);
    assert!(axes_disabled.first().expect("graph.setAxes entry").format_palette_row().contains("[disabled]"));
}

#[test]
fn w6_pcr_catalog_r10_leakage_and_mv_response_default() -> Result<(), String> {
    use super::{ANALYSIS_CATALOG, AnalyzeDialogState, default_analysis_kind_for_id};
    use curvebook_doc::{MultivariateSpec, VariableRole};

    let pcr = ANALYSIS_CATALOG.iter().find(|c| c.id == "stats.pcr").ok_or("missing stats.pcr in ANALYSIS_CATALOG")?;
    assert!(pcr.label.contains("Principal Component Regression"));
    assert!(matches!(default_analysis_kind_for_id("stats.pcr"), Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { .. }))));

    let mut mv = curvebook_doc::MultipleVariablesTable::new(5, 4).map_err(|e| e.to_string())?;
    mv.variables[0].role = VariableRole::Label;
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[3].role = VariableRole::Continuous;
    mv.variables[4].role = VariableRole::Categorical;
    for r in 0..4 {
        mv.variables[2].cells[r] = curvebook_doc::Cell::number((r + 1) as f64);
        mv.variables[3].cells[r] = curvebook_doc::Cell::number((r + 1) as f64 * 2.0);
    }
    let ds = DataSheet::new("mv-pcr", "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    let cands = collect_variable_candidates(&ds);
    assert_eq!(cands.iter().find(|c| c.is_response).map(|c| c.col_index), Some(3));

    let mut an = AnalyzeDialogState::default();
    an.open_for_data_sheet(&ds);
    an.select_catalog_entry("stats.pcr");
    assert!(an.selected_data_sets.is_empty());
    an.toggle_pca_predictor(2);
    assert!(an.select_pcr_outcome(2).is_err());
    an.select_pcr_outcome(3)?;
    assert_eq!(an.pcr_outcome_column, Some(3));
    Ok(())
}

#[test]
fn w6_heatmap_commit_intents_and_grouped_superimposed() -> Result<(), String> {
    use super::{ChangeGraphTypeState, EditAnalysisParametersState, GRAPH_TYPE_OPTIONS};
    use curvebook_doc::{DiagnosticPlotKind, GraphKind, GraphSpec, GroupedArrangement, GroupedTable, MultivariateSpec, SheetId};
    use serde_json::json;

    assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "grouped.superimposed"));
    let mut cgt = ChangeGraphTypeState { selected_option_id: String::from("grouped.superimposed"), ..ChangeGraphTypeState::default() };
    cgt.graph_sheet_id = Some(SheetId::from("g-1"));
    let grp = GroupedTable::new(curvebook_doc::SubcolumnMode::RawReplicates(2), 2, 3).map_err(|e| e.to_string())?;
    let ds_g = DataSheet::new("ds-g", "Grouped", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;
    let mut spec = GraphSpec::default_for_data_sheet(&ds_g);
    assert!(cgt.apply_to_spec(&mut spec));
    assert_eq!(spec.grouped_arrangement, GroupedArrangement::Superimposed);

    let mut ep = EditAnalysisParametersState::default();
    let analysis = AnalysisSpec::new("res-corr", "mv-1", AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }))
        .map_err(|e| e.to_string())?;
    ep.result_sheet_id = Some(SheetId::from("res-corr"));
    ep.draft = Some(analysis);
    ep.correlation_all_pairs = true;
    ep.correlation_heatmap = true;

    let primary = ep.commit_intent().ok_or("missing commit_intent")?;
    assert_eq!(primary.params["correlation_all_pairs"], json!(true));
    assert_eq!(primary.params["correlation_heatmap"], json!(true));

    let intents = ep.commit_intents(None);
    assert_eq!(intents.len(), 2);
    assert_eq!(intents[1].command_id, "graph.create");
    assert_eq!(intents[1].params["kind"], json!(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)));
    Ok(())
}

mod r13_10_format_graph_unit_tests {
    use std::collections::BTreeSet;

    use curvebook_doc::{
        CartesianOrientation, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphicalDataObject, ReplicateLineMode, SheetId, SubcolumnMode,
        TableArchetype, XyTable,
    };
    use curvebook_palette::{MarkerStyle, Rgba8};
    use serde_json::{Value, json};

    use crate::dialogs_canvas::FormatGraphState;

    #[test]
    fn orientation_dirty_gate_sync_and_commit_intents() {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
        table.data_sets[0].title = String::from("Ctrl");
        table.data_sets[1].title = String::from("Drug");
        let ds = DataSheet::new(SheetId::new("col-orient"), "Col", TableArchetype::Column(table)).unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-col"), &ds).unwrap();
        gs.spec.orientation = CartesianOrientation::Vertical;

        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert_eq!(fg.orientation, CartesianOrientation::Vertical);
        assert!(!fg.orientation_dirty);

        // Clean state must NOT emit `"orientation"` in `graph.setSpec`.
        let clean_intents = fg.commit_intents();
        let spec_intent = clean_intents.iter().find(|i| i.command_id == "graph.setSpec").unwrap();
        assert!(spec_intent.params.get("orientation").is_none());

        // Patching orientation marks dirty and emits `"orientation"` in `graph.setSpec`.
        let mut patch = serde_json::Map::new();
        patch.insert(String::from("orientation"), json!("horizontal"));
        super::super::dialog_patch::patch_format_graph(&mut fg, &patch, &crate::i18n::LocalePreset::default()).unwrap();
        assert_eq!(fg.orientation, CartesianOrientation::Horizontal);
        assert!(fg.orientation_dirty);

        let dirty_intents = fg.commit_intents();
        let dirty_spec = dirty_intents.iter().find(|i| i.command_id == "graph.setSpec").unwrap();
        assert_eq!(dirty_spec.params.get("orientation"), Some(&json!("horizontal")));

        // Re-syncing clears dirty flag.
        gs.spec.orientation = CartesianOrientation::Horizontal;
        fg.sync_from_graph(&gs);
        assert_eq!(fg.orientation, CartesianOrientation::Horizontal);
        assert!(!fg.orientation_dirty);
    }

    #[test]
    fn show_symbols_null_marker_and_replicate_lines_dirty_gate() {
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 3).unwrap();
        xy.data_sets[0].title = String::from("Dose");
        let ds = DataSheet::new(SheetId::new("xy-sym"), "XY", TableArchetype::Xy(xy)).unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy"), &ds).unwrap();
        gs.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle::default()),
            line: None,
            area_fill: None,
            replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories,
        };

        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert!(fg.show_symbols);
        assert_eq!(fg.replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
        assert!(!fg.replicate_lines_dirty);

        // When clean, `replicate_lines`, `marker`, and `connecting_line` are omitted (`AUD-09-14`).
        let clean = fg.commit_intents();
        let s_clean = clean.iter().find(|i| i.command_id == "graph.setSeries").unwrap();
        assert!(s_clean.params.get("replicate_lines").is_none());
        assert!(s_clean.params.get("marker").is_none());
        assert!(s_clean.params.get("connecting_line").is_none());

        // Hiding symbols + switching replicate line mode emits `"marker": null` & `"replicate_lines"`.
        let mut patch = serde_json::Map::new();
        patch.insert(String::from("show_symbols"), json!(false));
        patch.insert(String::from("replicate_lines"), json!("summary_center"));
        super::super::dialog_patch::patch_format_graph(&mut fg, &patch, &crate::i18n::LocalePreset::default()).unwrap();
        assert!(!fg.show_symbols);
        assert!(fg.show_symbols_dirty);
        assert_eq!(fg.replicate_lines, ReplicateLineMode::SummaryCenter);
        assert!(fg.replicate_lines_dirty);

        let dirty = fg.commit_intents();
        let s_dirty = dirty.iter().find(|i| i.command_id == "graph.setSeries").unwrap();
        assert_eq!(s_dirty.params.get("marker"), Some(&Value::Null));
        assert_eq!(s_dirty.params.get("replicate_lines"), Some(&json!(ReplicateLineMode::SummaryCenter)));

        // Syncing a series that already has `marker: None` sets `show_symbols = false`.
        gs.spec.series[0].gdo =
            GraphicalDataObject::ScatterAndLine { marker: None, line: None, area_fill: None, replicate_lines: ReplicateLineMode::SummaryCenter };
        fg.sync_from_graph(&gs);
        assert!(!fg.show_symbols);
    }

    #[test]
    fn y2_nudge_bar_border_and_forest_pooled_diamond_sync_and_commit() {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
        table.data_sets[0].title = String::from("Group A");
        table.data_sets[1].title = String::from("Group B");
        let ds = DataSheet::new(SheetId::new("col-y2"), "Col", TableArchetype::Column(table)).unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-y2"), &ds).unwrap();
        gs.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
            summary: curvebook_doc::SummaryMeasure::Mean,
            fill_color: Rgba8::rgb(40, 120, 220),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
            replicate_overlay: None,
        };
        gs.spec.series[1].gdo = GraphicalDataObject::BarWithReplicates {
            summary: curvebook_doc::SummaryMeasure::Mean,
            fill_color: Rgba8::rgb(40, 180, 90),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.5,
            replicate_overlay: None,
        };
        gs.spec.series[1].bind_to_right_y2 = true;
        gs.spec.series[1].offset.x_data = 0.35;
        gs.spec.forest_show_pooled_diamond = false;

        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert!(!fg.bind_to_right_y2);
        assert!((fg.offset_x_data - 0.0).abs() < 1e-9);
        assert!((fg.bar_border_width_pt - 1.0).abs() < 1e-9);
        assert!(!fg.forest_show_pooled_diamond);

        // Selecting series 1 syncs Y2 binding, horizontal nudge, and border width.
        fg.selected_series_index = 1;
        fg.sync_selected_series_from_spec(&gs.spec);
        assert!(fg.bind_to_right_y2);
        assert!((fg.offset_x_data - 0.35).abs() < 1e-9);
        assert!((fg.bar_border_width_pt - 1.5).abs() < 1e-9);

        let mut patch = serde_json::Map::new();
        patch.insert(String::from("apply_all_data_sets"), json!(true));
        patch.insert(String::from("bar_border_width_pt"), json!(0.5));
        patch.insert(String::from("bind_to_right_y2"), json!(false));
        patch.insert(String::from("offset_x_data"), json!(0.25));
        patch.insert(String::from("forest_show_pooled_diamond"), json!(true));
        super::super::dialog_patch::patch_format_graph(&mut fg, &patch, &crate::i18n::LocalePreset::default()).unwrap();

        let intents = fg.commit_intents();
        let spec_p = &intents.iter().find(|i| i.command_id == "graph.setSpec").unwrap().params;
        assert_eq!(spec_p.get("forest_show_pooled_diamond"), Some(&json!(true)));

        let series_p = &intents.iter().find(|i| i.command_id == "graph.setSeries").unwrap().params;
        assert_eq!(series_p.get("all_series"), Some(&json!(true)));
        assert_eq!(series_p.get("border_width_pt"), Some(&json!(0.5)));
        assert_eq!(series_p.get("bind_to_right_y2"), Some(&json!(false)));
        assert_eq!(series_p.get("offset"), Some(&json!({ "x_data": 0.25 })));

        // Verify unique widget coverage invariant on serde roundtrip.
        let roundtrip: FormatGraphState = serde_json::from_value(serde_json::to_value(&fg).unwrap()).unwrap();
        assert_eq!(roundtrip, fg);
        let _ = BTreeSet::<String>::new();
    }
}

mod aud09_remediation_tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
        JournalWidthPreset, LayoutSheet, LayoutSpec, ProjectDocument, ResultCell, ResultSheet, SheetId, SubcolumnMode, TableArchetype,
        TwoGroupMethod, TwoGroupSpec, XyTable,
    };
    use serde_json::{Value, json};

    use crate::dialogs_canvas::analysis_catalog::default_analysis_kind_for_id;
    use crate::dialogs_canvas::dialog_patch;
    use crate::dialogs_canvas::edit_params::sync_fit_parameter_inputs;
    use crate::dialogs_canvas::{
        ActiveDialog, AnalyzeDialogState, ApplyStyleFromState, BubblePlotInspectorState, CanvasDialogState, ChangeDataAnalyzedState,
        ChangeGraphTypeState, CommandPaletteState, ConstantHookPickerState, CsvExportOptionsState, DuplicateChoice, EditAnalysisParametersState,
        ExcludeWithReasonDialogState, FormatAxesState, FormatGraphState, FormatSelectedPointsState, PageLayoutComposerState,
        PairwiseComparisonsState, copy_result_table_to_clipboard,
    };

    fn map_from(val: Value) -> serde_json::Map<String, Value> {
        val.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn aud09_10_11_30_33_and_ext01_analyze_dialog_fixes() {
        // AUD-09-10: One-sample hypothetical propagation in AnalyzeDialogState + EditAnalysisParametersState.
        let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        let col_ds = DataSheet::new(SheetId::new("col-1"), "Col", TableArchetype::Column(col_tbl)).unwrap();
        let mut az = AnalyzeDialogState::default();
        az.open_for_data_sheet(&col_ds);
        az.selected_entry_id = Some(String::from("stats.two_group"));
        az.two_group_method = Some(TwoGroupMethod::OneSampleT { hypothetical: 5.0 });
        az.two_group_hypothetical = 12.5;
        assert!(matches!(
            az.build_analysis_kind(),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical }, .. }))
            if (hypothetical - 12.5).abs() < 1e-12
        ));
        // AUD-09-10: Reverting hypothetical from 5.0 back to 0.0 must take effect cleanly.
        az.two_group_method = Some(TwoGroupMethod::OneSampleT { hypothetical: 5.0 });
        az.two_group_hypothetical = 0.0;
        assert!(matches!(
            az.build_analysis_kind(),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical }, .. }))
            if hypothetical.to_bits() == 0.0_f64.to_bits()
        ));
        let preset = crate::i18n::LocalePreset::default();
        let mut ep = EditAnalysisParametersState::default();
        let one_sample_spec = AnalysisSpec::new(
            "res-os",
            "col-1",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleWilcoxon { hypothetical: 0.0 }, ..Default::default() }),
        )
        .unwrap();
        ep.open_for_result(&ResultSheet::new(one_sample_spec).unwrap());
        dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "hypothetical": 4.25 })), &preset).unwrap();
        assert!(matches!(
            ep.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleWilcoxon { hypothetical }, .. }))
            if (*hypothetical - 4.25).abs() < 1e-12
        ));
        // Reject hypothetical on unpaired two-group test.
        let unpaired_spec = AnalysisSpec::new("res-ut", "col-1", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
        ep.open_for_result(&ResultSheet::new(unpaired_spec).unwrap());
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "hypothetical": 1.0 })), &preset).is_err());

        // AUD-09-11: GLM honors `is_response` column (last numeric column) rather than first numeric column.
        let mut mv = curvebook_doc::MultipleVariablesTable::new(3, 3).unwrap();
        for r in 0..3 {
            mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
            mv.variables[1].cells[r] = Cell::number((r + 2) as f64);
            mv.variables[2].cells[r] = Cell::number((r + 10) as f64);
        }
        let mv_ds = DataSheet::new(SheetId::new("mv-glm"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();
        az.open_for_data_sheet(&mv_ds);
        az.selected_entry_id = Some(String::from("stats.glm"));
        assert!(az.explicit_response_column.is_none());
        let Some(AnalysisKind::GeneralizedLinearModel(glm)) = az.build_analysis_kind() else {
            panic!("expected GLM");
        };
        assert_eq!(glm.response_column, 2);
        assert_eq!(glm.predictor_columns, vec![0, 1]);

        // AUD-09-30: invalid/incomplete AnalyzeDialogState never fabricates `analyze.run {}` or closes dialog.
        let doc = ProjectDocument::new("Analyze Test").unwrap();
        let mut cd = CanvasDialogState {
            active_dialog: ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("col-1") },
            ..CanvasDialogState::default()
        };
        cd.analyze.source_sheet_id = Some(SheetId::new("col-1"));
        cd.analyze.selected_entry_id = Some(String::from("nonexistent.entry"));
        assert_eq!(cd.analyze.try_build_run_intent(&doc), Err(Vec::new()));
        assert!(cd.submit_analyze_or_prompt(&doc).is_none());
        assert!(matches!(cd.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
        assert!(dialog_patch::patch_analyze(&mut az, &map_from(json!({ "entry_id": "bogus.analysis" })), &preset).is_err());
    }

    #[test]
    fn aud09_33_and_ext01_analyze_dialog_fixes() {
        let preset = crate::i18n::LocalePreset::default();
        let mut mv = curvebook_doc::MultipleVariablesTable::new(3, 3).unwrap();
        for r in 0..3 {
            mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
            mv.variables[1].cells[r] = Cell::number((r + 2) as f64);
            mv.variables[2].cells[r] = Cell::number((r + 10) as f64);
        }
        let mv_ds = DataSheet::new(SheetId::new("mv-glm"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();
        let mut az = AnalyzeDialogState::default();

        // AUD-09-33: Duplicate detection distinguishes inner variants of Transform, Multivariate, Survival, and OutlierDetection.
        let mut dup_doc = ProjectDocument::new("Dup Test").unwrap();
        dup_doc.data_sheets.push(mv_ds.clone());
        let pca_spec = AnalysisSpec::new("res-pca", "mv-glm", default_analysis_kind_for_id("stats.pca").unwrap()).unwrap();
        dup_doc.result_sheets.push(ResultSheet::new(pca_spec).unwrap());
        let math_spec = AnalysisSpec::new("res-math", "mv-glm", default_analysis_kind_for_id("transform.math").unwrap()).unwrap();
        dup_doc.result_sheets.push(ResultSheet::new(math_spec).unwrap());
        let km_spec = AnalysisSpec::new("res-km", "mv-glm", default_analysis_kind_for_id("stats.survival_km").unwrap()).unwrap();
        dup_doc.result_sheets.push(ResultSheet::new(km_spec).unwrap());
        let rout_spec =
            AnalysisSpec::new("res-rout", "mv-glm", AnalysisKind::OutlierDetection(curvebook_doc::OutlierSpec::Rout { fdr_q_percent: 1.0 })).unwrap();
        dup_doc.result_sheets.push(ResultSheet::new(rout_spec).unwrap());
        az.open_for_data_sheet(&mv_ds);
        az.selected_entry_id = Some(String::from("stats.corr_matrix"));
        assert!(az.matching_existing_analyses(&dup_doc).is_empty());
        az.selected_entry_id = Some(String::from("stats.pca"));
        assert_eq!(az.matching_existing_analyses(&dup_doc), vec![SheetId::new("res-pca")]);
        az.selected_entry_id = Some(String::from("transform.normalize"));
        assert!(az.matching_existing_analyses(&dup_doc).is_empty());
        az.selected_entry_id = Some(String::from("stats.survival_cox"));
        assert!(az.matching_existing_analyses(&dup_doc).is_empty());
        az.selected_entry_id = Some(String::from("stats.outliers"));
        az.outlier_method = String::from("grubbs");
        assert!(az.matching_existing_analyses(&dup_doc).is_empty(), "Grubbs must not collide with existing ROUT");
        az.outlier_method = String::from("rout");
        assert_eq!(az.matching_existing_analyses(&dup_doc), vec![SheetId::new("res-rout")]);

        // EXT-01: Dose-response default auto-selects non-log equation for raw positive concentrations and resets across sheets.
        let mut raw_xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
        for (r, &x) in [1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4].iter().enumerate() {
            raw_xy.x_cells[r][0] = Cell::number(x);
        }
        let raw_ds = DataSheet::new(SheetId::new("xy-raw"), "Raw Dose", TableArchetype::Xy(raw_xy)).unwrap();
        az.open_for_data_sheet(&raw_ds);
        assert_eq!(az.equation_id, "agonist_vs_response_4pl");
        az.selected_entry_id = Some(String::from("fit.nonlinear"));
        assert!(matches!(az.build_analysis_kind(), Some(AnalysisKind::NonlinearRegression(nl)) if nl.equation_id == "agonist_vs_response_4pl"));

        let mut log_xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
        for (r, &x) in [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0].iter().enumerate() {
            log_xy.x_cells[r][0] = Cell::number(x);
        }
        let log_ds = DataSheet::new(SheetId::new("xy-log"), "Log Dose", TableArchetype::Xy(log_xy)).unwrap();
        az.open_for_data_sheet(&log_ds);
        assert_eq!(az.equation_id, "log_agonist_vs_response_4pl");
        dialog_patch::patch_analyze(&mut az, &map_from(json!({ "fit_equation_id": "michaelis_menten" })), &preset).unwrap();
        assert_eq!(az.equation_id, "michaelis_menten");
    }

    #[test]
    fn aud09_12_graph_ranks_and_backed_diag_toggles() {
        let mut doc = ProjectDocument::new("Ranks").unwrap();
        let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        let col_ds = DataSheet::new(SheetId::new("col-mw"), "MW Data", TableArchetype::Column(col_tbl)).unwrap();
        doc.data_sheets.push(col_ds);

        let mw_spec = AnalysisSpec::new(
            "res-mw",
            "col-mw",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..Default::default() }),
        )
        .unwrap();
        let mut ep = EditAnalysisParametersState::default();
        ep.open_for_result(&ResultSheet::new(mw_spec).unwrap());
        ep.graph_ranks = true;
        let intents = ep.commit_intents(Some(&doc));
        assert!(intents.iter().any(|i| i.command_id == "graph.create" && i.params["sheet_id"] == json!("col-mw_ranks")));
        assert!(intents.iter().any(|i| i.command_id == "graph.setKind" && i.params["sheet_id"] == json!("col-mw_ranks")));

        // NonlinearRegression backed diag toggles propagate when toggled (including normality_of_residuals <-> diag_qq_plot).
        let mut nl_spec = curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        nl_spec.normality_of_residuals = false;
        nl_spec.emit_residuals_tab = true;
        nl_spec.homoscedasticity_test = false;
        let aspec = AnalysisSpec::new("res-nl", "col-mw", AnalysisKind::NonlinearRegression(nl_spec)).unwrap();
        ep.open_for_result(&ResultSheet::new(aspec).unwrap());
        assert!(!ep.diag_qq_plot);
        ep.diag_qq_plot = true;
        ep.diag_residuals_plot = false;
        ep.diag_homoscedasticity = true;
        let nl_intent = ep.commit_intent().unwrap();
        assert_eq!(nl_intent.params["kind"]["NonlinearRegression"]["normality_of_residuals"], json!(true));
        assert_eq!(nl_intent.params["kind"]["NonlinearRegression"]["emit_residuals_tab"], json!(false));
        assert_eq!(nl_intent.params["kind"]["NonlinearRegression"]["homoscedasticity_test"], json!(true));

        // AUD-09-12: Unknown CI methods fail patch validation; engine-unsupported non-default
        // CI methods ("Woolf"/"Katz"/"Wald"/"Clopper-Pearson") surface `unsupported_contingency_ci_error`
        // and refuse to silently commit engine defaults (while preserving `fa_sec13_11` / `r10_28_wf19` cycling).
        let preset = crate::i18n::LocalePreset::default();
        let cont_spec = AnalysisSpec::new("res-ct", "col-mw", AnalysisKind::Contingency(curvebook_doc::ContingencySpec::default())).unwrap();
        ep.open_for_result(&ResultSheet::new(cont_spec).unwrap());
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "edit.contingency.ci.baptista_pike": "BogusCI" })), &preset).is_err());
        assert!(ep.unsupported_contingency_ci_error().is_none());
        assert!(ep.commit_intent().is_some());
        for (key, unsup_val, ok_val) in [
            ("edit.contingency.ci.baptista_pike", "Woolf", "Baptista-Pike"),
            ("edit.contingency.ci.koopman", "Katz", "Koopman"),
            ("edit.contingency.ci.newcombe_wilson", "Wald", "Newcombe-Wilson"),
            ("edit.contingency.ci.wilson_brown", "Clopper-Pearson", "Wilson-Brown"),
        ] {
            assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ key: unsup_val })), &preset).is_ok());
            assert!(ep.unsupported_contingency_ci_error().is_some(), "{key}={unsup_val} must report unsupported CI error");
            assert!(ep.commit_intent().is_none(), "{key}={unsup_val} must block silent default commit");
            assert!(ep.commit_intents(None).is_empty(), "{key}={unsup_val} must block commit_intents");
            assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ key: ok_val })), &preset).is_ok());
            assert!(ep.unsupported_contingency_ci_error().is_none(), "{key}={ok_val} must clear error");
            assert!(ep.commit_intent().is_some(), "{key}={ok_val} must allow commit");
        }
    }

    #[test]
    fn aud09_13_25_37_49_results_and_hooks_fixes() {
        // AUD-09-13: analyzable_data-only ResultSheet exports >200 rows, X column, and all replicate subcolumns.
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 205).unwrap();
        xy.x_title = String::from("Conc");
        xy.data_sets[0].title = String::from("Resp");
        for r in 0..205 {
            xy.x_cells[r][0] = Cell::number((r + 1) as f64);
            xy.data_sets[0].cells[r][0] = Cell::number((r * 2) as f64);
            xy.data_sets[0].cells[r][1] = Cell::number((r * 2 + 1) as f64);
        }
        let derived_ds = DataSheet::new(SheetId::new("xy-derived"), "Derived", TableArchetype::Xy(xy)).unwrap();
        let mut rs =
            ResultSheet::new(AnalysisSpec::new("res-tx", "xy-derived", default_analysis_kind_for_id("transform.math").unwrap()).unwrap()).unwrap();
        rs.analyzable_data = Some(derived_ds);
        let mut doc = ProjectDocument::new("Copy Full").unwrap();
        doc.result_sheets.push(rs);
        let mut app = crate::UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let out = copy_result_table_to_clipboard(&mut app, "res-tx", true);
        let tsv = out.command_result.unwrap()["copied_text"].as_str().unwrap().to_owned();
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 206, "header + all 205 rows without 200-row cap");
        assert!(lines[0].contains("Conc") && lines[0].contains("Resp:1") && lines[0].contains("Resp:2"));
        assert!(lines[205].contains("205") && lines[205].contains("408") && lines[205].contains("409"));

        // AUD-09-25 + AUD-09-49(3): tab switch and sheet switch clear stale selection and last_methods_report.
        let mut rs_multi =
            ResultSheet::new(AnalysisSpec::new("res-multi", "xy-derived", default_analysis_kind_for_id("stats.descriptive").unwrap()).unwrap())
                .unwrap();
        rs_multi.tabs = vec![
            curvebook_doc::ResultTab {
                id: String::from("t0"),
                title: String::from("Tab0"),
                kind: curvebook_doc::ResultTabKind::MainSummary,
                column_headers: vec![String::from("Val")],
                row_labels: vec![String::from("R1")],
                rows: vec![vec![ResultCell::Text(String::from("TAB_ZERO"))]],
                footnotes: Vec::new(),
            },
            curvebook_doc::ResultTab {
                id: String::from("t1"),
                title: String::from("Tab1"),
                kind: curvebook_doc::ResultTabKind::Diagnostics,
                column_headers: vec![String::from("Val")],
                row_labels: vec![String::from("R1")],
                rows: vec![vec![ResultCell::Text(String::from("TAB_ONE"))]],
                footnotes: Vec::new(),
            },
        ];
        let mut doc2 = ProjectDocument::new("Tab Copy").unwrap();
        doc2.result_sheets.push(rs_multi.clone());
        let mut app2 = crate::UiApp::from_session(curvebook_engine::Session::from_document(doc2));
        app2.canvas_dialogs.result_view.sync_with_result(&rs_multi);
        app2.canvas_dialogs.result_view.last_methods_report = Some(String::from("stale report"));
        app2.canvas_dialogs.result_view.bound_sheet_id = Some(String::from("res-multi"));
        app2.canvas_dialogs.result_view.bound_tab_idx = Some(0);
        app2.canvas_dialogs.result_view.selected_anchor = Some((0, 0));
        app2.canvas_dialogs.result_view.select_tab(1, 2);
        assert_eq!(app2.canvas_dialogs.result_view.bound_tab_idx, None);
        assert_eq!(app2.canvas_dialogs.result_view.selected_anchor, None);
        let copy_t1 = copy_result_table_to_clipboard(&mut app2, "res-multi", false);
        assert!(copy_t1.command_result.unwrap()["copied_text"].as_str().unwrap().contains("TAB_ONE"));

        let rs_other =
            ResultSheet::new(AnalysisSpec::new("res-other", "xy-derived", default_analysis_kind_for_id("stats.descriptive").unwrap()).unwrap())
                .unwrap();
        app2.canvas_dialogs.result_view.sync_with_result(&rs_other);
        assert_eq!(app2.canvas_dialogs.result_view.last_methods_report, None);

        // AUD-09-37: ConstantHookPickerState supports NonlinearRegression + Normalize, rejects unsupported ResultSheet kinds.
        let mut hdoc = ProjectDocument::new("Hooks").unwrap();
        let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Info").unwrap();
        info.constants.push(InfoConstant::new("Ctrl", ConstantValue::Number(50.0), None).unwrap());
        hdoc.info_sheets.push(info);
        let lr_rs = ResultSheet::new(
            AnalysisSpec::new("r-lr", "xy-derived", AnalysisKind::LinearRegression(curvebook_doc::LinearRegressionSpec::default())).unwrap(),
        )
        .unwrap();
        let norm_rs =
            ResultSheet::new(AnalysisSpec::new("r-norm", "xy-derived", default_analysis_kind_for_id("transform.normalize").unwrap()).unwrap())
                .unwrap();
        hdoc.result_sheets.push(lr_rs);
        hdoc.result_sheets.push(norm_rs);
        let mut picker = ConstantHookPickerState::default();
        picker.open_for_document(SheetId::new("r-lr"), Some(String::from("Slope")), &hdoc);
        assert!(picker.commit_intent().is_none(), "LinearRegression does not accept parameter_rules hooks");
        picker.open_for_document(SheetId::new("r-norm"), Some(String::from("zero_anchor")), &hdoc);
        let norm_intent = picker.commit_intent().expect("Normalize accepts zero_anchor hook");
        assert!(norm_intent.params.get("zero_anchor").is_some());

        // AUD-09-49(2) & (4): ChangeDataAnalyzed clears selected_data_sets on target switch; linear range rejects start == end.
        let mut rdoc = ProjectDocument::new("Retarget").unwrap();
        let c1 =
            DataSheet::new(SheetId::new("c1"), "C1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 3).unwrap()))
                .unwrap();
        let c2 =
            DataSheet::new(SheetId::new("c2"), "C2", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()))
                .unwrap();
        rdoc.data_sheets.push(c1);
        rdoc.data_sheets.push(c2);
        let mut aspec = AnalysisSpec::new("r1", "c1", default_analysis_kind_for_id("stats.descriptive").unwrap()).unwrap();
        aspec.selected_data_sets = vec![0, 1, 2, 3];
        let mut cda = ChangeDataAnalyzedState::default();
        cda.open_for_result(&ResultSheet::new(aspec).unwrap(), &rdoc);
        assert_eq!(cda.selected_data_sets, vec![0, 1]);
        cda.select_target(&SheetId::new("c1")).unwrap();
        assert!(cda.selected_data_sets.is_empty());

        let mut lr_app = crate::UiApp::new();
        let lr_spec = AnalysisSpec::new("r-lr", "c1", AnalysisKind::LinearRegression(curvebook_doc::LinearRegressionSpec::default())).unwrap();
        lr_app.canvas_dialogs.edit_params.open_for_result(&ResultSheet::new(lr_spec).unwrap());
        lr_app
            .interaction
            .text_field_buffers
            .insert(String::from("fit.lr.range.input.plot_min"), crate::interaction::TextFieldEditState::new("5", false));
        lr_app
            .interaction
            .text_field_buffers
            .insert(String::from("fit.lr.range.input.plot_max"), crate::interaction::TextFieldEditState::new("5", false));
        assert!(sync_fit_parameter_inputs(&mut lr_app).is_err());
    }

    #[test]
    fn aud09_14_15_16_29_32_34_38_47_and_aud13_07_graph_layout_fixes() {
        // AUD-09-14 + AUD-09-29: FormatGraph omits marker/connecting_line when clean, skips setSeries on empty series,
        // and emits per-series swatches when applying GDO replacement to all data sets.
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
        col.data_sets[0].title = String::from("A");
        col.data_sets[1].title = String::from("B");
        let ds = DataSheet::new(SheetId::new("col-fg"), "Col", TableArchetype::Column(col)).unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-fg"), &ds).unwrap();
        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        fg.apply_all_data_sets = true;
        fg.series_gdo_bar = true;
        fg.series_gdo_dirty = true;
        let all_intents = fg.commit_intents();
        let series_intents: Vec<_> = all_intents.iter().filter(|i| i.command_id == "graph.setSeries").collect();
        assert_eq!(series_intents.len(), 2);
        assert_ne!(series_intents[0].params["gdo"], series_intents[1].params["gdo"]);

        // Empty series skips graph.setSeries and resets stale fields (AUD-09-29 & AUD-09-34).
        gs.spec.series.clear();
        fg.mv_selected_level = Some(String::from("StaleLevel"));
        fg.sync_from_graph(&gs);
        assert_eq!(fg.mv_selected_level, None);
        assert!(!fg.error_bar_enabled);
        assert!(!fg.commit_intents().iter().any(|i| i.command_id == "graph.setSeries"));

        // AUD-09-15: PageLayoutComposerState preserves custom page dimensions & transparent background_fill.
        let mut layout = LayoutSheet::new(SheetId::new("lay-1"), "Custom", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).unwrap();
        layout.spec.width = 123.0;
        layout.spec.height = 97.0;
        layout.spec.unit = curvebook_doc::PhysicalUnit::Millimeters;
        layout.spec.background_fill = None;
        let mut plc = PageLayoutComposerState::default();
        plc.sync_from_layout(&layout);
        let materialized = plc.to_layout_spec();
        assert!((materialized.width - 123.0).abs() < 1e-9);
        assert!((materialized.height - 97.0).abs() < 1e-9);
        assert_eq!(materialized.background_fill, None);
        plc.apply_preset(JournalWidthPreset::DoubleColumn174Mm);
        let switched = plc.to_layout_spec();
        assert!((switched.width - 174.0).abs() < 1e-9);
        assert_eq!(switched.background_fill, None);

        // AUD-09-32: CsvExportOptionsState pairs comma decimal separator with semicolon delimiter.
        let mut csv_opts = CsvExportOptionsState::default();
        csv_opts.reset_for_locale(',');
        assert_eq!((csv_opts.delimiter, csv_opts.decimal_separator), (';', ','));

        // AUD-09-16 + AUD-09-34(1): FormatSelectedPointsState and ExcludeWithReasonDialogState reset stale fields on reopen.
        let mut fsp = FormatSelectedPointsState {
            color: Some(String::from("#ff0000")),
            size_pt: Some(12.0),
            show_row_labels: true,
            ..FormatSelectedPointsState::default()
        };
        fsp.open_for_graph(SheetId::new("g-2"), vec![(0, 1)]);
        assert_eq!(fsp.color, None);
        assert_eq!(fsp.size_pt, None);
        assert!(!fsp.show_row_labels);

        let mut excl = ExcludeWithReasonDialogState {
            reason_tag: String::from("Bubble"),
            note: String::from("old note"),
            timestamp_utc: Some(String::from("2026-01-01T00:00:00Z")),
            ..ExcludeWithReasonDialogState::default()
        };
        excl.open_for_coords(SheetId::new("d-2"), vec![]);
        assert_eq!(excl.reason_tag, "Outlier");
        assert!(excl.note.is_empty());
        assert_eq!(excl.timestamp_utc, None);

        // AUD-09-34(2): PairwiseComparisonsState syncs bracket style/label/hide flags from graph.
        let mut doc = ProjectDocument::new("Overlays").unwrap();
        let mut pw = PairwiseComparisonsState {
            bracket_style: curvebook_doc::BracketStyle::CurlyBrace,
            hide_non_significant: false,
            ..PairwiseComparisonsState::default()
        };
        pw.sync_from_graph(&gs, &doc);
        assert_eq!(pw.bracket_style, curvebook_doc::BracketStyle::Square);
        assert!(pw.hide_non_significant);

        // AUD-09-34(4) & AUD-13-07: BubblePlotInspectorState resolves ResultSheet analyzable_data and resets when multivariate is None.
        let mv = curvebook_doc::MultipleVariablesTable::new(3, 2).unwrap();
        let mut rs_derived =
            ResultSheet::new(AnalysisSpec::new("res-mv", "missing-src", default_analysis_kind_for_id("transform.math").unwrap()).unwrap()).unwrap();
        rs_derived.analyzable_data = Some(DataSheet::new(SheetId::new("res-mv-data"), "Derived MV", TableArchetype::MultipleVariables(mv)).unwrap());
        doc.result_sheets.push(rs_derived);
        let mut g_mv = gs.clone();
        g_mv.spec.primary_source_sheet_id = SheetId::new("res-mv");
        g_mv.spec.multivariate = None;
        let mut bp = BubblePlotInspectorState { x_column: 2, show_convex_hulls: true, ..BubblePlotInspectorState::default() };
        bp.sync_from_graph(&g_mv, &doc);
        assert_eq!(bp.available_columns.len(), 3);
        assert_eq!(bp.x_column, 0);
        assert!(!bp.show_convex_hulls);

        // AUD-09-38: graph.applyCanvasPreset targets active_graph even when a LayoutSheet is active.
        let mut cdoc = ProjectDocument::new("Palette").unwrap();
        cdoc.data_sheets.push(ds.clone());
        let g_sheet = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds).unwrap();
        cdoc.graph_sheets.push(g_sheet);
        cdoc.layout_sheets.push(layout);
        cdoc.active_sheet_id = Some(SheetId::new("lay-1"));
        let sess = curvebook_engine::Session::from_document(cdoc);
        let cp = CommandPaletteState { query: String::from("graph.applyCanvasPreset"), ..Default::default() };
        let intent = cp.activate_selected_for_session(Some(&sess)).expect("graph.applyCanvasPreset intent");
        assert_eq!(intent.command_id, "graph.applyCanvasPreset");
        assert_eq!(intent.params["sheet_id"], json!("g-1"));

        // AUD-09-47: ChangeGraphTypeState grouped.superimposed emits GroupedArrangement::Superimposed in commit_intent & commit_intents.
        let cgt = ChangeGraphTypeState {
            graph_sheet_id: Some(SheetId::new("g-missing")),
            selected_option_id: String::from("grouped.superimposed"),
            ..ChangeGraphTypeState::default()
        };
        let single = cgt.commit_intent().unwrap();
        assert_eq!(single.command_id, "graph.setSpec");
        assert_eq!(single.params["grouped_arrangement"], json!(curvebook_doc::GroupedArrangement::Superimposed));
        let fallback_many = cgt.commit_intents(&doc);
        assert!(
            fallback_many
                .iter()
                .any(|i| i.command_id == "graph.setSpec" && i.params["grouped_arrangement"] == json!(curvebook_doc::GroupedArrangement::Superimposed))
        );
    }

    #[test]
    fn aud09_31_35_dialog_patch_analysis_fixes() {
        let preset = crate::i18n::LocalePreset::default();

        // AUD-09-31: pre-summarized guard blocks paired_sign_test, mw, anova_method, and Grouped repeated_measures;
        // paired_sign_test: false does not mutate UnpairedT into WilcoxonSignedRank.
        let mut doc = ProjectDocument::new("PreSum").unwrap();
        let col = ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 1).unwrap();
        doc.data_sheets.push(DataSheet::new(SheetId::new("col-sum"), "Sum", TableArchetype::Column(col)).unwrap());
        let grp = curvebook_doc::GroupedTable::new(SubcolumnMode::MeanSdN, 2, 2).unwrap();
        doc.data_sheets.push(DataSheet::new(SheetId::new("grp-sum"), "GrpSum", TableArchetype::Grouped(grp)).unwrap());
        let sid_col = SheetId::new("col-sum");
        let sid_grp = SheetId::new("grp-sum");
        assert!(dialog_patch::validate_pre_summarized_patch(&doc, Some(&sid_col), &map_from(json!({ "paired_sign_test": true }))).is_err());
        assert!(dialog_patch::validate_pre_summarized_patch(&doc, Some(&sid_col), &map_from(json!({ "two_group_method": "mw" }))).is_err());
        assert!(dialog_patch::validate_pre_summarized_patch(&doc, Some(&sid_col), &map_from(json!({ "anova_method": "kruskal" }))).is_err());
        assert!(dialog_patch::validate_pre_summarized_patch(&doc, Some(&sid_grp), &map_from(json!({ "repeated_measures": "row" }))).is_err());

        let mut ep = EditAnalysisParametersState::default();
        ep.open_for_result(
            &ResultSheet::new(AnalysisSpec::new("r-ut", "col-sum", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap()).unwrap(),
        );
        dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "paired_sign_test": false })), &preset).unwrap();
        assert!(matches!(
            ep.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, .. }))
        ));

        // AUD-09-35: Nonlinear ROUT Q capped at 50%, R18.2 fields + Linear/SimpleLogistic/ROC/MultipleRow/BlandAltman/PassingBablok/SplineLowess.
        ep.open_for_result(
            &ResultSheet::new(
                AnalysisSpec::new(
                    "r-nl",
                    "col-sum",
                    AnalysisKind::NonlinearRegression(curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "fit_method": "rout:60" })), &preset).is_err());
        dialog_patch::patch_edit_params(
            &mut ep,
            &map_from(json!({
                "fit_method": "rout:10",
                "identifiability_mode": "IdentifyAmbiguousFits",
                "output_significant_digits": 5,
                "prefer_well_defined_fit": true,
                "plot_x_min": -10.0,
                "plot_x_max": -2.0
            })),
            &preset,
        )
        .unwrap();
        assert!(matches!(
            ep.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::NonlinearRegression(nl)) if nl.output_significant_digits == Some(5) && nl.prefer_well_defined_fit
        ));
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "plot_x_min": 5.0, "plot_x_max": 5.0 })), &preset).is_err());

        ep.open_for_result(
            &ResultSheet::new(
                AnalysisSpec::new("r-lr", "col-sum", AnalysisKind::LinearRegression(curvebook_doc::LinearRegressionSpec::default())).unwrap(),
            )
            .unwrap(),
        );
        dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "force_through_origin": true, "plot_segments": 250 })), &preset).unwrap();
        assert!(matches!(
            ep.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::LinearRegression(lr)) if lr.force_through_origin && lr.plot_segments == Some(250)
        ));

        ep.open_for_result(
            &ResultSheet::new(
                AnalysisSpec::new("r-sp", "col-sum", AnalysisKind::FitSplineLowess(curvebook_doc::SplineLowessSpec::default())).unwrap(),
            )
            .unwrap(),
        );
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "interpolate_unknowns": "yes" })), &preset).is_err());
        dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "plot_x_min": 0.0, "plot_x_max": 10.0 })), &preset).unwrap();
    }

    #[test]
    fn aud09_36_48_and_aud13_11_dialog_patch_fixes() {
        let preset = crate::i18n::LocalePreset::default();

        // AUD-09-36: transform formula propagation, force_create_new_once preservation, custom_dsl non-string rejection,
        // wrong-variant Transform keys rejected, non-boolean widget keys rejected, bubble_plot & format_axes fixes.
        let mut az = AnalyzeDialogState {
            selected_entry_id: Some(String::from("transform.math")),
            force_create_new_once: true,
            ..AnalyzeDialogState::default()
        };
        assert!(dialog_patch::patch_analyze(&mut az, &map_from(json!({ "formula": "bogus_fn(y)" })), &preset).is_err());
        dialog_patch::patch_analyze(&mut az, &map_from(json!({ "search": "log" })), &preset).unwrap();
        assert!(az.force_create_new_once);
        assert!(dialog_patch::patch_analyze(&mut az, &map_from(json!({ "custom_dsl": 42 })), &preset).is_err());

        let mut ep = EditAnalysisParametersState::default();
        ep.open_for_result(
            &ResultSheet::new(AnalysisSpec::new("r-tx", "col-sum", default_analysis_kind_for_id("transform.normalize").unwrap()).unwrap()).unwrap(),
        );
        assert!(dialog_patch::patch_edit_params(&mut ep, &map_from(json!({ "x_transform": "log10" })), &preset).is_err());

        let mut bp = BubblePlotInspectorState::default();
        bp.set_color_continuous(1, curvebook_palette::ColormapId::Viridis, None);
        dialog_patch::patch_bubble_plot(&mut bp, &map_from(json!({ "color_column": 2 }))).unwrap();
        assert!(matches!(bp.color_channel, curvebook_doc::ColorChannelMapping::Continuous { column: 2, .. }));
        dialog_patch::patch_bubble_plot(&mut bp, &map_from(json!({ "color_column": null }))).unwrap();
        assert!(matches!(bp.color_channel, curvebook_doc::ColorChannelMapping::Constant(_)));

        let mut fa = FormatAxesState::default();
        assert!(dialog_patch::patch_format_axes(&mut fa, &map_from(json!({ "remove_segment": 0 })), None, &preset).is_err());

        // AUD-09-48: point style, simulate distributions domain, apply_style SheetId, randomize/sample info_sheet_id.
        let mut fsp = FormatSelectedPointsState::default();
        assert!(dialog_patch::patch_format_selected_points(&mut fsp, &map_from(json!({ "size_pt": -1.0 })), &preset).is_err());
        assert!(dialog_patch::patch_format_selected_points(&mut fsp, &map_from(json!({ "color": "not-a-color" })), &preset).is_err());
        assert!(dialog_patch::patch_format_selected_points(&mut fsp, &map_from(json!({ "symbol": "pentagram" })), &preset).is_err());

        let mut sim = crate::dialogs_r9::SimulateDistributionsDialogState::default();
        assert!(dialog_patch::patch_simulate_distributions(&mut sim, &map_from(json!({ "sd": -1.0 })), &preset).is_err());
        assert!(
            dialog_patch::patch_simulate_distributions(&mut sim, &map_from(json!({ "distribution": "uniform", "min": 5.0, "max": 2.0 })), &preset)
                .is_err()
        );
        assert!(dialog_patch::patch_simulate_distributions(&mut sim, &map_from(json!({ "distribution": "binomial", "p": 2.0 })), &preset).is_err());
        assert!(
            dialog_patch::patch_simulate_distributions(&mut sim, &map_from(json!({ "distribution": "exponential", "rate": 0.0 })), &preset).is_err()
        );

        let mut asf = ApplyStyleFromState::default();
        assert!(dialog_patch::patch_apply_style(&mut asf, &map_from(json!({ "donor_graph_id": "" }))).is_err());

        let mut rg = crate::dialogs_r9::RandomizeGroupsDialogState::default();
        assert!(dialog_patch::patch_randomize_groups(&mut rg, &map_from(json!({ "info_sheet_id": 99 }))).is_err());
        let mut rsamp = crate::dialogs_r9::RandomSampleRowsDialogState::default();
        assert!(dialog_patch::patch_random_sample_rows(&mut rsamp, &map_from(json!({ "source_sheet_id": "" }))).is_err());

        // AUD-13-11: DuplicateAnalysisPrompt choice aliases + target_id validation.
        let mut app = crate::UiApp::new();
        app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: SheetId::new("d1"),
            existing_result_ids: vec![SheetId::new("r1"), SheetId::new("r2")],
            pending_entry_id: String::from("stats.descriptive"),
        };
        dialog_patch::patch_active_dialog(&mut app, &map_from(json!({ "duplicate_choice": "modify", "target_id": "r2" }))).unwrap();
        assert_eq!(app.canvas_dialogs.analyze.duplicate_choice, DuplicateChoice::ModifyExisting);
        assert_eq!(app.canvas_dialogs.analyze.duplicate_target_id.as_deref(), Some("r2"));
        assert!(dialog_patch::patch_active_dialog(&mut app, &map_from(json!({ "target_id": "missing" }))).is_err());
    }
}
