//! Compiled density samplers (`DensitySampler` implementations of 26.3).
//!
//! Each variant ports one vanilla sampler class with both `sampleValue` and `sampleVolume`.
//! Operation order and float/double promotions follow the Java source exactly.

use std::sync::Arc;

use super::context::SamplerContext;
use super::spline::CompiledSpline;
use super::{Axis, DensityVolume, DistanceMetric, RoundKind};
use crate::math::{
    clamp_f32, cube_f32, floor_div, floor_f32, floor_mod, lerp_f32, lerp3_f32, max_f32, min_f32, signum_f32,
    sqrt_f32,
};
use crate::noise::{NoiseStack, SimplexNoise};

pub type SamplerRef = Arc<Sampler>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKey {
    BlendAlpha,
    BlendOffset,
    Beardifier,
}

#[derive(Debug)]
pub enum GradientMode {
    Clamped { min: i32, max: i32 },
    Repeat { range: i32 },
    MirroredRepeat { range: i32 },
}

#[derive(Debug)]
pub enum Sampler {
    Constant(f32),
    /// A chunk's `Beardifier`, bound into the context (never produced by the compiler).
    Beardifier(Arc<crate::terrain::beardifier::Beardifier>),
    ContextBound {
        key: ContextKey,
        fallback: f32,
    },
    Noise {
        noise: Arc<NoiseStack>,
        xz_scale: f64,
        y_scale: f64,
    },
    NoiseShiftedXz {
        shift_x: SamplerRef,
        shift_z: SamplerRef,
        noise: Arc<NoiseStack>,
        xz_scale: f64,
        y_scale: f64,
    },
    NoiseShiftedXyz {
        shift_x: SamplerRef,
        shift_y: SamplerRef,
        shift_z: SamplerRef,
        noise: Arc<NoiseStack>,
        xz_scale: f64,
        y_scale: f64,
    },
    ShiftB {
        noise: Arc<NoiseStack>,
    },
    EndIslands {
        noise: Box<SimplexNoise>,
    },
    DistanceToPoint {
        x: i32,
        y: i32,
        z: i32,
        metric: DistanceMetric,
    },
    Gradient {
        axis: Axis,
        from_coordinate: i32,
        from_value: f32,
        factor: f32,
        mode: GradientMode,
    },
    Abs(SamplerRef),
    Square(SamplerRef),
    Cube(SamplerRef),
    Sqrt(SamplerRef),
    LeakyRelu(SamplerRef, f32),
    Reciprocal(SamplerRef),
    Negate(SamplerRef),
    Squeeze(SamplerRef),
    Log(SamplerRef),
    Sign(SamplerRef),
    RoundInteger(RoundKind, SamplerRef),
    Round(RoundKind, SamplerRef, SamplerRef),
    Add(SamplerRef, SamplerRef),
    ConstAdd(SamplerRef, f32),
    Sub(SamplerRef, SamplerRef),
    ConstSub(f32, SamplerRef),
    Mul(SamplerRef, SamplerRef),
    ConstMul(SamplerRef, f32),
    Div(SamplerRef, SamplerRef),
    ConstDiv(f32, SamplerRef),
    Min {
        left: SamplerRef,
        right: SamplerRef,
        right_min: f32,
    },
    ConstMin(SamplerRef, f32),
    Max {
        left: SamplerRef,
        right: SamplerRef,
        right_max: f32,
    },
    ConstMax(SamplerRef, f32),
    Pow(SamplerRef, SamplerRef),
    PowConstBase(f64, SamplerRef),
    PowConstExponent(SamplerRef, f64),
    Spline {
        spline: CompiledSpline,
        coordinates: Vec<SamplerRef>,
    },
    Lerp(SamplerRef, SamplerRef, SamplerRef),
    LerpConstFirst(SamplerRef, f32, SamplerRef),
    LerpConstSecond(SamplerRef, SamplerRef, f32),
    Clamp(SamplerRef, f32, f32),
    RangeChoice {
        input: SamplerRef,
        min: f32,
        max: f32,
        in_range: SamplerRef,
        out_of_range: SamplerRef,
    },
    RangeChoiceConst {
        input: SamplerRef,
        min: f32,
        max: f32,
        in_range: f32,
        out_of_range: f32,
    },
    IntervalSelect {
        input: SamplerRef,
        thresholds: Vec<f32>,
        samplers: Vec<SamplerRef>,
    },
    IntervalSelectSingle {
        input: SamplerRef,
        threshold: f32,
        if_below: SamplerRef,
        if_above: SamplerRef,
    },
    Caching {
        id: usize,
        input: SamplerRef,
    },
    BlendDensity(SamplerRef),
    Interpolated {
        input: SamplerRef,
        cell_xz: i32,
        cell_y: i32,
        inv_xz: f32,
        inv_y: f32,
    },
    SliceX(SamplerRef, i32),
    SliceY(SamplerRef, i32),
    SliceZ(SamplerRef, i32),
    SliceXz(SamplerRef, i32, i32),
    FindTopSurface {
        density: SamplerRef,
        upper_bound: SamplerRef,
        lower_bound: i32,
        cell_height: i32,
    },
}

/// `UnaryFunction.LeakyReLUSampler.apply`.
#[inline(always)]
pub fn leaky_relu(negative_factor: f32, input: f32) -> f32 {
    if input > 0.0 {
        input
    } else {
        input * negative_factor
    }
}

/// `UnaryFunction.SqueezeSampler.apply`.
#[inline(always)]
pub fn squeeze(input: f32) -> f32 {
    let c = clamp_f32(input, -1.0, 1.0);
    c / 2.0 - cube_f32(c) / 24.0
}

#[inline(always)]
fn jpow(base: f64, exp: f64) -> f32 {
    base.powf(exp) as f32
}

/// `EndIslandFunction.getHeightValue`.
fn end_island_height(noise: &SimplexNoise, section_x: i32, section_z: i32) -> f32 {
    let chunk_x = section_x / 2;
    let chunk_z = section_z / 2;
    let sub_x = section_x % 2;
    let sub_z = section_z % 2;
    let mut doffs = -100.0f32;
    for xo in -12..=12 {
        for zo in -12..=12 {
            let tx = (chunk_x + xo) as i64;
            let tz = (chunk_z + zo) as i64;
            if tx * tx + tz * tz > 4096 && noise.get2(tx as f64, tz as f64) < -0.9 {
                let island_size = ((tx as f32).abs() * 3439.0 + (tz as f32).abs() * 147.0) % 13.0 + 9.0;
                let xd = (sub_x - xo * 2) as f32;
                let zd = (sub_z - zo * 2) as f32;
                let mut nd = 100.0 - sqrt_f32(xd * xd + zd * zd) * island_size;
                nd = clamp_f32(nd, -100.0, 80.0);
                doffs = max_f32(doffs, nd);
            }
        }
    }
    doffs
}

impl Sampler {
    /// `sampleValue(context, x, y, z)`.
    pub fn sample_value(&self, ctx: &mut SamplerContext, x: i32, y: i32, z: i32) -> f32 {
        use Sampler as S;
        match self {
            S::Constant(v) => *v,
            S::Beardifier(b) => b.sample_value(x, y, z),
            S::ContextBound { key, fallback } => match ctx_field(ctx, *key).take() {
                Some(s) => {
                    let v = s.sample_value(ctx, x, y, z);
                    *ctx_field(ctx, *key) = Some(s);
                    v
                }
                None => *fallback,
            },
            S::Noise {
                noise,
                xz_scale,
                y_scale,
            } => noise.get(x as f64 * xz_scale, y as f64 * y_scale, z as f64 * xz_scale),
            S::NoiseShiftedXz {
                shift_x,
                shift_z,
                noise,
                xz_scale,
                y_scale,
            } => {
                let nx = x as f64 * xz_scale + shift_x.sample_value(ctx, x, y, z) as f64;
                let ny = y as f64 * y_scale;
                let nz = z as f64 * xz_scale + shift_z.sample_value(ctx, x, y, z) as f64;
                noise.get(nx, ny, nz)
            }
            S::NoiseShiftedXyz {
                shift_x,
                shift_y,
                shift_z,
                noise,
                xz_scale,
                y_scale,
            } => {
                let nx = x as f64 * xz_scale + shift_x.sample_value(ctx, x, y, z) as f64;
                let ny = y as f64 * y_scale + shift_y.sample_value(ctx, x, y, z) as f64;
                let nz = z as f64 * xz_scale + shift_z.sample_value(ctx, x, y, z) as f64;
                noise.get(nx, ny, nz)
            }
            S::ShiftB { noise } => noise.get(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0,
            S::EndIslands { noise } => (end_island_height(noise, x / 8, z / 8) - 8.0) / 128.0,
            S::DistanceToPoint {
                x: px,
                y: py,
                z: pz,
                metric,
            } => metric.compute((px - x) as f32, (py - y) as f32, (pz - z) as f32),
            S::Gradient { axis, .. } => self.gradient_compute(axis.choose(x, y, z)),
            S::Abs(i) => i.sample_value(ctx, x, y, z).abs(),
            S::Square(i) => {
                let v = i.sample_value(ctx, x, y, z);
                v * v
            }
            S::Cube(i) => cube_f32(i.sample_value(ctx, x, y, z)),
            S::Sqrt(i) => sqrt_f32(i.sample_value(ctx, x, y, z)),
            S::LeakyRelu(i, f) => leaky_relu(*f, i.sample_value(ctx, x, y, z)),
            S::Reciprocal(i) => 1.0 / i.sample_value(ctx, x, y, z),
            S::Negate(i) => -i.sample_value(ctx, x, y, z),
            S::Squeeze(i) => squeeze(i.sample_value(ctx, x, y, z)),
            S::Log(i) => (i.sample_value(ctx, x, y, z) as f64).ln() as f32,
            S::Sign(i) => signum_f32(i.sample_value(ctx, x, y, z)),
            S::RoundInteger(k, i) => k.apply(i.sample_value(ctx, x, y, z)),
            S::Round(k, i, m) => {
                let input = i.sample_value(ctx, x, y, z);
                let multiple = m.sample_value(ctx, x, y, z);
                round_apply(*k, input, multiple)
            }
            S::Add(l, r) => l.sample_value(ctx, x, y, z) + r.sample_value(ctx, x, y, z),
            S::ConstAdd(l, c) => l.sample_value(ctx, x, y, z) + c,
            S::Sub(l, r) => l.sample_value(ctx, x, y, z) - r.sample_value(ctx, x, y, z),
            S::ConstSub(c, r) => c - r.sample_value(ctx, x, y, z),
            S::Mul(l, r) => {
                let left = l.sample_value(ctx, x, y, z);
                if left == 0.0 {
                    0.0
                } else {
                    left * r.sample_value(ctx, x, y, z)
                }
            }
            S::ConstMul(l, c) => l.sample_value(ctx, x, y, z) * c,
            S::Div(l, r) => {
                let left = l.sample_value(ctx, x, y, z);
                if left == 0.0 {
                    0.0
                } else {
                    left / r.sample_value(ctx, x, y, z)
                }
            }
            S::ConstDiv(c, r) => c / r.sample_value(ctx, x, y, z),
            S::Min {
                left,
                right,
                right_min,
            } => {
                let l = left.sample_value(ctx, x, y, z);
                if l <= *right_min {
                    l
                } else {
                    min_f32(l, right.sample_value(ctx, x, y, z))
                }
            }
            S::ConstMin(l, c) => min_f32(l.sample_value(ctx, x, y, z), *c),
            S::Max {
                left,
                right,
                right_max,
            } => {
                let l = left.sample_value(ctx, x, y, z);
                if l >= *right_max {
                    l
                } else {
                    max_f32(l, right.sample_value(ctx, x, y, z))
                }
            }
            S::ConstMax(l, c) => max_f32(l.sample_value(ctx, x, y, z), *c),
            S::Pow(b, e) => {
                let base = b.sample_value(ctx, x, y, z);
                let exp = e.sample_value(ctx, x, y, z);
                jpow(base as f64, exp as f64)
            }
            S::PowConstBase(b, e) => jpow(*b, e.sample_value(ctx, x, y, z) as f64),
            S::PowConstExponent(b, e) => jpow(b.sample_value(ctx, x, y, z) as f64, *e),
            S::Spline { spline, coordinates } => {
                // `PointSplineInput`: lazily sampled, cached per coordinate (NaN = not cached).
                let mut cache = [f32::NAN; 16];
                let mut big: Vec<f32> = if coordinates.len() > 16 {
                    vec![f32::NAN; coordinates.len()]
                } else {
                    Vec::new()
                };
                spline.sample(&mut |i| {
                    let slot = if coordinates.len() > 16 {
                        &mut big[i]
                    } else {
                        &mut cache[i]
                    };
                    if !slot.is_nan() {
                        return *slot;
                    }
                    let v = coordinates[i].sample_value(ctx, x, y, z);
                    *slot = v;
                    v
                })
            }
            S::Lerp(a, f, s) => {
                let alpha = a.sample_value(ctx, x, y, z);
                if alpha == 0.0 {
                    f.sample_value(ctx, x, y, z)
                } else if alpha == 1.0 {
                    s.sample_value(ctx, x, y, z)
                } else {
                    let first = f.sample_value(ctx, x, y, z);
                    let second = s.sample_value(ctx, x, y, z);
                    lerp_f32(alpha, first, second)
                }
            }
            S::LerpConstFirst(a, first, s) => {
                let alpha = a.sample_value(ctx, x, y, z);
                if alpha == 0.0 {
                    *first
                } else if alpha == 1.0 {
                    s.sample_value(ctx, x, y, z)
                } else {
                    lerp_f32(alpha, *first, s.sample_value(ctx, x, y, z))
                }
            }
            S::LerpConstSecond(a, f, second) => {
                let alpha = a.sample_value(ctx, x, y, z);
                if alpha == 0.0 {
                    f.sample_value(ctx, x, y, z)
                } else if alpha == 1.0 {
                    *second
                } else {
                    lerp_f32(alpha, f.sample_value(ctx, x, y, z), *second)
                }
            }
            S::Clamp(i, min, max) => clamp_f32(i.sample_value(ctx, x, y, z), *min, *max),
            S::RangeChoice {
                input,
                min,
                max,
                in_range,
                out_of_range,
            } => {
                let v = input.sample_value(ctx, x, y, z);
                if v >= *min && v < *max {
                    in_range.sample_value(ctx, x, y, z)
                } else {
                    out_of_range.sample_value(ctx, x, y, z)
                }
            }
            S::RangeChoiceConst {
                input,
                min,
                max,
                in_range,
                out_of_range,
            } => {
                let v = input.sample_value(ctx, x, y, z);
                if v >= *min && v < *max {
                    *in_range
                } else {
                    *out_of_range
                }
            }
            S::IntervalSelect {
                input,
                thresholds,
                samplers,
            } => {
                let v = input.sample_value(ctx, x, y, z);
                samplers[select_index(thresholds, samplers.len(), v)].sample_value(ctx, x, y, z)
            }
            S::IntervalSelectSingle {
                input,
                threshold,
                if_below,
                if_above,
            } => {
                let v = input.sample_value(ctx, x, y, z);
                if v < *threshold {
                    if_below.sample_value(ctx, x, y, z)
                } else {
                    if_above.sample_value(ctx, x, y, z)
                }
            }
            S::Caching { id, input } => ctx.sample_value_cached(*id, input, x, y, z),
            // Blending of old-world chunks is not supported yet: `Blender` is always empty.
            S::BlendDensity(i) => i.sample_value(ctx, x, y, z),
            S::Interpolated {
                input,
                cell_xz,
                cell_y,
                ..
            } => {
                let xi = floor_mod(x, *cell_xz);
                let yi = floor_mod(y, *cell_y);
                let zi = floor_mod(z, *cell_xz);
                if xi == 0 && yi == 0 && zi == 0 {
                    return input.sample_value(ctx, x, y, z);
                }
                let volume = DensityVolume::new(2, 2, 2, x - xi, y - yi, z - zi, *cell_xz, *cell_y, *cell_xz);
                let mut buf = ctx.acquire(8);
                input.sample_volume(ctx, &mut buf, &volume);
                let v = lerp3_f32(
                    xi as f32 / *cell_xz as f32,
                    yi as f32 / *cell_y as f32,
                    zi as f32 / *cell_xz as f32,
                    buf[volume.index(0, 0, 0)],
                    buf[volume.index(1, 0, 0)],
                    buf[volume.index(0, 1, 0)],
                    buf[volume.index(1, 1, 0)],
                    buf[volume.index(0, 0, 1)],
                    buf[volume.index(1, 0, 1)],
                    buf[volume.index(0, 1, 1)],
                    buf[volume.index(1, 1, 1)],
                );
                ctx.release(buf);
                v
            }
            S::SliceX(i, sx) => i.sample_value(ctx, *sx, y, z),
            S::SliceY(i, sy) => i.sample_value(ctx, x, *sy, z),
            S::SliceZ(i, sz) => i.sample_value(ctx, x, y, *sz),
            S::SliceXz(i, sx, sz) => i.sample_value(ctx, *sx, y, *sz),
            S::FindTopSurface { upper_bound, .. } => {
                let ub = upper_bound.sample_value(ctx, x, y, z);
                self.find_surface_from(ctx, x, z, ub) as f32
            }
        }
    }

    #[inline]
    fn gradient_compute(&self, coordinate: i32) -> f32 {
        let Sampler::Gradient {
            from_coordinate,
            from_value,
            factor,
            mode,
            ..
        } = self
        else {
            unreachable!()
        };
        match mode {
            GradientMode::Clamped { min, max } => {
                let rel = coordinate.clamp(*min, *max) - from_coordinate;
                from_value + rel as f32 * factor
            }
            GradientMode::Repeat { range } => {
                let rel = coordinate - from_coordinate;
                from_value + floor_mod(rel, *range) as f32 * factor
            }
            GradientMode::MirroredRepeat { range } => {
                let rel = coordinate - from_coordinate;
                let tile = floor_div(rel, *range);
                let local = rel - tile * range;
                if tile & 1 == 0 {
                    from_value + local as f32 * factor
                } else {
                    from_value + (range - local) as f32 * factor
                }
            }
        }
    }

    fn find_surface_from(&self, ctx: &mut SamplerContext, x: i32, z: i32, upper_bound: f32) -> i32 {
        let Sampler::FindTopSurface {
            density,
            lower_bound,
            cell_height,
            ..
        } = self
        else {
            unreachable!()
        };
        let top_y = floor_f32(upper_bound / *cell_height as f32) * cell_height;
        if top_y <= *lower_bound {
            return *lower_bound;
        }
        let mut probe = top_y;
        while probe >= *lower_bound {
            if density.sample_value(ctx, x, probe, z) > 0.0 {
                return probe;
            }
            probe -= cell_height;
        }
        *lower_bound
    }

    /// `sampleVolume(context, outputBuffer, volume)`; `out.len() == volume.size()`.
    pub fn sample_volume(&self, ctx: &mut SamplerContext, out: &mut [f32], vol: &DensityVolume) {
        use Sampler as S;
        debug_assert_eq!(out.len(), vol.size());
        match self {
            S::Constant(v) => out.fill(*v),
            S::Beardifier(b) => b.sample_volume(out, vol),
            S::ContextBound { key, fallback } => match ctx_field(ctx, *key).take() {
                Some(s) => {
                    s.sample_volume(ctx, out, vol);
                    *ctx_field(ctx, *key) = Some(s);
                }
                None => out.fill(*fallback),
            },
            S::Noise {
                noise,
                xz_scale,
                y_scale,
            } => {
                out.fill(0.0);
                noise.add_to_volume(out, vol, *xz_scale, *y_scale, 1.0);
            }
            S::NoiseShiftedXz {
                shift_x,
                shift_z,
                noise,
                xz_scale,
                y_scale,
            } => {
                shift_x.sample_volume(ctx, out, vol);
                let mut sz = ctx.acquire(vol.size());
                shift_z.sample_volume(ctx, &mut sz, vol);
                let mut index = 0;
                for z in 0..vol.size_z {
                    let base_z = vol.block_z(z) as f64 * xz_scale;
                    for x in 0..vol.size_x {
                        let base_x = vol.block_x(x) as f64 * xz_scale;
                        for y in 0..vol.size_y {
                            let nx = base_x + out[index] as f64;
                            let ny = vol.block_y(y) as f64 * y_scale;
                            let nz = base_z + sz[index] as f64;
                            out[index] = noise.get(nx, ny, nz);
                            index += 1;
                        }
                    }
                }
                ctx.release(sz);
            }
            S::NoiseShiftedXyz {
                shift_x,
                shift_y,
                shift_z,
                noise,
                xz_scale,
                y_scale,
            } => {
                shift_x.sample_volume(ctx, out, vol);
                let mut sy = ctx.acquire(vol.size());
                shift_y.sample_volume(ctx, &mut sy, vol);
                let mut sz = ctx.acquire(vol.size());
                shift_z.sample_volume(ctx, &mut sz, vol);
                let mut index = 0;
                for z in 0..vol.size_z {
                    let base_z = vol.block_z(z) as f64 * xz_scale;
                    for x in 0..vol.size_x {
                        let base_x = vol.block_x(x) as f64 * xz_scale;
                        for y in 0..vol.size_y {
                            let nx = base_x + out[index] as f64;
                            let ny = vol.block_y(y) as f64 * y_scale + sy[index] as f64;
                            let nz = base_z + sz[index] as f64;
                            out[index] = noise.get(nx, ny, nz);
                            index += 1;
                        }
                    }
                }
                ctx.release(sz);
                ctx.release(sy);
            }
            S::ShiftB { noise } => {
                let t = DensityVolume::new(
                    vol.size_z, vol.size_x, 1, vol.min_z, vol.min_x, 0, vol.step_z, vol.step_x, 1,
                );
                let mut tb = ctx.acquire(t.size());
                tb.fill(0.0);
                noise.add_to_volume(&mut tb, &t, 0.25, 0.25, 4.0);
                for z in 0..vol.size_z {
                    for x in 0..vol.size_x {
                        let v = tb[t.index(z, x, 0)];
                        let start = vol.index(x, 0, z);
                        out[start..start + vol.size_y as usize].fill(v);
                    }
                }
                ctx.release(tb);
            }
            S::EndIslands { noise } => {
                for z in 0..vol.size_z {
                    let bz = vol.block_z(z);
                    for x in 0..vol.size_x {
                        let bx = vol.block_x(x);
                        let v = (end_island_height(noise, bx / 8, bz / 8) - 8.0) / 128.0;
                        let start = vol.index(x, 0, z);
                        out[start..start + vol.size_y as usize].fill(v);
                    }
                }
            }
            S::DistanceToPoint { .. } => self.sample_volume_naive(ctx, out, vol),
            S::Gradient { axis, .. } => match axis {
                Axis::X => {
                    for x in 0..vol.size_x {
                        let v = self.gradient_compute(vol.block_x(x));
                        for z in 0..vol.size_z {
                            let start = vol.index(x, 0, z);
                            out[start..start + vol.size_y as usize].fill(v);
                        }
                    }
                }
                Axis::Y => {
                    for y in 0..vol.size_y {
                        let v = self.gradient_compute(vol.block_y(y));
                        for z in 0..vol.size_z {
                            for x in 0..vol.size_x {
                                out[vol.index(x, y, z)] = v;
                            }
                        }
                    }
                }
                Axis::Z => {
                    for z in 0..vol.size_z {
                        let v = self.gradient_compute(vol.block_z(z));
                        let start = vol.index(0, 0, z);
                        out[start..start + (vol.size_x * vol.size_y) as usize].fill(v);
                    }
                }
            },
            S::Abs(i) => map_in_place(ctx, i, out, vol, f32::abs),
            S::Square(i) => map_in_place(ctx, i, out, vol, |v| v * v),
            S::Cube(i) => map_in_place(ctx, i, out, vol, cube_f32),
            S::Sqrt(i) => map_in_place(ctx, i, out, vol, sqrt_f32),
            S::LeakyRelu(i, f) => {
                let f = *f;
                map_in_place(ctx, i, out, vol, |v| leaky_relu(f, v))
            }
            S::Reciprocal(i) => map_in_place(ctx, i, out, vol, |v| 1.0 / v),
            S::Negate(i) => map_in_place(ctx, i, out, vol, |v| -v),
            S::Squeeze(i) => map_in_place(ctx, i, out, vol, squeeze),
            S::Log(i) => map_in_place(ctx, i, out, vol, |v| (v as f64).ln() as f32),
            S::Sign(i) => map_in_place(ctx, i, out, vol, signum_f32),
            S::RoundInteger(k, i) => {
                let k = *k;
                map_in_place(ctx, i, out, vol, |v| k.apply(v))
            }
            S::Round(k, i, m) => {
                i.sample_volume(ctx, out, vol);
                let mut mb = ctx.acquire(vol.size());
                m.sample_volume(ctx, &mut mb, vol);
                for (o, &mul) in out.iter_mut().zip(mb.iter()) {
                    *o = round_apply(*k, *o, mul);
                }
                ctx.release(mb);
            }
            S::Add(l, r) => zip_with(ctx, l, r, out, vol, |a, b| a + b),
            S::ConstAdd(l, c) => {
                let c = *c;
                map_in_place(ctx, l, out, vol, |v| v + c)
            }
            // `addTo(i, -right)`.
            S::Sub(l, r) => zip_with(ctx, l, r, out, vol, |a, b| a + -b),
            S::ConstSub(c, r) => {
                let c = *c;
                map_in_place(ctx, r, out, vol, |v| c - v)
            }
            S::Mul(l, r) => zip_with(ctx, l, r, out, vol, |a, b| a * b),
            S::ConstMul(l, c) => {
                let c = *c;
                map_in_place(ctx, l, out, vol, |v| v * c)
            }
            S::Div(l, r) => zip_with(ctx, l, r, out, vol, |a, b| a / b),
            S::ConstDiv(c, r) => {
                let c = *c;
                map_in_place(ctx, r, out, vol, |v| c / v)
            }
            S::Min { left, right, .. } => {
                zip_with(ctx, left, right, out, vol, |a, b| if b < a { b } else { a })
            }
            S::ConstMin(l, c) => {
                let c = *c;
                map_in_place(ctx, l, out, vol, |v| if c < v { c } else { v })
            }
            S::Max { left, right, .. } => {
                zip_with(ctx, left, right, out, vol, |a, b| if b > a { b } else { a })
            }
            S::ConstMax(l, c) => {
                let c = *c;
                map_in_place(ctx, l, out, vol, |v| if c > v { c } else { v })
            }
            S::Pow(b, e) => zip_with(ctx, b, e, out, vol, |a, b| jpow(a as f64, b as f64)),
            S::PowConstBase(b, e) => {
                let b = *b;
                map_in_place(ctx, e, out, vol, |v| jpow(b, v as f64))
            }
            S::PowConstExponent(b, e) => {
                let e = *e;
                map_in_place(ctx, b, out, vol, |v| jpow(v as f64, e))
            }
            S::Spline { spline, coordinates } => {
                // `BufferSplineInput`: each coordinate is sampled over the whole volume on first use.
                let mut buffers: Vec<Option<Vec<f32>>> = (0..coordinates.len()).map(|_| None).collect();
                for i in 0..vol.size() {
                    let v = spline.sample(&mut |c| {
                        if buffers[c].is_none() {
                            let mut b = ctx.acquire(vol.size());
                            coordinates[c].sample_volume(ctx, &mut b, vol);
                            buffers[c] = Some(b);
                        }
                        buffers[c].as_ref().unwrap()[i]
                    });
                    out[i] = v;
                }
                for b in buffers.into_iter().flatten() {
                    ctx.release(b);
                }
            }
            S::Lerp(a, f, s) => {
                a.sample_volume(ctx, out, vol);
                let mut fb = ctx.acquire(vol.size());
                f.sample_volume(ctx, &mut fb, vol);
                let mut sb = ctx.acquire(vol.size());
                s.sample_volume(ctx, &mut sb, vol);
                for i in 0..out.len() {
                    let alpha = out[i];
                    out[i] = if alpha == 0.0 {
                        fb[i]
                    } else if alpha == 1.0 {
                        sb[i]
                    } else {
                        lerp_f32(alpha, fb[i], sb[i])
                    };
                }
                ctx.release(sb);
                ctx.release(fb);
            }
            S::LerpConstFirst(a, first, s) => {
                a.sample_volume(ctx, out, vol);
                let mut sb = ctx.acquire(vol.size());
                s.sample_volume(ctx, &mut sb, vol);
                for i in 0..out.len() {
                    let alpha = out[i];
                    out[i] = if alpha == 0.0 {
                        *first
                    } else if alpha == 1.0 {
                        sb[i]
                    } else {
                        lerp_f32(alpha, *first, sb[i])
                    };
                }
                ctx.release(sb);
            }
            S::LerpConstSecond(a, f, second) => {
                a.sample_volume(ctx, out, vol);
                let mut fb = ctx.acquire(vol.size());
                f.sample_volume(ctx, &mut fb, vol);
                for i in 0..out.len() {
                    let alpha = out[i];
                    out[i] = if alpha == 0.0 {
                        fb[i]
                    } else if alpha == 1.0 {
                        *second
                    } else {
                        lerp_f32(alpha, fb[i], *second)
                    };
                }
                ctx.release(fb);
            }
            S::Clamp(i, min, max) => {
                let (min, max) = (*min, *max);
                map_in_place(ctx, i, out, vol, |v| clamp_f32(v, min, max))
            }
            S::RangeChoice {
                input,
                min,
                max,
                in_range,
                out_of_range,
            } => {
                in_range.sample_volume(ctx, out, vol);
                let mut ib = ctx.acquire(vol.size());
                input.sample_volume(ctx, &mut ib, vol);
                let mut ob = ctx.acquire(vol.size());
                out_of_range.sample_volume(ctx, &mut ob, vol);
                for i in 0..out.len() {
                    let v = ib[i];
                    if !(v >= *min) || !(v < *max) {
                        out[i] = ob[i];
                    }
                }
                ctx.release(ob);
                ctx.release(ib);
            }
            S::RangeChoiceConst {
                input,
                min,
                max,
                in_range,
                out_of_range,
            } => {
                let (min, max, a, b) = (*min, *max, *in_range, *out_of_range);
                map_in_place(ctx, input, out, vol, |v| if v >= min && v < max { a } else { b })
            }
            S::IntervalSelect {
                input,
                thresholds,
                samplers,
            } => {
                input.sample_volume(ctx, out, vol);
                let mut bufs = Vec::with_capacity(samplers.len());
                for s in samplers {
                    let mut b = ctx.acquire(vol.size());
                    s.sample_volume(ctx, &mut b, vol);
                    bufs.push(b);
                }
                for (i, o) in out.iter_mut().enumerate() {
                    let si = select_index(thresholds, samplers.len(), *o);
                    *o = bufs[si][i];
                }
                for b in bufs {
                    ctx.release(b);
                }
            }
            S::IntervalSelectSingle {
                input,
                threshold,
                if_below,
                if_above,
            } => {
                input.sample_volume(ctx, out, vol);
                let mut bb = ctx.acquire(vol.size());
                if_below.sample_volume(ctx, &mut bb, vol);
                let mut ab = ctx.acquire(vol.size());
                if_above.sample_volume(ctx, &mut ab, vol);
                for i in 0..out.len() {
                    out[i] = if out[i] < *threshold { bb[i] } else { ab[i] };
                }
                ctx.release(ab);
                ctx.release(bb);
            }
            S::Caching { id, input } => ctx.sample_volume_cached(*id, input, out, vol),
            S::BlendDensity(i) => i.sample_volume(ctx, out, vol),
            S::Interpolated {
                input,
                cell_xz,
                cell_y,
                ..
            } => {
                let (cxz, cy) = (*cell_xz, *cell_y);
                if (vol.step_x == cxz || vol.size_x == 1)
                    && (vol.step_y == cy || vol.size_y == 1)
                    && (vol.step_z == cxz || vol.size_z == 1)
                    && floor_mod(vol.min_x, cxz) == 0
                    && floor_mod(vol.min_y, cy) == 0
                    && floor_mod(vol.min_z, cxz) == 0
                {
                    input.sample_volume(ctx, out, vol);
                } else if vol.step_x == 1 && vol.step_y == 1 && vol.step_z == 1 {
                    self.interpolate_block_step(ctx, out, vol);
                } else {
                    let bv = DensityVolume::blocks(
                        vol.size_x * vol.step_x,
                        vol.size_y * vol.step_y,
                        vol.size_z * vol.step_z,
                        vol.min_x,
                        vol.min_y,
                        vol.min_z,
                    );
                    let mut bb = ctx.acquire(bv.size());
                    self.interpolate_block_step(ctx, &mut bb, &bv);
                    for z in 0..vol.size_z {
                        for x in 0..vol.size_x {
                            for y in 0..vol.size_y {
                                out[vol.index(x, y, z)] =
                                    bb[bv.index(x * vol.step_x, y * vol.step_y, z * vol.step_z)];
                            }
                        }
                    }
                    ctx.release(bb);
                }
            }
            S::SliceX(i, sx) => {
                if vol.size_x == 1 && vol.min_x == *sx {
                    return i.sample_volume(ctx, out, vol);
                }
                let iv = DensityVolume::new(
                    1, vol.size_y, vol.size_z, *sx, vol.min_y, vol.min_z, vol.step_x, vol.step_y, vol.step_z,
                );
                let mut ib = ctx.acquire(iv.size());
                i.sample_volume(ctx, &mut ib, &iv);
                let mut index = 0;
                for z in 0..vol.size_z {
                    for _x in 0..vol.size_x {
                        for y in 0..vol.size_y {
                            out[index] = ib[iv.index(0, y, z)];
                            index += 1;
                        }
                    }
                }
                ctx.release(ib);
            }
            S::SliceY(i, sy) => {
                if vol.size_y == 1 && vol.min_y == *sy {
                    return i.sample_volume(ctx, out, vol);
                }
                let iv = DensityVolume::new(
                    vol.size_x, 1, vol.size_z, vol.min_x, *sy, vol.min_z, vol.step_x, vol.step_y, vol.step_z,
                );
                let mut ib = ctx.acquire(iv.size());
                i.sample_volume(ctx, &mut ib, &iv);
                for z in 0..vol.size_z {
                    for x in 0..vol.size_x {
                        let v = ib[iv.index(x, 0, z)];
                        let start = vol.index(x, 0, z);
                        out[start..start + vol.size_y as usize].fill(v);
                    }
                }
                ctx.release(ib);
            }
            S::SliceZ(i, sz) => {
                if vol.size_z == 1 && vol.min_z == *sz {
                    return i.sample_volume(ctx, out, vol);
                }
                let iv = DensityVolume::new(
                    vol.size_x, vol.size_y, 1, vol.min_x, vol.min_y, *sz, vol.step_x, vol.step_y, vol.step_z,
                );
                let mut ib = ctx.acquire(iv.size());
                i.sample_volume(ctx, &mut ib, &iv);
                let mut index = 0;
                for _z in 0..vol.size_z {
                    for x in 0..vol.size_x {
                        for y in 0..vol.size_y {
                            out[index] = ib[iv.index(x, y, 0)];
                            index += 1;
                        }
                    }
                }
                ctx.release(ib);
            }
            S::SliceXz(i, sx, sz) => {
                if vol.size_x == 1 && vol.size_z == 1 && vol.min_x == *sx && vol.min_z == *sz {
                    return i.sample_volume(ctx, out, vol);
                }
                let iv = DensityVolume::new(
                    1, vol.size_y, 1, *sx, vol.min_y, *sz, vol.step_x, vol.step_y, vol.step_z,
                );
                let mut ib = ctx.acquire(iv.size());
                i.sample_volume(ctx, &mut ib, &iv);
                for y in 0..vol.size_y {
                    let v = ib[iv.index(0, y, 0)];
                    let mut index = vol.index(0, y, 0);
                    for _ in 0..(vol.size_x * vol.size_z) {
                        out[index] = v;
                        index += vol.size_y as usize;
                    }
                }
                ctx.release(ib);
            }
            S::FindTopSurface { upper_bound, .. } => {
                assert_eq!(vol.size_y, 1, "Cannot sample with sizeY={}", vol.size_y);
                upper_bound.sample_volume(ctx, out, vol);
                let mut index = 0;
                for z in 0..vol.size_z {
                    let bz = vol.block_z(z);
                    for x in 0..vol.size_x {
                        let bx = vol.block_x(x);
                        let ub = out[index];
                        out[index] = self.find_surface_from(ctx, bx, bz, ub) as f32;
                        index += 1;
                    }
                }
            }
        }
    }

    /// `DensitySampler.sampleVolumeNaive`.
    fn sample_volume_naive(&self, ctx: &mut SamplerContext, out: &mut [f32], vol: &DensityVolume) {
        let mut index = 0;
        for z in 0..vol.size_z {
            let bz = vol.block_z(z);
            for x in 0..vol.size_x {
                let bx = vol.block_x(x);
                for y in 0..vol.size_y {
                    out[index] = self.sample_value(ctx, bx, vol.block_y(y), bz);
                    index += 1;
                }
            }
        }
    }

    /// `InterpolatedFunction.Sampler.sampleWithBlockStep` + `fillCell`.
    fn interpolate_block_step(&self, ctx: &mut SamplerContext, out: &mut [f32], vol: &DensityVolume) {
        let Sampler::Interpolated {
            input,
            cell_xz,
            cell_y,
            inv_xz,
            inv_y,
        } = self
        else {
            unreachable!()
        };
        let (cxz, cy, inv_xz, inv_y) = (*cell_xz, *cell_y, *inv_xz, *inv_y);
        let min_cx = floor_div(vol.min_x, cxz);
        let min_cy = floor_div(vol.min_y, cy);
        let min_cz = floor_div(vol.min_z, cxz);
        let max_cx = floor_div(vol.max_x(), cxz);
        let max_cy = floor_div(vol.max_y(), cy);
        let max_cz = floor_div(vol.max_z(), cxz);
        let count_x = max_cx - min_cx + 1;
        let count_y = max_cy - min_cy + 1;
        let count_z = max_cz - min_cz + 1;
        let cv = DensityVolume::new(
            if floor_mod(vol.max_x(), cxz) == 0 {
                count_x
            } else {
                count_x + 1
            },
            if floor_mod(vol.max_y(), cy) == 0 {
                count_y
            } else {
                count_y + 1
            },
            if floor_mod(vol.max_z(), cxz) == 0 {
                count_z
            } else {
                count_z + 1
            },
            min_cx * cxz,
            min_cy * cy,
            min_cz * cxz,
            cxz,
            cy,
            cxz,
        );
        let mut cb = ctx.acquire(cv.size());
        input.sample_volume(ctx, &mut cb, &cv);
        for cz in 0..count_z {
            let ncz = (cz + 1).min(cv.size_z - 1);
            for cx in 0..count_x {
                let ncx = (cx + 1).min(cv.size_x - 1);
                let mut v000 = cb[cv.index(cx, 0, cz)];
                let mut v100 = cb[cv.index(ncx, 0, cz)];
                let mut v001 = cb[cv.index(cx, 0, ncz)];
                let mut v101 = cb[cv.index(ncx, 0, ncz)];
                for cyi in 0..count_y {
                    let ncy = (cyi + 1).min(cv.size_y - 1);
                    let v010 = cb[cv.index(cx, ncy, cz)];
                    let v110 = cb[cv.index(ncx, ncy, cz)];
                    let v011 = cb[cv.index(cx, ncy, ncz)];
                    let v111 = cb[cv.index(ncx, ncy, ncz)];
                    // fillCell
                    let ox = cv.block_x(cx) - vol.min_x;
                    let oy = cv.block_y(cyi) - vol.min_y;
                    let oz = cv.block_z(cz) - vol.min_z;
                    let x0 = 0.max(-ox);
                    let y0 = 0.max(-oy);
                    let z0 = 0.max(-oz);
                    let x1 = cxz.min(vol.size_x - ox) - 1;
                    let y1 = cy.min(vol.size_y - oy) - 1;
                    let z1 = cxz.min(vol.size_z - oz) - 1;
                    for z in z0..=z1 {
                        let out_z = oz + z;
                        let az = z as f32 * inv_xz;
                        let v00 = lerp_f32(az, v000, v001);
                        let v01 = lerp_f32(az, v010, v011);
                        let v10 = lerp_f32(az, v100, v101);
                        let v11 = lerp_f32(az, v110, v111);
                        for x in x0..=x1 {
                            let out_x = ox + x;
                            let ax = x as f32 * inv_xz;
                            let v0 = lerp_f32(ax, v00, v10);
                            let v1 = lerp_f32(ax, v01, v11);
                            let step = (v1 - v0) * inv_y;
                            let mut value = v0 + step * y0 as f32;
                            let start = vol.index(out_x, oy + y0, out_z);
                            let n = (y1 - y0 + 1).max(0) as usize;
                            for o in out.iter_mut().skip(start).take(n) {
                                *o = value;
                                value += step;
                            }
                        }
                    }
                    v000 = v010;
                    v100 = v110;
                    v001 = v011;
                    v101 = v111;
                }
            }
        }
        ctx.release(cb);
    }
}

/// The context slot of `key`. Samplers take the value out while evaluating it (and put it back) instead
/// of cloning the `Arc`: a refcount bump per sample would make all worker threads share a cache line.
/// Bound samplers never refer to their own key.
#[inline(always)]
fn ctx_field(ctx: &mut SamplerContext, key: ContextKey) -> &mut Option<Arc<Sampler>> {
    match key {
        ContextKey::BlendAlpha => &mut ctx.fields.blend_alpha,
        ContextKey::BlendOffset => &mut ctx.fields.blend_offset,
        ContextKey::Beardifier => &mut ctx.fields.beardifier,
    }
}

#[inline(always)]
fn round_apply(kind: RoundKind, input: f32, multiple: f32) -> f32 {
    if multiple == 0.0 {
        input
    } else {
        kind.apply(input / multiple) * multiple
    }
}

#[inline(always)]
fn select_index(thresholds: &[f32], count: usize, input: f32) -> usize {
    for (i, t) in thresholds.iter().enumerate() {
        if input < *t {
            return i;
        }
    }
    count - 1
}

#[inline(always)]
fn map_in_place(
    ctx: &mut SamplerContext,
    input: &Sampler,
    out: &mut [f32],
    vol: &DensityVolume,
    f: impl Fn(f32) -> f32,
) {
    input.sample_volume(ctx, out, vol);
    for v in out.iter_mut() {
        *v = f(*v);
    }
}

#[inline(always)]
fn zip_with(
    ctx: &mut SamplerContext,
    left: &Sampler,
    right: &Sampler,
    out: &mut [f32],
    vol: &DensityVolume,
    f: impl Fn(f32, f32) -> f32,
) {
    left.sample_volume(ctx, out, vol);
    let mut rb = ctx.acquire(vol.size());
    right.sample_volume(ctx, &mut rb, vol);
    for (o, &r) in out.iter_mut().zip(rb.iter()) {
        *o = f(*o, r);
    }
    ctx.release(rb);
}
