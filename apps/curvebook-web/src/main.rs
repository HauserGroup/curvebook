//! Minimal `wasm32` placeholder binary entry point for CurveBook (`R11.11`).
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

fn main() {
    #[cfg(target_arch = "wasm32")]
    let _status = curvebook_web::web_placeholder_status();
    #[cfg(not(target_arch = "wasm32"))]
    let _status = curvebook_web::placeholder_status();
}
