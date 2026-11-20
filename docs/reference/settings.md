# Settings, Shortcuts & Languages

CurveBook stores interface preferences and custom keyboard bindings locally in plain versioned JSON files with zero remote syncing or cloud accounts.

---

## Preference Keys & Storage Paths {#preferences}

Desktop GUI settings persist in `preferences.json`, governed by [`docs/schemas/preferences.schema.json`](../schemas/preferences.schema.json) (`CURRENT_PREFERENCES_VERSION = 1`, `MAX_PREFERENCES_FILE_BYTES = 4 MiB`):

| Key | JSON Type | Allowed Values / Meaning |
|---|---|---|
| `version` | integer (`0..=1`) | Schema version (`1` written by current CurveBook; files with `version > 1` are refused rather than overwritten). |
| `locale` | string \| `null` | BCP-47 UI locale code (e.g., `"en"`, `"de"`, `"pt-BR"`, `"ar"`). Defaults to `"en"` when omitted or `null`. |
| `theme` | string \| `null` | Desktop theme mode: `"light"`, `"dark"`, or `"high-contrast"`. |
| `shortcuts` | object \| `null` | Map of `command_id` -> key-combo string (e.g., `{"ui.gallery.toggle": "Ctrl+Alt+G"}`), merged atop default bindings. |

Forward-compatible extension keys (`additionalProperties: true`) are preserved on deserialize/upgrade; corrupt `preferences.json` or `shortcuts.json` files emit a stderr warning and fall back cleanly to built-in defaults rather than blocking startup.

### Storage Paths & Environment Overrides {#storage-paths}

By default (unless overridden), `curvebook` resolves configuration (`preferences.json`, `shortcuts.json`) and recovery (`recovery/`) paths (`apps/curvebook/src/main.rs`) in priority order:

1. `--portable-dir <dir>` CLI flag -> `<dir>/curvebook-data/`
2. `CURVEBOOK_CONFIG_DIR` environment variable -> `<CURVEBOOK_CONFIG_DIR>/`
3. Portable marker file `portable.txt` located beside the `curvebook` binary -> `<exe_dir>/curvebook-data/`
4. **Platform Standard Directories:**
   - **macOS:** `~/Library/Application Support/CurveBook/` (both preferences and recovery)
   - **Windows:** `%APPDATA%\CurveBook\` (preferences/shortcuts) and `%LOCALAPPDATA%\CurveBook\` (recovery data)
   - **Linux & FreeBSD:** `$XDG_CONFIG_HOME/curvebook/` (fallback `~/.config/curvebook/`) and `$XDG_DATA_HOME/curvebook/` (fallback `~/.local/share/curvebook/`), with strict UID `0700` fallback validation when `$HOME` is unset.

Setting `CURVEBOOK_NO_PREFS=1` (or `true`/`yes`/`on`) runs in stateless mode without loading or writing preferences, shortcuts, or recovery journals.

---

## Keyboard Shortcut Customization {#keyboard-shortcuts}

All engine commands (`cmd::registry()`) and interactive UI commands (`UI_COMMANDS`) register customizable bindings in `ShortcutRegistry` (`crates/ui-egui/src/menus/shortcuts.rs`):

- **Context Scoping (`ShortcutScope`):** Bindings declare `Global`, `ResultSheet`, or `NonResultSheet` scope so context-sensitive shortcuts (such as `Cmd/Ctrl+T` switching parameters on a `ResultSheet` vs. creating a data table elsewhere) coexist without false conflicts.
- **Persistence & Conflict Detection:** Overrides save atomically to `preferences.json` (`shortcuts` map) and `shortcuts.json` (`curves` compatibility mirror), enforce conflict detection across overlapping `ShortcutScope`s, and can be reset individually or wholesale to defaults (`shortcuts.reset_all()`).

---

## UI Locales & Beta Translation Status (`[Draft]`) {#locales}

CurveBook's i18n catalog (`crates/ui-egui/src/i18n.rs`) provides full key coverage across every supported BCP-47 locale (`LOCALES`, verified at zero missing keys via `cargo xtask i18n-report`):

| Region / Group | Locale Codes | Script Direction | Beta Review Status |
|---|---|---|---|
| **English** | `en` | LTR | Primary reviewed source (`English (en)`) |
| **Tier-1 Scientific** | `de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW` | LTR | Marked **`[Draft]`** in Beta pending native-speaker scientific review |
| **Tier-2 European** | `it`, `nl`, `pl`, `ru`, `tr` | LTR | Marked **`[Draft]`** in Beta pending native-speaker scientific review |
| **Right-to-Left (RTL)** | `ar`, `he` | RTL | Catalog & PDF/SVG BiDi ready; desktop GUI picker deferred to `1.0` (`normalize_beta_gui_locale` falls back to `"en"` in Beta) |

- **`[Draft]` Beta indicator:** Every non-English entry in the UI locale picker appends `[Draft]` (`<Autonym> (<code>) [Draft]`) to indicate machine-drafted translations awaiting community/native review.
- **Locale normalization (`normalize_locale`):** Regional variants normalize cleanly onto supported tags (e.g., `en-US` -> `en`, `zh-Hans` -> `zh-CN`, `zh-Hant` -> `zh-TW`, `pt` -> `pt-BR`). See [Localization Architecture](../localization.md).

---

## Right-to-Left (RTL) Mirroring {#rtl-mirroring}

Selecting `ar` or `he` (`is_rtl_locale == true`) enables complete horizontal UI layout mirroring (`mirror_x_for_rtl`) across the Project Navigator, toolbar/menu chrome, spreadsheet gutter & columns, dialog steppers, and hit-test coordinates (`crates/ui-egui`), paired with Unicode BiDi visual run reordering (`reorder_visual_bidi` in `curvebook-text`) for multi-script figure labels and PDF exports.

---

## Locale Number, Tick & Delimiter Formatting {#locale-formatting}

UI locale selection integrates with `LocaleNumberPolicy` (`crates/text/src/number.rs`) and `CsvOptions` defaults without altering stored `f64` values or numerical computations:

- **Decimal & Thousands Formatting:** Continental European locales default cleanly to `,` decimal display and `;`-delimited regional CSV presets (`CsvOptions::european_csv()`), while English/CJK locales default to `.` decimals and `,`-delimited CSVs.
- **Statistical Precision:** Internally, every transform, statistical hypothesis test, nonlinear solver, and Monte Carlo RNG evaluates over IEEE-754 double-precision values independent of active display locale.
