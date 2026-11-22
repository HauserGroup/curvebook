//! Pure-Rust, `wasm32`-clean NIST FIPS PUB 180-4 SHA-256 cryptographic hash implementation.
//!
//! Provides both the streaming [`Sha256`] hasher (`new` / `update` / `finalize`) and one-shot
//! convenience functions [`sha256_bytes`] and [`sha256_hex`].
//!
//! Reference: National Institute of Standards and Technology (NIST). (2015). *Secure Hash Standard
//! (SHS)*, Federal Information Processing Standards Publication 180-4 (`FIPS PUB 180-4`).

const K256: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const H0: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];

fn compress_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0_u32; 64];
    for (i, slot) in w.iter_mut().take(16).enumerate() {
        let base = i * 4;
        *slot = u32::from_be_bytes([block[base], block[base + 1], block[base + 2], block[base + 3]]);
    }
    for i in 16..64 {
        let w15 = w[i - 15];
        let w2 = w[i - 2];
        let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
        let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    let deltas = [a, b, c, d, e, f, g, h];
    for (slot, delta) in state.iter_mut().zip(deltas) {
        *slot = slot.wrapping_add(delta);
    }
}

/// Streaming NIST FIPS PUB 180-4 SHA-256 hasher.
#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_bytes: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// Create a fresh SHA-256 hasher initialized with the FIPS 180-4 initial hash values.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: H0, buffer: [0_u8; 64], buffer_len: 0, total_bytes: 0 }
    }

    /// Absorb `data` into the running SHA-256 digest state.
    pub fn update(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);
        let mut rest = data;

        if self.buffer_len > 0 {
            let space = 64 - self.buffer_len;
            if rest.len() < space {
                self.buffer[self.buffer_len..self.buffer_len + rest.len()].copy_from_slice(rest);
                self.buffer_len += rest.len();
                return;
            }
            self.buffer[self.buffer_len..64].copy_from_slice(&rest[..space]);
            let block = self.buffer;
            compress_block(&mut self.state, &block);
            self.buffer_len = 0;
            rest = &rest[space..];
        }

        let (blocks, rem) = rest.as_chunks::<64>();
        for block in blocks {
            compress_block(&mut self.state, block);
        }
        if !rem.is_empty() {
            self.buffer[..rem.len()].copy_from_slice(rem);
            self.buffer_len = rem.len();
        }
    }

    /// Finalize padding and return the 32-byte SHA-256 digest.
    #[must_use]
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..64].fill(0);
            let block = self.buffer;
            compress_block(&mut self.state, &block);
            self.buffer[..56].fill(0);
        } else {
            self.buffer[self.buffer_len..56].fill(0);
        }
        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let final_block = self.buffer;
        compress_block(&mut self.state, &final_block);

        let mut out = [0_u8; 32];
        for (idx, word) in self.state.into_iter().enumerate() {
            out[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// Finalize padding and return the lowercase 64-character hexadecimal SHA-256 digest.
    #[must_use]
    pub fn finalize_hex(self) -> String {
        digest_to_hex(&self.finalize())
    }
}

/// Format a 32-byte digest as a lowercase 64-character hexadecimal string.
#[must_use]
pub fn digest_to_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    out
}

/// Compute the 32-byte NIST FIPS PUB 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

/// Compute the lowercase 64-character hexadecimal NIST FIPS PUB 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    digest_to_hex(&sha256_bytes(data))
}

#[cfg(test)]
mod tests {
    use super::{Sha256, digest_to_hex, sha256_bytes, sha256_hex};

    #[test]
    fn matches_nist_fips_180_4_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // 448-bit (56 bytes) — exact single-block padding overflow boundary.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 896-bit (112 bytes) — NIST CAVP multi-block vector.
        assert_eq!(
            sha256_hex(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
        // 1,000,000 repetitions of 'a' — NIST FIPS 180-4 long message vector.
        let mut hasher = Sha256::new();
        let block_a = [b'a'; 1000];
        for _ in 0..1000 {
            hasher.update(&block_a);
        }
        assert_eq!(hasher.finalize_hex(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn streaming_matches_one_shot_across_block_boundaries() {
        for len in [0_usize, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 129, 255] {
            let msg: Vec<u8> = (0..len).map(|i| ((i * 37 + 11) & 0xFF) as u8).collect();
            let expected = sha256_bytes(&msg);
            for chunk_size in [1_usize, 3, 7, 13, 31, 64, 65] {
                let mut h = Sha256::default();
                h.update(&[]);
                for chunk in msg.chunks(chunk_size) {
                    h.update(chunk);
                }
                h.update(&[]);
                assert_eq!(h.finalize(), expected, "mismatch for len={len}, chunk_size={chunk_size}");
            }
            assert_eq!(digest_to_hex(&expected), sha256_hex(&msg));
        }
    }
}
