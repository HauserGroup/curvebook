//! Relocated Wave 6 `R0.21` strict alpha branding + live UI / MCP / 16-locale `egui_shell` /
//! export metadata brand surface sweeps (moved from `xtask/src/brands.rs` and
//! `xtask/tests/r021_strict_alpha_branding.rs` so `xtask` stays decoupled from heavy app crates).

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConstantValue, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetId,
    TableKind,
};
use curvebook_engine::{Session, cmd::registry as engine_cmd_registry};
use curvebook_format::{ZipArchive, ZipLimits, save_project_bytes, save_template_bytes};
use curvebook_mcp::{GuiSessionHost, McpServerState, dispatch_json_value};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveInspector, AnalysisCategory, CommandPaletteState};
use curvebook_ui_egui::i18n;
use curvebook_ui_egui::menus::{TOOLBAR_BUTTONS, UI_COMMANDS, menu_bar_tooltips, toolbar_tooltips};
use curvebook_ui_egui::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};
use curvebook_ui_egui::{
    ActiveDialog, ClipboardBundle, EguiSurfaceCapture, EguiSurfaceScanner, UiApp, UiTreeSnapshot, ViewportMetrics, inspect_ui_tree,
};

use xtask::brands::{
    BrandHit, check_html_metadata, check_i18n_catalog_entries, check_labeled_text_runs, check_manifest_metadata, check_mcp_rpc_response,
    check_pdf_metadata, check_svg_metadata, check_text_runs, check_text_runs_strict, check_ui_text_run, check_window_title,
};

fn import_cmd_id() -> String {
    join(&["table.", "im", "port", "Pr", "ism"])
}

/// Scan all registered 16-locale `i18n::CATALOG` translations and locale display names.
#[must_use]
pub fn check_i18n_catalog() -> Vec<BrandHit> {
    let mut entries: Vec<(&str, &str, &str)> = Vec::with_capacity(i18n::CATALOG.len() * i18n::LOCALES.len() + i18n::LOCALES.len());
    for entry in i18n::CATALOG {
        for (idx, &locale) in i18n::LOCALES.iter().enumerate() {
            if let Some(&text) = entry.translations.get(idx) {
                entries.push((entry.key, locale, text));
            }
        }
    }
    for &locale in i18n::LOCALES {
        entries.push(("locale_display_name", locale, i18n::locale_display_name(locale)));
    }
    check_i18n_catalog_entries(entries)
}

fn collect_snapshot_strings(snap: &UiTreeSnapshot, context: &str, hits: &mut Vec<BrandHit>) {
    let mut runs: Vec<(String, String)> = Vec::new();
    if let Some(banner) = &snap.status_banner {
        runs.push((format!("{context}:status_banner"), banner.clone()));
    }
    if let Some(target) = &snap.context_menu_target {
        runs.push((format!("{context}:context_menu_target"), target.clone()));
    }
    if let Some(diag) = &snap.result_diagnostic_banner {
        runs.push((format!("{context}:diag.summary"), diag.summary.clone()));
        runs.push((format!("{context}:diag.limitation"), diag.limitation_note.clone()));
        if let Some(metric) = &diag.diagnostic_metric {
            runs.push((format!("{context}:diag.metric"), metric.clone()));
        }
    }
    if let Some(menus) = &snap.menus {
        for m in menus {
            runs.push((format!("{context}:menu:{}", m.menu_key), m.title.clone()));
            for n in &m.items {
                push_widget_node_runs(context, n, &mut runs);
            }
        }
    }
    for (group, nodes) in [
        ("toolbar", snap.toolbar.as_deref()),
        ("tabs", snap.tabs.as_deref()),
        ("canvas", snap.canvas_elements.as_deref()),
        ("grid", snap.grid_cells.as_deref()),
    ] {
        if let Some(slice) = nodes {
            for n in slice {
                push_widget_node_runs(&format!("{context}:{group}"), n, &mut runs);
            }
        }
    }
    if let Some(nav) = &snap.navigator_sheets {
        for entry in nav {
            let nav_ctx = format!("{context}:nav:{}", entry.sheet_id);
            hits.extend(check_text_runs_strict(&nav_ctx, [&entry.title, &entry.label]));
        }
    }
    if let Some(dlg) = &snap.dialog {
        runs.push((format!("{context}:dialog:{}:title", dlg.dialog_kind), dlg.title.clone()));
        if let Some(stage) = &dlg.stage {
            runs.push((format!("{context}:dialog:{}:stage", dlg.dialog_kind), stage.clone()));
        }
        for n in dlg.tabs.iter().chain(&dlg.controls).chain(&dlg.buttons) {
            push_widget_node_runs(&format!("{context}:dialog:{}", dlg.dialog_kind), n, &mut runs);
        }
    }
    for (label, text) in &runs {
        let allow_it_stat_word = label.contains(":it:");
        check_ui_text_run(label, 1, text, allow_it_stat_word, false, hits);
    }
}

fn check_labeled_ui_pairs<'a>(runs: impl IntoIterator<Item = (&'a str, &'a str)>, allow_it_stat_word: bool, hits: &mut Vec<BrandHit>) {
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        check_ui_text_run(label, idx + 1, text, allow_it_stat_word, false, hits);
    }
}

fn check_ui_strings_with_locale<'a>(source: &str, runs: impl IntoIterator<Item = &'a str>, allow_it_stat_word: bool, hits: &mut Vec<BrandHit>) {
    for (idx, text) in runs.into_iter().enumerate() {
        check_ui_text_run(source, idx + 1, text, allow_it_stat_word, false, hits);
    }
}

fn push_widget_node_runs(prefix: &str, node: &curvebook_ui_egui::inspect::UiWidgetNode, out: &mut Vec<(String, String)>) {
    out.push((format!("{prefix}:{}", node.widget_id), node.label.clone()));
    if let Some(reason) = &node.disabled_reason {
        out.push((format!("{prefix}:{}:disabled_reason", node.widget_id), reason.clone()));
    }
    if let Some(serde_json::Value::String(v)) = &node.value {
        out.push((format!("{prefix}:{}:value", node.widget_id), v.clone()));
    }
}

fn build_brands_sweep_document() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("CurveBook Brand Sweep").map_err(|e| format!("project init: {e}"))?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let id = format!("data-{}", idx + 1);
        let ds = instantiate_sample_fixture(fixture.id, id).map_err(|e| format!("fixture {}: {e}", fixture.id))?;
        doc.add_data_sheet(ds).map_err(|e| format!("add fixture {}: {e}", fixture.id))?;
    }
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Constants").map_err(|e| format!("info sheet: {e}"))?;
    info.constants
        .push(curvebook_doc::InfoConstant::new("ControlEC50", ConstantValue::Number(12.5), Some(String::from("nM"))).map_err(|e| e.to_string())?);
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let res = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(res).map_err(|e| e.to_string())?;

    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        g1.title = String::from("Dose-Response Inhibition");
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;

        let mut g2 = GraphSheet::new_uninitialized_for_data("graph-2", &ds_xy).map_err(|e| e.to_string())?;
        g2.initialized = true;
        g2.title = String::from("Secondary Curve Panel");
        doc.add_graph_sheet(g2).map_err(|e| e.to_string())?;
    }

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(18.0, 18.0, 220.0, 160.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    let layout = LayoutSheet::new("layout-1", "Publication Panel Layout", layout_spec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(layout).map_err(|e| e.to_string())?;
    Ok(doc)
}

fn check_tooltips_and_catalogs(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    for &btn in TOOLBAR_BUTTONS {
        hits.extend(check_labeled_text_runs([("TOOLBAR_BUTTONS.id", btn.id), ("TOOLBAR_BUTTONS.label_key", btn.label_key)]));
    }
    for &locale in i18n::LOCALES {
        app.locale = locale.to_owned();
        let is_it = locale == "it";
        for section in menu_bar_tooltips(app) {
            let ctx = format!("menu:{locale}:{}", section.menu_key);
            check_ui_strings_with_locale(&ctx, [section.title], is_it, hits);
            for tip in section.items {
                let p_label = format!("{ctx}:{}:label", tip.command_id);
                let p_loc = format!("{ctx}:{}:localized", tip.command_id);
                let p_sum = format!("{ctx}:{}:summary", tip.command_id);
                let p_beg = format!("{ctx}:{}:beginner", tip.command_id);
                let p_path = format!("{ctx}:{}:menu_path", tip.command_id);
                check_labeled_ui_pairs(
                    [
                        (p_label.as_str(), tip.label),
                        (p_loc.as_str(), tip.localized_label),
                        (p_sum.as_str(), tip.summary),
                        (p_beg.as_str(), tip.beginner),
                        (p_path.as_str(), tip.menu_path),
                    ],
                    is_it,
                    hits,
                );
            }
        }
        for tb in toolbar_tooltips(app) {
            let ctx = format!("toolbar:{locale}:{}", tb.button.id);
            check_ui_strings_with_locale(&ctx, [tb.caption, tb.tooltip.localized_label, tb.tooltip.summary, tb.tooltip.beginner], is_it, hits);
        }
    }
    app.locale = String::from("en");

    for cat in AnalysisCategory::ALL {
        hits.extend(check_text_runs("AnalysisCategory", [cat.label()]));
    }
    for item in ANALYSIS_CATALOG {
        hits.extend(check_labeled_text_runs([
            ("ANALYSIS_CATALOG.label", item.label),
            ("ANALYSIS_CATALOG.summary", item.summary),
            ("ANALYSIS_CATALOG.keywords", item.keywords),
        ]));
    }
    for kind in TableKind::ALL {
        let draft = TableCreationDraft::for_kind(kind);
        hits.extend(check_text_runs("TableKind", [kind.label(), draft.title.as_str()]));
    }
    for fixture in SAMPLE_FIXTURES {
        hits.extend(check_labeled_text_runs([("SAMPLE_FIXTURES.title", fixture.title), ("SAMPLE_FIXTURES.summary", fixture.summary)]));
    }
    let palette = CommandPaletteState::default();
    for entry in palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts) {
        hits.extend(check_labeled_text_runs([
            ("CommandPalette.label", entry.label),
            ("CommandPalette.menu_path", entry.menu_path),
            ("CommandPalette.summary", entry.summary),
            ("CommandPalette.beginner", entry.beginner),
        ]));
    }
}

fn check_all_dialogs_and_inspectors(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    let data_id = SheetId::new("data-1");
    let res_id = SheetId::new("res-1");
    let graph_id = SheetId::new("graph-1");
    let layout_id = SheetId::new("layout-1");

    for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
        app.nav.welcome.open = true;
        app.nav.welcome.mode = mode;
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("welcome:{mode:?}"), hits);
    }
    app.nav.welcome.open = false;

    for inspector in [
        ActiveInspector::None,
        ActiveInspector::FormatGraph,
        ActiveInspector::FormatAxes,
        ActiveInspector::PairwiseComparisons,
        ActiveInspector::BubblePlot,
        ActiveInspector::PageLayout,
        ActiveInspector::ResultDetails,
    ] {
        app.canvas_dialogs.active_inspector = inspector;
        hits.extend(check_text_runs("ActiveInspector", [inspector.label()]));
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("inspector:{inspector:?}"), hits);
    }

    let dialogs = [
        ActiveDialog::FormatAxes { sheet_id: graph_id.clone() },
        ActiveDialog::AnalyzeSheet { source_sheet_id: data_id.clone() },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: data_id.clone(),
            existing_result_ids: vec![res_id.clone()],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: res_id.clone() },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_id.clone() },
        ActiveDialog::ResultViewer { result_sheet_id: res_id.clone() },
        ActiveDialog::ConstantHookPicker { target_sheet_id: res_id.clone() },
        ActiveDialog::ChangeGraphType { graph_sheet_id: graph_id.clone() },
        ActiveDialog::FormatGraph { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_id.clone() },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_id.clone() },
        ActiveDialog::ApplyStyleFrom { target_graph_id: graph_id.clone() },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: layout_id.clone() },
        ActiveDialog::ExportFigure { sheet_id: graph_id.clone() },
        ActiveDialog::ConvertArchetype { sheet_id: data_id.clone() },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: data_id.clone() },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(layout_id.clone()) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: graph_id.clone() },
        ActiveDialog::CreateGraph { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: layout_id.clone() },
        ActiveDialog::FormatEmbeddedTable { sheet_id: graph_id.clone(), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: graph_id.clone(), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::Checkpoints,
        ActiveDialog::About,
    ];
    for dlg in dialogs {
        app.canvas_dialogs.active_dialog = dlg.clone();
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, "dialog", hits);
    }
    app.canvas_dialogs.active_dialog = ActiveDialog::None;
}

fn check_export_metadata_sweep(doc: &ProjectDocument, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let graph = doc.graph_sheet(&SheetId::new("graph-1")).ok_or_else(|| String::from("sweep graph-1 missing"))?;
    let mut scene = compile_graph(graph, doc).map_err(|e| format!("compile sweep graph: {e}"))?;
    scene.title = String::from("Dose-Response Inhibition");

    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| format!("sweep render_svg: {e}"))?;
    hits.extend(check_svg_metadata("export:render_svg", &svg));

    let pdf_ascii = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| format!("sweep render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf", &pdf_ascii));

    let mut unicode_scene = scene.clone();
    unicode_scene.title = String::from("Étude Dose–Réponse — αβγ");
    let pdf_unicode = render_pdf(&unicode_scene, &PdfExportOptions::default()).map_err(|e| format!("sweep unicode render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf:unicode", &pdf_unicode));

    let bundle = ClipboardBundle::from_scene(&scene, 144)?;
    hits.extend(check_svg_metadata("clipboard:svg", &bundle.svg));
    hits.extend(check_pdf_metadata("clipboard:pdf", &bundle.pdf_bytes));
    hits.extend(check_html_metadata("clipboard:html", &bundle.html));

    for (label, bytes_res) in
        [("archive:project:manifest.json", save_project_bytes(doc)), ("archive:template:manifest.json", save_template_bytes(doc))]
    {
        let bytes = bytes_res.map_err(|e| format!("{label}: {e}"))?;
        let zip = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).map_err(|e| format!("{label} zip: {e}"))?;
        let manifest = zip.get_utf8("manifest.json").map_err(|e| format!("{label}: {e}"))?.ok_or_else(|| format!("{label} missing manifest.json"))?;
        hits.extend(check_manifest_metadata(label, manifest));
    }

    hits.extend(check_window_title("desktop:window_title:untitled", "Untitled — CurveBook"));
    hits.extend(check_window_title("desktop:window_title:dirty", "study.curvebook * — CurveBook"));
    Ok(())
}

/// Sweep in-process UI strings across 16-locale `i18n`, tooltips, all 8 table archetypes,
/// all 26 `ActiveDialog` states, all 7 `ActiveInspector` tabs, live 16-locale `egui_shell` renders,
/// live MCP headless & `--gui-only` JSON-RPC surfaces, `CommandPalette`, engine/UI command catalogs,
/// and SVG/PDF/HTML/manifest/window export metadata.
pub fn check_ui_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = check_i18n_catalog();

    let mut empty_app = UiApp::new();
    let empty_snap = inspect_ui_tree(&empty_app, None, true);
    collect_snapshot_strings(&empty_snap, "empty_ui", &mut hits);
    check_tooltips_and_catalogs(&mut empty_app, &mut hits);
    check_engine_and_ui_commands(&mut hits);

    let doc = build_brands_sweep_document()?;
    check_export_metadata_sweep(&doc, &mut hits)?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("recalculate sweep session: {e}"))?;
    let mut app = UiApp::from_session(session);
    check_all_dialogs_and_inspectors(&mut app, &mut hits);
    hits.extend(check_live_egui_shell_surfaces()?);
    hits.extend(check_mcp_surfaces()?);

    Ok(hits)
}

struct BrandsGuiSessionHost {
    app: UiApp,
}

impl BrandsGuiSessionHost {
    fn new(mut app: UiApp, gui_only: bool) -> Self {
        app.set_gui_only(gui_only);
        Self { app }
    }
}

impl GuiSessionHost for BrandsGuiSessionHost {
    fn call_gui(&mut self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        if method == "ui.screenshot"
            && let Some(obj) = params.as_object_mut()
            && matches!(obj.get("target").and_then(serde_json::Value::as_str), Some("workspace" | "viewport"))
        {
            obj.insert(String::from("target"), serde_json::Value::String(String::from("full_window")));
        }
        curvebook_ui_egui::dispatch_gui_action(&mut self.app, method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

fn send_mcp_rpc(state: &mut McpServerState, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    dispatch_json_value(req, state).ok_or_else(|| format!("MCP `{method}` produced no JSON-RPC response"))
}

fn ensure_mcp_ok<'a>(label: &str, resp: &'a serde_json::Value) -> Result<&'a serde_json::Value, String> {
    if let Some(err) = resp.get("error") {
        return Err(format!("MCP `{label}` returned unexpected JSON-RPC error: {err}"));
    }
    let result = resp.get("result").ok_or_else(|| format!("MCP `{label}` missing `result` field"))?;
    if result.get("isError").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(format!("MCP `{label}` returned tool execution error: {result}"));
    }
    Ok(result)
}

/// Drive actual in-memory `curvebook-mcp` JSON-RPC 2.0 sessions (both default headless mode and
/// strict `--gui-only` mode) over a populated 5-sheet `ProjectDocument`, scanning `initialize`,
/// `tools/list` (core + all progressive `CommandGroup`s + `gui`), `resources/list` + `resources/read`,
/// `prompts/list` + `prompts/get`, and `tools/call` (`doc_inspect`, `render_preview`, `export`,
/// `ui_inspect`, `ui_screenshot`).
pub fn check_mcp_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let doc = build_brands_sweep_document()?;
    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("MCP sweep recalculate: {e}"))?;

    check_mcp_headless_mode(session.clone(), &mut hits)?;
    check_mcp_gui_only_mode(session, &mut hits)?;
    Ok(hits)
}

fn check_mcp_headless_mode(session: Session, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let mut state = McpServerState::from_session(session);

    let init = send_mcp_rpc(
        &mut state,
        "initialize",
        serde_json::json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "xtask-brands", "version": "0.1.0" } }),
    )?;
    ensure_mcp_ok("headless:initialize", &init)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:initialize", "initialize", None, &init));

    let core_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    ensure_mcp_ok("headless:tools/list:core", &core_tools)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:tools/list:core", "tools/list", None, &core_tools));

    let toolsets = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "list_toolsets", "arguments": {} }))?;
    ensure_mcp_ok("headless:list_toolsets", &toolsets)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:list_toolsets", "tools/call", Some("list_toolsets"), &toolsets));

    let enable = send_mcp_rpc(
        &mut state,
        "tools/call",
        serde_json::json!({ "name": "enable_toolset", "arguments": { "group": "all", "groups": ["gui"], "enabled": true } }),
    )?;
    ensure_mcp_ok("headless:enable_toolset", &enable)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:enable_toolset", "tools/call", Some("enable_toolset"), &enable));
    for notif in state.take_pending_notifications() {
        hits.extend(check_mcp_rpc_response("mcp:headless:notif", "notification", None, &notif));
    }

    let all_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    let all_res = ensure_mcp_ok("headless:tools/list:all", &all_tools)?;
    let tool_count = all_res.get("tools").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    if tool_count < 110 {
        return Err(format!("expected >= 110 enabled MCP tools in sweep, got {tool_count}"));
    }
    hits.extend(check_mcp_rpc_response("mcp:headless:tools/list:all", "tools/list", None, &all_tools));

    let cmd_list = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "command_list", "arguments": { "include_ui": true } }))?;
    ensure_mcp_ok("headless:command_list", &cmd_list)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:command_list", "tools/call", Some("command_list"), &cmd_list));

    check_mcp_resources_and_prompts(&mut state, "mcp:headless", false, hits)?;
    check_mcp_headless_previews_and_inspect(&mut state, hits)?;
    Ok(())
}

fn check_mcp_headless_previews_and_inspect(state: &mut McpServerState, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let overview = send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    ensure_mcp_ok("headless:doc_inspect", &overview)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:doc_inspect:root", "tools/call", Some("doc_inspect"), &overview));

    for sid in ["data-1", "info-1", "res-1", "graph-1", "layout-1"] {
        let sheet_ins =
            send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": { "sheet_id": sid, "include_cells": true } }))?;
        ensure_mcp_ok("headless:doc_inspect:sheet", &sheet_ins)?;
        hits.extend(check_mcp_rpc_response(&format!("mcp:headless:doc_inspect:{sid}"), "tools/call", Some("doc_inspect"), &sheet_ins));
    }

    for sid in ["graph-1", "layout-1"] {
        let preview = send_mcp_rpc(
            state,
            "tools/call",
            serde_json::json!({ "name": "render_preview", "arguments": { "sheet_id": sid, "width_px": 320, "height_px": 220, "dpi": 144 } }),
        )?;
        ensure_mcp_ok("headless:render_preview", &preview)?;
        hits.extend(check_mcp_rpc_response(&format!("mcp:headless:render_preview:{sid}"), "tools/call", Some("render_preview"), &preview));
    }

    let svg_export =
        send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "export", "arguments": { "format": "svg", "sheet_id": "graph-1" } }))?;
    ensure_mcp_ok("headless:export:svg", &svg_export)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:export:svg", "tools/call", Some("export"), &svg_export));
    Ok(())
}

fn check_mcp_gui_only_mode(session: Session, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let mut app = UiApp::from_session(session);
    app.sync_views();
    let mut state = McpServerState::with_gui_host(Box::new(BrandsGuiSessionHost::new(app, true)));

    let init = send_mcp_rpc(
        &mut state,
        "initialize",
        serde_json::json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "xtask-brands-gui", "version": "0.1.0" } }),
    )?;
    ensure_mcp_ok("gui_only:initialize", &init)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:initialize", "initialize", None, &init));

    let gui_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    let gui_res = ensure_mcp_ok("gui_only:tools/list", &gui_tools)?;
    let gui_count = gui_res.get("tools").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    if gui_count != 8 {
        return Err(format!("expected 8 --gui-only MCP tools, got {gui_count}"));
    }
    hits.extend(check_mcp_rpc_response("mcp:gui_only:tools/list", "tools/list", None, &gui_tools));

    check_mcp_resources_and_prompts(&mut state, "mcp:gui_only", true, hits)?;

    let ui_ins = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "ui_inspect", "arguments": { "scope": "all" } }))?;
    ensure_mcp_ok("gui_only:ui_inspect", &ui_ins)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:ui_inspect", "tools/call", Some("ui_inspect"), &ui_ins));

    let ui_shot = send_mcp_rpc(
        &mut state,
        "tools/call",
        serde_json::json!({ "name": "ui_screenshot", "arguments": { "target": "workspace", "width_px": 320, "height_px": 200, "dpi": 144 } }),
    )?;
    ensure_mcp_ok("gui_only:ui_screenshot", &ui_shot)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:ui_screenshot", "tools/call", Some("ui_screenshot"), &ui_shot));

    let blocked = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:blocked_doc_inspect", "tools/call", Some("doc_inspect"), &blocked));
    Ok(())
}

fn check_mcp_resources_and_prompts(state: &mut McpServerState, prefix: &str, gui_only: bool, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let res_list = send_mcp_rpc(state, "resources/list", serde_json::json!({}))?;
    let res_ok = ensure_mcp_ok(&format!("{prefix}:resources/list"), &res_list)?;
    hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/list"), "resources/list", None, &res_list));

    let uris: Vec<String> = res_ok
        .get("resources")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("uri").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if !gui_only && (!uris.iter().any(|u| u.starts_with("sheet://")) || !uris.iter().any(|u| u.starts_with("result://"))) {
        return Err(String::from("expected both sheet:// and result:// resources in headless MCP sweep"));
    }
    if gui_only {
        let blocked_read = send_mcp_rpc(state, "resources/read", serde_json::json!({ "uri": "sheet://data-1" }))?;
        hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/read:blocked"), "resources/read", None, &blocked_read));
    } else {
        for uri in &uris {
            let read_resp = send_mcp_rpc(state, "resources/read", serde_json::json!({ "uri": uri }))?;
            ensure_mcp_ok(&format!("{prefix}:resources/read:{uri}"), &read_resp)?;
            hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/read:{uri}"), "resources/read", None, &read_resp));
        }
    }

    let prompts_list = send_mcp_rpc(state, "prompts/list", serde_json::json!({}))?;
    let prompts_ok = ensure_mcp_ok(&format!("{prefix}:prompts/list"), &prompts_list)?;
    hits.extend(check_mcp_rpc_response(&format!("{prefix}:prompts/list"), "prompts/list", None, &prompts_list));

    let names: Vec<String> = prompts_ok
        .get("prompts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if names.is_empty() {
        return Err(format!("{prefix}: expected non-empty prompts/list"));
    }
    for name in &names {
        for (variant, args) in prompt_arg_variants(name) {
            let prompt_resp = send_mcp_rpc(state, "prompts/get", serde_json::json!({ "name": name, "arguments": args }))?;
            ensure_mcp_ok(&format!("{prefix}:prompts/get:{name}:{variant}"), &prompt_resp)?;
            hits.extend(check_mcp_rpc_response(&format!("{prefix}:prompts/get:{name}:{variant}"), "prompts/get", None, &prompt_resp));
        }
    }
    Ok(())
}

fn prompt_arg_variants(name: &str) -> Vec<(&'static str, serde_json::Value)> {
    match name {
        "dose_response_curve_fit" => {
            vec![("default", serde_json::json!({})), ("antagonist", serde_json::json!({ "sheet_id": "data-1", "response_type": "antagonist" }))]
        }
        "choose_two_group_test" => vec![
            ("default", serde_json::json!({})),
            ("paired_param", serde_json::json!({ "paired": "true", "parametric": "true" })),
            ("paired_nonparam", serde_json::json!({ "paired": "true", "parametric": "false" })),
            ("unpaired_param", serde_json::json!({ "paired": "false", "parametric": "true" })),
            ("unpaired_nonparam", serde_json::json!({ "paired": "false", "parametric": "false" })),
        ],
        "publication_figure_checklist" => {
            vec![("default", serde_json::json!({})), ("svg", serde_json::json!({ "graph_sheet_id": "graph-1", "target_format": "svg" }))]
        }
        _ => vec![("default", serde_json::json!({}))],
    }
}

fn check_engine_and_ui_commands(hits: &mut Vec<BrandHit>) {
    for spec in engine_cmd_registry().specs() {
        let is_import_ext_cmd = spec.id == import_cmd_id();
        check_ui_text_run(&format!("engine_cmd:{}:id", spec.id), 1, spec.id, false, is_import_ext_cmd, hits);
        for (field, val) in [("label", spec.label), ("menu_path", spec.menu_path), ("summary", spec.summary), ("beginner", spec.beginner)] {
            check_ui_text_run(&format!("engine_cmd:{}:{field}", spec.id), 1, val, false, false, hits);
        }
        if let Ok(schema_str) = serde_json::to_string(&(spec.input_schema)()) {
            check_ui_text_run(&format!("engine_cmd:{}:input_schema", spec.id), 1, &schema_str, false, false, hits);
        }
    }
    for cmd in UI_COMMANDS {
        let is_import_ext_cmd = cmd.id == import_cmd_id();
        check_ui_text_run(&format!("ui_cmd:{}:id", cmd.id), 1, cmd.id, false, is_import_ext_cmd, hits);
        for (field, val) in [("label", cmd.label), ("params_doc", cmd.params_doc), ("summary", cmd.summary), ("beginner", cmd.beginner)] {
            check_ui_text_run(&format!("ui_cmd:{}:{field}", cmd.id), 1, val, false, false, hits);
        }
    }
}

/// Sweep live `egui_shell` painted text runs, `FrameInspectNodeMeta`, and AccessKit strings
/// across ALL `16` `LOCALES`, all `5` sheet kinds, all `4` `WelcomeMode`s, all `7` `ActiveInspector`
/// tabs, all `26` `ActiveDialog` variants, open dropdown menus, toolbar overflow, split view, and
/// portfolio gallery.
pub fn check_live_egui_shell_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let desktop_vp = ViewportMetrics::new(1440.0, 900.0, 1.0);
    let narrow_vp = ViewportMetrics::new(640.0, 560.0, 1.0);

    let mut empty_app = UiApp::new();
    let doc = build_brands_sweep_document()?;
    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("egui_shell sweep recalculate: {e}"))?;
    let mut app = UiApp::from_session(session);
    app.sync_views();

    let sheet_ids = ["data-1", "info-1", "res-1", "graph-1", "layout-1"];
    let menu_keys: Vec<&'static str> = menu_bar_tooltips(&app).into_iter().map(|s| s.menu_key).collect();
    let dialogs = sweep_dialog_states();

    for &locale in i18n::LOCALES {
        let is_it = locale == "it";
        let scanner = EguiSurfaceScanner::new();
        empty_app.set_locale(locale);
        for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
            empty_app.nav.welcome.open = true;
            empty_app.nav.welcome.mode = mode;
            let cap = scanner.scan(&mut empty_app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:welcome:{mode:?}"), is_it, false, &cap, &mut hits);
        }
        empty_app.nav.welcome.open = false;

        app.set_locale(locale);
        app.nav.welcome.open = false;
        app.canvas_dialogs.active_dialog = ActiveDialog::None;
        app.canvas_dialogs.active_inspector = ActiveInspector::None;
        app.interaction.open_menu = None;
        app.egui_views.top_chrome.overflow_menu_open = false;
        app.split.enabled = false;
        app.nav.gallery.open = false;

        for sid in sheet_ids {
            app.execute_command("sheet.select", serde_json::json!({ "sheet_id": sid })).map_err(|e| format!("select sheet `{sid}`: {e}"))?;
            app.sync_views();
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:sheet:{sid}"), is_it, false, &cap, &mut hits);
        }
        app.execute_command("sheet.select", serde_json::json!({ "sheet_id": "graph-1" })).map_err(|e| format!("select graph-1: {e}"))?;
        app.sync_views();

        for inspector in [
            ActiveInspector::None,
            ActiveInspector::FormatGraph,
            ActiveInspector::FormatAxes,
            ActiveInspector::PairwiseComparisons,
            ActiveInspector::BubblePlot,
            ActiveInspector::PageLayout,
            ActiveInspector::ResultDetails,
        ] {
            app.canvas_dialogs.active_inspector = inspector;
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:inspector:{inspector:?}"), is_it, false, &cap, &mut hits);
        }
        app.canvas_dialogs.active_inspector = ActiveInspector::None;

        for dlg in &dialogs {
            app.canvas_dialogs.active_dialog = dlg.clone();
            if matches!(dlg, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.command_palette.open();
                let cap_default = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog:CommandPalette:default"), is_it, false, &cap_default, &mut hits);
                app.canvas_dialogs.command_palette.query = String::from("import");
                let cap_import = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog:CommandPalette:import"), is_it, true, &cap_import, &mut hits);
                app.canvas_dialogs.command_palette.open();
            } else {
                let cap = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog"), is_it, false, &cap, &mut hits);
            }
        }
        app.canvas_dialogs.active_dialog = ActiveDialog::None;

        for key in &menu_keys {
            app.interaction.open_menu = Some((*key).to_owned());
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:menu:{key}"), is_it, false, &cap, &mut hits);
        }
        app.interaction.open_menu = None;

        app.egui_views.top_chrome.overflow_menu_open = true;
        let overflow_cap = scanner.scan(&mut app, narrow_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:toolbar_overflow"), is_it, false, &overflow_cap, &mut hits);
        app.egui_views.top_chrome.overflow_menu_open = false;

        app.split.enabled = true;
        app.split.primary_sheet_id = Some(SheetId::new("data-1"));
        app.split.secondary_sheet_id = Some(SheetId::new("graph-1"));
        let split_cap = scanner.scan(&mut app, desktop_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:split_workspace"), is_it, false, &split_cap, &mut hits);
        app.split.enabled = false;

        app.nav.gallery.open = true;
        let gallery_doc = app.document().clone();
        app.nav.sync_gallery(&gallery_doc);
        let gallery_cap = scanner.scan(&mut app, desktop_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:gallery"), is_it, false, &gallery_cap, &mut hits);
        app.nav.gallery.open = false;
    }

    Ok(hits)
}

fn sweep_dialog_states() -> Vec<ActiveDialog> {
    let data_id = SheetId::new("data-1");
    let res_id = SheetId::new("res-1");
    let graph_id = SheetId::new("graph-1");
    let layout_id = SheetId::new("layout-1");
    vec![
        ActiveDialog::FormatAxes { sheet_id: graph_id.clone() },
        ActiveDialog::AnalyzeSheet { source_sheet_id: data_id.clone() },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: data_id.clone(),
            existing_result_ids: vec![res_id.clone()],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: res_id.clone() },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_id.clone() },
        ActiveDialog::ResultViewer { result_sheet_id: res_id.clone() },
        ActiveDialog::ConstantHookPicker { target_sheet_id: res_id.clone() },
        ActiveDialog::ChangeGraphType { graph_sheet_id: graph_id.clone() },
        ActiveDialog::FormatGraph { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_id.clone() },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_id.clone() },
        ActiveDialog::ApplyStyleFrom { target_graph_id: graph_id.clone() },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: layout_id.clone() },
        ActiveDialog::ExportFigure { sheet_id: graph_id.clone() },
        ActiveDialog::ConvertArchetype { sheet_id: data_id.clone() },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: data_id.clone() },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(layout_id.clone()) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: graph_id },
        ActiveDialog::CreateGraph { source_sheet_id: Some(data_id) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: layout_id },
        ActiveDialog::FormatEmbeddedTable { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::Checkpoints,
        ActiveDialog::About,
    ]
}

fn collect_egui_surface_hits(
    context: &str,
    allow_it_stat_word: bool,
    allow_palette_cmd_id: bool,
    cap: &EguiSurfaceCapture,
    hits: &mut Vec<BrandHit>,
) {
    for (idx, (owner, text)) in cap.text_runs.iter().enumerate() {
        let is_palette_cmd = allow_palette_cmd_id && (owner == "palette.status" || owner.starts_with("palette.item."));
        let run_ctx = format!("{context}:text_run:{owner}");
        check_ui_text_run(&run_ctx, idx + 1, text, allow_it_stat_word, is_palette_cmd, hits);
    }
    for (idx, entry) in cap.a11y_entries.iter().enumerate() {
        let a11y_ctx = format!("{context}:a11y:{}", entry.widget_id);
        for txt in [Some(entry.label.as_str()), entry.value_text.as_deref(), entry.disabled_reason.as_deref(), entry.full_label.as_deref()]
            .into_iter()
            .flatten()
        {
            check_ui_text_run(&a11y_ctx, idx + 1, txt, allow_it_stat_word, false, hits);
        }
        if let Some(cmd) = &entry.command_id
            && cmd != &import_cmd_id()
        {
            hits.extend(check_text_runs_strict(&format!("{a11y_ctx}:command_id"), [cmd.as_str()]));
        }
    }
    for (idx, ak) in cap.accesskit_labels.iter().enumerate() {
        check_ui_text_run(&format!("{context}:accesskit"), idx + 1, ak, allow_it_stat_word, false, hits);
    }
}

#[test]
fn live_16_locale_i18n_catalog_clean() {
    assert_eq!(check_i18n_catalog(), Vec::new());
}

#[test]
fn live_ui_mcp_egui_and_export_brand_surface_sweep() -> Result<(), String> {
    let hits = check_ui_surfaces()?;
    assert!(hits.is_empty(), "live surface brand hits: {hits:?}");
    Ok(())
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn join(parts: &[&str]) -> String {
    parts.concat()
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
        }
    }
    !crc
}

fn banned_probe_tokens() -> Vec<String> {
    vec![
        join(&["gr", "aph", "pad"]),
        join(&["dot", "mat", "ics"]),
        join(&["pr", "ism"]),
        join(&["in", "st", "at"]),
        join(&["stat", "ma", "te"]),
        join(&["ori", "gin", "lab"]),
        join(&["ori", "gin", "pro"]),
        join(&["sig", "ma", "plot"]),
        join(&["min", "it", "ab"]),
        join(&["sys", "ta", "t"]),
        join(&["stat", "is", "ti", "ca"]),
        join(&["xl", "st", "at"]),
        join(&["sp", "ss"]),
        join(&["j", "m", "p"]),
        join(&["art", "cr", "aft"]),
        join(&["sto", "ry", "told"]),
    ]
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    let lower = haystack.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let nlen = needle.len();
    let mut start = 0usize;
    while let Some(rel) = lower.get(start..).and_then(|s| s.find(needle)) {
        let pos = start + rel;
        let end = pos + nlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn assert_clean_ui_string(ctx: &str, text: &str, locale: &str, allow_palette_cmd_id: bool) -> Result<(), String> {
    let normalized_text = if allow_palette_cmd_id {
        let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
        text.replace(&cmd_id, "table.import_ext")
    } else {
        text.to_owned()
    };
    let check_str = if locale == "it" {
        let ital_word = join(&["stat", "is", "ti", "ca"]);
        let bytes = normalized_text.as_bytes();
        let wlen = ital_word.len();
        let mut masked = normalized_text.clone();
        let mut search = 0usize;
        while let Some(rel) = masked.get(search..).and_then(|s| s.find(&ital_word)) {
            let pos = search + rel;
            let end = pos + wlen;
            let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                masked.replace_range(pos..end, "statist___");
            }
            search = pos + 1;
        }
        masked
    } else {
        normalized_text
    };
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&check_str, &probe) {
            return Err(format!("{ctx}: UI surface leaked `{probe}` in `{text}`"));
        }
    }
    Ok(())
}

fn assert_clean_egui_capture(ctx: &str, cap: &EguiSurfaceCapture, locale: &str, allow_palette_cmd_id: bool) -> Result<(), String> {
    for (owner, text) in &cap.text_runs {
        let is_palette_cmd = allow_palette_cmd_id && (owner == "palette.status" || owner.starts_with("palette.item."));
        let allow = is_palette_cmd || text.contains("*.prism");
        assert_clean_ui_string(&format!("{ctx}:{owner}"), text, locale, allow)?;
    }
    for entry in &cap.a11y_entries {
        for txt in [Some(entry.label.as_str()), entry.value_text.as_deref(), entry.disabled_reason.as_deref(), entry.full_label.as_deref()]
            .into_iter()
            .flatten()
        {
            assert_clean_ui_string(&format!("{ctx}:a11y:{}", entry.widget_id), txt, locale, txt.contains("*.prism"))?;
        }
    }
    for ak in &cap.accesskit_labels {
        assert_clean_ui_string(&format!("{ctx}:accesskit"), ak, locale, ak.contains("*.prism"))?;
    }
    Ok(())
}

#[test]
fn r021_live_egui_shell_and_a11y_clean_across_all_16_locales_and_5_sheets() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R021 Live Egui Shell Sweep").map_err(|e| e.to_string())?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let ds = instantiate_sample_fixture(fixture.id, format!("data-{}", idx + 1)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    let info = InfoSheet::new_preseeded("info-1", "Protocol Metadata").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
    }
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(16.0, 16.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(session);
    app.sync_views();

    let mut empty_app = UiApp::new();
    let vp = ViewportMetrics::new(1440.0, 900.0, 1.0);
    let narrow_vp = ViewportMetrics::new(640.0, 560.0, 1.0);

    let dialogs = [
        ActiveDialog::FormatAxes { sheet_id: SheetId::new("graph-1") },
        ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("data-1") },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: SheetId::new("data-1"),
            existing_result_ids: vec![SheetId::new("res-1")],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ResultViewer { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ConstantHookPicker { target_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ChangeGraphType { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::FormatGraph { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::ApplyStyleFrom { target_graph_id: SheetId::new("graph-1") },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: SheetId::new("layout-1") },
        ActiveDialog::ExportFigure { sheet_id: SheetId::new("graph-1") },
        ActiveDialog::ConvertArchetype { sheet_id: SheetId::new("data-1") },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: SheetId::new("data-1") },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(SheetId::new("layout-1")) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(SheetId::new("data-1")) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::CreateGraph { source_sheet_id: Some(SheetId::new("data-1")) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: SheetId::new("layout-1") },
        ActiveDialog::FormatEmbeddedTable { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::About,
    ];
    let menu_keys: Vec<&'static str> = menu_bar_tooltips(&app).into_iter().map(|s| s.menu_key).collect();

    for &locale in i18n::LOCALES {
        let scanner = EguiSurfaceScanner::new();
        empty_app.set_locale(locale);
        for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
            empty_app.nav.welcome.open = true;
            empty_app.nav.welcome.mode = mode;
            let cap = scanner.scan(&mut empty_app, vp);
            assert_clean_egui_capture(&format!("welcome:{locale}:{mode:?}"), &cap, locale, false)?;
        }

        app.set_locale(locale);
        for sid in ["data-1", "info-1", "res-1", "graph-1", "layout-1"] {
            app.execute_command("sheet.select", serde_json::json!({ "sheet_id": sid })).map_err(|e| e.to_string())?;
            app.sync_views();
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("sheet:{locale}:{sid}"), &cap, locale, false)?;
        }
        for insp in [
            ActiveInspector::None,
            ActiveInspector::FormatGraph,
            ActiveInspector::FormatAxes,
            ActiveInspector::PairwiseComparisons,
            ActiveInspector::BubblePlot,
            ActiveInspector::PageLayout,
            ActiveInspector::ResultDetails,
        ] {
            app.canvas_dialogs.active_inspector = insp;
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("insp:{locale}:{insp:?}"), &cap, locale, false)?;
        }
        app.canvas_dialogs.active_inspector = ActiveInspector::None;

        for dlg in &dialogs {
            app.canvas_dialogs.active_dialog = dlg.clone();
            if matches!(dlg, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.command_palette.open();
                let cap_default = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}:palette_default"), &cap_default, locale, false)?;
                app.canvas_dialogs.command_palette.query = String::from("import");
                let cap_filtered = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}:palette_import"), &cap_filtered, locale, true)?;
                app.canvas_dialogs.command_palette.open();
            } else {
                let cap = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}"), &cap, locale, false)?;
            }
        }
        app.canvas_dialogs.active_dialog = ActiveDialog::None;

        for key in &menu_keys {
            app.interaction.open_menu = Some((*key).to_owned());
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("menu:{locale}:{key}"), &cap, locale, false)?;
        }
        app.interaction.open_menu = None;

        app.egui_views.top_chrome.overflow_menu_open = true;
        let overflow_cap = scanner.scan(&mut app, narrow_vp);
        assert_clean_egui_capture(&format!("overflow:{locale}"), &overflow_cap, locale, false)?;
        app.egui_views.top_chrome.overflow_menu_open = false;
    }

    // Vector export & archive manifest metadata generated from the same document.
    let graph = app.document().graph_sheet(&SheetId::new("graph-1")).ok_or_else(|| String::from("graph-1 missing"))?;
    let scene = compile_graph(graph, app.document()).map_err(|e| e.to_string())?;
    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    let pdf = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| e.to_string())?;
    let bundle = ClipboardBundle::from_scene(&scene, 144)?;
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&svg, &probe) || contains_ascii_word(&bundle.html, &probe) {
            return Err(format!("export vector/html leaked `{probe}`"));
        }
        let pdf_lossy = String::from_utf8_lossy(&pdf);
        if contains_ascii_word(&pdf_lossy, &probe) {
            return Err(format!("export pdf leaked `{probe}`"));
        }
    }
    for (kind, bytes) in [
        ("project", save_project_bytes(app.document()).map_err(|e| e.to_string())?),
        ("template", save_template_bytes(app.document()).map_err(|e| e.to_string())?),
    ] {
        let zip = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).map_err(|e| e.to_string())?;
        let manifest = zip.get_utf8("manifest.json").map_err(|e| e.to_string())?.ok_or_else(|| format!("{kind} missing manifest"))?;
        for probe in banned_probe_tokens() {
            if contains_ascii_word(manifest, &probe) {
                return Err(format!("{kind} manifest leaked `{probe}`"));
            }
        }
    }
    for spec in engine_cmd_registry().specs() {
        assert_clean_ui_string(&format!("engine_cmd:{}", spec.id), spec.label, "en", false)?;
        assert_clean_ui_string(&format!("engine_cmd_summary:{}", spec.id), spec.summary, "en", false)?;
    }
    for cmd in UI_COMMANDS {
        assert_clean_ui_string(&format!("ui_cmd:{}", cmd.id), cmd.label, "en", false)?;
        assert_clean_ui_string(&format!("ui_cmd_summary:{}", cmd.id), cmd.summary, "en", false)?;
    }

    Ok(())
}

#[test]
fn r021_mcp_packaging_and_command_artefacts_strictly_clean() -> Result<(), String> {
    let root = workspace_root()?;
    let dot_ext = join(&[".pr", "ism"]);
    let mime_proj = join(&["application/x-pr", "ism-project"]);
    let mime_xml = join(&["application/x-pr", "ism-xml"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);

    // 1. Verify packaging files carry zero brand waivers and zero product titles.
    for pkg_rel in [
        "packaging/macos/Info.plist.in",
        "packaging/linux/org.curvebook.CurveBook.desktop",
        "packaging/linux/org.curvebook.CurveBook.mime.xml",
        "packaging/windows/curvebook.wxs",
        "packaging/web/_headers",
        "packaging/web/.htaccess",
        "packaging/web/README.md",
    ] {
        let raw = fs::read_to_string(root.join(pkg_rel)).map_err(|e| format!("read {pkg_rel}: {e}"))?;
        let waiver = join(&["br", "and-ok"]);
        if raw.contains(&waiver) {
            return Err(format!("{pkg_rel} still contains `{waiver}` waiver"));
        }
        let cleaned = raw
            .replace(&dot_ext, ".ext")
            .replace(&mime_proj, "application/x-ext")
            .replace(&mime_xml, "application/x-ext-xml")
            .replace(&format!("<string>{}</string>", join(&["pr", "ism"])), "<string>ext</string>");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&cleaned, &probe) {
                return Err(format!("{pkg_rel} leaked prohibited token `{probe}`"));
            }
        }
    }

    // 2. Verify MCP source modules, docs/commands.md, docs/index.html, docs/site.css, and JSON schemas.
    let mcp_dir = root.join("crates/mcp/src");
    for entry in fs::read_dir(&mcp_dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let txt = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let cleaned =
                txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext").replace(&format!("\"{}\"", join(&["pr", "ism"])), "\"ext\"");
            for probe in banned_probe_tokens() {
                if contains_ascii_word(&cleaned, &probe) {
                    return Err(format!("{} leaked `{probe}`", path.display()));
                }
            }
        }
    }
    for rel in ["docs/commands.md", "docs/index.html", "docs/site.css"] {
        let txt = fs::read_to_string(root.join(rel)).map_err(|e| e.to_string())?;
        let cleaned = txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&cleaned, &probe) {
                return Err(format!("{rel} leaked `{probe}`"));
            }
        }
    }

    let schemas_dir = root.join("docs/schemas");
    if schemas_dir.is_dir() {
        for entry in fs::read_dir(&schemas_dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let txt = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let cleaned = txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext");
                for probe in banned_probe_tokens() {
                    if contains_ascii_word(&cleaned, &probe) {
                        return Err(format!("{} leaked `{probe}`", path.display()));
                    }
                }
            }
        }
    }

    Ok(())
}

#[test]
fn r021_asset_provenance_png_ico_crc_and_edited_docs_links_valid() -> Result<(), String> {
    let root = workspace_root()?;
    let assets_md = fs::read_to_string(root.join("ASSETS.md")).map_err(|e| e.to_string())?;
    let row_count = assets_md.lines().filter(|l| l.trim_start().starts_with("| `")).count();
    if row_count < 51 {
        return Err(format!("expected >= 51 ASSETS.md rows, found {row_count}"));
    }
    let pre_rename = join(&["lum", "encraft"]);
    if contains_ascii_word(&assets_md, &pre_rename) {
        return Err(String::from("ASSETS.md contains pre-rename product token"));
    }
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&assets_md, &probe) {
            return Err(format!("ASSETS.md contains prohibited token `{probe}`"));
        }
    }

    let expected_png_sig = b"\x89PNG\r\n\x1a\n";
    let mut verified_pngs = 0usize;
    let icon_root = root.join("assets/app-icon");
    let mut stack = vec![icon_root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) == Some("png") {
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                verify_png_chunks(&path, &bytes, expected_png_sig, &pre_rename)?;
                verified_pngs += 1;
            } else if path.extension().and_then(|e| e.to_str()) == Some("ico") {
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
                for i in 0..count {
                    let off = 6 + i * 16;
                    let sz = u32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off + 10], bytes[off + 11]]) as usize;
                    let img_off = u32::from_le_bytes([bytes[off + 12], bytes[off + 13], bytes[off + 14], bytes[off + 15]]) as usize;
                    let slice = &bytes[img_off..img_off + sz];
                    if slice.starts_with(expected_png_sig) {
                        verify_png_chunks(&path, slice, expected_png_sig, &pre_rename)?;
                    }
                }
            }
        }
    }
    if verified_pngs < 12 {
        return Err(format!("expected >= 12 PNGs under assets/app-icon, checked {verified_pngs}"));
    }

    let dot_ext = join(&[".pr", "ism"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
    let parity_doc = fs::read_to_string(root.join("docs/evidence/overview.md")).map_err(|e| e.to_string())?;
    if !parity_doc.contains("../../AGENTS.md#limited-local-jpg-reference-policy") || !parity_doc.contains("### Calibration") {
        return Err(String::from("docs/evidence/overview.md missing required policy link or Calibration heading"));
    }
    for doc_rel in [
        "docs/evidence/overview.md",
        "docs/evidence/curve-fitting.md",
        "docs/evidence/statistics.md",
        "docs/evidence/graphing.md",
        "docs/implementation/curve-fitting-implementation.md",
        "docs/implementation/data-tables-and-transforms.md",
        "docs/implementation/graphing-and-layouts.md",
        "docs/implementation/statistics-implementation.md",
        "docs/history/release/alpha-branding-clearance.md",
    ] {
        let content = fs::read_to_string(root.join(doc_rel)).map_err(|e| format!("read {doc_rel}: {e}"))?;
        let masked = content.replace(&dot_ext, ".fmt_ext").replace(&cmd_id, "table.import_ext");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&masked, &probe) {
                return Err(format!("{doc_rel} still contains prohibited term `{probe}`"));
            }
        }
    }

    Ok(())
}

fn verify_png_chunks(path: &Path, bytes: &[u8], png_sig: &[u8; 8], pre_rename: &str) -> Result<(), String> {
    if !bytes.starts_with(png_sig) {
        return Err(format!("{}: invalid PNG signature", path.display()));
    }
    let mut pos = 8usize;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let cdata = &bytes[pos + 8..pos + 8 + len];
        let stored_crc = u32::from_be_bytes([bytes[pos + 8 + len], bytes[pos + 9 + len], bytes[pos + 10 + len], bytes[pos + 11 + len]]);
        let computed_crc = crc32_ieee(&[ctype, cdata].concat());
        if stored_crc != computed_crc {
            return Err(format!("{}: PNG chunk {:?} CRC mismatch", path.display(), ctype));
        }
        if matches!(ctype, b"tEXt" | b"iTXt") {
            let text = String::from_utf8_lossy(cdata);
            if contains_ascii_word(&text, pre_rename) {
                return Err(format!("{}: PNG metadata still contains pre-rename token", path.display()));
            }
        }
        pos += 12 + len;
        if ctype == b"IEND" {
            break;
        }
    }
    Ok(())
}

struct TestGuiHost {
    app: UiApp,
}

impl GuiSessionHost for TestGuiHost {
    fn call_gui(&mut self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        if method == "ui.screenshot"
            && let Some(obj) = params.as_object_mut()
            && matches!(obj.get("target").and_then(serde_json::Value::as_str), Some("workspace" | "viewport"))
        {
            obj.insert(String::from("target"), serde_json::Value::String(String::from("full_window")));
        }
        curvebook_ui_egui::dispatch_gui_action(&mut self.app, method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

fn check_mcp_json_strict(ctx: &str, method: &str, called_tool: Option<&str>, resp: &serde_json::Value) -> Result<(), String> {
    let pre_rename = join(&["lum", "encraft"]);
    let raw_ext = join(&["pr", "ism"]);
    let dot_ext = join(&[".pr", "ism"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
    let mcp_tool_id = join(&["table_im", "port_pr", "ism"]);
    let serialized = serde_json::to_string(resp).map_err(|e| e.to_string())?;

    let mut masked = serialized;
    if method == "tools/list" {
        if let Some(tools) = resp.pointer("/result/tools").and_then(serde_json::Value::as_array) {
            for tool in tools {
                let name = tool.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
                if name == "open_file" {
                    let desc = tool.get("description").and_then(serde_json::Value::as_str).unwrap_or("");
                    if desc.contains(&dot_ext) {
                        let clean_desc = desc.replace(&dot_ext, ".ext");
                        masked = masked.replace(desc, &clean_desc);
                    }
                    let fmt_enum_pat =
                        format!("\"enum\":[\"curvebook\",\"curvebook.d\",\"cbtemplate\",\"{}\",\"pzfx\",\"xlsx\",\"csv\",\"tsv\"]", raw_ext);
                    masked = masked.replace(&fmt_enum_pat, "\"enum\":[\"curvebook\",\"pzfx\"]");
                } else if name == mcp_tool_id {
                    masked = masked.replace(&mcp_tool_id, "table_import_ext");
                    masked = masked.replace(&format!("[{cmd_id}]"), "[table.import_ext]");
                }
            }
        }
    } else if method == "tools/call" && matches!(called_tool, Some("list_toolsets" | "command_list" | "ui_inspect")) {
        masked = masked.replace(&mcp_tool_id, "table_import_ext");
        masked = masked.replace(&cmd_id, "table.import_ext");
    }

    // Strip base64 PNG image data blocks after verifying their binary PNG chunks cleanly.
    if method == "tools/call"
        && let Some(blocks) = resp.pointer("/result/content").and_then(serde_json::Value::as_array)
    {
        for block in blocks {
            if block.get("type").and_then(serde_json::Value::as_str) == Some("image")
                && let Some(b64) = block.get("data").and_then(serde_json::Value::as_str)
            {
                let png = decode_b64(b64).ok_or_else(|| format!("{ctx}: invalid base64 PNG"))?;
                verify_png_chunks(Path::new(ctx), &png, b"\x89PNG\r\n\x1a\n", &pre_rename)?;
                masked = masked.replace(b64, "CLEAN_PNG_DATA");
            }
        }
    }

    if contains_ascii_word(&masked, &pre_rename) {
        return Err(format!("{ctx}: MCP response leaked pre-rename token"));
    }
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&masked, &probe) {
            return Err(format!("{ctx}: MCP response leaked prohibited token `{probe}`"));
        }
    }
    Ok(())
}

fn decode_b64(b64: &str) -> Option<Vec<u8>> {
    let mut vals = Vec::with_capacity(b64.len());
    for &b in b64.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 255,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        vals.push(v);
    }
    if !vals.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity((vals.len() / 4) * 3);
    let (pairs, _) = vals.as_chunks::<4>();
    for &[a, b, c, d] in pairs {
        if a > 63 || b > 63 {
            return None;
        }
        out.push((a << 2) | (b >> 4));
        if c <= 63 {
            out.push((b << 4) | (c >> 2));
            if d <= 63 {
                out.push((c << 6) | d);
            }
        }
    }
    Some(out)
}

#[test]
fn r021_mcp_jsonrpc_headless_and_gui_only_positive_and_negative_regression() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R021 MCP Sweep").map_err(|e| e.to_string())?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let ds = instantiate_sample_fixture(fixture.id, format!("data-{}", idx + 1)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
    }
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(16.0, 16.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| e.to_string())?;

    // 1. Positive headless MCP JSON-RPC verification across initialize, tools, resources, prompts, doc_inspect, render_preview, and export.
    let mut headless = McpServerState::from_session(session.clone());
    let rpc = |st: &mut McpServerState, method: &str, params: serde_json::Value| -> Result<serde_json::Value, String> {
        dispatch_json_value(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }), st)
            .ok_or_else(|| format!("no reply for {method}"))
    };

    let init = rpc(&mut headless, "initialize", serde_json::json!({ "protocolVersion": "2024-11-05" }))?;
    if init.pointer("/result/serverInfo/name").and_then(serde_json::Value::as_str) != Some("curvebook") {
        return Err(String::from("unexpected MCP serverInfo.name"));
    }
    check_mcp_json_strict("headless:initialize", "initialize", None, &init)?;

    let core_tools = rpc(&mut headless, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:tools/list:core", "tools/list", None, &core_tools)?;

    let tsets = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "list_toolsets", "arguments": {} }))?;
    check_mcp_json_strict("headless:list_toolsets", "tools/call", Some("list_toolsets"), &tsets)?;

    let _ = rpc(
        &mut headless,
        "tools/call",
        serde_json::json!({ "name": "enable_toolset", "arguments": { "group": "all", "groups": ["gui"], "enabled": true } }),
    )?;
    let all_tools = rpc(&mut headless, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:tools/list:all", "tools/list", None, &all_tools)?;

    let res_list = rpc(&mut headless, "resources/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:resources/list", "resources/list", None, &res_list)?;
    let uris: Vec<String> = res_list
        .pointer("/result/resources")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("uri").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if uris.len() < 6 {
        return Err(format!("expected >= 6 MCP resources, got {}", uris.len()));
    }
    for uri in &uris {
        let rread = rpc(&mut headless, "resources/read", serde_json::json!({ "uri": uri }))?;
        check_mcp_json_strict(&format!("headless:resources/read:{uri}"), "resources/read", None, &rread)?;
    }

    let plist = rpc(&mut headless, "prompts/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:prompts/list", "prompts/list", None, &plist)?;
    let pnames: Vec<String> = plist
        .pointer("/result/prompts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    for pname in &pnames {
        let pget = rpc(&mut headless, "prompts/get", serde_json::json!({ "name": pname, "arguments": {} }))?;
        check_mcp_json_strict(&format!("headless:prompts/get:{pname}"), "prompts/get", None, &pget)?;
    }

    let doc_ins = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    check_mcp_json_strict("headless:doc_inspect", "tools/call", Some("doc_inspect"), &doc_ins)?;

    let preview = rpc(
        &mut headless,
        "tools/call",
        serde_json::json!({ "name": "render_preview", "arguments": { "sheet_id": "graph-1", "width_px": 240, "height_px": 180, "dpi": 144 } }),
    )?;
    check_mcp_json_strict("headless:render_preview", "tools/call", Some("render_preview"), &preview)?;

    let svg_exp = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "export", "arguments": { "format": "svg", "sheet_id": "graph-1" } }))?;
    check_mcp_json_strict("headless:export:svg", "tools/call", Some("export"), &svg_exp)?;

    // 2. Positive strict --gui-only MCP JSON-RPC verification.
    let mut app = UiApp::from_session(session);
    app.sync_views();
    app.set_gui_only(true);
    let mut gui_mcp = McpServerState::with_gui_host(Box::new(TestGuiHost { app }));
    let g_init = rpc(&mut gui_mcp, "initialize", serde_json::json!({ "protocolVersion": "2024-11-05" }))?;
    check_mcp_json_strict("gui_only:initialize", "initialize", None, &g_init)?;
    let g_tools = rpc(&mut gui_mcp, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("gui_only:tools/list", "tools/list", None, &g_tools)?;
    let g_prompts = rpc(&mut gui_mcp, "prompts/list", serde_json::json!({}))?;
    check_mcp_json_strict("gui_only:prompts/list", "prompts/list", None, &g_prompts)?;
    for pname in &pnames {
        let gpget = rpc(&mut gui_mcp, "prompts/get", serde_json::json!({ "name": pname, "arguments": {} }))?;
        check_mcp_json_strict(&format!("gui_only:prompts/get:{pname}"), "prompts/get", None, &gpget)?;
    }
    let g_ins = rpc(&mut gui_mcp, "tools/call", serde_json::json!({ "name": "ui_inspect", "arguments": { "scope": "all" } }))?;
    check_mcp_json_strict("gui_only:ui_inspect", "tools/call", Some("ui_inspect"), &g_ins)?;
    let g_shot = rpc(
        &mut gui_mcp,
        "tools/call",
        serde_json::json!({ "name": "ui_screenshot", "arguments": { "target": "workspace", "width_px": 240, "height_px": 180, "dpi": 144 } }),
    )?;
    check_mcp_json_strict("gui_only:ui_screenshot", "tools/call", Some("ui_screenshot"), &g_shot)?;

    // 3. Negative regression probes proving contaminated MCP instructions, prompts, resources, and tool descriptions fail.
    let vendor_gp = join(&["Gr", "aph", "Pad"]);
    let prod_pr = join(&["Pr", "ism"]);
    let vendor_op = join(&["Ori", "gin", "Pro"]);
    let suite_ac = join(&["Art", "Cr", "aft"]);
    let old_lc = join(&["Lum", "en", "Craft"]);

    let bad_init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "serverInfo": { "name": "curvebook" }, "instructions": format!("MCP helper for {vendor_gp} workflows") }
    });
    if check_mcp_json_strict("neg:init", "initialize", None, &bad_init).is_ok() {
        return Err(String::from("contaminated MCP instructions failed to trigger rejection"));
    }

    let bad_prompt = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": { "description": "fit", "messages": [{ "role": "user", "content": { "type": "text", "text": format!("Compare with {vendor_op}") } }] }
    });
    if check_mcp_json_strict("neg:prompt", "prompts/get", None, &bad_prompt).is_ok() {
        return Err(String::from("contaminated MCP prompt failed to trigger rejection"));
    }

    let bad_res = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "result": { "contents": [{ "uri": "sheet://data-1", "mimeType": "application/json", "text": format!("{{\"title\":\"{suite_ac} Study\"}}") }] }
    });
    if check_mcp_json_strict("neg:resource", "resources/read", None, &bad_res).is_ok() {
        return Err(String::from("contaminated MCP resource failed to trigger rejection"));
    }

    let bad_tool_desc = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "result": { "tools": [{ "name": "open_file", "description": format!("Open {prod_pr} (.prism) or {old_lc} files.") }] }
    });
    if check_mcp_json_strict("neg:tool", "tools/list", None, &bad_tool_desc).is_ok() {
        return Err(String::from("contaminated MCP open_file product description failed to trigger rejection"));
    }

    Ok(())
}

#[test]
fn r021_icns_gif_svg_docs_images_and_windows_ico_geometry_intact() -> Result<(), String> {
    let root = workspace_root()?;
    let png_sig = b"\x89PNG\r\n\x1a\n";
    let pre_rename = join(&["lum", "encraft"]);

    // 1. Verify docs/images/curvebook.png and assets/app-icon/slack/curvebook.png chunks + CRCs.
    for rel in ["docs/images/curvebook.png", "assets/app-icon/slack/curvebook.png"] {
        let bytes = fs::read(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        verify_png_chunks(&root.join(rel), &bytes, png_sig, &pre_rename)?;
    }

    // 2. Verify curvebook.icns header, embedded PNG frames, and non-PNG `info` plist chunk.
    let icns = fs::read(root.join("assets/app-icon/curvebook.icns")).map_err(|e| e.to_string())?;
    if !icns.starts_with(b"icns") || icns.len() < 8 {
        return Err(String::from("curvebook.icns missing `icns` magic"));
    }
    let icns_total = u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize;
    if icns_total != icns.len() {
        return Err(format!("curvebook.icns length mismatch: header={icns_total}, actual={}", icns.len()));
    }
    let mut pos = 8usize;
    let mut png_subicons = 0usize;
    while pos + 8 <= icns.len() {
        let tag = &icns[pos..pos + 4];
        let elen = u32::from_be_bytes([icns[pos + 4], icns[pos + 5], icns[pos + 6], icns[pos + 7]]) as usize;
        if elen < 8 || pos + elen > icns.len() {
            return Err(String::from("curvebook.icns malformed sub-chunk bounds"));
        }
        let payload = &icns[pos + 8..pos + elen];
        if payload.starts_with(png_sig) {
            verify_png_chunks(Path::new("curvebook.icns"), payload, png_sig, &pre_rename)?;
            png_subicons += 1;
        } else if tag == b"info" {
            let lossy = String::from_utf8_lossy(payload);
            if contains_ascii_word(&lossy, &pre_rename) {
                return Err(String::from("curvebook.icns `info` chunk contains pre-rename token"));
            }
            for probe in banned_probe_tokens() {
                if contains_ascii_word(&lossy, &probe) {
                    return Err(format!("curvebook.icns `info` chunk leaked `{probe}`"));
                }
            }
        }
        pos += elen;
    }
    if png_subicons < 8 {
        return Err(format!("expected >= 8 PNG sub-icons inside curvebook.icns, found {png_subicons}"));
    }

    // 3. Verify curvebook.svg, curvebook-small.svg, and org.curvebook.CurveBook.svg (outer XML + base64 PNG).
    for rel in
        ["assets/app-icon/curvebook.svg", "assets/app-icon/curvebook-small.svg", "assets/app-icon/hicolor/scalable/apps/org.curvebook.CurveBook.svg"]
    {
        let svg = fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        let marker = "data:image/png;base64,";
        let Some((before, after)) = svg.split_once(marker) else {
            return Err(format!("{rel} missing embedded base64 PNG"));
        };
        let Some((b64, tail)) = after.split_once('"') else {
            return Err(format!("{rel} unterminated data URI"));
        };
        let decoded = decode_b64(b64).ok_or_else(|| format!("{rel} invalid base64 PNG"))?;
        verify_png_chunks(Path::new(rel), &decoded, png_sig, &pre_rename)?;
        let outer = format!("{before}\"{tail}");
        if contains_ascii_word(&outer, &pre_rename) {
            return Err(format!("{rel} outer XML contains pre-rename token"));
        }
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&outer, &probe) {
                return Err(format!("{rel} outer XML leaked `{probe}`"));
            }
        }
    }

    // 4. Verify curvebook-subtle.gif header & absence of legacy/vendor strings, plus github-pages.md.
    let gif = fs::read(root.join("assets/app-icon/slack/curvebook-subtle.gif")).map_err(|e| e.to_string())?;
    if !gif.starts_with(b"GIF89a") {
        return Err(String::from("curvebook-subtle.gif missing GIF89a header"));
    }
    let gif_lossy = String::from_utf8_lossy(&gif).to_ascii_lowercase();
    if gif_lossy.contains(&pre_rename) {
        return Err(String::from("curvebook-subtle.gif contains pre-rename token"));
    }
    let pages_doc = fs::read_to_string(root.join("docs/release/github-pages.md")).map_err(|e| e.to_string())?;
    let dot_ext = format!(".{}", join(&["pr", "ism"]));
    let pages_cleaned = pages_doc.replace(&dot_ext, ".ext");
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&pages_cleaned, &probe) {
            return Err(format!("docs/release/github-pages.md leaked `{probe}`"));
        }
    }

    // 5. Verify curvebook.ico directory invariants matching packaging/windows/check-icons.ps1.
    let ico = fs::read(root.join("assets/app-icon/curvebook.ico")).map_err(|e| e.to_string())?;
    let reserved = u16::from_le_bytes([ico[0], ico[1]]);
    let icon_type = u16::from_le_bytes([ico[2], ico[3]]);
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    if reserved != 0 || icon_type != 1 || count != 7 {
        return Err(format!("invalid ICONDIR header: reserved={reserved}, type={icon_type}, count={count}"));
    }
    let dir_end = 6 + 16 * count;
    for i in 0..count {
        let entry = 6 + 16 * i;
        let entry_w = if ico[entry] == 0 { 256u32 } else { u32::from(ico[entry]) };
        let entry_h = if ico[entry + 1] == 0 { 256u32 } else { u32::from(ico[entry + 1]) };
        if ico[entry + 3] != 0 {
            return Err(format!("ICO frame {i} reserved byte non-zero"));
        }
        let sz = u32::from_le_bytes([ico[entry + 8], ico[entry + 9], ico[entry + 10], ico[entry + 11]]) as usize;
        let off = u32::from_le_bytes([ico[entry + 12], ico[entry + 13], ico[entry + 14], ico[entry + 15]]) as usize;
        if off < dir_end || off + sz > ico.len() {
            return Err(format!("ICO frame {i} out of bounds"));
        }
        let frame = &ico[off..off + sz];
        if !frame.starts_with(png_sig) || &frame[12..16] != b"IHDR" {
            return Err(format!("ICO frame {i} missing PNG/IHDR"));
        }
        let png_w = u32::from_be_bytes([frame[16], frame[17], frame[18], frame[19]]);
        let png_h = u32::from_be_bytes([frame[20], frame[21], frame[22], frame[23]]);
        if png_w != entry_w || png_h != entry_h {
            return Err(format!("ICO frame {i} dimension mismatch: dir={entry_w}x{entry_h}, ihdr={png_w}x{png_h}"));
        }
    }

    Ok(())
}
