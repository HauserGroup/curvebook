//! Shared value/string parsers and state helpers used across analysis, graph, and print dialog state machines.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, ContingencyMethod, ErrorPropagationMode, FramePreset, GraphKind, GroupedArrangement,
    MathTransformPreset, MixedModelDfMethod, MultiplicityMethod, OneWayMethod, PowerSolveTarget, PowerTestFamily, RepeatedMeasuresDesign,
    ResidualCovariance, SubcolumnAggregation, TableKind, TwoGroupMethod,
};
use curvebook_palette::PaletteId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dialogs_r9::{norm_key, parse_finite_f64_with_locale, parse_usize_min};
use crate::i18n::LocalePreset;

pub(super) const fn default_tg_param_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::UnpairedT
}

pub(super) const fn default_tg_param_paired() -> TwoGroupMethod {
    TwoGroupMethod::PairedT
}

pub(super) const fn default_tg_nonparam_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::MannWhitneyU
}

pub(super) const fn default_tg_nonparam_paired() -> TwoGroupMethod {
    TwoGroupMethod::WilcoxonSignedRank
}

/// Preserves each quadrant's last chosen [`TwoGroupMethod`] across Gaussian / paired toggles (`RV-76`, `RV-78`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoGroupSubmethodMemory {
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
}

impl Default for TwoGroupSubmethodMemory {
    fn default() -> Self {
        Self {
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
        }
    }
}

impl TwoGroupSubmethodMemory {
    pub fn remember(&mut self, method: TwoGroupMethod) {
        match method {
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT => {
                self.saved_two_group_param_unpaired = method;
            }
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT => {
                self.saved_two_group_param_paired = method;
            }
            TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov => {
                self.saved_two_group_nonparam_unpaired = method;
            }
            TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                self.saved_two_group_nonparam_paired = method;
            }
            TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
        }
    }

    #[must_use]
    pub const fn resolve(&self, assume_gaussian: bool, paired: bool, current: Option<TwoGroupMethod>) -> TwoGroupMethod {
        if let Some(TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical }) = current {
            return if assume_gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } };
        }
        match (assume_gaussian, paired) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
        }
    }
}

pub(super) const fn two_group_is_paired(method: TwoGroupMethod) -> bool {
    matches!(method, TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest)
}

pub(super) const fn two_group_is_gaussian(method: TwoGroupMethod) -> bool {
    !matches!(
        method,
        TwoGroupMethod::MannWhitneyU
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::KolmogorovSmirnov
            | TwoGroupMethod::OneSampleWilcoxon { .. }
            | TwoGroupMethod::PairedSignTest
    )
}

pub(super) fn parse_bool_field(val: &Value, field: &str) -> Result<bool, String> {
    val.as_bool().ok_or_else(|| format!("`{field}` must be boolean"))
}

pub(super) fn parse_str_field<'a>(val: &'a Value, field: &str) -> Result<&'a str, String> {
    val.as_str().ok_or_else(|| format!("`{field}` must be a string"))
}

pub(super) fn parse_opt_usize_min(val: &Value, key: &str, min: usize) -> Result<Option<usize>, String> {
    if val.is_null() { Ok(None) } else { Ok(Some(parse_usize_min(val, key, min)?)) }
}

pub(super) fn parse_open_unit_f64_with_val(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let t = parse_finite_f64_with_locale(val, key, preset)?;
    if !(0.0 < t && t < 1.0) {
        return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
    }
    Ok(t)
}

pub(super) fn parse_open_unit_f64(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let v = parse_finite_f64_with_locale(val, key, preset)?;
    if !(0.0 < v && v < 1.0) {
        return Err(format!("`{key}` must lie strictly in (0, 1)"));
    }
    Ok(v)
}

pub(super) fn parse_pos_f64(val: &Value, key: &str, preset: &LocalePreset) -> Result<f64, String> {
    let v = parse_finite_f64_with_locale(val, key, preset)?;
    if v <= 0.0 {
        return Err(format!("`{key}` must be > 0"));
    }
    Ok(v)
}

pub(super) fn parse_ci_method_choice(val: &Value, key: &str, label: &str, allowed: [&str; 2]) -> Result<String, String> {
    let s = parse_str_field(val, key)?;
    if !allowed.contains(&s) {
        return Err(format!("unsupported {label} CI method `{s}`; expected `{}` or `{}`", allowed[0], allowed[1]));
    }
    Ok(s.to_owned())
}

pub(super) fn check_pcr_target_leakage(col: usize, blocked: bool) -> Result<(), String> {
    if blocked {
        return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
    }
    Ok(())
}

/// Toggle `col` inside a sorted `Vec<usize>`, returning `true` if inserted or `false` if removed.
pub(super) fn toggle_sorted_col(cols: &mut Vec<usize>, col: usize) -> bool {
    if let Some(pos) = cols.iter().position(|&c| c == col) {
        cols.remove(pos);
        false
    } else {
        cols.push(col);
        cols.sort_unstable();
        true
    }
}

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

pub(super) fn parse_power_family_str(raw: &str) -> Result<PowerTestFamily, String> {
    match norm_key(raw).as_str() {
        "unpairedt" | "twosampleunpairedt" | "independentt" => Ok(PowerTestFamily::TwoSampleUnpairedT),
        "pairedt" | "twosamplepairedt" | "dependentt" => Ok(PowerTestFamily::TwoSamplePairedT),
        "onesamplet" | "singlesamplet" => Ok(PowerTestFamily::OneSampleT),
        "onewayanova" | "anova" => Ok(PowerTestFamily::OneWayAnova),
        "twoproportions" | "proportions" => Ok(PowerTestFamily::TwoProportions),
        "correlation" | "pearsoncorrelation" => Ok(PowerTestFamily::Correlation),
        "chisquare" | "chi2" => Ok(PowerTestFamily::ChiSquare),
        "simplelinearregression" | "linearregression" | "regression" => Ok(PowerTestFamily::SimpleLinearRegression),
        _ => Err(format!("unknown power test family `{raw}`")),
    }
}

pub(super) fn parse_power_solve_target_str(raw: &str) -> Result<PowerSolveTarget, String> {
    match norm_key(raw).as_str() {
        "samplesize" | "n" => Ok(PowerSolveTarget::SampleSize),
        "power" => Ok(PowerSolveTarget::Power),
        "detectableeffect" | "effect" | "mde" => Ok(PowerSolveTarget::DetectableEffect),
        _ => Err(format!("unknown power solve target `{raw}`")),
    }
}

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

pub(super) fn parse_baseline_operation_str(raw: &str) -> Result<BaselineOperation, String> {
    match norm_key(raw).as_str() {
        "difference" | "subtract" => Ok(BaselineOperation::Difference),
        "ratio" | "divide" => Ok(BaselineOperation::Ratio),
        "percentchange" | "pctchange" | "percent" => Ok(BaselineOperation::PercentChange),
        "fractionalchange" | "fraction" => Ok(BaselineOperation::FractionalChange),
        _ => Err(format!("unknown baseline operation `{raw}`")),
    }
}

pub(super) fn parse_grouped_arrangement_str(raw: &str) -> Result<GroupedArrangement, String> {
    match norm_key(raw).as_str() {
        "interleaved" | "sidebyside" => Ok(GroupedArrangement::Interleaved),
        "stacked" => Ok(GroupedArrangement::Stacked),
        "superimposed" | "overlaid" | "overlay" => Ok(GroupedArrangement::Superimposed),
        _ => Err(format!("unknown grouped_arrangement `{raw}`")),
    }
}

pub(super) fn parse_graph_kind_str(raw: &str) -> Result<GraphKind, String> {
    match norm_key(raw).as_str() {
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
    match norm_key(raw).as_str() {
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
    match norm_key(raw).as_str() {
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
