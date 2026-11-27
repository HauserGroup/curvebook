# Settings, shortcuts and languages

<!-- Audience: Tier 2 Technical Reference — complete preferences schema keys, storage paths, shortcut scopes, and locale rules belong here; keep get-started.md and desktop-gui.md concise for scientists. -->

CurveBook stores interface preferences and custom keyboard bindings locally, in plain versioned JSON files. There are no cloud accounts and no remote syncing. This reference lists the exact JSON keys, operating-system storage paths, shortcut scopes, and locale rules. For installation or everyday workspace controls, start with [Install CurveBook](get-started.md) or the [Desktop GUI overview](desktop-gui.md).

---

## Preference keys and storage paths {#preferences}

The desktop app saves its settings in `preferences.json` (schema version 1, maximum size 4 MiB). The file follows [`preferences.schema.json`](../schemas/preferences.schema.json):

| Key | JSON type | Allowed values |
|---|---|---|
| `version` | integer (`0` or `1`) | Schema version. Current releases write `1`. |
| `locale` | string \| `null` | BCP-47 UI locale code (e.g., `"en"`, `"de"`, `"pt-BR"`, `"ar"`). |
| `theme` | string \| `null` | Desktop theme: `"light"`, `"dark"`, or `"high-contrast"`. |
| `shortcuts` | object \| `null` | Map from command ID to key combination (e.g., `{"ui.gallery.toggle": "Ctrl+Alt+G"}`). |

How CurveBook handles the preference file:

- If `locale` is missing or `null`, CurveBook uses `"en"`.
- Entries in `shortcuts` are merged on top of CurveBook's built-in defaults.
- CurveBook refuses to overwrite a file with `version > 1`. Unknown extension keys are preserved when the file is loaded and upgraded.
- If `preferences.json` or `shortcuts.json` is corrupt, CurveBook prints a warning to `stderr` and starts with the built-in defaults.

### Storage paths and environment overrides {#storage-paths}

`curvebook` chooses the directories for configuration (`preferences.json`, `shortcuts.json`) and recovery data (`recovery/`) in this order of priority:

1. `--portable-dir <dir>` CLI flag → `<dir>/curvebook-data/`
2. `CURVEBOOK_CONFIG_DIR` environment variable → `<CURVEBOOK_CONFIG_DIR>/`
3. A `portable.txt` marker file next to the `curvebook` binary → `<exe_dir>/curvebook-data/`
4. Standard platform directories:
   - **macOS:** `~/Library/Application Support/CurveBook/` for both preferences and recovery data.
   - **Windows:** `%APPDATA%\CurveBook\` for preferences and shortcuts, and `%LOCALAPPDATA%\CurveBook\` for recovery data.
   - **Linux and FreeBSD:** `$XDG_CONFIG_HOME/curvebook/` (fallback `~/.config/curvebook/`) and `$XDG_DATA_HOME/curvebook/` (fallback `~/.local/share/curvebook/`). When `$HOME` is not set, CurveBook validates a fallback directory that must be owned by the current UID with `0700` permissions.

Set `CURVEBOOK_NO_PREFS=1` (or `true`, `yes`, `on`) to run without saved state. CurveBook then neither reads nor writes preferences, shortcuts, or recovery journals.

Optional desktop launch flags (see `curvebook --help`) include `--portable-dir <dir>`, `--rulers`, `--in-window-menus`, `--ttf-font <path>`, and `--new-instance`.

---

## Keyboard shortcut customization {#keyboard-shortcuts}

You can assign your own keyboard shortcuts to commands. Bindings follow these rules:

- **Context scopes:** Each binding applies globally, only on results sheets, or only on other sheets. Shortcuts that depend on context can share a key without being reported as conflicts. For example, **Cmd/Ctrl+T** edits analysis parameters on a results sheet and creates a data table elsewhere.
- **Saving and conflict checks:** CurveBook saves custom bindings atomically to `preferences.json` (under `shortcuts`) and to `shortcuts.json`. It rejects a binding that conflicts with another binding in an overlapping scope. You can reset individual shortcuts or restore all defaults.

---

## UI locales and Beta translation status (`[Draft]`) {#locales}

Every supported BCP-47 locale has complete UI string coverage, with no missing keys (verified by `cargo xtask i18n-report`):

| Region or group | Locale codes | Script direction | Beta review status |
|---|---|---|---|
| English | `en` | LTR | Primary reviewed source (`English (en)`) |
| Tier-1 scientific | `de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW` | LTR | Marked `[Draft]` until native scientific review |
| Tier-2 European | `it`, `nl`, `pl`, `ru`, `tr` | LTR | Marked `[Draft]` until native scientific review |
| Right-to-left (RTL) | `ar`, `he` | RTL | Catalog and PDF/SVG BiDi ready; GUI picker deferred to 1.0 |

- **`[Draft]` indicator:** Every non-English entry in the locale picker ends with `[Draft]` (`<Autonym> (<code>) [Draft]`). The tag shows that the translation was machine-drafted and is waiting for native scientific review.
- **Locale normalization:** Regional variants map to supported tags, for example `en-US` → `en`, `zh-Hans` → `zh-CN`, `zh-Hant` → `zh-TW`, and `pt` → `pt-BR`. See [Localization Architecture](../localization.md).

---

## Right-to-left (RTL) and bidirectional text {#rtl-mirroring}

Full mirroring of the desktop GUI layout for Arabic (`ar`) and Hebrew (`he`) is deferred to the 1.0 release. In the Beta, **View › Language** lists only the active LTR and CJK locales. If `preferences.json` contains an RTL tag, CurveBook falls back to `"en"`.

Unicode bidirectional reordering of visual runs stays active in the Beta for multi-script figure labels and for PDF and SVG exports. The `ar` and `he` translation catalogs and support for horizontal layout mirroring are already built in, ahead of their enablement in 1.0.

---

## Locale number, tick and delimiter formatting {#locale-formatting}

The UI locale changes how numbers are displayed and which CSV defaults apply. It does not change stored numeric values or calculations:

- **Decimal and delimiter defaults:** Continental European locales default to a `,` decimal separator and `;`-delimited CSV presets. English and CJK locales default to `.` decimals and `,`-delimited CSV.
- **Statistical precision:** Every transform, hypothesis test, nonlinear solver, and random number generator computes in IEEE-754 double precision, whatever the display locale.
