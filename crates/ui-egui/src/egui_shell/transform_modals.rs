//! Reactive data transformation, normalization, baseline removal, MV filter/transform,
//! extract & rearrange, and exclusion-with-reason modals in `egui 0.37` (`R15.4`,
//! `gui-layout-spec.md` §8.6).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, DataSheet, DerivedColumnFormula, ErrorPropagationMode, MAX_ROWS, MAX_SUBCOLUMNS,
    MathTransformPreset, NormalizeAnchor, SheetId, SimulationNoise, TableArchetype, TableKind, TransformAnalysisSpec,
};
use curvebook_transform::formula::parser::parse_formula;
use egui::{Align, Context, Key, Layout, Modifiers, Rect, RichText, Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ExcludeWithReasonDialogState};
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, widgets};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformModalsEguiState {
    pub open_kind: Option<TransformModalKind>,
    pub target_sheet_id: Option<SheetId>,
    pub active_tab: usize,
    pub formula_draft: String,
    pub filter_formula_draft: String,
    pub constant_k_draft: String,
    pub create_new_graph: bool,
    pub selected_k_op: Option<ConstantKOp>,
    pub synced_edit_sheet_id: Option<String>,
    pub synced_standalone_key: Option<(TransformModalKind, Option<SheetId>)>,
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
    pub transpose_swap_titles: bool,
    pub prune_every_k_rows: usize,
    pub prune_every_k_draft: String,
    pub prune_average_window: bool,
    pub standalone_selected_cols: Vec<usize>,
    pub sim_seed_draft: String,
    pub sim_row_count_draft: String,
    pub sim_replicates_draft: String,
    pub sim_x_min_draft: String,
    pub sim_x_max_draft: String,
    pub sim_equation_id_draft: String,
    pub sim_params_draft: String,
    pub sim_noise_kind: usize,
    pub sim_noise_param_draft: String,
    pub rand_sample_size_draft: String,
    pub rand_seed_draft: String,
    pub rand_stratify_col: Option<usize>,
    pub rand_proportional_allocation: bool,
}

impl Default for TransformModalsEguiState {
    fn default() -> Self {
        Self {
            open_kind: None,
            target_sheet_id: None,
            active_tab: 0,
            formula_draft: String::new(),
            filter_formula_draft: String::new(),
            constant_k_draft: String::new(),
            create_new_graph: false,
            selected_k_op: None,
            synced_edit_sheet_id: None,
            synced_standalone_key: None,
            filter_rows: Vec::new(),
            exclude_reason_draft: String::new(),
            validation_error: None,
            standalone_x_preset: None,
            standalone_y_preset: None,
            standalone_error_propagation: ErrorPropagationMode::default(),
            pharmacology_mode: false,
            baseline_cols: Vec::new(),
            baseline_op: 0,
            derived_title_draft: String::new(),
            derived_cols: Vec::new(),
            extract_target_kind: None,
            extract_binding_x: String::new(),
            extract_binding_y: String::new(),
            extract_binding_grp: String::new(),
            norm_zero_idx: 0,
            norm_hundred_idx: 0,
            norm_percent: true,
            norm_per_dataset: true,
            hook_picker_open: false,
            exclude_note_draft: String::new(),
            synced_exclude_tag: String::new(),
            transpose_swap_titles: true,
            prune_every_k_rows: 2,
            prune_every_k_draft: String::from("2"),
            prune_average_window: false,
            standalone_selected_cols: Vec::new(),
            sim_seed_draft: String::from("42"),
            sim_row_count_draft: String::from("13"),
            sim_replicates_draft: String::from("3"),
            sim_x_min_draft: String::from("-9"),
            sim_x_max_draft: String::from("-3"),
            sim_equation_id_draft: String::from("hill_4pl"),
            sim_params_draft: String::new(),
            sim_noise_kind: 0,
            sim_noise_param_draft: String::from("5"),
            rand_sample_size_draft: String::from("5"),
            rand_seed_draft: String::from("42"),
            rand_stratify_col: None,
            rand_proportional_allocation: true,
        }
    }
}

/// Frame geometry and modal capture report emitted by transform modals.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

struct TransformModalCtx<'a> {
    app: &'a UiApp,
    ds: Option<&'a DataSheet>,
    state: &'a mut TransformModalsEguiState,
    from_edit: bool,
    tokens: &'a Tokens,
    meta: &'a mut FrameInspectMeta,
    intents: &'a mut PendingUiIntents,
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
    } else {
        state.synced_standalone_key = None;
    }
    if !from_edit && state.open_kind.is_none() {
        state.synced_standalone_key = None;
        state.hook_picker_open = false;
        state.validation_error = None;
        return TransformModalFrameReport::default();
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if state.hook_picker_open {
            state.hook_picker_open = false;
        } else if from_edit {
            intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
        } else {
            intents.push(UiIntent::CloseTransformModal);
        }
    }
    if from_edit {
        sync_transform_edit_state(app, state);
    } else if let Some(kind) = state.open_kind {
        sync_standalone_transform_state(kind, state);
    }

    let draft_kind = if from_edit { app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) } else { None };
    let effective_kind = resolve_effective_modal_kind(draft_kind, state.open_kind);
    let title = modal_title_for(effective_kind, tokens.locale);

    let vp = ctx.content_rect();
    let dlg_w = (vp.width() - 24.0).clamp(320.0, 620.0);
    let dlg_h = (vp.height() - 24.0).clamp(240.0, 500.0);

    let wants_kb_before = ctx.egui_wants_keyboard_input();
    let source_sheet = resolve_source_sheet(app, state, from_edit);
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_transform_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (dlg_w - 24.0).max(280.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(dlg_h);

        ui.heading(RichText::new(title).color(tokens.text_primary));
        ui.add_space(4.0);
        widgets::divider(ui);

        let mut modal_cx = TransformModalCtx { app, ds: source_sheet, state, from_edit, tokens, meta, intents };
        let body_h = (dlg_h - 92.0).max(110.0);
        egui::ScrollArea::vertical().id_salt("transform_modal_scroll").max_width(inner_w).max_height(body_h).auto_shrink([false, false]).show(
            ui,
            |body| {
                body.set_max_width((inner_w - 18.0).max(200.0));
                match effective_kind {
                    TransformModalKind::TransformValues => render_transform_values_body(body, &mut modal_cx),
                    TransformModalKind::Normalize => render_normalize_body(body, &mut modal_cx),
                    TransformModalKind::RemoveBaseline => render_remove_baseline_body(body, &mut modal_cx),
                    TransformModalKind::MvFilterAndTransform => render_mv_filter_transform_body(body, &mut modal_cx),
                    TransformModalKind::Transpose => render_transpose_body(body, &mut modal_cx),
                    TransformModalKind::PruneRows => render_prune_rows_body(body, &mut modal_cx),
                    TransformModalKind::ExtractAndRearrange => render_extract_and_rearrange_body(body, &mut modal_cx),
                    TransformModalKind::Simulate => render_simulate_body(body, &mut modal_cx),
                    TransformModalKind::RandomSample => render_random_sample_body(body, &mut modal_cx),
                }

                if modal_cx.state.hook_picker_open {
                    body.add_space(6.0);
                    render_inline_hook_picker(body, modal_cx.app, modal_cx.state, modal_cx.tokens, modal_cx.meta);
                }

                if let Some(err) = &modal_cx.state.validation_error {
                    body.add_space(4.0);
                    body.label(RichText::new(err).color(modal_cx.tokens.link_red).size(11.5));
                }
            },
        );

        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        render_transform_footer(ui, &mut modal_cx, effective_kind);
    });

    if !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && !state.hook_picker_open
        && state.validation_error.is_none()
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
    {
        if from_edit {
            if commit_edit_transform_draft_if_needed(app, source_sheet, effective_kind, state, intents) {
                intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                push_normalize_graph_if_requested(app, effective_kind, state, intents);
            }
        } else if let Some(sheet) = source_sheet {
            match build_standalone_transform_params(app, sheet, effective_kind, state) {
                Ok(cmd_params) => {
                    intents.push_engine("transform.run", cmd_params);
                    intents.push(UiIntent::CloseTransformModal);
                }
                Err(err) => {
                    state.validation_error = Some(err);
                }
            }
        } else {
            intents.push(UiIntent::CloseTransformModal);
        }
    }

    let title_for_ak = title.to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.transform", title, z_dialog: ZLayer::ModalDialog },
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

fn modal_title_for(kind: TransformModalKind, locale: &str) -> &'static str {
    let norm = normalize_locale(locale);
    match kind {
        TransformModalKind::TransformValues => tr(norm, "transform.title.values"),
        TransformModalKind::Normalize => tr(norm, "transform.title.normalize"),
        TransformModalKind::RemoveBaseline => tr(norm, "analysis.remove_baseline"),
        TransformModalKind::MvFilterAndTransform => tr(norm, "transform.title.mv_filter"),
        TransformModalKind::ExtractAndRearrange => tr(norm, "transform.title.extract"),
        TransformModalKind::Transpose => tr(norm, "transform.title.transpose"),
        TransformModalKind::PruneRows => tr(norm, "transform.title.prune"),
        TransformModalKind::Simulate => tr(norm, "transform.title.simulate"),
        TransformModalKind::RandomSample => tr(norm, "transform.title.random_sample"),
    }
}

fn resolve_source_sheet<'a>(app: &'a UiApp, state: &TransformModalsEguiState, from_edit: bool) -> Option<&'a DataSheet> {
    let doc = app.document();
    if from_edit && let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return doc.data_sheet(&spec.source_sheet_id).or_else(|| doc.result_sheet(&spec.source_sheet_id).and_then(|rs| rs.analyzable_data.as_ref()));
    }
    state
        .target_sheet_id
        .as_ref()
        .and_then(|id| doc.data_sheet(id))
        .or_else(|| doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)))
        .or_else(|| doc.data_sheets.first())
}

fn sync_standalone_transform_state(kind: TransformModalKind, state: &mut TransformModalsEguiState) {
    let key = (kind, state.target_sheet_id.clone());
    if state.synced_standalone_key.as_ref() == Some(&key) {
        return;
    }
    state.synced_standalone_key = Some(key);
    state.active_tab = 0;
    state.formula_draft.clear();
    state.filter_formula_draft.clear();
    state.filter_rows.clear();
    state.constant_k_draft.clear();
    state.selected_k_op = None;
    state.hook_picker_open = false;
    state.standalone_x_preset = None;
    state.standalone_y_preset = None;
    state.standalone_error_propagation = ErrorPropagationMode::default();
    state.pharmacology_mode = false;
    state.baseline_cols = vec![0];
    state.baseline_op = 0;
    state.derived_title_draft.clear();
    state.derived_cols.clear();
    state.extract_target_kind = None;
    state.extract_binding_x.clear();
    state.extract_binding_y.clear();
    state.extract_binding_grp.clear();
    state.transpose_swap_titles = true;
    state.prune_every_k_rows = 2;
    state.prune_every_k_draft = String::from("2");
    state.prune_average_window = false;
    state.norm_zero_idx = 0;
    state.norm_hundred_idx = 0;
    state.norm_percent = true;
    state.norm_per_dataset = true;
    state.create_new_graph = false;
    state.standalone_selected_cols.clear();
    state.sim_seed_draft = String::from("42");
    state.sim_row_count_draft = String::from("13");
    state.sim_replicates_draft = String::from("3");
    state.sim_x_min_draft = String::from("-9");
    state.sim_x_max_draft = String::from("-3");
    state.sim_equation_id_draft = String::from("hill_4pl");
    state.sim_params_draft.clear();
    state.sim_noise_kind = 0;
    state.sim_noise_param_draft = String::from("5");
    state.rand_sample_size_draft = String::from("5");
    state.rand_seed_draft = String::from("42");
    state.rand_stratify_col = None;
    state.rand_proportional_allocation = true;
    state.validation_error = None;
}

fn sync_transform_edit_state(app: &UiApp, state: &mut TransformModalsEguiState) {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return;
    };
    if state.synced_edit_sheet_id.as_deref() == Some(result_sheet_id.as_str()) {
        return;
    }
    state.synced_edit_sheet_id = Some(result_sheet_id.as_str().to_owned());
    state.active_tab = 0;
    state.validation_error = None;
    let spec = app.canvas_dialogs.edit_params.draft.as_ref().or_else(|| app.document().result_sheet(result_sheet_id).map(|rs| &rs.analysis));
    state.standalone_selected_cols = spec.map(|s| s.selected_data_sets.clone()).unwrap_or_default();
    state.selected_k_op = None;
    state.constant_k_draft.clear();
    state.hook_picker_open = false;
    state.pharmacology_mode = false;
    state.baseline_cols = vec![0];
    state.baseline_op = 0;
    state.derived_title_draft.clear();
    state.formula_draft.clear();
    state.filter_rows.clear();
    state.transpose_swap_titles = true;
    state.prune_every_k_rows = 2;
    state.prune_every_k_draft = String::from("2");
    state.prune_average_window = false;
    state.norm_zero_idx = 0;
    state.norm_hundred_idx = 0;
    state.norm_percent = true;
    state.norm_per_dataset = true;
    state.create_new_graph = false;
    state.filter_formula_draft.clear();
    state.derived_cols.clear();
    state.extract_target_kind = None;
    state.extract_binding_x.clear();
    state.extract_binding_y.clear();
    state.extract_binding_grp.clear();
    state.sim_seed_draft = String::from("42");
    state.sim_row_count_draft = String::from("13");
    state.sim_replicates_draft = String::from("3");
    state.sim_x_min_draft = String::from("-9");
    state.sim_x_max_draft = String::from("-3");
    state.sim_equation_id_draft = String::from("hill_4pl");
    state.sim_params_draft.clear();
    state.sim_noise_kind = 0;
    state.sim_noise_param_draft = String::from("5");
    state.rand_sample_size_draft = String::from("5");
    state.rand_seed_draft = String::from("42");
    state.rand_stratify_col = None;
    state.rand_proportional_allocation = true;
    match spec.map(|s| &s.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. })) => {
            if let Some((op, k)) = y_transform.and_then(ConstantKOp::infer_from_preset) {
                state.selected_k_op = Some(op);
                state.constant_k_draft = format!("{k}");
            }
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation })) => {
            state.baseline_cols = if baseline_columns.is_empty() { vec![0] } else { baseline_columns.clone() };
            state.baseline_op = match operation {
                BaselineOperation::Ratio => 1,
                BaselineOperation::PercentChange => 2,
                BaselineOperation::FractionalChange => 3,
                BaselineOperation::Difference => 0,
            };
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set })) => {
            state.norm_zero_idx = match zero_anchor {
                NormalizeAnchor::FirstRow => 1,
                NormalizeAnchor::FixedValue(_) => 2,
                _ => 0,
            };
            state.norm_hundred_idx = match hundred_anchor {
                NormalizeAnchor::ColumnSum => 1,
                NormalizeAnchor::LastRow => 2,
                _ => 0,
            };
            state.norm_percent = *output_percent;
            state.norm_per_dataset = *per_data_set;
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles })) => {
            state.transpose_swap_titles = *swap_row_and_column_titles;
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows, average_window })) => {
            state.prune_every_k_rows = (*every_k_rows).max(1);
            state.prune_every_k_draft = state.prune_every_k_rows.to_string();
            state.prune_average_window = *average_window;
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns })) => {
            state.filter_formula_draft = filter_formula.clone().unwrap_or_default();
            state.extract_target_kind = Some(*target_archetype);
            state.derived_cols = derived_columns.iter().map(|d| (d.title.clone(), d.formula.clone())).collect();
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed,
            row_count,
            replicates,
            x_min,
            x_max,
            equation_id,
            parameters,
            noise,
        })) => {
            state.sim_seed_draft = seed.to_string();
            state.sim_row_count_draft = row_count.to_string();
            state.sim_replicates_draft = replicates.to_string();
            state.sim_x_min_draft = format_draft_f64(*x_min, &app.locale_preset);
            state.sim_x_max_draft = format_draft_f64(*x_max, &app.locale_preset);
            state.sim_equation_id_draft = equation_id.clone();
            state.sim_params_draft =
                parameters.iter().map(|(k, v)| format!("{k}={}", format_draft_f64(*v, &app.locale_preset))).collect::<Vec<_>>().join(", ");
            let (nkind, nparam) = match noise {
                SimulationNoise::None => (0, String::from("5")),
                SimulationNoise::Gaussian { sd } => (1, format_draft_f64(*sd, &app.locale_preset)),
                SimulationNoise::RelativePercent { cv_percent } => (2, format_draft_f64(*cv_percent, &app.locale_preset)),
                SimulationNoise::Poisson => (3, String::from("5")),
            };
            state.sim_noise_kind = nkind;
            state.sim_noise_param_draft = nparam;
        }
        Some(AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation })) => {
            state.rand_seed_draft = seed.to_string();
            state.rand_sample_size_draft = sample_size.to_string();
            state.rand_stratify_col = *stratify_column;
            state.rand_proportional_allocation = *proportional_allocation;
        }
        _ => {}
    }
}

fn format_draft_f64(value: f64, preset: &crate::i18n::LocalePreset) -> String {
    let norm = if value == 0.0 { 0.0 } else { value };
    let text = format!("{norm}");
    if preset.decimal_separator != '.' { text.replace('.', &preset.decimal_separator.to_string()) } else { text }
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

fn render_transform_values_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
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
        meta.record_modal_response(
            row,
            &std_r,
            FrameInspectEntryInput::modal("edit.transform.mode.standard", "tab", "Standard Functions", None, true),
        );
        if std_r.clicked() {
            state.pharmacology_mode = false;
        }
        let pharm_r = choice_pill(row, tokens, "Pharmacology (-log10[M])", state.pharmacology_mode, true);
        meta.record_modal_response(
            row,
            &pharm_r,
            FrameInspectEntryInput::modal("edit.transform.mode.pharmacology", "tab", "Pharmacology", None, true),
        );
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
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.x_transform.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
            if r.clicked() {
                if from_edit {
                    apply_math_patch(draft_spec.as_mut(), Some(preset_opt), None, None, intents);
                } else {
                    state.standalone_x_preset = (slug != "none").then(|| slug.to_owned());
                }
            }
        }
        let legacy_x = widgets::secondary_button(row, "Cycle X Transform");
        meta.record_modal_response(row, &legacy_x, FrameInspectEntryInput::modal("edit.transform.x_transform", "button", "Transform X", None, true));
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
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.y_transform.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
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
        meta.record_modal_response(row, &legacy_y, FrameInspectEntryInput::modal("edit.transform.y_transform", "button", "Transform Y", None, true));
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
        meta.record_modal_response(
            row,
            &kr,
            FrameInspectEntryInput::modal("edit.transform.constant_k", "textbox", "Constant K", Some(state.constant_k_draft.clone()), true),
        );
        if kr.changed()
            && from_edit
            && let Some(k_op) = state.selected_k_op
            && let Ok(linear) = build_linear_scale_for_k(k_op, &state.constant_k_draft, &app.locale_preset)
        {
            apply_math_patch(draft_spec.as_mut(), None, Some(Some(linear)), None, intents);
        }
        let hr = widgets::secondary_button(row, "Hook Constant");
        meta.record_modal_response(row, &hr, FrameInspectEntryInput::modal("edit.transform.hook_k", "button", "Hook Constant", None, true));
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
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.error_propagation.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
            if r.clicked() {
                if from_edit {
                    apply_math_patch(draft_spec.as_mut(), None, None, Some(mode), intents);
                } else {
                    state.standalone_error_propagation = mode;
                }
            }
        }
        let legacy_ep = widgets::secondary_button(row, "Toggle Error Prop");
        meta.record_modal_response(
            row,
            &legacy_ep,
            FrameInspectEntryInput::modal("edit.transform.error_propagation", "button", "Error Propagation", None, true),
        );
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

fn normalize_zero_anchor_from_idx(idx: usize) -> NormalizeAnchor {
    match idx {
        1 => NormalizeAnchor::FirstRow,
        2 => NormalizeAnchor::FixedValue(0.0),
        _ => NormalizeAnchor::ColumnMin,
    }
}

fn normalize_hundred_anchor_from_idx(idx: usize) -> NormalizeAnchor {
    match idx {
        1 => NormalizeAnchor::ColumnSum,
        2 => NormalizeAnchor::LastRow,
        _ => NormalizeAnchor::ColumnMax,
    }
}

fn push_normalize_edit_draft(
    app: &UiApp,
    state: &TransformModalsEguiState,
    zero_override: Option<NormalizeAnchor>,
    hundred_override: Option<NormalizeAnchor>,
    intents: &mut PendingUiIntents,
) {
    let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() else {
        return;
    };
    if let AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set }) = &mut spec.kind {
        if let Some(z) = zero_override {
            *zero_anchor = z;
        } else if matches!(zero_anchor, NormalizeAnchor::ColumnMin | NormalizeAnchor::FirstRow | NormalizeAnchor::FixedValue(0.0)) {
            *zero_anchor = normalize_zero_anchor_from_idx(state.norm_zero_idx);
        }
        if let Some(h) = hundred_override {
            *hundred_anchor = h;
        } else if matches!(hundred_anchor, NormalizeAnchor::ColumnMax | NormalizeAnchor::ColumnSum | NormalizeAnchor::LastRow) {
            *hundred_anchor = normalize_hundred_anchor_from_idx(state.norm_hundred_idx);
        }
        *output_percent = state.norm_percent;
        *per_data_set = state.norm_per_dataset;
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
    }
}

fn render_normalize_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    ui.label(RichText::new(format!("Normalize Series ({})", ds.map_or("Sheet", |s| s.title.as_str()))).strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        row.label("0% Anchor:");
        for (idx, lbl) in [(0usize, "Smallest value"), (1, "First row"), (2, "Fixed 0.0")] {
            let r = choice_pill(row, tokens, lbl, state.norm_zero_idx == idx, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(&format!("edit.transform.norm_zero.{idx}"), "radio", lbl, None, true));
            if r.clicked() {
                state.norm_zero_idx = idx;
                if from_edit {
                    push_normalize_edit_draft(app, state, Some(normalize_zero_anchor_from_idx(idx)), None, intents);
                }
            }
        }
    });
    ui.horizontal_wrapped(|row| {
        row.label("100% Anchor:");
        for (idx, lbl) in [(0usize, "Largest value"), (1, "Sum of column"), (2, "Last row")] {
            let r = choice_pill(row, tokens, lbl, state.norm_hundred_idx == idx, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.norm_hundred.{idx}"), "radio", lbl, None, true),
            );
            if r.clicked() {
                state.norm_hundred_idx = idx;
                if from_edit {
                    push_normalize_edit_draft(app, state, None, Some(normalize_hundred_anchor_from_idx(idx)), intents);
                }
            }
        }
    });
    ui.horizontal_wrapped(|row| {
        let pct_resp = row.checkbox(&mut state.norm_percent, "Scale output to 0-100% (unchecked = 0-1 fraction)");
        meta.record_modal_response(
            row,
            &pct_resp,
            FrameInspectEntryInput::modal(
                "edit.transform.norm_percent",
                "checkbox",
                "Scale Output to 0-100%",
                Some(state.norm_percent.to_string()),
                true,
            ),
        );
        if pct_resp.changed() && from_edit {
            push_normalize_edit_draft(app, state, None, None, intents);
        }

        let ds_resp = row.checkbox(&mut state.norm_per_dataset, "Normalize each data set independently");
        meta.record_modal_response(
            row,
            &ds_resp,
            FrameInspectEntryInput::modal(
                "edit.transform.norm_per_dataset",
                "checkbox",
                "Normalize Each Data Set",
                Some(state.norm_per_dataset.to_string()),
                true,
            ),
        );
        if ds_resp.changed() && from_edit {
            push_normalize_edit_draft(app, state, None, None, intents);
        }

        let cr = row.checkbox(&mut state.create_new_graph, "Create new graph of normalized results");
        meta.record_modal_response(
            row,
            &cr,
            FrameInspectEntryInput::modal(
                "edit.transform.normalize.create_graph",
                "checkbox",
                "Create New Graph",
                Some(state.create_new_graph.to_string()),
                true,
            ),
        );
    });
}

fn render_remove_baseline_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
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
    let col_count = ds.map_or(2, |s| s.archetype.column_count());
    ui.horizontal_wrapped(|row| {
        for c in 0..col_count {
            let selected = cur_cols.contains(&c);
            let title = ds.map_or_else(|| format!("Col {}", c + 1), |s| s.title_at(c));
            let r = choice_pill(row, tokens, &title, selected, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.baseline_col.{c}"), "checkbox", &title, Some(selected.to_string()), true),
            );
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
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
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
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.baseline_operation.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
            if r.clicked() {
                if from_edit {
                    if let Some(spec) = draft_spec.as_mut() {
                        spec.kind =
                            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: cur_cols.clone(), operation: op });
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
                    }
                } else {
                    state.baseline_op = idx;
                }
            }
        }
        let legacy_op = widgets::secondary_button(row, "Cycle Operation");
        meta.record_modal_response(
            row,
            &legacy_op,
            FrameInspectEntryInput::modal("edit.transform.baseline_operation", "button", "Baseline Operation", None, true),
        );
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
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
        }
    });

    state.validation_error = if cur_cols.is_empty() { Some(String::from("Select at least one baseline column.")) } else { None };
}

fn render_mv_filter_transform_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let (mut filter_formula, target_arch, derived) = match draft_spec.as_ref().map(|s| &s.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns })) => {
            (filter_formula.clone().unwrap_or_default(), *target_archetype, derived_columns.clone())
        }
        _ => (
            state.filter_formula_draft.clone(),
            state.extract_target_kind.unwrap_or(TableKind::MultipleVariables),
            state.derived_cols.iter().map(|(t, f)| DerivedColumnFormula::new(t.clone(), f.clone())).collect(),
        ),
    };

    ui.horizontal_wrapped(|tabs| {
        for (idx, wid, lbl) in [
            (0usize, "edit.tab.transform", "1. Transform"),
            (1, "edit.tab.select_rows", "2. Select Rows"),
            (2, "edit.tab.select_cols", "3. Select Columns"),
            (3, "edit.tab.target_format", "4. Target Format"),
        ] {
            let r = choice_pill(tabs, tokens, lbl, state.active_tab == idx, true);
            meta.record_modal_response(tabs, &r, FrameInspectEntryInput::modal(wid, "tab", lbl, Some((state.active_tab == idx).to_string()), true));
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
                let tr = row.add(egui::TextEdit::singleline(&mut state.derived_title_draft).hint_text("Derived1").desired_width(110.0));
                meta.record_modal_response(
                    row,
                    &tr,
                    FrameInspectEntryInput::modal(
                        "edit.transform.derived_title",
                        "textbox",
                        "Derived Name",
                        Some(state.derived_title_draft.clone()),
                        true,
                    ),
                );
                row.label("Expr:");
                let er = row.add(egui::TextEdit::singleline(&mut state.formula_draft).hint_text("= log10([Dose])").desired_width(210.0));
                meta.record_modal_response(
                    row,
                    &er,
                    FrameInspectEntryInput::modal("edit.transform.formula", "textbox", "Derived Formula", Some(state.formula_draft.clone()), true),
                );
                let can_add = !state.derived_title_draft.trim().is_empty() && !state.formula_draft.trim().is_empty();
                let add_resp = row.add_enabled(can_add, egui::Button::new("Add Derived"));
                meta.record_modal_response(
                    row,
                    &add_resp,
                    FrameInspectEntryInput::modal("edit.transform.add_derived", "button", "Add Derived", None, can_add),
                );
                if add_resp.clicked() {
                    let t = state.derived_title_draft.trim().to_owned();
                    let f = state.formula_draft.trim().to_owned();
                    match parse_formula(&f) {
                        Ok(_) => {
                            if from_edit && let Some(spec) = draft_spec.as_mut() {
                                let mut next_d = derived.clone();
                                next_d.push(DerivedColumnFormula::new(t, f));
                                spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                                    filter_formula: (!filter_formula.trim().is_empty()).then_some(filter_formula.clone()),
                                    target_archetype: target_arch,
                                    derived_columns: next_d,
                                });
                                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
                            } else {
                                state.derived_cols.push((t, f));
                            }
                            state.derived_title_draft.clear();
                            state.formula_draft.clear();
                            state.validation_error = None;
                        }
                        Err(err) => {
                            state.validation_error = Some(format!("Formula error: {err}"));
                        }
                    }
                }
            });
            if !derived.is_empty() {
                ui.add_space(4.0);
                for (idx, dcol) in derived.iter().enumerate() {
                    ui.horizontal(|drow| {
                        drow.label(RichText::new(format!("{} = {}", dcol.title, dcol.formula)).size(11.5).color(tokens.text_muted));
                        let rem = widgets::secondary_button(drow, "Remove");
                        meta.record_modal_response(
                            drow,
                            &rem,
                            FrameInspectEntryInput::modal(
                                &format!("edit.transform.derived.remove.{idx}"),
                                "button",
                                "Remove",
                                Some(dcol.title.clone()),
                                true,
                            ),
                        );
                        if rem.clicked() {
                            if from_edit && let Some(spec) = draft_spec.as_mut() {
                                let mut next_d = derived.clone();
                                if idx < next_d.len() {
                                    next_d.remove(idx);
                                }
                                spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                                    filter_formula: (!filter_formula.trim().is_empty()).then_some(filter_formula.clone()),
                                    target_archetype: target_arch,
                                    derived_columns: next_d,
                                });
                                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
                            } else if idx < state.derived_cols.len() {
                                state.derived_cols.remove(idx);
                            }
                        }
                    });
                }
            }
        }
        1 => {
            ui.label(RichText::new("Conjunctive Row Filter Formula:").strong().color(tokens.text_primary));
            let fw = (ui.available_width() - 16.0).clamp(180.0, 420.0);
            let fr = ui.add(egui::TextEdit::singleline(&mut filter_formula).hint_text("[Dose] > 0 && [Arm] == \"Drug\"").desired_width(fw));
            meta.record_modal_response(
                ui,
                &fr,
                FrameInspectEntryInput::modal("edit.transform.filter_formula", "textbox", "Filter Formula", Some(filter_formula.clone()), true),
            );
            if fr.changed() {
                state.filter_formula_draft = filter_formula.clone();
                if from_edit && let Some(spec) = draft_spec.as_mut() {
                    spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula: (!filter_formula.trim().is_empty()).then_some(filter_formula.clone()),
                        target_archetype: target_arch,
                        derived_columns: derived.clone(),
                    });
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
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
                state.filter_formula_draft = synth.clone();
                filter_formula = synth.clone();
                if from_edit && let Some(spec) = draft_spec.as_mut() {
                    spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula: Some(synth),
                        target_archetype: target_arch,
                        derived_columns: derived.clone(),
                    });
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
                }
            }
        }
        2 => {
            ui.label(RichText::new("Selected Data Columns:").strong().color(tokens.text_primary));
            if let Some(s) = ds {
                let all_cols: Vec<usize> = (0..s.archetype.column_count()).collect();
                let mut selected = if from_edit {
                    let raw = draft_spec.as_ref().map(|d| d.selected_data_sets.clone()).unwrap_or_default();
                    if raw.is_empty() { all_cols } else { raw }
                } else if state.standalone_selected_cols.is_empty() {
                    all_cols
                } else {
                    state.standalone_selected_cols.clone()
                };
                ui.horizontal_wrapped(|row| {
                    for c in 0..s.archetype.column_count() {
                        let active = selected.contains(&c);
                        let cname = s.title_at(c);
                        let r = choice_pill(row, tokens, &cname, active, true);
                        meta.record_modal_response(
                            row,
                            &r,
                            FrameInspectEntryInput::modal(&format!("edit.transform.col.{c}"), "checkbox", &cname, Some(active.to_string()), true),
                        );
                        if r.clicked() {
                            if let Some(pos) = selected.iter().position(|x| *x == c) {
                                if selected.len() > 1 {
                                    selected.remove(pos);
                                }
                            } else {
                                selected.push(c);
                                selected.sort_unstable();
                            }
                            state.standalone_selected_cols = selected.clone();
                            if from_edit && let Some(spec) = draft_spec.as_mut() {
                                spec.selected_data_sets = selected.clone();
                                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
                            }
                        }
                    }
                });
            }
        }
        _ => {
            render_target_archetype_selector(
                ui,
                TargetArchetypePickerCtx { tokens, meta, state, from_edit, draft_spec: &mut draft_spec, intents },
                target_arch,
            );
        }
    }
    state.validation_error = validate_mv_formulas(&state.formula_draft, &filter_formula);
}

fn validate_mv_formulas(derived_expr: &str, filter_formula: &str) -> Option<String> {
    if !derived_expr.trim().is_empty()
        && let Err(err) = parse_formula(derived_expr.trim())
    {
        return Some(format!("Formula error: {err}"));
    }
    if !filter_formula.trim().is_empty()
        && let Err(err) = parse_formula(filter_formula.trim())
    {
        return Some(format!("Filter syntax: {err}"));
    }
    None
}

struct TargetArchetypePickerCtx<'a> {
    tokens: &'a Tokens,
    meta: &'a mut FrameInspectMeta,
    state: &'a mut TransformModalsEguiState,
    from_edit: bool,
    draft_spec: &'a mut Option<AnalysisSpec>,
    intents: &'a mut PendingUiIntents,
}

fn render_target_archetype_selector(ui: &mut Ui, pcx: TargetArchetypePickerCtx<'_>, target_kind: TableKind) {
    let TargetArchetypePickerCtx { tokens, meta, state, from_edit, draft_spec, intents } = pcx;
    let apply_target =
        |tk: TableKind, state: &mut TransformModalsEguiState, draft_spec: &mut Option<AnalysisSpec>, intents: &mut PendingUiIntents| {
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
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
            }
        };
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
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.target_archetype.{slug}"), "radio", lbl, Some(active.to_string()), true),
            );
            if r.clicked() {
                apply_target(tk, state, draft_spec, intents);
            }
        }
    });
    let legacy_ta = widgets::secondary_button(ui, "Cycle Target Archetype");
    meta.record_modal_response(
        ui,
        &legacy_ta,
        FrameInspectEntryInput::modal("edit.transform.target_archetype", "button", "Target Archetype", None, true),
    );
    if legacy_ta.clicked() {
        let next_tk = if target_kind == TableKind::Xy { TableKind::Column } else { TableKind::Xy };
        apply_target(next_tk, state, draft_spec, intents);
    }
}

fn render_transpose_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let mut swap_titles = match draft_spec.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles })) => *swap_row_and_column_titles,
        _ => state.transpose_swap_titles,
    };

    ui.label(RichText::new("Transpose Orientation:").strong().color(tokens.text_primary));
    ui.label(RichText::new("Flip rows and columns into a transposed output table.").size(11.5).color(tokens.text_muted));
    ui.add_space(4.0);
    let chk = ui.checkbox(&mut swap_titles, "Swap row and column titles");
    meta.record_modal_response(
        ui,
        &chk,
        FrameInspectEntryInput::modal(
            "edit.transform.transpose.swap_titles",
            "checkbox",
            "Swap row and column titles",
            Some(swap_titles.to_string()),
            true,
        ),
    );
    if chk.changed() {
        state.transpose_swap_titles = swap_titles;
        if from_edit && let Some(spec) = draft_spec.as_mut() {
            spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: swap_titles });
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
        }
    }
}

fn render_prune_rows_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut draft_spec = app.canvas_dialogs.edit_params.draft.clone();
    let (every_k, mut avg_win) = match draft_spec.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows, average_window })) => ((*every_k_rows).max(1), *average_window),
        _ => (state.prune_every_k_rows.max(1), state.prune_average_window),
    };

    let push_prune = |k: usize, avg: bool, draft_spec: &mut Option<AnalysisSpec>, intents: &mut PendingUiIntents| {
        if from_edit && let Some(spec) = draft_spec.as_mut() {
            spec.kind = AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: k, average_window: avg });
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
        }
    };

    ui.label(RichText::new("Row Pruning Options:").strong().color(tokens.text_primary));
    ui.horizontal(|row| {
        row.label("Keep every K rows:");
        let can_dec = every_k > 1;
        let minus = row.add_enabled(can_dec, egui::Button::new("-"));
        meta.record_modal_response(row, &minus, FrameInspectEntryInput::modal("edit.transform.prune.every_k.minus", "button", "-", None, can_dec));
        if minus.clicked() {
            let next_k = every_k.saturating_sub(1).max(1);
            state.prune_every_k_rows = next_k;
            state.prune_every_k_draft = next_k.to_string();
            state.validation_error = None;
            push_prune(next_k, avg_win, &mut draft_spec, intents);
        }

        let k_edit = row.add(egui::TextEdit::singleline(&mut state.prune_every_k_draft).hint_text("2").desired_width(56.0));
        meta.record_modal_response(
            row,
            &k_edit,
            FrameInspectEntryInput::modal(
                "edit.transform.prune.every_k_rows",
                "textbox",
                "Keep every K rows",
                Some(state.prune_every_k_draft.clone()),
                true,
            ),
        );
        if k_edit.changed() {
            let trimmed = state.prune_every_k_draft.trim();
            match trimmed.parse::<usize>() {
                Ok(k) if (1..=1000).contains(&k) => {
                    state.prune_every_k_rows = k;
                    state.validation_error = None;
                    push_prune(k, avg_win, &mut draft_spec, intents);
                }
                _ => {
                    state.validation_error = Some(String::from("Keep every K rows must be an integer between 1 and 1000."));
                }
            }
        }

        let can_inc = every_k < 1000;
        let plus = row.add_enabled(can_inc, egui::Button::new("+"));
        meta.record_modal_response(row, &plus, FrameInspectEntryInput::modal("edit.transform.prune.every_k.plus", "button", "+", None, can_inc));
        if plus.clicked() {
            let next_k = (every_k + 1).min(1000);
            state.prune_every_k_rows = next_k;
            state.prune_every_k_draft = next_k.to_string();
            state.validation_error = None;
            push_prune(next_k, avg_win, &mut draft_spec, intents);
        }
    });

    ui.add_space(4.0);
    let avg_chk = ui.checkbox(&mut avg_win, "Average each K-row block");
    meta.record_modal_response(
        ui,
        &avg_chk,
        FrameInspectEntryInput::modal("edit.transform.prune.average_window", "checkbox", "Average each K-row block", Some(avg_win.to_string()), true),
    );
    if avg_chk.changed() {
        state.prune_average_window = avg_win;
        push_prune(state.prune_every_k_rows.max(1), avg_win, &mut draft_spec, intents);
    }
}

fn render_extract_and_rearrange_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    let mut draft_spec = cx.app.canvas_dialogs.edit_params.draft.clone();
    let target_kind = state.extract_target_kind.unwrap_or_else(|| match draft_spec.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, .. })) => *target_archetype,
        _ => TableKind::Xy,
    });

    ui.horizontal(|tabs| {
        for (idx, wid, lbl) in [(0usize, "edit.tab.target_format", "1. Target Table Format"), (1, "edit.tab.data_arrangement", "2. Data Arrangement")]
        {
            let r = choice_pill(tabs, tokens, lbl, state.active_tab == idx, true);
            meta.record_modal_response(tabs, &r, FrameInspectEntryInput::modal(wid, "tab", lbl, Some((state.active_tab == idx).to_string()), true));
            if r.clicked() {
                state.active_tab = idx;
            }
        }
    });
    widgets::divider(ui);

    if state.active_tab == 0 {
        render_target_archetype_selector(
            ui,
            TargetArchetypePickerCtx { tokens, meta, state, from_edit, draft_spec: &mut draft_spec, intents },
            target_kind,
        );
    } else {
        ui.label(RichText::new("Map source columns into target table roles (comma-separated titles supported)").color(tokens.text_muted));
        ui.add_space(4.0);
        let sheet_cols: Vec<String> = ds.map(|s| (0..s.archetype.column_count()).map(|c| s.title_at(c)).collect()).unwrap_or_default();
        if !sheet_cols.is_empty() {
            ui.label(RichText::new(format!("Available columns: {}", sheet_cols.join(", "))).size(11.0).color(tokens.text_muted));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|row| {
                row.label("Predictor / X:");
                let rx = row.add(egui::TextEdit::singleline(&mut state.extract_binding_x).hint_text(sheet_cols[0].as_str()).desired_width(100.0));
                meta.record_modal_response(
                    row,
                    &rx,
                    FrameInspectEntryInput::modal(
                        "edit.transform.extract.binding_x",
                        "textbox",
                        "Predictor / X",
                        Some(state.extract_binding_x.clone()),
                        true,
                    ),
                );
                row.label("Response / Y:");
                let ry = row.add(
                    egui::TextEdit::singleline(&mut state.extract_binding_y)
                        .hint_text(sheet_cols.get(1).map_or("Y", String::as_str))
                        .desired_width(100.0),
                );
                meta.record_modal_response(
                    row,
                    &ry,
                    FrameInspectEntryInput::modal(
                        "edit.transform.extract.binding_y",
                        "textbox",
                        "Response / Y",
                        Some(state.extract_binding_y.clone()),
                        true,
                    ),
                );
                if matches!(target_kind, TableKind::Grouped | TableKind::Nested | TableKind::Survival) {
                    row.label("Group / Factor:");
                    let rgrp = row.add(egui::TextEdit::singleline(&mut state.extract_binding_grp).hint_text("Group").desired_width(90.0));
                    meta.record_modal_response(
                        row,
                        &rgrp,
                        FrameInspectEntryInput::modal(
                            "edit.transform.extract.binding_grp",
                            "textbox",
                            "Group / Factor",
                            Some(state.extract_binding_grp.clone()),
                            true,
                        ),
                    );
                }
            });
        }
    }

    state.validation_error = ds.and_then(|sheet| resolve_extract_column_bindings(sheet, state, target_kind).err());
}

fn render_simulate_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let mut changed = false;

    ui.label(RichText::new("Equation Preset & Curve Expression").strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        for (eq_id, lbl) in [
            ("hill_4pl", "4PL Sigmoidal"),
            ("hill_3pl", "3PL Sigmoidal"),
            ("linear", "Linear"),
            ("quadratic", "Quadratic"),
            ("one_phase_decay", "1-Phase Decay"),
            ("michaelis_menten", "Michaelis-Menten"),
            ("gaussian", "Gaussian"),
            ("sine", "Sine"),
        ] {
            let sel = state.sim_equation_id_draft.trim().eq_ignore_ascii_case(eq_id);
            let r = choice_pill(row, tokens, lbl, sel, true);
            meta.record_modal_response(
                row,
                &r,
                FrameInspectEntryInput::modal(&format!("edit.transform.simulate.equation.{eq_id}"), "radio", lbl, Some(sel.to_string()), true),
            );
            if r.clicked() {
                state.sim_equation_id_draft = eq_id.to_owned();
                changed = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("Equation ID / formula:");
        let eq_r = row.add(egui::TextEdit::singleline(&mut state.sim_equation_id_draft).hint_text("hill_4pl").desired_width(180.0));
        meta.record_modal_response(
            row,
            &eq_r,
            FrameInspectEntryInput::modal(
                "edit.transform.simulate.equation_id",
                "textbox",
                "Equation ID",
                Some(state.sim_equation_id_draft.clone()),
                true,
            ),
        );
        if eq_r.changed() {
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("X min:");
        let xmin_r = row.add(egui::TextEdit::singleline(&mut state.sim_x_min_draft).desired_width(64.0));
        meta.record_modal_response(
            row,
            &xmin_r,
            FrameInspectEntryInput::modal("edit.transform.simulate.x_min", "textbox", "X Min", Some(state.sim_x_min_draft.clone()), true),
        );
        row.label("X max:");
        let xmax_r = row.add(egui::TextEdit::singleline(&mut state.sim_x_max_draft).desired_width(64.0));
        meta.record_modal_response(
            row,
            &xmax_r,
            FrameInspectEntryInput::modal("edit.transform.simulate.x_max", "textbox", "X Max", Some(state.sim_x_max_draft.clone()), true),
        );
        row.label("Rows:");
        let rows_r = row.add(egui::TextEdit::singleline(&mut state.sim_row_count_draft).desired_width(56.0));
        meta.record_modal_response(
            row,
            &rows_r,
            FrameInspectEntryInput::modal("edit.transform.simulate.row_count", "textbox", "Row Count", Some(state.sim_row_count_draft.clone()), true),
        );
        row.label("Replicates:");
        let reps_r = row.add(egui::TextEdit::singleline(&mut state.sim_replicates_draft).desired_width(48.0));
        meta.record_modal_response(
            row,
            &reps_r,
            FrameInspectEntryInput::modal(
                "edit.transform.simulate.replicates",
                "textbox",
                "Replicates",
                Some(state.sim_replicates_draft.clone()),
                true,
            ),
        );
        row.label("Seed:");
        let seed_r = row.add(egui::TextEdit::singleline(&mut state.sim_seed_draft).desired_width(64.0));
        meta.record_modal_response(
            row,
            &seed_r,
            FrameInspectEntryInput::modal("edit.transform.simulate.seed", "textbox", "RNG Seed", Some(state.sim_seed_draft.clone()), true),
        );
        if xmin_r.changed() || xmax_r.changed() || rows_r.changed() || reps_r.changed() || seed_r.changed() {
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("Parameters (name=val, ...):");
        let pr = row.add(
            egui::TextEdit::singleline(&mut state.sim_params_draft).hint_text("Bottom=0, Top=100, LogEC50=-6, HillSlope=1").desired_width(280.0),
        );
        meta.record_modal_response(
            row,
            &pr,
            FrameInspectEntryInput::modal("edit.transform.simulate.params", "textbox", "Parameters", Some(state.sim_params_draft.clone()), true),
        );
        if pr.changed() {
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("Noise:");
        for (idx, slug, lbl) in
            [(0usize, "none", "None"), (1, "gaussian", "Gaussian (SD)"), (2, "relative_cv", "Relative (%CV)"), (3, "poisson", "Poisson")]
        {
            let sel = state.sim_noise_kind == idx;
            let nr = choice_pill(row, tokens, lbl, sel, true);
            meta.record_modal_response(
                row,
                &nr,
                FrameInspectEntryInput::modal(&format!("edit.transform.simulate.noise.{slug}"), "radio", lbl, Some(sel.to_string()), true),
            );
            if nr.clicked() {
                state.sim_noise_kind = idx;
                changed = true;
            }
        }
        if matches!(state.sim_noise_kind, 1 | 2) {
            row.label(if state.sim_noise_kind == 1 { "SD:" } else { "CV (%):" });
            let npr = row.add(egui::TextEdit::singleline(&mut state.sim_noise_param_draft).desired_width(64.0));
            meta.record_modal_response(
                row,
                &npr,
                FrameInspectEntryInput::modal(
                    "edit.transform.simulate.noise_param",
                    "textbox",
                    "Noise Level",
                    Some(state.sim_noise_param_draft.clone()),
                    true,
                ),
            );
            if npr.changed() {
                changed = true;
            }
        }
    });

    match validate_and_build_simulate_spec(state, &app.locale_preset) {
        Ok(tr_spec) => {
            state.validation_error = None;
            if from_edit
                && changed
                && let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone()
            {
                spec.kind = AnalysisKind::Transform(tr_spec);
                cx.intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
            }
        }
        Err(err) => {
            state.validation_error = Some(err);
        }
    }
}

fn render_random_sample_body(ui: &mut Ui, cx: &mut TransformModalCtx<'_>) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let mut changed = false;

    ui.label(RichText::new("Deterministic Random / Stratified Row Sampling").strong().color(tokens.text_primary));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label("Sample size (rows):");
        let sr = row.add(egui::TextEdit::singleline(&mut state.rand_sample_size_draft).hint_text("5").desired_width(72.0));
        meta.record_modal_response(
            row,
            &sr,
            FrameInspectEntryInput::modal(
                "edit.transform.random_sample.sample_size",
                "textbox",
                "Sample Size",
                Some(state.rand_sample_size_draft.clone()),
                true,
            ),
        );
        row.label("RNG seed:");
        let seed_r = row.add(egui::TextEdit::singleline(&mut state.rand_seed_draft).hint_text("42").desired_width(80.0));
        meta.record_modal_response(
            row,
            &seed_r,
            FrameInspectEntryInput::modal("edit.transform.random_sample.seed", "textbox", "RNG Seed", Some(state.rand_seed_draft.clone()), true),
        );
        if sr.changed() || seed_r.changed() {
            changed = true;
        }
    });

    ui.add_space(6.0);
    ui.label("Stratification column:");
    ui.horizontal_wrapped(|row| {
        let none_sel = state.rand_stratify_col.is_none();
        let nr = choice_pill(row, tokens, "None (Simple Random)", none_sel, true);
        meta.record_modal_response(
            row,
            &nr,
            FrameInspectEntryInput::modal(
                "edit.transform.random_sample.stratify.none",
                "radio",
                "None (Simple Random)",
                Some(none_sel.to_string()),
                true,
            ),
        );
        if nr.clicked() {
            state.rand_stratify_col = None;
            changed = true;
        }
        if let Some(sheet) = ds {
            for col in 0..sheet.archetype.column_count() {
                let lbl = sheet.title_at(col);
                let sel = state.rand_stratify_col == Some(col);
                let cr = choice_pill(row, tokens, &lbl, sel, true);
                meta.record_modal_response(
                    row,
                    &cr,
                    FrameInspectEntryInput::modal(
                        &format!("edit.transform.random_sample.stratify.{col}"),
                        "radio",
                        &lbl,
                        Some(sel.to_string()),
                        true,
                    ),
                );
                if cr.clicked() {
                    state.rand_stratify_col = Some(col);
                    changed = true;
                }
            }
        }
    });

    ui.add_space(4.0);
    let has_strat = state.rand_stratify_col.is_some();
    let pr = ui.add_enabled(has_strat, egui::Checkbox::new(&mut state.rand_proportional_allocation, "Proportional allocation across strata"));
    meta.record_modal_response(
        ui,
        &pr,
        FrameInspectEntryInput::modal(
            "edit.transform.random_sample.proportional",
            "checkbox",
            "Proportional Allocation",
            Some(state.rand_proportional_allocation.to_string()),
            has_strat,
        ),
    );
    if pr.changed() {
        changed = true;
    }

    match validate_and_build_random_sample_spec(ds, state) {
        Ok(tr_spec) => {
            state.validation_error = None;
            if from_edit
                && changed
                && let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone()
            {
                spec.kind = AnalysisKind::Transform(tr_spec);
                cx.intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
            }
        }
        Err(err) => {
            state.validation_error = Some(err);
        }
    }
}

fn validate_and_build_simulate_spec(state: &TransformModalsEguiState, preset: &crate::i18n::LocalePreset) -> Result<TransformAnalysisSpec, String> {
    let eq = state.sim_equation_id_draft.trim();
    if eq.is_empty() {
        return Err(String::from("Enter a simulation equation ID or expression."));
    }
    let seed = state.sim_seed_draft.trim().parse::<u64>().map_err(|_| String::from("Simulation seed must be a non-negative integer."))?;
    let row_count = state
        .sim_row_count_draft
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|&n| (1..=MAX_ROWS).contains(&n))
        .ok_or_else(|| format!("Row count must be between 1 and {MAX_ROWS}."))?;
    let replicates = state
        .sim_replicates_draft
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|&n| (1..=MAX_SUBCOLUMNS).contains(&n))
        .ok_or_else(|| format!("Replicates must be between 1 and {MAX_SUBCOLUMNS}."))?;
    let x_min = parse_localized_number_input(state.sim_x_min_draft.trim(), preset)
        .filter(|v| v.is_finite())
        .ok_or_else(|| String::from("X min must be a finite number."))?;
    let x_max = parse_localized_number_input(state.sim_x_max_draft.trim(), preset)
        .filter(|v| v.is_finite())
        .ok_or_else(|| String::from("X max must be a finite number."))?;
    if row_count > 1 && x_min >= x_max {
        return Err(String::from("X min must be strictly less than X max when rows > 1."));
    }
    if x_min > x_max {
        return Err(String::from("X min must not exceed X max."));
    }
    let parameters = parse_simulate_parameters(&state.sim_params_draft, preset)?;
    let noise = match state.sim_noise_kind {
        1 => {
            let sd = parse_localized_number_input(state.sim_noise_param_draft.trim(), preset)
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| String::from("Gaussian SD must be finite and >= 0."))?;
            SimulationNoise::Gaussian { sd }
        }
        2 => {
            let cv_percent = parse_localized_number_input(state.sim_noise_param_draft.trim(), preset)
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| String::from("Relative %CV must be finite and >= 0."))?;
            SimulationNoise::RelativePercent { cv_percent }
        }
        3 => SimulationNoise::Poisson,
        _ => SimulationNoise::None,
    };
    Ok(TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id: eq.to_owned(), parameters, noise })
}

fn parse_simulate_parameters(raw: &str, preset: &crate::i18n::LocalePreset) -> Result<Vec<(String, f64)>, String> {
    let mut out = Vec::new();
    for entry in raw.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
        let Some((name, val_s)) = entry.split_once('=') else {
            return Err(format!("Parameter `{entry}` must use `name=value` syntax."));
        };
        let key = name.trim();
        if key.is_empty() {
            return Err(String::from("Parameter name must not be empty."));
        }
        let val = parse_localized_number_input(val_s.trim(), preset)
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("Parameter `{key}` has non-numeric value `{}`.", val_s.trim()))?;
        out.push((key.to_owned(), val));
    }
    Ok(out)
}

fn validate_and_build_random_sample_spec(ds: Option<&DataSheet>, state: &TransformModalsEguiState) -> Result<TransformAnalysisSpec, String> {
    let sample_size = state
        .rand_sample_size_draft
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|&n| n >= 1)
        .ok_or_else(|| String::from("Sample size must be an integer >= 1."))?;
    let seed = state.rand_seed_draft.trim().parse::<u64>().map_err(|_| String::from("RNG seed must be a non-negative integer."))?;
    if let (Some(col), Some(sheet)) = (state.rand_stratify_col, ds)
        && col >= sheet.archetype.column_count()
    {
        return Err(format!("Stratification column index {col} is out of range for `{}`.", sheet.title));
    }
    Ok(TransformAnalysisSpec::RandomSample {
        seed,
        sample_size,
        stratify_column: state.rand_stratify_col,
        proportional_allocation: state.rand_proportional_allocation,
    })
}

fn resolve_extract_column_bindings(ds: &DataSheet, state: &TransformModalsEguiState, target_kind: TableKind) -> Result<Option<Vec<usize>>, String> {
    let mut raw_fields = vec![state.extract_binding_x.trim(), state.extract_binding_y.trim()];
    if matches!(target_kind, TableKind::Grouped | TableKind::Nested | TableKind::Survival) {
        raw_fields.push(state.extract_binding_grp.trim());
    }
    let non_empty: Vec<&str> = raw_fields.into_iter().filter(|s| !s.is_empty()).collect();
    if non_empty.is_empty() {
        return Ok(None);
    }

    let col_count = ds.archetype.column_count();
    let titles: Vec<String> = (0..col_count).map(|c| ds.title_at(c)).collect();
    let axis_title = match &ds.archetype {
        TableArchetype::Xy(t) => Some(if t.x_title.trim().is_empty() { String::from("X") } else { t.x_title.clone() }),
        TableArchetype::Survival(t) => Some(if t.time_title.trim().is_empty() { String::from("Time") } else { t.time_title.clone() }),
        _ => None,
    };

    let mut cols = Vec::new();
    for field in non_empty {
        if let Some(idx) = match_single_column_title(&titles, field) {
            if !cols.contains(&idx) {
                cols.push(idx);
            }
            continue;
        }
        if axis_title.as_deref().is_some_and(|at| at.trim().eq_ignore_ascii_case(field)) {
            continue;
        }
        for tok in field.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(idx) = match_single_column_title(&titles, tok) {
                if !cols.contains(&idx) {
                    cols.push(idx);
                }
            } else if axis_title.as_deref().is_some_and(|at| at.trim().eq_ignore_ascii_case(tok)) {
                continue;
            } else {
                return Err(format!("Unknown column binding `{tok}` on sheet `{}`.", ds.title));
            }
        }
    }
    Ok(if cols.is_empty() { None } else { Some(cols) })
}

fn match_single_column_title(titles: &[String], needle: &str) -> Option<usize> {
    titles.iter().position(|t| t.trim() == needle).or_else(|| titles.iter().position(|t| t.trim().eq_ignore_ascii_case(needle)))
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
                        meta.record_modal_response(
                            box_ui,
                            &r,
                            FrameInspectEntryInput::modal(&format!("edit.transform.hook_item.{found}"), "menuitem", &lbl, None, true),
                        );
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

fn render_transform_footer(ui: &mut Ui, cx: &mut TransformModalCtx<'_>, kind: TransformModalKind) {
    let app = cx.app;
    let ds = cx.ds;
    let from_edit = cx.from_edit;
    let tokens = cx.tokens;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;
    ui.horizontal(|footer| {
        let cancel_lbl = tr(tokens.locale, "btn.cancel");
        let cancel_resp = widgets::secondary_button(footer, cancel_lbl);
        meta.record_modal_response(footer, &cancel_resp, FrameInspectEntryInput::modal("modal.cancel", "button", cancel_lbl, None, true));
        if cancel_resp.clicked() {
            if from_edit {
                intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
            } else {
                intents.push(UiIntent::CloseTransformModal);
            }
        }

        footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
            let ok_lbl = tr(tokens.locale, "btn.ok");
            let can_ok = state.validation_error.is_none();
            let ok_resp = widgets::primary_button_enabled(rhs, ok_lbl, can_ok);
            meta.record_modal_response(rhs, &ok_resp, FrameInspectEntryInput::modal("modal.ok", "button", ok_lbl, None, can_ok));
            if ok_resp.clicked() {
                if from_edit {
                    if commit_edit_transform_draft_if_needed(app, ds, kind, state, intents) {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                        push_normalize_graph_if_requested(app, kind, state, intents);
                    }
                } else {
                    if let Some(sheet) = ds {
                        match build_standalone_transform_params(app, sheet, kind, state) {
                            Ok(cmd_params) => {
                                intents.push_engine("transform.run", cmd_params);
                                intents.push(UiIntent::CloseTransformModal);
                            }
                            Err(err) => {
                                state.validation_error = Some(err);
                            }
                        }
                    } else {
                        intents.push(UiIntent::CloseTransformModal);
                    }
                }
            }

            if from_edit {
                let apply_lbl = tr(tokens.locale, "btn.apply");
                let apply_resp = rhs.add_enabled(can_ok, egui::Button::new(apply_lbl));
                meta.record_modal_response(rhs, &apply_resp, FrameInspectEntryInput::modal("modal.apply", "button", apply_lbl, None, can_ok));
                if apply_resp.clicked() && commit_edit_transform_draft_if_needed(app, ds, kind, state, intents) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("apply") });
                    push_normalize_graph_if_requested(app, kind, state, intents);
                }
            }
        });
    });
}

fn push_normalize_graph_if_requested(app: &UiApp, kind: TransformModalKind, state: &TransformModalsEguiState, intents: &mut PendingUiIntents) {
    if kind != TransformModalKind::Normalize || !state.create_new_graph {
        return;
    }
    if let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog
        && !app.document().graph_sheets.iter().any(|g| g.spec.primary_source_sheet_id == *result_sheet_id)
    {
        intents.push_engine(
            "graph.create",
            json!({
                "data_sheet_id": result_sheet_id.as_str(),
                "initialized": true,
            }),
        );
    }
}

fn commit_edit_transform_draft_if_needed(
    app: &UiApp,
    ds: Option<&DataSheet>,
    kind: TransformModalKind,
    state: &mut TransformModalsEguiState,
    intents: &mut PendingUiIntents,
) -> bool {
    match kind {
        TransformModalKind::TransformValues => {
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
        TransformModalKind::Simulate => match validate_and_build_simulate_spec(state, &app.locale_preset) {
            Ok(tr_spec) => {
                if let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() {
                    spec.kind = AnalysisKind::Transform(tr_spec);
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
                }
                true
            }
            Err(err) => {
                state.validation_error = Some(err);
                false
            }
        },
        TransformModalKind::RandomSample => match validate_and_build_random_sample_spec(ds, state) {
            Ok(tr_spec) => {
                if let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() {
                    spec.kind = AnalysisKind::Transform(tr_spec);
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
                }
                true
            }
            Err(err) => {
                state.validation_error = Some(err);
                false
            }
        },
        TransformModalKind::ExtractAndRearrange => {
            let target_kind = state.extract_target_kind.unwrap_or(TableKind::Xy);
            if let Some(sheet) = ds {
                match resolve_extract_column_bindings(sheet, state, target_kind) {
                    Ok(Some(cols)) => {
                        if let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() {
                            spec.selected_data_sets = cols;
                            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
                        }
                        true
                    }
                    Ok(None) => true,
                    Err(err) => {
                        state.validation_error = Some(err);
                        false
                    }
                }
            } else {
                true
            }
        }
        _ => true,
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
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec.clone()) });
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
            let mut obj = json!({
                "source_sheet_id": ds.id.as_str(),
                "kind": AnalysisKind::Transform(TransformAnalysisSpec::Math {
                    x_transform,
                    y_transform,
                    error_propagation: state.standalone_error_propagation,
                }),
            });
            if state.create_new_graph
                && let Some(m) = obj.as_object_mut()
            {
                m.insert(String::from("create_graph"), Value::Bool(true));
            }
            Ok(obj)
        }
        TransformModalKind::Normalize => {
            let mut obj = json!({
                "source_sheet_id": ds.id.as_str(),
                "kind": "normalize",
                "zero_anchor": match state.norm_zero_idx { 1 => json!("first_row"), 2 => json!({ "FixedValue": 0.0 }), _ => json!("smallest") },
                "hundred_anchor": match state.norm_hundred_idx { 1 => "column_sum", 2 => "last_row", _ => "largest" },
                "as_fraction": !state.norm_percent,
                "per_data_set": state.norm_per_dataset,
            });
            if state.create_new_graph
                && let Some(m) = obj.as_object_mut()
            {
                m.insert(String::from("create_graph"), Value::Bool(true));
            }
            Ok(obj)
        }
        TransformModalKind::RemoveBaseline => Ok(json!({
            "source_sheet_id": ds.id.as_str(),
            "kind": "remove_baseline",
            "baseline_columns": if state.baseline_cols.is_empty() { vec![0] } else { state.baseline_cols.clone() },
            "operation": match state.baseline_op {
                1 => "ratio",
                2 => "percent_change",
                3 => "fractional_change",
                _ => "difference",
            },
        })),
        TransformModalKind::MvFilterAndTransform | TransformModalKind::ExtractAndRearrange => {
            let default_target = if kind == TransformModalKind::MvFilterAndTransform { TableKind::MultipleVariables } else { TableKind::Xy };
            let filter_formula = (!state.filter_formula_draft.trim().is_empty()).then(|| state.filter_formula_draft.trim().to_owned());
            let target_archetype = state.extract_target_kind.unwrap_or(default_target);
            let derived_columns =
                state.derived_cols.iter().map(|(title, formula)| DerivedColumnFormula::new(title.clone(), formula.clone())).collect();
            let bound_cols =
                if kind == TransformModalKind::ExtractAndRearrange { resolve_extract_column_bindings(ds, state, target_archetype)? } else { None };
            let selected_cols = bound_cols.unwrap_or_else(|| state.standalone_selected_cols.clone());
            let kind_val = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                filter_formula,
                target_archetype,
                derived_columns,
            }))
            .map_err(|err| format!("Failed to encode select/transform spec: {err}"))?;
            let mut obj = json!({
                "source_sheet_id": ds.id.as_str(),
                "kind": kind_val,
            });
            if !selected_cols.is_empty()
                && let Some(m) = obj.as_object_mut()
            {
                m.insert(String::from("selected_data_sets"), json!(selected_cols));
            }
            Ok(obj)
        }
        TransformModalKind::Transpose => Ok(json!({
            "source_sheet_id": ds.id.as_str(),
            "kind": "transpose",
            "swap_row_and_column_titles": state.transpose_swap_titles,
        })),
        TransformModalKind::PruneRows => Ok(json!({
            "source_sheet_id": ds.id.as_str(),
            "kind": "prune_rows",
            "every_k_rows": state.prune_every_k_rows.max(1),
            "average_window": state.prune_average_window,
        })),
        TransformModalKind::Simulate => {
            let tr_spec = validate_and_build_simulate_spec(state, &app.locale_preset)?;
            let kind_val = serde_json::to_value(AnalysisKind::Transform(tr_spec)).map_err(|err| format!("Failed to encode simulate spec: {err}"))?;
            Ok(json!({
                "source_sheet_id": ds.id.as_str(),
                "kind": kind_val,
            }))
        }
        TransformModalKind::RandomSample => {
            let tr_spec = validate_and_build_random_sample_spec(Some(ds), state)?;
            let kind_val =
                serde_json::to_value(AnalysisKind::Transform(tr_spec)).map_err(|err| format!("Failed to encode random sample spec: {err}"))?;
            Ok(json!({
                "source_sheet_id": ds.id.as_str(),
                "kind": kind_val,
            }))
        }
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
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    if state.synced_exclude_tag.is_empty() {
        state.synced_exclude_tag = app.canvas_dialogs.exclude_with_reason.reason_tag.clone();
        state.exclude_reason_draft = app.canvas_dialogs.exclude_with_reason.reason_tag.clone();
        state.exclude_note_draft = app.canvas_dialogs.exclude_with_reason.note.clone();
    }

    let title = tr(tokens.locale, "w6.view.modal.exclude_with_reason");
    let vp = ctx.content_rect();
    let dlg_w = (vp.width() - 24.0).clamp(280.0, 460.0);

    let wants_kb_before = ctx.egui_wants_keyboard_input();
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_exclude_reason_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (dlg_w - 24.0).max(240.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.heading(RichText::new(title).color(tokens.text_primary));
        ui.add_space(6.0);
        ui.label(format!("Target cells selected: {}", app.canvas_dialogs.exclude_with_reason.coords.len()));
        ui.add_space(4.0);
        ui.label("Preset reason tags:");
        ui.horizontal_wrapped(|row| {
            for (idx, tag) in ExcludeWithReasonDialogState::preset_reason_tags().iter().enumerate() {
                let active = app.canvas_dialogs.exclude_with_reason.reason_tag == *tag;
                let r = choice_pill(row, tokens, tag, active, true);
                meta.record_modal_response(
                    row,
                    &r,
                    FrameInspectEntryInput::modal(&format!("exclude.preset.{idx}"), "radio", tag, Some(active.to_string()), true),
                );
                if r.clicked() {
                    state.exclude_reason_draft = (*tag).to_owned();
                    let mut f = Map::new();
                    f.insert(String::from("reason_tag"), Value::String((*tag).to_owned()));
                    intents.push(UiIntent::PatchActiveDialog { fields: f });
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|row| {
            row.label("Reason Tag:");
            let tr_box = row.add(egui::TextEdit::singleline(&mut state.exclude_reason_draft).hint_text("Reason tag").desired_width(180.0));
            meta.record_modal_response(
                row,
                &tr_box,
                FrameInspectEntryInput::modal("exclude.reason_tag", "textbox", "Reason Tag", Some(state.exclude_reason_draft.clone()), true),
            );
            if tr_box.changed() {
                let mut f = Map::new();
                f.insert(String::from("reason_tag"), Value::String(state.exclude_reason_draft.clone()));
                intents.push(UiIntent::PatchActiveDialog { fields: f });
            }
        });
        ui.label("Audit Note:");
        let note_id = egui::Id::unique("exclude.note.input");
        if !ui.ctx().memory(|m| m.has_focus(note_id)) && state.exclude_note_draft != app.canvas_dialogs.exclude_with_reason.note {
            state.exclude_note_draft = app.canvas_dialogs.exclude_with_reason.note.clone();
        }
        let nr = ui.add(
            egui::TextEdit::multiline(&mut state.exclude_note_draft)
                .id(note_id)
                .hint_text("Optional audit note…")
                .desired_rows(2)
                .desired_width((dlg_w - 44.0).max(160.0)),
        );
        meta.record_modal_response(
            ui,
            &nr,
            FrameInspectEntryInput::modal("exclude.note", "textbox", "Audit Note", Some(state.exclude_note_draft.clone()), true),
        );
        if nr.changed() {
            let mut f = Map::new();
            f.insert(String::from("note"), Value::String(state.exclude_note_draft.clone()));
            intents.push(UiIntent::PatchActiveDialog { fields: f });
        }

        ui.add_space(8.0);
        ui.horizontal(|footer| {
            let cancel_lbl = tr(tokens.locale, "btn.cancel");
            let cancel_resp = widgets::secondary_button(footer, cancel_lbl);
            meta.record_modal_response(footer, &cancel_resp, FrameInspectEntryInput::modal("modal.cancel", "button", cancel_lbl, None, true));
            if cancel_resp.clicked() {
                intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
            }
            footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
                let ok_lbl = tr(tokens.locale, "btn.ok");
                let can_ok = !state.exclude_reason_draft.trim().is_empty() && !app.canvas_dialogs.exclude_with_reason.coords.is_empty();
                let ok_resp = widgets::primary_button_enabled(rhs, ok_lbl, can_ok);
                meta.record_modal_response(rhs, &ok_resp, FrameInspectEntryInput::modal("modal.ok", "button", ok_lbl, None, can_ok));
                if ok_resp.clicked() {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
            });
        });
    });

    let title_for_ak = title.to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.exclude_with_reason", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    let can_ok = !state.exclude_reason_draft.trim().is_empty() && !app.canvas_dialogs.exclude_with_reason.coords.is_empty();
    let cmd_enter = ctx.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::Enter) || i.consume_key(Modifiers::CTRL, Key::Enter) || i.consume_key(Modifiers::MAC_CMD, Key::Enter)
    });
    let plain_enter = !wants_kb_before && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if can_ok && (cmd_enter || plain_enter) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    TransformModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn choice_pill(ui: &mut Ui, tokens: &Tokens, label: &str, selected: bool, enabled: bool) -> egui::Response {
    super::ui_kit::choice_pill(ui, tokens, label, selected, enabled)
}
