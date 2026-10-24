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
        xdelta <= self.wd2(dg, p) + self.wd2(dg, q) && ydelta <= self.ht2(dg, p) + self.ht2(dg, q)
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
        if super::trace(2) {
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
        if super::trace(2) {
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
            let len2 = disp[0].mul_add(disp[0], disp[1] * disp[1]);
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
    d[0] = xdelta.mul_add(force, d[0]);
    d[1] = ydelta.mul_add(force, d[1]);
}

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
            if super::trace(1) {
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

/// `fdp_xLayout(g, xpms)`: `overlap` may start with `n:` for `n` tries of
/// [`x_layout`] before `removeOverlapAs` with the rest.
pub fn fdp_x_layout(st: &mut TState, dg: &mut DG, comp: &Comp, xpms: &XParams) {
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
        let marg = with_lgraph(dg, &nodes, |lg, _, _| sep_factor(lg));
        if x_layout(st, dg, comp, xpms, tries, marg) == 0 {
            return;
        }
    }
    with_lgraph(dg, &nodes, |lg, sub, _| {
        sub.name.clone_from(&comp.name);
        remove_overlap_as(lg, sub, Some(&rest));
    });
}
