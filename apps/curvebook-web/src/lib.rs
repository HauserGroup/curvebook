//! Minimal strict-CSP `wasm32` browser placeholder for CurveBook (`R11.11`).
//!
//! Legacy pre-`R15` browser bridges were retired pending the post-Beta `R11.11`
//! `eframe::WebRunner` migration. This crate keeps the workspace `wasm32-unknown-unknown`
//! target, Trunk packaging hook, and zero-egress CSP contract green.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

/// Embedded strict-CSP placeholder `index.html` served by Trunk.
pub const PLACEHOLDER_INDEX_HTML: &str = include_str!("../index.html");

/// Human-readable status notice for the deferred `R11.11` browser placeholder.
pub const PLACEHOLDER_NOTICE: &str = "CurveBook WebAssembly placeholder pending R11.11 (eframe::WebRunner).";

/// Required strict-CSP directives enforced on `apps/curvebook-web/index.html`.
pub const REQUIRED_CSP_DIRECTIVES: &[&str] = &[
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self'",
    "connect-src 'self'",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'none'",
];

/// Return the static placeholder status message without OS or network calls.
#[must_use]
pub const fn placeholder_status() -> &'static str {
    PLACEHOLDER_NOTICE
}

/// Export the placeholder status string on `wasm32` so `wasm-bindgen` links cleanly for Trunk.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
#[must_use]
pub fn web_placeholder_status() -> String {
    String::from(PLACEHOLDER_NOTICE)
}

#[cfg(test)]
mod tests {
    use super::{PLACEHOLDER_INDEX_HTML, PLACEHOLDER_NOTICE, REQUIRED_CSP_DIRECTIVES, placeholder_status};

    #[test]
    fn strict_csp_zero_egress_placeholder_contract_holds() {
        assert_eq!(placeholder_status(), PLACEHOLDER_NOTICE);
        assert!(PLACEHOLDER_INDEX_HTML.contains("<title>CurveBook</title>"));
        assert!(PLACEHOLDER_INDEX_HTML.contains("data-wasm-opt"));
        for directive in REQUIRED_CSP_DIRECTIVES {
            assert!(PLACEHOLDER_INDEX_HTML.contains(directive), "index.html missing CSP directive `{directive}`");
        }

        let lower = PLACEHOLDER_INDEX_HTML.to_ascii_lowercase();
        let forbidden: [&str; 12] = [
            "<script",
            "<style",
            " style=\"",
            "'unsafe-inline'",
            "http://",
            "https://",
            "ws://",
            "wss://",
            "fetch(",
            concat!("web", "socket"),
            concat!("xmlhttp", "request"),
            concat!("send", "beacon"),
        ];
        for token in forbidden {
            assert!(!lower.contains(token), "index.html must not contain forbidden token `{token}`");
        }
    }
}
