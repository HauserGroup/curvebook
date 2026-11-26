//! Bounded stack VM that evaluates compiled bytecode and built-in functions.
//!
//! References:
//! - Abramowitz, M. & Stegun, I. A. (eds.) (1964). *Handbook of Mathematical Functions with Formulas, Graphs, and Mathematical Tables*. National Bureau of Standards.
//! - Olver, F. W. J. et al. (eds.) (2010). *NIST Handbook of Mathematical Functions* (NIST Digital Library of Mathematical Functions, <https://dlmf.nist.gov/>). Cambridge University Press.

use curvebook_math::dist::{
    chi2_cdf, chi2_isf, chi2_sf, f_isf, f_pdf, f_sf, norm_cdf, norm_pdf, norm_ppf, student_t_pdf, student_t_sf, student_t_two_sided_isf,
};
use curvebook_math::special::{beta, digamma, erf, erfc, gamma, reg_beta, reg_gamma_p, reg_gamma_q};

use crate::dsl::ast::{BinaryKind, BuiltinFn, CompiledProgram, OpCode, UnaryKind};
use crate::dsl::limits::MAX_VM_STEPS;
use crate::dsl::special::{
    bessel_i0_counted, bessel_i1_counted, bessel_in_counted, bessel_j0, bessel_j1, bessel_jn_counted, bessel_k0, bessel_k1, bessel_kn_counted,
    bessel_y0, bessel_y1, bessel_yn_counted, binomial_upper_tail, confluent_u_counted, factorial_f64, hyp1f1_counted, hyp2f1_counted, ln_gamma_abs,
    parse_bessel_order,
};

pub(in crate::dsl) struct VmInput<'a> {
    pub(in crate::dsl) x: &'a [f64],
    pub(in crate::dsl) y: f64,
    pub(in crate::dsl) params: &'a [f64],
    pub(in crate::dsl) ds_idx: usize,
}

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
    eval_bytecode_with_budget(prog, &VmInput { x, y, params, ds_idx }, locals, stack, MAX_VM_STEPS).0
}

fn catch_defined_fault(try_scopes: &mut Vec<(usize, usize)>, stack: &mut Vec<f64>, ip: &mut usize) -> bool {
    if let Some((saved_depth, target)) = try_scopes.pop() {
        stack.truncate(saved_depth);
        stack.push(0.0);
        *ip = target;
        true
    } else {
        false
    }
}

pub(in crate::dsl) fn eval_bytecode_with_budget(
    prog: &CompiledProgram,
    input: &VmInput<'_>,
    locals: &mut [Option<f64>],
    stack: &mut Vec<f64>,
    max_steps: usize,
) -> (Option<f64>, usize) {
    stack.clear();
    let mut try_scopes: Vec<(usize, usize)> = Vec::new();
    let mut ip = 0_usize;
    let mut steps = 0_usize;

    while ip < prog.code.len() {
        steps += 1;
        if steps > max_steps {
            return (None, steps);
        }
        match prog.code[ip] {
            OpCode::PushConst(v) => stack.push(v),
            OpCode::LoadPredictor(idx) => {
                let Some(&val) = input.x.get(idx) else {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                };
                stack.push(val);
            }
            OpCode::LoadY => stack.push(input.y),
            OpCode::LoadDatasetIndex => stack.push(input.ds_idx as f64),
            OpCode::LoadParam(idx) => {
                let Some(&val) = input.params.get(idx) else {
                    return (None, steps);
                };
                stack.push(val);
            }
            OpCode::LoadLocal(slot) => {
                let Some(val) = locals.get(slot).copied().flatten() else {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                };
                stack.push(val);
            }
            OpCode::StoreLocal(slot) => {
                let (Some(v), Some(dst)) = (stack.pop(), locals.get_mut(slot)) else {
                    return (None, steps);
                };
                *dst = Some(v);
            }
            OpCode::Unary(kind) => {
                let Some(v) = stack.pop() else {
                    return (None, steps);
                };
                if v.is_nan() {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                }
                let out = match kind {
                    UnaryKind::Neg => -v,
                    UnaryKind::Not => {
                        if v == 0.0 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                };
                stack.push(out);
            }
            OpCode::Binary(kind) => {
                let (Some(b), Some(a)) = (stack.pop(), stack.pop()) else {
                    return (None, steps);
                };
                let Some(v) = eval_binary_op(kind, a, b) else {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                };
                stack.push(v);
            }
            OpCode::CallBuiltin(bf, arity) => {
                if stack.len() < arity {
                    return (None, steps);
                }
                let base = stack.len() - arity;
                let max_work = max_steps.saturating_sub(steps).saturating_add(1);
                let (res_opt, work) = eval_builtin_counted(bf, &stack[base..], max_work);
                steps = steps.saturating_add(work.saturating_sub(1));
                if steps > max_steps {
                    return (None, steps);
                }
                let Some(res) = res_opt else {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                };
                stack.truncate(base);
                stack.push(res);
            }
            OpCode::Jump(target) => {
                ip = target;
                continue;
            }
            OpCode::JumpIfZero(target) => {
                let Some(cond) = stack.pop() else {
                    return (None, steps);
                };
                if cond.is_nan() {
                    if catch_defined_fault(&mut try_scopes, stack, &mut ip) {
                        continue;
                    }
                    return (None, steps);
                }
                if cond == 0.0 {
                    ip = target;
                    continue;
                }
            }
            OpCode::Pop => {
                if stack.pop().is_none() {
                    return (None, steps);
                }
            }
            OpCode::EnterDefinedScope(target) => try_scopes.push((stack.len(), target)),
            OpCode::ExitDefinedScope => {
                let _ = try_scopes.pop();
            }
            OpCode::Halt => break,
        }
        ip += 1;
    }
    let Some(out) = stack.pop() else {
        return (None, steps);
    };
    let final_val = if out.is_finite() { Some(out) } else { None };
    (final_val, steps)
}

pub(in crate::dsl) fn eval_binary_op(kind: BinaryKind, a: f64, b: f64) -> Option<f64> {
    if a.is_nan() || b.is_nan() {
        return None;
    }
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
        BinaryKind::Eq => f64::from(a == b),
        BinaryKind::Ne => f64::from(a != b),
        BinaryKind::Lt => f64::from(a < b),
        BinaryKind::Le => f64::from(a <= b),
        BinaryKind::Gt => f64::from(a > b),
        BinaryKind::Ge => f64::from(a >= b),
    };
    if v.is_nan() { None } else { Some(v) }
}

pub(in crate::dsl) fn eval_builtin_counted(bf: BuiltinFn, a: &[f64], max_work: usize) -> (Option<f64>, usize) {
    if matches!(bf, BuiltinFn::IsDefined) {
        return (Some(if a.first().is_some_and(|v| v.is_finite()) { 1.0 } else { 0.0 }), 1);
    }
    if a.iter().any(|v| v.is_nan()) {
        return (None, 1);
    }
    let allow_inf = matches!(
        bf,
        BuiltinFn::Abs
            | BuiltinFn::Sqrt
            | BuiltinFn::Sqr
            | BuiltinFn::Cbrt
            | BuiltinFn::Exp
            | BuiltinFn::Exp10
            | BuiltinFn::Sinh
            | BuiltinFn::Cosh
            | BuiltinFn::Floor
            | BuiltinFn::Ceil
            | BuiltinFn::Round
            | BuiltinFn::Trunc
            | BuiltinFn::Sign
            | BuiltinFn::Min
            | BuiltinFn::Max
            | BuiltinFn::Clamp
            | BuiltinFn::Hypot
            | BuiltinFn::PowFn
            | BuiltinFn::Atan
            | BuiltinFn::Tanh
            | BuiltinFn::Erf
            | BuiltinFn::Erfc
            | BuiltinFn::NormCdf
            | BuiltinFn::NormPdf
            | BuiltinFn::NormDist3
            | BuiltinFn::NormPdf3
    );
    let mut work = 1_usize;
    let val = match bf {
        BuiltinFn::Abs => a[0].abs(),
        BuiltinFn::Sqrt => {
            if a[0] < 0.0 {
                return (None, 1);
            }
            a[0].sqrt()
        }
        BuiltinFn::Sqr => a[0] * a[0],
        BuiltinFn::Cbrt => a[0].cbrt(),
        BuiltinFn::Exp => a[0].exp(),
        BuiltinFn::Exp10 => 10.0_f64.powf(a[0]),
        BuiltinFn::Ln => {
            if a[0] <= 0.0 || !a[0].is_finite() {
                return (None, 1);
            }
            a[0].ln()
        }
        BuiltinFn::Log10 => {
            if a[0] <= 0.0 || !a[0].is_finite() {
                return (None, 1);
            }
            a[0].log10()
        }
        BuiltinFn::Log2 => {
            if a[0] <= 0.0 || !a[0].is_finite() {
                return (None, 1);
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
            if a[0] > 0.0 {
                1.0
            } else if a[0] < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        BuiltinFn::Mod => {
            if a[1] == 0.0 {
                return (None, 1);
            }
            a[0] % a[1]
        }
        BuiltinFn::Min => a.iter().copied().reduce(f64::min).unwrap_or(f64::NAN),
        BuiltinFn::Max => a.iter().copied().reduce(f64::max).unwrap_or(f64::NAN),
        BuiltinFn::Clamp => {
            let lo = a[1].min(a[2]);
            let hi = a[1].max(a[2]);
            if lo <= hi { a[0].clamp(lo, hi) } else { return (None, 1) }
        }
        BuiltinFn::Hypot => a[0].hypot(a[1]),
        BuiltinFn::PowFn => a[0].powf(a[1]),
        BuiltinFn::Perc => a[0] / 100.0,
        BuiltinFn::Deg => a[0].to_degrees(),
        BuiltinFn::Rad => a[0].to_radians(),
        BuiltinFn::IsDefined => unreachable_is_defined(a[0]),
        other => {
            let (opt, w) = eval_special_or_dist_builtin(other, a, max_work);
            work = w;
            let Some(v) = opt else {
                return (None, work);
            };
            v
        }
    };
    if val.is_nan() {
        return (None, work);
    }
    let out = if allow_inf || val.is_finite() { Some(val) } else { None };
    (out, work)
}

fn eval_special_or_dist_builtin(bf: BuiltinFn, a: &[f64], max_work: usize) -> (Option<f64>, usize) {
    match bf {
        BuiltinFn::BesselJn => {
            let Some(n) = parse_bessel_order(a[0]) else {
                return (None, 1);
            };
            bessel_jn_counted(n, a[1], max_work)
        }
        BuiltinFn::BesselYn => {
            let Some(n) = parse_bessel_order(a[0]) else {
                return (None, 1);
            };
            bessel_yn_counted(n, a[1], max_work)
        }
        BuiltinFn::BesselI0 => bessel_i0_counted(a[0], max_work),
        BuiltinFn::BesselI1 => bessel_i1_counted(a[0], max_work),
        BuiltinFn::BesselIn => {
            let Some(n) = parse_bessel_order(a[0]) else {
                return (None, 1);
            };
            bessel_in_counted(n, a[1], max_work)
        }
        BuiltinFn::BesselKn => {
            let Some(n) = parse_bessel_order(a[0]) else {
                return (None, 1);
            };
            bessel_kn_counted(n, a[1], max_work)
        }
        BuiltinFn::Hyp1F1 => hyp1f1_counted(a[0], a[1], a[2], max_work),
        BuiltinFn::Hyp2F1 => hyp2f1_counted(a[0], a[1], a[2], a[3], max_work),
        BuiltinFn::ConfluentU => confluent_u_counted(a[0], a[1], a[2], max_work),
        other => (eval_simple_special_or_dist(other, a), 1),
    }
}

fn eval_simple_special_or_dist(bf: BuiltinFn, a: &[f64]) -> Option<f64> {
    let val = match bf {
        BuiltinFn::Fact => factorial_f64(a[0])?,
        BuiltinFn::Logit => {
            if !(a[0] > 0.0 && a[0] < 1.0) {
                return None;
            }
            a[0].ln() - (-a[0]).ln_1p()
        }
        BuiltinFn::Probit => 5.0 + norm_ppf(a[0]).ok()?,
        BuiltinFn::Erf => erf(a[0]),
        BuiltinFn::Erfc => erfc(a[0]),
        BuiltinFn::IBeta => reg_beta(a[2], a[0], a[1]).ok()?,
        BuiltinFn::IGamma => reg_gamma_p(a[0], a[1]).ok()?,
        BuiltinFn::IGammaC => reg_gamma_q(a[0], a[1]).ok()?,
        BuiltinFn::NormCdf => norm_cdf(a[0]),
        BuiltinFn::NormPdf => norm_pdf(a[0]),
        BuiltinFn::NormInv => norm_ppf(a[0]).ok()?,
        BuiltinFn::NormDist3 => {
            if !a[1].is_finite() || !a[2].is_finite() || a[2] <= 0.0 {
                return None;
            }
            norm_cdf((a[0] - a[1]) / a[2])
        }
        BuiltinFn::NormInv3 => {
            if !a[1].is_finite() || !a[2].is_finite() || a[2] <= 0.0 {
                return None;
            }
            a[1] + a[2] * norm_ppf(a[0]).ok()?
        }
        BuiltinFn::NormPdf3 => {
            if !a[1].is_finite() || !a[2].is_finite() || a[2] <= 0.0 {
                return None;
            }
            norm_pdf((a[0] - a[1]) / a[2]) / a[2]
        }
        BuiltinFn::TDist => student_t_sf(a[0], a[1]).ok()?,
        BuiltinFn::TInv => student_t_two_sided_isf(a[0], a[1]).ok()?,
        BuiltinFn::TPdf => student_t_pdf(a[0], a[1]).ok()?,
        BuiltinFn::FDist => f_sf(a[0], a[1], a[2]).ok()?,
        BuiltinFn::FInv => f_isf(a[0], a[1], a[2]).ok()?,
        BuiltinFn::FPdf => f_pdf(a[0], a[1], a[2]).ok()?,
        BuiltinFn::ChiDist => chi2_sf(a[0], a[1]).ok()?,
        BuiltinFn::ChiSqC => chi2_cdf(a[0], a[1]).ok()?,
        BuiltinFn::ChiInv => chi2_isf(a[0], a[1]).ok()?,
        BuiltinFn::Binomial => binomial_upper_tail(a[0], a[1], a[2])?,
        BuiltinFn::Gamma => gamma(a[0]).ok()?,
        BuiltinFn::LnGamma => ln_gamma_abs(a[0])?,
        BuiltinFn::Beta => beta(a[0], a[1]).ok()?,
        BuiltinFn::Digamma => digamma(a[0]).ok()?,
        BuiltinFn::BesselJ0 => bessel_j0(a[0]),
        BuiltinFn::BesselJ1 => bessel_j1(a[0]),
        BuiltinFn::BesselY0 => bessel_y0(a[0])?,
        BuiltinFn::BesselY1 => bessel_y1(a[0])?,
        BuiltinFn::BesselK0 => bessel_k0(a[0])?,
        BuiltinFn::BesselK1 => bessel_k1(a[0])?,
        _ => return None,
    };
    Some(val)
}

fn unreachable_is_defined(x: f64) -> f64 {
    if x.is_finite() { 1.0 } else { 0.0 }
}
