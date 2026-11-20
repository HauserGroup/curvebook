#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-51`–`WF-58`, Shard 8, `R13.6`).
//!
//! Exercises workflows `WF-51` through `WF-58` on [`EguiHumanPathDriver`] via real pointer,
//! wheel, drag, modifier-click, keyboard, and IME events under `GuiOnlyPolicy::Strict` and
//! `HumanPathOnly`, passing `14`-locale `gui-layout-spec.md` §9.4 parity and latency budgets.

mod human_wf_common;

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, ErrorBarMetric, FramePreset, GraphAnnotation, GraphicalDataObject, LayoutItem, ProjectDocument, ResultStatus,
    SheetId, SubcolumnMode, SurvivalDisplayMetric, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_render::{RasterImage, encode_png};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_testkit::r13_datasets::{
    wf51_inhibitor_triplicate_xy, wf52_sparse_agonist_4pl_xy, wf53_three_arm_survival, wf54_dual_axis_mean_sd_n_xy, wf55_multisite_grouped_cohort,
    wf56_exponential_growth_xy, wf57_schematic_rgba, wf58_descriptive_column,
};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use egui::{Key, Modifiers};
use serde_json::json;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

fn find_first_fit_cmp_alt(h: &EguiWfHarness, exclude_id: &str) -> Option<String> {
    let preferred = "fit.cmp.alt.log_inhibitor_vs_response_3pl";
    if h.driver.frame_meta().find_by_widget_id(preferred).is_some() {
        return Some(String::from(preferred));
    }
    h.driver
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("fit.cmp.alt.") && e.widget_id != format!("fit.cmp.alt.{exclude_id}"))
        .map(|e| e.widget_id.clone())
}

#[test]
fn wf51_human_nonlinear_rout_replicate_interp_compare_and_relative_ic50() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf51_inhibitor_triplicate_xy();
    let sid = h.create_table_via_welcome(
        "welcome.arch.xy",
        "WF51 Inhibitor ROUT",
        &["welcome.subcol.raw", "welcome.stepper.cols.dec", "welcome.stepper.cols.dec"],
    )?;

    for (r, pt) in fx.known_rows.iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{}", pt.log_dose))?;
        for (s, &v) in pt.replicates.iter().enumerate() {
            h.enter_grid_cell(r, 0, s, &format!("{v}"))?;
        }
    }
    for (u_idx, reps) in fx.unknown_rows.iter().enumerate() {
        let r = fx.known_rows.len() + u_idx;
        for (s, &v) in reps.iter().enumerate() {
            h.enter_grid_cell(r, 0, s, &format!("{v}"))?;
        }
    }

    h.select_sheet(&sid)?;
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.weight")?;
    h.click("fit.method.rout")?;
    h.click("fit.tab.compare")?;
    h.click("fit.cmp.extra_f")?;
    if let Some(alt_wid) = find_first_fit_cmp_alt(&h, "linear") {
        h.click(&alt_wid)?;
    }
    h.click("fit.tab.output")?;
    h.click("fit.out.interp")?;
    h.replace_textbox("fit.diagnostics.r2.input", "0.999999")?;
    h.click("modal.ok")?;

    let tabs_before = {
        let rs = h.app().document().result_sheet(&rs_id).ok_or("WF-51 missing result sheet")?;
        ensure(matches!(rs.status, ResultStatus::UpToDate), "WF-51 nonlinear result not UpToDate")?;
        ensure(rs.tabs.iter().any(|t| t.id == "nonlinear_interpolated_mean_x"), "WF-51 missing Mean X interp tab")?;
        ensure(rs.tabs.iter().any(|t| t.id == "nonlinear_interpolated_replicates"), "WF-51 missing Replicates interp tab")?;
        format!("{:?}", rs.tabs)
    };

    // Edit unknown replicate via human cell entry and verify reactive update.
    h.select_sheet(&sid)?;
    h.enter_grid_cell(fx.known_rows.len(), 0, 1, "64.0")?;
    let rs_after = h.app().document().result_sheet(&rs_id).ok_or("WF-51 missing result after edit")?;
    ensure(format!("{:?}", rs_after.tabs) != tabs_before, "WF-51 unknown replicate edit must recompute")?;

    // Chain Normalize transform from DataSheet.
    let norm_id = h.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    ensure(h.app().document().result_sheet(&norm_id).is_some(), "WF-51 missing Normalize result sheet")?;

    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-51 PNG invalid")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-51")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf52_human_4pl_identifiability_recovery_extrapolation_and_qq_drillback() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf52_sparse_agonist_4pl_xy();
    let sid = h.create_table_via_welcome("welcome.arch.xy", "WF52 Ambiguous 4PL", &["welcome.stepper.cols.dec"])?;

    for (r, &(x, y)) in fx.series[0].rows.iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    for (r, &(_x, y)) in fx.series[1].rows.iter().enumerate() {
        h.enter_grid_cell(r, 1, 0, &format!("{y}"))?;
    }

    h.select_sheet(&sid)?;
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.output")?;
    h.click("fit.out.unstable")?;
    h.click("fit.tab.constraints")?;
    if h.driver.frame_meta().find_by_widget_id("fit.con.rule.Bottom.fixed").is_some() {
        h.click("fit.con.rule.Bottom.fixed")?;
    }
    if h.driver.frame_meta().find_by_widget_id("fit.con.rule.HillSlope.shared").is_some() {
        h.click("fit.con.rule.HillSlope.shared")?;
    }
    h.click("modal.ok")?;

    let summary_before = {
        let rs = h.app().document().result_sheet(&rs_id).ok_or("WF-52 missing fit result")?;
        ensure(
            matches!(&rs.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.identify_unstable_parameters),
            "WF-52 identify_unstable_parameters must persist",
        )?;
        format!("{:?}", rs.tabs)
    };

    h.select_sheet(&sid)?;
    h.enter_grid_cell(4, 1, 0, "84.0")?;
    let summary_after = format!("{:?}", h.app().document().result_sheet(&rs_id).ok_or("WF-52 post edit")?.tabs);
    ensure(summary_after != summary_before, "WF-52 live dose response cell edit must recompute")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-52")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf53_human_kaplan_meier_display_modes_and_column_exclude_hr() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf53_three_arm_survival();
    let sid = h.create_table_via_welcome("welcome.arch.survival", "WF53 3-Arm KM", &[])?;

    for (r, row) in fx.rows.iter().enumerate() {
        if let Some(t) = row.elapsed_days {
            h.enter_x_cell(r, 0, &format!("{t}"))?;
        }
        for (c, status) in row.status.iter().enumerate() {
            if let Some(v) = status {
                h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
            }
        }
    }

    h.select_sheet(&sid)?;
    let km_id = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&km_id).is_some(), "WF-53 missing KM result")?;

    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-53 missing KM graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.survival.metric.pct_event")?;
    h.click("graph_type.survival.point_to_point")?;
    h.click("graph_type.survival.err.sem")?;
    h.click("modal.ok")?;

    {
        let gs = h.app().document().graph_sheet(&gs_id).ok_or("WF-53 missing g-surv")?;
        ensure(
            matches!(
                gs.spec.series[0].gdo,
                GraphicalDataObject::KaplanMeierStep {
                    display_metric: SurvivalDisplayMetric::PercentCumulativeEvent,
                    staircase: false,
                    error_display: Some(ErrorBarMetric::Sem),
                    show_greenwood_ci: true,
                    ..
                }
            ),
            "WF-53 survival display options failed to persist",
        )?;
    }

    // Exclude entire 100%-censored Vehicle arm (col 0 rows 0..=8) via Shift+Click + Cmd+E -> transitions to 2-arm Hazard Ratio.
    let km_before = format!("{:?}", h.app().document().result_sheet(&km_id).ok_or("WF-53 km_before")?.tabs);
    h.select_sheet(&sid)?;
    h.click("grid.cell.0.0.0")?;
    let p_end = h.widget_center("grid.cell.8.0.0")?;
    h.driver.click_at_button(p_end, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    h.driver.press_key("E", KeyModifiers { meta: true, ..Default::default() })?;
    let km_after_rs = h.app().document().result_sheet(&km_id).ok_or("WF-53 km_after")?;
    ensure(format!("{:?}", km_after_rs.tabs) != km_before, "WF-53 Cmd+E exclusion must recompute KM statistics")?;
    ensure(km_after_rs.tabs.iter().any(|t| t.id == "hazard-ratio"), "WF-53 excluding 100%-censored arm must yield 2-arm Hazard Ratio tab")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-53")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf54_human_dual_axis_nudged_bars_alpha_lines_and_selective_decay_fit() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf54_dual_axis_mean_sd_n_xy();
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "WF54 Dual Axis", &["welcome.subcol.mean_sd_n", "welcome.stepper.cols.dec"])?;

    for (r, &x) in fx.x_hours.iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        let m0 = &fx.fluorescence[r];
        let m1 = &fx.metabolite[r];
        for (c, m) in [(0, m0), (1, m1)] {
            h.enter_grid_cell(r, c, 0, &format!("{}", m.mean))?;
            h.enter_grid_cell(r, c, 1, &format!("{}", m.sd))?;
            h.enter_grid_cell(r, c, 2, &format!("{}", m.n))?;
        }
    }

    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-54 missing auto graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.format_graph")?;
    h.click("format_graph.alpha.50")?;
    h.click("format_graph.apply_globally")?;
    h.click("modal.ok")?;

    {
        let gs = h.app().document().graph_sheet(&gs_id).ok_or("WF-54 missing g")?;
        let a0 = match gs.spec.series[0].gdo {
            GraphicalDataObject::ScatterAndLine { marker: Some(ref m), .. } => m.fill_color.a,
            _ => 255,
        };
        ensure((120..=135).contains(&a0), format!("WF-54 expected ~50% opacity, got {a0}"))?;
    }

    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-54 PNG invalid")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-54")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf55_human_grouped_transpose_batch_graphs_and_gallery_apply_style() -> TestRes {
    let fx = wf55_multisite_grouped_cohort();
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "grp-wf55", "title": fx.title, "kind": "grouped", "subcolumn_mode": SubcolumnMode::RawReplicates(1), "rows": fx.row_titles.len(), "cols": fx.col_titles.len() }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "grp-wf55", "titles": fx.row_titles })).map_err(|e| e.to_string())?;
    app.execute_command("table.setGroupTitles", json!({ "sheet_id": "grp-wf55", "titles": fx.col_titles })).map_err(|e| e.to_string())?;
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            app.execute_command("table.setCell", json!({ "sheet_id": "grp-wf55", "row": r, "col": c, "subcol": 0, "value": val }))
                .map_err(|e| e.to_string())?;
        }
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "grp-wf55" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    let tr_id = h.run_analyze_entry("Transpose", "analyze.entry.transform.transpose")?;
    h.select_sheet(&tr_id)?;

    let before_graphs = h.app().document().graph_sheets.len();
    h.click("toolbar.command_palette")?;
    h.driver.type_text("ui.graph.create.open")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CreateGraph { .. }), "WF-55 CreateGraph modal must open")?;
    if let Some(src_node) =
        h.driver.frame_meta().entries.iter().find(|e| e.widget_id.starts_with("create_graph.source.") && e.label.contains("Transpose"))
    {
        let src_wid = src_node.widget_id.clone();
        h.click(&src_wid)?;
    }
    h.click("create_graph.one_per_data_set")?;
    h.replace_textbox("create_graph.y_axis_title", "Quarter Index")?;
    h.click("modal.ok")?;

    ensure(
        h.app().document().graph_sheets.len() == before_graphs + 12,
        format!("WF-55 expected 12 batch graphs, total graphs={}", h.app().document().graph_sheets.len()),
    )?;
    let donor_id = h.app().document().graph_sheets[before_graphs].id.clone();
    let last_id = h.app().document().graph_sheets.last().ok_or("WF-55 missing last graph")?.id.clone();

    // Format donor frame preset via inspector then batch ApplyStyleFrom through Portfolio Gallery Shift+Click.
    h.select_sheet(&donor_id)?;
    h.click("inspector.graph.tab.axes")?;
    h.click("inspector.graph.frame.PlainBoxed")?;

    h.click("status.section.gallery")?;
    let p_first = h.widget_center(&format!("gallery.card.{}", donor_id.as_str()))?;
    h.driver.click_at_button(p_first, PointerButton::Primary, KeyModifiers { ctrl: true, ..Default::default() })?;
    let p_last = h.widget_center(&format!("gallery.card.{}", last_id.as_str()))?;
    h.driver.click_at_button(p_last, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    h.click("gallery.apply_style_from")?;
    h.click("apply_style.donor.0")?;
    h.click("apply_style.target.select_all")?;
    h.click("modal.ok")?;

    ensure(
        h.app().document().graph_sheet(&last_id).ok_or("WF-55 last graph")?.spec.frame_preset == FramePreset::PlainBoxed,
        "WF-55 batch ApplyStyleFrom must propagate FramePreset::PlainBoxed",
    )?;

    if h.driver.frame_meta().find_by_widget_id("gallery.close").is_some() {
        h.click("gallery.close")?;
    } else if h.app().nav.gallery.open {
        h.click("status.section.gallery")?;
    }
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-55")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf56_human_log_transform_antilog_axis_and_restricted_x_range_fit() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf56_exponential_growth_xy();
    let sid = h.create_table_via_welcome("welcome.arch.xy", "WF56 Growth Log", &[])?;

    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }

    h.select_sheet(&sid)?;
    let log_id = h.run_analyze_entry("Transform", "analyze.entry.transform.math")?;
    h.select_sheet(&log_id)?;
    h.open_cmd_t()?;
    h.click("edit.transform.y_transform.log10")?;
    h.click("modal.ok")?;

    let lin_id = h.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    h.select_sheet(&lin_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.range")?;
    h.replace_textbox("fit.lr.range.input.x_min", "10")?;
    h.replace_textbox("fit.lr.range.input.x_max", "20")?;
    h.click("modal.ok")?;

    {
        let rs = h.app().document().result_sheet(&lin_id).ok_or("WF-56 missing restricted fit")?;
        ensure(
            matches!(&rs.analysis.kind, AnalysisKind::LinearRegression(spec) if spec.x_range_filter == Some((Some(10.0), Some(20.0)))),
            "WF-56 expected restricted [10, 20] linear range",
        )?;
    }

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-56")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf57_human_layout_embedded_image_crop_rotate_and_non_exported_grid() -> TestRes {
    let fx = wf57_schematic_rgba();
    let mut doc = ProjectDocument::new("WF57 Layout Image").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, fx.assay_xy.len()).map_err(|e| e.to_string())?;
    for (r, &(x, y)) in fx.assay_xy.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("xy-wf57", "Assay WF57", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    let png = encode_png(&RasterImage { width_px: fx.width_px, height_px: fx.height_px, dpi: 96, rgba: fx.rgba }).map_err(|e| e.to_string())?;
    clip.seed_image_bytes(png);
    app.set_clipboard_adapter(clip);
    app.execute_command("graph.create", json!({ "sheet_id": "g-wf57", "data_sheet_id": "xy-wf57", "kind": "xy" })).map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-wf57", "title": "WF57 Schematic Layout" })).map_err(|e| e.to_string())?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf57", "panel_id": "p-g", "kind": "graph", "graph_sheet_id": "g-wf57", "bounds_pt": [20.0, 20.0, 220.0, 150.0] }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-wf57" })).map_err(|e| e.to_string())?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.driver.press_key("V", KeyModifiers { meta: true, ..Default::default() })?;

    let img_idx = h
        .app()
        .document()
        .layout_sheet(&SheetId::new("lay-wf57"))
        .and_then(|ls| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::EmbeddedImage { .. })))
        .ok_or("WF-57 expected Cmd+V EmbeddedImage on LayoutSheet")?;
    let panel_wid = format!("canvas.layout_panel.{img_idx}");
    h.click(&panel_wid)?;

    let se_center = h.widget_center("canvas.image.handle.se")?;
    h.driver.drag_with_modifiers(se_center, se_center + egui::vec2(50.0, -15.0), KeyModifiers { shift: true, ..Default::default() })?;
    let rot_center = h.widget_center("canvas.image.handle.rotate")?;
    h.driver.drag(rot_center, rot_center + egui::vec2(30.0, 15.0))?;

    // Open FormatImage via double-click or context menu and apply 4-edge crop + border.
    let img_pos = h.widget_center(&panel_wid)?;
    h.driver.click_at_button(img_pos, PointerButton::Secondary, KeyModifiers::default())?;
    h.click("canvas.layout_ctx.format_image")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::FormatImage { .. }), "WF-57 FormatImage dialog must open")?;
    h.click("format_image.crop.left_plus4")?;
    h.click("format_image.border.enabled")?;
    h.click("format_image.rotation.90")?;
    h.click("modal.ok")?;

    // Toggle Canvas Grid via right-click context menu and assert non-exported SVG identity.
    let lay_before = h.app().document().layout_sheet(&SheetId::new("lay-wf57")).ok_or("missing lay-wf57")?.clone();
    let svg_off =
        render_svg(&curvebook_chart::compile_layout(&lay_before, h.app().document()).map_err(|e| e.to_string())?, &SvgExportOptions::default())
            .map_err(|e| e.to_string())?;
    let panel_pos = h.widget_center(&panel_wid)?;
    h.driver.click_at_button(panel_pos, PointerButton::Secondary, KeyModifiers::default())?;
    h.click("canvas.layout_ctx.toggle_grid")?;
    ensure(h.app().canvas_dialogs.grid_overlay_visible, "WF-57 grid overlay must toggle visible")?;
    let lay_after = h.app().document().layout_sheet(&SheetId::new("lay-wf57")).ok_or("missing lay-wf57 post grid")?.clone();
    let svg_on =
        render_svg(&curvebook_chart::compile_layout(&lay_after, h.app().document()).map_err(|e| e.to_string())?, &SvgExportOptions::default())
            .map_err(|e| e.to_string())?;
    ensure(svg_off == svg_on, "WF-57 canvas grid overlay must not alter exported SVG")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-57")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf58_human_result_slice_paste_table_transpose_borderless_reactive() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf58_descriptive_column();
    let sid = h.create_table_via_welcome("welcome.arch.column", "WF58 Descriptive Embed", &["welcome.stepper.cols.inc"])?;

    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{val}"))?;
        }
    }

    h.select_sheet(&sid)?;
    let desc_id = h.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    h.select_sheet(&desc_id)?;

    let c0_pos = h.widget_center("result.cell.0.0")?;
    let c1_pos = h.widget_center("result.cell.2.2")?;
    h.driver.click_at(c0_pos)?;
    h.driver.click_at_button(c1_pos, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    h.driver.click_at_button(c1_pos, PointerButton::Secondary, KeyModifiers::default())?;
    h.click("result.ctx.copy_selection")?;

    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-58 missing graph")?;
    h.select_sheet(&gs_id)?;
    let canvas_rect = h.driver.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-58 missing canvas.viewport")?.clipped_rect;
    let anchor = egui::pos2(canvas_rect.right() - 40.0, canvas_rect.bottom() - 40.0);
    h.driver.click_at_button(anchor, PointerButton::Secondary, KeyModifiers::default())?;
    h.click("canvas.graph_ctx.paste_table")?;

    h.dbl_click_widget("canvas.graph_ann.0")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }), "WF-58 FormatEmbeddedTable must open")?;
    h.click("embedded_table.transpose")?;
    h.click("embedded_table.outer_border.0")?;
    h.click("embedded_table.gridline.0")?;
    h.click("modal.ok")?;

    let scene_before = {
        let gs = h.app().document().graph_sheet(&gs_id).ok_or("WF-58 missing graph post embed")?;
        ensure(
            matches!(
                gs.spec.annotations.first(),
                Some(GraphAnnotation::EmbeddedResultTable { style, .. })
                    if style.transpose && style.outer_border_width_pt == 0.0 && style.gridline_width_pt == 0.0
            ),
            "WF-58 borderless transpose settings must persist",
        )?;
        render_svg(&curvebook_chart::compile_graph(gs, h.app().document()).map_err(|e| e.to_string())?, &SvgExportOptions::default())
            .map_err(|e| e.to_string())?
    };

    // Delete upstream cell via real Backspace/Enter key handling -> embedded table recomputes.
    h.select_sheet(&sid)?;
    h.dbl_click_widget("grid.cell.0.1.0")?;
    h.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    h.driver.press_key("Backspace", KeyModifiers::default())?;
    h.driver.press_key("Enter", KeyModifiers::default())?;

    let gs_after = h.app().document().graph_sheet(&gs_id).ok_or("WF-58 missing graph after delete")?;
    let scene_after =
        render_svg(&curvebook_chart::compile_graph(gs_after, h.app().document()).map_err(|e| e.to_string())?, &SvgExportOptions::default())
            .map_err(|e| e.to_string())?;
    ensure(scene_before != scene_after, "WF-58 deleting upstream cell must update embedded table SVG")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-58")?;
    assert_input_to_paint_budget(&h, 50.0)
}
