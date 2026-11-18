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
        let mut arch = None;
        let mut version = None;
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
    let numbers: Result<Vec<u16>, _> = parts.iter().map(|part| part.parse::<u16>()).collect();
    if parts.len() != 4
        || parts.iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) || (part.len() > 1 && part.starts_with('0')))
        || !numbers.is_ok_and(|values| values.first().is_some_and(|v| *v > 0) && values.last() == Some(&0))
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
    let bytes = std::fs::read(source).map_err(|e| format!("read {}: {e}", source.display()))?;
    write(destination, &bytes)
}

/// Reject wrong-architecture or truncated inputs before invoking SDK packaging tools.
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
    // Render package-size raster variants from the existing attributed raster icon.
    // No SVG tools, new source artwork, or bundled fonts are involved.
    let icon_bytes = std::fs::read(root.join("assets/app-icon/curvebook-1024.png")).map_err(|e| format!("read icon: {e}"))?;
    let icon = image::load_from_memory_with_format(&icon_bytes, image::ImageFormat::Png).map_err(|e| format!("decode icon: {e}"))?;
    for (name, size) in [("StoreLogo.png", 50), ("Square150x150Logo.png", 150), ("Square44x44Logo.png", 44)] {
        let rendered = icon.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        let mut encoded = Cursor::new(Vec::new());
        rendered.write_to(&mut encoded, image::ImageFormat::Png).map_err(|e| format!("encode logo: {e}"))?;
        write(&assets.join(name), &encoded.into_inner())?;
    }
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md", "ATTRIBUTION.md"] {
        copy(&root.join(name), &directory.join(name))?;
    }
    copy(&root.join("licenses/THIRD-PARTY-LICENSES.html"), &directory.join("THIRD-PARTY-LICENSES.html"))?;
    write(&directory.join("AppxManifest.xml"), manifest(options)?.as_bytes())
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).map_or_else(|| root.join("target"), |p| if p.is_absolute() { p } else { root.join(p) })
}

fn makeappx() -> Result<PathBuf, String> {
    let program_files =
        std::env::var_os("ProgramFiles(x86)").ok_or("missing ProgramFiles(x86); run this command on Windows with the Windows SDK installed")?;
    let bin = PathBuf::from(program_files).join("Windows Kits/10/bin");
    let entries = std::fs::read_dir(&bin).map_err(|e| format!("read {}: {e}", bin.display()))?;
    let mut tools = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("SDK directory: {e}"))?;
        let version: Result<Vec<u32>, _> = entry.file_name().to_string_lossy().split('.').map(str::parse).collect();
        let path = entry.path().join("x64/MakeAppx.exe");
        if let Ok(version) = version
            && path.is_file()
        {
            tools.push((version, path));
        }
    }
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    tools.pop().map(|(_, path)| path).ok_or_else(|| format!("MakeAppx.exe not found under {}", bin.display()))
}

fn pack(tool: &Path, staging: &Path, package: &Path) -> Result<(), String> {
    let output = ProcessCommand::new(tool.to_string_lossy())
        .args(["pack", "/d", &staging.to_string_lossy(), "/p", &package.to_string_lossy(), "/o"])
        .wall_timeout(Duration::from_secs(300))
        .idle_timeout(Duration::from_secs(300))
        .run()
        .map_err(|e| format!("MakeAppx pack: {e}"))?;
    println!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.success {
        return Err(format!("MakeAppx pack failed ({:?}): {}", output.status_code, String::from_utf8_lossy(&output.stderr)));
    }
    Ok(())
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = Options::parse(args)?;
    if !cfg!(windows) {
        return Err(String::from("MSIX packaging requires Windows SDK tools; use the Windows Store package GitHub workflow"));
    }
    let tool = makeappx()?;
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let version = crate::version::read_workspace_version(&cargo_toml)?;
    let output_dir = root.join("dist/msix");
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("create output directory: {e}"))?;
    let scratch = output_dir.join(format!(".stage-{}-{}", options.arch, std::process::id()));
    std::fs::create_dir(&scratch).map_err(|e| format!("create {}: {e}", scratch.display()))?;
    let result = (|| {
        let payload = scratch.join("payload");
        let binaries = target_dir(root).join(options.target()?.0).join("release");
        stage(root, &payload, &binaries, &options)?;
        let packed = scratch.join("submission.msix");
        pack(&tool, &payload, &packed)?;
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
    let cleanup = std::fs::remove_dir_all(&scratch).map_err(|e| format!("remove MSIX staging: {e}"));
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::{Options, escape_xml, manifest, stage, validate_executable, validate_version};

    #[test]
    fn rejects_store_versions_that_cannot_be_submitted() {
        for bad in ["0.1.0.0", "1.0.0.1", "1.0.0-alpha.2", "1.0.0", "65536.0.0.0", "01.0.0.0", "+1.0.0.0", "1.0.0.0/other"] {
            assert!(validate_version(bad).is_err(), "accepted {bad}");
        }
        for good in ["1.0.2.0", "1.0.3.0", "65535.65535.65535.0"] {
            validate_version(good).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_arguments_and_pe_architectures() {
        for args in [vec![], vec!["--arch", "x64"], vec!["--arch", "x64", "--package-version", "1.0.2.0", "--arch", "arm64"]] {
            assert!(Options::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
        assert!(validate_executable(&[], 0x8664).is_err());
        let mut pe = vec![0; 128];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        pe[64..68].copy_from_slice(b"PE\0\0");
        pe[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        validate_executable(&pe, 0x8664).unwrap();
        assert!(validate_executable(&pe, 0xaa64).is_err());
        pe[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_executable(&pe, 0x8664).is_err());
    }

    #[test]
    fn stages_only_package_payload_with_correct_identity_and_logo_sizes() {
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let temp = std::env::temp_dir().join(format!("cb-msix-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let result = (|| {
            let root = temp.join("repo");
            std::fs::create_dir_all(root.join("assets/app-icon")).unwrap();
            std::fs::create_dir_all(root.join("licenses")).unwrap();
            std::fs::copy(source_root.join("assets/app-icon/curvebook-1024.png"), root.join("assets/app-icon/curvebook-1024.png")).unwrap();
            for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md", "ATTRIBUTION.md"] {
                std::fs::copy(source_root.join(name), root.join(name)).unwrap();
            }
            std::fs::write(root.join("licenses/THIRD-PARTY-LICENSES.html"), "<p>Package staging test fixture</p>").unwrap();
            let binary_dir = temp.join("binaries");
            std::fs::create_dir_all(&binary_dir).unwrap();
            let mut pe = vec![0; 128];
            pe[..2].copy_from_slice(b"MZ");
            pe[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
            pe[64..68].copy_from_slice(b"PE\0\0");
            pe[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
            for name in ["curvebook.exe", "curvebook-cli.exe", "should-not-ship.pfx"] {
                std::fs::write(binary_dir.join(name), &pe).unwrap();
            }
            let options = Options { arch: "x64".into(), version: "1.0.2.0".into() };
            let payload = temp.join("payload");
            stage(&root, &payload, &binary_dir, &options)?;
            let xml = manifest(&options)?;
            assert!(xml.contains("Name=\"JakobSture.CurveBook\""));
            assert!(xml.contains("Publisher=\"CN=8373FC7D-82BB-4F48-BE69-7A5260B5630A\""));
            assert!(xml.contains("<PublisherDisplayName>Jakob Sture</PublisherDisplayName>"));
            assert!(!xml.contains('@'));
            assert!(xml.contains("Alias=\"curvebook-cli.exe\""));
            assert!(!payload.join("should-not-ship.pfx").exists());
            for (name, expected) in [("StoreLogo.png", 50), ("Square150x150Logo.png", 150), ("Square44x44Logo.png", 44)] {
                let (width, height) = image::image_dimensions(payload.join("Assets").join(name)).unwrap();
                assert_eq!((width, height), (expected, expected));
            }
            assert_eq!(escape_xml("<&\"'>"), "&lt;&amp;&quot;&apos;&gt;");
            Ok::<(), String>(())
        })();
        std::fs::remove_dir_all(&temp).unwrap();
        result.unwrap();
    }
}
