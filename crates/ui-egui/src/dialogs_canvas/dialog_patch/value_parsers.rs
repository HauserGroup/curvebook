//! Consolidated locale-aware numeric, list, color, dash, enum shorthand, and string normalizers
//! used by `dialog_patch` before `serde_json` merging.

use curvebook_doc::{
    AxisThreshold, BaselineOperation, BracketLabelMode, BracketStyle, BreakOrnament, CldOrdering, ClusterDistance, ClusterLinkage, ContingencyMethod,
    CoxTieMethod, EquationDisplayMode, ErrorBarMetric, ErrorPropagationMode, FitMethod, FramePreset, GlmFamily, GraphAxisScale, GraphKind,
    GroupedArrangement, JournalWidthPreset, KappaWeighting, MathTransformPreset, MixedModelDfMethod, MultiplicityMethod, OneWayMethod,
    PValueStackMethod, PageOrientation, PairwiseDisplayMode, PanelBadgeStyle, ParameterRule, PowerSolveTarget, PowerTestFamily,
    RepeatedMeasuresDesign, ResidualCovariance, SubcolumnAggregation, SumOfSquaresType, SummaryMeasure, SurvivalDisplayMetric, TableKind,
    TickDirection, TwoGroupMethod, WeightingScheme,
};
use curvebook_palette::{ColormapId, LineDashStyle, PaletteId, Rgba8};
use curvebook_render::TiffCompression;
use serde_json::Value;

use crate::dialogs_canvas::ExportFigureFormat;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;

/// Normalize an identifier to lowercase ASCII alphanumeric for flexible enum parsing.
#[must_use]
pub(super) fn norm_key(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

macro_rules! norm_enum_parser {
    ($fn_name:ident, $ty:ty, $err_label:literal, $( $($pat:literal)|+ => $expr:expr ),+ $(,)?) => {
        pub(super) fn $fn_name(raw: &str) -> Result<$ty, String> {
            match norm_key(raw).as_str() {
                $( $($pat)|+ => Ok($expr), )+
                _ => Err(format!(concat!("unknown ", $err_label, " `{}`"), raw)),
            }
        }
    };
}

pub(super) fn parse_bool_field(val: &Value, field: &str) -> Result<bool, String> {
    val.as_bool().ok_or_else(|| format!("`{field}` must be boolean"))
}

pub(super) fn parse_str_field<'a>(val: &'a Value, field: &str) -> Result<&'a str, String> {
    val.as_str().ok_or_else(|| format!("`{field}` must be a string"))
}

pub(super) fn parse_finite_f64_with_locale(val: &Value, field: &str, preset: &LocalePreset) -> Result<f64, String> {
    val.as_f64()
        .or_else(|| val.as_str().and_then(|s| parse_localized_number_input(s, preset)))
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("`{field}` must be a finite number"))
}

pub(super) fn parse_opt_finite_f64_with_locale(val: &Value, field: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    if val.is_null() || val.as_str().is_some_and(|s| s.trim().is_empty()) {
        return Ok(None);
    }
    parse_finite_f64_with_locale(val, field, preset).map(Some)
}

pub(super) fn parse_usize_min(val: &Value, field: &str, min_val: usize) -> Result<usize, String> {
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

pub(super) fn parse_opt_usize_min(val: &Value, key: &str, min: usize) -> Result<Option<usize>, String> {
    if val.is_null() { Ok(None) } else { Ok(Some(parse_usize_min(val, key, min)?)) }
}

pub(super) fn parse_u64_val(val: &Value, field: &str) -> Result<u64, String> {
    val.as_u64()
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        .ok_or_else(|| format!("`{field}` must be a non-negative integer"))
}

pub(super) fn parse_positive_usize_arg(val: &Value, field: &str, preset: &LocalePreset) -> Result<usize, String> {
    if let Some(u) = val.as_u64().filter(|&n| n >= 1) {
        return usize::try_from(u).map_err(|_| format!("`{field}` exceeds platform integer bounds"));
    }
    let f = parse_finite_f64_with_locale(val, field, preset).map_err(|_| format!("`{field}` must be a positive integer"))?;
    if f < 1.0 || (f - f.round()).abs() > 1e-9 || f > (usize::MAX as f64) {
        return Err(format!("`{field}` must be a positive integer"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(f.round() as usize)
}

pub(super) fn parse_open_unit_f64(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let t = parse_finite_f64_with_locale(val, key, preset)?;
    if !(0.0 < t && t < 1.0) {
        return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
    }
    Ok(t)
}

pub(super) fn parse_pos_f64(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let v = parse_finite_f64_with_locale(val, key, preset)?;
    if v <= 0.0 {
        return Err(format!("`{key}` must be > 0"));
    }
    Ok(v)
}

pub(super) fn parse_opt_upper_percentile(val: &Value, key: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    let Some(p) = parse_opt_finite_f64_with_locale(val, key, preset)? else {
        return Ok(None);
    };
    if !(50.0 < p && p < 100.0) {
        return Err(format!("`{key}` ({p}) must lie strictly in (50, 100)"));
    }
    Ok(Some(p))
}

pub(super) fn parse_nonneg_f64(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let f = parse_finite_f64_with_locale(val, key, preset)?;
    if f < 0.0 {
        return Err(format!("`{key}` must be finite and >= 0"));
    }
    Ok(f)
}

pub(super) fn parse_ci_method_choice(val: &Value, key: &str, label: &str, allowed: [&str; 2]) -> Result<String, String> {
    let s = parse_str_field(val, key)?;
    if !allowed.contains(&s) {
        return Err(format!("unsupported {label} CI method `{s}`; expected `{}` or `{}`", allowed[0], allowed[1]));
    }
    Ok(s.to_owned())
}

pub(super) fn parse_usize_list(val: &Value, field: &str) -> Result<Vec<usize>, String> {
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

pub(super) fn parse_f64_list_with_locale(val: &Value, field: &str, preset: &LocalePreset) -> Result<Vec<f64>, String> {
    if let Some(arr) = val.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(parse_finite_f64_with_locale(item, field, preset)?);
        }
        return Ok(out);
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let iter: Box<dyn Iterator<Item = &str>> = if preset.decimal_separator == ',' {
            Box::new(trimmed.split([';', ' ', '\t', '\n', '\r']))
        } else {
            Box::new(trimmed.split([',', ';', ' ', '\t', '\n', '\r']))
        };
        for tok in iter.map(str::trim).filter(|t| !t.is_empty()) {
            let v = parse_localized_number_input(tok, preset)
                .filter(|x| x.is_finite())
                .ok_or_else(|| format!("`{field}` contains invalid finite number `{tok}`"))?;
            out.push(v);
        }
        return Ok(out);
    }
    Err(format!("`{field}` must be a numeric array or delimited list"))
}

pub(super) fn parse_reference_levels(val: &Value) -> Result<Vec<(usize, String)>, String> {
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

pub(super) fn parse_kappa_weighting(val: &Value) -> Result<KappaWeighting, String> {
    let raw = val.as_str().ok_or_else(|| String::from("`kappa_weighting` must be a string"))?;
    match norm_key(raw).as_str() {
        "unweighted" | "none" | "simple" => Ok(KappaWeighting::Unweighted),
        "linear" => Ok(KappaWeighting::Linear),
        "quadratic" | "squared" | "fleiss" => Ok(KappaWeighting::Quadratic),
        _ => Err(format!("unknown kappa weighting `{raw}`")),
    }
}

pub(super) fn parse_two_group_method_str(raw: &str, hypothetical: f64) -> Result<TwoGroupMethod, String> {
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

pub(super) fn parse_df_method_str(raw: &str) -> Result<MixedModelDfMethod, String> {
    match norm_key(raw).as_str() {
        "kenwardroger" | "kr" => Ok(MixedModelDfMethod::KenwardRoger),
        "satterthwaite" | "sat" | "faicornelius" => Ok(MixedModelDfMethod::Satterthwaite),
        _ => Err(format!("unknown REML df method `{raw}`; expected `kenward_roger` or `satterthwaite`")),
    }
}

pub(super) fn parse_one_way_method_str(raw: &str, saved_gg: bool) -> Result<OneWayMethod, String> {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "ordinary" | "ordinary_anova" | "anova" => Ok(OneWayMethod::OrdinaryAnova),
        "welch" | "brown_forsythe" | "brown_forsythe_welch" | "brown_forsythe_and_welch" => Ok(OneWayMethod::BrownForsytheAndWelch),
        "kruskal" | "kruskal_wallis" => Ok(OneWayMethod::KruskalWallis),
        "friedman" => Ok(OneWayMethod::Friedman),
        "rm" | "repeated_measures" => Ok(OneWayMethod::RepeatedMeasures { geisser_greenhouse: saved_gg }),
        _ => Err(format!("unknown one-way ANOVA method `{lower}`")),
    }
}

norm_enum_parser!(parse_power_family_str, PowerTestFamily, "power test family",
    "unpairedt" | "twosampleunpairedt" | "independentt" | "twosamplet" => PowerTestFamily::TwoSampleUnpairedT,
    "pairedt" | "twosamplepairedt" | "dependentt" => PowerTestFamily::TwoSamplePairedT,
    "onesamplet" | "singlesamplet" => PowerTestFamily::OneSampleT,
    "onewayanova" | "anova" => PowerTestFamily::OneWayAnova,
    "twoproportions" | "proportions" => PowerTestFamily::TwoProportions,
    "correlation" | "pearsoncorrelation" => PowerTestFamily::Correlation,
    "chisquare" | "chi2" => PowerTestFamily::ChiSquare,
    "simplelinearregression" | "linearregression" | "regression" | "simplelinear" => PowerTestFamily::SimpleLinearRegression,
);

norm_enum_parser!(parse_power_solve_target_str, PowerSolveTarget, "power solve target",
    "samplesize" | "n" => PowerSolveTarget::SampleSize,
    "power" => PowerSolveTarget::Power,
    "detectableeffect" | "effect" | "mde" => PowerSolveTarget::DetectableEffect,
);

pub(super) fn parse_multiplicity_method_name(raw: &str, control_col: usize) -> Result<MultiplicityMethod, String> {
    match norm_key(raw).as_str() {
        "none" => Ok(MultiplicityMethod::None),
        "tukey" | "tukeyhsd" => Ok(MultiplicityMethod::TukeyHsd),
        "dunnett" => Ok(MultiplicityMethod::Dunnett { control_column: control_col }),
        "sidak" => Ok(MultiplicityMethod::Sidak),
        "bonferroni" => Ok(MultiplicityMethod::Bonferroni),
        "holmsidak" => Ok(MultiplicityMethod::HolmSidak),
        "dunnettt3" => Ok(MultiplicityMethod::DunnettT3),
        "gameshowell" => Ok(MultiplicityMethod::GamesHowell),
        "tamhanet2" | "tamhane" => Ok(MultiplicityMethod::TamhaneT2),
        "dunn" | "dunnranksum" => Ok(MultiplicityMethod::DunnRankSum),
        "lineartrend" => Ok(MultiplicityMethod::LinearTrend),
        "benjaminihochbergfdr" | "bh" | "fdr" => Ok(MultiplicityMethod::BenjaminiHochbergFdr),
        "benjaminiyekutielifdr" | "by" => Ok(MultiplicityMethod::BenjaminiYekutieliFdr),
        "twostagebkyfdr" | "bky" => Ok(MultiplicityMethod::TwoStageBkyFdr),
        "newmankeuls" | "snk" => Ok(MultiplicityMethod::NewmanKeuls),
        "fisherlsd" | "lsd" => Ok(MultiplicityMethod::FisherLsd),
        _ => Err(format!("unknown multiplicity method `{raw}`")),
    }
}

pub(super) fn post_hoc_method_for_widget_key(key: &str, control_col: usize) -> Option<MultiplicityMethod> {
    let suffix = key.strip_prefix("edit.post_hoc.").or_else(|| key.strip_prefix("edit.posthoc."))?;
    parse_multiplicity_method_name(suffix, control_col).ok()
}

pub(super) fn parse_contingency_method_name(raw: &str) -> Result<ContingencyMethod, String> {
    match norm_key(raw).as_str() {
        "chisquare" | "pearsonchisquare" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        "yates" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
        "likelihoodratiog2" | "g2" | "likelihoodratio" => Ok(ContingencyMethod::LikelihoodRatioG2),
        "fisher" | "fisherexact" => Ok(ContingencyMethod::FisherExact),
        "barnard" | "barnardexact" => Ok(ContingencyMethod::BarnardExact),
        "cochranarmitage" | "cochranarmitagetrend" | "trend" => Ok(ContingencyMethod::CochranArmitageTrend),
        "mcnemar" | "mcnemarpaired" => Ok(ContingencyMethod::McNemarPaired),
        "cohenkappa" | "kappa" | "unweightedkappa" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted }),
        "linearkappa" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear }),
        "quadratickappa" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }),
        "fleisskappa" => Ok(ContingencyMethod::FleissKappa),
        "exactbinomial" | "binomial" => Ok(ContingencyMethod::ExactBinomial),
        _ => Err(format!("unknown contingency test method `{raw}`")),
    }
}

pub(super) fn parse_table_kind_str(raw: &str) -> Result<TableKind, String> {
    match norm_key(raw).as_str() {
        "column" | "columnar" => Ok(TableKind::Column),
        "grouped" => Ok(TableKind::Grouped),
        "xy" => Ok(TableKind::Xy),
        "multiplevariables" | "multivariate" | "mv" => Ok(TableKind::MultipleVariables),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "partsofwhole" | "parts" => Ok(TableKind::PartsOfWhole),
        "nested" => Ok(TableKind::Nested),
        _ => Err(format!("unknown table archetype `{raw}`")),
    }
}

pub(super) fn parse_subcolumn_aggregation_val(val: &Value) -> Result<SubcolumnAggregation, String> {
    if let Some(idx) = val.as_u64() {
        return match idx {
            0 => Ok(SubcolumnAggregation::RowMeans),
            1 => Ok(SubcolumnAggregation::PerSubcolumn),
            2 => Ok(SubcolumnAggregation::PooledGrandSample),
            _ => Err(format!("unknown subcolumn aggregation index `{idx}`")),
        };
    }
    let raw = val.as_str().ok_or_else(|| String::from("`subcolumn_agg` must be a string or index"))?;
    match norm_key(raw).as_str() {
        "rowmeans" | "mean" | "means" => Ok(SubcolumnAggregation::RowMeans),
        "persubcolumn" | "eachreplicate" | "subcolumns" => Ok(SubcolumnAggregation::PerSubcolumn),
        "pooledgrandsample" | "pooled" => Ok(SubcolumnAggregation::PooledGrandSample),
        _ => Err(format!("unknown subcolumn aggregation `{raw}`")),
    }
}

norm_enum_parser!(parse_baseline_operation_str, BaselineOperation, "baseline operation",
    "difference" | "subtract" => BaselineOperation::Difference,
    "ratio" | "divide" => BaselineOperation::Ratio,
    "percentchange" | "pctchange" | "percent" => BaselineOperation::PercentChange,
    "fractionalchange" | "fraction" => BaselineOperation::FractionalChange,
);

norm_enum_parser!(parse_grouped_arrangement_str, GroupedArrangement, "grouped_arrangement",
    "interleaved" | "sidebyside" => GroupedArrangement::Interleaved,
    "stacked" => GroupedArrangement::Stacked,
    "superimposed" | "overlaid" | "overlay" => GroupedArrangement::Superimposed,
);

norm_enum_parser!(parse_graph_kind_str, GraphKind, "graph_kind",
    "xy" | "scatter" | "scatterline" => GraphKind::Xy,
    "column" | "col" => GraphKind::Column,
    "grouped" => GraphKind::Grouped,
    "contingency" => GraphKind::Contingency,
    "partsofwhole" | "parts" | "donut" | "pie" => GraphKind::PartsOfWhole,
    "survival" | "km" | "kaplanmeier" => GraphKind::Survival,
    "multiplevariables" | "multivariate" | "bubble" => GraphKind::MultipleVariables,
);

norm_enum_parser!(parse_palette_str, PaletteId, "palette",
    "okabeito" | "default" => PaletteId::OkabeIto,
    "paultolbright" | "bright" => PaletteId::PaulTolBright,
    "paultolmuted" | "muted" => PaletteId::PaulTolMuted,
    "paultolhighcontrast" | "highcontrast" => PaletteId::PaulTolHighContrast,
    "paultolvibrant" | "vibrant" => PaletteId::PaulTolVibrant,
    "monochromeprint" | "monochrome" | "grayscale" | "greyscale" => PaletteId::MonochromePrint,
);

norm_enum_parser!(parse_frame_preset_str, FramePreset, "frame_preset",
    "openl" | "lframe" => FramePreset::OpenL,
    "offsetrangetufte" | "offset" | "tufte" => FramePreset::OffsetRangeTufte,
    "plainboxed" | "boxed" | "plain" => FramePreset::PlainBoxed,
    "dualysecondaryright" | "dualy" => FramePreset::DualYSecondaryRight,
    "origincrossing" | "origin" => FramePreset::OriginCrossing,
);

pub(super) fn parse_math_preset_opt(val: &Value) -> Result<Option<MathTransformPreset>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(raw) = val.as_str() {
        return parse_math_preset_str(raw);
    }
    if val.is_object() {
        let preset: MathTransformPreset = serde_json::from_value(val.clone()).map_err(|e| format!("invalid math transform preset: {e}"))?;
        return Ok(Some(preset));
    }
    Err(String::from("math transform preset must be a string, preset object, or null"))
}

fn parse_math_preset_str(raw: &str) -> Result<Option<MathTransformPreset>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    if matches!(compact.as_str(), "none" | "identity" | "null" | "x" | "y") {
        return Ok(None);
    }
    let norm = norm_key(trimmed);
    let preset = match (compact.as_str(), norm.as_str()) {
        (_, "log10" | "log10x" | "log10y" | "ylog10y" | "ylog10x" | "xlog10x") => MathTransformPreset::Log10,
        (_, "ln" | "lnx" | "lny" | "loge" | "ylny" | "ylnx" | "xlnx") => MathTransformPreset::Ln,
        (_, "log2" | "log2x" | "log2y" | "ylog2y" | "ylog2x" | "xlog2x") => MathTransformPreset::Log2,
        ("10^x" | "10^y" | "10**x" | "10**y", _) | (_, "exp10" | "exp10x" | "exp10y" | "10powx" | "10powy") => MathTransformPreset::Exp10,
        ("e^x" | "e^y", _) | (_, "exp" | "expx" | "expy" | "epowx" | "epowy") => MathTransformPreset::Exp,
        ("1/x" | "1/y", _) | (_, "reciprocal" | "reciprocalx" | "reciprocaly") => MathTransformPreset::Reciprocal,
        (_, "sqrt" | "sqrtx" | "sqrty") => MathTransformPreset::Sqrt,
        ("x^2" | "y^2" | "x*x" | "y*y" | "x**2" | "y**2", _) | (_, "square" | "squarex" | "squarey") => MathTransformPreset::Square,
        (_, "logit" | "logitx" | "logity") => MathTransformPreset::Logit,
        (_, "probit" | "probitx" | "probity") => MathTransformPreset::Probit,
        (_, "arcsinsqrt" | "arcsinsqrtx" | "arcsinsqrty") => MathTransformPreset::ArcsinSqrt,
        _ => return Err(format!("unknown math transform preset `{raw}`")),
    };
    Ok(Some(preset))
}

pub(super) fn parse_error_prop_value(val: &Value) -> Result<ErrorPropagationMode, String> {
    let raw = parse_str_field(val, "error_propagation")?;
    match norm_key(raw).as_str() {
        "asymmetric" | "replicateendpoints" | "endpoints" => Ok(ErrorPropagationMode::ReplicateEndpoints),
        "delta" | "deltamethod" | "firstorderdelta" | "firstorderdeltamethod" => Ok(ErrorPropagationMode::FirstOrderDeltaMethod),
        _ => Err(format!("unknown error propagation mode `{raw}`")),
    }
}

pub(super) fn apply_transform_formula_str(
    raw: &str,
    x_slot: &mut Option<MathTransformPreset>,
    y_slot: &mut Option<MathTransformPreset>,
) -> Result<(), String> {
    for part in raw.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((lhs, rhs)) = part.split_once('=') {
            let axis = lhs.trim().to_ascii_lowercase();
            let parsed = parse_math_preset_str(rhs)?;
            match axis.as_str() {
                "x" | "x_transform" => *x_slot = parsed,
                "y" | "y_transform" => *y_slot = parsed,
                _ => return Err(format!("unsupported transform formula target `{lhs}`")),
            }
        } else {
            *y_slot = parse_math_preset_str(part)?;
        }
    }
    Ok(())
}

norm_enum_parser!(parse_cluster_distance_str, ClusterDistance, "cluster distance",
    "euclidean" | "l2" => ClusterDistance::Euclidean,
    "manhattan" | "cityblock" | "l1" => ClusterDistance::Manhattan,
    "pearson" | "correlation" => ClusterDistance::Pearson,
    "cosine" => ClusterDistance::Cosine,
);

norm_enum_parser!(parse_cluster_linkage_str, ClusterLinkage, "cluster linkage",
    "ward" | "wardd2" => ClusterLinkage::Ward,
    "complete" => ClusterLinkage::Complete,
    "single" => ClusterLinkage::Single,
    "upgma" | "average" => ClusterLinkage::Upgma,
    "wpgma" | "mcquitty" => ClusterLinkage::Wpgma,
    "centroid" => ClusterLinkage::Centroid,
    "median" => ClusterLinkage::Median,
);

norm_enum_parser!(parse_glm_family_str, GlmFamily, "GLM family",
    "linear" | "gaussian" | "ols" => GlmFamily::Linear,
    "logistic" | "binomial" | "logit" => GlmFamily::Logistic,
    "poisson" | "loglinear" => GlmFamily::Poisson,
);

norm_enum_parser!(parse_cox_tie_method_str, CoxTieMethod, "Cox tie method",
    "breslow" => CoxTieMethod::Breslow,
    "efron" => CoxTieMethod::Efron,
    "exact" => CoxTieMethod::Exact,
);

norm_enum_parser!(parse_ss_type_str, SumOfSquaresType, "sum-of-squares type",
    "typei" | "type1" | "i" | "1" | "sequential" => SumOfSquaresType::TypeI,
    "typeiii" | "type3" | "iii" | "3" => SumOfSquaresType::TypeIII,
);

norm_enum_parser!(parse_rm_design_str, RepeatedMeasuresDesign, "repeated measures design",
    "none" | "nomatching" => RepeatedMeasuresDesign::None,
    "rowfactormatched" | "row" | "rowfactor" => RepeatedMeasuresDesign::RowFactorMatched,
    "columnfactormatched" | "column" | "col" | "columnfactor" => RepeatedMeasuresDesign::ColumnFactorMatched,
    "bothfactorsmatched" | "both" => RepeatedMeasuresDesign::BothFactorsMatched,
);

norm_enum_parser!(parse_residual_covariance_str, ResidualCovariance, "residual covariance structure",
    "compoundsymmetry" | "cs" | "randomintercept" => ResidualCovariance::CompoundSymmetry,
    "unstructured" | "un" | "general" => ResidualCovariance::Unstructured,
);

pub(super) fn parse_weighting_scheme(val: &Value) -> Result<WeightingScheme, String> {
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

pub(super) fn parse_outlier_fdr_q_percent(val: &Value, preset: &LocalePreset) -> Result<f64, String> {
    let q = parse_finite_f64_with_locale(val, "fdr_q_percent", preset)?;
    if q <= 0.0 || q > 100.0 {
        return Err(format!("`fdr_q_percent` must lie in (0, 100], got {q}"));
    }
    Ok(q)
}

pub(super) fn parse_outlier_alpha(val: &Value, preset: &LocalePreset) -> Result<f64, String> {
    let a = parse_finite_f64_with_locale(val, "alpha", preset)?;
    if a <= 0.0 || a >= 1.0 {
        return Err(format!("`alpha` must lie strictly in (0, 1), got {a}"));
    }
    Ok(a)
}

pub(super) fn parse_outlier_method_str(raw: &str) -> Result<(&'static str, Option<bool>), String> {
    match norm_key(raw).as_str() {
        "rout" | "routq" | "fdrrout" => Ok(("rout", None)),
        "grubbs" | "grubbss" | "grubbsesd" => Ok(("grubbs", None)),
        "grubbssingle" | "singlegrubbs" => Ok(("grubbs", Some(false))),
        "grubbsiterative" | "iterativegrubbs" => Ok(("grubbs", Some(true))),
        _ => Err(format!("unknown outlier method `{raw}`; use `rout` or `grubbs`")),
    }
}

fn parse_rout_inline_q(raw: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("rout") {
        return Ok(None);
    }
    let Some(after_rout) = trimmed.get(4..) else {
        return Ok(None);
    };
    let rest = after_rout.trim();
    if rest.is_empty() {
        return Ok(None);
    }
    let cleaned = rest.trim_start_matches([':', '@', '(', '=', ',', ' ', '_']).trim_end_matches([')', '%', ' ']).trim();
    let after_q = cleaned
        .strip_prefix("fdr_q_percent")
        .or_else(|| cleaned.strip_prefix("fdr_q"))
        .or_else(|| cleaned.strip_prefix("q_percent"))
        .or_else(|| cleaned.strip_prefix("rout_q"))
        .or_else(|| cleaned.strip_prefix('q'))
        .or_else(|| cleaned.strip_prefix('Q'))
        .map(|s| s.trim_start_matches(['=', ':', ' ']))
        .unwrap_or(cleaned)
        .trim_end_matches('%')
        .trim();
    if after_q.is_empty() || !after_q.chars().any(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    let q: f64 = after_q
        .parse()
        .ok()
        .or_else(|| parse_localized_number_input(after_q, preset))
        .ok_or_else(|| format!("invalid ROUT Q% in fit method `{raw}`"))?;
    if !q.is_finite() || q <= 0.0 || q > 100.0 {
        return Err(format!("ROUT `fdr_q_percent` must lie in (0, 100], got {q}"));
    }
    Ok(Some(q))
}

pub(super) fn parse_fit_method(val: &Value, preset: &LocalePreset) -> Result<FitMethod, String> {
    if let Ok(direct) = serde_json::from_value::<FitMethod>(val.clone()) {
        if let FitMethod::RoutOutlierElimination { fdr_q_percent } = direct
            && (!fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 100.0)
        {
            return Err(format!("`fdr_q_percent` must lie in (0, 100], got {fdr_q_percent}"));
        }
        return Ok(direct);
    }
    if let Some(obj) = val.as_object() {
        if let Some(rout_val) = obj.get("rout").or_else(|| obj.get("RoutOutlierElimination")) {
            let q = match rout_val {
                Value::Object(inner) => inner
                    .get("fdr_q_percent")
                    .or_else(|| inner.get("q_percent"))
                    .or_else(|| inner.get("rout_q"))
                    .or_else(|| inner.get("q"))
                    .map(|v| parse_outlier_fdr_q_percent(v, preset))
                    .transpose()?
                    .unwrap_or(1.0),
                other => parse_outlier_fdr_q_percent(other, preset)?,
            };
            return Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: q });
        }
        if let Some(method_raw) = obj.get("method").or_else(|| obj.get("type")).and_then(Value::as_str) {
            let parsed = parse_fit_method(&Value::String(method_raw.to_owned()), preset)?;
            if let FitMethod::RoutOutlierElimination { fdr_q_percent } = parsed {
                let q = obj
                    .get("fdr_q_percent")
                    .or_else(|| obj.get("q_percent"))
                    .or_else(|| obj.get("rout_q"))
                    .or_else(|| obj.get("q"))
                    .map(|v| parse_outlier_fdr_q_percent(v, preset))
                    .transpose()?
                    .unwrap_or(fdr_q_percent);
                return Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: q });
            }
            return Ok(parsed);
        }
        return Err(String::from("`fit_method` object must specify `method` or `RoutOutlierElimination`"));
    }
    let raw = val.as_str().ok_or_else(|| String::from("`fit_method` must be a string or FitMethod object"))?;
    if let Some(q) = parse_rout_inline_q(raw, preset)? {
        return Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: q });
    }
    match norm_key(raw).as_str() {
        "leastsquares" | "ols" | "ls" => Ok(FitMethod::LeastSquares),
        "poissondeviance" | "poisson" => Ok(FitMethod::PoissonDeviance),
        "robustcauchy" | "robust" | "cauchy" => Ok(FitMethod::RobustCauchy),
        "rout" | "routoutlierelimination" | "routoutlier" => Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }),
        _ => Err(format!("unknown fit method `{raw}`")),
    }
}

pub(super) fn parse_parameter_rules(val: &Value) -> Result<Vec<ParameterRule>, String> {
    serde_json::from_value::<Vec<ParameterRule>>(val.clone()).map_err(|e| format!("invalid `parameter_rules`: {e}"))
}

pub(super) fn parse_equation_display_mode_str(raw: &str) -> Result<EquationDisplayMode, String> {
    match norm_key(raw).as_str() {
        "substituted" | "values" | "numeric" | "evaluated" => Ok(EquationDisplayMode::Substituted),
        "symbolic" | "formula" | "parameters" => Ok(EquationDisplayMode::Symbolic),
        _ => Err(format!("unknown equation display mode `{raw}`")),
    }
}

pub(super) fn parse_axis_scale_str(raw: &str) -> Result<GraphAxisScale, String> {
    match norm_key(raw).as_str() {
        "linear" => Ok(GraphAxisScale::Linear),
        "log10" => Ok(GraphAxisScale::Log10),
        "log2" => Ok(GraphAxisScale::Log2),
        "ln" | "loge" => Ok(GraphAxisScale::Ln),
        "logit" | "logitproportion" => Ok(GraphAxisScale::Logit(curvebook_doc::LogitDomain::Proportion)),
        "logitpercent" => Ok(GraphAxisScale::Logit(curvebook_doc::LogitDomain::Percent)),
        "probit" => Ok(GraphAxisScale::Probit),
        "reciprocal" | "1x" => Ok(GraphAxisScale::Reciprocal),
        _ => Err(format!("unknown axis scale `{raw}`")),
    }
}

pub(super) fn parse_pvalue_stack_method_val(val: &Value) -> Result<PValueStackMethod, String> {
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

pub(super) fn parse_break_ornament_str(raw: &str) -> Result<BreakOrnament, String> {
    match norm_key(raw).as_str() {
        "doubleslash" | "slash" => Ok(BreakOrnament::DoubleSlash),
        "zigzag" => Ok(BreakOrnament::Zigzag),
        "blankgap" | "blank" | "gap" => Ok(BreakOrnament::BlankGap),
        _ => Err(format!("unknown break_ornament `{raw}`")),
    }
}

pub(super) fn parse_tick_direction_str(raw: &str) -> Result<TickDirection, String> {
    match norm_key(raw).as_str() {
        "outside" => Ok(TickDirection::Outside),
        "inside" => Ok(TickDirection::Inside),
        "both" => Ok(TickDirection::Both),
        "none" | "off" => Ok(TickDirection::None),
        other => Err(format!("unknown `tick_direction` `{other}`")),
    }
}

pub(super) fn parse_bracket_style_str(raw: &str) -> Result<BracketStyle, String> {
    match norm_key(raw).as_str() {
        "square" => Ok(BracketStyle::Square),
        "halftick" => Ok(BracketStyle::HalfTick),
        "flatline" | "flat" => Ok(BracketStyle::FlatLine),
        "curlybrace" | "curly" => Ok(BracketStyle::CurlyBrace),
        _ => Err(format!("unknown bracket_style `{raw}`")),
    }
}

pub(super) fn parse_bracket_label_mode(val: &Value) -> Result<BracketLabelMode, String> {
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

norm_enum_parser!(parse_pairwise_display_mode_str, PairwiseDisplayMode, "pairwise display mode",
    "bracketsonly" | "brackets" => PairwiseDisplayMode::BracketsOnly,
    "compactletterdisplayonly" | "cldonly" | "cld" | "lettersonly" | "letters" => PairwiseDisplayMode::CompactLetterDisplayOnly,
    "bracketsandcompactletterdisplay" | "bracketsandcld" | "both" => PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
);

norm_enum_parser!(parse_cld_ordering_str, CldOrdering, "CLD ordering",
    "descendingmean" | "descending" | "desc" | "highestfirst" => CldOrdering::DescendingMean,
    "ascendingmean" | "ascending" | "asc" | "lowestfirst" => CldOrdering::AscendingMean,
);

norm_enum_parser!(parse_colormap_id_str, ColormapId, "colormap",
    "viridis" => ColormapId::Viridis,
    "cividis" => ColormapId::Cividis,
    "plasma" => ColormapId::Plasma,
    "divergingbluered" | "bluered" => ColormapId::DivergingBlueRed,
    "grayscale" | "gray" => ColormapId::Grayscale,
);

pub(super) fn parse_journal_preset_str(raw: &str) -> Result<JournalWidthPreset, String> {
    JournalWidthPreset::from_canonical_name(&norm_key(raw)).ok_or_else(|| format!("unknown journal preset `{raw}`"))
}

norm_enum_parser!(parse_page_orientation_str, PageOrientation, "page orientation",
    "portrait" => PageOrientation::Portrait,
    "landscape" => PageOrientation::Landscape,
);

norm_enum_parser!(parse_panel_badge_style_str, PanelBadgeStyle, "panel badge style",
    "uppercaseletters" | "uppercase" | "abc" => PanelBadgeStyle::UpperCaseLetters,
    "lowercaseletters" | "lowercase" => PanelBadgeStyle::LowerCaseLetters,
    "none" => PanelBadgeStyle::None,
);

pub(super) fn parse_export_format(val: &Value) -> Result<ExportFigureFormat, String> {
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

pub(super) fn parse_axis_thresholds_value(val: &Value, key: &str, preset: &LocalePreset) -> Result<Vec<AxisThreshold>, String> {
    match val {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => arr.iter().map(|item| parse_single_axis_threshold(item, key, preset)).collect(),
        _ => Ok(vec![parse_single_axis_threshold(val, key, preset)?]),
    }
}

fn parse_single_axis_threshold(val: &Value, key: &str, preset: &LocalePreset) -> Result<AxisThreshold, String> {
    if val.is_number() || val.is_string() {
        let num = parse_finite_f64_with_locale(val, key, preset)?;
        return Ok(AxisThreshold {
            value: num,
            shade_to: None,
            color: Rgba8::rgb(120, 120, 120),
            dash_style: LineDashStyle::Dashed,
            stroke_width_pt: 1.0,
            label: None,
        });
    }
    if let Ok(mut parsed) = serde_json::from_value::<AxisThreshold>(val.clone()) {
        if !parsed.value.is_finite() || parsed.shade_to.is_some_and(|v| !v.is_finite()) {
            return Err(format!("`{key}` threshold bounds must be finite"));
        }
        parsed.stroke_width_pt = parsed.stroke_width_pt.clamp(0.2, 8.0);
        return Ok(parsed);
    }
    let obj = val.as_object().ok_or_else(|| format!("`{key}` entries must be objects or numbers"))?;
    let val_num = obj.get("value").ok_or_else(|| format!("`{key}` threshold missing `value`"))?;
    let value = parse_finite_f64_with_locale(val_num, key, preset)?;
    let shade_to = obj.get("shade_to").map(|v| parse_opt_finite_f64_with_locale(v, "shade_to", preset)).transpose()?.flatten();
    let label = match obj.get("label") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| format!("`{key}` `label` must be a string"))?.to_owned()),
    };
    let color = match obj.get("color") {
        None | Some(Value::Null) => Rgba8::rgb(120, 120, 120),
        Some(v) => parse_threshold_color(v, key)?,
    };
    let dash_style = match obj.get("dash_style") {
        None | Some(Value::Null) => LineDashStyle::Dashed,
        Some(v) => parse_threshold_dash_style(v, key)?,
    };
    let stroke_width_pt = match obj.get("stroke_width_pt") {
        None | Some(Value::Null) => 1.0,
        Some(v) => parse_finite_f64_with_locale(v, "stroke_width_pt", preset)?.clamp(0.2, 8.0),
    };
    Ok(AxisThreshold { value, shade_to, color, dash_style, stroke_width_pt, label })
}

pub(super) fn parse_threshold_color(val: &Value, key: &str) -> Result<Rgba8, String> {
    if let Ok(c) = serde_json::from_value::<Rgba8>(val.clone()) {
        return Ok(c);
    }
    if let Some(hex) = val.as_str() {
        let s = hex.trim().trim_start_matches('#');
        if s.is_ascii() && (s.len() == 6 || s.len() == 8) {
            let err = || format!("invalid `{key}` hex color `{hex}`");
            let r = u8::from_str_radix(s.get(0..2).ok_or_else(err)?, 16).map_err(|_| err())?;
            let g = u8::from_str_radix(s.get(2..4).ok_or_else(err)?, 16).map_err(|_| err())?;
            let b = u8::from_str_radix(s.get(4..6).ok_or_else(err)?, 16).map_err(|_| err())?;
            let a = if s.len() == 8 { u8::from_str_radix(s.get(6..8).ok_or_else(err)?, 16).map_err(|_| err())? } else { 255 };
            return Ok(Rgba8::rgba(r, g, b, a));
        }
    }
    Err(format!("invalid `{key}` color value"))
}

pub(super) fn parse_threshold_dash_style(val: &Value, key: &str) -> Result<LineDashStyle, String> {
    if let Ok(d) = serde_json::from_value::<LineDashStyle>(val.clone()) {
        return Ok(d);
    }
    if let Some(raw) = val.as_str() {
        return match norm_key(raw).as_str() {
            "solid" => Ok(LineDashStyle::Solid),
            "dashed" | "dash" => Ok(LineDashStyle::Dashed),
            "dotted" | "dot" => Ok(LineDashStyle::Dotted),
            "dashdot" | "dash_dot" => Ok(LineDashStyle::DashDot),
            _ => Err(format!("unknown `{key}` dash_style `{raw}`")),
        };
    }
    Err(format!("`{key}` dash_style must be a string"))
}

pub(super) fn parse_paper_fill_patch(val: &Value) -> Result<Option<Rgba8>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "none" | "transparent" => Ok(None),
            "white" => Ok(Some(Rgba8::WHITE)),
            "cream" => Ok(Some(Rgba8::rgb(255, 251, 235))),
            "slate" => Ok(Some(Rgba8::rgb(241, 245, 249))),
            other => Rgba8::from_hex(other).map(Some).ok_or_else(|| format!("invalid paper fill `{other}`")),
        };
    }
    serde_json::from_value::<Rgba8>(val.clone()).map(Some).map_err(|err| format!("invalid `paper_fill`: {err}"))
}

pub(super) fn parse_summary_measure_str(raw: &str) -> Result<SummaryMeasure, String> {
    match norm_key(raw).as_str() {
        "mean" => Ok(SummaryMeasure::Mean),
        "median" => Ok(SummaryMeasure::Median),
        "geometricmean" | "geomean" | "geometric" => Ok(SummaryMeasure::GeometricMean),
        _ => Err(format!("unsupported summary_measure `{raw}`")),
    }
}

pub(super) fn parse_error_bar_metric_str(raw: &str) -> Result<ErrorBarMetric, String> {
    match norm_key(raw).as_str() {
        "sd" => Ok(ErrorBarMetric::Sd),
        "sem" => Ok(ErrorBarMetric::Sem),
        "ci95" | "confidenceinterval95" | "95ci" => Ok(ErrorBarMetric::ConfidenceInterval95),
        "iqr" | "interquartilerange" => Ok(ErrorBarMetric::InterquartileRange),
        "range" | "minmax" => Ok(ErrorBarMetric::Range),
        "explicitsubcolumns" | "explicit" => Ok(ErrorBarMetric::ExplicitSubcolumns),
        _ => Err(format!("unsupported error_bar_metric `{raw}`")),
    }
}

pub(super) fn parse_survival_metric_str(raw: &str) -> Result<SurvivalDisplayMetric, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "percent_survival" | "percentsurvival" | "survival" => Ok(SurvivalDisplayMetric::PercentSurvival),
        "percent_cumulative_event" | "percentcumulativeevent" | "event" => Ok(SurvivalDisplayMetric::PercentCumulativeEvent),
        "fraction_survival" | "fractionsurvival" | "fraction" => Ok(SurvivalDisplayMetric::FractionSurvival),
        _ => Err(format!("unknown `survival_metric` `{raw}`")),
    }
}

pub(super) fn parse_opt_nonempty_string(val: &Value, field: &str) -> Result<Option<String>, String> {
    if val.is_null() {
        return Ok(None);
    }
    let s = val.as_str().ok_or_else(|| format!("`{field}` must be a string or null"))?.trim();
    Ok((!s.is_empty()).then(|| s.to_owned()))
}

fn parse_plate_cell_token(tok: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    let t = tok.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("na") || t.eq_ignore_ascii_case("null") || t == "-" {
        return Ok(None);
    }
    parse_localized_number_input(t, preset).filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("invalid plate numeric token `{t}`"))
}

pub(super) fn parse_plate_values_text(raw: &str, preset: &LocalePreset) -> Result<Vec<Option<f64>>, String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains('\t') || trimmed.contains(';') || (preset.decimal_separator != ',' && trimmed.contains(',')) {
            let split_iter: Box<dyn Iterator<Item = &str>> =
                if preset.decimal_separator == ',' { Box::new(trimmed.split(['\t', ';'])) } else { Box::new(trimmed.split([',', '\t', ';'])) };
            for cell in split_iter {
                out.push(parse_plate_cell_token(cell, preset)?);
            }
        } else {
            for cell in trimmed.split_whitespace() {
                out.push(parse_plate_cell_token(cell, preset)?);
            }
        }
    }
    Ok(out)
}

pub(super) fn parse_plate_values_json(val: &Value, preset: &LocalePreset) -> Result<Vec<Option<f64>>, String> {
    if let Some(s) = val.as_str() {
        return parse_plate_values_text(s, preset);
    }
    let arr = val.as_array().ok_or_else(|| String::from("`plate_values` must be an array or delimited string"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(inner) = item.as_array() {
            for sub in inner {
                out.push(parse_plate_scalar(sub, preset)?);
            }
        } else {
            out.push(parse_plate_scalar(item, preset)?);
        }
    }
    Ok(out)
}

fn parse_plate_scalar(val: &Value, preset: &LocalePreset) -> Result<Option<f64>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(f) = val.as_f64() {
        return f.is_finite().then_some(Some(f)).ok_or_else(|| String::from("plate well value must be finite"));
    }
    if let Some(s) = val.as_str() {
        return parse_plate_cell_token(s, preset);
    }
    Err(String::from("plate well value must be a number, string, or null"))
}
