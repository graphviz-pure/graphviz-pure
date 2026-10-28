//! Geometric differential comparison against C Graphviz 16.1.0 (`gv-diff`).
//!
//! Golden files are C `dot -Tdot` output produced with only the core, dot and neato
//! plugins loaded (no pango), so C measures text with the same built-in font metric
//! tables that this port uses. See `scripts/gen-golden.sh`.
//!
//! Both sides are parsed from `-Tdot` text and reduced to a [`Layout`]: the graph
//! bounding box, node centres and sizes, edge splines (including arrow endpoints) and
//! label positions, and cluster bounding boxes. [`compare`] scores how many elements
//! match within fixed tolerances.

pub mod render;

use graphviz_pure_core::math;
use graphviz_pure_core::{Graph, GraphKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Node centre tolerance in points.
pub const TOL_POS: f64 = 1.0;
/// Node width/height tolerance in points.
pub const TOL_SIZE: f64 = 1.0;
/// Edge spline (sampled Hausdorff) tolerance in points.
pub const TOL_EDGE: f64 = 2.0;
/// Label position tolerance in points.
pub const TOL_LABEL: f64 = 2.0;
/// Bounding box corner tolerance in points.
pub const TOL_BB: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    fn dist(self, o: Pt) -> f64 {
        math::hypot(self.x - o.x, self.y - o.y)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeGeom {
    pub pos: Pt,
    /// Width and height in points.
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default)]
pub struct EdgeGeom {
    /// One entry per spline (multiple when the edge `pos` has `;`-separated parts).
    pub beziers: Vec<Vec<Pt>>,
    pub start_arrow: Option<Pt>,
    pub end_arrow: Option<Pt>,
    pub label: Option<Pt>,
    pub head_label: Option<Pt>,
    pub tail_label: Option<Pt>,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub bb: Option<[f64; 4]>,
    pub nodes: BTreeMap<String, NodeGeom>,
    /// Keyed by `tail -> head #k` where `k` counts repeated edges between the pair.
    pub edges: BTreeMap<String, EdgeGeom>,
    pub clusters: BTreeMap<String, [f64; 4]>,
    /// True when ranks run along x (`rankdir=LR|RL`).
    pub ranks_along_x: bool,
}

fn parse_pt(s: &str) -> Option<Pt> {
    let mut it = s.trim().split(',');
    let x = it.next()?.trim().parse().ok()?;
    let y = it.next()?.trim().parse().ok()?;
    Some(Pt { x, y })
}

fn parse_bb(s: &str) -> Option<[f64; 4]> {
    let v: Vec<f64> = s
        .split(',')
        .map(|t| t.trim().parse().ok())
        .collect::<Option<_>>()?;
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

fn parse_edge_pos(s: &str, e: &mut EdgeGeom) {
    for part in s.split(';') {
        let mut pts = Vec::new();
        for tok in part.split_whitespace() {
            if let Some(rest) = tok.strip_prefix("e,") {
                e.end_arrow = parse_pt(rest);
            } else if let Some(rest) = tok.strip_prefix("s,") {
                e.start_arrow = parse_pt(rest);
            } else if let Some(p) = parse_pt(tok) {
                pts.push(p);
            }
        }
        if !pts.is_empty() {
            e.beziers.push(pts);
        }
    }
}

/// Extracts the geometry of the first graph in a `-Tdot` document.
pub fn extract(dot_text: &str) -> Result<Layout, String> {
    let g = graphviz_pure_parser::parse_dot(dot_text).map_err(|e| e.to_string())?;
    Ok(extract_graph(&g))
}

pub fn extract_graph(g: &Graph) -> Layout {
    let mut out = Layout {
        bb: g.graph_attrs.get("bb").and_then(|s| parse_bb(s)),
        ranks_along_x: g
            .graph_attrs
            .get("rankdir")
            .is_some_and(|d| matches!(d.trim().to_ascii_uppercase().as_str(), "LR" | "RL")),
        ..Layout::default()
    };
    for n in &g.nodes {
        let Some(pos) = n.attrs.get("pos").and_then(|s| parse_pt(s)) else {
            continue;
        };
        let inch = |k: &str| {
            n.attrs
                .get(k)
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0)
                * 72.0
        };
        out.nodes.insert(
            n.name.clone(),
            NodeGeom {
                pos,
                width: inch("width"),
                height: inch("height"),
            },
        );
    }
    let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
    for e in &g.edges {
        let mut t = g.nodes[e.tail.0 as usize].name.clone();
        let mut h = g.nodes[e.head.0 as usize].name.clone();
        if g.kind == GraphKind::Undirected && t > h {
            std::mem::swap(&mut t, &mut h);
        }
        let k = seen.entry((t.clone(), h.clone())).or_insert(0);
        let key = format!("{t} -> {h} #{k}");
        *k += 1;
        let mut geom = EdgeGeom::default();
        if let Some(p) = e.attrs.get("pos") {
            parse_edge_pos(p, &mut geom);
        }
        geom.label = e.attrs.get("lp").and_then(|s| parse_pt(s));
        geom.head_label = e.attrs.get("head_lp").and_then(|s| parse_pt(s));
        geom.tail_label = e.attrs.get("tail_lp").and_then(|s| parse_pt(s));
        out.edges.insert(key, geom);
    }
    for sg in &g.subgraphs {
        // Only clusters carry their own `bb`; a non-cluster subgraph may show
        // the root's `bb` through attribute inheritance in the parser.
        let is_cluster = sg.is_cluster || sg.name.starts_with("cluster");
        if !is_cluster {
            continue;
        }
        if let Some(bb) = sg.graph_attrs.get("bb").and_then(|s| parse_bb(s)) {
            out.clusters.insert(sg.name.clone(), bb);
        }
    }
    out
}

fn sample_bezier(pts: &[Pt]) -> Vec<Pt> {
    const STEPS: usize = 12;
    if pts.len() < 4 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i + 3 < pts.len() {
        let (p0, p1, p2, p3) = (pts[i], pts[i + 1], pts[i + 2], pts[i + 3]);
        for s in 1..=STEPS {
            let t = s as f64 / STEPS as f64;
            let u = 1.0 - t;
            let b = |a: f64, b: f64, c: f64, d: f64| {
                u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
            };
            out.push(Pt {
                x: b(p0.x, p1.x, p2.x, p3.x),
                y: b(p0.y, p1.y, p2.y, p3.y),
            });
        }
        i += 3;
    }
    out
}

fn hausdorff(a: &[Pt], b: &[Pt]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() {
            0.0
        } else {
            f64::INFINITY
        };
    }
    let one = |p: &[Pt], q: &[Pt]| {
        p.iter()
            .map(|x| q.iter().map(|y| x.dist(*y)).fold(f64::INFINITY, f64::min))
            .fold(0.0, f64::max)
    };
    one(a, b).max(one(b, a))
}

fn opt_dist(a: Option<Pt>, b: Option<Pt>) -> f64 {
    match (a, b) {
        (None, None) => 0.0,
        (Some(a), Some(b)) => a.dist(b),
        _ => f64::INFINITY,
    }
}

fn bb_dist(a: [f64; 4], b: [f64; 4]) -> f64 {
    (0..4).map(|i| (a[i] - b[i]).abs()).fold(0.0, f64::max)
}

/// Per-file comparison result. Errors are in points; `*_ok` count elements within tolerance.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FileReport {
    pub nodes: usize,
    pub nodes_ok: usize,
    pub edges: usize,
    pub edges_ok: usize,
    pub clusters: usize,
    pub clusters_ok: usize,
    pub bb_ok: bool,
    /// Worst node centre error.
    pub node_err: f64,
    /// Worst edge spline error.
    pub edge_err: f64,
    /// Elements present on one side only.
    pub missing: usize,
    /// Nodes whose width and height match C, regardless of position.
    #[serde(default)]
    pub sizes_ok: usize,
    /// Nodes fall into the same ranks as in C (dot only; informational).
    #[serde(default)]
    pub rank_ok: bool,
    /// Within every rank, nodes appear in the same left-to-right order as in C.
    #[serde(default)]
    pub order_ok: bool,
    /// Set when the port failed or panicked on this input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FileReport {
    /// Weighted fraction of matching elements in `[0, 1]`.
    pub fn score(&self) -> f64 {
        if self.error.is_some() {
            return 0.0;
        }
        let frac = |ok: usize, n: usize| if n == 0 { 1.0 } else { ok as f64 / n as f64 };
        let s = 0.4 * frac(self.nodes_ok, self.nodes)
            + 0.4 * frac(self.edges_ok, self.edges)
            + 0.1 * frac(self.clusters_ok, self.clusters)
            + 0.1 * f64::from(u8::from(self.bb_ok));
        let total = self.nodes + self.edges + self.clusters;
        if total == 0 {
            s
        } else {
            s * (1.0 - self.missing as f64 / (total + self.missing) as f64)
        }
    }

    pub fn exact(&self) -> bool {
        self.error.is_none()
            && self.missing == 0
            && self.bb_ok
            && self.nodes_ok == self.nodes
            && self.edges_ok == self.edges
            && self.clusters_ok == self.clusters
    }
}

fn finite_or(v: f64, cap: f64) -> f64 {
    if v.is_finite() { v } else { cap }
}

/// Groups nodes into ranks by their rank-axis coordinate, each rank sorted along the
/// other axis. Coordinates within 0.5pt are treated as one rank.
fn ranks(l: &Layout, names: &[&String]) -> Vec<Vec<String>> {
    let along_x = l.ranks_along_x;
    let mut v: Vec<(f64, f64, &String)> = names
        .iter()
        .filter_map(|n| {
            let p = l.nodes.get(*n)?.pos;
            Some(if along_x {
                (p.x, p.y, *n)
            } else {
                (p.y, p.x, *n)
            })
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut last = f64::NAN;
    for (r, _, n) in v {
        if out.is_empty() || (r - last).abs() > 0.5 {
            out.push(Vec::new());
        }
        last = r;
        out.last_mut().unwrap().push(n.clone());
    }
    out
}

pub fn compare(golden: &Layout, actual: &Layout) -> FileReport {
    let mut r = FileReport {
        bb_ok: match (golden.bb, actual.bb) {
            (Some(a), Some(b)) => bb_dist(a, b) <= TOL_BB,
            (None, None) => true,
            _ => false,
        },
        ..FileReport::default()
    };
    for (name, g) in &golden.nodes {
        r.nodes += 1;
        let Some(a) = actual.nodes.get(name) else {
            r.missing += 1;
            continue;
        };
        let d = g.pos.dist(a.pos);
        r.node_err = r.node_err.max(d);
        let size_ok =
            (g.width - a.width).abs() <= TOL_SIZE && (g.height - a.height).abs() <= TOL_SIZE;
        r.sizes_ok += usize::from(size_ok);
        if d <= TOL_POS && size_ok {
            r.nodes_ok += 1;
        }
    }
    r.missing += actual
        .nodes
        .keys()
        .filter(|k| !golden.nodes.contains_key(*k))
        .count();
    for (key, g) in &golden.edges {
        r.edges += 1;
        let Some(a) = actual.edges.get(key) else {
            r.missing += 1;
            continue;
        };
        let gs: Vec<Pt> = g.beziers.iter().flat_map(|b| sample_bezier(b)).collect();
        let as_: Vec<Pt> = a.beziers.iter().flat_map(|b| sample_bezier(b)).collect();
        let d = hausdorff(&gs, &as_)
            .max(opt_dist(g.start_arrow, a.start_arrow))
            .max(opt_dist(g.end_arrow, a.end_arrow));
        r.edge_err = r.edge_err.max(finite_or(d, 1e6));
        let labels = opt_dist(g.label, a.label)
            .max(opt_dist(g.head_label, a.head_label))
            .max(opt_dist(g.tail_label, a.tail_label));
        if d <= TOL_EDGE && labels <= TOL_LABEL {
            r.edges_ok += 1;
        }
    }
    r.missing += actual
        .edges
        .keys()
        .filter(|k| !golden.edges.contains_key(*k))
        .count();
    let shared: Vec<&String> = golden
        .nodes
        .keys()
        .filter(|k| actual.nodes.contains_key(*k))
        .collect();
    let (gr, ar) = (ranks(golden, &shared), ranks(actual, &shared));
    let as_sets = |v: &Vec<Vec<String>>| {
        v.iter()
            .map(|r| {
                let mut r = r.clone();
                r.sort();
                r
            })
            .collect::<Vec<_>>()
    };
    r.rank_ok = as_sets(&gr) == as_sets(&ar);
    r.order_ok = r.rank_ok && gr == ar;
    for (name, g) in &golden.clusters {
        r.clusters += 1;
        match actual.clusters.get(name) {
            Some(a) if bb_dist(*g, *a) <= TOL_BB => r.clusters_ok += 1,
            Some(_) => {}
            None => r.missing += 1,
        }
    }
    r
}

/// Summary over a corpus run, stored as the regression baseline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Baseline {
    pub files: BTreeMap<String, FileReport>,
}

impl Baseline {
    pub fn total_score(&self) -> f64 {
        self.files.values().map(FileReport::score).sum()
    }
    pub fn exact_count(&self) -> usize {
        self.files.values().filter(|r| r.exact()).count()
    }
}

/// Layout engine named by a graph's `layout` attribute, else `dot` (C precedence).
pub fn engine_for(source: &str) -> String {
    graphviz_pure_parser::parse_dot(source)
        .ok()
        .and_then(|g| g.graph_attrs.get("layout").cloned())
        .unwrap_or_else(|| "dot".to_string())
}

/// Lays out `source` with the port and returns its geometry. Panics are reported as errors.
pub fn run_port(source: &str) -> Result<Layout, String> {
    let src = source.to_string();
    let res = std::panic::catch_unwind(move || {
        graphviz_pure_render::decoded::render_dot(
            &graphviz_pure_render::decoded::Decoded::from_decoded(src),
            "dot",
            "dot",
        )
    });
    match res {
        Ok(Ok(text)) => extract(&text),
        Ok(Err(e)) => Err(format!("error: {e}")),
        Err(p) => Err(format!(
            "panic: {}",
            p.downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| p.downcast_ref::<&str>().copied())
                .unwrap_or("?")
        )),
    }
}

pub fn compare_file(source: &str, golden_text: &str) -> FileReport {
    let golden = match extract(golden_text) {
        Ok(g) => g,
        Err(e) => {
            return FileReport {
                error: Some(format!("golden: {e}")),
                ..FileReport::default()
            };
        }
    };
    match run_port(source) {
        Ok(actual) => compare(&golden, &actual),
        Err(e) => FileReport {
            nodes: golden.nodes.len(),
            edges: golden.edges.len(),
            clusters: golden.clusters.len(),
            error: Some(e),
            ..FileReport::default()
        },
    }
}
