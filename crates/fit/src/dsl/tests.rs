//! Unit tests for private items of this module.

use std::collections::{BTreeSet, HashMap};

use crate::library::ParametricModel;

use crate::dsl::ast::{BinaryOp, DatasetFilter, Expr, UnaryOp};
use crate::dsl::compile::compile_expr;
use crate::dsl::limits::{MAX_DATASET_EXPANSION, MAX_PARSE_DEPTH};
use crate::dsl::model::CompiledDslModel;
use crate::dsl::ode::integrate_rk45;
use crate::dsl::special::{bessel_jn, bessel_yn};
use crate::dsl::statements::{collect_free_idents, expr_references_y, parse_dataset_prefix};

#[test]
fn rk45_tiny_and_non_finite_spans_do_not_panic_or_hang() {
    let tiny = integrate_rk45(0.0, 1e-10, 2.0, |_t, y| Some(-y)).unwrap();
    assert!((tiny - 2.0 * (-1e-10_f64).exp()).abs() < 1e-14);
    assert!(integrate_rk45(f64::NAN, 1.0, 1.0, |_t, y| Some(-y)).is_none());
    assert!(integrate_rk45(0.0, f64::NAN, 1.0, |_t, y| Some(-y)).is_none());
    assert!(integrate_rk45(0.0, f64::INFINITY, 1.0, |_t, y| Some(-y)).is_none());
    assert!(integrate_rk45(-1e308, 1e308, 1.0, |_t, _y| Some(0.0)).is_none());
}

#[test]
fn clamp_and_bessel_guard_extremes() {
    let clamp_m = CompiledDslModel::compile("c", "Y = clamp(X, a, b)").unwrap();
    assert_eq!(clamp_m.eval(&[1.0], &[f64::NAN, f64::NAN], 0), None);
    assert_eq!(clamp_m.eval(&[5.0], &[0.0, f64::INFINITY], 0), Some(5.0));

    assert!(bessel_jn(i32::MIN, 1.0).is_nan());
    assert!(bessel_jn(1_000_000, 1.0).is_nan());
    assert!(bessel_yn(i32::MIN, 1.0).is_none());
    assert!(bessel_yn(1_000_000, 1.0).is_none());

    let jm = CompiledDslModel::compile("jn", "Y = jn(n, X)").unwrap();
    assert_eq!(jm.eval(&[1.0], &[-1e12], 0), None);
    assert_eq!(jm.eval(&[1.0], &[f64::NAN], 0), None);
}

#[test]
fn dataset_prefix_caps_expansion_and_overflow() {
    let ok = parse_dataset_prefix("<0:9999> Y = X", 1).unwrap();
    assert!(matches!(ok.0, DatasetFilter::Include(v) if v.len() == MAX_DATASET_EXPANSION));
    assert!(parse_dataset_prefix("<0:10000> Y = X", 1).is_err());
    assert!(parse_dataset_prefix("<0:18446744073709551615> Y = X", 1).is_err());
}

#[test]
fn parser_depth_limit_and_iterative_ast_walkers_prevent_stack_overflow() {
    let ok_src = format!("Y = {}X{}", "(".repeat(MAX_PARSE_DEPTH), ")".repeat(MAX_PARSE_DEPTH));
    assert!(CompiledDslModel::compile("ok_depth", &ok_src).is_ok());

    let too_deep = format!("Y = {}X{}", "(".repeat(MAX_PARSE_DEPTH + 1), ")".repeat(MAX_PARSE_DEPTH + 1));
    assert!(CompiledDslModel::compile("too_deep", &too_deep).is_err());

    let too_many_unary = format!("Y = {}X", "-".repeat(MAX_PARSE_DEPTH + 1));
    assert!(CompiledDslModel::compile("unary_deep", &too_many_unary).is_err());

    let mut deep = Expr::Ident("Y".to_owned());
    for _ in 0..20_000 {
        deep = Expr::Unary { op: UnaryOp::Neg, rhs: Box::new(deep) };
    }
    assert!(expr_references_y(&deep));
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    collect_free_idents(&deep, &BTreeSet::new(), &mut seen, &mut order);
    let mut code = Vec::new();
    compile_expr(&deep, &HashMap::new(), &HashMap::new(), &mut code).unwrap();
    assert_eq!(code.len(), 20_001);
    drop(deep);
}

#[test]
fn deep_expr_all_variants_clone_and_debug_without_stack_overflow() {
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut expr = Expr::Ident("X".to_owned());
            for step in 0..50_000_usize {
                expr = match step % 4 {
                    0 => Expr::Unary { op: UnaryOp::Neg, rhs: Box::new(expr) },
                    1 => Expr::Binary { op: BinaryOp::Add, lhs: Box::new(expr), rhs: Box::new(Expr::Num(1.0)) },
                    2 => Expr::Call { name: "sin".to_owned(), args: vec![Expr::Num(0.5), expr] },
                    _ => Expr::If { cond: Box::new(Expr::Ident("k".to_owned())), then_br: Box::new(expr), else_br: Box::new(Expr::Num(0.0)) },
                };
            }
            let cloned = expr.clone();
            let dbg_orig = format!("{expr:?}");
            let dbg_cloned = format!("{cloned:?}");
            assert_eq!(dbg_orig, dbg_cloned);
            assert!(dbg_orig.contains("Unary"));
            assert!(dbg_orig.contains("Binary"));
            assert!(dbg_orig.contains("Call"));
            assert!(dbg_orig.contains("If"));
            assert!(dbg_orig.contains("Ident"));
            assert!(dbg_orig.contains("Num"));

            let mut empty_call = Expr::Call { name: "f".to_owned(), args: Vec::new() };
            for _ in 0..10 {
                empty_call = Expr::Call { name: "g".to_owned(), args: vec![empty_call] };
            }
            assert_eq!(format!("{empty_call:?}"), format!("{:?}", empty_call.clone()));
            drop(cloned);
            drop(expr);
        })
        .unwrap();
    handle.join().unwrap();
}
