//! Density functions (26.3 `levelgen.densityfunction` rewrite).
//!
//! Pipeline, mirroring vanilla:
//! 1. [`parse`] turns JSON into a [`DensityFunction`] tree ([`Df`]); registry references become
//!    [`DensityFunction::Reference`] nodes (vanilla `HolderHolder`).
//! 2. [`compiler::DensityCompiler`] applies the vanilla rewrite rules (inline references,
//!    deduplicate `cache` nodes into numbered cache cells, slice away unused axes) and compiles the
//!    tree into a [`sampler::Sampler`].
//! 3. Samplers are evaluated per point (`sample_value`) or per grid (`sample_volume`) with a
//!    [`context::SamplerContext`] that owns cache cells and scratch buffers.
//!
//! The point and volume paths of vanilla are *not* numerically identical (e.g. interpolation
//! accumulates in volume mode), so both are ported exactly and callers must use the same path as
//! vanilla does.

pub mod compiler;
pub mod context;
pub mod parse;
pub mod sampler;
pub mod spline;
mod volume;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chunker_data::Identifier;

pub use context::SamplerContext;
pub use spline::Spline;
pub use volume::DensityVolume;

use crate::math::{Interval, cube_f32};
use crate::noise::NormalNoise;

pub type Df = Arc<DensityFunction>;

/// Axis bitmask (`DensityFunction.AXIS_X/Y/Z`).
pub mod axes {
    pub const X: u8 = 1;
    pub const Y: u8 = 2;
    pub const Z: u8 = 4;
    pub const NONE: u8 = 0;
    pub const ALL: u8 = 7;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn mask(self) -> u8 {
        match self {
            Axis::X => axes::X,
            Axis::Y => axes::Y,
            Axis::Z => axes::Z,
        }
    }

    #[inline(always)]
    pub fn choose(self, x: i32, y: i32, z: i32) -> i32 {
        match self {
            Axis::X => x,
            Axis::Y => y,
            Axis::Z => z,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "x" => Some(Axis::X),
            "y" => Some(Axis::Y),
            "z" => Some(Axis::Z),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TilingMode {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DistanceMetric {
    Euclidean,
    EuclideanSquared,
    Manhattan,
    Chebyshev,
}

impl DistanceMetric {
    #[inline]
    pub fn compute(self, dx: f32, dy: f32, dz: f32) -> f32 {
        use crate::math::{length3_f32, max_f32};
        match self {
            DistanceMetric::Euclidean => length3_f32(dx, dy, dz),
            DistanceMetric::EuclideanSquared => dx * dx + dy * dy + dz * dz,
            DistanceMetric::Manhattan => dx.abs() + dy.abs() + dz.abs(),
            DistanceMetric::Chebyshev => max_f32(max_f32(dx.abs(), dy.abs()), dz.abs()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimpleKind {
    BlendAlpha,
    BlendOffset,
    Beardifier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryKind {
    Abs,
    Square,
    Cube,
    Sqrt,
    HalfNegative,
    QuarterNegative,
    Reciprocal,
    Negate,
    Squeeze,
    Log,
    Sign,
}

impl UnaryKind {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "abs" => UnaryKind::Abs,
            "square" => UnaryKind::Square,
            "cube" => UnaryKind::Cube,
            "sqrt" => UnaryKind::Sqrt,
            "half_negative" => UnaryKind::HalfNegative,
            "quarter_negative" => UnaryKind::QuarterNegative,
            "reciprocal" => UnaryKind::Reciprocal,
            "negate" => UnaryKind::Negate,
            "squeeze" => UnaryKind::Squeeze,
            "log" => UnaryKind::Log,
            "sign" => UnaryKind::Sign,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryKind {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

impl BinaryKind {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "add" => BinaryKind::Add,
            "sub" => BinaryKind::Sub,
            "mul" => BinaryKind::Mul,
            "div" => BinaryKind::Div,
            "min" => BinaryKind::Min,
            "max" => BinaryKind::Max,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundKind {
    Floor,
    Round,
    Ceil,
    Truncate,
}

impl RoundKind {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "floor" => RoundKind::Floor,
            "round" => RoundKind::Round,
            "ceil" => RoundKind::Ceil,
            "truncate" => RoundKind::Truncate,
            _ => return None,
        })
    }

    /// `RoundFunction.roundToInteger`.
    #[inline]
    pub fn apply(self, input: f32) -> f32 {
        match self {
            RoundKind::Floor => (input as f64).floor() as f32,
            RoundKind::Round => crate::math::round_f32(input) as f32,
            RoundKind::Ceil => (input as f64).ceil() as f32,
            RoundKind::Truncate => {
                if input > 0.0 {
                    (input as f64).floor() as f32
                } else {
                    (input as f64).ceil() as f32
                }
            }
        }
    }
}

/// A noise holder: registry reference to `worldgen/noise/<id>` with its parsed parameters.
#[derive(Clone, Debug)]
pub struct NoiseHolder {
    pub id: Identifier,
    pub noise: Arc<NormalNoise>,
}

impl PartialEq for NoiseHolder {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for NoiseHolder {}

impl Hash for NoiseHolder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

/// `old_blended_noise` parameters.
#[derive(Clone, Copy, Debug)]
pub struct BlendedNoiseParams {
    pub xz_scale: f64,
    pub y_scale: f64,
    pub xz_factor: f64,
    pub y_factor: f64,
    pub smear_scale_multiplier: f64,
}

/// A density function node. Equality and hashing are structural (Java record semantics:
/// floats compare by bits), except references, which compare by registry id like `Holder`.
#[derive(Debug)]
pub enum DensityFunction {
    Constant(f32),
    Simple(SimpleKind),
    Noise {
        noise: NoiseHolder,
        xz_scale: f64,
        y_scale: f64,
        shift_x: Df,
        shift_y: Df,
        shift_z: Df,
    },
    EndIslands,
    DistanceToPoint {
        point: [i32; 3],
        metric: DistanceMetric,
    },
    Gradient {
        axis: Axis,
        tiling: TilingMode,
        from_coordinate: i32,
        to_coordinate: i32,
        from_value: f32,
        to_value: f32,
    },
    ShiftA(NoiseHolder),
    ShiftB(NoiseHolder),
    Shift(NoiseHolder),
    Unary(UnaryKind, Df),
    Round {
        kind: RoundKind,
        input: Df,
        multiple: Df,
    },
    Binary(BinaryKind, Df, Df),
    Pow {
        base: Df,
        exponent: Df,
    },
    Spline(Arc<Spline>),
    Lerp {
        alpha: Df,
        first: Df,
        second: Df,
    },
    Clamp {
        input: Df,
        min: f32,
        max: f32,
    },
    RangeChoice {
        input: Df,
        min_inclusive: f32,
        max_exclusive: f32,
        when_in_range: Df,
        when_out_of_range: Df,
    },
    IntervalSelect {
        input: Df,
        thresholds: Vec<f32>,
        functions: Vec<Df>,
    },
    Cache(Df),
    BlendDensity(Df),
    Interpolated {
        input: Df,
        cell_size_xz: i32,
        cell_size_y: i32,
    },
    Slice {
        axis: Axis,
        coordinate: i32,
        input: Df,
    },
    FindTopSurface {
        density: Df,
        upper_bound: Df,
        lower_bound: i32,
        cell_height: i32,
    },
    OldBlendedNoise(BlendedNoiseParams),
    /// `HolderHolder`: a named reference into the density function registry.
    Reference {
        id: Identifier,
        target: Df,
    },
    /// Compiler-internal `PreparedCache`.
    PreparedCache {
        id: usize,
        range: Interval,
        domain_axes: u8,
        sampler: Arc<sampler::Sampler>,
    },
}

impl DensityFunction {
    pub fn zero() -> Df {
        Arc::new(DensityFunction::Constant(0.0))
    }

    pub fn is_zero_constant(&self) -> bool {
        matches!(self, DensityFunction::Constant(v) if v.to_bits() == 0.0f32.to_bits())
    }

    pub fn constant_value(&self) -> Option<f32> {
        match self {
            DensityFunction::Constant(v) => Some(*v),
            _ => None,
        }
    }

    /// `domainAxes()`: which axes the value depends on.
    pub fn domain_axes(&self) -> u8 {
        use DensityFunction as D;
        match self {
            D::Constant(_) => axes::NONE,
            D::Simple(SimpleKind::BlendAlpha | SimpleKind::BlendOffset) => axes::X | axes::Z,
            D::Simple(SimpleKind::Beardifier) => axes::ALL,
            D::Noise {
                xz_scale,
                y_scale,
                shift_x,
                shift_y,
                shift_z,
                ..
            } => {
                let mut a = axes::ALL;
                if *y_scale == 0.0 {
                    a &= !axes::Y;
                }
                if *xz_scale == 0.0 {
                    a &= !(axes::X | axes::Z);
                }
                a | shift_x.domain_axes() | shift_y.domain_axes() | shift_z.domain_axes()
            }
            D::EndIslands => axes::X | axes::Z,
            D::DistanceToPoint { .. } => axes::ALL,
            D::Gradient { axis, .. } => axis.mask(),
            D::Shift(_) => axes::ALL,
            D::ShiftA(_) | D::ShiftB(_) => axes::X | axes::Z,
            D::Unary(_, i) | D::Cache(i) | D::BlendDensity(i) => i.domain_axes(),
            D::Round { input, multiple, .. } => input.domain_axes() | multiple.domain_axes(),
            D::Binary(_, l, r) => l.domain_axes() | r.domain_axes(),
            D::Pow { base, exponent } => base.domain_axes() | exponent.domain_axes(),
            D::Spline(s) => {
                let mut a = 0;
                s.for_each_coordinate(&mut |c| a |= c.domain_axes());
                a
            }
            D::Lerp { alpha, first, second } => {
                alpha.domain_axes() | first.domain_axes() | second.domain_axes()
            }
            D::Clamp { input, .. } => input.domain_axes(),
            D::RangeChoice {
                input,
                when_in_range,
                when_out_of_range,
                ..
            } => input.domain_axes() | when_in_range.domain_axes() | when_out_of_range.domain_axes(),
            D::IntervalSelect { input, functions, .. } => functions
                .iter()
                .fold(input.domain_axes(), |a, f| a | f.domain_axes()),
            D::Interpolated { input, .. } => input.domain_axes(),
            D::Slice { axis, input, .. } => input.domain_axes() & !axis.mask(),
            D::FindTopSurface {
                density, upper_bound, ..
            } => (density.domain_axes() | upper_bound.domain_axes()) & !axes::Y,
            D::OldBlendedNoise(_) => axes::ALL,
            D::Reference { target, .. } => target.domain_axes(),
            D::PreparedCache { domain_axes, .. } => *domain_axes,
        }
    }

    /// `range()`: value interval used by compile-time optimisations.
    pub fn range(&self) -> Interval {
        use DensityFunction as D;
        match self {
            D::Constant(v) => Interval::of_exact(*v),
            D::Simple(SimpleKind::BlendAlpha) => Interval::of(0.0, 1.0),
            D::Simple(SimpleKind::BlendOffset) => Interval::INFINITE,
            D::Simple(SimpleKind::Beardifier) => crate::terrain::beardifier::RANGE,
            D::Noise { noise, .. } => noise.noise.range(),
            D::EndIslands => Interval::of(-0.84375, 0.5625),
            D::DistanceToPoint { .. } => Interval::of(0.0, f32::INFINITY),
            D::Gradient {
                from_value, to_value, ..
            } => Interval::encapsulating(*from_value, *to_value),
            D::ShiftA(n) | D::ShiftB(n) | D::Shift(n) => {
                Interval::mul(n.noise.range(), Interval::of_exact(4.0))
            }
            D::Unary(kind, input) => {
                let i = input.range();
                match kind {
                    UnaryKind::Abs => Interval::abs(i),
                    UnaryKind::Square => Interval::square(i),
                    UnaryKind::Cube => Interval::map_monotonic(i, cube_f32),
                    UnaryKind::Sqrt => Interval::pow(i, Interval::of_exact(0.5)),
                    UnaryKind::HalfNegative => Interval::map_monotonic(i, |v| sampler::leaky_relu(0.5, v)),
                    UnaryKind::QuarterNegative => {
                        Interval::map_monotonic(i, |v| sampler::leaky_relu(0.25, v))
                    }
                    UnaryKind::Reciprocal => Interval::reciprocal(i),
                    UnaryKind::Negate => Interval::sub(Interval::of_exact(0.0), i),
                    UnaryKind::Squeeze => Interval::map_monotonic(i, sampler::squeeze),
                    UnaryKind::Log => Interval::log(i),
                    UnaryKind::Sign => Interval::sign(i),
                }
            }
            D::Round {
                kind,
                input,
                multiple,
            } => {
                let m = multiple.range();
                Interval::mul(
                    Interval::map_monotonic(Interval::div(input.range(), m), |v| kind.apply(v)),
                    m,
                )
            }
            D::Binary(kind, l, r) => {
                let (l, r) = (l.range(), r.range());
                match kind {
                    BinaryKind::Add => Interval::add(l, r),
                    BinaryKind::Sub => Interval::sub(l, r),
                    BinaryKind::Mul => Interval::mul(l, r),
                    BinaryKind::Div => Interval::div(l, r),
                    BinaryKind::Min => Interval::min_of(l, r),
                    BinaryKind::Max => Interval::max_of(l, r),
                }
            }
            D::Pow { base, exponent } => Interval::pow(base.range(), exponent.range()),
            D::Spline(s) => s.range(),
            D::Lerp { alpha, first, second } => Interval::lerp(alpha.range(), first.range(), second.range()),
            D::Clamp { input, min, max } => Interval::clamp(input.range(), *min, *max),
            D::RangeChoice {
                when_in_range,
                when_out_of_range,
                ..
            } => Interval::encapsulating_all(&[when_in_range.range(), when_out_of_range.range()]),
            D::IntervalSelect { functions, .. } => {
                let ranges: Vec<Interval> = functions.iter().map(|f| f.range()).collect();
                Interval::encapsulating_all(&ranges)
            }
            D::Cache(i) | D::BlendDensity(i) => i.range(),
            D::Interpolated { input, .. } => input.range(),
            D::Slice { input, .. } => input.range(),
            D::FindTopSurface {
                upper_bound,
                lower_bound,
                ..
            } => Interval::of(
                *lower_bound as f32,
                crate::math::max_f32(*lower_bound as f32, upper_bound.range().max()),
            ),
            D::OldBlendedNoise(p) => crate::noise::blended_noise_range(p),
            D::Reference { target, .. } => target.range(),
            D::PreparedCache { range, .. } => *range,
        }
    }
}

fn f32_key(v: f32) -> u32 {
    // Java `Float.compare` equality: all NaNs are equal, +0.0 != -0.0.
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}

fn f64_key(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits()
    }
}

impl PartialEq for DensityFunction {
    fn eq(&self, other: &Self) -> bool {
        use DensityFunction as D;
        if std::ptr::eq(self, other) {
            return true;
        }
        match (self, other) {
            (D::Constant(a), D::Constant(b)) => f32_key(*a) == f32_key(*b),
            (D::Simple(a), D::Simple(b)) => a == b,
            (
                D::Noise {
                    noise: n1,
                    xz_scale: a1,
                    y_scale: b1,
                    shift_x: x1,
                    shift_y: y1,
                    shift_z: z1,
                },
                D::Noise {
                    noise: n2,
                    xz_scale: a2,
                    y_scale: b2,
                    shift_x: x2,
                    shift_y: y2,
                    shift_z: z2,
                },
            ) => {
                n1 == n2
                    && f64_key(*a1) == f64_key(*a2)
                    && f64_key(*b1) == f64_key(*b2)
                    && x1 == x2
                    && y1 == y2
                    && z1 == z2
            }
            (D::EndIslands, D::EndIslands) => true,
            (
                D::DistanceToPoint {
                    point: p1,
                    metric: m1,
                },
                D::DistanceToPoint {
                    point: p2,
                    metric: m2,
                },
            ) => p1 == p2 && m1 == m2,
            (
                D::Gradient {
                    axis: a1,
                    tiling: t1,
                    from_coordinate: f1,
                    to_coordinate: c1,
                    from_value: v1,
                    to_value: w1,
                },
                D::Gradient {
                    axis: a2,
                    tiling: t2,
                    from_coordinate: f2,
                    to_coordinate: c2,
                    from_value: v2,
                    to_value: w2,
                },
            ) => {
                a1 == a2
                    && t1 == t2
                    && f1 == f2
                    && c1 == c2
                    && f32_key(*v1) == f32_key(*v2)
                    && f32_key(*w1) == f32_key(*w2)
            }
            (D::ShiftA(a), D::ShiftA(b)) | (D::ShiftB(a), D::ShiftB(b)) | (D::Shift(a), D::Shift(b)) => {
                a == b
            }
            (D::Unary(k1, i1), D::Unary(k2, i2)) => k1 == k2 && i1 == i2,
            (
                D::Round {
                    kind: k1,
                    input: i1,
                    multiple: m1,
                },
                D::Round {
                    kind: k2,
                    input: i2,
                    multiple: m2,
                },
            ) => k1 == k2 && i1 == i2 && m1 == m2,
            (D::Binary(k1, l1, r1), D::Binary(k2, l2, r2)) => k1 == k2 && l1 == l2 && r1 == r2,
            (
                D::Pow {
                    base: b1,
                    exponent: e1,
                },
                D::Pow {
                    base: b2,
                    exponent: e2,
                },
            ) => b1 == b2 && e1 == e2,
            (D::Spline(a), D::Spline(b)) => a == b,
            (
                D::Lerp {
                    alpha: a1,
                    first: f1,
                    second: s1,
                },
                D::Lerp {
                    alpha: a2,
                    first: f2,
                    second: s2,
                },
            ) => a1 == a2 && f1 == f2 && s1 == s2,
            (
                D::Clamp {
                    input: i1,
                    min: a1,
                    max: b1,
                },
                D::Clamp {
                    input: i2,
                    min: a2,
                    max: b2,
                },
            ) => i1 == i2 && f32_key(*a1) == f32_key(*a2) && f32_key(*b1) == f32_key(*b2),
            (
                D::RangeChoice {
                    input: i1,
                    min_inclusive: a1,
                    max_exclusive: b1,
                    when_in_range: w1,
                    when_out_of_range: o1,
                },
                D::RangeChoice {
                    input: i2,
                    min_inclusive: a2,
                    max_exclusive: b2,
                    when_in_range: w2,
                    when_out_of_range: o2,
                },
            ) => {
                i1 == i2
                    && f32_key(*a1) == f32_key(*a2)
                    && f32_key(*b1) == f32_key(*b2)
                    && w1 == w2
                    && o1 == o2
            }
            (
                D::IntervalSelect {
                    input: i1,
                    thresholds: t1,
                    functions: f1,
                },
                D::IntervalSelect {
                    input: i2,
                    thresholds: t2,
                    functions: f2,
                },
            ) => i1 == i2 && t1.len() == t2.len() && t1.iter().zip(t2).all(|(a, b)| a == b) && f1 == f2,
            (D::Cache(a), D::Cache(b)) | (D::BlendDensity(a), D::BlendDensity(b)) => a == b,
            (
                D::Interpolated {
                    input: i1,
                    cell_size_xz: a1,
                    cell_size_y: b1,
                },
                D::Interpolated {
                    input: i2,
                    cell_size_xz: a2,
                    cell_size_y: b2,
                },
            ) => i1 == i2 && a1 == a2 && b1 == b2,
            (
                D::Slice {
                    axis: a1,
                    coordinate: c1,
                    input: i1,
                },
                D::Slice {
                    axis: a2,
                    coordinate: c2,
                    input: i2,
                },
            ) => a1 == a2 && c1 == c2 && i1 == i2,
            (
                D::FindTopSurface {
                    density: d1,
                    upper_bound: u1,
                    lower_bound: l1,
                    cell_height: c1,
                },
                D::FindTopSurface {
                    density: d2,
                    upper_bound: u2,
                    lower_bound: l2,
                    cell_height: c2,
                },
            ) => d1 == d2 && u1 == u2 && l1 == l2 && c1 == c2,
            (D::OldBlendedNoise(a), D::OldBlendedNoise(b)) => {
                f64_key(a.xz_scale) == f64_key(b.xz_scale)
                    && f64_key(a.y_scale) == f64_key(b.y_scale)
                    && f64_key(a.xz_factor) == f64_key(b.xz_factor)
                    && f64_key(a.y_factor) == f64_key(b.y_factor)
                    && f64_key(a.smear_scale_multiplier) == f64_key(b.smear_scale_multiplier)
            }
            (D::Reference { id: a, .. }, D::Reference { id: b, .. }) => a == b,
            (D::PreparedCache { id: a, .. }, D::PreparedCache { id: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl Eq for DensityFunction {}

impl Hash for DensityFunction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use DensityFunction as D;
        std::mem::discriminant(self).hash(state);
        match self {
            D::Constant(v) => f32_key(*v).hash(state),
            D::Simple(k) => k.hash(state),
            D::Noise {
                noise,
                xz_scale,
                y_scale,
                shift_x,
                shift_y,
                shift_z,
            } => {
                noise.hash(state);
                f64_key(*xz_scale).hash(state);
                f64_key(*y_scale).hash(state);
                shift_x.hash(state);
                shift_y.hash(state);
                shift_z.hash(state);
            }
            D::EndIslands => {}
            D::DistanceToPoint { point, metric } => {
                point.hash(state);
                metric.hash(state);
            }
            D::Gradient {
                axis,
                tiling,
                from_coordinate,
                to_coordinate,
                from_value,
                to_value,
            } => {
                axis.hash(state);
                tiling.hash(state);
                from_coordinate.hash(state);
                to_coordinate.hash(state);
                f32_key(*from_value).hash(state);
                f32_key(*to_value).hash(state);
            }
            D::ShiftA(n) | D::ShiftB(n) | D::Shift(n) => n.hash(state),
            D::Unary(k, i) => {
                k.hash(state);
                i.hash(state);
            }
            D::Round {
                kind,
                input,
                multiple,
            } => {
                kind.hash(state);
                input.hash(state);
                multiple.hash(state);
            }
            D::Binary(k, l, r) => {
                k.hash(state);
                l.hash(state);
                r.hash(state);
            }
            D::Pow { base, exponent } => {
                base.hash(state);
                exponent.hash(state);
            }
            D::Spline(s) => s.hash(state),
            D::Lerp { alpha, first, second } => {
                alpha.hash(state);
                first.hash(state);
                second.hash(state);
            }
            D::Clamp { input, min, max } => {
                input.hash(state);
                f32_key(*min).hash(state);
                f32_key(*max).hash(state);
            }
            D::RangeChoice {
                input,
                min_inclusive,
                max_exclusive,
                when_in_range,
                when_out_of_range,
            } => {
                input.hash(state);
                f32_key(*min_inclusive).hash(state);
                f32_key(*max_exclusive).hash(state);
                when_in_range.hash(state);
                when_out_of_range.hash(state);
            }
            D::IntervalSelect {
                input,
                thresholds,
                functions,
            } => {
                input.hash(state);
                for t in thresholds {
                    f32_key(*t).hash(state);
                }
                functions.hash(state);
            }
            D::Cache(i) | D::BlendDensity(i) => i.hash(state),
            D::Interpolated {
                input,
                cell_size_xz,
                cell_size_y,
            } => {
                input.hash(state);
                cell_size_xz.hash(state);
                cell_size_y.hash(state);
            }
            D::Slice {
                axis,
                coordinate,
                input,
            } => {
                axis.hash(state);
                coordinate.hash(state);
                input.hash(state);
            }
            D::FindTopSurface {
                density,
                upper_bound,
                lower_bound,
                cell_height,
            } => {
                density.hash(state);
                upper_bound.hash(state);
                lower_bound.hash(state);
                cell_height.hash(state);
            }
            D::OldBlendedNoise(p) => {
                f64_key(p.xz_scale).hash(state);
                f64_key(p.y_scale).hash(state);
                f64_key(p.xz_factor).hash(state);
                f64_key(p.y_factor).hash(state);
                f64_key(p.smear_scale_multiplier).hash(state);
            }
            D::Reference { id, .. } => id.hash(state),
            D::PreparedCache { id, .. } => id.hash(state),
        }
    }
}
