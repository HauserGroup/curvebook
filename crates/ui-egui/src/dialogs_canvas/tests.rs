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
    assert!(ci.contains("unbounded"));

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
    use curvebook_doc::{
        CartesianOrientation, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphicalDataObject, ReplicateLineMode, SheetId, SubcolumnMode,
        TableArchetype, XyTable,
    };
    use curvebook_palette::{MarkerStyle, Rgba8};
    use serde_json::{Value, json};

    use crate::dialogs_canvas::FormatGraphState;

    #[test]
    fn orientation_dirty_gate_sync_and_commit_intents() {
        let ds = DataSheet::new(
            SheetId::new("col-orient"),
            "Col",
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()),
        )
        .unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-col"), &ds).unwrap();
        gs.spec.orientation = CartesianOrientation::Vertical;
        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert!(fg.orientation == CartesianOrientation::Vertical && !fg.orientation_dirty);
        assert!(fg.commit_intents().iter().find(|i| i.command_id == "graph.setSpec").unwrap().params.get("orientation").is_none());

        (fg.orientation, fg.orientation_dirty) = (CartesianOrientation::Horizontal, true);
        assert_eq!(
            fg.commit_intents().iter().find(|i| i.command_id == "graph.setSpec").unwrap().params.get("orientation"),
            Some(&json!("horizontal"))
        );
        gs.spec.orientation = CartesianOrientation::Horizontal;
        fg.sync_from_graph(&gs);
        assert!(fg.orientation == CartesianOrientation::Horizontal && !fg.orientation_dirty);
    }

    #[test]
    fn show_symbols_null_marker_and_replicate_lines_dirty_gate() {
        let ds =
            DataSheet::new(SheetId::new("xy-sym"), "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 3).unwrap())).unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy"), &ds).unwrap();
        gs.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle::default()),
            line: None,
            area_fill: None,
            replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories,
        };
        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert!(fg.show_symbols && fg.replicate_lines == ReplicateLineMode::IndividualReplicateTrajectories && !fg.replicate_lines_dirty);
        let clean = fg.commit_intents();
        let s_clean = clean.iter().find(|i| i.command_id == "graph.setSeries").unwrap();
        assert!(
            s_clean.params.get("replicate_lines").is_none()
                && s_clean.params.get("marker").is_none()
                && s_clean.params.get("connecting_line").is_none()
        );

        (fg.show_symbols, fg.show_symbols_dirty, fg.replicate_lines, fg.replicate_lines_dirty) =
            (false, true, ReplicateLineMode::SummaryCenter, true);
        let dirty = fg.commit_intents();
        let s_dirty = dirty.iter().find(|i| i.command_id == "graph.setSeries").unwrap();
        assert_eq!(
            (s_dirty.params.get("marker"), s_dirty.params.get("replicate_lines")),
            (Some(&Value::Null), Some(&json!(ReplicateLineMode::SummaryCenter)))
        );
        gs.spec.series[0].gdo =
            GraphicalDataObject::ScatterAndLine { marker: None, line: None, area_fill: None, replicate_lines: ReplicateLineMode::SummaryCenter };
        fg.sync_from_graph(&gs);
        assert!(!fg.show_symbols);
    }

    #[test]
    fn y2_nudge_bar_border_and_forest_pooled_diamond_sync_and_commit() {
        let ds = DataSheet::new(
            SheetId::new("col-y2"),
            "Col",
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()),
        )
        .unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-y2"), &ds).unwrap();
        for (idx, bw) in [(0, 1.0), (1, 1.5)] {
            gs.spec.series[idx].gdo = GraphicalDataObject::BarWithReplicates {
                summary: curvebook_doc::SummaryMeasure::Mean,
                fill_color: Rgba8::rgb(40, 120, 220),
                border_color: Rgba8::BLACK,
                border_width_pt: bw,
                replicate_overlay: None,
            };
        }
        (gs.spec.series[1].bind_to_right_y2, gs.spec.series[1].offset.x_data, gs.spec.forest_show_pooled_diamond) = (true, 0.35, false);
        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        assert!(
            !fg.bind_to_right_y2 && fg.offset_x_data.abs() < 1e-9 && (fg.bar_border_width_pt - 1.0).abs() < 1e-9 && !fg.forest_show_pooled_diamond
        );

        fg.selected_series_index = 1;
        fg.sync_selected_series_from_spec(&gs.spec);
        assert!(fg.bind_to_right_y2 && (fg.offset_x_data - 0.35).abs() < 1e-9 && (fg.bar_border_width_pt - 1.5).abs() < 1e-9);

        fg.apply_all_data_sets = true;
        (fg.bar_border_width_pt, fg.bar_border_width_dirty) = (0.5, true);
        (fg.bind_to_right_y2, fg.bind_to_right_y2_dirty) = (false, true);
        (fg.offset_x_data, fg.offset_x_data_dirty) = (0.25, true);
        (fg.forest_show_pooled_diamond, fg.forest_show_pooled_diamond_dirty) = (true, true);

        let intents = fg.commit_intents();
        assert_eq!(intents.iter().find(|i| i.command_id == "graph.setSpec").unwrap().params.get("forest_show_pooled_diamond"), Some(&json!(true)));
        let sp = &intents.iter().find(|i| i.command_id == "graph.setSeries").unwrap().params;
        assert_eq!(
            (sp.get("all_series"), sp.get("border_width_pt"), sp.get("bind_to_right_y2"), sp.get("offset")),
            (Some(&json!(true)), Some(&json!(0.5)), Some(&json!(false)), Some(&json!({ "x_data": 0.25 })))
        );
        assert_eq!(serde_json::from_value::<FormatGraphState>(serde_json::to_value(&fg).unwrap()).unwrap(), fg);
    }
}

mod aud09_remediation_tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
        JournalWidthPreset, LayoutSheet, LayoutSpec, ProjectDocument, ResultCell, ResultSheet, SheetId, SubcolumnMode, TableArchetype,
        TwoGroupMethod, TwoGroupSpec, XyTable,
    };
    use serde_json::json;

    use crate::dialogs_canvas::analysis_catalog::default_analysis_kind_for_id;
    use crate::dialogs_canvas::edit_params::sync_fit_parameter_inputs;
    use crate::dialogs_canvas::{
        ActiveDialog, AnalyzeDialogState, BubblePlotInspectorState, CanvasDialogState, ChangeDataAnalyzedState, ChangeGraphTypeState,
        CommandPaletteState, ConstantHookPickerState, CsvExportOptionsState, EditAnalysisParametersState, ExcludeWithReasonDialogState,
        FormatGraphState, FormatSelectedPointsState, PageLayoutComposerState, PairwiseComparisonsState, copy_result_table_to_clipboard,
    };

    fn mv_3x3() -> DataSheet {
        let mut mv = curvebook_doc::MultipleVariablesTable::new(3, 3).unwrap();
        for r in 0..3 {
            mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
            mv.variables[1].cells[r] = Cell::number((r + 2) as f64);
            mv.variables[2].cells[r] = Cell::number((r + 10) as f64);
        }
        DataSheet::new(SheetId::new("mv-glm"), "MV", TableArchetype::MultipleVariables(mv)).unwrap()
    }

    fn mk_rs(id: &str, src: &str, kind: AnalysisKind) -> ResultSheet {
        ResultSheet::new(AnalysisSpec::new(id, src, kind).unwrap()).unwrap()
    }

    #[test]
    fn aud09_analyze_dialog_and_ext01_fixes() {
        let col_ds = DataSheet::new(
            SheetId::new("col-1"),
            "Col",
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap()),
        )
        .unwrap();
        let mut az = AnalyzeDialogState::default();
        az.open_for_data_sheet(&col_ds);
        az.selected_entry_id = Some(String::from("stats.two_group"));
        for (hyp, bits_eq) in [(12.5, false), (0.0, true)] {
            az.two_group_method = Some(TwoGroupMethod::OneSampleT { hypothetical: 5.0 });
            az.two_group_hypothetical = hyp;
            assert!(matches!(
                az.build_analysis_kind(),
                Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical }, .. }))
                if (!bits_eq && (hypothetical - hyp).abs() < 1e-12) || (bits_eq && hypothetical.to_bits() == 0.0_f64.to_bits())
            ));
        }

        // AUD-09-11: GLM honors `is_response` column (last numeric column).
        let mv_ds = mv_3x3();
        az.open_for_data_sheet(&mv_ds);
        az.selected_entry_id = Some(String::from("stats.glm"));
        assert!(
            matches!(az.build_analysis_kind(), Some(AnalysisKind::GeneralizedLinearModel(glm)) if glm.response_column == 2 && glm.predictor_columns == vec![0, 1] && az.explicit_response_column.is_none())
        );

        // AUD-09-30: invalid entry never fabricates `analyze.run` or closes dialog.
        let doc = ProjectDocument::new("Analyze").unwrap();
        let mut cd = CanvasDialogState { active_dialog: ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("col-1") }, ..Default::default() };
        (cd.analyze.source_sheet_id, cd.analyze.selected_entry_id) = (Some(SheetId::new("col-1")), Some(String::from("nonexistent.entry")));
        assert!(
            cd.analyze.try_build_run_intent(&doc) == Err(Vec::new())
                && cd.submit_analyze_or_prompt(&doc).is_none()
                && matches!(cd.active_dialog, ActiveDialog::AnalyzeSheet { .. })
        );

        // AUD-09-33: Duplicate detection distinguishes inner variants of Transform/Multivariate/Survival/Outlier.
        let mut dup_doc = ProjectDocument::new("Dup").unwrap();
        dup_doc.data_sheets.push(mv_ds.clone());
        for (id, kind) in [
            ("res-pca", default_analysis_kind_for_id("stats.pca").unwrap()),
            ("res-math", default_analysis_kind_for_id("transform.math").unwrap()),
            ("res-km", default_analysis_kind_for_id("stats.survival_km").unwrap()),
            ("res-rout", AnalysisKind::OutlierDetection(curvebook_doc::OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        ] {
            dup_doc.result_sheets.push(mk_rs(id, "mv-glm", kind));
        }
        az.open_for_data_sheet(&mv_ds);
        for (entry, outlier, expected) in [
            ("stats.corr_matrix", "", None),
            ("stats.pca", "", Some("res-pca")),
            ("transform.normalize", "", None),
            ("stats.survival_cox", "", None),
            ("stats.outliers", "grubbs", None),
            ("stats.outliers", "rout", Some("res-rout")),
        ] {
            az.selected_entry_id = Some(entry.to_owned());
            if !outlier.is_empty() {
                az.outlier_method = outlier.to_owned();
            }
            let exp: Vec<SheetId> = expected.into_iter().map(SheetId::new).collect();
            assert_eq!(az.matching_existing_analyses(&dup_doc), exp, "{entry}/{outlier}");
        }

        // EXT-01: Dose-response auto-selects non-log for raw positive concentrations and resets across sheets.
        for (id, xs, expected_eq) in [
            ("xy-raw", [1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4], "agonist_vs_response_4pl"),
            ("xy-log", [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0], "log_agonist_vs_response_4pl"),
        ] {
            let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
            for (r, x) in xs.into_iter().enumerate() {
                xy.x_cells[r][0] = Cell::number(x);
            }
            az.open_for_data_sheet(&DataSheet::new(SheetId::new(id), id, TableArchetype::Xy(xy)).unwrap());
            az.selected_entry_id = Some(String::from("fit.nonlinear"));
            assert_eq!(az.equation_id, expected_eq);
            assert!(matches!(az.build_analysis_kind(), Some(AnalysisKind::NonlinearRegression(nl)) if nl.equation_id == expected_eq));
        }
    }

    #[test]
    fn aud09_12_graph_ranks_and_backed_diag_toggles() {
        let mut doc = ProjectDocument::new("Ranks").unwrap();
        doc.data_sheets.push(
            DataSheet::new(
                SheetId::new("col-mw"),
                "MW",
                TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap()),
            )
            .unwrap(),
        );
        let mut ep = EditAnalysisParametersState::default();
        ep.open_for_result(&mk_rs(
            "res-mw",
            "col-mw",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..Default::default() }),
        ));
        ep.graph_ranks = true;
        let intents = ep.commit_intents(Some(&doc));
        assert!(intents.iter().any(|i| i.command_id == "graph.create" && i.params["sheet_id"] == json!("col-mw_ranks")));
        assert!(intents.iter().any(|i| i.command_id == "graph.setKind" && i.params["sheet_id"] == json!("col-mw_ranks")));

        let mut nl = curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        (nl.normality_of_residuals, nl.emit_residuals_tab, nl.homoscedasticity_test) = (false, true, false);
        ep.open_for_result(&mk_rs("res-nl", "col-mw", AnalysisKind::NonlinearRegression(nl)));
        assert!(!ep.diag_qq_plot);
        (ep.diag_qq_plot, ep.diag_residuals_plot, ep.diag_homoscedasticity) = (true, false, true);
        let p = &ep.commit_intent().unwrap().params["kind"]["NonlinearRegression"];
        assert_eq!(
            (&p["normality_of_residuals"], &p["emit_residuals_tab"], &p["homoscedasticity_test"]),
            (&json!(true), &json!(false), &json!(true))
        );

        // AUD-09-12: Unsupported contingency CI methods refuse silent commit across all 4 slots.
        ep.open_for_result(&mk_rs("res-ct", "col-mw", AnalysisKind::Contingency(curvebook_doc::ContingencySpec::default())));
        assert!(ep.unsupported_contingency_ci_error().is_none() && ep.commit_intent().is_some());
        for (slot, bad, ok) in
            [(0usize, "Woolf", "Baptista-Pike"), (1, "Katz", "Koopman"), (2, "Wald", "Newcombe-Wilson"), (3, "Clopper-Pearson", "Wilson-Brown")]
        {
            let set_slot = |s: &mut EditAnalysisParametersState, v: &str| match slot {
                0 => s.contingency_or_ci_method = v.to_owned(),
                1 => s.contingency_rr_ci_method = v.to_owned(),
                2 => s.contingency_arr_ci_method = v.to_owned(),
                _ => s.contingency_diag_ci_method = v.to_owned(),
            };
            set_slot(&mut ep, bad);
            assert!(ep.unsupported_contingency_ci_error().is_some() && ep.commit_intent().is_none() && ep.commit_intents(None).is_empty());
            set_slot(&mut ep, ok);
            assert!(ep.unsupported_contingency_ci_error().is_none() && ep.commit_intent().is_some());
        }
    }

    #[test]
    fn aud09_13_25_37_49_results_and_hooks_fixes() {
        // AUD-09-13: analyzable_data-only ResultSheet exports >200 rows, X column, and all replicate subcolumns.
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 205).unwrap();
        (xy.x_title, xy.data_sets[0].title) = (String::from("Conc"), String::from("Resp"));
        for r in 0..205 {
            xy.x_cells[r][0] = Cell::number((r + 1) as f64);
            xy.data_sets[0].cells[r][0] = Cell::number((r * 2) as f64);
            xy.data_sets[0].cells[r][1] = Cell::number((r * 2 + 1) as f64);
        }
        let mut rs = mk_rs("res-tx", "xy-derived", default_analysis_kind_for_id("transform.math").unwrap());
        rs.analyzable_data = Some(DataSheet::new(SheetId::new("xy-derived"), "Derived", TableArchetype::Xy(xy)).unwrap());
        let mut doc = ProjectDocument::new("Copy").unwrap();
        doc.result_sheets.push(rs);
        let mut app = crate::UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let tsv = copy_result_table_to_clipboard(&mut app, "res-tx", true).command_result.unwrap()["copied_text"].as_str().unwrap().to_owned();
        let lines: Vec<&str> = tsv.lines().collect();
        assert!(
            lines.len() == 206
                && lines[0].contains("Conc")
                && lines[0].contains("Resp:1")
                && lines[0].contains("Resp:2")
                && lines[205].contains("409")
        );

        // AUD-09-25 + AUD-09-49(3): tab/sheet switch clears stale selection & last_methods_report.
        let mut rs_multi = mk_rs("res-multi", "xy-derived", default_analysis_kind_for_id("stats.descriptive").unwrap());
        rs_multi.tabs = [("t0", curvebook_doc::ResultTabKind::MainSummary, "TAB_ZERO"), ("t1", curvebook_doc::ResultTabKind::Diagnostics, "TAB_ONE")]
            .into_iter()
            .map(|(id, kind, txt)| curvebook_doc::ResultTab {
                id: id.to_owned(),
                title: id.to_owned(),
                kind,
                column_headers: vec![String::from("Val")],
                row_labels: vec![String::from("R1")],
                rows: vec![vec![ResultCell::Text(txt.to_owned())]],
                footnotes: Vec::new(),
            })
            .collect();
        let mut doc2 = ProjectDocument::new("Tab").unwrap();
        doc2.result_sheets.push(rs_multi.clone());
        let mut app2 = crate::UiApp::from_session(curvebook_engine::Session::from_document(doc2));
        app2.canvas_dialogs.result_view.sync_with_result(&rs_multi);
        (
            app2.canvas_dialogs.result_view.last_methods_report,
            app2.canvas_dialogs.result_view.bound_sheet_id,
            app2.canvas_dialogs.result_view.bound_tab_idx,
            app2.canvas_dialogs.result_view.selected_anchor,
        ) = (Some(String::from("stale")), Some(String::from("res-multi")), Some(0), Some((0, 0)));
        app2.canvas_dialogs.result_view.select_tab(1, 2);
        assert!(app2.canvas_dialogs.result_view.bound_tab_idx.is_none() && app2.canvas_dialogs.result_view.selected_anchor.is_none());
        assert!(
            copy_result_table_to_clipboard(&mut app2, "res-multi", false).command_result.unwrap()["copied_text"]
                .as_str()
                .unwrap()
                .contains("TAB_ONE")
        );
        app2.canvas_dialogs.result_view.sync_with_result(&mk_rs(
            "res-other",
            "xy-derived",
            default_analysis_kind_for_id("stats.descriptive").unwrap(),
        ));
        assert_eq!(app2.canvas_dialogs.result_view.last_methods_report, None);

        // AUD-09-37: ConstantHookPicker accepts Normalize zero_anchor, rejects LinearRegression.
        let mut hdoc = ProjectDocument::new("Hooks").unwrap();
        let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Info").unwrap();
        info.constants.push(InfoConstant::new("Ctrl", ConstantValue::Number(50.0), None).unwrap());
        hdoc.info_sheets.push(info);
        hdoc.result_sheets.push(mk_rs("r-lr", "xy-derived", AnalysisKind::LinearRegression(Default::default())));
        hdoc.result_sheets.push(mk_rs("r-norm", "xy-derived", default_analysis_kind_for_id("transform.normalize").unwrap()));
        let mut picker = ConstantHookPickerState::default();
        picker.open_for_document(SheetId::new("r-lr"), Some(String::from("Slope")), &hdoc);
        assert!(picker.commit_intent().is_none());
        picker.open_for_document(SheetId::new("r-norm"), Some(String::from("zero_anchor")), &hdoc);
        assert!(picker.commit_intent().expect("Normalize hook").params.get("zero_anchor").is_some());

        // AUD-09-49(2) & (4): ChangeDataAnalyzed target switch & linear plot range start == end rejection.
        let mut rdoc = ProjectDocument::new("Retarget").unwrap();
        for (id, cols) in [("c1", 4), ("c2", 2)] {
            rdoc.data_sheets.push(
                DataSheet::new(
                    SheetId::new(id),
                    id,
                    TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, 3).unwrap()),
                )
                .unwrap(),
            );
        }
        let mut aspec = AnalysisSpec::new("r1", "c1", default_analysis_kind_for_id("stats.descriptive").unwrap()).unwrap();
        aspec.selected_data_sets = vec![0, 1, 2, 3];
        let mut cda = ChangeDataAnalyzedState::default();
        cda.open_for_result(&ResultSheet::new(aspec).unwrap(), &rdoc);
        assert_eq!(cda.selected_data_sets, vec![0, 1]);
        cda.select_target(&SheetId::new("c1")).unwrap();
        assert!(cda.selected_data_sets.is_empty());

        let mut lr_app = crate::UiApp::new();
        lr_app.canvas_dialogs.edit_params.open_for_result(&mk_rs("r-lr", "c1", AnalysisKind::LinearRegression(Default::default())));
        for k in ["fit.lr.range.input.plot_min", "fit.lr.range.input.plot_max"] {
            lr_app.interaction.text_field_buffers.insert(k.to_owned(), crate::interaction::TextFieldEditState::new("5", false));
        }
        assert!(sync_fit_parameter_inputs(&mut lr_app).is_err());
    }

    #[test]
    fn aud09_14_15_16_29_32_34_38_47_and_aud13_07_graph_layout_fixes() {
        let ds = DataSheet::new(
            SheetId::new("col-fg"),
            "Col",
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()),
        )
        .unwrap();
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-fg"), &ds).unwrap();
        let mut fg = FormatGraphState::default();
        fg.sync_from_graph(&gs);
        (fg.apply_all_data_sets, fg.series_gdo_bar, fg.series_gdo_dirty) = (true, true, true);
        let si: Vec<_> = fg.commit_intents().into_iter().filter(|i| i.command_id == "graph.setSeries").collect();
        assert!(si.len() == 2 && si[0].params["gdo"] != si[1].params["gdo"]);

        gs.spec.series.clear();
        fg.mv_selected_level = Some(String::from("Stale"));
        fg.sync_from_graph(&gs);
        assert!(fg.mv_selected_level.is_none() && !fg.error_bar_enabled && !fg.commit_intents().iter().any(|i| i.command_id == "graph.setSeries"));

        let mut layout = LayoutSheet::new(SheetId::new("lay-1"), "Custom", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).unwrap();
        (layout.spec.width, layout.spec.height, layout.spec.unit, layout.spec.background_fill) =
            (123.0, 97.0, curvebook_doc::PhysicalUnit::Millimeters, None);
        let mut plc = PageLayoutComposerState::default();
        plc.sync_from_layout(&layout);
        let m = plc.to_layout_spec();
        assert!((m.width - 123.0).abs() < 1e-9 && (m.height - 97.0).abs() < 1e-9 && m.background_fill.is_none());
        plc.apply_preset(JournalWidthPreset::DoubleColumn174Mm);
        let sw = plc.to_layout_spec();
        assert!((sw.width - 174.0).abs() < 1e-9 && sw.background_fill.is_none());

        let mut csv = CsvExportOptionsState::default();
        csv.reset_for_locale(',');
        assert_eq!((csv.delimiter, csv.decimal_separator), (';', ','));

        let mut fsp =
            FormatSelectedPointsState { color: Some(String::from("#ff0000")), size_pt: Some(12.0), show_row_labels: true, ..Default::default() };
        fsp.open_for_graph(SheetId::new("g-2"), vec![(0, 1)]);
        assert!(fsp.color.is_none() && fsp.size_pt.is_none() && !fsp.show_row_labels);

        let mut excl = ExcludeWithReasonDialogState {
            reason_tag: String::from("Bubble"),
            note: String::from("old"),
            timestamp_utc: Some(String::from("2026-01-01T00:00:00Z")),
            ..Default::default()
        };
        excl.open_for_coords(SheetId::new("d-2"), vec![]);
        assert!(excl.reason_tag == "Outlier" && excl.note.is_empty() && excl.timestamp_utc.is_none());

        let mut doc = ProjectDocument::new("Overlays").unwrap();
        let mut pw =
            PairwiseComparisonsState { bracket_style: curvebook_doc::BracketStyle::CurlyBrace, hide_non_significant: false, ..Default::default() };
        pw.sync_from_graph(&gs, &doc);
        assert!(pw.bracket_style == curvebook_doc::BracketStyle::Square && pw.hide_non_significant);

        let mut rs_derived = mk_rs("res-mv", "missing-src", default_analysis_kind_for_id("transform.math").unwrap());
        rs_derived.analyzable_data = Some(
            DataSheet::new(
                SheetId::new("res-mv-data"),
                "Derived",
                TableArchetype::MultipleVariables(curvebook_doc::MultipleVariablesTable::new(3, 2).unwrap()),
            )
            .unwrap(),
        );
        doc.result_sheets.push(rs_derived);
        let mut g_mv = gs;
        (g_mv.spec.primary_source_sheet_id, g_mv.spec.multivariate) = (SheetId::new("res-mv"), None);
        let mut bp = BubblePlotInspectorState { x_column: 2, show_convex_hulls: true, ..Default::default() };
        bp.sync_from_graph(&g_mv, &doc);
        assert!(bp.available_columns.len() == 3 && bp.x_column == 0 && !bp.show_convex_hulls);

        let mut cdoc = ProjectDocument::new("Palette").unwrap();
        cdoc.data_sheets.push(ds.clone());
        cdoc.graph_sheets.push(GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds).unwrap());
        cdoc.layout_sheets.push(layout);
        cdoc.active_sheet_id = Some(SheetId::new("lay-1"));
        let sess = curvebook_engine::Session::from_document(cdoc);
        let intent = CommandPaletteState { query: String::from("graph.applyCanvasPreset"), ..Default::default() }
            .activate_selected_for_session(Some(&sess))
            .unwrap();
        assert_eq!((intent.command_id.as_str(), &intent.params["sheet_id"]), ("graph.applyCanvasPreset", &json!("g-1")));

        let cgt = ChangeGraphTypeState {
            graph_sheet_id: Some(SheetId::new("g-missing")),
            selected_option_id: String::from("grouped.superimposed"),
            ..Default::default()
        };
        assert_eq!(cgt.commit_intent().unwrap().params["grouped_arrangement"], json!(curvebook_doc::GroupedArrangement::Superimposed));
        assert!(cgt.commit_intents(&doc).iter().any(|i| i.command_id == "graph.setSpec" && i.params["grouped_arrangement"] == json!(curvebook_doc::GroupedArrangement::Superimposed)));
    }

    #[test]
    fn r20_6_and_r20_10_xy_wire_name_ambiguity_only_banner_and_tsv_explicit_ci() {
        use crate::AssumptionDiagnosticBanner;
        use crate::dialogs_canvas::ResultSheetViewState;
        use crate::dialogs_canvas::workspace_dialogs::convert_target_wire_name;
        use curvebook_doc::{ChecklistItem, ChecklistStatus, CiBound, PValueFormat, ResultStatus, ResultTab, ResultTabKind};

        assert_eq!(convert_target_wire_name(curvebook_doc::TableKind::Xy), "XY");

        let both_open = ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open };
        let half_open = ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Open };
        let half_unavail = ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Unavailable };
        assert_eq!(ResultSheetViewState::format_result_cell(&both_open, PValueFormat::FourDecimal), "Not bounded");
        assert_eq!(ResultSheetViewState::format_result_cell(&half_open, PValueFormat::FourDecimal), "[0.1753, unbounded]");
        assert_eq!(ResultSheetViewState::format_result_cell(&half_unavail, PValueFormat::FourDecimal), "[0.1753, Unavailable]");
        assert_eq!(ResultSheetViewState::format_result_cell_with_survival(&both_open, PValueFormat::FourDecimal, true), "Not reached");
        assert_eq!(
            ResultSheetViewState::format_result_cell_with_survival(
                &ResultCell::ConfidenceInterval { lower: CiBound::Finite(14.0), upper: CiBound::Open },
                PValueFormat::FourDecimal,
                true,
            ),
            "[14, Not reached]"
        );

        let mut doc = ProjectDocument::default();
        let mut rs = ResultSheet::new(
            AnalysisSpec::new(
                "res-nl-only",
                "ds-1",
                AnalysisKind::NonlinearRegression(curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
            )
            .unwrap(),
        )
        .unwrap();
        rs.status = ResultStatus::UpToDate;
        rs.tabs.push(ResultTab {
            id: String::from("nonlinear_parameters"),
            title: String::from("Nonlinear fit"),
            kind: ResultTabKind::ParametersAndConfidenceIntervals,
            column_headers: vec![String::from("VEHICLE"), String::from("DRUG")],
            row_labels: vec![String::from("IC50")],
            rows: vec![vec![both_open.clone(), ResultCell::Number { value: 1.5, prefix: None, units: None }]],
            footnotes: Vec::new(),
        });
        assert!(rs.checklist.is_empty());
        let banner = AssumptionDiagnosticBanner::from_result(&rs).expect("ambiguity-only NL cells must surface single banner");
        assert!(banner.summary.contains("VEHICLE") && !banner.summary.contains("???"));

        rs.checklist.push(ChecklistItem {
            assumption: String::from("X-axis scale matches log10(X) equation"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: None,
            explanation: String::from("Non-log X scale."),
            remediation: None,
        });
        let combined = AssumptionDiagnosticBanner::from_result(&rs).expect("combined banner");
        assert!(combined.summary.contains("VEHICLE") && combined.summary.contains("X-axis scale matches log10(X) equation"));

        doc.result_sheets.push(rs);
        let mut app = crate::UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let out = copy_result_table_to_clipboard(&mut app, "res-nl-only", true);
        let tsv = out.command_result.unwrap()["copied_text"].as_str().unwrap().to_owned();
        assert!(tsv.contains("Not bounded") && !tsv.contains("???"), "unexpected TSV: {tsv}");
    }
}
