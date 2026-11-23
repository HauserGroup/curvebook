# Security & Correctness Audit — SEC-10: Text & Publication Output (`crates/text`, `crates/render`, `crates/svg`, `crates/pdf`)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-10` — Text & Publication Output (`crates/{text,render,svg,pdf}/Cargo.toml` and all 15 files in `crates/{text,render,svg,pdf}/src/`)
- **Date:** `2026-11-22`
- **Summary Counts:** `S1 = 0`, `S2 = 5`, `S3 = 4`, `S4 = 2` (**Total: 11 findings**)

---

## File Coverage Table

| File | Exact Lines | Lines Inspected | Concrete Invariants Checked | Finding IDs / Status |
|---|---:|---|---|---|
| `crates/text/Cargo.toml` | 24 | `1..=24` | `[lints] workspace = true`, `bundled-fonts` default feature, `wasm32`-clean deps (no `chrono`), active usage of declared deps | Clean — all declared dependencies used in `crates/text/src/` (`curvebook_math`, `curvebook_palette`, `serde`) |
| `crates/render/Cargo.toml` | 26 | `1..=26` | `[lints] workspace = true`, `wasm32`-clean deps (no `chrono`), active usage of declared deps | `FA-SEC10-11` |
| `crates/svg/Cargo.toml` | 25 | `1..=25` | `[lints] workspace = true`, `wasm32`-clean deps (no `chrono`), active usage of declared deps | `FA-SEC10-11` |
| `crates/pdf/Cargo.toml` | 23 | `1..=23` | `[lints] workspace = true`, `wasm32`-clean deps (no `chrono`), active usage of declared deps | `FA-SEC10-11` |
| `crates/text/src/lib.rs` | 97 | `1..=97` | `#![forbid(unsafe_code)]`, public text/metrics/shaping/outline API exports, `BUNDLED_FONTS` re-export | Clean — `#![forbid(unsafe_code)]` enforced, clean module boundaries |
| `crates/text/src/number.rs` | 359 | `1..=359` | Axis tick formatting, p-value formatting (`format_pvalue_label`), non-finite/subnormal guards, power-of-10 / engineering SI decomposition, no indexing panics | Clean — safe guards on non-finite/zero values and deterministic formatting |
| `crates/text/src/markup.rs` | 2182 | `1..=2182` | Markup span parser (`parse_markup`), template hook expansion (`expand_template_hooks`), inline equation segments, UAX #9 BiDi (`shape_bidi_runs`, `reorder_visual_bidi`) | Clean — balanced tag/hook parsing, bounded recursion, deterministic BiDi ordering |
| `crates/text/src/metrics.rs` | 2348 | `1..=2348` | Core-14 advance tables, `measure_text_pt`, `reorder_rich_text_bidi`, SFNT `TrueTypeFont` / `parse_truetype_font_info`, fallback SFNT builders, Win-1252 width mapping (`build_win1252_widths`) | `FA-SEC10-01` |
| `crates/text/src/shaping.rs` | 1207 | `1..=1207` | `SystemFontSet`, `TextMeasurer`, `reorder_rich_line`, bounded font/manifest loading (`load_faces_from_dir`), `#[cfg(not(target_arch = "wasm32"))]` `std::fs` gating | `FA-SEC10-09`, `FA-SEC10-10` |
| `crates/text/src/glyph_paths.rs` | 1915 | `1..=1915` | `outline_rich_text`, `outline_visual_rich_text_rotated`, TrueType glyph decoding (`outline_ttf_glyph`, `decode_ttf_glyph_recursive`, `decode_simple_ttf_glyph`), clean-room fallback stroke glyphs | `FA-SEC10-04`, `FA-SEC10-05` |
| `crates/render/src/lib.rs` | 59 | `1..=59` | `#![forbid(unsafe_code)]`, raster/EMF public API exports | Clean — `#![forbid(unsafe_code)]` enforced |
| `crates/render/src/png.rs` | 42 | `1..=42` | `validated_rgba("PNG")` dimensions/DPI/length validation, `pHYs` pixels-per-meter encoding (`clamp(1, u32::MAX)`) | Clean — rejects zero/oversized/zero-DPI buffers cleanly via `validated_rgba` |
| `crates/render/src/tiff.rs` | 112 | `1..=112` | `validated_rgba("TIFF")`, TIFF 6.0 LE 13 sorted IFD tags, checked 4 GiB strip budget, Unassociated alpha tag, Adobe Deflate / LZW encoding | Clean — validated buffer bounds, sorted IFD tags, and overflow checks |
| `crates/render/src/emf.rs` | 1132 | `1..=1132` | ECMA-234 EMF framing, `encode_raster_emf`, `encode_emf_with_raster`, text span emission (`emit_text`, `emit_text_span`), raster DIB emission, GDI `HandlePool`, bounds tracking | `FA-SEC10-06` |
| `crates/render/src/rasterizer.rs` | 3294 | `1..=3294` | `MAX_RASTER_DIM_PX` / `MAX_RASTER_PIXELS`, `from_scene_dpi` / `from_scene_capped`, `RasterImage::new` (`try_reserve_exact`), `render_scene`, `draw_ellipse_fast`, `clip_segment_to_rect`, `render_text` | `FA-SEC10-03`, `FA-SEC10-04`, `FA-SEC10-08` |
| `crates/svg/src/lib.rs` | 38 | `1..=38` | `#![forbid(unsafe_code)]`, SVG exporter/reader public exports | Clean — `#![forbid(unsafe_code)]` enforced |
| `crates/svg/src/writer.rs` | 1192 | `1..=1192` | `render_svg`, `write_defs` `@font-face` / clipPath generation, `write_raster_image_element` bounds + PNG encode, `write_editable_text` / `write_outlined_text`, CSS/XML escaping, structural validation | `FA-SEC10-04`, `FA-SEC10-07` |
| `crates/pdf/src/lib.rs` | 59 | `1..=59` | `#![forbid(unsafe_code)]`, `PdfVersion` / `PdfColorSpace` public exports | Clean — `#![forbid(unsafe_code)]` enforced |
| `crates/pdf/src/writer.rs` | 1752 | `1..=1752` | `render_pdf` / multi-page limits (`MAX_PDF_PAGES`, `MAX_PDF_TOTAL_PRIMITIVES`, `MAX_PDF_OUTPUT_BYTES`), `PdfImageRegistry::register`, `/FontFile2` embedding, `emit_selectable_text`, `emit_outlined_text`, stream compression | `FA-SEC10-01`, `FA-SEC10-02`, `FA-SEC10-04` |

**Total Audited:** 15,886 lines (`crates/{text,render,svg,pdf}/Cargo.toml`: 98 + `crates/{text,render,svg,pdf}/src/*.rs`: 15,788), `100%` (`1..=N`) across all 19 files.

---

## Findings

### FA-SEC10-01 (S2) — Embedded TrueType `/F6` emits unmapped Win-1252 chars as `.notdef`, drops Bold/Italic styles, and routes Greek away from `/F6`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/pdf/src/writer.rs:1286-1328, 1341-1356`; `crates/text/src/metrics.rs:1790-1808`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** In selectable PDF mode with an embedded TrueType font (`/F6`, `PdfFontId::EmbeddedTrueType`), `split_span_into_pdf_segments` assigns every character accepted by `try_winansi_font_byte(ch)` unconditionally to `/F6` without checking whether the embedded font's `cmap` maps `ch` (`gid != 0`). Simultaneously, `build_win1252_widths` fills `/Widths` entries for unmapped Win-1252 codes (`gid == 0`) with `notdef_1000`. In addition, `PdfFontId::from_style_with_ttf` only uses `EmbeddedTrueType` for `(SpanWeight::Regular, SpanSlope::Upright)`, falling back to Core-14 Helvetica (`/F2`/`/F3`/`/F5`) for Bold/Italic spans, and `symbol_font_byte` precedes `try_winansi_font_byte`, sending Greek/math glyphs (including Win-1252-representable characters like `µ`, `°`, `±`) to Core-14 `/F4` (`Symbol`) even when the embedded font contains them.
- **Failure scenario:** Exporting a PDF with a subsetted or partial TrueType font (for example lacking curly quotes, dashes, or accented Latin characters) emits Win-1252 byte codes referencing glyph `0` (`.notdef` tofu boxes) instead of falling back to Core-14 or outlines, while Bold/Italic headings ignore the embedded TrueType typeface entirely.
- **Evidence:**
  ```rust
  // crates/pdf/src/writer.rs:1286-1292
      const fn from_style_with_ttf(weight: SpanWeight, slope: SpanSlope, has_ttf: bool) -> Self {
          if has_ttf && matches!((weight, slope), (SpanWeight::Regular, SpanSlope::Upright)) {
              Self::EmbeddedTrueType
          } else {
              Self::from_style(weight, slope)
          }
      }
  ```
  ```rust
  // crates/pdf/src/writer.rs:1302-1318
      let base_font = PdfFontId::from_style_with_ttf(span.weight, span.slope, embedded_info.is_some());
      let mut segments: Vec<SpanPdfSegment> = Vec::new();
      for ch in span.text.chars() {
          ...
          } else if let Some(sym_byte) = symbol_font_byte(ch) {
              let w = size_pt * core14_advance_em(ch, span.weight, span.slope);
              push_pdf_segment_byte(&mut segments, PdfFontId::Symbol, sym_byte, w);
          } else if let Some(win_byte) = try_winansi_font_byte(ch) {
              let w = winansi_char_width_pt(ch, span.weight, span.slope, base_font, size_pt, embedded_info);
              push_pdf_segment_byte(&mut segments, base_font, win_byte, w);
  ```
  ```rust
  // crates/text/src/metrics.rs:1795-1802
      for code in 32u8..=255u8 {
          let gid = win1252_byte_to_char(code)
              .and_then(|ch| cmap.binary_search_by_key(&u32::from(ch), |&(cp, _)| cp).ok().and_then(|idx| cmap.get(idx).map(|&(_, g)| g)))
              .unwrap_or(0);
          if gid == 0 {
              widths.push(notdef_1000);
              continue;
          }
  ```
- **Suggested fix:** Track supported Unicode/Win-1252 coverage (non-zero `gid` or `advance_em`) on `TrueTypeFontInfo` / `TrueTypeFont`, only emit `/F6` for characters present in the embedded font's `cmap` (falling back to Core-14 or outlined segments otherwise), and check embedded-font coverage before defaulting scientific symbols to Core-14 `/F4`.
- **Related:** `FA-SEC10-02`

---

### FA-SEC10-02 (S2) — PDF `outline_text: true` ignores `embedded_truetype_font`, still writes unused `/F6` font streams, and per-char fallback detaches combining marks
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/pdf/src/writer.rs:423-425, 1095-1116, 1388-1395`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** When `options.outline_text` is `true`, `StreamCtx::emit_outlined_text` calls `outline_visual_rich_text_rotated`, which has no TrueType parameter and only consults `BUNDLED_FONTS`, completely ignoring `options.embedded_truetype_font` (`self.embedded_ttf`). At the same time, `assemble_pdf_document` still allocates and writes the unused `/FontFile2` font objects (`ttf_ids`) into the PDF output even though no page content references `/F6`, bloating output size against `MAX_PDF_OUTPUT_BYTES`. Separately, in selectable mode's outlined fallback (`append_outlined_char` when `embedded_ttf` lacks a glyph), fallback characters are outlined one `char` at a time via `outline_visual_rich_text_rotated(&single, ...)`, stripping `last_base` context so combining marks cannot attach to their preceding base glyph.
- **Failure scenario:** Callers exporting PDF with `outline_text: true` and a custom `embedded_truetype_font` (e.g., CJK or domain-specific font) receive generic clean-room stroke outlines / tofu rectangles instead of their font's glyphs while still paying the full embedded TrueType compressed byte size inside the PDF.
- **Evidence:**
  ```rust
  // crates/pdf/src/writer.rs:423-425
      if let Some((ttf, font_id, desc_id, stream_id)) = ttf_ids {
          write_truetype_font_objects(&mut pdf, &mut offsets, ttf, font_id, desc_id, stream_id);
      }
  ```
  ```rust
  // crates/pdf/src/writer.rs:1095-1104
      fn emit_outlined_text(&mut self, text: &PositionedText) -> Result<(), PdfError> {
          let lines = split_rich_text_lines(&text.rich);
          let line_step = text.font_size_pt * 1.20;
          let rad = text.rotation_deg.to_radians();
          let (sin_a, cos_a) = rad.sin_cos();

          for (line_idx, line) in lines.iter().enumerate() {
              let visual_line: Cow<'_, RichText> = if rich_contains_rtl(line) { Cow::Owned(reorder_rich_text_bidi(line)) } else { Cow::Borrowed(line) };
              let outlines = outline_visual_rich_text_rotated(&visual_line, 0.0, 0.0, text.font_size_pt, 0.0);
  ```
  ```rust
  // crates/pdf/src/writer.rs:1388-1394
      let single = RichText::new(vec![RichSpan { text: ch.to_string(), ..span.clone() }]);
      let mut glyphs = outline_visual_rich_text_rotated(&single, pen_x, 0.0, base_font_size_pt, 0.0);
      let adv = glyphs.iter().map(|g| g.advance_x_pt).sum();
      if let Some(dst) = out {
          dst.append(&mut glyphs);
      }
      adv
  ```
- **Suggested fix:** Thread `Option<&TrueTypeFont<'_>>` through the outlined line builder (`emit_outlined_text`) so `outline_text: true` uses `embedded_ttf` first, skip resolving/writing `/FontFile2` objects when `options.outline_text` is `true`, and outline contiguous non-WinAnsi runs together instead of isolated single `char`s.
- **Related:** `FA-SEC10-01`, `FA-SEC10-04`

---

### FA-SEC10-03 (S2) — `RenderTargetSpec::from_scene_capped` clamps axes independently while retaining raw DPI (`[REGRESSION vs #9]`)
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/render/src/rasterizer.rs:128-140` (callers in `crates/mcp/src/formats.rs:1118, 1140`)
- **Category:** `wrong-result`
- **Defect:** Closed local audit item `#9` fixed DPI scaling distortion in `RenderTargetSpec::from_scene_dpi` (`rasterizer.rs:87-117`) and `crates/engine/src/cmd/export.rs` so oversized exports return an error instead of distorting aspect ratio. However, `RenderTargetSpec::from_scene_capped` still clamps `width_px` and `height_px` independently to `[64, 8192]` (`scaled.clamp(64.0, 8192.0)`) while leaving `dpi` unmodified. Both `McpPngExporter` and `McpTiffExporter` call `from_scene_capped` during document PNG/TIFF export.
- **Failure scenario:** Exporting a multi-panel or wide figure at `1200 DPI` (e.g., `10 in x 4 in` -> `12000 x 4800 px`) through MCP PNG/TIFF export silently clamps width to `8192 px` while keeping height at `4800 px`, squashing the plot horizontally and tagging the distorted image with `1200 DPI` physical resolution metadata (`pHYs` / TIFF tags).
- **Evidence:**
  ```rust
  // crates/render/src/rasterizer.rs:128-140
      pub fn from_scene_capped(scene: &ChartScene, requested_width_px: Option<u32>, requested_height_px: Option<u32>, dpi: u16) -> Self {
          let scale_axis = |dim_pt: f64, fallback_px: u32| -> u32 {
              if dim_pt.is_finite() && dim_pt > 0.0 {
                  let scaled = (dim_pt * f64::from(dpi) / 72.0).round();
                  scaled.clamp(f64::from(MIN_CAPPED_RASTER_DIM_PX), f64::from(MAX_CAPPED_RASTER_DIM_PX)) as u32
              } else {
                  fallback_px
              }
          };
          let default_w = scale_axis(scene.width_pt, DEFAULT_CAPPED_RASTER_WIDTH_PX);
          let default_h = scale_axis(scene.height_pt, DEFAULT_CAPPED_RASTER_HEIGHT_PX);
          Self { width_px: requested_width_px.unwrap_or(default_w), height_px: requested_height_px.unwrap_or(default_h), dpi }
      }
  ```
  ```rust
  // crates/mcp/src/formats.rs:1118 & 1140
          let spec = RenderTargetSpec::from_scene_capped(&scene, self.cfg.width_px, self.cfg.height_px, self.cfg.dpi);
  ```
- **Suggested fix:** Replace `from_scene_capped` in publication/file exporters (`mcp/src/formats.rs`) with `RenderTargetSpec::from_scene_dpi` (or uniformly scale both axes plus effective `dpi` when a caller explicitly requests bounded preview sizing).
- **Related:** `local-code-audit-2026-11-18.md` `#9`

---

### FA-SEC10-04 (S2) — Outlined text pipeline ignores `SystemFontSet` and renders CJK, Arabic, Hebrew, and unmapped scripts as tofu rectangles
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/text/src/glyph_paths.rs:124, 876-882`; `crates/render/src/rasterizer.rs:2917`; `crates/svg/src/writer.rs:593`; `crates/pdf/src/writer.rs:1103`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Under the repository's clean-room default build (`CURVEBOOK_FONTS_DIR` unset), `BUNDLED_FONTS` is empty (`&[]`). `outline_rich_text` (used by `rasterizer.rs` and `svg/writer.rs` when `outline_text: true`) and `outline_visual_rich_text_rotated` (used by `pdf/writer.rs` when `outline_text: true` or non-WinAnsi fallback runs) only call `select_bundled_font(span.weight, span.slope)` and never query `SystemFontSet` (`crates/text/src/shaping.rs`). When `BUNDLED_FONTS` is empty, `build_glyph_strokes` falls back to identical `0.12..0.88` placeholder rectangles for every character not in the ASCII/Greek/Cyrillic/math vector stroke tables.
- **Failure scenario:** Any chart title, legend, annotation, or axis label containing CJK, Arabic, Hebrew, Devanagari, IPA, or extended Unicode symbols renders entirely as rectangular boxes (`□`) in PNG/TIFF raster exports, `outline_text: true` SVG exports, and `outline_text: true` / non-WinAnsi PDF exports whenever built without `CURVEBOOK_FONTS_DIR`.
- **Evidence:**
  ```rust
  // crates/text/src/glyph_paths.rs:124
          let bundled = select_bundled_font(span.weight, span.slope);
  ```
  ```rust
  // crates/text/src/glyph_paths.rs:876-882
  fn build_glyph_strokes(ch: char) -> Vec<StrokeItem> {
      if let Some(items) = try_build_glyph_strokes(ch) {
          return items;
      }
      // Fallback box for arbitrary unmapped glyphs
      vec![StrokeItem::Ring(vec![(0.12, 0.04), (0.12, 0.68), (0.88, 0.68), (0.88, 0.04)])]
  }
  ```
  ```rust
  // crates/render/src/rasterizer.rs:2917 & crates/svg/src/writer.rs:593
      let outlines = outline_rich_text(line, 0.0, y_line, text.font_size_pt);
  ```
- **Suggested fix:** Add a font-resolver/face-lookup path (`outline_rich_text_with_fonts` / `SystemFontSet` fallback) so PNG/TIFF rasterization and outlined SVG/PDF exporters resolve missing script glyphs from loaded system/user TrueType faces before emitting placeholder boxes.
- **Related:** `FA-SEC10-02`

---

### FA-SEC10-05 (S2) — TrueType glyph decoder silently clamps `num_contours > MAX_TTF_CONTOURS` and misreads remaining table bytes as instructions/points
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/text/src/glyph_paths.rs:20, 379-382, 399-411`
- **Category:** `wrong-result` / `robustness`
- **Defect:** In `decode_ttf_glyph_recursive`, when a simple glyph header declares `num_contours > MAX_TTF_CONTOURS` (`256`), `count` is clamped via `.min(MAX_TTF_CONTOURS)` instead of rejecting the glyph. `decode_simple_ttf_glyph` then reads only `256` `endPtsOfContours` entries and computes `inst_len_off = 10 + 256 * 2` directly inside the unread portion of the actual `endPtsOfContours` array, misinterpreting a contour endpoint index as `instructionLength` and parsing subsequent contour endpoints as point flags and `(dx, dy)` coordinates.
- **Failure scenario:** Loading a complex or malformed TrueType glyph with `> 256` contours silently renders corrupted polygons/lines rather than returning `None` and falling back cleanly.
- **Evidence:**
  ```rust
  // crates/text/src/glyph_paths.rs:20 & 379-382
  const MAX_TTF_CONTOURS: usize = 256;
  ...
      if num_contours > 0 {
          let count = usize::try_from(num_contours).ok()?.min(MAX_TTF_CONTOURS);
          return decode_simple_ttf_glyph(slice, count, affine, scale, origin_x, origin_y, slant, state, out);
      }
  ```
  ```rust
  // crates/text/src/glyph_paths.rs:399-411
      let mut end_pts = Vec::with_capacity(num_contours);
      for i in 0..num_contours {
          end_pts.push(usize::from(read_u16_be(slice, 10 + i * 2)?));
      }
      ...
      let inst_len_off = 10 + num_contours * 2;
      let inst_len = usize::from(read_u16_be(slice, inst_len_off)?);
  ```
- **Suggested fix:** Return `None` whenever `usize::try_from(num_contours)? > MAX_TTF_CONTOURS` instead of truncating `count`.
- **Related:** None

---

### FA-SEC10-06 (S3) — EMF exporter skips BiDi RTL reordering, silently coerces `dpi == 0`, and lacks primitive/output bounds
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/render/src/emf.rs:113-116, 149-155, 548-591, 645-654`
- **Category:** `wrong-result` / `robustness` / `resources`
- **Defect:**
  1. `EmfWriter::emit_text` iterates `for line in &lines` and `for span in &line.spans` without checking `rich_contains_rtl` or calling `reorder_rich_text_bidi`, advancing `cursor_x` left-to-right across logical spans and misordering RTL / mixed-BiDi multi-span text relative to raster, PDF, and outlined SVG exports.
  2. `encode_raster_emf` silently clamps `image.dpi.max(1)` instead of validating `image.dpi == 0` as `RenderError::InvalidTarget` (unlike `RasterImage::validated_rgba` used by PNG and TIFF exporters).
  3. Unlike `pdf` (`MAX_PDF_TOTAL_PRIMITIVES`, `MAX_PDF_OUTPUT_BYTES`) and `rasterizer`, `encode_emf_with_raster` sets no upper limit on scene primitive count or EMF stream size before building all GDI records (`u32` byte lengths only checked at final serialization).
- **Failure scenario:** Exporting mixed RTL/LTR labels to EMF reverses span placement and visual character order; passing a `dpi: 0` `RasterImage` silently produces an unintended 1-DPI giant frame instead of an `InvalidTarget` error; pathological huge scenes are unrestricted until hitting memory or 4 GiB EMF header bounds.
- **Evidence:**
  ```rust
  // crates/render/src/emf.rs:113-116
  pub fn encode_raster_emf(image: &RasterImage) -> Result<Vec<u8>, RenderError> {
      let dpi = f64::from(image.dpi.max(1));
      let width_pt = f64::from(image.width_px) * 72.0 / dpi;
      let height_pt = f64::from(image.height_px) * 72.0 / dpi;
  ```
  ```rust
  // crates/render/src/emf.rs:561-589
          for (line_idx, line) in lines.iter().enumerate() {
              ...
              for span in &line.spans {
                  ...
                  cursor_x += span_w;
              }
          }
  ```
- **Suggested fix:** Call `reorder_rich_text_bidi(line)` when `rich_contains_rtl(line)` (matching raster/PDF), reject `image.dpi == 0` with `RenderError::InvalidTarget`, and enforce explicit primitive/output byte budgets in `encode_emf_with_raster`.
- **Related:** None

---

### FA-SEC10-07 (S3) — SVG `embed_fonts` `@font-face` names do not match default `<text font-family>` and CDATA lacks `]]>` escaping
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/svg/src/writer.rs:24, 210-237, 502, 784-797`
- **Category:** `contract-drift` / `robustness`
- **Defect:** When `options.embed_fonts` is `true`, `!options.outline_text`, and `BUNDLED_FONTS` is populated, `write_defs` emits `@font-face { font-family: "{name}"; ... }` using each bundled font entry's name (such as `"BIZUDPGothic-Regular"`), while `write_editable_text` unconditionally sets `<text ... font-family="Helvetica, Arial, sans-serif">` (`DEFAULT_FONT_FAMILY`) and only sets `font-family` on `<tspan>` when `span.font_family` explicitly overrides it. Furthermore, `escape_css_string` escapes `"` and `\` inside `<style type="text/css"><![CDATA[...]]></style>` without sanitizing `]]>`, allowing a font name containing `]]>` to break out of the XML CDATA section.
- **Failure scenario:** Even when built with `CURVEBOOK_FONTS_DIR` and `embed_fonts: true`, editable SVG `<text>` labels without an explicit per-span `font_family` matching a bundled font name do not select the embedded `@font-face` resource in browsers or vector editors.
- **Evidence:**
  ```rust
  // crates/svg/src/writer.rs:24 & 227-235 & 502
  const DEFAULT_FONT_FAMILY: &str = "Helvetica, Arial, sans-serif";
  ...
      if embed_faces {
          let _ = write!(out, "{indent2}<style type=\"text/css\"><![CDATA[{nl}");
          for (name, bytes) in BUNDLED_FONTS {
              let b64 = encode_base64(bytes);
              let family = escape_css_string(name);
              let _ = write!(out, "@font-face {{ font-family: \"{family}\"; src: url(\"data:font/ttf;base64,{b64}\"); }}{nl}");
          }
          let _ = write!(out, "]]></style>{nl}");
      }
  ...
      "<text x=\"{x}\" y=\"{y}\" font-family=\"{DEFAULT_FONT_FAMILY}\" font-size=\"{font_size}\" text-anchor=\"{anchor}\" fill=\"{color}\""
  ```
  ```rust
  // crates/svg/src/writer.rs:784-797
  fn escape_css_string(raw: &str) -> String {
      let mut out = String::with_capacity(raw.len());
      for ch in raw.chars() {
          if !is_valid_xml_char(ch) {
              continue;
          }
          match ch {
              '"' => out.push_str("\\\""),
              '\\' => out.push_str("\\\\"),
              _ => out.push(ch),
          }
      }
      out
  }
  ```
- **Suggested fix:** Prepend embedded `BUNDLED_FONTS` family names to the `<text font-family="...">` fallback stack whenever `embed_faces` is active, and sanitize or escape `]]>` before embedding text inside `<![CDATA[...]]>`.
- **Related:** `FA-SEC10-04`

---

### FA-SEC10-08 (S3) — Rasterizer collapses stroked ellipses within 5% eccentricity into circles
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/render/src/rasterizer.rs:1151-1156, 1171-1177`
- **Category:** `wrong-result`
- **Defect:** `draw_ellipse_fast` computes `is_near_circle = (ellipse.rx - ellipse.ry).abs() <= 0.05 * ellipse.rx.max(ellipse.ry).max(1.0)` and accepts stroked solid ellipses satisfying that check; then inside the stamp branch (`1172-1177`), whenever `stroke_color.is_some()`, it replaces distinct `(ellipse.rx, ellipse.ry)` with the arithmetic mean `(r, r)` (`0.5 * (ellipse.rx + ellipse.ry)`). Unstroked or dashed ellipses with the exact same radii retain elliptical `(rx, ry)`.
- **Failure scenario:** Small or mildly elliptical stroked markers, bubbles, or confidence ellipses with `|rx - ry| <= 5%` (or sub-pixel ellipses bounded by `.max(1.0)`) are rendered as symmetric circles when stroked, misrepresenting unequal x/y uncertainty or aspect ratio.
- **Evidence:**
  ```rust
  // crates/render/src/rasterizer.rs:1151-1156 & 1171-1177
      let is_near_circle = (ellipse.rx - ellipse.ry).abs() <= 0.05 * ellipse.rx.max(ellipse.ry).max(1.0);
      if let Some(sp) = stroke_paint
          && (sp.dash != LineDashStyle::Solid || !is_near_circle)
      {
          return false;
      }
  ...
      if half_w <= MAX_STAMP_RADIUS_PX && half_h <= MAX_STAMP_RADIUS_PX {
          let (rx_eff_raw, ry_eff_raw) = if stroke_color.is_some() {
              let r = 0.5 * (ellipse.rx + ellipse.ry);
              (r, r)
          } else {
              (ellipse.rx, ellipse.ry)
          };
  ```
- **Suggested fix:** Gate stroked circular stamp caching on exact or sub-0.25-px equality (`(ellipse.rx - ellipse.ry).abs() < 1e-4`), letting slightly elliptical stroked shapes fall through to the general path stroke rasterizer (`return false`).
- **Related:** None

---

### FA-SEC10-09 (S3) — `SystemFontSet::reorder_rich_line` leaves multi-span LTR segments out of visual order when `direction == TextDirection::Rtl`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/text/src/shaping.rs:828-840`
- **Category:** `wrong-result`
- **Defect:** When `direction == TextDirection::Rtl` and `!rich_contains_rtl(line)`, `reorder_rich_line` maps `reorder_visual_bidi(&span.text, TextDirection::Rtl)` independently over each span while leaving `line.spans` in original logical order, instead of reordering spans across the entire line under an explicit RTL base direction.
- **Failure scenario:** An RTL-aligned annotation containing multiple styled Latin/numeric spans without Hebrew/Arabic characters (e.g., `"10 "` + bold `"mg"` + `": "`) keeps its spans in left-to-right order while only reversing neutral punctuation inside individual spans, yielding mismatched layout vs line-wide BiDi shaping.
- **Evidence:**
  ```rust
  // crates/text/src/shaping.rs:828-840
  fn reorder_rich_line<'a>(line: &'a RichText, direction: TextDirection) -> Cow<'a, RichText> {
      if direction == TextDirection::Rtl || rich_contains_rtl(line) {
          if direction == TextDirection::Rtl && !rich_contains_rtl(line) {
              let reordered_spans =
                  line.spans.iter().map(|span| RichSpan { text: reorder_visual_bidi(&span.text, TextDirection::Rtl), ..span.clone() }).collect();
              Cow::Owned(RichText::new(reordered_spans))
          } else {
              Cow::Owned(reorder_rich_text_bidi(line))
          }
      } else {
          Cow::Borrowed(line)
      }
  }
  ```
- **Suggested fix:** Add a `reorder_rich_text_bidi_with_direction(line, direction)` helper (or pass `Some(direction)` into `reorder_rich_text_bidi`) so multi-span lines use line-level BiDi run reordering rather than per-span reordering.
- **Related:** None

---

### FA-SEC10-10 (S4) — `load_faces_from_dir` manifest check rejects `".."` substrings while allowing absolute path entries
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/text/src/shaping.rs:1011-1019`
- **Category:** `robustness`
- **Defect:** When reading `manifest.txt` in a font directory, `load_faces_from_dir` filters lines using `if rel.is_empty() || rel.contains("..") { continue; }` before calling `base_dir.join(rel)`. In Rust, `Path::join` with an absolute path discards `base_dir`, so an absolute path in `manifest.txt` bypasses the directory root, whereas a valid relative filename containing `".."` is rejected.
- **Failure scenario:** A malformed or third-party font pack `manifest.txt` referencing `/some/other/path/font.ttf` escapes `base_dir`, while a filename containing consecutive dots is unnecessarily skipped by manifest ordering.
- **Evidence:**
  ```rust
  // crates/text/src/shaping.rs:1011-1019
                  let rel = line.split('#').next().unwrap_or_default().trim();
                  if rel.is_empty() || rel.contains("..") {
                      continue;
                  }
                  let candidate = base_dir.join(rel);
                  if load_font_file(&candidate, tier, false, out) {
  ```
- **Suggested fix:** Validate manifest paths using `std::path::Path::new(rel).components()` to permit only `Component::Normal(_)` entries (rejecting `ParentDir`, `RootDir`, `Prefix`, and empty paths).
- **Related:** None

---

### FA-SEC10-11 (S4) — Unused `curvebook-math` workspace dependency in `render`, `svg`, and `pdf` crates
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/render/Cargo.toml:19`; `crates/svg/Cargo.toml:18`; `crates/pdf/Cargo.toml:17`
- **Category:** `cleanup`
- **Defect:** All three output crates declare `curvebook-math = { workspace = true }` under `[dependencies]`, yet zero references to `curvebook_math` exist in source or test files of `crates/render`, `crates/svg`, or `crates/pdf`.
- **Failure scenario:** Unnecessary workspace crate edges slow incremental builds and broaden dependency graphs for downstream consumers without any functional use.
- **Evidence:**
  ```toml
  # crates/render/Cargo.toml:19
  curvebook-math = { workspace = true }

  # crates/svg/Cargo.toml:18
  curvebook-math = { workspace = true }

  # crates/pdf/Cargo.toml:17
  curvebook-math = { workspace = true }
  ```
- **Suggested fix:** Remove `curvebook-math = { workspace = true }` from `crates/render/Cargo.toml`, `crates/svg/Cargo.toml`, and `crates/pdf/Cargo.toml`.
- **Related:** None
