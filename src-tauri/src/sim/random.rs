//! Deterministic PRNG, ported bit-for-bit from `src/sim/random.ts`.
//!
//! This is the single most determinism-critical file in the port (see
//! MIGRATION_PLAN.md #9, item 1): "same seed and topology means
//! byte-identical snapshots" (AGENTS.md) depends on every operation here
//! running in the exact order, with the exact constants, that the TS source
//! uses. `Rng::next` is a mulberry32 generator; `Rng::gamma` is Marsaglia-Tsang;
//! `Rng::normal` is Box-Muller.
//!
//! JS's `>>> 0` (unsigned-right-shift-by-zero) forces a value into the
//! unsigned 32-bit range; `Math.imul` is a 32-bit signed multiply truncated
//! to 32 bits, returning the low 32 bits. Rust's `u32` arithmetic already
//! lives in that 32-bit range, and `u32::wrapping_mul` produces the same low
//! 32 bits as `Math.imul` for the same bit pattern (the two only disagree on
//! how the *result* is interpreted -- signed vs. unsigned -- and every use
//! of that result here is a bitwise XOR/shift, which does not care). So this
//! port uses `u32` wrapping arithmetic throughout and never needs an
//! explicit `>>> 0`.

/// Deterministic PRNG (mulberry32) so a given seed replays identically.
pub struct Rng {
    s: u32,
}

impl Rng {
    /// TS: `this.s = seed >>> 0` unsigned-wraps a possibly-signed seed into
    /// `u32`. The parameter here is already `u32`, so that coercion is a
    /// no-op and this is just a store.
    pub fn new(seed: u32) -> Self {
        Self { s: seed }
    }

    pub fn next(&mut self) -> f64 {
        self.s = self.s.wrapping_add(0x6d2b79f5);
        let mut t = self.s;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4294967296.0
    }

    /// Exponential with the given mean.
    pub fn exponential(&mut self, mean: f64) -> f64 {
        if mean <= 0.0 {
            return 0.0;
        }
        // Guard against log(0).
        let u = 1.0 - self.next();
        -u.ln() * mean
    }

    /// Service time with a target mean and coefficient of variation.
    /// cv=0 -> deterministic, cv=1 -> exponential, cv>1 -> heavy tailed.
    /// Implemented as a gamma draw with shape k = 1/cv^2.
    pub fn service_time(&mut self, mean: f64, cv: f64) -> f64 {
        if mean <= 0.0 {
            return 0.0;
        }
        if cv <= 0.01 {
            return mean;
        }
        let shape = 1.0 / (cv * cv);
        let scale = mean / shape;
        self.gamma(shape, scale)
    }

    /// Marsaglia-Tsang gamma sampler.
    pub fn gamma(&mut self, shape: f64, scale: f64) -> f64 {
        if shape < 1.0 {
            // Boost low shapes into the valid range.
            let u = 1.0 - self.next();
            return self.gamma(shape + 1.0, scale) * u.powf(1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let mut x: f64;
            let mut v: f64;
            loop {
                x = self.normal();
                v = 1.0 + c * x;
                if v > 0.0 {
                    break;
                }
            }
            v = v * v * v;
            let u = 1.0 - self.next();
            if u < 1.0 - 0.0331 * x * x * x * x {
                return d * v * scale;
            }
            if u.ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
                return d * v * scale;
            }
        }
    }

    /// Standard normal via Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u = 1.0 - self.next();
        let v = self.next();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// These tests verify the Rust implementation is deterministic and
/// internally consistent. They do NOT verify the output sequence matches
/// the TypeScript engine bit-for-bit -- that requires a cross-language
/// fixture (same seed, compare N draws) which was not run in this session.
/// See MIGRATION_PLAN.md #9.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_deterministic_for_a_given_seed() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(1);
        let seq_a: Vec<f64> = (0..10).map(|_| a.next()).collect();
        let seq_b: Vec<f64> = (0..10).map(|_| b.next()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn consecutive_draws_differ() {
        let mut rng = Rng::new(1);
        let a = rng.next();
        let b = rng.next();
        assert_ne!(a, b);
    }

    #[test]
    fn next_stays_in_unit_interval() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let v = rng.next();
            assert!((0.0..1.0).contains(&v), "draw {v} out of [0, 1)");
        }
    }

    #[test]
    fn exponential_is_nonnegative_and_zero_for_nonpositive_mean() {
        let mut rng = Rng::new(7);
        assert_eq!(rng.exponential(0.0), 0.0);
        assert_eq!(rng.exponential(-5.0), 0.0);
        for _ in 0..100 {
            assert!(rng.exponential(10.0) >= 0.0);
        }
    }

    #[test]
    fn service_time_is_exactly_mean_when_cv_is_near_zero() {
        let mut rng = Rng::new(3);
        assert_eq!(rng.service_time(25.0, 0.0), 25.0);
        assert_eq!(rng.service_time(25.0, 0.01), 25.0);
    }

    #[test]
    fn gamma_draws_are_positive() {
        let mut rng = Rng::new(9);
        for _ in 0..200 {
            assert!(rng.gamma(2.5, 4.0) > 0.0);
            // Exercise the shape < 1 boosting branch too.
            assert!(rng.gamma(0.5, 4.0) > 0.0);
        }
    }
}
