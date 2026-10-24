//! Spring embedder for one connected component of a derived graph, with
//! boundary ports, ported from `lib/fdpgen/tlayout.c`: `fdp_initParams`,
//! `init_params`, `initPositions`, `doRep`/`applyRep`/`applyAttr`,
//! `gridRepulse`/`doNeighbor`, `updatePos`, `gAdjust`, `adjust`, `fdp_tLayout`.

use super::grid::Grid;
use super::layout::{Comp, DG};
use super::xlayout::XParams;
use crate::common::types::{P_FIX, P_SET};
use crate::common::utils::{late_double, late_int, parse_leading_int};
use graphviz_pure_core::cnum::{c_max, c_min};
use graphviz_pure_core::math;
use graphviz_pure_core::{AttrMap, Point};
use std::f64::consts::PI;

const EXPFACTOR: f64 = 1.2;
const DFLT_MAXITERS: i32 = 600;
const DFLT_K: f64 = 0.3;
const DFLT_CELL: f64 = 0.0;
const DFLT_SEED: i64 = 1;

pub const INIT_SELF: i32 = 0;
pub const INIT_REGULAR: i32 = 1;
pub const INIT_RANDOM: i32 = 2;

/// `fdpParms_t` defaults (`lib/common/globals.c`); never changed without
/// command-line flags (`-L`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FdpParms {
    pub use_grid: bool,
    pub use_new: bool,
    pub num_iters: i32,
    pub unscaled: i32,
    pub c: f64,
    pub tfact: f64,
    pub k: f64,
    pub t0: f64,
    pub mlimit: f64,
}

impl Default for FdpParms {
    fn default() -> Self {
        Self {
            use_grid: true,
            use_new: true,
            num_iters: -1,
            unscaled: 50,
            c: 0.0,
            tfact: 1.0,
            k: -1.0,
            t0: -1.0,
            mlimit: f64::INFINITY,
        }
    }
}

thread_local! {
    /// `fdp_parms`: the defaults, or what `-L` set.
    static FDP_PARMS: std::cell::Cell<FdpParms> = std::cell::Cell::new(FdpParms::default());
}

/// Sets `fdp_parms` (`dot -L`) for the layouts that follow on this thread.
pub fn set_fdp_parms(parms: FdpParms) {
    FDP_PARMS.with(|p| p.set(parms));
}

/// `parms_t`: the parameters in use, from [`FdpParms`] and graph attributes.
#[derive(Debug, Clone, Default)]
pub struct TParms {
    pub use_grid: bool,
    pub use_new: bool,
    pub seed: i64,
    pub num_iters: i32,
    pub max_iters: i32,
    pub unscaled: i32,
    pub c: f64,
    pub tfact: f64,
    pub k: f64,
    pub t0: f64,
    pub smode: i32,
    pub cell: f64,
    pub wd: f64,
    pub ht: f64,
    pub pass1: i32,
    pub loopcnt: i32,
}

/// Global fdp state: parameters and the C library random generators.
pub struct TState {
    pub d: FdpParms,
    pub t: TParms,
}

impl TState {
    /// `cool(t)`; the `int` difference wraps as in C.
    fn cool(&self, t: i32) -> f64 {
        self.t.t0 * f64::from(self.t.max_iters.wrapping_sub(t)) / f64::from(self.t.max_iters)
    }
}

/// `setSeed(G, dflt, &seed)`. C seeds from the clock when `start=random`
/// has no number; the port keeps the default seed to stay deterministic.
pub fn set_seed(attrs: &AttrMap, dflt: i32, seed: &mut i64) -> i32 {
    let Some(p) = attrs.get("start").filter(|s| !s.is_empty()) else {
        return dflt;
    };
    let b = p.as_bytes();
    let mut init = dflt;
    let mut rest = p.as_str();
    if b[0].is_ascii_alphabetic() {
        if let Some(r) = p.strip_prefix("self") {
            init = INIT_SELF;
            rest = r;
        } else if let Some(r) = p.strip_prefix("regular") {
            init = INIT_REGULAR;
            rest = r;
        } else if let Some(r) = p.strip_prefix("random") {
            init = INIT_RANDOM;
            rest = r;
        }
    } else if b[0].is_ascii_digit() {
        init = INIT_RANDOM;
    }
    if init == INIT_RANDOM
        && rest.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && let Some(v) = parse_leading_int(rest)
    {
        *seed = v;
    }
    init
}

/// `fdp_initParams(g)`: parameters from the root graph's attributes.
pub fn fdp_init_params(attrs: &AttrMap) -> TState {
    let mut d = FDP_PARMS.with(std::cell::Cell::get);
    let mut t = TParms {
        use_grid: d.use_grid,
        use_new: d.use_new,
        num_iters: d.num_iters,
        unscaled: d.unscaled,
        cell: DFLT_CELL,
        c: d.c,
        tfact: d.tfact,
        max_iters: late_int(attrs, "maxiter", DFLT_MAXITERS, 0),
        ..TParms::default()
    };
    t.k = late_double(attrs, "K", DFLT_K, 0.0);
    d.k = t.k;
    t.t0 = if d.t0 == -1.0 {
        late_double(attrs, "T0", -1.0, 0.0)
    } else {
        d.t0
    };
    t.seed = DFLT_SEED;
    t.smode = set_seed(attrs, INIT_RANDOM, &mut t.seed);
    if t.smode == INIT_SELF {
        graphviz_pure_core::diag::agwarningf("fdp does not support start=self - ignoring\n");
        t.seed = i64::from(INIT_RANDOM);
    }
    // `int` arithmetic, which wraps in C: maxiter=INT_MAX gives a pass 1 of 0 iterations.
    t.pass1 = t.unscaled.wrapping_mul(t.max_iters) / 100;
    if t.use_grid && t.cell <= 0.0 {
        t.cell = 3.0 * t.k;
    }
    TState { d, t }
}

/// `init_params(g, xpms)`; returns whether `T0` must be reset afterwards.
fn init_params(st: &mut TState, nnodes: usize, xpms: &mut XParams) -> bool {
    let mut ret = false;
    if st.t.t0 == -1.0 {
        st.t.t0 = st.t.tfact * st.t.k * (nnodes as f64).sqrt() / 5.0;
        ret = true;
    }
    xpms.t0 = st.cool(st.t.pass1);
    xpms.k = st.t.k;
    xpms.c = st.t.c;
    xpms.num_iters = st.t.max_iters.wrapping_sub(st.t.pass1);
    if st.t.num_iters >= 0 {
        if st.t.num_iters <= st.t.pass1 {
            st.t.loopcnt = st.t.num_iters;
            xpms.loopcnt = 0;
        } else if st.t.num_iters <= st.t.max_iters {
            st.t.loopcnt = st.t.pass1;
            xpms.loopcnt = st.t.num_iters.wrapping_sub(st.t.pass1);
        }
    } else {
        st.t.loopcnt = st.t.pass1;
        xpms.loopcnt = xpms.num_iters;
    }
    ret
}

/// `doRep(p, q, xdelta, ydelta, dist)`.
fn do_rep(
    st: &mut TState,
    dg: &mut DG,
    p: usize,
    q: usize,
    mut xdelta: f64,
    mut ydelta: f64,
    mut dist: f64,
) {
    while !(dist > 0.0) {
        xdelta = f64::from(5 - crate::cutil::rand() % 10);
        ydelta = f64::from(5 - crate::cutil::rand() % 10);
        dist = math::hypot(xdelta, ydelta);
    }
    let k = st.t.k;
    let mut force = if dist > st.d.mlimit {
        0.0
    } else if st.t.use_new {
        k * k / (dist * dist * dist)
    } else {
        k * k / (dist * dist)
    };
    if dg.nodes[p].is_port() && dg.nodes[q].is_port() {
        force *= 10.0;
    }
    add_disp(dg, q, xdelta, ydelta, force);
    add_disp(dg, p, -xdelta, -ydelta, force);
    if super::trace(6) {
        graphviz_pure_core::diag::dump(&format!(
            "tr {} {} {:?} {:?} {:?}",
            dg.nodes[p].name, dg.nodes[q].name, xdelta, ydelta, force
        ));
    }
}

/// `DISP(n)[i] += delta * force`, fused as clang compiles it.
fn add_disp(dg: &mut DG, n: usize, xdelta: f64, ydelta: f64, force: f64) {
    let d = &mut dg.nodes[n].disp;
    d[0] = xdelta.mul_add(force, d[0]);
    d[1] = ydelta.mul_add(force, d[1]);
}

/// `applyRep(p, q)`.
fn apply_rep(st: &mut TState, dg: &mut DG, p: usize, q: usize) {
    let xdelta = dg.nodes[q].pos[0] - dg.nodes[p].pos[0];
    let ydelta = dg.nodes[q].pos[1] - dg.nodes[p].pos[1];
    do_rep(st, dg, p, q, xdelta, ydelta, math::hypot(xdelta, ydelta));
}

/// `doNeighbor(grid, i, j, nodes)`.
fn do_neighbor(st: &mut TState, dg: &mut DG, grid: &Grid, i: i32, j: i32, nodes: &[usize]) {
    let Some(cell) = grid.find(i, j) else { return };
    let qs: Vec<usize> = cell.collect();
    for &p in nodes {
        for &q in &qs {
            let xdelta = dg.nodes[q].pos[0] - dg.nodes[p].pos[0];
            let ydelta = dg.nodes[q].pos[1] - dg.nodes[p].pos[1];
            let dist = math::hypot(xdelta, ydelta);
            if dist < st.t.cell {
                do_rep(st, dg, p, q, xdelta, ydelta, dist);
            }
        }
    }
}

/// `gridRepulse(cell, grid)`.
fn grid_repulse(st: &mut TState, dg: &mut DG, grid: &Grid, (i, j): (i32, i32), nodes: &[usize]) {
    for &p in nodes {
        for &q in nodes {
            if p != q {
                apply_rep(st, dg, p, q);
            }
        }
    }
    for (di, dj) in [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ] {
        // A node near 1e17 puts its cell index at INT_MAX, as
        // `(int)floor(x / Cell)` saturates on arm64. C's `i + 1` there is UB,
        // but clang emits a plain 32-bit add, which wraps (#196).
        do_neighbor(st, dg, grid, i.wrapping_add(di), j.wrapping_add(dj), nodes);
    }
}

/// `applyAttr(p, q, e)`.
fn apply_attr(st: &mut TState, dg: &mut DG, p: usize, q: usize, e: usize) {
    let mut xdelta = dg.nodes[q].pos[0] - dg.nodes[p].pos[0];
    let mut ydelta = dg.nodes[q].pos[1] - dg.nodes[p].pos[1];
    let mut dist2 = xdelta.mul_add(xdelta, ydelta * ydelta);
    while dist2 == 0.0 {
        xdelta = f64::from(5 - crate::cutil::rand() % 10);
        ydelta = f64::from(5 - crate::cutil::rand() % 10);
        dist2 = xdelta.mul_add(xdelta, ydelta * ydelta);
    }
    let dist = dist2.sqrt();
    let ed = &dg.edges[e];
    let force = if st.t.use_new {
        ed.factor * (dist - ed.dist) / dist
    } else {
        ed.factor * dist / ed.dist
    };
    add_disp(dg, q, -xdelta, -ydelta, force);
    add_disp(dg, p, xdelta, ydelta, force);
    if super::trace(6) {
        graphviz_pure_core::diag::dump(&format!(
            "ta {} {} {:?} {:?} {:?}",
            dg.nodes[p].name, dg.nodes[q].name, xdelta, ydelta, force
        ));
    }
}

/// `updatePos(g, temp, pp)`.
fn update_pos(st: &TState, dg: &mut DG, comp: &Comp, temp: f64, has_ports: bool) {
    let temp2 = temp * temp;
    for &n in &comp.nodes {
        let nd = &mut dg.nodes[n];
        if nd.pinned & P_FIX != 0 {
            continue;
        }
        let dx = nd.disp[0];
        let dy = nd.disp[1];
        let len2 = dx.mul_add(dx, dy * dy);
        let (x, y) = if len2 < temp2 {
            (nd.pos[0] + dx, nd.pos[1] + dy)
        } else {
            let fact = temp / len2.sqrt();
            (dx.mul_add(fact, nd.pos[0]), dy.mul_add(fact, nd.pos[1]))
        };
        if has_ports {
            let (wd, ht) = (st.t.wd, st.t.ht);
            let d = (x * x / (wd * wd) + y * y / (ht * ht)).sqrt();
            if nd.is_port() {
                nd.pos[0] = x / d;
                nd.pos[1] = y / d;
            } else if d >= 1.0 {
                nd.pos[0] = 0.95 * x / d;
                nd.pos[1] = 0.95 * y / d;
            } else {
                nd.pos[0] = x;
                nd.pos[1] = y;
            }
        } else {
            nd.pos[0] = x;
            nd.pos[1] = y;
        }
    }
}

/// `gAdjust(g, temp, pp, grid)`.
fn g_adjust(
    st: &mut TState,
    dg: &mut DG,
    comp: &Comp,
    temp: f64,
    has_ports: bool,
    grid: &mut Grid,
) {
    if temp <= 0.0 {
        return;
    }
    grid.clear();
    let cell = st.t.cell;
    for &n in &comp.nodes {
        let nd = &mut dg.nodes[n];
        nd.disp = [0.0, 0.0];
        let (i, j) = (
            (nd.pos[0] / cell).floor() as i32,
            (nd.pos[1] / cell).floor() as i32,
        );
        grid.add(i, j, n);
        if graphviz_pure_core::diag::verbose_byte() >= 3 {
            graphviz_pure_core::diag::fprintf_stderr(&format!("grid({i},{j}): {}\n", nd.name));
        }
    }
    for &n in &comp.nodes {
        for e in dg.outs(comp, n).collect::<Vec<_>>() {
            let h = dg.edges[e].h;
            if n != h {
                apply_attr(st, dg, n, h, e);
            }
        }
    }
    for (key, nodes) in grid.walk_order() {
        grid_repulse(st, dg, grid, key, &nodes);
    }
    update_pos(st, dg, comp, temp, has_ports);
}

/// `adjust(g, temp, pp)`.
fn adjust(st: &mut TState, dg: &mut DG, comp: &Comp, temp: f64, has_ports: bool) {
    if temp <= 0.0 {
        return;
    }
    for &n in &comp.nodes {
        dg.nodes[n].disp = [0.0, 0.0];
    }
    for (k, &n) in comp.nodes.iter().enumerate() {
        for &n1 in &comp.nodes[k + 1..] {
            apply_rep(st, dg, n, n1);
        }
        for e in dg.outs(comp, n).collect::<Vec<_>>() {
            let h = dg.edges[e].h;
            if n != h {
                apply_attr(st, dg, n, h, e);
            }
        }
    }
    update_pos(st, dg, comp, temp, has_ports);
}

/// `initPositions(g, pp)`: the initial layout; returns the boundary centre.
fn init_positions(st: &mut TState, dg: &mut DG, comp: &Comp, has_ports: bool) -> Point {
    let nports = if has_ports { dg.ports.len() } else { 0 };
    let ng = comp.nodes.len() as i64 - nports as i64;
    let mut n_pos = 0;
    let (mut llx, mut lly, mut urx, mut ury) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &np in &comp.nodes {
        let nd = &dg.nodes[np];
        if nd.pinned != 0 {
            if n_pos > 0 {
                // C's MIN/MAX ternaries: a NaN already in the box stays there.
                llx = c_min(nd.pos[0], llx);
                lly = c_min(nd.pos[1], lly);
                urx = c_max(nd.pos[0], urx);
                ury = c_max(nd.pos[1], ury);
            } else {
                llx = nd.pos[0];
                urx = llx;
                lly = nd.pos[1];
                ury = lly;
            }
            n_pos += 1;
        }
    }
    let size = st.t.k * ((ng as f64).sqrt() + 1.0);
    st.t.wd = EXPFACTOR * (size / 2.0);
    st.t.ht = st.t.wd;
    let ctr;
    if n_pos == 1 {
        ctr = Point::new(llx, lly);
    } else if n_pos > 1 {
        ctr = Point::new((llx + urx) / 2.0, (lly + ury) / 2.0);
        let mut width = EXPFACTOR * (urx - llx);
        let mut height = EXPFACTOR * (ury - lly);
        let area = 4.0 * st.t.wd * st.t.ht;
        let mut quot = width * height / area;
        if quot >= 1.0 {
            st.t.wd = width / 2.0;
            st.t.ht = height / 2.0;
        } else if quot > 0.0 {
            quot = 2.0 * quot.sqrt();
            st.t.wd = width / quot;
            st.t.ht = height / quot;
        } else if width > 0.0 {
            height = area / width;
            st.t.wd = width / 2.0;
            st.t.ht = height / 2.0;
        } else if height > 0.0 {
            width = area / height;
            st.t.wd = width / 2.0;
            st.t.ht = height / 2.0;
        }
        let alpha = math::atan2(st.t.ht, st.t.wd);
        st.t.wd /= math::cos(alpha);
        st.t.ht /= math::sin(alpha);
    } else {
        ctr = Point::new(0.0, 0.0);
    }

    // Seed: C uses the clock unless smode is INIT_RANDOM.
    let local_seed = if st.t.smode == INIT_RANDOM {
        st.t.seed
    } else {
        DFLT_SEED
    };
    crate::cutil::srand48(local_seed);

    let (wd, ht) = (st.t.wd, st.t.ht);
    if has_ports {
        for i in 0..dg.ports.len() {
            let (np, alpha) = (dg.ports[i].n, dg.ports[i].alpha);
            let nd = &mut dg.nodes[np];
            // C (tlayout.c:501-502) calls `cos` and `sin` separately here: the store to
            // `ND_pos(np)[0]` may alias `pp->alpha`, so clang reloads it and cannot merge the
            // two into `__sincos_stret`, whose sine differs from `sin()` in the last bit on
            // some inputs (issue #15). `black_box` keeps LLVM from merging them in the port.
            // (The `atan2` block above and the random placement below are merged in C too.)
            nd.pos[0] = wd.mul_add(math::cos(alpha), ctr.x);
            nd.pos[1] = ht.mul_add(math::sin(std::hint::black_box(alpha)), ctr.y);
            nd.pinned = P_SET;
        }
        for &np in &comp.nodes {
            if dg.nodes[np].is_port() {
                continue;
            }
            if dg.nodes[np].pinned != 0 {
                dg.nodes[np].pos[0] -= ctr.x;
                dg.nodes[np].pos[1] -= ctr.y;
            } else {
                let (mut px, mut py) = (0.0f64, 0.0f64);
                let mut cnt = 0i32;
                for ep in dg.all_edges(comp, np) {
                    let (t, h) = (dg.edges[ep].t, dg.edges[ep].h);
                    if t == h {
                        continue;
                    }
                    let op = if h == np { t } else { h };
                    if dg.nodes[op].pinned == 0 {
                        continue;
                    }
                    let o = dg.nodes[op].pos;
                    if cnt > 0 {
                        px = px.mul_add(f64::from(cnt), o[0]) / f64::from(cnt + 1);
                        py = py.mul_add(f64::from(cnt), o[1]) / f64::from(cnt + 1);
                    } else {
                        px = o[0];
                        py = o[1];
                    }
                    cnt += 1;
                }
                if cnt > 1 {
                    dg.nodes[np].pos = [px, py];
                } else if cnt == 1 {
                    dg.nodes[np].pos = [
                        0.98f64.mul_add(px, 0.1 * ctr.x),
                        0.9f64.mul_add(py, 0.1 * ctr.y),
                    ];
                } else {
                    let angle = PI * 2.0 * crate::cutil::drand48();
                    let radius = 0.9 * crate::cutil::drand48();
                    dg.nodes[np].pos = [
                        radius * wd * math::cos(angle),
                        radius * ht * math::sin(angle),
                    ];
                }
                dg.nodes[np].pinned = P_SET;
            }
        }
    } else if n_pos > 0 {
        for &np in &comp.nodes {
            if dg.nodes[np].pinned != 0 {
                dg.nodes[np].pos[0] -= ctr.x;
                dg.nodes[np].pos[1] -= ctr.y;
            } else {
                let x = wd * rnd();
                let y = ht * rnd();
                dg.nodes[np].pos = [x, y];
            }
        }
    } else {
        for &np in &comp.nodes {
            let x = wd * rnd();
            let y = ht * rnd();
            dg.nodes[np].pos = [x, y];
        }
    }
    ctr
}

/// `2.0 * drand48() - 1.0` (fused).
fn rnd() -> f64 {
    2.0f64.mul_add(crate::cutil::drand48(), -1.0)
}

/// `fdp_tLayout(g, xpms)`.
pub fn fdp_t_layout(st: &mut TState, dg: &mut DG, comp: &Comp, xpms: &mut XParams) {
    let has_ports = comp.has_ports;
    let reset = init_params(st, comp.nodes.len(), xpms);
    let ctr = init_positions(st, dg, comp, has_ports);
    if st.t.use_grid {
        let mut grid = Grid::new();
        let tr = |st: &TState, dg: &DG| {
            for &n in &comp.nodes {
                graphviz_pure_core::diag::dump(&format!(
                    "  ti {:?} {:?}",
                    dg.nodes[n].pos[0], dg.nodes[n].pos[1]
                ));
            }
            let _ = st;
        };
        if super::trace(5) {
            graphviz_pure_core::diag::dump(&format!(
                "TI init Wd={:?} Ht={:?} ctr={:?},{:?} loop={} T0={:?}",
                st.t.wd, st.t.ht, ctr.x, ctr.y, st.t.loopcnt, st.t.t0
            ));
            tr(st, dg);
        }
        for i in 0..st.t.loopcnt {
            let temp = st.cool(i);
            g_adjust(st, dg, comp, temp, has_ports, &mut grid);
            if super::trace(5) {
                graphviz_pure_core::diag::dump(&format!("TI {i} {temp:?}"));
                tr(st, dg);
            }
        }
    } else {
        for i in 0..st.t.loopcnt {
            let temp = st.cool(i);
            adjust(st, dg, comp, temp, has_ports);
        }
    }
    if ctr.x != 0.0 || ctr.y != 0.0 {
        for &n in &comp.nodes {
            dg.nodes[n].pos[0] += ctr.x;
            dg.nodes[n].pos[1] += ctr.y;
        }
    }
    if reset {
        st.t.t0 = -1.0;
    }
}
