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

fn assert_tier_heights(driver: &EguiHumanPathDriver<'_, TopChromeHarnessState>, label: &str) -> TestRes {
    let rep = driver.state().last_report.as_ref().ok_or_else(|| format!("{label}: missing report"))?;
    let t1 = rep.tier1_rect_pt.ok_or_else(|| format!("{label}: missing tier1"))?;
    let t2 = rep.tier2_rect_pt.ok_or_else(|| format!("{label}: missing tier2"))?;
    assert!((t1.height() - 32.0).abs() <= 1.0 && (t2.height() - 38.0).abs() <= 1.0, "{label}: t1={}, t2={}", t1.height(), t2.height());
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
            assert_tier_heights(&driver, &format!("{locale} wide"))?;
            assert!(driver.frame_meta().find_by_widget_id("toolbar.command_palette").is_some());
            assert!(driver.frame_meta().find_by_widget_id("toolbar.preset").is_none());

            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.5));
            assert_clean_parity(&driver, &format!("{locale} host_menu={host_draws_menu_bar} 640x420@1.5x"))?;
            assert_tier_heights(&driver, &format!("{locale} compact"))?;
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
    for wid in ["toolbar.formula_bar", "toolbar.randomize_groups", "toolbar.sample_rows"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing MV `{wid}`");
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
    for wid in ["toolbar.graph_type", "toolbar.axes", "toolbar.format_graph", "toolbar.export_svg"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "GraphSheet missing `{wid}`");
    }

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
        driver.frame_meta().find_by_widget_id("toolbar.rulers").is_some() && driver.frame_meta().find_by_widget_id("toolbar.canvas_preset").is_some()
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
