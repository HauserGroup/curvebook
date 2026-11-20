//! Build-time hook for `curvebook` desktop executable metadata (`CURVEBOOK_BUILD_SHA`, Windows
//! icon/VERSIONINFO).
//!
//! On non-Windows targets this emits only rerun directives. On Windows targets, `winresource`
//! embeds `assets/app-icon/curvebook.ico` and `VERSIONINFO` into `curvebook.exe`; a missing
//! resource compiler is a warning unless `CURVEBOOK_REQUIRE_WINRES=1` promotes it to an error.
#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/curvebook.ico");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/curvebook.ico")
        .set("ProductName", "CurveBook")
        .set("ProductVersion", env!("CARGO_PKG_VERSION"))
        .set("FileVersion", env!("CARGO_PKG_VERSION"))
        .set("FileDescription", "CurveBook scientific biostatistics and curve-fitting application")
        .set("CompanyName", "CurveBook contributors")
        .set("LegalCopyright", "Copyright (c) the CurveBook contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "curvebook.exe")
        .set("InternalName", "curvebook");
    if let Err(err) = res.compile() {
        if std::env::var_os("CURVEBOOK_REQUIRE_WINRES").is_some() {
            return Err(format!("embedding Windows resources failed: {err}"));
        }
        println!("cargo:warning=curvebook.exe built without icon/version resources: {err}");
    }
    Ok(())
}
