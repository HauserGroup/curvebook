#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R10.4` Welcome & New Data Table Launcher human-path workflow tests (`w5a`).
//!
//! Covers:
//! - Two-pane Welcome & New Data Table Launcher rendering when `welcome.open == true` or project is empty
//! - All 8 scientific `TableKind` archetypes, `TableCreationDraft` X-column modes, Y replicate &
//!   pre-computed error formats (`SubcolumnMode`), `ColumnDesignHint` Unpaired vs. Paired, and
//!   `auto_graph` / `auto_info` toggles
//! - `TutorialSample` clean-room `SAMPLE_FIXTURES` instantiation (including `MultipleVariables`
//!   formulas & `DoseResponse` 4PL XY replicates) via canonical engine commands
//! - `ClonePipeline` (`sheet.clonePipeline`) and `.cbtemplate` (`project.instantiateTemplate`) flows
//! - `Open Existing File...` trigger, `Cancel`, and `"welcome"` custom action `"smoke"` no-op parity
//! - Strict `gui_only` + `human_path_only` zero-shortcut compliance, pixel-to-hit-box & click parity,
//!   16-locale / RTL (`ar`, `he`) / Light-Dark-HighContrast / 2.0x HiDPI rendering, and `w5_welcome.rs`
//!   localization catalog verification.

mod human_wf_common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TableKind,
    XColumnMode,
};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::compositor::render_workspace_rgba;
use curvebook_ui_egui::i18n::LOCALES;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetAction;
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::layout::verify::verify_click_action_parity;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::views::ViewShardStates;
use curvebook_ui_egui::views::dispatch_custom_view_action;
use curvebook_ui_egui::workspace_nav::WelcomeMode;
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_widget_center, default_viewport, new_human_driver, press_cmd_shortcut,
    type_into_focused,
};

fn empty_welcome_app() -> UiApp {
    let mut app = UiApp::new();
    app.nav.welcome.open = true;
    app.nav.gallery.open = true;
    app
}

fn replace_text_input(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, widget_id: &str, text: &str) {
    click_widget_center(driver, &WidgetId::from(widget_id)).expect(widget_id);
    press_cmd_shortcut(driver, "a").expect("Cmd+A select all");
    type_into_focused(driver, text).expect("type replacement text");
}

fn populated_anova_app() -> UiApp {
    let mut doc = ProjectDocument::new("Reference Study").expect("new doc");
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("column table");
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Low Dose");
    col.data_sets[2].title = String::from("High Dose");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(15.0 + r as f64);
        col.data_sets[2].cells[r][0] = Cell::number(22.0 + r as f64);
    }
    let sheet = DataSheet::new("ref-col-1", "Dose Screen Assay", TableArchetype::Column(col)).expect("data sheet");
    let mut graph = GraphSheet::new_uninitialized_for_data("ref-graph-1", &sheet).expect("graph");
    graph.spec.kind = GraphKind::Column;
    graph.initialized = true;
    doc.add_data_sheet(sheet).expect("add data sheet");
    doc.add_graph_sheet(graph).expect("add graph sheet");
    doc.select_in_navigator(&SheetId::from("ref-col-1")).expect("select sheet");

    let mut driver = new_human_driver(UiApp::from_session(Session::from_document(doc)), default_viewport());
    driver.click_widget("toolbar.analyze").expect("open Analyze dialog");
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = driver.wheel_widget("analyze.entries_list", 0.0, -180.0, KeyModifiers::default());
    }
    driver.click_widget("analyze.entry.stats.one_way_anova").expect("select One-Way ANOVA");
    driver.click_widget("modal.ok").expect("confirm One-Way ANOVA");
    driver.app().clone()
}

#[test]
fn r10_4_welcome_zero_shortcut_compliance_and_click_parity_all_modes() {
    let metrics = default_viewport();
    let mut driver = new_human_driver(empty_welcome_app(), metrics);

    // 1. Strict gui_only / human_path_only + parity report on initial Welcome screen.
    let report = assert_zero_shortcut_compliance(&mut driver).expect("welcome parity & compliance");
    assert!(report.checked_nodes >= 20, "expected >= 20 interactive controls, got {}", report.checked_nodes);

    // 2. Click-to-action parity across representative Welcome rail & setup widgets.
    let probe_app = driver.app().clone();
    for wid in [
        "welcome.arch.column",
        "welcome.arch.grouped",
        "welcome.tab.sample",
        "welcome.x_mode.elapsed",
        "welcome.subcol.mean_sd_n",
        "welcome.toggle.auto_graph",
        "welcome.create_cta",
    ] {
        assert!(verify_click_action_parity(&probe_app, metrics, wid).expect(wid), "click parity mismatch for `{wid}`");
    }

    // 3. Hovering a disabled control (Column design on XY, Clone on empty doc) surfaces disabled tooltip.
    driver.hover_widget("welcome.mode.clone").expect("hover disabled clone button");
    let hover_tree = driver.layout_tree();
    let clone_node = hover_tree.node("welcome.mode.clone").expect("clone node");
    assert!(!clone_node.state.enablement.enabled);
    let reason = clone_node.state.enablement.disabled_reason.as_deref().expect("clone disabled reason");
    assert!(
        hover_tree.text_runs.iter().any(|r| r.text.contains("Data Sheet") || reason.starts_with(&r.text[..r.text.len().min(12)])),
        "expected tooltip for disabled clone button `{reason}`"
    );

    // 4. Scaffold smoke custom action compatibility (`welcome` unknown payload -> handled: false).
    let mut smoke_app = UiApp::new();
    let mut shards = ViewShardStates::default();
    let smoke_out = dispatch_custom_view_action(&mut smoke_app, &mut shards, "welcome", "smoke").expect("smoke dispatch");
    assert!(!smoke_out.handled, "unknown payload `smoke` must remain unhandled no-op");
    let _ = WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("smoke") };

    assert_input_to_paint_budget(&driver, 50.0).expect("input-to-paint latency budget");
}

#[test]
fn r10_4_welcome_eight_archetypes_creation_and_draft_controls_human_path() {
    let metrics = default_viewport();
    let mut driver = new_human_driver(empty_welcome_app(), metrics);

    // Visit every one of the 8 table archetypes via raw pointer clicks.
    let expected = [
        ("welcome.arch.xy", TableKind::Xy),
        ("welcome.arch.column", TableKind::Column),
        ("welcome.arch.grouped", TableKind::Grouped),
        ("welcome.arch.contingency", TableKind::Contingency),
        ("welcome.arch.survival", TableKind::Survival),
        ("welcome.arch.parts_of_whole", TableKind::PartsOfWhole),
        ("welcome.arch.multiple_variables", TableKind::MultipleVariables),
        ("welcome.arch.nested", TableKind::Nested),
    ];
    for (wid, kind) in expected {
        click_widget_center(&mut driver, &WidgetId::from(wid)).expect(wid);
        assert_eq!(driver.app().nav.welcome.table_draft.kind, kind, "clicking `{wid}` should select {kind:?}");
        assert!(driver.verify_parity().is_clean(), "parity failed after switching to {kind:?}");
    }

    // Configure XY table with Dates X-column + Mean/SD/N + custom title, then Create.
    click_widget_center(&mut driver, &WidgetId::from("welcome.arch.xy")).expect("select XY");
    click_widget_center(&mut driver, &WidgetId::from("welcome.x_mode.dates")).expect("select Dates X mode");
    assert!(matches!(
        driver.app().nav.welcome.table_draft.x_mode,
        XColumnMode::DateOrElapsedTime { ref unit_label } if unit_label == "Date"
    ));
    click_widget_center(&mut driver, &WidgetId::from("welcome.subcol.mean_sd_n")).expect("select Mean/SD/N");
    assert_eq!(driver.app().nav.welcome.table_draft.subcolumn_mode, SubcolumnMode::MeanSdN);

    replace_text_input(&mut driver, "welcome.title.input", "Ligand Equilibrium Run");
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("create XY table");

    assert!(!driver.app().nav.welcome.open);
    assert!(!driver.app().nav.gallery.open);
    assert_eq!(driver.app().document().data_sheets.len(), 1);
    assert_eq!(driver.app().document().graph_sheets.len(), 1);
    assert_eq!(driver.app().document().info_sheets.len(), 1);
    let xy_ds = &driver.app().document().data_sheets[0];
    assert_eq!(xy_ds.title, "Ligand Equilibrium Run");
    match &xy_ds.archetype {
        TableArchetype::Xy(xy) => {
            assert_eq!(xy.x_title, "Date");
            assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::MeanSdN);
        }
        other => panic!("expected Xy table, got {other:?}"),
    }

    // Re-open Welcome on existing workspace -> configure Paired Column table without Info sheet.
    driver.app_mut().nav.welcome.open = true;
    driver.app_mut().nav.gallery.open = true;
    click_widget_center(&mut driver, &WidgetId::from("welcome.arch.column")).expect("select Column");
    click_widget_center(&mut driver, &WidgetId::from("welcome.design.paired")).expect("select Paired design");
    assert_eq!(driver.app().nav.welcome.table_draft.column_design, ColumnDesignHint::PairedRepeatedMeasures);
    click_widget_center(&mut driver, &WidgetId::from("welcome.toggle.auto_info")).expect("uncheck auto_info");
    assert!(!driver.app().nav.welcome.table_draft.create_paired_info);
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("create Column table");

    assert_eq!(driver.app().document().data_sheets.len(), 2);
    assert_eq!(driver.app().document().graph_sheets.len(), 2);
    assert_eq!(driver.app().document().info_sheets.len(), 1, "unchecking auto_info must skip extra InfoSheet");
    let col_ds = &driver.app().document().data_sheets[1];
    assert!(matches!(&col_ds.archetype, TableArchetype::Column(c) if c.design_hint == ColumnDesignHint::PairedRepeatedMeasures));

    // Re-open Welcome -> configure Nested replicates stepper (`3 -> 4`) and create.
    driver.app_mut().nav.welcome.open = true;
    driver.app_mut().nav.gallery.open = true;
    click_widget_center(&mut driver, &WidgetId::from("welcome.arch.nested")).expect("select Nested");
    click_widget_center(&mut driver, &WidgetId::from("welcome.replicates.inc")).expect("increment Nested replicates");
    assert_eq!(driver.app().nav.welcome.table_draft.replicates, 4);
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("create Nested table");
    let nested_ds = &driver.app().document().data_sheets[2];
    assert!(matches!(&nested_ds.archetype, TableArchetype::Nested(n) if n.groups[0].subcolumn_titles.len() == 4));
    assert_input_to_paint_budget(&driver, 50.0).expect("budget");
}

#[test]
fn r10_4_welcome_tutorial_samples_human_path() {
    let metrics = default_viewport();
    let mut driver = new_human_driver(empty_welcome_app(), metrics);

    // Switch to Tutorial Sample Data tab via raw pointer click.
    click_widget_center(&mut driver, &WidgetId::from("welcome.tab.sample")).expect("switch to sample tab");
    assert_eq!(driver.app().nav.welcome.mode, WelcomeMode::TutorialSample);
    assert!(driver.verify_parity().is_clean(), "sample mode parity clean");

    // Pick Dose-Response 4PL sample and instantiate via canonical engine commands.
    click_widget_center(&mut driver, &WidgetId::from("welcome.fixture.dose-response-4pl")).expect("pick dose-response-4pl");
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("open sample");

    assert_eq!(driver.app().document().data_sheets.len(), 1);
    assert_eq!(driver.app().document().graph_sheets.len(), 1);
    assert_eq!(driver.app().document().info_sheets.len(), 1);
    let dr = &driver.app().document().data_sheets[0];
    assert_eq!(dr.title, "Ligand Dose-Response Inhibition");
    match &dr.archetype {
        TableArchetype::Xy(xy) => {
            assert_eq!(xy.x_title, "Log [Inhibitor] (M)");
            assert_eq!(xy.x_value_at(0), Some(-9.0));
            assert_eq!(xy.data_sets[0].title, "Control");
            assert_eq!(xy.data_sets[1].title, "Mutant");
            assert_eq!(xy.data_sets[0].cells[0][0].active_number(), Some(99.0));
        }
        other => panic!("expected XY fixture, got {other:?}"),
    }

    // Open MultipleVariables clean-room fixture (`clinical-biomarker-cohort`) with calculated BMI column.
    driver.app_mut().nav.welcome.open = true;
    driver.app_mut().nav.gallery.open = true;
    click_widget_center(&mut driver, &WidgetId::from("welcome.tab.sample")).expect("sample tab again");
    click_widget_center(&mut driver, &WidgetId::from("welcome.fixture.clinical-biomarker-cohort")).expect("pick mv sample");
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("create mv sample");

    let mv_ds = &driver.app().document().data_sheets[1];
    assert_eq!(mv_ds.title, "Multivariate Biomarker Cohort");
    match &mv_ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            assert_eq!(mv.variables.len(), 4);
            assert_eq!(mv.variables[3].title, "BMI");
            let bmi0 = mv.variables[3].cells[0].active_number().expect("calculated BMI value");
            let expected = 68.0 / (1.72 * 1.72);
            assert!((bmi0 - expected).abs() < 1e-6, "BMI mismatch: {bmi0} vs {expected}");
        }
        other => panic!("expected MultipleVariables fixture, got {other:?}"),
    }
}

#[test]
fn r10_4_welcome_clone_pipeline_and_cbtemplate_modes_human_path() {
    let metrics = default_viewport();
    let mut app = populated_anova_app();
    app.nav.welcome.open = true;
    app.nav.gallery.open = true;
    let mut driver = new_human_driver(app, metrics);

    // 1. Open Existing File trigger increments request counter cleanly without erroring, and Cancel closes.
    click_widget_center(&mut driver, &WidgetId::from("welcome.action.open_existing")).expect("open existing trigger");
    assert_eq!(driver.app().view_shards.welcome.last_open_file_request, 1);

    // 2. Switch to Clone Pipeline mode on populated project -> verify parity, customize title, clone!
    click_widget_center(&mut driver, &WidgetId::from("welcome.mode.clone")).expect("select Clone mode");
    assert_eq!(driver.app().nav.welcome.mode, WelcomeMode::ClonePipeline);
    assert!(driver.verify_parity().is_clean(), "clone mode parity clean");

    replace_text_input(&mut driver, "welcome.clone.title.input", "Cohort B Screen");
    click_widget_center(&mut driver, &WidgetId::from("welcome.clone.clear_y")).expect("toggle clear_y");
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("execute Clone Pipeline");

    assert!(!driver.app().nav.welcome.open);
    assert_eq!(driver.app().document().data_sheets.len(), 2);
    assert_eq!(driver.app().document().result_sheets.len(), 2, "downstream ANOVA result sheet should be cloned");
    assert_eq!(driver.app().document().graph_sheets.len(), 2, "downstream graph sheet should be cloned");
    assert_eq!(driver.app().document().data_sheets[1].title, "Cohort B Screen");

    // 3. Open Templates mode -> pick Grouped Kinetics .cbtemplate -> instantiate real template archive!
    driver.app_mut().nav.welcome.open = true;
    driver.app_mut().nav.gallery.open = true;
    click_widget_center(&mut driver, &WidgetId::from("welcome.mode.template")).expect("select Template mode");
    assert_eq!(driver.app().nav.welcome.mode, WelcomeMode::FromTemplate);
    assert!(driver.verify_parity().is_clean(), "template mode parity clean");

    click_widget_center(&mut driver, &WidgetId::from("welcome.template.2")).expect("select grouped template");
    replace_text_input(&mut driver, "welcome.title.input", "Kinetics Assay Q4");
    click_widget_center(&mut driver, &WidgetId::from("welcome.create_cta")).expect("instantiate .cbtemplate");

    assert_eq!(driver.app().document().title, "Kinetics Assay Q4");
    assert_eq!(driver.app().document().data_sheets.len(), 1);
    assert_eq!(driver.app().document().graph_sheets.len(), 1);
    assert_eq!(driver.app().document().info_sheets.len(), 1);
    assert_eq!(driver.app().document().data_sheets[0].kind(), TableKind::Grouped);

    // 4. Re-open Welcome and test Cancel closes without mutating sheets.
    let sheet_count_before = driver.app().document().total_sheet_count();
    driver.app_mut().nav.welcome.open = true;
    driver.app_mut().nav.gallery.open = true;
    click_widget_center(&mut driver, &WidgetId::from("welcome.cancel")).expect("click Cancel");
    assert!(!driver.app().nav.welcome.open);
    assert!(!driver.app().nav.gallery.open);
    assert_eq!(driver.app().document().total_sheet_count(), sheet_count_before);
}

fn verify_welcome_locale_scale(locale: &str, idx: usize, scale: f64) -> i32 {
    let mut app = empty_welcome_app();
    app.locale = locale.to_owned();
    app.theme = match (idx + if scale > 1.5 { 1 } else { 0 }) % 3 {
        0 => UiThemeMode::Light.spec(),
        1 => UiThemeMode::Dark.spec(),
        _ => UiThemeMode::HighContrast.spec(),
    };
    let metrics = ViewportMetrics::new(1280.0, 800.0, scale);
    let mut driver = new_human_driver(app, metrics);
    let report = driver.verify_parity();
    let welcome_violations: Vec<_> = human_wf_common::filter_locale_parity_violations(locale, false, report.violations)
        .into_iter()
        .filter(|v| v.widget_id.starts_with("welcome."))
        .collect();
    assert!(welcome_violations.is_empty(), "welcome parity violation in `{locale}` @ {scale}x: {welcome_violations:?}");
    let tree = driver.layout_tree();
    if human_wf_common::host_covers_locale_sample(locale) {
        assert!(
            tree.text_runs.iter().filter(|r| r.owner_widget_id.as_str().starts_with("welcome.")).all(|r| r.shaped.missing_glyph_count == 0),
            "missing glyph detected in Welcome text_runs for `{locale}` @ {scale}x"
        );
    }
    if locale == "en" && (scale - 1.0).abs() < f64::EPSILON {
        let en_report = driver.verify_parity();
        assert!(en_report.is_clean(), "full-tree 1.0x English parity clean: {:?}", en_report.violations);
    }
    tree.node("welcome.left_rail").expect("left rail").bounds.rect_px.x
}

#[test]
fn r10_4_welcome_i18n_16_locales_themes_rtl_hidpi_and_catalog_completeness() {
    // 1. Parity + zero missing glyphs for Welcome widgets across all 16 locales, 3 themes, and 1.0x/2.0x HiDPI.
    let mut prev_ltr_rail_x = 0;
    let mut ar_rail_x = 0;
    let mut he_rail_x = 0;
    for (idx, &locale) in LOCALES.iter().enumerate() {
        for &scale in &[1.0, 2.0] {
            let rail_x = verify_welcome_locale_scale(locale, idx, scale);
            if (scale - 1.0).abs() < f64::EPSILON {
                match locale {
                    "en" => prev_ltr_rail_x = rail_x,
                    "ar" => ar_rail_x = rail_x,
                    "he" => he_rail_x = rail_x,
                    _ => {}
                }
            }
        }
    }
    assert!(ar_rail_x > prev_ltr_rail_x + 200, "RTL locale `ar` must mirror rail to right side ({ar_rail_x} vs LTR {prev_ltr_rail_x})");
    assert!(he_rail_x > prev_ltr_rail_x + 200, "RTL locale `he` must mirror rail to right side ({he_rail_x} vs LTR {prev_ltr_rail_x})");

    // 2. Render software RGBA on Welcome screen across Light vs Dark to confirm proportional glyph output (non-blank & distinct).
    let mut light_app = empty_welcome_app();
    light_app.theme = UiThemeMode::Light.spec();
    let mut dark_app = empty_welcome_app();
    dark_app.theme = UiThemeMode::Dark.spec();
    let target = RenderTargetSpec { width_px: 1024, height_px: 680, dpi: 96 };
    let img_light = render_workspace_rgba(&light_app, target).expect("render welcome light");
    let img_dark = render_workspace_rgba(&dark_app, target).expect("render welcome dark");
    assert_ne!(img_light.rgba, img_dark.rgba, "Light and Dark theme Welcome renders must differ");

    // 3. Verify `w5_welcome.rs` covers all `"welcome.*"` keys used by `welcome.rs` across 16 locales.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let welcome_rs = std::fs::read_to_string(manifest_dir.join("src/views/welcome.rs")).expect("read welcome.rs");
    let w5_i18n_rs = std::fs::read_to_string(manifest_dir.join("src/i18n/w5_welcome.rs")).expect("read w5_welcome.rs");
    let core_i18n_rs = std::fs::read_to_string(manifest_dir.join("src/i18n.rs")).expect("read i18n.rs");

    let mut used_welcome_keys = BTreeSet::new();
    let mut rest = welcome_rs.as_str();
    while let Some(pos) = rest.find("welcome_tr(&locale, \"") {
        rest = &rest[pos + "welcome_tr(&locale, \"".len()..];
        if let Some(q1) = rest.find('"') {
            used_welcome_keys.insert(rest[..q1].to_owned());
        }
    }
    assert!(used_welcome_keys.len() >= 18, "expected >= 18 inline welcome_tr keys, found {}", used_welcome_keys.len());

    let mut catalog_keys = BTreeSet::new();
    let mut scan = w5_i18n_rs.as_str();
    while let Some(pos) = scan.find("entry!(\"") {
        scan = &scan[pos + "entry!(\"".len()..];
        if let Some(end) = scan.find('"') {
            let key = &scan[..end];
            assert!(catalog_keys.insert(key.to_owned()), "duplicate key in w5_welcome.rs: {key}");
            assert!(!core_i18n_rs.contains(&format!("entry!(\"{key}\"")), "w5_welcome.rs key collides with i18n.rs: {key}");
            let block_end = scan.find("])").expect("entry closing");
            let block = &scan[..block_end];
            let ru_line = block.lines().find(|l| l.trim_start().starts_with("ru:")).expect("ru line");
            assert!(ru_line.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)), "missing Cyrillic in ru for `{key}`: {ru_line}");
            let ar_line = block.lines().find(|l| l.trim_start().starts_with("ar:")).expect("ar line");
            assert!(ar_line.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)), "missing Arabic in ar for `{key}`: {ar_line}");
            let he_line = block.lines().find(|l| l.trim_start().starts_with("he:")).expect("he line");
            assert!(he_line.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c)), "missing Hebrew in he for `{key}`: {he_line}");
        }
    }
    assert!(catalog_keys.len() >= 45, "expected >= 45 localized entries in w5_welcome.rs, found {}", catalog_keys.len());
    for used in &used_welcome_keys {
        assert!(catalog_keys.contains(used), "welcome.rs key `{used}` missing from src/i18n/w5_welcome.rs");
    }
    for table_key in [
        "welcome.x_mode.none",
        "welcome.x_mode.numbers",
        "welcome.x_mode.dates",
        "welcome.x_mode.elapsed",
        "welcome.x_mode.sequence",
        "welcome.subcol.single",
        "welcome.subcol.raw",
        "welcome.subcol.mean_sd_n",
        "welcome.subcol.mean_sem_n",
        "welcome.subcol.mean_cv_n",
        "welcome.subcol.mean_low_high",
        "welcome.subcol.upper_lower",
        "welcome.subcol.median_iqr",
        "welcome.design.unpaired",
        "welcome.design.paired",
        "welcome.design.precomputed",
    ] {
        assert!(catalog_keys.contains(table_key), "table-driven key `{table_key}` missing from w5_welcome.rs");
    }
}
