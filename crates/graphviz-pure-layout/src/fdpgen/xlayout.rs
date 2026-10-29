//! Expansion of an initial layout to remove node overlaps, ported from
//! `lib/fdpgen/xlayout.c`: `xinit_params`, `overlap`, `cntOverlaps`,
//! `doRep`/`applyRep`/`applyAttr`, `adjust`, `x_layout`, `fdp_xLayout`.

use super::layout::{Comp, DG, with_lgraph};
use super::tlayout::TState;
use crate::common::types::P_PIN;
use crate::neatogen::adjust::{Expand, remove_overlap_as, sep_factor};
use graphviz_pure_core::math;

/// Default `overlap` for fdp.
const DFLT_OVERLAP: &str = "9:prism";

/// `xparams`.
#[derive(Debug, Clone, Copy, Default)]
pub struct XParams {
    pub num_iters: i32,
    pub t0: f64,
    pub k: f64,
    pub c: f64,
    pub loopcnt: i32,
}

struct X {
    p: XParams,
    marg: Expand,
}

impl X {
    fn wd2(&self, w: f64) -> f64 {
        if self.marg.do_add {
            w / 2.0 + self.marg.x
        } else {
            w * self.marg.x / 2.0
        }
    }

    fn ht2(&self, h: f64) -> f64 {
        if self.marg.do_add {
            h / 2.0 + self.marg.y
        } else {
            h * self.marg.y / 2.0
        }
    }

    /// `cool(t)`.
    fn cool(&self, t: i32) -> f64 {
        self.p.t0 * f64::from(self.p.num_iters.wrapping_sub(t)) / f64::from(self.p.num_iters)
    }

    /// C: lib/fdpgen/xlayout.c:cntOverlaps
    fn cnt_overlaps(&self, dg: &DG, comp: &Comp) -> i64 {
        let mut cnt = 0;
        let mut nodes = comp.nodes.as_slice();
        while let Some((&p, rest)) = nodes.split_first() {
            let pd = &dg.nodes[p];
            let phalf = [self.wd2(pd.width), self.ht2(pd.height)];
            for &q in rest {
                let qd = &dg.nodes[q];
                let qhalf = [self.wd2(qd.width), self.ht2(qd.height)];
                cnt += i64::from(overlap(pd.pos, qd.pos, phalf, qhalf));
            }
            nodes = rest;
        }
        cnt
    }

    /// C: lib/fdpgen/xlayout.c:adjust, doRep, applyAttr
    /// Component-local position/displacement slices retain the original pair and edge order.
    fn adjust(
        &self,
        st: &TState,
        dg: &DG,
        comp: &Comp,
        work: &mut Work,
        temp: f64,
        x_ov: f64,
        x_nonov: f64,
    ) -> i64 {
        work.disp.fill([0.0, 0.0]);
        let mut overlaps = 0;
        for (i, (((&p, pg), &n), heads)) in work
            .pos
            .iter()
            .zip(&work.geom)
            .zip(&comp.nodes)
            .zip(&work.ranges)
            .enumerate()
        {
            let end = i + 1;
            // All three arrays have one entry per component node. Split only once per
            // outer node; the quadratic pair walk below uses references, without indexing.
            for ((pd, qds), (_, qps)) in work.disp[i..]
                .split_first_mut()
                .into_iter()
                .zip(work.pos[i..].split_first())
            {
                for (((&q, qd), qg), &n1) in qps
                    .iter()
                    .zip(qds)
                    .zip(&work.geom[end..])
                    .zip(comp.nodes.iter().skip(end))
                {
                    let mut xdelta = q[0] - p[0];
                    let mut ydelta = q[1] - p[1];
                    let mut dist = math::hypot(xdelta, ydelta);
                    while !(dist > 0.0) {
                        xdelta = f64::from(5 - crate::cutil::rand() % 10);
                        ydelta = f64::from(5 - crate::cutil::rand() % 10);
                        dist = math::hypot(xdelta, ydelta);
                    }
                    // Overlap uses the actual positions, even when the force delta was jittered.
                    let ov = overlap(p, q, pg.half, qg.half);
                    let mut force = if ov {
                        x_ov / (dist * dist)
                    } else {
                        x_nonov / (dist * dist)
                    };
                    if dist > st.d.mlimit {
                        force = 0.0;
                    }
                    add_disp(qd, xdelta, ydelta, force);
                    add_disp(pd, -xdelta, -ydelta, force);
                    overlaps += i64::from(ov);
                    if super::trace(2) {
                        graphviz_pure_core::diag::dump(&format!(
                            "R {} {} {:?} {:?} {:?} {}",
                            dg.nodes[n].name,
                            dg.nodes[n1].name,
                            xdelta,
                            ydelta,
                            force,
                            i32::from(ov)
                        ));
                    }
                }
            }
            // CSR heads retain dg.outs(comp, n)'s order. Attraction remains interleaved
            // after each outer node's repulsion, including heads earlier in the component.
            for &h in &work.heads[heads.clone()] {
                let q = work.pos[h];
                let qg = &work.geom[h];
                if overlap(p, q, pg.half, qg.half) {
                    continue;
                }
                let xdelta = q[0] - p[0];
                let ydelta = q[1] - p[1];
                let dist = math::hypot(xdelta, ydelta);
                let din = pg.radius + qg.radius;
                let dout = dist - din;
                let force = dout * dout / ((self.p.k + din) * dist);
                add_disp(&mut work.disp[h], -xdelta, -ydelta, force);
                add_disp(&mut work.disp[i], xdelta, ydelta, force);
                if super::trace(2) {
                    graphviz_pure_core::diag::dump(&format!(
                        "A {} {} {:?} {:?} {:?}",
                        dg.nodes[n].name, dg.nodes[comp.nodes[h]].name, xdelta, ydelta, force
                    ));
                }
            }
        }
        if overlaps == 0 {
            return 0;
        }
        let temp2 = temp * temp;
        for ((pos, &disp), g) in work.pos.iter_mut().zip(&work.disp).zip(&work.geom) {
            if g.pinned {
                continue;
            }
            let len2 = math::fma(disp[0], disp[0], disp[1] * disp[1]);
            if len2 < temp2 {
                pos[0] += disp[0];
                pos[1] += disp[1];
            } else {
                let len = len2.sqrt();
                pos[0] += disp[0] * temp / len;
                pos[1] += disp[1] * temp / len;
            }
        }
        overlaps
    }
}

/// C: lib/fdpgen/xlayout.c:overlap
fn overlap(p: [f64; 2], q: [f64; 2], phalf: [f64; 2], qhalf: [f64; 2]) -> bool {
    let xdelta = (q[0] - p[0]).abs();
    let ydelta = (q[1] - p[1]).abs();
    xdelta <= phalf[0] + qhalf[0] && ydelta <= phalf[1] + qhalf[1]
}

/// C: lib/fdpgen/xlayout.c:WD2, HT2, RAD, adjust
struct Geometry {
    half: [f64; 2],
    radius: f64,
    pinned: bool,
}

/// C: lib/fdpgen/xlayout.c:x_layout
/// One component's compact working arrays. Geometry is immutable throughout the walk;
/// positions and displacements are copied back together, in component order.
struct Work {
    pos: Vec<[f64; 2]>,
    disp: Vec<[f64; 2]>,
    geom: Vec<Geometry>,
    ranges: Vec<std::ops::Range<usize>>,
    heads: Vec<usize>,
}
impl Work {
    fn new(x: &X, dg: &DG, comp: &Comp, nedges: usize) -> Self {
        let mut work = Self {
            pos: Vec::with_capacity(comp.nodes.len()),
            disp: Vec::with_capacity(comp.nodes.len()),
            geom: Vec::with_capacity(comp.nodes.len()),
            ranges: Vec::with_capacity(comp.nodes.len()),
            heads: Vec::with_capacity(nedges),
        };
        for &n in &comp.nodes {
            let nd = &dg.nodes[n];
            let half = [x.wd2(nd.width), x.ht2(nd.height)];
            work.pos.push(nd.pos);
            work.disp.push(nd.disp);
            work.geom.push(Geometry {
                half,
                radius: math::hypot(half[0], half[1]),
                pinned: nd.pinned == P_PIN,
            });
            let start = work.heads.len();
            for e in dg.outs(comp, n) {
                // Comp::new builds member from its sorted nodes; Comp::remove clears both.
                // outs filters every head through that same member set, and neither
                // structure changes during x_layout. Thus binary_search must succeed.
                let h = match comp.nodes.binary_search(&dg.edges[e].h) {
                    Ok(h) => h,
                    Err(_) => unreachable!("component outgoing head is a member"),
                };
                work.heads.push(h);
            }
            work.ranges.push(start..work.heads.len());
        }
        work
    }
    fn write_back(self, dg: &mut DG, comp: &Comp) {
        for ((&n, pos), disp) in comp.nodes.iter().zip(self.pos).zip(self.disp) {
            dg.nodes[n].pos = pos;
            dg.nodes[n].disp = disp;
        }
    }
}

/// C: lib/fdpgen/xlayout.c:DISP (doRep, applyAttr)
/// `DISP(n) += delta * force`, fused as clang compiles it.
fn add_disp(d: &mut [f64; 2], xdelta: f64, ydelta: f64, force: f64) {
    d[0] = math::fma(xdelta, force, d[0]);
    d[1] = math::fma(ydelta, force, d[1]);
}

/// C: lib/fdpgen/xlayout.c:x_layout
/// `x_layout(g, pxpms, tries)`: returns the overlaps that remain.
fn x_layout(
    st: &mut TState,
    dg: &mut DG,
    comp: &Comp,
    pxpms: &XParams,
    tries: i64,
    marg: Expand,
) -> i64 {
    let nnodes = comp.nodes.len() as i64;
    let nedges = comp
        .nodes
        .iter()
        .map(|&n| dg.outs(comp, n).count())
        .sum::<usize>() as i64;
    let mut marg = marg;
    if marg.do_add {
        marg.x /= 72.0;
        marg.y /= 72.0;
    }
    // The static xParams starts with C = 1.5 and is only overridden by a
    // positive C.
    let mut x = X {
        p: XParams {
            num_iters: 60,
            t0: 0.0,
            k: 0.3,
            c: 1.5,
            loopcnt: 0,
        },
        marg,
    };
    let mut ov = x.cnt_overlaps(dg, comp);
    if ov == 0 {
        return 0;
    }
    // Keep the zero-overlap early return allocation-free.
    let mut work = Work::new(&x, dg, comp, nedges as usize);
    let mut xpms = *pxpms;
    let k = xpms.k;
    let mut t = 0;
    while ov != 0 && t < tries {
        // xinit_params
        x.p.k = xpms.k;
        x.p.num_iters = xpms.num_iters;
        x.p.t0 = xpms.t0;
        x.p.loopcnt = xpms.loopcnt;
        if xpms.c > 0.0 {
            x.p.c = xpms.c;
        }
        let k2 = x.p.k * x.p.k;
        if x.p.t0 == 0.0 {
            x.p.t0 = x.p.k * (nnodes as f64).sqrt() / 5.0;
        }
        let x_ov = x.p.c * k2;
        let x_nonov = nedges as f64 * x_ov * 2.0 / (nnodes * (nnodes - 1)) as f64;
        for i in 0..x.p.loopcnt {
            let temp = x.cool(i);
            if temp <= 0.0 {
                break;
            }
            ov = x.adjust(st, dg, comp, &mut work, temp, x_ov, x_nonov);
            if super::trace(1) {
                graphviz_pure_core::diag::dump(&format!(
                    "XI {t} {i} {ov} {:?} {:?} {:?}",
                    temp, x_ov, x_nonov
                ));
                for (pos, disp) in work.pos.iter().zip(&work.disp) {
                    graphviz_pure_core::diag::dump(&format!(
                        "  xi {:?} {:?} {:?} {:?}",
                        pos[0], pos[1], disp[0], disp[1]
                    ));
                }
            }
            if ov == 0 {
                break;
            }
        }
        xpms.k += k;
        t += 1;
    }
    work.write_back(dg, comp);
    ov
}

/// `fdp_xLayout(g, xpms)`: `overlap` may start with `n:` for `n` tries of
/// [`x_layout`] before `removeOverlapAs` with the rest.
pub fn fdp_x_layout(
    st: &mut TState,
    dg: &mut DG,
    comp: &Comp,
    xpms: &XParams,
) -> graphviz_pure_core::Result<()> {
    graphviz_pure_core::verbose_msg!("xLayout ");
    let ovlp = dg
        .attrs
        .get("overlap")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DFLT_OVERLAP)
        .to_string();
    let (tries, rest) = match ovlp.find(':') {
        Some(i) if i == 0 || ovlp.as_bytes()[0].is_ascii_digit() => {
            let t = i64::from(crate::common::utils::atoi(&ovlp)).max(0);
            (t, ovlp[i + 1..].to_string())
        }
        _ => (0, ovlp.clone()),
    };
    graphviz_pure_core::verbose_msg!("tries = {tries}, mode = {rest}\n");
    let nodes = comp.nodes.clone();
    if tries > 0 {
        // x_layout's sepFactor
        let marg = with_lgraph(dg, &nodes, |lg, _, _| sep_factor(lg))?;
        if x_layout(st, dg, comp, xpms, tries, marg) == 0 {
            return Ok(());
        }
    }
    with_lgraph(dg, &nodes, |lg, sub, _| {
        sub.name.clone_from(&comp.name);
        remove_overlap_as(lg, sub, Some(&rest));
    })?;
    Ok(())
}

/// C: lib/fdpgen/xlayout.c: original scalar traversal for differential tests.
#[cfg(test)]
mod reference {
    use super::*;
    struct X {
        p: XParams,
        marg: Expand,
    }
    impl X {
        fn wd2(&self, dg: &DG, n: usize) -> f64 {
            let w = dg.nodes[n].width;
            if self.marg.do_add {
                w / 2.0 + self.marg.x
            } else {
                w * self.marg.x / 2.0
            }
        }

        fn ht2(&self, dg: &DG, n: usize) -> f64 {
            let h = dg.nodes[n].height;
            if self.marg.do_add {
                h / 2.0 + self.marg.y
            } else {
                h * self.marg.y / 2.0
            }
        }

        fn rad(&self, dg: &DG, n: usize) -> f64 {
            math::hypot(self.wd2(dg, n), self.ht2(dg, n))
        }

        /// `cool(t)`.
        fn cool(&self, t: i32) -> f64 {
            self.p.t0 * f64::from(self.p.num_iters.wrapping_sub(t)) / f64::from(self.p.num_iters)
        }

        /// `overlap(p, q)`.
        fn overlap(&self, dg: &DG, p: usize, q: usize) -> bool {
            let xdelta = (dg.nodes[q].pos[0] - dg.nodes[p].pos[0]).abs();
            let ydelta = (dg.nodes[q].pos[1] - dg.nodes[p].pos[1]).abs();
            xdelta <= self.wd2(dg, p) + self.wd2(dg, q)
                && ydelta <= self.ht2(dg, p) + self.ht2(dg, q)
        }

        /// `cntOverlaps(g)`.
        fn cnt_overlaps(&self, dg: &DG, comp: &Comp) -> i64 {
            let mut cnt = 0;
            for (k, &p) in comp.nodes.iter().enumerate() {
                for &q in &comp.nodes[k + 1..] {
                    cnt += i64::from(self.overlap(dg, p, q));
                }
            }
            cnt
        }

        /// `doRep(p, q, xdelta, ydelta, dist, X_ov, X_nonov)`.
        fn do_rep(
            &self,
            st: &mut TState,
            dg: &mut DG,
            p: usize,
            q: usize,
            (mut xdelta, mut ydelta, mut dist): (f64, f64, f64),
            x_ov: f64,
            x_nonov: f64,
        ) -> bool {
            while !(dist > 0.0) {
                xdelta = f64::from(5 - crate::cutil::rand() % 10);
                ydelta = f64::from(5 - crate::cutil::rand() % 10);
                dist = math::hypot(xdelta, ydelta);
            }
            let ov = self.overlap(dg, p, q);
            let mut force = if ov {
                x_ov / (dist * dist)
            } else {
                x_nonov / (dist * dist)
            };
            if dist > st.d.mlimit {
                force = 0.0;
            }
            add_disp(dg, q, xdelta, ydelta, force);
            add_disp(dg, p, -xdelta, -ydelta, force);
            if crate::fdpgen::trace(2) {
                graphviz_pure_core::diag::dump(&format!(
                    "R {} {} {:?} {:?} {:?} {}",
                    dg.nodes[p].name,
                    dg.nodes[q].name,
                    xdelta,
                    ydelta,
                    force,
                    i32::from(ov)
                ));
            }
            ov
        }

        /// `applyAttr(p, q)`.
        fn apply_attr(&self, dg: &mut DG, p: usize, q: usize) {
            if self.overlap(dg, p, q) {
                return;
            }
            let xdelta = dg.nodes[q].pos[0] - dg.nodes[p].pos[0];
            let ydelta = dg.nodes[q].pos[1] - dg.nodes[p].pos[1];
            let dist = math::hypot(xdelta, ydelta);
            let din = self.rad(dg, p) + self.rad(dg, q);
            let dout = dist - din;
            let force = dout * dout / ((self.p.k + din) * dist);
            add_disp(dg, q, -xdelta, -ydelta, force);
            add_disp(dg, p, xdelta, ydelta, force);
            if crate::fdpgen::trace(2) {
                graphviz_pure_core::diag::dump(&format!(
                    "A {} {} {:?} {:?} {:?}",
                    dg.nodes[p].name, dg.nodes[q].name, xdelta, ydelta, force
                ));
            }
        }

        /// `adjust(g, temp, X_ov, X_nonov)`: returns the overlaps before the move.
        fn adjust(
            &self,
            st: &mut TState,
            dg: &mut DG,
            comp: &Comp,
            temp: f64,
            x_ov: f64,
            x_nonov: f64,
        ) -> i64 {
            let mut overlaps = 0;
            for &n in &comp.nodes {
                dg.nodes[n].disp = [0.0, 0.0];
            }
            for (k, &n) in comp.nodes.iter().enumerate() {
                for &n1 in &comp.nodes[k + 1..] {
                    let xdelta = dg.nodes[n1].pos[0] - dg.nodes[n].pos[0];
                    let ydelta = dg.nodes[n1].pos[1] - dg.nodes[n].pos[1];
                    let d = (xdelta, ydelta, math::hypot(xdelta, ydelta));
                    overlaps += i64::from(self.do_rep(st, dg, n, n1, d, x_ov, x_nonov));
                }
                for e in dg.outs(comp, n).collect::<Vec<_>>() {
                    let h = dg.edges[e].h;
                    self.apply_attr(dg, n, h);
                }
            }
            if overlaps == 0 {
                return 0;
            }
            let temp2 = temp * temp;
            for &n in &comp.nodes {
                let nd = &mut dg.nodes[n];
                if nd.pinned == P_PIN {
                    continue;
                }
                let disp = nd.disp;
                let len2 = math::fma(disp[0], disp[0], disp[1] * disp[1]);
                if len2 < temp2 {
                    nd.pos[0] += disp[0];
                    nd.pos[1] += disp[1];
                } else {
                    let len = len2.sqrt();
                    nd.pos[0] += disp[0] * temp / len;
                    nd.pos[1] += disp[1] * temp / len;
                }
            }
            overlaps
        }
    }

    /// `DISP(n) += delta * force`, fused as clang compiles it.
    fn add_disp(dg: &mut DG, n: usize, xdelta: f64, ydelta: f64, force: f64) {
        let d = &mut dg.nodes[n].disp;
        d[0] = math::fma(xdelta, force, d[0]);
        d[1] = math::fma(ydelta, force, d[1]);
    }

    /// `x_layout(g, pxpms, tries)`: returns the overlaps that remain.
    pub(super) fn run(
        st: &mut TState,
        dg: &mut DG,
        comp: &Comp,
        pxpms: &XParams,
        tries: i64,
        marg: Expand,
    ) -> i64 {
        let nnodes = comp.nodes.len() as i64;
        let nedges = comp
            .nodes
            .iter()
            .map(|&n| dg.outs(comp, n).count())
            .sum::<usize>() as i64;
        let mut marg = marg;
        if marg.do_add {
            marg.x /= 72.0;
            marg.y /= 72.0;
        }
        // The static xParams starts with C = 1.5 and is only overridden by a
        // positive C.
        let mut x = X {
            p: XParams {
                num_iters: 60,
                t0: 0.0,
                k: 0.3,
                c: 1.5,
                loopcnt: 0,
            },
            marg,
        };
        let mut ov = x.cnt_overlaps(dg, comp);
        if ov == 0 {
            return 0;
        }
        let mut xpms = *pxpms;
        let k = xpms.k;
        let mut t = 0;
        while ov != 0 && t < tries {
            // xinit_params
            x.p.k = xpms.k;
            x.p.num_iters = xpms.num_iters;
            x.p.t0 = xpms.t0;
            x.p.loopcnt = xpms.loopcnt;
            if xpms.c > 0.0 {
                x.p.c = xpms.c;
            }
            let k2 = x.p.k * x.p.k;
            if x.p.t0 == 0.0 {
                x.p.t0 = x.p.k * (nnodes as f64).sqrt() / 5.0;
            }
            let x_ov = x.p.c * k2;
            let x_nonov = nedges as f64 * x_ov * 2.0 / (nnodes * (nnodes - 1)) as f64;
            for i in 0..x.p.loopcnt {
                let temp = x.cool(i);
                if temp <= 0.0 {
                    break;
                }
                ov = x.adjust(st, dg, comp, temp, x_ov, x_nonov);
                if crate::fdpgen::trace(1) {
                    graphviz_pure_core::diag::dump(&format!(
                        "XI {t} {i} {ov} {:?} {:?} {:?}",
                        temp, x_ov, x_nonov
                    ));
                    for &n in &comp.nodes {
                        let d = &dg.nodes[n];
                        graphviz_pure_core::diag::dump(&format!(
                            "  xi {:?} {:?} {:?} {:?}",
                            d.pos[0], d.pos[1], d.disp[0], d.disp[1]
                        ));
                    }
                }
                if ov == 0 {
                    break;
                }
            }
            xpms.k += k;
            t += 1;
        }
        ov
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdpgen::layout::{DEdge, DKind, DNode};

    fn derived() -> DG {
        let mut dg = DG::default();
        for i in 0..8 {
            dg.nodes.push(DNode {
                kind: DKind::Real(i),
                addr: i as u64,
                name: format!("n{i}"),
                pos: if i == 2 {
                    [0.0, 0.0]
                } else {
                    [i as f64 * 0.02, i as f64 * -0.01]
                },
                width: 0.7 + i as f64 * 0.05,
                height: 0.5 + i as f64 * 0.02,
                lw: 0.0,
                rw: 0.0,
                ht: 0.0,
                pinned: if i == 3 { P_PIN } else { 0 },
                deg: 0,
                wdeg: 0,
                disp: [0.125, -0.75],
                shape: None,
            });
            dg.out.push(Vec::new());
            dg.inn.push(Vec::new());
        }
        // Include a head earlier in component order, a self edge, and a non-member head.
        for (t, h) in [(0, 2), (0, 5), (2, 2), (2, 3), (3, 7), (7, 2), (2, 1)] {
            let e = dg.edges.len();
            dg.edges.push(DEdge {
                t,
                h,
                dist: 1.0,
                factor: 1.0,
                reals: Vec::new(),
            });
            dg.out[t].push(e);
            dg.inn[h].push(e);
        }
        dg
    }

    #[test]
    fn flat_overlap_walk_matches_scalar_positions_displacements_and_rng() {
        crate::cutil::with_rand_state(&mut crate::cutil::RandState::default(), || {
            for additive in [false, true] {
                for separated in [false, true] {
                    for loopcnt in [0, 1, 8] {
                        for limit in [0.05, f64::INFINITY] {
                            let mut old = derived();
                            if separated {
                                for (i, n) in old.nodes.iter_mut().enumerate() {
                                    n.pos = [i as f64 * 20.0, 0.0];
                                }
                            }
                            let mut new = old.clone();
                            let comp = Comp::new(8, vec![0, 2, 3, 5, 7], false);
                            let parms = XParams {
                                num_iters: 12,
                                t0: 0.0,
                                k: 0.3,
                                c: 1.5,
                                loopcnt,
                            };
                            let marg = Expand {
                                x: if additive { 3.2 } else { 1.1 },
                                y: if additive { 4.8 } else { 1.2 },
                                do_add: additive,
                            };
                            let mut state = super::super::tlayout::fdp_init_params(
                                &graphviz_pure_core::AttrMap::new(),
                            );
                            state.d.mlimit = limit;
                            crate::cutil::srand(77);
                            let expected =
                                reference::run(&mut state, &mut old, &comp, &parms, 3, marg);
                            let next = crate::cutil::rand();
                            crate::cutil::srand(77);
                            let actual = x_layout(&mut state, &mut new, &comp, &parms, 3, marg);
                            assert_eq!(actual, expected);
                            assert_eq!(crate::cutil::rand(), next);
                            for (a, b) in old.nodes.iter().zip(&new.nodes) {
                                assert_eq!(a.pos.map(f64::to_bits), b.pos.map(f64::to_bits));
                                assert_eq!(a.disp.map(f64::to_bits), b.disp.map(f64::to_bits));
                            }
                        }
                    }
                }
            }
        });
    }
}
