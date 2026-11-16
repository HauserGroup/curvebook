# Localization Guide

How CurveBook's interface strings, locales, text direction, and fonts work today, and how to extend them. Everything here describes code that exists in the repository; planned work is labelled **planned**. Mutable totals (locale count, key count) are deliberately not repeated here: read them from `LOCALES` / `catalog_len()` in [`crates/ui-egui/src/i18n.rs`](../crates/ui-egui/src/i18n.rs), from the `cargo xtask i18n-report` output, or from [STATUS.md](STATUS.md) (see [pitfalls.md](pitfalls.md), item 2).

Related: [extending.md](extending.md) (extension matrix), [localization-parity.md](localization-parity.md) (parity checklist and the human-GUI caveat), [licences.md](licences.md) and [../ASSETS.md](../ASSETS.md) (font and asset rules).

---

## 1. Where the strings live

Interface translations use a static Rust catalog rooted in `crates/ui-egui/src/i18n.rs`:

- `LOCALES` is the ordered list of registered BCP-47 tags. English (`en`) is first and is the reference locale.
- `CATALOG` and the slices registered in `EXTRA_SHARD_CATALOGS` contain `CatalogEntry { key, translations }` rows. `all_entries()` combines them for lookup and reporting. `translations` has one slot per registered locale, in `LOCALES` order. Read the registered shard modules before choosing where to add an entry; parallel UI work can add further shards.
- Entries are written with the `entry!` macro, which names every locale explicitly (`en:`, `de:`, ... `he:`), so a missing locale is a compile error rather than a silent gap.

There are no external catalog files (no `.po`, `.ftl`, or JSON): adding strings means editing Rust source.

## 2. Adding a key

1. Choose a dotted, lowercase-ish key that groups by area. Follow the neighbouring entries: `menu.*` for menus, `table.*` for data-table archetypes, and `cmd.<command_id>` for command labels (the id after `cmd.` must be the engine `CommandSpec` or `UiCommandSpec` id, see [command-ids.md](command-ids.md)).
2. Add an `entry!("your.key" => [ ... ])` to `CATALOG` or the appropriate registered shard with a non-empty string for **every** locale. Keys must be unique (a test fails on duplicates). Place it with related entries; do not reuse an existing key for a different meaning.
3. Use it from UI code:
   - `app.tr("your.key")` on `UiApp` (uses the app's current locale), or
   - `i18n::tr(locale, "your.key")` when you only have a locale tag.
   - `i18n::try_tr(locale, key)` returns `None` instead of falling back (useful for tests and tooling).
   - `i18n::tr_or(locale, key)` returns the key text itself when the key is not in the catalog, which makes a missing registration visible on screen instead of blank.
4. Pass the key as a plain string literal. A unit test (`tr_string_literal_calls_all_exist_in_catalog`) scans `crates/ui-egui/src` for string-literal `tr` / `try_tr` / `tr_or` calls and fails when a literal key is not registered. Keys built at run time (for example with `format!`) are not checked by that scan, so prefer literals.
5. Run the checks in section 8.

Adding or changing a command also means updating its label entry; the report test cross-checks every engine and UI command against the catalog (see section 8).

## 3. Placeholders and interpolation

CurveBook's catalog has **no interpolation layer**. `tr` returns a `&'static str` taken verbatim from the entry; there is no `{name}` substitution, plural-rule machinery, or format-argument validation in `i18n.rs`. Consequences for translators and authors:

- Write each catalog string as a complete, standalone phrase. Do not put placeholder tokens in catalog text expecting them to be filled in.
- When a message needs a value, keep a complete translated label separate from the formatted value. Do not concatenate translated sentence fragments in an English-specific order. Messages that require grammatical interpolation need a separately designed formatting mechanism before adding placeholder tokens.
- Numbers are not formatted by the catalog. Decimal separator, grouping, CSV delimiter, axis ticks and p-value text come from `LocalePreset` (built on `curvebook_text::LocaleNumberPolicy`), selected per locale by `LocalePreset::for_locale`. Format numbers through those helpers rather than embedding digits in translated text.
- Keep command labels and menu labels free of mnemonics or markup; shortcuts are rendered separately from `UiCommandSpec`.

If a future change adds real placeholders, it must add validation to the report test and update this section in the same PR.

## 4. Adding a locale

A locale is a new column in the catalog, so the change touches every entry. In `crates/ui-egui/src/i18n.rs`:

1. Append the tag to `LOCALES` and widen the `translations` array length in `CatalogEntry` to match.
2. Add the new field to the `entry!` macro pattern and its output array, in the same position as the new `LOCALES` slot.
3. Add a translation for the new locale to **every** `entry!` in `CATALOG` and every registered shard (including every `cmd.*` entry).
4. Add the tag to `exact_locale_index` (lower-cased, `-` separated) with the matching index, to `normalize_locale` (so regional variants resolve to it), and to `locale_display_name` (native autonym).
5. If it is a right-to-left language, check `is_rtl_locale` (section 6). If it needs a different number or CSV convention, extend `LocalePreset::for_locale`.
6. If it uses a script that no installed font may cover, add per-script handling in `curvebook-text` font selection (section 7) and, for CJK-style fallbacks, `system_cjk_font_candidates` in `crates/text/src/metrics.rs`.
7. Update the hard-coded expectations in the `untranslated_report` test (it pins the expected locale list and totals) and in the locale-normalization tests. The test file is the authority on which assertions exist; read it before editing.
8. Update [localization-parity.md](localization-parity.md) and the extension row in [extending.md](extending.md) if the process changes. Do not add a count to prose.

Fallback behaviour for an incomplete locale is described in section 5, but note that the `untranslated_report` test currently requires **zero** missing or empty entries in every registered locale, so a new locale cannot land partially translated.

## 5. Locale matching and fallback order

Matching is done by `normalize_locale(tag)`:

1. Trim; an empty tag is `en`.
2. Underscores become hyphens and the tag is lower-cased.
3. Only the primary subtag is used, with these special cases: `pt` (any region) maps to `pt-BR`; `zh` maps to `zh-TW` when a `tw`, `hk`, `mo`, or `hant` subtag is present and to `zh-CN` otherwise; `iw` (legacy) maps to `he`.
4. Any primary subtag that is not registered maps to `en`.

Lookup is done by `tr(locale, key)`:

1. Normalize the locale.
2. Return the entry for that locale if it exists and is non-blank.
3. Otherwise return the `en` entry.
4. Otherwise return the empty string (unknown key). `tr_or` substitutes the key itself in that last case.

Command labels follow the same order through `command_label(locale, command_id, fallback)`: locale entry, then `en`, then the caller-supplied fallback.

The active locale is stored on `UiApp` and changed through the control/MCP operation `ui.locale.set`, which normalizes the tag. Example: `de-DE` selects `de`; `zh-Hant-TW` selects `zh-TW`; `unknown-XX` selects `en`.

## 6. Right-to-left locales

- `is_rtl_locale(tag)` is true for primary subtags `ar`, `he`, `iw`, `fa`, and `ur`, including regional forms such as `ar-EG`. Only `ar` and `he` have catalogs today. The direction helper recognizes `fa` and `ur`, but active-locale normalization maps these unsupported tags to `en`; that recognition does not provide a Persian or Urdu interface.
- Layout mirroring uses `mirror_x_for_rtl(x, width, canvas_width, is_rtl)`, which uses saturating arithmetic. The grid hit-testing code in `crates/ui-egui/src/interaction.rs` applies it so columns and gutters mirror under an RTL locale.
- Text shaping applies bidirectional reordering for RTL locales (`reorder_visual_bidi` / `TextDirection` in `curvebook-text`), used by `FontBook::rasterize_shaped_text` and the PDF/SVG export paths.
- The control-protocol state response reports `is_rtl` for the active locale (`crates/ui-egui/src/control_io.rs`).
- When you add UI, never hard-code left/right positions that bypass the mirroring helper, and test with `ar` and `he`.

Rendering and human usability are separate from catalog coverage. Check the current implementation and [localization-parity.md](localization-parity.md), together with the `R10.9` typography and `R10.21` translation acceptance work. A complete catalog entry does not prove that a person sees correct glyphs, direction, clipping, or layout in a rendered screen.

## 7. Fonts and glyph coverage

CurveBook ships **zero font files** (enforced by `cargo xtask assets`; see [../AGENTS.md](../AGENTS.md)). Coverage therefore depends on fonts that exist at run time:

- `SystemFontSet` in `crates/text/src/shaping.rs` discovers faces in this order: (1) a directory named by the `CURVEBOOK_FONTS_DIR` environment variable, read at run time on native targets; (2) faces compiled in via the optional `bundled-fonts` feature, if any; (3) host OS font files on native platforms (not on `wasm32`); (4) built-in deterministic metrics and vector outlines, which are not a substitute for a real font covering CJK, Arabic, or Hebrew.
- At build time `crates/text/build.rs` also reads `CURVEBOOK_FONTS_DIR` (expects `fonts/manifest.txt`) to embed external faces when `bundled-fonts` is enabled; `CURVEBOOK_FONTS_REQUIRED=1` makes a missing directory a build error. Those fonts stay outside the repository, and any OFL face you stage needs its licence text and attribution ([licences.md](licences.md)). [development.md](development.md) describes the build-time workflow.
- `font_diagnostics(locale)` in `crates/ui-egui/src/theme.rs` reports how many bundled faces are active, the CJK family names the locale would look for (`system_cjk_font_candidates`), and whether the locale routes as RTL. It does not probe the filesystem.
- Web: there is no host filesystem on `wasm32`, so system font discovery is skipped. The web host measures and draws interface text through the browser's own canvas text API and fonts (`BrowserCanvasTextMeasurer` in `apps/curvebook-web/src/text_bridge.rs`), and the page's own chrome uses the browser's `font-family`, so web glyph coverage depends on the viewer's browser and OS fonts. Optional faces listed in `crates/text/web-fonts.txt` can be staged beside the build with `cargo xtask web-fonts` when `CURVEBOOK_FONTS_DIR` is set; without it the command is a no-op. The web security policy forbids external font origins.

Practical limits to document for users: a locale whose script has no installed font may render with missing or substitute glyphs even though its catalog is complete. Before declaring a locale usable, confirm the target OS has a font for its script, or provide one through `CURVEBOOK_FONTS_DIR`.

## 8. Checking coverage: `cargo xtask i18n-report`

`cargo xtask i18n-report` (also run inside `cargo xtask ci`) runs exactly one test, `i18n::tests::untranslated_report` in `curvebook-ui-egui`, with output shown, and fails unless exactly that one test ran and passed. That test currently:

- checks the registered locale list, rejects duplicate keys, and fails if any locale has a missing or blank entry for any key;
- checks that every engine `CommandSpec` and every `UiCommandSpec` has a `cmd.<id>` entry whose English text equals the command's label, and that all locales have a non-blank entry;
- checks that Russian, Arabic, and Hebrew entries contain characters from their script (skipping the single key `table.xy`), which catches untranslated English left in those columns;
- prints a one-line summary of locales, keys, and verified translations.

The same file also contains the `tr` literal-key scan and normalization tests, run by `cargo test -p curvebook-ui-egui`.

Not covered today: the report cannot tell whether a non-English string was merely copied from English for scripts other than Cyrillic, Arabic, and Hebrew, and it does not look for user-visible literals that bypass `tr()`. A scan for untranslated user-visible literals is separate work coordinated with `R10.21`; do not treat the existing coverage report as that gate. Review new UI code for hard-coded text.

Because `untranslated_report` pins expected totals as literal assertions, adding a key or a command requires updating those assertions; keep any such numbers out of prose docs.

## 9. Review policy for machine-drafted translations

The repository has no field or flag in `CATALOG` that marks a translation as machine-drafted or reviewed, and no tool that enforces review. The following is the contribution policy for this guide; it is a process rule, not an automated gate.

- Machine-drafted translations (from any translation tool or language model) are acceptable as a starting point only when disclosed. Say so in the pull request description, naming the locales and the tool class used, and list the keys affected.
- Do not present a machine draft as reviewed. A translation counts as reviewed only when a fluent human reviewer, ideally with scientific or statistical background, has read it in context and said so in the PR thread. Reviewers are community contributors; maintainers merge but are not expected to read every language.
- Statistical terminology (test names, effect sizes, "confidence interval", fit terms) must follow the standard usage of the target language's scientific literature, not a literal word-for-word rendering.
- Translations must be written from CurveBook's own English strings and public terminology. Do not copy interface text from other statistical or graphing applications, and keep competitor product names out of strings (`cargo xtask brands` and `cargo xtask cleanroom` check the repository).
- Unreviewed drafts may land when clearly disclosed; do not describe them as linguistically verified. Record pending locale review in [localization-parity.md](localization-parity.md), with the affected keys or a link to the PR, so the review state survives the PR discussion. Do not weaken the coverage checks to hide missing work.

## 10. Before you open a PR

For catalog or UI text changes:

```sh
cargo test -p curvebook-ui-egui
cargo xtask i18n-report
cargo xtask brands
cargo xtask cleanroom
cargo xtask ui-commands --check   # when UI commands changed
cargo xtask commands --check      # when engine commands changed
```

Select the additional checks affected by the change using the verification policy in [../AGENTS.md](../AGENTS.md); documentation-only edits do not require a full workspace rebuild. If you added a command, also regenerate derived files per [extending.md](extending.md).
