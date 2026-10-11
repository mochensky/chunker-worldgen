//! `BiomeManager`: block-resolution biome lookup with the fuzzy "zoom" offset.

use sha2::{Digest, Sha256};

use super::BiomeId;

/// `BiomeManager.obfuscateSeed` = Guava `sha256().hashLong(seed).asLong()` (little endian).
pub fn obfuscate_seed(seed: i64) -> i64 {
    let digest = Sha256::digest(seed.to_le_bytes());
    i64::from_le_bytes(digest[0..8].try_into().unwrap())
}

/// `LinearCongruentialGenerator.next`.
#[inline(always)]
fn lcg_next(rval: i64, c: i64) -> i64 {
    rval.wrapping_mul(
        rval.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(c)
}

#[inline(always)]
fn fiddle(rval: i64) -> f64 {
    let uniform = crate::math::floor_mod_i64(rval >> 24, 1024) as f64 / 1024.0;
    (uniform - 0.5) * 0.9
}

#[inline]
fn fiddled_distance(seed: i64, x: i32, y: i32, z: i32, dx: f64, dy: f64, dz: f64) -> f64 {
    let (x, y, z) = (x as i64, y as i64, z as i64);
    let mut r = lcg_next(seed, x);
    r = lcg_next(r, y);
    r = lcg_next(r, z);
    r = lcg_next(r, x);
    r = lcg_next(r, y);
    r = lcg_next(r, z);
    let fx = fiddle(r);
    r = lcg_next(r, seed);
    let fy = fiddle(r);
    r = lcg_next(r, seed);
    let fz = fiddle(r);
    let sq = |v: f64| v * v;
    sq(dz + fz) + sq(dy + fy) + sq(dx + fx)
}

#[derive(Clone, Copy, Debug)]
pub struct BiomeManager {
    pub zoom_seed: i64,
}

impl BiomeManager {
    pub fn new(world_seed: i64) -> Self {
        Self {
            zoom_seed: obfuscate_seed(world_seed),
        }
    }

    /// `getBiome(x, y, z)`; `noise_biome` resolves quart coordinates.
    pub fn get_biome(
        &self,
        x: i32,
        y: i32,
        z: i32,
        noise_biome: &mut dyn FnMut(i32, i32, i32) -> BiomeId,
    ) -> BiomeId {
        let (ax, ay, az) = (x - 2, y - 2, z - 2);
        let (px, py, pz) = (ax >> 2, ay >> 2, az >> 2);
        let fx = (ax & 3) as f64 / 4.0;
        let fy = (ay & 3) as f64 / 4.0;
        let fz = (az & 3) as f64 / 4.0;
        // Fast path: the fiddled pick only matters when the 8 candidate cells disagree.
        let first = noise_biome(px, py, pz);
        let uniform =
            (1..8).all(|i| noise_biome(px + ((i >> 2) & 1), py + ((i >> 1) & 1), pz + (i & 1)) == first);
        if uniform {
            return first;
        }
        let mut min_i = 0;
        let mut min_d = f64::INFINITY;
        for i in 0..8 {
            let xe = i & 4 == 0;
            let ye = i & 2 == 0;
            let ze = i & 1 == 0;
            let d = fiddled_distance(
                self.zoom_seed,
                if xe { px } else { px + 1 },
                if ye { py } else { py + 1 },
                if ze { pz } else { pz + 1 },
                if xe { fx } else { fx - 1.0 },
                if ye { fy } else { fy - 1.0 },
                if ze { fz } else { fz - 1.0 },
            );
            if min_d > d {
                min_i = i;
                min_d = d;
            }
        }
        noise_biome(
            if min_i & 4 == 0 { px } else { px + 1 },
            if min_i & 2 == 0 { py } else { py + 1 },
            if min_i & 1 == 0 { pz } else { pz + 1 },
        )
    }
}
