//! Optional bundled font embedding hook (`LUMENCRAFT_FONTS_DIR`).
//!
//! LumenCraft builds, tests and runs without `LUMENCRAFT_FONTS_DIR`; when `bundled-fonts` is
//! enabled and `LUMENCRAFT_FONTS_DIR` points to an external font directory containing
//! `fonts/manifest.txt`, listed faces are registered in `lumencraft_text::BUNDLED_FONTS`.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LUMENCRAFT_FONTS_DIR");
    println!("cargo:rerun-if-env-changed=LUMENCRAFT_FONTS_REQUIRED");

    let out_dir = std::env::var_os("OUT_DIR").map(PathBuf::from).ok_or_else(|| String::from("OUT_DIR not set"))?;
    let dest = out_dir.join("bundled_fonts.rs");

    let bundled = std::env::var_os("CARGO_FEATURE_BUNDLED_FONTS").is_some();
    let required = std::env::var_os("LUMENCRAFT_FONTS_REQUIRED").is_some_and(|v| v != "0" && !v.is_empty());

    let Some(raw_dir) = std::env::var_os("LUMENCRAFT_FONTS_DIR").filter(|v| !v.is_empty()) else {
        if required && bundled {
            return Err(String::from("LUMENCRAFT_FONTS_REQUIRED=1 is set, but LUMENCRAFT_FONTS_DIR is unset"));
        }
        return write_empty(&dest);
    };

    if !bundled {
        return write_empty(&dest);
    }

    let dir = resolve_dir(Path::new(&raw_dir))?;
    let manifest = dir.join("fonts").join("manifest.txt");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = match std::fs::read_to_string(&manifest) {
        Ok(s) => s,
        Err(e) if required => return Err(format!("read {}: {e}", manifest.display())),
        Err(e) => {
            println!("cargo:warning=LUMENCRAFT_FONTS_DIR manifest unreadable ({}): {e}", manifest.display());
            return write_empty(&dest);
        }
    };

    let is_wasm = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    let mut rows = String::from("pub const BUNDLED_FONTS: &[(&str, &[u8])] = &[\n");
    for line in text.lines() {
        let rel = line.split('#').next().unwrap_or_default().trim();
        if rel.is_empty() {
            continue;
        }
        if is_wasm && !rel.contains("BIZUDPGothic-Regular") {
            continue;
        }
        let font_path = dir.join("fonts").join(rel);
        if !font_path.is_file() {
            if required {
                return Err(format!("missing bundled font file {}", font_path.display()));
            }
            println!("cargo:warning=skipping missing bundled font face {}", font_path.display());
            continue;
        }
        println!("cargo:rerun-if-changed={}", font_path.display());
        let escaped = font_path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let name_escaped = rel.replace('\\', "/").replace('"', "\\\"");
        rows.push_str(&format!("    (\"{name_escaped}\", include_bytes!(\"{escaped}\")),\n"));
    }
    rows.push_str("];\n");
    std::fs::write(&dest, rows).map_err(|e| format!("write {}: {e}", dest.display()))
}

fn resolve_dir(raw: &Path) -> Result<PathBuf, String> {
    if raw.is_absolute() {
        return Ok(raw.to_path_buf());
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().and_then(Path::parent).ok_or_else(|| String::from("workspace root not found"))?;
    Ok(root.join(raw))
}

fn write_empty(dest: &Path) -> Result<(), String> {
    std::fs::write(dest, "pub const BUNDLED_FONTS: &[(&str, &[u8])] = &[];\n").map_err(|e| format!("write {}: {e}", dest.display()))
}
