//! Permanent regression gate for Wave 6 `R0.21` strict alpha branding, narrow external-archive
//! boundaries, 16-locale live `LayoutTree` rendering, packaging/MCP/doc inspection, and complete
//! asset-provenance verification (`R0.21`, `R10.14`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use curvebook_chart::compile_graph;
use curvebook_doc::{AnalysisKind, AnalysisSpec, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetId};
use curvebook_engine::{Session, cmd::registry as engine_cmd_registry};
use curvebook_format::{ZipArchive, ZipLimits, save_project_bytes, save_template_bytes};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_ui_egui::compositor::FramePalette;
use curvebook_ui_egui::dialogs_canvas::ActiveInspector;
use curvebook_ui_egui::i18n;
use curvebook_ui_egui::menus::{UI_COMMANDS, menu_bar_tooltips};
use curvebook_ui_egui::views::build_workspace_layout_tree;
use curvebook_ui_egui::workspace_nav::{SAMPLE_FIXTURES, WelcomeMode, instantiate_sample_fixture};
use curvebook_ui_egui::{ActiveDialog, ClipboardBundle, DesktopTextMeasurer, UiApp, ViewportMetrics};

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn join(parts: &[&str]) -> String {
    parts.concat()
}

fn run_xtask_in(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("run xtask {args:?} in {}: {e}", dir.display()))
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

#[test]
fn r021_xtask_cli_brands_cleanroom_assets_and_doc_checks_pass_zero_hits() -> Result<(), String> {
    let root = workspace_root()?;
    for subcmd in [&["brands"][..], &["cleanroom"][..], &["assets"][..], &["commands", "--check"][..], &["ui-commands", "--check"][..]] {
        let out = run_xtask_in(&root, subcmd)?;
        if !out.status.success() {
            return Err(format!(
                "`cargo xtask {}` failed ({}):\nstdout:\n{}\nstderr:\n{}",
                subcmd.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    Ok(())
}

#[test]
fn r021_live_layout_tree_and_a11y_clean_across_all_16_locales_and_5_sheets() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R021 Live LayoutTree Sweep").map_err(|e| e.to_string())?;
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
    let measurer = DesktopTextMeasurer::global();

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
        ActiveDialog::CommandPalette,
        ActiveDialog::About,
    ];
    let menu_keys: Vec<&'static str> = menu_bar_tooltips(&app).into_iter().map(|s| s.menu_key).collect();

    for &locale in i18n::LOCALES {
        empty_app.set_locale(locale);
        let empty_palette = FramePalette::from_theme(&empty_app.theme);
        for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
            empty_app.nav.welcome.open = true;
            empty_app.nav.welcome.mode = mode;
            let tree = build_workspace_layout_tree(&empty_app, vp, measurer, &empty_palette);
            for run in &tree.text_runs {
                assert_clean_ui_string(&format!("welcome:{locale}:{mode:?}"), &run.text, locale, false)?;
            }
        }

        app.set_locale(locale);
        let palette = FramePalette::from_theme(&app.theme);
        for sid in ["data-1", "info-1", "res-1", "graph-1", "layout-1"] {
            app.execute_command("sheet.select", serde_json::json!({ "sheet_id": sid })).map_err(|e| e.to_string())?;
            app.sync_views();
            let tree = build_workspace_layout_tree(&app, vp, measurer, &palette);
            for run in &tree.text_runs {
                assert_clean_ui_string(&format!("sheet:{locale}:{sid}:{}", run.owner_widget_id.as_str()), &run.text, locale, false)?;
            }
            for node in &tree.nodes {
                assert_clean_ui_string(&format!("sheet_a11y:{locale}:{sid}:{}", node.id.as_str()), &node.a11y.label, locale, false)?;
            }
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
            let tree = build_workspace_layout_tree(&app, vp, measurer, &palette);
            for run in &tree.text_runs {
                assert_clean_ui_string(&format!("insp:{locale}:{insp:?}"), &run.text, locale, false)?;
            }
        }
        app.canvas_dialogs.active_inspector = ActiveInspector::None;

        for dlg in &dialogs {
            app.canvas_dialogs.active_dialog = dlg.clone();
            if matches!(dlg, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.command_palette.open();
                let tree_default = build_workspace_layout_tree(&app, vp, measurer, &palette);
                for run in &tree_default.text_runs {
                    assert_clean_ui_string(&format!("dlg:{locale}:palette_default"), &run.text, locale, false)?;
                }
                app.canvas_dialogs.command_palette.query = String::from("import");
                let tree_filtered = build_workspace_layout_tree(&app, vp, measurer, &palette);
                for run in &tree_filtered.text_runs {
                    let is_status = run.owner_widget_id.as_str() == "palette.status";
                    assert_clean_ui_string(&format!("dlg:{locale}:palette_import"), &run.text, locale, is_status)?;
                }
                app.canvas_dialogs.command_palette.open();
            } else {
                let tree = build_workspace_layout_tree(&app, vp, measurer, &palette);
                for run in &tree.text_runs {
                    assert_clean_ui_string(&format!("dlg:{locale}"), &run.text, locale, false)?;
                }
            }
        }
        app.canvas_dialogs.active_dialog = ActiveDialog::None;

        for key in &menu_keys {
            app.interaction.open_menu = Some((*key).to_owned());
            let tree = build_workspace_layout_tree(&app, vp, measurer, &palette);
            for run in &tree.text_runs {
                assert_clean_ui_string(&format!("menu:{locale}:{key}"), &run.text, locale, false)?;
            }
        }
        app.interaction.open_menu = None;

        app.view_shards.toolbar_menus.overflow_open = true;
        let overflow_tree = build_workspace_layout_tree(&app, narrow_vp, measurer, &palette);
        for run in &overflow_tree.text_runs {
            assert_clean_ui_string(&format!("overflow:{locale}"), &run.text, locale, false)?;
        }
        app.view_shards.toolbar_menus.overflow_open = false;
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
    let parity_doc = fs::read_to_string(root.join("docs/target-app-parity.md")).map_err(|e| e.to_string())?;
    if !parity_doc.contains("../AGENTS.md#limited-local-jpg-reference-policy") || !parity_doc.contains("### Calibration") {
        return Err(String::from("docs/target-app-parity.md missing required policy link or Calibration heading"));
    }
    for doc_rel in [
        "docs/target-app-parity.md",
        "docs/fitting-parity.md",
        "docs/stats-parity.md",
        "docs/graphing-parity.md",
        "docs/implementation/curve-fitting-implementation.md",
        "docs/implementation/data-tables-and-transforms.md",
        "docs/implementation/graphing-and-layouts.md",
        "docs/implementation/statistics-implementation.md",
        "docs/release/alpha-branding-clearance.md",
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
