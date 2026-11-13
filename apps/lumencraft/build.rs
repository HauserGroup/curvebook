//! Build-time hook for `lumencraft` desktop executable metadata (`LUMENCRAFT_BUILD_SHA`, Windows
//! icon/VERSIONINFO).
#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/lumencraft.ico");
    println!("cargo:rerun-if-env-changed=LUMENCRAFT_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=LUMENCRAFT_REQUIRE_WINRES");
    Ok(())
}
