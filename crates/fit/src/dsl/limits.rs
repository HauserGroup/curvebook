//! Resource limits that bound DSL parsing, compilation, VM execution, and ODE integration.

pub(in crate::dsl) const MAX_VM_STEPS: usize = 100_000;
pub(in crate::dsl) const MAX_ODE_STEPS: usize = 50_000;
pub(in crate::dsl) const MAX_ODE_VM_OPS_PER_EVAL: usize = 5_000_000;
pub(in crate::dsl) const ODE_VM_OPS_PER_NOMINAL_STEP: usize = 100;
pub(crate) const DEFAULT_MAX_ODE_STEPS_PER_FIT: usize = 300_000;
pub(in crate::dsl) const MAX_CACHED_ODE_TRAJECTORIES: usize = 4;
pub(in crate::dsl) const MAX_CACHED_ODE_KNOTS: usize = 4_096;
pub(in crate::dsl) const MAX_PARSE_DEPTH: usize = 128;
pub(in crate::dsl) const MAX_AST_NODES: usize = 4_096;
pub(in crate::dsl) const MAX_DATASET_EXPANSION: usize = 10_000;
pub(in crate::dsl) const MAX_DSL_SOURCE_BYTES: usize = 1_048_576;
pub(in crate::dsl) const MAX_DSL_LINES: usize = 16_384;
pub(in crate::dsl) const MAX_TOTAL_DATASET_INDICES: usize = 50_000;
pub(in crate::dsl) const MAX_DATASET_CLASSES: usize = 4_096;
pub(in crate::dsl) const MAX_TOTAL_OPCODES: usize = 100_000;
pub(in crate::dsl) const MAX_BESSEL_ORDER: i32 = 100_000;
