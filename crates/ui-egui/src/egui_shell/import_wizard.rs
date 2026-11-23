//! Five-stage `Import & Paste Special` wizard (`680x520 pt`), `ImportReshape` (96/384 Plate Map
//! & Tidy Long Pivot), and `Convert Table Archetype...` modal in `egui 0.37` (`R15.4`,
//! `gui-layout-spec.md` §8.5).

use std::collections::BTreeMap;

use curvebook_doc::{Cell, CellCoord, CellValue, CiBound, LinkBorderColor, PlacementMode, ResultCell, TableKind};
use egui::{Align, Context, Key, Layout, Modifiers, Rect, RichText, Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::grid_view::{
    ImportOriginKind, ImportPasteWizardState, ImportReshapeMode, ImportWizardStage, format_cell_display, parse_localized_number_input,
};
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, widgets};

/// Five pipeline steps of the `Import & Paste Special` wizard (`gui-layout-spec.md` §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImportWizardStep {
    #[default]
    Source,
    Preview,
    FilterAndUnstack,
    PlacementAndReshape,
    Metadata,
}

impl ImportWizardStep {
    /// Ordered stepper stages rendered across the wizard header bar.
    pub const ALL: [Self; 5] = [Self::Source, Self::Preview, Self::FilterAndUnstack, Self::PlacementAndReshape, Self::Metadata];

    fn tab_id(self) -> &'static str {
        match self {
            Self::Source => "import_paste.tab.source",
            Self::Preview => "import_paste.tab.view",
            Self::FilterAndUnstack => "import_paste.tab.filter",
            Self::PlacementAndReshape => "import_paste.tab.placement",
            Self::Metadata => "import_paste.tab.notes",
        }
    }

    fn label(self, locale: &str) -> &'static str {
        match self {
            Self::Source => tr(locale, "import.step.1"),
            Self::Preview => tr(locale, "import.step.2"),
            Self::FilterAndUnstack => tr(locale, "import.step.3"),
            Self::PlacementAndReshape => tr(locale, "import.step.4"),
            Self::Metadata => tr(locale, "import.step.5"),
        }
    }

    fn sync_backend_stage(self) -> ImportWizardStage {
        match self {
            Self::Source | Self::Preview => ImportWizardStage::Source,
            Self::FilterAndUnstack => ImportWizardStage::Filter,
            Self::PlacementAndReshape => ImportWizardStage::Placement,
            Self::Metadata => ImportWizardStage::Provenance,
        }
    }

    fn from_backend_stage(stage: ImportWizardStage) -> Self {
        match stage {
            ImportWizardStage::Source => Self::Source,
            ImportWizardStage::Filter => Self::FilterAndUnstack,
            ImportWizardStage::Placement => Self::PlacementAndReshape,
            ImportWizardStage::Provenance => Self::Metadata,
        }
    }
}

/// Plate or block reshape mode offered in Stage 4 (`Placement & Plate/Block Reshape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WizardBlockReshape {
    #[default]
    None,
    RowWrapBlock,
    Plate96,
    Plate384,
}

/// User-saved default preset for the 5-stage `Import & Paste Special` wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportWizardSavedPreset {
    pub delimiter: char,
    pub decimal_separator: char,
    pub propagate_title_header: bool,
    pub parse_asterisk_exclusions: bool,
    pub encoding_name: String,
    pub missing_tokens_draft: String,
    pub filter_predicate_draft: String,
    pub placement_mode: PlacementMode,
    pub transpose: bool,
    pub skip_contiguous_blanks: bool,
    pub block_reshape: WizardBlockReshape,
    pub plate_values_per_row_draft: String,
    pub plate_rows_per_dataset_draft: String,
    pub record_in_info_sheet: bool,
    pub extract_info_metadata: bool,
    pub add_floating_note: bool,
}

/// Persistent transient UI state for `render_import_or_convert_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportWizardEguiState {
    pub active_step: ImportWizardStep,
    pub raw_source_draft: String,
    pub missing_tokens_draft: String,
    pub plate_values_per_row_draft: String,
    pub plate_rows_per_dataset_draft: String,
    pub extract_info_metadata: bool,
    pub add_floating_note: bool,
    pub encoding_name: String,
    pub row_start_draft: String,
    pub row_end_draft: String,
    pub col_start_draft: String,
    pub col_end_draft: String,
    pub filter_predicate_draft: String,
    pub decimate_draft: String,
    pub block_reshape: WizardBlockReshape,
    pub plate_values_draft: String,
    pub reshape_title_draft: String,
    pub reshape_x_title_draft: String,
    pub reshape_group_col_draft: String,
    pub reshape_value_col_draft: String,
    pub reshape_x_col_draft: String,
    pub reshape_replicate_col_draft: String,
    pub reshape_excluded_col_draft: String,
    pub reshape_tidy_csv_draft: String,
    pub saved_preset: Option<ImportWizardSavedPreset>,
    pub preset_saved_feedback: bool,
    pub initialized_for_open: bool,
}

/// Frame geometry and modal capture report emitted by `render_import_or_convert_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportWizardFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

struct ImportModalCtx<'a> {
    app: &'a UiApp,
    state: &'a mut ImportWizardEguiState,
    wiz: ImportPasteWizardState,
    wiz_dirty: bool,
    tokens: &'a Tokens,
    meta: &'a mut FrameInspectMeta,
    intents: &'a mut PendingUiIntents,
}

/// Render `Import & Paste Special` (`app.grid.import_wizard.open`), `ImportReshape`, or `ConvertArchetype`.
pub fn render_import_or_convert_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ImportWizardFrameReport {
    let is_wizard = app.grid.import_wizard.open;
    let is_reshape = matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape);
    let is_convert = matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. });
    if !is_wizard && !is_reshape && !is_convert {
        state.initialized_for_open = false;
        return ImportWizardFrameReport::default();
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if is_wizard {
            intents.push_close_import_wizard();
        } else {
            intents.push_dialog_button("cancel");
        }
    }

    let (wiz, mut wiz_dirty) = if !state.initialized_for_open {
        let res = sync_drafts_from_app(app, state);
        state.initialized_for_open = true;
        res
    } else {
        (app.grid.import_wizard.clone(), false)
    };

    let (widget_id, title) = if is_convert {
        ("dialog.convert_archetype", tr(tokens.locale, "cmd.table.convertArchetype"))
    } else if is_reshape {
        ("dialog.import_reshape", tr(tokens.locale, "w6.view.modal.import_reshape"))
    } else {
        ("dialog.import_paste_special", tr(tokens.locale, "menu.edit.paste_special"))
    };

    let vp = ctx.content_rect();
    let dlg_w = (vp.width() - 24.0).clamp(320.0, 680.0);
    let dlg_h = (vp.height() - 24.0).clamp(240.0, 520.0);

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_import_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (dlg_w - 24.0).max(280.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(dlg_h);

        ui.heading(RichText::new(title).color(tokens.text_primary));
        ui.add_space(4.0);

        let mut modal_cx = ImportModalCtx { app, state, wiz, wiz_dirty, tokens, meta, intents };
        if is_convert {
            render_convert_archetype_body(ui, &mut modal_cx, dlg_h - 88.0);
        } else if is_reshape {
            render_import_reshape_body(ui, &mut modal_cx, inner_w, dlg_h - 88.0);
        } else {
            render_five_stage_wizard_body(ui, &mut modal_cx, inner_w, dlg_h - 88.0);
        }

        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        render_wizard_footer(ui, &mut modal_cx, is_wizard, is_reshape);
        wiz_dirty = modal_cx.wiz_dirty;
        if wiz_dirty && is_wizard {
            modal_cx.intents.push_update_import_wizard(modal_cx.wiz);
        }
    });

    let title_for_ak = title.to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    ImportWizardFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn sync_drafts_from_app(app: &UiApp, state: &mut ImportWizardEguiState) -> (ImportPasteWizardState, bool) {
    let mut wiz = app.grid.import_wizard.clone();
    let mut wiz_dirty = false;
    state.active_step = ImportWizardStep::from_backend_stage(wiz.stage);
    state.preset_saved_feedback = false;
    state.raw_source_draft = if wiz.raw_clipboard_text.is_empty() && !app.view_shards.import_paste.raw_source_text.is_empty() {
        app.view_shards.import_paste.raw_source_text.clone()
    } else {
        wiz.raw_clipboard_text.clone()
    };
    state.row_start_draft = wiz.filter.row_window.map_or_else(String::new, |(a, _)| a.to_string());
    state.row_end_draft = wiz.filter.row_window.map_or_else(String::new, |(_, b)| b.to_string());
    state.col_start_draft = wiz.filter.col_window.map_or_else(String::new, |(a, _)| a.to_string());
    state.col_end_draft = wiz.filter.col_window.map_or_else(String::new, |(_, b)| b.to_string());
    state.decimate_draft = wiz.filter.decimate_every_k.map_or_else(String::new, |k| k.to_string());

    if let Some(preset) = state.saved_preset.clone() {
        state.encoding_name = preset.encoding_name;
        state.missing_tokens_draft = preset.missing_tokens_draft;
        state.filter_predicate_draft = preset.filter_predicate_draft;
        state.block_reshape = preset.block_reshape;
        state.plate_values_per_row_draft = preset.plate_values_per_row_draft;
        state.plate_rows_per_dataset_draft = preset.plate_rows_per_dataset_draft;
        state.extract_info_metadata = preset.extract_info_metadata;
        state.add_floating_note = preset.add_floating_note;
        wiz.delimiter = preset.delimiter;
        wiz.decimal_separator = preset.decimal_separator;
        wiz.placement.propagate_title_header = preset.propagate_title_header;
        wiz.filter.parse_asterisk_exclusions = preset.parse_asterisk_exclusions;
        wiz.placement.mode = preset.placement_mode;
        wiz.placement.transpose = preset.transpose;
        wiz.placement.skip_contiguous_blanks = preset.skip_contiguous_blanks;
        wiz.record_in_info_sheet = preset.record_in_info_sheet;
        wiz_dirty = true;
    } else {
        state.missing_tokens_draft = String::from("NA, NaN, .");
        state.encoding_name = String::from("UTF-8");
        state.plate_values_per_row_draft = String::from("12");
        state.plate_rows_per_dataset_draft = String::from("8");
        state.filter_predicate_draft.clear();
        state.block_reshape = WizardBlockReshape::None;
        state.extract_info_metadata = false;
        state.add_floating_note = false;
    }

    let ir = &app.grid.import_reshape;
    state.reshape_title_draft = ir.title.clone();
    state.reshape_x_title_draft = ir.x_title.clone();
    state.reshape_group_col_draft = ir.group_col.clone();
    state.reshape_value_col_draft = ir.value_col.clone();
    state.reshape_x_col_draft = ir.x_col.clone().unwrap_or_default();
    state.reshape_replicate_col_draft = ir.replicate_col.clone().unwrap_or_default();
    state.reshape_excluded_col_draft = ir.excluded_col.clone().unwrap_or_default();
    state.reshape_tidy_csv_draft = ir.tidy_csv.clone();
    state.plate_values_draft =
        ir.plate_values.iter().map(|v| v.map_or_else(|| String::from("NA"), |n| format!("{n}"))).collect::<Vec<_>>().join(", ");
    (wiz, wiz_dirty)
}

fn render_five_stage_wizard_body(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, avail_w: f32, body_h: f32) {
    // Persistent 5-stage horizontal stepper bar.
    ui.horizontal_wrapped(|bar| {
        for step in ImportWizardStep::ALL {
            let active = cx.state.active_step == step;
            let step_lbl = step.label(cx.tokens.locale);
            let resp = choice_pill(bar, cx.tokens, step_lbl, active, true);
            cx.meta.record_modal_response(bar, &resp, FrameInspectEntryInput::modal(step.tab_id(), "tab", step_lbl, Some(active.to_string()), true));
            if resp.clicked() {
                cx.state.active_step = step;
                cx.wiz.stage = step.sync_backend_stage();
                cx.wiz_dirty = true;
            }
        }
    });
    widgets::divider(ui);

    let scroll_h = (body_h - 34.0).max(100.0);
    egui::ScrollArea::vertical().id_salt("import_wizard_body_scroll").max_height(scroll_h).auto_shrink([false, false]).show(ui, |body| {
        match cx.state.active_step {
            ImportWizardStep::Source => render_step_source(body, cx, avail_w),
            ImportWizardStep::Preview => render_step_preview(body, cx.app, &cx.wiz, cx.state, cx.tokens, cx.meta),
            ImportWizardStep::FilterAndUnstack => render_step_filter_unstack(body, cx),
            ImportWizardStep::PlacementAndReshape => render_step_placement(body, cx),
            ImportWizardStep::Metadata => render_step_metadata(body, cx),
        }
    });
}

fn render_step_source(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, avail_w: f32) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let wiz = &mut cx.wiz;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut dirty = false;

    ui.label(RichText::new("Data Origin & Link Policy").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        for (kind, wid, label) in [
            (ImportOriginKind::Clipboard, "import_paste.source.clipboard", "Clipboard"),
            (ImportOriginKind::ExternalDelimitedFile, "import_paste.source.file", "CSV / TSV File"),
            (ImportOriginKind::InternalDataSheet, "import_paste.source.internal_data", "Internal Data Sheet"),
            (ImportOriginKind::InternalResultSheet, "import_paste.source.internal_result", "Result Sheet"),
        ] {
            let active = wiz.origin_kind == kind;
            let r = choice_pill(row, tokens, label, active, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", label, Some(active.to_string()), true));
            if r.clicked() {
                wiz.origin_kind = kind;
                if kind == ImportOriginKind::InternalDataSheet
                    && wiz.source_sheet_id.as_ref().is_none_or(|id| app.document().data_sheet(id).is_none())
                {
                    wiz.source_sheet_id = app.document().data_sheets.first().map(|d| d.id.clone());
                    wiz.source_tab_id = None;
                } else if kind == ImportOriginKind::InternalResultSheet
                    && wiz.source_sheet_id.as_ref().is_none_or(|id| app.document().result_sheet(id).is_none())
                {
                    let first_rs =
                        app.document().result_sheets.iter().find(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.rows.is_empty()));
                    wiz.source_sheet_id = first_rs.map(|rs| rs.id.clone());
                    wiz.source_tab_id =
                        first_rs.and_then(|rs| if rs.analyzable_data.is_some() { None } else { rs.tabs.first().map(|t| t.id.clone()) });
                }
                dirty = true;
            }
        }
    });

    ui.horizontal(|row| {
        let can_link = wiz.origin_kind != ImportOriginKind::Clipboard;
        let r = row.add_enabled(can_link, egui::Checkbox::new(&mut wiz.is_live_link, "Maintain Live Link (Provenance Badge)"));
        meta.record_modal_response(
            row,
            &r,
            FrameInspectEntryInput::modal(
                "import_paste.opt.live_link",
                "checkbox",
                "Maintain Live Link",
                Some(wiz.is_live_link.to_string()),
                can_link,
            ),
        );
        if r.changed() {
            dirty = true;
        }
        if let Some(col) = wiz.preview_border_color(Some(app.document())) {
            let badge_c = match col {
                LinkBorderColor::Green => tokens.link_green,
                LinkBorderColor::Red => tokens.link_red,
                LinkBorderColor::Blue => tokens.link_blue,
                LinkBorderColor::Charcoal => tokens.text_muted,
            };
            row.label(RichText::new(format!("Link color: {col:?}")).color(badge_c).strong());
        }
    });

    ui.add_space(6.0);
    if wiz.origin_kind == ImportOriginKind::ExternalDelimitedFile {
        ui.horizontal_wrapped(|row| {
            row.label("File Path:");
            let pr = row.add(
                egui::TextEdit::singleline(&mut wiz.external_file_path).hint_text("/path/to/data.csv").desired_width((avail_w - 90.0).max(160.0)),
            );
            meta.record_modal_response(
                row,
                &pr,
                FrameInspectEntryInput::modal("import_paste.input.path", "textbox", "External File Path", Some(wiz.external_file_path.clone()), true),
            );
            if pr.changed() {
                dirty = true;
            }
        });
    } else if wiz.origin_kind == ImportOriginKind::InternalDataSheet {
        ui.horizontal_wrapped(|row| {
            row.label("Source Sheet:");
            for ds in &app.document().data_sheets {
                let sel = wiz.source_sheet_id.as_ref() == Some(&ds.id);
                let sr = choice_pill(row, tokens, &ds.title, sel, true);
                meta.record_modal_response(
                    row,
                    &sr,
                    FrameInspectEntryInput::modal(
                        &format!("import_paste.source_sheet.{}", ds.id.as_str()),
                        "radio",
                        &ds.title,
                        Some(sel.to_string()),
                        true,
                    ),
                );
                if sr.clicked() {
                    wiz.source_sheet_id = Some(ds.id.clone());
                    wiz.source_tab_id = None;
                    dirty = true;
                }
            }
        });
    } else if wiz.origin_kind == ImportOriginKind::InternalResultSheet {
        let result_sheets = &app.document().result_sheets;
        ui.horizontal_wrapped(|row| {
            row.label("Source Result Sheet:");
            if result_sheets.is_empty() {
                row.label(RichText::new("No Result Sheets in project.").color(tokens.text_muted));
            }
            for rs in result_sheets {
                let eligible = rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.rows.is_empty());
                let sel = wiz.source_sheet_id.as_ref() == Some(&rs.id);
                let sr = choice_pill(row, tokens, rs.title(), sel, eligible);
                meta.record_modal_response(
                    row,
                    &sr,
                    FrameInspectEntryInput::modal(
                        &format!("import_paste.source_sheet.{}", rs.id.as_str()),
                        "radio",
                        rs.title(),
                        Some(sel.to_string()),
                        eligible,
                    ),
                );
                if sr.clicked() {
                    wiz.source_sheet_id = Some(rs.id.clone());
                    wiz.source_tab_id = if rs.analyzable_data.is_some() { None } else { rs.tabs.first().map(|t| t.id.clone()) };
                    dirty = true;
                }
            }
        });
        if let Some(sel_rs) = wiz.source_sheet_id.as_ref().and_then(|id| app.document().result_sheet(id))
            && sel_rs.analyzable_data.is_none()
            && sel_rs.tabs.len() > 1
        {
            ui.horizontal_wrapped(|row| {
                row.label("Result Tab:");
                for tab in &sel_rs.tabs {
                    let sel = wiz.source_tab_id.as_deref() == Some(tab.id.as_str());
                    let tr_btn = choice_pill(row, tokens, &tab.title, sel, !tab.rows.is_empty());
                    meta.record_modal_response(
                        row,
                        &tr_btn,
                        FrameInspectEntryInput::modal(
                            &format!("import_paste.source_tab.{}", tab.id),
                            "radio",
                            &tab.title,
                            Some(sel.to_string()),
                            !tab.rows.is_empty(),
                        ),
                    );
                    if tr_btn.clicked() {
                        wiz.source_tab_id = Some(tab.id.clone());
                        dirty = true;
                    }
                }
            });
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|row| {
        row.label("Delimited Source Text / Preview Feed:");
        render_paste_clipboard_button(row, meta, intents);
    });
    let tr_edit = ui.add(
        egui::TextEdit::multiline(&mut state.raw_source_draft)
            .hint_text("Paste CSV / TSV table rows…")
            .desired_rows(5)
            .desired_width((avail_w - 20.0).max(200.0)),
    );
    meta.record_modal_response(
        ui,
        &tr_edit,
        FrameInspectEntryInput::modal("import_paste.input.csv", "textbox", "Delimited Source Data", Some(state.raw_source_draft.clone()), true),
    );
    if tr_edit.changed() {
        wiz.raw_clipboard_text = state.raw_source_draft.clone();
        dirty = true;
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|row| {
        row.label("Delimiter:");
        for (delim, slug, lbl) in [('\t', "tab", "Tab"), (',', "comma", "Comma"), (';', "semi", "Semicolon"), (' ', "space", "Space")] {
            let dr = choice_pill(row, tokens, lbl, wiz.delimiter == delim, true);
            meta.record_modal_response(
                row,
                &dr,
                FrameInspectEntryInput::modal(&format!("import_paste.delim.{slug}"), "radio", lbl, Some((wiz.delimiter == delim).to_string()), true),
            );
            if dr.clicked() {
                wiz.delimiter = delim;
                dirty = true;
            }
        }

        row.separator();
        row.label("Decimal:");
        for (dec, slug, lbl) in [('.', "dot", "Period (.)"), (',', "comma", "Comma (,)")] {
            let r = choice_pill(row, tokens, lbl, wiz.decimal_separator == dec, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(
                    &format!("import_paste.decimal.{slug}"),
                    "radio",
                    lbl,
                    Some((wiz.decimal_separator == dec).to_string()),
                    true,
                ),
            );
            if r.clicked() {
                wiz.decimal_separator = dec;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        let mut hdr = wiz.placement.propagate_title_header;
        let hr = row.checkbox(&mut hdr, "First row contains column titles");
        meta.record_modal_response(
            row,
            &hr,
            FrameInspectEntryInput::modal("import_paste.opt.propagate_headers", "checkbox", "Header Row", Some(hdr.to_string()), true),
        );
        if hr.changed() {
            wiz.placement.propagate_title_header = hdr;
            dirty = true;
        }
        row.label("Encoding:");
        for (enc, slug) in [("UTF-8", "utf8"), ("UTF-16", "utf16"), ("Latin-1", "latin1")] {
            let active = state.encoding_name == enc;
            let er = choice_pill(row, tokens, enc, active, true);
            meta.record_modal_response(
                row,
                &er,
                FrameInspectEntryInput::modal(&format!("import_paste.encoding.{slug}"), "radio", enc, Some(active.to_string()), true),
            );
            if er.clicked() {
                state.encoding_name = enc.to_owned();
                state.preset_saved_feedback = false;
            }
        }
        row.label("Missing sentinels:");
        let mr = row.add(egui::TextEdit::singleline(&mut state.missing_tokens_draft).hint_text("NA, NaN, .").desired_width(110.0));
        meta.record_modal_response(
            row,
            &mr,
            FrameInspectEntryInput::modal(
                "import_paste.input.missing_tokens",
                "textbox",
                "Missing Sentinels",
                Some(state.missing_tokens_draft.clone()),
                true,
            ),
        );
        if mr.changed() {
            state.preset_saved_feedback = false;
        }
    });

    if dirty {
        state.preset_saved_feedback = false;
        cx.wiz_dirty = true;
    }
}

fn render_step_preview(
    ui: &mut Ui,
    app: &UiApp,
    wiz: &ImportPasteWizardState,
    state: &ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) {
    let prepared = prepare_wizard_import_data(app, wiz, state, true);
    let preview = &prepared.grid;
    let rows = preview.len();
    let cols = preview.iter().map(Vec::len).max().unwrap_or(0);
    let excluded_count: usize = preview.iter().flatten().filter(|c| c.excluded).count();

    let summary = format!("Parsed Preview: {rows} rows × {cols} cols · {excluded_count} excluded · Encoding: {}", state.encoding_name);
    let sr = ui.label(RichText::new(&summary).strong().color(tokens.text_primary));
    meta.record_modal_response(
        ui,
        &sr,
        FrameInspectEntryInput::modal("import_paste.preview.summary", "label", &summary, Some(summary.clone()), true),
    );
    ui.add_space(6.0);

    render_preview_grid_table(ui, app, preview, tokens);
}

fn render_step_filter_unstack(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let wiz = &mut cx.wiz;
    let meta = &mut *cx.meta;
    let mut dirty = false;

    ui.horizontal_wrapped(|row| {
        let r = row.checkbox(&mut wiz.filter.parse_asterisk_exclusions, "Treat * values (e.g. 45.6*) as excluded");
        meta.record_modal_response(
            row,
            &r,
            FrameInspectEntryInput::modal(
                "import_paste.opt.asterisk_excl",
                "checkbox",
                "Parse Asterisk Exclusions",
                Some(wiz.filter.parse_asterisk_exclusions.to_string()),
                true,
            ),
        );
        if r.changed() {
            dirty = true;
        }

        let mut unstack_on = wiz.filter.unstack_group_column.is_some();
        let ur = row.checkbox(&mut unstack_on, "Unstack / Pivot by group column");
        meta.record_modal_response(
            row,
            &ur,
            FrameInspectEntryInput::modal("import_paste.opt.unstack", "checkbox", "Unstack Enabled", Some(unstack_on.to_string()), true),
        );
        if ur.changed() {
            wiz.filter.unstack_group_column = if unstack_on { Some(0) } else { None };
            dirty = true;
        }

        if let Some(ref mut gcol) = wiz.filter.unstack_group_column {
            row.label("Group col:");
            if choice_pill(row, tokens, "-", false, *gcol > 0).clicked() {
                *gcol = gcol.saturating_sub(1);
                dirty = true;
            }
            row.label(format!("C{}", *gcol + 1));
            if choice_pill(row, tokens, "+", false, true).clicked() {
                *gcol += 1;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    let col_range_enabled = wiz.filter.unstack_group_column.is_none();
    ui.horizontal_wrapped(|row| {
        row.label("Row window:");
        let r0 = row.add(egui::TextEdit::singleline(&mut state.row_start_draft).hint_text("start").desired_width(48.0));
        meta.record_modal_response(
            row,
            &r0,
            FrameInspectEntryInput::modal("import_paste.input.row_start", "textbox", "Row Start", Some(state.row_start_draft.clone()), true),
        );
        let r1 = row.add(egui::TextEdit::singleline(&mut state.row_end_draft).hint_text("end").desired_width(48.0));
        meta.record_modal_response(
            row,
            &r1,
            FrameInspectEntryInput::modal("import_paste.input.row_end", "textbox", "Row End", Some(state.row_end_draft.clone()), true),
        );
        if r0.changed() || r1.changed() {
            wiz.filter.row_window = parse_opt_window(&state.row_start_draft, &state.row_end_draft);
            dirty = true;
        }

        row.separator();
        row.label("Col window:");
        let c0 = row.add_enabled(col_range_enabled, egui::TextEdit::singleline(&mut state.col_start_draft).hint_text("start").desired_width(48.0));
        meta.record_modal_response(
            row,
            &c0,
            FrameInspectEntryInput::modal(
                "import_paste.input.col_start",
                "textbox",
                "Column Start",
                Some(state.col_start_draft.clone()),
                col_range_enabled,
            ),
        );
        let c1 = row.add_enabled(col_range_enabled, egui::TextEdit::singleline(&mut state.col_end_draft).hint_text("end").desired_width(48.0));
        meta.record_modal_response(
            row,
            &c1,
            FrameInspectEntryInput::modal(
                "import_paste.input.col_end",
                "textbox",
                "Column End",
                Some(state.col_end_draft.clone()),
                col_range_enabled,
            ),
        );
        if !col_range_enabled {
            c0.clone().on_disabled_hover_text("Disabled while Unstack by group column is active");
        } else if c0.changed() || c1.changed() {
            wiz.filter.col_window = parse_opt_window(&state.col_start_draft, &state.col_end_draft);
            dirty = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|row| {
        row.label("Keep every K rows:");
        let dr = row.add(egui::TextEdit::singleline(&mut state.decimate_draft).hint_text("1").desired_width(48.0));
        meta.record_modal_response(
            row,
            &dr,
            FrameInspectEntryInput::modal(
                "import_paste.input.decimate",
                "textbox",
                "Decimate Every K Rows",
                Some(state.decimate_draft.clone()),
                true,
            ),
        );
        if dr.changed() {
            wiz.filter.decimate_every_k = state.decimate_draft.trim().parse::<usize>().ok().filter(|&k| k >= 1);
            dirty = true;
        }
        row.label("Text filter:");
        let fr = row.add(egui::TextEdit::singleline(&mut state.filter_predicate_draft).hint_text("substring…").desired_width(140.0));
        meta.record_modal_response(
            row,
            &fr,
            FrameInspectEntryInput::modal(
                "import_paste.input.filter_pattern",
                "textbox",
                "Row Filter Pattern",
                Some(state.filter_predicate_draft.clone()),
                true,
            ),
        );
        if fr.changed() {
            state.preset_saved_feedback = false;
        }
    });

    if wiz.is_live_link && wiz.origin_kind != ImportOriginKind::Clipboard && !state.filter_predicate_draft.trim().is_empty() {
        ui.label(
            RichText::new("Text substring filter applies to static paste / unlinked import.")
                .size(tokens.metrics.font_ui_small_pt)
                .color(tokens.text_muted),
        );
    }

    if dirty {
        state.preset_saved_feedback = false;
        cx.wiz_dirty = true;
    }
    ui.add_space(6.0);
    let preview = prepare_wizard_import_data(app, &cx.wiz, state, true).grid;
    render_preview_grid_table(ui, app, &preview, tokens);
}

fn render_step_placement(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let wiz = &mut cx.wiz;
    let meta = &mut *cx.meta;
    let mut dirty = false;

    ui.label(RichText::new("Target Table Placement").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        for (mode, slug, lbl) in [
            (PlacementMode::Overwrite, "overwrite", "Overwrite at Anchor"),
            (PlacementMode::InsertRows, "insert_rows", "Insert Rows"),
            (PlacementMode::AppendRows, "append_rows", "Append Rows Below"),
            (PlacementMode::AppendColumns, "append_cols", "Append Columns Right"),
        ] {
            let active = wiz.placement.mode == mode;
            let r = choice_pill(row, tokens, lbl, active, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("import_paste.place.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
            if r.clicked() {
                wiz.placement.mode = mode;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label(format!(
            "Anchor: Row {} · Col {} · Subcol {}",
            wiz.placement.anchor.row + 1,
            wiz.placement.anchor.col + 1,
            wiz.placement.anchor.subcol + 1
        ));
        if choice_pill(row, tokens, "Row-", false, wiz.placement.anchor.row > 0).clicked() {
            wiz.placement.anchor.row -= 1;
            dirty = true;
        }
        if choice_pill(row, tokens, "Row+", false, true).clicked() {
            wiz.placement.anchor.row += 1;
            dirty = true;
        }
        if choice_pill(row, tokens, "Col-", false, wiz.placement.anchor.col > 0).clicked() {
            wiz.placement.anchor.col -= 1;
            dirty = true;
        }
        if choice_pill(row, tokens, "Col+", false, true).clicked() {
            wiz.placement.anchor.col += 1;
            dirty = true;
        }
        if row.checkbox(&mut wiz.placement.transpose, "Transpose rows/cols").changed() {
            dirty = true;
        }
        if row.checkbox(&mut wiz.placement.skip_contiguous_blanks, "Skip blank rows").changed() {
            dirty = true;
        }
    });

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    ui.label(RichText::new("Plate & Row-Wrap Block Reshape").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        for (mode, slug, lbl) in [
            (WizardBlockReshape::None, "none", "Standard Grid"),
            (WizardBlockReshape::RowWrapBlock, "row_wrap", "Row-Wrap Block"),
            (WizardBlockReshape::Plate96, "plate96", "96-Well Plate (8×12)"),
            (WizardBlockReshape::Plate384, "plate384", "384-Well Plate (16×24)"),
        ] {
            let r = choice_pill(row, tokens, lbl, state.block_reshape == mode, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(
                    &format!("import_paste.reshape.{slug}"),
                    "radio",
                    lbl,
                    Some((state.block_reshape == mode).to_string()),
                    true,
                ),
            );
            if r.clicked() {
                state.block_reshape = mode;
                state.preset_saved_feedback = false;
                if mode == WizardBlockReshape::Plate96 {
                    state.plate_values_per_row_draft = String::from("12");
                    state.plate_rows_per_dataset_draft = String::from("8");
                } else if mode == WizardBlockReshape::Plate384 {
                    state.plate_values_per_row_draft = String::from("24");
                    state.plate_rows_per_dataset_draft = String::from("16");
                }
            }
        }
    });

    if state.block_reshape != WizardBlockReshape::None {
        ui.horizontal_wrapped(|row| {
            row.label("Values/Row:");
            let vr = row.add(egui::TextEdit::singleline(&mut state.plate_values_per_row_draft).hint_text("12").desired_width(56.0));
            meta.record_modal_response(
                row,
                &vr,
                FrameInspectEntryInput::modal(
                    "import_paste.input.plate_values_per_row",
                    "textbox",
                    "Values Per Row",
                    Some(state.plate_values_per_row_draft.clone()),
                    true,
                ),
            );
            row.label("Rows/DataSet:");
            let rr = row.add(egui::TextEdit::singleline(&mut state.plate_rows_per_dataset_draft).hint_text("8").desired_width(56.0));
            meta.record_modal_response(
                row,
                &rr,
                FrameInspectEntryInput::modal(
                    "import_paste.input.plate_rows_per_dataset",
                    "textbox",
                    "Rows Per DataSet",
                    Some(state.plate_rows_per_dataset_draft.clone()),
                    true,
                ),
            );
            if vr.changed() || rr.changed() {
                state.preset_saved_feedback = false;
            }
        });
    }

    if wiz.is_live_link && wiz.origin_kind != ImportOriginKind::Clipboard && state.block_reshape != WizardBlockReshape::None {
        ui.label(
            RichText::new("Block reshape applies to static paste / unlinked import.").size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted),
        );
    }

    if dirty {
        state.preset_saved_feedback = false;
        cx.wiz_dirty = true;
    }
    ui.add_space(6.0);
    let preview = prepare_wizard_import_data(app, &cx.wiz, state, true).grid;
    render_preview_grid_table(ui, app, &preview, tokens);
}

fn render_step_metadata(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let wiz = &mut cx.wiz;
    let meta = &mut *cx.meta;
    let mut dirty = false;

    ui.label(RichText::new("Provenance & Reproducibility Metadata").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        row.label("Link ID:");
        let lr = row.add(egui::TextEdit::singleline(&mut wiz.link_id).hint_text("Optional link ID").desired_width(140.0));
        meta.record_modal_response(
            row,
            &lr,
            FrameInspectEntryInput::modal("import_paste.input.link_id", "textbox", "Link ID", Some(wiz.link_id.clone()), true),
        );
        if lr.changed() {
            dirty = true;
        }
    });
    ui.label("Audit Note:");
    let nr = ui.add(
        egui::TextEdit::multiline(&mut wiz.provenance_note)
            .hint_text("Optional provenance note…")
            .desired_rows(3)
            .desired_width((ui.available_width() - 20.0).clamp(180.0, 480.0)),
    );
    meta.record_modal_response(
        ui,
        &nr,
        FrameInspectEntryInput::modal("import_paste.input.notes", "textbox", "Provenance Note", Some(wiz.provenance_note.clone()), true),
    );
    if nr.changed() {
        dirty = true;
    }

    let rec_chk = ui.checkbox(&mut wiz.record_in_info_sheet, "Record source lineage in project InfoSheet");
    meta.record_modal_response(
        ui,
        &rec_chk,
        FrameInspectEntryInput::modal(
            "import_paste.opt.record_info",
            "checkbox",
            "Record InfoSheet Lineage",
            Some(wiz.record_in_info_sheet.to_string()),
            true,
        ),
    );
    if rec_chk.changed() {
        dirty = true;
    }

    let ext_chk = ui.checkbox(&mut state.extract_info_metadata, "Extract key-value header lines into Info constants");
    meta.record_modal_response(
        ui,
        &ext_chk,
        FrameInspectEntryInput::modal(
            "import_paste.opt.extract_info_metadata",
            "checkbox",
            "Extract Header Constants",
            Some(state.extract_info_metadata.to_string()),
            true,
        ),
    );
    if ext_chk.changed() {
        state.preset_saved_feedback = false;
    }

    let note_chk = ui.checkbox(&mut state.add_floating_note, "Attach FloatingNote summary card onto target sheet");
    meta.record_modal_response(
        ui,
        &note_chk,
        FrameInspectEntryInput::modal(
            "import_paste.opt.add_floating_note",
            "checkbox",
            "Attach Summary Note",
            Some(state.add_floating_note.to_string()),
            true,
        ),
    );
    if note_chk.changed() {
        state.preset_saved_feedback = false;
    }

    if dirty {
        state.preset_saved_feedback = false;
        cx.wiz_dirty = true;
    }

    ui.add_space(8.0);
    ui.horizontal(|row| {
        let reset_r = widgets::secondary_button(row, "Reset Defaults");
        meta.record_modal_response(
            row,
            &reset_r,
            FrameInspectEntryInput::modal("import_paste.btn.reset_defaults", "button", "Reset Defaults", None, true),
        );
        if reset_r.clicked() {
            reset_wizard_defaults(state, &mut cx.wiz);
            cx.wiz_dirty = true;
            return;
        }
        let save_lbl = if state.preset_saved_feedback { "Preset Saved" } else { "Save Preset" };
        let save_r = widgets::secondary_button(row, save_lbl);
        meta.record_modal_response(row, &save_r, FrameInspectEntryInput::modal("import_paste.btn.save_default", "button", save_lbl, None, true));
        if save_r.clicked() {
            save_wizard_preset(state, &cx.wiz);
        } else if state.saved_preset.is_some() {
            row.label(RichText::new("Preset active").size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
        }
    });
}

fn render_paste_clipboard_button(ui: &mut Ui, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let pb = widgets::secondary_button(ui, "Paste Clipboard");
    meta.record_modal_response(ui, &pb, FrameInspectEntryInput::modal("import_paste.btn.paste_clipboard", "button", "Paste Clipboard", None, true));
    if pb.clicked() {
        intents.push_paste_clipboard_into_import_reshape();
    }
}

fn patch_import_reshape_field(intents: &mut PendingUiIntents, key: &str, value: Value) {
    let mut fields = Map::new();
    fields.insert(key.to_owned(), value);
    intents.push_patch_active_dialog(fields);
}

fn render_import_reshape_body(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, avail_w: f32, body_h: f32) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let ir = &app.grid.import_reshape;
    egui::ScrollArea::vertical().id_salt("import_reshape_scroll").max_width(avail_w).max_height(body_h).auto_shrink([false, false]).show(
        ui,
        |body| {
            body.set_max_width((avail_w - 18.0).max(200.0));
            body.horizontal_wrapped(|row| {
                for (mode, wid, lbl) in [
                    (ImportReshapeMode::Plate96, "import_paste.source.plate96", "96-Well Plate"),
                    (ImportReshapeMode::Plate384, "import_paste.source.plate384", "384-Well Plate"),
                    (ImportReshapeMode::TidyLongPivot, "import_paste.source.tidy", "Tidy Long Pivot"),
                ] {
                    let r = choice_pill(row, tokens, lbl, ir.mode == mode, true);
                    meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", lbl, Some((ir.mode == mode).to_string()), true));
                    if r.clicked() {
                        patch_import_reshape_field(
                            intents,
                            "mode",
                            Value::String(String::from(match mode {
                                ImportReshapeMode::Plate96 => "plate96",
                                ImportReshapeMode::Plate384 => "plate384",
                                ImportReshapeMode::TidyLongPivot => "tidylongpivot",
                            })),
                        );
                    }
                }
            });

            body.add_space(4.0);
            body.horizontal_wrapped(|row| {
                row.label("Target Kind:");
                for (kind, slug, wid, lbl) in [
                    (TableKind::Xy, "xy", "import_paste.target.xy", "XY"),
                    (TableKind::Column, "column", "import_paste.target.column", "Column"),
                    (TableKind::Grouped, "grouped", "import_paste.target.grouped", "Grouped"),
                    (TableKind::MultipleVariables, "multiple_variables", "import_paste.target.multiple_variables", "Multiple Variables"),
                ] {
                    let selected = ir.target_kind == kind;
                    let kr = choice_pill(row, tokens, lbl, selected, true);
                    meta.record_modal_response(row, &kr, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(selected.to_string()), true));
                    if kr.clicked() {
                        patch_import_reshape_field(intents, "target_kind", Value::String(String::from(slug)));
                    }
                }
            });

            body.add_space(6.0);
            body.horizontal_wrapped(|row| {
                row.label("Table Title:");
                let tr = row.add(egui::TextEdit::singleline(&mut state.reshape_title_draft).hint_text("Sheet title").desired_width(140.0));
                meta.record_modal_response(
                    row,
                    &tr,
                    FrameInspectEntryInput::modal(
                        "import_paste.input.title",
                        "textbox",
                        "Sheet Title",
                        Some(state.reshape_title_draft.clone()),
                        true,
                    ),
                );
                if tr.changed() {
                    patch_import_reshape_field(intents, "title", Value::String(state.reshape_title_draft.clone()));
                }
                row.label("X Title:");
                let xr = row.add(egui::TextEdit::singleline(&mut state.reshape_x_title_draft).hint_text("X column title").desired_width(110.0));
                meta.record_modal_response(
                    row,
                    &xr,
                    FrameInspectEntryInput::modal(
                        "import_paste.input.x_title",
                        "textbox",
                        "X Title",
                        Some(state.reshape_x_title_draft.clone()),
                        true,
                    ),
                );
                if xr.changed() {
                    patch_import_reshape_field(intents, "x_title", Value::String(state.reshape_x_title_draft.clone()));
                }
                render_paste_clipboard_button(row, meta, intents);
            });

            body.add_space(6.0);
            let mut sub_blank = ir.subtract_blank_mean;
            let sb_resp = body.checkbox(&mut sub_blank, "Subtract blank well mean automatically");
            meta.record_modal_response(
                body,
                &sb_resp,
                FrameInspectEntryInput::modal("import.subtract_blank_mean", "checkbox", "Subtract Blank Mean", Some(sub_blank.to_string()), true),
            );
            if sb_resp.changed() {
                intents.push_dialog_widget_click("import.subtract_blank_mean");
                let is_384 = ir.mode == ImportReshapeMode::Plate384;
                let expected = if is_384 { 384 } else { 96 };
                if sub_blank && (ir.plate_roles.len() != expected || !ir.plate_roles.iter().any(|r| r.as_str() == Some("Blank"))) {
                    patch_import_reshape_field(intents, "plate_roles", Value::Array(default_plate_roles(is_384, true)));
                }
            }

            if ir.mode == ImportReshapeMode::TidyLongPivot {
                body.add_space(4.0);
                body.horizontal_wrapped(|row| {
                    row.label("Delimiter:");
                    for (delim, wire, wid, lbl) in [
                        (',', "comma", "import_paste.delim.comma", "Comma"),
                        ('\t', "tab", "import_paste.delim.tab", "Tab"),
                        (';', "semicolon", "import_paste.delim.semi", "Semicolon"),
                    ] {
                        let sel = ir.tidy_delimiter == delim;
                        let dr = choice_pill(row, tokens, lbl, sel, true);
                        meta.record_modal_response(row, &dr, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(sel.to_string()), true));
                        if dr.clicked() {
                            patch_import_reshape_field(intents, "delimiter", Value::String(String::from(wire)));
                        }
                    }
                });

                body.add_space(4.0);
                body.horizontal_wrapped(|row| {
                    for (field_key, wid, lbl, hint, draft) in [
                        ("x_col", "import_paste.input.x_col", "X Col", "Optional", &mut state.reshape_x_col_draft),
                        ("group_col", "import_paste.input.group_col", "Group Col", "Group", &mut state.reshape_group_col_draft),
                        ("replicate_col", "import_paste.input.replicate_col", "Replicate Col", "Optional", &mut state.reshape_replicate_col_draft),
                        ("value_col", "import_paste.input.value_col", "Value Col", "Value", &mut state.reshape_value_col_draft),
                        ("excluded_col", "import_paste.input.excluded_col", "Excluded Col", "Optional", &mut state.reshape_excluded_col_draft),
                    ] {
                        row.label(format!("{lbl}:"));
                        let ed = row.add(egui::TextEdit::singleline(draft).id(egui::Id::unique(wid)).hint_text(hint).desired_width(82.0));
                        meta.record_modal_response(row, &ed, FrameInspectEntryInput::modal(wid, "textbox", lbl, Some(draft.clone()), true));
                        if ed.changed() {
                            patch_import_reshape_field(intents, field_key, Value::String(draft.clone()));
                        }
                    }
                });

                body.add_space(4.0);
                body.label("Delimited Tidy-Long CSV / TSV:");
                let csv_ed = body.add(
                    egui::TextEdit::multiline(&mut state.reshape_tidy_csv_draft)
                        .id(egui::Id::unique("import_paste.input.csv"))
                        .hint_text("Paste header + tidy-long rows (e.g. Group,Replicate,Value)...")
                        .desired_rows(5)
                        .desired_width((avail_w - 28.0).clamp(180.0, 560.0)),
                );
                meta.record_modal_response(
                    body,
                    &csv_ed,
                    FrameInspectEntryInput::modal(
                        "import_paste.input.csv",
                        "textbox",
                        "Delimited Tidy-Long CSV",
                        Some(state.reshape_tidy_csv_draft.clone()),
                        true,
                    ),
                );
                if csv_ed.changed() {
                    patch_import_reshape_field(intents, "tidy_csv", Value::String(state.reshape_tidy_csv_draft.clone()));
                }
            } else {
                let is_384 = ir.mode == ImportReshapeMode::Plate384;
                let expected = if is_384 { 384 } else { 96 };
                body.label(format!("Expected well count: {expected} (currently {} parsed values)", ir.plate_values.len()));
                let pr = body.add(
                    egui::TextEdit::multiline(&mut state.plate_values_draft)
                        .hint_text("Paste well numbers…")
                        .desired_rows(4)
                        .desired_width((avail_w - 28.0).clamp(180.0, 520.0)),
                );
                meta.record_modal_response(
                    body,
                    &pr,
                    FrameInspectEntryInput::modal(
                        "import_paste.input.plate_values",
                        "textbox",
                        "Plate Values",
                        Some(state.plate_values_draft.clone()),
                        true,
                    ),
                );
                if pr.changed() {
                    let mut f = Map::new();
                    f.insert(String::from("plate_values"), Value::String(state.plate_values_draft.clone()));
                    if ir.plate_roles.len() != expected {
                        f.insert(String::from("plate_roles"), Value::Array(default_plate_roles(is_384, ir.subtract_blank_mean)));
                    }
                    intents.push_patch_active_dialog(f);
                }
            }
        },
    );
}

fn render_convert_archetype_body(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, body_h: f32) {
    let app = cx.app;
    let tokens = cx.tokens;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let ca = &app.canvas_dialogs.convert_archetype;
    egui::ScrollArea::vertical().id_salt("convert_archetype_scroll").max_height(body_h).auto_shrink([false, false]).show(ui, |body| {
        body.label(RichText::new(format!("Source Archetype: {:?}", ca.source_kind.unwrap_or(TableKind::Column))).strong().color(tokens.text_primary));
        body.add_space(6.0);
        body.label("Target Table Archetype:");
        body.horizontal_wrapped(|row| {
            for (kind, slug, lbl) in [
                (TableKind::Xy, "xy", "Coordinate (XY)"),
                (TableKind::Column, "column", "Column"),
                (TableKind::Grouped, "grouped", "Grouped"),
                (TableKind::Contingency, "contingency", "Contingency"),
                (TableKind::Survival, "survival", "Survival"),
                (TableKind::PartsOfWhole, "parts_of_whole", "Parts of Whole"),
                (TableKind::MultipleVariables, "multiple_variables", "Multiple Variables"),
                (TableKind::Nested, "nested", "Nested"),
            ] {
                let active = ca.target_kind == kind;
                let r = choice_pill(row, tokens, lbl, active, true);
                meta.record_modal_response(
                    row,
                    &r,
                    FrameInspectEntryInput::modal(&format!("convert.target.{slug}"), "radio", lbl, Some(active.to_string()), true),
                );
                if r.clicked() {
                    let mut f = Map::new();
                    f.insert(String::from("target_kind"), Value::String(slug.to_owned()));
                    intents.push_patch_active_dialog(f);
                }
            }
        });

        body.add_space(6.0);
        let mut lossy = ca.allow_lossy;
        let lr = body.checkbox(&mut lossy, "Allow lossy reshape if structure drops subcolumns or X values");
        meta.record_modal_response(
            body,
            &lr,
            FrameInspectEntryInput::modal("convert.allow_lossy", "checkbox", "Allow Lossy Conversion", Some(lossy.to_string()), true),
        );
        if lr.changed() {
            intents.push_dialog_widget_click("convert.allow_lossy");
        }

        let mut prev_only = ca.preview_only;
        let pr = body.checkbox(&mut prev_only, "Preview mapping without mutating table");
        meta.record_modal_response(
            body,
            &pr,
            FrameInspectEntryInput::modal("convert.preview_only", "checkbox", "Preview Only", Some(prev_only.to_string()), true),
        );
        if pr.changed() {
            intents.push_dialog_widget_click("convert.preview_only");
        }

        if ca.preview_ready {
            body.add_space(6.0);
            egui::Frame::new()
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, if ca.preview_lossy { tokens.link_red } else { tokens.border }))
                .corner_radius(4.0)
                .inner_margin(vec2(8.0, 6.0))
                .show(body, |box_ui| {
                    box_ui.label(
                        RichText::new(format!(
                            "Preserved cells: {} · Dropped cells: {} · Preserved exclusions: {}",
                            ca.preserved_cells, ca.dropped_cells, ca.preserved_exclusions
                        ))
                        .color(tokens.text_primary),
                    );
                    for w in &ca.preview_warnings {
                        box_ui.label(RichText::new(format!("Warning: {w}")).color(tokens.link_red).size(11.5));
                    }
                });
        }
    });
}

fn render_wizard_footer(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, is_wizard: bool, is_reshape: bool) {
    let tokens = cx.tokens;
    ui.horizontal(|footer| {
        let cancel_lbl = tr(tokens.locale, "btn.cancel");
        let cancel_resp = widgets::secondary_button(footer, cancel_lbl);
        cx.meta.record_modal_response(footer, &cancel_resp, FrameInspectEntryInput::modal("modal.cancel", "button", cancel_lbl, None, true));
        if cancel_resp.clicked() {
            if is_wizard {
                cx.intents.push_close_import_wizard();
                cx.wiz_dirty = false;
            } else {
                cx.intents.push_dialog_button("cancel");
            }
        }

        footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
            let finish_lbl = if is_wizard { tr(tokens.locale, "btn.import") } else { tr(tokens.locale, "btn.ok") };
            let ok_resp = widgets::primary_button(rhs, finish_lbl);
            cx.meta.record_modal_response(rhs, &ok_resp, FrameInspectEntryInput::modal("modal.ok", "button", finish_lbl, None, true));
            if ok_resp.clicked() {
                if is_wizard {
                    commit_wizard_import(cx);
                } else {
                    cx.intents.push_dialog_button("ok");
                }
            }

            if is_wizard {
                let idx = ImportWizardStep::ALL.iter().position(|s| *s == cx.state.active_step).unwrap_or(0);
                let has_next = idx + 1 < ImportWizardStep::ALL.len();
                let next_lbl = tr(tokens.locale, "import.btn.next");
                let next_r = rhs.add_enabled(has_next, egui::Button::new(next_lbl));
                cx.meta.record_modal_response(rhs, &next_r, FrameInspectEntryInput::modal("modal.next", "button", next_lbl, None, has_next));
                if next_r.clicked() && has_next {
                    cx.state.active_step = ImportWizardStep::ALL[idx + 1];
                    cx.wiz.stage = cx.state.active_step.sync_backend_stage();
                    cx.wiz_dirty = true;
                }

                let has_back = idx > 0;
                let back_lbl = tr(tokens.locale, "import.btn.back");
                let back_r = rhs.add_enabled(has_back, egui::Button::new(back_lbl));
                cx.meta.record_modal_response(rhs, &back_r, FrameInspectEntryInput::modal("modal.back", "button", back_lbl, None, has_back));
                if back_r.clicked() && has_back {
                    cx.state.active_step = ImportWizardStep::ALL[idx - 1];
                    cx.wiz.stage = cx.state.active_step.sync_backend_stage();
                    cx.wiz_dirty = true;
                }
            } else if is_reshape {
                let _ = rhs.label(RichText::new("Single atomic commit").size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
            }
        });
    });
}

fn save_wizard_preset(state: &mut ImportWizardEguiState, wiz: &ImportPasteWizardState) {
    state.saved_preset = Some(ImportWizardSavedPreset {
        delimiter: wiz.delimiter,
        decimal_separator: wiz.decimal_separator,
        propagate_title_header: wiz.placement.propagate_title_header,
        parse_asterisk_exclusions: wiz.filter.parse_asterisk_exclusions,
        encoding_name: state.encoding_name.clone(),
        missing_tokens_draft: state.missing_tokens_draft.clone(),
        filter_predicate_draft: state.filter_predicate_draft.clone(),
        placement_mode: wiz.placement.mode,
        transpose: wiz.placement.transpose,
        skip_contiguous_blanks: wiz.placement.skip_contiguous_blanks,
        block_reshape: state.block_reshape,
        plate_values_per_row_draft: state.plate_values_per_row_draft.clone(),
        plate_rows_per_dataset_draft: state.plate_rows_per_dataset_draft.clone(),
        record_in_info_sheet: wiz.record_in_info_sheet,
        extract_info_metadata: state.extract_info_metadata,
        add_floating_note: state.add_floating_note,
    });
    state.preset_saved_feedback = true;
}

fn reset_wizard_defaults(state: &mut ImportWizardEguiState, wiz: &mut ImportPasteWizardState) {
    state.saved_preset = None;
    state.preset_saved_feedback = false;
    state.missing_tokens_draft = String::from("NA, NaN, .");
    state.encoding_name = String::from("UTF-8");
    state.row_start_draft.clear();
    state.row_end_draft.clear();
    state.col_start_draft.clear();
    state.col_end_draft.clear();
    state.filter_predicate_draft.clear();
    state.decimate_draft.clear();
    state.block_reshape = WizardBlockReshape::None;
    state.plate_values_per_row_draft = String::from("12");
    state.plate_rows_per_dataset_draft = String::from("8");
    state.extract_info_metadata = false;
    state.add_floating_note = false;
    *wiz = ImportPasteWizardState {
        open: true,
        stage: state.active_step.sync_backend_stage(),
        raw_clipboard_text: wiz.raw_clipboard_text.clone(),
        target_sheet_id: wiz.target_sheet_id.clone(),
        ..ImportPasteWizardState::default()
    };
}

fn sync_wizard_filter_from_drafts(wiz: &mut ImportPasteWizardState, state: &ImportWizardEguiState) {
    wiz.filter.row_window = parse_opt_window(&state.row_start_draft, &state.row_end_draft);
    if wiz.filter.unstack_group_column.is_none() {
        wiz.filter.col_window = parse_opt_window(&state.col_start_draft, &state.col_end_draft);
    }
    wiz.filter.decimate_every_k = state.decimate_draft.trim().parse::<usize>().ok().filter(|&k| k >= 1);
    if !state.raw_source_draft.is_empty() {
        wiz.raw_clipboard_text = state.raw_source_draft.clone();
    }
}

fn commit_wizard_import(cx: &mut ImportModalCtx<'_>) {
    sync_wizard_filter_from_drafts(&mut cx.wiz, cx.state);
    let doc = cx.app.document();
    let prepared = prepare_wizard_import_data(cx.app, &cx.wiz, cx.state, false);
    let row_count = prepared.grid.len();
    let col_count = prepared.grid.iter().map(Vec::len).max().unwrap_or(0);

    let is_live_link = cx.wiz.is_live_link && cx.wiz.origin_kind != ImportOriginKind::Clipboard;
    let is_empty_file_import = doc.data_sheets.is_empty()
        && cx.wiz.origin_kind == ImportOriginKind::ExternalDelimitedFile
        && !cx.wiz.is_live_link
        && cx.wiz.raw_clipboard_text.trim().is_empty()
        && !cx.wiz.external_file_path.trim().is_empty();

    let mut commit_wiz = cx.wiz.clone();
    if !is_live_link && !is_empty_file_import {
        commit_wiz.raw_clipboard_text = serialize_grid_for_import(
            &prepared.grid,
            commit_wiz.delimiter,
            commit_wiz.decimal_separator,
            commit_wiz.filter.parse_asterisk_exclusions,
        );
        // Clear transforms already baked into `raw_clipboard_text` so `table.paste` does not double-apply.
        commit_wiz.filter.row_window = None;
        commit_wiz.filter.col_window = None;
        commit_wiz.filter.decimate_every_k = None;
        commit_wiz.filter.unstack_group_column = None;
        commit_wiz.placement.transpose = false;
        commit_wiz.placement.skip_contiguous_blanks = false;
    }

    cx.intents.push_update_import_wizard(commit_wiz);
    cx.intents.push_commit_import_wizard(true);
    cx.wiz_dirty = false;

    let target_ds_id = cx
        .wiz
        .target_sheet_id
        .as_ref()
        .filter(|id| doc.data_sheet(id).is_some())
        .or_else(|| doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()))
        .or_else(|| doc.data_sheets.first().map(|d| &d.id));
    let info_target_id = target_ds_id
        .and_then(|did| doc.data_sheet(did).and_then(|d| d.info_sheet_id.as_ref()))
        .filter(|iid| doc.info_sheet(iid).is_some())
        .or_else(|| doc.info_sheets.first().map(|i| &i.id));

    if let Some(info_id) = info_target_id {
        for (key, val) in prepared.extracted_kv {
            cx.intents.push_engine(
                "info.setConstant",
                json!({
                    "sheet_id": info_id.as_str(),
                    "name": key,
                    "value": val,
                }),
            );
        }

        let mut note_parts = Vec::new();
        if !is_live_link && cx.wiz.record_in_info_sheet && !cx.wiz.provenance_note.trim().is_empty() {
            note_parts.push(cx.wiz.provenance_note.trim().to_owned());
        }
        if cx.state.add_floating_note {
            let mut summary = format!(
                "[Import Summary] {row_count} rows x {col_count} cols · Origin: {:?} · Encoding: {} · Reshape: {:?}",
                cx.wiz.origin_kind, cx.state.encoding_name, cx.state.block_reshape
            );
            if !cx.wiz.provenance_note.trim().is_empty() && note_parts.is_empty() {
                summary.push_str(&format!(" · Note: {}", cx.wiz.provenance_note.trim()));
            }
            note_parts.push(summary);
        }
        if !note_parts.is_empty() {
            cx.intents.push_engine(
                "info.setNote",
                json!({
                    "sheet_id": info_id.as_str(),
                    "text": note_parts.join("\n"),
                    "append": true,
                }),
            );
        }
    }
}

fn parse_opt_window(start_s: &str, end_s: &str) -> Option<(usize, usize)> {
    let a = start_s.trim().parse::<usize>().ok()?;
    let b = end_s.trim().parse::<usize>().ok()?;
    (a <= b).then_some((a, b))
}

struct PreparedWizardImport {
    grid: Vec<Vec<Cell>>,
    extracted_kv: Vec<(String, Value)>,
}

fn normalize_source_text(raw: &str, encoding_name: &str) -> String {
    let mut text = raw.replace("\r\n", "\n").replace('\r', "\n");
    if let Some(stripped) = text.strip_prefix('\u{FEFF}') {
        text = stripped.to_owned();
    }
    match encoding_name {
        "UTF-16" => {
            text.retain(|c| c != '\0' && c != '\u{FFFD}' && c != '\u{FEFF}');
        }
        "Latin-1" => {
            text = text
                .replace('\u{00A0}', " ")
                .replace(['\u{2018}', '\u{2019}'], "'")
                .replace(['\u{201C}', '\u{201D}'], "\"")
                .replace(['\u{2013}', '\u{2014}'], "-");
        }
        _ => {}
    }
    text
}

fn parse_kv_constant_value(raw_val: &str, preset: &crate::i18n::LocalePreset) -> Value {
    let trimmed = raw_val.trim();
    if let Some(n) = parse_localized_number_input(trimmed, preset).filter(|v| v.is_finite()) {
        json!(n)
    } else if let Ok(n) = trimmed.parse::<f64>()
        && n.is_finite()
    {
        json!(n)
    } else {
        json!(trimmed)
    }
}

fn try_parse_kv_line(raw_line: &str, delimiter: char, allow_unprefixed: bool) -> Option<(String, String)> {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let is_comment = trimmed.starts_with('#') || trimmed.starts_with("//");
    if !is_comment && (!allow_unprefixed || (delimiter != ' ' && trimmed.contains(delimiter))) {
        return None;
    }
    let body = trimmed.trim_start_matches('#').trim_start_matches('/').trim();
    let sep_pos = match (body.find(':'), body.find('=')) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }?;
    let key = body[..sep_pos].trim();
    let val = body[sep_pos + 1..].trim();
    if key.is_empty() || key.len() > 64 || val.is_empty() || body.contains("://") {
        return None;
    }
    let first_ch = key.chars().next()?;
    if first_ch.is_ascii_digit() || first_ch == '-' || first_ch == '+' {
        return None;
    }
    Some((key.to_owned(), val.to_owned()))
}

fn extract_kv_metadata_and_strip<'a>(
    text: &'a str,
    delimiter: char,
    extract_enabled: bool,
    preset: &crate::i18n::LocalePreset,
) -> (Vec<&'a str>, Vec<(String, Value)>) {
    let mut data_lines = Vec::new();
    let mut kv_pairs = Vec::new();
    let mut in_leading_block = true;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if extract_enabled && let Some((k, v)) = try_parse_kv_line(line, delimiter, in_leading_block) {
            kv_pairs.push((k, parse_kv_constant_value(&v, preset)));
            continue;
        }
        in_leading_block = false;
        data_lines.push(line);
    }
    (data_lines, kv_pairs)
}

fn prepare_wizard_import_data(app: &UiApp, wiz: &ImportPasteWizardState, state: &ImportWizardEguiState, preview_cap: bool) -> PreparedWizardImport {
    let missing_tokens: Vec<&str> = state.missing_tokens_draft.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut extracted_kv = Vec::new();
    let mut grid = match wiz.origin_kind {
        ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile => {
            let src_raw = if !state.raw_source_draft.is_empty() { state.raw_source_draft.as_str() } else { wiz.raw_clipboard_text.as_str() };
            let normalized = normalize_source_text(src_raw, &state.encoding_name);
            let (data_lines, kvs) = extract_kv_metadata_and_strip(&normalized, wiz.delimiter, state.extract_info_metadata, &app.locale_preset);
            extracted_kv = kvs;
            data_lines
                .into_iter()
                .map(|line| parse_delimited_line(line, wiz.delimiter, wiz.decimal_separator, wiz.filter.parse_asterisk_exclusions, &missing_tokens))
                .collect::<Vec<Vec<Cell>>>()
        }
        ImportOriginKind::InternalDataSheet => read_internal_data_sheet_grid(app, wiz, &missing_tokens, preview_cap),
        ImportOriginKind::InternalResultSheet => read_internal_result_sheet_grid(app, wiz),
    };

    apply_wizard_grid_transforms(&mut grid, app, wiz, state);
    PreparedWizardImport { grid, extracted_kv }
}

fn parse_delimited_line(line: &str, delimiter: char, decimal_sep: char, parse_asterisk: bool, missing_tokens: &[&str]) -> Vec<Cell> {
    line.split(delimiter)
        .map(|tok| {
            let trimmed = tok.trim();
            if missing_tokens.iter().any(|m| m.eq_ignore_ascii_case(trimmed)) {
                Cell::empty()
            } else {
                Cell::parse_import_token(trimmed, decimal_sep, parse_asterisk)
            }
        })
        .collect()
}

fn read_internal_data_sheet_grid(app: &UiApp, wiz: &ImportPasteWizardState, missing_tokens: &[&str], preview_cap: bool) -> Vec<Vec<Cell>> {
    let Some(sid) = wiz.source_sheet_id.as_ref() else {
        return Vec::new();
    };
    let Some(ds) = app.document().data_sheet(sid) else {
        return Vec::new();
    };
    let max_r = if preview_cap { ds.archetype.row_count().min(128) } else { ds.archetype.row_count() };
    let max_c = if preview_cap { ds.archetype.column_count().min(24) } else { ds.archetype.column_count() };
    let mut rows = Vec::with_capacity(max_r);
    for r in 0..max_r {
        let mut row = Vec::with_capacity(max_c);
        for c in 0..max_c {
            let mut cell = ds.archetype.data_cell(CellCoord::new(r, c, 0)).cloned().unwrap_or_else(Cell::empty);
            if let CellValue::Text(ref s) = cell.value
                && missing_tokens.iter().any(|m| m.eq_ignore_ascii_case(s.trim()))
            {
                cell = Cell::empty();
            }
            row.push(cell);
        }
        rows.push(row);
    }
    rows
}

fn read_internal_result_sheet_grid(app: &UiApp, wiz: &ImportPasteWizardState) -> Vec<Vec<Cell>> {
    let Some(sid) = wiz.source_sheet_id.as_ref() else {
        return Vec::new();
    };
    let Some(rs) = app.document().result_sheet(sid) else {
        return Vec::new();
    };
    if let Some(ds) = rs.analyzable_data.as_ref() {
        return (0..ds.archetype.row_count())
            .map(|r| {
                (0..ds.archetype.column_count())
                    .map(|c| ds.archetype.data_cell(CellCoord::new(r, c, 0)).cloned().unwrap_or_else(Cell::empty))
                    .collect()
            })
            .collect();
    }
    let Some(tab) = wiz.source_tab_id.as_deref().and_then(|tid| rs.tabs.iter().find(|t| t.id == tid)).or_else(|| rs.tabs.first()) else {
        return Vec::new();
    };
    let mut grid = Vec::with_capacity(tab.rows.len() + 1);
    if !tab.column_headers.is_empty() {
        grid.push(tab.column_headers.iter().map(|h| Cell::text(h.clone())).collect());
    }
    for row in &tab.rows {
        let cells = row
            .iter()
            .map(|rc| match rc {
                ResultCell::Empty => Cell::empty(),
                ResultCell::Number { value, .. } => Cell::number(*value),
                ResultCell::PValue { raw, .. } => Cell::number(*raw),
                ResultCell::ConfidenceInterval { lower, upper } => {
                    let lo = match lower {
                        CiBound::Finite(v) => format!("{v}"),
                        CiBound::Open => String::from("-inf"),
                    };
                    let hi = match upper {
                        CiBound::Finite(v) => format!("{v}"),
                        CiBound::Open => String::from("+inf"),
                    };
                    Cell::text(format!("[{lo}, {hi}]"))
                }
                ResultCell::Text(s) => Cell::text(s.clone()),
                ResultCell::Boolean(b) => Cell::text(b.to_string()),
            })
            .collect();
        grid.push(cells);
    }
    grid
}

fn apply_wizard_grid_transforms(grid: &mut Vec<Vec<Cell>>, app: &UiApp, wiz: &ImportPasteWizardState, state: &ImportWizardEguiState) {
    if !state.filter_predicate_draft.trim().is_empty() {
        let needle = state.filter_predicate_draft.trim().to_ascii_lowercase();
        grid.retain(|row| row.iter().any(|c| format_cell_display(c, &app.locale_preset).to_ascii_lowercase().contains(&needle)));
    }
    if let Some((r0, r1)) = wiz.filter.row_window {
        if r0 <= r1 && r0 < grid.len() {
            *grid = grid[r0..=r1.min(grid.len() - 1)].to_vec();
        } else {
            grid.clear();
        }
    }
    if let Some(gcol) = wiz.filter.unstack_group_column {
        *grid = unstack_preview(grid, gcol, &app.locale_preset);
    } else if let Some((c0, c1)) = wiz.filter.col_window {
        for row in grid.iter_mut() {
            if c0 <= c1 && c0 < row.len() {
                *row = row[c0..=c1.min(row.len() - 1)].to_vec();
            } else {
                row.clear();
            }
        }
    }
    if let Some(k) = wiz.filter.decimate_every_k
        && k >= 1
    {
        *grid = std::mem::take(grid).into_iter().step_by(k).collect();
    }
    if wiz.placement.skip_contiguous_blanks {
        grid.retain(|row| row.iter().any(|c| !c.is_empty()));
    }
    if wiz.placement.transpose {
        *grid = curvebook_doc::transpose_cell_grid(grid);
    }
    if state.block_reshape != WizardBlockReshape::None {
        *grid = reshape_block_grid(std::mem::take(grid), state);
    }
}

fn reshape_block_grid(grid: Vec<Vec<Cell>>, state: &ImportWizardEguiState) -> Vec<Vec<Cell>> {
    let (width, rows_per_ds) = match state.block_reshape {
        WizardBlockReshape::Plate96 => (12usize, 8usize),
        WizardBlockReshape::Plate384 => (24usize, 16usize),
        WizardBlockReshape::RowWrapBlock => (
            state.plate_values_per_row_draft.trim().parse::<usize>().unwrap_or(12).clamp(1, 64),
            state.plate_rows_per_dataset_draft.trim().parse::<usize>().unwrap_or(8).clamp(1, 512),
        ),
        WizardBlockReshape::None => return grid,
    };
    let flat: Vec<Cell> = grid.into_iter().flatten().collect();
    if flat.is_empty() {
        return Vec::new();
    }
    let block_cap = width.saturating_mul(rows_per_ds).max(width);
    let blocks: Vec<&[Cell]> = flat.chunks(block_cap).collect();
    if blocks.len() <= 1 {
        return flat.chunks(width).map(<[Cell]>::to_vec).collect();
    }
    let out_rows = blocks.iter().map(|b| b.len().div_ceil(width)).max().unwrap_or(0);
    let mut out = vec![vec![Cell::empty(); blocks.len() * width]; out_rows];
    for (b_idx, block) in blocks.iter().enumerate() {
        for (r_idx, row_slice) in block.chunks(width).enumerate() {
            for (c_idx, cell) in row_slice.iter().enumerate() {
                if let Some(slot) = out.get_mut(r_idx).and_then(|r| r.get_mut(b_idx * width + c_idx)) {
                    *slot = cell.clone();
                }
            }
        }
    }
    out
}

fn format_cell_for_import_text(cell: &Cell, decimal_sep: char, preserve_asterisk: bool) -> String {
    let mut s = match &cell.value {
        CellValue::Empty => return String::new(),
        CellValue::Number(v) if v.is_finite() => {
            let n = if *v == 0.0 { 0.0 } else { *v };
            let raw = format!("{n}");
            if decimal_sep != '.' { raw.replace('.', &decimal_sep.to_string()) } else { raw }
        }
        CellValue::Number(_) => return String::new(),
        CellValue::Text(t) => t.clone(),
    };
    if preserve_asterisk && cell.excluded && !s.ends_with('*') {
        s.push('*');
    }
    s
}

fn serialize_grid_for_import(grid: &[Vec<Cell>], delimiter: char, decimal_sep: char, preserve_asterisk: bool) -> String {
    let delim = if delimiter == '\0' { String::from("\t") } else { delimiter.to_string() };
    grid.iter()
        .map(|row| row.iter().map(|c| format_cell_for_import_text(c, decimal_sep, preserve_asterisk)).collect::<Vec<_>>().join(&delim))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unstack_preview(grid: &[Vec<Cell>], group_col: usize, preset: &crate::i18n::LocalePreset) -> Vec<Vec<Cell>> {
    if grid.is_empty() {
        return Vec::new();
    }
    let val_col = if group_col == 0 { 1 } else { 0 };
    let mut grouped: BTreeMap<String, Vec<Cell>> = BTreeMap::new();
    for row in grid {
        let key = row.get(group_col).map(|c| format_cell_display(c, preset)).unwrap_or_default();
        if let Some(val) = row.get(val_col) {
            grouped.entry(key).or_default().push(val.clone());
        }
    }
    let headers: Vec<Cell> = grouped.keys().map(|k| Cell { value: CellValue::Text(k.clone()), excluded: false }).collect();
    let max_rows = grouped.values().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![headers];
    for r in 0..max_rows {
        let mut row_cells = Vec::with_capacity(grouped.len());
        for col_vec in grouped.values() {
            row_cells.push(col_vec.get(r).cloned().unwrap_or_else(Cell::empty));
        }
        out.push(row_cells);
    }
    out
}

fn render_preview_grid_table(ui: &mut Ui, app: &UiApp, grid: &[Vec<Cell>], tokens: &Tokens) {
    let max_rows = grid.len().min(30);
    let max_cols = grid.iter().map(Vec::len).max().unwrap_or(0).min(12);
    egui::Frame::new().fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).corner_radius(4.0).inner_margin(vec2(6.0, 4.0)).show(
        ui,
        |box_ui| {
            if max_rows == 0 || max_cols == 0 {
                box_ui.label(RichText::new("No preview rows parsed.").color(tokens.text_muted));
                return;
            }
            egui::ScrollArea::horizontal().id_salt("import_preview_hscroll").auto_shrink([false, true]).show(box_ui, |hscroll| {
                egui::Grid::new("import_preview_table").striped(true).spacing(vec2(10.0, 3.0)).show(hscroll, |tbl| {
                    tbl.label(RichText::new("#").strong().color(tokens.text_muted));
                    for c in 0..max_cols {
                        tbl.label(RichText::new(format!("Col {}", c + 1)).strong().color(tokens.text_muted));
                    }
                    tbl.end_row();
                    for (r, row) in grid.iter().take(max_rows).enumerate() {
                        tbl.label(RichText::new(format!("{}", r + 1)).color(tokens.text_muted));
                        for c in 0..max_cols {
                            let cell_opt = row.get(c);
                            let txt = cell_opt.map(|cell| format_cell_display(cell, &app.locale_preset)).unwrap_or_default();
                            let ex = cell_opt.is_some_and(|cell| cell.excluded);
                            let mut rt = RichText::new(if ex { format!("{txt}*") } else { txt }).color(if ex {
                                tokens.excluded_cell_text
                            } else {
                                tokens.text_primary
                            });
                            if ex {
                                rt = rt.italics().strikethrough();
                            }
                            tbl.label(rt);
                        }
                        tbl.end_row();
                    }
                });
            });
        },
    );
}

pub(super) fn default_plate_roles(is_384: bool, blank_row_a: bool) -> Vec<Value> {
    let row_len = if is_384 { 24 } else { 12 };
    let total = if is_384 { 384 } else { 96 };
    (0..total)
        .map(|idx| {
            let r = idx / row_len;
            let c = idx % row_len;
            if blank_row_a && r == 0 {
                Value::String(String::from("Blank"))
            } else {
                #[allow(clippy::cast_precision_loss)]
                let x = (c + 1) as f64;
                json!({ "role": "treatment", "group": "Sample", "x": x, "rep": r + 1 })
            }
        })
        .collect()
}

fn choice_pill(ui: &mut Ui, tokens: &Tokens, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let mut text = RichText::new(label);
    if selected {
        text = text.color(tokens.accent).strong();
    }
    ui.add_enabled(enabled, egui::Button::new(text).selected(selected))
}
