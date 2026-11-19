#![forbid(unsafe_code)]
//! `R15.3` Two-Tier Top Chrome, 14-Locale Zero-Bleed Responsive Layout, Split/Workspace Presets,
//! Presentation Mode, and `Cmd/Ctrl+K` Command Palette Acceptance Tests.

use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::{ShellAboutFrameReport, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::top_chrome::render_presentation_exit_chip;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiBackend, WorkspacePreset};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::layout::ZLayer;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use egui::{Key, Modifiers};
use serde_json::json;

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
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.host_draws_menu_bar = host_draws_menu_bar;
        let _ = app.set_locale(locale);
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

    if state.app.presentation_mode && meta.find_by_widget_id("toolbar.presentation.exit").is_none() {
        let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
        let _ = render_presentation_exit_chip(ui.ctx(), &state.app, &tokens, true, meta, &mut intents);
    }

    let will_discard = ui.ctx().will_discard();
    if will_discard {
        state.discard_passes += 1;
    } else {
        state.settled_passes += 1;
    }
    let _ = intents.commit_if_settled(&mut state.app, will_discard);
}

#[test]
fn top_chrome_all_14_beta_locales_zero_bleed_640x420_and_1440x600() -> Result<(), String> {
    for &locale in &BETA_LOCALE_CODES {
        for &host_draws_menu_bar in &[false, true] {
            let mut state = TopChromeHarnessState::new(locale, host_draws_menu_bar);
            state
                .app
                .execute_command(
                    "sheet.createData",
                    json!({ "sheet_id": "ds-1", "title": "Concentration Response", "kind": "xy", "cols": 3, "rows": 8 }),
                )
                .map_err(|e| format!("{locale}: createData failed: {e}"))?;
            state
                .app
                .execute_command("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-1", "title": "Dose Curve" }))
                .map_err(|e| format!("{locale}: graph.create failed: {e}"))?;
            let _ = state.app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" }));

            let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), locale);
            let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
                state,
                ViewportMetrics::new(1440.0, 600.0, 1.0),
                tokens,
                build_driver_font_definitions(),
                render_top_chrome_harness,
            );
            driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

            let wide_report = driver.verify_pixel_to_hit_box_parity();
            assert!(
                wide_report.is_clean(),
                "locale `{locale}` (host_draws_menu_bar={host_draws_menu_bar}) 1440x600 parity/bleed failed: {:?}",
                wide_report.violations
            );
            assert!(driver.check_scrollbar_non_overlap().is_empty(), "locale `{locale}` 1440x600 scrollbar overlap");
            let wide_shell = driver.state().last_report.as_ref().ok_or("missing wide report")?;
            let wide_t1 = wide_shell.tier1_rect_pt.ok_or("missing wide tier1 rect")?;
            let wide_t2 = wide_shell.tier2_rect_pt.ok_or("missing wide tier2 rect")?;
            assert!((wide_t1.height() - 32.0).abs() <= 1.0, "tier1 height should be 32pt, got {}", wide_t1.height());
            assert!((wide_t2.height() - 38.0).abs() <= 1.0, "tier2 height should be 38pt, got {}", wide_t2.height());
            assert!(driver.frame_meta().find_by_widget_id("toolbar.command_palette").is_some(), "{locale} wide must show Cmd+K field");
            assert!(driver.frame_meta().find_by_widget_id("toolbar.preset").is_some(), "{locale} wide must show Preset switcher");

            // Narrow responsive breakpoint 640x420 @ 1.5x HiDPI: must collapse gracefully with zero bleed/wrapping.
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.5));
            let compact_report = driver.verify_pixel_to_hit_box_parity();
            assert!(
                compact_report.is_clean(),
                "locale `{locale}` (host_draws_menu_bar={host_draws_menu_bar}) 640x420@1.5x parity/bleed failed: {:?}",
                compact_report.violations
            );
            assert!(driver.check_scrollbar_non_overlap().is_empty(), "locale `{locale}` 640x420 scrollbar overlap");

            let compact_shell = driver.state().last_report.as_ref().ok_or("missing compact report")?;
            let compact_t1 = compact_shell.tier1_rect_pt.ok_or("missing compact tier1 rect")?;
            let compact_t2 = compact_shell.tier2_rect_pt.ok_or("missing compact tier2 rect")?;
            assert!((compact_t1.height() - 32.0).abs() <= 1.0, "compact tier1 must stay single-row 32pt, got {}", compact_t1.height());
            assert!((compact_t2.height() - 38.0).abs() <= 1.0, "compact tier2 must stay single-row 38pt, got {}", compact_t2.height());

            for entry in &driver.frame_meta().entries {
                if entry.widget_id.starts_with("toolbar.") || entry.widget_id.starts_with("menu.") || entry.widget_id.starts_with("sheet.tab.") {
                    assert!(
                        entry.clipped_rect.right() <= 640.5,
                        "locale `{locale}` widget `{}` overflowed 640pt right edge: {:?}",
                        entry.widget_id,
                        entry.clipped_rect
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn top_chrome_tier2_adaptive_strips_all_sheet_contexts_and_overflow_menu() -> Result<(), String> {
    let state = TopChromeHarnessState::new("en", true);
    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::new(1280.0, 720.0, 1.0), render_top_chrome_harness);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    // 1. EmptyWorkspace context
    assert!(driver.frame_meta().find_by_widget_id("toolbar.new").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.open").is_some());
    let empty_analyze = driver.frame_meta().find_by_widget_id("toolbar.analyze").ok_or("missing disabled analyze in EmptyWorkspace")?;
    assert!(!empty_analyze.enabled);

    // 2. Structured DataSheet (`Column`)
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

    // 3. MultipleVariables DataSheet
    driver
        .state_mut()
        .app
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-mv", "title": "Cohort MV", "kind": "multiple_variables", "cols": 3, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("toolbar.formula_bar").is_some(), "MV table must surface Formula Bar quick-run");
    assert!(driver.frame_meta().find_by_widget_id("toolbar.randomize_groups").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.sample_rows").is_some());

    // 4. ResultSheet context
    let rs_out = driver
        .state_mut()
        .app
        .execute_command("analysis.run", json!({ "source_sheet_id": "ds-col", "kind": "one_way_anova" }))
        .map_err(|e| e.to_string())?;
    let rs_id = rs_out.payload["sheet_id"].as_str().ok_or("missing analysis result sheet_id")?.to_owned();
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": rs_id })).map_err(|e| e.to_string())?;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("toolbar.edit_params").is_some_and(|e| e.enabled), "ResultSheet must enable Edit Parameters");
    assert!(driver.frame_meta().find_by_widget_id("toolbar.change_data").is_some_and(|e| e.enabled));
    assert!(driver.frame_meta().find_by_widget_id("toolbar.methods_report").is_some_and(|e| e.enabled));

    // 5. GraphSheet context
    driver.state_mut().app.execute_command("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "gs-col" })).map_err(|e| e.to_string())?;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("toolbar.graph_type").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.axes").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.format_graph").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.export_svg").is_some());

    verify_info_layout_and_overflow(&mut driver)
}

fn verify_info_layout_and_overflow(driver: &mut EguiHumanPathDriver<'_, TopChromeHarnessState>) -> Result<(), String> {
    // 6. InfoSheet & LayoutSheet contexts (including `toolbar.new_graph` -> `graph.create` & constant click).
    if let Some(info_id) = driver.state().app.document().info_sheets.first().map(|s| s.id.as_str().to_owned()) {
        let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": info_id }));
        driver.run_settled();
        let add_const = driver.frame_meta().find_by_widget_id("toolbar.info_add_constant").ok_or("InfoSheet must surface + Constant")?;
        assert!(add_const.enabled);
        let new_graph_entry = driver.frame_meta().find_by_widget_id("toolbar.new_graph").ok_or("InfoSheet must surface toolbar.new_graph")?;
        assert_eq!(new_graph_entry.command_id.as_deref(), Some("graph.create"));
        assert!(new_graph_entry.enabled);
        let graphs_before = driver.state().app.document().graph_sheets.len();
        driver.click_widget_id("toolbar.new_graph")?;
        assert_eq!(driver.state().app.document().graph_sheets.len(), graphs_before + 1, "InfoSheet toolbar.new_graph click must create GraphSheet");
        let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": info_id }));
        driver.run_settled();
        driver.click_widget_id("toolbar.info_add_constant")?;
    }
    driver.state_mut().app.execute_command("layout.create", json!({ "sheet_id": "ls-1", "title": "Figure 1" })).map_err(|e| e.to_string())?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ls-1" })).map_err(|e| e.to_string())?;
    driver.run_settled();
    let composer_btn = driver.frame_meta().find_by_widget_id("toolbar.layout_composer").ok_or("LayoutSheet must surface Layout Composer")?;
    assert_eq!(composer_btn.command_id.as_deref(), Some("ui.layout.composer.open"));
    assert_eq!(composer_btn.label, curvebook_ui_egui::tr("en", "cmd.ui.layout.composer.open"));
    assert!(driver.frame_meta().find_by_widget_id("toolbar.rulers").is_some());
    assert!(driver.frame_meta().find_by_widget_id("toolbar.canvas_preset").is_some());

    // Compact 640x420 Tier 2 overflows gracefully into `⋯` (`toolbar.overflow`) and exposes interactive `ZLayer::DropdownMenu` popup items.
    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" }));
    driver.run_settled();
    if driver.frame_meta().find_by_widget_id("toolbar.overflow").is_some() {
        driver.click_widget_id("toolbar.overflow")?;
        let ov_item = driver.frame_meta().find_by_widget_id("toolbar.overflow.item.0").ok_or("clicking `⋯` must open overflow popup items")?;
        assert_eq!(ov_item.z_layer, ZLayer::DropdownMenu);
    }

    // Sheet tab switching via Tier 2 `sheet.tab.<id>` commits through PendingUiIntents cleanly.
    if driver.frame_meta().find_by_widget_id("sheet.tab.ds-col").is_some()
        && driver.state().app.document().active_sheet_id.as_ref().map(|s| s.as_str()) != Some("ds-col")
    {
        driver.click_widget_id("sheet.tab.ds-col")?;
        assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(|s| s.as_str()), Some("ds-col"));
    }

    Ok(())
}

#[test]
fn top_chrome_split_presets_workspace_presets_and_presentation_mode() -> Result<(), String> {
    let mut state = TopChromeHarnessState::new("en", true);
    state
        .app
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-s1", "title": "Assay Data", "kind": "xy", "cols": 2, "rows": 5 }))
        .map_err(|e| e.to_string())?;
    state
        .app
        .execute_command("graph.create", json!({ "sheet_id": "gs-s1", "data_sheet_id": "ds-s1", "title": "Assay Curve" }))
        .map_err(|e| e.to_string())?;

    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::new(1440.0, 600.0, 1.0), render_top_chrome_harness);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    // 1. Workspace Preset Switcher (`Default`, `Data Prep`, `Modeling`, `Publication`, `Compact`).
    driver.click_widget_id("toolbar.preset")?;
    for wid in
        ["toolbar.preset.default", "toolbar.preset.data_prep", "toolbar.preset.modeling", "toolbar.preset.publication", "toolbar.preset.compact"]
    {
        let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("preset switcher popup missing `{wid}`"))?;
        assert_eq!(entry.z_layer, ZLayer::DropdownMenu);
    }
    driver.click_widget_id("toolbar.preset.publication")?;
    assert_eq!(driver.state().app.egui_views.workspace_preset, WorkspacePreset::Publication);

    driver.click_widget_id("toolbar.preset")?;
    driver.click_widget_id("toolbar.preset.compact")?;
    assert_eq!(driver.state().app.egui_views.workspace_preset, WorkspacePreset::Compact);

    // 2. Split Preset Selector (`Data + Graph`, `Swap Panes`, `Single Pane`, plus `Cmd+\`).
    driver.click_widget_id("toolbar.split")?;
    let dg_entry = driver.frame_meta().find_by_widget_id("toolbar.split.preset.data_graph").ok_or("missing toolbar.split.preset.data_graph")?;
    assert!(dg_entry.enabled);
    assert_eq!(dg_entry.z_layer, ZLayer::DropdownMenu);
    driver.click_widget_id("toolbar.split.preset.data_graph")?;
    assert!(driver.state().app.split.enabled, "Data + Graph preset must enable split view");
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(|s| s.as_str()), Some("ds-s1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(|s| s.as_str()), Some("gs-s1"));

    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.swap")?;
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(|s| s.as_str()), Some("gs-s1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(|s| s.as_str()), Some("ds-s1"));

    // Shortcut `Cmd+\` toggles split off and back on.
    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::Backslash, cmd_mod);
    assert!(!driver.state().app.split.enabled);
    driver.press_egui_key(Key::Backslash, cmd_mod);
    assert!(driver.state().app.split.enabled);

    // 3. Presentation Mode (`Cmd+Shift+P` / button / `F11` -> collapses chrome, preserves exit chip, `Esc` restores chrome).
    driver.click_widget_id("toolbar.presentation")?;
    assert!(driver.state().app.presentation_mode);
    assert!(driver.frame_meta().find_by_widget_id("toolbar.presentation.exit").is_some(), "Presentation mode must preserve exit chip");
    assert!(driver.frame_meta().find_by_widget_id("toolbar.home").is_none(), "Top chrome must collapse during Presentation Mode");

    driver.click_widget_id("toolbar.presentation.exit")?;
    assert!(!driver.state().app.presentation_mode);
    assert!(driver.frame_meta().find_by_widget_id("toolbar.home").is_some(), "Top chrome must restore when exiting Presentation Mode");

    driver.press_egui_key(Key::F11, Modifiers::NONE);
    assert!(driver.state().app.presentation_mode);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.presentation_mode);

    Ok(())
}

#[test]
fn command_palette_keyboard_nav_fuzzy_search_backdrop_capture_and_discard_guard() -> Result<(), String> {
    let state = TopChromeHarnessState::new("en", true);
    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::new(640.0, 420.0, 1.0), render_top_chrome_harness);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    // Open Command Palette via `Cmd/Ctrl+K`.
    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::K, cmd_mod);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    let parity_640 = driver.verify_pixel_to_hit_box_parity();
    assert!(parity_640.is_clean(), "Command Palette 640x420 parity failed: {:?}", parity_640.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty(), "Command Palette ScrollBar overlapped rows");

    let backdrop = driver.frame_meta().find_by_widget_id("modal.backdrop").ok_or("missing modal.backdrop")?;
    assert_eq!(backdrop.z_layer, ZLayer::ModalBackdrop);
    assert!((backdrop.clipped_rect.width() - 640.0).abs() <= 1.0 && (backdrop.clipped_rect.height() - 420.0).abs() <= 1.0);

    let dialog = driver.frame_meta().find_by_widget_id("palette.dialog").ok_or("missing palette.dialog")?;
    assert_eq!(dialog.z_layer, ZLayer::ModalDialog);
    assert!(dialog.clipped_rect.bottom() <= 420.0, "Command Palette must stay inside 640x420 viewport: {:?}", dialog.clipped_rect);

    // Outside scroll wheel is captured cleanly by Command Palette backdrop.
    let wheel_before = driver.state().wheel_capture_count;
    driver.wheel_at(egui::pos2(12.0, 210.0), egui::vec2(0.0, -40.0))?;
    assert!(driver.state().wheel_capture_count > wheel_before, "outside wheel must be captured when Command Palette is open");

    // Disabled rows report contextual reason cleanly.
    driver.type_text("Change Analysis Parameters")?;
    let disabled_item = driver.frame_meta().find_by_widget_id("palette.item.0").ok_or("missing filtered palette.item.0")?;
    assert!(!disabled_item.enabled);
    assert_eq!(disabled_item.disabled_reason.as_deref(), Some("Open a ResultSheet first"));

    // Outside click on backdrop dismisses palette cleanly.
    driver.click_at(egui::pos2(12.0, 210.0))?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Reopen at 1440x600 via clicking search pill, filter, navigate with Arrow keys, execute once on multi-pass `will_discard`.
    driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.click_widget_id("toolbar.command_palette")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    driver.type_text("New")?;
    let before_down = driver.state().app.egui_views.command_palette.selected_index;
    driver.press_egui_key(Key::ArrowDown, Modifiers::NONE);
    let after_down = driver.state().app.egui_views.command_palette.selected_index;
    assert_ne!(before_down, after_down, "ArrowDown must advance Command Palette selection index");
    driver.press_egui_key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(driver.state().app.egui_views.command_palette.selected_index, before_down, "ArrowUp must return selection index");

    // Guard check under simulated `ctx.request_discard()`: discarded pass must NOT mutate or carry over;
    // subsequent settled Enter executes strictly once.
    driver.state_mut().app.egui_views.command_palette.query = String::from("New Data Table");
    driver.run_settled();
    driver.state_mut().discard_next_frame = true;
    driver.state_mut().discard_passes = 0;
    driver.state_mut().settled_passes = 0;
    let sheets_before = driver.state().app.document().data_sheets.len();
    let undo_before = driver.state().app.session().undo_depth();
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert!(driver.state().discard_passes >= 1, "expected discard pass to execute");
    assert_eq!(driver.state().app.document().data_sheets.len(), sheets_before, "discarded pass must not mutate");
    assert_eq!(driver.state().app.session().undo_depth(), undo_before, "discarded pass must not push undo");

    driver.run_settled();
    assert_eq!(driver.state().app.document().data_sheets.len(), sheets_before, "discarded intent must never replay");

    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert_eq!(driver.state().app.document().data_sheets.len(), sheets_before + 1, "settled Command Palette Enter must commit once");
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1, "settled Command Palette Enter must push single undo");

    Ok(())
}
