//! Spike S1 1,000,000-cell grid virtualization benchmark and end-to-end tests for Project
//! Navigator, Family pane anchor stability, provenance badges, Welcome wizard, two-tier headers,
//! `MultipleVariables` role/formula tooling, 4-stage Import/Paste Link wizard, and quick stats.

use std::time::Instant;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    GraphSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet,
    LayoutSpec, LevelOrderKind, LevelOrderSpec, LinkBorderColor, LinkSource, LinkedDataRegion, MultipleVariablesTable, NestedTable, NormalizeAnchor,
    RectPt, ResultSheet, ResultStatus, SheetCellRange, SheetId, SheetSection, SubcolumnMode, TableArchetype, TableKind, TransformAnalysisSpec,
    VariableRole, VariableSource,
};
use lumencraft_engine::Session;
use lumencraft_tools::{GridCellCoord, GridViewportSpec};
use lumencraft_ui_egui::UiApp;
use lumencraft_ui_egui::grid_view::{
    GridQuickStats, GridViewState, ImportOriginKind, ImportWizardStage, MoveDirection, provenance_border_rgba, total_flat_data_subcolumns,
};
use lumencraft_ui_egui::workspace_nav::{
    ClonePipelineDraft, FamilyRelation, NavigatorBadgeColor, NavigatorBadgeKind, NavigatorNestingMode, SAMPLE_FIXTURES, TableCreationDraft,
    WelcomeWizardState, WorkspaceNavState, instantiate_sample_fixture,
};

fn build_large_grouped_doc() -> Result<lumencraft_doc::ProjectDocument, lumencraft_doc::DocError> {
    let mut doc = lumencraft_doc::ProjectDocument::default();

    // Small internal donor sheet to attach a live Blue provenance link inside the 1M-cell grid.
    let mut src_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    for r in 0..4 {
        src_col.data_sets[0].cells[r][0] = Cell::number(100.0 + r as f64);
    }
    let donor = DataSheet::new("tbl-donor", "Donor Table", TableArchetype::Column(src_col))?;
    doc.add_data_sheet(donor)?;

    // 250 groups x 4 replicates = 1,000 subcolumns; 1,000 rows => 1,000,000 cells.
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(4), 250, 1_000)?;
    for (g_idx, group) in grouped.data_sets.iter_mut().enumerate() {
        group.title = format!("Cohort {}", g_idx + 1);
        for (r_idx, row) in group.cells.iter_mut().enumerate() {
            for (s_idx, cell) in row.iter_mut().enumerate() {
                *cell = Cell::number(((g_idx * 4 + s_idx) as f64) * 0.1 + (r_idx as f64));
            }
        }
    }

    let mut big_sheet = DataSheet::new("tbl-big", "Mega Assay 1M", TableArchetype::Grouped(grouped))?;
    big_sheet.set_excluded(CellCoord::new(0, 0, 0), true)?;
    big_sheet.linked_regions.push(LinkedDataRegion {
        id: String::from("link-big-blue"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::from("tbl-donor"),
                tab_id: None,
                kind: InternalLinkKind::DataSheet,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0)),
        provenance_summary: String::from("Linked from Donor Table"),
    });

    doc.add_data_sheet(big_sheet)?;
    doc.select_in_navigator(&SheetId::from("tbl-big"))?;
    Ok(doc)
}

#[test]
fn spike_s1_grouped_one_million_cells_120_frames_sub_16ms() {
    let doc = build_large_grouped_doc().expect("build 1M grouped doc");
    let big = doc.data_sheet(&SheetId::from("tbl-big")).expect("big sheet exists");
    assert_eq!(big.archetype.row_count(), 1_000);
    assert_eq!(total_flat_data_subcolumns(&big.archetype), 1_000);

    let session = Session::from_document(doc);
    let mut app = UiApp::from_session(session);
    app.grid.tool.viewport = GridViewportSpec {
        scroll_x_px: 0.0,
        scroll_y_px: 0.0,
        viewport_width_px: 1_440.0,
        viewport_height_px: 900.0,
        default_col_width_px: 110.0,
        row_height_px: 24.0,
        frozen_header_rows: 2,
        frozen_row_title_cols: 1,
    };
    app.sync_views();

    // Verify excluded badge & Blue provenance border on the initial frame at (0,0,0).
    let initial_frame = app.grid.visible_frame(app.document()).expect("initial visible frame");
    let top_left_cell = &initial_frame.rows[0].cells[0];
    assert!(top_left_cell.excluded);
    assert!(top_left_cell.visual.italic);
    assert!(top_left_cell.visual.strikethrough);
    assert_eq!(top_left_cell.visual.excluded_badge.as_deref(), Some("*"));
    assert!(top_left_cell.visual.text_color.b > top_left_cell.visual.text_color.r);
    assert_eq!(top_left_cell.visual.provenance_border, Some(LinkBorderColor::Blue));
    assert_eq!(top_left_cell.visual.provenance_border_rgba, Some(provenance_border_rgba(LinkBorderColor::Blue)));
    assert_eq!(app.nav.family_anchor_sheet_id, Some(SheetId::from("tbl-big")));

    let mut frame_ms = Vec::with_capacity(120);
    let mut last_visible_rows = 0usize;
    let mut last_visible_cols = 0usize;

    for frame_idx in 0..120usize {
        let start = Instant::now();

        app.grid.tool.viewport.scroll_y_px = ((frame_idx * 192) % (950 * 24)) as f64;
        app.grid.tool.viewport.scroll_x_px = ((frame_idx * 330) % (960 * 110)) as f64;

        let target_row = (frame_idx * 8) % 990;
        let target_group = (frame_idx * 2) % 245;
        let target_subcol = frame_idx % 4;
        if frame_idx % 3 == 0 {
            app.grid.tool.select_single(GridCellCoord::new(target_row, target_group, target_subcol));
        } else {
            app.grid.tool.anchor = Some(GridCellCoord::new(target_row, target_group, 0));
            app.grid.tool.active = Some(GridCellCoord::new(target_row + 5, target_group + 1, 3));
        }

        app.sync_views();
        let window = app.visible_grid_window().expect("window visible");
        let frame = app.grid.visible_frame(app.document()).expect("frame visible");

        assert!(window.end_row > window.start_row);
        assert!(window.end_col > window.start_col);
        assert_eq!(frame.total_rows, 1_000);
        assert_eq!(frame.total_flat_cols, 1_000);
        assert!(!frame.tier1_groups.is_empty());
        assert!(!frame.tier2_subcolumns.is_empty());
        assert!(!app.nav.entries.is_empty());

        last_visible_rows = frame.rows.len();
        last_visible_cols = frame.tier2_subcolumns.len();

        let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
        frame_ms.push(elapsed_ms);
    }

    let mut sorted = frame_ms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() * 95) / 100];
    let max_ms = *sorted.last().expect("non-empty timings");
    let mean_ms = frame_ms.iter().sum::<f64>() / (frame_ms.len() as f64);

    println!(
        "SPIKE_S1_UI_1M grouped 120 frames: p50={p50:.3}ms p95={p95:.3}ms mean={mean_ms:.3}ms max={max_ms:.3}ms visible={last_visible_rows}x{last_visible_cols}"
    );
    assert!(p95 < 16.67, "p95 frame time {p95:.3}ms exceeded 16.67ms budget");
    assert!(max_ms < 16.67, "max frame time {max_ms:.3}ms exceeded 16.67ms budget");
}

#[test]
fn spike_s1_grouped_one_million_cells_select_all_and_exclude_budget() {
    let doc = build_large_grouped_doc().expect("build 1M cell doc");
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    // Select All across all 1,000,000 cells must stay O(1) without scanning/sorting 1M cells or
    // allocating 1M per-cell intents.
    app.grid.tool.anchor = Some(GridCellCoord::new(0, 0, 0));
    app.grid.tool.active = Some(GridCellCoord::new(999, 249, 3));
    let select_all_start = Instant::now();
    app.sync_views();
    let select_all_frame = app.grid.visible_frame(app.document()).expect("select all visible frame");
    let mut detached_grid = std::mem::take(&mut app.grid);
    let emitted = {
        let ds_1m = app.document().data_sheet(&SheetId::from("tbl-big")).expect("tbl-big");
        detached_grid.emit_toggle_exclude_intents(ds_1m, true)
    };
    app.grid = detached_grid;
    let select_all_ms = select_all_start.elapsed().as_secs_f64() * 1_000.0;
    println!("SPIKE_S1_UI_1M Select All (1,000,000 cells): sync+frame+exclude={select_all_ms:.3}ms intents={emitted}");

    assert!(select_all_ms < 16.67, "Select All frame time {select_all_ms:.3}ms exceeded 16.67ms budget");
    assert!(app.grid.quick_stats.is_none(), "quick_stats should skip >50k clone/sort pass");
    assert_eq!(app.grid.extended_stats.expect("extended stats").selected_cells, 1_000_000);
    assert_eq!(select_all_frame.total_rows, 1_000);
    assert_eq!(emitted, 1);
    assert_eq!(app.grid.tool.pending_intents.len(), 1);
    assert_eq!(app.grid.tool.pending_intents[0].command_id, "table.excludeCells");
    assert_eq!(app.grid.tool.pending_intents[0].params["range"]["start"]["row"], 0);
    assert_eq!(app.grid.tool.pending_intents[0].params["range"]["end"]["col"], 249);
    assert_eq!(app.grid.tool.pending_intents[0].params["range"]["end"]["subcol"], 3);
}

#[test]
fn spike_s1_multiple_variables_one_million_cells_scroll_and_role_strip() {
    let mut mv = MultipleVariablesTable::new(1_000, 1_000).expect("1000x1000 multivar");
    for (c, var) in mv.variables.iter_mut().enumerate() {
        var.title = format!("Var_{c}");
        var.role = if c % 5 == 0 { VariableRole::Categorical } else { VariableRole::Continuous };
        var.role_inferred = false;
        for (r, cell) in var.cells.iter_mut().enumerate() {
            *cell = if c % 5 == 0 { Cell::text(if r % 2 == 0 { "A" } else { "B" }) } else { Cell::number((r + c) as f64) };
        }
    }
    mv.variables[999].set_calculated_formula("= Var_1 + Var_2");
    let sheet = DataSheet::new("mv-1m", "Cohort 1M", TableArchetype::MultipleVariables(mv)).expect("sheet");

    let mut doc = lumencraft_doc::ProjectDocument::default();
    doc.add_data_sheet(sheet).expect("add mv-1m");
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    let mut frame_ms = Vec::with_capacity(30);
    for step in 0..30usize {
        let start = Instant::now();
        app.grid.tool.viewport.scroll_y_px = ((step * 600) % 22_000) as f64;
        app.grid.tool.viewport.scroll_x_px = ((step * 1_800) % 95_000) as f64;
        app.grid.tool.select_single(GridCellCoord::new(step * 25, step * 30, 0));
        app.sync_views();
        let frame = app.grid.visible_frame(app.document()).expect("mv visible frame");
        assert_eq!(frame.total_rows, 1_000);
        assert_eq!(frame.total_flat_cols, 1_000);
        assert!(frame.tier1_groups.iter().all(|g| g.variable_role.is_some()));
        frame_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
    }

    let max_ms = frame_ms.iter().copied().fold(0.0_f64, f64::max);
    let mean_ms = frame_ms.iter().sum::<f64>() / (frame_ms.len() as f64);
    println!("SPIKE_S1_UI_1M multivar 30 frames: mean={mean_ms:.3}ms max={max_ms:.3}ms");
    assert!(max_ms < 16.67, "multivar max frame time {max_ms:.3}ms exceeded 16.67ms budget");

    let role_strip = app.grid.variable_role_strip(app.document().data_sheet(&SheetId::from("mv-1m")).expect("mv sheet"));
    assert_eq!(role_strip.len(), 1_000);
    assert!(role_strip[999].is_calculated);
}

#[test]
fn two_tier_headers_and_all_four_provenance_colors() {
    let mut doc = lumencraft_doc::ProjectDocument::default();

    let mut nested = NestedTable::new(2, 3, 4).expect("nested table");
    nested.groups[0].title = String::from("Wildtype");
    nested.groups[0].subcolumn_titles = vec![String::from("WT:1"), String::from("WT:2"), String::from("WT:3")];
    nested.groups[1].title = String::from("Mutant");
    nested.groups[1].subcolumn_titles = vec![String::from("Mut:1"), String::from("Mut:2"), String::from("Mut:3")];
    for r in 0..4 {
        for s in 0..3 {
            nested.groups[0].cells[r][s] = Cell::number(10.0 + r as f64 + s as f64);
            nested.groups[1].cells[r][s] = Cell::number(20.0 + r as f64 + s as f64);
        }
    }

    let mut target = DataSheet::new("tbl-nested", "Nested Target", TableArchetype::Nested(nested)).expect("target");
    target.set_excluded(CellCoord::new(0, 0, 1), true).expect("exclude nested cell");

    let specs = [
        (
            "link-blue",
            LinkSource::InternalSheet {
                sheet_id: SheetId::from("src-data"),
                tab_id: None,
                kind: InternalLinkKind::DataSheet,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            },
            0usize,
            0usize,
        ),
        (
            "link-green",
            LinkSource::InternalSheet {
                sheet_id: SheetId::from("res-norm"),
                tab_id: Some(String::from("transformed")),
                kind: InternalLinkKind::AnalyzableResultSheet,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            },
            0,
            2,
        ),
        (
            "link-red",
            LinkSource::InternalSheet {
                sheet_id: SheetId::from("res-stats"),
                tab_id: Some(String::from("summary")),
                kind: InternalLinkKind::TerminalResultSheet,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            },
            1,
            0,
        ),
        ("link-charcoal", LinkSource::ExternalDelimitedFile { path: String::from("/data/plate.tsv"), delimiter: '\t', decimal_separator: '.' }, 1, 2),
    ];

    for (id, source, col, subcol) in specs {
        let coord = CellCoord::new(0, col, subcol);
        target.linked_regions.push(LinkedDataRegion {
            id: String::from(id),
            spec: ImportLinkSpec { source, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() },
            target_range: SheetCellRange::new(coord, coord),
            provenance_summary: String::from("test link"),
        });
    }
    doc.add_data_sheet(target).expect("add target");

    let mut grid = GridViewState::default();
    let frame = grid.visible_frame(&doc).expect("frame");
    assert_eq!(frame.tier1_groups.len(), 2);
    assert_eq!(frame.tier1_groups[0].title, "Wildtype");
    assert_eq!(frame.tier1_groups[0].span, 3);
    assert_eq!(frame.tier1_groups[1].title, "Mutant");
    assert_eq!(frame.tier1_groups[1].start_flat_col, 3);
    assert_eq!(frame.tier2_subcolumns.len(), 6);
    assert_eq!(frame.tier2_subcolumns[0].label, "WT:1");
    assert_eq!(frame.tier2_subcolumns[5].label, "Mut:3");

    let row0 = &frame.rows[0].cells;
    assert_eq!(row0[0].visual.provenance_border, Some(LinkBorderColor::Blue));
    assert!(row0[1].excluded && row0[1].visual.italic && row0[1].visual.strikethrough);
    assert_eq!(row0[2].visual.provenance_border, Some(LinkBorderColor::Green));
    assert_eq!(row0[3].visual.provenance_border, Some(LinkBorderColor::Red));
    assert_eq!(row0[5].visual.provenance_border, Some(LinkBorderColor::Charcoal));

    let sheet_ref = doc.data_sheet(&SheetId::from("tbl-nested")).expect("sheet");
    assert!(grid.inspect_link_at(sheet_ref, GridCellCoord::new(0, 1, 2)));
    assert_eq!(grid.format_link_inspector.border_color, Some(LinkBorderColor::Charcoal));
    assert!(grid.format_link_inspector.open);
}

#[test]
fn navigator_five_folders_nesting_transitive_bold_badges_and_family_anchor() {
    let mut doc = lumencraft_doc::ProjectDocument::default();

    let mut d1 = instantiate_sample_fixture("dose-response-4pl", "d1").expect("d1");
    d1.title = String::from("Binding Assay");
    d1.frozen = true;
    d1.linked_regions.push(LinkedDataRegion {
        id: String::from("ext-1"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("/tmp/assay.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(3, 0, 0)),
        provenance_summary: String::from("/tmp/assay.csv"),
    });
    doc.add_data_sheet(d1).expect("add d1");

    let mut d2 = instantiate_sample_fixture("unpaired-treatment-groups", "d2").expect("d2");
    d2.title = String::from("Unrelated Assay");
    doc.add_data_sheet(d2).expect("add d2");

    let info = InfoSheet::new_preseeded("info-1", "Study Metadata").expect("info");
    doc.add_info_sheet(info).expect("add info");

    let norm_spec = AnalysisSpec::new(
        "r-norm",
        "d1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        }),
    )
    .expect("norm spec");
    doc.add_result_sheet(ResultSheet::new(norm_spec).expect("r-norm")).expect("add r-norm");

    let desc_spec = AnalysisSpec::new("r-desc", "r-norm", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("desc spec");
    let mut r_desc = ResultSheet::new(desc_spec).expect("r-desc");
    r_desc.status = ResultStatus::Dirty;
    doc.add_result_sheet(r_desc).expect("add r-desc");

    let mut g1 = GraphSheet::new_uninitialized_for_data("g1", doc.data_sheet(&SheetId::from("d1")).expect("d1")).expect("g1");
    g1.title = String::from("Normalized Curve");
    if let Some(first_series) = g1.spec.series.first_mut() {
        first_series.source_sheet_id = SheetId::from("r-desc");
    }
    doc.add_graph_sheet(g1).expect("add g1");

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::from("g1"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    let lay = LayoutSheet::new("lay-1", "Figure 1", layout_spec).expect("lay-1");
    doc.add_layout_sheet(lay).expect("add lay-1");

    let mut nav = WorkspaceNavState::default();
    nav.click_navigator_entry(&mut doc, &SheetId::from("lay-1")).expect("select lay-1");

    // Verify all 5 canonical folders exist and transitive dependency bold highlighting ignores unrelated d2.
    assert_eq!(nav.folders.len(), 5);
    assert_eq!(nav.folders[0].section, SheetSection::Data);
    assert_eq!(nav.folders[1].section, SheetSection::Info);
    assert_eq!(nav.folders[2].section, SheetSection::Analyses);
    assert_eq!(nav.folders[3].section, SheetSection::Graphs);
    assert_eq!(nav.folders[4].section, SheetSection::Layouts);

    for id in ["d1", "r-norm", "r-desc", "g1", "lay-1"] {
        let entry = nav.entries.iter().find(|e| e.sheet_id == SheetId::from(id)).expect("entry present");
        assert!(entry.is_bold, "expected {id} to be transitively bold");
    }
    let d2_entry = nav.entries.iter().find(|e| e.sheet_id == SheetId::from("d2")).expect("d2 entry");
    assert!(!d2_entry.is_bold, "unrelated d2 should not be bold");

    // Verify Freeze (Charcoal) & Link badges on d1, Dirty (Red) badge on r-desc.
    let d1_entry = nav.entries.iter().find(|e| e.sheet_id == SheetId::from("d1")).expect("d1 entry");
    assert!(d1_entry.is_frozen);
    assert!(d1_entry.badges.iter().any(|b| b.kind == NavigatorBadgeKind::Frozen && b.color == NavigatorBadgeColor::Charcoal));
    assert!(d1_entry.link_colors.contains(&LinkBorderColor::Charcoal));
    let desc_entry = nav.entries.iter().find(|e| e.sheet_id == SheetId::from("r-desc")).expect("r-desc entry");
    assert!(desc_entry.badges.iter().any(|b| b.kind == NavigatorBadgeKind::Dirty && b.color == NavigatorBadgeColor::Red));

    // Verify NestedResultsUnderData mode moves r-norm (depth 1) & r-desc (depth 2) into Data Tables folder.
    nav.set_nesting_mode(NavigatorNestingMode::NestedResultsUnderData, &doc);
    let data_folder = nav.folders.iter().find(|f| f.section == SheetSection::Data).expect("data folder");
    let results_folder = nav.folders.iter().find(|f| f.section == SheetSection::Analyses).expect("results folder");
    assert!(results_folder.entries.is_empty());
    let nested_norm = data_folder.entries.iter().find(|e| e.sheet_id == SheetId::from("r-norm")).expect("nested r-norm");
    let nested_desc = data_folder.entries.iter().find(|e| e.sheet_id == SheetId::from("r-desc")).expect("nested r-desc");
    assert_eq!(nested_norm.nesting_depth, 1);
    assert_eq!(nested_desc.nesting_depth, 2);

    // Search filter narrows entries cleanly.
    nav.set_search_query("Normalized", &doc);
    let filtered = nav.filtered_entries();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].sheet_id, SheetId::from("g1"));
    nav.set_search_query("", &doc);

    // Stable-anchor Family pane behavior: anchor on d1, single-click r-desc/g1 keeps anchor = d1.
    nav.click_navigator_entry(&mut doc, &SheetId::from("d1")).expect("anchor on d1");
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("d1")));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("d1") && n.relation == FamilyRelation::Anchor));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("lay-1") && n.relation == FamilyRelation::Downstream));

    nav.click_family_entry(&mut doc, &SheetId::from("r-desc"), false).expect("single-click result");
    assert_eq!(doc.active_sheet_id, Some(SheetId::from("r-desc")));
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("d1")), "anchor must remain stable on single-click");
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("lay-1")));

    nav.click_family_entry(&mut doc, &SheetId::from("g1"), false).expect("single-click graph");
    assert_eq!(doc.active_sheet_id, Some(SheetId::from("g1")));
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("d1")), "anchor must remain stable on graph click");

    // Double-clicking a downstream node shifts the Family anchor!
    nav.click_family_entry(&mut doc, &SheetId::from("r-norm"), true).expect("double-click result");
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("r-norm")));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("d1") && n.relation == FamilyRelation::Upstream));
}

#[test]
fn welcome_wizard_all_archetypes_fixtures_and_clone_pipeline() {
    let mut doc = lumencraft_doc::ProjectDocument::default();
    let mut wizard = WelcomeWizardState::default();

    for (idx, kind) in TableKind::ALL.iter().enumerate() {
        wizard.select_archetype(*kind);
        wizard.set_replicates(4);
        wizard.table_draft.sheet_id = format!("tbl-kind-{idx}");
        let id = wizard.table_draft.clone().create_into_document(&mut doc).expect("create typed table");
        let ds = doc.data_sheet(&id).expect("sheet inserted");
        assert_eq!(ds.kind(), *kind);
        if idx + 1 < TableKind::ALL.len() {
            wizard.focus_next_archetype();
        }
    }

    // Column paired vs unpaired hint support.
    let paired = TableCreationDraft::for_kind(TableKind::Column)
        .with_column_design(ColumnDesignHint::PairedRepeatedMeasures)
        .build_archetype()
        .expect("paired");
    assert_eq!(paired.kind(), TableKind::Column);

    // Every built-in curated fixture instantiates cleanly.
    assert_eq!(SAMPLE_FIXTURES.len(), 9);
    for (idx, fx) in SAMPLE_FIXTURES.iter().enumerate() {
        let ds = instantiate_sample_fixture(fx.id, format!("fx-{idx}")).expect("fixture");
        assert_eq!(ds.kind(), fx.kind);
    }

    // Clone Pipeline duplicates source sheet + downstream result graph via engine command.
    let src_sheet = instantiate_sample_fixture("dose-response-4pl", "src-xy").expect("src-xy");
    doc.add_data_sheet(src_sheet).expect("add src-xy");
    let src_res = AnalysisSpec::new(
        "src-res",
        "src-xy",
        AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: None, y_transform: None, error_propagation: Default::default() }),
    )
    .expect("src-res");
    doc.add_result_sheet(ResultSheet::new(src_res).expect("result")).expect("add src-res");
    let src_graph =
        GraphSheet::new_uninitialized_for_data("src-graph", doc.data_sheet(&SheetId::from("src-xy")).expect("src-xy")).expect("src-graph");
    doc.add_graph_sheet(src_graph).expect("add src-graph");

    let clone_opts = CloneDataSheetOptions {
        title: String::from("Draft Clone"),
        keep_x_values: true,
        clear_y_values: false,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: None,
    };
    let draft_only = ClonePipelineDraft {
        source_sheet_id: Some(SheetId::from("src-xy")),
        new_sheet_id: String::from("cloned-draft"),
        options: clone_opts.clone(),
        ..ClonePipelineDraft::default()
    };
    assert_eq!(draft_only.execute_in_document(&mut doc).expect("local draft clone"), SheetId::from("cloned-draft"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    let cmd_draft = ClonePipelineDraft {
        source_sheet_id: Some(SheetId::from("src-xy")),
        new_sheet_id: String::from("cloned-xy"),
        options: CloneDataSheetOptions { title: String::from("Cloned Assay"), ..clone_opts },
        ..ClonePipelineDraft::default()
    };
    let intent = cmd_draft.to_command_intent().expect("clone intent");
    app.execute_command(&intent.command_id, intent.params).expect("execute sheet.clonePipeline");
    assert!(app.document().data_sheet(&SheetId::from("cloned-xy")).is_some());
    assert!(
        app.document().result_sheets.iter().any(|r| r.analysis.source_sheet_id == SheetId::from("cloned-xy")),
        "cloned pipeline should duplicate downstream result referencing cloned-xy"
    );
    assert!(
        app.document()
            .graph_sheets
            .iter()
            .any(|g| g.id != SheetId::from("src-graph") && g.spec.series.iter().any(|s| s.source_sheet_id == SheetId::from("cloned-xy"))),
        "cloned pipeline should duplicate downstream graph referencing cloned-xy"
    );
}

#[test]
fn multivar_variables_formula_bar_import_wizard_and_quick_stats() {
    let mut doc = lumencraft_doc::ProjectDocument::default();
    let mut mv = MultipleVariablesTable::new(3, 6).expect("mv table");
    mv.variables[0].title = String::from("Group");
    mv.variables[1].title = String::from("Response");
    mv.variables[2].title = String::from("LogResp");
    for (r, label) in ["Trt", "Ctrl", "Trt", "Ctrl", "Tret", ""].iter().enumerate() {
        if !label.is_empty() {
            mv.variables[0].cells[r] = Cell::text(*label);
        }
    }
    for (r, val) in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0].iter().enumerate() {
        mv.variables[1].cells[r] = Cell::number(*val);
    }
    mv.variables[0].refresh_inferred_role();
    mv.variables[1].refresh_inferred_role();

    let mut sheet = DataSheet::new("mv-pipe", "Multivar Cohort", TableArchetype::MultipleVariables(mv)).expect("sheet");
    sheet.set_excluded(CellCoord::new(4, 1, 0), true).expect("exclude r=4");
    doc.add_data_sheet(sheet).expect("add sheet");

    let mut grid = GridViewState::default();
    let ds = doc.data_sheet(&SheetId::from("mv-pipe")).expect("ds");

    // Variables... recoding & level ordering intents.
    grid.variables_dialog.open_for_column(ds, 0);
    grid.variables_dialog.merge_levels_into(&["Tret"], "Trt");
    grid.variables_dialog.set_level_order(
        LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("Trt")]), reversed: false },
        Some(ds),
    );
    let (edited_count, preview_levels) = grid.variables_dialog.preview_recode(ds).expect("preview");
    assert_eq!(edited_count, 1);
    assert_eq!(preview_levels.len(), 2);
    assert_eq!(grid.variables_dialog.build_recode_command(&ds.id).expect("recode cmd").command_id, "table.recodeLevels");
    assert_eq!(grid.variables_dialog.build_order_command(&ds.id).expect("order cmd").command_id, "table.setCategoricalOrder");

    // Formula bar autocomplete & intent.
    grid.formula_bar.open_for_column(ds, 2, Some(&doc));
    grid.formula_bar.set_input("= Resp", None, &doc);
    assert!(grid.formula_bar.completions.iter().any(|c| c.insert_text == "Response"));
    assert!(grid.formula_bar.accept_completion(&doc));
    assert!(grid.formula_bar.is_valid);
    assert_eq!(grid.formula_bar.commit_intent(&ds.id).expect("formula cmd").command_id, "table.setCalculatedVariable");

    // 4-stage Import & Paste Special / Paste Link wizard (Source -> Filter -> Placement -> Provenance).
    grid.import_wizard.stage = ImportWizardStage::Source;
    grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    grid.import_wizard.raw_clipboard_text = String::from("12,5*;18,5\n25,0;30,0");
    grid.import_wizard.delimiter = ';';
    grid.import_wizard.decimal_separator = ',';
    grid.import_wizard.next_stage();
    assert_eq!(grid.import_wizard.stage, ImportWizardStage::Filter);
    grid.import_wizard.next_stage();
    assert_eq!(grid.import_wizard.stage, ImportWizardStage::Placement);
    grid.import_wizard.placement.transpose = true;
    grid.import_wizard.next_stage();
    assert_eq!(grid.import_wizard.stage, ImportWizardStage::Provenance);
    let preview = grid.import_wizard.preview_grid(Some(&doc));
    assert_eq!(preview.len(), 2);
    assert!(preview[0][0].excluded);
    assert_eq!(preview[0][0].value.finite_f64(), Some(12.5));
    assert_eq!(preview[0][1].value.finite_f64(), Some(25.0));

    grid.import_wizard.is_live_link = true;
    grid.import_wizard.origin_kind = ImportOriginKind::InternalResultSheet;
    grid.import_wizard.source_sheet_id = Some(SheetId::from("r-norm"));
    assert_eq!(grid.import_wizard.preview_border_color(Some(&doc)), Some(LinkBorderColor::Red));
    assert_eq!(grid.import_wizard.build_command_intent(&ds.id, Some(&doc)).command_id, "table.pasteLink");

    // Selection quick-stats footer (n=5, excluded=1 across column 1 rows 0..6).
    let stats = GridQuickStats::from_data_sheet(ds, GridCellCoord::new(0, 1, 0), GridCellCoord::new(5, 1, 0));
    assert_eq!(stats.n, 5);
    assert_eq!(stats.excluded, 1);
    assert!((stats.sum.expect("sum") - 160.0).abs() < 1e-12);
    assert!((stats.mean.expect("mean") - 32.0).abs() < 1e-12);
    assert!((stats.median.expect("median") - 30.0).abs() < 1e-12);
    let footer = stats.format_footer();
    assert!(footer.contains("n=5") && footer.contains("excluded=1") && footer.contains("SEM="));

    // Boundary jump navigation & exclude intent emission.
    grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    grid.navigate(ds, MoveDirection::Down, false, true);
    assert_eq!(grid.tool.active, Some(GridCellCoord::new(4, 0, 0)));
    assert_eq!(grid.emit_toggle_exclude_intents(ds, true), 1);
    assert_eq!(grid.tool.pending_intents.len(), 1);
}

#[test]
fn emitted_intents_round_trip_through_ui_app_execute_command() {
    let mut app = UiApp::new();

    // 1. sheet.createData with auto_graph, auto_info, replicates, and cols.
    let mut xy_draft = TableCreationDraft::for_kind(TableKind::Xy).with_replicates(3);
    xy_draft.sheet_id = String::from("assay-xy");
    xy_draft.title = String::from("Dose Curve");
    xy_draft.data_set_count = 2;
    xy_draft.row_count = 8;
    xy_draft.create_default_graph = true;
    xy_draft.create_paired_info = true;
    let xy_intent = xy_draft.to_command_intent();
    app.execute_command(&xy_intent.command_id, xy_intent.params).expect("sheet.createData xy");

    let created_xy = app.document().data_sheet(&SheetId::from("assay-xy")).expect("assay-xy");
    assert_eq!(created_xy.archetype.column_count(), 2);
    assert_eq!(total_flat_data_subcolumns(&created_xy.archetype), 6);
    assert!(created_xy.info_sheet_id.is_some(), "auto_info should create paired info sheet");
    assert_eq!(app.document().info_sheets.len(), 1);
    assert_eq!(app.document().graph_sheets.len(), 1, "auto_graph should create default graph");

    let mut col_draft = TableCreationDraft::for_kind(TableKind::Column).with_column_design(ColumnDesignHint::PairedRepeatedMeasures);
    col_draft.sheet_id = String::from("col-paste");
    col_draft.title = String::from("Paired Cohort");
    col_draft.data_set_count = 2;
    col_draft.row_count = 4;
    let col_intent = col_draft.to_command_intent();
    app.execute_command(&col_intent.command_id, col_intent.params).expect("sheet.createData column");

    // 2. sheet.select mode preservation vs re-anchoring via WorkspaceNavState.
    let mut scratch_doc = app.document().clone();
    let mut scratch_nav = WorkspaceNavState::default();
    let xy_id = SheetId::from("assay-xy");
    scratch_nav.click_navigator_entry(&mut scratch_doc, &xy_id).expect("nav select");
    let nav_select = scratch_nav.pending_intents.pop().expect("nav intent");
    app.execute_command(&nav_select.command_id, nav_select.params).expect("exec nav select");
    assert_eq!(app.document().family_anchor_sheet_id, Some(xy_id.clone()));

    let graph_id = app.document().graph_sheets[0].id.clone();
    scratch_nav.click_family_entry(&mut scratch_doc, &graph_id, false).expect("family single-click");
    let fam_click = scratch_nav.pending_intents.pop().expect("family intent");
    app.execute_command(&fam_click.command_id, fam_click.params).expect("exec family click");
    assert_eq!(app.document().active_sheet_id.as_ref(), Some(&graph_id));
    assert_eq!(app.document().family_anchor_sheet_id, Some(xy_id.clone()), "family click must keep anchor");

    let info_id = app.document().data_sheet(&xy_id).expect("xy").info_sheet_id.clone().expect("info id");
    scratch_nav.click_family_entry(&mut scratch_doc, &info_id, true).expect("family double-click");
    let dbl_click = scratch_nav.pending_intents.pop().expect("double-click intent");
    app.execute_command(&dbl_click.command_id, dbl_click.params).expect("exec double-click");
    assert_eq!(app.document().family_anchor_sheet_id, Some(info_id), "double-click must shift anchor");

    scratch_nav.click_navigator_entry(&mut scratch_doc, &xy_id).expect("re-anchor back");
    let back_intent = scratch_nav.pending_intents.pop().expect("back intent");
    app.execute_command(&back_intent.command_id, back_intent.params).expect("exec back");
}

#[test]
fn emitted_paste_paste_link_and_exclude_intents_round_trip() {
    let mut app = UiApp::new();
    let mut xy_draft = TableCreationDraft::for_kind(TableKind::Xy).with_replicates(3);
    xy_draft.sheet_id = String::from("assay-xy");
    xy_draft.data_set_count = 2;
    xy_draft.row_count = 6;
    let xy_intent = xy_draft.to_command_intent();
    app.execute_command(&xy_intent.command_id, xy_intent.params).expect("create assay-xy");
    let xy_id = SheetId::from("assay-xy");

    let mut col_draft = TableCreationDraft::for_kind(TableKind::Column);
    col_draft.sheet_id = String::from("col-paste");
    col_draft.title = String::from("Paired Cohort");
    col_draft.data_set_count = 2;
    col_draft.row_count = 4;
    let col_intent = col_draft.to_command_intent();
    app.execute_command(&col_intent.command_id, col_intent.params).expect("create col-paste");

    // 3. table.paste (clipboard semicolon delimited + comma decimals + '*' exclusion + transpose).
    let mut grid = GridViewState::default();
    grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    grid.import_wizard.is_live_link = false;
    grid.import_wizard.raw_clipboard_text = String::from("GroupA;10,5*;20,0\nGroupB;30,5;40,5");
    grid.import_wizard.delimiter = ';';
    grid.import_wizard.decimal_separator = ',';
    grid.import_wizard.placement.transpose = true;
    grid.import_wizard.placement.propagate_title_header = true;
    let paste_id = SheetId::from("col-paste");
    let paste_intent = grid.import_wizard.build_command_intent(&paste_id, Some(app.document()));
    assert_eq!(paste_intent.command_id, "table.paste");
    app.execute_command(&paste_intent.command_id, paste_intent.params).expect("exec table.paste");

    let pasted_ds = app.document().data_sheet(&paste_id).expect("col-paste");
    assert_eq!(pasted_ds.cell(CellCoord::new(0, 0, 0)).expect("c00").value, CellValue::Text(String::from("GroupA")));
    let c10 = pasted_ds.cell(CellCoord::new(1, 0, 0)).expect("c10");
    assert!(c10.excluded);
    assert_eq!(c10.value.finite_f64(), Some(10.5));
    assert_eq!(pasted_ds.cell(CellCoord::new(2, 0, 0)).expect("c20").value.finite_f64(), Some(20.0));
    assert_eq!(pasted_ds.cell(CellCoord::new(1, 1, 0)).expect("c11").value.finite_f64(), Some(30.5));

    // 4. table.pasteLink (internal DataSheet live link + Blue border + custom provenance_summary).
    let mut link_target = TableCreationDraft::for_kind(TableKind::Column);
    link_target.sheet_id = String::from("col-linked");
    link_target.title = String::from("Linked Cohort");
    link_target.data_set_count = 2;
    link_target.row_count = 4;
    let lt_intent = link_target.to_command_intent();
    app.execute_command(&lt_intent.command_id, lt_intent.params).expect("create col-linked");

    grid.import_wizard = Default::default();
    grid.import_wizard.is_live_link = true;
    grid.import_wizard.origin_kind = ImportOriginKind::InternalDataSheet;
    grid.import_wizard.source_sheet_id = Some(paste_id);
    grid.import_wizard.source_range = SheetCellRange::new(CellCoord::new(1, 0, 0), CellCoord::new(2, 1, 0));
    grid.import_wizard.provenance_note = String::from("Linked from Paired Cohort");
    let linked_id = SheetId::from("col-linked");
    let paste_link_intent = grid.import_wizard.build_command_intent(&linked_id, Some(app.document()));
    assert_eq!(paste_link_intent.command_id, "table.pasteLink");
    app.execute_command(&paste_link_intent.command_id, paste_link_intent.params).expect("exec table.pasteLink");

    let linked_ds = app.document().data_sheet(&linked_id).expect("col-linked");
    assert_eq!(linked_ds.linked_regions.len(), 1);
    assert_eq!(linked_ds.linked_regions[0].border_color(), Some(LinkBorderColor::Blue));
    assert_eq!(linked_ds.linked_regions[0].provenance_summary, "Linked from Paired Cohort");
    assert_eq!(linked_ds.cell(CellCoord::new(1, 1, 0)).expect("linked cell").value.finite_f64(), Some(40.5));

    // 5. table.excludeCells batched partial-subcolumn rectangular ranges on replicate Xy sheet.
    grid.tool.pending_intents.clear();
    grid.tool.anchor = Some(GridCellCoord::new(0, 0, 1));
    grid.tool.active = Some(GridCellCoord::new(2, 1, 1));
    let excl_count = grid.emit_toggle_exclude_intents(app.document().data_sheet(&xy_id).expect("xy"), true);
    assert_eq!(excl_count, 2, "partial leading/trailing replicates should emit 2 range intents");
    for intent in grid.tool.pending_intents.drain(..) {
        app.execute_command(&intent.command_id, intent.params).expect("exec batched exclude range");
    }
    let xy_after = app.document().data_sheet(&xy_id).expect("xy after");
    assert!(!xy_after.cell(CellCoord::new(0, 0, 0)).expect("000").excluded);
    assert!(xy_after.cell(CellCoord::new(0, 0, 1)).expect("001").excluded);
    assert!(xy_after.cell(CellCoord::new(2, 0, 2)).expect("202").excluded);
    assert!(xy_after.cell(CellCoord::new(1, 1, 1)).expect("111").excluded);
    assert!(!xy_after.cell(CellCoord::new(0, 1, 2)).expect("012").excluded);
}

#[test]
fn emitted_multivar_formula_and_clone_intents_round_trip() {
    let mut app = UiApp::new();
    let mut grid = GridViewState::default();

    let mut xy_draft = TableCreationDraft::for_kind(TableKind::Xy).with_replicates(2);
    xy_draft.sheet_id = String::from("assay-xy");
    let xy_intent = xy_draft.to_command_intent();
    app.execute_command(&xy_intent.command_id, xy_intent.params).expect("create assay-xy");
    let xy_id = SheetId::from("assay-xy");

    // 6. MultipleVariables intents: table.setVariableRole, table.recodeLevels, table.setCategoricalOrder, table.setCalculatedVariable.
    let mut mv_draft = TableCreationDraft::for_kind(TableKind::MultipleVariables);
    mv_draft.sheet_id = String::from("mv-rt");
    mv_draft.title = String::from("Multivar Roundtrip");
    mv_draft.data_set_count = 3;
    mv_draft.row_count = 4;
    let mv_intent = mv_draft.to_command_intent();
    app.execute_command(&mv_intent.command_id, mv_intent.params).expect("create mv-rt");

    let mv_id = SheetId::from("mv-rt");
    app.execute_command(
        "table.paste",
        serde_json::json!({
            "sheet_id": "mv-rt",
            "text": "Ctrl\t10\t0\nTret\t20\t0\nCtrl\t30\t0\nTrt\t40\t0",
            "delimiter": "\t",
        }),
    )
    .expect("seed mv-rt");

    grid.variables_dialog.open_for_column(app.document().data_sheet(&mv_id).expect("mv"), 0);
    grid.variables_dialog.pending_role = Some(VariableRole::Categorical);
    let role_cmd = grid.variables_dialog.build_role_command(&mv_id).expect("role cmd");
    app.execute_command(&role_cmd.command_id, role_cmd.params).expect("exec role");

    grid.variables_dialog.merge_levels_into(&["Tret"], "Trt");
    let recode_cmd = grid.variables_dialog.build_recode_command(&mv_id).expect("recode cmd");
    app.execute_command(&recode_cmd.command_id, recode_cmd.params).expect("exec recode");

    grid.variables_dialog.set_level_order(
        LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Trt"), String::from("Ctrl")]), reversed: false },
        app.document().data_sheet(&mv_id),
    );
    let order_cmd = grid.variables_dialog.build_order_command(&mv_id).expect("order cmd");
    app.execute_command(&order_cmd.command_id, order_cmd.params).expect("exec order");

    grid.formula_bar.open_for_column(app.document().data_sheet(&mv_id).expect("mv"), 2, Some(app.document()));
    grid.formula_bar.set_input("= B * 2", None, app.document());
    assert!(grid.formula_bar.is_valid);
    let calc_cmd = grid.formula_bar.commit_intent(&mv_id).expect("calc cmd");
    app.execute_command(&calc_cmd.command_id, calc_cmd.params).expect("exec calculated var");

    let mv_after = app.document().data_sheet(&mv_id).expect("mv after");
    assert!(matches!(&mv_after.archetype, TableArchetype::MultipleVariables(_)));
    if let TableArchetype::MultipleVariables(mv) = &mv_after.archetype {
        assert_eq!(mv.variables[0].role, VariableRole::Categorical);
        assert_eq!(mv.variables[0].cells[1].value, CellValue::Text(String::from("Trt")));
        assert_eq!(
            mv.variables[0].level_order,
            LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Trt"), String::from("Ctrl")]), reversed: false }
        );
        assert!(matches!(mv.variables[2].source, VariableSource::Calculated { .. }));
        assert_eq!(mv.variables[2].cells[3].value.finite_f64(), Some(80.0));
    }

    // 7. sheet.clonePipeline via ClonePipelineDraft.
    let clone_draft = ClonePipelineDraft {
        source_sheet_id: Some(xy_id),
        new_sheet_id: String::from("assay-xy-clone"),
        options: CloneDataSheetOptions {
            title: String::from("Dose Curve Clone"),
            keep_x_values: true,
            clear_y_values: true,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: None,
        },
        ..ClonePipelineDraft::default()
    };
    let clone_cmd = clone_draft.to_command_intent().expect("clone intent");
    app.execute_command(&clone_cmd.command_id, clone_cmd.params).expect("exec clonePipeline");
    assert!(app.document().data_sheet(&SheetId::from("assay-xy-clone")).is_some());
}
