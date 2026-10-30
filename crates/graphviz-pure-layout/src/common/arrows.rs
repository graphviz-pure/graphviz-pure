//! Arrowhead lengths for edge clipping, ported from `lib/common/arrows.c` (`arrow_length`,
//! `arrowStartClip`, `arrowEndClip` and the penwidth-aware length helpers). `arrow_flags` is
//! `graphviz_pure_core::arrows`'.

use super::splines::bezier_clip;
use super::utils::late_double;
pub use graphviz_pure_core::arrows::arrow_flags;
use graphviz_pure_core::arrows::*;
use graphviz_pure_core::math;
use graphviz_pure_core::{AttrMap, Point};

const ARROW_LENGTH: f64 = 10.0;

struct Tri {
    p: [Point; 3],
}

fn miter_shape(base_left: Point, p: Point, base_right: Point, penwidth: f64) -> Tri {
    use std::f64::consts::PI;
    if (base_left.x == p.x && base_left.y == p.y) || (base_right.x == p.x && base_right.y == p.y) {
        return Tri { p: [p, p, p] };
    }
    let dxa = p.x - base_left.x;
    let dya = p.y - base_left.y;
    let hypota = math::hypot(dxa, dya);
    let cos_alpha = dxa / hypota;
    let sin_alpha = dya / hypota;
    let alpha = if dya > 0.0 {
        math::acos(cos_alpha)
    } else {
        -math::acos(cos_alpha)
    };
    let p1 = Point::new(
        // C (clang) fuses each `P ± penwidth / 2.0 * …` of `miter_shape` into one fma.
        math::fma(-(penwidth / 2.0), sin_alpha, p.x),
        math::fma(penwidth / 2.0, cos_alpha, p.y),
    );
    let dxb = base_right.x - p.x;
    let dyb = base_right.y - p.y;
    let hypotb = math::hypot(dxb, dyb);
    let cos_beta = dxb / hypotb;
    let beta = if dyb > 0.0 {
        math::acos(cos_beta)
    } else {
        -math::acos(cos_beta)
    };
    let beta_rev = beta - PI;
    let theta = beta_rev - alpha
        + if beta_rev - alpha <= -PI {
            2.0 * PI
        } else {
            0.0
        };
    let normalized_miter_length = 1.0 / math::sin(theta / 2.0);
    let sin_beta = dyb / hypotb;
    let p2 = Point::new(
        math::fma(penwidth / 2.0, -sin_beta, p.x),
        math::fma(-(penwidth / 2.0), -cos_beta, p.y),
    );
    if normalized_miter_length > 4.0 {
        let pb = Point::new((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0);
        return Tri { p: [pb, p1, p2] };
    }
    let l = penwidth / 2.0 / math::tan(theta / 2.0);
    let p3 = Point::new(math::fma(l, cos_alpha, p1.x), math::fma(l, sin_alpha, p1.y));
    Tri { p: [p3, p1, p2] }
}

fn side_delta(
    base_left: Point,
    p: Point,
    base_right: Point,
    penwidth: f64,
    idx: usize,
    cos_phi: f64,
    sin_phi: f64,
    phi: f64,
) -> Point {
    let t = miter_shape(base_left, p, base_right, penwidth);
    let q = t.p[idx];
    let dx = q.x - p.x;
    let dy = q.y - p.y;
    let h = math::hypot(dx, dy);
    let ca = dx / h;
    let alpha = if dy > 0.0 {
        math::acos(ca)
    } else {
        -math::acos(ca)
    };
    let len = h * math::cos(alpha - phi);
    Point::new(len * cos_phi, len * sin_phi)
}

fn arrow_type_normal0(
    mut p: Point,
    u: Point,
    penwidth: f64,
    flag: u32,
    a: &mut [Point; 5],
) -> Point {
    let mut arrowwidth = 0.35;
    if penwidth > 4.0 {
        arrowwidth *= penwidth / 4.0;
    }
    let v = Point::new(-u.y * arrowwidth, u.x * arrowwidth);
    let mut q = Point::new(p.x + u.x, p.y + u.y);
    let origin = Point::new(0.0, 0.0);
    let v_inv = Point::new(-v.x, -v.y);
    let normal_left = if flag & ARR_MOD_RIGHT != 0 {
        origin
    } else {
        v_inv
    };
    let normal_right = if flag & ARR_MOD_LEFT != 0 { origin } else { v };
    let inv = flag & ARR_MOD_INV != 0;
    let base_left = if inv { normal_right } else { normal_left };
    let base_right = if inv { normal_left } else { normal_right };
    let pp = if inv { u } else { Point::new(-u.x, -u.y) };
    let mut delta_tip = origin;
    let mut delta_base = origin;
    if u.x != 0.0 || u.y != 0.0 {
        let cos_phi = pp.x / math::hypot(pp.x, pp.y);
        let sin_phi = pp.y / math::hypot(pp.x, pp.y);
        let phi = if pp.y > 0.0 {
            math::acos(cos_phi)
        } else {
            -math::acos(cos_phi)
        };
        if flag & ARR_MOD_LEFT != 0 {
            delta_tip = side_delta(
                base_left, pp, base_right, penwidth, 1, cos_phi, sin_phi, phi,
            );
        } else if flag & ARR_MOD_RIGHT != 0 {
            delta_tip = side_delta(
                base_left, pp, base_right, penwidth, 2, cos_phi, sin_phi, phi,
            );
        } else {
            let t = miter_shape(base_left, pp, base_right, penwidth);
            delta_tip = Point::new(t.p[0].x - pp.x, t.p[0].y - pp.y);
        }
        delta_base = Point::new(penwidth / 2.0 * cos_phi, penwidth / 2.0 * sin_phi);
    }
    if inv {
        p.x += delta_base.x;
        p.y += delta_base.y;
        q.x += delta_base.x;
        q.y += delta_base.y;
        a[0] = p;
        a[4] = p;
        a[1] = Point::new(p.x - v.x, p.y - v.y);
        a[2] = q;
        a[3] = Point::new(p.x + v.x, p.y + v.y);
        q.x += delta_tip.x;
        q.y += delta_tip.y;
    } else {
        p.x -= delta_tip.x;
        p.y -= delta_tip.y;
        q.x -= delta_tip.x;
        q.y -= delta_tip.y;
        a[0] = q;
        a[4] = q;
        a[1] = Point::new(q.x - v.x, q.y - v.y);
        a[2] = p;
        a[3] = Point::new(q.x + v.x, q.y + v.y);
        q.x -= delta_base.x;
        q.y -= delta_base.y;
    }
    q
}

fn arrow_type_crow0(
    mut p: Point,
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
    let v = Point::new(-u.y * arrowwidth, u.x * arrowwidth);
    let w = Point::new(-u.y * shaftwidth, u.x * shaftwidth);
    let mut q = Point::new(p.x + u.x, p.y + u.y);
    let m = Point::new(p.x + u.x * 0.5, p.y + u.y * 0.5);
    let origin = Point::new(0.0, 0.0);
    let v_inv = Point::new(-v.x, -v.y);
    let normal_left = if flag & ARR_MOD_RIGHT != 0 { origin } else { v };
    let normal_right = if flag & ARR_MOD_LEFT != 0 {
        origin
    } else {
        v_inv
    };
    let base_left = if inv { normal_right } else { normal_left };
    let base_right = if inv { normal_left } else { normal_right };
    let pp = if inv { Point::new(-u.x, -u.y) } else { u };
    let mut delta_tip = origin;
    let mut delta_base = origin;
    if u.x != 0.0 || u.y != 0.0 {
        let cos_phi = pp.x / math::hypot(pp.x, pp.y);
        let sin_phi = pp.y / math::hypot(pp.x, pp.y);
        let phi = if pp.y > 0.0 {
            math::acos(cos_phi)
        } else {
            -math::acos(cos_phi)
        };
        let left = flag & ARR_MOD_LEFT != 0;
        let right = flag & ARR_MOD_RIGHT != 0;
        if (left && inv) || (right && !inv) {
            delta_tip = side_delta(
                base_left, pp, base_right, penwidth, 2, cos_phi, sin_phi, phi,
            );
        } else if (left && !inv) || (right && inv) {
            delta_tip = side_delta(
                base_left, pp, base_right, penwidth, 1, cos_phi, sin_phi, phi,
            );
        } else {
            let t = miter_shape(base_left, pp, base_right, penwidth);
            delta_tip = Point::new(t.p[0].x - pp.x, t.p[0].y - pp.y);
        }
        if inv {
            delta_base = Point::new(penwidth / 2.0 * cos_phi, penwidth / 2.0 * sin_phi);
        } else {
            let toe_base_left = Point::new(m.x - q.x + w.x, m.y - q.y + w.y);
            let toe_p = Point::new(v.x - u.x, v.y - u.y);
            let d = side_delta(
                toe_base_left,
                toe_p,
                origin,
                penwidth,
                1,
                cos_phi,
                sin_phi,
                phi,
            );
            delta_base = Point::new(-d.x, -d.y);
        }
    }
    if inv {
        p.x -= delta_tip.x;
        p.y -= delta_tip.y;
        q.x -= delta_tip.x;
        q.y -= delta_tip.y;
        a[0] = p;
        a[8] = p;
        a[1] = Point::new(q.x - v.x, q.y - v.y);
        a[2] = Point::new(m.x - w.x, m.y - w.y);
        a[3] = Point::new(q.x - w.x, q.y - w.y);
        a[4] = q;
        a[5] = Point::new(q.x + w.x, q.y + w.y);
        a[6] = Point::new(m.x + w.x, m.y + w.y);
        a[7] = Point::new(q.x + v.x, q.y + v.y);
        q.x -= delta_base.x;
        q.y -= delta_base.y;
    } else {
        p.x += delta_base.x;
        p.y += delta_base.y;
        q.x += delta_base.x;
        q.y += delta_base.y;
        a[0] = q;
        a[8] = q;
        a[1] = Point::new(p.x - v.x, p.y - v.y);
        a[2] = Point::new(m.x - w.x, m.y - w.y);
        a[3] = Point::new(p.x + delta_base.x, p.y + delta_base.y);
        a[4] = a[3];
        a[5] = a[3];
        a[6] = Point::new(m.x + w.x, m.y + w.y);
        a[7] = Point::new(p.x + v.x, p.y + v.y);
        q.x += delta_tip.x;
        q.y += delta_tip.y;
    }
    q
}

fn arrow_type_diamond0(
    mut p: Point,
    u: Point,
    penwidth: f64,
    flag: u32,
    a: &mut [Point; 5],
) -> Point {
    let v = Point::new(-u.y / 3.0, u.x / 3.0);
    let mut r = Point::new(p.x + u.x / 2.0, p.y + u.y / 2.0);
    let mut q = Point::new(p.x + u.x, p.y + u.y);
    let origin = Point::new(0.0, 0.0);
    let unmod_left = Point::new(-0.5 * u.x - v.x, -0.5 * u.y - v.y);
    let unmod_right = Point::new(-0.5 * u.x + v.x, -0.5 * u.y + v.y);
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
    let tip = Point::new(-u.x, -u.y);
    let t = miter_shape(base_left, tip, base_right, penwidth);
    let delta = Point::new(t.p[0].x - tip.x, t.p[0].y - tip.y);
    p = Point::new(p.x - delta.x, p.y - delta.y);
    r = Point::new(r.x - delta.x, r.y - delta.y);
    q = Point::new(q.x - delta.x, q.y - delta.y);
    a[0] = q;
    a[4] = q;
    a[1] = Point::new(r.x + v.x, r.y + v.y);
    a[2] = p;
    a[3] = Point::new(r.x - v.x, r.y - v.y);
    Point::new(q.x - delta.x, q.y - delta.y)
}

fn len_for(t: u32, lenfact: f64, arrowsize: f64, penwidth: f64, flag: u32) -> f64 {
    let nominal = lenfact * arrowsize * ARROW_LENGTH;
    let u = Point::new(nominal, 0.0);
    let o = Point::new(0.0, 0.0);
    match t {
        ARR_TYPE_NORM => {
            let mut a = [o; 5];
            let q = arrow_type_normal0(o, u, penwidth, flag, &mut a);
            let full_length = q.x;
            let nominal_length = (a[1].x - a[2].x).abs();
            let nominal_base_width = a[3].y - a[1].y;
            let full_base_width = nominal_base_width * full_length / nominal_length;
            let overlap = if flag & ARR_MOD_INV != 0 {
                full_length * penwidth / full_base_width
            } else {
                penwidth / 2.0
            };
            full_length - overlap
        }
        ARR_TYPE_CROW => {
            let mut a = [o; 9];
            let q = arrow_type_crow0(o, u, arrowsize, penwidth, flag, &mut a);
            let full_length = q.x;
            let fl_ws = full_length - (a[1].x - a[3].x);
            let nominal_length = (a[1].x - a[0].x).abs();
            let nominal_base_width = a[7].y - a[1].y;
            let full_base_width = nominal_base_width * fl_ws / nominal_length;
            let overlap = if flag & ARR_MOD_INV != 0 {
                penwidth / 2.0
            } else {
                fl_ws * penwidth / full_base_width
            };
            full_length - overlap
        }
        ARR_TYPE_TEE => {
            let mut length = nominal;
            // C (clang) fuses `penwidth / 2 - k * nominal_length` into one fma.
            let start = math::fma(-(1.0 - 0.6f64), nominal, penwidth / 2.0);
            if start > 0.0 {
                length += start;
            }
            let end = math::fma(-0.2, nominal, penwidth / 2.0);
            if start > 0.0 {
                length += end;
            }
            length
        }
        // C (clang) fuses `lenfact * arrowsize * ARROW_LENGTH + penwidth…`.
        ARR_TYPE_BOX | ARR_TYPE_CURVE => {
            math::fma(lenfact * arrowsize, ARROW_LENGTH, penwidth / 2.0)
        }
        ARR_TYPE_DIAMOND => {
            let mut a = [o; 5];
            let q = arrow_type_diamond0(o, u, penwidth, flag, &mut a);
            let full_length = q.x / 2.0;
            let nominal_length = (a[3].x - a[2].x).abs();
            let nominal_base_width = a[1].y - a[3].y;
            let full_base_width = nominal_base_width * full_length / nominal_length;
            2.0 * full_length - full_length * penwidth / full_base_width
        }
        ARR_TYPE_DOT => math::fma(lenfact * arrowsize, ARROW_LENGTH, penwidth),
        _ => nominal,
    }
}

fn lenfact(t: u32) -> Option<f64> {
    Some(match t {
        ARR_TYPE_NORM | ARR_TYPE_CROW | ARR_TYPE_BOX | ARR_TYPE_CURVE => 1.0,
        ARR_TYPE_TEE | ARR_TYPE_GAP => 0.5,
        ARR_TYPE_DIAMOND => 1.2,
        ARR_TYPE_DOT => 0.8,
        _ => return None,
    })
}

/// `arrow_length(e, flag)`.
pub fn arrow_length(attrs: &AttrMap, flag: u32) -> f64 {
    let penwidth = late_double(attrs, "penwidth", 1.0, 0.0);
    let arrowsize = late_double(attrs, "arrowsize", 1.0, 0.0);
    if arrowsize == 0.0 {
        return 0.0;
    }
    let mut length = 0.0;
    for i in 0..NUMB_OF_ARROW_HEADS {
        let f = (flag >> (i * BITS_PER_ARROW)) & ((1 << BITS_PER_ARROW_TYPE) - 1);
        if let Some(lf) = lenfact(f) {
            let af = (flag >> (i * BITS_PER_ARROW)) & ((1 << BITS_PER_ARROW) - 1);
            length += len_for(f, lf, arrowsize, penwidth, af);
        }
    }
    length
}

use super::geom::dist;

/// `arrowEndClip`; returns the new `endp` and sets `eflag`/`ep` on the bezier.
pub fn arrow_end_clip(
    attrs: &AttrMap,
    ps: &mut [Point],
    startp: usize,
    mut endp: usize,
    spl: &mut super::types::Bezier,
    eflag: u32,
) -> usize {
    let elen = arrow_length(attrs, eflag);
    spl.eflag = eflag;
    spl.ep = ps[endp + 3];
    if endp > startp && dist(ps[endp], ps[endp + 3]) < elen {
        endp -= 3;
    }
    let mut sp = [spl.ep, ps[endp + 2], ps[endp + 1], ps[endp]];
    if elen > 0.0 {
        let c = sp[0];
        bezier_clip(&mut |p| dist(p, c) <= elen, &mut sp, true);
    }
    ps[endp] = sp[3];
    ps[endp + 1] = sp[2];
    ps[endp + 2] = sp[1];
    ps[endp + 3] = sp[0];
    endp
}

/// `arrowStartClip`; returns the new `startp` and sets `sflag`/`sp`.
pub fn arrow_start_clip(
    attrs: &AttrMap,
    ps: &mut [Point],
    mut startp: usize,
    endp: usize,
    spl: &mut super::types::Bezier,
    sflag: u32,
) -> usize {
    let slen = arrow_length(attrs, sflag);
    spl.sflag = sflag;
    spl.sp = ps[startp];
    if endp > startp && dist(ps[startp], ps[startp + 3]) < slen {
        startp += 3;
    }
    let mut sp = [ps[startp + 3], ps[startp + 2], ps[startp + 1], spl.sp];
    if slen > 0.0 {
        let c = sp[3];
        bezier_clip(&mut |p| dist(p, c) <= slen, &mut sp, false);
    }
    ps[startp] = sp[3];
    ps[startp + 1] = sp[2];
    ps[startp + 2] = sp[1];
    ps[startp + 3] = sp[0];
    startp
}

/// `arrowOrthoClip`: shorten the end segments of an orthogonal route by the
/// arrowhead lengths.
pub fn arrow_ortho_clip(
    attrs: &AttrMap,
    ps: &mut [Point],
    startp: usize,
    endp: usize,
    spl: &mut super::types::Bezier,
    sflag: u32,
    eflag: u32,
) {
    if sflag != 0 && eflag != 0 && endp == startp {
        let p = ps[endp];
        let q = ps[endp + 3];
        let mut tlen = arrow_length(attrs, sflag);
        let mut hlen = arrow_length(attrs, eflag);
        let d = dist(p, q);
        if hlen + tlen >= d {
            hlen = d / 3.0;
            tlen = d / 3.0;
        }
        let (s, t);
        if p.y == q.y {
            if p.x < q.x {
                t = Point::new(q.x - hlen, p.y);
                s = Point::new(p.x + tlen, p.y);
            } else {
                t = Point::new(q.x + hlen, p.y);
                s = Point::new(p.x - tlen, p.y);
            }
        } else if p.y < q.y {
            t = Point::new(p.x, q.y - hlen);
            s = Point::new(p.x, p.y + tlen);
        } else {
            t = Point::new(p.x, q.y + hlen);
            s = Point::new(p.x, p.y - tlen);
        }
        ps[endp] = s;
        ps[endp + 1] = s;
        ps[endp + 2] = t;
        ps[endp + 3] = t;
        spl.sflag = sflag;
        spl.sp = p;
        spl.eflag = eflag;
        spl.ep = q;
        return;
    }
    if eflag != 0 {
        let mut hlen = arrow_length(attrs, eflag);
        let p = ps[endp];
        let q = ps[endp + 3];
        let maxd = 0.9 * dist(p, q);
        if hlen >= maxd {
            hlen = maxd;
        }
        let r = if p.y == q.y {
            Point::new(if p.x < q.x { q.x - hlen } else { q.x + hlen }, p.y)
        } else {
            Point::new(p.x, if p.y < q.y { q.y - hlen } else { q.y + hlen })
        };
        ps[endp + 1] = p;
        ps[endp + 2] = r;
        ps[endp + 3] = r;
        spl.eflag = eflag;
        spl.ep = q;
    }
    if sflag != 0 {
        let mut tlen = arrow_length(attrs, sflag);
        let p = ps[startp];
        let q = ps[startp + 3];
        let maxd = 0.9 * dist(p, q);
        if tlen >= maxd {
            tlen = maxd;
        }
        let r = if p.y == q.y {
            Point::new(if p.x < q.x { p.x + tlen } else { p.x - tlen }, p.y)
        } else {
            Point::new(p.x, if p.y < q.y { p.y + tlen } else { p.y - tlen })
        };
        ps[startp] = r;
        ps[startp + 1] = r;
        ps[startp + 2] = q;
        spl.sflag = sflag;
        spl.sp = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_arrow_length() {
        let a = AttrMap::new();
        let l = arrow_length(&a, ARR_TYPE_NORM);
        // Value computed by the C arrow_length_normal for penwidth 1.
        assert!((l - 11.513_544_292_886_934).abs() < 1e-12, "{l}");
        assert_eq!(arrow_flags(&a, true), (0, 1));
        assert_eq!(arrow_flags(&a, false), (0, 0));
    }
}
