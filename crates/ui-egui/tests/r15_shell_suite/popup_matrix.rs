//! Cross-viewport (`640×420`, `1280×800`) and `de`-locale popup geometry & two-frame stability
//! matrix across all 10 migrated dropdown, overflow, and context menus (`R20.16`).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_doc::{ResultCell, ResultTab, ResultTabKind, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, check_popup_two_frame_stability};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use egui::{Key, Modifiers, pos2};
use serde_json::json;

use crate::common::{ShellTestState, make_shell_driver_with_state, select_sheet_and_settle, widget_center};

fn build_popup_matrix_app() -> UiApp {
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
        (
            "sheet.createData",
            json!({
                "sheet_id": "ds-mv",
                "title": "Cohort MV",
                "kind": "multiple_variables",
                "cols": 3,
                "rows": 4,
                "variables": [
                    { "title": "Group", "role": "categorical" },
                    { "title": "Dose", "role": "continuous" },
                    { "title": "SampleID", "role": "label" }
                ],
                "auto_graph": false
            }),
        ),
        ("stats.run", json!({ "sheet_id": "rs-anova", "source_sheet_id": "ds-col", "kind": "one_way_anova", "multiplicity": "tukey_hsd" })),
        ("graph.create", json!({ "sheet_id": "gs-xy", "data_sheet_id": "ds-xy", "title": "Dose Curve" })),
        ("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col", "kind": "column", "title": "Group Bars" })),
        ("layout.create", json!({ "sheet_id": "ls-fig", "title": "Figure 1" })),
        ("layout.addPanel", json!({ "sheet_id": "ls-fig", "kind": "graph", "graph_sheet_id": "gs-xy", "bounds_pt": [18.0, 22.0, 200.0, 160.0] })),
    ] {
        app.execute_command(cmd, params).unwrap();
    }

    // Add enough tabs to `rs-anova` so `result.tabs.overflow` appears at both 640×420 and 1280×800.
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("rs-anova")) {
        for i in 0..14 {
            rs.tabs.push(ResultTab {
                id: format!("extra-tab-{i}"),
                title: format!("Extended Statistical Diagnostics Tab {i}"),
                kind: ResultTabKind::Diagnostics,
                column_headers: vec![String::from("Metric"), String::from("Value")],
                row_labels: vec![String::from("Row 1")],
                rows: vec![vec![ResultCell::Text(format!("m{i}")), ResultCell::Text(format!("{i}.0"))]],
                footnotes: Vec::new(),
            });
        }
    }
    *app.session_mut() = Session::from_document(doc);
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).unwrap();
    app
}

fn make_matrix_driver(mut app: UiApp, locale: &str, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let _ = app.set_locale(locale);
    app.theme = UiThemeSpec::for_mode(UiThemeMode::Light);
    app.sync_views();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(w, h, 1.0), tokens)
}

fn verify_open_popup_and_dismiss(
    driver: &mut EguiHumanPathDriver<'_, ShellTestState>,
    menu_widget_id: &str,
    expected_z: ZLayer,
    tag: &str,
    failures: &mut Vec<String>,
) {
    let Some(menu_entry) = driver.frame_meta().find_by_widget_id(menu_widget_id).cloned() else {
        failures.push(format!("[{tag}] missing menu container `{menu_widget_id}`"));
        return;
    };
    if menu_entry.role != "menu" || menu_entry.z_layer != expected_z {
        failures.push(format!(
            "[{tag}] `{menu_widget_id}` expected role=\"menu\" z={expected_z:?}, got role={:?} z={:?}",
            menu_entry.role, menu_entry.z_layer
        ));
    }

    // Frame 1 (first painted frame during opening click/dispatch) vs settled frame.
    let observed = driver.observed_menu_rects(menu_widget_id);
    if let Some(&frame1_rect) = observed.first()
        && let Some(v) = check_popup_two_frame_stability(menu_widget_id, frame1_rect, menu_entry.unclipped_rect)
    {
        failures.push(format!("[{tag}] {} (open->settle): {}", v.check, v.detail));
    }

    // Full parity gate (all 4 single-frame popup geometry invariants + AccessKit + scrollbar + overlap + WCAG).
    let parity = driver.verify_pixel_to_hit_box_parity();
    for v in parity.violations {
        failures.push(format!("[{tag}] {}: {} — {}", v.check, v.widget_id, v.detail));
    }

    // Advance one explicit subsequent frame and verify Frame N -> Frame N+1 stability (<= 0.5 pt).
    let before_rect = menu_entry.unclipped_rect;
    driver.step_single_frame();
    match driver.frame_meta().find_by_widget_id(menu_widget_id) {
        Some(after) => {
            if let Some(v) = check_popup_two_frame_stability(menu_widget_id, before_rect, after.unclipped_rect) {
                failures.push(format!("[{tag}] {} (frame1->frame2): {}", v.check, v.detail));
            }
        }
        None => failures.push(format!("[{tag}] `{menu_widget_id}` vanished unexpectedly on Frame 2")),
    }

    // Escape must cleanly dismiss the popup.
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    if driver.frame_meta().find_by_widget_id(menu_widget_id).is_some() {
        failures.push(format!("[{tag}] `{menu_widget_id}` did not dismiss on Escape"));
    }
}

#[test]
fn popup_matrix_all_ten_menus_across_viewports_and_de_locale_pass_parity_and_two_frame_stability() {
    let base = build_popup_matrix_app();
    let configs = [
        ("640x420_en", "en", 640.0, 420.0),
        ("1280x800_en", "en", 1280.0, 800.0),
        ("640x420_de", "de", 640.0, 420.0),
        ("1280x800_de", "de", 1280.0, 800.0),
    ];

    let mut checked_cases = 0usize;
    let mut verified_menus = std::collections::BTreeSet::new();
    let mut failures = Vec::new();

    for &(cfg_name, locale, w, h) in &configs {
        let mut drv = make_matrix_driver(base.clone(), locale, w, h);

        // 0. Inline search dropdown (`toolbar.search_dropdown`) when `toolbar.search_input` is visible.
        select_sheet_and_settle(&mut drv, "ds-xy").unwrap();
        if drv.frame_meta().find_by_widget_id("toolbar.search_input").is_some_and(|e| e.enabled) {
            drv.click_widget_id("toolbar.search_input").unwrap();
            verify_open_popup_and_dismiss(
                &mut drv,
                "toolbar.search_dropdown",
                ZLayer::DropdownMenu,
                &format!("search_dropdown/{cfg_name}"),
                &mut failures,
            );
            verified_menus.insert("toolbar.search_dropdown");
            checked_cases += 1;
        }

        // 1. Split presets dropdown (`popup.toolbar.split`), plus right-edge right-alignment check.
        select_sheet_and_settle(&mut drv, "ds-xy").unwrap();
        let split_anchor = drv.frame_meta().find_by_widget_id("toolbar.split").map(|e| e.clipped_rect);
        if let Some(anchor) = split_anchor {
            drv.click_widget_id("toolbar.split").unwrap();
            if w >= 1000.0
                && let Some(menu) = drv.frame_meta().find_by_widget_id("popup.toolbar.split")
                && (menu.unclipped_rect.right() > anchor.right() + 4.0 || menu.unclipped_rect.left() >= anchor.left())
            {
                failures.push(format!(
                    "[split/{cfg_name}] expected right-aligned popup {:?} against right-edge trigger {:?}",
                    menu.unclipped_rect, anchor
                ));
            }
            verify_open_popup_and_dismiss(&mut drv, "popup.toolbar.split", ZLayer::DropdownMenu, &format!("split/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.toolbar.split");
            checked_cases += 1;
        }

        // 2. Toolbar `⋯` overflow dropdown (`popup.toolbar.overflow`) — guaranteed at 640×420 on ds-mv.
        select_sheet_and_settle(&mut drv, "ds-mv").unwrap();
        if drv.frame_meta().find_by_widget_id("toolbar.overflow").is_some_and(|e| e.enabled) {
            drv.click_widget_id("toolbar.overflow").unwrap();
            verify_open_popup_and_dismiss(
                &mut drv,
                "popup.toolbar.overflow",
                ZLayer::DropdownMenu,
                &format!("toolbar_overflow/{cfg_name}"),
                &mut failures,
            );
            verified_menus.insert("popup.toolbar.overflow");
            checked_cases += 1;
        }

        // 3. MV variable-role dropdown (`popup.mv.role`).
        if drv.frame_meta().find_by_widget_id("mv_formula.role_badge.0").is_some_and(|e| e.enabled) {
            drv.click_widget_id("mv_formula.role_badge.0").unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.mv.role", ZLayer::DropdownMenu, &format!("mv_role/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.mv.role");
            checked_cases += 1;
        }

        // 4. Data grid context menu (`popup.grid.ctx`).
        select_sheet_and_settle(&mut drv, "ds-xy").unwrap();
        if let Ok(row_pos) = widget_center(&drv, "grid.row_num.0") {
            drv.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default()).unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.grid.ctx", ZLayer::ContextMenu, &format!("grid_ctx/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.grid.ctx");
            checked_cases += 1;
        }

        // 5. Results tabs `⋯` overflow (`popup.result.tabs.overflow`) & 6. Results table context menu (`popup.result.ctx`).
        select_sheet_and_settle(&mut drv, "rs-anova").unwrap();
        if drv.frame_meta().find_by_widget_id("result.tabs.overflow").is_some_and(|e| e.enabled) {
            drv.click_widget_id("result.tabs.overflow").unwrap();
            verify_open_popup_and_dismiss(
                &mut drv,
                "popup.result.tabs.overflow",
                ZLayer::DropdownMenu,
                &format!("result_tabs_overflow/{cfg_name}"),
                &mut failures,
            );
            verified_menus.insert("popup.result.tabs.overflow");
            checked_cases += 1;
        }
        let first_res_cell = drv
            .frame_meta()
            .entries
            .iter()
            .find(|e| e.widget_id.starts_with("result.cell.") && e.clipped_rect.is_positive())
            .map(|e| e.clipped_rect.center());
        if let Some(cell_pos) = first_res_cell {
            drv.click_at_button(cell_pos, PointerButton::Secondary, KeyModifiers::default()).unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.result.ctx", ZLayer::ContextMenu, &format!("result_ctx/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.result.ctx");
            checked_cases += 1;
        }

        // 7. Footer upstream links popup (`popup.status.link_indicator`) on linked result sheet.
        if drv.frame_meta().find_by_widget_id("status.link_indicator").is_some_and(|e| e.enabled) {
            drv.click_widget_id("status.link_indicator").unwrap();
            verify_open_popup_and_dismiss(
                &mut drv,
                "popup.status.link_indicator",
                ZLayer::DropdownMenu,
                &format!("footer_link/{cfg_name}"),
                &mut failures,
            );
            verified_menus.insert("popup.status.link_indicator");
            checked_cases += 1;
        }

        // 8. Footer sheet switcher popup (`popup.status.sheet_switcher`) with navigator collapsed.
        if drv.frame_meta().find_by_widget_id("status.sheet_switcher").is_none()
            && drv.frame_meta().find_by_widget_id("nav.collapse_rail").is_some_and(|e| e.enabled)
        {
            drv.click_widget_id("nav.collapse_rail").unwrap();
        }
        if drv.frame_meta().find_by_widget_id("status.sheet_switcher").is_some_and(|e| e.enabled) {
            drv.click_widget_id("status.sheet_switcher").unwrap();
            verify_open_popup_and_dismiss(
                &mut drv,
                "popup.status.sheet_switcher",
                ZLayer::DropdownMenu,
                &format!("footer_sheet_switcher/{cfg_name}"),
                &mut failures,
            );
            verified_menus.insert("popup.status.sheet_switcher");
            checked_cases += 1;
        }

        // 9. Navigator sheet context menu (`popup.nav.ctx`).
        if drv.frame_meta().find_by_widget_id("nav.sheet.ds-xy").is_none()
            && drv.frame_meta().find_by_widget_id("nav.rail.expand").is_some_and(|e| e.enabled)
        {
            drv.click_widget_id("nav.rail.expand").unwrap();
        }
        if let Ok(nav_pos) = widget_center(&drv, "nav.sheet.ds-xy") {
            drv.click_at_button(nav_pos, PointerButton::Secondary, KeyModifiers::default()).unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.nav.ctx", ZLayer::ContextMenu, &format!("nav_ctx/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.nav.ctx");
            checked_cases += 1;
        }

        // 10. Graph canvas context menu (`popup.canvas.graph_ctx`).
        select_sheet_and_settle(&mut drv, "gs-xy").unwrap();
        if let Some(canvas_r) = drv.state().canvas_rect_pt {
            let pt = pos2(canvas_r.center().x, canvas_r.center().y);
            drv.click_at_button(pt, PointerButton::Secondary, KeyModifiers::default()).unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.canvas.graph_ctx", ZLayer::ContextMenu, &format!("graph_ctx/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.canvas.graph_ctx");
            checked_cases += 1;
        }

        // 11. Layout panel context menu (`popup.canvas.layout_ctx`).
        select_sheet_and_settle(&mut drv, "ls-fig").unwrap();
        if let Some(canvas_r) = drv.state().canvas_rect_pt {
            let pt = pos2(canvas_r.center().x, canvas_r.center().y);
            drv.click_at_button(pt, PointerButton::Secondary, KeyModifiers::default()).unwrap();
            verify_open_popup_and_dismiss(&mut drv, "popup.canvas.layout_ctx", ZLayer::ContextMenu, &format!("layout_ctx/{cfg_name}"), &mut failures);
            verified_menus.insert("popup.canvas.layout_ctx");
            checked_cases += 1;
        }
    }

    for expected in [
        "toolbar.search_dropdown",
        "popup.toolbar.split",
        "popup.toolbar.overflow",
        "popup.mv.role",
        "popup.grid.ctx",
        "popup.result.tabs.overflow",
        "popup.result.ctx",
        "popup.status.link_indicator",
        "popup.status.sheet_switcher",
        "popup.nav.ctx",
        "popup.canvas.graph_ctx",
        "popup.canvas.layout_ctx",
    ] {
        assert!(verified_menus.contains(expected), "popup_matrix missed menu `{expected}`; verified={verified_menus:?}");
    }
    assert!(checked_cases >= 38, "expected comprehensive cross-viewport popup coverage (>= 38 checks), ran {checked_cases}");
    assert!(failures.is_empty(), "popup_matrix failures ({}):\n{}", failures.len(), failures.join("\n"));
}
