//! Layout attributes written back to the graph: port of `attach_attrs_and_arrows`,
//! `set_record_rects` and `rec_attach_bb` in `lib/common/output.c`, and `write_plain`.

use super::model::Model;
use super::num::fmt_g_into;
use super::{y_dir, y_offset};
use graphviz_pure_core::{Graph, Point, math};
use graphviz_pure_geom::shapes::{RecordField, ShapeKind};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write;

/// `"%.5g"` of `v`, written into `out`.
fn g5_into(out: &mut String, v: f64) {
    fmt_g_into(out, v, 5);
}

/// `"%.5g,%.5g"` of `p.x, yDir(p.y, y_off)`, written into `out`.
fn pt_into(out: &mut String, p: Point, y_off: f64) {
    g5_into(out, p.x);
    out.push(',');
    g5_into(out, y_dir(p.y, y_off));
}

/// One `"%.5g"` as a `String`: an attribute value is a `String` of its own.
fn one_g5(v: f64) -> String {
    let mut s = String::new();
    g5_into(&mut s, v);
    s
}

/// One `"%.5g,%.5g"` as a `String`: an attribute value is a `String` of its own.
fn one_pt(p: Point, y_off: f64) -> String {
    let mut s = String::new();
    pt_into(&mut s, p, y_off);
    s
}

fn record_rects(f: &RecordField, c: Point, y_off: f64, out: &mut String) {
    if f.children.is_empty() {
        g5_into(out, f.bbox.ll.x + c.x);
        out.push(',');
        g5_into(out, y_dir(f.bbox.ll.y + c.y, y_off));
        out.push(',');
        g5_into(out, f.bbox.ur.x + c.x);
        out.push(',');
        g5_into(out, y_dir(f.bbox.ur.y + c.y, y_off));
        out.push(' ');
    }
    for sub in &f.children {
        record_rects(sub, c, y_off, out);
    }
}

/// The `vertices` of a polygon node (`attach_attrs_and_arrows`, `N_vertices && isPolygon(n)`):
/// its polygon's first ring in inches, or for an ellipse (`sides < 3`) `samplepoints` points
/// on it.
fn vertices(n: &super::model::RNode<'_>, y_off: f64) -> Option<String> {
    let poly = n.geom.polygon.as_ref()?;
    // isPolygon: the shape's init function is poly_init (point_init makes points).
    if n.geom.shape == ShapeKind::Point {
        return None;
    }
    let yf = |y: f64| y_dir(y, y_off / 72.0);
    let mut sides = poly.sides;
    if sides < 3 {
        sides = n.attrs.get("samplepoints").map_or(8, |p| {
            usize::try_from(graphviz_pure_core::cnum::strtoul(p, 0).0).unwrap_or(usize::MAX)
        });
        if sides < 3 {
            sides = 8;
        }
    }
    let mut s = String::new();
    for i in 0..sides {
        if i > 0 {
            s.push(' ');
        }
        let (x, y) = if poly.sides >= 3 {
            let v = poly.vertices.get(i).copied().unwrap_or_default();
            (v.x / 72.0, v.y / 72.0)
        } else {
            let a = i as f64 / sides as f64 * std::f64::consts::PI * 2.0;
            (
                n.geom.width / 2.0 * math::cos(a),
                n.geom.height / 2.0 * math::sin(a),
            )
        };
        g5_into(&mut s, x);
        s.push(' ');
        g5_into(&mut s, yf(y));
    }
    Some(s)
}

/// `attach_attrs_and_arrows`: returns a copy of `cgraph` (the model's graph with what earlier
/// jobs set on it) with `pos`, `width`, `height`, `rects`, `vertices`, `bb`, `lp`, `lwidth`,
/// `lheight`, `xlp`, `head_lp`, `tail_lp` set, and whether any edge has start or end arrows.
pub(crate) fn attach_attrs(
    m: &Model<'_>,
    cgraph: &Graph,
) -> (Graph, bool, bool, Vec<BTreeSet<String>>) {
    let mut g = cgraph.clone();
    let (mut s_arrows, mut e_arrows) = (false, false);
    let y_off = y_offset(m.bb);
    let pt = |out: &mut String, p: Point| pt_into(out, p, y_off);
    // `N_vertices`: the input declares `vertices` for nodes.
    let n_vertices = m.g.node_attr_declared("vertices");
    // dim3 = GD_odim(g) >= 3
    let dim3 = m.g.layout.odim >= 3;
    for (i, n) in m.nodes.iter().enumerate() {
        let layout = &m.g.nodes[i].layout;
        if layout.clust_node {
            continue;
        }
        let a = &mut g.nodes[i].attrs;
        let mut pos = String::new();
        pt(&mut pos, n.coord);
        if dim3 {
            // POINTS_PER_INCH * ND_pos(n)[k] for k in 2..GD_odim(g)
            for k in 2..m.g.layout.odim {
                let z = layout.higher_dims.get(k - 2).copied().unwrap_or(0.0);
                pos.push(',');
                g5_into(&mut pos, z);
            }
        }
        a.set("pos", pos, false);
        a.set("height", one_g5(n.ht / 72.0), false);
        a.set("width", one_g5((n.lw + n.rw) / 72.0), false);
        if let Some(xl) = n.xlabel.as_ref().filter(|l| l.set) {
            a.set("xlp", one_pt(xl.pos, y_off), false);
        }
        if n.geom.shape == ShapeKind::Record
            && let Some(f) = &n.geom.record
        {
            let mut s = String::new();
            record_rects(f, n.coord, y_off, &mut s);
            s.pop();
            a.set("rects", s, false);
        } else if n_vertices && let Some(v) = vertices(n, y_off) {
            a.set("vertices", v, false);
        }
    }
    for (i, e) in m.edges.iter().enumerate() {
        if e.bz.is_empty() {
            continue;
        }
        // `if (ED_edge_type(e) == IGNORED) continue;`, and no edge below `GVSPLINES`.
        if m.g.edges[i].layout.no_attrs {
            continue;
        }
        let mut s = String::new();
        for (k, b) in e.bz.iter().enumerate() {
            if k > 0 {
                s.push(';');
            }
            if b.sflag != 0 {
                s_arrows = true;
                s.push_str("s,");
                pt(&mut s, b.sp);
                s.push(' ');
            }
            if b.eflag != 0 {
                e_arrows = true;
                s.push_str("e,");
                pt(&mut s, b.ep);
                s.push(' ');
            }
            for (j, p) in b.list.iter().enumerate() {
                if j > 0 {
                    s.push(' ');
                }
                pt(&mut s, *p);
            }
        }
        let a = &mut g.edges[i].attrs;
        a.set("pos", s, false);
        if let Some(l) = &e.label {
            a.set("lp", one_pt(l.pos, y_off), false);
        }
        if let Some(l) = e.xlabel.as_ref().filter(|l| l.set) {
            a.set("xlp", one_pt(l.pos, y_off), false);
        }
        if let Some(l) = &e.head_label {
            a.set("head_lp", one_pt(l.pos, y_off), false);
        }
        if let Some(l) = &e.tail_label {
            a.set("tail_lp", one_pt(l.pos, y_off), false);
        }
    }
    // rec_attach_bb
    let bbs = |b: graphviz_pure_core::BoundingBox| {
        let mut s = String::new();
        g5_into(&mut s, b.ll.x);
        s.push(',');
        g5_into(&mut s, y_dir(b.ll.y, y_off));
        s.push(',');
        g5_into(&mut s, b.ur.x);
        s.push(',');
        g5_into(&mut s, y_dir(b.ur.y, y_off));
        s
    };
    g.graph_attrs.set("bb", bbs(m.bb), false);
    if let Some(l) = m.label.as_ref().filter(|l| !l.text.is_empty()) {
        g.graph_attrs.set("lp", one_pt(l.pos, y_off), false);
        g.graph_attrs.set(
            "lwidth",
            graphviz_pure_core::cnum::fmt_f(l.dimen.x / 72.0, 2),
            false,
        );
        g.graph_attrs.set(
            "lheight",
            graphviz_pure_core::cnum::fmt_f(l.dimen.y / 72.0, 2),
            false,
        );
    }
    let mut explicit: Vec<BTreeSet<String>> = vec![BTreeSet::new(); g.subgraphs.len()];
    for c in &m.clusters {
        let a = &mut g.subgraphs[c.sub].graph_attrs;
        a.set("bb", bbs(c.bb), false);
        explicit[c.sub].insert("bb".into());
        if let Some(l) = c.label.as_ref().filter(|l| !l.text.is_empty()) {
            a.set("lp", one_pt(l.pos, y_off), false);
            a.set(
                "lwidth",
                graphviz_pure_core::cnum::fmt_f(l.dimen.x / 72.0, 2),
                false,
            );
            a.set(
                "lheight",
                graphviz_pure_core::cnum::fmt_f(l.dimen.y / 72.0, 2),
                false,
            );
            for k in ["lp", "lwidth", "lheight"] {
                explicit[c.sub].insert(k.into());
            }
        }
    }
    (g, s_arrows, e_arrows, explicit)
}

/// For `agwrite`: subgraphs that only inherited `bb`, `lp`, `lwidth` or `lheight` follow
/// their parent's new value (cgraph keeps graph attribute values in per-graph dictionaries,
/// so a value set on the root is not a local value of the subgraph).
pub(crate) fn propagate_inherited(g: &mut Graph, orig: &Graph, explicit: &[BTreeSet<String>]) {
    // Subgraphs that only inherited these attributes keep following their parent: cgraph
    // stores graph attribute values in per-graph dictionaries, so a changed root value is
    // not a local value of the subgraph.
    let mut is_child = vec![false; g.subgraphs.len()];
    for sg in &orig.subgraphs {
        for c in &sg.subgraphs {
            is_child[c.0 as usize] = true;
        }
    }
    fn walk(
        g: &mut Graph,
        orig: &Graph,
        s: usize,
        parent: Option<usize>,
        explicit: &[BTreeSet<String>],
    ) {
        for k in ["bb", "lp", "lwidth", "lheight"] {
            if explicit[s].contains(k) {
                continue;
            }
            let porig = match parent {
                None => orig.graph_attrs.get(k),
                Some(p) => orig.subgraphs[p].graph_attrs.get(k),
            };
            if orig.subgraphs[s].graph_attrs.get(k) == porig {
                let pattrs = match parent {
                    None => &g.graph_attrs,
                    Some(p) => &g.subgraphs[p].graph_attrs,
                };
                let pfinal = pattrs.get(k).cloned();
                let html = pattrs.is_html(k);
                match pfinal {
                    Some(v) => {
                        g.subgraphs[s].graph_attrs.set(k, v, html);
                    }
                    None => {
                        g.subgraphs[s].graph_attrs.remove(k);
                    }
                }
            }
        }
        let kids: Vec<usize> = orig.subgraphs[s]
            .subgraphs
            .iter()
            .map(|c| c.0 as usize)
            .collect();
        for c in kids {
            walk(g, orig, c, Some(s), explicit);
        }
    }
    for (s, child) in is_child.iter().enumerate() {
        if !child {
            walk(g, orig, s, None, explicit);
        }
    }
}

/// The plain strings (not HTML ones) the root graph's string dictionary holds: a name, an
/// attribute name or a plain attribute value. C answers this with one hash lookup
/// (C: lib/common/output.c:canon, lib/cgraph/refstr.c:agstrdup). Build the set once per
/// graph so label lookups take expected constant time. The keys borrow the graph strings;
/// the set is never iterated to write output.
fn plain_strings(g: &Graph) -> HashSet<&str> {
    fn add_map<'g>(set: &mut HashSet<&'g str>, m: &'g graphviz_pure_core::AttrMap) {
        for (k, v) in m.iter() {
            set.insert(k.as_str());
            if !m.is_html(k) {
                set.insert(v.as_str());
            }
        }
    }
    let mut set: HashSet<&str> = HashSet::new();
    if !g.empty_name {
        set.insert(g.name.as_str());
    }
    add_map(&mut set, &g.graph_attrs);
    add_map(&mut set, &g.node_defaults);
    add_map(&mut set, &g.edge_defaults);
    for sg in &g.subgraphs {
        if !sg.html_name {
            set.insert(sg.name.as_str());
        }
        add_map(&mut set, &sg.graph_attrs);
        add_map(&mut set, &sg.node_defaults);
        add_map(&mut set, &sg.edge_defaults);
    }
    // fdp's cluster nodes (`__0:cluster_x`) are gone again after undoClusterEdges
    for n in &g.nodes {
        if n.layout.clust_node {
            continue;
        }
        if !n.html_name {
            set.insert(n.name.as_str());
        }
        add_map(&mut set, &n.attrs);
    }
    for e in &g.edges {
        if !e.key_html
            && let Some(k) = e.key.as_deref()
        {
            set.insert(k);
        }
        add_map(&mut set, &e.attrs);
    }
    for d in &g.dicts {
        for m in [&d.graph, &d.node, &d.edge] {
            for (k, sym) in m {
                set.insert(k.as_str());
                set.insert(sym.defval.as_str());
            }
        }
    }
    set
}

/// `canon` in `output.c`, for the text of an edge label, of a node's non-HTML label, or of the
/// cluster an fdp cluster edge ends at. It interns the text in the root graph (`agstrdup`),
/// canonicalizes it, and frees it again (`agstrfree`): when `agstrcanon` needs no quotes it
/// returns the interned string itself, which that free releases unless the dictionary held the
/// plain string already. C then prints freed memory, a byte that differs
/// from run to run or nothing; the port writes the empty field. An HTML edge label whose text
/// is not also a name or a plain value takes that path, and so does the default label of a
/// node with an HTML name.
fn plain_label_canon(g: &HashSet<&str>, earlier: Option<&HashSet<&str>>, text: &str) -> String {
    let c = graphviz_pure_parser::canon_str(text);
    if c == text && !g.contains(text) && !earlier.is_some_and(|e| e.contains(text)) {
        return String::new();
    }
    c
}

/// `write_plain` (`-Tplain`, `-Tplain-ext`).
/// `earlier`: what the jobs before this one attached (`-Tdot -Tplain`), whose values are in
/// the root graph's dictionary too.
pub(crate) fn write_plain(
    m: &Model<'_>,
    zoom: f64,
    extend: bool,
    earlier: Option<&Graph>,
) -> String {
    use graphviz_pure_parser::canon_value;
    let g = m.g;
    // The plain output holds a line per node and per edge; the numbers are the bulk of it.
    let mut out = String::with_capacity(m.nodes.len() * 64 + m.edges.len() * 64 + 64);
    let y_off = y_offset(m.bb);
    let plain = plain_strings(g);
    let earlier = earlier.map(plain_strings);
    out.push_str("graph ");
    g5_into(&mut out, zoom);
    out.push(' ');
    g5_into(&mut out, m.bb.ur.x / 72.0);
    out.push(' ');
    g5_into(&mut out, m.bb.ur.y / 72.0);
    out.push('\n');
    for (i, n) in m.nodes.iter().enumerate() {
        let a = n.attrs;
        if g.nodes[i].layout.clust_node {
            continue;
        }
        let lbl = if n.label.is_html {
            canon_value(a.get("label").unwrap_or(""), true)
        } else {
            plain_label_canon(&plain, earlier.as_ref(), &n.label.text)
        };
        let fill = a
            .nonempty("fillcolor")
            .unwrap_or_else(|| a.late_nnstring("color", "lightgrey"));
        out.push_str("node ");
        out.push_str(&canon_value(n.name, g.nodes[i].html_name));
        out.push(' ');
        g5_into(&mut out, n.coord.x / 72.0);
        out.push(' ');
        g5_into(&mut out, y_dir(n.coord.y, y_off) / 72.0);
        out.push(' ');
        g5_into(&mut out, n.geom.width);
        out.push(' ');
        g5_into(&mut out, n.geom.height);
        out.push(' ');
        out.push_str(lbl.as_str());
        out.push(' ');
        out.push_str(a.late_nnstring("style", "solid"));
        out.push(' ');
        {
            let sh = a.late_nnstring("shape", "ellipse");
            if graphviz_pure_core::usershape::binds_custom(sh, a.nonempty("shapefile")) {
                out.push_str("custom");
            } else {
                out.push_str(sh);
            }
        }
        out.push(' ');
        out.push_str(a.late_nnstring("color", "black"));
        out.push(' ');
        out.push_str(fill);
        out.push('\n');
        let _ = i;
    }
    for n in 0..m.nodes.len() {
        for &ei in &m.out_edges[n] {
            let e = &m.edges[ei];
            let ge = &g.edges[ei];
            let (tport, hport) = if extend {
                (
                    e.attrs
                        .get("tailport")
                        .map(str::to_string)
                        .or_else(|| port_string(&ge.tail_port)),
                    e.attrs
                        .get("headport")
                        .map(str::to_string)
                        .or_else(|| port_string(&ge.head_port)),
                )
            } else {
                (None, None)
            };
            if !e.bz.is_empty() {
                out.push_str("edge");
                // `writenodeandport`: `agcanonStr` of the node name and the port.
                let ports = [
                    (e.tail, &tport, ge.attrs.is_html("tailport")),
                    (e.head, &hport, ge.attrs.is_html("headport")),
                ];
                for (node, port, html) in ports {
                    out.push(' ');
                    let gn = &g.nodes[node];
                    if gn.layout.clust_node {
                        // writenodeandport: `canon` of the cluster's name
                        out.push_str(&plain_label_canon(
                            &plain,
                            earlier.as_ref(),
                            m.nodes[node].name,
                        ));
                    } else {
                        out.push_str(&canon_value(m.nodes[node].name, gn.html_name));
                    }
                    if let Some(p) = port.as_deref().filter(|p| !p.is_empty()) {
                        out.push(':');
                        out.push_str(&canon_value(p, html));
                    }
                }
                let npts: usize = e.bz.iter().map(|b| b.list.len()).sum();
                let _ = write!(out, " {npts}");
                for b in &e.bz {
for p in b.list {
                        out.push(' ');
                        g5_into(&mut out, p.x / 72.0);
                        out.push(' ');
                        g5_into(&mut out, y_dir(p.y, y_off) / 72.0);
                    }
                }
            }
            if let Some(l) = &e.label {
                out.push(' ');
                out.push_str(&plain_label_canon(&plain, earlier.as_ref(), &l.text));
                out.push(' ');
                g5_into(&mut out, l.pos.x / 72.0);
                out.push(' ');
                g5_into(&mut out, y_dir(l.pos.y, y_off) / 72.0);
            }
            let _ = writeln!(
                out,
                " {} {}",
                e.attrs.late_nnstring("style", "solid"),
                e.attrs.late_nnstring("color", "black")
            );
        }
    }
    out.push_str("stop\n");
    out
}

fn port_string(p: &graphviz_pure_core::Port) -> Option<String> {
    match (&p.name, &p.compass) {
        (Some(n), Some(c)) => Some(format!("{n}:{c}")),
        (Some(n), None) => Some(n.clone()),
        (None, Some(c)) => Some(c.clone()),
        (None, None) => None,
    }
}
