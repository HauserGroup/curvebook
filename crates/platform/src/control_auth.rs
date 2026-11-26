//! Per-instance bearer authentication token for local loopback control sockets and streams.
//!
//! Kept in `curvebook-platform` (`L0`) so headless binaries (`curvebook-cli`) can validate
//! `--control-token` without pulling in `curvebook-ui-egui` or GUI crates.

use std::fmt;
use std::io;

/// Number of random bytes encoded into an auto-generated control authentication token (256 bits).
pub const CONTROL_AUTH_TOKEN_BYTES: usize = 32;

/// Hard upper bound on accepted control token length in bytes.
pub const MAX_CONTROL_TOKEN_BYTES: usize = 512;

/// Per-instance bearer secret enforcing authentication on local control sockets and streams.
#[derive(Clone, PartialEq, Eq)]
pub struct ControlAuth {
    token: String,
}

impl fmt::Debug for ControlAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ControlAuth([REDACTED])")
    }
}

impl ControlAuth {
    /// Generate a fresh 256-bit cryptographically random control authentication token.
    ///
    /// # Errors
    /// Returns an I/O error if OS entropy cannot be read, or `Unsupported` on `wasm32`.
    pub fn generate() -> io::Result<Self> {
        let bytes = generate_entropy_bytes()?;
        Ok(Self { token: encode_lower_hex(&bytes) })
    }

    /// Validate and wrap a caller-supplied control authentication token.
    ///
    /// # Errors
    /// Returns a descriptive error when `token` is empty, contains whitespace or control
    /// characters, or exceeds [`MAX_CONTROL_TOKEN_BYTES`].
    pub fn from_token(token: impl Into<String>) -> Result<Self, String> {
        let candidate = token.into();
        if candidate.is_empty() {
            return Err(String::from("control token must not be empty"));
        }
        if candidate.len() > MAX_CONTROL_TOKEN_BYTES {
            return Err(format!("control token exceeds {MAX_CONTROL_TOKEN_BYTES} bytes"));
        }
        if candidate.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(String::from("control token must not contain whitespace or control characters"));
        }
        Ok(Self { token: candidate })
    }

    /// Borrow the raw secret token string for writing to a restricted local token file or client.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.token
    }

    /// Verify `candidate` against the secret token using constant-time byte comparison.
    #[must_use]
    pub fn verify(&self, candidate: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), candidate.as_bytes())
    }
}

fn constant_time_eq(expected: &[u8], actual: &[u8]) -> bool {
    let max_len = expected.len().max(actual.len());
    let mut diff: u8 = u8::from(expected.len() != actual.len());
    for idx in 0..max_len {
        let a = expected.get(idx).copied().unwrap_or(0);
        let b = actual.get(idx).copied().unwrap_or(0);
        diff |= core::hint::black_box(a ^ b);
    }
    core::hint::black_box(diff) == 0
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            let ch = HEX.get(usize::from(nibble)).copied().unwrap_or(b'0');
            out.push(char::from(ch));
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn generate_entropy_bytes() -> io::Result<[u8; CONTROL_AUTH_TOKEN_BYTES]> {
    let mut buf = [0u8; CONTROL_AUTH_TOKEN_BYTES];
    getrandom::fill(&mut buf).map_err(|err| io::Error::other(format!("OS CSPRNG entropy generation failed: {err}")))?;
    Ok(buf)
}

#[cfg(target_arch = "wasm32")]
fn generate_entropy_bytes() -> io::Result<[u8; CONTROL_AUTH_TOKEN_BYTES]> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "OS entropy generation for TCP control auth tokens is unavailable on wasm32"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{CONTROL_AUTH_TOKEN_BYTES, ControlAuth, MAX_CONTROL_TOKEN_BYTES};

    #[test]
    fn control_auth_validates_redacts_and_verifies() {
        let auth = ControlAuth::from_token("secret-token-123").expect("valid token");
        assert_eq!(auth.as_str(), "secret-token-123");
        assert_eq!(format!("{auth:?}"), "ControlAuth([REDACTED])");
        assert!(auth.verify("secret-token-123"));
        assert!(!auth.verify("secret-token-124"));
        assert!(!auth.verify("secret-token-12"));

        assert!(ControlAuth::from_token("").is_err());
        assert!(ControlAuth::from_token("has space").is_err());
        assert!(ControlAuth::from_token("has\nnewline").is_err());
        assert!(ControlAuth::from_token("a".repeat(MAX_CONTROL_TOKEN_BYTES + 1)).is_err());

        let generated = ControlAuth::generate().expect("generate token");
        assert_eq!(generated.as_str().len(), CONTROL_AUTH_TOKEN_BYTES * 2);
        assert!(generated.verify(generated.as_str()));
    }

    #[test]
    fn aud_07_27_control_auth_uses_os_csprng_without_random_state_fallback() {
        // Verify generated tokens are 256-bit lowercase hex strings with no collisions across draws.
        let mut seen = HashSet::new();
        for _ in 0..16 {
            let auth = ControlAuth::generate().expect("OS CSPRNG token generation");
            let tok = auth.as_str();
            assert_eq!(tok.len(), CONTROL_AUTH_TOKEN_BYTES * 2);
            assert!(tok.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "non-hex token: {tok}");
            assert!(tok.bytes().any(|b| b != b'0'), "degenerate all-zero token");
            assert!(seen.insert(tok.to_owned()), "duplicate CSPRNG token generated");
        }

        // Guard against reintroducing a non-cryptographic RandomState fallback in production code.
        let prod = include_str!("control_auth.rs").split("#[cfg(test)]").next().unwrap_or("");
        assert!(!prod.contains("RandomState"), "control_auth.rs production code must use OS CSPRNG rather than SipHash RandomState");
        assert!(prod.contains("getrandom::fill"), "control_auth.rs production code must invoke getrandom::fill");
    }
}
