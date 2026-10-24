//! Port of `lib/sfdpgen/spring_electrical.c`: the spring–electrical
//! embedding (`spring_electrical_embedding` with Barnes–Hut supernodes,
//! the `_fast` variant with cell–cell interactions, the `_slow` all-pairs
//! variant), the multilevel driver, prolongation, leaf beautification and
//! the final principal-axis rotation.
//!
//! Floating-point expressions keep C's operation order; where clang
//! contracts `a*b + c` into a fused multiply-add on arm64, `mul_add` is used.

use super::multilevel::{MultilevelControl, multilevel_new};
use super::post_process::RelativePositionConstraints;
use super::post_process::post_process_smoothing;
use crate::neatogen::overlap::remove_overlap_el;
use crate::sparse::quad_tree::{QuadTree, Supernodes};
use crate::sparse::{
    MINDIST, SparseMatrix, distance, distance_cropped, drand, from_coordinate_arrays,
};
use graphviz_pure_core::cnum::c_max;
use graphviz_pure_core::math;

/// `ERROR_NOT_SQUARE_MATRIX`.
pub const ERROR_NOT_SQUARE_MATRIX: i32 = -100;
/// `AUTOP`: `p` is chosen automatically.
pub const AUTOP: f64 = -1.000_123_4;

pub const SMOOTHING_NONE: i32 = 0;
pub const SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST: i32 = 1;
pub const SMOOTHING_STRESS_MAJORIZATION_AVG_DIST: i32 = 2;
pub const SMOOTHING_STRESS_MAJORIZATION_POWER_DIST: i32 = 3;
pub const SMOOTHING_SPRING: i32 = 4;
pub const SMOOTHING_TRIANGLE: i32 = 5;
pub const SMOOTHING_RNG: i32 = 6;

pub const QUAD_TREE_HYBRID_SIZE: usize = 10000;
pub const QUAD_TREE_NONE: i32 = 0;
pub const QUAD_TREE_NORMAL: i32 = 1;
pub const QUAD_TREE_FAST: i32 = 2;
pub const QUAD_TREE_HYBRID: i32 = 3;

/// `ELSCHEME_*` (`lib/neatogen/overlap.h`).
pub const ELSCHEME_NONE: i32 = 0;
pub const ELSCHEME_PENALTY: i32 = 1;
pub const ELSCHEME_PENALTY2: i32 = 2;
pub const ELSCHEME_STRAIGHTLINE_PENALTY: i32 = 3;
pub const ELSCHEME_STRAIGHTLINE_PENALTY2: i32 = 4;

/// `C`: attractive force constant.
const C: f64 = 0.2;
/// `quadtree_size`: below this many nodes the quadtree is not used.
const QUADTREE_SIZE: usize = 45;
/// `bh`: Barnes–Hut constant.
const BH: f64 = 0.6;
/// `tol`: minimum step before terminating.
const TOL: f64 = 0.001;
const COOL: f64 = 0.90;

/// `spring_electrical_control`.
#[derive(Debug, Clone, Copy)]
pub struct SpringElectricalControl {
    pub p: f64,
    pub k: f64,
    pub multilevels: i32,
    pub max_qtree_level: i32,
    pub maxiter: i32,
    pub step: f64,
    pub random_seed: i32,
    pub random_start: bool,
    pub adaptive_cooling: bool,
    pub beautify_leaves: bool,
    pub smoothing: i32,
    pub overlap: i32,
    pub do_shrinking: bool,
    pub tscheme: i32,
    pub initial_scaling: f64,
    pub rotation: f64,
    pub edge_labeling_scheme: i32,
}

/// `spring_electrical_control_print(ctrl)` (`-v`).
pub fn spring_electrical_control_print(ctrl: &SpringElectricalControl) {
    use graphviz_pure_core::diag::{fmt_f, fprintf_stderr};
    const SMOOTHINGS: [&str; 7] = [
        "NONE",
        "STRESS_MAJORIZATION_GRAPH_DIST",
        "STRESS_MAJORIZATION_AVG_DIST",
        "STRESS_MAJORIZATION_POWER_DIST",
        "SPRING",
        "TRIANGLE",
        "RNG",
    ];
    const TSCHEMES: [&str; 4] = ["NONE", "NORMAL", "FAST", "HYBRID"];
    let f = |v: f64| fmt_f(v, 3);
    let name = |names: &[&'static str], i: i32| {
        usize::try_from(i).ok().and_then(|i| names.get(i).copied())
    };
    fprintf_stderr(&format!(
        "spring_electrical_control:\n  repulsive exponent: {}\n  random start {} seed {}\n  \
         K : {} C : {}\n  max levels {}\n  quadtree size {QUADTREE_SIZE} max_level {}\n  \
         Barnes-Hutt constant {} tolerance  {} maxiter {}\n  cooling {} step size  {} \
         adaptive {}\n  beautify_leaves {} node weights 0 rotation {}\n  smoothing {} overlap \
         {} initial_scaling {} do_shrinking {}\n  octree scheme {}\n  edge_labeling_scheme {}\n",
        f(ctrl.p),
        i32::from(ctrl.random_start),
        ctrl.random_seed,
        f(ctrl.k),
        f(C),
        ctrl.multilevels,
        ctrl.max_qtree_level,
        f(BH),
        f(TOL),
        ctrl.maxiter,
        f(COOL),
        f(ctrl.step),
        i32::from(ctrl.adaptive_cooling),
        i32::from(ctrl.beautify_leaves),
        f(ctrl.rotation),
        name(&SMOOTHINGS, ctrl.smoothing).unwrap_or(""),
        ctrl.overlap,
        f(ctrl.initial_scaling),
        i32::from(ctrl.do_shrinking),
        name(&TSCHEMES, ctrl.tscheme).unwrap_or(""),
        ctrl.edge_labeling_scheme,
    ));
}

impl Default for SpringElectricalControl {
    /// `spring_electrical_control_new()`.
    fn default() -> Self {
        SpringElectricalControl {
            p: AUTOP,
            k: -1.0,
            multilevels: 0,
            max_qtree_level: 10,
            maxiter: 500,
            step: 0.1,
            random_seed: 123,
            random_start: true,
            adaptive_cooling: true,
            beautify_leaves: false,
            smoothing: SMOOTHING_NONE,
            overlap: 0,
            do_shrinking: true,
            tscheme: QUAD_TREE_HYBRID,
            initial_scaling: -4.0,
            rotation: 0.0,
            edge_labeling_scheme: 0,
        }
    }
}

const MAX_I: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptDir {
    Init,
    Up,
    Down,
}

/// `oned_optimizer`.
#[derive(Debug, Clone)]
struct OnedOptimizer {
    i: usize,
    work: [f64; MAX_I + 1],
    direction: OptDir,
}

impl OnedOptimizer {
    fn new(i: i32) -> Self {
        OnedOptimizer {
            i: i as usize,
            work: [0.0; MAX_I + 1],
            direction: OptDir::Init,
        }
    }

    /// `oned_optimizer_train(opt, work)`.
    fn train(&mut self, work: f64) {
        let i = self.i;
        self.work[i] = work;
        match self.direction {
            OptDir::Init => {
                if self.i == MAX_I {
                    self.direction = OptDir::Down;
                    self.i -= 1;
                } else {
                    self.direction = OptDir::Up;
                    self.i = MAX_I.min(self.i + 1);
                }
            }
            OptDir::Up => {
                if self.work[i] < self.work[i - 1] && self.i < MAX_I {
                    self.i = MAX_I.min(self.i + 1);
                } else {
                    self.i -= 1;
                    self.direction = OptDir::Down;
                }
            }
            OptDir::Down => {
                if self.work[i] < self.work[i + 1] && self.i > 0 {
                    self.i = self.i.saturating_sub(1);
                } else {
                    self.i += 1;
                    self.direction = OptDir::Up;
                }
            }
        }
    }

    fn get(&self) -> i32 {
        self.i as i32
    }
}

/// `average_edge_length(A, dim, coord)`; like C, the second coordinate
/// index lacks `+ k`.
pub fn average_edge_length(a: &SparseMatrix, dim: usize, coord: &[f64]) -> f64 {
    if a.ia[a.m] == 0 {
        return 1.0;
    }
    let mut dist = 0.0;
    for i in 0..a.m {
        for j in a.ia[i]..a.ia[i + 1] {
            let c = a.ja[j as usize] as usize;
            let mut d = 0.0f64;
            for k in 0..dim {
                let t = coord[dim * i + k] - coord[dim * c];
                d = t.mul_add(t, d);
            }
            dist += d.sqrt();
        }
    }
    dist / f64::from(a.ia[a.m])
}

/// `update_step(adaptive_cooling, step, Fnorm, Fnorm0)`.
fn update_step(adaptive_cooling: bool, step: f64, fnorm: f64, fnorm0: f64) -> f64 {
    if !adaptive_cooling {
        return COOL * step;
    }
    if fnorm >= fnorm0 {
        COOL * step
    } else if fnorm > 0.95 * fnorm0 {
        step
    } else {
        0.99 * step / COOL
    }
}

fn node_degree(a: &SparseMatrix, i: usize) -> i32 {
    a.ia[i + 1] - a.ia[i]
}

/// `set_leaves(x, dim, dist, ang, i, j)`.
fn set_leaves(x: &mut [f64], dim: usize, dist: f64, ang: f64, i: usize, j: usize) {
    x[dim * j] = math::cos(ang).mul_add(dist, x[dim * i]);
    x[dim * j + 1] = math::sin(ang).mul_add(dist, x[dim * i + 1]);
}

/// `beautify_leaves(dim, A, x)`: spread the leaves of each node evenly around it.
fn beautify_leaves(dim: usize, a: &SparseMatrix, x: &mut [f64]) {
    let m = a.m;
    let (ia, ja) = (&a.ia, &a.ja);
    let mut checked = vec![false; m];
    for i in 0..m {
        if ia[i + 1] - ia[i] != 1 {
            continue;
        }
        if checked[i] {
            continue;
        }
        let p = ja[ia[i] as usize] as usize;
        if !checked[p] {
            checked[p] = true;
            let mut dist = 0.0;
            let mut leaves = Vec::new();
            for j in ia[p]..ia[p + 1] {
                let c = ja[j as usize] as usize;
                if node_degree(a, c) == 1 {
                    checked[c] = true;
                    dist += distance(x, dim, p, c);
                    leaves.push(c);
                }
            }
            dist /= leaves.len() as f64;
            let pad = 0.1;
            let mut ang1 = 0.0 + pad;
            let ang2 = 2.0 * std::f64::consts::PI - pad;
            let mut step = 0.0;
            if leaves.len() > 1 {
                step = (ang2 - ang1) / leaves.len() as f64;
            }
            for &l in &leaves {
                set_leaves(x, dim, dist, ang1, p, l);
                ang1 += step;
            }
        }
    }
}

/// Shared set-up of the embeddings: symmetrize, random start, K and p.
/// Returns the symmetrized matrix with `KP` and `CRK`.
fn embedding_setup(
    a0: &SparseMatrix,
    dim: usize,
    ctrl: &mut SpringElectricalControl,
    x: &mut [f64],
) -> (SparseMatrix, f64, f64) {
    let n = a0.n;
    let mut a = a0.clone();
    let a = a.symmetrize(true);
    if ctrl.random_start {
        crate::cutil::srand(ctrl.random_seed as u32);
        for v in x.iter_mut().take(dim * n) {
            *v = drand();
        }
    }
    let mut k = ctrl.k;
    let mut p = ctrl.p;
    if k < 0.0 {
        k = average_edge_length(&a, dim, x);
        ctrl.k = k;
    }
    if p >= 0.0 {
        p = -1.0;
        ctrl.p = p;
    }
    let kp = math::powf(k, 1.0 - p);
    let crk = math::powf(C, (2.0 - p) / 3.0) / k;
    (a, kp, crk)
}

/// Attractive force `C^((2-p)/3) ||x_i-x_j||/K * (x_j - x_i)` on node `i`.
fn attractive(a: &SparseMatrix, dim: usize, x: &[f64], i: usize, crk: f64, f: &mut [f64]) {
    for j in a.ia[i]..a.ia[i + 1] {
        let c = a.ja[j as usize] as usize;
        if c == i {
            continue;
        }
        let dist = distance(x, dim, i, c);
        for k in 0..dim {
            let t = crk * (x[i * dim + k] - x[c * dim + k]);
            f[k] = (-t).mul_add(dist, f[k]);
        }
    }
}

/// Normalize `f` and move node `i` by `step` along it; returns `|f|`.
fn move_node(dim: usize, x: &mut [f64], i: usize, f: &mut [f64], step: f64) -> f64 {
    let mut ff = 0.0f64;
    for k in 0..dim {
        ff = f[k].mul_add(f[k], ff);
    }
    ff = ff.sqrt();
    if ff > 0.0 {
        for k in 0..dim {
            f[k] /= ff;
        }
    }
    for k in 0..dim {
        x[i * dim + k] = step.mul_add(f[k], x[i * dim + k]);
    }
    ff
}

/// `spring_electrical_embedding_fast(dim, A0, ctrl, x, flag)`.
fn spring_electrical_embedding_fast(
    dim: usize,
    a0: &SparseMatrix,
    ctrl: &mut SpringElectricalControl,
    x: &mut [f64],
    flag: &mut i32,
) {
    let maxiter = f64::from(ctrl.maxiter);
    let mut step = ctrl.step;
    let adaptive_cooling = ctrl.adaptive_cooling;
    let mut max_qtree_level = ctrl.max_qtree_level;
    if maxiter <= 0.0 {
        return;
    }
    let m = a0.m;
    let n = a0.n;
    if n == 0 || dim == 0 {
        return;
    }
    let mut opt = OnedOptimizer::new(max_qtree_level);
    *flag = 0;
    if m != n {
        *flag = ERROR_NOT_SQUARE_MATRIX;
        ctrl.max_qtree_level = max_qtree_level;
        return;
    }
    let (a, kp, crk) = embedding_setup(a0, dim, ctrl, x);
    let p = ctrl.p;
    let mut force = vec![0.0f64; dim * n];
    let mut fnorm = 0.0f64;
    let mut iter = 0;
    let mut counts = [0.0f64; 4];
    loop {
        iter += 1;
        let fnorm0 = fnorm;
        fnorm = 0.0;
        max_qtree_level = opt.get();
        let mut qt = QuadTree::new_from_point_list(dim, n, max_qtree_level, x);
        qt.get_repulsive_force(&mut force, x, BH, p, kp, &mut counts);
        for i in 0..n {
            attractive(&a, dim, x, i, crk, &mut force[i * dim..i * dim + dim]);
        }
        for i in 0..n {
            let mut f = force[i * dim..i * dim + dim].to_vec();
            fnorm += move_node(dim, x, i, &mut f, step);
            force[i * dim..i * dim + dim].copy_from_slice(&f);
        }
        opt.train(3.3f64.mul_add(counts[2], 0.85f64.mul_add(counts[1], counts[0])));
        step = update_step(adaptive_cooling, step, fnorm, fnorm0);
        if !(step > TOL && f64::from(iter) < maxiter) {
            break;
        }
    }
    if ctrl.beautify_leaves {
        beautify_leaves(dim, &a, x);
    }
    ctrl.max_qtree_level = max_qtree_level;
}

/// `spring_electrical_embedding_slow(dim, A0, ctrl, x, flag)`.
fn spring_electrical_embedding_slow(
    dim: usize,
    a0: &SparseMatrix,
    ctrl: &mut SpringElectricalControl,
    x: &mut [f64],
    flag: &mut i32,
) {
    // C's own trace, with no newline, on every call.
    graphviz_pure_core::diag::fprintf_stderr("spring_electrical_embedding_slow");
    let maxiter = f64::from(ctrl.maxiter);
    let mut step = ctrl.step;
    let adaptive_cooling = ctrl.adaptive_cooling;
    if maxiter <= 0.0 {
        return;
    }
    let m = a0.m;
    let n = a0.n;
    if n == 0 || dim == 0 {
        return;
    }
    *flag = 0;
    if m != n {
        *flag = ERROR_NOT_SQUARE_MATRIX;
        return;
    }
    let (a, kp, crk) = embedding_setup(a0, dim, ctrl, x);
    let p = ctrl.p;
    let mut force = vec![0.0f64; dim * n];
    let mut f = vec![0.0f64; dim];
    let mut fnorm = 0.0f64;
    let mut iter = 0;
    loop {
        force.iter_mut().for_each(|v| *v = 0.0);
        iter += 1;
        let fnorm0 = fnorm;
        fnorm = 0.0;
        for i in 0..n {
            f.iter_mut().for_each(|v| *v = 0.0);
            for j in 0..n {
                if j == i {
                    continue;
                }
                let dist = distance_cropped(x, dim, i, j);
                for k in 0..dim {
                    f[k] += kp * (x[i * dim + k] - x[j * dim + k]) / math::powf(dist, 1.0 - p);
                }
            }
            for k in 0..dim {
                force[i * dim + k] += f[k];
            }
        }
        for i in 0..n {
            f.iter_mut().for_each(|v| *v = 0.0);
            attractive(&a, dim, x, i, crk, &mut f);
            for k in 0..dim {
                force[i * dim + k] += f[k];
            }
        }
        for i in 0..n {
            f.copy_from_slice(&force[i * dim..i * dim + dim]);
            fnorm += move_node(dim, x, i, &mut f, step);
        }
        step = update_step(adaptive_cooling, step, fnorm, fnorm0);
        if !(step > TOL && f64::from(iter) < maxiter) {
            break;
        }
    }
    if ctrl.beautify_leaves {
        beautify_leaves(dim, &a, x);
    }
}

/// Repulsive force on node `i` from the Barnes–Hut supernodes.
fn repulsive_supernodes(
    sn: &Supernodes,
    dim: usize,
    x: &[f64],
    i: usize,
    kp: f64,
    p: f64,
    f: &mut [f64],
) {
    for j in 0..sn.len() {
        let dist = c_max(sn.distances[j], MINDIST);
        for k in 0..dim {
            f[k] += sn.wgts[j] * kp * (x[i * dim + k] - sn.center[j * dim + k])
                / math::powf(dist, 1.0 - p);
        }
    }
}

/// Repulsive force on node `i` from every other node.
fn repulsive_all(n: usize, dim: usize, x: &[f64], i: usize, kp: f64, p: f64, f: &mut [f64]) {
    for j in 0..n {
        if j == i {
            continue;
        }
        let dist = distance_cropped(x, dim, i, j);
        for k in 0..dim {
            f[k] += kp * (x[i * dim + k] - x[j * dim + k]) / math::powf(dist, 1.0 - p);
        }
    }
}

/// `spring_electrical_embedding(dim, A0, ctrl, x, flag)`.
fn spring_electrical_embedding(
    dim: usize,
    a0: &SparseMatrix,
    ctrl: &mut SpringElectricalControl,
    x: &mut [f64],
    flag: &mut i32,
) {
    let maxiter = f64::from(ctrl.maxiter);
    let mut step = ctrl.step;
    let adaptive_cooling = ctrl.adaptive_cooling;
    let mut max_qtree_level = ctrl.max_qtree_level;
    if maxiter <= 0.0 {
        return;
    }
    let m = a0.m;
    let n = a0.n;
    if n == 0 || dim == 0 {
        return;
    }
    let use_qt = n >= QUADTREE_SIZE;
    let mut opt = OnedOptimizer::new(0);
    if use_qt {
        opt = OnedOptimizer::new(max_qtree_level);
    }
    *flag = 0;
    if m != n {
        *flag = ERROR_NOT_SQUARE_MATRIX;
        if use_qt {
            ctrl.max_qtree_level = max_qtree_level;
        }
        return;
    }
    let (a, kp, crk) = embedding_setup(a0, dim, ctrl, x);
    let p = ctrl.p;
    let mut f = vec![0.0f64; dim];
    let mut sn = Supernodes::default();
    let mut fnorm = 0.0f64;
    let mut iter = 0;
    loop {
        iter += 1;
        let fnorm0 = fnorm;
        fnorm = 0.0;
        let mut nsuper_avg = 0.0f64;
        let mut counts_avg = 0.0f64;
        let qt = if use_qt {
            max_qtree_level = opt.get();
            Some(QuadTree::new_from_point_list(dim, n, max_qtree_level, x))
        } else {
            None
        };
        for i in 0..n {
            f.iter_mut().for_each(|v| *v = 0.0);
            attractive(&a, dim, x, i, crk, &mut f);
            if let Some(qt) = &qt {
                let counts = qt.get_supernodes(BH, &x[dim * i..dim * i + dim], i, &mut sn);
                counts_avg += counts;
                nsuper_avg += sn.len() as f64;
                repulsive_supernodes(&sn, dim, x, i, kp, p, &mut f);
            } else {
                repulsive_all(n, dim, x, i, kp, p, &mut f);
            }
            fnorm += move_node(dim, x, i, &mut f, step);
        }
        if qt.is_some() {
            nsuper_avg /= n as f64;
            counts_avg /= n as f64;
            opt.train(5.0f64.mul_add(nsuper_avg, counts_avg));
        }
        step = update_step(adaptive_cooling, step, fnorm, fnorm0);
        if !(step > TOL && f64::from(iter) < maxiter) {
            break;
        }
    }
    if ctrl.beautify_leaves {
        beautify_leaves(dim, &a, x);
    }
    if use_qt {
        ctrl.max_qtree_level = max_qtree_level;
    }
}

/// `spring_electrical_spring_embedding(dim, A0, D, ctrl, x)`: spring–electrical
/// with additional springs of ideal lengths `D` (the spring smoother).
pub fn spring_electrical_spring_embedding(
    dim: usize,
    a0: &SparseMatrix,
    d: &SparseMatrix,
    ctrl: &mut SpringElectricalControl,
    x: &mut [f64],
) {
    let maxiter = f64::from(ctrl.maxiter);
    let mut step = ctrl.step;
    let adaptive_cooling = ctrl.adaptive_cooling;
    let max_qtree_level = 10;
    if maxiter <= 0.0 {
        return;
    }
    let n = a0.n;
    if n == 0 || dim == 0 {
        return;
    }
    let use_qt = n >= QUADTREE_SIZE;
    let (a, kp, crk) = embedding_setup(a0, dim, ctrl, x);
    let p = ctrl.p;
    let (id, jd) = (&d.ia, &d.ja);
    let dv = d.a.as_ref().expect("real matrix");
    let mut f = vec![0.0f64; dim];
    let mut sn = Supernodes::default();
    let mut fnorm = 0.0f64;
    let mut iter = 0;
    loop {
        iter += 1;
        let fnorm0 = fnorm;
        fnorm = 0.0;
        let qt = use_qt.then(|| QuadTree::new_from_point_list(dim, n, max_qtree_level, x));
        for i in 0..n {
            f.iter_mut().for_each(|v| *v = 0.0);
            attractive(&a, dim, x, i, crk, &mut f);
            for j in id[i]..id[i + 1] {
                let j = j as usize;
                let c = jd[j] as usize;
                if c == i {
                    continue;
                }
                let dist = distance_cropped(x, dim, i, c);
                for k in 0..dim {
                    let t = 0.2
                        * crk
                        * (x[i * dim + k] - x[c * dim + k])
                        * (dist - dv[j])
                        * (dist - dv[j])
                        / dist;
                    if dist < dv[j] {
                        f[k] += t;
                    } else {
                        f[k] -= t;
                    }
                }
            }
            if let Some(qt) = &qt {
                qt.get_supernodes(BH, &x[dim * i..dim * i + dim], i, &mut sn);
                repulsive_supernodes(&sn, dim, x, i, kp, p, &mut f);
            } else {
                repulsive_all(n, dim, x, i, kp, p, &mut f);
            }
            fnorm += move_node(dim, x, i, &mut f, step);
        }
        step = update_step(adaptive_cooling, step, fnorm, fnorm0);
        if !(step > TOL && f64::from(iter) < maxiter) {
            break;
        }
    }
    if ctrl.beautify_leaves {
        beautify_leaves(dim, &a, x);
    }
}

/// `interpolate_coord(dim, A, x)`.
fn interpolate_coord(dim: usize, a: &SparseMatrix, x: &mut [f64]) {
    let alpha = 0.5;
    let mut y = vec![0.0f64; dim];
    for i in 0..a.m {
        y.iter_mut().for_each(|v| *v = 0.0);
        let mut nz = 0;
        for j in a.ia[i]..a.ia[i + 1] {
            let c = a.ja[j as usize] as usize;
            if c == i {
                continue;
            }
            nz += 1;
            for k in 0..dim {
                y[k] += x[c * dim + k];
            }
        }
        if nz > 0 {
            let beta = (1.0 - alpha) / f64::from(nz);
            for k in 0..dim {
                x[i * dim + k] = alpha.mul_add(x[i * dim + k], beta * y[k]);
            }
        }
    }
}

/// `prolongate(dim, A, P, R, x, y, delta)`.
fn prolongate(
    dim: usize,
    a: &SparseMatrix,
    p: &SparseMatrix,
    r: &SparseMatrix,
    x: &[f64],
    y: &mut [f64],
    delta: f64,
) {
    p.multiply_dense(x, y, dim);
    interpolate_coord(dim, a, y);
    for i in 0..r.m {
        for j in (r.ia[i] + 1)..r.ia[i + 1] {
            let c = r.ja[j as usize] as usize;
            for k in 0..dim {
                let d = drand() - 0.5;
                y[c * dim + k] = delta.mul_add(d, y[c * dim + k]);
            }
        }
    }
}

/// `power_law_graph(A)`.
fn power_law_graph(a: &SparseMatrix) -> bool {
    let m = a.m;
    let mut mask = vec![0i32; m + 1];
    let mut max = 0;
    for i in 0..m {
        let mut deg = 0usize;
        for j in a.ia[i]..a.ia[i + 1] {
            if i as i32 == a.ja[j as usize] {
                continue;
            }
            deg += 1;
        }
        mask[deg] += 1;
        max = max.max(mask[deg]);
    }
    f64::from(mask[1]) > 0.8 * f64::from(max) && f64::from(mask[1]) > 0.3 * m as f64
}

/// `pcp_rotate(n, dim, x)`: rotate to the principal axes.
fn pcp_rotate(n: usize, dim: usize, x: &mut [f64]) {
    let mut y = [0.0f64; 4];
    let mut center = [0.0f64; 2];
    for i in 0..n {
        for k in 0..dim {
            center[k] += x[i * dim + k];
        }
    }
    for c in center.iter_mut().take(dim) {
        *c /= n as f64;
    }
    for i in 0..n {
        for k in 0..dim {
            x[dim * i + k] -= center[k];
        }
    }
    for i in 0..n {
        for k in 0..dim {
            for l in 0..dim {
                y[dim * k + l] = x[i * dim + k].mul_add(x[i * dim + l], y[dim * k + l]);
            }
        }
    }
    let mut axis = [0.0f64; 2];
    if y[1] == 0.0 {
        axis = [0.0, 1.0];
    } else {
        let s = y[0].mul_add(y[0], 4.0 * y[1] * y[1]);
        let s = (-(2.0 * y[0])).mul_add(y[3], s);
        let s = y[3].mul_add(y[3], s);
        axis[0] = -(-y[0] + y[3] - s.sqrt()) / (2.0 * y[1]);
        axis[1] = 1.0;
    }
    let dist = axis[0].mul_add(axis[0], 1.0).sqrt();
    axis[0] /= dist;
    axis[1] /= dist;
    for i in 0..n {
        let x0 = x[dim * i].mul_add(axis[0], x[dim * i + 1] * axis[1]);
        let x1 = (-x[dim * i]).mul_add(axis[1], x[dim * i + 1] * axis[0]);
        x[dim * i] = x0;
        x[dim * i + 1] = x1;
    }
}

/// `rotate(n, dim, x, angle)`: rotate by `angle` degrees about the centroid.
fn rotate(n: usize, dim: usize, x: &mut [f64], angle: f64) {
    // C's truncated pi.
    #[allow(clippy::approx_constant)]
    let radian = 3.14159 / 180.0;
    let mut center = [0.0f64; 2];
    for i in 0..n {
        for k in 0..dim {
            center[k] += x[i * dim + k];
        }
    }
    for c in center.iter_mut().take(dim) {
        *c /= n as f64;
    }
    for i in 0..n {
        for k in 0..dim {
            x[dim * i + k] -= center[k];
        }
    }
    let axis = [math::cos(-angle * radian), math::sin(-angle * radian)];
    for i in 0..n {
        let x0 = x[dim * i].mul_add(axis[0], x[dim * i + 1] * axis[1]);
        let x1 = (-x[dim * i]).mul_add(axis[1], x[dim * i + 1] * axis[0]);
        x[dim * i] = x0;
        x[dim * i + 1] = x1;
    }
}

/// `attach_edge_label_coordinates(dim, A, n_edge_label_nodes, edge_label_nodes, x, x2)`.
fn attach_edge_label_coordinates(
    dim: usize,
    a: &SparseMatrix,
    edge_label_nodes: &[usize],
    x: &mut [f64],
    x2: &[f64],
) {
    let mut mask = vec![1i32; a.m];
    for &e in edge_label_nodes {
        if e < a.m {
            mask[e] = -1;
        }
    }
    let mut nnodes = 0;
    for m in mask.iter_mut() {
        if *m >= 0 {
            *m = nnodes;
            nnodes += 1;
        }
    }
    for i in 0..a.m {
        if mask[i] >= 0 {
            for k in 0..dim {
                x[i * dim + k] = x2[mask[i] as usize * dim + k];
            }
        }
    }
    for &ii in edge_label_nodes {
        let len = f64::from(a.ia[ii + 1] - a.ia[ii]);
        for k in 0..dim {
            x[ii * dim + k] = 0.0;
        }
        for j in a.ia[ii]..a.ia[ii + 1] {
            let c = a.ja[j as usize] as usize;
            for k in 0..dim {
                x[ii * dim + k] += x[c * dim + k];
            }
        }
        for k in 0..dim {
            x[ii * dim + k] /= len;
        }
    }
}

/// `shorting_edge_label_nodes(A, n_edge_label_nodes, edge_label_nodes)`.
fn shorting_edge_label_nodes(a: &SparseMatrix, edge_label_nodes: &[usize]) -> SparseMatrix {
    let (ia, ja) = (&a.ia, &a.ja);
    let mut mask = vec![1i32; a.m];
    for &e in edge_label_nodes {
        mask[e] = -1;
    }
    let mut id = 0;
    for m in mask.iter_mut() {
        if *m > 0 {
            *m = id;
            id += 1;
        }
    }
    let (mut irn, mut jcn) = (Vec::new(), Vec::new());
    for i in 0..a.m {
        if mask[i] < 0 {
            continue;
        }
        for j in ia[i]..ia[i + 1] {
            let c = ja[j as usize] as usize;
            if mask[c] >= 0 {
                irn.push(mask[i]);
                jcn.push(mask[c]);
                continue;
            }
            for jj in ia[c]..ia[c + 1] {
                let cc = ja[jj as usize] as usize;
                if cc != i && mask[cc] >= 0 {
                    irn.push(mask[i]);
                    jcn.push(mask[cc]);
                }
            }
        }
    }
    let id = id as usize;
    from_coordinate_arrays(irn.len(), id, id, &irn, &jcn, None, true)
}

/// `remove_overlap(dim, A, x, label_sizes, ctrl->overlap, ctrl->initial_scaling,
/// ctrl->edge_labeling_scheme, n_edge_label_nodes, edge_label_nodes, A, ctrl->do_shrinking)`.
fn remove_overlap_ctrl(
    dim: usize,
    a: &mut SparseMatrix,
    x: &mut [f64],
    label_sizes: Option<&[f64]>,
    ctrl: &SpringElectricalControl,
    edge_label_nodes: &[usize],
) {
    let Some(ls) = label_sizes else { return };
    let constraints = RelativePositionConstraints {
        constr_penalty: 1.0,
        edge_labeling_scheme: ctrl.edge_labeling_scheme,
        constr_nodes: edge_label_nodes.to_vec(),
        a_constr: a.clone(),
    };
    remove_overlap_el(
        dim,
        a,
        x,
        ls,
        ctrl.overlap,
        ctrl.initial_scaling,
        Some(&constraints),
        ctrl.do_shrinking,
    );
}

/// `multilevel_spring_electrical_embedding(dim, A0, ctrl, label_sizes, x, ...)`.
pub fn multilevel_spring_electrical_embedding(
    dim: usize,
    a0: &mut SparseMatrix,
    ctrl: &mut SpringElectricalControl,
    label_sizes: Option<&[f64]>,
    x: &mut [f64],
    edge_label_nodes: &[usize],
    flag: &mut i32,
) {
    let ctrl0 = *ctrl;
    *flag = 0;
    let n = a0.n;
    if n == 0 || dim == 0 {
        return;
    }
    let mut a = if !a0.is_symmetric(false) || !a0.is_real() {
        a0.real_adjacency_symmetrized()
    } else {
        // In place in C: A0 loses its diagonal.
        a0.remove_diagonal();
        a0.clone()
    };

    if (ctrl.edge_labeling_scheme == ELSCHEME_STRAIGHTLINE_PENALTY
        || ctrl.edge_labeling_scheme == ELSCHEME_STRAIGHTLINE_PENALTY2)
        && !edge_label_nodes.is_empty()
    {
        let mut x2 = vec![0.0f64; a.m * dim];
        let mut a2 = shorting_edge_label_nodes(&a, edge_label_nodes);
        multilevel_spring_electrical_embedding(dim, &mut a2, ctrl, None, &mut x2, &[], flag);
        attach_edge_label_coordinates(dim, &a, edge_label_nodes, x, &x2);
        remove_overlap_ctrl(dim, &mut a, x, label_sizes, ctrl, edge_label_nodes);
        return;
    }

    let mctrl = MultilevelControl {
        maxlevel: ctrl.multilevels,
    };
    let grid = multilevel_new(&a, mctrl);
    let mut lvl = grid.len() - 1;
    let mut xc: Vec<f64> = if lvl == 0 {
        x[..grid[0].n * dim].to_vec()
    } else {
        vec![0.0f64; grid[lvl].n * dim]
    };
    let plg = power_law_graph(&a);
    if ctrl.p == AUTOP {
        ctrl.p = -1.0;
        if plg {
            ctrl.p = -1.8;
        }
    }
    loop {
        let ga = &grid[lvl].a;
        if ctrl.tscheme == QUAD_TREE_NONE {
            spring_electrical_embedding_slow(dim, ga, ctrl, &mut xc, flag);
        } else if ctrl.tscheme == QUAD_TREE_FAST
            || (ctrl.tscheme == QUAD_TREE_HYBRID && ga.m > QUAD_TREE_HYBRID_SIZE)
        {
            if ctrl.tscheme == QUAD_TREE_HYBRID && ga.m > 10 {
                graphviz_pure_core::verbose_msg!(
                    "QUAD_TREE_HYBRID, size larger than {QUAD_TREE_HYBRID_SIZE}, switch to fast quadtree"
                );
            }
            spring_electrical_embedding_fast(dim, ga, ctrl, &mut xc, flag);
        } else {
            spring_electrical_embedding(dim, ga, ctrl, &mut xc, flag);
        }
        if lvl == 0 {
            break;
        }
        if *flag != 0 {
            *ctrl = ctrl0;
            return;
        }
        let p = grid[lvl].p.as_ref().expect("prolongation");
        lvl -= 1;
        let mut xf = vec![0.0f64; grid[lvl].n * dim];
        let r = grid[lvl].r.as_ref().expect("restriction");
        prolongate(dim, &grid[lvl].a, p, r, &xc, &mut xf, ctrl.k * 0.001);
        xc = xf;
        ctrl.random_start = false;
        ctrl.k *= 0.75;
        ctrl.adaptive_cooling = false;
        ctrl.step = 0.1;
    }
    x[..xc.len()].copy_from_slice(&xc);

    post_process_smoothing(dim, &mut a, ctrl, x);
    graphviz_pure_core::verbose_msg!("ctrl->overlap={}\n", ctrl.overlap);

    if dim == 2 {
        pcp_rotate(n, dim, x);
    }
    if ctrl.rotation != 0.0 {
        // C's `rotate` asserts `dim == 2` and aborts on `dim=3` or `dimen=3`
        // (#273). The port lays out as without `rotation`, with a warning.
        if dim == 2 {
            rotate(n, dim, x, ctrl.rotation);
        } else {
            graphviz_pure_core::diag::agwarningf(&format!(
                "sfdp: rotation needs a 2-dimensional layout (dim={dim}); the layout is not rotated\n"
            ));
        }
    }
    remove_overlap_ctrl(dim, &mut a, x, label_sizes, ctrl, edge_label_nodes);
    *ctrl = ctrl0;
}
