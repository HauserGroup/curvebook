#![forbid(unsafe_code)]
//! `R15.3` Two-Tier Top Chrome, 14-Locale Zero-Bleed Responsive Layout, Split/Workspace Presets,
//! and `Cmd/Ctrl+K` Command Palette Acceptance Tests.

use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::{ShellAboutFrameReport, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, WorkspacePreset};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use egui::{Key, Modifiers};
use serde_json::json;

use crate::common::{TestRes, assert_clean_parity};

struct TopChromeHarnessState {
    app: UiApp,
    last_report: Option<ShellAboutFrameReport>,
    discard_next_frame: bool,
    discard_passes: usize,
    settled_passes: usize,
    wheel_capture_count: usize,
}

impl TopChromeHarnessState {
    fn new(locale: &str, host_draws_menu_bar: bool) -> Self {
        let mut app = UiApp::new();
        app.host_draws_menu_bar = host_draws_menu_bar;
        assert_eq!(app.set_locale(locale), locale);
        Self { app, last_report: None, discard_next_frame: false, discard_passes: 0, settled_passes: 0, wheel_capture_count: 0 }
    }
}

fn render_top_chrome_harness(ui: &mut egui::Ui, meta: &mut FrameInspectMeta, state: &mut TopChromeHarnessState) {
    if state.discard_next_frame {
        state.discard_next_frame = false;
        ui.ctx().request_discard("r15_3 multi-pass layout");
    }
    let mut intents = PendingUiIntents::new();
    let mut views = std::mem::take(&mut state.app.egui_views);
    let report = render_egui_shell_frame(ui, &state.app, &mut views, meta, &mut intents);
    state.app.egui_views = views;
    if report.outside_wheel_captured {
        state.wheel_capture_count += 1;
    }
    state.last_report = Some(report);
    let will_discard = ui.ctx().will_discard();
    if will_discard {
        state.discard_passes += 1;
    } else {
        state.settled_passes += 1;
    }
    let _ = intents.commit_if_settled(&mut state.app, will_discard);
}

fn make_top_driver(state: TopChromeHarnessState, metrics: ViewportMetrics, locale: &str) -> EguiHumanPathDriver<'static, TopChromeHarnessState> {
    let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(state, metrics, tokens, build_driver_font_definitions(), render_top_chrome_harness);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver
}

fn seed_xy_graph(state: &mut TopChromeHarnessState, ds: &str, gs: &str, rows: usize) -> TestRes {
    state
        .app
        .execute_command("sheet.createData", json!({ "sheet_id": ds, "title": "Assay Data", "kind": "xy", "cols": 3, "rows": rows }))
        .map_err(|e| e.to_string())?;
    state.app.execute_command("graph.create", json!({ "sheet_id": gs, "data_sheet_id": ds, "title": "Dose Curve" })).map_err(|e| e.to_string())?;
    let _ = state.app.execute_command("sheet.select", json!({ "sheet_id": ds }));
    Ok(())
}

fn assert_tier_heights(driver: &EguiHumanPathDriver<'_, TopChromeHarnessState>, host_draws_menu_bar: bool, label: &str) -> TestRes {
    let rep = driver.state().last_report.as_ref().ok_or_else(|| format!("{label}: missing report"))?;
    let t2 = rep.tier2_rect_pt.ok_or_else(|| format!("{label}: missing tier2"))?;
    assert!((t2.height() - 38.0).abs() <= 1.0, "{label}: expected 38pt unified toolbar, got t2={}", t2.height());
    if host_draws_menu_bar {
        assert!(rep.tier1_rect_pt.is_none(), "{label}: expected tier1 collapsed when host_draws_menu_bar=true, got {:?}", rep.tier1_rect_pt);
    } else {
        let t1 = rep.tier1_rect_pt.ok_or_else(|| format!("{label}: missing tier1"))?;
        assert!((t1.height() - 28.0).abs() <= 1.0, "{label}: expected 28pt menu bar, got t1={}", t1.height());
    }
    Ok(())
}

#[test]
fn top_chrome_all_14_beta_locales_zero_bleed_640x420_and_1440x600() -> TestRes {
    for &locale in &BETA_LOCALE_CODES {
        for &host_draws_menu_bar in &[false, true] {
            let mut state = TopChromeHarnessState::new(locale, host_draws_menu_bar);
            seed_xy_graph(&mut state, "ds-1", "gs-1", 8)?;
            let mut driver = make_top_driver(state, ViewportMetrics::new(1440.0, 600.0, 1.0), locale);

            assert_clean_parity(&driver, &format!("{locale} host_menu={host_draws_menu_bar} 1440x600"))?;
            assert_tier_heights(&driver, host_draws_menu_bar, &format!("{locale} host_menu={host_draws_menu_bar} wide"))?;
            assert!(driver.frame_meta().find_by_widget_id("toolbar.command_palette").is_some());
            assert!(driver.frame_meta().find_by_widget_id("toolbar.preset").is_none());
            if host_draws_menu_bar {
                assert!(driver.frame_meta().find_by_widget_id("toolbar.project_title").is_none(), "{locale}: duplicate title pill");
                assert!(!driver.frame_meta().entries.iter().any(|e| e.widget_id.starts_with("menu.")));
            }

            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.5));
            assert_clean_parity(&driver, &format!("{locale} host_menu={host_draws_menu_bar} 640x420@1.5x"))?;
            assert_tier_heights(&driver, host_draws_menu_bar, &format!("{locale} host_menu={host_draws_menu_bar} compact"))?;
            for required in ["toolbar.home", "toolbar.analyze", "toolbar.command_palette", "toolbar.split"] {
                assert!(
                    driver.frame_meta().find_by_widget_id(required).is_some(),
                    "{locale} host_menu={host_draws_menu_bar} 640x420 missing `{required}`"
                );
            }
            for entry in &driver.frame_meta().entries {
                if entry.widget_id.starts_with("toolbar.") || entry.widget_id.starts_with("menu.") {
                    assert!(entry.clipped_rect.right() <= 640.5, "{locale} `{}` overflowed: {:?}", entry.widget_id, entry.clipped_rect);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn r20_18_and_r20_9_logo_home_button_and_icon_only_undo_redo_tooltips() -> TestRes {
    for &host_draws_menu_bar in &[false, true] {
        let mut state = TopChromeHarnessState::new("de", host_draws_menu_bar);
        seed_xy_graph(&mut state, "ds-logo", "gs-logo", 4)?;
        let mut driver = make_top_driver(state, ViewportMetrics::new(1280.0, 720.0, 1.0), "de");

        let home = driver.frame_meta().find_by_widget_id("toolbar.home").ok_or("missing toolbar.home")?.clone();
        assert_eq!(home.label, "Start");
        assert!(home.clipped_rect.width() >= 36.0 && home.clipped_rect.height() >= 24.0, "bad toolbar.home rect: {:?}", home.clipped_rect);
        let top_y = if host_draws_menu_bar { 0.0 } else { 28.0 };
        assert!((home.clipped_rect.top() - top_y).abs() <= 8.0, "host_menu={host_draws_menu_bar}: home={:?}", home.clipped_rect);

        driver.hover_widget_id("toolbar.home")?;
        let home_tips = driver.visible_tooltip_texts();
        assert!(home_tips.iter().any(|t| t == "Start"), "missing Start tooltip on toolbar.home: {home_tips:?}");

        let undo = driver.frame_meta().find_by_widget_id("toolbar.undo").ok_or("missing toolbar.undo")?.clone();
        let redo = driver.frame_meta().find_by_widget_id("toolbar.redo").ok_or("missing toolbar.redo")?.clone();
        assert!((undo.clipped_rect.width() - 28.0).abs() <= 1.0 && (undo.clipped_rect.height() - 28.0).abs() <= 1.0, "{undo:?}");
        assert!((redo.clipped_rect.width() - 28.0).abs() <= 1.0 && (redo.clipped_rect.height() - 28.0).abs() <= 1.0, "{redo:?}");
        assert_eq!(undo.label, curvebook_ui_egui::tr("de", "toolbar.undo"));
        assert_eq!(redo.label, curvebook_ui_egui::tr("de", "toolbar.redo"));

        driver.hover_widget_id("toolbar.undo")?;
        let undo_tips = driver.visible_tooltip_texts();
        let expected_undo_sc = if cfg!(target_os = "macos") { "⌘Z" } else { "Ctrl+Z" };
        assert!(undo_tips.iter().any(|t| t.contains(expected_undo_sc)), "missing {expected_undo_sc} in undo tooltip: {undo_tips:?}");

        driver.hover_widget_id("toolbar.redo")?;
        let redo_tips = driver.visible_tooltip_texts();
        let expected_redo_sc = if cfg!(target_os = "macos") { "⇧⌘Z" } else { "Ctrl+Shift+Z" };
        assert!(redo_tips.iter().any(|t| t.contains(expected_redo_sc)), "missing {expected_redo_sc} in redo tooltip: {redo_tips:?}");

        driver.click_widget_id("toolbar.home")?;
        assert!(driver.state().app.egui_views.launcher_open);
        driver.click_widget_id("toolbar.home")?;
        assert!(!driver.state().app.egui_views.launcher_open);

        // Verify post-click hover after realistic >=100 ms pointer travel opens enabled Undo & Redo tooltips.
        driver
            .state_mut()
            .app
            .execute_command("table.setCell", json!({ "sheet_id": "ds-logo", "row": 0, "col": 0, "value": 42.0 }))
            .map_err(|e| e.to_string())?;
        for _ in 0..8 {
            driver.step_single_frame();
        }
        driver.run_settled();
        assert!(driver.frame_meta().find_by_widget_id("toolbar.undo").is_some_and(|e| e.enabled));
        driver.hover_widget_id("toolbar.undo")?;
        let enabled_undo_tips = driver.visible_tooltip_texts();
        assert!(
            enabled_undo_tips.iter().any(|t| t.contains(expected_undo_sc)),
            "missing {expected_undo_sc} in enabled undo tooltip: {enabled_undo_tips:?}"
        );
        driver.click_widget_id("toolbar.undo")?;
        for _ in 0..8 {
            driver.step_single_frame();
        }
        driver.run_settled();
        assert!(driver.frame_meta().find_by_widget_id("toolbar.redo").is_some_and(|e| e.enabled));
        driver.hover_widget_id("toolbar.redo")?;
        let enabled_redo_tips = driver.visible_tooltip_texts();
        assert!(
            enabled_redo_tips.iter().any(|t| t.contains(expected_redo_sc)),
            "missing {expected_redo_sc} in enabled redo tooltip: {enabled_redo_tips:?}"
        );
    }
    Ok(())
}

#[test]
fn top_chrome_tier2_adaptive_strips_all_sheet_contexts_and_overflow_menu() -> TestRes {
    let mut driver = make_top_driver(TopChromeHarnessState::new("en", true), ViewportMetrics::new(1280.0, 720.0, 1.0), "en");

    assert!(driver.frame_meta().find_by_widget_id("toolbar.new").is_some() && driver.frame_meta().find_by_widget_id("toolbar.open").is_some());
    assert!(!driver.frame_meta().find_by_widget_id("toolbar.analyze").ok_or("missing toolbar.analyze")?.enabled);

    driver
        .state_mut()
        .app
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "title": "Column Controls", "kind": "column", "cols": 3, "rows": 6 }))
        .map_err(|e| e.to_string())?;
    for col in 0..3 {
        for row in 0..4 {
            driver
                .state_mut()
                .app
                .execute_command(
                    "table.setCell",
                    json!({ "sheet_id": "ds-col", "row": row, "col": col, "value": 10.0 + col as f64 + row as f64 * 0.5 }),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("toolbar.analyze").is_some_and(|e| e.enabled));
    assert!(driver.frame_meta().find_by_widget_id("toolbar.new_graph").is_some_and(|e| e.enabled));

    driver
        .state_mut()
        .app
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-mv", "title": "Cohort MV", "kind": "multiple_variables", "cols": 3, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("toolbar.analyze").is_some(), "missing MV `toolbar.analyze`");
    assert!(driver.frame_meta().find_by_widget_id("toolbar.formula_bar").is_some(), "missing MV `toolbar.formula_bar`");
    for removed in ["toolbar.randomize_groups", "toolbar.sample_rows"] {
        assert!(driver.frame_meta().find_by_widget_id(removed).is_none(), "MV toolbar must not expose `{removed}`");
    }

    let rs_out = driver
        .state_mut()
        .app
        .execute_command("analysis.run", json!({ "source_sheet_id": "ds-col", "kind": "one_way_anova" }))
        .map_err(|e| e.to_string())?;
    let rs_id = rs_out.payload["sheet_id"].as_str().ok_or("missing result sheet_id")?.to_owned();
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": rs_id })).map_err(|e| e.to_string())?;
    driver.run_settled();
    for wid in ["toolbar.edit_params", "toolbar.change_data", "toolbar.methods_report"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some_and(|e| e.enabled), "ResultSheet `{wid}` disabled");
    }

    driver.state_mut().app.execute_command("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "gs-col" })).map_err(|e| e.to_string())?;
    driver.run_settled();
    for wid in ["toolbar.graph_type", "toolbar.axes", "toolbar.format_graph", "toolbar.export_figure"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "GraphSheet missing `{wid}`");
    }
    assert!(driver.frame_meta().find_by_widget_id("toolbar.export_svg").is_none());

    if let Some(info_id) = driver.state().app.document().info_sheets.first().map(|s| s.id.as_str().to_owned()) {
        let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": info_id }));
        driver.run_settled();
        assert!(driver.frame_meta().find_by_widget_id("toolbar.info_add_constant").is_some_and(|e| e.enabled));
        let ng = driver.frame_meta().find_by_widget_id("toolbar.new_graph").ok_or("InfoSheet toolbar.new_graph")?;
        assert!(ng.command_id.as_deref() == Some("graph.create") && ng.enabled);
        let graphs_before = driver.state().app.document().graph_sheets.len();
        driver.click_widget_id("toolbar.new_graph")?;
        assert_eq!(driver.state().app.document().graph_sheets.len(), graphs_before + 1);
        let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": info_id }));
        driver.run_settled();
        driver.click_widget_id("toolbar.info_add_constant")?;
    }

    driver.state_mut().app.execute_command("layout.create", json!({ "sheet_id": "ls-1", "title": "Figure 1" })).map_err(|e| e.to_string())?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ls-1" })).map_err(|e| e.to_string())?;
    driver.run_settled();
    let composer_btn = driver.frame_meta().find_by_widget_id("toolbar.layout_composer").ok_or("LayoutSheet composer")?;
    assert_eq!(composer_btn.command_id.as_deref(), Some("ui.layout.composer.open"));
    assert_eq!(composer_btn.label, curvebook_ui_egui::tr("en", "cmd.ui.layout.composer.open"));
    assert!(
        driver.frame_meta().find_by_widget_id("toolbar.rulers").is_some()
            && driver.frame_meta().find_by_widget_id("toolbar.canvas_preset").is_some()
            && driver.frame_meta().find_by_widget_id("toolbar.export_figure").is_some()
    );

    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" }));
    driver.run_settled();
    if driver.frame_meta().find_by_widget_id("toolbar.overflow").is_some() {
        driver.click_widget_id("toolbar.overflow")?;
        assert_eq!(driver.frame_meta().find_by_widget_id("toolbar.overflow.item.0").ok_or("overflow popup")?.z_layer, ZLayer::DropdownMenu);
    }
    assert!(driver.frame_meta().find_by_widget_id("sheet.tab.ds-col").is_none());
    Ok(())
}

#[test]
fn top_chrome_split_presets_and_workspace_presets() -> TestRes {
    let mut state = TopChromeHarnessState::new("en", false);
    seed_xy_graph(&mut state, "ds-s1", "gs-s1", 5)?;
    let mut driver = make_top_driver(state, ViewportMetrics::new(1440.0, 600.0, 1.0), "en");

    assert!(driver.frame_meta().find_by_widget_id("toolbar.preset").is_none());
    driver.click_widget_id("menu.view")?;
    driver.hover_widget_id("menu.ui.workspacePreset.set")?;
    for opt in ["default", "data_prep", "modeling", "publication", "compact"] {
        let wid = format!("popup.menu.ui.workspacePreset.set.opt.{opt}");
        assert_eq!(driver.frame_meta().find_by_widget_id(&wid).ok_or_else(|| format!("missing `{wid}`"))?.z_layer, ZLayer::DropdownMenu);
    }
    driver.click_widget_id("popup.menu.ui.workspacePreset.set.opt.publication")?;
    assert_eq!(driver.state().app.egui_views.workspace_preset, WorkspacePreset::Publication);

    driver.click_widget_id("menu.view")?;
    driver.hover_widget_id("menu.ui.workspacePreset.set")?;
    driver.click_widget_id("popup.menu.ui.workspacePreset.set.opt.compact")?;
    assert_eq!(driver.state().app.egui_views.workspace_preset, WorkspacePreset::Compact);

    driver.click_widget_id("toolbar.split")?;
    let dg = driver.frame_meta().find_by_widget_id("toolbar.split.preset.data_graph").ok_or("missing data_graph")?;
    assert!(dg.enabled && dg.z_layer == ZLayer::DropdownMenu);
    driver.click_widget_id("toolbar.split.preset.data_graph")?;
    assert!(driver.state().app.split.enabled);
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(|s| s.as_str()), Some("ds-s1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(|s| s.as_str()), Some("gs-s1"));

    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.swap")?;
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(|s| s.as_str()), Some("gs-s1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(|s| s.as_str()), Some("ds-s1"));

    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::Backslash, cmd_mod);
    assert!(!driver.state().app.split.enabled);
    driver.press_egui_key(Key::Backslash, cmd_mod);
    assert!(driver.state().app.split.enabled);
    Ok(())
}

#[test]
fn command_palette_keyboard_nav_fuzzy_search_backdrop_capture_and_discard_guard() -> TestRes {
    let mut driver = make_top_driver(TopChromeHarnessState::new("en", true), ViewportMetrics::new(640.0, 420.0, 1.0), "en");

    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::K, cmd_mod);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    assert_clean_parity(&driver, "Command Palette 640x420")?;

    let backdrop = driver.frame_meta().find_by_widget_id("modal.backdrop").ok_or("missing modal.backdrop")?;
    assert_eq!(backdrop.z_layer, ZLayer::ModalBackdrop);
    assert!((backdrop.clipped_rect.width() - 640.0).abs() <= 1.0 && (backdrop.clipped_rect.height() - 420.0).abs() <= 1.0);
    let dialog = driver.frame_meta().find_by_widget_id("palette.dialog").ok_or("missing palette.dialog")?;
    assert!(dialog.z_layer == ZLayer::ModalDialog && dialog.clipped_rect.bottom() <= 420.0);

    let wheel_before = driver.state().wheel_capture_count;
    driver.wheel_at(egui::pos2(12.0, 210.0), egui::vec2(0.0, -40.0))?;
    assert!(driver.state().wheel_capture_count > wheel_before);

    driver.type_text("Change Analysis Parameters")?;
    let disabled_item = driver.frame_meta().find_by_widget_id("palette.item.0").ok_or("missing palette.item.0")?;
    assert!(!disabled_item.enabled && disabled_item.disabled_reason.as_deref() == Some("Open a ResultSheet first"));

    driver.click_at(egui::pos2(12.0, 210.0))?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.click_widget_id("toolbar.command_palette")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    driver.type_text("New")?;
    let before_down = driver.state().app.egui_views.command_palette.selected_index;
    driver.press_egui_key(Key::ArrowDown, Modifiers::NONE);
    assert_ne!(driver.state().app.egui_views.command_palette.selected_index, before_down);
    driver.press_egui_key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(driver.state().app.egui_views.command_palette.selected_index, before_down);

    driver.state_mut().app.egui_views.command_palette.query = String::from("New Data Table");
    driver.run_settled();
    driver.state_mut().discard_next_frame = true;
    driver.state_mut().discard_passes = 0;
    driver.state_mut().settled_passes = 0;
    let (sheets_before, undo_before) = (driver.state().app.document().data_sheets.len(), driver.state().app.session().undo_depth());
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert!(driver.state().discard_passes >= 1);
    assert_eq!((driver.state().app.document().data_sheets.len(), driver.state().app.session().undo_depth()), (sheets_before, undo_before));

    driver.run_settled();
    assert_eq!(driver.state().app.document().data_sheets.len(), sheets_before);

    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert_eq!((driver.state().app.document().data_sheets.len(), driver.state().app.session().undo_depth()), (sheets_before + 1, undo_before + 1));
    Ok(())
}

#[test]
fn da_05_06_top_chrome_popups_dismiss_on_escape() -> TestRes {
    let mut state = TopChromeHarnessState::new("en", true);
    seed_xy_graph(&mut state, "ds-esc", "gs-esc", 4)?;
    let mut driver = make_top_driver(state, ViewportMetrics::new(1440.0, 600.0, 1.0), "en");

    driver.click_widget_id("toolbar.split")?;
    assert!(driver.frame_meta().find_by_widget_id("toolbar.split.preset.data_graph").is_some());
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("toolbar.split.preset.data_graph").is_none());

    driver.set_viewport(ViewportMetrics::new(420.0, 420.0, 1.0));
    driver.click_widget_id("toolbar.overflow")?;
    assert!(driver.frame_meta().find_by_widget_id("toolbar.overflow.item.0").is_some());
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("toolbar.overflow.item.0").is_none());
    Ok(())
}

#[test]
fn aud_07_23_08_14_18_and_09_26_44_history_top_bar_fit_search_recents_and_palette() -> TestRes {
    use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, InfoSheet, ProjectDocument, SheetId, TableArchetype};
    use curvebook_engine::Session;
    use curvebook_ui_egui::egui_shell::EguiShellViewState;
    use curvebook_ui_egui::egui_shell::command_palette::filter_palette_commands;
    use curvebook_ui_egui::egui_shell::top_bar_fit::{RightWidths, SearchVisibility, fit_right_group};
    use curvebook_ui_egui::is_desktop_host_routed_command;
    use curvebook_ui_egui::search_engine::{SearchRecents, UnifiedSearchAction, UnifiedSearchItem, UnifiedSearchKind, query_unified};

    // AUD-08-14: sync_history_with_document adjusts cursor on predecessor deletion & preserves forward history.
    let mut doc_h = ProjectDocument::new("Hist").map_err(|e| e.to_string())?;
    for id in ["s1", "s2", "s3", "s4"] {
        doc_h
            .data_sheets
            .push(DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap())).unwrap());
    }
    let mut views = EguiShellViewState::default();
    for id in ["s1", "s2", "s3", "s4"] {
        views.record_active_sheet_visit(&SheetId::new(id));
    }
    views.history_cursor = 2;
    doc_h.data_sheets.retain(|ds| ds.id.as_str() != "s1");
    doc_h.active_sheet_id = Some(SheetId::new("s3"));
    views.sync_history_with_document(&doc_h);
    assert_eq!(
        (&views.active_sheet_history, views.history_cursor, views.can_go_forward()),
        (&vec![SheetId::new("s2"), SheetId::new("s3"), SheetId::new("s4")], 1, true)
    );

    // AUD-08-18: fit_right_group with switcher == 0 reserves zero phantom width.
    let tight = fit_right_group(&RightWidths {
        room: 180.0,
        has_pinned_badge: false,
        switcher: 0.0,
        field: 176.0,
        icon: 48.0,
        gap: 6.0,
        trailing_extra: 56.0,
    });
    assert!(!tight.switcher && tight.switcher_w == 0.0 && tight.search == SearchVisibility::Field);

    // AUD-07-23: search_engine evaluates live enabled state for Recent RunCommand entries.
    let mut recents = SearchRecents::default();
    recents.record(&UnifiedSearchItem {
        kind: UnifiedSearchKind::Command,
        title: "Undo".into(),
        subtitle: "Edit".into(),
        category_badge: "Command".into(),
        shortcut: "Cmd+Z".into(),
        match_spans: Vec::new(),
        enabled: true,
        score: 100,
        action: UnifiedSearchAction::RunCommand { command_id: "edit.undo".into(), is_ui_command: false },
        completion_text: "Undo".into(),
    });
    assert!(
        query_unified(&UiApp::new(), "", 20, &recents)
            .iter()
            .any(|it| matches!(&it.action, UnifiedSearchAction::RunCommand { command_id, .. } if command_id == "edit.undo") && !it.enabled)
    );

    // AUD-09-26 & AUD-09-44: desktop host routing & ui.sheet.find hidden on InfoSheet.
    for cmd in ["table.importPrism", "table.importPzfx", "table.importXlsx"] {
        assert!(is_desktop_host_routed_command(cmd));
    }
    let mut doc_cp = ProjectDocument::new("Palette").map_err(|e| e.to_string())?;
    doc_cp.info_sheets.push(InfoSheet::new_preseeded("info-1", "Info").map_err(|e| e.to_string())?);
    let mut app = UiApp::from_session(Session::from_document(doc_cp));
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    assert!(filter_palette_commands(&app, "ui.sheet.find", 24).iter().all(|r| r.command_id != "ui.sheet.find"));
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    assert!(filter_palette_commands(&app, "ui.sheet.find", 24).iter().any(|r| r.command_id == "ui.sheet.find"));
    Ok(())
}

#[test]
fn r20_21_command_tooltips_render_summary_only_and_preserve_beginner_in_metadata() -> TestRes {
    let mut state = TopChromeHarnessState::new("en", false);
    seed_xy_graph(&mut state, "ds-r2021", "gs-r2021", 4)?;
    let mut driver = make_top_driver(state, ViewportMetrics::new(1440.0, 600.0, 1.0), "en");

    // 1. Metadata in menus/tooltips.rs must keep `.beginner` intact for MCP / docs / palette.
    let palette_tip = curvebook_ui_egui::menus::command_tooltip(&driver.state().app, "ui.palette.open").ok_or("missing ui.palette.open")?;
    let new_tip = curvebook_ui_egui::menus::command_tooltip(&driver.state().app, "sheet.createData").ok_or("missing sheet.createData")?;
    assert!(!palette_tip.beginner.is_empty() && palette_tip.format_tooltip().contains(palette_tip.beginner));
    assert!(!new_tip.beginner.is_empty() && new_tip.format_tooltip().contains(new_tip.beginner));

    // 2. Hovering toolbar buttons renders CommandTooltip.summary + platform shortcut pill, NEVER .beginner.
    driver.hover_widget_id("toolbar.command_palette")?;
    let pal_tips = driver.visible_tooltip_texts();
    assert!(pal_tips.iter().any(|t| t == palette_tip.summary), "expected summary in toolbar.command_palette tooltip: {pal_tips:?}");
    let expected_k = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+K" };
    assert!(pal_tips.iter().any(|t| t.contains(expected_k)), "expected {expected_k} shortcut pill: {pal_tips:?}");
    assert!(!pal_tips.iter().any(|t| t.contains(palette_tip.beginner)), "GUI tooltip must not render .beginner: {pal_tips:?}");

    driver.hover_widget_id("toolbar.new")?;
    let new_tips = driver.visible_tooltip_texts();
    assert!(new_tips.iter().any(|t| t == new_tip.summary), "expected summary on toolbar.new: {new_tips:?}");
    assert!(!new_tips.iter().any(|t| t.contains(new_tip.beginner)), "toolbar.new must not render .beginner: {new_tips:?}");

    // 3. Hovering in-window menu item leaves surfaces .summary without .beginner.
    let layout_tip = curvebook_ui_egui::menus::command_tooltip(&driver.state().app, "layout.create").ok_or("missing layout.create")?;
    assert!(!layout_tip.beginner.is_empty());
    driver.click_widget_id("menu.insert")?;
    driver.hover_widget_id("menu.layout.create")?;
    let leaf_tips = driver.visible_tooltip_texts();
    assert!(leaf_tips.iter().any(|t| t == layout_tip.summary), "expected summary on menu.layout.create: {leaf_tips:?}");
    assert!(!leaf_tips.iter().any(|t| t.contains(layout_tip.beginner)), "menu leaf must not render .beginner: {leaf_tips:?}");

    driver.hover_widget_id("menu.sheet.createData")?;
    let menu_tips = driver.visible_tooltip_texts();
    assert!(menu_tips.iter().any(|t| t == new_tip.summary), "expected summary on menu.sheet.createData: {menu_tips:?}");
    assert!(!menu_tips.iter().any(|t| t.contains(new_tip.beginner)), "menu leaf must not render .beginner: {menu_tips:?}");
    Ok(())
}
