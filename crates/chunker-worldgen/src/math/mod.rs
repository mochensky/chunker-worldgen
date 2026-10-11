//! Java-exact math helpers (`java.lang.Math` and `net.minecraft.util.Mth`).
//!
//! Rust's float `min`/`max`/`signum`/`round` differ from Java for NaN, signed zero and halves,
//! so every port must use these helpers instead of the std methods.

mod interval;
pub mod trig;

pub use interval::Interval;

const NEG_ZERO_F32: u32 = 0x8000_0000;
const NEG_ZERO_F64: u64 = 0x8000_0000_0000_0000;

/// `Math.max(float, float)`.
#[inline(always)]
pub fn max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == NEG_ZERO_F32 {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.min(float, float)`.
#[inline(always)]
pub fn min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == NEG_ZERO_F32 {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.max(double, double)`.
#[inline(always)]
pub fn max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == NEG_ZERO_F64 {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.min(double, double)`.
#[inline(always)]
pub fn min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == NEG_ZERO_F64 {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.signum(float)`: zero and NaN are returned unchanged.
#[inline]
pub fn signum_f32(v: f32) -> f32 {
    if v == 0.0 || v.is_nan() {
        v
    } else {
        1.0f32.copysign(v)
    }
}

/// `Math.round(float)` -> int: `floor(v + 0.5)` evaluated exactly, saturating, NaN -> 0.
#[inline]
pub fn round_f32(v: f32) -> i32 {
    let x = v as f64 + 0.5;
    let i = x as i32;
    if x < i as f64 { i.saturating_sub(1) } else { i }
}

/// `Math.round(double)` -> long.
#[inline]
pub fn round_f64(v: f64) -> i64 {
    // floor(v + 0.5) must be computed without double rounding; mirror Java's bit algorithm.
    let bits = v.to_bits() as i64;
    let biased_exp = (bits & 0x7ff0_0000_0000_0000) >> 52;
    let shift = (51 + 1023) - biased_exp;
    if shift & -64 == 0 {
        let mut r = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
        if bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        v as i64
    }
}

// The Mth helpers are written with casts like the Java originals: besides matching their overflow
// behaviour exactly, this avoids a libm `floor` call per sample on baseline x86-64 (no SSE4.1).

/// `Mth.floor(double)`.
#[inline(always)]
pub fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 { i.wrapping_sub(1) } else { i }
}

/// `Mth.floor(float)`.
#[inline(always)]
pub fn floor_f32(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i.wrapping_sub(1) } else { i }
}

/// `Mth.lfloor(double)`.
#[inline(always)]
pub fn lfloor(v: f64) -> i64 {
    let i = v as i64;
    if v < i as f64 { i.wrapping_sub(1) } else { i }
}

/// `Mth.ceil(double)`.
#[inline(always)]
pub fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > i as f64 { i.wrapping_add(1) } else { i }
}

/// `Mth.abs(float)` / `Math.abs(float)`.
#[inline(always)]
pub fn abs_f32(v: f32) -> f32 {
    v.abs()
}

/// `Math.floorDiv(int, int)`.
#[inline(always)]
pub fn floor_div(a: i32, b: i32) -> i32 {
    let q = a.wrapping_div(b);
    if (a % b != 0) && ((a ^ b) < 0) { q - 1 } else { q }
}

/// `Math.floorMod(int, int)`.
#[inline(always)]
pub fn floor_mod(a: i32, b: i32) -> i32 {
    let m = a.wrapping_rem(b);
    if m != 0 && ((m ^ b) < 0) { m + b } else { m }
}

/// `Math.floorDiv(long, long)`.
#[inline(always)]
pub fn floor_div_i64(a: i64, b: i64) -> i64 {
    let q = a.wrapping_div(b);
    if (a % b != 0) && ((a ^ b) < 0) { q - 1 } else { q }
}

/// `Math.floorMod(long, long)`.
#[inline(always)]
pub fn floor_mod_i64(a: i64, b: i64) -> i64 {
    let m = a.wrapping_rem(b);
    if m != 0 && ((m ^ b) < 0) { m + b } else { m }
}

/// `Mth.positiveCeilDiv(int, int)`.
#[inline]
pub fn positive_ceil_div(input: i32, divisor: i32) -> i32 {
    -floor_div(-input, divisor)
}

/// `Mth.roundToward(int, int)`.
#[inline]
pub fn round_toward(input: i32, multiple: i32) -> i32 {
    positive_ceil_div(input, multiple) * multiple
}

/// `Mth.clamp(float, float, float)`: `value < min ? min : Math.min(value, max)`.
#[inline(always)]
pub fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min { min } else { min_f32(value, max) }
}

/// `Mth.clamp(double, double, double)`.
#[inline(always)]
pub fn clamp_f64(value: f64, min: f64, max: f64) -> f64 {
    if value < min { min } else { min_f64(value, max) }
}

/// `Mth.clamp(int, int, int)`.
#[inline(always)]
pub fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

/// `Mth.lerp(float, float, float)`: `p0 + alpha * (p1 - p0)`.
#[inline(always)]
pub fn lerp_f32(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}

/// `Mth.lerp(double, double, double)`.
#[inline(always)]
pub fn lerp_f64(alpha: f64, p0: f64, p1: f64) -> f64 {
    p0 + alpha * (p1 - p0)
}

#[inline(always)]
pub fn lerp2_f32(a1: f32, a2: f32, x00: f32, x10: f32, x01: f32, x11: f32) -> f32 {
    lerp_f32(a2, lerp_f32(a1, x00, x10), lerp_f32(a1, x01, x11))
}

#[inline(always)]
pub fn lerp3_f32(
    a1: f32,
    a2: f32,
    a3: f32,
    x000: f32,
    x100: f32,
    x010: f32,
    x110: f32,
    x001: f32,
    x101: f32,
    x011: f32,
    x111: f32,
) -> f32 {
    lerp_f32(
        a3,
        lerp2_f32(a1, a2, x000, x100, x010, x110),
        lerp2_f32(a1, a2, x001, x101, x011, x111),
    )
}

#[inline(always)]
pub fn lerp2_f64(a1: f64, a2: f64, x00: f64, x10: f64, x01: f64, x11: f64) -> f64 {
    lerp_f64(a2, lerp_f64(a1, x00, x10), lerp_f64(a1, x01, x11))
}

#[inline(always)]
pub fn lerp3_f64(
    a1: f64,
    a2: f64,
    a3: f64,
    x000: f64,
    x100: f64,
    x010: f64,
    x110: f64,
    x001: f64,
    x101: f64,
    x011: f64,
    x111: f64,
) -> f64 {
    lerp_f64(
        a3,
        lerp2_f64(a1, a2, x000, x100, x010, x110),
        lerp2_f64(a1, a2, x001, x101, x011, x111),
    )
}

/// `Mth.smoothstep(float)`.
#[inline(always)]
pub fn smoothstep(x: f32) -> f32 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// `Mth.smoothstepDerivative(float)`.
#[inline(always)]
pub fn smoothstep_derivative(x: f32) -> f32 {
    30.0 * x * x * (x - 1.0) * (x - 1.0)
}

/// `Mth.inverseLerp(double, double, double)`.
#[inline(always)]
pub fn inverse_lerp_f64(value: f64, min: f64, max: f64) -> f64 {
    (value - min) / (max - min)
}

/// `Mth.inverseLerp(float, float, float)`.
#[inline(always)]
pub fn inverse_lerp_f32(value: f32, min: f32, max: f32) -> f32 {
    (value - min) / (max - min)
}

/// `Mth.clampedLerp(double, double, double)`.
#[inline]
pub fn clamped_lerp_f64(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        lerp_f64(factor, min, max)
    }
}

/// `Mth.clampedLerp(float, float, float)`.
#[inline]
pub fn clamped_lerp_f32(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        lerp_f32(factor, min, max)
    }
}

/// `Mth.clampedMap(double, ...)`.
#[inline]
pub fn clamped_map_f64(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    clamped_lerp_f64(inverse_lerp_f64(value, from_min, from_max), to_min, to_max)
}

/// `Mth.clampedMap(float, ...)`.
#[inline]
pub fn clamped_map_f32(value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32) -> f32 {
    clamped_lerp_f32(inverse_lerp_f32(value, from_min, from_max), to_min, to_max)
}

/// `Mth.map(double, ...)`.
#[inline]
pub fn map_f64(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    lerp_f64(inverse_lerp_f64(value, from_min, from_max), to_min, to_max)
}

/// `Mth.map(float, ...)`.
#[inline]
pub fn map_f32(value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32) -> f32 {
    lerp_f32(inverse_lerp_f32(value, from_min, from_max), to_min, to_max)
}

/// `Mth.quantize(double, int)`.
#[inline]
pub fn quantize(value: f64, resolution: i32) -> i32 {
    floor(value / resolution as f64) * resolution
}

/// `Mth.getSeed(int, int, int)`.
#[inline]
pub fn get_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut seed = (x.wrapping_mul(3_129_871) as i64) ^ (z as i64).wrapping_mul(116_129_781) ^ (y as i64);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11));
    seed >> 16
}

/// `Mth.binarySearch(from, to, predicate)`: first index in `[from, to)` where `pred` is true.
#[inline]
pub fn binary_search(mut from: i32, to: i32, pred: impl Fn(i32) -> bool) -> i32 {
    let mut len = to - from;
    while len > 0 {
        let half = len / 2;
        let middle = from + half;
        if pred(middle) {
            len = half;
        } else {
            from = middle + 1;
            len -= half + 1;
        }
    }
    from
}

/// `Mth.square(float)`.
#[inline(always)]
pub fn square_f32(x: f32) -> f32 {
    x * x
}

/// `Mth.cube(float)`.
#[inline(always)]
pub fn cube_f32(x: f32) -> f32 {
    x * x * x
}

/// `Mth.length(float, float, float)` = `sqrt(x*x + y*y + z*z)` in float.
#[inline]
pub fn length3_f32(x: f32, y: f32, z: f32) -> f32 {
    ((x * x + y * y + z * z) as f64).sqrt() as f32
}

#[inline]
pub fn length2_f32(x: f32, z: f32) -> f32 {
    ((x * x + z * z) as f64).sqrt() as f32
}

/// `Mth.sqrt(float)` = `(float) Math.sqrt(x)`.
#[inline(always)]
pub fn sqrt_f32(x: f32) -> f32 {
    (x as f64).sqrt() as f32
}

/// Java `String.hashCode()` over UTF-16 code units.
pub fn java_string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// `Mth.murmurHash3Mixer(int)`.
#[inline]
pub fn murmur_hash3_mixer(mut hash: i32) -> i32 {
    hash ^= ((hash as u32) >> 16) as i32;
    hash = hash.wrapping_mul(-2_048_144_789);
    hash ^= ((hash as u32) >> 13) as i32;
    hash = hash.wrapping_mul(-1_028_477_387);
    hash ^ ((hash as u32) >> 16) as i32
}

/// `Mth.sin` lookup table: `(float) Math.sin(i / 10430.378350470453)`.
pub fn sin_table() -> &'static [f32; 65536] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Box<[f32; 65536]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([0f32; 65536]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64 / 10430.378350470453).sin() as f32;
        }
        t
    })
}

/// `Mth.sin(double)`.
#[inline]
pub fn mth_sin(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453) as i64 & 65535) as usize]
}

/// `Mth.cos(double)`.
#[inline]
pub fn mth_cos(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453 + 16384.0) as i64 & 65535) as usize]
}

/// `Mth.wrapDegrees(float)`.
pub fn wrap_degrees_f32(degrees: f32) -> f32 {
    let mut r = degrees % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_min_max_semantics() {
        assert_eq!(max_f32(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(max_f32(0.0, -0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(min_f32(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert!(max_f32(1.0, f32::NAN).is_nan());
        assert!(min_f32(f32::NAN, 1.0).is_nan());
    }

    #[test]
    fn rounding_and_division() {
        assert_eq!(round_f32(-2.5), -2);
        assert_eq!(round_f32(2.5), 3);
        assert_eq!(round_f32(f32::NAN), 0);
        assert_eq!(round_f64(-2.5), -2);
        assert_eq!(round_f64(0.49999999999999994), 0);
        assert_eq!(floor_div(-7, 2), -4);
        assert_eq!(floor_mod(-7, 2), 1);
        assert_eq!(floor_mod(7, -2), -1);
        assert_eq!(signum_f32(-0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(round_toward(17, 16), 32);
    }

    #[test]
    fn string_hash_matches_java() {
        assert_eq!(
            java_string_hash("minecraft:terrain"),
            "minecraft:terrain"
                .encode_utf16()
                .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
        );
        assert_eq!(java_string_hash("hello"), 99162322);
    }
}
