//! Pure-Rust, `wasm32`-clean RFC 4648 standard Base64 encoder and bounded decoder.
//!
//! Provides allocation-guarded decoding (`decode_capped`) that ignores ASCII whitespace,
//! validates length and decoded-byte ceilings before allocating output buffers, and rejects
//! empty inputs, invalid characters, and malformed `=` padding with typed [`Base64Error`]s.

use std::fmt;

const ENCODE_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Structured error returned by [`decode`] and [`decode_capped`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base64Error {
    /// Input was empty or contained only ASCII whitespace.
    Empty,
    /// Number of non-whitespace input bytes was not a multiple of 4.
    InvalidLength {
        /// Count of non-whitespace bytes observed in the input.
        len: usize,
    },
    /// Input contained a byte outside the RFC 4648 standard alphabet (`A-Z`, `a-z`, `0-9`, `+`, `/`).
    InvalidCharacter {
        /// Offending raw byte value.
        byte: u8,
    },
    /// Padding character `=` appeared in an invalid position or before trailing data.
    InvalidPadding,
    /// Input or decoded payload exceeded the caller-supplied `max_decoded_bytes` limit.
    MaxSizeExceeded {
        /// Maximum number of decoded bytes permitted by the caller.
        max_bytes: usize,
    },
}

impl fmt::Display for Base64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("base64 payload must not be empty"),
            Self::InvalidLength { len } => {
                write!(f, "base64 payload length ({len}) must be a multiple of 4")
            }
            Self::InvalidCharacter { byte } => {
                write!(f, "invalid base64 character `{}` (0x{byte:02x})", char::from(*byte))
            }
            Self::InvalidPadding => f.write_str("invalid or misplaced base64 `=` padding"),
            Self::MaxSizeExceeded { max_bytes } => {
                write!(f, "base64 payload exceeds maximum allowed size ({max_bytes} bytes)")
            }
        }
    }
}

impl std::error::Error for Base64Error {}

/// Encode raw binary bytes into standard RFC 4648 Base64 with `=` padding and no line wrapping.
#[must_use]
pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3).saturating_mul(4));
    let (chunks, remainder) = input.as_chunks::<3>();
    for &[b0, b1, b2] in chunks {
        let triple = (usize::from(b0) << 16) | (usize::from(b1) << 8) | usize::from(b2);
        out.push(char::from(ENCODE_TABLE[(triple >> 18) & 0x3F]));
        out.push(char::from(ENCODE_TABLE[(triple >> 12) & 0x3F]));
        out.push(char::from(ENCODE_TABLE[(triple >> 6) & 0x3F]));
        out.push(char::from(ENCODE_TABLE[triple & 0x3F]));
    }
    match remainder {
        [b0] => {
            let triple = usize::from(*b0) << 16;
            out.push(char::from(ENCODE_TABLE[(triple >> 18) & 0x3F]));
            out.push(char::from(ENCODE_TABLE[(triple >> 12) & 0x3F]));
            out.push('=');
            out.push('=');
        }
        [b0, b1] => {
            let triple = (usize::from(*b0) << 16) | (usize::from(*b1) << 8);
            out.push(char::from(ENCODE_TABLE[(triple >> 18) & 0x3F]));
            out.push(char::from(ENCODE_TABLE[(triple >> 12) & 0x3F]));
            out.push(char::from(ENCODE_TABLE[(triple >> 6) & 0x3F]));
            out.push('=');
        }
        _ => {}
    }
    out
}

const fn decode_sextet(byte: u8) -> Result<u8, Base64Error> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(Base64Error::InvalidCharacter { byte }),
    }
}

fn decode_quad(quad: [u8; 4]) -> Result<([u8; 3], usize, bool), Base64Error> {
    let [q0, q1, q2, q3] = quad;
    if q0 == b'=' || q1 == b'=' || (q2 == b'=' && q3 != b'=') {
        return Err(Base64Error::InvalidPadding);
    }
    let s0 = u32::from(decode_sextet(q0)?);
    let s1 = u32::from(decode_sextet(q1)?);
    let pad_len = usize::from(q2 == b'=') + usize::from(q3 == b'=');
    let s2 = if q2 == b'=' { 0 } else { u32::from(decode_sextet(q2)?) };
    let s3 = if q3 == b'=' { 0 } else { u32::from(decode_sextet(q3)?) };
    let word = (s0 << 18) | (s1 << 12) | (s2 << 6) | s3;
    let bytes = [((word >> 16) & 0xFF) as u8, ((word >> 8) & 0xFF) as u8, (word & 0xFF) as u8];
    Ok((bytes, 3 - pad_len, pad_len > 0))
}

/// Zero-allocation pre-scan that enforces raw/non-whitespace/decoded byte bounds, rejects
/// empty inputs and non-multiple-of-4 lengths, and returns the exact decoded byte count.
fn prevalidate_capped(input: &str, max_decoded_bytes: usize) -> Result<usize, Base64Error> {
    let max_b64_len = max_decoded_bytes.div_ceil(3).saturating_mul(4);
    // Permit modest MIME whitespace overhead (~25% + 1 KiB) while rejecting huge raw inputs before scanning.
    let max_raw_len = max_b64_len.saturating_add(max_b64_len / 4).saturating_add(1024);
    if input.len() > max_raw_len {
        return Err(Base64Error::MaxSizeExceeded { max_bytes: max_decoded_bytes });
    }

    let mut non_ws_len = 0_usize;
    let mut last2 = 0_u8;
    let mut last1 = 0_u8;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        non_ws_len = non_ws_len.saturating_add(1);
        if non_ws_len > max_b64_len {
            return Err(Base64Error::MaxSizeExceeded { max_bytes: max_decoded_bytes });
        }
        last2 = last1;
        last1 = byte;
    }

    if non_ws_len == 0 {
        return Err(Base64Error::Empty);
    }
    if !non_ws_len.is_multiple_of(4) {
        return Err(Base64Error::InvalidLength { len: non_ws_len });
    }

    let trailing_pad = if last1 == b'=' { if last2 == b'=' { 2_usize } else { 1_usize } } else { 0_usize };
    let decoded_len = (non_ws_len / 4).saturating_mul(3).saturating_sub(trailing_pad);
    if decoded_len > max_decoded_bytes {
        return Err(Base64Error::MaxSizeExceeded { max_bytes: max_decoded_bytes });
    }
    Ok(decoded_len)
}

/// Decode a standard RFC 4648 Base64 string (ignoring ASCII whitespace) up to `max_decoded_bytes`.
///
/// Validates raw length, non-whitespace length, empty input, and decoded byte ceiling before
/// allocating the output buffer.
///
/// # Errors
/// Returns [`Base64Error::Empty`] if `input` contains no non-whitespace bytes,
/// [`Base64Error::MaxSizeExceeded`] if the input or decoded length exceeds `max_decoded_bytes`,
/// [`Base64Error::InvalidLength`] if the non-whitespace length is not a multiple of 4,
/// [`Base64Error::InvalidPadding`] for misplaced or malformed `=` padding, or
/// [`Base64Error::InvalidCharacter`] for any non-whitespace byte outside the standard Base64 alphabet.
pub fn decode_capped(input: &str, max_decoded_bytes: usize) -> Result<Vec<u8>, Base64Error> {
    let expected_len = prevalidate_capped(input, max_decoded_bytes)?;
    let mut out = Vec::with_capacity(expected_len);
    let mut quad = [0_u8; 4];
    let mut quad_len = 0_usize;
    let mut saw_padding = false;

    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if saw_padding {
            return Err(Base64Error::InvalidPadding);
        }
        quad[quad_len] = byte;
        quad_len += 1;
        if quad_len < 4 {
            continue;
        }
        quad_len = 0;
        let (bytes, decoded_len, padded) = decode_quad(quad)?;
        saw_padding = padded;
        if out.len().saturating_add(decoded_len) > max_decoded_bytes {
            return Err(Base64Error::MaxSizeExceeded { max_bytes: max_decoded_bytes });
        }
        out.extend_from_slice(&bytes[..decoded_len]);
    }
    Ok(out)
}

/// Decode a standard RFC 4648 Base64 string (ignoring ASCII whitespace) with no caller byte cap.
///
/// # Errors
/// Returns [`Base64Error::Empty`], [`Base64Error::InvalidLength`], [`Base64Error::InvalidPadding`],
/// or [`Base64Error::InvalidCharacter`] on malformed input.
pub fn decode(input: &str) -> Result<Vec<u8>, Base64Error> {
    decode_capped(input, usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::{Base64Error, decode, decode_capped, encode};

    #[test]
    fn rfc_4648_section_10_vectors_roundtrip() {
        let vectors: &[(&[u8], &str)] =
            &[(b"", ""), (b"f", "Zg=="), (b"fo", "Zm8="), (b"foo", "Zm9v"), (b"foob", "Zm9vYg=="), (b"fooba", "Zm9vYmE="), (b"foobar", "Zm9vYmFy")];
        for &(raw, encoded) in vectors {
            assert_eq!(encode(raw), encoded);
            if !encoded.is_empty() {
                assert_eq!(decode(encoded), Ok(raw.to_vec()));
            }
        }
    }

    #[test]
    fn binary_payload_all_byte_values_roundtrip() {
        let all_bytes: Vec<u8> = (0_u8..=255_u8).collect();
        let b64 = encode(&all_bytes);
        assert_eq!(decode(&b64), Ok(all_bytes));
    }

    #[test]
    fn ignores_ascii_whitespace_cleanly() {
        let wrapped = "  Zm9v\r\n  YmFy\t\n ";
        assert_eq!(decode(wrapped), Ok(b"foobar".to_vec()));
        assert_eq!(decode_capped(wrapped, 6), Ok(b"foobar".to_vec()));
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(decode(""), Err(Base64Error::Empty));
        assert_eq!(decode("   \r\n\t  "), Err(Base64Error::Empty));
        assert_eq!(decode_capped("", 0), Err(Base64Error::Empty));
        assert_eq!(decode_capped(" \n ", 10), Err(Base64Error::Empty));
    }

    #[test]
    fn rejects_invalid_lengths_and_characters() {
        assert_eq!(decode("A"), Err(Base64Error::InvalidLength { len: 1 }));
        assert_eq!(decode("AB"), Err(Base64Error::InvalidLength { len: 2 }));
        assert_eq!(decode("ABC"), Err(Base64Error::InvalidLength { len: 3 }));
        assert_eq!(decode("ABCDE"), Err(Base64Error::InvalidLength { len: 5 }));
        assert_eq!(decode("Zm9$"), Err(Base64Error::InvalidCharacter { byte: b'$' }));
        assert_eq!(decode("Zm9-"), Err(Base64Error::InvalidCharacter { byte: b'-' }));
        assert_eq!(decode("Zm9_"), Err(Base64Error::InvalidCharacter { byte: b'_' }));
    }

    #[test]
    fn rejects_malformed_and_misplaced_padding() {
        assert_eq!(decode("===="), Err(Base64Error::InvalidPadding));
        assert_eq!(decode("=AAA"), Err(Base64Error::InvalidPadding));
        assert_eq!(decode("A=AA"), Err(Base64Error::InvalidPadding));
        assert_eq!(decode("AA=A"), Err(Base64Error::InvalidPadding));
        assert_eq!(decode("Zg==Zm9v"), Err(Base64Error::InvalidPadding));
        assert_eq!(decode("Zm8=Zm9v"), Err(Base64Error::InvalidPadding));
    }

    #[test]
    fn decode_capped_enforces_exact_decoded_and_raw_bounds() {
        // Exact boundary passes: 1 byte ("Zg=="), 2 bytes ("Zm8="), 3 bytes ("Zm9v").
        assert_eq!(decode_capped("Zg==", 1), Ok(b"f".to_vec()));
        assert_eq!(decode_capped("Zm8=", 2), Ok(b"fo".to_vec()));
        assert_eq!(decode_capped("Zm9v", 3), Ok(b"foo".to_vec()));

        // One byte over cap fails cleanly.
        assert_eq!(decode_capped("Zg==", 0), Err(Base64Error::MaxSizeExceeded { max_bytes: 0 }));
        assert_eq!(decode_capped("Zm8=", 1), Err(Base64Error::MaxSizeExceeded { max_bytes: 1 }));
        assert_eq!(decode_capped("Zm9v", 2), Err(Base64Error::MaxSizeExceeded { max_bytes: 2 }));
        assert_eq!(decode_capped("Zm9vYg==", 3), Err(Base64Error::MaxSizeExceeded { max_bytes: 3 }));

        // Oversized raw whitespace input fails early before scanning.
        let huge_ws = " ".repeat(2048);
        assert_eq!(decode_capped(&huge_ws, 16), Err(Base64Error::MaxSizeExceeded { max_bytes: 16 }));
    }

    #[test]
    fn error_display_messages_are_clear() {
        assert!(Base64Error::Empty.to_string().contains("empty"));
        assert!(Base64Error::InvalidLength { len: 3 }.to_string().contains("multiple of 4"));
        assert!(Base64Error::InvalidCharacter { byte: b'?' }.to_string().contains('?'));
        assert!(Base64Error::InvalidPadding.to_string().contains("padding"));
        assert!(Base64Error::MaxSizeExceeded { max_bytes: 64 }.to_string().contains("64 bytes"));
    }
}
