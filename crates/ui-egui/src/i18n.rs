//! Interface localization catalog registry and regional number/delimiter presets for CurveBook.
//!
//! Supports English (`en`), eight Tier-1 scientific locales (`de`, `es`, `fr`, `ja`, `ko`,
//! `pt-BR`, `zh-CN`, `zh-TW`), five Tier-2 European locales (`it`, `nl`, `pl`, `ru`, `tr`), and
//! two right-to-left (RTL) locales (`ar`, `he`), integrating with
//! [`curvebook_text::LocaleNumberPolicy`] and [`curvebook_format::CsvOptions`].

use std::borrow::Cow;

use curvebook_doc::{SheetSection, TableKind};
use curvebook_format::CsvOptions;
use curvebook_math::{AxisScale, PValueFormat};
use curvebook_text::{LocaleNumberPolicy, format_axis_tick, format_pvalue_label};
use serde::{Deserialize, Serialize};

/// Supported BCP-47 interface locale identifiers (`en` + 8 Tier-1 + 5 Tier-2 European + 2 RTL).
pub const LOCALES: &[&str] = &["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "ar", "he"];

/// Beta-active LTR/CJK interface locale identifiers exposed in the Beta desktop GUI language
/// picker (`R15.1`: 14 non-RTL registered locales, excluding RTL locales `ar`, `fa`, `he`, `ur`
/// which are deferred until CurveBook `1.0`).
pub const BETA_ACTIVE_LOCALES: &[&str] = &["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr"];

/// Single catalog entry containing translations for all 16 registered [`LOCALES`] in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub key: &'static str,
    pub translations: [&'static str; 16],
}

/// Legacy shard-slice registry retained empty after consolidating all active entries into
/// [`CATALOG`] (generated from `locales/catalog.tsv` in lexicographic key order).
pub const EXTRA_SHARD_CATALOGS: &[&[CatalogEntry]] = &[];

include!(concat!(env!("OUT_DIR"), "/i18n_catalog.rs"));

/// Return `true` when `code` identifies a right-to-left (RTL) locale (`ar`, `he`, `fa`, `ur`, and
/// regional variants such as `ar-EG` or `he-IL`, plus legacy `iw`).
#[must_use]
pub fn is_rtl_locale(code: &str) -> bool {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.replace('_', "-").to_ascii_lowercase();
    let primary = normalized.split('-').next().unwrap_or("");
    matches!(primary, "ar" | "he" | "iw" | "fa" | "ur")
}

/// Mirror a horizontal layout coordinate `x` of span `width` inside a `canvas_width`-wide area
/// when `is_rtl` is `true`, using saturating arithmetic so out-of-bounds spans never underflow.
#[must_use]
pub const fn mirror_x_for_rtl(x: u32, width: u32, canvas_width: u32, is_rtl: bool) -> u32 {
    if is_rtl { canvas_width.saturating_sub(x).saturating_sub(width) } else { x }
}

/// Normalize an arbitrary BCP-47 locale tag to a supported [`LOCALES`] tag, falling back to `"en"`.
#[must_use]
pub fn normalize_locale(locale: &str) -> &'static str {
    let trimmed = locale.trim();
    if trimmed.is_empty() {
        return "en";
    }
    let normalized = trimmed.replace('_', "-").to_ascii_lowercase();
    let primary = normalized.split('-').next().unwrap_or("en");
    if primary == "pt" {
        return "pt-BR";
    }
    if primary == "zh" {
        let is_traditional = normalized.split('-').skip(1).any(|sub| matches!(sub, "tw" | "hk" | "mo" | "hant"));
        return if is_traditional { "zh-TW" } else { "zh-CN" };
    }
    match primary {
        "de" => "de",
        "es" => "es",
        "fr" => "fr",
        "ja" => "ja",
        "ko" => "ko",
        "it" => "it",
        "nl" => "nl",
        "pl" => "pl",
        "ru" => "ru",
        "tr" => "tr",
        "ar" => "ar",
        "he" | "iw" => "he",
        _ => "en",
    }
}

/// Returns `true` when `locale` normalizes to one of the 14 non-RTL [`BETA_ACTIVE_LOCALES`]
/// enabled for the Beta desktop GUI (`R15.1`).
#[must_use]
pub fn is_beta_active_locale(locale: &str) -> bool {
    !is_rtl_locale(locale) && BETA_ACTIVE_LOCALES.contains(&normalize_locale(locale))
}

/// Normalize an arbitrary or saved `preferences.json` locale tag for the Beta desktop GUI,
/// mapping RTL locales (`ar`, `fa`, `he`, `ur`, and legacy `iw`) cleanly to `"en"` until
/// full RTL GUI support lands in CurveBook `1.0` (`R15.1`).
#[must_use]
pub fn normalize_beta_gui_locale(locale: &str) -> &'static str {
    if is_rtl_locale(locale) {
        return "en";
    }
    normalize_locale(locale)
}

fn exact_locale_index(locale: &str) -> Option<usize> {
    let trimmed = locale.trim();
    let lower = trimmed.replace('_', "-").to_ascii_lowercase();
    match lower.as_str() {
        "en" => Some(0),
        "de" => Some(1),
        "es" => Some(2),
        "fr" => Some(3),
        "ja" => Some(4),
        "ko" => Some(5),
        "pt-br" => Some(6),
        "zh-cn" => Some(7),
        "zh-tw" => Some(8),
        "it" => Some(9),
        "nl" => Some(10),
        "pl" => Some(11),
        "ru" => Some(12),
        "tr" => Some(13),
        "ar" => Some(14),
        "he" => Some(15),
        _ => None,
    }
}

/// Native autonym display label for a supported locale tag.
#[must_use]
pub fn locale_display_name(locale: &str) -> &'static str {
    match normalize_locale(locale) {
        "de" => "Deutsch",
        "es" => "Español",
        "fr" => "Français",
        "ja" => "日本語",
        "ko" => "한국어",
        "pt-BR" => "Português (Brasil)",
        "zh-CN" => "简体中文",
        "zh-TW" => "繁體中文",
        "it" => "Italiano",
        "nl" => "Nederlands",
        "pl" => "Polski",
        "ru" => "Русский",
        "tr" => "Türkçe",
        "ar" => "العربية",
        "he" => "עברית",
        _ => "English",
    }
}

/// Returns `true` when `locale` uses machine-drafted UI strings pending native-speaker review (`R10.21` / `R10.23`).
///
/// Only `"en"` is authored directly; all 15 non-English [`LOCALES`] are labelled as draft translations in the
/// language picker until native-speaker review completes.
#[must_use]
pub fn is_draft_translation_locale(locale: &str) -> bool {
    normalize_locale(locale) != "en"
}

/// Display caption for a locale row in the UI language picker (`R10.23`).
///
/// Formats `"en"` as `"English (en)"` and appends `" [Draft]"` to all 15 non-English [`LOCALES`].
#[must_use]
pub fn locale_picker_label(locale: &str) -> String {
    let norm = normalize_locale(locale);
    let autonym = locale_display_name(norm);
    if is_draft_translation_locale(norm) { format!("{autonym} ({norm}) [Draft]") } else { format!("{autonym} ({norm})") }
}

/// Iterate over every registered [`CatalogEntry`] across [`CATALOG`] and [`EXTRA_SHARD_CATALOGS`].
pub fn all_entries() -> impl Iterator<Item = &'static CatalogEntry> {
    CATALOG.iter().chain(EXTRA_SHARD_CATALOGS.iter().flat_map(|shard| shard.iter()))
}

fn lookup_catalog_entry(key: &str) -> Option<&'static CatalogEntry> {
    CATALOG.binary_search_by_key(&key, |entry| entry.key).ok().and_then(|idx| CATALOG.get(idx))
}

fn cmp_prefixed_key(entry_key: &str, prefix: &str, suffix: &str) -> std::cmp::Ordering {
    let p_len = prefix.len();
    match entry_key.as_bytes().get(..p_len) {
        Some(head) => head.cmp(prefix.as_bytes()).then_with(|| entry_key.as_bytes()[p_len..].cmp(suffix.as_bytes())),
        None => entry_key.as_bytes().cmp(prefix.as_bytes()),
    }
}

fn lookup_command_entry(command_id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.binary_search_by(|entry| cmp_prefixed_key(entry.key, "cmd.", command_id)).ok().and_then(|idx| CATALOG.get(idx))
}

fn try_tr_in_entries<'a>(entries: impl Iterator<Item = &'a CatalogEntry>, locale: &str, key: &str) -> Option<&'static str> {
    let idx = exact_locale_index(locale)?;
    entries.filter(|entry| entry.key == key).filter_map(|entry| entry.translations.get(idx).copied()).find(|text| !text.trim().is_empty())
}

/// Look up a translation strictly for `locale` across custom [`CatalogEntry`] shard slices.
#[must_use]
pub fn try_tr_in_slices(slices: &[&[CatalogEntry]], locale: &str, key: &str) -> Option<&'static str> {
    try_tr_in_entries(slices.iter().flat_map(|slice| slice.iter()), locale, key)
}

/// Look up a translation strictly for `locale` without falling back to English.
///
/// Returns `None` if `locale` is not a registered code or if `key` is missing/empty in that locale.
#[must_use]
pub fn try_tr(locale: &str, key: &str) -> Option<&'static str> {
    let idx = exact_locale_index(locale)?;
    if let Some(text) = lookup_catalog_entry(key).and_then(|entry| entry.translations.get(idx).copied()).filter(|text| !text.trim().is_empty()) {
        return Some(text);
    }
    try_tr_in_slices(EXTRA_SHARD_CATALOGS, locale, key)
}

/// Look up a localized string for `(locale, key)`, normalizing BCP-47 tags and falling back to
/// `"en"` when the key is missing in `locale`, or `""` when `key` is unknown.
#[must_use]
pub fn tr(locale: &str, key: &str) -> &'static str {
    let canonical = normalize_locale(locale);
    try_tr(canonical, key).or_else(|| try_tr("en", key)).unwrap_or("")
}

/// Look up a localized string for `(locale, key)`, normalizing BCP-47 tags and falling back to
/// `"en"`, or returning caller-supplied `fallback` when `key` is not registered or empty.
#[must_use]
pub fn tr_or_fallback<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    let canonical = normalize_locale(locale);
    try_tr(canonical, key).or_else(|| try_tr("en", key)).unwrap_or(fallback)
}

/// Look up a localized string, returning `key` unchanged if it is not registered in [`CATALOG`] or [`EXTRA_SHARD_CATALOGS`].
#[must_use]
pub fn tr_or<'a>(locale: &str, key: &'a str) -> Cow<'a, str> {
    let resolved = tr(locale, key);
    if resolved.is_empty() { Cow::Borrowed(key) } else { Cow::Borrowed(resolved) }
}

/// Look up the localized command label strictly for `(locale, command_id)` (`cmd.<command_id>`).
#[must_use]
pub fn try_tr_command(locale: &str, command_id: &str) -> Option<&'static str> {
    let idx = exact_locale_index(locale)?;
    if let Some(text) = lookup_command_entry(command_id).and_then(|entry| entry.translations.get(idx).copied()).filter(|text| !text.trim().is_empty())
    {
        return Some(text);
    }
    EXTRA_SHARD_CATALOGS
        .iter()
        .flat_map(|shard| shard.iter())
        .filter(|entry| entry.key.strip_prefix("cmd.") == Some(command_id))
        .filter_map(|entry| entry.translations.get(idx).copied())
        .find(|text| !text.trim().is_empty())
}

/// Resolve the localized label for `command_id` in `locale`, falling back to `"en"` or `fallback`.
#[must_use]
pub fn command_label<'a>(locale: &str, command_id: &str, fallback: &'a str) -> &'a str {
    let canonical = normalize_locale(locale);
    try_tr_command(canonical, command_id).or_else(|| try_tr_command("en", command_id)).unwrap_or(fallback)
}

/// Translate an English command `label` across [`LOCALES`] using registered `cmd.*` entries.
#[must_use]
pub fn translate_command_label(locale: &str, english_label: &'static str) -> &'static str {
    let canonical = normalize_locale(locale);
    let Some(idx) = exact_locale_index(canonical) else {
        return english_label;
    };
    let start = CATALOG.partition_point(|entry| entry.key < "cmd.");
    CATALOG[start..]
        .iter()
        .take_while(|entry| entry.key.starts_with("cmd."))
        .filter(|entry| entry.translations[0] == english_label)
        .filter_map(|entry| entry.translations.get(idx).copied())
        .find(|text| !text.trim().is_empty())
        .unwrap_or(english_label)
}

/// Iterate over every unique registered localization key across [`CATALOG`] and [`EXTRA_SHARD_CATALOGS`].
pub fn all_keys() -> impl Iterator<Item = &'static str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut keys = Vec::new();
    for entry in all_entries() {
        if seen.insert(entry.key) {
            keys.push(entry.key);
        }
    }
    keys.into_iter()
}

/// Total number of message entries registered across [`CATALOG`] and [`EXTRA_SHARD_CATALOGS`].
#[must_use]
pub const fn catalog_len() -> usize {
    let mut total = CATALOG.len();
    let mut i = 0;
    while i < EXTRA_SHARD_CATALOGS.len() {
        total += EXTRA_SHARD_CATALOGS[i].len();
        i += 1;
    }
    total
}

/// Collect all `(locale, key)` pairs that are missing or empty across [`LOCALES`] x all registered shards.
#[must_use]
pub fn missing_translations() -> Vec<(&'static str, &'static str)> {
    let mut missing = Vec::new();
    for &locale in LOCALES {
        for key in all_keys() {
            if try_tr(locale, key).is_none() {
                missing.push((locale, key));
            }
        }
    }
    missing
}

/// Return the localized label for one of the eight [`TableKind`] scientific data-table archetypes.
#[must_use]
pub fn table_kind_label(locale: &str, kind: TableKind) -> &'static str {
    let key = match kind {
        TableKind::Xy => "table.xy",
        TableKind::Column => "table.column",
        TableKind::Grouped => "table.grouped",
        TableKind::Contingency => "table.contingency",
        TableKind::Survival => "table.survival",
        TableKind::PartsOfWhole => "table.parts_of_whole",
        TableKind::MultipleVariables => "table.multiple_variables",
        TableKind::Nested => "table.nested",
    };
    tr(locale, key)
}

/// Return the localized navigator folder label for a [`SheetSection`].
#[must_use]
pub fn sheet_section_label(locale: &str, section: SheetSection) -> &'static str {
    let key = match section {
        SheetSection::Data => "nav.data_tables",
        SheetSection::Info => "nav.info",
        SheetSection::Analyses => "nav.results",
        SheetSection::Graphs => "nav.graphs",
        SheetSection::Layouts => "nav.layouts",
    };
    tr(locale, key)
}

/// Regional numeric and CSV delimiter convention bound to a UI locale or explicit user override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalePreset {
    pub locale: String,
    pub decimal_separator: char,
    pub csv_delimiter: char,
    pub thousands_separator: Option<char>,
    pub minus_sign: String,
}

impl Default for LocalePreset {
    fn default() -> Self {
        Self::for_locale("en")
    }
}

impl LocalePreset {
    /// Construct the regional formatting preset for `locale` (`de`/`es`/`fr`/`pt-BR`/`it`/`nl`/
    /// `pl`/`ru`/`tr` default to comma decimals and semicolon CSV delimiters;
    /// `en`/`ja`/`ko`/`zh-CN`/`zh-TW`/`ar`/`he` use period decimals and comma CSV delimiters).
    #[must_use]
    pub fn for_locale(locale: &str) -> Self {
        let canonical = normalize_locale(locale);
        match canonical {
            "de" | "es" | "fr" | "pt-BR" | "it" | "nl" | "pl" | "ru" | "tr" => Self::comma_semicolon(canonical),
            _ => Self::period_comma(canonical),
        }
    }

    /// Standard period-decimal (`.` / `,` CSV) preset for reproducible international figures.
    #[must_use]
    pub fn period_comma(locale: &str) -> Self {
        Self {
            locale: normalize_locale(locale).to_owned(),
            decimal_separator: '.',
            csv_delimiter: ',',
            thousands_separator: None,
            minus_sign: String::from("-"),
        }
    }

    /// European/Latin regional preset using comma decimals (`,`) and semicolon CSV delimiter (`;`).
    #[must_use]
    pub fn comma_semicolon(locale: &str) -> Self {
        Self {
            locale: normalize_locale(locale).to_owned(),
            decimal_separator: ',',
            csv_delimiter: ';',
            thousands_separator: None,
            minus_sign: String::from("-"),
        }
    }

    /// Publication typography variant using Unicode minus (`U+2212`) with `locale`'s decimal rule.
    #[must_use]
    pub fn publication(locale: &str) -> Self {
        let mut base = Self::for_locale(locale);
        base.minus_sign = String::from("\u{2212}");
        base
    }

    /// Whether this preset's locale uses right-to-left (RTL) layout and text direction.
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        is_rtl_locale(&self.locale)
    }

    /// Build the [`LocaleNumberPolicy`] consumed by `curvebook-text` axis and *p*-value formatters.
    #[must_use]
    pub fn number_policy(&self) -> LocaleNumberPolicy {
        LocaleNumberPolicy {
            decimal_separator: self.decimal_separator,
            thousands_separator: self.thousands_separator,
            minus_sign: self.minus_sign.clone(),
        }
    }

    /// Build matching [`CsvOptions`] for `curvebook-format` delimited import/export dialogs.
    #[must_use]
    pub fn csv_options(&self) -> CsvOptions {
        CsvOptions {
            delimiter: self.csv_delimiter,
            decimal_separator: self.decimal_separator,
            thousands_separator: self.thousands_separator,
            ..CsvOptions::csv()
        }
    }

    /// Format a numeric axis tick using this preset's [`LocaleNumberPolicy`].
    #[must_use]
    pub fn format_axis_tick(&self, value: f64, scale: AxisScale) -> String {
        format_axis_tick(value, scale, &self.number_policy())
    }

    /// Format a *p*-value string using this preset's [`LocaleNumberPolicy`].
    #[must_use]
    pub fn format_pvalue(&self, p: f64, format: PValueFormat, include_prefix: bool, include_stars: bool) -> String {
        format_pvalue_label(p, format, include_prefix, include_stars, &self.number_policy())
    }
}

#[cfg(test)]
mod tests;
