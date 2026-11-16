#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 5 Shard `w5g1` human-path integration & visual parity suite:
//! `R10.3` (Graph Type, Hook Constant, Export Figure, ALL remaining `ActiveDialog`s,
//! `R1`–`R10` `modal_predicates` tooltips), `R10.6` (`Cmd+K` Command Palette), and
//! `R10.15` (Aspect-ratio zoom selector, pan/scrollbars, hover/selected-only chrome).

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphSheet, InfoSheet, JournalWidthPreset, LayoutItem,
    LayoutSheet, LayoutSpec, LinearRegressionSpec, ProjectDocument, RectPt, ResultScalarEntry, ResultSheet, SheetId, SubcolumnMode, TableArchetype,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::PaletteId;
use curvebook_render::RasterImage;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ExportFigureFormat};
use curvebook_ui_egui::dialogs_r9::RandomizationSchemeKind;
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::layout::canvas_viewport::{should_show_graph_series_chip, should_show_layout_panel_chrome};
use curvebook_ui_egui::{HumanPathDriver, KeyModifiers, UiApp};
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, hover_widget_center,
    new_human_driver, press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused,
};

fn build_rich_workspace() -> UiApp {
    let mut doc = ProjectDocument::new("W5G1 Workspace").expect("valid doc title");

    // Info sheet with InfoConstant entry for HookConstant branch
    let mut info = InfoSheet::new_preseeded("info-1", "Project Constants").expect("valid info sheet");
    info.set_constant("DoseRef", ConstantValue::Number(10.0), Some(String::from("uM"))).expect("set constant");
    doc.info_sheets.push(info);

    // Primary XY table with numeric ColumnTitleValue ("25") + 6 points
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).expect("xy table");
    xy.data_sets[0].title = String::from("25");
    xy.data_sets[1].title = String::from("Vehicle");
    for r in 0..6 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0);
        xy.data_sets[1].cells[r][0] = Cell::number(1.2 * x + 0.5);
    }
    let ds_xy = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy)).expect("ds_xy");

    // Compatible XY candidate for ChangeDataAnalyzed
    let mut xy2 = XyTable::new(SubcolumnMode::Single, 2, 5).expect("xy2");
    xy2.data_sets[0].title = String::from("Cohort B");
    xy2.data_sets[1].title = String::from("Control B");
    for r in 0..5 {
        let x = (r + 1) as f64 * 2.0;
        xy2.x_cells[r][0] = Cell::number(x);
        xy2.data_sets[0].cells[r][0] = Cell::number(1.5 * x + 2.0);
        xy2.data_sets[1].cells[r][0] = Cell::number(x + 1.0);
    }
    let ds_xy2 = DataSheet::new("data-xy-2", "Dose Replication", TableArchetype::Xy(xy2)).expect("ds_xy2");

    // Column table (unequal replicate counts -> incompatible with XY regression & disables R1 paired/matched)
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("col_tbl");
    col_tbl.data_sets[0].title = String::from("Ctrl");
    col_tbl.data_sets[1].title = String::from("Low");
    col_tbl.data_sets[2].title = String::from("High");
    for r in 0..4 {
        col_tbl.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        if r < 3 {
            col_tbl.data_sets[1].cells[r][0] = Cell::number(14.0 + r as f64);
        }
        col_tbl.data_sets[2].cells[r][0] = Cell::number(19.0 + r as f64);
    }
    let ds_col = DataSheet::new("data-col", "Summary Stats", TableArchetype::Column(col_tbl)).expect("ds_col");

    // Primary and donor graphs
    let mut gs1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).expect("gs1");
    gs1.title = String::from("Dose Curve");
    gs1.initialized = true;
    let mut gs2 = GraphSheet::new_uninitialized_for_data("graph-2", &ds_xy2).expect("gs2");
    gs2.title = String::from("Donor Style Graph");
    gs2.initialized = true;
    gs2.spec.palette = PaletteId::PaulTolVibrant;

    // Layout sheet with 2 LiveGraph panels
    let mut ls_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    ls_spec
        .add_item(LayoutItem::LiveGraph {
            id: String::from("item-a"),
            graph_sheet_id: gs1.id.clone(),
            bounds_pt: RectPt::new(16.0, 16.0, 200.0, 140.0),
            scale_percent: 100.0,
            badge_override: None,
        })
        .expect("item a");
    ls_spec
        .add_item(LayoutItem::LiveGraph {
            id: String::from("item-b"),
            graph_sheet_id: gs2.id.clone(),
            bounds_pt: RectPt::new(236.0, 16.0, 200.0, 140.0),
            scale_percent: 100.0,
            badge_override: None,
        })
        .expect("item b");
    let ls = LayoutSheet::new("layout-1", "Publication Layout", ls_spec).expect("layout-1");

    doc.data_sheets.push(ds_xy);
    doc.data_sheets.push(ds_xy2);
    doc.data_sheets.push(ds_col);
    doc.graph_sheets.push(gs1);
    doc.graph_sheets.push(gs2);
    doc.layout_sheets.push(ls);
    doc.active_sheet_id = Some(SheetId::new("graph-1"));

    // Seed one regression ResultSheet for Equation/HookConstant/ChangeDataAnalyzed/EditParams
    let mut reg_spec = AnalysisSpec::new("res-lin", "data-xy", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("reg spec");
    reg_spec.selected_data_sets = vec![0];
    let mut rs = ResultSheet::new(reg_spec).expect("rs");
    rs.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 2.0, units: None });
    doc.result_sheets.push(rs);

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.print.discovered_printers = vec![String::from("LabLaser-PDF")];
    app
}

fn open_menu_command(driver: &mut HumanPathDriver, menu_id: &str, cmd_id: &str) {
    click_widget_center(driver, &WidgetId::from(menu_id)).expect("open menu bar");
    let item = WidgetId::from(format!("menu.{cmd_id}"));
    scroll_until_widget_visible(driver, &WidgetId::from(format!("popup.{menu_id}")), &item, 160.0, 20).expect("scroll menu item into view");
    click_widget_center(driver, &item).expect("click menu item");
}

fn hover_disabled_widget_for_tooltip(driver: &mut HumanPathDriver, id: &str) -> String {
    let node = driver.find_widget(id).unwrap_or_else(|| panic!("widget `{id}` not found"));
    assert!(!node.state.enablement.enabled, "expected widget `{id}` to be disabled");
    let r = node.bounds.visible_rect_px;
    let lx = f64::from(r.x) + f64::from(r.width) / 2.0;
    let ly = f64::from(r.y) + f64::from(r.height) / 2.0;
    driver.move_pointer(lx, ly).expect("hover disabled widget");
    driver.layout_tree().node("overlay.tooltip").unwrap_or_else(|| panic!("expected overlay.tooltip when hovering `{id}`")).a11y.label.clone()
}

fn assert_modal_parity_clean(driver: &mut HumanPathDriver, label: &str) {
    let report = driver.verify_parity();
    assert!(report.is_clean(), "{label} failed parity: {:?}", report.violations);
}

fn count_diff_pixels(a: &RasterImage, b: &RasterImage) -> usize {
    assert_eq!(a.width_px, b.width_px);
    assert_eq!(a.height_px, b.height_px);
    a.rgba.as_chunks::<4>().0.iter().zip(b.rgba.as_chunks::<4>().0.iter()).filter(|(pa, pb)| pa != pb).count()
}

#[test]
fn r10_3_change_graph_type_hook_constant_and_export_figure() {
    let mut driver = new_human_driver(build_rich_workspace(), default_viewport());

    // 1. ChangeGraphType: menu open -> thumbnail cards -> live preview -> Summary/ErrorBars -> OK
    open_menu_command(&mut driver, "menu.graph", "ui.changeGraphType.open");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }));
    assert_modal_parity_clean(&mut driver, "ChangeGraphType");

    let frame = driver.render_frame().expect("render ChangeGraphType preview");
    assert!(frame.rgba.iter().any(|&v| v > 0));

    click_widget_center(&mut driver, &WidgetId::from("graph_type.card.xy.scatter_line")).expect("select xy.scatter_line");
    click_widget_center(&mut driver, &WidgetId::from("graph_type.sum.median")).expect("pick Median summary");
    click_widget_center(&mut driver, &WidgetId::from("graph_type.eb.sem")).expect("pick SEM error bars");
    assert_eq!(driver.app().canvas_dialogs.change_graph_type.selected_option_id, "xy.scatter_line");
    click_widget_center(&mut driver, &WidgetId::from("modal.ok")).expect("confirm ChangeGraphType");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None));

    // 2. ConstantHookPicker (HookConstant): Info/Analysis/Document tree + expand/collapse + live search + select
    {
        let app = driver.app_mut();
        let doc = app.document().clone();
        app.canvas_dialogs.open_hook_picker(&doc, &SheetId::new("res-lin"), Some(String::from("ConstraintA")));
    }
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ConstantHookPicker { .. }));
    assert_modal_parity_clean(&mut driver, "HookConstant initial");
    assert!(driver.find_widget("hook.cat.info").is_some());
    assert!(driver.find_widget("hook.cat.analysis").is_some());
    assert!(driver.find_widget("hook.cat.document").is_some());

    // Collapse Info branch, verify Info item hides, then expand again
    let info_item = driver
        .layout_tree()
        .nodes
        .iter()
        .find(|n| n.id.as_str().starts_with("hook.item.") && n.a11y.label.contains("DoseRef"))
        .map(|n| n.id.0.clone())
        .expect("DoseRef info item visible");
    click_widget_center(&mut driver, &WidgetId::from("hook.cat.info")).expect("collapse info");
    assert!(driver.find_widget(&info_item).is_none(), "collapsing branch hides child");
    click_widget_center(&mut driver, &WidgetId::from("hook.cat.info")).expect("expand info");
    assert!(driver.find_widget(&info_item).is_some(), "expanding branch restores child");

    // Live search filtering
    click_and_type(&mut driver, &WidgetId::from("hook.search_input"), "DoseRef").expect("type live filter");
    assert!(driver.find_widget(&info_item).is_some());
    click_widget_center(&mut driver, &WidgetId::from(info_item.as_str())).expect("select hooked constant");
    assert!(driver.app().canvas_dialogs.hook_picker.selected_option().is_some());
    click_widget_center(&mut driver, &WidgetId::from("modal.ok")).expect("commit hook picker");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None));

    // 3. ExportFigure: format, DPI (with disabled tooltip on SVG), physical size, transparency, colour space
    open_menu_command(&mut driver, "menu.file", "ui.export.figure.open");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }));
    assert_modal_parity_clean(&mut driver, "ExportFigure");

    click_widget_center(&mut driver, &WidgetId::from("export.fmt.svg")).expect("select SVG");
    let tip = hover_disabled_widget_for_tooltip(&mut driver, "export.dpi.dec");
    assert!(tip.contains("resolution-independent"), "unexpected tooltip: {tip}");

    click_widget_center(&mut driver, &WidgetId::from("export.fmt.png")).expect("select PNG");
    click_widget_center(&mut driver, &WidgetId::from("export.dpi.600")).expect("select 600 DPI");
    click_widget_center(&mut driver, &WidgetId::from("export.unit.mm")).expect("switch unit mm");
    click_widget_center(&mut driver, &WidgetId::from("export.width.inc")).expect("step physical width");
    click_widget_center(&mut driver, &WidgetId::from("export.transparent_bg")).expect("toggle transparent bg");
    click_widget_center(&mut driver, &WidgetId::from("export.cs.cmyk")).expect("select CMYK-Proof");
    assert!(matches!(driver.app().canvas_dialogs.export_figure.format, ExportFigureFormat::Png));
    assert_eq!(driver.app().canvas_dialogs.export_figure.dpi, 600);
    assert!(driver.app().view_shards.dialogs_graph_misc.export_transparent_bg);
    assert_eq!(driver.app().view_shards.dialogs_graph_misc.export_color_space, "CMYK-Proof");

    click_widget_center(&mut driver, &WidgetId::from("export.fmt.pdf")).expect("select PDF");
    let pdf_tip = hover_disabled_widget_for_tooltip(&mut driver, "export.transparent_bg");
    assert!(pdf_tip.contains("PDF"), "unexpected PDF tip: {pdf_tip}");

    click_widget_center(&mut driver, &WidgetId::from("modal.cancel")).expect("close ExportFigure");
    assert_zero_shortcut_compliance(&mut driver).expect("zero-shortcut compliance");
    assert_input_to_paint_budget(&driver, 500.0).expect("input-to-paint budget");
}

fn verify_duplicate_and_change_data_dialogs(driver: &mut HumanPathDriver) {
    // 1. DuplicateAnalysisPrompt (trigger via human-path Analyze twice on data-xy)
    click_widget_center(driver, &WidgetId::from("nav.sheet.data-xy")).expect("select data-xy");
    click_widget_center(driver, &WidgetId::from("toolbar.analyze")).expect("open AnalyzeSheet 1");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("create first analysis");
    click_widget_center(driver, &WidgetId::from("nav.sheet.data-xy")).expect("back to data-xy");
    click_widget_center(driver, &WidgetId::from("toolbar.analyze")).expect("open AnalyzeSheet 2");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("triggers DuplicateAnalysisPrompt");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
    assert_modal_parity_clean(driver, "DuplicateAnalysisPrompt");
    click_widget_center(driver, &WidgetId::from("dup.choice.modify_existing")).expect("select modify existing");
    click_widget_center(driver, &WidgetId::from("dup.confirm")).expect("apply duplicate choice");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None));

    // 2. R1-R10 modal_predicates.rs disabled tooltip check on EditAnalysisParameters + Data-Col
    click_widget_center(driver, &WidgetId::from("nav.sheet.data-col")).expect("select data-col");
    click_widget_center(driver, &WidgetId::from("toolbar.analyze")).expect("open Analyze for Column table");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("run one-way ANOVA");
    press_cmd_shortcut(driver, "t").expect("open EditAnalysisParameters via Cmd+T");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    let r1_tip = hover_disabled_widget_for_tooltip(driver, "edit.paired_or_matched");
    assert!(!r1_tip.is_empty(), "R1 disabled_reason tooltip should be rendered");
    click_widget_center(driver, &WidgetId::from("modal.cancel")).expect("close EditAnalysisParameters");

    // 3. ChangeDataAnalyzed (compatible candidate selectable + incompatible candidate tooltip)
    {
        let app = driver.app_mut();
        let doc = app.document().clone();
        app.canvas_dialogs.open_change_data_analyzed(&doc, &SheetId::new("res-lin"));
    }
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { .. }));
    assert_modal_parity_clean(driver, "ChangeDataAnalyzed");
    let cd_tip = hover_disabled_widget_for_tooltip(driver, "change_data.cand.2");
    assert!(!cd_tip.is_empty());
    click_widget_center(driver, &WidgetId::from("change_data.cand.1")).expect("select compatible XY candidate");
    assert_eq!(driver.app().canvas_dialogs.change_data.selected_target_id, Some(SheetId::new("data-xy-2")));
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit retarget");
}

fn verify_format_exclude_style_layout_equation_dialogs(driver: &mut HumanPathDriver) {
    // 4. FormatSelectedPoints (empty OK disabled tooltip -> populated interactive swatches & OK)
    click_widget_center(driver, &WidgetId::from("nav.sheet.graph-1")).expect("select graph-1");
    open_menu_command(driver, "menu.graph", "ui.formatPoints.open");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }));
    assert_modal_parity_clean(driver, "FormatSelectedPoints empty");
    let _empty_fp_tip = hover_disabled_widget_for_tooltip(driver, "modal.ok");
    click_widget_center(driver, &WidgetId::from("modal.cancel")).expect("close empty FormatPoints");

    open_menu_command(driver, "menu.graph", "ui.formatPoints.open");
    {
        let app = driver.app_mut();
        app.canvas_dialogs.format_points.selected_points = vec![(0, 0), (0, 1)];
        app.view_shards.dialogs_graph_misc.layout_revision = app.view_shards.dialogs_graph_misc.layout_revision.wrapping_add(1);
    }
    click_widget_center(driver, &WidgetId::from("fmt_pts.col.red")).expect("pick Red");
    click_widget_center(driver, &WidgetId::from("fmt_pts.sym.diamond")).expect("pick Diamond");
    click_widget_center(driver, &WidgetId::from("fmt_pts.size.inc")).expect("inc marker size");
    click_widget_center(driver, &WidgetId::from("fmt_pts.labels")).expect("toggle row labels");
    assert_modal_parity_clean(driver, "FormatSelectedPoints active");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit format points");

    // 5. ExcludeWithReason
    open_menu_command(driver, "menu.edit", "ui.grid.excludeWithReason");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }));
    assert_modal_parity_clean(driver, "ExcludeWithReason");
    click_widget_center(driver, &WidgetId::from("exclude.tag.1")).expect("pick preset reason tag");
    click_and_type(driver, &WidgetId::from("exclude.note_input"), "Instrument bubble").expect("type audit note");
    click_widget_center(driver, &WidgetId::from("exclude.apply_note")).expect("save audit note");
    assert_eq!(driver.app().canvas_dialogs.exclude_with_reason.note, "Instrument bubble");
    click_widget_center(driver, &WidgetId::from("modal.cancel")).expect("close ExcludeWithReason");

    // 6. ApplyStyleFrom
    {
        let app = driver.app_mut();
        let doc = app.document().clone();
        app.canvas_dialogs.open_apply_style_from(&doc, &SheetId::new("graph-1"));
    }
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
    assert_modal_parity_clean(driver, "ApplyStyleFrom");
    click_widget_center(driver, &WidgetId::from("apply_style.donor.0")).expect("select donor graph");
    click_widget_center(driver, &WidgetId::from("apply_style.clone_frame_and_axes")).expect("toggle clone frame");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit ApplyStyleFrom");

    // 7. PageLayoutComposer
    click_widget_center(driver, &WidgetId::from("nav.sheet.layout-1")).expect("select layout-1");
    open_menu_command(driver, "menu.graph", "ui.layout.composer.open");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::PageLayoutComposer { .. }));
    assert_modal_parity_clean(driver, "PageLayoutComposer");
    click_widget_center(driver, &WidgetId::from("page_layout.p.2")).expect("apply 174mm preset");
    click_widget_center(driver, &WidgetId::from("page_layout.orientation")).expect("flip orientation");
    click_widget_center(driver, &WidgetId::from("page_layout.rows.inc")).expect("inc rows");
    click_widget_center(driver, &WidgetId::from("layout.align_plot_origins")).expect("toggle align plot origins");
    click_widget_center(driver, &WidgetId::from("page_layout.badge.lower")).expect("switch badges to a, b");
    click_and_type(driver, &WidgetId::from("page_layout.callout_input"), "Figure 1A").expect("type callout");
    click_widget_center(driver, &WidgetId::from("page_layout.add_callout")).expect("add callout");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit PageLayoutComposer");

    // 8. InsertModelEquation
    click_widget_center(driver, &WidgetId::from("nav.sheet.graph-1")).expect("select graph-1");
    open_menu_command(driver, "menu.graph", "ui.graph.insertModelEquation");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::InsertModelEquation { .. }));
    assert_modal_parity_clean(driver, "InsertModelEquation");
    click_widget_center(driver, &WidgetId::from("eq.font.inc")).expect("inc eq font");
    click_widget_center(driver, &WidgetId::from("equation.toggle_mode")).expect("toggle eq mode");
    click_widget_center(driver, &WidgetId::from("equation.bordered")).expect("toggle eq border");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit InsertModelEquation");
}

fn verify_randomization_sampling_simulation_dialogs(driver: &mut HumanPathDriver) {
    // 9. RandomizeGroups
    open_menu_command(driver, "menu.insert", "ui.table.randomizeGroups");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::RandomizeGroups));
    assert_modal_parity_clean(driver, "RandomizeGroups");
    let _rg_tip = hover_disabled_widget_for_tooltip(driver, "rand_groups.block.inc");
    click_widget_center(driver, &WidgetId::from("rand_groups.sch.block")).expect("switch to PermutedBlock");
    assert_eq!(driver.app().canvas_dialogs.randomize_groups.scheme, RandomizationSchemeKind::PermutedBlock);
    click_widget_center(driver, &WidgetId::from("rand_groups.block.inc")).expect("step block size");
    click_widget_center(driver, &WidgetId::from("rand_groups.n.inc")).expect("step subject count");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit RandomizeGroups");

    // 10. RandomSampleRows
    open_menu_command(driver, "menu.insert", "ui.table.randomSampleRows");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::RandomSampleRows { .. }));
    assert_modal_parity_clean(driver, "RandomSampleRows");
    let _rs_tip = hover_disabled_widget_for_tooltip(driver, "sample.proportional_allocation");
    click_widget_center(driver, &WidgetId::from("rand_sample.n.inc")).expect("step sample size");
    click_widget_center(driver, &WidgetId::from("modal.cancel")).expect("close RandomSampleRows");

    // 11. SimulateDistributions
    open_menu_command(driver, "menu.insert", "ui.table.simulateDistributions");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::SimulateDistributions));
    assert_modal_parity_clean(driver, "SimulateDistributions");
    click_widget_center(driver, &WidgetId::from("sim_dist.rows.inc")).expect("step n_rows");
    click_widget_center(driver, &WidgetId::from("sim_dist.d.bin")).expect("switch to Binomial");
    click_widget_center(driver, &WidgetId::from("sim_dist.p1.inc")).expect("step Trials");
    click_widget_center(driver, &WidgetId::from("sim_dist.add_col")).expect("add second variable");
    assert_eq!(driver.app().canvas_dialogs.simulate_distributions.columns.len(), 2);
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("commit SimulateDistributions");
}

fn verify_print_and_about_dialogs(driver: &mut HumanPathDriver) {
    // 12. Print (verify gui_only disabled tooltip on submit_to_os + bleed disabled until crop_marks)
    open_menu_command(driver, "menu.file", "ui.print.open");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::Print { .. }));
    assert_modal_parity_clean(driver, "Print");
    let print_tip = hover_disabled_widget_for_tooltip(driver, "print.submit_to_os");
    assert!(print_tip.contains("gui-only"));
    let _bleed_tip = hover_disabled_widget_for_tooltip(driver, "print.bleed.inc");
    click_widget_center(driver, &WidgetId::from("print.crop_marks")).expect("enable crop marks");
    click_widget_center(driver, &WidgetId::from("print.bleed.inc")).expect("step bleed");
    click_widget_center(driver, &WidgetId::from("print.copies.inc")).expect("inc copies");
    click_widget_center(driver, &WidgetId::from("modal.cancel")).expect("close Print");

    // 13. About
    open_menu_command(driver, "menu.help", "app.about");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::About));
    assert_modal_parity_clean(driver, "About");
    click_widget_center(driver, &WidgetId::from("about.tab.fonts")).expect("switch to Typography tab");
    click_widget_center(driver, &WidgetId::from("about.tab.shortcuts")).expect("switch to Key Bindings tab");
    assert_modal_parity_clean(driver, "About Shortcuts tab");
    click_widget_center(driver, &WidgetId::from("modal.ok")).expect("close About");
}

#[test]
fn r10_3_remaining_active_dialogs_and_modal_predicate_tooltips() {
    let mut driver = new_human_driver(build_rich_workspace(), default_viewport());
    verify_duplicate_and_change_data_dialogs(&mut driver);
    verify_format_exclude_style_layout_equation_dialogs(&mut driver);
    verify_randomization_sampling_simulation_dialogs(&mut driver);
    verify_print_and_about_dialogs(&mut driver);
    assert_zero_shortcut_compliance(&mut driver).expect("zero-shortcut compliance");
    assert_input_to_paint_budget(&driver, 500.0).expect("input-to-paint budget");
}

#[test]
fn r10_6_command_palette_interactive_search_selection_and_execution() {
    let mut driver = new_human_driver(build_rich_workspace(), default_viewport());

    // Open Command Palette via Cmd+K shortcut, focus search field, and type live filter
    press_cmd_shortcut(&mut driver, "k").expect("Cmd+K opens command palette");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    assert_modal_parity_clean(&mut driver, "CommandPalette open");

    // Live search typing filters command rows
    click_and_type(&mut driver, &WidgetId::from("palette.query"), "Print").expect("type Print into palette.query");
    assert_eq!(driver.app().focus_widget_id.as_deref(), Some("palette.query"));
    let first_hit_label = driver.find_widget("palette.hit.0").expect("filtered Print row").a11y.label.clone();
    assert!(first_hit_label.to_ascii_lowercase().contains("print"), "unexpected first hit: {first_hit_label}");

    // Interactive Prev/Next selection updates index & status cleanly
    click_widget_center(&mut driver, &WidgetId::from("palette.next")).expect("next palette hit");
    click_widget_center(&mut driver, &WidgetId::from("palette.prev")).expect("prev palette hit");
    assert_eq!(driver.app().canvas_dialogs.command_palette.selected_index, 0);

    // Filter to About and execute via Enter key
    click_and_type(&mut driver, &WidgetId::from("palette.query"), "").expect("focus query");
    for _ in 0..8 {
        let _ = press_key_combo(&mut driver, "Backspace", KeyModifiers::default());
    }
    type_into_focused(&mut driver, "About").expect("filter to About");
    assert_modal_parity_clean(&mut driver, "CommandPalette About filter");
    press_key_combo(&mut driver, "Enter", KeyModifiers::default()).expect("Enter executes selected command");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::About));
    click_widget_center(&mut driver, &WidgetId::from("modal.ok")).expect("close About");

    // Reopen via toolbar button, clear buffer, filter to About, and run via palette.run
    click_widget_center(&mut driver, &WidgetId::from("toolbar.palette")).expect("reopen via toolbar");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    click_and_type(&mut driver, &WidgetId::from("palette.query"), "").expect("focus palette.query");
    for _ in 0..8 {
        let _ = press_key_combo(&mut driver, "Backspace", KeyModifiers::default());
    }
    type_into_focused(&mut driver, "About").expect("filter palette for Run button");
    click_widget_center(&mut driver, &WidgetId::from("palette.run")).expect("execute via Run button");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::About));
    click_widget_center(&mut driver, &WidgetId::from("modal.ok")).expect("close About after Run");

    assert_zero_shortcut_compliance(&mut driver).expect("zero-shortcut compliance");
    assert_input_to_paint_budget(&driver, 500.0).expect("input-to-paint budget");
}

#[test]
fn r10_15_zoom_selector_pan_scrollbars_and_hover_selected_only_chrome() {
    let mut driver = new_human_driver(build_rich_workspace(), default_viewport());

    // 1. Series chips hidden until hovered or selected
    click_widget_center(&mut driver, &WidgetId::from("nav.sheet.graph-1")).expect("activate graph-1");
    assert!(!should_show_graph_series_chip(driver.app(), 0));
    let idle_graph = driver.render_frame().expect("idle graph render");
    hover_widget_center(&mut driver, &WidgetId::from("canvas.series.0")).expect("hover series 0 hotspot");
    assert!(should_show_graph_series_chip(driver.app(), 0));
    let hover_graph = driver.render_frame().expect("hovered graph render");
    assert!(count_diff_pixels(&idle_graph, &hover_graph) > 0, "hovering series hotspot must paint visible chip pixels");
    click_widget_center(&mut driver, &WidgetId::from("canvas.series.0")).expect("select series 0");
    assert_eq!(driver.app().canvas_dialogs.canvas.selected_object_id.as_deref(), Some("series.0"));

    // 2. Layout panel selection borders & badges hidden unless hovered/selected
    click_widget_center(&mut driver, &WidgetId::from("nav.sheet.layout-1")).expect("activate layout-1");
    assert!(!should_show_layout_panel_chrome(driver.app(), "panel.0"));
    let idle_layout = driver.render_frame().expect("idle layout render");
    hover_widget_center(&mut driver, &WidgetId::from("canvas.layout_panel.panel.0")).expect("hover layout panel 0");
    assert!(should_show_layout_panel_chrome(driver.app(), "panel.0"));
    let hover_layout = driver.render_frame().expect("hovered layout render");
    assert!(count_diff_pixels(&idle_layout, &hover_layout) > 10, "hovering layout panel hotspot must show selection border and badge");

    // 3. Explicit Zoom Percentage Selector dropdown & presets (Fit, 50%, 150%, 200%) + Interactive Pan/Scrollbars
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_select")).expect("open zoom preset dropdown");
    assert!(driver.find_widget("canvas.main.zoom_menu").is_some());
    assert_modal_parity_clean(&mut driver, "Canvas Zoom DropdownMenu");
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_preset.150")).expect("select 150%");
    assert!((driver.app().canvas_dialogs.canvas.zoom - 1.5).abs() < 1e-6);
    assert!(driver.find_widget("canvas.main.zoom_menu").is_none());

    // 4. Interactive Pan & Scrollbars while zoomed in (150% overflows paper inside mat)
    let before_pan = driver.app().canvas_dialogs.canvas.pan_pt;
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.h_scrollbar")).expect("click horizontal scrollbar");
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.v_scrollbar")).expect("click vertical scrollbar");
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.pan.right")).expect("nudge pan right");
    assert_ne!(driver.app().canvas_dialogs.canvas.pan_pt, before_pan);
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.pan.reset")).expect("reset pan");

    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_select")).expect("reopen zoom dropdown");
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_preset.50")).expect("select 50%");
    assert!((driver.app().canvas_dialogs.canvas.zoom - 0.5).abs() < 1e-6);

    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_select")).expect("open zoom menu for Fit");
    click_widget_center(&mut driver, &WidgetId::from("canvas.main.zoom_preset.fit")).expect("select Fit");
    assert!((driver.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);
    assert!((driver.app().canvas_dialogs.canvas.pan_pt.x).abs() < 1e-6);
    assert!((driver.app().canvas_dialogs.canvas.pan_pt.y).abs() < 1e-6);

    assert_zero_shortcut_compliance(&mut driver).expect("zero-shortcut compliance");
    assert_input_to_paint_budget(&driver, 500.0).expect("input-to-paint budget");
}
