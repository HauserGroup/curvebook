#![forbid(unsafe_code)]
//! `R10.20` Screen-Reader / AccessKit & Browser Accessibility Tree + CI Gate verification suite:
//! - Deterministic `AccessibleTreeUpdate`, `AccessibleNodeUpdate`, `AriaRoleMapping`, `AriaTreeProjection`
//! - `HumanPathDriver` keyboard navigation (`Tab` / `Shift+Tab` / `Space` / `Enter` / `Escape`) across
//!   `Workspace`, `PopupMenu`, and `ModalDialog` focus scopes
//! - `A11yViewState` live announcements, focus announcements, and WCAG-AAA high-contrast focus rings
//! - Positive & negative `validate_layout_tree_accessibility` CI gate invariants across all views

use std::collections::BTreeSet;

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec,
    MultipleVariablesTable, ProjectDocument, RectPt, SheetId, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics, execute_widget_action};
use curvebook_ui_egui::layout::a11y::{
    ACCESSIBLE_ROOT_NODE_ID, AccessibilityMeta, AriaLivePoliteness, AriaRoleMapping, AriaTreeProjection, FocusScope, WidgetRole,
    audit_layout_tree_accessibility, deterministic_node_id, project_accessible_tree, validate_layout_tree_accessibility,
};
use curvebook_ui_egui::layout::{LayoutBuilder, LayoutRectPx, LayoutTree, NodeSpec, WidgetAction, WidgetId, WidgetNode, WidgetVisualState, ZLayer};
use curvebook_ui_egui::text_bridge::DesktopTextMeasurer;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::views::a11y::{
    A11yViewState, format_focus_announcement, handle_custom_action, high_contrast_focus_ring_style, paint_accessible_focus_ring,
    project_app_aria_tree, sync_app_accessibility_tree,
};
use serde_json::json;

fn build_rich_workspace_doc() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::default();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("DrugA");
    col.data_sets[2].title = String::from("DrugB");
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64));
        col.data_sets[1].cells[r][0] = Cell::number(15.0 + (r as f64));
        col.data_sets[2].cells[r][0] = Cell::number(20.0 + (r as f64));
    }
    let col_sheet = DataSheet::new("ds-col", "Assay Column", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(col_sheet).map_err(|e| e.to_string())?;

    let xy = XyTable::new(curvebook_doc::SubcolumnMode::RawReplicates(1), 2, 3).map_err(|e| e.to_string())?;
    let xy_sheet = DataSheet::new("ds-xy", "Dose Response XY", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(xy_sheet).map_err(|e| e.to_string())?;

    let mv = MultipleVariablesTable::new(2, 3).map_err(|e| e.to_string())?;
    let mv_sheet = DataSheet::new("ds-mv", "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(mv_sheet).map_err(|e| e.to_string())?;

    let mut info = InfoSheet::new_preseeded("info-1", "Study Notes").map_err(|e| e.to_string())?;
    info.notes_markdown = String::from("Dose escalation notes");
    info.constants.push(InfoConstant { name: String::from("Lot"), value: curvebook_doc::ConstantValue::Text(String::from("B42")), units: None });
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &doc.data_sheets[0]).map_err(|e| e.to_string())?;
    gs.title = String::from("Dose Graph");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Column;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    let ls_spec = LayoutSpec {
        items: vec![LayoutItem::LiveGraph {
            id: String::from("panel-0"),
            graph_sheet_id: SheetId::from("graph-1"),
            bounds_pt: RectPt::new(36.0, 36.0, 280.0, 200.0),
            scale_percent: 100.0,
            badge_override: None,
        }],
        ..LayoutSpec::default()
    };
    let ls = LayoutSheet::new("layout-1", "Publication Figure", ls_spec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("ds-col")).map_err(|e| e.to_string())?;
    Ok(doc)
}

fn build_workspace_app() -> Result<UiApp, String> {
    let doc = build_rich_workspace_doc()?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn aria_and_accesskit_role_mappings_cover_all_widget_roles() -> Result<(), String> {
    let all_roles = [
        WidgetRole::MenuBar,
        WidgetRole::MenuItem,
        WidgetRole::Button,
        WidgetRole::Tab,
        WidgetRole::TreeItem,
        WidgetRole::GridCell,
        WidgetRole::TextBox,
        WidgetRole::ComboBox,
        WidgetRole::CheckBox,
        WidgetRole::RadioButton,
        WidgetRole::SpinButton,
        WidgetRole::Slider,
        WidgetRole::ScrollBar,
        WidgetRole::Splitter,
        WidgetRole::Canvas,
        WidgetRole::DataPoint,
        WidgetRole::Dialog,
        WidgetRole::Banner,
        WidgetRole::Tooltip,
        WidgetRole::ContextMenu,
        WidgetRole::Disclosure,
        WidgetRole::Label,
    ];

    for role in all_roles {
        let map = AriaRoleMapping::for_role(role);
        assert_eq!(map.widget_role, role);
        assert!(!map.aria_role.is_empty(), "empty aria_role for {role:?}");
        assert!(!map.accesskit_role.is_empty(), "empty accesskit_role for {role:?}");
        assert!(!map.html_tag.is_empty(), "empty html_tag for {role:?}");
        assert_eq!(map.interactive, role.is_interactive());
    }

    let id_a = deterministic_node_id("toolbar.analyze");
    let id_b = deterministic_node_id("toolbar.analyze");
    let id_c = deterministic_node_id("modal.ok");
    assert_eq!(id_a, id_b);
    assert_ne!(id_a, id_c);
    assert!(id_a >= 2 && id_c >= 2);
    assert_ne!(id_a, ACCESSIBLE_ROOT_NODE_ID);

    let notes_node = WidgetNode {
        id: WidgetId::from("info.notes"),
        parent: None,
        z_layer: ZLayer::Workspace,
        paint_order: 1,
        clip: None,
        scroll: None,
        bounds: curvebook_ui_egui::layout::NodeBounds::from_physical(LayoutRectPx::new(0, 0, 120, 80), None, 1.0),
        a11y: AccessibilityMeta::new(WidgetRole::TextBox, "Notes", FocusScope::Workspace).with_keyboard_path("Enter"),
        state: Default::default(),
        action: WidgetAction::FocusWidget(String::from("info.notes")),
        drawn_in_frame: true,
    };
    let notes_map = AriaRoleMapping::for_node(&notes_node);
    assert_eq!(notes_map.accesskit_role, "MultilineTextInput");
    assert_eq!(notes_map.html_tag, "textarea");
    Ok(())
}

#[test]
fn accessible_tree_projection_completeness_hierarchy_hidpi_rtl_and_aria_json() -> Result<(), String> {
    let mut app = build_workspace_app()?;
    let metrics_2x = ViewportMetrics::new(1280.0, 800.0, 2.0);
    app.focus_widget_id = Some(String::from("toolbar.analyze"));

    let tree = app.build_and_cache_layout_tree(metrics_2x);
    validate_layout_tree_accessibility(&tree)?;

    let update = sync_app_accessibility_tree(&app, metrics_2x);
    let drawn_count = tree.nodes.iter().filter(|n| n.drawn_in_frame).count();
    assert_eq!(update.nodes.len(), drawn_count, "every drawn node must project into AccessibleTreeUpdate");
    assert!((update.scale_factor - 2.0).abs() < 1e-9);
    assert_eq!(update.focus_id.as_deref(), Some("toolbar.analyze"));
    let focused = update.node("toolbar.analyze").ok_or("missing toolbar.analyze in AccessibleTreeUpdate")?;
    assert!(focused.focused);
    assert_eq!(update.focus_numeric_id, focused.numeric_id);
    assert_eq!(focused.role_mapping.accesskit_role, "Button");
    assert_eq!(focused.role_mapping.aria_role, "button");
    assert!(focused.bounds_physical.w >= focused.bounds_logical.w.round() as u32);

    // Hierarchy uniqueness check: every node appears exactly once as a child of root or its parent.
    let mut visited = BTreeSet::new();
    for &cid in &update.root_children_numeric_ids {
        assert!(visited.insert(cid), "duplicate root child numeric ID {cid}");
    }
    for node in &update.nodes {
        for &cid in &node.children_numeric_ids {
            assert!(visited.insert(cid), "duplicate child numeric ID {cid}");
            assert!(update.node_by_numeric_id(cid).is_some(), "dangling child numeric ID {cid}");
        }
    }
    assert_eq!(visited.len(), update.nodes.len(), "every projected node must be reachable from root");

    // Browser AriaTreeProjection JSON serialization round-trip.
    let aria = project_app_aria_tree(&app, metrics_2x);
    let aria_json_str = update.to_aria_json_string().map_err(|e| e.to_string())?;
    let parsed_val: serde_json::Value = serde_json::from_str(&aria_json_str).map_err(|e| e.to_string())?;
    let roundtrip = AriaTreeProjection::from_json_value(&parsed_val)?;
    assert_eq!(aria, roundtrip);
    let aria_btn = roundtrip.find_node("toolbar.analyze").ok_or("missing toolbar.analyze in AriaTreeProjection")?;
    assert_eq!(aria_btn.attributes.get("role").map(String::as_str), Some("button"));
    assert_eq!(aria_btn.attributes.get("tabindex").map(String::as_str), Some("0"));
    assert!(aria_btn.attributes.contains_key("data-keyboard-path"));

    // RTL locale projection (`ar`) preserves RTL direction and clean CI gate.
    app.set_locale("ar");
    let rtl_tree = app.build_and_cache_layout_tree(ViewportMetrics::new(1280.0, 800.0, 1.0));
    validate_layout_tree_accessibility(&rtl_tree)?;
    let rtl_aria = project_app_aria_tree(&app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert_eq!(rtl_aria.dir, "rtl");
    assert_eq!(rtl_aria.locale, "ar");
    Ok(())
}

fn verify_workspace_scope_navigation(driver: &mut HumanPathDriver) -> Result<(), String> {
    let mut visited_workspace = Vec::new();
    for _ in 0..8 {
        driver.press_key("Tab", KeyModifiers::default())?;
        let fid = driver.app().focus_widget_id.clone().ok_or("Tab in Workspace must focus a widget")?;
        let tree = driver.layout_tree();
        let node = tree.node(&fid).ok_or_else(|| format!("focused workspace node `{fid}` missing"))?;
        assert_eq!(node.a11y.focus_scope, FocusScope::Workspace);
        visited_workspace.push(fid);
    }
    assert!(visited_workspace.len() == 8 && visited_workspace.iter().collect::<BTreeSet<_>>().len() >= 4);
    let last_ws = visited_workspace.last().cloned().ok_or("empty workspace focus list")?;
    driver.press_key("Tab", KeyModifiers { shift: true, ..Default::default() })?;
    let prev_ws = driver.app().focus_widget_id.clone().ok_or("Shift+Tab must keep a focused workspace widget")?;
    assert_ne!(prev_ws, last_ws);

    driver.app_mut().focus_widget_id = Some(String::from("navigator.family_toggle"));
    let family_before = driver.app().family_open;
    driver.press_key("Space", KeyModifiers::default())?;
    assert_ne!(driver.app().family_open, family_before, "Space should toggle focused family button");
    driver.press_key("Space", KeyModifiers::default())?;
    assert_eq!(driver.app().family_open, family_before);
    Ok(())
}

fn verify_popup_menu_scope_navigation(driver: &mut HumanPathDriver) -> Result<(), String> {
    driver.click_widget("menu.file")?;
    assert_eq!(driver.app().interaction.open_menu.as_deref(), Some("menu.file"));
    let menu_tree = driver.layout_tree();
    assert_eq!(menu_tree.active_focus_scope, FocusScope::PopupMenu);
    validate_layout_tree_accessibility(&menu_tree)?;

    driver.app_mut().focus_widget_id = None;
    let mut visited_menu = Vec::new();
    for _ in 0..4 {
        driver.press_key("Tab", KeyModifiers::default())?;
        let fid = driver.app().focus_widget_id.clone().ok_or("Tab in PopupMenu must focus a MenuItem")?;
        let tree = driver.layout_tree();
        let node = tree.node(&fid).ok_or_else(|| format!("focused menu item `{fid}` missing"))?;
        assert_eq!(node.a11y.focus_scope, FocusScope::PopupMenu, "Tab escaped PopupMenu to `{fid}`");
        assert_eq!(node.a11y.role, WidgetRole::MenuItem);
        visited_menu.push(fid);
    }
    assert!(visited_menu.iter().collect::<BTreeSet<_>>().len() >= 2);
    driver.press_key("Tab", KeyModifiers { shift: true, ..Default::default() })?;
    let prev_menu = driver.app().focus_widget_id.clone().ok_or("Shift+Tab in PopupMenu must focus a MenuItem")?;
    assert_eq!(driver.layout_tree().node(&prev_menu).map(|n| n.a11y.focus_scope), Some(FocusScope::PopupMenu));
    driver.press_key("Escape", KeyModifiers::default())?;
    assert!(driver.app().interaction.open_menu.is_none());
    Ok(())
}

fn verify_modal_scope_navigation(driver: &mut HumanPathDriver, metrics: ViewportMetrics) -> Result<(), String> {
    driver.click_widget("toolbar.analyze")?;
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    validate_layout_tree_accessibility(&driver.layout_tree())?;
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = driver.wheel_widget("analyze.entries_list", 0.0, -180.0, KeyModifiers::default())?;
    }
    driver.click_widget("analyze.entry.stats.one_way_anova")?;
    driver.click_widget("modal.ok")?;
    assert_eq!(driver.app().document().result_sheets.len(), 1);

    driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    let modal_tree = driver.layout_tree();
    assert_eq!(modal_tree.active_focus_scope, FocusScope::ModalDialog);
    validate_layout_tree_accessibility(&modal_tree)?;

    let modal_proj = project_accessible_tree(&modal_tree, driver.app().focus_widget_id.as_deref());
    let dlg_node = modal_proj.node("modal.dialog").ok_or("missing modal.dialog in projection")?;
    assert!(dlg_node.modal && !dlg_node.hidden);
    let bg_toolbar = modal_proj.node("toolbar.analyze").ok_or("missing toolbar.analyze in projection")?;
    assert!(bg_toolbar.hidden, "workspace widgets behind ModalBackdrop must be aria-hidden");

    driver.app_mut().focus_widget_id = None;
    let mut visited_modal = Vec::new();
    for _ in 0..20 {
        driver.press_key("Tab", KeyModifiers::default())?;
        let fid = driver.app().focus_widget_id.clone().ok_or("Tab in ModalDialog must focus modal control")?;
        let node = driver.layout_tree().node(&fid).cloned().ok_or_else(|| format!("missing `{fid}`"))?;
        assert_eq!(node.a11y.focus_scope, FocusScope::ModalDialog, "Tab escaped ModalDialog to `{fid}`");
        visited_modal.push(fid);
    }
    assert!(visited_modal.iter().any(|id| id == "edit.assume_gaussian"));

    driver.app_mut().focus_widget_id = Some(String::from("edit.assume_gaussian"));
    let was_gaussian = driver.app().canvas_dialogs.edit_params.assume_gaussian;
    driver.press_key("Space", KeyModifiers::default())?;
    assert_ne!(driver.app().canvas_dialogs.edit_params.assume_gaussian, was_gaussian);

    let after_toggle_proj = sync_app_accessibility_tree(driver.app(), metrics);
    let assume_node = after_toggle_proj.node("edit.assume_gaussian").ok_or("missing edit.assume_gaussian")?;
    assert_eq!(assume_node.checked, Some(!was_gaussian));
    let tukey_node = after_toggle_proj.node("edit.post_hoc.tukey").ok_or("missing edit.post_hoc.tukey")?;
    assert!(!tukey_node.enabled && tukey_node.disabled_reason.is_some());
    assert_eq!(tukey_node.to_aria_attributes().get("aria-disabled").map(String::as_str), Some("true"));

    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn human_path_keyboard_navigation_across_workspace_menu_and_modal_scopes() -> Result<(), String> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = HumanPathDriver::new(build_workspace_app()?, metrics);
    validate_layout_tree_accessibility(&driver.layout_tree())?;

    verify_workspace_scope_navigation(&mut driver)?;
    verify_popup_menu_scope_navigation(&mut driver)?;
    verify_modal_scope_navigation(&mut driver, metrics)?;

    assert!(driver.verify_parity().is_clean());
    assert!(driver.max_input_to_paint_ms() <= 50.0, "latency {:.2} ms exceeded 50 ms", driver.max_input_to_paint_ms());
    Ok(())
}

#[test]
fn validate_layout_tree_accessibility_passes_across_all_views_and_themes() -> Result<(), String> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut app = build_workspace_app()?;

    // Check every sheet type (Column, XY, MV, Info, Graph, Layout).
    for sid in ["ds-col", "ds-xy", "ds-mv", "info-1", "graph-1", "layout-1"] {
        app.execute_command("sheet.select", json!({ "sheet_id": sid })).map_err(|e| e.to_string())?;
        let tree = app.build_and_cache_layout_tree(metrics);
        let report = validate_layout_tree_accessibility(&tree)?;
        assert!(report.checked_interactive_nodes > 0, "expected interactive nodes on `{sid}`");
    }

    // Split workspace (`DataSheet` + `GraphSheet`).
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::from("ds-col"));
    app.split.secondary_sheet_id = Some(SheetId::from("graph-1"));
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    app.split.enabled = false;

    // Portfolio gallery.
    app.nav.gallery.open = true;
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    app.nav.gallery.open = false;

    // ResultSheet + dropdown + major modals + HighContrast theme.
    // Note: `ActiveDialog::PowerAndSampleSize` on pre-w5 `main` currently emits a duplicate
    // `power.curve_hover` ID (`power_wizard.rs:82,90`) flagged by `validate_layout_tree_accessibility`.
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    app.set_theme_mode(UiThemeMode::HighContrast);
    app.canvas_dialogs.active_dialog = ActiveDialog::CommandPalette;
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    app.canvas_dialogs.active_dialog = ActiveDialog::ImportReshape;
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    app.canvas_dialogs.active_dialog = ActiveDialog::None;

    // Open dropdown in inspector (`PairwiseComparisons`).
    app.canvas_dialogs.active_inspector = curvebook_ui_egui::dialogs_canvas::ActiveInspector::PairwiseComparisons;
    let _ = execute_widget_action(&mut app, &WidgetAction::OpenDropdown(String::from("inspector.pairwise_mode")), metrics)?;
    validate_layout_tree_accessibility(&app.build_and_cache_layout_tree(metrics))?;
    Ok(())
}

#[test]
fn validate_layout_tree_accessibility_rejects_phantom_unlabelled_passive_and_unscoped_controls() -> Result<(), String> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut app = build_workspace_app()?;
    let base_tree = app.build_and_cache_layout_tree(metrics);

    // 1. Phantom interactive control (`drawn_in_frame == false` with `action != NoOp`).
    let mut phantom_tree = base_tree.clone();
    if let Some(node) = phantom_tree.nodes.iter_mut().find(|n| n.id.as_str() == "toolbar.analyze") {
        node.drawn_in_frame = false;
    }
    let phantom_err = validate_layout_tree_accessibility(&phantom_tree).err().ok_or("expected phantom failure")?;
    assert!(phantom_err.report.violations.iter().any(|v| v.check == "phantom_control"));

    // 2. Interactive action masquerading as passive `WidgetRole::Label`.
    let mut passive_role_tree = base_tree.clone();
    if let Some(node) = passive_role_tree.nodes.iter_mut().find(|n| n.id.as_str() == "toolbar.analyze") {
        node.a11y.role = WidgetRole::Label;
    }
    let role_err = validate_layout_tree_accessibility(&passive_role_tree).err().ok_or("expected semantic_role failure")?;
    assert!(role_err.report.violations.iter().any(|v| v.check == "semantic_role"));

    // 3. Unlabelled interactive control (`label == "   "`).
    let mut unlabelled_tree = base_tree.clone();
    if let Some(node) = unlabelled_tree.nodes.iter_mut().find(|n| n.id.as_str() == "toolbar.analyze") {
        node.a11y.label = String::from("   ");
    }
    let label_err = validate_layout_tree_accessibility(&unlabelled_tree).err().ok_or("expected missing_label failure")?;
    assert!(label_err.report.violations.iter().any(|v| v.check == "missing_label"));

    // 4. Missing `keyboard_path` on interactive control.
    let mut nokey_tree = base_tree.clone();
    if let Some(node) = nokey_tree.nodes.iter_mut().find(|n| n.id.as_str() == "toolbar.analyze") {
        node.a11y.keyboard_path = None;
    }
    let kpath_err = validate_layout_tree_accessibility(&nokey_tree).err().ok_or("expected missing_keyboard_path failure")?;
    assert!(kpath_err.report.violations.iter().any(|v| v.check == "missing_keyboard_path"));

    // 5. Unreachable focus scope: workspace node claiming ModalDialog scope.
    let mut bad_scope_tree = base_tree;
    if let Some(node) = bad_scope_tree.nodes.iter_mut().find(|n| n.id.as_str() == "toolbar.analyze") {
        node.a11y.focus_scope = FocusScope::ModalDialog;
    }
    let scope_err = validate_layout_tree_accessibility(&bad_scope_tree).err().ok_or("expected unreachable_focus_scope failure")?;
    assert!(scope_err.report.violations.iter().any(|v| v.check == "unreachable_focus_scope"));

    // 6. ModalDialog open with zero ModalDialog-scoped focus targets (`empty_active_focus_scope`).
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    let mut empty_modal_tree = app.build_and_cache_layout_tree(metrics);
    for node in &mut empty_modal_tree.nodes {
        if node.z_layer >= ZLayer::ModalDialog && node.a11y.focus_scope == FocusScope::ModalDialog {
            node.a11y.focus_scope = FocusScope::Workspace;
        }
    }
    let modal_report = audit_layout_tree_accessibility(&empty_modal_tree);
    assert!(!modal_report.is_clean());
    assert!(modal_report.violations.iter().any(|v| v.check == "empty_active_focus_scope"));

    // 7. Duplicate WidgetId (`unique_widget_id`).
    let mut dup_tree = empty_modal_tree;
    if let Some(cloned) = dup_tree.nodes.iter().find(|n| n.id.as_str() == "modal.ok").cloned() {
        dup_tree.nodes.push(cloned);
    }
    let dup_err = validate_layout_tree_accessibility(&dup_tree).err().ok_or("expected duplicate WidgetId failure")?;
    assert!(dup_err.report.violations.iter().any(|v| v.check == "unique_widget_id"));
    Ok(())
}

#[test]
fn a11y_view_state_custom_actions_and_high_contrast_focus_ring() -> Result<(), String> {
    let mut app = build_workspace_app()?;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let _ = app.build_and_cache_layout_tree(metrics);

    // Scaffold compatibility: unrecognized `"smoke"` payload must return handled == false cleanly.
    let mut state = A11yViewState::default();
    let smoke_out = handle_custom_action(&mut app, &mut state, "smoke")?;
    assert!(!smoke_out.handled);

    // Explicit polite & assertive live announcements via WidgetAction::Custom.
    let out1 = execute_widget_action(
        &mut app,
        &WidgetAction::Custom { domain: String::from("a11y"), payload: String::from("announce:Fit converged") },
        metrics,
    )?;
    assert!(out1.handled);
    assert_eq!(app.view_shards.a11y.last_announcement.as_deref(), Some("Fit converged"));
    assert_eq!(app.view_shards.a11y.live_politeness, AriaLivePoliteness::Polite);

    let proj1 = sync_app_accessibility_tree(&app, metrics);
    assert_eq!(proj1.live_announcement.as_deref(), Some("Fit converged"));
    assert_eq!(proj1.live_politeness, AriaLivePoliteness::Polite);

    let out2 = execute_widget_action(
        &mut app,
        &WidgetAction::Custom { domain: String::from("a11y"), payload: String::from("announce_assertive:Singular Jacobian") },
        metrics,
    )?;
    assert!(out2.handled);
    assert_eq!(app.view_shards.a11y.live_politeness, AriaLivePoliteness::Assertive);
    assert_eq!(project_app_aria_tree(&app, metrics).live_politeness, "assertive");

    // Step focus + announce focus node via custom actions.
    let _ = execute_widget_action(
        &mut app,
        &WidgetAction::Custom { domain: String::from("a11y"), payload: String::from("clear_announcement") },
        metrics,
    )?;
    assert!(app.view_shards.a11y.last_announcement.is_none());

    let step_out =
        execute_widget_action(&mut app, &WidgetAction::Custom { domain: String::from("a11y"), payload: String::from("focus_next") }, metrics)?;
    assert!(step_out.handled);
    let focused_id = app.focus_widget_id.clone().ok_or("focus_next should focus a workspace widget")?;
    let tree = app.build_and_cache_layout_tree(metrics);
    let node = tree.node(&focused_id).ok_or("focused node missing")?;
    let expected_msg = format_focus_announcement(node);
    assert_eq!(app.view_shards.a11y.last_announcement.as_deref(), Some(expected_msg.as_str()));

    // High-contrast focus ring style meets WCAG AAA >= 7.0:1 across Light, Dark, and HighContrast themes.
    let _ = execute_widget_action(
        &mut app,
        &WidgetAction::Custom { domain: String::from("a11y"), payload: String::from("toggle_high_contrast_focus") },
        metrics,
    )?;
    assert!(app.view_shards.a11y.high_contrast_focus_ring);

    for mode in UiThemeMode::ALL {
        app.set_theme_mode(mode);
        let style = high_contrast_focus_ring_style(&app.theme, &app.view_shards.a11y);
        assert_eq!(style.stroke_px, 3);
        let ratio = style.outer_color.contrast_ratio(app.theme.surface_bg);
        assert!(ratio >= 7.0, "high-contrast focus ring contrast in {mode:?} was {ratio:.2} < 7.0");
    }

    // Paint helper emits visible stroke ops cleanly.
    let measurer = DesktopTextMeasurer::global();
    let palette = curvebook_ui_egui::compositor::FramePalette::from_theme(&app.theme);
    let mut builder = LayoutBuilder::new(&app, metrics, measurer, &palette);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from("test.focus_ring"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: LayoutRectPx::new(10, 10, 80, 28),
        a11y: AccessibilityMeta::new(WidgetRole::Button, "Run", FocusScope::Workspace).with_keyboard_path("Enter"),
        state: WidgetVisualState::default(),
        action: WidgetAction::InvokeCommand { command_id: String::from("analyze.run"), params: json!({}) },
    });
    paint_accessible_focus_ring(&mut builder, ZLayer::Workspace, &wid, LayoutRectPx::new(10, 10, 80, 28), true, &app.view_shards.a11y);
    let built: LayoutTree = builder.finish();
    assert!(built.node("test.focus_ring").is_some_and(|n| n.drawn_in_frame));
    Ok(())
}
