//! R13 canvas modals: `Create Graph`, `Equalize Scaling`, `Format Embedded Table`, and `Format Image`.

use curvebook_doc::CropInsetsPt;
use curvebook_palette::{LineDashStyle, Rgba8};
use egui::Ui;

use crate::dialogs_canvas::FormatImageDialogState;
use crate::egui_shell::UiIntent;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{LocalePreset, tr};

use super::helpers::{ButtonEntry, GraphModalCtx, format_localized_draft_f64, record_button, record_checkbox, record_input};

/// Format a numeric value for an `egui::DragValue` respecting `preset.decimal_separator` (`AUD-10-52`).
pub(super) fn format_drag_value_localized(val: f64, preset: &LocalePreset) -> String {
    let rounded = (val * 100.0).round() / 100.0;
    format_localized_draft_f64(rounded, preset)
}

/// Parse user input from a localized `egui::DragValue` (stripping any trailing unit suffix) (`AUD-10-52`).
pub(super) fn parse_drag_value_localized(raw: &str, suffix: &str, preset: &LocalePreset) -> Option<f64> {
    let trimmed_suffix = suffix.trim();
    let trimmed = raw.trim();
    let stripped = if !trimmed_suffix.is_empty() { trimmed.strip_suffix(trimmed_suffix).unwrap_or(trimmed).trim() } else { trimmed };
    parse_localized_number_input(stripped, preset)
}

fn localized_drag_value<'a>(
    val: &'a mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
    suffix: &'a str,
    preset: &LocalePreset,
) -> egui::DragValue<'a> {
    let fmt_preset = preset.clone();
    let parse_preset = preset.clone();
    let suffix_owned = suffix.to_owned();
    egui::DragValue::new(val)
        .range(range)
        .speed(speed)
        .suffix(suffix)
        .custom_formatter(move |v, _| format_drag_value_localized(v, &fmt_preset))
        .custom_parser(move |raw| parse_drag_value_localized(raw, &suffix_owned, &parse_preset))
}

fn localized_fixed1(val: f64, preset: &LocalePreset) -> String {
    let s = format!("{val:.1}");
    if preset.decimal_separator != '.' { s.replace('.', &preset.decimal_separator.to_string()) } else { s }
}

/// Proportionally clamp `crop` insets whenever image bounds shrink so `left_pt + right_pt < width_pt`
/// and `top_pt + bottom_pt < height_pt`, preserving at least `2.0 pt` of interior (`AUD-10-36`).
pub(super) fn clamp_image_crop_to_bounds(crop: &mut CropInsetsPt, width_pt: f64, height_pt: f64) {
    let max_lr = (width_pt - 2.0).max(0.0);
    let left_pos = if crop.left_pt.is_finite() { crop.left_pt.max(0.0) } else { 0.0 };
    let right_pos = if crop.right_pt.is_finite() { crop.right_pt.max(0.0) } else { 0.0 };
    let lr_sum = left_pos + right_pos;
    if lr_sum > max_lr && lr_sum > 0.0 {
        let scale = max_lr / lr_sum;
        crop.left_pt = (left_pos * scale).min(max_lr);
        crop.right_pt = (right_pos * scale).min((max_lr - crop.left_pt).max(0.0));
    } else {
        crop.left_pt = left_pos.clamp(0.0, max_lr);
        crop.right_pt = right_pos.clamp(0.0, (max_lr - crop.left_pt).max(0.0));
    }

    let max_tb = (height_pt - 2.0).max(0.0);
    let top_pos = if crop.top_pt.is_finite() { crop.top_pt.max(0.0) } else { 0.0 };
    let bottom_pos = if crop.bottom_pt.is_finite() { crop.bottom_pt.max(0.0) } else { 0.0 };
    let tb_sum = top_pos + bottom_pos;
    if tb_sum > max_tb && tb_sum > 0.0 {
        let scale = max_tb / tb_sum;
        crop.top_pt = (top_pos * scale).min(max_tb);
        crop.bottom_pt = (bottom_pos * scale).min((max_tb - crop.top_pt).max(0.0));
    } else {
        crop.top_pt = top_pos.clamp(0.0, max_tb);
        crop.bottom_pt = bottom_pos.clamp(0.0, (max_tb - crop.top_pt).max(0.0));
    }
}

/// Apply a width update to `draft` respecting pre-mutation `aspect_h_over_w()` and clamping `crop` (`AUD-10-36`).
pub(super) fn apply_format_image_width(draft: &mut FormatImageDialogState, new_width_pt: f64) {
    let ratio = draft.aspect_h_over_w();
    draft.bounds_pt.width = new_width_pt.clamp(8.0, 2000.0);
    if draft.lock_aspect_ratio && ratio.is_finite() && ratio > 0.0 {
        draft.bounds_pt.height = (draft.bounds_pt.width * ratio).clamp(8.0, 2000.0);
    }
    clamp_image_crop_to_bounds(&mut draft.crop, draft.bounds_pt.width, draft.bounds_pt.height);
}

/// Apply a height update to `draft` respecting pre-mutation `aspect_h_over_w()` and clamping `crop` (`AUD-10-36`).
pub(super) fn apply_format_image_height(draft: &mut FormatImageDialogState, new_height_pt: f64) {
    let ratio = draft.aspect_h_over_w();
    draft.bounds_pt.height = new_height_pt.clamp(8.0, 2000.0);
    if draft.lock_aspect_ratio && ratio.is_finite() && ratio > 0.0 {
        draft.bounds_pt.width = (draft.bounds_pt.height / ratio.max(1e-9)).clamp(8.0, 2000.0);
    }
    clamp_image_crop_to_bounds(&mut draft.crop, draft.bounds_pt.width, draft.bounds_pt.height);
}

pub(super) fn render_create_graph_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.create_graph.clone();
    let mut dirty = false;

    ui.label(egui::RichText::new(tr(gctx.locale, "r13.create_graph.source")).strong());
    if draft.source_candidates.is_empty() {
        ui.label(egui::RichText::new(tr(gctx.locale, "r13.create_graph.no_source")).color(gctx.tokens.text_muted));
    } else {
        let candidates = draft.source_candidates.clone();
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (id, title)) in candidates.iter().enumerate() {
                let selected = draft.source_sheet_id.as_ref() == Some(id);
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("create_graph.source.{idx}"),
                        label: title,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.source_sheet_id = Some(id.clone());
                    dirty = true;
                }
            }
        });
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.create_graph.kind")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl) in [
            ("auto", "Auto"),
            ("xy", "XY"),
            ("column", "Column"),
            ("grouped", "Grouped"),
            ("contingency", "Contingency"),
            ("survival", "Survival"),
            ("parts_of_whole", "Parts of Whole"),
            ("multiple_variables", "Multiple Variables"),
            ("heatmap", "Heatmap"),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("create_graph.kind.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.kind_slug == slug,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.kind_slug = slug.to_owned();
                dirty = true;
            }
        }
    });

    ui.add_space(6.0);
    let mut per_ds = draft.one_per_data_set;
    let per_lbl = tr(gctx.locale, "r13.create_graph.one_per_ds");
    let per_chk = ui.checkbox(&mut per_ds, per_lbl);
    record_checkbox(ui, &per_chk, gctx.meta, "create_graph.one_per_data_set", per_lbl, per_ds);
    if per_chk.changed() {
        draft.one_per_data_set = per_ds;
        dirty = true;
    }

    let field_w = (gctx.body_w - 110.0).clamp(120.0, 320.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.create_graph.y_title"));
        let r = ui.add(egui::TextEdit::singleline(&mut draft.y_axis_title).desired_width(field_w));
        record_input(ui, &r, gctx.meta, "create_graph.y_axis_title", "Y-axis Title", &draft.y_axis_title, true);
        if r.changed() {
            dirty = true;
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.create_graph.title"));
        let r = ui.add(egui::TextEdit::singleline(&mut draft.graph_title).desired_width(field_w));
        record_input(ui, &r, gctx.meta, "create_graph.title", "Graph Title", &draft.graph_title, true);
        if r.changed() {
            dirty = true;
        }
    });

    if dirty {
        gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(draft)));
    }
}

pub(super) fn render_equalize_scaling_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.equalize_scaling.clone();
    let mut dirty = false;

    ui.label(egui::RichText::new(tr(gctx.locale, "r13.equalize.mode")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (mode_slug, btn_slug, lbl_key) in [
            ("match_smallest", "smallest", "r13.equalize.match_smallest"),
            ("match_largest", "largest", "r13.equalize.match_largest"),
            ("explicit_percent", "explicit", "r13.equalize.explicit_pct"),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("equalize_scaling.mode.{btn_slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.mode == mode_slug,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.mode = mode_slug.to_owned();
                dirty = true;
            }
        }
    });

    if draft.mode == "explicit_percent" {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label(tr(gctx.locale, "r13.equalize.scale_pct"));
            for pct in [50.0_f64, 75.0, 100.0, 125.0] {
                let lbl = format!("{}%", pct as i32);
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("equalize_scaling.percent.{}", pct as i32),
                        label: &lbl,
                        command_id: None,
                        selected: (draft.explicit_percent - pct).abs() < 0.05,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.explicit_percent = pct;
                    dirty = true;
                }
            }
            let mut cur_pct = draft.explicit_percent;
            let dv = ui.add(localized_drag_value(&mut cur_pct, 5.0..=500.0, 1.0, "%", &gctx.app.locale_preset));
            let rec_val = localized_fixed1(cur_pct, &gctx.app.locale_preset);
            record_input(ui, &dv, gctx.meta, "equalize_scaling.percent", "Scale %", &rec_val, true);
            if dv.changed() {
                draft.explicit_percent = cur_pct.clamp(5.0, 500.0);
                dirty = true;
            }
        });
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.equalize.panels")).strong());
    if draft.available_panels.is_empty() {
        ui.label(egui::RichText::new(tr(gctx.locale, "r13.equalize.no_panels")).color(gctx.tokens.text_muted));
    } else {
        let panels = draft.available_panels.clone();
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (pid, label)) in panels.iter().enumerate() {
                let selected = draft.selected_panel_ids.contains(pid);
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("equalize_scaling.panel.{idx}"),
                        label,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    if let Some(pos) = draft.selected_panel_ids.iter().position(|id| id == pid) {
                        draft.selected_panel_ids.remove(pos);
                    } else {
                        draft.selected_panel_ids.push(pid.clone());
                    }
                    dirty = true;
                }
            }
        });
    }

    if dirty {
        gctx.intents.push(UiIntent::UpdateEqualizeScalingState(Box::new(draft)));
    }
}

pub(super) fn render_format_embedded_table_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.format_embedded_table.clone();
    let mut dirty = false;

    if !draft.source_label.is_empty() {
        ui.label(egui::RichText::new(format!("{}: {}", tr(gctx.locale, "r13.table.source"), draft.source_label)).color(gctx.tokens.text_muted));
        ui.add_space(4.0);
    }

    for (wid, lbl_key, val_ref) in [
        ("embedded_table.show_column_titles", "r13.table.show_col_titles", &mut draft.style.show_column_titles),
        ("embedded_table.show_row_titles", "r13.table.show_row_titles", &mut draft.style.show_row_titles),
        ("embedded_table.transpose", "r13.table.transpose", &mut draft.style.transpose),
        ("embedded_table.bold_headers", "r13.table.bold_headers", &mut draft.style.bold_headers),
    ] {
        let lbl = tr(gctx.locale, lbl_key);
        let chk = ui.checkbox(val_ref, lbl);
        record_checkbox(ui, &chk, gctx.meta, wid, lbl, *val_ref);
        dirty |= chk.changed();
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.table.font_size"));
        for (slug, sz) in [("6", 6.0_f64), ("75", 7.5), ("9", 9.0), ("11", 11.0)] {
            let lbl = if (sz - 7.5).abs() < 0.05 { String::from("7.5 pt") } else { format!("{} pt", sz as i32) };
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("embedded_table.font_size.{slug}"),
                    label: &lbl,
                    command_id: None,
                    selected: (draft.style.font_size_pt - sz).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.style.font_size_pt = sz;
                dirty = true;
            }
        }
        let mut cur_sz = draft.style.font_size_pt;
        let dv = ui.add(localized_drag_value(&mut cur_sz, 4.0..=36.0, 0.5, " pt", &gctx.app.locale_preset));
        let rec_val = localized_fixed1(cur_sz, &gctx.app.locale_preset);
        record_input(ui, &dv, gctx.meta, "embedded_table.font_size", "Font pt", &rec_val, true);
        if dv.changed() {
            draft.style.font_size_pt = cur_sz.clamp(4.0, 36.0);
            dirty = true;
        }
    });

    render_embedded_table_border_and_fill(ui, gctx, &mut draft.style, &mut dirty);

    if dirty {
        gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(draft)));
    }
}

fn render_embedded_table_border_and_fill(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, style: &mut curvebook_doc::EmbeddedTableStyle, dirty: &mut bool) {
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.table.outer_border"));
        for (slug, lbl, w) in [("0", "0 pt", 0.0_f64), ("05", "0.5 pt", 0.5), ("075", "0.75 pt", 0.75), ("15", "1.5 pt", 1.5)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("embedded_table.outer_border.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: (style.outer_border_width_pt - w).abs() < 0.05,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                style.outer_border_width_pt = w;
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.table.gridlines"));
        for (slug, lbl, w) in [("0", "0 pt", 0.0_f64), ("05", "0.5 pt", 0.5), ("1", "1 pt", 1.0)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("embedded_table.gridline.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: (style.gridline_width_pt - w).abs() < 0.05,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                style.gridline_width_pt = w;
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.table.fill"));
        for (slug, lbl_key, fill) in [
            ("white", "r13.fg.paper_white", Some(Rgba8::WHITE)),
            ("cream", "r13.fg.paper_cream", Some(Rgba8::rgb(250, 248, 242))),
            ("transparent", "r13.common.none", None),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("embedded_table.fill.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: style.fill_color == fill,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                style.fill_color = fill;
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.table.border_color"));
        for (slug, lbl, col) in
            [("black", "Black", Rgba8::BLACK), ("slate", "Slate", Rgba8::rgb(71, 85, 105)), ("navy", "Navy", Rgba8::rgb(30, 58, 138))]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("embedded_table.border_color.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: style.border_color == col,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                style.border_color = col;
                *dirty = true;
            }
        }
    });
}

pub(super) fn render_format_image_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.format_image.clone();
    let mut dirty = false;

    if let Some((pw, ph)) = draft.pixel_size {
        ui.label(egui::RichText::new(format!("{} · {pw}×{ph} px", draft.asset_id)).small().color(gctx.tokens.text_muted));
        ui.add_space(4.0);
    }
    render_format_image_geometry(ui, gctx, &mut draft, &mut dirty);
    ui.add_space(6.0);
    render_format_image_crop(ui, gctx, &mut draft, &mut dirty);
    ui.add_space(6.0);
    render_format_image_border(ui, gctx, &mut draft, &mut dirty);

    if dirty {
        gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(draft)));
    }
}

fn render_format_image_geometry(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatImageDialogState, dirty: &mut bool) {
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.image.size_rotation")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("W:");
        let mut w_pt = draft.bounds_pt.width;
        let w_dv = ui.add_sized(egui::vec2(72.0, 24.0), localized_drag_value(&mut w_pt, 8.0..=2000.0, 1.0, " pt", &gctx.app.locale_preset));
        let w_rec = localized_fixed1(w_pt, &gctx.app.locale_preset);
        record_input(ui, &w_dv, gctx.meta, "format_image.width_pt", "Width", &w_rec, true);
        if w_dv.changed() {
            apply_format_image_width(draft, w_pt);
            *dirty = true;
        }

        ui.label("H:");
        let mut h_pt = draft.bounds_pt.height;
        let h_dv = ui.add_sized(egui::vec2(72.0, 24.0), localized_drag_value(&mut h_pt, 8.0..=2000.0, 1.0, " pt", &gctx.app.locale_preset));
        let h_rec = localized_fixed1(h_pt, &gctx.app.locale_preset);
        record_input(ui, &h_dv, gctx.meta, "format_image.height_pt", "Height", &h_rec, true);
        if h_dv.changed() {
            apply_format_image_height(draft, h_pt);
            *dirty = true;
        }

        let mut lock = draft.lock_aspect_ratio;
        let lock_lbl = tr(gctx.locale, "r13.image.lock_aspect");
        let lock_chk = ui.checkbox(&mut lock, lock_lbl);
        record_checkbox(ui, &lock_chk, gctx.meta, "format_image.lock_aspect_ratio", lock_lbl, lock);
        if lock_chk.changed() {
            draft.lock_aspect_ratio = lock;
            *dirty = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.image.rotation"));
        for deg in [0.0_f64, 90.0, 180.0, 270.0] {
            let lbl = format!("{}°", deg as i32);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_image.rotation.{}", deg as i32),
                    label: &lbl,
                    command_id: None,
                    selected: (draft.rotation_deg - deg).abs() < 0.25,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.rotation_deg = deg;
                *dirty = true;
            }
        }
        let mut rot = draft.rotation_deg;
        let rot_dv = ui.add_sized(egui::vec2(72.0, 24.0), localized_drag_value(&mut rot, -360.0..=360.0, 1.0, "°", &gctx.app.locale_preset));
        let rot_rec = localized_fixed1(rot, &gctx.app.locale_preset);
        record_input(ui, &rot_dv, gctx.meta, "format_image.rotation_deg", "Angle", &rot_rec, true);
        if rot_dv.changed() {
            draft.rotation_deg = rot.clamp(-360.0, 360.0);
            *dirty = true;
        }
    });
}

fn render_format_image_crop(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatImageDialogState, dirty: &mut bool) {
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.image.crop")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let max_lr = (draft.bounds_pt.width - 2.0).max(0.0);
        let max_tb = (draft.bounds_pt.height - 2.0).max(0.0);
        let prev_crop = draft.crop;
        let mut crop = draft.crop;
        let mut changed = false;

        for (wid, lbl, val_ref, cap) in [
            ("format_image.crop.left", "L", &mut crop.left_pt, (max_lr - prev_crop.right_pt).max(0.0)),
            ("format_image.crop.right", "R", &mut crop.right_pt, (max_lr - prev_crop.left_pt).max(0.0)),
            ("format_image.crop.top", "T", &mut crop.top_pt, (max_tb - prev_crop.bottom_pt).max(0.0)),
            ("format_image.crop.bottom", "B", &mut crop.bottom_pt, (max_tb - prev_crop.top_pt).max(0.0)),
        ] {
            ui.label(format!("{lbl}:"));
            let dv = ui.add(localized_drag_value(val_ref, 0.0..=cap, 0.5, " pt", &gctx.app.locale_preset));
            let rec_val = localized_fixed1(*val_ref, &gctx.app.locale_preset);
            record_input(ui, &dv, gctx.meta, wid, lbl, &rec_val, true);
            changed |= dv.changed();
        }
        if changed {
            clamp_image_crop_to_bounds(&mut crop, draft.bounds_pt.width, draft.bounds_pt.height);
            draft.crop = crop;
            *dirty = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (edge, lbl) in [("left", "L +4"), ("right", "R +4"), ("top", "T +4"), ("bottom", "B +4")] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_image.crop.{edge}_plus4"),
                    label: lbl,
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                match edge {
                    "left" => draft.crop.left_pt = (draft.crop.left_pt + 4.0).min((draft.bounds_pt.width - draft.crop.right_pt - 2.0).max(0.0)),
                    "right" => draft.crop.right_pt = (draft.crop.right_pt + 4.0).min((draft.bounds_pt.width - draft.crop.left_pt - 2.0).max(0.0)),
                    "top" => draft.crop.top_pt = (draft.crop.top_pt + 4.0).min((draft.bounds_pt.height - draft.crop.bottom_pt - 2.0).max(0.0)),
                    _ => draft.crop.bottom_pt = (draft.crop.bottom_pt + 4.0).min((draft.bounds_pt.height - draft.crop.top_pt - 2.0).max(0.0)),
                }
                *dirty = true;
            }
        }
        let reset_lbl = tr(gctx.locale, "r13.image.crop_reset");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_image.crop.reset",
                label: reset_lbl,
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            draft.crop = CropInsetsPt::default();
            *dirty = true;
        }
    });
}

fn render_format_image_border(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatImageDialogState, dirty: &mut bool) {
    let mut border_on = draft.border_enabled;
    let border_lbl = tr(gctx.locale, "r13.image.border");
    let chk = ui.checkbox(&mut border_on, border_lbl);
    record_checkbox(ui, &chk, gctx.meta, "format_image.border.enabled", border_lbl, border_on);
    if chk.changed() {
        draft.border_enabled = border_on;
        *dirty = true;
    }

    if draft.border_enabled {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, w) in [("05", "0.5 pt", 0.5_f64), ("1", "1 pt", 1.0), ("2", "2 pt", 2.0)] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_image.border.width.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: (draft.border_width_pt - w).abs() < 0.05,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.border_width_pt = w;
                    *dirty = true;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, col) in
                [("black", "Black", Rgba8::BLACK), ("slate", "Slate", Rgba8::rgb(71, 85, 105)), ("accent", "Blue", Rgba8::rgb(37, 99, 235))]
            {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_image.border.color.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: draft.border_color == col,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.border_color = col;
                    *dirty = true;
                }
            }
            for (slug, lbl, dash) in
                [("solid", "Solid", LineDashStyle::Solid), ("dashed", "Dashed", LineDashStyle::Dashed), ("dotted", "Dotted", LineDashStyle::Dotted)]
            {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_image.border.dash.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: draft.border_dash == dash,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.border_dash = dash;
                    *dirty = true;
                }
            }
        });
    }
}
