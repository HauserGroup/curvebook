//! Store submission packaging. No signing credentials are read or required.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use curvebook_platform::process::ProcessCommand;
use curvebook_platform::write_atomic;

const USAGE: &str = "usage: cargo xtask msix --arch <x64|x86|arm64> --package-version <A.B.C.0>";
const TEMPLATE: &str = include_str!("../../packaging/windows/AppxManifest.xml.in");
const IDENTITY: &str = include_str!("../../packaging/windows/store-identity.json");

struct Options {
    arch: String,
    version: String,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let (mut arch, mut version) = (None, None);
        let (pairs, remainder) = args.as_chunks::<2>();
        for [key, value] in pairs {
            match key.as_str() {
                "--arch" if arch.is_none() => arch = Some(value.clone()),
                "--package-version" if version.is_none() => version = Some(value.clone()),
                _ => return Err(USAGE.to_owned()),
            }
        }
        if !remainder.is_empty() {
            return Err(USAGE.to_owned());
        }
        let result = Self { arch: arch.ok_or(USAGE)?, version: version.ok_or(USAGE)? };
        result.target()?;
        validate_version(&result.version)?;
        Ok(result)
    }

    fn target(&self) -> Result<(&'static str, u16), String> {
        match self.arch.as_str() {
            "x64" => Ok(("x86_64-pc-windows-msvc", 0x8664)),
            "x86" => Ok(("i686-pc-windows-msvc", 0x014c)),
            "arm64" => Ok(("aarch64-pc-windows-msvc", 0xaa64)),
            _ => Err(USAGE.to_owned()),
        }
    }
}

fn validate_version(version: &str) -> Result<(), String> {
    let parts: Vec<&str> = version.split('.').collect();
    let numbers: Result<Vec<u16>, _> = parts.iter().map(|p| p.parse::<u16>()).collect();
    if parts.len() != 4
        || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || (p.len() > 1 && p.starts_with('0')))
        || !numbers.is_ok_and(|v| v.first().is_some_and(|n| *n > 0) && v.last() == Some(&0))
    {
        return Err(String::from(
            "Store version must be A.B.C.0: four integers in 0..=65535, A >= 1, no leading zeros; increase it for each Store update",
        ));
    }
    Ok(())
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn manifest(options: &Options) -> Result<String, String> {
    let identity: serde_json::Value = serde_json::from_str(IDENTITY).map_err(|e| format!("Store identity: {e}"))?;
    let mut xml = TEMPLATE.replace("@ARCH@", &options.arch).replace("@VERSION@", &options.version);
    for (key, token) in [("name", "@NAME@"), ("publisher", "@PUBLISHER@"), ("publisher_display_name", "@PUBLISHER_DISPLAY_NAME@")] {
        let value = identity
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("missing Store identity {key}"))?;
        xml = xml.replace(token, &escape_xml(value));
    }
    Ok(xml)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    write(destination, &std::fs::read(source).map_err(|e| format!("read {}: {e}", source.display()))?)
}

fn validate_executable(bytes: &[u8], machine: u16) -> Result<(), String> {
    let offset = bytes.get(0x3c..0x40).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes);
    let pe = offset.and_then(|n| usize::try_from(n).ok()).and_then(|n| bytes.get(n..)).ok_or("missing PE header")?;
    let found = pe.get(4..6).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_le_bytes);
    if bytes.get(..2) != Some(b"MZ") || pe.get(..4) != Some(b"PE\0\0") || found != Some(machine) {
        return Err(String::from("not a Windows executable for the requested architecture"));
    }
    Ok(())
}

fn stage(root: &Path, directory: &Path, binary_dir: &Path, options: &Options) -> Result<(), String> {
    let assets = directory.join("Assets");
    std::fs::create_dir_all(&assets).map_err(|e| format!("create MSIX staging: {e}"))?;
    for name in ["curvebook.exe", "curvebook-cli.exe"] {
        let path = binary_dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("read {} (build both binaries for the requested target first): {e}", path.display()))?;
        validate_executable(&bytes, options.target()?.1).map_err(|e| format!("{}: {e}", path.display()))?;
        write(&directory.join(name), &bytes)?;
    }
    let icon_bytes = std::fs::read(root.join("assets/app-icon/curvebook-1024.png")).map_err(|e| format!("read icon: {e}"))?;
    let (src_w, src_h, src_rgba) = decode_png_rgba8(&icon_bytes)?;
    for (name, size) in [("StoreLogo.png", 50), ("Square150x150Logo.png", 150), ("Square44x44Logo.png", 44)] {
        write(&assets.join(name), &encode_png_rgba8(size, size, &downscale_rgba_lanczos3(&src_rgba, src_w, src_h, size, size)?)?)?;
    }
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md", "ATTRIBUTION.md"] {
        copy(&root.join(name), &directory.join(name))?;
    }
    copy(&root.join("licenses/THIRD-PARTY-LICENSES.html"), &directory.join("THIRD-PARTY-LICENSES.html"))?;
    write(&directory.join("AppxManifest.xml"), manifest(options)?.as_bytes())
}

fn makeappx() -> Result<PathBuf, String> {
    let pf = std::env::var_os("ProgramFiles(x86)").ok_or("missing ProgramFiles(x86); run this command on Windows with the Windows SDK installed")?;
    let bin = PathBuf::from(pf).join("Windows Kits/10/bin");
    let mut tools = Vec::new();
    for entry in std::fs::read_dir(&bin).map_err(|e| format!("read {}: {e}", bin.display()))? {
        let entry = entry.map_err(|e| format!("SDK directory: {e}"))?;
        let path = entry.path().join("x64/MakeAppx.exe");
        if let Ok(ver) = entry.file_name().to_string_lossy().split('.').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>()
            && path.is_file()
        {
            tools.push((ver, path));
        }
    }
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    tools.pop().map(|(_, p)| p).ok_or_else(|| format!("MakeAppx.exe not found under {}", bin.display()))
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = Options::parse(args)?;
    if !cfg!(windows) {
        return Err(String::from("MSIX packaging requires Windows SDK tools; use the Windows Store package GitHub workflow"));
    }
    let tool = makeappx()?;
    let version = crate::version::read_workspace_version(&std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?)?;
    let output_dir = root.join("dist/msix");
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("create output directory: {e}"))?;
    let scratch = output_dir.join(format!(".stage-{}-{}", options.arch, std::process::id()));
    std::fs::create_dir(&scratch).map_err(|e| format!("create {}: {e}", scratch.display()))?;
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(|| root.join("target"), |p| if p.is_absolute() { p } else { root.join(p) });
    let result = (|| {
        let (payload, packed) = (scratch.join("payload"), scratch.join("submission.msix"));
        stage(root, &payload, &target_dir.join(options.target()?.0).join("release"), &options)?;
        let out = ProcessCommand::new(tool.to_string_lossy())
            .args(["pack", "/d", &payload.to_string_lossy(), "/p", &packed.to_string_lossy(), "/o"])
            .wall_timeout(Duration::from_secs(300))
            .idle_timeout(Duration::from_secs(300))
            .run()
            .map_err(|e| format!("MakeAppx pack: {e}"))?;
        println!("{}", String::from_utf8_lossy(&out.stdout));
        if !out.success {
            return Err(format!("MakeAppx pack failed ({:?}): {}", out.status_code, String::from_utf8_lossy(&out.stderr)));
        }
        let name = format!("curvebook-{version}-windows-{}-store.msix", options.arch);
        copy(&packed, &output_dir.join(&name))?;
        copy(&payload.join("AppxManifest.xml"), &output_dir.join(format!("AppxManifest-{}.xml", options.arch)))?;
        println!(
            "Store submission: {} (package version {}). Unsigned: Microsoft signs it after certification.",
            output_dir.join(name).display(),
            options.version
        );
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(&scratch).map_err(|e| format!("remove MSIX staging: {e}")))
}

fn decode_png_rgba8(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("decode icon header: {e}"))?;
    let mut raw = vec![0u8; reader.output_buffer_size().ok_or_else(|| String::from("icon buffer size overflow"))?];
    let info = reader.next_frame(&mut raw).map_err(|e| format!("decode icon pixels: {e}"))?;
    let src = raw.get(..info.buffer_size()).ok_or_else(|| String::from("decoded pixel frame out of bounds"))?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => src.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("unsupported icon PNG color type {other:?}")),
    };
    Ok((info.width, info.height, rgba))
}

fn encode_png_rgba8(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| format!("encode icon header: {e}"))?;
    w.write_image_data(rgba).map_err(|e| format!("encode icon pixels: {e}"))?;
    w.finish().map_err(|e| format!("finish icon png: {e}"))?;
    Ok(out)
}

fn lanczos3(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 1e-12 {
        1.0
    } else if ax >= 3.0 {
        0.0
    } else {
        let px = std::f64::consts::PI * ax;
        (px.sin() / px) * ((px / 3.0).sin() / (px / 3.0))
    }
}

fn downscale_rgba_lanczos3(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Result<Vec<u8>, String> {
    let (sw, sh, dw, dh) = (src_w as usize, src_h as usize, dst_w as usize, dst_h as usize);
    let exp = sw.checked_mul(sh).and_then(|p| p.checked_mul(4)).ok_or_else(|| String::from("source dimensions overflow"))?;
    if src.len() != exp || sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Err(String::from("invalid icon dimensions for resize"));
    }
    let (sx_r, sy_r) = (sw as f64 / dw as f64, sh as f64 / dh as f64);
    let (sup_x, sup_y, inv_sx, inv_sy) = (3.0 * sx_r.max(1.0), 3.0 * sy_r.max(1.0), sx_r.max(1.0).recip(), sy_r.max(1.0).recip());
    let mut horiz = vec![0.0_f64; dw * sh * 4];
    for y in 0..sh {
        for x in 0..dw {
            let center = (x as f64 + 0.5) * sx_r - 0.5;
            let (lo, hi) = ((center - sup_x).floor().max(0.0) as usize, ((center + sup_x).ceil() as isize).min(sw as isize - 1).max(0) as usize);
            let (mut acc, mut wsum) = ([0.0; 4], 0.0);
            for ix in lo..=hi {
                let w = lanczos3((ix as f64 - center) * inv_sx);
                let px = src.get((y * sw + ix) * 4..(y * sw + ix) * 4 + 4).ok_or_else(|| String::from("source pixel out of bounds"))?;
                let pa = f64::from(px[3]) / 255.0;
                for c in 0..3 {
                    acc[c] += f64::from(px[c]) * pa * w;
                }
                acc[3] += pa * w;
                wsum += w;
            }
            if wsum.abs() > 1e-12
                && let Some(dst) = horiz.get_mut((y * dw + x) * 4..(y * dw + x) * 4 + 4)
            {
                for c in 0..4 {
                    dst[c] = acc[c] / wsum;
                }
            }
        }
    }
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let center = (y as f64 + 0.5) * sy_r - 0.5;
        let (lo, hi) = ((center - sup_y).floor().max(0.0) as usize, ((center + sup_y).ceil() as isize).min(sh as isize - 1).max(0) as usize);
        for x in 0..dw {
            let (mut acc, mut wsum) = ([0.0; 4], 0.0);
            for iy in lo..=hi {
                let w = lanczos3((iy as f64 - center) * inv_sy);
                let px = horiz.get((iy * dw + x) * 4..(iy * dw + x) * 4 + 4).ok_or_else(|| String::from("intermediate pixel out of bounds"))?;
                for c in 0..4 {
                    acc[c] += px[c] * w;
                }
                wsum += w;
            }
            if let Some(dst) = out.get_mut((y * dw + x) * 4..(y * dw + x) * 4 + 4) {
                let norm = if wsum.abs() > 1e-12 { wsum } else { 1.0 };
                let alpha = (acc[3] / norm).clamp(0.0, 1.0);
                for c in 0..3 {
                    dst[c] = if alpha > 1e-6 { ((acc[c] / norm / alpha).clamp(0.0, 255.0)).round() as u8 } else { 0 };
                }
                dst[3] = (alpha * 255.0).clamp(0.0, 255.0).round() as u8;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Options, escape_xml, manifest, stage, validate_executable, validate_version};
    use crate::scan_util::test_support::TestDir;

    fn dummy_pe(machine: u16) -> Vec<u8> {
        let mut pe = vec![0; 128];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        pe[64..68].copy_from_slice(b"PE\0\0");
        pe[68..70].copy_from_slice(&machine.to_le_bytes());
        pe
    }

    #[test]
    fn validates_store_versions_args_pe_and_stages_package_payload() {
        for bad in ["0.1.0.0", "1.0.0.1", "1.0.0-alpha.2", "1.0.0", "65536.0.0.0", "01.0.0.0", "+1.0.0.0", "1.0.0.0/other"] {
            assert!(validate_version(bad).is_err(), "{bad}");
        }
        for good in ["1.0.2.0", "1.0.3.0", "65535.65535.65535.0"] {
            validate_version(good).unwrap();
        }
        for args in [vec![], vec!["--arch", "x64"], vec!["--arch", "x64", "--package-version", "1.0.2.0", "--arch", "arm64"]] {
            assert!(Options::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
        let mut pe = dummy_pe(0x8664);
        assert!(validate_executable(&[], 0x8664).is_err() && validate_executable(&pe, 0x8664).is_ok() && validate_executable(&pe, 0xaa64).is_err());
        pe[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_executable(&pe, 0x8664).is_err());

        let (source_root, temp) = (std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap(), TestDir::new("msix-stage"));
        let root = temp.join("repo");
        std::fs::create_dir_all(root.join("assets/app-icon")).unwrap();
        for rel in ["assets/app-icon/curvebook-1024.png", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md", "ATTRIBUTION.md"] {
            std::fs::copy(source_root.join(rel), root.join(rel)).unwrap();
        }
        temp.write("repo/licenses/THIRD-PARTY-LICENSES.html", "<p>Package staging test fixture</p>");
        let valid_pe = dummy_pe(0x8664);
        for name in ["curvebook.exe", "curvebook-cli.exe", "should-not-ship.pfx"] {
            temp.write_bytes(&format!("binaries/{name}"), &valid_pe);
        }
        let (options, payload) = (Options { arch: "x64".into(), version: "1.0.2.0".into() }, temp.join("payload"));
        stage(&root, &payload, &temp.join("binaries"), &options).unwrap();
        let xml = manifest(&options).unwrap();
        for needle in [
            "Name=\"JakobSture.CurveBook\"",
            "Publisher=\"CN=8373FC7D-82BB-4F48-BE69-7A5260B5630A\"",
            "<PublisherDisplayName>Jakob Sture</PublisherDisplayName>",
            "Alias=\"curvebook-cli.exe\"",
        ] {
            assert!(xml.contains(needle), "missing {needle}");
        }
        assert!(!xml.contains('@') && !payload.join("should-not-ship.pfx").exists());
        for (name, exp) in [("StoreLogo.png", 50), ("Square150x150Logo.png", 150), ("Square44x44Logo.png", 44)] {
            let info = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(payload.join("Assets").join(name)).unwrap()))
                .read_info()
                .unwrap()
                .info()
                .clone();
            assert_eq!((info.width, info.height), (exp, exp));
        }
        assert_eq!(escape_xml("<&\"'>"), "&lt;&amp;&quot;&apos;&gt;");
    }
}
