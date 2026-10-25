//! The neato driver, ported from `lib/neatogen/neatoinit.c`: `neato_layout`,
//! `neatoLayout`, `majorization`, `user_pos`, `checkStart`/`setSeed`,
//! `initRegular`, `neatoMode`, `neatoModel`, `checkExp`, `addCluster`, and the `Nop`
//! (`neato -n`) path: `init_nop`, `nop_init_graphs`, `nop_init_edges`, `user_spline`,
//! `set_label`, `chkBB`.

use super::adjust::{AdjustData, graph_adjust_mode, remove_overlap_with};
use super::graphdata::make_graph_data;
use super::neatosplines::{neato_set_aspect, neato_translate, spline_edges, spline_edges0};
use super::stress::{DFLT_ITERATIONS, OPT_SMART_INIT, stress_majorization_kd_mkernel};
use super::stuff::{kk_neato, scan_graph_mode};
use super::*;
use crate::common::postproc::gv_postprocess;
use crate::common::types::*;
use crate::common::utils::{atoi, compute_bb, late_int, mapbool};
use crate::pack::{PackInfo, PackMode, get_pack, get_pack_mode_info, pack_graphs, pccomps};
use graphviz_pure_core::math;
use graphviz_pure_core::strtod::{Scanned, scan_lf, sscanf};
use graphviz_pure_core::{Graph, NodeData, Point, Result};

/// `cc_pfx`: the prefix of neato's component subgraphs.
const CC_PFX: &str = "_neato_cc";
/// `CL_OFFSET`: default margin around packed components, in points.
pub const CL_OFFSET: i64 = 8;

/// `user_pos(posptr, pinptr, np, nG)`: parse a node's `pos`/`pin` (and, with `dim > 2`, its
/// `z`). `ng` is the graph's node count.
fn user_pos(nd: &NodeData, node: &mut LNode, ng: usize, inputscale: f64, decl: PosDecl) {
    if !decl.pos {
        return;
    }
    let Some(p) = nd.attrs.get("pos").map(String::as_str) else {
        return;
    };
    let z = if decl.z {
        nd.attrs.get("z").map(String::as_str)
    } else {
        None
    };
    let pin = mapbool(nd.attrs.get("pin").map(String::as_str));
    if let Some(pinned) = user_pos_values(p, z, pin, &nd.name, inputscale, ng, &mut node.pos) {
        node.pinned = pinned;
    }
}

/// Whether the graph declares the node attributes `pos` and `z`, as `user_pos` reads them.
#[derive(Clone, Copy)]
struct PosDecl {
    pos: bool,
    z: bool,
}

impl PosDecl {
    fn of(g: &Graph) -> Self {
        PosDecl {
            pos: g.node_attr_declared("pos"),
            z: g.node_attr_declared("z"),
        }
    }
}

/// `neato_init_node_edge(g)`: each node's `neato_init_node` then its `user_pos`, so a
/// node's `pos` error follows its own init diagnostics, then every edge's init.
fn neato_init_node_edge<'a>(
    src: &'a Graph,
    ndim: usize,
    inputscale: f64,
    decl: PosDecl,
) -> LGraph<'a> {
    let ng = src.nodes.len();
    LGraph::new_with(src, ndim, EdgeType::Line, |nd, node| {
        user_pos(nd, node, ng, inputscale, decl);
    })
}

/// The body of `user_pos` on a node's `pos`, `z` (`None` when `z` is not declared) and
/// `pin`: fills `pvec` (`ND_pos`, of length `Ndim`) and returns `ND_pinned`, or `None` when
/// `pos` is empty or unreadable (after C's error message).
pub(crate) fn user_pos_values(
    p: &str,
    z: Option<&str>,
    pin: bool,
    name: &str,
    inputscale: f64,
    ng: usize,
    pvec: &mut [f64],
) -> Option<u8> {
    if p.is_empty() {
        return None;
    }
    let ndim = pvec.len();
    // sscanf assigns each value it reads, even when it then fails.
    let assign = |pvec: &mut [f64], r: &Scanned<'_>, max: usize| {
        let k = usize::try_from(r.count).unwrap_or(0).min(max);
        for (p, v) in pvec.iter_mut().zip(r.values).take(k) {
            *p = v;
        }
    };
    let scale = |pvec: &mut [f64]| {
        if inputscale > 0.0 {
            for v in pvec.iter_mut() {
                *v /= inputscale;
            }
        }
    };
    let pinned = |c: Option<u8>| if c == Some(b'!') || pin { P_PIN } else { P_SET };
    if ndim >= 3 {
        let r = sscanf(p, "%lf,%lf,%lf%c");
        assign(pvec, &r, 3);
        if r.count >= 3 {
            scale(pvec);
            if ndim > 3 {
                jitter_d(pvec, ng, 3);
            }
            return Some(pinned(r.c));
        }
    }
    let r = sscanf(p, "%lf,%lf%c");
    assign(pvec, &r, 2);
    if r.count >= 2 {
        scale(pvec);
        if ndim > 2 {
            if let Some((z, _)) = z.and_then(scan_lf) {
                if let Some(p) = pvec.get_mut(2) {
                    *p = if inputscale > 0.0 { z / inputscale } else { z };
                }
                jitter_d(pvec, ng, 3);
            } else {
                jitter_d(pvec, ng, 2);
            }
        }
        return Some(pinned(r.c));
    }
    graphviz_pure_core::diag::agerrorf(&format!(
        "node {name}, position {p}, expected two doubles\n"
    ));
    None
}

/// `jitter_d(np, nG, n)` (`jitter3d` is `jitter_d(np, nG, 2)`): random coordinates from
/// dimension `from` on.
fn jitter_d(pvec: &mut [f64], ng: usize, from: usize) {
    for v in pvec.iter_mut().skip(from) {
        *v = ng as f64 * crate::cutil::drand48();
    }
}

/// `addZ(g)`: with `dim >= 3` and a declared `z`, every node's `z` becomes its third
/// coordinate in points, printed with `%lf`.
fn add_z(g: &LGraph<'_>, z_declared: bool, graph: &mut Graph) {
    if g.ndim >= 3 && z_declared {
        for (nd, n) in graph.nodes.iter_mut().zip(&g.nodes) {
            set_z(nd, n.pos.get(2).copied().unwrap_or(0.0));
        }
    }
}

/// `addZ`'s `agxset(n, N_z, "%lf" of POINTS_PER_INCH * ND_pos(n)[2])`.
pub(crate) fn set_z(node: &mut graphviz_pure_core::NodeData, z: f64) {
    node.attrs.set("z", c_lf(72.0 * z), false);
}

/// `printf("%lf", v)`.
fn c_lf(v: f64) -> String {
    graphviz_pure_core::cnum::fmt_f(v, 6)
}

/// `agnameof(g)` of the root graph.
fn root_name(g: &LGraph<'_>) -> String {
    g.src.root_name().to_string()
}

/// `neatoMode(g)`.
fn neato_mode(g: &LGraph<'_>) -> i32 {
    match g.gattr("mode") {
        None | Some("") | Some("major") => MODE_MAJOR,
        Some("KK") => MODE_KK,
        Some("sgd") => MODE_SGD,
        Some("hier") => MODE_HIER,
        Some("ipsep") => MODE_IPSEP,
        Some(s) => {
            graphviz_pure_core::diag::agwarningf(&format!(
                "Illegal value {s} for attribute \"mode\" in graph {} - ignored\n",
                root_name(g)
            ));
            MODE_MAJOR
        }
    }
}

/// `neatoModel(g)`.
fn neato_model(g: &LGraph<'_>) -> i32 {
    match g.gattr("model") {
        None | Some("") | Some("shortpath") => MODEL_SHORTPATH,
        Some("circuit") => MODEL_CIRCUIT,
        Some("subset") => MODEL_SUBSET,
        Some("mds") => {
            if g.edge_attr_declared("len") {
                MODEL_MDS
            } else {
                graphviz_pure_core::diag::agwarningf(&format!(
                    "edges in graph {} have no len attribute. Hence, the mds model\n",
                    root_name(g)
                ));
                graphviz_pure_core::diag::agerr_prev(
                    "is inappropriate. Reverting to the shortest path model.\n",
                );
                MODEL_SHORTPATH
            }
        }
        Some(s) => {
            graphviz_pure_core::diag::agwarningf(&format!(
                "Unknown value {s} for attribute \"model\" in graph {} - ignored\n",
                root_name(g)
            ));
            MODEL_SHORTPATH
        }
    }
}

/// `setSeed(G, dflt, &seed)`.
pub fn set_seed(g: &LGraph<'_>, dflt: i32, seed: &mut i64) -> i32 {
    let Some(p) = g.gattr("start").filter(|s| !s.is_empty()) else {
        return dflt;
    };
    let mut init = dflt;
    let mut rest = p;
    let first = p.as_bytes()[0];
    if first.is_ascii_alphabetic() {
        if let Some(r) = p.strip_prefix("self") {
            init = INIT_SELF;
            rest = r;
        } else if let Some(r) = p.strip_prefix("regular") {
            init = INIT_REGULAR;
            rest = r;
        } else if let Some(r) = p.strip_prefix("random") {
            init = INIT_RANDOM;
            rest = r;
        }
    } else if first.is_ascii_digit() {
        init = INIT_RANDOM;
    }
    if init == INIT_RANDOM {
        // C seeds from time(NULL) when no number follows; the port stays
        // deterministic and uses 1 instead.
        *seed = match rest.as_bytes().first() {
            Some(b) if b.is_ascii_digit() => {
                crate::common::utils::parse_leading_int(rest).unwrap_or(1)
            }
            _ => 1,
        };
    }
    init
}

/// `initRegular(G, nG)`: nodes evenly on a circle.
fn init_regular(g: &mut LGraph<'_>, sub: &Sub, ng: usize) {
    let mut a = 0.0f64;
    let da = 2.0 * std::f64::consts::PI / ng as f64;
    for &n in &sub.nodes {
        g.nodes[n].pos[0] = ng as f64 * math::cos(a);
        g.nodes[n].pos[1] = ng as f64 * math::sin(a);
        g.nodes[n].pinned = P_SET;
        a += da;
        if g.ndim > 2 {
            for d in 2..g.ndim {
                g.nodes[n].pos[d] = ng as f64 * crate::cutil::drand48();
            }
        }
    }
}

/// `checkStart(G, nG, dflt)`: interpret `start` and seed `drand48`.
pub fn check_start(g: &mut LGraph<'_>, sub: &Sub, ng: usize, dflt: i32, _p: &NeatoParams) -> i32 {
    let mut seed = 1i64;
    let init = set_seed(g, dflt, &mut seed);
    // `N_pos`: the node attribute `pos` is declared.
    if g.node_attr_declared("pos") && init != INIT_RANDOM {
        graphviz_pure_core::diag::agwarningf("node positions are ignored unless start=random\n");
    }
    if init == INIT_REGULAR {
        init_regular(g, sub, ng);
    }
    crate::cutil::srand48(seed);
    init
}

/// `checkExp(G)`: the `stresswt` exponent (1 or 2).
fn check_exp(g: &LGraph<'_>) -> i32 {
    let e = late_int(&g.src.graph_attrs, "stresswt", 2, 0);
    if e == 0 || e > 2 {
        graphviz_pure_core::diag::agwarningf(
            "stresswt attribute value must be 1 or 2 - ignoring\n",
        );
        2
    } else {
        e
    }
}

/// `majorization(mg, g, nv, mode, model, dim, am)`.
fn majorization(
    g: &mut LGraph<'_>,
    sub: &Sub,
    nv: usize,
    mode: i32,
    model: i32,
    dim: usize,
    p: &NeatoParams,
    am: &AdjustData,
) {
    let init = check_start(
        g,
        sub,
        nv,
        if mode == MODE_HIER {
            INIT_SELF
        } else {
            INIT_RANDOM
        },
        p,
    );
    let mut opts = check_exp(g);
    if init == INIT_SELF {
        opts |= OPT_SMART_INIT;
    }
    let mut coords = vec![vec![0.0f64; nv]; dim];
    let verbose = graphviz_pure_core::diag::verbose_on;
    let say = graphviz_pure_core::diag::fprintf_stderr;
    let secs = || graphviz_pure_core::diag::fmt_f(graphviz_pure_core::diag::elapsed_sec(), 2);
    if verbose() {
        say(&format!(
            "model {model} smart_init {} stresswt {} iterations {} tol {}\nconvert graph: ",
            i32::from(init == INIT_SELF),
            opts & super::stress::OPT_EXP_FLAG,
            p.max_iter,
            graphviz_pure_core::diag::fmt_f(p.epsilon, 6)
        ));
        graphviz_pure_core::diag::start_timer();
        say("majorization\n");
    }
    let (gp, _ne, nodes) = make_graph_data(g, sub, mode, model);
    if verbose() {
        say(&format!("{nv} nodes {} sec\n", secs()));
    }
    let lgap = crate::common::utils::late_double(&g.src.graph_attrs, "levelsgap", 0.0, -f64::MAX);
    let rv = if mode == MODE_IPSEP {
        let mut opt = super::quad_prog_vpsc::IpsepOptions {
            edge_gap: lgap,
            nsize: sub
                .nodes
                .iter()
                .map(|&v| (g.nodes[v].width, g.nodes[v].height))
                .collect(),
            clusters: super::cola::cluster_map(g, sub),
            ..Default::default()
        };
        let s = g.gattr("diredgeconstraints");
        opt.diredges = if mapbool(s) {
            graphviz_pure_core::verbose_msg!("Generating Edge Constraints...\n");
            1
        } else if s.is_some_and(|s| {
            s.as_bytes()
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case(b"hier"))
        }) {
            graphviz_pure_core::verbose_msg!("Generating DiG-CoLa Edge Constraints...\n");
            2
        } else {
            0
        };
        opt.noverlap = match am.mode {
            super::adjust::AdjustMode::Ipsep => {
                graphviz_pure_core::verbose_msg!("Generating Non-overlap Constraints...\n");
                1
            }
            super::adjust::AdjustMode::Vpsc => {
                graphviz_pure_core::verbose_msg!("Removing overlaps as postprocess...\n");
                2
            }
            _ => 0,
        };
        let margin = super::adjust::sep_factor(g);
        opt.gap = if margin.do_add {
            (2.0 * (margin.x / 72.0), 2.0 * (margin.y / 72.0))
        } else {
            let d = 2.0 * (super::adjust::DFLT_MARGIN / 72.0);
            (d, d)
        };
        graphviz_pure_core::verbose_msg!(
            "gap={},{}\n",
            graphviz_pure_core::diag::fmt_f(opt.gap.0, 6),
            graphviz_pure_core::diag::fmt_f(opt.gap.1, 6)
        );
        super::cola::stress_majorization_cola(
            g,
            p,
            &gp,
            nv,
            &mut coords,
            &nodes,
            dim,
            model,
            p.max_iter,
            &mut opt,
        )
    } else if mode == MODE_HIER {
        super::digcola::stress_majorization_with_hierarchy(
            g,
            p,
            &gp,
            nv,
            &mut coords,
            &nodes,
            dim,
            opts,
            model,
            p.max_iter,
            lgap,
        )
    } else {
        stress_majorization_kd_mkernel(
            g,
            p,
            &gp,
            nv,
            &mut coords,
            &nodes,
            dim,
            opts,
            model,
            p.max_iter,
        )
    };
    if rv.is_ok() {
        for &v in &sub.nodes {
            let idx = g.nodes[v].id;
            for (i, c) in coords.iter().enumerate().take(g.ndim) {
                g.nodes[v].pos[i] = c[idx];
            }
        }
    }
}

/// `neatoLayout(mg, g, layoutMode, layoutModel, am)`: lay out one component.
fn neato_layout_component(
    g: &mut LGraph<'_>,
    sub: &Sub,
    name: &str,
    layout_mode: i32,
    model: i32,
    am: &AdjustData,
) {
    let max_iter = match g.gattr("maxiter") {
        // `MaxIter = atoi(str)`: an `int`, so 2147483648 reads as INT_MIN.
        Some(s) => i64::from(atoi(s)),
        None if layout_mode == MODE_MAJOR => DFLT_ITERATIONS,
        None if layout_mode == MODE_SGD => 30,
        None => 100 * sub.nodes.len() as i64,
    };
    let mut p = NeatoParams {
        max_iter,
        graph_name: name.to_string(),
        ..NeatoParams::default()
    };
    let (ng, kk) = scan_graph_mode(g, sub, layout_mode, &mut p);
    if ng < 2 || p.max_iter < 0 {
        return;
    }
    if layout_mode == MODE_KK {
        let mut kk = kk.expect("KK state");
        kk_neato(g, sub, &mut kk, ng, model, &p);
    } else if layout_mode == MODE_SGD {
        super::sgd::sgd(g, sub, model, &p);
    } else {
        let dim = g.ndim;
        majorization(g, sub, ng, layout_mode, model, dim, &p, am);
    }
}

pub(crate) fn is_a_cluster(sg: &graphviz_pure_core::SubgraphData) -> bool {
    graphviz_pure_core::has_cluster_prefix(&sg.name)
        || mapbool(sg.graph_attrs.get("cluster").map(String::as_str))
}

/// Top-level subgraphs of the root in creation order.
pub fn top_level_subgraphs(graph: &Graph) -> Vec<usize> {
    let mut child = vec![false; graph.subgraphs.len()];
    for sg in &graph.subgraphs {
        for c in &sg.subgraphs {
            child[c.0 as usize] = true;
        }
    }
    (0..graph.subgraphs.len()).filter(|&i| !child[i]).collect()
}

/// `addCluster(g)` (IPSEPCOLA builds): register the root's top-level
/// clusters and compute their boxes from member nodes and edges.
fn add_cluster(g: &mut LGraph<'_>) {
    let src = g.src;
    for si in top_level_subgraphs(src) {
        let sg = &src.subgraphs[si];
        if !is_a_cluster(sg) {
            continue;
        }
        let (label, label_pos) = crate::common::init::graph_label(
            &sg.graph_attrs,
            &src.subgraph_name(si),
            sg.html_name,
            false,
        );
        let mut border = [Point::default(); 4];
        if let Some(l) = &label {
            let d = crate::common::init::pad(l.dimen);
            border[if label_pos & LABEL_AT_TOP != 0 { 2 } else { 0 }] = d;
        }
        let ci = g.clusters.len();
        g.clusters.push(LCluster {
            subgraph: si,
            bb: Default::default(),
            label,
            label_pos,
            border,
            children: Vec::new(),
        });
        g.root_clusters.push(ci);
        // compute_bb(subg): nodes and the subgraph's own edges.
        let nodes = subgraph_nodes(src, si);
        if nodes.is_empty() {
            continue;
        }
        let m = f64::from(i32::MAX);
        let mut bb = graphviz_pure_core::BoundingBox::new(m, m, -m, -m);
        for &n in &nodes {
            let nd = &g.nodes[n];
            let c = crate::common::utils::coord(g, n);
            let b = graphviz_pure_core::BoundingBox::new(
                c.x - (nd.lw + nd.rw) / 2.0,
                c.y - nd.ht / 2.0,
                c.x + (nd.lw + nd.rw) / 2.0,
                c.y + nd.ht / 2.0,
            );
            crate::common::geom::expandbb(&mut bb, &b);
        }
        for e in subgraph_edges(src, si) {
            let ed = &g.edges[e];
            if let Some(spl) = &ed.spl {
                for b in spl {
                    for pt in &b.list {
                        crate::common::geom::expandbp(&mut bb, *pt);
                    }
                }
            }
            for l in [&ed.label, &ed.head_label, &ed.tail_label, &ed.xlabel]
                .into_iter()
                .flatten()
            {
                if l.set {
                    crate::common::splines::add_label_bb(&mut bb, l, false);
                }
            }
        }
        g.clusters[ci].bb = bb;
    }
}

/// Nodes of a subgraph including nested subgraphs, in sequence order
/// (`agfstnode(subg)`..`agnxtnode`). The subgraph's list already holds its descendants'.
pub fn subgraph_nodes(graph: &Graph, si: usize) -> Vec<usize> {
    let mut v: Vec<usize> = graph.subgraphs[si]
        .nodes
        .iter()
        .map(|n| n.0 as usize)
        .collect();
    v.sort_unstable();
    v
}

/// Edges of a subgraph including nested subgraphs, in sequence order. The subgraph's list
/// already holds its descendants'.
pub fn subgraph_edges(graph: &Graph, si: usize) -> Vec<usize> {
    let mut v: Vec<usize> = graph.subgraphs[si]
        .edges
        .iter()
        .map(|e| e.0 as usize)
        .collect();
    v.sort_unstable();
    v
}

/// `doEdges(g)`: route edges without translating first.
fn do_edges(g: &mut LGraph<'_>, sub: &mut Sub, is_root: bool) {
    compute_bb(g, sub, is_root);
    spline_edges0(g, sub, is_root, true);
}

/// `numFields(pos)`: white-space separated tokens up to the next `;`. A `;` after white
/// space is a token of its own, as C counts any byte there before it stops.
fn num_fields(b: &[u8]) -> usize {
    let mut rest = b;
    let mut cnt = 0usize;
    loop {
        rest = skip_c_space(rest);
        if rest.is_empty() {
            return cnt;
        }
        cnt = cnt.saturating_add(1);
        let end = rest
            .iter()
            .position(|&c| is_c_space(c) || c == b';')
            .unwrap_or(rest.len());
        rest = rest.get(end..).unwrap_or_default();
        if !rest.first().copied().is_some_and(is_c_space) {
            return cnt;
        }
    }
}

/// `while (gv_isspace(*pos)) pos++;`
fn skip_c_space(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|&&c| is_c_space(c)).count();
    b.get(n..).unwrap_or_default()
}

/// `gv_isspace`: C's `isspace` in the C locale.
fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// `sscanf(s, "%lf,%lf%n", ...) == 2`: the point and the rest after it.
fn scan_point(s: &str) -> Option<(Point, &str)> {
    let r = sscanf(s, "%lf,%lf");
    (r.count == 2).then(|| (Point::new(r.values[0], r.values[1]), r.rest))
}

/// Why [`parse_user_spline`] rejected an edge `pos`.
#[derive(Debug, PartialEq)]
enum SplineError {
    /// `doesn't have 3n+1 points`.
    Count,
    /// `syntax error in pos attribute`.
    Syntax,
}

/// The parser of `user_spline`: the beziers of an edge `pos`, with `stype`/`etype` as
/// the arrow flags of their `s,`/`e,` points. `pos` is not empty: `user_spline` returns
/// before.
///
/// C walks `pos` a byte at a time and skips one byte after a segment, so the next segment can
/// start inside a multi-byte character. The port keeps a byte offset for the same reason:
/// text that does not start on a character boundary matches no `sscanf` pattern, as the
/// continuation bytes it starts with match none in C.
fn parse_user_spline(
    pos: &str,
    stype: u32,
    etype: u32,
) -> std::result::Result<Vec<Bezier>, SplineError> {
    debug_assert!(!pos.is_empty(), "user_spline returns before an empty pos");
    let bytes = pos.as_bytes();
    // The byte offset of `rest`, a tail of `pos` (or of `bytes`).
    let offset = |rest: usize| pos.len().saturating_sub(rest);
    let mut spl = Vec::new();
    let mut at = 0;
    // The `s,` and `e,` points persist across `;`-separated beziers: a later one without
    // its own reuses the previous one.
    let (mut sflag, mut eflag) = (false, false);
    let (mut sp, mut ep) = (Point::default(), Point::default());
    loop {
        if let Some(r) = pos.get(at..).and_then(|r| r.strip_prefix("s,"))
            && let Some((p, r)) = scan_point(r)
        {
            sflag = true;
            sp = p;
            at = offset(r.len());
        }
        let e_at = offset(skip_c_space(bytes.get(at..).unwrap_or_default()).len());
        if let Some(r) = pos.get(e_at..).and_then(|r| r.strip_prefix("e,"))
            && let Some((p, r)) = scan_point(r)
        {
            eflag = true;
            ep = p;
            at = offset(r.len());
        }
        let npts = num_fields(bytes.get(at..).unwrap_or_default());
        if npts < 4 || npts % 3 != 1 {
            return Err(SplineError::Count);
        }
        let mut list = Vec::with_capacity(npts);
        for _ in 0..npts {
            let (p, r) = pos
                .get(at..)
                .and_then(scan_point)
                .ok_or(SplineError::Syntax)?;
            list.push(p);
            at = offset(r.len());
        }
        let rest = skip_c_space(bytes.get(at..).unwrap_or_default());
        // C steps over the byte after the bezier, whatever it is.
        let more = !rest.is_empty();
        at = offset(rest.len().saturating_sub(1));
        spl.push(Bezier {
            list,
            sflag: if sflag { stype } else { 0 },
            eflag: if eflag { etype } else { 0 },
            sp,
            ep,
        });
        if !more {
            return Ok(spl);
        }
    }
}

/// `set_label(obj, l, name)`: the label's position from the `name` attribute.
fn set_label(l: Option<&mut TextLabel>, lp: Option<&str>) {
    if let Some(l) = l
        && let Some((p, _)) = lp.and_then(scan_point)
    {
        l.pos = p;
        l.set = true;
    }
}

/// `user_spline(E_pos, e)`: use the edge's `pos` as its spline; false if it has none or
/// it is unreadable.
fn user_spline(g: &mut LGraph<'_>, e: usize) -> bool {
    use crate::common::splines::SplineGraph;
    // A `static atomic_flag`: one warning per process.
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let pos = g.eattr(e, "pos").unwrap_or("");
    if pos.is_empty() {
        return false;
    }
    // neatoinit.c:278: after the empty `pos` test, so only an edge with one warns.
    let (stype, etype) = g.arrow_flags(e);
    let spl = match parse_user_spline(pos, stype, etype) {
        Ok(spl) => spl,
        Err(err) => {
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let (t, h) = g.edges.get(e).map_or(
                    (
                        graphviz_pure_core::AgName::own(""),
                        graphviz_pure_core::AgName::own(""),
                    ),
                    |ed| (g.node_agnameof(ed.tail), g.node_agnameof(ed.head)),
                );
                // neatoinit.c:298, :307: both names share agnameof's static buffer when
                // anonymous.
                let [t, h] = graphviz_pure_core::agnameof_buffer([t, h]);
                graphviz_pure_core::diag::agwarningf(&match err {
                    SplineError::Count => {
                        format!("pos attribute for edge ({t},{h}) doesn't have 3n+1 points\n")
                    }
                    _ => format!("syntax error in pos attribute for edge ({t},{h})\n"),
                });
            }
            return false;
        }
    };
    let attr = |k: &str| g.eattr(e, k);
    let (lp, xlp, head_lp, tail_lp) = (attr("lp"), attr("xlp"), attr("head_lp"), attr("tail_lp"));
    let Some(ed) = g.edges.get_mut(e) else {
        return false;
    };
    ed.spl = Some(spl);
    set_label(ed.label.as_mut(), lp);
    set_label(ed.xlabel.as_mut(), xlp);
    set_label(ed.head_label.as_mut(), head_lp);
    set_label(ed.tail_label.as_mut(), tail_lp);
    true
}

/// How many edges have their position (`pos_edge`), with C's names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum PosEdge {
    NoEdges,
    SomeEdges,
    AllEdges,
}

/// `nop_init_edges(g)`: with `Nop > 1`, the edges' splines and label positions from
/// their `pos`, `lp`, `xlp`, `head_lp` and `tail_lp`.
fn nop_init_edges(g: &mut LGraph<'_>, nop: u8) -> PosEdge {
    if g.edges.is_empty() {
        return PosEdge::AllEdges;
    }
    if !g.edge_attr_declared("pos") || nop < 2 {
        return PosEdge::NoEdges;
    }
    let mut nedges = 0usize;
    for e in g.out.concat() {
        if user_spline(g, e) {
            nedges = nedges.saturating_add(1);
        }
    }
    if nedges == 0 {
        PosEdge::NoEdges
    } else if nedges == g.edges.len() {
        PosEdge::AllEdges
    } else {
        PosEdge::SomeEdges
    }
}

/// `freeEdgeInfo(g)`: drop the given splines and the label positions read with them. C
/// frees the edge labels but keeps the pointers and reads them again: it crashes, or
/// places them from freed memory. The port places them again on the new edges, as C does
/// for the same edge without `pos`.
fn free_edge_info(g: &mut LGraph<'_>) {
    for e in &mut g.edges {
        e.spl = None;
        for l in [
            &mut e.label,
            &mut e.xlabel,
            &mut e.head_label,
            &mut e.tail_label,
        ]
        .into_iter()
        .flatten()
        {
            l.set = false;
        }
    }
}

/// `chkBB(g, G_bb, &bb)`: a cluster's `bb`, normalised when it was written with `-y`.
fn chk_bb(s: Option<&str>) -> Option<graphviz_pure_core::BoundingBox> {
    let r = sscanf(s?, "%lf,%lf,%lf,%lf");
    let [llx, mut lly, urx, mut ury, _] = r.values;
    if r.count != 4 {
        return None;
    }
    if lly > ury {
        std::mem::swap(&mut lly, &mut ury);
    }
    Some(graphviz_pure_core::BoundingBox::new(llx, lly, urx, ury))
}

/// `dfs(subg, parentg, G_lp, G_bb)`: a cluster with a readable `bb` is registered below
/// `parent` (`add_cluster`, `do_graph_label`) and read in turn (`nop_init_graphs`);
/// another subgraph passes its subgraphs on to `parent`.
fn nop_dfs(g: &mut LGraph<'_>, si: usize, parent: Option<usize>) {
    let src = g.src;
    let Some(sg) = src.subgraphs.get(si) else {
        return;
    };
    let Some(bb) = is_a_cluster(sg)
        .then(|| chk_bb(sg.graph_attrs.get("bb").map(String::as_str)))
        .flatten()
    else {
        for c in &sg.subgraphs {
            nop_dfs(g, c.0 as usize, parent);
        }
        return;
    };
    let (mut label, label_pos) = crate::common::init::graph_label(
        &sg.graph_attrs,
        &src.subgraph_name(si),
        sg.html_name,
        false,
    );
    let mut border = [Point::default(); 4];
    let [bottom, _, top, _] = &mut border;
    if let Some(l) = &label {
        *if label_pos & LABEL_AT_TOP != 0 {
            top
        } else {
            bottom
        } = crate::common::init::pad(l.dimen);
    }
    set_label(label.as_mut(), sg.graph_attrs.get("lp").map(String::as_str));
    let ci = g.clusters.len();
    g.clusters.push(LCluster {
        subgraph: si,
        bb,
        label,
        label_pos,
        border,
        children: Vec::new(),
    });
    match parent.and_then(|p| g.clusters.get_mut(p)) {
        Some(p) => p.children.push(ci),
        None => g.root_clusters.push(ci),
    }
    for c in &sg.subgraphs {
        nop_dfs(g, c.0 as usize, Some(ci));
    }
}

/// `nop_init_graphs(g, G_lp, G_bb)` on the root: its label's `lp`, and the clusters with a
/// `bb` (`init_nop` declares `bb`, so they are always read).
fn nop_init_graphs(g: &mut LGraph<'_>) {
    let lp = g.gattr("lp");
    set_label(g.label.as_mut(), lp);
    for si in top_level_subgraphs(g.src) {
        nop_dfs(g, si, None);
    }
}

/// `init_nop(g, 1)`: keep the nodes where `pos` put them, then add what the input lacks.
/// Whether `State` became `GVSPLINES`; `Err` after the error for a node without a
/// position. `xdotBB` sets the boxes of the background's ops in place.
fn init_nop(
    g: &mut LGraph<'_>,
    nop: u8,
    mut background: Option<&mut graphviz_pure_core::xdot::Background>,
) -> std::result::Result<bool, ()> {
    let translate = !mapbool(g.gattr("notranslate"));
    // scan_graph (Reduce, edge lengths) ran in neato_layout_nop.
    let src = g.src;
    for (i, (node, sn)) in g.nodes.iter_mut().zip(&src.nodes).enumerate() {
        if node.pinned == 0 && !sn.name.starts_with("cluster") {
            // neatoinit.c:529-530: both names share agnameof's static buffer when anonymous.
            let [n, gname] = graphviz_pure_core::agnameof_buffer([
                src.agnameof(graphviz_pure_core::Obj::Node(graphviz_pure_core::NodeId(
                    i as u32,
                ))),
                src.agnameof(graphviz_pure_core::Obj::Root),
            ]);
            graphviz_pure_core::diag::agerrorf(&format!(
                "node {n} in graph {gname} has no position\n"
            ));
            return Err(());
        }
        set_label(
            node.xlabel.as_mut(),
            sn.attrs.get("xlp").map(String::as_str),
        );
    }
    nop_init_graphs(g);
    let mut pos_edges = nop_init_edges(g, nop);
    if background.is_some() {
        // Turn off any aspect change if background present.
        g.drawing.ratio_kind = RatioKind::None;
    }
    let did_adjust = if nop == 1 && background.is_none() {
        let root = g.root_sub();
        super::adjust::adjust_nodes(g, &root)
    } else {
        false
    };
    if did_adjust && let Some(l) = g.label.as_mut() {
        l.set = false;
    }
    let mut root = g.root_sub();
    compute_bb(g, &mut root, true);
    // Adjust bounding box for any background.
    if let Some(xd) = background.as_deref_mut() {
        let bb = ((g.bb.ll.x, g.bb.ll.y), (g.bb.ur.x, g.bb.ur.y));
        let (ll, ur) = graphviz_pure_core::xdot::xdot_bb(xd, bb);
        g.bb.ll = Point::new(ll.0, ll.1);
        g.bb.ur = Point::new(ur.0, ur.1);
    }
    if translate && background.is_none() && (g.bb.ll.x != 0.0 || g.bb.ll.y != 0.0) {
        neato_translate(g);
    }
    let root = g.root_sub();
    let did_shift = neato_set_aspect(g, &root, true);
    if pos_edges != PosEdge::NoEdges && (did_shift || did_adjust) {
        free_edge_info(g);
        pos_edges = PosEdge::NoEdges;
    }
    if pos_edges != PosEdge::AllEdges || nop == 3 {
        let root = g.root_sub();
        spline_edges0(g, &root, true, false);
        // splineEdges sets State; spline_edges0 returns before it with splines="".
        Ok(g.edge_type != EdgeType::None)
    } else {
        Ok(true)
    }
}

/// `neato_layout(g)` with `Nop`: `init_nop(g, 1)`, then `gv_postprocess(g, 0)`. A node
/// without a position ends the layout of this graph only, which is still drawn.
fn neato_layout_nop(graph: &mut Graph) {
    let nop = super::neatosplines::nop();
    let attrs = &graph.graph_attrs;
    let outdim = late_int(attrs, "dimen", 2, 2);
    let ndim = late_int(attrs, "dim", outdim, 2).min(MAXDIM as i32) as usize;
    // PSinputscale = POINTS_PER_INCH: `pos` is in points.
    let inputscale = 72.0;
    // neato_init_graph: user_pos on every node, then addZ, before init_nop's scan_graph
    // applies `Reduce`.
    let full = graph.clone();
    let decl = PosDecl::of(&full);
    let g0 = neato_init_node_edge(&full, ndim, inputscale, decl);
    add_z(&g0, decl.z, graph);
    // init_nop's scan_graph, before `Reduce` deletes nodes
    graphviz_pure_core::verbose_msg!(
        "Scanning graph {}, {} nodes\n",
        graph.root_name(),
        graph.nodes.len()
    );
    let reduced;
    let mut g = match super::stuff::reduce_set(graph) {
        None => g0,
        Some(deleted) => {
            graph.delete_nodes(&deleted);
            reduced = graph.clone();
            let mut g =
                graphviz_pure_core::diag::silently(|| LGraph::new(&reduced, ndim, EdgeType::Line));
            let kept = g0.nodes.iter().zip(&deleted).filter(|(_, d)| !**d);
            for (node, (old, _)) in g.nodes.iter_mut().zip(kept) {
                node.pos.clone_from(&old.pos);
                node.pinned = old.pinned;
            }
            g
        }
    };
    // The rest of init_nop's scan_graph: its `bad edge len` warnings.
    super::stuff::nop_check_edge_lens(graph);
    // `GD_drawing(g)->xdots`, whose op boxes `init_nop`'s `xdotBB` sets.
    let mut background = graph.layout.background.take();
    let done = init_nop(&mut g, nop, background.as_mut());
    graph.layout.background = background;
    let Ok(splines_state) = done else {
        graphviz_pure_core::diag::agerr_prev("as required by the -n flag\n");
        // `neato_layout` returns before any coordinate, spline or cluster is set, and
        // `dot` draws that: every node at the origin, no edge, the root label unplaced.
        // `neato_init_graph` set GD_odim, and `user_pos` the coordinates after `y`.
        let odim = (outdim as usize).min(ndim);
        graph.layout.odim = odim;
        for (nd, n) in graph.nodes.iter_mut().zip(&g.nodes) {
            nd.pos = Point::default();
            nd.width = n.width;
            nd.height = n.height;
            nd.layout.higher_dims = crate::common::init::higher_dims(&n.pos, odim);
        }
        for e in &mut graph.edges {
            e.spline = graphviz_pure_core::Spline::default();
            e.label_pos = None;
        }
        for sg in graph.subgraphs.iter_mut().filter(|s| s.is_cluster) {
            sg.layout.reg = graphviz_pure_core::ClusterReg::Unregistered;
        }
        graph.bb = graphviz_pure_core::BoundingBox::default();
        graph.layout.label_pos = Some(Point::default());
        return;
    };
    gv_postprocess(&mut g, false);
    // GD_odim(g->root) = MIN(outdim, Ndim)
    g.odim = (outdim as usize).min(ndim);
    g.write_back(graph);
    // With State below GVSPLINES, attach_attrs_and_arrows writes no edge position: the
    // given splines are drawn, and the edges keep their input attributes, as an IGNORED
    // edge's do.
    if !splines_state {
        for (e, ge) in graph.edges.iter_mut().zip(&g.edges) {
            e.layout.no_attrs = ge.spl.is_some();
        }
    }
}

/// `nop1_layout` and `nop2_layout` (`plugin/neato_layout/gvlayout_neato_layout.c:48-57`):
/// `neato_layout` with `Nop` set to `nop`, then `Nop = 0`, whatever `-n` set it to.
pub fn nop_layout(graph: &mut Graph, nop: u8) -> Result<()> {
    let _clear = super::neatosplines::ClearNop::new();
    super::neatosplines::set_nop(nop);
    neato_layout(graph)
}

/// `neato_layout(g)`, with `Nop` (the `-n` level) as the run set it.
pub fn neato_layout(graph: &mut Graph) -> Result<()> {
    if super::neatosplines::nop() > 0 {
        neato_layout_nop(graph);
        return Ok(());
    }
    let attrs = &graph.graph_attrs;
    let no_translate = mapbool(attrs.get("notranslate").map(String::as_str));
    let inputscale = crate::common::utils::get_inputscale(attrs);
    let outdim = late_int(attrs, "dimen", 2, 2);
    let ndim = late_int(attrs, "dim", outdim, 2).min(MAXDIM as i32) as usize;

    // `Reduce` (`dot -x`) deletes nodes in `scan_graph_mode`, per component once `pccomps`
    // has split the graph: the components are those of the whole graph, and one it empties
    // is still packed.
    let reduced = super::stuff::reduce_set(graph).map(|deleted| {
        let full = graph.clone();
        // `user_pos` on the whole graph, with what it declares before `Reduce` deletes nodes.
        let g0 = neato_init_node_edge(&full, ndim, inputscale, PosDecl::of(&full));
        let (ccs, pin) = pccomps(&g0, CC_PFX);
        let mut new_index = vec![None; deleted.len()];
        let kept = deleted.iter().enumerate().filter(|(_, d)| !**d);
        for (new, (old, _)) in kept.enumerate() {
            if let Some(slot) = new_index.get_mut(old) {
                *slot = Some(new);
            }
        }
        let comps: Vec<Vec<usize>> = ccs
            .iter()
            .map(|c| {
                c.nodes
                    .iter()
                    .filter_map(|&v| new_index.get(v).copied().flatten())
                    .collect()
            })
            .collect();
        // `user_pos` ran on the whole graph, before `scan_graph_mode`: its positions, pins
        // and `drand48` draws (`nG` is the whole graph's count) are kept.
        let kept_pos: Vec<(Vec<f64>, u8)> = g0
            .nodes
            .iter()
            .zip(&deleted)
            .filter(|(_, d)| !**d)
            .map(|(n, _)| (n.pos.clone(), n.pinned))
            .collect();
        graph.delete_nodes(&deleted);
        (comps, pin, kept_pos)
    });
    let src = graph.clone();
    // What the graph declares after `Reduce`: a graph without attribute dictionaries
    // declares what its remaining nodes carry.
    let decl = PosDecl::of(&src);
    let mut g = match &reduced {
        // `user_pos` ran on the whole graph, before `scan_graph_mode` deleted nodes.
        Some((_, _, kept_pos)) => {
            let mut g =
                graphviz_pure_core::diag::silently(|| LGraph::new(&src, ndim, EdgeType::Line));
            for (node, (pos, pinned)) in g.nodes.iter_mut().zip(kept_pos) {
                node.pos.clone_from(pos);
                node.pinned = *pinned;
            }
            g
        }
        None => neato_init_node_edge(&src, ndim, inputscale, decl),
    };
    let z_declared = decl.z;
    let layout_mode = neato_mode(&g);
    let am = graph_adjust_mode(&g, None);
    let model = neato_model(&g);
    let mut pinfo = PackInfo::default();
    let mode = get_pack_mode_info(g.gattr("packmode"), PackMode::Undef, &mut pinfo);
    let mut pack = get_pack(g.gattr("pack"), -1, CL_OFFSET);
    if mode == PackMode::Undef {
        if pack < 0 && layout_mode != MODE_KK {
            pack = CL_OFFSET;
        }
        pinfo.mode = PackMode::Node;
    } else if pack < 0 {
        pack = CL_OFFSET;
    }

    if pack >= 0 {
        let (mut ccs, pin) = match reduced {
            Some((comps, pin, _)) => {
                let n = g.nodes.len();
                let subs = comps.into_iter().enumerate().map(|(i, c)| {
                    let mut s = Sub::new(n, c);
                    s.name = format!("{CC_PFX}{i}");
                    s
                });
                (subs.collect(), pin)
            }
            None => pccomps(&g, CC_PFX),
        };
        if ccs.len() > 1 {
            for (i, gc) in ccs.iter_mut().enumerate() {
                layout_and_route(&mut g, gc, Some(i), layout_mode, model, &am, no_translate);
            }
            let mut fixed = None;
            if pin {
                let mut bp = vec![false; ccs.len()];
                bp[0] = true;
                fixed = Some(bp);
            }
            pinfo.margin = pack as u32;
            pinfo.fixed = fixed;
            pinfo.do_splines = true;
            pack_graphs(&mut g, &mut ccs, &pinfo);
        } else {
            let mut root = g.root_sub();
            layout_and_route(
                &mut g,
                &mut root,
                None,
                layout_mode,
                model,
                &am,
                no_translate,
            );
        }
        let mut root = g.root_sub();
        compute_bb(&mut g, &mut root, true);
        add_z(&g, z_declared, graph);
        add_cluster(&mut g);
    } else {
        let mut root = g.root_sub();
        layout_and_route(
            &mut g,
            &mut root,
            None,
            layout_mode,
            model,
            &am,
            no_translate,
        );
        // C adds z before routing the edges, which does not move the nodes.
        add_z(&g, z_declared, graph);
    }
    gv_postprocess(&mut g, !no_translate);
    // GD_odim(g->root) = MIN(outdim, Ndim)
    g.odim = (outdim as usize).min(ndim);
    g.write_back(graph);
    Ok(())
}

/// Lays out and routes `sub`: the root graph, or the component `pccomps` named
/// `_neato_cc<cc>`.
fn layout_and_route(
    g: &mut LGraph<'_>,
    sub: &mut Sub,
    cc: Option<usize>,
    layout_mode: i32,
    model: i32,
    am: &AdjustData,
    no_translate: bool,
) {
    let is_root = cc.is_none();
    let name = match cc {
        Some(i) => format!("_neato_cc{i}"),
        None => root_name(g),
    };
    neato_layout_component(g, sub, &name, layout_mode, model, am);
    remove_overlap_with(g, sub, am);
    if !is_root {
        crate::common::init::component_set_edge_type(g);
    }
    if no_translate {
        do_edges(g, sub, is_root);
    } else {
        spline_edges(g, sub, is_root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutil::{Drand48State, RandState, with_rand_state};
    use graphviz_pure_core::{AttrMap, EdgeId, GraphKind, NodeId, SubgraphData, SubgraphId};

    /// `user_pos` in a fresh process; with the generator state it leaves.
    fn user_pos_of(p: &str, z: Option<&str>, ndim: usize) -> (Option<u8>, Vec<f64>, RandState) {
        let mut state = RandState::default();
        let mut pvec = vec![0.0; ndim];
        let pinned = with_rand_state(&mut state, || {
            graphviz_pure_core::diag::quietly(|| {
                user_pos_values(p, z, false, "n", -1.0, 4, &mut pvec)
            })
        });
        (pinned, pvec, state)
    }

    // Issue #245: neatoinit.c's user_pos with Ndim > 2 and two values in `pos`.
    #[test]
    fn user_pos_takes_z_or_a_random_third_coordinate() {
        let mut fresh = Drand48State::default();
        // `z` is read and no random number is drawn.
        let (pinned, pvec, state) = user_pos_of("1,2", Some("3"), 3);
        assert_eq!(
            (pinned, pvec, state),
            (Some(P_SET), vec![1.0, 2.0, 3.0], RandState::default())
        );
        // No usable `z`: jitter3d, nG * drand48().
        let (_, pvec, _) = user_pos_of("1,2!", Some("x"), 3);
        assert_eq!(pvec, vec![1.0, 2.0, 4.0 * fresh.drand48()]);
        // Three values in `pos`: `z` is not read; with Ndim 4, jitter_d from the fourth on.
        let mut fresh = Drand48State::default();
        let (pinned, pvec, _) = user_pos_of("0x1p3,1e1,nan(1)!", Some("3"), 4);
        assert_eq!(pinned, Some(P_PIN));
        assert_eq!(pvec[..2], [8.0, 10.0]);
        assert!(pvec[2].is_nan());
        assert_eq!(pvec[3], 4.0 * fresh.drand48());
    }

    // sscanf assigns what it read before failing: `pos="5"` leaves ND_pos[0] at 5.
    #[test]
    fn user_pos_keeps_the_values_of_a_failed_scan() {
        let (pinned, pvec, _) = user_pos_of("5", None, 2);
        assert_eq!((pinned, pvec), (None, vec![5.0, 0.0]));
    }

    fn parse(pos: &str) -> std::result::Result<Vec<Bezier>, SplineError> {
        parse_user_spline(pos, 1, 1)
    }

    // Issue #220: user_spline's sscanf("%lf,%lf%n") reads hexadecimal floats and `nan(chars)`.
    #[test]
    fn user_spline_reads_hex_and_nan_chars_like_c() {
        let s = parse("0x10,0 10,10 20,20 30,30").unwrap();
        assert_eq!(s[0].list[0], Point::new(16.0, 0.0));
        let s = parse("nan(1),0 10,10 20,20 0x1p-1,30").unwrap();
        assert!(s[0].list[0].x.is_nan());
        assert_eq!(s[0].list[3], Point::new(0.5, 30.0));
        assert_eq!(scan_point("0x1.8p1,2x").map(|x| x.1), Some("x"));
    }

    #[test]
    fn user_spline_parses_arrows_and_multiple_beziers() {
        let s = parse("s,1,2 e,9,9 1,1 2,2 3,3 4,4;5,5 6,6 7,7 8,8").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].sflag, s[0].sp), (1, Point::new(1.0, 2.0)));
        assert_eq!((s[0].eflag, s[0].ep), (1, Point::new(9.0, 9.0)));
        // The `s,` and `e,` points carry over to the next bezier.
        assert_eq!((s[1].sflag, s[1].eflag), (1, 1));
        assert_eq!(s[1].list.len(), 4);
        // An `s,` point without a tail arrow gets `sflag = stype`, 0.
        let s = parse_user_spline("s,1,2 1,1 2,2 3,3 4,4", 0, 1).unwrap();
        assert_eq!(s[0].sflag, 0);
        assert_eq!(parse("1,1 2,2 3,3").err(), Some(SplineError::Count));
        assert_eq!(parse("1,1 2,2 x,3 4,4").err(), Some(SplineError::Syntax));
        // numFields counts a ';' after white space as a field.
        let p = "0,0 10,10 20,20 30,30";
        assert_eq!(parse(&format!("{p} ;{p}")).err(), Some(SplineError::Count));
        assert_eq!(parse("0,0 10,10 20,20 ;").err(), Some(SplineError::Syntax));
        assert_eq!(parse(&format!("{p};{p}")).unwrap().len(), 2);
        assert_eq!(parse(&format!("{p} \x0b;")).err(), Some(SplineError::Count));
    }

    // Issue #181: C skips one byte after a segment, so a segment that ends in a multi-byte
    // character leaves its continuation bytes as the next segment. The port sliced the
    // `str` at that byte and panicked.
    #[test]
    fn user_spline_skips_one_byte_after_a_segment_like_c() {
        let p = "0,0 10,10 20,20 30,30";
        // One field left (the continuation byte): not 3n+1.
        assert_eq!(parse(&format!("{p}é")).err(), Some(SplineError::Count));
        // The continuation byte is a field of its own, so five fields, not a second segment.
        assert_eq!(
            parse(&format!("{p}é 1,1 2,2 3,3 4,4")).err(),
            Some(SplineError::Count)
        );
        // Four fields, the first starting with the continuation byte: a syntax error.
        assert_eq!(
            parse(&format!("{p}é 2,2 3,3 4,4")).err(),
            Some(SplineError::Syntax)
        );
        assert_eq!(
            parse(&format!("{p}é1,1 2,2 3,3 4,4")).err(),
            Some(SplineError::Syntax)
        );
        // `s,` and `e,` do not match after the skipped byte either.
        assert_eq!(
            parse(&format!("{p}és,1,1 2,2 3,3 4,4")).err(),
            Some(SplineError::Syntax)
        );
    }

    type Attrs<'a> = &'a [(&'a str, &'a str)];

    /// A graph for `neato -n`: graph attributes, nodes and edges (tail, head, attributes).
    fn nop_graph(
        directed: bool,
        gattrs: Attrs<'_>,
        nodes: &[(&str, Attrs<'_>)],
        edges: &[(usize, usize, Attrs<'_>)],
    ) -> Graph {
        use graphviz_pure_core::{AttrMap, GraphKind, NodeId};
        let kind = if directed {
            GraphKind::Directed
        } else {
            GraphKind::Undirected
        };
        let mut g = Graph::new("G", kind, false);
        let map = |a: Attrs<'_>| -> AttrMap {
            AttrMap::from_plain(a.iter().map(|(k, v)| (k.to_string(), v.to_string())))
        };
        g.graph_attrs = map(gattrs);
        for (name, a) in nodes {
            let n = g.ensure_node(name);
            g.nodes[n.0 as usize].attrs = map(a);
        }
        for &(t, h, a) in edges {
            g.add_edge(NodeId(t as u32), NodeId(h as u32), map(a));
        }
        g
    }

    fn nop_layout(mut g: Graph, nop: u8) -> Graph {
        let mut nop = nop;
        graphviz_pure_core::diag::quietly(|| {
            super::super::neatosplines::with_nop(&mut nop, || neato_layout(&mut g))
        })
        .unwrap();
        g
    }

    fn near(p: Point, x: f64, y: f64) -> bool {
        (p.x - x).abs() < 0.01 && (p.y - y).abs() < 0.01
    }

    #[test]
    fn nop_translates_to_origin_and_keeps_edges() {
        let g = nop_graph(
            false,
            &[],
            &[("a", &[("pos", "127,118")]), ("b", &[("pos", "227,118")])],
            &[(0, 1, &[("pos", "127,118 150,118 180,118 227,118")])],
        );
        let g = nop_layout(g, 2);
        assert_eq!(g.nodes[0].pos, Point::new(27.0, 18.0));
        assert_eq!(g.bb.ll, Point::new(0.0, 0.0));
        assert_eq!(
            g.edges[0].spline.segments[0].points[0],
            Point::new(27.0, 18.0)
        );
    }

    // Issue #149: init_nop runs adjustNodes for -n1, so `overlap=false` moves b to x =
    // 2.8143in, as C 16.1.0 does.
    #[test]
    fn nop1_removes_overlaps_like_c() {
        let g = nop_graph(
            false,
            &[("overlap", "false")],
            &[("a", &[("pos", "0,0")]), ("b", &[("pos", "10,0")])],
            &[(0, 1, &[])],
        );
        let g = nop_layout(g, 1);
        assert!(near(g.nodes[0].pos, 27.0, 18.0), "{:?}", g.nodes[0].pos);
        assert!(
            near(g.nodes[1].pos, 2.8143 * 72.0, 18.0),
            "{:?}",
            g.nodes[1].pos
        );
    }

    // Issue #434: `computeScaleXY` (constraint.c:729) asserts when overlapping
    // nodes share a position. `-n` keeps the pins; the scale is not applied.
    #[test]
    fn nop1_scalexy_leaves_coincident_nodes_where_nop_put_them() {
        let nodes = |overlap: &[(&str, &str)]| {
            nop_graph(
                false,
                overlap,
                &[
                    ("a", &[("pos", "0,0")]),
                    ("b", &[("pos", "0,0")]),
                    ("c", &[("pos", "1,1")]),
                ],
                &[],
            )
        };
        let scaled = nop_layout(nodes(&[("overlap", "scalexy")]), 1);
        let plain = nop_layout(nodes(&[]), 1);
        assert_eq!(
            (
                scaled.nodes[0].pos,
                scaled.nodes[1].pos,
                scaled.nodes[2].pos
            ),
            (plain.nodes[0].pos, plain.nodes[1].pos, plain.nodes[2].pos)
        );
        assert!(scaled.nodes[0].pos.x.is_finite() && scaled.nodes[0].pos.y.is_finite());
        // Same x, different y: C scales in y. `-Tdot` positions are 27,18 and 27,62.
        let apart = nop_layout(
            nop_graph(
                false,
                &[("overlap", "scalexy")],
                &[("a", &[("pos", "0,0")]), ("b", &[("pos", "0,10")])],
                &[],
            ),
            1,
        );
        assert!(
            near(apart.nodes[0].pos, 27.0, 18.0),
            "{:?}",
            apart.nodes[0].pos
        );
        assert!(
            near(apart.nodes[1].pos, 27.0, 62.0),
            "{:?}",
            apart.nodes[1].pos
        );
    }

    // Issue #293: setRatio compares `ratio` as it is, so " fill" sets no ratio: C's bb is
    // 0,0,84,66, not the 3in `size`.
    #[test]
    fn nop_does_not_trim_ratio() {
        let g = nop_graph(
            false,
            &[("ratio", " fill"), ("size", "3,3")],
            &[("a", &[("pos", "0,0")]), ("b", &[("pos", "30,30")])],
            &[(0, 1, &[])],
        );
        let g = nop_layout(g, 2);
        assert_eq!(
            (g.bb.ll, g.bb.ur),
            (Point::new(0.0, 0.0), Point::new(84.0, 66.0))
        );
    }

    // Issue #317: with -n3, spline_edges_ keeps the first bezier of the edge's `pos` and
    // clips and installs it again: C writes `pos="42,33 46.844,37.844 51.688,42.688
    // 56.531,47.531"`.
    #[test]
    fn nop3_clips_and_installs_the_first_bezier() {
        let g = nop_graph(
            false,
            &[],
            &[("a", &[("pos", "0,0")]), ("b", &[("pos", "100,100")])],
            &[(
                0,
                1,
                &[("pos", "0,0 10,10 20,20 30,30;40,40 50,50 60,60 70,70")],
            )],
        );
        let g = nop_layout(g, 3);
        let segs = &g.edges[0].spline.segments;
        assert_eq!(segs.len(), 1);
        let want = [
            (42.0, 33.0),
            (46.844, 37.844),
            (51.688, 42.688),
            (56.531, 47.531),
        ];
        for (p, (x, y)) in segs[0].points.iter().zip(want) {
            assert!(near(*p, x, y), "{:?}", segs[0].points);
        }
    }

    // Issue #266: gv_postprocess's addXLabels places a node xlabel that has no `xlp` and
    // grows the bb: C gives bb="-7,0,54,52.8" and xlp="-3.5,44.4".
    #[test]
    fn nop_places_node_xlabels() {
        let g = nop_graph(
            false,
            &[],
            &[("a", &[("pos", "0,0"), ("xlabel", "x")])],
            &[],
        );
        let g = nop_layout(g, 1);
        assert!(
            near(g.bb.ll, -7.0, 0.0) && near(g.bb.ur, 54.0, 52.8),
            "{:?}",
            g.bb
        );
        let xlp = g.nodes[0].layout.xlabel.expect("xlp");
        assert!(near(xlp, -3.5, 44.4), "{xlp:?}");
    }

    // freeEdgeInfo: after `ratio` rescales the drawing, the given splines are dropped and
    // the edges routed again. C reads the freed labels (it crashes or places them from freed
    // memory); the port places them again, as for the same edge without `pos`: on the new
    // edge, not at the scaled input `lp`.
    #[test]
    fn nop_rescale_places_the_edge_labels_again() {
        let g = nop_graph(
            false,
            &[("ratio", "2")],
            &[("a", &[("pos", "0,0")]), ("b", &[("pos", "100,0")])],
            &[(
                0,
                1,
                &[
                    ("pos", "0,0 30,0 60,0 100,0"),
                    ("label", "L"),
                    ("lp", "50,10"),
                ],
            )],
        );
        let g = nop_layout(g, 2);
        let e = &g.edges[0];
        // The rerouted edge is horizontal; C puts the label of the same edge without `pos`
        // 8.4pt above it.
        let y = e.spline.segments[0].points[0].y;
        let lp = e.label_pos.expect("label position");
        assert!(!e.label_unplaced, "{e:?}");
        assert!((lp.y - y - 8.4).abs() < 0.01, "{lp:?} vs edge at y = {y}");
    }

    // Issue #142: an edge label without `lp` is placed on the routed edge (addXLabels), at
    // 0.94179,0.36667in in C's -Tplain.
    #[test]
    fn nop_places_edge_labels() {
        let g = nop_graph(
            true,
            &[],
            &[("a", &[("pos", "0,0")]), ("b", &[("pos", "100,0")])],
            &[(0, 1, &[("label", "x")])],
        );
        let g = nop_layout(g, 1);
        let e = &g.edges[0];
        assert!(!e.label_unplaced);
        let lp = e.label_pos.expect("label position");
        assert!(near(lp, 0.94179 * 72.0, 0.36667 * 72.0), "{lp:?}");
    }

    // A graph built without attribute dictionaries declares what its nodes carry, and after
    // `Reduce` (`-x`) only what the remaining ones carry: a `z` on a deleted singleton is
    // not declared for addZ, so no remaining node gets one.
    #[test]
    fn reduce_declares_z_from_the_remaining_nodes() {
        crate::cutil::in_fresh_run(|| {
            let g = nop_graph(
                false,
                &[("dim", "3")],
                &[("a", &[]), ("b", &[]), ("c", &[]), ("d", &[("z", "5")])],
                &[(0, 1, &[]), (1, 2, &[]), (2, 0, &[])],
            );
            let mut g = g;
            super::super::stuff::set_reduce(true);
            let done = graphviz_pure_core::diag::quietly(|| neato_layout(&mut g));
            super::super::stuff::set_reduce(false);
            done.unwrap();
            assert_eq!(g.nodes.len(), 3);
            assert!(g.nodes.iter().all(|n| !n.attrs.contains_key("z")));
        });
    }

    // Issue #148: spline_edges_ routes `splines=ortho` through orthoEdges: C's edge a--c
    // has 7 points, from 0.375,0.50474in up and across.
    #[test]
    fn nop_routes_ortho_edges() {
        crate::cutil::in_fresh_run(|| {
            let g = nop_graph(
                false,
                &[("splines", "ortho")],
                &[
                    ("a", &[("pos", "0,0")]),
                    ("b", &[("pos", "100,0")]),
                    ("c", &[("pos", "200,100")]),
                ],
                &[(0, 2, &[]), (1, 2, &[])],
            );
            let g = nop_layout(g, 1);
            let pts = &g.edges[0].spline.segments[0].points;
            assert_eq!(pts.len(), 7, "{pts:?}");
            assert!(near(pts[0], 27.0, 0.50474 * 72.0), "{pts:?}");
            assert!(near(pts[6], 2.7724 * 72.0, 1.6389 * 72.0), "{pts:?}");
        });
    }

    // Issue #195: `splines=curved` goes through makeStraightEdges' curved branch: C's a--c
    // starts at 0.27902,0.4932in.
    #[test]
    fn nop_routes_curved_edges() {
        let g = nop_graph(
            false,
            &[("splines", "curved")],
            &[
                ("a", &[("pos", "0,0")]),
                ("b", &[("pos", "50,0")]),
                ("c", &[("pos", "0,100")]),
            ],
            &[(0, 2, &[]), (1, 2, &[])],
        );
        let g = nop_layout(g, 1);
        let pts = &g.edges[0].spline.segments[0].points;
        assert!(near(pts[0], 0.27902 * 72.0, 0.4932 * 72.0), "{pts:?}");
        assert!(near(pts[3], 0.27738 * 72.0, 1.3914 * 72.0), "{pts:?}");
    }

    /// `digraph { subgraph s0 { x0 subgraph s1 { x1 … subgraph s{n-1} { x{n-1} } } } }` with
    /// an edge `x{i} -> x{i+1}` in each `s{i}`, its lists closed as the parser makes them.
    fn nested(n: usize) -> Graph {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let xs: Vec<NodeId> = (0..n).map(|i| g.ensure_node(&format!("x{i}"))).collect();
        let es: Vec<EdgeId> = (1..n)
            .map(|i| g.add_edge(xs[i - 1], xs[i], AttrMap::new()))
            .collect();
        for i in 0..n {
            g.subgraphs.push(SubgraphData {
                id: SubgraphId(i as u32),
                name: format!("s{i}"),
                is_cluster: false,
                graph_attrs: AttrMap::new(),
                node_defaults: AttrMap::new(),
                edge_defaults: AttrMap::new(),
                nodes: xs[i..].to_vec(),
                edges: es[i..].to_vec(),
                subgraphs: (i + 1 < n)
                    .then(|| SubgraphId(i as u32 + 1))
                    .into_iter()
                    .collect(),
                bb: Default::default(),
                html_name: false,
                empty_name: false,
                layout: Default::default(),
            });
        }
        g
    }

    /// Issue #323: a subgraph's members are its own list, sorted; collecting every nested
    /// subgraph's list again made each cluster's vector (and its kept capacity) grow with
    /// the square of the depth, and fdp's memory with its cube.
    #[test]
    fn subgraph_members_are_the_subgraphs_own_lists() {
        let n = 300;
        let g = nested(n);
        let (mut nodes, mut edges) = (0, 0);
        for k in 0..n {
            let v = subgraph_nodes(&g, k);
            assert_eq!(v, (k..n).collect::<Vec<_>>());
            nodes += v.capacity();
            let v = subgraph_edges(&g, k);
            assert_eq!(v, (k..n - 1).collect::<Vec<_>>());
            edges += v.capacity();
        }
        assert_eq!(nodes, n * (n + 1) / 2);
        assert_eq!(edges, n * (n - 1) / 2);
    }
}
