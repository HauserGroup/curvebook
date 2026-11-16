//! Customizable keyboard shortcut registry seeded from engine `cmd::registry()` and `UI_COMMANDS`.

use std::collections::BTreeMap;

use curvebook_engine::cmd::registry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::menus::ui_commands::UI_COMMANDS;

/// A single command's default and currently active keyboard shortcut binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub command_id: String,
    pub label: String,
    pub default_combo: String,
    pub active_combo: String,
    pub is_ui: bool,
}

impl ShortcutBinding {
    /// Returns `true` when the active shortcut differs from the built-in default.
    #[must_use]
    pub fn is_customized(&self) -> bool {
        self.active_combo != self.default_combo
    }
}

/// Matched command and parameter payload produced by [`ShortcutRegistry::handle_key_combo`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutMatch {
    pub command_id: String,
    pub params: Value,
}

/// Execution summary returned by [`crate::UiApp::handle_key_combo`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutExecution {
    pub command_id: String,
    pub ui: bool,
    pub result: Value,
}

/// Registry mapping stable command IDs to customizable key combinations with conflict detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutRegistry {
    bindings: BTreeMap<String, ShortcutBinding>,
}

impl Default for ShortcutRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ShortcutRegistry {
    /// Seed the shortcut registry from engine [`curvebook_engine::cmd::registry`] and front-end [`UI_COMMANDS`].
    #[must_use]
    pub fn new() -> Self {
        let mut bindings = BTreeMap::new();
        for spec in registry().specs() {
            let normalized = normalize_shortcut_combo(spec.shortcut).unwrap_or_else(|_| spec.shortcut.trim().to_owned());
            bindings.insert(
                spec.id.to_owned(),
                ShortcutBinding {
                    command_id: spec.id.to_owned(),
                    label: spec.label.to_owned(),
                    default_combo: normalized.clone(),
                    active_combo: normalized,
                    is_ui: false,
                },
            );
        }
        for spec in UI_COMMANDS {
            let normalized = normalize_shortcut_combo(spec.shortcut).unwrap_or_else(|_| spec.shortcut.trim().to_owned());
            bindings.insert(
                spec.id.to_owned(),
                ShortcutBinding {
                    command_id: spec.id.to_owned(),
                    label: spec.label.to_owned(),
                    default_combo: normalized.clone(),
                    active_combo: normalized,
                    is_ui: true,
                },
            );
        }
        Self { bindings }
    }

    /// Number of commands tracked in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether the registry contains zero bindings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Look up the [`ShortcutBinding`] for `command_id`.
    #[must_use]
    pub fn get(&self, command_id: &str) -> Option<&ShortcutBinding> {
        self.bindings.get(command_id)
    }

    /// Return the currently active key combination string for `command_id` (empty if unbound).
    #[must_use]
    pub fn active_combo(&self, command_id: &str) -> Option<&str> {
        self.bindings.get(command_id).map(|b| b.active_combo.as_str())
    }

    /// Iterate over all registered bindings in deterministic command-ID order.
    pub fn bindings(&self) -> impl Iterator<Item = &ShortcutBinding> {
        self.bindings.values()
    }

    /// Return the conflicting `command_id` if `combo` is already assigned to another command.
    #[must_use]
    pub fn find_conflict(&self, combo: &str, exclude_command_id: Option<&str>) -> Option<&str> {
        let Ok(canonical) = normalize_shortcut_combo(combo) else {
            return None;
        };
        if canonical.is_empty() {
            return None;
        }
        self.bindings
            .values()
            .find(|b| Some(b.command_id.as_str()) != exclude_command_id && b.active_combo == canonical)
            .map(|b| b.command_id.as_str())
    }

    /// Assign a custom key combination `combo` (or `""` to unbind) to `command_id`.
    ///
    /// # Errors
    /// Returns an error when `command_id` is unknown, `combo` has invalid syntax, or another
    /// command already owns the same normalized key combination.
    pub fn set_shortcut(&mut self, command_id: &str, combo: &str) -> Result<(), String> {
        if !self.bindings.contains_key(command_id) {
            return Err(format!("unknown command `{command_id}`"));
        }
        let canonical = normalize_shortcut_combo(combo)?;
        if let Some(conflict_id) = self.find_conflict(&canonical, Some(command_id)) {
            return Err(format!("shortcut `{canonical}` conflicts with command `{conflict_id}`"));
        }
        if let Some(binding) = self.bindings.get_mut(command_id) {
            binding.active_combo = canonical;
        }
        Ok(())
    }

    /// Restore `command_id` back to its built-in default shortcut.
    ///
    /// # Errors
    /// Returns an error if `command_id` is unknown or its default shortcut is currently bound to
    /// another customized command.
    pub fn reset_shortcut(&mut self, command_id: &str) -> Result<(), String> {
        let default_combo =
            self.bindings.get(command_id).map(|b| b.default_combo.clone()).ok_or_else(|| format!("unknown command `{command_id}`"))?;
        if let Some(conflict_id) = self.find_conflict(&default_combo, Some(command_id)) {
            return Err(format!("cannot reset `{command_id}` to `{default_combo}`: conflicts with `{conflict_id}`"));
        }
        if let Some(binding) = self.bindings.get_mut(command_id) {
            binding.active_combo = default_combo;
        }
        Ok(())
    }

    /// Reset all shortcuts across the registry back to their built-in defaults.
    pub fn reset_all(&mut self) {
        for binding in self.bindings.values_mut() {
            binding.active_combo = binding.default_combo.clone();
        }
    }

    /// Export customized shortcut overrides (excluding unchanged defaults) as JSON.
    ///
    /// # Errors
    /// Returns an error if JSON serialization fails.
    pub fn export_overrides_json(&self) -> Result<String, String> {
        let overrides: BTreeMap<&str, &str> =
            self.bindings.values().filter(|b| b.is_customized()).map(|b| (b.command_id.as_str(), b.active_combo.as_str())).collect();
        serde_json::to_string_pretty(&overrides).map_err(|err| format!("failed to serialize shortcut overrides: {err}"))
    }

    /// Atomically import customized shortcut overrides from JSON (`{"cmd": "Combo"}` or
    /// `{"overrides": {"cmd": "Combo"}}`).
    ///
    /// Unknown commands are ignored with a warning on stderr so user files across versions stay
    /// loadable, while multi-command shortcut swaps apply cleanly without false conflicts.
    ///
    /// # Errors
    /// Returns an error without mutating `self` if the JSON is invalid, contains non-string or
    /// malformed shortcut values, or introduces duplicate conflicts among bindings.
    pub fn import_overrides_json(&mut self, raw_json: &str) -> Result<usize, String> {
        let parsed: Value = serde_json::from_str(raw_json).map_err(|err| format!("invalid shortcut overrides JSON: {err}"))?;
        let Some(root_obj) = parsed.as_object() else {
            return Err(String::from("shortcut overrides JSON must be an object"));
        };
        let overrides_map = match root_obj.get("overrides") {
            Some(Value::Object(inner)) if root_obj.len() == 1 || !self.bindings.contains_key("overrides") => inner,
            Some(other) if !self.bindings.contains_key("overrides") => {
                return Err(format!("shortcut overrides 'overrides' field must be an object, got {other}"));
            }
            _ => root_obj,
        };

        let mut parsed_entries = Vec::with_capacity(overrides_map.len());
        for (command_id, combo_val) in overrides_map {
            let Some(combo) = combo_val.as_str() else {
                return Err(format!("shortcut override for `{command_id}` must be a string"));
            };
            if self.bindings.contains_key(command_id) {
                parsed_entries.push((command_id.as_str(), combo));
            } else {
                eprintln!("warning: ignoring shortcut override for unknown command `{command_id}`");
            }
        }

        let mut staged = self.clone();
        staged.reset_all();
        // Clear existing bindings for overridden commands first so shortcut swaps succeed cleanly.
        for &(command_id, _) in &parsed_entries {
            if let Some(binding) = staged.bindings.get_mut(command_id) {
                binding.active_combo.clear();
            }
        }

        let mut applied = 0usize;
        for &(command_id, combo) in &parsed_entries {
            staged.set_shortcut(command_id, combo)?;
            applied += 1;
        }

        *self = staged;
        Ok(applied)
    }

    /// Resolve a keyboard combination (such as `"F6"`, `"Shift+F6"`, `"Cmd+K"`, or a custom
    /// binding) into its target command ID and parameters.
    #[must_use]
    pub fn handle_key_combo(&self, combo: &str) -> Option<ShortcutMatch> {
        let canonical = normalize_shortcut_combo(combo).ok()?;
        if canonical.is_empty() {
            return None;
        }

        if let Some(binding) = self.bindings.values().find(|b| b.active_combo == canonical) {
            let params = if binding.command_id == "ui.pane.cycle" { json!({ "reverse": false }) } else { json!({}) };
            return Some(ShortcutMatch { command_id: binding.command_id.clone(), params });
        }

        if let Some(pane_binding) = self.bindings.get("ui.pane.cycle")
            && !pane_binding.active_combo.is_empty()
            && let Some(shifted) = with_shift_modifier(&pane_binding.active_combo)
            && canonical == shifted
        {
            return Some(ShortcutMatch { command_id: String::from("ui.pane.cycle"), params: json!({ "reverse": true }) });
        }

        None
    }
}

/// Normalize a shortcut string into canonical modifier-ordered form (`Cmd+Ctrl+Alt+Shift+<Key>`).
///
/// An empty or whitespace-only string normalizes to `""` (unbound).
///
/// # Errors
/// Returns an error if the shortcut contains empty segments, duplicate or unknown modifiers
/// without a primary key, or multiple primary keys.
pub fn normalize_shortcut_combo(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut has_cmd = false;
    let mut has_ctrl = false;
    let mut has_alt = false;
    let mut has_shift = false;
    let mut primary_key: Option<String> = None;

    for part in trimmed.split('+') {
        let token = part.trim();
        if token.is_empty() {
            return Err(format!("invalid shortcut `{raw}`: empty key token"));
        }
        match token.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" => has_cmd = true,
            "ctrl" | "control" => has_ctrl = true,
            "alt" | "option" | "opt" => has_alt = true,
            "shift" => has_shift = true,
            _ => {
                if primary_key.is_some() {
                    return Err(format!("invalid shortcut `{raw}`: multiple primary keys"));
                }
                primary_key = Some(canonical_key_token(token)?);
            }
        }
    }

    let Some(key) = primary_key else {
        return Err(format!("invalid shortcut `{raw}`: missing non-modifier key"));
    };

    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if has_cmd {
        parts.push("Cmd");
    }
    if has_ctrl {
        parts.push("Ctrl");
    }
    if has_alt {
        parts.push("Alt");
    }
    if has_shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Ok(parts.join("+"))
}

fn canonical_key_token(token: &str) -> Result<String, String> {
    if token.chars().any(char::is_whitespace) {
        return Err(format!("invalid key token `{token}` containing whitespace"));
    }
    let lower = token.to_ascii_lowercase();
    let named = match lower.as_str() {
        "enter" | "return" => Some("Enter"),
        "esc" | "escape" => Some("Escape"),
        "space" | "spacebar" => Some("Space"),
        "tab" => Some("Tab"),
        "backspace" => Some("Backspace"),
        "del" | "delete" => Some("Delete"),
        "ins" | "insert" => Some("Insert"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" | "pgup" => Some("PageUp"),
        "pagedown" | "pgdn" => Some("PageDown"),
        "up" | "arrowup" => Some("Up"),
        "down" | "arrowdown" => Some("Down"),
        "left" | "arrowleft" => Some("Left"),
        "right" | "arrowright" => Some("Right"),
        _ => None,
    };
    if let Some(canon) = named {
        return Ok(canon.to_owned());
    }
    if let Some(rest) = lower.strip_prefix('f')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
        && let Ok(num) = rest.parse::<u8>()
        && (1..=24).contains(&num)
    {
        return Ok(format!("F{num}"));
    }
    if token.len() == 1 {
        return Ok(token.to_ascii_uppercase());
    }
    Ok(token.to_owned())
}

fn with_shift_modifier(canonical_combo: &str) -> Option<String> {
    if canonical_combo.split('+').any(|p| p == "Shift") {
        return None;
    }
    normalize_shortcut_combo(&format!("Shift+{canonical_combo}")).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ShortcutRegistry, normalize_shortcut_combo};

    #[test]
    fn registry_seeds_all_defaults_without_conflicts() {
        let reg = ShortcutRegistry::new();
        assert!(!reg.is_empty());
        assert_eq!(reg.active_combo("ui.palette.open"), Some("Cmd+K"));
        assert_eq!(reg.active_combo("ui.pane.cycle"), Some("F6"));
        assert_eq!(reg.active_combo("file.new"), Some("Cmd+N"));

        for binding in reg.bindings() {
            assert!(!binding.is_customized());
            if !binding.active_combo.is_empty() {
                assert_eq!(reg.find_conflict(&binding.active_combo, Some(&binding.command_id)), None);
            }
        }
    }

    #[test]
    fn shortcut_customization_conflict_reset_and_json_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let mut reg = ShortcutRegistry::new();

        // Conflict detection on set_shortcut
        assert!(reg.set_shortcut("ui.gallery.toggle", "Cmd+K").is_err());
        assert!(reg.set_shortcut("ui.gallery.toggle", "Shift+").is_err());
        assert!(reg.set_shortcut("nonexistent.cmd", "Ctrl+P").is_err());

        reg.set_shortcut("ui.gallery.toggle", "ctrl + shift + g")?;
        assert_eq!(reg.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+G"));
        assert!(reg.get("ui.gallery.toggle").is_some_and(super::ShortcutBinding::is_customized));

        let exported = reg.export_overrides_json()?;
        let mut restored = ShortcutRegistry::new();
        let count = restored.import_overrides_json(&exported)?;
        assert_eq!(count, 1);
        assert_eq!(restored.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+G"));

        // Invalid JSON import does not mutate registry
        assert!(restored.import_overrides_json("{\"ui.gallery.toggle\":\"Cmd+K\"}").is_err());
        assert_eq!(restored.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+G"));

        restored.reset_shortcut("ui.gallery.toggle")?;
        assert_eq!(restored.active_combo("ui.gallery.toggle"), Some(""));

        // F6 and Shift+F6 pane cycling + custom binding dispatch
        assert_eq!(reg.handle_key_combo("f6").map(|m| (m.command_id, m.params)), Some((String::from("ui.pane.cycle"), json!({"reverse": false}))));
        assert_eq!(
            reg.handle_key_combo("shift+f6").map(|m| (m.command_id, m.params)),
            Some((String::from("ui.pane.cycle"), json!({"reverse": true})))
        );
        assert_eq!(reg.handle_key_combo("Ctrl+Shift+G").map(|m| m.command_id), Some(String::from("ui.gallery.toggle")));

        reg.reset_all();
        assert_eq!(reg.active_combo("ui.gallery.toggle"), Some(""));
        assert_eq!(normalize_shortcut_combo("shift+cmd+z")?, "Cmd+Shift+Z");
        Ok(())
    }

    #[test]
    fn r1_s11_05_import_overrides_json_handles_swaps_and_unknown_commands() -> Result<(), Box<dyn std::error::Error>> {
        let mut reg = ShortcutRegistry::new();

        // Direct two-way swap between built-in bindings must succeed atomically without conflict.
        let swapped = reg.import_overrides_json(r#"{"ui.palette.open":"F6","ui.pane.cycle":"Cmd+K"}"#)?;
        assert_eq!(swapped, 2);
        assert_eq!(reg.active_combo("ui.palette.open"), Some("F6"));
        assert_eq!(reg.active_combo("ui.pane.cycle"), Some("Cmd+K"));
        assert_eq!(reg.handle_key_combo("f6").map(|m| m.command_id), Some(String::from("ui.palette.open")));
        assert_eq!(reg.handle_key_combo("cmd+k").map(|m| (m.command_id, m.params)), Some((String::from("ui.pane.cycle"), json!({"reverse": false}))));
        assert_eq!(
            reg.handle_key_combo("shift+cmd+k").map(|m| (m.command_id, m.params)),
            Some((String::from("ui.pane.cycle"), json!({"reverse": true})))
        );

        // Unknown command entries must warn and skip instead of failing load, while valid entries apply.
        let with_unknown = reg.import_overrides_json(r#"{"future.unknown.action":"Ctrl+Alt+9","ui.gallery.toggle":"Ctrl+Shift+G"}"#)?;
        assert_eq!(with_unknown, 1);
        assert_eq!(reg.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+G"));
        assert_eq!(reg.active_combo("ui.palette.open"), Some("Cmd+K"));

        // Real duplicate bindings within the same overrides payload must fail atomically.
        assert!(reg.import_overrides_json(r#"{"ui.palette.open":"F6","ui.gallery.toggle":"F6"}"#).is_err());
        assert_eq!(reg.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+G"));
        Ok(())
    }
}
