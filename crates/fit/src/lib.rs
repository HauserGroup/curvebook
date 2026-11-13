//! Nonlinear and linear regression engine for LumenCraft: dose-response (3PL/4PL/5PL), enzyme
//! kinetics (Michaelis-Menten, allosteric, competitive/noncompetitive inhibition), binding
//! saturation/competition, exponential growth/decay, global parameter sharing, constraints,
//! confidence/prediction bands, AICc / extra-sum-of-squares model comparison and unknown
//! interpolation.
//!
//! Layer L2 domain crate. Governing spec: `docs/curve-fitting-implementation.md` and `docs/fitting-parity.md`.
#![forbid(unsafe_code)]

/// Built-in equation library namespaces for nonlinear regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquationCategory {
    DoseResponseStimulation,
    DoseResponseInhibition,
    ReceptorBinding,
    EnzymeKinetics,
    Exponential,
    Polynomial,
    Gaussian,
    SineWave,
    UserDefined,
}

#[cfg(test)]
mod tests {
    use super::EquationCategory;

    #[test]
    fn equation_categories_are_distinct() {
        assert_ne!(EquationCategory::DoseResponseInhibition, EquationCategory::EnzymeKinetics);
    }
}
