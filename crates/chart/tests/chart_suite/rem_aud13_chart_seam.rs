//! Wave 2 chart-seam remediation regression tests covering `AUD-13-05`, `AUD-13-06`, `AUD-13-07`,
//! `AUD-13-16`, `AUD-13-17`, and `AUD-13-18`.

use curvebook_chart::equation_box::{
    ModelEquationParams, is_parametric_equation_analysis, render_model_equation_annotation, resolve_model_equation_tex,
};
use curvebook_chart::scales::resolve_template_hooks;
use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph, extract_fit_summary_entries};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ContingencyTable, CurveSamplePoint, DataSheet,
    DemingErrorSpec, DiagnosticPlotKind, EquationDisplayMode, ErrorCode, ErrorPropagationMode, FittedCurveOverlay, GlmFamily, GlmSpec,
    GraphAnnotation, GraphKind, GraphSheet, GroupedTable, IdentifiabilityMode, LinearRegressionSpec, MultipleVariablesTable,
    MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, ParameterRule, ParameterScope, PartsOfWholeTable, ProjectDocument, RectPt,
    RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SimpleLogisticSpec,
    SizeChannelMapping, SplineLowessSpec, SubcolumnMode, SurvivalTable, TableArchetype, TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_palette::Rgba8;
use curvebook_stats::run_analysis;

fn collect_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn empty_scene() -> ChartScene {
    ChartScene::new("rem-aud13", "Test", 480.0, 320.0, None)
}

#[test]
fn aud_13_05_identify_unstable_parameters_never_leaks_to_equation_box_hooks_or_fit_summary() {
    // 1. Live Wave 1 fit: collinear 4PL / linear-overparameterized model under IdentifyUnstableParameters.
    //    Use custom DSL `Y = A * X + B + C * X` where A and C are perfectly collinear, so Wave 1
    //    flags an unstable parameter and omits it from `res.scalars`.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy");
    xy.data_sets[0].title = String::from("Assay");
    for r in 0..8 {
        let x = (r + 1) as f64;
        xy.set_x_cell(r, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(3.0 * x + 5.0);
    }
    let ds = DataSheet::new("xy-collinear", "Collinear", TableArchetype::Xy(xy)).expect("ds");
    let mut nl = NonlinearFitSpec::new("custom_collinear");
    nl.custom_dsl = Some(String::from("Y = A * X + B + C * X"));
    nl.identifiability_mode = Some(IdentifiabilityMode::IdentifyUnstableParameters);
    nl.identify_unstable_parameters = true;
    let spec = AnalysisSpec::new("res-unstable-live", "xy-collinear", AnalysisKind::NonlinearRegression(nl)).expect("spec");
    let res_live = execute_analysis(&spec, &ds, &HookContext::default()).expect("execute_analysis");
    assert_eq!(res_live.status, ResultStatus::UpToDate);

    // At least one parameter must be masked as "Unstable" in the parameter tab and omitted from scalars.
    let param_tab = res_live.tabs.iter().find(|t| t.id == "nonlinear_parameters").expect("nonlinear_parameters tab");
    let unstable_names: Vec<String> = param_tab
        .row_labels
        .iter()
        .zip(param_tab.rows.iter())
        .filter(|(_, row)| matches!(row.first(), Some(ResultCell::Text(s)) if s.eq_ignore_ascii_case("Unstable")))
        .map(|(lbl, _)| lbl.strip_suffix("(Unstable)").unwrap_or(lbl).trim().to_owned())
        .collect();
    assert!(!unstable_names.is_empty(), "expected at least one unstable parameter in collinear fit: {:?}", param_tab.row_labels);

    let mut doc = ProjectDocument::new("AUD13-05").expect("doc");
    doc.add_data_sheet(ds.clone()).expect("add ds");
    doc.add_result_sheet(res_live).expect("add res_live");

    // Live equation box substitutes stable params and leaves unstable params symbolic (never crashes / leaks).
    let live_tex = resolve_model_equation_tex(&doc, &SheetId::new("res-unstable-live"), 0, EquationDisplayMode::Substituted)
        .expect("expected live TeX for partially stable collinear fit");
    for uname in &unstable_names {
        let hook_in = format!("{{{{result:res-unstable-live:0:{uname}}}}}");
        assert_eq!(
            resolve_template_hooks(&hook_in, &doc, Some(&ds.id)),
            hook_in,
            "omitted unstable parameter `{uname}` must stay unexpanded in template hooks"
        );
        assert!(live_tex.contains(uname.as_str()), "omitted unstable param `{uname}` should remain symbolic in `{live_tex}`");
    }

    // 2. Legacy pre-Wave-1 saved ResultSheet defense-in-depth: `Top` is marked "Unstable" in the
    //    parameter tab while a raw non-identifiable float `1.234e7` still sits in `res.scalars`.
    let mut nl_4pl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_4pl.identifiability_mode = Some(IdentifiabilityMode::IdentifyUnstableParameters);
    nl_4pl.identify_unstable_parameters = true;
    let legacy_spec = AnalysisSpec::new("res-unstable-legacy", "xy-collinear", AnalysisKind::NonlinearRegression(nl_4pl)).expect("legacy spec");
    let mut legacy_res = ResultSheet::new(legacy_spec).expect("legacy res");
    legacy_res.status = ResultStatus::UpToDate;
    legacy_res.tabs.push(ResultTab {
        id: String::from("nonlinear_parameters"),
        title: String::from("Best-fit values & confidence intervals"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![String::from("Best-fit value")],
        row_labels: vec![String::from("Bottom"), String::from("Top (Unstable)"), String::from("LogEC50"), String::from("HillSlope")],
        rows: vec![
            vec![ResultCell::Number { value: 10.5, prefix: None, units: None }],
            vec![ResultCell::Text(String::from("Unstable"))],
            vec![ResultCell::Number { value: -6.25, prefix: None, units: None }],
            vec![ResultCell::Number { value: 1.45, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    });
    legacy_res.scalars = vec![
        ResultScalarEntry { key: String::from("Bottom"), data_set_index: Some(0), value: 10.5, units: None },
        ResultScalarEntry { key: String::from("Top"), data_set_index: Some(0), value: 1.234e7, units: None },
        ResultScalarEntry { key: String::from("LogEC50"), data_set_index: Some(0), value: -6.25, units: None },
        ResultScalarEntry { key: String::from("HillSlope"), data_set_index: Some(0), value: 1.45, units: None },
    ];
    legacy_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("log_agonist_vs_response_4pl"),
        converged: true,
        ambiguous: true,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 10.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 8.0, y: 90.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(legacy_res).expect("add legacy_res");

    let mut eq_scene = empty_scene();
    render_model_equation_annotation(
        &mut eq_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &SheetId::new("res-unstable-legacy"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let eq_joined = collect_texts(&eq_scene).join(" ");
    assert!(!eq_joined.contains("1.234"), "unstable Top float leaked into equation box: {eq_joined}");
    assert!(eq_joined.contains("10.5"), "stable Bottom missing from equation box: {eq_joined}");
    assert!(eq_joined.contains("Top"), "unstable Top must remain symbolic rather than leaking float: {eq_joined}");

    assert_eq!(
        resolve_template_hooks("Top={{result:res-unstable-legacy:0:Top}}, Bottom={{result:res-unstable-legacy:0:Bottom}}", &doc, Some(&ds.id)),
        "Top={{result:res-unstable-legacy:0:Top}}, Bottom=10.5"
    );
    assert_eq!(resolve_template_hooks("{{result:res-unstable-legacy:Top}}", &doc, Some(&ds.id)), "{{result:res-unstable-legacy:Top}}");

    // Fit summary card must also never leak unstable Top.
    let mut graph = GraphSheet::new_uninitialized_for_data("g-legacy-unstable", &ds).expect("graph");
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-unstable-legacy"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    let summary_entries = extract_fit_summary_entries(&graph, &doc, 6);
    assert_eq!(summary_entries.len(), 1);
    assert!(
        summary_entries[0].metrics.iter().all(|m| m.label != "Top" && !m.formatted_value.contains("1.234")),
        "unstable Top leaked into fit summary: {:?}",
        summary_entries[0].metrics
    );

    // 3. When ALL parameters are omitted (all unstable), equation box renders clean placeholder.
    let all_omitted_spec = AnalysisSpec::new(
        "res-all-unstable",
        "xy-collinear",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            identifiability_mode: Some(IdentifiabilityMode::IdentifyUnstableParameters),
            identify_unstable_parameters: true,
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        }),
    )
    .expect("spec");
    let mut all_omitted = ResultSheet::new(all_omitted_spec).expect("res");
    all_omitted.status = ResultStatus::UpToDate;
    doc.add_result_sheet(all_omitted).expect("add all_omitted");
    let mut empty_eq_scene = empty_scene();
    render_model_equation_annotation(
        &mut empty_eq_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &SheetId::new("res-all-unstable"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let ph_text = collect_texts(&empty_eq_scene).join(" ");
    assert!(ph_text.contains("Fit parameters unavailable"), "expected graceful placeholder, got: {ph_text}");
}

#[test]
fn aud_13_06_simple_logistic_and_glm_roc_overlay_parity_and_no_raw_xy_fabrication() {
    // Multi-dataset XY table with Dose in [-5, 5] and binary Y in {0, 1} across two datasets
    // having different DeLong ROC AUCs.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 12).expect("xy");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    let rows: [(f64, f64, f64); 12] = [
        (-5.0, 0.0, 0.0),
        (-4.0, 0.0, 1.0),
        (-3.0, 0.0, 0.0),
        (-2.0, 0.0, 0.0),
        (-1.0, 1.0, 1.0),
        (0.0, 0.0, 0.0),
        (1.0, 1.0, 1.0),
        (2.0, 1.0, 0.0),
        (3.0, 1.0, 1.0),
        (4.0, 1.0, 1.0),
        (5.0, 1.0, 1.0),
        (5.5, 1.0, 0.0),
    ];
    for (r, (x, y0, y1)) in rows.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y0);
        xy.data_sets[1].cells[r][0] = Cell::number(y1);
    }
    let xy_ds = DataSheet::new("xy-roc-sl", "Binary Dose", TableArchetype::Xy(xy)).expect("ds");
    let sl_spec = AnalysisSpec::new("res-sl", "xy-roc-sl", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("sl spec");
    let sl_res = run_analysis(&xy_ds, &sl_spec).expect("run simple logistic");
    let auc_ds0 = sl_res.lookup_scalar("ROC_AUC", Some(0)).expect("DS0 ROC_AUC");
    let auc_ds1 = sl_res.lookup_scalar("ROC_AUC", Some(1)).expect("DS1 ROC_AUC");
    assert!((auc_ds0 - auc_ds1).abs() > 0.05, "expected distinct AUCs for DS0 ({auc_ds0}) and DS1 ({auc_ds1})");

    // Also add an unrelated LinearRegression result sheet BEFORE res-sl to ensure ROC lookup skips it.
    let lin_spec = AnalysisSpec::new("res-lin", "xy-roc-sl", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("lin spec");
    let lin_res = execute_analysis(&lin_spec, &xy_ds, &HookContext::default()).expect("linear fit");

    let mut doc = ProjectDocument::new("AUD13-06").expect("doc");
    doc.add_data_sheet(xy_ds.clone()).expect("add xy");
    doc.add_result_sheet(lin_res).expect("add lin first");
    doc.add_result_sheet(sl_res.clone()).expect("add sl");

    // 1. DiagnosticPlotKind::RocCurve targeting res-sl (DS0 & DS1) and targeting xy-roc-sl must match exact DeLong AUC!
    for (target_id, ds_idx, expected_auc) in [("res-sl", 0usize, auc_ds0), ("res-sl", 1usize, auc_ds1), ("xy-roc-sl", 0usize, auc_ds0)] {
        let mut g = GraphSheet::new_uninitialized_for_data(format!("g-roc-{target_id}-{ds_idx}"), &xy_ds).expect("g");
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
        g.spec.primary_source_sheet_id = SheetId::new(target_id);
        g.spec.series.truncate(1);
        g.spec.series[0].data_set_index = ds_idx;
        let scene = compile_graph(&g, &doc).expect("compile roc");
        assert!(scene.warnings.is_empty(), "unexpected warnings for {target_id} ds={ds_idx}: {:?}", scene.warnings);
        let texts = collect_texts(&scene);
        let expected_lbl = format!("AUC = {expected_auc:.3}");
        assert!(texts.iter().any(|t| t.contains(&expected_lbl)), "expected `{expected_lbl}` for {target_id} ds={ds_idx}, got {texts:?}");
    }

    // 2. Logistic GLM ROC curve emitted by Wave 1 also matches exact DeLong AUC parity.
    let mut mv = MultipleVariablesTable::new(3, 12).expect("mv");
    mv.variables[0].title = String::from("Outcome");
    mv.variables[1].title = String::from("Dose");
    mv.variables[2].title = String::from("Age");
    for (r, (x, y0, _)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(y0);
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number(30.0 + (r as f64) * 2.5);
    }
    let mv_ds = DataSheet::new("mv-glm", "GLM Cohort", TableArchetype::MultipleVariables(mv)).expect("mv ds");
    let glm_spec = AnalysisSpec::new(
        "res-glm-logit",
        "mv-glm",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .expect("glm spec");
    let glm_res = run_analysis(&mv_ds, &glm_spec).expect("run glm logistic");
    let glm_auc = glm_res.lookup_scalar("ROC AUC", None).expect("GLM ROC AUC");
    doc.add_data_sheet(mv_ds.clone()).expect("add mv");
    doc.add_result_sheet(glm_res).expect("add glm");

    let mut g_glm = GraphSheet::new_uninitialized_for_data("g-roc-glm", &mv_ds).expect("g glm");
    g_glm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    g_glm.spec.primary_source_sheet_id = SheetId::new("res-glm-logit");
    let glm_scene = compile_graph(&g_glm, &doc).expect("compile glm roc");
    assert!(glm_scene.warnings.is_empty(), "unexpected GLM ROC warnings: {:?}", glm_scene.warnings);
    let glm_texts = collect_texts(&glm_scene);
    assert!(glm_texts.iter().any(|t| t.contains(&format!("AUC = {glm_auc:.3}"))), "expected GLM DeLong `AUC = {glm_auc:.3}`, got {glm_texts:?}");

    // 3. Logistic ResultSheet WITHOUT an "ROC" overlay MUST NEVER silently fall through to raw XY
    //    `(Dose, {0,1})` pairs to fabricate a bogus curve/AUC!
    let mut doc_no_roc = ProjectDocument::new("AUD13-06-NoRoc").expect("doc no roc");
    doc_no_roc.add_data_sheet(xy_ds.clone()).expect("add xy");
    let mut stripped = sl_res;
    stripped.fitted_curves.retain(|fc| !fc.equation_id.eq_ignore_ascii_case("ROC"));
    doc_no_roc.add_result_sheet(stripped).expect("add stripped");

    for target_id in ["res-sl", "xy-roc-sl"] {
        let mut g = GraphSheet::new_uninitialized_for_data(format!("g-noroc-{target_id}"), &xy_ds).expect("g");
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
        g.spec.primary_source_sheet_id = SheetId::new(target_id);
        let scene = compile_graph(&g, &doc_no_roc).expect("compile stripped roc");
        let texts = collect_texts(&scene);
        assert!(
            texts.iter().all(|t| !t.contains("AUC =")),
            "must never fabricate bogus AUC from raw XY `(Dose, {{0,1}})` for target `{target_id}`: {texts:?}"
        );
        assert!(
            scene.warnings.iter().any(|w| w.code == "missing_roc_overlay") && scene.warnings.iter().any(|w| w.code == "empty_roc"),
            "expected missing_roc_overlay + empty_roc warnings for `{target_id}`, got {:?}",
            scene.warnings
        );
    }

    // 4. XY graph attaching SimpleLogisticRegression overlay must render the sigmoid (not the ROC curve).
    let mut g_xy = GraphSheet::new_uninitialized_for_data("g-xy-sl", &xy_ds).expect("g xy");
    g_xy.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-sl"),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    let xy_scene = compile_graph(&g_xy, &doc).expect("compile xy sigmoid");
    assert!(xy_scene.warnings.is_empty(), "unexpected XY sigmoid overlay warnings: {:?}", xy_scene.warnings);
}

#[test]
fn aud_13_07_analyzable_data_chaining_across_mv_survival_contingency_pow_and_diagnostics() {
    let mut doc = ProjectDocument::new("AUD13-07").expect("doc");

    let wrap_transform = |id: &str, source_id: &str, ds: DataSheet| -> ResultSheet {
        let spec = AnalysisSpec::new(
            id,
            source_id,
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: None,
                error_propagation: ErrorPropagationMode::default(),
            }),
        )
        .expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        res.analyzable_data = Some(ds);
        res
    };

    // --- 1. MultipleVariables (raw has 6 rows including 3 pediatric; filtered analyzable_data has 3 adult rows) ---
    let mut raw_mv = MultipleVariablesTable::new(3, 6).expect("raw mv");
    raw_mv.variables[0].title = String::from("Age");
    raw_mv.variables[1].title = String::from("Response");
    raw_mv.variables[2].title = String::from("Group");
    raw_mv.variables[2].lock_role(VariableRole::Categorical);
    for (r, (age, resp, grp)) in
        [(5.0, 1.0, "Ped"), (8.0, 2.0, "Ped"), (12.0, 3.0, "Ped"), (25.0, 40.0, "Adult"), (35.0, 55.0, "Adult"), (45.0, 70.0, "Adult")]
            .into_iter()
            .enumerate()
    {
        raw_mv.variables[0].cells[r] = Cell::number(age);
        raw_mv.variables[1].cells[r] = Cell::number(resp);
        raw_mv.variables[2].cells[r] = Cell::text(grp);
    }
    let raw_mv_ds = DataSheet::new("mv-raw", "Raw Patients", TableArchetype::MultipleVariables(raw_mv)).expect("raw mv ds");
    let mut filt_mv = MultipleVariablesTable::new(3, 3).expect("filt mv");
    filt_mv.variables[0].title = String::from("Age");
    filt_mv.variables[1].title = String::from("Response");
    filt_mv.variables[2].title = String::from("Group");
    filt_mv.variables[2].lock_role(VariableRole::Categorical);
    for (r, (age, resp)) in [(25.0, 40.0), (35.0, 55.0), (45.0, 70.0)].into_iter().enumerate() {
        filt_mv.variables[0].cells[r] = Cell::number(age);
        filt_mv.variables[1].cells[r] = Cell::number(resp);
        filt_mv.variables[2].cells[r] = Cell::text("Adult");
    }
    let filt_mv_ds = DataSheet::new("mv-filt-data", "Adult Only", TableArchetype::MultipleVariables(filt_mv)).expect("filt mv ds");
    doc.add_data_sheet(raw_mv_ds).expect("add raw mv");
    doc.add_result_sheet(wrap_transform("res-mv-filt", "mv-raw", filt_mv_ds.clone())).expect("add res mv filt");

    // 1a. MultipleVariables BubbleScatter renders strictly the 3 filtered adult points (NOT blank and NOT 6 raw points).
    let mut g_mv = GraphSheet::new_uninitialized_for_data("g-mv-filt", &filt_mv_ds).expect("g mv");
    g_mv.spec.primary_source_sheet_id = SheetId::new("res-mv-filt");
    g_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(Rgba8::BLACK),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    let mv_scene = compile_graph(&g_mv, &doc).expect("compile mv");
    assert_eq!(mv_scene.plotted_points.len(), 3, "MV bubble must plot 3 filtered points, not raw/empty: {:?}", mv_scene.warnings);

    // 1b. Diagnostic Normal Q-Q, Residual, Volcano, Heatmap, Bland-Altman on res-mv-filt must plot filtered data (3 rows, not 6 raw rows)!
    for dkind in [DiagnosticPlotKind::NormalQqPlot, DiagnosticPlotKind::ResidualPlot, DiagnosticPlotKind::Volcano, DiagnosticPlotKind::BlandAltman] {
        let mut g_diag = GraphSheet::new_uninitialized_for_data(format!("g-diag-{dkind:?}"), &filt_mv_ds).expect("g diag");
        g_diag.spec.kind = GraphKind::Diagnostic(dkind);
        g_diag.spec.primary_source_sheet_id = SheetId::new("res-mv-filt");
        g_diag.spec.multivariate = None;
        let d_scene = compile_graph(&g_diag, &doc).expect("compile diag");
        assert_eq!(
            d_scene.plotted_points.len(),
            3,
            "diagnostic {dkind:?} on transformed ResultSheet must plot 3 filtered points (not 6 untransformed raw points!), warnings={:?}",
            d_scene.warnings
        );
    }

    // 1c. Heatmap on 2-hop chain (raw -> res-mv-filt [analyzable] -> res-mv-hop2 [analyzable_data=None]) plots 3 rows (not 6 raw)!
    let hop2_spec = AnalysisSpec::new("res-mv-hop2", "res-mv-filt", AnalysisKind::DescriptiveStats(curvebook_doc::DescriptiveSpec::default()))
        .expect("hop2 spec");
    let mut hop2 = ResultSheet::new(hop2_spec).expect("hop2");
    hop2.status = ResultStatus::UpToDate;
    doc.add_result_sheet(hop2).expect("add hop2");
    let mut g_hm = GraphSheet::new_uninitialized_for_data("g-hm-hop2", &filt_mv_ds).expect("g hm");
    g_hm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    g_hm.spec.primary_source_sheet_id = SheetId::new("res-mv-hop2");
    g_hm.spec.heatmap_labels.annotate_cells = true;
    let hm_scene = compile_graph(&g_hm, &doc).expect("compile hm");
    assert!(hm_scene.warnings.is_empty(), "expected clean 2-hop heatmap, got {:?}", hm_scene.warnings);
    assert!(hm_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("40"))));
    assert!(
        hm_scene.primitives.iter().all(|p| !matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "5")),
        "untransformed raw pediatric Age=5 leaked into 2-hop heatmap!"
    );

    // 1d. ForestPlot on transformed Grouped(UpperLowerLimits) ResultSheet plots transformed studies (not raw)!
    let mut raw_for = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2).expect("raw for");
    raw_for.row_titles = vec![String::from("RawStudyA"), String::from("RawStudyB")];
    raw_for.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(0.8), Cell::number(1.2)];
    raw_for.data_sets[0].cells[1] = vec![Cell::number(2.0), Cell::number(1.5), Cell::number(2.5)];
    let raw_for_ds = DataSheet::new("for-raw", "RawForest", TableArchetype::Grouped(raw_for)).expect("raw for ds");
    let mut tx_for = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2).expect("tx for");
    tx_for.row_titles = vec![String::from("CleanTrialX"), String::from("CleanTrialY")];
    tx_for.data_sets[0].cells[0] = vec![Cell::number(0.6), Cell::number(0.4), Cell::number(0.9)];
    tx_for.data_sets[0].cells[1] = vec![Cell::number(0.7), Cell::number(0.5), Cell::number(0.95)];
    let tx_for_ds = DataSheet::new("for-tx-data", "CleanForest", TableArchetype::Grouped(tx_for)).expect("tx for ds");
    doc.add_data_sheet(raw_for_ds).expect("add raw for");
    doc.add_result_sheet(wrap_transform("res-for-tx", "for-raw", tx_for_ds.clone())).expect("add res for");
    let mut g_for = GraphSheet::new_uninitialized_for_data("g-for-tx", &tx_for_ds).expect("g for");
    g_for.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    g_for.spec.primary_source_sheet_id = SheetId::new("res-for-tx");
    let for_scene = compile_graph(&g_for, &doc).expect("compile forest");
    let for_texts = collect_texts(&for_scene).join(" | ");
    assert!(for_texts.contains("CleanTrialX") && !for_texts.contains("RawStudyA"), "forest plotted wrong sheet: {for_texts}");

    // 1e. ROC on transformed Column(Controls, Cases) ResultSheet uses transformed values (AUC=1.000, not raw AUC=0.000)!
    let mut raw_roc = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("raw roc");
    for (r, (c, k)) in [(10.0, 1.0), (11.0, 2.0), (12.0, 3.0), (13.0, 4.0)].into_iter().enumerate() {
        raw_roc.data_sets[0].cells[r][0] = Cell::number(c);
        raw_roc.data_sets[1].cells[r][0] = Cell::number(k);
    }
    let raw_roc_ds = DataSheet::new("roc-raw", "RawROC", TableArchetype::Column(raw_roc)).expect("raw roc ds");
    let mut tx_roc = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("tx roc");
    for (r, (c, k)) in [(1.0, 10.0), (2.0, 11.0), (3.0, 12.0), (4.0, 13.0)].into_iter().enumerate() {
        tx_roc.data_sets[0].cells[r][0] = Cell::number(c);
        tx_roc.data_sets[1].cells[r][0] = Cell::number(k);
    }
    let tx_roc_ds = DataSheet::new("roc-tx-data", "TransformedROC", TableArchetype::Column(tx_roc)).expect("tx roc ds");
    doc.add_data_sheet(raw_roc_ds).expect("add raw roc");
    doc.add_result_sheet(wrap_transform("res-roc-tx", "roc-raw", tx_roc_ds.clone())).expect("add res roc");
    let mut g_roc_tx = GraphSheet::new_uninitialized_for_data("g-roc-tx", &tx_roc_ds).expect("g roc tx");
    g_roc_tx.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    g_roc_tx.spec.primary_source_sheet_id = SheetId::new("res-roc-tx");
    let roc_tx_scene = compile_graph(&g_roc_tx, &doc).expect("compile roc tx");
    assert!(
        collect_texts(&roc_tx_scene).iter().any(|t| t.contains("AUC = 1.000")),
        "expected AUC = 1.000 from transformed analyzable_data, got {:?}",
        collect_texts(&roc_tx_scene)
    );

    // --- 2. Survival on transformed ResultSheet ---
    let mut surv = SurvivalTable::new(2, 5).expect("surv");
    surv.cohorts[0].title = String::from("FiltVehicle");
    surv.cohorts[1].title = String::from("FiltDrug");
    for (r, (t, s0, s1)) in [(2.0, 1.0, 0.0), (4.0, 1.0, 1.0), (6.0, 0.0, 0.0), (8.0, 1.0, 1.0), (10.0, 1.0, 0.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(s0);
        surv.cohorts[1].cells[r][0] = Cell::number(s1);
    }
    let surv_tx_ds = DataSheet::new("surv-tx-data", "Filtered OS", TableArchetype::Survival(surv)).expect("surv ds");
    doc.add_result_sheet(wrap_transform("res-surv-tx", "mv-raw", surv_tx_ds.clone())).expect("add surv res");
    let mut g_surv = GraphSheet::new_uninitialized_for_data("g-surv-tx", &surv_tx_ds).expect("g surv");
    g_surv.spec.primary_source_sheet_id = SheetId::new("res-surv-tx");
    let surv_scene = compile_graph(&g_surv, &doc).expect("compile surv");
    assert!(surv_scene.warnings.is_empty(), "unexpected survival warnings: {:?}", surv_scene.warnings);
    assert!(collect_texts(&surv_scene).iter().any(|t| t.contains("FiltDrug")), "missing transformed survival cohort legend");

    // --- 3. Contingency on transformed ResultSheet ---
    let mut cont = ContingencyTable::new(2, 2).expect("cont");
    cont.row_titles = vec![String::from("FilteredPos"), String::from("FilteredNeg")];
    cont.data_sets[0].title = String::from("Placebo");
    cont.data_sets[1].title = String::from("Active");
    cont.data_sets[0].cells[0][0] = Cell::number(12.0);
    cont.data_sets[0].cells[1][0] = Cell::number(28.0);
    cont.data_sets[1].cells[0][0] = Cell::number(30.0);
    cont.data_sets[1].cells[1][0] = Cell::number(10.0);
    let cont_tx_ds = DataSheet::new("cont-tx-data", "Filtered Cont", TableArchetype::Contingency(cont)).expect("cont ds");
    doc.add_result_sheet(wrap_transform("res-cont-tx", "mv-raw", cont_tx_ds.clone())).expect("add cont res");
    let mut g_cont = GraphSheet::new_uninitialized_for_data("g-cont-tx", &cont_tx_ds).expect("g cont");
    g_cont.spec.primary_source_sheet_id = SheetId::new("res-cont-tx");
    let cont_scene = compile_graph(&g_cont, &doc).expect("compile cont");
    assert!(cont_scene.warnings.is_empty(), "unexpected contingency warnings: {:?}", cont_scene.warnings);
    assert!(collect_texts(&cont_scene).iter().any(|t| t.contains("FilteredPos")), "missing transformed contingency row label");

    // --- 4. PartsOfWhole on transformed ResultSheet ---
    let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
    pow.slice_labels = vec![String::from("AlphaPart"), String::from("BetaPart"), String::from("GammaPart")];
    pow.data_sets[0].cells[0][0] = Cell::number(50.0);
    pow.data_sets[0].cells[1][0] = Cell::number(30.0);
    pow.data_sets[0].cells[2][0] = Cell::number(20.0);
    let pow_tx_ds = DataSheet::new("pow-tx-data", "Filtered PoW", TableArchetype::PartsOfWhole(pow)).expect("pow ds");
    doc.add_result_sheet(wrap_transform("res-pow-tx", "mv-raw", pow_tx_ds.clone())).expect("add pow res");
    let mut g_pow = GraphSheet::new_uninitialized_for_data("g-pow-tx", &pow_tx_ds).expect("g pow");
    g_pow.spec.primary_source_sheet_id = SheetId::new("res-pow-tx");
    let pow_scene = compile_graph(&g_pow, &doc).expect("compile pow");
    assert!(pow_scene.warnings.is_empty(), "unexpected PoW warnings: {:?}", pow_scene.warnings);
    assert!(collect_texts(&pow_scene).iter().any(|t| t.contains("AlphaPart (50.0%)")), "missing transformed PoW slice");

    // --- 5. Failed transform ResultSheet must NEVER leak raw upstream data ---
    let mut failed_res = wrap_transform("res-mv-failed", "mv-raw", filt_mv_ds.clone());
    failed_res.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("filter failed") };
    doc.add_result_sheet(failed_res).expect("add failed res");
    let mut g_fail = GraphSheet::new_uninitialized_for_data("g-diag-fail", &filt_mv_ds).expect("g fail");
    g_fail.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    g_fail.spec.primary_source_sheet_id = SheetId::new("res-mv-failed");
    let fail_scene = compile_graph(&g_fail, &doc).expect("compile fail");
    assert!(fail_scene.plotted_points.is_empty(), "failed transform result sheet must never fall back to raw upstream data");
}

#[test]
fn aud_13_16_simple_logistic_multidataset_equation_box_sig_figs_and_hooks() {
    // 1. Multi-dataset SimpleLogisticRegression with large-X scale (small slope ~5e-6) so `.4`
    //    hardcoding would have rounded beta1 to `0.0000`.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 12).expect("xy");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    let xs = [0.0, 1e5, 2e5, 3e5, 4e5, 5e5, 6e5, 7e5, 8e5, 9e5, 1e6, 1.1e6];
    let y0s = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let y1s = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    for r in 0..12 {
        xy.set_x_cell(r, Cell::number(xs[r])).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y0s[r]);
        xy.data_sets[1].cells[r][0] = Cell::number(y1s[r]);
    }
    let ds = DataSheet::new("xy-sl-multi", "Large-Scale Binary", TableArchetype::Xy(xy)).expect("ds");
    let spec = AnalysisSpec::new("res-sl-multi", "xy-sl-multi", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec");
    let res = run_analysis(&ds, &spec).expect("run simple logistic");
    let b1_ds0 = res.lookup_scalar("Beta1", Some(0)).expect("DS0 Beta1");
    let b1_ds1 = res.lookup_scalar("Beta1", Some(1)).expect("DS1 Beta1");
    let x50_ds0 = res.lookup_scalar("X50", Some(0)).expect("DS0 X50");
    let x50_ds1 = res.lookup_scalar("X50", Some(1)).expect("DS1 X50");
    assert!(b1_ds0.abs() > 0.0 && b1_ds0.abs() < 1e-4, "expected tiny positive slope for DS0, got {b1_ds0}");
    assert!((x50_ds0 - x50_ds1).abs() > 1e5, "expected distinct X50 across datasets: {x50_ds0} vs {x50_ds1}");

    let mut doc = ProjectDocument::new("AUD13-16").expect("doc");
    doc.add_data_sheet(ds.clone()).expect("add ds");
    doc.add_result_sheet(res).expect("add res");

    let tex0 = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-multi"), 0, EquationDisplayMode::Substituted).expect("DS0 tex");
    let tex1 = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-multi"), 1, EquationDisplayMode::Substituted).expect("DS1 tex");
    assert_ne!(tex0, tex1, "DS0 and DS1 must render distinct logistic equations: {tex0} vs {tex1}");
    assert!(!tex0.contains("0.0000"), "DS0 slope must not round to 0.0000: {tex0}");
    assert!(!tex1.contains("0.0000"), "DS1 slope must not round to 0.0000: {tex1}");
    assert!(tex0.contains("\\times 10^{"), "tiny slope {b1_ds0} must format in sig-fig scientific notation: {tex0}");
    assert!(tex1.contains("\\times 10^{"), "tiny slope {b1_ds1} must format in sig-fig scientific notation: {tex1}");

    // Render DS1 into scene and verify clean typesetting with no unexpanded TeX macros.
    let mut scene = empty_scene();
    render_model_equation_annotation(
        &mut scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &SheetId::new("res-sl-multi"),
            data_set_index: 1,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let rendered = collect_texts(&scene).join(" ");
    assert!(!rendered.contains("Fit parameters unavailable") && !rendered.contains("0.0000") && !rendered.contains("\\times"), "rendered={rendered}");

    // Template hooks resolve per-dataset Wave 1 scalars cleanly without rounding tiny slopes to 0.0000.
    let hook0 = resolve_template_hooks("{{result:res-sl-multi:0:Beta1}} | {{result:res-sl-multi:0:X50}}", &doc, Some(&ds.id));
    let hook1 = resolve_template_hooks("{{result:res-sl-multi:1:Beta1}} | {{result:res-sl-multi:1:X50}}", &doc, Some(&ds.id));
    assert!(!hook0.contains("{{") && !hook0.contains("0.0000"), "hook0={hook0}");
    assert!(!hook1.contains("{{") && !hook1.contains("0.0000"), "hook1={hook1}");
    assert_ne!(hook0, hook1, "DS0 and DS1 hooks must differ");

    // 2. Legacy pre-Wave-1 fallback (`data_set_index: None`, key `"DrugB.Beta0"` / `"DrugB.Beta1"` / `"DrugB.X50"`).
    let legacy_spec =
        AnalysisSpec::new("res-sl-legacy", "xy-sl-multi", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec");
    let mut legacy = ResultSheet::new(legacy_spec).expect("legacy");
    legacy.status = ResultStatus::UpToDate;
    legacy.scalars = vec![
        ResultScalarEntry { key: String::from("DrugA.Beta0"), data_set_index: None, value: -2.0, units: None },
        ResultScalarEntry { key: String::from("DrugA.Beta1"), data_set_index: None, value: 4.0e-6, units: None },
        ResultScalarEntry { key: String::from("DrugB.Beta0"), data_set_index: None, value: -3.5, units: None },
        ResultScalarEntry { key: String::from("DrugB.Beta1"), data_set_index: None, value: 5.0e-6, units: None },
        ResultScalarEntry { key: String::from("DrugB.X50"), data_set_index: None, value: 7.0e5, units: None },
    ];
    doc.add_result_sheet(legacy).expect("add legacy");
    let legacy_tex = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-legacy"), 1, EquationDisplayMode::Substituted).expect("legacy DS1 tex");
    assert!(legacy_tex.contains("-3.5") && legacy_tex.contains("5 \\times 10^{-6}"), "legacy_tex={legacy_tex}");
    let legacy_hook = resolve_template_hooks("{{result:res-sl-legacy:1:X50}}", &doc, Some(&ds.id));
    assert_eq!(legacy_hook, "7.000e5", "legacy_hook={legacy_hook}");
}

#[test]
fn aud_13_17_default_model_ii_deming_overlay_renders_cleanly() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy");
    xy.data_sets[0].title = String::from("Method B");
    for (r, (x, y)) in [(1.0, 2.2), (2.0, 4.1), (3.0, 5.9), (4.0, 8.3), (5.0, 10.0), (6.0, 12.1), (7.0, 13.8), (8.0, 16.2)].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-deming", "Method Comparison", TableArchetype::Xy(xy)).expect("ds");

    for (tag, err_spec) in [
        ("equal", DemingErrorSpec::EqualError),
        ("sds", DemingErrorSpec::StandardDeviations { sd_x: 0.4, sd_y: 0.8 }),
        ("ratio", DemingErrorSpec::VarianceRatio { lambda: 2.0 }),
    ] {
        let lin_spec = LinearRegressionSpec { deming_model_ii: true, deming_error: err_spec, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new(format!("res-deming-{tag}"), "xy-deming", AnalysisKind::LinearRegression(lin_spec)).expect("spec");
        let res = execute_analysis(&spec, &ds, &HookContext::default()).expect("deming execute");
        assert_eq!(res.status, ResultStatus::UpToDate);
        assert_eq!(res.fitted_curves.len(), 1, "Wave 1 must emit default Deming overlay for {tag}");

        let mut doc = ProjectDocument::new("AUD13-17").expect("doc");
        doc.add_data_sheet(ds.clone()).expect("add ds");
        doc.add_result_sheet(res).expect("add res");

        let mut graph = GraphSheet::new_uninitialized_for_data(format!("g-deming-{tag}"), &ds).expect("graph");
        graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: SheetId::new(format!("res-deming-{tag}")),
            sample_segments: 256,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: true,
        });
        graph.spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new(format!("res-deming-{tag}")),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(80.0, 30.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        });

        let scene = compile_graph(&graph, &doc).expect("compile deming chart");
        assert!(scene.warnings.is_empty(), "expected zero warnings for default Deming ({tag}), got {:?}", scene.warnings);
        // Must emit the fitted curve polyline stroke clipped to plot_rect_pt!
        let overlay_strokes = scene
            .primitives
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: Some(_) }
                        if (st.width_pt - 1.75).abs() < 1e-6 && path.commands.len() >= 16
                )
            })
            .count();
        assert!(overlay_strokes >= 1, "expected rendered Deming fitted curve path for {tag}");
    }
}

#[test]
fn aud_13_18_linear_fixed_point_custom_dsl_dataset_b_and_spline_suppression() {
    // 1. Linear regression with `force_through_origin: true` AND `fixed_point: Some((10.0, 25.0))`:
    //    `fixed_point` takes precedence, giving Y = 2*X + 5; equation box MUST preserve `+ 5`!
    let mut xy_lin = XyTable::new(SubcolumnMode::Single, 1, 5).expect("xy lin");
    for (r, (x, y)) in [(0.0, 5.0), (5.0, 15.0), (10.0, 25.0), (15.0, 35.0), (20.0, 45.0)].into_iter().enumerate() {
        xy_lin.set_x_cell(r, Cell::number(x)).expect("x");
        xy_lin.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let lin_ds = DataSheet::new("xy-fp", "FixedPoint", TableArchetype::Xy(xy_lin)).expect("ds");
    let fp_spec = AnalysisSpec::new(
        "res-fp",
        "xy-fp",
        AnalysisKind::LinearRegression(LinearRegressionSpec {
            force_through_origin: true,
            fixed_point: Some((10.0, 25.0)),
            ..LinearRegressionSpec::default()
        }),
    )
    .expect("fp spec");
    let fp_res = execute_analysis(&fp_spec, &lin_ds, &HookContext::default()).expect("fp fit");

    let mut doc = ProjectDocument::new("AUD13-18").expect("doc");
    doc.add_data_sheet(lin_ds).expect("add lin_ds");
    doc.add_result_sheet(fp_res).expect("add fp_res");

    let fp_sym = resolve_model_equation_tex(&doc, &SheetId::new("res-fp"), 0, EquationDisplayMode::Symbolic).expect("fp sym");
    let fp_sub = resolve_model_equation_tex(&doc, &SheetId::new("res-fp"), 0, EquationDisplayMode::Substituted).expect("fp sub");
    assert!(fp_sym.contains("Intercept"), "fixed_point symbolic equation dropped Intercept: {fp_sym}");
    assert!(fp_sub.contains('2') && fp_sub.contains("+ 5"), "fixed_point substituted equation dropped + 5 intercept: {fp_sub}");

    // 2. Two-branch custom DSL (`<A>` total binding, `<B>` nonspecific linear binding with shared NS):
    //    Dataset B equation box MUST render `<B>` branch (`NS * X`), NEVER Dataset A's `<A>` branch!
    let mut xy_dsl = XyTable::new(SubcolumnMode::Single, 2, 8).expect("xy dsl");
    xy_dsl.data_sets[0].title = String::from("Total");
    xy_dsl.data_sets[1].title = String::from("Nonspecific");
    let dsl_xs = [0.5, 1.0, 2.0, 4.0, 8.0, 12.0, 16.0, 20.0];
    for (r, &x) in dsl_xs.iter().enumerate() {
        xy_dsl.set_x_cell(r, Cell::number(x)).expect("x");
        xy_dsl.data_sets[0].cells[r][0] = Cell::number(100.0 * x / (2.0 + x) + 1.5 * x);
        xy_dsl.data_sets[1].cells[r][0] = Cell::number(1.5 * x);
    }
    let dsl_ds = DataSheet::new("xy-dsl", "Binding", TableArchetype::Xy(xy_dsl)).expect("dsl ds");
    let mut nl_dsl = NonlinearFitSpec::new("custom_two_branch");
    nl_dsl.custom_dsl = Some(String::from("<A> Y = Bmax * X / (Kd + X) + NS * X\n<B> Y = NS * X"));
    nl_dsl.parameter_rules = vec![ParameterRule {
        name: String::from("NS"),
        scope: ParameterScope::SharedGlobal,
        initial_override: Some(1.0),
        lower_bound: None,
        upper_bound: None,
    }];
    let dsl_spec = AnalysisSpec::new("res-dsl-ab", "xy-dsl", AnalysisKind::NonlinearRegression(nl_dsl)).expect("dsl spec");
    let dsl_res = execute_analysis(&dsl_spec, &dsl_ds, &HookContext::default()).expect("dsl fit");
    doc.add_data_sheet(dsl_ds).expect("add dsl_ds");
    doc.add_result_sheet(dsl_res).expect("add dsl_res");

    let sym_a = resolve_model_equation_tex(&doc, &SheetId::new("res-dsl-ab"), 0, EquationDisplayMode::Symbolic).expect("sym A");
    let sym_b = resolve_model_equation_tex(&doc, &SheetId::new("res-dsl-ab"), 1, EquationDisplayMode::Symbolic).expect("sym B");
    assert!(sym_a.contains("Bmax") && sym_a.contains("Kd") && sym_a.contains("NS"), "expected <A> branch for DS0: {sym_a}");
    assert!(sym_b.contains("NS") && !sym_b.contains("Bmax") && !sym_b.contains("Kd"), "DS1 must render <B> branch, not <A>: {sym_b}");

    let sub_b = resolve_model_equation_tex(&doc, &SheetId::new("res-dsl-ab"), 1, EquationDisplayMode::Substituted).expect("sub B");
    assert!(sub_b.contains("1.5") && !sub_b.contains("100") && !sub_b.contains("\\frac"), "DS1 substituted must render <B> `1.5 * X`: {sub_b}");

    // 3. Model-free `FitSplineLowess` MUST suppress bogus `Y = f(X; n_points = ..., segments = ...)` dumps!
    let spline_kind = AnalysisKind::FitSplineLowess(SplineLowessSpec::default());
    assert!(!is_parametric_equation_analysis(&spline_kind), "FitSplineLowess must not be classified as parametric");
    let spline_spec = AnalysisSpec::new("res-spline", "xy-fp", spline_kind).expect("spline spec");
    let mut spline_res = ResultSheet::new(spline_spec).expect("spline res");
    spline_res.status = ResultStatus::UpToDate;
    spline_res.scalars = vec![
        ResultScalarEntry { key: String::from("n_points"), data_set_index: Some(0), value: 10.0, units: None },
        ResultScalarEntry { key: String::from("distinct_x"), data_set_index: Some(0), value: 10.0, units: None },
        ResultScalarEntry { key: String::from("segments"), data_set_index: Some(0), value: 200.0, units: None },
    ];
    spline_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("cubic_spline"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 5.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 20.0, y: 45.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(spline_res).expect("add spline_res");

    for mode in [EquationDisplayMode::Symbolic, EquationDisplayMode::Substituted] {
        assert_eq!(resolve_model_equation_tex(&doc, &SheetId::new("res-spline"), 0, mode), None, "FitSplineLowess must return None for {mode:?}");
        let mut s = empty_scene();
        render_model_equation_annotation(
            &mut s,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &SheetId::new("res-spline"),
                data_set_index: 0,
                mode,
                bounds_pt: RectPt::new(20.0, 20.0, 200.0, 40.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        assert!(s.primitives.is_empty(), "FitSplineLowess must render zero primitives (no diagnostic dump), got {:?}", collect_texts(&s));
    }
}
