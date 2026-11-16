//! Test-only numerical tolerance assertions, 106-bit `DoubleDouble` reference oracles, SHA-256
//! corpus verification, and NIST Statistical Reference Datasets (StRD) harness for CurveBook.
//!
//! May only be depended on as a `[dev-dependencies]` entry (enforced by `cargo xtask layers`).
//! Governing spec: `docs/development.md` (§Tests & numerical verification, §Verified NIST StRD corpus).
//!
//! # Example
//!
//! ```
//! use curvebook_testkit::{assert_lre_at_least, oracle_univariate_f64, sha256_hex};
//! use curvebook_testkit::nist_strd::numacc4;
//!
//! let ds = numacc4();
//! let (mean, sd, r1) = oracle_univariate_f64(&ds.values);
//! assert_lre_at_least(mean, ds.certified_mean, 15.0, "NumAcc4 mean");
//! // Decimal 10_000_000.2 rounds in IEEE-754 f64 (~8.43 digits vs decimal 0.1).
//! assert_lre_at_least(sd, ds.certified_sd, 8.0, "NumAcc4 sd");
//! assert_lre_at_least(r1, ds.certified_r1, 14.5, "NumAcc4 r1");
//! assert_eq!(sha256_hex(b"abc").len(), 64);
//! ```
#![forbid(unsafe_code)]

pub mod dd;
pub mod metrics;
pub mod nist_strd;
pub mod sha256;

pub use dd::{DoubleDouble, oracle_univariate_f64};
pub use metrics::{MAX_F64_LRE_DIGITS, approx_eq, assert_approx_eq, assert_lre_at_least, assert_ulp_le, log_relative_error, ulp_distance};
pub use sha256::{CorpusError, CorpusManifestEntry, load_verified_corpus_file, sha256_bytes, sha256_hex, verify_blob};

#[cfg(test)]
mod tests {
    use super::{approx_eq, log_relative_error, ulp_distance};

    #[test]
    fn approx_eq_and_lre_handle_finite_zero_and_nan() {
        assert!(approx_eq(1.000_000_1, 1.0, 1e-9, 1e-6));
        assert!(!approx_eq(f64::NAN, 1.0, 1e-3, 1e-3));
        assert_eq!(ulp_distance(0.0, -0.0), Some(0));
        assert_eq!(ulp_distance(1.0, 1.0 + f64::EPSILON), Some(1));
        assert!((log_relative_error(1.000_001, 1.0) - 6.0).abs() < 1e-3);
        assert_eq!(log_relative_error(f64::NAN, 1.0), 0.0);
    }
}
