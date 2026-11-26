//! Native desktop `egui` system font discovery, `CURVEBOOK_FONTS_DIR` overrides, and
//! deterministic cleanroom SFNT symbol fallback (`R15.1`).
//!
//! Zero font binaries are bundled in the repository. On native targets this module walks
//! `CURVEBOOK_FONTS_DIR` and host OS font directories through `curvebook-platform`'s bounded
//! regular-file readers (no `memmap`, `#![forbid(unsafe_code)]`), indexes faces into
//! `fontdb::Database`, and appends a deterministic SFNT symbol fallback built by
//! [`curvebook_text::metrics::build_symbol_fallback_truetype_font`] so required UI and
//! scientific glyphs (`→ ▾ ● ✓ ± ≤ ≥ μ χ²`) always resolve cleanly even when host or
//! primary proportional faces lack them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use curvebook_platform::fs::{read_regular_file_bytes_if_exists, read_regular_file_utf8};
use curvebook_text::BUNDLED_FONTS;
use curvebook_text::metrics::{build_minimal_truetype_font, build_symbol_fallback_truetype_font};

/// Required UI and scientific symbols guaranteed to have non-missing glyph coverage in `egui`
/// (`R15.1` font & symbol gate).
pub const MANDATORY_UI_AND_SCIENTIFIC_SYMBOLS: &[char] = &['→', '▾', '●', '✓', '±', '≤', '≥', 'μ', 'χ', '²'];

/// Key under which CurveBook's deterministic SFNT symbol fallback font is registered in
/// [`egui::FontDefinitions::font_data`].
pub const SYMBOL_FALLBACK_FONT_NAME: &str = "curvebook_symbol_fallback";

/// Key under which CurveBook's cleanroom minimal sans SFNT fallback font is registered in
/// [`egui::FontDefinitions::font_data`].
pub const CLEANROOM_SANS_FALLBACK_FONT_NAME: &str = "curvebook_cleanroom_sans";

/// Named [`egui::FontFamily::Name`] family holding the bold system UI face (`>= Semibold`)
/// followed by the proportional fallback chain (§10.2).
pub const BOLD_FONT_FAMILY: &str = "curvebook_bold";

/// Named [`egui::FontFamily::Name`] family for `13 pt` aligned grid and result-table numerals
/// selected via runtime ASCII `'0'..='9'` advance-width verification (`R20.2`).
pub const ALIGNED_NUMERAL_FONT_FAMILY: &str = "curvebook_numeral";

const BOLD_AVAILABLE_DATA_KEY: &str = "curvebook_bold_face_loaded";
const NUMERAL_FAMILY_LOADED_DATA_KEY: &str = "curvebook_numeral_family_loaded";

/// Strategy chosen at runtime for [`ALIGNED_NUMERAL_FONT_FAMILY`] (`docs/design-language.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumeralFontStrategy {
    /// Primary proportional system sans face has uniform ASCII `'0'..='9'` advance widths.
    #[default]
    PrimarySans,
    /// Secondary regular system sans face with uniform ASCII `'0'..='9'` advance widths.
    UniformSystemSans,
    /// Cap-height-matched system monospace fallback when no proportional face has uniform digits.
    MonospaceFallback,
}

const MAX_FONT_FILE_BYTES: usize = 48 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_ENV_FONT_FILES: usize = 48;
const MAX_OS_FONT_FILES: usize = 36;
const MAX_DIR_DEPTH: usize = 3;

type SharedFontBlob = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Options controlling how [`build_egui_font_definitions_with_config`] discovers system and
/// override fonts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontDiscoveryConfig {
    /// Optional explicit fonts directory (defaults to `CURVEBOOK_FONTS_DIR` when unset via `Default`).
    pub fonts_dir: Option<PathBuf>,
    /// Whether to scan curated host OS font candidates and system font directories.
    pub include_os_fonts: bool,
    /// Whether to include compile-time `BUNDLED_FONTS` when built with the `bundled-fonts` feature.
    pub include_bundled_fonts: bool,
}

impl Default for FontDiscoveryConfig {
    fn default() -> Self {
        Self {
            fonts_dir: std::env::var_os("CURVEBOOK_FONTS_DIR").filter(|v| !v.is_empty()).map(PathBuf::from),
            include_os_fonts: true,
            include_bundled_fonts: true,
        }
    }
}

/// Summary of discovered font sources and resolved `egui` fallback chains.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontDiscoveryReport {
    /// Number of faces loaded from `CURVEBOOK_FONTS_DIR`.
    pub env_override_faces: usize,
    /// Number of faces loaded from optional `BUNDLED_FONTS`.
    pub bundled_faces: usize,
    /// Number of faces loaded from host OS system font paths and directories.
    pub os_faces: usize,
    /// Ordered font keys placed into `egui::FontFamily::Proportional`.
    pub proportional_chain: Vec<String>,
    /// Ordered font keys placed into `egui::FontFamily::Monospace`.
    pub monospace_chain: Vec<String>,
    /// Whether [`SYMBOL_FALLBACK_FONT_NAME`] was installed in `FontDefinitions`.
    pub symbol_fallback_loaded: bool,
    /// Whether [`CLEANROOM_SANS_FALLBACK_FONT_NAME`] was installed in `FontDefinitions`.
    pub cleanroom_sans_loaded: bool,
    /// Whether a distinct bold (`>= Semibold`) face of the primary UI family was registered in [`BOLD_FONT_FAMILY`].
    pub bold_face_loaded: bool,
    /// Whether the resolved primary proportional sans face has uniform ASCII `'0'..='9'` advance widths.
    pub primary_digits_uniform: bool,
    /// Runtime alignment strategy selected for [`ALIGNED_NUMERAL_FONT_FAMILY`].
    pub numeral_font_strategy: NumeralFontStrategy,
}

/// Constructed [`egui::FontDefinitions`] paired with its [`FontDiscoveryReport`].
#[derive(Debug, Clone)]
pub struct BuiltEguiFonts {
    /// Complete font registry and family fallback chains for `egui`.
    pub definitions: egui::FontDefinitions,
    /// Diagnostic discovery counts and active fallback chain keys.
    pub report: FontDiscoveryReport,
}

/// Build [`egui::FontDefinitions`] using the default runtime configuration (`CURVEBOOK_FONTS_DIR`,
/// optional `bundled-fonts`, host OS fonts, cleanroom sans, and deterministic symbol fallback).
#[must_use]
pub fn build_egui_font_definitions() -> BuiltEguiFonts {
    build_egui_font_definitions_with_config(&FontDiscoveryConfig::default())
}

/// Discover system/override fonts and install them into `ctx`, returning a [`FontDiscoveryReport`].
pub fn install_egui_fonts(ctx: &egui::Context) -> FontDiscoveryReport {
    let built = build_egui_font_definitions();
    install_font_definitions(ctx, built.definitions);
    built.report
}

/// Install an already-built [`egui::FontDefinitions`] into `ctx`, recording whether [`BOLD_FONT_FAMILY`]
/// is populated so [`has_bold_font`] and [`section_heading_font_id`] match §10.2 reliably.
pub fn install_font_definitions(ctx: &egui::Context, definitions: egui::FontDefinitions) {
    let has_bold = definitions.families.get(&egui::FontFamily::Name(Arc::from(BOLD_FONT_FAMILY))).is_some_and(|chain| !chain.is_empty());
    let has_numeral =
        definitions.families.get(&egui::FontFamily::Name(Arc::from(ALIGNED_NUMERAL_FONT_FAMILY))).is_some_and(|chain| !chain.is_empty());
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique(BOLD_AVAILABLE_DATA_KEY), has_bold);
        d.insert_temp(egui::Id::unique(NUMERAL_FAMILY_LOADED_DATA_KEY), has_numeral);
    });
    ctx.set_fonts(definitions);
}

/// Returns `true` when the active font setup registered a distinct bold UI face (`>= Semibold`).
#[must_use]
pub fn has_bold_font(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(BOLD_AVAILABLE_DATA_KEY))).unwrap_or(false)
}

/// Returns `true` when [`ALIGNED_NUMERAL_FONT_FAMILY`] is installed on `ctx`.
#[must_use]
pub fn has_aligned_numeral_font(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(NUMERAL_FAMILY_LOADED_DATA_KEY))).unwrap_or(false)
}

/// Section heading font per §10.2: `type.section` (`13 pt` bold) or `14 pt` regular fallback when no bold face was loaded.
#[must_use]
pub fn section_heading_font_id(ctx: &egui::Context, tokens: &super::Tokens) -> egui::FontId {
    if has_bold_font(ctx) {
        egui::FontId::new(tokens.metrics.font_ui_section_pt, egui::FontFamily::Name(Arc::from(BOLD_FONT_FAMILY)))
    } else {
        egui::FontId::proportional(14.0)
    }
}

/// Aligned `13 pt` numeral font (`type.numeral`) backed by the runtime `'0'..='9'` advance-width
/// check (`docs/design-language.md` §3).
#[must_use]
pub fn aligned_numeral_font_id(ctx: &egui::Context, tokens: &super::Tokens) -> egui::FontId {
    if has_aligned_numeral_font(ctx) {
        egui::FontId::new(tokens.metrics.font_grid_numeral_pt, egui::FontFamily::Name(Arc::from(ALIGNED_NUMERAL_FONT_FAMILY)))
    } else {
        egui::FontId::monospace(tokens.metrics.font_grid_numeral_pt)
    }
}

/// Measure ASCII `'0'..='9'` advance widths on an SFNT or `.ttc` sub-face and return `true` when
/// all ten digits share uniform advance width (within `0.005 em`).
#[must_use]
pub fn face_has_uniform_ascii_digits(bytes: &[u8], face_index: u32) -> bool {
    let faces = curvebook_text::TrueTypeFont::parse_collection(bytes);
    let Some((_, ttf)) = faces.get(face_index as usize).or_else(|| faces.first()) else {
        return false;
    };
    let mut min_adv = f64::INFINITY;
    let mut max_adv = f64::NEG_INFINITY;
    for digit in '0'..='9' {
        let Some(adv) = ttf.advance_em(digit) else {
            return false;
        };
        if !adv.is_finite() || adv <= 0.0 {
            return false;
        }
        min_adv = min_adv.min(adv);
        max_adv = max_adv.max(adv);
    }
    (max_adv - min_adv) <= 0.005
}

/// Install [`egui::FontDefinitions`] into `ctx`, reusing a process-wide cached default build when
/// `CURVEBOOK_FONTS_DIR` is unset so offscreen inspection/raster passes do not re-index host fonts.
pub fn install_default_egui_fonts_cached(ctx: &egui::Context) {
    static DEFAULT_FONT_DEFS: OnceLock<(egui::FontDefinitions, bool, bool)> = OnceLock::new();
    let has_env_override = std::env::var_os("CURVEBOOK_FONTS_DIR").is_some_and(|v| !v.is_empty());
    if has_env_override {
        let _ = install_egui_fonts(ctx);
        return;
    }
    let (defs, has_bold, has_numeral) = DEFAULT_FONT_DEFS.get_or_init(|| {
        let built = build_egui_font_definitions();
        let has_num = built.definitions.families.get(&egui::FontFamily::Name(Arc::from(ALIGNED_NUMERAL_FONT_FAMILY))).is_some_and(|c| !c.is_empty());
        (built.definitions, built.report.bold_face_loaded, has_num)
    });
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique(BOLD_AVAILABLE_DATA_KEY), *has_bold);
        d.insert_temp(egui::Id::unique(NUMERAL_FAMILY_LOADED_DATA_KEY), *has_numeral);
    });
    ctx.set_fonts(defs.clone());
}

/// Build [`egui::FontDefinitions`] from an explicit [`FontDiscoveryConfig`].
#[must_use]
pub fn build_egui_font_definitions_with_config(config: &FontDiscoveryConfig) -> BuiltEguiFonts {
    let mut db = fontdb::Database::new();
    let mut env_ids = Vec::new();
    let mut bundled_ids = Vec::new();
    let mut os_ids = Vec::new();

    if let Some(dir) = config.fonts_dir.as_deref() {
        load_dir_into_db(dir, &mut db, &mut env_ids);
    }
    if config.include_bundled_fonts {
        load_bundled_into_db(&mut db, &mut bundled_ids);
    }
    if config.include_os_fonts {
        for blob in cached_host_font_blobs() {
            let ids = db.load_font_source(fontdb::Source::Binary(Arc::clone(blob)));
            os_ids.extend(ids);
        }
    }

    let report = FontDiscoveryReport {
        env_override_faces: env_ids.len(),
        bundled_faces: bundled_ids.len(),
        os_faces: os_ids.len(),
        ..FontDiscoveryReport::default()
    };

    assemble_font_definitions(&db, &env_ids, &bundled_ids, &os_ids, report)
}

/// Append CurveBook's deterministic cleanroom SFNT symbol fallback (`→ ▾ ● ✓ ± ≤ ≥ μ χ²`) to the
/// end of both `Proportional` and `Monospace` chains in `definitions`.
///
/// Useful both for default font assembly and for augmenting custom/primary-only `FontDefinitions`
/// whose primary proportional face lacks UI or scientific symbols such as `▾` (`U+25BE`).
pub fn append_symbol_fallback(definitions: &mut egui::FontDefinitions) {
    let key = String::from(SYMBOL_FALLBACK_FONT_NAME);
    definitions.font_data.entry(key.clone()).or_insert_with(|| Arc::new(egui::FontData::from_owned(build_symbol_fallback_truetype_font())));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let chain = definitions.families.entry(family).or_default();
        if !chain.iter().any(|existing| existing == &key) {
            chain.push(key.clone());
        }
    }
}

fn append_cleanroom_sans_fallback(definitions: &mut egui::FontDefinitions) {
    let key = String::from(CLEANROOM_SANS_FALLBACK_FONT_NAME);
    definitions.font_data.entry(key.clone()).or_insert_with(|| Arc::new(egui::FontData::from_owned(build_minimal_truetype_font())));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let chain = definitions.families.entry(family).or_default();
        if !chain.iter().any(|existing| existing == &key) {
            chain.push(key.clone());
        }
    }
}

fn assemble_font_definitions(
    db: &fontdb::Database,
    env_ids: &[fontdb::ID],
    bundled_ids: &[fontdb::ID],
    os_ids: &[fontdb::ID],
    mut report: FontDiscoveryReport,
) -> BuiltEguiFonts {
    let mut defs = egui::FontDefinitions::empty();
    let mut id_keys: HashMap<fontdb::ID, String> = HashMap::new();
    let mut proportional = Vec::new();
    let mut monospace = Vec::new();
    let mut prop_seen = HashSet::new();
    let mut mono_seen = HashSet::new();
    let mut rctx = FontRegisterCtx { db, defs: &mut defs, id_keys: &mut id_keys };

    // Tier 1: explicit CURVEBOOK_FONTS_DIR overrides first (regular normal faces prioritized).
    push_sorted_tier_faces(&mut rctx, env_ids, false, &mut prop_seen, &mut proportional);
    push_sorted_tier_faces(&mut rctx, env_ids, true, &mut mono_seen, &mut monospace);

    // Tier 2: compile-time optional bundled faces (empty unless external staging was compiled).
    push_sorted_tier_faces(&mut rctx, bundled_ids, false, &mut prop_seen, &mut proportional);
    push_sorted_tier_faces(&mut rctx, bundled_ids, true, &mut mono_seen, &mut monospace);

    // Tier 3: host OS faces selected via fontdb queries (zero memmap, zero fontconfig).
    let os_prop_id = query_primary_proportional(rctx.db).or_else(|| first_regular_id(rctx.db, os_ids, false));
    if let Some(id) = os_prop_id {
        register_face_for_family(&mut rctx, id, "os_proportional", &mut prop_seen, &mut proportional);
    }

    let os_mono_id = query_primary_monospace(rctx.db).or_else(|| first_regular_id(rctx.db, os_ids, true));
    if let Some(id) = os_mono_id {
        register_face_for_family(&mut rctx, id, "os_monospace", &mut mono_seen, &mut monospace);
    }

    for (idx, family_name) in OS_SYMBOL_FAMILIES.iter().enumerate() {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family_name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        if let Some(id) = rctx.db.query(&query) {
            let prefix = format!("os_symbol_{idx}");
            register_face_for_family(&mut rctx, id, &prefix, &mut prop_seen, &mut proportional);
            register_face_for_family(&mut rctx, id, &prefix, &mut mono_seen, &mut monospace);
        }
    }

    for (idx, family_name) in OS_CJK_FAMILIES.iter().enumerate() {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family_name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        if let Some(id) = rctx.db.query(&query) {
            let prefix = format!("os_cjk_{idx}");
            register_face_for_family(&mut rctx, id, &prefix, &mut prop_seen, &mut proportional);
            register_face_for_family(&mut rctx, id, &prefix, &mut mono_seen, &mut monospace);
        }
    }

    for (idx, family_name) in OS_RTL_FAMILIES.iter().enumerate() {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family_name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        if let Some(id) = rctx.db.query(&query) {
            let prefix = format!("os_rtl_{idx}");
            register_face_for_family(&mut rctx, id, &prefix, &mut prop_seen, &mut proportional);
            register_face_for_family(&mut rctx, id, &prefix, &mut mono_seen, &mut monospace);
        }
    }

    // Add a bounded tail of other discovered regular OS faces in case a non-standard distro
    // provides an arbitrary family name outside the curated lists above.
    let mut extra_count = 0usize;
    for &id in os_ids {
        if extra_count >= 6 {
            break;
        }
        if is_regular_face(rctx.db, id) && register_face_for_family(&mut rctx, id, "os_extra", &mut prop_seen, &mut proportional) {
            extra_count += 1;
        }
    }

    // Ensure monospace can still fall back through primary proportional & CJK/RTL/symbol OS faces.
    for key in &proportional {
        if !monospace.iter().any(|existing| existing == key) {
            monospace.push(key.clone());
        }
    }

    let primary_prop_id = first_regular_nonmono_id(rctx.db, env_ids).or_else(|| first_regular_nonmono_id(rctx.db, bundled_ids)).or(os_prop_id);
    let bold_id = primary_prop_id.and_then(|reg_id| query_primary_proportional_bold(rctx.db, reg_id));

    rctx.defs.families.insert(egui::FontFamily::Proportional, proportional);
    rctx.defs.families.insert(egui::FontFamily::Monospace, monospace);

    // Deterministic cleanroom SFNT layers: sans first, mandatory symbols right after.
    append_cleanroom_sans_fallback(rctx.defs);
    append_symbol_fallback(rctx.defs);

    let (numeral_chain, primary_digits_uniform, numeral_strategy) =
        build_aligned_numeral_chain(&mut rctx, primary_prop_id, env_ids, bundled_ids, os_ids);
    rctx.defs.families.insert(egui::FontFamily::Name(Arc::from(ALIGNED_NUMERAL_FONT_FAMILY)), numeral_chain);
    report.primary_digits_uniform = primary_digits_uniform;
    report.numeral_font_strategy = numeral_strategy;

    if let Some(bid) = bold_id {
        let mut bold_chain = Vec::new();
        let mut bold_seen = HashSet::new();
        if register_face_for_family(&mut rctx, bid, "os_proportional_bold", &mut bold_seen, &mut bold_chain) {
            if let Some(prop_chain) = rctx.defs.families.get(&egui::FontFamily::Proportional) {
                for key in prop_chain {
                    if !bold_chain.iter().any(|existing| existing == key) {
                        bold_chain.push(key.clone());
                    }
                }
            }
            rctx.defs.families.insert(egui::FontFamily::Name(Arc::from(BOLD_FONT_FAMILY)), bold_chain);
            report.bold_face_loaded = true;
        }
    }

    report.proportional_chain = defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    report.monospace_chain = defs.families.get(&egui::FontFamily::Monospace).cloned().unwrap_or_default();
    report.cleanroom_sans_loaded = defs.font_data.contains_key(CLEANROOM_SANS_FALLBACK_FONT_NAME);
    report.symbol_fallback_loaded = defs.font_data.contains_key(SYMBOL_FALLBACK_FONT_NAME);

    BuiltEguiFonts { definitions: defs, report }
}

struct FontRegisterCtx<'a> {
    db: &'a fontdb::Database,
    defs: &'a mut egui::FontDefinitions,
    id_keys: &'a mut HashMap<fontdb::ID, String>,
}

fn push_sorted_tier_faces(
    rctx: &mut FontRegisterCtx<'_>,
    ids: &[fontdb::ID],
    prefer_mono: bool,
    seen: &mut HashSet<fontdb::ID>,
    chain: &mut Vec<String>,
) {
    let mut ordered: Vec<(u8, fontdb::ID)> = ids
        .iter()
        .copied()
        .map(|id| {
            let Some(face) = rctx.db.face(id) else {
                return (9, id);
            };
            let reg_rank = u8::from(!(face.style == fontdb::Style::Normal && face.weight == fontdb::Weight::NORMAL));
            let mono_rank = if prefer_mono { u8::from(!face.monospaced) * 2 } else { u8::from(face.monospaced) * 2 };
            (reg_rank + mono_rank, id)
        })
        .collect();
    ordered.sort_by_key(|&(rank, _)| rank);
    let prefix = if prefer_mono { "tier_mono" } else { "tier_prop" };
    for (_, id) in ordered {
        register_face_for_family(rctx, id, prefix, seen, chain);
    }
}

fn register_face_for_family(
    rctx: &mut FontRegisterCtx<'_>,
    id: fontdb::ID,
    prefix: &str,
    seen: &mut HashSet<fontdb::ID>,
    chain: &mut Vec<String>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if let Some(existing_key) = rctx.id_keys.get(&id).cloned() {
        if !chain.iter().any(|existing| existing == &existing_key) {
            chain.push(existing_key);
        }
        return true;
    }
    let Some((fontdb::Source::Binary(blob), face_index)) = rctx.db.face_source(id) else {
        return false;
    };
    let fam_slug = id_to_slug(rctx.db, id);
    let ps_slug = id_to_ps_slug(rctx.db, id);
    let base = if ps_slug.is_empty() || ps_slug == fam_slug {
        format!("{prefix}_{fam_slug}_{face_index}")
    } else {
        format!("{prefix}_{fam_slug}_{ps_slug}_{face_index}")
    };
    let mut key = base.clone();
    let mut suffix = 2usize;
    while rctx.defs.font_data.contains_key(&key) {
        key = format!("{base}_{suffix}");
        suffix += 1;
    }
    rctx.defs.font_data.insert(key.clone(), Arc::new(egui::FontData::from_blob(blob, face_index)));
    rctx.id_keys.insert(id, key.clone());
    if !chain.iter().any(|existing| existing == &key) {
        chain.push(key);
    }
    true
}

fn id_to_ps_slug(db: &fontdb::Database, id: fontdb::ID) -> String {
    let raw = db.face(id).map_or("", |f| f.post_script_name.as_str());
    raw.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' }).collect()
}

fn id_to_slug(db: &fontdb::Database, id: fontdb::ID) -> String {
    let raw = db.face(id).and_then(|f| f.families.first().map(|(name, _)| name.as_str()).or(Some(f.post_script_name.as_str()))).unwrap_or("face");
    let slug: String = raw.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' }).collect();
    if slug.is_empty() { String::from("face") } else { slug }
}

fn is_regular_face(db: &fontdb::Database, id: fontdb::ID) -> bool {
    db.face(id).is_some_and(|face| face.style == fontdb::Style::Normal && face.weight == fontdb::Weight::NORMAL)
}

fn first_regular_id(db: &fontdb::Database, ids: &[fontdb::ID], require_mono: bool) -> Option<fontdb::ID> {
    ids.iter()
        .copied()
        .find(|&id| {
            db.face(id).is_some_and(|f| f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL && (!require_mono || f.monospaced))
        })
        .or_else(|| ids.iter().copied().find(|&id| !require_mono || db.face(id).is_some_and(|f| f.monospaced)))
}

fn first_regular_nonmono_id(db: &fontdb::Database, ids: &[fontdb::ID]) -> Option<fontdb::ID> {
    ids.iter()
        .copied()
        .find(|&id| db.face(id).is_some_and(|f| f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL && !f.monospaced))
}

fn db_face_has_uniform_ascii_digits(db: &fontdb::Database, id: fontdb::ID) -> bool {
    let Some((fontdb::Source::Binary(blob), face_index)) = db.face_source(id) else {
        return false;
    };
    face_has_uniform_ascii_digits((*blob).as_ref(), face_index)
}

fn find_uniform_digit_sans_id(
    db: &fontdb::Database,
    exclude_id: Option<fontdb::ID>,
    env_ids: &[fontdb::ID],
    bundled_ids: &[fontdb::ID],
    os_ids: &[fontdb::ID],
) -> Option<fontdb::ID> {
    for slice in [env_ids, bundled_ids, os_ids] {
        for &id in slice {
            if Some(id) == exclude_id {
                continue;
            }
            let is_reg_sans = db.face(id).is_some_and(|f| f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL && !f.monospaced);
            if is_reg_sans && db_face_has_uniform_ascii_digits(db, id) {
                return Some(id);
            }
        }
    }
    None
}

fn build_aligned_numeral_chain(
    rctx: &mut FontRegisterCtx<'_>,
    primary_prop_id: Option<fontdb::ID>,
    env_ids: &[fontdb::ID],
    bundled_ids: &[fontdb::ID],
    os_ids: &[fontdb::ID],
) -> (Vec<String>, bool, NumeralFontStrategy) {
    let primary_uniform = primary_prop_id.is_some_and(|id| db_face_has_uniform_ascii_digits(rctx.db, id));
    if primary_uniform {
        let chain = rctx.defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        return (chain, true, NumeralFontStrategy::PrimarySans);
    }
    if let Some(alt_id) = find_uniform_digit_sans_id(rctx.db, primary_prop_id, env_ids, bundled_ids, os_ids) {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        if register_face_for_family(rctx, alt_id, "os_numeral_sans", &mut seen, &mut chain) {
            if let Some(prop_chain) = rctx.defs.families.get(&egui::FontFamily::Proportional) {
                for key in prop_chain {
                    if !chain.iter().any(|existing| existing == key) {
                        chain.push(key.clone());
                    }
                }
            }
            return (chain, false, NumeralFontStrategy::UniformSystemSans);
        }
    }
    let mono_chain = rctx.defs.families.get(&egui::FontFamily::Monospace).cloned().unwrap_or_default();
    (mono_chain, false, NumeralFontStrategy::MonospaceFallback)
}

fn query_primary_proportional(db: &fontdb::Database) -> Option<fontdb::ID> {
    db.query(&fontdb::Query {
        families: &[
            fontdb::Family::Name("SF Pro Text"),
            fontdb::Family::Name("SF NS Text"),
            fontdb::Family::Name("Helvetica Neue"),
            fontdb::Family::Name(".SF NS"),
            fontdb::Family::Name("Segoe UI"),
            fontdb::Family::Name("DejaVu Sans"),
            fontdb::Family::Name("Liberation Sans"),
            fontdb::Family::Name("Noto Sans"),
            fontdb::Family::Name("Arial"),
            fontdb::Family::Name("Helvetica"),
            fontdb::Family::SansSerif,
        ],
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    })
}

fn query_primary_proportional_bold(db: &fontdb::Database, regular_id: fontdb::ID) -> Option<fontdb::ID> {
    let regular_face = db.face(regular_id)?;
    let family_filters: Vec<fontdb::Family<'_>> = regular_face.families.iter().map(|(name, _)| fontdb::Family::Name(name.as_str())).collect();
    if family_filters.is_empty() {
        return None;
    }
    let bold_id = db.query(&fontdb::Query {
        families: &family_filters,
        weight: fontdb::Weight::BOLD,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    })?;
    let is_distinct_bold =
        bold_id != regular_id && db.face(bold_id).is_some_and(|f| f.style == fontdb::Style::Normal && f.weight.0 >= fontdb::Weight::SEMIBOLD.0);
    is_distinct_bold.then_some(bold_id)
}

fn query_primary_monospace(db: &fontdb::Database) -> Option<fontdb::ID> {
    db.query(&fontdb::Query {
        families: &[
            fontdb::Family::Name("SF Mono"),
            fontdb::Family::Name("Menlo"),
            fontdb::Family::Name("Consolas"),
            fontdb::Family::Name("DejaVu Sans Mono"),
            fontdb::Family::Name("Liberation Mono"),
            fontdb::Family::Name("Noto Sans Mono"),
            fontdb::Family::Name("Courier New"),
            fontdb::Family::Name("Monaco"),
            fontdb::Family::Monospace,
        ],
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    })
}

const OS_SYMBOL_FAMILIES: &[&str] =
    &["Apple Symbols", "Segoe UI Symbol", "DejaVu Sans", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math", "Arial Unicode MS"];

const OS_CJK_FAMILIES: &[&str] = &[
    "Hiragino Sans",
    "Hiragino Sans GB",
    "PingFang SC",
    "PingFang TC",
    "Apple SD Gothic Neo",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Noto Sans CJK KR",
    "Microsoft YaHei",
    "MS Gothic",
    "Malgun Gothic",
    "Arial Unicode MS",
    "AppleGothic",
];

const OS_RTL_FAMILIES: &[&str] = &[
    "Geeza Pro",
    "Damascus",
    "Arial Hebrew",
    "SF Arabic",
    "Al Bayan",
    "Noto Sans Arabic",
    "Noto Naskh Arabic",
    "Noto Sans Hebrew",
    "Tahoma",
    "Segoe UI",
    "DejaVu Sans",
    "Arial Unicode MS",
];

fn load_bundled_into_db(db: &mut fontdb::Database, out_ids: &mut Vec<fontdb::ID>) {
    if !cfg!(feature = "bundled-fonts") {
        return;
    }
    for &(name, bytes) in BUNDLED_FONTS {
        if !name.is_empty() && !bytes.is_empty() {
            let blob: SharedFontBlob = Arc::new(bytes);
            out_ids.extend(db.load_font_source(fontdb::Source::Binary(blob)));
        }
    }
}

fn load_dir_into_db(dir: &Path, db: &mut fontdb::Database, out_ids: &mut Vec<fontdb::ID>) {
    let mut loaded_paths = HashSet::new();
    let manifest_specs = [(dir.join("fonts").join("manifest.txt"), dir.join("fonts")), (dir.join("manifest.txt"), dir.to_path_buf())];
    for (manifest_path, base_dir) in manifest_specs {
        let Ok(canonical_base) = std::fs::canonicalize(&base_dir) else {
            continue;
        };
        if let Ok(text) = read_regular_file_utf8(&manifest_path, MAX_MANIFEST_BYTES) {
            for line in text.lines() {
                let rel = line.split('#').next().unwrap_or_default().trim();
                if rel.is_empty() {
                    continue;
                }
                let rel_path = Path::new(rel);
                if rel_path.is_absolute() || !rel_path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
                    continue;
                }
                let candidate = base_dir.join(rel_path);
                let Ok(canonical_candidate) = std::fs::canonicalize(&candidate) else {
                    continue;
                };
                if !canonical_candidate.starts_with(&canonical_base) {
                    continue;
                }
                if loaded_paths.len() >= MAX_ENV_FONT_FILES {
                    break;
                }
                if is_sfnt_extension(&canonical_candidate)
                    && !loaded_paths.contains(&canonical_candidate)
                    && let Some(blob) = read_font_blob(&canonical_candidate)
                {
                    loaded_paths.insert(canonical_candidate);
                    out_ids.extend(db.load_font_source(fontdb::Source::Binary(blob)));
                }
            }
        }
    }

    if let Ok(canonical_dir) = std::fs::canonicalize(dir) {
        let mut discovered = Vec::new();
        collect_sfnt_files_bounded(dir, 0, MAX_ENV_FONT_FILES, &mut discovered);
        discovered.sort();
        for path in discovered {
            if loaded_paths.len() >= MAX_ENV_FONT_FILES {
                break;
            }
            let Ok(dedup_key) = std::fs::canonicalize(&path) else {
                continue;
            };
            if !dedup_key.starts_with(&canonical_dir) {
                continue;
            }
            if !loaded_paths.contains(&dedup_key)
                && let Some(blob) = read_font_blob(&dedup_key)
            {
                loaded_paths.insert(dedup_key);
                out_ids.extend(db.load_font_source(fontdb::Source::Binary(blob)));
            }
        }
    }
}

fn cached_host_font_blobs() -> &'static [SharedFontBlob] {
    static HOST_BLOBS: OnceLock<Vec<SharedFontBlob>> = OnceLock::new();
    HOST_BLOBS.get_or_init(discover_host_font_blobs)
}

fn discover_host_font_blobs() -> Vec<SharedFontBlob> {
    let mut seen_paths = HashSet::new();
    let mut blobs = Vec::new();

    for candidate in curated_os_font_candidates() {
        if blobs.len() >= MAX_OS_FONT_FILES {
            break;
        }
        let path = Path::new(candidate);
        if !seen_paths.contains(path)
            && let Some(blob) = read_font_blob(path)
        {
            seen_paths.insert(path.to_path_buf());
            blobs.push(blob);
        }
    }

    // Also traverse standard OS font directories cleanly (bounded, zero memmap) so fonts
    // installed in distro-specific paths or user font directories are always discoverable.
    let mut dir_candidates = Vec::new();
    for dir in os_font_directories() {
        if dir_candidates.len() >= MAX_OS_FONT_FILES * 2 {
            break;
        }
        collect_sfnt_files_bounded(&dir, 0, MAX_OS_FONT_FILES * 2, &mut dir_candidates);
    }
    dir_candidates.sort();
    for path in dir_candidates {
        if blobs.len() >= MAX_OS_FONT_FILES {
            break;
        }
        if is_skipped_huge_bitmap_font(&path) || seen_paths.contains(&path) {
            continue;
        }
        if let Some(blob) = read_font_blob(&path) {
            seen_paths.insert(path);
            blobs.push(blob);
        }
    }

    blobs
}

fn os_font_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(home).join("Library/Fonts"));
        }
        dirs.push(PathBuf::from("/Library/Fonts"));
        dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
        dirs.push(PathBuf::from("/System/Library/Fonts"));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
        if let Some(win) = std::env::var_os("WINDIR").filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(win).join("Fonts"));
        } else {
            dirs.push(PathBuf::from(r"C:\Windows\Fonts"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(data_home).join("fonts"));
        } else if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            let home_dir = PathBuf::from(home);
            dirs.push(home_dir.join(".local/share/fonts"));
            dirs.push(home_dir.join(".fonts"));
        }
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        dirs.push(PathBuf::from("/usr/share/fonts"));
    }
    dirs
}

fn curated_os_font_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "/System/Library/Fonts/HelveticaNeue.ttc",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/System/Library/Fonts/GeezaPro.ttc",
            "/System/Library/Fonts/Supplemental/Damascus.ttc",
            "/System/Library/Fonts/Supplemental/ArialHB.ttc",
            "/System/Library/Fonts/SFArabic.ttf",
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
            r"C:\Windows\Fonts\consola.ttf",
            r"C:\Windows\Fonts\cour.ttf",
            r"C:\Windows\Fonts\seguisym.ttf",
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
            r"C:\Windows\Fonts\malgun.ttf",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ]
    }
}

fn is_skipped_huge_bitmap_font(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.contains("emoji") || lower.contains("lastresort")
}

fn read_font_blob(path: &Path) -> Option<SharedFontBlob> {
    let bytes = read_regular_file_bytes_if_exists(path, MAX_FONT_FILE_BYTES).ok()??;
    if bytes.is_empty() {
        return None;
    }
    let blob: SharedFontBlob = Arc::new(bytes);
    Some(blob)
}

fn collect_sfnt_files_bounded(dir: &Path, depth: usize, cap: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DIR_DEPTH || out.len() >= cap {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            subdirs.push(path);
        } else if is_sfnt_extension(&path) {
            files.push(path);
        }
    }
    files.sort();
    for path in files {
        if out.len() >= cap {
            return;
        }
        out.push(path);
    }
    subdirs.sort();
    for sub in subdirs {
        if out.len() >= cap {
            return;
        }
        collect_sfnt_files_bounded(&sub, depth + 1, cap, out);
    }
}

fn is_sfnt_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf") || ext.eq_ignore_ascii_case("ttc"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use curvebook_text::metrics::build_minimal_truetype_font;

    use super::{
        CLEANROOM_SANS_FALLBACK_FONT_NAME, FontDiscoveryConfig, MANDATORY_UI_AND_SCIENTIFIC_SYMBOLS, SYMBOL_FALLBACK_FONT_NAME,
        append_symbol_fallback, build_egui_font_definitions, build_egui_font_definitions_with_config, install_egui_fonts,
    };

    fn temp_fonts_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("r15-fonts-test-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp fonts dir");
        dir
    }

    #[test]
    fn runtime_font_discovery_covers_all_mandatory_symbols_in_proportional_and_mono() {
        let ctx = egui::Context::default();
        let report = install_egui_fonts(&ctx);
        assert!(report.symbol_fallback_loaded);
        assert!(report.cleanroom_sans_loaded);
        assert!(!report.proportional_chain.is_empty());
        assert!(!report.monospace_chain.is_empty());

        let mandatory_str: String = MANDATORY_UI_AND_SCIENTIFIC_SYMBOLS.iter().collect();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let font_id = egui::FontId::new(13.0, family);
                    for &ch in MANDATORY_UI_AND_SCIENTIFIC_SYMBOLS {
                        assert!(
                            fonts.has_glyph(&font_id, ch),
                            "egui font family {font_id:?} reported missing glyph for {ch:?} (U+{:04X})",
                            u32::from(ch)
                        );
                    }
                    assert!(fonts.has_glyphs(&font_id, &mandatory_str));
                }
                let galley = fonts.layout_no_wrap(mandatory_str.clone(), egui::FontId::proportional(13.0), egui::Color32::WHITE);
                assert!(galley.rect.width() > 0.0, "expected positive laid-out width for mandatory symbols");
            });
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn cleanroom_headless_without_os_fonts_covers_ascii_and_all_mandatory_symbols() {
        let built =
            build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
        assert_eq!(built.report.env_override_faces, 0);
        assert_eq!(built.report.os_faces, 0);
        assert_eq!(built.report.proportional_chain, vec![CLEANROOM_SANS_FALLBACK_FONT_NAME, SYMBOL_FALLBACK_FONT_NAME]);
        assert_eq!(built.report.monospace_chain, vec![CLEANROOM_SANS_FALLBACK_FONT_NAME, SYMBOL_FALLBACK_FONT_NAME]);

        let ctx = egui::Context::default();
        ctx.set_fonts(built.definitions);
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let id = egui::FontId::new(13.0, family);
                    assert!(fonts.has_glyphs(&id, "ABCxyz0123 → ▾ ● ✓ ± ≤ ≥ μ χ²"));
                }
            });
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn automatic_fallback_fires_when_primary_proportional_face_lacks_disclosure_triangle() {
        // 1. Confirm primary minimal sans SFNT on its own lacks '▾' (U+25BE), '→', '●', '✓', '≤', '≥', 'χ'.
        let mut primary_only = egui::FontDefinitions::empty();
        primary_only.font_data.insert(String::from("primary_sans"), Arc::new(egui::FontData::from_owned(build_minimal_truetype_font())));
        primary_only.families.insert(egui::FontFamily::Proportional, vec![String::from("primary_sans")]);

        let primary_ctx = egui::Context::default();
        primary_ctx.set_fonts(primary_only.clone());
        primary_ctx
            .run_ui(egui::RawInput::default(), |ui| {
                ui.ctx().fonts_mut(|fonts| {
                    let id = egui::FontId::proportional(13.0);
                    assert!(fonts.has_glyph(&id, 'A'));
                    assert!(!fonts.has_glyph(&id, '▾'), "fixture precondition: primary face must lack U+25BE");
                    assert!(!fonts.has_glyph(&id, '→'), "fixture precondition: primary face must lack U+2192");
                });
            })
            .drop_without_applying_deltas();

        // 2. Append CurveBook symbol fallback onto that custom primary-only definition.
        append_symbol_fallback(&mut primary_only);
        let fallback_ctx = egui::Context::default();
        fallback_ctx.set_fonts(primary_only);
        fallback_ctx
            .run_ui(egui::RawInput::default(), |ui| {
                ui.ctx().fonts_mut(|fonts| {
                    let id = egui::FontId::proportional(13.0);
                    assert!(fonts.has_glyph(&id, 'A'));
                    for &ch in MANDATORY_UI_AND_SCIENTIFIC_SYMBOLS {
                        assert!(
                            fonts.has_glyph(&id, ch),
                            "automatic fallback failed to cover {ch:?} (U+{:04X}) when primary lacked U+25BE",
                            u32::from(ch)
                        );
                    }
                });
            })
            .drop_without_applying_deltas();

        // 3. Also verify CURVEBOOK_FONTS_DIR override loading that same U+25BE-lacking face via fontdb.
        let dir = temp_fonts_dir("override-fallback");
        let fonts_sub = dir.join("fonts");
        std::fs::create_dir_all(&fonts_sub).expect("create fonts subdir");
        std::fs::write(fonts_sub.join("MinimalPrimary.ttf"), build_minimal_truetype_font()).expect("write minimal ttf");
        std::fs::write(fonts_sub.join("manifest.txt"), "# primary sans lacking U+25BE\nMinimalPrimary.ttf\n../escape.ttf\n").expect("write manifest");

        let built = build_egui_font_definitions_with_config(&FontDiscoveryConfig {
            fonts_dir: Some(dir.clone()),
            include_os_fonts: false,
            include_bundled_fonts: false,
        });
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(built.report.env_override_faces, 1);
        assert!(built.report.proportional_chain.first().is_some_and(|k| k.starts_with("tier_prop_")));
        assert_eq!(built.report.proportional_chain.last().map(String::as_str), Some(SYMBOL_FALLBACK_FONT_NAME));

        let env_ctx = egui::Context::default();
        env_ctx.set_fonts(built.definitions);
        env_ctx
            .run_ui(egui::RawInput::default(), |ui| {
                ui.ctx().fonts_mut(|fonts| {
                    let id = egui::FontId::proportional(13.0);
                    assert!(fonts.has_glyphs(&id, "IC50 → ▾ ● ✓ ± ≤ ≥ μ χ²"));
                });
            })
            .drop_without_applying_deltas();
    }

    #[test]
    fn default_build_preserves_zero_bundled_binary_invariant() {
        let built = build_egui_font_definitions();
        if !cfg!(feature = "bundled-fonts") || curvebook_text::BUNDLED_FONTS.is_empty() {
            assert_eq!(built.report.bundled_faces, 0);
        }
    }

    #[test]
    fn bold_section_heading_uses_bold_face_or_14pt_fallback() {
        let headless =
            build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
        assert!(!headless.report.bold_face_loaded);
        let headless_ctx = egui::Context::default();
        super::install_font_definitions(&headless_ctx, headless.definitions);
        let tokens = crate::egui_shell::Tokens::default();
        assert_eq!(super::section_heading_font_id(&headless_ctx, &tokens), egui::FontId::proportional(14.0));

        let host = build_egui_font_definitions();
        let host_ctx = egui::Context::default();
        let report = install_egui_fonts(&host_ctx);
        assert_eq!(report.bold_face_loaded, host.report.bold_face_loaded);
        if cfg!(target_os = "macos") {
            assert!(report.bold_face_loaded, "expected distinct bold UI face on macOS via Helvetica Neue");
            assert_eq!(
                super::section_heading_font_id(&host_ctx, &tokens),
                egui::FontId::new(13.0, egui::FontFamily::Name(std::sync::Arc::from(super::BOLD_FONT_FAMILY)))
            );
        }
    }

    #[test]
    fn runtime_digit_width_check_selects_aligned_numeral_font_family() {
        // Minimal cleanroom fallback TTF has uniform glyph advance widths across ASCII '0'..='9'.
        let minimal = build_minimal_truetype_font();
        assert!(super::face_has_uniform_ascii_digits(&minimal, 0));
        assert!(!super::face_has_uniform_ascii_digits(b"not-a-font", 0));

        let ctx = egui::Context::default();
        let report = install_egui_fonts(&ctx);
        let tokens = crate::egui_shell::Tokens::default();
        assert!(super::has_aligned_numeral_font(&ctx));
        assert_eq!(
            super::aligned_numeral_font_id(&ctx, &tokens),
            egui::FontId::new(13.0, egui::FontFamily::Name(std::sync::Arc::from(super::ALIGNED_NUMERAL_FONT_FAMILY)))
        );
        // Verify all ASCII digits lay out with uniform advance width in the resolved numeral family.
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let num_font = super::aligned_numeral_font_id(ui.ctx(), &tokens);
            let mut widths = Vec::new();
            ui.ctx().fonts_mut(|fonts| {
                for d in '0'..='9' {
                    let g = fonts.layout_no_wrap(d.to_string(), num_font.clone(), egui::Color32::WHITE);
                    widths.push(g.rect.width());
                }
            });
            let min_w = widths.iter().copied().fold(f32::INFINITY, f32::min);
            let max_w = widths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!((max_w - min_w) <= 0.5, "aligned numeral family ({:?}) produced uneven digit widths: {widths:?}", report.numeral_font_strategy);
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn manifest_font_loading_rejects_traversal_absolute_and_escaping_symlinks() {
        let root = std::env::temp_dir().join(format!("curvebook-font-manifest-sec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundle_dir = root.join("bundle");
        let fonts_dir = bundle_dir.join("fonts");
        let outside_dir = root.join("outside");
        std::fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        std::fs::create_dir_all(&outside_dir).expect("create outside dir");

        let minimal_ttf = build_minimal_truetype_font();
        let outside_font = outside_dir.join("escaped.ttf");
        std::fs::write(&outside_font, &minimal_ttf).expect("write outside font");

        let mut manifest_lines =
            vec![String::from("../../outside/escaped.ttf"), outside_font.to_string_lossy().into_owned(), String::from("./../../outside/escaped.ttf")];

        #[cfg(unix)]
        {
            let link_path = fonts_dir.join("escape_link.ttf");
            std::os::unix::fs::symlink(&outside_font, &link_path).expect("create symlink");
            manifest_lines.push(String::from("escape_link.ttf"));
        }

        std::fs::write(fonts_dir.join("manifest.txt"), manifest_lines.join("\n")).expect("write manifest");

        let mut db = fontdb::Database::new();
        let mut ids = Vec::new();
        super::load_dir_into_db(&bundle_dir, &mut db, &mut ids);
        assert!(ids.is_empty(), "manifest traversal/escape entries must not load fonts outside base_dir; loaded={ids:?}");

        // Valid in-directory entry still loads cleanly.
        let inside_font = fonts_dir.join("safe.ttf");
        std::fs::write(&inside_font, &minimal_ttf).expect("write safe font");
        std::fs::write(fonts_dir.join("manifest.txt"), "safe.ttf\n").expect("write safe manifest");
        super::load_dir_into_db(&bundle_dir, &mut db, &mut ids);
        assert!(!ids.is_empty(), "valid relative in-dir manifest entry must load");

        let _ = std::fs::remove_dir_all(&root);
    }
}
