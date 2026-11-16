//! Deterministic workspace screenshot generation and offline PNG integrity check (`R0.14`).
//!
//! `cargo xtask screenshots` invokes the clean-room `curvebook-ui-egui` `doc_screenshots`
//! example to regenerate `docs/images/workspace-split-fit.png` and
//! `docs/images/workspace-layout-dark.png`.
//! `cargo xtask screenshots --check` runs purely in-process (no nested Cargo build) and verifies:
//! - both PNG files exist and start with the 8-byte PNG signature,
//! - the leading `IHDR` chunk decodes to exact `1440x900` (`>= 1280x800`) RGBA output,
//! - every PNG chunk passes IEEE 802.3 CRC-32 validation cleanly through `IEND`,
//! - total `IDAT` payload size is non-trivial (`>= 1024` bytes), and
//! - both files are attributed in `ASSETS.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const EXPECTED_WIDTH: u32 = 1440;
const EXPECTED_HEIGHT: u32 = 900;
const MIN_WIDTH: u32 = 1280;
const MIN_HEIGHT: u32 = 800;
const MIN_IDAT_BYTES: usize = 1024;

pub const SCREENSHOT_REL_PATHS: &[&str] = &["docs/images/workspace-split-fit.png", "docs/images/workspace-layout-dark.png"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PngSummary {
    width: u32,
    height: u32,
    color_type: u8,
    idat_bytes: usize,
    chunk_count: usize,
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
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("doc-screenshots-nested"),
        _ => root.join("target").join("doc-screenshots-nested"),
    }
}

fn generate_and_verify(root: &Path) -> Result<(), String> {
    let out_dir = root.join("docs/images");
    fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let nested_target = nested_target_dir(root);
    let args = ["run", "--offline", "--locked", "--quiet", "-p", "curvebook-ui-egui", "--example", "doc_screenshots", "--", "docs/images"];
    println!("+ CARGO_TARGET_DIR={} cargo {}", nested_target.display(), args.join(" "));

    let status = Command::new("cargo")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", &nested_target)
        .args(args)
        .status()
        .map_err(|e| format!("invoke doc_screenshots example: {e}"))?;
    if !status.success() {
        return Err(format!("doc_screenshots example failed with {status}"));
    }

    verify_existing(root)
}

fn verify_existing(root: &Path) -> Result<(), String> {
    let assets_path = root.join("ASSETS.md");
    let assets_md = fs::read_to_string(&assets_path).map_err(|e| format!("read {}: {e}", assets_path.display()))?;

    for rel in SCREENSHOT_REL_PATHS {
        let abs = root.join(rel);
        let bytes = fs::read(&abs).map_err(|e| format!("missing or unreadable screenshot `{rel}`: {e}"))?;
        let summary = validate_png_bytes(rel, &bytes)?;
        ensure_asset_listed(rel, &assets_md)?;
        println!(
            "screenshots: {rel} OK ({}x{}, color_type={}, idat={} B, chunks={})",
            summary.width, summary.height, summary.color_type, summary.idat_bytes, summary.chunk_count
        );
    }
    Ok(())
}

fn ensure_asset_listed(rel: &str, assets_md: &str) -> Result<(), String> {
    let needle = format!("`{rel}`");
    if assets_md.contains(&needle) { Ok(()) } else { Err(format!("ASSETS.md is missing attribution row containing {needle}")) }
}

fn validate_png_bytes(label: &str, bytes: &[u8]) -> Result<PngSummary, String> {
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
            ihdr = Some(parse_ihdr(label, kind, data)?);
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
    if idat_bytes < MIN_IDAT_BYTES {
        return Err(format!("{label}: IDAT payload too small ({idat_bytes} B < {MIN_IDAT_BYTES} B)"));
    }

    Ok(PngSummary { width, height, color_type, idat_bytes, chunk_count })
}

fn parse_ihdr(label: &str, kind: [u8; 4], data: &[u8]) -> Result<(u32, u32, u8), String> {
    if &kind != b"IHDR" || data.len() != 13 {
        return Err(format!("{label}: first PNG chunk must be 13-byte IHDR"));
    }
    let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let bit_depth = data[8];
    let color_type = data[9];
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return Err(format!("{label}: dimensions {width}x{height} below minimum {MIN_WIDTH}x{MIN_HEIGHT}"));
    }
    if width != EXPECTED_WIDTH || height != EXPECTED_HEIGHT {
        return Err(format!("{label}: dimensions {width}x{height} != expected {EXPECTED_WIDTH}x{EXPECTED_HEIGHT}"));
    }
    if bit_depth != 8 || color_type != 6 {
        return Err(format!("{label}: expected 8-bit RGBA (color_type=6), got bit_depth={bit_depth}, color_type={color_type}"));
    }
    Ok((width, height, color_type))
}

fn png_chunk_crc32(kind: [u8; 4], data: &[u8]) -> u32 {
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
    use super::{EXPECTED_HEIGHT, EXPECTED_WIDTH, PNG_SIGNATURE, parse_args, png_chunk_crc32, validate_png_bytes};

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
}
