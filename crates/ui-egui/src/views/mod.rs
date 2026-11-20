//! Interactive view shard state and domain action dispatchers shared with `egui_shell`.

pub mod a11y;
pub mod analyze_picker;
pub mod dialogs_graph_misc;
pub mod fit_params;
pub mod grid_core;
pub mod import_paste;
pub mod inspector;
pub mod mv_formula;
pub mod nav_notes;
pub mod power_wizard;
pub mod results;
pub mod stat_params;
pub mod toolbar_menus;
pub mod welcome;

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;

use analyze_picker::AnalyzePickerViewState;
use dialogs_graph_misc::DialogsGraphMiscViewState;
use fit_params::FitParamsViewState;
use grid_core::GridCoreViewState;
use import_paste::ImportPasteViewState;
use inspector::InspectorViewState;
use mv_formula::MvFormulaViewState;
use nav_notes::NavNotesViewState;
use power_wizard::PowerWizardViewState;
use results::ResultsViewState;
use stat_params::StatParamsViewState;
use toolbar_menus::ToolbarMenusViewState;
use welcome::WelcomeViewState;

/// Aggregated transient UI state for all interactive view shards wired onto [`UiApp`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ViewShardStates {
    pub a11y: a11y::A11yViewState,
    pub welcome: WelcomeViewState,
    pub nav_notes: NavNotesViewState,
    pub toolbar_menus: ToolbarMenusViewState,
    pub grid_core: GridCoreViewState,
    pub mv_formula: MvFormulaViewState,
    pub results: ResultsViewState,
    pub analyze_picker: AnalyzePickerViewState,
    pub stat_params: StatParamsViewState,
    pub fit_params: FitParamsViewState,
    pub power_wizard: PowerWizardViewState,
    pub dialogs_graph_misc: DialogsGraphMiscViewState,
    pub import_paste: ImportPasteViewState,
    pub inspector: InspectorViewState,
}

/// Dispatch a custom shard action to the owning view shard by `domain`.
///
/// # Errors
/// Returns an error if `domain` is unknown or the shard handler fails.
pub fn dispatch_custom_view_action(
    app: &mut UiApp,
    shard_states: &mut ViewShardStates,
    domain: &str,
    payload: &str,
) -> Result<UiEventOutcome, String> {
    match domain {
        "a11y" => a11y::handle_custom_action(app, &mut shard_states.a11y, payload),
        "welcome" => welcome::handle_custom_action(app, &mut shard_states.welcome, payload),
        "nav_notes" => nav_notes::handle_custom_action(app, &mut shard_states.nav_notes, payload),
        "toolbar_menus" => toolbar_menus::handle_custom_action(app, &mut shard_states.toolbar_menus, payload),
        "grid_core" => grid_core::handle_custom_action(app, &mut shard_states.grid_core, payload),
        "mv_formula" => mv_formula::handle_custom_action(app, &mut shard_states.mv_formula, payload),
        "results" => results::handle_custom_action(app, &mut shard_states.results, payload),
        "analyze_picker" => analyze_picker::handle_custom_action(app, &mut shard_states.analyze_picker, payload),
        "stat_params" => stat_params::handle_custom_action(app, &mut shard_states.stat_params, payload),
        "fit_params" => fit_params::handle_custom_action(app, &mut shard_states.fit_params, payload),
        "power_wizard" => power_wizard::handle_custom_action(app, &mut shard_states.power_wizard, payload),
        "dialogs_graph_misc" => dialogs_graph_misc::handle_custom_action(app, &mut shard_states.dialogs_graph_misc, payload),
        "import_paste" => import_paste::handle_custom_action(app, &mut shard_states.import_paste, payload),
        "inspector" => inspector::handle_custom_action(app, &mut shard_states.inspector, payload),
        _ => Err(format!("unknown custom view action domain `{domain}`")),
    }
}
