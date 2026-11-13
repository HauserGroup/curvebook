//! Seeded, platform-deterministic pseudo-random number generator (`SplitMix64` + `xoshiro256**`).
//!
//! Used for Monte Carlo permutation tests, Horn's parallel analysis in PCA, and synthetic table
//! simulation (`lumencraft-transform`). Identical bit-for-bit across x86_64, aarch64, and wasm32.
//!
//! Reference:
//! - Blackman, D., & Vigna, S. (2021). Scrambled linear pseudorandom number generators.
//!   *ACM Transactions on Mathematical Software*, 47(4), 1-32.

use crate::error::MathError;

/// Deterministic 256-bit `xoshiro256**` pseudo-random number generator.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeterministicRng {
    state: [u64; 4],
    cached_normal: Option<f64>,
}

impl DeterministicRng {
    /// Seed a new deterministic generator from a 64-bit integer via `SplitMix64`.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        let s0 = splitmix64_step(&mut sm);
        let s1 = splitmix64_step(&mut sm);
        let s2 = splitmix64_step(&mut sm);
        let s3 = splitmix64_step(&mut sm);
        Self { state: [s0, s1, s2, s3], cached_normal: None }
    }

    /// Generate the next raw 64-bit pseudo-random integer.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.state[1] << 17;

        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];

        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);
        result
    }

    /// Uniform `f64` in `[0.0, 1.0)` with full 53-bit significand resolution.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / ((1u64 << 53) as f64);
        ((self.next_u64() >> 11) as f64) * SCALE
    }

    /// Uniform `f64` strictly inside `(0.0, 1.0)` (never `0.0` or `1.0`), safe for log/quantile transforms.
    #[inline]
    pub fn next_open_f64(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / ((1u64 << 52) as f64);
        (((self.next_u64() >> 12) as f64) + 0.5) * SCALE
    }

    /// Unbiased uniform index in `0..upper_exclusive`, or `None` if `upper_exclusive == 0`.
    pub fn gen_index(&mut self, upper_exclusive: usize) -> Option<usize> {
        if upper_exclusive == 0 {
            return None;
        }
        let bound = upper_exclusive as u64;
        if bound.is_power_of_two() {
            return Some((self.next_u64() & (bound - 1)) as usize);
        }
        let threshold = bound.wrapping_neg() % bound;
        for _ in 0..128 {
            let candidate = self.next_u64();
            if candidate >= threshold {
                return Some((candidate % bound) as usize);
            }
        }
        Some((self.next_u64() % bound) as usize)
    }

    /// In-place unbiased Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        let len = slice.len();
        for i in (1..len).rev() {
            if let Some(j) = self.gen_index(i + 1) {
                slice.swap(i, j);
            }
        }
    }

    /// Draw a standard normal `N(0, 1)` variate via Box-Muller transform.
    pub fn standard_normal(&mut self) -> f64 {
        if let Some(z) = self.cached_normal.take() {
            return z;
        }
        let u1 = self.next_open_f64();
        let u2 = self.next_f64();
        let radius = (-2.0 * u1.ln()).sqrt();
        let theta = std::f64::consts::TAU * u2;
        let (sin_t, cos_t) = theta.sin_cos();
        self.cached_normal = Some(radius * sin_t);
        radius * cos_t
    }

    /// Draw a Gaussian `N(mean, sd^2)` variate.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `mean` or `sd` is non-finite or `sd < 0`.
    pub fn normal(&mut self, mean: f64, sd: f64) -> Result<f64, MathError> {
        if !mean.is_finite() {
            return Err(MathError::InvalidParameter { name: "mean", value: mean });
        }
        if !sd.is_finite() || sd < 0.0 {
            return Err(MathError::InvalidParameter { name: "sd", value: sd });
        }
        Ok(mean + sd * self.standard_normal())
    }

    /// Draw a `Poisson(lambda)` count variate (`lambda >= 0`).
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `lambda` is negative, non-finite, or exceeds `1e15`.
    pub fn poisson(&mut self, lambda: f64) -> Result<u64, MathError> {
        if !lambda.is_finite() || !(0.0..=1e15).contains(&lambda) {
            return Err(MathError::InvalidParameter { name: "lambda", value: lambda });
        }
        if lambda == 0.0 {
            return Ok(0);
        }
        if lambda < 30.0 {
            let limit = (-lambda).exp();
            let mut k: u64 = 0;
            let mut prod = self.next_open_f64();
            while prod > limit && k < 10_000 {
                k += 1;
                prod *= self.next_open_f64();
            }
            return Ok(k);
        }
        // Atkinson (1979) PTRS transformed rejection for lambda >= 30.
        let b = 0.931 + 2.53 * lambda.sqrt();
        let a = -0.059 + 0.02483 * b;
        let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
        let vr = 0.9277 - 3.6224 / (b - 2.0);
        for _ in 0..10_000 {
            let u = self.next_open_f64() - 0.5;
            let v = self.next_open_f64();
            let us = 0.5 - u.abs();
            let k_f = ((2.0 * a / us + b) * u + lambda + 0.43).floor();
            if k_f < 0.0 {
                continue;
            }
            if us >= 0.07 && v <= vr {
                return Ok(k_f as u64);
            }
            if us < 0.013 && v > us {
                continue;
            }
            let k = k_f as u64;
            let ln_accept = (v * inv_alpha / (a / (us * us) + b)).ln();
            let ln_pmf = (k_f * lambda.ln()) - lambda - ln_factorial_approx(k);
            if ln_accept <= ln_pmf {
                return Ok(k);
            }
        }
        Ok(lambda.round() as u64)
    }
}

const fn splitmix64_step(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn ln_factorial_approx(k: u64) -> f64 {
    if k <= 1 {
        return 0.0;
    }
    if k <= 20 {
        let mut prod = 1.0_f64;
        for i in 2..=k {
            prod *= i as f64;
        }
        return prod.ln();
    }
    let kf = k as f64;
    let inv = 1.0 / kf;
    let inv2 = inv * inv;
    (kf + 0.5) * kf.ln() - kf + 0.5 * std::f64::consts::TAU.ln() + inv * (1.0 / 12.0 - inv2 * (1.0 / 360.0 - inv2 / 1260.0))
}

#[cfg(test)]
mod tests {
    use super::DeterministicRng;
    use crate::sum::SummaryMoments;

    #[test]
    fn seed_reproducibility_and_open_interval_bounds() {
        let mut a = DeterministicRng::from_seed(42);
        let mut b = DeterministicRng::from_seed(42);
        for _ in 0..256 {
            let x = a.next_open_f64();
            assert!(x > 0.0 && x < 1.0);
            assert_eq!(x.to_bits(), b.next_open_f64().to_bits());
            let idx = a.gen_index(13).unwrap();
            assert!(idx < 13);
            assert_eq!(idx, b.gen_index(13).unwrap());
        }
        assert_eq!(a.gen_index(0), None);
        assert_eq!(a.gen_index(1), Some(0));
    }

    #[test]
    fn standard_normal_and_poisson_moments_match_theory() {
        let mut rng = DeterministicRng::from_seed(2026);
        let samples: Vec<f64> = (0..20_000).map(|_| rng.standard_normal()).collect();
        let m = SummaryMoments::from_slice(&samples).unwrap();
        assert!(m.mean.abs() < 0.03);
        assert!((m.sample_sd().unwrap() - 1.0).abs() < 0.03);

        let pois_big: Vec<f64> = (0..10_000).map(|_| rng.poisson(64.0).unwrap() as f64).collect();
        let mp = SummaryMoments::from_slice(&pois_big).unwrap();
        assert!((mp.mean - 64.0).abs() < 0.3);
        assert!((mp.sample_variance().unwrap() - 64.0).abs() < 1.8);
    }
}
