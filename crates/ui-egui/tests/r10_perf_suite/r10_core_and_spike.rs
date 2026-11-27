#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R10` Phase 2 Core Infrastructure acceptance suite & interactive `R10.1` ANOVA spike on native `egui`:
//! `R10.1`, `R10.2`, `R10.9`, `R10.15`, `R10.16`, and `R10.22` 1M-cell grid frame budget.

use std::time::Instant;

use crate::common::seed_column_doc;
use crate::human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, default_viewport};
use crate::suite_helpers::{add_init_graph, new_shell_app};
use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, DataSheet, ErrorCode, GraphKind, GroupedTable, LayoutItem, LayoutSheet, LayoutSpec, MultiplicityMethod,
    ProjectDocument, RectPt, ResultCell, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype,
};
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
    let (doc, _) = seed_column_doc(
        "Core Spike",
        "col-spike",
        "Drug Dose Assay",
        ColumnDesignHint::UnpairedReplicates,
        &[("Control", &[10.0, 11.2, 9.8, 10.6, 10.4]), ("Treated", &[14.5, 15.1, 13.9, 14.8, 15.3]), ("HighDose", &[20.1, 21.0, 19.6, 20.8, 21.5])],
    )
    .expect("seed col-spike");
    new_shell_app(doc)
}

fn run_one_way_anova_via_gui(h: &mut EguiWfHarness) {
    h.click("toolbar.analyze").expect("click toolbar.analyze");
    if h.driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_none()
        && let Some(anchor) =
            h.driver.frame_meta().find_by_widget_id("analyze.search").map(|e| Pos2::new(e.clipped_rect.center().x, e.clipped_rect.bottom() + 80.0))
    {
        let _ = h.driver.wheel_at(anchor, Vec2::new(0.0, -180.0));
    }
    h.click("analyze.entry.stats.one_way_anova").expect("click one_way_anova");
    h.click("modal.ok").expect("confirm AnalyzeSheet");
}

#[test]
fn r10_1_spike_analyze_and_edit_params_one_way_anova_human_path() {
    let mut h = EguiWfHarness::from_app(build_three_group_column_app(), default_viewport());
    h.assert_parity_and_layout("DataSheet").expect("DataSheet parity");

    h.click("toolbar.analyze").expect("click toolbar.analyze");
    assert!(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    h.assert_parity_and_layout("AnalyzeSheet").expect("AnalyzeSheet parity");
    if h.driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_none()
        && let Some(anchor) =
            h.driver.frame_meta().find_by_widget_id("analyze.search").map(|e| Pos2::new(e.clipped_rect.center().x, e.clipped_rect.bottom() + 80.0))
    {
        let _ = h.driver.wheel_at(anchor, Vec2::new(0.0, -180.0));
    }
    h.click("analyze.entry.stats.one_way_anova").expect("click one_way_anova");
    h.click("modal.ok").expect("confirm AnalyzeSheet");
    assert_eq!((&h.app().canvas_dialogs.active_dialog, h.app().document().result_sheets.len()), (&ActiveDialog::None, 1));
    let rs_id = h.app().document().result_sheets[0].id.clone();
    assert_eq!(h.app().document().active_sheet_id.as_ref(), Some(&rs_id));
    h.assert_parity_and_layout("ResultSheet").expect("ResultSheet parity");

    // Cmd+T on ResultSheet -> EditAnalysisParameters + backdrop capture + focus trap.
    h.open_cmd_t().expect("Cmd+T");
    h.assert_parity_and_layout("EditAnalysisParameters").expect("EditAnalysisParameters parity");
    let res_before = h.app().document().result_sheets.len();
    h.driver.click_at(Pos2::new(8.0, 20.0)).expect("backdrop click");
    assert!(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    assert_eq!(h.app().document().result_sheets.len(), res_before);
    for mods in [
        KeyModifiers::default(),
        KeyModifiers::default(),
        KeyModifiers::default(),
        KeyModifiers::default(),
        KeyModifiers::default(),
        KeyModifiers { shift: true, ..Default::default() },
    ] {
        h.driver.press_key("Tab", mods).expect("Tab inside modal");
        assert!(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    }

    // Uncheck Assume Gaussian -> R2 disables Tukey + tooltip on hover -> OK switches to Kruskal-Wallis.
    h.click("edit.assume_gaussian").expect("toggle Assume Gaussian off");
    assert!(!h.app().canvas_dialogs.edit_params.assume_gaussian);
    h.click("edit.tab.multiple_comparisons").expect("Multiple Comparisons tab");
    let tukey = h.driver.frame_meta().find_by_widget_id("edit.post_hoc.tukey").cloned().expect("tukey control");
    assert!(!tukey.enabled, "R2 must disable Tukey when nonparametric");
    let expected_prefix: String = tukey.disabled_reason.expect("disabled reason").chars().take(12).collect();
    h.driver.hover_widget_id("edit.post_hoc.tukey").expect("hover disabled tukey");
    let tooltips = h.driver.visible_tooltip_texts();
    assert!(tooltips.iter().any(|t| t.contains("disabled") || t.contains(&expected_prefix)), "got {tooltips:?}");

    h.click("modal.ok").expect("confirm EditAnalysisParameters");
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(h.app().document().result_sheet(&rs_id).is_some_and(|rs| rs.title().contains("Kruskal-Wallis")));
    assert_input_to_paint_budget(&h, 50.0).expect("input-to-paint budget");
}

#[test]
fn r10_2_human_path_only_blocks_synthetic_bypasses() {
    let mut app = build_three_group_column_app();
    app.set_human_path_only(true);
    let cases = [
        ("ui.invoke_menu", json!({ "command_id": "sheet.createData", "params": { "kind": "column", "cols": 3, "rows": 4 } }), 0u8),
        ("ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }), 1),
        ("ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }), 1),
        ("ui.type_text", json!({ "widget_id": "grid.cell.1.1.0", "text": "42" }), 2),
    ];
    for (action, params, prep) in cases {
        if prep == 1 {
            app.canvas_dialogs.open_analyze_dialog(&app.document().clone(), &SheetId::from("col-spike"));
        } else if prep == 2 {
            app.canvas_dialogs.close_dialog();
            app.focus_widget_id = Some("grid.cell.0.0.0".into());
        }
        let err = dispatch_gui_action(&mut app, action, params).expect_err(action);
        assert_eq!((err.code, err.details), (ErrorCode::Conflict, Some(json!({ "code": "human_path_only_policy" }))));
    }
}

#[test]
fn r10_2_and_r10_16_parity_across_views_plus_textbox_ime_clipboard_dropdown_and_scroll() {
    let metrics = default_viewport();
    let mut h = EguiWfHarness::from_app(build_three_group_column_app(), metrics);

    // CommandPalette paste + IME commit + Escape close.
    h.click("toolbar.command_palette").expect("open Command Palette");
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    h.assert_parity_and_layout("CommandPalette").expect("CommandPalette parity");
    h.click("palette.query").expect("focus palette search");
    h.driver.type_text("ANOVA").expect("type ANOVA");
    h.driver.dispatch_events(vec![egui::Event::Paste("A".into())]);
    assert_eq!(h.driver.state().views.command_palette.query, "ANOVAA");
    h.driver.ime_preedit("α", None);
    h.driver.ime_commit("α");
    assert!(h.driver.state().views.command_palette.query.contains('α'));
    h.driver.press_key("Escape", KeyModifiers::default()).expect("Escape");
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    // Click-to-action parity across toolbar & navigator widgets.
    for wid in ["toolbar.analyze", "toolbar.command_palette", "nav.sheet.col-spike"] {
        let mut ch = EguiWfHarness::from_app(build_three_group_column_app(), metrics);
        ch.assert_parity_and_layout(&format!("click parity pre `{wid}`")).expect("pre parity");
        ch.click(wid).unwrap();
        match wid {
            "toolbar.analyze" => assert!(matches!(ch.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. })),
            "toolbar.command_palette" => assert_eq!(ch.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
            _ => assert_eq!(ch.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-spike")),
        }
    }

    // Multi-row ResultSheet (>40 rows, >7 tabs) renders scrollable table without 4-tab truncation cap.
    run_one_way_anova_via_gui(&mut h);
    let mut doc = h.app().document().clone();
    if let Some(rs) = doc.result_sheets.get_mut(0) {
        if let Some(t0) = rs.tabs.get_mut(0) {
            for r in 0..40 {
                t0.row_labels.push(format!("Extra Pair {}", r + 1));
                t0.rows.push(vec![ResultCell::Number { value: 0.001 * (r as f64 + 1.0), prefix: None, units: None }]);
            }
        }
        for i in 0..7 {
            rs.tabs.push(ResultTab {
                id: format!("tab-extra-{i}"),
                title: format!("Tab {i}"),
                kind: ResultTabKind::MainSummary,
                column_headers: vec!["Estimate".into(), "P value".into()],
                row_labels: vec!["R1".into()],
                rows: vec![vec![ResultCell::Number { value: 0.05, prefix: None, units: None }]],
                footnotes: Vec::new(),
            });
        }
    }
    let mut big_h = EguiWfHarness::from_app(new_shell_app(doc), metrics);
    big_h.assert_parity_and_layout("multi-row result sheet").expect("result sheet parity");
    let visible_tabs = big_h.driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("result.tab.")).count();
    let hidden_tabs = big_h
        .driver
        .frame_meta()
        .find_by_widget_id("result.tabs.overflow")
        .and_then(|e| e.label.rsplit_once('+').and_then(|(_, n)| n.trim().parse::<usize>().ok()))
        .unwrap_or(0);
    assert!(visible_tabs >= 4 && visible_tabs + hidden_tabs >= 9, "expected >4 tabs without cap, got visible={visible_tabs} hidden={hidden_tabs}");
    let sec_pos = big_h.widget_center("result.section.0").or_else(|_| big_h.widget_center("result.cell.0.0")).expect("scrollable result widget");
    big_h.driver.wheel_at(sec_pos, Vec2::new(0.0, -240.0)).expect("scroll result table");
    assert!(big_h.driver.verify_pixel_to_hit_box_parity().is_clean());

    // EditAnalysisParameters -> Multiple Comparisons -> Bonferroni.
    h.open_cmd_t().expect("open edit params");
    h.click("edit.tab.multiple_comparisons").expect("mc tab");
    h.assert_parity_and_layout("multiple comparisons tab").expect("mc parity");
    h.click("edit.post_hoc.bonferroni").expect("Bonferroni");
    h.click("modal.ok").expect("confirm");
    let AnalysisKind::OneWayAnova(spec) = &h.app().document().result_sheets.last().expect("rs").analysis.kind else {
        panic!("expected OneWayAnova");
    };
    assert_eq!(spec.multiplicity.method, MultiplicityMethod::Bonferroni);
}

#[test]
fn r10_9_text_measurer_and_coverage_atlas_retire_font_5x7() {
    let measurer = DesktopTextMeasurer::global();
    let sans = UiTextStyle::ui_sans(14.0);
    assert!(measurer.measure("W", &sans, "en").width_px > measurer.measure("i", &sans, "en").width_px * 1.2);
    let tab = UiTextStyle::grid_tabular(13.0);
    assert!((measurer.measure("11111", &tab, "en").width_px - measurer.measure("88888", &tab, "en").width_px).abs() < 0.05);
    assert!(measurer.shape_line("IC50 α±μ 10⁻⁶", &sans, "en").metrics.width_px > 20.0);

    let (mut app_a, mut app_b) = (build_three_group_column_app(), build_three_group_column_app());
    let ds_id = app_a.session().document().data_sheets[0].id.as_str().to_owned();
    let _ = app_a.execute_command("table.setCell", json!({ "sheet_id": ds_id, "row": 0, "col": 0, "subcol": 0, "value": "AbcXy" }));
    let _ = app_b.execute_command("table.setCell", json!({ "sheet_id": ds_id, "row": 0, "col": 0, "subcol": 0, "value": "ABCXY" }));
    let target = RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 };
    assert_ne!(
        app_a.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba,
        app_b.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba
    );
}

#[test]
fn r10_15_aspect_ratio_viewport_and_hover_selected_only_chrome() {
    let mut doc = build_three_group_column_app().document().clone();
    let ds_id = doc.data_sheets[0].id.clone();
    let gid = add_init_graph(&mut doc, &ds_id, "g-ar", GraphKind::Column).expect("graph");
    let ls_spec = LayoutSpec {
        items: vec![LayoutItem::LiveGraph {
            id: "p0".into(),
            graph_sheet_id: gid,
            bounds_pt: RectPt::new(36.0, 36.0, 300.0, 220.0),
            scale_percent: 100.0,
            badge_override: None,
        }],
        ..LayoutSpec::default()
    };
    let page_pt = ls_spec.page_size_pt();
    doc.add_layout_sheet(LayoutSheet::new("l-ar", "Page Layout", ls_spec).expect("layout")).expect("add layout");
    doc.select_in_navigator(&SheetId::from("l-ar")).expect("select layout");
    let app = new_shell_app(doc);

    struct CanvasState {
        app: UiApp,
        view: CanvasViewEguiState,
        intents: PendingUiIntents,
        paper: Option<egui::Rect>,
        vp: Option<egui::Rect>,
    }
    let init = CanvasState { app: app.clone(), view: CanvasViewEguiState::default(), intents: PendingUiIntents::new(), paper: None, vp: None };
    let mut cd = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
        ViewportMetrics::new(1000.0, 600.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            st.vp = Some(ui.max_rect());
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false };
            st.paper = render_canvas_view(ui, &st.app, &mut st.view, ctx, meta, &mut st.intents).paper_rect_pt;
        },
    );
    cd.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    cd.run_settled();

    let (paper, vp) = (cd.state().paper.expect("paper rect"), cd.state().vp.expect("vp rect"));
    assert!((f64::from(paper.width()) / f64::from(paper.height().max(1.0)) - page_pt.width / page_pt.height).abs() < 0.03);
    assert!(((paper.left() - vp.left()).round() - (vp.right() - paper.right()).round()).abs() <= 24.0);

    let mut h = EguiWfHarness::from_app(app, default_viewport());
    h.assert_parity_and_layout("LayoutSheet status zoom").expect("layout zoom parity");
    h.click("status.zoom.in").expect("zoom in");
    assert!(h.app().canvas_dialogs.canvas.zoom > 1.05);
    h.click("status.zoom.fit").expect("zoom fit");
    assert!((h.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);
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
    doc.add_data_sheet(DataSheet::new("tbl-1m", "Mega Assay 1M", TableArchetype::Grouped(grouped)).expect("sheet")).expect("add");
    doc.select_in_navigator(&SheetId::from("tbl-1m")).expect("select");

    let mut app = new_shell_app(doc);
    let img = app
        .render_egui_shell_rgba(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
        .expect("render 1M");
    assert_eq!(img.width_px, 1280);

    let mut h = EguiWfHarness::from_app(app, default_viewport());
    let grid_pos = h.widget_center("grid.cell.0.0.0").expect("visible grid.cell.0.0.0");
    let (mut max_ms, mut total_ms, frame_count) = (0.0_f64, 0.0_f64, 24usize);
    for step in 0..frame_count {
        let t0 = Instant::now();
        if step > 0 {
            h.driver.wheel_at(grid_pos, Vec2::new(0.0, -48.0)).expect("wheel 1M grid");
        } else {
            h.driver.run_settled();
        }
        let dt_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        total_ms += dt_ms;
        max_ms = max_ms.max(dt_ms);
    }
    let mean_ms = total_ms / (frame_count as f64);
    let mean_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(
        mean_ms <= mean_budget_ms,
        "R10.22 1M-cell grid mean budget exceeded: mean={mean_ms:.2}ms (max={max_ms:.2}ms, limit={mean_budget_ms:.2}ms)"
    );
}
