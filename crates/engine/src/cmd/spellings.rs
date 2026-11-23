//! Accepted spellings for command parameters that handlers read directly from JSON.
//!
//! Each table lists every accepted spelling of one parameter in precedence order: the canonical
//! `snake_case` spelling comes first (index `0`), followed by retired aliases kept during the
//! R16.5 deprecation window. [`first_present`] implements that precedence rule and records
//! deprecation hits for any retired spelling supplied by a caller. Parameters declared in a
//! command's input schema use `"x-deprecated-aliases"` instead (see [`super::params`]).
//!
//! The static tables are defined once in [`curvebook_doc::param_spellings`] so both persisted
//! format migration and engine execution share one source of truth. [`ALL`] lists every table by
//! name for tests and tooling.

use serde_json::{Map, Value};

pub use curvebook_doc::param_spellings::{
    ALL, ANNOTATION_BORDERED, ANNOTATION_BOUNDS, ANNOTATION_DASH, ANNOTATION_STROKE_COLOR, CLUSTER_DISTANCE, CLUSTER_LINKAGE, CONTINGENCY_METHOD,
    CONTINGENCY_YATES, CONVERT_PREVIEW, COX_PREDICTOR_COLUMNS, DERIVED_COLUMN_FORMULA, DERIVED_COLUMN_TITLE, GLM_FAMILY, KAPPA_WEIGHTING,
    LINE_DASH_STYLE, LINE_STROKE_WIDTH, LINEAR_DEMING, MARKER_FILL_COLOR, MODEL_COMPARISON_ALTERNATIVE, MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION,
    MODEL_COMPARISON_METHOD, MULTIPLICITY, MULTIPLICITY_ALPHA, MULTIVARIATE_IMPUTATIONS, MULTIVARIATE_K_CLUSTERS, MULTIVARIATE_MAX_ITERS,
    MULTIVARIATE_PREDICTOR_COLUMNS, MULTIVARIATE_RESPONSE_COLUMN, MULTIVARIATE_SPEARMAN, MV_GROUP_COLUMN, MV_RESPONSE_COLUMN, MV_STATUS_COLUMN,
    MV_TIME_COLUMN, NONLINEAR_COMPARISON, NONLINEAR_FIT_METHOD, NONLINEAR_NORMALITY_TESTS, ONE_SAMPLE_HYPOTHETICAL, OUTLIER_Q_PERCENT,
    PARAMETER_RULE_NAME, PARAMETER_SCOPE_HOOK, PARAMETER_SCOPE_VALUES, POINT_SUBCOLUMN, POWER_CHI_SQUARE_DF, POWER_EFFECT_SIZE, POWER_NUM_GROUPS,
    POWER_SAMPLE_SIZE, POWER_SAMPLE_SIZE_GROUP2, PVALUE_STACK_COLUMN, PVALUE_STACK_METHOD, PVALUE_STACK_THRESHOLD, REFERENCE_LEVEL_COLUMN,
    REFERENCE_LEVEL_LEVEL, SERIES_COLOR, SERIES_CONNECTING_LINE, SERIES_GRAPHICAL_OBJECT, SERIES_MARKER, SERIES_Y_ERROR_BARS, SIMULATED_COLUMN_KIND,
    SIMULATED_COLUMN_NAME, SURVIVAL_GEHAN, SURVIVAL_LOG_RANK, TRANSFORM_AVERAGE_WINDOW, TRANSFORM_DERIVED_COLUMNS, TRANSFORM_ERROR_PROPAGATION,
    TRANSFORM_EVERY_K_ROWS, TRANSFORM_HUNDRED_ANCHOR, TRANSFORM_ROW_COUNT, TRANSFORM_SAMPLE_SIZE, TRANSFORM_Y, TRANSFORM_ZERO_ANCHOR,
};

/// A JSON object that parameters can be looked up in.
pub trait ParamSource {
    /// Look up one parameter by its exact key.
    fn param(&self, key: &str) -> Option<&Value>;
}

impl ParamSource for Value {
    fn param(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl ParamSource for Map<String, Value> {
    fn param(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

/// Return `(matched_key, value)` of the first spelling present in `source`, even when `null`.
///
/// Any retired spelling (`&spellings[1..]`) present in `source` is recorded against the canonical
/// spelling (`spellings[0]`) for the active command deprecation scope.
#[must_use]
pub fn first_present_with_key<'a, 'b, S: ParamSource + ?Sized>(source: &'a S, spellings: &[&'b str]) -> Option<(&'b str, &'a Value)> {
    if let Some((&canonical, deprecated_list)) = spellings.split_first() {
        for &deprecated in deprecated_list {
            if deprecated != canonical && source.param(deprecated).is_some() {
                super::params::record_deprecated_alias(deprecated, canonical);
            }
        }
    }
    spellings.iter().find_map(|key| source.param(key).map(|val| (*key, val)))
}

/// Return the value of the first spelling present in `source`, even when that value is `null`.
#[must_use]
pub fn first_present<'a, S: ParamSource + ?Sized>(source: &'a S, spellings: &[&str]) -> Option<&'a Value> {
    first_present_with_key(source, spellings).map(|(_, val)| val)
}
