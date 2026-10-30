//! Exact-sign geometric predicates, standing in for Shewchuk's
//! `orient2d`/`incircle` (`predicates.c`) that GTS uses.
//!
//! Each predicate first evaluates the determinant in floating point with
//! Shewchuk's static error bound; when the result is too close to zero to
//! trust, it is recomputed exactly with integer arithmetic on the binary
//! expansions of the inputs. Only the sign of the result is meaningful.

use graphviz_pure_core::math;
use std::cmp::Ordering;

// Shewchuk's `(3 + 16ε)ε` and `(10 + 96ε)ε` (ε = 2^-53) as GTS's generated
// `predicates_init.h` stores them: printed with `%.16g`, so each is one ulp
// above the exact product.
const CCW_ERRBOUND_A: f64 = 3.330669073875472e-16;
const ICC_ERRBOUND_A: f64 = 1.110223024625158e-15;

/// Sign-magnitude big integer (little-endian 32-bit limbs).
#[derive(Clone, Debug, Default)]
struct Big {
    neg: bool,
    mag: Vec<u32>,
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for i in 0..a.len().max(b.len()) {
        let s = u64::from(*a.get(i).unwrap_or(&0)) + u64::from(*b.get(i).unwrap_or(&0)) + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b` for `|a| >= |b|`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let mut d = i64::from(a[i]) - i64::from(*b.get(i).unwrap_or(&0)) - borrow;
        borrow = 0;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        }
        out.push(d as u32);
    }
    trim(&mut out);
    out
}

impl Big {
    fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    fn signum(&self) -> f64 {
        if self.is_zero() {
            0.0
        } else if self.neg {
            -1.0
        } else {
            1.0
        }
    }

    fn add(&self, o: &Big) -> Big {
        if self.neg == o.neg {
            return Big {
                neg: self.neg,
                mag: add_mag(&self.mag, &o.mag),
            };
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Equal => Big::default(),
            Ordering::Greater => Big {
                neg: self.neg,
                mag: sub_mag(&self.mag, &o.mag),
            },
            Ordering::Less => Big {
                neg: o.neg,
                mag: sub_mag(&o.mag, &self.mag),
            },
        }
    }

    fn neg(&self) -> Big {
        Big {
            neg: !self.neg && !self.is_zero(),
            mag: self.mag.clone(),
        }
    }

    fn sub(&self, o: &Big) -> Big {
        self.add(&o.neg())
    }

    fn mul(&self, o: &Big) -> Big {
        if self.is_zero() || o.is_zero() {
            return Big::default();
        }
        let mut out = vec![0u32; self.mag.len() + o.mag.len()];
        for (i, &a) in self.mag.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in o.mag.iter().enumerate() {
                let t = u64::from(a) * u64::from(b) + u64::from(out[i + j]) + carry;
                out[i + j] = t as u32;
                carry = t >> 32;
            }
            let mut k = i + o.mag.len();
            while carry != 0 {
                let t = u64::from(out[k]) + carry;
                out[k] = t as u32;
                carry = t >> 32;
                k += 1;
            }
        }
        trim(&mut out);
        Big {
            neg: self.neg != o.neg,
            mag: out,
        }
    }
}

/// `(mantissa, exponent)` with `v = mantissa * 2^exponent` exactly.
fn decompose(v: f64) -> (i64, i32) {
    if v == 0.0 {
        return (0, 0);
    }
    let bits = v.to_bits();
    let sign = if bits >> 63 != 0 { -1 } else { 1 };
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = (bits & ((1u64 << 52) - 1)) as i64;
    let (m, e) = if exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1i64 << 52), exp - 1075)
    };
    (sign * m, e)
}

/// The inputs as exact integers sharing one power-of-two scale.
fn to_bigs(vals: &[f64]) -> Vec<Big> {
    let parts: Vec<(i64, i32)> = vals.iter().map(|&v| decompose(v)).collect();
    let emin = parts
        .iter()
        .filter(|p| p.0 != 0)
        .map(|p| p.1)
        .min()
        .unwrap_or(0);
    parts
        .iter()
        .map(|&(m, e)| {
            if m == 0 {
                return Big::default();
            }
            let shift = (e - emin) as usize;
            let mag = m.unsigned_abs();
            let mut limbs = vec![0u32; shift / 32];
            let s = shift % 32;
            let wide = u128::from(mag) << s;
            limbs.push(wide as u32);
            limbs.push((wide >> 32) as u32);
            limbs.push((wide >> 64) as u32);
            trim(&mut limbs);
            Big {
                neg: m < 0,
                mag: limbs,
            }
        })
        .collect()
}

/// `orient2d(pa, pb, pc)`: positive if `a, b, c` turn counterclockwise.
pub fn orient2d(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let detleft = (a.0 - c.0) * (b.1 - c.1);
    let detright = (a.1 - c.1) * (b.0 - c.0);
    let det = detleft - detright;
    let errbound = CCW_ERRBOUND_A * (detleft.abs() + detright.abs());
    if det.abs() > errbound {
        return det;
    }
    let v = to_bigs(&[a.0, a.1, b.0, b.1, c.0, c.1]);
    let (acx, acy) = (v[0].sub(&v[4]), v[1].sub(&v[5]));
    let (bcx, bcy) = (v[2].sub(&v[4]), v[3].sub(&v[5]));
    acx.mul(&bcy).sub(&acy.mul(&bcx)).signum()
}

/// `incircle(pa, pb, pc, pd)`: positive if `d` lies inside the circle
/// through `a, b, c` when they are counterclockwise.
pub fn incircle(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> f64 {
    let (adx, ady) = (a.0 - d.0, a.1 - d.1);
    let (bdx, bdy) = (b.0 - d.0, b.1 - d.1);
    let (cdx, cdy) = (c.0 - d.0, c.1 - d.1);
    // C (clang) fuses the lifts, `det` and `permanent` (the `fmadd`s of
    // `_incircle` in libgts): `x*x + y*y` = fma(x, x, y*y), and
    // `a*b + c*d + e*f` = fma(e, f, fma(a, b, c*d)). This gives C's `det` bit for bit,
    // but callers use only the sign, which is exact either way, so no output depends on it.
    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let alift = math::fma(adx, adx, ady * ady);
    let cdxady = cdx * ady;
    let adxcdy = adx * cdy;
    let blift = math::fma(bdx, bdx, bdy * bdy);
    let adxbdy = adx * bdy;
    let bdxady = bdx * ady;
    let clift = math::fma(cdx, cdx, cdy * cdy);
    let det = math::fma(
        clift,
        adxbdy - bdxady,
        math::fma(alift, bdxcdy - cdxbdy, blift * (cdxady - adxcdy)),
    );
    let permanent = math::fma(
        adxbdy.abs() + bdxady.abs(),
        clift,
        math::fma(
            bdxcdy.abs() + cdxbdy.abs(),
            alift,
            (cdxady.abs() + adxcdy.abs()) * blift,
        ),
    );
    if det.abs() > ICC_ERRBOUND_A * permanent {
        return det;
    }
    let v = to_bigs(&[a.0, a.1, b.0, b.1, c.0, c.1, d.0, d.1]);
    let (adx, ady) = (v[0].sub(&v[6]), v[1].sub(&v[7]));
    let (bdx, bdy) = (v[2].sub(&v[6]), v[3].sub(&v[7]));
    let (cdx, cdy) = (v[4].sub(&v[6]), v[5].sub(&v[7]));
    let alift = adx.mul(&adx).add(&ady.mul(&ady));
    let blift = bdx.mul(&bdx).add(&bdy.mul(&bdy));
    let clift = cdx.mul(&cdx).add(&cdy.mul(&cdy));
    let t1 = alift.mul(&bdx.mul(&cdy).sub(&cdx.mul(&bdy)));
    let t2 = blift.mul(&cdx.mul(&ady).sub(&adx.mul(&cdy)));
    let t3 = clift.mul(&adx.mul(&bdy).sub(&bdx.mul(&ady)));
    t1.add(&t2).add(&t3).signum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_signs_for_degenerate_inputs() {
        // collinear, and exactly co-circular
        assert_eq!(orient2d((0.0, 0.0), (1.0, 1.0), (2.0, 2.0)), 0.0);
        assert_eq!(
            incircle((1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)),
            0.0
        );
        // a nearly collinear triple where the naive determinant rounds to zero
        let a = (0.5, 0.5);
        let b = (12.0, 12.0);
        let c = (24.0, 24.000000000000004);
        assert!(orient2d(a, b, c) > 0.0);
        assert!(incircle((1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, 0.5)) > 0.0);
        assert!(incircle((1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -2.0)) < 0.0);
    }
}
