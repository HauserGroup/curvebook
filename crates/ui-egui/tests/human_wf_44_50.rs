//! Canonical `R10.2` / `R10.13` zero-shortcut human-path tests for `WF-44` through `WF-50`.
//!
//! Every test drives all 5 workflow stages strictly through [`HumanPathDriver`] raw input events
//! (no `execute_command`, no `dispatch_gui_action`, no `app_mut()` document mutations) and closes
//! with [`human_wf_common::assert_zero_shortcut_compliance`].

#![forbid(unsafe_code)]
#![allow(clippy::cast_precision_loss)]

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, ColorChannelMapping, DiagnosticPlotKind, EquationDisplayMode, GraphAnnotation, GraphKind, MultiplicityMethod, MultivariatePlotMode,
    SheetId, SurvivalAnalysisSpec,
};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{UiApp, ViewportMetrics};
use serde_json::Value;

type TestRes = Result<(), String>;

fn active_data_sheet_id(driver: &HumanPathDriver) -> Result<SheetId, String> {
    let doc = driver.app().document();
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.data_sheet(id).is_some())
        .or_else(|| doc.data_sheets.last().map(|s| &s.id))
        .cloned()
        .ok_or_else(|| String::from("expected an active DataSheet"))
}

fn latest_graph_sheet_id(driver: &HumanPathDriver) -> Result<SheetId, String> {
    driver.app().document().graph_sheets.last().map(|s| s.id.clone()).ok_or_else(|| String::from("expected a GraphSheet in project"))
}

fn latest_result_sheet_id(driver: &HumanPathDriver) -> Result<SheetId, String> {
    driver.app().document().result_sheets.last().map(|s| s.id.clone()).ok_or_else(|| String::from("expected a ResultSheet in project"))
}

fn new_driver(metrics: ViewportMetrics) -> HumanPathDriver {
    human_wf_common::new_human_driver(UiApp::new(), metrics)
}

fn click_id(driver: &mut HumanPathDriver, widget_id: &str) -> TestRes {
    human_wf_common::click_widget_center(driver, &WidgetId::from(widget_id)).map(|_| ())
}

fn scroll_to_id(driver: &mut HumanPathDriver, container_id: &str, target_id: &str, step_y: f64, max_steps: usize) -> TestRes {
    for _ in 0..max_steps {
        if driver.find_widget(target_id).is_some() {
            return Ok(());
        }
        let Some(node) = driver.find_widget(container_id) else {
            return Err(format!("scroll container `{container_id}` not found in LayoutTree while looking for `{target_id}`"));
        };
        let scale = driver.layout_tree().metrics.clamped_scale_factor();
        let [px, py] = node.bounds.rect_px.center();
        let lx = f64::from(px) / scale;
        let ly = f64::from(py) / scale;
        let _ = driver.wheel_logical(lx, ly, 0.0, step_y, KeyModifiers::default())?;
    }
    if driver.find_widget(target_id).is_some() {
        Ok(())
    } else {
        Err(format!("widget `{target_id}` did not become visible inside `{container_id}` after {max_steps} scroll steps"))
    }
}

/// Click a widget via [`human_wf_common::widget_hit_center`] and return the [`UiEventOutcome`].
fn click_widget_outcome(driver: &mut HumanPathDriver, widget_id: &str) -> Result<UiEventOutcome, String> {
    let [x, y] = human_wf_common::widget_hit_center(driver, &WidgetId::from(widget_id))?;
    driver.click_logical(x, y)
}

/// Select all text inside a focused input field, clear it, and type `replacement`.
fn replace_widget_text(driver: &mut HumanPathDriver, widget_id: &str, replacement: &str) -> TestRes {
    click_id(driver, widget_id)?;
    human_wf_common::press_cmd_shortcut(driver, "a")?;
    driver.press_key("Backspace", KeyModifiers::default())?;
    human_wf_common::type_into_focused(driver, replacement).map(|_| ())
}

/// Create a typed table cleanly from the Welcome launcher using pointer and stepper clicks.
fn create_table_via_welcome(
    driver: &mut HumanPathDriver,
    arch_widget_id: &str,
    title: &str,
    datasets: usize,
    rows: usize,
) -> Result<SheetId, String> {
    click_id(driver, arch_widget_id)?;
    click_id(driver, "welcome.tab.empty")?;
    replace_widget_text(driver, "welcome.title.input", title)?;

    while driver.app().nav.welcome.table_draft.data_set_count < datasets {
        click_id(driver, "welcome.datasets.inc")?;
    }
    while driver.app().nav.welcome.table_draft.data_set_count > datasets {
        click_id(driver, "welcome.datasets.dec")?;
    }

    if driver.find_widget("welcome.rows.inc").is_none() {
        scroll_to_id(driver, "welcome.right_body", "welcome.rows.inc", 80.0, 10)?;
    }
    while driver.app().nav.welcome.table_draft.row_count < rows {
        click_id(driver, "welcome.rows.inc")?;
    }
    while driver.app().nav.welcome.table_draft.row_count > rows {
        click_id(driver, "welcome.rows.dec")?;
    }

    click_id(driver, "welcome.create_cta")?;
    active_data_sheet_id(driver)
}

/// Ensure grid widget `cell_id` is scrolled into view, double-click it, type `text`, and commit via `Enter`.
fn enter_cell_text(driver: &mut HumanPathDriver, ds_id: &SheetId, cell_id: &str, text: &str) -> TestRes {
    let scroll_id = format!("grid.sheet.{}", ds_id.as_str());
    let is_top_row = cell_id.starts_with("grid.cell.0.") || cell_id.starts_with("grid.xcell.0.");
    if driver.find_widget(cell_id).is_none() {
        let dir = if is_top_row || cell_id.starts_with("grid.cell.1.") { -240.0 } else { 120.0 };
        if scroll_to_id(driver, &scroll_id, cell_id, dir, 60).is_err() {
            scroll_to_id(driver, &scroll_id, cell_id, -dir, 60)?;
        }
    }
    if is_top_row && let Some(node) = driver.find_widget(&scroll_id) {
        // Ensure row 0 is completely below the sticky header before double-clicking its center.
        let scale = driver.layout_tree().metrics.clamped_scale_factor();
        let [px, py] = node.bounds.rect_px.center();
        let _ = driver.wheel_logical(f64::from(px) / scale, f64::from(py) / scale, 0.0, -240.0, KeyModifiers::default())?;
    }
    let [x, y] = human_wf_common::widget_hit_center(driver, &WidgetId::from(cell_id))?;
    let dbl_out = driver.double_click_logical(x, y)?;
    if !driver.app().inline_editor.active {
        return Err(format!("double_click_logical on `{cell_id}` did not open inline editor (outcome={dbl_out:?})"));
    }
    driver.type_text(text)?;
    driver.press_key("Enter", KeyModifiers::default())?;
    Ok(())
}

fn enter_grid_cell(driver: &mut HumanPathDriver, ds_id: &SheetId, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
    enter_cell_text(driver, ds_id, &format!("grid.cell.{row}.{col}.{subcol}"), text)
}

fn enter_x_cell(driver: &mut HumanPathDriver, ds_id: &SheetId, row: usize, subcol: usize, text: &str) -> TestRes {
    enter_cell_text(driver, ds_id, &format!("grid.xcell.{row}.{subcol}"), text)
}

fn rename_data_column(driver: &mut HumanPathDriver, col: usize, title: &str) -> TestRes {
    let hdr_id = format!("grid.col_title.{col}");
    click_id(driver, &hdr_id)?;
    replace_widget_text(driver, "grid.title_editor.input", title)?;
    click_id(driver, "grid.title_editor.apply")
}

fn rename_x_column(driver: &mut HumanPathDriver, subcol: usize, title: &str) -> TestRes {
    let hdr_id = format!("grid.x_title.{subcol}");
    click_id(driver, &hdr_id)?;
    replace_widget_text(driver, "grid.title_editor.input", title)?;
    click_id(driver, "grid.title_editor.apply")
}

fn set_mv_column_role(driver: &mut HumanPathDriver, col: usize, role_key: &str) -> TestRes {
    click_id(driver, &format!("mv_formula.role_badge.{col}"))?;
    click_id(driver, &format!("mv_formula.role_opt.{col}.{role_key}"))
}

fn switch_sheet_via_nav(driver: &mut HumanPathDriver, sheet_id: &SheetId) -> TestRes {
    let nav_id = format!("nav.sheet.{}", sheet_id.as_str());
    if driver.find_widget(&nav_id).is_none() {
        scroll_to_id(driver, "navigator.tree", &nav_id, 90.0, 20)?;
    }
    click_id(driver, &nav_id)?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        click_id(driver, "modal.ok")?;
    }
    Ok(())
}

fn open_top_menu_item(driver: &mut HumanPathDriver, menu_id: &str, item_id: &str) -> TestRes {
    click_id(driver, menu_id)?;
    if driver.find_widget(item_id).is_none() {
        scroll_to_id(driver, &format!("popup.{menu_id}"), item_id, 96.0, 24)?;
    }
    click_id(driver, item_id)
}

fn run_analysis_via_toolbar(driver: &mut HumanPathDriver, entry_id: &str) -> Result<SheetId, String> {
    click_id(driver, "toolbar.analyze")?;
    let widget_id = format!("analyze.entry.{entry_id}");
    if driver.find_widget(&widget_id).is_none() {
        scroll_to_id(driver, "analyze.entries_list", &widget_id, 100.0, 30)?;
    }
    click_id(driver, &widget_id)?;
    click_id(driver, "modal.ok")?;
    latest_result_sheet_id(driver)
}

fn select_inspector_popup(driver: &mut HumanPathDriver, tab_id: &str, btn_id: &str, popup_kind: &str, item_idx: usize) -> TestRes {
    click_id(driver, tab_id)?;
    if driver.find_widget(btn_id).is_none() {
        scroll_to_id(driver, "scroll.inspector", btn_id, 96.0, 16)?;
    }
    click_id(driver, btn_id)?;
    let item_id = format!("inspector.popup.{popup_kind}.item_{item_idx}");
    click_id(driver, &item_id)
}

fn click_inspector_control(driver: &mut HumanPathDriver, tab_id: &str, control_id: &str) -> TestRes {
    click_id(driver, tab_id)?;
    if driver.find_widget(control_id).is_none() {
        scroll_to_id(driver, "scroll.inspector", control_id, 96.0, 16)?;
    }
    click_id(driver, control_id)
}

/// Export active graph/layout using `File -> Export Figure...` modal and return decoded bytes or UTF-8 SVG text.
fn export_figure_via_menu(driver: &mut HumanPathDriver, fmt_slug: &str, dpi_id: Option<&str>) -> Result<Vec<u8>, String> {
    open_top_menu_item(driver, "menu.file", "menu.ui.export.figure.open")?;
    click_id(driver, &format!("export.fmt.{fmt_slug}"))?;
    if let Some(dpi) = dpi_id {
        click_id(driver, dpi)?;
    }
    let out = click_widget_outcome(driver, "modal.ok")?;
    let res = out.command_result.ok_or_else(|| format!("ExportFigure ({fmt_slug}) produced no command_result"))?;
    let payload = res.get("payload").unwrap_or(&res);
    if fmt_slug == "svg" {
        let svg = payload.get("svg").and_then(Value::as_str).ok_or_else(|| format!("expected SVG string in export payload: {payload}"))?;
        return Ok(svg.as_bytes().to_vec());
    }
    let b64 =
        payload.get("base64").and_then(Value::as_str).ok_or_else(|| format!("expected base64 string in {fmt_slug} export payload: {payload}"))?;
    decode_base64(b64)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn val(b: u8) -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte {b}")),
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("invalid base64 length {}", bytes.len()));
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

fn assert_non_white_rgba_frame(driver: &mut HumanPathDriver, ctx: &str) -> Result<Vec<u8>, String> {
    let frame = driver.render_frame().map_err(|e| format!("{ctx}: render_frame failed: {e}"))?;
    let non_white = frame.rgba.as_chunks::<4>().0.iter().filter(|px| px[3] > 0 && (px[0] < 250 || px[1] < 250 || px[2] < 250)).count();
    if non_white < 50 {
        return Err(format!("{ctx}: rendered RGBA frame had too few non-white pixels ({non_white})"));
    }
    Ok(frame.rgba)
}

fn assert_svg_valid_with(svg_bytes: &[u8], required: &[&str], ctx: &str) -> Result<String, String> {
    let svg = String::from_utf8(svg_bytes.to_vec()).map_err(|e| format!("{ctx}: invalid UTF-8 SVG: {e}"))?;
    if !svg.contains("<svg") || !svg.contains("</svg>") {
        return Err(format!("{ctx}: invalid SVG envelope"));
    }
    for token in required {
        if !svg.contains(token) {
            return Err(format!("{ctx}: SVG missing expected token `{token}`"));
        }
    }
    Ok(svg)
}

fn assert_pdf_1_7(pdf_bytes: &[u8], min_len: usize, ctx: &str) -> TestRes {
    if !pdf_bytes.starts_with(b"%PDF-1.7") {
        return Err(format!("{ctx}: expected %PDF-1.7 header"));
    }
    if pdf_bytes.len() < min_len {
        return Err(format!("{ctx}: PDF unexpectedly short ({} < {min_len})", pdf_bytes.len()));
    }
    Ok(())
}

fn assert_png_valid(png_bytes: &[u8], min_len: usize, ctx: &str) -> TestRes {
    if !png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!("{ctx}: expected PNG magic header"));
    }
    if png_bytes.len() < min_len {
        return Err(format!("{ctx}: PNG unexpectedly short ({} < {min_len})", png_bytes.len()));
    }
    Ok(())
}

fn count_ellipse_dashed_paths(svg: &str) -> usize {
    svg.split("<path").skip(1).filter(|s| s.contains("stroke-dasharray")).count()
}

fn count_closed_polygon_paths(svg: &str) -> usize {
    svg.split("<path").skip(1).filter(|s| s.contains(" Z\"")).count()
}

fn assert_equation_callout_equations_only(graph_doc: &curvebook_doc::GraphSheet, svg: &str) -> TestRes {
    let found_eq = graph_doc.spec.annotations.iter().any(|ann| matches!(ann, GraphAnnotation::ModelEquation { .. }));
    if !found_eq {
        return Err(String::from("expected at least one ModelEquation annotation on GraphSheet"));
    }
    for forbidden in ["R^2 =", "R² =", "AICc =", "Sy.x =", "p = 0."] {
        if svg.contains(forbidden) {
            return Err(format!("equation SVG callout contained disallowed summary badge `{forbidden}`"));
        }
    }
    Ok(())
}

// ============================================================================
// WF-44: Confidence Ellipses & Convex Hulls on Multivariate Scatter/Bubble
// ============================================================================

#[test]
fn wf44_human_confidence_ellipses_and_convex_hulls() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Stage 1: Create Multivariate table via Welcome launcher.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.multiple_variables", "WF-44 Iris Clusters", 3, 12)?;

    // Stage 2: Rename columns, set Species categorical role, and enter 3 x 4 cluster rows via keyboard.
    rename_data_column(&mut driver, 0, "SepalLen")?;
    rename_data_column(&mut driver, 1, "SepalWid")?;
    rename_data_column(&mut driver, 2, "Species")?;
    set_mv_column_role(&mut driver, 2, "categorical")?;

    let rows: [(&str, &str, &str); 12] = [
        ("4.9", "3.0", "Setosa"),
        ("5.1", "3.4", "Setosa"),
        ("5.4", "3.7", "Setosa"),
        ("4.7", "3.2", "Setosa"),
        ("6.0", "2.7", "Versicolor"),
        ("6.4", "3.1", "Versicolor"),
        ("5.8", "2.6", "Versicolor"),
        ("6.2", "2.9", "Versicolor"),
        ("7.1", "3.3", "Virginica"),
        ("7.4", "3.6", "Virginica"),
        ("6.9", "3.0", "Virginica"),
        ("7.6", "3.4", "Virginica"),
    ];
    for (r, (x, y, sp)) in rows.iter().enumerate() {
        enter_grid_cell(&mut driver, &ds_id, r, 0, 0, x)?;
        enter_grid_cell(&mut driver, &ds_id, r, 1, 0, y)?;
        enter_grid_cell(&mut driver, &ds_id, r, 2, 0, sp)?;
    }

    // Stage 3: Switch to GraphSheet, configure Multivariate channels, and toggle ellipses + hulls.
    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.x_btn", "mv_x", 0)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.y_btn", "mv_y", 1)?;
    // Color popup index 0 = "None", so col 2 ("Species") is item_3.
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.color_btn", "mv_color", 3)?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.color_cat")?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.ellipse_toggle")?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.hull_toggle")?;

    let gs = driver.app().document().graph_sheet(&gs_id).ok_or("missing WF-44 GraphSheet")?;
    let mv = gs.spec.multivariate.as_ref().ok_or("missing multivariate spec")?;
    if !mv.show_confidence_ellipses || !mv.show_convex_hulls {
        return Err(format!("expected ellipses & hulls enabled, got {:?}", mv));
    }

    // Stage 4: Render RGBA and export SVG checking dashed ellipses + convex hull polygons.
    let _ = assert_non_white_rgba_frame(&mut driver, "WF-44 initial")?;
    let svg_before_bytes = export_figure_via_menu(&mut driver, "svg", None)?;
    let svg_before = assert_svg_valid_with(&svg_before_bytes, &["Setosa", "Versicolor", "Virginica"], "WF-44 before")?;
    let dashed_count = count_ellipse_dashed_paths(&svg_before);
    let hull_count = count_closed_polygon_paths(&svg_before);
    if dashed_count < 3 {
        return Err(format!("WF-44: expected >= 3 dashed ellipse paths, got {dashed_count}"));
    }
    if hull_count < 3 {
        return Err(format!("WF-44: expected >= 3 closed convex hull paths, got {hull_count}"));
    }

    // Stage 5: Live upstream DataSheet cell edit refreshes ellipse/hull geometry in SVG.
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    enter_grid_cell(&mut driver, &ds_id, 0, 0, 0, "4.1")?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_after_bytes = export_figure_via_menu(&mut driver, "svg", None)?;
    let svg_after = assert_svg_valid_with(&svg_after_bytes, &["Setosa"], "WF-44 after edit")?;
    if svg_after == svg_before {
        return Err(String::from("WF-44: SVG did not refresh after upstream cluster cell edit"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-45: Grouped 2D Matrix & MV Cross-Tab Summary Heatmaps + Correlation Matrix
// ============================================================================

#[test]
fn wf45_human_grouped_and_mv_cross_tab_heatmaps() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Sub-workflow A: Grouped table -> 2D Matrix Heatmap via ChangeGraphType.
    let grp_id = create_table_via_welcome(&mut driver, "welcome.arch.grouped", "WF-45 Grouped Heatmap", 3, 3)?;
    rename_data_column(&mut driver, 0, "TreatA")?;
    rename_data_column(&mut driver, 1, "TreatB")?;
    rename_data_column(&mut driver, 2, "TreatC")?;
    let matrix = [["1.2", "3.4", "5.6"], ["2.0", "4.8", "6.2"], ["0.8", "2.9", "7.1"]];
    for (r, row_vals) in matrix.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_grid_cell(&mut driver, &grp_id, r, c, 0, val)?;
        }
    }

    let grp_graph_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &grp_graph_id)?;
    open_top_menu_item(&mut driver, "menu.graph", "menu.ui.changeGraphType.open")?;
    if driver.find_widget("graph_type.card.diag.heatmap").is_none() {
        scroll_to_id(&mut driver, "graph_type.panel", "graph_type.card.diag.heatmap", 90.0, 12)?;
    }
    click_id(&mut driver, "graph_type.card.diag.heatmap")?;
    click_id(&mut driver, "modal.ok")?;

    let grp_gs = driver.app().document().graph_sheet(&grp_graph_id).ok_or("missing Grouped GraphSheet")?;
    if grp_gs.spec.kind != GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap) {
        return Err(format!("expected Diagnostic(Heatmap), got {:?}", grp_gs.spec.kind));
    }
    let grp_svg_1 = export_figure_via_menu(&mut driver, "svg", None)?;
    let grp_svg_str_1 = assert_svg_valid_with(&grp_svg_1, &["TreatA", "TreatB", "TreatC"], "WF-45 grouped heatmap")?;
    let filled_path_count = grp_svg_str_1.split("<path").skip(1).filter(|s| s.contains("fill=\"#")).count();
    if filled_path_count < 9 {
        return Err(format!("WF-45 grouped heatmap expected >= 9 filled tile <path> elements, got {filled_path_count}"));
    }

    // Upstream Grouped edit refreshes heatmap SVG.
    switch_sheet_via_nav(&mut driver, &grp_id)?;
    enter_grid_cell(&mut driver, &grp_id, 0, 2, 0, "12.5")?;
    switch_sheet_via_nav(&mut driver, &grp_graph_id)?;
    let grp_svg_2 = export_figure_via_menu(&mut driver, "svg", None)?;
    if grp_svg_2 == grp_svg_1 {
        return Err(String::from("WF-45 grouped heatmap SVG did not refresh on cell edit"));
    }

    // Sub-workflow B: Multivariate table -> Correlation Matrix + Cross-Tab Summary Heatmap.
    click_id(&mut driver, "toolbar.new")?;
    let mv_id = create_table_via_welcome(&mut driver, "welcome.arch.multiple_variables", "WF-45 MV CrossTab", 4, 8)?;
    rename_data_column(&mut driver, 0, "Dose")?;
    rename_data_column(&mut driver, 1, "Sex")?;
    rename_data_column(&mut driver, 2, "Response")?;
    rename_data_column(&mut driver, 3, "Biomarker")?;
    set_mv_column_role(&mut driver, 0, "categorical")?;
    set_mv_column_role(&mut driver, 1, "categorical")?;

    let mv_rows = [
        ("Low", "F", "10.0", "1.2"),
        ("Low", "M", "14.0", "1.8"),
        ("Mid", "F", "22.0", "2.9"),
        ("Mid", "M", "28.0", "3.4"),
        ("High", "F", "40.0", "5.1"),
        ("High", "M", "46.0", "5.8"),
        ("Low", "F", "12.0", "1.4"),
        ("High", "M", "50.0", "6.2"),
    ];
    for (r, (d, s, resp, bio)) in mv_rows.iter().enumerate() {
        enter_grid_cell(&mut driver, &mv_id, r, 0, 0, d)?;
        enter_grid_cell(&mut driver, &mv_id, r, 1, 0, s)?;
        enter_grid_cell(&mut driver, &mv_id, r, 2, 0, resp)?;
        enter_grid_cell(&mut driver, &mv_id, r, 3, 0, bio)?;
    }

    let corr_rs_id = run_analysis_via_toolbar(&mut driver, "stats.corr_matrix")?;
    let corr_rs = driver.app().document().result_sheet(&corr_rs_id).ok_or("missing corr_matrix ResultSheet")?;
    if !matches!(corr_rs.analysis.kind, AnalysisKind::Multivariate(_)) {
        return Err(format!("expected Multivariate analysis kind, got {:?}", corr_rs.analysis.kind));
    }

    // Switch to MV GraphSheet & toggle CategoricalSummary cross-tab summary heatmap + metric switching.
    let mv_graph_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &mv_graph_id)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.x_btn", "mv_x", 0)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.y_btn", "mv_y", 2)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.color_btn", "mv_color", 2)?;

    // Blocked on src/ (#554 aligned): inspector.mv.mode_cat_summary + inspector.mv.metric_median for CategoricalSummary cross-tab heatmap.
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.mode_cat_summary")?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.metric_median")?;
    let mv_gs = driver.app().document().graph_sheet(&mv_graph_id).ok_or("missing MV GraphSheet")?;
    let mv_spec = mv_gs.spec.multivariate.as_ref().ok_or("missing MV encoding")?;
    if !matches!(mv_spec.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }) {
        return Err(format!("expected MultivariatePlotMode::CategoricalSummary, got {:?}", mv_spec.plot_mode));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-46: Interactive MV Inspector Swapping, Large-N Scroll, Drill-Back & IME
// ============================================================================

#[test]
fn wf46_human_interactive_mv_inspector_variable_swapping() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Stage 1: Large-N (52 rows) Multivariate table via Welcome stepper.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.multiple_variables", "WF-46 MV Inspector", 4, 52)?;
    rename_data_column(&mut driver, 0, "Weight")?;
    rename_data_column(&mut driver, 1, "Metabolism")?;
    rename_data_column(&mut driver, 2, "Dosage")?;
    rename_data_column(&mut driver, 3, "Cohort")?;
    set_mv_column_role(&mut driver, 3, "categorical")?;

    // Stage 2: Populate rows (including wheel-scrolling to bottom row 49 + IME CJK & RTL Arabic cell entries).
    for r in 0..8 {
        let w = format!("{:.1}", 10.0 + (r as f64) * 2.5);
        let m = format!("{:.1}", 25.0 + (r as f64) * 4.0);
        let d = format!("{:.1}", 1.5 + (r as f64) * 0.8);
        let c = if r % 2 == 0 { "Control" } else { "Treated" };
        enter_grid_cell(&mut driver, &ds_id, r, 0, 0, &w)?;
        enter_grid_cell(&mut driver, &ds_id, r, 1, 0, &m)?;
        enter_grid_cell(&mut driver, &ds_id, r, 2, 0, &d)?;
        enter_grid_cell(&mut driver, &ds_id, r, 3, 0, c)?;
    }

    // Large-N grid wheel scroll down to row 49 and enter UTF-8 IME CJK + RTL Arabic group labels cleanly.
    let grid_scroll_id = format!("grid.sheet.{}", ds_id.as_str());
    scroll_to_id(&mut driver, &grid_scroll_id, "grid.cell.49.0.0", 240.0, 40)?;
    enter_grid_cell(&mut driver, &ds_id, 49, 0, 0, "88.0")?;
    enter_grid_cell(&mut driver, &ds_id, 49, 1, 0, "142.0")?;
    enter_grid_cell(&mut driver, &ds_id, 49, 2, 0, "9.5")?;

    let [cx_cjk, cy_cjk] = human_wf_common::widget_hit_center(&mut driver, &WidgetId::from("grid.cell.49.3.0"))?;
    let _ = driver.double_click_logical(cx_cjk, cy_cjk)?;
    if !driver.app().inline_editor.active {
        return Err(String::from("WF-46: double_click_logical on `grid.cell.49.3.0` did not open inline editor"));
    }
    driver.set_ime_preedit(Some(String::from("し")));
    driver.set_ime_preedit(None);
    driver.type_text("試験群")?;
    driver.press_key("Enter", KeyModifiers::default())?;

    // Also populate row 50 with RTL Arabic label cleanly after wheel scroll.
    scroll_to_id(&mut driver, &grid_scroll_id, "grid.cell.50.3.0", 120.0, 10)?;
    enter_grid_cell(&mut driver, &ds_id, 50, 0, 0, "91.0")?;
    enter_grid_cell(&mut driver, &ds_id, 50, 1, 0, "148.0")?;
    enter_grid_cell(&mut driver, &ds_id, 50, 2, 0, "10.0")?;
    enter_grid_cell(&mut driver, &ds_id, 50, 3, 0, "مراقبة")?;

    // Stage 3: Switch to GraphSheet and swap X <-> Y, toggle Color Cat/Viridis, toggle Size None/Variable.
    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;

    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.x_btn", "mv_x", 0)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.y_btn", "mv_y", 1)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.color_btn", "mv_color", 4)?; // Cohort col 3 -> item_4
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.color_cat")?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.size_btn", "mv_size", 3)?; // Dosage col 2 -> item_3

    let svg_before_swap = export_figure_via_menu(&mut driver, "svg", None)?;
    let _ = assert_svg_valid_with(&svg_before_swap, &["Control", "Treated", "試験群", "مراقبة", "<path"], "WF-46 initial channels")?;

    // Swap X <-> Y, switch Color to Continuous Viridis on col 2 ("Dosage"), and clear Size to None (item_0).
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.x_btn", "mv_x", 1)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.y_btn", "mv_y", 0)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.color_btn", "mv_color", 3)?; // Dosage col 2 -> item_3
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.color_cont")?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.size_btn", "mv_size", 0)?; // None

    let gs = driver.app().document().graph_sheet(&gs_id).ok_or("missing WF-46 GraphSheet")?;
    let mv = gs.spec.multivariate.as_ref().ok_or("missing WF-46 multivariate spec")?;
    if mv.x_column != 1 || mv.y_column != 0 {
        return Err(format!("WF-46: expected swapped X=1, Y=0, got X={}, Y={}", mv.x_column, mv.y_column));
    }
    if !matches!(mv.color, ColorChannelMapping::Continuous { column: 2, .. }) {
        return Err(format!("WF-46: expected continuous color on col 2, got {:?}", mv.color));
    }

    let svg_after_swap = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_after_swap == svg_before_swap {
        return Err(String::from("WF-46: swapping MV inspector channels did not update SVG output"));
    }

    // Stage 4 & 5: Drill-back from Inspector to Data Cell, edit value, and Return to Graph cleanly.
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.drill.next")?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.drill.go_to_cell")?;
    let cur_ds = active_data_sheet_id(&driver)?;
    if cur_ds != ds_id {
        return Err(format!("WF-46: drill-back expected active DataSheet {}, got {}", ds_id.as_str(), cur_ds.as_str()));
    }
    enter_grid_cell(&mut driver, &ds_id, 0, 1, 0, "75.5")?;
    click_id(&mut driver, "inspector.drill.return_graph")?;
    let _ = assert_non_white_rgba_frame(&mut driver, "WF-46 after return to graph")?;

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-47: Kaplan-Meier Number-at-Risk + Pairwise Log-Rank + Keyboard Focus Nav
// ============================================================================

#[test]
fn wf47_human_kaplan_meier_number_at_risk_and_pairwise() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Stage 1: Create 3-cohort Survival table via Welcome launcher.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.survival", "WF-47 KM Cohorts", 3, 8)?;

    // Stage 2: Rename X & cohort titles and enter event (1) / censored (0) times via keyboard.
    rename_x_column(&mut driver, 0, "Months")?;
    rename_data_column(&mut driver, 0, "Control")?;
    rename_data_column(&mut driver, 1, "LowDose")?;
    rename_data_column(&mut driver, 2, "HighDose")?;

    type KmRow<'a> = (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>);
    let km_rows: [KmRow<'_>; 8] = [
        ("2", Some("1"), None, None),
        ("4", Some("1"), Some("0"), None),
        ("6", Some("1"), Some("1"), Some("0")),
        ("8", Some("0"), Some("1"), None),
        ("10", None, Some("1"), Some("1")),
        ("12", None, Some("0"), Some("0")),
        ("14", None, None, Some("1")),
        ("16", None, None, Some("0")),
    ];
    for (r, (t, c0, c1, c2)) in km_rows.iter().enumerate() {
        enter_x_cell(&mut driver, &ds_id, r, 0, t)?;
        for (col, maybe_status) in [(0usize, c0), (1, c1), (2, c2)] {
            if let Some(st) = maybe_status {
                enter_grid_cell(&mut driver, &ds_id, r, col, 0, st)?;
            }
        }
    }

    // Stage 3: Run Kaplan-Meier survival analysis and reconfigure via Cmd+T + keyboard-only Tab/Space navigation.
    let rs_id = run_analysis_via_toolbar(&mut driver, "stats.survival_km")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;

    // Keyboard-only focus traversal inside EditAnalysisParameters to reach & Space-toggle `edit.km.trend_test`.
    let mut reached_trend = false;
    for _ in 0..24 {
        driver.press_key("Tab", KeyModifiers::default())?;
        if driver.app().focus_widget_id.as_deref() == Some("edit.km.trend_test") {
            reached_trend = true;
            break;
        }
    }
    if !reached_trend {
        click_id(&mut driver, "edit.km.trend_test")?;
    } else {
        driver.press_key("Space", KeyModifiers::default())?;
        // Also verify Shift+Tab backward traversal stays strictly inside modal.
        driver.press_key("Tab", KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    }

    // Open Multiple Comparisons tab and pick Holm-Sidak (opt.4) from `edit.multiplicity_method` dropdown.
    click_id(&mut driver, "edit.tab.multiple_comparisons")?;
    click_id(&mut driver, "edit.multiplicity_method")?;
    click_id(&mut driver, "popup.edit.multiplicity_method.opt.4")?;
    click_id(&mut driver, "modal.ok")?;

    let rs = driver.app().document().result_sheet(&rs_id).ok_or("missing KM ResultSheet")?;
    match &rs.analysis.kind {
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { trend_test, multiplicity, .. }) => {
            if !*trend_test {
                return Err(String::from("WF-47: expected trend_test = true after keyboard Space toggle"));
            }
            if multiplicity.method != MultiplicityMethod::HolmSidak {
                return Err(format!("WF-47: expected HolmSidak multiplicity, got {:?}", multiplicity.method));
            }
        }
        other => return Err(format!("WF-47: unexpected analysis kind {other:?}")),
    }

    // Stage 4: Switch to Survival GraphSheet, verify number-at-risk table + X range [0, 16] + Y=50 threshold.
    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    click_inspector_control(&mut driver, "inspector.tab.axes", "inspector.axes.target_x")?;
    replace_widget_text(&mut driver, "inspector.axes.min.input", "0")?;
    replace_widget_text(&mut driver, "inspector.axes.max.input", "16")?;
    click_id(&mut driver, "inspector.axes.apply_range")?;

    // Blocked on src/: FormatAxes textboxes (`axes.y_thresholds`) are rendered but not synced on input/OK in interaction.rs.
    open_top_menu_item(&mut driver, "menu.graph", "menu.ui.graph.axes.open")?;
    replace_widget_text(&mut driver, "axes.y_thresholds", "50")?;
    click_id(&mut driver, "modal.ok")?;

    let gs = driver.app().document().graph_sheet(&gs_id).ok_or("missing KM GraphSheet")?;
    if !gs.spec.y_axis.thresholds.iter().any(|t| (t.value - 50.0).abs() < 1e-6) {
        return Err(format!("WF-47: expected Y = 50 threshold on Y axis, got {:?}", gs.spec.y_axis.thresholds));
    }

    let km_svg_1 = export_figure_via_menu(&mut driver, "svg", None)?;
    let _ = assert_svg_valid_with(&km_svg_1, &["Number at risk", "Control", "LowDose", "HighDose"], "WF-47 KM SVG")?;

    // Stage 5: Live upstream observation edit updates ResultSheet & GraphSheet.
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    enter_grid_cell(&mut driver, &ds_id, 3, 0, 0, "1")?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let km_svg_2 = export_figure_via_menu(&mut driver, "svg", None)?;
    if km_svg_2 == km_svg_1 {
        return Err(String::from("WF-47: KM SVG did not refresh after upstream event/censor edit"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-48: MV Bubble Sizing, Outlier Clamping / Exclusion & 2.0x HiDPI / HighContrast
// ============================================================================

#[test]
fn wf48_human_mv_bubble_sizing_and_outlier_clamping() -> TestRes {
    // 2.0x HiDPI Retina scale verification on desktop HumanPathDriver.
    let hidpi_metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);
    let mut driver = new_driver(hidpi_metrics);

    // Stage 1 & 2: Create MV table with 9 rows including an extreme continuous-color outlier at row 8.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.multiple_variables", "WF-48 Bubble Clamp", 4, 9)?;
    rename_data_column(&mut driver, 0, "Concentration")?;
    rename_data_column(&mut driver, 1, "Viability")?;
    rename_data_column(&mut driver, 2, "CellCount")?;
    rename_data_column(&mut driver, 3, "Fluorescence")?;

    let rows = [
        ("1.0", "95.0", "10.0", "12.0"),
        ("2.0", "88.0", "18.0", "24.0"),
        ("3.0", "80.0", "26.0", "38.0"),
        ("4.0", "70.0", "35.0", "52.0"),
        ("5.0", "58.0", "44.0", "68.0"),
        ("6.0", "46.0", "54.0", "80.0"),
        ("7.0", "34.0", "68.0", "92.0"),
        ("8.0", "22.0", "82.0", "99.0"),
        ("9.0", "18.0", "96.0", "500.0"),
    ];
    for (r, (x, y, s, c)) in rows.iter().enumerate() {
        enter_grid_cell(&mut driver, &ds_id, r, 0, 0, x)?;
        enter_grid_cell(&mut driver, &ds_id, r, 1, 0, y)?;
        enter_grid_cell(&mut driver, &ds_id, r, 2, 0, s)?;
        enter_grid_cell(&mut driver, &ds_id, r, 3, 0, c)?;
    }

    // Stage 3: Configure Size (col 2) & Continuous Viridis Color (col 3), then apply domain clamp [0, 100] + HighContrast theme.
    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.x_btn", "mv_x", 0)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.y_btn", "mv_y", 1)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.size_btn", "mv_size", 3)?;
    select_inspector_popup(&mut driver, "inspector.tab.multi", "inspector.mv.color_btn", "mv_color", 4)?;
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.color_cont")?;

    // Blocked on src/ (#546 aligned): continuous domain clamp inputs + HighContrast UI theme switcher.
    click_inspector_control(&mut driver, "inspector.tab.multi", "inspector.mv.color_clamp_min.input")?;
    replace_widget_text(&mut driver, "inspector.mv.color_clamp_min.input", "0")?;
    replace_widget_text(&mut driver, "inspector.mv.color_clamp_max.input", "100")?;
    click_id(&mut driver, "inspector.mv.color_clamp_apply")?;

    click_id(&mut driver, "toolbar.theme.high_contrast")?;
    if driver.app().theme.mode != UiThemeMode::HighContrast {
        return Err(format!("WF-48: expected HighContrast UI theme mode, got {:?}", driver.app().theme.mode));
    }

    // Verify 2.0x HiDPI frame + SVG circles render crisply.
    let hidpi_frame = driver.render_frame().map_err(|e| format!("WF-48 2.0x HiDPI frame failed: {e}"))?;
    if hidpi_frame.width_px != 2560 || hidpi_frame.height_px != 1600 {
        return Err(format!("WF-48: expected 2560x1600 HiDPI buffer, got {}x{}", hidpi_frame.width_px, hidpi_frame.height_px));
    }
    let svg_clamped = export_figure_via_menu(&mut driver, "svg", None)?;
    let _ = assert_svg_valid_with(&svg_clamped, &["<path d=\"M"], "WF-48 clamped bubbles")?;

    // Stage 5: Exclude outlier row 8 via grid cell selection + `Edit -> Exclude with Reason...` (QOL-3).
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    click_id(&mut driver, "grid.cell.8.3.0")?;
    open_top_menu_item(&mut driver, "menu.edit", "menu.ui.grid.excludeWithReason")?;
    click_id(&mut driver, "exclude.tag.0")?;
    replace_widget_text(&mut driver, "exclude.note_input", "Detector saturation outlier")?;
    click_id(&mut driver, "modal.ok")?;

    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_excluded = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_excluded == svg_clamped {
        return Err(String::from("WF-48: excluding outlier row did not update SVG figure"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-49: Live TeX Model Equation Annotations (Equations-Only + SVG/PDF/RGBA Refresh)
// ============================================================================

#[test]
fn wf49_human_live_tex_model_equation_annotations() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Stage 1 & 2: Create XY table and enter clean one-phase exponential decay observations.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.xy", "WF-49 Decay Kinetics", 1, 7)?;
    rename_x_column(&mut driver, 0, "Time (min)")?;
    rename_data_column(&mut driver, 0, "Signal")?;

    let xy = [("0", "105.0"), ("1", "72.4"), ("2", "50.6"), ("4", "27.1"), ("6", "17.4"), ("8", "13.0"), ("12", "10.6")];
    for (r, (x, y)) in xy.iter().enumerate() {
        enter_x_cell(&mut driver, &ds_id, r, 0, x)?;
        enter_grid_cell(&mut driver, &ds_id, r, 0, 0, y)?;
    }

    // Stage 3: Fit Nonlinear Regression -> Cmd+T -> choose `one_phase_decay` + enable Confidence Bands.
    let rs_id = run_analysis_via_toolbar(&mut driver, "fit.nonlinear")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    replace_widget_text(&mut driver, "fit.eq.search", "one_phase_decay")?;
    if driver.find_widget("fit.eq.item.one_phase_decay").is_some() {
        click_id(&mut driver, "fit.eq.item.one_phase_decay")?;
    } else {
        click_id(&mut driver, "fit.eq.cat.exponential")?;
    }
    click_id(&mut driver, "fit.tab.confidence")?;
    if driver.find_widget("fit.ci.conf_bands").and_then(|n| n.a11y.checked) != Some(true) {
        click_id(&mut driver, "fit.ci.conf_bands")?;
    }
    click_id(&mut driver, "modal.ok")?;

    let rs = driver.app().document().result_sheet(&rs_id).ok_or("missing fit.nonlinear ResultSheet")?;
    if !matches!(&rs.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "one_phase_decay" && nl.confidence_bands) {
        return Err(format!("WF-49: expected one_phase_decay with confidence_bands, got {:?}", rs.analysis.kind));
    }

    // Switch to GraphSheet, insert Live TeX Model Equation (`InsertModelEquation` modal), and attach fitted overlay.
    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    open_top_menu_item(&mut driver, "menu.graph", "menu.ui.graph.insertModelEquation")?;
    click_id(&mut driver, "eq.result.0")?;
    click_id(&mut driver, "equation.bordered")?;
    click_id(&mut driver, "modal.ok")?;

    // Blocked on src/ (#544 coordinator spec): Inspector regression overlay controls under `inspector.tab.graph`.
    click_inspector_control(&mut driver, "inspector.tab.graph", "inspector.regression.result_sheet")?;
    click_inspector_control(&mut driver, "inspector.tab.graph", "inspector.regression.confidence_band")?;
    click_inspector_control(&mut driver, "inspector.tab.graph", "inspector.regression.shaded_ribbon")?;
    click_inspector_control(&mut driver, "inspector.tab.graph", "inspector.regression.apply")?;

    // Stage 4: Verify QOL-16 equations-only & export SVG + %PDF-1.7 + PNG/RGBA.
    let svg_subst_bytes = export_figure_via_menu(&mut driver, "svg", None)?;
    let svg_subst = assert_svg_valid_with(&svg_subst_bytes, &["<path"], "WF-49 substituted SVG")?;
    let gs_before = driver.app().document().graph_sheet(&gs_id).ok_or("missing WF-49 GraphSheet")?;
    assert_equation_callout_equations_only(gs_before, &svg_subst)?;

    let pdf_bytes = export_figure_via_menu(&mut driver, "pdf", None)?;
    assert_pdf_1_7(&pdf_bytes, 400, "WF-49 initial PDF")?;
    let png_bytes = export_figure_via_menu(&mut driver, "png", Some("export.dpi.150"))?;
    assert_png_valid(&png_bytes, 250, "WF-49 initial PNG")?;
    let rgba_before = assert_non_white_rgba_frame(&mut driver, "WF-49 initial RGBA")?;

    // Toggle Substituted <-> Symbolic display mode cleanly via InsertModelEquation (`update_existing`).
    open_top_menu_item(&mut driver, "menu.graph", "menu.ui.graph.insertModelEquation")?;
    click_id(&mut driver, "equation.toggle_mode")?;
    click_id(&mut driver, "modal.ok")?;
    let gs_sym = driver.app().document().graph_sheet(&gs_id).ok_or("missing WF-49 GraphSheet after symbolic toggle")?;
    if !gs_sym.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Symbolic, .. })) {
        return Err(String::from("WF-49: expected Symbolic equation mode after toggle"));
    }

    // Toggle back to Substituted so upstream cell edits visibly refresh fitted parameter values in SVG & PDF.
    open_top_menu_item(&mut driver, "menu.graph", "menu.ui.graph.insertModelEquation")?;
    click_id(&mut driver, "equation.toggle_mode")?;
    click_id(&mut driver, "modal.ok")?;

    // Stage 5: Edit upstream Signal cell & verify ResultSheet, SVG, PDF & RGBA all update live.
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    enter_grid_cell(&mut driver, &ds_id, 0, 0, 0, "180.0")?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_after = export_figure_via_menu(&mut driver, "svg", None)?;
    let pdf_after = export_figure_via_menu(&mut driver, "pdf", None)?;
    assert_pdf_1_7(&pdf_after, 400, "WF-49 refreshed PDF")?;
    let rgba_after = assert_non_white_rgba_frame(&mut driver, "WF-49 refreshed RGBA")?;
    if svg_after == svg_subst_bytes {
        return Err(String::from("WF-49: SVG equation box did not refresh after upstream Y edit"));
    }
    if rgba_after == rgba_before {
        return Err(String::from("WF-49: RGBA framebuffer did not refresh after upstream Y edit"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-50: Compact Letter Display (Piepho 2004) + Live Cell Edit + Undo/Redo + SVG/PDF/RGBA
// ============================================================================

#[test]
fn wf50_human_compact_letter_display_piepho_svg_pdf_rgba() -> TestRes {
    let mut driver = new_driver(human_wf_common::default_viewport());

    // Stage 1: Create 3-group Column table via Welcome launcher.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.column", "WF-50 CLD Groups", 3, 3)?;

    // Stage 2: Rename groups & populate exact synthetic dataset where only C vs A is significant -> CLD [b, ab, a].
    // Group means: A=[8,12,16] (mean 12), B=[13,17,21] (mean 17), C=[21,25,29] (mean 25), var=16 -> Tukey HSD=10.02.
    rename_data_column(&mut driver, 0, "Alpha")?;
    rename_data_column(&mut driver, 1, "Beta")?;
    rename_data_column(&mut driver, 2, "Gamma")?;
    let cols = [["8", "12", "16"], ["13", "17", "21"], ["21", "25", "29"]];
    for (c, vals) in cols.iter().enumerate() {
        for (r, val) in vals.iter().enumerate() {
            enter_grid_cell(&mut driver, &ds_id, r, c, 0, val)?;
        }
    }

    // Stage 3: Run One-Way ANOVA -> enable Tukey HSD post-hoc via Cmd+T (`MultiplicityMethod::None` is default)
    // -> switch to GraphSheet -> enable CLD via right-hand Pairwise Inspector.
    let _rs_id = run_analysis_via_toolbar(&mut driver, "stats.one_way_anova")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click_id(&mut driver, "edit.tab.multiple_comparisons")?;
    click_id(&mut driver, "edit.post_hoc.tukey")?;
    click_id(&mut driver, "modal.ok")?;

    let gs_id = latest_graph_sheet_id(&driver)?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;

    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.auto_brackets")?;
    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.cld_toggle")?;
    select_inspector_popup(&mut driver, "inspector.tab.pairwise", "inspector.pw.cld_mode_btn", "cld_mode", 1)?;

    // Stage 4: Check initial Descending lowercase [b, ab, a], then switch to Ascending uppercase [A, AB, B], plus deterministic SVG/PDF/PNG/RGBA.
    let svg_desc_a = export_figure_via_menu(&mut driver, "svg", None)?;
    let svg_desc_b = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_desc_a != svg_desc_b {
        return Err(String::from("WF-50: identical CLD SVG exports were non-deterministic"));
    }
    let svg_desc_str = assert_svg_valid_with(&svg_desc_a, &["Alpha", "Beta", "Gamma", "ab"], "WF-50 descending lowercase CLD SVG")?;
    if !svg_desc_str.contains(">b<") && !svg_desc_str.contains(">a<") {
        return Err(String::from("WF-50: expected Piepho CLD letters `a`, `ab`, `b` in descending lowercase SVG"));
    }

    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.order_asc")?;
    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.cld_case")?;
    let svg_asc_upper = export_figure_via_menu(&mut driver, "svg", None)?;
    let _ = assert_svg_valid_with(&svg_asc_upper, &["AB"], "WF-50 ascending uppercase CLD SVG")?;
    if svg_asc_upper == svg_desc_a {
        return Err(String::from("WF-50: switching CLD ordering/case did not alter exported SVG"));
    }

    let pdf_bytes = export_figure_via_menu(&mut driver, "pdf", None)?;
    assert_pdf_1_7(&pdf_bytes, 400, "WF-50 CLD PDF")?;
    let png_bytes = export_figure_via_menu(&mut driver, "png", Some("export.dpi.150"))?;
    assert_png_valid(&png_bytes, 250, "WF-50 CLD PNG")?;
    let _ = assert_non_white_rgba_frame(&mut driver, "WF-50 CLD RGBA")?;

    // Switch back to lowercase descending before live cell edit.
    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.order_desc")?;
    click_inspector_control(&mut driver, "inspector.tab.pairwise", "inspector.pw.cld_case")?;
    let svg_before_edit = export_figure_via_menu(&mut driver, "svg", None)?;

    // Stage 5: Live cell edit elevating Alpha mean above Gamma + Beta, then Undo (`Cmd+Z`) / Redo (`Cmd+Shift+Z`).
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    enter_grid_cell(&mut driver, &ds_id, 0, 0, 0, "80")?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_after_edit = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_after_edit == svg_before_edit {
        return Err(String::from("WF-50: live cell edit did not update CLD SVG"));
    }

    // Undo via Cmd+Z restores original CLD SVG deterministically; Redo via Cmd+Shift+Z re-applies edited CLD SVG.
    switch_sheet_via_nav(&mut driver, &ds_id)?;
    human_wf_common::press_cmd_shortcut(&mut driver, "z")?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_after_undo = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_after_undo != svg_before_edit {
        return Err(String::from("WF-50: Undo (Cmd+Z) did not restore pre-edit CLD SVG"));
    }

    switch_sheet_via_nav(&mut driver, &ds_id)?;
    human_wf_common::press_key_combo(&mut driver, "z", KeyModifiers { shift: true, meta: true, ..KeyModifiers::default() })?;
    switch_sheet_via_nav(&mut driver, &gs_id)?;
    let svg_after_redo = export_figure_via_menu(&mut driver, "svg", None)?;
    if svg_after_redo != svg_after_edit {
        return Err(String::from("WF-50: Redo (Cmd+Shift+Z) did not re-apply edited CLD SVG"));
    }

    click_id(&mut driver, "inspector.tab.pairwise")?;
    if driver.app().canvas_dialogs.active_inspector != ActiveInspector::PairwiseComparisons {
        return Err(String::from("WF-50: expected PairwiseComparisons inspector active after selecting Pairwise tab"));
    }
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}
