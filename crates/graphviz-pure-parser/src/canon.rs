//! Canonical DOT writer (`dot -Tcanon`) matching Graphviz `lib/cgraph/write.c`.

use graphviz_pure_core::{
    AttrMap, EdgeData, EdgeId, Graph, GraphKind, NodeData, NodeId, Port, SubgraphId,
};
use std::collections::BTreeSet;

/// Formats a [`Graph`] into canonical DOT representation (`dot -Tcanon`).
pub fn write_canon(graph: &Graph) -> String {
    write_canon_forced(graph, &[])
}

/// [`write_canon`] where `forced[i]` names graph attributes of subgraph `i` that are set on
/// the subgraph itself (a local dictionary entry in cgraph) and so are written even when
/// they equal the parent's value.
pub fn write_canon_forced(graph: &Graph, forced: &[BTreeSet<String>]) -> String {
    let mut out = String::new();
    let mut state = WriteState {
        forced: forced.to_vec(),
        written_nodes: BTreeSet::new(),
        written_edges: BTreeSet::new(),
        attrs_written_nodes: BTreeSet::new(),
        subgraph_edges: subgraph_edge_membership(graph),
    };

    if graph.strict {
        out.push_str("strict ");
    }
    match graph.kind {
        GraphKind::Directed => out.push_str("digraph "),
        GraphKind::Undirected => out.push_str("graph "),
    }
    if !graph.name.is_empty() {
        out.push_str(&canon_str(&graph.name));
        out.push(' ');
    }
    out.push_str("{\n");

    let all_nodes: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
    let all_edges: Vec<EdgeId> = graph.edges.iter().map(|e| e.id).collect();

    let mut root_node_defaults = effective_node_defaults(graph, &all_nodes, &graph.node_defaults);
    // dot declares `label="\N"` as the node default when the input does not.
    if !root_node_defaults.contains_key("label") {
        root_node_defaults.insert("label".to_string(), "\\N".to_string());
    }
    let root_edge_defaults = effective_edge_defaults(graph, &all_edges, &graph.edge_defaults);

    write_dict(&mut out, 1, "graph", &graph.graph_attrs, None, None);
    write_dict(&mut out, 1, "node", &root_node_defaults, None, None);
    write_dict(&mut out, 1, "edge", &root_edge_defaults, None, None);

    // Determine top-level subgraphs (those not nested inside another subgraph)
    let mut child_subgraphs = BTreeSet::new();
    for subg in &graph.subgraphs {
        for &child_id in &subg.subgraphs {
            child_subgraphs.insert(child_id);
        }
    }

    for subg in &graph.subgraphs {
        if !child_subgraphs.contains(&subg.id) {
            write_subgraph(
                graph,
                subg.id,
                &graph.graph_attrs,
                &root_node_defaults,
                &root_edge_defaults,
                1,
                &mut state,
                &mut out,
            );
        }
    }

    write_nodes_and_edges(
        graph,
        &all_nodes,
        &all_edges,
        &root_node_defaults,
        &root_edge_defaults,
        1,
        &mut state,
        &mut out,
    );

    out.push_str("}\n");
    out
}

struct WriteState {
    forced: Vec<BTreeSet<String>>,
    written_nodes: BTreeSet<NodeId>,
    written_edges: BTreeSet<EdgeId>,
    attrs_written_nodes: BTreeSet<NodeId>,
    subgraph_edges: Vec<Vec<EdgeId>>,
}

/// Edge lists per subgraph. After ranking, dot adds every edge whose endpoints both lie in a
/// cluster to that cluster and its ancestors (`mark_lowclusters`), so `-Tcanon` prints them there.
fn subgraph_edge_membership(graph: &Graph) -> Vec<Vec<EdgeId>> {
    let mut parent: Vec<Option<usize>> = vec![None; graph.subgraphs.len()];
    for (i, sg) in graph.subgraphs.iter().enumerate() {
        for child in &sg.subgraphs {
            parent[child.0 as usize] = Some(i);
        }
    }
    let mut sets: Vec<BTreeSet<EdgeId>> = graph
        .subgraphs
        .iter()
        .map(|sg| sg.edges.iter().copied().collect())
        .collect();
    for (i, sg) in graph.subgraphs.iter().enumerate() {
        if !sg.is_cluster {
            continue;
        }
        let members: BTreeSet<NodeId> = sg.nodes.iter().copied().collect();
        for e in &graph.edges {
            if members.contains(&e.tail) && members.contains(&e.head) {
                let mut cur = Some(i);
                while let Some(c) = cur {
                    sets[c].insert(e.id);
                    cur = parent[c];
                }
            }
        }
    }
    sets.into_iter().map(|s| s.into_iter().collect()).collect()
}

fn effective_node_defaults(graph: &Graph, nodes: &[NodeId], defaults: &AttrMap) -> AttrMap {
    defaults.filtered(|k, _| {
        nodes
            .iter()
            .all(|nid| graph.nodes[nid.0 as usize].attrs.contains_key(k))
    })
}

fn effective_edge_defaults(graph: &Graph, edges: &[EdgeId], defaults: &AttrMap) -> AttrMap {
    defaults.filtered(|k, _| {
        edges
            .iter()
            .all(|eid| graph.edges[eid.0 as usize].attrs.contains_key(k))
    })
}

#[allow(clippy::too_many_arguments)]
fn write_subgraph(
    graph: &Graph,
    subg_id: SubgraphId,
    parent_graph_attrs: &AttrMap,
    parent_node_defaults: &AttrMap,
    parent_edge_defaults: &AttrMap,
    level: usize,
    state: &mut WriteState,
    out: &mut String,
) {
    let subg = &graph.subgraphs[subg_id.0 as usize];
    let forced = state
        .forced
        .get(subg_id.0 as usize)
        .cloned()
        .unwrap_or_default();
    let subg_node_defaults = effective_node_defaults(graph, &subg.nodes, &subg.node_defaults);
    let subg_edge_defaults = effective_edge_defaults(graph, &subg.edges, &subg.edge_defaults);

    if forced.is_empty()
        && is_irrelevant_subgraph(
            subg.name.as_str(),
            &subg.graph_attrs,
            &subg_node_defaults,
            &subg_edge_defaults,
            parent_graph_attrs,
            parent_node_defaults,
            parent_edge_defaults,
        )
    {
        for &child_id in &subg.subgraphs {
            write_subgraph(
                graph,
                child_id,
                parent_graph_attrs,
                parent_node_defaults,
                parent_edge_defaults,
                level,
                state,
                out,
            );
        }
        return;
    }

    push_indent(out, level);
    if !subg.name.is_empty() {
        out.push_str("subgraph ");
        out.push_str(&canon_value(&subg.name, subg.html_name));
        out.push_str(" {\n");
    } else {
        out.push_str("{\n");
    }

    let inner_level = level + 1;
    write_dict(
        out,
        inner_level,
        "graph",
        &subg.graph_attrs,
        Some(parent_graph_attrs),
        Some(&forced),
    );
    write_dict(
        out,
        inner_level,
        "node",
        &subg_node_defaults,
        Some(parent_node_defaults),
        None,
    );
    write_dict(
        out,
        inner_level,
        "edge",
        &subg_edge_defaults,
        Some(parent_edge_defaults),
        None,
    );

    for &child_id in &subg.subgraphs {
        write_subgraph(
            graph,
            child_id,
            &subg.graph_attrs,
            &subg_node_defaults,
            &subg_edge_defaults,
            inner_level,
            state,
            out,
        );
    }

    let scope_edges = state.subgraph_edges[subg_id.0 as usize].clone();
    write_nodes_and_edges(
        graph,
        &subg.nodes,
        &scope_edges,
        &subg_node_defaults,
        &subg_edge_defaults,
        inner_level,
        state,
        out,
    );

    push_indent(out, level);
    out.push_str("}\n");
}

#[allow(clippy::too_many_arguments)]
fn write_nodes_and_edges(
    graph: &Graph,
    scope_nodes: &[NodeId],
    scope_edges: &[EdgeId],
    node_defaults: &AttrMap,
    edge_defaults: &AttrMap,
    level: usize,
    state: &mut WriteState,
    out: &mut String,
) {
    let scope_edge_set: BTreeSet<EdgeId> = scope_edges.iter().copied().collect();

    for &node_id in scope_nodes {
        let node = &graph.nodes[node_id.0 as usize];
        let has_edges_in_scope = graph
            .edges
            .iter()
            .any(|e| scope_edge_set.contains(&e.id) && (e.tail == node_id || e.head == node_id));
        let non_default = collect_non_default_node_attrs(node, node_defaults);

        if !state.written_nodes.contains(&node_id)
            && (!has_edges_in_scope || !non_default.is_empty())
        {
            write_node_stmt(out, level, node, &non_default, state);
        }

        // Out-edges iterate by head node sequence, then edge sequence (`agedgeseqcmpf`).
        let mut out_edges: Vec<&EdgeData> = graph
            .edges
            .iter()
            .filter(|e| e.tail == node_id && scope_edge_set.contains(&e.id))
            .collect();
        out_edges.sort_by_key(|e| (e.head, e.id));
        for edge in out_edges {
            if !state.written_edges.contains(&edge.id) {
                let head_node = &graph.nodes[edge.head.0 as usize];
                let head_non_default = collect_non_default_node_attrs(head_node, node_defaults);
                if edge.head != node_id
                    && !state.written_nodes.contains(&edge.head)
                    && !head_non_default.is_empty()
                {
                    write_node_stmt(out, level, head_node, &head_non_default, state);
                }
                write_edge_stmt(out, level, graph, edge, edge_defaults, state);
            }
        }
    }
}

fn write_node_stmt(
    out: &mut String,
    level: usize,
    node: &NodeData,
    non_default_attrs: &[(&String, &String)],
    state: &mut WriteState,
) {
    push_indent(out, level);
    out.push_str(&canon_value(&node.name, node.html_name));
    if !state.attrs_written_nodes.contains(&node.id) && !non_default_attrs.is_empty() {
        write_inline_attrs(out, level, non_default_attrs, &node.attrs);
        state.attrs_written_nodes.insert(node.id);
    }
    out.push_str(";\n");
    state.written_nodes.insert(node.id);
}

fn write_edge_stmt(
    out: &mut String,
    level: usize,
    graph: &Graph,
    edge: &EdgeData,
    edge_defaults: &AttrMap,
    state: &mut WriteState,
) {
    let tail_node = &graph.nodes[edge.tail.0 as usize];
    let head_node = &graph.nodes[edge.head.0 as usize];

    push_indent(out, level);
    out.push_str(&canon_value(&tail_node.name, tail_node.html_name));
    write_port(out, &edge.tail_port);

    match graph.kind {
        GraphKind::Directed => out.push_str(" -> "),
        GraphKind::Undirected => out.push_str(" -- "),
    }

    out.push_str(&canon_value(&head_node.name, head_node.html_name));
    write_port(out, &edge.head_port);

    let non_default = collect_non_default_edge_attrs(edge, edge_defaults);
    if !non_default.is_empty() {
        write_inline_attrs(out, level, &non_default, &edge.attrs);
    }

    out.push_str(";\n");
    state.written_edges.insert(edge.id);
    state.written_nodes.insert(edge.tail);
    state.written_nodes.insert(edge.head);
}

fn write_port(out: &mut String, port: &Port) {
    if let Some(name) = &port.name {
        out.push(':');
        out.push_str(&canon_str(name));
    }
    if let Some(compass) = &port.compass {
        out.push(':');
        out.push_str(&canon_str(compass));
    }
}

fn write_dict(
    out: &mut String,
    level: usize,
    dict_name: &str,
    attrs: &AttrMap,
    parent_attrs: Option<&AttrMap>,
    forced: Option<&BTreeSet<String>>,
) {
    let entries: Vec<(&String, &String)> = attrs
        .iter()
        .filter(|(k, v)| match parent_attrs {
            Some(_) if forced.is_some_and(|f| f.contains(*k)) => true,
            Some(parent) => parent.get(*k) != Some(*v) || parent.is_html(k) != attrs.is_html(k),
            None => !v.is_empty(),
        })
        .collect();

    if entries.is_empty() {
        return;
    }

    push_indent(out, level);
    out.push_str(dict_name);
    out.push_str(" [");
    for (idx, (k, v)) in entries.iter().enumerate() {
        if idx > 0 {
            out.push_str(",\n");
            push_indent(out, level + 1);
        }
        out.push_str(&canon_str(k));
        out.push('=');
        out.push_str(&canon_value(v, attrs.is_html(k)));
    }
    if entries.len() > 1 {
        out.push('\n');
        push_indent(out, level);
    }
    out.push_str("];\n");
}

fn write_inline_attrs(out: &mut String, level: usize, attrs: &[(&String, &String)], all: &AttrMap) {
    out.push_str("\t[");
    for (idx, (k, v)) in attrs.iter().enumerate() {
        if idx > 0 {
            out.push_str(",\n");
            push_indent(out, level + 1);
        }
        out.push_str(&canon_str(k));
        out.push('=');
        out.push_str(&canon_value(v, all.is_html(k)));
    }
    out.push(']');
}

fn collect_non_default_node_attrs<'a>(
    node: &'a NodeData,
    node_defaults: &AttrMap,
) -> Vec<(&'a String, &'a String)> {
    node.attrs
        .iter()
        .filter(|(k, v)| match node_defaults.get(*k) {
            Some(def_val) => *v != def_val || node.attrs.is_html(k) != node_defaults.is_html(k),
            None => !(*k == "label" && *v == "\\N"),
        })
        .collect()
}

fn collect_non_default_edge_attrs<'a>(
    edge: &'a EdgeData,
    edge_defaults: &AttrMap,
) -> Vec<(&'a String, &'a String)> {
    edge.attrs
        .iter()
        .filter(|(k, v)| {
            if (*k == "tailport" && edge.tail_port != Port::default())
                || (*k == "headport" && edge.head_port != Port::default())
            {
                return false;
            }
            edge_defaults.get(*k) != Some(*v) || edge.attrs.is_html(k) != edge_defaults.is_html(k)
        })
        .collect()
}

fn is_irrelevant_subgraph(
    name: &str,
    graph_attrs: &AttrMap,
    node_defaults: &AttrMap,
    edge_defaults: &AttrMap,
    parent_graph_attrs: &AttrMap,
    parent_node_defaults: &AttrMap,
    parent_edge_defaults: &AttrMap,
) -> bool {
    name.is_empty()
        && graph_attrs
            .iter()
            .all(|(k, v)| parent_graph_attrs.get(k) == Some(v))
        && node_defaults
            .iter()
            .all(|(k, v)| parent_node_defaults.get(k) == Some(v))
        && edge_defaults
            .iter()
            .all(|(k, v)| parent_edge_defaults.get(k) == Some(v))
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push('\t');
    }
}

/// `agstrcanon`: an HTML string (`html`, with `s` its inner text) is written `<s>`; anything
/// else (a quoted `"<x>"` too) is quoted as needed.
pub fn canon_value(s: &str, html: bool) -> String {
    if html { format!("<{s}>") } else { canon_str(s) }
}

/// Canonicalizes a single identifier or string value following `_agstrcanon` in `write.c`,
/// breaking long strings after 128 characters like `agwrite` (`MAX_OUTPUTLINE`).
pub fn canon_str(s: &str) -> String {
    canon_str_with(s, 128)
}

/// [`canon_str`] with an explicit maximum output line length (`linelength`; 0 disables
/// line breaking).
pub fn canon_str_with(arg: &str, max_line: usize) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    // `_agstrcanon` works on C's bytes (`graphviz_pure_core::charset`).
    let bytes = graphviz_pure_core::charset::encode(arg);
    let b = &*bytes;
    let is_id_char = |c: u8| c.is_ascii_alphanumeric() || c == b'.' || c == b'-' || !c.is_ascii();
    let is_escape =
        |i: usize| b[i] == b'\\' && b.get(i + 1).is_some_and(|&c| is_graphviz_escape_char(c));
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 4);
    out.push(b'"');
    let mut cnt = 0usize;
    let mut dotcnt = 0usize;
    let mut needs_quotes = false;
    let mut part_of_escape = false;
    let mut backslash_pending = false;
    let mut maybe_num = b[0].is_ascii_digit() || b[0] == b'.' || b[0] == b'-';
    let mut i = 0usize;
    while i < b.len() {
        let uc = b[i];
        if uc == b'"' && !part_of_escape {
            out.push(b'\\');
            needs_quotes = true;
        } else if !part_of_escape && is_escape(i) {
            needs_quotes = true;
            part_of_escape = true;
        } else if maybe_num {
            if uc == b'-' {
                if cnt > 0 {
                    maybe_num = false;
                    needs_quotes = true;
                }
            } else if uc == b'.' {
                if dotcnt > 0 {
                    maybe_num = false;
                    needs_quotes = true;
                }
                dotcnt += 1;
            } else if !uc.is_ascii_digit() {
                maybe_num = false;
                needs_quotes = true;
            }
            part_of_escape = false;
        } else if !(uc.is_ascii_alphanumeric() || uc == b'_' || !uc.is_ascii()) {
            needs_quotes = true;
            part_of_escape = false;
        } else {
            part_of_escape = false;
        }
        out.push(uc);
        i += 1;
        cnt += 1;
        if max_line > 0 && i < b.len() {
            let next = b[i];
            let last = *out.last().unwrap();
            if backslash_pending && !(is_id_char(last) || last == b'\\') && is_id_char(next) {
                out.push(b'\\');
                out.push(b'\n');
                needs_quotes = true;
                backslash_pending = false;
                cnt = 0;
            } else if cnt >= max_line {
                if !(is_id_char(last) || last == b'\\') && is_id_char(next) {
                    out.push(b'\\');
                    out.push(b'\n');
                    needs_quotes = true;
                    cnt = 0;
                } else {
                    backslash_pending = true;
                }
            }
        }
    }
    out.push(b'"');
    let quoted = graphviz_pure_core::charset::decode(out);
    if needs_quotes || (cnt == 1 && (b[0] == b'.' || b[0] == b'-')) {
        return quoted;
    }
    if ["node", "edge", "strict", "graph", "digraph", "subgraph"]
        .iter()
        .any(|t| t.eq_ignore_ascii_case(arg))
    {
        return quoted;
    }
    arg.to_string()
}

fn is_graphviz_escape_char(b: u8) -> bool {
    matches!(
        b,
        b'E' | b'G' | b'H' | b'L' | b'N' | b'T' | b'l' | b'n' | b'r' | b'\\' | b'"'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::charset;

    // Issue #151: `_agstrcanon` counts C's bytes when it breaks a long string: a Latin-1
    // byte is one byte (C 16.1.0 breaks this label after 43 of its 60 groups).
    #[test]
    fn long_strings_break_after_128_bytes_of_c() {
        let s = charset::decode(b" \xe9y".repeat(60));
        let expected = [
            &b"\""[..],
            &b" \xe9y".repeat(43),
            b" \\\n\xe9y",
            &b" \xe9y".repeat(16),
            b"\"",
        ]
        .concat();
        assert_eq!(&*charset::encode(&canon_str(&s)), &expected[..]);
    }
}
