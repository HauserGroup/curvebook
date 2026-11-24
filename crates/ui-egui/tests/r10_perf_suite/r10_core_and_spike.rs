#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R10` Phase 2 Core Infrastructure acceptance suite & interactive `R10.1` ANOVA spike on native `egui`:
//! - `R10.1`: End-to-end human-path `AnalyzeSheet` -> One-Way ANOVA -> `EditAnalysisParameters` (`Cmd+T`)
//! - `R10.2`: `HumanPathOnly` policy blocks synthetic bypasses + `verify_pixel_to_hit_box_parity` + click-to-action parity
//! - `R10.9`: L6 `TextMeasurer` & proportional/tabular/Greek coverage + case-distinct rasterized `egui` pixels
//! - `R10.15`: Aspect-ratio centered paper viewport, zoom-to-fit / status zoom controls, hover/selected chrome
//! - `R10.16`: Modal backdrop capture, modal focus trapping, IME/paste textbox, tab/post-hoc controls, scrolling
//! - `R10.22`: `1,000,000`-cell virtualized grid frame-time (`<= 16.67 ms` release) & input-to-paint (`<= 50 ms`) budgets

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GraphKind, GraphSheet, GroupedTable, LayoutItem, LayoutSheet,
    LayoutSpec, MultiplicityMethod, ProjectDocument, RectPt, ResultCell, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::{PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::{ScreenshotRegionTarget, UiApp, dispatch_gui_action};
use egui::{Pos2, Vec2};
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
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app
}

#[test]
fn r10_1_spike_analyze_and_edit_params_one_way_anova_human_path() {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(build_three_group_column_app(), metrics);

    // 1. Clean R10.2 parity on workspace DataSheet before opening modal.
    harness.assert_parity_and_layout("DataSheet").expect("DataSheet parity");

    // 2. Click rendered Toolbar -> Analyze... button strictly by physical/logical center.
    harness.click("toolbar.analyze").expect("click toolbar.analyze");
    assert!(matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    harness.assert_parity_and_layout("AnalyzeSheet").expect("AnalyzeSheet parity");

    // 3. Select One-Way ANOVA in the scrollable Analyze list and confirm via modal.ok.
    if harness.driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_none()
        && let Some(anchor) = harness
            .driver
            .frame_meta()
            .find_by_widget_id("analyze.search")
            .map(|e| Pos2::new(e.clipped_rect.center().x, e.clipped_rect.bottom() + 80.0))
    {
        let _ = harness.driver.wheel_at(anchor, Vec2::new(0.0, -180.0));
    }
    harness.click("analyze.entry.stats.one_way_anova").expect("click one_way_anova");
    harness.click("modal.ok").expect("confirm AnalyzeSheet");
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(harness.app().document().result_sheets.len(), 1, "One-Way ANOVA ResultSheet should be created");

    let rs_id = harness.app().document().result_sheets[0].id.clone();
    assert_eq!(harness.app().document().active_sheet_id.as_ref(), Some(&rs_id));
    harness.assert_parity_and_layout("ResultSheet").expect("ResultSheet parity");

    // 4. Press Cmd+T on active ResultSheet -> MUST open EditAnalysisParameters (never create blank table!).
    harness.open_cmd_t().expect("Cmd+T shortcut on ResultSheet");
    assert!(
        matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
        "expected Cmd+T on ResultSheet to open EditAnalysisParameters, got {:?}",
        harness.app().canvas_dialogs.active_dialog
    );
    harness.assert_parity_and_layout("EditAnalysisParameters").expect("EditAnalysisParameters parity");

    // 5. ModalBackdrop captures outside clicks (zero click-through to toolbar/navigator behind dialog!).
    let res_count_before = harness.app().document().result_sheets.len();
    harness.driver.click_at(Pos2::new(8.0, 20.0)).expect("click outside modal on backdrop");
    assert!(matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    assert_eq!(harness.app().document().result_sheets.len(), res_count_before);

    // 6. Focus trapping inside ModalDialog: Tab / Shift+Tab stays inside ModalDialog scope.
    for _ in 0..5 {
        harness.driver.press_key("Tab", KeyModifiers::default()).expect("Tab inside modal");
        assert!(matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    }
    harness.driver.press_key("Tab", KeyModifiers { shift: true, ..Default::default() }).expect("Shift+Tab in modal");
    assert!(matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    // 7. Uncheck Assume Gaussian -> Kruskal-Wallis / Dunn's R2 gate disables Tukey + exposes tooltip on hover.
    harness.click("edit.assume_gaussian").expect("toggle Assume Gaussian off");
    assert!(!harness.app().canvas_dialogs.edit_params.assume_gaussian);
    harness.click("edit.tab.multiple_comparisons").expect("switch to Multiple Comparisons tab");
    let tukey =
        harness.driver.frame_meta().find_by_widget_id("edit.post_hoc.tukey").cloned().expect("tukey control rendered in EditAnalysisParameters");
    assert!(!tukey.enabled, "R2 must disable Tukey when nonparametric");
    let expected_reason = tukey.disabled_reason.clone().expect("disabled reason present");
    harness.driver.hover_widget_id("edit.post_hoc.tukey").expect("hover disabled tukey control");
    let tooltips = harness.driver.visible_tooltip_texts();
    let expected_prefix: String = expected_reason.chars().take(12).collect();
    assert!(
        tooltips.iter().any(|t| t.contains("disabled") || t.contains(&expected_prefix)),
        "expected hover tooltip text for disabled Tukey reason `{expected_reason}`, got {tooltips:?}"
    );

    // 8. Confirm via modal.ok -> ResultSheet updates to Kruskal-Wallis + Dunn's, keeping input-to-paint <= 50ms!
    harness.click("modal.ok").expect("confirm EditAnalysisParameters");
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(
        harness.app().document().result_sheet(&rs_id).is_some_and(|rs| rs.title().contains("Kruskal-Wallis")),
        "expected ResultSheet switched to Kruskal-Wallis"
    );
    crate::human_wf_common::assert_input_to_paint_budget(&harness, 50.0).expect("input-to-paint budget");
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
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(build_three_group_column_app(), metrics);

    // CommandPalette: click Cmd+K button, verify parity, test paste & IME commit + Escape close.
    harness.click("toolbar.command_palette").expect("open Command Palette");
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    harness.assert_parity_and_layout("CommandPalette").expect("CommandPalette parity");
    harness.click("palette.query").expect("focus palette search field");
    harness.driver.type_text("ANOVA").expect("type ANOVA in palette search");
    harness.driver.dispatch_events(vec![egui::Event::Paste(String::from("A"))]);
    assert_eq!(harness.driver.state().views.command_palette.query, "ANOVAA");
    harness.driver.ime_preedit("α", None);
    harness.driver.ime_commit("α");
    assert!(harness.driver.state().views.command_palette.query.contains('α'));
    harness.driver.press_key("Escape", KeyModifiers::default()).expect("Escape closes Command Palette");
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    // Click-to-action parity test across toolbar & navigator widgets on native egui.
    for wid in ["toolbar.analyze", "toolbar.command_palette", "nav.sheet.col-spike"] {
        let mut h = crate::human_wf_common::EguiWfHarness::from_app(build_three_group_column_app(), metrics);
        h.assert_parity_and_layout(&format!("click parity pre `{wid}`")).expect("pre parity");
        h.click(wid).unwrap_or_else(|e| panic!("click `{wid}` failed: {e}"));
        match wid {
            "toolbar.analyze" => assert!(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. })),
            "toolbar.command_palette" => assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
            "nav.sheet.col-spike" => assert_eq!(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-spike")),
            _ => panic!("unexpected widget id `{wid}`"),
        }
    }

    // Multi-row ResultSheet (>40 rows, >7 tabs) renders scrollable table without 4-tab truncation cap.
    let mut doc = build_three_group_column_app().document().clone();
    let mut big_h = crate::human_wf_common::EguiWfHarness::from_app(UiApp::from_session(Session::from_document(doc.clone())), metrics);
    big_h.click("toolbar.analyze").expect("open analyze");
    big_h.click("analyze.entry.stats.one_way_anova").expect("select anova");
    big_h.click("modal.ok").expect("confirm");
    doc = big_h.app().document().clone();
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
    let mut app_big = UiApp::from_session(Session::from_document(doc));
    app_big.nav.welcome.open = false;
    let mut big_h2 = crate::human_wf_common::EguiWfHarness::from_app(app_big, metrics);
    big_h2.assert_parity_and_layout("multi-row result sheet").expect("result sheet parity");
    let drawn_tabs = big_h2.driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("result.tab.")).count();
    assert!(drawn_tabs >= 6, "expected >4 tabs rendered without 4-tab cap, got {drawn_tabs}");
    let sec_pos = big_h2
        .driver
        .frame_meta()
        .find_by_widget_id("result.section.0")
        .or_else(|| big_h2.driver.frame_meta().find_by_widget_id("result.cell.0.0"))
        .map(|e| e.clipped_rect.center())
        .expect("missing scrollable result.section.0 or result.cell.0.0 widget");
    big_h2.driver.wheel_at(sec_pos, Vec2::new(0.0, -240.0)).expect("scroll result table");
    assert!(big_h2.driver.verify_pixel_to_hit_box_parity().is_clean());

    // Analyze -> ANOVA -> Open EditAnalysisParameters -> switch to Multiple Comparisons & select Bonferroni.
    let mut an_h = crate::human_wf_common::EguiWfHarness::from_app(build_three_group_column_app(), metrics);
    an_h.click("toolbar.analyze").expect("open analyze");
    an_h.click("analyze.entry.stats.one_way_anova").expect("select anova");
    an_h.click("modal.ok").expect("confirm");
    an_h.open_cmd_t().expect("open edit params");
    an_h.click("edit.tab.multiple_comparisons").expect("open multiple comparisons tab");
    an_h.assert_parity_and_layout("multiple comparisons tab").expect("mc parity");
    an_h.click("edit.post_hoc.bonferroni").expect("select Bonferroni");
    an_h.click("modal.ok").expect("confirm edit params");
    let updated_rs = an_h.app().document().result_sheets.last().expect("updated result sheet");
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

    // Mixed case vs uppercase must render distinct software pixels via EguiSoftwareRasterizer.
    let mut app_a = build_three_group_column_app();
    let mut app_b = build_three_group_column_app();
    if let Some(ds) = app_a.session().document().data_sheets.first().cloned() {
        let _ = app_a.execute_command("table.setCell", json!({ "sheet_id": ds.id.as_str(), "row": 0, "col": 0, "subcol": 0, "value": "AbcXy" }));
        let _ = app_b.execute_command("table.setCell", json!({ "sheet_id": ds.id.as_str(), "row": 0, "col": 0, "subcol": 0, "value": "ABCXY" }));
    }
    let target = RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 };
    let img_a = app_a.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("render mixed case");
    let img_b = app_b.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("render uppercase");
    assert_ne!(img_a.rgba, img_b.rgba, "real atlas in egui_shell must render distinct mixed vs uppercase pixels");
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
    app.nav.welcome.open = false;

    // 1. Wide pane (1000 x 600): native egui canvas_view paper preserves physical aspect ratio & horizontal centering.
    struct CanvasState {
        app: UiApp,
        view: CanvasViewEguiState,
        intents: PendingUiIntents,
        paper_rect: Option<egui::Rect>,
        viewport_rect: Option<egui::Rect>,
    }
    let init = CanvasState {
        app: app.clone(),
        view: CanvasViewEguiState::default(),
        intents: PendingUiIntents::new(),
        paper_rect: None,
        viewport_rect: None,
    };
    let mut canvas_driver = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
        ViewportMetrics::new(1000.0, 600.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            st.viewport_rect = Some(ui.max_rect());
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false };
            let rep = render_canvas_view(ui, &st.app, &mut st.view, ctx, meta, &mut st.intents);
            st.paper_rect = rep.paper_rect_pt;
        },
    );
    canvas_driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    canvas_driver.run_settled();

    let paper = canvas_driver.state().paper_rect.expect("paper rect");
    let vp = canvas_driver.state().viewport_rect.expect("viewport rect");
    let ratio_wide = f64::from(paper.width()) / f64::from(paper.height().max(1.0));
    let expected_ratio = page_pt.width / page_pt.height;
    assert!((ratio_wide - expected_ratio).abs() < 0.03, "expected aspect {expected_ratio}, got {ratio_wide}");
    let left_margin = (paper.left() - vp.left()).round();
    let right_margin = (vp.right() - paper.right()).round();
    assert!((left_margin - right_margin).abs() <= 24.0, "paper should be centered horizontally: {left_margin} vs {right_margin}");

    // 2. Status bar zoom controls on LayoutSheet (`status.zoom.in` / `status.zoom.fit`).
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    harness.assert_parity_and_layout("LayoutSheet status zoom").expect("layout zoom parity");
    harness.click("status.zoom.in").expect("click status.zoom.in");
    assert!(harness.app().canvas_dialogs.canvas.zoom > 1.05, "zoom in must exceed 1.05");
    harness.click("status.zoom.fit").expect("click status.zoom.fit");
    assert!((harness.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6, "zoom fit must reset to 1.0");
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
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    let img = app
        .render_egui_shell_rgba(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
        .expect("render 1M shell");
    assert_eq!(img.width_px, 1280);

    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, metrics);
    let grid_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.cell.0.0.0")
        .map(|e| e.clipped_rect.center())
        .expect("visible grid.cell.0.0.0 in 1M table");

    let mut max_ms = 0.0_f64;
    let mut total_ms = 0.0_f64;
    let frame_count = 24usize;
    for step in 0..frame_count {
        let t0 = Instant::now();
        if step > 0 {
            harness.driver.wheel_at(grid_pos, Vec2::new(0.0, -48.0)).expect("wheel 1M grid");
        } else {
            harness.driver.run_settled();
        }
        let dt_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        total_ms += dt_ms;
        max_ms = max_ms.max(dt_ms);
    }
    let mean_ms = total_ms / (frame_count as f64);
    let mean_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(
        mean_ms <= mean_budget_ms,
        "R10.22 1M-cell grid mean frame budget exceeded: mean={mean_ms:.2}ms (max={max_ms:.2}ms, limit={mean_budget_ms:.2}ms)"
    );
}
