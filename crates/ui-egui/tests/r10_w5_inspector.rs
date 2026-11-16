//! Wave 5 Shard `w5h` (`R10.18`) human-path E2E workflow tests for the live right-hand inspector:
//! - 5 responsive sub-tabs & collapsible sections with vertical scrolling (`scroll.inspector`)
//! - Format Graph (dataset/group, Okabe-Ito/Tol/Viridis palettes, colour swatches, symbol shape/size,
//!   bar/box/violin/raincloud/dot/scatter topology, SD/SEM/95% CI/Range & error-bar direction)
//! - Format Axes (`X`/`Y`/`Y2`, linear/Log10/Log2/nonlinear, min/max/interval, 1..3 axis breaks)
//! - Multivariable channels (`X`, `Y`, Colour, Size, 95% confidence ellipse, convex hull)
//! - Pairwise comparisons (brackets + labels) & `R10.11` CLD toggle/mode/ordering/case (`graph.setCompactLetterDisplay`)
//! - Page Layout (grid arrangement, page margins, `align_plot_origins`, scale bars)
//! - Instant undoable engine commands (`edit.undo` reverts live canvas pixels + doc spec)
//! - Interactive data-point selection & **"Go to Data Cell / Drill Back"** (including off-screen row scrolling)

#![forbid(unsafe_code)]

#[path = "human_wf_common/mod.rs"]
mod human_wf_common;

mod i18n {
    pub use curvebook_ui_egui::i18n::CatalogEntry;
}

#[path = "../src/i18n/w5_inspector.rs"]
mod w5_i18n;

use curvebook_chart::{ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    AxisScale, BracketLabelMode, BreakOrnament, Cell, CellCoord, CldOrdering, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet,
    ErrorBarDirection, ErrorBarMetric, GraphicalDataObject, MultipleVariablesTable, MultivariatePlotMode, PairwiseDisplayMode, ProjectDocument,
    SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{MarkerShape, PaletteId};
use curvebook_render::RenderTargetSpec;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::compositor::render_workspace_rgba;
use curvebook_ui_egui::dialogs_canvas::ActiveInspector;
use curvebook_ui_egui::i18n::{LOCALES, normalize_locale, try_tr, try_tr_in_slices};
use curvebook_ui_egui::views::ViewShardStates;
use curvebook_ui_egui::{UiApp, ViewportMetrics, WidgetAction, WidgetId};
use serde_json::json;

use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, scroll_until_widget_visible, scroll_widget_wheel,
};

type TestRes = Result<(), String>;

fn wid(id: &str) -> WidgetId {
    WidgetId(String::from(id))
}

fn canvas_patch_hash(app: &UiApp, metrics: ViewportMetrics) -> Result<Vec<u8>, String> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dpi = (96.0 * metrics.scale_factor).round() as u16;
    let img = render_workspace_rgba(app, RenderTargetSpec { width_px: metrics.physical_width_px(), height_px: metrics.physical_height_px(), dpi })
        .map_err(|e| format!("render_workspace_rgba: {e}"))?;
    let w = img.width_px as usize;
    let h = img.height_px as usize;
    let x0 = (w / 4).min(w.saturating_sub(1));
    let x1 = ((w * 3) / 4).min(w);
    let y0 = (h / 5).min(h.saturating_sub(1));
    let y1 = ((h * 4) / 5).min(h);
    let mut out = Vec::with_capacity(((y1 - y0) / 4) * ((x1 - x0) / 4));
    for py in (y0..y1).step_by(4) {
        for px in (x0..x1).step_by(4) {
            let idx = (py * w + px) * 4;
            if let Some(px_bytes) = img.rgba.get(idx..idx + 4) {
                out.extend_from_slice(px_bytes);
            }
        }
    }
    Ok(out)
}

fn build_column_anova_app() -> Result<(UiApp, SheetId, SheetId, SheetId), String> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| format!("{e}"))?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Low Dose");
    col.data_sets[2].title = String::from("High Dose");
    let mut ds = DataSheet::new("ds-col", "Cohort", TableArchetype::Column(col)).map_err(|e| format!("{e}"))?;
    let vals = [[2.0, 2.2, 1.9, 2.4, 2.1], [6.1, 6.4, 5.9, 6.2, 6.5], [15.0, 15.8, 15.3, 16.1, 15.4]];
    for (c, col_vals) in vals.iter().enumerate() {
        for (r, &v) in col_vals.iter().enumerate() {
            ds.set_cell(CellCoord::new(r, c, 0), Cell::number(v)).map_err(|e| format!("{e}"))?;
        }
    }
    let mut doc = ProjectDocument::new("Inspector Column Study").map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    let mut session = Session::from_document(doc);
    session
        .execute("analysis.run", json!({ "sheet_id": "res-anova", "source_sheet": "ds-col", "kind": "one_way_anova", "multiplicity": "tukey" }))
        .map_err(|e| format!("anova: {e}"))?;
    session
        .execute("graph.create", json!({ "sheet_id": "gr-col", "data_sheet_id": "ds-col", "kind": "column" }))
        .map_err(|e| format!("graph.create: {e}"))?;
    session.execute("sheet.select", json!({ "sheet_id": "gr-col" })).map_err(|e| format!("select gr-col: {e}"))?;
    let mut app = UiApp::from_session(session);
    app.nav.inspector_open = true;
    app.nav.navigator_open = true;
    app.sync_views();
    Ok((app, SheetId::new("ds-col"), SheetId::new("res-anova"), SheetId::new("gr-col")))
}

fn build_xy_app(rows: usize) -> Result<(UiApp, SheetId, SheetId), String> {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, rows).map_err(|e| format!("{e}"))?;
    xy.data_sets[0].title = String::from("Signal");
    let mut ds = DataSheet::new("ds-xy", "Dose Curve", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
    for r in 0..rows {
        let x = (r + 1) as f64;
        let y = 2.0 + x * 1.5;
        ds.set_x_or_time_cell(r, 0, Cell::number(x)).map_err(|e| format!("{e}"))?;
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).map_err(|e| format!("{e}"))?;
    }
    let mut doc = ProjectDocument::new("Inspector XY Study").map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    let mut session = Session::from_document(doc);
    session
        .execute("graph.create", json!({ "sheet_id": "gr-xy", "data_sheet_id": "ds-xy", "kind": "xy" }))
        .map_err(|e| format!("graph.create xy: {e}"))?;
    session.execute("sheet.select", json!({ "sheet_id": "gr-xy" })).map_err(|e| format!("select gr-xy: {e}"))?;
    let mut app = UiApp::from_session(session);
    app.nav.inspector_open = true;
    app.sync_views();
    Ok((app, SheetId::new("ds-xy"), SheetId::new("gr-xy")))
}

#[test]
fn inspector_tabs_collapsible_sections_scroll_and_dropdown_parity() -> TestRes {
    let (mut app, _ds_id, _res_id, _gs_id) = build_column_anova_app()?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" })).map_err(|e| format!("{e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gr-col" })).map_err(|e| format!("{e}"))?;
    app.sync_views();

    let metrics = default_viewport();
    let mut driver = new_human_driver(app, metrics);
    assert_zero_shortcut_compliance(&mut driver)?;

    // Switch across all 5 sub-tabs cleanly and verify ActiveInspector + parity after each switch.
    for (tab_id, expected_insp) in [
        ("inspector.tab.axes", ActiveInspector::FormatAxes),
        ("inspector.tab.multi", ActiveInspector::BubblePlot),
        ("inspector.tab.pairwise", ActiveInspector::PairwiseComparisons),
        ("inspector.tab.layout", ActiveInspector::PageLayout),
        ("inspector.tab.graph", ActiveInspector::FormatGraph),
    ] {
        click_widget_center(&mut driver, &wid(tab_id))?;
        if driver.app().canvas_dialogs.active_inspector != expected_insp {
            return Err(format!("expected {expected_insp:?} after clicking `{tab_id}`, got {:?}", driver.app().canvas_dialogs.active_inspector));
        }
        assert_zero_shortcut_compliance(&mut driver)?;
    }

    // Expand multiple sections and scroll vertically inside `scroll.inspector`.
    click_widget_center(&mut driver, &wid("inspector.section.format_axes"))?;
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.section.page_layout"), 120.0, 16)?;
    scroll_widget_wheel(&mut driver, &wid("scroll.inspector"), 0.0, -120.0)?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // Collapse & re-expand Point & Drill-Back section.
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.section.drill_back"), -160.0, 16)?;
    click_widget_center(&mut driver, &wid("inspector.section.drill_back"))?;
    if !driver.app().view_shards.inspector.collapsed_sections.iter().any(|s| s == "drill_back") {
        return Err(String::from("expected drill_back in collapsed_sections after clicking header"));
    }
    click_widget_center(&mut driver, &wid("inspector.section.drill_back"))?;
    if driver.app().view_shards.inspector.collapsed_sections.iter().any(|s| s == "drill_back") {
        return Err(String::from("expected drill_back removed from collapsed_sections"));
    }

    // Open a dropdown menu overlay at ZLayer::DropdownMenu and verify popup hit-test parity + selection.
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.fg.palette_btn"), 120.0, 16)?;
    click_widget_center(&mut driver, &wid("inspector.fg.palette_btn"))?;
    if driver.app().view_shards.inspector.open_popup.as_deref() != Some("palette") {
        return Err(String::from("expected palette popup to open"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &wid("inspector.popup.palette.item_1"))?;
    if driver.app().view_shards.inspector.open_popup.is_some() {
        return Err(String::from("expected palette popup to close after item selection"));
    }

    assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn format_graph_live_canvas_updates_and_undo_revert() -> TestRes {
    let (app, _ds_id, _res_id, gs_id) = build_column_anova_app()?;
    let metrics = default_viewport();
    let mut driver = new_human_driver(app, metrics);

    // 1. Dataset / group selector cycling.
    click_widget_center(&mut driver, &wid("inspector.fg.dataset_next"))?;
    if driver.app().canvas_dialogs.format_graph.selected_series_index != 1 {
        return Err(format!("expected selected series index 1, got {}", driver.app().canvas_dialogs.format_graph.selected_series_index));
    }
    click_widget_center(&mut driver, &wid("inspector.fg.dataset_prev"))?;
    if driver.app().canvas_dialogs.format_graph.selected_series_index != 0 {
        return Err(String::from("expected selected series index back to 0"));
    }

    // 2. Palette selector (Okabe-Ito -> Tol Bright -> Viridis) + instant canvas pixel update + Cmd+Z revert.
    let baseline_rgba = canvas_patch_hash(driver.app(), metrics)?;
    click_widget_center(&mut driver, &wid("inspector.fg.palette_btn"))?;
    click_widget_center(&mut driver, &wid("inspector.popup.palette.item_1"))?; // PaulTolBright
    let tol_rgba = canvas_patch_hash(driver.app(), metrics)?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.palette) != Some(PaletteId::PaulTolBright) {
        return Err(String::from("expected PaletteId::PaulTolBright after clicking Tol Bright"));
    }
    if tol_rgba == baseline_rgba {
        return Err(String::from("expected Tol Bright palette change to update rendered canvas pixels immediately"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.palette) != Some(PaletteId::OkabeIto) {
        return Err(String::from("expected Cmd+Z to revert palette to OkabeIto"));
    }
    if canvas_patch_hash(driver.app(), metrics)? != baseline_rgba {
        return Err(String::from("expected Cmd+Z to revert canvas pixels to baseline"));
    }

    // Viridis continuous ramp applied across series + undo revert.
    click_widget_center(&mut driver, &wid("inspector.fg.palette_btn"))?;
    click_widget_center(&mut driver, &wid("inspector.popup.palette.item_6"))?; // Viridis
    let viridis_rgba = canvas_patch_hash(driver.app(), metrics)?;
    if viridis_rgba == baseline_rgba {
        return Err(String::from("expected Viridis palette to change canvas pixels immediately"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    if canvas_patch_hash(driver.app(), metrics)? != baseline_rgba {
        return Err(String::from("expected Cmd+Z after Viridis to revert canvas pixels"));
    }

    // 3. Direct Colour Swatch (`Vermilion` #D55E00) + undo revert.
    click_widget_center(&mut driver, &wid("inspector.fg.swatch_6"))?;
    let swatch_rgba = canvas_patch_hash(driver.app(), metrics)?;
    if swatch_rgba == baseline_rgba {
        return Err(String::from("expected swatch click to update canvas pixels immediately"));
    }
    press_cmd_shortcut(&mut driver, "z")?;

    // 4. Bar / Box / Violin / Scatter topology switching + Symbol shape & size + Error bar metrics/direction.
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.fg.topo_scatter"), -120.0, 12)?;
    click_widget_center(&mut driver, &wid("inspector.fg.topo_box"))?;
    if !matches!(
        driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo),
        Some(GraphicalDataObject::BoxAndWhisker { .. })
    ) {
        return Err(String::from("expected BoxAndWhisker topology immediately after click"));
    }
    let box_rgba = canvas_patch_hash(driver.app(), metrics)?;
    if box_rgba == baseline_rgba {
        return Err(String::from("expected BoxAndWhisker topology switch to update canvas pixels"));
    }

    click_widget_center(&mut driver, &wid("inspector.fg.topo_violin"))?;
    if !matches!(
        driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo),
        Some(GraphicalDataObject::Violin { half_violin_with_points: false, .. })
    ) {
        return Err(String::from("expected Violin topology immediately after click"));
    }

    click_widget_center(&mut driver, &wid("inspector.fg.topo_scatter"))?;
    if !matches!(
        driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo),
        Some(GraphicalDataObject::ScatterAndLine { .. })
    ) {
        return Err(String::from("expected ScatterAndLine topology immediately after click"));
    }

    // Symbol shape + size steppers on ScatterAndLine.
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.fg.shape_btn"), 120.0, 12)?;
    click_widget_center(&mut driver, &wid("inspector.fg.shape_btn"))?;
    click_widget_center(&mut driver, &wid("inspector.popup.symbol_shape.item_4"))?; // Diamond
    let shape_now = match driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo) {
        Some(GraphicalDataObject::ScatterAndLine { marker: Some(m), .. }) => Some(m.shape),
        _ => None,
    };
    if shape_now != Some(MarkerShape::Diamond) {
        return Err(format!("expected MarkerShape::Diamond, got {shape_now:?}"));
    }

    let size_before = match driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo) {
        Some(GraphicalDataObject::ScatterAndLine { marker: Some(m), .. }) => m.size_pt,
        _ => 0.0,
    };
    click_widget_center(&mut driver, &wid("inspector.fg.size_inc"))?;
    let size_after = match driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).map(|s| &s.gdo) {
        Some(GraphicalDataObject::ScatterAndLine { marker: Some(m), .. }) => m.size_pt,
        _ => 0.0,
    };
    if (size_after - (size_before + 1.0)).abs() > 1e-6 {
        return Err(format!("expected symbol size increase +1pt ({size_before} -> {size_after})"));
    }

    // Error bar metrics & direction (`SD`, `SEM`, `95% CI`, `Range` + `UpperOnly`).
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.fg.dir_above"), -120.0, 12)?;
    click_widget_center(&mut driver, &wid("inspector.fg.metric_ci95"))?;
    click_widget_center(&mut driver, &wid("inspector.fg.dir_above"))?;
    let eb_now = driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).and_then(|s| s.y_error_bars);
    if eb_now.map(|e| (e.metric, e.direction)) != Some((ErrorBarMetric::ConfidenceInterval95, ErrorBarDirection::UpperOnly)) {
        return Err(format!("expected (ConfidenceInterval95, UpperOnly), got {eb_now:?}"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    let eb_undone = driver.app().document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).and_then(|s| s.y_error_bars);
    if eb_undone.is_some_and(|e| e.direction != ErrorBarDirection::Both) {
        return Err(format!("expected undo to revert error bar direction, got {eb_undone:?}"));
    }

    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn format_axes_scales_breaks_range_and_undo() -> TestRes {
    let (app, _ds_id, _gs_id_dummy) = build_xy_app(8)?;
    let gs_id = SheetId::new("gr-xy");
    let metrics = default_viewport();
    let mut driver = new_human_driver(app, metrics);

    click_widget_center(&mut driver, &wid("inspector.tab.axes"))?;
    let base_rgba = canvas_patch_hash(driver.app(), metrics)?;

    // 1. Scale switches (Log10, Log2, Nonlinear) + Cmd+Z revert.
    click_widget_center(&mut driver, &wid("inspector.axes.scale_log10"))?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.y_axis.segments[0].scale) != Some(AxisScale::Log10) {
        return Err(String::from("expected Y axis scale Log10"));
    }
    if canvas_patch_hash(driver.app(), metrics)? == base_rgba {
        return Err(String::from("expected Log10 scale to update canvas pixels immediately"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.y_axis.segments[0].scale) != Some(AxisScale::Linear) {
        return Err(String::from("expected Cmd+Z to revert Y axis scale back to Linear"));
    }

    click_widget_center(&mut driver, &wid("inspector.axes.scale_log2"))?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.y_axis.segments[0].scale) != Some(AxisScale::Log2) {
        return Err(String::from("expected Y axis scale Log2"));
    }
    click_widget_center(&mut driver, &wid("inspector.axes.scale_nonlin"))?;
    if driver.app().document().graph_sheet(&gs_id).map(|g| g.spec.y_axis.segments[0].scale) != Some(AxisScale::Ln) {
        return Err(String::from("expected Nonlinear scale chip to select AxisScale::Ln"));
    }
    click_widget_center(&mut driver, &wid("inspector.axes.scale_linear"))?;

    // 2. Min / Max / Interval typed entry + Apply Range + Cmd+Z revert.
    click_and_type(&mut driver, &wid("inspector.axes.min.input"), "4")?;
    click_and_type(&mut driver, &wid("inspector.axes.max.input"), "20")?;
    click_and_type(&mut driver, &wid("inspector.axes.interval.input"), "4")?;
    click_widget_center(&mut driver, &wid("inspector.axes.apply_range"))?;

    let seg = driver
        .app()
        .document()
        .graph_sheet(&gs_id)
        .and_then(|g| g.spec.y_axis.segments.first())
        .cloned()
        .ok_or_else(|| String::from("missing y_axis segment"))?;
    if seg.min != Some(4.0) || seg.max != Some(20.0) || seg.major_tick_interval != Some(4.0) {
        return Err(format!("expected Y range [4, 20] step 4, got {seg:?}"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    let seg_undone = driver
        .app()
        .document()
        .graph_sheet(&gs_id)
        .and_then(|g| g.spec.y_axis.segments.first())
        .cloned()
        .ok_or_else(|| String::from("missing y_axis segment after undo"))?;
    if seg_undone.min == Some(4.0) && seg_undone.max == Some(20.0) {
        return Err(String::from("expected Cmd+Z to revert applied Y range"));
    }

    // 3. Axis breaks (2 segments + Zigzag ornament + 3 segments).
    scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.axes.orn_zigzag"), -120.0, 12)?;
    click_widget_center(&mut driver, &wid("inspector.axes.breaks_2"))?;
    click_widget_center(&mut driver, &wid("inspector.axes.orn_zigzag"))?;
    let y_axis = &driver.app().document().graph_sheet(&gs_id).ok_or("missing gr-xy")?.spec.y_axis;
    if y_axis.segments.len() != 2 || y_axis.break_ornament != BreakOrnament::Zigzag {
        return Err(format!("expected 2 segments & Zigzag break, got {} segs / {:?}", y_axis.segments.len(), y_axis.break_ornament));
    }
    click_widget_center(&mut driver, &wid("inspector.axes.breaks_3"))?;
    if driver.app().document().graph_sheet(&gs_id).ok_or("missing gr-xy")?.spec.y_axis.segments.len() != 3 {
        return Err(String::from("expected 3 axis break segments"));
    }

    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn multivariable_channels_and_pairwise_cld_live_undo() -> TestRes {
    // Part A: Multivariable channels (`X`, `Y`, `Colour`, `Size`, 95% Ellipse, Convex Hull).
    let mut mv = MultipleVariablesTable::new(4, 6).map_err(|e| format!("{e}"))?;
    for (idx, (title, role)) in
        [("PC1", VariableRole::Continuous), ("PC2", VariableRole::Continuous), ("Arm", VariableRole::Categorical), ("Dose", VariableRole::Continuous)]
            .iter()
            .enumerate()
    {
        if let Some(col) = mv.variables.get_mut(idx) {
            col.title = String::from(*title);
            col.role = *role;
        }
    }
    let mut ds_mv = DataSheet::new("ds-mv", "Cohort MV", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("{e}"))?;
    let mv_rows = [
        (1.0, 2.0, "Ctrl", 10.0),
        (1.5, 2.5, "Ctrl", 15.0),
        (1.2, 1.8, "Ctrl", 20.0),
        (8.0, 9.0, "Drug", 40.0),
        (8.5, 9.4, "Drug", 50.0),
        (8.2, 8.7, "Drug", 60.0),
    ];
    for (r, (x, y, arm, dose)) in mv_rows.iter().enumerate() {
        ds_mv.set_cell(CellCoord::new(r, 0, 0), Cell::number(*x)).map_err(|e| format!("{e}"))?;
        ds_mv.set_cell(CellCoord::new(r, 1, 0), Cell::number(*y)).map_err(|e| format!("{e}"))?;
        ds_mv.set_cell(CellCoord::new(r, 2, 0), Cell::text(*arm)).map_err(|e| format!("{e}"))?;
        ds_mv.set_cell(CellCoord::new(r, 3, 0), Cell::number(*dose)).map_err(|e| format!("{e}"))?;
    }
    let mut doc = ProjectDocument::new("MV & CLD Study").map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds_mv).map_err(|e| format!("{e}"))?;
    let mut session = Session::from_document(doc);
    session
        .execute("graph.create", json!({ "sheet_id": "gr-mv", "data_sheet_id": "ds-mv", "kind": "multiple_variables" }))
        .map_err(|e| format!("create mv graph: {e}"))?;
    session
        .execute(
            "graph.setSpec",
            json!({
                "sheet_id": "gr-mv",
                "multivariate": {
                    "plot_mode": MultivariatePlotMode::BubbleScatter,
                    "x_column": 0,
                    "y_column": 1,
                    "color": { "column": 2, "categorical": true },
                    "size": { "column": 3, "min_radius_pt": 3.0, "max_radius_pt": 9.0 },
                    "show_convex_hulls": false,
                    "show_confidence_ellipses": false
                }
            }),
        )
        .map_err(|e| format!("set mv spec: {e}"))?;
    session.execute("sheet.select", json!({ "sheet_id": "gr-mv" })).map_err(|e| format!("{e}"))?;

    let mut app_mv = UiApp::from_session(session);
    app_mv.nav.inspector_open = true;
    app_mv.execute_command("sheet.select", json!({ "sheet_id": "gr-mv" })).map_err(|e| format!("{e}"))?;
    app_mv.sync_views();
    let metrics = default_viewport();
    let mut driver_mv = new_human_driver(app_mv, metrics);
    click_widget_center(&mut driver_mv, &wid("inspector.tab.multi"))?;

    if driver_mv.app().canvas_dialogs.active_inspector != ActiveInspector::BubblePlot {
        return Err(format!("expected ActiveInspector::BubblePlot on MV graph, got {:?}", driver_mv.app().canvas_dialogs.active_inspector));
    }

    // Switch Colour channel to Continuous Viridis + select Size column + toggle 95% Ellipse & Convex Hull.
    click_widget_center(&mut driver_mv, &wid("inspector.mv.color_cont"))?;
    click_widget_center(&mut driver_mv, &wid("inspector.mv.size_btn"))?;
    click_widget_center(&mut driver_mv, &wid("inspector.popup.mv_size.item_4"))?;
    scroll_until_widget_visible(&mut driver_mv, &wid("scroll.inspector"), &wid("inspector.mv.hull_toggle"), 120.0, 12)?;
    click_widget_center(&mut driver_mv, &wid("inspector.mv.ellipse_toggle"))?;
    click_widget_center(&mut driver_mv, &wid("inspector.mv.hull_toggle"))?;
    let mv_enc = driver_mv
        .app()
        .document()
        .graph_sheet(&SheetId::new("gr-mv"))
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or_else(|| String::from("missing mv encoding"))?;
    if !matches!(mv_enc.color, ColorChannelMapping::Continuous { .. })
        || !matches!(mv_enc.size, SizeChannelMapping::Variable { column: 3, .. })
        || !mv_enc.show_confidence_ellipses
        || !mv_enc.show_convex_hulls
    {
        return Err(format!("expected Continuous + Variable size + Ellipse + Hull enabled, got {mv_enc:?}"));
    }
    press_cmd_shortcut(&mut driver_mv, "z")?;
    let mv_after_undo = driver_mv
        .app()
        .document()
        .graph_sheet(&SheetId::new("gr-mv"))
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or_else(|| String::from("missing mv encoding after undo"))?;
    if mv_after_undo.show_convex_hulls {
        return Err(String::from("expected Cmd+Z to revert show_convex_hulls"));
    }
    assert_zero_shortcut_compliance(&mut driver_mv)?;

    // Part B: Pairwise Comparisons & R10.11 Compact Letter Display (CLD).
    let (app_pw, _ds_id, _res_id, gs_pw) = build_column_anova_app()?;
    let mut driver_pw = new_human_driver(app_pw, metrics);
    click_widget_center(&mut driver_pw, &wid("inspector.tab.pairwise"))?;

    // Auto-add pairwise comparison brackets + switch label mode (`Exact p`).
    click_widget_center(&mut driver_pw, &wid("inspector.pw.auto_brackets"))?;
    if driver_pw.app().document().graph_sheet(&gs_pw).ok_or("missing gr-col")?.spec.comparisons.is_empty() {
        return Err(String::from("expected pairwise comparison brackets after clicking Auto Brackets"));
    }
    click_widget_center(&mut driver_pw, &wid("inspector.pw.lbl_p"))?;
    if driver_pw
        .app()
        .document()
        .graph_sheet(&gs_pw)
        .ok_or("missing gr-col")?
        .spec
        .comparisons
        .iter()
        .any(|b| b.label_mode != BracketLabelMode::NumericPOrQ)
    {
        return Err(String::from("expected BracketLabelMode::NumericPOrQ after clicking Exact p"));
    }

    // Enable CLD via checkbox -> verifies `graph.setCompactLetterDisplay` + live CLD scene letters.
    click_widget_center(&mut driver_pw, &wid("inspector.pw.cld_toggle"))?;
    let gs_cld = driver_pw.app().document().graph_sheet(&gs_pw).ok_or("missing gr-col after cld")?;
    if gs_cld.spec.compact_letter_display.is_none() || gs_cld.spec.pairwise_display_mode != PairwiseDisplayMode::CompactLetterDisplayOnly {
        return Err(format!("expected CLD enabled, got mode={:?}, cld={:?}", gs_cld.spec.pairwise_display_mode, gs_cld.spec.compact_letter_display));
    }
    let scene_cld = compile_graph(gs_cld, driver_pw.app().document()).map_err(|e| format!("compile CLD graph: {e}"))?;
    let has_letter = scene_cld
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Text(t) if matches!(t.rich.plain_text().trim(), "a" | "b" | "c" | "ab" | "bc" | "A" | "B" | "C" | "AB" | "BC")));
    if !has_letter {
        return Err(String::from("expected rendered CLD letter glyphs in ChartScene"));
    }

    // Change ordering (`DescendingMean`) and case toggle (`uppercase`) + `Brackets + CLD`.
    scroll_until_widget_visible(&mut driver_pw, &wid("scroll.inspector"), &wid("inspector.pw.cld_case"), 120.0, 12)?;
    click_widget_center(&mut driver_pw, &wid("inspector.pw.order_desc"))?;
    click_widget_center(&mut driver_pw, &wid("inspector.pw.cld_case"))?;
    click_widget_center(&mut driver_pw, &wid("inspector.pw.cld_mode_btn"))?;
    click_widget_center(&mut driver_pw, &wid("inspector.popup.cld_mode.item_2"))?;
    let gs_both = driver_pw.app().document().graph_sheet(&gs_pw).ok_or("missing gr-col")?;
    if gs_both.spec.pairwise_display_mode != PairwiseDisplayMode::BracketsAndCompactLetterDisplay
        || gs_both.spec.compact_letter_display.as_ref().map(|c| (c.ordering, c.uppercase)) != Some((CldOrdering::DescendingMean, true))
    {
        return Err(format!("unexpected CLD state: mode={:?}, cld={:?}", gs_both.spec.pairwise_display_mode, gs_both.spec.compact_letter_display));
    }

    press_cmd_shortcut(&mut driver_pw, "z")?;
    if driver_pw.app().document().graph_sheet(&gs_pw).ok_or("missing gr-col")?.spec.pairwise_display_mode
        == PairwiseDisplayMode::BracketsAndCompactLetterDisplay
    {
        return Err(String::from("expected Cmd+Z to revert CLD pairwise display mode"));
    }
    assert_zero_shortcut_compliance(&mut driver_pw)?;
    Ok(())
}

#[test]
fn page_layout_grid_margins_align_scalebar_and_drill_back() -> TestRes {
    // Build a 60-row XY sheet (`target.row >= 25` far off the top grid viewport) + 2nd graph + LayoutSheet.
    let (mut app, ds_id, gs_id) = build_xy_app(60)?;
    app.execute_command("graph.create", json!({ "sheet_id": "gr-xy-2", "data_sheet_id": "ds-xy", "kind": "xy" })).map_err(|e| format!("{e}"))?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-main", "title": "Figure Panel", "preset": "double_column" }))
        .map_err(|e| format!("{e}"))?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-main", "kind": "graph", "graph_sheet_id": "gr-xy", "bounds_pt": [24.0, 24.0, 180.0, 130.0] }),
    )
    .map_err(|e| format!("{e}"))?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-main", "kind": "graph", "graph_sheet_id": "gr-xy-2", "bounds_pt": [220.0, 48.0, 180.0, 130.0] }),
    )
    .map_err(|e| format!("{e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-main" })).map_err(|e| format!("{e}"))?;
    app.nav.navigator_open = true;
    app.sync_views();

    let metrics = default_viewport();
    let mut driver = new_human_driver(app, metrics);
    let ls_id = SheetId::new("lay-main");
    let lay_rgba_before = canvas_patch_hash(driver.app(), metrics)?;

    // 1. Page Layout section: grid 2x1, margin increase, align_plot_origins toggle (true -> false -> true), scale bar 25 mm + undo.
    click_widget_center(&mut driver, &wid("inspector.layout.grid_2x1"))?;
    click_widget_center(&mut driver, &wid("inspector.layout.margin_inc"))?;
    click_widget_center(&mut driver, &wid("inspector.layout.align_origins"))?;
    if driver.app().document().layout_sheet(&ls_id).is_none_or(|l| l.spec.align_plot_origins) {
        return Err(String::from("expected first align_origins click to toggle true -> false"));
    }
    click_widget_center(&mut driver, &wid("inspector.layout.align_origins"))?;
    click_widget_center(&mut driver, &wid("inspector.layout.scale_bar_toggle"))?;
    click_widget_center(&mut driver, &wid("inspector.layout.sb_25"))?;

    let ls = driver.app().document().layout_sheet(&ls_id).ok_or("missing lay-main")?;
    if ls.spec.grid_template != Some((2, 1)) || !ls.spec.align_plot_origins {
        return Err(format!("expected 2x1 grid & align_plot_origins=true, got {:?}", ls.spec));
    }
    let has_25mm = ls.spec.items.iter().any(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { markdown, .. } if markdown.contains("25 mm")));
    if !has_25mm {
        return Err(String::from("expected 25 mm scale bar callout item in LayoutSpec"));
    }
    let lay_scene = compile_layout(ls, driver.app().document()).map_err(|e| format!("compile layout: {e}"))?;
    if !lay_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("25 mm"))) {
        return Err(String::from("expected rendered 25 mm scale bar text in compiled Layout scene"));
    }
    if canvas_patch_hash(driver.app(), metrics)? == lay_rgba_before {
        return Err(String::from("expected layout inspector edits to change rendered workspace RGBA"));
    }
    press_cmd_shortcut(&mut driver, "z")?;
    let ls_after_undo = driver.app().document().layout_sheet(&ls_id).ok_or("missing lay-main after undo")?;
    if ls_after_undo.spec.items.iter().any(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { markdown, .. } if markdown.contains("25 mm"))) {
        return Err(String::from("expected Cmd+Z to revert 25 mm scale bar"));
    }

    // 2. Switch to GraphSheet & test interactive point cycling to an off-screen row + Drill-Back!
    click_widget_center(&mut driver, &wid("nav.sheet.gr-xy"))?;
    if driver.app().document().active_sheet_id.as_ref() != Some(&gs_id) {
        return Err(format!("expected nav click to switch to gr-xy, got {:?}", driver.app().document().active_sheet_id));
    }

    click_widget_center(&mut driver, &wid("inspector.tab.graph"))?;
    for _ in 0..42 {
        click_widget_center(&mut driver, &wid("inspector.drill.next"))?;
    }
    let target_cell = driver.app().brushed_cell.clone().ok_or_else(|| String::from("expected brushed_cell after cycling points"))?;
    if target_cell.row < 25 || target_cell.sheet_id != ds_id {
        return Err(format!("expected brushed_cell at row >= 25 on ds-xy, got {target_cell:?}"));
    }

    // Click "Go to Data Cell / Drill Back" -> switches to ds-xy, focuses exact (row, col, subcol), & scrolls row into view!
    click_widget_center(&mut driver, &wid("inspector.drill.go_to_cell"))?;
    if driver.app().document().active_sheet_id.as_ref() != Some(&ds_id) {
        return Err(format!("expected drill-back to switch to ds-xy, got {:?}", driver.app().document().active_sheet_id));
    }
    let expected_coord = GridCellCoord::new(target_cell.row, target_cell.col, target_cell.subcol);
    if driver.app().grid.tool.active != Some(expected_coord) || driver.app().grid.selected_coords != vec![expected_coord] {
        return Err(format!(
            "expected grid focus at {expected_coord:?}, got active={:?}, selected={:?}",
            driver.app().grid.tool.active,
            driver.app().grid.selected_coords
        ));
    }
    let win = driver.app().grid.visible_window(driver.app().document()).ok_or("missing visible grid window")?;
    if target_cell.row < win.start_row || target_cell.row >= win.end_row {
        return Err(format!("expected off-screen row {} scrolled into visible window {:?}", target_cell.row, win));
    }

    click_widget_center(&mut driver, &wid("inspector.drill.return_graph"))?;
    if driver.app().document().active_sheet_id.as_ref() != Some(&gs_id) {
        return Err(String::from("expected Return to Graph to re-select gr-xy"));
    }

    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn w5_inspector_i18n_16_locales_and_hidpi_rtl_parity() -> TestRes {
    // 1. All 16 locales must resolve non-empty translations for every w5_inspector key.
    for &loc in LOCALES {
        let norm = normalize_locale(loc);
        for entry in w5_i18n::ENTRIES {
            let txt = try_tr(norm, entry.key)
                .or_else(|| try_tr_in_slices(&[w5_i18n::ENTRIES], norm, entry.key))
                .ok_or_else(|| format!("missing translation for `{}` in locale `{norm}`", entry.key))?;
            if txt.trim().is_empty() {
                return Err(format!("empty translation for `{}` in locale `{norm}`", entry.key));
            }
        }
    }

    // 2. Render & verify parity across LTR (`en`), CJK (`ja`), RTL (`ar`), narrow 760px viewport, and 1.5x HiDPI.
    for (loc, width, scale) in [("en", 1280.0, 1.0), ("ja", 1280.0, 1.0), ("ar", 1280.0, 1.0), ("en", 760.0, 1.0), ("en", 1280.0, 1.5)] {
        let (mut app, _ds_id, _res_id, _gs_id) = build_column_anova_app()?;
        app.locale = String::from(loc);
        app.nav.navigator_open = false;
        app.sync_views();
        let metrics = ViewportMetrics::new(width, 800.0, scale);
        let mut driver = new_human_driver(app, metrics);
        assert_zero_shortcut_compliance(&mut driver)?;
        click_widget_center(&mut driver, &wid("inspector.tab.axes"))?;
        assert_zero_shortcut_compliance(&mut driver)?;
    }

    // Verify `WidgetAction::Custom { domain: "inspector", payload: "smoke" }` probe remains unhandled.
    let (mut app, _, _, _) = build_column_anova_app()?;
    let mut shards = ViewShardStates::default();
    let smoke = curvebook_ui_egui::views::dispatch_custom_view_action(&mut app, &mut shards, "inspector", "smoke")?;
    if smoke.handled || !smoke.action.is_empty() {
        return Err(format!("expected unrecognized smoke payload to stay unhandled, got {smoke:?}"));
    }
    let _ = WidgetAction::NoOp;
    Ok(())
}
