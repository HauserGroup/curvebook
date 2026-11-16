//! Build-time hook for `curvebook` desktop executable metadata (`CURVEBOOK_BUILD_SHA`, Windows
//! icon/VERSIONINFO).
#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/curvebook.ico");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_REQUIRE_WINRES");
    Ok(())
}
