#![forbid(unsafe_code)]

//! `R16.19` Verification of upstream `egui::ComboBox` #5225 on pinned `egui 0.37` and
//! replacement with growable [`widgets::dropdown`] (`stat_anova_modal.rs` & `center_stage.rs`).

use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, MultiplicityMethod, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::center_stage::{CenterStageContext, CenterStageFrameReport, render_center_stage};
use curvebook_ui_egui::egui_shell::driver::{
    AccessKitRoleBounds, EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions,
    check_role_bounds_scrollbar_non_overlap,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::HybridCoexistenceArbiter;
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::egui_shell::widgets::dropdown;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use crate::common::{assert_clean_parity, fill_col_values, make_default_driver};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct ComboProbeState {
    item_count: usize,
    selected: usize,
    enabled: bool,
    popup_clip_rect: Option<egui::Rect>,
    popup_min_rect: Option<egui::Rect>,
    item_clipped: Vec<bool>,
}

impl ComboProbeState {
    fn reset_frame(&mut self) {
        self.popup_clip_rect = None;
        self.popup_min_rect = None;
        self.item_clipped.clear();
    }

    fn render_items(&mut self, ui: &mut egui::Ui, prefix: &str) {
        self.popup_clip_rect = Some(ui.clip_rect());
        for idx in 0..self.item_count {
            let resp = ui.selectable_value(&mut self.selected, idx, format!("{prefix} {idx}"));
            self.item_clipped.push(!ui.clip_rect().contains_rect(resp.rect));
        }
        self.popup_min_rect = Some(ui.min_rect());
    }
}

fn build_probe_harness(
    size: egui::Vec2,
    init: ComboProbeState,
    ui_fn: impl FnMut(&mut egui::Ui, &mut ComboProbeState) + 'static,
) -> Harness<'static, ComboProbeState> {
    let mut harness = Harness::builder().with_size(size).allow_missing_glyphs().build_ui_state(ui_fn, init);
    harness.ctx.set_fonts(build_driver_font_definitions());
    harness.run();
    harness
}

#[test]
#[allow(clippy::disallowed_types)] // Deliberate baseline reproduction of upstream egui::ComboBox #5225 on egui 0.37.0.
fn egui_combobox_5225_reproduces_on_pinned_egui_0_37() {
    let mut harness =
        build_probe_harness(egui::vec2(640.0, 480.0), ComboProbeState { item_count: 2, enabled: true, ..Default::default() }, |ui, st| {
            Tokens::default().apply_to_ctx(ui.ctx());
            st.reset_frame();
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ComboBox::from_label("Probe Combo")
                    .selected_text(format!("Option {}", st.selected))
                    .width(180.0)
                    .show_ui(ui, |ui| st.render_items(ui, "Option"));
            });
        });

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    let clip_2 = harness.state().popup_clip_rect.expect("clip 2");
    assert!(harness.state().popup_min_rect.expect("min 2").height() < 70.0 && harness.query_by_label("Option 1").is_some());

    harness.state_mut().item_count = 8;
    for _ in 0..6 {
        harness.step();
    }
    harness.run();

    let (clip_8, min_8) = (harness.state().popup_clip_rect.expect("clip 8"), harness.state().popup_min_rect.expect("min 8"));
    assert!(min_8.height() > 180.0 && (clip_8.height() - clip_2.height()).abs() <= 1.0);
    assert!(harness.state().item_clipped.get(7).copied().unwrap_or(false));
    assert!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count() >= 1);

    if let Some(opt7) = harness.query_by_label("Option 7") {
        opt7.click();
        harness.run();
    }
    assert_eq!(harness.state().selected, 0);
}

#[test]
fn widgets_dropdown_grows_and_shrinks_dynamically_while_open() {
    let mut harness =
        build_probe_harness(egui::vec2(640.0, 480.0), ComboProbeState { item_count: 2, enabled: true, ..Default::default() }, |ui, st| {
            Tokens::default().apply_to_ctx(ui.ctx());
            st.reset_frame();
            egui::CentralPanel::default().show(ui, |ui| {
                dropdown("growable_probe", format!("Option {}", st.selected)).width(180.0).show_ui(ui, |ui| st.render_items(ui, "Option"));
            });
        });

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    assert!(harness.state().popup_min_rect.expect("min 2").height() < 70.0 && harness.state().item_clipped.iter().all(|c| !c));

    harness.state_mut().item_count = 8;
    harness.step();
    let (min_8, clip_8) = (harness.state().popup_min_rect.expect("min 8"), harness.state().popup_clip_rect.expect("clip 8"));
    assert!(min_8.height() > 180.0 && clip_8.height() >= min_8.height() && harness.state().item_clipped.iter().all(|c| !c));
    harness.run();

    harness.state_mut().item_count = 20;
    harness.run();
    assert!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count() >= 1);
    let ak_bounds: Vec<AccessKitRoleBounds> = harness
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
    assert!(check_role_bounds_scrollbar_non_overlap(&ak_bounds).is_empty());

    harness.state_mut().item_count = 2;
    harness.run();
    assert!(harness.state().popup_min_rect.expect("min shrunk").height() < 70.0);
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count(), 0);

    harness.state_mut().item_count = 8;
    harness.run();
    harness.get_by_label("Option 7").click();
    harness.run();
    assert_eq!(harness.state().selected, 7);
    assert!(harness.state().popup_min_rect.is_none());
}

#[test]
fn widgets_dropdown_bottom_anchor_growth_and_disabled_state() {
    let mut harness =
        build_probe_harness(egui::vec2(480.0, 420.0), ComboProbeState { item_count: 2, enabled: false, ..Default::default() }, |ui, st| {
            Tokens::default().apply_to_ctx(ui.ctx());
            st.reset_frame();
            egui::CentralPanel::default().show(ui, |ui| {
                ui.add_space(360.0);
                dropdown("bottom_edge_dropdown", format!("Item {}", st.selected))
                    .width(150.0)
                    .enabled(st.enabled)
                    .show_ui(ui, |ui| st.render_items(ui, "Item"));
            });
        });

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    assert!(harness.state().popup_min_rect.is_none());

    harness.state_mut().enabled = true;
    harness.run();
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    assert!(harness.state().popup_min_rect.is_some());

    harness.state_mut().item_count = 8;
    harness.run();
    assert!(harness.state().item_clipped.iter().all(|c| !c));
    harness.get_by_label("Item 7").click();
    harness.run();
    assert_eq!(harness.state().selected, 7);
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
        if !node.is_hidden()
            && accesskit_accessible_name(node) == Some(target)
            && let Some(b) = node.bounds()
        {
            #[allow(clippy::cast_possible_truncation)]
            matches.push((*id, egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)).center()));
        }
    }
    if matches.is_empty() {
        return Err(format!("AccessKit node `{target}` not found"));
    }
    Ok(matches.iter().find(|(id, _)| driver.frame_meta().find_by_accesskit_id(*id).is_none()).unwrap_or(&matches[0]).1)
}

#[test]
fn human_path_edit_multiplicity_method_dropdown() -> BoxRes {
    let mut app = UiApp::new();
    let doc = ProjectDocument {
        data_sheets: vec![make_column_sheet("col-1", "Dose Cohort", 0.0)?],
        active_sheet_id: Some(SheetId::new("col-1")),
        ..Default::default()
    };
    *app.session_mut() = Session::from_document(doc);
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" }));
    app.sync_views();

    let mut driver = make_default_driver(app);
    driver.press_key("P", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    let entry = driver.frame_meta().find_by_widget_id("edit.multiplicity_method").ok_or("missing multiplicity")?.clone();
    assert_eq!((entry.role.as_str(), entry.z_layer, entry.label.as_str()), ("combobox", ZLayer::ModalDialog, "Tukey"));
    assert_clean_parity(&driver, "initial multiplicity")?;

    driver.click_widget_id("edit.multiplicity_method")?;
    assert_clean_parity(&driver, "open multiplicity")?;
    driver.click_at(find_accesskit_center_by_label(&driver, "Bonferroni")?)?;
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.multiplicity_method").map(|e| e.label.as_str()), Some("Bonferroni"));
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Bonferroni"));

    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    driver.click_widget_id("edit.multiplicity_method")?;
    driver.click_at(find_accesskit_center_by_label(&driver, "Holm-Sidak")?)?;
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.multiplicity_method").map(|e| e.label.as_str()), Some("Holm-Sidak"));

    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.assume_gaussian")?;
    driver.click_widget_id("edit.multiplicity_method")?;
    driver.click_at(find_accesskit_center_by_label(&driver, "Tukey HSD")?)?;
    assert_ne!(driver.frame_meta().find_by_widget_id("edit.multiplicity_method").map(|e| e.label.as_str()), Some("Tukey"));
    driver.click_widget_id("edit.multiplicity_method")?;
    driver.click_at(find_accesskit_center_by_label(&driver, "Dunn")?)?;

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs = driver.state().app.document().result_sheet(&SheetId::new("res-anova")).ok_or("missing ResultSheet")?;
    assert!(format!("{:?}", rs.analysis.kind).contains("KruskalWallis"));
    assert!(matches!(rs.analysis.kind, curvebook_doc::AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::DunnRankSum));
    Ok(())
}

#[test]
fn human_path_split_selectors_grow_while_open() -> BoxRes {
    struct SplitStageState {
        app: UiApp,
        views: EguiShellViewState,
        intents: PendingUiIntents,
        report: CenterStageFrameReport,
    }
    let col1 = make_column_sheet("col-1", "Cohort 1", 0.0)?;
    let mut gs1 = GraphSheet::new_uninitialized_for_data("g-1", &col1)?;
    gs1.title = String::from("Dose Graph");
    gs1.initialized = true;
    let doc =
        ProjectDocument { data_sheets: vec![col1], graph_sheets: vec![gs1], active_sheet_id: Some(SheetId::new("col-1")), ..Default::default() };

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.sync_views();
    let views = app.egui_views.clone();

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        SplitStageState { app, views, intents: PendingUiIntents::new(), report: CenterStageFrameReport::default() },
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
            assert!(st.intents.commit_if_settled(&mut st.app, false).is_ok());
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    for (wid, label) in [("split.primary_selector", "Cohort 1"), ("split.secondary_selector", "Dose Graph")] {
        let e = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?;
        assert_eq!((e.role.as_str(), e.z_layer, e.label.as_str()), ("combobox", ZLayer::Workspace, label));
    }
    assert_clean_parity(&driver, "initial split selectors")?;

    driver.click_widget_id("split.primary_selector")?;
    assert!(find_accesskit_center_by_label(&driver, "[Graph] Dose Graph")?.y > find_accesskit_center_by_label(&driver, "[Data] Cohort 1")?.y);

    {
        let mut next_doc = driver.state().app.document().clone();
        for idx in 2..=7 {
            next_doc.data_sheets.push(make_column_sheet(&format!("col-{idx}"), &format!("Cohort {idx}"), idx as f64)?);
        }
        *driver.state_mut().app.session_mut() = Session::from_document(next_doc);
        driver.state_mut().app.sync_views();
    }
    driver.run_settled();
    assert_clean_parity(&driver, "grown split.primary_selector")?;
    driver.click_at(find_accesskit_center_by_label(&driver, "[Data] Cohort 7")?)?;
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str), Some("col-7"));
    assert_eq!(driver.frame_meta().find_by_widget_id("split.primary_selector").map(|e| e.label.as_str()), Some("Cohort 7"));

    let long_title = "Very Long Experimental Dose Cohort Title With Extra Replicate Metadata And Assay Notes";
    {
        let mut next_doc = driver.state().app.document().clone();
        next_doc.data_sheets.push(make_column_sheet("col-long", long_title, 9.0)?);
        *driver.state_mut().app.session_mut() = Session::from_document(next_doc);
        driver.state_mut().app.sync_views();
    }
    driver.run_settled();
    driver.click_widget_id("split.secondary_selector")?;
    assert_clean_parity(&driver, "open split.secondary_selector")?;
    driver.click_at(find_accesskit_center_by_label(&driver, &format!("[Data] {long_title}"))?)?;
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str), Some("col-long"));

    driver.set_viewport(ViewportMetrics::new(640.0, 640.0, 1.0));
    assert_clean_parity(&driver, "narrow 640x640 split selectors")?;
    driver.verify_ellipsis_tooltip_and_accesskit("split.secondary_selector", long_title)?;
    Ok(())
}
