//! Wave 23 Shard 6 `gui_only` acceptance test suite (`WF-37`..`WF-43`):
//! Hierarchical Clustering (4 distances × 7 linkages, cophenetic correlation, clustered heatmap),
//! K-Means Clustering (WCSS/Silhouette sweep, cluster column append, scatter with ellipses/hulls),
//! Cox Proportional Hazards (categorical reference levels, Efron vs Breslow ties, AIC, covariance, survival curves),
//! Multiple Linear Regression (Type III partial SS, VIF, AIC, covariance, residuals, QOL-7 diagnostics, diagnostic plots),
//! Multiple Logistic Regression (VIF, AIC, Hosmer-Lemeshow, 2×2 cutoff & sweep, row predictions, ROC curve),
//! Poisson Count GLM (IRRs, VIF pruning, dispersion, AIC, McFadden R², residuals), and
//! XY Serial Dilution 4PL IC50 (Log10(X), unconstrained vs Bottom=0/Top=1 profile CI, Trapezoidal & Fitted AUC).
//!
//! All state mutations and interactions are driven strictly through `dispatch_gui_action`
//! (`ui.inspect`, `ui.invoke_menu`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`,
//! `ui.switch_tab`, `ui.answer_dialog`, `ui.screenshot`) under `app.set_gui_only(true)`.

#![forbid(unsafe_code)]

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, CellCoord, CellValue, ChecklistStatus, CiBound, ClusterDistance, ClusterLinkage, CoxTieMethod, DataSheet, MultivariatePlotMode,
    MultivariateSpec, ParameterScope, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SurvivalAnalysisSpec, TableArchetype,
    VariableRole,
};
use curvebook_palette::LineDashStyle;
use curvebook_testkit::metrics::approx_eq;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn new_strict_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    ensure(app.is_gui_only(), "app.is_gui_only() must be true")?;
    Ok(app)
}

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {err}", err.code))
}

fn invoke_menu_cmd(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": command_id, "params": params }))
}

fn invoke_menu_path(app: &mut UiApp, path: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "path": path, "params": params }))
}

fn inspect_ui(app: &mut UiApp, scope: &str) -> Result<Value, String> {
    gui(app, "ui.inspect", json!({ "scope": scope, "include_bounds": true }))
}

fn answer_dialog(app: &mut UiApp, fields: Value, action: &str) -> Result<Value, String> {
    gui(app, "ui.answer_dialog", json!({ "fields": fields, "action": action }))
}

fn press_shortcut(app: &mut UiApp, key: &str, modifiers: Value) -> Result<Value, String> {
    gui(app, "ui.press_key", json!({ "key": key, "modifiers": modifiers }))
}

fn select_sheet(app: &mut UiApp, sheet_id: &str) -> TestRes {
    invoke_menu_path(app, "View/Select Sheet", json!({ "sheet_id": sheet_id }))?;
    Ok(())
}

fn click_widget(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    gui(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id }))
}

fn type_widget(app: &mut UiApp, widget_id: &str, text: &str) -> Result<Value, String> {
    click_widget(app, widget_id)?;
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "clear_existing": true, "commit_enter": true }))
}

fn paste_grid_tsv(app: &mut UiApp, row: usize, col: usize, subcol: usize, tsv: &str) -> TestRes {
    let wid = format!("grid.cell.{row}.{col}.{subcol}");
    type_widget(app, &wid, tsv)?;
    Ok(())
}

fn extract_sheet_id(val: &Value, ctx: &str) -> Result<String, String> {
    val.get("sheet_id")
        .or_else(|| val.get("payload").and_then(|p| p.get("sheet_id")))
        .or_else(|| val.get("result").and_then(|r| r.get("sheet_id")))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{ctx} missing `sheet_id` in output: {val}"))
}

fn create_table(app: &mut UiApp, params: Value) -> Result<String, String> {
    let out = invoke_menu_path(app, "Insert/New Data Table", params)?;
    extract_sheet_id(&out, "Insert/New Data Table")
}

fn set_x_values(app: &mut UiApp, sheet_id: &str, values: &[f64]) -> TestRes {
    invoke_menu_path(app, "Data/Set X Values", json!({ "sheet_id": sheet_id, "values": values }))?;
    Ok(())
}

fn run_analyze_dialog(app: &mut UiApp, target_sheet_id: &str, fields: Value) -> Result<String, String> {
    let before = app.document().result_sheets.len();
    select_sheet(app, target_sheet_id)?;
    invoke_menu_cmd(app, "ui.analyze.open", json!({}))?;
    let snap = inspect_ui(app, "dialog")?;
    let kind = snap.pointer("/dialog/dialog_kind").or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "analyze_sheet", format!("expected analyze_sheet dialog, got `{kind}`"))?;

    let out = answer_dialog(app, fields, "ok")?;
    if out.get("pending_confirmation").and_then(Value::as_bool) == Some(true) {
        let dup_snap = inspect_ui(app, "dialog")?;
        let dup_kind = dup_snap.get("active_dialog").and_then(Value::as_str).unwrap_or("");
        ensure(dup_kind == "duplicate_analysis_prompt", format!("expected duplicate_analysis_prompt, got `{dup_kind}`"))?;
        answer_dialog(app, json!({ "choice": "create_new" }), "ok")?;
    }
    ensure(
        app.document().result_sheets.len() == before + 1,
        format!("expected ResultSheet count {} -> {}, got {}", before, before + 1, app.document().result_sheets.len()),
    )?;
    let created = app.document().result_sheets.last().ok_or_else(|| String::from("missing newly created ResultSheet"))?;
    ensure(created.status == ResultStatus::UpToDate, format!("ResultSheet `{}` expected UpToDate, got {:?}", created.id, created.status))?;
    Ok(created.id.as_str().to_owned())
}

fn edit_params_cmd_t(app: &mut UiApp, result_sheet_id: &str, fields: Value) -> Result<Value, String> {
    select_sheet(app, result_sheet_id)?;
    press_shortcut(app, "t", json!({ "meta": true }))?;
    let snap = inspect_ui(app, "dialog")?;
    let kind = snap.pointer("/dialog/dialog_kind").or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "edit_analysis_parameters", format!("expected edit_analysis_parameters dialog, got `{kind}`"))?;
    let out = answer_dialog(app, fields, "ok")?;
    let rs = result_sheet(app, result_sheet_id)?;
    ensure(rs.status == ResultStatus::UpToDate, format!("edited `{result_sheet_id}` expected UpToDate, got {:?}", rs.status))?;
    Ok(out)
}

fn exclude_cell_cmd_shift_e(app: &mut UiApp, sheet_id: &str, row: usize, col: usize, subcol: usize, reason_tag: &str, note: &str) -> TestRes {
    select_sheet(app, sheet_id)?;
    click_widget(app, &format!("grid.cell.{row}.{col}.{subcol}"))?;
    press_shortcut(app, "e", json!({ "meta": true, "shift": true }))?;
    let snap = inspect_ui(app, "dialog")?;
    let kind = snap.pointer("/dialog/dialog_kind").or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "exclude_with_reason", format!("expected exclude_with_reason dialog via Cmd+Shift+E, got `{kind}`"))?;
    answer_dialog(app, json!({ "reason_tag": reason_tag, "note": note }), "ok")?;
    let ds = data_sheet(app, sheet_id)?;
    let coord = CellCoord::new(row, col, subcol);
    ensure(ds.cell(coord).is_some_and(|c| c.excluded), format!("cell ({row},{col},{subcol}) must be excluded"))?;
    ensure(ds.exclusion_note(coord).is_some_and(|n| n.reason_tag == reason_tag), "exclusion reason note missing")?;
    Ok(())
}

fn switch_graph_type(app: &mut UiApp, graph_id: &str, option_id: &str) -> TestRes {
    select_sheet(app, graph_id)?;
    invoke_menu_cmd(app, "ui.changeGraphType.open", json!({}))?;
    let snap = inspect_ui(app, "dialog")?;
    let kind = snap.pointer("/dialog/dialog_kind").or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "change_graph_type", format!("expected change_graph_type, got `{kind}`"))?;
    answer_dialog(app, json!({ "selected_option_id": option_id }), "ok")?;
    Ok(())
}

fn find_widget<'a>(snap: &'a Value, id: &str) -> Option<&'a Value> {
    let match_item = |w: &&Value| w.get("widget_id").or_else(|| w.get("id")).and_then(Value::as_str) == Some(id);
    for key in ["controls", "buttons", "tabs"] {
        if let Some(found) = snap.get("dialog").and_then(|d| d.get(key)).and_then(Value::as_array).and_then(|arr| arr.iter().find(match_item)) {
            return Some(found);
        }
    }
    for key in ["toolbar", "grid_cells", "navigator_sheets", "canvas_elements", "tabs"] {
        if let Some(found) = snap.get(key).and_then(Value::as_array).and_then(|arr| arr.iter().find(match_item)) {
            return Some(found);
        }
    }
    None
}

fn data_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a DataSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    app.document().data_sheet(&sid).ok_or_else(|| format!("data sheet `{id}` not found"))
}

fn result_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a ResultSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    app.document().result_sheet(&sid).ok_or_else(|| format!("result sheet `{id}` not found"))
}

fn linked_graph_id(app: &UiApp, data_sheet_id: &str) -> Result<String, String> {
    app.document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id.as_str() == data_sheet_id)
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| format!("no GraphSheet linked to `{data_sheet_id}`"))
}

fn tab_by_id<'a>(rs: &'a ResultSheet, tab_id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", rs.id))
}

fn scalar_val(rs: &ResultSheet, key: &str, ds_idx: Option<usize>) -> Result<f64, String> {
    rs.lookup_scalar(key, ds_idx).ok_or_else(|| format!("missing scalar `{key}` (ds={ds_idx:?}) in `{}`", rs.id))
}

fn cell_num(cell: &ResultCell, ctx: &str) -> Result<f64, String> {
    match cell {
        ResultCell::Number { value, .. } if value.is_finite() => Ok(*value),
        other => Err(format!("expected finite numeric cell for `{ctx}`, got {other:?}")),
    }
}

fn cell_p_val(cell: &ResultCell, ctx: &str) -> Result<f64, String> {
    match cell {
        ResultCell::PValue { raw, .. } | ResultCell::Number { value: raw, .. } if raw.is_finite() => Ok(*raw),
        other => Err(format!("expected P-value cell for `{ctx}`, got {other:?}")),
    }
}

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) -> TestRes {
    ensure(approx_eq(actual, expected, tol, 0.0), format!("{label}: expected ~{expected}, got {actual} (tol={tol})"))
}

fn compile_scene(app: &UiApp, graph_id: &str) -> Result<ChartScene, String> {
    let gid = SheetId::try_new(graph_id).map_err(|e| e.to_string())?;
    let gs = app.document().graph_sheet(&gid).ok_or_else(|| format!("graph `{graph_id}` not found"))?;
    compile_graph(gs, app.document()).map_err(|e| format!("compile_graph({graph_id}) failed: {e}"))
}

fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn assert_screenshot_png(app: &mut UiApp, target: &str) -> TestRes {
    let shot = gui(app, "ui.screenshot", json!({ "target": target, "width_px": 900, "height_px": 600, "dpi": 144 }))?;
    let b64 = shot
        .get("data_base64")
        .or_else(|| shot.get("base64"))
        .or_else(|| shot.get("png_base64"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ui.screenshot({target}) missing base64 PNG: {shot}"))?;
    ensure(b64.len() > 100 && b64.starts_with("iVBORw0K"), format!("invalid PNG base64 for target `{target}`: len={}", b64.len()))
}

fn assert_qol7_diagnostics_only(app: &mut UiApp, result_sheet_id: &str) -> TestRes {
    {
        let rs = result_sheet(app, result_sheet_id)?;
        ensure(!rs.checklist.is_empty(), format!("expected QOL-7 checklist items on `{result_sheet_id}`"))?;
        for item in &rs.checklist {
            ensure(item.remediation.is_none(), format!("QOL-7 diagnostics-only violation on `{}`: {:?}", item.assumption, item.remediation))?;
        }
    }
    select_sheet(app, result_sheet_id)?;
    let snap = inspect_ui(app, "all")?;
    ensure(find_widget(&snap, "result.diagnostic_banner").is_some(), "expected visible `result.diagnostic_banner` widget")?;
    let click_out = click_widget(app, "result.diagnostic_banner")?;
    ensure(click_out.get("mutated").and_then(Value::as_bool) == Some(false), format!("diagnostic banner click must not mutate: {click_out}"))?;
    let blocked = dispatch_gui_action(
        app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": result_sheet_id } }),
    );
    ensure(blocked.is_err(), "ui.result.applyRemediation must be disabled/rejected under QOL-7 diagnostics-only policy")?;
    Ok(())
}

// ============================================================================
// WF-37: Hierarchical Clustering (4 Distances x 7 Linkages, Cophenetic r,
//        Dendrogram & Linked Clustered Heatmap)
// ============================================================================

#[test]
fn wf37_hierarchical_clustering_linkages_cophenetic_and_dendrogram() -> TestRes {
    let mut app = new_strict_app()?;

    // Craft 5 labeled samples where Euclidean/Ward pairs RowSmallPos with RowSmallNeg,
    // whereas Pearson/Complete pairs RowSmallPos with RowScaledPos (r = +1.0).
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf37-mv",
            "title": "Gene Expression Profiles",
            "kind": "multiple_variables",
            "rows": 5,
            "cols": 4,
            "variables": [
                { "title": "Sample", "role": "label" },
                { "title": "GeneA", "role": "continuous" },
                { "title": "GeneB", "role": "continuous" },
                { "title": "GeneC", "role": "continuous" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "RowSmallPos\t1.0\t2.0\t3.0\n\
         RowMid\t10.0\t15.0\t12.0\n\
         RowSmallNeg\t3.0\t2.0\t1.0\n\
         RowScaledPos\t100.0\t200.0\t300.0\n\
         RowMidNeg\t12.0\t10.0\t8.0",
    )?;
    let graph_id = linked_graph_id(&app, &mv_id)?;

    let clust_res_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.clustering",
            "selected_data_sets": [1, 2, 3],
            "distance": "euclidean",
            "linkage": "ward",
            "k_means_k": Value::Null
        }),
    )?;

    let distances = [
        ("manhattan", ClusterDistance::Manhattan),
        ("pearson", ClusterDistance::Pearson),
        ("cosine", ClusterDistance::Cosine),
        ("euclidean", ClusterDistance::Euclidean),
    ];
    let linkages = [
        ("complete", ClusterLinkage::Complete),
        ("single", ClusterLinkage::Single),
        ("upgma", ClusterLinkage::Upgma),
        ("wpgma", ClusterLinkage::Wpgma),
        ("centroid", ClusterLinkage::Centroid),
        ("median", ClusterLinkage::Median),
        ("ward", ClusterLinkage::Ward),
    ];

    // Sweep all 4 distances x 7 linkages via Cmd+T (`EditAnalysisParameters`) and verify
    // dendrogram, distance/cophenetic matrix symmetry, silhouette cuts, and cophenetic_r.
    let mut upgma_coph_r = Vec::new();
    for (dist_str, dist_enum) in distances {
        for (link_str, link_enum) in linkages {
            edit_params_cmd_t(&mut app, &clust_res_id, json!({ "edit.cluster.distance": dist_str, "edit.cluster.linkage": link_str }))?;
            let rs = result_sheet(&app, &clust_res_id)?;
            ensure(
                matches!(
                    rs.analysis.kind,
                    AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, linkage, .. })
                        if distance == dist_enum && linkage == link_enum
                ),
                format!("unexpected Clustering spec for ({dist_str}, {link_str}): {:?}", rs.analysis.kind),
            )?;
            let coph_r = scalar_val(rs, "cophenetic_r", None)?;
            ensure(
                coph_r.is_finite() && (-1.0 - 1e-9..=1.0 + 1e-9).contains(&coph_r),
                format!("cophenetic_r out of [-1,1] for ({dist_str},{link_str}): {coph_r}"),
            )?;
            if link_enum == ClusterLinkage::Upgma {
                upgma_coph_r.push(coph_r);
            }

            let dendro = tab_by_id(rs, "hierarchical-dendrogram")?;
            ensure(dendro.rows.len() == 4, format!("5 objects must yield 4 merges for ({dist_str},{link_str}), got {}", dendro.rows.len()))?;
            let sils = tab_by_id(rs, "hierarchical-cut-silhouettes")?;
            ensure(!sils.rows.is_empty(), format!("missing hierarchical-cut-silhouettes for ({dist_str},{link_str})"))?;

            let dmat = tab_by_id(rs, "clustering-distance-matrix")?;
            let cmat = tab_by_id(rs, "cophenetic-distance-matrix")?;
            ensure(dmat.rows.len() == 5 && cmat.rows.len() == 5, "distance & cophenetic matrices must be 5x5")?;
            for i in 0..5 {
                assert_close(cell_num(&dmat.rows[i][i], "dmat diag")?, 0.0, 1e-12, "dmat diagonal")?;
                assert_close(cell_num(&cmat.rows[i][i], "cmat diag")?, 0.0, 1e-12, "cmat diagonal")?;
                for j in 0..5 {
                    assert_close(cell_num(&dmat.rows[i][j], "dmat ij")?, cell_num(&dmat.rows[j][i], "dmat ji")?, 1e-10, "dmat symmetry")?;
                    assert_close(cell_num(&cmat.rows[i][j], "cmat ij")?, cell_num(&cmat.rows[j][i], "cmat ji")?, 1e-10, "cmat symmetry")?;
                }
            }
        }
    }
    ensure(upgma_coph_r.iter().all(|&r| r > 0.45), format!("UPGMA cophenetic_r expected strong tree fidelity: {upgma_coph_r:?}"))?;

    // Loop ends on Euclidean / Ward -> switch graph to Heatmap and verify row order + dendrogram cues.
    switch_graph_type(&mut app, &graph_id, "diag.heatmap")?;
    let row_y_order = |scene: &ChartScene| -> Vec<String> {
        let mut labels: Vec<(f64, String)> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => {
                    let s = t.rich.plain_text();
                    s.starts_with("Row").then_some((t.anchor_pt.y, s))
                }
                _ => None,
            })
            .collect();
        labels.sort_by(|a, b| a.0.total_cmp(&b.0));
        labels.into_iter().map(|(_, s)| s).collect()
    };
    let euc_scene = compile_scene(&app, &graph_id)?;
    let euc_texts = scene_texts(&euc_scene);
    ensure(euc_texts.iter().any(|t| t == "Clustered: Euclidean / Ward"), format!("missing Euclidean/Ward cue: {euc_texts:?}"))?;
    let euc_order = row_y_order(&euc_scene);
    let euc_pos = |name: &str| -> Result<isize, String> {
        euc_order.iter().position(|s| s == name).map(|i| i as isize).ok_or_else(|| format!("missing `{name}` in {euc_order:?}"))
    };
    ensure(
        (euc_pos("RowSmallPos")? - euc_pos("RowSmallNeg")?).abs() == 1,
        format!("Euclidean/Ward should pair RowSmallPos & RowSmallNeg: {euc_order:?}"),
    )?;
    let dendro_segments = euc_scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() >= 4 && matches!(path.commands.first(), Some(PathCmd::MoveTo(_)))))
        .count();
    ensure(dendro_segments >= 4, format!("expected dendrogram branches on heatmap, got {dendro_segments}"))?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Switch clustering ResultSheet via Cmd+T to Pearson / Complete -> Heatmap must dynamically follow.
    edit_params_cmd_t(&mut app, &clust_res_id, json!({ "edit.cluster.distance": "pearson", "edit.cluster.linkage": "complete" }))?;
    let pearson_scene = compile_scene(&app, &graph_id)?;
    let pearson_texts = scene_texts(&pearson_scene);
    ensure(pearson_texts.iter().any(|t| t == "Clustered: Pearson / Complete"), format!("missing Pearson/Complete cue: {pearson_texts:?}"))?;
    let p_order = row_y_order(&pearson_scene);
    let p_pos = |name: &str| -> Result<isize, String> {
        p_order.iter().position(|s| s == name).map(|i| i as isize).ok_or_else(|| format!("missing `{name}` in {p_order:?}"))
    };
    ensure(
        (p_pos("RowSmallPos")? - p_pos("RowScaledPos")?).abs() == 1,
        format!("Pearson/Complete must pair RowSmallPos & RowScaledPos: {p_order:?}"),
    )?;
    assert_screenshot_png(&mut app, "workspace")?;

    // Live upstream cell edit triggers immediate reactive clustering recomputation.
    let coph_before = scalar_val(result_sheet(&app, &clust_res_id)?, "cophenetic_r", None)?;
    select_sheet(&mut app, &mv_id)?;
    type_widget(&mut app, "grid.cell.0.3.0", "-9.0")?;
    let coph_after = scalar_val(result_sheet(&app, &clust_res_id)?, "cophenetic_r", None)?;
    ensure(
        (coph_after - coph_before).abs() > 1e-4,
        format!("upstream cell edit must recompute clustering cophenetic_r ({coph_before} -> {coph_after})"),
    )?;
    Ok(())
}

// ============================================================================
// WF-38: K-Means Clustering (Elbow/Silhouette Sweep, Append Cluster Column,
//        Cluster Scatter with Confidence Ellipses & Convex Hulls)
// ============================================================================

#[test]
fn wf38_kmeans_clustering_silhouette_elbow_and_cluster_scatter() -> TestRes {
    let mut app = new_strict_app()?;

    // 12 samples forming 3 tight, well-separated clusters (rows 0..4, 4..8, 8..12).
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf38-mv",
            "title": "Single-Cell Markers",
            "kind": "multiple_variables",
            "rows": 12,
            "cols": 4,
            "variables": [
                { "title": "CellId", "role": "label" },
                { "title": "MarkerA", "role": "continuous" },
                { "title": "MarkerB", "role": "continuous" },
                { "title": "MarkerC", "role": "continuous" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "C01\t1.0\t1.1\t0.9\n\
         C02\t1.2\t0.9\t1.1\n\
         C03\t0.8\t1.2\t1.0\n\
         C04\t1.1\t1.0\t1.2\n\
         C05\t10.0\t10.2\t1.1\n\
         C06\t10.3\t9.9\t0.9\n\
         C07\t9.8\t10.1\t1.2\n\
         C08\t10.1\t9.8\t1.0\n\
         C09\t2.0\t11.0\t12.1\n\
         C10\t2.2\t10.8\t11.9\n\
         C11\t1.9\t11.2\t12.0\n\
         C12\t2.1\t10.9\t12.2",
    )?;
    let graph_id = linked_graph_id(&app, &mv_id)?;

    // Auto-k via silhouette (`k_means_k: null`) + elbow/silhouette sweep up to k=6.
    let res_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.clustering",
            "selected_data_sets": [1, 2, 3],
            "distance": "euclidean",
            "linkage": "ward",
            "k_means_k": Value::Null,
            "k_sweep_max": 6
        }),
    )?;

    let wcss_k3 = {
        let rs = result_sheet(&app, &res_id)?;
        assert_close(scalar_val(rs, "Clusters (k)", None)?, 3.0, 1e-12, "auto-selected optimal k")?;
        let mean_sil = scalar_val(rs, "Mean silhouette", None)?;
        ensure(mean_sil > 0.85, format!("expected high silhouette on 3 clear clusters, got {mean_sil}"))?;
        let wcss3 = scalar_val(rs, "Total WCSS", None)?;

        let sweep = tab_by_id(rs, "kmeans-sweep-curve")?;
        ensure(sweep.rows.len() == 5, format!("expected k=2..=6 (5 rows) in kmeans-sweep-curve, got {}", sweep.rows.len()))?;
        let wcss_col = sweep.column_headers.iter().position(|h| h == "WCSS").ok_or("missing WCSS column")?;
        let sil_col = sweep.column_headers.iter().position(|h| h.contains("silhouette")).ok_or("missing silhouette column")?;
        let mut prev_wcss = f64::INFINITY;
        for row in &sweep.rows {
            let w = cell_num(&row[wcss_col], "sweep WCSS")?;
            let s = cell_num(&row[sil_col], "sweep silhouette")?;
            ensure(w <= prev_wcss + 1e-9, format!("WCSS must monotonically decrease over k: prev={prev_wcss}, cur={w}"))?;
            ensure((-1.0..=1.0).contains(&s), format!("invalid silhouette in sweep: {s}"))?;
            prev_wcss = w;
        }
        ensure(tab_by_id(rs, "kmeans-clusters")?.rows.len() == 3, "expected 3 cluster summary rows")?;
        ensure(tab_by_id(rs, "kmeans-assignments")?.rows.len() == 12, "expected 12 cluster assignment rows")?;
        wcss3
    };

    // Cmd+T explicit override k=4 reduces WCSS, then restore auto-k=3.
    edit_params_cmd_t(&mut app, &res_id, json!({ "edit.cluster.k_means_k": 4 }))?;
    let wcss_k4 = scalar_val(result_sheet(&app, &res_id)?, "Total WCSS", None)?;
    assert_close(scalar_val(result_sheet(&app, &res_id)?, "Clusters (k)", None)?, 4.0, 1e-12, "manual k=4")?;
    ensure(wcss_k4 < wcss_k3, format!("k=4 WCSS ({wcss_k4}) must be < k=3 WCSS ({wcss_k3})"))?;
    edit_params_cmd_t(&mut app, &res_id, json!({ "edit.cluster.k_means_k": 3 }))?;

    // Append categorical "Cluster" column onto source MultipleVariables sheet via UI menu command.
    select_sheet(&mut app, &res_id)?;
    let append_out = invoke_menu_cmd(&mut app, "ui.result.appendClusterColumn", json!({}))?;
    ensure(
        append_out.pointer("/result/updated_existing").and_then(Value::as_bool) == Some(false),
        format!("expected new Cluster col: {append_out}"),
    )?;
    {
        let ds = data_sheet(&app, &mv_id)?;
        let TableArchetype::MultipleVariables(ref mv) = ds.archetype else {
            return Err(String::from("expected MV archetype"));
        };
        ensure(
            mv.variables.len() == 5 && mv.variables[4].title == "Cluster" && mv.variables[4].role == VariableRole::Categorical,
            "missing appended Cluster categorical col",
        )?;
        let cell_text = |r: usize| match ds.archetype.data_cell(CellCoord::new(r, 4, 0)).map(|c| &c.value) {
            Some(CellValue::Text(s)) => Some(s.as_str()),
            _ => None,
        };
        let (c0, c4, c8) = (cell_text(0), cell_text(4), cell_text(8));
        ensure(c0 != c4 && c4 != c8 && c0 != c8, format!("distinct clusters expected for C01/C05/C09: {c0:?}, {c4:?}, {c8:?}"))?;
    }

    // Configure MV cluster scatter with categorical colors, confidence ellipses & convex hulls via BubblePlotInspector.
    select_sheet(&mut app, &graph_id)?;
    invoke_menu_cmd(&mut app, "ui.graph.bubblePlot.open", json!({}))?;
    let bubble_dlg = inspect_ui(&mut app, "dialog")?;
    for wid in [
        "bubble.plot_mode",
        "bubble.x_column",
        "bubble.y_column",
        "bubble.color_mode",
        "bubble.color_column",
        "bubble.show_convex_hulls",
        "bubble.show_confidence_ellipses",
    ] {
        ensure(find_widget(&bubble_dlg, wid).is_some(), format!("missing BubblePlotInspector control `{wid}`"))?;
    }
    answer_dialog(
        &mut app,
        json!({
            "plot_mode": "bubble_scatter",
            "x_column": 1,
            "y_column": 2,
            "color_mode": "categorical",
            "color_column": 4,
            "label_column": 0,
            "show_convex_hulls": true,
            "show_confidence_ellipses": true
        }),
        "ok",
    )?;
    let gs = app.document().graph_sheet(&SheetId::try_new(&graph_id).map_err(|e| e.to_string())?).ok_or("missing wf38 graph")?;
    let enc = gs.spec.multivariate.as_ref().ok_or("missing multivariate encoding")?;
    ensure(
        enc.plot_mode == MultivariatePlotMode::BubbleScatter && enc.show_convex_hulls && enc.show_confidence_ellipses,
        "scatter hulls/ellipses not persisted",
    )?;
    let scene = compile_scene(&app, &graph_id)?;
    ensure(!scene.warnings.iter().any(|w| w.code.starts_with("empty_")), format!("unexpected warning in scatter scene: {:?}", scene.warnings))?;
    ensure(scene.primitives.len() >= 15, format!("expected points + hulls/ellipses primitives, got {}", scene.primitives.len()))?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Cmd+Shift+E exclude sample row 0 -> reactive DAG recompute shrinks kmeans-assignments & updates WCSS; re-append updates in-place.
    exclude_cell_cmd_shift_e(&mut app, &mv_id, 0, 1, 0, "QC", "low RNA yield")?;
    let rs_excl = result_sheet(&app, &res_id)?;
    let wcss_after_excl = scalar_val(rs_excl, "Total WCSS", None)?;
    ensure((wcss_after_excl - wcss_k3).abs() > 1e-6, format!("exclusion must update WCSS ({wcss_k3} -> {wcss_after_excl})"))?;
    ensure(tab_by_id(rs_excl, "kmeans-assignments")?.rows.len() == 11, "expected 11 active rows after excluding row 0")?;
    select_sheet(&mut app, &res_id)?;
    let reappend = invoke_menu_cmd(&mut app, "ui.result.appendClusterColumn", json!({}))?;
    ensure(
        reappend.pointer("/result/updated_existing").and_then(Value::as_bool) == Some(true),
        format!("repeat append must update existing Cluster col: {reappend}"),
    )?;
    Ok(())
}

// ============================================================================
// WF-39: Cox Proportional Hazards (Categorical Reference Levels, Efron vs
//        Breslow Ties, AIC, Correlation Matrix, Survival Curve Graph)
// ============================================================================

#[test]
fn wf39_cox_proportional_hazards_efron_ties_and_covariate_curves() -> TestRes {
    let mut app = new_strict_app()?;

    // 14 patients with intentional TIED failure times (t=4.0, t=7.0, t=12.0) so Efron != Breslow.
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf39-mv",
            "title": "Oncology Survival Trial",
            "kind": "multiple_variables",
            "rows": 14,
            "cols": 4,
            "variables": [
                { "title": "Time", "role": "continuous" },
                { "title": "Status", "role": "continuous" },
                { "title": "Arm", "role": "categorical" },
                { "title": "Age", "role": "continuous" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "2.0\t1\tPlacebo\t68\n\
         4.0\t1\tPlacebo\t66\n\
         4.0\t1\tPlacebo\t64\n\
         6.0\t1\tPlacebo\t62\n\
         8.0\t0\tPlacebo\t60\n\
         4.0\t1\tLowDose\t65\n\
         7.0\t1\tLowDose\t63\n\
         7.0\t1\tLowDose\t59\n\
         11.0\t0\tLowDose\t56\n\
         12.0\t1\tLowDose\t61\n\
         9.0\t0\tHighDose\t58\n\
         12.0\t1\tHighDose\t64\n\
         12.0\t0\tHighDose\t55\n\
         16.0\t1\tHighDose\t62",
    )?;
    let graph_id = linked_graph_id(&app, &mv_id)?;

    let cox_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.survival_cox",
            "cox_time_column": 0,
            "cox_status_column": 1,
            "cox_predictor_columns": [2, 3],
            "reference_levels": [[2, "Placebo"]],
            "tie_method": "efron"
        }),
    )?;

    let (efron_aic, efron_beta_low, efron_hr_high) = {
        let rs = result_sheet(&app, &cox_id)?;
        ensure(
            matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Efron, .. })),
            "expected Efron Cox PH spec",
        )?;
        let ph_tab = tab_by_id(rs, "cox-ph")?;
        ensure(
            ph_tab.row_labels == vec![String::from("Arm[LowDose vs Placebo]"), String::from("Arm[HighDose vs Placebo]"), String::from("Age")],
            format!("unexpected Cox row labels: {:?}", ph_tab.row_labels),
        )?;
        let beta_low = cell_num(&ph_tab.rows[0][0], "beta LowDose")?;
        let beta_high = cell_num(&ph_tab.rows[1][0], "beta HighDose")?;
        let hr_high = cell_num(&ph_tab.rows[1][4], "HR HighDose")?;
        assert_close(hr_high, beta_high.exp(), 1e-10, "HR == exp(beta)")?;
        ensure(hr_high < 1.0 && beta_high < 0.0, format!("HighDose should reduce hazard vs Placebo: beta={beta_high}, HR={hr_high}"))?;

        let c_idx = scalar_val(rs, "Concordance (C-index)", None)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let lr_p = scalar_val(rs, "Likelihood ratio P", None)?;
        let global_schoen = scalar_val(rs, "Global Schoenfeld P", None)?;
        ensure(c_idx > 0.65 && c_idx <= 1.0, format!("unexpected C-index {c_idx}"))?;
        ensure(aic.is_finite() && lr_p.is_finite() && (0.0..=1.0).contains(&global_schoen), "invalid Cox global scalars")?;

        let cov_tab = tab_by_id(rs, "cox-covariance")?;
        ensure(cov_tab.kind == ResultTabKind::CovarianceMatrix && cov_tab.rows.len() == 3, "expected 3x3 cox-covariance")?;
        for i in 0..3 {
            assert_close(cell_num(&cov_tab.rows[i][i], "cov diag")?, 1.0, 1e-10, "correlation diag == 1")?;
            for j in 0..3 {
                assert_close(cell_num(&cov_tab.rows[i][j], "cij")?, cell_num(&cov_tab.rows[j][i], "cji")?, 1e-10, "correlation symmetric")?;
            }
        }
        tab_by_id(rs, "cox-baseline-survival")?;
        tab_by_id(rs, "cox-log-minus-log")?;
        ensure(rs.fitted_curves.iter().any(|c| c.equation_id == "cox_baseline"), "missing cox_baseline fitted curve")?;
        ensure(rs.fitted_curves.iter().filter(|c| c.equation_id == "cox_covariate").count() == 3, "expected 3 cox_covariate curves")?;
        (aic, beta_low, hr_high)
    };

    // Switch to Breslow via Cmd+T -> tied times must produce different AIC & beta.
    edit_params_cmd_t(&mut app, &cox_id, json!({ "edit.cox.tie_method": "breslow" }))?;
    let breslow_aic = scalar_val(result_sheet(&app, &cox_id)?, "AIC", None)?;
    ensure((breslow_aic - efron_aic).abs() > 1e-6, format!("tied events must separate Efron ({efron_aic}) vs Breslow ({breslow_aic})"))?;

    // Restore Efron and swap reference level Placebo -> LowDose: reciprocal contrast flips beta sign, AIC invariant!
    edit_params_cmd_t(&mut app, &cox_id, json!({ "edit.cox.tie_method": "efron", "edit.cox.reference_levels": [[2, "LowDose"]] }))?;
    {
        let rs_swapped = result_sheet(&app, &cox_id)?;
        let ph_swapped = tab_by_id(rs_swapped, "cox-ph")?;
        ensure(ph_swapped.row_labels[0] == "Arm[Placebo vs LowDose]", format!("expected flipped contrast label, got {:?}", ph_swapped.row_labels))?;
        let beta_placebo_vs_low = cell_num(&ph_swapped.rows[0][0], "Placebo vs LowDose beta")?;
        assert_close(beta_placebo_vs_low, -efron_beta_low, 1e-5, "reference level flip negates beta")?;
        assert_close(scalar_val(rs_swapped, "AIC", None)?, efron_aic, 1e-5, "AIC invariant to reference level")?;
    }
    edit_params_cmd_t(&mut app, &cox_id, json!({ "edit.cox.reference_levels": [[2, "Placebo"]] }))?;

    // Switch MV graph to Survival (`survival.km`) -> renders Cox Baseline + Covariate survival curves cleanly.
    switch_graph_type(&mut app, &graph_id, "survival.km")?;
    let surv_scene = compile_scene(&app, &graph_id)?;
    ensure(!surv_scene.warnings.iter().any(|w| w.code == "empty_survival_data"), format!("empty_survival_data warning: {:?}", surv_scene.warnings))?;
    let texts = scene_texts(&surv_scene);
    ensure(texts.iter().any(|t| t == "Baseline"), format!("missing Cox Baseline legend label: {texts:?}"))?;
    ensure(texts.iter().any(|t| t.contains("Arm[HighDose vs Placebo]")), format!("missing Cox covariate legend label: {texts:?}"))?;
    let step_paths = surv_scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() >= 4))
        .count();
    ensure(step_paths >= 4, format!("expected >= 4 Cox survival curves (baseline + 3 covariates), got {step_paths}"))?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Upstream Cmd+Shift+E exclusion recomputes Cox HR & AIC reactively.
    exclude_cell_cmd_shift_e(&mut app, &mv_id, 13, 0, 0, "lost_to_followup", "withdrew consent")?;
    let rs_after = result_sheet(&app, &cox_id)?;
    let hr_after = cell_num(&tab_by_id(rs_after, "cox-ph")?.rows[1][4], "HR after excl")?;
    let aic_after = scalar_val(rs_after, "AIC", None)?;
    ensure((hr_after - efron_hr_high).abs() > 1e-4 && (aic_after - efron_aic).abs() > 1e-3, "exclusion must trigger Cox DAG recompute")?;
    Ok(())
}

// ============================================================================
// WF-40: Multiple Linear Regression (Type III ANOVA, VIF, AIC, Covariance,
//        Residuals/Influence, QOL-7 Diagnostics-Only, Residual & Q-Q Plots)
// ============================================================================

#[test]
fn wf40_multiple_linear_regression_type_iii_vif_and_diagnostics() -> TestRes {
    let mut app = new_strict_app()?;

    // Y = 5.0 + 2.0*Temp + 1.5*Nutrients + 3.0*I(Media=="Rich") + small noise,
    // with correlated Temp & Nutrients (moderate VIF > 1).
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf40-mv",
            "title": "Fermentation Yield",
            "kind": "multiple_variables",
            "rows": 14,
            "cols": 4,
            "variables": [
                { "title": "Biomass", "role": "continuous" },
                { "title": "Temp", "role": "continuous" },
                { "title": "Nutrients", "role": "continuous" },
                { "title": "Media", "role": "categorical" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "10.1\t1.0\t2.0\tControl\n\
         11.3\t2.0\t1.5\tControl\n\
         16.8\t3.0\t4.0\tControl\n\
         18.4\t4.0\t3.5\tControl\n\
         23.9\t5.0\t6.0\tControl\n\
         24.6\t6.0\t5.0\tControl\n\
         31.7\t7.0\t8.5\tControl\n\
         17.3\t1.5\t2.2\tRich\n\
         21.5\t2.5\t3.6\tRich\n\
         24.9\t3.5\t4.5\tRich\n\
         29.8\t4.5\t6.4\tRich\n\
         32.6\t5.5\t7.1\tRich\n\
         38.4\t6.5\t9.5\tRich\n\
         40.2\t7.5\t9.2\tRich",
    )?;
    let resid_graph_id = linked_graph_id(&app, &mv_id)?;

    let glm_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.glm",
            "glm_family": "linear",
            "response_column": 0,
            "selected_data_sets": [1, 2, 3],
            "reference_levels": [[3, "Control"]],
            "compute_vif": true
        }),
    )?;

    let r2_initial = {
        let rs = result_sheet(&app, &glm_id)?;
        let r2 = scalar_val(rs, "R squared", None)?;
        let adj_r2 = scalar_val(rs, "Adjusted R squared", None)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let dw = scalar_val(rs, "Durbin-Watson", None)?;
        ensure(
            r2 > 0.97 && adj_r2 > 0.95 && aic.is_finite() && dw > 0.5,
            format!("unexpected Linear GLM scalars: R2={r2}, adj={adj_r2}, aic={aic}"),
        )?;

        let params = tab_by_id(rs, "glm-linear-params")?;
        let vif_col = params.column_headers.iter().position(|h| h == "VIF").ok_or("missing VIF column in glm-linear-params")?;
        ensure(params.rows[0][vif_col] == ResultCell::Empty, "Intercept VIF must be Empty")?;
        let vif1 = cell_num(&params.rows[1][vif_col], "Temp VIF")?;
        let vif2 = cell_num(&params.rows[2][vif_col], "Nutrients VIF")?;
        ensure(vif1 > 2.0 && vif2 > 2.0, format!("correlated Temp/Nutrients must yield VIF > 2: {vif1}, {vif2}"))?;

        ensure(tab_by_id(rs, "glm-linear-anova")?.rows.len() == 3, "glm-linear-anova must retain Regression/Residual/Total")?;
        let t3 = tab_by_id(rs, "glm-linear-type-iii")?;
        ensure(
            t3.kind == ResultTabKind::AnovaTable && t3.column_headers == ["SS", "df", "MS", "F", "P value"],
            format!("unexpected Type III headers: {:?}", t3.column_headers),
        )?;
        ensure(t3.row_labels == ["Temp", "Nutrients", "Media[Rich vs Control]"], format!("unexpected Type III rows: {:?}", t3.row_labels))?;
        for r in 0..3 {
            ensure(
                cell_num(&t3.rows[r][3], "Type III F")? > 5.0 && cell_p_val(&t3.rows[r][4], "Type III P")? < 0.05,
                "Type III term expected significant",
            )?;
        }

        let cov = tab_by_id(rs, "glm-linear-covariance")?;
        ensure(cov.kind == ResultTabKind::CovarianceMatrix && cov.rows.len() == 4, "expected 4x4 glm-linear-covariance")?;
        for j in 0..4 {
            let se = cell_num(&params.rows[j][1], "SE")?;
            assert_close(cell_num(&cov.rows[j][j], "cov jj")?.sqrt(), se, 1e-8, "sqrt(Cov diag) == SE")?;
        }

        let resid = tab_by_id(rs, "glm-linear-residuals")?;
        ensure(resid.kind == ResultTabKind::Residuals && resid.rows.len() == 14, "expected 14 rows in glm-linear-residuals")?;
        let mut lev_sum = 0.0;
        for row in &resid.rows {
            let obs = cell_num(&row[0], "Observed Y")?;
            let pred = cell_num(&row[1], "Predicted Y")?;
            let r_val = cell_num(&row[2], "Residual")?;
            let lev = cell_num(&row[3], "Leverage")?;
            let cook = cell_num(&row[4], "Cook's D")?;
            assert_close(r_val, obs - pred, 1e-10, "Residual == Observed - Predicted")?;
            ensure(cook >= 0.0, "Cook's D non-negative")?;
            lev_sum += lev;
        }
        assert_close(lev_sum, 4.0, 1e-8, "sum of leverages == p (4)")?;
        ensure(rs.checklist.iter().any(|c| c.assumption == "Residual normality"), "missing Residual normality checklist")?;
        r2
    };

    // QOL-7 diagnostics-only adherence & non-remediation verification.
    assert_qol7_diagnostics_only(&mut app, &glm_id)?;

    // Switch auto graph to Residual Plot (`diag.residual`) + create second graph for Normal Q-Q Plot (`diag.normal_qq`).
    switch_graph_type(&mut app, &resid_graph_id, "diag.residual")?;
    let resid_scene = compile_scene(&app, &resid_graph_id)?;
    let resid_pts = resid_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();
    ensure(resid_pts >= 14, format!("expected >= 14 points on ResidualPlot, got {resid_pts}"))?;
    let has_dashed = resid_scene.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if path.commands.len() == 2 && st.dash == LineDashStyle::Dashed)
    });
    ensure(has_dashed, "expected dashed zero line on ResidualPlot")?;
    assert_screenshot_png(&mut app, "canvas")?;

    invoke_menu_cmd(&mut app, "graph.create", json!({ "data_sheet_id": mv_id, "sheet_id": "wf40-qq-graph" }))?;
    switch_graph_type(&mut app, "wf40-qq-graph", "diag.normal_qq")?;
    let qq_scene = compile_scene(&app, "wf40-qq-graph")?;
    let qq_pts = qq_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();
    ensure(qq_pts >= 14, format!("expected >= 14 points on NormalQqPlot, got {qq_pts}"))?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Live edit outlier Y -> triggers Residual normality Warn (still remediation: None), then Cmd+Shift+E exclude restores high R2.
    select_sheet(&mut app, &mv_id)?;
    type_widget(&mut app, "grid.cell.13.0.0", "250.0")?;
    {
        let rs_skew = result_sheet(&app, &glm_id)?;
        ensure(scalar_val(rs_skew, "R squared", None)? < r2_initial - 0.1, "outlier edit must lower R2 immediately")?;
        let norm = rs_skew.checklist.iter().find(|c| c.assumption == "Residual normality").ok_or("missing Residual normality")?;
        ensure(norm.status == ChecklistStatus::Warn && norm.remediation.is_none(), format!("expected QOL-7 Warn with no remediation: {norm:?}"))?;
    }
    exclude_cell_cmd_shift_e(&mut app, &mv_id, 13, 0, 0, "outlier", "contaminated bioreactor")?;
    let rs_recovered = result_sheet(&app, &glm_id)?;
    ensure(scalar_val(rs_recovered, "R squared", None)? > 0.97, "excluding outlier must recover R2 > 0.97")?;
    ensure(tab_by_id(rs_recovered, "glm-linear-residuals")?.rows.len() == 13, "expected 13 residual rows after exclusion")?;
    Ok(())
}

// ============================================================================
// WF-41: Multiple Logistic Regression (VIF, AIC, Hosmer-Lemeshow, 2x2 & Sweep
//        Classification Table, Row Predictions, Linked ROC Curve Graph)
// ============================================================================

#[test]
fn wf41_multiple_logistic_classification_hosmer_lemeshow_and_roc() -> TestRes {
    let mut app = new_strict_app()?;

    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf41-mv",
            "title": "Biomarker Classifier",
            "kind": "multiple_variables",
            "rows": 16,
            "cols": 4,
            "variables": [
                { "title": "Outcome", "role": "continuous" },
                { "title": "Dose", "role": "continuous" },
                { "title": "Biomarker", "role": "continuous" },
                { "title": "Genotype", "role": "categorical" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "0\t1.0\t2.0\tWT\n\
         0\t1.5\t2.4\tWT\n\
         0\t2.0\t2.8\tMut\n\
         0\t2.5\t3.1\tWT\n\
         1\t2.1\t2.7\tWT\n\
         0\t3.2\t3.6\tWT\n\
         0\t3.6\t4.0\tWT\n\
         1\t4.0\t4.5\tMut\n\
         0\t4.9\t5.0\tWT\n\
         1\t4.8\t5.2\tMut\n\
         1\t5.2\t5.6\tWT\n\
         1\t5.6\t6.0\tMut\n\
         1\t6.0\t6.5\tMut\n\
         1\t6.5\t6.9\tWT\n\
         1\t7.0\t7.3\tMut\n\
         1\t7.5\t7.8\tMut",
    )?;
    let mv_graph_id = linked_graph_id(&app, &mv_id)?;

    let log_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.glm",
            "glm_family": "logistic",
            "response_column": 0,
            "selected_data_sets": [1, 2, 3],
            "reference_levels": [[3, "WT"]],
            "compute_vif": true
        }),
    )?;

    let (auc_initial, aic_initial) = {
        let rs = result_sheet(&app, &log_id)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let hl_chi2 = scalar_val(rs, "Hosmer-Lemeshow chi2", None)?;
        let hl_p = scalar_val(rs, "Hosmer-Lemeshow P", None)?;
        let auc = scalar_val(rs, "ROC AUC", None)?;
        ensure(aic.is_finite() && hl_chi2 >= 0.0 && (0.0..=1.0).contains(&hl_p), format!("invalid Logistic scalars: AIC={aic}, HL_P={hl_p}"))?;
        ensure(auc > 0.80 && auc < 1.0, format!("expected non-degenerate high Logistic ROC AUC in (0.80, 1.0), got {auc}"))?;

        let params = tab_by_id(rs, "glm-logistic-params")?;
        ensure(
            params.column_headers.len() == 7 && params.column_headers[6] == "VIF",
            format!("expected 7 columns with VIF, got {:?}", params.column_headers),
        )?;
        ensure(params.row_labels.iter().any(|l| l == "Genotype[Mut vs WT]"), format!("missing categorical contrast row: {:?}", params.row_labels))?;
        ensure(
            params.rows[0][6] == ResultCell::Empty && cell_num(&params.rows[1][6], "Dose VIF")? > 1.0,
            "expected Intercept Empty & predictor VIF > 1",
        )?;

        let cls = tab_by_id(rs, "glm-logistic-classification")?;
        ensure(
            cls.column_headers == ["Cutoff", "TP", "FP", "FN", "TN", "Sensitivity", "Specificity", "PPV", "NPV", "Accuracy"],
            format!("unexpected classification headers: {:?}", cls.column_headers),
        )?;
        ensure(cls.rows.len() == 10, format!("expected cutoff 0.5 + 9 sweep rows, got {}", cls.rows.len()))?;
        assert_close(cell_num(&cls.rows[0][0], "primary cutoff")?, 0.5, 1e-12, "cutoff 0.5 row 0")?;
        let tp = cell_num(&cls.rows[0][1], "TP")?;
        let fp = cell_num(&cls.rows[0][2], "FP")?;
        let fn_c = cell_num(&cls.rows[0][3], "FN")?;
        let tn = cell_num(&cls.rows[0][4], "TN")?;
        assert_close(tp + fp + fn_c + tn, 16.0, 1e-12, "2x2 total count == 16")?;
        assert_close(cell_num(&cls.rows[0][9], "Accuracy")?, (tp + tn) / 16.0, 1e-12, "Accuracy == (TP+TN)/N")?;
        for k in 1..=9 {
            assert_close(cell_num(&cls.rows[k][0], "sweep cutoff")?, (k as f64) * 0.1, 1e-12, "sweep cutoff")?;
            let sum_k = cell_num(&cls.rows[k][1], "TP")?
                + cell_num(&cls.rows[k][2], "FP")?
                + cell_num(&cls.rows[k][3], "FN")?
                + cell_num(&cls.rows[k][4], "TN")?;
            assert_close(sum_k, 16.0, 1e-12, "sweep row sum == N")?;
        }

        let preds = tab_by_id(rs, "glm-logistic-predictions")?;
        ensure(preds.kind == ResultTabKind::Residuals && preds.rows.len() == 16, "expected 16 row predictions")?;
        for row in &preds.rows {
            let p = cell_num(&row[1], "Predicted prob")?;
            let cls_val = cell_num(&row[2], "Predicted class")?;
            ensure((0.0..=1.0).contains(&p), format!("prob out of [0,1]: {p}"))?;
            assert_close(cls_val, if p >= 0.5 { 1.0 } else { 0.0 }, 1e-12, "Predicted class at 0.5 cutoff")?;
        }
        (auc, aic)
    };

    // Toggle compute_vif off via Cmd+T (`glm-logistic-params` drops from 7 to 6 cols), then restore on.
    edit_params_cmd_t(&mut app, &log_id, json!({ "edit.glm.compute_vif": false }))?;
    ensure(tab_by_id(result_sheet(&app, &log_id)?, "glm-logistic-params")?.column_headers.len() == 6, "disabling VIF must remove VIF column")?;
    edit_params_cmd_t(&mut app, &log_id, json!({ "edit.glm.compute_vif": true }))?;

    // Check Logistic response residuals plot via `diag.residual` AND generate companion ROC Curve (`stats.roc` -> `diag.roc`).
    switch_graph_type(&mut app, &mv_graph_id, "diag.residual")?;
    let log_resid_scene = compile_scene(&app, &mv_graph_id)?;
    ensure(
        log_resid_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count() >= 16,
        "missing logistic residual points",
    )?;

    let roc_tbl = create_table(
        &mut app,
        json!({ "sheet_id": "wf41-roc-tbl", "title": "Logistic Score ROC", "kind": "column", "rows": 8, "cols": 2, "auto_graph": true, "graph_sheet_id": "wf41-roc-graph" }),
    )?;
    select_sheet(&mut app, &roc_tbl)?;
    paste_grid_tsv(&mut app, 0, 0, 0, "0.08\t0.62\n0.14\t0.75\n0.22\t0.84\n0.35\t0.91\n0.41\t0.95\n0.19\t0.54\n0.52\t0.97\n0.28\t0.88")?;
    let roc_res_id = run_analyze_dialog(&mut app, &roc_tbl, json!({ "entry_id": "stats.roc" }))?;
    let roc_auc_standalone = scalar_val(result_sheet(&app, &roc_res_id)?, "Area under ROC curve (AUC)", None)?;
    ensure(roc_auc_standalone > 0.85, format!("expected high ROC AUC, got {roc_auc_standalone}"))?;
    switch_graph_type(&mut app, "wf41-roc-graph", "diag.roc")?;
    let roc_scene = compile_scene(&app, "wf41-roc-graph")?;
    ensure(roc_scene.primitives.len() >= 3, "ROC graph must render curve & diagonal")?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Upstream Cmd+Shift+E exclusion check triggers Logistic GLM reactive DAG recompute.
    exclude_cell_cmd_shift_e(&mut app, &mv_id, 4, 1, 0, "assay_error", "hemolyzed serum")?;
    let rs_after = result_sheet(&app, &log_id)?;
    ensure(tab_by_id(rs_after, "glm-logistic-predictions")?.rows.len() == 15, "expected 15 active rows after exclusion")?;
    let auc_after = scalar_val(rs_after, "ROC AUC", None)?;
    let aic_after = scalar_val(rs_after, "AIC", None)?;
    ensure(
        (auc_after - auc_initial).abs() > 1e-4 && (aic_after - aic_initial).abs() > 0.1,
        format!("exclusion must update ROC AUC ({auc_initial} -> {auc_after}) & AIC ({aic_initial} -> {aic_after})"),
    )?;
    Ok(())
}

// ============================================================================
// WF-42: Poisson Count GLM (Incidence Rate Ratios, Equidispersion, VIF
//        Pruning via Cmd+T, AIC, McFadden R2, Residuals & Diagnostic Plot)
// ============================================================================

#[test]
fn wf42_poisson_count_glm_rate_ratios_dispersion_and_vif() -> TestRes {
    let mut app = new_strict_app()?;

    // ExposureX1 & ExposureX2 are strongly collinear (VIF >= 10) alongside uncorrelated Temp & Batch.
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf42-mv",
            "title": "Foci Count Assay",
            "kind": "multiple_variables",
            "rows": 14,
            "cols": 5,
            "variables": [
                { "title": "Counts", "role": "continuous" },
                { "title": "ExposureX1", "role": "continuous" },
                { "title": "ExposureX2", "role": "continuous" },
                { "title": "Temp", "role": "continuous" },
                { "title": "Batch", "role": "categorical" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid_tsv(
        &mut app,
        0,
        0,
        0,
        "2\t0.5\t1.01\t20.0\tBatchA\n\
         3\t1.0\t2.03\t24.0\tBatchA\n\
         4\t1.5\t2.99\t21.0\tBatchB\n\
         5\t2.0\t4.02\t25.0\tBatchA\n\
         7\t2.5\t4.98\t22.0\tBatchB\n\
         9\t3.0\t6.01\t26.0\tBatchA\n\
         11\t3.5\t7.04\t19.0\tBatchB\n\
         15\t4.0\t7.97\t23.0\tBatchA\n\
         19\t4.5\t9.02\t27.0\tBatchB\n\
         24\t5.0\t9.99\t20.5\tBatchA\n\
         31\t5.5\t11.03\t24.5\tBatchB\n\
         38\t6.0\t12.01\t21.5\tBatchA\n\
         49\t6.5\t12.98\t25.5\tBatchB\n\
         62\t7.0\t14.02\t22.5\tBatchA",
    )?;
    let graph_id = linked_graph_id(&app, &mv_id)?;

    let pois_id = run_analyze_dialog(
        &mut app,
        &mv_id,
        json!({
            "entry_id": "stats.glm",
            "glm_family": "poisson",
            "response_column": 0,
            "selected_data_sets": [1, 2, 3, 4],
            "reference_levels": [[4, "BatchA"]],
            "compute_vif": true
        }),
    )?;

    let aic_full = {
        let rs = result_sheet(&app, &pois_id)?;
        let deviance = scalar_val(rs, "Residual deviance", None)?;
        let dispersion = scalar_val(rs, "Pearson dispersion", None)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let mcfadden = scalar_val(rs, "McFadden R squared", None)?;
        ensure(
            deviance >= 0.0 && dispersion > 0.0 && aic.is_finite() && mcfadden > 0.70,
            format!("unexpected Poisson scalars: dev={deviance}, disp={dispersion}, r2={mcfadden}"),
        )?;

        let params = tab_by_id(rs, "glm-poisson-params")?;
        ensure(
            params.column_headers.len() == 7 && params.column_headers[6] == "VIF",
            format!("expected 7 columns with VIF, got {:?}", params.column_headers),
        )?;
        ensure(params.rows.len() == 5, "expected Intercept + 4 terms")?;
        let beta_x1 = cell_num(&params.rows[1][0], "beta X1")?;
        let irr_x1 = cell_num(&params.rows[1][4], "IRR X1")?;
        assert_close(irr_x1, beta_x1.exp(), 1e-10, "Rate Ratio == exp(beta)")?;
        let vif_x1 = cell_num(&params.rows[1][6], "VIF X1")?;
        let vif_x2 = cell_num(&params.rows[2][6], "VIF X2")?;
        ensure(vif_x1 >= 10.0 && vif_x2 >= 10.0, format!("collinear ExposureX1/X2 must trigger VIF >= 10: {vif_x1}, {vif_x2}"))?;

        let resid = tab_by_id(rs, "glm-poisson-residuals")?;
        ensure(
            resid.column_headers == ["Observed Y", "Predicted count", "Pearson residual", "Deviance residual"],
            "unexpected Poisson residual headers",
        )?;
        ensure(resid.rows.len() == 14, "expected 14 Poisson residual rows")?;
        ensure(
            rs.checklist.iter().any(|c| c.assumption == "Equidispersion assumption (Poisson)" && c.remediation.is_none()),
            "missing QOL-7 Poisson equidispersion item",
        )?;
        aic
    };

    // Switch MV graph to Poisson Residual Plot and verify screenshot PNG.
    switch_graph_type(&mut app, &graph_id, "diag.residual")?;
    let pois_scene = compile_scene(&app, &graph_id)?;
    ensure(
        pois_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count() >= 14,
        "expected >= 14 points on Poisson residual plot",
    )?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Prune collinear ExposureX2 via Cmd+T -> VIF collapses < 5.0 cleanly!
    edit_params_cmd_t(&mut app, &pois_id, json!({ "edit.glm.predictor_columns": [1, 3, 4] }))?;
    let (aic_pruned, disp_pruned) = {
        let rs_pruned = result_sheet(&app, &pois_id)?;
        let params_pruned = tab_by_id(rs_pruned, "glm-poisson-params")?;
        ensure(params_pruned.rows.len() == 4, format!("expected 4 rows after pruning ExposureX2, got {}", params_pruned.rows.len()))?;
        for r in 1..4 {
            let v = cell_num(&params_pruned.rows[r][6], "pruned VIF")?;
            ensure(v < 5.0, format!("pruned predictor VIF must drop < 5.0, got {v}"))?;
        }
        let aic_p = scalar_val(rs_pruned, "AIC", None)?;
        let disp_p = scalar_val(rs_pruned, "Pearson dispersion", None)?;
        ensure((aic_p - aic_full).abs() > 0.1, format!("pruning predictor must update AIC ({aic_full} -> {aic_p})"))?;
        (aic_p, disp_p)
    };

    // Upstream live count cell edit (row 10, col 0 is within visible 64-cell window) triggers Poisson reactive recompute.
    select_sheet(&mut app, &mv_id)?;
    type_widget(&mut app, "grid.cell.10.0.0", "55")?;
    let disp_after = scalar_val(result_sheet(&app, &pois_id)?, "Pearson dispersion", None)?;
    let aic_after = scalar_val(result_sheet(&app, &pois_id)?, "AIC", None)?;
    ensure(
        (aic_after - aic_pruned).abs() > 1.0 && (disp_after - disp_pruned).abs() > 0.05,
        format!("live count edit must recompute Poisson AIC ({aic_pruned} -> {aic_after}) & dispersion ({disp_pruned} -> {disp_after})"),
    )?;
    Ok(())
}

// ============================================================================
// WF-43: XY Serial Dilution (Log10(X), Unconstrained vs Bottom=0/Top=1
//        Constrained 4PL IC50 Profile-Likelihood CIs, Trapezoidal & Fitted AUC)
// ============================================================================

#[test]
fn wf43_xy_serial_dilution_constrained_4pl_ic50_and_auc() -> TestRes {
    let mut app = new_strict_app()?;

    // Raw molar concentrations with an INCOMPLETE lower asymptote (~0.33 at highest [Inhibitor])
    // around true LogIC50 = -6.0 (IC50 = 1e-6 M) on Bottom=0.0, Top=1.0 viability curve.
    let xy_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf43-xy",
            "title": "Serial Dilution Viability",
            "kind": "xy",
            "rows": 8,
            "cols": 1,
            "replicates": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf43-raw-graph",
            "x_title": "Concentration (M)"
        }),
    )?;
    let concs = [1e-9_f64, 3.16227766e-9, 1e-8, 3.16227766e-8, 1e-7, 3.16227766e-7, 1e-6, 2e-6];
    let true_hill = |lx: f64| 1.0 / (1.0 + 10_f64.powf(lx - (-6.0)));
    set_x_values(&mut app, &xy_id, &concs)?;
    select_sheet(&mut app, &xy_id)?;
    let mut tsv_lines = Vec::with_capacity(8);
    for (r, &c) in concs.iter().enumerate() {
        let base = true_hill(c.log10());
        let jitter = if r % 2 == 0 { 0.012 } else { -0.012 };
        tsv_lines.push(format!("{:.6}\t{:.6}", base + jitter, base - jitter));
    }
    paste_grid_tsv(&mut app, 0, 0, 0, &tsv_lines.join("\n"))?;

    // Step 1: Log10(X) transform (`y_transform: null` so viability Y stays in [0, 1]).
    let log_rs_id = run_analyze_dialog(
        &mut app,
        &xy_id,
        json!({
            "entry_id": "transform.math",
            "x_transform": "log10",
            "y_transform": Value::Null
        }),
    )?;

    // Step 2: XY Descriptive Statistics directly on the Log10(X) ResultSheet -> Trapezoidal AUC scalar.
    let desc_rs_id = run_analyze_dialog(&mut app, &log_rs_id, json!({ "entry_id": "stats.descriptive" }))?;
    let desc_trap_auc = scalar_val(result_sheet(&app, &desc_rs_id)?, "Trapezoidal AUC", Some(0))?;
    ensure(desc_trap_auc > 2.0 && desc_trap_auc < 3.5, format!("unexpected XY Trapezoidal AUC: {desc_trap_auc}"))?;

    // Step 3: Unconstrained 4PL nonlinear fit + profile likelihood CI via Cmd+T.
    let free_id = run_analyze_dialog(
        &mut app,
        &log_rs_id,
        json!({
            "entry_id": "fit.nonlinear",
            "equation_id": "log_inhibitor_vs_response_4pl"
        }),
    )?;
    edit_params_cmd_t(&mut app, &free_id, json!({ "edit.fit.profile_likelihood_ci": true }))?;

    // Step 4: Constrained Bottom=0.0, Top=1.0 4PL fit + profile likelihood CI + confidence bands.
    let const_id = run_analyze_dialog(
        &mut app,
        &log_rs_id,
        json!({
            "entry_id": "fit.nonlinear",
            "equation_id": "log_inhibitor_vs_response_4pl",
            "parameter_rules": [
                { "name": "Bottom", "scope": { "Constant": 0.0 } },
                { "name": "Top", "scope": { "Constant": 1.0 } }
            ]
        }),
    )?;
    edit_params_cmd_t(&mut app, &const_id, json!({ "edit.fit.profile_likelihood_ci": true, "edit.fit.confidence_bands": true }))?;

    let (ic50_const_initial, fit_auc_const_initial, trap_auc_initial) = {
        let free_rs = result_sheet(&app, &free_id)?;
        let const_rs = result_sheet(&app, &const_id)?;
        ensure(
            matches!(
                &const_rs.analysis.kind,
                AnalysisKind::NonlinearRegression(nl)
                    if nl.parameter_rules.iter().any(|r| r.name == "Bottom" && r.scope == ParameterScope::Constant(0.0))
                    && nl.parameter_rules.iter().any(|r| r.name == "Top" && r.scope == ParameterScope::Constant(1.0))
                    && nl.profile_likelihood_ci
            ),
            "expected constrained Bottom=0/Top=1 4PL spec with profile CI",
        )?;

        let ic50_const = scalar_val(const_rs, "IC50", Some(0))?;
        let span_const = scalar_val(const_rs, "Span", Some(0))?;
        let trap_auc = scalar_val(const_rs, "AUC (trapezoidal)", Some(0))?;
        let fit_auc_const = scalar_val(const_rs, "AUC (fitted curve)", Some(0))?;
        let fit_auc_free = scalar_val(free_rs, "AUC (fitted curve)", Some(0))?;

        assert_close(span_const, 1.0, 1e-6, "constrained Top-Bottom Span")?;
        ensure((ic50_const - 1e-6).abs() < 2.5e-7, format!("constrained IC50 expected ~1e-6, got {ic50_const}"))?;
        assert_close(trap_auc, desc_trap_auc, 1e-8, "fit.nonlinear AUC (trapezoidal) matches stats.descriptive Trapezoidal AUC")?;
        ensure(fit_auc_const > 1.5 && fit_auc_free > 1.5, format!("positive fitted AUCs expected: const={fit_auc_const}, free={fit_auc_free}"))?;

        let params_const = tab_by_id(const_rs, "nonlinear_parameters")?;
        let logic50_row = params_const.row_labels.iter().position(|l| l == "LogIC50").ok_or("missing LogIC50 row")?;
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(c_lo), upper: CiBound::Finite(c_hi) } = params_const.rows[logic50_row][2] else {
            return Err(format!("expected finite LogIC50 CI bounds on constrained fit: {:?}", params_const.rows[logic50_row]));
        };
        let const_width = c_hi - c_lo;
        ensure(const_width < 0.5, format!("constrained profile CI should be tight: [{c_lo}, {c_hi}]"))?;

        let params_free = tab_by_id(free_rs, "nonlinear_parameters")?;
        let free_row = params_free.row_labels.iter().position(|l| l == "LogIC50").ok_or("missing LogIC50 in free fit")?;
        if let ResultCell::ConfidenceInterval { lower: f_lo, upper: f_hi } = params_free.rows[free_row][2] {
            let free_width = match (f_lo, f_hi) {
                (CiBound::Finite(lo), CiBound::Finite(hi)) => hi - lo,
                _ => f64::INFINITY,
            };
            ensure(const_width <= free_width + 1e-9, format!("constrained CI width {const_width} should not exceed unconstrained {free_width}"))?;
        }

        let auc_tab = tab_by_id(const_rs, "fit_auc")?;
        ensure(
            auc_tab.row_labels == ["Trapezoidal AUC", "Fitted curve AUC", "X min", "X max"],
            format!("unexpected fit_auc rows: {:?}", auc_tab.row_labels),
        )?;
        assert_close(cell_num(&auc_tab.rows[0][0], "tab trap AUC")?, trap_auc, 1e-10, "fit_auc trapezoidal row matches scalar")?;
        assert_close(cell_num(&auc_tab.rows[1][0], "tab fitted AUC")?, fit_auc_const, 1e-10, "fit_auc fitted row matches scalar")?;
        (ic50_const, fit_auc_const, trap_auc)
    };

    // Create graph from Log10(X) ResultSheet and overlay the constrained 4PL fit + CI ribbon.
    invoke_menu_cmd(&mut app, "graph.create", json!({ "data_sheet_id": log_rs_id, "sheet_id": "wf43-fit-graph", "kind": "xy" }))?;
    invoke_menu_path(
        &mut app,
        "Graph/Regression Overlay...",
        json!({ "sheet_id": "wf43-fit-graph", "result_sheet_id": const_id, "all_series": true, "show_confidence_band": true, "shaded_ribbon": true }),
    )?;
    select_sheet(&mut app, "wf43-fit-graph")?;
    let fit_scene = compile_scene(&app, "wf43-fit-graph")?;
    ensure(fit_scene.primitives.len() >= 4, format!("4PL overlay & ribbon must appear in ChartScene: {}", fit_scene.primitives.len()))?;
    assert_screenshot_png(&mut app, "canvas")?;

    // 2-hop reactive DAG cascade: live edit & Cmd+Shift+E exclude on raw XY -> propagates through
    // Log10(X) -> stats.descriptive (`Trapezoidal AUC`) AND constrained `fit.nonlinear` (`IC50`, `AUC`).
    select_sheet(&mut app, &xy_id)?;
    type_widget(&mut app, "grid.cell.6.0.0", "0.75")?;
    let ic50_edited = scalar_val(result_sheet(&app, &const_id)?, "IC50", Some(0))?;
    let trap_edited = scalar_val(result_sheet(&app, &desc_rs_id)?, "Trapezoidal AUC", Some(0))?;
    ensure((ic50_edited - ic50_const_initial).abs() > 1e-9, "2-hop edit must update constrained IC50")?;
    ensure((trap_edited - trap_auc_initial).abs() > 1e-4, "2-hop edit must update stats.descriptive Trapezoidal AUC")?;

    exclude_cell_cmd_shift_e(&mut app, &xy_id, 6, 0, 0, "edge_effect", "evaporated border well")?;
    let fit_auc_excl = scalar_val(result_sheet(&app, &const_id)?, "AUC (fitted curve)", Some(0))?;
    ensure(
        (fit_auc_excl - fit_auc_const_initial).abs() > 1e-4,
        format!("exclusion must update fitted curve AUC ({fit_auc_const_initial} -> {fit_auc_excl})"),
    )?;
    Ok(())
}
