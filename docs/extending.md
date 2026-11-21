# Extending CurveBook

Follow the existing seams below whenever adding a capability. Every user-facing action must preserve **headless-first parity** (accessible via `curvebook-engine`, CLI, MCP, and GUI where applicable) and satisfy [AGENTS.md](../AGENTS.md) and [docs/pitfalls.md](pitfalls.md).

## Extension Matrix ("You want to add -> Where -> How")

### Engine command (`CommandSpec`)

- **Where:** `crates/engine/src/cmd/{project,edit,sheet,table,info,analysis,graph,layout,export}.rs` (per-area `specs()` slices), assembled by `build_registry()` / `cmd::registry()` in `crates/engine/src/cmd/mod.rs`
- **How:** Name the ID per [docs/command-ids.md](command-ids.md) (`<area>.<camelCaseAction>`). Add one `CommandSpec` to the owning area's `specs()` slice with its `CommandGroup`, accurate `Effects` bitflags (e.g. `MUTATES_DOCUMENT`, `WRITES_FILESYSTEM`, `MAY_DISCARD_DATA`, `IDEMPOTENT`; read-only commands carry no mutating flag, see `Effects::read_only_hint`), `input_schema`, `enabled` predicate, and typed `run` handler (parameters parsed via `serde_path_to_error`). Never rename shipped IDs. Add sweep/unit tests (`crates/engine/tests/command_sweep_props.rs`, `crates/engine/tests/registry_and_errors.rs`) and run `cargo xtask commands` / `cargo xtask commands --check`.

### UI command (`UiCommandSpec`)

- **Where:** `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`) + `crates/ui-egui/` menu/panel wiring + generated `crates/mcp/src/ui_catalog.rs`
- **How:** Choose a `ui.*` (or `app.*`) ID per [docs/command-ids.md](command-ids.md). Add a `UiCommandSpec` entry using its named fields (`id`, `label`, `shortcut`, `params_doc`, `summary`, `beginner`). Wire its UI handler in `crates/ui-egui`. Run `cargo xtask ui-commands` so `crates/mcp/src/ui_catalog.rs` regenerates cleanly, then verify `cargo xtask ui-commands --check`.

### Cursor

- **Where:** `crates/tools/src/cursors.toml`, `crates/tools/src/lib.rs` (`Cursor` enum), `UiApp::active_cursor` in `crates/ui-egui/src/lib.rs`
- **How:** Add the `Cursor` variant in `crates/tools/src/lib.rs` and a matching `[[cursor]]` entry (`name`, `hotspot`, `when`) in `crates/tools/src/cursors.toml`; select it from `UiApp::active_cursor`. Record any new cursor image files in [ASSETS.md](../ASSETS.md) and run `cargo xtask cursors` + `cargo xtask assets`.

### Statistical / Scientific `AnalysisKind`

- **Where:** `crates/doc/src/analysis.rs` (`AnalysisKind`, typed spec and result structs), `crates/doc/src/analysis/descriptor.rs` (`AnalysisKindId`, `AnalysisKindDescriptor`, `ALL_ANALYSIS_DESCRIPTORS`), and `crates/doc/src/analysis/finite.rs`; `crates/stats` / `crates/fit` / `crates/transform`; `crates/engine` (`dag.rs`, `cmd/analysis.rs`, `methods_report.rs`); `crates/chart` when the result is graphable; `crates/ui-egui` (`dialogs_canvas.rs`, `inspect.rs`, `egui_shell/`); `xtask/src/docs_site/registries.rs`.
- **How:** [`ALL_ANALYSIS_DESCRIPTORS`](../crates/doc/src/analysis/descriptor.rs) is the declarative single source of truth for `AnalysisKind` metadata, guarded by zero-wildcard compile-time exhaustive matches in `AnalysisKindId::from_kind` and `AnalysisKindId::descriptor`. Adding a variant makes the compiler stop at every *exhaustive* `match` listed below; check the remaining *wildcard* sites by hand.
  1. **Model & descriptor (`curvebook-doc`).** Add the typed spec struct and variant in `crates/doc/src/analysis.rs`, keeping serde compatibility (follow [file-formats.md](file-formats.md) if a persisted shape changes). Compiler stops:
     - `AnalysisKind::derive_auto_title` (`crates/doc/src/analysis.rs`),
     - `AnalysisKindId::from_kind` and `AnalysisKindId::descriptor` (`crates/doc/src/analysis/descriptor.rs`, enforced at compile time alongside `ALL_ANALYSIS_DESCRIPTORS` uniqueness/bijection checks), and
     - `kind_first_non_finite_float` (`crates/doc/src/analysis/finite.rs` — add a finiteness check for every user-set float).
     Add the matching `AnalysisKindId` variant and `AnalysisKindDescriptor` entry in `ALL_ANALYSIS_DESCRIPTORS` (`slug`, `serde_variant`, `title`, `short_label`, `description`, `domain`, `recompute_strategy`, `ui_category`, `compatible_table_kinds`, `canonical_shorthand`, `shorthands`, `canonical_param_keys`, `ui_catalog_ids`, `docs_anchor`), plus variant-level compatibility rules in `AnalysisKind::is_compatible_with` (`analysis.rs`) only if sub-method parameters refine `compatible_table_kinds` (`has_method_specific_table_rules: true`). Add a representative variant entry to `crates/doc/tests/analysis_descriptors.rs`.
  2. **References to other sheets or Info constants** (only if the spec holds hooks or sheet IDs). *Wildcard:* `AnalysisKind::hooked_sources` and `AnalysisKind::upstream_sheet_ids` (`crates/doc/src/analysis.rs`), `ProjectDocument::upstream_dependencies` (`crates/doc/src/project.rs`); `validate_analysis_hooks_and_bounds`, `detect_dependency_cycles`, and `break_hook_edge_in_result_sheets` (`crates/format/src/archive.rs`); `remap_analysis_hooks` (`crates/engine/src/cmd/helpers.rs` and `cmd/sheet.rs`); `count_analysis_hooks` (`crates/engine/src/cmd/project.rs`).
  3. **Math.** Implement it in `crates/stats`, `crates/transform`, or `crates/fit` with peer-reviewed citations, edge-case coverage, and `D1` oracle checks (NIST StRD or verified reference values).
  4. **Routing (`crates/engine/src/dag.rs`).** `recompute_result_sheet` reads `descriptor_for_kind(&existing.analysis.kind).recompute_strategy` (`AnalysisRecomputeStrategy`) exhaustively with zero wildcards: `Transform` -> `execute_analysis_transform` (`crates/transform/src/ops/mod.rs`), `Fit` -> `curvebook_fit::execute_analysis` (`crates/fit/src/result.rs`), `PowerWithInfoHooks` -> `resolve_power_analysis_with_info_hooks`, and `Stats` -> `curvebook_stats::run_analysis` (`crates/stats/src/lib.rs`).
     - A new statistics kind only needs an arm in `run_analysis` (compiler stop) plus `AnalysisRecomputeStrategy::Stats` in its descriptor.
     - A new transform or fit kind selects `AnalysisRecomputeStrategy::Transform` or `AnalysisRecomputeStrategy::Fit` in its descriptor and adds an arm in `execute_analysis_transform` or `curvebook_fit::execute_analysis`.
  5. **Engine and automation.**
     - Compiler stop: `describe_analysis_kind` (`crates/engine/src/methods_report.rs`) for the methods report text.
     - Command wiring (`crates/engine/src/cmd/analysis.rs`): the descriptor contract supplies `domain` (`AnalysisDomainKind`), `supports_table_kind` / `is_compatible_with`, `shorthands` (`descriptor_by_shorthand`), `canonical_param_keys`, `supports_multiplicity`, and `auto_selects_mv_numeric_columns`; wire per-kind constructor defaults, field overrides, and semantic checks in `resolve_kind_shorthand`, `infer_default_kind`, `apply_shorthand_kind_overrides`, and `validate_analysis_spec_semantics` (and check `validate_map_keys_for_kind` / `is_key_allowed_for_kind`).
     - Regenerate `docs/commands.md` and the schemas (`cargo xtask commands`, `cargo xtask schemas`) if any schema text changes.
  6. **Graphs** (only if the result can be plotted). *Wildcard:* the extractors in `crates/chart/src/` (`multivariate_layout.rs`, `survival_cat.rs`, `xy.rs`, `equation_box.rs`, `column_grouped.rs`).
  7. **GUI and documentation site.** `xtask/src/docs_site/registries.rs` (`analysis_kind_slug`, `all_analysis_kinds_meta`) and `xtask/src/docs_site/sitemap.rs` read directly from `ALL_ANALYSIS_DESCRIPTORS` and `descriptor_for_kind`; add the corresponding `{#analysis-<slug>}` entry to `docs/reference/analyses.md` (and `xtask/tests/r12_docs_site.rs`) so `cargo xtask docs-site --check` passes. The catalog and parameter dialogs in `crates/ui-egui` (`dialogs_canvas.rs`, `inspect.rs`, `egui_shell/`) use *wildcard* arms; expose the new parameters and result views there and document them for CLI and MCP users.

### Curve-fit `ParametricModel`

- **Where:** `crates/fit/src/library/{pharma,curves,nist}.rs` (resolved via `get_builtin_model` / `list_builtin_equations` in `crates/fit/src/library/mod.rs`)
- **How:** Implement `ParametricModel` (`id`, `name`, `category`, `param_names`, `eval`, optional analytical `grad`, deterministic `initial_guess`, optional `default_bounds` / `derived_transforms`) with peer-reviewed textbook reference in doc comments. Register it by adding its ID to the category's `*_EQUATION_IDS` list and resolver (`resolve_pharma_model` / `resolve_curve_model` / `resolve_nist_model`), add NIST StRD / `D1` validation tests (including bad/degenerate initial inputs returning typed `Err`, never panics).

### `Importer` / `Exporter` format

- **Where:** `crates/format/src/registry.rs` (`Importer` / `Exporter` traits, `FormatDescriptor`, `FormatRegistry::default_registry()`), parser modules such as `crates/format/src/{pzfx,prism,csv,export}.rs`; consumers `apps/curvebook-cli/src/cmd_convert.rs`, `apps/curvebook/src/desktop_host.rs`, `crates/mcp/src/formats.rs`
- **How:** Implement `Importer` / `Exporter` cleanly (no panics or unchecked indexing on adversarial/malformed inputs; document format assumptions neutrally in clean-room terms) with a `FormatDescriptor` (`id`, `label`, `extensions`, `kind`). Register it in `FormatRegistry::default_registry()` so CLI, desktop host, and MCP pick it up. Use `curvebook-platform` atomic writes for file outputs. Add round-trip and malformed-input tests (see `crates/format/tests/versioned_and_seams.rs`).

### Locale / UI string

- **Where:** `crates/ui-egui/src/i18n.rs` and registered catalog shards; [localization guide](localization.md)
- **How:** Add the key in the appropriate registered catalog or shard and provide translations across all active locale tables without duplicating existing keys. Run `cargo xtask i18n-report` (runs the `untranslated_report` test in `curvebook-ui-egui`) to list untranslated keys, and `cargo test -p curvebook-ui-egui`.

### Checked shared-state lock (`curvebook_platform::sync`)

- **Where:** `crates/platform/src/sync.rs` (`Mutex`, `RwLock`, `LockError`, `LockLimits`), enforced workspace-wide by `disallowed-types` in [`clippy.toml`](../clippy.toml)
- **How:** Prefer single-owner state or message passing first. When shared mutable state across threads is genuinely needed, wrap it in `curvebook_platform::sync::Mutex::named` or `RwLock::named` instead of `std::sync::{Mutex, RwLock}` (never place `#[allow(clippy::disallowed_types)]` outside `crates/platform/src/sync.rs`). Keep guards short-lived (never perform rendering, I/O, or callbacks while holding a lock), label intentionally long critical sections with `lock_long_job` / `read_long_job` / `write_long_job`, surface or recover from `LockError::{WouldDeadlock, TimedOut, Poisoned}` without `unwrap()` / `catch_unwind`, and verify `cargo xtask ci wasm` stays green (the wrapper avoids `Instant::now()` and watchdog threads on `wasm32-unknown-unknown`).

### Persisted-file version / migration

- **Where:** `crates/format/src/versioned.rs` (`VersionStep`, `upgrade`, `PREFERENCES_MIGRATION_STEPS`), `crates/format/src/archive.rs` (`MIGRATION_STEPS`), config loaders in `apps/curvebook/src/main.rs`
- **How:** Bump the format version constant (`CURRENT_SCHEMA_VERSION`, `CURRENT_PREFERENCES_VERSION`, or the envelope's `format_version`), add a deterministic forward `VersionStep` migration to the matching steps slice (a compile-time assert ties slice length to the version), verify that reading a newer unsupported version fails safely without overwriting disk state ([docs/file-formats.md](file-formats.md)), add checked-in golden fixture tests, and regenerate schemas with `cargo xtask schemas` (`--check` in CI).

### Asset (icon, image, SVG, sample)

- **Where:** Asset directory (never `.ttf`/`.otf`/`.woff`/`.woff2`!) + [ASSETS.md](../ASSETS.md) + [ATTRIBUTION.md](../ATTRIBUTION.md)
- **How:** Verify the asset licence matches [docs/licences.md](licences.md). Add the file (never commit binary font files). Add a metadata row to [ASSETS.md](../ASSETS.md) and third-party credit to [ATTRIBUTION.md](../ATTRIBUTION.md) if applicable. Run `cargo xtask assets` (and `cargo xtask ico` if Windows icon sources changed).
