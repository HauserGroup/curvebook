use std::time::{Duration, Instant};

use curvebook_doc::{
    Cell, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, GroupedTable, LevelOrderKind, LevelOrderSpec,
    MultipleVariablesTable, PartsOfWholeTable, SubcolumnMode, SurvivalTable, TableArchetype, VariableColumn, XyTable,
};

#[test]
fn r1_s05_04_clone_structure_rejects_override_replicates_on_presummarized_and_incompatible_tables() {
    let base_opts = CloneDataSheetOptions {
        title: String::from("Cloned"),
        keep_x_values: true,
        clear_y_values: false,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: Some(3),
    };

    // Pre-summarized XY / Grouped must reject `override_replicates` when `clear_y_values == false`.
    let xy_presum = DataSheet::new("xy-sum", "XY Sum", TableArchetype::Xy(XyTable::new(SubcolumnMode::MeanSdN, 1, 2).unwrap())).unwrap();
    assert!(xy_presum.clone_structure("xy-sum-c1", &base_opts).is_err());

    let grp_presum =
        DataSheet::new("grp-sum", "Grp Sum", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::MeanSemN, 2, 2).unwrap())).unwrap();
    assert!(grp_presum.clone_structure("grp-sum-c1", &base_opts).is_err());

    // Clearing Y values explicitly allows switching subcolumn mode to RawReplicates(n).
    let clear_opts = CloneDataSheetOptions { clear_y_values: true, ..base_opts.clone() };
    let xy_cleared = xy_presum.clone_structure("xy-sum-c2", &clear_opts).unwrap();
    let TableArchetype::Xy(xy_out) = xy_cleared.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy_out.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));

    let grp_cleared = grp_presum.clone_structure("grp-sum-c2", &clear_opts).unwrap();
    let TableArchetype::Grouped(grp_out) = grp_cleared.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp_out.subcolumn_mode, SubcolumnMode::RawReplicates(3));

    // Incompatible archetypes must reject `override_replicates` regardless of `clear_y_values`.
    let incompatible = [
        DataSheet::new("col", "Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap())).unwrap(),
        DataSheet::new("con", "Con", TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("srv", "Srv", TableArchetype::Survival(SurvivalTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("pow", "PoW", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("mv", "MV", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 2).unwrap())).unwrap(),
    ];
    for sheet in &incompatible {
        assert!(sheet.clone_structure("bad-1", &base_opts).is_err(), "expected error on {:?} with clear_y_values=false", sheet.kind());
        assert!(sheet.clone_structure("bad-2", &clear_opts).is_err(), "expected error on {:?} with clear_y_values=true", sheet.kind());
    }

    // RawReplicates XY with clear_y_values=false preserves existing replicates up to new width.
    let mut xy_raw = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap();
    xy_raw.data_sets[0].cells[0][0] = Cell::number(11.0);
    xy_raw.data_sets[0].cells[0][1] = Cell::number(22.0);
    let xy_raw_sheet = DataSheet::new("xy-raw", "XY Raw", TableArchetype::Xy(xy_raw)).unwrap();
    let cloned_raw = xy_raw_sheet.clone_structure("xy-raw-c", &base_opts).unwrap();
    let TableArchetype::Xy(raw_out) = cloned_raw.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(raw_out.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert_eq!(raw_out.data_sets[0].cells[0][0].active_number(), Some(11.0));
    assert_eq!(raw_out.data_sets[0].cells[0][1].active_number(), Some(22.0));
    assert!(raw_out.data_sets[0].cells[0][2].is_empty());
}

#[test]
fn r1_s05_06_ordered_levels_with_scales_linearly() {
    let n = 50_000;
    let mut var = VariableColumn::new("LargeCat", n).unwrap();
    let mut explicit = Vec::with_capacity(n);
    for i in 0..n {
        let label = format!("level_{i:05}");
        var.cells[i] = Cell::text(&label);
        explicit.push(label);
    }
    explicit.reverse();

    let t0 = Instant::now();
    let entry_levels = var.ordered_levels_with(&LevelOrderSpec { order: LevelOrderKind::EntryOrder, reversed: false });
    let explicit_levels = var.ordered_levels_with(&LevelOrderSpec { order: LevelOrderKind::Explicit(explicit), reversed: false });
    let elapsed = t0.elapsed();

    assert_eq!(entry_levels.len(), n);
    assert_eq!(entry_levels[0].level, "level_00000");
    assert_eq!(entry_levels[n - 1].level, "level_49999");

    assert_eq!(explicit_levels.len(), n);
    assert_eq!(explicit_levels[0].level, "level_49999");
    assert_eq!(explicit_levels[n - 1].level, "level_00000");
    assert!(elapsed < Duration::from_secs(3), "ordered_levels_with on {n} distinct levels took {elapsed:?} (must scale linearly)");
}
