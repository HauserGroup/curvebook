# Platform, GPU & Print Hardware Parity

> **Scope & Environment Note:** Cross-platform installer manifests (`packaging/`), headless `--control <port>` / `--headless-once` desktop binary (`apps/lumencraft`), WebAssembly shell init (`apps/lumencraft-web`), CPU rasterizer (`lumencraft-render`), multi-flavor clipboard payload builder (`ClipboardBundle`), and physical-scale PDF print spool builder (`PrintJobSpec::spool_pdf`) are implemented and verified headlessly. Native OS GUI windowing, live GPU (`wgpu`), live OS pasteboard, and live OS print-driver calls remain unchecked (`[ ]`) pending native GUI/CI runner validation.

## Operating Systems & Targets
- [ ] macOS 11+ universal (`aarch64-apple-darwin`, `x86_64-apple-darwin`) native menu bar, file associations, and Retina HiDPI scaling *(`packaging/macos/` bundle generator & `lumencraft` desktop entrypoint (`--in-window-menus`, `--control`) verified; live Cocoa menu-bar window loop pending)*
- [ ] Windows 10/11 (`x86_64`, `i686`, `aarch64`) static-CRT builds, dual-scope WiX v5 MSI, and `portable.txt` portable mode *(`packaging/windows/` MSI manifests & `portable.txt` policy tests present; Windows native runner execution pending)*
- [ ] Linux (`x86_64`, `aarch64`, `riscv64`) Wayland + X11 windowing, XDG file-chooser portal, AppImage, `.deb`, `.rpm`, and sandboxed Flatpak *(`packaging/linux/` `nfpm`, AppImage, `.desktop`, metainfo, MIME, and Flatpak manifests present; live Wayland/X11 window runtime pending)*
- [ ] FreeBSD (`x86_64`, `aarch64`) native desktop & CLI tarball *(`packaging/freebsd/package.sh` present; FreeBSD runner execution pending)*
- [ ] Browser WebAssembly (`wasm32-unknown-unknown` with `+simd128`) running offline-capable in Chrome, Firefox, Edge, and Safari *(`apps/lumencraft-web::init_web_app` tested natively on host; `cargo xtask wasm` skipped locally because `wasm32-unknown-unknown` sysroot is absent under `rustup`, enforced in CI)*

## Rendering, Clipboard & Printing
- [ ] `wgpu` hardware-accelerated canvas rendering (Metal, Vulkan, DX12, WebGPU, WebGL2) + deterministic CPU fallback for headless CLI exports *(deterministic CPU software rasterizer `lumencraft-render` + UI software frame compositor `render_workspace_frame_png` verified; `wgpu` GPU backend not bundled in offline `Cargo.lock`)*
- [ ] Smooth trackpad pinch-to-zoom and inertial pan on Graph and Layout sheets *(logical `CanvasDialogState` zoom & `ui.zoom.fitSheet` wired; OS trackpad gesture stream pending)*
- [ ] Native vector clipboard copy (PDF pasteboard on macOS, EMF/SVG/HTML+PNG on Windows/Linux) for pasting editable figures directly into manuscripts and slide decks *(multi-flavor `ClipboardBundle` emitting `PDF`, `SVG`, `HTML`, high-DPI `PNG`, and locale-aware TSV verified in `lumencraft-ui-egui::control_io`; live OS pasteboard IPC pending)*
- [ ] Native OS print pipeline (CUPS on macOS/Linux/FreeBSD, Print Ticket / PDF vector spooling on Windows) *(physical `100%`-scale multi-page PDF print spooling with optional crop/bleed marks verified via `PrintJobSpec::spool_pdf` in `control_io`; live OS CUPS/Win32 print driver submission pending)*
