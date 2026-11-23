//! Deterministic workspace screenshot generation and offline PNG integrity check (`R0.14`).
//!
//! `cargo xtask screenshots` invokes the clean-room `curvebook-ui-egui` `doc_screenshots`
//! example to regenerate `docs/images/workspace-*.png` and `docs/images/specimen-*.png`.
//! `cargo xtask screenshots --check` runs purely in-process (no nested Cargo build) and verifies:
//! - both PNG files exist and start with the 8-byte PNG signature,
//! - the leading `IHDR` chunk decodes to exact `1440x900` (`>= 1280x800`) RGBA output,
//! - every PNG chunk passes IEEE 802.3 CRC-32 validation cleanly through `IEND`,
//! - total `IDAT` payload size is non-trivial (`>= 1024` bytes), and
//! - both files are attributed in `ASSETS.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use curvebook_platform::read_regular_file_bytes;

const MAX_DOC_PNG_BYTES: usize = 32 * 1024 * 1024;
const MAX_ASSETS_MD_BYTES: usize = 4 * 1024 * 1024;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const EXPECTED_WIDTH: u32 = 1440;
const EXPECTED_HEIGHT: u32 = 900;
const MIN_WIDTH: u32 = 1280;
const MIN_HEIGHT: u32 = 800;
const MIN_IDAT_BYTES: usize = 1024;

pub const SCREENSHOT_REL_PATHS: &[&str] = &[
    "docs/images/workspace-split-fit.png",
    "docs/images/workspace-layout-dark.png",
    "docs/images/workspace-anova-results.png",
    "docs/images/specimen-light.png",
    "docs/images/specimen-dark.png",
    "docs/images/specimen-high-contrast.png",
];
const EXTRA_DOC_IMAGES: &[(&str, (u32, u32), usize)] = &[("docs/images/curvebook.png", (128, 128), 64)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngSummary {
    pub width: u32,
    pub height: u32,
    pub color_type: u8,
    pub idat_bytes: usize,
    pub chunk_count: usize,
}

/// Entry point for `cargo xtask screenshots [--check]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let check_only = parse_args(args)?;
    if check_only { verify_existing(root) } else { generate_and_verify(root) }
}

fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        [other] => Err(format!("unknown screenshots flag `{other}` (expected `--check` or no flags)")),
        _ => Err(String::from("usage: cargo xtask screenshots [--check]")),
    }
}

fn nested_target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) if !dir.is_empty() => {
            let p = PathBuf::from(dir);
            if p.is_absolute() { p } else { root.join(p) }.join("doc-screenshots-nested")
        }
        _ => root.join("target").join("doc-screenshots-nested"),
    }
}

fn generate_and_verify(root: &Path) -> Result<(), String> {
    let out_dir = root.join("docs/images");
    fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let nested_target = nested_target_dir(root);
    let nested_str = nested_target.to_str().ok_or_else(|| format!("non-UTF-8 path {}", nested_target.display()))?;
    let args = ["run", "--offline", "--locked", "--quiet", "-p", "curvebook-ui-egui", "--example", "doc_screenshots", "--", "docs/images"];
    println!("+ CARGO_TARGET_DIR={} cargo {}", nested_target.display(), args.join(" "));

    let out = crate::dev_tool_command("cargo")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", nested_str)
        .args(args)
        .wall_timeout(Duration::from_secs(600))
        .idle_timeout(Duration::from_secs(180))
        .run()
        .map_err(|e| format!("invoke doc_screenshots example: {e}"))?;
    if !out.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    if !out.success {
        return Err(format!("doc_screenshots example failed with exit code {:?}", out.status_code));
    }

    verify_existing(root)
}

fn read_non_symlink_png_bytes(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("missing or unreadable `{label}`: {e}"))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{label}` must be a regular non-symlink PNG file"));
    }
    if meta.len() > u64::try_from(MAX_DOC_PNG_BYTES).unwrap_or(u64::MAX) {
        return Err(format!("`{label}` size {} exceeds {MAX_DOC_PNG_BYTES} bytes", meta.len()));
    }
    read_regular_file_bytes(path, MAX_DOC_PNG_BYTES).map_err(|e| format!("read `{label}`: {e}"))
}

fn verify_existing(root: &Path) -> Result<(), String> {
    let assets_path = root.join("ASSETS.md");
    let assets_bytes = read_regular_file_bytes(&assets_path, MAX_ASSETS_MD_BYTES).map_err(|e| format!("read {}: {e}", assets_path.display()))?;
    let assets_md = String::from_utf8(assets_bytes).map_err(|_| format!("{} is not valid UTF-8", assets_path.display()))?;

    for rel in SCREENSHOT_REL_PATHS {
        let abs = root.join(rel);
        let bytes = read_non_symlink_png_bytes(&abs, rel)?;
        let summary = validate_png_bytes(rel, &bytes)?;
        ensure_asset_listed(rel, &assets_md)?;
        println!(
            "screenshots: {rel} OK ({}x{}, color_type={}, idat={} B, chunks={}, crc32={:#010x})",
            summary.width,
            summary.height,
            summary.color_type,
            summary.idat_bytes,
            summary.chunk_count,
            crc32_ieee(&bytes)
        );
    }

    for &(rel, (exp_w, exp_h), min_idat) in EXTRA_DOC_IMAGES {
        let abs = root.join(rel);
        if abs.exists() {
            let bytes = read_non_symlink_png_bytes(&abs, rel)?;
            let summary = validate_png_bytes_with_bounds(rel, &bytes, Some((exp_w, exp_h)), min_idat)?;
            ensure_asset_listed(rel, &assets_md)?;
            println!(
                "screenshots: {rel} OK ({}x{}, color_type={}, idat={} B, chunks={}, crc32={:#010x})",
                summary.width,
                summary.height,
                summary.color_type,
                summary.idat_bytes,
                summary.chunk_count,
                crc32_ieee(&bytes)
            );
        }
    }
    Ok(())
}

fn ensure_asset_listed(rel: &str, assets_md: &str) -> Result<(), String> {
    let found = assets_md.lines().any(|raw| {
        let Some(inner) = raw.trim().strip_prefix('|').and_then(|s| s.strip_suffix('|')) else {
            return false;
        };
        let Some(first_cell) = inner.split('|').next() else {
            return false;
        };
        let asset = first_cell.trim().trim_matches('`').trim();
        asset == rel
    });
    if found { Ok(()) } else { Err(format!("ASSETS.md is missing attribution table row for `{rel}`")) }
}

/// Validate a 1440x900 8-bit RGBA PNG payload (`R0.14`).
pub fn validate_png_bytes(label: &str, bytes: &[u8]) -> Result<PngSummary, String> {
    validate_png_bytes_with_bounds(label, bytes, Some((EXPECTED_WIDTH, EXPECTED_HEIGHT)), MIN_IDAT_BYTES)
}

/// Validate any 8-bit RGBA PNG payload against optional exact `(width, height)` bounds and minimum `IDAT` bytes.
pub fn validate_png_bytes_with_bounds(
    label: &str,
    bytes: &[u8],
    expected_dims: Option<(u32, u32)>,
    min_idat_bytes: usize,
) -> Result<PngSummary, String> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(format!("{label}: invalid PNG signature"));
    }

    let mut pos = PNG_SIGNATURE.len();
    let mut ihdr: Option<(u32, u32, u8)> = None;
    let mut idat_bytes = 0usize;
    let mut chunk_count = 0usize;
    let mut saw_iend = false;

    while pos < bytes.len() {
        let header = bytes.get(pos..pos + 8).ok_or_else(|| format!("{label}: truncated PNG chunk header at offset {pos}"))?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = [header[4], header[5], header[6], header[7]];
        pos += 8;

        let data_end = pos.checked_add(length).ok_or_else(|| format!("{label}: chunk length overflow"))?;
        let crc_end = data_end.checked_add(4).ok_or_else(|| format!("{label}: chunk CRC overflow"))?;
        let data = bytes.get(pos..data_end).ok_or_else(|| format!("{label}: truncated chunk data"))?;
        let crc_slice = bytes.get(data_end..crc_end).ok_or_else(|| format!("{label}: truncated chunk CRC"))?;
        let stored_crc = u32::from_be_bytes([crc_slice[0], crc_slice[1], crc_slice[2], crc_slice[3]]);
        let computed_crc = png_chunk_crc32(kind, data);
        if stored_crc != computed_crc {
            return Err(format!(
                "{label}: CRC-32 mismatch in `{}` chunk (stored {stored_crc:#010x}, computed {computed_crc:#010x})",
                String::from_utf8_lossy(&kind)
            ));
        }

        if chunk_count == 0 {
            ihdr = Some(parse_ihdr_with_bounds(label, kind, data, expected_dims)?);
        } else if &kind == b"IDAT" {
            idat_bytes = idat_bytes.saturating_add(data.len());
        } else if &kind == b"IEND" {
            if !data.is_empty() {
                return Err(format!("{label}: non-empty IEND chunk ({} B)", data.len()));
            }
            saw_iend = true;
            pos = crc_end;
            chunk_count += 1;
            break;
        }

        pos = crc_end;
        chunk_count += 1;
    }

    if !saw_iend || pos != bytes.len() {
        return Err(format!("{label}: missing or non-terminal IEND chunk (pos={pos}, total={})", bytes.len()));
    }

    let (width, height, color_type) = ihdr.ok_or_else(|| format!("{label}: missing IHDR chunk"))?;
    if idat_bytes < min_idat_bytes {
        return Err(format!("{label}: IDAT payload too small ({idat_bytes} B < {min_idat_bytes} B)"));
    }

    Ok(PngSummary { width, height, color_type, idat_bytes, chunk_count })
}

fn parse_ihdr_with_bounds(label: &str, kind: [u8; 4], data: &[u8], expected_dims: Option<(u32, u32)>) -> Result<(u32, u32, u8), String> {
    if &kind != b"IHDR" || data.len() != 13 {
        return Err(format!("{label}: first PNG chunk must be 13-byte IHDR"));
    }
    let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let bit_depth = data[8];
    let color_type = data[9];
    if let Some((exp_w, exp_h)) = expected_dims {
        if (exp_w, exp_h) == (EXPECTED_WIDTH, EXPECTED_HEIGHT) && (width < MIN_WIDTH || height < MIN_HEIGHT) {
            return Err(format!("{label}: dimensions {width}x{height} below minimum {MIN_WIDTH}x{MIN_HEIGHT}"));
        }
        if width != exp_w || height != exp_h {
            return Err(format!("{label}: dimensions {width}x{height} != expected {exp_w}x{exp_h}"));
        }
    } else if width == 0 || height == 0 {
        return Err(format!("{label}: non-positive PNG dimensions {width}x{height}"));
    }
    if bit_depth != 8 || color_type != 6 {
        return Err(format!("{label}: expected 8-bit RGBA (color_type=6), got bit_depth={bit_depth}, color_type={color_type}"));
    }
    Ok((width, height, color_type))
}

/// Compute IEEE 802.3 CRC-32 over raw PNG bytes.
#[must_use]
pub fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320_u32 & mask);
        }
    }
    !crc
}

#[must_use]
pub fn png_chunk_crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in kind.into_iter().chain(data.iter().copied()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320_u32 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::{
        EXPECTED_HEIGHT, EXPECTED_WIDTH, PNG_SIGNATURE, parse_args, png_chunk_crc32, validate_png_bytes, validate_png_bytes_with_bounds,
        verify_existing,
    };

    fn make_chunk(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u32;
        let crc = png_chunk_crc32(kind, payload);
        let mut out = Vec::with_capacity(12 + payload.len());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    fn make_synthetic_png(width: u32, height: u32, color_type: u8, idat_len: usize) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.push(8);
        ihdr.push(color_type);
        ihdr.extend_from_slice(&[0, 0, 0]);
        png.extend_from_slice(&make_chunk(*b"IHDR", &ihdr));
        png.extend_from_slice(&make_chunk(*b"IDAT", &vec![0x5Au8; idat_len]));
        png.extend_from_slice(&make_chunk(*b"IEND", &[]));
        png
    }

    #[test]
    fn accepts_valid_1440x900_rgba_png() {
        let bytes = make_synthetic_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 2048);
        let res = validate_png_bytes("sample.png", &bytes);
        assert!(res.is_ok(), "{res:?}");
        if let Ok(summary) = res {
            assert_eq!(summary.width, 1440);
            assert_eq!(summary.height, 900);
            assert_eq!(summary.idat_bytes, 2048);
            assert_eq!(summary.chunk_count, 3);
        }

        let custom = make_synthetic_png(1280, 800, 6, 1024);
        let custom_res = validate_png_bytes_with_bounds("custom.png", &custom, Some((1280, 800)), 512);
        assert!(custom_res.is_ok(), "{custom_res:?}");
    }

    #[test]
    fn verifies_repo_doc_screenshots_against_assets() {
        let root = crate::workspace_root().unwrap();
        assert!(verify_existing(&root).is_ok());
    }

    #[test]
    fn rejects_bad_magic_crc_small_dims_and_tiny_idat() {
        let bad_magic = b"NOTAPNG!";
        assert!(validate_png_bytes("bad-magic", bad_magic).is_err());

        let small = make_synthetic_png(800, 600, 6, 2048);
        assert!(validate_png_bytes("small", &small).is_err());

        let wrong_exact = make_synthetic_png(1280, 800, 6, 2048);
        assert!(validate_png_bytes("wrong-exact", &wrong_exact).is_err());

        let tiny_idat = make_synthetic_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 64);
        assert!(validate_png_bytes("tiny-idat", &tiny_idat).is_err());

        let mut bad_crc = make_synthetic_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 2048);
        if let Some(last) = bad_crc.last_mut() {
            *last ^= 0xFF;
        }
        assert!(validate_png_bytes("bad-crc", &bad_crc).is_err());
    }

    #[test]
    fn parses_screenshots_cli_flags() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&[String::from("--check")]), Ok(true));
        assert!(parse_args(&[String::from("--bogus")]).is_err());
        assert!(parse_args(&[String::from("--check"), String::from("extra")]).is_err());
    }

    #[test]
    fn fa_sec23_13_ensure_asset_listed_requires_table_row_not_prose() {
        let prose_only = "|\nMentioning `docs/images/workspace-split-fit.png` in prose is not a table row.\n";
        assert!(super::ensure_asset_listed("docs/images/workspace-split-fit.png", prose_only).is_err());
        assert!(super::ensure_asset_listed("docs/images/workspace-split-fit.png", "|").is_err());
        let row = "| `docs/images/workspace-split-fit.png` | CurveBook | Generated | MIT OR Apache-2.0 | Light |\n";
        assert!(super::ensure_asset_listed("docs/images/workspace-split-fit.png", row).is_ok());
    }
}
