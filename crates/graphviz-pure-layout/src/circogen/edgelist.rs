//! `lib/circogen/edgelist.c`: the set of open edges used when counting
//! crossings.
//!
//! Used only as a test oracle now: production crossing counts use the indexed
//! bitset in `blockpath.rs`, preserving this set's edge-half ordering.
//!
//! C keys the set by `Agedge_t *`. An undirected cgraph edge has two halves
//! with distinct addresses, and `agfstedge`/`agnxtedge` yield the out-half
//! from the tail and the in-half from the head, so an edge opened at one end
//! is never found when it is removed at the other. The key therefore
//! includes the half; set order does not affect the crossing count.

use std::collections::BTreeSet;

/// An edge half: the edge id and whether it is the in-half (`AGINEDGE`).
pub type EdgeHalf = (usize, bool);

pub type EdgeList = BTreeSet<EdgeHalf>;

/// `init_edgelist()`.
pub fn init_edgelist() -> EdgeList {
    BTreeSet::new()
}

/// `add_edge(list, e)`.
pub fn add_edge(list: &mut EdgeList, e: EdgeHalf) {
    list.insert(e);
}

/// `remove_edge(list, e)`.
pub fn remove_edge(list: &mut EdgeList, e: EdgeHalf) {
    list.remove(&e);
}
