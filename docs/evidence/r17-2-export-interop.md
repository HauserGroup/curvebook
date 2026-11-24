# CurveBook R17.2 Export & Clipboard Interoperability Evidence

> Workstream: `R17.2` (Downstream vector, document, raster, and clipboard interoperability) · Branch: `feature/R17.2-export-interop`
> Governing crates: `curvebook-svg`, `curvebook-pdf`, `curvebook-render`, `curvebook-engine`, `curvebook-ui-egui`, `curvebook-mcp`, `curvebook`

## 1. Interoperability Fixes Landed

1. **SVG 1.1 + SVG 2 Compatibility & Degenerate Paint Hygiene (`crates/svg/src/writer.rs`):**
   - Added `has_images` tracking on `SvgRenderCtx`: root `<svg>` now declares `xmlns:xlink="http://www.w3.org/1999/xlink"` whenever embedded raster images are emitted, and `<image>` elements emit both `xlink:href="data:..."` (for Illustrator / Office / SVG 1.1 consumers) and `href="data:..."` (for SVG 2 viewers). Vector-only scenes omit `xmlns:xlink`.
   - Filtered zero-alpha fills (`alpha > 0`), zero-alpha strokes (`alpha > 0 && width_pt > 0.0`), and zero-alpha text in `write_path_element` and `write_text_element`, plus guarded all-zero `stroke-dasharray` in `write_stroke_attrs` after finiteness validation. Eliminates `stroke-width="0.000"`, `stroke-dasharray="0.000,..."`, and invisible ghost primitives in downstream vector editors.
   - Regression coverage: `crates/svg/tests/svg_export.rs` (`svg_interop_emits_xlink_and_suppresses_degenerate_strokes_and_alpha`).

2. **PDF Page `/Resources` `/Font` Dictionary Pruning (`crates/pdf/src/writer.rs`):**
   - Derived `PartialOrd, Ord` on `PdfFontId`, added `fonts: BTreeSet<PdfFontId>` to `PageResourcesUsed`, recorded referenced fonts in `emit_pdf_text_segment`, and updated `build_page_dict` to emit `/Font << ... >>` strictly for fonts referenced on that page (omitting `/Font` completely when `outline_text: true` or on textless pages, while keeping PDF object IDs `3..7` intact for Core-14 structure parity).
   - Verified with `pdffonts` (`26.11.0`): all `.pdf17_outlined.pdf` exports now report `0` referenced fonts (`fonts=0`) and `.pdf17_ttf.pdf` exports no longer leak unreferenced Core-14 Type 1 font references (`/F1`–`/F3`, `/F5`).
   - Regression coverage: `crates/pdf/tests/pdf17_truetype_embed.rs` (`pdf17_prunes_unreferenced_fonts_and_omits_font_dict_when_outlined`).

3. **ECMA-234 `.emf` Pen Geometry & GDI Font Family Normalization (`crates/render/src/emf.rs`):**
   - Updated `create_pen` so thick solid strokes (`width_pt > 1.0`) with `LineCap::Butt` or `LineJoin::Miter` emit geometric `EMR_EXTCREATEPEN` (`PS_GEOMETRIC | PS_SOLID | cap | join`) instead of `EMR_CREATEPEN` (which Windows GDI renders with round caps/joins above 1 px), while keeping narrow `<= 1.0 pt` solid or round/round pens on `EMR_CREATEPEN`.
   - Added `normalize_emf_font_family` in `emit_text_span` so CSS comma-separated/quoted font stacks (e.g. `"Helvetica Neue", Helvetica, sans-serif`) normalize to a single unquoted primary GDI `lfFaceName` (`Helvetica Neue`).
   - Regression coverage: `crates/render/tests/emf_and_budget.rs` (`emf_thick_non_round_solid_uses_extcreatepen_and_normalizes_css_font_stack`).

4. **Multi-Flavor Clipboard Bundle & One-Click `ui.clipboard.copyForSlides` (`crates/ui-egui`, `apps/curvebook`, `crates/mcp`):**
   - Added Deflate-compressed TIFF (`tiff_bytes: Option<Vec<u8>>`, macOS UTType `public.tiff`) to `ClipboardBundle` and `available_flavors()`.
   - Updated `build_html_clipboard_wrapper` to embed `<img src="data:image/png;base64,..."/>` inside `<div class="curvebook-figure">` whenever `png_bytes` is non-empty (falling back to inline SVG only when PNG bytes are absent), preventing rich-text targets from stripping inline `<svg>` tags or pasting raw text.
   - Added typed UI command `ui.clipboard.copyForSlides` (`Copy for Slides & Documents`, default shortcut `Cmd+Shift+C`, `Rank::Secondary`; `UI_COMMANDS` `53 -> 54`, 16-locale `catalog.tsv` `708 -> 709` keys, regenerated `crates/mcp/src/ui_catalog.rs`). Exposed via Edit menu, Graph/Layout canvas right-click context menus (`canvas.graph_ctx.copy_for_slides`, `canvas.layout_ctx.copy_for_slides`), Command Palette, `--control` JSONL dispatcher, and `DesktopHostRuntime::handle_clipboard_copy_for_slides`.
   - Writes `300 DPI` PDF + TIFF + PNG + EMF + SVG + `<img>` HTML via `write_os_clipboard_bundle_for_slides` while omitting raw `<svg>` XML from plain-text pasteboard flavor so Office/Keynote/Docs paste rendered figures instead of XML text. Existing `edit.copy` (`Cmd+C`) continues writing `bundle.svg` to `public.utf8-plain-text` to preserve `macos_clipboard_live.rs` roundtrip expectations.
   - Regression coverage: `crates/ui-egui/src/os_integration.rs` (`os_integration::tests::clipboard_bundle_populates_valid_emf_and_writes_bundle_status`), `crates/ui-egui/tests/r10_23_shortcuts_and_copy.rs` (`r17_2_copy_for_slides_shortcut_context_menu_and_dispatch_ui_command`), and `apps/curvebook/tests/audit_r2_desktop_host.rs` (`r17_2_desktop_host_copy_for_slides_routes_at_300_dpi_without_plain_text_svg`).

## 2. Export Interop Suite (`crates/engine/tests/r17_2_export_interop.rs`)

Built and exported 5 single-page scientific figures (`01_xy_broken` with `DoubleSlash` broken X axis, log threshold, `ModelEquation`, rich superscripts/subscripts/Greek/Unicode; `02_col_brackets` ANOVA columns with `Square`/`HalfTick`/`CurlyBrace` significance brackets; `03_heatmap` correlation/expression heatmap + colorbar; `04_layout_multipanel` `DoubleColumn174Mm` `493.228 x 340.157 pt` layout with `align_plot_origins`, cropped/rotated PNG & JPEG images, and callout; `05_vector_stress` with nested `PushClip`/`PopClip`, rotated cropped RGBA PNG, all `LineDashStyle`s across `1.6 pt`/`0.25 pt`/`0.0 pt`, zero-alpha paths, combining marks, rotated `-30°` subscript, and CSS font-family stacks) plus multi-page `06_multipage_all.pdf` (`360x260 pt` pages 1–3, `493.228x340.157 pt` page 4) and `Session::execute("export.{svg,pdf,png,tiff}")`.

## 3. Downstream Tool Verification on macOS Host (Post-Fix Runs)

All external tool checks below were executed on the final edited exporters against `.tmp-export-interop/` (which was deleted before commit):

| Tool & Version | Target Set | Command / Execution Mode | Result Summary |
|---|---|---|---|
| **Inkscape `1.4.4 (dcaf3e7, 2026-05-05)`** | All `16` SVGs (`01..05` editable/outlined/embedfonts + `engine_layout_cmd.svg`) | `--export-type=png --export-dpi=150` & `--export-type=pdf` with isolated `HOME`/`XDG_*` (`BypassSandbox: true`; note Inkscape times out at startup inside sandbox) | `png_rc=0`, `pdf_rc=0` across all `16/16` SVGs, `0` stderr warnings (`png_err=''`, `pdf_err=''`). Cross-renderer MAE vs 2x-downsampled `300` DPI CPU raster: `01_xy_broken` `5.27` (edit) / `1.30` (outl), `02_col_brackets` `2.04` / `0.73`, `03_heatmap` `1.72` / `0.71`, `04_layout_multipanel` `3.08` / `1.64`, `05_vector_stress` `1.12` / `0.06` (out of 255). |
| **Poppler `pdfinfo` / `pdffonts` `26.11.0`** | All `27` exported PDFs | `pdfinfo <file>` & `pdffonts <file>` (plus `pdfinfo -f 1 -l 4 06_multipage_all.pdf`) | `rc=(0,0)` on `27/27` PDFs; all `5` `.pdf17_outlined.pdf` report `fonts=0`; `.pdf17_ttf.pdf` & `06_multipage_all.pdf` report `fonts=2` (`Symbol` Type 1 + embedded `ArialMT` TrueType). Multi-page boxes match `360 x 260 pts` (p1–3) & `493.228 x 340.157 pts` (p4). |
| **Poppler `pdftoppm` `26.11.0`** | `25` `.pdf` files (`01..05` sRGB/CMYK/outlined/TTF) | `pdftoppm -png -r 150 -singlefile` | `rc=0` on all; outlined PDF MAE vs CPU raster: `1.21` (`01`), `0.88` (`02`), `1.15` (`03`), `1.22` (`04`), `0.50` (`05`); Inkscape-outlined vs Poppler-outlined MAE: `1.50` (`01`), `1.16` (`02`), `1.12` (`03`), `1.90` (`04`), `0.49` (`05`). |
| **Poppler `pdftotext` `26.11.0`** | Selectable PDFs (`01_xy_broken.pdf17_ttf`, `02_col_brackets.pdf17_srgb`, `04_layout_multipanel.pdf17_ttf`) | `pdftotext <file> -` | `rc=0`; preserved `IC50 = 10-6 µM (α, β, ∆G, χ², ±0.05, °C, Å)`, `−12.4 kJ·mol−1`, `p ≤ 0.001`, `Tukey HSD`. |
| **Ghostscript `10.08.0`** | `04_layout_multipanel`, `05_vector_stress` (`pdf17_srgb`, `pdf17_outlined`), `06_multipage_all.pdf` | `gs -dBATCH -dNOPAUSE -sDEVICE=png16m -r150` | `rc=0`; `04` MAE vs CPU `3.88`/`4.49`, `05` MAE vs CPU `1.44`/`1.39`; `06_multipage_all.pdf` rendered 4 pages with exact sizes `[(750, 542), (750, 542), (750, 542), (1028, 709)]`. |
| **pdfTeX `3.141592653-2.6-1.40.29` (TeX Live 2026)** | `01_xy_broken.pdf17_srgb.pdf`, `02_col_brackets.pdf17_ttf.pdf`, `03_heatmap.pdf17_cmyk.pdf`, `04_layout_multipanel.pdf17_outlined.pdf`, `05_vector_stress_600dpi.png` | `pdflatex -interaction=nonstopmode -halt-on-error` compiling 5-figure `\includegraphics` `manuscript.tex` | `rc=0`, `Pages: 5`, `warnings=0`. |
| **macOS `sips` & Binary Inspection** | All `32` exported PNG (`300`/`600` DPI) & TIFF (`Deflate`/`Lzw`/`None`) + `10` EMF files | `sips -g pixelWidth -g pixelHeight -g dpiWidth -g dpiHeight` + binary header/IFD/EMF parser | `32/32` PNG & TIFF files opened cleanly (`rc=0`); exact `300.000` / `600.000` DPI (`04_layout_multipanel` `2055x1417` @ 300 DPI, `4110x2835` @ 600 DPI); PNG `pHYs` `11_811`/`23_622` px/m; TIFF `Compression` (`8`/`5`/`1`), `ResolutionUnit=2`, `ExtraSamples=2`; EMF `sig=0x464d4520`, balanced GDI handles, geometric `EMR_EXTCREATEPEN` (`95`), normalized `EMR_EXTCREATEFONTINDIRECTW` (`82`), and bottom-up BGRA `EMR_STRETCHDIBITS` (`81`). |
| **macOS `NSPasteboard` (`macos_clipboard_live`)** | Multi-flavor clipboard live pasteboard smoke test | `cargo test -p curvebook-ui-egui --test macos_clipboard_live -- --nocapture` | Inside sandbox prints `SKIPPED (headless macOS pasteboard server unavailable on pbcopy: status=exit status: 1 stderr='')`; deterministic adapter & `StubPlatformClipboardAdapter` suites verify standard & slide clipboard bundles (`tif`/`png`/`pdf`/`svg`/`emf`/`html`). |

## 4. External Applications Not Installed / Not Run on This Host

To keep verification records strictly factual, the following downstream targets were **not** interactively pasted into on this machine because the applications are not installed or require a live Windows / interactive desktop session:
- **Adobe Illustrator** (not installed on this host; SVG 1.1 `xmlns:xlink` + `xlink:href` + Inkscape 1.4.4 import/render used as local vector proxy).
- **Microsoft Word & Microsoft PowerPoint (macOS/Windows)** (not installed on this host; structural `cf-html`, `<img>` data-URI HTML wrapper, PDF/TIFF/PNG/EMF payloads, and `write_os_clipboard_bundle_for_slides` behavior audited offline).
- **Windows native clipboard paste (`OpenClipboard` / `CF_ENHMETAFILE` / `CF_HTML` / `PNG`)** (tested via Win32 byte-offset unit tests and cross-platform command framing in `crates/ui-egui/src/os_integration.rs`, not executed on a Windows host).
