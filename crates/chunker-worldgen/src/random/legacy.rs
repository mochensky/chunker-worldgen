use super::{AnyRandom, DOUBLE_UNIT, FLOAT_UNIT, Gaussian, PositionalRandomFactory, RandomSource};

const MULTIPLIER: i64 = 25_214_903_917;
const INCREMENT: i64 = 11;
const MASK: i64 = (1 << 48) - 1;

/// `LegacyRandomSource` (java.util.Random LCG) including the `BitRandomSource` defaults.
#[derive(Clone)]
pub struct LegacyRandom {
    seed: i64,
    gaussian: Gaussian,
}

impl LegacyRandom {
    pub fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ MULTIPLIER) & MASK,
            gaussian: Gaussian::default(),
        }
    }

    #[inline]
    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT)) & MASK;
        (self.seed >> (48 - bits)) as i32
    }
}

impl RandomSource for LegacyRandom {
    fn set_seed(&mut self, seed: i64) {
        self.seed = (seed ^ MULTIPLIER) & MASK;
        self.gaussian.reset();
    }

    #[inline]
    fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        bit_next_int_bounded(bound, &mut |bits| self.next(bits))
    }

    fn next_long(&mut self) -> i64 {
        let upper = self.next(32) as i64;
        let lower = self.next(32) as i64;
        (upper << 32).wrapping_add(lower)
    }

    fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * FLOAT_UNIT
    }

    fn next_double(&mut self) -> f64 {
        let upper = self.next(26) as i64;
        let lower = self.next(27) as i64;
        ((upper << 27) + lower) as f64 * DOUBLE_UNIT
    }

    fn next_gaussian(&mut self) -> f64 {
        let mut g = std::mem::take(&mut self.gaussian);
        let v = g.next(&mut || self.next_double());
        self.gaussian = g;
        v
    }

    fn fork(&mut self) -> AnyRandom {
        AnyRandom::Legacy(LegacyRandom::new(self.next_long()))
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        PositionalRandomFactory::Legacy {
            seed: self.next_long(),
        }
    }
}

/// `BitRandomSource.nextInt(int)`.
#[inline]
pub(crate) fn bit_next_int_bounded(bound: i32, next: &mut dyn FnMut(u32) -> i32) -> i32 {
    assert!(bound > 0, "Bound must be positive");
    if bound & (bound - 1) == 0 {
        return ((bound as i64 * next(31) as i64) >> 31) as i32;
    }
    loop {
        let sample = next(31);
        let modulo = sample % bound;
        if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
            return modulo;
        }
    }
}
