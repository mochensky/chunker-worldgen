//! `CubicSpline<SplineFunction.Coordinate>` and its range computation.

use std::hash::{Hash, Hasher};

use super::Df;
use crate::math::{Interval, lerp_f32, max_f32, min_f32};

#[derive(Debug)]
pub enum Spline {
    Constant(f32),
    Multipoint {
        coordinate: Df,
        locations: Vec<f32>,
        values: Vec<Spline>,
        derivatives: Vec<f32>,
    },
}

impl PartialEq for Spline {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Spline::Constant(a), Spline::Constant(b)) => {
                a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
            }
            (
                Spline::Multipoint {
                    coordinate: c1,
                    locations: l1,
                    values: v1,
                    derivatives: d1,
                },
                Spline::Multipoint {
                    coordinate: c2,
                    locations: l2,
                    values: v2,
                    derivatives: d2,
                },
            ) => {
                // `Arrays.equals(float[], float[])` compares with `Float.floatToIntBits`.
                let feq = |a: &[f32], b: &[f32]| {
                    a.len() == b.len()
                        && a.iter()
                            .zip(b)
                            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
                };
                c1 == c2 && feq(l1, l2) && feq(d1, d2) && v1 == v2
            }
            _ => false,
        }
    }
}

impl Eq for Spline {}

impl Hash for Spline {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Spline::Constant(v) => {
                0u8.hash(state);
                v.to_bits().hash(state);
            }
            Spline::Multipoint {
                coordinate,
                locations,
                values,
                derivatives,
            } => {
                1u8.hash(state);
                coordinate.hash(state);
                for l in locations {
                    l.to_bits().hash(state);
                }
                for d in derivatives {
                    d.to_bits().hash(state);
                }
                values.hash(state);
            }
        }
    }
}

impl Spline {
    pub fn for_each_coordinate(&self, f: &mut dyn FnMut(&Df)) {
        if let Spline::Multipoint {
            coordinate, values, ..
        } = self
        {
            f(coordinate);
            for v in values {
                v.for_each_coordinate(f);
            }
        }
    }

    /// Returns a copy with every coordinate mapped (`mapCoordinates`), preserving traversal order:
    /// own coordinate first, then values depth-first.
    pub fn map_coordinates(&self, f: &mut dyn FnMut(&Df) -> Df) -> Spline {
        match self {
            Spline::Constant(v) => Spline::Constant(*v),
            Spline::Multipoint {
                coordinate,
                locations,
                values,
                derivatives,
            } => {
                let coordinate = f(coordinate);
                let values = values.iter().map(|v| v.map_coordinates(f)).collect();
                Spline::Multipoint {
                    coordinate,
                    locations: locations.clone(),
                    values,
                    derivatives: derivatives.clone(),
                }
            }
        }
    }

    pub fn range(&self) -> Interval {
        match self {
            Spline::Constant(v) => Interval::of_exact(*v),
            Spline::Multipoint {
                coordinate,
                locations,
                values,
                derivatives,
            } => {
                let last = locations.len() - 1;
                let mut min_value = f32::INFINITY;
                let mut max_value = f32::NEG_INFINITY;
                let input = coordinate.range();
                if input.is_nai() {
                    return input;
                }
                if input.min() < locations[0] {
                    let first = values[0].range();
                    let e1 = linear_extend(input.min(), locations, first.min(), derivatives, 0);
                    let e2 = linear_extend(input.min(), locations, first.max(), derivatives, 0);
                    min_value = min_f32(min_value, min_f32(e1, e2));
                    max_value = max_f32(max_value, max_f32(e1, e2));
                }
                if input.max() > locations[last] {
                    let lr = values[last].range();
                    let e1 = linear_extend(input.max(), locations, lr.min(), derivatives, last);
                    let e2 = linear_extend(input.max(), locations, lr.max(), derivatives, last);
                    min_value = min_f32(min_value, min_f32(e1, e2));
                    max_value = max_f32(max_value, max_f32(e1, e2));
                }
                let ranges: Vec<Interval> = values.iter().map(Spline::range).collect();
                for r in &ranges {
                    min_value = min_f32(min_value, r.min());
                    max_value = max_f32(max_value, r.max());
                }
                for i in 0..last {
                    let x_diff = locations[i + 1] - locations[i];
                    let (r1, r2) = (ranges[i], ranges[i + 1]);
                    let (min1, max1, min2, max2) = (r1.min(), r1.max(), r2.min(), r2.max());
                    let d1 = derivatives[i];
                    let d2 = derivatives[i + 1];
                    if d1 != 0.0 || d2 != 0.0 {
                        let p1 = d1 * x_diff;
                        let p2 = d2 * x_diff;
                        let min_lerp1 = min_f32(min1, min2);
                        let max_lerp1 = max_f32(max1, max2);
                        let min_a = p1 - max2 + min1;
                        let max_a = p1 - min2 + max1;
                        let min_b = -p2 + min2 - max1;
                        let max_b = -p2 + max2 - min1;
                        let min_lerp2 = min_f32(min_a, min_b);
                        let max_lerp2 = max_f32(max_a, max_b);
                        min_value = min_f32(min_value, min_lerp1 + 0.25 * min_lerp2);
                        max_value = max_f32(max_value, max_lerp1 + 0.25 * max_lerp2);
                    }
                }
                Interval::of(min_value, max_value)
            }
        }
    }
}

#[inline(always)]
pub(crate) fn linear_extend(
    input: f32,
    locations: &[f32],
    value: f32,
    derivatives: &[f32],
    index: usize,
) -> f32 {
    let d = derivatives[index];
    if d == 0.0 {
        value
    } else {
        value + d * (input - locations[index])
    }
}

/// Compiled spline whose coordinates are indices into a per-sample coordinate cache.
#[derive(Debug)]
pub enum CompiledSpline {
    Constant(f32),
    Multipoint {
        coordinate: usize,
        locations: Vec<f32>,
        values: Vec<CompiledSpline>,
        derivatives: Vec<f32>,
    },
}

impl CompiledSpline {
    /// `CubicSpline.Multipoint.sample`, `coord(i)` returns the value of coordinate `i`.
    pub fn sample(&self, coord: &mut dyn FnMut(usize) -> f32) -> f32 {
        match self {
            CompiledSpline::Constant(v) => *v,
            CompiledSpline::Multipoint {
                coordinate,
                locations,
                values,
                derivatives,
            } => {
                let input = coord(*coordinate);
                let start =
                    crate::math::binary_search(0, locations.len() as i32, |i| input < locations[i as usize])
                        - 1;
                let last = locations.len() - 1;
                if start < 0 {
                    linear_extend(input, locations, values[0].sample(coord), derivatives, 0)
                } else if start as usize == last {
                    linear_extend(input, locations, values[last].sample(coord), derivatives, last)
                } else {
                    let s = start as usize;
                    let x1 = locations[s];
                    let x2 = locations[s + 1];
                    let t = (input - x1) / (x2 - x1);
                    let d1 = derivatives[s];
                    let d2 = derivatives[s + 1];
                    let y1 = values[s].sample(coord);
                    let y2 = values[s + 1].sample(coord);
                    let a = d1 * (x2 - x1) - (y2 - y1);
                    let b = -d2 * (x2 - x1) + (y2 - y1);
                    lerp_f32(t, y1, y2) + t * (1.0 - t) * lerp_f32(t, a, b)
                }
            }
        }
    }
}
