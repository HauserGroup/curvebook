# Localization & Number Formatting Parity

## Interface Localization (`lumencraft-ui-egui::i18n`)
- [x] Locale registry and `untranslated_report` CI check (`cargo xtask i18n-report`)
- [x] Complete English (`en`) reference catalog for menus, dialogs, and statistical checklists (`167` catalog keys verified in `lumencraft-ui-egui::i18n`)
- [x] Tier-1 interface translations (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`) (`9` locales × `167` keys = `1,503` verified entries, `0` missing translations; `ui.locale.set` wired)
- [ ] Right-to-left (`ar`, `he`) UI layout & bidirectional text shaping support *(deferred until a Tier-2 RTL locale joins the supported set)*

## Scientific & Regional Number Conventions
- [x] Configurable decimal separator (period `12.34` vs. comma `12,34`) independent of OS locale for reproducible journal figures (`LocalePreset`, `LocaleNumberPolicy`, `Format > Decimal Separator`)
- [x] Configurable thousands/grouping separator (none, thin space `12 345`, comma, period) (`LocaleNumberPolicy::thousands_separator` across `lumencraft-text`, `lumencraft-format`, and `lumencraft-ui-egui`)
- [x] CSV/clipboard import locale presets (`en-US` comma-delimited vs. European semicolon-delimited `;` with decimal comma) (`CsvOptions::csv()` / `LocalePreset::comma_semicolon`, tested in `grid_view` & `format`)
- [x] Localized *p*-value formatting presets (APA no-leading-zero `.004`, NEJM, Nature/Cell exact $p = 2.4 \times 10^{-5}$, or configurable star thresholds) (`PValueFormat::{FourDecimal, Apa, Nejm, Custom}` + `format_pvalue_label`)
