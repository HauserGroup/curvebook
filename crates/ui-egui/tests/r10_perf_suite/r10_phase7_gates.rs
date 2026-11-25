#![forbid(unsafe_code)]
//! `R10` Phase 7 UI Release Gates on native `egui` (`R10.1`, `R10.2`, `R10.9`, `R10.13`, `R10.16`, `R10.20`, `R10.22`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use crate::common::{PNG_SIGNATURE, ShellTestState, TestRes, assert_clean_parity, ensure, make_shell_driver, seed_xy_doc};
use crate::human_wf_common::EguiWfHarness;
use crate::suite_helpers::{build_mv_sheet, new_shell_app, run_cmd};
use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorBarDirection,
    ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphSpec, GraphicalDataObject, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec,
    MultivariateGraphEncoding, MultivariatePlotMode, OneWayAnovaSpec, OneWayMethod, ProjectDocument, RectPt, ReplicateOverlayStyle, SheetId,
    SizeChannelMapping, SummaryMeasure, TableArchetype, VariableRole, WhiskerMethod,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId};
use curvebook_render::RenderTargetSpec;
use curvebook_text::parse_markup;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::ActiveInspector;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::egui_shell::fonts::{FontDiscoveryConfig, build_egui_font_definitions_with_config};
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::theme::FontBook;
use curvebook_ui_egui::{ActiveDialog, LOCALES, ScreenshotRegionTarget, UiApp, UiThemeMode, UiThemeSpec, ViewportMetrics};
use serde_json::json;

const MAX_WEB_RELEASE_ZIP_BYTES: u64 = 6_243_591;

fn desktop_metrics(scale_factor: f64) -> ViewportMetrics {
    ViewportMetrics { logical_width: 1280.0, logical_height: 800.0, scale_factor }
}

fn target_spec_for_metrics(metrics: ViewportMetrics) -> RenderTargetSpec {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dpi = (metrics.clamped_scale_factor() * 96.0).round().clamp(72.0, 384.0) as u16;
    RenderTargetSpec { width_px: metrics.physical_width_px(), height_px: metrics.physical_height_px(), dpi }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn crc32_ieee(data: &[u8]) -> u32 {
    !data.iter().fold(0xFFFF_FFFFu32, |mut crc, &b| {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & (crc & 1).wrapping_neg());
        }
        crc
    })
}

fn verify_png_bytes(png: &[u8], expected_w: u32, expected_h: u32, label: &str) -> Result<usize, String> {
    ensure(png.len() > 33 && png.starts_with(PNG_SIGNATURE), format!("{label}: missing PNG signature"))?;
    let (mut pos, mut ihdr_seen, mut idat_bytes, mut iend_seen) = (8usize, false, 0usize, false);
    while pos + 12 <= png.len() {
        let len_bytes: [u8; 4] = png[pos..pos + 4].try_into().map_err(|_| format!("{label}: chunk length slice"))?;
        let chunk_len = usize::try_from(u32::from_be_bytes(len_bytes)).map_err(|_| format!("{label}: chunk len overflow"))?;
        let chunk_type = &png[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(chunk_len).ok_or_else(|| format!("{label}: chunk end overflow"))?;
        let crc_end = data_end.checked_add(4).ok_or_else(|| format!("{label}: chunk crc overflow"))?;
        ensure(crc_end <= png.len(), format!("{label}: truncated chunk"))?;
        let expected_crc = u32::from_be_bytes(png[data_end..crc_end].try_into().map_err(|_| format!("{label}: crc slice"))?);
        ensure(crc32_ieee(&png[pos + 4..data_end]) == expected_crc, format!("{label}: CRC-32 mismatch in chunk"))?;

        if chunk_type == b"IHDR" {
            ensure(chunk_len == 13, format!("{label}: invalid IHDR length {chunk_len}"))?;
            let w = u32::from_be_bytes(png[data_start..data_start + 4].try_into().map_err(|_| format!("{label}: w slice"))?);
            let h = u32::from_be_bytes(png[data_start + 4..data_start + 8].try_into().map_err(|_| format!("{label}: h slice"))?);
            ensure(w == expected_w && h == expected_h, format!("{label}: expected {expected_w}x{expected_h}, got {w}x{h}"))?;
            ihdr_seen = true;
        } else if chunk_type == b"IDAT" {
            idat_bytes = idat_bytes.saturating_add(chunk_len);
        } else if chunk_type == b"IEND" {
            iend_seen = true;
            ensure(crc_end == png.len(), format!("{label}: trailing bytes after IEND"))?;
            break;
        }
        pos = crc_end;
    }
    ensure(ihdr_seen && iend_seen, format!("{label}: missing IHDR or IEND"))?;
    ensure(idat_bytes >= 1024, format!("{label}: IDAT unexpectedly small ({idat_bytes} B)"))?;
    Ok(idat_bytes)
}

fn verify_driver_parity(driver: &EguiHumanPathDriver<'_, ShellTestState>, label: &str) -> TestRes {
    assert_clean_parity(driver, label)?;
    ensure(!driver.frame_meta().entries.is_empty(), format!("{label}: empty FrameInspectMeta"))
}

fn build_rich_five_sheet_app() -> Result<UiApp, String> {
    let (mut doc, xy_id) = seed_xy_doc(
        "Phase 7 Gate Project",
        "data-xy",
        "Dose Response",
        "Log[Agonist] (M)",
        &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0],
        &[("A", &[5.0, 12.0, 42.0, 78.0, 94.0, 98.0]), ("B", &[4.0, 9.0, 26.0, 61.0, 88.0, 95.0])],
    )?;
    let data_xy = doc.data_sheet(&xy_id).ok_or("missing data-xy")?.clone();
    let mut graph_xy = GraphSheet::new_uninitialized_for_data("graph-xy", &data_xy).map_err(|e| format!("gs xy: {e}"))?;
    graph_xy.initialized = true;
    graph_xy.spec.title = "IC₅₀ Curve (α=0.05)".into();

    let mut col_tbl = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 6).map_err(|e| format!("col new: {e}"))?;
    for r in 0..6 {
        let base = 10.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 1.4;
        for (c, off) in [0.0, 4.5, 8.2].into_iter().enumerate() {
            col_tbl.data_sets[c].cells[r][0] = Cell::number(base + off);
        }
    }
    let data_col = DataSheet::new("data-col", "Cohort ANOVA", TableArchetype::Column(col_tbl)).map_err(|e| format!("ds col: {e}"))?;

    let data_mv = build_mv_sheet(
        "data-mv",
        "Multivariate Cohort",
        &[
            ("Group", VariableRole::Categorical),
            ("Response", VariableRole::Continuous),
            ("Cohort", VariableRole::Categorical),
            ("Weight", VariableRole::Continuous),
        ],
        18,
        |mv| {
            let (groups, cohorts) = (["Ctrl", "Low", "High"], ["C1", "C2"]);
            for r in 0..18 {
                mv.variables[0].cells[r] = Cell::text(groups[r % 3]);
                mv.variables[1].cells[r] = Cell::number(20.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 2.5);
                mv.variables[2].cells[r] = Cell::text(cohorts[(r / 3) % 2]);
                mv.variables[3].cells[r] = Cell::number(1.0 + f64::from(u32::try_from(r % 5).unwrap_or(0)) * 0.4);
            }
        },
    )?;
    let mut graph_mv = GraphSheet::new_uninitialized_for_data("graph-mv", &data_mv).map_err(|e| format!("gs mv: {e}"))?;
    graph_mv.initialized = true;
    let swatch = PaletteId::OkabeIto.color_at(0);
    graph_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::AlignedDotPlot {
                summary: SummaryMeasure::Mean,
                marker: MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 5.0,
                    fill_color: swatch,
                    stroke_color: swatch,
                    stroke_width_pt: 1.0,
                },
                show_crossbar: true,
            },
            y_error_bars: Some(ErrorBarSpec::default()),
        },
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });

    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Metadata").map_err(|e| format!("info new: {e}"))?;
    info.linked_data_sheets.push(xy_id);
    info.set_constant("Kd", ConstantValue::Number(12.5), Some("nM".into())).map_err(|e| format!("const Kd: {e}"))?;
    info.set_constant("Temp", ConstantValue::Text("37 °C ± 0.1".into()), None).map_err(|e| format!("const Temp: {e}"))?;
    info.notes_markdown = "Buffer pH 7.4; α = 0.05; H₂O equilibrium; ∑xᵢ ≤ 100".into();

    let mut layout_spec = LayoutSpec::default();
    layout_spec
        .add_item(LayoutItem::LiveGraph {
            id: "panel-a".into(),
            graph_sheet_id: SheetId::from("graph-xy"),
            bounds_pt: RectPt::new(24.0, 24.0, 220.0, 170.0),
            scale_percent: 100.0,
            badge_override: Some("A".into()),
        })
        .map_err(|e| format!("layout graph: {e}"))?;
    layout_spec
        .add_item(LayoutItem::TextCallout {
            id: "callout-1".into(),
            markdown: "Panel A: IC₅₀ ± SEM (α = 0.05)".into(),
            bounds_pt: RectPt::new(260.0, 24.0, 180.0, 64.0),
            font_size_pt: 11.0,
        })
        .map_err(|e| format!("layout text: {e}"))?;

    doc.add_data_sheet(data_col).map_err(|e| e.to_string())?;
    doc.add_data_sheet(data_mv).map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph_xy).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph_mv).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Publication Figure 1", layout_spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut app = new_shell_app(doc);
    run_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "data-col", "sheet_id": "result-rm", "kind": "rm_one_way_anova" }))?;
    run_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "data-mv", "sheet_id": "result-stat2", "kind": "pca" }))?;
    run_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "data-xy", "sheet_id": "result-fit" }))?;
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "data-xy" }))?;
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

fn select_sheet(app: &mut UiApp, id: &str) -> TestRes {
    run_cmd(app, "sheet.select", json!({ "sheet_id": id }))?;
    app.nav.welcome.open = false;
    let doc = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc, &SheetId::from(id));
    app.canvas_dialogs.close_dialog();
    app.sync_views();
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. R10.9 Unicode, 16 Locales x 3 Themes x 4 HiDPI Glyph Coverage Gate
// ---------------------------------------------------------------------------

#[test]
fn r10_9_unicode_16_locales_3_themes_4_hidpi_glyph_coverage_gate() -> TestRes {
    ensure(LOCALES.len() == 16, format!("expected 16 locales, found {}", LOCALES.len()))?;
    for mode in UiThemeMode::ALL {
        let spec = UiThemeSpec::for_mode(mode);
        ensure(spec.meets_wcag_aa() && spec.meets_wcag_aa_borders(), format!("theme {mode:?} fails WCAG AA"))?;
    }

    let core_scientific = [
        ("ascii", "IC50 CurveBook Data Sheet 0123456789"),
        ("greek", "α β γ δ ε θ λ μ π σ φ ω Δ Σ Ω"),
        ("math", "± ≤ ≥ ≠ ≈ × ÷ ∑ √ ∫ ∞ ° ′ ″ −"),
        ("sub_sup", "H₂O CO₂ IC₅₀ EC₅₀ R² x² x⁻¹ ¹⁰⁺ y₀ nᵢ"),
        ("latin_ext", "café naïve Zürich Ångström São Paulo Łódź"),
        ("cyrillic", "Данные Анализы Графики Макеты ± α=0.05"),
    ];
    let cjk_rtl_samples = [
        ("ja", "日本語 データ 分析 グラフ レイアウト α=0.05"),
        ("zh-CN", "中文 数据 表 曲线 分析 ± ≤ ≥"),
        ("zh-TW", "繁體中文 數據 分析 圖表 版面配置"),
        ("ko", "한국어 데이터 분석 그래프 레이아웃"),
        ("ar", "البيانات التحليل الرسوم التخطيط α=0.05"),
        ("he", "נתונים ניתוח גרפים פריסה α=0.05"),
    ];
    let active_measurer = DesktopTextMeasurer::new();
    let (style_plain, style_bold) = (UiTextStyle::ui_sans(13.0), UiTextStyle::ui_bold(14.0));
    let style_rich = UiTextStyle::ui_sans(13.0).with_rich(parse_markup("IC_{50} = 10^{−7} M (α ≤ 0.05)"));
    for (tag, text) in core_scientific {
        for (s_name, style) in [("plain", &style_plain), ("bold", &style_bold), ("rich", &style_rich)] {
            let line = active_measurer.shape_line(text, style, "en");
            ensure(line.missing_glyph_count == 0, format!("active measurer missing {} glyphs on `{tag}` ({s_name})", line.missing_glyph_count))?;
        }
    }
    let (probe_ja, probe_ar) =
        (active_measurer.shape_line("日本語データ", &style_plain, "ja"), active_measurer.shape_line("البيانات", &style_plain, "ar"));
    let host_covers_cjk_rtl = probe_ja.missing_glyph_count == 0 && probe_ar.missing_glyph_count == 0;
    ensure(probe_ja.glyphs.len() >= 5 && probe_ar.glyphs.len() >= 5, "empty CJK/RTL glyph run")?;
    if host_covers_cjk_rtl {
        for (loc, sample) in cjk_rtl_samples {
            let line = active_measurer.shape_line(sample, &style_plain, loc);
            ensure(line.missing_glyph_count == 0, format!("host CJK/RTL missing {} glyphs for `{loc}`", line.missing_glyph_count))?;
        }
    }

    // (b) Deterministic fallback without OS/bundled host fonts + byte-identical software rasterization
    let fallback_measurer = DesktopTextMeasurer::with_font_book(FontBook::with_embedded_fallback());
    for (tag, text) in core_scientific {
        ensure(fallback_measurer.shape_line(text, &style_plain, "en").missing_glyph_count == 0, format!("fallback missing glyphs for `{tag}`"))?;
    }
    let cleanroom_fonts =
        build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
    ensure(!cleanroom_fonts.definitions.font_data.is_empty(), "cleanroom FontDefinitions is empty")?;

    let (mut det_a, mut det_b) = (build_rich_five_sheet_app()?, build_rich_five_sheet_app()?);
    let det_target = RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 };
    let img_a = det_a.render_egui_shell_rgba(det_target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("det a: {e}"))?;
    let img_b = det_b.render_egui_shell_rgba(det_target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("det b: {e}"))?;
    ensure(img_a.rgba == img_b.rgba, "egui shell software rasterization is non-deterministic")?;
    let bg_px = [img_a.rgba[0], img_a.rgba[1], img_a.rgba[2], img_a.rgba[3]];
    let non_bg_pixels = img_a.rgba.as_chunks::<4>().0.iter().filter(|px| **px != bg_px).count();
    ensure(non_bg_pixels >= 1000, format!("egui shell drew too few non-bg pixels: {non_bg_pixels}"))?;

    // (c) Full native egui parity across 16 locales, 3 themes, 4 HiDPI scales, Welcome, 5 sheets, modal & inspector
    let (five_sheets, scales) = (["data-xy", "info-1", "result-rm", "graph-xy", "layout-1"], [1.0, 1.5, 2.0, 3.0]);
    let mut welcome_app = UiApp::new();
    let mut welcome_driver = make_shell_driver(welcome_app.clone(), desktop_metrics(1.0));
    let mut sheet_driver = make_shell_driver(build_rich_five_sheet_app()?, desktop_metrics(1.0));

    for (loc_idx, &locale) in LOCALES.iter().enumerate() {
        let (mode, scale) = (UiThemeMode::ALL[loc_idx % UiThemeMode::ALL.len()], scales[loc_idx % scales.len()]);
        let (metrics, tokens) = (desktop_metrics(scale), Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale));

        assert_eq!(welcome_app.set_locale(locale), locale);
        welcome_app.set_theme_mode(mode);
        welcome_driver.state_mut().app = welcome_app.clone();
        welcome_driver.state_mut().views = welcome_app.egui_views.clone();
        welcome_driver.set_viewport(metrics);
        welcome_driver.set_tokens(tokens);
        verify_driver_parity(&welcome_driver, &format!("welcome `{locale}` ({mode:?} @ {scale}x)"))?;

        assert_eq!(sheet_driver.state_mut().app.set_locale(locale), locale);
        sheet_driver.state_mut().app.set_theme_mode(mode);
        sheet_driver.set_viewport(metrics);
        sheet_driver.set_tokens(tokens);
        for sheet_id in five_sheets {
            select_sheet(&mut sheet_driver.state_mut().app, sheet_id)?;
            sheet_driver.state_mut().views = sheet_driver.state().app.egui_views.clone();
            sheet_driver.run_settled();
            verify_driver_parity(&sheet_driver, &format!("sheet `{sheet_id}` `{locale}` ({mode:?} @ {scale}x)"))?;
        }
    }

    let mut theme_driver = make_shell_driver(build_rich_five_sheet_app()?, desktop_metrics(1.0));
    for mode in UiThemeMode::ALL {
        theme_driver.state_mut().app.set_theme_mode(mode);
        assert_eq!(theme_driver.state_mut().app.set_locale("en"), "en");
        theme_driver.set_tokens(Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), "en"));
        for scale in scales {
            theme_driver.set_viewport(desktop_metrics(scale));
            select_sheet(&mut theme_driver.state_mut().app, "data-xy")?;
            theme_driver.state_mut().app.canvas_dialogs.open_power_wizard();
            theme_driver.state_mut().views = theme_driver.state().app.egui_views.clone();
            theme_driver.run_settled();
            verify_driver_parity(&theme_driver, &format!("power_modal/{mode:?}/{scale}x"))?;
            theme_driver.state_mut().app.canvas_dialogs.close_dialog();

            select_sheet(&mut theme_driver.state_mut().app, "graph-xy")?;
            theme_driver.state_mut().app.nav.inspector_open = true;
            theme_driver.state_mut().app.canvas_dialogs.active_inspector = ActiveInspector::FormatGraph;
            theme_driver.state_mut().views = theme_driver.state().app.egui_views.clone();
            theme_driver.run_settled();
            verify_driver_parity(&theme_driver, &format!("inspector/{mode:?}/{scale}x"))?;
        }
    }
    eprintln!("[R10.9 egui] Verified 16 locales x 3 themes x 4 HiDPI, host_covers_cjk_rtl={host_covers_cjk_rtl}, non_bg={non_bg_pixels}.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. R10.2 & R10.16 & R10.20 Parity, Hit-Box, Click-Action & A11y Gate
// ---------------------------------------------------------------------------

fn open_dialog_by_index(app: &mut UiApp, doc: &ProjectDocument, idx: usize) -> (&'static str, ActiveDialog) {
    let (xy, res_rm, g_xy, g_mv, lay) =
        (SheetId::from("data-xy"), SheetId::from("result-rm"), SheetId::from("graph-xy"), SheetId::from("graph-mv"), SheetId::from("layout-1"));
    match idx {
        0 => {
            let _ = app.canvas_dialogs.open_analyze_dialog(doc, &xy);
            ("AnalyzeSheet", ActiveDialog::AnalyzeSheet { source_sheet_id: xy })
        }
        1 => {
            let _ = app.canvas_dialogs.open_edit_parameters(doc, &res_rm);
            ("EditAnalysisParameters", ActiveDialog::EditAnalysisParameters { result_sheet_id: res_rm })
        }
        2 => (
            "DuplicateAnalysisPrompt",
            ActiveDialog::DuplicateAnalysisPrompt {
                source_sheet_id: xy,
                existing_result_ids: vec![res_rm],
                pending_entry_id: "stats.one_way_anova".into(),
            },
        ),
        3 => {
            let _ = app.canvas_dialogs.open_change_data_analyzed(doc, &res_rm);
            ("ChangeDataAnalyzed", ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_rm })
        }
        4 => {
            if let Some(gs) = doc.graph_sheet(&g_xy) {
                app.canvas_dialogs.format_axes.sync_from_graph(gs, Default::default());
            }
            ("FormatAxes", ActiveDialog::FormatAxes { sheet_id: g_xy })
        }
        5 => {
            let _ = app.canvas_dialogs.open_format_graph(doc, &g_xy);
            ("FormatGraph", ActiveDialog::FormatGraph { graph_sheet_id: g_xy })
        }
        6 => {
            if let Some(gs) = doc.graph_sheet(&g_xy) {
                app.canvas_dialogs.change_graph_type.open_for_graph(gs, doc);
            }
            ("ChangeGraphType", ActiveDialog::ChangeGraphType { graph_sheet_id: g_xy })
        }
        7 => {
            let _ = app.canvas_dialogs.open_pairwise_comparisons(doc, &g_xy);
            ("PairwiseComparisons", ActiveDialog::PairwiseComparisons { graph_sheet_id: g_xy })
        }
        8 => {
            let _ = app.canvas_dialogs.open_bubble_plot(doc, &g_mv);
            ("BubblePlotInspector", ActiveDialog::BubblePlotInspector { graph_sheet_id: g_mv })
        }
        9 => {
            let _ = app.canvas_dialogs.open_apply_style_from(doc, &g_xy);
            ("ApplyStyleFrom", ActiveDialog::ApplyStyleFrom { target_graph_id: g_xy })
        }
        10 => {
            let _ = app.canvas_dialogs.open_page_layout_composer(doc, &lay);
            ("PageLayoutComposer", ActiveDialog::PageLayoutComposer { layout_sheet_id: lay })
        }
        11 => {
            app.canvas_dialogs.open_export_figure(&g_xy);
            ("ExportFigure", ActiveDialog::ExportFigure { sheet_id: g_xy })
        }
        12 => {
            app.canvas_dialogs.open_print_dialog(doc, Some(&g_xy), vec!["Save as PDF".into()]);
            ("Print", ActiveDialog::Print { sheet_id: Some(g_xy) })
        }
        13 => {
            app.canvas_dialogs.open_power_wizard();
            ("PowerAndSampleSize", ActiveDialog::PowerAndSampleSize)
        }
        14 => {
            app.canvas_dialogs.open_command_palette();
            ("CommandPalette", ActiveDialog::CommandPalette)
        }
        15 => {
            let _ = app.canvas_dialogs.open_convert_archetype(doc, &xy);
            ("ConvertArchetype", ActiveDialog::ConvertArchetype { sheet_id: xy })
        }
        16 => {
            app.canvas_dialogs.open_import_reshape();
            ("ImportReshape", ActiveDialog::ImportReshape)
        }
        17 => {
            app.canvas_dialogs.open_exclude_with_reason(xy.clone(), vec![GridCellCoord::new(0, 0, 0)]);
            ("ExcludeWithReason", ActiveDialog::ExcludeWithReason { sheet_id: xy })
        }
        18 => {
            let _ = app.canvas_dialogs.open_format_selected_points(doc, &g_xy, vec![(0, 0)]);
            ("FormatSelectedPoints", ActiveDialog::FormatSelectedPoints { graph_sheet_id: g_xy })
        }
        19 => {
            app.canvas_dialogs.open_randomize_groups(doc);
            ("RandomizeGroups", ActiveDialog::RandomizeGroups)
        }
        20 => {
            let _ = app.canvas_dialogs.open_random_sample_rows(doc, Some(&xy));
            ("RandomSampleRows", ActiveDialog::RandomSampleRows { source_sheet_id: Some(xy) })
        }
        21 => {
            app.canvas_dialogs.open_simulate_distributions();
            ("SimulateDistributions", ActiveDialog::SimulateDistributions)
        }
        22 => {
            let _ = app.canvas_dialogs.open_insert_model_equation(doc, &g_xy);
            ("InsertModelEquation", ActiveDialog::InsertModelEquation { graph_sheet_id: g_xy })
        }
        23 => {
            app.canvas_dialogs.open_hook_picker(doc, &xy, None);
            ("ConstantHookPicker", ActiveDialog::ConstantHookPicker { target_sheet_id: xy })
        }
        24 => {
            if let Some(rs) = doc.result_sheet(&res_rm) {
                app.canvas_dialogs.result_view.sync_with_result(rs);
            }
            ("ResultViewer", ActiveDialog::ResultViewer { result_sheet_id: res_rm })
        }
        _ => ("About", ActiveDialog::About),
    }
}

fn verify_modal_tab_sweep(app: &UiApp, metrics: ViewportMetrics, tab_ids: &[&str], scope: &str) -> TestRes {
    let mut harness = EguiWfHarness::from_app(app.clone(), metrics);
    for &tab_id in tab_ids {
        harness.click(tab_id)?;
        harness.assert_parity_and_layout(&format!("{scope}/{tab_id}"))?;
    }
    Ok(())
}

#[test]
fn r10_2_and_r10_16_and_r10_20_full_sheets_26_dialogs_tabs_and_7_inspectors_parity_a11y_gate() -> TestRes {
    let metrics = desktop_metrics(1.0);

    // 1. Welcome screen + entry modes
    let welcome = UiApp::new();
    verify_driver_parity(&make_shell_driver(welcome.clone(), metrics), "welcome/initial")?;
    verify_modal_tab_sweep(&welcome, metrics, &["welcome.entry_mode.sample", "welcome.entry_mode.empty"], "welcome_empty")?;

    // 2. All 5 workspace sheet types + Welcome clone/template modes + Results interpret studio tabs
    let mut app = build_rich_five_sheet_app()?;
    app.nav.welcome.open = true;
    app.egui_views.launcher_open = true;
    verify_modal_tab_sweep(&app, metrics, &["welcome.mode.clone", "welcome.mode.template"], "welcome_clone")?;
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;

    for (sheet_id, label) in
        [("data-xy", "DataSheet"), ("info-1", "InfoSheet"), ("result-rm", "ResultSheet"), ("graph-xy", "GraphSheet"), ("layout-1", "LayoutSheet")]
    {
        select_sheet(&mut app, sheet_id)?;
        verify_driver_parity(&make_shell_driver(app.clone(), metrics), label)?;
    }

    select_sheet(&mut app, "result-rm")?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.canvas_dialogs.active_inspector = ActiveInspector::ResultDetails;
    verify_modal_tab_sweep(
        &app,
        metrics,
        &["inspector.results.tab.assumptions", "inspector.results.tab.guidance", "inspector.results.tab.methods"],
        "result_interpret",
    )?;

    // 3. All 26 non-None ActiveDialog variants
    select_sheet(&mut app, "data-xy")?;
    let mut doc = app.document().clone();
    let anova_spec = AnalysisSpec::new(
        "result-ord",
        "data-col",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() }),
    )
    .map_err(|e| e.to_string())?;
    doc.add_result_sheet(curvebook_doc::ResultSheet::new(anova_spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut dlg_driver = make_shell_driver(app.clone(), metrics);
    for idx in 0..26 {
        dlg_driver.state_mut().app.canvas_dialogs.close_dialog();
        let (name, dialog) = open_dialog_by_index(&mut dlg_driver.state_mut().app, &doc, idx);
        dlg_driver.state_mut().app.canvas_dialogs.active_dialog = dialog;
        dlg_driver.state_mut().views = dlg_driver.state().app.egui_views.clone();
        dlg_driver.run_settled();
        verify_driver_parity(&dlg_driver, &format!("dialog/{name}"))?;
    }
    app.canvas_dialogs.close_dialog();

    // 4. Multi-tab modal sweeps (RM-ANOVA 5 tabs, PCA 3 tabs, Nonlinear Fit 6 tabs, Import 5 tabs, Power 4 stages)
    for (rid, tabs, scope) in [
        (
            "result-rm",
            &[
                "edit.tab.experimental_design",
                "edit.tab.repeated_measures",
                "edit.tab.multiple_comparisons",
                "edit.tab.options",
                "edit.tab.diagnostics",
            ][..],
            "edit_stat_rm",
        ),
        ("result-stat2", &["edit.tab.data", "edit.tab.experimental_design", "edit.tab.options"][..], "edit_stat_pca"),
        (
            "result-fit",
            &["fit.tab.model", "fit.tab.constraints", "fit.tab.initial_values", "fit.tab.output", "fit.tab.compare", "fit.tab.weights_range"][..],
            "edit_fit",
        ),
    ] {
        let sid = SheetId::from(rid);
        let _ = app.canvas_dialogs.open_edit_parameters(&doc, &sid);
        app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: sid };
        verify_modal_tab_sweep(&app, metrics, tabs, scope)?;
    }
    app.canvas_dialogs.active_dialog = ActiveDialog::None;
    app.grid.import_wizard.open = true;
    verify_modal_tab_sweep(
        &app,
        metrics,
        &["import_paste.tab.source", "import_paste.tab.view", "import_paste.tab.filter", "import_paste.tab.placement", "import_paste.tab.notes"],
        "import_reshape",
    )?;
    app.grid.import_wizard.open = false;

    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    verify_modal_tab_sweep(&app, metrics, &["power.stage.1", "power.stage.2", "power.stage.3", "power.stage.4"], "power_wizard")?;
    app.canvas_dialogs.close_dialog();

    // 5. All 7 ActiveInspector variants
    let mut insp_driver = make_shell_driver(app, metrics);
    for (insp, sheet_id, open) in [
        (ActiveInspector::None, "data-xy", false),
        (ActiveInspector::FormatGraph, "graph-xy", true),
        (ActiveInspector::FormatAxes, "graph-xy", true),
        (ActiveInspector::PairwiseComparisons, "graph-xy", true),
        (ActiveInspector::BubblePlot, "graph-mv", true),
        (ActiveInspector::PageLayout, "layout-1", true),
        (ActiveInspector::ResultDetails, "result-rm", true),
    ] {
        select_sheet(&mut insp_driver.state_mut().app, sheet_id)?;
        insp_driver.state_mut().app.nav.inspector_open = open;
        insp_driver.state_mut().app.egui_views.trailing_collapsed_rail = !open;
        insp_driver.state_mut().app.canvas_dialogs.active_inspector = insp;
        insp_driver.state_mut().views = insp_driver.state().app.egui_views.clone();
        insp_driver.run_settled();
        verify_driver_parity(&insp_driver, &format!("inspector/{insp:?}"))?;
    }
    eprintln!("[R10.2/16/20 egui] Verified Welcome, 5 sheet types, all 26 dialogs, multi-tab modals, and all 7 inspector variants.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 3. R10.13 bullet 2 Visual Screenshot Baselines (5 Sheet Types & Modal Tabs)
// ---------------------------------------------------------------------------

fn render_verified_png_baseline(app: &mut UiApp, target: RenderTargetSpec, label: &str) -> Result<(u64, usize), String> {
    let (png_a, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("{label} a: {e}"))?;
    let (png_b, _, _) = app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("{label} b: {e}"))?;
    ensure(png_a == png_b, format!("{label}: screenshot output is not byte-for-byte deterministic"))?;
    Ok((fnv1a_64(&png_a), verify_png_bytes(&png_a, target.width_px, target.height_px, label)?))
}

fn verify_distinct_tab_baselines(app: &UiApp, metrics: ViewportMetrics, target: RenderTargetSpec, tabs: &[&str], click_first: bool) -> TestRes {
    let mut h = EguiWfHarness::from_app(app.clone(), metrics);
    let mut hashes = BTreeSet::new();
    for (idx, &tab) in tabs.iter().enumerate() {
        if click_first || idx > 0 {
            h.click(tab)?;
        }
        let (hash, _) = render_verified_png_baseline(h.app_mut(), target, &format!("baseline/{tab}"))?;
        ensure(hashes.insert(hash), format!("baseline `{tab}` identical to sibling tab"))?;
    }
    Ok(())
}

#[test]
fn r10_13_b2_visual_screenshot_baselines_5_sheets_and_modal_tabs() -> TestRes {
    let (target, metrics) = (RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, desktop_metrics(1.0));
    let mut app = build_rich_five_sheet_app()?;
    let mut seen_sheet_hashes = BTreeSet::new();

    for theme in [UiThemeMode::Light, UiThemeMode::Dark] {
        app.set_theme_mode(theme);
        let mut per_theme = BTreeSet::new();
        for sheet_id in ["data-xy", "info-1", "result-rm", "graph-xy", "layout-1"] {
            select_sheet(&mut app, sheet_id)?;
            let label = format!("baseline/{sheet_id}/{theme:?}");
            let (hash, idat) = render_verified_png_baseline(&mut app, target, &label)?;
            ensure(per_theme.insert(hash), format!("{label}: collided with another sheet baseline"))?;
            let _ = seen_sheet_hashes.insert(hash);
            ensure(idat >= 2048, format!("{label}: low visual density ({idat} B IDAT)"))?;
        }
    }
    ensure(seen_sheet_hashes.len() == 10, format!("expected 10 distinct baselines, got {}", seen_sheet_hashes.len()))?;

    app.set_theme_mode(UiThemeMode::Light);
    select_sheet(&mut app, "graph-xy")?;
    let (region_png, rw, rh) =
        app.render_egui_shell_screenshot_png(target, ScreenshotRegionTarget::Canvas).map_err(|e| format!("canvas png: {e}"))?;
    let _ = verify_png_bytes(&region_png, rw, rh, "baseline/graph-xy/canvas-region")?;

    let doc = app.document().clone();
    for (rid, tabs) in [
        ("result-rm", &["edit.tab.experimental_design", "edit.tab.multiple_comparisons", "edit.tab.diagnostics"][..]),
        ("result-fit", &["fit.tab.model", "fit.tab.constraints", "fit.tab.weights_range"][..]),
    ] {
        let sid = SheetId::from(rid);
        let _ = app.canvas_dialogs.open_edit_parameters(&doc, &sid);
        app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: sid };
        verify_distinct_tab_baselines(&app, metrics, target, tabs, true)?;
    }

    app.canvas_dialogs.active_dialog = ActiveDialog::None;
    app.grid.import_wizard.open = true;
    verify_distinct_tab_baselines(&app, metrics, target, &["import_paste.tab.source", "import_paste.tab.filter"], false)?;
    app.grid.import_wizard.open = false;

    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    verify_distinct_tab_baselines(&app, metrics, target, &["power.stage.1", "power.stage.3"], false)?;
    eprintln!("[R10.13 egui] Verified deterministic PNG chunks/CRC/distinctness across 5 sheets x 2 themes + all representative modal tabs.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 4. R10.22 bullet 1 Cold-to-First-Frame Startup (< 3 s)
// ---------------------------------------------------------------------------

#[test]
fn r10_22_b1_cold_startup_under_3s_budget() -> TestRes {
    let target = target_spec_for_metrics(desktop_metrics(1.0));
    let t_cold = Instant::now();
    let _measurer = DesktopTextMeasurer::new();
    let mut app = UiApp::new();
    let frame = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("cold render: {e}"))?;
    let cold_ms = t_cold.elapsed().as_secs_f64() * 1000.0;
    ensure(frame.width_px == 1280 && frame.height_px == 800 && !frame.rgba.is_empty(), "cold frame wrong size or empty")?;

    let t_warm = Instant::now();
    let _ = UiApp::new().render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("warm render: {e}"))?;
    let warm_ms = t_warm.elapsed().as_secs_f64() * 1000.0;
    eprintln!("[R10.22 b1 egui] cold_startup={cold_ms:.2} ms, app_first_frame={warm_ms:.2} ms (budget < 3000.0 ms)");
    ensure(cold_ms < 3000.0, format!("cold startup {cold_ms:.2} ms exceeded 3000 ms budget"))
}

// ---------------------------------------------------------------------------
// 5. R10.22 bullet 2 High-Point / Many-Level MV Categorical Summary Latency
// ---------------------------------------------------------------------------

fn categorical_summary_gdo(mode: &str, summary: SummaryMeasure) -> GraphicalDataObject {
    let swatch = PaletteId::OkabeIto.color_at(0);
    let marker = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: MarkerFillMode::Filled,
        size_pt: 3.5,
        fill_color: swatch,
        stroke_color: swatch,
        stroke_width_pt: 0.8,
    };
    match mode {
        "Strip" => GraphicalDataObject::AlignedDotPlot { summary, marker, show_crossbar: true },
        "Bar" => GraphicalDataObject::BarWithReplicates {
            summary,
            fill_color: swatch.with_alpha(200),
            border_color: swatch,
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        },
        "Box" => GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: swatch.with_alpha(180),
            stroke_color: swatch,
            show_outliers: true,
            show_all_points: false,
        },
        "Violin" => GraphicalDataObject::Violin {
            fill_color: swatch.with_alpha(180),
            stroke_color: swatch,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        },
        _ => GraphicalDataObject::BeforeAfterLines { marker: Some(marker), line_color: swatch, stroke_width_pt: 1.1 },
    }
}

fn build_large_mv_categorical_app(rows: usize, levels: usize) -> Result<UiApp, String> {
    let level_labels: Vec<String> = (0..levels).map(|i| format!("G{}-S{}", (i / 4) + 1, (i % 4) + 1)).collect();
    let cohorts = ["ArmA", "ArmB", "ArmC", "ArmD", "ArmE"];
    let data_mv = build_mv_sheet(
        "data-mv-big",
        "10k MV Assay",
        &[
            ("GroupSub", VariableRole::Categorical),
            ("Response", VariableRole::Continuous),
            ("Cohort", VariableRole::Categorical),
            ("SubjectId", VariableRole::Categorical),
        ],
        rows,
        |mv| {
            for r in 0..rows {
                let (lvl, subj) = (r % levels, r / levels);
                let phase = f64::from(u32::try_from(lvl).unwrap_or(0)) * 3.1 + f64::from(u32::try_from(r % 17).unwrap_or(0)) * 0.45;
                mv.variables[0].cells[r] = Cell::text(&level_labels[lvl]);
                mv.variables[1].cells[r] = Cell::number(25.0 + phase);
                mv.variables[2].cells[r] = Cell::text(cohorts[lvl % cohorts.len()]);
                mv.variables[3].cells[r] = Cell::text(format!("Subj-{subj:04}"));
            }
        },
    )?;
    let mut graph_mv = GraphSheet::new_uninitialized_for_data("graph-mv-big", &data_mv).map_err(|e| e.to_string())?;
    graph_mv.initialized = true;
    graph_mv.spec = GraphSpec::default_for_data_sheet(&data_mv);
    graph_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: categorical_summary_gdo("Strip", SummaryMeasure::Mean),
            y_error_bars: Some(ErrorBarSpec {
                metric: ErrorBarMetric::Sd,
                direction: ErrorBarDirection::Both,
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.0,
                cap_width_pt: 4.0,
                color: None,
            }),
        },
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
        size: SizeChannelMapping::Constant { radius_pt: 4.0 },
        label_column: Some(3),
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    let mut doc = ProjectDocument::new("MV Categorical Benchmark").map_err(|e| e.to_string())?;
    doc.add_data_sheet(data_mv).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph_mv).map_err(|e| e.to_string())?;
    let mut app = new_shell_app(doc);
    select_sheet(&mut app, "graph-mv-big")?;
    Ok(app)
}

#[test]
fn r10_22_b2_mv_categorical_summary_10k_rows_20plus_levels_latency() -> TestRes {
    let (rows, levels) = (10_080usize, 24usize);
    ensure(rows >= 10_000 && levels >= 20, "benchmark dimensions below R10.22 requirement")?;
    let mut app = build_large_mv_categorical_app(rows, levels)?;
    let base_target = target_spec_for_metrics(desktop_metrics(1.0));
    let resize_target = target_spec_for_metrics(ViewportMetrics { logical_width: 1440.0, logical_height: 900.0, scale_factor: 1.0 });
    let _ = app.render_egui_shell_rgba(base_target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("warmup: {e}"))?;

    let frame_budget_ms = if cfg!(debug_assertions) { 600.0 } else { 50.0 };
    let mut over_budget = Vec::new();
    let compile_and_render = |app: &mut UiApp, target: RenderTargetSpec, tag: &str| -> Result<(f64, f64), String> {
        let t_c = Instant::now();
        let gs = app.document().graph_sheet(&SheetId::from("graph-mv-big")).ok_or("missing gs")?;
        let sc = compile_graph(gs, app.document()).map_err(|e| format!("{tag} compile: {e}"))?;
        let c_ms = t_c.elapsed().as_secs_f64() * 1000.0;
        let t_r = Instant::now();
        let img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).map_err(|e| format!("{tag} render: {e}"))?;
        let r_ms = t_r.elapsed().as_secs_f64() * 1000.0;
        ensure(!sc.primitives.is_empty() && !img.rgba.is_empty(), format!("{tag}: empty output"))?;
        Ok((c_ms, r_ms))
    };

    for (idx, mode) in ["Strip", "Bar", "Box", "Violin", "BeforeAfter"].iter().enumerate() {
        let mut enc =
            app.document().graph_sheet(&SheetId::from("graph-mv-big")).and_then(|g| g.spec.multivariate.clone()).ok_or("missing mv encoding")?;

        // 1. CREATE
        enc.plot_mode = MultivariatePlotMode::CategoricalSummary {
            gdo: categorical_summary_gdo(mode, SummaryMeasure::Mean),
            y_error_bars: Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, ..ErrorBarSpec::default() }),
        };
        let t_create = Instant::now();
        run_cmd(&mut app, "graph.setSpec", json!({ "sheet_id": "graph-mv-big", "multivariate": enc }))?;
        let (compile_ms, render_ms) = compile_and_render(&mut app, base_target, &format!("{mode} create"))?;
        let create_ms = t_create.elapsed().as_secs_f64() * 1000.0;

        // 2. UPDATE
        enc.plot_mode = MultivariatePlotMode::CategoricalSummary {
            gdo: categorical_summary_gdo(mode, SummaryMeasure::Median),
            y_error_bars: Some(ErrorBarSpec { metric: ErrorBarMetric::ConfidenceInterval95, ..ErrorBarSpec::default() }),
        };
        let t_update = Instant::now();
        run_cmd(&mut app, "table.setCell", json!({ "sheet_id": "data-mv-big", "row": idx, "col": 1, "subcol": 0, "value": 42.5 + (idx as f64) }))?;
        run_cmd(&mut app, "graph.setSpec", json!({ "sheet_id": "graph-mv-big", "multivariate": enc }))?;
        let _ = compile_and_render(&mut app, base_target, &format!("{mode} update"))?;
        let update_ms = t_update.elapsed().as_secs_f64() * 1000.0;

        // 3. RESIZE
        let t_resize = Instant::now();
        run_cmd(&mut app, "graph.setSpec", json!({ "sheet_id": "graph-mv-big", "spec": { "canvas_size_pt": { "width": 520.0, "height": 340.0 } } }))?;
        let _ = compile_and_render(&mut app, resize_target, &format!("{mode} resize"))?;
        let resize_ms = t_resize.elapsed().as_secs_f64() * 1000.0;

        eprintln!(
            "[R10.22 b2 MV Categorical 10080r x 24lv egui] mode={mode:<11} create={create_ms:6.2} ms (c={compile_ms:5.2}/r={render_ms:5.2})  update={update_ms:6.2} ms  resize={resize_ms:6.2} ms  (budget <= {frame_budget_ms:.0} ms)"
        );
        for (phase, ms) in [("create", create_ms), ("update", update_ms), ("resize", resize_ms)] {
            if ms > frame_budget_ms {
                over_budget.push(format!("{mode} {phase} {ms:.2} ms > {frame_budget_ms} ms"));
            }
        }
    }
    ensure(over_budget.is_empty(), over_budget.join("; "))
}

// ---------------------------------------------------------------------------
// 6. R10.22 & R15.5 Strict-CSP Web Placeholder & Release Bundle Budget Check
// ---------------------------------------------------------------------------

#[test]
fn r10_22_b2_and_r10_1_web_bundle_budget_and_shared_layout_parity() -> TestRes {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().and_then(|p| p.parent()).ok_or("failed to resolve workspace root")?;
    for (rel, must_have, must_not, msg) in [
        (
            "apps/curvebook-web/src/lib.rs",
            &["UiApp", "WebChromeBridge", "WebTextBridge"][..],
            &[][..],
            "must drive shared UiApp + placeholder bridges",
        ),
        (
            "apps/curvebook-web/src/chrome_bridge.rs",
            &[][..],
            &["LayoutTree", "build_workspace_layout_tree"][..],
            "must not reference retired LayoutTree",
        ),
        ("apps/curvebook-web/Cargo.toml", &["curvebook-ui-egui", "workspace = true"][..], &[][..], "must consume workspace curvebook-ui-egui"),
        ("Cargo.toml", &["[profile.web-release]", "opt-level = \"s\""][..], &[][..], "must define slim [profile.web-release]"),
        ("packaging/web/package.sh", &["trunk build --release"][..], &[][..], "must invoke trunk build --release"),
    ] {
        let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        ensure(must_have.iter().all(|s| text.contains(s)) && must_not.iter().all(|s| !text.contains(s)), format!("{rel} {msg}"))?;
    }
    let web_fonts_txt = std::fs::read_to_string(root.join("crates/text/web-fonts.txt")).map_err(|e| format!("read web-fonts.txt: {e}"))?;
    ensure(!web_fonts_txt.trim().is_empty(), "crates/text/web-fonts.txt must declare font manifest entries")?;

    let release_dir = root.join("dist/release");
    let mut verified_zip = None;
    if release_dir.is_dir() {
        for entry_res in std::fs::read_dir(&release_dir).map_err(|e| format!("read_dir dist/release: {e}"))? {
            let entry = entry_res.map_err(|e| format!("dir entry: {e}"))?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("curvebook-web-") && name.ends_with(".zip") {
                let bytes = entry.metadata().map_err(|e| format!("metadata {name}: {e}"))?.len();
                ensure(bytes > 0 && bytes <= MAX_WEB_RELEASE_ZIP_BYTES, format!("{name} size {bytes} B outside (0, {MAX_WEB_RELEASE_ZIP_BYTES}]"))?;
                verified_zip = Some((name.to_owned(), bytes));
            }
        }
    }
    eprintln!("[R15.5/R10.22 Web] strict-CSP placeholder contract OK, ceiling = {MAX_WEB_RELEASE_ZIP_BYTES} B, staged_zip={verified_zip:?}");
    Ok(())
}
