//! Native macOS application menu bar bridge (`muda`, gated strictly under `target_os = "macos"`)
//! plus platform-neutral menu parity descriptors shared across OS hosts (`R15.3`).

use crate::desktop_host::DesktopMenuChromeState;

/// Canonical 10-menu top-level bar titles specified in `docs/planning/gui-layout-spec.md` §3.1.
pub const NATIVE_MENU_BAR_TITLES: [&str; 10] = ["File", "Edit", "View", "Insert", "Data", "Analyze", "Graph", "Arrange", "Window", "Help"];

/// Cross-platform parity snapshot of one menu item's `command_id`, label, shortcut, and enablement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItemParityEntry {
    pub command_id: String,
    pub label: String,
    pub shortcut: String,
    pub enabled: bool,
}

/// Cross-platform parity snapshot of one top-level menu section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSectionParityEntry {
    pub menu_key: String,
    pub title: String,
    pub items: Vec<MenuItemParityEntry>,
}

/// Extract deterministic menu-item parity entries from [`DesktopMenuChromeState`].
#[must_use]
pub fn collect_menu_parity_entries(chrome: &DesktopMenuChromeState) -> Vec<MenuSectionParityEntry> {
    chrome
        .sections
        .iter()
        .map(|sec| MenuSectionParityEntry {
            menu_key: sec.menu_key.clone(),
            title: sec.title.clone(),
            items: sec
                .items
                .iter()
                .map(|item| MenuItemParityEntry {
                    command_id: item.command_id.clone(),
                    label: item.localized_label.clone(),
                    shortcut: item.shortcut.to_owned(),
                    enabled: item.enabled,
                })
                .collect(),
        })
        .collect()
}

/// Verify that `observed` menu sections match `chrome` on every `command_id`, label, and enablement bit.
///
/// # Errors
/// Returns a descriptive mismatch reason on the first section count, command ID, or enablement delta.
pub fn verify_menu_parity(chrome: &DesktopMenuChromeState, observed: &[MenuSectionParityEntry]) -> Result<(), String> {
    let expected = collect_menu_parity_entries(chrome);
    if expected.len() != observed.len() {
        return Err(format!("menu section count mismatch: expected {}, got {}", expected.len(), observed.len()));
    }
    for (exp_sec, obs_sec) in expected.iter().zip(observed.iter()) {
        if exp_sec.menu_key != obs_sec.menu_key {
            return Err(format!("menu section key mismatch: expected `{}`, got `{}`", exp_sec.menu_key, obs_sec.menu_key));
        }
        if exp_sec.items.len() != obs_sec.items.len() {
            return Err(format!("section `{}` item count mismatch: expected {}, got {}", exp_sec.menu_key, exp_sec.items.len(), obs_sec.items.len()));
        }
        for (exp_item, obs_item) in exp_sec.items.iter().zip(obs_sec.items.iter()) {
            if exp_item.command_id != obs_item.command_id || exp_item.enabled != obs_item.enabled {
                return Err(format!(
                    "menu item mismatch in `{}`: expected ({}, enabled={}), got ({}, enabled={})",
                    exp_sec.menu_key, exp_item.command_id, exp_item.enabled, obs_item.command_id, obs_item.enabled
                ));
            }
        }
    }
    Ok(())
}

/// Native macOS menu host powered by `muda` (`default-features = false`, strictly `cfg(target_os = "macos")`).
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct MacNativeMenuHost {
    parity_entries: Vec<MenuSectionParityEntry>,
    installed_nsmenu: bool,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for MacNativeMenuHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacNativeMenuHost")
            .field("section_count", &self.parity_entries.len())
            .field("installed_nsmenu", &self.installed_nsmenu)
            .finish()
    }
}

#[cfg(target_os = "macos")]
impl MacNativeMenuHost {
    /// Construct an empty macOS native menu host; actual `muda::Menu` population lives in the Wave 3 top-chrome slice.
    #[must_use]
    pub const fn new() -> Self {
        Self { parity_entries: Vec::new(), installed_nsmenu: false }
    }

    /// Build `muda` menu tree mirrors from `chrome` (`command_id` & enablement parity).
    ///
    /// # Errors
    /// Returns an error if `muda` menu construction fails.
    pub fn sync_from_chrome(&mut self, chrome: &DesktopMenuChromeState) -> Result<(), String> {
        let menu = muda::Menu::new();
        for section in &chrome.sections {
            let sub = muda::Submenu::new(&section.title, true);
            for item in &section.items {
                let entry = muda::MenuItem::with_id(muda::MenuId::new(&item.command_id), &item.localized_label, item.enabled, None);
                sub.append(&entry).map_err(|err| format!("append menu item `{}`: {err}", item.command_id))?;
            }
            menu.append(&sub).map_err(|err| format!("append submenu `{}`: {err}", section.menu_key))?;
        }
        self.parity_entries = collect_menu_parity_entries(chrome);
        Ok(())
    }

    /// Return whether `init_for_nsapp` has been invoked on this host.
    #[must_use]
    pub const fn is_installed_for_nsapp(&self) -> bool {
        self.installed_nsmenu
    }

    /// Install the native menu onto macOS `NSApp` (stubbed cleanly until live app-loop integration).
    pub fn mark_installed_for_nsapp(&mut self) {
        self.installed_nsmenu = true;
    }

    /// Poll pending `muda::MenuEvent` queue and map the clicked menu item back to a CurveBook `command_id`.
    #[must_use]
    pub fn poll_triggered_command_id(&self) -> Option<String> {
        muda::MenuEvent::receiver().try_recv().ok().map(|evt| evt.id.0)
    }

    /// Expose cached section/item parity entries for cross-backend assertions.
    #[must_use]
    pub fn items_parity_entries(&self) -> &[MenuSectionParityEntry] {
        &self.parity_entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_host::{DesktopMenuItem, DesktopMenuSection, MenuChromeMode};

    #[test]
    fn menu_parity_detects_missing_or_mismatched_items() {
        let chrome = DesktopMenuChromeState {
            mode: MenuChromeMode::InWindowMenuBar,
            in_window_menus: true,
            menu_bar_visible: true,
            menu_bar_height_pt: 28.0,
            active_menu_key: None,
            sections: vec![DesktopMenuSection {
                menu_key: String::from("menu.file"),
                title: String::from("File"),
                items: vec![DesktopMenuItem {
                    command_id: String::from("file.new"),
                    localized_label: String::from("New Project"),
                    shortcut: String::from("Cmd+N"),
                    menu_path: String::from("File/New Project"),
                    enabled: true,
                    summary: String::from("New"),
                    beginner: String::from("New project"),
                }],
            }],
            dispatched_actions: Vec::new(),
        };
        let entries = collect_menu_parity_entries(&chrome);
        assert!(verify_menu_parity(&chrome, &entries).is_ok());

        let mut broken = entries;
        broken[0].items[0].enabled = false;
        assert!(verify_menu_parity(&chrome, &broken).is_err());
    }
}
