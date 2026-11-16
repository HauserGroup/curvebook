//! Import, Paste Special (5-tab `Source`, `View`, `Filter`, `Placement`, `Notes`),
//! and interactive `Convert Table Archetype…` dialog view builders (`R10.12`, `R10.4`).

use curvebook_doc::{
    Cell, CellCoord, CellValue, CiBound, DataSheet, ImportFilterSpec, ImportPlacementSpec, InternalLinkKind, LinkBorderColor, PlacementMode,
    ResultCell, SheetCellRange, SheetId, TableArchetype, TableKind,
};
use curvebook_format::{CsvOptions, parse_delimited_records};
use curvebook_palette::Rgba8;
use curvebook_transform::{
    ConversionPreview, PlateGeometry, PlateMapTemplate, PlateWellRole, TargetArchetypeKind, convert_datasheet_archetype, map_plate_to_datasheet,
    pivot_tidy_long_to_datasheet,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::grid_view::{ImportOriginKind, ImportReshapeMode, format_cell_display, parse_localized_number_input};
use crate::i18n::LocalePreset;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldEditState, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab,
    emit_text_field, emit_toggle, paint_text_fitted,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

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
    const ALL: [Self; 5] = [Self::Source, Self::View, Self::Filter, Self::Placement, Self::Notes];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::View => "view",
            Self::Filter => "filter",
            Self::Placement => "placement",
            Self::Notes => "notes",
        }
    }

    const fn widget_id(self) -> &'static str {
        match self {
            Self::Source => "import_paste.tab.source",
            Self::View => "import_paste.tab.view",
            Self::Filter => "import_paste.tab.filter",
            Self::Placement => "import_paste.tab.placement",
            Self::Notes => "import_paste.tab.notes",
        }
    }

    const fn i18n_key(self) -> &'static str {
        match self {
            Self::Source => "import_paste.tab.source",
            Self::View => "import_paste.tab.view",
            Self::Filter => "import_paste.tab.filter",
            Self::Placement => "import_paste.tab.placement",
            Self::Notes => "import_paste.tab.notes",
        }
    }

    const fn fallback_label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::View => "View",
            Self::Filter => "Filter",
            Self::Placement => "Placement",
            Self::Notes => "Notes",
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

impl ImportPasteViewState {
    fn normalized_tab(&self) -> ImportPasteTab {
        ImportPasteTab::from_str(&self.active_tab)
    }
}

/// Lookup helper resolving `w5_import_paste` entries once wired into `i18n.rs`,
/// falling back cleanly to `fallback` beforehand without triggering literal `tr()` lint checks.
fn tr_or_default<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
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
    // Bump shard revision so `LayoutTree` fingerprint invalidates even when underlying `grid`/`convert_archetype` state changes.
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

fn execute_import_paste_commit(app: &mut UiApp, state: &mut ImportPasteViewState) -> Result<Value, String> {
    if state.source_origin == "reshape" {
        if matches!(app.grid.import_reshape.mode, ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384) {
            let intent = app.grid.import_reshape.build_import_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
            return Ok(out.payload);
        }
        // TidyLongPivot: honor extra slicing/row-wrap if enabled, else standard importTidyLong intent.
        if state.row_wrap_width == 0
            && state.row_start.is_empty()
            && state.row_end.is_empty()
            && state.col_start.is_empty()
            && state.col_end.is_empty()
            && state.decimate_every_k <= 1
            && state.filter_pattern.trim().is_empty()
            && state.exclude_predicate.trim().is_empty()
            && !state.transpose
            && state.target_mode == "new_sheet"
        {
            let intent = app.grid.import_reshape.build_import_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
            return Ok(out.payload);
        }
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
    let out = app
        .execute_command(
            "table.importCsv",
            json!({
                "title": title,
                "archetype": table_kind_wire_name(app.grid.import_reshape.target_kind),
                "csv": csv_text,
                "delimiter": "\t",
                "decimal_separator": ".",
                "has_header": true,
                "has_row_titles": state.has_row_titles && !preview.row_labels.is_empty(),
            }),
        )
        .map_err(|e| e.to_string())?;
    maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
    Ok(out.payload)
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
        let toks: Vec<String> = row.iter().map(format_cell_tsv).collect();
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}

fn serialize_preview_as_csv_with_headers(preview: &ImportPreviewGrid) -> String {
    let mut lines = Vec::new();
    if !preview.headers.is_empty() {
        lines.push(preview.headers.join("\t"));
    } else if let Some(first) = preview.rows.first() {
        let hdrs: Vec<String> = (0..first.len()).map(curvebook_doc::default_column_label).collect();
        lines.push(hdrs.join("\t"));
    }
    for (r_idx, row) in preview.rows.iter().enumerate() {
        let mut toks = Vec::new();
        if !preview.row_labels.is_empty() {
            toks.push(preview.row_labels.get(r_idx).cloned().unwrap_or_default());
        }
        toks.extend(row.iter().map(format_cell_tsv));
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}

fn format_cell_tsv(cell: &Cell) -> String {
    let base = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) => v.to_string(),
        CellValue::Text(s) => s.replace(['\t', '\n', '\r'], " "),
    };
    if cell.excluded && !base.is_empty() { format!("{base}*") } else { base }
}

// ============================================================================
// Live Parsed Preview & Reshape Engine (shared across Source/View/Filter/Placement)
// ============================================================================

struct ImportPreviewGrid {
    headers: Vec<String>,
    row_labels: Vec<String>,
    rows: Vec<Vec<Cell>>,
    excluded_count: usize,
    provenance_border: Option<LinkBorderColor>,
    mode_summary: String,
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
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("Empty source"),
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
            return ImportPreviewGrid {
                headers: Vec::new(),
                row_labels: Vec::new(),
                rows: Vec::new(),
                excluded_count: 0,
                provenance_border: None,
                mode_summary: String::from("Parse error"),
                validation_error: Some(err.to_string()),
            };
        }
    };
    records.retain(|row| row.iter().any(|c| !c.trim().is_empty()) || !state.skip_contiguous_blanks);
    if records.is_empty() {
        return ImportPreviewGrid {
            headers: Vec::new(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("No records"),
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

    let (headers, row_labels, rows, excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
    let border = (state.paste_as_live_link && state.source_origin == "external_csv").then_some(LinkBorderColor::Charcoal);
    ImportPreviewGrid {
        headers,
        row_labels,
        rows,
        excluded_count,
        provenance_border: border,
        mode_summary: if state.source_origin == "external_csv" { String::from("External Delimited") } else { String::from("Clipboard / Delimited") },
        validation_error: None,
    }
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
                let final_excl = rows.iter().flatten().filter(|c| c.excluded).count();
                return ImportPreviewGrid {
                    headers,
                    row_labels,
                    rows,
                    excluded_count: final_excl,
                    provenance_border: None,
                    mode_summary: format!("{expected}-Well Plate Reshape (Blank sub={})", st.subtract_blank_mean),
                    validation_error: None,
                };
            }
        }
    }

    // Fallback: display raw plate matrix (8x12 or 16x24) with helpful validation message.
    let cols = geom.cols();
    let rows_n = geom.rows();
    let headers: Vec<String> = (1..=cols).map(|c| format!("{c:02}")).collect();
    let mut row_labels = Vec::with_capacity(rows_n);
    let mut grid = Vec::with_capacity(rows_n);
    let mut excluded_count = 0usize;
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
                        excluded_count += 1;
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
        excluded_count,
        provenance_border: None,
        mode_summary: format!("{expected}-Well Raw Plate"),
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

fn compute_tidy_pivot_preview(app: &UiApp, state: &ImportPasteViewState, mut records: Vec<Vec<String>>) -> ImportPreviewGrid {
    let st = &app.grid.import_reshape;
    if records.len() < 2 {
        return ImportPreviewGrid {
            headers: records.first().cloned().unwrap_or_default(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("Tidy Long Pivot"),
            validation_error: Some(String::from("Tidy CSV needs a header row and at least 1 data row.")),
        };
    }
    let headers = records.remove(0);
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
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("Tidy Long Pivot"),
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
    let sliced_rows = slice_raw_records(records, state);
    match pivot_tidy_long_to_datasheet(
        if st.title.trim().is_empty() { "Tidy Import" } else { st.title.trim() },
        &headers,
        &sliced_rows,
        &mapping,
        target,
        st.decimal_separator,
    ) {
        Ok(ds) => {
            let (hdrs, r_labels, grid, _) = extract_datasheet_preview_grid(&ds, &app.locale_preset);
            let mut post_state = state.clone();
            // Slicing was already applied to raw tidy rows before pivoting; only apply placement/markers below.
            post_state.row_start.clear();
            post_state.row_end.clear();
            post_state.col_start.clear();
            post_state.col_end.clear();
            post_state.decimate_every_k = 1;
            post_state.unstack_enabled = false;
            let (hdrs, r_labels, rows, excl) = apply_filter_and_placement_to_grid(hdrs, r_labels, grid, &post_state, &app.locale_preset);
            ImportPreviewGrid {
                headers: hdrs,
                row_labels: r_labels,
                rows,
                excluded_count: excl,
                provenance_border: None,
                mode_summary: format!("Tidy Long Pivot -> {}", st.target_kind.label()),
                validation_error: None,
            }
        }
        Err(err) => ImportPreviewGrid {
            headers,
            row_labels: Vec::new(),
            rows: Vec::new(),
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("Tidy Long Pivot"),
            validation_error: Some(err.to_string()),
        },
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
            excluded_count: 0,
            provenance_border: None,
            mode_summary: String::from("Internal Sheet"),
            validation_error: Some(String::from("Select an internal DataSheet or ResultSheet in Source tab.")),
        };
    };
    let sid = SheetId::new(sid_str);
    let doc = app.document();
    if let Some(ds) = doc.data_sheet(&sid) {
        let (headers, row_labels, grid, _) = extract_datasheet_preview_grid(ds, &app.locale_preset);
        let (headers, row_labels, rows, excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
        let border = state.paste_as_live_link.then_some(LinkBorderColor::Blue);
        return ImportPreviewGrid {
            headers,
            row_labels,
            rows,
            excluded_count,
            provenance_border: border,
            mode_summary: format!("Internal DataSheet `{}`", ds.title),
            validation_error: None,
        };
    }
    if let Some(rs) = doc.result_sheet(&sid) {
        let (headers, row_labels, grid) = extract_resultsheet_preview_grid(rs);
        let (headers, row_labels, rows, excluded_count) = apply_filter_and_placement_to_grid(headers, row_labels, grid, state, &app.locale_preset);
        let kind = if rs.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet };
        let border = state.paste_as_live_link.then_some(match kind {
            InternalLinkKind::AnalyzableResultSheet => LinkBorderColor::Green,
            _ => LinkBorderColor::Red,
        });
        return ImportPreviewGrid {
            headers,
            row_labels,
            rows,
            excluded_count,
            provenance_border: border,
            mode_summary: format!("Internal ResultSheet `{}`", rs.title()),
            validation_error: None,
        };
    }
    ImportPreviewGrid {
        headers: Vec::new(),
        row_labels: Vec::new(),
        rows: Vec::new(),
        excluded_count: 0,
        provenance_border: None,
        mode_summary: String::from("Internal Sheet"),
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

// ============================================================================
// Modal Entry Point & Tab Builders (`ActiveDialog::ImportReshape`)
// ============================================================================

/// Build the `ImportReshape` / `ConvertArchetype` modal body inside `body_rect`.
pub fn build_import_or_convert_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    match &builder.app.canvas_dialogs.active_dialog {
        ActiveDialog::ImportReshape => build_import_paste_special_body(builder, parent, body_rect),
        ActiveDialog::ConvertArchetype { sheet_id } => {
            let sid = sheet_id.clone();
            build_convert_archetype_body(builder, parent, body_rect, &sid);
        }
        _ => {}
    }
}

fn scaled_px(builder: &LayoutBuilder<'_>, base: f64, min: u32, max: u32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let v = (base * builder.scale()).round() as u32;
    v.clamp(min, max)
}

fn build_import_paste_special_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let state = &builder.app.view_shards.import_paste;
    let active_tab = state.normalized_tab();
    let locale = builder.tree.locale.clone();

    let tab_h = scaled_px(builder, 24.0, 20, 44);
    let tab_count = ImportPasteTab::ALL.len() as u32;
    let gap = 4u32;
    let tab_w = (body_rect.width.saturating_sub(gap * (tab_count.saturating_sub(1))) / tab_count).max(48);

    for (idx, tab) in ImportPasteTab::ALL.iter().enumerate() {
        let tx = body_rect.x + i32::try_from(idx as u32 * (tab_w + gap)).unwrap_or(0);
        let lbl = tr_or_default(&locale, tab.i18n_key(), tab.fallback_label());
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: tab.widget_id(),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(tx, body_rect.y, tab_w, tab_h),
                label: lbl,
                keyboard_path: &format!("import_paste.tab.{}", tab.as_str()),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + idx as u32),
                primary: false,
                selected: active_tab == *tab,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: format!("tab:{}", tab.as_str()) },
            },
        );
    }

    let content_y = body_rect.y + i32::try_from(tab_h + 6).unwrap_or(30);
    let content_h = u32::try_from((body_rect.bottom() - content_y).max(0)).unwrap_or(0);
    if content_h < 24 || body_rect.width < 80 {
        return;
    }
    let content_rect = LayoutRectPx::new(body_rect.x, content_y, body_rect.width, content_h);

    match active_tab {
        ImportPasteTab::Source => build_source_tab(builder, &parent, content_rect, state),
        ImportPasteTab::View => build_view_tab(builder, &parent, content_rect, state),
        ImportPasteTab::Filter => build_filter_tab(builder, &parent, content_rect, state),
        ImportPasteTab::Placement => build_placement_tab(builder, &parent, content_rect, state),
        ImportPasteTab::Notes => build_notes_tab(builder, &parent, content_rect, state),
    }
}

struct ScrollTabFrame {
    panel_wid: WidgetId,
    clip: LayoutRectPx,
    y_scroll: i32,
    row_h: u32,
    step_y: i32,
    size_px: f64,
    scroll_id: String,
    scroll_meta: ScrollMeta,
}

fn begin_tab_scroll_frame(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    panel_id: &str,
    scroll_id: &str,
    total_rows: u32,
) -> ScrollTabFrame {
    let row_h = scaled_px(builder, 24.0, 20, 46);
    let step_y = i32::try_from(row_h + 4).unwrap_or(28);
    let content_h = total_rows.saturating_mul(row_h + 4).saturating_add(12);
    let scroll_off = builder.scroll_offset(scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id, rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(panel_id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Import Options", FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.border, 1);

    let scroll_pad = if scroll_meta.has_vertical_overflow() { 14 } else { 4 };
    let clip = LayoutRectPx::new(rect.x + 4, rect.y + 4, rect.width.saturating_sub(scroll_pad + 4), rect.height.saturating_sub(8));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (11.5 * builder.scale()).clamp(8.5, 22.0);
    ScrollTabFrame { panel_wid, clip, y_scroll, row_h, step_y, size_px, scroll_id: scroll_id.to_owned(), scroll_meta }
}

fn end_tab_scroll_frame(builder: &mut LayoutBuilder<'_>, frame: ScrollTabFrame) {
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &frame.scroll_meta, Some(frame.panel_wid));
}

fn row_y_if_visible(frame: &ScrollTabFrame, row_idx: u32, height: u32) -> Option<i32> {
    let ry = frame.clip.y + i32::try_from(row_idx).unwrap_or(0).saturating_mul(frame.step_y) - frame.y_scroll;
    if ry >= frame.clip.y && ry + i32::try_from(height).unwrap_or(24) <= frame.clip.bottom() { Some(ry) } else { None }
}

fn emit_row_label(builder: &mut LayoutBuilder<'_>, frame: &ScrollTabFrame, rect: LayoutRectPx, text: &str) {
    paint_text_fitted(
        builder,
        &frame.panel_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect,
            pad_x: 2,
            text,
            style: &UiTextStyle::ui_sans(frame.size_px),
            color: builder.palette.text,
            scroll_container: Some(&frame.scroll_id),
        },
    );
}

struct SegmentedItem<'a> {
    id: &'a str,
    label: &'a str,
    payload: &'a str,
    selected: bool,
}

#[allow(clippy::too_many_arguments)]
fn emit_segmented_row(builder: &mut LayoutBuilder<'_>, frame: &ScrollTabFrame, ry: i32, row_label: &str, items: &[SegmentedItem<'_>], base_tab: u32) {
    let lbl_w = (frame.clip.width / 4).clamp(76, 150);
    emit_row_label(builder, frame, LayoutRectPx::new(frame.clip.x, ry, lbl_w.saturating_sub(4), frame.row_h), row_label);
    let avail_w = frame.clip.width.saturating_sub(lbl_w + 4);
    let count = (items.len() as u32).max(1);
    let gap = 4u32;
    let btn_w = (avail_w.saturating_sub(gap * count.saturating_sub(1)) / count).max(36);
    for (idx, item) in items.iter().enumerate() {
        let bx = frame.clip.x + i32::try_from(lbl_w + 2 + (idx as u32) * (btn_w + gap)).unwrap_or(0);
        if bx + i32::try_from(btn_w).unwrap_or(36) > frame.clip.right() {
            break;
        }
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: item.id,
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(bx, ry, btn_w, frame.row_h),
                label: item.label,
                keyboard_path: item.id,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(base_tab + idx as u32),
                primary: false,
                selected: item.selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: item.payload.to_owned() },
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_labeled_text_field(
    builder: &mut LayoutBuilder<'_>,
    frame: &ScrollTabFrame,
    ry: i32,
    h: u32,
    id: &str,
    label: &str,
    fallback: &str,
    multiline: bool,
    tab_order: u32,
) {
    let lbl_w = (frame.clip.width / 4).clamp(76, 150);
    emit_row_label(builder, frame, LayoutRectPx::new(frame.clip.x, ry, lbl_w.saturating_sub(4), frame.row_h), label);
    let fx = frame.clip.x + i32::try_from(lbl_w + 2).unwrap_or(80);
    let fw = frame.clip.width.saturating_sub(lbl_w + 6).max(60);
    emit_text_field(
        builder,
        TextFieldSpec {
            id,
            parent: Some(frame.panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(fx, ry, fw, h),
            label,
            fallback_text: fallback,
            keyboard_path: id,
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(tab_order),
            multiline,
            enablement: ControlEnablement::enabled(),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_stepper_row(
    builder: &mut LayoutBuilder<'_>,
    frame: &ScrollTabFrame,
    ry: i32,
    id_prefix: &str,
    label: &str,
    value_text: &str,
    dec_payload: &str,
    inc_payload: &str,
    tab_order: u32,
) {
    let lbl_w = (frame.clip.width / 3).clamp(90, 200);
    emit_row_label(builder, frame, LayoutRectPx::new(frame.clip.x, ry, lbl_w.saturating_sub(4), frame.row_h), label);
    let btn_w = frame.row_h.clamp(20, 34);
    let x0 = frame.clip.x + i32::try_from(lbl_w + 2).unwrap_or(92);
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{id_prefix}.dec"),
            parent: Some(frame.panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(x0, ry, btn_w, frame.row_h),
            label: "-",
            keyboard_path: &format!("{id_prefix}.dec"),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(tab_order),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("import_paste"), payload: dec_payload.to_owned() },
        },
    );
    let val_w = 84u32.min(frame.clip.width.saturating_sub(lbl_w + btn_w * 2 + 12));
    let val_rect = LayoutRectPx::new(x0 + i32::try_from(btn_w + 4).unwrap_or(28), ry, val_w.max(32), frame.row_h);
    builder.paint_fill(ZLayer::ModalDialog, Some(&frame.panel_wid), val_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&frame.panel_wid), val_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &frame.panel_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: val_rect,
            pad_x: 4,
            text: value_text,
            style: &UiTextStyle::grid_tabular(frame.size_px),
            color: builder.palette.text,
            scroll_container: Some(&frame.scroll_id),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{id_prefix}.inc"),
            parent: Some(frame.panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(val_rect.right() + 4, ry, btn_w, frame.row_h),
            label: "+",
            keyboard_path: &format!("{id_prefix}.inc"),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(tab_order + 1),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("import_paste"), payload: inc_payload.to_owned() },
        },
    );
}

fn build_source_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, state: &ImportPasteViewState) {
    let st = &builder.app.grid.import_reshape;
    let locale_owned = builder.tree.locale.clone();
    let locale = locale_owned.as_str();
    let frame = begin_tab_scroll_frame(builder, parent, rect, "import_paste.panel.source", "scroll.import_paste.source", 12);

    // Row 0: Source origin selector
    if let Some(ry) = row_y_if_visible(&frame, 0, frame.row_h) {
        let is_reshape = state.source_origin == "reshape";
        emit_segmented_row(
            builder,
            &frame,
            ry,
            tr_or_default(locale, "import_paste.source.origin", "Source Origin"),
            &[
                SegmentedItem {
                    id: "import_paste.source.plate96",
                    label: "Plate 96",
                    payload: "source:plate96",
                    selected: is_reshape && st.mode == ImportReshapeMode::Plate96,
                },
                SegmentedItem {
                    id: "import_paste.source.plate384",
                    label: "Plate 384",
                    payload: "source:plate384",
                    selected: is_reshape && st.mode == ImportReshapeMode::Plate384,
                },
                SegmentedItem {
                    id: "import_paste.source.tidy",
                    label: "Tidy Long",
                    payload: "source:tidy",
                    selected: is_reshape && st.mode == ImportReshapeMode::TidyLongPivot,
                },
                SegmentedItem {
                    id: "import_paste.source.clipboard",
                    label: "Clipboard",
                    payload: "source:clipboard",
                    selected: state.source_origin == "clipboard",
                },
                SegmentedItem {
                    id: "import_paste.source.csv",
                    label: "CSV / Text",
                    payload: "source:external_csv",
                    selected: state.source_origin == "external_csv",
                },
                SegmentedItem {
                    id: "import_paste.source.internal",
                    label: "Sheet Link",
                    payload: "source:internal_sheet",
                    selected: state.source_origin == "internal_sheet",
                },
            ],
            20,
        );
    }

    // Row 1: Sheet title
    if let Some(ry) = row_y_if_visible(&frame, 1, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_TITLE, "Sheet Title", &st.title, false, 30);
    }

    // Row 2: Target Table Archetype
    if let Some(ry) = row_y_if_visible(&frame, 2, frame.row_h) {
        emit_segmented_row(
            builder,
            &frame,
            ry,
            "Target Kind",
            &[
                SegmentedItem {
                    id: "import_paste.target.column",
                    label: tr_or_default(locale, "table.column", "Column"),
                    payload: "target:Column",
                    selected: st.target_kind == TableKind::Column,
                },
                SegmentedItem {
                    id: "import_paste.target.grouped",
                    label: tr_or_default(locale, "table.grouped", "Grouped"),
                    payload: "target:Grouped",
                    selected: st.target_kind == TableKind::Grouped,
                },
                SegmentedItem {
                    id: "import_paste.target.xy",
                    label: tr_or_default(locale, "table.xy", "XY"),
                    payload: "target:Xy",
                    selected: st.target_kind == TableKind::Xy,
                },
                SegmentedItem {
                    id: "import_paste.target.mv",
                    label: tr_or_default(locale, "table.multiple_variables", "Multi-Var"),
                    payload: "target:MultipleVariables",
                    selected: st.target_kind == TableKind::MultipleVariables,
                },
            ],
            35,
        );
    }

    // Row 3: Delimiter + Auto-Detect
    if let Some(ry) = row_y_if_visible(&frame, 3, frame.row_h) {
        emit_segmented_row(
            builder,
            &frame,
            ry,
            tr_or_default(locale, "import_paste.source.delimiter", "Delimiter"),
            &[
                SegmentedItem { id: "import_paste.delim.comma", label: "Comma (,)", payload: "delim:,", selected: st.tidy_delimiter == ',' },
                SegmentedItem { id: "import_paste.delim.tab", label: "Tab (\\t)", payload: "delim:tab", selected: st.tidy_delimiter == '\t' },
                SegmentedItem { id: "import_paste.delim.semicolon", label: "Semi (;)", payload: "delim:;", selected: st.tidy_delimiter == ';' },
                SegmentedItem { id: "import_paste.delim.pipe", label: "Pipe (|)", payload: "delim:|", selected: st.tidy_delimiter == '|' },
                SegmentedItem {
                    id: "import_paste.detect_format",
                    label: tr_or_default(locale, "import_paste.source.detect", "Auto-Detect"),
                    payload: "detect_format",
                    selected: false,
                },
            ],
            40,
        );
    }

    // Row 4: Decimal & Encoding
    if let Some(ry) = row_y_if_visible(&frame, 4, frame.row_h) {
        emit_segmented_row(
            builder,
            &frame,
            ry,
            tr_or_default(locale, "import_paste.source.decimal", "Decimal"),
            &[
                SegmentedItem { id: "import_paste.decimal.dot", label: "Dot (.)", payload: "decimal:.", selected: st.decimal_separator == '.' },
                SegmentedItem { id: "import_paste.decimal.comma", label: "Comma (,)", payload: "decimal:,", selected: st.decimal_separator == ',' },
                SegmentedItem { id: "import_paste.enc.utf8", label: "UTF-8", payload: "encoding:utf-8", selected: state.encoding == "utf-8" },
                SegmentedItem {
                    id: "import_paste.enc.utf8_bom",
                    label: "UTF-8 BOM",
                    payload: "encoding:utf-8-bom",
                    selected: state.encoding == "utf-8-bom",
                },
                SegmentedItem {
                    id: "import_paste.enc.utf16le",
                    label: "UTF-16LE",
                    payload: "encoding:utf-16le",
                    selected: state.encoding == "utf-16le",
                },
                SegmentedItem { id: "import_paste.enc.latin1", label: "Latin-1", payload: "encoding:latin-1", selected: state.encoding == "latin-1" },
            ],
            50,
        );
    }

    // Row 5: Header detection toggles
    if let Some(ry) = row_y_if_visible(&frame, 5, frame.row_h) {
        let half = (frame.clip.width / 2).saturating_sub(4).max(80);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.has_header",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, half, frame.row_h),
                label: tr_or_default(locale, "import_paste.source.has_header", "First row as column headers"),
                keyboard_path: "import_paste.has_header",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(60),
                checked: state.has_header_row,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:has_header") },
            },
        );
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.has_row_titles",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x + i32::try_from(half + 6).unwrap_or(90), ry, half, frame.row_h),
                label: "First column as row titles",
                keyboard_path: "import_paste.has_row_titles",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(61),
                checked: state.has_row_titles,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:has_row_titles") },
            },
        );
    }

    // Row 6..9: Source payload or internal sheet picker
    if state.source_origin == "internal_sheet" {
        if let Some(ry) = row_y_if_visible(&frame, 6, frame.row_h) {
            emit_row_label(builder, &frame, LayoutRectPx::new(frame.clip.x, ry, frame.clip.width, frame.row_h), "Select Upstream Source Sheet:");
        }
        let doc = builder.app.document();
        let mut sheet_opts: Vec<(String, String)> = Vec::new();
        for ds in &doc.data_sheets {
            sheet_opts.push((ds.id.as_str().to_owned(), format!("Data: {}", ds.title)));
        }
        for rs in &doc.result_sheets {
            sheet_opts.push((rs.id.as_str().to_owned(), format!("Result: {}", rs.title())));
        }
        for (idx, (sid, lbl)) in sheet_opts.iter().enumerate() {
            if let Some(ry) = row_y_if_visible(&frame, 7 + idx as u32, frame.row_h) {
                emit_button(
                    builder,
                    ButtonWidgetSpec {
                        id: &format!("import_paste.internal_sheet.{sid}"),
                        parent: Some(frame.panel_wid.clone()),
                        z_layer: ZLayer::ModalDialog,
                        rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(8), frame.row_h),
                        label: lbl,
                        keyboard_path: &format!("import_paste.internal_sheet.{idx}"),
                        focus_scope: FocusScope::ModalDialog,
                        tab_order: Some(70 + idx as u32),
                        primary: false,
                        selected: state.internal_source_sheet_id.as_deref() == Some(sid.as_str()),
                        shortcut: None,
                        enablement: ControlEnablement::enabled(),
                        action: WidgetAction::Custom { domain: String::from("import_paste"), payload: format!("internal_sheet:{sid}") },
                    },
                );
            }
        }
    } else if state.source_origin == "reshape" && matches!(st.mode, ImportReshapeMode::Plate96 | ImportReshapeMode::Plate384) {
        if let Some(ry) = row_y_if_visible(&frame, 6, frame.row_h) {
            emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_X_TITLE, "X Axis Title", &st.x_title, false, 70);
        }
        let plate_h = frame.row_h * 2 + 4;
        if let Some(ry) = row_y_if_visible(&frame, 7, plate_h) {
            let plate_txt = format_plate_values_csv(&st.plate_values, st.decimal_separator);
            emit_labeled_text_field(builder, &frame, ry, plate_h, WID_PLATE_VALUES, "Well Readings", &plate_txt, true, 71);
        }
        if let Some(ry) = row_y_if_visible(&frame, 9, frame.row_h) {
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: "import_paste.load_sample",
                    parent: Some(frame.panel_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(frame.clip.x, ry, 180u32.min(frame.clip.width), frame.row_h),
                    label: "Load Sample Plate",
                    keyboard_path: "import_paste.load_sample",
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(72),
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("load_sample") },
                },
            );
        }
    } else {
        let mut next_row = 6u32;
        if state.source_origin == "external_csv" {
            if let Some(ry) = row_y_if_visible(&frame, next_row, frame.row_h) {
                emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_PATH, "CSV File Path", &state.external_path, false, 70);
            }
            next_row += 1;
        }
        let csv_h = frame.row_h * 2 + 4;
        if let Some(ry) = row_y_if_visible(&frame, next_row, csv_h) {
            let csv_fb = if !st.tidy_csv.is_empty() { st.tidy_csv.as_str() } else { state.raw_source_text.as_str() };
            emit_labeled_text_field(builder, &frame, ry, csv_h, WID_CSV, "Raw Data (CSV/TSV)", csv_fb, true, 71);
        }
        next_row += 2;
        if let Some(ry) = row_y_if_visible(&frame, next_row, frame.row_h) {
            let bw = 148u32.min((frame.clip.width / 2).saturating_sub(6));
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: "import_paste.paste_clipboard",
                    parent: Some(frame.panel_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(frame.clip.x, ry, bw, frame.row_h),
                    label: "Paste Clipboard",
                    keyboard_path: "import_paste.paste_clipboard",
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(72),
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("paste_clipboard") },
                },
            );
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: "import_paste.load_sample",
                    parent: Some(frame.panel_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(frame.clip.x + i32::try_from(bw + 6).unwrap_or(154), ry, bw, frame.row_h),
                    label: "Load Sample Data",
                    keyboard_path: "import_paste.load_sample",
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(73),
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("load_sample") },
                },
            );
        }
    }

    if !state.last_status.is_empty()
        && let Some(ry) = row_y_if_visible(&frame, 10, frame.row_h)
    {
        emit_row_label(builder, &frame, LayoutRectPx::new(frame.clip.x, ry, frame.clip.width, frame.row_h), &state.last_status);
    }

    end_tab_scroll_frame(builder, frame);
}

fn build_view_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, state: &ImportPasteViewState) {
    let preview = compute_import_preview(builder.app, state);
    let size_px = (11.5 * builder.scale()).clamp(8.5, 22.0);
    let banner_h = scaled_px(builder, 24.0, 20, 42);
    let banner_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, banner_h);

    let summary_text = if let Some(ref err) = preview.validation_error {
        format!("{} | {err}", preview.mode_summary)
    } else {
        format!(
            "Preview: {} rows × {} cols | {} | Delim: {} | Dec: {} | Enc: {} | Excluded: {}",
            preview.rows.len(),
            preview.headers.len(),
            preview.mode_summary,
            delimiter_display_short(builder.app.grid.import_reshape.tidy_delimiter),
            builder.app.grid.import_reshape.decimal_separator,
            state.encoding,
            preview.excluded_count,
        )
    };
    let banner_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("import_paste.view.summary"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: banner_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Banner, summary_text.clone(), FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    let border_col = match preview.provenance_border {
        Some(LinkBorderColor::Blue) => builder.palette.link_blue,
        Some(LinkBorderColor::Green) => builder.palette.link_green,
        Some(LinkBorderColor::Red) => builder.palette.link_red,
        Some(LinkBorderColor::Charcoal) => Rgba8::rgb(71, 85, 105),
        None => builder.palette.border,
    };
    builder.paint_fill(ZLayer::ModalDialog, Some(&banner_wid), banner_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&banner_wid), banner_rect, border_col, 1);
    paint_text_fitted(
        builder,
        &banner_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: banner_rect,
            pad_x: 6,
            text: &summary_text,
            style: &UiTextStyle::ui_sans(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let grid_y = banner_rect.bottom() + 4;
    let grid_h = u32::try_from((rect.bottom() - grid_y).max(0)).unwrap_or(0);
    if grid_h < 28 || rect.width < 60 {
        return;
    }
    let grid_rect = LayoutRectPx::new(rect.x, grid_y, rect.width, grid_h);
    let col_w = scaled_px(builder, 92.0, 72, 160);
    let gutter_w = scaled_px(builder, 46.0, 36, 90);
    let row_h = scaled_px(builder, 22.0, 18, 42);
    let total_cols = preview.headers.len().max(preview.rows.iter().map(Vec::len).max().unwrap_or(0));
    let content_w = gutter_w.saturating_add((total_cols as u32).saturating_mul(col_w)).saturating_add(16);
    let content_h = ((preview.rows.len() + 1) as u32).saturating_mul(row_h).saturating_add(16);
    let scroll_id = String::from("scroll.import_paste.view_grid");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), grid_rect, [content_w, content_h], scroll_off, ScrollAxes::Both);

    let grid_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("import_paste.view_grid"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: grid_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Parsed Preview Grid", FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&grid_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&grid_wid), grid_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&grid_wid), grid_rect, border_col, if preview.provenance_border.is_some() { 2 } else { 1 });

    let sb_r = if scroll_meta.has_vertical_overflow() { 14 } else { 2 };
    let sb_b = if scroll_meta.has_horizontal_overflow() { 14 } else { 2 };
    let grid_clip =
        LayoutRectPx::new(grid_rect.x + 2, grid_rect.y + 2, grid_rect.width.saturating_sub(sb_r + 2), grid_rect.height.saturating_sub(sb_b + 2));
    builder.push_clip(grid_clip);

    let x_scroll = scroll_meta.scroll_offset_px[0].round() as i32;
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let hdr_style = UiTextStyle::ui_bold(size_px);
    let cell_style = UiTextStyle::grid_tabular(size_px);

    // Header row
    let hdr_rect = LayoutRectPx::new(grid_clip.x, grid_clip.y, grid_clip.width, row_h);
    builder.paint_fill(ZLayer::ModalDialog, Some(&grid_wid), hdr_rect, builder.palette.header);
    let corner_rect = LayoutRectPx::new(grid_clip.x, grid_clip.y, gutter_w, row_h).intersect(grid_clip);
    paint_text_fitted(
        builder,
        &grid_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: corner_rect,
            pad_x: 3,
            text: "#",
            style: &hdr_style,
            color: builder.palette.muted,
            scroll_container: Some(&scroll_id),
        },
    );
    for c_idx in 0..total_cols {
        let cx = grid_clip.x + i32::try_from(gutter_w).unwrap_or(46) + i32::try_from(c_idx as u32 * col_w).unwrap_or(0) - x_scroll;
        if cx + i32::try_from(col_w).unwrap_or(92) <= grid_clip.x + i32::try_from(gutter_w).unwrap_or(46) || cx >= grid_clip.right() {
            continue;
        }
        let h_cell_rect = LayoutRectPx::new(cx, grid_clip.y, col_w, row_h).intersect(LayoutRectPx::new(
            grid_clip.x + i32::try_from(gutter_w).unwrap_or(46),
            grid_clip.y,
            grid_clip.width.saturating_sub(gutter_w),
            row_h,
        ));
        if h_cell_rect.width < 6 {
            continue;
        }
        builder.paint_stroke(ZLayer::ModalDialog, Some(&grid_wid), h_cell_rect, builder.palette.border, 1);
        let h_lbl = preview.headers.get(c_idx).map(String::as_str).unwrap_or("");
        paint_text_fitted(
            builder,
            &grid_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: h_cell_rect,
                pad_x: 4,
                text: h_lbl,
                style: &hdr_style,
                color: builder.palette.text,
                scroll_container: Some(&scroll_id),
            },
        );
    }

    // Virtualized visible rows & columns inside data_clip
    let data_clip =
        LayoutRectPx::new(grid_clip.x, grid_clip.y + i32::try_from(row_h).unwrap_or(22), grid_clip.width, grid_clip.height.saturating_sub(row_h));
    if data_clip.height >= row_h {
        builder.push_clip(data_clip);
        for (r_idx, row) in preview.rows.iter().enumerate() {
            let ry = data_clip.y + i32::try_from(r_idx as u32 * row_h).unwrap_or(0) - y_scroll;
            if ry + i32::try_from(row_h).unwrap_or(22) <= data_clip.y || ry >= data_clip.bottom() {
                continue;
            }
            let r_label = preview.row_labels.get(r_idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("{}", r_idx + 1));
            let gutter_rect = LayoutRectPx::new(data_clip.x, ry, gutter_w, row_h).intersect(data_clip);
            builder.paint_fill(ZLayer::ModalDialog, Some(&grid_wid), gutter_rect, builder.palette.panel);
            builder.paint_stroke(ZLayer::ModalDialog, Some(&grid_wid), gutter_rect, builder.palette.border, 1);
            paint_text_fitted(
                builder,
                &grid_wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: gutter_rect,
                    pad_x: 3,
                    text: &r_label,
                    style: &cell_style,
                    color: builder.palette.muted,
                    scroll_container: Some(&scroll_id),
                },
            );

            let cells_clip = LayoutRectPx::new(
                data_clip.x + i32::try_from(gutter_w).unwrap_or(46),
                data_clip.y,
                data_clip.width.saturating_sub(gutter_w),
                data_clip.height,
            );
            for (c_idx, cell) in row.iter().enumerate() {
                let cx = cells_clip.x + i32::try_from(c_idx as u32 * col_w).unwrap_or(0) - x_scroll;
                if cx + i32::try_from(col_w).unwrap_or(92) <= cells_clip.x || cx >= cells_clip.right() {
                    continue;
                }
                let cell_rect = LayoutRectPx::new(cx, ry, col_w, row_h).intersect(cells_clip);
                if cell_rect.width < 6 || cell_rect.height < 6 {
                    continue;
                }
                builder.paint_stroke(ZLayer::ModalDialog, Some(&grid_wid), cell_rect, builder.palette.border, 1);
                let disp = format_cell_display(cell, &builder.app.locale_preset);
                if !disp.is_empty() {
                    let ink = if cell.excluded { builder.palette.excluded } else { builder.palette.text };
                    let style = if cell.excluded { cell_style.clone().italic() } else { cell_style.clone() };
                    paint_text_fitted(
                        builder,
                        &grid_wid,
                        FittedTextSpec {
                            z_layer: ZLayer::ModalDialog,
                            rect: cell_rect,
                            pad_x: 4,
                            text: &disp,
                            style: &style,
                            color: ink,
                            scroll_container: Some(&scroll_id),
                        },
                    );
                    if cell.excluded && cell_rect.width >= 10 {
                        let mid_y = cell_rect.center()[1];
                        builder.paint_line(ZLayer::ModalDialog, Some(&grid_wid), [cell_rect.x + 3, mid_y], [cell_rect.right() - 3, mid_y], ink, 1);
                    }
                }
            }
        }
        builder.pop_clip();
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(grid_wid));
}

fn build_filter_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, state: &ImportPasteViewState) {
    let st = &builder.app.grid.import_reshape;
    let locale_owned = builder.tree.locale.clone();
    let locale = locale_owned.as_str();
    let frame = begin_tab_scroll_frame(builder, parent, rect, "import_paste.panel.filter", "scroll.import_paste.filter", 14);

    // Row 0: Row slicing (start / end / reset)
    if let Some(ry) = row_y_if_visible(&frame, 0, frame.row_h) {
        let lbl_w = 100u32.min(frame.clip.width / 4);
        emit_row_label(builder, &frame, LayoutRectPx::new(frame.clip.x, ry, lbl_w, frame.row_h), "Row Slice");
        let field_w = ((frame.clip.width.saturating_sub(lbl_w + 96)) / 2).max(48);
        let x1 = frame.clip.x + i32::try_from(lbl_w + 4).unwrap_or(104);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: WID_ROW_START,
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x1, ry, field_w, frame.row_h),
                label: "Row Start",
                fallback_text: &state.row_start,
                keyboard_path: WID_ROW_START,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        let x2 = x1 + i32::try_from(field_w + 6).unwrap_or(60);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: WID_ROW_END,
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x2, ry, field_w, frame.row_h),
                label: "Row End",
                fallback_text: &state.row_end,
                keyboard_path: WID_ROW_END,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(21),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "import_paste.clear_slice",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x2 + i32::try_from(field_w + 6).unwrap_or(60), ry, 80, frame.row_h),
                label: "Reset Slice",
                keyboard_path: "import_paste.clear_slice",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(22),
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("clear_slice") },
            },
        );
    }

    // Row 1: Column slicing (start / end)
    if let Some(ry) = row_y_if_visible(&frame, 1, frame.row_h) {
        let lbl_w = 100u32.min(frame.clip.width / 4);
        emit_row_label(builder, &frame, LayoutRectPx::new(frame.clip.x, ry, lbl_w, frame.row_h), "Col Slice");
        let field_w = ((frame.clip.width.saturating_sub(lbl_w + 16)) / 2).max(48);
        let x1 = frame.clip.x + i32::try_from(lbl_w + 4).unwrap_or(104);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: WID_COL_START,
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x1, ry, field_w, frame.row_h),
                label: "Col Start",
                fallback_text: &state.col_start,
                keyboard_path: WID_COL_START,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(23),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        let x2 = x1 + i32::try_from(field_w + 6).unwrap_or(60);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: WID_COL_END,
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x2, ry, field_w, frame.row_h),
                label: "Col End",
                fallback_text: &state.col_end,
                keyboard_path: WID_COL_END,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(24),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
    }

    // Row 2: Decimate every k-th row
    if let Some(ry) = row_y_if_visible(&frame, 2, frame.row_h) {
        emit_stepper_row(
            builder,
            &frame,
            ry,
            "import_paste.decimate",
            "Keep every k-th row",
            &format!("{}", state.decimate_every_k.max(1)),
            "decimate:-1",
            "decimate:+1",
            25,
        );
    }

    // Row 3: Unstacking toggle
    if let Some(ry) = row_y_if_visible(&frame, 3, frame.row_h) {
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.unstack_toggle",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h),
                label: tr_or_default(locale, "import_paste.filter.unstack", "Unstack by group column"),
                keyboard_path: "import_paste.unstack_toggle",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(28),
                checked: state.unstack_enabled,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:unstack") },
            },
        );
    }

    // Row 4: Unstack group column stepper
    if let Some(ry) = row_y_if_visible(&frame, 4, frame.row_h) {
        emit_stepper_row(
            builder,
            &frame,
            ry,
            "import_paste.unstack_col",
            "Unstack Group Col",
            &format!("Col {}", state.unstack_group_col),
            "unstack_col:-1",
            "unstack_col:+1",
            29,
        );
    }

    // Row 5 & 6: Tidy Pivot column mappings
    if let Some(ry) = row_y_if_visible(&frame, 5, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_GROUP_COL, "Tidy Group Col", &st.group_col, false, 32);
    }
    if let Some(ry) = row_y_if_visible(&frame, 6, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_VALUE_COL, "Tidy Value Col", &st.value_col, false, 33);
    }
    if let Some(ry) = row_y_if_visible(&frame, 7, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_X_COL, "Tidy X Col", st.x_col.as_deref().unwrap_or(""), false, 34);
    }

    // Row 8: Plate-map reshape options & presets
    if let Some(ry) = row_y_if_visible(&frame, 8, frame.row_h) {
        let chk_w = (frame.clip.width / 2).saturating_sub(10).max(120);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import.subtract_blank_mean",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, chk_w, frame.row_h),
                label: tr_or_default(locale, "import_paste.filter.subtract_blank", "Subtract Blank well mean"),
                keyboard_path: "import.subtract_blank_mean",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(36),
                checked: st.subtract_blank_mean,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:subtract_blank_mean") },
            },
        );
        let btn_w = ((frame.clip.width.saturating_sub(chk_w + 14)) / 2).max(60);
        let bx1 = frame.clip.x + i32::try_from(chk_w + 6).unwrap_or(126);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "import_paste.plate.preset_96",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(bx1, ry, btn_w, frame.row_h),
                label: "96-Well Preset",
                keyboard_path: "import_paste.plate.preset_96",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(37),
                primary: false,
                selected: state.plate_preset == "96",
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("plate_preset:96") },
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "import_paste.plate.preset_384",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(bx1 + i32::try_from(btn_w + 4).unwrap_or(64), ry, btn_w, frame.row_h),
                label: "384-Well Preset",
                keyboard_path: "import_paste.plate.preset_384",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(38),
                primary: false,
                selected: state.plate_preset == "384",
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("plate_preset:384") },
            },
        );
    }

    // Row 9: Asterisk exclusion markers toggle
    if let Some(ry) = row_y_if_visible(&frame, 9, frame.row_h) {
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.parse_asterisk",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h),
                label: tr_or_default(locale, "import_paste.filter.parse_asterisk", "Parse * exclusion markers (e.g. 45.6*, *45.6)"),
                keyboard_path: "import_paste.parse_asterisk",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(40),
                checked: state.parse_asterisk_exclusions,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:parse_asterisk") },
            },
        );
    }

    // Row 10 & 11: Exclusion pattern & row predicate
    if let Some(ry) = row_y_if_visible(&frame, 10, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_FILTER_PATTERN, "Exclusion Token", &state.filter_pattern, false, 41);
    }
    if let Some(ry) = row_y_if_visible(&frame, 11, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_EXCLUDE_PRED, "Exclude Predicate", &state.exclude_predicate, false, 42);
    }
    if let Some(ry) = row_y_if_visible(&frame, 12, frame.row_h) {
        emit_labeled_text_field(
            builder,
            &frame,
            ry,
            frame.row_h,
            WID_REPLICATE_COL,
            "Tidy Rep Col",
            st.replicate_col.as_deref().unwrap_or(""),
            false,
            43,
        );
    }
    if let Some(ry) = row_y_if_visible(&frame, 13, frame.row_h) {
        emit_labeled_text_field(
            builder,
            &frame,
            ry,
            frame.row_h,
            WID_EXCLUDED_COL,
            "Tidy Excl Col",
            st.excluded_col.as_deref().unwrap_or(""),
            false,
            44,
        );
    }

    end_tab_scroll_frame(builder, frame);
}

fn build_placement_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, state: &ImportPasteViewState) {
    let locale_owned = builder.tree.locale.clone();
    let locale = locale_owned.as_str();
    let frame = begin_tab_scroll_frame(builder, parent, rect, "import_paste.panel.placement", "scroll.import_paste.placement", 12);

    // Row 0: Transpose toggle
    if let Some(ry) = row_y_if_visible(&frame, 0, frame.row_h) {
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.transpose",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h),
                label: tr_or_default(locale, "import_paste.placement.transpose", "Transpose (swap rows and columns)"),
                keyboard_path: "import_paste.transpose",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20),
                checked: state.transpose,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:transpose") },
            },
        );
    }

    // Row 1: Row-wrap stepper + quick presets
    if let Some(ry) = row_y_if_visible(&frame, 1, frame.row_h) {
        let wrap_txt = if state.row_wrap_width == 0 { String::from("Off") } else { format!("{} cols", state.row_wrap_width) };
        let half_w = (frame.clip.width * 3 / 5).max(180);
        let sub_frame = ScrollTabFrame {
            panel_wid: frame.panel_wid.clone(),
            clip: LayoutRectPx::new(frame.clip.x, frame.clip.y, half_w, frame.clip.height),
            ..ScrollTabFrame {
                panel_wid: frame.panel_wid.clone(),
                clip: frame.clip,
                y_scroll: frame.y_scroll,
                row_h: frame.row_h,
                step_y: frame.step_y,
                size_px: frame.size_px,
                scroll_id: frame.scroll_id.clone(),
                scroll_meta: frame.scroll_meta.clone(),
            }
        };
        emit_stepper_row(
            builder,
            &sub_frame,
            ry,
            "import_paste.row_wrap",
            tr_or_default(locale, "import_paste.placement.row_wrap", "Row-Wrap Columns"),
            &wrap_txt,
            "row_wrap:-1",
            "row_wrap:+1",
            21,
        );
        let px = frame.clip.x + i32::try_from(half_w + 4).unwrap_or(184);
        let rem_w = frame.clip.width.saturating_sub(half_w + 8);
        let p_w = (rem_w.saturating_sub(12) / 4).max(28);
        for (idx, (pid, lbl, wval)) in [
            ("import_paste.row_wrap.off", "Off", "row_wrap_set:0"),
            ("import_paste.row_wrap.2", "2", "row_wrap_set:2"),
            ("import_paste.row_wrap.3", "3", "row_wrap_set:3"),
            ("import_paste.row_wrap.4", "4", "row_wrap_set:4"),
        ]
        .iter()
        .enumerate()
        {
            let bx = px + i32::try_from(idx as u32 * (p_w + 4)).unwrap_or(0);
            if bx + i32::try_from(p_w).unwrap_or(28) > frame.clip.right() {
                break;
            }
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: pid,
                    parent: Some(frame.panel_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, ry, p_w, frame.row_h),
                    label: lbl,
                    keyboard_path: pid,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(24 + idx as u32),
                    primary: false,
                    selected: (idx == 0 && state.row_wrap_width == 0) || (idx > 0 && state.row_wrap_width == idx + 1),
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("import_paste"), payload: (*wval).to_owned() },
                },
            );
        }
    }

    // Row 2: Header propagation & skip blanks
    if let Some(ry) = row_y_if_visible(&frame, 2, frame.row_h) {
        let half = (frame.clip.width / 2).saturating_sub(4).max(80);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.propagate_headers",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, half, frame.row_h),
                label: "Promote header row to titles",
                keyboard_path: "import_paste.propagate_headers",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(29),
                checked: state.propagate_title_header,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:propagate_headers") },
            },
        );
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.skip_blanks",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x + i32::try_from(half + 6).unwrap_or(90), ry, half, frame.row_h),
                label: "Skip contiguous blank rows",
                keyboard_path: "import_paste.skip_blanks",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(30),
                checked: state.skip_contiguous_blanks,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:skip_blanks") },
            },
        );
    }

    // Row 3: Target destination (New sheet vs Existing sheet)
    if let Some(ry) = row_y_if_visible(&frame, 3, frame.row_h) {
        emit_segmented_row(
            builder,
            &frame,
            ry,
            "Destination",
            &[
                SegmentedItem {
                    id: "import_paste.target_dest.new",
                    label: "New Data Sheet",
                    payload: "target_dest:new",
                    selected: state.target_mode != "existing_sheet",
                },
                SegmentedItem {
                    id: "import_paste.target_dest.existing",
                    label: "Existing Sheet",
                    payload: "target_dest:existing",
                    selected: state.target_mode == "existing_sheet",
                },
            ],
            32,
        );
    }

    // Row 4: Placement Mode
    if let Some(ry) = row_y_if_visible(&frame, 4, frame.row_h) {
        emit_segmented_row(
            builder,
            &frame,
            ry,
            "Insert Mode",
            &[
                SegmentedItem {
                    id: "import_paste.place_mode.overwrite",
                    label: "Overwrite",
                    payload: "place_mode:overwrite",
                    selected: state.placement_mode == "overwrite",
                },
                SegmentedItem {
                    id: "import_paste.place_mode.insert_rows",
                    label: "Insert Rows",
                    payload: "place_mode:insert_rows",
                    selected: state.placement_mode == "insert_rows",
                },
                SegmentedItem {
                    id: "import_paste.place_mode.append_rows",
                    label: "Append Rows",
                    payload: "place_mode:append_rows",
                    selected: state.placement_mode == "append_rows",
                },
                SegmentedItem {
                    id: "import_paste.place_mode.append_columns",
                    label: "Append Cols",
                    payload: "place_mode:append_columns",
                    selected: state.placement_mode == "append_columns",
                },
            ],
            35,
        );
    }

    // Row 5..7: Anchor offset steppers
    if let Some(ry) = row_y_if_visible(&frame, 5, frame.row_h) {
        emit_stepper_row(
            builder,
            &frame,
            ry,
            "import_paste.anchor_row",
            "Target Row Offset",
            &format!("Row {}", state.anchor_row),
            "anchor_row:-1",
            "anchor_row:+1",
            40,
        );
    }
    if let Some(ry) = row_y_if_visible(&frame, 6, frame.row_h) {
        emit_stepper_row(
            builder,
            &frame,
            ry,
            "import_paste.anchor_col",
            "Target Col Offset",
            &format!("Col {}", state.anchor_col),
            "anchor_col:-1",
            "anchor_col:+1",
            42,
        );
    }
    if let Some(ry) = row_y_if_visible(&frame, 7, frame.row_h) {
        emit_stepper_row(
            builder,
            &frame,
            ry,
            "import_paste.anchor_subcol",
            "Target Subcol Offset",
            &format!("Sub {}", state.anchor_subcol),
            "anchor_subcol:-1",
            "anchor_subcol:+1",
            44,
        );
    }

    // Row 8: Live link toggle
    if let Some(ry) = row_y_if_visible(&frame, 8, frame.row_h) {
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.live_link",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h),
                label: tr_or_default(locale, "import_paste.placement.live_link", "Keep Live Link (Paste Link provenance region)"),
                keyboard_path: "import_paste.live_link",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(46),
                checked: state.paste_as_live_link,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:live_link") },
            },
        );
    }

    // Row 9+: Existing target DataSheets selector
    let sheets = &builder.app.document().data_sheets;
    if !sheets.is_empty() && state.target_mode == "existing_sheet" {
        for (idx, ds) in sheets.iter().enumerate() {
            if let Some(ry) = row_y_if_visible(&frame, 9 + idx as u32, frame.row_h) {
                emit_button(
                    builder,
                    ButtonWidgetSpec {
                        id: &format!("import_paste.dest_sheet.{}", ds.id.as_str()),
                        parent: Some(frame.panel_wid.clone()),
                        z_layer: ZLayer::ModalDialog,
                        rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(8), frame.row_h),
                        label: &format!("Target Sheet: {} ({})", ds.title, ds.id.as_str()),
                        keyboard_path: &format!("import_paste.dest_sheet.{idx}"),
                        focus_scope: FocusScope::ModalDialog,
                        tab_order: Some(50 + idx as u32),
                        primary: false,
                        selected: state.target_sheet_id.as_deref() == Some(ds.id.as_str()),
                        shortcut: None,
                        enablement: ControlEnablement::enabled(),
                        action: WidgetAction::Custom { domain: String::from("import_paste"), payload: format!("dest_sheet:{}", ds.id.as_str()) },
                    },
                );
            }
        }
    }

    end_tab_scroll_frame(builder, frame);
}

fn build_notes_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, state: &ImportPasteViewState) {
    let locale_owned = builder.tree.locale.clone();
    let locale = locale_owned.as_str();
    let preview = compute_import_preview(builder.app, state);
    let frame = begin_tab_scroll_frame(builder, parent, rect, "import_paste.panel.notes", "scroll.import_paste.notes", 9);

    if let Some(ry) = row_y_if_visible(&frame, 0, frame.row_h) {
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "import_paste.record_info",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h),
                label: tr_or_default(locale, "import_paste.notes.record_info", "Record provenance note in linked InfoSheet"),
                keyboard_path: "import_paste.record_info",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20),
                checked: state.record_in_info_sheet,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&frame.scroll_id),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("toggle:record_info") },
            },
        );
    }

    if let Some(ry) = row_y_if_visible(&frame, 1, frame.row_h) {
        emit_labeled_text_field(builder, &frame, ry, frame.row_h, WID_LINK_ID, "Link / Import ID", &state.link_id, false, 21);
    }

    let notes_h = frame.row_h * 2 + 4;
    if let Some(ry) = row_y_if_visible(&frame, 2, notes_h) {
        emit_labeled_text_field(builder, &frame, ry, notes_h, WID_NOTES, "Provenance Notes", &state.provenance_notes, true, 22);
    }

    if let Some(ry) = row_y_if_visible(&frame, 4, frame.row_h) {
        let border_tag = match preview.provenance_border {
            Some(LinkBorderColor::Blue) => "Blue border (Internal DataSheet)",
            Some(LinkBorderColor::Green) => "Green border (Analyzable ResultSheet)",
            Some(LinkBorderColor::Red) => "Red border (Terminal ResultSheet)",
            Some(LinkBorderColor::Charcoal) => "Charcoal border (External CSV File)",
            None => "Static Import (No Live Link)",
        };
        let link_id_disp = effective_text(builder.app, WID_LINK_ID, &state.link_id);
        let prov_text = format!(
            "Origin: {} | ID: {} | {border_tag} | {}×{} cells | Row-Wrap={} | Excluded={}",
            state.source_origin,
            link_id_disp,
            preview.rows.len(),
            preview.headers.len(),
            if state.row_wrap_width == 0 { String::from("Off") } else { state.row_wrap_width.to_string() },
            preview.excluded_count,
        );
        let prov_rect = LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from("import_paste.notes.provenance_preview"),
            parent: Some(frame.panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: prov_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Banner, prov_text.clone(), FocusScope::ModalDialog),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), prov_rect, builder.palette.surface);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), prov_rect, builder.palette.border, 1);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: prov_rect,
                pad_x: 6,
                text: &prov_text,
                style: &UiTextStyle::ui_sans(frame.size_px),
                color: builder.palette.text,
                scroll_container: Some(&frame.scroll_id),
            },
        );
    }

    if let Some(ry) = row_y_if_visible(&frame, 5, frame.row_h) {
        let status_msg = if !state.last_status.is_empty() {
            state.last_status.clone()
        } else if let Some(ref err) = preview.validation_error {
            format!("Incomplete: {err}")
        } else {
            format!("Ready: {} ({} rows × {} cols)", preview.mode_summary, preview.rows.len(), preview.headers.len())
        };
        let st_rect = LayoutRectPx::new(frame.clip.x, ry, frame.clip.width.saturating_sub(4), frame.row_h);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from("import_paste.notes.status"),
            parent: Some(frame.panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: st_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, status_msg.clone(), FocusScope::ModalDialog),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: st_rect,
                pad_x: 4,
                text: &status_msg,
                style: &UiTextStyle::ui_sans(frame.size_px),
                color: builder.palette.muted,
                scroll_container: Some(&frame.scroll_id),
            },
        );
    }

    if let Some(ry) = row_y_if_visible(&frame, 6, frame.row_h) {
        let bw = 160u32.min((frame.clip.width / 2).saturating_sub(6));
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "import_paste.sync",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x, ry, bw, frame.row_h),
                label: "Sync Inputs",
                keyboard_path: "import_paste.sync",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(25),
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("sync") },
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "import_paste.commit",
                parent: Some(frame.panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(frame.clip.x + i32::try_from(bw + 8).unwrap_or(168), ry, bw, frame.row_h),
                label: tr_or_default(locale, "import_paste.commit", "Apply & Import"),
                keyboard_path: "import_paste.commit",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(26),
                primary: true,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("commit") },
            },
        );
    }

    end_tab_scroll_frame(builder, frame);
}

// ============================================================================
// Interactive Convert Table Archetype Dialog (`R10.4`)
// ============================================================================

struct ConvertColumnMappingRow {
    source_col: String,
    target_col: String,
    status: String,
    lossy: bool,
}

fn table_kind_to_target(kind: TableKind) -> TargetArchetypeKind {
    match kind {
        TableKind::Column => TargetArchetypeKind::Column,
        TableKind::Grouped => TargetArchetypeKind::Grouped,
        TableKind::Xy => TargetArchetypeKind::Xy,
        TableKind::MultipleVariables => TargetArchetypeKind::MultipleVariables,
        TableKind::Contingency => TargetArchetypeKind::Contingency,
        _ => TargetArchetypeKind::Column,
    }
}

fn build_convert_mapping_rows(src: &DataSheet, dst: &DataSheet, preview: &ConversionPreview) -> Vec<ConvertColumnMappingRow> {
    let mut rows = Vec::new();
    let src_cols = src.archetype.column_count();
    let dst_cols = dst.archetype.column_count();
    let dst_has_x = matches!(dst.archetype, TableArchetype::Xy(_));

    for c in 0..src_cols.max(1) {
        let src_title = crate::dialogs_canvas::data_column_title(src, c).map_or_else(|| format!("Col {}", c + 1), str::to_owned);
        let src_subs = match &src.archetype {
            TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
            _ => 1,
        };
        let target_label = if dst_has_x && src.kind() != TableKind::Xy && src_cols > 1 && c == 0 {
            String::from("X Coordinate Column")
        } else {
            let dst_idx = if dst_has_x && src.kind() != TableKind::Xy && src_cols > 1 { c.saturating_sub(1) } else { c };
            if dst_idx < dst_cols {
                crate::dialogs_canvas::data_column_title(dst, dst_idx).map_or_else(|| format!("Target Col {}", dst_idx + 1), str::to_owned)
            } else {
                String::from("Reshaped / Merged")
            }
        };
        let dst_subs = match &dst.archetype {
            TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
            _ => 1,
        };
        let drops_subs = src_subs > dst_subs && preview.lossy;
        let status = if src.kind() == dst.kind() {
            String::from("Unchanged")
        } else if drops_subs {
            format!("Drops {}-subcols", src_subs - dst_subs)
        } else if preview.lossy {
            String::from("Lossy reshape")
        } else {
            String::from("Preserved")
        };
        rows.push(ConvertColumnMappingRow {
            source_col: if src_subs > 1 { format!("{src_title} ({src_subs} subcols)") } else { src_title },
            target_col: target_label,
            status,
            lossy: drops_subs || preview.lossy,
        });
    }
    rows
}

fn build_convert_archetype_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx, sheet_id: &SheetId) {
    let locale_owned = builder.tree.locale.clone();
    let locale = locale_owned.as_str();
    let st = &builder.app.canvas_dialogs.convert_archetype;
    let Some(src_sheet) = builder.app.document().data_sheet(sheet_id) else {
        return;
    };

    let target_arch = table_kind_to_target(st.target_kind);
    let live_res = convert_datasheet_archetype(src_sheet, target_arch, true);
    let (converted_ds, preview) = match live_res {
        Ok((ds, prev)) => (Some(ds), prev),
        Err(err) => (
            None,
            ConversionPreview {
                source_kind: src_sheet.kind(),
                target_kind: target_arch,
                preserved_cells: 0,
                dropped_cells: 0,
                preserved_exclusions: 0,
                warnings: vec![err.to_string()],
                lossy: false,
            },
        ),
    };

    let size_px = (11.5 * builder.scale()).clamp(8.5, 22.0);
    let row_h = scaled_px(builder, 24.0, 20, 42);
    let mut y = body_rect.y;

    // 1. Target archetype selector strip
    let lbl_w = 112u32.min(body_rect.width / 4);
    let sel_lbl_rect = LayoutRectPx::new(body_rect.x, y, lbl_w, row_h);
    paint_text_fitted(
        builder,
        &parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: sel_lbl_rect,
            pad_x: 2,
            text: tr_or_default(locale, "convert_archetype.target_selector", "Target Archetype"),
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let targets = [
        ("convert.target.column", tr_or_default(locale, "table.column", "Column"), TableKind::Column, "Column"),
        ("convert.target.grouped", tr_or_default(locale, "table.grouped", "Grouped"), TableKind::Grouped, "Grouped"),
        ("convert.target.xy", tr_or_default(locale, "table.xy", "XY"), TableKind::Xy, "Xy"),
        (
            "convert.target.multiple_variables",
            tr_or_default(locale, "table.multiple_variables", "Multi-Var"),
            TableKind::MultipleVariables,
            "MultipleVariables",
        ),
        ("convert.target.contingency", tr_or_default(locale, "table.contingency", "Contingency"), TableKind::Contingency, "Contingency"),
    ];
    let avail_w = body_rect.width.saturating_sub(lbl_w + 6);
    let btn_w = (avail_w.saturating_sub(16) / 5).max(46);
    for (idx, (wid, lbl, kind, wire)) in targets.iter().enumerate() {
        let bx = body_rect.x + i32::try_from(lbl_w + 4 + (idx as u32) * (btn_w + 4)).unwrap_or(0);
        if bx + i32::try_from(btn_w).unwrap_or(46) > body_rect.right() {
            break;
        }
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: wid,
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(bx, y, btn_w, row_h),
                label: lbl,
                keyboard_path: wid,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + idx as u32),
                primary: false,
                selected: st.target_kind == *kind,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: format!("convert.target:{wire}") },
            },
        );
    }
    y += i32::try_from(row_h + 6).unwrap_or(30);

    // 2. Explicit Lossy-Change Warning / Status Banner
    let banner_h = scaled_px(builder, 26.0, 22, 46);
    if y + i32::try_from(banner_h).unwrap_or(26) <= body_rect.bottom() {
        let banner_rect = LayoutRectPx::new(body_rect.x, y, body_rect.width, banner_h);
        let warn_detail = preview.warnings.first().map(String::as_str).unwrap_or("");
        let banner_blocked = converted_ds.is_none();
        let banner_msg = if banner_blocked {
            format!("Conversion unavailable: {warn_detail}")
        } else if preview.lossy {
            let prefix = tr_or_default(locale, "convert_archetype.lossy_warning", "Lossy change warning:");
            if warn_detail.is_empty() {
                format!("{prefix} drops {} cell(s), subcolumns, or grouping.", preview.dropped_cells)
            } else {
                format!("{prefix} {} dropped cell(s) — {warn_detail}", preview.dropped_cells)
            }
        } else if !warn_detail.is_empty() {
            format!("Lossless reshape ({} cells preserved): {warn_detail}", preview.preserved_cells)
        } else {
            format!("Lossless conversion: {} cells and {} exclusions preserved.", preview.preserved_cells, preview.preserved_exclusions)
        };
        let b_wid = builder.push_node(NodeSpec {
            id: WidgetId::from("convert.lossy_banner"),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: banner_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Banner, banner_msg.clone(), FocusScope::ModalDialog),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        let is_alert = preview.lossy || banner_blocked;
        let stroke = if is_alert { builder.palette.link_red } else { builder.palette.link_green };
        let fill = if is_alert { Rgba8::rgba(220, 38, 38, 28) } else { builder.palette.panel };
        builder.paint_fill(ZLayer::ModalDialog, Some(&b_wid), banner_rect, fill);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&b_wid), banner_rect, stroke, if is_alert { 2 } else { 1 });
        let banner_style = if is_alert { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
        paint_text_fitted(
            builder,
            &b_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: banner_rect,
                pad_x: 6,
                text: &banner_msg,
                style: &banner_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        y += i32::try_from(banner_h + 6).unwrap_or(32);
    }

    // 3. Checkboxes (`convert.allow_lossy` & `convert.preview_only`)
    if y + i32::try_from(row_h).unwrap_or(24) <= body_rect.bottom() {
        let half = (body_rect.width / 2).saturating_sub(4).max(120);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "convert.allow_lossy",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(body_rect.x, y, half, row_h),
                label: tr_or_default(locale, "convert_archetype.allow_lossy", "Allow lossy conversion"),
                keyboard_path: "convert.allow_lossy",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20),
                checked: st.allow_lossy,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("convert.toggle:allow_lossy") },
            },
        );
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "convert.preview_only",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(body_rect.x + i32::try_from(half + 6).unwrap_or(126), y, half, row_h),
                label: "Preview only (do not modify table)",
                keyboard_path: "convert.preview_only",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(21),
                checked: st.preview_only,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("import_paste"), payload: String::from("convert.toggle:preview_only") },
            },
        );
        y += i32::try_from(row_h + 6).unwrap_or(30);
    }

    // 4. Column-Mapping & Live Preview Table
    let rem_h = u32::try_from((body_rect.bottom() - y).max(0)).unwrap_or(0);
    if rem_h < 32 || body_rect.width < 80 {
        return;
    }
    let Some(dst_ds) = converted_ds else {
        return;
    };
    let prev_rect = LayoutRectPx::new(body_rect.x, y, body_rect.width, rem_h);
    build_convert_preview_panel(builder, parent, prev_rect, src_sheet, &dst_ds, &preview, st.target_kind, size_px);
}

#[allow(clippy::too_many_arguments)]
fn build_convert_preview_panel(
    builder: &mut LayoutBuilder<'_>,
    parent: WidgetId,
    prev_rect: LayoutRectPx,
    src_sheet: &DataSheet,
    dst_ds: &DataSheet,
    preview: &ConversionPreview,
    target_kind: TableKind,
    size_px: f64,
) {
    let map_rows = build_convert_mapping_rows(src_sheet, dst_ds, preview);
    let (dst_headers, _, dst_grid, _) = extract_datasheet_preview_grid(dst_ds, &builder.app.locale_preset);

    let item_h = scaled_px(builder, 22.0, 18, 40);
    let col_w = scaled_px(builder, 94.0, 72, 160);
    let total_rows = 1 + map_rows.len() + 1 + dst_grid.len().min(60);
    let content_h = (total_rows as u32).saturating_mul(item_h).saturating_add(16);
    let content_w = ((dst_headers.len().max(3) as u32).saturating_mul(col_w)).max(prev_rect.width);
    let scroll_id = String::from("scroll.convert.preview");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), prev_rect, [content_w, content_h], scroll_off, ScrollAxes::Both);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("convert.preview_panel"),
        parent: Some(parent),
        z_layer: ZLayer::ModalDialog,
        rect_px: prev_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Column Mapping and Live Preview", FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), prev_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), prev_rect, builder.palette.border, 1);

    let sb_r = if scroll_meta.has_vertical_overflow() { 14 } else { 2 };
    let sb_b = if scroll_meta.has_horizontal_overflow() { 14 } else { 2 };
    let clip =
        LayoutRectPx::new(prev_rect.x + 2, prev_rect.y + 2, prev_rect.width.saturating_sub(sb_r + 2), prev_rect.height.saturating_sub(sb_b + 2));
    builder.push_clip(clip);

    let x_scroll = scroll_meta.scroll_offset_px[0].round() as i32;
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let bold_style = UiTextStyle::ui_bold(size_px);
    let mut cur_y = clip.y - y_scroll;

    // Section A: Column Mapping header & rows
    if cur_y >= clip.y && cur_y + i32::try_from(item_h).unwrap_or(22) <= clip.bottom() {
        let hdr_r = LayoutRectPx::new(clip.x, cur_y, clip.width, item_h);
        builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), hdr_r, builder.palette.header);
        paint_text_fitted(
            builder,
            &panel_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: hdr_r,
                pad_x: 6,
                text: &format!(
                    "Column Mapping ({} -> {}) | Preserved: {} | Dropped: {}",
                    src_sheet.kind().label(),
                    target_kind.label(),
                    preview.preserved_cells,
                    preview.dropped_cells
                ),
                style: &bold_style,
                color: builder.palette.text,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    cur_y += i32::try_from(item_h).unwrap_or(22);

    for (idx, mrow) in map_rows.iter().enumerate() {
        if cur_y >= clip.y && cur_y + i32::try_from(item_h).unwrap_or(22) <= clip.bottom() {
            let row_rect = LayoutRectPx::new(clip.x, cur_y, clip.width, item_h);
            let lbl = format!("{}  ->  {}  [{}]", mrow.source_col, mrow.target_col, mrow.status);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("convert.map_row.{idx}"),
                    parent: Some(panel_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: row_rect,
                    label: &lbl,
                    keyboard_path: &format!("convert.map_row.{idx}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(30 + idx as u32),
                    primary: false,
                    selected: builder.app.view_shards.import_paste.convert_selected_col == idx,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("import_paste"), payload: format!("convert.map_row:{idx}") },
                },
            );
            if mrow.lossy {
                builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), row_rect, builder.palette.link_red, 1);
            }
        }
        cur_y += i32::try_from(item_h).unwrap_or(22);
    }

    // Section B: Live Converted Table Preview
    paint_convert_preview_grid(builder, &panel_wid, &scroll_id, clip, cur_y, x_scroll, item_h, col_w, size_px, &dst_headers, &dst_grid);

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(panel_wid));
}

#[allow(clippy::too_many_arguments)]
fn paint_convert_preview_grid(
    builder: &mut LayoutBuilder<'_>,
    panel_wid: &WidgetId,
    scroll_id: &str,
    clip: LayoutRectPx,
    mut cur_y: i32,
    x_scroll: i32,
    item_h: u32,
    col_w: u32,
    size_px: f64,
    dst_headers: &[String],
    dst_grid: &[Vec<Cell>],
) {
    let bold_style = UiTextStyle::ui_bold(size_px);
    let tab_style = UiTextStyle::grid_tabular(size_px);
    if cur_y >= clip.y && cur_y + i32::try_from(item_h).unwrap_or(22) <= clip.bottom() {
        let hdr_r = LayoutRectPx::new(clip.x, cur_y, clip.width, item_h);
        builder.paint_fill(ZLayer::ModalDialog, Some(panel_wid), hdr_r, builder.palette.header);
        for (c_idx, h_lbl) in dst_headers.iter().enumerate() {
            let cx = clip.x + i32::try_from(c_idx as u32 * col_w).unwrap_or(0) - x_scroll;
            if cx + i32::try_from(col_w).unwrap_or(94) <= clip.x || cx >= clip.right() {
                continue;
            }
            let h_rect = LayoutRectPx::new(cx, cur_y, col_w, item_h).intersect(clip);
            if h_rect.width >= 6 {
                builder.paint_stroke(ZLayer::ModalDialog, Some(panel_wid), h_rect, builder.palette.border, 1);
                paint_text_fitted(
                    builder,
                    panel_wid,
                    FittedTextSpec {
                        z_layer: ZLayer::ModalDialog,
                        rect: h_rect,
                        pad_x: 4,
                        text: h_lbl,
                        style: &bold_style,
                        color: builder.palette.text,
                        scroll_container: Some(scroll_id),
                    },
                );
            }
        }
    }
    cur_y += i32::try_from(item_h).unwrap_or(22);

    for row in dst_grid.iter().take(60) {
        if cur_y >= clip.y && cur_y + i32::try_from(item_h).unwrap_or(22) <= clip.bottom() {
            for (c_idx, cell) in row.iter().enumerate() {
                let cx = clip.x + i32::try_from(c_idx as u32 * col_w).unwrap_or(0) - x_scroll;
                if cx + i32::try_from(col_w).unwrap_or(94) <= clip.x || cx >= clip.right() {
                    continue;
                }
                let cell_rect = LayoutRectPx::new(cx, cur_y, col_w, item_h).intersect(clip);
                if cell_rect.width < 6 {
                    continue;
                }
                builder.paint_stroke(ZLayer::ModalDialog, Some(panel_wid), cell_rect, builder.palette.border, 1);
                let disp = format_cell_display(cell, &builder.app.locale_preset);
                if !disp.is_empty() {
                    let ink = if cell.excluded { builder.palette.excluded } else { builder.palette.text };
                    let style = if cell.excluded { tab_style.clone().italic() } else { tab_style.clone() };
                    paint_text_fitted(
                        builder,
                        panel_wid,
                        FittedTextSpec {
                            z_layer: ZLayer::ModalDialog,
                            rect: cell_rect,
                            pad_x: 4,
                            text: &disp,
                            style: &style,
                            color: ink,
                            scroll_container: Some(scroll_id),
                        },
                    );
                    if cell.excluded && cell_rect.width >= 10 {
                        let mid_y = cell_rect.center()[1];
                        builder.paint_line(ZLayer::ModalDialog, Some(panel_wid), [cell_rect.x + 3, mid_y], [cell_rect.right() - 3, mid_y], ink, 1);
                    }
                }
            }
        }
        cur_y += i32::try_from(item_h).unwrap_or(22);
    }
}
