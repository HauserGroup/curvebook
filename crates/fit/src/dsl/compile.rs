//! Bytecode compilation of expression trees with opcode budgets and identifier resolution.
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use std::collections::HashMap;

use crate::dsl::ast::{BinaryKind, BinaryOp, BuiltinFn, CompiledProgram, Expr, OpCode, UnaryKind, UnaryOp};
use crate::dsl::limits::MAX_TOTAL_OPCODES;
use crate::dsl::parser::LocalResolver;
use crate::dsl::statements::{EqItem, parse_predictor_index};
use crate::error::FitError;

pub(in crate::dsl) fn check_opcode_budget(code: &[OpCode], max_opcodes: usize) -> Result<(), FitError> {
    if code.len() > max_opcodes {
        let message = if max_opcodes == MAX_TOTAL_OPCODES {
            format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)")
        } else {
            format!("Compiled equation exceeds remaining bytecode budget ({max_opcodes} of {MAX_TOTAL_OPCODES} total opcodes)")
        };
        return Err(FitError::DslCompileError { message, line: 1, column: 1 });
    }
    Ok(())
}

pub(in crate::dsl) fn compile_program_from_indices(
    eq_items: &[EqItem],
    local_indices: &[usize],
    target: &Expr,
    subtract_from_y: bool,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut code = Vec::new();
    for &idx in local_indices {
        if let EqItem::Local { name: lname, rhs, .. } = &eq_items[idx] {
            let resolver = LocalResolver::Flat(&local_map);
            compile_expr_resolved(rhs, &resolver, params, &mut code, max_opcodes)?;
            let slot = if let Some(&s) = local_map.get(lname) {
                s
            } else {
                let s = local_map.len();
                local_map.insert(lname.clone(), s);
                s
            };
            code.push(OpCode::StoreLocal(slot));
            check_opcode_budget(&code, max_opcodes)?;
        }
    }
    let resolver = LocalResolver::Flat(&local_map);
    if subtract_from_y {
        code.push(OpCode::LoadY);
        check_opcode_budget(&code, max_opcodes)?;
        compile_expr_resolved(target, &resolver, params, &mut code, max_opcodes)?;
        code.push(OpCode::Binary(BinaryKind::Sub));
        check_opcode_budget(&code, max_opcodes)?;
    } else {
        compile_expr_resolved(target, &resolver, params, &mut code, max_opcodes)?;
    }
    code.push(OpCode::Halt);
    check_opcode_budget(&code, max_opcodes)?;
    Ok(CompiledProgram { code, local_count: local_map.len() })
}

pub(in crate::dsl) fn compile_target_resolved(
    target: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    compile_target_resolved_with_curve(target, locals, params, max_opcodes, false)
}

pub(in crate::dsl) fn compile_transform_target_resolved(
    target: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    compile_target_resolved_with_curve(target, locals, params, max_opcodes, true)
}

fn compile_target_resolved_with_curve(
    target: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
    allow_curve_eval: bool,
) -> Result<CompiledProgram, FitError> {
    let mut code = Vec::new();
    compile_expr_resolved_inner(target, locals, params, &mut code, max_opcodes, allow_curve_eval)?;
    code.push(OpCode::Halt);
    check_opcode_budget(&code, max_opcodes)?;
    Ok(CompiledProgram { code, local_count: 0 })
}

pub(in crate::dsl) enum CompileTask<'a> {
    Eval(&'a Expr),
    EmitUnary(UnaryKind),
    EmitBinary(BinaryKind),
    EmitCall(BuiltinFn, usize),
    EmitEvalCurveY,
    EmitSolveCurveX,
    AndAfterLhs { rhs: &'a Expr },
    AndAfterRhs { jz_idx: usize },
    OrAfterLhs { rhs: &'a Expr },
    OrAfterRhs { jump_end: usize },
    IfAfterCond { then_br: &'a Expr, else_br: &'a Expr },
    IfAfterThen { else_br: &'a Expr, jz_idx: usize },
    IfAfterElse { jump_end: usize },
    IsDefinedAfterArg { enter_idx: usize },
}

pub(in crate::dsl) fn binary_op_to_kind(op: BinaryOp) -> BinaryKind {
    match op {
        BinaryOp::Add => BinaryKind::Add,
        BinaryOp::Sub => BinaryKind::Sub,
        BinaryOp::Mul => BinaryKind::Mul,
        BinaryOp::Div => BinaryKind::Div,
        BinaryOp::Rem => BinaryKind::Rem,
        BinaryOp::Pow => BinaryKind::Pow,
        BinaryOp::Eq => BinaryKind::Eq,
        BinaryOp::Ne => BinaryKind::Ne,
        BinaryOp::Lt => BinaryKind::Lt,
        BinaryOp::Le => BinaryKind::Le,
        BinaryOp::Gt => BinaryKind::Gt,
        BinaryOp::Ge => BinaryKind::Ge,
        BinaryOp::And | BinaryOp::Or => BinaryKind::Eq,
    }
}

pub(in crate::dsl) fn schedule_expr_compile<'a>(
    node: &'a Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
    tasks: &mut Vec<CompileTask<'a>>,
    allow_curve_eval: bool,
) -> Result<(), FitError> {
    match node {
        Expr::Num(v) => code.push(OpCode::PushConst(*v)),
        Expr::Ident(name) => compile_ident(name, locals, params, code)?,
        Expr::Unary { op, rhs } => {
            let kind = match op {
                UnaryOp::Neg => UnaryKind::Neg,
                UnaryOp::Not => UnaryKind::Not,
            };
            tasks.push(CompileTask::EmitUnary(kind));
            tasks.push(CompileTask::Eval(rhs));
        }
        Expr::Binary { op, lhs, rhs } => match op {
            BinaryOp::And => {
                tasks.push(CompileTask::AndAfterLhs { rhs });
                tasks.push(CompileTask::Eval(lhs));
            }
            BinaryOp::Or => {
                tasks.push(CompileTask::OrAfterLhs { rhs });
                tasks.push(CompileTask::Eval(lhs));
            }
            _ => {
                tasks.push(CompileTask::EmitBinary(binary_op_to_kind(*op)));
                tasks.push(CompileTask::Eval(rhs));
                tasks.push(CompileTask::Eval(lhs));
            }
        },
        Expr::If { cond, then_br, else_br } => {
            tasks.push(CompileTask::IfAfterCond { then_br, else_br });
            tasks.push(CompileTask::Eval(cond));
        }
        Expr::Call { name, args } => {
            if name == "__curve_y" {
                if !allow_curve_eval {
                    return Err(FitError::DslCompileError {
                        message: "`Y[expr]` curve evaluation is only supported in [Transforms]".to_owned(),
                        line: 1,
                        column: 1,
                    });
                }
                if args.len() != 1 {
                    return Err(FitError::DslCompileError { message: "`Y[expr]` expects 1 argument".to_owned(), line: 1, column: 1 });
                }
                tasks.push(CompileTask::EmitEvalCurveY);
                if let Some(arg) = args.first() {
                    tasks.push(CompileTask::Eval(arg));
                }
                return Ok(());
            }
            if name == "__curve_x" {
                if !allow_curve_eval {
                    return Err(FitError::DslCompileError {
                        message: "`X[expr]` inverse curve evaluation is only supported in [Transforms]".to_owned(),
                        line: 1,
                        column: 1,
                    });
                }
                if args.len() != 1 {
                    return Err(FitError::DslCompileError { message: "`X[expr]` expects 1 argument".to_owned(), line: 1, column: 1 });
                }
                tasks.push(CompileTask::EmitSolveCurveX);
                if let Some(arg) = args.first() {
                    tasks.push(CompileTask::Eval(arg));
                }
                return Ok(());
            }
            let bf = resolve_builtin_fn(name, args.len())?;
            if matches!(bf, BuiltinFn::IsDefined) {
                let enter_idx = code.len();
                code.push(OpCode::EnterDefinedScope(0));
                tasks.push(CompileTask::IsDefinedAfterArg { enter_idx });
                if let Some(arg) = args.first() {
                    tasks.push(CompileTask::Eval(arg));
                }
            } else {
                tasks.push(CompileTask::EmitCall(bf, args.len()));
                for a in args.iter().rev() {
                    tasks.push(CompileTask::Eval(a));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::dsl) fn compile_expr(
    expr: &Expr,
    locals: &HashMap<String, usize>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
) -> Result<(), FitError> {
    compile_expr_resolved(expr, &LocalResolver::Flat(locals), params, code, MAX_TOTAL_OPCODES)
}

pub(in crate::dsl) fn compile_expr_resolved(
    expr: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
    max_opcodes: usize,
) -> Result<(), FitError> {
    compile_expr_resolved_inner(expr, locals, params, code, max_opcodes, false)
}

fn compile_expr_resolved_inner(
    expr: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
    max_opcodes: usize,
    allow_curve_eval: bool,
) -> Result<(), FitError> {
    let mut tasks = vec![CompileTask::Eval(expr)];
    while let Some(task) = tasks.pop() {
        check_opcode_budget(code, max_opcodes)?;
        match task {
            CompileTask::Eval(node) => schedule_expr_compile(node, locals, params, code, &mut tasks, allow_curve_eval)?,
            CompileTask::EmitUnary(kind) => code.push(OpCode::Unary(kind)),
            CompileTask::EmitBinary(kind) => code.push(OpCode::Binary(kind)),
            CompileTask::EmitCall(bf, arity) => code.push(OpCode::CallBuiltin(bf, arity)),
            CompileTask::EmitEvalCurveY => code.push(OpCode::EvalCurveY),
            CompileTask::EmitSolveCurveX => code.push(OpCode::SolveCurveX),
            CompileTask::AndAfterLhs { rhs } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                tasks.push(CompileTask::AndAfterRhs { jz_idx });
                tasks.push(CompileTask::Eval(rhs));
            }
            CompileTask::AndAfterRhs { jz_idx } => {
                code.push(OpCode::PushConst(0.0));
                code.push(OpCode::Binary(BinaryKind::Ne));
                let jump_end = code.len();
                code.push(OpCode::Jump(0));
                let false_target = code.len();
                code.push(OpCode::PushConst(0.0));
                let end_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(false_target);
                code[jump_end] = OpCode::Jump(end_target);
            }
            CompileTask::OrAfterLhs { rhs } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                code.push(OpCode::PushConst(1.0));
                let jump_end = code.len();
                code.push(OpCode::Jump(0));
                let rhs_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(rhs_target);
                tasks.push(CompileTask::OrAfterRhs { jump_end });
                tasks.push(CompileTask::Eval(rhs));
            }
            CompileTask::OrAfterRhs { jump_end } => {
                code.push(OpCode::PushConst(0.0));
                code.push(OpCode::Binary(BinaryKind::Ne));
                let end_target = code.len();
                code[jump_end] = OpCode::Jump(end_target);
            }
            CompileTask::IfAfterCond { then_br, else_br } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                tasks.push(CompileTask::IfAfterThen { else_br, jz_idx });
                tasks.push(CompileTask::Eval(then_br));
            }
            CompileTask::IfAfterThen { else_br, jz_idx } => {
                let jump_end = code.len();
                code.push(OpCode::Jump(0));
                let else_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(else_target);
                tasks.push(CompileTask::IfAfterElse { jump_end });
                tasks.push(CompileTask::Eval(else_br));
            }
            CompileTask::IfAfterElse { jump_end } => {
                let end_target = code.len();
                code[jump_end] = OpCode::Jump(end_target);
            }
            CompileTask::IsDefinedAfterArg { enter_idx } => {
                code.push(OpCode::ExitDefinedScope);
                code.push(OpCode::CallBuiltin(BuiltinFn::IsDefined, 1));
                let after_idx = code.len();
                code[enter_idx] = OpCode::EnterDefinedScope(after_idx);
            }
        }
    }
    check_opcode_budget(code, max_opcodes)
}

pub(in crate::dsl) fn compile_ident(
    name: &str,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
) -> Result<(), FitError> {
    if let Some(slot) = locals.slot(name) {
        code.push(OpCode::LoadLocal(slot));
        return Ok(());
    }
    let u = name.to_ascii_uppercase();
    match u.as_str() {
        "X" | "T" => code.push(OpCode::LoadPredictor(0)),
        "Y" => code.push(OpCode::LoadY),
        "DS" | "DATASET" => code.push(OpCode::LoadDatasetIndex),
        "PI" => code.push(OpCode::PushConst(core::f64::consts::PI)),
        "E" => code.push(OpCode::PushConst(core::f64::consts::E)),
        "LN2" => code.push(OpCode::PushConst(core::f64::consts::LN_2)),
        "LN10" => code.push(OpCode::PushConst(core::f64::consts::LN_10)),
        "SQRT2" => code.push(OpCode::PushConst(core::f64::consts::SQRT_2)),
        "INF" => code.push(OpCode::PushConst(f64::INFINITY)),
        "NAN" => code.push(OpCode::PushConst(f64::NAN)),
        _ => {
            if let Some(idx) = parse_predictor_index(name) {
                code.push(OpCode::LoadPredictor(idx));
                return Ok(());
            }
            if let Some(&pidx) = params.get(name) {
                code.push(OpCode::LoadParam(pidx));
                return Ok(());
            }
            return Err(FitError::DslCompileError { message: format!("Unknown symbol `{name}`"), line: 1, column: 1 });
        }
    }
    Ok(())
}

pub(in crate::dsl) fn resolve_builtin_fn(name: &str, argc: usize) -> Result<BuiltinFn, FitError> {
    let u = name.to_ascii_lowercase();
    if u == "norminv" {
        return match argc {
            1 => Ok(BuiltinFn::NormInv),
            3 => Ok(BuiltinFn::NormInv3),
            _ => Err(FitError::DslCompileError { message: format!("Function `{name}` expects 1 or 3 argument(s), got {argc}"), line: 1, column: 1 }),
        };
    }
    if u == "round" {
        return match argc {
            1 | 2 => Ok(BuiltinFn::Round),
            _ => Err(FitError::DslCompileError { message: format!("Function `{name}` expects 1 or 2 argument(s), got {argc}"), line: 1, column: 1 }),
        };
    }
    if u == "min" || u == "max" {
        if argc < 2 {
            return Err(FitError::DslCompileError {
                message: format!("Function `{name}` expects at least 2 argument(s), got {argc}"),
                line: 1,
                column: 1,
            });
        }
        return Ok(if u == "min" { BuiltinFn::Min } else { BuiltinFn::Max });
    }
    let (bf, expected_arity) = match u.as_str() {
        "abs" => (BuiltinFn::Abs, 1),
        "sqrt" => (BuiltinFn::Sqrt, 1),
        "sqr" => (BuiltinFn::Sqr, 1),
        "cbrt" => (BuiltinFn::Cbrt, 1),
        "exp" => (BuiltinFn::Exp, 1),
        "exp10" => (BuiltinFn::Exp10, 1),
        "ln" | "loge" => (BuiltinFn::Ln, 1),
        "log" | "log10" => (BuiltinFn::Log10, 1),
        "log2" => (BuiltinFn::Log2, 1),
        "sin" => (BuiltinFn::Sin, 1),
        "cos" => (BuiltinFn::Cos, 1),
        "tan" => (BuiltinFn::Tan, 1),
        "asin" | "arcsin" => (BuiltinFn::Asin, 1),
        "acos" | "arccos" => (BuiltinFn::Acos, 1),
        "atan" | "arctan" => (BuiltinFn::Atan, 1),
        "atan2" | "arctan2" => (BuiltinFn::Atan2, 2),
        "sinh" => (BuiltinFn::Sinh, 1),
        "cosh" => (BuiltinFn::Cosh, 1),
        "tanh" => (BuiltinFn::Tanh, 1),
        "asinh" | "arcsinh" => (BuiltinFn::Asinh, 1),
        "acosh" | "arccosh" => (BuiltinFn::Acosh, 1),
        "atanh" | "arctanh" => (BuiltinFn::Atanh, 1),
        "floor" => (BuiltinFn::Floor, 1),
        "ceil" => (BuiltinFn::Ceil, 1),
        "trunc" | "int" => (BuiltinFn::Trunc, 1),
        "sign" | "sgn" => (BuiltinFn::Sign, 1),
        "mod" => (BuiltinFn::Mod, 2),
        "clamp" => (BuiltinFn::Clamp, 3),
        "hypot" => (BuiltinFn::Hypot, 2),
        "pow" => (BuiltinFn::PowFn, 2),
        "fact" => (BuiltinFn::Fact, 1),
        "logit" => (BuiltinFn::Logit, 1),
        "probit" => (BuiltinFn::Probit, 1),
        "perc" => (BuiltinFn::Perc, 1),
        "deg" => (BuiltinFn::Deg, 1),
        "rad" => (BuiltinFn::Rad, 1),
        "is_defined" => (BuiltinFn::IsDefined, 1),
        "erf" => (BuiltinFn::Erf, 1),
        "erfc" => (BuiltinFn::Erfc, 1),
        "ibeta" => (BuiltinFn::IBeta, 3),
        "igamma" => (BuiltinFn::IGamma, 2),
        "igammac" => (BuiltinFn::IGammaC, 2),
        "normcdf" | "phi" | "zdist" => (BuiltinFn::NormCdf, 1),
        "zinv" => (BuiltinFn::NormInv, 1),
        "zpdf" => (BuiltinFn::NormPdf, 1),
        "normdist" => (BuiltinFn::NormDist3, 3),
        "normpdf" => (BuiltinFn::NormPdf3, 3),
        "tdist" => (BuiltinFn::TDist, 2),
        "tinv" => (BuiltinFn::TInv, 2),
        "tpdf" => (BuiltinFn::TPdf, 2),
        "fdist" => (BuiltinFn::FDist, 3),
        "finv" => (BuiltinFn::FInv, 3),
        "fpdf" => (BuiltinFn::FPdf, 3),
        "chidist" => (BuiltinFn::ChiDist, 2),
        "chisqc" => (BuiltinFn::ChiSqC, 2),
        "chiinv" => (BuiltinFn::ChiInv, 2),
        "binomial" => (BuiltinFn::Binomial, 3),
        "gamma" => (BuiltinFn::Gamma, 1),
        "lngamma" | "lgamma" | "gammaln" => (BuiltinFn::LnGamma, 1),
        "beta" => (BuiltinFn::Beta, 2),
        "digamma" | "psi" => (BuiltinFn::Digamma, 1),
        "besselj0" | "j0" => (BuiltinFn::BesselJ0, 1),
        "besselj1" | "j1" => (BuiltinFn::BesselJ1, 1),
        "besselj" | "besseljn" | "jn" => (BuiltinFn::BesselJn, 2),
        "bessely0" | "y0" => (BuiltinFn::BesselY0, 1),
        "bessely1" | "y1" => (BuiltinFn::BesselY1, 1),
        "bessely" | "besselyn" | "yn" => (BuiltinFn::BesselYn, 2),
        "besseli0" | "i0" => (BuiltinFn::BesselI0, 1),
        "besseli1" | "i1" => (BuiltinFn::BesselI1, 1),
        "besseli" | "besselin" => (BuiltinFn::BesselIn, 2),
        "besselk0" | "k0" => (BuiltinFn::BesselK0, 1),
        "besselk1" | "k1" => (BuiltinFn::BesselK1, 1),
        "besselk" | "besselkn" => (BuiltinFn::BesselKn, 2),
        "hyp1f1" | "m" | "hypgeometricm" => (BuiltinFn::Hyp1F1, 3),
        "hyp2f1" | "hypgeometricf" => (BuiltinFn::Hyp2F1, 4),
        "confluent_u" | "u" | "hypgeometricu" => (BuiltinFn::ConfluentU, 3),
        _ => {
            return Err(FitError::DslCompileError { message: format!("Unknown function `{name}`"), line: 1, column: 1 });
        }
    };
    if argc != expected_arity {
        return Err(FitError::DslCompileError {
            message: format!("Function `{name}` expects {expected_arity} argument(s), got {argc}"),
            line: 1,
            column: 1,
        });
    }
    Ok(bf)
}
