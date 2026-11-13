# Platform, GPU & Print Hardware Parity

## Operating Systems & Targets
- [ ] macOS 11+ universal (`aarch64-apple-darwin`, `x86_64-apple-darwin`) native menu bar, file associations, and Retina HiDPI scaling
- [ ] Windows 10/11 (`x86_64`, `i686`, `aarch64`) static-CRT builds, dual-scope WiX v5 MSI, and `portable.txt` portable mode
- [ ] Linux (`x86_64`, `aarch64`, `riscv64`) Wayland + X11 windowing, XDG file-chooser portal, AppImage, `.deb`, `.rpm`, and sandboxed Flatpak
- [ ] FreeBSD (`x86_64`, `aarch64`) native desktop & CLI tarball
- [ ] Browser WebAssembly (`wasm32-unknown-unknown` with `+simd128`) running offline-capable in Chrome, Firefox, Edge, and Safari

## Rendering, Clipboard & Printing
- [ ] `wgpu` hardware-accelerated canvas rendering (Metal, Vulkan, DX12, WebGPU, WebGL2) + deterministic CPU fallback for headless CLI exports
- [ ] Smooth trackpad pinch-to-zoom and inertial pan on Graph and Layout sheets
- [ ] Native vector clipboard copy (PDF pasteboard on macOS, EMF/SVG/HTML+PNG on Windows/Linux) for pasting editable figures directly into manuscripts and slide decks
- [ ] Native OS print pipeline (CUPS on macOS/Linux/FreeBSD, Print Ticket / PDF vector spooling on Windows)
