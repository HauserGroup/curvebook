# Localization & Number Formatting Parity

## Interface Localization (`lumencraft-ui-egui::i18n`)
- [x] Locale registry and `untranslated_report` CI check (`cargo xtask i18n-report`)
- [x] Complete English (`en`) reference catalog for menus, dialogs, and statistical checklists (`167` catalog keys verified in `lumencraft-ui-egui::i18n`)
- [x] Tier-1, Tier-2, and RTL interface translations (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`, `it`, `nl`, `pl`, `ru`, `tr`, `ar`, `he`, plus `en`) (`16` locales × `167` keys = `2,672` verified entries, `0` missing translations via `cargo xtask i18n-report`; `ui.locale.set` wired; `font_diagnostics` + `system_cjk_font_candidates` inspect host external `.ttf` CJK/BiDi font availability)
- [x] Right-to-left (`ar`, `he`) UI layout & bidirectional text shaping support *(BiDi text reordering via `reorder_rich_text_bidi` / `contains_rtl_script` in `lumencraft-text` & `lumencraft-pdf` plus RTL UI layout mirroring `is_rtl_locale` / `mirror_x_for_rtl` verified across `ui-egui` compositor PNG tests; live OS desktop RTL/CJK viewer QA remains in `G13`)*

## Scientific & Regional Number Conventions
- [x] Configurable decimal separator (period `12.34` vs. comma `12,34`) independent of OS locale for reproducible journal figures (`LocalePreset`, `LocaleNumberPolicy`, `Format > Decimal Separator`, and `grid_view::parse_localized_number_input` preserving `=` formulas)
- [x] Configurable thousands/grouping separator (none, thin space `12 345`, comma, period) (`LocaleNumberPolicy::thousands_separator` across `lumencraft-text`, `lumencraft-format`, and `lumencraft-ui-egui`)
- [x] CSV/clipboard import locale presets (`en-US` comma-delimited vs. European semicolon-delimited `;` with decimal comma) (`CsvOptions::csv()` / `LocalePreset::comma_semicolon`, tested in `grid_view` & `format`)
- [x] Localized *p*-value formatting presets (APA no-leading-zero `.004`, NEJM, Nature/Cell exact $p = 2.4 \times 10^{-5}$, or configurable star thresholds) (`PValueFormat::{FourDecimal, Apa, Nejm, Custom}` + `format_pvalue_label`)
