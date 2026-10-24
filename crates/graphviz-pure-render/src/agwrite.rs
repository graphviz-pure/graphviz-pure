//! `agwrite` (`lib/cgraph/write.c`) over the parsed cgraph dictionaries kept in
//! [`Graph::dicts`]: the writer behind `-Tdot`, `-Txdot` and `-Tcanon`.

use graphviz_pure_core::{AttrMap, AttrSym, Graph, GraphKind};
use graphviz_pure_parser::canon_str_with;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Graph,
    Node,
    Edge,
}

struct W<'g> {
    g: &'g Graph,
    out: String,
    level: usize,
    maxline: usize,
    /// Parent dictionary index of every dictionary (index 0: root).
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    pre: Vec<u64>,
    node_written: Vec<u64>,
    edge_written: Vec<u64>,
    node_attrs_written: Vec<bool>,
    edge_attrs_written: Vec<bool>,
    /// Nodes (sorted) and edges of each graph.
    members_n: Vec<Vec<usize>>,
    members_e: Vec<BTreeSet<usize>>,
    /// Out-edges of each node in `agfstout` order.
    outs: Vec<Vec<usize>>,
    edge_attrs: Vec<AttrMap>,
}

fn empty_sym() -> AttrSym {
    AttrSym::default()
}

impl W<'_> {
    /// `agstrcanon`: an HTML string (`html`, `s` its inner text) is written `<s>`.
    fn canon(&self, s: &str, html: bool) -> String {
        if html {
            format!("<{s}>")
        } else {
            canon_str_with(s, self.maxline)
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.level {
            self.out.push('\t');
        }
    }

    fn dict(&self, gi: usize, k: Kind) -> &BTreeMap<String, AttrSym> {
        let d = &self.g.dicts[gi];
        match k {
            Kind::Graph => &d.graph,
            Kind::Node => &d.node,
            Kind::Edge => &d.edge,
        }
    }

    /// Search through the view path (`dtsearch` on a viewed dictionary).
    fn view_lookup(&self, gi: usize, k: Kind, name: &str) -> Option<&AttrSym> {
        let mut cur = Some(gi);
        while let Some(i) = cur {
            if let Some(s) = self.dict(i, k).get(name) {
                return Some(s);
            }
            cur = self.parent[i];
        }
        None
    }

    /// All symbols visible from `gi` with their nearest definition.
    fn view_all(&self, gi: usize, k: Kind) -> BTreeMap<&str, &AttrSym> {
        let mut out: BTreeMap<&str, &AttrSym> = BTreeMap::new();
        let mut cur = Some(gi);
        while let Some(i) = cur {
            for (n, s) in self.dict(i, k) {
                out.entry(n.as_str()).or_insert(s);
            }
            cur = self.parent[i];
        }
        out
    }

    fn write_dict(&mut self, gi: usize, name: &str, k: Kind, top: bool) {
        let mut entries: Vec<(String, bool, String, bool)> = Vec::new();
        for (n, sym) in self.dict(gi, k) {
            if sym.defval.is_empty() && !sym.print {
                if top {
                    continue;
                }
                let p = self.parent[gi].and_then(|p| self.view_lookup(p, k, n));
                let p = p.cloned().unwrap_or_else(empty_sym);
                if p.defval.is_empty() && p.print {
                    continue;
                }
            }
            entries.push((n.clone(), sym.html, sym.defval.clone(), sym.html_defval));
        }
        if entries.is_empty() {
            return;
        }
        for (i, (n, html, v, html_v)) in entries.iter().enumerate() {
            if i == 0 {
                self.indent();
                self.out.push_str(name);
                self.out.push_str(" [");
                self.level += 1;
            } else {
                self.out.push_str(",\n");
                self.indent();
            }
            let (cn, cv) = (self.canon(n, *html), self.canon(v, *html_v));
            self.out.push_str(&cn);
            self.out.push('=');
            self.out.push_str(&cv);
        }
        self.level -= 1;
        if entries.len() > 1 {
            self.out.push('\n');
            self.indent();
        }
        self.out.push_str("];\n");
    }

    fn graph_name(&self, gi: usize) -> Option<&str> {
        if gi == 0 {
            (!self.g.is_anonymous()).then_some(self.g.name.as_str())
        } else {
            let s = &self.g.subgraphs[gi - 1];
            (!s.is_anonymous()).then_some(s.name.as_str())
        }
    }

    fn graph_name_html(&self, gi: usize) -> bool {
        gi > 0 && self.g.subgraphs[gi - 1].html_name
    }

    fn write_hdr(&mut self, gi: usize) {
        let top = gi == 0;
        let name = self.graph_name(gi).map(str::to_string);
        self.indent();
        if top {
            if self.g.strict {
                self.out.push_str("strict ");
            }
            if self.g.kind == GraphKind::Directed {
                self.out.push_str("di");
            }
            self.out.push_str("graph ");
        } else if name.is_some() {
            self.out.push_str("subgraph ");
        }
        if let Some(n) = &name {
            let c = self.canon(n, self.graph_name_html(gi));
            self.out.push_str(&c);
            self.out.push(' ');
        }
        self.out.push_str("{\n");
        self.level += 1;
        self.write_dict(gi, "graph", Kind::Graph, top);
        self.write_dict(gi, "node", Kind::Node, top);
        self.write_dict(gi, "edge", Kind::Edge, top);
    }

    fn values(&self, gi: usize) -> &AttrMap {
        if gi == 0 {
            &self.g.graph_attrs
        } else {
            &self.g.subgraphs[gi - 1].graph_attrs
        }
    }

    fn irrelevant(&self, gi: usize) -> bool {
        if self.graph_name(gi).is_some() {
            return false;
        }
        let p = self.parent[gi].unwrap_or(0);
        let (sv, pv) = (self.values(gi), self.values(p));
        for name in self.g.dicts[0].graph.keys() {
            let a = sv.get(name).map_or("", String::as_str);
            let b = pv.get(name).map_or("", String::as_str);
            if a != b {
                return false;
            }
        }
        let d = &self.g.dicts[gi];
        d.node.is_empty() && d.edge.is_empty()
    }

    fn write_subgs(&mut self, gi: usize) {
        for c in self.children[gi].clone() {
            if self.irrelevant(c) {
                self.write_subgs(c);
            } else {
                self.write_hdr(c);
                self.write_body(c);
                self.level -= 1;
                self.indent();
                self.out.push_str("}\n");
            }
        }
    }

    fn value<'a>(attrs: &'a AttrMap, k: &str) -> &'a str {
        attrs.get(k).map_or("", String::as_str)
    }

    /// `write_edge_name`: an edge's key, `\t[key=<key>`, which `agnameof` gives unless the
    /// edge has none or a local `%` one. The bracket is left open when `terminate` is false.
    fn write_edge_name(&mut self, e: usize, terminate: bool) -> bool {
        let ed = &self.g.edges[e];
        let Some(key) = ed
            .key
            .as_deref()
            .filter(|k| !k.is_empty() && !k.starts_with('%'))
        else {
            return false;
        };
        let key = self.canon(key, ed.key_html);
        if !terminate {
            self.level += 1;
        }
        self.out.push_str("\t[key=");
        self.out.push_str(&key);
        if terminate {
            self.out.push(']');
        }
        true
    }

    fn write_nondefault(&mut self, gi: usize, attrs: &AttrMap, k: Kind, edge: Option<usize>) {
        let mut cnt = 0;
        if let Some(e) = edge
            && self.write_edge_name(e, false)
        {
            cnt += 1;
        }
        let view: Vec<(String, bool, String, bool)> = self
            .view_all(gi, k)
            .into_iter()
            .map(|(n, s)| (n.to_string(), s.html, s.defval.clone(), s.html_defval))
            .collect();
        for (n, html, defval, html_defval) in view {
            if k == Kind::Edge && (n == "tailport" || n == "headport") {
                continue;
            }
            let v = Self::value(attrs, &n);
            let html_v = attrs.is_html(&n);
            // `data->str[sym->id] != sym->defval` compares interned strings, flag included.
            if (v, html_v) != (defval.as_str(), html_defval) {
                if cnt == 0 {
                    self.out.push_str("\t[");
                    self.level += 1;
                } else {
                    self.out.push_str(",\n");
                    self.indent();
                }
                cnt += 1;
                let (cn, cv) = (self.canon(&n, html), self.canon(v, html_v));
                self.out.push_str(&cn);
                self.out.push('=');
                self.out.push_str(&cv);
            }
        }
        if cnt > 0 {
            self.out.push(']');
            self.level -= 1;
        }
    }

    fn not_default_attrs(&self, n: usize) -> bool {
        let attrs = &self.g.nodes[n].attrs;
        self.g.dicts[0].node.iter().any(|(k, s)| {
            (Self::value(attrs, k), attrs.is_html(k)) != (s.defval.as_str(), s.html_defval)
        })
    }

    fn has_no_edges(&self, gi: usize, n: usize) -> bool {
        !self.members_e[gi].iter().any(|&e| {
            let ed = &self.g.edges[e];
            ed.tail.0 as usize == n || ed.head.0 as usize == n
        })
    }

    fn node_test(&self, gi: usize, n: usize) -> bool {
        if self.node_written[n] >= self.pre[gi] {
            return false;
        }
        self.has_no_edges(gi, n) || self.not_default_attrs(n)
    }

    fn write_node(&mut self, gi: usize, n: usize) {
        self.indent();
        let nd = &self.g.nodes[n];
        let name = self.canon(&nd.name, nd.html_name);
        self.out.push_str(&name);
        if !self.node_attrs_written[n] {
            let attrs = self.g.nodes[n].attrs.clone();
            self.write_nondefault(gi, &attrs, Kind::Node, None);
            self.node_attrs_written[n] = true;
        }
        self.node_written[n] = self.pre[gi];
        self.out.push_str(";\n");
    }

    fn write_port(&mut self, e: usize, key: &str) {
        let v = Self::value(&self.edge_attrs[e], key).to_string();
        if v.is_empty() {
            return;
        }
        self.out.push(':');
        if self.edge_attrs[e].is_html(key) {
            let c = self.canon(&v, true);
            self.out.push_str(&c);
        } else if let Some((a, b)) = v.split_once(':') {
            let (a, b) = (
                canon_str_with(a, self.maxline),
                canon_str_with(b, self.maxline),
            );
            self.out.push_str(&a);
            self.out.push(':');
            self.out.push_str(&b);
        } else {
            let c = canon_str_with(&v, self.maxline);
            self.out.push_str(&c);
        }
    }

    fn write_edge(&mut self, gi: usize, e: usize) {
        let ed = &self.g.edges[e];
        let (t, h) = (ed.tail.0 as usize, ed.head.0 as usize);
        self.indent();
        let tn = self.canon(&self.g.nodes[t].name, self.g.nodes[t].html_name);
        self.out.push_str(&tn);
        self.write_port(e, "tailport");
        self.out.push_str(if self.g.kind == GraphKind::Directed {
            " -> "
        } else {
            " -- "
        });
        let hn = self.canon(&self.g.nodes[h].name, self.g.nodes[h].html_name);
        self.out.push_str(&hn);
        self.write_port(e, "headport");
        if !self.edge_attrs_written[e] {
            let attrs = self.edge_attrs[e].clone();
            self.write_nondefault(gi, &attrs, Kind::Edge, Some(e));
            self.edge_attrs_written[e] = true;
        } else {
            self.write_edge_name(e, true);
        }
        self.edge_written[e] = self.pre[gi];
        self.out.push_str(";\n");
    }

    fn write_body(&mut self, gi: usize) {
        self.write_subgs(gi);
        for n in self.members_n[gi].clone() {
            if self.node_test(gi, n) {
                self.write_node(gi, n);
            }
            let mut prev = n;
            let outs: Vec<usize> = self.outs[n]
                .iter()
                .copied()
                .filter(|e| self.members_e[gi].contains(e))
                .collect();
            for e in outs {
                let h = self.g.edges[e].head.0 as usize;
                if prev != h && self.node_test(gi, h) {
                    self.write_node(gi, h);
                    prev = h;
                }
                if self.edge_written[e] < self.pre[gi] {
                    self.write_edge(gi, e);
                }
            }
        }
    }
}

/// Edge attributes with `tailport`/`headport` from `a:p -> b` endpoints.
pub(crate) fn edge_attrs_with_ports(g: &Graph) -> Vec<AttrMap> {
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

/// `agwrite`.
pub(crate) fn agwrite(g: &Graph) -> String {
    let ns = g.subgraphs.len();
    let mut parent = vec![None; ns + 1];
    let mut children = vec![Vec::new(); ns + 1];
    let mut is_child = vec![false; ns];
    for (i, s) in g.subgraphs.iter().enumerate() {
        for c in &s.subgraphs {
            parent[c.0 as usize + 1] = Some(i + 1);
            is_child[c.0 as usize] = true;
            children[i + 1].push(c.0 as usize + 1);
        }
    }
    for i in 0..ns {
        if !is_child[i] {
            parent[i + 1] = Some(0);
            children[0].push(i + 1);
        }
    }
    // subgdfs
    let mut pre = vec![0u64; ns + 1];
    fn dfs(gi: usize, ix: u64, pre: &mut [u64], children: &[Vec<usize>]) -> u64 {
        pre[gi] = ix;
        let mut ix0 = ix;
        for &c in &children[gi] {
            ix0 = dfs(c, ix0, pre, children);
        }
        ix0 + 1
    }
    dfs(0, 1, &mut pre, &children);
    let mut members_n = vec![(0..g.nodes.len()).collect::<Vec<_>>()];
    let mut members_e = vec![(0..g.edges.len()).collect::<BTreeSet<_>>()];
    for s in &g.subgraphs {
        let mut n: Vec<usize> = s.nodes.iter().map(|x| x.0 as usize).collect();
        n.sort_unstable();
        n.dedup();
        members_n.push(n);
        members_e.push(s.edges.iter().map(|x| x.0 as usize).collect());
    }
    let mut out = vec![Vec::new(); g.nodes.len()];
    for (i, e) in g.edges.iter().enumerate() {
        out[e.tail.0 as usize].push(i);
    }
    for l in &mut out {
        l.sort_by_key(|&e| (g.edges[e].head.0, e));
    }
    let maxline = g
        .graph_attrs
        .get("linelength")
        .filter(|s| s.bytes().next().is_some_and(|c| c.is_ascii_digit()))
        .and_then(|s| {
            let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<usize>().ok()
        })
        .filter(|&l| l == 0 || l >= 60)
        .unwrap_or(128);
    let mut w = W {
        g,
        out: String::new(),
        level: 0,
        maxline,
        parent,
        children,
        pre,
        node_written: vec![0; g.nodes.len()],
        edge_written: vec![0; g.edges.len()],
        node_attrs_written: vec![false; g.nodes.len()],
        edge_attrs_written: vec![false; g.edges.len()],
        members_n,
        members_e,
        outs: out,
        edge_attrs: edge_attrs_with_ports(g),
    };
    w.write_hdr(0);
    w.write_body(0);
    w.level -= 1;
    w.out.push_str("}\n");
    w.out
}
