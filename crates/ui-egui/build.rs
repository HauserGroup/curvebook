//! Build-time env tracking and sorted 16-locale i18n catalog generation for `curvebook-ui-egui`.
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const EXPECTED_HEADER: &str = "key\ten\tde\tes\tfr\tja\tko\tpt-BR\tzh-CN\tzh-TW\tit\tnl\tpl\tru\ttr\tar\the";
const LOCALE_COUNT: usize = 16;

fn unescape_tsv_field(raw: &str, line_no: usize, col_idx: usize) -> Result<String, String> {
    if !raw.contains('\\') {
        return Ok(raw.to_owned());
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(esc) = chars.next() else {
            return Err(format!("locales/catalog.tsv:{line_no} col {col_idx}: trailing backslash"));
        };
        match esc {
            '\\' => out.push('\\'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            other => {
                return Err(format!("locales/catalog.tsv:{line_no} col {col_idx}: unsupported escape `\\{other}`"));
            }
        }
    }
    Ok(out)
}

fn parse_catalog_tsv(src: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut saw_header = false;
    let mut seen_keys = HashSet::new();
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !saw_header {
            if line != EXPECTED_HEADER {
                return Err(format!("locales/catalog.tsv:{line_no}: header mismatch (expected `{EXPECTED_HEADER}`)"));
            }
            saw_header = true;
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != LOCALE_COUNT + 1 {
            return Err(format!("locales/catalog.tsv:{line_no}: expected {} tab-separated columns, found {}", LOCALE_COUNT + 1, cols.len()));
        }
        let key = unescape_tsv_field(cols[0], line_no, 0)?;
        if key.trim().is_empty() {
            return Err(format!("locales/catalog.tsv:{line_no}: empty i18n key"));
        }
        if !seen_keys.insert(key.clone()) {
            return Err(format!("locales/catalog.tsv:{line_no}: duplicate i18n key `{key}`"));
        }
        let mut translations = Vec::with_capacity(LOCALE_COUNT);
        for (loc_idx, raw_val) in cols[1..].iter().enumerate() {
            let val = unescape_tsv_field(raw_val, line_no, loc_idx + 1)?;
            if val.trim().is_empty() {
                return Err(format!("locales/catalog.tsv:{line_no}: empty translation for key `{key}` at locale column {loc_idx}"));
            }
            translations.push(val);
        }
        entries.push((key, translations));
    }

    if !saw_header {
        return Err(String::from("locales/catalog.tsv: missing header row"));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn generate_catalog_rs(entries: &[(String, Vec<String>)]) -> String {
    let mut out = String::with_capacity(entries.len() * 512);
    out.push_str("/// Complete sorted 16-locale interface translation catalog generated from `locales/catalog.tsv`.\n");
    out.push_str("pub const CATALOG: &[CatalogEntry] = &[\n");
    for (key, translations) in entries {
        let _ = writeln!(out, "    CatalogEntry {{");
        let _ = writeln!(out, "        key: {key:?},");
        out.push_str("        translations: [\n");
        for tr in translations {
            let _ = writeln!(out, "            {tr:?},");
        }
        out.push_str("        ],\n");
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}

fn run() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=locales/catalog.tsv");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_BUILD_SHA");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| format!("CARGO_MANIFEST_DIR: {e}"))?);
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?);
    let tsv_path = manifest_dir.join("locales").join("catalog.tsv");
    let src = fs::read_to_string(&tsv_path).map_err(|e| format!("failed to read `{}`: {e}", tsv_path.display()))?;
    let entries = parse_catalog_tsv(&src)?;
    let generated = generate_catalog_rs(&entries);
    let dest = out_dir.join("i18n_catalog.rs");
    fs::write(&dest, generated).map_err(|e| format!("failed to write `{}`: {e}", dest.display()))?;
    Ok(())
}

fn main() -> ExitCode {
    if let Err(err) = run() {
        eprintln!("curvebook-ui-egui build.rs error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
