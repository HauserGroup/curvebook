//! Deterministic workspace screenshot generation and offline PNG integrity check (`R0.14`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use curvebook_platform::read_regular_file_bytes;

const MAX_DOC_PNG_BYTES: usize = 32 * 1024 * 1024;
const MAX_SCREENSHOT_BYTES: usize = 512 * 1024;
const MAX_ICON_BYTES: usize = 64 * 1024;
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

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if parse_args(args)? { verify_existing(root) } else { generate_and_verify(root) }
}

fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        [other] => Err(format!("unknown screenshots flag `{other}` (expected `--check` or no flags)")),
        _ => Err(String::from("usage: cargo xtask screenshots [--check]")),
    }
}

fn generate_and_verify(root: &Path) -> Result<(), String> {
    let out_dir = root.join("docs/images");
    fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let nested_target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) if !dir.is_empty() => {
            let p = PathBuf::from(dir);
            if p.is_absolute() { p } else { root.join(p) }.join("doc-screenshots-nested")
        }
        _ => root.join("target/doc-screenshots-nested"),
    };
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

fn verify_doc_image(root: &Path, assets_md: &str, rel: &str, max_bytes: usize, dims: (u32, u32), min_idat: usize, kind: &str) -> Result<(), String> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path).map_err(|e| format!("missing or unreadable `{rel}`: {e}"))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{rel}` must be a regular non-symlink PNG file"));
    }
    if meta.len() > u64::try_from(MAX_DOC_PNG_BYTES).unwrap_or(u64::MAX) {
        return Err(format!("`{rel}` size {} exceeds {MAX_DOC_PNG_BYTES} bytes", meta.len()));
    }
    let bytes = read_regular_file_bytes(&path, MAX_DOC_PNG_BYTES).map_err(|e| format!("read `{rel}`: {e}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("`{rel}` size {} bytes exceeds {kind} budget ({max_bytes} bytes)", bytes.len()));
    }
    let s = validate_png_bytes_with_bounds(rel, &bytes, Some(dims), min_idat)?;
    ensure_asset_listed(rel, assets_md)?;
    println!(
        "screenshots: {rel} OK ({}x{}, color_type={}, idat={} B, chunks={}, crc32={:#010x})",
        s.width,
        s.height,
        s.color_type,
        s.idat_bytes,
        s.chunk_count,
        crc32_ieee(&bytes)
    );
    Ok(())
}

fn verify_existing(root: &Path) -> Result<(), String> {
    let assets_path = root.join("ASSETS.md");
    let assets_md =
        String::from_utf8(read_regular_file_bytes(&assets_path, MAX_ASSETS_MD_BYTES).map_err(|e| format!("read {}: {e}", assets_path.display()))?)
            .map_err(|_| format!("{} is not valid UTF-8", assets_path.display()))?;
    for rel in SCREENSHOT_REL_PATHS {
        verify_doc_image(root, &assets_md, rel, MAX_SCREENSHOT_BYTES, (EXPECTED_WIDTH, EXPECTED_HEIGHT), MIN_IDAT_BYTES, "screenshot")?;
    }
    for &(rel, dims, min_idat) in EXTRA_DOC_IMAGES {
        verify_doc_image(root, &assets_md, rel, MAX_ICON_BYTES, dims, min_idat, "icon")?;
    }
    Ok(())
}

fn ensure_asset_listed(rel: &str, assets_md: &str) -> Result<(), String> {
    let found = assets_md.lines().any(|raw| {
        raw.trim()
            .strip_prefix('|')
            .and_then(|s| s.strip_suffix('|'))
            .and_then(|i| i.split('|').next())
            .is_some_and(|c| c.trim().trim_matches('`').trim() == rel)
    });
    if found { Ok(()) } else { Err(format!("ASSETS.md is missing attribution table row for `{rel}`")) }
}

#[cfg(test)]
fn validate_png_bytes(label: &str, bytes: &[u8]) -> Result<PngSummary, String> {
    validate_png_bytes_with_bounds(label, bytes, Some((EXPECTED_WIDTH, EXPECTED_HEIGHT)), MIN_IDAT_BYTES)
}

pub fn validate_png_bytes_with_bounds(
    label: &str,
    bytes: &[u8],
    expected_dims: Option<(u32, u32)>,
    min_idat_bytes: usize,
) -> Result<PngSummary, String> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(format!("{label}: invalid PNG signature"));
    }
    let (mut pos, mut ihdr, mut idat_bytes, mut chunk_count, mut saw_iend) = (PNG_SIGNATURE.len(), None, 0usize, 0usize, false);
    while pos < bytes.len() {
        let hdr = bytes.get(pos..pos + 8).ok_or_else(|| format!("{label}: truncated PNG chunk header at offset {pos}"))?;
        let (length, kind) = (u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize, [hdr[4], hdr[5], hdr[6], hdr[7]]);
        pos += 8;
        let data_end = pos.checked_add(length).ok_or_else(|| format!("{label}: chunk length overflow"))?;
        let crc_end = data_end.checked_add(4).ok_or_else(|| format!("{label}: chunk CRC overflow"))?;
        let data = bytes.get(pos..data_end).ok_or_else(|| format!("{label}: truncated chunk data"))?;
        let crc_s = bytes.get(data_end..crc_end).ok_or_else(|| format!("{label}: truncated chunk CRC"))?;
        let (stored, computed) = (u32::from_be_bytes([crc_s[0], crc_s[1], crc_s[2], crc_s[3]]), png_chunk_crc32(kind, data));
        if stored != computed {
            return Err(format!(
                "{label}: CRC-32 mismatch in `{}` chunk (stored {stored:#010x}, computed {computed:#010x})",
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
            (saw_iend, pos, chunk_count) = (true, crc_end, chunk_count + 1);
            break;
        }
        (pos, chunk_count) = (crc_end, chunk_count + 1);
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
    let (w, h, bd, ct) =
        (u32::from_be_bytes([data[0], data[1], data[2], data[3]]), u32::from_be_bytes([data[4], data[5], data[6], data[7]]), data[8], data[9]);
    if let Some((ew, eh)) = expected_dims {
        if (ew, eh) == (EXPECTED_WIDTH, EXPECTED_HEIGHT) && (w < MIN_WIDTH || h < MIN_HEIGHT) {
            return Err(format!("{label}: dimensions {w}x{h} below minimum {MIN_WIDTH}x{MIN_HEIGHT}"));
        }
        if w != ew || h != eh {
            return Err(format!("{label}: dimensions {w}x{h} != expected {ew}x{eh}"));
        }
    } else if w == 0 || h == 0 {
        return Err(format!("{label}: non-positive PNG dimensions {w}x{h}"));
    }
    if bd != 8 || ct != 6 {
        return Err(format!("{label}: expected 8-bit RGBA (color_type=6), got bit_depth={bd}, color_type={ct}"));
    }
    Ok((w, h, ct))
}

#[must_use]
pub fn crc32_ieee(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

#[must_use]
pub fn png_chunk_crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(&kind);
    h.update(data);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn make_chunk(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + payload.len());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&png_chunk_crc32(kind, payload).to_be_bytes());
        out
    }

    fn make_png(w: u32, h: u32, ct: u8, idat_len: usize) -> Vec<u8> {
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, ct, 0, 0, 0]);
        [PNG_SIGNATURE.as_slice(), &make_chunk(*b"IHDR", &ihdr), &make_chunk(*b"IDAT", &vec![0x5Au8; idat_len]), &make_chunk(*b"IEND", &[])].concat()
    }

    #[test]
    fn validates_pngs_cli_assets_table_symlinks_and_repo_screenshots() {
        assert_eq!((crc32_ieee(b"123456789"), png_chunk_crc32(*b"IEND", &[])), (0xCBF4_3926, 0xAE42_6082));
        assert_eq!(
            validate_png_bytes("ok.png", &make_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 2048)).unwrap(),
            PngSummary { width: 1440, height: 900, color_type: 6, idat_bytes: 2048, chunk_count: 3 }
        );
        assert!(validate_png_bytes_with_bounds("c.png", &make_png(1280, 800, 6, 1024), Some((1280, 800)), 512).is_ok());
        assert!(verify_existing(&crate::workspace_root().unwrap()).is_ok());
        let mut bad_crc = make_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 2048);
        *bad_crc.last_mut().unwrap() ^= 0xFF;
        for bad in [
            b"NOTAPNG!".as_slice(),
            &make_png(800, 600, 6, 2048),
            &make_png(1280, 800, 6, 2048),
            &make_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 64),
            &bad_crc,
        ] {
            assert!(validate_png_bytes("bad", bad).is_err());
        }
        assert_eq!((parse_args(&[]), parse_args(&[String::from("--check")])), (Ok(false), Ok(true)));
        assert!(parse_args(&[String::from("--bogus")]).is_err() && parse_args(&[String::from("--check"), String::from("extra")]).is_err());
        assert!(
            ensure_asset_listed("docs/images/workspace-split-fit.png", "|\nMentioning `docs/images/workspace-split-fit.png` in prose.\n").is_err()
        );
        assert!(
            ensure_asset_listed(
                "docs/images/workspace-split-fit.png",
                "| `docs/images/workspace-split-fit.png` | CurveBook | Generated | MIT OR Apache-2.0 | Light |\n"
            )
            .is_ok()
        );

        let dir = TestDir::new("screenshots-aud-12-43");
        let (shot, icon) = (make_png(EXPECTED_WIDTH, EXPECTED_HEIGHT, 6, 2048), make_png(128, 128, 6, 128));
        let mut assets = String::from("| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n");
        for rel in SCREENSHOT_REL_PATHS {
            dir.write_bytes(rel, &shot);
            assets.push_str(&format!("| `{rel}` | CurveBook | Generated | MIT OR Apache-2.0 | ok |\n"));
        }
        assets.push_str("| `docs/images/curvebook.png` | CurveBook | Generated | MIT OR Apache-2.0 | icon |\n");
        dir.write("ASSETS.md", &assets);
        assert!(verify_existing(dir.path()).unwrap_err().contains("curvebook.png"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/nonexistent/curvebook.png", dir.join("docs/images/curvebook.png")).unwrap();
            assert!(verify_existing(dir.path()).unwrap_err().contains("non-symlink"));
            std::fs::remove_file(dir.join("docs/images/curvebook.png")).unwrap();
        }
        dir.write_bytes("docs/images/curvebook.png", &icon);
        assert!(verify_existing(dir.path()).is_ok());
    }
}
