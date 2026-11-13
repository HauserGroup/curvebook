# Localization & Number Formatting Parity

## Interface Localization (`lumencraft-ui-egui::i18n`)
- [x] Locale registry and `untranslated_report` CI check (`cargo xtask i18n-report`)
- [ ] Complete English (`en`) reference catalog for menus, dialogs, and statistical checklists
- [ ] Tier-1 interface translations (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`)
- [ ] Right-to-left (`ar`, `he`) UI layout & bidirectional text shaping support

## Scientific & Regional Number Conventions
- [ ] Configurable decimal separator (period `12.34` vs. comma `12,34`) independent of OS locale for reproducible journal figures
- [ ] Configurable thousands/grouping separator (none, thin space `12 345`, comma, period)
- [ ] CSV/clipboard import locale presets (`en-US` comma-delimited vs. European semicolon-delimited `;` with decimal comma)
- [ ] Localized *p*-value formatting presets (APA no-leading-zero `.004`, NEJM, Nature/Cell exact $p = 2.4 \times 10^{-5}$, or configurable star thresholds)
