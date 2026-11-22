//! Import, Paste Special (5-tab `Source`, `View`, `Filter`, `Placement`, `Notes`),
//! and interactive `Convert Table Archetype…` dialog view builders (`R10.12`, `R10.4`).

use curvebook_doc::{
    Cell, CellCoord, CiBound, DataSheet, ImportFilterSpec, ImportPlacementSpec, PlacementMode, ResultCell, SheetCellRange, SheetId, TableArchetype,
    TableKind, format_linked_paste_tsv_cell,
};
use curvebook_format::{CsvOptions, parse_delimited_records};
use curvebook_transform::{
    PlateGeometry, PlateMapTemplate, PlateWellRole, TargetArchetypeKind, map_plate_to_datasheet, pivot_tidy_long_to_datasheet,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::grid_view::{ImportOriginKind, ImportReshapeMode, format_cell_display, parse_localized_number_input};
use crate::i18n::LocalePreset;
use crate::interaction::{TextFieldEditState, UiEventOutcome};

const WID_TITLE: &str = "import_paste.input.title";
const WID_X_TITLE: &str = "import_paste.input.x_title";
const WID_CSV: &str = "import_paste.input.csv";
const WID_PATH: &str = "import_paste.input.path";
const WID_PLATE_VALUES: &str = "import_paste.input.plate_values";
const WID_GROUP_COL: &str = "import_paste.input.group_col";
const WID_VALUE_COL: &str = "import_paste.input.value_col";
const WID_X_COL: &str = "import_paste.input.x_col";
const WID_REPLICATE_COL: &str = "import_paste.input.replicate_col";
const WID_EXCLUDED_COL: &str = "import_paste.input.excluded_col";
const WID_ROW_START: &str = "import_paste.input.row_start";
const WID_ROW_END: &str = "import_paste.input.row_end";
const WID_COL_START: &str = "import_paste.input.col_start";
const WID_COL_END: &str = "import_paste.input.col_end";
const WID_FILTER_PATTERN: &str = "import_paste.input.filter_pattern";
const WID_EXCLUDE_PRED: &str = "import_paste.input.exclude_predicate";
const WID_LINK_ID: &str = "import_paste.input.link_id";
const WID_NOTES: &str = "import_paste.input.notes";

/// Active tab of the 5-tab `Import & Paste Special` modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportPasteTab {
    Source,
    View,
    Filter,
    Placement,
    Notes,
}

impl ImportPasteTab {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::View => "view",
            Self::Filter => "filter",
            Self::Placement => "placement",
            Self::Notes => "notes",
        }
    }

    fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "view" => Self::View,
            "filter" => Self::Filter,
            "placement" => Self::Placement,
            "notes" => Self::Notes,
            _ => Self::Source,
        }
    }
}

/// Transient view state owned by the import / paste / format dialog shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportPasteViewState {
    pub active_tab: String,
    pub filter_pattern: String,
    pub source_origin: String,
    pub encoding: String,
    pub auto_detect_header: bool,
    pub has_header_row: bool,
    pub has_row_titles: bool,
    pub raw_source_text: String,
    pub external_path: String,
    pub internal_source_sheet_id: Option<String>,
    pub row_start: String,
    pub row_end: String,
    pub col_start: String,
    pub col_end: String,
    pub decimate_every_k: usize,
    pub unstack_enabled: bool,
    pub unstack_group_col: usize,
    pub parse_asterisk_exclusions: bool,
    pub exclude_predicate: String,
    pub plate_preset: String,
    pub transpose: bool,
    pub row_wrap_width: usize,
    pub propagate_title_header: bool,
    pub skip_contiguous_blanks: bool,
    pub target_mode: String,
    pub target_sheet_id: Option<String>,
    pub placement_mode: String,
    pub anchor_row: usize,
    pub anchor_col: usize,
    pub anchor_subcol: usize,
    pub paste_as_live_link: bool,
    pub link_id: String,
    pub provenance_notes: String,
    pub record_in_info_sheet: bool,
    pub last_status: String,
    pub convert_selected_col: usize,
    #[serde(default)]
    pub layout_revision: u64,
}

impl Default for ImportPasteViewState {
    fn default() -> Self {
        Self {
            active_tab: String::from("source"),
            filter_pattern: String::new(),
            source_origin: String::from("reshape"),
            encoding: String::from("utf-8"),
            auto_detect_header: true,
            has_header_row: true,
            has_row_titles: false,
            raw_source_text: String::new(),
            external_path: String::new(),
            internal_source_sheet_id: None,
            row_start: String::new(),
            row_end: String::new(),
            col_start: String::new(),
            col_end: String::new(),
            decimate_every_k: 1,
            unstack_enabled: false,
            unstack_group_col: 0,
            parse_asterisk_exclusions: true,
            exclude_predicate: String::new(),
            plate_preset: String::new(),
            transpose: false,
            row_wrap_width: 0,
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            target_mode: String::from("new_sheet"),
            target_sheet_id: None,
            placement_mode: String::from("overwrite"),
            anchor_row: 0,
            anchor_col: 0,
            anchor_subcol: 0,
            paste_as_live_link: false,
            link_id: String::from("link-1"),
            provenance_notes: String::new(),
            record_in_info_sheet: true,
            last_status: String::new(),
            convert_selected_col: 0,
            layout_revision: 0,
        }
    }
}

fn effective_text(app: &UiApp, wid: &str, fallback: &str) -> String {
    app.interaction.text_field_buffers.get(wid).map_or_else(|| fallback.to_owned(), |b| b.text.clone())
}

fn ensure_multiline_buffers(app: &mut UiApp) {
    let csv_init = if app.grid.import_reshape.tidy_csv.is_empty() {
        app.view_shards.import_paste.raw_source_text.clone()
    } else {
        app.grid.import_reshape.tidy_csv.clone()
    };
    app.interaction.text_field_buffers.entry(String::from(WID_CSV)).or_insert_with(|| TextFieldEditState::new(csv_init, true));
    let notes_init = app.view_shards.import_paste.provenance_notes.clone();
    app.interaction.text_field_buffers.entry(String::from(WID_NOTES)).or_insert_with(|| TextFieldEditState::new(notes_init, true));
    let plate_init = format_plate_values_csv(&app.grid.import_reshape.plate_values, app.grid.import_reshape.decimal_separator);
    app.interaction.text_field_buffers.entry(String::from(WID_PLATE_VALUES)).or_insert_with(|| TextFieldEditState::new(plate_init, true));
}

fn format_plate_values_csv(values: &[Option<f64>], decimal_sep: char) -> String {
    if values.is_empty() {
        return String::new();
    }
    values
        .iter()
        .map(|opt| match opt {
            None => String::from("NA"),
            Some(v) => {
                let s = format!("{v}");
                if decimal_sep == ',' { s.replace('.', ",") } else { s }
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Synchronize live `text_field_buffers` edits for `import_paste.input.*` when typed (`w5-integ` hook).
pub fn on_text_field_changed(app: &mut UiApp, field_id: &str, text: &str) {
    if !field_id.starts_with("import_paste.input.") {
        return;
    }
    let mut state = std::mem::take(&mut app.view_shards.import_paste);
    apply_single_text_field(app, &mut state, field_id, text);
    state.layout_revision = state.layout_revision.wrapping_add(1);
    app.view_shards = crate::views::ViewShardStates { import_paste: state, ..std::mem::take(&mut app.view_shards) };
}

/// Synchronize all modified `import_paste.input.*` text buffers into `state` and `app.grid`.
pub fn sync_text_buffers_into_state(app: &mut UiApp, state: &mut ImportPasteViewState) {
    let entries: Vec<(String, String)> = app
        .interaction
        .text_field_buffers
        .iter()
        .filter(|(k, _)| k.starts_with("import_paste.input."))
        .map(|(k, v)| (k.clone(), v.text.clone()))
        .collect();
    for (wid, text) in entries {
        apply_single_text_field(app, state, &wid, &text);
    }
    sync_import_wizard_from_state(app, state);
}

fn apply_single_text_field(app: &mut UiApp, state: &mut ImportPasteViewState, wid: &str, text: &str) {
    match wid {
        WID_TITLE => app.grid.import_reshape.title = text.to_owned(),
        WID_X_TITLE => app.grid.import_reshape.x_title = text.to_owned(),
        WID_CSV => {
            state.raw_source_text = text.to_owned();
            app.grid.import_reshape.tidy_csv = text.to_owned();
            app.grid.import_wizard.raw_clipboard_text = text.to_owned();
        }
        WID_PATH => {
            state.external_path = text.to_owned();
            app.grid.import_wizard.external_file_path = text.to_owned();
        }
        WID_PLATE_VALUES => {
            if text.trim().is_empty() {
                app.grid.import_reshape.plate_values.clear();
            } else if let Ok(vals) = parse_plate_text_with_decimal(text, app.grid.import_reshape.decimal_separator) {
                app.grid.import_reshape.plate_values = vals;
            }
        }
        WID_GROUP_COL => app.grid.import_reshape.group_col = text.to_owned(),
        WID_VALUE_COL => app.grid.import_reshape.value_col = text.to_owned(),
        WID_X_COL => app.grid.import_reshape.x_col = (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        WID_REPLICATE_COL => app.grid.import_reshape.replicate_col = (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        WID_EXCLUDED_COL => app.grid.import_reshape.excluded_col = (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        WID_ROW_START => state.row_start = text.trim().to_owned(),
        WID_ROW_END => state.row_end = text.trim().to_owned(),
        WID_COL_START => state.col_start = text.trim().to_owned(),
        WID_COL_END => state.col_end = text.trim().to_owned(),
        WID_FILTER_PATTERN => state.filter_pattern = text.to_owned(),
        WID_EXCLUDE_PRED => state.exclude_predicate = text.to_owned(),
        WID_LINK_ID => {
            state.link_id = text.to_owned();
            app.grid.import_wizard.link_id = text.to_owned();
        }
        WID_NOTES => {
            state.provenance_notes = text.to_owned();
            app.grid.import_wizard.provenance_note = text.to_owned();
        }
        _ => {}
    }
}

fn sync_import_wizard_from_state(app: &mut UiApp, state: &ImportPasteViewState) {
    let sid_opt = state.internal_source_sheet_id.as_deref().map(SheetId::new);
    let is_result_sheet = sid_opt.as_ref().is_some_and(|id| app.document().result_sheet(id).is_some());
    let w = &mut app.grid.import_wizard;
    w.origin_kind = match state.source_origin.as_str() {
        "external_csv" => ImportOriginKind::ExternalDelimitedFile,
        "internal_sheet" => {
            if is_result_sheet {
                ImportOriginKind::InternalResultSheet
            } else {
                ImportOriginKind::InternalDataSheet
            }
        }
        _ => ImportOriginKind::Clipboard,
    };
    w.delimiter = app.grid.import_reshape.tidy_delimiter;
    w.decimal_separator = app.grid.import_reshape.decimal_separator;
    w.source_sheet_id = state.internal_source_sheet_id.as_deref().and_then(|s| SheetId::try_new(s).ok());
    w.is_live_link = state.paste_as_live_link;
    w.filter = build_filter_spec(state);
    w.placement = build_placement_spec(state);
    w.link_id = state.link_id.clone();
    w.provenance_note = state.provenance_notes.clone();
    w.record_in_info_sheet = state.record_in_info_sheet;
}

fn parse_plate_text_with_decimal(raw: &str, decimal_sep: char) -> Result<Vec<Option<f64>>, String> {
    let mut preset = LocalePreset::period_comma("en-US");
    preset.decimal_separator = decimal_sep;
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = if trimmed.contains('\t') || trimmed.contains(';') || (decimal_sep != ',' && trimmed.contains(',')) {
            if decimal_sep == ',' { trimmed.split(['\t', ';']).collect() } else { trimmed.split([',', '\t', ';']).collect() }
        } else {
            trimmed.split_whitespace().collect()
        };
        for tok in tokens {
            let t = tok.trim();
            if t.is_empty() || t.eq_ignore_ascii_case("na") || t.eq_ignore_ascii_case("null") || t == "-" {
                out.push(None);
            } else {
                let val =
                    parse_localized_number_input(t, &preset).filter(|v| v.is_finite()).ok_or_else(|| format!("invalid plate numeric token `{t}`"))?;
                out.push(Some(val));
            }
        }
    }
    Ok(out)
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `import_paste` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ImportPasteViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "smoke" || payload.is_empty() {
        return Ok(UiEventOutcome::default());
    }
    // Bump shard revision so view-state fingerprints invalidate even when underlying `grid`/`convert_archetype` state changes.
    state.layout_revision = state.layout_revision.wrapping_add(1);
    ensure_multiline_buffers(app);
    sync_text_buffers_into_state(app, state);

    if let Some(tab_id) = payload.strip_prefix("tab:") {
        state.active_tab = ImportPasteTab::from_str(tab_id).as_str().to_owned();
        if state.active_tab == "source" {
            ensure_multiline_buffers(app);
        }
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.tab:{}", state.active_tab), ..Default::default() });
    }

    if let Some(target_str) = payload.strip_prefix("convert.target:") {
        return handle_convert_target_action(app, target_str);
    }

    if let Some(idx_str) = payload.strip_prefix("convert.map_row:")
        && let Ok(idx) = idx_str.parse::<usize>()
    {
        state.convert_selected_col = idx;
        return Ok(UiEventOutcome { handled: true, action: format!("convert.map_row:{idx}"), ..Default::default() });
    }

    if let Some(toggle_field) = payload.strip_prefix("convert.toggle:") {
        match toggle_field {
            "allow_lossy" => app.canvas_dialogs.convert_archetype.allow_lossy = !app.canvas_dialogs.convert_archetype.allow_lossy,
            "preview_only" => app.canvas_dialogs.convert_archetype.preview_only = !app.canvas_dialogs.convert_archetype.preview_only,
            _ => return Ok(UiEventOutcome::default()),
        }
        return Ok(UiEventOutcome { handled: true, action: format!("convert.toggle:{toggle_field}"), ..Default::default() });
    }

    if let Some(src) = payload.strip_prefix("source:") {
        match src {
            "plate96" => {
                state.source_origin = String::from("reshape");
                app.grid.import_reshape.mode = ImportReshapeMode::Plate96;
            }
            "plate384" => {
                state.source_origin = String::from("reshape");
                app.grid.import_reshape.mode = ImportReshapeMode::Plate384;
            }
            "tidy" => {
                state.source_origin = String::from("reshape");
                app.grid.import_reshape.mode = ImportReshapeMode::TidyLongPivot;
            }
            "clipboard" => {
                state.source_origin = String::from("clipboard");
                state.paste_as_live_link = false;
            }
            "external_csv" => {
                state.source_origin = String::from("external_csv");
            }
            "internal_sheet" => {
                state.source_origin = String::from("internal_sheet");
                if state.internal_source_sheet_id.is_none() {
                    state.internal_source_sheet_id = first_available_source_sheet_id(app);
                }
            }
            _ => return Ok(UiEventOutcome::default()),
        }
        sync_import_wizard_from_state(app, state);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.source:{src}"), ..Default::default() });
    }

    if let Some(target_kind) = payload.strip_prefix("target:") {
        let parsed = match target_kind {
            "Column" => TableKind::Column,
            "Grouped" => TableKind::Grouped,
            "Xy" => TableKind::Xy,
            "MultipleVariables" => TableKind::MultipleVariables,
            _ => return Ok(UiEventOutcome::default()),
        };
        app.grid.import_reshape.target_kind = parsed;
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.target:{target_kind}"), ..Default::default() });
    }

    if let Some(delim) = payload.strip_prefix("delim:") {
        app.grid.import_reshape.tidy_delimiter = match delim {
            "comma" | "," => ',',
            "tab" | "\\t" | "\t" => '\t',
            "semicolon" | ";" => ';',
            "pipe" | "|" => '|',
            _ => ',',
        };
        sync_import_wizard_from_state(app, state);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.delim:{delim}"), ..Default::default() });
    }

    if let Some(dec) = payload.strip_prefix("decimal:") {
        app.grid.import_reshape.decimal_separator = if dec == "," { ',' } else { '.' };
        sync_import_wizard_from_state(app, state);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.decimal:{dec}"), ..Default::default() });
    }

    if let Some(enc) = payload.strip_prefix("encoding:") {
        state.encoding = enc.to_owned();
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.encoding:{enc}"), ..Default::default() });
    }

    if let Some(sheet_id) = payload.strip_prefix("internal_sheet:") {
        state.internal_source_sheet_id = Some(sheet_id.to_owned());
        state.source_origin = String::from("internal_sheet");
        sync_import_wizard_from_state(app, state);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.internal_sheet:{sheet_id}"), ..Default::default() });
    }

    if let Some(dest) = payload.strip_prefix("target_dest:") {
        state.target_mode = if dest == "existing" { String::from("existing_sheet") } else { String::from("new_sheet") };
        if state.target_mode == "existing_sheet" && state.target_sheet_id.is_none() {
            state.target_sheet_id = default_target_data_sheet_id(app);
        }
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.target_dest:{dest}"), ..Default::default() });
    }

    if let Some(sheet_id) = payload.strip_prefix("dest_sheet:") {
        state.target_sheet_id = Some(sheet_id.to_owned());
        state.target_mode = String::from("existing_sheet");
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.dest_sheet:{sheet_id}"), ..Default::default() });
    }

    if let Some(pmode) = payload.strip_prefix("place_mode:") {
        state.placement_mode = pmode.to_owned();
        sync_import_wizard_from_state(app, state);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.place_mode:{pmode}"), ..Default::default() });
    }

    if let Some(preset) = payload.strip_prefix("plate_preset:") {
        apply_plate_preset(app, state, preset);
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.plate_preset:{preset}"), ..Default::default() });
    }

    if let Some(w_str) = payload.strip_prefix("row_wrap_set:")
        && let Ok(w) = w_str.parse::<usize>()
    {
        state.row_wrap_width = w;
        return Ok(UiEventOutcome { handled: true, action: format!("import_paste.row_wrap_set:{w}"), ..Default::default() });
    }

    match payload {
        "detect_format" => {
            auto_detect_source_format(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.detect_format"), ..Default::default() })
        }
        "paste_clipboard" => {
            if let Some(clip) = app.interaction.last_clipboard_text.clone() {
                set_raw_csv_text(app, state, &clip);
                state.last_status = format!("Pasted {} bytes from clipboard.", clip.len());
            } else {
                state.last_status = String::from("Clipboard buffer is empty.");
            }
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.paste_clipboard"), ..Default::default() })
        }
        "load_sample" => {
            load_sample_for_active_origin(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.load_sample"), ..Default::default() })
        }
        "clear_slice" => {
            state.row_start.clear();
            state.row_end.clear();
            state.col_start.clear();
            state.col_end.clear();
            state.decimate_every_k = 1;
            for wid in [WID_ROW_START, WID_ROW_END, WID_COL_START, WID_COL_END] {
                app.interaction.text_field_buffers.insert(String::from(wid), TextFieldEditState::new("", false));
            }
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.clear_slice"), ..Default::default() })
        }
        "toggle:has_header" => {
            state.has_header_row = !state.has_header_row;
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:has_header"), ..Default::default() })
        }
        "toggle:has_row_titles" => {
            state.has_row_titles = !state.has_row_titles;
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:has_row_titles"), ..Default::default() })
        }
        "toggle:auto_detect_header" => {
            state.auto_detect_header = !state.auto_detect_header;
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:auto_detect_header"), ..Default::default() })
        }
        "toggle:unstack" => {
            state.unstack_enabled = !state.unstack_enabled;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:unstack"), ..Default::default() })
        }
        "toggle:parse_asterisk" => {
            state.parse_asterisk_exclusions = !state.parse_asterisk_exclusions;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:parse_asterisk"), ..Default::default() })
        }
        "toggle:subtract_blank_mean" => {
            app.grid.import_reshape.subtract_blank_mean = !app.grid.import_reshape.subtract_blank_mean;
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:subtract_blank_mean"), ..Default::default() })
        }
        "toggle:transpose" => {
            state.transpose = !state.transpose;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:transpose"), ..Default::default() })
        }
        "toggle:propagate_headers" => {
            state.propagate_title_header = !state.propagate_title_header;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:propagate_headers"), ..Default::default() })
        }
        "toggle:skip_blanks" => {
            state.skip_contiguous_blanks = !state.skip_contiguous_blanks;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:skip_blanks"), ..Default::default() })
        }
        "toggle:live_link" => {
            state.paste_as_live_link = !state.paste_as_live_link;
            if state.paste_as_live_link && state.target_mode == "new_sheet" {
                state.target_mode = String::from("existing_sheet");
                if state.target_sheet_id.is_none() {
                    state.target_sheet_id = default_target_data_sheet_id(app);
                }
            }
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:live_link"), ..Default::default() })
        }
        "toggle:record_info" => {
            state.record_in_info_sheet = !state.record_in_info_sheet;
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.toggle:record_info"), ..Default::default() })
        }
        "decimate:-1" => {
            state.decimate_every_k = state.decimate_every_k.saturating_sub(1).max(1);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.decimate:-1"), ..Default::default() })
        }
        "decimate:+1" => {
            state.decimate_every_k = state.decimate_every_k.saturating_add(1).min(100);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.decimate:+1"), ..Default::default() })
        }
        "unstack_col:-1" => {
            state.unstack_group_col = state.unstack_group_col.saturating_sub(1);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.unstack_col:-1"), ..Default::default() })
        }
        "unstack_col:+1" => {
            state.unstack_group_col = state.unstack_group_col.saturating_add(1).min(64);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.unstack_col:+1"), ..Default::default() })
        }
        "row_wrap:-1" => {
            state.row_wrap_width = state.row_wrap_width.saturating_sub(1);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.row_wrap:-1"), ..Default::default() })
        }
        "row_wrap:+1" => {
            state.row_wrap_width = state.row_wrap_width.saturating_add(1).min(64);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.row_wrap:+1"), ..Default::default() })
        }
        "anchor_row:-1" => {
            state.anchor_row = state.anchor_row.saturating_sub(1);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_row:-1"), ..Default::default() })
        }
        "anchor_row:+1" => {
            state.anchor_row = state.anchor_row.saturating_add(1).min(10_000);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_row:+1"), ..Default::default() })
        }
        "anchor_col:-1" => {
            state.anchor_col = state.anchor_col.saturating_sub(1);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_col:-1"), ..Default::default() })
        }
        "anchor_col:+1" => {
            state.anchor_col = state.anchor_col.saturating_add(1).min(256);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_col:+1"), ..Default::default() })
        }
        "anchor_subcol:-1" => {
            state.anchor_subcol = state.anchor_subcol.saturating_sub(1);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_subcol:-1"), ..Default::default() })
        }
        "anchor_subcol:+1" => {
            state.anchor_subcol = state.anchor_subcol.saturating_add(1).min(64);
            sync_import_wizard_from_state(app, state);
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.anchor_subcol:+1"), ..Default::default() })
        }
        "sync" => {
            state.last_status = String::from("Synchronized source, filter, and placement preview.");
            Ok(UiEventOutcome { handled: true, action: String::from("import_paste.sync"), ..Default::default() })
        }
        "commit" => match execute_import_paste_commit(app, state) {
            Ok(res) => {
                app.canvas_dialogs.close_dialog();
                Ok(UiEventOutcome { handled: true, action: String::from("import_paste.committed"), command_result: Some(res), ..Default::default() })
            }
            Err(err) => {
                state.last_status = err.clone();
                Err(err)
            }
        },
        _ => Ok(UiEventOutcome::default()),
    }
}

fn handle_convert_target_action(app: &mut UiApp, target_str: &str) -> Result<UiEventOutcome, String> {
    let kind = crate::dialogs_canvas::parse_table_kind_str(target_str)?;
    app.canvas_dialogs.convert_archetype.set_target_kind(kind);
    if let Some(intent) = app.canvas_dialogs.convert_archetype.preview_intent() {
        match app.execute_session_ui_cmd(&intent.command_id, intent.params) {
            Ok(out) => app.canvas_dialogs.convert_archetype.apply_preview_payload(&out.payload),
            Err(err) => {
                app.canvas_dialogs.convert_archetype.preview_lossy = false;
                app.canvas_dialogs.convert_archetype.preserved_cells = 0;
                app.canvas_dialogs.convert_archetype.dropped_cells = 0;
                app.canvas_dialogs.convert_archetype.preview_warnings = vec![err.to_string()];
            }
        }
    }
    Ok(UiEventOutcome { handled: true, action: format!("convert.target:{target_str}"), ..Default::default() })
}

fn set_raw_csv_text(app: &mut UiApp, state: &mut ImportPasteViewState, text: &str) {
    state.raw_source_text = text.to_owned();
    app.grid.import_reshape.tidy_csv = text.to_owned();
    app.grid.import_wizard.raw_clipboard_text = text.to_owned();
    app.interaction.text_field_buffers.insert(String::from(WID_CSV), TextFieldEditState::new(text, true));
}

fn first_available_source_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.data_sheets.first().map(|s| s.id.as_str().to_owned()).or_else(|| doc.result_sheets.first().map(|s| s.id.as_str().to_owned()))
}

fn default_target_data_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.data_sheet(id).is_some())
        .or_else(|| doc.data_sheets.first().map(|d| &d.id))
        .map(|id| id.as_str().to_owned())
}

fn load_sample_for_active_origin(app: &mut UiApp, state: &mut ImportPasteViewState) {
    if state.source_origin == "reshape" && matches!(app.grid.import_reshape.mode, ImportReshapeMode::Plate96) {
        apply_plate_preset(app, state, "96");
        return;
    }
    if state.source_origin == "reshape" && matches!(app.grid.import_reshape.mode, ImportReshapeMode::Plate384) {
        apply_plate_preset(app, state, "384");
        return;
    }
    let sample =
        "Group,Concentration,Value,Excluded\nControl,0.1,10.2,false\nControl,1.0,11.4*,false\nTreated,0.1,18.5,false\nTreated,1.0,24.8,false\n";
    set_raw_csv_text(app, state, sample);
    app.grid.import_reshape.group_col = String::from("Group");
    app.grid.import_reshape.value_col = String::from("Value");
    app.grid.import_reshape.x_col = Some(String::from("Concentration"));
    app.grid.import_reshape.excluded_col = Some(String::from("Excluded"));
    app.interaction.text_field_buffers.insert(String::from(WID_GROUP_COL), TextFieldEditState::new("Group", false));
    app.interaction.text_field_buffers.insert(String::from(WID_VALUE_COL), TextFieldEditState::new("Value", false));
    app.interaction.text_field_buffers.insert(String::from(WID_X_COL), TextFieldEditState::new("Concentration", false));
    app.interaction.text_field_buffers.insert(String::from(WID_EXCLUDED_COL), TextFieldEditState::new("Excluded", false));
    auto_detect_source_format(app, state);
}

fn apply_plate_preset(app: &mut UiApp, state: &mut ImportPasteViewState, preset: &str) {
    let is_384 = preset == "384";
    app.grid.import_reshape.mode = if is_384 { ImportReshapeMode::Plate384 } else { ImportReshapeMode::Plate96 };
    state.source_origin = String::from("reshape");
    state.plate_preset = preset.to_owned();
    let count = if is_384 { 384 } else { 96 };
    let cols = if is_384 { 24 } else { 12 };
    let mut vals = Vec::with_capacity(count);
    let mut roles = Vec::with_capacity(count);
    let doses = [0.1_f64, 1.0, 10.0, 100.0];
    for idx in 0..count {
        let r = idx / cols;
        let c = idx % cols;
        if r == 0 && c < 4 {
            roles.push(json!("Blank"));
            vals.push(Some(10.0));
        } else if r == 0 && c < 8 {
            roles.push(json!("Excluded"));
            vals.push(Some(99.0));
        } else {
            let grp = if r % 2 == 0 { "CompoundA" } else { "CompoundB" };
            let dose = doses[c % doses.len()];
            let rep = (r / 2) + 1;
            roles.push(json!({ "group_name": grp, "dilution_x": dose, "replicate": rep }));
            vals.push(Some(10.0 + dose * if grp == "CompoundA" { 1.8 } else { 1.0 }));
        }
    }
    app.grid.import_reshape.plate_values = vals;
    app.grid.import_reshape.plate_roles = roles;
    let formatted = format_plate_values_csv(&app.grid.import_reshape.plate_values, app.grid.import_reshape.decimal_separator);
    app.interaction.text_field_buffers.insert(String::from(WID_PLATE_VALUES), TextFieldEditState::new(formatted, true));
    state.last_status = format!("Applied {count}-well dose-response plate template.");
}

fn detect_encoding_label(raw: &str) -> &'static str {
    if raw.starts_with('\u{FEFF}') {
        "utf-8-bom"
    } else if raw.contains('\0') {
        "utf-16le"
    } else if raw.chars().any(|c| ('\u{00A0}'..='\u{00FF}').contains(&c)) && !raw.is_ascii() {
        "latin-1"
    } else {
        "utf-8"
    }
}

fn normalize_text_for_encoding(raw: &str, encoding: &str) -> String {
    let stripped = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    match encoding {
        "utf-16le" | "utf-16be" => stripped.chars().filter(|&c| c != '\0').collect(),
        _ => stripped.to_owned(),
    }
}

fn auto_detect_source_format(app: &mut UiApp, state: &mut ImportPasteViewState) {
    let raw = effective_source_text(app, state);
    if raw.trim().is_empty() {
        state.last_status = String::from("No source text present to auto-detect.");
        return;
    }
    let detected_enc = detect_encoding_label(&raw);
    state.encoding = String::from(detected_enc);
    let clean = normalize_text_for_encoding(&raw, &state.encoding);
    let mut opts = CsvOptions::detect(&clean);
    let pipe_count = clean.lines().take(5).map(|l| l.matches('|').count()).sum::<usize>();
    let comma_count = clean.lines().take(5).map(|l| l.matches(',').count()).sum::<usize>();
    let semi_count = clean.lines().take(5).map(|l| l.matches(';').count()).sum::<usize>();
    let tab_count = clean.lines().take(5).map(|l| l.matches('\t').count()).sum::<usize>();
    if pipe_count > comma_count.max(semi_count).max(tab_count) {
        opts.delimiter = '|';
    }
    app.grid.import_reshape.tidy_delimiter = opts.delimiter;
    app.grid.import_reshape.decimal_separator = opts.decimal_separator;

    if state.auto_detect_header
        && let Ok(recs) = parse_delimited_records(&clean, &opts)
        && recs.len() >= 2
    {
        let row0_has_text = recs[0].iter().any(|tok| {
            let t = tok.trim();
            !t.is_empty() && parse_cell_token_preview(t, opts.decimal_separator, true).active_number().is_none()
        });
        let row1_has_num = recs[1].iter().any(|tok| parse_cell_token_preview(tok, opts.decimal_separator, true).active_number().is_some());
        state.has_header_row = row0_has_text && row1_has_num;
    }

    sync_import_wizard_from_state(app, state);
    state.last_status = format!(
        "Detected delim `{}`, decimal `{}`, encoding {}, header={}",
        delimiter_display_short(opts.delimiter),
        opts.decimal_separator,
        state.encoding,
        state.has_header_row
    );
}

fn delimiter_display_short(delim: char) -> &'static str {
    match delim {
        '\t' => "Tab",
        ';' => ";",
        '|' => "|",
        _ => ",",
    }
}

fn effective_source_text(app: &UiApp, state: &ImportPasteViewState) -> String {
    let fallback = if !app.grid.import_reshape.tidy_csv.is_empty() {
        app.grid.import_reshape.tidy_csv.as_str()
    } else if !state.raw_source_text.is_empty() {
        state.raw_source_text.as_str()
    } else {
        app.grid.import_wizard.raw_clipboard_text.as_str()
    };
    effective_text(app, WID_CSV, fallback)
}

fn effective_plate_values(app: &UiApp) -> Vec<Option<f64>> {
    if let Some(buf) = app.interaction.text_field_buffers.get(WID_PLATE_VALUES)
        && !buf.text.trim().is_empty()
        && let Ok(parsed) = parse_plate_text_with_decimal(&buf.text, app.grid.import_reshape.decimal_separator)
    {
        return parsed;
    }
    app.grid.import_reshape.plate_values.clone()
}

fn parse_cell_token_preview(tok: &str, decimal_sep: char, parse_asterisk: bool) -> Cell {
    Cell::parse_import_token(tok, decimal_sep, parse_asterisk)
}

fn build_filter_spec(state: &ImportPasteViewState) -> ImportFilterSpec {
    let r0 = state.row_start.trim().parse::<usize>().ok();
    let r1 = state.row_end.trim().parse::<usize>().ok();
    let row_window = match (r0, r1) {
        (Some(a), Some(b)) => Some((a, b.max(a))),
        (Some(a), None) => Some((a, usize::MAX / 2)),
        (None, Some(b)) => Some((0, b)),
        (None, None) => None,
    };
    let c0 = state.col_start.trim().parse::<usize>().ok();
    let c1 = state.col_end.trim().parse::<usize>().ok();
    let col_window = match (c0, c1) {
        (Some(a), Some(b)) => Some((a, b.max(a))),
        (Some(a), None) => Some((a, usize::MAX / 2)),
        (None, Some(b)) => Some((0, b)),
        (None, None) => None,
    };
    ImportFilterSpec {
        row_window,
        col_window,
        decimate_every_k: (state.decimate_every_k > 1).then_some(state.decimate_every_k),
        unstack_group_column: state.unstack_enabled.then_some(state.unstack_group_col),
        parse_asterisk_exclusions: state.parse_asterisk_exclusions,
        baseline: None,
        exclude_row_predicate: None,
    }
}

fn parse_placement_mode_str(raw: &str) -> PlacementMode {
    match raw {
        "insert_rows" => PlacementMode::InsertRows,
        "append_rows" => PlacementMode::AppendRows,
        "append_columns" => PlacementMode::AppendColumns,
        _ => PlacementMode::Overwrite,
    }
}

fn build_placement_spec(state: &ImportPasteViewState) -> ImportPlacementSpec {
    ImportPlacementSpec {
        mode: parse_placement_mode_str(&state.placement_mode),
        anchor: CellCoord::new(state.anchor_row, state.anchor_col, state.anchor_subcol),
        propagate_title_header: state.propagate_title_header,
        skip_contiguous_blanks: state.skip_contiguous_blanks,
        transpose: state.transpose,
    }
}

/// Execute the currently configured `ImportReshape` / `Paste Special` operation (`w5-integ` submit hook).
///
/// # Errors
/// Returns a descriptive error string when inputs are incomplete or the engine rejects the command.
pub fn commit_import_paste_dialog(app: &mut UiApp) -> Result<Value, String> {
    let mut state = std::mem::take(&mut app.view_shards.import_paste);
    sync_text_buffers_into_state(app, &mut state);
    let res = execute_import_paste_commit(app, &mut state);
    app.view_shards.import_paste = state;
    res
}

/// Execute the currently configured `ConvertArchetype` operation (`w5-integ` submit hook).
///
/// # Errors
/// Returns a descriptive error string when conversion preview fails or lossy confirmation is missing.
pub fn commit_convert_archetype_dialog(app: &mut UiApp) -> Result<Value, String> {
    if app.canvas_dialogs.convert_archetype.preview_only {
        let intent =
            app.canvas_dialogs.convert_archetype.preview_intent().ok_or_else(|| String::from("no data sheet bound for archetype conversion"))?;
        let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
        app.canvas_dialogs.convert_archetype.apply_preview_payload(&out.payload);
        app.canvas_dialogs.close_dialog();
        return Ok(json!({ "preview_only": true, "payload": out.payload }));
    }
    let out = app.commit_convert_archetype()?;
    Ok(json!({ "mutated": out.mutated, "payload": out.payload }))
}

fn is_fast_path_tidy_pivot(app: &UiApp, state: &ImportPasteViewState) -> bool {
    let st = &app.grid.import_reshape;
    state.row_wrap_width == 0
        && state.row_start.is_empty()
        && state.row_end.is_empty()
        && state.col_start.is_empty()
        && state.col_end.is_empty()
        && state.decimate_every_k <= 1
        && state.filter_pattern.trim().is_empty()
        && state.exclude_predicate.trim().is_empty()
        && !state.transpose
        && !state.unstack_enabled
        && !state.has_row_titles
        && !state.skip_contiguous_blanks
        && state.has_header_row
        && state.parse_asterisk_exclusions
        && state.encoding.eq_ignore_ascii_case("utf-8")
        && state.target_mode == "new_sheet"
        && effective_source_text(app, state) == st.tidy_csv
        && effective_text(app, WID_GROUP_COL, &st.group_col) == st.group_col
        && effective_text(app, WID_VALUE_COL, &st.value_col) == st.value_col
}

fn commit_external_csv_path(app: &mut UiApp, state: &ImportPasteViewState, path: &str) -> Result<Value, String> {
    let target_id = resolve_existing_target_sheet(app, state)?;
    let filter = build_filter_spec(state);
    let placement = build_placement_spec(state);
    let mut params = serde_json::Map::new();
    params.insert(String::from("target_sheet_id"), json!(target_id.as_str()));
    params.insert(String::from("filter"), json!(filter));
    params.insert(String::from("anchor"), json!(placement.anchor));
    params.insert(String::from("mode"), json!(state.placement_mode));
    params.insert(String::from("transpose"), json!(placement.transpose));
    params.insert(String::from("propagate_title_header"), json!(placement.propagate_title_header));
    params.insert(String::from("skip_contiguous_blanks"), json!(placement.skip_contiguous_blanks));
    if !state.link_id.trim().is_empty() {
        params.insert(String::from("link_id"), json!(state.link_id.trim()));
    }
    if !state.provenance_notes.trim().is_empty() {
        params.insert(String::from("provenance"), json!(state.provenance_notes.trim()));
    }
    if !state.exclude_predicate.trim().is_empty() {
        params.insert(String::from("exclude_row_predicate"), json!(state.exclude_predicate.trim()));
    }
    params.insert(String::from("source_path"), json!(path.trim()));
    params.insert(String::from("delimiter"), json!(app.grid.import_reshape.tidy_delimiter.to_string()));
    params.insert(String::from("decimal_separator"), json!(app.grid.import_reshape.decimal_separator.to_string()));
    let out = app.execute_command("table.pasteLink", Value::Object(params)).map_err(|e| e.to_string())?;
    maybe_append_provenance_to_info(app, state, Some(target_id.as_str()));
    Ok(out.payload)
}

fn commit_external_csv_non_live(app: &mut UiApp, state: &ImportPasteViewState, path: &str) -> Result<Value, String> {
    let mut params = serde_json::Map::new();
    params.insert(String::from("path"), json!(path.trim()));
    params.insert(String::from("link"), json!(false));
    params.insert(String::from("delimiter"), json!(app.grid.import_reshape.tidy_delimiter.to_string()));
    params.insert(String::from("decimal_separator"), json!(app.grid.import_reshape.decimal_separator.to_string()));
    params.insert(String::from("has_header"), json!(state.has_header_row));
    params.insert(String::from("has_row_titles"), json!(state.has_row_titles));
    let target_id_opt = if state.target_mode == "existing_sheet" {
        let target_id = resolve_existing_target_sheet(app, state)?;
        let filter = build_filter_spec(state);
        let placement = build_placement_spec(state);
        params.insert(String::from("sheet_id"), json!(target_id.as_str()));
        params.insert(String::from("filter"), json!(filter));
        params.insert(String::from("anchor"), json!(placement.anchor));
        params.insert(String::from("mode"), json!(state.placement_mode));
        params.insert(String::from("propagate_title_header"), json!(placement.propagate_title_header));
        params.insert(String::from("skip_contiguous_blanks"), json!(placement.skip_contiguous_blanks));
        params.insert(String::from("transpose"), json!(placement.transpose));
        Some(target_id)
    } else {
        let raw_title = app.grid.import_reshape.title.trim();
        let title = if raw_title.is_empty() { "Imported Table" } else { raw_title };
        let target_kind = app.grid.import_reshape.target_kind;
        let subcol_wire = if matches!(target_kind, TableKind::Grouped | TableKind::Nested) { "raw_replicates" } else { "single" };
        params.insert(String::from("title"), json!(title));
        params.insert(String::from("archetype"), json!(table_kind_wire_name(target_kind)));
        params.insert(String::from("subcolumn_mode"), json!(subcol_wire));
        params.insert(String::from("replicates"), json!(1));
        None
    };
    let out = app.execute_command("table.importCsv", Value::Object(params)).map_err(|e| e.to_string())?;
    let resolved_sid = target_id_opt.as_ref().map_or_else(|| out.payload.get("sheet_id").and_then(Value::as_str), |id| Some(id.as_str()));
    maybe_append_provenance_to_info(app, state, resolved_sid);
    Ok(out.payload)
}

fn execute_import_paste_commit(app: &mut UiApp, state: &mut ImportPasteViewState) -> Result<Value, String> {
    if state.source_origin == "reshape" {
        if matches!(app.grid.import_reshape.mode, ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384) {
            let intent = app.grid.import_reshape.build_import_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
            return Ok(out.payload);
        }
        // TidyLongPivot: honor extra slicing/row-wrap/header/exclusion/row-title options via preview pipeline,
        // else take standard table.importTidyLong fast path.
        if is_fast_path_tidy_pivot(app, state) {
            let intent = app.grid.import_reshape.build_import_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
            return Ok(out.payload);
        }
    }

    if state.source_origin == "external_csv" && effective_source_text(app, state).trim().is_empty() {
        let path = effective_text(app, WID_PATH, &state.external_path);
        if path.trim().is_empty() {
            return Err(String::from(if state.paste_as_live_link {
                "live external CSV link requires a file path"
            } else {
                "external CSV import requires a file path or CSV text"
            }));
        }
        if state.paste_as_live_link {
            return commit_external_csv_path(app, state, &path);
        }
        return commit_external_csv_non_live(app, state, &path);
    }

    let preview = compute_import_preview(app, state);
    if let Some(err) = preview.validation_error {
        return Err(err);
    }

    if state.paste_as_live_link && matches!(state.source_origin.as_str(), "internal_sheet" | "external_csv") {
        let target_id = resolve_existing_target_sheet(app, state)?;
        let filter = build_filter_spec(state);
        let placement = build_placement_spec(state);
        let mut params = serde_json::Map::new();
        params.insert(String::from("target_sheet_id"), json!(target_id.as_str()));
        params.insert(String::from("filter"), json!(filter));
        params.insert(String::from("anchor"), json!(placement.anchor));
        params.insert(String::from("mode"), json!(state.placement_mode));
        params.insert(String::from("transpose"), json!(placement.transpose));
        params.insert(String::from("propagate_title_header"), json!(placement.propagate_title_header));
        params.insert(String::from("skip_contiguous_blanks"), json!(placement.skip_contiguous_blanks));
        if !state.link_id.trim().is_empty() {
            params.insert(String::from("link_id"), json!(state.link_id.trim()));
        }
        if !state.provenance_notes.trim().is_empty() {
            params.insert(String::from("provenance"), json!(state.provenance_notes.trim()));
        }
        if !state.exclude_predicate.trim().is_empty() {
            params.insert(String::from("exclude_row_predicate"), json!(state.exclude_predicate.trim()));
        }
        if state.source_origin == "internal_sheet" {
            let src = state.internal_source_sheet_id.as_deref().ok_or_else(|| String::from("select an internal source sheet first"))?;
            params.insert(String::from("source_sheet_id"), json!(src));
        } else {
            let path = effective_text(app, WID_PATH, &state.external_path);
            if path.trim().is_empty() {
                return Err(String::from("live external CSV link requires a file path"));
            }
            params.insert(String::from("source_path"), json!(path.trim()));
            params.insert(String::from("delimiter"), json!(app.grid.import_reshape.tidy_delimiter.to_string()));
            params.insert(String::from("decimal_separator"), json!(app.grid.import_reshape.decimal_separator.to_string()));
        }
        let out = app.execute_command("table.pasteLink", Value::Object(params)).map_err(|e| e.to_string())?;
        maybe_append_provenance_to_info(app, state, Some(target_id.as_str()));
        return Ok(out.payload);
    }

    if state.target_mode == "existing_sheet" {
        let target_id = resolve_existing_target_sheet(app, state)?;
        let tsv = serialize_preview_as_tsv(&preview, state.propagate_title_header);
        if tsv.trim().is_empty() {
            return Err(String::from("no data rows available to paste after filtering"));
        }
        let out = app
            .execute_command(
                "table.paste",
                json!({
                    "sheet_id": target_id.as_str(),
                    "text": tsv,
                    "delimiter": "\t",
                    "decimal_separator": ".",
                    "start_row": state.anchor_row,
                    "start_col": state.anchor_col,
                    "start_subcol": state.anchor_subcol,
                    "mode": state.placement_mode,
                    "propagate_title_header": state.propagate_title_header,
                    "skip_contiguous_blanks": state.skip_contiguous_blanks,
                    "transpose": false,
                }),
            )
            .map_err(|e| e.to_string())?;
        let placed_start_row =
            out.payload.pointer("/target_range/start/row").and_then(Value::as_u64).and_then(|r| usize::try_from(r).ok()).unwrap_or(state.anchor_row);
        if !state.parse_asterisk_exclusions {
            let placed_start_col = out
                .payload
                .pointer("/target_range/start/col")
                .and_then(Value::as_u64)
                .and_then(|c| usize::try_from(c).ok())
                .unwrap_or(state.anchor_col);
            let placed_start_sub = out
                .payload
                .pointer("/target_range/start/subcol")
                .and_then(Value::as_u64)
                .and_then(|s| usize::try_from(s).ok())
                .unwrap_or(state.anchor_subcol);
            restore_unexcluded_asterisk_cells(app, target_id.as_str(), &preview.rows, placed_start_row, placed_start_col, placed_start_sub, false)?;
        }
        if (state.has_row_titles || state.transpose || state.source_origin == "internal_sheet")
            && !preview.row_labels.is_empty()
            && !preview.rows.is_empty()
        {
            let titles: Vec<Option<String>> = preview
                .row_labels
                .iter()
                .take(preview.rows.len())
                .map(|lbl| {
                    let trimmed = lbl.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_owned())
                })
                .collect();
            if titles.iter().any(Option::is_some) {
                app.execute_command(
                    "table.setRowTitles",
                    json!({
                        "sheet_id": target_id.as_str(),
                        "start_row": placed_start_row,
                        "titles": titles,
                    }),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        maybe_append_provenance_to_info(app, state, Some(target_id.as_str()));
        return Ok(out.payload);
    }

    // Create new sheet via table.importCsv from the reshaped preview matrix.
    let csv_text = serialize_preview_as_csv_with_headers(&preview);
    if csv_text.trim().is_empty() {
        return Err(String::from("no rows available to import"));
    }
    let title = {
        let raw = app.grid.import_reshape.title.trim();
        if raw.is_empty() { "Imported Table" } else { raw }
    };
    let target_kind = app.grid.import_reshape.target_kind;
    let subcol_wire = if matches!(target_kind, TableKind::Grouped | TableKind::Nested) { "raw_replicates" } else { "single" };
    let out = app
        .execute_command(
            "table.importCsv",
            json!({
                "title": title,
                "archetype": table_kind_wire_name(target_kind),
                "subcolumn_mode": subcol_wire,
                "replicates": 1,
                "csv": csv_text,
                "delimiter": "\t",
                "decimal_separator": ".",
                "has_header": true,
                "has_row_titles": !preview.row_labels.is_empty(),
            }),
        )
        .map_err(|e| e.to_string())?;
    let new_sid = out.payload.get("sheet_id").and_then(Value::as_str);
    if !state.parse_asterisk_exclusions
        && let Some(sid) = new_sid
    {
        restore_unexcluded_asterisk_cells(app, sid, &preview.rows, 0, 0, 0, target_kind == TableKind::Xy)?;
    }
    maybe_append_provenance_to_info(app, state, new_sid);
    Ok(out.payload)
}

fn restore_unexcluded_asterisk_cells(
    app: &mut UiApp,
    sheet_id: &str,
    rows: &[Vec<Cell>],
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    first_col_is_x: bool,
) -> Result<(), String> {
    let mut cell_edits = Vec::new();
    let mut x_edits = Vec::new();
    for (r_off, row) in rows.iter().enumerate() {
        for (c_off, cell) in row.iter().enumerate() {
            if !cell.excluded
                && let curvebook_doc::CellValue::Text(s) = &cell.value
                && (s.starts_with('*') || s.ends_with('*'))
            {
                let target_row = start_row + r_off;
                if first_col_is_x && c_off == 0 {
                    x_edits.push(json!({
                        "row": target_row,
                        "subcol": 0,
                        "value": s,
                        "excluded": false,
                        "parse_asterisk_exclusion": false,
                    }));
                } else {
                    let target_col = if first_col_is_x { start_col + c_off - 1 } else { start_col + c_off };
                    cell_edits.push(json!({
                        "row": target_row,
                        "col": target_col,
                        "subcol": start_subcol,
                        "value": s,
                        "excluded": false,
                        "parse_asterisk_exclusion": false,
                    }));
                }
            }
        }
    }
    if !x_edits.is_empty() {
        app.execute_command("table.setXCells", json!({ "sheet_id": sheet_id, "edits": x_edits })).map_err(|e| e.to_string())?;
    }
    if !cell_edits.is_empty() {
        app.execute_command("table.setCells", json!({ "sheet_id": sheet_id, "edits": cell_edits })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn resolve_existing_target_sheet(app: &UiApp, state: &ImportPasteViewState) -> Result<SheetId, String> {
    if let Some(raw) = &state.target_sheet_id {
        let id = SheetId::try_new(raw).map_err(|e| e.to_string())?;
        if app.document().data_sheet(&id).is_some() {
            return Ok(id);
        }
    }
    if let Some(active) = app.document().active_sheet_id.as_ref().filter(|id| app.document().data_sheet(id).is_some()) {
        return Ok(active.clone());
    }
    app.document().data_sheets.first().map(|d| d.id.clone()).ok_or_else(|| String::from("no target data sheet exists"))
}

fn maybe_append_provenance_to_info(app: &mut UiApp, state: &ImportPasteViewState, target_sheet_id: Option<&str>) {
    if !state.record_in_info_sheet || state.provenance_notes.trim().is_empty() {
        return;
    }
    let info_id = target_sheet_id
        .and_then(|sid| app.document().data_sheet(&SheetId::new(sid)).and_then(|d| d.info_sheet_id.clone()))
        .or_else(|| app.document().info_sheets.first().map(|i| i.id.clone()));
    let Some(iid) = info_id else {
        return;
    };
    let line = format!("- [Import {}] {}", state.link_id.trim(), state.provenance_notes.trim());
    let _ = app.execute_command("info.setNote", json!({ "sheet_id": iid.as_str(), "text": line, "append": true }));
}

fn table_kind_wire_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Column => "Column",
        TableKind::Grouped => "Grouped",
        TableKind::Xy => "Xy",
        TableKind::MultipleVariables => "MultipleVariables",
        TableKind::Contingency => "Contingency",
        _ => "Column",
    }
}

fn serialize_preview_as_tsv(preview: &ImportPreviewGrid, include_headers: bool) -> String {
    let mut lines = Vec::new();
    if include_headers && !preview.headers.is_empty() {
        lines.push(preview.headers.join("\t"));
    }
    for row in &preview.rows {
        let toks: Vec<String> = row.iter().map(format_linked_paste_tsv_cell).collect();
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}

fn serialize_preview_as_csv_with_headers(preview: &ImportPreviewGrid) -> String {
    let mut lines = Vec::new();
    let has_rows_col = !preview.row_labels.is_empty();
    if !preview.headers.is_empty() {
        let mut hdr_toks = Vec::with_capacity(preview.headers.len() + usize::from(has_rows_col));
        if has_rows_col {
            hdr_toks.push(String::from("Row"));
        }
        hdr_toks.extend(preview.headers.iter().cloned());
        lines.push(hdr_toks.join("\t"));
    } else if let Some(first) = preview.rows.first() {
        let mut hdr_toks = Vec::with_capacity(first.len() + usize::from(has_rows_col));
        if has_rows_col {
            hdr_toks.push(String::from("Row"));
        }
        hdr_toks.extend((0..first.len()).map(curvebook_doc::default_column_label));
        lines.push(hdr_toks.join("\t"));
    }
    for (r_idx, row) in preview.rows.iter().enumerate() {
        let mut toks = Vec::new();
        if has_rows_col {
            toks.push(preview.row_labels.get(r_idx).cloned().unwrap_or_default());
        }
        toks.extend(row.iter().map(format_linked_paste_tsv_cell));
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}

// ============================================================================
// Live Parsed Preview & Reshape Engine (shared across Source/View/Filter/Placement)
// ============================================================================

struct ImportPreviewGrid {
    headers: Vec<String>,
    row_labels: Vec<String>,
    rows: Vec<Vec<Cell>>,
    validation_error: Option<String>,
}

fn compute_import_preview(app: &UiApp, state: &ImportPasteViewState) -> ImportPreviewGrid {
    let st = &app.grid.import_reshape;
    let delim = st.tidy_delimiter;
    let dec = st.decimal_separator;

    if state.source_origin == "reshape" && matches!(st.mode, ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384) {
        return compute_plate_preview(app, state);
    }

    if state.source_origin == "internal_sheet" {
        return compute_internal_sheet_preview(app, state);
    }

    let raw = effective_source_text(app, state);
    let clean = normalize_text_for_encoding(&raw, &state.encoding);
    if clean.trim().is_empty() {
        return ImportPreviewGrid {
            headers: Vec::new(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            validation_error: Some(String::from("Enter or paste CSV/TSV data in Source tab.")),
        };
    }

    let opts = CsvOptions {
        delimiter: delim,
        quote_char: '"',
        decimal_separator: dec,
        thousands_separator: None,
        header_rows: usize::from(state.has_header_row),
        has_row_titles: state.has_row_titles,
        parse_asterisk_exclusions: state.parse_asterisk_exclusions,
        missing_sentinels: Vec::new(),
    };
    let mut records = match parse_delimited_records(&clean, &opts) {
        Ok(r) => r,
        Err(err) => {
            return ImportPreviewGrid { headers: Vec::new(), row_labels: Vec::new(), rows: Vec::new(), validation_error: Some(err.to_string()) };
        }
    };
    records.retain(|row| row.iter().any(|c| !c.trim().is_empty()) || !state.skip_contiguous_blanks);
    if records.is_empty() {
        return ImportPreviewGrid {
            headers: Vec::new(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            validation_error: Some(String::from("Parsed 0 non-empty rows.")),
        };
    }

    if state.source_origin == "reshape" && st.mode == ImportReshapeMode::TidyLongPivot {
        return compute_tidy_pivot_preview(app, state, records);
    }

    let mut headers = if state.has_header_row && !records.is_empty() { records.remove(0) } else { Vec::new() };
    let mut row_labels = Vec::new();
    let mut grid: Vec<Vec<Cell>> = Vec::with_capacity(records.len());
    for row in records {
        let mut iter = row.into_iter();
        if state.has_row_titles {
            row_labels.push(iter.next().unwrap_or_default());
        }
        let cells: Vec<Cell> = iter.map(|tok| parse_cell_token_preview(&tok, dec, state.parse_asterisk_exclusions)).collect();
        grid.push(cells);
    }
    if state.has_row_titles && !headers.is_empty() {
        let _ = headers.remove(0);
    }

    let (headers, row_labels, rows, _excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
    ImportPreviewGrid { headers, row_labels, rows, validation_error: None }
}

fn compute_plate_preview(app: &UiApp, state: &ImportPasteViewState) -> ImportPreviewGrid {
    let st = &app.grid.import_reshape;
    let geom = if st.mode == ImportReshapeMode::Plate384 { PlateGeometry::Wells384 } else { PlateGeometry::Wells96 };
    let expected = geom.well_count();
    let values = effective_plate_values(app);
    let roles_ok = st.plate_roles.len() == expected;
    let vals_ok = values.len() == expected;

    if vals_ok && roles_ok {
        let parsed_roles: Result<Vec<PlateWellRole>, _> = st.plate_roles.iter().map(parse_ui_plate_role).collect();
        if let Ok(roles) = parsed_roles
            && let Ok(tpl) = PlateMapTemplate::from_wells(&st.title, geom, &st.x_title, roles)
        {
            let target = match st.target_kind {
                TableKind::Column => TargetArchetypeKind::Column,
                TableKind::Xy => TargetArchetypeKind::Xy,
                TableKind::MultipleVariables => TargetArchetypeKind::MultipleVariables,
                _ => TargetArchetypeKind::Grouped,
            };
            if let Ok(ds) = map_plate_to_datasheet(
                if st.title.trim().is_empty() { "Plate Data" } else { st.title.trim() },
                geom,
                &values,
                &tpl,
                target,
                st.subtract_blank_mean,
            ) {
                let (_headers, _row_labels, _rows, _excluded_count) = extract_datasheet_preview_grid(&ds, &app.locale_preset);
                let (headers, row_labels, rows, _excluded_count) =
                    apply_filter_and_placement_to_grid(_headers, _row_labels, _rows, state, &app.locale_preset);
                return ImportPreviewGrid { headers, row_labels, rows, validation_error: None };
            }
        }
    }

    // Fallback: display raw plate matrix (8x12 or 16x24) with helpful validation message.
    let cols = geom.cols();
    let rows_n = geom.rows();
    let headers: Vec<String> = (1..=cols).map(|c| format!("{c:02}")).collect();
    let mut row_labels = Vec::with_capacity(rows_n);
    let mut grid = Vec::with_capacity(rows_n);
    for r in 0..rows_n {
        let r_ch = char::from(b'A'.saturating_add(u8::try_from(r).unwrap_or(0)));
        row_labels.push(r_ch.to_string());
        let mut row_cells = Vec::with_capacity(cols);
        for c in 0..cols {
            let idx = r * cols + c;
            let is_excl = st.plate_roles.get(idx).and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("Excluded"));
            let cell = match values.get(idx).copied().flatten() {
                Some(v) => {
                    if is_excl {
                        Cell::excluded_number(v)
                    } else {
                        Cell::number(v)
                    }
                }
                None => Cell::empty(),
            };
            row_cells.push(cell);
        }
        grid.push(row_cells);
    }
    ImportPreviewGrid {
        headers,
        row_labels,
        rows: grid,
        validation_error: Some(format!("Need {expected} well values ({}) & roles ({})", values.len(), st.plate_roles.len())),
    }
}

fn parse_ui_plate_role(val: &Value) -> Result<PlateWellRole, String> {
    if let Some(tag) = val.as_str() {
        return match tag.trim().to_ascii_lowercase().as_str() {
            "blank" => Ok(PlateWellRole::Blank),
            "positivecontrol" | "positive_control" => Ok(PlateWellRole::PositiveControl),
            "excluded" | "unused" => Ok(PlateWellRole::Excluded),
            _ => Err(format!("unknown plate role `{tag}`")),
        };
    }
    if let Ok(role) = serde_json::from_value::<PlateWellRole>(val.clone()) {
        return Ok(role);
    }
    let group_name = val.get("group_name").and_then(Value::as_str).ok_or_else(|| String::from("missing group_name"))?;
    let dilution_x = val.get("dilution_x").and_then(Value::as_f64).ok_or_else(|| String::from("missing dilution_x"))?;
    let replicate = val.get("replicate").and_then(Value::as_u64).and_then(|u| usize::try_from(u).ok()).unwrap_or(1);
    Ok(PlateWellRole::Treatment { group_name: group_name.to_owned(), dilution_x, replicate })
}

const TIDY_LITERAL_STAR_PREFIX: &str = "\u{E000}cb_star_";
const TIDY_LITERAL_STAR_SUFFIX: &str = "\u{E000}";

fn sanitize_tidy_asterisk_tokens(rows: &mut [Vec<String>], value_col: usize, x_col: Option<usize>) -> Vec<String> {
    let mut originals = Vec::new();
    for row in rows {
        for col_idx in std::iter::once(value_col).chain(x_col) {
            if let Some(tok) = row.get_mut(col_idx) {
                let trimmed = tok.trim();
                if (trimmed.starts_with('*') || trimmed.ends_with('*')) && trimmed.len() > 1 {
                    let marker = format!("{TIDY_LITERAL_STAR_PREFIX}{}{TIDY_LITERAL_STAR_SUFFIX}", originals.len());
                    originals.push(trimmed.to_owned());
                    *tok = marker;
                }
            }
        }
    }
    originals
}

fn restore_tidy_asterisk_cells(grid: &mut [Vec<Cell>], originals: &[String]) {
    if originals.is_empty() {
        return;
    }
    for row in grid {
        for cell in row {
            if let curvebook_doc::CellValue::Text(s) = &cell.value
                && let Some(inner) = s.strip_prefix(TIDY_LITERAL_STAR_PREFIX).and_then(|rest| rest.strip_suffix(TIDY_LITERAL_STAR_SUFFIX))
                && let Ok(idx) = inner.parse::<usize>()
                && let Some(orig) = originals.get(idx)
            {
                cell.value = curvebook_doc::CellValue::Text(orig.clone());
            }
        }
    }
}

fn compute_tidy_pivot_preview(app: &UiApp, state: &ImportPasteViewState, mut records: Vec<Vec<String>>) -> ImportPreviewGrid {
    let st = &app.grid.import_reshape;
    let headers = if state.has_header_row {
        if records.len() < 2 {
            return ImportPreviewGrid {
                headers: records.first().cloned().unwrap_or_default(),
                row_labels: Vec::new(),
                rows: Vec::new(),
                validation_error: Some(String::from("Tidy CSV needs a header row and at least 1 data row.")),
            };
        }
        records.remove(0)
    } else {
        let col_count = records.iter().map(Vec::len).max().unwrap_or(0);
        if records.is_empty() || col_count == 0 {
            return ImportPreviewGrid {
                headers: Vec::new(),
                row_labels: Vec::new(),
                rows: Vec::new(),
                validation_error: Some(String::from("Tidy CSV needs at least 1 data row.")),
            };
        }
        (0..col_count).map(curvebook_doc::default_column_label).collect()
    };
    let find_col = |name: &str| -> Option<usize> {
        let needle = name.trim().to_ascii_lowercase();
        headers.iter().position(|h| h.trim().to_ascii_lowercase() == needle)
    };
    let group_name = effective_text(app, WID_GROUP_COL, &st.group_col);
    let value_name = effective_text(app, WID_VALUE_COL, &st.value_col);
    let x_name = effective_text(app, WID_X_COL, st.x_col.as_deref().unwrap_or(""));
    let rep_name = effective_text(app, WID_REPLICATE_COL, st.replicate_col.as_deref().unwrap_or(""));
    let excl_name = effective_text(app, WID_EXCLUDED_COL, st.excluded_col.as_deref().unwrap_or(""));

    let (Some(group_col), Some(value_col)) = (find_col(&group_name), find_col(&value_name)) else {
        return ImportPreviewGrid {
            headers,
            row_labels: Vec::new(),
            rows: Vec::new(),
            validation_error: Some(format!("Column `{group_name}` or `{value_name}` not found in headers.")),
        };
    };

    let mapping = curvebook_transform::TidyColumnMapping {
        x_col: (!x_name.trim().is_empty()).then(|| find_col(&x_name)).flatten(),
        group_col,
        replicate_col: (!rep_name.trim().is_empty()).then(|| find_col(&rep_name)).flatten(),
        value_col,
        excluded_col: (!excl_name.trim().is_empty()).then(|| find_col(&excl_name)).flatten(),
    };
    let target = match st.target_kind {
        TableKind::Column => TargetArchetypeKind::Column,
        TableKind::Xy => TargetArchetypeKind::Xy,
        TableKind::MultipleVariables => TargetArchetypeKind::MultipleVariables,
        _ => TargetArchetypeKind::Grouped,
    };
    let mut sliced_rows = slice_raw_records(records, state);
    let asterisk_originals =
        if state.parse_asterisk_exclusions { Vec::new() } else { sanitize_tidy_asterisk_tokens(&mut sliced_rows, mapping.value_col, mapping.x_col) };
    match pivot_tidy_long_to_datasheet(
        if st.title.trim().is_empty() { "Tidy Import" } else { st.title.trim() },
        &headers,
        &sliced_rows,
        &mapping,
        target,
        st.decimal_separator,
    ) {
        Ok(ds) => {
            let (hdrs, r_labels, mut grid, _) = extract_datasheet_preview_grid(&ds, &app.locale_preset);
            restore_tidy_asterisk_cells(&mut grid, &asterisk_originals);
            let mut post_state = state.clone();
            // Slicing was already applied to raw tidy rows before pivoting; only apply placement/markers below.
            post_state.row_start.clear();
            post_state.row_end.clear();
            post_state.col_start.clear();
            post_state.col_end.clear();
            post_state.decimate_every_k = 1;
            post_state.unstack_enabled = false;
            let (hdrs, r_labels, rows, _excl) = apply_filter_and_placement_to_grid(hdrs, r_labels, grid, &post_state, &app.locale_preset);
            ImportPreviewGrid { headers: hdrs, row_labels: r_labels, rows, validation_error: None }
        }
        Err(err) => ImportPreviewGrid { headers, row_labels: Vec::new(), rows: Vec::new(), validation_error: Some(err.to_string()) },
    }
}

fn slice_raw_records(records: Vec<Vec<String>>, state: &ImportPasteViewState) -> Vec<Vec<String>> {
    let filter = build_filter_spec(state);
    let mut out = records;
    if let Some((r0, r1)) = filter.row_window {
        if r0 < out.len() {
            out = out[r0..=r1.min(out.len() - 1)].to_vec();
        } else {
            out.clear();
        }
    }
    if let Some(k) = filter.decimate_every_k
        && k > 1
    {
        out = out.into_iter().step_by(k).collect();
    }
    out
}

fn compute_internal_sheet_preview(app: &UiApp, state: &ImportPasteViewState) -> ImportPreviewGrid {
    let Some(sid_str) = state.internal_source_sheet_id.as_deref() else {
        return ImportPreviewGrid {
            headers: Vec::new(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            validation_error: Some(String::from("Select an internal DataSheet or ResultSheet in Source tab.")),
        };
    };
    let sid = SheetId::new(sid_str);
    let doc = app.document();
    if let Some(ds) = doc.data_sheet(&sid) {
        let (headers, row_labels, grid, _) = extract_datasheet_preview_grid(ds, &app.locale_preset);
        let (headers, row_labels, rows, _excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
        return ImportPreviewGrid { headers, row_labels, rows, validation_error: None };
    }
    if let Some(rs) = doc.result_sheet(&sid) {
        let (headers, row_labels, grid) = extract_resultsheet_preview_grid(rs);
        let (headers, row_labels, rows, _excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
        return ImportPreviewGrid { headers, row_labels, rows, validation_error: None };
    }
    ImportPreviewGrid {
        headers: Vec::new(),
        row_labels: Vec::new(),
        rows: Vec::new(),
        validation_error: Some(format!("Sheet `{sid_str}` not found.")),
    }
}

fn apply_filter_and_placement_to_grid(
    mut headers: Vec<String>,
    mut row_labels: Vec<String>,
    mut grid: Vec<Vec<Cell>>,
    state: &ImportPasteViewState,
    preset: &LocalePreset,
) -> (Vec<String>, Vec<String>, Vec<Vec<Cell>>, usize) {
    let filter = build_filter_spec(state);
    if let Some((r0, r1)) = filter.row_window {
        if r0 < grid.len() {
            let end = r1.min(grid.len() - 1);
            grid = grid[r0..=end].to_vec();
            if r0 < row_labels.len() {
                row_labels = row_labels[r0..=end.min(row_labels.len() - 1)].to_vec();
            } else {
                row_labels.clear();
            }
        } else {
            grid.clear();
            row_labels.clear();
        }
    }
    if let Some((c0, c1)) = filter.col_window {
        for row in &mut grid {
            if c0 < row.len() {
                *row = row[c0..=c1.min(row.len() - 1)].to_vec();
            } else {
                row.clear();
            }
        }
        if c0 < headers.len() {
            headers = headers[c0..=c1.min(headers.len() - 1)].to_vec();
        } else {
            headers.clear();
        }
    }
    if let Some(k) = filter.decimate_every_k
        && k > 1
    {
        grid = grid.into_iter().step_by(k).collect();
        if !row_labels.is_empty() {
            row_labels = row_labels.into_iter().step_by(k).collect();
        }
    }
    if let Some(gcol) = filter.unstack_group_column
        && !grid.is_empty()
    {
        let (unstacked, new_headers) = unstack_preview_grid(&grid, gcol, preset);
        grid = unstacked;
        headers = new_headers;
        row_labels.clear();
    }
    if state.skip_contiguous_blanks {
        let mut prev_blank = false;
        let mut kept = Vec::with_capacity(grid.len());
        let mut kept_labels = Vec::with_capacity(row_labels.len());
        for (idx, row) in grid.into_iter().enumerate() {
            let blank = row.iter().all(Cell::is_empty);
            if blank && prev_blank {
                continue;
            }
            prev_blank = blank;
            kept.push(row);
            if let Some(lbl) = row_labels.get(idx) {
                kept_labels.push(lbl.clone());
            }
        }
        grid = kept;
        row_labels = kept_labels;
    }

    // Apply exclusion marker pattern & optional numeric predicate.
    let pat = state.filter_pattern.trim().to_ascii_lowercase();
    let pred = parse_simple_row_predicate(&state.exclude_predicate);
    for row in &mut grid {
        let row_matches_pred = pred.as_ref().is_some_and(|p| p.matches_row(row));
        for cell in row.iter_mut() {
            if !state.parse_asterisk_exclusions {
                // If user disabled asterisk parsing in source mode, keep existing explicit flags only.
            }
            if row_matches_pred && !cell.is_empty() {
                cell.excluded = true;
            }
            if !pat.is_empty() {
                let disp = format_cell_display(cell, preset).to_ascii_lowercase();
                if disp.contains(&pat) {
                    cell.excluded = true;
                }
            }
        }
    }

    if state.transpose {
        grid = curvebook_doc::transpose_cell_grid(&grid);
        let old_headers = std::mem::take(&mut headers);
        let old_labels = std::mem::take(&mut row_labels);
        row_labels = old_headers;
        headers = old_labels;
    }

    if state.row_wrap_width > 0 {
        let flat: Vec<Cell> = grid.into_iter().flatten().collect();
        grid = flat.chunks(state.row_wrap_width).map(<[Cell]>::to_vec).collect();
        headers = (0..state.row_wrap_width).map(curvebook_doc::default_column_label).collect();
        row_labels.clear();
    }

    let max_cols = grid.iter().map(Vec::len).max().unwrap_or(0).max(headers.len());
    if headers.is_empty() && max_cols > 0 {
        headers = (0..max_cols).map(curvebook_doc::default_column_label).collect();
    }
    let excluded_count = grid.iter().flatten().filter(|c| c.excluded).count();
    (headers, row_labels, grid, excluded_count)
}

fn unstack_preview_grid(grid: &[Vec<Cell>], group_col: usize, preset: &LocalePreset) -> (Vec<Vec<Cell>>, Vec<String>) {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::BTreeMap<String, Vec<Cell>> = std::collections::BTreeMap::new();
    for row in grid {
        let Some(g_cell) = row.get(group_col) else {
            continue;
        };
        let key = format_cell_display(g_cell, preset).trim().to_owned();
        if key.is_empty() {
            continue;
        }
        let val_cell = row.iter().enumerate().find(|&(i, _)| i != group_col).map_or_else(Cell::empty, |(_, c)| c.clone());
        if !buckets.contains_key(&key) {
            order.push(key.clone());
        }
        buckets.entry(key).or_default().push(val_cell);
    }
    let max_rows = buckets.values().map(Vec::len).max().unwrap_or(0);
    let cols = order.len();
    let mut out = vec![vec![Cell::empty(); cols]; max_rows];
    for (c, k) in order.iter().enumerate() {
        if let Some(vals) = buckets.get(k) {
            for (r, cell) in vals.iter().enumerate() {
                out[r][c] = cell.clone();
            }
        }
    }
    (out, order)
}

struct SimpleRowPred {
    col_idx: Option<usize>,
    op: &'static str,
    threshold: f64,
}

impl SimpleRowPred {
    fn matches_row(&self, row: &[Cell]) -> bool {
        let vals: Vec<f64> = match self.col_idx {
            Some(c) => row.get(c).and_then(Cell::active_number).into_iter().collect(),
            None => row.iter().filter_map(Cell::active_number).collect(),
        };
        vals.into_iter().any(|v| match self.op {
            "<" => v < self.threshold,
            "<=" => v <= self.threshold,
            ">" => v > self.threshold,
            ">=" => v >= self.threshold,
            "==" => (v - self.threshold).abs() < 1e-12,
            _ => false,
        })
    }
}

fn parse_simple_row_predicate(raw: &str) -> Option<SimpleRowPred> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    for op in ["<=", ">=", "==", "<", ">"] {
        if let Some((lhs, rhs)) = s.split_once(op)
            && let Ok(threshold) = rhs.trim().parse::<f64>()
            && threshold.is_finite()
        {
            let l = lhs.trim();
            let col_idx = if l.is_empty() {
                None
            } else if l.len() == 1 && l.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                let c = l.chars().next()?.to_ascii_uppercase() as u8;
                Some(usize::from(c.saturating_sub(b'A')))
            } else {
                l.parse::<usize>().ok()
            };
            return Some(SimpleRowPred { col_idx, op, threshold });
        }
    }
    None
}

fn extract_datasheet_preview_grid(ds: &DataSheet, preset: &LocalePreset) -> (Vec<String>, Vec<String>, Vec<Vec<Cell>>, usize) {
    let row_count = ds.archetype.row_count().min(200);
    let mut headers = Vec::new();
    let mut grid = vec![Vec::new(); row_count];
    let mut row_labels = Vec::new();

    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            headers.push(if xy.x_title.trim().is_empty() { String::from("X") } else { xy.x_title.clone() });
            row_labels = xy.row_titles.iter().take(row_count).cloned().collect();
            for (r, row_slot) in grid.iter_mut().enumerate() {
                row_slot.push(xy.x_cells.get(r).and_then(|x| x.first()).cloned().unwrap_or_default());
                for dcol in &xy.data_sets {
                    let subs = xy.y_subcolumn_mode.subcolumn_count();
                    for s in 0..subs {
                        row_slot.push(dcol.cells.get(r).and_then(|rc| rc.get(s)).cloned().unwrap_or_default());
                    }
                }
            }
            for dcol in &xy.data_sets {
                let subs = xy.y_subcolumn_mode.subcolumn_count();
                for s in 0..subs {
                    if subs == 1 {
                        headers.push(dcol.title.clone());
                    } else {
                        headers.push(format!("{}:{}", dcol.title, s + 1));
                    }
                }
            }
        }
        TableArchetype::Column(col) => {
            row_labels = col.row_titles.iter().take(row_count).cloned().collect();
            for dcol in &col.data_sets {
                let subs = col.subcolumn_mode.subcolumn_count();
                for s in 0..subs {
                    headers.push(if subs == 1 { dcol.title.clone() } else { format!("{}:{}", dcol.title, s + 1) });
                }
            }
            for (r, row_slot) in grid.iter_mut().enumerate() {
                for dcol in &col.data_sets {
                    let subs = col.subcolumn_mode.subcolumn_count();
                    for s in 0..subs {
                        row_slot.push(dcol.cells.get(r).and_then(|rc| rc.get(s)).cloned().unwrap_or_default());
                    }
                }
            }
        }
        TableArchetype::Grouped(grp) => {
            row_labels = grp.row_titles.iter().take(row_count).cloned().collect();
            for dcol in &grp.data_sets {
                let subs = grp.subcolumn_mode.subcolumn_count();
                for s in 0..subs {
                    headers.push(if subs == 1 { dcol.title.clone() } else { format!("{}:{}", dcol.title, s + 1) });
                }
            }
            for (r, row_slot) in grid.iter_mut().enumerate() {
                for dcol in &grp.data_sets {
                    let subs = grp.subcolumn_mode.subcolumn_count();
                    for s in 0..subs {
                        row_slot.push(dcol.cells.get(r).and_then(|rc| rc.get(s)).cloned().unwrap_or_default());
                    }
                }
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            row_labels = mv.row_titles.iter().take(row_count).cloned().collect();
            for v in &mv.variables {
                headers.push(v.title.clone());
            }
            for (r, row_slot) in grid.iter_mut().enumerate() {
                for v in &mv.variables {
                    row_slot.push(v.cells.get(r).cloned().unwrap_or_default());
                }
            }
        }
        TableArchetype::Contingency(ct) => {
            row_labels = ct.row_titles.iter().take(row_count).cloned().collect();
            for dcol in &ct.data_sets {
                headers.push(dcol.title.clone());
            }
            for (r, row_slot) in grid.iter_mut().enumerate() {
                for dcol in &ct.data_sets {
                    row_slot.push(dcol.cells.get(r).and_then(|rc| rc.first()).cloned().unwrap_or_default());
                }
            }
        }
        _ => {
            let full = ds.extract_cell_grid(SheetCellRange::new(CellCoord::default(), CellCoord::new(row_count, 32, 8)));
            grid = full;
            let cols = grid.first().map_or(0, Vec::len);
            headers =
                (0..cols).map(|c| crate::dialogs_canvas::data_column_title(ds, c).map_or_else(|| format!("C{}", c + 1), str::to_owned)).collect();
        }
    }
    let _ = preset;
    let excl = grid.iter().flatten().filter(|c| c.excluded).count();
    (headers, row_labels, grid, excl)
}

fn extract_resultsheet_preview_grid(rs: &curvebook_doc::ResultSheet) -> (Vec<String>, Vec<String>, Vec<Vec<Cell>>) {
    let Some(tab) = rs.tabs.first() else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let headers = tab.column_headers.clone();
    let row_labels = tab.row_labels.clone();
    let rows = tab
        .rows
        .iter()
        .take(200)
        .map(|r| {
            r.iter()
                .map(|rc| match rc {
                    ResultCell::Empty => Cell::empty(),
                    ResultCell::Number { value, .. } => Cell::number(*value),
                    ResultCell::PValue { raw, .. } => Cell::number(*raw),
                    ResultCell::Text(s) => Cell::text(s.clone()),
                    ResultCell::Boolean(b) => Cell::number(if *b { 1.0 } else { 0.0 }),
                    ResultCell::ConfidenceInterval { lower, upper } => {
                        let lo = match lower {
                            CiBound::Finite(v) => format!("{v:.3}"),
                            CiBound::Open => String::from("-inf"),
                        };
                        let hi = match upper {
                            CiBound::Finite(v) => format!("{v:.3}"),
                            CiBound::Open => String::from("+inf"),
                        };
                        Cell::text(format!("[{lo}, {hi}]"))
                    }
                })
                .collect()
        })
        .collect();
    (headers, row_labels, rows)
}
