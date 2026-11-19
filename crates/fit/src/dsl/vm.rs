//! Bounded stack VM that evaluates compiled bytecode and built-in functions.
//!
//! References:
//! - Abramowitz, M. & Stegun, I. A. (eds.) (1964). *Handbook of Mathematical Functions with Formulas, Graphs, and Mathematical Tables*. National Bureau of Standards.
//! - Olver, F. W. J. et al. (eds.) (2010). *NIST Handbook of Mathematical Functions* (NIST Digital Library of Mathematical Functions, <https://dlmf.nist.gov/>). Cambridge University Press.

use curvebook_math::dist::{norm_cdf, norm_ppf};
use curvebook_math::special::{beta, digamma, erf, erfc, gamma, ln_gamma};

use crate::dsl::ast::{BinaryKind, BuiltinFn, CompiledProgram, OpCode, UnaryKind};
use crate::dsl::limits::MAX_VM_STEPS;
use crate::dsl::special::{
    bessel_i0, bessel_i1, bessel_j0, bessel_j1, bessel_jn, bessel_k0, bessel_k1, bessel_y0, bessel_y1, bessel_yn, confluent_u, hyp1f1, hyp2f1,
    parse_bessel_order,
};

pub(in crate::dsl) fn eval_bytecode(prog: &CompiledProgram, x: &[f64], y: f64, params: &[f64], ds_idx: usize) -> Option<f64> {
    let mut locals = vec![None; prog.local_count];
    eval_bytecode_with_locals(prog, x, y, params, ds_idx, &mut locals)
}

pub(in crate::dsl) fn eval_bytecode_with_locals(
    prog: &CompiledProgram,
    x: &[f64],
    y: f64,
    params: &[f64],
    ds_idx: usize,
    locals: &mut [Option<f64>],
) -> Option<f64> {
    let mut stack: Vec<f64> = Vec::with_capacity(32);
    eval_bytecode_with_bufs(prog, x, y, params, ds_idx, locals, &mut stack)
}

pub(in crate::dsl) fn eval_bytecode_with_bufs(
    prog: &CompiledProgram,
    x: &[f64],
    y: f64,
    params: &[f64],
    ds_idx: usize,
    locals: &mut [Option<f64>],
    stack: &mut Vec<f64>,
) -> Option<f64> {
    stack.clear();
    let mut ip = 0_usize;
    let mut steps = 0_usize;

    while ip < prog.code.len() {
        steps += 1;
        if steps > MAX_VM_STEPS {
            return None;
        }
        match prog.code[ip] {
            OpCode::PushConst(v) => stack.push(v),
            OpCode::LoadPredictor(idx) => {
                stack.push(*x.get(idx)?);
            }
            OpCode::LoadY => stack.push(y),
            OpCode::LoadDatasetIndex => stack.push(ds_idx as f64),
            OpCode::LoadParam(idx) => stack.push(*params.get(idx)?),
            OpCode::LoadLocal(slot) => stack.push(locals.get(slot).copied().flatten()?),
            OpCode::StoreLocal(slot) => {
                let v = stack.pop()?;
                *locals.get_mut(slot)? = Some(v);
            }
            OpCode::Unary(kind) => {
                let v = stack.pop()?;
                stack.push(match kind {
                    UnaryKind::Neg => -v,
                    UnaryKind::Not => {
                        if v == 0.0 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                });
            }
            OpCode::Binary(kind) => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let v = match kind {
                    BinaryKind::Add => a + b,
                    BinaryKind::Sub => a - b,
                    BinaryKind::Mul => a * b,
                    BinaryKind::Div => {
                        if b == 0.0 {
                            return None;
                        }
                        a / b
                    }
                    BinaryKind::Rem => {
                        if b == 0.0 {
                            return None;
                        }
                        a % b
                    }
                    BinaryKind::Pow => a.powf(b),
                    BinaryKind::Eq => f64::from((a - b).abs() == 0.0),
                    BinaryKind::Ne => f64::from(a != b),
                    BinaryKind::Lt => f64::from(a < b),
                    BinaryKind::Le => f64::from(a <= b),
                    BinaryKind::Gt => f64::from(a > b),
                    BinaryKind::Ge => f64::from(a >= b),
                };
                stack.push(v);
            }
            OpCode::CallBuiltin(bf, arity) => {
                if stack.len() < arity {
                    return None;
                }
                let base = stack.len() - arity;
                let res = eval_builtin(bf, &stack[base..])?;
                stack.truncate(base);
                stack.push(res);
            }
            OpCode::Jump(target) => {
                ip = target;
                continue;
            }
            OpCode::JumpIfZero(target) => {
                let cond = stack.pop()?;
                if cond == 0.0 || !cond.is_finite() {
                    ip = target;
                    continue;
                }
            }
            OpCode::Pop => {
                stack.pop()?;
            }
            OpCode::Halt => break,
        }
        ip += 1;
    }
    let out = stack.pop()?;
    if out.is_finite() { Some(out) } else { None }
}

pub(in crate::dsl) fn eval_builtin(bf: BuiltinFn, a: &[f64]) -> Option<f64> {
    let val = match bf {
        BuiltinFn::Abs => a[0].abs(),
        BuiltinFn::Sqrt => {
            if a[0] < 0.0 {
                return None;
            }
            a[0].sqrt()
        }
        BuiltinFn::Cbrt => a[0].cbrt(),
        BuiltinFn::Exp => {
            if a[0] > 709.0 {
                return None;
            }
            a[0].exp()
        }
        BuiltinFn::Exp10 => {
            if a[0] > 308.0 {
                return None;
            }
            10.0_f64.powf(a[0])
        }
        BuiltinFn::Ln => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].ln()
        }
        BuiltinFn::Log10 => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].log10()
        }
        BuiltinFn::Log2 => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].log2()
        }
        BuiltinFn::Sin => a[0].sin(),
        BuiltinFn::Cos => a[0].cos(),
        BuiltinFn::Tan => a[0].tan(),
        BuiltinFn::Asin => a[0].asin(),
        BuiltinFn::Acos => a[0].acos(),
        BuiltinFn::Atan => a[0].atan(),
        BuiltinFn::Atan2 => a[0].atan2(a[1]),
        BuiltinFn::Sinh => a[0].sinh(),
        BuiltinFn::Cosh => a[0].cosh(),
        BuiltinFn::Tanh => a[0].tanh(),
        BuiltinFn::Asinh => a[0].asinh(),
        BuiltinFn::Acosh => a[0].acosh(),
        BuiltinFn::Atanh => a[0].atanh(),
        BuiltinFn::Floor => a[0].floor(),
        BuiltinFn::Ceil => a[0].ceil(),
        BuiltinFn::Round => a[0].round(),
        BuiltinFn::Trunc => a[0].trunc(),
        BuiltinFn::Sign => {
            if a[0].is_nan() {
                return None;
            }
            if a[0] > 0.0 {
                1.0
            } else if a[0] < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        BuiltinFn::Min => {
            if a[0].is_nan() || a[1].is_nan() {
                return None;
            }
            a[0].min(a[1])
        }
        BuiltinFn::Max => {
            if a[0].is_nan() || a[1].is_nan() {
                return None;
            }
            a[0].max(a[1])
        }
        BuiltinFn::Clamp => {
            if a[0].is_nan() || a[1].is_nan() || a[2].is_nan() {
                f64::NAN
            } else {
                let lo = a[1].min(a[2]);
                let hi = a[1].max(a[2]);
                if lo <= hi { a[0].clamp(lo, hi) } else { f64::NAN }
            }
        }
        BuiltinFn::Hypot => a[0].hypot(a[1]),
        BuiltinFn::PowFn => a[0].powf(a[1]),
        BuiltinFn::Erf => erf(a[0]),
        BuiltinFn::Erfc => erfc(a[0]),
        BuiltinFn::NormCdf => norm_cdf(a[0]),
        BuiltinFn::NormInv => norm_ppf(a[0]).ok()?,
        BuiltinFn::Gamma => gamma(a[0]).ok()?,
        BuiltinFn::LnGamma => ln_gamma(a[0]).ok()?,
        BuiltinFn::Beta => beta(a[0], a[1]).ok()?,
        BuiltinFn::Digamma => digamma(a[0]).ok()?,
        BuiltinFn::BesselJ0 => bessel_j0(a[0]),
        BuiltinFn::BesselJ1 => bessel_j1(a[0]),
        BuiltinFn::BesselJn => {
            let n = parse_bessel_order(a[0])?;
            bessel_jn(n, a[1])
        }
        BuiltinFn::BesselY0 => bessel_y0(a[0])?,
        BuiltinFn::BesselY1 => bessel_y1(a[0])?,
        BuiltinFn::BesselYn => {
            let n = parse_bessel_order(a[0])?;
            bessel_yn(n, a[1])?
        }
        BuiltinFn::BesselI0 => bessel_i0(a[0])?,
        BuiltinFn::BesselI1 => bessel_i1(a[0])?,
        BuiltinFn::BesselK0 => bessel_k0(a[0])?,
        BuiltinFn::BesselK1 => bessel_k1(a[0])?,
        BuiltinFn::Hyp1F1 => hyp1f1(a[0], a[1], a[2])?,
        BuiltinFn::Hyp2F1 => hyp2f1(a[0], a[1], a[2], a[3])?,
        BuiltinFn::ConfluentU => confluent_u(a[0], a[1], a[2])?,
    };
    if val.is_finite() { Some(val) } else { None }
}
