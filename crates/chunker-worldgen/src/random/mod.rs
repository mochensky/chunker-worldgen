//! Vanilla random sources (`LegacyRandomSource`, `XoroshiroRandomSource`, `WorldgenRandom`,
//! positional factories). All arithmetic follows Java's wrapping semantics.

mod legacy;
mod worldgen;
mod xoroshiro;

pub use legacy::LegacyRandom;
pub use worldgen::WorldgenRandom;
pub use xoroshiro::{Xoroshiro128PlusPlus, XoroshiroRandom};

use md5::{Digest, Md5};

/// `float` multiplier `5.9604645E-8F` = 2^-24.
pub(crate) const FLOAT_UNIT: f32 = 5.960_464_5e-8;
/// `1.110223E-16F` (a float literal promoted to double) = 2^-53.
pub(crate) const DOUBLE_UNIT: f64 = 1.110_223e-16_f32 as f64;

/// Common interface of vanilla random sources (`net.minecraft.util.RandomSource`).
pub trait RandomSource {
    fn set_seed(&mut self, seed: i64);
    fn next_int(&mut self) -> i32;
    fn next_int_bounded(&mut self, bound: i32) -> i32;
    fn next_long(&mut self) -> i64;
    fn next_bool(&mut self) -> bool;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
    fn next_gaussian(&mut self) -> f64;
    fn fork(&mut self) -> AnyRandom;
    fn fork_positional(&mut self) -> PositionalRandomFactory;

    fn consume_count(&mut self, rounds: i32) {
        for _ in 0..rounds {
            self.next_int();
        }
    }

    /// `nextIntBetweenInclusive(min, maxInclusive)`.
    fn next_int_between_inclusive(&mut self, min: i32, max_inclusive: i32) -> i32 {
        self.next_int_bounded(max_inclusive - min + 1) + min
    }

    /// `nextInt(origin, bound)`.
    fn next_int_range(&mut self, origin: i32, bound: i32) -> i32 {
        assert!(origin < bound, "bound - origin is non positive");
        origin + self.next_int_bounded(bound - origin)
    }

    fn triangle_f64(&mut self, mean: f64, spread: f64) -> f64 {
        mean + spread * (self.next_double() - self.next_double())
    }

    fn triangle_f32(&mut self, mean: f32, spread: f32) -> f32 {
        mean + spread * (self.next_float() - self.next_float())
    }
}

/// Dynamically chosen random source (vanilla uses polymorphism here).
#[derive(Clone)]
pub enum AnyRandom {
    Legacy(LegacyRandom),
    Xoroshiro(XoroshiroRandom),
}

macro_rules! dispatch {
    ($self:ident, $r:ident => $e:expr) => {
        match $self {
            AnyRandom::Legacy($r) => $e,
            AnyRandom::Xoroshiro($r) => $e,
        }
    };
}

impl RandomSource for AnyRandom {
    fn set_seed(&mut self, seed: i64) {
        dispatch!(self, r => r.set_seed(seed))
    }
    fn next_int(&mut self) -> i32 {
        dispatch!(self, r => r.next_int())
    }
    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        dispatch!(self, r => r.next_int_bounded(bound))
    }
    fn next_long(&mut self) -> i64 {
        dispatch!(self, r => r.next_long())
    }
    fn next_bool(&mut self) -> bool {
        dispatch!(self, r => r.next_bool())
    }
    fn next_float(&mut self) -> f32 {
        dispatch!(self, r => r.next_float())
    }
    fn next_double(&mut self) -> f64 {
        dispatch!(self, r => r.next_double())
    }
    fn next_gaussian(&mut self) -> f64 {
        dispatch!(self, r => r.next_gaussian())
    }
    fn fork(&mut self) -> AnyRandom {
        dispatch!(self, r => r.fork())
    }
    fn fork_positional(&mut self) -> PositionalRandomFactory {
        dispatch!(self, r => r.fork_positional())
    }
    fn consume_count(&mut self, rounds: i32) {
        dispatch!(self, r => r.consume_count(rounds))
    }
}

/// `WorldgenRandom.Algorithm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomAlgorithm {
    Legacy,
    Xoroshiro,
}

impl RandomAlgorithm {
    pub fn new_instance(self, seed: i64) -> AnyRandom {
        match self {
            RandomAlgorithm::Legacy => AnyRandom::Legacy(LegacyRandom::new(seed)),
            RandomAlgorithm::Xoroshiro => AnyRandom::Xoroshiro(XoroshiroRandom::new(seed)),
        }
    }
}

/// `PositionalRandomFactory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionalRandomFactory {
    Legacy { seed: i64 },
    Xoroshiro { seed_lo: i64, seed_hi: i64 },
}

impl PositionalRandomFactory {
    pub fn at(&self, x: i32, y: i32, z: i32) -> AnyRandom {
        let positional = crate::math::get_seed(x, y, z);
        match *self {
            PositionalRandomFactory::Legacy { seed } => {
                AnyRandom::Legacy(LegacyRandom::new(positional ^ seed))
            }
            PositionalRandomFactory::Xoroshiro { seed_lo, seed_hi } => {
                AnyRandom::Xoroshiro(XoroshiroRandom::from_seeds(positional ^ seed_lo, seed_hi))
            }
        }
    }

    pub fn from_hash_of(&self, name: &str) -> AnyRandom {
        match *self {
            PositionalRandomFactory::Legacy { seed } => {
                let positional = crate::math::java_string_hash(name) as i64;
                AnyRandom::Legacy(LegacyRandom::new(positional ^ seed))
            }
            PositionalRandomFactory::Xoroshiro { seed_lo, seed_hi } => {
                let (lo, hi) = seed_from_hash_of(name);
                AnyRandom::Xoroshiro(XoroshiroRandom::from_seeds(lo ^ seed_lo, hi ^ seed_hi))
            }
        }
    }

    pub fn from_seed(&self, seed: i64) -> AnyRandom {
        match *self {
            PositionalRandomFactory::Legacy { .. } => AnyRandom::Legacy(LegacyRandom::new(seed)),
            PositionalRandomFactory::Xoroshiro { seed_lo, seed_hi } => {
                AnyRandom::Xoroshiro(XoroshiroRandom::from_seeds(seed ^ seed_lo, seed ^ seed_hi))
            }
        }
    }
}

/// `RandomSupport.GOLDEN_RATIO_64`.
pub const GOLDEN_RATIO_64: i64 = -7_046_029_254_386_353_131;
/// `RandomSupport.SILVER_RATIO_64`.
pub const SILVER_RATIO_64: i64 = 7_640_891_576_956_012_809;

/// `RandomSupport.mixStafford13`.
#[inline]
pub fn mix_stafford13(mut z: i64) -> i64 {
    z = (z ^ ((z as u64) >> 30) as i64).wrapping_mul(-4_658_895_280_553_007_687);
    z = (z ^ ((z as u64) >> 27) as i64).wrapping_mul(-7_723_592_293_110_705_685);
    z ^ ((z as u64) >> 31) as i64
}

/// `RandomSupport.upgradeSeedTo128bit` (mixed).
pub fn upgrade_seed_to_128bit(seed: i64) -> (i64, i64) {
    let lo = seed ^ SILVER_RATIO_64;
    let hi = lo.wrapping_add(GOLDEN_RATIO_64);
    (mix_stafford13(lo), mix_stafford13(hi))
}

/// `RandomSupport.seedFromHashOf`: MD5 of the UTF-8 string, big-endian halves.
pub fn seed_from_hash_of(name: &str) -> (i64, i64) {
    let digest = Md5::digest(name.as_bytes());
    let lo = i64::from_be_bytes(digest[0..8].try_into().unwrap());
    let hi = i64::from_be_bytes(digest[8..16].try_into().unwrap());
    (lo, hi)
}

/// `MarsagliaPolarGaussian` state shared by the random implementations.
#[derive(Clone, Default)]
pub(crate) struct Gaussian {
    next_next: f64,
    have_next_next: bool,
}

impl Gaussian {
    pub(crate) fn reset(&mut self) {
        self.have_next_next = false;
    }

    pub(crate) fn next(&mut self, next_double: &mut dyn FnMut() -> f64) -> f64 {
        if self.have_next_next {
            self.have_next_next = false;
            return self.next_next;
        }
        loop {
            let x = 2.0 * next_double() - 1.0;
            let y = 2.0 * next_double() - 1.0;
            let r = x * x + y * y;
            if r >= 1.0 || r == 0.0 {
                continue;
            }
            let multiplier = (-2.0 * r.ln() / r).sqrt();
            self.next_next = y * multiplier;
            self.have_next_next = true;
            return x * multiplier;
        }
    }
}

#[cfg(test)]
mod tests;
