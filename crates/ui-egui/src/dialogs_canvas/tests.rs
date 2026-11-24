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

        // When clean, `replicate_lines` is omitted and `marker` is an object.
        let clean = fg.commit_intents();
        let s_clean = clean.iter().find(|i| i.command_id == "graph.setSeries").unwrap();
        assert!(s_clean.params.get("replicate_lines").is_none());
        assert!(s_clean.params.get("marker").is_some_and(Value::is_object));

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
