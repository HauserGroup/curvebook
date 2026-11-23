//! `cargo xtask ico <out.ico> <png>...`: pack PNG renders (each <= 256x256) into a Windows `.ico`
//! container without external tools.

use std::path::{Path, PathBuf};

use curvebook_platform::{read_regular_file_bytes, write_atomic};

const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const MAX_ICO_INPUT_PNG_BYTES: usize = 64 * 1024 * 1024;

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
    if bytes.len() < 24 || bytes[..8] != PNG_SIG || &bytes[12..16] != b"IHDR" {
        return Err(String::from("not a valid PNG with leading IHDR chunk"));
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((w, h))
}

fn encode_ico(frames: &[Frame]) -> Result<Vec<u8>, String> {
    let count = u16::try_from(frames.len()).map_err(|_| String::from("too many ICO frames"))?;
    let header_len = 6usize + 16usize * frames.len();
    let total_payload: usize = frames.iter().map(|f| f.bytes.len()).sum();
    let mut buf = Vec::with_capacity(header_len + total_payload);

    // ICONDIR: reserved (0), type (1 = icon), count
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());

    let mut offset = u32::try_from(header_len).map_err(|_| String::from("ICO header overflow"))?;
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
    use super::{Frame, encode_ico, png_dimensions};

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
}
