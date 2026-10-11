//! `DensityFunctionCompiler` + `DfRewriteRule` + every `compileSampler` of 26.3.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::sampler::{ContextKey, GradientMode, Sampler, SamplerRef};
use super::spline::{CompiledSpline, Spline};
use super::{
    Axis, BinaryKind, DensityFunction as D, Df, NoiseHolder, SimpleKind, TilingMode, UnaryKind, axes,
};
use crate::noise::{NoiseStack, SimplexNoise};
use crate::random::{AnyRandom, LegacyRandom, RandomSource};

/// Environment needed while compiling (`DensityFunction.CompileContext`).
pub trait CompileEnv {
    /// `createNoiseSampler(holder)`.
    fn noise(&self, holder: &NoiseHolder) -> Arc<NoiseStack>;
    /// `createRandom(identifier)`.
    fn random(&self, id: &str) -> AnyRandom;
    /// `createEndIslandRandom()`.
    fn end_island_random(&self) -> LegacyRandom;
}

/// Hash-map key wrapper using structural equality of density functions.
#[derive(Clone)]
struct Key(Df);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}
impl Eq for Key {}
impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

/// Stateful compiler; one per `RandomState`. Cache ids are shared across all compiled roots.
pub struct DensityCompiler {
    prepared: FxHashMap<Key, Df>,
    samplers: FxHashMap<Key, SamplerRef>,
    next_cache_id: usize,
}

impl Default for DensityCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl DensityCompiler {
    pub fn new() -> Self {
        Self {
            prepared: FxHashMap::default(),
            samplers: FxHashMap::default(),
            next_cache_id: 0,
        }
    }

    /// Number of cache cells allocated so far.
    pub fn cache_count(&self) -> usize {
        self.next_cache_id
    }

    /// `getSampler(function)`: optimise and compile, memoised per (structurally equal) root.
    pub fn sampler(&mut self, env: &dyn CompileEnv, function: &Df) -> SamplerRef {
        if let Some(s) = self.samplers.get(&Key(function.clone())) {
            return s.clone();
        }
        let optimized = self.optimize(env, function);
        let sampler = compile(env, &optimized);
        self.samplers.insert(Key(function.clone()), sampler.clone());
        sampler
    }

    /// The optimised tree (for diagnostics/tests).
    pub fn optimize(&mut self, env: &dyn CompileEnv, function: &Df) -> Df {
        let f = self.rule_inline(env, function);
        slice_uniform_axes(axes::ALL, &f)
    }

    /// The first optimiser rule: inline references, deduplicate caches, recurse.
    fn rule_inline(&mut self, env: &dyn CompileEnv, function: &Df) -> Df {
        let f = match &**function {
            D::Reference { target, .. } => target.clone(),
            _ => function.clone(),
        };
        if let D::Cache(_) = &*f {
            return self.reuse_or_prepare(env, &f);
        }
        rewrite_children(&f, &mut |c| self.rule_inline(env, c))
    }

    fn reuse_or_prepare(&mut self, env: &dyn CompileEnv, cache: &Df) -> Df {
        let D::Cache(input) = &**cache else { unreachable!() };
        if let Some(p) = self.prepared.get(&Key(input.clone())) {
            return p.clone();
        }
        let id = self.next_cache_id;
        self.next_cache_id += 1;
        let optimized_input = self.optimize(env, input);
        let caching = Arc::new(Sampler::Caching {
            id,
            input: compile(env, &optimized_input),
        });
        let prepared = Arc::new(D::PreparedCache {
            id,
            range: optimized_input.range(),
            domain_axes: optimized_input.domain_axes(),
            sampler: caching,
        });
        self.prepared.insert(Key(input.clone()), prepared.clone());
        prepared
    }
}

/// `DfRewriteRule.SliceUniformAxes`.
fn slice_uniform_axes(parent: u8, function: &Df) -> Df {
    if matches!(&**function, D::Constant(_) | D::Gradient { .. }) {
        return function.clone();
    }
    let domain = function.domain_axes();
    if parent == domain {
        return rewrite_children(function, &mut |c| slice_uniform_axes(parent, c));
    }
    let new = rewrite_children(function, &mut |c| slice_uniform_axes(domain, c));
    remove_axes(new, parent & !domain)
}

fn remove_axes(mut function: Df, removed: u8) -> Df {
    let mut existing = 0u8;
    let mut cur = &function;
    while let D::Slice { axis, input, .. } = &**cur {
        existing |= axis.mask();
        cur = input;
    }
    let filtered = removed & !existing;
    if filtered & axes::X != 0 {
        function = Arc::new(D::Slice {
            axis: Axis::X,
            coordinate: 0,
            input: function,
        });
    }
    if filtered & axes::Z != 0 {
        function = Arc::new(D::Slice {
            axis: Axis::Z,
            coordinate: 0,
            input: function,
        });
    }
    if filtered & axes::Y != 0 {
        function = Arc::new(D::Slice {
            axis: Axis::Y,
            coordinate: 0,
            input: function,
        });
    }
    function
}

/// `rewriteChildren(rule)`, preserving identity when nothing changed.
fn rewrite_children(f: &Df, rule: &mut dyn FnMut(&Df) -> Df) -> Df {
    macro_rules! same {
        ($($a:expr, $b:expr);*) => { true $(&& Arc::ptr_eq($a, $b))* };
    }
    match &**f {
        D::Constant(_)
        | D::Simple(_)
        | D::EndIslands
        | D::DistanceToPoint { .. }
        | D::Gradient { .. }
        | D::OldBlendedNoise(_)
        | D::PreparedCache { .. } => f.clone(),
        D::ShiftA(n) => Arc::new(D::ShiftA(n.clone())),
        D::ShiftB(n) => Arc::new(D::ShiftB(n.clone())),
        D::Shift(n) => Arc::new(D::Shift(n.clone())),
        D::Noise {
            noise,
            xz_scale,
            y_scale,
            shift_x,
            shift_y,
            shift_z,
        } => {
            let (x, y, z) = (rule(shift_x), rule(shift_y), rule(shift_z));
            if same!(&x, shift_x; &y, shift_y; &z, shift_z) {
                f.clone()
            } else {
                Arc::new(D::Noise {
                    noise: noise.clone(),
                    xz_scale: *xz_scale,
                    y_scale: *y_scale,
                    shift_x: x,
                    shift_y: y,
                    shift_z: z,
                })
            }
        }
        D::Unary(k, i) => {
            let n = rule(i);
            if same!(&n, i) {
                f.clone()
            } else {
                Arc::new(D::Unary(*k, n))
            }
        }
        D::Round {
            kind,
            input,
            multiple,
        } => {
            let (i, m) = (rule(input), rule(multiple));
            if same!(&i, input; &m, multiple) {
                f.clone()
            } else {
                Arc::new(D::Round {
                    kind: *kind,
                    input: i,
                    multiple: m,
                })
            }
        }
        D::Binary(k, l, r) => {
            let (nl, nr) = (rule(l), rule(r));
            if same!(&nl, l; &nr, r) {
                f.clone()
            } else {
                Arc::new(D::Binary(*k, nl, nr))
            }
        }
        D::Pow { base, exponent } => {
            let (b, e) = (rule(base), rule(exponent));
            if same!(&b, base; &e, exponent) {
                f.clone()
            } else {
                Arc::new(D::Pow { base: b, exponent: e })
            }
        }
        D::Spline(s) => {
            let mut changed = false;
            let new = s.map_coordinates(&mut |c| {
                let n = rule(c);
                if !Arc::ptr_eq(&n, c) {
                    changed = true;
                }
                n
            });
            if changed {
                Arc::new(D::Spline(Arc::new(new)))
            } else {
                f.clone()
            }
        }
        D::Lerp { alpha, first, second } => {
            let (a, b, c) = (rule(alpha), rule(first), rule(second));
            if same!(&a, alpha; &b, first; &c, second) {
                f.clone()
            } else {
                Arc::new(D::Lerp {
                    alpha: a,
                    first: b,
                    second: c,
                })
            }
        }
        D::Clamp { input, min, max } => {
            let i = rule(input);
            if same!(&i, input) {
                f.clone()
            } else {
                Arc::new(D::Clamp {
                    input: i,
                    min: *min,
                    max: *max,
                })
            }
        }
        D::RangeChoice {
            input,
            min_inclusive,
            max_exclusive,
            when_in_range,
            when_out_of_range,
        } => {
            let (i, a, b) = (rule(input), rule(when_in_range), rule(when_out_of_range));
            if same!(&i, input; &a, when_in_range; &b, when_out_of_range) {
                f.clone()
            } else {
                Arc::new(D::RangeChoice {
                    input: i,
                    min_inclusive: *min_inclusive,
                    max_exclusive: *max_exclusive,
                    when_in_range: a,
                    when_out_of_range: b,
                })
            }
        }
        D::IntervalSelect {
            input,
            thresholds,
            functions,
        } => {
            let i = rule(input);
            let mut changed = !Arc::ptr_eq(&i, input);
            let fs: Vec<Df> = functions
                .iter()
                .map(|x| {
                    let n = rule(x);
                    if !Arc::ptr_eq(&n, x) {
                        changed = true;
                    }
                    n
                })
                .collect();
            if changed {
                Arc::new(D::IntervalSelect {
                    input: i,
                    thresholds: thresholds.clone(),
                    functions: fs,
                })
            } else {
                f.clone()
            }
        }
        D::Cache(i) => {
            let n = rule(i);
            if same!(&n, i) {
                f.clone()
            } else {
                Arc::new(D::Cache(n))
            }
        }
        D::BlendDensity(i) => {
            let n = rule(i);
            if same!(&n, i) {
                f.clone()
            } else {
                Arc::new(D::BlendDensity(n))
            }
        }
        D::Interpolated {
            input,
            cell_size_xz,
            cell_size_y,
        } => {
            let n = rule(input);
            if same!(&n, input) {
                f.clone()
            } else {
                Arc::new(D::Interpolated {
                    input: n,
                    cell_size_xz: *cell_size_xz,
                    cell_size_y: *cell_size_y,
                })
            }
        }
        D::Slice {
            axis,
            coordinate,
            input,
        } => {
            let n = rule(input);
            if same!(&n, input) {
                f.clone()
            } else {
                Arc::new(D::Slice {
                    axis: *axis,
                    coordinate: *coordinate,
                    input: n,
                })
            }
        }
        D::FindTopSurface {
            density,
            upper_bound,
            lower_bound,
            cell_height,
        } => {
            let (d, u) = (rule(density), rule(upper_bound));
            if same!(&d, density; &u, upper_bound) {
                f.clone()
            } else {
                Arc::new(D::FindTopSurface {
                    density: d,
                    upper_bound: u,
                    lower_bound: *lower_bound,
                    cell_height: *cell_height,
                })
            }
        }
        // `HolderHolder.rewriteChildren`: rewrite the target, keep the reference if unchanged.
        D::Reference { target, .. } => {
            let n = rule(target);
            if Arc::ptr_eq(&n, target) { f.clone() } else { n }
        }
    }
}

/// `compileSampler` of every density function type.
pub fn compile(env: &dyn CompileEnv, f: &Df) -> SamplerRef {
    match &**f {
        D::PreparedCache { sampler, .. } => sampler.clone(),
        D::Reference { target, .. } => compile(env, target),
        _ => compile_inner(env, f),
    }
}

fn compile_inner(env: &dyn CompileEnv, f: &Df) -> SamplerRef {
    use Sampler as S;
    let sampler: Sampler = match &**f {
        D::Constant(v) => S::Constant(*v),
        D::Simple(SimpleKind::BlendAlpha) => S::ContextBound {
            key: ContextKey::BlendAlpha,
            fallback: 1.0,
        },
        D::Simple(SimpleKind::BlendOffset) => S::ContextBound {
            key: ContextKey::BlendOffset,
            fallback: 0.0,
        },
        D::Simple(SimpleKind::Beardifier) => S::ContextBound {
            key: ContextKey::Beardifier,
            fallback: 0.0,
        },
        D::Noise {
            noise,
            xz_scale,
            y_scale,
            shift_x,
            shift_y,
            shift_z,
        } => {
            let n = env.noise(noise);
            if shift_x.is_zero_constant() && shift_y.is_zero_constant() && shift_z.is_zero_constant() {
                S::Noise {
                    noise: n,
                    xz_scale: *xz_scale,
                    y_scale: *y_scale,
                }
            } else {
                let sx = compile(env, shift_x);
                let sz = compile(env, shift_z);
                if shift_y.is_zero_constant() {
                    S::NoiseShiftedXz {
                        shift_x: sx,
                        shift_z: sz,
                        noise: n,
                        xz_scale: *xz_scale,
                        y_scale: *y_scale,
                    }
                } else {
                    let sy = compile(env, shift_y);
                    S::NoiseShiftedXyz {
                        shift_x: sx,
                        shift_y: sy,
                        shift_z: sz,
                        noise: n,
                        xz_scale: *xz_scale,
                        y_scale: *y_scale,
                    }
                }
            }
        }
        D::EndIslands => {
            let mut random = env.end_island_random();
            random.consume_count(17292);
            S::EndIslands {
                noise: Box::new(SimplexNoise::new_discard_offset(&mut random, true)),
            }
        }
        D::DistanceToPoint { point, metric } => S::DistanceToPoint {
            x: point[0],
            y: point[1],
            z: point[2],
            metric: *metric,
        },
        D::Gradient {
            axis,
            tiling,
            from_coordinate,
            to_coordinate,
            from_value,
            to_value,
        } => {
            let range = to_coordinate - from_coordinate;
            let factor = (to_value - from_value) / range as f32;
            let mode = match tiling {
                TilingMode::ClampToEdge => GradientMode::Clamped {
                    min: (*from_coordinate).min(*to_coordinate),
                    max: (*from_coordinate).max(*to_coordinate),
                },
                TilingMode::Repeat => GradientMode::Repeat { range },
                TilingMode::MirroredRepeat => GradientMode::MirroredRepeat { range },
            };
            S::Gradient {
                axis: *axis,
                from_coordinate: *from_coordinate,
                from_value: *from_value,
                factor,
                mode,
            }
        }
        D::ShiftA(n) => S::ConstMul(
            Arc::new(S::Noise {
                noise: env.noise(n),
                xz_scale: 0.25,
                y_scale: 0.0,
            }),
            4.0,
        ),
        D::Shift(n) => S::ConstMul(
            Arc::new(S::Noise {
                noise: env.noise(n),
                xz_scale: 0.25,
                y_scale: 0.25,
            }),
            4.0,
        ),
        D::ShiftB(n) => S::ShiftB { noise: env.noise(n) },
        D::Unary(kind, input) => {
            let i = compile(env, input);
            match kind {
                UnaryKind::Abs => S::Abs(i),
                UnaryKind::Square => S::Square(i),
                UnaryKind::Cube => S::Cube(i),
                UnaryKind::Sqrt => S::Sqrt(i),
                UnaryKind::HalfNegative => S::LeakyRelu(i, 0.5),
                UnaryKind::QuarterNegative => S::LeakyRelu(i, 0.25),
                UnaryKind::Reciprocal => S::Reciprocal(i),
                UnaryKind::Negate => S::Negate(i),
                UnaryKind::Squeeze => S::Squeeze(i),
                UnaryKind::Log => S::Log(i),
                UnaryKind::Sign => S::Sign(i),
            }
        }
        D::Round {
            kind,
            input,
            multiple,
        } => {
            let i = compile(env, input);
            if multiple.constant_value() == Some(1.0) {
                S::RoundInteger(*kind, i)
            } else {
                S::Round(*kind, i, compile(env, multiple))
            }
        }
        D::Binary(kind, left, right) => {
            let l = compile(env, left);
            let r = compile(env, right);
            let lc = left.constant_value();
            let rc = right.constant_value();
            match kind {
                BinaryKind::Add => match (lc, rc) {
                    (Some(c), _) => S::ConstAdd(r, c),
                    (_, Some(c)) => S::ConstAdd(l, c),
                    _ => S::Add(l, r),
                },
                BinaryKind::Sub => match (lc, rc) {
                    (Some(c), _) => S::ConstSub(c, r),
                    (_, Some(c)) => S::ConstAdd(l, -c),
                    _ => S::Sub(l, r),
                },
                BinaryKind::Mul => match (lc, rc) {
                    (Some(c), _) => S::ConstMul(r, c),
                    (_, Some(c)) => S::ConstMul(l, c),
                    _ => S::Mul(l, r),
                },
                BinaryKind::Div => match (lc, rc) {
                    (Some(c), _) => S::ConstDiv(c, r),
                    (_, Some(c)) => S::ConstMul(l, 1.0 / c),
                    _ => S::Div(l, r),
                },
                BinaryKind::Min => {
                    let (lr, rr) = (left.range(), right.range());
                    if lr.max() < rr.min() {
                        tracing::warn!("compiling min between non-overlapping inputs");
                        return l;
                    } else if rr.max() < lr.min() {
                        tracing::warn!("compiling min between non-overlapping inputs");
                        return r;
                    } else {
                        match (lc, rc) {
                            (Some(c), _) => S::ConstMin(r, c),
                            (_, Some(c)) => S::ConstMin(l, c),
                            _ => S::Min {
                                left: l,
                                right: r,
                                right_min: rr.min(),
                            },
                        }
                    }
                }
                BinaryKind::Max => {
                    let (lr, rr) = (left.range(), right.range());
                    if lr.min() > rr.max() {
                        tracing::warn!("compiling max between non-overlapping inputs");
                        return l;
                    } else if rr.min() > lr.max() {
                        tracing::warn!("compiling max between non-overlapping inputs");
                        return r;
                    } else {
                        match (lc, rc) {
                            (Some(c), _) => S::ConstMax(r, c),
                            (_, Some(c)) => S::ConstMax(l, c),
                            _ => S::Max {
                                left: l,
                                right: r,
                                right_max: rr.max(),
                            },
                        }
                    }
                }
            }
        }
        D::Pow { base, exponent } => {
            let b = compile(env, base);
            let e = compile(env, exponent);
            if let Some(c) = base.constant_value() {
                S::PowConstBase(c as f64, e)
            } else if let Some(c) = exponent.constant_value() {
                let abs = c.abs();
                let special: SamplerRef = if abs == 0.5 {
                    Arc::new(S::Sqrt(b))
                } else if abs == 1.0 {
                    b
                } else if abs == 2.0 {
                    Arc::new(S::Square(b))
                } else if abs == 3.0 {
                    Arc::new(S::Cube(b))
                } else {
                    return Arc::new(S::PowConstExponent(b, c as f64));
                };
                if c >= 0.0 {
                    return special;
                }
                S::Reciprocal(special)
            } else {
                S::Pow(b, e)
            }
        }
        D::Spline(spline) => {
            let mut keys: Vec<Df> = Vec::new();
            let mut samplers: Vec<SamplerRef> = Vec::new();
            let compiled = compile_spline(env, spline, &mut keys, &mut samplers);
            S::Spline {
                spline: compiled,
                coordinates: samplers,
            }
        }
        D::Lerp { alpha, first, second } => {
            let a = compile(env, alpha);
            let fi = compile(env, first);
            let se = compile(env, second);
            if let Some(c) = first.constant_value() {
                S::LerpConstFirst(a, c, se)
            } else if let Some(c) = second.constant_value() {
                S::LerpConstSecond(a, fi, c)
            } else {
                S::Lerp(a, fi, se)
            }
        }
        D::Clamp { input, min, max } => S::Clamp(compile(env, input), *min, *max),
        D::RangeChoice {
            input,
            min_inclusive,
            max_exclusive,
            when_in_range,
            when_out_of_range,
        } => {
            let i = compile(env, input);
            match (when_in_range.constant_value(), when_out_of_range.constant_value()) {
                (Some(a), Some(b)) => S::RangeChoiceConst {
                    input: i,
                    min: *min_inclusive,
                    max: *max_exclusive,
                    in_range: a,
                    out_of_range: b,
                },
                _ => S::RangeChoice {
                    input: i,
                    min: *min_inclusive,
                    max: *max_exclusive,
                    in_range: compile(env, when_in_range),
                    out_of_range: compile(env, when_out_of_range),
                },
            }
        }
        D::IntervalSelect {
            input,
            thresholds,
            functions,
        } => {
            let i = compile(env, input);
            if thresholds.len() == 1 {
                S::IntervalSelectSingle {
                    input: i,
                    threshold: thresholds[0],
                    if_below: compile(env, functions.first().unwrap()),
                    if_above: compile(env, functions.last().unwrap()),
                }
            } else {
                S::IntervalSelect {
                    input: i,
                    thresholds: thresholds.clone(),
                    samplers: functions.iter().map(|f| compile(env, f)).collect(),
                }
            }
        }
        D::Cache(_) => panic!("Cannot compile cache before it has been deduplicated"),
        D::PreparedCache { .. } | D::Reference { .. } => unreachable!("handled by compile()"),
        D::BlendDensity(i) => S::BlendDensity(compile(env, i)),
        D::Interpolated {
            input,
            cell_size_xz,
            cell_size_y,
        } => S::Interpolated {
            input: compile(env, input),
            cell_xz: *cell_size_xz,
            cell_y: *cell_size_y,
            inv_xz: 1.0 / *cell_size_xz as f32,
            inv_y: 1.0 / *cell_size_y as f32,
        },
        D::Slice {
            axis,
            coordinate,
            input,
        } => {
            if let D::Slice {
                axis: inner_axis,
                coordinate: inner_coord,
                input: inner,
            } = &**input
                && ((*axis == Axis::X && *inner_axis == Axis::Z)
                    || (*axis == Axis::Z && *inner_axis == Axis::X))
            {
                let (x, z) = if *axis == Axis::X {
                    (*coordinate, *inner_coord)
                } else {
                    (*inner_coord, *coordinate)
                };
                return Arc::new(S::SliceXz(compile(env, inner), x, z));
            }
            let i = compile(env, input);
            match axis {
                Axis::X => S::SliceX(i, *coordinate),
                Axis::Y => S::SliceY(i, *coordinate),
                Axis::Z => S::SliceZ(i, *coordinate),
            }
        }
        D::FindTopSurface {
            density,
            upper_bound,
            lower_bound,
            cell_height,
        } => {
            let inner = Sampler::FindTopSurface {
                density: compile(env, density),
                upper_bound: compile(env, upper_bound),
                lower_bound: *lower_bound,
                cell_height: *cell_height,
            };
            S::SliceY(Arc::new(inner), 0)
        }
        D::OldBlendedNoise(params) => {
            return crate::noise::compile_blended_noise(params, &mut env.random("minecraft:terrain"));
        }
    };
    Arc::new(sampler)
}

fn compile_spline(
    env: &dyn CompileEnv,
    spline: &Spline,
    keys: &mut Vec<Df>,
    samplers: &mut Vec<SamplerRef>,
) -> CompiledSpline {
    match spline {
        Spline::Constant(v) => CompiledSpline::Constant(*v),
        Spline::Multipoint {
            coordinate,
            locations,
            values,
            derivatives,
        } => {
            let index = match keys.iter().position(|k| **k == **coordinate) {
                Some(i) => i,
                None => {
                    keys.push(coordinate.clone());
                    samplers.push(compile(env, coordinate));
                    keys.len() - 1
                }
            };
            let values = values
                .iter()
                .map(|v| compile_spline(env, v, keys, samplers))
                .collect();
            CompiledSpline::Multipoint {
                coordinate: index,
                locations: locations.clone(),
                values,
                derivatives: derivatives.clone(),
            }
        }
    }
}
