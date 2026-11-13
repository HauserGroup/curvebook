//! Browser WebAssembly entry point for LumenCraft.
#![forbid(unsafe_code)]

fn main() {
    let _ = lumencraft_ui_egui::bundled_fonts_enabled();
    let _ = lumencraft_format::EXTENSION;
}
