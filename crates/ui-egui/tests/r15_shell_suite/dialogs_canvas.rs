//! Integration tests covering UI Parity dialogs, fuzzy Command Palette (`Cmd+K`), non-modal
//! graphing/layout inspectors, `Apply Style From...`, and deterministic software workspace PNG
//! screenshots (`ui.screenshot`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, Cell, CellCoord, ChecklistItem, ChecklistStatus, CiBound, ColorChannelMapping, ColumnDesignHint,
    ColumnTable, ConstantValue, ContingencyTable, DataSheet, FramePreset, GraphAnnotation, GraphKind, GraphSheet, GroupedTable, HookSource,
    InfoConstant, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, PValueStackMethod, PValueStackSpec, PairwiseComparisonRecord,
    PanelBadgeStyle, ParameterScope, ProjectDocument, RectPt, RemediationKind, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus,
    ResultTabKind, SizeChannelMapping, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, VariableRole, XyTable, extract_template_hooks,
};
use curvebook_engine::Session;
use curvebook_palette::{ColormapId, CvdKind, LineDashStyle, MarkerFillMode, PaletteId};
use curvebook_render::RenderTargetSpec;
use curvebook_tools::{GridCellCoord, JournalWidthPreset as ToolJournalWidthPreset, PhysicalUnit, SelectionQuickStats};
use curvebook_ui_egui::dialogs_canvas::{
    ANALYSIS_CATALOG, ActiveDialog, ActiveInspector, AnalyzeDialogState, AnalyzeDuplicateChoice, AxisTarget, CommandOrigin, CommandPaletteState,
    ResultSheetViewState, VariableRoleFilter, default_analysis_kind_for_id, variable_role_badge, variable_role_color,
};
use curvebook_ui_egui::theme::{UiThemeSpec, font_diagnostics};
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{ScreenshotRegionTarget, UiApp, dispatch_gui_action};
use serde_json::json;

use crate::common::{
    PixelRect, assert_valid_png_with_dimensions, fill_col_values, measure_egui_regions_for_target, region_distinct_colors, result_scalar,
};
use crate::helpers::make_result_tab;

fn build_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Biochem Panel").unwrap();

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).unwrap();
    xy.data_sets[0].title = "10.5".into();
    xy.data_sets[1].title = "Dose B".into();
    let mut ds_xy = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy)).unwrap();
    for (r, (x, y)) in [(-8.0, 5.0), (-7.0, 14.0), (-6.0, 48.0), (-5.0, 82.0), (-4.0, 95.0), (-3.0, 98.0)].into_iter().enumerate() {
        ds_xy.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        ds_xy.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).unwrap();
    }
    doc.add_data_sheet(ds_xy).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).unwrap();
    for (c, (t, vals)) in [("A", &[10.0, 11.2, 9.8, 10.5, 11.0][..]), ("B", &[18.5, 19.2, 20.1, 18.8, 19.6]), ("C", &[11.0, 10.7, 11.5, 10.9, 11.2])]
        .into_iter()
        .enumerate()
    {
        fill_col_values(&mut col.data_sets[c], t, vals);
    }
    doc.add_data_sheet(DataSheet::new("data-col", "Paired Treatment", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    for (c, (t, vals)) in [("A", &[5.1, 5.4, 4.9, 5.2][..]), ("B", &[9.1, 8.8, 9.4, 9.0]), ("C", &[6.0, 6.2, 5.9, 6.1])].into_iter().enumerate() {
        fill_col_values(&mut col2.data_sets[c], t, vals);
    }
    doc.add_data_sheet(DataSheet::new("data-col2", "Validation Cohort", TableArchetype::Column(col2)).unwrap()).unwrap();

    let mut mv = MultipleVariablesTable::new(4, 5).unwrap();
    for (i, (t, role)) in [
        ("Response", VariableRole::Continuous),
        ("Cohort", VariableRole::Categorical),
        ("SampleID", VariableRole::Label),
        ("LogRatio", VariableRole::Continuous),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[i].title = t.into();
        mv.variables[i].role = role;
    }
    mv.variables[3].set_calculated_formula("log(A)");
    for r in 0..5 {
        mv.variables[0].cells[r] = Cell::number((r as f64 + 1.0) * 10.0);
        mv.variables[1].cells[r] = Cell::text(if r % 2 == 0 { "Ctrl" } else { "Drug" });
        mv.variables[2].cells[r] = Cell::text(format!("S{}", r + 1));
        mv.variables[3].cells[r] = Cell::number(r as f64 + 1.0);
    }
    doc.add_data_sheet(DataSheet::new("data-mv", "Observational Cohort", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    doc.add_data_sheet(DataSheet::new("data-cont", "Responder Counts", TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap())).unwrap())
        .unwrap();

    let mut info = InfoSheet::new_preseeded("info-1", "Lab Protocol").unwrap();
    info.constants.push(InfoConstant::new("Control_EC50", ConstantValue::Number(12.5), Some("nM".into())).unwrap());
    info.notes_markdown = "Assay incubated at 37 C for 30 min.".into();
    doc.add_info_sheet(info).unwrap();

    let anova_kind = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::TukeyHsd,
            threshold: 0.05,
            report_confidence_intervals: true,
            control_column: None,
            selected_pairs: None,
        },
        ..Default::default()
    });
    let mut res = ResultSheet::new(AnalysisSpec::new("res-anova", "data-col", anova_kind).unwrap()).unwrap();
    res.status = ResultStatus::UpToDate;
    res.tabs = vec![
        make_result_tab(
            "summary",
            "ANOVA Summary",
            ResultTabKind::AnovaTable,
            &["Value"],
            &["P value", "95% CI"],
            vec![
                vec![ResultCell::PValue { raw: 0.0023, formatted: "0.0023".into(), stars: "**".into() }],
                vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(4.2), upper: CiBound::Open }],
            ],
        ),
        make_result_tab(
            "comparisons",
            "Multiple Comparisons",
            ResultTabKind::MultipleComparisons,
            &["Adj. P"],
            &["Ctrl vs Treated"],
            vec![vec![ResultCell::Number { value: 0.0041, prefix: None, units: None }]],
        ),
    ];
    res.scalars.push(ResultScalarEntry { key: "EC50".into(), data_set_index: Some(0), value: 18.75, units: Some("nM".into()) });
    res.pairwise_comparisons = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: "Control".into(),
            label_b: "Treated".into(),
            estimate_diff: Some(8.4),
            raw_p_or_q: 0.0041,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 1,
            group_b: 2,
            label_a: "Treated".into(),
            label_b: "Washout".into(),
            estimate_diff: Some(1.1),
            raw_p_or_q: 0.48,
            is_fdr_q: false,
            significant: false,
        },
    ];
    res.checklist = vec![
        ChecklistItem {
            assumption: "Normality of residuals (Shapiro-Wilk)".into(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: Some("W = 0.97, p = 0.62".into()),
            explanation: "Consistent with Gaussian.".into(),
            remediation: None,
        },
        ChecklistItem {
            assumption: "Equal variances (Brown-Forsythe)".into(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some("p = 0.041".into()),
            explanation: "Consider Welch ANOVA.".into(),
            remediation: Some(RemediationKind::WelchAnova),
        },
    ];
    doc.add_result_sheet(res).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new("res-fit", "data-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-col", doc.data_sheet(&"data-col".into()).unwrap()).unwrap()).unwrap();
    let mut g_donor = GraphSheet::new_uninitialized_for_data("graph-donor", doc.data_sheet(&"data-xy".into()).unwrap()).unwrap();
    g_donor.initialized = true;
    g_donor.title = "Styled Donor".into();
    g_donor.spec.palette = PaletteId::PaulTolVibrant;
    g_donor.spec.legend.bordered = true;
    g_donor.spec.x_axis.stroke_width_pt = 2.0;
    doc.add_graph_sheet(g_donor).unwrap();
    let mut g_mv = GraphSheet::new_uninitialized_for_data("graph-mv", doc.data_sheet(&"data-mv".into()).unwrap()).unwrap();
    g_mv.initialized = true;
    g_mv.spec.kind = GraphKind::MultipleVariables;
    doc.add_graph_sheet(g_mv).unwrap();
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1 Composite", LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm)).unwrap())
        .unwrap();
    doc
}

fn copy_rect_rgba(rgba: &[u8], frame_width: u32, r: PixelRect) -> Vec<u8> {
    let mut out = Vec::with_capacity((r.width as usize) * (r.height as usize) * 4);
    for py in r.y..r.y.saturating_add(r.height) {
        let s = ((py as usize) * (frame_width as usize) + (r.x as usize)) * 4;
        if let Some(slice) = rgba.get(s..s + (r.width as usize) * 4) {
            out.extend_from_slice(slice);
        }
    }
    out
}

#[test]
fn analyze_dialog_filters_by_table_archetype_search_and_role_badges() {
    let doc = build_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &"data-col".into()));
    let compat = app.canvas_dialogs.analyze.compatible_entries();
    assert!(compat.iter().any(|i| i.id == "stats.two_group") && compat.iter().any(|i| i.id == "stats.one_way_anova"));
    assert!(!compat.iter().any(|i| i.id == "stats.survival_km") && !compat.iter().any(|i| i.id == "fit.linear"));

    app.canvas_dialogs.analyze.search_query = "welch".into();
    assert!(!app.canvas_dialogs.analyze.compatible_entries().is_empty() && app.canvas_dialogs.analyze.empty_feedback().is_none());
    app.canvas_dialogs.analyze.search_query = "zzznonexistent".into();
    let empty_msg = app.canvas_dialogs.analyze.empty_feedback().unwrap();
    assert!(app.canvas_dialogs.analyze.compatible_entries().is_empty() && empty_msg.contains("zzznonexistent") && empty_msg.contains("Column"));

    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &"data-mv".into()));
    assert_eq!(app.canvas_dialogs.analyze.variable_candidates.len(), 4);
    app.canvas_dialogs.analyze.role_filter = VariableRoleFilter::NumericForModeling;
    let num = app.canvas_dialogs.analyze.filtered_variables();
    assert_eq!(num.len(), 2);
    assert!(num.iter().all(|v| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated)));
    app.canvas_dialogs.analyze.role_filter = VariableRoleFilter::CategoricalOnly;
    let cat = app.canvas_dialogs.analyze.filtered_variables();
    assert_eq!((cat.len(), cat[0].title.as_str()), (1, "Cohort"));

    for (role, badge) in
        [(VariableRole::Continuous, "NUM"), (VariableRole::Categorical, "CAT"), (VariableRole::Label, "LBL"), (VariableRole::Calculated, "FX=")]
    {
        assert_eq!(variable_role_badge(role), badge);
    }
    let light = UiThemeSpec::light();
    assert_ne!(variable_role_color(VariableRole::Continuous, &light), variable_role_color(VariableRole::Categorical, &light));

    app.canvas_dialogs.analyze.selected_entry_id = Some("stats.glm".into());
    app.canvas_dialogs.analyze.set_response_column(0);
    assert!(
        matches!(app.canvas_dialogs.analyze.build_analysis_kind().unwrap(), AnalysisKind::GeneralizedLinearModel(glm) if glm.response_column == 0 && !glm.predictor_columns.contains(&2))
    );
}

#[test]
fn result_viewer_checklist_and_constant_hook_picker_work() {
    let doc = build_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    app.canvas_dialogs.on_sheet_selected(&doc, &"res-anova".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::ResultDetails);
    let res = doc.result_sheet(&"res-anova".into()).unwrap();
    assert!(app.canvas_dialogs.result_view.select_tab_by_id(res, "comparisons"));
    assert_eq!(app.canvas_dialogs.result_view.active_tab(res).map(|t| t.id.as_str()), Some("comparisons"));
    assert_eq!(ResultSheetViewState::checklist_summary_counts(res), (1, 1, 0, 0));
    app.canvas_dialogs.result_view.checklist_filter = curvebook_ui_egui::dialogs_canvas::ChecklistSeverityFilter::WarningsAndViolations;
    assert_eq!(app.canvas_dialogs.result_view.visible_checklist(res).len(), 1);
    assert_eq!(
        ResultSheetViewState::format_result_cell(
            &ResultCell::ConfidenceInterval { lower: CiBound::Finite(4.2), upper: CiBound::Open },
            PValueFormat::Apa
        ),
        "[4.2000, unbounded]"
    );

    app.canvas_dialogs.open_hook_picker(&doc, &"graph-donor".into(), Some("Bottom".into()));
    app.canvas_dialogs.hook_picker.search_query = "EC50".into();
    let hits = app.canvas_dialogs.hook_picker.filtered_options();
    assert!(hits.len() >= 2);
    for opt in &hits {
        if opt.template_token.starts_with("{{info:") || opt.template_token.starts_with("{{result:") {
            assert_eq!(extract_template_hooks(&opt.template_token).len(), 1);
        }
    }
    let hook = app.canvas_dialogs.hook_picker.commit_intent().unwrap();
    assert_eq!((hook.command_id.as_str(), hook.params["sheet_id"].as_str()), ("graph.addAnnotation", Some("graph-donor")));
    assert!(hook.params["text"].as_str().is_some_and(|t| t.starts_with("{{")));
}

#[test]
fn engine_command_round_trips_for_analysis_and_constant_hook_dialogs() {
    let mut app = UiApp::from_session(Session::from_document(build_project()));
    app.execute_command("analyze.run", json!({})).unwrap();

    // 1. Analyze -> Create New (`analysis.run`)
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &"data-xy".into()));
    app.canvas_dialogs.analyze.selected_entry_id = Some("stats.descriptive".into());
    let create = app.canvas_dialogs.analyze.build_create_new_intent().unwrap();
    assert_eq!(create.command_id, "analysis.run");
    let before = app.document().result_sheets.len();
    app.execute_command(&create.command_id, create.params).unwrap();
    assert_eq!(app.document().result_sheets.len(), before + 1);

    // 2. DuplicateAnalysisPrompt -> ModifyExisting & CreateNew
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &"data-col".into()));
    app.canvas_dialogs.analyze.selected_entry_id = Some("stats.one_way_anova".into());
    assert!(app.canvas_dialogs.submit_analyze_or_prompt(&snap).is_none());
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
    assert_eq!(app.canvas_dialogs.analyze.resolve_duplicate_choice(&AnalyzeDuplicateChoice::CreateNew).unwrap().command_id, "analysis.run");
    let modi = app.canvas_dialogs.analyze.resolve_duplicate_choice(&AnalyzeDuplicateChoice::ModifyExisting("res-anova".into())).unwrap();
    assert_eq!((modi.command_id.as_str(), modi.params["sheet_id"].as_str()), ("analysis.updateParams", Some("res-anova")));
    app.execute_command(&modi.command_id, modi.params).unwrap();

    // 3. EditAnalysisParameters (Cmd+T) auto-title / custom title & commit
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &"res-anova".into()));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("Ordinary one-way ANOVA"));
    let mut rm_spec = OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }, ..Default::default() };
    assert!(app.canvas_dialogs.edit_params.set_kind(AnalysisKind::OneWayAnova(rm_spec.clone())));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("RM one-way ANOVA"));
    app.canvas_dialogs.edit_params.set_custom_title(Some("RM Assay Title".into())).unwrap();
    rm_spec.method = OneWayMethod::KruskalWallis;
    assert!(app.canvas_dialogs.edit_params.set_kind(AnalysisKind::OneWayAnova(rm_spec)));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("RM Assay Title"));
    let edit = app.canvas_dialogs.edit_params.commit_intent().unwrap();
    app.execute_command(&edit.command_id, edit.params).unwrap();
    assert_eq!(app.document().result_sheet(&"res-anova".into()).unwrap().analysis.effective_title(), "RM Assay Title");

    // 4. ChangeDataAnalyzed compatibility check + retarget
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_change_data_analyzed(&snap, &"res-anova".into()));
    assert!(app.canvas_dialogs.change_data.select_target(&"data-cont".into()).is_err());
    assert!(app.canvas_dialogs.change_data.select_target(&"data-mv".into()).is_ok());
    app.canvas_dialogs.change_data.select_target(&"data-col2".into()).unwrap();
    let ret = app.canvas_dialogs.change_data.commit_intent().unwrap();
    assert_eq!(ret.command_id, "analysis.retargetData");
    app.execute_command(&ret.command_id, ret.params).unwrap();
    assert_eq!(app.document().result_sheet(&"res-anova".into()).unwrap().analysis.source_sheet_id, Some("data-col2".into()));

    // 5. ConstantHookPicker -> HookedConstant, graph.addAnnotation, layout.addPanel
    let snap = app.document().clone();
    app.canvas_dialogs.open_hook_picker(&snap, &"res-fit".into(), Some("Bottom".into()));
    let fh = app.canvas_dialogs.hook_picker.commit_intent().unwrap();
    app.execute_command(&fh.command_id, fh.params).unwrap();
    assert!(
        matches!(&app.document().result_sheet(&"res-fit".into()).unwrap().analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.parameter_rules.iter().any(|r| r.name == "Bottom" && matches!(r.scope, ParameterScope::HookedConstant(HookSource::InfoConstant { .. }))))
    );

    app.canvas_dialogs.open_hook_picker(&snap, &"graph-donor".into(), None);
    let gh = app.canvas_dialogs.hook_picker.commit_intent().unwrap();
    app.execute_command(&gh.command_id, gh.params).unwrap();
    assert!(app.document().graph_sheet(&"graph-donor".into()).unwrap().spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::TextBox { .. })));

    app.canvas_dialogs.open_hook_picker(&snap, &"layout-1".into(), None);
    let lh = app.canvas_dialogs.hook_picker.commit_intent().unwrap();
    app.execute_command(&lh.command_id, lh.params).unwrap();
    assert!(app.document().layout_sheet(&"layout-1".into()).unwrap().spec.items.iter().any(|i| matches!(i, LayoutItem::TextCallout { .. })));
}

#[test]
fn engine_command_round_trips_for_graph_and_layout_inspectors() {
    let mut app = UiApp::from_session(Session::from_document(build_project()));
    app.execute_command("analyze.run", json!({})).unwrap();

    // 1. First-visit ChangeGraphType recommends Before-After first & initializes graph
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-col".into());
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { graph_sheet_id: "graph-col".into() });
    assert_eq!(app.canvas_dialogs.change_graph_type.recommended_options().first().map(|o| o.id), Some("col.before_after"));
    let kind_i = app.canvas_dialogs.change_graph_type.commit_intent().unwrap();
    app.execute_command(&kind_i.command_id, kind_i.params).unwrap();
    assert!(app.document().graph_sheet(&"graph-col".into()).unwrap().initialized);

    // 2. PairwiseComparisons brackets + CLD enable/disable
    let snap = app.document().clone();
    app.canvas_dialogs.pairwise.sync_from_graph(snap.graph_sheet(&"graph-col".into()).unwrap(), &snap);
    assert!(app.canvas_dialogs.pairwise.auto_add_from_analysis(&snap) >= 1);
    let cmp_i = app.canvas_dialogs.pairwise.commit_intent().unwrap();
    app.execute_command(&cmp_i.command_id, cmp_i.params).unwrap();
    assert!(!app.document().graph_sheet(&"graph-col".into()).unwrap().spec.comparisons.is_empty());

    app.canvas_dialogs.pairwise.toggle_cld_enabled();
    let cld_i = app.canvas_dialogs.pairwise.cld_commit_intent().unwrap();
    assert_eq!(cld_i.command_id, "graph.setCompactLetterDisplay");
    app.execute_command(&cld_i.command_id, cld_i.params).unwrap();
    let g_cld = app.document().graph_sheet(&"graph-col".into()).unwrap();
    assert!(g_cld.spec.compact_letter_display.is_some() && g_cld.spec.pairwise_display_mode != curvebook_doc::PairwiseDisplayMode::BracketsOnly);

    let snap2 = app.document().clone();
    app.canvas_dialogs.pairwise.sync_from_graph(snap2.graph_sheet(&"graph-col".into()).unwrap(), &snap2);
    assert!(app.canvas_dialogs.pairwise.cld_enabled);
    app.canvas_dialogs.pairwise.toggle_cld_enabled();
    let dis_i = app.canvas_dialogs.pairwise.cld_commit_intent().unwrap();
    app.execute_command(&dis_i.command_id, dis_i.params).unwrap();
    assert!(app.document().graph_sheet(&"graph-col".into()).unwrap().spec.compact_letter_display.is_none());

    // 3. FormatGraph + CVD preview + dashed error bars
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-donor".into());
    app.canvas_dialogs.format_graph.palette = PaletteId::PaulTolHighContrast;
    app.canvas_dialogs.format_graph.cvd_preview = Some(CvdKind::Deuteranopia);
    assert_ne!(app.canvas_dialogs.format_graph.preview_swatch(0), PaletteId::PaulTolHighContrast.color_at(0));
    app.canvas_dialogs.format_graph.frame_preset = FramePreset::PlainBoxed;
    app.canvas_dialogs.format_graph.legend_bordered = false;
    app.canvas_dialogs.format_graph.marker_size_pt = 8.0;
    app.canvas_dialogs.format_graph.marker_fill_mode = MarkerFillMode::Open;
    app.canvas_dialogs.format_graph.line_dash = LineDashStyle::Dotted;
    app.canvas_dialogs.format_graph.error_bar_spec.dash_style = LineDashStyle::Dashed;
    let fg_intents = app.canvas_dialogs.format_graph.commit_intents();
    assert_eq!((fg_intents.len(), fg_intents[0].command_id.as_str(), fg_intents[1].command_id.as_str()), (2, "graph.setSpec", "graph.setSeries"));
    for i in fg_intents {
        app.execute_command(&i.command_id, i.params).unwrap();
    }
    let donor = app.document().graph_sheet(&"graph-donor".into()).unwrap();
    assert_eq!(
        (donor.spec.palette, donor.spec.frame_preset, donor.spec.legend.bordered),
        (PaletteId::PaulTolHighContrast, FramePreset::PlainBoxed, false)
    );

    // 4. FormatAxes broken Y + Y2 dual axis + invalid bounds guard
    assert!(app.canvas_dialogs.format_axes.add_segment() && app.canvas_dialogs.format_axes.set_segment_scale(1, AxisScale::Log10));
    assert!(app.canvas_dialogs.format_axes.set_segment_bounds(0, Some(0.0), Some(10.0)).is_ok());
    assert!(app.canvas_dialogs.format_axes.set_segment_bounds(0, Some(20.0), Some(10.0)).is_err());
    let ya = app.canvas_dialogs.format_axes.commit_intent().unwrap();
    app.execute_command(&ya.command_id, ya.params).unwrap();
    assert_eq!(app.document().graph_sheet(&"graph-donor".into()).unwrap().spec.y_axis.segments.len(), 2);

    let snap = app.document().clone();
    app.canvas_dialogs.format_axes.sync_from_graph(snap.graph_sheet(&"graph-donor".into()).unwrap(), AxisTarget::Y2);
    app.canvas_dialogs.format_axes.enable_y2 = true;
    let y2a = app.canvas_dialogs.format_axes.commit_intent().unwrap();
    app.execute_command(&y2a.command_id, y2a.params).unwrap();
    assert!(app.document().graph_sheet(&"graph-donor".into()).unwrap().spec.y2_axis.is_some());
}

#[test]
fn engine_command_round_trips_for_bubble_style_and_layout_inspectors() {
    let mut app = UiApp::from_session(Session::from_document(build_project()));
    app.execute_command("analyze.run", json!({})).unwrap();

    // 5. BubblePlot 5-channel inspector (direct `apply_to_spec` + live engine round-trip)
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-mv".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::BubblePlot);
    app.canvas_dialogs.bubble_plot.set_color_continuous(0, ColormapId::Viridis, Some((0.0, 100.0)));
    app.canvas_dialogs.bubble_plot.set_size_variable(3, 3.0, 12.0);
    app.canvas_dialogs.bubble_plot.label_column = Some(2);
    app.canvas_dialogs.bubble_plot.show_convex_hulls = true;
    let mut local_spec = snap.graph_sheet(&"graph-mv".into()).unwrap().spec.clone();
    app.canvas_dialogs.bubble_plot.apply_to_spec(&mut local_spec);
    assert!(local_spec.multivariate.as_ref().is_some_and(|m| m.label_column == Some(2) && m.show_convex_hulls));
    let mv_i = app.canvas_dialogs.bubble_plot.commit_intent().unwrap();
    app.execute_command(&mv_i.command_id, mv_i.params).unwrap();
    let mv_enc = app.document().graph_sheet(&"graph-mv".into()).unwrap().spec.multivariate.as_ref().unwrap();
    assert!(matches!(mv_enc.color, ColorChannelMapping::Continuous { .. }) && matches!(mv_enc.size, SizeChannelMapping::Variable { .. }));
    assert_eq!((mv_enc.label_column, mv_enc.show_convex_hulls), (Some(2), true));

    // 6. ApplyStyleFrom preserves data bindings
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_apply_style_from(&snap, &"graph-col".into()));
    let preview = app.canvas_dialogs.apply_style.preview_applied_spec(&snap).unwrap();
    assert_eq!((preview.primary_source_sheet_id.as_str(), preview.palette, preview.legend.bordered), ("data-col", PaletteId::PaulTolVibrant, true));
    let st_i = app.canvas_dialogs.apply_style.commit_intent().unwrap();
    app.execute_command(&st_i.command_id, st_i.params).unwrap();
    let restyled = app.document().graph_sheet(&"graph-col".into()).unwrap();
    assert_eq!((restyled.spec.primary_source_sheet_id.as_str(), restyled.spec.palette), ("data-col", PaletteId::PaulTolVibrant));

    // 7. PageLayoutComposer grid + badges + callout
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"layout-1".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::PageLayout);
    app.canvas_dialogs.page_layout.panel_badges = PanelBadgeStyle::UpperCaseLetters;
    app.canvas_dialogs.page_layout.arrange_grid(&["graph-col".into(), "graph-donor".into()], 12.0, 8.0);
    app.canvas_dialogs.page_layout.add_text_callout("Mean = {{result:res-anova:0:R2}}".into(), RectPt::new(12.0, 200.0, 180.0, 24.0), 10.0);
    let li = app.canvas_dialogs.page_layout.commit_intents();
    assert_eq!((li.len(), li[0].command_id.as_str(), li[1].command_id.as_str()), (2, "layout.setSpec", "layout.alignOrigins"));
    for i in li {
        app.execute_command(&i.command_id, i.params).unwrap();
    }
    let layout = app.document().layout_sheet(&"layout-1".into()).unwrap();
    assert!(layout.spec.panel_badges == PanelBadgeStyle::UpperCaseLetters && layout.spec.align_plot_origins && layout.spec.items.len() >= 3);
}

#[test]
fn command_palette_fuzzy_matches_engine_and_ui_commands() {
    let mut palette = CommandPaletteState::default();
    palette.open();
    let all = palette.matches();
    assert!(all.iter().any(|c| c.origin == CommandOrigin::Engine) && all.iter().any(|c| c.origin == CommandOrigin::Ui));

    for (q, top, also_present, activate) in [
        ("fitSheet", "ui.zoom.fitSheet", None, None),
        ("exppng", "export.png", Some("graph.exportPng"), Some("export.png")),
        ("graphexppng", "graph.exportPng", None, Some("graph.exportPng")),
    ] {
        palette.query = q.into();
        let hits = palette.matches();
        assert_eq!(hits.first().map(|c| c.id), Some(top));
        if let Some(id) = also_present {
            assert!(hits.iter().any(|c| c.id == id));
        }
        if let Some(cmd) = activate {
            assert_eq!(palette.activate_selected().unwrap().command_id, cmd);
        }
    }
}

#[test]
fn deterministic_workspace_screenshots_png_and_ui_regions() {
    let doc = build_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    app.theme = UiThemeSpec::light();
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.canvas_dialogs.status_banner = Some("Visual Check Ready".into());
    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };

    app.execute_command("sheet.select", json!({ "sheet_id": "graph-donor" })).unwrap();
    app.sync_views();
    app.canvas_dialogs.on_sheet_selected(&doc, &"graph-donor".into());
    app.canvas_dialogs.open_command_palette();

    let (png_a, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).unwrap();
    let (png_b, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).unwrap();
    assert_eq!(png_a, png_b);
    assert_valid_png_with_dimensions(&png_a, 800, 520);

    let img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    let layout = measure_egui_regions_for_target(&app, target);
    for r in [layout.toolbar, layout.navigator, layout.canvas, layout.inspector, layout.status_bar, layout.modal_overlay.expect("modal")] {
        assert!(region_distinct_colors(&img.rgba, img.width_px, r, 8) >= 2, "blank region {r:?}");
    }

    app.canvas_dialogs.close_dialog();
    for (sheet, theme) in [
        ("data-xy", UiThemeSpec::dark()),
        ("res-anova", UiThemeSpec::dark()),
        ("layout-1", UiThemeSpec::dark()),
        ("layout-1", UiThemeSpec::high_contrast()),
    ] {
        app.theme = theme;
        app.execute_command("sheet.select", json!({ "sheet_id": sheet })).unwrap();
        if sheet == "data-xy" {
            app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
            app.grid.quick_stats = doc
                .data_sheet(&"data-xy".into())
                .map(|ds| SelectionQuickStats::from_data_sheet(ds, GridCellCoord::new(0, 0, 0), GridCellCoord::new(1, 0, 0)));
        }
        app.sync_views();
        app.canvas_dialogs.on_sheet_selected(&doc, &sheet.into());
        let (png, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).unwrap();
        assert_valid_png_with_dimensions(&png, 800, 520);
    }
    let hc_img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    let hc_layout = measure_egui_regions_for_target(&app, target);
    for r in [hc_layout.toolbar, hc_layout.navigator, hc_layout.canvas, hc_layout.inspector, hc_layout.status_bar] {
        assert!(region_distinct_colors(&hc_img.rgba, hc_img.width_px, r, 8) >= 2);
    }

    let mut live_app = UiApp::new();
    live_app.execute_command("sheet.createData", json!({ "sheet_id": "live-1", "title": "Live Assay" })).unwrap();
    live_app.execute_command("table.setCell", json!({ "sheet_id": "live-1", "row": 0, "col": 0, "value": 42.5 })).unwrap();
    live_app.execute_command("analyze.run", json!({})).unwrap();
    assert!(live_app.document().data_sheet(&"live-1".into()).is_some());
    let (live_png, _, _) = live_app
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 400, dpi: 150 }, ScreenshotRegionTarget::FullWindow)
        .unwrap();
    assert_valid_png_with_dimensions(&live_png, 640, 400);
}

#[test]
fn wave3_analyze_catalog_entries_and_survival_mv_fields_wire_cleanly() {
    for id in ["stats.multiple_row_t_tests", "stats.bland_altman", "stats.passing_bablok", "stats.multiple_imputation"] {
        let e = ANALYSIS_CATALOG.iter().find(|item| item.id == id).unwrap();
        assert!(!e.label.is_empty() && !e.keywords.is_empty() && !e.summary.is_empty() && default_analysis_kind_for_id(id).is_some());
    }
    assert!(matches!(default_analysis_kind_for_id("stats.multiple_row_t_tests"), Some(AnalysisKind::MultipleRowTTests(_))));
    assert!(matches!(default_analysis_kind_for_id("stats.bland_altman"), Some(AnalysisKind::BlandAltman(_))));
    assert!(matches!(default_analysis_kind_for_id("stats.passing_bablok"), Some(AnalysisKind::PassingBablok(_))));
    assert!(matches!(
        default_analysis_kind_for_id("stats.multiple_imputation"),
        Some(AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { .. }))
    ));
    assert!(matches!(
        default_analysis_kind_for_id("stats.survival_km"),
        Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column: None, mv_status_column: None, mv_group_column: None, .. }))
    ));

    let ds_grp =
        DataSheet::new("d-grp", "Row Series", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap())).unwrap();
    let mut dialog = AnalyzeDialogState::default();
    dialog.open_for_data_sheet(&ds_grp);
    assert_eq!(dialog.source_table_kind, Some(TableKind::Grouped));
    assert!(dialog.compatible_entries().iter().any(|e| e.id == "stats.multiple_row_t_tests"));

    let doc = build_project();
    dialog.open_for_data_sheet(doc.data_sheet(&"data-mv".into()).unwrap());
    dialog.selected_entry_id = Some("stats.multiple_imputation".into());
    dialog.set_response_column(0);
    assert!(
        matches!(dialog.build_analysis_kind().unwrap(), AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column: Some(0), predictor_columns, .. }) if predictor_columns == vec![1, 3])
    );

    for id in ["stats.bland_altman", "stats.passing_bablok"] {
        let kind = default_analysis_kind_for_id(id).unwrap();
        for tk in [TableKind::Column, TableKind::Xy, TableKind::MultipleVariables] {
            assert!(kind.is_compatible_with(tk));
        }
    }
}

#[test]
fn rtl_workspace_mirroring_and_font_diagnostics_behave_deterministically() {
    let doc = build_project();
    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };
    let make_app = |loc: &str| {
        let mut a = UiApp::from_session(Session::from_document(doc.clone()));
        assert_eq!(a.set_locale(loc), loc);
        a.nav.navigator_open = true;
        a.nav.inspector_open = true;
        a.sync_views();
        a
    };
    let mut ltr_app = make_app("en");
    let ltr_layout = measure_egui_regions_for_target(&ltr_app, target);
    assert!(ltr_layout.navigator.width > 0 && ltr_layout.canvas.width > 0 && ltr_layout.inspector.width > 0);
    let ltr_img = ltr_app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();

    for rtl in ["ar", "he"] {
        let mut rtl_app = make_app(rtl);
        let rtl_layout = measure_egui_regions_for_target(&rtl_app, target);
        assert!(rtl_layout.inspector.width > 0 && rtl_layout.navigator.width > 0 && rtl_layout.canvas.width > 0);
        let (a, _, _) = rtl_app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).unwrap();
        let (b, _, _) = rtl_app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).unwrap();
        assert_eq!(a, b);
        let rtl_img = rtl_app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
        assert_ne!(ltr_img.rgba, rtl_img.rgba);
        for r in [rtl_layout.toolbar, rtl_layout.navigator, rtl_layout.canvas, rtl_layout.inspector, rtl_layout.status_bar] {
            assert!(region_distinct_colors(&rtl_img.rgba, rtl_img.width_px, r, 8) >= 2);
        }
        assert!(font_diagnostics(rtl).rtl_supported);
    }
    assert!(
        !font_diagnostics("en").rtl_supported && !font_diagnostics("ja").rtl_supported && !font_diagnostics("ja").system_cjk_candidates.is_empty()
    );
}

#[test]
fn physical_ruler_chrome_toggles_and_renders_tick_bands() {
    let mut app = UiApp::from_session(Session::from_document(build_project()));
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).unwrap();
    app.sync_views();
    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };
    let layout = measure_egui_regions_for_target(&app, target);

    assert!(!app.canvas_dialogs.rulers_visible);
    let without = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    app.canvas_dialogs.rulers_visible = true;
    app.canvas_dialogs.ruler_unit = PhysicalUnit::Millimeters;
    app.canvas_dialogs.journal_preset = Some(ToolJournalWidthPreset::SingleColumn85Mm);
    let with_mm = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();

    for strip in [
        PixelRect { x: layout.canvas.x, y: layout.canvas.y, width: layout.canvas.width, height: 28 },
        PixelRect { x: layout.canvas.x, y: layout.canvas.y, width: 28, height: layout.canvas.height },
    ] {
        assert_ne!(copy_rect_rgba(&without.rgba, target.width_px, strip), copy_rect_rgba(&with_mm.rgba, target.width_px, strip));
        assert!(region_distinct_colors(&with_mm.rgba, target.width_px, strip, 8) >= 3);
    }

    app.canvas_dialogs.ruler_unit = PhysicalUnit::Inches;
    app.canvas_dialogs.journal_preset = Some(ToolJournalWidthPreset::LetterHalf);
    app.canvas_dialogs.pan_zoom.apply_pinch_zoom(1.5, [120.0, 80.0]);
    app.canvas_dialogs.canvas.zoom = 1.5;
    assert_ne!(with_mm.rgba, app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba);
    app.canvas_dialogs.reset_zoom_to_fit();
    assert!((app.canvas_dialogs.pan_zoom.zoom - 1.0).abs() < 1e-12);
}

#[test]
fn portfolio_gallery_and_active_pane_focus_indicator_render_distinctly() {
    let mut app = UiApp::from_session(Session::from_document(build_project()));
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };
    let layout = measure_egui_regions_for_target(&app, target);

    let closed = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    assert!(app.nav.toggle_gallery() && app.nav.gallery.cards.len() >= 4);
    let open = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    assert_ne!(copy_rect_rgba(&closed.rgba, target.width_px, layout.canvas), copy_rect_rgba(&open.rgba, target.width_px, layout.canvas));
    assert!(region_distinct_colors(&open.rgba, target.width_px, layout.canvas, 8) >= 4);

    let first_id = app.document().graph_sheets[0].id.clone();
    app.nav.select_gallery_card(&first_id, false);
    assert_ne!(open.rgba, app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba);

    let _ = app.nav.toggle_gallery();
    let mut frames: Vec<(ActiveWorkspacePane, Vec<u8>)> = Vec::new();
    for _ in 0..4 {
        frames.push((app.nav.active_pane, app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba));
        app.nav.cycle_focus_pane(false);
    }
    assert_eq!(frames.len(), 4);
}

#[test]
fn r10_10_pvalue_stack_catalog_analyze_and_edit_params_roundtrip() {
    let entry = ANALYSIS_CATALOG.iter().find(|i| i.id == "stats.pvalue_stack").expect("pvalue_stack entry");
    assert_eq!(entry.label, "Analyze a Stack of P Values");
    let def = default_analysis_kind_for_id("stats.pvalue_stack").unwrap();
    assert!(matches!(def, AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::BenjaminiHochberg, .. })));
    for tk in [TableKind::Column, TableKind::Xy, TableKind::Grouped, TableKind::MultipleVariables] {
        assert!(def.is_compatible_with(tk));
    }
    for tk in [TableKind::Contingency, TableKind::Survival, TableKind::PartsOfWhole, TableKind::Nested] {
        assert!(!def.is_compatible_with(tk));
    }

    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-pvals", "title": "P Stack", "kind": "Column", "rows": 5, "cols": 2 })).unwrap();
    app.execute_command(
        "table.paste",
        json!({ "sheet_id": "col-pvals", "start_row": 0, "start_col": 0, "text": "0.001\t0.0005\n0.014\t0.009\n0.040\n0.200\n0.900" }),
    )
    .unwrap();
    app.execute_command("sheet.select", json!({ "sheet_id": "col-pvals" })).unwrap();
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" })).unwrap();
    app.canvas_dialogs.analyze.select_catalog_entry("stats.pvalue_stack");
    app.canvas_dialogs.analyze.pvalue_stack_method = PValueStackMethod::Sidak;
    app.canvas_dialogs.analyze.pvalue_stack_threshold = 0.05;
    app.canvas_dialogs.analyze.pvalue_stack_column = Some(0);

    let analyze_txt = serde_json::to_string(&dispatch_gui_action(&mut app, "ui.inspect", json!({ "include_widgets": true })).unwrap()).unwrap();
    for wid in ["analyze.pvalue_stack.method", "analyze.pvalue_stack.threshold", "analyze.pvalue_stack.column"] {
        assert!(analyze_txt.contains(wid));
    }

    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" })).unwrap();
    let res_id = app.document().result_sheets.last().unwrap().id.clone();
    let created = app.document().result_sheet(&res_id).unwrap();
    assert_eq!(
        created.analysis.kind,
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Sidak, threshold: 0.05, column: Some(0) })
    );
    assert_eq!(result_scalar(created, "TotalDiscoveries").unwrap(), 1.0);

    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let edit_txt = serde_json::to_string(&dispatch_gui_action(&mut app, "ui.inspect", json!({ "include_widgets": true })).unwrap()).unwrap();
    for wid in ["edit.pvalue_stack.method", "edit.pvalue_stack.threshold", "edit.pvalue_stack.column"] {
        assert!(edit_txt.contains(wid));
    }

    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
        && let AnalysisKind::PValueStack(ps) = &mut spec.kind
    {
        ps.method = PValueStackMethod::TwoStageBky;
        ps.threshold = 0.10;
        ps.column = Some(1);
    }
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" })).unwrap();
    let updated = app.document().result_sheet(&res_id).unwrap();
    assert_eq!(
        updated.analysis.kind,
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::TwoStageBky, threshold: 0.10, column: Some(1) })
    );
    assert_eq!((result_scalar(updated, "TotalInputCount").unwrap(), result_scalar(updated, "TotalDiscoveries").unwrap()), (2.0, 2.0));
}
