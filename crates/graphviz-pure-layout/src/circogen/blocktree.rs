//! `lib/circogen/blocktree.c`: biconnected components and the block tree.

use super::block::{append_block, block_size, insert_block};
use super::circular::{Circ, PARENT_F};
use super::graph::CGraph;

/// `addNode(bp, n)`.
fn add_node(c: &mut Circ, bp: usize, n: usize) {
    c.blocks[bp].sub_graph.add_node(n);
    c.nd[n].block = Some(bp);
}

/// `dfs(g, u, state, isRoot, stk)`: Hopcroft–Tarjan biconnected components.
///
/// An articulation point goes to its first non-trivial child block; failing
/// that, to its parent's block or a block of its own.
fn dfs(c: &mut Circ, g: &CGraph, u: usize, is_root: bool, stk: &mut Vec<usize>) {
    c.nd[u].val = c.order_count;
    c.nd[u].low_val = c.order_count;
    c.order_count += 1;
    for e in g.edges(u) {
        let mut v = c.arena.head[e];
        if v == u {
            v = c.arena.tail[e];
            if c.arena.order[e] == 0 {
                c.arena.order[e] = -1;
            }
        } else if c.arena.order[e] == 0 {
            c.arena.order[e] = 1;
        }

        if c.nd[v].val == 0 {
            c.nd[v].parent = Some(u);
            stk.push(e);
            dfs(c, g, v, false, stk);
            c.nd[u].low_val = c.nd[u].low_val.min(c.nd[v].low_val);
            if c.nd[v].low_val >= c.nd[u].val {
                // u is an articulation point
                let mut block: Option<usize> = None;
                loop {
                    let ep = stk.pop().expect("edge stack");
                    let np = if c.arena.order[ep] == 1 {
                        c.arena.head[ep]
                    } else {
                        c.arena.tail[ep]
                    };
                    if c.nd[np].block.is_none() {
                        let b = match block {
                            Some(b) => b,
                            None => {
                                let b = c.make_block();
                                block = Some(b);
                                b
                            }
                        };
                        add_node(c, b, np);
                    }
                    if ep == e {
                        break;
                    }
                }
                if let Some(b) = block {
                    if c.nd[u].block.is_none() && block_size(&c.blocks[b]) > 1 {
                        add_node(c, b, u);
                    }
                    if is_root && c.nd[u].block == Some(b) {
                        insert_block(&mut c.bl, b);
                    } else {
                        append_block(&mut c.bl, b);
                    }
                }
            }
        } else if c.nd[u].parent != Some(v) {
            c.nd[u].low_val = c.nd[u].low_val.min(c.nd[v].val);
        }
    }
    if is_root && c.nd[u].block.is_none() {
        let b = c.make_block();
        add_node(c, b, u);
        insert_block(&mut c.bl, b);
    }
}

/// `find_blocks(g, state)`.
fn find_blocks(c: &mut Circ, g: &CGraph) {
    let mut root = None;
    if let Some(name) = &c.rootname {
        root = g.nodes().into_iter().find(|&n| c.names[n] == *name);
    }
    if root.is_none() && c.n_root {
        root = g.nodes().into_iter().find(|&n| c.node_root[n]);
    }
    let root = root.unwrap_or_else(|| g.first_node().expect("non-empty component"));
    let name = c.names.get(root).map_or("", String::as_str);
    graphviz_pure_core::gv_info!("blocktree.c", 133, "root = {name}");
    let mut stk = Vec::new();
    dfs(c, g, root, true, &mut stk);
}

/// `createBlocktree(g, state)`: build the block list, then hang each block
/// below the block holding the parent of its earliest-visited node.
pub fn create_blocktree(c: &mut Circ, g: &CGraph) -> usize {
    find_blocks(c, g);
    let bl = std::mem::take(&mut c.bl);
    let root = bl[0];
    for &bp in &bl[1..] {
        let nodes = c.blocks[bp].sub_graph.nodes();
        let mut child = nodes[0];
        let mut min = c.nd[child].val;
        let mut parent = c.nd[child].parent;
        for &n in &nodes[1..] {
            if c.nd[n].val < min {
                child = n;
                min = c.nd[n].val;
                parent = c.nd[n].parent;
            }
        }
        let parent = parent.expect("non-root block has a parent");
        c.set_flag(parent, PARENT_F);
        c.blocks[bp].child = Some(child);
        let pb = c.nd[parent].block.expect("parent in a block");
        append_block(&mut c.blocks[pb].children, bp);
    }
    root
}
