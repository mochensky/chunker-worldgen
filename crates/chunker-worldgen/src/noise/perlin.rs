use super::gradient::{GradientNoise, grad_dot};
use crate::density::DensityVolume;
use crate::math::{Interval, floor, lerp3_f32, smoothstep};
use crate::random::RandomSource;

/// `PerlinNoise` (single octave, 26.3 float version).
#[derive(Clone)]
pub struct PerlinNoise {
    pub base: GradientNoise,
}

impl PerlinNoise {
    pub fn new(random: &mut dyn RandomSource) -> Self {
        Self {
            base: GradientNoise::new(random, 256.0),
        }
    }

    pub fn range() -> Interval {
        Interval::of_symmetric(2.0)
    }

    #[inline]
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        self.get(GradientNoise::wrap(x), 0.0, GradientNoise::wrap(y))
    }

    #[inline]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let b = &self.base;
        let x = GradientNoise::wrap(x) + b.offset_x;
        let y = GradientNoise::wrap(y) + b.offset_y;
        let z = GradientNoise::wrap(z) + b.offset_z;
        let fx = floor(x);
        let fy = floor(y);
        let fz = floor(z);
        let rx = (x - fx as f64) as f32;
        let ry = (y - fy as f64) as f32;
        let rz = (z - fz as f64) as f32;
        sample_and_lerp(b, fx, fy, fz, rx, ry, rz, ry)
    }

    /// `PerlinNoise.addToVolume`.
    pub fn add_to_volume(
        &self,
        buf: &mut [f32],
        volume: &DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) {
        add_to_volume_impl(
            &self.base,
            buf,
            volume,
            xz_scale,
            y_scale,
            amplitude,
            |_, relative_y| relative_y as f32,
        );
    }
}

/// `PerlinNoise.sampleAndLerp`.
#[inline(always)]
pub(crate) fn sample_and_lerp(
    b: &GradientNoise,
    x: i32,
    y: i32,
    z: i32,
    rx: f32,
    ry: f32,
    rz: f32,
    original_ry: f32,
) -> f32 {
    let x0 = b.permute(x);
    let x1 = b.permute(x + 1);
    let xy00 = b.permute(x0 + y);
    let xy01 = b.permute(x0 + y + 1);
    let xy10 = b.permute(x1 + y);
    let xy11 = b.permute(x1 + y + 1);
    let d000 = grad_dot(b.permute(xy00 + z), rx, ry, rz);
    let d100 = grad_dot(b.permute(xy10 + z), rx - 1.0, ry, rz);
    let d010 = grad_dot(b.permute(xy01 + z), rx, ry - 1.0, rz);
    let d110 = grad_dot(b.permute(xy11 + z), rx - 1.0, ry - 1.0, rz);
    let d001 = grad_dot(b.permute(xy00 + z + 1), rx, ry, rz - 1.0);
    let d101 = grad_dot(b.permute(xy10 + z + 1), rx - 1.0, ry, rz - 1.0);
    let d011 = grad_dot(b.permute(xy01 + z + 1), rx, ry - 1.0, rz - 1.0);
    let d111 = grad_dot(b.permute(xy11 + z + 1), rx - 1.0, ry - 1.0, rz - 1.0);
    let xa = smoothstep(rx);
    let ya = smoothstep(original_ry);
    let za = smoothstep(rz);
    lerp3_f32(xa, ya, za, d000, d100, d010, d110, d001, d101, d011, d111)
}

/// Shared body of `PerlinNoise.addToVolume` / `SmearedPerlinNoise.addToVolume`.
/// `fudge(original_y, relative_y)` returns the (possibly smeared) relative y used for the gradient
/// y-term; the interpolation alpha always uses the unsmeared relative y.
///
/// Each column is processed in two passes: a scalar pass resolves the lattice cell and gradients of
/// every y (table lookups), then a branch-free pass does the float math, which the compiler
/// vectorizes across y. The float operations and their order are exactly the per-point ones.
#[inline]
fn add_to_volume_impl(
    b: &GradientNoise,
    buf: &mut [f32],
    volume: &DensityVolume,
    xz_scale: f64,
    y_scale: f64,
    amplitude: f32,
    fudge: impl Fn(f64, f64) -> f32,
) {
    add_to_volume_body(b, buf, volume, xz_scale, y_scale, amplitude, fudge)
}

#[inline(always)]
fn add_to_volume_body(
    b: &GradientNoise,
    buf: &mut [f32],
    volume: &DensityVolume,
    xz_scale: f64,
    y_scale: f64,
    amplitude: f32,
    fudge: impl Fn(f64, f64) -> f32,
) {
    const LANES: usize = 64;
    // Per-y inputs of the arithmetic pass: xz dot products and y gradients of the 8 corners.
    let mut dots = [[0.0f32; LANES]; 8];
    let mut grads_y = [[0.0f32; LANES]; 8];
    let mut rys = [0.0f32; LANES];
    let mut ays = [0.0f32; LANES];
    let mut d = [0.0f32; 8];
    let mut gy = [0.0f32; 8];
    let size_y = volume.size_y as usize;
    let mut index = 0usize;
    for iz in 0..volume.size_z {
        let z = GradientNoise::wrap(volume.block_z(iz) as f64 * xz_scale) + b.offset_z;
        let fz = floor(z);
        let rz = (z - fz as f64) as f32;
        let az = smoothstep(rz);
        for ix in 0..volume.size_x {
            let x = GradientNoise::wrap(volume.block_x(ix) as f64 * xz_scale) + b.offset_x;
            let fx = floor(x);
            let rx = (x - fx as f64) as f32;
            let x0 = b.permute(fx);
            let x1 = b.permute(fx + 1);
            let ax = smoothstep(rx);
            let mut last_fy = i32::MIN;
            let mut y0 = 0usize;
            while y0 < size_y {
                let n = LANES.min(size_y - y0);
                for k in 0..n {
                    let original_y = volume.block_y((y0 + k) as i32) as f64 * y_scale;
                    let y = GradientNoise::wrap(original_y) + b.offset_y;
                    let fy = floor(y);
                    let relative_y = y - fy as f64;
                    ays[k] = smoothstep(relative_y as f32);
                    if last_fy != fy {
                        let xy00 = b.permute(x0 + fy);
                        let xy01 = b.permute(x0 + fy + 1);
                        let xy10 = b.permute(x1 + fy);
                        let xy11 = b.permute(x1 + fy + 1);
                        let corners = [
                            (xy00 + fz, rx, rz),
                            (xy10 + fz, rx - 1.0, rz),
                            (xy01 + fz, rx, rz),
                            (xy11 + fz, rx - 1.0, rz),
                            (xy00 + fz + 1, rx, rz - 1.0),
                            (xy10 + fz + 1, rx - 1.0, rz - 1.0),
                            (xy01 + fz + 1, rx, rz - 1.0),
                            (xy11 + fz + 1, rx - 1.0, rz - 1.0),
                        ];
                        for (i, &(h, cx, cz)) in corners.iter().enumerate() {
                            let g = b.grad(h);
                            // `Gradient.dotXz`: x * gx + z * gz.
                            d[i] = g[0] * cx + g[2] * cz;
                            gy[i] = g[1];
                        }
                        last_fy = fy;
                    }
                    for i in 0..8 {
                        dots[i][k] = d[i];
                        grads_y[i][k] = gy[i];
                    }
                    rys[k] = fudge(original_y, relative_y);
                }
                let out = &mut buf[index..index + n];
                for k in 0..n {
                    let ry = rys[k];
                    let ry1 = ry - 1.0;
                    let v = lerp3_f32(
                        ax,
                        ays[k],
                        az,
                        dots[0][k] + grads_y[0][k] * ry,
                        dots[1][k] + grads_y[1][k] * ry,
                        dots[2][k] + grads_y[2][k] * ry1,
                        dots[3][k] + grads_y[3][k] * ry1,
                        dots[4][k] + grads_y[4][k] * ry,
                        dots[5][k] + grads_y[5][k] * ry,
                        dots[6][k] + grads_y[6][k] * ry1,
                        dots[7][k] + grads_y[7][k] * ry1,
                    );
                    out[k] += amplitude * v;
                }
                index += n;
                y0 += n;
            }
        }
    }
}

/// `SmearedPerlinNoise` (deprecated "old blended noise" octave).
#[derive(Clone)]
pub struct SmearedPerlinNoise {
    pub base: GradientNoise,
    pub fudge_y_scale: f64,
}

/// `SHIFT_UP_EPSILON = 1.0E-7F` promoted to double.
const SHIFT_UP_EPSILON: f64 = 1.0e-7_f32 as f64;

impl SmearedPerlinNoise {
    pub fn new(random: &mut dyn RandomSource, fudge_y_scale: f64) -> Self {
        Self {
            base: GradientNoise::new(random, 256.0),
            fudge_y_scale,
        }
    }

    pub fn range_for(fudge_y_scale: f64) -> Interval {
        Interval::of_symmetric((fudge_y_scale.abs() + 2.0) as f32)
    }

    #[inline(always)]
    fn compute_fudge_y(&self, original_y: f64, relative_y: f64) -> f64 {
        let limit = if original_y >= 0.0 && original_y < relative_y {
            original_y
        } else {
            relative_y
        };
        floor(limit / self.fudge_y_scale + SHIFT_UP_EPSILON) as f64 * self.fudge_y_scale
    }

    pub fn get(&self, ox: f64, oy: f64, oz: f64) -> f32 {
        let b = &self.base;
        let x = GradientNoise::wrap(ox) + b.offset_x;
        let y = GradientNoise::wrap(oy) + b.offset_y;
        let z = GradientNoise::wrap(oz) + b.offset_z;
        let fx = floor(x);
        let fy = floor(y);
        let fz = floor(z);
        let rx = (x - fx as f64) as f32;
        let ry = y - fy as f64;
        let rz = (z - fz as f64) as f32;
        let fudged = (ry - self.compute_fudge_y(oy, ry)) as f32;
        sample_and_lerp(b, fx, fy, fz, rx, fudged, rz, ry as f32)
    }

    pub fn get2(&self, x: f64, y: f64) -> f32 {
        // `PerlinNoise.get(x, y)` is inherited and dispatches to the overridden 3D `get`.
        self.get(GradientNoise::wrap(x), 0.0, GradientNoise::wrap(y))
    }

    pub fn add_to_volume(
        &self,
        buf: &mut [f32],
        volume: &DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) {
        add_to_volume_impl(
            &self.base,
            buf,
            volume,
            xz_scale,
            y_scale,
            amplitude,
            |original_y, relative_y| (relative_y - self.compute_fudge_y(original_y, relative_y)) as f32,
        );
    }
}
