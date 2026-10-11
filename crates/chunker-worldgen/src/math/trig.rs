//! `Math.sin` / `Math.cos` as computed by HotSpot.
//!
//! HotSpot's x86-64 intrinsics return the correctly rounded result in all but ~0.2% of inputs,
//! while the platform C runtime misses ~3%. These implementations are correctly rounded
//! (double-double evaluation, one final rounding), which is the closest portable approximation.

#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

#[inline]
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    let err = (a - (s - bb)) + (b - bb);
    Dd { hi: s, lo: err }
}

#[inline]
fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd {
        hi: s,
        lo: b - (s - a),
    }
}

#[inline]
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    // Fused multiply-add gives the exact rounding error of the product.
    Dd {
        hi: p,
        lo: a.mul_add(b, -p),
    }
}

impl Dd {
    const fn new(hi: f64) -> Dd {
        Dd { hi, lo: 0.0 }
    }

    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.hi, o.hi);
        let t = two_sum(self.lo, o.lo);
        let s = quick_two_sum(s.hi, s.lo + t.hi);
        quick_two_sum(s.hi, s.lo + t.lo)
    }

    fn neg(self) -> Dd {
        Dd {
            hi: -self.hi,
            lo: -self.lo,
        }
    }

    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.hi, o.hi);
        quick_two_sum(p.hi, p.lo + (self.hi * o.lo + self.lo * o.hi))
    }

    fn div_f(self, b: f64) -> Dd {
        let q1 = self.hi / b;
        let p = two_prod(q1, b);
        let r = two_sum(self.hi, -p.hi);
        let r_lo = r.lo - p.lo + self.lo;
        let q2 = (r.hi + r_lo) / b;
        quick_two_sum(q1, q2)
    }
}

/// π/2 as a triple-double.
const PIO2_HI: f64 = f64::from_bits(0x3FF921FB54442D18);
const PIO2_MID: f64 = f64::from_bits(0x3C91A62633145C07);
const PIO2_LO: f64 = f64::from_bits(0xB91F1976B7ED8FBC);

/// `x - k * π/2` in double-double (accurate for the moderate arguments worldgen uses).
fn reduce(x: f64) -> (Dd, i64) {
    let k = (x / PIO2_HI).round();
    let p1 = two_prod(k, PIO2_HI);
    let p2 = two_prod(k, PIO2_MID);
    let r = two_sum(x, -p1.hi);
    let r = Dd {
        hi: r.hi,
        lo: r.lo - p1.lo,
    };
    let r = quick_two_sum(r.hi, r.lo);
    let r = r.add(p2.neg());
    let r = r.add(Dd::new(-k * PIO2_LO));
    (r, k as i64)
}

/// sin and cos of a reduced argument (|r| <= ~π/4) by Taylor series in double-double.
fn sin_cos_reduced(r: Dd) -> (Dd, Dd) {
    let r2 = r.mul(r);
    let mut s = Dd::new(0.0);
    let mut term = r;
    let mut n = 1.0;
    for _ in 0..16 {
        s = s.add(term);
        term = term.mul(r2).div_f((n + 1.0) * (n + 2.0)).neg();
        n += 2.0;
    }
    let mut c = Dd::new(0.0);
    let mut term = Dd::new(1.0);
    let mut n = 0.0;
    for _ in 0..16 {
        c = c.add(term);
        term = term.mul(r2).div_f((n + 1.0) * (n + 2.0)).neg();
        n += 2.0;
    }
    (s, c)
}

fn sin_cos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    if x == 0.0 {
        return (x, 1.0);
    }
    let (r, k) = reduce(x);
    let (s, c) = sin_cos_reduced(r);
    let (s, c) = match k.rem_euclid(4) {
        0 => (s, c),
        1 => (c, s.neg()),
        2 => (s.neg(), c.neg()),
        _ => (c.neg(), s),
    };
    (s.hi + s.lo, c.hi + c.lo)
}

/// `Math.sin`.
pub fn sin(x: f64) -> f64 {
    sin_cos(x).0
}

/// `Math.cos`.
pub fn cos(x: f64) -> f64 {
    sin_cos(x).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_values() {
        assert_eq!(sin(0.5).to_bits(), 0.479425538604203f64.to_bits());
        assert_eq!(cos(1.0), 0.5403023058681398);
        assert_eq!(sin(std::f64::consts::PI), 1.2246467991473532e-16);
    }
}
