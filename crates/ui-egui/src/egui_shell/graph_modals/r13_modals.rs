//! R13 canvas modals: `Create Graph`, `Equalize Scaling`, `Format Embedded Table`, and `Format Image`.

use crate::egui_shell::UiIntent;
use curvebook_doc::CropInsetsPt;
use curvebook_palette::{LineDashStyle, Rgba8};
use egui::Ui;

use crate::dialogs_canvas::FormatImageDialogState;
use crate::i18n::tr;

use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input};

pub(super) fn render_create_graph_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let cg = gctx.app.canvas_dialogs.create_graph.clone();
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.create_graph.source")).strong());
    if cg.source_candidates.is_empty() {
        ui.label(egui::RichText::new(tr(gctx.locale, "r13.create_graph.no_source")).color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (id, title)) in cg.source_candidates.iter().enumerate() {
                let selected = cg.source_sheet_id.as_ref() == Some(id);
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
                    let mut next = cg.clone();
                    next.source_sheet_id = Some(id.clone());
                    gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(next)));
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
                    selected: cg.kind_slug == slug,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = cg.clone();
                next.kind_slug = slug.to_owned();
                gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(6.0);
    let mut per_ds = cg.one_per_data_set;
    let per_lbl = tr(gctx.locale, "r13.create_graph.one_per_ds");
    let per_chk = ui.checkbox(&mut per_ds, per_lbl);
    record_checkbox(ui, &per_chk, gctx.meta, "create_graph.one_per_data_set", per_lbl, per_ds);
    if per_chk.changed() {
        let mut next = cg.clone();
        next.one_per_data_set = per_ds;
        gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(next)));
    }

    let field_w = (gctx.body_w - 110.0).clamp(120.0, 320.0);
    let mut y_title = cg.y_axis_title.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.create_graph.y_title"));
        let r = ui.add(egui::TextEdit::singleline(&mut y_title).desired_width(field_w));
        record_input(ui, &r, gctx.meta, "create_graph.y_axis_title", "Y-axis Title", &y_title, true);
        if r.changed() {
            let mut next = cg.clone();
            next.y_axis_title = y_title.clone();
            gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(next)));
        }
    });

    let mut g_title = cg.graph_title.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.create_graph.title"));
        let r = ui.add(egui::TextEdit::singleline(&mut g_title).desired_width(field_w));
        record_input(ui, &r, gctx.meta, "create_graph.title", "Graph Title", &g_title, true);
        if r.changed() {
            let mut next = cg.clone();
            next.graph_title = g_title.clone();
            gctx.intents.push(UiIntent::UpdateCreateGraphState(Box::new(next)));
        }
    });
}

pub(super) fn render_equalize_scaling_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let eq = gctx.app.canvas_dialogs.equalize_scaling.clone();
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
                    selected: eq.mode == mode_slug,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = eq.clone();
                next.mode = mode_slug.to_owned();
                gctx.intents.push(UiIntent::UpdateEqualizeScalingState(Box::new(next)));
            }
        }
    });

    if eq.mode == "explicit_percent" {
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
                        selected: (eq.explicit_percent - pct).abs() < 0.05,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = eq.clone();
                    next.explicit_percent = pct;
                    gctx.intents.push(UiIntent::UpdateEqualizeScalingState(Box::new(next)));
                }
            }
            let mut cur_pct = eq.explicit_percent;
            let dv = ui.add(egui::DragValue::new(&mut cur_pct).range(5.0..=500.0).speed(1.0).suffix("%"));
            record_input(ui, &dv, gctx.meta, "equalize_scaling.percent", "Scale %", &format!("{cur_pct:.1}"), true);
            if dv.changed() {
                let mut next = eq.clone();
                next.explicit_percent = cur_pct.clamp(5.0, 500.0);
                gctx.intents.push(UiIntent::UpdateEqualizeScalingState(Box::new(next)));
            }
        });
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.equalize.panels")).strong());
    if eq.available_panels.is_empty() {
        ui.label(egui::RichText::new(tr(gctx.locale, "r13.equalize.no_panels")).color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (pid, label)) in eq.available_panels.iter().enumerate() {
                let selected = eq.selected_panel_ids.contains(pid);
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
                    let mut next = eq.clone();
                    if let Some(pos) = next.selected_panel_ids.iter().position(|id| id == pid) {
                        next.selected_panel_ids.remove(pos);
                    } else {
                        next.selected_panel_ids.push(pid.clone());
                    }
                    gctx.intents.push(UiIntent::UpdateEqualizeScalingState(Box::new(next)));
                }
            }
        });
    }
}

pub(super) fn render_format_embedded_table_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let et = gctx.app.canvas_dialogs.format_embedded_table.clone();
    if !et.source_label.is_empty() {
        ui.label(egui::RichText::new(format!("{}: {}", tr(gctx.locale, "r13.table.source"), et.source_label)).color(gctx.tokens.text_muted));
        ui.add_space(4.0);
    }

    let mut style = et.style;
    let mut dirty = false;
    for (wid, lbl_key, val_ref) in [
        ("embedded_table.show_column_titles", "r13.table.show_col_titles", &mut style.show_column_titles),
        ("embedded_table.show_row_titles", "r13.table.show_row_titles", &mut style.show_row_titles),
        ("embedded_table.transpose", "r13.table.transpose", &mut style.transpose),
        ("embedded_table.bold_headers", "r13.table.bold_headers", &mut style.bold_headers),
    ] {
        let lbl = tr(gctx.locale, lbl_key);
        let chk = ui.checkbox(val_ref, lbl);
        record_checkbox(ui, &chk, gctx.meta, wid, lbl, *val_ref);
        dirty |= chk.changed();
    }
    if dirty {
        let mut next = et.clone();
        next.style = style;
        gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
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
                    selected: (et.style.font_size_pt - sz).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = et.clone();
                next.style.font_size_pt = sz;
                gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
            }
        }
        let mut cur_sz = et.style.font_size_pt;
        let dv = ui.add(egui::DragValue::new(&mut cur_sz).range(4.0..=36.0).speed(0.5).suffix(" pt"));
        record_input(ui, &dv, gctx.meta, "embedded_table.font_size", "Font pt", &format!("{cur_sz:.1}"), true);
        if dv.changed() {
            let mut next = et.clone();
            next.style.font_size_pt = cur_sz.clamp(4.0, 36.0);
            gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
        }
    });

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
                    selected: (et.style.outer_border_width_pt - w).abs() < 0.05,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = et.clone();
                next.style.outer_border_width_pt = w;
                gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
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
                    selected: (et.style.gridline_width_pt - w).abs() < 0.05,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = et.clone();
                next.style.gridline_width_pt = w;
                gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
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
                    selected: et.style.fill_color == fill,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = et.clone();
                next.style.fill_color = fill;
                gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
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
                    selected: et.style.border_color == col,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = et.clone();
                next.style.border_color = col;
                gctx.intents.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(next)));
            }
        }
    });
}

pub(super) fn render_format_image_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fi = gctx.app.canvas_dialogs.format_image.clone();
    if let Some((pw, ph)) = fi.pixel_size {
        ui.label(egui::RichText::new(format!("{} · {pw}×{ph} px", fi.asset_id)).small().color(gctx.tokens.text_muted));
        ui.add_space(4.0);
    }
    render_format_image_geometry(ui, gctx, &fi);
    ui.add_space(6.0);
    render_format_image_crop(ui, gctx, &fi);
    ui.add_space(6.0);
    render_format_image_border(ui, gctx, &fi);
}

fn render_format_image_geometry(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, fi: &FormatImageDialogState) {
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.image.size_rotation")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("W:");
        let mut w_pt = fi.bounds_pt.width;
        let w_dv = ui.add(egui::DragValue::new(&mut w_pt).range(8.0..=2000.0).speed(1.0).suffix(" pt"));
        record_input(ui, &w_dv, gctx.meta, "format_image.width_pt", "Width", &format!("{w_pt:.1}"), true);
        if w_dv.changed() {
            let mut next = fi.clone();
            next.bounds_pt.width = w_pt.clamp(8.0, 2000.0);
            if next.lock_aspect_ratio {
                next.bounds_pt.height = (next.bounds_pt.width * next.aspect_h_over_w()).clamp(8.0, 2000.0);
            }
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
        }

        ui.label("H:");
        let mut h_pt = fi.bounds_pt.height;
        let h_dv = ui.add(egui::DragValue::new(&mut h_pt).range(8.0..=2000.0).speed(1.0).suffix(" pt"));
        record_input(ui, &h_dv, gctx.meta, "format_image.height_pt", "Height", &format!("{h_pt:.1}"), true);
        if h_dv.changed() {
            let mut next = fi.clone();
            next.bounds_pt.height = h_pt.clamp(8.0, 2000.0);
            if next.lock_aspect_ratio {
                next.bounds_pt.width = (next.bounds_pt.height / next.aspect_h_over_w().max(1e-9)).clamp(8.0, 2000.0);
            }
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
        }

        let mut lock = fi.lock_aspect_ratio;
        let lock_lbl = tr(gctx.locale, "r13.image.lock_aspect");
        let lock_chk = ui.checkbox(&mut lock, lock_lbl);
        record_checkbox(ui, &lock_chk, gctx.meta, "format_image.lock_aspect_ratio", lock_lbl, lock);
        if lock_chk.changed() {
            let mut next = fi.clone();
            next.lock_aspect_ratio = lock;
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
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
                    selected: (fi.rotation_deg - deg).abs() < 0.25,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fi.clone();
                next.rotation_deg = deg;
                gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
            }
        }
        let mut rot = fi.rotation_deg;
        let rot_dv = ui.add(egui::DragValue::new(&mut rot).range(-360.0..=360.0).speed(1.0).suffix("°"));
        record_input(ui, &rot_dv, gctx.meta, "format_image.rotation_deg", "Angle", &format!("{rot:.1}"), true);
        if rot_dv.changed() {
            let mut next = fi.clone();
            next.rotation_deg = rot.clamp(-360.0, 360.0);
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
        }
    });
}

fn render_format_image_crop(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, fi: &FormatImageDialogState) {
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.image.crop")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let max_lr = (fi.bounds_pt.width - 2.0).max(0.0);
        let max_tb = (fi.bounds_pt.height - 2.0).max(0.0);
        let mut crop = fi.crop;
        let mut changed = false;

        for (wid, lbl, val_ref, cap) in [
            ("format_image.crop.left", "L", &mut crop.left_pt, (max_lr - fi.crop.right_pt).max(0.0)),
            ("format_image.crop.right", "R", &mut crop.right_pt, (max_lr - fi.crop.left_pt).max(0.0)),
            ("format_image.crop.top", "T", &mut crop.top_pt, (max_tb - fi.crop.bottom_pt).max(0.0)),
            ("format_image.crop.bottom", "B", &mut crop.bottom_pt, (max_tb - fi.crop.top_pt).max(0.0)),
        ] {
            ui.label(format!("{lbl}:"));
            let dv = ui.add(egui::DragValue::new(val_ref).range(0.0..=cap).speed(0.5).suffix(" pt"));
            record_input(ui, &dv, gctx.meta, wid, lbl, &format!("{:.1}", *val_ref), true);
            changed |= dv.changed();
        }
        if changed {
            let mut next = fi.clone();
            next.crop = crop;
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
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
                let mut next = fi.clone();
                match edge {
                    "left" => next.crop.left_pt = (next.crop.left_pt + 4.0).min((next.bounds_pt.width - next.crop.right_pt - 2.0).max(0.0)),
                    "right" => next.crop.right_pt = (next.crop.right_pt + 4.0).min((next.bounds_pt.width - next.crop.left_pt - 2.0).max(0.0)),
                    "top" => next.crop.top_pt = (next.crop.top_pt + 4.0).min((next.bounds_pt.height - next.crop.bottom_pt - 2.0).max(0.0)),
                    _ => next.crop.bottom_pt = (next.crop.bottom_pt + 4.0).min((next.bounds_pt.height - next.crop.top_pt - 2.0).max(0.0)),
                }
                gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
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
            let mut next = fi.clone();
            next.crop = CropInsetsPt::default();
            gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
        }
    });
}

fn render_format_image_border(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, fi: &FormatImageDialogState) {
    let mut border_on = fi.border_enabled;
    let border_lbl = tr(gctx.locale, "r13.image.border");
    let chk = ui.checkbox(&mut border_on, border_lbl);
    record_checkbox(ui, &chk, gctx.meta, "format_image.border.enabled", border_lbl, border_on);
    if chk.changed() {
        let mut next = fi.clone();
        next.border_enabled = border_on;
        gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
    }

    if fi.border_enabled {
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
                        selected: (fi.border_width_pt - w).abs() < 0.05,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fi.clone();
                    next.border_width_pt = w;
                    gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
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
                        selected: fi.border_color == col,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fi.clone();
                    next.border_color = col;
                    gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
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
                        selected: fi.border_dash == dash,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fi.clone();
                    next.border_dash = dash;
                    gctx.intents.push(UiIntent::UpdateFormatImageState(Box::new(next)));
                }
            }
        });
    }
}
