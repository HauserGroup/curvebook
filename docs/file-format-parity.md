# File Format & Interchange Parity (`lumencraft-format`, `lumencraft-svg`, `lumencraft-pdf`)

> Related specifications: [`data-tables-and-transforms.md`](data-tables-and-transforms.md), [`graphing-and-layouts.md`](graphing-and-layouts.md), [`scripting-and-automation.md`](scripting-and-automation.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G2`, `G5`, `G10`)

## Native FAIR-Aligned Project Formats
- [x] `.lumencraft` project container: deterministic ZIP archive containing `manifest.json` (`schema_version`), sheet catalog, standard CSV data tables, JSON Info constants, JSON parameterized analysis/fit specs, JSON graph/layout specs, and optional cached result snapshots (human-inspectable & FAIR-aligned)
- [x] `.lctemplate` reusable method & project template (empty or placeholder data tables pre-wired to transforms, statistical analyses, curve fits, styled graphs, and figure layouts)
- [x] Forward/backward schema migration harness, circular-hook recovery guard, and crash-recovery journal (`MIGRATION_STEPS`, `sanitize_circular_hooks`, and `CrashRecoveryJournal` WAL + torn-tail replay via desktop `DirectoryJournalBackend` & web-storage JSON model `MemoryJournalBackend` verified in `recovery_and_interop_parity`; browser `wasm32` Web Locks runtime awaits `wasm32` sysroot in `G6`)

## Tabular & Scientific Project Interchange
- [x] RFC-4180 CSV and TSV import/export with configurable delimiter (`,` / `;` / `\t`), quote character, header row count, missing-value sentinels (`NA`, `NaN`, blank), and decimal separator (`.` vs. `,`)
- [x] Clean-room `.prism` ZIP container import (reading standard CSV data sheets + JSON sheet/table metadata from public format conventions; bounded ZIP entry count and uncompressed byte ceiling)
- [x] Clean-room legacy `.pzfx` XML import (data tables, replicate subcolumn layouts, row/column titles with super/subscripts, excluded-cell masks, and Info constants; bounded XML nesting depth, zero external entities)
- [x] Tidy CSV and structured JSON export of all `ResultSheet` statistical tables, post-hoc comparisons, residuals, and nonlinear regression parameters for R / Python / Snakemake / Nextflow pipelines

## Publication Figure Export
- [x] Vector **SVG** export (`lumencraft-svg`) with editable `<text>` (preserving super/subscripts) or outlined glyph paths, crisp clip regions, and deterministic output *(structurally validated in `lumencraft-svg` tests & CLI Slice 0; interactive GUI viewer inspection not performed locally)*
- [x] Direct vector **PDF 1.7** export (`lumencraft-pdf`) with optional embedded SFNT `/FontFile2` fonts (or Core-14 + outlined fallback), sRGB or print CMYK colour spaces, and exact `MediaBox` / `TrimBox` / `BleedBox` physical dimensions *(pure-Rust `%PDF-1.7` default / `--pdf-version 1.4|1.7`, optional `/FontFile2` SFNT embedding via `--ttf-font` or `LUMENCRAFT_FONTS_DIR`, Core-14 Helvetica/Symbol + outlined fallback, `FlateDecode`, sRGB/CMYK/Grayscale, `/MediaBox`/`/TrimBox`/`/BleedBox`, and 20-byte `xref` verified in `lumencraft-pdf`; manual GUI viewer QA with external `.ttf` fonts remains in `G5`)*
- [x] High-DPI raster **PNG** and uncompressed/LZW **TIFF** export (`72`–`1200` DPI, transparent, white, or custom paper fill)
