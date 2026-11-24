# Verification Verdicts: SEC-10 — Typography, Rasterizer, SVG, PDF & EMF Exports

- **Section:** `SEC-10` (Typography, Rasterizer, SVG, PDF & EMF Exports)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-10.md`](SEC-10.md)
- **Baseline SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend`
- **Date:** `2026-11-22`

## Summary

| Verdict | Count | Finding IDs |
|---|---:|---|
| `CONFIRMED` | 11 | `FA-SEC10-01`..`FA-SEC10-11` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Finding IDs |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 5 | `FA-SEC10-01`, `FA-SEC10-02`, `FA-SEC10-03`, `FA-SEC10-04`, `FA-SEC10-05` |
| `S3` | 4 | `FA-SEC10-06`, `FA-SEC10-07`, `FA-SEC10-08`, `FA-SEC10-09` |
| `S4` | 2 | `FA-SEC10-10`, `FA-SEC10-11` |

## File Coverage Completeness Check

- **Result:** **PASS (15 / 15 source files + 4 `Cargo.toml` manifests, 15,886 LOC — 100% covered)**
- Independently verified all files in `SEC-10`'s scope via directory enumeration and `wc -l`:
  - Manifests (`98` LOC): `crates/text/Cargo.toml` (`24`), `crates/render/Cargo.toml` (`26`), `crates/svg/Cargo.toml` (`25`), `crates/pdf/Cargo.toml` (`23`).
  - `crates/text/src/` (`8,108` LOC): `glyph_paths.rs` (`1,915`), `lib.rs` (`97`), `markup.rs` (`2,182`), `metrics.rs` (`2,348`), `number.rs` (`359`), `shaping.rs` (`1,207`).
  - `crates/render/src/` (`4,639` LOC): `emf.rs` (`1,132`), `lib.rs` (`59`), `png.rs` (`42`), `rasterizer.rs` (`3,294`), `tiff.rs` (`112`).
  - `crates/svg/src/` (`1,230` LOC): `lib.rs` (`38`), `writer.rs` (`1,192`).
  - `crates/pdf/src/` (`1,811` LOC): `lib.rs` (`59`), `writer.rs` (`1,752`).
- Every file and exact LOC matches the File Coverage Table in [`docs/audit/full-2026-11-22/SEC-10.md`](SEC-10.md).

---

## Per-Finding Verdicts

### Verdict `FA-SEC10-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent `.notdef` tofu in `/F6` + style fallback discarding embedded TTF)
- **Evidence:**
  - `crates/pdf/src/writer.rs:1300-1328`: `split_span_into_pdf_segments` routes `symbol_font_byte(ch)` (`:1313`) to `/F4` (`Symbol`, preserving test contract at `crates/pdf/tests/pdf17_truetype_embed.rs:310`) and then routes every `try_winansi_font_byte(ch)` match (`:1316`) to `base_font` (`PdfFontId::EmbeddedTrueType` `/F6` when `embedded_info.is_some()` and `(Regular, Upright)`). It never calls `info.glyph_id(ch)` (`crates/text/src/metrics.rs:1124-1133`) to check whether the embedded font's `cmap` maps `ch`.
  - `crates/text/src/metrics.rs:1790-1808`: `build_win1252_widths` emits `notdef_1000` for any WinAnsi byte where `gid == 0`, and `writer.rs:485` binds `/F6` with `/Encoding /WinAnsiEncoding`. Consequently, unmapped WinAnsi characters in an embedded custom/subset/CJK TTF emit WinAnsi byte codes into `/F6` that render `.notdef` rather than falling back to Core-14 Helvetica (`/F1`) or glyph outlines.
  - `crates/pdf/src/writer.rs:1286-1292`: `PdfFontId::from_style_with_ttf` returns `Self::EmbeddedTrueType` strictly for `(SpanWeight::Regular, SpanSlope::Upright)` — Bold, Italic, and Bold-Italic spans always revert to unembedded Core-14 Helvetica (`/F2`, `/F3`, `/F5`).

### Verdict `FA-SEC10-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unconditional dead `/FontFile2` embedding vs ignored `embedded_ttf` in `outline_text: true` + broken combining-mark context)
- **Evidence:**
  - `crates/pdf/src/writer.rs:139-145, 423-425, 593-597`: `render_pdf_page_refs` calls `resolve_embedded_truetype(options)` unconditionally (counting `compressed_bytes.len()` against `MAX_PDF_OUTPUT_BYTES`), `assemble_pdf_document` always writes `/F6` + `/FontDescriptor` + compressed `/FontFile2`, and `build_page_dict` always adds `/F6` to page `/Resources`, even when `options.outline_text == true` or no page ever selects `/F6`. (`crates/pdf/tests/pdf17_truetype_embed.rs:352` only tested `outline_text: true` with `embedded_truetype_font: None` and empty `BUNDLED_FONTS`.)
  - `crates/pdf/src/writer.rs:1095-1116` (`emit_outlined_text`): calls `outline_visual_rich_text_rotated(&visual_line, 0.0, 0.0, text.font_size_pt, 0.0)` at `:1103`, ignoring `self.embedded_ttf` completely (whereas `emit_selectable_text` -> `build_outlined_fallback_glyphs` -> `append_outlined_char` at `:1372-1374` passes `embedded_ttf` to `outline_ttf_glyph`).
  - `crates/pdf/src/writer.rs:1388-1395`: when `outline_ttf_glyph` returns `None`, `append_outlined_char` outlines each `ch` in isolation (`RichText::new(vec![RichSpan { text: ch.to_string(), ..span.clone() }])`), losing `last_base` (`crates/text/src/glyph_paths.rs:118, 158`) so combining diacritics do not attach to preceding base characters.

### Verdict `FA-SEC10-03` · `CONFIRMED` `[REGRESSION vs #9]`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (non-uniform aspect distortion and misstated DPI metadata on capped raster export paths)
- **Evidence:**
  - `crates/audit/local-code-audit-2026-11-18.md:90-96, 1338, 1341` closed finding `#9` by switching `crates/engine/src/cmd/export.rs:637-702` to `resolve_raster_target` and adding `RenderTargetSpec::from_scene_dpi` (`crates/render/src/rasterizer.rs:87-117`, tested at `crates/render/tests/render_export.rs:801-851`).
  - However, `RenderTargetSpec::from_scene_capped` (`crates/render/src/rasterizer.rs:128-140`) still scales and clamps each axis independently to `MIN_CAPPED_RASTER_DIM_PX..=MAX_CAPPED_RASTER_DIM_PX` (`64..=8192`) while returning the caller's `dpi` unchanged (`:139`), and if only one of `requested_width_px` or `requested_height_px` is supplied, the unspecified axis still scales independently at `dpi` instead of preserving `scene.width_pt / scene.height_pt`.
  - Actual figure exporters — `McpPngExporter` (`crates/mcp/src/formats.rs:1118`), `McpTiffExporter` (`crates/mcp/src/formats.rs:1140`), and `ui-egui` control `doc.export` (`crates/ui-egui/src/control_io.rs:2335`) — plus `render.preview` (`crates/mcp/src/formats.rs:723`, `crates/ui-egui/src/control_io.rs:2163`) all call `from_scene_capped`. A `2000 x 60 pt` scene exported at `360 DPI` yields `8192 x 300 px` tagged `360 DPI` (~18% horizontal compression), reproducing `#9`.

### Verdict `FA-SEC10-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (export outlining ignores `SystemFontSet` / runtime fonts, collapsing non-Latin scripts to `□`)
- **Evidence:**
  - `crates/text/build.rs:21-26`: emits `pub const BUNDLED_FONTS: &[(&str, &[u8])] = &[];` whenever `CURVEBOOK_FONTS_DIR` is unset at compile time, and `git ls-files '*.ttf' '*.otf' '*.ttc'` confirms zero bundled fonts in the repository (consistent with `docs/release/releasing.md:187`).
  - `crates/text/src/glyph_paths.rs:124, 301-317, 876-913`: `outline_visual_rich_text_rotated` (called via `outline_rich_text` by `crates/render/src/rasterizer.rs:2917`, `crates/svg/src/writer.rs:593`, and `crates/pdf/src/writer.rs:1103`) only calls `select_bundled_font(span.weight, span.slope)` (`BUNDLED_FONTS`) and never queries `SystemFontSet` (`crates/text/src/shaping.rs:275-307`), which loads runtime `CURVEBOOK_FONTS_DIR` and host OS script fallback faces (including Arabic, Hebrew, and CJK).
  - When `BUNDLED_FONTS` is empty, `try_build_glyph_strokes` only covers ASCII, Latin-Extended, Greek, Cyrillic, math symbols, and combining marks, so `build_glyph_strokes` (`glyph_paths.rs:880-881`) emits identical `0.12..0.88` placeholder rectangles (`□`) for CJK (`crates/text/tests/audit_r1_s4b_s09.rs:164` uses `\u{4E2D}` as the canonical fallback-box oracle), Arabic, Hebrew, Devanagari, and other Unicode scripts across PNG, TIFF, `outline_text: true` SVG, and default PDF exports.

### Verdict `FA-SEC10-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently clamped `num_contours` corrupts `glyf` simple-glyph binary offsets)
- **Evidence:**
  - `crates/text/src/glyph_paths.rs:20, 379-382`: `decode_ttf_glyph_recursive` clamps positive `num_contours` via `usize::try_from(num_contours).ok()?.min(MAX_TTF_CONTOURS)` (`MAX_TTF_CONTOURS = 256`) rather than returning `None`.
  - `crates/text/src/glyph_paths.rs:399-415`: `decode_simple_ttf_glyph` then reads only `256` `endPtsOfContours` entries and computes `let inst_len_off = 10 + num_contours * 2;` (`522`). For any TrueType simple glyph with `actual_num_contours > 256`, bytes `522..524` contain `endPtsOfContours[256]`, not `instructionLength`. Advancing `cursor` by that misread `u16` either returns `None` if past `slice.len()` or parses remaining contour endpoints, instructions, and misaligned flags/coordinates as bogus contours (no panic or UB thanks to checked `.get` / `read_u16_be` indexing, but silent outline corruption).

### Verdict `FA-SEC10-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (EMF DPI=0 silent coercion, missing RTL BiDi span reordering, unbounded output allocation)
- **Evidence:**
  - `crates/render/src/emf.rs:113-116, 681-717`: `encode_raster_emf` clamps `image.dpi.max(1)` instead of returning `RenderError::InvalidTarget` when `image.dpi == 0` (unlike `validate_raster_target_dims` at `crates/render/src/rasterizer.rs:63`), inflating `width_pt`/`height_pt` by up to 72×; `emit_stretch_dibits` likewise omits `dpi == 0` validation.
  - `crates/render/src/emf.rs:548-591, 663`: `EmfWriter::emit_text` never calls `rich_contains_rtl` or `reorder_rich_text_bidi`. Instead it emits per-span `EMR_EXTTEXTOUTW` records (`fOptions = 0`, without `ETO_RTLREADING`) with monotonically left-to-right `cursor_x += span_w` and explicit `dx_logical` arrays in logical span order, misordering RTL/mixed multi-span labels relative to PDF, SVG-outlined, and raster exports.
  - `crates/render/src/emf.rs:128-155, 974`: `encode_emf_with_raster` applies no primitive count limit or incremental byte budget (compare `MAX_PDF_TOTAL_PRIMITIVES` / `MAX_PDF_OUTPUT_BYTES` in `crates/pdf/src/writer.rs:64-67`), checking only `u32::try_from(self.buf.len())` (4 GiB) at the end in `EmfWriter::finalize()`.

### Verdict `FA-SEC10-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (SVG `@font-face` rules unreferenced by `<text>`, missing `]]>` CDATA escaping)
- **Evidence:**
  - `crates/svg/src/writer.rs:210-237`: when `options.embed_fonts && !options.outline_text && !BUNDLED_FONTS.is_empty()`, `write_defs` emits `@font-face { font-family: "<name>"; src: url("data:font/ttf;base64,..."); }` using the raw manifest entry name from `BUNDLED_FONTS` (e.g., `BIZUDPGothic-Regular.ttf` or `sub/Font.ttf` per `crates/text/build.rs:64`).
  - `crates/svg/src/writer.rs:24, 500-503, 556-561`: `write_editable_text` unconditionally emits `font-family="Helvetica, Arial, sans-serif"` (`DEFAULT_FONT_FAMILY`) and only emits per-`<tspan>` `font-family` when `span.font_family` is explicitly populated (and `crates/chart` never sets `span.font_family`), leaving the embedded `@font-face` rules unused unless the caller manually sets matching manifest names on spans.
  - `crates/svg/src/writer.rs:784-797`: `escape_css_string` escapes `"` and `\` and strips invalid XML chars, but emits raw `name` inside `<![CDATA[ ... ]]>` (`:228-234`) without escaping or stripping `]]>`, allowing `]]>` in a font key to terminate the CDATA block prematurely.

### Verdict `FA-SEC10-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (stroked near-circular ellipses collapsed to circles within 5% tolerance)
- **Evidence:**
  - `crates/render/src/rasterizer.rs:1151-1156, 1171-1177`: `try_rasterize_ellipse_path` defines `is_near_circle` as `(ellipse.rx - ellipse.ry).abs() <= 0.05 * ellipse.rx.max(ellipse.ry).max(1.0)` and accepts stroked solid paths meeting that tolerance. On the mask cache branch (`:1172-1177`), whenever `stroke_color.is_some()`, it averages `(ellipse.rx, ellipse.ry)` into `(r, r)` where `r = 0.5 * (ellipse.rx + ellipse.ry)`.
  - `crates/render/src/rasterizer.rs:1039-1140, 1497`: `rasterize_ellipse_direct` shares `EllipseEvalGeom::new(..., stroke_color.is_some() && sr > 0.0)`, whose `has_stroke == true` paths (`:1093-1100, 1125-1132`) test `d2 <= self.r_sq` and `self.r_in_sq..=self.r_out_sq` using the circular average `r` for both fill and stroke. Any stroked ellipse with up to 5% aspect asymmetry (e.g., `rx = 20, ry = 19`) rasterizes as an exact circle instead of an ellipse.

### Verdict `FA-SEC10-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (RTL-directed non-RTL multi-span line runs BiDi per span instead of across the line)
- **Evidence:**
  - `crates/text/src/shaping.rs:828-840`: when `direction == TextDirection::Rtl && !rich_contains_rtl(line)` (for example, explicit `dir_override = Some(TextDirection::Rtl)` or a neutral/Latin follow-up line in an RTL multi-line block at `:575`, since `reorder_rich_text_bidi` at `crates/text/src/metrics.rs:198-199` infers direction rather than taking a base direction), `reorder_rich_line` maps `reorder_visual_bidi(&span.text, TextDirection::Rtl)` independently over each span without reversing span order or resolving cross-span brackets/neutrals. For instance, `["(", "100", ")"]` across three spans reorders into `")100("` visually instead of paragraph-ordered `"(100)"`.

### Verdict `FA-SEC10-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (incomplete manifest path containment check allows absolute paths and blocks `..` in filenames)
- **Evidence:**
  - `crates/text/src/shaping.rs:1011-1019`: `load_faces_from_dir` filters `manifest.txt` entries with `if rel.is_empty() || rel.contains("..") { continue; }` and immediately calls `let candidate = base_dir.join(rel);`. `Path::join` replaces `base_dir` entirely if `rel` is absolute (`/etc/...` or Windows drive-rooted paths), while `rel.contains("..")` rejects legitimate filenames containing two dots (`Font..v2.ttf`). Only local `CURVEBOOK_FONTS_DIR` / `from_fonts_dir` manifests reach this helper (`S4` hygiene/hardening).

### Verdict `FA-SEC10-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (unused `curvebook-math` dependency in three L3 crates)
- **Evidence:**
  - `crates/render/Cargo.toml:19`, `crates/svg/Cargo.toml:18`, and `crates/pdf/Cargo.toml:17` each declare `curvebook-math = { workspace = true }`. Workspace-wide search (`rg -n "curvebook_math|curvebook-math" curvebook/crates/render curvebook/crates/svg curvebook/crates/pdf`) confirms zero source or test imports of `curvebook_math` in any of the three crates.
