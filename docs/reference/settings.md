# Settings, shortcuts & languages

CurveBook stores interface preferences and custom keyboard bindings locally in plain versioned JSON files. It uses no cloud accounts or remote syncing.

---

## Preference keys & storage paths {#preferences}

Desktop GUI settings persist in `preferences.json` (schema version 1, maximum size 4 MiB), governed by [`preferences.schema.json`](../schemas/preferences.schema.json):

| Key | JSON type | Allowed values |
|---|---|---|
| `version` | integer (`0` or `1`) | Schema version (`1` written by current releases). |
| `locale` | string \| `null` | BCP-47 UI locale code (e.g., `"en"`, `"de"`, `"pt-BR"`, `"ar"`). |
| `theme` | string \| `null` | Desktop theme: `"light"`, `"dark"`, or `"high-contrast"`. |
| `shortcuts` | object \| `null` | Map of command ID to key combo (e.g., `{"ui.gallery.toggle": "Ctrl+Alt+G"}`). |

Preference file behavior:

- Omitting `locale` or setting it to `null` defaults to `"en"`.
- Entries in `shortcuts` merge on top of CurveBook's built-in defaults.
- Files with `version > 1` are refused rather than overwritten, while unknown extension keys are preserved on load and upgrade.
- If `preferences.json` or `shortcuts.json` is corrupt, CurveBook prints a warning to `stderr` and starts cleanly with built-in defaults.

### Storage paths & environment overrides {#storage-paths}

Unless overridden, `curvebook` resolves configuration (`preferences.json`, `shortcuts.json`) and recovery (`recovery/`) directories in this priority order:

1. `--portable-dir <dir>` CLI flag → `<dir>/curvebook-data/`
2. `CURVEBOOK_CONFIG_DIR` environment variable → `<CURVEBOOK_CONFIG_DIR>/`
3. A `portable.txt` marker file beside the `curvebook` binary → `<exe_dir>/curvebook-data/`
4. **Platform standard directories:**
   - **macOS:** `~/Library/Application Support/CurveBook/` (both preferences and recovery).
   - **Windows:** `%APPDATA%\CurveBook\` (preferences and shortcuts) and `%LOCALAPPDATA%\CurveBook\` (recovery data).
   - **Linux & FreeBSD:** `$XDG_CONFIG_HOME/curvebook/` (fallback `~/.config/curvebook/`) and `$XDG_DATA_HOME/curvebook/` (fallback `~/.local/share/curvebook/`). When `$HOME` is unset, CurveBook validates a strict UID-owned `0700` fallback directory.

Set `CURVEBOOK_NO_PREFS=1` (or `true`, `yes`, `on`) to run statelessly without reading or writing preferences, shortcuts, or recovery journals.

---

## Keyboard shortcut customization {#keyboard-shortcuts}

All engine commands and interactive desktop actions support customizable keyboard shortcuts:

- **Context scopes:** Bindings use global, results-sheet, or non-results-sheet scopes. Context-sensitive shortcuts—such as **Cmd/Ctrl+T** editing analysis parameters on a results sheet versus creating a data table elsewhere—coexist without false conflicts.
- **Saving and conflict checks:** Custom bindings save atomically to `preferences.json` (under `shortcuts`) and `shortcuts.json`. CurveBook rejects conflicts across overlapping scopes and supports resetting individual shortcuts or restoring all defaults.

---

## UI locales & Beta translation status (`[Draft]`) {#locales}

CurveBook provides complete UI string coverage across all supported BCP-47 locales with zero missing keys (verified by `cargo xtask i18n-report`):

| Region / group | Locale codes | Script direction | Beta review status |
|---|---|---|---|
| **English** | `en` | LTR | Primary reviewed source (`English (en)`) |
| **Tier-1 scientific** | `de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW` | LTR | Marked **`[Draft]`** pending native scientific review |
| **Tier-2 European** | `it`, `nl`, `pl`, `ru`, `tr` | LTR | Marked **`[Draft]`** pending native scientific review |
| **Right-to-left (RTL)** | `ar`, `he` | RTL | Catalog and PDF/SVG BiDi ready; GUI picker deferred to 1.0 |

- **`[Draft]` indicator:** Every non-English entry in the locale picker displays `[Draft]` (`<Autonym> (<code>) [Draft]`) to show that machine-drafted translations await native scientific review.
- **Locale normalization:** Regional variants map cleanly to supported tags (such as `en-US` → `en`, `zh-Hans` → `zh-CN`, `zh-Hant` → `zh-TW`, `pt` → `pt-BR`). See [Localization Architecture](../localization.md).

---

## Right-to-left (RTL) & bidirectional text {#rtl-mirroring}

Full desktop GUI layout mirroring for Arabic (`ar`) and Hebrew (`he`) is deferred to the 1.0 release. In Beta, **View › Language** lists only active LTR and CJK locales, and loading an RTL tag from `preferences.json` falls back cleanly to `"en"`.

Unicode bidirectional visual run reordering stays active in Beta for multi-script figure labels and PDF/SVG exports. The `ar` and `he` translation catalogs and horizontal layout mirroring support remain built in ahead of 1.0 enablement.

---

## Locale number, tick & delimiter formatting {#locale-formatting}

UI locale selection updates display formatting and CSV defaults without altering stored numeric values or calculations:

- **Decimal and delimiter defaults:** Continental European locales default to `,` decimal display and `;`-delimited CSV presets. English and CJK locales default to `.` decimals and `,`-delimited CSVs.
- **Statistical precision:** Every transform, hypothesis test, nonlinear solver, and random number generator computes in IEEE-754 double precision regardless of the active display locale.
