//! Live TeX model equation box annotation compiler (`R9.16` / `QOL-16` / `R10.19`).
//!
//! Resolves regression equations dynamically from the target [`ResultSheet`] in
//! [`ProjectDocument`], parses a structured TeX-subset math grammar into recursive
//! two-dimensional math nodes (nested fractions, superscripts/subscripts, radicals,
//! scaled delimiters, Greek parameters, and roman math operators), and lays out
//! publication-grade boxes following Knuth, D. E. (1984), *The TeXbook*, Addison-Wesley,
//! Chapter 17 (pp. 140–145) and Appendix G ("Generating Boxes from Formulas", pp. 441–447):
//!
//! - **Math styles & cramped transitions** (Ch. 17 p. 140, Appendix G p. 441):
//!   eight styles (`D`, `D'`, `T`, `T'`, `S`, `S'`, `SS`, `SS'`) using `1.0 / 0.70 / 0.50`
//!   font scaling (`cmr10` / `cmr7` / `cmr5` ratios).
//! - **Appendix G fontdimens** (p. 447, normalized in current-style `em` and matched to
//!   `curvebook-text` metrics with math axis $\sigma_{22} = 0.34\,\text{em}$ and x-height
//!   $\sigma_5 = 0.52\,\text{em}$, rule thickness $\theta = \xi_8 = \max(0.050\,\text{em}, 0.30\,\text{pt})$).
//! - **Rule 11** (`\sqrt`, p. 443): radicand in cramped style $C'$, clearance
//!   $\varphi = \theta + \frac{1}{4}\psi$ ($\psi = \sigma_5$ in display style, $\theta$ otherwise),
//!   vector radical surd joined to a horizontal overbar vinculum rule of thickness $\theta$.
//! - **Rules 15a–15e** (`\frac`, `\dfrac`, `\tfrac`, `\binom`, pp. 444–445): numerator shifts
//!   $\sigma_8 \dots \sigma_{10}$, denominator shifts $\sigma_{11} \dots \sigma_{12}$, math-axis
//!   $\sigma_{22}$ alignment, minimum clearances $3\theta$ / $\theta$ (with rule) and $7\theta$ / $3\theta$
//!   (rule-less `\binom`), centered numerator/denominator alignment, and `\nulldelimiterspace`.
//! - **Rules 18a–18f** (`^`, `_`, simultaneous `_sub^sup`, p. 445): nucleus box height/depth drop
//!   ($\sigma_{18}, \sigma_{19}$) for composite boxes vs zero initial drop for single-character nuclei,
//!   shift clamps ($\sigma_{13} \dots \sigma_{17}$), guaranteed $\ge 4\theta$ vertical separation between
//!   stacked superscripts and subscripts, $\frac{4}{5}\sigma_5$ adjustment, and `scriptspace`.
//! - **Rule 19** (scaled delimiters `\left...\right`, `( ... )`, `[ ... ]`, `| ... |`, `\{ ... \}`,
//!   p. 446): symmetric vertical span around the math axis $\sigma_{22}$ covering tall stacked
//!   subexpressions via smooth vector [`crate::scene::Path2D`] outlines.

mod layout;
mod parser;
mod render;

use curvebook_doc::{AnalysisKind, EquationDisplayMode, ProjectDocument, ResultSheet, ResultStatus, SheetId};
use curvebook_fit::tex_format::{format_model_equation_tex, format_polynomial_equation_tex};

pub use render::{render_model_equation_annotation, render_tex_equation_box, resolve_model_equation_hit_bounds};

const DEFAULT_FONT_SIZE_PT: f64 = 10.0;
const EQUATION_SIG_FIGS: usize = 4;
const MAX_TEX_DEPTH: usize = 48;
const MIN_RULE_THICKNESS_PT: f64 = 0.30;

/// Inputs required to compile a [`curvebook_doc::GraphAnnotation::ModelEquation`] annotation.
#[derive(Debug, Clone, Copy)]
pub struct ModelEquationParams<'a> {
    /// Target regression [`ResultSheet`] ID in the [`ProjectDocument`].
    pub result_sheet_id: &'a SheetId,
    /// Zero-based dataset index within the result sheet.
    pub data_set_index: usize,
    /// Symbolic parameter names vs live substituted coefficient values.
    pub mode: EquationDisplayMode,
    /// Annotation bounding box in points (zero width/height triggers auto-sizing).
    pub bounds_pt: curvebook_doc::RectPt,
    /// Base font size in points for top-level equation runs.
    pub font_size_pt: f64,
    /// Whether to draw a white filled background and 0.8 pt black border.
    pub bordered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedEquation {
    Tex(String),
    Placeholder(String),
    Empty,
}

fn is_summary_or_gof_scalar(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "r2" | "r^2"
            | "adjusted r2"
            | "adjusted r^2"
            | "pseudo r2"
            | "rss"
            | "rsdr"
            | "sy.x"
            | "residual sd (sy.x)"
            | "rmse"
            | "aic"
            | "aicc"
            | "bic"
            | "auc"
            | "roc auc"
            | "dof"
    ) || lower.starts_with("auc (")
        || lower.starts_with("residual sd")
}

fn normalize_scalar_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn polynomial_order_for_equation_id(equation_id: &str) -> Option<(usize, bool)> {
    match normalize_scalar_key(equation_id).as_str() {
        "straight_line" => Some((1, false)),
        "line_through_origin" => Some((1, true)),
        "polynomial_2" | "quadratic" => Some((2, false)),
        "polynomial_3" | "cubic" => Some((3, false)),
        "polynomial_4" | "quartic" => Some((4, false)),
        "polynomial_5" => Some((5, false)),
        "polynomial_6" => Some((6, false)),
        _ => None,
    }
}

fn has_all_polynomial_coeffs(params: &[(String, f64)], order: usize, through_origin: bool) -> bool {
    if order <= 1 {
        let lin = collect_linear_params(params, through_origin);
        return lin.len() == if through_origin { 1 } else { 2 };
    }
    let start = usize::from(through_origin);
    (start..=order).all(|power| {
        let b0 = format!("b{power}");
        let b1 = format!("b_{power}");
        let a0 = format!("a{power}");
        let a1 = format!("a_{power}");
        let beta = format!("beta{power}");
        let c = format!("c{power}");
        params.iter().any(|(name, _)| {
            let key = normalize_scalar_key(name);
            key == b0
                || key == b1
                || key == a0
                || key == a1
                || key == beta
                || key == c
                || (power == 0 && matches!(key.as_str(), "intercept" | "yintercept" | "y_intercept"))
                || (power == 1 && key == "slope")
        })
    })
}

/// Collect scalar parameters for `data_set_index` from `res`, preferring
/// dataset-specific entries and falling back to shared (`data_set_index: None`)
/// parameters for keys not already bound to the dataset when another dataset
/// hasn't already claimed dataset-scoped parameters alone.
#[must_use]
pub(crate) fn collect_dataset_scalars(res: &ResultSheet, data_set_index: usize) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();

    for scalar in &res.scalars {
        if scalar.data_set_index == Some(data_set_index) && scalar.value.is_finite() && !is_summary_or_gof_scalar(&scalar.key) {
            let lower = scalar.key.trim().to_ascii_lowercase();
            if !seen_keys.iter().any(|k| k == &lower) {
                seen_keys.push(lower);
                out.push((scalar.key.clone(), scalar.value));
            }
        }
    }

    let has_other_dataset_scoped = res
        .scalars
        .iter()
        .any(|s| s.data_set_index.is_some_and(|idx| idx != data_set_index) && s.value.is_finite() && !is_summary_or_gof_scalar(&s.key));
    if !out.is_empty() || !has_other_dataset_scoped {
        for scalar in &res.scalars {
            if scalar.data_set_index.is_none() && scalar.value.is_finite() && !is_summary_or_gof_scalar(&scalar.key) {
                let lower = scalar.key.trim().to_ascii_lowercase();
                if !seen_keys.iter().any(|k| k == &lower) {
                    seen_keys.push(lower);
                    out.push((scalar.key.clone(), scalar.value));
                }
            }
        }
    }

    out
}

/// Resolve the live TeX equation string (or diagnostic placeholder) from `doc`.
pub(super) fn resolve_model_equation(
    doc: &ProjectDocument,
    result_sheet_id: &SheetId,
    data_set_index: usize,
    mode: EquationDisplayMode,
) -> ResolvedEquation {
    let Some(res) = doc.result_sheet(result_sheet_id) else {
        return ResolvedEquation::Placeholder("(Equation unavailable)".to_owned());
    };

    if let ResultStatus::Failed { message, .. } = &res.status {
        let label = if message.trim().is_empty() { "(Fit failed)".to_owned() } else { "(Fit failed — check analysis)".to_owned() };
        return ResolvedEquation::Placeholder(label);
    }
    if matches!(res.status, ResultStatus::Dirty) {
        return ResolvedEquation::Placeholder("(Fit pending)".to_owned());
    }

    let substituted = matches!(mode, EquationDisplayMode::Substituted);
    let params = collect_dataset_scalars(res, data_set_index);
    let overlay_for_dataset = if res.fitted_curves.is_empty() {
        None
    } else {
        let Some(ov) = res.fitted_curves.iter().find(|c| c.data_set_index == data_set_index) else {
            return ResolvedEquation::Placeholder("(Fit unavailable)".to_owned());
        };
        if !ov.converged {
            return ResolvedEquation::Placeholder("(Fit did not converge)".to_owned());
        }
        Some(ov)
    };

    match &res.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => {
            let equation_id =
                overlay_for_dataset.map(|c| c.equation_id.as_str()).filter(|id| !id.trim().is_empty()).unwrap_or(nl.equation_id.as_str());
            if equation_id.trim().is_empty() {
                return ResolvedEquation::Placeholder("(No nonlinear model selected)".to_owned());
            }
            let custom_dsl = if overlay_for_dataset.is_none() || equation_id == nl.equation_id { nl.custom_dsl.as_deref() } else { None };
            if substituted {
                if params.is_empty() {
                    return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                }
                if custom_dsl.is_none()
                    && let Some((order, through_origin)) = polynomial_order_for_equation_id(equation_id)
                    && !has_all_polynomial_coeffs(&params, order, through_origin)
                {
                    return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                }
            }
            let tex = format_model_equation_tex(equation_id, custom_dsl, &params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::LinearRegression(lin) => {
            let through_origin = lin.force_through_origin && !lin.deming_model_ii;
            let lin_params = collect_linear_params(&params, through_origin);
            if substituted && !has_all_polynomial_coeffs(&lin_params, 1, through_origin) {
                return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
            }
            let tex = format_polynomial_equation_tex(1, through_origin, &lin_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::PassingBablok(_) => {
            let pb_params = collect_linear_params(&params, false);
            if substituted && !has_all_polynomial_coeffs(&pb_params, 1, false) {
                return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
            }
            let tex = format_polynomial_equation_tex(1, false, &pb_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::SimpleLogisticRegression(_) => {
            if !substituted {
                return ResolvedEquation::Tex(String::from("P(Y=1) = \\frac{1}{1 + e^{-(\\beta_0 + \\beta_1 X)}}"));
            }
            let b0 = params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "beta0" | "b0" | "b_0")).map(|(_, v)| *v);
            let b1 = params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "beta1" | "b1" | "b_1")).map(|(_, v)| *v);
            match (b0, b1) {
                (Some(v0), Some(v1)) if v0.is_finite() && v1.is_finite() => {
                    let sign = if v1 < 0.0 { "-" } else { "+" };
                    let abs_b1 = v1.abs();
                    ResolvedEquation::Tex(format!("P(Y=1) = \\frac{{1}}{{1 + e^{{-({v0:.4} {sign} {abs_b1:.4} X)}}}}"))
                }
                _ => ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned()),
            }
        }
        _ => {
            if let Some(curve) = overlay_for_dataset.filter(|c| !c.equation_id.trim().is_empty()) {
                if substituted {
                    if params.is_empty() {
                        return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                    }
                    if let Some((order, through_origin)) = polynomial_order_for_equation_id(&curve.equation_id)
                        && !has_all_polynomial_coeffs(&params, order, through_origin)
                    {
                        return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                    }
                }
                let tex = format_model_equation_tex(&curve.equation_id, None, &params, substituted, EQUATION_SIG_FIGS);
                ResolvedEquation::Tex(tex)
            } else {
                ResolvedEquation::Empty
            }
        }
    }
}

/// Extract canonical `("Slope", ..)` and `("YIntercept", ..)` entries so
/// unrelated scalars like `"R2"` are never consumed positionally.
fn collect_linear_params(params: &[(String, f64)], force_through_origin: bool) -> Vec<(String, f64)> {
    let mut out = Vec::with_capacity(2);
    if let Some((_, slope)) = params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "slope" | "b1" | "b_1")) {
        out.push(("Slope".to_owned(), *slope));
    }
    if !force_through_origin
        && let Some((_, intercept)) =
            params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "yintercept" | "y_intercept" | "intercept" | "b0" | "b_0"))
    {
        out.push(("YIntercept".to_owned(), *intercept));
    }
    out
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, EquationDisplayMode, ErrorCode, FittedCurveOverlay, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument,
        RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
    };

    use curvebook_fit::tex_format::format_model_equation_tex;

    use super::parser::{parse_math_slice, parse_next_step};
    use super::{MAX_TEX_DEPTH, ModelEquationParams, render_model_equation_annotation, render_tex_equation_box};
    use crate::scene::{ChartScene, ScenePrimitive};

    fn empty_scene() -> ChartScene {
        ChartScene::new("graph-1", "Test", 400.0, 300.0, None)
    }

    fn make_base_doc_with_result(sheet: ResultSheet) -> (ProjectDocument, SheetId) {
        let mut doc = ProjectDocument::new("EquationBoxTest").expect("ProjectDocument::new");
        let id = sheet.id.clone();
        doc.result_sheets.push(sheet);
        (doc, id)
    }

    fn make_nonlinear_result(
        id: &str,
        equation_id: &str,
        custom_dsl: Option<&str>,
        scalars: Vec<ResultScalarEntry>,
        overlays: Vec<FittedCurveOverlay>,
    ) -> ResultSheet {
        let mut nl = NonlinearFitSpec::new(equation_id);
        nl.custom_dsl = custom_dsl.map(str::to_owned);
        let spec = AnalysisSpec::new(id, "data-1", AnalysisKind::NonlinearRegression(nl)).expect("AnalysisSpec::new");
        let mut sheet = ResultSheet::new(spec).expect("ResultSheet::new");
        sheet.status = ResultStatus::UpToDate;
        sheet.scalars = scalars;
        sheet.fitted_curves = overlays;
        sheet
    }

    fn collect_text_strings(scene: &ChartScene) -> Vec<String> {
        scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.spans.iter().map(|s| s.text.as_str()).collect::<String>()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn four_pl_symbolic_and_substituted_emit_fraction_vinculum_and_live_numbers() {
        let scalars = vec![
            ResultScalarEntry { key: "Bottom".to_owned(), data_set_index: Some(0), value: 10.5, units: None },
            ResultScalarEntry { key: "Top".to_owned(), data_set_index: Some(0), value: 98.2, units: None },
            ResultScalarEntry { key: "LogEC50".to_owned(), data_set_index: Some(0), value: -6.25, units: None },
            ResultScalarEntry { key: "HillSlope".to_owned(), data_set_index: Some(0), value: 1.45, units: None },
            ResultScalarEntry { key: "R2".to_owned(), data_set_index: Some(0), value: 0.9987, units: None },
        ];
        let sheet = make_nonlinear_result("res-nl", "log_agonist_vs_response_4pl", None, scalars, Vec::new());
        let (mut doc, id) = make_base_doc_with_result(sheet);

        let mut sym_scene = empty_scene();
        render_model_equation_annotation(
            &mut sym_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sym_texts = collect_text_strings(&sym_scene).join(" | ");
        assert!(sym_texts.contains("Bottom"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("Top"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("HillSlope"), "sym_texts={sym_texts}");
        assert!(!sym_texts.contains("R2"), "must not include extra R2 callout: {sym_texts}");

        let path_count = sym_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
        assert!(path_count >= 2, "expected box border + vinculum path, got {path_count}");

        let mut sub_scene = empty_scene();
        render_model_equation_annotation(
            &mut sub_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sub_texts = collect_text_strings(&sub_scene).join(" | ");
        assert!(sub_texts.contains("10.5"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("98.2"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("1.45"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("6.25"), "sub_texts={sub_texts}");

        // Live refresh invariant: mutate ResultSheet::scalars without changing params
        doc.result_sheets[0].scalars[1].value = 142.0;
        let mut updated_scene = empty_scene();
        render_model_equation_annotation(
            &mut updated_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let updated_texts = collect_text_strings(&updated_scene).join(" | ");
        assert!(updated_texts.contains("142"), "updated_texts={updated_texts}");
        assert!(!updated_texts.contains("98.2"), "stale Top remained: {updated_texts}");
    }

    #[test]
    fn overlay_winner_equation_id_overrides_spec_equation_id() {
        let sheet = make_nonlinear_result(
            "res-mm",
            "log_agonist_vs_response_4pl",
            None,
            vec![
                ResultScalarEntry { key: "Vmax".to_owned(), data_set_index: Some(0), value: 52.5, units: None },
                ResultScalarEntry { key: "Km".to_owned(), data_set_index: Some(0), value: 3.2, units: None },
            ],
            vec![FittedCurveOverlay {
                data_set_index: 0,
                equation_id: "michaelis_menten".to_owned(),
                converged: true,
                ambiguous: false,
                points: Vec::new(),
            }],
        );
        let (doc, id) = make_base_doc_with_result(sheet);
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(20.0, 20.0, 200.0, 50.0),
                font_size_pt: 9.0,
                bordered: false,
            },
        );
        let texts = collect_text_strings(&scene).join(" | ");
        assert!(texts.contains("52.5"), "texts={texts}");
        assert!(texts.contains("3.2"), "texts={texts}");
        let path_count = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
        assert!(path_count >= 1, "expected vinculum line path for Michaelis-Menten");
    }

    #[test]
    fn linear_regression_origin_and_standard_modes() {
        let lin_std = LinearRegressionSpec { force_through_origin: false, ..Default::default() };
        let spec = AnalysisSpec::new("res-lin", "data-1", AnalysisKind::LinearRegression(lin_std)).expect("AnalysisSpec::new");
        let mut sheet = ResultSheet::new(spec).expect("ResultSheet::new");
        sheet.status = ResultStatus::UpToDate;
        sheet.scalars = vec![
            ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.5, units: None },
            ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -1.25, units: None },
            ResultScalarEntry { key: "R2".to_owned(), data_set_index: Some(0), value: 0.99, units: None },
        ];
        let (mut doc, id) = make_base_doc_with_result(sheet);

        let mut sym_scene = empty_scene();
        render_model_equation_annotation(
            &mut sym_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sym_texts = collect_text_strings(&sym_scene).join(" ");
        assert!(sym_texts.contains("Slope"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("Intercept"), "sym_texts={sym_texts}");

        let mut sub_scene = empty_scene();
        render_model_equation_annotation(
            &mut sub_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&sub_scene).join(" ");
        assert!(texts.contains("2.5"), "texts={texts}");
        assert!(texts.contains("1.25"), "texts={texts}");
        assert!(!texts.contains("0.99"), "R2 must not leak: {texts}");

        // Force through origin
        doc.result_sheets[0].analysis.kind =
            AnalysisKind::LinearRegression(LinearRegressionSpec { force_through_origin: true, ..Default::default() });
        let mut origin_scene = empty_scene();
        render_model_equation_annotation(
            &mut origin_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: false,
            },
        );
        let origin_texts = collect_text_strings(&origin_scene).join(" ");
        assert!(origin_texts.contains("2.5"), "origin_texts={origin_texts}");
        assert!(!origin_texts.contains("1.25"), "intercept should be omitted: {origin_texts}");
    }

    #[test]
    fn custom_dsl_and_shared_scalar_fallback_and_narrow_wrap() {
        let sheet = make_nonlinear_result(
            "res-dsl",
            "custom",
            Some("Y = k * X / (Kd + X)"),
            vec![
                ResultScalarEntry { key: "k".to_owned(), data_set_index: Some(1), value: 12.0, units: None },
                ResultScalarEntry { key: "Kd".to_owned(), data_set_index: None, value: 4.5, units: None },
            ],
            Vec::new(),
        );
        let (doc, id) = make_base_doc_with_result(sheet);
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 1,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(10.0, 10.0, 75.0, 28.0),
                font_size_pt: f64::NAN,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&scene).join(" ");
        assert!(texts.contains("12"), "texts={texts}");
        assert!(texts.contains("4.5"), "texts={texts}");
    }

    #[test]
    fn missing_and_failed_result_sheets_render_placeholders_without_nans() {
        let mut doc = ProjectDocument::new("Empty").expect("ProjectDocument::new");
        let missing_id = SheetId::new("does-not-exist");
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &missing_id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(10.0, 10.0, 120.0, 24.0),
                font_size_pt: -3.0,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&scene).join(" ");
        assert!(texts.contains("unavailable"), "texts={texts}");

        let mut failed_sheet = make_nonlinear_result("res-failed", "michaelis_menten", None, Vec::new(), Vec::new());
        failed_sheet.status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "Singular Jacobian".to_owned() };
        let failed_id = failed_sheet.id.clone();
        doc.result_sheets.push(failed_sheet);
        let mut failed_scene = empty_scene();
        render_model_equation_annotation(
            &mut failed_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &failed_id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(10.0, 10.0, 140.0, 28.0),
                font_size_pt: 9.0,
                bordered: true,
            },
        );
        let failed_texts = collect_text_strings(&failed_scene).join(" ");
        assert!(failed_texts.contains("failed"), "failed_texts={failed_texts}");
    }

    #[test]
    fn tex_text_escapes_backslash_less_greater_and_star_render_cleanly() {
        let tex = format_model_equation_tex("a<b\\c>d*e", None, &[], false, 4);
        assert!(tex.contains("\\textless{}") && tex.contains("\\textbackslash{}") && tex.contains("\\textgreater{}"), "tex={tex}");

        let mut scene = empty_scene();
        render_tex_equation_box(&mut scene, DocRectPt::new(10.0, 10.0, 0.0, 0.0), &tex, 10.0, false);
        let joined = collect_text_strings(&scene).join("");
        assert!(!joined.contains("textless") && !joined.contains("textgreater") && !joined.contains("textbackslash"), "joined={joined}");
        assert!(joined.contains('<') && joined.contains('>') && joined.contains('\\') && joined.contains('*'), "joined={joined}");
    }

    #[test]
    fn malformed_tex_scripts_and_max_depth_never_hang_or_oom() {
        for sample in ["^", "_", "^ ", "_ ", "^{unclosed", "_{unclosed", "^}", "_}"] {
            let mut pos = 0usize;
            let step = parse_next_step(sample, &mut pos, false, 0);
            assert!(step.is_some(), "expected progress step for {sample:?}");
            assert!(pos > 0, "expected strict forward progress for {sample:?}, got pos={pos}");

            let mut max_depth_pos = 0usize;
            let step_depth = parse_next_step(sample, &mut max_depth_pos, false, MAX_TEX_DEPTH);
            assert!(step_depth.is_some(), "expected fallback atom at MAX_TEX_DEPTH for {sample:?}");
            assert!(max_depth_pos > 0, "expected forward progress at MAX_TEX_DEPTH for {sample:?}");
        }

        let chained = format!("x{}", "^2".repeat(120));
        let nested = format!("{}x^2{}", "{".repeat(120), "}".repeat(120));
        let cases = ["x^", "x_ ", "y = ^", "y = ^{unclosed", "^}", "x^1^2^3^4", chained.as_str(), nested.as_str()];
        for input in cases {
            let nodes = parse_math_slice(input, false, 0);
            assert!(!nodes.is_empty(), "expected parsed AST nodes for {input:?}");
            let mut scene = empty_scene();
            render_tex_equation_box(&mut scene, DocRectPt::new(10.0, 10.0, 220.0, 40.0), input, 10.0, false);
            assert!(!scene.primitives.is_empty(), "expected rendered primitives for {input:?}");
            assert!(scene.primitives.len() < 5_000, "unexpected primitive explosion for {input:?}: {}", scene.primitives.len());
        }
    }
}
