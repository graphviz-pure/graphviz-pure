//! Faithful, function-by-function port of Graphviz 16.1.0 `lib/dotgen`
//! (plus `lib/common/ns.c` and the parts of `lib/common` that dot uses).
//!
//! See `docs/porting/dotgen.md` for the module <-> C file map, status and the
//! phase-2/3 handoff notes. Each submodule mirrors one C file; functions keep
//! their C names (snake_case) so the two can be read side by side.

// Loops and late initialisations mirror the C control flow on purpose.
#![allow(clippy::explicit_counter_loop, clippy::needless_late_init)]

pub mod acyclic;
pub mod cg;
pub mod class1;
pub mod class2;
pub mod cluster;
pub mod compound;
pub mod conc;
pub mod decomp;
pub mod dotsplines;
pub mod fastgr;
pub mod flat;
pub mod init;
pub mod ix;
pub mod mincross;
pub mod ns;
pub mod output;
pub mod pack;
pub mod position;
pub mod postproc;
pub mod qsort;
pub mod rank;
pub mod reach;
pub mod sameport;
pub mod splinegraph;
pub mod types;
pub mod utils;

use graphviz_pure_core::{Graph, GraphvizError, GraphvizErrorKind, Result};
use types::*;

/// Not in C: reads the limits of the caller's `Limits::run` now, at a phase boundary, and
/// stops the layout with [`graphviz_pure_core::limits::STOPPED`] when one is reached (the
/// library call returns the limit's own error).
pub(crate) fn check_limits() -> std::result::Result<(), String> {
    graphviz_pure_core::limits::check().map_err(|_| graphviz_pure_core::limits::STOPPED.to_string())
}

/// The prefix of a fatal error in the dotgen's message channel.
///
/// The dotgen's errors are C's own messages, which the CLI reports as a layout error.
/// Where C exits with a message of its own (`gv_calloc`'s "out of memory when trying to
/// allocate N bytes"), the port stops the layout with that message, and the CLI writes it
/// as C wrote it: the message travels with this prefix, as [`mincross::ABORT_LAYOUT`]
/// travels with its own.
pub const FATAL: &str = "\u{1}fatal:";

/// The message of a fatal error in the dotgen's message channel, where the port stops the
/// layout where C exits.
pub fn fatal(error: impl Into<GraphvizError>) -> String {
    let error = error.into();
    match error.kind() {
        GraphvizErrorKind::Allocation => format!("{FATAL}{}", error.message()),
        _ => error.message().to_string(),
    }
}

/// The layout error of the dotgen's message channel: C's own message, or the message of a
/// fatal error, which the CLI writes as C wrote it.
pub fn dot_error(message: String) -> GraphvizError {
    match message.strip_prefix(FATAL) {
        Some(text) => GraphvizError::allocation(text),
        None => GraphvizError::layout("dot", message),
    }
}

/// All dot layout state: arenas mirroring the cgraph objects and their
/// `Agraphinfo_t`/`Agnodeinfo_t`/`Agedgeinfo_t` records, plus the C file-scope
/// statics.
pub struct Dot<'a> {
    pub input: &'a Graph,
    /// The cgraph root whose attribute dictionaries are visible (`agattr(agroot(g), ...)`):
    /// `input` itself, or the whole graph when `input` is a pack component.
    pub root_input: &'a Graph,
    /// The layout graph is a component subgraph (`dot_root(g) != agroot(g)`, doDot's
    /// pack branch).
    pub comp: bool,
    /// `i` of the component subgraph `_cc_<i>` when `comp`.
    pub comp_index: usize,
    /// `GD_bb(agroot(g))` while a pack component is laid out. `clip_and_install`
    /// and `addEdgeLabels` grow `GD_bb(agraphof(tail))`, the root graph rather
    /// than the component, and `packSubgraphs` recomputes the root's box
    /// (`compute_bb`), so these updates never reach the component's `GD_bb`.
    pub root_bb: BoxF,
    /// The root graph's in-edge dictionaries by tail (`agsubrep(g, n)->in_id`), each
    /// built on the first `agfindedge` into its node. See [`Dot::agfindedge_root`].
    pub cg_in_id: std::cell::RefCell<Vec<Option<crate::cdt::Cdt<E>>>>,
    /// Input edges `agsubedge` found in the root graph's `in_id` dictionaries (a search
    /// that splays them), in order, for the dictionaries not built yet.
    pub cg_in_id_hits: std::cell::RefCell<Vec<E>>,
    /// `cccomps` ran on the root (doDot's pack branch): `graphviz_node_induce` found
    /// every edge in the root's dictionaries, by tail and then edge.
    pub cg_in_id_packed: bool,
    /// Root node index of each input node, when `input` is a pack component.
    pub cg_root_node: Option<Vec<usize>>,
    /// What [`init::MeasuredNodes::measure`] measured of `root_input`'s nodes, by root
    /// node index, while the nodes are initialised (when this graph measures its nodes as
    /// `root_input` does).
    pub measured: Option<init::MeasuredNodes>,
    /// Root only: [`Self::measured`] once the nodes are initialised, kept for the renderer
    /// as C keeps `ND_label` and `ND_shape_info` (`layout_dot` leaves it on the graph).
    pub handoff: Option<init::MeasuredNodes>,
    /// The node arenas, one per part of `Agnodeinfo_t` ([`NodeInfo`]), indexed alike.
    /// [`Self::push_node`] grows the first three together.
    pub nodes: Vec<NodeInfo>,
    pub ns_nodes: Vec<NodeNs>,
    pub mc_nodes: Vec<NodeMincross>,
    /// Up to the last node whose cold part was written ([`Self::cold`], [`Self::cold_mut`]).
    cold: Vec<NodeCold>,
    pub edges: Vec<EdgeInfo>,
    pub graphs: Vec<GraphInfo>,
    ns_scratch: ns::NsScratch,
    pub labels: ix::IndexVec<L, TextLabel>,
    /// Backing storage of `rank_t.v`/`av` arrays.
    pub bufs: Vec<Vec<Option<N>>>,
    pub n_real_nodes: usize,
    pub edge_seq: u64,
    // globals
    pub cl_type: i32,
    pub concentrate: bool,
    /// The node and edge symbols `setState` NULLs on `make_flat_adj_edges`' auxg although
    /// cloneGraph declared them: [`Dot::node_attr_declared`] and [`Dot::edge_attr_declared`]
    /// read them as undeclared. Empty for every other graph.
    pub null_syms: (&'static [&'static str], &'static [&'static str]),
    // decomp.c statics
    pub cmark: usize,
    pub decomp_last: Option<N>,
    // rank.c (newrank) statics
    pub xg_last: Option<N>,
    pub weak_id: i32,
    // mincross.c statics
    pub min_quit: i32,
    pub max_iter: i32,
    pub global_min_rank: i32,
    pub global_max_rank: i32,
    pub remincross: bool,
    mincross_store: mincross::MinCrossStore,
    /// `EDGE_TYPE(g)` (`setEdgeType(g, EDGETYPE_SPLINE)`).
    pub et: crate::common::types::EdgeType,
    /// `State == GVSPLINES`: splines exist and are mapped by postprocessing.
    pub splines_done: bool,
    /// The layout gave up (dot_position or orthoEdges failed): no splines, no
    /// postprocessing.
    pub aborted: bool,
    /// An aux abort returned before `cleanupCloneGraph`, leaving every main node's `N_style`
    /// NULL. Other aborts do not clear node styles.
    pub node_styles_cleared: bool,
    /// The `_cc_<i>` subgraphs a failed layout leaves in the root (doDot returns
    /// before `agdelete`), with the projected subgraphs under them.
    pub cc_kept: Vec<G>,
    /// The input's own `_cc_<i>` subgraphs that cccomps took as components, which doDot
    /// deletes after packing (`agdelete(g, ccs[i])`, #335).
    pub cc_deleted: Vec<usize>,
    /// The `attach_phase_attrs` level of each node's pack component, where its own `phase`
    /// (agget(sg), #335, #415) stopped it at 1 to 3; empty unless doDot laid out several
    /// components.
    pub node_phase: Vec<Option<i32>>,
    /// The `tend`/`hend` ends dot's edge routers last left (#360).
    pub stale_ends: dotsplines::StaleEnds,
    /// The input's edge and cluster labels `common_init_edge` and `do_graph_label`
    /// measured, which C keeps in `ED_label` and `GD_label` for the renderer. Only the
    /// root's layout keeps them (`Some`): a pack component's or an aux graph's objects are
    /// not the input's.
    pub kept_labels: Option<graphviz_pure_geom::LaidOutLabels>,
}

impl Dot<'_> {
    /// Appends a node to the node arenas: `info`, and the default network-simplex and
    /// mincross parts. Its cold part stays empty until [`Self::cold_mut`] writes one.
    pub(super) fn push_node(&mut self, info: NodeInfo) -> N {
        let n = self.nodes.len();
        self.nodes.push(info);
        self.ns_nodes.push(NodeNs::default());
        self.mc_nodes.push(NodeMincross::default());
        n
    }

    /// `Vec::reserve_exact` on the node arenas that [`Self::push_node`] grows.
    pub(super) fn reserve_nodes_exact(&mut self, additional: usize) {
        self.nodes.reserve_exact(additional);
        self.ns_nodes.reserve_exact(additional);
        self.mc_nodes.reserve_exact(additional);
    }

    /// Bytes of one node in the arenas [`Self::push_node`] grows.
    pub(super) const NODE_BYTES: usize =
        size_of::<NodeInfo>() + size_of::<NodeNs>() + size_of::<NodeMincross>();

    /// The cold part of node `n`: [`NO_COLD`] for a node that has none.
    pub fn cold(&self, n: N) -> &NodeCold {
        self.cold.get(n).unwrap_or(&NO_COLD)
    }

    /// The cold part of node `n`, for writing. It covers the arena up to `n` from now on,
    /// with empty cold parts for the nodes before `n` that had none.
    pub fn cold_mut(&mut self, n: N) -> &mut NodeCold {
        let (len, need) = (self.cold.len(), n.saturating_add(1));
        if len < need {
            // Not in C: checked against `Limits::max_allocation` like the other node arenas
            // ([`Self::note_node_push`]); past the limit the layout stops at its next check.
            let bytes = fastgr::arena_growth_bytes(
                len,
                self.cold.capacity(),
                need.saturating_sub(len),
                size_of::<NodeCold>(),
            );
            let _ = graphviz_pure_core::limits::check_allocation(bytes);
            self.cold.resize_with(need, NodeCold::default);
        }
        // In bounds: the arena reaches `n` now.
        &mut self.cold[n]
    }

    /// Not in C: `count` default nodes in every node arena, for tests that build a graph
    /// by hand.
    #[cfg(test)]
    pub(super) fn reset_nodes_for_test(&mut self, count: usize) {
        self.nodes = vec![NodeInfo::default(); count];
        self.ns_nodes = vec![NodeNs::default(); count];
        self.mc_nodes = vec![NodeMincross::default(); count];
        self.cold.clear();
    }
}

impl<'a> Dot<'a> {
    fn empty(input: &'a Graph) -> Self {
        Dot {
            input,
            root_input: input,
            comp: false,
            comp_index: 0,
            root_bb: BoxF::default(),
            cc_kept: Vec::new(),
            cc_deleted: Vec::new(),
            node_phase: Vec::new(),
            cg_in_id: std::cell::RefCell::new(Vec::new()),
            cg_in_id_hits: std::cell::RefCell::new(Vec::new()),
            cg_in_id_packed: false,
            cg_root_node: None,
            measured: None,
            handoff: None,
            nodes: Vec::new(),
            ns_nodes: Vec::new(),
            mc_nodes: Vec::new(),
            cold: Vec::new(),
            edges: Vec::new(),
            graphs: Vec::new(),
            ns_scratch: ns::NsScratch::default(),
            labels: ix::IndexVec::new(),
            bufs: Vec::new(),
            n_real_nodes: 0,
            edge_seq: 0,
            cl_type: LOCAL,
            concentrate: false,
            null_syms: (&[], &[]),
            cmark: 0,
            decomp_last: None,
            xg_last: None,
            weak_id: 0,
            min_quit: 8,
            max_iter: 24,
            global_min_rank: 0,
            global_max_rank: 0,
            remincross: false,
            mincross_store: Default::default(),
            et: crate::common::types::EdgeType::Spline,
            splines_done: false,
            aborted: false,
            node_styles_cleared: false,
            stale_ends: Default::default(),
            kept_labels: None,
        }
    }

    // ---- rank_t accessors (GD_rank(g)[r]) ----

    #[inline]
    pub fn rk(&self, g: G, r: i32) -> &Rank {
        &self.graphs[g].rank[(r + 1) as usize]
    }

    #[inline]
    pub fn rk_mut(&mut self, g: G, r: i32) -> &mut Rank {
        &mut self.graphs[g].rank[(r + 1) as usize]
    }

    /// `GD_rank(g)[r].n`, 0 when the rank slot does not exist.
    #[inline]
    pub fn rank_n(&self, g: G, r: i32) -> i32 {
        self.graphs[g].rank.get((r + 1) as usize).map_or(0, |x| x.n)
    }

    /// `GD_rank(g)[r].v[i]`
    #[inline]
    pub fn rv(&self, g: G, r: i32, i: i32) -> Option<N> {
        if i < 0 {
            return None;
        }
        let rk = self.graphs[g].rank.get((r + 1) as usize)?;
        self.bufs
            .get(rk.vbuf)?
            .get(rk.voff + i as usize)
            .copied()
            .flatten()
    }

    /// `GD_rank(g)[r].v[i] = n`
    #[inline]
    pub fn set_rv(&mut self, g: G, r: i32, i: i32, n: Option<N>) {
        let rk = &self.graphs[g].rank[(r + 1) as usize];
        let (b, o) = (rk.vbuf, rk.voff + i as usize);
        let buf = &mut self.bufs[b];
        if o >= buf.len() {
            buf.resize(o + 1, None);
        }
        buf[o] = n;
    }

    /// Allocates a zeroed `node_t*` array of `len` slots.
    pub fn new_buf(&mut self, len: usize) -> usize {
        self.bufs.push(vec![None; len]);
        self.bufs.len() - 1
    }

    /// Debug dump of the layout state (`GVDUMP=1`), matching the instrumented C build
    /// described in docs/porting/dotgen.md.
    pub fn gvdump(&self, phase: &str) {
        if std::env::var_os("GVDUMP").is_none() {
            return;
        }
        if phase == "R" {
            for n in self.nodes_of(0) {
                graphviz_pure_core::diag::dump(&format!(
                    "R {} {}",
                    self.nodes[n].name, self.nodes[n].rank
                ));
            }
            return;
        }
        for r in self.graphs[0].minrank..=self.graphs[0].maxrank {
            let mut line = format!("{phase} {r}:");
            for i in 0..self.rank_n(0, r) {
                let Some(v) = self.rv(0, r, i) else { continue };
                if self.nodes[v].node_type == NORMAL {
                    line.push(' ');
                    line.push_str(&self.nodes[v].name);
                } else {
                    line.push_str(" _");
                }
                if phase == "P" {
                    line.push_str(&format!("@{}", self.nodes[v].coord.x));
                }
            }
            graphviz_pure_core::diag::dump(&line);
        }
    }

    /// `setEdgeType(g, EDGETYPE_SPLINE)` (utils.c).
    pub fn set_edge_type(&mut self) {
        use crate::common::types::EdgeType;
        self.et = match self.agget(0, "splines") {
            None => EdgeType::Spline,
            Some("") => EdgeType::None,
            Some(s) => crate::common::init::edge_type_of(s, EdgeType::Spline),
        };
    }

    /// `dotLayout` (dotinit.c).
    pub fn dot_layout(&mut self) -> std::result::Result<(), String> {
        let maxphase = utils::late_int(self.agget(0, "phase"), -1, 1);
        self.set_edge_type();
        // `setAspect` (aspect.c): `sscanf(p, "%lf,%d", ...) > 0`.
        if self
            .agget(0, "aspect")
            .is_some_and(|p| graphviz_pure_core::strtod::scan_lf(p).is_some())
        {
            graphviz_pure_core::diag::agwarningf(
                "the aspect attribute has been disabled due to implementation flaws - \
                 attribute ignored.\n",
            );
        }
        self.dot_init_subg();
        self.dot_init_node_edge()?;
        // A pack component's caller takes the measurement back for the next component and
        // keeps it for the renderer once the last one is done.
        if !self.comp {
            self.handoff = self.measured.take();
        }
        // Not in C: each phase starts with the limits of the caller's `Limits::run`.
        check_limits()?;
        graphviz_pure_core::gv_info!("dotinit.c", 305, "Starting phase 1 [dot_rank]");
        self.dot_rank()?;
        self.gvdump("R");
        if maxphase == 1 {
            return Ok(());
        }
        check_limits()?;
        graphviz_pure_core::gv_info!("dotinit.c", 311, "Starting phase 2 [dot_mincross]");
        if let Err(e) = self.dot_mincross() {
            if e == mincross::ABORT_LAYOUT {
                self.aborted = true;
                return Ok(());
            }
            return Err(e);
        }
        self.gvdump("M");
        if maxphase == 2 {
            return Ok(());
        }
        check_limits()?;
        graphviz_pure_core::gv_info!("dotinit.c", 320, "Starting phase 3 [dot_position]");
        self.dot_position()?;
        if self.aborted {
            return Ok(());
        }
        self.gvdump("P");
        if maxphase == 3 {
            return Ok(());
        }
        if self.graphs[0].flags & NEW_RANK != 0 {
            self.remove_fill();
        }
        self.dot_sameports();
        check_limits()?;
        if let Err(e) = self.dot_splines() {
            if e == mincross::ABORT_LAYOUT {
                self.aborted = true;
                return Ok(());
            }
            return Err(e);
        }
        if self.aborted {
            return Ok(());
        }
        if utils::mapbool(self.agget(0, "compound")) {
            self.dot_compound_edges();
        }
        Ok(())
    }
}

/// Lays out `graph` with the dotgen port.
pub fn layout_dot(graph: &mut Graph) -> Result<()> {
    // dot_init_node's common_init_node reports the node diagnostics, in C's place.
    let measured = graphviz_pure_core::diag::quietly(|| init::MeasuredNodes::measure(graph))?;
    let aborted;
    let (result, labels, handoff) = {
        let mut d = Dot::new(graph);
        d.measured = Some(measured);
        d.kept_labels = Some(Default::default());
        d.graph_init();
        let maxphase = utils::late_int(d.agget(0, "phase"), -1, 1);
        // dot_layout: doDot only when there are nodes; dotneato_postprocess always.
        if !graph.nodes.is_empty() {
            pack::do_dot(&mut d, graph).map_err(dot_error)?;
        }
        if !d.aborted {
            d.dotneato_postprocess();
        }
        aborted = d.aborted;
        // dotLayout stops after dot_rank, dot_mincross or dot_position with
        // attach_phase_attrs(g, 1 or 2), and dot_layout still post-processes what it left
        // (dotinit.c:306-329, :517). A layout that failed returned before
        // attach_phase_attrs.
        // When doDot packed several components, each attached its own phase
        // (`Dot::node_phase`) and the root's dotLayout never ran.
        let packed = !d.node_phase.is_empty();
        let phase_attrs =
            ((1..=3).contains(&maxphase) && !aborted && !packed).then(|| maxphase.min(2));
        let mut labels = d.kept_labels.take().unwrap_or_default();
        if !d.cc_deleted.is_empty() {
            // agdelete renumbers the subgraphs the clusters' labels are keyed by
            labels.clusters.clear();
        }
        (output::Output::collect(&d, phase_attrs), labels.sorted(), d.handoff.take())
    };
    result.apply(graph);
    // ED_label and GD_label stay on the input's edges and clusters for the renderer.
    if !(labels.edges.is_empty() && labels.clusters.is_empty()) {
        graph.layout.label_geometry.put(labels);
    }
    // What the layout measured of each node, which the renderer draws instead of measuring
    // the nodes again (`ND_label`, `ND_shape_info`).
    if let Some(measured) = handoff {
        graph.layout.node_geometry.put(measured.into_laid_out());
    }
    if aborted {
        // no place_graph_label: labels keep their zeroed position
        crate::common::postproc::labels_at_origin(graph);
    }
    Ok(())
}
