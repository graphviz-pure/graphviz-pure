//! Port of Graphviz `lib/neatogen` (the `neato` layout engine).
//!
//! Module ↔ C file map:
//! - [`neatoinit`] — `neatoinit.c` (`neato_layout` and helpers)
//! - [`stuff`] — `stuff.c` (graph scan, Kamada–Kawai)
//! - [`stress`] — `stress.c` (stress majorization) and parts of `kkutils.c`
//! - [`smart_ini`] — `stress.c` subspace majorization, `embed_graph.c`, `pca.c`
//! - [`sgd`] — `sgd.c` (stochastic gradient descent)
//! - [`conjgrad`], [`matrix_ops`], [`bfs`], [`linalg`] — `conjgrad.c`,
//!   `matrix_ops.c`, `bfs.c` + `dijkstra.c`, `solve.c` + `lu.c` + `matinv.c`
//! - [`graphdata`] — `vtx_data` and `makeGraphData`
//! - [`neatosplines`] — `neatosplines.c` and `makeStraightEdge(s)` from `routespl.c`
//! - [`routing`] — obstacle-avoiding edges (`splines=true|polyline`)
//! - [`adjust`] — `adjust.c` overlap removal

// Faithful ports of numeric C loops index several arrays in lockstep.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod adjust;
pub mod bfs;
pub mod call_tri;
pub mod cola;
pub mod conjgrad;
pub mod constraint;
pub mod delaunay;
pub mod digcola;
pub mod graphdata;
pub mod legal;
pub mod linalg;
pub mod matrix_ops;
#[cfg(all(feature = "parallel", not(target_family = "wasm")))]
mod matvec_parallel;
pub mod multispline;
pub mod neatoinit;
pub mod neatosplines;
pub mod overlap;
#[cfg(all(feature = "parallel", not(target_family = "wasm")))]
pub(crate) mod par;
pub mod quad_prog_vpsc;
pub mod robust;
pub mod routing;
pub mod scadjust;
pub mod sgd;
pub mod smart_ini;
pub mod stress;
pub mod stuff;
pub mod voronoi;

pub const MODEL_SHORTPATH: i32 = 0;
pub const MODEL_CIRCUIT: i32 = 1;
pub const MODEL_SUBSET: i32 = 2;
pub const MODEL_MDS: i32 = 3;

pub const MODE_KK: i32 = 0;
pub const MODE_MAJOR: i32 = 1;
pub const MODE_HIER: i32 = 2;
pub const MODE_IPSEP: i32 = 3;
pub const MODE_SGD: i32 = 4;

pub const INIT_SELF: i32 = 0;
pub const INIT_REGULAR: i32 = 1;
pub const INIT_RANDOM: i32 = 2;

/// Process-global numeric parameters of neato (`Epsilon`, `Damping`,
/// `Initial_dist`, `MaxIter`).
#[derive(Debug, Clone, Default)]
pub struct NeatoParams {
    pub epsilon: f64,
    pub damping: f64,
    pub initial_dist: f64,
    pub max_iter: i64,
    /// `agnameof(G)` of the graph being laid out, for the warnings.
    pub graph_name: String,
}

/// A numeric solver failure (C returns a negative iteration count).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverError;

impl std::fmt::Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("neato solver failure")
    }
}

impl std::error::Error for SolverError {}
