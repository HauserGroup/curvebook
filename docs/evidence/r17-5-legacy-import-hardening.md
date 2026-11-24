# Legacy `.pzfx` and `.prism` Project Import Hardening (R17.5)

This record documents the clean-room audit, dialect hardening fixes, structured warning mappings, and consumer propagation completed for `.pzfx` XML and `.prism` ZIP container imports in CurveBook.

## 1. Scope & Modified Components

- Warning taxonomy: [`ImportWarningKind`](../../crates/format/src/error.rs) and [`ImportWarning`](../../crates/format/src/error.rs) in `crates/format/src/error.rs`.
- XML entity decoding: [`decode_xml_entities`](../../crates/format/src/xml.rs) / `lookup_named_xml_entity` in `crates/format/src/xml.rs`.
- Legacy XML project importer: `crates/format/src/pzfx.rs`.
- Legacy ZIP archive container importer: `.prism` archive parser module inside `crates/format` (`table.importPrism` backend).
- Command & CLI consumers: `crates/engine/src/cmd/table/import.rs` (`table.importPzfx`, `table.importPrism`) and `apps/curvebook-cli/src/cmd_convert.rs` (`curvebook convert` / `export` / `info`).
- Clean-room regression suites: [`r17_5_legacy_import_hardening.rs`](../../crates/format/tests/r17_5_legacy_import_hardening.rs) and [`r17_5_import_warning_propagation.rs`](../../crates/engine/tests/r17_5_import_warning_propagation.rs).

## 2. Catalogued `.pzfx` XML Quirks & Hardening

| Quirk / Dialect Case | Prior Behavior | Hardened Behavior | Warning Emitted |
| :--- | :--- | :--- | :--- |
| `YFormat="SDN"` | Mis-mapped to `MeanLowHigh` | Maps to `SubcolumnMode::MeanSdN` (`[Mean, SD, N]`); accepts `sd`, `sdn`, `sem`, `semn`, `sen`, `se`, `cv`, `cvn` case-insensitively | None when well-formed |
| `EVFormat="AsteriskAfterNumber"` / `AsteriskBeforeNumber` / `Asterisk` | Conflated with `YFormat` causing bogus `YFormat` warnings and ignoring `*` exclusions | Classified separately as excluded-value encoding; strips leading/trailing `*` when enabled by `EVFormat` or `Excluded` attribute and marks finite values `excluded: true`; falls back to subcolumn mode only when `YFormat` is absent and `EVFormat` is a summary-error token | None on valid asterisk masking |
| `YFormat="PlusMinus"` (`2` or `3` subcolumns) & swapped `UpperLower` / `low-high` limits | Unrecognized `PlusMinus`; reversed `[+Error, -Error]` or `[Upper, Lower]` subcolumns kept out of order | Expands 2-subcolumn `PlusMinus` (`[Mean, ±Err]`) symmetrically to `MeanLowHigh`; swaps subcolumns `1` and `2` when `<Title>` tags place Upper/Plus before Lower/Minus | `ImportWarningKind::CoercedValue` |
| 2-subcolumn summary modes (`SD`/`SEM`/`CV`/`low-high`/`upper-lower`/`median-iqr`) | Silently left 3rd slot empty | Retains `[Mean, Err]` and leaves 3rd slot (`N`/limit) empty with an explicit diagnostic | `ImportWarningKind::MissingMetadata` |
| `XFormat="text"` / `"labels"` / `"categories"` | Silently treated as empty/numeric X | Preserves text X cells verbatim and populates `row_titles` when `<RowTitlesColumn>` is absent | `ImportWarningKind::CoercedValue` |
| `XFormat="date"` / `"time"` / `"elapsed-time"` with string dates | Silently stored text without warning | Keeps `XColumnMode::DateOrElapsedTime` and string dates verbatim without calendar math | `ImportWarningKind::CoercedValue` |
| `XFormat="start-step"` / `"sequence"` (`<XColumn>` child elements, comma decimals, non-finite `X0`/`DeltaX`) | Checked only `<Table>` dot attributes | Reads `<Table>` and `<XColumn>` attrs/child elements, supports comma decimals, defaults non-finite values safely | `ImportWarningKind::CoercedValue` when defaulted |
| `XEVFormat` on `err`/`error` or 2-subcolumn X | Matched only exact `"SEM"`/`"CV"` at 3 subcolumns | Case-insensitive summary modes including `PlusMinus`, `UpperLower`, `MedianIqr`, and 2-subcolumn error X | `MissingMetadata` / `CoercedValue` as above |
| `<RowTitlesColumn>` `<Title>` & `Title="..."` attributes | `<RowTitlesColumn><Title>` dropped; attribute-only `Title="..."` ignored | Populates `GroupedTable::row_factor_title` and `ContingencyTable::row_factor_title`; supports `Title="..."` attribute fallback across root, info, table, and columns | None (lossless) |
| Direct `<d>` without `<Subcolumn>`, column/subcolumn `Excluded="1"\|"true"\|"yes"` | Required `<Subcolumn>` wrapper and cell-only `Excluded` | Falls back to direct `<d>` children as 1 subcolumn and propagates column/subcolumn exclusion flags | None (lossless) |
| Locale decimal commas (`1,25`, `1.234,56`), US thousands (`1,234.56`), Unicode minus `\u{2212}`, scientific notation | Comma numbers degraded to `CellValue::Text` | Parses Unicode minus, explicit `DecimalSeparator=","`, EU/US thousands, and unambiguous comma decimals in numeric cells, sequence params, and Info constants | `ImportWarningKind::CoercedValue` when comma notation is normalized |
| Non-numeric placeholders (`#N/A`, `Inf`, `1e500`, `NaN`, `NA`) & out-of-domain values (`Contingency`, `Survival`, `PartsOfWhole`) | Silently accepted without warning | Preserves raw cell text/numbers verbatim and flags non-numeric/domain mismatches per table | `ImportWarningKind::CoercedValue` |
| `<HugeTableSequence>` / `<HugeTable>` / `TableType="Huge"` | Skipped entirely | Imported as `TableKind::MultipleVariables` | `ImportWarningKind::UnsupportedTableType` |
| `<Notes>`, `<Note>`, `<Description>` on root, `<Info>`, and `<Table>` | Table/root notes dropped; only single `<Notes>` on `<Info>` read | Concatenates `<Info>` notes/descriptions; attaches `<Table>` notes to `DataSheet::floating_notes`; attaches root notes cleanly | None (lossless) |
| `SchemaVersion` / XML version attributes & unsupported sections (`*Sequence`, `<Analysis>`, `<Graph>`, `<Layout>`, `<Script>`, `<UserEquations>`) | Unchecked schema version; unwrapped `<Equation>`/`<Analysis>`/`<Graph>`/`<Layout>`/`<Script>` ignored silently | Flags unparseable or `> 10` dialect versions and warns on every wrapped or unwrapped unsupported section | `IgnoredSection`, `UnsupportedAnalysis`, `UnsupportedEquation`, `UnsupportedGraphOrLayout`, `UnsupportedScript`, `MissingMetadata` |
| Scientific & XHTML entities (`&copy;`, `&reg;`, `&trade;`, `&frac12;`, `&uarr;`, `&darr;`, `&sum;`, `&radic;`, `&part;`, `&int;`, `&Aring;`, typographic quotes) | Returned `XmlParse` unknown-entity error | Decoded to exact Unicode scalars in `crates/format/src/xml.rs` while continuing to reject arbitrary unknown entities (`&unknown;`) | None |

## 3. Catalogued `.prism` Container Quirks & Hardening

| Quirk / Container Case | Prior Behavior | Hardened Behavior | Warning Emitted |
| :--- | :--- | :--- | :--- |
| Single-folder nested ZIP root & case-variant entry paths (`Manifest.json`, `Data/Table1.CSV`, `Data/T1.PZFX`) | Missed nested manifests and mixed-case extensions | Detects optional shared root folder prefix cleanly and resolves manifest, `document.json`, info, `.csv`/`.tsv`, `.pzfx`, `.json`, and asset paths case-insensitively | None |
| External `document.json` (lacking CurveBook `"data_sheets"`) | Ignored or mishandled when `"title"` triggered table-metadata fallback | Parsed through unified manifest/project metadata loader supporting `projectTitle`/`name`, array or map `tables`/`dataTables`, `info`/`infoTables`, and array/map `constants` | Emits specific unsupported warnings for `analyses`/`graphs`/`layouts`/`equations`/`scripts` arrays |
| JSON sidecars in `analyses/`, `graphs/`, `layouts/`, `equations/`, `scripts/`, or unknown JSON with `"title"` | Swallowed silently by sidecar table metadata parser because all fields were optional | Explicit directory + schema-key routing before table-sidecar matching; never silently drops non-table JSON | `UnsupportedAnalysis`, `UnsupportedGraphOrLayout`, `UnsupportedEquation`, `UnsupportedScript`, or `UnsupportedChunk` |
| Directory-named table sidecars (`meta.json` / `sheet.json` / `table.json`) & sidecar-relative `./path.csv` | Only matched stem or root-exact `csv_path` | Resolves relative `csv_path` against both archive root and sidecar parent directory, plus parent-directory match for `meta.json`/`sheet.json`/`table.json` | None when matched |
| Missing `csv_path` target declared in manifest/sidecar | Silently omitted table without notice | Detects unmatched explicit `csv_path` references after CSV pass | `ImportWarningKind::MissingMetadata` |
| Embedded SQLite blobs (`*.sqlite`, `*.sqlite3`, `*.db`, `SQLite format 3\0` magic) | Generic opaque skip message | Identifies embedded SQLite containers explicitly without adding SQLite crate dependencies | `ImportWarningKind::UnsupportedChunk` |
| Oversized titles/constants/units & disallowed subcolumn/replicate specs | Returned fatal `TitleTooLong` error or silently coerced invalid modes | Clamps titles/constants/units via `clamp_title`, coerces `replicates: 0` or disallowed subcolumn modes cleanly with diagnostics | `TruncatedTitle` / `CoercedValue` |
| UTF-8 BOM & multi-line quoted CSV headers | `row_titles` header peek only checked 2 raw lines and could keep BOM prefix | Strips UTF-8 BOM before detection/import and reads up to 16 lines for multi-line quoted `Row Title` / `Row Factor` / `Outcome / Exposure` headers | None |

## 4. Consumer Warning Propagation

- **Engine (`crates/engine/src/cmd/table/import.rs`)**: Consolidated `table.importPrism` and `table.importPzfx` completion into `finish_project_import_report`, preserving structured JSON warnings in `outcome.payload["warnings"]` while populating `CommandOutcome::warnings` with formatted `"[<Kind>] <source>: <message>"` strings for Engine, CLI batch/exec, and desktop host callers.
- **CLI (`apps/curvebook-cli/src/cmd_convert.rs`)**: Human-readable `curvebook convert` / `export` prints every warning in `loaded.warnings` to `stderr`, and `load_import_report` formats warnings with `[{kind:?}] {source}: {message}` matching `cmd_info.rs` JSON/human output.
- **MCP (`crates/mcp/src/formats.rs`)**: `open_file` serializes structured `ImportReport::warnings` directly into response payloads; `execute_command` surfaces `CommandOutcome::warnings`.

## 5. Verification

Verified with `CARGO_TARGET_DIR=/tmp/cb-r17-legacy` on:
- `cargo test -p curvebook-format -p curvebook-engine -p curvebook-mcp -p curvebook-cli`
- `cargo xtask brands`
- `cargo xtask cleanroom`
- `cargo xtask telemetry`
- `cargo xtask licenses`
- `cargo xtask wasm`
- `cargo xtask ci quick --base main`
