//! R9 statistical option parsers and modal dialog state machines (`QOL-7`, `QOL-12`, `QOL-14`,
//! `QOL-16`, `WF-44..WF-48`).
//!
//! Kept isolated from `dialogs_canvas.rs` so parsers and workflow dialog states share one typed,
//! declarative contract emitting pure [`ToolCommandIntent`] payloads into `lumencraft-engine`.

use lumencraft_doc::{
    AnalysisKind, BracketLabelMode, BracketStyle, BreakOrnament, ChecklistItem, ChecklistStatus, CldOrdering, ClusterDistance, ClusterLinkage,
    CoxTieMethod, EquationDisplayMode, FitMethod, GlmFamily, GraphAnnotation, GraphAxisScale, GraphSheet, JournalWidthPreset, KappaWeighting,
    PValueStackMethod, PageOrientation, PairwiseDisplayMode, PanelBadgeStyle, ParameterRule, ProjectDocument, RectPt, RepeatedMeasuresDesign,
    SheetId, SumOfSquaresType, TwoGroupMethod, WeightingScheme,
};
use lumencraft_palette::{ColormapId, PaletteId};
use lumencraft_render::TiffCompression;
use lumencraft_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_canvas::{ExportFigureFormat, data_column_title};

// ============================================================================
// Shared String / Enum / JSON Parsers (reused across R9.12, R9.14, R9.16, WF-44..48)
// ============================================================================

/// Normalize an identifier to lowercase ASCII alphanumeric for flexible enum parsing.
#[must_use]
pub(crate) fn norm_key(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// Parse a finite `f64` from JSON number or numeric string.
pub(crate) fn parse_finite_f64(val: &Value, field: &str) -> Result<f64, String> {
    val.as_f64()
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("`{field}` must be a finite number"))
}

/// Parse an optional finite `f64` (null or empty string resets to `None`).
pub(crate) fn parse_opt_finite_f64(val: &Value, field: &str) -> Result<Option<f64>, String> {
    if val.is_null() || val.as_str().is_some_and(|s| s.trim().is_empty()) {
        return Ok(None);
    }
    parse_finite_f64(val, field).map(Some)
}

/// Parse a positive `usize` (`>= min_val`) from JSON integer or numeric string.
pub(crate) fn parse_usize_min(val: &Value, field: &str, min_val: usize) -> Result<usize, String> {
    let raw = val
        .as_u64()
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        .ok_or_else(|| format!("`{field}` must be a non-negative integer"))?;
    let n = usize::try_from(raw).map_err(|_| format!("`{field}` exceeds platform usize range"))?;
    if n < min_val {
        return Err(format!("`{field}` ({n}) must be >= {min_val}"));
    }
    Ok(n)
}

/// Parse a `u64` seed or count from JSON integer or string.
pub(crate) fn parse_u64_val(val: &Value, field: &str) -> Result<u64, String> {
    val.as_u64()
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        .ok_or_else(|| format!("`{field}` must be a non-negative integer"))
}

/// Parse a list of `usize` indices from JSON array or comma/semicolon-separated string.
pub(crate) fn parse_usize_list(val: &Value, field: &str) -> Result<Vec<usize>, String> {
    if let Some(arr) = val.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(parse_usize_min(item, field, 0)?);
        }
        return Ok(out);
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for tok in trimmed.split([',', ';', ' ']).filter(|t| !t.trim().is_empty()) {
            let v = tok.trim().parse::<usize>().map_err(|_| format!("`{field}` contains invalid integer `{tok}`"))?;
            out.push(v);
        }
        return Ok(out);
    }
    Err(format!("`{field}` must be an integer array or comma-separated list"))
}

/// Parse a list of finite `f64` values from JSON array or comma-separated string.
pub(crate) fn parse_f64_list(val: &Value, field: &str) -> Result<Vec<f64>, String> {
    if let Some(arr) = val.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(parse_finite_f64(item, field)?);
        }
        return Ok(out);
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for tok in trimmed.split([',', ';', ' ']).filter(|t| !t.trim().is_empty()) {
            let v = tok
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| format!("`{field}` contains invalid finite number `{tok}`"))?;
            out.push(v);
        }
        return Ok(out);
    }
    Err(format!("`{field}` must be a numeric array or comma-separated list"))
}

/// Parse `(column, level)` reference-level overrides from JSON array or `"0:Control, 1:Placebo"` text.
pub(crate) fn parse_reference_levels(val: &Value) -> Result<Vec<(usize, String)>, String> {
    if val.is_null() {
        return Ok(Vec::new());
    }
    if let Some(arr) = val.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            if let Ok(pair) = serde_json::from_value::<(usize, String)>(item.clone()) {
                out.push(pair);
            } else if let Some(obj) = item.as_object() {
                let col = obj.get("column").or_else(|| obj.get("col")).ok_or_else(|| String::from("reference_levels item missing `column`"))?;
                let lvl = obj
                    .get("level")
                    .or_else(|| obj.get("reference"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| String::from("reference_levels item missing non-empty `level`"))?;
                out.push((parse_usize_min(col, "column", 0)?, lvl.to_owned()));
            } else {
                return Err(String::from("`reference_levels` elements must be `[col, level]` or `{column, level}`"));
            }
        }
        return Ok(out);
    }
    if let Some(s) = val.as_str() {
        let mut out = Vec::new();
        for tok in s.split([',', ';']).filter(|t| !t.trim().is_empty()) {
            let (left, right) =
                tok.split_once(':').or_else(|| tok.split_once('=')).ok_or_else(|| format!("expected `col:level` entry, got `{tok}`"))?;
            let col = left.trim().parse::<usize>().map_err(|_| format!("invalid column index `{left}`"))?;
            let lvl = right.trim();
            if lvl.is_empty() {
                return Err(format!("reference level for column {col} cannot be empty"));
            }
            out.push((col, lvl.to_owned()));
        }
        return Ok(out);
    }
    Err(String::from("`reference_levels` must be an array or `col:level` string"))
}

pub(crate) fn parse_kappa_weighting(val: &Value) -> Result<KappaWeighting, String> {
    let raw = val.as_str().ok_or_else(|| String::from("`kappa_weighting` must be a string"))?;
    match norm_key(raw).as_str() {
        "unweighted" | "none" | "simple" => Ok(KappaWeighting::Unweighted),
        "linear" => Ok(KappaWeighting::Linear),
        "quadratic" | "squared" | "fleiss" => Ok(KappaWeighting::Quadratic),
        _ => Err(format!("unknown kappa weighting `{raw}`")),
    }
}

pub(crate) fn parse_two_group_method_str(raw: &str, hypothetical: f64) -> Result<TwoGroupMethod, String> {
    match norm_key(raw).as_str() {
        "unpairedt" | "unpaired" | "studentt" | "t" => Ok(TwoGroupMethod::UnpairedT),
        "welcht" | "welch" => Ok(TwoGroupMethod::WelchT),
        "pairedt" | "paired" => Ok(TwoGroupMethod::PairedT),
        "ratiopairedt" | "ratiopaired" | "ratio" => Ok(TwoGroupMethod::RatioPairedT),
        "lognormalunpairedt" | "lognormal" | "lognormalt" => Ok(TwoGroupMethod::LognormalUnpairedT),
        "mannwhitneyu" | "mannwhitney" | "mw" => Ok(TwoGroupMethod::MannWhitneyU),
        "wilcoxonsignedrank" | "wilcoxon" => Ok(TwoGroupMethod::WilcoxonSignedRank),
        "kolmogorovsmirnov" | "ks" => Ok(TwoGroupMethod::KolmogorovSmirnov),
        "onesamplet" => Ok(TwoGroupMethod::OneSampleT { hypothetical }),
        "onesamplewilcoxon" => Ok(TwoGroupMethod::OneSampleWilcoxon { hypothetical }),
        "pairedsigntest" | "signtest" | "pairedsign" => Ok(TwoGroupMethod::PairedSignTest),
        _ => Err(format!("unknown two-group method `{raw}`")),
    }
}

pub(crate) fn parse_cluster_distance_str(raw: &str) -> Result<ClusterDistance, String> {
    match norm_key(raw).as_str() {
        "euclidean" | "l2" => Ok(ClusterDistance::Euclidean),
        "manhattan" | "cityblock" | "l1" => Ok(ClusterDistance::Manhattan),
        "pearson" | "correlation" => Ok(ClusterDistance::Pearson),
        "cosine" => Ok(ClusterDistance::Cosine),
        _ => Err(format!("unknown cluster distance `{raw}`")),
    }
}

pub(crate) fn parse_cluster_linkage_str(raw: &str) -> Result<ClusterLinkage, String> {
    match norm_key(raw).as_str() {
        "ward" | "wardd2" => Ok(ClusterLinkage::Ward),
        "complete" => Ok(ClusterLinkage::Complete),
        "single" => Ok(ClusterLinkage::Single),
        "upgma" | "average" => Ok(ClusterLinkage::Upgma),
        "wpgma" | "mcquitty" => Ok(ClusterLinkage::Wpgma),
        "centroid" => Ok(ClusterLinkage::Centroid),
        "median" => Ok(ClusterLinkage::Median),
        _ => Err(format!("unknown cluster linkage `{raw}`")),
    }
}

pub(crate) fn parse_glm_family_str(raw: &str) -> Result<GlmFamily, String> {
    match norm_key(raw).as_str() {
        "linear" | "gaussian" | "ols" => Ok(GlmFamily::Linear),
        "logistic" | "binomial" | "logit" => Ok(GlmFamily::Logistic),
        "poisson" | "loglinear" => Ok(GlmFamily::Poisson),
        _ => Err(format!("unknown GLM family `{raw}`")),
    }
}

pub(crate) fn parse_cox_tie_method_str(raw: &str) -> Result<CoxTieMethod, String> {
    match norm_key(raw).as_str() {
        "breslow" => Ok(CoxTieMethod::Breslow),
        "efron" => Ok(CoxTieMethod::Efron),
        _ => Err(format!("unknown Cox tie method `{raw}`")),
    }
}

pub(crate) fn parse_ss_type_str(raw: &str) -> Result<SumOfSquaresType, String> {
    match norm_key(raw).as_str() {
        "typei" | "type1" | "i" | "1" | "sequential" => Ok(SumOfSquaresType::TypeI),
        "typeiii" | "type3" | "iii" | "3" => Ok(SumOfSquaresType::TypeIII),
        _ => Err(format!("unknown sum-of-squares type `{raw}`")),
    }
}

pub(crate) fn parse_rm_design_str(raw: &str) -> Result<RepeatedMeasuresDesign, String> {
    match norm_key(raw).as_str() {
        "none" | "nomatching" => Ok(RepeatedMeasuresDesign::None),
        "rowfactormatched" | "row" | "rowfactor" => Ok(RepeatedMeasuresDesign::RowFactorMatched),
        "columnfactormatched" | "column" | "col" | "columnfactor" => Ok(RepeatedMeasuresDesign::ColumnFactorMatched),
        "bothfactorsmatched" | "both" => Ok(RepeatedMeasuresDesign::BothFactorsMatched),
        _ => Err(format!("unknown repeated measures design `{raw}`")),
    }
}

pub(crate) fn parse_weighting_scheme(val: &Value) -> Result<WeightingScheme, String> {
    if let Ok(direct) = serde_json::from_value::<WeightingScheme>(val.clone()) {
        return Ok(direct);
    }
    let raw = val.as_str().ok_or_else(|| String::from("`weighting` must be a string or WeightingScheme object"))?;
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    match (compact.as_str(), norm_key(raw).as_str()) {
        (_, "none" | "equal" | "unweighted") => Ok(WeightingScheme::None),
        ("1/y^2", _) | (_, "relativesquare" | "1y2") => Ok(WeightingScheme::RelativeSquare),
        ("1/|y|" | "1/y", _) | (_, "relative") => Ok(WeightingScheme::Relative),
        ("1/|x|" | "1/x", _) | (_, "inversex") => Ok(WeightingScheme::InverseX),
        ("1/x^2", _) | (_, "inversex2") => Ok(WeightingScheme::InverseX2),
        (_, "observedsd" | "1sd2") => Ok(WeightingScheme::ObservedSd),
        _ => Err(format!("unknown weighting scheme `{raw}`")),
    }
}

pub(crate) fn parse_fit_method(val: &Value) -> Result<FitMethod, String> {
    if let Ok(direct) = serde_json::from_value::<FitMethod>(val.clone()) {
        return Ok(direct);
    }
    let raw = val.as_str().ok_or_else(|| String::from("`fit_method` must be a string or FitMethod object"))?;
    match norm_key(raw).as_str() {
        "leastsquares" | "ols" | "ls" => Ok(FitMethod::LeastSquares),
        "poissondeviance" | "poisson" => Ok(FitMethod::PoissonDeviance),
        "robustcauchy" | "robust" | "cauchy" => Ok(FitMethod::RobustCauchy),
        "rout" | "routoutlierelimination" => Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }),
        _ => Err(format!("unknown fit method `{raw}`")),
    }
}

pub(crate) fn parse_parameter_rules(val: &Value) -> Result<Vec<ParameterRule>, String> {
    serde_json::from_value::<Vec<ParameterRule>>(val.clone()).map_err(|e| format!("invalid `parameter_rules`: {e}"))
}

pub(crate) fn parse_equation_display_mode_str(raw: &str) -> Result<EquationDisplayMode, String> {
    match norm_key(raw).as_str() {
        "substituted" | "values" | "numeric" | "evaluated" => Ok(EquationDisplayMode::Substituted),
        "symbolic" | "formula" | "parameters" => Ok(EquationDisplayMode::Symbolic),
        _ => Err(format!("unknown equation display mode `{raw}`")),
    }
}

pub(crate) fn equation_display_mode_str(mode: EquationDisplayMode) -> &'static str {
    match mode {
        EquationDisplayMode::Substituted => "substituted",
        EquationDisplayMode::Symbolic => "symbolic",
    }
}

pub(crate) fn parse_axis_scale_str(raw: &str) -> Result<GraphAxisScale, String> {
    match norm_key(raw).as_str() {
        "linear" => Ok(GraphAxisScale::Linear),
        "log10" => Ok(GraphAxisScale::Log10),
        "log2" => Ok(GraphAxisScale::Log2),
        "ln" | "loge" => Ok(GraphAxisScale::Ln),
        "logit" | "logitproportion" => Ok(GraphAxisScale::Logit(lumencraft_doc::LogitDomain::Proportion)),
        "logitpercent" => Ok(GraphAxisScale::Logit(lumencraft_doc::LogitDomain::Percent)),
        "probit" => Ok(GraphAxisScale::Probit),
        "reciprocal" | "1x" => Ok(GraphAxisScale::Reciprocal),
        _ => Err(format!("unknown axis scale `{raw}`")),
    }
}

pub(crate) fn parse_pvalue_stack_method_val(val: &Value) -> Result<PValueStackMethod, String> {
    if let Ok(parsed) = serde_json::from_value::<PValueStackMethod>(val.clone()) {
        return Ok(parsed);
    }
    let raw = val.as_str().ok_or_else(|| String::from("`pvalue_stack.method` must be a valid P-value stack multiplicity method string"))?;
    let folded: String = raw
        .chars()
        .map(|c| match c {
            'š' | 'Š' => 's',
            'í' | 'Í' | 'ì' | 'î' | 'ï' => 'i',
            'á' | 'Á' => 'a',
            _ => c,
        })
        .collect();
    match norm_key(&folded).as_str() {
        "bonferroni" => Ok(PValueStackMethod::Bonferroni),
        "holm" | "holmbonferroni" | "holm1979" => Ok(PValueStackMethod::HolmBonferroni),
        "sidak" | "sidak1967" => Ok(PValueStackMethod::Sidak),
        "holmsidak" => Ok(PValueStackMethod::HolmSidak),
        "bh" | "benjaminihochberg" | "benjaminihochberg1995" | "benjaminihochbergfdr" | "fdrbh" | "fdr" => Ok(PValueStackMethod::BenjaminiHochberg),
        "bky" | "twostagebky" | "twostagebkyfdr" | "benjaminikriegeryekutieli" | "benjaminikriegeryekutielitwostage2006" | "bky2006" => {
            Ok(PValueStackMethod::TwoStageBky)
        }
        "by" | "benjaminiyekutieli" | "benjaminiyekutieli2001" | "benjaminiyekutielifdr" | "fdrby" => Ok(PValueStackMethod::BenjaminiYekutieli),
        _ => Err(format!("unknown P-value stack multiplicity method `{raw}`")),
    }
}

pub(crate) fn parse_pvalue_stack_threshold_val(val: &Value, field: &str) -> Result<f64, String> {
    let t = parse_finite_f64(val, field)?;
    if !(0.0 < t && t < 1.0) {
        return Err(format!("`{field}` ({t}) must lie strictly in (0, 1)"));
    }
    Ok(t)
}

pub(crate) fn parse_break_ornament_str(raw: &str) -> Result<BreakOrnament, String> {
    match norm_key(raw).as_str() {
        "doubleslash" | "slash" => Ok(BreakOrnament::DoubleSlash),
        "zigzag" => Ok(BreakOrnament::Zigzag),
        "blankgap" | "blank" | "gap" => Ok(BreakOrnament::BlankGap),
        _ => Err(format!("unknown break_ornament `{raw}`")),
    }
}

pub(crate) fn parse_bracket_style_str(raw: &str) -> Result<BracketStyle, String> {
    match norm_key(raw).as_str() {
        "square" => Ok(BracketStyle::Square),
        "halftick" => Ok(BracketStyle::HalfTick),
        "flatline" | "flat" => Ok(BracketStyle::FlatLine),
        "curlybrace" | "curly" => Ok(BracketStyle::CurlyBrace),
        _ => Err(format!("unknown bracket_style `{raw}`")),
    }
}

pub(crate) fn parse_bracket_label_mode(val: &Value) -> Result<BracketLabelMode, String> {
    if let Ok(direct) = serde_json::from_value::<BracketLabelMode>(val.clone()) {
        return Ok(direct);
    }
    let raw = val.as_str().ok_or_else(|| String::from("`label_mode` must be a string or BracketLabelMode"))?;
    match norm_key(raw).as_str() {
        "summarystars" | "stars" => Ok(BracketLabelMode::SummaryStars),
        "numericporq" | "numeric" | "pvalue" | "porq" => Ok(BracketLabelMode::NumericPOrQ),
        _ => Ok(BracketLabelMode::Custom(raw.trim().to_owned())),
    }
}

pub(crate) fn parse_pairwise_display_mode_str(raw: &str) -> Result<PairwiseDisplayMode, String> {
    match norm_key(raw).as_str() {
        "bracketsonly" | "brackets" => Ok(PairwiseDisplayMode::BracketsOnly),
        "compactletterdisplayonly" | "cldonly" | "cld" | "lettersonly" | "letters" => Ok(PairwiseDisplayMode::CompactLetterDisplayOnly),
        "bracketsandcompactletterdisplay" | "bracketsandcld" | "both" => Ok(PairwiseDisplayMode::BracketsAndCompactLetterDisplay),
        _ => Err(format!("unknown pairwise display mode `{raw}`")),
    }
}

pub(crate) fn pairwise_display_mode_str(mode: PairwiseDisplayMode) -> &'static str {
    match mode {
        PairwiseDisplayMode::BracketsOnly => "brackets_only",
        PairwiseDisplayMode::CompactLetterDisplayOnly => "compact_letter_display_only",
        PairwiseDisplayMode::BracketsAndCompactLetterDisplay => "brackets_and_compact_letter_display",
    }
}

pub(crate) fn parse_cld_ordering_str(raw: &str) -> Result<CldOrdering, String> {
    match norm_key(raw).as_str() {
        "descendingmean" | "descending" | "desc" | "highestfirst" => Ok(CldOrdering::DescendingMean),
        "ascendingmean" | "ascending" | "asc" | "lowestfirst" => Ok(CldOrdering::AscendingMean),
        _ => Err(format!("unknown CLD ordering `{raw}`")),
    }
}

pub(crate) fn cld_ordering_str(ordering: CldOrdering) -> &'static str {
    match ordering {
        CldOrdering::DescendingMean => "descending_mean",
        CldOrdering::AscendingMean => "ascending_mean",
    }
}

pub(crate) fn parse_colormap_id_str(raw: &str) -> Result<ColormapId, String> {
    match norm_key(raw).as_str() {
        "viridis" => Ok(ColormapId::Viridis),
        "cividis" => Ok(ColormapId::Cividis),
        "plasma" => Ok(ColormapId::Plasma),
        "divergingbluered" | "bluered" => Ok(ColormapId::DivergingBlueRed),
        "grayscale" | "gray" => Ok(ColormapId::Grayscale),
        _ => Err(format!("unknown colormap `{raw}`")),
    }
}

pub(crate) fn parse_palette_str(raw: &str) -> Result<PaletteId, String> {
    match norm_key(raw).as_str() {
        "okabeito" | "default" => Ok(PaletteId::OkabeIto),
        "paultolbright" | "bright" => Ok(PaletteId::PaulTolBright),
        "paultolmuted" | "muted" => Ok(PaletteId::PaulTolMuted),
        "paultolhighcontrast" | "highcontrast" => Ok(PaletteId::PaulTolHighContrast),
        "paultolvibrant" | "vibrant" => Ok(PaletteId::PaulTolVibrant),
        "monochromeprint" | "monochrome" | "grayscale" => Ok(PaletteId::MonochromePrint),
        _ => Err(format!("unknown palette `{raw}`")),
    }
}

pub(crate) fn parse_journal_preset_str(raw: &str) -> Result<JournalWidthPreset, String> {
    match norm_key(raw).as_str() {
        "singlecolumn85mm" | "85mm" => Ok(JournalWidthPreset::SingleColumn85Mm),
        "singlecolumn89mm" | "89mm" => Ok(JournalWidthPreset::SingleColumn89Mm),
        "oneandhalfcolumn120mm" | "120mm" => Ok(JournalWidthPreset::OneAndHalfColumn120Mm),
        "oneandhalfcolumn136mm" | "136mm" => Ok(JournalWidthPreset::OneAndHalfColumn136Mm),
        "doublecolumn174mm" | "174mm" => Ok(JournalWidthPreset::DoubleColumn174Mm),
        "doublecolumn183mm" | "183mm" => Ok(JournalWidthPreset::DoubleColumn183Mm),
        "letterportrait" | "letter" => Ok(JournalWidthPreset::LetterPortrait),
        "letterlandscape" => Ok(JournalWidthPreset::LetterLandscape),
        "a4portrait" | "a4" => Ok(JournalWidthPreset::A4Portrait),
        "a4landscape" => Ok(JournalWidthPreset::A4Landscape),
        _ => Err(format!("unknown journal preset `{raw}`")),
    }
}

pub(crate) fn parse_page_orientation_str(raw: &str) -> Result<PageOrientation, String> {
    match norm_key(raw).as_str() {
        "portrait" => Ok(PageOrientation::Portrait),
        "landscape" => Ok(PageOrientation::Landscape),
        _ => Err(format!("unknown page orientation `{raw}`")),
    }
}

pub(crate) fn parse_panel_badge_style_str(raw: &str) -> Result<PanelBadgeStyle, String> {
    match norm_key(raw).as_str() {
        "uppercaseletters" | "uppercase" | "abc" => Ok(PanelBadgeStyle::UpperCaseLetters),
        "lowercaseletters" | "lowercase" => Ok(PanelBadgeStyle::LowerCaseLetters),
        "none" => Ok(PanelBadgeStyle::None),
        _ => Err(format!("unknown panel badge style `{raw}`")),
    }
}

pub(crate) fn parse_export_format(val: &Value) -> Result<ExportFigureFormat, String> {
    if let Ok(direct) = serde_json::from_value::<ExportFigureFormat>(val.clone()) {
        return Ok(direct);
    }
    let raw = val.as_str().ok_or_else(|| String::from("`format` must be `svg`, `pdf`, `png`, or `tiff`"))?;
    match norm_key(raw).as_str() {
        "svg" => Ok(ExportFigureFormat::Svg),
        "pdf" => Ok(ExportFigureFormat::Pdf),
        "png" => Ok(ExportFigureFormat::Png),
        "tiff" | "tifflzw" | "lzw" => Ok(ExportFigureFormat::Tiff(TiffCompression::Lzw)),
        "tiffnone" | "uncompressedtiff" => Ok(ExportFigureFormat::Tiff(TiffCompression::None)),
        "tiffdeflate" | "deflate" => Ok(ExportFigureFormat::Tiff(TiffCompression::Deflate)),
        _ => Err(format!("unknown export format `{raw}`")),
    }
}

// ============================================================================
// QOL-7 Diagnostics-Only Helpers
// ============================================================================

/// Whether a [`ChecklistStatus::Info`] item reports an inconclusive check or small-sample/asymptotic limitation.
#[must_use]
pub(crate) fn is_inconclusive_info_item(item: &ChecklistItem) -> bool {
    if item.status != ChecklistStatus::Info {
        return false;
    }
    let combined = format!("{} {}", item.assumption, item.explanation).to_ascii_lowercase();
    [
        "small",
        "n <",
        "n=",
        "inconclusive",
        "underpowered",
        "low power",
        "asymptotic",
        "expected <",
        "sparse",
        "ties",
        "unbalanced",
        "too few",
        "insufficient",
        "cannot test",
        "not enough",
        "normality test",
    ]
    .iter()
    .any(|kw| combined.contains(kw))
}

/// Format statistical limitations strictly as neutral diagnostics without recommending alternative
/// tests, data transforms, or outlier exclusion (`QOL-7` policy).
#[must_use]
pub(crate) fn sanitize_diagnostics_only_text(item: &ChecklistItem) -> String {
    let raw = item.explanation.trim();
    if raw.is_empty() {
        return format!("Diagnostic status for {} ({:?}); inspect the Diagnostics tab on this ResultSheet.", item.assumption, item.status);
    }
    let lower = raw.to_ascii_lowercase();
    let prescriptive = [
        "recommend",
        "consider ",
        "consider running",
        "consider switching",
        "switch to",
        "re-run with",
        "use welch",
        "use mann",
        "use kruskal",
        "use wilcoxon",
        "use friedman",
        "apply ",
        "transform ",
        "exclude ",
        "remove outlier",
    ]
    .iter()
    .any(|kw| lower.contains(kw));
    if prescriptive {
        let metric_part = item.diagnostic_metric.as_deref().map(|m| format!(" ({m})")).unwrap_or_default();
        return format!(
            "{} flagged {}{}: statistical assumptions may not hold as tested; review Residuals / Normal Q-Q / Diagnostics tabs or adjust parameters explicitly via Cmd+T.",
            item.assumption,
            match item.status {
                ChecklistStatus::Violated => "violation",
                ChecklistStatus::Warn => "caution",
                ChecklistStatus::Info => "an inconclusive or small-sample limitation",
                ChecklistStatus::Pass => "pass",
            },
            metric_part
        );
    }
    raw.to_owned()
}

// ============================================================================
// R9.14 Dialogs: Randomize Groups, Random Sample Rows, Simulate Distributions
// ============================================================================

/// Allocation scheme for [`RandomizeGroupsDialogState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RandomizationSchemeKind {
    #[default]
    CompleteBalanced,
    PermutedBlock,
}

/// Modal state for `Randomize Subjects to Groups...` (`R9.14`, `table.randomizeGroups`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomizeGroupsDialogState {
    pub groups: Vec<(String, usize)>,
    pub subject_count: usize,
    pub subject_ids: Vec<String>,
    pub scheme: RandomizationSchemeKind,
    pub block_sizes: Vec<usize>,
    pub seed: u64,
    pub title: String,
    pub target_info_sheet_id: Option<SheetId>,
}

impl Default for RandomizeGroupsDialogState {
    fn default() -> Self {
        Self {
            groups: vec![(String::from("Control"), 1), (String::from("Treatment"), 1)],
            subject_count: 12,
            subject_ids: Vec::new(),
            scheme: RandomizationSchemeKind::CompleteBalanced,
            block_sizes: vec![4],
            seed: 1,
            title: String::from("Randomization Schedule"),
            target_info_sheet_id: None,
        }
    }
}

impl RandomizeGroupsDialogState {
    /// Synchronize default InfoSheet destination from `doc` when opening the dialog.
    pub fn open_for_document(&mut self, doc: &ProjectDocument) {
        self.target_info_sheet_id = doc.info_sheets.first().map(|s| s.id.clone());
    }

    /// Parse comma/semicolon-separated `"Arm, Arm2:2"` group entries.
    pub fn set_groups_from_str(&mut self, raw: &str) -> Result<(), String> {
        let mut parsed = Vec::new();
        for tok in raw.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()) {
            if let Some((name, w_str)) = tok.split_once(':') {
                let label = name.trim();
                let weight = w_str.trim().parse::<usize>().map_err(|_| format!("invalid group weight `{w_str}`"))?;
                if label.is_empty() || weight == 0 {
                    return Err(format!("invalid weighted group specification `{tok}`"));
                }
                parsed.push((label.to_owned(), weight));
            } else {
                parsed.push((tok.to_owned(), 1));
            }
        }
        if parsed.is_empty() {
            return Err(String::from("`groups` must contain at least one treatment arm"));
        }
        self.groups = parsed;
        Ok(())
    }

    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "groups" | "randomize.groups" => {
                    if let Some(s) = val.as_str() {
                        self.set_groups_from_str(s)?;
                    } else if let Some(arr) = val.as_array() {
                        let mut next = Vec::with_capacity(arr.len());
                        for item in arr {
                            if let Some(label) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                next.push((label.to_owned(), 1));
                            } else if let Some(obj) = item.as_object() {
                                let label = obj
                                    .get("label")
                                    .or_else(|| obj.get("name"))
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .ok_or_else(|| String::from("group object missing non-empty `label`"))?;
                                let weight = match obj.get("weight") {
                                    Some(w) => parse_usize_min(w, "weight", 1)?,
                                    None => 1,
                                };
                                next.push((label.to_owned(), weight));
                            } else {
                                return Err(String::from("`groups` items must be strings or `{label, weight}` objects"));
                            }
                        }
                        if next.is_empty() {
                            return Err(String::from("`groups` must contain at least one group"));
                        }
                        self.groups = next;
                    } else {
                        return Err(String::from("`groups` must be an array or comma-separated string"));
                    }
                }
                "subject_count" | "n_subjects" | "randomize.subject_count" => {
                    self.subject_count = parse_usize_min(val, key, 1)?;
                }
                "subject_ids" | "randomize.subject_ids" => {
                    if val.is_null() {
                        self.subject_ids.clear();
                    } else if let Some(arr) = val.as_array() {
                        self.subject_ids = arr.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())).collect();
                    } else if let Some(s) = val.as_str() {
                        self.subject_ids = s.split([',', ';', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(ToOwned::to_owned).collect();
                    } else {
                        return Err(String::from("`subject_ids` must be an array, string, or null"));
                    }
                }
                "scheme" | "randomize.scheme" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`scheme` must be a string"))?;
                    self.scheme = match norm_key(raw).as_str() {
                        "completebalanced" | "complete" | "balanced" => RandomizationSchemeKind::CompleteBalanced,
                        "permutedblock" | "block" => RandomizationSchemeKind::PermutedBlock,
                        _ => return Err(format!("unknown randomization scheme `{raw}`")),
                    };
                }
                "block_sizes" | "block_size" | "randomize.block_sizes" => {
                    let blocks = if val.is_u64() { vec![parse_usize_min(val, key, 1)?] } else { parse_usize_list(val, key)? };
                    if blocks.is_empty() || blocks.contains(&0) {
                        return Err(String::from("`block_sizes` entries must be positive integers"));
                    }
                    self.block_sizes = blocks;
                }
                "seed" | "randomize.seed" => {
                    self.seed = parse_u64_val(val, key)?;
                }
                "title" | "randomize.title" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.trim();
                    if !raw.is_empty() {
                        self.title = raw.to_owned();
                    }
                }
                "info_sheet_id" | "target_info_sheet_id" | "randomize.info_sheet_id" => {
                    self.target_info_sheet_id = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(SheetId::from);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Emit `table.randomizeGroups` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        if self.groups.is_empty() || (self.subject_ids.is_empty() && self.subject_count == 0) {
            return None;
        }
        let groups_json: Vec<Value> = self.groups.iter().map(|(label, weight)| json!({ "label": label, "weight": weight })).collect();
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("groups"), Value::Array(groups_json));
        if self.subject_ids.is_empty() {
            payload.insert(String::from("subject_count"), json!(self.subject_count));
        } else {
            payload.insert(String::from("subject_ids"), json!(self.subject_ids));
        }
        payload.insert(
            String::from("scheme"),
            json!(match self.scheme {
                RandomizationSchemeKind::CompleteBalanced => "complete_balanced",
                RandomizationSchemeKind::PermutedBlock => "permuted_block",
            }),
        );
        if self.scheme == RandomizationSchemeKind::PermutedBlock && !self.block_sizes.is_empty() {
            payload.insert(String::from("block_sizes"), json!(self.block_sizes));
        }
        payload.insert(String::from("seed"), json!(self.seed));
        if !self.title.trim().is_empty() {
            payload.insert(String::from("title"), json!(self.title.trim()));
        }
        if let Some(iid) = &self.target_info_sheet_id {
            payload.insert(String::from("info_sheet_id"), json!(iid.as_str()));
        }
        Some(ToolCommandIntent::new("table.randomizeGroups", Value::Object(payload)))
    }
}

/// Modal state for `Random Sample Rows...` (`R9.14`, `table.randomSampleRows`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomSampleRowsDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub sample_size: usize,
    pub seed: u64,
    pub stratify_column: Option<usize>,
    pub proportional_allocation: bool,
    pub title: String,
    pub target_info_sheet_id: Option<SheetId>,
    pub available_columns: Vec<String>,
}

impl Default for RandomSampleRowsDialogState {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            sample_size: 10,
            seed: 1,
            stratify_column: None,
            proportional_allocation: true,
            title: String::from("Random Sample"),
            target_info_sheet_id: None,
            available_columns: Vec::new(),
        }
    }
}

impl RandomSampleRowsDialogState {
    /// Load source sheet columns and InfoSheet defaults for `source_sheet_id`.
    pub fn open_for_sheet(&mut self, doc: &ProjectDocument, source_sheet_id: &SheetId) -> bool {
        let Some(ds) = doc.data_sheet(source_sheet_id) else {
            return false;
        };
        self.source_sheet_id = Some(ds.id.clone());
        self.target_info_sheet_id = ds.info_sheet_id.clone().or_else(|| doc.info_sheets.first().map(|s| s.id.clone()));
        let col_count = ds.archetype.column_count();
        self.available_columns = (0..col_count)
            .map(|i| data_column_title(ds, i).map_or_else(|| format!("Col {}", lumencraft_doc::default_column_label(i)), ToOwned::to_owned))
            .collect();
        self.sample_size = self.sample_size.max(1).min(ds.archetype.row_count().max(1));
        true
    }

    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "source_sheet_id" | "sample.source_sheet_id" => {
                    if let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        self.source_sheet_id = Some(SheetId::from(s));
                    }
                }
                "sample_size" | "sample.sample_size" => {
                    self.sample_size = parse_usize_min(val, key, 1)?;
                }
                "seed" | "sample.seed" => {
                    self.seed = parse_u64_val(val, key)?;
                }
                "stratify_column" | "sample.stratify_column" => {
                    if val.is_null() {
                        self.stratify_column = None;
                    } else {
                        self.stratify_column = Some(parse_usize_min(val, key, 0)?);
                    }
                }
                "proportional_allocation" | "sample.proportional_allocation" => {
                    self.proportional_allocation = val.as_bool().ok_or_else(|| String::from("`proportional_allocation` must be boolean"))?;
                }
                "title" | "sample.title" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.trim();
                    if !raw.is_empty() {
                        self.title = raw.to_owned();
                    }
                }
                "info_sheet_id" | "target_info_sheet_id" | "sample.info_sheet_id" => {
                    self.target_info_sheet_id = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(SheetId::from);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Emit `table.randomSampleRows` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        if self.sample_size == 0 {
            return None;
        }
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("source_sheet_id"), json!(src.as_str()));
        payload.insert(String::from("sample_size"), json!(self.sample_size));
        payload.insert(String::from("seed"), json!(self.seed));
        if let Some(col) = self.stratify_column {
            payload.insert(String::from("stratify_column"), json!(col));
            payload.insert(String::from("proportional_allocation"), json!(self.proportional_allocation));
        }
        if !self.title.trim().is_empty() {
            payload.insert(String::from("title"), json!(self.title.trim()));
        }
        if let Some(iid) = &self.target_info_sheet_id {
            payload.insert(String::from("info_sheet_id"), json!(iid.as_str()));
        }
        Some(ToolCommandIntent::new("table.randomSampleRows", Value::Object(payload)))
    }
}

/// Parametric distribution family for [`SimulateDistributionsDialogState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimDistKind {
    Uniform,
    #[default]
    Gaussian,
    LogNormal,
    Binomial,
    Poisson,
    Exponential,
}

impl SimDistKind {
    /// Canonical lowercase name accepted by `table.simulateDistributions`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Gaussian => "gaussian",
            Self::LogNormal => "log_normal",
            Self::Binomial => "binomial",
            Self::Poisson => "poisson",
            Self::Exponential => "exponential",
        }
    }

    fn from_str(raw: &str) -> Result<Self, String> {
        match norm_key(raw).as_str() {
            "uniform" => Ok(Self::Uniform),
            "gaussian" | "normal" => Ok(Self::Gaussian),
            "lognormal" => Ok(Self::LogNormal),
            "binomial" => Ok(Self::Binomial),
            "poisson" => Ok(Self::Poisson),
            "exponential" | "exp" => Ok(Self::Exponential),
            _ => Err(format!("unknown simulated distribution `{raw}`")),
        }
    }
}

/// One column specification in [`SimulateDistributionsDialogState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulatedDistColumnDraft {
    pub name: String,
    pub distribution: SimDistKind,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub sd: f64,
    pub mean_log: f64,
    pub sd_log: f64,
    pub trials: u64,
    pub p: f64,
    pub lambda: f64,
    pub rate: f64,
}

impl Default for SimulatedDistColumnDraft {
    fn default() -> Self {
        Self {
            name: String::from("Simulated 1"),
            distribution: SimDistKind::Gaussian,
            min: 0.0,
            max: 1.0,
            mean: 0.0,
            sd: 1.0,
            mean_log: 0.0,
            sd_log: 0.5,
            trials: 10,
            p: 0.5,
            lambda: 3.0,
            rate: 1.0,
        }
    }
}

impl SimulatedDistColumnDraft {
    /// Convert draft column into `table.simulateDistributions` column JSON object.
    #[must_use]
    pub fn to_engine_json(&self) -> Value {
        match self.distribution {
            SimDistKind::Uniform => json!({ "name": self.name, "distribution": "uniform", "min": self.min, "max": self.max }),
            SimDistKind::Gaussian => json!({ "name": self.name, "distribution": "gaussian", "mean": self.mean, "sd": self.sd }),
            SimDistKind::LogNormal => json!({ "name": self.name, "distribution": "log_normal", "mean_log": self.mean_log, "sd_log": self.sd_log }),
            SimDistKind::Binomial => json!({ "name": self.name, "distribution": "binomial", "trials": self.trials, "p": self.p }),
            SimDistKind::Poisson => json!({ "name": self.name, "distribution": "poisson", "lambda": self.lambda }),
            SimDistKind::Exponential => json!({ "name": self.name, "distribution": "exponential", "rate": self.rate }),
        }
    }

    fn apply_object(&mut self, obj: &serde_json::Map<String, Value>) -> Result<(), String> {
        if let Some(name) = obj.get("name").or_else(|| obj.get("title")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
            self.name = name.to_owned();
        }
        if let Some(dist_raw) =
            obj.get("distribution").and_then(|d| d.as_str().or_else(|| d.as_object().and_then(|o| o.get("kind").and_then(Value::as_str))))
        {
            self.distribution = SimDistKind::from_str(dist_raw)?;
        }
        for (field, val) in obj {
            match field.as_str() {
                "min" => self.min = parse_finite_f64(val, field)?,
                "max" => self.max = parse_finite_f64(val, field)?,
                "mean" | "mu" => self.mean = parse_finite_f64(val, field)?,
                "sd" | "std" | "sigma" => self.sd = parse_finite_f64(val, field)?,
                "mean_log" | "meanlog" => self.mean_log = parse_finite_f64(val, field)?,
                "sd_log" | "sdlog" => self.sd_log = parse_finite_f64(val, field)?,
                "trials" | "n" => self.trials = parse_u64_val(val, field)?,
                "p" | "prob" | "probability" => self.p = parse_finite_f64(val, field)?,
                "lambda" => self.lambda = parse_finite_f64(val, field)?,
                "rate" => self.rate = parse_finite_f64(val, field)?,
                _ => {}
            }
        }
        Ok(())
    }
}

/// Modal state for `Simulate Distributions...` (`R9.14`, `table.simulateDistributions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulateDistributionsDialogState {
    pub n_rows: usize,
    pub seed: u64,
    pub use_multiple_variables: bool,
    pub title: String,
    pub columns: Vec<SimulatedDistColumnDraft>,
}

impl Default for SimulateDistributionsDialogState {
    fn default() -> Self {
        Self {
            n_rows: 30,
            seed: 42,
            use_multiple_variables: false,
            title: String::from("Simulated Distributions"),
            columns: vec![SimulatedDistColumnDraft::default()],
        }
    }
}

impl SimulateDistributionsDialogState {
    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        if self.columns.is_empty() {
            self.columns.push(SimulatedDistColumnDraft::default());
        }
        for (key, val) in fields {
            match key.as_str() {
                "n_rows" | "row_count" | "rows" | "simulate.n_rows" => {
                    self.n_rows = parse_usize_min(val, key, 1)?;
                }
                "seed" | "simulate.seed" => {
                    self.seed = parse_u64_val(val, key)?;
                }
                "use_multiple_variables" | "multiple_variables" | "simulate.use_multiple_variables" => {
                    self.use_multiple_variables = val.as_bool().ok_or_else(|| String::from("`use_multiple_variables` must be boolean"))?;
                }
                "title" | "simulate.title" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.trim();
                    if !raw.is_empty() {
                        self.title = raw.to_owned();
                    }
                }
                "columns" | "simulate.columns" => {
                    let arr = val.as_array().ok_or_else(|| String::from("`columns` must be an array"))?;
                    if arr.is_empty() {
                        return Err(String::from("`columns` cannot be empty"));
                    }
                    let mut parsed = Vec::with_capacity(arr.len());
                    for item in arr {
                        let obj = item.as_object().ok_or_else(|| String::from("each column item must be an object"))?;
                        let mut col = SimulatedDistColumnDraft::default();
                        col.apply_object(obj)?;
                        if let Some(params) = obj.get("params").and_then(Value::as_object) {
                            col.apply_object(params)?;
                        }
                        if let Some(params) = obj.get("distribution").and_then(Value::as_object) {
                            col.apply_object(params)?;
                        }
                        parsed.push(col);
                    }
                    self.columns = parsed;
                }
                "column_name" | "simulate.column_name" => {
                    let s = val.as_str().ok_or_else(|| String::from("`column_name` must be a string"))?.trim();
                    if !s.is_empty() {
                        self.columns[0].name = s.to_owned();
                    }
                }
                "distribution" | "simulate.distribution" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`distribution` must be a string"))?;
                    self.columns[0].distribution = SimDistKind::from_str(raw)?;
                }
                "min" | "simulate.min" => self.columns[0].min = parse_finite_f64(val, key)?,
                "max" | "simulate.max" => self.columns[0].max = parse_finite_f64(val, key)?,
                "mean" | "simulate.mean" => self.columns[0].mean = parse_finite_f64(val, key)?,
                "sd" | "simulate.sd" => self.columns[0].sd = parse_finite_f64(val, key)?,
                "mean_log" | "simulate.mean_log" => self.columns[0].mean_log = parse_finite_f64(val, key)?,
                "sd_log" | "simulate.sd_log" => self.columns[0].sd_log = parse_finite_f64(val, key)?,
                "trials" | "simulate.trials" => self.columns[0].trials = parse_u64_val(val, key)?,
                "p" | "simulate.p" => self.columns[0].p = parse_finite_f64(val, key)?,
                "lambda" | "simulate.lambda" => self.columns[0].lambda = parse_finite_f64(val, key)?,
                "rate" | "simulate.rate" => self.columns[0].rate = parse_finite_f64(val, key)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Emit `table.simulateDistributions` engine command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        if self.columns.is_empty() || self.n_rows == 0 {
            return None;
        }
        let cols: Vec<Value> = self.columns.iter().map(SimulatedDistColumnDraft::to_engine_json).collect();
        Some(ToolCommandIntent::new(
            "table.simulateDistributions",
            json!({
                "columns": cols,
                "n_rows": self.n_rows,
                "seed": self.seed,
                "use_multiple_variables": self.use_multiple_variables,
                "title": self.title.trim(),
            }),
        ))
    }
}

// ============================================================================
// R9.16 Dialog: Insert / Toggle ModelEquation Annotation (strictly equations only)
// ============================================================================

/// Modal state for inserting or toggling a `GraphAnnotation::ModelEquation` callout (`QOL-16`).
///
/// Strictly equations-only: renders symbolic or substituted model equations from an existing
/// regression [`ResultSheet`] without Compact Letter Display (CLD) or fit-statistic callouts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertModelEquationDialogState {
    pub graph_sheet_id: Option<SheetId>,
    pub result_sheet_id: Option<SheetId>,
    pub available_result_ids: Vec<SheetId>,
    pub data_set_index: usize,
    pub mode: EquationDisplayMode,
    pub font_size_pt: f64,
    pub bordered: bool,
    pub bounds_pt: RectPt,
    pub update_existing: bool,
}

impl Default for InsertModelEquationDialogState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            result_sheet_id: None,
            available_result_ids: Vec::new(),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            font_size_pt: 10.0,
            bordered: false,
            bounds_pt: RectPt::new(24.0, 24.0, 210.0, 42.0),
            update_existing: false,
        }
    }
}

impl InsertModelEquationDialogState {
    /// Discover equation-bearing regression result sheets and prefill existing annotation state.
    pub fn open_for_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.available_result_ids = doc
            .result_sheets
            .iter()
            .filter(|rs| {
                matches!(
                    rs.analysis.kind,
                    AnalysisKind::NonlinearRegression(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::GeneralizedLinearModel(_)
                ) || !rs.fitted_curves.is_empty()
            })
            .map(|rs| rs.id.clone())
            .collect();
        if let Some(GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered }) =
            graph.spec.annotations.iter().find(|a| matches!(a, GraphAnnotation::ModelEquation { .. }))
        {
            self.result_sheet_id = Some(result_sheet_id.clone());
            self.data_set_index = *data_set_index;
            self.mode = *mode;
            self.bounds_pt = *bounds_pt;
            self.font_size_pt = *font_size_pt;
            self.bordered = *bordered;
            self.update_existing = true;
        } else {
            self.update_existing = false;
            if self.result_sheet_id.as_ref().is_none_or(|id| !self.available_result_ids.contains(id)) {
                self.result_sheet_id = self.available_result_ids.first().cloned();
            }
        }
    }

    /// Toggle draft display mode between `Substituted` (fitted parameters) and `Symbolic` (formula).
    pub fn toggle_mode(&mut self) -> EquationDisplayMode {
        self.mode = match self.mode {
            EquationDisplayMode::Substituted => EquationDisplayMode::Symbolic,
            EquationDisplayMode::Symbolic => EquationDisplayMode::Substituted,
        };
        self.mode
    }

    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "result_sheet_id" | "equation.result_sheet_id" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`result_sheet_id` must be a string"))?.trim();
                    if raw.is_empty() {
                        return Err(String::from("`result_sheet_id` cannot be empty"));
                    }
                    self.result_sheet_id = Some(SheetId::from(raw));
                }
                "data_set_index" | "dataset_index" | "equation.data_set_index" => {
                    self.data_set_index = parse_usize_min(val, key, 0)?;
                }
                "mode" | "display_mode" | "equation.mode" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`mode` must be `substituted` or `symbolic`"))?;
                    self.mode = parse_equation_display_mode_str(raw)?;
                }
                "font_size_pt" | "equation.font_size_pt" => {
                    let pt = parse_finite_f64(val, key)?;
                    if !(4.0..=72.0).contains(&pt) {
                        return Err(format!("`font_size_pt` ({pt}) must be within 4.0..=72.0"));
                    }
                    self.font_size_pt = pt;
                }
                "bordered" | "equation.bordered" => {
                    self.bordered = val.as_bool().ok_or_else(|| String::from("`bordered` must be boolean"))?;
                }
                "update_existing" | "equation.update_existing" => {
                    self.update_existing = val.as_bool().ok_or_else(|| String::from("`update_existing` must be boolean"))?;
                }
                "x_pt" | "equation.x_pt" => self.bounds_pt.x = parse_finite_f64(val, key)?,
                "y_pt" | "equation.y_pt" => self.bounds_pt.y = parse_finite_f64(val, key)?,
                "width_pt" | "equation.width_pt" => self.bounds_pt.width = parse_finite_f64(val, key)?.max(24.0),
                "height_pt" | "equation.height_pt" => self.bounds_pt.height = parse_finite_f64(val, key)?.max(12.0),
                _ => {}
            }
        }
        Ok(())
    }

    /// Emit `graph.addAnnotation` (`kind: "equation"`) or update existing `ModelEquation` via `graph.setSpec`.
    #[must_use]
    pub fn commit_intent(&self, doc: Option<&ProjectDocument>) -> Option<ToolCommandIntent> {
        let graph_id = self.graph_sheet_id.as_ref()?;
        let res_id = self.result_sheet_id.as_ref()?;
        if self.update_existing
            && let Some(gs) = doc.and_then(|d| d.graph_sheet(graph_id))
            && gs.spec.annotations.iter().any(|ann| matches!(ann, GraphAnnotation::ModelEquation { .. }))
        {
            let mut next_annotations = gs.spec.annotations.clone();
            let mut updated = false;
            for ann in &mut next_annotations {
                if let GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } = ann
                    && (result_sheet_id == res_id || !updated)
                {
                    *result_sheet_id = res_id.clone();
                    *data_set_index = self.data_set_index;
                    *mode = self.mode;
                    *bounds_pt = self.bounds_pt;
                    *font_size_pt = self.font_size_pt;
                    *bordered = self.bordered;
                    updated = true;
                }
            }
            if updated {
                return Some(ToolCommandIntent::new(
                    "graph.setSpec",
                    json!({
                        "sheet_id": graph_id.as_str(),
                        "annotations": next_annotations,
                    }),
                ));
            }
        }
        Some(ToolCommandIntent::new(
            "graph.addAnnotation",
            json!({
                "sheet_id": graph_id.as_str(),
                "kind": "equation",
                "result_sheet_id": res_id.as_str(),
                "data_set_index": self.data_set_index,
                "mode": equation_display_mode_str(self.mode),
                "bounds_pt": self.bounds_pt,
                "font_size_pt": self.font_size_pt,
                "bordered": self.bordered,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, ColumnTable, DataSheet, GraphKind, GraphSheet, LinearRegressionSpec,
        ProjectDocument, RemediationKind, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, TableArchetype,
    };
    use serde_json::json;

    #[test]
    fn r9_14_dialogs_validate_patches_and_emit_expected_intents() {
        let mut rand_dlg = RandomizeGroupsDialogState::default();
        let patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "groups": "Vehicle:1, DrugA:2",
            "subject_count": 9,
            "scheme": "permuted_block",
            "block_sizes": [3, 6],
            "seed": 77,
            "title": "Preclinical Arms"
        }))
        .unwrap();
        rand_dlg.apply_field_patch(&patch).unwrap();
        let intent = rand_dlg.commit_intent().expect("randomize intent");
        assert_eq!(intent.command_id, "table.randomizeGroups");
        assert_eq!(intent.params["scheme"], "permuted_block");
        assert_eq!(intent.params["block_sizes"], json!([3, 6]));
        assert_eq!(intent.params["subject_count"], 9);

        let mut sample_dlg = RandomSampleRowsDialogState::default();
        let mut col = ColumnTable::new(lumencraft_doc::ColumnDesignHint::UnpairedReplicates, 2, 12).unwrap();
        col.data_sets[0].title = String::from("Weight");
        col.data_sets[1].title = String::from("Group");
        let ds = DataSheet::new("data-src", "Cohort", TableArchetype::Column(col)).unwrap();
        let mut doc = ProjectDocument::new("Sample Test").unwrap();
        doc.add_data_sheet(ds).unwrap();
        assert!(sample_dlg.open_for_sheet(&doc, &SheetId::new("data-src")));
        let s_patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "sample_size": 6,
            "seed": 19,
            "stratify_column": 1,
            "proportional_allocation": false
        }))
        .unwrap();
        sample_dlg.apply_field_patch(&s_patch).unwrap();
        let s_intent = sample_dlg.commit_intent().expect("sample intent");
        assert_eq!(s_intent.command_id, "table.randomSampleRows");
        assert_eq!(s_intent.params["stratify_column"], 1);
        assert_eq!(s_intent.params["proportional_allocation"], false);

        let mut sim_dlg = SimulateDistributionsDialogState::default();
        let sim_patch = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "n_rows": 24,
            "seed": 11,
            "use_multiple_variables": true,
            "columns": [
                { "name": "Control", "distribution": "gaussian", "mean": 10.0, "sd": 2.0 },
                { "name": "Counts", "distribution": "poisson", "lambda": 4.5 },
                { "name": "Latency", "distribution": "exponential", "rate": 0.25 }
            ]
        }))
        .unwrap();
        sim_dlg.apply_field_patch(&sim_patch).unwrap();
        let sim_intent = sim_dlg.commit_intent().expect("sim intent");
        assert_eq!(sim_intent.command_id, "table.simulateDistributions");
        assert_eq!(sim_intent.params["n_rows"], 24);
        assert_eq!(sim_intent.params["use_multiple_variables"], true);
        assert_eq!(sim_intent.params["columns"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn r9_16_insert_model_equation_toggles_modes_and_excludes_cld() {
        let mut doc = ProjectDocument::new("Eq Project").unwrap();
        let col = ColumnTable::new(lumencraft_doc::ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        doc.add_data_sheet(DataSheet::new("data-1", "Raw", TableArchetype::Column(col)).unwrap()).unwrap();
        let res_spec = AnalysisSpec::new("res-lin", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
        let mut res = ResultSheet::new(res_spec).unwrap();
        res.status = ResultStatus::UpToDate;
        doc.add_result_sheet(res).unwrap();
        let mut graph = GraphSheet::new_uninitialized_for_data("g-lin", doc.data_sheet(&"data-1".into()).unwrap()).unwrap();
        graph.initialized = true;
        graph.spec.kind = GraphKind::Xy;
        doc.add_graph_sheet(graph).unwrap();

        let mut dlg = InsertModelEquationDialogState::default();
        dlg.open_for_graph(doc.graph_sheet(&"g-lin".into()).unwrap(), &doc);
        assert_eq!(dlg.result_sheet_id.as_ref().map(SheetId::as_str), Some("res-lin"));
        assert_eq!(dlg.mode, EquationDisplayMode::Substituted);

        let insert_intent = dlg.commit_intent(Some(&doc)).expect("insert equation");
        assert_eq!(insert_intent.command_id, "graph.addAnnotation");
        assert_eq!(insert_intent.params["kind"], "equation");
        assert_eq!(insert_intent.params["mode"], "substituted");

        assert_eq!(dlg.toggle_mode(), EquationDisplayMode::Symbolic);
        // Simulate updating an already-present ModelEquation annotation via `graph.setSpec`.
        doc.graph_sheet_mut(&"g-lin".into()).unwrap().spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("res-lin"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(10.0, 10.0, 180.0, 36.0),
            font_size_pt: 10.0,
            bordered: false,
        });
        dlg.open_for_graph(doc.graph_sheet(&"g-lin".into()).unwrap(), &doc);
        assert!(dlg.update_existing);
        assert_eq!(dlg.toggle_mode(), EquationDisplayMode::Symbolic);
        let update_intent = dlg.commit_intent(Some(&doc)).expect("update equation");
        assert_eq!(update_intent.command_id, "graph.setSpec");
        let serialized = serde_json::to_string(&update_intent).unwrap();
        assert!(serialized.contains("\"ModelEquation\""));
        assert!(!serialized.contains("CompactLetterDisplay"));
    }

    #[test]
    fn r9_7_sanitize_preserves_metrics_and_collects_diagnostic_tabs() {
        let violated = ChecklistItem {
            assumption: String::from("Homoscedasticity"),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some(String::from("F(1, 14) = 8.42, p = 0.011")),
            explanation: String::from("Equal variance violated; consider Welch's t test and switch to nonparametric rank tests."),
            remediation: Some(RemediationKind::WelchT),
        };
        let cleaned = sanitize_diagnostics_only_text(&violated);
        assert!(!cleaned.to_ascii_lowercase().contains("consider"));
        assert!(!cleaned.to_ascii_lowercase().contains("switch to"));
        assert!(cleaned.contains("F(1, 14) = 8.42, p = 0.011"));
        let info_small_n = ChecklistItem {
            assumption: String::from("Normality"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("n = 4")),
            explanation: String::from("Inconclusive normality power with small sample size."),
            remediation: Some(RemediationKind::MannWhitneyU),
        };
        assert!(is_inconclusive_info_item(&info_small_n));
        let tab = ResultTab {
            id: String::from("qq"),
            title: String::from("Q-Q"),
            kind: ResultTabKind::NormalQq,
            column_headers: vec![String::from("Q")],
            row_labels: vec![String::from("1")],
            rows: vec![vec![ResultCell::Text(String::from("ok"))]],
            footnotes: vec![],
        };
        assert_eq!(tab.kind, ResultTabKind::NormalQq);
    }
}
