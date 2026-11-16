#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R10` Phase 2 Core Infrastructure acceptance suite & interactive `R10.1` ANOVA spike:
//! - `R10.1`: End-to-end human-path `AnalyzeSheet` -> One-Way ANOVA -> `EditAnalysisParameters` (`Cmd+T`)
//! - `R10.2`: `HumanPathOnly` policy blocks synthetic bypasses + `verify_pixel_to_hit_box_parity` + `verify_click_action_parity`
//! - `R10.9`: L6 `TextMeasurer` & 4x4 coverage-mask glyph atlas (zero `FONT_5X7`, proportional/tabular/Greek)
//! - `R10.15`: Aspect-ratio centered paper viewport, zoom-to-fit / zoom-%, hover/selected-only chrome
//! - `R10.16`: Retained widgets, `ModalBackdrop` capture, modal focus trapping, IME/clipboard textbox, dropdown, scrolling
//! - `R10.22`: `1,000,000`-cell virtualized grid frame-time (`<= 16.67 ms`) & input-to-paint (`<= 50 ms`) budgets

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GraphKind, GraphSheet, GroupedTable, LayoutItem, LayoutSheet,
    LayoutSpec, MultiplicityMethod, ProjectDocument, RectPt, ResultCell, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
use curvebook_ui_egui::compositor::{FramePalette, compute_workspace_layout_map, render_workspace_rgba};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::FocusScope;
use curvebook_ui_egui::layout::canvas_viewport::{
    compute_aspect_ratio_paper_mapping, should_show_graph_series_chip, should_show_layout_panel_chrome,
};
use curvebook_ui_egui::layout::verify::verify_click_action_parity;
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, SoftwareTextPainter, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::json;

fn build_three_group_column_app() -> UiApp {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).expect("create column table");
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[2].title = String::from("HighDose");
    let ctrl = [10.0, 11.2, 9.8, 10.6, 10.4];
    let trt = [14.5, 15.1, 13.9, 14.8, 15.3];
    let high = [20.1, 21.0, 19.6, 20.8, 21.5];
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(ctrl[r]);
        col.data_sets[1].cells[r][0] = Cell::number(trt[r]);
        col.data_sets[2].cells[r][0] = Cell::number(high[r]);
    }
    let sheet = DataSheet::new("col-spike", "Drug Dose Assay", TableArchetype::Column(col)).expect("data sheet");
    doc.add_data_sheet(sheet).expect("add sheet");
    doc.select_in_navigator(&SheetId::from("col-spike")).expect("select sheet");
    UiApp::from_session(Session::from_document(doc))
}

#[test]
fn r10_1_spike_analyze_and_edit_params_one_way_anova_human_path() {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = HumanPathDriver::new(build_three_group_column_app(), metrics);

    // 1. Clean R10.2 parity on workspace DataSheet before opening modal.
    let report_data = driver.verify_parity();
    assert!(report_data.is_clean(), "DataSheet parity violations: {:?}", report_data.violations);

    // 2. Click rendered Toolbar -> Analyze... button strictly by physical/logical center.
    driver.click_widget("toolbar.analyze").expect("click toolbar.analyze");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    let report_analyze = driver.verify_parity();
    assert!(report_analyze.is_clean(), "AnalyzeSheet parity violations: {:?}", report_analyze.violations);

    // 3. Select One-Way ANOVA in the scrollable Analyze list (wheel scroll if below fold) and confirm via modal.ok.
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = driver.wheel_widget("analyze.entries_list", 0.0, -180.0, KeyModifiers::default());
    }
    driver.click_widget("analyze.entry.stats.one_way_anova").expect("click one_way_anova");
    driver.click_widget("modal.ok").expect("confirm AnalyzeSheet");
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.app().document().result_sheets.len(), 1, "One-Way ANOVA ResultSheet should be created");

    let rs_id = driver.app().document().result_sheets[0].id.clone();
    assert_eq!(driver.app().document().active_sheet_id.as_ref(), Some(&rs_id));
    let report_res = driver.verify_parity();
    assert!(report_res.is_clean(), "ResultSheet parity violations: {:?}", report_res.violations);

    // 4. Press Cmd+T on active ResultSheet -> MUST open EditAnalysisParameters (never create blank table!).
    driver.press_key("T", KeyModifiers { meta: true, ..Default::default() }).expect("Cmd+T shortcut on ResultSheet");
    assert!(
        matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
        "expected Cmd+T on ResultSheet to open EditAnalysisParameters, got {:?}",
        driver.app().canvas_dialogs.active_dialog
    );
    let report_edit = driver.verify_parity();
    assert!(report_edit.is_clean(), "EditAnalysisParameters parity violations: {:?}", report_edit.violations);

    // 5. ModalBackdrop captures outside clicks (zero click-through to toolbar/navigator behind dialog!).
    let res_count_before = driver.app().document().result_sheets.len();
    let backdrop_outcome = driver.click_logical(8.0, 20.0).expect("click outside modal on backdrop");
    assert_eq!(backdrop_outcome.action, "modal.backdrop_blocked");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    assert_eq!(driver.app().document().result_sheets.len(), res_count_before);

    // 6. Focus trapping inside ModalDialog: Tab / Shift+Tab stays inside ModalDialog scope.
    let mut visited_modal_focus = Vec::new();
    for _ in 0..5 {
        driver.press_key("Tab", KeyModifiers::default()).expect("Tab inside modal");
        if let Some(fid) = driver.app().focus_widget_id.clone() {
            let tree = driver.layout_tree();
            let node = tree.node(&fid).expect("focused modal widget exists in LayoutTree");
            assert_eq!(node.a11y.focus_scope, FocusScope::ModalDialog, "Tab escaped modal scope to `{fid}`");
            visited_modal_focus.push(fid);
        }
    }
    assert!(visited_modal_focus.len() >= 3, "expected Tab to traverse modal widgets");
    driver.press_key("Tab", KeyModifiers { shift: true, ..Default::default() }).expect("Shift+Tab in modal");
    assert_eq!(
        driver.layout_tree().node(driver.app().focus_widget_id.as_deref().unwrap_or("")).map(|n| n.a11y.focus_scope),
        Some(FocusScope::ModalDialog)
    );

    // 7. Uncheck Assume Gaussian -> Kruskal-Wallis / Dunn's R2 gate disables Tukey + exposes tooltip on hover.
    driver.click_widget("edit.assume_gaussian").expect("toggle Assume Gaussian off");
    assert!(!driver.app().canvas_dialogs.edit_params.assume_gaussian);
    let tukey = driver.find_widget("edit.post_hoc.tukey").expect("tukey control rendered in EditAnalysisParameters");
    assert!(!tukey.state.enablement.enabled, "R2 must disable Tukey when nonparametric");
    let expected_reason = tukey.state.enablement.disabled_reason.clone().expect("disabled reason present");
    driver.hover_widget("edit.post_hoc.tukey").expect("hover disabled tukey control");
    let post_hover_tree = driver.layout_tree();
    assert!(
        post_hover_tree.text_runs.iter().any(|r| r.text.contains("disabled") || expected_reason.starts_with(&r.text[..r.text.len().min(12)])),
        "expected hover tooltip text run for disabled Tukey reason `{expected_reason}`"
    );

    // 8. Confirm via modal.ok -> ResultSheet updates to Kruskal-Wallis + Dunn's, keeping input-to-paint <= 50ms!
    driver.click_widget("modal.ok").expect("confirm EditAnalysisParameters");
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(
        driver.app().document().result_sheet(&rs_id).is_some_and(|rs| rs.title().contains("Kruskal-Wallis")),
        "expected ResultSheet switched to Kruskal-Wallis"
    );
    assert!(driver.max_input_to_paint_ms() <= 50.0, "R10.22 input-to-paint budget exceeded: {:.2} ms > 50 ms", driver.max_input_to_paint_ms());
}

#[test]
fn r10_2_human_path_only_blocks_synthetic_bypasses() {
    let mut app = build_three_group_column_app();
    app.set_gui_only(true);
    app.set_human_path_only(true);

    let err_menu = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "sheet.createData", "params": { "kind": "column", "cols": 3, "rows": 4 } }),
    )
    .expect_err("invoke_menu with semantic params must fail under human_path_only");
    assert_eq!(err_menu.code, ErrorCode::GuiOnlyViolation);

    app.canvas_dialogs.open_analyze_dialog(&app.document().clone(), &SheetId::from("col-spike"));
    let err_dlg = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }))
        .expect_err("ui.answer_dialog must fail under human_path_only");
    assert_eq!(err_dlg.code, ErrorCode::GuiOnlyViolation);

    let err_sem = dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))
        .expect_err("semantic widget click shortcut must fail under human_path_only");
    assert_eq!(err_sem.code, ErrorCode::GuiOnlyViolation);

    app.canvas_dialogs.close_dialog();
    app.focus_widget_id = Some(String::from("grid.cell.0.0.0"));
    let err_jump = dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.1.1.0", "text": "42" }))
        .expect_err("focus-jumping ui.type_text must fail under human_path_only");
    assert_eq!(err_jump.code, ErrorCode::GuiOnlyViolation);
}

#[test]
fn r10_2_and_r10_16_parity_across_views_plus_textbox_ime_clipboard_dropdown_and_scroll() {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = HumanPathDriver::new(build_three_group_column_app(), metrics);

    // CommandPalette: click Cmd+K button, verify parity, test caret/selection/copy/paste + IME preedit!
    driver.click_widget("toolbar.palette").expect("open Command Palette");
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    assert!(driver.verify_parity().is_clean());
    driver.click_widget("palette.query").expect("focus palette search field");
    driver.type_text("ANOVA").expect("type ANOVA in palette search");
    driver.press_key("ArrowLeft", KeyModifiers { shift: true, ..Default::default() }).expect("select last char");
    driver.press_key("c", KeyModifiers { meta: true, ..Default::default() }).expect("copy selection");
    driver.press_key("ArrowRight", KeyModifiers::default()).expect("move to end");
    driver.press_key("v", KeyModifiers { meta: true, ..Default::default() }).expect("paste selection");
    assert_eq!(driver.app().canvas_dialogs.command_palette.query, "ANOVAA");
    driver.set_ime_preedit(Some(String::from("α")));
    assert!(driver.layout_tree().text_runs.iter().any(|r| r.text.contains('α')));
    driver.press_key("Escape", KeyModifiers::default()).expect("Escape closes Command Palette");
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    // Click-to-action parity test across toolbar & navigator widgets.
    let parity_app = build_three_group_column_app();
    for wid in ["toolbar.analyze", "toolbar.palette", "nav.sheet.col-spike"] {
        assert!(verify_click_action_parity(&parity_app, metrics, wid).expect(wid), "click parity failed for {wid}");
    }

    // Multi-row ResultSheet (>6 rows, >4 tabs) renders scrollable table without 4-8 truncation cap.
    let mut big_driver = HumanPathDriver::new(build_three_group_column_app(), metrics);
    big_driver.click_widget("toolbar.analyze").expect("open analyze");
    big_driver.click_widget("analyze.entry.stats.one_way_anova").expect("select anova");
    big_driver.click_widget("modal.ok").expect("confirm");
    let big_rs_id = big_driver.app().document().result_sheets[0].id.clone();
    let mut doc = big_driver.app().document().clone();
    if let Some(rs) = doc.result_sheets.get_mut(0) {
        for r in 0..40 {
            if let Some(t0) = rs.tabs.get_mut(0) {
                t0.row_labels.push(format!("Extra Pair {}", r + 1));
                t0.rows.push(vec![ResultCell::Number { value: 0.001 * (r as f64 + 1.0), prefix: None, units: None }]);
            }
        }
        for i in 0..7 {
            rs.tabs.push(ResultTab {
                id: format!("tab-extra-{i}"),
                title: format!("Tab {i}"),
                kind: ResultTabKind::MainSummary,
                column_headers: vec![String::from("Estimate"), String::from("P value")],
                row_labels: vec![String::from("R1")],
                rows: vec![vec![ResultCell::Number { value: 0.05, prefix: None, units: None }]],
                footnotes: Vec::new(),
            });
        }
    }
    let mut big_driver = HumanPathDriver::new(UiApp::from_session(Session::from_document(doc)), metrics);
    assert!(big_driver.verify_parity().is_clean());
    let tree_before = big_driver.layout_tree();
    let drawn_tabs = tree_before.nodes.iter().filter(|n| n.id.as_str().starts_with("result.tab.") && n.drawn_in_frame).count();
    assert!(drawn_tabs >= 6, "expected >4 tabs rendered without 4-tab cap, got {drawn_tabs}");
    let scroll_out = big_driver.wheel_widget("result.table", 0.0, 120.0, KeyModifiers::default()).expect("scroll result table");
    assert_eq!(scroll_out.action, "scroll_container.wheel");
    let scroll_key = format!("scroll.result.{}", big_rs_id.as_str());
    assert!(
        big_driver.app().interaction.scroll_offsets.offsets.get(&scroll_key).copied().unwrap_or_default()[1] > 0.0,
        "expected vertical scroll offset > 0 on `{scroll_key}`"
    );

    // Analyze -> ANOVA -> Open EditAnalysisParameters -> open & wheel-scroll multiplicity dropdown popup.
    let mut an_driver = HumanPathDriver::new(build_three_group_column_app(), metrics);
    an_driver.click_widget("toolbar.analyze").expect("open analyze");
    an_driver.click_widget("analyze.entry.stats.one_way_anova").expect("select anova");
    an_driver.click_widget("modal.ok").expect("confirm");
    an_driver.press_key("T", KeyModifiers { meta: true, ..Default::default() }).expect("open edit params");
    an_driver.click_widget("edit.multiplicity_method").expect("open dropdown popup");
    assert!(an_driver.app().interaction.open_dropdown.is_some());
    assert!(an_driver.verify_parity().is_clean(), "open dropdown parity must be clean");
    an_driver.click_widget("popup.edit.multiplicity_method.opt.2").expect("select Bonferroni option");
    assert!(an_driver.app().interaction.open_dropdown.is_none());
    an_driver.click_widget("modal.ok").expect("confirm edit params");
    let updated_rs = an_driver.app().document().result_sheets.last().expect("updated result sheet");
    if let AnalysisKind::OneWayAnova(spec) = &updated_rs.analysis.kind {
        assert_eq!(spec.multiplicity.method, MultiplicityMethod::Bonferroni);
    } else {
        panic!("expected OneWayAnova spec");
    }
}

#[test]
fn r10_9_text_measurer_and_coverage_atlas_retire_font_5x7() {
    let measurer = DesktopTextMeasurer::global();
    let sans = UiTextStyle::ui_sans(14.0);
    let w_w = measurer.measure("W", &sans, "en").width_px;
    let w_i = measurer.measure("i", &sans, "en").width_px;
    assert!(w_w > w_i * 1.2, "proportional sans expected W ({w_w}) > i ({w_i})");

    let tab = UiTextStyle::grid_tabular(13.0);
    let w_1 = measurer.measure("11111", &tab, "en").width_px;
    let w_8 = measurer.measure("88888", &tab, "en").width_px;
    assert!((w_1 - w_8).abs() < 0.05, "tabular digits must align equally: 1={w_1}, 8={w_8}");

    let shaped = measurer.shape_line("IC50 α±μ 10⁻⁶", &sans, "en");
    assert!(shaped.metrics.width_px > 20.0);

    // Mixed case vs uppercase must render distinct software pixels (proving FONT_5X7 is gone!).
    let mut app_a = build_three_group_column_app();
    let mut app_b = build_three_group_column_app();
    if let Some(ds) = app_a.session().document().data_sheets.first().cloned() {
        let _ = app_a.execute_command("table.setCell", json!({ "sheet_id": ds.id.as_str(), "row": 0, "col": 0, "subcol": 0, "value": "AbcXy" }));
        let _ = app_b.execute_command("table.setCell", json!({ "sheet_id": ds.id.as_str(), "row": 0, "col": 0, "subcol": 0, "value": "ABCXY" }));
    }
    let target = RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 };
    let img_a = render_workspace_rgba(&app_a, target).expect("render mixed case");
    let img_b = render_workspace_rgba(&app_b, target).expect("render uppercase");
    assert_ne!(img_a.rgba, img_b.rgba, "FONT_5X7 uppercased all glyphs; real atlas must render distinct pixels");

    let _ = SoftwareTextPainter::paint_text_top_left;
    let _ = FramePalette::from_theme;
}

#[test]
fn r10_15_aspect_ratio_viewport_and_hover_selected_only_chrome() {
    let mut app = build_three_group_column_app();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-ar", &app.document().data_sheets[0]).expect("graph");
    gs.title = String::from("Dose Plot");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Column;

    let ls_spec = LayoutSpec {
        items: vec![LayoutItem::LiveGraph {
            id: String::from("p0"),
            graph_sheet_id: SheetId::from("g-ar"),
            bounds_pt: RectPt::new(36.0, 36.0, 300.0, 220.0),
            scale_percent: 100.0,
            badge_override: None,
        }],
        ..LayoutSpec::default()
    };
    let page_pt = ls_spec.page_size_pt();
    let ls = LayoutSheet::new("l-ar", "Page Layout", ls_spec).expect("layout");

    let mut doc = app.document().clone();
    doc.add_graph_sheet(gs).expect("add graph");
    doc.add_layout_sheet(ls).expect("add layout");
    doc.select_in_navigator(&SheetId::from("l-ar")).expect("select layout");
    app = UiApp::from_session(Session::from_document(doc));

    // Wide pane (1000 x 600): paper preserves physical aspect ratio and stays centered.
    let wide_outer = curvebook_ui_egui::layout::LayoutRectPx::new(200, 40, 1000, 600);
    let geom_wide = compute_aspect_ratio_paper_mapping(SheetId::from("l-ar"), wide_outer, page_pt.width, page_pt.height, &app, 1.0);
    let ratio_wide = f64::from(geom_wide.paper_rect_px.width) / f64::from(geom_wide.paper_rect_px.height.max(1));
    let expected_ratio = page_pt.width / page_pt.height;
    assert!((ratio_wide - expected_ratio).abs() < 0.03, "expected aspect {expected_ratio}, got {ratio_wide}");
    let left_margin = geom_wide.paper_rect_px.x - geom_wide.mat_rect_px.x;
    let right_margin = geom_wide.mat_rect_px.right() - geom_wide.paper_rect_px.right();
    assert!((left_margin - right_margin).abs() <= 2, "paper must be centered horizontally: {left_margin} vs {right_margin}");
    assert!(geom_wide.h_scrollbar.is_none() && geom_wide.v_scrollbar.is_none());

    // Selection/hover-only chrome gates (`R10.15`): hidden by default, shown only when hovered/selected.
    assert!(!should_show_graph_series_chip(&app, 0));
    assert!(!should_show_layout_panel_chrome(&app, "panel.0"));
    app.interaction.hovered_widget_id = Some(String::from("canvas.layout_panel.panel.0"));
    assert!(should_show_layout_panel_chrome(&app, "panel.0"));
    app.interaction.hovered_widget_id = None;
    app.canvas_dialogs.canvas.selected_object_id = Some(String::from("panel.0"));
    assert!(should_show_layout_panel_chrome(&app, "panel.0"));

    // Zoom-to-Fit vs explicit zoom-% button & pan scrollbars at zoom 200%.
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = HumanPathDriver::new(app, metrics);
    assert!(driver.verify_parity().is_clean());
    driver.click_widget("canvas.main.zoom_pct").expect("cycle zoom % preset");
    assert!(driver.app().canvas_dialogs.canvas.zoom > 1.05 || (driver.app().canvas_dialogs.canvas.zoom - 0.5).abs() < 0.05);
    driver.app_mut().canvas_dialogs.canvas.zoom = 2.0;
    driver.app_mut().canvas_dialogs.pan_zoom.zoom = 2.0;
    let geom_zoomed = compute_aspect_ratio_paper_mapping(SheetId::from("l-ar"), wide_outer, page_pt.width, page_pt.height, driver.app(), 1.0);
    assert!(geom_zoomed.h_scrollbar.is_some() && geom_zoomed.v_scrollbar.is_some());
    driver.click_widget("canvas.main.zoom_fit").expect("Zoom to Fit resets to 100%");
    assert!((driver.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    let _ = compute_workspace_layout_map(driver.app(), metrics);
}

#[test]
fn r10_22_one_million_cell_virtualized_grid_frame_budget() {
    let mut doc = ProjectDocument::default();
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(4), 250, 1_000).expect("1M grouped table");
    for (g_idx, group) in grouped.data_sets.iter_mut().enumerate() {
        group.title = format!("Cohort {}", g_idx + 1);
        for (r_idx, row) in group.cells.iter_mut().enumerate() {
            for (s_idx, cell) in row.iter_mut().enumerate() {
                *cell = Cell::number(((g_idx * 4 + s_idx) as f64) * 0.1 + (r_idx as f64));
            }
        }
    }
    let big_sheet = DataSheet::new("tbl-1m", "Mega Assay 1M", TableArchetype::Grouped(grouped)).expect("sheet");
    doc.add_data_sheet(big_sheet).expect("add big sheet");
    doc.select_in_navigator(&SheetId::from("tbl-1m")).expect("select big sheet");

    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = HumanPathDriver::new(UiApp::from_session(Session::from_document(doc)), metrics);

    // Warm glyph atlas & retained layout frame once (steady-state interactive frame budget).
    let _ = driver.render_frame().expect("warm frame");

    let mut max_ms = 0.0_f64;
    let mut total_ms = 0.0_f64;
    let frame_count = 24usize;
    for step in 0..frame_count {
        if step > 0 {
            let _ = driver.wheel_widget("grid.sheet.tbl-1m", 0.0, 48.0, KeyModifiers::default());
        }
        let t0 = Instant::now();
        let img = driver.render_frame().expect("render 1M grid frame");
        assert_eq!(img.width_px, 1280);
        let dt_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        total_ms += dt_ms;
        max_ms = max_ms.max(dt_ms);
    }
    let mean_ms = total_ms / (frame_count as f64);
    assert!(mean_ms <= 16.67, "R10.22 1M-cell grid mean frame budget exceeded: mean={mean_ms:.2}ms (max={max_ms:.2}ms, limit=16.67ms)");
}
