use super::legacy::bit_next_int_bounded;
use super::{AnyRandom, DOUBLE_UNIT, FLOAT_UNIT, Gaussian, PositionalRandomFactory, RandomSource};

/// `WorldgenRandom`: a `LegacyRandomSource` subclass whose `next(bits)` delegates to a wrapped
/// source. All `BitRandomSource` helpers (`nextInt(bound)`, `nextDouble`, ...) are therefore built
/// on top of `next(bits)` even when the wrapped source is Xoroshiro.
#[derive(Clone)]
pub struct WorldgenRandom {
    inner: AnyRandom,
    gaussian: Gaussian,
    count: i32,
}

impl WorldgenRandom {
    pub fn new(inner: AnyRandom) -> Self {
        Self {
            inner,
            gaussian: Gaussian::default(),
            count: 0,
        }
    }

    pub fn count(&self) -> i32 {
        self.count
    }

    #[inline]
    pub fn next(&mut self, bits: u32) -> i32 {
        self.count += 1;
        match &mut self.inner {
            AnyRandom::Legacy(l) => l.next(bits),
            AnyRandom::Xoroshiro(x) => ((x.next_long() as u64) >> (64 - bits)) as i32,
        }
    }

    /// `setDecorationSeed(seed, chunkX, chunkZ)` (block coordinates in practice).
    pub fn set_decoration_seed(&mut self, seed: i64, chunk_x: i32, chunk_z: i32) -> i64 {
        self.set_seed(seed);
        let x_scale = self.next_long() | 1;
        let z_scale = self.next_long() | 1;
        let result = ((chunk_x as i64).wrapping_mul(x_scale))
            .wrapping_add((chunk_z as i64).wrapping_mul(z_scale))
            ^ seed;
        self.set_seed(result);
        result
    }

    /// `setFeatureSeed(seed, index, step)`.
    pub fn set_feature_seed(&mut self, seed: i64, index: i32, step: i32) {
        let result = seed
            .wrapping_add(index as i64)
            .wrapping_add((10000i32.wrapping_mul(step)) as i64);
        self.set_seed(result);
    }

    /// `setLargeFeatureSeed(seed, chunkX, chunkZ)`.
    pub fn set_large_feature_seed(&mut self, seed: i64, chunk_x: i32, chunk_z: i32) {
        self.set_seed(seed);
        let x_scale = self.next_long();
        let z_scale = self.next_long();
        let result = (chunk_x as i64).wrapping_mul(x_scale) ^ (chunk_z as i64).wrapping_mul(z_scale) ^ seed;
        self.set_seed(result);
    }

    /// `setLargeFeatureWithSalt(seed, x, z, salt)`.
    pub fn set_large_feature_with_salt(&mut self, seed: i64, x: i32, z: i32, salt: i32) {
        let result = (x as i64)
            .wrapping_mul(341_873_128_712)
            .wrapping_add((z as i64).wrapping_mul(132_897_987_541))
            .wrapping_add(seed)
            .wrapping_add(salt as i64);
        self.set_seed(result);
    }
}

impl RandomSource for WorldgenRandom {
    fn set_seed(&mut self, seed: i64) {
        // Vanilla `WorldgenRandom.setSeed` only reseeds the wrapped source; the gaussian state of
        // the LegacyRandomSource super class is not reset.
        self.inner.set_seed(seed);
    }

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
        self.inner.fork()
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        self.inner.fork_positional()
    }
}
