//! Dev-only visual design & structural hierarchy specimen screen (`docs/design-language.md` §7).
//!
//! Renders CurveBook's OKLab/OKLCH derived surface/accent/status tokens, data-only series strip,
//! ranked action toolbar (`Primary` / `Secondary` / `Overflow`), button states, form inputs,
//! growable dropdowns, toggles, tabs, role/provenance badges, tabular data preview, and dialog
//! chrome in a single deterministic view captured across `Light`, `Dark`, and `HighContrast` by
//! `cargo xtask screenshots`.

use curvebook_render::{RenderError, RenderTargetSpec};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2, vec2};

use crate::egui_shell::top_bar_fit::{RankedToolbarSlot, fit_ranked_toolbar};
use crate::egui_shell::{ActionRank, FrameInspectEntryInput, FrameInspectMeta, OKABE_ITO_SERIES_PALETTE, Tokens, widgets};
use crate::inspect::ZLayer;
use crate::theme::{UiThemeMode, UiThemeSpec};

/// Interactive state backing [`render_design_specimen_frame`].
#[derive(Debug, Clone)]
pub struct DesignSpecimenState {
    pub alpha_text: String,
    pub search_query: String,
    pub formula_text: String,
    pub selected_model: usize,
    pub selected_tab: usize,
    pub assume_gaussian: bool,
    pub flag_outliers: bool,
    pub method_choice: usize,
    pub segmented_choice: usize,
    pub live_refit: bool,
    pub details_open: bool,
    pub landscape: bool,
}

impl Default for DesignSpecimenState {
    fn default() -> Self {
        Self {
            alpha_text: String::from("0.05"),
            search_query: String::from("EC50"),
            formula_text: String::from("=LOG10([Conc_M])"),
            selected_model: 0,
            selected_tab: 0,
            assume_gaussian: true,
            flag_outliers: false,
            method_choice: 0,
            segmented_choice: 0,
            live_refit: true,
            details_open: true,
            landscape: true,
        }
    }
}

const MODEL_CHOICES: [&str; 3] = ["Four-Parameter Logistic (4PL)", "Michaelis-Menten Kinetics", "One-Phase Exponential Decay"];

/// Render the complete design-language & hierarchy specimen into `ui` and record widgets in `meta`.
pub fn render_design_specimen_frame(ui: &mut Ui, tokens: &Tokens, state: &mut DesignSpecimenState, meta: &mut FrameInspectMeta) {
    let ctx = ui.ctx().clone();
    ctx.enable_accesskit();
    tokens.apply_to_ctx(&ctx);
    tokens.apply_to_ui(ui);
    meta.begin_pass(&ctx);

    let m = tokens.metrics;
    egui::Panel::top("specimen_top_header")
        .exact_size(m.app_bar_h + m.action_strip_h + m.space_md)
        .frame(
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .inner_margin(egui::Margin::symmetric(m.space_lg as i8, m.space_sm as i8))
                .stroke(Stroke::new(m.stroke_thin_pt, tokens.border)),
        )
        .show(ui, |ui| {
            render_specimen_header_and_toolbar(ui, tokens, meta);
        });

    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tokens.surface_bg).inner_margin(egui::Margin::same(m.space_lg as i8))).show(
        ui,
        |ui| {
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
            egui::ScrollArea::vertical().id_salt("specimen_main_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.columns(2, |cols| {
                    cols[0].spacing_mut().item_spacing.y = m.space_md;
                    render_swatches_card(&mut cols[0], tokens);
                    render_buttons_and_controls_card(&mut cols[0], tokens, state, meta);

                    cols[1].spacing_mut().item_spacing.y = m.space_md;
                    render_inputs_toggles_tabs_card(&mut cols[1], tokens, state, meta);
                    render_tabular_and_badges_card(&mut cols[1], tokens);
                    render_dialog_specimen_card(&mut cols[1], tokens, meta);
                });
            });
        },
    );
}

fn render_specimen_header_and_toolbar(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    let m = tokens.metrics;
    let mode_name = match tokens.mode {
        UiThemeMode::Light => "Light",
        UiThemeMode::Dark => "Dark",
        UiThemeMode::HighContrast => "High Contrast (AAA)",
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = m.space_sm;
        ui.label(RichText::new("CurveBook Design Language & Hierarchy Specimen").size(m.font_ui_title_pt).strong().color(tokens.text_strong));
        paint_pill_badge(ui, tokens, mode_name, tokens.accent_strong, tokens.on_accent);
        let contrast_ok = tokens.contrast_violations().is_empty();
        let wcag_tag = if contrast_ok { "WCAG 2.2 PASS" } else { "WCAG FAIL" };
        let wcag_col = if contrast_ok { tokens.role_fx } else { tokens.error };
        paint_outline_badge(ui, tokens, wcag_tag, wcag_col);
    });

    ui.add_space(m.space_xs);

    let slots = [
        ("toolbar.specimen.analyze", "Analyze", ActionRank::Primary),
        ("toolbar.specimen.graph_type", "Change Graph Type", ActionRank::Secondary),
        ("toolbar.specimen.duplicate", "Duplicate Sheet", ActionRank::Secondary),
        ("toolbar.specimen.freeze", "Freeze Sheet", ActionRank::Overflow),
        ("toolbar.specimen.export", "Export Figure", ActionRank::Overflow),
    ];
    let btn_h = m.min_hit_target_pt + m.space_xs;
    let gap = m.space_xs;
    let overflow_w = m.min_hit_target_pt + m.space_sm;
    let ranked_specs: Vec<RankedToolbarSlot> = slots
        .iter()
        .map(|(_, label, rank)| RankedToolbarSlot {
            desired_width_pt: widgets::flat_button_fit_width(ui, label, m.min_hit_target_pt * 3.0),
            rank: *rank,
        })
        .collect();
    // Reserve ~46% of header width to demonstrate deterministic overflow into `⋯` alongside visible primary/secondary controls.
    let strip_budget = (ui.available_width() * 0.46).max(overflow_w * 3.0);
    let fit = fit_ranked_toolbar(strip_budget, gap, overflow_w, m.min_hit_target_pt, 3, &ranked_specs);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.label(RichText::new("Ranked Strip:").size(m.font_ui_small_pt).color(tokens.text_muted));
        for idx in fit.visible_indices {
            let (wid, label, rank) = slots[idx];
            let w = ranked_specs[idx].desired_width_pt;
            let is_primary = rank == ActionRank::Primary;
            let resp = render_toolbar_chip(ui, tokens, w, btn_h, label, is_primary);
            meta.record_ranked_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: wid,
                    role: "button",
                    label,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                rank,
            );
        }
        if !fit.overflow_indices.is_empty() {
            let ov_label = format!("⋯ +{}", fit.overflow_indices.len());
            let ov_w = widgets::flat_button_fit_width(ui, &ov_label, overflow_w);
            let resp = render_toolbar_chip(ui, tokens, ov_w, btn_h, &ov_label, false);
            meta.record_ranked_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: "toolbar.specimen.overflow_more",
                    role: "button",
                    label: "More toolbar actions",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                ActionRank::Overflow,
            );
        }
    });
}

fn render_toolbar_chip(ui: &mut Ui, tokens: &Tokens, w: f32, h: f32, label: &str, primary: bool) -> egui::Response {
    let m = tokens.metrics;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let bg = if primary {
        if resp.hovered() { tokens.accent_hover } else { tokens.accent_strong }
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let fg = if primary { tokens.on_accent } else { tokens.text_strong };
    let border = if primary { tokens.accent_strong } else { tokens.button_border };
    let radius = CornerRadius::same(m.radius_md);
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(rect, radius, Stroke::new(m.stroke_thin_pt, border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(m.font_ui_body_pt), fg);
    let a11y = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, a11y.clone()));
    resp
}

fn specimen_card(ui: &mut Ui, tokens: &Tokens, title: &str, add_body: impl FnOnce(&mut Ui)) {
    let m = tokens.metrics;
    egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(m.stroke_thin_pt, tokens.border))
        .corner_radius(CornerRadius::same(m.radius_lg))
        .inner_margin(egui::Margin::same(m.space_md as i8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            widgets::subheader(ui, title);
            ui.add_space(m.space_xs);
            add_body(ui);
        });
}

fn render_swatches_card(ui: &mut Ui, tokens: &Tokens) {
    let m = tokens.metrics;
    specimen_card(ui, tokens, "1. Derived OKLab/OKLCH Tokens & Data-First Chroma", |ui| {
        let surfaces = [
            ("chrome", tokens.panel_bg, tokens.text_primary),
            ("recessed", tokens.surface_recessed, tokens.text_primary),
            ("content", tokens.surface_bg, tokens.text_primary),
            ("header", tokens.header_bg, tokens.text_strong),
            ("unemph", tokens.selection_unemphasized_bg, tokens.text_strong),
            ("hover", tokens.hover_bg, tokens.text_strong),
            ("active", tokens.active_bg, tokens.text_strong),
        ];
        let count_f = surfaces.len() as f32;
        let gap = m.space_xs;
        let sw_w = ((ui.available_width() - gap * (count_f - 1.0)) / count_f).max(m.min_hit_target_pt);
        let sw_h = m.min_hit_target_pt + m.space_xs;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (name, bg, fg) in surfaces {
                paint_swatch_box(ui, tokens, sw_w, sw_h, name, bg, fg);
            }
        });

        ui.add_space(m.space_xs);
        let sem_ink = if tokens.surface_bg.r() < 128 { tokens.surface_bg } else { tokens.paper_white };
        let semantic = [
            ("accent", tokens.accent_strong, tokens.on_accent),
            ("focus", tokens.focus_ring, tokens.surface_bg),
            ("error", tokens.error, sem_ink),
            ("warning", tokens.warning, sem_ink),
            ("stale", tokens.stale, sem_ink),
        ];
        let sem_f = semantic.len() as f32;
        let sem_w = ((ui.available_width() - gap * (sem_f - 1.0)) / sem_f).max(m.min_hit_target_pt);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (name, bg, fg) in semantic {
                paint_swatch_box(ui, tokens, sem_w, sw_h, name, bg, fg);
            }
        });

        ui.add_space(m.space_xs);
        egui::Frame::NONE
            .fill(tokens.canvas_mat)
            .corner_radius(CornerRadius::same(m.radius_md))
            .inner_margin(egui::Margin::same(m.space_xs as i8))
            .show(ui, |ui| {
                egui::Frame::NONE
                    .fill(tokens.paper_white)
                    .stroke(Stroke::new(m.stroke_thin_pt, tokens.border))
                    .corner_radius(CornerRadius::same(m.radius_sm))
                    .inner_margin(egui::Margin::symmetric(m.space_sm as i8, m.space_xs as i8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = m.space_xs;
                            let ink = OKABE_ITO_SERIES_PALETTE[0];
                            ui.label(RichText::new("Data-only Okabe-Ito series on paper_white:").size(m.font_ui_small_pt).color(ink));
                            let chip = m.space_md;
                            for series_col in OKABE_ITO_SERIES_PALETTE {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(chip), Sense::hover());
                                ui.painter().rect_filled(r, CornerRadius::same(m.radius_sm), series_col);
                            }
                        });
                    });
            });
    });
}

fn paint_swatch_box(ui: &mut Ui, tokens: &Tokens, w: f32, h: f32, label: &str, bg: Color32, fg: Color32) {
    let m = tokens.metrics;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let radius = CornerRadius::same(m.radius_sm);
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(rect, radius, Stroke::new(m.stroke_thin_pt, tokens.border), StrokeKind::Inside);
    ui.painter().with_clip_rect(rect.shrink(m.space_xxs)).text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(m.font_ui_small_pt),
        fg,
    );
}

fn render_buttons_and_controls_card(ui: &mut Ui, tokens: &Tokens, state: &mut DesignSpecimenState, meta: &mut FrameInspectMeta) {
    let m = tokens.metrics;
    specimen_card(ui, tokens, "2. Buttons, Icon Actions & Hit Targets (>= 24 pt)", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(m.space_sm);
            let p_resp = widgets::primary_button(ui, "Run Fit");
            meta.record_ranked_response(
                ui,
                &p_resp,
                FrameInspectEntryInput {
                    widget_id: "specimen.btn.primary",
                    role: "button",
                    label: "Run Fit",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                ActionRank::Primary,
            );

            let s_resp = widgets::secondary_button(ui, "Duplicate");
            meta.record_ranked_response(
                ui,
                &s_resp,
                FrameInspectEntryInput {
                    widget_id: "specimen.btn.secondary",
                    role: "button",
                    label: "Duplicate",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                ActionRank::Secondary,
            );

            let f_resp = widgets::ghost_button(ui, "Inspect Residuals", false);
            meta.record_ranked_response(
                ui,
                &f_resp,
                FrameInspectEntryInput {
                    widget_id: "specimen.btn.flat",
                    role: "button",
                    label: "Inspect Residuals",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                ActionRank::Secondary,
            );

            let dis_resp = widgets::primary_button_enabled(ui, "Locked", false);
            meta.record_ranked_response(
                ui,
                &dis_resp,
                FrameInspectEntryInput {
                    widget_id: "specimen.btn.disabled",
                    role: "button",
                    label: "Locked",
                    command_id: None,
                    value: None,
                    enabled: false,
                    disabled_reason: Some("Sheet is frozen"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
                ActionRank::Primary,
            );
        });

        ui.add_space(m.space_xs);
        let icon_sz = m.min_hit_target_pt + m.space_xs;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = m.space_sm;
            let search_btn = widgets::icon_button(ui, "search", "Search sheets", false, icon_sz);
            meta.record_response(
                ui,
                &search_btn,
                FrameInspectEntryInput {
                    widget_id: "specimen.icon.search",
                    role: "button",
                    label: "Search sheets",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            let add_btn = widgets::icon_button(ui, "plus", "Add series", true, icon_sz);
            meta.record_response(
                ui,
                &add_btn,
                FrameInspectEntryInput {
                    widget_id: "specimen.icon.add",
                    role: "button",
                    label: "Add series",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if widgets::orientation_button(ui, true, state.landscape, "Landscape page") {
                state.landscape = true;
            }
            if widgets::orientation_button(ui, false, !state.landscape, "Portrait page") {
                state.landscape = false;
            }
            let _ = widgets::layout_button(ui, &[(0, 0), (1, 0)], true, "Two-panel row");
            let link_resp = widgets::link_label(ui, "Reset defaults");
            let _ = link_resp;
        });
    });
}

fn render_inputs_toggles_tabs_card(ui: &mut Ui, tokens: &Tokens, state: &mut DesignSpecimenState, meta: &mut FrameInspectMeta) {
    let m = tokens.metrics;
    let label_col_w = m.min_hit_target_pt * 5.0;
    specimen_card(ui, tokens, "3. Fields, Growable Dropdown, Toggles & Tabs", |ui| {
        let field_w = (ui.available_width() - label_col_w - m.space_md).clamp(m.min_hit_target_pt * 6.0, m.min_hit_target_pt * 10.0);
        widgets::label_row(ui, "Model family", label_col_w, |ui| {
            let current_model = MODEL_CHOICES[state.selected_model.min(MODEL_CHOICES.len() - 1)];
            let dd = widgets::dropdown("specimen_model_dd", current_model).width(field_w).show_ui(ui, |ui| {
                for (idx, choice) in MODEL_CHOICES.iter().enumerate() {
                    if ui.selectable_label(state.selected_model == idx, *choice).clicked() {
                        state.selected_model = idx;
                    }
                }
            });
            meta.record_response(
                ui,
                &dd.response,
                FrameInspectEntryInput {
                    widget_id: "inspector.specimen.model_dropdown",
                    role: "combobox",
                    label: "Model family",
                    command_id: None,
                    value: Some(serde_json::Value::String(String::from(current_model))),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: dd.ellipsized,
                    full_label: dd.ellipsized.then_some(current_model),
                },
            );
        });

        widgets::label_row(ui, "Significance alpha", label_col_w, |ui| {
            let te_resp = ui.add_sized(vec2(field_w, m.min_hit_target_pt), TextEdit::singleline(&mut state.alpha_text));
            meta.record_response(
                ui,
                &te_resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.specimen.alpha_input",
                    role: "textbox",
                    label: "Significance alpha",
                    command_id: None,
                    value: Some(serde_json::Value::String(state.alpha_text.clone())),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        });

        widgets::label_row(ui, "Derived column", label_col_w, |ui| {
            let ro_resp = ui.add_enabled_ui(false, |ui| {
                ui.add_sized(
                    vec2(field_w, m.min_hit_target_pt),
                    TextEdit::singleline(&mut state.formula_text).font(FontId::monospace(m.font_grid_numeral_pt)),
                )
            });
            meta.record_response(
                ui,
                &ro_resp.inner,
                FrameInspectEntryInput {
                    widget_id: "inspector.specimen.formula_readonly",
                    role: "textbox",
                    label: "Derived column",
                    command_id: None,
                    value: Some(serde_json::Value::String(state.formula_text.clone())),
                    enabled: false,
                    disabled_reason: Some("Calculated column formula"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        });

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = m.space_md;
            if widgets::check(ui, "Assume Gaussian", state.assume_gaussian, true) {
                state.assume_gaussian = !state.assume_gaussian;
            }
            if widgets::check(ui, "Flag ROUT outliers", state.flag_outliers, true) {
                state.flag_outliers = !state.flag_outliers;
            }
            let _ = widgets::check3(ui, "Partial replicate set", None, true);
        });

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = m.space_md;
            if widgets::radio(ui, "Ordinary Least Squares", state.method_choice == 0, true) {
                state.method_choice = 0;
            }
            if widgets::radio(ui, "Robust Huber Regression", state.method_choice == 1, true) {
                state.method_choice = 1;
            }
            if widgets::switch(ui, "Live refit", state.live_refit, true) {
                state.live_refit = !state.live_refit;
            }
        });

        ui.add_space(m.space_xs);
        if let Some(seg) = widgets::segmented_control(ui, &["Linear", "Log10", "Log2"], state.segmented_choice) {
            state.segmented_choice = seg;
        }

        if let Some(tab_idx) = widgets::tab_bar(ui, &["Summary", "Parameters", "Diagnostics", "Residuals"], state.selected_tab) {
            state.selected_tab = tab_idx;
        }
    });
}

fn render_tabular_and_badges_card(ui: &mut Ui, tokens: &Tokens) {
    let m = tokens.metrics;
    specimen_card(ui, tokens, "4. Role Badges & Tabular Result Preview", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = m.space_sm;
            paint_neutral_role_badge(ui, tokens, "NUM");
            paint_outline_badge(ui, tokens, "CAT", tokens.role_cat);
            paint_outline_badge(ui, tokens, "LBL", tokens.role_lbl);
            paint_outline_badge(ui, tokens, "FX=", tokens.role_fx);
            paint_outline_badge(ui, tokens, "STALE", tokens.stale);
            paint_outline_badge(ui, tokens, "WARN", tokens.warning);
            paint_outline_badge(ui, tokens, "ERR", tokens.error);
        });

        ui.add_space(m.space_xs);
        let rows: [(&str, &str, &str, &str); 3] = [
            ("LogEC50", "-6.850", "-6.942 to -6.758", "< 0.0001"),
            ("HillSlope", " 1.150", " 1.012 to  1.288", "< 0.0001"),
            ("Span (%)", "88.020", "85.410 to 90.630", "< 0.0001"),
        ];
        let table_w = ui.available_width();
        let row_h = m.grid_row_h;
        let col_w = table_w / 4.0;
        egui::Frame::NONE.stroke(Stroke::new(m.stroke_thin_pt, tokens.border)).corner_radius(CornerRadius::same(m.radius_sm)).show(ui, |ui| {
            let (hdr_rect, _) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::hover());
            ui.painter().rect_filled(hdr_rect, CornerRadius::ZERO, tokens.header_bg);
            for (c_idx, h_lbl) in ["Parameter", "Estimate", "95% CI", "P value"].iter().enumerate() {
                let x = hdr_rect.left() + (c_idx as f32) * col_w;
                let cell_r = Rect::from_min_size(pos2(x, hdr_rect.top()), vec2(col_w, row_h));
                let align = if c_idx == 0 { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER };
                let anchor = if c_idx == 0 {
                    pos2(cell_r.left() + m.space_sm, cell_r.center().y)
                } else {
                    pos2(cell_r.right() - m.space_sm, cell_r.center().y)
                };
                ui.painter().text(anchor, align, *h_lbl, FontId::proportional(m.font_ui_small_pt), tokens.text_strong);
            }
            ui.painter().hline(hdr_rect.x_range(), hdr_rect.bottom(), Stroke::new(m.stroke_thin_pt, tokens.border));

            let num_font = crate::egui_shell::fonts::aligned_numeral_font_id(ui.ctx(), tokens);
            for (r_idx, (param, est, ci, pval)) in rows.iter().enumerate() {
                let (r_rect, _) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::hover());
                let fill = if r_idx % 2 == 1 { tokens.panel_bg } else { tokens.surface_bg };
                ui.painter().rect_filled(r_rect, CornerRadius::ZERO, fill);
                for (c_idx, val) in [param, est, ci, pval].iter().enumerate() {
                    let x = r_rect.left() + (c_idx as f32) * col_w;
                    let cell_r = Rect::from_min_size(pos2(x, r_rect.top()), vec2(col_w, row_h));
                    if r_idx == 0 && c_idx == 1 {
                        ui.painter().rect_stroke(
                            cell_r.shrink(1.0),
                            CornerRadius::ZERO,
                            Stroke::new(m.stroke_focus_pt, tokens.focus_ring),
                            StrokeKind::Inside,
                        );
                    }
                    let (anchor, align, font, col) = if c_idx == 0 {
                        (
                            pos2(cell_r.left() + m.space_sm, cell_r.center().y),
                            Align2::LEFT_CENTER,
                            FontId::proportional(m.font_ui_body_pt),
                            tokens.text_primary,
                        )
                    } else {
                        (pos2(cell_r.right() - m.space_sm, cell_r.center().y), Align2::RIGHT_CENTER, num_font.clone(), tokens.text_strong)
                    };
                    ui.painter().text(anchor, align, **val, font, col);
                }
                if r_idx + 1 < rows.len() {
                    ui.painter().hline(r_rect.x_range(), r_rect.bottom(), Stroke::new(m.stroke_thin_pt, tokens.divider));
                }
            }
        });
    });
}

fn render_dialog_specimen_card(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    let m = tokens.metrics;
    egui::Frame::NONE
        .fill(tokens.header_bg)
        .stroke(Stroke::new(m.stroke_thin_pt, tokens.border))
        .corner_radius(CornerRadius::same(m.radius_lg))
        .inner_margin(egui::Margin::same(m.space_md as i8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("5. Dialog Chrome: Parameters — Nonlinear Regression").size(m.font_ui_title_pt).strong().color(tokens.text_strong),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_resp = widgets::icon_button(ui, "close", "Close dialog", false, m.min_hit_target_pt);
                    meta.record_response(
                        ui,
                        &close_resp,
                        FrameInspectEntryInput {
                            widget_id: "inspector.specimen.dialog_close",
                            role: "button",
                            label: "Close dialog",
                            command_id: None,
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                });
            });

            ui.add_space(m.space_xs);
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(m.stroke_thin_pt, tokens.warning))
                .corner_radius(CornerRadius::same(m.radius_md))
                .inner_margin(egui::Margin::symmetric(m.space_sm as i8, m.space_xs as i8))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Diagnostics notice: 1 replicate excluded with recorded audit note.")
                            .size(m.font_ui_small_pt)
                            .color(tokens.warning),
                    );
                });

            widgets::divider(ui);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = m.space_sm;
                let cancel_resp = widgets::secondary_button(ui, "Cancel");
                meta.record_ranked_response(
                    ui,
                    &cancel_resp,
                    FrameInspectEntryInput {
                        widget_id: "inspector.specimen.dialog_cancel",
                        role: "button",
                        label: "Cancel",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                    ActionRank::Secondary,
                );
                let ok_resp = widgets::primary_button(ui, "Apply & Run");
                meta.record_ranked_response(
                    ui,
                    &ok_resp,
                    FrameInspectEntryInput {
                        widget_id: "inspector.specimen.dialog_run",
                        role: "button",
                        label: "Apply & Run",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                    ActionRank::Primary,
                );
            });
        });
}

fn paint_pill_badge(ui: &mut Ui, tokens: &Tokens, text: &str, bg: Color32, fg: Color32) {
    let m = tokens.metrics;
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(m.font_ui_small_pt), fg);
    let sz = vec2(galley.size().x + m.space_md, (galley.size().y + m.space_xs).max(m.space_lg + m.space_xs));
    let (rect, _) = ui.allocate_exact_size(sz, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(m.radius_sm), bg);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, fg);
}

fn paint_neutral_role_badge(ui: &mut Ui, tokens: &Tokens, text: &str) {
    let m = tokens.metrics;
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(m.font_ui_small_pt), tokens.role_num);
    let sz = vec2(galley.size().x + m.space_sm, (galley.size().y + m.space_xs).max(m.space_lg + m.space_xs));
    let (rect, _) = ui.allocate_exact_size(sz, Sense::hover());
    let radius = CornerRadius::same(m.radius_sm);
    ui.painter().rect_filled(rect, radius, tokens.surface_recessed);
    ui.painter().rect_stroke(rect, radius, Stroke::new(m.stroke_thin_pt, tokens.border), StrokeKind::Inside);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, tokens.role_num);
}

fn paint_outline_badge(ui: &mut Ui, tokens: &Tokens, text: &str, color: Color32) {
    let m = tokens.metrics;
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(m.font_ui_small_pt), color);
    let sz = vec2(galley.size().x + m.space_sm, (galley.size().y + m.space_xs).max(m.space_lg + m.space_xs));
    let (rect, _) = ui.allocate_exact_size(sz, Sense::hover());
    let radius = CornerRadius::same(m.radius_sm);
    ui.painter().rect_filled(rect, radius, tokens.panel_bg);
    ui.painter().rect_stroke(rect, radius, Stroke::new(m.stroke_thin_pt, color), StrokeKind::Inside);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, color);
}

/// Render the design specimen screen into an 8-bit RGBA PNG at `target` for `mode` (`R15.10`).
///
/// # Errors
/// Returns [`RenderError`] if `target` dimensions are invalid or PNG encoding fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_design_specimen_png(mode: UiThemeMode, locale: &str, target: RenderTargetSpec) -> Result<Vec<u8>, RenderError> {
    use crate::egui_shell::rasterizer::{EguiScreenshotRegions, EguiSoftwareRasterizer, pixels_per_point_for_dpi};
    use crate::inspect::ScreenshotRegionTarget;

    let ppp = pixels_per_point_for_dpi(target.dpi);
    #[allow(clippy::cast_precision_loss)]
    let w_pt = (target.width_px.max(1) as f32) / ppp;
    #[allow(clippy::cast_precision_loss)]
    let h_pt = (target.height_px.max(1) as f32) / ppp;

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    super::fonts::install_default_egui_fonts_cached(&ctx);

    let spec = UiThemeSpec::for_mode(mode);
    let tokens = Tokens::from_theme_spec(&spec, locale);
    tokens.apply_theme_to_ctx(&ctx);
    ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.animation_time = 0.0;
    });

    let mut rasterizer = EguiSoftwareRasterizer::new();
    let mut state = DesignSpecimenState::default();
    let mut meta = FrameInspectMeta::new();
    let mut final_output: Option<egui::FullOutput> = None;

    for pass in 0..3_u32 {
        let raw_input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w_pt, h_pt))),
            time: Some(f64::from(pass) * (1.0 / 60.0)),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw_input, |ui| {
            render_design_specimen_frame(ui, &tokens, &mut state, &mut meta);
        });
        if pass < 2 {
            rasterizer.consume_textures_set(&mut out.textures_delta);
            rasterizer.consume_textures_free(&mut out.textures_delta);
        } else {
            final_output = Some(out);
        }
    }

    let mut out = final_output.ok_or_else(|| RenderError::InvalidTarget { message: String::from("specimen frame produced no output") })?;
    let (png, _, _) = rasterizer.render_egui_screenshot_png(
        &ctx,
        &mut out,
        target,
        ScreenshotRegionTarget::FullWindow,
        EguiScreenshotRegions::default(),
        tokens.surface_bg,
    )?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::{DesignSpecimenState, render_design_specimen_frame, render_design_specimen_png};
    use crate::egui_shell::Tokens;
    use crate::egui_shell::driver::{EguiHumanPathDriver, RegionBudgets, build_driver_font_definitions, check_region_control_budgets};
    use crate::interaction::ViewportMetrics;
    use crate::theme::{UiThemeMode, UiThemeSpec};
    use curvebook_render::RenderTargetSpec;

    #[test]
    fn specimen_screen_passes_parity_across_all_themes_and_zoom_levels() {
        for mode in UiThemeMode::ALL {
            for (w, h, scale) in [(1440.0, 900.0, 1.0_f32), (1280.0, 840.0, 2.0_f32)] {
                let mut spec = UiThemeSpec::for_mode(mode);
                spec.ui_scale = scale;
                let tokens = Tokens::from_theme_spec(&spec, "en");
                let state = DesignSpecimenState::default();
                let tok_for_ui = tokens;
                let driver = EguiHumanPathDriver::with_tokens_and_fonts(
                    state,
                    ViewportMetrics::new(w, h, 1.0),
                    tokens,
                    build_driver_font_definitions(),
                    move |ui, meta, st| {
                        render_design_specimen_frame(ui, &tok_for_ui, st, meta);
                    },
                );
                let report = driver.verify_pixel_to_hit_box_parity();
                assert!(report.is_clean(), "specimen parity violations in {mode:?} ({w}x{h} @{scale}x): {:?}", report.violations);
                let budget_violations = check_region_control_budgets(driver.frame_meta(), &RegionBudgets::CANONICAL);
                assert!(budget_violations.is_empty(), "specimen budget violations in {mode:?}: {budget_violations:?}");
            }
        }
    }

    #[test]
    fn specimen_png_renders_valid_1440x900_in_light_dark_and_high_contrast() {
        let target = RenderTargetSpec { width_px: 1440, height_px: 900, dpi: 96 };
        let light = render_design_specimen_png(UiThemeMode::Light, "en", target).expect("light specimen PNG");
        let dark = render_design_specimen_png(UiThemeMode::Dark, "en", target).expect("dark specimen PNG");
        let hc = render_design_specimen_png(UiThemeMode::HighContrast, "en", target).expect("high-contrast specimen PNG");
        for bytes in [&light, &dark, &hc] {
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(bytes.len() > 2048);
        }
        assert_ne!(light, dark);
        assert_ne!(dark, hc);
    }
}
