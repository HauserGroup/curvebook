#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R10.13` Canonical Human-Path Workflows (`WF-38`..`WF-43`):
//! - `WF-38`: K-Means & Hierarchical Observation Clustering + Append Cluster Column + Scatter (`stats.clustering`)
//! - `WF-39`: Cox Proportional Hazards (`Efron` vs `Breslow` Ties, Reference Levels, Survival Curves) (`stats.survival_cox`)
//! - `WF-40`: Multiple Linear Regression (Type III ANOVA, VIF, Diagnostics-Only, Residual & Q-Q Plots) (`stats.glm` Linear)
//! - `WF-41`: Multiple Logistic Regression (VIF, Classification Cutoff Sweep, Standalone ROC Curve) (`stats.glm` Logistic + `stats.roc`)
//! - `WF-42`: Poisson Count GLM (Incidence Rate Ratios, Dispersion, Collinear VIF Pruning) (`stats.glm` Poisson)
//! - `WF-43`: XY Serial Dilution (`Log10(X)`, Constrained `Bottom=0`/`Top=1` 4PL IC50, Profile CIs, AUC, Regression Overlay)

mod human_wf_common;

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, ChecklistStatus, CiBound, CoxTieMethod, DataSheet, MathTransformPreset, MultivariatePlotMode, MultivariateSpec, ParameterScope,
    ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec,
    VariableRole,
};
use curvebook_render::RenderTargetSpec;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::compositor::render_workspace_rgba;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome};
use curvebook_ui_egui::layout::WidgetId;
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused, widget_hit_center,
};

type TestRes = Result<(), String>;

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) -> Result<(), String> {
    let diff = (actual - expected).abs();
    ensure(diff <= tol, format!("{label}: expected {expected} +/- {tol}, got {actual} (diff={diff})"))
}

fn click_id(driver: &mut HumanPathDriver, id: &str) -> Result<UiEventOutcome, String> {
    click_widget_center(driver, &wid(id))
}

fn replace_text_in_widget(driver: &mut HumanPathDriver, id: &str, text: &str) -> Result<(), String> {
    let _ = click_id(driver, id)?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, text)?;
    Ok(())
}

fn open_menu_and_click(driver: &mut HumanPathDriver, menu_id: &str, item_id: &str) -> Result<UiEventOutcome, String> {
    let _ = click_id(driver, menu_id)?;
    let popup = wid(&format!("popup.{menu_id}"));
    scroll_until_widget_visible(driver, &popup, &wid(item_id), 120.0, 28)?;
    click_id(driver, item_id)
}

fn scroll_find_any_dir(driver: &mut HumanPathDriver, container_id: &str, target_id: &str, step_y: f64, max_steps: usize) -> Result<(), String> {
    if scroll_until_widget_visible(driver, &wid(container_id), &wid(target_id), step_y, max_steps).is_ok() {
        return Ok(());
    }
    scroll_until_widget_visible(driver, &wid(container_id), &wid(target_id), -step_y, max_steps)
}

fn select_sheet_in_nav(driver: &mut HumanPathDriver, sheet_id: &SheetId) -> Result<(), String> {
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. })
        && driver.app().document().active_sheet_id.as_ref() != Some(sheet_id)
    {
        let _ = press_key_combo(driver, "Escape", KeyModifiers::default())?;
    }
    if driver.app().document().active_sheet_id.as_ref() == Some(sheet_id) {
        return Ok(());
    }
    let nav_id = format!("nav.sheet.{}", sheet_id.as_str());
    scroll_find_any_dir(driver, "navigator.tree", &nav_id, 100.0, 16)?;
    let _ = click_id(driver, &nav_id)?;
    ensure(
        driver.app().document().active_sheet_id.as_ref() == Some(sheet_id),
        format!("failed to select sheet `{}` via Navigator", sheet_id.as_str()),
    )?;
    Ok(())
}

fn import_tsv_table(driver: &mut HumanPathDriver, title: &str, target_widget_id: &str, tsv: &str) -> Result<SheetId, String> {
    let before = driver.app().document().data_sheets.len();
    let _ = open_menu_and_click(driver, "menu.file", "menu.ui.import.reshape.open")?;
    ensure(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ImportReshape), "ImportReshape dialog did not open")?;
    let _ = click_id(driver, "import_paste.source.clipboard")?;
    let _ = click_id(driver, target_widget_id)?;
    let _ = click_id(driver, "import_paste.delim.tab")?;
    replace_text_in_widget(driver, "import_paste.input.title", title)?;
    scroll_until_widget_visible(driver, &wid("import_paste.panel.source"), &wid("import_paste.input.csv"), 100.0, 14)?;
    let _ = click_and_type(driver, &wid("import_paste.input.csv"), tsv)?;
    scroll_until_widget_visible(driver, &wid("import_paste.panel.source"), &wid("import_paste.detect_format"), -100.0, 14)?;
    let _ = click_id(driver, "import_paste.detect_format")?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    let _ = click_id(driver, "modal.ok")?;
    ensure(driver.app().canvas_dialogs.active_dialog == ActiveDialog::None, "ImportReshape modal did not close on OK")?;
    ensure(driver.app().document().data_sheets.len() == before + 1, "ImportReshape did not create new DataSheet")?;
    let new_id = driver.app().document().data_sheets.last().map(|d| d.id.clone()).ok_or_else(|| String::from("missing newly imported DataSheet"))?;
    select_sheet_in_nav(driver, &new_id)?;
    Ok(new_id)
}

fn create_graph_for_active_sheet(driver: &mut HumanPathDriver) -> Result<SheetId, String> {
    let before = driver.app().document().graph_sheets.len();
    let _ = click_id(driver, "nav.create.graphs")?;
    ensure(driver.app().document().graph_sheets.len() == before + 1, "nav.create.graphs did not create a GraphSheet")?;
    driver.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or_else(|| String::from("missing newly created GraphSheet"))
}

fn set_mv_variable_role_badge(driver: &mut HumanPathDriver, mv_id: &SheetId, col_idx: usize, role_slug: &str) -> Result<(), String> {
    select_sheet_in_nav(driver, mv_id)?;
    let _ = click_id(driver, &format!("mv_formula.role_badge.{col_idx}"))?;
    let _ = click_id(driver, &format!("mv_formula.role_opt.{col_idx}.{role_slug}"))?;
    Ok(())
}

fn set_mv_reference_level_human(driver: &mut HumanPathDriver, mv_id: &SheetId, col_idx: usize, ref_level: &str) -> Result<(), String> {
    select_sheet_in_nav(driver, mv_id)?;
    let _ = click_id(driver, "mv_formula.variables_btn")?;
    let _ = click_id(driver, &format!("mv_formula.vars.col.{col_idx}"))?;
    let idx = driver.app().grid.variables_dialog.ordered_levels.iter().position(|lvl| lvl.level == ref_level).ok_or_else(|| {
        let avail: Vec<&str> = driver.app().grid.variables_dialog.ordered_levels.iter().map(|l| l.level.as_str()).collect();
        format!("reference level `{ref_level}` not found in col {col_idx} levels {avail:?}")
    })?;
    scroll_find_any_dir(driver, "mv_formula.vars.levels_list", &format!("mv_formula.vars.level_ref.{idx}"), 60.0, 12)?;
    let _ = click_id(driver, &format!("mv_formula.vars.level_ref.{idx}"))?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    let _ = click_id(driver, "mv_formula.vars.close")?;
    Ok(())
}

fn run_analyze_human(driver: &mut HumanPathDriver, source_id: &SheetId, entry_id: &str) -> Result<SheetId, String> {
    select_sheet_in_nav(driver, source_id)?;
    let before = driver.app().document().result_sheets.len();
    let _ = open_menu_and_click(driver, "menu.analyze", "menu.ui.analyze.open")?;
    ensure(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "AnalyzeSheet modal did not open")?;
    replace_text_in_widget(driver, "analyze.search", entry_id)?;
    scroll_find_any_dir(driver, "analyze.entries_list", &format!("analyze.entry.{entry_id}"), 100.0, 12)?;
    let _ = click_id(driver, &format!("analyze.entry.{entry_id}"))?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    let _ = click_id(driver, "modal.ok")?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        let _ = click_id(driver, "analyze_picker.dup.create_new")?;
        let _ = click_id(driver, "modal.ok")?;
    }
    ensure(driver.app().canvas_dialogs.active_dialog == ActiveDialog::None, format!("Analyze modal stayed open after running `{entry_id}`"))?;
    ensure(driver.app().document().result_sheets.len() == before + 1, format!("expected new ResultSheet after `{entry_id}`"))?;
    driver.app().document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing newly created ResultSheet"))
}

fn open_edit_params_cmd_t(driver: &mut HumanPathDriver, result_id: &SheetId) -> Result<(), String> {
    select_sheet_in_nav(driver, result_id)?;
    let _ = press_cmd_shortcut(driver, "t")?;
    ensure(
        matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
        "Cmd+T did not open EditAnalysisParameters",
    )?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    Ok(())
}

fn click_modal_control(driver: &mut HumanPathDriver, container_id: &str, widget_id: &str) -> Result<UiEventOutcome, String> {
    if driver.find_widget(widget_id).is_none_or(|n| !n.drawn_in_frame) {
        scroll_find_any_dir(driver, container_id, widget_id, 120.0, 14)?;
    }
    click_id(driver, widget_id)
}

fn confirm_modal_ok(driver: &mut HumanPathDriver) -> Result<(), String> {
    let _ = click_id(driver, "modal.ok")?;
    ensure(driver.app().canvas_dialogs.active_dialog == ActiveDialog::None, "modal did not close after modal.ok")?;
    Ok(())
}

fn switch_result_tab_human(driver: &mut HumanPathDriver, result_id: &SheetId, tab_id: &str) -> Result<(), String> {
    select_sheet_in_nav(driver, result_id)?;
    let tab_wid = format!("result.tab.{tab_id}");
    for _ in 0..16 {
        if driver.find_widget(&tab_wid).is_some_and(|n| n.drawn_in_frame && n.state.enablement.enabled) {
            let _ = click_id(driver, &tab_wid)?;
            let rs = result_sheet(driver.app().document(), result_id)?;
            let idx = driver.app().canvas_dialogs.result_view.active_tab_index.min(rs.tabs.len().saturating_sub(1));
            ensure(rs.tabs.get(idx).is_some_and(|t| t.id == tab_id), format!("active ResultSheet tab did not switch to `{tab_id}`"))?;
            return Ok(());
        }
        if driver.find_widget("result.tabs.next").is_some_and(|n| n.drawn_in_frame && n.state.enablement.enabled) {
            let _ = click_id(driver, "result.tabs.next")?;
        } else {
            break;
        }
    }
    Err(format!("result tab `{tab_wid}` was not reachable in ResultViewer"))
}

fn scroll_result_table_to_cell(driver: &mut HumanPathDriver, result_id: &SheetId, tab_id: &str, row: usize, col: usize) -> Result<(), String> {
    switch_result_tab_human(driver, result_id, tab_id)?;
    let target = format!("result.cell.{row}.{col}");
    scroll_until_widget_visible(driver, &wid("result.table"), &wid(&target), 180.0, 48)?;
    let _ = click_id(driver, &target)?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    Ok(())
}

fn switch_graph_type_human(driver: &mut HumanPathDriver, graph_id: &SheetId, option_id: &str, needs_show_all: bool) -> Result<(), String> {
    select_sheet_in_nav(driver, graph_id)?;
    if !matches!(&driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { graph_sheet_id } if graph_sheet_id == graph_id) {
        if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
            let _ = press_key_combo(driver, "Escape", KeyModifiers::default())?;
        }
        let _ = open_menu_and_click(driver, "menu.graph", "menu.ui.changeGraphType.open")?;
    }
    ensure(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }), "ChangeGraphType modal did not open")?;
    if needs_show_all {
        let _ = click_id(driver, "graph_type.show_all")?;
    }
    let card_wid = format!("graph_type.card.{option_id}");
    scroll_find_any_dir(driver, "graph_type.panel", &card_wid, 100.0, 28)?;
    let _ = click_id(driver, &card_wid)?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    confirm_modal_ok(driver)
}

fn verify_graph_renders(driver: &mut HumanPathDriver, graph_id: &SheetId) -> Result<ChartScene, String> {
    select_sheet_in_nav(driver, graph_id)?;
    let gs = driver.app().document().graph_sheet(graph_id).ok_or_else(|| format!("missing GraphSheet `{}`", graph_id.as_str()))?;
    let scene = compile_graph(gs, driver.app().document()).map_err(|e| format!("compile_graph failed: {e}"))?;
    ensure(
        !scene.warnings.iter().any(|w| w.code.starts_with("empty_")),
        format!("unexpected empty warning in scene for `{}`: {:?}", graph_id.as_str(), scene.warnings),
    )?;
    let raster = render_workspace_rgba(driver.app(), RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }).map_err(|e| e.to_string())?;
    ensure(raster.rgba.iter().any(|&b| b != 0), "rendered graph frame buffer must be non-empty")?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    assert_input_to_paint_budget(driver, 250.0)?;
    Ok(scene)
}

fn scroll_grid_cell_visible(driver: &mut HumanPathDriver, sheet_id: &SheetId, widget_id: &str) -> Result<(), String> {
    let container = format!("grid.sheet.{}", sheet_id.as_str());
    for dir in [180.0_f64, -180.0_f64] {
        for _ in 0..=48 {
            if widget_hit_center(driver, &wid(widget_id)).is_ok() {
                return Ok(());
            }
            let _ = driver.wheel_widget(&container, 0.0, dir, KeyModifiers::default())?;
        }
    }
    Err(format!("grid cell `{widget_id}` did not become clickable inside `{container}`"))
}

fn commit_inline_overwrite(driver: &mut HumanPathDriver, target_wid: &str, value: &str) -> Result<(), String> {
    let [lx, ly] = widget_hit_center(driver, &wid(target_wid))?;
    let out = driver.double_click_logical(lx, ly)?;
    ensure(out.handled, format!("double_click_logical did not activate inline editor for `{target_wid}`"))?;
    let clear_len = driver.app().inline_editor.buffer.chars().count();
    for _ in 0..clear_len {
        let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    }
    let _ = driver.type_text(value)?;
    let _ = press_key_combo(driver, "Enter", KeyModifiers::default())?;
    ensure(!driver.app().inline_editor.active, "Enter did not commit inline editor")?;
    Ok(())
}

fn edit_grid_cell_live(driver: &mut HumanPathDriver, sheet_id: &SheetId, row: usize, col: usize, subcol: usize, value: &str) -> Result<(), String> {
    select_sheet_in_nav(driver, sheet_id)?;
    let cell_wid = format!("grid.cell.{row}.{col}.{subcol}");
    scroll_grid_cell_visible(driver, sheet_id, &cell_wid)?;
    commit_inline_overwrite(driver, &cell_wid, value)
}

fn edit_x_cell_live(driver: &mut HumanPathDriver, sheet_id: &SheetId, row: usize, subcol: usize, value: &str) -> Result<(), String> {
    select_sheet_in_nav(driver, sheet_id)?;
    let cell_wid = format!("grid.xcell.{row}.{subcol}");
    scroll_grid_cell_visible(driver, sheet_id, &cell_wid)?;
    commit_inline_overwrite(driver, &cell_wid, value)
}

fn exclude_cell_cmd_shift_e(
    driver: &mut HumanPathDriver,
    sheet_id: &SheetId,
    row: usize,
    col: usize,
    subcol: usize,
    tag_idx: usize,
    note: &str,
) -> Result<(), String> {
    select_sheet_in_nav(driver, sheet_id)?;
    let cell_wid = format!("grid.cell.{row}.{col}.{subcol}");
    scroll_grid_cell_visible(driver, sheet_id, &cell_wid)?;
    let _ = click_id(driver, &cell_wid)?;
    let _ = press_key_combo(driver, "E", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    ensure(
        matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }),
        "Cmd+Shift+E did not open ExcludeWithReason modal",
    )?;
    let _ = click_id(driver, &format!("exclude.tag.{tag_idx}"))?;
    replace_text_in_widget(driver, "exclude.note_input", note)?;
    let _ = click_id(driver, "exclude.apply_note")?;
    let _ = assert_zero_shortcut_compliance(driver)?;
    confirm_modal_ok(driver)
}

fn result_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a ResultSheet, String> {
    doc.result_sheet(id).ok_or_else(|| format!("missing ResultSheet `{}`", id.as_str()))
}

fn data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a DataSheet, String> {
    doc.data_sheet(id).ok_or_else(|| format!("missing DataSheet `{}`", id.as_str()))
}

fn tab_by_id<'a>(rs: &'a ResultSheet, tab_id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == tab_id).ok_or_else(|| format!("missing ResultTab `{tab_id}`"))
}

fn scalar_val(rs: &ResultSheet, key: &str, dataset_index: Option<usize>) -> Result<f64, String> {
    rs.lookup_scalar(key, dataset_index).ok_or_else(|| format!("missing scalar `{key}` (dataset={dataset_index:?}) on `{}`", rs.id.as_str()))
}

fn cell_num(cell: &ResultCell, ctx: &str) -> Result<f64, String> {
    match cell {
        ResultCell::Number { value, .. } => Ok(*value),
        other => Err(format!("{ctx}: expected Number ResultCell, got {other:?}")),
    }
}

fn cell_p_val(cell: &ResultCell, ctx: &str) -> Result<f64, String> {
    match cell {
        ResultCell::PValue { raw, .. } | ResultCell::Number { value: raw, .. } if raw.is_finite() => Ok(*raw),
        other => Err(format!("{ctx}: expected finite PValue/Number, got {other:?}")),
    }
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

fn cycle_dialog_control_until<F>(driver: &mut HumanPathDriver, wid_str: &str, max_clicks: usize, mut check: F) -> Result<(), String>
where
    F: FnMut(&UiApp) -> bool,
{
    for _ in 0..=max_clicks {
        if check(driver.app()) {
            return Ok(());
        }
        let _ = click_id(driver, wid_str)?;
    }
    Err(format!("control `{wid_str}` did not satisfy expected state within {max_clicks} clicks"))
}

fn ensure_cox_predictor(driver: &mut HumanPathDriver, col: usize, include: bool) -> Result<(), String> {
    let currently_included = matches!(
        driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }))
            if predictor_columns.contains(&col)
    );
    if currently_included != include {
        let _ = click_modal_control(driver, "edit.content_panel", &format!("edit.cox.pred.{col}"))?;
    }
    Ok(())
}

fn ensure_glm_predictor(driver: &mut HumanPathDriver, col: usize, include: bool) -> Result<(), String> {
    let currently_included = matches!(
        driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::GeneralizedLinearModel(glm)) if glm.predictor_columns.contains(&col)
    );
    if currently_included != include {
        let _ = click_modal_control(driver, "edit.content_panel", &format!("edit.glm.pred.{col}"))?;
    }
    Ok(())
}

// ============================================================================
// WF-38: K-Means & Hierarchical Observation Clustering + Cluster Scatter
// ============================================================================

#[test]
fn wf38_human_kmeans_clustering_silhouette_elbow_and_scatter() -> TestRes {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 120 samples across 3 tight, well-separated clusters (rows 0..40, 40..80, 80..120).
    let mut tsv_lines = vec![String::from("CellId\tMarkerA\tMarkerB\tMarkerC")];
    for r in 0..120usize {
        let step = ((r % 40) as f64 - 20.0) * 0.01;
        let (ma, mb, mc) = match r / 40 {
            0 => (1.0 + step, 1.1 - step, 0.9 + step * 0.5),
            1 => (10.0 + step, 10.2 - step, 1.0 + step * 0.5),
            _ => (2.0 + step, 11.0 - step, 12.0 + step * 0.5),
        };
        tsv_lines.push(format!("C{:03}\t{ma:.4}\t{mb:.4}\t{mc:.4}", r + 1));
    }
    let mv_id = import_tsv_table(&mut driver, "Single-Cell Markers", "import_paste.target.mv", &tsv_lines.join("\n"))?;
    set_mv_variable_role_badge(&mut driver, &mv_id, 0, "label")?;
    let graph_id = create_graph_for_active_sheet(&mut driver)?;

    // Run Clustering via Analyze picker, then configure Auto-k (`k_means_k: None`) + sweep (`k_sweep_max: 6`).
    let res_id = run_analyze_human(&mut driver, &mv_id, "stats.clustering")?;
    open_edit_params_cmd_t(&mut driver, &res_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.dist.euclidean")?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.link.ward")?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.k_means_auto")?;
    for _ in 0..8 {
        if matches!(
            driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max: Some(6), .. }))
        ) {
            break;
        }
        click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.k_sweep_max.inc")?;
    }
    confirm_modal_ok(&mut driver)?;

    let wcss_k3 = {
        let rs = result_sheet(driver.app().document(), &res_id)?;
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
            ensure(w <= prev_wcss + 1e-9, format!("WCSS must monotonically decrease: prev={prev_wcss}, cur={w}"))?;
            ensure((-1.0..=1.0).contains(&s), format!("invalid silhouette in sweep: {s}"))?;
            prev_wcss = w;
        }
        ensure(tab_by_id(rs, "kmeans-clusters")?.rows.len() == 3, "expected 3 cluster summary rows")?;
        ensure(tab_by_id(rs, "kmeans-assignments")?.rows.len() == 120, "expected 120 cluster assignment rows")?;
        wcss3
    };

    // Scroll large-N assignments table beyond row 110 via wheel.
    scroll_result_table_to_cell(&mut driver, &res_id, "kmeans-assignments", 115, 0)?;

    // Cmd+T explicit override k=4 reduces WCSS, then decrement back to k=3.
    open_edit_params_cmd_t(&mut driver, &res_id)?;
    for _ in 0..4 {
        if matches!(
            driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(4), .. }))
        ) {
            break;
        }
        click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.k_means_k.inc")?;
    }
    confirm_modal_ok(&mut driver)?;
    let wcss_k4 = scalar_val(result_sheet(driver.app().document(), &res_id)?, "Total WCSS", None)?;
    assert_close(scalar_val(result_sheet(driver.app().document(), &res_id)?, "Clusters (k)", None)?, 4.0, 1e-12, "manual k=4")?;
    ensure(wcss_k4 < wcss_k3, format!("k=4 WCSS ({wcss_k4}) must be < k=3 WCSS ({wcss_k3})"))?;

    open_edit_params_cmd_t(&mut driver, &res_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cluster.k_means_k.dec")?;
    confirm_modal_ok(&mut driver)?;
    assert_close(scalar_val(result_sheet(driver.app().document(), &res_id)?, "Clusters (k)", None)?, 3.0, 1e-12, "restored k=3")?;

    // Append categorical "Cluster" column onto source MV table via Analyze menu.
    select_sheet_in_nav(&mut driver, &res_id)?;
    let _ = open_menu_and_click(&mut driver, "menu.analyze", "menu.ui.result.appendClusterColumn")?;
    {
        let ds = data_sheet(driver.app().document(), &mv_id)?;
        let TableArchetype::MultipleVariables(ref mv) = ds.archetype else {
            return Err(String::from("expected MV archetype"));
        };
        ensure(
            mv.variables.len() == 5 && mv.variables[4].title == "Cluster" && mv.variables[4].role == VariableRole::Categorical,
            "missing appended Cluster categorical col",
        )?;
    }

    // Configure MV cluster scatter with categorical colors, convex hulls & confidence ellipses via BubblePlotInspector.
    select_sheet_in_nav(&mut driver, &graph_id)?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;
    }
    let _ = open_menu_and_click(&mut driver, "menu.graph", "menu.ui.graph.bubblePlot.open")?;
    ensure(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::BubblePlotInspector { .. }), "BubblePlotInspector did not open")?;
    cycle_dialog_control_until(&mut driver, "bubble.plot_mode", 4, |app| {
        app.canvas_dialogs.bubble_plot.plot_mode == MultivariatePlotMode::BubbleScatter
    })?;
    cycle_dialog_control_until(&mut driver, "bubble.x_column", 8, |app| app.canvas_dialogs.bubble_plot.x_column == 1)?;
    cycle_dialog_control_until(&mut driver, "bubble.y_column", 8, |app| app.canvas_dialogs.bubble_plot.y_column == 2)?;
    cycle_dialog_control_until(&mut driver, "bubble.color_mode", 6, |app| {
        matches!(app.canvas_dialogs.bubble_plot.color_channel, curvebook_doc::ColorChannelMapping::Categorical { .. })
    })?;
    cycle_dialog_control_until(&mut driver, "bubble.color_column", 8, |app| {
        matches!(app.canvas_dialogs.bubble_plot.color_channel, curvebook_doc::ColorChannelMapping::Categorical { column: 4, .. })
    })?;
    if !driver.app().canvas_dialogs.bubble_plot.show_convex_hulls {
        let _ = click_id(&mut driver, "bubble.show_convex_hulls")?;
    }
    if !driver.app().canvas_dialogs.bubble_plot.show_confidence_ellipses {
        let _ = click_id(&mut driver, "bubble.show_confidence_ellipses")?;
    }
    confirm_modal_ok(&mut driver)?;
    let scene = verify_graph_renders(&mut driver, &graph_id)?;
    ensure(scene.primitives.len() >= 120, format!("expected scatter + hulls/ellipses primitives, got {}", scene.primitives.len()))?;

    // Upstream Cmd+Shift+E exclude sample row 0 -> shrinking assignments & updating WCSS; re-append updates in-place.
    exclude_cell_cmd_shift_e(&mut driver, &mv_id, 0, 1, 0, 0, "low RNA yield")?;
    let rs_excl = result_sheet(driver.app().document(), &res_id)?;
    let wcss_after = scalar_val(rs_excl, "Total WCSS", None)?;
    ensure((wcss_after - wcss_k3).abs() > 1e-6, format!("exclusion must update WCSS ({wcss_k3} -> {wcss_after})"))?;
    ensure(tab_by_id(rs_excl, "kmeans-assignments")?.rows.len() == 119, "expected 119 active rows after excluding row 0")?;

    select_sheet_in_nav(&mut driver, &res_id)?;
    let _ = open_menu_and_click(&mut driver, "menu.analyze", "menu.ui.result.appendClusterColumn")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-39: Cox Proportional Hazards (Efron vs Breslow Ties, Ref Levels, Curves)
// ============================================================================

#[test]
fn wf39_human_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 126 patients (9 blocks of the canonical 14-patient trial with tied event times t=4.0, 7.0, 12.0).
    let base_rows: [(f64, u8, &str, f64); 14] = [
        (2.0, 1, "Placebo", 68.0),
        (4.0, 1, "Placebo", 66.0),
        (4.0, 1, "Placebo", 64.0),
        (6.0, 1, "Placebo", 62.0),
        (8.0, 0, "Placebo", 60.0),
        (4.0, 1, "LowDose", 65.0),
        (7.0, 1, "LowDose", 63.0),
        (7.0, 1, "LowDose", 59.0),
        (11.0, 0, "LowDose", 56.0),
        (12.0, 1, "LowDose", 61.0),
        (9.0, 0, "HighDose", 58.0),
        (12.0, 1, "HighDose", 64.0),
        (12.0, 0, "HighDose", 55.0),
        (16.0, 1, "HighDose", 62.0),
    ];
    let mut tsv_lines = vec![String::from("Time\tStatus\tArm\tAge")];
    for rep in 0..9usize {
        for (time, status, arm, age) in base_rows {
            let age_adj = age + (rep as f64) * 0.25;
            tsv_lines.push(format!("{time:.1}\t{status}\t{arm}\t{age_adj:.2}"));
        }
    }
    let mv_id = import_tsv_table(&mut driver, "Oncology Survival Trial", "import_paste.target.mv", &tsv_lines.join("\n"))?;
    // Set canonical reference level `Placebo` via MV Variables drawer.
    set_mv_reference_level_human(&mut driver, &mv_id, 2, "Placebo")?;
    let graph_id = create_graph_for_active_sheet(&mut driver)?;

    let cox_id = run_analyze_human(&mut driver, &mv_id, "stats.survival_cox")?;
    // Ensure Age (`col 3`) is included alongside Arm (`col 2`) and verify Efron ties.
    open_edit_params_cmd_t(&mut driver, &cox_id)?;
    ensure_cox_predictor(&mut driver, 2, true)?;
    ensure_cox_predictor(&mut driver, 3, true)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cox.tie.efron")?;
    confirm_modal_ok(&mut driver)?;

    let (efron_aic, efron_beta_low, efron_hr_high) = {
        let rs = result_sheet(driver.app().document(), &cox_id)?;
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
        ensure(c_idx > 0.60 && c_idx <= 1.0, format!("unexpected C-index {c_idx}"))?;
        ensure(aic.is_finite() && lr_p.is_finite() && (0.0..=1.0).contains(&global_schoen), "invalid Cox global scalars")?;

        let cov_tab = tab_by_id(rs, "cox-covariance")?;
        ensure(cov_tab.kind == ResultTabKind::CovarianceMatrix && cov_tab.rows.len() == 3, "expected 3x3 cox-covariance")?;
        for i in 0..3 {
            assert_close(cell_num(&cov_tab.rows[i][i], "cov diag")?, 1.0, 1e-10, "correlation diag == 1")?;
            for j in 0..3 {
                assert_close(cell_num(&cov_tab.rows[i][j], "cij")?, cell_num(&cov_tab.rows[j][i], "cji")?, 1e-10, "cov symmetric")?;
            }
        }
        tab_by_id(rs, "cox-baseline-survival")?;
        tab_by_id(rs, "cox-log-minus-log")?;
        ensure(rs.fitted_curves.iter().any(|c| c.equation_id == "cox_baseline"), "missing cox_baseline curve")?;
        ensure(rs.fitted_curves.iter().filter(|c| c.equation_id == "cox_covariate").count() == 3, "expected 3 cox_covariate curves")?;
        (aic, beta_low, hr_high)
    };

    // Inspect multiple Cox tabs and scroll baseline survival table.
    switch_result_tab_human(&mut driver, &cox_id, "cox-covariance")?;
    switch_result_tab_human(&mut driver, &cox_id, "cox-log-minus-log")?;
    switch_result_tab_human(&mut driver, &cox_id, "cox-baseline-survival")?;

    // Switch to Breslow ties via Cmd+T -> tied times produce different AIC.
    open_edit_params_cmd_t(&mut driver, &cox_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cox.tie.breslow")?;
    confirm_modal_ok(&mut driver)?;
    let breslow_aic = scalar_val(result_sheet(driver.app().document(), &cox_id)?, "AIC", None)?;
    ensure((breslow_aic - efron_aic).abs() > 1e-6, format!("tied events must separate Efron ({efron_aic}) vs Breslow ({breslow_aic})"))?;

    // Restore Efron ties and flip reference level Placebo -> LowDose via MV Variables drawer.
    open_edit_params_cmd_t(&mut driver, &cox_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.cox.tie.efron")?;
    confirm_modal_ok(&mut driver)?;

    set_mv_reference_level_human(&mut driver, &mv_id, 2, "LowDose")?;
    {
        let rs_swapped = result_sheet(driver.app().document(), &cox_id)?;
        let ph_swapped = tab_by_id(rs_swapped, "cox-ph")?;
        let placebo_idx = ph_swapped
            .row_labels
            .iter()
            .position(|l| l == "Arm[Placebo vs LowDose]")
            .ok_or_else(|| format!("expected flipped contrast label, got {:?}", ph_swapped.row_labels))?;
        let beta_placebo_vs_low = cell_num(&ph_swapped.rows[placebo_idx][0], "Placebo vs LowDose beta")?;
        assert_close(beta_placebo_vs_low, -efron_beta_low, 1e-5, "reference level flip negates beta")?;
        assert_close(scalar_val(rs_swapped, "AIC", None)?, efron_aic, 1e-4, "AIC invariant to reference level")?;
    }
    set_mv_reference_level_human(&mut driver, &mv_id, 2, "Placebo")?;

    // Switch MV graph to Survival (`survival.km` via Show All) -> renders Cox Baseline + Covariate survival curves.
    switch_graph_type_human(&mut driver, &graph_id, "survival.km", true)?;
    let surv_scene = verify_graph_renders(&mut driver, &graph_id)?;
    let texts = scene_texts(&surv_scene);
    ensure(texts.iter().any(|t| t == "Baseline"), format!("missing Cox Baseline legend label: {texts:?}"))?;
    ensure(texts.iter().any(|t| t.contains("Arm[HighDose vs Placebo]")), format!("missing Cox covariate label: {texts:?}"))?;

    // Scroll deep into 126-row MV survival table and exclude row 125 via Cmd+Shift+E -> reactive Cox recompute.
    exclude_cell_cmd_shift_e(&mut driver, &mv_id, 125, 0, 0, 4, "withdrew consent")?;
    let rs_after = result_sheet(driver.app().document(), &cox_id)?;
    let hr_after = cell_num(&tab_by_id(rs_after, "cox-ph")?.rows[1][4], "HR after excl")?;
    let aic_after = scalar_val(rs_after, "AIC", None)?;
    ensure((hr_after - efron_hr_high).abs() > 1e-6 && (aic_after - efron_aic).abs() > 1e-3, "exclusion must trigger Cox DAG recompute")?;
    Ok(())
}

// ============================================================================
// WF-40: Multiple Linear Regression (Type III ANOVA, VIF, Diagnostics-Only)
// ============================================================================

#[test]
fn wf40_human_multiple_linear_regression_type_iii_and_vif() -> TestRes {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 126 fermentation runs (9 replicates of the 14-run linear model with correlated Temp & Nutrients).
    let base_rows: [(f64, f64, f64, &str); 14] = [
        (10.1, 1.0, 2.0, "Control"),
        (11.3, 2.0, 1.5, "Control"),
        (16.8, 3.0, 4.0, "Control"),
        (18.4, 4.0, 3.5, "Control"),
        (23.9, 5.0, 6.0, "Control"),
        (24.6, 6.0, 5.0, "Control"),
        (31.7, 7.0, 8.5, "Control"),
        (17.3, 1.5, 2.2, "Rich"),
        (21.5, 2.5, 3.6, "Rich"),
        (24.9, 3.5, 4.5, "Rich"),
        (29.8, 4.5, 6.4, "Rich"),
        (32.6, 5.5, 7.1, "Rich"),
        (38.4, 6.5, 9.5, "Rich"),
        (40.2, 7.5, 9.2, "Rich"),
    ];
    let mut tsv_lines = vec![String::from("Biomass\tTemp\tNutrients\tMedia")];
    for rep in 0..9usize {
        let jitter = (rep as f64 - 4.0) * 0.02;
        for (biomass, temp, nutr, media) in base_rows {
            tsv_lines.push(format!("{:.3}\t{:.3}\t{:.3}\t{media}", biomass + jitter, temp + jitter * 0.5, nutr - jitter * 0.3));
        }
    }
    let mv_id = import_tsv_table(&mut driver, "Fermentation Yield", "import_paste.target.mv", &tsv_lines.join("\n"))?;
    set_mv_reference_level_human(&mut driver, &mv_id, 3, "Control")?;
    let resid_graph_id = create_graph_for_active_sheet(&mut driver)?;

    let glm_id = run_analyze_human(&mut driver, &mv_id, "stats.glm")?;
    open_edit_params_cmd_t(&mut driver, &glm_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.family.linear")?;
    ensure_glm_predictor(&mut driver, 1, true)?;
    ensure_glm_predictor(&mut driver, 2, true)?;
    ensure_glm_predictor(&mut driver, 3, true)?;
    confirm_modal_ok(&mut driver)?;

    let r2_initial = {
        let rs = result_sheet(driver.app().document(), &glm_id)?;
        let r2 = scalar_val(rs, "R squared", None)?;
        let adj_r2 = scalar_val(rs, "Adjusted R squared", None)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let dw = scalar_val(rs, "Durbin-Watson", None)?;
        ensure(r2 > 0.97 && adj_r2 > 0.95 && aic.is_finite() && dw > 0.5, format!("unexpected Linear GLM scalars: R2={r2}, adj={adj_r2}"))?;

        let params = tab_by_id(rs, "glm-linear-params")?;
        let vif_col = params.column_headers.iter().position(|h| h == "VIF").ok_or("missing VIF column")?;
        ensure(params.rows[0][vif_col] == ResultCell::Empty, "Intercept VIF must be Empty")?;
        let vif1 = cell_num(&params.rows[1][vif_col], "Temp VIF")?;
        let vif2 = cell_num(&params.rows[2][vif_col], "Nutrients VIF")?;
        ensure(vif1 > 2.0 && vif2 > 2.0, format!("correlated Temp/Nutrients must yield VIF > 2: {vif1}, {vif2}"))?;

        ensure(tab_by_id(rs, "glm-linear-anova")?.rows.len() == 3, "glm-linear-anova must have 3 rows")?;
        let t3 = tab_by_id(rs, "glm-linear-type-iii")?;
        ensure(t3.kind == ResultTabKind::AnovaTable && t3.column_headers == ["SS", "df", "MS", "F", "P value"], "unexpected Type III headers")?;
        ensure(t3.row_labels == ["Temp", "Nutrients", "Media[Rich vs Control]"], format!("unexpected Type III rows: {:?}", t3.row_labels))?;
        for r in 0..3 {
            ensure(cell_num(&t3.rows[r][3], "Type III F")? > 5.0 && cell_p_val(&t3.rows[r][4], "Type III P")? < 0.05, "Type III F/P")?;
        }

        let cov = tab_by_id(rs, "glm-linear-covariance")?;
        ensure(cov.kind == ResultTabKind::CovarianceMatrix && cov.rows.len() == 4, "expected 4x4 covariance")?;
        for j in 0..4 {
            let se = cell_num(&params.rows[j][1], "SE")?;
            assert_close(cell_num(&cov.rows[j][j], "cov jj")?.sqrt(), se, 1e-8, "sqrt(Cov diag) == SE")?;
        }

        let resid = tab_by_id(rs, "glm-linear-residuals")?;
        ensure(resid.kind == ResultTabKind::Residuals && resid.rows.len() == 126, "expected 126 residual rows")?;
        let mut lev_sum = 0.0;
        for row in &resid.rows {
            let obs = cell_num(&row[0], "Obs Y")?;
            let pred = cell_num(&row[1], "Pred Y")?;
            let r_val = cell_num(&row[2], "Residual")?;
            let lev = cell_num(&row[3], "Leverage")?;
            let cook = cell_num(&row[4], "Cook's D")?;
            assert_close(r_val, obs - pred, 1e-10, "Residual == Obs - Pred")?;
            ensure(cook >= 0.0, "Cook's D non-negative")?;
            lev_sum += lev;
        }
        assert_close(lev_sum, 4.0, 1e-8, "sum of leverages == p (4)")?;
        ensure(rs.checklist.iter().all(|c| c.remediation.is_none()), "QOL-7 forbids auto remediation on MV GLM")?;
        r2
    };

    // Multi-tab inspection + large-N scroll to row 120 + open QOL-7 Interpret drawer.
    switch_result_tab_human(&mut driver, &glm_id, "glm-linear-anova")?;
    switch_result_tab_human(&mut driver, &glm_id, "glm-linear-type-iii")?;
    switch_result_tab_human(&mut driver, &glm_id, "glm-linear-covariance")?;
    scroll_result_table_to_cell(&mut driver, &glm_id, "glm-linear-residuals", 120, 2)?;
    let _ = click_id(&mut driver, "result.interpret_toggle_btn")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = click_id(&mut driver, "result.interpret_toggle_btn")?;

    // Switch graph 1 to Residual Plot (`diag.residual`), then create graph 2 for Normal Q-Q Plot (`diag.normal_qq`).
    switch_graph_type_human(&mut driver, &resid_graph_id, "diag.residual", false)?;
    let resid_scene = verify_graph_renders(&mut driver, &resid_graph_id)?;
    ensure(
        resid_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count() >= 120,
        "expected >= 120 points on ResidualPlot",
    )?;

    select_sheet_in_nav(&mut driver, &mv_id)?;
    let qq_graph_id = create_graph_for_active_sheet(&mut driver)?;
    switch_graph_type_human(&mut driver, &qq_graph_id, "diag.normal_qq", false)?;
    let qq_scene = verify_graph_renders(&mut driver, &qq_graph_id)?;
    ensure(
        qq_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count() >= 120,
        "expected >= 120 points on NormalQqPlot",
    )?;

    // Live edit outlier Biomass on row 125 -> lowers R2 & triggers QOL-7 Residual normality Warn (remediation: None); exclude restores R2!
    edit_grid_cell_live(&mut driver, &mv_id, 125, 0, 0, "450.0")?;
    {
        let rs_skew = result_sheet(driver.app().document(), &glm_id)?;
        ensure(scalar_val(rs_skew, "R squared", None)? < r2_initial - 0.1, "outlier edit must lower R2")?;
        let norm = rs_skew.checklist.iter().find(|c| c.assumption == "Residual normality").ok_or("missing Residual normality")?;
        ensure(norm.status == ChecklistStatus::Warn && norm.remediation.is_none(), format!("expected QOL-7 Warn with no remediation: {norm:?}"))?;
    }

    exclude_cell_cmd_shift_e(&mut driver, &mv_id, 125, 0, 0, 0, "contaminated bioreactor")?;
    let rs_recovered = result_sheet(driver.app().document(), &glm_id)?;
    ensure(scalar_val(rs_recovered, "R squared", None)? > 0.97, "excluding outlier must recover R2 > 0.97")?;
    ensure(tab_by_id(rs_recovered, "glm-linear-residuals")?.rows.len() == 125, "expected 125 residual rows after exclusion")?;
    Ok(())
}

// ============================================================================
// WF-41: Multiple Logistic Regression (VIF, Classification Sweep, ROC Graph)
// ============================================================================

#[test]
fn wf41_human_multiple_logistic_classification_and_roc() -> TestRes {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 128 biomarker observations (8 blocks of 16 observations; non-separable overlap + correlated Dose/Biomarker).
    let base_rows: [(u8, f64, f64, &str); 16] = [
        (0, 1.0, 2.0, "WT"),
        (0, 1.5, 2.4, "WT"),
        (0, 2.0, 2.8, "Mut"),
        (0, 2.5, 3.1, "WT"),
        (1, 2.1, 2.7, "WT"),
        (0, 3.2, 3.6, "WT"),
        (0, 3.6, 4.0, "WT"),
        (1, 4.0, 4.5, "Mut"),
        (0, 4.9, 5.0, "WT"),
        (1, 4.8, 5.2, "Mut"),
        (1, 5.2, 5.6, "WT"),
        (1, 5.6, 6.0, "Mut"),
        (1, 6.0, 6.5, "Mut"),
        (1, 6.5, 6.9, "WT"),
        (1, 7.0, 7.3, "Mut"),
        (1, 7.5, 7.8, "Mut"),
    ];
    let mut tsv_lines = vec![String::from("Outcome\tDose\tBiomarker\tGenotype")];
    for rep in 0..8usize {
        let jitter = (rep as f64 - 3.5) * 0.03;
        for (outc, dose, bio, geno) in base_rows {
            tsv_lines.push(format!("{outc}\t{:.3}\t{:.3}\t{geno}", dose + jitter, bio - jitter * 0.5));
        }
    }
    let mv_id = import_tsv_table(&mut driver, "Biomarker Classifier", "import_paste.target.mv", &tsv_lines.join("\n"))?;
    // Alphabetically "Mut" < "WT", so set "WT" explicitly as reference level via Variables drawer.
    set_mv_reference_level_human(&mut driver, &mv_id, 3, "WT")?;
    let mv_graph_id = create_graph_for_active_sheet(&mut driver)?;

    let log_id = run_analyze_human(&mut driver, &mv_id, "stats.glm")?;
    open_edit_params_cmd_t(&mut driver, &log_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.family.logistic")?;
    ensure_glm_predictor(&mut driver, 1, true)?;
    ensure_glm_predictor(&mut driver, 2, true)?;
    ensure_glm_predictor(&mut driver, 3, true)?;
    confirm_modal_ok(&mut driver)?;

    let (auc_initial, aic_initial) = {
        let rs = result_sheet(driver.app().document(), &log_id)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let hl_chi2 = scalar_val(rs, "Hosmer-Lemeshow chi2", None)?;
        let hl_p = scalar_val(rs, "Hosmer-Lemeshow P", None)?;
        let auc = scalar_val(rs, "ROC AUC", None)?;
        ensure(aic.is_finite() && hl_chi2 >= 0.0 && (0.0..=1.0).contains(&hl_p), "invalid Logistic scalars")?;
        ensure(auc > 0.80 && auc < 1.0, format!("expected non-degenerate Logistic ROC AUC in (0.80, 1.0), got {auc}"))?;

        let params = tab_by_id(rs, "glm-logistic-params")?;
        ensure(params.column_headers.len() == 7 && params.column_headers[6] == "VIF", "expected VIF column in params")?;
        ensure(params.row_labels.iter().any(|l| l == "Genotype[Mut vs WT]"), format!("missing WT ref contrast: {:?}", params.row_labels))?;
        ensure(params.rows[0][6] == ResultCell::Empty && cell_num(&params.rows[1][6], "Dose VIF")? > 1.0, "expected Intercept Empty & VIF > 1")?;

        let cls = tab_by_id(rs, "glm-logistic-classification")?;
        ensure(cls.rows.len() == 10, format!("expected 10 classification rows, got {}", cls.rows.len()))?;
        assert_close(cell_num(&cls.rows[0][0], "primary cutoff")?, 0.5, 1e-12, "cutoff 0.5 row 0")?;
        let sum0: f64 = (1..=4).try_fold(0.0_f64, |acc, c| Ok::<f64, String>(acc + cell_num(&cls.rows[0][c], "2x2")?))?;
        assert_close(sum0, 128.0, 1e-12, "2x2 sum == 128")?;

        let preds = tab_by_id(rs, "glm-logistic-predictions")?;
        ensure(preds.kind == ResultTabKind::Residuals && preds.rows.len() == 128, "expected 128 row predictions")?;
        (auc, aic)
    };

    // Inspect classification table and scroll large-N predictions beyond row 120.
    switch_result_tab_human(&mut driver, &log_id, "glm-logistic-classification")?;
    scroll_result_table_to_cell(&mut driver, &log_id, "glm-logistic-predictions", 122, 1)?;

    // Toggle compute_vif off via Cmd+T -> drops VIF column, then restore on.
    open_edit_params_cmd_t(&mut driver, &log_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.compute_vif")?;
    confirm_modal_ok(&mut driver)?;
    ensure(
        tab_by_id(result_sheet(driver.app().document(), &log_id)?, "glm-logistic-params")?.column_headers.len() == 6,
        "disabling VIF must remove VIF column",
    )?;
    open_edit_params_cmd_t(&mut driver, &log_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.compute_vif")?;
    confirm_modal_ok(&mut driver)?;

    // Logistic residuals plot (`diag.residual`) + standalone ROC (`stats.roc` -> `diag.roc`).
    switch_graph_type_human(&mut driver, &mv_graph_id, "diag.residual", false)?;
    let _ = verify_graph_renders(&mut driver, &mv_graph_id)?;

    let roc_tsv = "Control\tScore\n0.08\t0.62\n0.14\t0.75\n0.22\t0.84\n0.35\t0.91\n0.41\t0.95\n0.19\t0.54\n0.52\t0.97\n0.28\t0.88";
    let roc_tbl = import_tsv_table(&mut driver, "Logistic Score ROC", "import_paste.target.column", roc_tsv)?;
    let roc_graph_id = create_graph_for_active_sheet(&mut driver)?;
    let roc_res_id = run_analyze_human(&mut driver, &roc_tbl, "stats.roc")?;
    let roc_auc = scalar_val(result_sheet(driver.app().document(), &roc_res_id)?, "Area under ROC curve (AUC)", None)?;
    ensure(roc_auc > 0.85, format!("expected high ROC AUC, got {roc_auc}"))?;
    switch_graph_type_human(&mut driver, &roc_graph_id, "diag.roc", false)?;
    let roc_scene = verify_graph_renders(&mut driver, &roc_graph_id)?;
    ensure(roc_scene.primitives.len() >= 3, "ROC graph must render curve & diagonal")?;

    // Exclude cell on MV table via Cmd+Shift+E -> reactive Logistic GLM recompute.
    exclude_cell_cmd_shift_e(&mut driver, &mv_id, 4, 1, 0, 1, "hemolyzed serum")?;
    let rs_after = result_sheet(driver.app().document(), &log_id)?;
    ensure(tab_by_id(rs_after, "glm-logistic-predictions")?.rows.len() == 127, "expected 127 active rows after exclusion")?;
    let auc_after = scalar_val(rs_after, "ROC AUC", None)?;
    let aic_after = scalar_val(rs_after, "AIC", None)?;
    ensure(
        (auc_after - auc_initial).abs() > 1e-5 && (aic_after - aic_initial).abs() > 0.05,
        format!("exclusion must update ROC AUC ({auc_initial} -> {auc_after}) & AIC ({aic_initial} -> {aic_after})"),
    )?;
    Ok(())
}

// ============================================================================
// WF-42: Poisson Count GLM (IRR, Equidispersion, Collinear VIF Pruning)
// ============================================================================

#[test]
fn wf42_human_poisson_count_glm_dispersion_and_vif() -> TestRes {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 126 assay runs (9 blocks of 14 runs; strongly collinear ExposureX1 & ExposureX2 -> VIF >= 10).
    let base_rows: [(i64, f64, f64, f64, &str); 14] = [
        (2, 0.5, 1.01, 20.0, "BatchA"),
        (3, 1.0, 2.03, 24.0, "BatchA"),
        (4, 1.5, 2.99, 21.0, "BatchB"),
        (5, 2.0, 4.02, 25.0, "BatchA"),
        (7, 2.5, 4.98, 22.0, "BatchB"),
        (9, 3.0, 6.01, 26.0, "BatchA"),
        (11, 3.5, 7.04, 19.0, "BatchB"),
        (15, 4.0, 7.97, 23.0, "BatchA"),
        (19, 4.5, 9.02, 27.0, "BatchB"),
        (24, 5.0, 9.99, 20.5, "BatchA"),
        (31, 5.5, 11.03, 24.5, "BatchB"),
        (38, 6.0, 12.01, 21.5, "BatchA"),
        (49, 6.5, 12.98, 25.5, "BatchB"),
        (62, 7.0, 14.02, 22.5, "BatchA"),
    ];
    let mut tsv_lines = vec![String::from("Counts\tExposureX1\tExposureX2\tTemp\tBatch")];
    for rep in 0..9usize {
        let jitter = (rep as f64 - 4.0) * 0.005;
        for (cnt, x1, x2, temp, batch) in base_rows {
            tsv_lines.push(format!("{cnt}\t{:.3}\t{:.3}\t{:.2}\t{batch}", x1 + jitter, x2 + jitter * 2.0, temp + (rep % 3) as f64 * 0.2));
        }
    }
    let mv_id = import_tsv_table(&mut driver, "Foci Count Assay", "import_paste.target.mv", &tsv_lines.join("\n"))?;
    set_mv_reference_level_human(&mut driver, &mv_id, 4, "BatchA")?;
    let graph_id = create_graph_for_active_sheet(&mut driver)?;

    let pois_id = run_analyze_human(&mut driver, &mv_id, "stats.glm")?;
    open_edit_params_cmd_t(&mut driver, &pois_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.family.poisson")?;
    ensure_glm_predictor(&mut driver, 1, true)?;
    ensure_glm_predictor(&mut driver, 2, true)?;
    ensure_glm_predictor(&mut driver, 3, true)?;
    ensure_glm_predictor(&mut driver, 4, true)?;
    confirm_modal_ok(&mut driver)?;

    let aic_full = {
        let rs = result_sheet(driver.app().document(), &pois_id)?;
        let deviance = scalar_val(rs, "Residual deviance", None)?;
        let dispersion = scalar_val(rs, "Pearson dispersion", None)?;
        let aic = scalar_val(rs, "AIC", None)?;
        let mcfadden = scalar_val(rs, "McFadden R squared", None)?;
        ensure(deviance >= 0.0 && dispersion > 0.0 && aic.is_finite() && mcfadden > 0.70, "unexpected Poisson scalars")?;

        let params = tab_by_id(rs, "glm-poisson-params")?;
        ensure(params.column_headers.len() == 7 && params.column_headers[6] == "VIF", "expected 7 columns with VIF")?;
        ensure(params.rows.len() == 5, "expected Intercept + 4 terms")?;
        let beta_x1 = cell_num(&params.rows[1][0], "beta X1")?;
        let irr_x1 = cell_num(&params.rows[1][4], "IRR X1")?;
        assert_close(irr_x1, beta_x1.exp(), 1e-10, "Rate Ratio == exp(beta)")?;
        let vif_x1 = cell_num(&params.rows[1][6], "VIF X1")?;
        let vif_x2 = cell_num(&params.rows[2][6], "VIF X2")?;
        ensure(vif_x1 >= 10.0 && vif_x2 >= 10.0, format!("collinear ExposureX1/X2 must trigger VIF >= 10: {vif_x1}, {vif_x2}"))?;

        let resid = tab_by_id(rs, "glm-poisson-residuals")?;
        ensure(resid.rows.len() == 126, "expected 126 Poisson residual rows")?;
        ensure(
            rs.checklist.iter().any(|c| c.assumption == "Equidispersion assumption (Poisson)" && c.remediation.is_none()),
            "missing QOL-7 Poisson equidispersion item",
        )?;
        aic
    };

    // Scroll large-N Poisson residuals table beyond row 120.
    scroll_result_table_to_cell(&mut driver, &pois_id, "glm-poisson-residuals", 120, 1)?;

    // Switch MV graph to Poisson Residual Plot and verify render frame.
    switch_graph_type_human(&mut driver, &graph_id, "diag.residual", false)?;
    let _ = verify_graph_renders(&mut driver, &graph_id)?;

    // Prune collinear ExposureX2 (`edit.glm.pred.2`) via Cmd+T -> VIF collapses < 5.0!
    open_edit_params_cmd_t(&mut driver, &pois_id)?;
    click_modal_control(&mut driver, "edit.content_panel", "edit.glm.pred.2")?;
    confirm_modal_ok(&mut driver)?;
    let (aic_pruned, disp_pruned) = {
        let rs_pruned = result_sheet(driver.app().document(), &pois_id)?;
        let params_pruned = tab_by_id(rs_pruned, "glm-poisson-params")?;
        ensure(params_pruned.rows.len() == 4, format!("expected 4 rows after pruning ExposureX2, got {}", params_pruned.rows.len()))?;
        for r in 1..4 {
            let v = cell_num(&params_pruned.rows[r][6], "pruned VIF")?;
            ensure(v < 5.0, format!("pruned predictor VIF must drop < 5.0, got {v}"))?;
        }
        let aic_p = scalar_val(rs_pruned, "AIC", None)?;
        let disp_p = scalar_val(rs_pruned, "Pearson dispersion", None)?;
        ensure((aic_p - aic_full).abs() > 0.1, format!("pruning must update AIC ({aic_full} -> {aic_p})"))?;
        (aic_p, disp_p)
    };

    // Live upstream count cell edit triggers Poisson reactive DAG recompute.
    edit_grid_cell_live(&mut driver, &mv_id, 10, 0, 0, "180")?;
    let disp_after = scalar_val(result_sheet(driver.app().document(), &pois_id)?, "Pearson dispersion", None)?;
    let aic_after = scalar_val(result_sheet(driver.app().document(), &pois_id)?, "AIC", None)?;
    ensure(
        (aic_after - aic_pruned).abs() > 1.0 && (disp_after - disp_pruned).abs() > 0.05,
        format!("live edit must recompute Poisson AIC ({aic_pruned} -> {aic_after}) & dispersion ({disp_pruned} -> {disp_after})"),
    )?;
    Ok(())
}

// ============================================================================
// WF-43: XY Serial Dilution (Log10(X), Constrained 4PL IC50, AUC, Overlay)
// ============================================================================

#[test]
fn wf43_human_xy_serial_dilution_constrained_4pl_and_auc() -> TestRes {
    let mut app = UiApp::new();
    app.nav.welcome.open = true;
    app.nav.gallery.open = true;
    let mut driver = new_human_driver(app, default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // Create 8-row, 1-dataset, 2-replicate XY sheet via Welcome launcher.
    let _ = click_id(&mut driver, "welcome.arch.xy")?;
    let _ = click_id(&mut driver, "welcome.subcol.raw")?;
    let _ = click_id(&mut driver, "welcome.datasets.dec")?;
    for _ in 0..4 {
        let _ = click_id(&mut driver, "welcome.rows.dec")?;
    }
    replace_text_in_widget(&mut driver, "welcome.title.input", "Serial Dilution Viability")?;
    let _ = click_id(&mut driver, "welcome.create_cta")?;
    ensure(!driver.app().nav.welcome.open, "Welcome launcher did not close after Create")?;

    let xy_id = driver.app().document().data_sheets.last().map(|d| d.id.clone()).ok_or_else(|| String::from("missing created XY sheet"))?;
    select_sheet_in_nav(&mut driver, &xy_id)?;

    // Rename X column title via header editor.
    let _ = click_id(&mut driver, "grid.x_title.0")?;
    replace_text_in_widget(&mut driver, "grid.title_editor.input", "Concentration (M)")?;
    let _ = click_id(&mut driver, "grid.title_editor.apply")?;

    // Enter molar concentrations & replicate viabilities via double-click inline cell editor.
    let concs = [1e-9_f64, 3.16227766e-9, 1e-8, 3.16227766e-8, 1e-7, 3.16227766e-7, 1e-6, 2e-6];
    let true_hill = |lx: f64| 1.0 / (1.0 + 10_f64.powf(lx - (-6.0)));
    for (r, &c) in concs.iter().enumerate() {
        let base = true_hill(c.log10());
        let jitter = if r % 2 == 0 { 0.012 } else { -0.012 };
        edit_x_cell_live(&mut driver, &xy_id, r, 0, &format!("{c:.8e}"))?;
        edit_grid_cell_live(&mut driver, &xy_id, r, 0, 0, &format!("{:.6}", base + jitter))?;
        edit_grid_cell_live(&mut driver, &xy_id, r, 0, 1, &format!("{:.6}", base - jitter))?;
    }

    // Step 1: Log10(X) transform (`x_transform: Log10`, `y_transform: None`).
    let log_rs_id = run_analyze_human(&mut driver, &xy_id, "transform.math")?;
    open_edit_params_cmd_t(&mut driver, &log_rs_id)?;
    cycle_dialog_control_until(&mut driver, "edit.transform.x_transform", 12, |app| {
        matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), .. }))
        )
    })?;
    cycle_dialog_control_until(&mut driver, "edit.transform.y_transform", 12, |app| {
        matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: None, .. }))
        )
    })?;
    confirm_modal_ok(&mut driver)?;
    ensure(
        matches!(
            result_sheet(driver.app().document(), &log_rs_id)?.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), y_transform: None, .. })
        ),
        "expected Log10(X) and untransformed Y on transform.math ResultSheet",
    )?;

    // Step 2: XY Descriptive Statistics on Log10(X) ResultSheet -> Trapezoidal AUC scalar.
    let desc_rs_id = run_analyze_human(&mut driver, &log_rs_id, "stats.descriptive")?;
    let desc_trap_auc = scalar_val(result_sheet(driver.app().document(), &desc_rs_id)?, "Trapezoidal AUC", Some(0))?;
    ensure(desc_trap_auc > 2.0 && desc_trap_auc < 3.5, format!("unexpected XY Trapezoidal AUC: {desc_trap_auc}"))?;

    // Step 3: Unconstrained 4PL nonlinear fit + Profile Likelihood CI via fit_params tabs.
    let free_id = run_analyze_human(&mut driver, &log_rs_id, "fit.nonlinear")?;
    open_edit_params_cmd_t(&mut driver, &free_id)?;
    let _ = click_id(&mut driver, "fit.tab.model")?;
    replace_text_in_widget(&mut driver, "fit.eq.search", "log_inhibitor_vs_response_4pl")?;
    click_modal_control(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.log_inhibitor_vs_response_4pl")?;
    let _ = click_id(&mut driver, "fit.tab.confidence")?;
    let _ = click_id(&mut driver, "fit.ci.profile")?;
    confirm_modal_ok(&mut driver)?;

    // Step 4: Constrained Bottom=0.0, Top=1.0 4PL fit + Profile Likelihood CI + Confidence bands.
    let const_id = run_analyze_human(&mut driver, &log_rs_id, "fit.nonlinear")?;
    open_edit_params_cmd_t(&mut driver, &const_id)?;
    let _ = click_id(&mut driver, "fit.tab.model")?;
    replace_text_in_widget(&mut driver, "fit.eq.search", "log_inhibitor_vs_response_4pl")?;
    click_modal_control(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.log_inhibitor_vs_response_4pl")?;

    let _ = click_id(&mut driver, "fit.tab.constraints")?;
    let _ = click_id(&mut driver, "fit.con.row.0")?;
    let _ = click_id(&mut driver, "fit.con.set_scope.constant")?;
    replace_text_in_widget(&mut driver, "fit.con.const.input", "0.0")?;
    let _ = click_id(&mut driver, "fit.con.apply")?;

    let _ = click_id(&mut driver, "fit.con.row.1")?;
    let _ = click_id(&mut driver, "fit.con.set_scope.constant")?;
    replace_text_in_widget(&mut driver, "fit.con.const.input", "1.0")?;
    let _ = click_id(&mut driver, "fit.con.apply")?;

    let _ = click_id(&mut driver, "fit.tab.confidence")?;
    let _ = click_id(&mut driver, "fit.ci.profile")?;
    let _ = click_id(&mut driver, "fit.ci.conf_bands")?;
    confirm_modal_ok(&mut driver)?;

    let (ic50_const_initial, fit_auc_const_initial, trap_auc_initial) = {
        let free_rs = result_sheet(driver.app().document(), &free_id)?;
        let const_rs = result_sheet(driver.app().document(), &const_id)?;
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
        assert_close(trap_auc, desc_trap_auc, 1e-8, "fit.nonlinear AUC (trapezoidal) matches descriptive AUC")?;
        ensure(fit_auc_const > 1.5 && fit_auc_free > 1.5, "positive fitted AUCs expected")?;

        let params_const = tab_by_id(const_rs, "nonlinear_parameters")?;
        let logic50_row = params_const.row_labels.iter().position(|l| l == "LogIC50").ok_or("missing LogIC50")?;
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(c_lo), upper: CiBound::Finite(c_hi) } = params_const.rows[logic50_row][2] else {
            return Err("expected finite LogIC50 CI bounds on constrained fit".to_owned());
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
            ensure(const_width <= free_width + 1e-9, format!("constrained CI {const_width} <= free {free_width}"))?;
        }
        (ic50_const, fit_auc_const, trap_auc)
    };

    // Inspect constrained 4PL tabs (`nonlinear_parameters`, `fit_auc`).
    switch_result_tab_human(&mut driver, &const_id, "fit_auc")?;

    // Create XY graph from Log10(X) ResultSheet and attach constrained 4PL Regression Overlay via Inspector.
    select_sheet_in_nav(&mut driver, &log_rs_id)?;
    let fit_graph_id = create_graph_for_active_sheet(&mut driver)?;
    select_sheet_in_nav(&mut driver, &fit_graph_id)?;
    let _ = click_id(&mut driver, "inspector.tab.graph")?;
    click_modal_control(&mut driver, "scroll.inspector", "inspector.regression.result_sheet")?;
    cycle_dialog_control_until(&mut driver, "inspector.regression.result_sheet", 8, |app| {
        app.document()
            .graph_sheet(&fit_graph_id)
            .is_some_and(|g| g.spec.series.first().and_then(|s| s.regression_overlay.as_ref()).is_some_and(|o| o.result_sheet_id == const_id))
    })?;
    if driver.app().document().graph_sheet(&fit_graph_id).is_some_and(|g| g.spec.series.iter().any(|s| s.regression_overlay.is_none())) {
        click_modal_control(&mut driver, "scroll.inspector", "inspector.regression.all_series")?;
    }
    if driver
        .app()
        .document()
        .graph_sheet(&fit_graph_id)
        .and_then(|g| g.spec.series.first().and_then(|s| s.regression_overlay.as_ref()))
        .is_some_and(|o| !o.show_confidence_band)
    {
        click_modal_control(&mut driver, "scroll.inspector", "inspector.regression.confidence_band")?;
    }
    if driver
        .app()
        .document()
        .graph_sheet(&fit_graph_id)
        .and_then(|g| g.spec.series.first().and_then(|s| s.regression_overlay.as_ref()))
        .is_some_and(|o| !o.shaded_ribbon)
    {
        click_modal_control(&mut driver, "scroll.inspector", "inspector.regression.shaded_ribbon")?;
    }
    let fit_scene = verify_graph_renders(&mut driver, &fit_graph_id)?;
    ensure(fit_scene.primitives.len() >= 4, format!("4PL overlay & ribbon must appear in ChartScene: {}", fit_scene.primitives.len()))?;

    // 2-hop reactive DAG cascade: live edit & Cmd+Shift+E exclusion on raw XY propagate through
    // Log10(X) -> stats.descriptive (`Trapezoidal AUC`) AND constrained `fit.nonlinear` (`IC50`, `AUC`).
    edit_grid_cell_live(&mut driver, &xy_id, 6, 0, 0, "0.75")?;
    let ic50_edited = scalar_val(result_sheet(driver.app().document(), &const_id)?, "IC50", Some(0))?;
    let trap_edited = scalar_val(result_sheet(driver.app().document(), &desc_rs_id)?, "Trapezoidal AUC", Some(0))?;
    ensure((ic50_edited - ic50_const_initial).abs() > 1e-9, "2-hop edit must update constrained IC50")?;
    ensure((trap_edited - trap_auc_initial).abs() > 1e-4, "2-hop edit must update Trapezoidal AUC")?;

    exclude_cell_cmd_shift_e(&mut driver, &xy_id, 6, 0, 0, 0, "evaporated border well")?;
    let fit_auc_excl = scalar_val(result_sheet(driver.app().document(), &const_id)?, "AUC (fitted curve)", Some(0))?;
    ensure(
        (fit_auc_excl - fit_auc_const_initial).abs() > 1e-4,
        format!("exclusion must update fitted curve AUC ({fit_auc_const_initial} -> {fit_auc_excl})"),
    )?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}
