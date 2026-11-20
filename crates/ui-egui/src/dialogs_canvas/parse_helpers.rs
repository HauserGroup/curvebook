//! Shared value/string parsers used across analysis, graph, and print dialog state machines.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, ContingencyMethod, ErrorPropagationMode, FramePreset, GraphKind, GroupedArrangement,
    MathTransformPreset, MixedModelDfMethod, MultiplicityMethod, OneWayMethod, RepeatedMeasuresDesign, ResidualCovariance, SubcolumnAggregation,
    TableKind,
};
use curvebook_palette::PaletteId;

use serde_json::Value;

use crate::dialogs_r9::{norm_key, parse_finite_f64_with_locale};
use crate::i18n::LocalePreset;

pub(super) fn draft_forces_reml_mode(spec: &AnalysisSpec) -> bool {
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            matches!(a.method, OneWayMethod::RepeatedMeasures { .. })
                && (a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured)
        }
        AnalysisKind::FactorialAnova(f) => {
            f.repeated_measures != RepeatedMeasuresDesign::None
                && (f.mixed_effects_reml || f.residual_covariance == ResidualCovariance::Unstructured || f.three_way)
        }
        _ => false,
    }
}

pub(super) fn parse_df_method_str(raw: &str) -> Result<MixedModelDfMethod, String> {
    let norm = norm_key(raw);
    match norm.as_str() {
        "kenwardroger" | "kr" => Ok(MixedModelDfMethod::KenwardRoger),
        "satterthwaite" | "sat" | "faicornelius" => Ok(MixedModelDfMethod::Satterthwaite),
        _ => Err(format!("unknown REML df method `{raw}`; expected `kenward_roger` or `satterthwaite`")),
    }
}

pub(super) fn parse_multiplicity_method_name(raw: &str, control_col: usize) -> Result<MultiplicityMethod, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
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
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "chisquare" | "pearsonchisquare" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        "yates" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
        "likelihoodratiog2" | "g2" | "likelihoodratio" => Ok(ContingencyMethod::LikelihoodRatioG2),
        "fisher" | "fisherexact" => Ok(ContingencyMethod::FisherExact),
        "barnard" | "barnardexact" => Ok(ContingencyMethod::BarnardExact),
        "cochranarmitage" | "cochranarmitagetrend" | "trend" => Ok(ContingencyMethod::CochranArmitageTrend),
        "mcnemar" | "mcnemarpaired" => Ok(ContingencyMethod::McNemarPaired),
        "cohenkappa" | "kappa" | "unweightedkappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Unweighted }),
        "linearkappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Linear }),
        "quadratickappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Quadratic }),
        "fleisskappa" => Ok(ContingencyMethod::FleissKappa),
        "exactbinomial" | "binomial" => Ok(ContingencyMethod::ExactBinomial),
        _ => Err(format!("unknown contingency test method `{raw}`")),
    }
}

pub(super) fn parse_dialog_num_with_locale(val: &Value, field: &str, preset: &LocalePreset) -> Result<f64, String> {
    parse_finite_f64_with_locale(val, field, preset)
}

pub(crate) fn parse_table_kind_str(raw: &str) -> Result<TableKind, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
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

pub(super) fn parse_baseline_operation_str(raw: &str) -> Result<BaselineOperation, String> {
    let norm = norm_key(raw);
    match norm.as_str() {
        "difference" | "subtract" => Ok(BaselineOperation::Difference),
        "ratio" | "divide" => Ok(BaselineOperation::Ratio),
        "percentchange" | "pctchange" | "percent" => Ok(BaselineOperation::PercentChange),
        "fractionalchange" | "fraction" => Ok(BaselineOperation::FractionalChange),
        _ => Err(format!("unknown baseline operation `{raw}`")),
    }
}

pub(super) fn parse_grouped_arrangement_str(raw: &str) -> Result<GroupedArrangement, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "interleaved" | "sidebyside" => Ok(GroupedArrangement::Interleaved),
        "stacked" => Ok(GroupedArrangement::Stacked),
        "superimposed" | "overlaid" | "overlay" => Ok(GroupedArrangement::Superimposed),
        _ => Err(format!("unknown grouped_arrangement `{raw}`")),
    }
}

pub(super) fn parse_graph_kind_str(raw: &str) -> Result<GraphKind, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "xy" | "scatter" | "scatterline" => Ok(GraphKind::Xy),
        "column" | "col" => Ok(GraphKind::Column),
        "grouped" => Ok(GraphKind::Grouped),
        "contingency" => Ok(GraphKind::Contingency),
        "partsofwhole" | "parts" | "donut" | "pie" => Ok(GraphKind::PartsOfWhole),
        "survival" | "km" | "kaplanmeier" => Ok(GraphKind::Survival),
        "multiplevariables" | "multivariate" | "bubble" => Ok(GraphKind::MultipleVariables),
        _ => Err(format!("unknown graph_kind `{raw}`")),
    }
}

pub(super) fn parse_palette_id_str(raw: &str) -> Result<PaletteId, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "okabeito" | "default" => Ok(PaletteId::OkabeIto),
        "paultolbright" | "bright" => Ok(PaletteId::PaulTolBright),
        "paultolmuted" | "muted" => Ok(PaletteId::PaulTolMuted),
        "paultolhighcontrast" | "highcontrast" => Ok(PaletteId::PaulTolHighContrast),
        "paultolvibrant" | "vibrant" => Ok(PaletteId::PaulTolVibrant),
        "monochromeprint" | "monochrome" | "grayscale" | "greyscale" => Ok(PaletteId::MonochromePrint),
        _ => Err(format!("unknown palette `{raw}`")),
    }
}

pub(super) fn parse_frame_preset_str(raw: &str) -> Result<FramePreset, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "openl" | "lframe" => Ok(FramePreset::OpenL),
        "offsetrangetufte" | "offset" | "tufte" => Ok(FramePreset::OffsetRangeTufte),
        "plainboxed" | "boxed" | "plain" => Ok(FramePreset::PlainBoxed),
        "dualysecondaryright" | "dualy" => Ok(FramePreset::DualYSecondaryRight),
        "origincrossing" | "origin" => Ok(FramePreset::OriginCrossing),
        _ => Err(format!("unknown frame_preset `{raw}`")),
    }
}

pub(crate) fn parse_math_preset_opt(val: &Value) -> Result<Option<MathTransformPreset>, String> {
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
    let norm: String = trimmed.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
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
    let raw = val.as_str().ok_or_else(|| String::from("`error_propagation` must be a string"))?;
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
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
