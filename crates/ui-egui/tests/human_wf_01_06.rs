#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, CellCoord, DataSheet, MathTransformPreset, MultivariateSpec, ResultCell, ResultStatus, SheetId, TableArchetype, TableKind,
    TransformAnalysisSpec, VariableRole,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, UiInputEvent, dispatch_ui_event};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::{HumanPathDriver, UiApp};
use human_wf_common::{
    assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, hover_widget_center, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused, widget_hit_center,
};

type TestRes = Result<(), String>;

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn new_default_human_driver() -> Result<HumanPathDriver, String> {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let _ = driver.render_frame().map_err(|e| format!("initial render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(driver)
}

fn is_widget_visible(driver: &mut HumanPathDriver, id: &str) -> bool {
    let tree = driver.layout_tree();
    tree.node(id).is_some_and(|n| n.drawn_in_frame && n.bounds.visible_rect_px.width > 0 && n.bounds.visible_rect_px.height > 0)
}

fn scroll_into_view(driver: &mut HumanPathDriver, container: &str, target: &str, step_y: f64, max_steps: usize) -> TestRes {
    if is_widget_visible(driver, target) {
        return Ok(());
    }
    if scroll_until_widget_visible(driver, &wid(container), &wid(target), step_y, max_steps).is_ok() {
        return Ok(());
    }
    scroll_until_widget_visible(driver, &wid(container), &wid(target), -step_y, max_steps)
}

fn scroll_and_click(driver: &mut HumanPathDriver, container: &str, target: &str, step_y: f64, max_steps: usize) -> TestRes {
    scroll_into_view(driver, container, target, step_y, max_steps)?;
    click_widget_center(driver, &wid(target))?;
    Ok(())
}

fn replace_textbox(driver: &mut HumanPathDriver, id: &str, text: &str) -> TestRes {
    click_widget_center(driver, &wid(id))?;
    press_key_combo(driver, "a", KeyModifiers { ctrl: true, ..Default::default() })?;
    press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    if !text.is_empty() {
        type_into_focused(driver, text)?;
    }
    Ok(())
}

fn modifier_click_logical(driver: &mut HumanPathDriver, lx: f64, ly: f64, button: PointerButton, modifiers: KeyModifiers) -> TestRes {
    let metrics = driver.metrics();
    let _ = dispatch_ui_event(driver.app_mut(), UiInputEvent::PointerDown { x: lx, y: ly, button, modifiers }, metrics)?;
    let _ = dispatch_ui_event(driver.app_mut(), UiInputEvent::PointerUp { x: lx, y: ly, button, modifiers }, metrics)?;
    Ok(())
}

fn modifier_click_widget(driver: &mut HumanPathDriver, widget_id: &str, modifiers: KeyModifiers) -> TestRes {
    let [lx, ly] = widget_hit_center(driver, &wid(widget_id))?;
    modifier_click_logical(driver, lx, ly, PointerButton::Primary, modifiers)
}

fn right_click_widget(driver: &mut HumanPathDriver, widget_id: &str) -> TestRes {
    let [lx, ly] = widget_hit_center(driver, &wid(widget_id))?;
    modifier_click_logical(driver, lx, ly, PointerButton::Secondary, KeyModifiers::default())
}

fn click_menu_item(driver: &mut HumanPathDriver, menu_id: &str, popup_id: &str, item_id: &str) -> TestRes {
    click_widget_center(driver, &wid(menu_id))?;
    scroll_and_click(driver, popup_id, item_id, 66.0, 20)
}

fn confirm_change_graph_type_if_open(driver: &mut HumanPathDriver) -> TestRes {
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        click_widget_center(driver, &wid("modal.ok"))?;
    }
    Ok(())
}

struct WelcomeTableSpec<'a> {
    arch_slug: &'a str,
    title: &'a str,
    cols: usize,
    rows: usize,
    replicates: Option<usize>,
    auto_graph: bool,
    auto_info: bool,
}

fn step_welcome_count(driver: &mut HumanPathDriver, prefix: &str, target: usize, read_cur: fn(&UiApp) -> usize) -> TestRes {
    let inc_id = format!("{prefix}.inc");
    let dec_id = format!("{prefix}.dec");
    scroll_into_view(driver, "welcome.right_body", &inc_id, 80.0, 20)?;
    for _ in 0..520 {
        let cur = read_cur(driver.app());
        if cur == target {
            return Ok(());
        }
        let step_id = if cur < target { inc_id.as_str() } else { dec_id.as_str() };
        click_widget_center(driver, &wid(step_id))?;
    }
    Err(format!("welcome stepper `{prefix}` did not reach target {target} (stuck at {})", read_cur(driver.app())))
}

fn set_welcome_toggle(driver: &mut HumanPathDriver, id: &str, desired: bool, read_cur: fn(&UiApp) -> bool) -> TestRes {
    scroll_into_view(driver, "welcome.right_body", id, 80.0, 20)?;
    if read_cur(driver.app()) != desired {
        click_widget_center(driver, &wid(id))?;
    }
    if read_cur(driver.app()) != desired {
        return Err(format!("welcome toggle `{id}` failed to reach {desired}"));
    }
    Ok(())
}

fn create_table_via_welcome(driver: &mut HumanPathDriver, spec: &WelcomeTableSpec<'_>) -> Result<(String, Option<String>, Option<String>), String> {
    if !is_widget_visible(driver, "welcome.create_cta") && driver.app().document().total_sheet_count() > 0 {
        click_menu_item(driver, "menu.file", "popup.menu.file", "menu.ui.clonePipeline.open")?;
    }
    if !is_widget_visible(driver, "welcome.create_cta") {
        return Err(String::from("expected Welcome overlay to be open before creating table"));
    }

    let arch_id = format!("welcome.arch.{}", spec.arch_slug);
    scroll_and_click(driver, "welcome.left_rail", &arch_id, 60.0, 16)?;
    click_widget_center(driver, &wid("welcome.tab.empty"))?;

    scroll_into_view(driver, "welcome.right_body", "welcome.title.input", -80.0, 20)?;
    replace_textbox(driver, "welcome.title.input", spec.title)?;

    if let Some(reps) = spec.replicates {
        let sub_id = if reps > 1 { "welcome.subcol.raw" } else { "welcome.subcol.single" };
        scroll_and_click(driver, "welcome.right_body", sub_id, 80.0, 20)?;
        if reps > 1 {
            step_welcome_count(driver, "welcome.replicates", reps, |app| app.nav.welcome.table_draft.replicates)?;
        }
    }

    step_welcome_count(driver, "welcome.datasets", spec.cols, |app| app.nav.welcome.table_draft.data_set_count)?;
    step_welcome_count(driver, "welcome.rows", spec.rows, |app| app.nav.welcome.table_draft.row_count)?;
    set_welcome_toggle(driver, "welcome.toggle.auto_graph", spec.auto_graph, |app| app.nav.welcome.table_draft.create_default_graph)?;
    set_welcome_toggle(driver, "welcome.toggle.auto_info", spec.auto_info, |app| app.nav.welcome.table_draft.create_paired_info)?;

    let before_count = driver.app().document().data_sheets.len();
    click_widget_center(driver, &wid("welcome.create_cta"))?;

    let doc = driver.app().document();
    if doc.data_sheets.len() != before_count + 1 {
        return Err(format!("expected new DataSheet after Create Table, count remained {}", doc.data_sheets.len()));
    }
    let ds = doc.data_sheets.last().ok_or_else(|| String::from("missing newly created DataSheet"))?;
    let linked_graph = doc.graph_sheets.iter().rfind(|g| g.spec.primary_source_sheet_id == ds.id).map(|g| g.id.as_str().to_owned());
    Ok((ds.id.as_str().to_owned(), linked_graph, ds.info_sheet_id.as_ref().map(|id| id.as_str().to_owned())))
}

fn set_grid_title(driver: &mut HumanPathDriver, title_widget_id: &str, new_title: &str) -> TestRes {
    click_widget_center(driver, &wid(title_widget_id))?;
    if !is_widget_visible(driver, "grid.title_editor.input") {
        return Err(format!("clicking `{title_widget_id}` did not open grid.title_editor.input"));
    }
    replace_textbox(driver, "grid.title_editor.input", new_title)?;
    press_key_combo(driver, "Enter", KeyModifiers::default())?;
    Ok(())
}

fn enter_grid_cell(driver: &mut HumanPathDriver, grid_scroll_id: &str, cell_widget_id: &str, text: &str, backspaces: usize) -> TestRes {
    scroll_into_view(driver, grid_scroll_id, cell_widget_id, 180.0, 80)?;
    let [lx, ly] = widget_hit_center(driver, &wid(cell_widget_id))?;
    let _ = driver.double_click_logical(lx, ly)?;
    for _ in 0..backspaces {
        press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    }
    if !text.is_empty() {
        let _ = driver.type_text(text)?;
    }
    press_key_combo(driver, "Enter", KeyModifiers::default())?;
    if driver.app().inline_editor.active {
        return Err(format!("inline editor remained active after Enter on `{cell_widget_id}`"));
    }
    Ok(())
}

fn fill_numeric_rows(driver: &mut HumanPathDriver, sheet_id: &str, rows: &[&[&str]]) -> TestRes {
    let grid_scroll = format!("grid.sheet.{sheet_id}");
    for (r_idx, row) in rows.iter().enumerate() {
        for (c_idx, val) in row.iter().enumerate() {
            if val.is_empty() {
                continue;
            }
            let cell_id = format!("grid.cell.{r_idx}.{c_idx}.0");
            enter_grid_cell(driver, &grid_scroll, &cell_id, val, 0)?;
        }
    }
    Ok(())
}

fn run_analyze_entry(driver: &mut HumanPathDriver, search: &str, entry_id: &str) -> Result<String, String> {
    let before_len = driver.app().document().result_sheets.len();
    click_menu_item(driver, "menu.analyze", "popup.menu.analyze", "menu.ui.analyze.open")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("expected AnalyzeSheet dialog after menu.ui.analyze.open"));
    }
    replace_textbox(driver, "analyze.search", search)?;
    let entry_wid = format!("analyze.entry.{entry_id}");
    scroll_and_click(driver, "analyze.entries_list", &entry_wid, 60.0, 20)?;
    click_widget_center(driver, &wid("modal.ok"))?;

    let doc = driver.app().document();
    if doc.result_sheets.len() <= before_len {
        return Err(format!("expected `{entry_id}` confirmation to create a ResultSheet"));
    }
    let active_rs = doc
        .active_sheet_id
        .as_ref()
        .filter(|id| doc.result_sheet(id).is_some())
        .or_else(|| doc.result_sheets.last().map(|r| &r.id))
        .ok_or_else(|| format!("missing ResultSheet after `{entry_id}`"))?;
    Ok(active_rs.as_str().to_owned())
}

fn result_sheet_scalar(app: &UiApp, sheet_id: &str, name_fragment: &str) -> Result<f64, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let rs = app.document().result_sheet(&sid).ok_or_else(|| format!("missing result sheet `{sheet_id}`"))?;
    rs.scalars
        .iter()
        .find(|s| s.key.contains(name_fragment))
        .map(|s| s.value)
        .ok_or_else(|| format!("missing scalar `{name_fragment}` on `{sheet_id}`"))
}

fn assert_result_cell_close(cell: Option<&ResultCell>, expected: f64, tol: f64, label: &str) -> TestRes {
    let actual = match cell {
        Some(ResultCell::Number { value, .. }) => *value,
        other => return Err(format!("expected numeric cell for `{label}`, got {other:?}")),
    };
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("`{label}` expected ~{expected}, got {actual}"));
    }
    Ok(())
}

fn verify_canvas_point_override_scene(driver: &mut HumanPathDriver, graph_id_str: &str) -> TestRes {
    let graph_id = SheetId::try_new(graph_id_str).map_err(|e| e.to_string())?;
    let gs = driver.app().document().graph_sheet(&graph_id).ok_or_else(|| format!("missing `{graph_id_str}`"))?;
    if gs.spec.point_overrides.is_empty() {
        return Err(String::from("expected Format Selected Points to add point_overrides"));
    }
    let diamond_present =
        gs.spec.point_overrides.iter().any(|ov| ov.symbol.is_some_and(|s| format!("{s:?}") == "Diamond") && ov.size_pt.is_some_and(|sz| sz >= 8.0));
    if !diamond_present {
        return Err(format!("expected diamond/size>=8 override in {:?}", gs.spec.point_overrides));
    }
    let scene = driver.app().session().render_sheet_scene(&graph_id).map_err(|e| e.to_string())?;
    if scene.plotted_points.is_empty() {
        return Err(String::from("expected linked graph scene to plot XY points"));
    }
    Ok(())
}

#[test]
fn wf01_human_xy_dose_response_format_exclude_with_reason() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Create 6x1 XY table with 2 replicates + linked graph + info sheet via Welcome.
    let (xy_id_str, graph_opt, info_opt) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec { arch_slug: "xy", title: "Dose Response", cols: 1, rows: 6, replicates: Some(2), auto_graph: true, auto_info: true },
    )?;
    let graph_id_str = graph_opt.ok_or_else(|| String::from("expected auto-created linked graph"))?;
    if info_opt.is_none() {
        return Err(String::from("expected auto-created paired info sheet"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // Stage 2: Set X title, enter X concentrations (incl. edit 105 -> 100), and enter replicate Y responses.
    let grid_scroll = format!("grid.sheet.{xy_id_str}");
    set_grid_title(&mut driver, "grid.x_title.0", "Concentration (nM)")?;
    for (row, x_str) in [(0, "1"), (1, "3"), (2, "10"), (3, "30"), (4, "105"), (5, "300")] {
        enter_grid_cell(&mut driver, &grid_scroll, &format!("grid.xcell.{row}.0"), x_str, 0)?;
    }
    let xy_sid = SheetId::try_new(&xy_id_str).map_err(|e| e.to_string())?;
    let x4_initial =
        driver.app().document().data_sheet(&xy_sid).and_then(|ds| ds.x_or_time_cell(4, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
    if (x4_initial - 105.0).abs() > 1e-9 {
        return Err(format!("expected grid.xcell.4.0 == 105 after X typing, got {x4_initial}"));
    }
    enter_grid_cell(&mut driver, &grid_scroll, "grid.xcell.4.0", "100", 3)?;

    let rep_values = [("98.0", "101.0"), ("88.0", "91.0"), ("68.0", "72.0"), ("42.0", "46.0"), ("19.0", "21.0"), ("6.0", "8.0")];
    for (row, (y0, y1)) in rep_values.iter().enumerate() {
        enter_grid_cell(&mut driver, &grid_scroll, &format!("grid.cell.{row}.0.0"), y0, 0)?;
        enter_grid_cell(&mut driver, &grid_scroll, &format!("grid.cell.{row}.0.1"), y1, 0)?;
    }
    enter_grid_cell(&mut driver, &grid_scroll, "grid.cell.4.0.1", "21.5", 4)?;
    let cell_4_1 = driver
        .app()
        .document()
        .data_sheet(&xy_sid)
        .and_then(|ds| ds.cell(CellCoord::new(4, 0, 1)))
        .and_then(|c| c.active_number())
        .unwrap_or(f64::NAN);
    if (cell_4_1 - 21.5).abs() > 1e-9 {
        return Err(format!("expected cell (4,0,1) == 21.5 after edit, got {cell_4_1}"));
    }

    // Stage 3: Shift/Meta multi-row selection -> Format Selected Points... + Exclude with Reason...
    click_widget_center(&mut driver, &wid("grid.cell.1.0.0"))?;
    modifier_click_widget(&mut driver, "grid.cell.3.0.0", KeyModifiers { shift: true, ..Default::default() })?;
    modifier_click_widget(&mut driver, "grid.cell.5.0.0", KeyModifiers { meta: true, ..Default::default() })?;
    let selected_rows = &driver.app().grid.selected_rows;
    if !selected_rows.contains(&1) || !selected_rows.contains(&3) || !selected_rows.contains(&5) {
        return Err(format!("expected selected_rows to contain 1,3,5, got {selected_rows:?}"));
    }

    click_menu_item(&mut driver, "menu.graph", "popup.menu.graph", "menu.ui.formatPoints.open")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }) {
        return Err(String::from("expected FormatSelectedPoints modal open"));
    }
    click_widget_center(&mut driver, &wid("fmt_pts.col.red"))?;
    click_widget_center(&mut driver, &wid("fmt_pts.sym.diamond"))?;
    for _ in 0..3 {
        click_widget_center(&mut driver, &wid("fmt_pts.size.inc"))?;
    }
    scroll_and_click(&mut driver, "fmt_pts.panel", "fmt_pts.labels", 80.0, 10)?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    // Right-click outlier cell (2,0,1) -> Exclude with Reason...
    click_widget_center(&mut driver, &wid("grid.cell.2.0.1"))?;
    right_click_widget(&mut driver, "grid.cell.2.0.1")?;
    click_widget_center(&mut driver, &wid("ctx.grid.exclude_reason"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
        return Err(String::from("expected ExcludeWithReason modal open"));
    }
    click_widget_center(&mut driver, &wid("exclude.tag.0"))?;
    replace_textbox(&mut driver, "exclude.note_input", "Pipette bubble at 10 nM replicate B")?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    let outlier_coord = CellCoord::new(2, 0, 1);
    {
        let ds = driver.app().document().data_sheet(&xy_sid).ok_or_else(|| String::from("missing xy sheet"))?;
        if !ds.cell(outlier_coord).is_some_and(|c| c.excluded) {
            return Err(String::from("expected cell (2,0,1) to be excluded"));
        }
        let note = ds.exclusion_note(outlier_coord).ok_or_else(|| String::from("missing audit exclusion note"))?;
        if note.reason_tag != "Outlier" || !note.note.contains("Pipette bubble") {
            return Err(format!("unexpected exclusion note: {note:?}"));
        }
    }
    hover_widget_center(&mut driver, &wid("grid.cell.2.0.1"))?;
    if driver.app().grid.hover_exclusion_tooltip.as_deref().unwrap_or("").is_empty() {
        return Err(String::from("expected hover exclusion provenance tooltip on excluded cell"));
    }

    // Stage 4: Switch to linked graph, verify overrides & scene, parity, and RGBA render.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{graph_id_str}"), 60.0, 16)?;
    confirm_change_graph_type_if_open(&mut driver)?;
    verify_canvas_point_override_scene(&mut driver, &graph_id_str)?;
    let _ = driver.render_frame().map_err(|e| format!("graph render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // Stage 5 + Large-N (>= 120 rows) off-screen scroll-to-row edit & verify check + reactive cell edit.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{xy_id_str}"), -60.0, 16)?;
    enter_grid_cell(&mut driver, &grid_scroll, "grid.cell.0.0.0", "97.5", 4)?;

    let (large_id_str, _, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec {
            arch_slug: "xy",
            title: "Large N Dose Scroll",
            cols: 1,
            rows: 125,
            replicates: Some(1),
            auto_graph: false,
            auto_info: false,
        },
    )?;
    let large_scroll = format!("grid.sheet.{large_id_str}");
    if is_widget_visible(&mut driver, "grid.cell.122.0.0") {
        return Err(String::from("expected row 122 to start off-screen before wheel scrolling"));
    }
    enter_grid_cell(&mut driver, &large_scroll, "grid.cell.122.0.0", "765.25", 0)?;
    let large_sid = SheetId::try_new(&large_id_str).map_err(|e| e.to_string())?;
    let val_122 = driver
        .app()
        .document()
        .data_sheet(&large_sid)
        .and_then(|ds| ds.cell(CellCoord::new(122, 0, 0)))
        .and_then(|c| c.active_number())
        .unwrap_or(f64::NAN);
    if (val_122 - 765.25).abs() > 1e-9 {
        return Err(format!("expected off-screen row 122 value 765.25, got {val_122}"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn wf02_human_normalize_to_result_sheet_chaining_and_log_x() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Create 6x1 XY table via Welcome and enter logarithmic X + response Y data.
    let (xy_id_str, _, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec { arch_slug: "xy", title: "Raw Assay", cols: 1, rows: 6, replicates: Some(1), auto_graph: true, auto_info: false },
    )?;
    let grid_scroll = format!("grid.sheet.{xy_id_str}");
    for (row, (x_s, y_s)) in [("1e-9", "10"), ("1e-8", "20"), ("1e-7", "50"), ("1e-6", "80"), ("1e-5", "90"), ("1e-4", "100")].into_iter().enumerate()
    {
        enter_grid_cell(&mut driver, &grid_scroll, &format!("grid.xcell.{row}.0"), x_s, 0)?;
        enter_grid_cell(&mut driver, &grid_scroll, &format!("grid.cell.{row}.0.0"), y_s, 0)?;
    }

    // Stage 2: Normalize XY responses (0-100%) via Analyze... picker.
    let norm_res_id = run_analyze_entry(&mut driver, "normalize", "transform.normalize")?;
    let norm_sid = SheetId::try_new(&norm_res_id).map_err(|e| e.to_string())?;
    {
        let norm_rs = driver.app().document().result_sheet(&norm_sid).ok_or_else(|| String::from("missing normalize result"))?;
        if norm_rs.status != ResultStatus::UpToDate {
            return Err(format!("expected UpToDate normalize result, got {:?}", norm_rs.status));
        }
        let norm_data = norm_rs.analyzable_data.as_ref().ok_or_else(|| String::from("missing normalize analyzable_data"))?;
        let y0 = norm_data.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        let y5 = norm_data.cell(CellCoord::new(5, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        if y0.abs() > 1e-9 || (y5 - 100.0).abs() > 1e-9 {
            return Err(format!("expected normalized endpoints 0 and 100, got {y0} and {y5}"));
        }
    }

    // Stage 3: Chain transform.math directly from active Normalize ResultSheet, then configure Log10(X), Y=None via Cmd+T.
    let log_res_id = run_analyze_entry(&mut driver, "math", "transform.math")?;
    let log_sid = SheetId::try_new(&log_res_id).map_err(|e| e.to_string())?;

    press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("expected EditAnalysisParameters dialog via Cmd+T"));
    }
    for _ in 0..16 {
        let is_log10_x = matches!(
            driver.app().document().result_sheet(&log_sid).map(|_| &driver.app().canvas_dialogs.edit_params.draft),
            Some(Some(spec)) if matches!(spec.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), .. }))
        );
        if is_log10_x {
            break;
        }
        scroll_and_click(&mut driver, "modal.body", "edit.transform.x_transform", 60.0, 8)?;
    }
    for _ in 0..16 {
        let is_none_y = matches!(
            &driver.app().canvas_dialogs.edit_params.draft,
            Some(spec) if matches!(spec.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: None, .. }))
        );
        if is_none_y {
            break;
        }
        scroll_and_click(&mut driver, "modal.body", "edit.transform.y_transform", 60.0, 8)?;
    }
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    // Stage 4: Verify chained Log10(X) / None(Y) analyzable_data and parity.
    {
        let log_rs = driver.app().document().result_sheet(&log_sid).ok_or_else(|| format!("missing `{log_res_id}`"))?;
        if log_rs.analysis.source_sheet_id.as_str() != norm_res_id {
            return Err(format!("expected `{log_res_id}` parent `{norm_res_id}`, got `{}`", log_rs.analysis.source_sheet_id));
        }
        if !matches!(
            &log_rs.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), y_transform: None, .. })
        ) {
            return Err(format!("expected Transform::Math(x=Log10, y=None) configured via EditAnalysisParameters, got {:?}", log_rs.analysis.kind));
        }
        let log_data = log_rs.analyzable_data.as_ref().ok_or_else(|| String::from("missing log-x analyzable_data"))?;
        let TableArchetype::Xy(xy_log) = &log_data.archetype else {
            return Err(String::from("expected XY archetype in log-x analyzable_data"));
        };
        let x0 = xy_log.x_value_at(0).unwrap_or(f64::NAN);
        let x5 = xy_log.x_value_at(5).unwrap_or(f64::NAN);
        let y4 = log_data.cell(CellCoord::new(4, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        if (x0 - (-9.0)).abs() > 1e-9 || (x5 - (-4.0)).abs() > 1e-9 {
            return Err(format!("expected Log10(X) in [-9.0, -4.0], got [{x0}, {x5}]"));
        }
        let expected_y4 = 80.0 / 90.0 * 100.0;
        if (y4 - expected_y4).abs() > 1e-6 {
            return Err(format!("expected normalized Y[4]={expected_y4}, got {y4}"));
        }
    }

    // Stage 5: Live upstream cell edit (Y[5]: 100 -> 110) propagates across Normalize -> Log10(X) chain.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{xy_id_str}"), -60.0, 16)?;
    enter_grid_cell(&mut driver, &grid_scroll, "grid.cell.5.0.0", "110", 3)?;
    let log_rs_after = driver.app().document().result_sheet(&log_sid).ok_or_else(|| String::from("missing log-x after edit"))?;
    let log_after_data = log_rs_after.analyzable_data.as_ref().ok_or_else(|| String::from("missing downstream analyzable_data"))?;
    let y4_after = log_after_data.cell(CellCoord::new(4, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
    if (y4_after - 80.0).abs() > 1e-6 {
        return Err(format!("expected downstream reactive recompute Y[4]=80.0, got {y4_after}"));
    }
    let _ = driver.render_frame().map_err(|e| format!("render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn wf03_human_column_brushing_format_and_undo_redo() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Create 3-group Column table (5 rows) + linked graph + info sheet via Welcome.
    let (col_id_str, graph_opt, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec { arch_slug: "column", title: "Three Treatments", cols: 3, rows: 5, replicates: None, auto_graph: true, auto_info: true },
    )?;
    let graph_id_str = graph_opt.ok_or_else(|| String::from("expected auto-created linked graph"))?;

    // Stage 2: Set group column titles and enter 5x3 observations via grid double-click/typing.
    for (col_idx, title) in [(0, "Vehicle"), (1, "LowDose"), (2, "HighDose")] {
        set_grid_title(&mut driver, &format!("grid.col_title.{col_idx}"), title)?;
    }
    fill_numeric_rows(
        &mut driver,
        &col_id_str,
        &[&["10.1", "14.2", "19.8"], &["10.8", "15.0", "20.4"], &["11.2", "14.7", "21.1"], &["10.5", "15.3", "19.5"], &["11.0", "14.9", "20.8"]],
    )?;

    // Multi-row grid brushing (Shift range + Meta toggle).
    click_widget_center(&mut driver, &wid("grid.cell.0.0.0"))?;
    modifier_click_widget(&mut driver, "grid.cell.2.0.0", KeyModifiers { shift: true, ..Default::default() })?;
    modifier_click_widget(&mut driver, "grid.cell.4.1.0", KeyModifiers { meta: true, ..Default::default() })?;
    if driver.app().grid.selected_rows.len() < 3 || driver.app().brushed_points.len() < 2 {
        return Err(format!(
            "expected multi-row selection + synced brushed_points, got rows={:?}, brushed={}",
            driver.app().grid.selected_rows,
            driver.app().brushed_points.len()
        ));
    }

    // Stage 3: Switch to linked graph, brush 2 points directly on canvas via physical Shift+Click, Format Selected Points.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{graph_id_str}"), 60.0, 16)?;
    confirm_change_graph_type_if_open(&mut driver)?;

    let canvas_pts: Vec<[f64; 2]> = {
        let snap = inspect_ui_tree_for_metrics(driver.app(), driver.metrics(), Some("canvas"), true);
        snap.canvas_elements
            .iter()
            .flatten()
            .filter(|w| w.widget_id.starts_with("canvas.point."))
            .filter_map(|w| w.bounds.map(|b| [b[0] + b[2] * 0.5, b[1] + b[3] * 0.5]))
            .take(2)
            .collect()
    };
    if canvas_pts.len() < 2 {
        return Err(format!("expected >= 2 canvas.point.* bounds, found {}", canvas_pts.len()));
    }
    modifier_click_logical(&mut driver, canvas_pts[0][0], canvas_pts[0][1], PointerButton::Primary, KeyModifiers::default())?;
    modifier_click_logical(
        &mut driver,
        canvas_pts[1][0],
        canvas_pts[1][1],
        PointerButton::Primary,
        KeyModifiers { shift: true, ..Default::default() },
    )?;
    if driver.app().brushed_points.len() != 2 {
        return Err(format!("expected 2 brushed_points after graph Shift+Click, got {}", driver.app().brushed_points.len()));
    }

    click_menu_item(&mut driver, "menu.graph", "popup.menu.graph", "menu.ui.formatPoints.open")?;
    click_widget_center(&mut driver, &wid("fmt_pts.col.blue"))?;
    click_widget_center(&mut driver, &wid("fmt_pts.sym.square"))?;
    for _ in 0..3 {
        click_widget_center(&mut driver, &wid("fmt_pts.size.inc"))?;
    }
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    let graph_sid = SheetId::try_new(&graph_id_str).map_err(|e| e.to_string())?;
    if driver.app().document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len()) != 2 {
        return Err(String::from("expected 2 point_overrides after Format Selected Points"));
    }

    // Stage 4: Undo / Redo via Cmd+Z / Cmd+Shift+Z AND Edit menu Undo / Redo.
    press_cmd_shortcut(&mut driver, "z")?;
    if !driver.app().document().graph_sheet(&graph_sid).is_some_and(|g| g.spec.point_overrides.is_empty()) {
        return Err(String::from("expected Cmd+Z to undo point_overrides"));
    }
    press_key_combo(&mut driver, "z", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    if driver.app().document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len()) != 2 {
        return Err(String::from("expected Cmd+Shift+Z to redo point_overrides"));
    }
    click_menu_item(&mut driver, "menu.edit", "popup.menu.edit", "menu.edit.undo")?;
    if !driver.app().document().graph_sheet(&graph_sid).is_some_and(|g| g.spec.point_overrides.is_empty()) {
        return Err(String::from("expected Edit/Undo menu to clear point_overrides"));
    }
    click_menu_item(&mut driver, "menu.edit", "popup.menu.edit", "menu.edit.redo")?;
    if driver.app().document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len()) != 2 {
        return Err(String::from("expected Edit/Redo menu to restore point_overrides"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // Stage 5: Live upstream edit on source Column sheet keeps parity & updates scene cleanly.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{col_id_str}"), -60.0, 16)?;
    let col_scroll = format!("grid.sheet.{col_id_str}");
    enter_grid_cell(&mut driver, &col_scroll, "grid.cell.0.0.0", "12.5", 4)?;
    let _ = driver.render_frame().map_err(|e| format!("render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn wf04_human_import_reshape_and_convert_archetype() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Open ImportReshape from File menu, verify empty confirm guard, then import Tidy Long CSV -> Column 2x2.
    click_menu_item(&mut driver, "menu.file", "popup.menu.file", "menu.ui.import.reshape.open")?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::ImportReshape {
        return Err(String::from("expected ImportReshape dialog after File/Import Reshape..."));
    }
    if click_widget_center(&mut driver, &wid("modal.ok")).is_ok() {
        return Err(String::from("unexpectedly succeeded confirming empty ImportReshape modal"));
    }
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::ImportReshape {
        return Err(String::from("expected ImportReshape to stay open after empty validation error"));
    }

    click_widget_center(&mut driver, &wid("import_paste.source.tidy"))?;
    replace_textbox(&mut driver, "import_paste.input.title", "Dialog Tidy Pivot")?;
    click_widget_center(&mut driver, &wid("import_paste.target.column"))?;
    scroll_into_view(&mut driver, "import_paste.panel.source", "import_paste.input.csv", 80.0, 12)?;
    click_and_type(&mut driver, &wid("import_paste.input.csv"), "Group,Value\nCtrl,10\nCtrl,12\nDrug,20\nDrug,22")?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    let tidy_sid =
        driver.app().document().data_sheets.last().map(|s| s.id.clone()).ok_or_else(|| String::from("missing Dialog Tidy Pivot DataSheet"))?;
    {
        let ds = driver.app().document().data_sheet(&tidy_sid).ok_or_else(|| String::from("missing tidy sheet"))?;
        if ds.kind() != TableKind::Column || ds.archetype.column_count() != 2 || ds.archetype.row_count() != 2 {
            return Err(format!("expected 2x2 Column table from ImportReshape modal, got {:?}", ds.archetype));
        }
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // Stage 2: Import 96-well Plate Map via ImportReshape (load 96-well sample, blank subtraction, Grouped target).
    click_menu_item(&mut driver, "menu.file", "popup.menu.file", "menu.ui.import.reshape.open")?;
    click_widget_center(&mut driver, &wid("import_paste.source.plate96"))?;
    replace_textbox(&mut driver, "import_paste.input.title", "Plate 96 Assay")?;
    click_widget_center(&mut driver, &wid("import_paste.target.grouped"))?;
    scroll_and_click(&mut driver, "import_paste.panel.source", "import_paste.load_sample", 80.0, 12)?;
    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    scroll_into_view(&mut driver, "import_paste.panel.filter", "import.subtract_blank_mean", 80.0, 12)?;
    if !driver.app().grid.import_reshape.subtract_blank_mean {
        click_widget_center(&mut driver, &wid("import.subtract_blank_mean"))?;
    }
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    let plate_sid = driver.app().document().data_sheets.last().map(|s| s.id.clone()).ok_or_else(|| String::from("missing Plate 96 DataSheet"))?;
    if driver.app().document().data_sheet(&plate_sid).map(DataSheet::kind) != Some(TableKind::Grouped) {
        return Err(String::from("expected 96-well Plate Map import to produce TableKind::Grouped"));
    }

    // Stage 3: Select Tidy 2x2 Column table, preview & apply Convert Table Archetype... (Column -> Grouped).
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{}", tidy_sid.as_str()), -60.0, 16)?;
    click_menu_item(&mut driver, "menu.insert", "popup.menu.insert", "menu.ui.table.convertArchetype.open")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }) {
        return Err(String::from("expected ConvertArchetype modal after Insert/Convert Table Archetype..."));
    }
    click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
    if !driver.app().canvas_dialogs.convert_archetype.preview_ready {
        return Err(String::from("expected ConvertArchetype live preview_ready=true"));
    }
    if !driver.app().canvas_dialogs.convert_archetype.preview_only {
        click_widget_center(&mut driver, &wid("convert.preview_only"))?;
    }
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().data_sheet(&tidy_sid).map(DataSheet::kind) != Some(TableKind::Column) {
        return Err(String::from("preview_only ConvertArchetype should not mutate source Column table"));
    }

    // Stage 4: Re-open ConvertArchetype, disable preview_only, confirm lossy conversion if flagged, verify Grouped + live edit.
    click_menu_item(&mut driver, "menu.insert", "popup.menu.insert", "menu.ui.table.convertArchetype.open")?;
    click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
    if driver.app().canvas_dialogs.convert_archetype.preview_only {
        click_widget_center(&mut driver, &wid("convert.preview_only"))?;
    }
    if driver.app().canvas_dialogs.convert_archetype.preview_lossy && !driver.app().canvas_dialogs.convert_archetype.allow_lossy {
        click_widget_center(&mut driver, &wid("convert.allow_lossy"))?;
    }
    click_widget_center(&mut driver, &wid("modal.ok"))?;

    if driver.app().document().data_sheet(&tidy_sid).map(DataSheet::kind) != Some(TableKind::Grouped) {
        return Err(String::from("expected ConvertArchetype confirmation to convert tidy table to Grouped"));
    }

    // Stage 5: Live cell edit on converted Grouped table + full render & parity verification.
    let tidy_scroll = format!("grid.sheet.{}", tidy_sid.as_str());
    enter_grid_cell(&mut driver, &tidy_scroll, "grid.cell.0.0.0", "15.5", 2)?;
    let _ = driver.render_frame().map_err(|e| format!("render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn wf05_human_transform_pca_and_clustering_pipeline() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Create 12x4 MultipleVariables table via Welcome.
    let (mv_id_str, _, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec {
            arch_slug: "multiple_variables",
            title: "Omics Matrix",
            cols: 4,
            rows: 12,
            replicates: None,
            auto_graph: true,
            auto_info: false,
        },
    )?;

    // Stage 2: Name GeneA..GeneD headers and enter 12x4 matrix via grid cells.
    for (idx, name) in ["GeneA", "GeneB", "GeneC", "GeneD"].iter().enumerate() {
        set_grid_title(&mut driver, &format!("grid.col_title.{idx}"), name)?;
    }
    fill_numeric_rows(
        &mut driver,
        &mv_id_str,
        &[
            &["1.0", "2.0", "10.0", "11.0"],
            &["1.2", "2.1", "10.5", "11.2"],
            &["5.0", "6.0", "4.0", "3.0"],
            &["5.3", "6.2", "4.1", "3.2"],
            &["9.0", "10.0", "1.0", "2.0"],
            &["9.4", "10.3", "1.2", "2.1"],
            &["1.5", "2.4", "9.8", "10.9"],
            &["1.6", "2.6", "9.6", "10.7"],
            &["5.1", "6.1", "4.3", "3.1"],
            &["5.4", "6.3", "4.4", "3.4"],
            &["9.2", "10.2", "1.3", "2.2"],
            &["9.5", "10.5", "1.4", "2.3"],
        ],
    )?;

    // Stage 3a: Transform/PruneRows (every 2nd row -> 6 rows x 4 variables).
    let prune_id = run_analyze_entry(&mut driver, "prune", "transform.prune")?;
    let prune_sid = SheetId::try_new(&prune_id).map_err(|e| e.to_string())?;
    let pruned_rows =
        driver.app().document().result_sheet(&prune_sid).and_then(|rs| rs.analyzable_data.as_ref()).map_or(0, |d| d.archetype.row_count());
    if pruned_rows != 6 {
        return Err(format!("expected 6 pruned rows, got {pruned_rows}"));
    }

    // Stage 3b: Chain PCA directly on active Pruned ResultSheet.
    let pca_id = run_analyze_entry(&mut driver, "pca", "stats.pca")?;
    let pc1_ev = result_sheet_scalar(driver.app(), &pca_id, "PC1 eigenvalue")?;
    if !pc1_ev.is_finite() || pc1_ev <= 0.0 {
        return Err(format!("expected positive finite PC1 eigenvalue, got {pc1_ev}"));
    }

    // Stage 4: Switch PCA tab to `pca-loadings` and chain Cluster Analysis (K-means++ & Ward.D2).
    click_widget_center(&mut driver, &wid("result.tab.pca-loadings"))?;
    let clust_id = run_analyze_entry(&mut driver, "cluster", "stats.clustering")?;
    let clust_sid = SheetId::try_new(&clust_id).map_err(|e| e.to_string())?;
    {
        let clust_rs = driver.app().document().result_sheet(&clust_sid).ok_or_else(|| String::from("missing clustering ResultSheet"))?;
        if !matches!(clust_rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })) {
            return Err(format!("unexpected clustering analysis kind: {:?}", clust_rs.analysis.kind));
        }
        let has_kmeans = clust_rs.tabs.iter().any(|t| t.id == "kmeans-clusters" && t.rows.len() == 3);
        let has_ward = clust_rs.tabs.iter().any(|t| t.id == "hierarchical-dendrogram" && !t.rows.is_empty());
        if !has_kmeans || !has_ward {
            return Err(format!("expected both k-means and hierarchical dendrogram tabs, got {:?}", clust_rs.tabs));
        }
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // Stage 5: Live upstream MV cell edit (row 0 GeneA: 1.0 -> 2.5) recomputes PCA eigenvalue cleanly.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{mv_id_str}"), -60.0, 16)?;
    let mv_scroll = format!("grid.sheet.{mv_id_str}");
    enter_grid_cell(&mut driver, &mv_scroll, "grid.cell.0.0.0", "2.5", 3)?;
    let pc1_after = result_sheet_scalar(driver.app(), &pca_id, "PC1 eigenvalue")?;
    if !pc1_after.is_finite() || (pc1_after - pc1_ev).abs() < 1e-6 {
        return Err(format!("expected live edit to recompute PC1 eigenvalue from {pc1_ev}, got {pc1_after}"));
    }
    let _ = driver.render_frame().map_err(|e| format!("render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn wf06_human_prune_descriptive_survival_imputation_reactive() -> TestRes {
    let mut driver = new_default_human_driver()?;

    // Stage 1: Create 12x4 MultipleVariables clinical survival table via Welcome.
    let (surv_mv_id, _, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec {
            arch_slug: "multiple_variables",
            title: "Clinical Survival",
            cols: 4,
            rows: 12,
            replicates: None,
            auto_graph: true,
            auto_info: false,
        },
    )?;

    for (idx, name) in ["Cohort", "Time", "Status", "Biomarker"].iter().enumerate() {
        set_grid_title(&mut driver, &format!("grid.col_title.{idx}"), name)?;
    }
    // Set `Cohort` column role explicitly to Categorical via the MV role badge popup.
    click_widget_center(&mut driver, &wid("mv_formula.role_badge.0"))?;
    click_widget_center(&mut driver, &wid("mv_formula.role_opt.0.categorical"))?;
    {
        let mv_sid = SheetId::try_new(&surv_mv_id).map_err(|e| e.to_string())?;
        let ds = driver.app().document().data_sheet(&mv_sid).ok_or_else(|| String::from("missing mv-surv"))?;
        let TableArchetype::MultipleVariables(ref mv) = ds.archetype else {
            return Err(String::from("expected MultipleVariables archetype"));
        };
        if mv.variables[0].role != VariableRole::Categorical {
            return Err(format!("expected Cohort role Categorical, got {:?}", mv.variables[0].role));
        }
    }

    // Stage 2: Enter 12-row survival cohort data.
    fill_numeric_rows(
        &mut driver,
        &surv_mv_id,
        &[
            &["Ctrl", "10", "1", "100.0"],
            &["Ctrl", "12", "0", "105.0"],
            &["Ctrl", "16", "1", "120.0"],
            &["Ctrl", "18", "1", "125.0"],
            &["Ctrl", "22", "0", "140.0"],
            &["Ctrl", "25", "1", "145.0"],
            &["Drug", "20", "1", "200.0"],
            &["Drug", "24", "0", "205.0"],
            &["Drug", "30", "1", "220.0"],
            &["Drug", "32", "0", "225.0"],
            &["Drug", "40", "1", "240.0"],
            &["Drug", "44", "0", "245.0"],
        ],
    )?;

    // Stage 3a: Prune every 2nd row -> 3 Ctrl + 3 Drug (even rows 0,2,4,6,8,10).
    let prune_id = run_analyze_entry(&mut driver, "prune", "transform.prune")?;

    // Stage 3b: Chain Descriptive Statistics directly on Pruned ResultSheet.
    let desc_id = run_analyze_entry(&mut driver, "descriptive", "stats.descriptive")?;
    let desc_sid = SheetId::try_new(&desc_id).map_err(|e| e.to_string())?;
    let bio_mean_before = {
        let desc_rs = driver.app().document().result_sheet(&desc_sid).ok_or_else(|| String::from("missing descriptive ResultSheet"))?;
        if desc_rs.analysis.source_sheet_id.as_str() != prune_id {
            return Err(format!("expected descriptive parent `{prune_id}`, got `{}`", desc_rs.analysis.source_sheet_id));
        }
        let tab = desc_rs.tabs.iter().find(|t| t.id == "desc-summary").ok_or_else(|| String::from("missing desc-summary tab"))?;
        let bio_idx = tab.column_headers.iter().position(|h| h == "Biomarker").ok_or_else(|| String::from("missing Biomarker col"))?;
        let mean_row = tab.row_labels.iter().position(|r| r == "Mean").ok_or_else(|| String::from("missing Mean row"))?;
        assert_result_cell_close(tab.rows.get(mean_row).and_then(|r| r.get(bio_idx)), 170.0, 1e-6, "Biomarker Mean before edit")?;
        170.0
    };

    // Stage 3c: Switch back to Prune ResultSheet and chain Kaplan-Meier Survival analysis.
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{prune_id}"), 60.0, 16)?;
    let surv_id = run_analyze_entry(&mut driver, "survival", "stats.survival_km")?;
    let ctrl_median = result_sheet_scalar(driver.app(), &surv_id, "Median survival (Ctrl)")?;
    if !ctrl_median.is_finite() {
        return Err(String::from("expected finite Ctrl median survival"));
    }

    // Stage 4: Create second MultipleVariables table (`Impute Cohort`, 6x3 with missing cell), run Multiple Imputation.
    let (imp_mv_id, _, _) = create_table_via_welcome(
        &mut driver,
        &WelcomeTableSpec {
            arch_slug: "multiple_variables",
            title: "Impute Cohort",
            cols: 3,
            rows: 6,
            replicates: None,
            auto_graph: false,
            auto_info: false,
        },
    )?;
    for (idx, name) in ["X1", "X2", "X3"].iter().enumerate() {
        set_grid_title(&mut driver, &format!("grid.col_title.{idx}"), name)?;
    }
    fill_numeric_rows(
        &mut driver,
        &imp_mv_id,
        &[&["10", "20", "30"], &["12", "", "36"], &["14", "28", "42"], &["16", "32", "48"], &["18", "36", "54"], &["20", "40", "60"]],
    )?;
    let mi_id = run_analyze_entry(&mut driver, "imputation", "stats.multiple_imputation")?;
    let missing_cnt = result_sheet_scalar(driver.app(), &mi_id, "Total missing cells")?;
    if (missing_cnt - 1.0).abs() > 1e-9 {
        return Err(format!("expected 1 imputed missing cell, got {missing_cnt}"));
    }

    // Stage 5: Reactive recompute on source Biomarker cell edit (+60 -> mean +10).
    scroll_and_click(&mut driver, "nav.panel", &format!("nav.sheet.{surv_mv_id}"), -60.0, 20)?;
    let surv_scroll = format!("grid.sheet.{surv_mv_id}");
    enter_grid_cell(&mut driver, &surv_scroll, "grid.cell.0.3.0", "160.0", 5)?;

    let desc_rs_after = driver.app().document().result_sheet(&desc_sid).ok_or_else(|| String::from("missing desc after edit"))?;
    let tab_after = desc_rs_after.tabs.iter().find(|t| t.id == "desc-summary").ok_or_else(|| String::from("missing desc-summary after edit"))?;
    let bio_idx_after = tab_after.column_headers.iter().position(|h| h == "Biomarker").ok_or_else(|| String::from("missing Biomarker col after"))?;
    let mean_row_after = tab_after.row_labels.iter().position(|r| r == "Mean").ok_or_else(|| String::from("missing Mean row after"))?;
    assert_result_cell_close(
        tab_after.rows.get(mean_row_after).and_then(|r| r.get(bio_idx_after)),
        bio_mean_before + 10.0,
        1e-6,
        "Biomarker Mean after reactive edit",
    )?;
    let _ = driver.render_frame().map_err(|e| format!("render failed: {e}"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}
