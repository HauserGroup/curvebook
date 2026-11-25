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
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use crate::common::{assert_clean_parity, widget_center};

struct TestShellState {
    app: UiApp,
    intents: PendingUiIntents,
    search_query: String,
    active_tab: usize,
    disabled_btn_clicks: usize,
    splitter_offset_pt: f32,
    discard_once: bool,
    observed_discard_passes: usize,
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
            disabled_btn_clicks: 0,
            splitter_offset_pt: 0.0,
            discard_once: false,
            observed_discard_passes: 0,
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
    let m = LayoutMetrics::CANONICAL;
    let total_w = ui.max_rect().width();
    if state.discard_once {
        state.discard_once = false;
        ui.ctx().request_discard("test multi-pass layout");
    }

    egui::Panel::top("shell.app_bar").exact_size(m.app_bar_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let home = icon_button(ui, "search", tr(&locale, "cmd.ui.palette.open"), false, BAR_ICON_PT);
            record_meta(ui, meta, &home, "appbar.palette", "Button", tr(&locale, "cmd.ui.palette.open"), Some("ui.palette.open"));
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
                let edit = ui.add_sized(
                    egui::vec2(160.0, 22.0),
                    egui::TextEdit::singleline(&mut state.search_query).hint_text(tr(&locale, "cmd.ui.palette.open")),
                );
                record_meta(ui, meta, &edit, "appbar.search_input", "TextBox", tr(&locale, "cmd.ui.palette.open"), None);
            }
            if fit.switcher {
                let new_lbl = tr(&locale, "menu.file.new_project");
                let r = flat_button_fit(ui, new_lbl, 84.0);
                if r.clicked() {
                    state
                        .intents
                        .push_engine("sheet.createData", json!({"sheet_id": "data-1", "title": "Assay 1", "kind": "column", "auto_graph": false}));
                }
                record_meta(ui, meta, &r, "appbar.new_sheet", "Button", new_lbl, Some("sheet.createData"));
            }
        });
    });

    egui::Panel::top("shell.action_strip").exact_size(m.action_strip_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let analyze_lbl = tr(&locale, "cmd.ui.analyze.open");
            let analyze_resp = primary_button(ui, analyze_lbl);
            if analyze_resp.clicked() {
                state.intents.push_ui("ui.navigator.toggle", json!({}));
            }
            record_meta(ui, meta, &analyze_resp, "toolbar.analyze", "Button", analyze_lbl, Some("ui.analyze.open"));
            let cancel_resp = secondary_button(ui, tr(&locale, "btn.cancel"));
            record_meta(ui, meta, &cancel_resp, "toolbar.cancel", "Button", tr(&locale, "btn.cancel"), None);
            let gated_resp = ui.add_enabled(false, egui::Button::new(tr(&locale, "btn.ok")));
            if gated_resp.clicked() {
                state.disabled_btn_clicks += 1;
            }
            meta.record_response(
                ui,
                &gated_resp,
                FrameInspectEntryInput {
                    widget_id: "toolbar.gated_ok",
                    role: "Button",
                    label: tr(&locale, "btn.ok"),
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

    egui::Panel::bottom("shell.status_bar").exact_size(m.status_bar_h).show(ui, |ui| {
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

    let nav_w = if total_w < m.navigator_collapse_breakpoint_w { m.dock_collapsed_rail_w } else { m.navigator_default_w };
    egui::Panel::left("shell.navigator").exact_size(nav_w).show(ui, |ui| {
        if nav_w > m.dock_collapsed_rail_w + 10.0 {
            let _ = section_toggle(ui, tr(&locale, "nav.data_tables"), true);
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

    let insp_w = if total_w < m.inspector_collapse_breakpoint_w { m.dock_collapsed_rail_w } else { m.inspector_default_w };
    egui::Panel::right("shell.inspector").exact_size(insp_w).show(ui, |ui| {
        if insp_w > m.dock_collapsed_rail_w + 10.0 {
            let _ = check(ui, tr(&locale, "table.multiple_variables"), true, true);
            egui::ScrollArea::vertical().id_salt("inspector.scroll").auto_shrink([false, false]).show_rows(
                ui,
                m.min_hit_target_pt,
                state.scroll_items,
                |ui, range| {
                    for idx in range {
                        let lbl = format!("{} #{idx}", tr(&locale, "nav.results"));
                        let r = ui.add_sized(egui::vec2(ui.available_width(), m.min_hit_target_pt), egui::Button::new(lbl.as_str()).truncate());
                        if ui.is_rect_visible(r.rect) {
                            record_meta(ui, meta, &r, &format!("inspector.row.{idx}"), "Button", &lbl, None);
                        }
                    }
                },
            );
        } else {
            for idx in 0..3 {
                let tip = format!("{} #{idx}", tr(&locale, "nav.results"));
                let r = icon_button(ui, "+", &tip, idx == 0, m.min_hit_target_pt);
                record_meta(ui, meta, &r, &format!("inspector.rail.{idx}"), "Button", &tip, None);
            }
        }
    });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(next_tab) = tab_bar(ui, &[tr(&locale, "nav.data_tables"), tr(&locale, "nav.results")], state.active_tab) {
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
        let edit = ui.add(egui::TextEdit::singleline(&mut state.search_query).hint_text("fx = "));
        record_meta(ui, meta, &edit, "center.formula_input", "TextBox", "fx", None);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("center.scroll").auto_shrink([false, false]).show_rows(
            ui,
            m.grid_row_h,
            state.scroll_items,
            |ui, range| {
                for idx in range {
                    let lbl = format!("{} #{idx}", tr(&locale, "nav.data_tables"));
                    let r = ui.add_sized(egui::vec2(ui.available_width(), m.grid_row_h), egui::Button::new(lbl.as_str()).truncate());
                    if ui.is_rect_visible(r.rect) {
                        record_meta(ui, meta, &r, &format!("center.row.{idx}"), "Button", &lbl, None);
                    }
                }
            },
        );
    });

    if ui.ctx().will_discard() {
        state.observed_discard_passes += 1;
        state.intents.clear();
    } else {
        let _ = state.intents.commit_if_settled(&mut state.app, false);
    }
}

#[test]
fn egui_driver_drives_shell_view_and_verifies_accesskit_and_kittest_parity() -> Result<(), String> {
    for mode in UiThemeMode::ALL {
        for &scale in &[1.0_f64, 1.5, 2.0] {
            let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
                TestShellState::new("en", 24),
                ViewportMetrics::new(1440.0, 600.0, scale),
                Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), "en"),
                build_driver_font_definitions(),
                render_test_shell,
            );
            driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
            assert_clean_parity(&driver, &format!("{mode:?}@{scale}x wide"))?;
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, scale));
            assert!(driver.geometry_stamp().matches_target(
                driver.metrics().physical_width_px(),
                driver.metrics().physical_height_px(),
                scale as f32
            ));
            assert_clean_parity(&driver, &format!("{mode:?}@{scale}x compact"))?;
        }
    }

    let mut driver = EguiHumanPathDriver::new(TestShellState::new("en", 30), ViewportMetrics::new(1280.0, 800.0, 1.0), render_test_shell);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.click_widget_id("appbar.new_sheet")?;
    assert_eq!((driver.state().app.document().total_sheet_count(), driver.state().app.session().undo_depth()), (1, 1));
    assert!(driver.click_widget_id("toolbar.gated_ok").expect_err("disabled must fail").contains("Requires valid selection"));
    assert_eq!(driver.state().disabled_btn_clicks, 0);

    let nav_before = driver.state().app.nav.navigator_open;
    driver.click_at(widget_center(&driver, "toolbar.analyze")?)?;
    assert_ne!(driver.state().app.nav.navigator_open, nav_before);

    let sc = widget_center(&driver, "center.splitter")?;
    driver.drag(sc, sc + egui::vec2(24.0, 0.0))?;
    assert!(driver.state().splitter_offset_pt > 10.0);

    let row0_before = driver.frame_meta().find_by_widget_id("inspector.row.0").ok_or("missing row0")?.clipped_rect.top();
    driver.wheel_at(widget_center(&driver, "inspector.row.1")?, egui::vec2(0.0, -80.0))?;
    let row0_after = driver.frame_meta().find_by_widget_id("inspector.row.0").map_or(f32::NEG_INFINITY, |e| e.clipped_rect.top());
    assert!(row0_after < row0_before);

    driver.click_widget_id("center.formula_input")?;
    driver.type_text("mean(A)")?;
    driver.press_key("Backspace", KeyModifiers::default())?;
    assert_eq!(driver.state().search_query, "mean(A");

    for (w, h) in [(640.0_f32, 420.0_f32), (1440.0_f32, 600.0_f32)] {
        let mut harness = Harness::builder().with_size(egui::vec2(w, h)).allow_missing_glyphs().build_ui_state(
            |ui, state| {
                Tokens::default().apply_to_ctx(ui.ctx());
                let mut meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                render_test_shell(ui, &mut meta, state);
            },
            TestShellState::new("en", 32),
        );
        harness.ctx.set_fonts(build_driver_font_definitions());
        harness.run();
        harness.check_accessibility();
        assert!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count() >= 1);
        let bounds: Vec<AccessKitRoleBounds> = harness
            .root()
            .children_recursive()
            .filter_map(|n| {
                let ak = n.accesskit_node();
                (!ak.is_hidden() && ak.bounding_box().is_some()).then(|| AccessKitRoleBounds {
                    id: ak.locate().0,
                    role: ak.role(),
                    label: ak.label().or_else(|| ak.value()).unwrap_or_else(|| String::from("<unnamed>")),
                    rect: n.rect(),
                })
            })
            .collect();
        assert!(check_role_bounds_scrollbar_non_overlap(&bounds).is_empty());
    }
    Ok(())
}

#[test]
fn egui_driver_cjk_rendering_and_ime_preedit_commit() -> Result<(), String> {
    for (raw_locale, preedit, commit, extra) in [
        ("ja", "にほん", "日本語", "メニュー"),
        ("zh-Hans", "nihao", "你好数据", "数据表"),
        ("zh-TW", "fanti", "繁體中文", "圖表"),
        ("ko", "gks", "한국어", "데이터"),
    ] {
        let canonical = normalize_locale(raw_locale);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            (canonical.to_owned(), String::new()),
            ViewportMetrics::new(800.0, 500.0, 1.0),
            Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), canonical),
            build_driver_font_definitions(),
            |ui, meta, (loc, val)| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label(tr(loc, "menu.file.new_project"));
                    ui.label(extra);
                    let edit = ui.add(egui::TextEdit::singleline(val).hint_text("IME Input"));
                    record_meta(ui, meta, &edit, "ime.field", "TextBox", "IME Input", None);
                });
            },
        );
        assert!(driver.verify_pixel_to_hit_box_parity().is_clean());
        driver.click_widget_id("ime.field")?;
        driver.ime_preedit(preedit, Some(0..preedit.chars().count()));
        assert!(driver.state().1 == preedit && driver.last_ime_output().is_some());
        driver.ime_commit(commit);
        assert_eq!(driver.state().1, commit);
        driver.ime_preedit("temp", Some(0..4));
        assert_eq!(driver.state().1, format!("{commit}temp"));
        driver.ime_preedit("", None);
        assert_eq!(driver.state().1, commit);
    }

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
    harness.ctx.set_fonts(build_driver_font_definitions());
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
    assert_eq!(LOCALES.iter().copied().filter(|loc| !is_rtl_locale(loc)).collect::<Vec<_>>().as_slice(), &BETA_LOCALE_CODES);
    assert_eq!((normalize_locale("pt"), normalize_locale("zh-Hans")), ("pt-BR", "zh-CN"));

    for &locale in &BETA_LOCALE_CODES {
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            TestShellState::new(locale, 18),
            ViewportMetrics::new(1280.0, 800.0, 1.0),
            Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), locale),
            build_driver_font_definitions(),
            render_test_shell,
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        assert_clean_parity(&driver, &format!("{locale} desktop"))?;

        let full_text = tr(locale, "cmd.table.excludeCellsWithReason");
        driver.verify_ellipsis_tooltip_and_accesskit("nav.truncated_item", full_text)?;
        let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
        assert!(collect_tooltip_texts_from_accesskit(&ak.nodes).iter().any(|t| t == full_text));
        let entry = driver.frame_meta().find_by_widget_id("nav.truncated_item").ok_or("missing truncated")?;
        assert!(entry.ellipsized);
        assert_eq!(ak.nodes.iter().find(|(id, _)| *id == entry.accesskit_id).and_then(|(_, n)| accesskit_accessible_name(n)), Some(full_text));

        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
        assert_clean_parity(&driver, &format!("{locale} compact"))?;
    }

    let mut bleed = EguiHumanPathDriver::new((), ViewportMetrics::new(220.0, 120.0, 1.0), |ui, meta, ()| {
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
        egui::CentralPanel::default().show(ui, |_| {});
    });
    bleed.set_bleed_policy(TextBleedPolicy::StrictAllEdges);
    assert!(bleed.verify_pixel_to_hit_box_parity().violations.iter().any(|v| v.check == "text_unclipped_bleed"));
    Ok(())
}

#[test]
fn egui_driver_never_executes_session_during_discard_passes() -> Result<(), String> {
    let mut driver = EguiHumanPathDriver::new(TestShellState::new("en", 10), ViewportMetrics::new(1280.0, 800.0, 1.0), render_test_shell);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.state_mut().discard_once = true;
    driver.click_widget_id("appbar.new_sheet")?;
    assert!(driver.state().observed_discard_passes >= 1);
    assert_eq!((driver.state().app.document().total_sheet_count(), driver.state().app.session().undo_depth()), (1, 1));
    assert!(
        verify_egui_frame_parity(driver.accesskit_tree(), driver.frame_meta(), &[], driver.viewport_rect(), TextBleedPolicy::AllowVerticalScrollClip)
            .is_clean()
    );
    Ok(())
}
