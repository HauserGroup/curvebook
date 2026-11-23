//! Integration tests covering UI Parity dialogs, fuzzy Command Palette (`Cmd+K`), non-modal
//! graphing/layout inspectors, `Apply Style From...`, and deterministic software workspace PNG
//! screenshots (`ui.screenshot`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, Cell, CellCoord, ChecklistItem, ChecklistStatus, CiBound, ColorChannelMapping, ColumnDesignHint,
    ColumnTable, ConstantValue, ContingencyTable, DataSheet, FramePreset, GraphAnnotation, GraphKind, GraphSheet, GroupedTable, HookSource,
    InfoConstant, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, PValueStackMethod, PValueStackSpec, PairwiseComparisonRecord,
    PanelBadgeStyle, ParameterScope, ProjectDocument, RectPt, RemediationKind, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SizeChannelMapping, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, VariableRole, XyTable, extract_template_hooks,
};
use curvebook_engine::Session;
use curvebook_palette::{ColormapId, CvdKind, LineDashStyle, MarkerFillMode, PaletteId};
use curvebook_render::RenderTargetSpec;
use curvebook_tools::{GridCellCoord, JournalWidthPreset as ToolJournalWidthPreset, PhysicalUnit, SelectionQuickStats};
use curvebook_ui_egui::dialogs_canvas::{
    ANALYSIS_CATALOG, ActiveDialog, ActiveInspector, AnalyzeDialogState, AnalyzeDuplicateChoice, AxisTarget, CommandOrigin, CommandPaletteState,
    VariableRoleFilter, default_analysis_kind_for_id, variable_role_badge, variable_role_color,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::theme::{UiThemeSpec, font_diagnostics};
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{ScreenshotRegionTarget, UiApp, ViewportMetrics, dispatch_gui_action};
use serde_json::json;

#[derive(Debug, Clone, Copy)]
struct TestPixelRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct TestWorkspaceRegions {
    toolbar: TestPixelRect,
    navigator: TestPixelRect,
    canvas: TestPixelRect,
    inspector: TestPixelRect,
    status_bar: TestPixelRect,
    modal_overlay: Option<TestPixelRect>,
}

fn egui_rect_to_px(rect: egui::Rect, scale: f64, max_w: u32, max_h: u32) -> TestPixelRect {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let x = ((f64::from(rect.min.x.max(0.0)) * scale).floor() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y = ((f64::from(rect.min.y.max(0.0)) * scale).floor() as u32).min(max_h);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((f64::from(rect.max.x.max(0.0)) * scale).ceil() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((f64::from(rect.max.y.max(0.0)) * scale).ceil() as u32).min(max_h);
    TestPixelRect { x, y, width: r.saturating_sub(x), height: b.saturating_sub(y) }
}

fn measure_egui_regions(app: &UiApp, target: RenderTargetSpec) -> TestWorkspaceRegions {
    let scale = f64::from(target.dpi) / 96.0;
    let metrics = ViewportMetrics::from_physical(target.width_px, target.height_px, scale);
    let ctx = egui::Context::default();
    curvebook_ui_egui::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    ctx.options_mut(|o| o.zoom_factor = app.ui_scale());
    let mut views = app.egui_views.clone();
    let mut report = None;
    for _ in 0..6 {
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp / app.ui_scale()), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut settled = false;
        let mut out = ctx.run_ui(raw, |ui| {
            let rep = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            settled = !ui.ctx().will_discard();
            if settled {
                report = Some(rep);
            }
        });
        out.textures_delta.clear();
        if settled {
            break;
        }
    }
    let rep = report.expect("settled egui shell report");
    let to_px = |r: Option<egui::Rect>| r.map(|rect| egui_rect_to_px(rect, scale, target.width_px, target.height_px));
    let zero = TestPixelRect { x: 0, y: 0, width: 0, height: 0 };
    TestWorkspaceRegions {
        toolbar: to_px(rep.tier1_rect_pt).unwrap_or(zero),
        navigator: to_px(rep.leading_dock_rect_pt).unwrap_or(zero),
        canvas: to_px(rep.regions.canvas_rect_pt).unwrap_or(zero),
        inspector: to_px(rep.trailing_dock_rect_pt).unwrap_or(zero),
        status_bar: to_px(rep.footer_rect_pt).unwrap_or(zero),
        modal_overlay: to_px(rep.regions.dialog_rect_pt),
    }
}

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn assert_valid_png_with_dimensions(png: &[u8], expected_width: u32, expected_height: u32) {
    assert!(png.starts_with(PNG_SIGNATURE), "missing PNG magic header");
    assert!(png.len() >= 24, "PNG too short for IHDR chunk");
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(width, expected_width);
    assert_eq!(height, expected_height);
}

fn region_distinct_colors(rgba: &[u8], frame_width: u32, x: u32, y: u32, w: u32, h: u32) -> usize {
    let mut colors = Vec::new();
    for py in y..y.saturating_add(h) {
        for px in x..x.saturating_add(w) {
            let idx = ((py as usize) * (frame_width as usize) + (px as usize)) * 4;
            if let Some(&[r, g, b, a]) = rgba.get(idx..idx + 4) {
                let packed = u32::from_be_bytes([r, g, b, a]);
                if !colors.contains(&packed) {
                    colors.push(packed);
                    if colors.len() >= 8 {
                        return colors.len();
                    }
                }
            }
        }
    }
    colors.len()
}

fn build_rich_test_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Biochem Panel").expect("project");

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).expect("xy");
    xy.data_sets[0].title = String::from("10.5");
    xy.data_sets[1].title = String::from("Dose B");
    let mut ds_xy = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy)).expect("ds xy");
    ds_xy.set_cell(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::Cell::number(1.0)).expect("c0");
    ds_xy.set_cell(curvebook_doc::CellCoord::new(1, 0, 0), curvebook_doc::Cell::excluded_number(99.0)).expect("c1");
    doc.add_data_sheet(ds_xy).expect("add xy");

    let col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).expect("col");
    let ds_col = DataSheet::new("data-col", "Paired Treatment", TableArchetype::Column(col)).expect("ds col");
    doc.add_data_sheet(ds_col).expect("add col");

    let mut mv = MultipleVariablesTable::new(4, 5).expect("mv");
    mv.variables[0].title = String::from("Response");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Cohort");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].title = String::from("SampleID");
    mv.variables[2].role = VariableRole::Label;
    mv.variables[3].title = String::from("LogRatio");
    mv.variables[3].set_calculated_formula("log(A)");
    let ds_mv = DataSheet::new("data-mv", "Observational Cohort", TableArchetype::MultipleVariables(mv)).expect("ds mv");
    doc.add_data_sheet(ds_mv).expect("add mv");

    let cont = ContingencyTable::new(2, 2).expect("contingency");
    let ds_cont = DataSheet::new("data-cont", "Responder Counts", TableArchetype::Contingency(cont)).expect("ds cont");
    doc.add_data_sheet(ds_cont).expect("add cont");

    let mut info = InfoSheet::new_preseeded("info-1", "Lab Protocol").expect("info");
    info.constants.push(InfoConstant::new("Control_EC50", ConstantValue::Number(12.5), Some(String::from("nM"))).expect("const"));
    info.notes_markdown = String::from("Assay incubated at 37 C for 30 min.");
    doc.add_info_sheet(info).expect("add info");

    let anova_spec = AnalysisSpec::new("res-anova", "data-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec");
    let mut res = ResultSheet::new(anova_spec).expect("res");
    res.status = ResultStatus::UpToDate;
    res.tabs = vec![
        ResultTab {
            id: String::from("summary"),
            title: String::from("ANOVA Summary"),
            kind: ResultTabKind::AnovaTable,
            column_headers: vec![String::from("Value")],
            row_labels: vec![String::from("P value"), String::from("95% CI")],
            rows: vec![
                vec![ResultCell::PValue { raw: 0.0023, formatted: String::from("0.0023"), stars: String::from("**") }],
                vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(4.2), upper: CiBound::Open }],
            ],
            footnotes: Vec::new(),
        },
        ResultTab {
            id: String::from("comparisons"),
            title: String::from("Multiple Comparisons"),
            kind: ResultTabKind::MultipleComparisons,
            column_headers: vec![String::from("Adj. P")],
            row_labels: vec![String::from("Ctrl vs Treated")],
            rows: vec![vec![ResultCell::Number { value: 0.0041, prefix: None, units: None }]],
            footnotes: Vec::new(),
        },
    ];
    res.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 18.75, units: Some(String::from("nM")) });
    res.pairwise_comparisons = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("Control"),
            label_b: String::from("Treated"),
            estimate_diff: Some(8.4),
            raw_p_or_q: 0.0041,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 1,
            group_b: 2,
            label_a: String::from("Treated"),
            label_b: String::from("Washout"),
            estimate_diff: Some(1.1),
            raw_p_or_q: 0.48,
            is_fdr_q: false,
            significant: false,
        },
    ];
    res.checklist = vec![
        ChecklistItem {
            assumption: String::from("Normality of residuals (Shapiro-Wilk)"),
            status: ChecklistStatus::Pass,
            diagnostic_metric: Some(String::from("W = 0.97, p = 0.62")),
            explanation: String::from("Residuals are consistent with Gaussian distribution."),
            remediation: None,
        },
        ChecklistItem {
            assumption: String::from("Equal variances (Brown-Forsythe)"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(String::from("p = 0.041")),
            explanation: String::from("Consider Brown-Forsythe / Welch ANOVA."),
            remediation: Some(RemediationKind::WelchAnova),
        },
    ];
    doc.add_result_sheet(res).expect("add res");

    let g_col = GraphSheet::new_uninitialized_for_data("graph-col", doc.data_sheet(&"data-col".into()).expect("col")).expect("g_col");
    doc.add_graph_sheet(g_col).expect("add g_col");

    let mut g_donor = GraphSheet::new_uninitialized_for_data("graph-donor", doc.data_sheet(&"data-xy".into()).expect("xy")).expect("g_donor");
    g_donor.initialized = true;
    g_donor.title = String::from("Styled Donor");
    g_donor.spec.palette = PaletteId::PaulTolVibrant;
    g_donor.spec.legend.bordered = true;
    g_donor.spec.x_axis.stroke_width_pt = 2.0;
    doc.add_graph_sheet(g_donor).expect("add donor");

    let mut g_mv = GraphSheet::new_uninitialized_for_data("graph-mv", doc.data_sheet(&"data-mv".into()).expect("mv")).expect("g_mv");
    g_mv.initialized = true;
    g_mv.spec.kind = GraphKind::MultipleVariables;
    doc.add_graph_sheet(g_mv).expect("add g_mv");

    let layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    let layout = LayoutSheet::new("layout-1", "Figure 1 Composite", layout_spec).expect("layout");
    doc.add_layout_sheet(layout).expect("add layout");

    doc
}

#[test]
fn analyze_dialog_filters_by_table_archetype_search_and_role_badges() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    let col_id = "data-col".into();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &col_id));
    let compatible = app.canvas_dialogs.analyze.compatible_entries();
    assert!(compatible.iter().any(|item| item.id == "stats.two_group"));
    assert!(compatible.iter().any(|item| item.id == "stats.one_way_anova"));
    assert!(!compatible.iter().any(|item| item.id == "stats.survival_km"));
    assert!(!compatible.iter().any(|item| item.id == "fit.linear"));

    app.canvas_dialogs.analyze.search_query = String::from("welch");
    let welch_hits = app.canvas_dialogs.analyze.compatible_entries();
    assert!(!welch_hits.is_empty());
    assert!(app.canvas_dialogs.analyze.empty_feedback().is_none());

    app.canvas_dialogs.analyze.search_query = String::from("zzznonexistent");
    assert!(app.canvas_dialogs.analyze.compatible_entries().is_empty());
    let empty_msg = app.canvas_dialogs.analyze.empty_feedback().expect("empty search message");
    assert!(empty_msg.contains("zzznonexistent"));
    assert!(empty_msg.contains("Column"));

    let mv_id = "data-mv".into();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &mv_id));
    assert_eq!(app.canvas_dialogs.analyze.variable_candidates.len(), 4);

    app.canvas_dialogs.analyze.role_filter = VariableRoleFilter::NumericForModeling;
    let numeric_vars = app.canvas_dialogs.analyze.filtered_variables();
    assert_eq!(numeric_vars.len(), 2);
    assert!(numeric_vars.iter().all(|v| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated)));

    app.canvas_dialogs.analyze.role_filter = VariableRoleFilter::CategoricalOnly;
    let cat_vars = app.canvas_dialogs.analyze.filtered_variables();
    assert_eq!(cat_vars.len(), 1);
    assert_eq!(cat_vars[0].title, "Cohort");

    assert_eq!(variable_role_badge(VariableRole::Continuous), "NUM");
    assert_eq!(variable_role_badge(VariableRole::Categorical), "CAT");
    assert_eq!(variable_role_badge(VariableRole::Label), "LBL");
    assert_eq!(variable_role_badge(VariableRole::Calculated), "FX=");
    let light = UiThemeSpec::light();
    assert_ne!(variable_role_color(VariableRole::Continuous, &light), variable_role_color(VariableRole::Categorical, &light));

    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.glm"));
    app.canvas_dialogs.analyze.set_response_column(0);
    let built = app.canvas_dialogs.analyze.build_analysis_kind().expect("glm kind");
    match built {
        AnalysisKind::GeneralizedLinearModel(glm) => {
            assert_eq!(glm.response_column, 0);
            assert!(!glm.predictor_columns.contains(&2));
        }
        other => panic!("expected GLM kind, got {other:?}"),
    }
}

#[test]
fn duplicate_prompt_cmd_t_reentry_and_retarget_data_behave_cleanly() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    // Existing ANOVA on data-col -> triggers DuplicateAnalysisPrompt
    let col_id = "data-col".into();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &col_id));
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.one_way_anova"));
    assert!(app.canvas_dialogs.submit_analyze_or_prompt(&doc).is_none());
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));

    let modify_intent =
        app.canvas_dialogs.analyze.resolve_duplicate_choice(&AnalyzeDuplicateChoice::ModifyExisting("res-anova".into())).expect("modify intent");
    assert_eq!(modify_intent.command_id, "analysis.updateParams");
    assert_eq!(modify_intent.params["sheet_id"], "res-anova");

    let create_intent = app.canvas_dialogs.analyze.resolve_duplicate_choice(&AnalyzeDuplicateChoice::CreateNew).expect("create intent");
    assert_eq!(create_intent.command_id, "analysis.run");

    // Cmd+T parameter re-entry updates auto_title dynamically unless custom title is pinned
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &"res-anova".into()));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("Ordinary one-way ANOVA"));
    let mut rm_spec = OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }, ..Default::default() };
    assert!(app.canvas_dialogs.edit_params.set_kind(AnalysisKind::OneWayAnova(rm_spec.clone())));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("RM one-way ANOVA"));

    app.canvas_dialogs.edit_params.set_custom_title(Some(String::from("My Custom Cohort Title"))).expect("custom title");
    rm_spec.method = OneWayMethod::KruskalWallis;
    assert!(app.canvas_dialogs.edit_params.set_kind(AnalysisKind::OneWayAnova(rm_spec)));
    assert_eq!(app.canvas_dialogs.edit_params.preview_title(), Some("My Custom Cohort Title"));
    let update_intent = app.canvas_dialogs.edit_params.commit_intent().expect("commit update");
    assert_eq!(update_intent.command_id, "analysis.updateParams");

    // Change Data Analyzed checks compatibility across tables
    assert!(app.canvas_dialogs.open_change_data_analyzed(&doc, &"res-anova".into()));
    assert!(app.canvas_dialogs.change_data.select_target(&"data-cont".into()).is_err());
    assert!(app.canvas_dialogs.change_data.select_target(&"data-mv".into()).is_ok());
    let retarget_intent = app.canvas_dialogs.change_data.commit_intent().expect("retarget intent");
    assert_eq!(retarget_intent.command_id, "analysis.retargetData");
    assert_eq!(retarget_intent.params["source_sheet_id"], "data-mv");
}

#[test]
fn result_viewer_checklist_and_constant_hook_picker_work() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    app.canvas_dialogs.on_sheet_selected(&doc, &"res-anova".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::ResultDetails);
    let res = doc.result_sheet(&"res-anova".into()).expect("result");
    assert!(app.canvas_dialogs.result_view.select_tab_by_id(res, "comparisons"));
    assert_eq!(app.canvas_dialogs.result_view.active_tab(res).map(|t| t.id.as_str()), Some("comparisons"));

    assert_eq!(curvebook_ui_egui::dialogs_canvas::ResultSheetViewState::checklist_summary_counts(res), (1, 1, 0, 0));
    app.canvas_dialogs.result_view.checklist_filter = curvebook_ui_egui::dialogs_canvas::ChecklistSeverityFilter::WarningsAndViolations;
    assert_eq!(app.canvas_dialogs.result_view.visible_checklist(res).len(), 1);

    let open_ci = curvebook_ui_egui::dialogs_canvas::ResultSheetViewState::format_result_cell(
        &ResultCell::ConfidenceInterval { lower: CiBound::Finite(4.2), upper: CiBound::Open },
        PValueFormat::Apa,
    );
    assert_eq!(open_ci, "[4.2000, Unbounded]");

    app.canvas_dialogs.open_hook_picker(&doc, &"graph-donor".into(), Some(String::from("Bottom")));
    assert!(!app.canvas_dialogs.hook_picker.options.is_empty());
    app.canvas_dialogs.hook_picker.search_query = String::from("EC50");
    let ec50_hits = app.canvas_dialogs.hook_picker.filtered_options();
    assert!(ec50_hits.len() >= 2);
    for opt in &ec50_hits {
        if opt.template_token.starts_with("{{info:") || opt.template_token.starts_with("{{result:") {
            assert_eq!(extract_template_hooks(&opt.template_token).len(), 1);
        }
    }
    let hook_intent = app.canvas_dialogs.hook_picker.commit_intent().expect("hook intent");
    assert_eq!(hook_intent.command_id, "graph.addAnnotation");
    assert_eq!(hook_intent.params["sheet_id"], "graph-donor");
    assert!(hook_intent.params["text"].as_str().is_some_and(|t| t.starts_with("{{")));
}

#[test]
fn first_visit_graph_chooser_format_graph_axes_and_pairwise_work() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    // First-visit selection of uninitialized paired-column graph opens ChangeGraphType with Before-After first
    app.canvas_dialogs.on_sheet_selected(&doc, &"graph-col".into());
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { graph_sheet_id: "graph-col".into() });
    let recs = app.canvas_dialogs.change_graph_type.recommended_options();
    assert_eq!(recs.first().map(|o| o.id), Some("col.before_after"));
    let kind_intent = app.canvas_dialogs.change_graph_type.commit_intent().expect("graph.setKind");
    assert_eq!(kind_intent.command_id, "graph.setKind");

    // Non-modal FormatGraph + CVD preview + dashed error bars
    app.canvas_dialogs.format_graph.palette = PaletteId::PaulTolHighContrast;
    app.canvas_dialogs.format_graph.cvd_preview = Some(CvdKind::Deuteranopia);
    app.canvas_dialogs.format_graph.error_bar_spec.dash_style = LineDashStyle::Dashed;
    assert_ne!(app.canvas_dialogs.format_graph.preview_swatch(0), PaletteId::PaulTolHighContrast.color_at(0));
    let fg_intents = app.canvas_dialogs.format_graph.commit_intents();
    assert_eq!(fg_intents.len(), 2);
    assert_eq!(fg_intents[0].command_id, "graph.setSpec");
    assert_eq!(fg_intents[1].command_id, "graph.setSeries");

    // FormatAxes supports 1..=3 segments + nonlinear scales
    assert!(app.canvas_dialogs.format_axes.add_segment());
    assert!(app.canvas_dialogs.format_axes.set_segment_scale(1, AxisScale::Log10));
    assert!(app.canvas_dialogs.format_axes.set_segment_bounds(0, Some(0.0), Some(10.0)).is_ok());
    assert!(app.canvas_dialogs.format_axes.set_segment_bounds(0, Some(20.0), Some(10.0)).is_err());
    let axes_intent = app.canvas_dialogs.format_axes.commit_intent().expect("graph.setAxes");
    assert_eq!(axes_intent.command_id, "graph.setAxes");

    // Pairwise comparisons auto-add significant comparisons
    let added = app.canvas_dialogs.pairwise.auto_add_from_analysis(&doc);
    assert_eq!(added, 1);
    let cmp_intent = app.canvas_dialogs.pairwise.commit_intent().expect("comparison intent");
    assert_eq!(cmp_intent.command_id, "graph.addComparisonBrackets");
}

#[test]
fn bubble_plot_apply_style_and_page_layout_emit_commands() {
    let mut doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));

    // 5-channel BubblePlot inspector
    app.canvas_dialogs.on_sheet_selected(&doc, &"graph-mv".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::BubblePlot);
    app.canvas_dialogs.bubble_plot.set_color_continuous(0, ColormapId::Viridis, Some((0.0, 100.0)));
    app.canvas_dialogs.bubble_plot.set_size_variable(3, 3.0, 12.0);
    app.canvas_dialogs.bubble_plot.label_column = Some(2);
    app.canvas_dialogs.bubble_plot.show_convex_hulls = true;
    if let Some(gs_mv) = doc.graph_sheet_mut(&"graph-mv".into()) {
        app.canvas_dialogs.bubble_plot.apply_to_spec(&mut gs_mv.spec);
        let mv_enc = gs_mv.spec.multivariate.as_ref().expect("mv enc");
        assert!(matches!(mv_enc.color, ColorChannelMapping::Continuous { .. }));
        assert!(matches!(mv_enc.size, SizeChannelMapping::Variable { .. }));
        assert_eq!(mv_enc.label_column, Some(2));
        assert!(mv_enc.show_convex_hulls);
    }
    assert_eq!(app.canvas_dialogs.bubble_plot.commit_intent().expect("mv intent").command_id, "graph.setSpec");

    // Apply Style From... preserves target data bindings
    assert!(app.canvas_dialogs.open_apply_style_from(&doc, &"graph-col".into()));
    let styled_preview = app.canvas_dialogs.apply_style.preview_applied_spec(&doc).expect("style preview");
    assert_eq!(styled_preview.primary_source_sheet_id, "data-col".into());
    assert_eq!(styled_preview.palette, PaletteId::PaulTolVibrant);
    assert!(styled_preview.legend.bordered);
    assert_eq!(app.canvas_dialogs.apply_style.commit_intent().expect("style intent").command_id, "graph.applyStyleFrom");

    // Page Layout Composer grid & panel badges
    app.canvas_dialogs.on_sheet_selected(&doc, &"layout-1".into());
    assert_eq!(app.canvas_dialogs.active_inspector, ActiveInspector::PageLayout);
    app.canvas_dialogs.page_layout.panel_badges = PanelBadgeStyle::UpperCaseLetters;
    app.canvas_dialogs.page_layout.arrange_grid(&["graph-col".into(), "graph-donor".into()], 12.0, 8.0);
    app.canvas_dialogs.page_layout.add_text_callout(String::from("EC50 = {{result:res-anova:0:EC50}}"), RectPt::new(12.0, 200.0, 180.0, 24.0), 10.0);
    let layout_intents = app.canvas_dialogs.page_layout.commit_intents();
    assert_eq!(layout_intents.len(), 2);
    assert_eq!(layout_intents[0].command_id, "layout.setSpec");
    assert_eq!(layout_intents[1].command_id, "layout.alignOrigins");
}

#[test]
fn command_palette_fuzzy_matches_engine_and_ui_commands() {
    let mut palette = CommandPaletteState::default();
    palette.open();

    let all = palette.matches();
    assert!(all.iter().any(|c| c.origin == CommandOrigin::Engine));
    assert!(all.iter().any(|c| c.origin == CommandOrigin::Ui));

    palette.query = String::from("fitSheet");
    let ui_hits = palette.matches();
    assert_eq!(ui_hits.first().map(|c| c.id), Some("ui.zoom.fitSheet"));

    palette.query = String::from("exppng");
    let engine_hits = palette.matches();
    assert_eq!(engine_hits.first().map(|c| c.id), Some("export.png"));
    assert!(engine_hits.iter().any(|c| c.id == "graph.exportPng"));
    assert_eq!(palette.activate_selected().expect("selected").command_id, "export.png");

    palette.query = String::from("graphexppng");
    assert_eq!(palette.matches().first().map(|c| c.id), Some("graph.exportPng"));
    assert_eq!(palette.activate_selected().expect("selected graph export").command_id, "graph.exportPng");
}

#[test]
fn deterministic_workspace_screenshots_png_and_ui_regions() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    app.theme = UiThemeSpec::light();
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.canvas_dialogs.status_banner = Some(String::from("Visual Check Ready"));

    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };

    // 1. Graph canvas + FormatGraph inspector + open Command Palette
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-donor" })).expect("select graph");
    app.sync_views();
    app.canvas_dialogs.on_sheet_selected(&doc, &"graph-donor".into());
    app.canvas_dialogs.open_command_palette();

    let (png_a, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("png A");
    let (png_b, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("png B");
    assert_eq!(png_a, png_b, "screenshot rendering must be byte-for-byte deterministic");
    assert_valid_png_with_dimensions(&png_a, 800, 520);

    let img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("rgba");
    let layout = measure_egui_regions(&app, target);
    for region in [layout.toolbar, layout.navigator, layout.canvas, layout.inspector, layout.status_bar, layout.modal_overlay.expect("modal")] {
        assert!(
            region_distinct_colors(&img.rgba, img.width_px, region.x, region.y, region.width, region.height) >= 2,
            "UI region {region:?} should render distinct non-blank pixels"
        );
    }

    // 2. Data Sheet view with selection quick stats in Dark theme
    app.canvas_dialogs.close_dialog();
    app.theme = UiThemeSpec::dark();
    app.execute_command("sheet.select", json!({ "sheet_id": "data-xy" })).expect("select data");
    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.quick_stats = doc
        .data_sheet(&"data-xy".into())
        .map(|ds| SelectionQuickStats::from_data_sheet(ds, GridCellCoord::new(0, 0, 0), GridCellCoord::new(1, 0, 0)));
    app.sync_views();
    let (grid_png, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("grid png");
    assert_valid_png_with_dimensions(&grid_png, 800, 520);

    // 3. Result Sheet multi-tab + assumption checklist view
    app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" })).expect("select result");
    app.sync_views();
    app.canvas_dialogs.on_sheet_selected(&doc, &"res-anova".into());
    let (res_png, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("res png");
    assert_valid_png_with_dimensions(&res_png, 800, 520);

    // 4. Page Layout composer view & HighContrast WCAG theme check
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).expect("select layout");
    app.sync_views();
    app.canvas_dialogs.on_sheet_selected(&doc, &"layout-1".into());
    let (layout_png, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("layout png");
    assert_valid_png_with_dimensions(&layout_png, 800, 520);

    app.theme = UiThemeSpec::high_contrast();
    let (hc_png, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("high contrast png");
    assert_valid_png_with_dimensions(&hc_png, 800, 520);
    let hc_img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("high contrast rgba");
    let hc_layout = measure_egui_regions(&app, target);
    for region in [hc_layout.toolbar, hc_layout.navigator, hc_layout.canvas, hc_layout.inspector, hc_layout.status_bar] {
        assert!(region_distinct_colors(&hc_img.rgba, hc_img.width_px, region.x, region.y, region.width, region.height) >= 2);
    }

    // 5. Live Session command round-trip check
    let mut live_app = UiApp::new();
    live_app.execute_command("sheet.createData", json!({ "sheet_id": "live-1", "title": "Live Assay" })).expect("create data");
    live_app.execute_command("table.setCell", json!({ "sheet_id": "live-1", "row": 0, "col": 0, "value": 42.5 })).expect("set cell");
    live_app.execute_command("analyze.run", json!({})).expect("analyze run");
    assert!(live_app.document().data_sheet(&"live-1".into()).is_some());
    let (live_png, _, _) = live_app
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 400, dpi: 150 }, ScreenshotRegionTarget::FullWindow)
        .expect("live png");
    assert_valid_png_with_dimensions(&live_png, 640, 400);
}

fn build_live_roundtrip_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Live Roundtrip Panel").expect("project");

    let mut ds_xy =
        DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 6).expect("xy"))).expect("ds xy");
    let xy_pts = [(-8.0, 5.0), (-7.0, 14.0), (-6.0, 48.0), (-5.0, 82.0), (-4.0, 95.0), (-3.0, 98.0)];
    for (r, (x, y)) in xy_pts.iter().enumerate() {
        ds_xy.set_x_or_time_cell(r, 0, Cell::number(*x)).expect("x cell");
        ds_xy.set_cell(CellCoord::new(r, 0, 0), Cell::number(*y)).expect("y cell");
    }
    doc.add_data_sheet(ds_xy).expect("add xy");

    let mut ds_col = DataSheet::new(
        "data-col",
        "Paired Treatment",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).expect("col")),
    )
    .expect("ds col");
    let col_vals = [[10.0, 11.2, 9.8, 10.5, 11.0], [18.5, 19.2, 20.1, 18.8, 19.6], [11.0, 10.7, 11.5, 10.9, 11.2]];
    for (c, vals) in col_vals.iter().enumerate() {
        for (r, v) in vals.iter().enumerate() {
            ds_col.set_cell(CellCoord::new(r, c, 0), Cell::number(*v)).expect("col cell");
        }
    }
    doc.add_data_sheet(ds_col).expect("add col");

    let mut ds_col2 = DataSheet::new(
        "data-col2",
        "Validation Cohort",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("col2")),
    )
    .expect("ds col2");
    let col2_vals = [[5.1, 5.4, 4.9, 5.2], [9.1, 8.8, 9.4, 9.0], [6.0, 6.2, 5.9, 6.1]];
    for (c, vals) in col2_vals.iter().enumerate() {
        for (r, v) in vals.iter().enumerate() {
            ds_col2.set_cell(CellCoord::new(r, c, 0), Cell::number(*v)).expect("col2 cell");
        }
    }
    doc.add_data_sheet(ds_col2).expect("add col2");

    let mut mv = MultipleVariablesTable::new(4, 5).expect("mv");
    mv.variables[0].title = String::from("Response");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Cohort");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].title = String::from("SampleID");
    mv.variables[2].role = VariableRole::Label;
    mv.variables[3].title = String::from("Dose");
    mv.variables[3].role = VariableRole::Continuous;
    let mut ds_mv = DataSheet::new("data-mv", "Observational Cohort", TableArchetype::MultipleVariables(mv)).expect("ds mv");
    for r in 0..5 {
        ds_mv.set_cell(CellCoord::new(r, 0, 0), Cell::number((r as f64 + 1.0) * 10.0)).expect("mv r");
        ds_mv.set_cell(CellCoord::new(r, 1, 0), Cell::text(if r % 2 == 0 { "Ctrl" } else { "Drug" })).expect("mv cat");
        ds_mv.set_cell(CellCoord::new(r, 2, 0), Cell::text(format!("S{}", r + 1))).expect("mv lbl");
        ds_mv.set_cell(CellCoord::new(r, 3, 0), Cell::number(r as f64 + 1.0)).expect("mv d");
    }
    doc.add_data_sheet(ds_mv).expect("add mv");

    let mut info = InfoSheet::new_preseeded("info-1", "Lab Protocol").expect("info");
    info.constants.push(InfoConstant::new("Control_Bottom", ConstantValue::Number(4.5), Some(String::from("%"))).expect("const"));
    doc.add_info_sheet(info).expect("add info");

    let anova_kind = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
        ..OneWayAnovaSpec::default()
    });
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-anova", "data-col", anova_kind).expect("spec")).expect("res")).expect("add res");

    let fit_spec = AnalysisSpec::new("res-fit", "data-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")))
        .expect("fit spec");
    doc.add_result_sheet(ResultSheet::new(fit_spec).expect("fit res")).expect("add fit");

    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-col", doc.data_sheet(&"data-col".into()).expect("col")).expect("g_col"))
        .expect("add g_col");
    let mut g_donor = GraphSheet::new_uninitialized_for_data("graph-donor", doc.data_sheet(&"data-xy".into()).expect("xy")).expect("g_donor");
    g_donor.initialized = true;
    g_donor.spec.palette = PaletteId::PaulTolVibrant;
    g_donor.spec.legend.bordered = true;
    doc.add_graph_sheet(g_donor).expect("add donor");
    let mut g_mv = GraphSheet::new_uninitialized_for_data("graph-mv", doc.data_sheet(&"data-mv".into()).expect("mv")).expect("g_mv");
    g_mv.initialized = true;
    g_mv.spec.kind = GraphKind::MultipleVariables;
    doc.add_graph_sheet(g_mv).expect("add g_mv");

    let layout = LayoutSheet::new("layout-1", "Figure 1 Composite", LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm)).expect("layout");
    doc.add_layout_sheet(layout).expect("add layout");
    doc
}

#[test]
fn engine_command_round_trips_for_analysis_and_constant_hook_dialogs() {
    let mut app = UiApp::from_session(Session::from_document(build_live_roundtrip_project()));
    app.execute_command("analyze.run", json!({})).expect("initial analyze run");

    // 1. Analyze -> Create New via `analysis.run`
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &"data-xy".into()));
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.descriptive"));
    let create_intent = app.canvas_dialogs.analyze.build_create_new_intent().expect("create intent");
    let before_res = app.document().result_sheets.len();
    app.execute_command(&create_intent.command_id, create_intent.params).expect("execute analysis.run");
    assert_eq!(app.document().result_sheets.len(), before_res + 1);

    // 2. DuplicateAnalysisPrompt -> ModifyExisting via `analysis.updateParams`
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &"data-col".into()));
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.one_way_anova"));
    assert!(app.canvas_dialogs.submit_analyze_or_prompt(&snap).is_none());
    let mod_intent =
        app.canvas_dialogs.analyze.resolve_duplicate_choice(&AnalyzeDuplicateChoice::ModifyExisting("res-anova".into())).expect("mod intent");
    app.execute_command(&mod_intent.command_id, mod_intent.params).expect("execute modify existing");

    // 3. EditAnalysisParametersState (Cmd+T) -> `analysis.updateParams`
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &"res-anova".into()));
    let rm_spec = OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }, ..OneWayAnovaSpec::default() };
    assert!(app.canvas_dialogs.edit_params.set_kind(AnalysisKind::OneWayAnova(rm_spec)));
    app.canvas_dialogs.edit_params.set_custom_title(Some(String::from("RM Assay Title"))).expect("custom title");
    let edit_intent = app.canvas_dialogs.edit_params.commit_intent().expect("edit params intent");
    app.execute_command(&edit_intent.command_id, edit_intent.params).expect("execute edit params");
    assert_eq!(app.document().result_sheet(&"res-anova".into()).expect("res-anova").analysis.effective_title(), "RM Assay Title");

    // 4. ChangeDataAnalyzedState -> `analysis.retargetData`
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_change_data_analyzed(&snap, &"res-anova".into()));
    app.canvas_dialogs.change_data.select_target(&"data-col2".into()).expect("select data-col2");
    let retarget_intent = app.canvas_dialogs.change_data.commit_intent().expect("retarget intent");
    app.execute_command(&retarget_intent.command_id, retarget_intent.params).expect("execute retarget");
    assert_eq!(app.document().result_sheet(&"res-anova".into()).expect("res-anova").analysis.source_sheet_id, "data-col2".into());

    // 5. ConstantHookPickerState -> `analysis.updateParams` (HookedConstant), `graph.addAnnotation`, `layout.addPanel`
    let snap = app.document().clone();
    app.canvas_dialogs.open_hook_picker(&snap, &"res-fit".into(), Some(String::from("Bottom")));
    let fit_hook_intent = app.canvas_dialogs.hook_picker.commit_intent().expect("fit hook intent");
    app.execute_command(&fit_hook_intent.command_id, fit_hook_intent.params).expect("execute fit hook");
    let fit_res = app.document().result_sheet(&"res-fit".into()).expect("res-fit");
    if let AnalysisKind::NonlinearRegression(nl) = &fit_res.analysis.kind {
        assert!(
            nl.parameter_rules
                .iter()
                .any(|r| r.name == "Bottom" && matches!(r.scope, ParameterScope::HookedConstant(HookSource::InfoConstant { .. })))
        );
    } else {
        panic!("expected nonlinear regression");
    }

    app.canvas_dialogs.open_hook_picker(&snap, &"graph-donor".into(), None);
    let graph_hook = app.canvas_dialogs.hook_picker.commit_intent().expect("graph hook");
    app.execute_command(&graph_hook.command_id, graph_hook.params).expect("execute graph.addAnnotation");
    assert!(
        app.document()
            .graph_sheet(&"graph-donor".into())
            .expect("donor")
            .spec
            .annotations
            .iter()
            .any(|a| matches!(a, GraphAnnotation::TextBox { .. }))
    );

    app.canvas_dialogs.open_hook_picker(&snap, &"layout-1".into(), None);
    let layout_hook = app.canvas_dialogs.hook_picker.commit_intent().expect("layout hook");
    app.execute_command(&layout_hook.command_id, layout_hook.params).expect("execute layout.addPanel");
    assert!(
        app.document().layout_sheet(&"layout-1".into()).expect("layout").spec.items.iter().any(|item| matches!(item, LayoutItem::TextCallout { .. }))
    );
}

#[test]
fn engine_command_round_trips_for_graph_and_layout_inspectors() {
    let mut app = UiApp::from_session(Session::from_document(build_live_roundtrip_project()));
    app.execute_command("analyze.run", json!({})).expect("initial analyze run");

    // 1. ChangeGraphTypeState -> `graph.setKind`
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-col".into());
    let kind_intent = app.canvas_dialogs.change_graph_type.commit_intent().expect("graph.setKind");
    app.execute_command(&kind_intent.command_id, kind_intent.params).expect("execute graph.setKind");
    assert!(app.document().graph_sheet(&"graph-col".into()).expect("g_col").initialized);

    // 2. PairwiseComparisonsState -> `graph.addComparisonBrackets` + `graph.setCompactLetterDisplay`
    let snap = app.document().clone();
    app.canvas_dialogs.pairwise.sync_from_graph(snap.graph_sheet(&"graph-col".into()).expect("g_col"), &snap);
    assert!(app.canvas_dialogs.pairwise.auto_add_from_analysis(&snap) >= 1);
    let cmp_intent = app.canvas_dialogs.pairwise.commit_intent().expect("comparison intent");
    app.execute_command(&cmp_intent.command_id, cmp_intent.params).expect("execute comparison brackets");
    assert!(!app.document().graph_sheet(&"graph-col".into()).expect("g_col").spec.comparisons.is_empty());

    app.canvas_dialogs.pairwise.toggle_cld_enabled();
    assert!(app.canvas_dialogs.pairwise.cld_enabled);
    let cld_intent = app.canvas_dialogs.pairwise.cld_commit_intent().expect("cld intent");
    assert_eq!(cld_intent.command_id, "graph.setCompactLetterDisplay");
    app.execute_command(&cld_intent.command_id, cld_intent.params).expect("execute compact letter display");
    let g_col_after = app.document().graph_sheet(&"graph-col".into()).expect("g_col after cld");
    assert!(g_col_after.spec.compact_letter_display.is_some());
    assert_ne!(g_col_after.spec.pairwise_display_mode, curvebook_doc::PairwiseDisplayMode::BracketsOnly);

    let snap2 = app.document().clone();
    app.canvas_dialogs.pairwise.sync_from_graph(snap2.graph_sheet(&"graph-col".into()).expect("g_col snap2"), &snap2);
    assert!(app.canvas_dialogs.pairwise.cld_enabled);
    app.canvas_dialogs.pairwise.toggle_cld_enabled();
    assert!(!app.canvas_dialogs.pairwise.cld_enabled);
    let disable_intent = app.canvas_dialogs.pairwise.cld_commit_intent().expect("disable cld intent");
    app.execute_command(&disable_intent.command_id, disable_intent.params).expect("disable cld");
    assert!(app.document().graph_sheet(&"graph-col".into()).expect("g_col disabled").spec.compact_letter_display.is_none());

    // 3. FormatGraphState -> `graph.setSpec` + `graph.setSeries`
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-donor".into());
    app.canvas_dialogs.format_graph.palette = PaletteId::PaulTolHighContrast;
    app.canvas_dialogs.format_graph.frame_preset = FramePreset::PlainBoxed;
    app.canvas_dialogs.format_graph.legend_bordered = false;
    app.canvas_dialogs.format_graph.marker_size_pt = 8.0;
    app.canvas_dialogs.format_graph.marker_fill_mode = MarkerFillMode::Open;
    app.canvas_dialogs.format_graph.line_dash = LineDashStyle::Dotted;
    app.canvas_dialogs.format_graph.error_bar_spec.dash_style = LineDashStyle::Dashed;
    for intent in app.canvas_dialogs.format_graph.commit_intents() {
        app.execute_command(&intent.command_id, intent.params).expect("execute format graph");
    }
    let donor = app.document().graph_sheet(&"graph-donor".into()).expect("donor");
    assert_eq!(donor.spec.palette, PaletteId::PaulTolHighContrast);
    assert_eq!(donor.spec.frame_preset, FramePreset::PlainBoxed);
    assert!(!donor.spec.legend.bordered);

    // 4. FormatAxesState -> `graph.setAxes` (Y broken axis + Y2 dual axis)
    assert!(app.canvas_dialogs.format_axes.add_segment());
    assert!(app.canvas_dialogs.format_axes.set_segment_scale(1, AxisScale::Log10));
    let y_axes_intent = app.canvas_dialogs.format_axes.commit_intent().expect("y axes intent");
    app.execute_command(&y_axes_intent.command_id, y_axes_intent.params).expect("execute Y axes");
    assert_eq!(app.document().graph_sheet(&"graph-donor".into()).expect("donor").spec.y_axis.segments.len(), 2);

    let snap = app.document().clone();
    app.canvas_dialogs.format_axes.sync_from_graph(snap.graph_sheet(&"graph-donor".into()).expect("donor"), AxisTarget::Y2);
    app.canvas_dialogs.format_axes.enable_y2 = true;
    let y2_axes_intent = app.canvas_dialogs.format_axes.commit_intent().expect("y2 axes intent");
    app.execute_command(&y2_axes_intent.command_id, y2_axes_intent.params).expect("execute Y2 axes");
    assert!(app.document().graph_sheet(&"graph-donor".into()).expect("donor").spec.y2_axis.is_some());

    // 5. BubblePlotInspectorState -> `graph.setSpec`
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"graph-mv".into());
    app.canvas_dialogs.bubble_plot.set_color_continuous(0, ColormapId::Viridis, Some((0.0, 100.0)));
    app.canvas_dialogs.bubble_plot.set_size_variable(3, 3.0, 12.0);
    app.canvas_dialogs.bubble_plot.label_column = Some(2);
    app.canvas_dialogs.bubble_plot.show_convex_hulls = true;
    let mv_intent = app.canvas_dialogs.bubble_plot.commit_intent().expect("mv intent");
    app.execute_command(&mv_intent.command_id, mv_intent.params).expect("execute bubble plot");
    let mv_enc = app.document().graph_sheet(&"graph-mv".into()).expect("mv").spec.multivariate.as_ref().expect("mv enc");
    assert_eq!(mv_enc.label_column, Some(2));
    assert!(mv_enc.show_convex_hulls);

    // 6. ApplyStyleFromState -> `graph.applyStyleFrom`
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_apply_style_from(&snap, &"graph-col".into()));
    let style_intent = app.canvas_dialogs.apply_style.commit_intent().expect("style intent");
    app.execute_command(&style_intent.command_id, style_intent.params).expect("execute applyStyleFrom");
    let restyled_col = app.document().graph_sheet(&"graph-col".into()).expect("g_col");
    assert_eq!(restyled_col.spec.primary_source_sheet_id, "data-col".into());
    assert_eq!(restyled_col.spec.palette, PaletteId::PaulTolHighContrast);

    // 7. PageLayoutComposerState -> `layout.setSpec` + `layout.alignOrigins`
    let snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&snap, &"layout-1".into());
    app.canvas_dialogs.page_layout.panel_badges = PanelBadgeStyle::UpperCaseLetters;
    app.canvas_dialogs.page_layout.arrange_grid(&["graph-col".into(), "graph-donor".into()], 12.0, 8.0);
    app.canvas_dialogs.page_layout.add_text_callout(String::from("Mean = {{result:res-anova:0:R2}}"), RectPt::new(12.0, 200.0, 180.0, 24.0), 10.0);
    for intent in app.canvas_dialogs.page_layout.commit_intents() {
        app.execute_command(&intent.command_id, intent.params).expect("execute layout intent");
    }
    let layout = app.document().layout_sheet(&"layout-1".into()).expect("layout");
    assert_eq!(layout.spec.panel_badges, PanelBadgeStyle::UpperCaseLetters);
    assert!(layout.spec.align_plot_origins);
    assert!(layout.spec.items.len() >= 3);
}

fn copy_rect_rgba(rgba: &[u8], frame_width: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for py in y..y.saturating_add(h) {
        let start = ((py as usize) * (frame_width as usize) + (x as usize)) * 4;
        let end = start + (w as usize) * 4;
        if let Some(slice) = rgba.get(start..end) {
            out.extend_from_slice(slice);
        }
    }
    out
}

#[test]
fn wave3_analyze_catalog_entries_and_survival_mv_fields_wire_cleanly() {
    let new_ids = ["stats.multiple_row_t_tests", "stats.bland_altman", "stats.passing_bablok", "stats.multiple_imputation"];
    for id in new_ids {
        let entry = ANALYSIS_CATALOG.iter().find(|item| item.id == id).expect("catalog item present");
        assert!(!entry.label.is_empty());
        assert!(!entry.keywords.is_empty());
        assert!(!entry.summary.is_empty());
        assert!(default_analysis_kind_for_id(id).is_some(), "missing default kind for {id}");
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

    let grouped = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 4).expect("grouped");
    let ds_grouped = DataSheet::new("d-grp", "Row Series", TableArchetype::Grouped(grouped)).expect("ds grouped");
    let mut dialog = AnalyzeDialogState::default();
    dialog.open_for_data_sheet(&ds_grouped);
    assert_eq!(dialog.source_table_kind, Some(TableKind::Grouped));
    assert!(dialog.compatible_entries().iter().any(|e| e.id == "stats.multiple_row_t_tests"));

    let doc = build_rich_test_project();
    let ds_mv = doc.data_sheet(&"data-mv".into()).expect("mv sheet");
    dialog.open_for_data_sheet(ds_mv);
    dialog.selected_entry_id = Some(String::from("stats.multiple_imputation"));
    dialog.set_response_column(0);
    let kind = dialog.build_analysis_kind().expect("imputation kind");
    match kind {
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. }) => {
            assert_eq!(response_column, Some(0));
            assert_eq!(predictor_columns, vec![1, 3]);
        }
        other => panic!("unexpected analysis kind: {other:?}"),
    }

    for col_id in ["stats.bland_altman", "stats.passing_bablok"] {
        let kind = default_analysis_kind_for_id(col_id).expect("col/xy compatible");
        assert!(kind.is_compatible_with(TableKind::Column));
        assert!(kind.is_compatible_with(TableKind::Xy));
        assert!(kind.is_compatible_with(TableKind::MultipleVariables));
    }
}

#[test]
fn rtl_workspace_mirroring_and_font_diagnostics_behave_deterministically() {
    let doc = build_rich_test_project();
    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };

    let mut ltr_app = UiApp::from_session(Session::from_document(doc.clone()));
    ltr_app.locale = String::from("en");
    ltr_app.nav.navigator_open = true;
    ltr_app.nav.inspector_open = true;
    ltr_app.sync_views();
    let ltr_layout = measure_egui_regions(&ltr_app, target);
    assert!(ltr_layout.navigator.width > 0 && ltr_layout.canvas.width > 0 && ltr_layout.inspector.width > 0);
    let ltr_img = ltr_app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("ltr rgba");

    for rtl_locale in ["ar", "he"] {
        let mut rtl_app = UiApp::from_session(Session::from_document(doc.clone()));
        assert_eq!(rtl_app.set_locale(rtl_locale), rtl_locale);
        rtl_app.nav.navigator_open = true;
        rtl_app.nav.inspector_open = true;
        rtl_app.sync_views();

        let rtl_layout = measure_egui_regions(&rtl_app, target);
        assert!(rtl_layout.inspector.width > 0 && rtl_layout.navigator.width > 0 && rtl_layout.canvas.width > 0);

        let (png_a, _, _) = rtl_app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("rtl png a");
        let (png_b, _, _) = rtl_app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).expect("rtl png b");
        assert_eq!(png_a, png_b, "RTL render for {rtl_locale} must be byte-for-byte deterministic");

        let rtl_img = rtl_app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("rtl rgba");
        assert_ne!(ltr_img.rgba, rtl_img.rgba, "LTR vs RTL chrome/glyphs must differ for {rtl_locale}");
        for region in [rtl_layout.toolbar, rtl_layout.navigator, rtl_layout.canvas, rtl_layout.inspector, rtl_layout.status_bar] {
            assert!(
                region_distinct_colors(&rtl_img.rgba, rtl_img.width_px, region.x, region.y, region.width, region.height) >= 2,
                "RTL region {region:?} should render distinct content"
            );
        }

        let diag = font_diagnostics(rtl_locale);
        assert!(diag.rtl_supported);
    }

    let en_diag = font_diagnostics("en");
    assert!(!en_diag.rtl_supported);
    let ja_diag = font_diagnostics("ja");
    assert!(!ja_diag.system_cjk_candidates.is_empty());
    assert!(!ja_diag.rtl_supported);
}

#[test]
fn physical_ruler_chrome_toggles_and_renders_tick_bands() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).expect("select layout");
    app.sync_views();

    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };
    let layout = measure_egui_regions(&app, target);

    assert!(!app.canvas_dialogs.rulers_visible);
    let without_rulers = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("no rulers");

    app.canvas_dialogs.rulers_visible = true;
    app.canvas_dialogs.ruler_unit = PhysicalUnit::Millimeters;
    app.canvas_dialogs.journal_preset = Some(ToolJournalWidthPreset::SingleColumn85Mm);
    let with_mm = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("with mm rulers");

    let top_strip_off = copy_rect_rgba(&without_rulers.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, 28);
    let top_strip_on = copy_rect_rgba(&with_mm.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, 28);
    assert_ne!(top_strip_off, top_strip_on);
    assert!(region_distinct_colors(&with_mm.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, 28) >= 3);

    let left_strip_off = copy_rect_rgba(&without_rulers.rgba, target.width_px, layout.canvas.x, layout.canvas.y, 28, layout.canvas.height);
    let left_strip_on = copy_rect_rgba(&with_mm.rgba, target.width_px, layout.canvas.x, layout.canvas.y, 28, layout.canvas.height);
    assert_ne!(left_strip_off, left_strip_on);
    assert!(region_distinct_colors(&with_mm.rgba, target.width_px, layout.canvas.x, layout.canvas.y, 28, layout.canvas.height) >= 3);

    app.canvas_dialogs.ruler_unit = PhysicalUnit::Inches;
    app.canvas_dialogs.journal_preset = Some(ToolJournalWidthPreset::LetterHalf);
    app.canvas_dialogs.pan_zoom.apply_pinch_zoom(1.5, [120.0, 80.0]);
    app.canvas_dialogs.canvas.zoom = 1.5;
    let with_in = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("with inches rulers");
    assert_ne!(with_mm.rgba, with_in.rgba);

    app.canvas_dialogs.reset_zoom_to_fit();
    assert!((app.canvas_dialogs.pan_zoom.zoom - 1.0).abs() < 1e-12);
}

#[test]
fn portfolio_gallery_and_active_pane_focus_indicator_render_distinctly() {
    let doc = build_rich_test_project();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();

    let target = RenderTargetSpec { width_px: 800, height_px: 520, dpi: 150 };
    let layout = measure_egui_regions(&app, target);

    let closed_img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("gallery closed");
    assert!(app.nav.toggle_gallery());
    assert!(app.nav.gallery.cards.len() >= 4);

    let open_img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("gallery open");
    assert_ne!(
        copy_rect_rgba(&closed_img.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, layout.canvas.height),
        copy_rect_rgba(&open_img.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, layout.canvas.height)
    );
    assert!(
        region_distinct_colors(&open_img.rgba, target.width_px, layout.canvas.x, layout.canvas.y, layout.canvas.width, layout.canvas.height) >= 4
    );

    let first_id = app.document().graph_sheets[0].id.clone();
    app.nav.select_gallery_card(&first_id, false);
    let selected_img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("gallery selected");
    assert_ne!(open_img.rgba, selected_img.rgba);

    let _ = app.nav.toggle_gallery();
    let mut focus_frames: Vec<(ActiveWorkspacePane, Vec<u8>)> = Vec::new();
    for _ in 0..4 {
        let pane = app.nav.active_pane;
        let frame = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("focus frame");
        focus_frames.push((pane, frame.rgba));
        app.nav.cycle_focus_pane(false);
    }
    assert_eq!(focus_frames.len(), 4);
}

#[test]
fn r10_10_pvalue_stack_catalog_analyze_and_edit_params_roundtrip() {
    // 1. Static catalog registration & compatibility matrix.
    let entry = ANALYSIS_CATALOG.iter().find(|item| item.id == "stats.pvalue_stack").expect("pvalue_stack catalog entry");
    assert_eq!(entry.label, "Analyze a Stack of P Values");
    let default_kind = default_analysis_kind_for_id("stats.pvalue_stack").expect("default pvalue_stack kind");
    assert!(matches!(default_kind, AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::BenjaminiHochberg, .. })));
    for compat in [TableKind::Column, TableKind::Xy, TableKind::Grouped, TableKind::MultipleVariables] {
        assert!(default_kind.is_compatible_with(compat), "expected {compat:?} compatibility");
    }
    for incompat in [TableKind::Contingency, TableKind::Survival, TableKind::PartsOfWhole, TableKind::Nested] {
        assert!(!default_kind.is_compatible_with(incompat), "expected {incompat:?} incompatibility");
    }

    // 2. End-to-end GUI flow: Analyze... -> inspect controls -> run -> EditAnalysisParameters (Cmd+T) -> update.
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-pvals", "title": "P Stack", "kind": "Column", "rows": 5, "cols": 2 }))
        .expect("create Column sheet");
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "col-pvals",
            "edits": [
                { "row": 0, "col": 0, "value": 0.001 },
                { "row": 1, "col": 0, "value": 0.014 },
                { "row": 2, "col": 0, "value": 0.040 },
                { "row": 3, "col": 0, "value": 0.200 },
                { "row": 4, "col": 0, "value": 0.900 },
                { "row": 0, "col": 1, "value": 0.0005 },
                { "row": 1, "col": 1, "value": 0.009 }
            ]
        }),
    )
    .expect("seed P values");

    app.execute_command("sheet.select", json!({ "sheet_id": "col-pvals" })).expect("select Column sheet");
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" })).expect("open Analyze dialog");
    dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "entry_id": "stats.pvalue_stack",
                "method": "Šídák (1967)",
                "threshold": 0.05,
                "column": 0
            }
        }),
    )
    .expect("patch Analyze dialog fields");

    let analyze_tree = dispatch_gui_action(&mut app, "ui.inspect", json!({ "include_widgets": true })).expect("inspect Analyze dialog");
    let analyze_txt = serde_json::to_string(&analyze_tree).expect("serialize inspect tree");
    assert!(analyze_txt.contains("analyze.pvalue_stack.method"));
    assert!(analyze_txt.contains("analyze.pvalue_stack.threshold"));
    assert!(analyze_txt.contains("analyze.pvalue_stack.column"));

    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok" })).expect("commit Analyze dialog");
    let res_id = app.document().result_sheets.last().expect("created ResultSheet").id.clone();
    let created = app.document().result_sheet(&res_id).expect("result sheet present");
    assert_eq!(
        created.analysis.kind,
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Sidak, threshold: 0.05, column: Some(0) })
    );
    assert_eq!(created.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(1.0));

    // Open EditAnalysisParameters for the active PValueStack result sheet and update via `multiplicity_method`.
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let edit_tree = dispatch_gui_action(&mut app, "ui.inspect", json!({ "include_widgets": true })).expect("inspect Edit Params dialog");
    let edit_txt = serde_json::to_string(&edit_tree).expect("serialize edit inspect tree");
    assert!(edit_txt.contains("edit.pvalue_stack.method"));
    assert!(edit_txt.contains("edit.pvalue_stack.threshold"));
    assert!(edit_txt.contains("edit.pvalue_stack.column"));

    dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "multiplicity_method": "Benjamini-Krieger-Yekutieli",
                "threshold": 0.10,
                "column": 1
            }
        }),
    )
    .expect("commit EditAnalysisParameters");

    let updated = app.document().result_sheet(&res_id).expect("updated result sheet");
    assert_eq!(
        updated.analysis.kind,
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::TwoStageBky, threshold: 0.10, column: Some(1) })
    );
    assert_eq!(updated.scalars.iter().find(|s| s.key == "TotalInputCount").map(|s| s.value), Some(2.0));
    assert_eq!(updated.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(2.0));
}
