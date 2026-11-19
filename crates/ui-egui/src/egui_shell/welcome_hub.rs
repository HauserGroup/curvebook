//! Native `egui` 3-pane Project Launcher, Table Schema Composer & Clone Pipeline Hub
//! (`docs/planning/gui-layout-spec.md` §8.1, `R15.3`).

use egui::{CornerRadius, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::layout::ZLayer;
use crate::tr;

/// Table archetypes selectable inside the 3-pane Workspace Hub (`§8.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WelcomeArchetypeChoice {
    Xy,
    #[default]
    Column,
    Grouped,
    Contingency,
    Survival,
    PartsOfWhole,
    MultipleVariables,
    Nested,
}

impl WelcomeArchetypeChoice {
    /// Ordered list of all eight supported CurveBook table archetypes.
    pub const ALL: [Self; 8] =
        [Self::Xy, Self::Column, Self::Grouped, Self::Contingency, Self::Survival, Self::PartsOfWhole, Self::MultipleVariables, Self::Nested];

    /// Canonical `sheet.createData` `kind` parameter value.
    #[must_use]
    pub const fn as_command_kind(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Column => "column",
            Self::Grouped => "grouped",
            Self::Contingency => "contingency",
            Self::Survival => "survival",
            Self::PartsOfWhole => "parts_of_whole",
            Self::MultipleVariables => "multiple_variables",
            Self::Nested => "nested",
        }
    }

    /// Concise display name shown in the Welcome Hub left rail.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Column => "Column",
            Self::Grouped => "Grouped",
            Self::Contingency => "Contingency",
            Self::Survival => "Survival",
            Self::PartsOfWhole => "Parts of Whole",
            Self::MultipleVariables => "Multiple Variables",
            Self::Nested => "Nested",
        }
    }
}

/// Active left-rail entry inside the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WelcomeLeftMode {
    NewTableArchetype(WelcomeArchetypeChoice),
    StarterTemplates,
    ClonePipeline,
    OpenFile,
}

impl Default for WelcomeLeftMode {
    fn default() -> Self {
        Self::NewTableArchetype(WelcomeArchetypeChoice::Column)
    }
}

/// Editable table schema & clone draft maintained locally inside [`WelcomeHubEguiState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeSchemaDraft {
    pub archetype: WelcomeArchetypeChoice,
    pub title: String,
    pub cols: usize,
    pub rows: usize,
    pub subcols: usize,
    pub paired_design: bool,
    pub auto_graph: bool,
    pub selected_template_index: usize,
    pub clone_source_sheet_id: Option<curvebook_doc::SheetId>,
    pub clone_include_downstream: bool,
    pub clone_clear_values: bool,
}

impl Default for WelcomeSchemaDraft {
    fn default() -> Self {
        Self {
            archetype: WelcomeArchetypeChoice::Column,
            title: String::from("Column Table 1"),
            cols: 4,
            rows: 16,
            subcols: 1,
            paired_design: false,
            auto_graph: true,
            selected_template_index: 0,
            clone_source_sheet_id: None,
            clone_include_downstream: true,
            clone_clear_values: true,
        }
    }
}

/// Persistent `egui_shell` state for the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WelcomeHubEguiState {
    pub left_mode: WelcomeLeftMode,
    pub schema: WelcomeSchemaDraft,
    pub clone_options_open: bool,
}

/// Per-frame layout report returned by [`render_welcome_hub`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WelcomeHubFrameReport {
    pub card_rect_pt: Option<egui::Rect>,
    pub cancel_visible: bool,
}

/// Render the centered 3-pane `Welcome & Workspace Hub` inside `CenterStage` (`§8.1`).
///
/// The Wave 3 base establishes the bounded card contract, `Cancel` visibility gating (`total_sheet_count() == 0`),
/// and `Create` / `Open` command wiring; `r15-welcome-nav` implements the full 3-pane composer,
/// `.cbtemplate` starter list, live structural schematic, and `Clone Options` sub-modal.
pub fn render_welcome_hub(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> WelcomeHubFrameReport {
    let mut report = WelcomeHubFrameReport::default();
    let avail = ui.available_rect_before_wrap();
    if avail.width() < 120.0 || avail.height() < 90.0 {
        return report;
    }

    let doc_empty = app.document().total_sheet_count() == 0;
    report.cancel_visible = !doc_empty;

    let card_w = (avail.width() - 32.0).clamp(180.0, 860.0);
    let card_h = (avail.height() - 32.0).clamp(140.0, 560.0);
    let card_rect = egui::Rect::from_center_size(avail.center(), Vec2::new(card_w, card_h));
    report.card_rect_pt = Some(card_rect);

    ui.painter().rect_filled(card_rect, CornerRadius::same(8), tokens.panel_bg);
    ui.painter().rect_stroke(card_rect, CornerRadius::same(8), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    ui.scope_builder(egui::UiBuilder::new().max_rect(card_rect.shrink(16.0)), |ui| {
        ui.heading(tr(&app.locale, "app.welcome.title"));
        ui.label("Choose a table archetype, starter template, or clone an existing pipeline.");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            for arch in WelcomeArchetypeChoice::ALL {
                let selected = matches!(state.left_mode, WelcomeLeftMode::NewTableArchetype(a) if a == arch);
                let resp = ui.add_enabled(enabled, egui::Button::new(arch.label()).selected(selected));
                let wid = format!("welcome.archetype.{}", arch.as_command_kind());
                meta.record_response(
                    ui,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: &wid,
                        role: "button",
                        label: arch.label(),
                        command_id: None,
                        value: Some(json!(selected)),
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if resp.clicked() && enabled {
                    state.left_mode = WelcomeLeftMode::NewTableArchetype(arch);
                    state.schema.archetype = arch;
                }
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let create_resp = ui.add_enabled(enabled, egui::Button::new("Create Table"));
            meta.record_response(
                ui,
                &create_resp,
                FrameInspectEntryInput {
                    widget_id: "welcome.create",
                    role: "button",
                    label: "Create Table",
                    command_id: Some("sheet.createData"),
                    value: None,
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if create_resp.clicked() && enabled {
                intents.push_engine(
                    "sheet.createData",
                    json!({
                        "title": state.schema.title,
                        "kind": state.schema.archetype.as_command_kind(),
                        "cols": state.schema.cols,
                        "rows": state.schema.rows,
                        "auto_graph": state.schema.auto_graph,
                    }),
                );
                intents.push_set_launcher_open(false);
            }

            if report.cancel_visible {
                let cancel_resp = ui.add_enabled(enabled, egui::Button::new(tr(&app.locale, "btn.cancel")));
                meta.record_response(
                    ui,
                    &cancel_resp,
                    FrameInspectEntryInput {
                        widget_id: "welcome.cancel",
                        role: "button",
                        label: tr(&app.locale, "btn.cancel"),
                        command_id: None,
                        value: None,
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if cancel_resp.clicked() && enabled {
                    intents.push_set_launcher_open(false);
                }
            }
        });
    });

    report
}
