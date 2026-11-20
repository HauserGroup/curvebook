//! Reactive data transformation, normalization, baseline removal, MV filter/transform,
//! extract & rearrange, and exclusion-with-reason modals in `egui 0.37` (`R15.4`,
//! `gui-layout-spec.md` §8.6).

use std::borrow::Cow;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, DataSheet, DerivedColumnFormula, ErrorPropagationMode, MathTransformPreset, SheetId,
    TableArchetype, TableKind, TransformAnalysisSpec,
};
use curvebook_transform::formula::parser::parse_formula;
use egui::{Align, Context, Layout, Rect, RichText, Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ExcludeWithReasonDialogState};
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{normalize_locale, try_tr};
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, widgets};

/// Specific data-reshaping / transformation dialog open natively in `egui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransformModalKind {
    TransformValues,
    Normalize,
    RemoveBaseline,
    Transpose,
    PruneRows,
    MvFilterAndTransform,
    ExtractAndRearrange,
    Simulate,
    RandomSample,
}

/// Conjunctive row filter row in `MV Filter & Transform`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MvFilterPredicateRow {
    pub left_col: usize,
    pub op: String,
    pub right_value_or_col: String,
    pub compare_variable: bool,
}

/// Selected constant-K arithmetic mode for `TransformValues` Y transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstantKOp {
    /// `Y + K` (`LinearScale { slope: 1.0, intercept: K }`).
    Add,
    /// `Y * K` (`LinearScale { slope: K, intercept: 0.0 }`).
    Multiply,
    /// `Y / K` (`LinearScale { slope: 1.0 / K, intercept: 0.0 }`, requires `K != 0`).
    Divide,
}

impl ConstantKOp {
    fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "affine_k" => Some(Self::Add),
            "multiply_k" => Some(Self::Multiply),
            "divide_k" => Some(Self::Divide),
            _ => None,
        }
    }

    fn infer_from_preset(preset: MathTransformPreset) -> Option<(Self, f64)> {
        let MathTransformPreset::LinearScale { slope, intercept } = preset else {
            return None;
        };
        if (slope - 1.0).abs() < 1e-12 && intercept.abs() >= 1e-12 {
            Some((Self::Add, intercept))
        } else if intercept.abs() < 1e-12 && slope > 0.0 && slope < 1.0 {
            Some((Self::Divide, 1.0 / slope))
        } else if intercept.abs() < 1e-12 {
            Some((Self::Multiply, slope))
        } else {
            Some((Self::Add, intercept))
        }
    }
}

/// Persistent transient UI state for `render_transform_modal` and `render_exclude_with_reason_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransformModalsEguiState {
    pub open_kind: Option<TransformModalKind>,
    pub target_sheet_id: Option<SheetId>,
    pub active_tab: usize,
    pub formula_draft: String,
    pub constant_k_draft: String,
    pub selected_k_op: Option<ConstantKOp>,
    pub synced_edit_sheet_id: Option<String>,
    pub filter_rows: Vec<MvFilterPredicateRow>,
    pub exclude_reason_draft: String,
    pub validation_error: Option<String>,
    pub standalone_x_preset: Option<String>,
    pub standalone_y_preset: Option<String>,
    pub standalone_error_propagation: ErrorPropagationMode,
    pub pharmacology_mode: bool,
    pub baseline_cols: Vec<usize>,
    pub baseline_op: usize,
    pub derived_title_draft: String,
    pub derived_cols: Vec<(String, String)>,
    pub extract_target_kind: Option<TableKind>,
    pub extract_binding_x: String,
    pub extract_binding_y: String,
    pub extract_binding_grp: String,
    pub norm_zero_idx: usize,
    pub norm_hundred_idx: usize,
    pub norm_percent: bool,
    pub norm_per_dataset: bool,
    pub hook_picker_open: bool,
    pub exclude_note_draft: String,
    pub synced_exclude_tag: String,
}

/// Frame geometry and modal capture report emitted by transform modals.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

fn tr_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> Cow<'a, str> {
    let norm = normalize_locale(locale);
    if let Some(s) = try_tr(norm, key).or_else(|| try_tr("en", key)) { Cow::Borrowed(s) } else { Cow::Borrowed(fallback) }
}

/// Return `true` when `ActiveDialog::EditAnalysisParameters` edits a `Transform` analysis spec.
#[must_use]
pub fn is_transform_edit_dialog_active(app: &UiApp) -> bool {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return false;
    };
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return matches!(spec.kind, AnalysisKind::Transform(_));
    }
    app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(_)))
}

/// Render reactive Transform / Remove Baseline / MV Filter / Extract modals.
pub fn render_transform_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut TransformModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TransformModalFrameReport {
    let from_edit = is_transform_edit_dialog_active(app);
    if !from_edit {
        state.synced_edit_sheet_id = None;
    }
    if !from_edit && state.open_kind.is_none() {
        return TransformModalFrameReport::default();
    }
    if from_edit {
        sync_transform_edit_state(app, state);
    }

    let draft_kind = if from_edit { app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) } else { None };
    let effective_kind = resolve_effective_modal_kind(draft_kind, state.open_kind);
    let title = modal_title_for(effective_kind, tokens.locale);

    let vp = ctx.content_rect();
    let dlg_w = (vp.width() - 24.0).clamp(320.0, 620.0);
    let dlg_h = (vp.height() - 24.0).clamp(240.0, 500.0);

    let source_sheet = resolve_source_sheet(app, state, from_edit);
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_transform_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (dlg_w - 24.0).max(280.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(dlg_h);

        ui.heading(RichText::new(title.as_ref()).color(tokens.text_primary));
        ui.add_space(4.0);
        widgets::divider(ui);

        let body_h = (dlg_h - 92.0).max(110.0);
        egui::ScrollArea::vertical().id_salt("transform_modal_scroll").max_width(inner_w).max_height(body_h).auto_shrink([false, false]).show(
            ui,
            |body| {
                body.set_max_width((inner_w - 18.0).max(200.0));
                match effective_kind {
                    TransformModalKind::TransformValues => {
                        render_transform_values_body(body, app, source_sheet, state, from_edit, tokens, meta, intents)
                    }
                    TransformModalKind::Normalize => render_normalize_body(body, app, source_sheet, state, from_edit, tokens, meta, intents),
                    TransformModalKind::RemoveBaseline => {
                        render_remove_baseline_body(body, app, source_sheet, state, from_edit, tokens, meta, intents)
                    }
                    TransformModalKind::MvFilterAndTransform => {
                        render_mv_filter_transform_body(body, app, source_sheet, state, from_edit, tokens, meta, intents)
                    }
                    TransformModalKind::ExtractAndRearrange
                    | TransformModalKind::Transpose
                    | TransformModalKind::PruneRows
                    | TransformModalKind::Simulate
                    | TransformModalKind::RandomSample => {
                        render_extract_and_rearrange_body(body, app, source_sheet, effective_kind, state, from_edit, tokens, meta, intents)
                    }
                }

                if state.hook_picker_open {
                    body.add_space(6.0);
                    render_inline_hook_picker(body, app, state, tokens, meta);
                }

                if let Some(err) = &state.validation_error {
                    body.add_space(4.0);
                    body.label(RichText::new(err).color(tokens.link_red).size(11.5));
                }
            },
        );

        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        render_transform_footer(ui, app, source_sheet, effective_kind, state, from_edit, tokens, meta, intents);
    });

    let title_for_ak = title.as_ref().to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.transform", title: title.as_ref(), z_dialog: ZLayer::ModalDialog },
        meta,
    );
    TransformModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn resolve_effective_modal_kind(draft_kind: Option<&AnalysisKind>, open_kind: Option<TransformModalKind>) -> TransformModalKind {
    if let Some(AnalysisKind::Transform(spec)) = draft_kind {
        return match spec {
            TransformAnalysisSpec::Math { .. } => TransformModalKind::TransformValues,
            TransformAnalysisSpec::Normalize { .. } => TransformModalKind::Normalize,
            TransformAnalysisSpec::RemoveBaseline { .. } => TransformModalKind::RemoveBaseline,
            TransformAnalysisSpec::SelectAndTransform { .. } => TransformModalKind::MvFilterAndTransform,
            TransformAnalysisSpec::Transpose { .. } => TransformModalKind::Transpose,
            TransformAnalysisSpec::PruneRows { .. } => TransformModalKind::PruneRows,
            TransformAnalysisSpec::Simulate { .. } => TransformModalKind::Simulate,
            TransformAnalysisSpec::RandomSample { .. } => TransformModalKind::RandomSample,
        };
    }
    open_kind.unwrap_or(TransformModalKind::TransformValues)
}

fn modal_title_for(kind: TransformModalKind, locale: &str) -> Cow<'_, str> {
    match kind {
        TransformModalKind::TransformValues => Cow::Borrowed("Transform Values & Concentrations"),
        TransformModalKind::Normalize => Cow::Borrowed("Normalize Data"),
        TransformModalKind::RemoveBaseline => tr_tr(locale, "analysis.remove_baseline", "Remove Baseline & Column Math"),
        TransformModalKind::MvFilterAndTransform => Cow::Borrowed("Multiple Variables: Filter & Transform"),
        TransformModalKind::ExtractAndRearrange => Cow::Borrowed("Extract & Rearrange Table"),
        TransformModalKind::Transpose => Cow::Borrowed("Transpose Table"),
        TransformModalKind::PruneRows => Cow::Borrowed("Prune Rows"),
        TransformModalKind::Simulate => Cow::Borrowed("Simulate XY Curve"),
        TransformModalKind::RandomSample => Cow::Borrowed("Random Sample Rows"),
    }
}

fn resolve_source_sheet<'a>(app: &'a UiApp, state: &TransformModalsEguiState, from_edit: bool) -> Option<&'a DataSheet> {
    let doc = app.document();
    if from_edit && let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return doc.data_sheet(&spec.source_sheet_id);
    }
    state
        .target_sheet_id
        .as_ref()
        .and_then(|id| doc.data_sheet(id))
        .or_else(|| doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)))
        .or_else(|| doc.data_sheets.first())
}

fn sync_transform_edit_state(app: &UiApp, state: &mut TransformModalsEguiState) {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return;
    };
    if state.synced_edit_sheet_id.as_deref() == Some(result_sheet_id.as_str()) {
        return;
    }
    state.synced_edit_sheet_id = Some(result_sheet_id.as_str().to_owned());
    let spec = app.canvas_dialogs.edit_params.draft.as_ref().or_else(|| app.document().result_sheet(result_sheet_id).map(|rs| &rs.analysis));
    if let Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. })) = spec.map(|s| &s.kind) {
        if let Some((op, k)) = y_transform.and_then(ConstantKOp::infer_from_preset) {
            state.selected_k_op = Some(op);
            state.constant_k_draft = format!("{k}");
        } else {
            state.selected_k_op = None;
        }
    } else {
        state.selected_k_op = None;
    }
}

fn parse_k_constant(raw: &str, locale_preset: &crate::i18n::LocalePreset) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    parse_localized_number_input(trimmed, locale_preset).or_else(|| trimmed.parse::<f64>().ok()).filter(|v| v.is_finite())
}

fn build_linear_scale_for_k(op: ConstantKOp, raw_k: &str, locale_preset: &crate::i18n::LocalePreset) -> Result<MathTransformPreset, String> {
    let trimmed = raw_k.trim();
    if trimmed.is_empty() {
        return Err(String::from("Enter a numeric value for Constant K."));
    }
    let Some(k) = parse_k_constant(trimmed, locale_preset) else {
        return Err(format!("Invalid numeric constant `{trimmed}`."));
    };
    match op {
        ConstantKOp::Add => Ok(MathTransformPreset::LinearScale { slope: 1.0, intercept: k }),
        ConstantKOp::Multiply => Ok(MathTransformPreset::LinearScale { slope: k, intercept: 0.0 }),
        ConstantKOp::Divide => {
            if k.abs() <= f64::EPSILON {
                Err(String::from("Constant K cannot be 0 when dividing (Y / K)."))
            } else {
                Ok(MathTransformPreset::LinearScale { slope: 1.0 / k, intercept: 0.0 })
            }
        }
    }
}

fn validate_transform_k(active_k_op: Option<ConstantKOp>, raw_k: &str, locale_preset: &crate::i18n::LocalePreset) -> Option<String> {
    if let Some(op) = active_k_op {
        return build_linear_scale_for_k(op, raw_k, locale_preset).err();
    }
    let trimmed = raw_k.trim();
    if !trimmed.is_empty() && parse_k_constant(trimmed, locale_preset).is_none() {
        return Some(format!("Invalid numeric constant `{trimmed}`."));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn render_transform_values_body(
    ui: &mut Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    if !from_edit {
        state.selected_k_op = state.standalone_y_preset.as_deref().and_then(ConstantKOp::from_slug);
    }
    let active_k_op = state.selected_k_op;
    let (cur_x, cur_y, cur_ep) = match draft_spec.as_ref().map(|s| &s.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation })) if from_edit => {
            let eff_y = active_k_op.map(|op| build_linear_scale_for_k(op, &state.constant_k_draft, &app.locale_preset).ok()).unwrap_or(*y_transform);
            (*x_transform, eff_y, *error_propagation)
        }
        _ => {
            let eff_y = match active_k_op {
                Some(op) => build_linear_scale_for_k(op, &state.constant_k_draft, &app.locale_preset).ok(),
                None => state.standalone_y_preset.as_deref().and_then(parse_math_slug),
            };
            (state.standalone_x_preset.as_deref().and_then(parse_math_slug), eff_y, state.standalone_error_propagation)
        }
    };

    ui.horizontal_wrapped(|row| {
        let std_r = choice_pill(row, tokens, "Standard Functions", !state.pharmacology_mode, true);
        record_tr_node(row, meta, "edit.transform.mode.standard", "tab", "Standard Functions", None, true, &std_r);
        if std_r.clicked() {
            state.pharmacology_mode = false;
        }
        let pharm_r = choice_pill(row, tokens, "Pharmacology (-log10[M])", state.pharmacology_mode, true);
        record_tr_node(row, meta, "edit.transform.mode.pharmacology", "tab", "Pharmacology", None, true, &pharm_r);
        if pharm_r.clicked() {
            state.pharmacology_mode = true;
            if from_edit {
                apply_math_patch(
                    draft_spec.as_mut(),
                    Some(Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 })),
                    None,
                    None,
                    intents,
                );
            } else {
                state.standalone_x_preset = Some(String::from("neg_log_m_um"));
            }
        }
    });

    ui.add_space(4.0);
    // Transform X choices (`edit.transform.x_transform`).
    ui.label(RichText::new("Transform X values:").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        let x_choices = [
            ("none", "None", None),
            ("log10", "log10(X)", Some(MathTransformPreset::Log10)),
            ("ln", "ln(X)", Some(MathTransformPreset::Ln)),
            ("exp10", "10^X", Some(MathTransformPreset::Exp10)),
            ("neg_log_m_um", "uM -> -log10[M]", Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 })),
            ("neg_log_m_nm", "nM -> -log10[M]", Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-9 })),
        ];
        for (slug, lbl, preset_opt) in x_choices {
            let active = cur_x == preset_opt;
            let r = choice_pill(row, tokens, lbl, active, true);
            record_tr_node(row, meta, &format!("edit.transform.x_transform.{slug}"), "radio", lbl, Some(active.to_string()), true, &r);
            if r.clicked() {
                if from_edit {
                    apply_math_patch(draft_spec.as_mut(), Some(preset_opt), None, None, intents);
                } else {
                    state.standalone_x_preset = (slug != "none").then(|| slug.to_owned());
                }
            }
        }
        let legacy_x = widgets::secondary_button(row, "Cycle X Transform");
        record_tr_node(row, meta, "edit.transform.x_transform", "button", "Transform X", None, true, &legacy_x);
        if legacy_x.clicked() {
            let next = match cur_x {
                None => Some(MathTransformPreset::Log10),
                Some(MathTransformPreset::Log10) => Some(MathTransformPreset::Ln),
                Some(MathTransformPreset::Ln) => Some(MathTransformPreset::Exp10),
                _ => None,
            };
            if from_edit {
                apply_math_patch(draft_spec.as_mut(), Some(next), None, None, intents);
            } else {
                state.standalone_x_preset = next.map(math_preset_to_slug);
            }
        }
    });

    ui.add_space(6.0);
    // Transform Y choices (`edit.transform.y_transform`).
    ui.label(RichText::new("Transform Y values:").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        let y_choices = [
            ("none", "None", None),
            ("log10", "log10(Y)", Some(MathTransformPreset::Log10)),
            ("ln", "ln(Y)", Some(MathTransformPreset::Ln)),
            ("log2", "log2(Y)", Some(MathTransformPreset::Log2)),
            ("exp10", "10^Y", Some(MathTransformPreset::Exp10)),
            ("reciprocal", "1/Y", Some(MathTransformPreset::Reciprocal)),
            ("sqrt", "sqrt(Y)", Some(MathTransformPreset::Sqrt)),
            ("logit", "logit(Y)", Some(MathTransformPreset::Logit)),
            ("affine_k", "Y + K", None),
            ("multiply_k", "Y * K", None),
            ("divide_k", "Y / K", None),
        ];
        for (slug, lbl, preset_opt) in y_choices {
            let active = match ConstantKOp::from_slug(slug) {
                Some(k_op) => active_k_op == Some(k_op),
                None => active_k_op.is_none() && cur_y == preset_opt,
            };
            let r = choice_pill(row, tokens, lbl, active, true);
            record_tr_node(row, meta, &format!("edit.transform.y_transform.{slug}"), "radio", lbl, Some(active.to_string()), true, &r);
            if r.clicked() {
                if let Some(k_op) = ConstantKOp::from_slug(slug) {
                    state.selected_k_op = Some(k_op);
                    if from_edit {
                        if let Ok(linear) = build_linear_scale_for_k(k_op, &state.constant_k_draft, &app.locale_preset) {
                            apply_math_patch(draft_spec.as_mut(), None, Some(Some(linear)), None, intents);
                        }
                    } else {
                        state.standalone_y_preset = Some(slug.to_owned());
                    }
                } else {
                    state.selected_k_op = None;
                    if from_edit {
                        apply_math_patch(draft_spec.as_mut(), None, Some(preset_opt), None, intents);
                    } else {
                        state.standalone_y_preset = (slug != "none").then(|| slug.to_owned());
                    }
                }
            }
        }
        let legacy_y = widgets::secondary_button(row, "Cycle Y Transform");
        record_tr_node(row, meta, "edit.transform.y_transform", "button", "Transform Y", None, true, &legacy_y);
        if legacy_y.clicked() {
            let next = match cur_y {
                Some(MathTransformPreset::Log10) => Some(MathTransformPreset::Ln),
                Some(MathTransformPreset::Ln) => Some(MathTransformPreset::Sqrt),
                Some(MathTransformPreset::Sqrt) => None,
                _ => Some(MathTransformPreset::Log10),
            };
            state.selected_k_op = None;
            if from_edit {
                apply_math_patch(draft_spec.as_mut(), None, Some(next), None, intents);
            } else {
                state.standalone_y_preset = next.map(math_preset_to_slug);
            }
        }
    });

    ui.add_space(4.0);
    // Constant `K` & Hook Constant (`🔗`).
    ui.horizontal(|row| {
        row.label("Constant K:");
        let kr = row.add(egui::TextEdit::singleline(&mut state.constant_k_draft).hint_text("0.0").desired_width(96.0));
        record_tr_node(row, meta, "edit.transform.constant_k", "textbox", "Constant K", Some(state.constant_k_draft.clone()), true, &kr);
        if kr.changed()
            && from_edit
            && let Some(k_op) = state.selected_k_op
            && let Ok(linear) = build_linear_scale_for_k(k_op, &state.constant_k_draft, &app.locale_preset)
        {
            apply_math_patch(draft_spec.as_mut(), None, Some(Some(linear)), None, intents);
        }
        let hr = widgets::secondary_button(row, "Hook Constant");
        record_tr_node(row, meta, "edit.transform.hook_k", "button", "Hook Constant", None, true, &hr);
        if hr.clicked() {
            state.hook_picker_open = !state.hook_picker_open;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("Error Propagation:");
        for (mode, slug, lbl) in [
            (ErrorPropagationMode::ReplicateEndpoints, "asymmetric", "Asymmetric Endpoints"),
            (ErrorPropagationMode::FirstOrderDeltaMethod, "delta", "Delta Method SE"),
        ] {
            let active = cur_ep == mode;
            let r = choice_pill(row, tokens, lbl, active, true);
            record_tr_node(row, meta, &format!("edit.transform.error_propagation.{slug}"), "radio", lbl, Some(active.to_string()), true, &r);
            if r.clicked() {
                if from_edit {
                    apply_math_patch(draft_spec.as_mut(), None, None, Some(mode), intents);
                } else {
                    state.standalone_error_propagation = mode;
                }
            }
        }
        let legacy_ep = widgets::secondary_button(row, "Toggle Error Prop");
        record_tr_node(row, meta, "edit.transform.error_propagation", "button", "Error Propagation", None, true, &legacy_ep);
        if legacy_ep.clicked() {
            let next = if cur_ep == ErrorPropagationMode::ReplicateEndpoints {
                ErrorPropagationMode::FirstOrderDeltaMethod
            } else {
                ErrorPropagationMode::ReplicateEndpoints
            };
            if from_edit {
                apply_math_patch(draft_spec.as_mut(), None, None, Some(next), intents);
            } else {
                state.standalone_error_propagation = next;
            }
        }
    });

    state.validation_error = validate_transform_k(state.selected_k_op, &state.constant_k_draft, &app.locale_preset);

    let y_preview = match (state.selected_k_op, cur_y) {
        (_, Some(p)) => format!("{p:?}"),
        (Some(_), None) => String::from("Invalid Constant K"),
        (None, None) => String::from("Unchanged"),
    };
    let preview = format!(
        "Preview ({}): X -> {}, Y -> {} | Error: {:?}",
        ds.map_or("No sheet", |s| s.title.as_str()),
        cur_x.map_or_else(|| String::from("Unchanged"), |p| format!("{p:?}")),
        y_preview,
        cur_ep
    );
    ui.add_space(6.0);
    egui::Frame::new().fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).corner_radius(4.0).inner_margin(vec2(8.0, 5.0)).show(
        ui,
        |box_ui| {
            box_ui.label(RichText::new(preview).size(11.5).monospace().color(tokens.text_muted));
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn render_normalize_body(
    ui: &mut Ui,
    _app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TransformModalsEguiState,
    _from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    _intents: &mut PendingUiIntents,
) {
    ui.label(RichText::new(format!("Normalize Series ({})", ds.map_or("Sheet", |s| s.title.as_str()))).strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        row.label("0% Anchor:");
        for (idx, lbl) in [(0usize, "Smallest value"), (1, "First row"), (2, "Fixed 0.0")] {
            let r = choice_pill(row, tokens, lbl, state.norm_zero_idx == idx, true);
            record_tr_node(row, meta, &format!("edit.transform.norm_zero.{idx}"), "radio", lbl, None, true, &r);
            if r.clicked() {
                state.norm_zero_idx = idx;
            }
        }
    });
    ui.horizontal_wrapped(|row| {
        row.label("100% Anchor:");
        for (idx, lbl) in [(0usize, "Largest value"), (1, "Sum of column"), (2, "Last row")] {
            let r = choice_pill(row, tokens, lbl, state.norm_hundred_idx == idx, true);
            record_tr_node(row, meta, &format!("edit.transform.norm_hundred.{idx}"), "radio", lbl, None, true, &r);
            if r.clicked() {
                state.norm_hundred_idx = idx;
            }
        }
    });
    ui.horizontal(|row| {
        let _ = row.checkbox(&mut state.norm_percent, "Scale output to 0-100% (unchecked = 0-1 fraction)");
        let _ = row.checkbox(&mut state.norm_per_dataset, "Normalize each data set independently");
    });
}

#[allow(clippy::too_many_arguments)]
fn render_remove_baseline_body(
    ui: &mut Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let (cur_cols, cur_op) = match draft_spec.as_ref().map(|s| &s.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation })) => {
            (baseline_columns.clone(), *operation)
        }
        _ => (
            if state.baseline_cols.is_empty() { vec![0] } else { state.baseline_cols.clone() },
            match state.baseline_op {
                1 => BaselineOperation::Ratio,
                2 => BaselineOperation::PercentChange,
                3 => BaselineOperation::FractionalChange,
                _ => BaselineOperation::Difference,
            },
        ),
    };

    ui.label(RichText::new("Baseline Columns (row-wise mean when multiple selected):").strong().color(tokens.text_primary));
    let col_count = ds.map_or(2, |s| s.archetype.column_count().min(16));
    ui.horizontal_wrapped(|row| {
        for c in 0..col_count {
            let selected = cur_cols.contains(&c);
            let title = ds.map_or_else(|| format!("Col {}", c + 1), |s| s.title_at(c));
            let r = choice_pill(row, tokens, &title, selected, true);
            record_tr_node(row, meta, &format!("edit.transform.baseline_col.{c}"), "checkbox", &title, Some(selected.to_string()), true, &r);
            if r.clicked() {
                let mut next_cols = cur_cols.clone();
                if let Some(pos) = next_cols.iter().position(|x| *x == c) {
                    if next_cols.len() > 1 {
                        next_cols.remove(pos);
                    }
                } else {
                    next_cols.push(c);
                    next_cols.sort_unstable();
                }
                if from_edit {
                    if let Some(spec) = draft_spec.as_mut() {
                        spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: next_cols, operation: cur_op });
                        intents.push_set_edit_params_draft(spec.clone());
                    }
                } else {
                    state.baseline_cols = next_cols;
                }
            }
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Baseline Operation:").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        for (idx, op, slug, lbl) in [
            (0usize, BaselineOperation::Difference, "difference", "Difference (Y - B)"),
            (1, BaselineOperation::Ratio, "ratio", "Ratio (Y / B)"),
            (2, BaselineOperation::PercentChange, "percent", "Percent Change 100*(Y-B)/B"),
            (3, BaselineOperation::FractionalChange, "fraction", "Fractional (Y-B)/B"),
        ] {
            let active = cur_op == op;
            let r = choice_pill(row, tokens, lbl, active, true);
            record_tr_node(row, meta, &format!("edit.transform.baseline_operation.{slug}"), "radio", lbl, Some(active.to_string()), true, &r);
            if r.clicked() {
                if from_edit {
                    if let Some(spec) = draft_spec.as_mut() {
                        spec.kind =
                            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: cur_cols.clone(), operation: op });
                        intents.push_set_edit_params_draft(spec.clone());
                    }
                } else {
                    state.baseline_op = idx;
                }
            }
        }
        let legacy_op = widgets::secondary_button(row, "Cycle Operation");
        record_tr_node(row, meta, "edit.transform.baseline_operation", "button", "Baseline Operation", None, true, &legacy_op);
        if legacy_op.clicked()
            && from_edit
            && let Some(spec) = draft_spec.as_mut()
        {
            let next_op = match cur_op {
                BaselineOperation::Difference => BaselineOperation::Ratio,
                BaselineOperation::Ratio => BaselineOperation::PercentChange,
                BaselineOperation::PercentChange => BaselineOperation::FractionalChange,
                BaselineOperation::FractionalChange => BaselineOperation::Difference,
            };
            spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: cur_cols.clone(), operation: next_op });
            intents.push_set_edit_params_draft(spec.clone());
        }
    });

    state.validation_error = if cur_cols.is_empty() { Some(String::from("Select at least one baseline column.")) } else { None };
}

#[allow(clippy::too_many_arguments)]
fn render_mv_filter_transform_body(
    ui: &mut Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let (mut filter_formula, target_arch, derived) = match draft_spec.as_ref().map(|s| &s.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns })) => {
            (filter_formula.clone().unwrap_or_default(), *target_archetype, derived_columns.clone())
        }
        _ => (state.formula_draft.clone(), TableKind::MultipleVariables, Vec::new()),
    };

    ui.horizontal(|tabs| {
        for (idx, wid, lbl) in [
            (0usize, "edit.tab.transform", "1. Transform"),
            (1, "edit.tab.select_rows", "2. Select Rows"),
            (2, "edit.tab.select_cols", "3. Select Columns"),
        ] {
            let r = choice_pill(tabs, tokens, lbl, state.active_tab == idx, true);
            record_tr_node(tabs, meta, wid, "tab", lbl, Some((state.active_tab == idx).to_string()), true, &r);
            if r.clicked() {
                state.active_tab = idx;
            }
        }
    });
    widgets::divider(ui);

    match state.active_tab {
        0 => {
            ui.label(RichText::new("Derived Variable Formula:").strong().color(tokens.text_primary));
            ui.horizontal(|row| {
                row.label("Name:");
                let _ = row.add(egui::TextEdit::singleline(&mut state.derived_title_draft).hint_text("Derived1").desired_width(110.0));
                row.label("Expr:");
                let er = row.add(egui::TextEdit::singleline(&mut state.formula_draft).hint_text("= log10([Dose])").desired_width(210.0));
                record_tr_node(row, meta, "edit.transform.formula", "textbox", "Derived Formula", Some(state.formula_draft.clone()), true, &er);
                let can_add = !state.derived_title_draft.trim().is_empty() && !state.formula_draft.trim().is_empty();
                if row.add_enabled(can_add, egui::Button::new("Add Derived")).clicked() {
                    let t = state.derived_title_draft.trim().to_owned();
                    let f = state.formula_draft.trim().to_owned();
                    if from_edit && let Some(spec) = draft_spec.as_mut() {
                        let mut next_d = derived.clone();
                        next_d.push(DerivedColumnFormula::new(t, f));
                        spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                            filter_formula: (!filter_formula.trim().is_empty()).then_some(filter_formula.clone()),
                            target_archetype: target_arch,
                            derived_columns: next_d,
                        });
                        intents.push_set_edit_params_draft(spec.clone());
                    } else {
                        state.derived_cols.push((t, f));
                    }
                }
            });
            state.validation_error = if !state.formula_draft.trim().is_empty() {
                parse_formula(state.formula_draft.trim()).err().map(|e| format!("Formula error: {e}"))
            } else {
                None
            };
        }
        1 => {
            ui.label(RichText::new("Conjunctive Row Filter Formula:").strong().color(tokens.text_primary));
            let fw = (ui.available_width() - 16.0).clamp(180.0, 420.0);
            let fr = ui.add(egui::TextEdit::singleline(&mut filter_formula).hint_text("[Dose] > 0 && [Arm] == \"Drug\"").desired_width(fw));
            record_tr_node(ui, meta, "edit.transform.filter_formula", "textbox", "Filter Formula", Some(filter_formula.clone()), true, &fr);
            if fr.changed() {
                if from_edit && let Some(spec) = draft_spec.as_mut() {
                    spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula: (!filter_formula.trim().is_empty()).then_some(filter_formula.clone()),
                        target_archetype: target_arch,
                        derived_columns: derived.clone(),
                    });
                    intents.push_set_edit_params_draft(spec.clone());
                } else {
                    state.formula_draft = filter_formula.clone();
                }
            }
            ui.horizontal(|row| {
                if widgets::secondary_button(row, "+ Add Predicate Rule").clicked() {
                    state.filter_rows.push(MvFilterPredicateRow {
                        left_col: 0,
                        op: String::from(">"),
                        right_value_or_col: String::from("0"),
                        compare_variable: false,
                    });
                }
                if widgets::secondary_button(row, "Hook Constant").clicked() {
                    state.hook_picker_open = !state.hook_picker_open;
                }
            });
            let mut rebuild_synth = false;
            for pred in &mut state.filter_rows {
                ui.horizontal(|prow| {
                    let col_lbl = ds.map_or_else(|| format!("Col{}", pred.left_col + 1), |s| s.title_at(pred.left_col));
                    if choice_pill(prow, tokens, &col_lbl, false, true).clicked()
                        && let Some(s) = ds
                    {
                        pred.left_col = (pred.left_col + 1) % s.archetype.column_count().max(1);
                    }
                    for op in [">", ">=", "<", "<=", "=="] {
                        if choice_pill(prow, tokens, op, pred.op == op, true).clicked() {
                            pred.op = op.to_owned();
                        }
                    }
                    let _ = prow.checkbox(&mut pred.compare_variable, "vs Var");
                    let _ = prow.add(egui::TextEdit::singleline(&mut pred.right_value_or_col).hint_text("Value").desired_width(90.0));
                    if choice_pill(prow, tokens, "Build", false, true).clicked() {
                        rebuild_synth = true;
                    }
                });
            }
            if rebuild_synth {
                let synth = state
                    .filter_rows
                    .iter()
                    .map(|row_p| {
                        let lname = ds.map_or_else(|| format!("V{}", row_p.left_col + 1), |s| s.title_at(row_p.left_col));
                        let rpart =
                            if row_p.compare_variable { format!("[{}]", row_p.right_value_or_col.trim()) } else { row_p.right_value_or_col.clone() };
                        format!("[{lname}] {} {rpart}", row_p.op)
                    })
                    .collect::<Vec<_>>()
                    .join(" && ");
                state.formula_draft = synth.clone();
                if from_edit && let Some(spec) = draft_spec.as_mut() {
                    spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula: Some(synth),
                        target_archetype: target_arch,
                        derived_columns: derived.clone(),
                    });
                    intents.push_set_edit_params_draft(spec.clone());
                }
            }
            state.validation_error = if !filter_formula.trim().is_empty() {
                parse_formula(filter_formula.trim()).err().map(|e| format!("Filter syntax: {e}"))
            } else {
                None
            };
        }
        _ => {
            ui.label(RichText::new("Selected Data Columns:").strong().color(tokens.text_primary));
            if let Some(s) = ds {
                let mut selected =
                    draft_spec.as_ref().map(|d| d.selected_data_sets.clone()).unwrap_or_else(|| (0..s.archetype.column_count()).collect());
                ui.horizontal_wrapped(|row| {
                    for c in 0..s.archetype.column_count().min(20) {
                        let active = selected.is_empty() || selected.contains(&c);
                        let cname = s.title_at(c);
                        let r = choice_pill(row, tokens, &cname, active, true);
                        record_tr_node(row, meta, &format!("edit.transform.col.{c}"), "checkbox", &cname, Some(active.to_string()), true, &r);
                        if r.clicked() && from_edit {
                            if let Some(pos) = selected.iter().position(|x| *x == c) {
                                selected.remove(pos);
                            } else {
                                selected.push(c);
                            }
                            if let Some(spec) = draft_spec.as_mut() {
                                spec.selected_data_sets = selected.clone();
                                intents.push_set_edit_params_draft(spec.clone());
                            }
                        }
                    }
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_extract_and_rearrange_body(
    ui: &mut Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    kind: TransformModalKind,
    state: &mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let target_kind = state.extract_target_kind.unwrap_or_else(|| match draft_spec.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, .. })) => *target_archetype,
        _ => TableKind::Xy,
    });

    ui.horizontal(|tabs| {
        for (idx, wid, lbl) in [(0usize, "edit.tab.target_format", "1. Target Table Format"), (1, "edit.tab.data_arrangement", "2. Data Arrangement")]
        {
            let r = choice_pill(tabs, tokens, lbl, state.active_tab == idx, true);
            record_tr_node(tabs, meta, wid, "tab", lbl, Some((state.active_tab == idx).to_string()), true, &r);
            if r.clicked() {
                state.active_tab = idx;
            }
        }
    });
    widgets::divider(ui);

    if state.active_tab == 0 {
        ui.label(RichText::new("Target Table Archetype:").strong().color(tokens.text_primary));
        ui.horizontal_wrapped(|row| {
            for (tk, slug, lbl) in [
                (TableKind::Xy, "xy", "Coordinate (XY)"),
                (TableKind::Column, "column", "Column"),
                (TableKind::Grouped, "grouped", "Grouped"),
                (TableKind::Contingency, "contingency", "Contingency"),
                (TableKind::Survival, "survival", "Survival"),
                (TableKind::PartsOfWhole, "parts_of_whole", "Parts of Whole"),
                (TableKind::MultipleVariables, "multiple_variables", "Multiple Variables"),
                (TableKind::Nested, "nested", "Nested"),
            ] {
                let active = target_kind == tk;
                let r = choice_pill(row, tokens, lbl, active, true);
                record_tr_node(row, meta, &format!("edit.transform.target_archetype.{slug}"), "radio", lbl, Some(active.to_string()), true, &r);
                if r.clicked() {
                    state.extract_target_kind = Some(tk);
                    if from_edit
                        && let Some(spec) = draft_spec.as_mut()
                        && let AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, derived_columns, .. }) = &spec.kind
                    {
                        spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                            filter_formula: filter_formula.clone(),
                            target_archetype: tk,
                            derived_columns: derived_columns.clone(),
                        });
                        intents.push_set_edit_params_draft(spec.clone());
                    }
                }
            }
        });
        let legacy_ta = widgets::secondary_button(ui, "Cycle Target Archetype");
        record_tr_node(ui, meta, "edit.transform.target_archetype", "button", "Target Archetype", None, true, &legacy_ta);
        if legacy_ta.clicked() && from_edit {
            let next_tk = if target_kind == TableKind::Xy { TableKind::Column } else { TableKind::Xy };
            state.extract_target_kind = Some(next_tk);
        }
    } else {
        let mode_desc = match kind {
            TransformModalKind::Transpose => "Transpose rows <-> columns (swap Row & Column titles)",
            TransformModalKind::PruneRows => "Prune every K rows (optional block averaging)",
            TransformModalKind::Simulate => "Simulate deterministic curve + replicate noise",
            TransformModalKind::RandomSample => "Deterministic random / stratified row sub-sampling",
            _ => "Map source variables into target table roles",
        };
        ui.label(RichText::new(mode_desc).color(tokens.text_muted));
        ui.add_space(4.0);
        let sheet_cols: Vec<String> = ds.map(|s| (0..s.archetype.column_count()).map(|c| s.title_at(c)).collect()).unwrap_or_default();
        if !sheet_cols.is_empty() {
            ui.horizontal(|row| {
                row.label("Predictor / X:");
                let _ = row.add(egui::TextEdit::singleline(&mut state.extract_binding_x).hint_text(sheet_cols[0].as_str()).desired_width(100.0));
                row.label("Response / Y:");
                let _ = row.add(
                    egui::TextEdit::singleline(&mut state.extract_binding_y)
                        .hint_text(sheet_cols.get(1).map_or("Y", String::as_str))
                        .desired_width(100.0),
                );
                if matches!(target_kind, TableKind::Grouped | TableKind::Nested | TableKind::Survival) {
                    row.label("Group / Factor:");
                    let _ = row.add(egui::TextEdit::singleline(&mut state.extract_binding_grp).hint_text("Group").desired_width(90.0));
                }
            });
        }
    }
}

fn render_inline_hook_picker(ui: &mut Ui, app: &UiApp, state: &mut TransformModalsEguiState, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    egui::Frame::new().fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.accent)).corner_radius(4.0).inner_margin(vec2(8.0, 6.0)).show(
        ui,
        |box_ui| {
            box_ui.label(RichText::new("Hook Constant from InfoSheet:").strong().color(tokens.text_primary));
            let mut found = 0usize;
            for info in &app.document().info_sheets {
                for c in &info.constants {
                    if let Some(num) = c.value.as_finite_f64() {
                        let lbl = format!("{} -> {num}", c.name);
                        let r = choice_pill(box_ui, tokens, &lbl, false, true);
                        record_tr_node(box_ui, meta, &format!("edit.transform.hook_item.{found}"), "menuitem", &lbl, None, true, &r);
                        found += 1;
                        if r.clicked() {
                            state.constant_k_draft = format!("{num}");
                            state.hook_picker_open = false;
                            state.validation_error = validate_transform_k(state.selected_k_op, &state.constant_k_draft, &app.locale_preset);
                        }
                    }
                }
            }
            if found == 0 {
                box_ui.label(RichText::new("No numeric Info constants found in active document.").size(11.5).color(tokens.text_muted));
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn render_transform_footer(
    ui: &mut Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    kind: TransformModalKind,
    state: &mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.horizontal(|footer| {
        let cancel_lbl = tr_tr(tokens.locale, "btn.cancel", "Cancel");
        let cancel_resp = widgets::secondary_button(footer, cancel_lbl.as_ref());
        record_tr_node(footer, meta, "modal.cancel", "button", cancel_lbl.as_ref(), None, true, &cancel_resp);
        if cancel_resp.clicked() {
            if from_edit {
                intents.push_dialog_button("cancel");
            } else {
                intents.push_close_transform_modal();
            }
        }

        footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
            let ok_lbl = tr_tr(tokens.locale, "btn.ok", "OK");
            let can_ok = state.validation_error.is_none();
            let ok_resp = widgets::primary_button_enabled(rhs, ok_lbl.as_ref(), can_ok);
            record_tr_node(rhs, meta, "modal.ok", "button", ok_lbl.as_ref(), None, can_ok, &ok_resp);
            if ok_resp.clicked() {
                if from_edit {
                    if commit_edit_transform_k_if_needed(app, kind, state, intents) {
                        intents.push_dialog_button("ok");
                    }
                } else {
                    if let Some(sheet) = ds {
                        match build_standalone_transform_params(app, sheet, kind, state) {
                            Ok(cmd_params) => {
                                intents.push_engine("transform.run", cmd_params);
                                intents.push_close_transform_modal();
                            }
                            Err(err) => {
                                state.validation_error = Some(err);
                            }
                        }
                    } else {
                        intents.push_close_transform_modal();
                    }
                }
            }

            if from_edit {
                let apply_lbl = tr_tr(tokens.locale, "btn.apply", "Apply");
                let apply_resp = rhs.add_enabled(can_ok, egui::Button::new(apply_lbl.as_ref()));
                record_tr_node(rhs, meta, "modal.apply", "button", apply_lbl.as_ref(), None, can_ok, &apply_resp);
                if apply_resp.clicked() && commit_edit_transform_k_if_needed(app, kind, state, intents) {
                    intents.push_dialog_button("apply");
                }
            }
        });
    });
}

fn commit_edit_transform_k_if_needed(
    app: &UiApp,
    kind: TransformModalKind,
    state: &mut TransformModalsEguiState,
    intents: &mut PendingUiIntents,
) -> bool {
    if kind != TransformModalKind::TransformValues {
        return true;
    }
    let Some(k_op) = state.selected_k_op else {
        return true;
    };
    match build_linear_scale_for_k(k_op, &state.constant_k_draft, &app.locale_preset) {
        Ok(linear) => {
            let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
            apply_math_patch(draft_spec.as_mut(), None, Some(Some(linear)), None, intents);
            true
        }
        Err(err) => {
            state.validation_error = Some(err);
            false
        }
    }
}

fn apply_math_patch(
    draft: Option<&mut AnalysisSpec>,
    new_x: Option<Option<MathTransformPreset>>,
    new_y: Option<Option<MathTransformPreset>>,
    new_ep: Option<ErrorPropagationMode>,
    intents: &mut PendingUiIntents,
) {
    let Some(spec) = draft else { return };
    if let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }) = &mut spec.kind {
        if let Some(x) = new_x {
            *x_transform = x;
        }
        if let Some(y) = new_y {
            *y_transform = y;
        }
        if let Some(ep) = new_ep {
            *error_propagation = ep;
        }
        intents.push_set_edit_params_draft(spec.clone());
    }
}

fn build_standalone_transform_params(
    app: &UiApp,
    ds: &DataSheet,
    kind: TransformModalKind,
    state: &TransformModalsEguiState,
) -> Result<Value, String> {
    match kind {
        TransformModalKind::TransformValues => {
            let x_transform = state.standalone_x_preset.as_deref().and_then(parse_math_slug);
            let y_transform = match state.standalone_y_preset.as_deref() {
                None | Some("none") => None,
                Some(slug) => {
                    if let Some(k_op) = ConstantKOp::from_slug(slug) {
                        Some(build_linear_scale_for_k(k_op, &state.constant_k_draft, &app.locale_preset)?)
                    } else {
                        parse_math_slug(slug)
                    }
                }
            };
            Ok(json!({
                "source_sheet": ds.id.as_str(),
                "kind": AnalysisKind::Transform(TransformAnalysisSpec::Math {
                    x_transform,
                    y_transform,
                    error_propagation: state.standalone_error_propagation,
                }),
            }))
        }
        TransformModalKind::Normalize => Ok(json!({
            "source_sheet": ds.id.as_str(),
            "kind": "normalize",
            "zero_anchor": match state.norm_zero_idx { 1 => json!("first_row"), 2 => json!({ "FixedValue": 0.0 }), _ => json!("smallest") },
            "hundred_anchor": match state.norm_hundred_idx { 1 => "column_sum", 2 => "last_row", _ => "largest" },
            "as_fraction": !state.norm_percent,
            "per_data_set": state.norm_per_dataset,
        })),
        TransformModalKind::RemoveBaseline => Ok(json!({
            "source_sheet": ds.id.as_str(),
            "kind": "remove_baseline",
            "baseline_columns": if state.baseline_cols.is_empty() { vec![0] } else { state.baseline_cols.clone() },
            "operation": match state.baseline_op {
                1 => "ratio",
                2 => "percent_change",
                3 => "fractional_change",
                _ => "difference",
            },
        })),
        TransformModalKind::MvFilterAndTransform | TransformModalKind::ExtractAndRearrange => Ok(json!({
            "source_sheet": ds.id.as_str(),
            "kind": "select_and_transform",
            "filter_formula": (!state.formula_draft.trim().is_empty()).then_some(state.formula_draft.trim()),
        })),
        TransformModalKind::Transpose => Ok(json!({ "source_sheet": ds.id.as_str(), "kind": "transpose", "swap_row_and_column_titles": true })),
        TransformModalKind::PruneRows => {
            Ok(json!({ "source_sheet": ds.id.as_str(), "kind": "prune_rows", "every_k_rows": 2, "average_window": false }))
        }
        TransformModalKind::Simulate => {
            Ok(json!({ "source_sheet": ds.id.as_str(), "kind": "simulate", "seed": 42, "row_count": 13, "replicates": 3 }))
        }
        TransformModalKind::RandomSample => Ok(json!({ "source_sheet": ds.id.as_str(), "kind": "random_sample", "seed": 42, "sample_size": 5 })),
    }
}

fn parse_math_slug(slug: &str) -> Option<MathTransformPreset> {
    match slug {
        "log10" => Some(MathTransformPreset::Log10),
        "ln" => Some(MathTransformPreset::Ln),
        "log2" => Some(MathTransformPreset::Log2),
        "exp10" => Some(MathTransformPreset::Exp10),
        "exp" => Some(MathTransformPreset::Exp),
        "reciprocal" => Some(MathTransformPreset::Reciprocal),
        "sqrt" => Some(MathTransformPreset::Sqrt),
        "square" => Some(MathTransformPreset::Square),
        "logit" => Some(MathTransformPreset::Logit),
        "probit" => Some(MathTransformPreset::Probit),
        "neg_log_m_um" => Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 }),
        "neg_log_m_nm" => Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-9 }),
        _ => None,
    }
}

fn math_preset_to_slug(preset: MathTransformPreset) -> String {
    String::from(match preset {
        MathTransformPreset::Log10 => "log10",
        MathTransformPreset::Ln => "ln",
        MathTransformPreset::Log2 => "log2",
        MathTransformPreset::Exp10 => "exp10",
        MathTransformPreset::Exp => "exp",
        MathTransformPreset::Reciprocal => "reciprocal",
        MathTransformPreset::Sqrt => "sqrt",
        MathTransformPreset::Square => "square",
        MathTransformPreset::Logit => "logit",
        MathTransformPreset::Probit => "probit",
        MathTransformPreset::ArcsinSqrt => "arcsin_sqrt",
        MathTransformPreset::LinearScale { .. } => "affine_k",
        MathTransformPreset::ConcentrationToNegativeLogM { .. } => "neg_log_m_um",
    })
}

trait TitleAtExt {
    fn title_at(&self, col: usize) -> String;
}

impl TitleAtExt for DataSheet {
    fn title_at(&self, col: usize) -> String {
        match &self.archetype {
            TableArchetype::MultipleVariables(mv) => mv.variables.get(col).map_or_else(|| format!("Var {}", col + 1), |v| v.title.clone()),
            TableArchetype::Column(t) => t.data_sets.get(col).map_or_else(|| format!("Col {}", col + 1), |d| d.title.clone()),
            TableArchetype::Xy(t) => t.data_sets.get(col).map_or_else(|| format!("Y{}", col + 1), |d| d.title.clone()),
            TableArchetype::Grouped(t) => t.data_sets.get(col).map_or_else(|| format!("Grp {}", col + 1), |d| d.title.clone()),
            TableArchetype::Contingency(t) => t.data_sets.get(col).map_or_else(|| format!("Col {}", col + 1), |d| d.title.clone()),
            TableArchetype::Survival(t) => t.cohorts.get(col).map_or_else(|| format!("Cohort {}", col + 1), |d| d.title.clone()),
            TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or_else(|| String::from("Values"), |d| d.title.clone()),
            TableArchetype::Nested(t) => t.groups.get(col).map_or_else(|| format!("Group {}", col + 1), |d| d.title.clone()),
        }
    }
}

/// Render `Exclude with Reason...` modal when `ActiveDialog::ExcludeWithReason` is active.
pub fn render_exclude_with_reason_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut TransformModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TransformModalFrameReport {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
        state.synced_exclude_tag.clear();
        return TransformModalFrameReport::default();
    }

    if state.synced_exclude_tag.is_empty() {
        state.synced_exclude_tag = app.canvas_dialogs.exclude_with_reason.reason_tag.clone();
        state.exclude_reason_draft = app.canvas_dialogs.exclude_with_reason.reason_tag.clone();
        state.exclude_note_draft = app.canvas_dialogs.exclude_with_reason.note.clone();
    }

    let title = tr_tr(tokens.locale, "w6.view.modal.exclude_with_reason", "Exclude with Reason");
    let vp = ctx.content_rect();
    let dlg_w = (vp.width() - 24.0).clamp(280.0, 460.0);

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_exclude_reason_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (dlg_w - 24.0).max(240.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.heading(RichText::new(title.as_ref()).color(tokens.text_primary));
        ui.add_space(6.0);
        ui.label(format!("Target cells selected: {}", app.canvas_dialogs.exclude_with_reason.coords.len()));
        ui.add_space(4.0);
        ui.label("Preset reason tags:");
        ui.horizontal_wrapped(|row| {
            for (idx, tag) in ExcludeWithReasonDialogState::preset_reason_tags().iter().enumerate() {
                let active = app.canvas_dialogs.exclude_with_reason.reason_tag == *tag;
                let r = choice_pill(row, tokens, tag, active, true);
                record_tr_node(row, meta, &format!("exclude.preset.{idx}"), "radio", tag, Some(active.to_string()), true, &r);
                if r.clicked() {
                    state.exclude_reason_draft = (*tag).to_owned();
                    let mut f = Map::new();
                    f.insert(String::from("reason_tag"), Value::String((*tag).to_owned()));
                    intents.push_patch_active_dialog(f);
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|row| {
            row.label("Reason Tag:");
            let tr_box = row.add(egui::TextEdit::singleline(&mut state.exclude_reason_draft).hint_text("Reason tag").desired_width(180.0));
            record_tr_node(row, meta, "exclude.reason_tag", "textbox", "Reason Tag", Some(state.exclude_reason_draft.clone()), true, &tr_box);
            if tr_box.changed() {
                let mut f = Map::new();
                f.insert(String::from("reason_tag"), Value::String(state.exclude_reason_draft.clone()));
                intents.push_patch_active_dialog(f);
            }
        });
        ui.label("Audit Note:");
        let nr = ui.add(
            egui::TextEdit::multiline(&mut state.exclude_note_draft)
                .hint_text("Optional audit note…")
                .desired_rows(2)
                .desired_width((dlg_w - 44.0).max(160.0)),
        );
        record_tr_node(ui, meta, "exclude.note", "textbox", "Audit Note", Some(state.exclude_note_draft.clone()), true, &nr);
        if nr.changed() {
            let mut f = Map::new();
            f.insert(String::from("note"), Value::String(state.exclude_note_draft.clone()));
            intents.push_patch_active_dialog(f);
        }

        ui.add_space(8.0);
        ui.horizontal(|footer| {
            let cancel_lbl = tr_tr(tokens.locale, "btn.cancel", "Cancel");
            let cancel_resp = widgets::secondary_button(footer, cancel_lbl.as_ref());
            record_tr_node(footer, meta, "modal.cancel", "button", cancel_lbl.as_ref(), None, true, &cancel_resp);
            if cancel_resp.clicked() {
                intents.push_dialog_button("cancel");
            }
            footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
                let ok_lbl = tr_tr(tokens.locale, "btn.ok", "OK");
                let can_ok = !state.exclude_reason_draft.trim().is_empty() && !app.canvas_dialogs.exclude_with_reason.coords.is_empty();
                let ok_resp = widgets::primary_button_enabled(rhs, ok_lbl.as_ref(), can_ok);
                record_tr_node(rhs, meta, "modal.ok", "button", ok_lbl.as_ref(), None, can_ok, &ok_resp);
                if ok_resp.clicked() {
                    intents.push_dialog_button("ok");
                }
            });
        });
    });

    let title_for_ak = title.as_ref().to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.exclude_with_reason", title: title.as_ref(), z_dialog: ZLayer::ModalDialog },
        meta,
    );
    TransformModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn choice_pill(ui: &mut Ui, tokens: &Tokens, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let mut text = RichText::new(label);
    if selected {
        text = text.color(tokens.accent).strong();
    }
    ui.add_enabled(enabled, egui::Button::new(text).selected(selected))
}

#[allow(clippy::too_many_arguments)]
fn record_tr_node(
    ui: &Ui,
    meta: &mut FrameInspectMeta,
    wid: &str,
    role: &str,
    label: &str,
    value: Option<String>,
    enabled: bool,
    resp: &egui::Response,
) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
        return;
    }
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id: wid,
            role,
            label,
            command_id: None,
            value: value.map(Value::String),
            enabled,
            disabled_reason: (!enabled).then_some("unavailable"),
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}
