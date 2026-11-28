//! R14.8 release archive and batch command compatibility.

use std::{error::Error, fs, time::Duration};

use curvebook_doc::{AnalysisKind, CellCoord, LayoutItem, ResultStatus, SheetId};
use curvebook_engine::{ExternalPathPolicy, Session};
use serde_json::json;

use super::common::{CliHarness, TempDir, assert_close};

const PROJECT: &[u8] = include_bytes!("../fixtures/release_compatibility/release-compatibility.curvebook");
const BATCH: &[u8] = include_bytes!("../fixtures/release_compatibility/batch.json");

fn open_project(dir: &TempDir, name: &str) -> Result<Session, Box<dyn Error>> {
    let mut session = Session::new();
    session.set_path_policy(ExternalPathPolicy::workdir(dir.path())?);
    session.execute("project.open", json!({ "path": name }))?;
    Ok(session)
}

fn check_project(session: &Session, expected_slope: f64) -> Result<(), Box<dyn Error>> {
    let doc = session.document();
    let data = doc.data_sheet(&SheetId::new("measurements")).ok_or("missing measurements")?;
    let fit = doc.result_sheet(&SheetId::new("calibration-fit")).ok_or("missing fit")?;
    let graph = doc.graph_sheet(&SheetId::new("calibration-graph")).ok_or("missing graph")?;
    let layout = doc.layout_sheet(&SheetId::new("figure")).ok_or("missing layout")?;

    let coord = CellCoord { row: 3, col: 0, subcol: 0 };
    let cell = data.cell(coord).ok_or("missing excluded measurement")?;
    assert!(cell.excluded);
    assert_eq!(cell.value.finite_f64(), Some(100.0));
    let note = data.exclusion_note(coord).ok_or("missing exclusion audit note")?;
    assert_eq!(note.reason_tag, "instrument_fault");
    assert_eq!(note.note, "Instrument saturated during this measurement.");

    assert!(matches!(&fit.analysis.kind, AnalysisKind::LinearRegression(_)));
    assert_eq!(fit.analysis.source_sheet_id.as_ref(), Some(&data.id));
    assert_eq!(fit.status, ResultStatus::UpToDate);
    assert_close(fit.lookup_scalar("Slope", None).ok_or("missing fitted slope")?, expected_slope, 1e-9, "calibration slope")?;

    assert_eq!(graph.spec.primary_source_sheet_id, data.id);
    let series = graph.spec.series.first().ok_or("missing graph series")?;
    let overlay = series.regression_overlay.as_ref().ok_or("missing fit overlay")?;
    assert_eq!(overlay.result_sheet_id, fit.id);
    assert!(overlay.show_confidence_band);
    assert!(matches!(
        layout.spec.items.as_slice(),
        [LayoutItem::LiveGraph { graph_sheet_id, .. }, LayoutItem::LiveTable { source_sheet_id, .. }]
            if graph_sheet_id == &graph.id && source_sheet_id == &fit.id
    ));
    Ok(())
}

#[test]
fn release_project_opens_recalculates_and_round_trips_and_batch_replays() -> Result<(), Box<dyn Error>> {
    // Excluding x=3, the least-squares denominator sum((x - mean(x))^2) is 86/5.
    // The centered cross-product is 173/5, then 212/5 after changing y at x=5 to 14.
    let dir = TempDir::new("release_compatibility")?;
    let original = dir.child("original.curvebook");
    fs::write(&original, PROJECT)?;
    let mut opened = open_project(&dir, "original.curvebook")?;
    check_project(&opened, 173.0 / 86.0)?;

    let batch = dir.child("batch.json");
    fs::write(&batch, BATCH)?;
    let args = ["batch", CliHarness::path_str(&batch)?, "--workdir", CliHarness::path_str(dir.path())?].map(str::to_owned);
    let output = CliHarness::run_bounded(&args, Duration::from_secs(30))?;
    assert!(output.status.success(), "batch replay failed: {}", String::from_utf8_lossy(&output.stderr));
    check_project(&open_project(&dir, "release-compatibility.curvebook")?, 173.0 / 86.0)?;

    opened.execute("table.setCell", json!({ "sheet_id": "measurements", "row": 5, "col": 0, "value": 14 }))?;
    check_project(&opened, 212.0 / 86.0)?;
    opened.execute("project.save", json!({ "path": "edited.curvebook" }))?;
    let reopened = open_project(&dir, "edited.curvebook")?;
    assert_eq!(reopened.document(), opened.document());
    assert_eq!(fs::read(original)?, PROJECT, "saving the edited copy must leave the original archive intact");
    Ok(())
}
