//! `lib/circogen/blockpath.c`: lay out a single block on a circle.

use super::circular::{Circ, NEIGHBOR_F, PARENT_F, PATH_F, VISITED_F};
use super::graph::CGraph;
use super::nodelist::{append_nodelist, insert_nodelist, realign_nodelist, reverse_append};
use crate::cutil::qsort_by;
use graphviz_pure_core::math;
use std::collections::HashMap;
use std::f64::consts::PI;

/// `DEGREE(n)` (`ND_order`) of the clone nodes.
type Degrees = HashMap<usize, i32>;

/// `clone_graph(ing, &xg, state)`: a subgraph copy of `ing` (returned) and a
/// separate root copy `xg` whose edges point back at `ing`'s (`ORIGE`).
fn clone_graph(c: &mut Circ, ing: &CGraph, deg: &mut Degrees) -> (CGraph, CGraph) {
    let mut clone = CGraph::default();
    let mut xclone = CGraph::default();
    for n in ing.nodes() {
        clone.add_node(n);
        xclone.add_node(n);
        // `agbindrec` zeroes `DEGREE` (`ND_order`), so a node with no edge
        // in the block still has degree 0: blocktree puts an articulation
        // point in only one of its blocks, which can leave a block whose
        // nodes share no edge.
        deg.insert(n, 0);
    }
    for n in ing.nodes() {
        for e in ing.outs(n) {
            clone.subedge(&c.arena, e);
            let xh = c.arena.head[e];
            let xe = xclone.edge(&mut c.arena, n, xh);
            c.arena.orig[xe] = Some(e);
            *deg.entry(n).or_insert(0) += 1;
            *deg.entry(xh).or_insert(0) += 1;
        }
    }
    (clone, xclone)
}

/// `cmpDegree`: by descending degree.
fn cmp_degree(deg: &Degrees, a: usize, b: usize) -> i32 {
    let (da, db) = (deg[&a], deg[&b]);
    if da < db {
        1
    } else if da > db {
        -1
    } else {
        0
    }
}

/// `getList(g)`: the nodes sorted by descending degree.
fn get_list(g: &CGraph, deg: &Degrees) -> Vec<usize> {
    let mut dl = g.nodes();
    qsort_by(&mut dl, |a, b| cmp_degree(deg, *a, *b));
    dl
}

/// `find_pair_edges(g, n, outg)`.
fn find_pair_edges(c: &mut Circ, g: &mut CGraph, n: usize, outg: &mut CGraph, deg: &mut Degrees) {
    let mut edge_cnt = 0i32;
    let node_degree = deg[&n];
    let mut neighbors_with: Vec<usize> = Vec::new();
    let mut neighbors_without: Vec<usize> = Vec::new();
    let edges = g.edges(n);
    for &e in &edges {
        let n1 = c.arena.other(e, n);
        let mut has_pair_edge = false;
        for &ep in &edges {
            if ep == e {
                continue;
            }
            let n2 = c.arena.other(ep, n);
            if let Some(ex) = g.find_edge(n1, n2) {
                has_pair_edge = true;
                // C counts each pair once by comparing node addresses.
                if n1 < n2 {
                    edge_cnt += 1;
                    if let Some(o) = c.arena.orig[ex] {
                        outg.delete_edge(&c.arena, o);
                        c.arena.orig[ex] = None;
                    }
                }
            }
        }
        if has_pair_edge {
            neighbors_with.push(n1);
        } else {
            neighbors_without.push(n1);
        }
    }

    let mut diff = node_degree - 1 - edge_cnt;
    if diff > 0 {
        let nw = neighbors_without.len();
        if (diff as usize) < nw {
            let mut mark = 0;
            while mark + 1 < nw {
                let tp = neighbors_without[mark];
                let hp = neighbors_without[mark + 1];
                g.edge(&mut c.arena, tp, hp);
                *deg.get_mut(&tp).unwrap() += 1;
                *deg.get_mut(&hp).unwrap() += 1;
                diff -= 1;
                mark += 2;
            }
            let mut mark = 2;
            while diff > 0 {
                let tp = neighbors_without[0];
                let hp = neighbors_without[mark];
                g.edge(&mut c.arena, tp, hp);
                *deg.get_mut(&tp).unwrap() += 1;
                *deg.get_mut(&hp).unwrap() += 1;
                mark += 1;
                diff -= 1;
            }
        } else if diff as usize == nw {
            // C passes a NULL tail when no neighbor has a pair edge; that
            // cannot happen for a node of degree > 1.
            if let Some(&tp) = neighbors_with.first() {
                for &hp in &neighbors_without {
                    g.edge(&mut c.arena, tp, hp);
                    *deg.get_mut(&tp).unwrap() += 1;
                    *deg.get_mut(&hp).unwrap() += 1;
                }
            }
        }
    }
}

/// `remove_pair_edges(ing, state)`: the layout skeleton of `ing`.
fn remove_pair_edges(c: &mut Circ, ing: &CGraph) -> CGraph {
    let mut deg = Degrees::new();
    let (mut outg, mut g) = clone_graph(c, ing, &mut deg);
    let node_count = g.nnodes() as i64;
    let mut dl = get_list(&g, &deg);

    let mut counter = 0i64;
    while counter < node_count - 3 {
        let currnode = dl.pop().expect("degree list");
        for e in g.edges(currnode) {
            let adj = c.arena.other(e, currnode);
            if let Some(i) = dl.iter().position(|&x| x == adj) {
                dl.remove(i);
            }
        }
        find_pair_edges(c, &mut g, currnode, &mut outg, &mut deg);
        for e in g.edges(currnode) {
            let adj = c.arena.other(e, currnode);
            *deg.get_mut(&adj).unwrap() -= 1;
            dl.push(adj);
        }
        qsort_by(&mut dl, |a, b| cmp_degree(&deg, *a, *b));
        g.delete_node(currnode);
        counter += 1;
    }
    outg
}

/// `measure_distance(n, ancestor, dist, change)`.
fn measure_distance(
    c: &mut Circ,
    n: usize,
    ancestor: usize,
    mut dist: i32,
    mut change: Option<usize>,
) {
    let mut ancestor = ancestor;
    loop {
        let Some(parent) = c.nd[ancestor].tparent else {
            return;
        };
        dist += 1;
        let p = &mut c.nd[parent];
        if p.distone == 0 {
            p.leafone = Some(n);
            p.distone = dist;
        } else if dist > p.distone {
            if p.leafone != change {
                if p.disttwo == 0 || p.leaftwo != change {
                    change = p.leafone;
                }
                p.leaftwo = p.leafone;
                p.disttwo = p.distone;
            }
            p.leafone = Some(n);
            p.distone = dist;
        } else if dist > p.disttwo {
            p.leaftwo = Some(n);
            p.disttwo = dist;
            return;
        } else {
            return;
        }
        ancestor = parent;
    }
}

/// `find_longest_path(tree)`.
fn find_longest_path(c: &mut Circ, tree: &CGraph) -> Vec<usize> {
    if tree.nnodes() == 1 {
        let n = tree.first_node().expect("one node");
        c.set_flag(n, PATH_F);
        return vec![n];
    }
    for n in tree.nodes() {
        if tree.edges(n).len() == 1 {
            measure_distance(c, n, n, 0, None);
        }
    }
    let mut common = None;
    let mut maxlength = 0;
    for n in tree.nodes() {
        let length = c.nd[n].distone + c.nd[n].disttwo;
        if length > maxlength {
            common = Some(n);
            maxlength = length;
        }
    }
    let Some(common) = common else {
        return Vec::new();
    };
    let mut begin_path = Vec::new();
    let mut n = c.nd[common].leafone.expect("leaf");
    while n != common {
        begin_path.push(n);
        c.set_flag(n, PATH_F);
        n = c.nd[n].tparent.expect("tree parent");
    }
    begin_path.push(common);
    c.set_flag(common, PATH_F);
    if c.nd[common].disttwo != 0 {
        let mut end_path = Vec::new();
        let mut n = c.nd[common].leaftwo.expect("leaf");
        while n != common {
            end_path.push(n);
            c.set_flag(n, PATH_F);
            n = c.nd[n].tparent.expect("tree parent");
        }
        reverse_append(&mut begin_path, end_path);
    }
    begin_path
}

/// `dfs(g, n, tree)`: depth-first search adding traversed edges to `tree`.
fn dfs(c: &mut Circ, g: &CGraph, n: usize, tree: &mut CGraph) {
    c.set_flag(n, VISITED_F);
    for e in g.edges(n) {
        let neighbor = c.arena.other(e, n);
        if !c.flag(neighbor, VISITED_F) {
            tree.subedge(&c.arena, e);
            c.nd[neighbor].tparent = Some(n);
            dfs(c, g, neighbor, tree);
        }
    }
}

/// `spanning_tree(g, state)`: a spanning forest of `g` as a subgraph.
fn spanning_tree(c: &mut Circ, g: &CGraph) -> CGraph {
    let mut tree = CGraph::default();
    for n in g.nodes() {
        tree.add_node(n);
        c.nd[n].distone = 0;
        c.nd[n].disttwo = 0;
        c.unset_flag(n, VISITED_F);
    }
    for n in g.nodes() {
        if !c.flag(n, VISITED_F) {
            c.nd[n].tparent = None;
            dfs(c, g, n, &mut tree);
        }
    }
    tree
}

/// `block_graph(g, sn)`: add the induced edges to the block subgraph.
fn block_graph(c: &mut Circ, g: &CGraph, sn: usize) {
    let mut subg = std::mem::take(&mut c.blocks[sn].sub_graph);
    for n in subg.nodes() {
        for e in g.outs(n) {
            if c.nd[c.arena.head[e]].block == Some(sn) {
                subg.subedge(&c.arena, e);
            }
        }
    }
    c.blocks[sn].sub_graph = subg;
}

/// C: lib/circogen/blockpath.c:count_all_crossings, reduce_edge_crossings
/// Immutable block adjacency and scratch shared by all tentative moves.
struct Crossings {
    nodes: Vec<usize>,
    offsets: Vec<std::ops::Range<usize>>,
    edges: Vec<usize>,
    open: Vec<u64>,
}

impl Crossings {
    // LEAFONE/LEAFTWO paths may overlap: opening orders index list
    // occurrences, so the tree must cover list_len, not distinct nodes.
    fn new(c: &Circ, subg: &CGraph, list_len: usize) -> Self {
        let nodes = subg.nodes();
        let mut offsets = vec![0..0; c.nd.len()];
        let mut edges = Vec::new();
        let mut ni = nodes.iter().peekable();
        for (n, offset) in offsets.iter_mut().enumerate() {
            let start = edges.len();
            if ni.peek() == Some(&&n) {
                edges.extend(subg.edges(n));
                ni.next();
            }
            *offset = start..edges.len();
        }
        Self {
            nodes,
            offsets,
            edges,
            open: vec![
                0;
                (c.arena.order.len() << 1)
                    .div_ceil(64)
                    .saturating_add(list_len)
                    .saturating_add(1)
            ],
        }
    }
}

/// C: lib/circogen/blockpath.c:count_all_crossings
/// Exact index of the reference half-set, NOT a chord-crossing count.
/// Before each node, tree[k] counts set halves whose EDGEORDER is k. Remove
/// incident halves for the whole query phase: the reference excludes both
/// endpoints, and its intervening erasures affect only these excluded halves.
/// Restore them before applying exactly the reference erasures and insertions.
/// Thus a strict suffix query counts precisely the reference predicate, including
/// retained closed halves, loops and distinct parallel edge ids. See
/// docs/perf/circo-crossings.md for the invariant and move/overflow proof.
fn count_all_crossings(
    c: &mut Circ,
    list: &[usize],
    offsets: &[std::ops::Range<usize>],
    adjacency: &[usize],
    scratch: &mut [u64],
) -> i32 {
    count_checked::<0>(c, list, offsets, adjacency, scratch)
}

/// C: lib/circogen/blockpath.c:count_all_crossings
// MUTATION is zero in production; tests exercise broken predicates through the
// same check wrapper, so they prove that the check detects wrong queried counts.
fn count_checked<const MUTATION: u8>(
    c: &mut Circ,
    list: &[usize],
    offsets: &[std::ops::Range<usize>],
    adjacency: &[usize],
    scratch: &mut [u64],
) -> i32 {
    let (open, tree) = scratch.split_at_mut(c.arena.order.len().wrapping_shl(1).div_ceil(64));
    // Index keys must be positive opening orders. Preserve even the reference's
    // order-counter overflow behavior by using its scan beyond that range.
    if list.len() > i32::MAX as usize
        || list.len() >= tree.len()
        || (!cfg!(any(test, feature = "circo-check")) && list.len() <= 16)
    {
        return count_all_crossings_reference(c, list, offsets, adjacency, open);
    }
    let Some(result) = count_indexed::<MUTATION>(c, list, offsets, adjacency, open, tree) else {
        return count_all_crossings_reference(c, list, offsets, adjacency, open);
    };
    #[cfg(any(test, feature = "circo-check"))]
    {
        let orders = c.arena.order.clone();
        let halves = open.to_vec();
        let expected = count_all_crossings_reference(c, list, offsets, adjacency, open);
        assert_eq!(result, expected, "circo indexed crossing count");
        assert!(c.arena.order == orders, "circo edge orders differ");
        assert!(open == halves, "circo half membership differs");
    }
    result
}

/// C: lib/circogen/blockpath.c:count_all_crossings
fn count_indexed<const MUTATION: u8>(
    c: &mut Circ,
    list: &[usize],
    offsets: &[std::ops::Range<usize>],
    adjacency: &[usize],
    open: &mut [u64],
    tree: &mut [u64],
) -> Option<i32> {
    open.fill(0);
    tree.fill(0);
    for &e in adjacency {
        *c.arena.order.get_mut(e)? = 0;
    }
    let mut crossings = 0i32;
    for (&n, order) in list.iter().zip(1..) {
        let edges = adjacency.get(offsets.get(n)?.clone())?;
        // All incident edges occur once in a CGraph adjacency (loops included).
        for &e in edges {
            for half in e.wrapping_shl(1)..e.wrapping_shl(1).wrapping_add(2) {
                if MUTATION != 1 && open.get(half / 64)? & (1 << (half % 64)) != 0 {
                    index_update(tree, *c.arena.order.get(e)? as usize, false)?;
                }
            }
        }
        let total = index_prefix(tree, list.len())?;
        for &e in edges {
            let edge_order = *c.arena.order.get(e)?;
            if edge_order > 0 {
                let count = total.checked_sub(index_prefix(tree, edge_order as usize)?)?;
                #[cfg(any(test, feature = "circo-check"))]
                {
                    let expected = reference_query(c, open, edge_order, n);
                    assert_eq!(count, expected, "circo indexed crossing count (edge query)");
                }
                crossings = add_crossings(crossings, count);
            }
        }
        for &e in edges {
            for half in e.wrapping_shl(1)..e.wrapping_shl(1).wrapping_add(2) {
                if MUTATION != 1 && open.get(half / 64)? & (1 << (half % 64)) != 0 {
                    index_update(tree, *c.arena.order.get(e)? as usize, true)?;
                }
            }
            if *c.arena.order.get(e)? > 0 {
                let half =
                    e.wrapping_shl(1) | usize::from((*c.arena.tail.get(e)? != n) ^ (MUTATION == 2));
                let mask = 1 << (half % 64);
                if open.get(half / 64)? & mask != 0 {
                    if MUTATION != 3 {
                        *open.get_mut(half / 64)? &= !mask;
                    }
                    if MUTATION != 4 {
                        index_update(tree, *c.arena.order.get(e)? as usize, false)?;
                    }
                }
            }
        }
        for &e in edges {
            if *c.arena.order.get(e)? == 0 {
                *c.arena.order.get_mut(e)? = order;
                let half = e.wrapping_shl(1) | usize::from(*c.arena.tail.get(e)? != n);
                *open.get_mut(half / 64)? |= 1 << (half % 64);
                index_update(tree, order as usize, true)?;
            }
        }
        #[cfg(any(test, feature = "circo-check"))]
        assert_eq!(
            index_prefix(tree, list.len()),
            Some(open.iter().map(|word| u64::from(word.count_ones())).sum()),
            "circo indexed crossing count (half membership weight)"
        );
    }
    Some(crossings)
}

#[cfg(any(test, feature = "circo-check"))]
/// C: lib/circogen/blockpath.c:count_all_crossings (inner predicate)
fn reference_query(c: &Circ, open: &[u64], edge_order: i32, n: usize) -> u64 {
    open.iter()
        .enumerate()
        .map(|(word_index, &word)| {
            let mut bits = word;
            let mut count = 0;
            while bits != 0 {
                let ep = ((word_index << 6) | bits.trailing_zeros() as usize) / 2;
                if c.arena.order[ep] > edge_order && c.arena.head[ep] != n && c.arena.tail[ep] != n
                {
                    count += 1;
                }
                bits &= bits.wrapping_sub(1);
            }
            count
        })
        .sum()
}

// Invalid keys must hand the entire recount back to the reference. In
// particular zero has no Fenwick successor and must never enter the loop.
fn index_update(tree: &mut [u64], mut i: usize, insert: bool) -> Option<()> {
    if i == 0 || i >= tree.len() {
        return None;
    }
    while i < tree.len() {
        let value = tree.get_mut(i)?;
        *value = if insert {
            value.wrapping_add(1)
        } else {
            value.wrapping_sub(1)
        };
        i = i.wrapping_add(i & i.wrapping_neg());
    }
    Some(())
}

fn index_prefix(tree: &[u64], mut i: usize) -> Option<u64> {
    if i >= tree.len() {
        return None;
    }
    let mut sum = 0u64;
    while i > 0 {
        sum = sum.wrapping_add(*tree.get(i)?);
        i &= i.wrapping_sub(1);
    }
    Some(sum)
}

fn add_crossings(value: i32, count: u64) -> i32 {
    // The reference adds only +1: in checked builds it panics exactly when
    // the nonnegative mathematical total exceeds i32::MAX; release wraps.
    if cfg!(debug_assertions) {
        assert!(
            count <= (i32::MAX as u64).wrapping_sub(value as u64),
            "crossing count overflow"
        );
    }
    value.wrapping_add(count as i32)
}

/// C: lib/circogen/blockpath.c:count_all_crossings
fn count_all_crossings_reference(
    c: &mut Circ,
    list: &[usize],
    offsets: &[std::ops::Range<usize>],
    adjacency: &[usize],
    open: &mut [u64],
) -> i32 {
    open.fill(0);
    let mut crossings = 0;
    for &e in adjacency {
        c.arena.order[e] = 0;
    }
    for (&n, order) in list.iter().zip(1..) {
        let edges = &adjacency[offsets[n].clone()];
        for &e in edges {
            let edge_order = c.arena.order[e];
            if edge_order > 0 {
                // Ascending (edge id, half), exactly the old BTreeSet order.
                for (word_index, &word) in open.iter().enumerate() {
                    let mut bits = word;
                    while bits != 0 {
                        let half = (word_index << 6) | bits.trailing_zeros() as usize;
                        let ep = half / 2;
                        if c.arena.order[ep] > edge_order
                            && c.arena.head[ep] != n
                            && c.arena.tail[ep] != n
                        {
                            crossings += 1;
                        }
                        bits &= bits.wrapping_sub(1);
                    }
                }
                // C's two edge halves are distinct keys: closing at the other
                // end must not erase the half opened at the first endpoint.
                let half = (e << 1) | usize::from(c.arena.tail[e] != n);
                open[half / 64] &= !(1u64.wrapping_shl((half % 64) as u32));
            }
        }
        for &e in edges {
            let edge_order = &mut c.arena.order[e];
            if *edge_order == 0 {
                *edge_order = order;
                let half = (e << 1) | usize::from(c.arena.tail[e] != n);
                open[half / 64] |= 1u64.wrapping_shl((half % 64) as u32);
            }
        }
    }
    crossings
}

const CROSS_ITER: i32 = 10;

/// `reduce(list, subg, cnt)`: move nodes next to their neighbours while that
/// lowers the crossing count.
fn reduce(
    c: &mut Circ,
    mut list: Vec<usize>,
    scratch: &mut Crossings,
    cnt: &mut i32,
) -> Vec<usize> {
    let mut crossings = *cnt;
    let mut list_copy = Vec::with_capacity(list.len());
    let Crossings {
        nodes,
        offsets,
        edges,
        open,
    } = scratch;
    for &curnode in nodes.iter() {
        for &e in &edges[offsets[curnode].clone()] {
            let neighbor = c.arena.other(e, curnode);
            for j in 0..2 {
                list_copy.clone_from(&list);
                insert_nodelist(&mut list, curnode, neighbor, j);
                let new_crossings = count_all_crossings(c, &list, offsets, edges, open);
                if new_crossings < crossings {
                    crossings = new_crossings;
                    if crossings == 0 {
                        *cnt = 0;
                        return list;
                    }
                } else {
                    list.clone_from(&list_copy);
                }
            }
        }
    }
    *cnt = crossings;
    list
}

/// `reduce_edge_crossings(list, subg)`.
fn reduce_edge_crossings(c: &mut Circ, mut list: Vec<usize>, subg: &CGraph) -> Vec<usize> {
    let mut scratch = Crossings::new(c, subg, list.len());
    let mut crossings = count_all_crossings(
        c,
        &list,
        &scratch.offsets,
        &scratch.edges,
        &mut scratch.open,
    );
    if crossings == 0 {
        return list;
    }
    for _ in 0..CROSS_ITER {
        let orig_crossings = crossings;
        list = reduce(c, list, &mut scratch, &mut crossings);
        if orig_crossings == crossings || crossings == 0 {
            return list;
        }
    }
    list
}

/// `largest_nodesize(list)`: the largest width or height, inches.
fn largest_nodesize(c: &Circ, list: &[usize]) -> f64 {
    let mut size = 0.0;
    for &n in list {
        let width = c.width[n];
        if width > size {
            size = width;
        }
        let height = c.height[n];
        if height > size {
            size = height;
        }
    }
    size
}

/// `place_node(g, n, list)`: insert `n` next to its neighbours on the list.
fn place_node(c: &mut Circ, g: &CGraph, n: usize, list: &mut Vec<usize>) {
    let mut placed = false;
    let mut neighbors = Vec::new();
    for e in g.outs(n) {
        let h = c.arena.head[e];
        neighbors.push(h);
        c.set_flag(h, NEIGHBOR_F);
    }
    for e in g.ins(n) {
        let t = c.arena.tail[e];
        neighbors.push(t);
        c.set_flag(t, NEIGHBOR_F);
    }
    if neighbors.len() >= 2 {
        for one in 0..list.len() {
            let two = (one + 1) % list.len();
            if c.flag(list[one], NEIGHBOR_F) && c.flag(list[two], NEIGHBOR_F) {
                append_nodelist(list, one + 1, n);
                placed = true;
                break;
            }
        }
    }
    if !placed && !neighbors.is_empty() {
        for one in 0..list.len() {
            if c.flag(list[one], NEIGHBOR_F) {
                append_nodelist(list, one + 1, n);
                placed = true;
                break;
            }
        }
    }
    if !placed {
        list.push(n);
    }
    for &x in &neighbors {
        c.unset_flag(x, NEIGHBOR_F);
    }
}

/// `place_residual_nodes(g, list)`.
fn place_residual_nodes(c: &mut Circ, g: &CGraph, list: &mut Vec<usize>) {
    for n in g.nodes() {
        if !c.flag(n, PATH_F) {
            place_node(c, g, n, list);
        }
    }
}

/// `layout_block(g, sn, min_dist, state)`: order the nodes of block `sn`
/// and place them on a circle around the origin.
pub fn layout_block(c: &mut Circ, g: &CGraph, sn: usize, min_dist: f64) -> Vec<usize> {
    block_graph(c, g, sn);
    let subg = std::mem::take(&mut c.blocks[sn].sub_graph);

    let copy_g = remove_pair_edges(c, &subg);
    let tree = spanning_tree(c, &copy_g);
    let mut longest_path = find_longest_path(c, &tree);
    place_residual_nodes(c, &subg, &mut longest_path);
    // longest_path now holds every node of the block

    let mut longest_path = reduce_edge_crossings(c, longest_path, &subg);

    let n_len = longest_path.len();
    let largest_node = largest_nodesize(c, &longest_path);
    let radius = if n_len == 1 {
        0.0
    } else {
        n_len as f64 * (min_dist + largest_node) / (2.0 * PI)
    };

    if let Some(item) = longest_path.iter().position(|&n| c.flag(n, PARENT_F)) {
        realign_nodelist(&mut longest_path, item);
    }

    for (k, &n) in longest_path.iter().enumerate() {
        c.nd[n].position = k as i32;
        c.nd[n].psi = 0.0;
        let theta = k as f64 * (2.0 * PI / n_len as f64);
        c.pos[n] = [radius * math::cos(theta), radius * math::sin(theta)];
    }

    let b = &mut c.blocks[sn];
    b.radius = if n_len == 1 {
        largest_node / 2.0
    } else {
        radius
    };
    b.rad0 = b.radius;
    b.parent_pos = -1.0;
    b.sub_graph = subg;
    longest_path
}

#[cfg(test)]
mod tests {
    use super::super::circular::CData;
    use super::super::edgelist::{add_edge, init_edgelist, remove_edge};
    use super::super::graph::EdgeArena;
    use super::*;

    fn reference(c: &mut Circ, list: &[usize], g: &CGraph) -> i32 {
        let mut open = init_edgelist();
        let mut crossings = 0;
        for n in g.nodes() {
            for e in g.outs(n) {
                c.arena.order[e] = 0;
            }
        }
        for (i, &n) in list.iter().enumerate() {
            let edges = g.edges(n);
            for &e in &edges {
                if c.arena.order[e] > 0 {
                    for &(ep, _) in &open {
                        if c.arena.order[ep] > c.arena.order[e]
                            && c.arena.head[ep] != n
                            && c.arena.tail[ep] != n
                        {
                            crossings += 1;
                        }
                    }
                    remove_edge(&mut open, (e, c.arena.tail[e] != n));
                }
            }
            for &e in &edges {
                if c.arena.order[e] == 0 {
                    c.arena.order[e] = i as i32 + 1;
                    add_edge(&mut open, (e, c.arena.tail[e] != n));
                }
            }
        }
        crossings
    }

    fn circ() -> Circ {
        Circ {
            nd: vec![CData::default(); 7],
            pos: vec![],
            width: vec![],
            height: vec![],
            arena: EdgeArena::default(),
            blocks: vec![],
            bl: vec![],
            order_count: 1,
            min_dist: 0.0,
            rootname: None,
            n_root: false,
            node_root: vec![],
            names: vec![],
        }
    }

    #[test]
    fn crossing_sequence_matches_edge_half_set() {
        let mut c = circ();
        let mut unused = CGraph::default();
        for n in 0..30 {
            unused.edge(&mut c.arena, n, n);
        }
        let mut g = CGraph::default();
        // Edge halves straddle a bitset word boundary.
        // Mixed orientations, a loop, sparse node ids, and an isolated node.
        for (t, h) in [(5, 0), (0, 2), (2, 5), (4, 0), (2, 4), (4, 5), (2, 2)] {
            g.edge(&mut c.arena, t, h);
        }
        g.add_node(6);
        let mut scratch = Crossings::new(&c, &g, 5);
        fn permutations(c: &mut Circ, g: &CGraph, s: &mut Crossings, list: &mut [usize], i: usize) {
            if i == list.len() {
                let expected = reference(c, list, g);
                assert_eq!(
                    count_all_crossings(c, list, &s.offsets, &s.edges, &mut s.open),
                    expected,
                    "{list:?}"
                );
                return;
            }
            for j in i..list.len() {
                list.swap(i, j);
                permutations(c, g, s, list, i + 1);
                list.swap(i, j);
            }
        }
        permutations(&mut c, &g, &mut scratch, &mut [0, 2, 4, 5, 6], 0);
    }
    fn generated<const MUTATION: u8>() {
        let mut seed = 7u64;
        for _ in 0..200 {
            let mut c = circ();
            let mut adj = vec![Vec::new(); 7];
            for e in 0..40 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let t = (seed >> 32) as usize % 7;
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let h = (seed >> 32) as usize % 7;
                c.arena.tail.push(t);
                c.arena.head.push(h);
                c.arena.order.push(0);
                c.arena.orig.push(None);
                adj[t].push(e);
                if t != h {
                    adj[h].push(e);
                }
            }
            let mut edges = Vec::new();
            let offsets: Vec<_> = adj
                .iter()
                .map(|a| {
                    let start = edges.len();
                    edges.extend(a);
                    start..edges.len()
                })
                .collect();
            let mut list: Vec<_> = (0..7).chain([1, 3, 1]).collect();
            let mut scratch = vec![0; (c.arena.order.len() * 2).div_ceil(64) + list.len() + 1];
            for _ in 0..20 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let len = list.len();
                list.swap((seed >> 32) as usize % len, seed as usize % len);
                count_checked::<MUTATION>(&mut c, &list, &offsets, &edges, &mut scratch);
            }
        }
    }

    #[test]
    fn indexed_counts_match_on_multigraph_lists_with_repeats() {
        generated::<0>();
    }

    #[test]
    #[should_panic(expected = "circo indexed crossing count")]
    fn check_catches_missing_endpoint_exclusions() {
        generated::<1>();
    }

    #[test]
    #[should_panic(expected = "circo indexed crossing count")]
    fn check_catches_erasing_the_retained_half() {
        generated::<2>();
    }

    #[test]
    fn crossing_addition_overflow_matches_reference() {
        assert_eq!(add_crossings(i32::MAX - 2, 2), i32::MAX);
        if cfg!(debug_assertions) {
            assert!(std::panic::catch_unwind(|| add_crossings(i32::MAX, 1)).is_err());
        } else {
            assert_eq!(add_crossings(i32::MAX, 1), i32::MIN);
            assert_eq!(add_crossings(0, (1u64 << 32) + 3), 3);
        }
    }
    fn repeated_node<const MUTATION: u8>() {
        let mut c = circ();
        let mut g = CGraph::default();
        g.edge(&mut c.arena, 0, 4);
        g.edge(&mut c.arena, 1, 3);
        g.edge(&mut c.arena, 1, 1);
        g.add_node(2);
        let list = [0, 1, 1, 2, 3, 4];
        let mut s = Crossings::new(&c, &g, list.len());
        assert_eq!(
            count_checked::<MUTATION>(&mut c, &list, &s.offsets, &s.edges, &mut s.open),
            0
        );
    }

    #[test]
    fn repeated_nodes_erase_open_halves_and_loops() {
        repeated_node::<0>();
    }

    #[test]
    #[should_panic(expected = "circo indexed crossing count")]
    fn check_catches_erasing_only_the_bit() {
        repeated_node::<4>();
    }

    #[test]
    #[should_panic(expected = "circo indexed crossing count")]
    fn check_catches_erasing_only_the_tree() {
        repeated_node::<3>();
    }

    #[test]
    fn invalid_fenwick_keys_decline_without_hanging_or_truncating() {
        let mut tree = [0; 24];
        assert_eq!(index_update(&mut tree, 0, true), None);
        assert_eq!(index_update(&mut tree, 24, true), None);
        assert_eq!(index_prefix(&tree, 24), None);
        assert_eq!(index_update(&mut tree, 23, true), Some(()));
        assert_eq!(index_prefix(&tree, 23), Some(1));
        let mut c = circ();
        let mut g = CGraph::default();
        g.edge(&mut c.arena, 0, 4);
        g.edge(&mut c.arena, 1, 3);
        let list = [0, 1, 1, 2, 3, 4];
        g.add_node(2);
        let mut s = Crossings::new(&c, &g, list.len());
        let expected = reference(&mut c, &list, &g);
        // Truncate exactly the top slot, reproducing the reviewed capacity bug.
        s.open.pop();
        assert_eq!(
            count_checked::<0>(&mut c, &list, &s.offsets, &s.edges, &mut s.open),
            expected
        );
    }
}
