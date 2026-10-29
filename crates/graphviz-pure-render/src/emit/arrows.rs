//! Arrowheads: port of `lib/common/arrows.c` (`arrow_gen` and the arrow type generators). The
//! flags they draw come from the layout (`graphviz_pure_core::arrows`).

use super::attrs::default_line_style;
use super::job::{EmitState, Emitter, Engine};
use graphviz_pure_core::Point;
use graphviz_pure_core::arrows::*;
use graphviz_pure_core::math;

const EPSILON: f64 = 0.0001;
const ARROW_LENGTH: f64 = 10.0;

/// `(type, lenfact)` in `Arrowtypes[]` order.
const TYPES: &[(u32, f64)] = &[
    (ARR_TYPE_NORM, 1.0),
    (ARR_TYPE_CROW, 1.0),
    (ARR_TYPE_TEE, 0.5),
    (ARR_TYPE_BOX, 1.0),
    (ARR_TYPE_DIAMOND, 1.2),
    (ARR_TYPE_DOT, 0.8),
    (ARR_TYPE_CURVE, 1.0),
    (ARR_TYPE_GAP, 0.5),
];

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn sub(a: Point, b: Point) -> Point {
    p(a.x - b.x, a.y - b.y)
}

fn add(a: Point, b: Point) -> Point {
    p(a.x + b.x, a.y + b.y)
}

fn scale(s: f64, a: Point) -> Point {
    p(a.x * s, a.y * s)
}

/// `miter_shape`.
fn miter_shape(base_left: Point, pt: Point, base_right: Point, penwidth: f64) -> [Point; 3] {
    if (base_left.x == pt.x && base_left.y == pt.y)
        || (base_right.x == pt.x && base_right.y == pt.y)
    {
        return [pt, pt, pt];
    }
    let dxa = pt.x - base_left.x;
    let dya = pt.y - base_left.y;
    let hypot_a = math::hypot(dxa, dya);
    let cos_alpha = dxa / hypot_a;
    let sin_alpha = dya / hypot_a;
    let alpha = if dya > 0.0 {
        math::acos(cos_alpha)
    } else {
        -math::acos(cos_alpha)
    };
    let p1 = p(
        // C (clang) fuses each `P ± penwidth / 2.0 * …` of `miter_shape` into one fma.
        math::fma(-(penwidth / 2.0), sin_alpha, pt.x),
        math::fma(penwidth / 2.0, cos_alpha, pt.y),
    );
    let dxb = base_right.x - pt.x;
    let dyb = base_right.y - pt.y;
    let hypot_b = math::hypot(dxb, dyb);
    let cos_beta = dxb / hypot_b;
    let beta = if dyb > 0.0 {
        math::acos(cos_beta)
    } else {
        -math::acos(cos_beta)
    };
    let beta_rev = beta - std::f64::consts::PI;
    let theta = beta_rev - alpha
        + if beta_rev - alpha <= -std::f64::consts::PI {
            2.0 * std::f64::consts::PI
        } else {
            0.0
        };
    let normalized_miter_length = 1.0 / math::sin(theta / 2.0);
    let sin_beta = dyb / hypot_b;
    let p2 = p(
        math::fma(penwidth / 2.0, -sin_beta, pt.x),
        math::fma(-(penwidth / 2.0), -cos_beta, pt.y),
    );
    if normalized_miter_length > 4.0 {
        let pbevel = p((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0);
        return [pbevel, p1, p2];
    }
    let l = penwidth / 2.0 / math::tan(theta / 2.0);
    let p3 = p(math::fma(l, cos_alpha, p1.x), math::fma(l, sin_alpha, p1.y));
    [p3, p1, p2]
}

fn angle_of(d: Point) -> f64 {
    let h = math::hypot(d.x, d.y);
    let c = d.x / h;
    if d.y > 0.0 {
        math::acos(c)
    } else {
        -math::acos(c)
    }
}

fn delta_from(shape_pt: Point, tip: Point, phi: f64, cos_phi: f64, sin_phi: f64) -> Point {
    let d = sub(shape_pt, tip);
    let hyp = math::hypot(d.x, d.y);
    let alpha = angle_of(d);
    let len = hyp * math::cos(alpha - phi);
    p(len * cos_phi, len * sin_phi)
}

fn normal0(mut pp: Point, u: Point, penwidth: f64, flag: u32, a: &mut [Point; 5]) -> Point {
    let mut arrowwidth = 0.35;
    if penwidth > 4.0 {
        arrowwidth *= penwidth / 4.0;
    }
    let v = p(-u.y * arrowwidth, u.x * arrowwidth);
    let mut q = p(pp.x + u.x, pp.y + u.y);
    let mut delta_base = p(0.0, 0.0);
    let origin = p(0.0, 0.0);
    let v_inv = p(-v.x, -v.y);
    let normal_left = if flag & ARR_MOD_RIGHT != 0 {
        origin
    } else {
        v_inv
    };
    let normal_right = if flag & ARR_MOD_LEFT != 0 { origin } else { v };
    let inv = flag & ARR_MOD_INV != 0;
    let base_left = if inv { normal_right } else { normal_left };
    let base_right = if inv { normal_left } else { normal_right };
    let tip = if inv { u } else { p(-u.x, -u.y) };
    let mut delta_tip = p(0.0, 0.0);
    if u.x != 0.0 || u.y != 0.0 {
        let h = math::hypot(tip.x, tip.y);
        let cos_phi = tip.x / h;
        let sin_phi = tip.y / h;
        let phi = if tip.y > 0.0 {
            math::acos(cos_phi)
        } else {
            -math::acos(cos_phi)
        };
        let shape = miter_shape(base_left, tip, base_right, penwidth);
        delta_tip = if flag & ARR_MOD_LEFT != 0 {
            delta_from(shape[1], tip, phi, cos_phi, sin_phi)
        } else if flag & ARR_MOD_RIGHT != 0 {
            delta_from(shape[2], tip, phi, cos_phi, sin_phi)
        } else {
            sub(shape[0], tip)
        };
        delta_base = p(penwidth / 2.0 * cos_phi, penwidth / 2.0 * sin_phi);
    }
    if inv {
        pp = add(pp, delta_base);
        q = add(q, delta_base);
        a[0] = pp;
        a[4] = pp;
        a[1] = p(pp.x - v.x, pp.y - v.y);
        a[2] = q;
        a[3] = p(pp.x + v.x, pp.y + v.y);
        q = add(q, delta_tip);
    } else {
        pp = sub(pp, delta_tip);
        q = sub(q, delta_tip);
        a[0] = q;
        a[4] = q;
        a[1] = p(q.x - v.x, q.y - v.y);
        a[2] = pp;
        a[3] = p(q.x + v.x, q.y + v.y);
        q = sub(q, delta_base);
    }
    q
}

fn crow0(
    mut pp: Point,
    u: Point,
    arrowsize: f64,
    penwidth: f64,
    flag: u32,
    a: &mut [Point; 9],
) -> Point {
    let inv = flag & ARR_MOD_INV != 0;
    let mut arrowwidth = 0.45;
    if penwidth > 4.0 * arrowsize && inv {
        arrowwidth *= penwidth / (4.0 * arrowsize);
    }
    let mut shaftwidth = 0.0;
    if penwidth > 1.0 && inv {
        shaftwidth = 0.05 * (penwidth - 1.0) / arrowsize;
    }
    let v = p(-u.y * arrowwidth, u.x * arrowwidth);
    let w = p(-u.y * shaftwidth, u.x * shaftwidth);
    let mut q = p(pp.x + u.x, pp.y + u.y);
    let m = p(pp.x + u.x * 0.5, pp.y + u.y * 0.5);
    let mut delta_base = p(0.0, 0.0);
    let origin = p(0.0, 0.0);
    let v_inv = p(-v.x, -v.y);
    let normal_left = if flag & ARR_MOD_RIGHT != 0 { origin } else { v };
    let normal_right = if flag & ARR_MOD_LEFT != 0 {
        origin
    } else {
        v_inv
    };
    let base_left = if inv { normal_right } else { normal_left };
    let base_right = if inv { normal_left } else { normal_right };
    let tip = if inv { p(-u.x, -u.y) } else { u };
    let mut delta_tip = p(0.0, 0.0);
    let left = flag & ARR_MOD_LEFT != 0;
    let right = flag & ARR_MOD_RIGHT != 0;
    if u.x != 0.0 || u.y != 0.0 {
        let h = math::hypot(tip.x, tip.y);
        let cos_phi = tip.x / h;
        let sin_phi = tip.y / h;
        let phi = if tip.y > 0.0 {
            math::acos(cos_phi)
        } else {
            -math::acos(cos_phi)
        };
        let shape = miter_shape(base_left, tip, base_right, penwidth);
        delta_tip = if (left && inv) || (right && !inv) {
            delta_from(shape[2], tip, phi, cos_phi, sin_phi)
        } else if (left && !inv) || (right && inv) {
            delta_from(shape[1], tip, phi, cos_phi, sin_phi)
        } else {
            sub(shape[0], tip)
        };
        if inv {
            delta_base = p(penwidth / 2.0 * cos_phi, penwidth / 2.0 * sin_phi);
        } else {
            let toe_base_left = add(sub(m, q), w);
            let toe_base_right = origin;
            let toe_p = sub(v, u);
            let ts = miter_shape(toe_base_left, toe_p, toe_base_right, penwidth);
            let d = sub(ts[1], toe_p);
            let hyp = math::hypot(d.x, d.y);
            let alpha = angle_of(d);
            let len = -hyp * math::cos(alpha - phi);
            delta_base = p(len * cos_phi, len * sin_phi);
        }
    }
    if inv {
        pp = sub(pp, delta_tip);
        q = sub(q, delta_tip);
        a[0] = pp;
        a[8] = pp;
        a[1] = p(q.x - v.x, q.y - v.y);
        a[2] = p(m.x - w.x, m.y - w.y);
        a[3] = p(q.x - w.x, q.y - w.y);
        a[4] = q;
        a[5] = p(q.x + w.x, q.y + w.y);
        a[6] = p(m.x + w.x, m.y + w.y);
        a[7] = p(q.x + v.x, q.y + v.y);
        q = sub(q, delta_base);
    } else {
        pp = add(pp, delta_base);
        q = add(q, delta_base);
        a[0] = q;
        a[8] = q;
        a[1] = p(pp.x - v.x, pp.y - v.y);
        a[2] = p(m.x - w.x, m.y - w.y);
        a[3] = p(pp.x + delta_base.x, pp.y + delta_base.y);
        a[4] = add(pp, delta_base);
        a[5] = p(pp.x + delta_base.x, pp.y + delta_base.y);
        a[6] = p(m.x + w.x, m.y + w.y);
        a[7] = p(pp.x + v.x, pp.y + v.y);
        q = add(q, delta_tip);
    }
    q
}

fn diamond0(mut pp: Point, u: Point, penwidth: f64, flag: u32, a: &mut [Point; 5]) -> Point {
    let v = p(-u.y / 3.0, u.x / 3.0);
    let mut r = p(pp.x + u.x / 2.0, pp.y + u.y / 2.0);
    let mut q = p(pp.x + u.x, pp.y + u.y);
    let origin = p(0.0, 0.0);
    let unmod_left = sub(scale(-0.5, u), v);
    let unmod_right = add(scale(-0.5, u), v);
    let base_left = if flag & ARR_MOD_RIGHT != 0 {
        origin
    } else {
        unmod_left
    };
    let base_right = if flag & ARR_MOD_LEFT != 0 {
        origin
    } else {
        unmod_right
    };
    let tip = scale(-1.0, u);
    let shape = miter_shape(base_left, tip, base_right, penwidth);
    let delta = sub(shape[0], tip);
    pp = sub(pp, delta);
    r = sub(r, delta);
    q = sub(q, delta);
    a[0] = q;
    a[4] = q;
    a[1] = p(r.x + v.x, r.y + v.y);
    a[2] = pp;
    a[3] = p(r.x - v.x, r.y - v.y);
    sub(q, delta)
}

fn back_delta(u: Point, penwidth: f64) -> Point {
    let tip = p(-u.x, -u.y);
    let h = math::hypot(tip.x, tip.y);
    p(penwidth / 2.0 * (tip.x / h), penwidth / 2.0 * (tip.y / h))
}

/// `Bezier` in `lib/common/utils.c`: evaluates at `t`, returning the left and right halves.
pub(crate) fn bezier_split(v: &[Point; 4], t: f64) -> (Point, [Point; 4], [Point; 4]) {
    let mut vt = [[Point::default(); 4]; 4];
    vt[0] = *v;
    for i in 1..=3 {
        for j in 0..=(3 - i) {
            // C (clang) fuses `(1.0 - t) * a + t * b` in `Bezier` (lib/common/utils.c).
            vt[i][j] = p(
                math::fma(1.0 - t, vt[i - 1][j].x, t * vt[i - 1][j + 1].x),
                math::fma(1.0 - t, vt[i - 1][j].y, t * vt[i - 1][j + 1].y),
            );
        }
    }
    let left = [vt[0][0], vt[1][0], vt[2][0], vt[3][0]];
    let right = [vt[3][0], vt[2][1], vt[1][2], vt[0][3]];
    (vt[3][0], left, right)
}

impl<E: Engine> Emitter<'_, E> {
    fn arrow_gen_type(
        &mut self,
        pp: Point,
        u: Point,
        arrowsize: f64,
        penwidth: f64,
        flag: u32,
    ) -> Point {
        let f = flag & ((1 << BITS_PER_ARROW_TYPE) - 1);
        let Some(&(_, lenfact)) = TYPES.iter().find(|t| t.0 == f) else {
            return pp;
        };
        let u = p(u.x * lenfact * arrowsize, u.y * lenfact * arrowsize);
        let filled = i32::from(flag & ARR_MOD_OPEN == 0);
        match f {
            ARR_TYPE_NORM => {
                let mut a = [Point::default(); 5];
                let q = normal0(pp, u, penwidth, flag, &mut a);
                if flag & ARR_MOD_LEFT != 0 {
                    self.polygon(&a[0..3], filled);
                } else if flag & ARR_MOD_RIGHT != 0 {
                    self.polygon(&a[2..5], filled);
                } else {
                    self.polygon(&a[1..4], filled);
                }
                q
            }
            ARR_TYPE_CROW => {
                let mut a = [Point::default(); 9];
                let q = crow0(pp, u, arrowsize, penwidth, flag, &mut a);
                if flag & ARR_MOD_LEFT != 0 {
                    self.polygon(&a[0..5], 1);
                } else if flag & ARR_MOD_RIGHT != 0 {
                    self.polygon(&a[4..9], 1);
                } else {
                    self.polygon(&a[0..8], 1);
                }
                q
            }
            ARR_TYPE_GAP => {
                let q = add(pp, u);
                self.polyline(&[pp, q]);
                q
            }
            ARR_TYPE_TEE => {
                let v = p(-u.y, u.x);
                let mut pp = pp;
                let mut q = add(pp, u);
                // C (clang) fuses `p + u * 0.2`, `p + u * 0.6` and `penwidth / 2 - 0.2 * length`.
                let mut m = p(math::fma(u.x, 0.2, pp.x), math::fma(u.y, 0.2, pp.y));
                let mut n = p(math::fma(u.x, 0.6, pp.x), math::fma(u.y, 0.6, pp.y));
                let length = math::hypot(u.x, u.y);
                let ext = math::fma(-0.2, length, penwidth / 2.0);
                if length > 0.0 && ext > 0.0 {
                    let tip = p(-u.x, -u.y);
                    let h = math::hypot(tip.x, tip.y);
                    let delta = p(ext * (tip.x / h), ext * (tip.y / h));
                    pp = sub(pp, delta);
                    m = sub(m, delta);
                    n = sub(n, delta);
                    q = sub(q, delta);
                }
                let mut a = [add(m, v), sub(m, v), sub(n, v), add(n, v)];
                if flag & ARR_MOD_LEFT != 0 {
                    a[0] = m;
                    a[3] = n;
                } else if flag & ARR_MOD_RIGHT != 0 {
                    a[1] = m;
                    a[2] = n;
                }
                self.polygon(&a, 1);
                self.polyline(&[pp, q]);
                q
            }
            ARR_TYPE_BOX => {
                let v = p(-u.y * 0.4, u.x * 0.4);
                let mut pp = pp;
                // C (clang) fuses `p + u * 0.8`.
                let mut m = p(math::fma(u.x, 0.8, pp.x), math::fma(u.y, 0.8, pp.y));
                let mut q = add(pp, u);
                if u.x != 0.0 || u.y != 0.0 {
                    let d = back_delta(u, penwidth);
                    pp = sub(pp, d);
                    m = sub(m, d);
                    q = sub(q, d);
                }
                let mut a = [add(pp, v), sub(pp, v), sub(m, v), add(m, v)];
                if flag & ARR_MOD_LEFT != 0 {
                    a[0] = pp;
                    a[3] = m;
                } else if flag & ARR_MOD_RIGHT != 0 {
                    a[1] = pp;
                    a[2] = m;
                }
                self.polygon(&a, filled);
                self.polyline(&[m, q]);
                q
            }
            ARR_TYPE_DIAMOND => {
                let mut a = [Point::default(); 5];
                let q = diamond0(pp, u, penwidth, flag, &mut a);
                if flag & ARR_MOD_LEFT != 0 {
                    self.polygon(&a[2..5], filled);
                } else if flag & ARR_MOD_RIGHT != 0 {
                    self.polygon(&a[0..3], filled);
                } else {
                    self.polygon(&a[0..4], filled);
                }
                q
            }
            ARR_TYPE_DOT => {
                let r = math::hypot(u.x, u.y) / 2.0;
                let mut pp = pp;
                let mut delta = p(0.0, 0.0);
                if u.x != 0.0 || u.y != 0.0 {
                    delta = back_delta(u, penwidth);
                    pp = sub(pp, delta);
                }
                let af = [
                    p(pp.x + u.x / 2.0 - r, pp.y + u.y / 2.0 - r),
                    p(pp.x + u.x / 2.0 + r, pp.y + u.y / 2.0 + r),
                ];
                self.ellipse(af, filled);
                sub(add(pp, u), delta)
            }
            ARR_TYPE_CURVE => {
                let arrowwidth = if penwidth > 4.0 {
                    0.5 * penwidth / 4.0
                } else {
                    0.5
                };
                let a0 = pp;
                let mut pp = pp;
                if flag & ARR_MOD_INV == 0 && (u.x != 0.0 || u.y != 0.0) {
                    pp = sub(pp, back_delta(u, penwidth));
                }
                let q = add(pp, u);
                let v = p(-u.y * arrowwidth, u.x * arrowwidth);
                let w = p(v.y, -v.x);
                let mut af = [Point::default(); 4];
                af[0] = p(pp.x + v.x + w.x, pp.y + v.y + w.y);
                af[3] = p(pp.x - v.x + w.x, pp.y - v.y + w.y);
                // C (clang) fuses `p.x ± 0.95 * v.x`.
                if flag & ARR_MOD_INV != 0 {
                    af[1] = p(
                        math::fma(0.95, v.x, pp.x) + w.x + w.x * 4.0 / 3.0,
                        af[0].y + w.y * 4.0 / 3.0,
                    );
                    af[2] = p(
                        math::fma(-0.95, v.x, pp.x) + w.x + w.x * 4.0 / 3.0,
                        af[3].y + w.y * 4.0 / 3.0,
                    );
                } else {
                    af[1] = p(
                        math::fma(0.95, v.x, pp.x) + w.x - w.x * 4.0 / 3.0,
                        af[0].y - w.y * 4.0 / 3.0,
                    );
                    af[2] = p(
                        math::fma(-0.95, v.x, pp.x) + w.x - w.x * 4.0 / 3.0,
                        af[3].y - w.y * 4.0 / 3.0,
                    );
                }
                self.polyline(&[a0, q]);
                if flag & ARR_MOD_LEFT != 0 {
                    af = bezier_split(&af, 0.5).2;
                } else if flag & ARR_MOD_RIGHT != 0 {
                    af = bezier_split(&af, 0.5).1;
                }
                self.bezier(&af, 0);
                q
            }
            _ => pp,
        }
    }

    /// `arrow_gen`.
    pub(crate) fn arrow_gen(
        &mut self,
        emit_state: EmitState,
        mut pp: Point,
        u: Point,
        arrowsize: f64,
        penwidth: f64,
        flag: u32,
    ) {
        let old = self.job.obj().emit_state;
        self.job.obj_mut().emit_state = emit_state;
        self.set_style(&default_line_style());
        self.set_penwidth(penwidth);
        let mut u = sub(u, pp);
        let s = ARROW_LENGTH / (math::hypot(u.x, u.y) + EPSILON);
        u.x += if u.x >= 0.0 { EPSILON } else { -EPSILON };
        u.y += if u.y >= 0.0 { EPSILON } else { -EPSILON };
        u.x *= s;
        u.y *= s;
        for i in 0..NUMB_OF_ARROW_HEADS {
            let f = (flag >> (i * BITS_PER_ARROW)) & ((1 << BITS_PER_ARROW) - 1);
            if f == ARR_TYPE_NONE {
                break;
            }
            pp = self.arrow_gen_type(pp, u, arrowsize, penwidth, f);
        }
        self.job.obj_mut().emit_state = old;
    }
}
