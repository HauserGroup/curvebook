//! Native macOS application menu bar bridge (`muda`, gated strictly under `target_os = "macos"`)
//! plus platform-neutral menu parity descriptors shared across OS hosts (`R15.3`).

use curvebook_ui_egui::UiApp;
#[cfg(target_os = "macos")]
use curvebook_ui_egui::egui_shell::top_chrome::submenu_variants_for_command;
use curvebook_ui_egui::egui_shell::top_chrome::{collect_in_window_menu_sections, effective_menu_item_enabled, is_menu_item_visible};
#[cfg(target_os = "macos")]
use curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES;
use curvebook_ui_egui::i18n::{normalize_locale, tr};

use crate::desktop_host::DesktopMenuChromeState;
#[cfg(target_os = "macos")]
use crate::desktop_host::{DesktopMenuItem, DesktopMenuNode};

/// Return the localized macOS application submenu `(about, settings, quit)` labels for `locale`.
#[must_use]
pub fn localized_app_submenu_labels(locale: &str) -> (&'static str, &'static str, &'static str) {
    let norm = normalize_locale(locale);
    let about = tr(norm, "menu.help.about");
    let prefs = tr(norm, "prefs.title");
    let quit = match norm {
        "de" => "CurveBook beenden",
        "es" => "Salir de CurveBook",
        "fr" => "Quitter CurveBook",
        "it" => "Esci da CurveBook",
        "ja" => "CurveBook を終了",
        "ko" => "CurveBook 종료",
        "nl" => "Stop CurveBook",
        "pl" => "Zakończ CurveBook",
        "pt-BR" => "Encerrar CurveBook",
        "ru" => "Завершить CurveBook",
        "tr" => "CurveBook'tan Çık",
        "zh-CN" => "退出 CurveBook",
        "zh-TW" => "結束 CurveBook",
        "ar" => "إنهاء CurveBook",
        "he" => "צא מ-CurveBook",
        _ => "Quit CurveBook",
    };
    (about, prefs, quit)
}

/// Return the localized macOS `Window` submenu title for `locale`.
#[must_use]
pub fn localized_window_submenu_title(locale: &str) -> &'static str {
    match normalize_locale(locale) {
        "de" => "Fenster",
        "es" => "Ventana",
        "fr" => "Fenêtre",
        "it" => "Finestra",
        "ja" => "ウィンドウ",
        "ko" => "창",
        "nl" => "Venster",
        "pl" => "Okno",
        "pt-BR" => "Janela",
        "ru" => "Окно",
        "tr" => "Pencere",
        "zh-CN" => "窗口",
        "zh-TW" => "視窗",
        "ar" => "النافذة",
        "he" => "חלון",
        _ => "Window",
    }
}

/// Ordered macOS `Window` submenu predefined entry descriptors (`muda::PredefinedMenuItem`)
/// inserted ahead of `Help` (`R20.22`).
pub const MACOS_WINDOW_SUBMENU_ITEMS: [&str; 4] = ["Minimize", "Zoom", "Enter Full Screen", "Bring All to Front"];

#[cfg(target_os = "macos")]
const MACOS_APP_SUBMENU_OWNED_COMMANDS: [&str; 3] = ["app.about", "app.preferences", "app.quit"];

/// Canonical 8-menu top-level bar titles specified in `docs/planning/gui-layout-spec.md` §3.1.
pub const NATIVE_MENU_BAR_TITLES: [&str; 8] = ["File", "Edit", "View", "Insert", "Data", "Analyze", "Graph", "Help"];

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

/// Extract deterministic, renderable menu-item parity entries from [`DesktopMenuChromeState`].
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
                .filter(|item| is_menu_item_visible(&item.command_id))
                .map(|item| MenuItemParityEntry {
                    command_id: item.command_id.clone(),
                    label: item.localized_label.clone(),
                    shortcut: item.shortcut.clone(),
                    enabled: effective_menu_item_enabled(&item.command_id, item.enabled),
                })
                .collect(),
        })
        .collect()
}

/// Extract matching in-window `egui::MenuBar` parity entries directly from [`UiApp`].
#[must_use]
pub fn collect_egui_menubar_parity(app: &UiApp) -> Vec<MenuSectionParityEntry> {
    collect_in_window_menu_sections(app)
        .into_iter()
        .map(|sec| MenuSectionParityEntry {
            menu_key: sec.menu_key,
            title: sec.title,
            items: sec
                .items
                .into_iter()
                .map(|item| MenuItemParityEntry { command_id: item.command_id, label: item.label, shortcut: item.shortcut, enabled: item.enabled })
                .collect(),
        })
        .collect()
}

/// Verify that `observed` menu sections match `chrome` on every `menu_key`, section `title`,
/// `command_id`, localized `label`, `shortcut`, and `enabled` bit.
///
/// # Errors
/// Returns a descriptive mismatch reason on the first section count, title, command ID, label,
/// shortcut, or enablement delta.
pub fn verify_menu_parity(chrome: &DesktopMenuChromeState, observed: &[MenuSectionParityEntry]) -> Result<(), String> {
    let expected = collect_menu_parity_entries(chrome);
    if expected.len() != observed.len() {
        return Err(format!("menu section count mismatch: expected {}, got {}", expected.len(), observed.len()));
    }
    for (exp_sec, obs_sec) in expected.iter().zip(observed.iter()) {
        if exp_sec.menu_key != obs_sec.menu_key {
            return Err(format!("menu section key mismatch: expected `{}`, got `{}`", exp_sec.menu_key, obs_sec.menu_key));
        }
        if exp_sec.title != obs_sec.title {
            return Err(format!("menu section `{}` title mismatch: expected `{}`, got `{}`", exp_sec.menu_key, exp_sec.title, obs_sec.title));
        }
        if exp_sec.items.len() != obs_sec.items.len() {
            return Err(format!("section `{}` item count mismatch: expected {}, got {}", exp_sec.menu_key, exp_sec.items.len(), obs_sec.items.len()));
        }
        for (exp_item, obs_item) in exp_sec.items.iter().zip(obs_sec.items.iter()) {
            if exp_item != obs_item {
                return Err(format!("menu item mismatch in `{}`: expected {:?}, got {:?}", exp_sec.menu_key, exp_item, obs_item));
            }
        }
    }
    Ok(())
}

/// Pure `muda` leaf item specification constructed off AppKit main thread for panic-free testing.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MudaMenuItemSpec {
    pub menu_id: muda::MenuId,
    pub command_id: String,
    pub label: String,
    pub shortcut: String,
    pub accelerator: Option<muda::accelerator::Accelerator>,
    pub enabled: bool,
    pub variants: Vec<(String, String)>,
}

/// Hierarchical node in a `muda` menu section.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MudaMenuNodeSpec {
    Action(MudaMenuItemSpec),
    Separator,
    Submenu { id: String, title: String, children: Vec<Self> },
}

/// Pure `muda` submenu specification constructed off AppKit main thread for panic-free testing.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MudaMenuSectionSpec {
    pub menu_key: String,
    pub title: String,
    pub nodes: Vec<MudaMenuNodeSpec>,
    pub items: Vec<MudaMenuItemSpec>,
}

/// Parse CurveBook shortcut notation (e.g. `"Cmd+Shift+I"`, `"Cmd+\\"`, `"Cmd++"`, `"Cmd+/"`,
/// `"F6"`, `"Cmd+K / Ctrl+K"`) into a [`muda::accelerator::Accelerator`] without panicking on
/// unknown or empty strings.
#[cfg(target_os = "macos")]
#[must_use]
pub fn parse_muda_accelerator(shortcut: &str) -> Option<muda::accelerator::Accelerator> {
    use muda::accelerator::{Accelerator, Code, Modifiers};

    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Split alternative bindings like `"Cmd+K / Ctrl+K"` without slicing literal slash keys
    // such as `"/"`, `"Cmd+/"`, or `"Cmd+Shift+/"`.
    let primary = match trimmed
        .char_indices()
        .find(|&(idx, ch)| ch == '/' && idx > 0 && !trimmed.get(..idx).is_some_and(|prefix| prefix.trim_end().ends_with('+')))
    {
        Some((idx, _)) => trimmed.get(..idx)?.trim(),
        None => trimmed,
    };
    if primary.is_empty() {
        return None;
    }

    let mut mods = Modifiers::empty();
    let (mod_tokens, key_token): (Vec<&str>, &str) = if primary == "+" {
        (Vec::new(), "+")
    } else if let Some(before_last_plus) = primary.strip_suffix('+') {
        let prefix = before_last_plus.trim_end().strip_suffix('+')?;
        let prefix_trimmed = prefix.trim();
        if prefix_trimmed.is_empty() || prefix_trimmed.ends_with('+') {
            return None;
        }
        (prefix_trimmed.split('+').map(str::trim).collect(), "+")
    } else if primary.contains('+') {
        let parts: Vec<&str> = primary.split('+').map(str::trim).collect();
        let (&last_tok, head_toks) = parts.split_last()?;
        (head_toks.to_vec(), last_tok)
    } else {
        (Vec::new(), primary)
    };

    for tok in mod_tokens {
        match tok.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" | "cmdorctrl" => mods |= Modifiers::META,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "opt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            _ => return None,
        }
    }

    let key = parse_muda_key_code(key_token)?;
    let _ = Code::KeyA;
    Some(Accelerator::new(mods, key))
}

#[cfg(target_os = "macos")]
fn parse_muda_key_code(raw: &str) -> Option<muda::accelerator::Code> {
    use muda::accelerator::Code;

    let upper = raw.trim().to_ascii_uppercase();
    match upper.as_str() {
        "A" => Some(Code::KeyA),
        "B" => Some(Code::KeyB),
        "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD),
        "E" => Some(Code::KeyE),
        "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG),
        "H" => Some(Code::KeyH),
        "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ),
        "K" => Some(Code::KeyK),
        "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM),
        "N" => Some(Code::KeyN),
        "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP),
        "Q" => Some(Code::KeyQ),
        "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS),
        "T" => Some(Code::KeyT),
        "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV),
        "W" => Some(Code::KeyW),
        "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY),
        "Z" => Some(Code::KeyZ),
        "0" => Some(Code::Digit0),
        "1" => Some(Code::Digit1),
        "2" => Some(Code::Digit2),
        "3" => Some(Code::Digit3),
        "4" => Some(Code::Digit4),
        "5" => Some(Code::Digit5),
        "6" => Some(Code::Digit6),
        "7" => Some(Code::Digit7),
        "8" => Some(Code::Digit8),
        "9" => Some(Code::Digit9),
        "F1" => Some(Code::F1),
        "F2" => Some(Code::F2),
        "F3" => Some(Code::F3),
        "F4" => Some(Code::F4),
        "F5" => Some(Code::F5),
        "F6" => Some(Code::F6),
        "F7" => Some(Code::F7),
        "F8" => Some(Code::F8),
        "F9" => Some(Code::F9),
        "F10" => Some(Code::F10),
        "F11" => Some(Code::F11),
        "F12" => Some(Code::F12),
        "\\" | "BACKSLASH" => Some(Code::Backslash),
        "/" | "SLASH" => Some(Code::Slash),
        "," | "COMMA" => Some(Code::Comma),
        "." | "PERIOD" => Some(Code::Period),
        "-" | "MINUS" => Some(Code::Minus),
        "=" | "+" | "EQUAL" | "PLUS" => Some(Code::Equal),
        "ENTER" | "RETURN" => Some(Code::Enter),
        "ESC" | "ESCAPE" => Some(Code::Escape),
        "SPACE" => Some(Code::Space),
        "TAB" => Some(Code::Tab),
        "BACKSPACE" => Some(Code::Backspace),
        "DELETE" | "DEL" => Some(Code::Delete),
        "HOME" => Some(Code::Home),
        "END" => Some(Code::End),
        "PAGEUP" => Some(Code::PageUp),
        "PAGEDOWN" => Some(Code::PageDown),
        "ARROWUP" | "UP" => Some(Code::ArrowUp),
        "ARROWDOWN" | "DOWN" => Some(Code::ArrowDown),
        "ARROWLEFT" | "LEFT" => Some(Code::ArrowLeft),
        "ARROWRIGHT" | "RIGHT" => Some(Code::ArrowRight),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
type MenuStructureSignature = ((String, String, String, String), Vec<(String, String, Vec<(String, String, String)>)>);

/// Native macOS menu host powered by `muda` (`default-features = false`, strictly `cfg(target_os = "macos")`).
///
/// Pure descriptor / `MenuId` / `Accelerator` building executes cleanly off the AppKit thread for
/// unit tests and headless `--control` runs; OS `muda::Menu` / `NSMenu` installation is guarded
/// strictly inside [`Self::attach_to_nsapp_for_windowed_host`].
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct MacNativeMenuHost {
    section_specs: Vec<MudaMenuSectionSpec>,
    parity_entries: Vec<MenuSectionParityEntry>,
    active_locale: String,
    structure_signature: MenuStructureSignature,
    enablement_signature: Vec<bool>,
    live_nsapp_mode: bool,
    installed_nsmenu: bool,
    live_menu: Option<muda::Menu>,
    live_items: Vec<(usize, muda::MenuItem)>,
    live_variant_items: Vec<(String, muda::MenuItem)>,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for MacNativeMenuHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacNativeMenuHost")
            .field("section_count", &self.parity_entries.len())
            .field("live_nsapp_mode", &self.live_nsapp_mode)
            .field("installed_nsmenu", &self.installed_nsmenu)
            .finish()
    }
}

#[cfg(target_os = "macos")]
static MAC_MENU_EVENT_QUEUE: std::sync::OnceLock<curvebook_platform::sync::Mutex<std::collections::VecDeque<String>>> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
static MAC_MENU_REPAINT_CTX: std::sync::OnceLock<curvebook_platform::sync::Mutex<Option<eframe::egui::Context>>> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
static MAC_MENU_EVENT_HANDLER_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Register a `muda::MenuEvent` handler that buffers triggered menu IDs and wakes an idle `egui`
/// event loop via [`eframe::egui::Context::request_repaint`].
#[cfg(target_os = "macos")]
pub fn install_egui_repaint_event_handler(ctx: eframe::egui::Context) {
    let ctx_slot = MAC_MENU_REPAINT_CTX.get_or_init(|| curvebook_platform::sync::Mutex::named("mac_menu.repaint_ctx", None));
    if let Ok(mut guard) = ctx_slot.lock() {
        *guard = Some(ctx);
    }
    if !MAC_MENU_EVENT_HANDLER_INSTALLED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        let _ =
            MAC_MENU_EVENT_QUEUE.get_or_init(|| curvebook_platform::sync::Mutex::named("mac_menu.event_queue", std::collections::VecDeque::new()));
        muda::MenuEvent::set_event_handler(Some(move |evt: muda::MenuEvent| {
            if let Some(queue) = MAC_MENU_EVENT_QUEUE.get()
                && let Ok(mut guard) = queue.lock()
            {
                guard.push_back(evt.id.0);
            }
            if let Some(slot) = MAC_MENU_REPAINT_CTX.get()
                && let Ok(guard) = slot.lock()
                && let Some(ref egui_ctx) = *guard
            {
                egui_ctx.request_repaint();
            }
        }));
    }
}

#[cfg(target_os = "macos")]
impl MacNativeMenuHost {
    /// Construct an empty macOS native menu host (no AppKit side-effects until windowed startup).
    #[must_use]
    pub fn new() -> Self {
        Self {
            section_specs: Vec::new(),
            parity_entries: Vec::new(),
            active_locale: String::from("en"),
            structure_signature: ((String::new(), String::new(), String::new(), String::new()), Vec::new()),
            enablement_signature: Vec::new(),
            live_nsapp_mode: false,
            installed_nsmenu: false,
            live_menu: None,
            live_items: Vec::new(),
            live_variant_items: Vec::new(),
        }
    }

    /// Active normalized locale mirrored by the macOS application submenu.
    #[must_use]
    pub fn active_locale(&self) -> &str {
        &self.active_locale
    }

    /// Localized macOS application submenu `(about, preferences, quit)` labels for the active locale.
    #[must_use]
    pub fn app_submenu_labels(&self) -> (&'static str, &'static str, &'static str) {
        localized_app_submenu_labels(&self.active_locale)
    }

    /// Update menu descriptors and parity entries from `chrome` cleanly on any thread, and update
    /// the live `muda::Menu` / `NSMenu` only when [`Self::attach_to_nsapp_for_windowed_host`] is active.
    ///
    /// # Errors
    /// Returns an error if live `muda` menu construction fails on macOS AppKit.
    pub fn sync_from_chrome(&mut self, chrome: &DesktopMenuChromeState) -> Result<(), String> {
        let locale = normalize_locale(&chrome.locale);
        let next_parity = collect_menu_parity_entries(chrome);
        let next_specs = build_muda_section_specs(chrome);
        let next_struct_sig = compute_structure_signature(locale, &next_specs);
        let next_enable_sig = compute_enablement_signature(&next_specs);

        self.parity_entries = next_parity;
        self.section_specs = next_specs;
        self.active_locale = locale.to_owned();

        if self.live_nsapp_mode {
            if next_struct_sig != self.structure_signature || self.live_menu.is_none() {
                self.rebuild_live_muda_menu(locale)?;
                if let Some(menu) = &self.live_menu {
                    menu.init_for_nsapp();
                    self.installed_nsmenu = true;
                }
            } else if next_enable_sig != self.enablement_signature {
                self.apply_incremental_enablement();
            }
        }

        self.structure_signature = next_struct_sig;
        self.enablement_signature = next_enable_sig;
        Ok(())
    }

    /// Initialize live AppKit `muda::Menu` and install onto `NSApp` strictly on windowed startup
    /// (never call from headless `--control` or worker test threads).
    ///
    /// # Errors
    /// Returns an error if `muda::Menu` assembly fails.
    pub fn attach_to_nsapp_for_windowed_host(&mut self, chrome: &DesktopMenuChromeState) -> Result<(), String> {
        self.live_nsapp_mode = true;
        self.sync_from_chrome(chrome)
    }

    /// Whether `init_for_nsapp` has been invoked on this host.
    #[must_use]
    pub const fn is_installed_for_nsapp(&self) -> bool {
        self.installed_nsmenu
    }

    /// Whether live AppKit `NSMenu` syncing is enabled for windowed host frames.
    #[must_use]
    pub const fn is_live_nsapp_mode(&self) -> bool {
        self.live_nsapp_mode
    }

    /// Mark host as installed onto macOS `NSApp` (testing / bookkeeping helper).
    pub fn mark_installed_for_nsapp(&mut self) {
        self.installed_nsmenu = true;
    }

    /// Poll pending `muda::MenuEvent` queue and map the clicked menu item back to a CurveBook `command_id`.
    #[must_use]
    pub fn poll_triggered_command_id(&self) -> Option<String> {
        if let Some(queue) = MAC_MENU_EVENT_QUEUE.get()
            && let Ok(mut guard) = queue.lock()
            && let Some(cmd_id) = guard.pop_front()
        {
            return Some(cmd_id);
        }
        muda::MenuEvent::receiver().try_recv().ok().map(|evt| evt.id.0)
    }

    /// Expose cached section/item parity entries for cross-backend assertions.
    #[must_use]
    pub fn items_parity_entries(&self) -> &[MenuSectionParityEntry] {
        &self.parity_entries
    }

    /// Expose pure `muda` descriptor sections (safe on non-main threads for unit/parity tests).
    #[must_use]
    pub fn muda_section_specs(&self) -> &[MudaMenuSectionSpec] {
        &self.section_specs
    }

    fn apply_incremental_enablement(&self) {
        let flat: Vec<&MudaMenuItemSpec> = self.section_specs.iter().flat_map(|s| s.items.iter()).collect();
        for (idx, item_handle) in &self.live_items {
            if let Some(spec) = flat.get(*idx) {
                item_handle.set_enabled(spec.enabled);
            }
        }
        for (cmd_prefix, var_handle) in &self.live_variant_items {
            if let Some(parent) = flat.iter().find(|s| &s.command_id == cmd_prefix) {
                var_handle.set_enabled(parent.enabled);
            }
        }
    }

    fn rebuild_live_muda_menu(&mut self, locale: &str) -> Result<(), String> {
        let (about_label, prefs_label, quit_label) = localized_app_submenu_labels(locale);
        let menu = muda::Menu::new();
        let app_sub = muda::Submenu::with_items(
            "CurveBook",
            true,
            &[
                &muda::MenuItem::with_id(muda::MenuId::new("app.about"), about_label, true, None),
                &muda::PredefinedMenuItem::separator(),
                &muda::MenuItem::with_id(muda::MenuId::new("app.preferences"), prefs_label, true, parse_muda_accelerator("Cmd+,")),
                &muda::PredefinedMenuItem::separator(),
                &muda::PredefinedMenuItem::hide(None),
                &muda::PredefinedMenuItem::hide_others(None),
                &muda::PredefinedMenuItem::show_all(None),
                &muda::PredefinedMenuItem::separator(),
                &muda::MenuItem::with_id(muda::MenuId::new("app.quit"), quit_label, true, parse_muda_accelerator("Cmd+Q")),
            ],
        )
        .map_err(|err| format!("build macOS app submenu: {err}"))?;
        menu.append(&app_sub).map_err(|err| format!("append app submenu: {err}"))?;

        let mut next_items = Vec::new();
        let mut next_variants = Vec::new();
        let mut flat_idx = 0usize;
        let mut window_inserted = false;

        for sec in &self.section_specs {
            if !window_inserted && sec.menu_key == "menu.help" {
                let win_sub = build_macos_window_submenu(locale)?;
                menu.append(&win_sub).map_err(|err| format!("append Window submenu: {err}"))?;
                window_inserted = true;
            }
            let sub = muda::Submenu::with_id(muda::MenuId::new(&sec.menu_key), &sec.title, true);
            append_muda_nodes_filtered(&sub, &sec.nodes, &mut next_items, &mut next_variants, &mut flat_idx)?;
            menu.append(&sub).map_err(|err| format!("append submenu `{}`: {err}", sec.menu_key))?;
        }
        if !window_inserted {
            let win_sub = build_macos_window_submenu(locale)?;
            menu.append(&win_sub).map_err(|err| format!("append Window submenu: {err}"))?;
        }

        self.live_menu = Some(menu);
        self.live_items = next_items;
        self.live_variant_items = next_variants;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn build_macos_window_submenu(locale: &str) -> Result<muda::Submenu, String> {
    muda::Submenu::with_id_and_items(
        muda::MenuId::new("menu.window"),
        localized_window_submenu_title(locale),
        true,
        &[
            &muda::PredefinedMenuItem::minimize(None),
            &muda::PredefinedMenuItem::maximize(None),
            &muda::PredefinedMenuItem::fullscreen(None),
            &muda::PredefinedMenuItem::separator(),
            &muda::PredefinedMenuItem::bring_all_to_front(None),
        ],
    )
    .map_err(|err| format!("build macOS Window submenu: {err}"))
}

#[cfg(target_os = "macos")]
fn append_muda_nodes_filtered(
    parent: &muda::Submenu,
    nodes: &[MudaMenuNodeSpec],
    next_items: &mut Vec<(usize, muda::MenuItem)>,
    next_variants: &mut Vec<(String, muda::MenuItem)>,
    flat_idx: &mut usize,
) -> Result<(), String> {
    let mut pending_separator = false;
    let mut emitted_any = false;

    for node in nodes {
        match node {
            MudaMenuNodeSpec::Separator => {
                if emitted_any {
                    pending_separator = true;
                }
            }
            MudaMenuNodeSpec::Submenu { id: _, title, children } => {
                if pending_separator {
                    parent.append(&muda::PredefinedMenuItem::separator()).map_err(|err| format!("append separator: {err}"))?;
                    pending_separator = false;
                }
                let nested = muda::Submenu::new(title, true);
                append_muda_nodes_filtered(&nested, children, next_items, next_variants, flat_idx)?;
                parent.append(&nested).map_err(|err| format!("append nested submenu `{title}`: {err}"))?;
                emitted_any = true;
            }
            MudaMenuNodeSpec::Action(item) => {
                let current_idx = *flat_idx;
                *flat_idx += 1;
                if MACOS_APP_SUBMENU_OWNED_COMMANDS.contains(&item.command_id.as_str()) {
                    continue;
                }
                if pending_separator {
                    parent.append(&muda::PredefinedMenuItem::separator()).map_err(|err| format!("append separator: {err}"))?;
                    pending_separator = false;
                }
                if item.variants.is_empty() {
                    let entry = muda::MenuItem::with_id(item.menu_id.clone(), &item.label, item.enabled, item.accelerator);
                    parent.append(&entry).map_err(|err| format!("append menu item `{}`: {err}", item.command_id))?;
                    next_items.push((current_idx, entry));
                } else {
                    let nested = muda::Submenu::new(&item.label, true);
                    for (v_label, v_cmd) in &item.variants {
                        let accel =
                            if item.command_id == "ui.canvas.applyPreset" && v_cmd.ends_with("SingleColumn88mm") { item.accelerator } else { None };
                        let v_item = muda::MenuItem::with_id(muda::MenuId::new(v_cmd), v_label, item.enabled, accel);
                        nested.append(&v_item).map_err(|err| format!("append variant `{v_cmd}`: {err}"))?;
                        next_variants.push((item.command_id.clone(), v_item));
                    }
                    parent.append(&nested).map_err(|err| format!("append submenu `{}`: {err}", item.command_id))?;
                }
                emitted_any = true;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_muda_nodes(nodes: &[DesktopMenuNode], flat_items: &mut Vec<MudaMenuItemSpec>) -> Vec<MudaMenuNodeSpec> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            DesktopMenuNode::Action(item) => {
                if is_menu_item_visible(&item.command_id) {
                    let spec = build_muda_item_spec(item);
                    flat_items.push(spec.clone());
                    out.push(MudaMenuNodeSpec::Action(spec));
                }
            }
            DesktopMenuNode::Separator => out.push(MudaMenuNodeSpec::Separator),
            DesktopMenuNode::Submenu { id, title, children } => {
                let child_nodes = build_muda_nodes(children, flat_items);
                if !child_nodes.is_empty() {
                    out.push(MudaMenuNodeSpec::Submenu { id: id.clone(), title: title.clone(), children: child_nodes });
                }
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn build_muda_section_specs(chrome: &DesktopMenuChromeState) -> Vec<MudaMenuSectionSpec> {
    chrome
        .sections
        .iter()
        .map(|sec| {
            let mut items = Vec::new();
            let nodes = if sec.nodes.is_empty() {
                sec.items
                    .iter()
                    .filter(|item| is_menu_item_visible(&item.command_id))
                    .map(|item| {
                        let spec = build_muda_item_spec(item);
                        items.push(spec.clone());
                        MudaMenuNodeSpec::Action(spec)
                    })
                    .collect()
            } else {
                build_muda_nodes(&sec.nodes, &mut items)
            };
            MudaMenuSectionSpec { menu_key: sec.menu_key.clone(), title: sec.title.clone(), nodes, items }
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn build_muda_item_spec(item: &DesktopMenuItem) -> MudaMenuItemSpec {
    let variants = if let Some(list) = submenu_variants_for_command(&item.command_id) {
        list.iter().map(|&(label, val)| (label.to_owned(), format!("{}:{val}", item.command_id))).collect()
    } else if item.command_id == "ui.locale.set" {
        BETA_ACTIVE_LOCALES
            .iter()
            .map(|&loc| {
                let canonical = normalize_locale(loc);
                (format!("{loc} ({canonical})"), format!("ui.locale.set:{canonical}"))
            })
            .collect()
    } else {
        Vec::new()
    };

    MudaMenuItemSpec {
        menu_id: muda::MenuId::new(&item.command_id),
        command_id: item.command_id.clone(),
        label: item.localized_label.clone(),
        shortcut: item.shortcut.clone(),
        accelerator: parse_muda_accelerator(&item.shortcut),
        enabled: effective_menu_item_enabled(&item.command_id, item.enabled),
        variants,
    }
}

#[cfg(target_os = "macos")]
fn push_node_structure_signature(nodes: &[MudaMenuNodeSpec], out: &mut Vec<(String, String, String)>) {
    for node in nodes {
        match node {
            MudaMenuNodeSpec::Action(it) => out.push((it.command_id.clone(), it.label.clone(), it.shortcut.clone())),
            MudaMenuNodeSpec::Separator => out.push((String::from("__separator__"), String::new(), String::new())),
            MudaMenuNodeSpec::Submenu { id, title, children } => {
                out.push((id.clone(), title.clone(), String::from("__submenu__")));
                push_node_structure_signature(children, out);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn compute_structure_signature(locale: &str, specs: &[MudaMenuSectionSpec]) -> MenuStructureSignature {
    let (about_label, prefs_label, quit_label) = localized_app_submenu_labels(locale);
    let sections = specs
        .iter()
        .map(|sec| {
            let mut items = Vec::new();
            push_node_structure_signature(&sec.nodes, &mut items);
            (sec.menu_key.clone(), sec.title.clone(), items)
        })
        .collect();
    ((normalize_locale(locale).to_owned(), about_label.to_owned(), prefs_label.to_owned(), quit_label.to_owned()), sections)
}

#[cfg(target_os = "macos")]
fn compute_enablement_signature(specs: &[MudaMenuSectionSpec]) -> Vec<bool> {
    specs.iter().flat_map(|s| s.items.iter().map(|it| it.enabled)).collect()
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
            locale: String::from("en"),
            sections: vec![DesktopMenuSection {
                menu_key: String::from("menu.file"),
                title: String::from("File"),
                nodes: Vec::new(),
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

        let mut broken_enable = entries.clone();
        broken_enable[0].items[0].enabled = false;
        assert!(verify_menu_parity(&chrome, &broken_enable).is_err());

        let mut broken_shortcut = entries.clone();
        broken_shortcut[0].items[0].shortcut = String::from("Cmd+X");
        assert!(verify_menu_parity(&chrome, &broken_shortcut).is_err());

        let mut broken_label = entries;
        broken_label[0].items[0].label = String::from("Other");
        assert!(verify_menu_parity(&chrome, &broken_label).is_err());
    }

    #[test]
    fn r20_22_macos_window_menu_dot_free_settings_and_app_menu_dedup() -> Result<(), String> {
        use curvebook_ui_egui::i18n::LOCALES;

        assert_eq!(MACOS_WINDOW_SUBMENU_ITEMS, ["Minimize", "Zoom", "Enter Full Screen", "Bring All to Front"]);
        for &loc in LOCALES {
            let (_about, prefs, quit) = localized_app_submenu_labels(loc);
            assert!(!prefs.contains("...") && !prefs.contains('…') && !prefs.contains("Preferences"), "`{loc}` prefs=`{prefs}`");
            assert!(!quit.is_empty());
            assert!(!localized_window_submenu_title(loc).is_empty());
        }
        assert_eq!(localized_app_submenu_labels("en").1, "Settings");
        assert_eq!(localized_window_submenu_title("en"), "Window");

        #[cfg(target_os = "macos")]
        {
            use crate::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
            use curvebook_ui_egui::interaction::ViewportMetrics;
            let mut rt_native =
                DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: false, recovery_root: None, ..DesktopHostConfig::default() });
            let wid_native = rt_native.open_window(None, ViewportMetrics::default())?;
            rt_native.window_mut(wid_native)?.app_mut().host_draws_menu_bar = true;
            let chrome_native = rt_native.window(wid_native)?.chrome_state();
            let mut mac_host = MacNativeMenuHost::new();
            mac_host.sync_from_chrome(&chrome_native)?;
            let section_keys: Vec<&str> = mac_host.muda_section_specs().iter().map(|s| s.menu_key.as_str()).collect();
            assert_eq!(section_keys, ["menu.file", "menu.edit", "menu.view", "menu.insert", "menu.data", "menu.analyze", "menu.graph", "menu.help"]);
            for forbidden in ["app.about", "app.preferences", "app.quit"] {
                assert!(
                    !mac_host.muda_section_specs().iter().flat_map(|s| s.items.iter()).any(|i| i.command_id == forbidden),
                    "macOS top-level menu bar must not duplicate `{forbidden}`"
                );
            }
            for item in mac_host.muda_section_specs().iter().flat_map(|s| s.items.iter()) {
                assert!(
                    !item.label.ends_with("...") && !item.label.ends_with('…'),
                    "R20.15 violation on macOS menu `{}`: `{}`",
                    item.command_id,
                    item.label
                );
            }

            let mut rt_in_win =
                DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, recovery_root: None, ..DesktopHostConfig::default() });
            let wid_in_win = rt_in_win.open_window(None, ViewportMetrics::default())?;
            rt_in_win.window_mut(wid_in_win)?.app_mut().host_draws_menu_bar = false;
            let chrome_in_win = rt_in_win.window(wid_in_win)?.chrome_state();
            let in_win_entries = collect_menu_parity_entries(&chrome_in_win);
            assert!(in_win_entries.iter().find(|s| s.menu_key == "menu.file").is_some_and(|s| s.items.iter().any(|i| i.command_id == "app.quit")));
            assert!(
                in_win_entries
                    .iter()
                    .find(|s| s.menu_key == "menu.edit")
                    .is_some_and(|s| s.items.iter().any(|i| i.command_id == "app.preferences" && i.label == "Settings" && i.shortcut == "Cmd+,"))
            );
            assert!(in_win_entries.iter().find(|s| s.menu_key == "menu.help").is_some_and(|s| s.items.iter().any(|i| i.command_id == "app.about")));
        }
        Ok(())
    }
}
