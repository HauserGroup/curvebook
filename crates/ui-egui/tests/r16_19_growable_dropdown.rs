#![forbid(unsafe_code)]

//! `R16.19` Verification of upstream `egui::ComboBox` #5225 on pinned `egui 0.37` and
//! replacement with growable [`widgets::dropdown`] (`stat_anova_modal.rs` & `center_stage.rs`).

use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, MultiplicityMethod, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::center_stage::{CenterStageContext, CenterStageFrameReport, render_center_stage};
use curvebook_ui_egui::egui_shell::driver::{
    AccessKitRoleBounds, EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions,
    check_role_bounds_scrollbar_non_overlap,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::{HybridCoexistenceArbiter, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::egui_shell::widgets::dropdown;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens, UiBackend};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

#[derive(Default)]
struct ComboProbeState {
    item_count: usize,
    selected: usize,
    popup_clip_rect: Option<egui::Rect>,
    popup_min_rect: Option<egui::Rect>,
    popup_max_rect: Option<egui::Rect>,
    item_rects: Vec<egui::Rect>,
    item_clipped: Vec<bool>,
}

impl ComboProbeState {
    fn with_items(item_count: usize) -> Self {
        Self { item_count, ..Self::default() }
    }
}

#[test]
#[allow(clippy::disallowed_types)] // Deliberate baseline reproduction of upstream egui::ComboBox #5225 on egui 0.37.0.
fn egui_combobox_5225_reproduces_on_pinned_egui_0_37() {
    let fonts = build_driver_font_definitions();
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 480.0)).allow_missing_glyphs().build_ui_state(
        |ui, state: &mut ComboProbeState| {
            Tokens::default().apply_to_ctx(ui.ctx());
            state.popup_clip_rect = None;
            state.popup_min_rect = None;
            state.popup_max_rect = None;
            state.item_rects.clear();
            state.item_clipped.clear();

            egui::CentralPanel::default().show(ui, |ui| {
                egui::ComboBox::from_label("Probe Combo").selected_text(format!("Option {}", state.selected)).width(180.0).show_ui(ui, |ui| {
                    state.popup_clip_rect = Some(ui.clip_rect());
                    state.popup_max_rect = Some(ui.max_rect());
                    for idx in 0..state.item_count {
                        let resp = ui.selectable_value(&mut state.selected, idx, format!("Option {idx}"));
                        state.item_rects.push(resp.rect);
                        state.item_clipped.push(!ui.clip_rect().contains_rect(resp.rect));
                    }
                    state.popup_min_rect = Some(ui.min_rect());
                });
            });
        },
        ComboProbeState::with_items(2),
    );
    harness.ctx.set_fonts(fonts);
    harness.run();

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();

    let clip_2 = harness.state().popup_clip_rect.expect("popup clip rect at 2 items");
    let min_2 = harness.state().popup_min_rect.expect("popup min rect at 2 items");
    assert!(min_2.height() < 70.0, "2-item opening min height expected compact, got {}", min_2.height());
    assert!(harness.query_by_label("Option 1").is_some(), "Option 1 must be present at 2 items");

    // Grow dynamically from 2 items to 8 items while popup stays open.
    harness.state_mut().item_count = 8;
    for _ in 0..6 {
        harness.step();
    }
    harness.run();

    let clip_8 = harness.state().popup_clip_rect.expect("popup clip rect after growing to 8 items");
    let min_8 = harness.state().popup_min_rect.expect("popup min rect after growing to 8 items");
    let item7_clipped = harness.state().item_clipped.get(7).copied().unwrap_or(false);

    // #5225 signature on egui 0.37.0: inner content wants >200pt, but popup clip rect remains trapped at 2-item height (64pt).
    assert!(min_8.height() > 180.0, "8 items require >180pt content height, got {}", min_8.height());
    assert!(
        (clip_8.height() - clip_2.height()).abs() <= 1.0,
        "upstream egui::ComboBox #5225 trapped height at opening {}pt (after grow: {}pt)",
        clip_2.height(),
        clip_8.height(),
    );
    assert!(item7_clipped, "8th item (Option 7) is clipped outside trapped ComboBox popup");
    assert!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count() >= 1);

    // Attempting to click Option 7 at its reported rect does not select it because the popup is clipped at 64pt.
    if let Some(opt7) = harness.query_by_label("Option 7") {
        opt7.click();
        harness.run();
    }
    assert_eq!(harness.state().selected, 0, "clicking clipped Option 7 must fail while ComboBox is trapped at 2-item height");
}

#[test]
fn widgets_dropdown_grows_and_shrinks_dynamically_while_open() {
    let fonts = build_driver_font_definitions();
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 480.0)).allow_missing_glyphs().build_ui_state(
        |ui, state: &mut ComboProbeState| {
            Tokens::default().apply_to_ctx(ui.ctx());
            state.popup_clip_rect = None;
            state.popup_min_rect = None;
            state.item_rects.clear();
            state.item_clipped.clear();

            egui::CentralPanel::default().show(ui, |ui| {
                dropdown("growable_probe", format!("Option {}", state.selected)).width(180.0).show_ui(ui, |ui| {
                    state.popup_clip_rect = Some(ui.clip_rect());
                    for idx in 0..state.item_count {
                        let resp = ui.selectable_value(&mut state.selected, idx, format!("Option {idx}"));
                        state.item_rects.push(resp.rect);
                        state.item_clipped.push(!ui.clip_rect().contains_rect(resp.rect));
                    }
                    state.popup_min_rect = Some(ui.min_rect());
                });
            });
        },
        ComboProbeState::with_items(2),
    );
    harness.ctx.set_fonts(fonts);
    harness.run();

    // Open with 2 items.
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    let min_2 = harness.state().popup_min_rect.expect("popup min rect at 2 items");
    assert!(min_2.height() < 70.0, "expected compact 2-item height, got {}", min_2.height());
    assert!(harness.state().item_clipped.iter().all(|c| !c));

    // Grow dynamically from 2 -> 8 items while open: grows on the very next step with all 8 items unclipped.
    harness.state_mut().item_count = 8;
    harness.step();
    let min_8_step1 = harness.state().popup_min_rect.expect("popup min rect step 1");
    let clip_8_step1 = harness.state().popup_clip_rect.expect("popup clip rect step 1");
    assert!(min_8_step1.height() > 180.0, "expected popup content to grow for 8 items, got {}", min_8_step1.height());
    assert!(clip_8_step1.height() >= min_8_step1.height(), "clip height {} must fit 8 items {}", clip_8_step1.height(), min_8_step1.height());
    assert!(harness.state().item_clipped.iter().all(|c| !c), "all 8 items must be unclipped immediately: {:?}", harness.state().item_clipped);
    harness.run();

    // Grow further to 20 items while open: caps at max_popup_height with non-overlapping ScrollBar.
    harness.state_mut().item_count = 20;
    harness.run();
    assert!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count() >= 1, "20 items must show a vertical scrollbar");
    let ak_bounds: Vec<AccessKitRoleBounds> = harness
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
    let sb_violations = check_role_bounds_scrollbar_non_overlap(&ak_bounds);
    assert!(sb_violations.is_empty(), "dropdown scrollbar overlapped items: {sb_violations:?}");

    // Shrink back 20 -> 2 items while still open: popup shrinks cleanly and disappears scrollbar.
    harness.state_mut().item_count = 2;
    harness.run();
    let min_shrunk = harness.state().popup_min_rect.expect("popup min rect after shrinking back to 2");
    assert!(min_shrunk.height() < 70.0, "expected popup to shrink back below 70pt, got {}", min_shrunk.height());
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count(), 0);

    // Grow again 2 -> 8 and click Option 7 directly to select and close popup.
    harness.state_mut().item_count = 8;
    harness.run();
    harness.get_by_label("Option 7").click();
    harness.run();
    assert_eq!(harness.state().selected, 7, "clicking Option 7 after growth must update selection");
    assert!(harness.state().popup_min_rect.is_none(), "popup must close after selecting an item");
}

#[test]
fn widgets_dropdown_bottom_anchor_growth_and_disabled_state() {
    #[derive(Default)]
    struct EdgeState {
        item_count: usize,
        selected: usize,
        enabled: bool,
        popup_min_rect: Option<egui::Rect>,
        item_clipped: Vec<bool>,
    }

    let fonts = build_driver_font_definitions();
    let mut harness = Harness::builder().with_size(egui::vec2(480.0, 420.0)).allow_missing_glyphs().build_ui_state(
        |ui, state: &mut EdgeState| {
            Tokens::default().apply_to_ctx(ui.ctx());
            state.popup_min_rect = None;
            state.item_clipped.clear();

            egui::CentralPanel::default().show(ui, |ui| {
                ui.add_space(360.0);
                dropdown("bottom_edge_dropdown", format!("Item {}", state.selected)).width(150.0).enabled(state.enabled).show_ui(ui, |ui| {
                    for idx in 0..state.item_count {
                        let resp = ui.selectable_value(&mut state.selected, idx, format!("Item {idx}"));
                        state.item_clipped.push(!ui.clip_rect().contains_rect(resp.rect));
                    }
                    state.popup_min_rect = Some(ui.min_rect());
                });
            });
        },
        EdgeState { item_count: 2, selected: 0, enabled: false, popup_min_rect: None, item_clipped: Vec::new() },
    );
    harness.ctx.set_fonts(fonts);
    harness.run();

    // Disabled dropdown cannot open.
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    assert!(harness.state().popup_min_rect.is_none(), "disabled dropdown must not open");

    // Enable, open near bottom of viewport with 2 items, grow to 8 items, and select Item 7.
    harness.state_mut().enabled = true;
    harness.run();
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    assert!(harness.state().popup_min_rect.is_some());

    harness.state_mut().item_count = 8;
    harness.run();
    assert!(
        harness.state().item_clipped.iter().all(|c| !c),
        "bottom-anchored dropdown must grow without clipping: {:?}",
        harness.state().item_clipped
    );
    harness.get_by_label("Item 7").click();
    harness.run();
    assert_eq!(harness.state().selected, 7);
}

fn fill_col_values(col: &mut curvebook_doc::DataSetColumn, title: &str, values: &[f64]) {
    col.title = title.to_owned();
    for (r, &v) in values.iter().enumerate() {
        if let Some(cell) = col.cell_mut(r, 0) {
            *cell = Cell::number(v);
        }
    }
}

fn make_column_sheet(id: &str, title: &str, seed_offset: f64) -> Result<DataSheet, Box<dyn std::error::Error>> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    fill_col_values(&mut table.data_sets[0], "Control", &[10.1 + seed_offset, 10.4, 9.9, 10.2]);
    fill_col_values(&mut table.data_sets[1], "Low Dose", &[12.8 + seed_offset, 13.1, 12.6, 13.0]);
    fill_col_values(&mut table.data_sets[2], "High Dose", &[16.4 + seed_offset, 16.9, 16.2, 16.7]);
    Ok(DataSheet::new(id, title, TableArchetype::Column(table))?)
}

fn find_accesskit_center_by_label<S>(driver: &EguiHumanPathDriver<'_, S>, target: &str) -> Result<egui::Pos2, String> {
    let ak = driver.accesskit_tree().ok_or("missing AccessKit update")?;
    let mut matches = Vec::new();
    for (id, node) in &ak.nodes {
        if node.is_hidden() {
            continue;
        }
        if accesskit_accessible_name(node) == Some(target)
            && let Some(bounds) = node.bounds()
        {
            #[allow(clippy::cast_possible_truncation)]
            let rect = egui::Rect::from_min_max(egui::pos2(bounds.x0 as f32, bounds.y0 as f32), egui::pos2(bounds.x1 as f32, bounds.y1 as f32));
            matches.push((*id, rect.center()));
        }
    }
    if matches.is_empty() {
        return Err(format!("AccessKit node with label `{target}` not found"));
    }
    // Prefer a popup option node that isn't an already-registered background modal/workspace widget.
    if let Some((_, pos)) = matches.iter().find(|(id, _)| driver.frame_meta().find_by_accesskit_id(*id).is_none()) {
        return Ok(*pos);
    }
    Ok(matches[0].1)
}

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl ShellDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

#[test]
fn human_path_edit_multiplicity_method_dropdown() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    let col = make_column_sheet("col-1", "Dose Cohort", 0.0)?;
    let doc = ProjectDocument { data_sheets: vec![col], active_sheet_id: Some(SheetId::new("col-1")), ..Default::default() };
    *app.session_mut() = Session::from_document(doc);
    app.set_gui_only(true);
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" }));
    app.sync_views();

    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = EguiHumanPathDriver::new(ShellDriverState::new(app), metrics, |ui, meta, st| {
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "intent commit failed: {res:?}");
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    // Open Parameters: One-Way ANOVA via Cmd+T.
    driver.press_key("t", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    let entry =
        driver.frame_meta().find_by_widget_id("edit.multiplicity_method").ok_or("missing edit.multiplicity_method in FrameInspectMeta")?.clone();
    assert_eq!(entry.role, "combobox");
    assert_eq!(entry.z_layer, ZLayer::ModalDialog);
    assert_eq!(entry.label, "Tukey");
    assert!(driver.verify_pixel_to_hit_box_parity().is_clean());

    // Open multiplicity dropdown on Tab 1 and verify parity while popup is open.
    driver.click_widget_id("edit.multiplicity_method")?;
    let parity_open = driver.verify_pixel_to_hit_box_parity();
    assert!(parity_open.is_clean(), "open multiplicity dropdown parity failed: {:?}", parity_open.violations);

    // Select "Bonferroni" via human pointer click on its AccessKit bounds.
    let bonf_pos = find_accesskit_center_by_label(&driver, "Bonferroni")?;
    driver.click_at(bonf_pos)?;
    let updated_entry = driver.frame_meta().find_by_widget_id("edit.multiplicity_method").ok_or("missing updated multiplicity")?;
    assert_eq!(updated_entry.label, "Bonferroni");
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Bonferroni"));

    // Switch to Tab 3 (Multiple Comparisons), open the tab-3 dropdown instance, and pick "Holm-Sidak".
    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    driver.click_widget_id("edit.multiplicity_method")?;
    let hs_pos = find_accesskit_center_by_label(&driver, "Holm-Sidak")?;
    driver.click_at(hs_pos)?;
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.multiplicity_method").map(|e| e.label.as_str()), Some("Holm-Sidak"));

    // Switch to nonparametric Kruskal-Wallis and verify disabled parametric option ("Tukey HSD") does not fire while "Dunn" succeeds.
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.assume_gaussian")?;
    driver.click_widget_id("edit.multiplicity_method")?;
    let tukey_pos = find_accesskit_center_by_label(&driver, "Tukey HSD")?;
    driver.click_at(tukey_pos)?;
    assert_ne!(
        driver.frame_meta().find_by_widget_id("edit.multiplicity_method").map(|e| e.label.as_str()),
        Some("Tukey"),
        "clicking disabled Tukey HSD under nonparametric mode must not select Tukey",
    );
    driver.click_widget_id("edit.multiplicity_method")?;
    let dunn_pos = find_accesskit_center_by_label(&driver, "Dunn")?;
    driver.click_at(dunn_pos)?;

    // Commit via modal.ok and verify the ResultSheet reflects Kruskal-Wallis + Dunn.
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs = driver.state().app.document().result_sheet(&SheetId::new("res-anova")).ok_or("missing ResultSheet")?;
    let kind_dbg = format!("{:?}", rs.analysis.kind);
    assert!(kind_dbg.contains("KruskalWallis"), "expected KruskalWallis, got {kind_dbg}");
    assert!(matches!(rs.analysis.kind, curvebook_doc::AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::DunnRankSum));

    Ok(())
}

struct SplitStageState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    report: CenterStageFrameReport,
}

impl SplitStageState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), report: CenterStageFrameReport::default() }
    }
}

#[test]
fn human_path_split_selectors_grow_while_open() -> Result<(), Box<dyn std::error::Error>> {
    let col1 = make_column_sheet("col-1", "Cohort 1", 0.0)?;
    let mut gs1 = GraphSheet::new_uninitialized_for_data("g-1", &col1)?;
    gs1.title = String::from("Dose Graph");
    gs1.initialized = true;

    let doc =
        ProjectDocument { data_sheets: vec![col1], graph_sheets: vec![gs1], active_sheet_id: Some(SheetId::new("col-1")), ..Default::default() };

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.sync_views();

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        SplitStageState::new(app),
        ViewportMetrics::new(1280.0, 720.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let decision = HybridCoexistenceArbiter::evaluate(&st.app, false);
            let layout = st.views.last_shell_layout.unwrap_or_else(|| {
                LayoutMetrics::default().solve_shell_layout(ShellLayoutInput {
                    viewport_w_pt: ui.available_width(),
                    viewport_h_pt: ui.available_height(),
                    empty_workspace: false,
                    navigator_open: true,
                    inspector_open: true,
                    family_open: false,
                    leading_user_collapsed: false,
                    trailing_user_collapsed: false,
                    leading_width_override_pt: None,
                    trailing_width_override_pt: None,
                    host_draws_menu_bar: false,
                    is_rtl: curvebook_ui_egui::i18n::is_rtl_locale(&st.app.locale),
                })
            });
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let ctx_in = CenterStageContext {
                tokens: &tokens,
                decision: &decision,
                layout,
                focused_pane: st.app.nav.active_pane,
                enabled: matches!(st.app.canvas_dialogs.active_dialog, ActiveDialog::None) && st.views.progress_modal.is_none(),
            };
            st.report = render_center_stage(ui, &st.app, &mut st.views, ctx_in, meta, &mut st.intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "intent commit failed: {res:?}");
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    let prim_sel = driver.frame_meta().find_by_widget_id("split.primary_selector").ok_or("missing split.primary_selector")?.clone();
    let sec_sel = driver.frame_meta().find_by_widget_id("split.secondary_selector").ok_or("missing split.secondary_selector")?.clone();
    assert_eq!(prim_sel.role, "combobox");
    assert_eq!(sec_sel.role, "combobox");
    assert_eq!(prim_sel.z_layer, ZLayer::Workspace);
    assert_eq!(sec_sel.z_layer, ZLayer::Workspace);
    assert_eq!(prim_sel.label, "Cohort 1");
    assert_eq!(sec_sel.label, "Dose Graph");
    assert!(driver.verify_pixel_to_hit_box_parity().is_clean());

    // Open split.primary_selector while only 2 sheets exist.
    driver.click_widget_id("split.primary_selector")?;
    let opt1_pos = find_accesskit_center_by_label(&driver, "[Data] Cohort 1")?;
    let opt_g_pos = find_accesskit_center_by_label(&driver, "[Graph] Dose Graph")?;
    assert!(opt_g_pos.y > opt1_pos.y);

    // Dynamically add 6 more sheets (total 8 sheets) WHILE the split.primary_selector popup stays open!
    {
        let mut next_doc = driver.state().app.document().clone();
        for idx in 2..=7 {
            next_doc.data_sheets.push(make_column_sheet(&format!("col-{idx}"), &format!("Cohort {idx}"), idx as f64)?);
        }
        *driver.state_mut().app.session_mut() = Session::from_document(next_doc);
        driver.state_mut().app.sync_views();
    }
    driver.run_settled();

    // Parity must stay clean and the 8th option ("[Data] Cohort 7") must be visible & selectable via human click!
    let parity_grown = driver.verify_pixel_to_hit_box_parity();
    assert!(parity_grown.is_clean(), "grown split.primary_selector parity failed: {:?}", parity_grown.violations);
    let cohort7_pos = find_accesskit_center_by_label(&driver, "[Data] Cohort 7")?;
    driver.click_at(cohort7_pos)?;

    assert_eq!(
        driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str),
        Some("col-7"),
        "selecting dynamically added 8th sheet in split.primary_selector must update primary_sheet_id"
    );
    assert_eq!(driver.frame_meta().find_by_widget_id("split.primary_selector").map(|e| e.label.as_str()), Some("Cohort 7"));

    // Now exercise split.secondary_selector with a long sheet title in a narrow 640x480 viewport to verify single-line ellipsis + parity.
    let col_long = make_column_sheet("col-long", "Very Long Experimental Dose Cohort Title", 9.0)?;
    {
        let mut next_doc = driver.state().app.document().clone();
        next_doc.data_sheets.push(col_long);
        *driver.state_mut().app.session_mut() = Session::from_document(next_doc);
        driver.state_mut().app.sync_views();
    }
    driver.run_settled();
    driver.click_widget_id("split.secondary_selector")?;
    let parity_sec = driver.verify_pixel_to_hit_box_parity();
    assert!(parity_sec.is_clean(), "open split.secondary_selector parity failed: {:?}", parity_sec.violations);
    let long_pos = find_accesskit_center_by_label(&driver, "[Data] Very Long Experimental Dose Cohort Title")?;
    driver.click_at(long_pos)?;
    assert_eq!(
        driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str),
        Some("col-long"),
        "split.secondary_selector must switch secondary_sheet_id to col-long"
    );
    driver.set_viewport(ViewportMetrics::new(640.0, 480.0, 1.0));
    let parity_narrow = driver.verify_pixel_to_hit_box_parity();
    assert!(parity_narrow.is_clean(), "narrow 640x480 split selectors parity failed: {:?}", parity_narrow.violations);
    driver.verify_ellipsis_tooltip_and_accesskit("split.secondary_selector", "Very Long Experimental Dose Cohort Title")?;

    Ok(())
}
