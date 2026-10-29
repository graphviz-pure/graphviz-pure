//! `-Tjson` and `-Tjson0`: port of `plugin/core/gvrender_core_json.c`. `_draw_` attributes are
//! expanded with the xdot parser of `lib/xdot/xdot.c` ([`graphviz_pure_core::xdot`]).

use graphviz_pure_core::xdot::{Grad, Op, parse_xdot};
use graphviz_pure_core::{AttrMap, Graph, GraphKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

thread_local! {
    static LATIN1: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `stoj`: converts Latin-1 input (`charset=latin1`) to UTF-8 first.
fn stoj(s: &str, out: &mut String) {
    if LATIN1.with(|c| c.get()) {
        let conv = graphviz_pure_core::font::latin1_to_utf8(s);
        stoj_raw(&conv, out);
    } else {
        stoj_raw(s, out);
    }
}

fn stoj_raw(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn indent(out: &mut String, level: i32) {
    for _ in 0..level.max(0) {
        out.push_str("  ");
    }
}

fn is_xdot(name: &str) -> bool {
    matches!(
        name,
        "_draw_" | "_ldraw_" | "_hdraw_" | "_tdraw_" | "_hldraw_" | "_tldraw_"
    )
}

/// `%.03f`, as C's `printf` prints it (`nan` where Rust prints `NaN`).
struct F3(f64);

impl std::fmt::Display for F3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The number goes straight into the formatter's buffer, without a `String` of its own.
        graphviz_pure_core::cnum::fmt_f_into(f, self.0, 3)
    }
}

fn write_polyline(out: &mut String, pts: &[(f64, f64)]) {
    out.push_str("\"points\": [");
    for (i, (x, y)) in pts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "[{},{}]", F3(*x), F3(*y));
    }
    out.push_str("]\n");
}

fn write_stops(out: &mut String, stops: &[(f64, String)]) {
    out.push_str("\"stops\": [");
    for (i, (f, c)) in stops.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"frac\": {}, \"color\": ", F3(*f));
        stoj(c, out);
        out.push('}');
    }
    out.push_str("]\n");
}

fn write_op(out: &mut String, op: &Op, level: &mut i32) {
    indent(out, *level);
    *level += 1;
    out.push_str("{\n");
    indent(out, *level);
    let l = *level;
    match op {
        Op::Ellipse(f, r) => {
            let _ = writeln!(out, "\"op\": \"{}\",", if *f { 'E' } else { 'e' });
            indent(out, l);
            let _ = writeln!(
                out,
                "\"rect\": [{},{},{},{}]",
                F3(r[0]),
                F3(r[1]),
                F3(r[2]),
                F3(r[3])
            );
        }
        Op::Polygon(f, v) => {
            let _ = writeln!(out, "\"op\": \"{}\",", if *f { 'P' } else { 'p' });
            indent(out, l);
            write_polyline(out, v);
        }
        Op::Bezier(f, v) => {
            let _ = writeln!(out, "\"op\": \"{}\",", if *f { 'B' } else { 'b' });
            indent(out, l);
            write_polyline(out, v);
        }
        Op::Polyline(v) => {
            out.push_str("\"op\": \"L\",\n");
            indent(out, l);
            write_polyline(out, v);
        }
        Op::Text(x, y, a, w, t) => {
            out.push_str("\"op\": \"T\",\n");
            indent(out, l);
            let _ = writeln!(out, "\"pt\": [{},{}],", F3(*x), F3(*y));
            indent(out, l);
            let al = if *a < 0 {
                'l'
            } else if *a > 0 {
                'r'
            } else {
                'c'
            };
            let _ = writeln!(out, "\"align\": \"{al}\",");
            indent(out, l);
            let _ = writeln!(out, "\"width\": {},", F3(*w));
            indent(out, l);
            out.push_str("\"text\": ");
            stoj(t, out);
            out.push('\n');
        }
        Op::Color(fill, g) => {
            let _ = writeln!(out, "\"op\": \"{}\",", if *fill { 'C' } else { 'c' });
            indent(out, l);
            match g {
                Grad::None(c) => {
                    out.push_str("\"grad\": \"none\",\n");
                    indent(out, l);
                    out.push_str("\"color\": ");
                    stoj(c, out);
                    out.push('\n');
                }
                Grad::Linear(x0, y0, x1, y1, st) => {
                    out.push_str("\"grad\": \"linear\",\n");
                    indent(out, l);
                    indent(out, l);
                    let _ = writeln!(out, "\"p0\": [{},{}],", F3(*x0), F3(*y0));
                    indent(out, l);
                    let _ = writeln!(out, "\"p1\": [{},{}],", F3(*x1), F3(*y1));
                    indent(out, l);
                    write_stops(out, st);
                }
                Grad::Radial(x0, y0, r0, x1, y1, r1, st) => {
                    out.push_str("\"grad\": \"radial\",\n");
                    indent(out, l);
                    indent(out, l);
                    let _ = writeln!(out, "\"p0\": [{},{},{}],", F3(*x0), F3(*y0), F3(*r0));
                    indent(out, l);
                    let _ = writeln!(out, "\"p1\": [{},{},{}],", F3(*x1), F3(*y1), F3(*r1));
                    indent(out, l);
                    write_stops(out, st);
                }
            }
        }
        Op::Font(sz, f) => {
            out.push_str("\"op\": \"F\",\n");
            indent(out, l);
            let _ = writeln!(out, "\"size\": {},", F3(*sz));
            indent(out, l);
            out.push_str("\"face\": ");
            stoj(f, out);
            out.push('\n');
        }
        Op::Style(s) => {
            out.push_str("\"op\": \"S\",\n");
            indent(out, l);
            out.push_str("\"style\": ");
            stoj(s, out);
            out.push('\n');
        }
        Op::Image => {}
        Op::FontChar(v) => {
            out.push_str("\"op\": \"t\",\n");
            indent(out, l);
            // `%d` of the `unsigned` `parseUInt` read.
            let _ = writeln!(out, "\"fontchar\": {}", *v as u32 as i32);
        }
    }
    *level -= 1;
    indent(out, *level);
    out.push('}');
}

/// `write_xdots`: the value's ops as a JSON list. When no op parses C warns and writes
/// nothing after the key, which leaves the JSON invalid. That is what the CLI writes
/// (`c_output`); for a library caller, which gets one JSON document, the value is `""`.
fn write_xdots(out: &mut String, val: &str, level: &mut i32, c_output: bool) {
    let ops = parse_xdot(val);
    if ops.is_empty() {
        graphviz_pure_core::diag::agwarningf(&format!("Could not parse xdot \"{val}\"\n"));
        if !c_output {
            out.push_str("\"\"");
        }
        return;
    }
    out.push('\n');
    indent(out, *level);
    *level += 1;
    out.push_str("[\n");
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        write_op(out, op, level);
    }
    *level -= 1;
    out.push('\n');
    indent(out, *level);
    out.push(']');
}

// ---------------------------------------------------------------------------------------------
// graph writer
// ---------------------------------------------------------------------------------------------

struct W<'g> {
    g: &'g Graph,
    do_xdot: bool,
    /// Write C's bytes where they are not a valid JSON document ([`write_xdots`]).
    c_output: bool,
    latin1: bool,
    level: i32,
    out: String,
    graph_dict: Vec<String>,
    node_dict: Vec<String>,
    edge_dict: Vec<String>,
    edge_attrs: Vec<AttrMap>,
    sub_gid: Vec<usize>,
    children: Vec<Vec<usize>>,
    top: Vec<usize>,
    node_gid: Vec<i64>,
    clust_nodes: &'g [bool],
    edge_gid: Vec<usize>,
}

impl W<'_> {
    fn write_attrs(&mut self, attrs: &AttrMap, dict_kind: u8) {
        let dict = match dict_kind {
            0 => &self.graph_dict,
            1 => &self.node_dict,
            _ => &self.edge_dict,
        };
        let out = &mut self.out;
        for name in dict {
            let v = attrs.get(name).map(String::as_str).unwrap_or("");
            if v.is_empty() && name != "label" {
                continue;
            }
            out.push_str(",\n");
            indent(out, self.level);
            stoj(name, out);
            out.push_str(": ");
            if self.do_xdot && is_xdot(name) {
                let mut level = self.level;
                write_xdots(out, v, &mut level, self.c_output);
            } else {
                stoj(v, out);
            }
        }
    }

    fn write_graph(&mut self, sub: Option<usize>) {
        let g = self.g;
        let top = sub.is_none();
        indent(&mut self.out, self.level);
        self.level += 1;
        self.out.push_str("{\n");
        indent(&mut self.out, self.level);
        self.out.push_str("\"name\": ");
        let name = match sub {
            None => g.root_name().to_string(),
            Some(s) => g.subgraph_name(s),
        };
        stoj(&name, &mut self.out);
        if top {
            self.out.push_str(",\n");
            indent(&mut self.out, self.level);
            let _ = writeln!(self.out, "\"directed\": {},", g.kind == GraphKind::Directed);
            indent(&mut self.out, self.level);
            let _ = write!(self.out, "\"strict\": {}", g.strict);
        }
        let attrs = sub.map_or(&g.graph_attrs, |s| &g.subgraphs[s].graph_attrs);
        self.write_attrs(attrs, 0);
        self.out.push_str(",\n");
        indent(&mut self.out, self.level);
        if top {
            let _ = write!(self.out, "\"_subgraph_cnt\": {}", g.subgraphs.len());
        } else {
            let _ = write!(self.out, "\"_gvid\": {}", self.sub_gid[sub.unwrap()]);
        }
        // write_subgs
        let subs: Vec<usize> = match sub {
            None => self.top.clone(),
            Some(s) => self.children[s].clone(),
        };
        let has_subgs = !subs.is_empty();
        if has_subgs {
            self.out.push_str(",\n");
            indent(&mut self.out, self.level);
            self.level += 1;
            if top {
                self.out.push_str("\"objects\": [\n");
            } else {
                self.out.push_str("\"subgraphs\": [\n");
                indent(&mut self.out, self.level);
            }
            for (i, s) in subs.iter().enumerate() {
                if i > 0 {
                    self.out.push_str(",\n");
                }
                if top {
                    self.write_subg(*s);
                } else {
                    let _ = write!(self.out, "{}", self.sub_gid[*s]);
                }
            }
            if !top {
                self.level -= 1;
                self.out.push('\n');
                indent(&mut self.out, self.level);
                self.out.push(']');
            }
        }
        // write_nodes: cluster nodes are not written.
        let mut nodes: Vec<usize> = match sub {
            None => (0..g.nodes.len()).collect(),
            Some(s) => {
                let mut v: Vec<usize> = g.subgraphs[s].nodes.iter().map(|n| n.0 as usize).collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        };
        nodes.retain(|&n| !self.clust_nodes.get(n).copied().unwrap_or(false));
        if nodes.is_empty() {
            if has_subgs && top {
                self.level -= 1;
                self.out.push('\n');
                indent(&mut self.out, self.level);
                self.out.push(']');
            }
        } else {
            self.out.push_str(",\n");
            if top {
                if !has_subgs {
                    indent(&mut self.out, self.level);
                    self.level += 1;
                    self.out.push_str("\"objects\": [\n");
                }
            } else {
                indent(&mut self.out, self.level);
                self.level += 1;
                self.out.push_str("\"nodes\": [\n");
                indent(&mut self.out, self.level);
            }
            for (i, &n) in nodes.iter().enumerate() {
                if i > 0 {
                    self.out.push_str(if top { ",\n" } else { "," });
                }
                if top {
                    indent(&mut self.out, self.level);
                    self.level += 1;
                    self.out.push_str("{\n");
                    indent(&mut self.out, self.level);
                    let _ = writeln!(self.out, "\"_gvid\": {},", self.node_gid[n]);
                    indent(&mut self.out, self.level);
                    self.out.push_str("\"name\": ");
                    stoj(&g.nodes[n].name, &mut self.out);
                    self.write_attrs(&g.nodes[n].attrs, 1);
                    self.out.push('\n');
                    self.level -= 1;
                    indent(&mut self.out, self.level);
                    self.out.push('}');
                } else {
                    let _ = write!(self.out, "{}", self.node_gid[n]);
                }
            }
            self.level -= 1;
            self.out.push('\n');
            indent(&mut self.out, self.level);
            self.out.push(']');
        }
        // write_edges
        let mut edges: Vec<usize> = match sub {
            None => (0..g.edges.len()).collect(),
            Some(s) => g.subgraphs[s].edges.iter().map(|e| e.0 as usize).collect(),
        };
        edges.sort_unstable();
        edges.dedup();
        if !edges.is_empty() {
            self.out.push_str(",\n");
            indent(&mut self.out, self.level);
            self.level += 1;
            self.out.push_str("\"edges\": [\n");
            if !top {
                indent(&mut self.out, self.level);
            }
            for (i, &e) in edges.iter().enumerate() {
                if i > 0 {
                    self.out.push_str(if top { ",\n" } else { "," });
                }
                if top {
                    indent(&mut self.out, self.level);
                    self.level += 1;
                    self.out.push_str("{\n");
                    indent(&mut self.out, self.level);
                    let _ = writeln!(self.out, "\"_gvid\": {},", self.edge_gid[e]);
                    indent(&mut self.out, self.level);
                    let _ = writeln!(
                        self.out,
                        "\"tail\": {},",
                        self.node_gid[g.edges[e].tail.0 as usize]
                    );
                    indent(&mut self.out, self.level);
                    let _ = write!(
                        self.out,
                        "\"head\": {}",
                        self.node_gid[g.edges[e].head.0 as usize]
                    );
                    let a = self.edge_attrs[e].clone();
                    self.write_attrs(&a, 2);
                    self.out.push('\n');
                    self.level -= 1;
                    indent(&mut self.out, self.level);
                    self.out.push('}');
                } else {
                    let _ = write!(self.out, "{}", self.edge_gid[e]);
                }
            }
            self.level -= 1;
            self.out.push('\n');
            indent(&mut self.out, self.level);
            self.out.push(']');
        }
        self.out.push('\n');
        self.level -= 1;
        indent(&mut self.out, self.level);
        self.out.push_str(if top { "}\n" } else { "}" });
    }

    fn write_subg(&mut self, s: usize) {
        self.write_graph(Some(s));
        for c in self.children[s].clone() {
            self.out.push_str(",\n");
            self.write_subg(c);
        }
    }
}

/// Edge attributes including `tailport`/`headport` from `a:p -> b` syntax.
fn edge_attrs_with_ports(g: &Graph) -> Vec<AttrMap> {
    g.edges
        .iter()
        .map(|e| {
            let mut a = e.attrs.clone();
            let port = |p: &graphviz_pure_core::Port| -> Option<String> {
                match (&p.name, &p.compass) {
                    (Some(n), Some(c)) => Some(format!("{n}:{c}")),
                    (Some(n), None) => Some(n.clone()),
                    (None, Some(c)) => Some(c.clone()),
                    (None, None) => None,
                }
            };
            if let Some(t) = port(&e.tail_port).filter(|_| !a.contains_key("tailport")) {
                a.insert("tailport".into(), t);
            }
            if let Some(h) = port(&e.head_port).filter(|_| !a.contains_key("headport")) {
                a.insert("headport".into(), h);
            }
            a
        })
        .collect()
}

/// Writes `g` (with its layout attributes already attached) as JSON. `edge_order` gives the
/// `agfstnode`/`agfstout` iteration order used to number edges.
/// `clust_nodes`: the nodes that are `IS_CLUST_NODE` (fdp's cluster-edge endpoints).
/// `c_output`: the caller keeps C's output-file bytes (the CLI), see [`write_xdots`].
pub(crate) fn write_json(
    g: &Graph,
    do_xdot: bool,
    c_output: bool,
    edge_order: &[usize],
    clust_nodes: &[bool],
) -> String {
    let edge_attrs = edge_attrs_with_ports(g);
    // `write_attrs` walks the root's dictionaries (`agnxtattr(agroot(obj), …)`), so a
    // symbol is written even when no object has a value for it: `label` as `""`. A
    // statement that makes no object can still declare one (`{} -> c [label=x]`). The
    // caller has recorded the layout's and renderers' attributes there (`sync_dicts`). A
    // graph built without dictionaries falls back to its defaults and its objects' values.
    let (gd, nd, ed): (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) = match g.dicts.first()
    {
        Some(d) if g.dicts.len() == g.subgraphs.len() + 1 => (
            d.graph.keys().cloned().collect(),
            d.node.keys().cloned().collect(),
            d.edge.keys().cloned().collect(),
        ),
        _ => (
            g.graph_attrs
                .keys()
                .chain(g.subgraphs.iter().flat_map(|s| s.graph_attrs.keys()))
                .cloned()
                .collect(),
            g.node_defaults
                .keys()
                .chain(g.nodes.iter().flat_map(|n| n.attrs.keys()))
                .cloned()
                .collect(),
            g.edge_defaults
                .keys()
                .chain(edge_attrs.iter().flat_map(|a| a.keys()))
                .cloned()
                .collect(),
        ),
    };
    let mut is_child = vec![false; g.subgraphs.len()];
    for s in &g.subgraphs {
        for c in &s.subgraphs {
            is_child[c.0 as usize] = true;
        }
    }
    let top: Vec<usize> = (0..g.subgraphs.len()).filter(|&i| !is_child[i]).collect();
    let children: Vec<Vec<usize>> = g
        .subgraphs
        .iter()
        .map(|s| s.subgraphs.iter().map(|c| c.0 as usize).collect())
        .collect();
    // label_subgs: preorder numbering
    let mut sub_gid = vec![0usize; g.subgraphs.len()];
    let mut cnt = 0;
    fn label(
        s: usize,
        cnt: &mut usize,
        gid: &mut [usize],
        children: &[Vec<usize>],
        g: &Graph,
        cl: &mut BTreeMap<String, usize>,
    ) {
        gid[s] = *cnt;
        let name = g.subgraph_name(s);
        // `insert(map, agnameof(g), GD_gid(g))`: the first cluster of a name keeps it.
        if crate::emit::model::is_a_cluster(&name, &g.subgraphs[s].graph_attrs) {
            match cl.get(&name) {
                Some(&v) if v != *cnt => graphviz_pure_core::diag::agwarningf(&format!(
                    "Duplicate cluster name \"{name}\"\n"
                )),
                Some(_) => {}
                None => {
                    cl.insert(name, *cnt);
                }
            }
        }
        *cnt += 1;
        for &c in &children[s] {
            label(c, cnt, gid, children, g, cl);
        }
    }
    let mut cl: BTreeMap<String, usize> = BTreeMap::new();
    for &s in &top {
        label(s, &mut cnt, &mut sub_gid, &children, g, &mut cl);
    }
    // A cluster node (IS_CLUST_NODE) takes the id of the cluster it names, or -1.
    let mut ncnt = 0;
    let node_gid: Vec<i64> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if clust_nodes.get(i).copied().unwrap_or(false) {
                cl.get(&n.name).map_or(-1, |&v| v as i64)
            } else {
                ncnt += 1;
                (cnt + ncnt - 1) as i64
            }
        })
        .collect();
    let mut edge_gid = vec![0usize; g.edges.len()];
    for (k, &e) in edge_order.iter().enumerate() {
        edge_gid[e] = k;
    }
    let mut w = W {
        g,
        do_xdot,
        c_output,
        latin1: graphviz_pure_core::font::charset_is_latin1(
            g.graph_attrs.get("charset").map(String::as_str),
        ),
        level: 0,
        out: String::new(),
        graph_dict: gd.into_iter().collect(),
        node_dict: nd.into_iter().collect(),
        edge_dict: ed.into_iter().collect(),
        edge_attrs,
        sub_gid,
        children,
        top,
        node_gid,
        edge_gid,
        clust_nodes,
    };
    LATIN1.with(|c| c.set(w.latin1));
    w.write_graph(None);
    LATIN1.with(|c| c.set(false));
    w.out
}
