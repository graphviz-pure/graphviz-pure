//! The failure boundary of the ortho router.
//!
//! `lib/ortho` asserts many of its invariants (`LIST_GET` checks every index,
//! `chanSearch` asserts its lookups) and relies on the rest without checking
//! (raw `N_DAD` and `cells[]` pointers, `int` distances). Where one does not
//! hold, C aborts, dereferences NULL, reads out of bounds or loops. The port
//! returns [`Fail`] from every function that can meet such a state, and
//! [`super::ortho_edges_of`] (`orthoEdges`) handles each failure in one
//! place, the way C handles the failure it does detect at that stage:
//!
//! - while the maze is built, as C's `mkMaze` returning NULL: `orthoEdges`
//!   returns -1, after printing what C prints on the way there
//!   ([`Fail::c_message`]);
//! - while edges are routed, as C's `goto orthofinish` after `shortPath` or
//!   `assignTracks` fails: no edge is routed and `orthoEdges` returns 0.
//!
//! Inputs C routes never fail, so their output does not change.

use std::panic::Location;

/// Why the router stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    /// `add_segment` finds no trapezoid below a segment: C prints "trapezoid
    /// segment construction failed" (`trapezoid.c:623`) and returns -1, so
    /// `construct_trapezoids` returns no trapezoids. `partition` turns it into
    /// [`Fail::Trapezoidation`].
    Segment,
    /// `partition`'s horizontal or vertical trapezoidation failed with
    /// [`Fail::Segment`]: C prints "horizontal trapezoid construction failed"
    /// or "vertical ..." (`partition.c:694/711`) and returns NULL, so `mkMaze`
    /// does.
    Trapezoidation { vertical: bool },
    /// `partition` leaves no free cell: its `LIST_DETACH` gives NULL, and
    /// `mkMaze` returns NULL without a message.
    NoFreeCell,
    /// `chkSgraph` finds a search-graph node without a cell on `side`: C
    /// prints "failed at node N[side]" (`maze.c:315/317`) and fails its
    /// assertion.
    NoCell { node: usize, side: usize },
    /// `PQ_insert` finds the heap full; C prints "Heap overflow" and
    /// `shortPath` fails.
    HeapOverflow,
    /// `seg_cmp` meets two segments in different channels; C prints
    /// "incomparable segments !! -- Aborting" and `assignTracks` fails.
    Incomparable,
    /// `shortPath` never reached the edge's start node, so `N_DAD(sn)` is
    /// NULL and C's `convertSPtoRoute` dereferences it (#186).
    NoPath,
    /// A route segment lies in no channel: C's `chanSearch` asserts (#170).
    NoChannel,
    /// Any other invariant C asserts or trusts: the port's source line.
    Broken(&'static Location<'static>),
    /// Not in C: a limit of the caller's `Limits::run` stopped the routing. Nothing is
    /// written; the library call returns the limit's error.
    Stopped,
}

pub type OResult<T> = Result<T, Fail>;

impl Fail {
    /// What C writes to stderr with a bare `fprintf` on the way to this
    /// failure, in C's order: the messages of the maze failures C detects.
    pub fn c_message(self) -> Option<String> {
        const SEGMENT: &str = "trapezoid segment construction failed\n";
        match self {
            Fail::Segment => Some(SEGMENT.to_string()),
            Fail::Trapezoidation { vertical } => Some(format!(
                "{SEGMENT}{} trapezoid construction failed\n",
                if vertical { "vertical" } else { "horizontal" }
            )),
            Fail::NoCell { node, side } => Some(format!("failed at node {node}[{side}]\n")),
            _ => None,
        }
    }

    /// [`Fail::Broken`] at the caller.
    #[track_caller]
    pub fn here() -> Fail {
        Fail::Broken(Location::caller())
    }
}

/// Fails unless `cond` holds: the port's form of a C `assert`.
#[track_caller]
pub fn ensure(cond: bool) -> OResult<()> {
    if cond { Ok(()) } else { Err(Fail::here()) }
}

/// Checked indexing: an index outside the data is a broken invariant.
pub trait At<T> {
    fn at(&self, i: usize) -> OResult<&T>;
    fn at_mut(&mut self, i: usize) -> OResult<&mut T>;
}

impl<T> At<T> for [T] {
    #[track_caller]
    fn at(&self, i: usize) -> OResult<&T> {
        match self.get(i) {
            Some(x) => Ok(x),
            None => Err(Fail::here()),
        }
    }

    #[track_caller]
    fn at_mut(&mut self, i: usize) -> OResult<&mut T> {
        match self.get_mut(i) {
            Some(x) => Ok(x),
            None => Err(Fail::here()),
        }
    }
}

/// A pointer C dereferences without a NULL check.
pub trait Need<T> {
    fn need(self) -> OResult<T>;
}

impl<T> Need<T> for Option<T> {
    #[track_caller]
    fn need(self) -> OResult<T> {
        match self {
            Some(x) => Ok(x),
            None => Err(Fail::here()),
        }
    }
}

/// An `int` index stored in the trapezoidation tables, as a `usize`.
#[track_caller]
pub fn idx(i: i32) -> OResult<usize> {
    usize::try_from(i).map_err(|_| Fail::here())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #127: the bare `fprintf` messages of `add_segment`
    /// (`trapezoid.c:623`), `partition` (`partition.c:694/711`) and
    /// `chkSgraph` (`maze.c:315/317`), in C's order. The other failures print
    /// through `agerr` at the boundary, or not at all.
    #[test]
    fn maze_failures_carry_cs_messages() {
        let msg = |f: Fail| f.c_message();
        assert_eq!(
            msg(Fail::Trapezoidation { vertical: false }).as_deref(),
            Some(
                "trapezoid segment construction failed\nhorizontal trapezoid construction failed\n"
            )
        );
        assert_eq!(
            msg(Fail::Trapezoidation { vertical: true }).as_deref(),
            Some("trapezoid segment construction failed\nvertical trapezoid construction failed\n")
        );
        assert_eq!(
            msg(Fail::NoCell { node: 7, side: 0 }).as_deref(),
            Some("failed at node 7[0]\n")
        );
        for f in [
            Fail::NoFreeCell,
            Fail::HeapOverflow,
            Fail::Incomparable,
            Fail::NoPath,
            Fail::NoChannel,
            Fail::here(),
        ] {
            assert_eq!(msg(f), None, "{f:?}");
        }
    }
}
