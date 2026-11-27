#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Spike S1 1,000,000-cell grid virtualization benchmark and end-to-end tests for Project
//! Navigator, Family pane anchor stability, provenance badges, Welcome wizard, two-tier headers,
//! `MultipleVariables` role/formula tooling, 4-stage Import/Paste Link wizard, and quick stats.

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    GraphSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet,
    LayoutSpec, LevelOrderKind, LevelOrderSpec, LinkBorderColor, LinkSource, LinkedDataRegion, MultipleVariablesTable, NestedTable, NormalizeAnchor,
    ProjectDocument, RectPt, ResultSheet, ResultStatus, SheetCellRange, SheetId, SheetSection, SubcolumnMode, TableArchetype, TableKind,
    TransformAnalysisSpec, VariableRole, VariableSource,
};
use curvebook_engine::Session;
use curvebook_tools::{GridCellCoord, GridViewportSpec, ToolCommandIntent};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::grid_view::{
    GridQuickStats, GridViewState, ImportOriginKind, ImportWizardStage, MoveDirection, provenance_border_rgba, total_flat_data_subcolumns,
};
use curvebook_ui_egui::workspace_nav::{
    ClonePipelineDraft, FamilyRelation, NavigatorBadgeColor, NavigatorBadgeKind, NavigatorNestingMode, SAMPLE_FIXTURES, TableCreationDraft,
    WelcomeWizardState, WorkspaceNavState, instantiate_sample_fixture,
};

fn make_link(id: &str, source: LinkSource, start: CellCoord, end: CellCoord, summary: &str) -> LinkedDataRegion {
    LinkedDataRegion {
        id: id.into(),
        spec: ImportLinkSpec { source, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() },
        target_range: SheetCellRange::new(start, end),
        provenance_summary: summary.into(),
    }
}

fn internal_link_src(sheet_id: &str, tab_id: Option<&str>, kind: InternalLinkKind, start: CellCoord, end: CellCoord) -> LinkSource {
    LinkSource::InternalSheet { sheet_id: SheetId::from(sheet_id), tab_id: tab_id.map(String::from), kind, range: SheetCellRange::new(start, end) }
}

fn exec_intent(app: &mut UiApp, intent: ToolCommandIntent) {
    app.execute_command(&intent.command_id, intent.params).expect("execute intent");
}

fn create_table(app: &mut UiApp, kind: TableKind, id: &str, title: &str, (cols, rows): (usize, usize), reps: Option<usize>, auto_extras: bool) {
    let mut draft = TableCreationDraft::for_kind(kind);
    if let Some(r) = reps {
        draft = draft.with_replicates(r);
    }
    draft.sheet_id = id.into();
    draft.title = title.into();
    draft.data_set_count = cols;
    draft.row_count = rows;
    draft.create_default_graph = auto_extras;
    draft.create_paired_info = auto_extras;
    exec_intent(app, draft.to_command_intent());
}

fn build_large_grouped_doc() -> Result<ProjectDocument, curvebook_doc::DocError> {
    let mut doc = ProjectDocument::default();
    let mut src_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    for r in 0..4 {
        src_col.data_sets[0].cells[r][0] = Cell::number(100.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("tbl-donor", "Donor Table", TableArchetype::Column(src_col))?)?;

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
    let (s0, s2) = (CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0));
    big_sheet.linked_regions.push(make_link(
        "link-big-blue",
        internal_link_src("tbl-donor", None, InternalLinkKind::DataSheet, s0, s2),
        s0,
        s2,
        "Linked from Donor Table",
    ));
    doc.add_data_sheet(big_sheet)?;
    doc.select_in_navigator(&SheetId::from("tbl-big"))?;
    Ok(doc)
}

#[test]
fn spike_s1_grouped_one_million_cells_120_frames_sub_16ms() {
    let doc = build_large_grouped_doc().expect("build 1M grouped doc");
    let big = doc.data_sheet(&SheetId::from("tbl-big")).expect("big sheet exists");
    assert_eq!((big.archetype.row_count(), total_flat_data_subcolumns(&big.archetype)), (1_000, 1_000));

    let mut app = UiApp::from_session(Session::from_document(doc));
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
    let tl = &initial_frame.rows[0].cells[0];
    assert!(tl.excluded && tl.visual.italic && tl.visual.strikethrough && tl.visual.text_color.b > tl.visual.text_color.r);
    assert_eq!(
        (tl.visual.excluded_badge.as_deref(), tl.visual.provenance_border, tl.visual.provenance_border_rgba),
        (Some("*"), Some(LinkBorderColor::Blue), Some(provenance_border_rgba(LinkBorderColor::Blue)))
    );
    assert_eq!(app.nav.family_anchor_sheet_id, Some(SheetId::from("tbl-big")));

    let mut frame_ms = Vec::with_capacity(120);
    let (mut last_visible_rows, mut last_visible_cols) = (0usize, 0usize);
    for frame_idx in 0..120usize {
        let start = Instant::now();
        app.grid.tool.viewport.scroll_y_px = ((frame_idx * 192) % (950 * 24)) as f64;
        app.grid.tool.viewport.scroll_x_px = ((frame_idx * 330) % (960 * 110)) as f64;
        let (target_row, target_group, target_subcol) = ((frame_idx * 8) % 990, (frame_idx * 2) % 245, frame_idx % 4);
        if frame_idx % 3 == 0 {
            app.grid.tool.select_single(GridCellCoord::new(target_row, target_group, target_subcol));
        } else {
            app.grid.tool.anchor = Some(GridCellCoord::new(target_row, target_group, 0));
            app.grid.tool.active = Some(GridCellCoord::new(target_row + 5, target_group + 1, 3));
        }
        app.sync_views();
        let window = app.visible_grid_window().expect("window visible");
        let frame = app.grid.visible_frame(app.document()).expect("frame visible");
        assert!(window.end_row > window.start_row && window.end_col > window.start_col);
        assert_eq!((frame.total_rows, frame.total_flat_cols), (1_000, 1_000));
        assert!(!frame.tier1_groups.is_empty() && !frame.tier2_subcolumns.is_empty() && !app.nav.entries.is_empty());
        last_visible_rows = frame.rows.len();
        last_visible_cols = frame.tier2_subcolumns.len();
        frame_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
    }

    let mut sorted = frame_ms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let (p50, p95, max_ms) = (sorted[sorted.len() / 2], sorted[(sorted.len() * 95) / 100], *sorted.last().expect("non-empty timings"));
    let mean_ms = frame_ms.iter().sum::<f64>() / (frame_ms.len() as f64);
    println!(
        "SPIKE_S1_UI_1M grouped 120 frames: p50={p50:.3}ms p95={p95:.3}ms mean={mean_ms:.3}ms max={max_ms:.3}ms visible={last_visible_rows}x{last_visible_cols}"
    );
    let median_budget_ms = if cfg!(debug_assertions) { 5.0 } else { 1.0 };
    let frame_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(p50 < median_budget_ms, "p50 median frame time {p50:.3}ms exceeded {median_budget_ms:.2}ms budget");
    assert!(p95 < frame_budget_ms, "p95 frame time {p95:.3}ms exceeded {frame_budget_ms:.2}ms budget");
    assert!(max_ms < frame_budget_ms, "max frame time {max_ms:.3}ms exceeded {frame_budget_ms:.2}ms budget");
}

#[test]
fn spike_s1_grouped_one_million_cells_select_all_and_exclude_budget() {
    let mut app = UiApp::from_session(Session::from_document(build_large_grouped_doc().expect("build 1M cell doc")));
    app.sync_views();
    app.grid.tool.anchor = Some(GridCellCoord::new(0, 0, 0));
    app.grid.tool.active = Some(GridCellCoord::new(999, 249, 3));
    let start = Instant::now();
    app.sync_views();
    let frame = app.grid.visible_frame(app.document()).expect("select all visible frame");
    let mut detached = std::mem::take(&mut app.grid);
    let emitted = detached.emit_toggle_exclude_intents(app.document().data_sheet(&SheetId::from("tbl-big")).expect("tbl-big"), true);
    app.grid = detached;
    let select_all_ms = start.elapsed().as_secs_f64() * 1_000.0;
    println!("SPIKE_S1_UI_1M Select All (1,000,000 cells): sync+frame+exclude={select_all_ms:.3}ms intents={emitted}");

    let select_all_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(select_all_ms < select_all_budget_ms, "Select All frame time {select_all_ms:.3}ms exceeded {select_all_budget_ms:.2}ms budget");
    assert!(app.grid.quick_stats.is_none() && app.grid.extended_stats.expect("extended stats").selected_cells == 1_000_000);
    assert_eq!((frame.total_rows, emitted, app.grid.tool.pending_intents.len()), (1_000, 1, 1));
    let p = &app.grid.tool.pending_intents[0];
    assert_eq!(
        (p.command_id.as_str(), &p.params["range"]["start"]["row"], &p.params["range"]["end"]["col"], &p.params["range"]["end"]["subcol"]),
        ("table.excludeCells", &serde_json::json!(0), &serde_json::json!(249), &serde_json::json!(3))
    );
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
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(DataSheet::new("mv-1m", "Cohort 1M", TableArchetype::MultipleVariables(mv)).expect("sheet")).expect("add mv-1m");
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
        assert_eq!((frame.total_rows, frame.total_flat_cols), (1_000, 1_000));
        assert!(frame.tier1_groups.iter().all(|g| g.variable_role.is_some()));
        frame_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut sorted = frame_ms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let (p50, max_ms) = (sorted[sorted.len() / 2], sorted.last().copied().unwrap_or(0.0));
    let mean_ms = frame_ms.iter().sum::<f64>() / (frame_ms.len() as f64);
    println!("SPIKE_S1_UI_1M multivar 30 frames: p50={p50:.3}ms mean={mean_ms:.3}ms max={max_ms:.3}ms");
    let median_budget_ms = if cfg!(debug_assertions) { 5.0 } else { 1.0 };
    let max_budget_ms = if cfg!(debug_assertions) { 100.0 } else { 16.67 };
    assert!(p50 < median_budget_ms, "multivar p50 median frame time {p50:.3}ms exceeded {median_budget_ms:.2}ms budget");
    assert!(max_ms < max_budget_ms, "multivar max frame time {max_ms:.3}ms exceeded {max_budget_ms:.2}ms budget");

    let role_strip = app.grid.variable_role_strip(app.document().data_sheet(&SheetId::from("mv-1m")).expect("mv sheet"));
    assert!(role_strip.len() == 1_000 && role_strip[999].is_calculated);
}

#[test]
fn two_tier_headers_and_all_four_provenance_colors() {
    let mut doc = ProjectDocument::default();
    let mut nested = NestedTable::new(2, 3, 4).expect("nested table");
    nested.groups[0].title = "Wildtype".into();
    nested.groups[0].subcolumn_titles = vec!["WT:1".into(), "WT:2".into(), "WT:3".into()];
    nested.groups[1].title = "Mutant".into();
    nested.groups[1].subcolumn_titles = vec!["Mut:1".into(), "Mut:2".into(), "Mut:3".into()];
    for r in 0..4 {
        for s in 0..3 {
            nested.groups[0].cells[r][s] = Cell::number(10.0 + r as f64 + s as f64);
            nested.groups[1].cells[r][s] = Cell::number(20.0 + r as f64 + s as f64);
        }
    }
    let mut target = DataSheet::new("tbl-nested", "Nested Target", TableArchetype::Nested(nested)).expect("target");
    target.set_excluded(CellCoord::new(0, 0, 1), true).expect("exclude nested cell");
    let z = CellCoord::new(0, 0, 0);
    for (id, src, col, sub) in [
        ("link-blue", internal_link_src("src-data", None, InternalLinkKind::DataSheet, z, z), 0, 0),
        ("link-green", internal_link_src("res-norm", Some("transformed"), InternalLinkKind::AnalyzableResultSheet, z, z), 0, 2),
        ("link-red", internal_link_src("res-stats", Some("summary"), InternalLinkKind::TerminalResultSheet, z, z), 1, 0),
        ("link-charcoal", LinkSource::ExternalDelimitedFile { path: "/data/plate.tsv".into(), delimiter: '\t', decimal_separator: '.' }, 1, 2),
    ] {
        let c = CellCoord::new(0, col, sub);
        target.linked_regions.push(make_link(id, src, c, c, "test link"));
    }
    doc.add_data_sheet(target).expect("add target");

    let mut grid = GridViewState::default();
    let frame = grid.visible_frame(&doc).expect("frame");
    assert_eq!(
        (
            frame.tier1_groups.len(),
            frame.tier1_groups[0].title.as_str(),
            frame.tier1_groups[0].span,
            frame.tier1_groups[1].title.as_str(),
            frame.tier1_groups[1].start_flat_col
        ),
        (2, "Wildtype", 3, "Mutant", 3)
    );
    assert_eq!(
        (frame.tier2_subcolumns.len(), frame.tier2_subcolumns[0].label.as_str(), frame.tier2_subcolumns[5].label.as_str()),
        (6, "WT:1", "Mut:3")
    );
    let row0 = &frame.rows[0].cells;
    assert!(row0[1].excluded && row0[1].visual.italic && row0[1].visual.strikethrough);
    assert_eq!(
        [row0[0].visual.provenance_border, row0[2].visual.provenance_border, row0[3].visual.provenance_border, row0[5].visual.provenance_border],
        [Some(LinkBorderColor::Blue), Some(LinkBorderColor::Green), Some(LinkBorderColor::Red), Some(LinkBorderColor::Charcoal)]
    );
    assert!(grid.inspect_link_at(doc.data_sheet(&SheetId::from("tbl-nested")).expect("sheet"), GridCellCoord::new(0, 1, 2)));
    assert_eq!((grid.format_link_inspector.open, grid.format_link_inspector.border_color), (true, Some(LinkBorderColor::Charcoal)));
}

#[test]
fn navigator_five_folders_nesting_transitive_bold_badges_and_family_anchor() {
    let mut doc = ProjectDocument::default();
    let mut d1 = instantiate_sample_fixture("dose-response-4pl", "d1").expect("d1");
    d1.title = "Binding Assay".into();
    d1.frozen = true;
    d1.linked_regions.push(make_link(
        "ext-1",
        LinkSource::ExternalDelimitedFile { path: "/data/assay.csv".into(), delimiter: ',', decimal_separator: '.' },
        CellCoord::new(0, 0, 0),
        CellCoord::new(3, 0, 0),
        "/data/assay.csv",
    ));
    doc.add_data_sheet(d1).expect("add d1");
    let mut d2 = instantiate_sample_fixture("unpaired-treatment-groups", "d2").expect("d2");
    d2.title = "Unrelated Assay".into();
    doc.add_data_sheet(d2).expect("add d2");
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Metadata").expect("info")).expect("add info");

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
    .expect("norm");
    doc.add_result_sheet(ResultSheet::new(norm_spec).expect("r-norm")).expect("add r-norm");
    let mut r_desc =
        ResultSheet::new(AnalysisSpec::new("r-desc", "r-norm", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("desc"))
            .expect("r-desc");
    r_desc.status = ResultStatus::Dirty;
    doc.add_result_sheet(r_desc).expect("add r-desc");

    let mut g1 = GraphSheet::new_uninitialized_for_data("g1", doc.data_sheet(&SheetId::from("d1")).expect("d1")).expect("g1");
    g1.title = "Normalized Curve".into();
    if let Some(s) = g1.spec.series.first_mut() {
        s.source_sheet_id = SheetId::from("r-desc");
    }
    doc.add_graph_sheet(g1).expect("add g1");
    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: "panel-a".into(),
        graph_sheet_id: SheetId::from("g1"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    doc.add_layout_sheet(LayoutSheet::new("lay-1", "Figure 1", layout_spec).expect("lay-1")).expect("add lay-1");

    let (mut session, mut nav) = (Session::from_document(doc), WorkspaceNavState::default());
    let drain = |nav: &mut WorkspaceNavState, sess: &mut Session| {
        for intent in nav.pending_intents.drain(..) {
            sess.execute(&intent.command_id, intent.params).expect("execute nav intent");
        }
        nav.sync_from_document(sess.document());
    };
    nav.click_navigator_entry(session.document(), &SheetId::from("lay-1")).expect("select lay-1");
    drain(&mut nav, &mut session);

    assert_eq!(
        nav.folders.iter().map(|f| f.section).collect::<Vec<_>>(),
        [SheetSection::Data, SheetSection::Info, SheetSection::Analyses, SheetSection::Graphs, SheetSection::Layouts]
    );
    for id in ["d1", "r-norm", "r-desc", "g1", "lay-1"] {
        assert!(nav.entries.iter().find(|e| e.sheet_id == SheetId::from(id)).expect("entry").is_bold, "{id}");
    }
    assert!(!nav.entries.iter().find(|e| e.sheet_id == SheetId::from("d2")).expect("d2").is_bold);
    let d1_e = nav.entries.iter().find(|e| e.sheet_id == SheetId::from("d1")).expect("d1");
    assert!(d1_e.is_frozen && d1_e.link_colors.contains(&LinkBorderColor::Charcoal));
    assert!(d1_e.badges.iter().any(|b| b.kind == NavigatorBadgeKind::Frozen && b.color == NavigatorBadgeColor::Charcoal));
    assert!(
        nav.entries
            .iter()
            .find(|e| e.sheet_id == SheetId::from("r-desc"))
            .expect("r-desc")
            .badges
            .iter()
            .any(|b| b.kind == NavigatorBadgeKind::Dirty && b.color == NavigatorBadgeColor::Red)
    );

    nav.set_nesting_mode(NavigatorNestingMode::NestedResultsUnderData, session.document());
    let data_f = nav.folders.iter().find(|f| f.section == SheetSection::Data).expect("data");
    assert!(nav.folders.iter().find(|f| f.section == SheetSection::Analyses).expect("results").entries.is_empty());
    assert_eq!(data_f.entries.iter().find(|e| e.sheet_id == SheetId::from("r-norm")).expect("r-norm").nesting_depth, 1);
    assert_eq!(data_f.entries.iter().find(|e| e.sheet_id == SheetId::from("r-desc")).expect("r-desc").nesting_depth, 2);

    nav.set_search_query("Normalized", session.document());
    assert_eq!(nav.filtered_entries().iter().map(|e| e.sheet_id.as_str()).collect::<Vec<_>>(), ["g1"]);
    nav.set_search_query("", session.document());

    // Stable-anchor Family pane behavior: single-click keeps anchor, double-click shifts anchor.
    nav.click_navigator_entry(session.document(), &SheetId::from("d1")).expect("anchor d1");
    drain(&mut nav, &mut session);
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("d1")));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("d1") && n.relation == FamilyRelation::Anchor));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("lay-1") && n.relation == FamilyRelation::Downstream));
    for target in ["r-desc", "g1"] {
        nav.click_family_entry(session.document(), &SheetId::from(target), false).expect("single-click");
        drain(&mut nav, &mut session);
        assert_eq!((&session.document().active_sheet_id, &nav.family_anchor_sheet_id), (&Some(SheetId::from(target)), &Some(SheetId::from("d1"))));
    }
    nav.click_family_entry(session.document(), &SheetId::from("r-norm"), true).expect("double-click");
    drain(&mut nav, &mut session);
    assert_eq!(nav.family_anchor_sheet_id, Some(SheetId::from("r-norm")));
    assert!(nav.family_nodes.iter().any(|n| n.sheet_id == SheetId::from("d1") && n.relation == FamilyRelation::Upstream));
}

#[test]
fn welcome_wizard_all_archetypes_fixtures_and_clone_pipeline() {
    let (mut session, mut wizard) = (Session::new(), WelcomeWizardState::default());
    for (idx, kind) in TableKind::ALL.iter().enumerate() {
        wizard.select_archetype(*kind);
        wizard.set_replicates(4);
        wizard.table_draft.sheet_id = format!("tbl-kind-{idx}");
        let intent = wizard.table_draft.to_command_intent();
        session.execute(&intent.command_id, intent.params).expect("create typed table");
        assert_eq!(session.document().data_sheet(&SheetId::from(format!("tbl-kind-{idx}"))).expect("sheet").kind(), *kind);
        if idx + 1 < TableKind::ALL.len() {
            wizard.focus_next_archetype();
        }
    }
    assert_eq!(
        TableCreationDraft::for_kind(TableKind::Column)
            .with_column_design(ColumnDesignHint::PairedRepeatedMeasures)
            .build_archetype()
            .expect("paired")
            .kind(),
        TableKind::Column
    );
    assert_eq!(SAMPLE_FIXTURES.len(), 9);
    for (idx, fx) in SAMPLE_FIXTURES.iter().enumerate() {
        assert_eq!(instantiate_sample_fixture(fx.id, format!("fx-{idx}")).expect("fixture").kind(), fx.kind);
    }

    let mut doc = session.document().clone();
    doc.add_data_sheet(instantiate_sample_fixture("dose-response-4pl", "src-xy").expect("src-xy")).expect("add src-xy");
    let src_res = AnalysisSpec::new(
        "src-res",
        "src-xy",
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: None,
            error_propagation: Default::default(),
            pharmacology_preset: None,
            interchange_xy: false,
            zero_x_substitute: None,
            asymmetric_ci_policy: Default::default(),
        }),
    )
    .expect("src-res");
    doc.add_result_sheet(ResultSheet::new(src_res).expect("result")).expect("add src-res");
    doc.add_graph_sheet(
        GraphSheet::new_uninitialized_for_data("src-graph", doc.data_sheet(&SheetId::from("src-xy")).expect("src-xy")).expect("src-graph"),
    )
    .expect("add src-graph");

    let clone_opts = CloneDataSheetOptions {
        title: "Draft Clone".into(),
        keep_x_values: true,
        clear_y_values: false,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: None,
    };
    let mut app = UiApp::from_session(Session::from_document(doc));
    let draft_only = ClonePipelineDraft {
        source_sheet_id: Some(SheetId::from("src-xy")),
        new_sheet_id: "cloned-draft".into(),
        options: clone_opts.clone(),
        clone_downstream_analyses: false,
        clone_downstream_graphs: false,
        clone_downstream_layouts: false,
    };
    exec_intent(&mut app, draft_only.to_command_intent().expect("draft clone intent"));
    assert!(app.document().data_sheet(&SheetId::from("cloned-draft")).is_some());

    let cmd_draft = ClonePipelineDraft {
        source_sheet_id: Some(SheetId::from("src-xy")),
        new_sheet_id: "cloned-xy".into(),
        options: CloneDataSheetOptions { title: "Cloned Assay".into(), ..clone_opts },
        ..ClonePipelineDraft::default()
    };
    exec_intent(&mut app, cmd_draft.to_command_intent().expect("clone intent"));
    assert!(app.document().data_sheet(&SheetId::from("cloned-xy")).is_some());
    assert!(app.document().result_sheets.iter().any(|r| r.analysis.source_sheet_id == Some(SheetId::from("cloned-xy"))));
    assert!(
        app.document()
            .graph_sheets
            .iter()
            .any(|g| g.id != SheetId::from("src-graph") && g.spec.series.iter().any(|s| s.source_sheet_id == SheetId::from("cloned-xy")))
    );
}

#[test]
fn multivar_variables_formula_bar_import_wizard_and_quick_stats() {
    let mut doc = ProjectDocument::default();
    let mut mv = MultipleVariablesTable::new(3, 6).expect("mv table");
    for (c, title) in ["Group", "Response", "LogResp"].into_iter().enumerate() {
        mv.variables[c].title = title.into();
    }
    for (r, label) in ["Trt", "Ctrl", "Trt", "Ctrl", "Tret", ""].iter().enumerate() {
        if !label.is_empty() {
            mv.variables[0].cells[r] = Cell::text(*label);
        }
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64 * 10.0);
    }
    mv.variables[0].refresh_inferred_role();
    mv.variables[1].refresh_inferred_role();
    let mut sheet = DataSheet::new("mv-pipe", "Multivar Cohort", TableArchetype::MultipleVariables(mv)).expect("sheet");
    sheet.set_excluded(CellCoord::new(4, 1, 0), true).expect("exclude r=4");
    doc.add_data_sheet(sheet).expect("add sheet");

    let mut grid = GridViewState::default();
    let ds = doc.data_sheet(&SheetId::from("mv-pipe")).expect("ds");
    grid.variables_dialog.open_for_column(ds, 0);
    grid.variables_dialog.merge_levels_into(&["Tret"], "Trt");
    grid.variables_dialog
        .set_level_order(LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["Ctrl".into(), "Trt".into()]), reversed: false }, Some(ds));
    let (edited_count, preview_levels) = grid.variables_dialog.preview_recode(ds).expect("preview");
    assert_eq!((edited_count, preview_levels.len()), (1, 2));
    assert_eq!(grid.variables_dialog.build_recode_command(&ds.id).expect("recode").command_id, "table.recodeLevels");
    assert_eq!(grid.variables_dialog.build_order_command(&ds.id).expect("order").command_id, "table.setCategoricalOrder");

    grid.formula_bar.open_for_column(ds, 2, Some(&doc));
    grid.formula_bar.set_input("= Resp", None, &doc);
    assert!(
        grid.formula_bar.completions.iter().any(|c| c.insert_text == "Response")
            && grid.formula_bar.accept_completion(&doc)
            && grid.formula_bar.is_valid
    );
    assert_eq!(grid.formula_bar.commit_intent(&ds.id).expect("formula").command_id, "table.setCalculatedVariable");

    grid.import_wizard.stage = ImportWizardStage::Source;
    grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    grid.import_wizard.raw_clipboard_text = "12,5*;18,5\n25,0;30,0".into();
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
    assert!(
        preview.len() == 2
            && preview[0][0].excluded
            && preview[0][0].value.finite_f64() == Some(12.5)
            && preview[0][1].value.finite_f64() == Some(25.0)
    );

    grid.import_wizard.is_live_link = true;
    grid.import_wizard.origin_kind = ImportOriginKind::InternalResultSheet;
    grid.import_wizard.source_sheet_id = Some(SheetId::from("r-norm"));
    assert_eq!(grid.import_wizard.preview_border_color(Some(&doc)), Some(LinkBorderColor::Red));
    assert_eq!(grid.import_wizard.build_command_intent(&ds.id, Some(&doc)).command_id, "table.pasteLink");

    let stats = GridQuickStats::from_data_sheet(ds, GridCellCoord::new(0, 1, 0), GridCellCoord::new(5, 1, 0));
    assert_eq!((stats.n, stats.excluded), (5, 1));
    assert!(
        (stats.sum.expect("sum") - 160.0).abs() < 1e-12
            && (stats.mean.expect("mean") - 32.0).abs() < 1e-12
            && (stats.median.expect("median") - 30.0).abs() < 1e-12
    );
    let footer = stats.format_footer();
    assert!(footer.contains("n=5") && footer.contains("excluded=1") && footer.contains("SEM="));

    grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    grid.navigate(ds, MoveDirection::Down, false, true);
    assert_eq!(
        (grid.tool.active, grid.emit_toggle_exclude_intents(ds, true), grid.tool.pending_intents.len()),
        (Some(GridCellCoord::new(4, 0, 0)), 1, 1)
    );
}

#[test]
fn emitted_intents_round_trip_through_ui_app_execute_command() {
    let mut app = UiApp::new();
    let mut grid = GridViewState::default();

    // 1. sheet.createData with auto_graph, auto_info, replicates, and cols.
    create_table(&mut app, TableKind::Xy, "assay-xy", "Dose Curve", (2, 8), Some(3), true);
    let xy_id = SheetId::from("assay-xy");
    let created_xy = app.document().data_sheet(&xy_id).expect("assay-xy");
    assert_eq!((created_xy.archetype.column_count(), total_flat_data_subcolumns(&created_xy.archetype)), (2, 6));
    assert!(created_xy.info_sheet_id.is_some() && app.document().info_sheets.len() == 1 && app.document().graph_sheets.len() == 1);
    create_table(&mut app, TableKind::Column, "col-paste", "Paired Cohort", (2, 4), None, false);
    create_table(&mut app, TableKind::Column, "col-linked", "Linked Cohort", (2, 4), None, false);

    // 2. sheet.select anchor preservation vs re-anchoring via WorkspaceNavState.
    let mut nav = WorkspaceNavState::default();
    nav.click_navigator_entry(app.document(), &xy_id).expect("nav select");
    exec_intent(&mut app, nav.pending_intents.pop().expect("nav intent"));
    assert_eq!(app.document().family_anchor_sheet_id, Some(xy_id.clone()));

    let graph_id = app.document().graph_sheets[0].id.clone();
    nav.click_family_entry(app.document(), &graph_id, false).expect("family single-click");
    exec_intent(&mut app, nav.pending_intents.pop().expect("family intent"));
    assert_eq!((app.document().active_sheet_id.as_ref(), &app.document().family_anchor_sheet_id), (Some(&graph_id), &Some(xy_id.clone())));

    let info_id = app.document().data_sheet(&xy_id).expect("xy").info_sheet_id.clone().expect("info id");
    nav.click_family_entry(app.document(), &info_id, true).expect("family double-click");
    exec_intent(&mut app, nav.pending_intents.pop().expect("double-click intent"));
    assert_eq!(app.document().family_anchor_sheet_id, Some(info_id));
    nav.click_navigator_entry(app.document(), &xy_id).expect("re-anchor");
    exec_intent(&mut app, nav.pending_intents.pop().expect("back intent"));

    // 3-4. table.paste (semicolon + comma decimals + '*' exclusion + transpose) & table.pasteLink (Blue link).
    let (paste_id, linked_id) = (SheetId::from("col-paste"), SheetId::from("col-linked"));
    grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    grid.import_wizard.raw_clipboard_text = "GroupA;10,5*;20,0\nGroupB;30,5;40,5".into();
    grid.import_wizard.delimiter = ';';
    grid.import_wizard.decimal_separator = ',';
    grid.import_wizard.placement.transpose = true;
    grid.import_wizard.placement.propagate_title_header = true;
    let paste_intent = grid.import_wizard.build_command_intent(&paste_id, Some(app.document()));
    assert_eq!(paste_intent.command_id, "table.paste");
    exec_intent(&mut app, paste_intent);
    let pasted = app.document().data_sheet(&paste_id).expect("col-paste");
    let c10 = pasted.cell(CellCoord::new(1, 0, 0)).expect("c10");
    assert!(c10.excluded && c10.value.finite_f64() == Some(10.5));
    assert_eq!(pasted.cell(CellCoord::new(0, 0, 0)).expect("c00").value, CellValue::Text("GroupA".into()));
    assert_eq!(
        (
            pasted.cell(CellCoord::new(2, 0, 0)).expect("c20").value.finite_f64(),
            pasted.cell(CellCoord::new(1, 1, 0)).expect("c11").value.finite_f64()
        ),
        (Some(20.0), Some(30.5))
    );

    grid.import_wizard = Default::default();
    grid.import_wizard.is_live_link = true;
    grid.import_wizard.origin_kind = ImportOriginKind::InternalDataSheet;
    grid.import_wizard.source_sheet_id = Some(paste_id);
    grid.import_wizard.source_range = SheetCellRange::new(CellCoord::new(1, 0, 0), CellCoord::new(2, 1, 0));
    grid.import_wizard.provenance_note = "Linked from Paired Cohort".into();
    let link_intent = grid.import_wizard.build_command_intent(&linked_id, Some(app.document()));
    assert_eq!(link_intent.command_id, "table.pasteLink");
    exec_intent(&mut app, link_intent);
    let linked = app.document().data_sheet(&linked_id).expect("col-linked");
    assert_eq!(
        (linked.linked_regions.len(), linked.linked_regions[0].border_color(), linked.linked_regions[0].provenance_summary.as_str()),
        (1, Some(LinkBorderColor::Blue), "Linked from Paired Cohort")
    );
    assert_eq!(linked.cell(CellCoord::new(1, 1, 0)).expect("linked cell").value.finite_f64(), Some(40.5));

    // 5. table.excludeCells batched partial-subcolumn rectangular ranges on replicate Xy sheet.
    grid.tool.anchor = Some(GridCellCoord::new(0, 0, 1));
    grid.tool.active = Some(GridCellCoord::new(2, 1, 1));
    assert_eq!(grid.emit_toggle_exclude_intents(app.document().data_sheet(&xy_id).expect("xy"), true), 2);
    for intent in grid.tool.pending_intents.drain(..) {
        exec_intent(&mut app, intent);
    }
    let xy_after = app.document().data_sheet(&xy_id).expect("xy after");
    for (coord, exp) in [
        (CellCoord::new(0, 0, 0), false),
        (CellCoord::new(0, 0, 1), true),
        (CellCoord::new(2, 0, 2), true),
        (CellCoord::new(1, 1, 1), true),
        (CellCoord::new(0, 1, 2), false),
    ] {
        assert_eq!(xy_after.cell(coord).expect("cell").excluded, exp);
    }

    // 6. MultipleVariables intents: role, recode, explicit order, calculated formula.
    create_table(&mut app, TableKind::MultipleVariables, "mv-rt", "Multivar Roundtrip", (3, 4), None, false);
    let mv_id = SheetId::from("mv-rt");
    app.execute_command(
        "table.paste",
        serde_json::json!({ "sheet_id": "mv-rt", "text": "Ctrl\t10\t0\nTret\t20\t0\nCtrl\t30\t0\nTrt\t40\t0", "delimiter": "\t" }),
    )
    .expect("seed mv-rt");
    grid.variables_dialog.open_for_column(app.document().data_sheet(&mv_id).expect("mv"), 0);
    grid.variables_dialog.pending_role = Some(VariableRole::Categorical);
    exec_intent(&mut app, grid.variables_dialog.build_role_command(&mv_id).expect("role"));
    grid.variables_dialog.merge_levels_into(&["Tret"], "Trt");
    exec_intent(&mut app, grid.variables_dialog.build_recode_command(&mv_id).expect("recode"));
    grid.variables_dialog.set_level_order(
        LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["Trt".into(), "Ctrl".into()]), reversed: false },
        app.document().data_sheet(&mv_id),
    );
    exec_intent(&mut app, grid.variables_dialog.build_order_command(&mv_id).expect("order"));
    grid.formula_bar.open_for_column(app.document().data_sheet(&mv_id).expect("mv"), 2, Some(app.document()));
    grid.formula_bar.set_input("= B * 2", None, app.document());
    assert!(grid.formula_bar.is_valid);
    exec_intent(&mut app, grid.formula_bar.commit_intent(&mv_id).expect("calc"));
    let TableArchetype::MultipleVariables(mv) = &app.document().data_sheet(&mv_id).expect("mv after").archetype else { panic!("expected MV") };
    assert_eq!(
        (mv.variables[0].role, &mv.variables[0].cells[1].value, mv.variables[2].cells[3].value.finite_f64()),
        (VariableRole::Categorical, &CellValue::Text("Trt".into()), Some(80.0))
    );
    assert_eq!(mv.variables[0].level_order, LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["Trt".into(), "Ctrl".into()]), reversed: false });
    assert!(matches!(mv.variables[2].source, VariableSource::Calculated { .. }));

    // 7. sheet.clonePipeline via ClonePipelineDraft.
    let clone_draft = ClonePipelineDraft {
        source_sheet_id: Some(xy_id),
        new_sheet_id: "assay-xy-clone".into(),
        options: CloneDataSheetOptions {
            title: "Dose Curve Clone".into(),
            keep_x_values: true,
            clear_y_values: true,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: None,
        },
        ..ClonePipelineDraft::default()
    };
    exec_intent(&mut app, clone_draft.to_command_intent().expect("clone intent"));
    assert!(app.document().data_sheet(&SheetId::from("assay-xy-clone")).is_some());
}
