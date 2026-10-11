//! Port of `net.minecraft.util.Interval` (float value ranges used to optimise density functions).
//!
//! The ranges influence compilation (`min`/`max` short-circuits), so this must match vanilla exactly.

use super::{max_f32, min_f32, signum_f32, square_f32};

#[derive(Clone, Copy, Debug)]
pub struct Interval {
    min: f32,
    max: f32,
    nai: bool,
}

impl PartialEq for Interval {
    fn eq(&self, other: &Self) -> bool {
        if self.nai || other.nai {
            return self.nai == other.nai;
        }
        self.min == other.min && self.max == other.max
    }
}

impl Interval {
    pub const NAI: Interval = Interval {
        min: f32::NAN,
        max: f32::NAN,
        nai: true,
    };
    pub const INFINITE: Interval = Interval {
        min: f32::NEG_INFINITY,
        max: f32::INFINITY,
        nai: false,
    };

    /// `Interval.of`; panics on invalid bounds like vanilla throws.
    pub fn of(min: f32, max: f32) -> Interval {
        assert!(!(max < min), "Interval max ({max}) < min ({min})");
        assert!(
            !min.is_nan() && !max.is_nan(),
            "Interval bounds cannot include NaN [{min}; {max}]"
        );
        Interval { min, max, nai: false }
    }

    pub fn of_symmetric(range: f32) -> Interval {
        Self::of(-range, range)
    }

    pub fn of_exact(value: f32) -> Interval {
        Self::of(value, value)
    }

    pub fn is_nai(self) -> bool {
        self.nai
    }

    pub fn min(self) -> f32 {
        self.min
    }

    pub fn max(self) -> f32 {
        self.max
    }

    pub fn contains(self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }

    pub fn encapsulating_all(intervals: &[Interval]) -> Interval {
        assert!(!intervals.is_empty(), "At least one interval required");
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for i in intervals {
            if !i.nai {
                min = min_f32(i.min, min);
                max = max_f32(i.max, max);
            }
        }
        if max < min { Self::NAI } else { Self::of(min, max) }
    }

    pub fn encapsulating(first: f32, second: f32) -> Interval {
        if first.is_nan() && second.is_nan() {
            Self::NAI
        } else if first.is_nan() {
            Self::of_exact(second)
        } else if second.is_nan() {
            Self::of_exact(first)
        } else {
            Self::of(min_f32(first, second), max_f32(first, second))
        }
    }

    fn encapsulating_value(first: Interval, second: f32) -> Interval {
        if second.is_nan() {
            first
        } else if first.nai {
            Self::of_exact(second)
        } else {
            Self::of(min_f32(first.min, second), max_f32(first.max, second))
        }
    }

    pub fn add(left: Interval, right: Interval) -> Interval {
        let min = left.min + right.min;
        let max = left.max + right.max;
        if !min.is_nan() && !max.is_nan() {
            Self::of(min, max)
        } else {
            Self::NAI
        }
    }

    pub fn sub(left: Interval, right: Interval) -> Interval {
        let min = left.min - right.max;
        let max = left.max - right.min;
        if !min.is_nan() && !max.is_nan() {
            Self::of(min, max)
        } else {
            Self::NAI
        }
    }

    fn mul_bound(left: f32, right: f32) -> f32 {
        if left != 0.0 && right != 0.0 {
            left * right
        } else {
            0.0
        }
    }

    pub fn mul(left: Interval, right: Interval) -> Interval {
        if left.nai || right.nai {
            return Self::NAI;
        }
        let a = Self::mul_bound(left.min, right.min);
        let b = Self::mul_bound(left.min, right.max);
        let c = Self::mul_bound(left.max, right.min);
        let d = Self::mul_bound(left.max, right.max);
        Self::of(
            min_f32(min_f32(a, b), min_f32(c, d)),
            max_f32(max_f32(a, b), max_f32(c, d)),
        )
    }

    pub fn reciprocal(input: Interval) -> Interval {
        if input.nai || (input.min == 0.0 && input.max == 0.0) {
            return Self::NAI;
        }
        if !input.contains(0.0) {
            Self::of(1.0 / input.max, 1.0 / input.min)
        } else if input.max == 0.0 {
            Self::of(f32::NEG_INFINITY, 1.0 / input.min)
        } else if input.min == 0.0 {
            Self::of(1.0 / input.max, f32::INFINITY)
        } else {
            Self::INFINITE
        }
    }

    pub fn div(left: Interval, right: Interval) -> Interval {
        Self::mul(left, Self::reciprocal(right))
    }

    pub fn min_of(left: Interval, right: Interval) -> Interval {
        if left.nai || right.nai {
            Self::NAI
        } else {
            Self::of(min_f32(left.min, right.min), min_f32(left.max, right.max))
        }
    }

    pub fn max_of(left: Interval, right: Interval) -> Interval {
        if left.nai || right.nai {
            Self::NAI
        } else {
            Self::of(max_f32(left.min, right.min), max_f32(left.max, right.max))
        }
    }

    pub fn clamp(input: Interval, min: f32, max: f32) -> Interval {
        assert!(!(min > max), "min ({min}) > max ({max})");
        if input.nai {
            Self::NAI
        } else if input.min >= max {
            Self::of(max, max)
        } else if input.max <= min {
            Self::of(min, min)
        } else {
            Self::of(max_f32(input.min, min), min_f32(input.max, max))
        }
    }

    pub fn abs(input: Interval) -> Interval {
        if input.nai {
            return Self::NAI;
        }
        let max = max_f32(input.min.abs(), input.max.abs());
        if input.contains(0.0) {
            Self::of(0.0, max)
        } else {
            Self::of(min_f32(input.min.abs(), input.max.abs()), max)
        }
    }

    pub fn square(input: Interval) -> Interval {
        if input.nai {
            return Self::NAI;
        }
        let max = max_f32(square_f32(input.min), square_f32(input.max));
        if input.contains(0.0) {
            Self::of(0.0, max)
        } else {
            Self::of(min_f32(square_f32(input.min), square_f32(input.max)), max)
        }
    }

    fn jpow(base: f32, exp: f32) -> f32 {
        (base as f64).powf(exp as f64) as f32
    }

    pub fn pow(base: Interval, exponent: Interval) -> Interval {
        if base.nai || exponent.nai {
            return Self::NAI;
        }
        if base.min == base.max {
            return Self::pow_scalar(base.min, exponent);
        }
        let mut result = Self::encapsulating_all(&[
            Self::pow_scalar(base.min, exponent),
            Self::pow_scalar(base.max, exponent),
        ]);
        if base.contains(0.0) {
            if base.max > 0.0 {
                result = Self::encapsulating_all(&[result, Self::pow_scalar(0.0, exponent)]);
            }
            if base.min < 0.0 {
                result = Self::encapsulating_all(&[result, Self::pow_scalar(-0.0, exponent)]);
            }
        }
        result
    }

    fn pow_scalar(base: f32, exponent: Interval) -> Interval {
        if base.is_nan() || exponent.nai {
            return Self::NAI;
        }
        if exponent.min == exponent.max {
            let v = Self::jpow(base, exponent.min);
            return if v.is_nan() { Self::NAI } else { Self::of_exact(v) };
        }
        if base == 0.0 {
            return Self::mul(
                Self::pow_zero_base(exponent),
                Self::of_exact(1.0f32.copysign(base)),
            );
        }
        if base == 1.0 {
            return Self::of_exact(1.0);
        }
        if base > 0.0 {
            Self::pow_positive_base(base, exponent)
        } else {
            Self::pow_negative_base(base, exponent)
        }
    }

    fn pow_positive_base(base: f32, exponent: Interval) -> Interval {
        if exponent.min.is_finite() && exponent.max.is_finite() {
            Self::encapsulating(Self::jpow(base, exponent.min), Self::jpow(base, exponent.max))
        } else {
            Self::pow_infinite_exponent(base, exponent)
        }
    }

    fn pow_zero_base(exponent: Interval) -> Interval {
        if exponent.contains(0.0) {
            if exponent.max == 0.0 {
                Self::of(1.0, f32::INFINITY)
            } else if exponent.min == 0.0 {
                Self::of(0.0, 1.0)
            } else {
                Self::of(0.0, f32::INFINITY)
            }
        } else if exponent.max < 0.0 {
            Self::of_exact(f32::INFINITY)
        } else {
            Self::of_exact(0.0)
        }
    }

    fn pow_infinite_exponent(base: f32, exponent: Interval) -> Interval {
        if exponent.min.is_infinite() && exponent.max.is_infinite() {
            Self::of(0.0, f32::INFINITY)
        } else if exponent.min.is_infinite() {
            if base < 1.0 {
                Self::of(Self::jpow(base, exponent.max), f32::INFINITY)
            } else {
                Self::of(0.0, Self::jpow(base, exponent.max))
            }
        } else if base < 1.0 {
            Self::of(0.0, Self::jpow(base, exponent.min))
        } else {
            Self::of(Self::jpow(base, exponent.min), f32::INFINITY)
        }
    }

    fn pow_negative_base(base: f32, exponent: Interval) -> Interval {
        let min_int = (exponent.min as f64).ceil() as f32;
        let max_int = (exponent.max as f64).floor() as f32;
        if max_int < min_int {
            return Self::NAI;
        }
        let to_min = Self::jpow(base, min_int);
        let to_max = Self::jpow(base, max_int);
        let mut result = Self::encapsulating(to_min, to_max);
        if min_int.is_infinite() {
            result = Self::encapsulating_value(result, -to_min);
        } else if min_int + 1.0 < max_int {
            result = Self::encapsulating_value(result, Self::jpow(base, min_int + 1.0));
        }
        if max_int.is_infinite() {
            result = Self::encapsulating_value(result, -to_max);
        } else if max_int - 1.0 > min_int {
            result = Self::encapsulating_value(result, Self::jpow(base, max_int - 1.0));
        }
        result
    }

    pub fn log(input: Interval) -> Interval {
        if input.max < 0.0 {
            return Self::NAI;
        }
        let clipped = Self::max_of(input, Self::of_exact(0.0));
        Self::map_monotonic(clipped, |x| (x as f64).ln() as f32)
    }

    pub fn map_monotonic(input: Interval, op: impl Fn(f32) -> f32) -> Interval {
        if input.nai {
            return Self::NAI;
        }
        let a = op(input.min);
        let b = op(input.max);
        assert!(
            !a.is_nan() && !b.is_nan(),
            "Monotonic operator should not produce NaN"
        );
        Self::of(min_f32(a, b), max_f32(a, b))
    }

    pub fn lerp(alpha: Interval, first: Interval, second: Interval) -> Interval {
        if alpha.nai || first.nai || second.nai {
            return Self::NAI;
        }
        Self::encapsulating_all(&[
            Self::lerp_scalar(alpha, first.min, second.min),
            Self::lerp_scalar(alpha, first.max, second.min),
            Self::lerp_scalar(alpha, first.min, second.max),
            Self::lerp_scalar(alpha, first.max, second.max),
        ])
    }

    pub fn lerp_scalar(alpha: Interval, first: f32, second: f32) -> Interval {
        if alpha.nai || first.is_nan() || second.is_nan() {
            return Self::NAI;
        }
        if first.is_finite() && second.is_finite() {
            Self::encapsulating(
                first + Self::mul_bound(alpha.min, second - first),
                first + Self::mul_bound(alpha.max, second - first),
            )
        } else if first == second {
            Self::of_exact(first)
        } else {
            let a = Self::lerp_infinite_bound(alpha.min, first, second);
            let b = Self::lerp_infinite_bound(alpha.max, first, second);
            if !a.is_nan() && !b.is_nan() {
                Self::encapsulating(a, b)
            } else {
                Self::NAI
            }
        }
    }

    fn lerp_infinite_bound(alpha: f32, first: f32, second: f32) -> f32 {
        let a = Self::mul_bound(1.0 - alpha, first);
        let b = Self::mul_bound(alpha, second);
        if !a.is_infinite() || !b.is_infinite() {
            a + b
        } else if alpha <= 0.0 {
            if second > first {
                f32::NEG_INFINITY
            } else {
                f32::INFINITY
            }
        } else if alpha >= 1.0 {
            if second > first {
                f32::INFINITY
            } else {
                f32::NEG_INFINITY
            }
        } else {
            f32::NAN
        }
    }

    pub fn sign(input: Interval) -> Interval {
        if input.nai {
            Self::NAI
        } else if input.min == input.max {
            Self::of_exact(signum_f32(input.min))
        } else if input.contains(0.0) {
            if input.min == 0.0 {
                Self::of(0.0, 1.0)
            } else if input.max == 0.0 {
                Self::of(-1.0, 0.0)
            } else {
                Self::of(-1.0, 1.0)
            }
        } else {
            Self::of_exact(if input.min > 0.0 { 1.0 } else { -1.0 })
        }
    }
}
