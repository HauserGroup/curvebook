//! Five-stage `Import & Paste Special` wizard (`680x520 pt`), `ImportReshape` (96/384 Plate Map
//! & Tidy Long Pivot), and `Convert Table Archetype...` modal in `egui 0.37` (`R15.4`,
//! `gui-layout-spec.md` §8.5).

use std::collections::BTreeMap;

use curvebook_doc::{Cell, CellCoord, CellValue, LinkBorderColor, PlacementMode, TableKind};
use egui::{Align, Context, Layout, Rect, RichText, Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::grid_view::{ImportOriginKind, ImportPasteWizardState, ImportReshapeMode, ImportWizardStage, format_cell_display};
use crate::i18n::tr_or_fallback;
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

    fn label(self) -> &'static str {
        match self {
            Self::Source => "1. Source",
            Self::Preview => "2. Preview",
            Self::FilterAndUnstack => "3. Filter & Unstack",
            Self::PlacementAndReshape => "4. Placement & Reshape",
            Self::Metadata => "5. Metadata",
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

    if !state.initialized_for_open {
        sync_drafts_from_app(app, state);
        state.initialized_for_open = true;
    }

    let (widget_id, title) = if is_convert {
        ("dialog.convert_archetype", tr_or_fallback(tokens.locale, "cmd.table.convertArchetype", "Convert Table Archetype"))
    } else if is_reshape {
        ("dialog.import_reshape", tr_or_fallback(tokens.locale, "w6.view.modal.import_reshape", "Import Plate & Tidy Data"))
    } else {
        ("dialog.import_paste_special", tr_or_fallback(tokens.locale, "menu.edit.paste_special", "Import & Paste Special"))
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

        let mut modal_cx = ImportModalCtx { app, state, tokens, meta, intents };
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

fn sync_drafts_from_app(app: &UiApp, state: &mut ImportWizardEguiState) {
    let wiz = &app.grid.import_wizard;
    state.raw_source_draft = if wiz.raw_clipboard_text.is_empty() && !app.view_shards.import_paste.raw_source_text.is_empty() {
        app.view_shards.import_paste.raw_source_text.clone()
    } else {
        wiz.raw_clipboard_text.clone()
    };
    if state.missing_tokens_draft.is_empty() {
        state.missing_tokens_draft = String::from("NA, NaN, .");
    }
    if state.encoding_name.is_empty() {
        state.encoding_name = String::from("UTF-8");
    }
    state.row_start_draft = wiz.filter.row_window.map_or_else(String::new, |(a, _)| a.to_string());
    state.row_end_draft = wiz.filter.row_window.map_or_else(String::new, |(_, b)| b.to_string());
    state.col_start_draft = wiz.filter.col_window.map_or_else(String::new, |(a, _)| a.to_string());
    state.col_end_draft = wiz.filter.col_window.map_or_else(String::new, |(_, b)| b.to_string());
    state.decimate_draft = wiz.filter.decimate_every_k.map_or_else(String::new, |k| k.to_string());
    if state.plate_values_per_row_draft.is_empty() {
        state.plate_values_per_row_draft = String::from("12");
    }
    if state.plate_rows_per_dataset_draft.is_empty() {
        state.plate_rows_per_dataset_draft = String::from("8");
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
}

fn render_five_stage_wizard_body(ui: &mut Ui, cx: &mut ImportModalCtx<'_>, avail_w: f32, body_h: f32) {
    // Persistent 5-stage horizontal stepper bar.
    ui.horizontal_wrapped(|bar| {
        for step in ImportWizardStep::ALL {
            let active = cx.state.active_step == step;
            let resp = choice_pill(bar, cx.tokens, step.label(), active, true);
            cx.meta.record_modal_response(
                bar,
                &resp,
                FrameInspectEntryInput::modal(step.tab_id(), "tab", step.label(), Some(active.to_string()), true),
            );
            if resp.clicked() {
                cx.state.active_step = step;
                let mut wiz = cx.app.grid.import_wizard.clone();
                wiz.stage = step.sync_backend_stage();
                cx.intents.push_update_import_wizard(wiz);
            }
        }
    });
    widgets::divider(ui);

    let scroll_h = (body_h - 34.0).max(100.0);
    egui::ScrollArea::vertical().id_salt("import_wizard_body_scroll").max_height(scroll_h).auto_shrink([false, false]).show(ui, |body| {
        match cx.state.active_step {
            ImportWizardStep::Source => render_step_source(body, cx, avail_w),
            ImportWizardStep::Preview => render_step_preview(body, cx.app, cx.state, cx.tokens, cx.meta),
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
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut wiz = app.grid.import_wizard.clone();
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
    } else if matches!(wiz.origin_kind, ImportOriginKind::InternalDataSheet | ImportOriginKind::InternalResultSheet) {
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
                    dirty = true;
                }
            }
        });
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
        for enc in ["UTF-8", "UTF-16", "Latin-1"] {
            let er = choice_pill(row, tokens, enc, state.encoding_name == enc, true);
            if er.clicked() {
                state.encoding_name = enc.to_owned();
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
    });

    if dirty {
        intents.push_update_import_wizard(wiz);
    }
}

fn render_step_preview(ui: &mut Ui, app: &UiApp, state: &ImportWizardEguiState, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    let preview = compute_wizard_transformed_grid(app, state);
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

    render_preview_grid_table(ui, app, &preview, tokens);
}

fn render_step_filter_unstack(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut wiz = app.grid.import_wizard.clone();
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
    });

    if dirty {
        intents.push_update_import_wizard(wiz);
    }
    ui.add_space(6.0);
    let preview = compute_wizard_transformed_grid(app, state);
    render_preview_grid_table(ui, app, &preview, tokens);
}

fn render_step_placement(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut wiz = app.grid.import_wizard.clone();
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
            }
        }
    });

    if state.block_reshape != WizardBlockReshape::None {
        ui.horizontal_wrapped(|row| {
            row.label("Values/Row:");
            let _ = row.add(egui::TextEdit::singleline(&mut state.plate_values_per_row_draft).hint_text("12").desired_width(56.0));
            row.label("Rows/DataSet:");
            let _ = row.add(egui::TextEdit::singleline(&mut state.plate_rows_per_dataset_draft).hint_text("8").desired_width(56.0));
        });
    }

    if dirty {
        intents.push_update_import_wizard(wiz);
    }
    ui.add_space(6.0);
    let preview = compute_wizard_transformed_grid(app, state);
    render_preview_grid_table(ui, app, &preview, tokens);
}

fn render_step_metadata(ui: &mut Ui, cx: &mut ImportModalCtx<'_>) {
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut wiz = app.grid.import_wizard.clone();
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

    if ui.checkbox(&mut wiz.record_in_info_sheet, "Record source lineage in project InfoSheet").changed() {
        dirty = true;
    }
    let _ = ui.checkbox(&mut state.extract_info_metadata, "Extract key-value header lines into Info constants");
    let _ = ui.checkbox(&mut state.add_floating_note, "Attach FloatingNote summary card onto target sheet");

    ui.add_space(8.0);
    ui.horizontal(|row| {
        let reset_r = widgets::secondary_button(row, "Reset Defaults");
        meta.record_modal_response(
            row,
            &reset_r,
            FrameInspectEntryInput::modal("import_paste.btn.reset_defaults", "button", "Reset Defaults", None, true),
        );
        if reset_r.clicked() {
            let fresh = ImportPasteWizardState {
                open: true,
                raw_clipboard_text: wiz.raw_clipboard_text.clone(),
                target_sheet_id: wiz.target_sheet_id.clone(),
                ..ImportPasteWizardState::default()
            };
            intents.push_update_import_wizard(fresh);
            return;
        }
        let save_r = widgets::secondary_button(row, "Save Preset");
        meta.record_modal_response(row, &save_r, FrameInspectEntryInput::modal("import_paste.btn.save_default", "button", "Save Preset", None, true));
    });

    if dirty {
        intents.push_update_import_wizard(wiz);
    }
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
    let app = cx.app;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    ui.horizontal(|footer| {
        let cancel_lbl = tr_or_fallback(tokens.locale, "btn.cancel", "Cancel");
        let cancel_resp = widgets::secondary_button(footer, cancel_lbl);
        meta.record_modal_response(footer, &cancel_resp, FrameInspectEntryInput::modal("modal.cancel", "button", cancel_lbl, None, true));
        if cancel_resp.clicked() {
            if is_wizard {
                intents.push_close_import_wizard();
            } else {
                intents.push_dialog_button("cancel");
            }
        }

        footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
            let finish_lbl = if is_wizard { "Import" } else { tr_or_fallback(tokens.locale, "btn.ok", "OK") };
            let ok_resp = widgets::primary_button(rhs, finish_lbl);
            meta.record_modal_response(rhs, &ok_resp, FrameInspectEntryInput::modal("modal.ok", "button", finish_lbl, None, true));
            if ok_resp.clicked() {
                if is_wizard {
                    let mut wiz = app.grid.import_wizard.clone();
                    if !state.raw_source_draft.is_empty() {
                        wiz.raw_clipboard_text = state.raw_source_draft.clone();
                    }
                    intents.push_update_import_wizard(wiz);
                    intents.push_commit_import_wizard(true);
                } else {
                    intents.push_dialog_button("ok");
                }
            }

            if is_wizard {
                let idx = ImportWizardStep::ALL.iter().position(|s| *s == state.active_step).unwrap_or(0);
                let has_next = idx + 1 < ImportWizardStep::ALL.len();
                let next_r = rhs.add_enabled(has_next, egui::Button::new("Next >"));
                meta.record_modal_response(rhs, &next_r, FrameInspectEntryInput::modal("modal.next", "button", "Next", None, has_next));
                if next_r.clicked() && has_next {
                    state.active_step = ImportWizardStep::ALL[idx + 1];
                }

                let has_back = idx > 0;
                let back_r = rhs.add_enabled(has_back, egui::Button::new("< Back"));
                meta.record_modal_response(rhs, &back_r, FrameInspectEntryInput::modal("modal.back", "button", "Back", None, has_back));
                if back_r.clicked() && has_back {
                    state.active_step = ImportWizardStep::ALL[idx - 1];
                }
            } else if is_reshape {
                let _ = rhs.label(RichText::new("Single atomic commit").size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
            }
        });
    });
}

fn parse_opt_window(start_s: &str, end_s: &str) -> Option<(usize, usize)> {
    let a = start_s.trim().parse::<usize>().ok()?;
    let b = end_s.trim().parse::<usize>().ok()?;
    (a <= b).then_some((a, b))
}

fn compute_wizard_transformed_grid(app: &UiApp, state: &ImportWizardEguiState) -> Vec<Vec<Cell>> {
    let wiz = &app.grid.import_wizard;
    let missing_tokens: Vec<&str> = state.missing_tokens_draft.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut grid = match wiz.origin_kind {
        ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile => {
            let src_text = if !state.raw_source_draft.is_empty() { state.raw_source_draft.as_str() } else { wiz.raw_clipboard_text.as_str() };
            src_text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    line.split(wiz.delimiter)
                        .map(|tok| {
                            let trimmed = tok.trim();
                            if missing_tokens.iter().any(|m| m.eq_ignore_ascii_case(trimmed)) {
                                Cell::empty()
                            } else {
                                Cell::parse_import_token(trimmed, wiz.decimal_separator, wiz.filter.parse_asterisk_exclusions)
                            }
                        })
                        .collect::<Vec<Cell>>()
                })
                .collect::<Vec<Vec<Cell>>>()
        }
        ImportOriginKind::InternalDataSheet => app
            .document()
            .data_sheet(wiz.source_sheet_id.as_ref().unwrap_or(&curvebook_doc::SheetId::new("")))
            .map(|ds| {
                let mut rows = Vec::new();
                for r in 0..ds.archetype.row_count().min(128) {
                    let mut row = Vec::new();
                    for c in 0..ds.archetype.column_count().min(24) {
                        row.push(ds.archetype.data_cell(CellCoord::new(r, c, 0)).cloned().unwrap_or_else(Cell::empty));
                    }
                    rows.push(row);
                }
                rows
            })
            .unwrap_or_default(),
        ImportOriginKind::InternalResultSheet => Vec::new(),
    };

    if !state.filter_predicate_draft.trim().is_empty() {
        let needle = state.filter_predicate_draft.trim().to_ascii_lowercase();
        grid.retain(|row| row.iter().any(|c| format_cell_display(c, &app.locale_preset).to_ascii_lowercase().contains(&needle)));
    }
    if let Some((r0, r1)) = wiz.filter.row_window {
        if r0 <= r1 && r0 < grid.len() {
            grid = grid[r0..=r1.min(grid.len() - 1)].to_vec();
        } else {
            grid.clear();
        }
    }
    if let Some(gcol) = wiz.filter.unstack_group_column {
        grid = unstack_preview(&grid, gcol, &app.locale_preset);
    } else if let Some((c0, c1)) = wiz.filter.col_window {
        for row in &mut grid {
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
        grid = grid.into_iter().step_by(k).collect();
    }
    if wiz.placement.skip_contiguous_blanks {
        grid.retain(|row| row.iter().any(|c| !c.is_empty()));
    }
    if wiz.placement.transpose {
        grid = curvebook_doc::transpose_cell_grid(&grid);
    }
    if state.block_reshape != WizardBlockReshape::None {
        let width = match state.block_reshape {
            WizardBlockReshape::Plate96 => 12usize,
            WizardBlockReshape::Plate384 => 24usize,
            WizardBlockReshape::RowWrapBlock => state.plate_values_per_row_draft.trim().parse::<usize>().unwrap_or(12).clamp(1, 64),
            WizardBlockReshape::None => 1,
        };
        let flat: Vec<Cell> = grid.into_iter().flatten().collect();
        grid = flat.chunks(width).map(|c| c.to_vec()).collect();
    }
    grid
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
