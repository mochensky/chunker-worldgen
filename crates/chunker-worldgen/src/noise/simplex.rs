use super::gradient::{GRADIENT, GradientNoise};
use crate::math::floor;
use crate::random::RandomSource;

const SQRT_3: f64 = 1.7320508075688772;
const F2: f64 = 0.5 * (SQRT_3 - 1.0);
const G2: f64 = (3.0 - SQRT_3) / 6.0;

/// `SimplexNoise` (double precision, result cast to float).
#[derive(Clone)]
pub struct SimplexNoise {
    pub base: GradientNoise,
}

impl SimplexNoise {
    pub fn new(random: &mut dyn RandomSource) -> Self {
        Self {
            base: GradientNoise::new(random, 256.0),
        }
    }

    /// `new SimplexNoise(random, discardNoiseOffset)`.
    pub fn new_discard_offset(random: &mut dyn RandomSource, discard: bool) -> Self {
        Self {
            base: GradientNoise::new(random, if discard { 0.0 } else { 256.0 }),
        }
    }

    #[inline(always)]
    fn corner(index: i32, x: f64, y: f64, z: f64, base: f64) -> f64 {
        let mut t0 = base - x * x - y * y - z * z;
        if t0 < 0.0 {
            0.0
        } else {
            t0 *= t0;
            let g = GRADIENT[index as usize];
            t0 * t0 * (g[0] as f64 * x + g[1] as f64 * y + g[2] as f64 * z)
        }
    }

    pub fn get2(&self, xin: f64, yin: f64) -> f32 {
        let b = &self.base;
        let xin = xin + b.offset_x;
        let yin = yin + b.offset_y;
        let s = (xin + yin) * F2;
        let i = floor(xin + s);
        let j = floor(yin + s);
        let t = (i.wrapping_add(j)) as f64 * G2;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + G2;
        let y1 = y0 - j1 as f64 + G2;
        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;
        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let gi0 = b.permute(ii + b.permute(jj)) % 12;
        let gi1 = b.permute(ii + i1 + b.permute(jj + j1)) % 12;
        let gi2 = b.permute(ii + 1 + b.permute(jj + 1)) % 12;
        let n0 = Self::corner(gi0, x0, y0, 0.0, 0.5);
        let n1 = Self::corner(gi1, x1, y1, 0.0, 0.5);
        let n2 = Self::corner(gi2, x2, y2, 0.0, 0.5);
        (70.0 * (n0 + n1 + n2)) as f32
    }

    pub fn get(&self, xin: f64, yin: f64, zin: f64) -> f32 {
        let b = &self.base;
        let xin = xin + b.offset_x;
        let yin = yin + b.offset_y;
        let zin = zin + b.offset_z;
        const F3: f64 = 0.3333333333333333;
        const G3: f64 = 0.16666666666666666;
        let s = (xin + yin + zin) * F3;
        let i = floor(xin + s);
        let j = floor(yin + s);
        let k = floor(zin + s);
        let t = (i.wrapping_add(j).wrapping_add(k)) as f64 * G3;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let z0 = zin - (k as f64 - t);
        let (i1, j1, k1, i2, j2, k2);
        if x0 >= y0 {
            if y0 >= z0 {
                (i1, j1, k1, i2, j2, k2) = (1, 0, 0, 1, 1, 0);
            } else if x0 >= z0 {
                (i1, j1, k1, i2, j2, k2) = (1, 0, 0, 1, 0, 1);
            } else {
                (i1, j1, k1, i2, j2, k2) = (0, 0, 1, 1, 0, 1);
            }
        } else if y0 < z0 {
            (i1, j1, k1, i2, j2, k2) = (0, 0, 1, 0, 1, 1);
        } else if x0 < z0 {
            (i1, j1, k1, i2, j2, k2) = (0, 1, 0, 0, 1, 1);
        } else {
            (i1, j1, k1, i2, j2, k2) = (0, 1, 0, 1, 1, 0);
        }
        let x1 = x0 - i1 as f64 + G3;
        let y1 = y0 - j1 as f64 + G3;
        let z1 = z0 - k1 as f64 + G3;
        let x2 = x0 - i2 as f64 + F3;
        let y2 = y0 - j2 as f64 + F3;
        let z2 = z0 - k2 as f64 + F3;
        let x3 = x0 - 1.0 + 0.5;
        let y3 = y0 - 1.0 + 0.5;
        let z3 = z0 - 1.0 + 0.5;
        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let kk = k & 0xFF;
        let gi0 = b.permute(ii + b.permute(jj + b.permute(kk))) % 12;
        let gi1 = b.permute(ii + i1 + b.permute(jj + j1 + b.permute(kk + k1))) % 12;
        let gi2 = b.permute(ii + i2 + b.permute(jj + j2 + b.permute(kk + k2))) % 12;
        let gi3 = b.permute(ii + 1 + b.permute(jj + 1 + b.permute(kk + 1))) % 12;
        let n0 = Self::corner(gi0, x0, y0, z0, 0.6);
        let n1 = Self::corner(gi1, x1, y1, z1, 0.6);
        let n2 = Self::corner(gi2, x2, y2, z2, 0.6);
        let n3 = Self::corner(gi3, x3, y3, z3, 0.6);
        (32.0 * (n0 + n1 + n2 + n3)) as f32
    }
}

impl std::fmt::Debug for SimplexNoise {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SimplexNoise")
    }
}
