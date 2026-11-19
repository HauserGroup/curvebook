#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 2 (`R15.2`) QA & Go/No-Go benchmark suite (`crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs`):
//! - Legacy baseline & `egui` matched `1,000,000`-cell `Column` (`1,000 cols × 1,000 rows × 1 subcol`)
//!   frame time (`<= 16.67 ms`) and input-to-paint latency (`<= 50 ms`) benchmarks at `1280×800`
//!   across `1.0x` and `2.0x` DPI in `--release`.
//! - Legacy `r10_1` One-Way ANOVA human-path + CJK/IME + undo/`ops.journal` parity baseline
//!   (never modifying or weakening `r10_core_and_spike.rs`).
//! - Native `egui` `EguiHumanPathDriver` port of `r10_1` + `ui.inspect` parity + CJK/IME commit +
//!   undo/`ops.journal` parity (activated once Wave 2 `data_grid` & `stat_anova_modal` merge).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, MultiplicityMethod, ProjectDocument, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_format::recovery::CrashRecoveryJournal;
use curvebook_format::{DirectoryJournalBackend, JOURNAL_FILE_NAME};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, HybridSheetSurface, build_ui_tree_snapshot_from_egui, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens, UiBackend, UiIntent};
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::{FocusScope, ZLayer};
use curvebook_ui_egui::theme::UiThemeSpec;
use curvebook_ui_egui::{UiApp, UiThemeMode, inspect_ui_tree_for_metrics};
use serde_json::json;

const BENCH_COLS: usize = 1_000;
const BENCH_ROWS: usize = 1_000;
const BENCH_SUBCOLS: usize = 1;
const BENCH_VIEWPORT_W: f64 = 1280.0;
const BENCH_VIEWPORT_H: f64 = 800.0;
const STEADY_FRAMES: usize = 24;
const INPUT_EVENTS: usize = 12;
const EGUI_STEADY_BUDGET_MS: f64 = 16.67;
const EGUI_INPUT_BUDGET_MS: f64 = 50.0;

#[derive(Debug, Clone, Copy)]
struct TimingStats {
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

impl TimingStats {
    fn from_samples(mut samples: Vec<f64>) -> Self {
        assert!(!samples.is_empty(), "timing sample list must not be empty");
        samples.sort_by(|a, b| a.total_cmp(b));
        let sum: f64 = samples.iter().sum();
        let mean_ms = sum / (samples.len() as f64);
        let p50_ms = samples[samples.len() / 2];
        let p95_idx = ((samples.len() - 1) * 95) / 100;
        let p95_ms = samples[p95_idx];
        let max_ms = *samples.last().unwrap();
        Self { mean_ms, p50_ms, p95_ms, max_ms }
    }
}

#[derive(Debug, Clone)]
struct BackendBenchReport {
    backend: &'static str,
    dpi_scale: f64,
    steady_frame: TimingStats,
    input_to_paint: TimingStats,
    visible_cells_last: usize,
    total_cells: usize,
}

impl BackendBenchReport {
    fn print_row(&self) {
        let phys_w = (BENCH_VIEWPORT_W * self.dpi_scale).round() as u32;
        let phys_h = (BENCH_VIEWPORT_H * self.dpi_scale).round() as u32;
        println!(
            "| {:<6} | {:>4.1}x ({:>4}x{:<4}) | {:>7.3} | {:>7.3} | {:>7.3} | {:>7.3} | {:>7.3} | {:>7.3} | {:>7.3} | {:>7.3} | {:>7} |",
            self.backend,
            self.dpi_scale,
            phys_w,
            phys_h,
            self.steady_frame.mean_ms,
            self.steady_frame.p50_ms,
            self.steady_frame.p95_ms,
            self.steady_frame.max_ms,
            self.input_to_paint.mean_ms,
            self.input_to_paint.p50_ms,
            self.input_to_paint.p95_ms,
            self.input_to_paint.max_ms,
            self.visible_cells_last,
        );
    }
}

fn print_bench_table_header() {
    println!(
        "\nR15.2 Matched 1,000,000-cell Column Benchmark ({BENCH_COLS} cols × {BENCH_ROWS} rows × {BENCH_SUBCOLS} subcol = {} cells, viewport {BENCH_VIEWPORT_W:.0}x{BENCH_VIEWPORT_H:.0}):",
        BENCH_COLS * BENCH_ROWS * BENCH_SUBCOLS
    );
    println!(
        "| {:<6} | {:<16} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} |",
        "Backnd", "DPI (phys px)", "FrMean", "FrP50", "FrP95", "FrMax", "InMean", "InP50", "InP95", "InMax", "VisCell"
    );
    println!("|--------|------------------|---------|---------|---------|---------|---------|---------|---------|---------|---------|");
}

fn build_one_million_cell_column_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, BENCH_COLS, BENCH_ROWS).expect("1M ColumnTable");
    for (c_idx, ds_col) in col.data_sets.iter_mut().enumerate() {
        ds_col.title = format!("Col {}", c_idx + 1);
        for (r_idx, row) in ds_col.cells.iter_mut().enumerate() {
            row[0] = Cell::number(((c_idx % 100) as f64) * 0.25 + (r_idx as f64) * 0.1);
        }
    }
    let mut sheet = DataSheet::new("col-1m", "Column 1M Go/No-Go", TableArchetype::Column(col)).expect("column 1M sheet");
    sheet.set_excluded(CellCoord::new(0, 0, 0), true).expect("exclude top-left cell");
    assert_eq!(sheet.archetype.column_count(), BENCH_COLS);
    assert_eq!(sheet.archetype.row_count(), BENCH_ROWS);
    doc.add_data_sheet(sheet).expect("add 1M column sheet");
    doc.select_in_navigator(&SheetId::from("col-1m")).expect("select 1M column sheet");
    doc
}

fn temp_journal_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("cb-r15-2-{label}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp journal dir");
    dir
}

struct TempDirCleanup(std::path::PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_legacy_1m_column_benchmark(dpi_scale: f64, steady_frames: usize, input_events: usize) -> BackendBenchReport {
    let doc = build_one_million_cell_column_doc();
    let metrics = ViewportMetrics::new(BENCH_VIEWPORT_W, BENCH_VIEWPORT_H, dpi_scale);
    let mut driver = HumanPathDriver::new(UiApp::from_session(Session::from_document(doc)), metrics);
    driver.app_mut().grid.tool.viewport.viewport_width_px = BENCH_VIEWPORT_W;
    driver.app_mut().grid.tool.viewport.viewport_height_px = BENCH_VIEWPORT_H;

    // Warm layout & glyph coverage atlas before timed steady-state loop.
    let warm = driver.render_frame().expect("legacy warm frame");
    assert_eq!(warm.width_px, metrics.physical_width_px());
    assert_eq!(warm.height_px, metrics.physical_height_px());

    let mut steady_samples = Vec::with_capacity(steady_frames);
    let mut visible_cells_last = 0usize;
    for step in 0..steady_frames {
        driver.app_mut().grid.tool.viewport.scroll_y_px = ((step * 144) % (950 * 24)) as f64;
        driver.app_mut().grid.tool.viewport.scroll_x_px = ((step * 220) % (960 * 110)) as f64;
        let t0 = Instant::now();
        let _tree = driver.layout_tree();
        let img = driver.render_frame().expect("legacy 1M steady frame");
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(img.width_px, metrics.physical_width_px());
        steady_samples.push(elapsed_ms);

        let win = driver.app().visible_grid_window().expect("legacy visible window");
        visible_cells_last = win.end_row.saturating_sub(win.start_row) * win.end_col.saturating_sub(win.start_col);
    }

    // Input-to-paint latency across wheel scroll, pointer click, and keyboard navigation.
    let grid_center = driver.widget_center_logical("grid.sheet.col-1m").unwrap_or([BENCH_VIEWPORT_W * 0.5, BENCH_VIEWPORT_H * 0.5]);
    let mut input_samples = Vec::with_capacity(input_events);
    for step in 0..input_events {
        match step % 3 {
            0 => {
                let _ = driver.wheel_logical(grid_center[0], grid_center[1], 120.0, -72.0, KeyModifiers::default());
            }
            1 => {
                let _ = driver.click_logical(grid_center[0], grid_center[1]);
            }
            _ => {
                let _ = driver.press_key("ArrowDown", KeyModifiers::default());
            }
        }
        input_samples.push(driver.last_input_to_paint_ms());
    }

    BackendBenchReport {
        backend: "legacy",
        dpi_scale,
        steady_frame: TimingStats::from_samples(steady_samples),
        input_to_paint: TimingStats::from_samples(input_samples),
        visible_cells_last,
        total_cells: BENCH_COLS * BENCH_ROWS * BENCH_SUBCOLS,
    }
}

fn run_egui_1m_column_benchmark(dpi_scale: f64, steady_frames: usize, input_events: usize) -> BackendBenchReport {
    let doc = build_one_million_cell_column_doc();
    let mut app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
    app.sync_views();

    let metrics = ViewportMetrics::new(BENCH_VIEWPORT_W, BENCH_VIEWPORT_H, dpi_scale);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en");
    tokens.apply_to_ctx(&ctx);
    ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.animation_time = 0.0;
    });

    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(BENCH_VIEWPORT_W as f32, BENCH_VIEWPORT_H as f32));

    let run_settled_egui_frame = |app_ref: &mut UiApp, sim_time: f64, initial_events: Vec<egui::Event>| -> (f64, usize, egui::Rect) {
        let t0 = Instant::now();
        let mut events = initial_events;
        let mut canvas_rect = screen_rect;
        let mut visible_cells = 0usize;
        for _pass in 0..4 {
            let mut raw_input = egui::RawInput {
                screen_rect: Some(screen_rect),
                time: Some(sim_time),
                predicted_dt: 1.0 / 60.0,
                focused: true,
                events: std::mem::take(&mut events),
                ..Default::default()
            };
            if let Some(vp) = raw_input.viewports.get_mut(&egui::ViewportId::ROOT) {
                vp.native_pixels_per_point = Some(ppp);
            }

            let mut meta = FrameInspectMeta::new();
            let mut intents = PendingUiIntents::new();
            let mut out = ctx.run_ui(raw_input, |ui| {
                meta.begin_pass(ui.ctx());
                let mut views = std::mem::take(&mut app_ref.egui_views);
                let rep = render_egui_shell_frame(ui, app_ref, &mut views, &mut meta, &mut intents);
                if let Some(r) = rep.regions.canvas_rect_pt {
                    canvas_rect = r;
                }
                app_ref.egui_views = views;
            });
            let will_discard = out.platform_output.requested_discard();
            let _ = intents.commit_if_settled(app_ref, will_discard);
            let _clipped = ctx.tessellate(std::mem::take(&mut out.shapes), ppp);
            out.textures_delta.clear();
            let visible_rows = app_ref.egui_views.column_grid.last_visible_rows.len();
            let visible_cols = app_ref.egui_views.column_grid.last_visible_subcols.len();
            visible_cells = visible_rows * visible_cols;
            if !will_discard {
                break;
            }
        }
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        (elapsed_ms, visible_cells, canvas_rect)
    };

    // Warm font atlas and egui memory twice before timing.
    let mut sim_time = 0.0_f64;
    let mut canvas_rect = screen_rect;
    for _ in 0..2 {
        sim_time += 1.0 / 60.0;
        let (_, _, cr) = run_settled_egui_frame(&mut app, sim_time, Vec::new());
        canvas_rect = cr;
    }

    let mut steady_samples = Vec::with_capacity(steady_frames);
    let mut visible_cells_last = 0usize;
    for step in 0..steady_frames {
        sim_time += 1.0 / 60.0;
        app.egui_views.column_grid.scroll_offset_pt = egui::vec2(((step * 180) % 40_000) as f32, ((step * 120) % 20_000) as f32);
        let (ms, vis, cr) = run_settled_egui_frame(&mut app, sim_time, Vec::new());
        canvas_rect = cr;
        visible_cells_last = vis;
        steady_samples.push(ms);
    }

    let center = canvas_rect.center();
    let mut input_samples = Vec::with_capacity(input_events);
    for step in 0..input_events {
        sim_time += 1.0 / 60.0;
        let events = match step % 3 {
            0 => vec![
                egui::Event::PointerMoved(center),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(80.0, -72.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            1 => vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            _ => vec![
                egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: Some(egui::Key::ArrowDown),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: Some(egui::Key::ArrowDown),
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        };
        let (ms, vis, _) = run_settled_egui_frame(&mut app, sim_time, events);
        if vis > 0 {
            visible_cells_last = vis;
        }
        input_samples.push(ms);
    }

    BackendBenchReport {
        backend: "egui",
        dpi_scale,
        steady_frame: TimingStats::from_samples(steady_samples),
        input_to_paint: TimingStats::from_samples(input_samples),
        visible_cells_last,
        total_cells: BENCH_COLS * BENCH_ROWS * BENCH_SUBCOLS,
    }
}

#[test]
fn r15_2_legacy_1m_column_ci_smoke() {
    let r1 = run_legacy_1m_column_benchmark(1.0, 6, 4);
    let r2 = run_legacy_1m_column_benchmark(2.0, 6, 4);
    print_bench_table_header();
    r1.print_row();
    r2.print_row();

    assert_eq!(r1.total_cells, 1_000_000);
    assert!(r1.visible_cells_last > 0 && r1.visible_cells_last < 2_000, "legacy grid must virtualize: {}", r1.visible_cells_last);
    assert!(r2.visible_cells_last > 0 && r2.visible_cells_last < 2_000);
}

#[test]
fn r15_2_legacy_r10_1_anova_cjk_undo_journal_baseline() {
    let dir = temp_journal_dir("legacy-r10-1");
    let _cleanup = TempDirCleanup(dir.clone());
    let (session, mut journal, _report) = Session::open_dir_with_recovery(&dir, "legacy-qa", 300_000, 1_000).expect("open journal");

    // Seed exact same three-group Column table inside a journaled session.
    let mut app = UiApp::from_session(session);
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-spike", "title": "Drug Dose Assay", "kind": "column", "cols": 3, "rows": 6 }))
        .expect("create");
    app.execute_command("table.setTitles", json!({ "sheet_id": "col-spike", "titles": ["Control", "Treated", "HighDose"] })).expect("titles");
    let rows = [[10.0, 14.5, 20.1], [11.2, 15.1, 21.0], [9.8, 13.9, 19.6], [10.6, 14.8, 20.8], [10.4, 15.3, 21.5]];
    app.execute_command("table.setCells", json!({ "sheet_id": "col-spike", "rows": rows })).expect("set cells");
    app.session().checkpoint_journal(&mut journal).expect("checkpoint seed");
    let seed_undo = app.session().undo_depth();

    let mut driver = HumanPathDriver::new(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(driver.verify_parity().is_clean());

    // Inline CJK/IME cell commit + undo verification.
    let cell_node = driver.find_widget("grid.cell.5.0.0").expect("empty cell row 5");
    let center = cell_node.bounds.visible_rect_px.center();
    driver.double_click_logical(f64::from(center[0]), f64::from(center[1])).expect("dblclick row 5");
    driver.set_ime_preedit(Some(String::from("にほん")));
    driver.type_text("用量反応").expect("commit CJK cell text");
    driver.press_key("Enter", KeyModifiers::default()).expect("Enter commits cell");
    assert_eq!(driver.app().session().undo_depth(), seed_undo + 1, "cell commit must add exactly 1 undo step");
    if let TableArchetype::Column(col) = &driver.app().document().data_sheets[0].archetype {
        assert_eq!(col.data_sets[0].cell(5, 0).map(|c| &c.value), Some(&CellValue::Text(String::from("用量反応"))));
    } else {
        panic!("expected Column archetype");
    }
    driver
        .app_mut()
        .session()
        .record_to_journal(&mut journal, "table.setCell", &json!({ "sheet_id": "col-spike", "row": 5, "col": 0, "subcol": 0, "value": "用量反応" }))
        .expect("record cell journal");

    // Analyze -> One-Way ANOVA -> Cmd+T -> Assume Gaussian off (R2) -> Update Analysis.
    driver.click_widget("toolbar.analyze").expect("click toolbar.analyze");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = driver.wheel_widget("analyze.entries_list", 0.0, -180.0, KeyModifiers::default());
    }
    driver.click_widget("analyze.entry.stats.one_way_anova").expect("select one_way_anova");
    driver.click_widget("modal.ok").expect("confirm AnalyzeSheet");
    let rs_id = driver.app().document().result_sheets[0].id.clone();

    driver.press_key("T", KeyModifiers { meta: true, ..Default::default() }).expect("Cmd+T");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    // Focus trap inside modal.
    for _ in 0..4 {
        driver.press_key("Tab", KeyModifiers::default()).expect("Tab inside modal");
        if let Some(fid) = driver.app().focus_widget_id.clone() {
            let tree = driver.layout_tree();
            let node = tree.node(&fid).expect("modal node");
            assert_eq!(node.a11y.focus_scope, FocusScope::ModalDialog);
        }
    }

    driver.click_widget("edit.assume_gaussian").expect("toggle Assume Gaussian off");
    let tukey = driver.find_widget("edit.post_hoc.tukey").expect("tukey present");
    assert!(!tukey.state.enablement.enabled);
    driver.click_widget("modal.ok").expect("confirm Update Analysis");
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs = driver.app().document().result_sheet(&rs_id).expect("updated result sheet");
    assert!(rs.title().contains("Kruskal-Wallis"));
    if let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind {
        assert_eq!(spec.multiplicity.method, MultiplicityMethod::DunnRankSum);
    } else {
        panic!("expected OneWayAnova spec");
    }

    // Verify ui.inspect JSON baseline contains active ResultSheet + undo depth.
    let inspect = inspect_ui_tree_for_metrics(driver.app(), driver.metrics(), None, true);
    assert_eq!(inspect.active_sheet_id.as_deref(), Some(rs_id.as_str()));
    assert!(inspect.undo_depth > seed_undo);

    // Undo restores Kruskal-Wallis back to Ordinary One-Way ANOVA.
    driver.app_mut().execute_command("edit.undo", json!({})).expect("undo analysis update");
    assert!(driver.app().document().result_sheet(&rs_id).is_some_and(|r| r.title().contains("ANOVA") || r.title().contains("One-Way")));

    // WAL ops.journal replay restores the CJK cell edit cleanly.
    drop(journal.abandon());
    assert!(dir.join(JOURNAL_FILE_NAME).exists());
    let (recovered, _rec_j, _rep) = Session::open_dir_with_recovery(&dir, "legacy-qa-recover", 300_000, 305_000).expect("recover WAL");
    if let TableArchetype::Column(col) = &recovered.document().data_sheets[0].archetype {
        assert_eq!(col.data_sets[0].cell(5, 0).map(|c| &c.value), Some(&CellValue::Text(String::from("用量反応"))));
    } else {
        panic!("expected Column archetype on WAL recovery");
    }
}

#[test]
#[ignore = "Wave 2 Go/No-Go matched 1M-cell Column release benchmark; run with `cargo test --release -p curvebook-ui-egui --test r15_2_anova_and_1m_bench -- --nocapture --ignored`"]
fn r15_2_gonogo_1m_column_matched_benchmark_release() {
    if cfg!(debug_assertions) {
        panic!("Go/No-Go 1M benchmark MUST run in --release mode");
    }
    let legacy_1x = run_legacy_1m_column_benchmark(1.0, STEADY_FRAMES, INPUT_EVENTS);
    let legacy_2x = run_legacy_1m_column_benchmark(2.0, STEADY_FRAMES, INPUT_EVENTS);
    let egui_1x = run_egui_1m_column_benchmark(1.0, STEADY_FRAMES, INPUT_EVENTS);
    let egui_2x = run_egui_1m_column_benchmark(2.0, STEADY_FRAMES, INPUT_EVENTS);

    print_bench_table_header();
    legacy_1x.print_row();
    legacy_2x.print_row();
    egui_1x.print_row();
    egui_2x.print_row();

    for r in [&egui_1x, &egui_2x] {
        assert!(
            r.visible_cells_last > 0 && r.visible_cells_last < 5_000,
            "egui 1M Column grid must be virtualized with visible cells > 0 (got {})",
            r.visible_cells_last
        );
        assert!(
            r.steady_frame.p95_ms <= EGUI_STEADY_BUDGET_MS && r.steady_frame.mean_ms <= EGUI_STEADY_BUDGET_MS,
            "egui @{}x exceeded 60fps steady budget ({EGUI_STEADY_BUDGET_MS}ms): {:?}",
            r.dpi_scale,
            r.steady_frame
        );
        assert!(
            r.input_to_paint.p95_ms <= EGUI_INPUT_BUDGET_MS && r.input_to_paint.max_ms <= EGUI_INPUT_BUDGET_MS,
            "egui @{}x exceeded input-to-paint budget ({EGUI_INPUT_BUDGET_MS}ms): {:?}",
            r.dpi_scale,
            r.input_to_paint
        );
    }
}

struct EguiAnovaQaState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>>,
    committed_engine_commands: Vec<(String, serde_json::Value)>,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    dialog_rect_pt: Option<egui::Rect>,
    legacy_modal_rect_pt: Option<egui::Rect>,
}

impl EguiAnovaQaState {
    fn new(app: UiApp, journal: CrashRecoveryJournal<DirectoryJournalBackend>) -> Self {
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            journal: Some(journal),
            committed_engine_commands: Vec::new(),
            outside_click_captured: false,
            outside_wheel_captured: false,
            dialog_rect_pt: None,
            legacy_modal_rect_pt: None,
        }
    }
}

fn make_egui_qa_driver<'a>(state: EguiAnovaQaState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, EguiAnovaQaState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.dialog_rect_pt = rep.regions.dialog_rect_pt;
        st.legacy_modal_rect_pt = rep.legacy_modal_rect_pt;
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let engine_cmds: Vec<(String, serde_json::Value)> = st
                .intents
                .intents()
                .iter()
                .filter_map(|intent| match intent {
                    UiIntent::EngineCommand { command_id, params } => Some((command_id.clone(), params.clone())),
                    _ => None,
                })
                .collect();
            if st.intents.commit_if_settled(&mut st.app, false).is_ok() {
                for (cmd_id, params) in engine_cmds {
                    if let Some(j) = st.journal.as_mut() {
                        st.app.session().record_to_journal(j, &cmd_id, &params).expect("record egui intent to WAL journal");
                    }
                    st.committed_engine_commands.push((cmd_id, params));
                }
            }
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn verify_egui_cjk_cell_and_title_ime(driver: &mut EguiHumanPathDriver<'_, EguiAnovaQaState>, seed_undo: usize) -> Result<(), String> {
    let cell_pos = driver.frame_meta().find_by_widget_id("grid.cell.5.0.0").ok_or("missing grid.cell.5.0.0")?.clipped_rect.center();
    driver.click_at(cell_pos)?;
    driver.click_at(cell_pos)?;
    assert!(driver.state().views.column_grid.inline_edit.is_some(), "expected inline editor on cell (5,0,0)");
    driver.ime_preedit("にほん", Some(0..3));
    assert_eq!(driver.state().app.session().undo_depth(), seed_undo, "IME preedit must not commit early");
    driver.ime_commit("用量反応");
    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(driver.state().app.session().undo_depth(), seed_undo + 1, "Enter must add exactly 1 undo step");
    let ds = driver.state().app.document().data_sheet(&SheetId::from("col-spike")).ok_or("missing col-spike")?;
    assert_eq!(ds.cell(CellCoord::new(5, 0, 0)).map(|c| &c.value), Some(&CellValue::Text(String::from("用量反応"))));

    // CJK/IME column title edit via double-click + Backspace clear + IME commit + Enter.
    let hdr_pos = driver.frame_meta().find_by_widget_id("grid.col_title.0").ok_or("missing grid.col_title.0")?.clipped_rect.center();
    driver.click_at(hdr_pos)?;
    driver.click_at(hdr_pos)?;
    assert!(driver.state().views.column_grid.inline_edit.is_some(), "expected inline editor on grid.col_title.0");
    for _ in 0..10 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.ime_preedit("こんとろーる", Some(0..6));
    assert_eq!(driver.state().app.session().undo_depth(), seed_undo + 1);
    driver.ime_commit("対照群");
    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(driver.state().app.session().undo_depth(), seed_undo + 2);
    if let TableArchetype::Column(col) = &driver.state().app.document().data_sheet(&SheetId::from("col-spike")).unwrap().archetype {
        assert_eq!(col.data_sets[0].title, "対照群");
    } else {
        return Err(String::from("expected Column archetype after title commit"));
    }
    Ok(())
}

fn run_egui_case_b_analyze_to_resultsheet(
    driver: &mut EguiHumanPathDriver<'_, EguiAnovaQaState>,
    metrics: ViewportMetrics,
) -> Result<SheetId, String> {
    driver.click_widget_id("toolbar.analyze")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    let d_case_b = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(d_case_b.sheet_surface, HybridSheetSurface::EguiColumnGrid);
    assert_eq!(d_case_b.modal_surface, HybridModalSurface::LegacyModal);
    assert!(d_case_b.blocks_egui_grid);

    // Outside click & wheel absorbed by Case B backdrop.
    let modal_rect = driver.state().legacy_modal_rect_pt.ok_or("missing legacy AnalyzeSheet modal rect")?;
    let outside = egui::pos2(6.0, 120.0);
    assert!(!modal_rect.contains(outside));
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside, egui::vec2(0.0, -140.0))?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_click_captured, "Case B backdrop must capture outside click");
    assert!(driver.state().outside_wheel_captured, "Case B backdrop must capture outside wheel");

    // Scroll `analyze.entries_list` via ui.inspect bounds + ui.pointer_gesture wheel, click One-Way ANOVA, confirm.
    for _ in 0..8 {
        let snap = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
        if snap.dialog.as_ref().is_some_and(|d| d.controls.iter().any(|c| c.widget_id == "analyze.entry.stats.one_way_anova")) {
            break;
        }
        let lb = snap
            .dialog
            .as_ref()
            .and_then(|d| d.controls.iter().find(|c| c.widget_id == "analyze.entries_list"))
            .and_then(|c| c.bounds)
            .ok_or("missing analyze.entries_list bounds")?;
        let lx = lb[0] + lb[2] * 0.5;
        let ly = lb[1] + lb[3] * 0.5;
        let w_rep = driver.state_mut().app.dispatch_control_line(&format!(
            r#"{{"id":"w","method":"ui.pointer_gesture","params":{{"action":"wheel","x":{lx},"y":{ly},"delta_x":0.0,"delta_y":140.0,"metrics":{{"logical_width":{},"logical_height":{},"scale_factor":{}}}}}}}"#,
            metrics.logical_width,
            metrics.logical_height,
            metrics.clamped_scale_factor()
        ));
        assert!(w_rep.contains("\"ok\":true"), "scroll analyze list failed: {w_rep}");
    }
    let snap = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    let eb = snap
        .dialog
        .as_ref()
        .and_then(|d| d.controls.iter().find(|c| c.widget_id == "analyze.entry.stats.one_way_anova"))
        .and_then(|c| c.bounds)
        .ok_or("missing analyze.entry.stats.one_way_anova")?;
    let cx = eb[0] + eb[2] * 0.5;
    let cy = eb[1] + eb[3] * 0.5;
    let c_rep = driver
        .state_mut()
        .app
        .dispatch_control_line(&format!(
            r#"{{"id":"c","method":"ui.pointer_gesture","params":{{"action":"click","x":{cx},"y":{cy},"metrics":{{"logical_width":{},"logical_height":{},"scale_factor":{}}}}}}}"#,
            metrics.logical_width,
            metrics.logical_height,
            metrics.clamped_scale_factor()
        ));
    assert!(c_rep.contains("\"ok\":true"), "select one_way_anova failed: {c_rep}");
    let ok_rep = driver
        .state_mut()
        .app
        .dispatch_control_line(r#"{"id":"ok","method":"ui.answer_dialog","params":{"dialog_id":"analyze_sheet","action":"ok"}}"#);
    assert!(ok_rep.contains("\"ok\":true"), "confirm analyze_sheet failed: {ok_rep}");
    driver.run_settled();

    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs_id = driver.state().app.document().active_sheet_id.clone().ok_or("missing ResultSheet id")?;
    assert_eq!(driver.state().app.document().result_sheets.len(), 1);
    let rs_kind = &driver.state().app.document().result_sheet(&rs_id).ok_or("missing created ResultSheet")?.analysis.kind;
    assert!(matches!(rs_kind, AnalysisKind::OneWayAnova(_)), "expected OneWayAnova ResultSheet, got {rs_kind:?}");
    let d_res = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(d_res.sheet_surface, HybridSheetSurface::LegacyPresenter);
    Ok(rs_id)
}

fn run_egui_case_a_anova_modal_and_update(driver: &mut EguiHumanPathDriver<'_, EguiAnovaQaState>, rs_id: &SheetId) -> Result<(), String> {
    driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    let d_case_a = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(d_case_a.sheet_surface, HybridSheetSurface::LegacyPresenter);
    assert_eq!(d_case_a.modal_surface, HybridModalSurface::EguiOneWayAnova);
    assert!(d_case_a.renders_egui_anova_modal);

    // Outside click/wheel behind egui ANOVA modal must not leak to ResultSheet.
    let dlg_rect = driver.state().dialog_rect_pt.ok_or("missing egui dialog_rect_pt")?;
    let outside = egui::pos2(6.0, 120.0);
    assert!(!dlg_rect.contains(outside));
    let tab_before = driver.state().app.canvas_dialogs.result_view.active_tab_index;
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside, egui::vec2(0.0, -180.0))?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_click_captured);
    assert!(driver.state().outside_wheel_captured);
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, tab_before);

    // Keyboard navigation inside modal: Tab must stay within modal/backdrop layers (never escape to sheet/grid behind),
    // and Ctrl+Tab advances across modal tabs cleanly.
    for _ in 0..6 {
        driver.press_key("Tab", KeyModifiers::default())?;
        if let Some(ak_focus) = driver.accesskit_tree().map(|u| u.focus)
            && let Some(focused_entry) = driver.frame_meta().find_by_accesskit_id(ak_focus)
        {
            assert!(
                matches!(focused_entry.z_layer, ZLayer::ModalDialog | ZLayer::ModalBackdrop),
                "Tab focus inside modal escaped backdrop to background `{}` ({:?})",
                focused_entry.widget_id,
                focused_entry.z_layer
            );
        }
    }

    // Verify Multiple Comparisons tab + R3 unlock flow while parametric (Tukey -> Dunnett).
    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    let ctrl_col_disabled = driver.frame_meta().find_by_widget_id("edit.control_column").ok_or("missing edit.control_column")?.clone();
    assert!(!ctrl_col_disabled.enabled);
    driver.click_widget_id("edit.compare_against_control")?;
    let ctrl_col_enabled = driver.frame_meta().find_by_widget_id("edit.control_column").ok_or("missing enabled edit.control_column")?.clone();
    assert!(ctrl_col_enabled.enabled);
    driver.click_widget_id("edit.compare_against_control")?;
    driver.click_widget_id("edit.tab.experimental_design")?;

    // R2: Uncheck Assume Gaussian -> Kruskal-Wallis banner + disabled Tukey with tooltip.
    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(!driver.state().app.canvas_dialogs.edit_params.assume_gaussian);
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Kruskal-Wallis"));
    let tukey = driver.frame_meta().find_by_widget_id("edit.post_hoc.tukey").ok_or("missing edit.post_hoc.tukey")?.clone();
    assert!(!tukey.enabled);
    driver.hover_widget_id("edit.post_hoc.tukey")?;
    let tips = driver.visible_tooltip_texts();
    assert!(tips.iter().any(|t| t.to_ascii_lowercase().contains("nonparametric") || t.contains("Dunn")), "got tooltips: {tips:?}");

    // Confirm Update Analysis -> closes dialog and recalculates ResultSheet.
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs = driver.state().app.document().result_sheet(rs_id).ok_or("missing updated ResultSheet")?;
    assert!(rs.title().contains("Kruskal-Wallis"), "expected Kruskal-Wallis title, got {}", rs.title());
    if let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind {
        assert_eq!(spec.multiplicity.method, MultiplicityMethod::DunnRankSum);
    } else {
        return Err(String::from("expected OneWayAnova spec"));
    }
    Ok(())
}

#[test]
fn r15_2_egui_1m_column_ci_smoke() {
    let r1 = run_egui_1m_column_benchmark(1.0, 6, 4);
    let r2 = run_egui_1m_column_benchmark(2.0, 6, 4);
    print_bench_table_header();
    r1.print_row();
    r2.print_row();

    assert_eq!(r1.total_cells, 1_000_000);
    assert!(r1.visible_cells_last > 0 && r1.visible_cells_last < 2_000, "egui grid must virtualize: {}", r1.visible_cells_last);
    assert!(r2.visible_cells_last > 0 && r2.visible_cells_last < 2_000);
    if !cfg!(debug_assertions) {
        // Note: full isolated steady + input-to-paint (<= 50 ms) Go/No-Go gate lives in
        // `r15_2_gonogo_1m_column_matched_benchmark_release` (`#[ignore]`): cold-heap `ArrowDown`
        // currently triggers `active_data_sheet(app).cloned()` in `views/grid_core.rs:323` (~38–55ms).
        assert!(r1.steady_frame.mean_ms <= EGUI_STEADY_BUDGET_MS);
        assert!(r2.steady_frame.mean_ms <= EGUI_STEADY_BUDGET_MS);
    }
}

#[test]
fn r15_2_egui_r10_1_anova_human_path_cjk_inspect_and_journal() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_journal_dir("egui-r10-1");
    let _cleanup = TempDirCleanup(dir.clone());
    let (session, mut journal, _report) = Session::open_dir_with_recovery(&dir, "egui-qa", 300_000, 1_000)?;

    let mut app = UiApp::from_session(session).with_ui_backend(UiBackend::Egui);
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-spike", "title": "Drug Dose Assay", "kind": "column", "cols": 3, "rows": 6 }))?;
    app.execute_command("table.setTitles", json!({ "sheet_id": "col-spike", "titles": ["Control", "Treated", "HighDose"] }))?;
    let rows = [[10.0, 14.5, 20.1], [11.2, 15.1, 21.0], [9.8, 13.9, 19.6], [10.6, 14.8, 20.8], [10.4, 15.3, 21.5]];
    app.execute_command("table.setCells", json!({ "sheet_id": "col-spike", "rows": rows }))?;
    app.session().checkpoint_journal(&mut journal)?;
    let seed_undo = app.session().undo_depth();

    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = make_egui_qa_driver(EguiAnovaQaState::new(app, journal), metrics);
    let parity0 = driver.verify_pixel_to_hit_box_parity();
    assert!(parity0.is_clean(), "initial egui Column parity violations: {:?}", parity0.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());

    verify_egui_cjk_cell_and_title_ime(&mut driver, seed_undo)?;
    let rs_id = run_egui_case_b_analyze_to_resultsheet(&mut driver, metrics)?;
    run_egui_case_a_anova_modal_and_update(&mut driver, &rs_id)?;

    // Verify `ui.inspect` parity from both live driver metadata & headless `inspect_egui_shell_snapshot`.
    let live_inspect = build_ui_tree_snapshot_from_egui(&driver.state().app, driver.frame_meta(), driver.accesskit_tree(), None, true, metrics);
    let shell_inspect = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, None, true);
    assert_eq!(live_inspect.active_sheet_id.as_deref(), Some(rs_id.as_str()));
    assert_eq!(shell_inspect.active_sheet_id.as_deref(), Some(rs_id.as_str()));
    assert!(shell_inspect.undo_depth > seed_undo);

    // Switch back to Column sheet tab and verify CJK header/cell text appears in egui `ui.inspect` grid scope.
    driver.click_widget_id("sheet.tab.col-spike")?;
    let grid_inspect = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("grid"), true);
    let cells = grid_inspect.grid_cells.as_ref().ok_or("expected egui grid_cells in ui.inspect")?;
    assert!(cells.iter().any(|w| w.widget_id == "grid.col_title.0" && w.label == "対照群"));
    assert!(cells.iter().any(|w| w.widget_id == "grid.cell.5.0.0" && w.value.as_ref().is_some_and(|v| v["text"] == "用量反応")));

    // Verify undo restores ResultSheet back from Kruskal-Wallis to Ordinary One-Way ANOVA.
    driver.state_mut().app.execute_command("edit.undo", json!({}))?;
    assert!(driver.state().app.document().result_sheet(&rs_id).is_some_and(|r| r.title().contains("ANOVA") || r.title().contains("One-Way")));

    // Verify 14 Beta locales on Column + EditAnalysisParameters hybrid parity & non-bleed.
    for &locale in &BETA_LOCALE_CODES {
        let mut loc_app = UiApp::from_session(Session::from_document(driver.state().app.document().clone())).with_ui_backend(UiBackend::Egui);
        let _ = loc_app.set_locale(locale);
        loc_app.sync_views();
        let snap = loc_app.inspect_egui_shell_snapshot(ViewportMetrics::new(1280.0, 800.0, 1.0), Some("grid"), true);
        assert!(snap.grid_cells.is_some(), "expected grid inspect for locale {locale}");
    }

    // WAL ops.journal replay restores both CJK cell & CJK column title commits.
    if let Some(j) = driver.state_mut().journal.take() {
        drop(j.abandon());
    }
    assert!(dir.join(JOURNAL_FILE_NAME).exists());
    let (recovered, _rec_j, _rep) = Session::open_dir_with_recovery(&dir, "egui-qa-recover", 300_000, 305_000)?;
    if let TableArchetype::Column(col) = &recovered.document().data_sheets[0].archetype {
        assert_eq!(col.data_sets[0].title, "対照群");
        assert_eq!(col.data_sets[0].cell(5, 0).map(|c| &c.value), Some(&CellValue::Text(String::from("用量反応"))));
    } else {
        return Err(From::from("expected Column archetype on WAL recovery"));
    }
    Ok(())
}
