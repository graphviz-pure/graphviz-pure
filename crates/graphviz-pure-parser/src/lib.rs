//! Re-entrant DOT and HTML-label parser and canonical serializer (`graphviz-pure-parser`).
//! Fixes GV-BUG-001 (global mutable lexer state) and GV-BUG-005 (memory leaks on syntax errors).

pub mod canon;
pub mod lexer;
pub mod parser;

pub use canon::{canon_str, canon_str_with, canon_value, write_canon, write_canon_forced};
pub use lexer::{Lexer, Token, TokenKind, reading_file};
pub use parser::{GlobalAttrs, GraphStream, Parser};

use graphviz_pure_core::{Graph, Result};

/// Parses a DOT graph specification into an arena-backed [`Graph`].
pub fn parse_dot(input: &str) -> Result<Graph> {
    Parser::parse_str(input)
}

/// Parses every graph in a DOT stream (`digraph G {..} digraph H {..}`), applying `globals`
/// (`-G`/`-N`/`-E` command line attributes) as if declared at the top of each graph.
pub fn parse_dot_all(input: &str, globals: &GlobalAttrs) -> Result<Vec<Graph>> {
    Parser::parse_all(input, globals)
}

/// [`parse_dot_all`] keeping the graphs read before a syntax error, with the error.
pub fn parse_dot_all_partial(
    input: &str,
    globals: &GlobalAttrs,
) -> (Vec<Graph>, Option<graphviz_pure_core::GraphvizError>) {
    Parser::parse_all_partial(input, globals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::GraphKind;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn read_fixture(name: &str) -> String {
        let path = fixture_path(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
    }

    #[test]
    fn parse_all_20_fixtures_and_roundtrip_canon() {
        let fixtures = [
            "ER.gv",
            "Heawood.gv",
            "NaN.gv",
            "Petersen.gv",
            "abstract.gv",
            "alf.gv",
            "arrows.gv",
            "clust.gv",
            "clust1.gv",
            "clust2.gv",
            "clust3.gv",
            "clust4.gv",
            "fsm.gv",
            "html.gv",
            "html2.gv",
            "pack.gv",
            "record.gv",
            "shapes.gv",
            "unix.gv",
            "world.gv",
        ];

        for name in fixtures {
            let src = read_fixture(name);
            let graph =
                parse_dot(&src).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"));
            assert!(
                !graph.nodes.is_empty(),
                "fixture {name} should have at least 1 node"
            );

            let canon = write_canon(&graph);
            let reparsed = parse_dot(&canon).unwrap_or_else(|e| {
                panic!("failed to re-parse canonical output for {name}: {e}\nCanon:\n{canon}")
            });
            assert_eq!(
                graph.kind, reparsed.kind,
                "kind mismatch after canon roundtrip for {name}"
            );
            assert_eq!(
                graph.nodes.len(),
                reparsed.nodes.len(),
                "node count mismatch after canon roundtrip for {name}"
            );
            assert_eq!(
                graph.edges.len(),
                reparsed.edges.len(),
                "edge count mismatch after canon roundtrip for {name}"
            );
        }
    }

    #[test]
    fn verify_fsm_fixture() {
        let g = parse_dot(&read_fixture("fsm.gv")).unwrap();
        assert_eq!(g.name, "finite_state_machine");
        assert_eq!(g.kind, GraphKind::Directed);
        assert!(!g.strict);
        assert_eq!(g.nodes.len(), 9);
        assert_eq!(g.edges.len(), 14);
        assert_eq!(g.graph_attrs.get("rankdir").map(String::as_str), Some("LR"));

        let lr0 = g.node_lookup["LR_0"];
        let lr1 = g.node_lookup["LR_1"];
        assert_eq!(
            g.nodes[lr0.0 as usize]
                .attrs
                .get("shape")
                .map(String::as_str),
            Some("doublecircle")
        );
        assert_eq!(
            g.nodes[lr1.0 as usize]
                .attrs
                .get("shape")
                .map(String::as_str),
            Some("circle")
        );
    }

    #[test]
    fn verify_clusters_and_attribute_scoping() {
        let clust = parse_dot(&read_fixture("clust.gv")).unwrap();
        assert_eq!(clust.nodes.len(), 8);
        assert_eq!(clust.edges.len(), 9);
        assert_eq!(clust.subgraphs.len(), 2);
        assert!(clust.subgraphs[0].is_cluster);
        assert_eq!(clust.subgraphs[0].name, "cluster_0");
        assert_eq!(
            clust.subgraphs[0]
                .graph_attrs
                .get("label")
                .map(String::as_str),
            Some("hello world")
        );
        assert_eq!(clust.subgraphs[0].nodes.len(), 3);
        assert_eq!(clust.subgraphs[0].edges.len(), 2);

        let clust4 = parse_dot(&read_fixture("clust4.gv")).unwrap();
        assert_eq!(clust4.nodes.len(), 10);
        assert_eq!(clust4.edges.len(), 13);
        assert_eq!(clust4.subgraphs.len(), 2);
        let a0 = clust4.node_lookup["a0"];
        let b0 = clust4.node_lookup["b0"];
        let start = clust4.node_lookup["start"];
        assert_eq!(
            clust4.nodes[a0.0 as usize]
                .attrs
                .get("color")
                .map(String::as_str),
            Some("white")
        );
        assert_eq!(
            clust4.nodes[b0.0 as usize].attrs.get("color"),
            None,
            "color=white in cluster_0 must not leak into cluster_1"
        );
        assert_eq!(
            clust4.nodes[start.0 as usize]
                .attrs
                .get("shape")
                .map(String::as_str),
            Some("Mdiamond")
        );
    }

    #[test]
    fn canon_distinguishes_quoted_and_html_labels() {
        let g = parse_dot(
            "digraph { a [label=\"<x>\"]; b [label=<<b>x</b>>]; a -> b [label=\"<e>\"]; }",
        )
        .unwrap();
        let canon = write_canon(&g);
        assert!(canon.contains("a\t[label=\"<x>\"];"), "{canon}");
        assert!(canon.contains("b\t[label=<<b>x</b>>];"), "{canon}");
        assert!(canon.contains("a -> b\t[label=\"<e>\"];"), "{canon}");
        assert!(canon.contains("node [label=\"\\N\"];"), "{canon}");
    }

    #[test]
    fn command_line_attrs_are_fixed_at_root() {
        // `global_def` marks the symbols `fixed`: root attribute statements keep
        // the command-line value, subgraphs and individual objects may change it.
        let mut globals = GlobalAttrs::default();
        globals.graph.insert("rankdir".into(), "LR".into());
        globals.node.insert("shape".into(), "box".into());
        let src = "digraph G { rankdir=TB; graph [rankdir=BT]; node [shape=circle]; a; \
                   b [shape=point]; subgraph s { rankdir=RL; node [shape=egg]; c } }";
        let g = &parse_dot_all(src, &globals).unwrap()[0];
        assert_eq!(g.graph_attrs.get("rankdir").map(String::as_str), Some("LR"));
        let shape = |n: usize| g.nodes[n].attrs.get("shape").map(String::as_str);
        assert_eq!(shape(0), Some("box"));
        assert_eq!(shape(1), Some("point"));
        assert_eq!(shape(2), Some("egg"));
        assert_eq!(
            g.subgraphs[0]
                .graph_attrs
                .get("rankdir")
                .map(String::as_str),
            Some("RL")
        );
    }

    #[test]
    fn parse_dot_all_drops_the_rest_of_a_graphs_last_line() {
        // cgraph reads line by line and starts a new scanner per graph.
        let graphs = parse_dot_all(
            "digraph G { a -> b } graph H { c -- d }",
            &GlobalAttrs::default(),
        )
        .unwrap();
        assert_eq!(graphs.len(), 1);
    }

    #[test]
    fn parse_dot_all_returns_every_graph_with_globals() {
        let mut globals = GlobalAttrs::default();
        globals.graph.insert("rankdir".into(), "LR".into());
        globals.node.insert("shape".into(), "box".into());
        globals.edge.insert("color".into(), "red".into());
        let graphs = parse_dot_all("digraph G { a -> b }\ngraph H { c -- d }", &globals).unwrap();
        assert_eq!(graphs.len(), 2);
        assert_eq!(graphs[0].name, "G");
        assert_eq!(graphs[1].name, "H");
        assert_eq!(graphs[1].kind, GraphKind::Undirected);
        for g in &graphs {
            assert_eq!(g.graph_attrs.get("rankdir").map(String::as_str), Some("LR"));
            assert_eq!(
                g.nodes[0].attrs.get("shape").map(String::as_str),
                Some("box")
            );
            assert_eq!(
                g.edges[0].attrs.get("color").map(String::as_str),
                Some("red")
            );
        }
        assert_eq!(
            parse_dot("digraph G { a } digraph H { b }").unwrap().name,
            "G"
        );
        assert!(
            parse_dot_all("", &GlobalAttrs::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn canon_orders_out_edges_by_head_sequence() {
        let g = parse_dot("digraph { b; c; a -> c; a -> b; }").unwrap();
        let canon = write_canon(&g);
        assert!(canon.find("a -> b").unwrap() < canon.find("a -> c").unwrap());
    }

    // #365 review: write_canon copies the effective node and edge defaults with their HTML
    // flags, so it writes HTML defaults once, as HTML, as C 16.1.0's `-Tcanon` does, and
    // no node or edge repeats them.
    #[test]
    fn canon_keeps_html_defaults() {
        let g = parse_dot(
            "digraph { node [label=<<b>x</b>>]; edge [label=<<i>e</i>>]; a -> b; subgraph s { \
             node [xlabel=<<u>s</u>>]; edge [headlabel=<h>]; c -> d } }",
        )
        .unwrap();
        let canon = write_canon(&g);
        for line in [
            "\tnode [label=<<b>x</b>>];\n",
            "\tedge [label=<<i>e</i>>];\n",
            "\t\tnode [xlabel=<<u>s</u>>];\n",
            "\t\tedge [headlabel=<h>];\n",
            "\t\tc -> d;\n",
            "\ta -> b;\n",
        ] {
            assert!(canon.contains(line), "{line:?} in {canon}");
        }
        assert_eq!(canon.matches("label=").count(), 4, "{canon}");
    }

    #[test]
    fn verify_html_labels_and_ports() {
        let html = parse_dot(&read_fixture("html.gv")).unwrap();
        assert_eq!(html.nodes.len(), 3);
        assert_eq!(html.edges.len(), 2);
        for node in &html.nodes {
            let label = node.attrs.get("label").expect("missing label");
            assert!(
                node.attrs.is_html("label"),
                "expected HTML label, got {label}"
            );
            assert!(label.contains("<TABLE"));
        }
        assert_eq!(html.edges[0].tail_port.name.as_deref(), Some("f1"));
        assert_eq!(html.edges[0].head_port.name.as_deref(), Some("f0"));
        assert_eq!(html.edges[1].tail_port.name.as_deref(), Some("f2"));
        assert_eq!(html.edges[1].head_port.name.as_deref(), Some("here"));

        let html2 = parse_dot(&read_fixture("html2.gv")).unwrap();
        assert_eq!(html2.nodes.len(), 4);
        assert_eq!(html2.edges.len(), 3);
        assert_eq!(html2.subgraphs.len(), 1);
        assert_eq!(html2.edges[0].tail_port.name.as_deref(), Some("here"));
        assert_eq!(html2.edges[0].head_port.name.as_deref(), Some("there"));
        assert!(html2.edges[2].attrs.contains_key("label"));
        assert!(html2.edges[2].attrs.is_html("label"));
    }

    #[test]
    fn verify_er_heawood_petersen_pack_world() {
        let er = parse_dot(&read_fixture("ER.gv")).unwrap();
        assert_eq!(er.kind, GraphKind::Undirected);
        assert_eq!(er.nodes.len(), 12);
        assert_eq!(er.edges.len(), 12);
        let name0 = er.node_lookup["name0"];
        let code = er.node_lookup["code"];
        assert_eq!(
            er.nodes[name0.0 as usize]
                .attrs
                .get("label")
                .map(String::as_str),
            Some("name")
        );
        assert_eq!(
            er.nodes[code.0 as usize]
                .attrs
                .get("label")
                .map(String::as_str),
            Some("\\N")
        );

        let heawood = parse_dot(&read_fixture("Heawood.gv")).unwrap();
        assert_eq!(heawood.name, "Heawood");
        assert_eq!(heawood.nodes.len(), 14);
        assert_eq!(heawood.edges.len(), 21);

        let petersen = parse_dot(&read_fixture("Petersen.gv")).unwrap();
        assert_eq!(petersen.name, "Petersen");
        assert_eq!(petersen.nodes.len(), 10);
        assert_eq!(petersen.edges.len(), 15);
        assert_eq!(
            petersen.edges[0].attrs.get("color").map(String::as_str),
            Some("blue")
        );

        let pack = parse_dot(&read_fixture("pack.gv")).unwrap();
        assert_eq!(pack.nodes.len(), 24);
        assert_eq!(pack.edges.len(), 20);

        let world = parse_dot(&read_fixture("world.gv")).unwrap();
        assert_eq!(world.subgraphs.len(), 14);
        assert!(world.edges.len() > 50);
    }

    #[test]
    fn verify_strict_concat_compass_and_syntax_errors() {
        let dot = r#"
            # Preprocessor comment
            strict digraph "My" + "Graph" {
                a:p1:ne -> b:p2:sw [label="hello " + "world"][color=red];
                a:p1:ne -> b:p2:sw [weight=2];
                { x, y } -> { z };
            }
        "#;
        let g = parse_dot(dot).unwrap();
        assert!(g.strict);
        assert_eq!(g.name, "MyGraph");
        assert_eq!(g.edges.len(), 3); // strict merges duplicate a->b, plus x->z and y->z
        assert_eq!(g.edges[0].tail_port.name.as_deref(), Some("p1"));
        assert_eq!(g.edges[0].tail_port.compass.as_deref(), Some("ne"));
        assert_eq!(g.edges[0].head_port.name.as_deref(), Some("p2"));
        assert_eq!(g.edges[0].head_port.compass.as_deref(), Some("sw"));
        assert_eq!(
            g.edges[0].attrs.get("label").map(String::as_str),
            Some("hello world")
        );
        assert_eq!(
            g.edges[0].attrs.get("weight").map(String::as_str),
            Some("2")
        );

        // Directed edge in undirected graph must fail
        assert!(parse_dot("graph G { a -> b }").is_err());
        // Undirected edge in directed graph must fail
        assert!(parse_dot("digraph G { a -- b }").is_err());
        // Unclosed HTML string must fail
        assert!(parse_dot("digraph G { a [label=<<table>] }").is_err());
    }

    /// Issue #29. scan.l returns `T_node`, `T_edge`, `T_graph`, `T_digraph`,
    /// `T_subgraph` and `T_strict` for the keywords (flex `--case-insensitive`),
    /// and grammar.y's `atom : T_atom | qatom` accepts none of them, so an
    /// unquoted keyword is a syntax error as an attribute name or value, a
    /// port or a compass point. C 16.1.0 exits 1 on every input below.
    #[test]
    fn unquoted_keywords_are_not_atoms() {
        for src in [
            "digraph G { packmode=node; }",
            "digraph G { x=strict }",
            "digraph G { x=digraph }",
            "digraph G { a [label=edge] }",
            "digraph G { a [label=NODE] }",
            "digraph G { a [label=Edge] }",
            "digraph G { a [node=1] }",
            "digraph G { a [NODE=1] }",
            "digraph G { a [label=node, b=c] }",
            "graph G { graph [x=subgraph] }",
            "digraph G { a:graph -> b }",
            "digraph G { a -> b:strict }",
            "digraph G { a:b:node }",
            "digraph G { a -> edge }",
        ] {
            assert!(parse_dot(src).is_err(), "{src} should be a syntax error");
        }
    }

    /// Issue #29: quoted keywords are `T_qatom` in C, so they stay valid.
    #[test]
    fn quoted_keywords_are_atoms() {
        let g = parse_dot(
            r#"digraph G {
                packmode="node";
                "node"="edge";
                a [label="edge", "node"="1", x="a" + "graph"];
                a:"graph":"n" -> b;
                nodes; edged; graphx;
            }"#,
        )
        .unwrap();
        assert_eq!(
            g.graph_attrs.get("packmode").map(String::as_str),
            Some("node")
        );
        assert_eq!(g.graph_attrs.get("node").map(String::as_str), Some("edge"));
        let a = &g.nodes[g.node_lookup["a"].0 as usize].attrs;
        assert_eq!(a.get("label").map(String::as_str), Some("edge"));
        assert_eq!(a.get("node").map(String::as_str), Some("1"));
        assert_eq!(a.get("x").map(String::as_str), Some("agraph"));
        assert_eq!(g.edges[0].tail_port.name.as_deref(), Some("graph"));
        assert_eq!(g.edges[0].tail_port.compass.as_deref(), Some("n"));
        for name in ["nodes", "edged", "graphx"] {
            assert!(g.node_lookup.contains_key(name), "{name}");
        }
    }

    // #81: `concat` joins an HTML operand's inner text into a plain string.
    #[test]
    fn concatenation_joins_inner_text_into_a_plain_string() {
        let g = parse_dot("digraph { a [label=\"x\" + <b>, xlabel=<b> + \"y\"] }").unwrap();
        let a = &g.nodes[0].attrs;
        assert_eq!(a.get("label").map(String::as_str), Some("xb"));
        assert_eq!(a.get("xlabel").map(String::as_str), Some("by"));
        assert!(!a.is_html("label") && !a.is_html("xlabel"));
        let g = parse_dot("digraph { a [label=<b>] }").unwrap();
        let a = &g.nodes[0].attrs;
        assert_eq!(a.get("label").map(String::as_str), Some("b"));
        assert!(a.is_html("label"));
    }

    // #98: `concatPort` makes `port:compass` a plain string; a lone HTML port stays HTML.
    #[test]
    fn html_port_with_compass_is_a_plain_string() {
        let g = parse_dot("digraph { a:<foo>:n -> b:<x> }").unwrap();
        let e = &g.edges[0];
        assert_eq!(e.attrs.get("tailport").map(String::as_str), Some("foo:n"));
        assert!(!e.attrs.is_html("tailport"));
        assert_eq!(e.tail_port.name.as_deref(), Some("foo"));
        assert_eq!(e.tail_port.compass.as_deref(), Some("n"));
        assert_eq!(e.attrs.get("headport").map(String::as_str), Some("x"));
        assert!(e.attrs.is_html("headport"));
        assert_eq!(e.head_port.name.as_deref(), Some("x"));
    }

    // #99: an attribute is named by the inner text; the first declaration's flag names it.
    #[test]
    fn html_attribute_name_is_its_inner_text() {
        let g = parse_dot("digraph { a [<zzz>=1]; b [zzz=2]; c [label=1, <label>=2] }").unwrap();
        assert_eq!(g.nodes[0].attrs.get("zzz").map(String::as_str), Some("1"));
        assert_eq!(g.nodes[1].attrs.get("zzz").map(String::as_str), Some("2"));
        assert!(g.dicts[0].node["zzz"].html);
        // `label` is declared by the ProtoGraph, as plain text.
        assert_eq!(g.nodes[2].attrs.get("label").map(String::as_str), Some("2"));
        assert!(!g.dicts[0].node["label"].html);
    }

    // #119: a node name keeps the flag; `<b>`, `b` and `"<b>"` are three nodes (`refstr_eq`).
    #[test]
    fn html_node_name_is_its_inner_text_and_a_flag() {
        let g = parse_dot("digraph <G> { <b>; b; \"<b>\"; subgraph <s> { <b> } }").unwrap();
        assert_eq!(g.name, "G");
        let names: Vec<(&str, bool)> = g
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.html_name))
            .collect();
        assert_eq!(names, [("b", true), ("b", false), ("<b>", false)]);
        assert_eq!(g.subgraphs[0].name, "s");
        assert!(g.subgraphs[0].html_name);
        assert_eq!(g.subgraphs[0].nodes, [g.nodes[0].id]);
    }

    fn edge_list(
        g: &graphviz_pure_core::Graph,
    ) -> Vec<(String, String, Option<String>, Option<&str>)> {
        g.edges
            .iter()
            .map(|e| {
                (
                    g.nodes[e.tail.0 as usize].name.clone(),
                    g.nodes[e.head.0 as usize].name.clone(),
                    e.key.clone(),
                    e.attrs.get("color").map(String::as_str),
                )
            })
            .collect()
    }

    // #92: in a strict graph, `agedge` with a key that names no edge makes none where the
    // nodes already have one (`ok_to_make_edge`), so `newedge` skips `mkport` and
    // `applyattrs`; the list's attributes are still declared (`bindattrs`).
    #[test]
    fn strict_graph_new_key_on_an_existing_edge_makes_nothing() {
        let g = parse_dot("strict digraph G { a -> b; a -> b:n [key=x, color=red]; }").unwrap();
        assert_eq!(edge_list(&g), [("a".into(), "b".into(), None, None)]);
        assert!(!g.edges[0].attrs.contains_key("headport"));
        assert!(g.dicts[0].edge.contains_key("color"));
        // `ok_to_make_edge` looks in the current subgraph only, and only from tail to head.
        let g = parse_dot("strict digraph { a -> b; subgraph s { a -> b [key=x, color=red] } }")
            .unwrap();
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.subgraphs[0].edges, [g.edges[1].id]);
        let g = parse_dot("strict graph { a -- b; b -- a [key=x, color=red] }").unwrap();
        assert_eq!(g.edges.len(), 2);
        // Without a key, the existing edge takes the statement's attributes.
        let g = parse_dot("strict graph { a -- b; b -- a [color=red] }").unwrap();
        assert_eq!(edge_list(&g), [("a".into(), "b".into(), None, Some("red"))]);
    }

    // #295: a key names one edge between two nodes (either way round when undirected), in
    // any graph; `key` is the edge's name, not an attribute (#297).
    #[test]
    fn a_repeated_key_names_the_same_edge() {
        let g =
            parse_dot("digraph { a -> b [key=x, color=red]; a -> b [key=x, label=y] }").unwrap();
        assert_eq!(
            edge_list(&g),
            [("a".into(), "b".into(), Some("x".into()), Some("red"))]
        );
        assert_eq!(g.edges[0].attrs.get("label").map(String::as_str), Some("y"));
        assert!(!g.edges[0].attrs.contains_key("key"));
        let g = parse_dot("graph { a -- b [key=x]; b -- a [key=x, color=red]; a -- b }").unwrap();
        assert_eq!(g.edges.len(), 2);
        assert_eq!(
            g.edges[0].attrs.get("color").map(String::as_str),
            Some("red")
        );
        // the HTML flag is part of the name
        let g = parse_dot("digraph { a -> b [key=<x>]; a -> b [key=\"x\"]; a -> b [key=<x>] }")
            .unwrap();
        assert_eq!(g.edges.len(), 2);
        assert!(g.edges[0].key_html && !g.edges[1].key_html);
    }

    // An edge statement's subgraph gives its nodes as they are once the statement is read.
    #[test]
    fn an_edge_statement_takes_a_subgraphs_nodes_at_its_end() {
        let g = parse_dot("digraph { subgraph s {a} -> subgraph s {b} }").unwrap();
        let ends: Vec<(String, String)> = edge_list(&g)
            .into_iter()
            .map(|(t, h, _, _)| (t, h))
            .collect();
        let pairs = [("a", "a"), ("a", "b"), ("b", "a"), ("b", "b")];
        assert_eq!(ends, pairs.map(|(t, h)| (t.to_string(), h.to_string())));
    }

    // #303: only a missing name makes a graph anonymous; "" and <> are names.
    #[test]
    fn an_empty_name_is_a_name() {
        let g = parse_dot(
            "digraph { subgraph \"\" { a } subgraph \"\" { b } { c } subgraph <> { d } }",
        )
        .unwrap();
        assert_eq!(g.subgraphs.len(), 3);
        assert!(!g.subgraphs[0].is_anonymous() && g.subgraphs[0].nodes.len() == 2);
        assert!(g.subgraphs[1].is_anonymous());
        assert!(!g.subgraphs[2].is_anonymous() && g.subgraphs[2].html_name);
        assert_eq!(g.anon_subgraph_ids.keys().collect::<Vec<_>>(), [&1]);
        assert!(g.is_anonymous());
        assert!(!parse_dot("digraph \"\" { }").unwrap().is_anonymous());
    }

    // A local name (`%…`) takes an anonymous id from the counter the root, anonymous
    // subgraphs and keyless edges share, once per name, and is then called `%<id>`.
    #[test]
    fn local_names_take_anonymous_ids() {
        let g = parse_dot(
            "digraph { \"%l\" -> b; \"%l\" -> \"%3\"; subgraph \"%s\" { c } { d } subgraph \"%s\" { e } }",
        )
        .unwrap();
        let names: Vec<&str> = g.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["%3", "b", "%7", "c", "d", "e"]);
        let ids: Vec<u64> = g.anon_subgraph_ids.values().copied().collect();
        assert_eq!(ids, [11, 13]);
        assert_eq!(g.subgraphs.len(), 2);
        // a local key names its edge while the graph is read, and no edge once it is
        let g =
            parse_dot("digraph { a -> b [key=\"%k\"]; a -> b [key=\"%k\", color=red] }").unwrap();
        assert_eq!(g.edges.len(), 1);
    }

    // Issue #291: a subgraph's defaults declare their attributes on the root's dictionary
    // (`agfindnodeattr` finds them from any graph), not only for the subgraph.
    #[test]
    fn subgraph_defaults_declare_attributes_for_the_whole_graph() {
        let g = parse_dot("graph { subgraph s { node [z=1]; edge [len=2]; foo=1 } a; b -- c }")
            .unwrap();
        assert!(g.node_attr_declared("z"));
        assert!(g.edge_attr_declared("len"));
        assert!(g.graph_attr_declared("foo"));
        assert!(g.node_attr_declared("label"));
        assert!(!g.node_attr_declared("len") && !g.edge_attr_declared("z"));
        assert!(!g.graph_attr_declared("z"));
    }

    // The root's dictionary is the whole answer when there is one: a value stored on an
    // object (a layout's write-back, a cluster's `bb`) without a declaration doesn't count,
    // as `agattr(root, kind, key, NULL)` returns NULL in C. `a -> b:p` declares `headport`
    // through `mkport` only.
    #[test]
    fn the_root_dictionary_alone_says_what_is_declared() {
        let mut g = parse_dot("digraph { subgraph cluster_c { x } a -> b:p }").unwrap();
        assert!(g.edge_attr_declared("headport") && !g.edge_attr_declared("tailport"));
        g.nodes[0].attrs.insert("pos".into(), "1,2".into());
        g.edges[0].attrs.insert("len".into(), "3".into());
        g.subgraphs[0]
            .graph_attrs
            .insert("bb".into(), "0,0,1,1".into());
        assert!(!g.node_attr_declared("pos"));
        assert!(!g.edge_attr_declared("len"));
        assert!(!g.graph_attr_declared("bb"));
        // A graph built without dictionaries falls back to its defaults and objects.
        let mut bare = graphviz_pure_core::Graph::new("g", GraphKind::Undirected, false);
        bare.node_defaults.insert("z".into(), String::new());
        bare.graph_attrs.insert("dim".into(), "3".into());
        assert!(bare.node_attr_declared("z") && bare.graph_attr_declared("dim"));
        assert!(!bare.edge_attr_declared("len"));
    }

    // Every symbol a subgraph's dictionary holds is also in the root's (`bindattrs` declares
    // it there first), so looking only at `dicts[0]` misses nothing.
    #[test]
    fn subgraph_dictionaries_are_covered_by_the_root_dictionary() {
        for src in [
            "graph { subgraph s { node [z=1]; edge [len=2]; foo=1; subgraph t { node [z=2, w=3] } } }",
            "digraph { node [shape=box]; subgraph cluster_a { node [color=red]; label=x; a -> b [weight=2] } }",
            "graph { subgraph { edge [style=dashed]; a -- b } subgraph s { s2=1 } subgraph s { s3=2 } }",
        ] {
            let g = parse_dot(src).unwrap();
            let root = &g.dicts[0];
            for d in &g.dicts[1..] {
                assert!(d.graph.keys().all(|k| root.graph.contains_key(k)), "{src}");
                assert!(d.node.keys().all(|k| root.node.contains_key(k)), "{src}");
                assert!(d.edge.keys().all(|k| root.edge.contains_key(k)), "{src}");
            }
        }
    }
}
