//! Build-time env tracking for `curvebook-ui-egui` (surfaces `CURVEBOOK_BUILD_SHA` in About).
#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CURVEBOOK_BUILD_SHA");
}
