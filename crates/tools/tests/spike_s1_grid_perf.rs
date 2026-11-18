//! Risk Spike S1 verification: O(1) virtualized 1,000,000-cell spreadsheet interaction latency.

use std::hint::black_box;
use std::time::Instant;

use curvebook_doc::{
    Cell, CellCoord, DataSheet, DocError, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkBorderColor,
    LinkSource, LinkedDataRegion, MAX_ROWS, SheetCellRange, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_tools::grid::{GridDirection, GridNavAction, format_selection_tsv, parse_tsv_clipboard};
use curvebook_tools::{GridCellCoord, GridToolState, SelectionQuickStats};

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (((sorted.len() - 1) as f64) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn build_million_cell_grouped_sheet() -> Result<DataSheet, DocError> {
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(4), 250, 1_000)?;
    grouped.row_titles = (0..1_000).map(|row| format!("Dose {row}")).collect();

    for (g_idx, ds) in grouped.data_sets.iter_mut().enumerate() {
        ds.title = format!("Cohort {}", g_idx + 1);
        for (r_idx, row_cells) in ds.cells.iter_mut().enumerate() {
            for (s_idx, cell) in row_cells.iter_mut().enumerate() {
                let value = ((g_idx * 4 + s_idx) as f64) * 0.25 + (r_idx as f64) * 0.01;
                *cell = if (r_idx * 1_000 + g_idx * 4 + s_idx) % 37 == 0 { Cell::excluded_number(value) } else { Cell::number(value) };
            }
        }
    }

    let mut sheet = DataSheet::new(SheetId::new("spike-s1-1m"), "Risk Spike S1 (1000x1000)", TableArchetype::Grouped(grouped))?;
    sheet.add_linked_region(LinkedDataRegion {
        id: String::from("link-s1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("upstream-1"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::default(),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(25, 4, 3)),
        provenance_summary: String::from("live link"),
    })?;
    Ok(sheet)
}

#[test]
fn spike_s1_million_cell_virtualized_60fps_scroll_and_selection_benchmark() {
    let sheet = build_million_cell_grouped_sheet().expect("1M-cell grouped fixture");
    assert_eq!(sheet.archetype.row_count(), 1_000);
    assert_eq!(GridToolState::sheet_total_flat_cols(&sheet), 1_000);

    let mut tool = GridToolState::default();
    tool.viewport.viewport_width_px = 1280.0;
    tool.viewport.viewport_height_px = 800.0;
    tool.viewport.row_height_px = 22.0;
    tool.viewport.default_col_width_px = 96.0;
    tool.select_single(GridCellCoord::new(0, 0, 0));

    let frames = 120usize;
    let mut frame_ms = Vec::with_capacity(frames);

    // Warm up one pass before measuring 120 simulated 60-fps ticks.
    let _ = black_box(tool.format_visible_window(&sheet));

    for tick in 0..frames {
        let t0 = Instant::now();

        let dx = if tick % 2 == 0 { 760.0 } else { -310.0 };
        let dy = if tick % 3 == 0 { 165.0 } else { -55.0 };
        if tick == 60 {
            // Jump cleanly to bottom-right corner mid-run to confirm O(1) far-offset behavior.
            let total_cols = GridToolState::sheet_total_flat_cols(&sheet);
            tool.set_scroll_clamped(96.0 * 985.0, 22.0 * 965.0, sheet.archetype.row_count(), total_cols);
        } else {
            tool.scroll_for_sheet(&sheet, dx, dy);
        }

        let pinned = tool.pinned_row_title_geometry(&sheet);
        let headers = tool.visible_two_tier_headers(&sheet);
        let frame = tool.format_visible_window(&sheet);

        let sel_end_row = (frame.window.start_row + 7).min(sheet.archetype.row_count().saturating_sub(1));
        let sel_start_flat = frame.window.start_col.min(999);
        let sel_end_flat = (frame.window.start_col + 7).min(999);
        let (sc, ss) = GridToolState::flat_col_to_coord(&sheet, sel_start_flat).unwrap_or((0, 0));
        let (ec, es) = GridToolState::flat_col_to_coord(&sheet, sel_end_flat).unwrap_or((0, 0));
        let sel_start = GridCellCoord::new(frame.window.start_row, sc, ss);
        let sel_end = GridCellCoord::new(sel_end_row, ec, es);
        tool.anchor = Some(sel_start);
        tool.active = Some(sel_end);

        let stats = SelectionQuickStats::from_data_sheet(&sheet, sel_start, sel_end);
        if tick % 10 == 0 {
            let _ = black_box(tool.navigate(&sheet, GridNavAction::Arrow { dir: GridDirection::Right, extend: true, jump_block: false }));
        }

        black_box((&pinned, &headers, &frame, &stats));
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        frame_ms.push(elapsed_ms);

        assert!(!frame.cells.is_empty(), "visible frame cells must be populated on tick {tick}");
        assert!(!headers.groups.is_empty(), "visible two-tier headers must be populated on tick {tick}");
        assert!(stats.selected_cells > 0 && stats.numeric_count > 0);
    }

    let mut sorted = frame_ms.clone();
    sorted.sort_by(f64::total_cmp);
    let min_ms = sorted.first().copied().unwrap_or(0.0);
    let p50_ms = percentile_sorted(&sorted, 0.50);
    let p95_ms = percentile_sorted(&sorted, 0.95);
    let max_ms = sorted.last().copied().unwrap_or(0.0);
    let mean_ms = frame_ms.iter().sum::<f64>() / (frames as f64);
    let profile = if cfg!(debug_assertions) { "dev(opt-level=1)" } else { "release" };

    println!(
        "SPIKE_S1_GRID_PERF profile={profile} grid=1000x1000(1_000_000_cells) frames={frames} min_ms={min_ms:.4} p50_ms={p50_ms:.4} p95_ms={p95_ms:.4} max_ms={max_ms:.4} mean_ms={mean_ms:.4}"
    );

    let median_budget_ms = if cfg!(debug_assertions) { 2.0 } else { 1.0 };
    let p95_budget_ms = if cfg!(debug_assertions) { 25.0 } else { 2.0 };
    let frame_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(p50_ms < median_budget_ms, "expected <{median_budget_ms:.2}ms median frame time in {profile}, measured p50={p50_ms:.4} ms");
    assert!(max_ms < frame_budget_ms, "60fps budget exceeded in {profile}: max={max_ms:.4} ms (p95={p95_ms:.4} ms, limit={frame_budget_ms:.2} ms)");
    assert!(p95_ms < p95_budget_ms, "expected <{p95_budget_ms:.2}ms p95 frame time in {profile}, measured p95={p95_ms:.4} ms");
}

#[test]
fn spike_s1_o1_one_million_rows_navigation_and_windowing() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, MAX_ROWS).expect("1M-row XyTable");
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[MAX_ROWS - 1][0] = Cell::number(1_000_000.0);
    let mut sheet = DataSheet::new("xy-1m-rows", "1M Rows", TableArchetype::Xy(xy)).expect("sheet");
    sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0)).expect("y0");
    sheet.set_cell(CellCoord::new(MAX_ROWS - 1, 0, 0), Cell::number(99.5)).expect("y_last");
    sheet.set_cell(CellCoord::new(MAX_ROWS - 1, 1, 0), Cell::number(199.5)).expect("yb_last");

    let mut tool = GridToolState::default();
    tool.viewport.viewport_width_px = 1280.0;
    tool.viewport.viewport_height_px = 768.0;
    tool.select_single(GridCellCoord::new(0, 0, 0));

    let t0 = Instant::now();
    let end_coord = tool.navigate(&sheet, GridNavAction::End { to_table_end: true, extend: false }).expect("jump to row 999_999");
    let frame = tool.format_visible_window(&sheet);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1_000.0;

    assert_eq!(end_coord.row, MAX_ROWS - 1);
    assert!(frame.window.end_row == MAX_ROWS);
    assert!(frame.cells.len() < 128, "only visible slice should be formatted, got {} cells", frame.cells.len());
    let jump_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(elapsed_ms < jump_budget_ms, "1M-row tail jump + format took {elapsed_ms:.4} ms (limit={jump_budget_ms:.2} ms)");
    println!("SPIKE_S1_MAX_ROWS_JUMP rows={MAX_ROWS} elapsed_ms={elapsed_ms:.4} visible_cells={}", frame.cells.len());
}

#[test]
fn spike_s1_link_border_and_tsv_roundtrip() {
    let sheet = build_million_cell_grouped_sheet().expect("1M-cell grouped fixture");
    let mut tool = GridToolState { decimal_separator: ',', ..GridToolState::default() };
    tool.viewport.viewport_width_px = 1280.0;
    tool.viewport.viewport_height_px = 800.0;

    let frame = tool.format_visible_window(&sheet);
    assert_eq!(frame.cells[0].link_border, Some(LinkBorderColor::Blue));

    tool.select_single(GridCellCoord::new(0, 0, 0));
    tool.extend_selection(GridCellCoord::new(1, 0, 1));
    let tsv = format_selection_tsv(&sheet, GridCellCoord::new(0, 0, 0), GridCellCoord::new(1, 0, 1), ',');
    let parsed = parse_tsv_clipboard(&tsv, ',', true).expect("parse european TSV");
    assert_eq!(parsed.len(), 2);
    assert!(parsed[0][0].excluded);

    let paste_intent = tool.emit_paste_intent(&sheet.id, &tsv, false, true).expect("paste intent");
    assert_eq!(paste_intent.command_id, "table.paste");
    let exclude_intent = tool.emit_exclude_selection_intent(&sheet.id, true).expect("exclude intent");
    assert_eq!(exclude_intent.command_id, "table.excludeCells");
}
