//! Port of `lib/neatogen/quad_prog_vpsc.c`: the VPSC environment for
//! constrained stress majorization (`initCMajVPSC`,
//! `constrained_majorization_vpsc`, `generateNonoverlapConstraints`,
//! `deleteCMajEnvVPSC`), `removeoverlaps` (used by `overlap=vpsc`) and the
//! DiG-CoLa level helpers.

use crate::vpsc::{self, BoxF, Solver};

/// `cluster_data`.
#[derive(Debug, Clone, Default)]
pub struct ClusterData {
    /// Total count of vars in clusters.
    pub nvars: usize,
    pub clusters: Vec<Vec<usize>>,
    pub toplevel: Vec<usize>,
    pub bb: Vec<BoxF>,
}

impl ClusterData {
    pub fn nclusters(&self) -> usize {
        self.clusters.len()
    }
}

/// `ipsep_options`.
#[derive(Debug, Clone, Default)]
pub struct IpsepOptions {
    /// 1 = directed edge constraints, 2 = DiG-CoLa level constraints.
    pub diredges: i32,
    pub edge_gap: f64,
    /// 1 = non-overlap constraints during layout, 2 = remove afterwards.
    pub noverlap: i32,
    pub gap: (f64, f64),
    /// Node widths and heights.
    pub nsize: Vec<(f64, f64)>,
    pub clusters: ClusterData,
}

/// `DigColaLevel`.
#[derive(Debug, Clone, Default)]
pub struct DigColaLevel {
    pub nodes: Vec<i32>,
}

/// `CMajEnvVPSC`.
#[derive(Debug, Clone, Default)]
pub struct CMajEnvVpsc {
    pub a: Vec<Vec<f32>>,
    /// Number of actual vars.
    pub nv: usize,
    /// Dummy nodes included in the Laplacian (cluster boundaries).
    pub nldv: usize,
    /// Dummy nodes not included in the Laplacian (DiG-CoLa boundaries).
    pub ndv: usize,
    pub vs: Vec<usize>,
    /// Total number of constraints for the next iteration (`cs.len()`).
    pub m: usize,
    /// Number of global constraints (a prefix of `cs`).
    pub gm: usize,
    pub cs: Vec<usize>,
    pub gcs: Vec<usize>,
    pub solver: Solver,
    pub f_array1: Vec<f32>,
    pub f_array2: Vec<f32>,
    pub f_array3: Vec<f32>,
    /// Not in C: how many `satisfyVPSC` calls have thrown "Cycle Error!"
    /// (see [`Self::satisfy_place`]).
    pub cycle_errors: u32,
}

const QUAD_PROG_TOL: f32 = 1e-4;

/// Not in C: the "Cycle Error!" failures of `satisfyVPSC` one ipsep layout may
/// have before it is ended, at the next one (#197). Each one is 10001 block
/// splits. Before #197, the most a corpus file reached while finishing within
/// 10 s was 241 (`tests/wild/gv16_dirgraph_apache_aef3b0.gv` with
/// `diredgeconstraints`).
pub const MAX_CYCLE_ERRORS: u32 = 500;

/// `unpackMatrix(packedMat, n)` from `matrix_ops.c`.
pub fn unpack_matrix(packed: &[f32], n: usize) -> Vec<Vec<f32>> {
    let mut mat = vec![vec![0.0f32; n]; n];
    let mut k = 0;
    for i in 0..n {
        for j in i..n {
            let v = packed[k];
            mat[i][j] = v;
            mat[j][i] = v;
            k += 1;
        }
    }
    mat
}

impl CMajEnvVpsc {
    fn satisfy_place(&mut self, place: &mut [f32], n: usize) {
        for i in 0..n {
            self.solver.vars[self.vs[i]].desired_position = place[i] as f64;
        }
        // IncVPSC::satisfy (lib/vpsc/solve_VPSC.cpp) throws a
        // std::runtime_error ("Cycle Error!" after 10000 in-block splits,
        // "Unsatisfied constraint") on an infeasible or cyclic constraint set,
        // e.g. a node in two clusters (#33). satisfyVPSC
        // (lib/vpsc/csolve_VPSC.cpp) catches only `const char *`, so C
        // aborts. The port models VPSC's pointer order as allocation order and
        // can fail where C, on some runs, succeeds, so it reads the positions
        // back and goes on. It counts cycle errors, the failures a runaway
        // repeats, so that the layout can end (#197).
        if let Err(e) = self.solver.satisfy()
            && e == vpsc::VpscError::CYCLE
        {
            self.cycle_errors += 1;
        }
        for i in 0..n {
            place[i] = self.solver.position(self.vs[i]) as f32;
        }
    }

    /// `constrained_majorization_vpsc(e, b, place, max_iterations)`:
    /// gradient projection; returns the iteration count.
    pub fn constrained_majorization_vpsc(
        &mut self,
        b: &[f32],
        place: &mut [f32],
        max_iterations: i32,
    ) -> i32 {
        let n = self.nv + self.nldv;
        let mut converged = false;
        if max_iterations == 0 {
            return 0;
        }
        let mut g = std::mem::take(&mut self.f_array1);
        let mut old_place = std::mem::take(&mut self.f_array2);
        let mut d = std::mem::take(&mut self.f_array3);
        if self.m > 0 {
            self.satisfy_place(place, n);
        }
        let mut counter = 0;
        while counter < max_iterations && !converged && self.cycle_errors <= MAX_CYCLE_ERRORS {
            let mut test = 0.0f32;
            let mut numerator = 0.0f32;
            let mut denominator = 0.0f32;
            converged = true;
            // find steepest descent direction
            for i in 0..n {
                old_place[i] = place[i];
                g[i] = 2.0 * b[i];
                for j in 0..n {
                    g[i] = (-(2.0 * self.a[i][j])).mul_add(place[j], g[i]);
                }
            }
            for i in 0..n {
                numerator = g[i].mul_add(g[i], numerator);
                let mut r = 0.0f32;
                for j in 0..n {
                    r = (2.0 * self.a[i][j]).mul_add(g[j], r);
                }
                denominator = (-r).mul_add(g[i], denominator);
            }
            let alpha = if denominator != 0.0 {
                numerator / denominator
            } else {
                1.0
            };
            for i in 0..n {
                place[i] = (-alpha).mul_add(g[i], place[i]);
            }
            if self.m > 0 {
                // project to constraint boundary
                self.satisfy_place(place, n);
            }
            for i in 0..n {
                d[i] = place[i] - old_place[i];
            }
            // now compute beta
            numerator = 0.0;
            denominator = 0.0;
            for i in 0..n {
                numerator = g[i].mul_add(d[i], numerator);
                let mut r = 0.0f32;
                for j in 0..n {
                    r = (2.0 * self.a[i][j]).mul_add(d[j], r);
                }
                denominator = r.mul_add(d[i], denominator);
            }
            let beta = if denominator != 0.0 {
                numerator / denominator
            } else {
                1.0
            };
            for i in 0..n {
                // beta > 1.0 takes us back outside the feasible region;
                // beta < 0 is not useful
                if beta > 0.0 && beta < 1.0 {
                    place[i] = beta.mul_add(d[i], old_place[i]);
                }
                test += (place[i] - old_place[i]).abs();
            }
            if test > QUAD_PROG_TOL {
                converged = false;
            }
            counter += 1;
        }
        self.f_array1 = g;
        self.f_array2 = old_place;
        self.f_array3 = d;
        counter
    }
}

/// `initCMajVPSC(n, packedMat, graph, opt, 0)`.
pub fn init_cmaj_vpsc(n: usize, packed_mat: Option<&[f32]>, opt: &IpsepOptions) -> CMajEnvVpsc {
    init_cmaj_vpsc_full(n, packed_mat, &[], opt, 0).expect("no hierarchy without diredges")
}

/// `initCMajVPSC(n, packedMat, graph, opt, diredges)`: variables, global
/// constraints (`diredges` 1: one per directed edge; 2: DiG-CoLa levels with
/// a dummy variable per boundary) and cluster containment constraints.
/// `None` when the hierarchy cannot be computed (C returns NULL).
pub fn init_cmaj_vpsc_full(
    n: usize,
    packed_mat: Option<&[f32]>,
    graph: &[super::graphdata::VtxData],
    opt: &IpsepOptions,
    diredges: i32,
) -> Option<CMajEnvVpsc> {
    let mut e = CMajEnvVpsc {
        nldv: 2 * opt.clusters.nclusters(),
        ..Default::default()
    };
    e.nv = n - e.nldv;
    e.ndv = 0;
    for i in 0..n {
        let v = e.solver.new_variable(i as i32, 1.0, 1.0);
        e.vs.push(v);
    }
    if diredges == 1 {
        graphviz_pure_core::verbose_msg!("  generate edge constraints...\n");
        for i in 0..e.nv {
            let ed = graph[i].edists.as_ref().unwrap();
            for j in 1..graph[i].nedges() {
                if ed[j] > 0 {
                    let v = graph[i].edges[j] as usize;
                    let c = e.solver.new_constraint(e.vs[i], e.vs[v], opt.edge_gap);
                    e.gcs.push(c);
                }
            }
        }
    } else if diredges == 2 {
        let (ordering, ls, ndv) =
            super::digcola::compute_hierarchy(graph, e.nv, 1e-2, 1e-1, None).ok()?;
        e.ndv = ndv;
        let levels = assign_digcola_levels(&ordering, e.nv, &ls, e.ndv);
        graphviz_pure_core::verbose_msg!("Found {} DiG-CoLa boundaries\n", e.ndv);
        // create dummy vars (0 weight)
        for i in 0..e.ndv {
            let cvar = n + i;
            let v = e.solver.new_variable(cvar as i32, 1.0, 0.000001);
            e.vs.push(v);
        }
        let halfgap = opt.edge_gap;
        for i in 0..e.ndv {
            let cvar = n + i;
            // outgoing constraints for each var in level below boundary
            for &nd in &levels[i].nodes {
                let c = e
                    .solver
                    .new_constraint(e.vs[nd as usize], e.vs[cvar], halfgap);
                e.gcs.push(c);
            }
            // incoming constraints for each var in level above boundary
            for &nd in &levels[i + 1].nodes {
                let c = e
                    .solver
                    .new_constraint(e.vs[cvar], e.vs[nd as usize], halfgap);
                e.gcs.push(c);
            }
        }
        // constraints between adjacent boundary dummy vars (C loops forever
        // when there is no boundary)
        for i in 0..e.ndv.saturating_sub(1) {
            let c = e.solver.new_constraint(e.vs[n + i], e.vs[n + i + 1], 0.0);
            e.gcs.push(c);
        }
    }
    if opt.clusters.nclusters() > 0 {
        for (i, cl) in opt.clusters.clusters.iter().enumerate() {
            for &j in cl {
                let Some(&v) = e.vs.get(j) else {
                    continue;
                };
                let cl = e.vs[e.nv + 2 * i];
                let cr = e.vs[e.nv + 2 * i + 1];
                let c1 = e.solver.new_constraint(cl, v, 0.0);
                e.gcs.push(c1);
                let c2 = e.solver.new_constraint(v, cr, 0.0);
                e.gcs.push(c2);
            }
        }
    }
    e.gm = e.gcs.len();
    e.m = 0;
    e.cs.clear();
    if e.gm > 0 {
        let nvars = n + e.ndv;
        let vs = e.vs[..nvars].to_vec();
        e.solver.new_inc_vpsc(&vs, &e.gcs);
        e.m = e.gm;
        e.cs = e.gcs.clone();
    }
    if let Some(pm) = packed_mat {
        e.a = unpack_matrix(pm, n);
    }
    e.f_array1 = vec![0.0; n];
    e.f_array2 = vec![0.0; n];
    e.f_array3 = vec![0.0; n];
    graphviz_pure_core::verbose_msg!(
        "  initCMajVPSC done: {} global constraints generated.\n",
        e.m
    );
    Some(e)
}

/// `expandbb`/`EXPANDBB` for `boxf`.
fn expand_bb(b: &mut BoxF, o: &BoxF) {
    b.llx = b.llx.min(o.llx);
    b.lly = b.lly.min(o.lly);
    b.urx = b.urx.max(o.urx);
    b.ury = b.ury.max(o.ury);
}

/// `generateNonoverlapConstraints(e, nsizeScale, coords, k, transitiveClosure, opt)`.
pub fn generate_nonoverlap_constraints(
    e: &mut CMajEnvVpsc,
    mut nsize_scale: f32,
    coords: &[Vec<f32>],
    k: usize,
    transitive_closure: bool,
    opt: &mut IpsepOptions,
) -> bool {
    let mut n = e.nv + e.nldv;
    // C allocates `bb` before taking the cluster dummies off `n`. When
    // `removeoverlaps` passes only the real nodes, cluster members among the
    // last `2 * nclusters` of them read these zeroed boxes (#185).
    let nbb = n;
    let genclusters = opt.clusters.nclusters() > 0;
    if genclusters {
        // n is the number of real variables, not dummy cluster vars
        n -= 2 * opt.clusters.nclusters();
    }
    if k == 0 {
        // grow a bit in the x dimension, so that if overlap is resolved
        // horizontally then it won't be considered overlapping vertically
        nsize_scale *= 1.0001f32;
    }
    let s = nsize_scale as f64;
    let mut bb = vec![BoxF::default(); nbb];
    for i in 0..n {
        let (nx, ny) = opt.nsize[i];
        bb[i].llx = coords[0][i] as f64 - s * nx / 2.0 - opt.gap.0 / 2.0;
        bb[i].urx = coords[0][i] as f64 + s * nx / 2.0 + opt.gap.0 / 2.0;
        bb[i].lly = coords[1][i] as f64 - s * ny / 2.0 - opt.gap.1 / 2.0;
        bb[i].ury = coords[1][i] as f64 + s * ny / 2.0 + opt.gap.1 / 2.0;
    }
    let csol: Vec<usize> = if genclusters {
        let ncl = opt.clusters.nclusters();
        let mut cscl: Vec<Vec<usize>> = Vec::with_capacity(ncl + 1);
        if opt.clusters.bb.len() < ncl {
            opt.clusters.bb.resize(ncl, BoxF::default());
        }
        for i in 0..ncl {
            let cn = opt.clusters.clusters[i].len();
            let mut cvs = Vec::with_capacity(cn + 2);
            let mut cbb = Vec::with_capacity(cn + 2);
            let mut container = BoxF {
                llx: f64::MAX,
                lly: f64::MAX,
                urx: -f64::MAX,
                ury: -f64::MAX,
            };
            for j in 0..cn {
                let iv = opt.clusters.clusters[i][j];
                cvs.push(e.vs[iv]);
                cbb.push(bb[iv]);
                expand_bb(&mut container, &bb[iv]);
            }
            opt.clusters.bb[i] = container;
            cvs.push(e.vs[n + 2 * i]);
            cvs.push(e.vs[n + 2 * i + 1]);
            cbb.push(container);
            cbb.push(container);
            if k == 0 {
                cbb[cn].urx = container.llx + 0.0001;
                cbb[cn + 1].llx = container.urx - 0.0001;
                let Some(c) =
                    vpsc::gen_x_constraints(&mut e.solver, &cbb, &cvs, transitive_closure)
                else {
                    return false;
                };
                cscl.push(c);
            } else {
                cbb[cn].ury = container.lly + 0.0001;
                cbb[cn + 1].lly = container.ury - 0.0001;
                let Some(c) = vpsc::gen_y_constraints(&mut e.solver, &cbb, &cvs) else {
                    return false;
                };
                cscl.push(c);
            }
        }
        // generate top level constraints
        {
            let ntop = opt.clusters.toplevel.len();
            let cn = ntop + ncl;
            let mut cvs = Vec::with_capacity(cn);
            let mut cbb = Vec::with_capacity(cn);
            for i in 0..ntop {
                let iv = opt.clusters.toplevel[i];
                cvs.push(e.vs[iv]);
                cbb.push(bb[iv]);
            }
            // make dummy variables for clusters
            for i in ntop..cn {
                let v = e.solver.new_variable(123 + i as i32, 1.0, 1.0);
                cvs.push(v);
                cbb.push(opt.clusters.bb[i - ntop]);
            }
            if k == 0 {
                let Some(c) =
                    vpsc::gen_x_constraints(&mut e.solver, &cbb, &cvs, transitive_closure)
                else {
                    return false;
                };
                cscl.push(c);
            } else {
                let Some(c) = vpsc::gen_y_constraints(&mut e.solver, &cbb, &cvs) else {
                    return false;
                };
                cscl.push(c);
            }
            // remap constraints from tmp dummy vars to cluster l and r vars
            for i in ntop..cn {
                let j = i - ntop;
                let dgap = if k == 0 {
                    -(cbb[i].urx - cbb[i].llx) / 2.0
                } else {
                    -(cbb[i].ury - cbb[i].lly) / 2.0
                };
                e.solver.remap_in_constraints(cvs[i], e.vs[n + 2 * j], dgap);
                e.solver
                    .remap_out_constraints(cvs[i], e.vs[n + 2 * j + 1], dgap);
            }
        }
        cscl.into_iter().flatten().collect()
    } else if k == 0 {
        let vs = e.vs[..n].to_vec();
        let Some(c) = vpsc::gen_x_constraints(&mut e.solver, &bb, &vs, transitive_closure) else {
            return false;
        };
        c
    } else {
        let vs = e.vs[..n].to_vec();
        let Some(c) = vpsc::gen_y_constraints(&mut e.solver, &bb, &vs) else {
            return false;
        };
        c
    };
    // remove constraints from previous iteration
    if e.m > 0 {
        e.solver.delete_vpsc();
        for i in e.gm..e.m {
            e.solver.delete_constraint(e.cs[i]);
        }
    }
    let mut cs = e.gcs[..e.gm].to_vec();
    cs.extend_from_slice(&csol);
    e.m = cs.len();
    e.cs = cs;
    let nvars = e.nv + e.nldv + e.ndv;
    let vs = e.vs[..nvars].to_vec();
    let cs = e.cs.clone();
    graphviz_pure_core::verbose_msg!("  generated {} constraints\n", e.m);
    e.solver.new_inc_vpsc(&vs, &cs);
    true
}

/// `removeoverlaps(n, coords, opt)`: statically remove all overlaps, moving
/// each node as little as possible (x pass, then y pass).
///
/// Not in C: false where C's `Rectangle` or `Node` asserts (`overlap=vpsc` with a NaN,
/// infinite or huge pin), and C aborts. `coords` may then hold the x pass only:
/// `vpsc_adjust` drops them, and ipsep's final pass keeps them.
pub fn removeoverlaps(n: usize, coords: &mut [Vec<f32>], opt: &mut IpsepOptions) -> bool {
    let mut e = init_cmaj_vpsc(n, None, opt);
    if !generate_nonoverlap_constraints(&mut e, 1.0, coords, 0, true, opt) {
        return false;
    }
    let _ = e.solver.solve();
    for i in 0..n {
        coords[0][i] = e.solver.position(e.vs[i]) as f32;
    }
    if !generate_nonoverlap_constraints(&mut e, 1.0, coords, 1, false, opt) {
        return false;
    }
    let _ = e.solver.solve();
    for i in 0..n {
        coords[1][i] = e.solver.position(e.vs[i]) as f32;
    }
    true
}

/// `assign_digcola_levels(ordering, n, level_inds, num_divisions)`.
pub fn assign_digcola_levels(
    ordering: &[i32],
    n: usize,
    level_inds: &[i32],
    num_divisions: usize,
) -> Vec<DigColaLevel> {
    let mut l = vec![DigColaLevel::default(); num_divisions + 1];
    // first level
    let n0 = level_inds.first().copied().unwrap_or(n as i32) as usize;
    l[0].nodes = ordering[..n0].to_vec();
    // second through second last level
    for i in 1..num_divisions {
        let (a, b) = (level_inds[i - 1] as usize, level_inds[i] as usize);
        l[i].nodes = ordering[a..b].to_vec();
    }
    // last level
    if num_divisions > 0 {
        let a = level_inds[num_divisions - 1] as usize;
        l[num_divisions].nodes = ordering[a..n].to_vec();
    }
    l
}

/// `get_num_digcola_constraints(levels, num_levels)`.
pub fn get_num_digcola_constraints(levels: &[DigColaLevel], num_levels: usize) -> usize {
    let mut nc = 0;
    for i in 1..num_levels {
        nc += levels[i].nodes.len() + levels[i - 1].nodes.len();
    }
    nc += levels[0].nodes.len() + levels[num_levels - 1].nodes.len();
    nc
}
