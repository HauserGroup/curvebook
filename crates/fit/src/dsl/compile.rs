//! Bytecode compilation of expression trees with opcode budgets and identifier resolution.
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use std::collections::HashMap;

use crate::dsl::ast::{BinaryKind, BinaryOp, BuiltinFn, CompiledProgram, Expr, OpCode, UnaryKind, UnaryOp};
use crate::dsl::limits::MAX_TOTAL_OPCODES;
use crate::dsl::parser::LocalResolver;
use crate::error::FitError;

pub(in crate::dsl) fn check_opcode_budget(code: &[OpCode], max_opcodes: usize) -> Result<(), FitError> {
    if code.len() > max_opcodes {
        return Err(FitError::DslCompileError {
            message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
            line: 1,
            column: 1,
        });
    }
    Ok(())
}

pub(in crate::dsl) fn compile_program_refs(
    locals: &[(String, Expr)],
    target: &Expr,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut code = Vec::new();
    for (lname, rhs) in locals {
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
    let resolver = LocalResolver::Flat(&local_map);
    compile_expr_resolved(target, &resolver, params, &mut code, max_opcodes)?;
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
    let mut code = Vec::new();
    compile_expr_resolved(target, locals, params, &mut code, max_opcodes)?;
    code.push(OpCode::Halt);
    check_opcode_budget(&code, max_opcodes)?;
    Ok(CompiledProgram { code, local_count: 0 })
}

pub(in crate::dsl) enum CompileTask<'a> {
    Eval(&'a Expr),
    EmitUnary(UnaryKind),
    EmitBinary(BinaryKind),
    EmitCall(BuiltinFn, usize),
    AndAfterLhs { rhs: &'a Expr },
    AndAfterRhs { jz_idx: usize },
    OrAfterLhs { rhs: &'a Expr },
    OrAfterRhs { jump_end: usize },
    IfAfterCond { then_br: &'a Expr, else_br: &'a Expr },
    IfAfterThen { else_br: &'a Expr, jz_idx: usize },
    IfAfterElse { jump_end: usize },
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
            let (bf, expected_arity) = resolve_builtin_fn(name)?;
            if args.len() != expected_arity {
                return Err(FitError::DslCompileError {
                    message: format!("Function `{name}` expects {expected_arity} argument(s), got {}", args.len()),
                    line: 1,
                    column: 1,
                });
            }
            tasks.push(CompileTask::EmitCall(bf, args.len()));
            for a in args.iter().rev() {
                tasks.push(CompileTask::Eval(a));
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
    let mut tasks = vec![CompileTask::Eval(expr)];
    while let Some(task) = tasks.pop() {
        check_opcode_budget(code, max_opcodes)?;
        match task {
            CompileTask::Eval(node) => schedule_expr_compile(node, locals, params, code, &mut tasks)?,
            CompileTask::EmitUnary(kind) => code.push(OpCode::Unary(kind)),
            CompileTask::EmitBinary(kind) => code.push(OpCode::Binary(kind)),
            CompileTask::EmitCall(bf, arity) => code.push(OpCode::CallBuiltin(bf, arity)),
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
            if let Some(rest) = u.strip_prefix('X')
                && !rest.is_empty()
                && rest != "0"
                && let Ok(idx) = rest.parse::<usize>()
                && idx >= 1
            {
                code.push(OpCode::LoadPredictor(idx - 1));
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

pub(in crate::dsl) fn resolve_builtin_fn(name: &str) -> Result<(BuiltinFn, usize), FitError> {
    let u = name.to_ascii_lowercase();
    let res = match u.as_str() {
        "abs" => (BuiltinFn::Abs, 1),
        "sqrt" => (BuiltinFn::Sqrt, 1),
        "cbrt" => (BuiltinFn::Cbrt, 1),
        "exp" => (BuiltinFn::Exp, 1),
        "exp10" => (BuiltinFn::Exp10, 1),
        "ln" | "loge" => (BuiltinFn::Ln, 1),
        "log" | "log10" => (BuiltinFn::Log10, 1),
        "log2" => (BuiltinFn::Log2, 1),
        "sin" => (BuiltinFn::Sin, 1),
        "cos" => (BuiltinFn::Cos, 1),
        "tan" => (BuiltinFn::Tan, 1),
        "asin" => (BuiltinFn::Asin, 1),
        "acos" => (BuiltinFn::Acos, 1),
        "atan" | "arctan" => (BuiltinFn::Atan, 1),
        "atan2" => (BuiltinFn::Atan2, 2),
        "sinh" => (BuiltinFn::Sinh, 1),
        "cosh" => (BuiltinFn::Cosh, 1),
        "tanh" => (BuiltinFn::Tanh, 1),
        "asinh" => (BuiltinFn::Asinh, 1),
        "acosh" => (BuiltinFn::Acosh, 1),
        "atanh" => (BuiltinFn::Atanh, 1),
        "floor" => (BuiltinFn::Floor, 1),
        "ceil" => (BuiltinFn::Ceil, 1),
        "round" => (BuiltinFn::Round, 1),
        "trunc" => (BuiltinFn::Trunc, 1),
        "sign" | "sgn" => (BuiltinFn::Sign, 1),
        "min" => (BuiltinFn::Min, 2),
        "max" => (BuiltinFn::Max, 2),
        "clamp" => (BuiltinFn::Clamp, 3),
        "hypot" => (BuiltinFn::Hypot, 2),
        "pow" => (BuiltinFn::PowFn, 2),
        "erf" => (BuiltinFn::Erf, 1),
        "erfc" => (BuiltinFn::Erfc, 1),
        "normcdf" | "phi" => (BuiltinFn::NormCdf, 1),
        "norminv" | "probit" => (BuiltinFn::NormInv, 1),
        "gamma" => (BuiltinFn::Gamma, 1),
        "lngamma" | "lgamma" => (BuiltinFn::LnGamma, 1),
        "beta" => (BuiltinFn::Beta, 2),
        "digamma" | "psi" => (BuiltinFn::Digamma, 1),
        "besselj0" | "j0" => (BuiltinFn::BesselJ0, 1),
        "besselj1" | "j1" => (BuiltinFn::BesselJ1, 1),
        "besseljn" | "jn" => (BuiltinFn::BesselJn, 2),
        "bessely0" | "y0" => (BuiltinFn::BesselY0, 1),
        "bessely1" | "y1" => (BuiltinFn::BesselY1, 1),
        "besselyn" | "yn" => (BuiltinFn::BesselYn, 2),
        "besseli0" | "i0" => (BuiltinFn::BesselI0, 1),
        "besseli1" | "i1" => (BuiltinFn::BesselI1, 1),
        "besselk0" | "k0" => (BuiltinFn::BesselK0, 1),
        "besselk1" | "k1" => (BuiltinFn::BesselK1, 1),
        "hyp1f1" | "m" => (BuiltinFn::Hyp1F1, 3),
        "hyp2f1" => (BuiltinFn::Hyp2F1, 4),
        "confluent_u" | "u" => (BuiltinFn::ConfluentU, 3),
        _ => {
            return Err(FitError::DslCompileError { message: format!("Unknown function `{name}`"), line: 1, column: 1 });
        }
    };
    Ok(res)
}
