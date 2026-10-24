//! Generalised connected components of a derived graph, ported from
//! `lib/fdpgen/comp.c` (`findCComp`). Components containing a port node or a
//! pinned node are merged into the first component.

use super::layout::{Comp, DG};
use crate::common::types::P_PIN;

/// `dfs(g, n, out, marks)`.
fn dfs(dg: &DG, n: usize, out: &mut Vec<usize>, marks: &mut [bool]) {
    let mut stack = vec![n];
    marks[n] = true;
    while let Some(v) = stack.pop() {
        out.push(v);
        for e in dg.out[v].iter().chain(dg.inn[v].iter()) {
            let ed = &dg.edges[*e];
            let other = if ed.t == v { ed.h } else { ed.t };
            if !marks[other] {
                marks[other] = true;
                stack.push(other);
            }
        }
    }
}

/// `findCComp(g, &pinned, counter)`: the components and whether a pinned
/// node exists.
pub fn find_ccomp(dg: &DG, counter: &mut usize) -> (Vec<Comp>, bool) {
    let n = dg.nodes.len();
    let mut marks = vec![false; n];
    let mut comps = Vec::new();
    let mut first: Option<(Vec<usize>, bool)> = None;
    if dg.has_ports {
        let mut v = Vec::new();
        for p in &dg.ports {
            if marks[p.n] {
                continue;
            }
            dfs(dg, p.n, &mut v, &mut marks);
        }
        first = Some((v, true));
    }
    let mut pinflag = false;
    for i in 0..n {
        if marks[i] || dg.nodes[i].pinned != P_PIN {
            continue;
        }
        let f = first.get_or_insert_with(|| (Vec::new(), false));
        pinflag = true;
        dfs(dg, i, &mut f.0, &mut marks);
    }
    if let Some((v, ports)) = first {
        comps.push(Comp::new(n, v, ports));
    }
    for i in 0..n {
        if marks[i] {
            continue;
        }
        let mut v = Vec::new();
        dfs(dg, i, &mut v, &mut marks);
        comps.push(Comp::new(n, v, false));
    }
    // The derived graph is named "derived". The ports' or pinned nodes' component is `_`,
    // the others `+`; the numbers go on across the layout's derived graphs.
    let first_special = dg.has_ports || pinflag;
    for (c, comp) in comps.iter_mut().enumerate() {
        let sep = if c == 0 && first_special { '_' } else { '+' };
        comp.name = format!("ccderived{sep}{}", c.wrapping_add(*counter));
    }
    *counter = counter.wrapping_add(comps.len());
    (comps, pinflag)
}
