//! Panic-recovery boundary isolating command execution and analytical recalculation from UI panics.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::EngineError;

fn extract_panic_payload(payload: &(dyn Any + Send)) -> String {
    if let Some(msg) = payload.downcast_ref::<&str>() {
        if msg.trim().is_empty() { String::from("empty panic payload") } else { (*msg).to_owned() }
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        if msg.trim().is_empty() { String::from("empty panic payload") } else { msg.clone() }
    } else {
        String::from("unexpected non-string panic payload")
    }
}

/// Execute `f` inside a panic boundary, converting any caught unwind into an [`EngineError`].
///
/// # Errors
/// Returns [`EngineError`] if `f` returns an error or panics while unwinding.
pub fn run_guarded<T, F>(op: &str, f: F) -> Result<T, EngineError>
where
    F: FnOnce() -> Result<T, EngineError>,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(res) => res,
        Err(payload) => {
            let reason = extract_panic_payload(payload.as_ref());
            if op.trim().is_empty() {
                Err(EngineError::new(format!("recovered from internal panic: {reason}")))
            } else {
                Err(EngineError::new(format!("{op}: recovered from internal panic: {reason}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::panic_any;

    use super::*;

    #[test]
    fn guard_passes_ok_and_err_results() {
        let ok = run_guarded("op.ok", || Ok::<_, EngineError>(42));
        assert_eq!(ok.expect("ok should pass through"), 42);

        let err = run_guarded::<(), _>("op.err", || Err(EngineError::new("expected failure")));
        let msg = err.expect_err("err should pass through").to_string();
        assert!(msg.contains("expected failure"), "got: {msg}");
    }

    #[test]
    #[allow(clippy::panic)]
    fn guard_catches_str_and_string_panics_for_rollback() {
        let mut committed = vec![1_i32, 2, 3];
        let snapshot = committed.clone();

        let err_str = run_guarded::<(), _>("data.set_cell", || {
            committed.push(99);
            panic!("index out of bounds in transform");
        })
        .expect_err("panic should convert to EngineError");
        committed = snapshot.clone();
        assert!(err_str.to_string().contains("data.set_cell"));
        assert!(err_str.to_string().contains("index out of bounds in transform"));
        assert_eq!(committed, vec![1, 2, 3]);

        let err_string = run_guarded::<(), _>("fit.solve", || {
            panic!("solver overflow at iter {}", 7);
        })
        .expect_err("String panic should convert to EngineError");
        assert!(err_string.to_string().contains("fit.solve"));
        assert!(err_string.to_string().contains("solver overflow at iter 7"));
        assert_eq!(committed, snapshot);
    }

    #[test]
    #[allow(clippy::panic)]
    fn guard_handles_empty_and_non_string_panics() {
        let empty = run_guarded::<(), _>("", || panic!("   ")).expect_err("empty panic should fail");
        assert!(empty.to_string().contains("empty panic payload"));

        let non_str = run_guarded::<(), _>("stats.anova", || panic_any(12345_usize)).expect_err("non-string panic should fail");
        assert!(non_str.to_string().contains("stats.anova"));
        assert!(non_str.to_string().contains("unexpected non-string panic payload"));
    }
}
