#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Permanent layout hierarchy, non-overlap, WCAG 2.2 SC 2.5.8 target-size, action-rank, and
//! per-region control-budget regression suite (`docs/planning/ui-overlap-and-target-size-audit.md`
//! `F1`–`F5`, `R15.10`).

use curvebook_doc::SheetId;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{
    EguiHumanPathDriver, RegionBudgets, RegionControlCounts, ShellRegion, TextBleedPolicy, build_driver_font_definitions,
    check_region_control_budgets, count_controls_by_region,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::menus::menu_bar_tooltips;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use curvebook_ui_egui::{UiApp, dispatch_ui_command};
use serde_json::json;

struct ProbeShellState {
    app: UiApp,
    views: EguiShellViewState,
    carried_intents: PendingUiIntents,
}

fn make_probe_driver(mut app: UiApp, locale: &str, ui_scale: f32, width: f64, height: f64) -> EguiHumanPathDriver<'static, ProbeShellState> {
    let _ = app.set_locale(locale);
    app.theme = UiThemeSpec::for_mode(UiThemeMode::Light);
    app.theme.ui_scale = ui_scale;
    app.sync_views();
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ProbeShellState { app, views, carried_intents: PendingUiIntents::new() },
        ViewportMetrics::new(width, height, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.app.egui_views = st.views.clone();
            let _ = st.carried_intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        }
    });
    driver.run_settled();
    driver
}

fn build_rich_project_app() -> UiApp {
    let mut app = UiApp::new();
    app.host_draws_menu_bar = false;

    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-xy",
            "title": "Dose Response",
            "kind": "xy",
            "cols": 2,
            "rows": 5,
            "auto_info": true,
            "info_sheet_id": "info-notes"
        }),
    )
    .unwrap();
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [-8.0, -7.0, -6.0, -5.0, -4.0] })).unwrap();
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "ds-xy",
            "rows": [[8.0, 10.0], [22.0, 25.0], [52.0, 50.0], [82.0, 79.0], [94.0, 92.0]]
        }),
    )
    .unwrap();

    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "title": "Treatment Groups", "kind": "column", "cols": 3, "rows": 5 }))
        .unwrap();
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "ds-col",
            "rows": [[10.2, 21.4, 35.1], [11.0, 22.8, 36.4], [9.8, 20.5, 34.0], [10.6, 23.1, 37.2], [10.4, 21.9, 35.8]]
        }),
    )
    .unwrap();

    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-grp", "title": "Two-Factor Assay", "kind": "grouped", "cols": 2, "rows": 4, "replicates": 2 }),
    )
    .unwrap();

    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-mv", "title": "Cohort MV", "kind": "multiple_variables", "cols": 3, "rows": 4 }))
        .unwrap();

    app.execute_command("analysis.run", json!({ "sheet_id": "rs-desc", "source_sheet_id": "ds-xy", "kind": "descriptive_stats" })).unwrap();
    app.execute_command(
        "stats.run",
        json!({
            "sheet_id": "rs-anova",
            "source_sheet_id": "ds-col",
            "kind": "one_way_anova",
            "multiplicity": "tukey_hsd"
        }),
    )
    .unwrap();
    app.execute_command(
        "fit.run",
        json!({
            "sheet_id": "rs-fit",
            "source_sheet_id": "ds-xy",
            "kind": "four_parameter_logistic"
        }),
    )
    .unwrap();
    app.execute_command(
        "transform.run",
        json!({
            "sheet_id": "rs-xfm",
            "source_sheet_id": "ds-xy",
            "kind": "normalize"
        }),
    )
    .unwrap();

    app.execute_command("graph.create", json!({ "sheet_id": "gs-xy", "data_sheet_id": "ds-xy", "title": "Dose Curve" })).unwrap();
    app.execute_command("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col", "kind": "column", "title": "Group Bars" })).unwrap();
    app.execute_command("layout.create", json!({ "sheet_id": "ls-fig", "title": "Figure 1" })).unwrap();
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "ls-fig", "kind": "graph", "graph_sheet_id": "gs-xy", "bounds_pt": [18.0, 22.0, 200.0, 160.0] }),
    )
    .unwrap();
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "ls-fig", "kind": "graph", "graph_sheet_id": "gs-col", "bounds_pt": [232.0, 22.0, 200.0, 160.0] }),
    )
    .unwrap();
    app
}

fn update_max_counts(max: &mut RegionControlCounts, cur: &RegionControlCounts) {
    max.top_chrome = max.top_chrome.max(cur.top_chrome);
    max.navigator = max.navigator.max(cur.navigator);
    max.center_stage = max.center_stage.max(cur.center_stage);
    max.inspector = max.inspector.max(cur.inspector);
    max.status_footer = max.status_footer.max(cur.status_footer);
    max.dropdown_menu = max.dropdown_menu.max(cur.dropdown_menu);
    max.modal_dialog = max.modal_dialog.max(cur.modal_dialog);
}

#[test]
fn workspace_views_pass_overlap_target_size_rank_and_region_budgets() {
    let base = build_rich_project_app();
    let views: Vec<(&str, Option<&str>)> = vec![
        ("start_screen", None),
        ("data_xy", Some("ds-xy")),
        ("data_grouped", Some("ds-grp")),
        ("data_column", Some("ds-col")),
        ("data_mv", Some("ds-mv")),
        ("info_notes", Some("info-notes")),
        ("result_desc", Some("rs-desc")),
        ("result_anova_tabs", Some("rs-anova")),
        ("graph_xy", Some("gs-xy")),
        ("layout_fig", Some("ls-fig")),
    ];
    let configs = [
        ("640x420_en", "en", 1.0_f32, 640.0, 420.0),
        ("800x600_en", "en", 1.0_f32, 800.0, 600.0),
        ("1024x700_en", "en", 1.0_f32, 1024.0, 700.0),
        ("1280x800_en", "en", 1.0_f32, 1280.0, 800.0),
        ("1440x820_en", "en", 1.0_f32, 1440.0, 820.0),
        ("1920x1080_en", "en", 1.0_f32, 1920.0, 1080.0),
        ("640x420_de", "de", 1.0_f32, 640.0, 420.0),
        ("1024x700_de", "de", 1.0_f32, 1024.0, 700.0),
        ("zoom200_de", "de", 2.0_f32, 1280.0, 840.0),
    ];

    let mut max_counts = RegionControlCounts::default();
    let mut failures = Vec::new();

    for &(view_name, sheet_opt) in &views {
        for &(cfg_name, locale, scale, w, h) in &configs {
            let app = if let Some(sid) = sheet_opt {
                let mut cloned = base.clone();
                cloned.execute_command("sheet.select", json!({ "sheet_id": sid })).unwrap();
                cloned
            } else {
                UiApp::new()
            };
            let driver = make_probe_driver(app, locale, scale, w, h);
            let meta = driver.frame_meta();

            // F1: legacy prev/next tab steppers must never exist.
            assert!(
                meta.find_by_widget_id("result.tabs.prev").is_none() && meta.find_by_widget_id("result.tabs.next").is_none(),
                "[{view_name}/{cfg_name}] legacy result.tabs.prev/next still present"
            );
            if view_name == "result_desc" && cfg_name == "1440x820_en" {
                assert!(
                    meta.find_by_widget_id("result.tabs.overflow").is_none(),
                    "single-tab result sheet at 1440x820 must hide result.tabs.overflow"
                );
            }

            let counts = count_controls_by_region(meta);
            update_max_counts(&mut max_counts, &counts);

            let report = driver.verify_pixel_to_hit_box_parity();
            for v in report.violations {
                failures.push(format!("[{view_name}/{cfg_name}] {}: {} — {}", v.check, v.widget_id, v.detail));
            }
            for bv in check_region_control_budgets(meta, &RegionBudgets::CANONICAL) {
                failures.push(format!("[{view_name}/{cfg_name}] {}: {}", bv.check, bv.detail));
            }
        }
    }

    println!(
        "workspace_max_region_counts: top={} nav={} center={} inspector={} footer={} modal={}",
        max_counts.count_for(ShellRegion::TopChrome),
        max_counts.count_for(ShellRegion::Navigator),
        max_counts.count_for(ShellRegion::CenterStage),
        max_counts.count_for(ShellRegion::Inspector),
        max_counts.count_for(ShellRegion::StatusFooter),
        max_counts.count_for(ShellRegion::ModalDialog),
    );
    assert!(failures.is_empty(), "workspace parity/budget failures ({}):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn all_reachable_dialogs_pass_overlap_target_size_rank_and_budgets() {
    let base = build_rich_project_app();
    type DialogSetup = Box<dyn Fn(&mut UiApp)>;
    let dialogs: Vec<(&str, DialogSetup)> = vec![
        (
            "palette",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "ui.palette.open", &json!({}));
            }),
        ),
        (
            "analyze",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.analyze.open", &json!({ "sheet_id": "ds-xy" }));
            }),
        ),
        (
            "edit_params_anova",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "rs-anova" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.editParams.open", &json!({ "sheet_id": "rs-anova" }));
            }),
        ),
        (
            "edit_params_fit",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "rs-fit" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.editParams.open", &json!({ "sheet_id": "rs-fit" }));
            }),
        ),
        (
            "edit_params_desc",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "rs-desc" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.editParams.open", &json!({ "sheet_id": "rs-desc" }));
            }),
        ),
        (
            "edit_params_transform",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "rs-xfm" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.editParams.open", &json!({ "sheet_id": "rs-xfm" }));
            }),
        ),
        (
            "change_graph_type",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.changeGraphType.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "apply_style_from",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-col" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.applyStyleFrom.open", &json!({ "sheet_id": "gs-col" }));
            }),
        ),
        (
            "create_graph",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.create.open", &json!({ "sheet_id": "ds-xy" }));
            }),
        ),
        (
            "equalize_scaling",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ls-fig" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.layout.equalizeScaling.open", &json!({ "sheet_id": "ls-fig" }));
            }),
        ),
        (
            "format_graph",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.format.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "format_axes",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.axes.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "pairwise_brackets",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-col" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.pairwise.open", &json!({ "sheet_id": "gs-col" }));
            }),
        ),
        (
            "bubble_plot",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.bubblePlot.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "insert_model_equation",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.graph.insertModelEquation", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "layout_composer",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ls-fig" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.layout.composer.open", &json!({ "sheet_id": "ls-fig" }));
            }),
        ),
        (
            "export_figure",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.export.figure.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "print",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.print.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "power_wizard",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "ui.powerWizard.open", &json!({}));
            }),
        ),
        (
            "format_points",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "gs-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.formatPoints.open", &json!({ "sheet_id": "gs-xy" }));
            }),
        ),
        (
            "randomize_groups",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "ui.table.randomizeGroups", &json!({}));
            }),
        ),
        (
            "random_sample_rows",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.table.randomSampleRows", &json!({ "sheet_id": "ds-xy" }));
            }),
        ),
        (
            "simulate_distributions",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "ui.table.simulateDistributions", &json!({}));
            }),
        ),
        (
            "exclude_with_reason",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.grid.excludeWithReason", &json!({ "sheet_id": "ds-xy" }));
            }),
        ),
        (
            "convert_archetype",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                let _ = dispatch_ui_command(app, "ui.table.convertArchetype.open", &json!({ "sheet_id": "ds-xy" }));
            }),
        ),
        (
            "import_reshape",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "ui.import.reshape.open", &json!({}));
            }),
        ),
        (
            "manage_variables",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" })).unwrap();
                app.grid.variables_dialog.open = true;
            }),
        ),
        (
            "preferences",
            Box::new(|app| {
                app.egui_views.app_modals.preferences_open = true;
            }),
        ),
        (
            "find_replace",
            Box::new(|app| {
                app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
                app.egui_views.app_modals.find_replace_open = true;
            }),
        ),
        (
            "change_data_analyzed",
            Box::new(|app| {
                app.canvas_dialogs.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id: SheetId::from("rs-anova") };
            }),
        ),
        (
            "about",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "app.about", &json!({}));
            }),
        ),
        (
            "feedback",
            Box::new(|app| {
                let _ = dispatch_ui_command(app, "app.feedback", &json!({}));
            }),
        ),
    ];

    let configs = [("800x600_en", "en", 800.0, 600.0), ("1280x800_en", "en", 1280.0, 800.0), ("1024x700_de", "de", 1024.0, 700.0)];

    let mut max_modal = 0usize;
    let mut failures = Vec::new();
    for (dlg_name, setup) in &dialogs {
        for &(cfg_name, locale, w, h) in &configs {
            let mut app = base.clone();
            setup(&mut app);
            let driver = make_probe_driver(app, locale, 1.0, w, h);
            let meta = driver.frame_meta();
            let counts = count_controls_by_region(meta);
            max_modal = max_modal.max(counts.count_for(ShellRegion::ModalDialog));

            let report = driver.verify_pixel_to_hit_box_parity();
            for v in report.violations {
                failures.push(format!("[dialog:{dlg_name}/{cfg_name}] {}: {} — {}", v.check, v.widget_id, v.detail));
            }
            for bv in check_region_control_budgets(meta, &RegionBudgets::CANONICAL) {
                failures.push(format!("[dialog:{dlg_name}/{cfg_name}] {}: {}", bv.check, bv.detail));
            }
        }
    }

    println!("dialogs_max_modal_controls: {max_modal}");
    assert!(failures.is_empty(), "dialog parity/budget failures ({}):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn all_eight_menu_bar_menus_pass_overlap_target_size_and_budgets() {
    let mut base = build_rich_project_app();
    base.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
    let menu_keys: Vec<String> = menu_bar_tooltips(&base).into_iter().map(|s| s.menu_key.to_owned()).collect();
    assert_eq!(menu_keys.len(), 8, "expected all 8 menu-bar menus");

    let configs = [("800x600_en", "en", 800.0, 600.0), ("1280x800_en", "en", 1280.0, 800.0), ("1024x700_de", "de", 1024.0, 700.0)];

    let mut max_menu = 0usize;
    let mut failures = Vec::new();
    for menu_key in &menu_keys {
        for &(cfg_name, locale, w, h) in &configs {
            let mut driver = make_probe_driver(base.clone(), locale, 1.0, w, h);
            driver.click_widget_id(menu_key).unwrap_or_else(|e| panic!("click `{menu_key}` at {cfg_name}: {e}"));
            let meta = driver.frame_meta();
            let popup_id = format!("popup.{menu_key}");
            assert!(meta.find_by_widget_id(&popup_id).is_some(), "[{menu_key}/{cfg_name}] expected `{popup_id}` open");
            let counts = count_controls_by_region(meta);
            max_menu = max_menu.max(counts.count_for(ShellRegion::DropdownMenu));

            let report = driver.verify_pixel_to_hit_box_parity();
            for v in report.violations {
                failures.push(format!("[menu:{menu_key}/{cfg_name}] {}: {} — {}", v.check, v.widget_id, v.detail));
            }
            for bv in check_region_control_budgets(meta, &RegionBudgets::CANONICAL) {
                failures.push(format!("[menu:{menu_key}/{cfg_name}] {}: {}", bv.check, bv.detail));
            }
        }
    }

    println!("menus_max_dropdown_controls: {max_menu}");
    assert!(failures.is_empty(), "menu parity/budget failures ({}):\n{}", failures.len(), failures.join("\n"));
}
