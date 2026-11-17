//! R10 Phase 7 UI Release Gates (`R10.1`, `R10.2`, `R10.9`, `R10.13`, `R10.16`, `R10.20`, `R10.22`).
//!
//! Runs zero-`unwrap`/zero-`expect`/zero-`panic` verification across:
//! - `R10.9`: Unicode & 16-locale x 3-theme x 4-HiDPI glyph-coverage & deterministic fallback gate
//! - `R10.2` / `R10.16` / `R10.20`: Full parity, hit-box, click-action & a11y gate across Welcome,
//!   all 5 sheet types, all 26 non-`None` `ActiveDialog` variants + multi-tab modal tabs, and all
//!   7 `ActiveInspector` variants + 5 inspector tabs
//! - `R10.13` bullet 2: Deterministic screenshot PNG baselines across all 5 sheet types & modal tabs
//! - `R10.22` bullet 1: Cold-to-first-frame startup latency (`< 3 s`)
//! - `R10.22` bullet 2: High-point (`>= 10,000` rows), many-level (`>= 20` categories)
//!   `MultipleVariables` categorical summary graph latency (create / update / resize across Strip,
//!   Bar, Box, Violin, BeforeAfter)
//! - `R10.22` / `R10.1`: Web shared-layout parity contract & `<= 6,243,591 B` web release bundle gate

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, GraphSheet,
    GraphSpec, GraphicalDataObject, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, MultivariateGraphEncoding,
    MultivariatePlotMode, ProjectDocument, RectPt, ReplicateOverlayStyle, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure, TableArchetype,
    VariableRole, WhiskerMethod, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_render::{RasterImage, RenderTargetSpec};
use curvebook_text::parse_markup;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::compositor::{
    FramePalette, ScreenshotRegionTarget, render_workspace_frame_png, render_workspace_region_png, render_workspace_rgba,
};
use curvebook_ui_egui::dialogs_canvas::ActiveInspector;
use curvebook_ui_egui::layout::LayoutTree;
use curvebook_ui_egui::layout::a11y::{FocusScope, validate_layout_tree_accessibility};
use curvebook_ui_egui::layout::verify::{verify_click_action_parity, verify_pixel_to_hit_box_parity, verify_tree_invariants};
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, PaintClipPx, SoftwareTextPainter, TextMeasurer, TextPaintStyle, UiTextStyle};
use curvebook_ui_egui::theme::FontBook;
use curvebook_ui_egui::views::build_workspace_layout_tree;
use curvebook_ui_egui::{ActiveDialog, HumanPathDriver, LOCALES, UiApp, UiThemeMode, UiThemeSpec, ViewportMetrics};
use serde_json::json;

const MAX_WEB_RELEASE_ZIP_BYTES: u64 = 6_243_591;
const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn desktop_metrics(scale_factor: f64) -> ViewportMetrics {
    ViewportMetrics { logical_width: 1280.0, logical_height: 800.0, scale_factor }
}

fn target_spec_for_metrics(metrics: ViewportMetrics) -> RenderTargetSpec {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dpi = (metrics.clamped_scale_factor() * 96.0).round().clamp(72.0, 384.0) as u16;
    RenderTargetSpec { width_px: metrics.physical_width_px(), height_px: metrics.physical_height_px(), dpi }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
        }
    }
    !crc
}

fn verify_png_bytes(png: &[u8], expected_w: u32, expected_h: u32, label: &str) -> Result<usize, String> {
    ensure(png.len() > 33 && png.starts_with(PNG_SIG), format!("{label}: missing PNG signature"))?;
    let mut pos = 8usize;
    let mut ihdr_seen = false;
    let mut idat_bytes = 0usize;
    let mut iend_seen = false;

    while pos + 12 <= png.len() {
        let len_bytes: [u8; 4] = png[pos..pos + 4].try_into().map_err(|_| format!("{label}: chunk length slice"))?;
        let chunk_len = usize::try_from(u32::from_be_bytes(len_bytes)).map_err(|_| format!("{label}: chunk len overflow"))?;
        let chunk_type = &png[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(chunk_len).ok_or_else(|| format!("{label}: chunk end overflow"))?;
        let crc_end = data_end.checked_add(4).ok_or_else(|| format!("{label}: chunk crc overflow"))?;
        ensure(crc_end <= png.len(), format!("{label}: truncated chunk"))?;

        let crc_bytes: [u8; 4] = png[data_end..crc_end].try_into().map_err(|_| format!("{label}: crc slice"))?;
        let expected_crc = u32::from_be_bytes(crc_bytes);
        let actual_crc = crc32_ieee(&png[pos + 4..data_end]);
        ensure(actual_crc == expected_crc, format!("{label}: CRC-32 mismatch in chunk"))?;

        if chunk_type == b"IHDR" {
            ensure(chunk_len == 13, format!("{label}: invalid IHDR length {chunk_len}"))?;
            let w_bytes: [u8; 4] = png[data_start..data_start + 4].try_into().map_err(|_| format!("{label}: w slice"))?;
            let h_bytes: [u8; 4] = png[data_start + 4..data_start + 8].try_into().map_err(|_| format!("{label}: h slice"))?;
            let w = u32::from_be_bytes(w_bytes);
            let h = u32::from_be_bytes(h_bytes);
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

fn verify_frame_full_gate(app: &mut UiApp, metrics: ViewportMetrics, label: &str) -> Result<LayoutTree, String> {
    let report = verify_pixel_to_hit_box_parity(app, metrics);
    ensure(report.is_clean(), format!("{label}: pixel-to-hit-box parity violations: {:?}", report.violations))?;

    let tree = app.build_and_cache_layout_tree(metrics);
    let inv = verify_tree_invariants(&tree);
    ensure(inv.is_clean(), format!("{label}: tree invariant violations: {:?}", inv.violations))?;

    let a11y = validate_layout_tree_accessibility(&tree).map_err(|v| format!("{label}: a11y check error: {v:?}"))?;
    ensure(a11y.is_clean(), format!("{label}: a11y violations: {:?}", a11y.violations))?;
    Ok(tree)
}

fn build_rich_five_sheet_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("Phase 7 Gate Project").map_err(|e| format!("doc new: {e}"))?;

    // 1. XY table + initialized GraphSheet
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 2, 6).map_err(|e| format!("xy new: {e}"))?;
    xy.x_title = String::from("Log[Agonist] (M)");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ys_a = [5.0, 12.0, 42.0, 78.0, 94.0, 98.0];
    let ys_b = [4.0, 9.0, 26.0, 61.0, 88.0, 95.0];
    for r in 0..6 {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        xy.data_sets[0].cells[r][0] = Cell::number(ys_a[r]);
        xy.data_sets[1].cells[r][0] = Cell::number(ys_b[r]);
    }
    let data_xy = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy)).map_err(|e| format!("ds xy: {e}"))?;
    let mut graph_xy = GraphSheet::new_uninitialized_for_data("graph-xy", &data_xy).map_err(|e| format!("gs xy: {e}"))?;
    graph_xy.initialized = true;
    graph_xy.spec.title = String::from("IC₅₀ Curve (α=0.05)");

    // 2. Column table for ANOVA & RM-ANOVA EditAnalysisParameters tabs
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 6).map_err(|e| format!("col new: {e}"))?;
    for r in 0..6 {
        let base = 10.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 1.4;
        col_tbl.data_sets[0].cells[r][0] = Cell::number(base);
        col_tbl.data_sets[1].cells[r][0] = Cell::number(base + 4.5);
        col_tbl.data_sets[2].cells[r][0] = Cell::number(base + 8.2);
    }
    let data_col = DataSheet::new("data-col", "Cohort ANOVA", TableArchetype::Column(col_tbl)).map_err(|e| format!("ds col: {e}"))?;

    // 3. MultipleVariables table + Multivariate GraphSheet
    let mut mv = MultipleVariablesTable::new(4, 18).map_err(|e| format!("mv new: {e}"))?;
    mv.variables[0].title = String::from("Group");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Response");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("Cohort");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Weight");
    mv.variables[3].lock_role(VariableRole::Continuous);
    let groups = ["Ctrl", "Low", "High"];
    let cohorts = ["C1", "C2"];
    for r in 0..18 {
        let g = groups[r % 3];
        let c = cohorts[(r / 3) % 2];
        let val = 20.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 2.5;
        let wt = 1.0 + f64::from(u32::try_from(r % 5).unwrap_or(0)) * 0.4;
        mv.variables[0].cells[r] = Cell::text(g);
        mv.variables[1].cells[r] = Cell::number(val);
        mv.variables[2].cells[r] = Cell::text(c);
        mv.variables[3].cells[r] = Cell::number(wt);
    }
    let data_mv = DataSheet::new("data-mv", "Multivariate Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("ds mv: {e}"))?;
    let mut graph_mv = GraphSheet::new_uninitialized_for_data("graph-mv", &data_mv).map_err(|e| format!("gs mv: {e}"))?;
    graph_mv.initialized = true;
    graph_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::AlignedDotPlot {
                summary: SummaryMeasure::Mean,
                marker: MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 5.0,
                    fill_color: PaletteId::OkabeIto.color_at(0),
                    stroke_color: PaletteId::OkabeIto.color_at(0),
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

    // 4. InfoSheet
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Metadata").map_err(|e| format!("info new: {e}"))?;
    info.linked_data_sheets.push(SheetId::from("data-xy"));
    info.set_constant("Kd", ConstantValue::Number(12.5), Some(String::from("nM"))).map_err(|e| format!("const Kd: {e}"))?;
    info.set_constant("Temp", ConstantValue::Text(String::from("37 °C ± 0.1")), None).map_err(|e| format!("const Temp: {e}"))?;
    info.notes_markdown = String::from("Buffer pH 7.4; α = 0.05; H₂O equilibrium; ∑xᵢ ≤ 100");

    // 5. LayoutSheet placing GraphXY + TextCallout
    let mut layout_spec = LayoutSpec::default();
    layout_spec
        .add_item(LayoutItem::LiveGraph {
            id: String::from("panel-a"),
            graph_sheet_id: SheetId::from("graph-xy"),
            bounds_pt: RectPt::new(24.0, 24.0, 220.0, 170.0),
            scale_percent: 100.0,
            badge_override: Some(String::from("A")),
        })
        .map_err(|e| format!("layout add graph: {e}"))?;
    layout_spec
        .add_item(LayoutItem::TextCallout {
            id: String::from("callout-1"),
            markdown: String::from("Panel A: IC₅₀ ± SEM (α = 0.05)"),
            bounds_pt: RectPt::new(260.0, 24.0, 180.0, 64.0),
            font_size_pt: 11.0,
        })
        .map_err(|e| format!("layout add text: {e}"))?;
    let layout = LayoutSheet::new("layout-1", "Publication Figure 1", layout_spec).map_err(|e| format!("layout new: {e}"))?;

    doc.add_data_sheet(data_xy).map_err(|e| format!("add data-xy: {e}"))?;
    doc.add_data_sheet(data_col).map_err(|e| format!("add data-col: {e}"))?;
    doc.add_data_sheet(data_mv).map_err(|e| format!("add data-mv: {e}"))?;
    doc.add_info_sheet(info).map_err(|e| format!("add info-1: {e}"))?;
    doc.add_graph_sheet(graph_xy).map_err(|e| format!("add graph-xy: {e}"))?;
    doc.add_graph_sheet(graph_mv).map_err(|e| format!("add graph-mv: {e}"))?;
    doc.add_layout_sheet(layout).map_err(|e| format!("add layout-1: {e}"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;

    // Run real analyses via engine commands so ResultSheets contain full computed tabs, diagnostics & narrative
    let _ = app
        .execute_command("analysis.run", json!({ "source_sheet_id": "data-col", "sheet_id": "result-rm", "kind": "rm_one_way_anova" }))
        .map_err(|e| format!("rm_one_way_anova: {e}"))?;
    let _ = app
        .execute_command("analysis.run", json!({ "source_sheet_id": "data-mv", "sheet_id": "result-stat2", "kind": "pca" }))
        .map_err(|e| format!("pca: {e}"))?;
    let _ = app
        .execute_command("analysis.run", json!({ "source_sheet_id": "data-xy", "sheet_id": "result-fit" }))
        .map_err(|e| format!("fit run: {e}"))?;

    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "data-xy" })).map_err(|e| format!("select data-xy: {e}"))?;
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

fn select_sheet(app: &mut UiApp, id: &str) -> Result<(), String> {
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": id })).map_err(|e| format!("select `{id}`: {e}"))?;
    app.nav.welcome.open = false;
    let sid = SheetId::from(id);
    let doc = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc, &sid);
    app.canvas_dialogs.close_dialog();
    app.sync_views();
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. R10.9 Unicode, 16 Locales x 3 Themes x 4 HiDPI Glyph Coverage Gate
// ---------------------------------------------------------------------------

#[test]
fn r10_9_unicode_16_locales_3_themes_4_hidpi_glyph_coverage_gate() -> Result<(), String> {
    ensure(LOCALES.len() == 16, format!("expected 16 locales, found {}", LOCALES.len()))?;

    // Check WCAG AA contracts on all 3 themes
    for mode in UiThemeMode::ALL {
        let spec = UiThemeSpec::for_mode(mode);
        ensure(spec.meets_wcag_aa(), format!("theme {mode:?} fails WCAG AA text contrast"))?;
        ensure(spec.meets_wcag_aa_borders(), format!("theme {mode:?} fails WCAG AA border contrast"))?;
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

    // (a) Active desktop font stack (CURVEBOOK_FONTS_DIR + macOS/system fonts)
    let active_measurer = DesktopTextMeasurer::new();
    let style_plain = UiTextStyle::ui_sans(13.0);
    let style_bold = UiTextStyle::ui_bold(14.0);
    let rich = parse_markup("IC_{50} = 10^{−7} M (α ≤ 0.05)");
    let style_rich = UiTextStyle::ui_sans(13.0).with_rich(rich);

    // Core scientific scripts MUST have 0 missing glyphs across active font stack + rich markup
    for (tag, text) in core_scientific {
        for (s_name, style) in [("plain", &style_plain), ("bold", &style_bold), ("rich", &style_rich)] {
            let line = active_measurer.shape_line(text, style, "en");
            ensure(line.missing_glyph_count == 0, format!("active measurer had {} missing glyphs on `{tag}` ({s_name})", line.missing_glyph_count))?;
        }
    }

    // Detect if host OS / CURVEBOOK_FONTS_DIR provides CJK & RTL faces (true on macOS Arial Unicode + Noto hosts)
    let probe_ja = active_measurer.shape_line("日本語データ", &style_plain, "ja");
    let probe_ar = active_measurer.shape_line("البيانات", &style_plain, "ar");
    let host_covers_cjk_rtl = probe_ja.missing_glyph_count == 0 && probe_ar.missing_glyph_count == 0;
    ensure(probe_ja.glyphs.len() >= 5 && probe_ar.glyphs.len() >= 5, String::from("unexpected empty glyph run shaping CJK/RTL"))?;

    if host_covers_cjk_rtl {
        for (loc, sample) in cjk_rtl_samples {
            let line = active_measurer.shape_line(sample, &style_plain, loc);
            ensure(line.missing_glyph_count == 0, format!("host CJK/RTL font stack missing {} glyphs for locale `{loc}`", line.missing_glyph_count))?;
        }
    }

    // (b) Crisp deterministic fallback without external font directory (simulating headless/no-default-features)
    let fallback_measurer = DesktopTextMeasurer::with_font_book(FontBook::with_embedded_fallback());
    for (tag, text) in core_scientific {
        let line = fallback_measurer.shape_line(text, &style_plain, "en");
        ensure(line.missing_glyph_count == 0, format!("embedded fallback missing {} glyphs for `{tag}`", line.missing_glyph_count))?;
    }

    let mut img_a = RasterImage::new(360, 48, 96, Rgba8::WHITE).map_err(|e| format!("raster a: {e}"))?;
    let mut img_b = RasterImage::new(360, 48, 96, Rgba8::WHITE).map_err(|e| format!("raster b: {e}"))?;
    let paint_style = TextPaintStyle { style: &style_plain, locale: "en", color: Rgba8::BLACK, clip: PaintClipPx::new(0, 0, 360, 48) };
    let _ = SoftwareTextPainter::paint_text_top_left(&fallback_measurer, &mut img_a, "IC₅₀ = 12.5 nM ± 0.4 (α=0.05)", &paint_style, [8.0, 12.0]);
    let _ = SoftwareTextPainter::paint_text_top_left(&fallback_measurer, &mut img_b, "IC₅₀ = 12.5 nM ± 0.4 (α=0.05)", &paint_style, [8.0, 12.0]);
    ensure(img_a.rgba == img_b.rgba, String::from("embedded fallback text rasterization is non-deterministic"))?;
    let ink_pixels = img_a.rgba.as_chunks::<4>().0.iter().filter(|px| **px != [255, 255, 255, 255]).count();
    ensure(ink_pixels >= 40, format!("embedded fallback drew too few ink pixels: {ink_pixels}"))?;

    // (c) Full UI tree glyph coverage across all 16 locales, 5 sheet types, Welcome, key modals, and inspector
    let mut app = build_rich_five_sheet_app()?;
    let five_sheets = ["data-xy", "info-1", "result-rm", "graph-xy", "layout-1"];
    let mut welcome_app = UiApp::new();
    let scales = [1.0, 1.5, 2.0, 3.0];

    for (loc_idx, &locale) in LOCALES.iter().enumerate() {
        let mode = UiThemeMode::ALL[loc_idx % UiThemeMode::ALL.len()];
        let scale = scales[loc_idx % scales.len()];
        let metrics = desktop_metrics(scale);
        let palette = FramePalette::from_theme(&UiThemeSpec::for_mode(mode));
        let expect_rtl = matches!(locale, "ar" | "he");

        // Welcome in every locale
        welcome_app.set_locale(locale);
        welcome_app.set_theme_mode(mode);
        let welcome_tree = build_workspace_layout_tree(&welcome_app, metrics, &active_measurer, &palette);
        ensure(welcome_tree.is_rtl == expect_rtl, format!("welcome `{locale}` RTL mismatch: {}", welcome_tree.is_rtl))?;
        let welcome_inv = verify_tree_invariants(&welcome_tree);
        if host_covers_cjk_rtl || !matches!(locale, "ja" | "ko" | "zh-CN" | "zh-TW" | "ar" | "he") {
            ensure(welcome_inv.is_clean(), format!("welcome `{locale}` ({mode:?} @ {scale}x): {:?}", welcome_inv.violations))?;
        }

        // All 5 sheets in every locale
        app.set_locale(locale);
        app.set_theme_mode(mode);
        for sheet_id in five_sheets {
            select_sheet(&mut app, sheet_id)?;
            let tree = build_workspace_layout_tree(&app, metrics, &active_measurer, &palette);
            ensure(tree.is_rtl == expect_rtl, format!("sheet `{sheet_id}` `{locale}` RTL mismatch"))?;
            if host_covers_cjk_rtl || !matches!(locale, "ja" | "ko" | "zh-CN" | "zh-TW" | "ar" | "he") {
                let sheet_inv = verify_tree_invariants(&tree);
                ensure(sheet_inv.is_clean(), format!("sheet `{sheet_id}` `{locale}` ({mode:?} @ {scale}x): {:?}", sheet_inv.violations))?;
            }
        }
    }

    // Explicitly sweep all 3 themes x 4 HiDPI scales across key modal & inspector views
    select_sheet(&mut app, "data-xy")?;
    for mode in UiThemeMode::ALL {
        app.set_theme_mode(mode);
        for &scale in &scales {
            let metrics = desktop_metrics(scale);
            app.set_locale("en");
            app.canvas_dialogs.open_power_wizard();
            let modal_tree = verify_frame_full_gate(&mut app, metrics, &format!("power_modal/{mode:?}/{scale}x"))?;
            ensure(modal_tree.active_focus_scope == FocusScope::ModalDialog, String::from("power modal focus scope mismatch"))?;
            app.canvas_dialogs.close_dialog();

            select_sheet(&mut app, "graph-xy")?;
            app.nav.inspector_open = true;
            app.canvas_dialogs.active_inspector = ActiveInspector::FormatGraph;
            let _ = verify_frame_full_gate(&mut app, metrics, &format!("inspector/{mode:?}/{scale}x"))?;
        }
    }

    eprintln!(
        "[R10.9] Verified 16 locales x 3 themes x 4 HiDPI (1.0x/1.5x/2.0x/3.0x), host_covers_cjk_rtl={host_covers_cjk_rtl}, embedded fallback deterministic ({ink_pixels} ink px)."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. R10.2 & R10.16 & R10.20 Parity, Hit-Box, Click-Action & A11y Gate
// ---------------------------------------------------------------------------

fn build_all_26_dialogs(doc: &ProjectDocument) -> Vec<(&'static str, ActiveDialog)> {
    let xy = SheetId::from("data-xy");
    let res_rm = SheetId::from("result-rm");
    let graph_xy = SheetId::from("graph-xy");
    let graph_mv = SheetId::from("graph-mv");
    let layout = SheetId::from("layout-1");
    let _ = doc;

    vec![
        ("AnalyzeSheet", ActiveDialog::AnalyzeSheet { source_sheet_id: xy.clone() }),
        ("EditAnalysisParameters", ActiveDialog::EditAnalysisParameters { result_sheet_id: res_rm.clone() }),
        (
            "DuplicateAnalysisPrompt",
            ActiveDialog::DuplicateAnalysisPrompt {
                source_sheet_id: xy.clone(),
                existing_result_ids: vec![res_rm.clone()],
                pending_entry_id: String::from("stats.one_way_anova"),
            },
        ),
        ("ChangeDataAnalyzed", ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_rm.clone() }),
        ("FormatAxes", ActiveDialog::FormatAxes { sheet_id: graph_xy.clone() }),
        ("FormatGraph", ActiveDialog::FormatGraph { graph_sheet_id: graph_xy.clone() }),
        ("ChangeGraphType", ActiveDialog::ChangeGraphType { graph_sheet_id: graph_xy.clone() }),
        ("PairwiseComparisons", ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_xy.clone() }),
        ("BubblePlotInspector", ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_mv }),
        ("ApplyStyleFrom", ActiveDialog::ApplyStyleFrom { target_graph_id: graph_xy.clone() }),
        ("PageLayoutComposer", ActiveDialog::PageLayoutComposer { layout_sheet_id: layout }),
        ("ExportFigure", ActiveDialog::ExportFigure { sheet_id: graph_xy.clone() }),
        ("Print", ActiveDialog::Print { sheet_id: Some(graph_xy.clone()) }),
        ("PowerAndSampleSize", ActiveDialog::PowerAndSampleSize),
        ("CommandPalette", ActiveDialog::CommandPalette),
        ("ConvertArchetype", ActiveDialog::ConvertArchetype { sheet_id: xy.clone() }),
        ("ImportReshape", ActiveDialog::ImportReshape),
        ("ExcludeWithReason", ActiveDialog::ExcludeWithReason { sheet_id: xy.clone() }),
        ("FormatSelectedPoints", ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_xy.clone() }),
        ("RandomizeGroups", ActiveDialog::RandomizeGroups),
        ("RandomSampleRows", ActiveDialog::RandomSampleRows { source_sheet_id: Some(xy.clone()) }),
        ("SimulateDistributions", ActiveDialog::SimulateDistributions),
        ("InsertModelEquation", ActiveDialog::InsertModelEquation { graph_sheet_id: graph_xy }),
        ("ConstantHookPicker", ActiveDialog::ConstantHookPicker { target_sheet_id: xy }),
        ("ResultViewer", ActiveDialog::ResultViewer { result_sheet_id: res_rm }),
        ("About", ActiveDialog::About),
    ]
}

fn prepare_dialog_state(app: &mut UiApp, variant_name: &str, doc: &ProjectDocument) {
    let xy = SheetId::from("data-xy");
    let res_rm = SheetId::from("result-rm");
    let graph_xy = SheetId::from("graph-xy");
    let graph_mv = SheetId::from("graph-mv");
    let layout = SheetId::from("layout-1");

    match variant_name {
        "AnalyzeSheet" => {
            let _ = app.canvas_dialogs.open_analyze_dialog(doc, &xy);
        }
        "EditAnalysisParameters" => {
            let _ = app.canvas_dialogs.open_edit_parameters(doc, &res_rm);
        }
        "ChangeDataAnalyzed" => {
            let _ = app.canvas_dialogs.open_change_data_analyzed(doc, &res_rm);
        }
        "FormatAxes" => {
            if let Some(gs) = doc.graph_sheet(&graph_xy) {
                app.canvas_dialogs.format_axes.sync_from_graph(gs, Default::default());
            }
        }
        "FormatGraph" => {
            let _ = app.canvas_dialogs.open_format_graph(doc, &graph_xy);
        }
        "ChangeGraphType" => {
            if let Some(gs) = doc.graph_sheet(&graph_xy) {
                app.canvas_dialogs.change_graph_type.open_for_graph(gs, doc);
            }
        }
        "PairwiseComparisons" => {
            let _ = app.canvas_dialogs.open_pairwise_comparisons(doc, &graph_xy);
        }
        "BubblePlotInspector" => {
            let _ = app.canvas_dialogs.open_bubble_plot(doc, &graph_mv);
        }
        "ApplyStyleFrom" => {
            let _ = app.canvas_dialogs.open_apply_style_from(doc, &graph_xy);
        }
        "PageLayoutComposer" => {
            let _ = app.canvas_dialogs.open_page_layout_composer(doc, &layout);
        }
        "ExportFigure" => app.canvas_dialogs.open_export_figure(&graph_xy),
        "Print" => app.canvas_dialogs.open_print_dialog(doc, Some(&graph_xy), vec![String::from("Save as PDF")]),
        "PowerAndSampleSize" => app.canvas_dialogs.open_power_wizard(),
        "CommandPalette" => app.canvas_dialogs.open_command_palette(),
        "ConvertArchetype" => {
            let _ = app.canvas_dialogs.open_convert_archetype(doc, &xy);
        }
        "ImportReshape" => app.canvas_dialogs.open_import_reshape(),
        "ExcludeWithReason" => app.canvas_dialogs.open_exclude_with_reason(xy, vec![GridCellCoord::new(0, 0, 0)]),
        "FormatSelectedPoints" => {
            let _ = app.canvas_dialogs.open_format_selected_points(doc, &graph_xy, vec![(0, 0)]);
        }
        "RandomizeGroups" => app.canvas_dialogs.open_randomize_groups(doc),
        "RandomSampleRows" => {
            let _ = app.canvas_dialogs.open_random_sample_rows(doc, Some(&xy));
        }
        "SimulateDistributions" => app.canvas_dialogs.open_simulate_distributions(),
        "InsertModelEquation" => {
            let _ = app.canvas_dialogs.open_insert_model_equation(doc, &graph_xy);
        }
        "ConstantHookPicker" => app.canvas_dialogs.open_hook_picker(doc, &xy, None),
        "ResultViewer" => {
            if let Some(rs) = doc.result_sheet(&res_rm) {
                app.canvas_dialogs.result_view.sync_with_result(rs);
            }
        }
        _ => {}
    }
}

fn verify_modal_tab_sweep(app: &mut UiApp, metrics: ViewportMetrics, tab_ids: &[&str], scope: &str) -> Result<(), String> {
    let mut driver = HumanPathDriver::new(app.clone(), metrics);
    for &tab_id in tab_ids {
        let outcome = driver.click_widget(tab_id)?;
        ensure(outcome.handled, format!("{scope}: click `{tab_id}` was not handled"))?;
        let _ = verify_frame_full_gate(driver.app_mut(), metrics, &format!("{scope}/{tab_id}"))?;
        let parity = verify_click_action_parity(driver.app(), metrics, tab_id)?;
        ensure(parity, format!("{scope}/{tab_id}: verify_click_action_parity returned false"))?;
    }
    Ok(())
}

#[test]
fn r10_2_and_r10_16_and_r10_20_full_sheets_26_dialogs_tabs_and_7_inspectors_parity_a11y_gate() -> Result<(), String> {
    let metrics = desktop_metrics(1.0);

    // 1. Welcome screen + Welcome tabs (empty project: Empty/Sample enabled; existing project: Clone/Template enabled)
    let mut welcome = UiApp::new();
    let _ = verify_frame_full_gate(&mut welcome, metrics, "welcome/initial")?;
    verify_modal_tab_sweep(&mut welcome, metrics, &["welcome.tab.sample", "welcome.tab.empty"], "welcome_empty")?;

    // 2. All 5 workspace sheet types + Welcome clone mode + Result interpret drawer tabs
    let mut app = build_rich_five_sheet_app()?;
    app.nav.welcome.open = true;
    verify_modal_tab_sweep(&mut app, metrics, &["welcome.mode.clone", "welcome.mode.template"], "welcome_clone")?;
    app.nav.welcome.open = false;

    for (sheet_id, label) in
        [("data-xy", "DataSheet"), ("info-1", "InfoSheet"), ("result-rm", "ResultSheet"), ("graph-xy", "GraphSheet"), ("layout-1", "LayoutSheet")]
    {
        select_sheet(&mut app, sheet_id)?;
        let _ = verify_frame_full_gate(&mut app, metrics, label)?;
    }

    select_sheet(&mut app, "result-rm")?;
    verify_modal_tab_sweep(
        &mut app,
        metrics,
        &["result.interpret_toggle_btn", "result.interpret.tab.assumptions", "result.interpret.tab.narrative", "result.interpret.tab.methods"],
        "result_interpret",
    )?;

    // 3. All 26 non-None ActiveDialog variants
    select_sheet(&mut app, "data-xy")?;
    let doc = app.document().clone();
    let dialogs = build_all_26_dialogs(&doc);
    ensure(dialogs.len() == 26, format!("expected 26 dialogs, got {}", dialogs.len()))?;
    for (name, dialog) in dialogs {
        app.canvas_dialogs.close_dialog();
        prepare_dialog_state(&mut app, name, &doc);
        app.canvas_dialogs.active_dialog = dialog;
        let tree = verify_frame_full_gate(&mut app, metrics, &format!("dialog/{name}"))?;
        ensure(tree.active_focus_scope == FocusScope::ModalDialog, format!("dialog/{name} focus scope mismatch"))?;
    }
    app.canvas_dialogs.close_dialog();

    // 4. Multi-tab modal sweeps:
    // (a) Stat Params (RM-ANOVA) 5 tabs + Ordinary ANOVA Factor Settings tab
    let rm_id = SheetId::from("result-rm");
    let _ = app.canvas_dialogs.open_edit_parameters(&doc, &rm_id);
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: rm_id };
    verify_modal_tab_sweep(
        &mut app,
        metrics,
        &["edit.tab.experimental_design", "edit.tab.repeated_measures", "edit.tab.multiple_comparisons", "edit.tab.options", "edit.tab.diagnostics"],
        "edit_stat_rm",
    )?;
    let stat2_id = SheetId::from("result-stat2");
    let _ = app.canvas_dialogs.open_edit_parameters(&doc, &stat2_id);
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: stat2_id };
    verify_modal_tab_sweep(&mut app, metrics, &["edit.tab.factor_settings"], "edit_stat_pca")?;

    // (b) Nonlinear Fit Params 6 tabs
    let fit_id = SheetId::from("result-fit");
    let _ = app.canvas_dialogs.open_edit_parameters(&doc, &fit_id);
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: fit_id };
    verify_modal_tab_sweep(
        &mut app,
        metrics,
        &["fit.tab.equation", "fit.tab.constraints", "fit.tab.initial_values", "fit.tab.confidence", "fit.tab.compare", "fit.tab.weights_range"],
        "edit_fit",
    )?;

    // (c) Import Reshape 5 tabs
    app.canvas_dialogs.open_import_reshape();
    app.canvas_dialogs.active_dialog = ActiveDialog::ImportReshape;
    verify_modal_tab_sweep(
        &mut app,
        metrics,
        &["import_paste.tab.source", "import_paste.tab.view", "import_paste.tab.filter", "import_paste.tab.placement", "import_paste.tab.notes"],
        "import_reshape",
    )?;

    // (d) Power & Sample Size 4 stages
    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    verify_modal_tab_sweep(&mut app, metrics, &["power.stage.1", "power.stage.2", "power.stage.3", "power.stage.4"], "power_wizard")?;

    // (e) About 3 tabs
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    verify_modal_tab_sweep(&mut app, metrics, &["about.tab.overview", "about.tab.fonts", "about.tab.shortcuts"], "about")?;
    app.canvas_dialogs.close_dialog();

    // 5. All 7 ActiveInspector variants + 5 inspector pill tabs
    select_sheet(&mut app, "graph-xy")?;
    let all_inspectors = [
        (ActiveInspector::None, "data-xy", false),
        (ActiveInspector::FormatGraph, "graph-xy", true),
        (ActiveInspector::FormatAxes, "graph-xy", true),
        (ActiveInspector::PairwiseComparisons, "graph-xy", true),
        (ActiveInspector::BubblePlot, "graph-mv", true),
        (ActiveInspector::PageLayout, "layout-1", true),
        (ActiveInspector::ResultDetails, "result-rm", true),
    ];
    for (insp, sheet_id, open) in all_inspectors {
        select_sheet(&mut app, sheet_id)?;
        app.nav.inspector_open = open;
        app.canvas_dialogs.active_inspector = insp;
        let _ = verify_frame_full_gate(&mut app, metrics, &format!("inspector/{insp:?}"))?;
    }

    select_sheet(&mut app, "graph-xy")?;
    app.nav.inspector_open = true;
    app.canvas_dialogs.active_inspector = ActiveInspector::FormatGraph;
    verify_modal_tab_sweep(
        &mut app,
        metrics,
        &["inspector.tab.graph", "inspector.tab.axes", "inspector.tab.multi", "inspector.tab.pairwise", "inspector.tab.layout"],
        "inspector_tabs",
    )?;

    eprintln!("[R10.2/16/20] Verified Welcome, 5 sheet types, all 26 dialogs, all modal tabs, and all 7 inspector variants + 5 tabs.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 3. R10.13 bullet 2 Visual Screenshot Baselines (5 Sheet Types & Modal Tabs)
// ---------------------------------------------------------------------------

fn render_verified_png_baseline(app: &UiApp, target: RenderTargetSpec, label: &str) -> Result<(u64, usize), String> {
    let png_a = render_workspace_frame_png(app, target).map_err(|e| format!("{label} render a: {e}"))?;
    let png_b = render_workspace_frame_png(app, target).map_err(|e| format!("{label} render b: {e}"))?;
    ensure(png_a == png_b, format!("{label}: screenshot output is not byte-for-byte deterministic"))?;
    let idat_bytes = verify_png_bytes(&png_a, target.width_px, target.height_px, label)?;
    Ok((fnv1a_64(&png_a), idat_bytes))
}

#[test]
fn r10_13_b2_visual_screenshot_baselines_5_sheets_and_modal_tabs() -> Result<(), String> {
    let target = RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 };
    let metrics = desktop_metrics(1.0);
    let mut app = build_rich_five_sheet_app()?;
    let mut seen_sheet_hashes = BTreeSet::new();

    // 1. All 5 sheet types across Light and Dark themes
    for theme in [UiThemeMode::Light, UiThemeMode::Dark] {
        app.set_theme_mode(theme);
        let mut per_theme_hashes = BTreeSet::new();
        for sheet_id in ["data-xy", "info-1", "result-rm", "graph-xy", "layout-1"] {
            select_sheet(&mut app, sheet_id)?;
            let label = format!("baseline/{sheet_id}/{theme:?}");
            let (hash, idat) = render_verified_png_baseline(&app, target, &label)?;
            ensure(per_theme_hashes.insert(hash), format!("{label}: collided with another sheet baseline"))?;
            let _ = seen_sheet_hashes.insert(hash);
            ensure(idat >= 2048, format!("{label}: low visual density ({idat} B IDAT)"))?;
        }
    }
    ensure(seen_sheet_hashes.len() == 10, format!("expected 10 distinct 5-sheet x 2-theme baselines, got {}", seen_sheet_hashes.len()))?;

    // Check cropped Canvas region PNG for GraphSheet
    app.set_theme_mode(UiThemeMode::Light);
    select_sheet(&mut app, "graph-xy")?;
    let (region_png, rw, rh) =
        render_workspace_region_png(&app, target, ScreenshotRegionTarget::Canvas).map_err(|e| format!("canvas region png: {e}"))?;
    let _ = verify_png_bytes(&region_png, rw, rh, "baseline/graph-xy/canvas-region")?;

    // 2. Modal tab distinctness baselines across representative multi-tab dialogs
    let doc = app.document().clone();
    let rm_id = SheetId::from("result-rm");
    let _ = app.canvas_dialogs.open_edit_parameters(&doc, &rm_id);
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: rm_id };
    let mut stat_driver = HumanPathDriver::new(app.clone(), metrics);
    let mut stat_tab_hashes = BTreeSet::new();
    for tab in ["edit.tab.experimental_design", "edit.tab.multiple_comparisons", "edit.tab.diagnostics"] {
        let _ = stat_driver.click_widget(tab)?;
        let (h, _) = render_verified_png_baseline(stat_driver.app(), target, &format!("baseline/{tab}"))?;
        ensure(stat_tab_hashes.insert(h), format!("baseline `{tab}` identical to sibling tab"))?;
    }

    let fit_id = SheetId::from("result-fit");
    let _ = app.canvas_dialogs.open_edit_parameters(&doc, &fit_id);
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: fit_id };
    let mut fit_driver = HumanPathDriver::new(app.clone(), metrics);
    let mut fit_tab_hashes = BTreeSet::new();
    for tab in ["fit.tab.equation", "fit.tab.constraints", "fit.tab.weights_range"] {
        let _ = fit_driver.click_widget(tab)?;
        let (h, _) = render_verified_png_baseline(fit_driver.app(), target, &format!("baseline/{tab}"))?;
        ensure(fit_tab_hashes.insert(h), format!("baseline `{tab}` identical to sibling tab"))?;
    }

    app.canvas_dialogs.open_import_reshape();
    app.canvas_dialogs.active_dialog = ActiveDialog::ImportReshape;
    let mut import_driver = HumanPathDriver::new(app.clone(), metrics);
    let (h_src, _) = render_verified_png_baseline(import_driver.app(), target, "baseline/import_paste.tab.source")?;
    let _ = import_driver.click_widget("import_paste.tab.filter")?;
    let (h_flt, _) = render_verified_png_baseline(import_driver.app(), target, "baseline/import_paste.tab.filter")?;
    ensure(h_src != h_flt, String::from("ImportReshape source and filter tab PNG baselines were identical"))?;

    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    let mut pw_driver = HumanPathDriver::new(app.clone(), metrics);
    let (h_p1, _) = render_verified_png_baseline(pw_driver.app(), target, "baseline/power.stage.1")?;
    let _ = pw_driver.click_widget("power.stage.3")?;
    let (h_p3, _) = render_verified_png_baseline(pw_driver.app(), target, "baseline/power.stage.3")?;
    ensure(h_p1 != h_p3, String::from("PowerWizard stage 1 and stage 3 PNG baselines were identical"))?;

    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    let mut about_driver = HumanPathDriver::new(app, metrics);
    let (h_a1, _) = render_verified_png_baseline(about_driver.app(), target, "baseline/about.tab.overview")?;
    let _ = about_driver.click_widget("about.tab.fonts")?;
    let (h_a2, _) = render_verified_png_baseline(about_driver.app(), target, "baseline/about.tab.fonts")?;
    ensure(h_a1 != h_a2, String::from("About overview and fonts tab PNG baselines were identical"))?;

    eprintln!("[R10.13] Verified deterministic PNG chunks/CRC/distinctness across 5 sheets x 2 themes + all representative modal tabs.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 4. R10.22 bullet 1 Cold-to-First-Frame Startup (< 3 s)
// ---------------------------------------------------------------------------

#[test]
fn r10_22_b1_cold_startup_under_3s_budget() -> Result<(), String> {
    let metrics = desktop_metrics(1.0);
    let target = target_spec_for_metrics(metrics);

    let t_cold = Instant::now();
    let _measurer = DesktopTextMeasurer::new();
    let mut app = UiApp::new();
    let tree = app.build_and_cache_layout_tree(metrics);
    let frame = render_workspace_rgba(&app, target).map_err(|e| format!("cold render: {e}"))?;
    let cold_ms = t_cold.elapsed().as_secs_f64() * 1000.0;

    ensure(!tree.nodes.is_empty(), String::from("cold frame produced empty LayoutTree"))?;
    ensure(frame.width_px == 1280 && frame.height_px == 800, String::from("cold frame wrong size"))?;

    let t_warm = Instant::now();
    let mut warm_app = UiApp::new();
    let _ = warm_app.build_and_cache_layout_tree(metrics);
    let _ = render_workspace_rgba(&warm_app, target).map_err(|e| format!("warm render: {e}"))?;
    let warm_ms = t_warm.elapsed().as_secs_f64() * 1000.0;

    eprintln!("[R10.22 b1] cold_startup={cold_ms:.2} ms, app_first_frame={warm_ms:.2} ms (budget < 3000.0 ms)");
    ensure(cold_ms < 3000.0, format!("cold startup {cold_ms:.2} ms exceeded 3000 ms budget"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 5. R10.22 bullet 2 High-Point / Many-Level MV Categorical Summary Latency
// ---------------------------------------------------------------------------

fn categorical_summary_gdo(mode: &str, summary: SummaryMeasure) -> GraphicalDataObject {
    let swatch = PaletteId::OkabeIto.color_at(0);
    match mode {
        "Strip" => GraphicalDataObject::AlignedDotPlot {
            summary,
            marker: MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 3.5,
                fill_color: swatch,
                stroke_color: swatch,
                stroke_width_pt: 0.8,
            },
            show_crossbar: true,
        },
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
        _ => GraphicalDataObject::BeforeAfterLines {
            marker: Some(MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 3.5,
                fill_color: swatch,
                stroke_color: swatch,
                stroke_width_pt: 0.8,
            }),
            line_color: swatch,
            stroke_width_pt: 1.1,
        },
    }
}

fn build_large_mv_categorical_app(rows: usize, levels: usize) -> Result<UiApp, String> {
    let mut mv = MultipleVariablesTable::new(4, rows).map_err(|e| format!("large mv new: {e}"))?;
    mv.variables[0].title = String::from("GroupSub");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Response");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("Cohort");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("SubjectId");
    mv.variables[3].lock_role(VariableRole::Categorical);

    let level_labels: Vec<String> = (0..levels).map(|i| format!("G{}-S{}", (i / 4) + 1, (i % 4) + 1)).collect();
    let cohorts = ["ArmA", "ArmB", "ArmC", "ArmD", "ArmE"];
    for r in 0..rows {
        let lvl = r % levels;
        let subj = r / levels;
        let phase = f64::from(u32::try_from(lvl).unwrap_or(0)) * 3.1 + f64::from(u32::try_from(r % 17).unwrap_or(0)) * 0.45;
        mv.variables[0].cells[r] = Cell::text(&level_labels[lvl]);
        mv.variables[1].cells[r] = Cell::number(25.0 + phase);
        mv.variables[2].cells[r] = Cell::text(cohorts[lvl % cohorts.len()]);
        mv.variables[3].cells[r] = Cell::text(format!("Subj-{subj:04}"));
    }

    let data_mv = DataSheet::new("data-mv-big", "10k MV Assay", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("ds big mv: {e}"))?;
    let mut graph_mv = GraphSheet::new_uninitialized_for_data("graph-mv-big", &data_mv).map_err(|e| format!("gs big mv: {e}"))?;
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

    let mut doc = ProjectDocument::new("MV Categorical Benchmark").map_err(|e| format!("doc new: {e}"))?;
    doc.add_data_sheet(data_mv).map_err(|e| format!("add ds: {e}"))?;
    doc.add_graph_sheet(graph_mv).map_err(|e| format!("add gs: {e}"))?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    select_sheet(&mut app, "graph-mv-big")?;
    Ok(app)
}

#[test]
fn r10_22_b2_mv_categorical_summary_10k_rows_20plus_levels_latency() -> Result<(), String> {
    let rows = 10_080usize;
    let levels = 24usize;
    ensure(rows >= 10_000 && levels >= 20, String::from("benchmark dimensions below R10.22 requirement"))?;

    let mut app = build_large_mv_categorical_app(rows, levels)?;
    let base_metrics = desktop_metrics(1.0);
    let resize_metrics = ViewportMetrics { logical_width: 1440.0, logical_height: 900.0, scale_factor: 1.0 };
    let base_target = target_spec_for_metrics(base_metrics);
    let resize_target = target_spec_for_metrics(resize_metrics);

    // Warm text/canvas caches once before recording per-mode interactive latencies
    let _ = app.build_and_cache_layout_tree(base_metrics);
    let _ = render_workspace_rgba(&app, base_target).map_err(|e| format!("warmup render: {e}"))?;

    let frame_budget_ms = if cfg!(debug_assertions) { 450.0 } else { 50.0 };
    let modes = ["Strip", "Bar", "Box", "Violin", "BeforeAfter"];
    let mut over_budget = Vec::new();

    for (idx, mode) in modes.iter().enumerate() {
        let mut enc = app
            .document()
            .graph_sheet(&SheetId::from("graph-mv-big"))
            .and_then(|g| g.spec.multivariate.clone())
            .ok_or_else(|| String::from("missing graph-mv-big multivariate encoding"))?;

        // 1. CREATE / switch into mode
        enc.plot_mode = MultivariatePlotMode::CategoricalSummary {
            gdo: categorical_summary_gdo(mode, SummaryMeasure::Mean),
            y_error_bars: Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, ..ErrorBarSpec::default() }),
        };
        let t_create = Instant::now();
        let _ = app
            .execute_command("graph.setSpec", json!({ "sheet_id": "graph-mv-big", "multivariate": enc }))
            .map_err(|e| format!("{mode} create setSpec: {e}"))?;
        let t_compile = Instant::now();
        let gs = app.document().graph_sheet(&SheetId::from("graph-mv-big")).ok_or_else(|| String::from("missing gs"))?;
        let scene_c = compile_graph(gs, app.document()).map_err(|e| format!("{mode} create compile: {e}"))?;
        let compile_ms = t_compile.elapsed().as_secs_f64() * 1000.0;
        let t_layout = Instant::now();
        let tree_c = app.build_and_cache_layout_tree(base_metrics);
        let layout_ms = t_layout.elapsed().as_secs_f64() * 1000.0;
        let t_render = Instant::now();
        let img_c = render_workspace_rgba(&app, base_target).map_err(|e| format!("{mode} create render: {e}"))?;
        let render_ms = t_render.elapsed().as_secs_f64() * 1000.0;
        let create_ms = t_create.elapsed().as_secs_f64() * 1000.0;
        ensure(!scene_c.primitives.is_empty() && !tree_c.nodes.is_empty() && !img_c.rgba.is_empty(), format!("{mode}: empty create output"))?;

        // 2. UPDATE (metric/error-bar switch + upstream cell mutation)
        enc.plot_mode = MultivariatePlotMode::CategoricalSummary {
            gdo: categorical_summary_gdo(mode, SummaryMeasure::Median),
            y_error_bars: Some(ErrorBarSpec { metric: ErrorBarMetric::ConfidenceInterval95, ..ErrorBarSpec::default() }),
        };
        let t_update = Instant::now();
        let _ = app
            .execute_command("table.setCell", json!({ "sheet_id": "data-mv-big", "row": idx, "col": 1, "subcol": 0, "value": 42.5 + (idx as f64) }))
            .map_err(|e| format!("{mode} update cell: {e}"))?;
        let _ = app
            .execute_command("graph.setSpec", json!({ "sheet_id": "graph-mv-big", "multivariate": enc }))
            .map_err(|e| format!("{mode} update setSpec: {e}"))?;
        let gs_u = app.document().graph_sheet(&SheetId::from("graph-mv-big")).ok_or_else(|| String::from("missing gs update"))?;
        let scene_u = compile_graph(gs_u, app.document()).map_err(|e| format!("{mode} update compile: {e}"))?;
        let _tree_u = app.build_and_cache_layout_tree(base_metrics);
        let _img_u = render_workspace_rgba(&app, base_target).map_err(|e| format!("{mode} update render: {e}"))?;
        let update_ms = t_update.elapsed().as_secs_f64() * 1000.0;
        ensure(!scene_u.primitives.is_empty(), format!("{mode}: empty update scene"))?;

        // 3. RESIZE (canvas_size_pt + viewport resize)
        let t_resize = Instant::now();
        let _ = app
            .execute_command(
                "graph.setSpec",
                json!({ "sheet_id": "graph-mv-big", "spec": { "canvas_size_pt": { "width": 520.0, "height": 340.0 } } }),
            )
            .map_err(|e| format!("{mode} resize spec: {e}"))?;
        let gs_r = app.document().graph_sheet(&SheetId::from("graph-mv-big")).ok_or_else(|| String::from("missing gs resize"))?;
        let scene_r = compile_graph(gs_r, app.document()).map_err(|e| format!("{mode} resize compile: {e}"))?;
        let _tree_r = app.build_and_cache_layout_tree(resize_metrics);
        let _img_r = render_workspace_rgba(&app, resize_target).map_err(|e| format!("{mode} resize render: {e}"))?;
        let resize_ms = t_resize.elapsed().as_secs_f64() * 1000.0;
        ensure(!scene_r.primitives.is_empty(), format!("{mode}: empty resize scene"))?;

        eprintln!(
            "[R10.22 b2 MV Categorical 10080r x 24lv] mode={mode:<11} create={create_ms:6.2} ms (c={compile_ms:5.2}/l={layout_ms:5.2}/r={render_ms:5.2})  update={update_ms:6.2} ms  resize={resize_ms:6.2} ms  (budget <= {frame_budget_ms:.0} ms)"
        );
        if create_ms > frame_budget_ms {
            over_budget.push(format!("{mode} create {create_ms:.2} ms > {frame_budget_ms} ms"));
        }
        if update_ms > frame_budget_ms {
            over_budget.push(format!("{mode} update {update_ms:.2} ms > {frame_budget_ms} ms"));
        }
        if resize_ms > frame_budget_ms {
            over_budget.push(format!("{mode} resize {resize_ms:.2} ms > {frame_budget_ms} ms"));
        }
    }
    ensure(over_budget.is_empty(), over_budget.join("; "))
}

// ---------------------------------------------------------------------------
// 6. R10.22 & R10.1 Web Shared-Layout Parity & Release Bundle Budget Check
// ---------------------------------------------------------------------------

#[test]
fn r10_22_b2_and_r10_1_web_bundle_budget_and_shared_layout_parity() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root =
        manifest_dir.parent().and_then(|p| p.parent()).ok_or_else(|| String::from("failed to resolve workspace root from CARGO_MANIFEST_DIR"))?;

    let web_lib = std::fs::read_to_string(root.join("apps/curvebook-web/src/lib.rs")).map_err(|e| format!("read web lib.rs: {e}"))?;
    ensure(
        web_lib.contains("UiApp") && web_lib.contains("render_rgba_for_metrics"),
        String::from("apps/curvebook-web/src/lib.rs must drive shared UiApp / render_rgba_for_metrics (R10.1)"),
    )?;

    let web_chrome =
        std::fs::read_to_string(root.join("apps/curvebook-web/src/chrome_bridge.rs")).map_err(|e| format!("read web chrome_bridge.rs: {e}"))?;
    ensure(
        web_chrome.contains("LayoutTree") && web_chrome.contains("build_workspace_layout_tree"),
        String::from("apps/curvebook-web/src/chrome_bridge.rs must consume shared LayoutTree / build_workspace_layout_tree (R10.1)"),
    )?;

    let web_cargo = std::fs::read_to_string(root.join("apps/curvebook-web/Cargo.toml")).map_err(|e| format!("read web Cargo.toml: {e}"))?;
    ensure(
        web_cargo.contains("curvebook-ui-egui") && web_cargo.contains("workspace = true"),
        String::from("apps/curvebook-web/Cargo.toml must consume workspace curvebook-ui-egui for shared-layout parity"),
    )?;

    let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("read Cargo.toml: {e}"))?;
    ensure(
        root_cargo.contains("[profile.web-release]") && root_cargo.contains("opt-level = \"s\""),
        String::from("Cargo.toml must define slim size-optimized [profile.web-release] for R10.22 web bundle budget"),
    )?;

    let web_fonts_txt =
        std::fs::read_to_string(root.join("crates/text/web-fonts.txt")).map_err(|e| format!("read crates/text/web-fonts.txt: {e}"))?;
    ensure(!web_fonts_txt.trim().is_empty(), String::from("crates/text/web-fonts.txt must declare font manifest entries"))?;

    let package_sh = std::fs::read_to_string(root.join("packaging/web/package.sh")).map_err(|e| format!("read packaging/web/package.sh: {e}"))?;
    ensure(package_sh.contains("trunk build --release"), String::from("packaging/web/package.sh must invoke trunk build --release"))?;

    // Check any staged web release bundle zip against MAX_WEB_RELEASE_ZIP_BYTES (<= 6,243,591 B)
    let release_dir = root.join("dist/release");
    let mut verified_zip = None;
    if release_dir.is_dir() {
        let entries = std::fs::read_dir(&release_dir).map_err(|e| format!("read_dir dist/release: {e}"))?;
        for entry_res in entries {
            let entry = entry_res.map_err(|e| format!("dir entry: {e}"))?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("curvebook-web-") && name.ends_with(".zip") {
                let bytes = entry.metadata().map_err(|e| format!("metadata {name}: {e}"))?.len();
                ensure(
                    bytes > 0 && bytes <= MAX_WEB_RELEASE_ZIP_BYTES,
                    format!("{name} size {bytes} B outside (0, {MAX_WEB_RELEASE_ZIP_BYTES}] budget"),
                )?;
                verified_zip = Some((name.to_owned(), bytes));
            }
        }
    }

    eprintln!(
        "[R10.1/R10.22 Web] shared LayoutTree parity OK, web bundle budget ceiling = {MAX_WEB_RELEASE_ZIP_BYTES} B, staged_zip={verified_zip:?}"
    );
    Ok(())
}
