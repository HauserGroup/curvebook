#![forbid(unsafe_code)]
//! `R15.1` Native `egui` Human-Path Driver, CJK/IME Composition, ScrollBar Non-Overlap,
//! and 14-Locale Beta Ellipsis & Unclipped-Bleed Gate.

use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{
    AccessKitRoleBounds, BETA_LOCALE_CODES, EguiHumanPathDriver, EllipsisLabelInput, TextBleedPolicy, accesskit_accessible_name,
    build_driver_font_definitions, check_role_bounds_scrollbar_non_overlap, collect_tooltip_texts_from_accesskit, render_tracked_label,
    verify_egui_frame_parity,
};
use curvebook_ui_egui::egui_shell::top_bar_fit::{BAR_ICON_PT, RightWidths, SearchVisibility, fit_right_group};
use curvebook_ui_egui::egui_shell::widgets::{check, flat_button_fit, icon_button, primary_button, secondary_button, section_toggle, tab_bar};
use curvebook_ui_egui::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens};
use curvebook_ui_egui::i18n::{LOCALES, is_rtl_locale, normalize_locale, tr};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

struct TestShellState {
    app: UiApp,
    intents: PendingUiIntents,
    search_query: String,
    active_tab: usize,
    auto_graph: bool,
    section_open: bool,
    disabled_btn_clicks: usize,
    splitter_offset_pt: f32,
    discard_once: bool,
    observed_discard_passes: usize,
    observed_settled_passes: usize,
    scroll_items: usize,
}

impl TestShellState {
    fn new(locale: &str, scroll_items: usize) -> Self {
        let mut app = UiApp::new();
        assert_eq!(app.set_locale(locale), locale);
        Self {
            app,
            intents: PendingUiIntents::new(),
            search_query: String::new(),
            active_tab: 0,
            auto_graph: true,
            section_open: true,
            disabled_btn_clicks: 0,
            splitter_offset_pt: 0.0,
            discard_once: false,
            observed_discard_passes: 0,
            observed_settled_passes: 0,
            scroll_items,
        }
    }
}

fn record_meta(
    ui: &egui::Ui,
    meta: &mut FrameInspectMeta,
    resp: &egui::Response,
    widget_id: &str,
    role: &str,
    label: &str,
    command_id: Option<&str>,
) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id,
            role,
            label,
            command_id,
            value: None,
            enabled: ui.is_enabled(),
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn render_test_shell(ui: &mut egui::Ui, meta: &mut FrameInspectMeta, state: &mut TestShellState) {
    let locale = state.app.locale.clone();
    let metrics = LayoutMetrics::CANONICAL;
    let total_w = ui.max_rect().width();

    if state.discard_once {
        state.discard_once = false;
        ui.ctx().request_discard("test multi-pass layout");
    }

    egui::Panel::top("shell.app_bar").exact_size(metrics.app_bar_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let home_resp = icon_button(ui, "search", tr(&locale, "cmd.ui.palette.open"), false, BAR_ICON_PT);
            record_meta(ui, meta, &home_resp, "appbar.palette", "Button", tr(&locale, "cmd.ui.palette.open"), Some("ui.palette.open"));

            let fit = fit_right_group(&RightWidths {
                room: (ui.available_width() - 8.0).max(0.0),
                has_pinned_badge: false,
                switcher: 120.0,
                field: 160.0,
                icon: BAR_ICON_PT,
                gap: 6.0,
                trailing_extra: 0.0,
            });
            if matches!(fit.search, SearchVisibility::Field) {
                let search_edit = ui.add_sized(
                    egui::vec2(160.0, 22.0),
                    egui::TextEdit::singleline(&mut state.search_query).hint_text(tr(&locale, "cmd.ui.palette.open")),
                );
                record_meta(ui, meta, &search_edit, "appbar.search_input", "TextBox", tr(&locale, "cmd.ui.palette.open"), None);
            }
            if fit.switcher {
                let new_lbl = tr(&locale, "menu.file.new_project");
                let new_resp = flat_button_fit(ui, new_lbl, 84.0);
                if new_resp.clicked() {
                    state
                        .intents
                        .push_engine("sheet.createData", json!({"sheet_id": "data-1", "title": "Assay 1", "kind": "column", "auto_graph": false}));
                }
                record_meta(ui, meta, &new_resp, "appbar.new_sheet", "Button", new_lbl, Some("sheet.createData"));
            }
        });
    });

    egui::Panel::top("shell.action_strip").exact_size(metrics.action_strip_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let analyze_lbl = tr(&locale, "cmd.ui.analyze.open");
            let analyze_resp = primary_button(ui, analyze_lbl);
            if analyze_resp.clicked() {
                state.intents.push_ui("ui.navigator.toggle", json!({}));
            }
            record_meta(ui, meta, &analyze_resp, "toolbar.analyze", "Button", analyze_lbl, Some("ui.analyze.open"));

            let cancel_lbl = tr(&locale, "btn.cancel");
            let cancel_resp = secondary_button(ui, cancel_lbl);
            record_meta(ui, meta, &cancel_resp, "toolbar.cancel", "Button", cancel_lbl, None);

            let gated_lbl = tr(&locale, "btn.ok");
            let gated_resp = ui.add_enabled(false, egui::Button::new(gated_lbl));
            if gated_resp.clicked() {
                state.disabled_btn_clicks += 1;
            }
            meta.record_response(
                ui,
                &gated_resp,
                FrameInspectEntryInput {
                    widget_id: "toolbar.gated_ok",
                    role: "Button",
                    label: gated_lbl,
                    command_id: None,
                    value: None,
                    enabled: false,
                    disabled_reason: Some("Requires valid selection"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        });
    });

    egui::Panel::bottom("shell.status_bar").exact_size(metrics.status_bar_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let _ = render_tracked_label(
                ui,
                meta,
                EllipsisLabelInput {
                    widget_id: "status.summary",
                    text: tr(&locale, "table.multiple_variables"),
                    truncate: true,
                    z_layer: ZLayer::Workspace,
                },
            );
        });
    });

    let nav_w = if total_w < metrics.navigator_collapse_breakpoint_w { metrics.dock_collapsed_rail_w } else { metrics.navigator_default_w };
    egui::Panel::left("shell.navigator").exact_size(nav_w).show(ui, |ui| {
        if nav_w > metrics.dock_collapsed_rail_w + 10.0 {
            let _ = section_toggle(ui, tr(&locale, "nav.data_tables"), state.section_open);
            ui.allocate_ui(egui::vec2(56.0, 22.0), |ui| {
                let _ = render_tracked_label(
                    ui,
                    meta,
                    EllipsisLabelInput {
                        widget_id: "nav.truncated_item",
                        text: tr(&locale, "cmd.table.excludeCellsWithReason"),
                        truncate: true,
                        z_layer: ZLayer::Workspace,
                    },
                );
            });
        } else {
            let _ = icon_button(ui, "+", tr(&locale, "nav.data_tables"), true, 24.0);
        }
    });

    let insp_w = if total_w < metrics.inspector_collapse_breakpoint_w { metrics.dock_collapsed_rail_w } else { metrics.inspector_default_w };
    egui::Panel::right("shell.inspector").exact_size(insp_w).show(ui, |ui| {
        if insp_w > metrics.dock_collapsed_rail_w + 10.0 {
            let _ = check(ui, tr(&locale, "table.multiple_variables"), state.auto_graph, true);
            let row_h = metrics.min_hit_target_pt;
            egui::ScrollArea::vertical().id_salt("inspector.scroll").auto_shrink([false, false]).show_rows(
                ui,
                row_h,
                state.scroll_items,
                |ui, row_range| {
                    for idx in row_range {
                        let row_lbl = format!("{} #{idx}", tr(&locale, "nav.results"));
                        let row_resp = ui.add_sized(egui::vec2(ui.available_width(), row_h), egui::Button::new(row_lbl.as_str()).truncate());
                        if ui.is_rect_visible(row_resp.rect) {
                            record_meta(ui, meta, &row_resp, &format!("inspector.row.{idx}"), "Button", &row_lbl, None);
                        }
                    }
                },
            );
        } else {
            for idx in 0..3 {
                let tip = format!("{} #{idx}", tr(&locale, "nav.results"));
                let r = icon_button(ui, "+", &tip, idx == 0, metrics.min_hit_target_pt);
                record_meta(ui, meta, &r, &format!("inspector.rail.{idx}"), "Button", &tip, None);
            }
        }
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let tabs = [tr(&locale, "nav.data_tables"), tr(&locale, "nav.results")];
        if let Some(next_tab) = tab_bar(ui, &tabs, state.active_tab) {
            state.active_tab = next_tab;
        }
        ui.separator();

        let (splitter_rect, splitter_resp) = ui.allocate_exact_size(egui::vec2(120.0, 20.0), egui::Sense::drag());
        ui.painter().rect_filled(splitter_rect, 2.0, Tokens::get(ui.ctx()).divider);
        splitter_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, true, "Workspace Split Handle"));
        if splitter_resp.dragged() {
            state.splitter_offset_pt += splitter_resp.drag_delta().x;
        }
        record_meta(ui, meta, &splitter_resp, "center.splitter", "Splitter", "Workspace Split Handle", None);

        let formula_edit = ui.add(egui::TextEdit::singleline(&mut state.search_query).hint_text("fx = "));
        record_meta(ui, meta, &formula_edit, "center.formula_input", "TextBox", "fx", None);

        ui.add_space(4.0);
        let center_row_h = metrics.grid_row_h;
        egui::ScrollArea::vertical().id_salt("center.scroll").auto_shrink([false, false]).show_rows(
            ui,
            center_row_h,
            state.scroll_items,
            |ui, row_range| {
                for idx in row_range {
                    let row_lbl = format!("{} #{idx}", tr(&locale, "nav.data_tables"));
                    let row_resp = ui.add_sized(egui::vec2(ui.available_width(), center_row_h), egui::Button::new(row_lbl.as_str()).truncate());
                    if ui.is_rect_visible(row_resp.rect) {
                        record_meta(ui, meta, &row_resp, &format!("center.row.{idx}"), "Button", &row_lbl, None);
                    }
                }
            },
        );
    });

    if ui.ctx().will_discard() {
        state.observed_discard_passes += 1;
        state.intents.clear();
    } else {
        state.observed_settled_passes += 1;
        let _ = state.intents.commit_if_settled(&mut state.app, false);
    }
}

#[test]
fn egui_driver_drives_shell_view_and_verifies_accesskit_and_kittest_parity() -> Result<(), String> {
    for mode in UiThemeMode::ALL {
        for &scale in &[1.0_f64, 1.5, 2.0] {
            let tokens = Tokens::from_theme_spec(&curvebook_ui_egui::theme::UiThemeSpec::for_mode(mode), "en");
            let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
                TestShellState::new("en", 24),
                ViewportMetrics::new(1440.0, 600.0, scale),
                tokens,
                build_driver_font_definitions(),
                render_test_shell,
            );
            driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

            let report_wide = driver.verify_pixel_to_hit_box_parity();
            assert!(report_wide.is_clean(), "{mode:?}@{scale}x wide parity failed: {:?}", report_wide.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty());

            // Compact 640x420 breakpoint + scrollbar non-overlap check
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, scale));
            assert!(driver.geometry_stamp().matches_target(
                driver.metrics().physical_width_px(),
                driver.metrics().physical_height_px(),
                scale as f32
            ));
            let report_compact = driver.verify_pixel_to_hit_box_parity();
            assert!(report_compact.is_clean(), "{mode:?}@{scale}x compact parity failed: {:?}", report_compact.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty());
        }
    }

    // End-to-end human interactions: click_widget_id -> single Session::execute commit, drag, wheel_at, type_text, press_key
    let mut driver = EguiHumanPathDriver::new(TestShellState::new("en", 30), ViewportMetrics::new(1280.0, 800.0, 1.0), render_test_shell);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    assert_eq!(driver.state().app.document().total_sheet_count(), 0);
    driver.click_widget_id("appbar.new_sheet")?;
    assert_eq!(driver.state().app.document().total_sheet_count(), 1, "click_widget_id must commit once via PendingUiIntents");
    assert_eq!(driver.state().app.session().undo_depth(), 1);

    // Disabled widget must be rejected with its disabled_reason and never actuate
    let disabled_err = driver.click_widget_id("toolbar.gated_ok").expect_err("clicking disabled widget must return disabled_reason");
    assert!(disabled_err.contains("Requires valid selection"), "unexpected disabled error: {disabled_err}");
    assert_eq!(driver.state().disabled_btn_clicks, 0);

    // Direct coordinate click_at on the second tab
    let tab_pos = driver.frame_meta().find_by_widget_id("toolbar.analyze").ok_or("missing toolbar.analyze")?.clipped_rect.center();
    let nav_before = driver.state().app.nav.navigator_open;
    driver.click_at(tab_pos)?;
    assert_ne!(driver.state().app.nav.navigator_open, nav_before, "click_at must toggle navigator via PendingUiIntents");

    // Drag splitter handle via raw PointerMoved + PointerButton
    let splitter_center = driver.frame_meta().find_by_widget_id("center.splitter").ok_or("missing center.splitter")?.clipped_rect.center();
    driver.drag(splitter_center, splitter_center + egui::vec2(24.0, 0.0))?;
    assert!(driver.state().splitter_offset_pt > 10.0, "expected splitter drag delta, got {}", driver.state().splitter_offset_pt);

    // Wheel over inspector scroll area
    let row0_before = driver.frame_meta().find_by_widget_id("inspector.row.0").ok_or("missing inspector.row.0")?.clipped_rect.top();
    let insp_pos = driver.frame_meta().find_by_widget_id("inspector.row.1").ok_or("missing inspector.row.1")?.clipped_rect.center();
    driver.wheel_at(insp_pos, egui::vec2(0.0, -80.0))?;
    let row0_after = driver.frame_meta().find_by_widget_id("inspector.row.0").map_or(f32::NEG_INFINITY, |entry| entry.clipped_rect.top());
    assert!(row0_after < row0_before, "wheel_at must scroll inspector downward ({row0_before} -> {row0_after})");

    // Type text + Backspace key via raw events into TextEdit
    driver.click_widget_id("center.formula_input")?;
    driver.type_text("mean(A)")?;
    assert_eq!(driver.state().search_query, "mean(A)");
    driver.press_key("Backspace", KeyModifiers::default())?;
    assert_eq!(driver.state().search_query, "mean(A");

    // Verify egui_kittest::Harness AccessKit completeness & Role::ScrollBar non-overlap at 640x420 and 1440x600
    for (w, h) in [(640.0_f32, 420.0_f32), (1440.0_f32, 600.0_f32)] {
        let fonts = build_driver_font_definitions();
        let mut harness = Harness::builder().with_size(egui::vec2(w, h)).allow_missing_glyphs().build_ui_state(
            |ui, state| {
                Tokens::default().apply_to_ctx(ui.ctx());
                let mut meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                render_test_shell(ui, &mut meta, state);
            },
            TestShellState::new("en", 32),
        );
        harness.ctx.set_fonts(fonts);
        harness.run();
        harness.check_accessibility();
        let scrollbars = harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count();
        assert!(scrollbars >= 1, "{w}x{h}: expected active Role::ScrollBar node in AccessKit");
        let bounds: Vec<AccessKitRoleBounds> = harness
            .root()
            .children_recursive()
            .filter_map(|n| {
                let ak = n.accesskit_node();
                if ak.is_hidden() || ak.bounding_box().is_none() {
                    return None;
                }
                Some(AccessKitRoleBounds {
                    id: ak.locate().0,
                    role: ak.role(),
                    label: ak.label().or_else(|| ak.value()).unwrap_or_else(|| String::from("<unnamed>")),
                    rect: n.rect(),
                })
            })
            .collect();
        let sb_violations = check_role_bounds_scrollbar_non_overlap(&bounds);
        assert!(sb_violations.is_empty(), "{w}x{h} ScrollBar overlap violations: {sb_violations:?}");
    }

    Ok(())
}

#[test]
fn egui_driver_cjk_rendering_and_ime_preedit_commit() -> Result<(), String> {
    #[derive(Default)]
    struct ImeState {
        locale: String,
        value: String,
    }

    let cases = [
        ("ja", "にほん", "日本語", "メニュー"),
        ("zh-Hans", "nihao", "你好数据", "数据表"),
        ("zh-TW", "fanti", "繁體中文", "圖表"),
        ("ko", "gks", "한국어", "데이터"),
    ];

    for (raw_locale, preedit, commit, extra_label) in cases {
        let canonical = normalize_locale(raw_locale);
        let tokens = Tokens::from_theme_spec(&curvebook_ui_egui::theme::UiThemeSpec::for_mode(UiThemeMode::Light), canonical);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            ImeState { locale: canonical.to_owned(), value: String::new() },
            ViewportMetrics::new(800.0, 500.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            |ui, meta, state| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label(tr(&state.locale, "menu.file.new_project"));
                    ui.label(extra_label);
                    let edit = ui.add(egui::TextEdit::singleline(&mut state.value).hint_text("IME Input"));
                    record_meta(ui, meta, &edit, "ime.field", "TextBox", "IME Input", None);
                });
            },
        );

        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "{canonical} parity failed: {:?}", parity.violations);

        driver.click_widget_id("ime.field")?;
        driver.ime_preedit(preedit, Some(0..preedit.chars().count()));
        assert_eq!(driver.state().value, preedit, "{canonical}: preedit text must appear during composition");
        assert!(driver.last_ime_output().is_some(), "{canonical}: egui must emit IME cursor/rect output");

        driver.ime_commit(commit);
        assert_eq!(driver.state().value, commit, "{canonical}: commit must replace preedit cleanly");

        // Verify empty preedit cancels cleanly without deleting committed text
        driver.ime_preedit("temp", Some(0..4));
        assert_eq!(driver.state().value, format!("{commit}temp"));
        driver.ime_preedit("", None);
        assert_eq!(driver.state().value, commit, "{canonical}: empty preedit must restore committed text");
    }

    // Also drive Event::Ime through egui_kittest::Harness directly
    let fonts = build_driver_font_definitions();
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 420.0)).allow_missing_glyphs().build_ui_state(
        |ui, state: &mut String| {
            Tokens::default().apply_to_ctx(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                let lbl = ui.label("CJK Field");
                ui.add(egui::TextEdit::singleline(state)).labelled_by(lbl.id);
            });
        },
        String::new(),
    );
    harness.ctx.set_fonts(fonts);
    harness.run();
    harness.get_by_role(egui::accesskit::Role::TextInput).click();
    harness.run();
    harness.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: String::from("にほん"), active_range_chars: Some(0..3) }));
    harness.run();
    assert_eq!(harness.state(), "にほん");
    harness.event(egui::Event::Ime(egui::ImeEvent::Commit(String::from("日本"))));
    harness.run();
    assert_eq!(harness.state(), "日本");
    Ok(())
}

#[test]
fn egui_driver_14_beta_locales_zero_bleed_and_ellipsis_tooltip_a11y() -> Result<(), String> {
    let non_rtl: Vec<&'static str> = LOCALES.iter().copied().filter(|loc| !is_rtl_locale(loc)).collect();
    assert_eq!(non_rtl.as_slice(), &BETA_LOCALE_CODES);
    assert_eq!(normalize_locale("pt"), "pt-BR");
    assert_eq!(normalize_locale("zh-Hans"), "zh-CN");

    for &locale in &BETA_LOCALE_CODES {
        let tokens = Tokens::from_theme_spec(&curvebook_ui_egui::theme::UiThemeSpec::for_mode(UiThemeMode::Light), locale);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            TestShellState::new(locale, 18),
            ViewportMetrics::new(1280.0, 800.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            render_test_shell,
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

        let report_desktop = driver.verify_pixel_to_hit_box_parity();
        assert!(report_desktop.is_clean(), "locale `{locale}` 1280x800 shell parity/bleed failed: {:?}", report_desktop.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "locale `{locale}` 1280x800 scrollbar overlapped child controls");

        let full_text = tr(locale, "cmd.table.excludeCellsWithReason");
        driver.verify_ellipsis_tooltip_and_accesskit("nav.truncated_item", full_text)?;

        // Check AccessKit tree directly for full label + tooltip subtree
        let ak = driver.accesskit_tree().ok_or("missing accesskit update")?;
        let tooltips = collect_tooltip_texts_from_accesskit(&ak.nodes);
        assert!(tooltips.iter().any(|t| t == full_text), "locale `{locale}` hover tooltip did not emit `{full_text}`: {tooltips:?}");
        let entry = driver.frame_meta().find_by_widget_id("nav.truncated_item").ok_or("missing nav.truncated_item")?;
        assert!(entry.ellipsized, "locale `{locale}` expected `ellipsized == true`");
        let ak_node = ak.nodes.iter().find(|(id, _)| *id == entry.accesskit_id).map(|(_, n)| n).ok_or("missing ak node")?;
        assert_eq!(accesskit_accessible_name(ak_node), Some(full_text));

        // Also check 640x420 compact auto-collapsed viewport for zero unclipped bleed & scrollbar non-overlap
        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
        let report_compact = driver.verify_pixel_to_hit_box_parity();
        assert!(report_compact.is_clean(), "locale `{locale}` 640x420 compact parity/bleed failed: {:?}", report_compact.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "locale `{locale}` 640x420 scrollbar overlapped child controls");
    }

    // Negative test: verify an unclipped overflowing label triggers `text_unclipped_bleed`
    let mut bleed_driver = EguiHumanPathDriver::new((), ViewportMetrics::new(220.0, 120.0, 1.0), |ui, meta, ()| {
        egui::Panel::left("narrow.bleed").exact_size(60.0).show(ui, |ui| {
            let _ = render_tracked_label(
                ui,
                meta,
                EllipsisLabelInput {
                    widget_id: "lbl.bleeding",
                    text: "Unclipped overflowing scientific label text beyond 60pt panel",
                    truncate: false,
                    z_layer: ZLayer::Workspace,
                },
            );
        });
        egui::CentralPanel::default().show(ui, |_ui| {});
    });
    bleed_driver.set_bleed_policy(TextBleedPolicy::StrictAllEdges);
    let bleed_report = bleed_driver.verify_pixel_to_hit_box_parity();
    assert!(
        bleed_report.violations.iter().any(|v| v.check == "text_unclipped_bleed"),
        "expected unclipped bleed detector to catch overflowing non-truncated label: {:?}",
        bleed_report.violations
    );
    Ok(())
}

#[test]
fn egui_driver_never_executes_session_during_discard_passes() -> Result<(), String> {
    let mut driver = EguiHumanPathDriver::new(TestShellState::new("en", 10), ViewportMetrics::new(1280.0, 800.0, 1.0), render_test_shell);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.state_mut().discard_once = true;
    driver.click_widget_id("appbar.new_sheet")?;

    assert!(driver.state().observed_discard_passes >= 1, "expected at least one discarded layout pass");
    assert_eq!(driver.state().app.document().total_sheet_count(), 1, "discarded pass must not duplicate Session::execute");
    assert_eq!(driver.state().app.session().undo_depth(), 1);

    let report =
        verify_egui_frame_parity(driver.accesskit_tree(), driver.frame_meta(), &[], driver.viewport_rect(), TextBleedPolicy::AllowVerticalScrollClip);
    assert!(report.is_clean(), "expected clean frame parity: {:?}", report.violations);
    Ok(())
}
