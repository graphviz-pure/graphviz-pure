//! Stochastic gradient descent layout (`mode=sgd`), ported from
//! `lib/neatogen/sgd.c` with the Mersenne Twister of `randomkit.c`.

use super::bfs::{GraphSgd, TermSgd, dijkstra_sgd};
use super::stress::is_fixed;
use super::{INIT_RANDOM, MODEL_SUBSET, NeatoParams};
use crate::common::types::{LGraph, Sub};
use crate::cutil::RkState;
use graphviz_pure_core::diag::{elapsed_sec, fmt_f, fprintf_stderr, start_timer, verbose_on};
use graphviz_pure_core::math;

fn extract_adjacency(g: &LGraph<'_>, sub: &Sub, model: i32) -> GraphSgd {
    let n = sub.nodes.len();
    let mut sources = vec![0usize; n + 1];
    let mut pinneds = vec![false; n];
    let mut targets = Vec::new();
    let mut weights = Vec::new();
    for (i, &np) in sub.nodes.iter().enumerate() {
        sources[i] = targets.len();
        pinneds[i] = is_fixed(g, np);
        for e in g.all_edges(sub, np) {
            let ed = &g.edges[e];
            if ed.tail == ed.head {
                continue;
            }
            let t = if ed.tail == np { ed.head } else { ed.tail };
            targets.push(g.nodes[t].id);
            weights.push(ed.dist as f32);
        }
    }
    sources[n] = targets.len();
    if model == MODEL_SUBSET {
        let mut ni = vec![false; n];
        let mut nj = vec![false; n];
        for i in 0..n {
            let mut deg_i = 0;
            for x in sources[i]..sources[i + 1] {
                let j = targets[x];
                if !ni[j] {
                    ni[j] = true;
                    deg_i += 1;
                }
            }
            for x in sources[i]..sources[i + 1] {
                let j = targets[x];
                let (mut intersect, mut deg_j) = (0, 0);
                for y in sources[j]..sources[j + 1] {
                    let k = targets[y];
                    if !nj[k] {
                        nj[k] = true;
                        deg_j += 1;
                        if ni[k] {
                            intersect += 1;
                        }
                    }
                }
                weights[x] = (deg_i + deg_j - 2 * intersect) as f32;
                for y in sources[j]..sources[j + 1] {
                    nj[targets[y]] = false;
                }
            }
            for x in sources[i]..sources[i + 1] {
                ni[targets[x]] = false;
            }
        }
    }
    GraphSgd {
        n,
        sources,
        pinneds,
        targets,
        weights,
    }
}

/// `calculate_stress(pos, terms, n_terms)`.
fn calculate_stress(pos: &[f64], terms: &[TermSgd]) -> f64 {
    let mut stress = 0.0f64;
    let at = |k: usize, d: usize| {
        pos.get(k.wrapping_mul(2).wrapping_add(d))
            .copied()
            .unwrap_or_default()
    };
    for t in terms {
        let (i, j) = (t.i as usize, t.j as usize);
        let dx = at(i, 0) - at(j, 0);
        let dy = at(i, 1) - at(j, 1);
        let r = math::hypot(dx, dy) - f64::from(t.d);
        stress = f64::from(t.w).mul_add(r * r, stress);
    }
    stress
}

/// `sgd(G, model)`.
pub fn sgd(g: &mut LGraph<'_>, sub: &Sub, model: i32, p: &NeatoParams) {
    if model == super::MODEL_CIRCUIT {
        graphviz_pure_core::diag::agwarningf(
            "circuit model not yet supported in Gmode=sgd, reverting to shortpath model\n",
        );
    }
    if model == super::MODEL_MDS {
        graphviz_pure_core::diag::agwarningf(
            "mds model not yet supported in Gmode=sgd, reverting to shortpath model\n",
        );
    }
    let model = if model == MODEL_SUBSET {
        MODEL_SUBSET
    } else {
        super::MODEL_SHORTPATH
    };
    let n = sub.nodes.len();
    if verbose_on() {
        fprintf_stderr("calculating shortest paths and setting up stress terms:");
        start_timer();
    }
    let graph = extract_adjacency(g, sub, model);
    let mut terms: Vec<TermSgd> = Vec::new();
    for i in 0..n {
        if !is_fixed(g, sub.nodes[i]) {
            dijkstra_sgd(&graph, i, &mut terms);
        }
    }
    let n_terms = terms.len();
    graphviz_pure_core::verbose_msg!(" {} sec\n", fmt_f(elapsed_sec(), 2));
    // With no terms (every node fixed, or one node) C reads `terms[0]` of a `calloc(0)`,
    // zeroed memory in practice, and goes on: the positions, the messages, no moves.
    let first = terms.first().map_or(0.0, |t| t.w);
    let (mut w_min, mut w_max) = (first, first);
    for t in terms.iter().skip(1) {
        w_min = w_min.min(t.w);
        w_max = w_max.max(t.w);
    }
    let eta_max = 1.0 / f64::from(w_min);
    let eta_min = p.epsilon / f64::from(w_max);
    let lambda = math::ln(eta_max / eta_min) / (p.max_iter - 1) as f64;
    // initial_positions(G, n)
    graphviz_pure_core::verbose_msg!("Setting initial positions\n");
    let init = super::neatoinit::check_start(g, sub, n, INIT_RANDOM, p);
    super::stuff::warn_start_self(init);
    if init != super::INIT_REGULAR {
        for &np in &sub.nodes {
            if super::stress::has_pos(g, np) {
                continue;
            }
            // randompos(np, 1): x, y, then jitter3d's third and later coordinates.
            for v in g.nodes[np].pos.iter_mut() {
                *v = crate::cutil::drand48();
            }
        }
    }
    let mut pos = vec![0.0f64; 2 * n];
    let mut unfixed = vec![false; n];
    for (i, &np) in sub.nodes.iter().enumerate() {
        pos[2 * i] = g.nodes[np].pos[0];
        pos[2 * i + 1] = g.nodes[np].pos[1];
        unfixed[i] = !is_fixed(g, np);
    }
    if verbose_on() {
        fprintf_stderr("solving model:");
        start_timer();
    }
    let mut rstate = RkState::new(0);
    for t in 0..p.max_iter {
        // fisheryates_shuffle: `n_terms > 0 && i >= 1`
        let mut i = n_terms.saturating_sub(1);
        while n_terms > 0 && i >= 1 {
            let j = rstate.interval(i as u64) as usize;
            terms.swap(i, j);
            i -= 1;
        }
        let eta = eta_max * math::exp(-lambda * t as f64);
        for term in &terms {
            let mu = (eta * f64::from(term.w)).min(1.0);
            let (ti, tj) = (term.i as usize, term.j as usize);
            let dx = pos[2 * ti] - pos[2 * tj];
            let dy = pos[2 * ti + 1] - pos[2 * tj + 1];
            let mag = math::hypot(dx, dy);
            let r = (mu * (mag - f64::from(term.d))) / (2.0 * mag);
            let (rx, ry) = (r * dx, r * dy);
            if unfixed[ti] {
                pos[2 * ti] -= rx;
                pos[2 * ti + 1] -= ry;
            }
            if unfixed[tj] {
                pos[2 * tj] += rx;
                pos[2 * tj + 1] += ry;
            }
        }
        if verbose_on() {
            fprintf_stderr(&format!(" {}", fmt_f(calculate_stress(&pos, &terms), 3)));
        }
    }
    graphviz_pure_core::verbose_msg!("\nfinished in {} sec\n", fmt_f(elapsed_sec(), 2));
    for (i, &np) in sub.nodes.iter().enumerate() {
        g.nodes[np].pos[0] = pos[2 * i];
        g.nodes[np].pos[1] = pos[2 * i + 1];
    }
}
