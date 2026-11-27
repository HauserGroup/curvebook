//! Permanent layout hierarchy, non-overlap, WCAG 2.2 SC 2.5.8 target-size, action-rank, and
//! per-region control-budget regression suite (`docs/planning/ui-overlap-and-target-size-audit.md`
//! `F1`–`F5`, `R15.10`).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_doc::SheetId;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::{
    EguiHumanPathDriver, RegionBudgets, RegionControlCounts, ShellRegion, check_region_control_budgets, count_controls_by_region,
};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::menus::menu_bar_tooltips;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use curvebook_ui_egui::{UiApp, dispatch_ui_command};
use serde_json::json;

use crate::common::{ShellTestState, make_shell_driver_with_state};

fn make_probe_driver(mut app: UiApp, locale: &str, ui_scale: f32, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let _ = app.set_locale(locale);
    app.theme = UiThemeSpec::for_mode(UiThemeMode::Light);
    app.theme.ui_scale = ui_scale;
    app.sync_views();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(w, h, 1.0), tokens)
}

fn build_rich_project_app() -> UiApp {
    let mut app = UiApp::new();
    app.host_draws_menu_bar = false;
    for (cmd, params) in [
        (
            "sheet.createData",
            json!({ "sheet_id": "ds-xy", "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 5, "auto_info": true, "info_sheet_id": "info-notes" }),
        ),
        ("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [-8.0, -7.0, -6.0, -5.0, -4.0] })),
        ("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[8.0, 10.0], [22.0, 25.0], [52.0, 50.0], [82.0, 79.0], [94.0, 92.0]] })),
        ("sheet.createData", json!({ "sheet_id": "ds-col", "title": "Treatment Groups", "kind": "column", "cols": 3, "rows": 5 })),
        (
            "table.setCells",
            json!({ "sheet_id": "ds-col", "rows": [[10.2, 21.4, 35.1], [11.0, 22.8, 36.4], [9.8, 20.5, 34.0], [10.6, 23.1, 37.2], [10.4, 21.9, 35.8]] }),
        ),
        ("sheet.createData", json!({ "sheet_id": "ds-grp", "title": "Two-Factor Assay", "kind": "grouped", "cols": 2, "rows": 4, "replicates": 2 })),
        ("sheet.createData", json!({ "sheet_id": "ds-mv", "title": "Cohort MV", "kind": "multiple_variables", "cols": 3, "rows": 4 })),
        ("analysis.run", json!({ "sheet_id": "rs-desc", "source_sheet_id": "ds-xy", "kind": "descriptive_stats" })),
        ("stats.run", json!({ "sheet_id": "rs-anova", "source_sheet_id": "ds-col", "kind": "one_way_anova", "multiplicity": "tukey_hsd" })),
        ("fit.run", json!({ "sheet_id": "rs-fit", "source_sheet_id": "ds-xy", "kind": "four_parameter_logistic" })),
        ("transform.run", json!({ "sheet_id": "rs-xfm", "source_sheet_id": "ds-xy", "kind": "normalize" })),
        ("graph.create", json!({ "sheet_id": "gs-xy", "data_sheet_id": "ds-xy", "title": "Dose Curve" })),
        ("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col", "kind": "column", "title": "Group Bars" })),
        ("layout.create", json!({ "sheet_id": "ls-fig", "title": "Figure 1" })),
        ("layout.addPanel", json!({ "sheet_id": "ls-fig", "kind": "graph", "graph_sheet_id": "gs-xy", "bounds_pt": [18.0, 22.0, 200.0, 160.0] })),
        ("layout.addPanel", json!({ "sheet_id": "ls-fig", "kind": "graph", "graph_sheet_id": "gs-col", "bounds_pt": [232.0, 22.0, 200.0, 160.0] })),
    ] {
        app.execute_command(cmd, params).unwrap();
    }
    app
}

fn collect_violations(driver: &EguiHumanPathDriver<'_, ShellTestState>, tag: &str, failures: &mut Vec<String>) {
    let meta = driver.frame_meta();
    for v in driver.verify_pixel_to_hit_box_parity().violations {
        failures.push(format!("[{tag}] {}: {} — {}", v.check, v.widget_id, v.detail));
    }
    for bv in check_region_control_budgets(meta, &RegionBudgets::CANONICAL) {
        failures.push(format!("[{tag}] {}: {}", bv.check, bv.detail));
    }
}

#[test]
fn workspace_views_pass_overlap_target_size_rank_and_region_budgets() {
    let base = build_rich_project_app();
    let views = [
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
        ("800x600_en", "en", 1.0, 800.0, 600.0),
        ("1024x700_en", "en", 1.0, 1024.0, 700.0),
        ("1280x800_en", "en", 1.0, 1280.0, 800.0),
        ("1440x820_en", "en", 1.0, 1440.0, 820.0),
        ("1920x1080_en", "en", 1.0, 1920.0, 1080.0),
        ("640x420_de", "de", 1.0, 640.0, 420.0),
        ("1024x700_de", "de", 1.0, 1024.0, 700.0),
        ("zoom200_de", "de", 2.0, 1280.0, 840.0),
    ];

    let mut max = RegionControlCounts::default();
    let mut failures = Vec::new();
    for (view_name, sheet_opt) in views {
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
            assert!(
                meta.find_by_widget_id("result.tabs.prev").is_none() && meta.find_by_widget_id("result.tabs.next").is_none(),
                "[{view_name}/{cfg_name}] legacy tab steppers"
            );
            if view_name == "result_desc" && cfg_name == "1440x820_en" {
                assert!(meta.find_by_widget_id("result.tabs.overflow").is_none());
            }
            let c = count_controls_by_region(meta);
            max.top_chrome = max.top_chrome.max(c.top_chrome);
            max.navigator = max.navigator.max(c.navigator);
            max.center_stage = max.center_stage.max(c.center_stage);
            max.inspector = max.inspector.max(c.inspector);
            max.status_footer = max.status_footer.max(c.status_footer);
            max.dropdown_menu = max.dropdown_menu.max(c.dropdown_menu);
            max.modal_dialog = max.modal_dialog.max(c.modal_dialog);
            collect_violations(&driver, &format!("{view_name}/{cfg_name}"), &mut failures);
        }
    }
    println!(
        "workspace_max_region_counts: top={} nav={} center={} inspector={} footer={} modal={}",
        max.count_for(ShellRegion::TopChrome),
        max.count_for(ShellRegion::Navigator),
        max.count_for(ShellRegion::CenterStage),
        max.count_for(ShellRegion::Inspector),
        max.count_for(ShellRegion::StatusFooter),
        max.count_for(ShellRegion::ModalDialog),
    );
    assert!(failures.is_empty(), "workspace parity/budget failures ({}):\n{}", failures.len(), failures.join("\n"));
}

enum DialogOp {
    Cmd(&'static str),
    ManageVariables,
    Preferences,
    FindReplace,
    ChangeDataAnalyzed,
}

#[test]
fn all_reachable_dialogs_pass_overlap_target_size_rank_and_budgets() {
    let base = build_rich_project_app();
    let dialogs: &[(&str, Option<&str>, DialogOp)] = &[
        ("palette", None, DialogOp::Cmd("ui.palette.open")),
        ("analyze", Some("ds-xy"), DialogOp::Cmd("ui.analyze.open")),
        ("edit_params_anova", Some("rs-anova"), DialogOp::Cmd("ui.editParams.open")),
        ("edit_params_fit", Some("rs-fit"), DialogOp::Cmd("ui.editParams.open")),
        ("edit_params_desc", Some("rs-desc"), DialogOp::Cmd("ui.editParams.open")),
        ("edit_params_transform", Some("rs-xfm"), DialogOp::Cmd("ui.editParams.open")),
        ("change_graph_type", Some("gs-xy"), DialogOp::Cmd("ui.changeGraphType.open")),
        ("apply_style_from", Some("gs-col"), DialogOp::Cmd("ui.applyStyleFrom.open")),
        ("create_graph", Some("ds-xy"), DialogOp::Cmd("ui.graph.create.open")),
        ("equalize_scaling", Some("ls-fig"), DialogOp::Cmd("ui.layout.equalizeScaling.open")),
        ("format_graph", Some("gs-xy"), DialogOp::Cmd("ui.graph.format.open")),
        ("format_axes", Some("gs-xy"), DialogOp::Cmd("ui.graph.axes.open")),
        ("pairwise_brackets", Some("gs-col"), DialogOp::Cmd("ui.graph.pairwise.open")),
        ("bubble_plot", Some("gs-xy"), DialogOp::Cmd("ui.graph.bubblePlot.open")),
        ("insert_model_equation", Some("gs-xy"), DialogOp::Cmd("ui.graph.insertModelEquation")),
        ("layout_composer", Some("ls-fig"), DialogOp::Cmd("ui.layout.composer.open")),
        ("export_figure", Some("gs-xy"), DialogOp::Cmd("ui.export.figure.open")),
        ("print", Some("gs-xy"), DialogOp::Cmd("ui.print.open")),
        ("power_wizard", None, DialogOp::Cmd("ui.powerWizard.open")),
        ("format_points", Some("gs-xy"), DialogOp::Cmd("ui.formatPoints.open")),
        ("randomize_groups", None, DialogOp::Cmd("ui.table.randomizeGroups")),
        ("random_sample_rows", Some("ds-xy"), DialogOp::Cmd("ui.table.randomSampleRows")),
        ("simulate_distributions", None, DialogOp::Cmd("ui.table.simulateDistributions")),
        ("exclude_with_reason", Some("ds-xy"), DialogOp::Cmd("ui.grid.excludeWithReason")),
        ("convert_archetype", Some("ds-xy"), DialogOp::Cmd("ui.table.convertArchetype.open")),
        ("import_reshape", None, DialogOp::Cmd("ui.import.reshape.open")),
        ("manage_variables", Some("ds-mv"), DialogOp::ManageVariables),
        ("preferences", None, DialogOp::Preferences),
        ("find_replace", Some("ds-xy"), DialogOp::FindReplace),
        ("change_data_analyzed", None, DialogOp::ChangeDataAnalyzed),
        ("about", None, DialogOp::Cmd("app.about")),
    ];
    let configs = [("800x600_en", "en", 800.0, 600.0), ("1280x800_en", "en", 1280.0, 800.0), ("1024x700_de", "de", 1024.0, 700.0)];

    let mut max_modal = 0usize;
    let mut failures = Vec::new();
    for (dlg_name, sheet_opt, op) in dialogs {
        for &(cfg_name, locale, w, h) in &configs {
            let mut app = base.clone();
            if let Some(sid) = sheet_opt {
                app.execute_command("sheet.select", json!({ "sheet_id": sid })).unwrap();
            }
            match op {
                DialogOp::Cmd(cmd) => {
                    let params = sheet_opt.map_or_else(|| json!({}), |sid| json!({ "sheet_id": sid }));
                    let _ = dispatch_ui_command(&mut app, cmd, &params);
                }
                DialogOp::ManageVariables => app.grid.variables_dialog.open = true,
                DialogOp::Preferences => app.egui_views.app_modals.preferences_open = true,
                DialogOp::FindReplace => app.egui_views.app_modals.find_replace_open = true,
                DialogOp::ChangeDataAnalyzed => {
                    app.canvas_dialogs.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id: SheetId::from("rs-anova") }
                }
            }
            let driver = make_probe_driver(app, locale, 1.0, w, h);
            max_modal = max_modal.max(count_controls_by_region(driver.frame_meta()).count_for(ShellRegion::ModalDialog));
            collect_violations(&driver, &format!("dialog:{dlg_name}/{cfg_name}"), &mut failures);
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
    assert_eq!(menu_keys.len(), 8);

    let configs = [("800x600_en", "en", 800.0, 600.0), ("1280x800_en", "en", 1280.0, 800.0), ("1024x700_de", "de", 1024.0, 700.0)];
    let mut max_menu = 0usize;
    let mut failures = Vec::new();
    for menu_key in &menu_keys {
        for &(cfg_name, locale, w, h) in &configs {
            let mut driver = make_probe_driver(base.clone(), locale, 1.0, w, h);
            driver.click_widget_id(menu_key).unwrap_or_else(|e| panic!("click `{menu_key}` at {cfg_name}: {e}"));
            let popup_id = format!("popup.{menu_key}");
            assert!(driver.frame_meta().find_by_widget_id(&popup_id).is_some(), "[{menu_key}/{cfg_name}] missing `{popup_id}`");
            max_menu = max_menu.max(count_controls_by_region(driver.frame_meta()).count_for(ShellRegion::DropdownMenu));
            collect_violations(&driver, &format!("menu:{menu_key}/{cfg_name}"), &mut failures);
        }
    }
    println!("menus_max_dropdown_controls: {max_menu}");
    assert!(failures.is_empty(), "menu parity/budget failures ({}):\n{}", failures.len(), failures.join("\n"));
}
