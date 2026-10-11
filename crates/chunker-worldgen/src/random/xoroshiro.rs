use super::{
    AnyRandom, DOUBLE_UNIT, FLOAT_UNIT, GOLDEN_RATIO_64, Gaussian, PositionalRandomFactory, RandomSource,
    SILVER_RATIO_64, upgrade_seed_to_128bit,
};

#[derive(Clone, Copy, Debug)]
pub struct Xoroshiro128PlusPlus {
    lo: i64,
    hi: i64,
}

impl Xoroshiro128PlusPlus {
    pub fn new(lo: i64, hi: i64) -> Self {
        if (lo | hi) == 0 {
            Self {
                lo: GOLDEN_RATIO_64,
                hi: SILVER_RATIO_64,
            }
        } else {
            Self { lo, hi }
        }
    }

    #[inline(always)]
    pub fn next_long(&mut self) -> i64 {
        let s0 = self.lo;
        let mut s1 = self.hi;
        let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        result
    }
}

/// `XoroshiroRandomSource`.
#[derive(Clone)]
pub struct XoroshiroRandom {
    rng: Xoroshiro128PlusPlus,
    gaussian: Gaussian,
}

impl XoroshiroRandom {
    pub fn new(seed: i64) -> Self {
        let (lo, hi) = upgrade_seed_to_128bit(seed);
        Self::from_seeds(lo, hi)
    }

    pub fn from_seeds(lo: i64, hi: i64) -> Self {
        Self {
            rng: Xoroshiro128PlusPlus::new(lo, hi),
            gaussian: Gaussian::default(),
        }
    }

    #[inline(always)]
    fn next_bits(&mut self, bits: u32) -> i64 {
        ((self.rng.next_long() as u64) >> (64 - bits)) as i64
    }
}

impl RandomSource for XoroshiroRandom {
    fn set_seed(&mut self, seed: i64) {
        let (lo, hi) = upgrade_seed_to_128bit(seed);
        self.rng = Xoroshiro128PlusPlus::new(lo, hi);
        self.gaussian.reset();
    }

    #[inline]
    fn next_int(&mut self) -> i32 {
        self.rng.next_long() as i32
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "Bound must be positive");
        let bound64 = bound as i64;
        let mut random_bits = (self.next_int() as u32) as i64;
        let mut multiplied = random_bits.wrapping_mul(bound64);
        let mut fractional = multiplied & 0xFFFF_FFFF;
        if fractional < bound64 {
            let threshold = ((!bound).wrapping_add(1) as u32 % bound as u32) as i64;
            while fractional < threshold {
                random_bits = (self.next_int() as u32) as i64;
                multiplied = random_bits.wrapping_mul(bound64);
                fractional = multiplied & 0xFFFF_FFFF;
            }
        }
        (multiplied >> 32) as i32
    }

    #[inline]
    fn next_long(&mut self) -> i64 {
        self.rng.next_long()
    }

    fn next_bool(&mut self) -> bool {
        self.rng.next_long() & 1 != 0
    }

    fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * FLOAT_UNIT
    }

    fn next_double(&mut self) -> f64 {
        self.next_bits(53) as f64 * DOUBLE_UNIT
    }

    fn next_gaussian(&mut self) -> f64 {
        let mut g = std::mem::take(&mut self.gaussian);
        let v = g.next(&mut || self.next_double());
        self.gaussian = g;
        v
    }

    fn fork(&mut self) -> AnyRandom {
        let lo = self.rng.next_long();
        let hi = self.rng.next_long();
        AnyRandom::Xoroshiro(XoroshiroRandom::from_seeds(lo, hi))
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        let seed_lo = self.rng.next_long();
        let seed_hi = self.rng.next_long();
        PositionalRandomFactory::Xoroshiro { seed_lo, seed_hi }
    }

    fn consume_count(&mut self, rounds: i32) {
        for _ in 0..rounds {
            self.rng.next_long();
        }
    }
}
