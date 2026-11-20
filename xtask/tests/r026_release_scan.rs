//! Regression gate for `R0.26` (`cargo xtask release-scan`): dynamic container fixture synthesis,
//! `SHA256SUMS.txt` integrity checks, planted brand/cleanroom/asset detection with container
//! attribution, path-traversal guards, and fail-closed handling of unknown or unextractable formats.
//!
//! Zero binary blobs are committed to git; every `.zip`, `.tar.gz`, `.deb`, `.rpm`, `.zsync`, and
//! (when host kernel loop mounts are permitted) `.dmg` fixture is built in temp storage at test time.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_format::ZipWriter;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
static CLEAN_FIXTURES: OnceLock<Result<CleanFixtures, String>> = OnceLock::new();

struct CleanFixtures {
    icon_32: Vec<u8>,
    icon_svg: Vec<u8>,
    mime_xml: Vec<u8>,
    desktop_entry: Vec<u8>,
    assets_md: Vec<u8>,
    notice: Vec<u8>,
    #[cfg(target_os = "macos")]
    plist_tpl: String,
    #[cfg(target_os = "macos")]
    icns: Vec<u8>,
}

impl CleanFixtures {
    fn shared(root: &Path) -> Result<&'static Self, String> {
        let cached = CLEAN_FIXTURES.get_or_init(|| Self::load(root));
        match cached {
            Ok(fx) => Ok(fx),
            Err(err) => Err(err.clone()),
        }
    }

    fn load(root: &Path) -> Result<Self, String> {
        let read_bytes = |rel: &str| fs::read(root.join(rel)).map_err(|e| format!("read {rel}: {e}"));
        Ok(Self {
            icon_32: read_bytes("assets/app-icon/hicolor/32x32/apps/org.curvebook.CurveBook.png")?,
            icon_svg: read_bytes("assets/ui/toolbar_menus/new_project.svg")?,
            mime_xml: read_bytes("packaging/linux/org.curvebook.CurveBook.mime.xml")?,
            desktop_entry: read_bytes("packaging/linux/org.curvebook.CurveBook.desktop")?,
            assets_md: read_bytes("ASSETS.md")?,
            notice: read_bytes("NOTICE")?,
            #[cfg(target_os = "macos")]
            plist_tpl: fs::read_to_string(root.join("packaging/macos/Info.plist.in")).map_err(|e| format!("read Info.plist.in: {e}"))?,
            #[cfg(target_os = "macos")]
            icns: read_bytes("assets/app-icon/curvebook.icns")?,
        })
    }
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-r026-test-{label}-{}-{nanos}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|e| format!("create_dir_all {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn join(parts: &[&str]) -> String {
    parts.concat()
}

fn run_release_scan(args: &[&str]) -> Result<std::process::Output, String> {
    let root = workspace_root()?;
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(root)
        .arg("release-scan")
        .args(args)
        .output()
        .map_err(|e| format!("run release-scan {args:?}: {e}"))
}

// ============================================================================
// Minimal pure-Rust SHA-256 + RFC 1952 GZIP + USTAR + AR + RPM v3 fixture builders
// ============================================================================

const SHA256_H0: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];
const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn sha256_hex(input: &[u8]) -> String {
    let mut state = SHA256_H0;
    let mut padded = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (dst, chunk) in w.iter_mut().take(16).zip(block.as_chunks::<4>().0) {
            *dst = u32::from_be_bytes(*chunk);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h.wrapping_add(sum1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = sum0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, delta) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(delta);
        }
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for word in state {
        for b in word.to_be_bytes() {
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
    }
    out
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
        }
    }
    !crc
}

/// Encode raw bytes as a valid RFC 1952 `.gz` stream using DEFLATE stored blocks (method 8, BTYPE=00).
fn gzip_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 32);
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03]);
    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    } else {
        let chunks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (idx, chunk) in chunks.iter().enumerate() {
            let bfinal = u8::from(idx + 1 == chunks.len());
            let len = chunk.len() as u16;
            let nlen = !len;
            out.push(bfinal);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&nlen.to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out.extend_from_slice(&crc32_ieee(raw).to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out
}

/// Build a POSIX ustar `.tar` archive from `(path, bytes)` entries (allows arbitrary paths for tar-slip negative tests).
fn build_ustar_bytes(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    for &(name, data) in entries {
        let mut hdr = [0u8; 512];
        let name_bytes = name.as_bytes();
        if name_bytes.len() > 100 {
            return Err(format!("test tar path > 100 bytes: {name}"));
        }
        hdr[..name_bytes.len()].copy_from_slice(name_bytes);
        hdr[100..108].copy_from_slice(b"0000644\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        let size_octal = format!("{:011o}\0", data.len());
        hdr[124..136].copy_from_slice(size_octal.as_bytes());
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].fill(b' ');
        hdr[156] = b'0';
        hdr[257..263].copy_from_slice(b"ustar\0");
        hdr[263..265].copy_from_slice(b"00");
        let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
        let chk_str = format!("{chksum:06o}\0 ");
        hdr[148..156].copy_from_slice(chk_str.as_bytes());
        tar.extend_from_slice(&hdr);
        tar.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        tar.resize(tar.len() + pad, 0);
    }
    tar.resize(tar.len() + 1024, 0);
    Ok(tar)
}

/// Build an `!<arch>\n` `.deb` archive in memory (`trailing_slash = true` for SVR4/GNU `ar`,
/// `false` for BSD `ar`).
fn build_ar_deb(members: &[(&str, &[u8])], trailing_slash: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::from(&b"!<arch>\n"[..]);
    for &(name, payload) in members {
        let tagged = if trailing_slash { format!("{name}/") } else { name.to_owned() };
        if tagged.len() > 16 {
            return Err(format!("ar member name > 16 bytes: {tagged}"));
        }
        let hdr = format!("{tagged:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, "100644", payload.len());
        if hdr.len() != 60 {
            return Err(format!("invalid ar header length {}", hdr.len()));
        }
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(payload);
        if !payload.len().is_multiple_of(2) {
            out.push(b'\n');
        }
    }
    Ok(out)
}

/// Build a SVR4/GNU `!<arch>\n` `.deb` archive (with trailing `/` member names, matching `nfpm` / GNU binutils).
fn build_gnu_ar_deb(members: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    build_ar_deb(members, true)
}

/// Build a pure-Rust `.deb` in memory with either BSD or GNU `ar` headers, avoiding subprocess spawns.
fn build_pure_deb(control_txt: &str, data_entries: &[(&str, &[u8])], trailing_slash: bool) -> Result<Vec<u8>, String> {
    let ctrl_tgz = gzip_stored(&build_ustar_bytes(&[("control", control_txt.as_bytes())])?);
    let data_tgz = gzip_stored(&build_ustar_bytes(data_entries)?);
    build_ar_deb(&[("debian-binary", b"2.0\n"), ("control.tar.gz", &ctrl_tgz), ("data.tar.gz", &data_tgz)], trailing_slash)
}

/// Build a `.deb` at test time using host `ar` + `tar` when present (falling back to pure-Rust ustar+ar on Windows).
fn build_deb_via_host_ar_and_tar(scratch: &Path, deb_out: &Path, control_txt: &str, data_entries: &[(&str, &[u8])]) -> Result<(), String> {
    #[cfg(unix)]
    if Path::new("/usr/bin/ar").is_file() && Path::new("/usr/bin/tar").is_file() {
        let work = scratch.join("deb-build");
        let _ = fs::remove_dir_all(&work);
        let ctrl_dir = work.join("ctrl");
        let data_dir = work.join("data");
        fs::create_dir_all(&ctrl_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

        fs::write(work.join("debian-binary"), b"2.0\n").map_err(|e| e.to_string())?;
        fs::write(ctrl_dir.join("control"), control_txt.as_bytes()).map_err(|e| e.to_string())?;
        for &(rel, bytes) in data_entries {
            let p = data_dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(p, bytes).map_err(|e| e.to_string())?;
        }

        let ctrl_tgz_str = work.join("control.tar.gz").to_string_lossy().into_owned();
        let data_tgz_str = work.join("data.tar.gz").to_string_lossy().into_owned();
        let ctrl_dir_str = ctrl_dir.to_string_lossy().into_owned();
        let data_dir_str = data_dir.to_string_lossy().into_owned();
        let deb_out_str = deb_out.to_string_lossy().into_owned();

        let st_ctrl = Command::new("/usr/bin/tar")
            .args(["-czf", ctrl_tgz_str.as_str(), "-C", ctrl_dir_str.as_str(), "."])
            .status()
            .map_err(|e| e.to_string())?;
        if !st_ctrl.success() {
            return Err(String::from("tar control.tar.gz failed"));
        }
        let st_data = Command::new("/usr/bin/tar")
            .args(["-czf", data_tgz_str.as_str(), "-C", data_dir_str.as_str(), "."])
            .status()
            .map_err(|e| e.to_string())?;
        if !st_data.success() {
            return Err(String::from("tar data.tar.gz failed"));
        }

        // `-rcS` avoids BSD `ranlib` `__.SYMDEF` injection on macOS while remaining portable to GNU `ar` on Linux.
        let st_ar = Command::new("/usr/bin/ar")
            .current_dir(&work)
            .args(["-rcS", deb_out_str.as_str(), "debian-binary", "control.tar.gz", "data.tar.gz"])
            .status()
            .map_err(|e| e.to_string())?;
        if !st_ar.success() {
            return Err(String::from("ar -rcS failed"));
        }
        return Ok(());
    }

    let ctrl_tgz = gzip_stored(&build_ustar_bytes(&[("control", control_txt.as_bytes())])?);
    let data_tgz = gzip_stored(&build_ustar_bytes(data_entries)?);
    let deb_bytes = build_gnu_ar_deb(&[("debian-binary", b"2.0\n"), ("control.tar.gz", &ctrl_tgz), ("data.tar.gz", &data_tgz)])?;
    fs::write(deb_out, deb_bytes).map_err(|e| e.to_string())
}

/// Build a synthetic RPM v3 package containing a gzipped SVR4 `newc` (`070701`) CPIO payload.
fn build_synthetic_rpm(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cpio = Vec::new();
    let mut ino = 100u32;
    let mut seen_dirs = std::collections::BTreeSet::new();

    for &(rel, _) in entries {
        let mut acc = String::new();
        let parts: Vec<&str> = rel.split('/').collect();
        for dir_seg in parts.iter().take(parts.len().saturating_sub(1)) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(dir_seg);
            if seen_dirs.insert(acc.clone()) {
                append_newc_entry(&mut cpio, ino, 0o040_755, 2, &acc, &[]);
                ino += 1;
            }
        }
    }
    for &(rel, bytes) in entries {
        append_newc_entry(&mut cpio, ino, 0o100_644, 1, rel, bytes);
        ino += 1;
    }
    append_newc_entry(&mut cpio, 0, 0, 1, "TRAILER!!!", &[]);

    let gz_cpio = gzip_stored(&cpio);
    let mut lead = [0u8; 96];
    lead[..4].copy_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
    lead[4] = 3;
    lead[5] = 0;
    lead[10..27].copy_from_slice(b"curvebook-0.1.0-1");
    lead[78..80].copy_from_slice(&5u16.to_be_bytes());

    let empty_rpm_header = [0x8e, 0xad, 0xe8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut rpm = Vec::with_capacity(128 + gz_cpio.len());
    rpm.extend_from_slice(&lead);
    rpm.extend_from_slice(&empty_rpm_header);
    rpm.extend_from_slice(&empty_rpm_header);
    rpm.extend_from_slice(&gz_cpio);
    rpm
}

fn append_newc_entry(out: &mut Vec<u8>, ino: u32, mode: u32, nlink: u32, name: &str, data: &[u8]) {
    let namesize = name.len() + 1;
    let hdr = format!(
        "070701{ino:08X}{mode:08X}{:08X}{:08X}{nlink:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{namesize:08X}{:08X}",
        0u32,
        0u32,
        0u32,
        data.len() as u32,
        0u32,
        0u32,
        0u32,
        0u32,
        0u32
    );
    out.extend_from_slice(hdr.as_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Craft a minimal stored ZIP archive WITHOUT path sanitization so we can test zip-slip rejection!
fn build_raw_stored_zip_unchecked(raw_name: &str, payload: &[u8]) -> Vec<u8> {
    let crc = crc32_ieee(payload);
    let len = payload.len() as u32;
    let name_bytes = raw_name.as_bytes();
    let name_len = name_bytes.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0x0800u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0x0021u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(payload);

    let cd_offset = out.len() as u32;
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&0x0314u16.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0x0800u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0x0021u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 14]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(name_bytes);
    let cd_size = (out.len() as u32) - cd_offset;

    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn write_sha256sums_for_dir(dir: &Path) -> Result<(), String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let name = entry.map_err(|e| e.to_string())?.file_name().to_string_lossy().into_owned();
        if name != "SHA256SUMS.txt" {
            names.push(name);
        }
    }
    names.sort();
    let mut lines = String::new();
    for name in names {
        let bytes = fs::read(dir.join(&name)).map_err(|e| e.to_string())?;
        lines.push_str(&format!("{}  {name}\n", sha256_hex(&bytes)));
    }
    fs::write(dir.join("SHA256SUMS.txt"), lines).map_err(|e| e.to_string())
}

/// The synthetic `.rpm` from [`build_synthetic_rpm`] has empty signature and main headers.
/// libarchive (`bsdtar`) reads its payload, but `rpm2cpio` from rpm 4.x rejects it
/// ("signature region: no tags"), so an `rpm2cpio`-only host cannot exercise these cases.
/// CI installs `bsdtar` (`libarchive-tools`) for the `tests` stage in `.github/workflows/ci.yml`.
fn host_can_unpack_rpm() -> bool {
    Command::new("bsdtar").arg("--version").output().is_ok_and(|o| o.status.success())
}

#[cfg(target_os = "macos")]
fn try_create_macos_dmg(src_folder: &Path, dmg_out: &Path) -> bool {
    let src_str = src_folder.to_string_lossy();
    let out_str = dmg_out.to_string_lossy();
    Command::new("/usr/bin/hdiutil")
        .args([
            "create",
            "-srcfolder",
            src_str.as_ref(),
            "-volname",
            "CurveBook",
            "-fs",
            "HFS+",
            "-format",
            "UDRO",
            "-nospotlight",
            "-ov",
            out_str.as_ref(),
        ])
        .output()
        .is_ok_and(|o| o.status.success() && dmg_out.is_file())
}

#[test]
fn r026_clean_release_directory_passes_sha256_and_all_container_scanners() -> Result<(), String> {
    let root = workspace_root()?;
    let fx = TempDirGuard::new("clean")?;
    let rel_dir = fx.path.join("release");
    fs::create_dir_all(&rel_dir).unwrap();

    let assets = CleanFixtures::shared(&root)?;

    // 1. Plain HTML compliance report
    fs::write(
        rel_dir.join("THIRD-PARTY-LICENSES.html"),
        "<!doctype html><html><head><meta name=\"generator\" content=\"cargo-about\"><title>CurveBook Third-Party Licenses</title></head><body><h1>SPDX License Report</h1><p>MIT OR Apache-2.0</p></body></html>\n",
    )
    .unwrap();

    // 2. Web & CLI `.zip` containers
    let mut web_zip = ZipWriter::new();
    web_zip
        .add_file("curvebook-web-0.1.0/index.html", b"<!doctype html><title>CurveBook</title><input accept=\".curvebook,.pzfx,.prism\">".to_vec())
        .unwrap();
    web_zip.add_file("curvebook-web-0.1.0/icons/32/org.curvebook.CurveBook.png", assets.icon_32.clone()).unwrap();
    web_zip.add_file("curvebook-web-0.1.0/icons/new_project.svg", assets.icon_svg.clone()).unwrap();
    web_zip.add_file("curvebook-web-0.1.0/ASSETS.md", assets.assets_md.clone()).unwrap();
    web_zip.add_file("curvebook-web-0.1.0/NOTICE", assets.notice.clone()).unwrap();
    fs::write(rel_dir.join("curvebook-web-0.1.0.zip"), web_zip.finish().unwrap()).unwrap();

    // 3. Linux/FreeBSD `.tar.gz` (including installed Freedesktop `usr/share/mime/packages/org.curvebook.CurveBook.xml`!)
    let tgz_bytes = gzip_stored(&build_ustar_bytes(&[
        ("curvebook-0.1.0-linux-x86_64/usr/bin/curvebook", b"\x7fELF_clean_binary_stub"),
        ("curvebook-0.1.0-linux-x86_64/usr/share/applications/org.curvebook.CurveBook.desktop", &assets.desktop_entry),
        ("curvebook-0.1.0-linux-x86_64/usr/share/mime/packages/org.curvebook.CurveBook.xml", &assets.mime_xml),
        ("curvebook-0.1.0-linux-x86_64/usr/share/icons/hicolor/32x32/apps/org.curvebook.CurveBook.png", &assets.icon_32),
        ("curvebook-0.1.0-linux-x86_64/usr/share/doc/curvebook/ASSETS.md", &assets.assets_md),
        ("curvebook-0.1.0-linux-x86_64/usr/share/doc/curvebook/NOTICE", &assets.notice),
    ])?);
    fs::write(rel_dir.join("curvebook-0.1.0-linux-x86_64.tar.gz"), tgz_bytes).unwrap();

    // 4. `.deb` built at test time via host `ar` + `tar` AND a second `.deb` with SVR4/GNU `/` ar member names
    let clean_ctrl = "Package: curvebook\nVersion: 0.1.0\nArchitecture: amd64\nMaintainer: CurveBook Contributors\nDescription: Clean-room biostatistics and graphing\n";
    let deb_entries: &[(&str, &[u8])] = &[
        ("usr/bin/curvebook", b"\x7fELF_clean_deb_stub"),
        ("usr/share/applications/org.curvebook.CurveBook.desktop", &assets.desktop_entry),
        ("usr/share/mime/packages/org.curvebook.CurveBook.xml", &assets.mime_xml),
        ("usr/share/icons/hicolor/32x32/apps/org.curvebook.CurveBook.png", &assets.icon_32),
        ("usr/share/doc/curvebook/NOTICE", &assets.notice),
    ];
    build_deb_via_host_ar_and_tar(&fx.path, &rel_dir.join("curvebook-0.1.0-linux-x86_64.deb"), clean_ctrl, deb_entries)?;

    let gnu_deb = build_pure_deb(clean_ctrl, deb_entries, true)?;
    fs::write(rel_dir.join("curvebook-0.1.0-linux-aarch64.deb"), gnu_deb).unwrap();

    // 5. `.rpm` built dynamically at test time when the host has `bsdtar`
    let mut expected_assets = 7usize; // SHA256SUMS.txt + html + zip + tar.gz + 2 debs + zsync
    if host_can_unpack_rpm() {
        let rpm_bytes = build_synthetic_rpm(&[
            ("usr/bin/curvebook", b"\x7fELF_clean_rpm_stub"),
            ("usr/share/applications/org.curvebook.CurveBook.desktop", &assets.desktop_entry),
            ("usr/share/icons/hicolor/32x32/apps/org.curvebook.CurveBook.png", &assets.icon_32),
            ("usr/share/doc/curvebook/NOTICE", &assets.notice),
        ]);
        fs::write(rel_dir.join("curvebook-0.1.0-linux-x86_64.rpm"), rpm_bytes).unwrap();
        expected_assets += 1;
    }

    // 6. `.AppImage.zsync` with ASCII metadata header + `\n\n` + raw non-UTF-8 binary rolling-checksum table
    let mut zsync_bytes = Vec::from(
        &b"zsync: 0.6.2\nFilename: curvebook-0.1.0-linux-x86_64.AppImage\nBlocksize: 2048\nLength: 65536\nHash-Lengths: 2,2,5\nURL: curvebook-0.1.0-linux-x86_64.AppImage\nSHA-1: 0123456789abcdef0123456789abcdef01234567\n\n"[..],
    );
    zsync_bytes.extend_from_slice(&[0xff, 0xfe, 0x80, 0x00, 0xde, 0xad, 0xbe, 0xef]);
    fs::write(rel_dir.join("curvebook-0.1.0-linux-x86_64.AppImage.zsync"), zsync_bytes).unwrap();

    // 7. macOS `.dmg` when the host kernel allows `hdiutil` loop-device creation (e.g. unsandboxed macOS runner)
    #[cfg(target_os = "macos")]
    {
        let dmg_src = fx.path.join("dmg-src");
        fs::create_dir_all(dmg_src.join("CurveBook.app/Contents/Resources")).unwrap();
        let plist = assets.plist_tpl.replace("@VERSION@", "0.1.0").replace("@SHORT_VERSION@", "0.1.0").replace("@BUILD_SHA@", "abc1234");
        fs::write(dmg_src.join("CurveBook.app/Contents/Info.plist"), plist).unwrap();
        fs::write(dmg_src.join("CurveBook.app/Contents/Resources/curvebook.icns"), &assets.icns).unwrap();
        std::os::unix::fs::symlink("/Applications", dmg_src.join("Applications")).unwrap();
        let dmg_out = rel_dir.join("curvebook-0.1.0-macos-universal.dmg");
        if try_create_macos_dmg(&dmg_src, &dmg_out) {
            expected_assets += 1;
        }
    }

    write_sha256sums_for_dir(&rel_dir)?;

    let report_path = fx.path.join("scan-report.json");
    let out = run_release_scan(&["--report", report_path.to_str().unwrap(), rel_dir.to_str().unwrap()])?;
    assert!(
        out.status.success(),
        "clean release-scan failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(report["ok"], serde_json::json!(true));
    assert_eq!(report["checksum_verified"], serde_json::json!(true));
    assert_eq!(report["asset_count"], serde_json::json!(expected_assets));
    assert!(report["total_unpacked_files"].as_u64().unwrap() >= 15);
    assert!(report["total_checked_assets"].as_u64().unwrap() >= 4);
    Ok(())
}

#[test]
fn r026_planted_brand_tokens_inside_each_container_fail_closed_naming_container() -> Result<(), String> {
    let fx = TempDirGuard::new("planted-brands")?;
    let dir = fx.path.join("planted-all");
    fs::create_dir_all(&dir).unwrap();

    let brand_gp = join(&["Gr", "aph", "Pad"]);
    let brand_op = join(&["Ori", "gin", "Pro"]);
    let brand_sp = join(&["SP", "SS"]);
    let brand_mt = join(&["Min", "it", "ab"]);
    let suite_ac = join(&["Art", "Cr", "aft"]);

    // 1. Planted token inside `.zip`
    let mut zw = ZipWriter::new();
    zw.add_file("curvebook-cli/README.md", format!("Compatibility wrapper for {brand_gp} scripts\n").into_bytes()).unwrap();
    fs::write(dir.join("curvebook-cli-0.1.0.zip"), zw.finish().unwrap()).unwrap();

    // 2. Planted token inside `.tar.gz`
    let tgz = gzip_stored(&build_ustar_bytes(&[(
        "curvebook-0.1.0-freebsd/share/doc/curvebook/README.md",
        format!("Replaces {brand_op} desktop installations\n").as_bytes(),
    )])?);
    fs::write(dir.join("curvebook-0.1.0-freebsd-x86_64.tar.gz"), tgz).unwrap();

    // 3. Planted token inside `.deb` `data.tar.gz`
    let ctrl = "Package: curvebook\nVersion: 0.1.0\nDescription: Clean control\n";
    let leak = format!("[Desktop Entry]\nName=CurveBook\nKeywords=biostatistics;{brand_sp};\n");
    let deb_data = build_pure_deb(ctrl, &[("usr/share/applications/org.curvebook.CurveBook.desktop", leak.as_bytes())], false)?;
    fs::write(dir.join("curvebook-0.1.0-amd64.deb"), deb_data).unwrap();

    // 4. Planted token inside `.deb` `control.tar.gz` metadata
    let dirty_ctrl = format!("Package: curvebook\nVersion: 0.1.0\nDescription: Suite tool by {suite_ac}\n");
    let deb_ctrl = build_pure_deb(&dirty_ctrl, &[("usr/share/doc/curvebook/README.txt", b"Clean payload\n")], true)?;
    fs::write(dir.join("curvebook-ctrl-leak.deb"), deb_ctrl).unwrap();

    // 5. Planted token inside `.rpm` (when the host has `bsdtar`)
    let has_rpm = host_can_unpack_rpm();
    if has_rpm {
        let rpm_leak = format!("Bundled {brand_mt} macro reference\n");
        let rpm = build_synthetic_rpm(&[("usr/share/doc/curvebook/NOTES.txt", rpm_leak.as_bytes())]);
        fs::write(dir.join("curvebook-0.1.0-1.x86_64.rpm"), rpm).unwrap();
    }

    // 6. Planted token inside `.AppImage.zsync` metadata header before binary block-hash trailer
    let mut zsync = format!("zsync: 0.6.2\nFilename: curvebook.AppImage\nURL: https://example.org/{suite_ac}/curvebook.AppImage\n\n").into_bytes();
    zsync.extend_from_slice(&[0xff, 0x00, 0xa5, 0x5a]);
    fs::write(dir.join("curvebook-0.1.0-linux-x86_64.AppImage.zsync"), zsync).unwrap();

    // 7. Planted token inside `THIRD-PARTY-LICENSES.html`
    fs::write(
        dir.join("THIRD-PARTY-LICENSES.html"),
        format!("<!doctype html><html><head><title>{brand_gp} Third-Party Notices</title></head><body>Clean body</body></html>\n"),
    )
    .unwrap();

    write_sha256sums_for_dir(&dir)?;

    let out = run_release_scan(&[dir.to_str().unwrap()])?;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("curvebook-cli-0.1.0.zip!curvebook-cli/README.md"), "{stderr}");
    assert!(stderr.contains("curvebook-0.1.0-freebsd-x86_64.tar.gz!"), "{stderr}");
    assert!(stderr.contains("curvebook-0.1.0-amd64.deb!"), "{stderr}");
    assert!(stderr.contains("curvebook-ctrl-leak.deb!_deb_control/"), "{stderr}");
    if has_rpm {
        assert!(stderr.contains("curvebook-0.1.0-1.x86_64.rpm!usr/share/doc/curvebook/NOTES.txt"), "{stderr}");
    }
    assert!(stderr.contains("curvebook-0.1.0-linux-x86_64.AppImage.zsync"), "{stderr}");
    assert!(stderr.contains("THIRD-PARTY-LICENSES.html"), "{stderr}");
    Ok(())
}

#[test]
fn r026_planted_cleanroom_copyright_and_unattributed_assets_or_fonts_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("cleanroom-assets")?;
    let dir = fx.path.join("planted-all");
    fs::create_dir_all(&dir).unwrap();
    let vendor_dm = join(&["Dot", "mat", "ics"]);

    // 1. Vendor copyright header inside `.deb` payload caught by `cleanroom`
    let notice_leak = format!("<!-- \u{00a9} 2025 {vendor_dm}. All rights reserved. -->\n");
    let deb = build_pure_deb("Package: curvebook\nVersion: 0.1.0\n", &[("usr/share/doc/curvebook/sample.xml", notice_leak.as_bytes())], false)?;
    fs::write(dir.join("curvebook-vendor-notice.deb"), deb).unwrap();

    // 2. Unattributed `.svg` visual asset inside `.tar.gz` caught by `assets`
    let tgz = gzip_stored(&build_ustar_bytes(&[(
        "curvebook-0.1.0/usr/share/icons/rogue-unattributed.svg",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle cx=\"8\" cy=\"8\" r=\"4\"/></svg>",
    )])?);
    fs::write(dir.join("curvebook-rogue-asset.tar.gz"), tgz).unwrap();

    // 3. Bundled `.ttf` font inside `.zip` caught by zero-bundled-fonts policy
    let mut zw = ZipWriter::new();
    zw.add_file("curvebook-web/fonts/Forbidden.ttf", vec![0x00, 0x01, 0x00, 0x00, 0x00]).unwrap();
    fs::write(dir.join("curvebook-web-font.zip"), zw.finish().unwrap()).unwrap();

    write_sha256sums_for_dir(&dir)?;

    let out = run_release_scan(&[dir.to_str().unwrap()])?;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("curvebook-vendor-notice.deb!") && stderr.contains("[cleanroom]"), "{stderr}");
    assert!(stderr.contains("curvebook-rogue-asset.tar.gz!") && stderr.contains("unattributed asset"), "{stderr}");
    assert!(stderr.contains("curvebook-web-font.zip!") && stderr.contains("zero-bundled-fonts"), "{stderr}");
    Ok(())
}

#[test]
fn r026_sha256sums_tampered_missing_extra_and_absent_manifest_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("checksums")?;
    let mut zw = ZipWriter::new();
    zw.add_file("curvebook-cli/README.txt", b"Clean CurveBook CLI\n".to_vec()).unwrap();
    let valid_zip = zw.finish().unwrap();

    // 1. Tampered byte after SHA256SUMS.txt generation -> fails closed
    {
        let dir = fx.path.join("tampered");
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("curvebook-cli.zip");
        fs::write(&zip_path, &valid_zip).unwrap();
        write_sha256sums_for_dir(&dir)?;
        let mut tampered = valid_zip.clone();
        tampered[0] ^= 0xff;
        fs::write(&zip_path, tampered).unwrap();

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("SHA-256 mismatch"), "{}", String::from_utf8_lossy(&out.stderr));
    }

    // 2. Sibling asset missing from SHA256SUMS.txt -> fails closed
    {
        let dir = fx.path.join("missing-entry");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("curvebook-a.zip"), &valid_zip).unwrap();
        write_sha256sums_for_dir(&dir)?;
        fs::write(dir.join("curvebook-b.zip"), &valid_zip).unwrap();

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("curvebook-b.zip") && stderr.contains("missing from `SHA256SUMS.txt`"), "{stderr}");
    }

    // 3. Extra nonexistent entry in SHA256SUMS.txt -> fails closed
    {
        let dir = fx.path.join("extra-entry");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("curvebook-a.zip"), &valid_zip).unwrap();
        write_sha256sums_for_dir(&dir)?;
        let mut sums = fs::read_to_string(dir.join("SHA256SUMS.txt")).unwrap();
        sums.push_str(&format!("{}  curvebook-ghost.zip\n", "0".repeat(64)));
        fs::write(dir.join("SHA256SUMS.txt"), sums).unwrap();

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("curvebook-ghost.zip") && stderr.contains("does not exist"), "{stderr}");
    }

    // 4. Absent SHA256SUMS.txt fails by default, passes only with explicit `--no-checksums`
    {
        let dir = fx.path.join("absent-sums");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("curvebook-a.zip"), &valid_zip).unwrap();

        let default_out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!default_out.status.success());
        assert!(String::from_utf8_lossy(&default_out.stderr).contains("SHA256SUMS.txt"));

        let opt_out = run_release_scan(&["--no-checksums", dir.to_str().unwrap()])?;
        assert!(opt_out.status.success(), "{}", String::from_utf8_lossy(&opt_out.stderr));
    }
    Ok(())
}

#[test]
fn r026_unknown_extensions_empty_dir_path_traversal_and_unextractable_containers_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("fail-closed")?;

    // 1. Empty directory
    let empty_dir = fx.path.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let empty_out = run_release_scan(&[empty_dir.to_str().unwrap()])?;
    assert!(!empty_out.status.success());
    assert!(String::from_utf8_lossy(&empty_out.stderr).contains("empty"));

    // 2. Unknown / unsupported asset extensions (.pkg, .exe, .rar)
    for unknown_name in ["curvebook-mystery.pkg", "curvebook-setup.exe", "curvebook-archive.rar"] {
        let dir = fx.path.join(format!("unknown-{unknown_name}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(unknown_name), b"opaque installer payload").unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success(), "{unknown_name} must fail closed");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(unknown_name) && stderr.contains("unsupported or unknown release asset format"), "{stderr}");
    }

    // 3. Zip-slip (`../escape.txt`) inside `.zip`
    {
        let dir = fx.path.join("zip-slip");
        fs::create_dir_all(&dir).unwrap();
        let evil_zip = build_raw_stored_zip_unchecked("../escape.txt", b"escaped");
        fs::write(dir.join("curvebook-slip.zip"), evil_zip).unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("curvebook-slip.zip"));
    }

    // 4. Tar-slip (`../escape.txt`) inside `.tar.gz`
    {
        let dir = fx.path.join("tar-slip");
        fs::create_dir_all(&dir).unwrap();
        let evil_tgz = gzip_stored(&build_ustar_bytes(&[("../escape.txt", b"escaped")])?);
        fs::write(dir.join("curvebook-slip.tar.gz"), evil_tgz).unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("curvebook-slip.tar.gz") && stderr.contains("../escape.txt"), "{stderr}");
    }

    // 5. `.deb` missing `data.tar.*` fails closed
    {
        let dir = fx.path.join("deb-no-data");
        fs::create_dir_all(&dir).unwrap();
        let ctrl_only = build_gnu_ar_deb(&[("debian-binary", b"2.0\n")])?;
        fs::write(dir.join("curvebook-broken.deb"), ctrl_only).unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("curvebook-broken.deb"));
    }

    // 6. Unextractable `.AppImage`, `.flatpak`, and cross-OS `.msi` / `.dmg` fail closed (never skipped)
    for (fname, dummy_bytes) in [
        ("curvebook-broken.AppImage", &b"\x7fELF_without_squashfs_superblock"[..]),
        ("curvebook-broken.flatpak", &b"not-a-valid-ostree-flatpak-bundle"[..]),
    ] {
        let dir = fx.path.join(format!("broken-{fname}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(fname), dummy_bytes).unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success(), "{fname} must fail closed");
        assert!(String::from_utf8_lossy(&out.stderr).contains(fname));
    }

    #[cfg(not(windows))]
    {
        let dir = fx.path.join("non-win-msi");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("curvebook-0.1.0-windows-x64.msi"), b"msi-placeholder").unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("curvebook-0.1.0-windows-x64.msi"));
    }

    #[cfg(target_os = "macos")]
    {
        // Even on macOS, an invalid/unmountable `.dmg` (or when sandbox blocks IOKit diskimage attach) must fail closed.
        let dir = fx.path.join("bad-dmg");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("curvebook-corrupt.dmg"), b"not-a-valid-udif-dmg").unwrap();
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("curvebook-corrupt.dmg"));
    }
    Ok(())
}
