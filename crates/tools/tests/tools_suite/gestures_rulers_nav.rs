//! Integration tests for trackpad pinch-zoom & pan inertia, physical rulers & journal presets,
//! and Project Navigator drag-reorder state (`L4`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GraphSheet, InfoSheet, LayoutSheet, LayoutSpec,
    ProjectDocument, ResultSheet, ResultStatus, SheetFolderKind, TableArchetype,
};
use curvebook_tools::{JournalWidthPreset, NavigatorDragState, PanZoomInertiaState, PhysicalUnit, RulerTick, build_ruler_ticks};

#[test]
fn pinch_zoom_preserves_anchor_and_clamps_bounds() {
    let mut state = PanZoomInertiaState { zoom: 1.5, pan_pt: [35.0, -18.0], vel_pt_per_sec: [0.0, 0.0], friction_per_sec: 8.0 };
    let anchor = [240.0, 180.0];
    let content_before = state.view_to_content(anchor);

    state.apply_pinch_zoom(1.75, anchor);
    let content_after = state.view_to_content(anchor);
    assert!((state.zoom - 2.625).abs() < 1e-12);
    assert!((content_before[0] - content_after[0]).abs() < 1e-9);
    assert!((content_before[1] - content_after[1]).abs() < 1e-9);

    let roundtrip_view = state.content_to_view(content_after);
    assert!((roundtrip_view[0] - anchor[0]).abs() < 1e-9);
    assert!((roundtrip_view[1] - anchor[1]).abs() < 1e-9);

    // Clamp at max zoom (32.0) and verify further zoom-in does not drift pan_pt.
    state.apply_pinch_zoom(100.0, anchor);
    assert_eq!(state.zoom, 32.0);
    let pan_at_max = state.pan_pt;
    state.apply_pinch_zoom(2.0, anchor);
    assert_eq!(state.zoom, 32.0);
    assert_eq!(state.pan_pt, pan_at_max);

    // Clamp at min zoom (0.1) and verify further zoom-out does not drift pan_pt.
    state.apply_pinch_zoom(0.0001, anchor);
    assert_eq!(state.zoom, 0.1);
    let pan_at_min = state.pan_pt;
    state.apply_pinch_zoom(0.5, anchor);
    assert_eq!(state.zoom, 0.1);
    assert_eq!(state.pan_pt, pan_at_min);

    // Percentage setter clamps cleanly.
    state.set_zoom_percent(250.0);
    assert!((state.zoom - 2.5).abs() < 1e-12);
    state.set_zoom_percent(1.0);
    assert_eq!(state.zoom, 0.1);
    state.set_zoom_percent(5_000.0);
    assert_eq!(state.zoom, 32.0);

    // Zoom-to-fit centers content and resets velocity.
    state.vel_pt_per_sec = [120.0, -45.0];
    state.zoom_to_fit([400.0, 200.0], [800.0, 600.0]);
    assert!((state.zoom - 2.0).abs() < 1e-12);
    assert!((state.pan_pt[0] - 0.0).abs() < 1e-12);
    assert!((state.pan_pt[1] - 100.0).abs() < 1e-12);
    assert_eq!(state.vel_pt_per_sec, [0.0, 0.0]);
}

#[test]
fn inertial_pan_decays_exponentially_and_snaps_to_zero() {
    let mut state = PanZoomInertiaState::default();
    let dt = 1.0 / 60.0;
    for _ in 0..10 {
        state.apply_pan_delta([16.0, -8.0], dt);
    }
    assert!((state.pan_pt[0] - 160.0).abs() < 1e-9);
    assert!((state.pan_pt[1] + 80.0).abs() < 1e-9);
    assert!(state.vel_pt_per_sec[0] > 900.0);
    assert!(state.vel_pt_per_sec[1] < -450.0);

    let v0 = state.vel_pt_per_sec;
    let pan_release = state.pan_pt;
    let mut prev_speed = v0[0].hypot(v0[1]);
    let mut steps = 0usize;

    while state.step_inertia(dt) {
        steps += 1;
        let speed = state.vel_pt_per_sec[0].hypot(state.vel_pt_per_sec[1]);
        assert!(speed < prev_speed, "speed must decay strictly monotonically");
        assert!(speed >= 0.5, "step_inertia returned true below threshold: {speed}");
        prev_speed = speed;
        assert!(steps < 500, "inertial decay failed to converge within 500 frames");
    }

    assert!(steps > 10, "expected multi-frame inertial coasting, stopped in {steps} frames");
    assert_eq!(state.vel_pt_per_sec, [0.0, 0.0]);

    // Total displacement should match analytical v0 / friction within sub-threshold tolerance.
    let expected_dx = v0[0] / state.friction_per_sec;
    let expected_dy = v0[1] / state.friction_per_sec;
    assert!(((state.pan_pt[0] - pan_release[0]) - expected_dx).abs() < 0.1);
    assert!(((state.pan_pt[1] - pan_release[1]) - expected_dy).abs() < 0.1);

    // Subsequent step keeps pan stationary and returns false.
    let settled_pan = state.pan_pt;
    assert!(!state.step_inertia(dt));
    assert_eq!(state.pan_pt, settled_pan);
}

#[test]
fn physical_units_and_journal_presets_exact_points() {
    assert_eq!(PhysicalUnit::Points.to_points(42.5), 42.5);
    assert_eq!(PhysicalUnit::Inches.to_points(1.0), 72.0);
    assert!((PhysicalUnit::Millimeters.to_points(25.4) - 72.0).abs() < 1e-12);
    assert!((PhysicalUnit::Millimeters.from_points(72.0) - 25.4).abs() < 1e-12);
    assert!((PhysicalUnit::Inches.from_points(180.0) - 2.5).abs() < 1e-12);

    let pt_per_mm = 72.0 / 25.4;
    let (w85, h85) = JournalWidthPreset::SingleColumn85Mm.size_pt();
    assert!((w85 - 85.0 * pt_per_mm).abs() < 1e-12);
    assert!((h85 - 85.0 * pt_per_mm).abs() < 1e-12);

    let (w114, h114) = JournalWidthPreset::OneHalfColumn114Mm.size_pt();
    assert!((w114 - 114.0 * pt_per_mm).abs() < 1e-12);
    assert!(h114 > 0.0);

    let (w174, h174) = JournalWidthPreset::DoubleColumn174Mm.size_pt();
    assert!((w174 - 174.0 * pt_per_mm).abs() < 1e-12);
    assert!((h174 - 120.0 * pt_per_mm).abs() < 1e-12);

    assert_eq!(JournalWidthPreset::LetterHalf.size_pt(), (396.0, 612.0));
    assert_eq!(JournalWidthPreset::CustomPt(310.0, 220.0).size_pt(), (310.0, 220.0));
    assert_eq!(JournalWidthPreset::CustomPt(-10.0, f64::NAN).size_pt(), (0.0, 0.0));
}

fn assert_ruler_invariants(ticks: &[RulerTick], doc_span_pt: f64, zoom: f64, pan_pt: f64, px_per_pt: f64) {
    assert!(!ticks.is_empty());
    let mut prev_pos = -1.0;
    let mut prev_major_px: Option<f64> = None;

    for tick in ticks {
        assert!(tick.pos_pt.is_finite() && tick.view_px.is_finite());
        assert!(tick.pos_pt >= 0.0 && tick.pos_pt <= doc_span_pt + 1e-9);
        assert!(tick.pos_pt > prev_pos, "tick positions must be strictly increasing");
        prev_pos = tick.pos_pt;

        let expected_px = (tick.pos_pt * zoom + pan_pt) * px_per_pt;
        assert!((tick.view_px - expected_px).abs() < 1e-6);

        if tick.major {
            assert!(tick.label.is_some());
            if let Some(label) = &tick.label {
                assert!(!label.is_empty() && !label.starts_with("-0"));
            }
            if let Some(prev_px) = prev_major_px {
                assert!(tick.view_px - prev_px >= 48.0 - 1e-6, "major tick labels overlapped: {} px apart", tick.view_px - prev_px);
            }
            prev_major_px = Some(tick.view_px);
        } else {
            assert!(tick.label.is_none(), "minor tick must not carry label");
        }
    }
}

#[test]
fn ruler_ticks_adapt_with_zoom_and_never_overlap() {
    let (span_174_pt, _) = JournalWidthPreset::DoubleColumn174Mm.size_pt();
    let (span_letter_pt, _) = JournalWidthPreset::LetterHalf.size_pt();

    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0, 16.0, 32.0] {
        let mm_ticks = build_ruler_ticks(PhysicalUnit::Millimeters, span_174_pt, zoom, 12.0, 1.0);
        assert_ruler_invariants(&mm_ticks, span_174_pt, zoom, 12.0, 1.0);

        let in_ticks = build_ruler_ticks(PhysicalUnit::Inches, span_letter_pt, zoom, -24.0, 1.25);
        assert_ruler_invariants(&in_ticks, span_letter_pt, zoom, -24.0, 1.25);

        let pt_ticks = build_ruler_ticks(PhysicalUnit::Points, 500.0, zoom, 0.0, 1.0);
        assert_ruler_invariants(&pt_ticks, 500.0, zoom, 0.0, 1.0);
    }

    // Check unit-specific major labels at 1.0x zoom, 1.0 px/pt.
    let mm_1x = build_ruler_ticks(PhysicalUnit::Millimeters, span_174_pt, 1.0, 0.0, 1.0);
    let mm_labels: Vec<&str> = mm_1x.iter().filter_map(|t| t.label.as_deref()).collect();
    assert_eq!(&mm_labels[..4], &["0", "20", "40", "60"]);

    let in_1x = build_ruler_ticks(PhysicalUnit::Inches, span_letter_pt, 1.0, 0.0, 1.0);
    let in_labels: Vec<&str> = in_1x.iter().filter_map(|t| t.label.as_deref()).collect();
    assert_eq!(in_labels, vec!["0", "1", "2", "3", "4", "5"]);

    let in_2x = build_ruler_ticks(PhysicalUnit::Inches, span_letter_pt, 2.0, 0.0, 1.0);
    let in_2x_labels: Vec<&str> = in_2x.iter().filter_map(|t| t.label.as_deref()).collect();
    assert_eq!(&in_2x_labels[..5], &["0", "0.5", "1", "1.5", "2"]);

    let pt_1x = build_ruler_ticks(PhysicalUnit::Points, 250.0, 1.0, 0.0, 1.0);
    let pt_labels: Vec<&str> = pt_1x.iter().filter_map(|t| t.label.as_deref()).collect();
    assert_eq!(pt_labels, vec!["0", "50", "100", "150", "200", "250"]);
}

#[test]
fn nan_inf_zero_and_huge_input_guards() {
    let mut state = PanZoomInertiaState { zoom: 2.0, pan_pt: [15.0, -25.0], vel_pt_per_sec: [100.0, -50.0], friction_per_sec: 8.0 };
    let snapshot = state;

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.5] {
        state.apply_pinch_zoom(bad, [100.0, 100.0]);
        assert_eq!(state, snapshot);
        state.set_zoom_percent(bad);
        assert_eq!(state, snapshot);
        state.zoom_to_fit([bad, 100.0], [400.0, 400.0]);
        assert_eq!(state, snapshot);
        state.zoom_to_fit([100.0, 100.0], [400.0, bad]);
        assert_eq!(state, snapshot);
    }

    state.apply_pinch_zoom(1.5, [f64::NAN, 100.0]);
    state.apply_pinch_zoom(1.5, [100.0, f64::INFINITY]);
    assert_eq!(state, snapshot);

    state.apply_pan_delta([f64::NAN, 5.0], 0.016);
    state.apply_pan_delta([5.0, f64::INFINITY], 0.016);
    state.apply_pan_delta([5.0, 5.0], -0.01);
    state.apply_pan_delta([5.0, 5.0], f64::NAN);
    state.apply_pan_delta([5.0, 5.0], f64::INFINITY);
    assert_eq!(state, snapshot);

    // Zero dt_sec pans without dividing by zero or injecting non-finite velocity.
    state.apply_pan_delta([5.0, -5.0], 0.0);
    assert_eq!(state.pan_pt, [20.0, -30.0]);
    assert_eq!(state.vel_pt_per_sec, [100.0, -50.0]);

    assert!(!state.step_inertia(0.0));
    assert!(!state.step_inertia(-0.1));
    assert!(!state.step_inertia(f64::NAN));
    assert!(!state.step_inertia(f64::INFINITY));

    // Corrupted internal fields recover cleanly to finite state.
    let mut broken =
        PanZoomInertiaState { zoom: f64::NAN, pan_pt: [f64::INFINITY, f64::NEG_INFINITY], vel_pt_per_sec: [f64::NAN, 40.0], friction_per_sec: -3.0 };
    broken.apply_pinch_zoom(2.0, [50.0, 50.0]);
    assert!(broken.zoom.is_finite() && broken.pan_pt[0].is_finite() && broken.pan_pt[1].is_finite());
    assert!(broken.step_inertia(0.016));
    assert!(broken.vel_pt_per_sec[0].is_finite() && broken.vel_pt_per_sec[1].is_finite());

    // Zero / negative / non-finite ruler inputs return empty vec cleanly.
    assert!(build_ruler_ticks(PhysicalUnit::Millimeters, 0.0, 1.0, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Millimeters, -10.0, 1.0, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Millimeters, f64::NAN, 1.0, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Millimeters, f64::INFINITY, 1.0, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Inches, 500.0, 0.0, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Inches, 500.0, f64::NAN, 0.0, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Points, 500.0, 1.0, f64::NEG_INFINITY, 1.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Points, 500.0, 1.0, 0.0, 0.0).is_empty());
    assert!(build_ruler_ticks(PhysicalUnit::Points, 500.0, 1.0, 0.0, f64::INFINITY).is_empty());

    // Huge inputs are bounded cleanly and never overflow or OOM.
    let huge_span_ticks = build_ruler_ticks(PhysicalUnit::Points, f64::MAX, 32.0, 0.0, 2.0);
    assert!(!huge_span_ticks.is_empty() && huge_span_ticks.len() <= 2_048);
    assert!(huge_span_ticks.iter().all(|t| t.pos_pt.is_finite() && t.view_px.is_finite()));

    let huge_zoom_ticks = build_ruler_ticks(PhysicalUnit::Millimeters, 500.0, 1e12, 0.0, 1.0);
    assert!(!huge_zoom_ticks.is_empty() && huge_zoom_ticks.len() <= 2_048);
    assert!(huge_zoom_ticks.iter().all(|t| t.pos_pt.is_finite() && t.view_px.is_finite()));
}

#[test]
fn navigator_drag_reorder_lifecycle_and_doc_integration() {
    let mut drag = NavigatorDragState::default();
    assert!(drag.active_drag.is_none());
    assert!(drag.hover_target_index.is_none());

    // Updating hover without an active drag is ignored.
    drag.update_hover(2);
    assert!(drag.hover_target_index.is_none());
    assert_eq!(drag.commit_drop(), None);

    // Committing without a hover target clears active drag and returns None.
    drag.begin_drag(SheetFolderKind::Data, "d-1", 0);
    assert_eq!(drag.active_drag, Some((SheetFolderKind::Data, String::from("d-1"), 0)));
    assert_eq!(drag.commit_drop(), None);
    assert!(drag.active_drag.is_none());

    // Dropping back onto the same index returns None and clears state.
    drag.begin_drag(SheetFolderKind::Info, "i-1", 1);
    drag.update_hover(1);
    assert_eq!(drag.commit_drop(), None);
    assert!(drag.active_drag.is_none());
    assert!(drag.hover_target_index.is_none());

    // Canceling clears both active drag and hover target.
    drag.begin_drag(SheetFolderKind::Results, "r-1", 0);
    drag.update_hover(2);
    assert_eq!(drag.hover_target_index, Some(2));
    drag.cancel();
    assert!(drag.active_drag.is_none());
    assert!(drag.hover_target_index.is_none());
    assert_eq!(drag.commit_drop(), None);

    // Valid drag + reorder applied directly to ProjectDocument across folders.
    let mut doc = ProjectDocument::new("Drag Reorder").expect("project");
    for idx in 0..3 {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("col");
        let ds = DataSheet::new(format!("d-{idx}"), format!("Data {idx}"), TableArchetype::Column(col)).expect("data sheet");
        doc.add_data_sheet(ds).expect("add data");

        let info = InfoSheet::new_preseeded(format!("i-{idx}"), format!("Info {idx}")).expect("info");
        doc.add_info_sheet(info).expect("add info");

        let mut gs = GraphSheet::new_uninitialized_for_data(format!("g-{idx}"), &doc.data_sheets[idx]).expect("graph");
        gs.title = format!("Graph {idx}");
        doc.add_graph_sheet(gs).expect("add graph");

        let ls = LayoutSheet::new(format!("l-{idx}"), format!("Layout {idx}"), LayoutSpec::default()).expect("layout");
        doc.add_layout_sheet(ls).expect("add layout");

        let analysis = AnalysisSpec::new(format!("r-{idx}"), "d-0", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis");
        let mut rs = ResultSheet::new(analysis).expect("result");
        rs.status = ResultStatus::UpToDate;
        doc.add_result_sheet(rs).expect("add result");
    }

    drag.begin_drag(SheetFolderKind::Data, "d-0", 0);
    drag.update_hover(2);
    let (folder, from_idx, to_idx) = drag.commit_drop().expect("drop result");
    assert_eq!((folder, from_idx, to_idx), (SheetFolderKind::Data, 0, 2));
    doc.reorder_sheet(folder, from_idx, to_idx).expect("reorder data");
    let data_ids: Vec<&str> = doc.data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(data_ids, vec!["d-1", "d-2", "d-0"]);

    drag.begin_drag(SheetFolderKind::Info, "i-0", 0);
    drag.update_hover(1);
    let (folder, from_idx, to_idx) = drag.commit_drop().expect("drop info");
    doc.reorder_sheet(folder, from_idx, to_idx).expect("reorder info");
    let info_ids: Vec<&str> = doc.info_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(info_ids, vec!["i-1", "i-0", "i-2"]);

    drag.begin_drag(SheetFolderKind::Results, "r-2", 2);
    drag.update_hover(1);
    let (folder, from_idx, to_idx) = drag.commit_drop().expect("drop results");
    doc.reorder_sheet(folder, from_idx, to_idx).expect("reorder results");
    let result_ids: Vec<&str> = doc.result_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(result_ids, vec!["r-0", "r-2", "r-1"]);

    drag.begin_drag(SheetFolderKind::Graphs, "g-2", 2);
    drag.update_hover(0);
    let (folder, from_idx, to_idx) = drag.commit_drop().expect("drop graph");
    doc.reorder_sheet(folder, from_idx, to_idx).expect("reorder graph");
    let graph_ids: Vec<&str> = doc.graph_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(graph_ids, vec!["g-2", "g-0", "g-1"]);

    drag.begin_drag(SheetFolderKind::Layouts, "l-0", 0);
    drag.update_hover(1);
    let (folder, from_idx, to_idx) = drag.commit_drop().expect("drop layout");
    doc.reorder_sheet(folder, from_idx, to_idx).expect("reorder layout");
    let layout_ids: Vec<&str> = doc.layout_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(layout_ids, vec!["l-1", "l-0", "l-2"]);
}
