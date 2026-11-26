//! `cargo xtask ico <out.ico> <png>...`: pack PNG renders (each <= 256x256) into a Windows `.ico`
//! container without external tools.

use std::path::{Path, PathBuf};

use curvebook_platform::{read_regular_file_bytes, write_atomic};

const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const MAX_ICO_INPUT_PNG_BYTES: usize = 64 * 1024 * 1024;
const MAX_ICO_TOTAL_BYTES: u32 = 256 * 1024 * 1024;
const MIN_PNG_IHDR_BYTES: usize = 29; // 8-byte signature + 4-byte length + 4-byte type + 13-byte IHDR data

struct Frame {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(String::from("usage: cargo xtask ico <out.ico> <in1.png> [in2.png ...]"));
    }
    let out = PathBuf::from(&args[0]);
    let inputs: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
    pack_files(&out, &inputs)?;
    println!("ico: packed {} PNG frame(s) -> {}", inputs.len(), out.display());
    Ok(())
}

pub fn pack_files(out: &Path, inputs: &[PathBuf]) -> Result<(), String> {
    if inputs.is_empty() {
        return Err(String::from("ICO requires at least one PNG input"));
    }
    let mut frames = Vec::with_capacity(inputs.len());
    for path in inputs {
        let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(format!("{}: ICO input PNG must be a regular non-symlink file", path.display()));
        }
        let bytes = read_regular_file_bytes(path, MAX_ICO_INPUT_PNG_BYTES).map_err(|e| format!("{}: {e}", path.display()))?;
        let (width, height) = png_dimensions(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return Err(format!("{}: ICO PNG dimensions must be in 1..=256 (got {width}x{height})", path.display()));
        }
        frames.push(Frame { width, height, bytes });
    }
    let ico = encode_ico(&frames)?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    write_atomic(out, &ico).map_err(|e| format!("write {}: {e}", out.display()))
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < MIN_PNG_IHDR_BYTES || bytes[..8] != PNG_SIG || bytes[8..12] != 13u32.to_be_bytes() || &bytes[12..16] != b"IHDR" {
        return Err(String::from("not a valid PNG with leading 13-byte IHDR chunk"));
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((w, h))
}

fn compute_ico_layout(frame_lens: &[usize], max_total_bytes: u32) -> Result<(u16, u32, u32), String> {
    if frame_lens.is_empty() {
        return Err(String::from("ICO requires at least one PNG frame"));
    }
    let count = u16::try_from(frame_lens.len()).map_err(|_| String::from("too many ICO frames"))?;
    let header_len = u32::from(count).checked_mul(16).and_then(|n| n.checked_add(6)).ok_or_else(|| String::from("ICO header overflow"))?;
    let mut total = header_len;
    for &len in frame_lens {
        let size = u32::try_from(len).map_err(|_| String::from("PNG frame > 4 GiB"))?;
        total = total.checked_add(size).ok_or_else(|| String::from("ICO offset overflow"))?;
    }
    if total > max_total_bytes {
        return Err(format!("ICO total size {total} bytes exceeds budget ({max_total_bytes} bytes)"));
    }
    Ok((count, header_len, total))
}

fn encode_ico(frames: &[Frame]) -> Result<Vec<u8>, String> {
    let frame_lens: Vec<usize> = frames.iter().map(|f| f.bytes.len()).collect();
    let (count, header_len, total_u32) = compute_ico_layout(&frame_lens, MAX_ICO_TOTAL_BYTES)?;
    let total_usize = usize::try_from(total_u32).map_err(|_| String::from("ICO total size overflows usize"))?;
    let mut buf = Vec::new();
    buf.try_reserve_exact(total_usize).map_err(|e| format!("ICO buffer allocation failed ({total_usize} bytes): {e}"))?;

    // ICONDIR: reserved (0), type (1 = icon), count
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());

    let mut offset = header_len;
    for f in frames {
        let w_byte = if f.width == 256 { 0u8 } else { u8::try_from(f.width).unwrap_or(0) };
        let h_byte = if f.height == 256 { 0u8 } else { u8::try_from(f.height).unwrap_or(0) };
        let size = u32::try_from(f.bytes.len()).map_err(|_| String::from("PNG frame > 4 GiB"))?;
        buf.push(w_byte);
        buf.push(h_byte);
        buf.push(0); // color count
        buf.push(0); // reserved
        buf.extend_from_slice(&1u16.to_le_bytes()); // color planes
        buf.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        offset = offset.checked_add(size).ok_or_else(|| String::from("ICO offset overflow"))?;
    }

    for f in frames {
        buf.extend_from_slice(&f.bytes);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::{Frame, MAX_ICO_INPUT_PNG_BYTES, MAX_ICO_TOTAL_BYTES, compute_ico_layout, encode_ico, pack_files, png_dimensions};
    use crate::scan_util::test_support::TestDir;

    fn fake_png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    #[test]
    fn encodes_valid_icondir_for_16_and_256_frames() {
        let p16 = fake_png_header(16, 16);
        let p256 = fake_png_header(256, 256);
        assert_eq!(png_dimensions(&p16).unwrap(), (16, 16));
        let ico = encode_ico(&[Frame { width: 16, height: 16, bytes: p16 }, Frame { width: 256, height: 256, bytes: p256 }]).unwrap();
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0); // 256 encoded as 0 in ICONDIRENTRY
    }

    #[test]
    fn aud_12_42_rejects_empty_inputs_bad_ihdr_length_and_offset_overflow() {
        let dir = TestDir::new("ico-aud-12-42");
        assert!(pack_files(&dir.join("empty.ico"), &[]).is_err());
        assert!(encode_ico(&[]).is_err());

        let mut bad_ihdr_len = fake_png_header(16, 16);
        bad_ihdr_len[11] = 12;
        assert!(png_dimensions(&bad_ihdr_len).is_err());

        let truncated = fake_png_header(16, 16)[..24].to_vec();
        assert!(png_dimensions(&truncated).is_err());

        // 65 * 64 MiB exceeds u32::MAX without allocating the payload buffer.
        let overflow_lens = vec![MAX_ICO_INPUT_PNG_BYTES; 65];
        assert!(compute_ico_layout(&overflow_lens, u32::MAX).unwrap_err().contains("offset overflow"));

        // 5 * 64 MiB fits u32 but exceeds the 256 MiB total ICO cap.
        let over_cap_lens = vec![MAX_ICO_INPUT_PNG_BYTES; 5];
        assert!(compute_ico_layout(&over_cap_lens, MAX_ICO_TOTAL_BYTES).unwrap_err().contains("exceeds budget"));
    }
}
