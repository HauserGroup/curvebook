//! Unit tests for the localization catalog, regional presets, and call-site key coverage.

use std::collections::HashSet;

use curvebook_math::{AxisScale, PValueFormat};

use super::{
    CATALOG, EXTRA_SHARD_CATALOGS, LOCALES, LocalePreset, all_keys, catalog_len, command_label, is_draft_translation_locale, is_rtl_locale,
    locale_display_name, locale_picker_label, mirror_x_for_rtl, missing_translations, normalize_locale, tr, translate_command_label, try_tr,
    try_tr_command, try_tr_in_slices,
};

fn assert_rtl_and_non_latin_scripts() {
    for rtl_tag in ["ar", "he", "fa", "ur", "ar-EG", "AR_SA", "he-IL", "he_IL", "fa-IR", "ur-PK", " ar-EG "] {
        assert!(is_rtl_locale(rtl_tag), "expected RTL for `{rtl_tag}`");
    }
    for ltr_tag in ["en", "en-US", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "", "  ", "ar2", "zz"] {
        assert!(!is_rtl_locale(ltr_tag), "expected LTR for `{ltr_tag}`");
    }

    assert_eq!(mirror_x_for_rtl(10, 20, 100, false), 10);
    assert_eq!(mirror_x_for_rtl(10, 20, 100, true), 70);
    assert_eq!(mirror_x_for_rtl(70, 20, 100, true), 10);
    assert_eq!(mirror_x_for_rtl(15, 0, 100, true), 85);
    assert_eq!(mirror_x_for_rtl(95, 20, 100, true), 0);
    assert_eq!(mirror_x_for_rtl(u32::MAX, 10, 100, true), 0);
    assert_eq!(mirror_x_for_rtl(10, u32::MAX, 100, true), 0);

    let has_cyrillic = |s: &str| s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let has_arabic = |s: &str| s.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let has_hebrew = |s: &str| s.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c));

    assert!(has_cyrillic(locale_display_name("ru")));
    assert!(has_arabic(locale_display_name("ar")));
    assert!(has_hebrew(locale_display_name("he")));
    assert!(!is_draft_translation_locale("en"));
    assert!(!is_draft_translation_locale("en-US"));
    assert_eq!(locale_picker_label("en"), "English (en)");
    for &loc in &LOCALES[1..] {
        assert!(is_draft_translation_locale(loc), "expected `{loc}` to be marked as draft translation");
        let picker_lbl = locale_picker_label(loc);
        assert_eq!(picker_lbl, format!("{} ({loc}) [Draft]", locale_display_name(loc)));
    }

    for key in all_keys() {
        if matches!(key, "table.xy" | "w5.grid_core.n_col" | "w5.grid_core.x_col" | "w5.grid_core.row_number_header") {
            continue;
        }
        assert!(has_cyrillic(try_tr("ru", key).unwrap()), "expected Cyrillic in ru:{key}");
        assert!(has_arabic(try_tr("ar", key).unwrap()), "expected Arabic script in ar:{key}");
        assert!(has_hebrew(try_tr("he", key).unwrap()), "expected Hebrew script in he:{key}");
    }
}

#[test]
fn untranslated_report() {
    let expected = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "ar", "he"];
    assert_eq!(LOCALES, &expected);
    assert_eq!(LOCALES.len(), 16);
    assert_eq!(catalog_len(), 710);
    assert_eq!(LOCALES.len() * catalog_len(), 11_360);
    assert!(!CATALOG.is_empty());

    let mut seen_keys = HashSet::new();
    for entry in super::all_entries() {
        assert!(seen_keys.insert(entry.key), "duplicate i18n catalog key: {}", entry.key);
    }

    let missing = missing_translations();
    assert!(missing.is_empty(), "missing catalog translations: {missing:?}");

    for &locale in LOCALES {
        let preset = LocalePreset::for_locale(locale);
        assert_eq!(preset.locale, locale);
        assert_eq!(preset.is_rtl(), matches!(locale, "ar" | "he"));
        assert!(!locale_display_name(locale).is_empty());
        for key in all_keys() {
            let direct = try_tr(locale, key).unwrap();
            assert_eq!(tr(locale, key), direct);
        }
    }

    verify_command_catalog_coverage();
    assert_rtl_and_non_latin_scripts();

    println!(
        "i18n: {} locale(s) registered ({}); {} locales x {} keys = {} translations verified; 0 missing catalog or command keys",
        LOCALES.len(),
        LOCALES.join(", "),
        LOCALES.len(),
        catalog_len(),
        LOCALES.len() * catalog_len()
    );
}

fn verify_command_catalog_coverage() {
    let engine_specs: Vec<_> = curvebook_engine::cmd::registry().specs().collect();
    assert_eq!(engine_specs.len(), 106, "expected 106 engine CommandSpec entries");
    assert_eq!(crate::menus::UI_COMMANDS.len(), 54, "expected 54 UiCommandSpec entries");

    let mut missing_cmd = Vec::new();
    for spec in &engine_specs {
        verify_single_command_entry(spec.id, spec.label, &mut missing_cmd);
    }
    for spec in crate::menus::UI_COMMANDS {
        verify_single_command_entry(spec.id, spec.label, &mut missing_cmd);
    }
    assert!(missing_cmd.is_empty(), "untranslated command labels across 16 locales: {missing_cmd:?}");
}

fn verify_single_command_entry(command_id: &str, english_label: &'static str, missing: &mut Vec<String>) {
    match try_tr_command("en", command_id) {
        Some(en) if en == english_label => {}
        other => missing.push(format!("en:{command_id} (expected `{english_label}`, got {other:?})")),
    }
    for &locale in LOCALES {
        match try_tr_command(locale, command_id) {
            Some(text) if !text.trim().is_empty() => {
                assert_eq!(command_label(locale, command_id, ""), text);
                let by_label = translate_command_label(locale, english_label);
                assert!(!by_label.trim().is_empty(), "empty label translation for `{english_label}` in {locale}");
            }
            _ => missing.push(format!("{locale}:{command_id}")),
        }
    }
}

#[test]
fn locale_normalization_and_presets_work_without_panics() {
    assert_eq!(normalize_locale("pt_br"), "pt-BR");
    assert_eq!(normalize_locale("zh-Hant-TW"), "zh-TW");
    assert_eq!(normalize_locale("zh_CN"), "zh-CN");
    assert_eq!(normalize_locale("it-IT"), "it");
    assert_eq!(normalize_locale("nl_BE"), "nl");
    assert_eq!(normalize_locale("pl-PL"), "pl");
    assert_eq!(normalize_locale("ru_RU"), "ru");
    assert_eq!(normalize_locale("tr-TR"), "tr");
    assert_eq!(normalize_locale("ar-EG"), "ar");
    assert_eq!(normalize_locale("he_IL"), "he");
    assert_eq!(normalize_locale("iw"), "he");
    assert_eq!(normalize_locale("unknown-XX"), "en");

    let de = LocalePreset::for_locale("de");
    assert_eq!(de.decimal_separator, ',');
    assert_eq!(de.csv_delimiter, ';');
    assert!(!de.is_rtl());
    assert_eq!(de.format_axis_tick(12.34, AxisScale::Linear), "12,34");
    assert_eq!(de.format_pvalue(0.0023, PValueFormat::FourDecimal, true, true), "P = 0,0023 **");

    let ar = LocalePreset::for_locale("ar-EG");
    assert_eq!(ar.locale, "ar");
    assert_eq!(ar.decimal_separator, '.');
    assert_eq!(ar.csv_delimiter, ',');
    assert!(ar.is_rtl());
}

#[test]
fn rtl_helpers_and_non_latin_scripts_behave_cleanly() {
    assert_rtl_and_non_latin_scripts();
}

fn collect_rs_paths(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs_paths(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn skip_raw_string_literal(bytes: &[u8], start: usize, out: Option<&mut String>) -> Option<usize> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let mut hashes = 0usize;
    let mut i = start + 1;
    while bytes.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"' && bytes.get(i + 1..i + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
            let end = i + 1 + hashes;
            if let Some(buf) = out
                && let Ok(raw) = std::str::from_utf8(&bytes[start..end])
            {
                buf.push_str(raw);
            }
            return Some(end);
        }
        i += 1;
    }
    Some(bytes.len())
}

fn skip_string_literal(bytes: &[u8], start: usize, out: &mut String) -> usize {
    out.push('"');
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        out.push(b as char);
        if b == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        i += 1;
        if b == b'"' {
            break;
        }
    }
    i
}

fn strip_comments_preserving_lines(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        if block_depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_depth += 1;
                out.push_str("  ");
                i += 2;
            } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_depth -= 1;
                out.push_str("  ");
                i += 2;
            } else {
                out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if let Some(end) = skip_raw_string_literal(bytes, i, Some(&mut out)) {
            i = end;
        } else if bytes[i] == b'"' {
            i = skip_string_literal(bytes, i, &mut out);
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_depth = 1;
            out.push_str("  ");
            i += 2;
        } else {
            let ch = src.get(i..).and_then(|s| s.chars().next()).unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn parse_plain_string_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let esc = chars.next()?;
            match esc {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn split_call_args_and_end(src: &str, open_paren: usize) -> Option<(Vec<String>, usize)> {
    let bytes = src.as_bytes();
    let mut args = Vec::new();
    let mut start = open_paren + 1;
    let mut depth = 1usize;
    let mut i = open_paren + 1;
    let mut in_str = false;
    while i < bytes.len() {
        if in_str {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'"' => in_str = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    let tail = src.get(start..i).unwrap_or("").trim();
                    if !tail.is_empty() {
                        args.push(tail.to_owned());
                    }
                    return Some((args, i + 1));
                }
            }
            b',' if depth == 1 => {
                args.push(src.get(start..i).unwrap_or("").trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn extract_tr_literal_keys(file_label: &str, src: &str, out: &mut Vec<(String, String)>) {
    let code = strip_comments_preserving_lines(src);
    let bytes = code.as_bytes();
    let mut i = 0usize;
    let mut in_str = false;
    let mut prev_ident = String::new();
    while i < bytes.len() {
        if in_str {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if let Some(end) = skip_raw_string_literal(bytes, i, None) {
            prev_ident.clear();
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            in_str = true;
            prev_ident.clear();
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            let ident = code.get(start..i).unwrap_or("");
            let mut lookahead = i;
            while lookahead < bytes.len() && bytes[lookahead].is_ascii_whitespace() {
                lookahead += 1;
            }
            if prev_ident != "fn"
                && matches!(ident, "tr" | "try_tr" | "tr_or")
                && lookahead < bytes.len()
                && bytes[lookahead] == b'('
                && let Some((args, _)) = split_call_args_and_end(&code, lookahead)
            {
                let target_arg = if args.len() == 1 { args.first() } else { args.get(1) };
                if let Some(key) = target_arg.and_then(|arg| parse_plain_string_literal(arg)) {
                    let line = code.get(..start).unwrap_or("").bytes().filter(|&b| b == b'\n').count() + 1;
                    out.push((format!("{file_label}:{line}"), key));
                }
            }
            prev_ident.clear();
            prev_ident.push_str(ident);
            continue;
        }
        if !bytes[i].is_ascii_whitespace() {
            prev_ident.clear();
        }
        i += 1;
    }
}

fn verify_tr_keys_exist(calls: &[(String, String)]) -> Result<(), String> {
    let known: HashSet<&'static str> = all_keys().collect();
    let mut missing = Vec::new();
    for (location, key) in calls {
        if !known.contains(key.as_str()) {
            missing.push(format!("{location}: unregistered i18n key `{key}`"));
        }
    }
    if missing.is_empty() { Ok(()) } else { Err(missing.join("; ")) }
}

#[test]
fn shard_catalog_aggregation_searches_extra_slices_cleanly() {
    assert!(EXTRA_SHARD_CATALOGS.is_empty());
    let eu_slice = &[super::CatalogEntry {
        key: "shard.test.key",
        translations: ["Test Key", "Testschlüssel", "", "", "", "", "", "", "", "", "", "", "", "", "", ""],
    }];
    let cjk_slice = &[super::CatalogEntry {
        key: "shard.test.key",
        translations: ["", "", "", "", "テストキー", "", "", "", "", "", "", "", "", "", "مفتاح اختبار", ""],
    }];
    let slices: &[&[super::CatalogEntry]] = &[eu_slice, cjk_slice];
    assert_eq!(try_tr_in_slices(slices, "de", "shard.test.key"), Some("Testschlüssel"));
    assert_eq!(try_tr_in_slices(slices, "ja", "shard.test.key"), Some("テストキー"));
    assert_eq!(try_tr_in_slices(slices, "ar", "shard.test.key"), Some("مفتاح اختبار"));
    assert_eq!(try_tr_in_slices(slices, "fr", "shard.test.key"), None);
}

#[test]
fn catalog_is_strictly_sorted_unique_and_matches_binary_search() {
    assert_eq!(CATALOG.len(), catalog_len());
    for pair in CATALOG.windows(2) {
        assert!(pair[0].key < pair[1].key, "CATALOG must be strictly sorted with zero duplicates: `{}` vs `{}`", pair[0].key, pair[1].key);
    }
    for entry in CATALOG {
        for (idx, &locale) in LOCALES.iter().enumerate() {
            let expected = Some(entry.translations[idx]);
            assert_eq!(try_tr(locale, entry.key), expected, "binary-search mismatch for ({locale}, {})", entry.key);
            if let Some(cmd_id) = entry.key.strip_prefix("cmd.") {
                assert_eq!(try_tr_command(locale, cmd_id), expected, "command binary-search mismatch for ({locale}, {cmd_id})");
            }
        }
    }
    assert_eq!(try_tr("en", ""), None);
    assert_eq!(try_tr("en", "zzz.nonexistent.key"), None);
    assert_eq!(try_tr_command("en", "nonexistent.command"), None);
}

#[test]
fn tr_string_literal_calls_all_exist_in_catalog() -> Result<(), Box<dyn std::error::Error>> {
    let src_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_paths(&src_dir, &mut files)?;
    files.sort();

    let mut extracted = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path)?;
        let rel = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(path);
        extract_tr_literal_keys(&rel.display().to_string(), &text, &mut extracted);
    }
    assert!(!extracted.is_empty(), "expected at least one string-literal tr() call in crates/ui-egui/src");
    verify_tr_keys_exist(&extracted).map_err(std::io::Error::other)?;
    Ok(())
}

#[test]
fn verify_tr_keys_exist_rejects_unregistered_key() {
    let err = verify_tr_keys_exist(&[(String::from("synthetic_call.rs:1"), String::from("missing.fake.i18n_key"))])
        .expect_err("unregistered i18n key must be rejected");
    assert!(err.contains("missing.fake.i18n_key"));

    let mut parsed = Vec::new();
    let fake_src = format!(r#"let _ = app.t{ch}("bogus.missing.key"); let _ = i18n::t{ch}("de", "also.missing");"#, ch = "r");
    extract_tr_literal_keys("fixture.rs", &fake_src, &mut parsed);
    assert_eq!(parsed.len(), 2);
    let err_parsed = verify_tr_keys_exist(&parsed).expect_err("extracted missing literal keys must fail validation");
    assert!(err_parsed.contains("bogus.missing.key"));
    assert!(err_parsed.contains("also.missing"));
}

#[test]
fn beta_active_locales_excludes_rtl_and_falls_back_saved_rtl_prefs_to_english() {
    use super::{BETA_ACTIVE_LOCALES, LOCALES, is_beta_active_locale, is_draft_translation_locale, is_rtl_locale, normalize_beta_gui_locale};

    assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
    assert_eq!(BETA_ACTIVE_LOCALES, &LOCALES[..14]);
    assert!(BETA_ACTIVE_LOCALES.iter().all(|loc| !is_rtl_locale(loc)));
    assert_eq!(BETA_ACTIVE_LOCALES.iter().filter(|loc| is_draft_translation_locale(loc)).count(), 13);

    for rtl_tag in ["ar", "ar-EG", "he", "he-IL", "iw", "iw-IL", "fa", "fa-IR", "ur", "ur-PK"] {
        assert!(!is_beta_active_locale(rtl_tag), "{rtl_tag} must not be Beta-active");
        assert_eq!(normalize_beta_gui_locale(rtl_tag), "en", "{rtl_tag} prefs must fall back to English in Beta");
    }
    for (input, expected) in
        [("en", "en"), ("de-DE", "de"), ("pt", "pt-BR"), ("zh-Hant-TW", "zh-TW"), ("ru-RU", "ru"), ("tr-TR", "tr"), ("xx-unknown", "en")]
    {
        assert!(is_beta_active_locale(expected));
        assert_eq!(normalize_beta_gui_locale(input), expected);
    }
}

#[test]
fn tr_or_fallback_normalizes_locale_falls_back_to_english_then_caller_fallback() {
    use super::{tr, tr_or_fallback};

    assert_eq!(tr_or_fallback("de-DE", "btn.cancel", "Fallback Cancel"), tr("de", "btn.cancel"));
    assert_eq!(tr_or_fallback("xx-unknown", "btn.cancel", "Fallback Cancel"), tr("en", "btn.cancel"));
    assert_eq!(tr_or_fallback("de-DE", "unregistered.test.key", "Custom Fallback"), "Custom Fallback");
    assert_eq!(tr_or_fallback("en", "unregistered.test.key", "Custom Fallback"), "Custom Fallback");
}
