use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> (String, String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A run rejected on its arguments may exit before reading stdin.
    if let Err(e) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

// Issue #181: neato -n2's user_spline (lib/neatogen/neatoinit.c) skips one byte after a
// segment of an edge `pos`. The port sliced the `str` there and panicked when the byte began a
// multi-byte character. C rejects the spline with a warning, routes the edge and exits 0;
// its stderr is Graphviz 16.1.0's.
#[test]
fn edge_pos_ending_in_multibyte_char_does_not_panic() {
    let src = "digraph { a [pos=\"0,0\"]; b [pos=\"30,30\"]; \
               a -> b [pos=\"0,0 10,10 20,20 30,30é\"] }";
    let (out, err, ok) = run(&["-Kneato", "-n2", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: pos attribute for edge (a,b) doesn't have 3n+1 points\n"
    );
    assert!(out.contains("<path"), "the edge is still drawn");
}

// Cause C2 (job pipeline): `dot` runs as `cmd/dot/dot.c`, `lib/common/args.c` and gvc's
// `gvLayoutJobs`/`gvRenderJobs`. Expected output is C 16.1.0's.

/// [`run`] in `dir`, with the exit status.
fn run_in(dir: &std::path::Path, args: &[&str], stdin: &str) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(e) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// A fresh scratch directory.
fn scratch(name: &str) -> PathBuf {
    let dir = tmp(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Issue #158: one job per `-T`, each through `emit_graph`.
#[test]
fn every_format_flag_renders_its_own_job() {
    let (out, err, ok) = run(&["-Tsvg", "-Tdot"], "digraph{s [style=\"bogus(1\"]}");
    assert!(!ok);
    let svg_end = out.find("</svg>").expect("the svg comes first");
    assert!(out[svg_end..].contains("digraph {"), "{out}");
    assert_eq!(err, "Error: unmatched '(' in style: bogus(1\n".repeat(4));
}

// Issues #125 and #281: the render-time diagnostics of `emit_graph` for every format; json
// renders the graph as xdot first.
#[test]
fn non_drawing_formats_report_render_diagnostics() {
    for (fmt, n) in [
        ("dot", 1),
        ("plain", 1),
        ("canon", 1),
        ("dot_json", 1),
        ("json", 2),
    ] {
        let (_, err, _) = run(&[&format!("-T{fmt}")], "digraph { a [style=striped] }");
        let warning = "Warning: gvrender_set_style: unsupported style striped - ignoring\n";
        assert_eq!(err, warning.repeat(n), "-T{fmt}");
    }
    let layers = "digraph{layers=\"a:b\"; a}";
    let (_, err, _) = run(&["-Tjson"], layers);
    assert_eq!(
        err,
        "Warning: layers not supported in xdot output\n\
         Warning: layers not supported in json output\n"
    );
    let (_, err, _) = run(&["-Tcanon"], layers);
    assert_eq!(err, "Warning: layers not supported in canon output\n");
}

// Issue #108: the layout runs for `canon` and the json formats without layout attributes.
#[test]
fn canon_and_dot_json_are_laid_out() {
    let src = "digraph G { subgraph cluster_a { x; y; x -> y } {rank=same; x; z} }";
    for fmt in ["-Tcanon", "-Tdot_json", "-Txdot_json"] {
        let (_, err, _) = run(&[fmt], src);
        assert_eq!(
            err, "Warning: x was already in a rankset, deleted from cluster G\n",
            "{fmt}"
        );
    }
    // patchwork's init sets the root default `shape=box`.
    let (out, _, _) = run(
        &["-Kpatchwork", "-Tcanon"],
        "digraph G { subgraph cluster_a {a subgraph cluster_c {c}} subgraph cluster_b {b} a->b }",
    );
    assert!(
        out.starts_with("digraph G {\n\tnode [label=\"\\N\",\n\t\tshape=box\n\t];\n"),
        "{out}"
    );
    // The label positions the layout computed are not the graph's attributes.
    let (out, _, _) = run(&["-Tcanon"], "graph { B -- A [taillabel=tail] }");
    assert_eq!(
        out,
        "graph {\n\tnode [label=\"\\N\"];\n\tB -- A\t[taillabel=tail];\n}\n"
    );
}

// What a job attaches stays in the graph for the jobs after it, as in C's cgraph.
#[test]
fn later_jobs_write_what_earlier_jobs_attached() {
    let src = "digraph { a [pos=\"1,2\"]; a -> b }";
    let (out, _, _) = run(&["-Tdot", "-Tcanon"], src);
    let (dot, canon) = out.split_at(out.len() / 2);
    assert_eq!(dot, canon);
    assert!(canon.contains("pos=\"27,90\""), "{canon}");
    let (out, _, _) = run(&["-Tcanon", "-Tdot"], src);
    assert!(
        out.starts_with("digraph {\n\tnode [label=\"\\N\"];\n\ta\t[pos=\"1,2\"];"),
        "{out}"
    );
}

// Issue #278: each graph is read, laid out and rendered before the next is read, and a
// syntax error is reported where the parser finds it.
#[test]
fn diagnostics_come_one_graph_at_a_time() {
    let (_, err, _) = run(
        &["-Tplain"],
        "digraph { a [style=bogus] }\ndigraph { 2x }\n",
    );
    assert_eq!(
        err,
        "Warning: gvrender_set_style: unsupported style bogus - ignoring\n\
         Warning: syntax ambiguity - badly delimited number '2x' in line 1 of <stdin> splits \
         into two tokens\n"
    );
    let (_, err, ok) = run(&["-Tcanon"], "\\\n.5.}");
    assert!(!ok);
    assert_eq!(
        err,
        "Error: <stdin>: syntax error in line 1 near '\\'\n\
         Warning: syntax ambiguity - badly delimited number '.5.' in line 2 of <stdin> splits \
         into two tokens\n"
    );
}

// Issue #126: `-y` sets `Y_invert`.
#[test]
fn y_flag_inverts_y_in_the_writers() {
    let (out, _, _) = run(&["-y", "-Tdot"], "digraph { a -> b }");
    assert!(out.contains("bb=\"0,108,54,0\""), "{out}");
    assert!(
        out.contains("pos=\"e,27,71.896 27,36.303 27,43.593 27,52.274 27,60.464\""),
        "{out}"
    );
    let (out, _, _) = run(&["-y", "-Tplain"], "digraph { a -> b }");
    assert_eq!(
        out,
        "graph 1 0.75 1.5\nnode a 0.375 0.25 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 0.375 1.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge a b 4 0.375 0.50421 0.375 0.60546 0.375 0.72602 0.375 0.83978 solid black\nstop\n"
    );
}

// Issue #190: args.c's option and error paths, and `gvNextInputGraph`'s exit status.
#[test]
fn args_c_option_and_error_paths() {
    let dir = scratch("c2_args");
    // `-O` with stdin names the output after `noname.gv`, a second graph with its number.
    let (out, _, code) = run_in(&dir, &["-Tplain", "-O"], "digraph{a}\ndigraph{b}");
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(dir.join("noname.gv.plain").exists() && dir.join("noname.gv.2.plain").exists());
    // `-` alone is an unknown option, its letter the terminating NUL.
    let (_, err, code) = run_in(&dir, &["-Tsvg", "-"], "");
    assert_eq!(code, 1);
    assert!(
        err.starts_with("Error: graphviz-pure: option -Usage: "),
        "{err}"
    );
    let (_, _, code) = run_in(&dir, &["-Tplain", "--filepath=/tmp"], "digraph{a}");
    assert_eq!(code, 0);
    // An input that cannot be opened counts in `graphviz_errors` and as an error.
    let (_, err, code) = run_in(&dir, &["-Tplain", "nosuch.gv"], "");
    assert_eq!(code, 2);
    // (`strerror`'s text is the OS's.)
    assert!(
        err.starts_with("Error: graphviz-pure: can't open nosuch.gv: "),
        "{err}"
    );
    #[cfg(unix)]
    assert_eq!(
        err,
        "Error: graphviz-pure: can't open nosuch.gv: No such file or directory\n"
    );
    let (_, err, code) = run_in(&dir, &["-Tfoo"], "");
    assert_eq!(code, 1);
    assert!(
        err.starts_with("Format: \"foo\" not recognized. Use one of: canon dot "),
        "{err}"
    );
    let (_, err, code) = run_in(&dir, &["-Kfoo"], "");
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "There is no layout engine support for \"foo\"\n Use one of: circo dot fdp neato nop \
         nop1 nop2 osage patchwork sfdp twopi\n"
    );
    // The n-th `-o` is the n-th `-T`'s file.
    let (out, _, code) = run_in(
        &dir,
        &["-Tplain", "-Tcanon", "-oa.plain", "-ob.gv"],
        "digraph{a}",
    );
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(
        std::fs::read_to_string(dir.join("a.plain"))
            .unwrap()
            .starts_with("graph 1")
    );
    assert!(
        std::fs::read_to_string(dir.join("b.gv"))
            .unwrap()
            .starts_with("digraph {")
    );
}

// Issues #263 and #282: `-L` sets fdp's parameters, and the `-L`, `-v`, `-O` and `-c` passes
// take their flags out before the other options read theirs.
#[test]
fn fdp_flags_and_extra_argument_passes() {
    let (out, err, ok) = run(&["-Kfdp", "-Tplain", "-Ln5"], "graph G { a -- b -- c }");
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "graph 1 2.3427 2.8003\nnode a 0.375 2.5503 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 1.9677 1.9392 0.75 0.5 b solid ellipse black lightgrey\n\
         node c 1.7296 0.25 0.75 0.5 c solid ellipse black lightgrey\n\
         edge a b 4 0.705 2.4237 0.97834 2.3188 1.3649 2.1705 1.6381 2.0656 solid black\n\
         edge b c 4 1.932 1.6863 1.8872 1.3678 1.8105 0.82387 1.7655 0.50461 solid black\nstop\n"
    );
    let (_, err, ok) = run(&["-Tplain", "-Lz"], "graph { a }");
    assert!(ok);
    assert_eq!(err, "Warning: unknown flag -Lz - ignored\n");
    let (out, err, ok) = run(&["-Tplain", "-Lnx"], "graph { a }");
    assert!(!ok && out.is_empty());
    assert!(
        err.starts_with("Error: bad value in flag -Lnx - ignored\nUsage: "),
        "{err}"
    );
    // `setDouble` names the flag from the `*` of `-LT*`.
    let (_, err, _) = run(&["-Tplain", "-LT*x"], "graph { a }");
    assert!(
        err.starts_with("Error: bad value in flag -L*x - ignored\n"),
        "{err}"
    );
    let dir = scratch("c2_passes");
    let (_, err, code) = run_in(&dir, &["-Tplain", "-o", "-Lg", "o.plain"], "graph { a }");
    assert_eq!(code, 0, "{err}");
    assert!(dir.join("o.plain").exists());
    let (out, _, code) = run_in(&dir, &["-T", "-v", "plain"], "graph { a }");
    assert_eq!(code, 0);
    assert!(out.starts_with("graph 1 "), "{out}");
}

// Issue #282: `-x` (`Reduce`) deletes singletons and trees from the root graph, which keeps
// the cgraph sequence numbers of the rest.
#[test]
fn reduce_flag_prunes_trees_like_c() {
    let src = "graph G { e -- a; a -- b -- c -- d -- a; b -- d }";
    let (out, _, _) = run(&["-Kneato", "-x", "-Tplain"], src);
    assert!(!out.contains("node e"), "{out}");
    assert_eq!(out.matches("\nnode ").count(), 4, "{out}");
    let (out, _, _) = run(&["-Kneato", "-x", "-Tsvg"], src);
    assert!(
        !out.contains("id=\"node1\"") && out.contains("id=\"node2\""),
        "{out}"
    );
    // Per component, after `pccomps`: a component Reduce empties is still packed.
    let src = "graph { a -- b -- c -- a; d; e -- f; g -- h -- i -- g; i -- j -- k }";
    let (out, _, _) = run(&["-Kneato", "-x", "-Tplain"], src);
    assert!(out.starts_with("graph 1 3.3779 2.2418\n"), "{out}");
    // `user_pos` ran on the whole graph first: the third coordinates are drawn as there.
    let src = "graph G { dim=3; a -- b; c -- d -- e -- c; f; c [pos=\"5,5\"] }";
    let (out, _, _) = run(&["-Kneato", "-x", "-Tdot"], src);
    assert!(out.contains("bb=\"0,0,108.66,80.61\""), "{out}");
}

// Issue #294: `-s` sets `PSinputscale`, which `get_inputscale` prefers to `inputscale`.
#[test]
fn s_flag_sets_the_input_scale() {
    let src = "graph { layout=neato; a [pos=\"100,100!\"]; b [pos=\"200,100!\"]; a -- b }";
    let (out, _, _) = run(&["-s36", "-Tdot"], src);
    assert!(out.contains("bb=\"0,0,254,36\""), "{out}");
}

// Issue #264, and `gvLayoutJobs`: a `layout` attribute replaces the engine for its graph and
// the later ones; one that names no engine leaves none, and the graph is not laid out.
#[test]
fn layout_attribute_selects_the_engine_as_gvlayoutjobs() {
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph G { subgraph s { layout=neato; a } a -> b }",
    );
    assert!(!ok && out.is_empty());
    assert_eq!(
        err,
        "Warning: layout attribute is invalid except on the root graph\n\
         Error: Layout type: \"\" not recognized. Use one of: circo dot fdp neato nop nop1 \
         nop2 osage patchwork sfdp twopi\n\
         Error: Layout was not done.  Missing layout plugins? \n"
    );
    let (out, err, ok) = run(&["-Tplain"], "digraph A {layout=foo; a}\ndigraph B {b}\n");
    assert!(!ok && out.is_empty());
    assert!(
        err.ends_with(
            "Missing layout plugins? \nError: Layout was not done.  Missing layout plugins? \n"
        ),
        "{err}"
    );
    let (out, _, _) = run(
        &["-Tplain"],
        "digraph A {layout=neato; a->b}\ndigraph B {c->d}\n",
    );
    assert_eq!(out.matches("graph 1 1.739 0.64777").count(), 2, "{out}");
}

// `init_nop` fails on a node with no position, and `dot` renders what it left: every node
// at the origin, no edge.
#[test]
fn nop_without_positions_still_renders() {
    let (out, err, ok) = run(
        &["-Kneato", "-n", "-Tplain"],
        "digraph G { a [pos=\"1,1\"]; a -> b }",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node b in graph G has no position\nas required by the -n flag\n"
    );
    assert_eq!(
        out,
        "graph 1 0 0\nnode a 0 0 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 0 0 0.75 0.5 b solid ellipse black lightgrey\n solid black\nstop\n"
    );
}

// Issue #362: C's jobs share one cgraph, so the second xdot job reads the `xdotversion` the
// first one set.
#[test]
fn xdot_after_xdot1_2_writes_version_1_2() {
    let (out, _, ok) = run(&["-Txdot1.2", "-Txdot"], "digraph { a }");
    assert!(ok);
    assert_eq!(out.matches("xdotversion=1.2").count(), 2, "{out}");
}

// Issue #367: a later job writes the `_draw_` an earlier xdot or json job set, on the root and
// on every cluster, also when the input declares `_draw_`.
#[test]
fn a_later_job_writes_the_draw_an_earlier_job_set() {
    let src = "digraph G { graph [bb=\"\", lp=\"\", _draw_=\"\", label=\"\"]; \
               subgraph cluster_a { label=A; a -> b; subgraph cluster_n { label=N; n } } \
               subgraph s { e } subgraph cluster_c { c } b -> c; d }";
    let (xdot, _, _) = run(&["-Txdot"], src);
    for first in ["-Txdot", "-Tjson"] {
        for later in ["-Tdot", "-Tcanon"] {
            let (out, _, ok) = run(&[first, later], src);
            assert!(ok);
            let start = out.rfind("digraph G {").unwrap();
            let last = &out[start..];
            assert!(!last.contains("_draw_=\"\""), "{first} {later}: {last}");
            assert_eq!(
                last.matches("_draw_=").count(),
                xdot.matches("_draw_=").count(),
                "{first} {later}: {last}"
            );
        }
    }
}

// Issue #108: `-Tcanon` lays out, so it reports what the layout reports and fails when the
// layout does.
#[test]
fn canon_reports_layout_warnings_and_failures() {
    let (_, err, ok) = run(
        &["-Tcanon"],
        "digraph G { pack=true; newrank=true; subgraph cluster_a { a -> b; } b -> c; }",
    );
    assert!(ok);
    assert_eq!(err, "Warning: removing empty cluster\n");
    let (out, err, ok) = run(
        &["-Tcanon"],
        "digraph { subgraph cluster_0 { a b } subgraph cluster_1 { b c } a -> c; \
         {rank=min; b} {rank=max; a} }",
    );
    assert!(!ok);
    assert!(err.contains("Error: trouble in init_rank\n"), "{err}");
    assert!(out.contains("\tsubgraph cluster_0 {\n\t}\n"), "{out}");
}

#[test]
fn undirected_graph_has_no_arrowhead() {
    let (out, _, ok) = run(&["-Tsvg"], "graph{a--b}");
    assert!(ok);
    assert_eq!(out.matches("<polygon").count(), 1, "only the background");
    let (out, _, _) = run(&["-Tsvg"], "digraph{a->b}");
    assert_eq!(out.matches("<polygon").count(), 2);
}

#[test]
fn unknown_flag_is_rejected() {
    let (_, err, ok) = run(&["-Z"], "graph{}");
    assert!(!ok);
    assert!(err.contains("option -Z unrecognized"), "{err}");
}

// Issue #165: every engine's `is_a_cluster` sliced the subgraph name at byte 7, which panics
// ("byte index 7 is not a char boundary") when byte 7 falls inside a multi-byte character.
// C's `strncasecmp(agnameof(g), "cluster", 7)` compares bytes, and C draws these graphs.
// "CLUSTERé" is still a cluster, as in C.
#[test]
fn subgraph_name_with_multibyte_char_at_byte_7_does_not_panic() {
    let src = "graph { subgraph \"aaaaaaé\" { a [pos=\"0,0\"] } \
               subgraph \"CLUSTERé\" { b [pos=\"1,1\"] } }\n";
    let engines = [
        "dot",
        "neato",
        "fdp",
        "sfdp",
        "twopi",
        "circo",
        "osage",
        "patchwork",
        "nop",
    ];
    for engine in engines {
        for fmt in ["-Tsvg", "-Tjson"] {
            let (out, err, ok) = run(&[&format!("-K{engine}"), fmt, "-Gpack=true"], src);
            assert!(ok && !err.contains("panicked"), "{engine} {fmt}: {err}");
            // The engines that draw this cluster in C (circo, twopi and sfdp ignore clusters;
            // nop has no cluster `bb` to draw).
            let draws = ["dot", "neato", "fdp", "osage", "patchwork"].contains(&engine);
            if fmt == "-Tsvg" && draws {
                assert_eq!(
                    out.matches("class=\"cluster\"").count(),
                    1,
                    "{engine}: {out}"
                );
            }
        }
    }
}

#[test]
fn global_attrs_output_file_and_separate_args() {
    let path = tmp("cli_out.svg");
    let (out, _, ok) = run(
        &[
            "-T",
            "svg",
            "-o",
            path.to_str().unwrap(),
            "-Nshape=box",
            "-Gbgcolor=yellow",
            "-Ecolor=red",
        ],
        "digraph G{a->b}\ndigraph H{c->d}",
    );
    assert!(ok);
    // C closes `-o`'s file after the first graph (`gvdevice_close` forgets its name), and
    // writes the second graph to stdout.
    let svg = std::fs::read_to_string(&path).unwrap();
    assert_eq!(svg.matches("<svg").count(), 1, "{svg}");
    assert!(svg.contains("<title>a</title>"));
    assert_eq!(out.matches("<svg").count(), 1, "{out}");
    assert!(out.contains("<title>c</title>"));
    assert!(!svg.contains("<ellipse"));
    assert!(svg.contains("fill=\"yellow\""));
    assert!(svg.contains("stroke=\"red\""));
}

#[test]
fn multiple_input_files_are_each_rendered() {
    let a = tmp("cli_a.gv");
    let b = tmp("cli_b.gv");
    std::fs::write(&a, "digraph{x->y}").unwrap();
    std::fs::write(&b, "graph{p--q}").unwrap();
    let (out, _, ok) = run(&["-Tsvg", a.to_str().unwrap(), b.to_str().unwrap()], "");
    assert!(ok);
    assert_eq!(out.matches("<svg").count(), 2);
}

#[test]
fn version_and_help_exit_successfully() {
    let (_, err, ok) = run(&["-V"], "");
    let version = concat!("graphviz-pure version ", env!("CARGO_PKG_VERSION"), " ");
    assert!(ok && err.contains(version), "{err}");
    let (out, _, ok) = run(&["-?"], "");
    assert!(ok && out.contains("Usage:"));
}

#[test]
fn no_layout_mode_keeps_positions() {
    let laid = "digraph{a [pos=\"127,118\"]; b [pos=\"127,18\"]; a -> b [pos=\"e,127,36 127,100 127,80 127,60 127,46\"]}";
    let (out, _, ok) = run(&["-Kneato", "-n2", "-Tdot"], laid);
    assert!(ok);
    // translated so the bounding box starts at the origin
    assert!(out.contains("pos=\"27,118\""), "{out}");
    assert!(
        out.contains("pos=\"e,27,36 27,100 27,80 27,60 27,46\""),
        "{out}"
    );
    let (_, err, ok) = run(&["-Kneato", "-n0", "-Tdot"], laid);
    assert!(!ok && err.contains("Invalid parameter"));
    let (out, _, ok) = run(&["-Knop2", "-Tplain"], laid);
    assert!(ok && out.starts_with("graph 1 "), "{out}");
}

#[test]
fn png_and_jpeg_output_are_images_at_graphviz_sizes() {
    let png = tmp("raster.png");
    let (_, err, ok) = run(
        &["-Tpng", "-o", png.to_str().unwrap()],
        "digraph { a -> b }",
    );
    assert!(ok, "{err}");
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    // 62 x 116 pt at 96 dpi, as C's -Tpng.
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 83);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 155);
    let jpg = tmp("raster.jpg");
    let (_, err, ok) = run(
        &["-Tjpg", "-o", jpg.to_str().unwrap()],
        "digraph { a -> b }",
    );
    assert!(ok, "{err}");
    assert_eq!(&std::fs::read(&jpg).unwrap()[..2], &[0xFF, 0xD8]);
}

// Issue #166: getAdjustMode (lib/neatogen/adjust.c) matches the "prism" prefix with
// strncasecmp, a byte comparison. The port sliced the `str` at byte 5 and panicked when that
// byte fell inside a multi-byte character. C's stderr is Graphviz 16.1.0's.
#[test]
fn overlap_prism_prefix_with_multibyte_char_does_not_panic() {
    for engine in ["neato", "fdp", "sfdp", "twopi", "circo"] {
        let (err, ok) = warnings(engine, "graph { overlap=\"prisé\"; a -- b }");
        assert!(ok, "{engine}: {err}");
        assert_eq!(
            err, "Warning: Unrecognized overlap value \"prisé\" - using false\n",
            "{engine}"
        );
        // A real "prism" prefix still matches whatever follows it.
        let (err, ok) = warnings(engine, "graph { overlap=\"PRISMé\"; a -- b }");
        assert!(ok, "{engine}: {err}");
        assert_eq!(err, "", "{engine}");
    }
}

// Issues #81, #98, #99, #119: cgraph's scanner gives an HTML string its inner text and a flag
// (scan.l `endstr_html`, `agstrdup_html`); `concat` and `concatPort` make plain strings and the
// writers test the flag (write.c `agstrcanon`). Expected output is Graphviz 16.1.0's.
#[test]
fn html_strings_keep_their_inner_text_and_a_flag() {
    let dot = |src: &str| {
        let (out, err, ok) = run(&["-Tdot"], src);
        assert!(ok, "{err}");
        (out, err)
    };
    // #81: `+` joins the inner texts into a plain string.
    let (out, _) = dot("digraph { a [label=\"x\" + <b>]; c [label=<b> + \"y\"] }");
    assert_eq!(
        out,
        "digraph {\n\tgraph [bb=\"0,0,126,36\"];\n\tnode [label=\"\\N\"];\n\
         \ta\t[height=0.5,\n\t\tlabel=xb,\n\t\tpos=\"27,18\",\n\t\twidth=0.75];\n\
         \tc\t[height=0.5,\n\t\tlabel=by,\n\t\tpos=\"99,18\",\n\t\twidth=0.75];\n}\n"
    );
    // #98: `port:compass` is a plain string; the port name is the inner text.
    let (out, err) = dot("digraph G { a:<foo>:n -> b }");
    assert_eq!(err, "Warning: node a, port foo unrecognized\n");
    assert!(out.contains("\ta:foo:n -> b\t[pos="), "{out}");
    // #99: an attribute is named by the inner text, which orders it.
    let (out, _) = dot("digraph G { a [<zzz>=1] }");
    assert!(
        out.contains("\ta\t[height=0.5,\n\t\tpos=\"27,18\",\n\t\twidth=0.75,\n\t\t<zzz>=1];"),
        "{out}"
    );
    // #119: a node named `<b>` is not `b` nor `"<b>"`, and is written `<b>`.
    let (out, _, ok) = run(&["-Tcanon"], "digraph { <b>; b; \"<b>\" }");
    assert!(ok);
    assert_eq!(
        out,
        "digraph {\n\tnode [label=\"\\N\"];\n\t<b>;\n\tb;\n\t\"<b>\";\n}\n"
    );
}

// Same cause, other routes: every reader but the label makers sees an HTML value's inner text
// (`shape=<box>`), an empty HTML label is plain (`make_label`), a record field is found by an
// HTML port's inner text, and `<x>` and `"<x>"` stay apart in one graph.
#[test]
fn html_values_are_their_inner_text_to_other_readers() {
    let (out, err, ok) = run(&["-Tdot"], "digraph { a [shape=<box>, label=<>] }");
    assert!(ok);
    assert_eq!(err, "");
    assert!(
        out.contains("\t\tlabel=<>,\n") && out.contains("\t\tshape=<box>,\n"),
        "{out}"
    );
    let (out, err, ok) = run(
        &["-Tdot"],
        "digraph { a [shape=record, label=\"<f0> x|<f1> y\"]; a:<f0> -> b }",
    );
    assert!(ok);
    assert_eq!(err, "");
    assert!(out.contains("graph [bb=\"0,0,67,109\"];"), "{out}");
    assert!(out.contains("\ta:<f0> -> b\t[pos="), "{out}");
    let (out, _, ok) = run(&["-Tdot"], "digraph { a [label=<x>, xlabel=\"<x>\"] }");
    assert!(ok);
    assert!(out.contains("graph [bb=\"0,0,76.791,52.8\"];"), "{out}");
    assert!(out.contains("\t\tlabel=<x>,\n") && out.contains("\t\txlabel=\"<x>\",\n"));
    // A value the layout computes is plain, whatever the input wrote (`agxset`).
    let (out, _, ok) = run(&["-Tdot"], "digraph { graph [bb=<0,0,1,1>]; a }");
    assert!(ok);
    assert!(out.contains("\tgraph [bb=\"0,0,54,36\"];\n"), "{out}");
    // ... even when it has the input's text.
    let (out, _, ok) = run(&["-Tdot"], "digraph { a [width=<0.75>, height=<0.5>] }");
    assert!(ok);
    assert!(
        out.contains("\ta\t[height=0.5,\n\t\tpos=\"27,18\",\n\t\twidth=0.75];"),
        "{out}"
    );
    // `agfindnode` tells `a` from `<a>`: twopi roots this graph at the plain `a`.
    let (out, _, ok) = run(
        &["-Ktwopi", "-Tdot"],
        "graph { <a> -- b; a -- c; c -- d; d -- e; root=a }",
    );
    assert!(ok);
    assert!(
        out.contains("\tgraph [bb=\"0,0,324,113\",\n\t\troot=a\n"),
        "{out}"
    );
    // `-Tplain-ext` writes names and ports with `agcanonStr` (output.c `writenodeandport`).
    let (out, _, ok) = run(
        &["-Tplain-ext"],
        "digraph { a [shape=record, label=\"<p> x|y\"]; a:<p> -> <h> }",
    );
    assert!(ok);
    assert!(out.contains("\nnode <h> 0.375 0.25 "), "{out}");
    assert!(out.contains("\nedge a:<p> <h> 4 "), "{out}");
    // Values the layouts write are plain: dot's phase rank/order, neato's z, patchwork's
    // shape. xdot sets back the input's own `xdotversion` string, flag included.
    // (arguments, input, expected in the output, not expected)
    type Case<'a> = (&'a [&'a str], &'a str, &'a [&'a str], &'a [&'a str]);
    let cases: [Case; 4] = [
        (
            &["-Tdot"],
            "digraph { phase=2; a [order=<0>, rank=<0>] }",
            &["\t\torder=0,\n", "\t\trank=0,\n"],
            &["<0>"],
        ),
        (
            &["-Kneato", "-Tdot"],
            "graph { dim=3; a [pos=\"1,2,3!\", z=<216.000000>] }",
            &["\t\tz=216.000000];"],
            &["z=<"],
        ),
        (
            &["-Kpatchwork", "-Tdot"],
            "graph { node [shape=<box>]; a [shape=<box>] }",
            &["\t\tshape=box\n\t];\n"],
            &["<box>", "\t\tshape=box,"],
        ),
        (
            &["-Txdot"],
            "digraph { xdotversion=<1.7>; a }",
            &["\t\txdotversion=<1.7>\n"],
            &[],
        ),
    ];
    for (args, src, want, not) in cases {
        let (out, err, ok) = run(args, src);
        assert!(ok, "{src}: {err}");
        for w in want {
            assert!(out.contains(w), "{src}: {out}");
        }
        for n in not {
            assert!(!out.contains(n), "{src}: {out}");
        }
    }
    // dot's `group` compares interned strings: `<g>` is not the group `g`.
    let (out, _, ok) = run(
        &["-Tplain"],
        "digraph { n0->n2; n0->n3; n0->n6; n1->n3; n1->n4; n3->n4; n3->n5; n3->n6; \
         n6 [group=<g>]; n0 [group=g]; n3 [group=g] }",
    );
    assert!(ok);
    assert!(out.contains("\nnode n0 1.9028 2.25 "), "{out}");
    // A viewport focus name is plain, so it does not find the node `<a>`.
    let (out, _, ok) = run(
        &["-Tsvg"],
        "digraph { viewport=\"100,100,1,'a'\"; <a> -> b -> c -> d }",
    );
    assert!(ok);
    assert!(out.contains("translate(23 176)"), "{out}");
}

// Issue #320: `-Tplain` writes a label with `canon` (output.c), which interns the text in the
// root graph and frees it again. A text that needs no quotes comes back as the interned string,
// so unless the graph held it as a plain string already, C prints freed memory: a byte that
// changes from run to run, or nothing. The port writes the empty field.
#[test]
fn plain_label_field_is_empty_where_c_reads_freed_memory() {
    let edge = |src: &str| {
        let (out, err, ok) = run(&["-Tplain"], src);
        assert!(ok, "{src}: {err}");
        out.lines()
            .find(|l| l.starts_with("edge "))
            .map(str::to_string)
            .unwrap_or_default()
    };
    // an HTML edge label: its text is no plain string of the graph
    assert!(
        edge("digraph { c -> e [label=<L>] }")
            .ends_with(" 0.375 0.66136  0.43439 0.86667 solid black"),
        "{}",
        edge("digraph { c -> e [label=<L>] }")
    );
    // the node `L` keeps the plain string alive
    assert!(
        edge("digraph { L; c -> e [label=<L>] }")
            .ends_with(" 0.66136 L 1.4344 0.86667 solid black")
    );
    // not only HTML labels: `\H\T` makes the text `ec`
    assert!(edge(r#"digraph { c -> e [label="\H\T"] }"#).contains(" 0.66136  0.4613 "));
    // an attribute name is in the dictionary too
    assert!(edge("digraph { c -> e [label=<label>] }").contains(" label "));
    // `-Tdot` before `-Tplain` stores the layout's values (`lp="0.5,..."`, `height=0.5`)
    let (out, _, ok) = run(&["-Tdot", "-Tplain"], "digraph { c -> e [label=<0.5>] }");
    assert!(ok);
    assert!(
        out.contains(" 0.66136 0.5 0.49653 0.86667 solid black"),
        "{out}"
    );
    // fdp's cluster nodes are gone by then: an edge end at `<cluster_x>` is freed memory too
    let (out, _, ok) = run(
        &["-Kfdp", "-Tplain"],
        "graph { subgraph <cluster_x> { a } cluster_x -- b [label=<cluster_x>] }",
    );
    assert!(ok);
    assert!(out.contains("\nedge  b 4 "), "{out}");
    // a node named `<b>` has the plain text `b` as its default label
    let (out, _, ok) = run(&["-Tplain"], "digraph { <b> }");
    assert!(ok);
    assert!(
        out.contains("\nnode <b> 0.375 0.25 0.75 0.5  solid ellipse black lightgrey\n"),
        "{out}"
    );
}

// Issue #305: circo names its derived nodes in a root graph of its own (`makeDerivedNode`),
// whose dictionary has no HTML strings, so `<G>` and `G` are one derived node there. The last of
// them gets the position; `G` keeps its initial one, and its out-edge `G -- a` is not in the
// component, so `a` is laid out alone, as in C 16.1.0.
#[test]
fn circo_merges_html_and_plain_names_in_its_derived_graph() {
    let (out, err, ok) = run(&["-Kcirco", "-Tplain"], "graph { G -- a; <G> -- b }");
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "graph 1 2.5 0.5\n\
         node G 1.25 0.25 0.75 0.5 G solid ellipse black lightgrey\n\
         node a 1.25 0.25 0.75 0.5 a solid ellipse black lightgrey\n\
         node <G> 0.375 0.25 0.75 0.5 G solid ellipse black lightgrey\n\
         node b 2.125 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge G a 4 1.25 0.25 1.25 0.25 1.25 0.25 1.25 0.25 solid black\n\
         edge <G> b 4 0.75472 0.25 1.0483 0.25 1.4527 0.25 1.746 0.25 solid black\n\
         stop\n"
    );
    // with a second component, the merged node is packed with its ORIGN's size
    let (out, _, ok) = run(
        &["-Kcirco", "-Tplain"],
        "graph { G -- a; <G> -- b; c -- d }",
    );
    assert!(ok);
    assert!(
        out.starts_with("graph 1 4.6389 1.5694\nnode G 1.375 0.30556 "),
        "{out}"
    );
    assert!(out.contains("\nnode <G> 0.375 0.25 ") && out.contains("\nnode d 4.2639 1.3194 "));
}

// Issue #305: fdp's derived graph merges nodes of one name too (`mkDeriveNode`), and C then fails
// `bitarray_get`'s assertion in findCComp. The port lays the nodes out apart.
#[test]
fn fdp_derived_name_collisions_do_not_panic() {
    for src in [
        "graph { G -- a; <G> -- b }",
        "digraph G { subgraph cluster_a { x } subgraph s { subgraph cluster_a { y } } x -> y; }",
        "graph { subgraph cluster_x { a } cluster_x }",
    ] {
        let (out, err, ok) = run(&["-Kfdp", "-Tdot"], src);
        assert!(ok, "{src}: {err}");
        assert!(out.contains("pos="), "{src}: {out}");
    }
}

// fdp's cluster map compares names as text (`mkClustMap`), so a node `<cluster_x>` is an edge
// end at the cluster `cluster_x`; `mapN` then makes the plain node `cluster_x` for it.
#[test]
fn fdp_html_named_cluster_node_comes_back_plain() {
    let (out, err, ok) = run(
        &["-Kfdp", "-Tdot"],
        "graph { subgraph cluster_x { a } G -- <cluster_x> }",
    );
    assert!(ok, "{err}");
    assert!(out.contains("\tG -- cluster_x\t[pos="), "{out}");
}

// Nodes `G` and `<G>` are two objects for the fonts their labels interned: `<G>` does not get
// the sizes `G` recorded. The same holds for graphs.
#[test]
fn html_and_plain_named_nodes_keep_their_own_label_fonts() {
    let (out, err, ok) = run(&["-Tsvg"], "graph { a; G [fontsize=9]; <G> }");
    assert!(ok, "{err}");
    assert!(out.contains("x=\"99\" y=\"-15.3\" font-family=\"Times,serif\" font-size=\"9.00\">G<"));
    assert!(
        out.contains("x=\"171\" y=\"-13.8\" font-family=\"Times,serif\" font-size=\"14.00\">G<")
    );
    // likewise a cluster `<cluster_x>` and a cluster `cluster_x`
    let (out, err, ok) = run(
        &["-Tsvg"],
        "digraph { subgraph cluster_x { label=X; fontsize=9; a } \
         subgraph <cluster_x> { label=Y; b } }",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("y=\"-68.2\" font-family=\"Times,serif\" font-size=\"14.00\">Y<"),
        "{out}"
    );
}

// Issue #166, same root cause: neato's ipsep mode matches diredgeconstraints="hier..." with
// strncasecmp(str, "hier", 4) (lib/neatogen/neatoinit.c, majorization).
#[test]
fn diredgeconstraints_hier_prefix_with_multibyte_char_does_not_panic() {
    let src = "digraph { mode=ipsep; diredgeconstraints=\"hieé\"; a -> b; b -> c }";
    let (err, ok) = warnings("neato", src);
    assert!(ok, "{err}");
    assert_eq!(err, "");
}

// Issue #196: at 1e17 `(int)floor(x / Cell)` saturates to INT_MAX (INT_MIN at -1e17), and
// gridRepulse's `doNeighbor(grid, i + 1, ...)` (lib/fdpgen/tlayout.c) wraps under clang where a
// plain Rust `+` panics in debug builds. The expected output is C Graphviz 16.1.0's.
#[test]
fn fdp_nodes_pinned_near_1e17_lay_out_like_c() {
    let cases = [
        (
            "graph { layout=fdp; a [pos=\"1e17,1e17!\"]; b [pos=\"1e17,1.0000001e17!\"] }",
            "graph 1 1e+17 1e+17\n\
             node a 1e+17 1e+17 0.75 0.5 a solid ellipse black lightgrey\n\
             node b 1e+17 1e+17 0.75 0.5 b solid ellipse black lightgrey\n\
             stop\n",
        ),
        (
            "graph { layout=fdp; a [pos=\"-1e17,-1e17!\"]; b [pos=\"-1e17,-1.0000001e17!\"] }",
            "graph 1 1e+17 1e+17\n\
             node a 0 1e+10 0.75 0.5 a solid ellipse black lightgrey\n\
             node b 0 0 0.75 0.5 b solid ellipse black lightgrey\n\
             stop\n",
        ),
    ];
    for (src, expected) in cases {
        let (out, err, ok) = run(&["-Tplain"], src);
        assert!(ok, "{src}: {err}");
        assert_eq!(out, expected, "{src}");
    }
}

// Issue #110: attribute names the port once reserved for layout results (`\x01…`) are
// ordinary attributes, as they are to C: they change no drawing and every writer prints them.
#[test]
fn control_character_attribute_names_are_ordinary_attributes() {
    let src = "digraph G { \"\u{1}unregclust\"=\"1\"; subgraph cluster_0 { a b } a -> b }\n";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(svg_cluster_titles(&out), ["cluster_0"]);
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("subgraph cluster_0 {\n\t\tgraph [bb=\"8,8,78,132\"];"),
        "{out}"
    );
    let (out, err, ok) = run(&["-Tjson"], "digraph G { \"\u{1}seq\"=\"x\"; a }\n");
    assert!(ok, "{err}");
    assert!(out.contains("\"\u{1}seq\": \"x\","), "{out}");
    let (out, err, ok) = run(&["-Tdot"], "digraph G { a [\"\u{1}lwrw\"=\"1,100\"] }\n");
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "a\t[\"\u{1}lwrw\"=\"1,100\",\n\t\theight=0.5,\n\t\tpos=\"27,18\",\n\t\twidth=0.75];"
        ),
        "{out}"
    );
}

// Issue #124: label positions in the input are not layout results. An edge with no spline,
// no main label and an unplaced xlabel is not emitted (`edge_in_box`), and an xlabel
// `placeLabels` left unplaced is not drawn (`emit_edge_label` tests `lp->set`).
#[test]
fn input_label_positions_are_not_placed_labels() {
    let src =
        "digraph { splines=none; a -> b [xlabel=X xlp=\"1,2\" headlabel=H head_lp=\"3,4\"]; }\n";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(!out.contains("class=\"edge\""), "{out}");
    let src = "digraph G { forcelabels=false; splines=\"ortho\"; concentrate=\"true\"; \
        nodesep=0.05; ranksep=0.05; \
        node [xlabel=\"NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN\"]; \
        \"8\" -> \"9\" [xlabel=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\" xlp=\"11,11\"]; \
        \"8\" -> \"13\"; \"9\" -> \"14\"; \"14\" -> \"13\"; \
        \"13\" -> \"14\" [xlabel=\"EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE\" xlp=\"3,4\"]; }\n";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: 4 out of 6 exterior labels positioned.\n");
    assert!(!out.contains("AAAA") && !out.contains("EEEE"), "{out}");
}

// Issue #129: `attach_attrs_and_arrows` writes the polygon of every `poly_init` node when the
// graph declares `vertices`: its vertices in inches, or `samplepoints` points on an ellipse.
#[test]
fn declared_vertices_are_written_for_polygon_nodes() {
    let src = "graph G { node [vertices=\"\"]; a [shape=box]; b [shape=hexagon]; \
        c [shape=ellipse]; f [shape=point]; a -- b -- c -- f; }\n";
    for engine in ["-Kneato", "-Kdot"] {
        let (out, err, ok) = run(&[engine, "-Tdot"], src);
        assert!(ok, "{err}");
        for v in [
            "vertices=\"0.375 0.25 -0.375 0.25 -0.375 -0.25 0.375 -0.25\"",
            "vertices=\"0.375 -5.9805e-17 0.1875 0.25 -0.1875 0.25 -0.375 3.5353e-17 \
             -0.1875 -0.25 0.1875 -0.25\"",
        ] {
            assert!(out.contains(v), "{engine}: {v} in {out}");
        }
        // the point keeps the declared default
        assert_eq!(out.matches("vertices=").count(), 4, "{engine}: {out}");
    }
    // an ellipse: `samplepoints` (8) points on it
    let (out, err, ok) = run(&["-Kneato", "-Tdot"], src);
    assert!(ok, "{err}");
    let ellipse = "vertices=\"0.375 0 0.26517 0.17678 2.2962e-17 0.25 -0.26517 0.17678 -0.375 \
        3.0616e-17 -0.26517 -0.17678 -6.8886e-17 -0.25 0.26517 -0.17678\"";
    assert!(out.contains(ellipse), "{out}");
}

// Issue #328: with `dimen=3` neato and sfdp write each node's `pos` as `x,y,z`
// (`GD_odim(g) >= 3` in `attach_attrs_and_arrows`).
#[test]
fn dimen_3_writes_three_coordinates() {
    let (out, err, ok) = run(
        &["-Kneato", "-n", "-Tdot"],
        "graph G { dimen=3; a [pos=\"1,2\"]; b [pos=\"50,60\" z=7]; a -- b }\n",
    );
    assert!(ok, "{err}");
    assert!(out.contains("pos=\"27,18,57.091\""), "{out}");
    assert!(out.contains("pos=\"76,76,7\""), "{out}");
    // A node without a position stops `-n` after `neato_init_graph` set GD_odim: the nodes
    // are drawn at the origin, each with the coordinates `user_pos` gave it after `y`.
    let (out, err, ok) = run(
        &["-Kneato", "-n", "-Tdot"],
        "graph { dimen=3; a [pos=\"1,2,3\"]; b [pos=\"5,5\"]; c }\n",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node c in graph %1 has no position\nas required by the -n flag\n"
    );
    for want in ["pos=\"0,0,3\"", "pos=\"0,0,85.636\"", "pos=\"0,0,0\""] {
        assert!(out.contains(want), "{want} in {out}");
    }
    for (engine, want) in [
        ("-Kneato", "pos=\"27,18,33.679\""),
        ("-Ksfdp", "pos=\"27,18,93.337\""),
    ] {
        let (out, err, ok) = run(&[engine, "-Tdot"], "graph G { dimen=3; a -- b }\n");
        assert!(ok, "{err}");
        assert!(out.contains(want), "{engine}: {out}");
    }
}

// Issue #329: `rec_attach_bb` sets a registered cluster's `bb` with `agxset`, which makes it
// the cluster's own value even where it equals the one it inherited, so `-Tdot` prints it.
#[test]
fn a_cluster_keeps_the_bb_attach_attrs_set_on_it() {
    let src = "graph G { graph [bb=\"0,0,100,100\"]; subgraph clusterB { d [pos=\"40,40\"] } \
        x [pos=\"90,90\"] }\n";
    let (out, err, ok) = run(&["-Kneato", "-n", "-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("subgraph clusterB {\n\t\tgraph [bb=\"0,0,100,100\"];"),
        "{out}"
    );
    // The value stays the cluster's own for the jobs after it.
    let (out, err, ok) = run(&["-Kneato", "-n", "-Tdot", "-Tcanon"], src);
    assert!(ok, "{err}");
    assert_eq!(
        out.matches("graph [bb=\"0,0,100,100\"];").count(),
        2,
        "{out}"
    );
}

// Extreme coordinates (docs/porting/crashes.md): C's `sscanf("%lf")` reads `nan` and `inf` in
// `pos`, and a finite `pos` can overflow once scaled to points. Where C produces output (often
// a component at NaN) the port matches it; where C crashes or hangs, the port must not. These
// are the routes by engine; none may panic or hang.
#[test]
fn non_finite_and_huge_positions_do_not_panic_in_any_engine() {
    let engines = [
        "dot",
        "neato",
        "fdp",
        "sfdp",
        "circo",
        "twopi",
        "osage",
        "patchwork",
    ];
    // 1e308 inches overflows to infinity once multiplied by 72.
    for v in ["nan", "inf", "-inf", "1e308"] {
        for engine in engines {
            for splines in ["spline", "curved", "ortho"] {
                let src = format!(
                    "graph {{ layout={engine}; splines={splines}; pack=false; \
                     a [pos=\"0,0!\"]; x [pos=\"{v},0!\"]; a -- x; x -- y [label=L]; \
                     subgraph cluster_c {{ label=C; c [pos=\"5,{v}\"] }} c -- a }}"
                );
                let (out, err, _) = run(&["-Tsvg"], &src);
                assert!(!err.contains("panicked"), "{src}: {err}");
                assert!(out.contains("</svg>"), "{src}: {err}");
            }
        }
    }
    for (flag, v) in [
        ("-n", "nan"),
        ("-n2", "nan"),
        ("-n2", "inf"),
        ("-n2", "1e400"),
    ] {
        let src = format!(
            "digraph {{ a [pos=\"{v},0\"]; b [pos=\"30,30\" xlabel=Q xlp=\"{v},0\"]; \
             a -> b [pos=\"0,0 {v},10 20,20 30,30\" label=L lp=\"{v},3\"]; \
             subgraph cluster_z {{ bb=\"0,0,{v},9\"; z [pos=\"4,4\"] }} }}"
        );
        let (out, err, ok) = run(&["-Kneato", flag, "-Tsvg"], &src);
        assert!(ok, "{flag} {src}: {err}");
        assert!(out.contains("</svg>"), "{flag} {src}: {err}");
    }
}

// Issue #228: neato splines=ortho with a NaN or infinite pin. C fails an assertion in
// ortho's list.c (exit 134); the port panicked in `convert_sp_to_route` until ortho got a
// failure boundary (#209, #248). The NaN component is drawn as C draws one with any other
// `splines`: its nodes are at NaN and the SVG leaves them out; `b` is drawn. Here the input
// side: the position reaches ortho as C reads it.
#[test]
fn ortho_with_a_non_finite_pin_does_not_panic() {
    for v in ["nan", "inf"] {
        let src = format!(
            "graph {{ layout=neato; pack=false; splines=ortho; a [pos=\"{v},0!\"]; \
             x [pos=\"1,0!\"]; a -- x; b }}"
        );
        let (out, err, ok) = run(&["-Tsvg"], &src);
        assert!(ok, "{src}: {err}");
        assert_eq!(err, "", "{src}");
        assert!(out.contains("<title>b</title>"), "{src}: {out}");
    }
}

// Issue #227: curved edges between pins 1e16 inches apart. `update_bb_bz` refines each
// segment until its control points are within 2pt of its chord, which the coordinates there
// cannot resolve, and C never finishes. The port stops a segment after 2^28 splits (about 5 s
// of C) and takes the rest whole; the graph bb still contains the curves. (neato's
// `compute_bb` rebuilds the bb afterwards; fdp keeps `update_bb_bz`'s.)
#[test]
fn curved_edges_between_far_pins_finish() {
    for (engine, bb) in [
        ("neato", "0,0,8.6241e+17,8.64e+17"),
        ("fdp", "-1.4241e+17,-1.08e+17,7.2e+17,7.2e+17"),
    ] {
        let src = format!(
            "graph {{ layout={engine}; pack=false; splines=curved; a [pos=\"0,0!\"]; \
             x [pos=\"1e16,0!\"]; y [pos=\"0,1e16!\"]; a -- x; a -- y }}"
        );
        let (out, err, ok) = run(&["-Tdot"], &src);
        assert!(ok, "{src}: {err}");
        assert!(out.contains(&format!("bb=\"{bb}\"")), "{src}: {out}");
    }
    // A complete graph of far pins (on a circle of radius 2e14 the same graph kept C running
    // for 18 minutes) must finish; once a segment runs out of splits, the later ones are
    // taken whole.
    let mut src = String::from("graph { layout=fdp; pack=false; splines=curved;");
    for i in 0..6 {
        let t = std::f64::consts::PI / 3.0 * f64::from(i);
        src.push_str(&format!(
            " n{i} [pos=\"{:?},{:?}!\"];",
            1.5e15 * graphviz_pure_core::math::cos(t),
            1.5e15 * graphviz_pure_core::math::sin(t)
        ));
        for j in 0..i {
            src.push_str(&format!(" n{j} -- n{i};"));
        }
    }
    src.push('}');
    let (_, err, ok) = run(&["-Tdot"], &src);
    assert!(ok, "{err}");
}

// The budget is per segment, not per layout: these 100 segments each finish within it but
// need 3.9e8 splits in all, and C (8 s) and the port print the same bb. The bb is C Graphviz
// 16.1.0's; a per-layout budget ran out at segment 66 and printed `0,-8.5824e+15,...`.
#[test]
fn many_curved_edges_c_finishes_keep_c_bb() {
    let mut src = String::from(
        "graph { layout=fdp; pack=false; splines=curved; top [pos=\"0,300000000000000.0!\"];",
    );
    for k in 0..100 {
        let l = 1e14 * (1.0 + f64::from(k) * 0.02);
        src.push_str(&format!(
            " l{k} [pos=\"{:?},0!\"]; r{k} [pos=\"{l:?},0!\"]; l{k} -- r{k};",
            -l
        ));
    }
    src.push('}');
    let (out, err, ok) = run(&["-Tdot"], &src);
    assert!(ok, "{err}");
    assert!(
        out.contains("bb=\"0,-6.4368e+15,4.2912e+16,2.16e+16\""),
        "{out}"
    );
}

// Issue #239: a pin at -inf gives a component an infinite box, so computeStep's discriminant
// is NaN and C fails `assert(d >= 0)` (exit 134; neato segfaults); the port filled grid
// cells without end. It now skips packing, as C does where `stepSize <= 0`.
#[test]
fn packing_a_component_with_an_infinite_pin_is_skipped() {
    for engine in ["fdp", "neato"] {
        let src = format!("digraph {{ layout={engine} n0 [pos=\"-inf,1e8!\"] n2 n1->n0 }}");
        let (out, err, ok) = run(&["-Tsvg"], &src);
        assert!(ok, "{engine}: {err}");
        assert_eq!(
            err, "Warning: components too large to pack; packing skipped\n",
            "{engine}"
        );
        assert!(out.contains("</svg>"), "{engine}");
    }
}

// Issue #240: overlap=portho* with pins from about 3e6 inches. C's initItem (constraint.c)
// converts the position to `int` through 64 bits and keeps the low 32, so it wraps, and
// its `int` arithmetic after that wraps too, where the port's panicked. C lays these out;
// the expected output is C Graphviz 16.1.0's. The last two have overlapping nodes, so
// overlap removal runs on the wrapped positions.
#[test]
fn portho_with_a_pin_at_1e12_lays_out_like_c() {
    let cases = [
        (
            "graph { layout=neato overlap=portho_yx a -- b; b [pos=\"1e12,0!\"] }",
            [
                "bb=\"0,0,3.6e+13,52.362\"",
                "pos=\"3.6e+13,18\"",
                "a -- b\t[pos=\"54.312,34.362 3.621e+07,34.362 3.6e+13,18 3.6e+13,18\"]",
            ],
        ),
        (
            "graph { layout=neato overlap=portho_yx a -- b; b [pos=\"-inf,1e12!\"] }",
            [
                "bb=\"0,0,-4.295e+09,7.2e+13\"",
                "pos=\"nan,7.2e+13\"",
                "a -- b\t[pos=\"nan,18.047 nan,2.1458e+06 nan,7.2e+13 nan,7.2e+13\"]",
            ],
        ),
    ];
    for (src, expected) in cases {
        let (out, err, ok) = run(&["-Tdot"], src);
        assert!(ok, "{src}: {err}");
        for e in expected {
            assert!(out.contains(e), "{src}: {e}\n{out}");
        }
    }
    let plain = [
        (
            "graph { overlap=portho a [pos=\"1e12,0!\"]; d [pos=\"-1e12,0!\"]; \
             b [pos=\"0,0!\"]; c [pos=\"0.1,0!\"] }",
            "graph 1 2.4722 1.7222\n\
             node a 2.0972 1.4722 0.75 0.5 a solid ellipse black lightgrey\n\
             node d 0.375 1.4722 0.75 0.5 d solid ellipse black lightgrey\n\
             node b 1.2361 0.86111 0.75 0.5 b solid ellipse black lightgrey\n\
             node c 1.2361 0.25 0.75 0.5 c solid ellipse black lightgrey\n\
             stop\n",
        ),
        (
            "graph { overlap=portho; a [pos=\"2e7,0!\"]; b [pos=\"0,0!\"]; c [pos=\"0.1,0!\"] }",
            "graph 1 1.6111 1.7222\n\
             node a 1.2361 1.4722 0.75 0.5 a solid ellipse black lightgrey\n\
             node b 0.375 0.86111 0.75 0.5 b solid ellipse black lightgrey\n\
             node c 0.375 0.25 0.75 0.5 c solid ellipse black lightgrey\n\
             stop\n",
        ),
    ];
    for (src, expected) in plain {
        let (out, err, ok) = run(&["-Kneato", "-Tplain"], src);
        assert!(ok, "{src}: {err}");
        assert_eq!(out, expected, "{src}");
    }
}

// overlap=vpsc: C's `Rectangle` and `Node` (lib/vpsc/generate-constraints.cpp) assert
// `x <= X` and `width < 1e40`, which a pin at NaN, infinity or 1e300 (infinite as the `float`
// coordinates removeoverlaps works on) fails, and C aborts; the port's solver never settled.
// The port skips overlap removal with a warning. Where the boxes pass and only the solver's
// cost is NaN, C's `while (fabs(lastcost - cost) > 0.0001)` ends the loop, and so does the
// port's: an unpinned node at 1e300 lays out as C's (all at NaN).
#[test]
fn vpsc_overlap_removal_with_extreme_positions_finishes() {
    for v in ["nan", "inf", "1e300"] {
        let src =
            format!("graph {{ layout=neato; overlap=vpsc; a [pos=\"{v},0!\"]; a -- b; b -- c }}");
        let (_, err, ok) = run(&["-Tsvg"], &src);
        assert!(ok, "{src}: {err}");
        assert_eq!(
            err, "Warning: node positions are not finite; overlap removal skipped\n",
            "{src}"
        );
    }
    let src = "digraph { layout=fdp; overlap=vpsc; n3 [pos=\"1,1e300\"]; n3->n4 }";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert!(out.contains("bb=\"0,0,104.5,-4.295e+09\""), "{out}");
    assert!(out.contains("pos=\"77.499,nan\""), "{out}");
}

// Issue #220: C reads `pos` with the C library's sscanf("%lf"), which takes hexadecimal
// floats and `nan(chars)`. The expected values are C Graphviz 16.1.0's output.
#[test]
fn neato_n_reads_hex_and_nan_chars_positions_like_c() {
    let src = "digraph { a [pos=\"0x10,0\"]; b [pos=\"30,30\"]; a -> b }";
    let (out, err, ok) = run(&["-Kneato", "-n2", "-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("pos=\"27,18\"") && out.contains("pos=\"41,48\""),
        "{out}"
    );
    let edge = |p: &str| {
        format!(
            "digraph {{ a [pos=\"0,0\"]; b [pos=\"30,30\"]; a -> b [pos=\"{p},0 10,10 20,20 30,30\"] }}"
        )
    };
    let (out, err, ok) = run(&["-Kneato", "-n2", "-Tsvg"], &edge("0x10"));
    assert!(ok && err.is_empty(), "{err}");
    assert!(out.contains("d=\"M43,-18C37,-28 47,-38 57,-48\""), "{out}");
    let (out, err, ok) = run(&["-Kneato", "-n2", "-Tsvg"], &edge("nan(1)"));
    assert!(ok && err.is_empty(), "{err}");
    assert!(out.contains("d=\"Mnan,-18C37,-28 47,-38 57,-48\""), "{out}");
}

// Issue #220 (fdp): initialPositions' sscanf("%lf,%lf%c") reads `0x1p3` as 8.
#[test]
fn fdp_reads_a_hex_float_pos_like_c() {
    let (out, err, ok) = run(
        &["-Tdot"],
        "graph { layout=fdp; a [pos=\"0x1p3,2\"]; a -- b }",
    );
    assert!(ok && err.is_empty(), "{err}");
    assert!(out.contains("pos=\"89.637,24.219\""), "{out}");
}

// fdp's chkPos (lib/fdpgen/layout.c) warns about a cluster `coords` it cannot read.
#[test]
fn fdp_warns_about_unreadable_cluster_coords_like_c() {
    let src = "graph { layout=fdp; subgraph cluster_a { coords=\"1,2,x\"; a } a -- b }";
    let (_, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: graph cluster_a, coord 1,2,x, expected four doubles\n"
    );
}

// Issue #245: with dim=3, user_pos (lib/neatogen/neatoinit.c) reads `z` when `pos` has two
// values, or else draws one with jitter3d, and addZ writes every node's `z` back. The
// expected values are C Graphviz 16.1.0's output.
#[test]
fn neato_dim3_reads_and_writes_z_like_c() {
    let src = "graph { layout=neato; dim=3; a [pos=\"1,2\" z=\"3\"]; a -- b; b -- c }";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert!(out.contains("bb=\"0,0,66.816,77.847\""), "{out}");
    for z in ["z=66.978896", "z=2.061028", "z=-69.039923"] {
        assert!(out.contains(z), "{z}: {out}");
    }
    // The -n path runs user_pos and addZ too.
    let src = "graph { dim=3; a [pos=\"10,20,5\" z=\"3\"]; b [pos=\"30,40\"]; a -- b }";
    let (out, err, ok) = run(&["-Kneato", "-n", "-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("z=5.000000") && out.contains("z=57.090927"),
        "{out}"
    );
}

// input.c's graph_init reads `ranksep` with sscanf("%lf"): for a blank value it returns EOF
// and assigns nothing, so the rank separation keeps the `nodesep` just read.
#[test]
fn blank_ranksep_takes_the_nodesep_like_c() {
    let (out, err, ok) = run(&["-Tdot"], "digraph { ranksep=\"\"; nodesep=1.5; a -> b }");
    assert!(ok, "{err}");
    assert!(out.contains("bb=\"0,0,54,180\""), "{out}");
}

// getdoubles2ptf (lib/common/input.c) rounds `size` to whole points.
#[test]
fn fractional_size_is_rounded_to_points_like_c() {
    let src = "digraph { size=\"1.013,1.017\"; a -> b -> c; a -> c }";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("transform=\"scale(0.388298 0.388298)"),
        "{out}"
    );
}

// init_job_viewport (lib/common/emit.c) reads `viewport="8,4,8,4"` with its second sscanf,
// "%lf,%lf,%lf,%[^,]%c", as a focus node named `4`; there is none, so the focus stays at the
// centre of the drawing.
#[test]
fn viewport_with_an_unknown_focus_node_keeps_the_centre_like_c() {
    let (out, err, ok) = run(&["-Tsvg"], "graph { viewport=\"8,4,8,4\"; a -- b }");
    assert!(ok, "{err}");
    assert!(out.contains("translate(-26.5 54.25)"), "{out}");
}

// Issue #245 with mode=sgd: initial_positions' randompos also draws the third coordinate
// (jitter3d), which addZ prints.
#[test]
fn neato_sgd_dim3_draws_z_like_c() {
    let src =
        "graph { layout=neato; mode=sgd; dim=3; node [z=\"\"]; a [pos=\"1,2!\"]; a -- b; b -- c }";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    for z in ["z=85.636391", "z=60.106840", "z=0.127218"] {
        assert!(out.contains(z), "{z}: {out}");
    }
}

// colorxlate's sscanf("%lf%lf%lf%lf") reads an HSV color written with hexadecimal floats.
#[test]
fn hsv_color_with_hex_floats_like_c() {
    let src = "graph { a [style=filled, fillcolor=\"0x1p-1 1 1\"] }";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(out.contains("fill=\"#00ffff\""), "{out}");
}

// graph_init: dpi = late_double(dpi, late_double(resolution, 0, 0), 0), so an unreadable
// `dpi` falls back to `resolution`.
#[test]
fn unreadable_dpi_falls_back_to_resolution_like_c() {
    let (out, err, ok) = run(&["-Tsvg"], "graph { dpi=\"x\"; resolution=144; a }");
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"124pt\" height=\"88pt\""),
        "{out}"
    );
}

// fdp's initialPositions (lib/fdpgen/fdpinit.c) warns about a `pos` it cannot read.
#[test]
fn fdp_warns_about_an_unreadable_pos_like_c() {
    let src = "graph { layout=fdp; a [pos=\"zz,0!\"]; a -- b }";
    let (_, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: node a, position zz,0!, expected two floats\n"
    );
}

// Fuzz case 003-pack-filledge-zero-point-spline-panic (issue #12), its repro.gv verbatim.
// Clipping leaves both edges between the overlapping pinned nodes with an empty spline. C
// crashes on it (exit 138, SIGBUS), so there is no C reference output and the fidelity
// ratchets never render it: this test is the regression check. Without the empty-list guard
// in `pack::fill_edge` the port panics with "index out of bounds: the len is 0 but the index
// is 0" and exits 101.
#[test]
fn packed_edges_with_empty_splines_do_not_panic() {
    let src = "graph G {
layout=neato;
splines=polyline;
pack=true;
n1 [pos=\"0,0!\"];
n9 [pos=\"0.2,0.1!\"];
n1 -- n9;
n9 -- n1;
n14;
}
";
    for fmt in ["-Tsvg", "-Tdot", "-Tpng"] {
        let (_, err, ok) = run(&[fmt], src);
        assert!(ok && !err.contains("panicked"), "{fmt}: {err}");
    }
}

/// Issue #17 (fuzz case `008-neato-router-star-cylinder-no-obstacle`): star and
/// cylinder nodes are `SH_POLY` in C (`star_fns` and `cylinder_fns` use
/// `poly_init`), so `makeObstacle` gives them a polygon obstacle and the
/// neato-family spline router routes edges around them. The expected paths are
/// C Graphviz 16.1.0's output.
#[test]
fn neato_splines_route_around_star_and_cylinder_nodes() {
    let src = "graph G {
layout=neato;
splines=true;
a [pos=\"0,0!\"];
b [pos=\"3,0!\"];
s [shape=star, pos=\"1.5,0!\"];
c [pos=\"0,1.5!\"];
d [pos=\"3,1.5!\"];
y [shape=cylinder, pos=\"1.5,1.5!\"];
a -- b;
c -- d;
}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"278pt\" height=\"188pt\""),
        "{out}"
    );
    assert!(
        out.contains(
            "d=\"M42.81,-67.3C62.71,-84.29 98.7,-109.87 135,-109.87 171.3,-109.87 207.29,-84.29 227.19,-67.3\""
        ),
        "a -- b must go around the star:\n{out}"
    );
    assert!(
        out.contains("d=\"M52.33,-167.2C67.41,-170.83 87,-174.94 104.68,-176.77 "),
        "c -- d must go around the cylinder:\n{out}"
    );
}

/// Issue #17, circo variant: with the star's obstacle, C's guiding polygon
/// makes `Pshortestpath` fail in `genroute` (`lib/neatogen/multispline.c`),
/// which warns and falls back to a straight edge.
#[test]
fn circo_star_obstacle_falls_back_to_straight_edge_with_warning() {
    let src = "graph G {
layout=circo;
splines=true;
n16 [shape=star];
n2:w -- n13;
}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "lib/pathplan/shortest.c:193: destination point not in any triangle\n\
         Warning: Could not create control points for multiple spline for edge (n2,n13)\n"
    );
    assert!(
        out.contains("<svg width=\"344pt\" height=\"150pt\""),
        "{out}"
    );
    assert!(
        out.contains("d=\"M155.98,-18C155.98,-18 237.75,-18 281.83,-18\""),
        "{out}"
    );
}

/// Issue #17, ortho: with `isOrtho`, C's `makeObstacle` gives every `SH_POLY`
/// node, star and cylinder included, its outline box without the margin
/// (`lib/neatogen/neatosplines.c:345-376`). Grown star and cylinder outlines
/// made the touch test fall back to straight lines (osage), or let a star whose
/// box overlaps a point reach the ortho router, which panicked (neato).
#[test]
fn ortho_star_and_cylinder_obstacles_are_outline_boxes() {
    let src = "graph G {
layout=osage;
splines=ortho;
a [shape=cylinder];
s [shape=star];
a -- b -- c -- a;
s -- b;
}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "d=\"M168.47,-111C177.84,-111 186,-111 186,-111 186,-111 186,0 186,0 186,0 55,0 55,0 55,0 55,-8.72 55,-17.58\""
        ),
        "c -- a must be routed orthogonally:\n{out}"
    );

    let src = "graph G {
layout=neato;
splines=ortho;
s [shape=star, pos=\"0,0!\"];
p [shape=point, width=0.05, pos=\"0.4,0.42!\"];
q [pos=\"2,0!\"];
r [pos=\"0,2!\"];
s -- q;
p -- r;
q -- r;
}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok && !err.contains("panicked"), "{err}");
    assert!(
        out.contains("d=\"M94.2,-52.53C118.85,-52.53 150.11,-52.53 172.09,-52.53\""),
        "touching boxes fall back to straight edges:\n{out}"
    );
}

/// Issue #17: a cylinder's obstacle repeats vertices, so some of its sides are
/// in no triangle. C's `addEndpoint` (`lib/neatogen/multispline.c:889`, `:897`)
/// then aborts on `findMap`'s `assert(ip)`; the port looped in `mk_poly`. It
/// now treats this as a failed multispline and falls back like other failures.
#[test]
fn cylinder_port_endpoint_does_not_hang_the_router() {
    let src = "graph G { layout=neato; splines=true; a [shape=cylinder]; a:n -- b; c -- a:w; }";
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .arg("-Tsvg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .unwrap();
    // The SVG is a few KB, well under the pipe buffer, so polling cannot block.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the router must terminate");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let (svg, err) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{err}");
    assert_eq!(svg.matches("class=\"edge\"").count(), 2, "{svg}");
}

// Issue #191. The repeated cluster gives the osage graph an infinite bb, and
// `xlhorder`'s `floor(log2(inf)) + 1` overflowed the Hilbert order in a debug build.
// C fails an assertion in `xlhdxload` (lib/label/xlabels.c) on these inputs; the port
// computes C's order (0), stops where C asserts without placing a label (#301), and exits
// cleanly.
#[test]
fn xlabels_with_an_infinite_bb_do_not_panic() {
    let cluster = "subgraph cluster_inner4{j1 j2}";
    let mut failures = Vec::new();
    for attrs in [
        "edge[label=L]",
        "edge[headlabel=L]",
        "j1[xlabel=X]",
        "splines=curved edge[label=L]",
        "splines=polyline edge[label=L]",
    ] {
        let src = format!(
            "digraph{{layout=osage {attrs}{{o2 {cluster}}}subgraph cluster_outer{{{{label=\"outer\"{cluster}}}->o1}}{cluster}}}"
        );
        let (_, err, ok) = run(&["-Tsvg"], &src);
        if !ok || err.contains("panicked") {
            failures.push(format!("{attrs}: {err}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// Issue #167, found by the fuzzer. `b` and `l` are articulation points that blocktree.c's
// `dfs` places in other blocks, so the block {i, k} has no edge of its own. C's
// `clone_graph` (lib/circogen/blockpath.c) zeroes every clone's `DEGREE`, and `getList`
// sorts the two nodes at degree 0; the port kept degrees only for nodes with an edge and
// panicked in `cmpDegree`. The input is a corpus file whose C -Tdot golden the circo ratchet
// also checks; the positions below are C's.
#[test]
fn circo_block_without_edges_lays_out_like_c() {
    let src = include_str!("../../../tests/circo/fuzz_blockpath_edgeless_block.gv");
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert!(out.contains("pos=\"373.36,289.22\""), "i: {out}");
    assert!(out.contains("pos=\"303.9,249.11\""), "k: {out}");
}

// Fuzz case 004-neato-router-record-obstacle-height (issue #13), its repro.gv verbatim. C's
// `makeObstacle` (lib/neatogen/neatosplines.c:447-449) builds a record's obstacle from the
// root field box `fld->b`, not from `ND_height`, which carries record_init's +1pt. With the
// taller box the port routed `a -- b` as a curve around the record instead of C's straight
// line.
#[test]
fn neato_record_obstacle_uses_field_box() {
    let src = "graph G {
layout=neato;
splines=true;
a [pos=\"0,0!\"];
b [pos=\"3,0!\"];
r [shape=record, label=\"a|b\", pos=\"1.5,0.3!\"];
a -- b;
}
";
    let (out, err, ok) = run(&["-Kneato", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(r#"d="M54.36,-18C95.79,-18 174.02,-18 215.53,-18""#),
        "{out}"
    );
}

// Issue #96. `round_corners` (lib/common/shapes.c:814) computes the folder tab's corner
// `AF[0].x - 2.25 * (AF[0].x - B[1].x)`, which clang fuses into one fmadd. Unfused, the
// corner rounds to -2.51 instead of C's -2.52.
#[test]
fn folder_tab_corner_is_fused_like_clang() {
    let src = "digraph G{layout=neato; notranslate=true; a [shape=folder, \
               width=0.13972222222222222, height=0.2, fixedsize=true, label=\"\", pos=\"0,0!\"];}\n";
    let (out, err, ok) = run(&["-Kneato", "-Tsvg"], src);
    assert!(ok, "{err}");
    let c_points = "points=\"5.03,-7.2 4.19,-8.32 -1.68,-8.32 -2.52,-7.2 -5.03,-7.2 \
                    -5.03,7.2 5.03,7.2 5.03,-7.2\"";
    assert!(out.contains(c_points), "{out}");
}

// Issue #89. `initRegular` (lib/neatogen/neatoinit.c:896) places the nodes of
// `start=regular` with `cos(a)` and `sin(a)`, which clang merges into one `__sincos_stret`
// call. C with the port's math (scripts/dot-portable-libm.sh, which interposes
// `__sincos_stret` with the portable `sin` and `cos`) lays a 37-node cycle out on a
// 758x735pt canvas with these node centres, and so must the port.
#[test]
fn start_regular_places_nodes_like_c_with_the_same_math() {
    let mut src = String::from("graph G { start=regular; mode=KK;\n");
    for i in 0..37 {
        src.push_str(&format!("  n{i} -- n{};\n", (i + 1) % 37));
    }
    src.push_str("}\n");
    let (out, err, ok) = run(&["-Kneato", "-Tsvg"], &src);
    assert!(ok, "{err}");
    for c in [
        "<svg width=\"758pt\" height=\"735pt\"",
        "cx=\"619.27\" cy=\"-429.2\"",
        "cx=\"592.3\" cy=\"-488.82\"",
        "cx=\"630.69\" cy=\"-548\"",
    ] {
        assert!(out.contains(c), "{c}: {out}");
    }
}

// The circo variant of issue #13 (repro-circo.gv). With C's record box the guiding polygon
// does not triangulate, so `Pshortestpath` fails, genroute warns (multispline.c:703) and
// the edge falls back to a straight line.
#[test]
fn circo_record_obstacle_falls_back_like_c() {
    let src = "graph G {
layout=circo;
splines=spline;
n7 [shape=record];
n1 -- n7:n;
n2;
}
";
    let (out, err, ok) = run(&["-Kcirco", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(r#"d="M126.28,-22.32C89.29,-27.6 27,-36.5 27,-36.5""#),
        "{out}"
    );
    assert_eq!(
        err,
        "lib/pathplan/shortest.c:193: destination point not in any triangle\n\
         Warning: Could not create control points for multiple spline for edge (n1,n7)\n"
    );
}

// Fuzz case 014-dot-pack-failed-component-not-absorbed (issue #23), its repro.gv verbatim.
// `n9` is both in `cluster3` and in a `rank=source` set, so mincross fails in the second
// pack component (`_cc_1`; the empty `cluster0` is `_cc_0`). C (`doDot`, dotinit.c) returns
// the failure and renders the half-made drawing: the component's nodes are the root's own
// objects, so they keep the sizes `dot_init_node_edge` gave them (`rx="27"`), and the
// messages name the component. The port used to drop the failed component's records
// (`rx="0"`), name the root `G` and omit the warning.
#[test]
fn failed_pack_component_keeps_its_node_sizes() {
    let src = "graph \"G\" {
pack=\"true\";
subgraph \"cluster0\" {
}
subgraph \"cluster3\" {
\"n1\" [];
\"n8\" [];
\"n9\" [];
}
{
rank=source;
\"n7\";
\"n9\";
}
\"n1\":\"c\" -- \"n7\";
\"n8\" -- \"n9\" [];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok);
    assert_eq!(
        err,
        "Warning: n9 was already in a rankset, deleted from cluster _cc_1\n\
         Error: install_in_rank, line 1158: _cc_1 %0 rank 2 i = 0 an = 0\n"
    );
    let sized = r#"<ellipse fill="none" stroke="black" cx="0" cy="0" rx="27" ry="18"/>"#;
    assert_eq!(out.matches(sized).count(), 4, "{out}");
}

// Fuzz case 017-dot-pack-curved-edge-subpoint (issue #26), its repro.gv verbatim. In a pack
// component, C's `clip_and_install` grows `GD_bb(agraphof(tail))`, the root graph, so the
// component's box stays at its `dot_position` value (y up to 180) while its edges are routed.
// The second curved edge bends towards that box's centre (`get_centroid`, routespl.c). The
// port grew the component's box with the first edge's spline (to 180.32), which moved the
// bend of `n59:sw -- n72`. C 16.1.0 output below.
#[test]
fn packed_curved_edge_bends_towards_component_box_without_splines() {
    let src = "graph \"G\" {
splines=\"curved\";
pack=\"8\";
\"n26\":\"n\" -- \"n59\":\"c\";
\"n59\":\"sw\" -- \"n72\";
\"n61\";
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("d=\"M26.98,-74.98C25.94,-53.57 25.46,-43.7 25.55,-36.45\""),
        "{out}"
    );
}

/// Issue #201, reduced: neato pins `a` at NaN, so `a -- x` gets NaN control points, while
/// `b` keeps `bb` finite and the segment falls outside it. `update_bb_bz`
/// (`lib/common/emit.c`) then splits the segment with `Bezier` until C's stack overflows
/// (SIGSEGV); the issue's second pin at 1e12 is not needed. The port leaves `bb` as it was.
#[test]
fn neato_nan_pin_edge_does_not_overflow_update_bb_bz() {
    let src = "graph { layout=neato; pack=false; a [pos=\"nan,0!\"]; a -- x; b }\n";
    for fmt in ["-Tsvg", "-Tdot"] {
        let (out, err, ok) = run(&[fmt], src);
        assert!(ok, "{fmt}: {err}");
        if fmt == "-Tdot" {
            assert!(out.contains("bb=\"0,0,54,68.723\""), "{out}");
            assert!(out.contains("a -- x\t[pos="), "{out}");
        }
    }
}

// Fuzz case 007-dot-port-endpoint-attr-sym-id (issue #16), its repro.gv verbatim. The flat
// edge `a:sw -> b` is routed by the nested layout of `make_flat_adj_edges`, where C's stale
// `E_arrowsz` reads the aux graph attribute whose id equals the main `arrowsize` id. C's
// `mkport` (lib/cgraph/grammar.y) declares `tailport` when it meets `a:sw`, so that id lands
// on `tailport` ("sw", not a number: arrow size 1). Without the declaration the id lands on
// `fontsize` (27.3), the edge is clipped to almost nothing and the graph is 44pt tall.
#[test]
fn port_endpoint_declares_its_edge_attribute_like_mkport() {
    let src = "digraph G {
edge [constraint=false];
a:sw -> b [fontsize=27.3];
c -> d [arrowsize=2];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"278pt\" height=\"56pt\""),
        "{out}"
    );
    assert!(
        out.contains(
            "<path fill=\"none\" stroke=\"black\" \
             d=\"M12.98,-14.75C-4.84,8.98 37.44,-1.06 68.55,-13.3\"/>"
        ),
        "{out}"
    );
}

// Issue #168, from the coverage-guided fuzzer. mode=ipsep always lays out the connected
// components separately, and cluster_map (lib/neatogen/neatoinit.c) takes the clusters from
// the root graph: a cluster member in another component gets the index agnnodes(g), and C fails
// bitarray_set's "out of bounds access" assertion (exit 134). The port used to carry that index
// into generateNonoverlapConstraints and panic under overlap=ipsep (without it the index
// silently named a cluster dummy variable); it now keeps each component's own members.
#[test]
fn neato_ipsep_clusters_in_other_components_do_not_panic() {
    for src in [
        "graph { layout=neato; mode=ipsep; overlap=ipsep; subgraph cluster_a { a } subgraph cluster_b { b } a -- c }",
        // one cluster split across two components
        "graph { layout=neato; mode=ipsep; overlap=ipsep; subgraph cluster_a { a b } a -- c }",
    ] {
        let (out, err, ok) = run(&["-Tsvg"], src);
        assert!(ok, "{src}: {err}");
        assert_eq!(err, "", "{src}");
        assert_eq!(out.matches("class=\"node\"").count(), 3, "{src}: {out}");
    }
}

// Fuzz case 013-dot-pack-component-missing-attr-dicts (issue #22), its repro.gv verbatim.
// The flat adjacent edge `25:ne -> 26` is routed through make_flat_adj_edges' auxiliary
// graph, whose setState keeps the main graph's N_penwidth/E_penwidth/E_arrowsz symbols and
// reads them by id (lib/dotgen/dotsplines.c). A pack component is a subgraph of the root and
// shares its dictionaries, so `xlabel` on 22 and `penwidth` on 24 (other components) shift
// those ids. Without the root dictionaries in `CComp::to_graph` the edge moved by about 1pt.
#[test]
fn pack_component_keeps_root_attribute_dictionaries() {
    let src = "digraph \"G\" {
pack=\"true\";
node [width=\"2.7\"];
\"22\" [xlabel=\"\u{df}\"];
\"24\" [penwidth=\"4.79\"];
\"25\" [fontsize=\"2\"];
\"25\":\"ne\" -> \"26\" [style=\"bold,dashed\", minlen=\"0\"];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    // C Graphviz 16.1.0.
    assert!(
        out.contains("d=\"M114.95,-111.75C149.34,-141.07 206.35,-128.53 249.55,-113.61\""),
        "{out}"
    );
    assert!(
        out.contains("points=\"249.13,-117.47 257.37,-110.81 246.77,-110.88 249.13,-117.47\""),
        "{out}"
    );
}

// Fuzz case 005-xlabels-hilbert-key-signed-order (issue #14). The drawings are wider than
// 32768pt, so some objects' Hilbert keys have bit 31 set. C stores the key in an `int`
// (`HDict_t.key`) and orders the keys with `icompare` on `int` (lib/label/xlabels.c:28-35),
// so those keys sort first; sorting them as `u32` builds a different R-tree and moves the
// label by about its width. The inputs are corpus files whose C -Tdot goldens the fdp and
// circo ratchets also check; the coordinates below are C's.
#[test]
fn xlabel_hilbert_keys_are_ordered_as_signed_int() {
    let fdp = include_str!("../../../tests/fdp/fuzz_xlabels_hilbert_signed_key.gv");
    let (out, err, ok) = run(&["-Tdot"], fdp);
    assert!(ok, "{err}");
    assert!(out.contains("10 -> 0\t[lp=\"14095,11133\""), "fdp: {out}");
    let circo = include_str!("../../../tests/circo/fuzz_xlabels_hilbert_signed_key.gv");
    let (out, err, ok) = run(&["-Tdot"], circo);
    assert!(ok, "{err}");
    assert!(
        out.contains("n68 -- n98\t[head_lp=\"34855,2663.7\""),
        "circo: {out}"
    );
}

// Fuzz case 015-dot-pack-unplaced-edge-label-shifted-off-page (issue #24), its repro.gv
// verbatim. With `splines=none` dot places no edge label (`set` stays false), but the label
// keeps its position: `(0,0)`, moved by the pack offset in `shiftEdge` (lib/pack/pack.c).
// `edge_in_box` (lib/common/emit.c) tests `ED_label(e)` without looking at `set`, so C finds
// the packed label off the page and does not emit the edge, while without pack the label sits
// at the origin and C writes an empty edge group. The port used to lose the unset position.
#[test]
fn dot_pack_unplaced_edge_label_off_page_skips_edge() {
    let src = "digraph G {
splines=none;
pack=true;
c;
a -> b [label=\"lbl\"];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(!out.contains("class=\"edge\""), "{out}");
    // -Tplain writes the edge with the shifted, unset label position (C 16.1.0).
    let (out, err, ok) = run(&["-Kdot", "-Tplain"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(" lbl -0.069444 -0.88889 solid black\n"),
        "{out}"
    );
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G { splines=none; c; a -> b [label=\"lbl\"]; }",
    );
    assert!(ok, "{err}");
    assert_eq!(out.matches("class=\"edge\"").count(), 1, "{out}");
}

// Issues #84 and #113 (cause:aux-graph). The flat adjacent edges with a port are laid out in
// make_flat_adj_edges' aux graph, which declares the main attributes in name order while
// E_penwidth, E_arrowsz and N_penwidth keep the main graph's symbol ids. #84: E_penwidth's id
// is the aux `weight`, read after the heavy edge `s -> n` gets weight=10000, so its arrow clip
// is huge and C cuts the spline down to a fraction of a point. #113: `fixed` is declared after
// node "2" was made, so the node holds "" and its aux copy is not fixedsize.
#[test]
fn flat_adjacent_aux_graph_reads_symbols_by_main_id_like_c() {
    let cases = [
        (
            "digraph G {
edge [weight=5, constraint=false];
s -> n;
s -> n [headclip=false, headport=w];
c -> d [fontcolor=blue, penwidth=3.99];
}
",
            "d=\"M54.46,-18C54.55,-18 54.64,-18 54.73,-18\"",
        ),
        (
            "digraph G { {rank=same; \"2\" [label=\"a long label here\"]; \"2\" -> \"10\" [label=\"x\", tailport=\"e\"];} node[fixed=true]; }",
            "d=\"M150.24,-18C158.3,-18 157.03,-26.42 164.68,-29 167.62,-29.99 168.59,-29.41 \
             171.68,-29 173.56,-28.75 175.48,-28.43 177.41,-28.07\"",
        ),
    ];
    for (src, path) in cases {
        let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
        assert!(ok, "{src}: {err}");
        assert!(out.contains(path), "{src}: {out}");
    }
    // `label` is the ProtoGraph's node symbol 0, so `aaa` is id 1 in both graphs and
    // N_penwidth (id 2) reads `penwidth`; sorted by name, id 2 would be `label`.
    let src = "digraph G { node [penwidth=1, aaa=9, shape=box]; {rank=same; a -> b [tailport=e]; a -> b [label=x];} }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    for path in [
        "d=\"M54,-18C64.27,-18 59.23,-31.57 68.5,-36 70.21,-36.82 71.31,-36.88 74.95,-35.97\"",
        "d=\"M58.07,-15.69C61.58,-15.49 65.12,-15.32 68.5,-15.2 70.4,-15.13 72.35,-15.1 74.31,-15.1\"",
    ] {
        assert!(out.contains(path), "{out}");
    }
    // `-G` attributes reach auxg through the ProtoGraph: the aux layout orders its edges.
    let src = "graph G { node [shape=box]; {rank=same; a -- b [tailport=s, label=\"long label one\", headlabel=\"H\"]; a -- b [headport=n, label=\"two\"]; a -- b [headport=s, label=\"three\", taillabel=\"T\"];} c -- a; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg", "-Gordering=out"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"213pt\" height=\"159pt\""),
        "{out}"
    );
    // `-E` declares `penwidth` on cgraph's ProtoGraph, so it keeps id 0 in auxg too, and
    // E_penwidth reads itself; sorted by name, id 0 would be `arrowsize` (3).
    let src = "digraph G { edge [arrowsize=3]; {rank=same; a -> b [tailport=e];} }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg", "-Epenwidth=1"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains("d=\"M54,-18C54.1,-17.95 54.2,-17.91 54.29,-17.87\""),
        "{out}"
    );
}

/// Issue #138: with a rotated box (or `shape=none`) and a compass port, both
/// endpoints get triangle edges but `triPath` never reaches the tail, so the
/// `dad` walk in C's `mkPoly` (`lib/neatogen/multispline.c:968-971`) stops on a
/// node that is its own parent and aborts on `assert(nxt != dad[nxt])`, or, when
/// node 0 was reached, on `edgeToSeg`'s `assert(0)` (which one varies between C
/// runs). The port looped forever in the first case. It now treats both as a
/// failed multispline and falls back like other failures, on every
/// neato-family engine. On twopi the record graph takes the `edgeToSeg` path.
#[test]
fn mk_poly_unreached_tail_does_not_hang_the_router() {
    let srcs = [
        (
            "graph G { splines=true; a [shape=box, orientation=90]; b:se -- a; }",
            1,
        ),
        (
            "graph G { splines=polyline; a [shape=none, orientation=90, peripheries=2]; b:se -- a; }",
            1,
        ),
        (
            "graph G { splines=true; n0 [shape=record, orientation=45, peripheries=2]; \
             n1 [shape=box, orientation=90]; n1:e -- n1:sw; n1:c -- n0:n; n0:nw -- n0:e; \
             n1:s -- n0:s; n1:c -- n0:se; }",
            5,
        ),
    ];
    for layout in ["neato", "fdp", "sfdp", "circo", "twopi"] {
        for (src, edges) in srcs {
            let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
                .args(["-Tsvg", &format!("-K{layout}")])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(src.as_bytes())
                .unwrap();
            // The SVG is a few KB, well under the pipe buffer, so polling cannot block.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while child.try_wait().unwrap().is_none() {
                if std::time::Instant::now() > deadline {
                    child.kill().unwrap();
                    panic!("{layout}: the router must terminate on {src}");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let out = child.wait_with_output().unwrap();
            let (svg, err) = (
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            assert!(out.status.success(), "{layout}: {err}");
            assert_eq!(
                svg.matches("class=\"edge\"").count(),
                edges,
                "{layout}: {svg}"
            );
        }
    }
}

// Fuzz case 009-xlabels-dtobag-equal-key-order (issue #18), its repro.gv verbatim. Five
// nodes share the pinned position, so several label objects get the same Hilbert key.
// `placeLabels` (lib/label/xlabels.c:xlhdxload/xlspdxload) loads the R-tree in the order of a
// cdt `Dtobag`, whose splay tree puts a duplicate key just before the equal element its
// search stops at (lib/cdt/dttree.c), so equal keys are not simply in reverse insertion
// order. With the old sort the edge label moved one candidate step down (y="-52.88").
#[test]
fn xlabels_equal_hilbert_keys_follow_dtobag_order() {
    let src = r#"graph "G" {
layout=neato;
node [pos="31,-83!"];
"n0" [];
"n1" [label=<<TABLE BORDER="0" COLOR="black"><TR><TD BGCOLOR="lightblue">x</TD><TD WIDTH="28" HEIGHT="30">&lt;x&gt;</TD><TD><FONT POINT-SIZE="8">small</FONT></TD></TR></TABLE>>];
"n3" [shape="star"];
"n7" [height="1.68"];
"n12" [pos="-241,74"];
"n1" -- "n5" [label="\E", headport="w"];
}
"#;
    let (out, err, ok) = run(&["-Kneato", "-Tsvg"], src);
    assert!(ok, "{err}");
    // C Graphviz 16.1.0's line for the edge label.
    let c = r#"<text xml:space="preserve" text-anchor="middle" x="18.66" y="-69.68" font-family="Times,serif" font-size="14.00">n1&#45;&#45;n5</text>"#;
    assert!(out.contains(c), "{out}");
}

// Fuzz case 012-dot-flat-adj-labelfloat (issue #21), its repro.gv verbatim. A flat edge
// between adjacent nodes with a port is routed by a nested dot layout
// (`make_flat_adj_edges`). There C's `setState` looks `E_label_float` up as "label_float"
// (lib/dotgen/dotsplines.c:745), so the nested layout ignores `labelfloat=true`. The
// expected values are C 16.1.0's -Tsvg output; passing `labelfloat` through gives a
// 49.8pt-high graph with the label at y=-33.2.
#[test]
fn flat_adjacent_edge_ignores_labelfloat_like_c() {
    let src = "digraph G {
{rank=same; \"2\" -> \"10\" [label=\"x\", tailport=\"e\", labelfloat=\"true\"];}
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    for want in [
        "translate(4 50.35)",
        "d=\"M54,-18C62.09,-18 60.83,-26.42 68.5,-29 71.45,-29.99 72.42,-29.41 75.5,-29 \
         77.39,-28.75 79.31,-28.43 81.24,-28.07\"",
        "points=\"81.77,-31.54 90.75,-25.92 80.23,-24.71 81.77,-31.54\"",
        "x=\"72\" y=\"-33.75\"",
    ] {
        assert!(out.contains(want), "missing {want}:\n{out}");
    }
}

/// Issue #169: with mode=hier, `compute_apsp_packed`'s integer BFS truncates every len
/// below 1 to 0 (`lib/neatogen/bfs.c`), so the stress Laplacian is all zero, or all NaN
/// with levelsgap > 0. Every desired place in `constrained_majorization_new_with_gaps`
/// (`lib/neatogen/quad_prog_solve.c`) is then NaN, the block comes out empty, and C fails
/// the `gv_list_get` bounds assertion. The port indexed the empty block at `len - 1` and
/// panicked; it must lay the graph out.
#[test]
fn neato_hier_zero_laplacian_does_not_panic() {
    for src in [
        "graph{layout=neato\nmode=hier\nb--e[len=0.5]}\n",
        "digraph{layout=neato\nmode=hier\nlevelsgap=1\na->b->c[len=0.5]}\n",
        "graph{layout=neato\nmode=hier\nstart=random5\ndim=3\nb--e[len=0.5]}\n",
    ] {
        let (out, err, ok) = run(&["-Tsvg"], src);
        assert!(ok, "{src}: {err}");
        assert!(out.contains("</svg>"), "{src}: {out}");
        assert!(!out.contains("NaN"), "{src}: {out}");
    }
}

// Issue #21, the other side of C's `label_float` lookup: an aux edge floats its label when
// the main graph sets `label_float`. C 16.1.0 then draws what the port drew for the
// repro above before the fix (label at y=-33.2).
#[test]
fn flat_adjacent_edge_floats_label_from_label_float_like_c() {
    let src = "digraph G {
edge [label_float=\"true\"];
{rank=same; \"2\" -> \"10\" [label=\"x\", tailport=\"e\"];}
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    for want in [
        "translate(4 49.8)",
        "d=\"M54,-18C63.19,-18 62.67,-26.46 71.5,-29 75.17,-30.06 79.03,-30.33 82.88,-30.08\"",
        "x=\"72\" y=\"-33.2\"",
    ] {
        assert!(out.contains(want), "missing {want}:\n{out}");
    }
}

// Issue #144: make_flat_adj_edges runs dot_init_node_edge on its aux graph, so C prints the
// label diagnostics of the copied nodes and edges again (dotsplines.c:1208). The aux graph
// reads `fixed`, not `fixedsize`, and has no xlabel or headlabel symbol, so `size too small
// for label` and the xlabel and headlabel warnings print once.
#[test]
fn flat_adjacent_aux_graph_repeats_label_diagnostics_like_c() {
    let src = "digraph G { {rank=same; a -> b [label=<<FONT Z=\"2\">x</FONT>>, \
               headlabel=<<FONT H=\"1\">h</FONT>>, tailport=\"e\"];} \
               a [label=<<FONT Q=\"1\">a</FONT>>, xlabel=<<FONT W=\"2\">w</FONT>>]; \
               b [shape=box, fixedsize=true, width=0.75, height=0.1, label=\"abcdefghij\"]; }\n";
    let (_, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: Illegal attribute Q in <FONT> - ignored\n\
         in label of node a\n\
         Warning: Illegal attribute W in <FONT> - ignored\n\
         in label of node a\n\
         Warning: node 'b', graph 'G' size too small for label\n\
         Warning: Illegal attribute Z in <FONT> - ignored\n\
         in label of edge a -> b\n\
         Warning: Illegal attribute H in <FONT> - ignored\n\
         in label of edge a -> b\n\
         Warning: Illegal attribute Q in <FONT> - ignored\n\
         in label of node a\n\
         Warning: Illegal attribute Z in <FONT> - ignored\n\
         in label of edge a -> b\n"
    );
}

// Issue #144: htmlEntityUTF8 warns about invalid UTF-8 once per graph, and starts again when
// called with another graph (`lastg`): the aux graph's dot_init_node_edge warns for `auxg`,
// and the next component's for `G` again.
#[test]
fn flat_adjacent_aux_graph_repeats_invalid_utf8_warning_like_c() {
    let path = tmp("issue144_utf8.gv");
    std::fs::write(
        &path,
        b"digraph G { pack=true; {rank=same; a -> b [tailport=e label=\"\xe9\"];} \
          c [label=\"\xe9\"] }\n",
    )
    .unwrap();
    let (_, err, ok) = run(&["-Kdot", "-Tsvg", path.to_str().unwrap()], "");
    assert!(ok, "{err}");
    let utf8 = |g: &str| {
        format!(
            "Warning: Invalid 3-byte UTF8 found in input of graph {g} - treated as Latin-1. \
             Perhaps \"-Gcharset=latin1\" is needed?\n"
        )
    };
    let want = [
        utf8("G"),
        "Warning: Warning: no value for width of non-ASCII character 195. Falling back to \
         width of space character\n"
            .to_string(),
        utf8("auxg"),
        utf8("G"),
    ]
    .concat();
    assert_eq!(err, want);
    // Only HTML labels in the aux graph, whose entities the HTML lexer decodes: htmlEntityUTF8
    // is not called for it, lastg stays G, and the tooltip rendered later does not warn again.
    std::fs::write(
        &path,
        b"digraph G { node [label=<x&amp;y>]; {rank=same; a -> b [tailport=e]} \
          c [label=\"\xe9\" tooltip=\"\xe9\"] }\n",
    )
    .unwrap();
    let (_, err, ok) = run(&["-Kdot", "-Tsvg", path.to_str().unwrap()], "");
    assert!(ok, "{err}");
    let want = [
        utf8("G"),
        "Warning: Warning: no value for width of non-ASCII character 195. Falling back to \
         width of space character\n"
            .to_string(),
    ]
    .concat();
    assert_eq!(err, want);
}

// Issue #105: with pack, doDot runs cccomps before dotLayout's dot_init_node_edge sizes the
// nodes, so its `two non-nested clusters` error comes before the font warning, with one
// component and with several.
#[test]
fn pack_cccomps_error_precedes_node_sizing_warnings_like_c() {
    for extra in ["", " y"] {
        let src = format!(
            "digraph G {{ pack=true; node [fontname=Bookman]; \
             subgraph cluster_a {{ x }} subgraph cluster_b {{ x }}{extra} }}\n"
        );
        let (_, err, _) = run(&["-Kdot", "-Tsvg"], &src);
        assert_eq!(
            err,
            "Error: node \"x\" belongs to two non-nested clusters \"cluster_b\" and \
             \"cluster_a\"\n\
             Warning: Warning: no hard-coded metrics for 'Bookman'.  Falling back to 'Times' \
             metrics\n",
            "{src}"
        );
    }
    // The port's sizing pre-pass is quiet, so it neither prints nor uses up the invalid UTF-8
    // warning, which comes from dot_init_node_edge after cccomps.
    let path = tmp("issue105_utf8.gv");
    std::fs::write(
        &path,
        b"digraph G { pack=true; subgraph cluster_a { x } subgraph cluster_b { x } \
          y [label=\"\xe9\"] }\n",
    )
    .unwrap();
    let (_, err, _) = run(&["-Kdot", "-Tsvg", path.to_str().unwrap()], "");
    assert_eq!(
        err,
        "Error: node \"x\" belongs to two non-nested clusters \"cluster_b\" and \"cluster_a\"\n\
         Warning: Invalid 3-byte UTF8 found in input of graph G - treated as Latin-1. Perhaps \
         \"-Gcharset=latin1\" is needed?\n\
         Warning: Warning: no value for width of non-ASCII character 195. Falling back to width \
         of space character\n"
    );
}

// Issue #21: `label_float` declared after the edge. cgraph gives the existing edge ""
// (not the later default), `cloneEdge` copies that, and the label does not float.
#[test]
fn label_float_declared_after_the_edge_does_not_float_like_c() {
    let src = "digraph G {
{rank=same; \"2\" -> \"10\" [label=\"x\", tailport=\"e\"];}
edge [label_float=\"true\"];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(out.contains("translate(4 50.35)"), "{out}");
    assert!(out.contains("x=\"72\" y=\"-33.75\""), "{out}");
}

// Issue #114: after `trouble in init_rank` C segfaults in mincross's `furthestnode` (exit
// 139). The port panicked indexing `GD_rank` in `maximal_bbox` with a node of `g -> h`'s
// chain that mincross never placed (its `ND_rank` is still an x coordinate);
// `make_regular_edge` now fails the layout there.
#[test]
fn init_rank_trouble_unplaced_node_fails_the_layout() {
    let src = "digraph G {
rank=sink;
subgraph cluster0 { a; b; c; d }
subgraph cluster_1 { e; subgraph cluster_2 { f } }
subgraph cluster_3 { }
{ d }
{ e }
{ }
f -> e;
g -> h [minlen=2];
d -> c;
i -> h [label=x];
subgraph s { rank=source; a; h; subgraph cluster_in { } }
}
";
    for fmt in ["-Tsvg", "-Tdot"] {
        let (_, err, ok) = run(&["-Kdot", fmt], src);
        assert!(!ok, "{fmt}: {err}");
        assert!(!err.contains("panicked"), "{fmt}: {err}");
        assert!(
            err.starts_with(
                "Warning: a was already in a rankset, deleted from cluster G\n\
                 Warning: c was already in a rankset, deleted from cluster G\n\
                 Warning: d was already in a rankset, deleted from cluster G\n\
                 Warning: e was already in a rankset, deleted from cluster G\n\
                 Error: trouble in init_rank\n\
                 \tc 3\n\
                 \t%0 1\n\
                 \tf 1\n\
                 \ti 1\n\
                 \tg 1\n"
            ),
            "{fmt}: {err}"
        );
        assert!(
            err.ends_with(
                "Error: <stdin>: Layout computation error in dot: make_regular_edge: edge g -> \
                 h reaches a node on rank 112, outside the ranks 0 to 5\n"
            ),
            "{fmt}: {err}"
        );
    }
}

// `trouble in init_rank` alone does not stop C: a node in both a min and a max set is
// reported, then laid out (exit 1). Issue #114's fix must leave this drawing as C's.
#[test]
fn init_rank_trouble_without_lost_nodes_lays_out_like_c() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tdot"],
        "digraph G { {rank=max; a} {rank=min; a} b -> a; }",
    );
    assert!(!ok, "{err}");
    assert_eq!(err, "Error: trouble in init_rank\n\ta 1\n\tb 1\n");
    assert!(out.contains("bb=\"0,0,126,36\""), "{out}");
    assert!(out.contains("pos=\"99,18\""), "{out}");
    assert!(out.contains("pos=\"27,18\""), "{out}");
    assert!(
        out.contains("pos=\"e,71.75,18 54.281,18 56.283,18 58.284,18 60.286,18\""),
        "{out}"
    );
}

// With `splines=false` the same kind of unplaced node is drawn: C's `makeLineEdge` reads no
// rank, so `make_regular_edge` must not check one (issue #114 review). stderr and the SVG
// are C 16.1.0's.
#[test]
fn init_rank_trouble_unplaced_node_with_straight_lines_draws_like_c() {
    let src = "digraph G {
splines=false;
rank=sink;
subgraph cluster0 { a; b; c; d }
subgraph cluster_1 { e; subgraph cluster_2 { f } }
{ d }
{ e }
f -> e;
g -> h [minlen=2];
d -> c;
d -> h [minlen=0];
i -> h [label=x];
subgraph s { rank=source; a; h; subgraph cluster_in { } }
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok, "{err}");
    assert_eq!(
        err,
        "Warning: a was already in a rankset, deleted from cluster G\n\
         Warning: c was already in a rankset, deleted from cluster G\n\
         Warning: d was already in a rankset, deleted from cluster G\n\
         Warning: e was already in a rankset, deleted from cluster G\n\
         Error: trouble in init_rank\n\
         \tc 3\n\
         \t%0 1\n\
         \tf 1\n\
         \ti 1\n\
         \tg 1\n"
    );
    for want in [
        "cx=\"376\" cy=\"-91\"",
        "d=\"M376,-205.51C376,-183.57 376,-146.71 376,-120.75\"",
        "d=\"M127.6,-33.88C149.38,-59.32 192.25,-109.39 217.99,-139.46\"",
        "d=\"M239,-205.6C239,-201.81 239,-197.71 239,-193.62\"",
    ] {
        assert!(out.contains(want), "{want}\n{out}");
    }
}

// Issue #252: C checks two weight sums for overflow, `virtual_weight` (mincross.c:1725) and
// `x_cutval` (ns.c:1060), and exits 1 with its error after writing the graphs before. The
// port fails the layout with C's message, and writes nothing more.
#[test]
fn weight_overflow_fails_the_layout_like_c() {
    for (src, msg, before) in [
        (
            "digraph A { x }\ndigraph B { a -> c [weight=2000000000 minlen=3] }\ndigraph C { y }",
            "overflow when calculating virtual weight of edge",
            "graph 1 0.75 0.5\nnode x 0.375 0.25 0.75 0.5 x solid ellipse black lightgrey\nstop\n",
        ),
        (
            "digraph { a -> b [weight=2000000000]; a -> c [weight=2000000000]; \
             b -> d [weight=2000000000]; c -> d [weight=2000000000] }",
            "overflow when computing edge weight sum",
            "",
        ),
    ] {
        let (out, err, ok) = run(&["-Tplain"], src);
        assert!(!ok, "{src}");
        assert_eq!(
            err,
            format!("Error: <stdin>: Layout computation error in dot: {msg}\n")
        );
        assert_eq!(out, before, "{src}");
    }
}

// Weights that wrap negative where C has no check leave a tree edge with a negative cut value
// and no edge to replace it. C passes the NULL edge to `update` and crashes (ns.c:710); the
// port fails the layout there. Before, it went on and panicked in mincross (with a cluster)
// or class2 (with newrank).
#[test]
fn wrapped_weights_without_an_entering_edge_fail_the_layout() {
    for src in [
        "digraph { subgraph cluster0 { n0 } n0 -> n2 [weight=1073741824]; \
         n0 -> n0 [weight=268435456]; n1 -> n2 [weight=268435456 minlen=0]; \
         n1 -> n0 [weight=2000000000]; n0 -> n1 [weight=0] }",
        "digraph { newrank=true; n3 -> n2 [weight=1000 label=x]; \
         n5 -> n1 [weight=100000000 minlen=2]; n0 -> n2 [weight=1073741824]; \
         n0 -> n2 [weight=1000 minlen=3]; n0 -> n1 [weight=2000000000]; \
         n5 -> n1 [weight=2147483647] }",
    ] {
        let (out, err, ok) = run(&["-Tplain"], src);
        assert!(!ok, "{src}");
        assert_eq!(out, "", "{src}");
        assert_eq!(
            err,
            "Error: <stdin>: Layout computation error in dot: network simplex: no edge can \
             enter the tree for a tree edge with a negative cut value\n",
            "{src}"
        );
    }
}

// C's `largeMinlen` (position.c:64) reports an aux edge longer than INT_MAX before clamping
// it, and the error makes dot exit 1 after drawing.
#[test]
fn overlong_aux_edge_is_reported_like_c() {
    let (out, err, ok) = run(&["-Tplain"], "graph { nodesep=\"1e8\"; a; b }");
    assert!(!ok);
    assert_eq!(
        err,
        "Error: Edge length 2147483701.000000 larger than maximum 2147483647 allowed.\n\
         Check for overwide node(s).\n"
    );
    assert_eq!(
        out,
        "graph 1 2.9826e+07 0.5\nnode a 0.375 0.25 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 2.9826e+07 0.25 0.75 0.5 b solid ellipse black lightgrey\nstop\n"
    );
}

// Issue #271: C's `int` sums on huge `ranksep` (`edgelabel_ranks`), `nodesep` and `margin`
// (network simplex's `init_rank` and `init_graph`) have no check; the reference build wraps
// and draws. The port wraps too, instead of panicking with overflow checks. Its output and
// stderr are C 16.1.0's.
#[test]
fn huge_rank_and_node_separations_wrap_like_c() {
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { ranksep=\"2147483647\"; a -> b [label=x] }",
    );
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "graph 1 0.75 1.4556\nnode a 0.375 1.2056 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 0.375 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge a b 4 0.375 0.95274 0.375 0.86469 0.375 0.76237 0.375 0.66396 x 0.42361 0.72778 \
         solid black\nstop\n"
    );
    let line = "Error: Edge length 2147483674.000000 larger than maximum 2147483647 allowed.\n\
                Check for overwide node(s).\n";
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { margin=\"2147483647\"; subgraph cluster_x { a } b }",
    );
    assert!(!ok);
    assert_eq!(err, line.repeat(3));
    assert_eq!(
        out,
        "graph 1 2.9826e+07 5.9652e+07\n\
         node a 0.375 2.9826e+07 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 1.375 2.9826e+07 0.75 0.5 b solid ellipse black lightgrey\nstop\n"
    );
    // Two INT_MAX minimum lengths in a chain wrap the ranks; C fails `assert(Maxrank >= 0)`
    // in `TB_balance` (ns.c:823), and the port fails the layout there.
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { a -> b [minlen=2147483647]; b -> c [minlen=2147483647] }",
    );
    assert!(!ok);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "Error: <stdin>: Layout computation error in dot: network simplex: the ranks \
         overflowed, the highest rank is below the lowest\n"
    );
}

// Issue #233: inside cluster2, `rank=sink` and the nested `rank=min` set contradict each
// other, network simplex prints `trouble in init_rank` and leaves no node of the cluster on
// rank 0. C fails `assert(leader != NULL)` in `cluster_leader` (rank.c:316, exit 134). The
// port fails the layout where the cluster's ranking failed, for a top-level cluster and
// for a nested one.
#[test]
fn cluster_ranking_trouble_fails_the_layout() {
    for src in [
        "digraph G { subgraph cluster2 { rank=sink; k; f; { a } { rank=min; a; j } } \
         j -> f; f -> k; }",
        "digraph G { subgraph cluster1 { subgraph cluster2 { rank=sink; k; f; { a } \
         { rank=min; a; j } } } j -> f; f -> k; }",
    ] {
        let (_, err, ok) = run(&["-Kdot", "-Tsvg"], src);
        assert!(!ok, "{src}: {err}");
        assert_eq!(
            err,
            "Error: trouble in init_rank\n\tk 1\n\ta 1\n\tf 1\n\
             Error: <stdin>: Layout computation error in dot: cluster_leader: the ranking of \
             cluster cluster2 failed: none of its nodes is on rank 0\n",
            "{src}"
        );
    }
}

// Issue #232, input A: in the root's `build_ranks`, a later component's cluster rank
// leaders are installed past the end of the rank array. C's `install_in_rank` reports it
// (mincross.c:1190), `dot_layout` abandons the layout and the SVG has the nodes and no edge
// paths (exit 1). stderr and SVG are C 16.1.0's.
#[test]
fn install_in_rank_past_the_root_slots_abandons_the_layout_like_c() {
    let src = "digraph G {
rank=sink;
subgraph cluster0 { a; b; c; d }
subgraph cluster_1 { e; f }
{ d }
subgraph cluster9 { g; i }
{ e }
{ i }
f -> e;
g -> h;
i -> g;
d -> c;
i -> h [label=x];
{ rank=source; a; h }
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok, "{err}");
    assert_eq!(
        err,
        "Warning: a was already in a rankset, deleted from cluster G\n\
         Warning: c was already in a rankset, deleted from cluster G\n\
         Warning: d was already in a rankset, deleted from cluster G\n\
         Warning: e was already in a rankset, deleted from cluster G\n\
         Warning: i was already in a rankset, deleted from cluster G\n\
         Error: trouble in init_rank\n\
         \tc 2\n\t%0 1\n\tf 1\n\t%0 1\n\tg 2\n\t%0 1\n\
         Error: install_in_rank, line 1190: GD_rank(g)[2].v + ND_order(%0) [0] > \
         GD_rank(g)[2].av + GD_rank(Root)[2].an [2]\n"
    );
    assert!(
        out.contains("<ellipse fill=\"none\" stroke=\"black\" cx=\"1\" cy=\"0\""),
        "{out}"
    );
    assert!(!out.contains("<path"), "{out}");
}

// Issue #232, input B and a cluster route: a rank gets more nodes than it has slots. C
// stores past the array and fails `assert(GD_rank(g)[r].n <= GD_rank(g)[r].an)`
// (mincross.c:1165, exit 134); the port fails the layout before the store.
#[test]
fn install_in_rank_on_a_full_rank_fails_the_layout() {
    let root = "digraph G {
rank=sink;
subgraph cluster0 { a; b; c; d }
b -> f;
subgraph cluster_1 { e; f }
{ d }
{ e }
k -> a;
f -> e;
d -> c;
i -> h [label=x];
}
";
    let clust = "digraph G { subgraph cluster6 { b e c d subgraph cluster73 { e subgraph \
                 cluster69 { e { rank=source; a } } } subgraph cluster77 { b { rank=min; a } } \
                 } { rank=sink; b a c } c -> d [label=x]; }";
    for (src, want) in [
        (
            root,
            "Warning: c was already in a rankset, deleted from cluster G\n\
             Warning: d was already in a rankset, deleted from cluster G\n\
             Warning: e was already in a rankset, deleted from cluster G\n\
             Error: <stdin>: Layout computation error in dot: install_in_rank: rank 4 of G is \
             full (2 allocated), no slot for %0\n",
        ),
        (
            clust,
            "Warning: b was already in a rankset, deleted from cluster G\n\
             Warning: c was already in a rankset, deleted from cluster G\n\
             Warning: a was already in a rankset, deleted from cluster G\n\
             Error: <stdin>: Layout computation error in dot: install_in_rank: rank 0 of \
             cluster6 is full (1 allocated), no slot for %0\n",
        ),
    ] {
        let (_, err, ok) = run(&["-Kdot", "-Tsvg"], src);
        assert!(!ok, "{src}: {err}");
        assert_eq!(err, want, "{src}");
    }
}

// Issue #231: a cluster below the root loses all its nodes after it was registered: to an
// earlier sibling cluster (newrank's `set_parent` registers it before `node_induce`), or to
// a rank set, through `mark_clusters` at its parent's level (the second input empties
// `cluster4` by taking `b` from `cluster1`; the third panicked in `clust_ht`). C keeps the
// cluster, and `expand_cluster` -> `build_ranks` dereferences its NULL node list
// (mincross.c:1227, exit 139). The port fails the layout there.
#[test]
fn cluster_emptied_by_membership_rules_fails_the_layout() {
    let err_for = |c: &str| {
        format!(
            "Error: <stdin>: Layout computation error in dot: expand_cluster: cluster {c} has \
             no nodes left (a rank set or an earlier cluster took them)\n"
        )
    };
    for (src, want) in [
        (
            "digraph G { newrank=true; subgraph cluster1 { subgraph cluster2 { c } \
             subgraph cluster4 { c } } }",
            err_for("cluster4"),
        ),
        (
            "digraph G { concentrate=true; subgraph cluster1 { c; subgraph cluster4 { b } } \
             { rank=same; b } b -> d [minlen=3]; }",
            "Warning: b was already in a rankset, deleted from cluster G\n".to_string()
                + &err_for("cluster4"),
        ),
        (
            "digraph G { concentrate=true; subgraph cluster39 { e subgraph cluster10 { h d a b \
             { rank=same; c d f e } subgraph cluster73 { c { rank=max; a } { rank=min; a b } } \
             { rank=source; a f b d } } subgraph cluster29 { g f subgraph cluster95 { f e a g \
             { rank=source; a g h } } { g f h } } } g -> e; f -> e [label=x,constraint=false,\
             weight=0]; }",
            "Warning: a was already in a rankset, deleted from cluster cluster10\n\
             Warning: b was already in a rankset, deleted from cluster cluster10\n"
                .to_string()
                + &err_for("cluster73"),
        ),
        (
            "digraph G { rank=max; subgraph cluster47 { rank=sink; subgraph cluster75 { k; } \
             { rank=sink; f k } } { rank=source; d c } a; { rank=source; c } b -> j; }",
            "Warning: k was already in a rankset, deleted from cluster cluster47\n".to_string()
                + &err_for("cluster75"),
        ),
    ] {
        let (_, err, ok) = run(&["-Kdot", "-Tsvg"], src);
        assert!(!ok, "{src}: {err}");
        assert_eq!(err, want, "{src}");
    }
}

// A cluster keeps its rank range when `mark_clusters` takes the nodes of one of its ranks
// for a rank set. On that rank `save_vlist` saves the root's node just right of the
// cluster. C goes on with that node. Here it is `b` itself, so `cluster0`'s box reaches
// down to `b`'s rank. The port draws the same.
#[test]
fn cluster_rank_emptied_by_a_rank_set_draws_like_c() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph { subgraph cluster0 { a -> b } {rank=same; b; x} }",
    );
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: b was already in a rankset, deleted from cluster %1\n"
    );
    assert!(
        out.contains("points=\"8,-18 8,-116 78,-116 78,-18 8,-18\""),
        "{out}"
    );
}

// A rank of `cluster50` loses its node `g` to the rank set, and `save_vlist` saves for it
// the rank leader of `cluster38`, which `cluster38`'s expansion then frees. C reads the
// freed leader in `rec_reset_vlists` -> `furthestnode`. Here it segfaults; on other inputs
// it finishes, with output that is the same on every run, so the port does not stop there
// (see docs/porting/crashes.md). It must not panic.
#[test]
fn cluster_rank_saved_leader_freed_does_not_panic() {
    let src = "digraph G { subgraph cluster50 { c -> g; } { rank=min; g } subgraph cluster38 { \
               j -> a; b -> k; } f -> b; e -> i; k -> h; }";
    for fmt in ["-Tsvg", "-Tdot"] {
        let (_, err, _) = run(&["-Kdot", fmt], src);
        assert!(!err.contains("panicked"), "{fmt}: {err}");
        assert!(
            err.starts_with("Warning: g was already in a rankset, deleted from cluster G\n"),
            "{fmt}: {err}"
        );
    }
}

// The `d` attributes of an SVG's paths, in order.
fn svg_path_data(svg: &str) -> Vec<&str> {
    svg.split(" d=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect()
}

// Issue #230: a labelled flat edge on rank 0 that is not its class's representative (a
// parallel `a -> b` behind an unlabelled one, or a `b -> a` merged by `flat_rev`) sits in
// `ND_other`. C's `flat_edges` makes rank -1 only for `ND_flat_in` edges, then `flat_node`
// reads `GD_rank(g)[-1]` and segfaults. The port makes rank -1 for these edges too. The
// layout then fails as C's does for the same edges on a lower rank (the `z -> a` input,
// whose stderr and paths are C 16.1.0's).
#[test]
fn labelled_flat_multi_edge_on_rank_0_gets_a_label_rank() {
    let lost = "Error: in routesplines, illegal values of prev 1 and next 1, line 378\n\
                Error: lost a b edge\n";
    for (src, want) in [
        (
            "{ rank=same; a; b; c } a -> b; a -> b [label=x]; a -> c;",
            lost,
        ),
        (
            "rankdir=LR; { rank=same; a; b; c } a -> b; a -> b [label=x]; a -> c;",
            lost,
        ),
        (
            "ordering=out; { rank=same; a; b; c } a -> b; a -> b [label=x]; a -> c;",
            lost,
        ),
        (
            "newrank=true; rank=sink; a -> b; a -> b [label=x]; a -> c;",
            lost,
        ),
        (
            "{ rank=same; a; b; c } a -> b; b -> a [label=x]; a -> c;",
            "Error: in routesplines, illegal values of prev 1 and next 1, line 378\n\
             Error: lost b a edge\n",
        ),
        (
            "subgraph cluster0 { { rank=same; a; b; c } } a -> b; a -> b [label=x]; a -> c;",
            "",
        ),
    ] {
        let src = format!("digraph G {{ {src} }}");
        let (out, err, ok) = run(&["-Kdot", "-Tsvg"], &src);
        assert_eq!(err, want, "{src}");
        assert_eq!(ok, want.is_empty(), "{src}");
        assert!(!ok || out.contains(">x</text>"), "{src}: {out}");
    }
    let src = "digraph G { z -> a; { rank=same; a; b; c } a -> b; a -> b [label=x]; a -> c; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok);
    assert_eq!(err, lost);
    assert_eq!(
        svg_path_data(&out),
        [
            "M27,-88.65C27,-76.96 27,-61.19 27,-47.62",
            "M42.58,-33.16C54.48,-43.63 71.89,-56.66 90,-62.4 112.88,-69.65 121.8,-71.51 144,\
             -62.4 154.14,-58.24 163.22,-50.57 170.52,-42.76",
            "M54.42,-18C62.4,-18 70.37,-18 78.35,-18",
        ]
    );
}

// Issue #241: after `trouble in init_rank` nothing normalised the ranks, and the top rank
// is 1, not 0. `flat_edges` tested rank 0 for labelled flat edges, made no rank above the
// top one, and `flat_node` read a rank that was never allocated (C segfaults there). The
// port runs the test on the top rank, and the label is drawn.
#[test]
fn labelled_flat_edge_on_an_unnormalised_top_rank_gets_a_label_rank() {
    let src = "digraph G { { rank=min; b a e } { rank=max; e } a -> b [label=x,weight=0]; \
               c -> d; a -> c; d -> b; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok);
    assert_eq!(err, "Error: trouble in init_rank\n\tb 1\n\td 2\n\tc 1\n");
    assert!(out.contains(">x</text>"), "{out}");
    // C draws a labelled flat edge on a lower rank when the top rank is 2; stderr and
    // paths are C 16.1.0's.
    let src = "digraph G { { rank=min; b a e } { rank=max; e } c -> d; a -> c; d -> b; \
               { rank=same; c x y } c -> y [label=z]; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok);
    assert_eq!(err, "Error: trouble in init_rank\n\tb 1\n\td 2\n\tc 1\n");
    assert_eq!(
        svg_path_data(&out),
        [
            "M173,-75.97C161.46,-65.86 145.67,-52.02 132.55,-40.52",
            "M92.03,-33C80.5,-43.1 64.72,-56.94 51.58,-68.45",
            "M135.49,-18C142.99,-18 151.27,-18 159.27,-18",
            "M54.42,-91C62.4,-91 70.37,-91 78.35,-91",
        ]
    );
}

// Issue #247: with `concentrate=true`, `arrow_flags` gives an edge with `ED_conc_opp_flag`
// the arrowheads of an opposing edge, found by `agfindedge`. Among several opposing edges
// cgraph's splay tree returns the first on its search path, not the first created. The port
// took the first created, and lost an arrowhead and its clip, or drew the wrong type. Each
// input after the first two needs one more of the searches that shape the tree: the
// parser's search before each insert, `agcontains` in mincross (`ordering=out`), and
// `cccomps` (`pack`). In the last, a `key` names an edge, whose id sorts after every
// anonymous one. The expected lines are C 16.1.0's.
#[test]
fn concentrated_edge_takes_the_arrowheads_of_the_opposing_edge_cgraph_finds() {
    for (src, want) in [
        (
            "d -> f [tailport=sw,dir=back]; g -> d [tailport=n,minlen=2,dir=back]; \
             g -> d [headport=s]; g -> d; d -> g [minlen=4];",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M101.76,-206.18C102.36,-188.74 \
                 103.37,-166.07 105,-146 107.77,-112 112.62,-73.06 116.05,-47.36\"/>",
                "<polygon fill=\"black\" stroke=\"black\" points=\"98.27,-205.96 101.46,-216.07 \
                 105.27,-206.18 98.27,-205.96\"/>",
            ][..],
        ),
        (
            "d -> g; g -> d [arrowhead=dot]; g -> d [arrowhead=box];",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M27,-61.38C27,-56.85 27,-52.14 \
                 27,-47.59\"/>",
                "<polygon fill=\"black\" stroke=\"black\" points=\"31,-71.2 23,-71.2 23,-63.2 \
                 31,-63.2 31,-71.2\"/>",
            ][..],
        ),
        (
            "subgraph cluster1 { h; c } subgraph cluster2 { a label=C2; } b -> d; \
             b -> d [dir=back]; d -> b; a -> d; b -> d; c -> d;",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M154,-71.7C154,-64.41 154,-55.73 \
               154,-47.54\"/>",
            ][..],
        ),
        (
            "ordering=out; g -> h; h -> b; b -> h [dir=back]; b -> h [tailport=se]; \
             g -> h [label=x];",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M31.2,-62.22C30.73,-57.48 \
                 30.25,-52.58 29.78,-47.85\"/>",
                "<polygon fill=\"black\" stroke=\"black\" points=\"27.69,-62.34 32.16,-71.95 \
                 34.66,-61.65 27.69,-62.34\"/>",
            ][..],
        ),
        (
            "pack=true; subgraph cluster0 { d } b -> e; e -> b; b -> e [dir=none]; d -> e;",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M43,-71.7C43,-64.41 43,-55.73 \
               43,-47.54\"/>",
            ][..],
        ),
        (
            "a -> c; c -> a [key=zz]; c -> a [arrowhead=dot]; b -> a;",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M27,-60.24C27,-56.01 27,-51.66 \
                 27,-47.44\"/>",
                "<polygon fill=\"black\" stroke=\"black\" points=\"23.5,-60.18 27,-70.18 \
                 30.5,-60.18 23.5,-60.18\"/>",
            ][..],
        ),
        // `edge [key=d]` names nothing (`bindattrs` skips `key`), and a key starting with
        // `%` leaves the edge anonymous (`agmapnametoid`).
        (
            "a -> c; subgraph s { edge [key=d]; c -> a [arrowhead=inv]; } \
             c -> a [arrowhead=dot]; b -> a;",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M27,-62.8C27,-57.78 27,-52.5 \
                 27,-47.41\"/>",
                "<ellipse fill=\"black\" stroke=\"black\" cx=\"27\" cy=\"-67.2\" rx=\"4\" \
                 ry=\"4\"/>",
            ][..],
        ),
        (
            "a -> c; c -> a [key=\"%zz\"]; c -> a [arrowhead=dot]; b -> a;",
            &[
                "<path fill=\"none\" stroke=\"black\" d=\"M27,-62.8C27,-57.78 27,-52.5 \
                 27,-47.41\"/>",
                "<ellipse fill=\"black\" stroke=\"black\" cx=\"27\" cy=\"-67.2\" rx=\"4\" \
                 ry=\"4\"/>",
            ][..],
        ),
    ] {
        let src = format!("digraph G {{ concentrate=true; {src} }}");
        let (out, err, ok) = run(&["-Kdot", "-Tsvg"], &src);
        assert!(ok, "{src}: {err}");
        for w in want {
            assert!(out.contains(w), "{src}: {w}\n{out}");
        }
    }
    // Undirected: `dir=both` draws both arrowheads of the named edges' group.
    let src = "graph G { concentrate=true; b -- a [key=k3]; a -- b [arrowtail=inv,dir=none]; \
               a -- b [key=k1,dir=both]; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    for w in [
        "<path fill=\"none\" stroke=\"black\" d=\"M27,-60.24C27,-56.01 27,-51.66 27,-47.44\"/>",
        "<polygon fill=\"black\" stroke=\"black\" points=\"30.5,-47.62 27,-37.62 23.5,-47.62 \
         30.5,-47.62\"/>",
    ] {
        assert!(out.contains(w), "{w}\n{out}");
    }
    // `-Ekey=q` gives every edge a `key` attribute, but names none of them.
    let src = "digraph G { concentrate=true; a -> c; c -> a [key=zz]; c -> a [arrowhead=dot]; \
               b -> a; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg", "-Ekey=q"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "<polygon fill=\"black\" stroke=\"black\" points=\"23.5,-60.18 27,-70.18 \
             30.5,-60.18 23.5,-60.18\"/>"
        ),
        "{out}"
    );
}

// Issue #242: `rebuild_vlists` warns when concentration leaves a cluster no node of its own
// on a rank (`conc.c:187`). stderr is C 16.1.0's.
#[test]
fn concentrate_warns_about_a_degenerate_cluster_rank() {
    let src = "digraph G { rank=same; concentrate=true; compound=true; \
               subgraph cluster31 { d f } { rank=same; e } e -> f; \
               e -> a [minlen=1,constraint=false]; e -> f [weight=0]; e -> f; \
               e -> d [constraint=false]; d -> a [label=x]; e -> f; d -> f; }";
    let (_, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok);
    assert_eq!(err, "Warning: degenerate concentrated rank cluster31,1\n");
}

// Issue #234: with `concentrate=true`, a cluster edge with other ports shares its chain
// (`interclrep`) and is routed alone through `make_regular_edge`'s `hackflag`, which follows
// the chain as it was before `dot_concentrate` rewired it. Where the walk stops at a
// concentrator, C ends the path at the real head with the concentrator's merge flag
// (`dotsplines.c:1845`), and `conc_slope` on a head with no out-edges segfaults. Where
// `mergevirtual` deleted the chain's node, C walks from a node with no edges and segfaults
// in `straight_len` (the last input but one). The last input gives the head an out-edge: C
// draws it, and the port's paths are C 16.1.0's.
#[test]
fn concentrated_cluster_edge_routed_alone_does_not_panic() {
    for src in [
        "subgraph cluster1 { c } a -> c [minlen=2]; a -> c [tailport=s]; b -> c [constraint=false];",
        "rankdir=LR; subgraph cluster1 { c } a -> c [minlen=2]; a -> c [tailport=s]; \
         b -> c [constraint=false];",
        "newrank=true; subgraph cluster1 { c } a -> c [minlen=2]; a -> c [tailport=s]; \
         b -> c [constraint=false];",
        "ordering=out; subgraph cluster1 { c } a -> c [minlen=2]; a -> c [tailport=s]; \
         b -> c [constraint=false];",
        "subgraph cluster1 { c } a -> c [minlen=2]; a -> c [tailport=s]; b -> c [minlen=2];",
        "subgraph cluster1 { a } a -> c [minlen=2]; a -> c [tailport=s]; a -> d [minlen=2];",
        "subgraph cluster0 { c } a -> c [tailport=s,minlen=2]; a -> c; a -> c;",
    ] {
        let src = format!("digraph G {{ concentrate=true; {src} }}");
        let (out, err, ok) = run(&["-Kdot", "-Tsvg"], &src);
        assert!(ok, "{src}: {err}");
        assert_eq!(err, "", "{src}");
        assert!(out.contains("<title>a:s&#45;&gt;c</title>"), "{src}: {out}");
    }
    // The deleted-node route draws both edge groups from `a` to `c`, with arrowheads.
    let src = "digraph G { concentrate=true; subgraph cluster0 { c } \
               a -> c [tailport=s,minlen=2]; a -> c; a -> c; }";
    let (out, _, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok);
    assert_eq!(
        svg_path_data(&out),
        [
            "M43,-125C43,-104.43 43,-81.22 43,-63.53",
            "M43,-124.81C43,-108.27 43,-82.98 43,-63.44",
        ]
    );
    assert_eq!(out.matches("<polygon fill=\"black\"").count(), 2, "{out}");
    let src = "digraph G { concentrate=true; subgraph cluster1 { c } a -> c [minlen=2]; \
               a -> c [tailport=s]; b -> c [constraint=false]; c -> d; }";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        svg_path_data(&out),
        [
            "M43,-71.7C43,-64.41 43,-55.73 43,-47.54",
            "M45.46,-180.93C46.39,-170.36 46.49,-156.63 43,-145",
            "M43,-144C40.65,-136.18 39.93,-127.42 39.99,-119.31",
            "M43,-181C43,-152.52 43,-139.25 43,-119.54",
            "M94.84,-186.89C74.83,-175.29 46.64,-157.11 43,-145",
        ]
    );
}

// Fuzz case 018-render-interpolate-points-fma (issue #27), its repro.gv verbatim. C's
// `interpolate_pointf` (`lib/common/geomprocs.h:115`, called by `alloc_interpolation_points`
// in `lib/common/shapes.c`) computes `p + t * (q - p)`, which Apple clang fuses (`fmla.2d`).
// Unfused, the rarrow's shaft corners round to 10.48 instead of C's 10.47.
#[test]
fn round_corners_interpolation_is_fused_like_clang() {
    let src = "digraph G{layout=neato; notranslate=true; \"n45\" [shape=\"rarrow\", height=\"1.73\", fontsize=\"27.3\", pos=\"0,0!\"];}\n";
    let (out, err, ok) = run(&["-Kneato", "-Tsvg"], src);
    assert!(ok, "{err}");
    let c_points = "points=\"10.47,-56.28 -28.48,-56.28 -28.48,56.28 10.47,56.28 \
                    10.47,62.28 28.48,0 10.47,-62.28 10.47,-56.28\"";
    assert!(out.contains(c_points), "{out}");
}

// Issue #115: `optmacroname : atom '='` (grammar.y). `attrstmt` warns, without a newline,
// and applies the list as usual. C 16.1.0's stderr and -Tcanon.
#[test]
fn attribute_macro_name_warns_and_applies_the_list() {
    let (out, err, ok) = run(&["-Tcanon"], "digraph { node a = [color=red] b }");
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: attribute macros not implemented");
    assert_eq!(
        out,
        "digraph {\n\tnode [color=red,\n\t\tlabel=\"\\N\"\n\t];\n\tb;\n}\n"
    );
}

// Issue #297: an edge's key is its cgraph name, which `agwrite` prints first
// (`write_edge_name`), and again alone where the edge is written a second time; `-Tjson`
// prints declared attributes only, so never `key`. C 16.1.0's output.
#[test]
fn edge_key_is_the_edges_name_in_the_writers() {
    let src =
        "digraph G { subgraph s { e -> f [key=q, color=red] } subgraph t { e -> f [key=q] } }";
    let (out, err, ok) = run(&["-Tcanon"], src);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "digraph G {\n\tnode [label=\"\\N\"];\n\tsubgraph s {\n\t\te -> f\t[key=q,\n\t\t\tcolor=red];\n\t}\n\
         \tsubgraph t {\n\t\te -> f\t[key=q];\n\t}\n}\n"
    );
    let (out, _, _) = run(&["-Txdot"], "digraph G { e -> f [key=q]; }");
    assert!(out.contains("\te -> f\t[key=q,\n\t\t_draw_="), "{out}");
    let (out, _, _) = run(&["-Tjson"], "digraph G { e -> f [key=q]; }");
    assert!(!out.contains("\"key\""), "{out}");
}

// Issue #303: `subgraph ""` and `subgraph <>` are named (grammar.y `opensubg` passes the
// name to `agsubg`), so `write_hdr` writes them. C 16.1.0's -Tcanon.
#[test]
fn subgraph_named_by_the_empty_string_is_written() {
    let (out, err, ok) = run(
        &["-Tcanon"],
        "digraph { subgraph \"\" { a } subgraph <> { b } }",
    );
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "digraph {\n\tnode [label=\"\\N\"];\n\tsubgraph \"\" {\n\t\ta;\n\t}\n\tsubgraph <> {\n\t\tb;\n\t}\n}\n"
    );
    let (out, _, _) = run(&["-Tcanon"], "digraph \"\" { }");
    assert!(out.starts_with("digraph \"\" {\n"), "{out}");
}

// Issue #118: nesting is limited by bison's stack (`YYMAXDEPTH`), as in C 16.1.0: 3331
// anonymous levels are laid out; at 3332 `aagparse` stops with `memory exhausted`, and
// `agread` returns the graph read so far, which dot lays out and writes, exit status 1.
#[test]
fn deep_subgraph_nesting_stops_where_bisons_stack_does() {
    let nest = |n: usize| format!("digraph G {{{}a{}}}", "{".repeat(n), "}".repeat(n));
    let (out, err, ok) = run(&["-Tsvg"], &nest(3331));
    assert!(ok, "{err}");
    assert!(out.contains("<title>a</title>"), "{out}");
    let (out, err, ok) = run(&["-Tcanon"], &nest(3332));
    assert!(!ok);
    assert_eq!(err, "Error: <stdin>: memory exhausted in line 1 near 'a'\n");
    assert_eq!(out, "digraph G {\n\tnode [label=\"\\N\"];\n}\n");
    // The next `agread` starts on the next line, in the middle of the statement, its lines
    // counted from 1.
    let (out, err, ok) = run(
        &["-Tcanon"],
        &format!("{}\n{{\n}}\ndigraph Z {{ z }}\n", nest(3332)),
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: <stdin>: memory exhausted in line 1 near 'a'\n\
         Error: <stdin>: syntax error in line 1 near '{'\n"
    );
    assert_eq!(out, "digraph G {\n\tnode [label=\"\\N\"];\n}\n");
}

/// Issue #29 (was open case `tests/fuzz-open/020-parser-keyword-as-attribute-value`).
/// scan.l lexes `node`, `edge`, `graph`, `digraph`, `subgraph` and `strict`
/// (case-insensitively) as keyword tokens, and grammar.y's `atom : T_atom | qatom`
/// takes none of them, so C 16.1.0 reports a syntax error, writes no output and
/// exits 1 when one appears unquoted as an attribute name, an attribute value or a
/// port. The first input is the case's repro.gv verbatim; C says
/// `syntax error in line 2 near 'node'`. The message wording is issue #31.
#[test]
fn unquoted_keyword_atoms_are_syntax_errors() {
    for src in [
        "digraph G {\npackmode=node;\na [label=edge];\na:graph -> b;\n}\n",
        "digraph G { a [label=Edge] }\n",
        "digraph G { a [node=1] }\n",
        "digraph G { a:graph -> b }\n",
        "digraph G { a -> b:c:SUBGRAPH }\n",
    ] {
        let (out, err, ok) = run(&["-Tsvg"], src);
        assert!(!ok, "{src:?} should fail");
        assert!(out.is_empty(), "{src:?} wrote output");
        assert!(err.contains("syntax error"), "{src:?}: {err}");
    }
}

/// Issue #29: the quoted keywords are `T_qatom` in C and keep working. The
/// `-Tdot` writer quotes them again (`agcanonstr`), so the output reads back.
#[test]
fn quoted_keyword_atoms_still_parse() {
    let src = "digraph G {\npackmode=\"node\";\na [label=\"edge\", \"node\"=\"1\"];\na:\"graph\" -> b;\n}\n";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok, "{err}");
    assert!(out.contains("packmode=\"node\""), "{out}");
    assert!(out.contains("label=\"edge\""), "{out}");
    assert!(out.contains("\"node\"=1"), "{out}");
    assert!(out.contains("a:\"graph\" -> b"), "{out}");
}

// Issue #170 (cargo-fuzz render_circo): a route segment that lies in no channel of the
// ortho maze. C's chanSearch (lib/ortho/ortho.c:388/390) fails `assert (chani)` or
// `assert (cp)` (here `cp`) and aborts (exit 134, no output). The port used to panic on the
// same lookup ("chanSearch: no channel", exit 101); it now abandons ortho routing, as C's
// orthoEdges does when shortPath or assignTracks fails, and the edges are drawn as straight
// lines.
const ISSUE170_GRAPH: &str = "graph{layout=circo
oneblock=true
splines=ortho
t5--t12--t13
t6--t14}
";

#[test]
fn circo_ortho_segment_in_no_channel_does_not_panic_chan_search() {
    let (out, err, ok) = run(&["-Tsvg"], ISSUE170_GRAPH);
    assert!(
        ok,
        "C crashes here, the port falls back to straight lines: {err}"
    );
    assert_eq!(
        err,
        "Warning: orthogonal routing found no channel for an edge segment - \
         edges not routed orthogonally\n"
    );
    assert_eq!(out.matches("class=\"edge\"").count(), 3, "{out}");
    // Straight edges: each path is a single cubic, not an ortho polyline.
    let paths: Vec<&str> = out
        .split("<path fill=\"none\" stroke=\"black\" d=\"")
        .skip(1)
        .map(|p| p.split('"').next().unwrap())
        .collect();
    assert_eq!(paths.len(), 3, "{out}");
    for d in paths {
        assert_eq!(d.matches('C').count(), 1, "{d}");
    }
}

// Issue #31, fuzz case 022-stderr-syntax-error-message: a DOT syntax error is reported in
// the words of cgraph's `aagerror` (lib/cgraph/scan.l), on the line where bison read the
// rejected lookahead token, and dot exits 1. Expected stderr is C Graphviz 16.1.0's.
#[test]
fn syntax_errors_are_reported_as_c_does() {
    // repro.gv: the line is the end of input, where the unterminated string ends
    let path = tmp("cli_syntax_repro.gv");
    std::fs::write(&path, "digraph G {\n a -> b\n c [label=\"abc\n\n]\n d\n").unwrap();
    let (out, err, ok) = run(&["-Tsvg", path.to_str().unwrap()], "");
    assert!(!ok && out.is_empty());
    assert_eq!(
        err,
        format!(
            "Error: {}: syntax error in line 7 scanning a quoted string (missing endquote? \
             longer than 16384?)\nString starting:\"abc\n\n]\n d\n\n",
            path.display()
        )
    );
    // repro2.gv, read from stdin
    let (_, err, ok) = run(&["-Tsvg"], "digraph G {\n a -> b\n c -> ;\n}\n");
    assert!(!ok);
    assert_eq!(err, "Error: <stdin>: syntax error in line 3 near ';'\n");
    let (_, err, ok) = run(&["-Tsvg"], "digraph G {\n a -> b\n");
    assert!(!ok);
    assert_eq!(err, "Error: <stdin>: syntax error in line 3\n");
    // a `# N "file"` line directive renames the input
    let (_, err, ok) = run(&["-Tsvg"], "digraph G {\n# 100 \"other.gv\"\n a -> ;\n}\n");
    assert!(!ok);
    assert_eq!(err, "Error: other.gv: syntax error in line 100 near ';'\n");
    // the graphs before the error are rendered; line numbers restart with each graph
    let (out, err, ok) = run(&["-Tsvg"], "digraph G { a }\ndigraph H { b -> ; }\n");
    assert!(!ok);
    assert_eq!(out.matches("<svg").count(), 1);
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near ';'\n");
}

// Issue #199, reduced from `tests/golden/sfdp/attr_label_scheme_4_overlap.dot`. sfdp throws
// the nodes out to ~1e143 (C too), and a segment's trapezoid then has no upper neighbour in
// `lib/ortho/trapezoid.c:update_trapezoid`. C reads `LIST_GET(tr, SIZE_MAX)`: depending on
// the run it fails the `gv_list_get_` bounds assertion (exit 134) or goes on to fail
// `add_segment` ("trapezoid segment construction failed") and draws the edges straight. The
// port panicked; now `add_segment` fails and the edges fall back to straight lines.
#[test]
fn ortho_trapezoid_without_upper_neighbour_does_not_panic() {
    let src = "graph {
	label_scheme=4 layout=sfdp overlap=false splines=ortho
	a -- \"|edgelabel|ab\"
	\"|edgelabel|ab\" -- b
	b -- \"|edgelabel|bc\"
	\"|edgelabel|bc\" -- c
	\"|edgelabel|ca\" [width=1.6798]
	c -- \"|edgelabel|ca\"
	\"|edgelabel|ca\" -- a
	e
	d -- e
	d -- \"|edgelabel|da\"
	e -- c
	\"|edgelabel|da\" -- a
	f -- g
	g -- \"|edgelabel|ga\"
	\"|edgelabel|ga\" -- a
}
";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok && !err.contains("panicked"), "{err}");
    let edges: Vec<&str> = out.lines().filter(|l| l.contains(" -- ")).collect();
    assert_eq!(edges.len(), 13, "{out}");
    for e in edges {
        let pos = e.split("pos=\"").nth(1).unwrap().trim_end_matches("\"];");
        assert_eq!(pos.split(' ').count(), 4, "a straight line: {e}");
    }
    // Only the fallback draws a diagonal; an ortho route is axis-aligned.
    let ab = out
        .lines()
        .find(|l| l.contains("a -- \"|edgelabel|ab\""))
        .unwrap();
    let pts: Vec<Vec<&str>> = ab
        .split("pos=\"")
        .nth(1)
        .unwrap()
        .trim_end_matches("\"];")
        .split(' ')
        .map(|p| p.split(',').collect())
        .collect();
    assert!(
        pts[0][0] != pts[3][0] && pts[0][1] != pts[3][1],
        "a diagonal: {ab}"
    );
}

// `%.80s` of the string read so far may end inside a UTF-8 sequence: C writes the bytes.
#[test]
fn unterminated_string_excerpt_is_cut_at_80_bytes() {
    let src = format!("digraph G {{\n a [label=\"x{}\n", "\u{e9}".repeat(43));
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .arg("-Tsvg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let mut expected = b"Error: <stdin>: syntax error in line 3 scanning a quoted string \
        (missing endquote? longer than 16384?)\nString starting:\"x"
        .to_vec();
    expected.extend("\u{e9}".repeat(39).as_bytes());
    expected.extend(b"\xc3\n");
    assert_eq!(out.stderr, expected);
}

// cgraph's `default_usererrf` (lib/cgraph/agerror.c) writes control characters other than
// white space as `\ooo`: in the near text, the string excerpt and a directive's file name.
#[test]
fn syntax_error_escapes_control_characters() {
    let (_, err, ok) = run(&["-Tsvg"], "digraph { a \u{1} }");
    assert!(!ok);
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near '\\001'\n");
    let (_, err, _) = run(&["-Tsvg"], "digraph { a \u{7f} }");
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near '\\177'\n");
    let (_, err, _) = run(&["-Tsvg"], "digraph {\n a [label=\"a\u{1b}b\tc\r\n");
    assert_eq!(
        err,
        "Error: <stdin>: syntax error in line 3 scanning a quoted string (missing endquote? \
         longer than 16384?)\nString starting:\"a\\033b\tc\r\n\n"
    );
    let (_, err, _) = run(&["-Tsvg"], "digraph {\n# 3 \"a\u{1}b\"\n a -> ;\n}\n");
    assert_eq!(err, "Error: a\\001b: syntax error in line 3 near ';'\n");
}

/// A byte-order mark is ignored wherever it stands alone (scan.l:125), and is part of a name
/// when a letter or digit follows it: `LETTER` holds \200-\377, so `{NAME}` is the longer
/// match (#117).
#[test]
fn byte_order_mark_follows_scan_l() {
    let (out, err, ok) = run(&["-Tcanon"], "digraph { \u{feff} a \u{feff}}");
    assert!(ok, "{err}");
    assert_eq!(out, "digraph {\n\tnode [label=\"\\N\"];\n\ta;\n}\n");
    let (_, err, ok) = run(&["-Tcanon"], "\u{feff}digraph { a }");
    assert!(!ok);
    assert_eq!(
        err,
        "Error: <stdin>: syntax error in line 1 near '\u{feff}digraph'\n"
    );
}

/// cgraph reads its input with `fgets` and `strlen` (`lib/cgraph/io.c:24`), so a NUL hides the
/// rest of its line, newline included, and a line that starts with a NUL ends the input (#120).
#[test]
fn nul_hides_the_rest_of_its_line() {
    let (_, err, _) = run(&["-Tcanon"], "digraph { a \0 b -> ; }\n c -> ;");
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near ';'\n");
    let (_, err, _) = run(&["-Tcanon"], "digraph { \"x\0y\" -> ; }");
    assert_eq!(
        err,
        "Error: <stdin>: syntax error in line 1 scanning a quoted string (missing endquote? \
         longer than 16384?)\nString starting:\"x\n"
    );
    // The next graph is read from the line after the closing brace, as fgets reads it.
    let (out, err, ok) = run(
        &["-Tcanon"],
        "digraph{a\0junk\n}\ndigraph{b}\n\0\ndigraph{c}\n",
    );
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "digraph {\n\tnode [label=\"\\N\"];\n\ta;\n}\ndigraph {\n\tnode [label=\"\\N\"];\n\tb;\n}\n"
    );
}

/// The scanner reads a token when the grammar asks for one, as bison calls `yylex`: once a
/// graph is open (`startgraph`), nothing after a syntax error is scanned (`endgraph` pushes
/// EOF), and each graph's tokens are scanned once. Before the header, bison's error recovery
/// discards tokens to the end of the input, so the scanner warns about all of them. C prints
/// those warnings after the error; the port prints the error last (#278).
#[test]
fn scanner_warnings_come_from_the_tokens_c_reads() {
    let (_, err, _) = run(&["-Tcanon"], "digraph { a -> ; 2x }\n3y\n");
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near ';'\n");
    let (_, err, ok) = run(&["-Tcanon"], "digraph { a }\ndigraph { 2x }\n");
    assert!(ok);
    assert_eq!(
        err,
        "Warning: syntax ambiguity - badly delimited number '2x' in line 1 of <stdin> splits \
         into two tokens\n"
    );
    // `@` is scan.l's end of input: recovery stops there.
    let (_, err, _) = run(&["-Tcanon"], "strict @ 2x\n");
    assert_eq!(err, "Error: <stdin>: syntax error in line 1 near '@'\n");
    let (_, err, _) = run(&["-Tcanon"], "; 2x\n3y\n");
    let mut lines: Vec<_> = err.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "Error: <stdin>: syntax error in line 1 near ';'",
            "Warning: syntax ambiguity - badly delimited number '2x' in line 1 of <stdin> splits \
             into two tokens",
            "Warning: syntax ambiguity - badly delimited number '3y' in line 2 of <stdin> splits \
             into two tokens",
        ]
    );
}

/// C: `Warning: syntax ambiguity - badly delimited number '1a' in line 2 of <stdin> splits
/// into two tokens` before the error (scan.l `chkNum`), which splits the token in two (#97).
#[test]
fn badly_delimited_number_warns() {
    let (_, err, _) = run(&["-Tsvg"], "digraph G {\n a -> b -> 1a ; c -> ;\n}\n");
    assert_eq!(
        err,
        "Warning: syntax ambiguity - badly delimited number '1a' in line 2 of <stdin> splits \
         into two tokens\nError: <stdin>: syntax error in line 2 near ';'\n"
    );
    // A line directive's file name, and a second `.`, as `twoDots` sees it.
    let (out, err, ok) = run(&["-Tcanon"], "# 7 \"foo.gv\"\ndigraph { 1.2.3 }\n");
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: syntax ambiguity - badly delimited number '1.2.' in line 7 of foo.gv splits \
         into two tokens\n"
    );
    assert!(out.contains("\t1.2;\n\t.3;\n"), "{out}");
}

/// C: a backslash before CR LF in a quoted string is kept with the CR and LF (scan.l:140
/// ignores only a backslash before LF; rule 142 keeps the backslash) (#112).
#[test]
fn backslash_crlf_in_quoted_string_is_kept() {
    let (out, err, ok) = run(&["-Tdot"], "digraph G { a [label=\"x\\\r\ny\"] }");
    assert!(ok, "{err}");
    assert!(out.contains("label=\"x\\\r\ny\""), "{out}");
}

// Fuzz case for issue #171, verbatim. osage puts `i1ortho` at x=inf (C does too: the
// repeated `cluster_inner4`, see #173). C overruns `vpos[4]` in
// `lib/ortho/partition.c:make_new_monotone_poly` and then fails `assert(np->cells[1])` in
// `lib/ortho/maze.c:chkSgraph` (exit 134); the port panicked with an index out of bounds in
// the partition. Now the maze fails at `chkSgraph`, and the edges fall back to straight lines
// (`lib/neatogen/neatosplines.c:spline_edges_`).
#[test]
fn ortho_partition_with_non_finite_node_does_not_panic() {
    let src = "digraph{layout=osage
splines=ortho{label=\"inner\"{o2{i1ortho
cluster_outer{o1}subgraph cluster_inner4{j1 j2}}subgrapa}subgraph cluster_inner4{j2}}x i2 i3}
";
    let (_, err, ok) = run(&["-Tsvg"], src);
    assert!(ok && !err.contains("panicked"), "{err}");

    let src = src.replace("x i2 i3}", "x->i2 i3}");
    let (out, err, ok) = run(&["-Tdot"], &src);
    assert!(ok && !err.contains("panicked"), "{err}");
    assert!(
        out.contains("x -> i2\t[pos=\"e,153.78,58 85.298,58 102.1,58 123.89,58 142.44,58\"]"),
        "the edge falls back to a straight line:\n{out}"
    );
}

// Issue #202, verbatim. Tiny touching dot nodes make `lib/ortho/maze.c:markSmall` follow a
// missing cell or read past a node cell's side list; C segfaults (exit 139). The port
// panicked on an `unwrap` in `mark_small`; now the maze fails like the other cases C crashes
// on (see #171 above), and the ortho router gives up. What dot does after a failed maze
// (exit status, nodes-only output) is #200's, so this checks only that nothing panics, and
// that a successful run draws the nodes.
#[test]
fn ortho_tiny_touching_nodes_do_not_panic_mark_small() {
    let src = "digraph{splines=ortho nodesep=0.02 ranksep=0.02 node[shape=circle width=0.01 height=0.02 fixedsize=true label=\"\"] n0->n3 n1->n1 n2->n3 n1->n1 n3->n3 n0->n0}
";
    let out = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .arg("-Tsvg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(src.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    let (svg, err) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_ne!(out.status.code(), Some(101), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    if out.status.success() {
        for n in ["n0", "n1", "n2", "n3"] {
            assert!(svg.contains(&format!("<title>{n}</title>")), "{n}:\n{svg}");
        }
    }
}

// The ortho failure boundary (#228, #229): every failure inside `lib/ortho` ends at
// `orthoEdges`, which handles it the way C handles the failure it detects at that stage.
// #229: neato pins 1e12 inches apart make the maze's cells 7.2e13 points wide, so the
// distances in `shortPath` (`lib/ortho/sgraph.c`) do not fit an `int`. C fails
// `PQcheck`'s assertion (exit 134); the port overflowed `N_VAL(n) *= -1`. Now the
// routing fails like C's `goto orthofinish`: a warning, and neato draws the edge straight.
#[test]
fn ortho_distances_beyond_int_fall_back_to_straight_edges() {
    for (engine, far) in [("neato", "1e12"), ("neato", "1e8"), ("fdp", "1e12")] {
        let src = format!(
            "graph {{ layout={engine}; pack=false; splines=ortho; a [pos=\"0,0!\"]; x [pos=\"{far},0!\"]; a -- x }}"
        );
        let (out, err, ok) = run(&["-Tdot"], &src);
        assert!(ok && !err.contains("panicked"), "{engine} {far}: {err}");
        assert!(
            err.contains("orthogonal routing failed - edges not routed orthogonally"),
            "{engine} {far}: {err}"
        );
        let pos =
            edge_pos(&out, "a -- x").unwrap_or_else(|| panic!("{engine} {far}: no edge:\n{out}"));
        assert_eq!(
            pos.split_whitespace().count(),
            4,
            "a straight 4-point edge: {pos}"
        );
    }
}

/// The `pos` of the edge `name` in `-Tdot` output, whitespace-normalised (the writer wraps
/// long attribute lists).
fn edge_pos(dot: &str, name: &str) -> Option<String> {
    let rest = &dot[dot.find(&format!("{name}\t"))?..];
    let rest = &rest[rest.find("pos=\"")? + 5..];
    let pos = &rest[..rest.find('"')?];
    Some(pos.split_whitespace().collect::<Vec<_>>().join(" "))
}

// #228: a NaN or infinite pin. In C, `monotonate_trapezoids` (`lib/ortho/partition.c`)
// finds no trapezoid inside the polygon and fails `LIST_GET`'s bounds assertion (exit
// 134). The port fails the maze there, as C's `mkMaze` returning NULL.
#[test]
fn ortho_non_finite_pins_do_not_panic() {
    for pin in ["nan,0", "inf,0", "0,-inf"] {
        let src = format!(
            "graph {{ layout=neato; pack=false; splines=ortho; a [pos=\"{pin}!\"]; x [pos=\"1,0!\"]; a -- x; b }}"
        );
        let (out, err, ok) = run(&["-Tdot"], &src);
        assert!(ok && !err.contains("panicked"), "{pin}: {err}");
        // The maze fails, so neato draws the edge without ortho routing.
        assert!(edge_pos(&out, "a -- x").is_some(), "{pin}: no edge:\n{out}");
    }
}

// Degenerate inputs through every engine that routes ortho edges, plus pack: zero-size and
// point nodes, self-loops, coincident pins, clusters, touching tiny nodes and huge pins.
// Whatever C does (it aborts or segfaults on several of these), the port must not panic.
#[test]
fn ortho_degenerate_inputs_do_not_panic_in_any_engine() {
    let bodies = [
        "node[shape=box width=0 height=0 fixedsize=true label=\"\"] a->b b->c c->a a->c",
        "node[shape=point width=0 height=0] a->b a->b b->c c->c d->a",
        "a->a a->a a->a b->b a->b b->a c->c",
        "a [pos=\"0,0!\"] b [pos=\"0,0!\"] c [pos=\"0,0!\"] a->b b->c c->a a->a",
        "subgraph cluster_x { a b c } subgraph cluster_y { d e } a->d b->e c->c e->e a->b",
        "nodesep=0.02 ranksep=0.02 node[shape=circle width=0.01 height=0.01 fixedsize=true label=\"\"] a->b b->c c->a a->a b->b d->d d->a",
        "a [pos=\"0,0!\"] b [pos=\"1e12,0!\"] c [pos=\"0,1e12!\"] a->b b->c c->a",
    ];
    for engine in ["dot", "neato", "fdp", "twopi", "circo", "osage"] {
        for pack in ["false", "true"] {
            for body in bodies {
                let src =
                    format!("digraph {{ layout={engine} splines=ortho pack={pack}; {body} }}");
                let (_, err, _) = run(&["-Tsvg"], &src);
                assert!(!err.contains("panicked"), "{src}\n{err}");
            }
        }
    }
}

// Issue #171: tiny touching dot nodes also make a vertex need a fifth chain slot in
// `lib/ortho/partition.c:make_new_monotone_poly`. C writes past `vpos[4]`, but only into
// bookkeeping, so its partition and routes are still right (exit 0). The port must route the
// same, not fail the maze.
#[test]
fn ortho_partition_chain_slot_overflow_routes_like_c() {
    let src = "digraph{splines=ortho nodesep=0 ranksep=0.02 node[shape=point width=0.02 height=0.01 fixedsize=true label=\"\"]
n7->n7 n4->n0 n11->n1 n9->n4 n10->n7 n11->n1 n11->n4 n0->n4 n3->n11 n4->n11
n10->n9 n0->n2 n5->n10 n8->n4 n1->n11 n10->n8 n3->n10 n4->n1 n1->n6 n3->n0}
";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok && !err.contains("panicked"), "{err}");
    assert!(
        out.contains("n4 -> n0\t[pos=\"e,39.816,20 36.103,52 38.215,52 72,52 72,52 72,52 72,20 72,20 72,20 51.329,20 51.329,20\"]"),
        "n4 -> n0 must be routed orthogonally, as C does:\n{out}"
    );
}

// C8 (#150): a node without a position ends only its graph's layout (`neato_layout`
// returns after init_nop's two-line error); `dot` draws it and goes on to the next graph.
// stdout, stderr and exit status are Graphviz 16.1.0's.
#[test]
fn nop_missing_position_ends_only_that_graph() {
    let (out, err, ok) = run(
        &["-Kneato", "-n", "-Tplain"],
        "graph A { a; b [pos=\"0,0\"] }\ngraph B { c [pos=\"1,1\"] }\n",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node a in graph A has no position\nas required by the -n flag\n"
    );
    assert_eq!(
        out,
        "graph 1 0 0\nnode a 0 0 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 0 0 0.75 0.5 b solid ellipse black lightgrey\nstop\n\
         graph 1 0.75 0.5\nnode c 0.375 0.25 0.75 0.5 c solid ellipse black lightgrey\nstop\n"
    );
}

// C8 (#300): -n runs neato_init_node_edge's user_pos, which reports a `pos` it cannot
// read before init_nop reports the node as unplaced.
#[test]
fn nop_reports_an_unreadable_pos_like_c() {
    let (_, err, ok) = run(
        &["-Kneato", "-n", "-Tplain"],
        "graph G { a [pos=\" 3 , 4 \"]; b [pos=\"1,1\"]; a -- b }",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node a, position  3 , 4 , expected two doubles\n\
         Error: node a in graph G has no position\nas required by the -n flag\n"
    );
}

// C8 (#317): with -n3, spline_edges_ keeps the first bezier of the edge's `pos`, warns
// about the others, and clips and installs it again.
#[test]
fn nop3_keeps_the_first_bezier_and_warns() {
    let (out, err, ok) = run(
        &["-Kneato", "-n3", "-Tplain"],
        "graph G { a [pos=\"0,0\"]; b [pos=\"100,100\"]; \
         a -- b [pos=\"0,0 10,10 20,20 30,30;40,40 50,50 60,60 70,70\"] }",
    );
    assert!(ok);
    assert_eq!(
        err,
        "Warning: edge a -> b : set more than one spline. First used, other dropped.\n"
    );
    assert!(
        out.contains(
            "edge a b 4 0.58333 0.45833 0.65061 0.52561 0.71788 0.59288 0.78516 0.66016 solid"
        ),
        "{out}"
    );
}

// C8: with -n3, a straight edge between overlapping nodes is a single point, which C's
// clip_and_install reads past the end of (it prints garbage). The port keeps the point.
#[test]
fn nop3_single_point_spline_does_not_panic() {
    let (out, err, ok) = run(
        &["-Kneato", "-n3", "-Tplain"],
        "graph { splines=polyline; node [width=2.46]; a [height=1.47, pos=\"88.56,143.48\"]; \
         b [pos=\"132.58,86.155\"]; b -- a; b -- a }",
    );
    assert!(ok && !err.contains("panicked"), "{err}");
    assert!(
        out.contains("edge b a 1 1.6244 0.34779 solid black"),
        "{out}"
    );
}

// C8: with -n2, orthoEdges skips an edge that has its spline (`Nop == 2 && ED_spl(e)`),
// and spline_edges_ keeps it; the other edges are routed orthogonally. Graphviz 16.1.0.
#[test]
fn nop2_ortho_keeps_a_given_spline() {
    let (out, err, ok) = run(
        &["-Kneato", "-n2", "-Tplain"],
        "digraph G { splines=ortho; a [pos=\"0,0\"]; b [pos=\"150,0\"]; c [pos=\"200,150\"]; \
         a -> c [pos=\"e,190,140 20,10 60,60 120,100 180,130\"]; b -> c; a -> b [label=ab]; }",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "edge a c 4 0.65278 0.38889 1.2083 1.0833 2.0417 1.6389 2.875 2.0556 solid black\n\
             edge b c 7 2.4306 0.50546 2.4306 1.0618 2.4306 2.3333 2.4306 2.3333 2.4306 2.3333 \
             2.6148 2.3333 2.6148 2.3333 solid black\n"
        ),
        "{out}"
    );
}

// C8: with -n2, splineEdges puts an edge that has its spline in a class of its own, so
// the other multi-edges are spread as if it were not there. Graphviz 16.1.0.
#[test]
fn nop2_given_spline_is_its_own_edge_class() {
    let (out, err, ok) = run(
        &["-Kneato", "-n2", "-Tplain"],
        "digraph G { a [pos=\"0,0\"]; b [pos=\"100,0\"]; a -> b; a -> b; a -> b [label=m]; \
         b -> a; a -> b [pos=\"e,80,0 20,0 40,10 60,10 70,0\"]; }",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "edge a b 4 0.59201 0.46094 0.80707 0.53836 1.1405 0.55184 1.3968 0.50139 solid black\n\
             edge a b 4 0.59201 0.039062 0.80707 -0.038359 1.1405 -0.051844 1.3968 -0.0013913 solid black\n\
             edge a b 4 0.73267 0.16683 0.88982 0.15612 1.0773 0.15354 1.2491 0.1591 m 0.91525 0.27963 solid black\n\
             edge a b 4 0.65278 0.25 0.93056 0.38889 1.2083 0.38889 1.3472 0.25 solid black\n"
        ),
        "{out}"
    );
}

// C8: when spline_edges0 returns before splineEdges (splines=""), State stays below
// GVSPLINES and attach_attrs_and_arrows writes no edge position: -n3 keeps the input `pos`
// and `lp`, untranslated.
#[test]
fn nop3_without_splines_keeps_the_input_edge_attributes() {
    let (out, err, ok) = run(
        &["-Kneato", "-n3", "-Tdot"],
        "graph { splines=none; a [pos=\"0,0\"]; b [pos=\"100,100\"]; \
         a -- b [pos=\"0,0 10,10 20,20 30,30\" label=L lp=\"15,15\"] }",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("\ta -- b\t[label=L,\n\t\tlp=\"15,15\",\n\t\tpos=\"0,0 10,10 20,20 30,30\"];"),
        "{out}"
    );
}

/// Fuzz case 019-neato-polyline-compass-port-endpoint (issue #28), minimised to fixed
/// positions. The head port `10:n` lies on the node boundary, so `spline_edges_` sends the
/// edge to the GTS router (`makeMultiSpline`, `lib/neatogen/multispline.c`). The two
/// stacked octagon obstacles are mirror images, so the shortest triangle path from the
/// head can pass either side of the `9`-bottom/`10`-top diagonal at an equal length.
///
/// Expected (C 16.1.0): in `triPath` both head edges have exactly the same length
/// (20.005238055389295, from triangle centres summed in GTS's vertex order by
/// `triCenter`, `multispline.c:391`), the first inserted (left) one is popped first and
/// the guiding polygon is the left sleeve. Its bends are not corners next to the end
/// points, so `tweakEnd` (`multispline.c:658`) leaves the head on the node centre:
/// `e,29.16,36.482 29.126,71.526 ...`.
///
/// Actual (port): the port's constrained Delaunay triangulation has the same triangles but
/// a different vertex rotation per face (GTS's `gts_triangle_vertices` order follows its
/// incremental construction behind `tri`/`mkSurface`, `lib/neatogen/delaunay.c:140,454`),
/// so the centres differ by an ulp, the right head edge is 1 ulp shorter and the path
/// takes the right sleeve. Its last bend is the obstacle's top corner, so `tweakEnd` moves
/// the head 0.1pt right: `e,29.26,36.483 29.026,71.859 ...`.
#[test]
#[ignore = "issue #28: triangle-path tie decided by GTS's face vertex order, which the port's CDT does not reproduce"]
fn neato_polyline_compass_port_routes_like_c() {
    let src = "digraph G { splines=\"polyline\"; \"10\" [width=0.81, pos=\"100,18\"]; \"9\" [pos=\"100,90\"]; \"9\" -> \"10\":\"n\"; }";
    let (out, err, ok) = run(&["-Kneato", "-n", "-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "pos=\"e,29.16,36.482 29.126,71.526 29.134,63.677 29.142,54.763 29.149,47.889\""
        ),
        "{out}"
    );
}

// Fuzz case 010-dot-pack-single-component-ignores-cc-subgraph (issue #19). With pack set and a
// single connected component, C's doDot (lib/dotgen/dotinit.c:466-475) lays out the root with
// `dotLayout(g)` while the `_cc_0` subgraph made by `cccomps` is still a child of the root.
// `_cc_0` inherits the root's `rank=max` and holds every node, so it is a rank set and a and b
// share a rank (C: 134x44pt, side by side). The port used to lay out the plain root, where the
// root's own `rank` is ignored (62x116pt, b below a).
#[test]
fn pack_single_component_lays_out_with_cc_subgraph() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G {\npack=true;\nrank=max;\na -> b;\n}\n",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"134pt\" height=\"44pt\""),
        "{out}"
    );
    assert!(out.contains("cx=\"27\" cy=\"-18\""), "{out}");
    assert!(out.contains("cx=\"99\" cy=\"-18\""), "{out}");
}

// Fuzz case 001-dot-cluster-rank-max-build-skeleton-panic (issue #10) as generated, with
// `pack="8"`. Through the `_cc_0` subgraph of issue #19 C ranks the clusters so that
// build_skeleton finds every rank leader; it exits 0 with the mark_clusters warning
// (lib/dotgen/cluster.c:318) and this layout. The pack-less input crashes C.
#[test]
fn pack_single_component_cluster_node_in_rank_max_set_lays_out_like_c() {
    let src = "strict digraph \"G\" {
pack=\"8\";
\"n148\";
subgraph \"cluster_1\" {
\"n165\";
\"n181\" [];
\"n235\";
subgraph \"cluster4\" {
\"n102\" [];
\"n225\" [];
}
}
{
rank=max;
\"n235\";
\"n148\";
}
{\"n15\" \"n181\" \"n205\"} -> \"n102\";
\"n148\" -> \"n164\";
\"n164\" -> \"n165\";
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: n235 was already in a rankset, deleted from cluster G\n"
    );
    assert!(
        out.contains("<svg width=\"418pt\" height=\"212pt\""),
        "{out}"
    );
    assert!(
        out.contains("points=\"79.35,-16 79.35,-68 234.35,-68 234.35,-16 79.35,-16\""),
        "cluster4: {out}"
    );
    assert!(out.contains("cx=\"379.35\" cy=\"-114\""), "n235: {out}");
}

// Issue #19 under newrank. `dot2_rank`'s compile_samerank also walks `_cc_0`'s clone of
// cluster_a; `set_parent` → `node_induce` empties the clone (cluster_a already holds a and
// b), so dot_mincross prints "removing empty cluster" (lib/dotgen/mincross.c:346).
#[test]
fn pack_single_component_newrank_removes_empty_cluster_clone() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G { pack=true; newrank=true; subgraph cluster_a { a -> b; } b -> c; }\n",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: removing empty cluster\n");
    assert!(
        out.contains("<svg width=\"94pt\" height=\"204pt\""),
        "{out}"
    );
}

// Issue #203, reduced. With tiny touching point nodes `partition` keeps no free rectangle,
// so C's mkMaze fails and `orthoEdges` returns -1 (lib/ortho/partition.c, maze.c). dot_layout
// then gives up: no edge is drawn and dotneato_postprocess is skipped, so the drawing keeps
// its negative x. The port panicked in `convert_sp_to_route`. C: exit 0, 19x94pt, 12 nodes.
#[test]
fn ortho_maze_without_free_cells_abandons_the_dot_layout_like_c() {
    let src = "digraph{splines=ortho nodesep=0 ranksep=0.02
node[shape=point width=0.05 height=0.01 fixedsize=true label=\"\"]
n3->n2 n10->n7 n1->n11 n10->n4 n8->n2 n5->n3 n6->n4 n3->n10 n6->n3 n6->n0 n3->n6
n0->n8 n11->n10 n2->n10 n3->n2 n6->n10 n0->n9 n11->n5}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(err, "");
    assert!(out.contains("<svg width=\"19pt\" height=\"94pt\""), "{out}");
    assert_eq!(out.matches("class=\"node\"").count(), 12, "{out}");
    assert!(!out.contains("class=\"edge\""), "{out}");
}

// Issue #200: zero-size point nodes make partition's trapezoidation fail, so mkMaze fails
// as in #203 and C's dot abandons the layout and exits 0 with the nodes only. The port exited 1
// with a layout error. C also prints two trapezoid failure messages, and so does the port
// (#127).
#[test]
fn ortho_trapezoid_failure_abandons_the_dot_layout_like_c() {
    let src = "digraph{splines=ortho nodesep=0 ranksep=0
node[shape=point width=0 height=0 fixedsize=true]
a->b a->c a->d b->e c->e d->e b->f c->f e->g f->g a->g}
";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok);
    assert_eq!(
        err,
        "trapezoid segment construction failed\nhorizontal trapezoid construction failed\n"
    );
    assert!(out.contains("<svg width=\"13pt\" height=\"35pt\""), "{out}");
    assert_eq!(out.matches("class=\"node\"").count(), 7, "{out}");
    assert!(!out.contains("class=\"edge\""), "{out}");
}

// As above with a root rank=max, which the emptied clone inherits: its union_all finds no
// node, and C's `union_one(NULL, GD_maxrep(clust))` returns NULL (lib/dotgen/rank.c:615-621,
// 684). The port unwrapped the missing leader and panicked. C: one rank, 222x76pt.
#[test]
fn pack_single_component_newrank_rank_max_empty_clone_does_not_panic() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G { pack=true; newrank=true; rank=max; subgraph cluster_a { a -> b; } b -> c; }\n",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: removing empty cluster\n");
    assert!(
        out.contains("<svg width=\"222pt\" height=\"76pt\""),
        "{out}"
    );
    for cx in ["43", "115", "187"] {
        assert!(out.contains(&format!("cx=\"{cx}\" cy=\"-34\"")), "{out}");
    }
}

/// Issue #173: `cluster_inner4` is repeated under three parents and each copy holds j1 and
/// j2. osage's `layout` (`lib/osage/osageinit.c`) places them in the first copy, packs
/// nothing for the others and leaves them a NaN box, so `j1 -> o1` and `j2 -> o1` get NaN
/// control points. `update_bb_bz` (`lib/common/emit.c`) splits such a segment forever
/// and C's stack overflows; the port must render the graph.
#[test]
fn osage_repeated_cluster_does_not_overflow_update_bb_bz() {
    for fmt in ["-Tsvg", "-Tdot"] {
        let (out, err, ok) = run(
            &[fmt],
            "digraph{layout=osage{o2\n  subgraph cluster_inner4{j1 j2}}subgraph cluster_outer{{label=\"outer\"\
         subgraph cluster_inner4{j1 j2}}->o1}subgraph cluster_inner4{j1 j2}}\n",
        );
        assert!(ok, "{fmt}: {err}");
        if fmt == "-Tdot" {
            // The edges leave the root's 18x18 empty-graph box as it was.
            assert!(out.contains("bb=\"0,0,18,18\""), "{out}");
            assert!(out.contains("j1 -> o1\t[pos="), "{out}");
        }
    }
}

// The same empty-cluster rank set without pack: cluster_b's nodes are already in
// cluster_a, so node_induce empties it before its rank=max is compiled (newrank).
#[test]
fn newrank_rank_max_cluster_emptied_by_node_induce_does_not_panic() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G { newrank=true; subgraph cluster_a { a -> b } \
         subgraph cluster_b { rank=max; a; b } b -> c; }\n",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: removing empty cluster\n");
    assert!(
        out.contains("<svg width=\"94pt\" height=\"204pt\""),
        "{out}"
    );
}

// #100: pack's `compute_bb` (lib/pack/pack.c:801) measures a component by its own edges.
// v6 is in two components; its edge v6 -> v0 belongs to _cc_1 only, so it must not widen
// _cc_0's box (C: _cc_0 is 8 0 78 176.48, and the cluster boxes below).
#[test]
fn pack_component_box_takes_only_its_own_edges() {
    let (out, _, _) = run(
        &["-Kdot", "-Tdot"],
        "digraph G { pack=true; v5; subgraph cluster_0 { v6 [height=1.34] } \
         subgraph cluster_2 { v6 -> v0; v1 -> v5 } v1 -> v0 }\n",
    );
    assert!(out.contains("graph [bb=\"0,0,317,340.48\""), "{out}");
    assert!(out.contains("graph [bb=\"114,167,184,279.48\"]"), "{out}");
    assert!(out.contains("graph [bb=\"78,0,220,154.24\"]"), "{out}");
}

// #101: a node of two non-nested clusters is in two pack components. C lays the second out
// on the records the first left; the port lays out each one on fresh records (not matched,
// see docs/porting/crashes.md, "dot's pack components"). It reports what C reports and
// draws one spline for the shared edge.
#[test]
fn pack_node_in_two_non_nested_clusters_lays_out_each_component_afresh() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tdot"],
        "digraph G { pack=true; subgraph cluster_0 { v0; v6 } subgraph cluster_2 { v6 -> v0 } }\n",
    );
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Error: node \"v0\" belongs to two non-nested clusters \"cluster_2\" and \"cluster_0\"\n\
         Error: node \"v6\" belongs to two non-nested clusters \"cluster_2\" and \"cluster_0\"\n"
    );
    assert!(
        out.contains("v6 -> v0\t[pos=\"e,99,36.104 99,71.697 99,64.407 99,55.726 99,47.536\"]"),
        "{out}"
    );
}

// #312: dot's node_induce gives a packed component's cluster clone the edges between its
// nodes; agdelete drops the clone, so -Tdot writes a loop declared at the root at the root
// (C), not inside the original cluster.
#[test]
fn pack_cluster_loop_edge_stays_at_the_root_in_dot_output() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tdot"],
        "graph { pack=true; n5; subgraph cluster_c { label=C; n0; n1 }; n0 -- n0 }\n",
    );
    assert!(ok, "{err}");
    assert!(out.contains("\n\tn0 -- n0\t[pos="), "{out}");
    assert!(!out.contains("\t\tn0 -- n0"), "{out}");
}

// #226: when a later component's ortho maze cannot be built, doDot returns before packing
// (lib/dotgen/dotinit.c): the earlier component keeps its layout, the `_cc_<i>` subgraphs
// stay, and the root's box is what clip_and_install grew it to (C: 0,0,0,43.595).
#[test]
fn pack_failed_component_keeps_cc_subgraphs_and_root_box() {
    let input = "digraph{pack=true a[shape=box width=1 height=0.5 label=A] \
        b[shape=box width=1 height=0.5 label=B] a->b splines=ortho nodesep=0 ranksep=0.02\n\
        node[shape=point width=0.05 height=0.01 fixedsize=true label=\"\"]\n\
        n3->n2 n10->n7 n1->n11 n10->n4 n8->n2 n5->n3 n6->n4 n3->n10 n6->n3 n6->n0 n3->n6\n\
        n0->n8 n11->n10 n2->n10 n3->n2 n6->n10 n0->n9 n11->n5}\n";
    let (out, err, ok) = run(&["-Kdot", "-Tdot"], input);
    assert!(ok, "{err}");
    assert!(out.contains("graph [bb=\"0,0,0,43.595\""), "{out}");
    assert!(out.contains("\tsubgraph _cc_0 {\n"), "{out}");
    assert!(out.contains("\tsubgraph _cc_1 {\n"), "{out}");
    assert!(
        out.contains("\t\ta -> b\t[pos=\"e,0,36.473 0,43.595 0,43.595 0,42.883 0,42.883\"];"),
        "{out}"
    );
    let (svg, _, _) = run(&["-Kdot", "-Tsvg"], input);
    assert!(svg.contains("<svg width=\"8pt\" height=\"52pt\""), "{svg}");
}

// #226 with clusters: the failed layout keeps `_cc_0`'s clones of cluster_a and cluster_b, each
// declaring the graph attributes agcopyattr copied, with the edges dot's node_induce gave them
// (C's -Tdot, verbatim). An anonymous subgraph's projection keeps its `%<id>` name (-Tjson).
#[test]
fn pack_failed_component_keeps_cluster_clones_like_c() {
    let ortho = "node[shape=point width=0.05 height=0.01 fixedsize=true label=\"\"]\n\
        n3->n2 n10->n7 n1->n11 n10->n4 n8->n2 n5->n3 n6->n4 n3->n10 n6->n3 n6->n0 n3->n6\n\
        n0->n8 n11->n10 n2->n10 n3->n2 n6->n10 n0->n9 n11->n5}\n";
    let input = format!(
        "digraph{{pack=true splines=ortho nodesep=0 ranksep=0.02\n\
         subgraph cluster_a {{ label=A; x1; subgraph cluster_b {{ x2; x3 }} x1->x2 }}\n\
         x2->x3 x3->x1\n{ortho}"
    );
    let (out, err, ok) = run(&["-Kdot", "-Tdot"], &input);
    assert!(ok, "{err}");
    let expected = concat!(
        "\tsubgraph _cc_0 {\n",
        "\t\tsubgraph cluster_a {\n",
        "\t\t\tgraph [label=A,\n",
        "\t\t\t\tnodesep=0,\n",
        "\t\t\t\tpack=true,\n",
        "\t\t\t\tranksep=0.02,\n",
        "\t\t\t\tsplines=ortho\n",
        "\t\t\t];\n",
        "\t\t\tsubgraph cluster_b {\n",
        "\t\t\t\tgraph [label=A,\n",
        "\t\t\t\t\tnodesep=0,\n",
        "\t\t\t\t\tpack=true,\n",
        "\t\t\t\t\tranksep=0.02,\n",
        "\t\t\t\t\tsplines=ortho\n",
        "\t\t\t\t];\n",
        "\t\t\t\tx2;\n",
        "\t\t\t\tx3;\n",
        "\t\t\t\tx2 -> x3\t[pos=\"e,51,96.473 51,103.59 51,103.59 51,102.88 51,102.88\"];\n",
        "\t\t\t}\n",
        "\t\t\tx1;\n",
        "\t\t\tx1 -> x2;\n",
        "\t\t\tx3 -> x1\t[pos=\"e,53,44.265 53,59.813 53,59.813 53,55.778 53,55.778\"];\n",
        "\t\t}\n",
        "\t}\n"
    );
    let start = out.find("\tsubgraph _cc_0 {").expect("_cc_0");
    let end = out.find("\tsubgraph _cc_1 {").expect("_cc_1");
    assert_eq!(&out[start..end], expected);

    let input = format!(
        "digraph{{pack=true splines=ortho nodesep=0 ranksep=0.02\n\
         subgraph s1 {{ x1; x2 }} {{rank=same; x3; x4}} subgraph {{ x5 }} \
         x1->x3 x2->x4 x3->x5 x4->x5\n{ortho}"
    );
    let (json, err, ok) = run(&["-Kdot", "-Tjson"], &input);
    assert!(ok, "{err}");
    assert_eq!(json.matches("\"name\": \"%3\"").count(), 2, "{json}");
    assert_eq!(json.matches("\"name\": \"%5\"").count(), 2, "{json}");
}

// With a non-default ratio C's doDot lays out the root with dotLayout(g) even with several
// components (lib/dotgen/dotinit.c:466-494), and the `_cc_<i>` subgraphs exist meanwhile.
// Each inherits rank=max, so a, b, c and d share one rank (C: 216x34pt). Without pack they
// would take two ranks.
#[test]
fn pack_ratio_several_components_lays_out_with_cc_subgraphs() {
    let (out, err, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G { pack=true; ratio=fill; size=\"3,3\"; rank=max; a -> b; c -> d; }\n",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("<svg width=\"216pt\" height=\"34pt\""),
        "{out}"
    );
    for cx in ["27", "99", "171", "243"] {
        assert!(out.contains(&format!("cx=\"{cx}\" cy=\"-18\"")), "{out}");
    }
}

/// The `<title>`s of the cluster groups of an SVG, in drawing order.
fn svg_cluster_titles(svg: &str) -> Vec<&str> {
    svg.split("class=\"cluster\">\n<title>")
        .skip(1)
        .map(|s| &s[..s.find("</title>").unwrap()])
        .collect()
}

// Issue #186: twopi stacks the nodes in a column whose sides differ by float noise, and the
// ortho search graph does not connect the two sides of t2's self-loop. shortPath never
// reaches the loop's start, and C's convertSPtoRoute (lib/ortho/ortho.c) dereferences its
// null N_DAD and segfaults (exit 139, no output). The port used to panic on the same
// parent ("called `Option::unwrap()` on a `None` value", exit 101); it now abandons ortho
// routing, as C's orthoEdges does when shortPath fails, and the edges are not routed
// orthogonally.
const ISSUE186_GRAPH: &str = "graph{layout=twopi splines=ortho oneblock=true rankdir=LR
node[shape=record]
t2--t2 t1--t1 t0--t1 t0--t1 t1--t2 t1--t2 t1--t1 t1--t0}
";

#[test]
fn twopi_ortho_self_loop_without_a_path_does_not_panic_convert_sp_to_route() {
    let (out, err, ok) = run(&["-Tsvg"], ISSUE186_GRAPH);
    assert!(ok, "C crashes here, the port abandons ortho routing: {err}");
    assert_eq!(
        err,
        "Warning: orthogonal routing found no path for an edge - \
         edges not routed orthogonally\n"
    );
    assert_eq!(out.matches("class=\"edge\"").count(), 8, "{out}");
}

// Issue #186, the case C survives: the same unreachable self-loop, routed after edges that
// reached the search graph's start node. C's reset (lib/ortho/sgraph.c) does not clear
// N_DAD, so convertSPtoRoute follows the stale parent and C routes every edge (exit 0).
// The port must not take the #186 fallback here: no warning, and t2--t2 gets the ortho route
// C draws through the stale parent.
const ISSUE186_STALE_PARENT_GRAPH: &str =
    "graph{layout=twopi splines=ortho oneblock=true rankdir=LR
node[shape=record]
a--t2 t0--t2 t2--t2 t1--t1 t0--t1 t0--t1 t1--t2 t1--t2 t1--t1 t1--t0}
";

#[test]
fn twopi_ortho_unreachable_self_loop_follows_the_stale_parent_like_c() {
    let (out, err, ok) = run(&["-Tsvg"], ISSUE186_STALE_PARENT_GRAPH);
    assert!(ok, "{err}");
    assert_eq!(err, "");
    assert!(
        out.contains(
            "d=\"M70.29,-168.25C70.29,-139.53 70.29,-87 70.29,-87 70.29,-87 96,-87 96,-87 \
             96,-87 96,-183.41 96,-186.43\""
        ),
        "{out}"
    );
}

// Issue #25 (fuzz case 016-dot-pack-cluster-list-from-subgraph-tree). With pack/packmode,
// dot's `copyClusterInfo` (lib/dotgen/dotinit.c) rebuilds `GD_clust(root)` in component
// order, appending a cluster once per component that kept it, and `emit_clusters`
// (lib/common/emit.c) draws that list as it is. A node of two non-nested clusters puts
// both clusters in two components, each of which keeps `cluster_0`, so C draws it twice;
// components whose clusters are not in declaration order are drawn in component order.
// The renderer used to rebuild the list from the subgraph tree (once each, declaration
// order).
#[test]
fn packed_dot_draws_the_root_cluster_list_of_copy_cluster_info() {
    let (out, _, ok) = run(
        &["-Kdot", "-Tsvg"],
        "digraph G {\npack=8;\nsubgraph cluster_0 { a }\nsubgraph cluster_1 { a }\n}\n",
    );
    assert!(ok);
    assert_eq!(svg_cluster_titles(&out), ["cluster_0", "cluster_0"]);
    assert_eq!(
        out.matches("<g id=\"clust1\" class=\"cluster\">").count(),
        2
    );
    let (out, _, ok) = run(
        &["-Kdot", "-Tsvg"],
        "graph G {\npackmode=\"node\";\nsubgraph cluster_0 { a }\nsubgraph cluster_3 { b }\n\
         subgraph cluster_5 { c }\nx -- a;\nx -- c;\n}\n",
    );
    assert!(ok);
    assert_eq!(
        svg_cluster_titles(&out),
        ["cluster_0", "cluster_5", "cluster_3"]
    );
}

// Issue #193 (C16): the json writer's `label_subgs` (gvrender_core_json.c:617) warns once
// for each later cluster with a taken name; `cluster=true` makes a cluster too. Two top-level
// `subgraph cluster_a` are one subgraph. With `compound=true`, `mkClustMap` warns first.
#[test]
fn json_warns_about_duplicate_cluster_names_like_c() {
    let dup = "Warning: Duplicate cluster name \"cluster_a\"\n";
    for (args, src, want) in [
        (
            &["-Tjson"][..],
            "digraph{subgraph cluster_a{x} subgraph cluster_b{subgraph cluster_a{y}}}",
            dup.to_string(),
        ),
        (
            &["-Kneato", "-Tdot_json"][..],
            "digraph{subgraph cluster_a{x} subgraph cluster_a{y} subgraph s{subgraph cluster_a{z}} subgraph t{subgraph cluster_a{w}}}",
            dup.repeat(2),
        ),
        (
            &["-Txdot_json"][..],
            "digraph{subgraph x{cluster=true; a} subgraph y{subgraph x{cluster=true; b}} subgraph cluster_q{c} subgraph z{subgraph cluster_q{d}}}",
            "Warning: Duplicate cluster name \"x\"\nWarning: Duplicate cluster name \"cluster_q\"\n".to_string(),
        ),
        (
            &["-Tjson0"][..],
            "digraph G { compound=true; subgraph cluster_a { x } subgraph s { subgraph cluster_a { y } } x -> y; }",
            format!("Warning: Two clusters named cluster_a - the second will be ignored\n{dup}"),
        ),
        (
            &["-Tsvg"][..],
            "digraph{subgraph cluster_a{x} subgraph cluster_b{subgraph cluster_a{y}}}",
            String::new(),
        ),
        // `insert` copies `agnameof`, so anonymous clusters (`%3`, `%5`) are two names.
        (
            &["-Tjson"][..],
            "digraph{subgraph {cluster=true; a} subgraph {cluster=true; b}}",
            String::new(),
        ),
    ] {
        let (_, err, ok) = run(args, src);
        assert!(ok, "{args:?} {src}: {err}");
        assert_eq!(err, want, "{args:?} {src}");
    }
}

// C16: `fillMap` keys an anonymous cluster by `agnameof`'s static buffer, so every later
// anonymous cluster is a second one (warned by its own `%<id>`), and `findCluster` then finds
// the first by the last anonymous name only. `subgraph ""` is named "".
#[test]
fn clust_map_keys_anonymous_clusters_like_c() {
    let two = |n: &str| format!("Warning: Two clusters named {n} - the second will be ignored\n");
    for (args, src, want) in [
        (
            &["-Tsvg"][..],
            "digraph{compound=true; subgraph {cluster=true; a} subgraph {cluster=true; b} x -> a [lhead=\"%3\"]; x -> b [lhead=\"%5\"]; y -> b [ltail=\"%2\"]}",
            format!(
                "{}Warning: cluster named %3 not found\nWarning: x -> b: head not inside head cluster %5\nWarning: cluster named %2 not found\n",
                two("%5")
            ),
        ),
        (
            &["-Tsvg"][..],
            "digraph{compound=true; subgraph \"\"{cluster=true; a} subgraph s{subgraph \"\"{cluster=true; b}} subgraph {cluster=true; c} subgraph {cluster=true; d}}",
            format!("{}{}", two(""), two("%5")),
        ),
        (
            &["-Kfdp", "-Tsvg"][..],
            "graph{subgraph {cluster=true; a b} subgraph {cluster=true; c} subgraph t{subgraph {cluster=true; e}} a -- c}",
            format!("{}{}", two("%5"), two("%7")),
        ),
    ] {
        let (_, err, ok) = run(args, src);
        assert!(ok, "{args:?} {src}: {err}");
        assert_eq!(err, want, "{args:?} {src}");
    }
}

// The reserved cluster-list key written in the input is an ordinary attribute to C: it
// must not reorder another engine's clusters, nor (inherited by the subgraphs, so a
// cluster lists itself) make the cluster tree cyclic, which overflowed the stack.
#[test]
fn input_cluster_list_key_is_ignored() {
    let src =
        "graph G { \"\u{1}clustlist\"=\"1 0\"; subgraph cluster_a {a} subgraph cluster_b {b} }\n";
    for engine in ["-Kneato", "-Kfdp", "-Kosage", "-Kpatchwork", "-Kdot"] {
        let (out, err, ok) = run(&[engine, "-Tsvg"], src);
        assert!(ok, "{engine}: {err}");
        assert_eq!(
            svg_cluster_titles(&out),
            ["cluster_a", "cluster_b"],
            "{engine}"
        );
    }
}

/// Issue #174 (found by cargo-fuzz): a component taller than INT_MAX polyomino steps.
/// C's `computeStep` (`lib/pack/pack.c:86`) saturates a step of ~1e29 to INT_MAX, and
/// `genPoly`/`genBox` then fill ~1e22 cells per side: C Graphviz 16.1.0 never finishes on
/// these inputs. The port gives a saturated step a cell budget, warns when it runs out and
/// leaves the components unpacked. A node pinned near 1e30 (C hangs too) packs. Each run gets
/// 30 s (it takes well under a second) so a regression fails instead of hanging CI.
#[test]
fn oversized_component_does_not_hang_the_packer() {
    let warning = "Warning: components too large to pack; packing skipped\n";
    let cases = [
        ("graph { layout=fdp; a [height=\"1e30\"]; b }", warning),
        (
            "graph { layout=fdp; quantum=\"1e40\"; a -- b; c -- d }",
            warning,
        ),
        (
            "graph { layout=neato; pack=false; a [height=\"1e30\"]; b }",
            warning,
        ),
        ("graph { layout=fdp; a [height=\"1e300\"]; b }", warning),
        ("graph { layout=fdp; a [pos=\"1e30,1e30!\"]; b; c }", ""),
        // A saturated step within the budget packs as C does.
        (
            "graph { layout=neato; pack=true; a [pos=\"1e9,1e9!\"]; b }",
            "",
        ),
    ];
    for (src, expected) in cases {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
            .arg("-Tsvg")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(src.as_bytes())
            .unwrap();
        // The SVG is a few KB, well under the pipe buffer, so polling cannot block.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                panic!("packing must terminate: {src}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let out = child.wait_with_output().unwrap();
        let (svg, err) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(out.status.success(), "{src}: {err}");
        assert_eq!(err, expected, "{src}");
        assert!(svg.contains("</svg>"), "{src}: {svg}");
    }
}

/// Issue #174: C's `fits` (`lib/pack/pack.c:437`) computes `step * x` in int, which wraps
/// once the step is large. With a saturated step (a 1e11-inch box) or a huge `pack` margin,
/// C places `b` near the origin; the expected output is C 16.1.0's `-Tplain`.
#[test]
fn large_pack_step_wraps_placement_like_c() {
    let cases = [
        (
            "graph { layout=fdp; node[shape=box fixedsize=true]; \
             a [width=\"1e11\" height=\"1e11\"]; b }",
            "graph 1 1e+11 1e+11
node a 5e+10 5e+10 1e+11 1e+11 a solid box black lightgrey
node b 0.375 0.27777 0.75 0.5 b solid box black lightgrey
stop
",
        ),
        (
            "graph { layout=neato; pack=2000000000; a; b }",
            "graph 1 4.5436e+07 1.2312e+07
node a 4.5436e+07 1.2312e+07 0.75 0.5 a solid ellipse black lightgrey
node b 0.375 0.25 0.75 0.5 b solid ellipse black lightgrey
stop
",
        ),
    ];
    for (src, expected) in cases {
        let (out, err, ok) = run(&["-Tplain"], src);
        assert!(ok && err.is_empty(), "{src}: {err}");
        assert_eq!(out, expected, "{src}");
    }
}

// Fuzz case 002-dot-ortho-concentrate-ignored-edge (issue #11), its repro.gv verbatim.
// Under concentrate, class2 marks `13 -> 14` IGNORED, yet orthoEdges (lib/ortho/ortho.c)
// routes it as the first edge of the {13, 14} pair. C draws it (emit_edge_graphics has no type
// check) but attach_attrs_and_arrows (lib/common/output.c) gives it no `pos`. The port used to
// drop its spline before rendering, so -Tsvg lost the edge. The expected bytes are C's.
#[test]
fn ortho_concentrate_draws_routed_ignored_edge() {
    let src = "digraph G {
splines=\"ortho\";
concentrate=\"true\";
\"8\" -> \"9\";
\"8\" -> \"13\";
\"9\" -> \"14\";
\"14\" -> \"13\";
\"13\" -> \"14\";
}
";
    let (svg, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    let edge = "<!-- 13&#45;&gt;14 -->
<g id=\"edge5\" class=\"edge\">
<title>13&#45;&gt;14</title>
<path fill=\"none\" stroke=\"black\" d=\"M40.5,-33.98C40.5,-33.98 40.5,-62.55 40.5,-62.55\"/>
<polygon fill=\"black\" stroke=\"black\" points=\"37,-62.55 40.5,-72.55 44,-62.55 37,-62.55\"/>
</g>
";
    assert!(svg.contains(edge), "{svg}");
    assert!(!svg.contains("<title>14&#45;&gt;13</title>"), "{svg}");
    let (dot, err, ok) = run(&["-Kdot", "-Tdot"], src);
    assert!(ok, "{err}");
    assert!(
        dot.contains("\t13 -> 14;\n") && dot.contains("\t14 -> 13;\n"),
        "{dot}"
    );
    // The layout places the labels of the IGNORED edge to draw them, but C writes back no
    // position for it: the input head_lp and xlp stay, and no pos appears.
    let (dot, err, ok) = run(
        &["-Kdot", "-Tdot"],
        &src.replace(
            "\"13\" -> \"14\";",
            "\"13\" -> \"14\" [headlabel=H xlabel=X head_lp=\"1,2\" xlp=\"3,4\"];",
        ),
    );
    assert!(ok, "{err}");
    let edge = "\t13 -> 14\t[head_lp=\"1,2\",
\t\theadlabel=H,
\t\txlabel=X,
\t\txlp=\"3,4\"];
";
    assert!(dot.contains(edge), "{dot}");
    // Label positions with no label to place stay as well.
    let (dot, err, ok) = run(
        &["-Kdot", "-Tdot"],
        &src.replace(
            "\"13\" -> \"14\";",
            "\"13\" -> \"14\" [head_lp=\"1,2\" tail_lp=\"5,6\" xlp=\"7,8\"];",
        ),
    );
    assert!(ok, "{err}");
    let edge = "\t13 -> 14\t[head_lp=\"1,2\",
\t\ttail_lp=\"5,6\",
\t\txlp=\"7,8\"];
";
    assert!(dot.contains(edge), "{dot}");
}

// Cause C18 (usershape): the files of `image`, `shapefile`, `shape=epsf` and HTML `<IMG>`
// go through `safefile` and `gvusershape_file_access`/`gvusershape_open`
// (`lib/gvc/gvusershape.c`) or `epsf_init` (`lib/common/psusershape.c`); HTML labels go
// through expat (`lib/common/htmllex.c`). Expected output is C 16.1.0's.

/// A scratch directory with a text file and an EPS file whose `%%BoundingBox` is 100x50.
fn usershape_dir(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(dir.join("notes.txt"), "not an image, just some text\n").unwrap();
    std::fs::write(
        dir.join("box.eps"),
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 110 70\n",
    )
    .unwrap();
    dir
}

const USERSHAPES: &str = "digraph {
  a [image=\"missing.png\"]
  b [image=\"notes.txt\"]
  c [shapefile=\"missing.ps\"]
  d [shape=custom]
  e [label=<<TABLE><TR><TD><IMG SRC=\"notes.txt\"/></TD></TR></TABLE>>]
}";

// Issue #130: a file that cannot be opened, or that is neither an image nor a user shape.
// `d`'s shapefile is the declared default "", not NULL. The `<IMG>` error exits 1.
#[test]
fn usershape_files_c_cannot_load_are_reported() {
    let dir = usershape_dir("c18-files");
    let (_, err, code) = run_in(&dir, &["-Tsvg"], USERSHAPES);
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "Warning: No such file or directory while opening missing.png\n\
         Warning: No or improper image=\"missing.png\" for node \"a\"\n\
         Warning: \"notes.txt\" was not found as a file or as a shape library member\n\
         Warning: No or improper image=\"notes.txt\" for node \"b\"\n\
         Warning: No such file or directory while opening missing.ps\n\
         Warning: No or improper shapefile=\"missing.ps\" for node \"c\"\n\
         Warning: No or improper shapefile=\"\" for node \"d\"\n\
         Warning: \"notes.txt\" was not found as a file or as a shape library member\n\
         Error: No or improper image file=\"notes.txt\"\n\
         in label of node e\n"
    );
    // safefile: with `imagepath`, a relative name must be found in one of its directories
    let src = "digraph { imagepath=\"/nonexistent\"; a [image=\"notes.txt\"] }";
    let (_, err, code) = run_in(&dir, &["-Tsvg"], src);
    assert_eq!(code, 0);
    assert_eq!(
        err,
        "Warning: Filename \"notes.txt\" is unsafe\n\
         Warning: No or improper image=\"notes.txt\" for node \"a\"\n"
    );
}

// Issue #130: `$SERVER_NAME` disables file loading (`safefile`, warned once), and a
// shapefile `safefile` refuses does not make the node `custom`.
#[test]
fn usershape_files_are_not_read_under_server_name() {
    let dir = usershape_dir("c18-server");
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .arg("-Tsvg")
        .current_dir(&dir)
        .env("SERVER_NAME", "web")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(USERSHAPES.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Warning: file loading is disabled because the environment contains SERVER_NAME=\"web\"\n\
         Warning: Filename \"missing.png\" is unsafe\n\
         Warning: No or improper image=\"missing.png\" for node \"a\"\n\
         Warning: Filename \"notes.txt\" is unsafe\n\
         Warning: No or improper image=\"notes.txt\" for node \"b\"\n\
         Warning: No or improper shapefile=\"\" for node \"d\"\n\
         Warning: Filename \"notes.txt\" is unsafe\n\
         Error: No or improper image file=\"notes.txt\"\n\
         in label of node e\n"
    );
}

// Issue #132: `epsf_init` leaves `ND_shape_info` NULL unless the shapefile has a
// `%%BoundingBox`, and `epsf_gencode` then draws nothing, not even the label. The node keeps
// its `width` and `height` (the label does not grow it); a loaded file gives the size.
#[test]
fn epsf_without_a_loadable_shapefile_draws_nothing() {
    let dir = usershape_dir("c18-epsf");
    let src = "digraph { a [shape=epsf label=\"a long label for the node\"]; \
               b [shape=epsf shapefile=\"notes.txt\"]; c [shape=epsf shapefile=\"box.eps\"] }";
    let (svg, err, code) = run_in(&dir, &["-Tsvg"], src);
    assert_eq!(code, 0);
    assert_eq!(
        err,
        "Warning: shapefile not set or not found for epsf node a\n\
         Warning: BoundingBox not found in epsf file notes.txt\n"
    );
    for (node, title) in [("node1", "a"), ("node2", "b")] {
        let group = format!("<g id=\"{node}\" class=\"node\">\n<title>{title}</title>\n</g>\n");
        assert!(svg.contains(&group), "{svg}");
    }
    // (C also writes `134 -20 translate newpath user_shape_0` here, and so does the port:
    // `epsf_writes_its_postscript_call_into_every_output_like_c`.)
    assert!(svg.contains(
        "<text xml:space=\"preserve\" text-anchor=\"middle\" x=\"194\" y=\"-20.8\" \
         font-family=\"Times,serif\" font-size=\"14.00\">c</text>"
    ));
    let (dot, _, _) = run_in(&dir, &["-Tdot"], src);
    assert!(dot.contains("graph [bb=\"0,0,244,50\"]"), "{dot}");
    assert!(
        dot.contains("pos=\"27,25\",\n\t\tshape=epsf,\n\t\twidth=0.75]"),
        "{dot}"
    );
    assert!(dot.contains("height=0.69444,"), "{dot}");
    assert!(dot.contains("width=1.3889]"), "{dot}");
}

// An epsf node has no polygon, so `makeAddPoly`/`makePoly` (`lib/neatogen/poly.c:189, 258`)
// report it and `makeInfo` fails: `removeOverlapWith` gives up on the overlaps (C then
// crashes, #265). The port used to index the missing polygon and panic.
#[test]
fn voronoi_overlap_reports_an_epsf_node_like_c() {
    for (sep, f) in [("", "makeAddPoly"), ("sep=0.5;", "makePoly")] {
        let src = format!("graph {{ overlap=voronoi; {sep} a [shape=epsf]; b; a -- b }}");
        for engine in ["neato", "fdp"] {
            let (err, ok) = warnings(engine, &src);
            assert!(!ok, "{engine}");
            assert_eq!(
                err,
                format!(
                    "Warning: shapefile not set or not found for epsf node a\n\
                     Error: {f}: unknown shape type epsf\n"
                ),
                "{engine}"
            );
        }
    }
}

// Issue #131: expat rejects bytes that are not UTF-8. A character cut off at the end of a
// `findNext` unit waits for more input (`a`'s is reported with the next unit, `</B>`), and a
// partial tag is parsed again only once its bytes have doubled (`b`'s waits past `x`).
// `charset=big5` is an encoding expat does not know. C exits 1.
#[test]
fn html_label_encoding_errors_are_reported() {
    let dir = scratch("c18-encoding");
    let utf8 = dir.join("utf8.gv");
    std::fs::write(
        &utf8,
        b"digraph {\n a [label=<<B>caf\xe9</B>>]\n b [label=<<B\xe9>x</B>>]\n}\n",
    )
    .unwrap();
    let (_, err, code) = run_in(&dir, &["-Tsvg", "utf8.gv"], "");
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "Error: not well-formed (invalid token) in line 1 \n\
         ... caf\u{FFFD}</B> ...\n\
         in label of node a\n\
         Error: not well-formed (invalid token) in line 1 \n\
         ... x</B> ...\n\
         in label of node b\n"
    );
    let src = "digraph { charset=big5; a [label=<<B>x</B>>] }";
    let (_, err, code) = run_in(&dir, &["-Tsvg"], src);
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "Error: unknown encoding in line 1 \n... <HTML> ...\nin label of node a\n"
    );
}

// Issue #185. Under overlap=vpsc, stress_majorization_cola (lib/neatogen/
// constrained_majorization_ipsep.c) ends with removeoverlaps(orig_n, ...), and initCMajVPSC
// treats the last 2 * nclusters real nodes as the cluster boundary variables. C sizes
// generateNonoverlapConstraints' boxes before dropping the dummies, so cluster members among
// those nodes read zeroed boxes; the port sized them after and panicked. With fewer real nodes
// than 2 * nclusters, C segfaults (the first two graphs) and the port passes the dummies too.
#[test]
fn neato_ipsep_vpsc_with_clusters_does_not_panic() {
    for (src, nodes) in [
        (
            "digraph { layout=neato; mode=ipsep; overlap=vpsc; subgraph cluster_a { a } subgraph cluster_b { b } a -> b; b -> c }",
            3,
        ),
        (
            "digraph { layout=neato; mode=ipsep; overlap=vpsc; subgraph cluster_a { a } subgraph cluster_b { b } a -> b; b -> c; c -> d; d -> e }",
            5,
        ),
    ] {
        let (out, err, ok) = run(&["-Tsvg"], src);
        assert!(ok, "{src}: {err}");
        assert_eq!(err, "", "{src}");
        assert_eq!(out.matches("class=\"node\"").count(), nodes, "{src}: {out}");
        assert_eq!(out.matches("class=\"cluster\"").count(), 2, "{src}: {out}");
    }
    // C renders this one; n4 and n5 are the cluster's boundary variables in removeoverlaps.
    let src = "digraph { layout=neato; mode=ipsep; overlap=vpsc; subgraph cluster_0 { n0 } \
               n0 -> n1; n1 -> n2; n2 -> n3; n3 -> n4; n4 -> n5; }";
    let (out, err, ok) = run(&["-Tplain"], src);
    assert!(ok, "{err}");
    // C Graphviz 16.1.0.
    for line in [
        "graph 1 3.0059 1.7638",
        "node n0 1.5883 0.2916 0.75 0.5 n0 solid ellipse black lightgrey",
        "node n1 2.6309 0.80008 0.75 0.5 n1 solid ellipse black lightgrey",
        "node n2 1.3891 1.5138 0.75 0.5 n2 solid ellipse black lightgrey",
        "node n3 0.375 1.4439 0.75 0.5 n3 solid ellipse black lightgrey",
        "node n4 0.7271 0.25 0.75 0.5 n4 solid ellipse black lightgrey",
        "node n5 1.7698 0.90271 0.75 0.5 n5 solid ellipse black lightgrey",
    ] {
        assert!(out.contains(line), "{line}\n{out}");
    }
}

// Fuzz case 011-dot-render-node-lw-rw-ht (issue #20), its repro.gv verbatim. With
// `splines=""` dot_splines_ returns before resetRW, so the self-loop space dot_position added
// to ND_rw stays (lw=27, rw=45). Mcircle_hack draws the diagonals from ND_rw alone; C's
// chords run from x+29.76 to x-29.76, and a renderer that halves `width` draws them from
// x+23.81 to x-23.81.
#[test]
fn diagonals_use_the_self_loop_widened_right_width() {
    let src = "graph \"G\" {
splines=\"\";
\"n7\" [style=\"diagonals\"];
\"n7\" -- \"n7\":\"ne\" [];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(ok, "{err}");
    assert!(out.contains("points=\"56.76,-31.5 -2.76,-31.5\""), "{out}");
    assert!(out.contains("points=\"56.76,-4.5 -2.76,-4.5\""), "{out}");
}

// Issue #20, minimised from fuzz seed 21064. mark_clusters deletes `0` from the cluster
// with a warning, and install_in_rank then abandons the layout, so no postprocessing resets
// the node sizes: ND_lw+ND_rw and ND_ht keep the rotated rankdir=RL values (36 and 54) and
// C draws the ellipses rx=18 ry=27, and prints width=0.5 height=0.75.
#[test]
fn abandoned_flipped_layout_keeps_rotated_node_sizes() {
    let src = "digraph G {
rankdir=RL;
2 [label=\"\"];
subgraph cluster_0 {
0;
1 [shape=diamond];
2 -> 2;
}
{rank=source; 0;}
1 -> 2 [dir=back];
}
";
    let (out, err, ok) = run(&["-Kdot", "-Tsvg"], src);
    assert!(!ok);
    assert!(
        err.contains("Warning: 0 was already in a rankset, deleted from cluster G\n"),
        "{err}"
    );
    assert!(err.contains("Error: install_in_rank"), "{err}");
    assert_eq!(out.matches("rx=\"18\" ry=\"27\"").count(), 2, "{out}");
    let (out, _, _) = run(&["-Kdot", "-Tdot"], src);
    assert_eq!(out.matches("height=0.75").count(), 3, "{out}");
    assert_eq!(out.matches("width=0.5").count(), 3, "{out}");
}

// Fuzz case 001-dot-cluster-rank-max-build-skeleton-panic (issue #10), without its `pack="8"`
// line. Node n235 of cluster_1 is also in a top-level rank=max set; ranking leaves n102 of
// cluster4 on rank 2 while cluster4 spans rank 1 only. C's build_skeleton
// (lib/dotgen/cluster.c:365-366) reads GD_rankleader(subg)[ND_rank(v)], gets NULL and crashes
// (SIGSEGV, exit 139, no output) after printing mark_clusters' "already in a rankset"
// warning, so only that line can be compared. The port used to panic on the same lookup
// ("called `Option::unwrap()` on a `None` value", exit 101); it now fails the layout with an
// error. The case as generated, with `pack="8"`, is
// `pack_single_component_cluster_node_in_rank_max_set_lays_out_like_c` (issue #19).
const ISSUE10_GRAPH: &str = "strict digraph \"G\" {
\"n148\";
subgraph \"cluster_1\" {
\"n165\";
\"n181\" [];
\"n235\";
subgraph \"cluster4\" {
\"n102\" [];
\"n225\" [];
}
}
{
rank=max;
\"n235\";
\"n148\";
}
{\"n15\" \"n181\" \"n205\"} -> \"n102\";
\"n148\" -> \"n164\";
\"n164\" -> \"n165\";
}
";

#[test]
fn cluster_node_in_rank_max_set_does_not_panic_build_skeleton() {
    for fmt in ["-Tsvg", "-Tdot"] {
        let (_, err, ok) = run(&["-Kdot", fmt], ISSUE10_GRAPH);
        assert!(
            !ok,
            "{fmt}: C crashes here, the port reports a layout error"
        );
        assert_eq!(
            err,
            "Warning: n235 was already in a rankset, deleted from cluster G\n\
             Error: <stdin>: Layout computation error in dot: build_skeleton: no rank leader \
             for rank 2 of node n102 in cluster cluster4\n",
            "{fmt}"
        );
    }
}

// Issue #30 (tests/fuzz-open/021-stderr-missing-warnings): C warnings the port left out.
// Every expected string below is C Graphviz 16.1.0's stderr for the same input.

/// stderr of `-Tsvg -K<engine>` for `src`, with the run's success.
fn warnings(engine: &str, src: &str) -> (String, bool) {
    let k = format!("-K{engine}");
    let (_, err, ok) = run(&["-Tsvg", &k], src);
    (err, ok)
}

/// Runs the CLI on raw-byte arguments; returns stdout, stderr and the exit status.
#[cfg(unix)]
fn run_bytes(args: &[&[u8]], stdin: &str) -> (Vec<u8>, Vec<u8>, bool) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .args(args.iter().map(|a| OsStr::from_bytes(a)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(e) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
    }
    let out = child.wait_with_output().unwrap();
    (out.stdout, out.stderr, out.status.success())
}

/// Issue #197: `x` is in two clusters (#33), so `overlap=ipsep` gives VPSC containment
/// constraints it cannot satisfy. C's `satisfyVPSC` (`lib/vpsc/csolve_VPSC.cpp`) lets the
/// "Cycle Error!" `std::runtime_error` escape and aborts. The port went on projecting, each
/// cycle error splitting 10000 blocks that it never freed, without bound in time or memory.
/// It now reuses unused blocks and ends the layout after `MAX_CYCLE_ERRORS` cycle errors, with
/// a warning, and every node keeps a position of its own.
#[test]
fn ipsep_vpsc_cycle_error_ends_the_layout() {
    let src = "digraph { layout=neato; mode=ipsep; overlap=ipsep; \
               subgraph cluster_a { a; x; c } subgraph cluster_b { b; x; d } \
               a -> b; x -> a; c -> d; h -> {a b c d} }";
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .arg("-Tplain")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .unwrap();
    // The output is under 1 KB, well under the pipe buffer, so polling cannot block. The capped
    // layout takes about 2 s; the deadline is short because the runaway also grows memory.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the ipsep layout must terminate");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let (plain, err) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{err}");
    assert_eq!(
        err,
        "Warning: VPSC: too many cycle errors; ending the ipsep layout early\n"
    );
    let positions: std::collections::BTreeSet<(&str, &str)> = plain
        .lines()
        .filter_map(|l| l.strip_prefix("node "))
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f[1], f[2])
        })
        .collect();
    assert_eq!(positions.len(), 6, "{plain}");
}

/// Issue #152: C's `main` takes its arguments as bytes; a file name that is not valid UTF-8
/// (here Latin-1 `né.gv`) is opened, and named in errors with its own bytes.
#[cfg(unix)]
#[test]
fn non_utf8_file_name_argument_is_opened() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tmp("issue152");
    std::fs::create_dir_all(&dir).unwrap();
    let name = std::ffi::OsStr::from_bytes(b"n\xe9.gv");
    let path = dir.join(name);
    let path_bytes = path.as_os_str().as_bytes();

    // macOS (APFS) refuses names that are not UTF-8; Linux keeps them.
    if std::fs::write(&path, "digraph{a->b}").is_ok() {
        let (out, err, ok) = run_bytes(&[b"-Tsvg", path_bytes], "");
        assert!(ok, "{}", String::from_utf8_lossy(&err));
        assert_eq!(String::from_utf8_lossy(&out).matches("<svg").count(), 1);

        // -O writes `<input>.svg` next to the input, with the input's bytes.
        let (_, err, ok) = run_bytes(&[b"-Tsvg", b"-O", path_bytes], "");
        assert!(ok, "{}", String::from_utf8_lossy(&err));
        let mut auto = path.clone().into_os_string();
        auto.push(".svg");
        assert!(std::fs::read(&auto).unwrap().starts_with(b"<?xml"));

        // -o, separate and attached.
        let target = dir.join(std::ffi::OsStr::from_bytes(b"out\xe9.svg"));
        let target_bytes = target.as_os_str().as_bytes();
        let (_, err, ok) = run_bytes(&[b"-Tsvg", b"-o", target_bytes, path_bytes], "");
        assert!(ok, "{}", String::from_utf8_lossy(&err));
        assert!(std::fs::read(&target).unwrap().starts_with(b"<?xml"));
        std::fs::remove_file(&target).unwrap();
        let attached = [b"-o", target_bytes].concat();
        let (_, err, ok) = run_bytes(&[b"-Tsvg", &attached, path_bytes], "");
        assert!(ok, "{}", String::from_utf8_lossy(&err));
        assert!(std::fs::read(&target).unwrap().starts_with(b"<?xml"));
    }

    // A missing file is an error naming its bytes, not a panic.
    let missing = dir.join(std::ffi::OsStr::from_bytes(b"missing\xe9.gv"));
    let missing = missing.as_os_str().as_bytes();
    let (_, err, ok) = run_bytes(&[b"-Tsvg", missing], "");
    assert!(!ok);
    let expected = [b"can't open ", missing, b": "].concat();
    assert!(
        err.windows(expected.len()).any(|w| w == expected),
        "{}",
        String::from_utf8_lossy(&err)
    );
}

/// Issue #152: other flags with bytes that are not UTF-8 behave as C's do
/// (`lib/common/input.c:dotneato_args_initialize`): the values keep their bytes.
#[cfg(unix)]
#[test]
fn non_utf8_flag_values_keep_their_bytes() {
    let (out, _, ok) = run_bytes(&[b"-Tdot", b"-Gx\xe9=1"], "digraph{a}");
    assert!(ok);
    assert!(out.windows(5).any(|w| w == b"x\xe9=1\n"), "{out:?}");

    let (_, err, ok) = run_bytes(&[b"-T\xe9"], "digraph{a}");
    assert!(!ok);
    assert!(
        err.starts_with(b"Format: \"\xe9\" not recognized."),
        "{err:?}"
    );

    // `fprintf`, not `agerrorf`: control bytes are written as they are. A char of the
    // range the port uses for raw bytes (U+10FF54) keeps its four bytes.
    let (_, err, ok) = run_bytes(&[b"-T\x01\xf4\x8f\xbd\x94"], "digraph{a}");
    assert!(!ok);
    assert!(
        err.starts_with(b"Format: \"\x01\xf4\x8f\xbd\x94\" not recognized."),
        "{err:?}"
    );

    // The same as a separate argument, and as a -G value.
    let (_, err, ok) = run_bytes(&[b"-T", b"\xf4\x8f\xbd\x94"], "digraph{a}");
    assert!(!ok);
    assert!(err.starts_with(b"Format: \"\xf4\x8f\xbd\x94\" not recognized."));
    let (out, _, ok) = run_bytes(&[b"-Tdot", b"-Gx=\xf4\x8f\xbd\x94"], "digraph{a}");
    assert!(ok);
    assert!(
        out.windows(7).any(|w| w == b"x=\xf4\x8f\xbd\x94\n"),
        "{out:?}"
    );

    // atoi skips only C's white space, not U+00A0.
    let (_, err, ok) = run_bytes(&[b"-n\xc2\xa01"], "digraph{a}");
    assert!(!ok);
    assert!(err.starts_with(b"Error: Invalid parameter \"\xc2\xa01\" for -n flag\n"));

    let (_, err, ok) = run_bytes(&[b"-K\xe9"], "digraph{a}");
    assert!(!ok);
    assert!(err.starts_with(b"There is no layout engine support for \"\xe9\""));

    let (_, err, ok) = run_bytes(&[b"-n\xe9"], "digraph{a}");
    assert!(!ok);
    assert!(err.starts_with(b"Error: Invalid parameter \"\xe9\" for -n flag\n"));

    // C switches on the byte after `-`, even inside a multi-byte UTF-8 char (`-é`).
    // A char of the range the port uses for raw bytes (U+10FF54 here) is split the same way.
    for (arg, flag) in [
        (&b"-\xe9"[..], &b"-\xe9 "[..]),
        (b"-\xc3\xa9", b"-\xc3 "),
        (b"-\xf4\x8f\xbd\x94svg", b"-\xf4 "),
        (b"-\xf4\x8f\xbd\x87x=1", b"-\xf4 "),
    ] {
        let (_, err, ok) = run_bytes(&[arg], "digraph{a}");
        assert!(!ok);
        let expected = [b"option ", flag, b"unrecognized"].concat();
        assert!(
            err.windows(expected.len()).any(|w| w == expected),
            "{}",
            String::from_utf8_lossy(&err)
        );
    }
}

// Issue #153: `-q` (`-q1`) sets cgraph's message level to errors (`agseterr(AGERR)`, args.c),
// `-q2` and up turn every message off, and `-q0` is reported and ignored. An error that is not
// written still fails the run (`agerrors()`). stderr and exit status are C 16.1.0's.
#[test]
fn q_drops_warnings_and_q2_drops_errors_like_c() {
    let src = "digraph { a [shape=foo]; b [shape=record,label=\"{a|\"] }";
    let both = "Warning: using box for unknown shape foo\nError: bad label format {a|\n";
    for (flag, want) in [
        (None, both.to_string()),
        (Some("-q"), "Error: bad label format {a|\n".to_string()),
        (Some("-q1"), "Error: bad label format {a|\n".to_string()),
        (Some("-q2"), String::new()),
        (Some("-q5"), String::new()),
        (
            Some("-q0"),
            format!("Invalid parameter \"0\" for -q flag - ignored\n{both}"),
        ),
    ] {
        let mut args = vec!["-Tsvg"];
        args.extend(flag);
        let (out, err, ok) = run(&args, src);
        assert_eq!(err, want, "{flag:?}");
        assert!(!ok, "{flag:?}: the error still sets the exit status");
        assert!(out.contains("</svg>"), "{flag:?}");
    }
}

// `-q` applies from where it is read (dotneato_args_initialize), so it drops the error of an
// unrecognized option after it; `-n` is read before (`neato_extra_args`), so its error is
// written whatever `-q` says. C 16.1.0 writes the usage after both.
#[test]
fn q2_drops_later_option_errors_but_not_the_n_flag_error() {
    let (_, err, ok) = run(&["-q2", "-Z"], "graph{}");
    assert!(!ok);
    assert!(err.starts_with("Usage: "), "{err}");
    let (_, err, ok) = run(&["-Z", "-q2"], "graph{}");
    assert!(!ok);
    // The program name is `graphviz-pure.exe` on Windows.
    assert!(
        err.starts_with("Error: graphviz-pure")
            && err.contains(": option -Z unrecognized\n\nUsage: "),
        "{err}"
    );
    let (_, err, ok) = run(&["-q2", "-n0"], "graph{}");
    assert!(!ok);
    assert!(
        err.starts_with("Error: Invalid parameter \"0\" for -n flag\nUsage: "),
        "{err}"
    );
}

// Issue #121: cgraph's `default_usererrf` (agerror.c) writes control characters other than
// white space as `\ooo`, in every message: the color warning of `gvrender_resolve_color`, the
// style warning of `gvrender_set_style` and the arrow warning of `arrow_match_name`. stderr is
// C 16.1.0's.
#[test]
fn control_characters_in_warnings_are_escaped_like_c() {
    let src = "digraph { a [color=\"x\u{1}y\", style=\"foo\u{2}\"]; \
               a -> b [arrowhead=\"zz\u{3}\"] }";
    let (_, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: Arrow type \"zz\\003\" unknown - ignoring\n\
         Warning: gvrender_set_style: unsupported style foo\\002 - ignoring\n\
         Warning: x\\001y is not a known color.\n"
    );
}

// Render warnings worded as C 16.1.0's: `firstlayer` names the output format, the
// `layersep`/`layerlistsep` warning names the character, and `parseSegs`' "Total size > 1"
// has no newline, so the next message continues its line.
#[test]
fn render_warnings_are_worded_like_c() {
    let (_, err, _) = run(
        &["-Txdot"],
        "digraph { layers=\"a:b\"; layersep=\":\"; layerlistsep=\":\"; a }",
    );
    assert_eq!(
        err,
        "Warning: The character ':' appears in both the layersep and layerlistsep attributes \
         - layerlistsep ignored.\n\
         Warning: layers not supported in xdot output\n"
    );
    let (_, err, _) = run(
        &["-Tsvg"],
        "digraph { a [style=filled fillcolor=\"red;0.8:blue;0.7\"] }",
    );
    assert!(
        err.starts_with(
            "Warning: Total size > 1 in \"red;0.8:blue;0.7\" color spec Warning: red;0.8 is \
             not a known color.\n"
        ),
        "{err}"
    );
    // `parseSegs` reports only the first bad color spec of the process (`static atomic_flag
    // warned`): the later ones say nothing.
    let (_, err, _) = run(
        &["-Tsvg"],
        "digraph { a [style=filled fillcolor=\"red;0.8:blue;0.7\"]; \
         b [style=filled fillcolor=\"red;x:blue\"]; c [style=filled fillcolor=\"red;x:blue\"] }",
    );
    assert_eq!(
        err,
        "Warning: Total size > 1 in \"red;0.8:blue;0.7\" color spec Warning: red;0.8 is not a \
         known color.\nWarning: red;x is not a known color.\n"
    );
}

// The callers of `parseSegs` continue its message with `agerr(AGPREV, ...)`: `in node`
// (`poly_gencode` for wedged and striped), `in cluster` (`emit_clusters`) and `in edge`
// (`multicolor`). One run each: C reports only the first bad spec of a process.
#[test]
fn color_spec_messages_name_the_object_like_c() {
    for (src, want) in [
        (
            "digraph { a [shape=box style=striped fillcolor=\"red;0.8:blue;0.7\"] }",
            "Warning: Total size > 1 in \"red;0.8:blue;0.7\" color spec in node a\n",
        ),
        (
            "digraph { a [style=wedged fillcolor=\"red;x:blue\"] }",
            "Error: Illegal value in \"red;x:blue\" color attribute; float expected after ';'\n\
             in node a\n",
        ),
        (
            "digraph { subgraph cluster_c { style=striped fillcolor=\"red;0.8:blue;0.7\"; a } }",
            "Warning: Total size > 1 in \"red;0.8:blue;0.7\" color spec in cluster cluster_c\n",
        ),
        (
            "digraph { a -> b [color=\"red;0.8:blue;0.7\"] }",
            "Warning: Total size > 1 in \"red;0.8:blue;0.7\" color spec in edge a -> b\n",
        ),
        (
            "graph { a -- b [color=\"red;x:blue\"] }",
            "Error: Illegal value in \"red;x:blue\" color attribute; float expected after ';'\n\
             in edge a -- b\n",
        ),
    ] {
        let (_, err, _) = run(&["-Tsvg"], src);
        assert_eq!(err, want, "{src}");
    }
}

// `neato_extra_args` takes `-n` out of the arguments before the others are read, so the file
// after `-o -n2` is the output file, as in C 16.1.0.
#[test]
fn n_flag_is_taken_out_before_the_other_options_like_c() {
    let path = tmp("n_flag_taken_out.svg");
    let _ = std::fs::remove_file(&path);
    let (_, err, ok) = run(
        &["-o", "-n2", path.to_str().unwrap(), "-Tsvg"],
        "digraph { a [pos=\"0,0\"] }",
    );
    assert!(ok, "{err}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("</svg>"));
}

// Issue #150 (its message only): `init_nop` reports a node with no position and
// `neato_layout` continues the message with `agerr(AGPREV, ...)`, on a second line, as
// C 16.1.0 does. `-q2` drops both lines, the continuation at the level of the first. The port's
// `Layout computation error` line after them goes with the fatal `LayoutError`, which the
// `-n` rework (C8, the rest of #150) replaces.
#[test]
fn neato_n_missing_pos_is_reported_on_two_lines_like_c() {
    let src = "graph A { a; b [pos=\"0,0\"] }";
    let (_, err, ok) = run(&["-Kneato", "-n", "-Tsvg"], src);
    assert!(!ok);
    assert!(
        err.starts_with("Error: node a in graph A has no position\nas required by the -n flag\n"),
        "{err}"
    );
    let (_, err, _) = run(&["-q2", "-Kneato", "-n", "-Tsvg"], src);
    assert!(!err.contains("as required"), "{err}");
}

// Issues #255, #256, #257 (C22): `lib/neatogen/stuff.c`, `sgd.c`, `neatoinit.c` and `stress.c`
// warnings the port handled silently. A component is named `_neato_cc<i>`, as `pccomps` names it.
#[test]
fn neato_stuff_sgd_neatoinit_warn_like_c() {
    for (args, src, want, code) in [
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { a -- b [len=-1]; b -- c }",
            "Warning: bad edge len \"-1\" in G - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G {pack=false; a -- b [len=-1]; c -- d [len=-2]}",
            "Warning: bad edge len \"-1\" in _neato_cc0 - setting to 1.00\nWarning: bad edge len \"-2\" in _neato_cc1 - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=KK; a -- b [len=x]; c -- d }",
            "Warning: bad edge len \"x\" in G - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G {mode=sgd; a -- b [len=0]}",
            "Warning: bad edge len \"0\" in G - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=KK; maxiter=1; a -- b -- c -- a; c -- d }",
            "Warning: Max. iterations (1) reached on graph G\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=KK; start=self; a -- b -- c; d -- e }",
            "Warning: start=0 not supported with mode=self - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=sgd; model=circuit; a -- b -- c }",
            "Warning: circuit model not yet supported in Gmode=sgd, reverting to shortpath model\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=sgd; model=mds; a -- b [len=2]; b -- c }",
            "Warning: mds model not yet supported in Gmode=sgd, reverting to shortpath model\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { start=regular; a [pos=\"1,1\"]; a -- b -- c }",
            "Warning: node positions are ignored unless start=random\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=KK; start=self; a [pos=\"1,1\"]; a -- b -- c }",
            "Warning: node positions are ignored unless start=random\nWarning: start=0 not supported with mode=self - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { stresswt=3; a -- b -- c }",
            "Warning: stresswt attribute value must be 1 or 2 - ignoring\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { model=bogus; a -- b }",
            "Warning: Unknown value bogus for attribute \"model\" in graph G - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { mode=bogus; a -- b }",
            "Warning: Illegal value bogus for attribute \"mode\" in graph G - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph { mode=bogus; model=bogus; a -- b }",
            "Warning: Illegal value bogus for attribute \"mode\" in graph %1 - ignored\nWarning: Unknown value bogus for attribute \"model\" in graph %1 - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { model=mds; a -- b }",
            "Warning: edges in graph G have no len attribute. Hence, the mds model\nis inappropriate. Reverting to the shortest path model.\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { model=circuit; mode=major; a [pos=\"0,0!\"]; c [pos=\"1,1!\"]; a -- b; c -- d }",
            "Warning: graph is disconnected. Hence, the circuit model\nis undefined. Reverting to the shortest path model.\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { model=circuit; mode=ipsep; a [pos=\"0,0!\"]; c [pos=\"1,1!\"]; a -- b; c -- d }",
            "Warning: graph is disconnected. Hence, the circuit model\nis undefined. Reverting to the shortest path model.\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { model=circuit; mode=hier; a [pos=\"0,0!\"]; c [pos=\"1,1!\"]; a -- b; c -- d }",
            "Warning: node positions are ignored unless start=random\nWarning: graph is disconnected. Hence, the circuit model\nis undefined. Reverting to the shortest path model.\n",
            0,
        ),
        (
            &["-Kneato", "-q", "-Tplain"][..],
            "graph G { mode=bogus; a -- b [len=-1] }",
            "",
            0,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Issues #258, #259, #262 (C22): `mincross.c`'s `ordering` errors raise the exit status;
// `aspect.c` and `dotsplines.c` warn. One aspect warning per packed component, as `dotLayout` gives.
#[test]
fn dot_ordering_aspect_curved_diagnostics_like_c() {
    for (args, src, want, code) in [
        (
            &["-Tplain"][..],
            "digraph G { aspect=2; a -> b }",
            "Warning: the aspect attribute has been disabled due to implementation flaws - attribute ignored.\n",
            0,
        ),
        (&["-Tplain"][..], "digraph G { aspect=x; a -> b }", "", 0),
        (
            &["-Tplain"][..],
            "digraph G { aspect=\"1.5,3\"; pack=true; a -> b; c -> d }",
            "Warning: the aspect attribute has been disabled due to implementation flaws - attribute ignored.\nWarning: the aspect attribute has been disabled due to implementation flaws - attribute ignored.\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { splines=curved; a -> b [label=x] }",
            "Warning: edge labels with splines=curved not supported in dot - use xlabels\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { ordering=bogus; a -> b; a -> c }",
            "Error: ordering 'bogus' not recognized.\n",
            1,
        ),
        (
            &["-Tplain"][..],
            "digraph G { a [ordering=bogus]; a -> b; a -> c }",
            "Error: ordering 'bogus' not recognized for node 'a'.\n",
            1,
        ),
        (
            &["-Tplain"][..],
            "digraph G { subgraph cluster_x { ordering=bad; a -> b; a -> c } }",
            "Error: ordering 'bad' not recognized.\n",
            1,
        ),
        (
            &["-q", "-Tplain"][..],
            "digraph G { ordering=bogus; a -> b }",
            "Error: ordering 'bogus' not recognized.\n",
            1,
        ),
        (
            &["-Tplain"][..],
            "digraph G { nodesep=\"2147483647\"; a; b }",
            "Error: Edge length 2147483701.000000 larger than maximum 2147483647 allowed.\nCheck for overwide node(s).\n",
            1,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Issues #262, #307 (C22): twopi's `circle.c` error for a weight=0 component and
// `twopiinit.c`'s warning for a `root` that names no node (C prints no space after the full stop).
#[test]
fn twopi_weight0_and_missing_root_diagnostics_like_c() {
    for (args, src, want, code) in [
        (
            &["-Ktwopi", "-Tplain"][..],
            "graph G { a -- b [weight=0]; a -- c }",
            "Error: twopi: use of weight=0 creates disconnected component.\n",
            1,
        ),
        (
            &["-Ktwopi", "-Tplain"][..],
            "graph { root=zz; a -- b }",
            "Warning: specified root node \"zz\" was not found.Using default calculation for root node\n",
            0,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Issues #260 (pagedir), #275, #287 (C22): `init_job_pagination`'s pagedir warning, once per
// output; sfdp's `spring_electrical_embedding_slow` trace (no newline); `routesplines_`'s
// box-space warning, naming the real edge.
#[test]
fn emit_pagedir_sfdp_slow_and_routespl_messages_like_c() {
    for (args, src, want, code) in [
        (
            &["-Tplain"][..],
            "digraph G { pagedir=XX; a -> b }",
            "Warning: pagedir=XX ignored\n",
            0,
        ),
        (
            &["-Ksfdp", "-Tplain"][..],
            "graph{quadtree=\"0\"; a--b; b--c}",
            "spring_electrical_embedding_slow",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { subgraph cluster0 { c; f subgraph cluster0_in { b } } b -> h [tailport=sw,minlen=0]; }",
            "Warning: Unable to reclaim box space in spline routing for edge \"b\" -> \"h\". Something is probably seriously wrong.\n",
            0,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Cause C22 siblings: `compound.c`'s lhead/ltail warnings, `utils.c`'s duplicate cluster and
// unknown `splines` warnings (again per neato and sfdp component, as `setEdgeType` runs there),
// and cgraph's `attribute macros not implemented` (no newline).
#[test]
fn compound_cluster_splines_and_macro_warnings_like_c() {
    for (args, src, want, code) in [
        (
            &["-Tplain"][..],
            "digraph G { compound=true; subgraph cluster_a { subgraph cluster_x {a} } subgraph cluster_b { subgraph cluster_x {b} } a -> b [lhead=cluster_x] }",
            "Warning: Two clusters named cluster_x - the second will be ignored\nWarning: a -> b: head not inside head cluster cluster_x\n",
            0,
        ),
        (
            &["-Kfdp", "-Tplain"][..],
            "digraph G { subgraph cluster_a { subgraph cluster_x {a} } subgraph cluster_b { subgraph cluster_x {b} } a -> b }",
            "Warning: Two clusters named cluster_x - the second will be ignored\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { compound=true; subgraph cluster_x {b} a -> c [lhead=cluster_x] }",
            "Warning: a -> c: head not inside head cluster cluster_x\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { compound=true; subgraph cluster_x {a b} a -> b [lhead=cluster_x] }",
            "Warning: a -> b: tail is inside head cluster cluster_x\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { compound=true; subgraph cluster_x {b} a -> c [ltail=cluster_x] }",
            "Warning: a -> c: tail not inside tail cluster cluster_x\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { compound=true; subgraph cluster_x {a b} a -> b [ltail=cluster_x] }",
            "Warning: a -> b: head is inside tail cluster cluster_x\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { subgraph cluster_a { subgraph cluster_x {a} } subgraph cluster_b { subgraph cluster_x {b} } a -> b [lhead=cluster_x] }",
            "",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph G { splines=bogus; a -> b }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph { node m = [color=red]; a }",
            "Warning: attribute macros not implemented",
            0,
        ),
        (
            &["-Kdot", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kdot", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kfdp", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kfdp", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Ksfdp", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Ksfdp", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\nWarning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Ktwopi", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Ktwopi", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kcirco", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kcirco", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kosage", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kosage", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kpatchwork", "-Tplain"][..],
            "graph G { splines=bogus; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kpatchwork", "-Tplain"][..],
            "graph G { splines=bogus; pack=true; a -- b; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Issue #255 (C22): C prints the KK circuit model's warning, in four lines, then crashes on
// this input some of the time (undefined behaviour after the warning); the port goes on with
// the shortest-path model.
#[test]
fn neato_kk_disconnected_circuit_model_warns_like_c() {
    let (_, err, status) = run_in(
        &std::env::temp_dir(),
        &["-Kneato", "-Tplain"],
        "graph G { model=circuit; mode=KK; a -- b; c -- d }",
    );
    assert_eq!(
        (err.as_str(), status),
        (
            "Warning: graph G is disconnected. Hence, the circuit model\n\
             is undefined. Reverting to the shortest path model.\n\
             Alternatively, consider running neato using -Gpack=true or decomposing\n\
             the graph into connected components.\n",
            0
        )
    );
}

// Cause C22, review of #318: the `splines` warning once under `-x` and `-n` (the port's
// repeat graphs are built `silently`); compound.c's `getCluster`; fdp's `start=self`
// (tlayout.c); sgd's `initial_positions`; and `scan_graph`'s `bad edge len` under `-n`, where
// `len=0` is good.
#[test]
fn review_c22_siblings_like_c() {
    for (args, src, want, code) in [
        (
            &["-Kneato", "-x", "-Tplain"][..],
            "graph G { splines=bogus; a -- b -- c -- a; c -- d }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-n", "-Tplain"][..],
            "graph G { splines=bogus; node [pos=\"0,0\"]; a -- b; b [pos=\"100,100\"] }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-n2", "-Tplain"][..],
            "graph G { splines=bogus; node [pos=\"0,0\"]; a -- b; b [pos=\"100,100\"] }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Kpatchwork", "-Tplain"][..],
            "graph G { splines=bogus; a -- b }",
            "Warning: Unknown \"splines\" value: \"bogus\" - ignored\n",
            0,
        ),
        (
            &["-Tplain"][..],
            "digraph { compound=true; subgraph cluster_a { a } b -> a [lhead=cluster_zz, ltail=nope] }",
            "Warning: cluster named cluster_zz not found\nWarning: cluster named nope not found\n",
            0,
        ),
        (
            &["-Kfdp", "-Tplain"][..],
            "graph { start=self; a -- b }",
            "Warning: fdp does not support start=self - ignoring\n",
            0,
        ),
        (
            &["-Kfdp", "-Tplain"][..],
            "graph { start=self; subgraph cluster_x { a -- b } c -- a }",
            "Warning: fdp does not support start=self - ignoring\n",
            0,
        ),
        (
            &["-Kneato", "-Gmode=sgd", "-Tplain"][..],
            "graph { start=self; a -- b }",
            "Warning: start=0 not supported with mode=self - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-Gmode=sgd", "-Tplain"][..],
            "graph { start=self; a -- b; c -- d }",
            "Warning: start=0 not supported with mode=self - ignored\n",
            0,
        ),
        (
            &["-Kneato", "-n", "-Tplain"][..],
            "graph G { node [pos=\"0,0\"]; a -- b [len=-1]; b -- c [len=0]; c -- d [len=abc] }",
            "Warning: bad edge len \"-1\" in G - setting to 1.00\nWarning: bad edge len \"abc\" in G - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-n", "-Tplain"][..],
            "graph G { node [pos=\"0,0\"]; c -- a [len=-3]; a -- b [len=-1]; b -- c [len=x] }",
            "Warning: bad edge len \"-3\" in G - setting to 1.00\nWarning: bad edge len \"-1\" in G - setting to 1.00\nWarning: bad edge len \"x\" in G - setting to 1.00\n",
            0,
        ),
        (
            &["-Kneato", "-n2", "-Tplain"][..],
            "graph { node [pos=\"0,0\"]; a -- b [len=-1] }",
            "Warning: bad edge len \"-1\" in %1 - setting to 1.00\n",
            0,
        ),
    ] {
        let (_, err, status) = run_in(&std::env::temp_dir(), args, src);
        assert_eq!((err.as_str(), status), (want, code), "{args:?} {src}");
    }
}

// Issue #311 (C22): `legal.c`'s `find_ints` reports a backward edge with no active line as an
// error, so the exit status is 1, and the arrangement is illegal (straight lines), as in C.
#[test]
fn fdp_legal_arrangement_non_line_is_an_error_like_c() {
    let src = "graph { splines=polyline; overlap=true; n0 [pos=\"0,1e-300!\"]; n1 [pos=\"2.5,nan!\"]; n2 [pos=\"1e-300,1e-300\"]; n3 [pos=\"-1e7,2.5!\", fontsize=nan]; n4 [pos=\"0,3e6\"]; n0 -- n1; n3 -- n4; n4 -- n1; n4 -- n1; n1 -- n0; n1 -- n2; n4 -- n0 }";
    let (_, err, status) = run_in(&std::env::temp_dir(), &["-Kfdp", "-Tdot"], src);
    assert_eq!(
        (err.as_str(), status),
        (
            "Error: trying to delete a non-line\n\
             Warning: some nodes with margin (3.20,3.20) touch - falling back to straight line \
             edges\n",
            1
        )
    );
}

// Issue #261 (C22): `eatComment` warns about a comment closed by a `>` without `--`. The error
// after it is still the port's (C's expat reads on into the following units); only the warning
// is checked.
#[test]
fn html_unclosed_comment_warns_like_c() {
    let (_, err, status) = run_in(
        &std::env::temp_dir(),
        &["-Tplain"],
        "digraph G { a [label=<hello<!-- x >world>] }",
    );
    assert!(
        err.starts_with("Warning: Unclosed comment\nError: "),
        "{err}"
    );
    assert_eq!(status, 1);
}

/// The issue's reproducer, verbatim: an illegal HTML attribute, an unknown port and an edge
/// label with no spline to place it on.
#[test]
fn issue30_reproducer_warnings() {
    let src = "graph G {
 splines=none
 a [xlabel=q, label=<<TABLE FOO=\"1\"><TR><TD>x</TD></TR></TABLE>>]
 a:nosuch -- b [label=x]
}
";
    let (err, ok) = warnings("dot", src);
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Illegal attribute FOO in <TABLE> - ignored\n\
         in label of node a\n\
         Warning: node a, port nosuch unrecognized\n\
         Warning: no position for edge with label x\n"
    );
}

/// `unrecognized` and the compass warnings of `poly_port` and `record_port`
/// (`lib/common/shapes.c:362-365`, `2894-2896`, `2909-2910`, `3748-3752`), once per
/// `chkPort` call of `common_init_edge`, in `agfstnode`/`agfstout` order.
#[test]
fn issue30_unknown_port_warnings() {
    let src = "digraph G {
 a -> b:nosuch
 r [shape=record, label=\"<f0> x|<f1> y\"]
 r:f0:zz -> r:bad
 r:f1:s -> a:ne
 h [label=<<TABLE><TR><TD PORT=\"p\">x</TD></TR></TABLE>>]
 h:p:qq -> h:missing
 c [shape=box]
 c:wat -> a
 a -> c:wat
}
";
    let want = "Warning: node b, port nosuch unrecognized\n\
                Warning: node c, port wat unrecognized\n\
                Warning: node r, port f0, unrecognized compass point 'zz' - ignored\n\
                Warning: node r, port bad unrecognized\n\
                Warning: node h, port p, unrecognized compass point 'qq' - ignored\n\
                Warning: node h, port missing unrecognized\n\
                Warning: node c, port wat unrecognized\n";
    for engine in ["dot", "neato", "fdp", "circo"] {
        assert_eq!(warnings(engine, src).0, want, "{engine}");
    }
    // patchwork_init_edge never calls common_init_edge, so C resolves no ports.
    assert_eq!(warnings("patchwork", src).0, "");
}

// Issues #102, #106 and #108: the writers read the subgraph membership dot's layout left
// (node_induce's `agsubedge` and `agdelete`, mark_clusters' `agdelete`). Expected output
// is Graphviz 16.1.0's.
#[test]
fn writers_read_the_membership_dot_left() {
    let canon = |src: &str| {
        let (out, _, ok) = run(&["-Kdot", "-Tcanon"], src);
        assert!(ok, "{src}");
        out
    };
    // #106: the root edge between a cluster's nodes is the cluster's.
    let src = "digraph G { subgraph cluster_1 { a; b; } a -> b; }";
    assert_eq!(
        canon(src),
        "digraph G {\n\tnode [label=\"\\N\"];\n\tsubgraph cluster_1 {\n\t\ta -> b;\n\t}\n}\n"
    );
    let (json, _, ok) = run(&["-Kdot", "-Tdot_json"], src);
    assert!(ok);
    assert!(
        json.contains(
            "\"nodes\": [\n        1,2\n      ],\n      \"edges\": [\n        0\n      ]"
        ),
        "{json}"
    );
    // #102: v6 is deleted from cluster_2, an earlier sibling holds it.
    let src = "digraph G {\nsubgraph cluster_0 { v6 }\nsubgraph cluster_2 { v6 -> v0; v1 -> v5; }\n\
               v1 -> v0;\n}\n";
    assert_eq!(
        canon(src),
        "digraph G {\n\tnode [label=\"\\N\"];\n\tsubgraph cluster_0 {\n\t\tv6;\n\t}\n\
         \tsubgraph cluster_2 {\n\t\tv1 -> v0;\n\t\tv1 -> v5;\n\t}\n\tv6 -> v0;\n}\n"
    );
    // #102, #108: x is deleted from cluster_a, it is in a rank set.
    let src = "digraph G { subgraph cluster_a { x; y; x -> y } {rank=same; x; z} }";
    let (out, err, ok) = run(&["-Kdot", "-Tcanon"], src);
    assert!(ok);
    assert_eq!(
        err,
        "Warning: x was already in a rankset, deleted from cluster G\n"
    );
    assert_eq!(
        out,
        "digraph G {\n\tnode [label=\"\\N\"];\n\tsubgraph cluster_a {\n\t\ty;\n\t}\n\t{\n\
         \t\tgraph [rank=same];\n\t\tx;\n\t\tz;\n\t}\n\tx -> y;\n}\n"
    );
    // #102: with pack and several components the original cluster keeps its members.
    let src = "digraph G {\npack=true;\nsubgraph cluster_2 { a -> b; c; }\nc -> b;\nx;\n}\n";
    assert_eq!(
        canon(src),
        "digraph G {\n\tgraph [pack=true];\n\tnode [label=\"\\N\"];\n\tsubgraph cluster_2 {\n\
         \t\ta -> b;\n\t\tc;\n\t}\n\tc -> b;\n\tx;\n}\n"
    );
}

// Issue #345: dot's pack names an anonymous cluster `agnameof(subg)`, `%<id>`, in
// deriveClusters and in projectG's copies, where findCluster finds it. Graphviz 16.1.0
// lays these out with no message.
#[test]
fn dot_pack_keeps_the_names_of_anonymous_clusters() {
    for src in [
        "digraph{pack=true; subgraph {cluster=true; a} subgraph {cluster=true; b} a->b; z}",
        "digraph{compound=true; pack=true; subgraph {cluster=true; a} z; x->a [lhead=\"%3\"]}",
        "digraph{compound=true; pack=true; subgraph {cluster=true; a; subgraph {cluster=true; c}} \
         c->a [ltail=\"%5\"]; q->r}",
    ] {
        let (_, err, ok) = run(&["-Kdot", "-Tdot"], src);
        assert!(ok, "{src}");
        assert_eq!(err, "", "{src}");
    }
    // Two components, side by side.
    let (out, _, ok) = run(
        &["-Kdot", "-Tdot"],
        "digraph{pack=true; subgraph {cluster=true; a} subgraph {cluster=true; b}}",
    );
    assert!(ok);
    assert!(out.contains("graph [bb=\"0,0,166,140\","), "{out}");
}

/// `doAttrs` and its attribute actions (`lib/common/htmllex.c:80-552`; the `doInt`
/// messages have no newline), `size_html_cell`'s fixed-size check
/// (`lib/common/htmltable.c:1141-1153`), then `make_label`'s `in label of ...`
/// (`lib/common/labels.c:148-160`).
#[test]
fn issue30_html_attribute_warnings() {
    let src = "digraph { a [label=<<TABLE BORDER=\"-1\" STYLE=\"rounded,wiggly\" SIDES=\"LTx\" \
COLUMNS=\"x\"><TR><TD COLSPAN=\"0\" ALIGN=\"up\" FIXEDSIZE=\"TRUE\" WIDTH=\"3\" HEIGHT=\"3\">long \
text</TD></TR></TABLE>>]; b [label=<<FONT POINT-SIZE=\"big\">f</FONT><BR CLEAR=\"all\"/>>]; \
a -> b [label=<<TABLE FOO=\"1\"><TR><TD>x</TD></TR></TABLE>>] }";
    let want = "Warning: BORDER value -1 < 0 - too small - ignored\
                Warning: Illegal value wiggly for STYLE - ignored\n\
                Warning: Unrecognized character 'x' (120) in sides attribute\n\
                Warning: Unknown value x for COLUMNS - ignored\n\
                Warning: COLSPAN value cannot be 0 - ignored\n\
                Warning: Illegal value up for ALIGN in TD - ignored\n\
                Warning: cell size too small for content\n\
                in label of node a\n\
                Warning: Improper POINT-SIZE value big - ignored\
                Warning: Illegal attribute CLEAR in <BR> - ignored\n\
                in label of node b\n\
                Warning: Illegal attribute FOO in <TABLE> - ignored\n\
                in label of edge a -> b\n";
    for engine in ["dot", "neato"] {
        let (err, ok) = warnings(engine, src);
        assert!(ok, "{engine}");
        assert_eq!(err, want, "{engine}");
    }
}

/// Issue #198: after `overlap=ipsep`, m1 cannot be reached from m3 among the obstacles
/// fdp's `compoundEdges` (`lib/fdpgen/clusteredges.c`) builds, so `shortestPath` leaves
/// `dad[N] == -1`. C's `Pobspath` (`lib/pathplan/cvt.c`) then reads `dad[-1]`: it hangs on
/// `tests/fdp/hw_compound_mid.gv` and routes through garbage on this reduction. The port
/// panicked; now it warns and routes the edges without compound edges.
#[test]
fn compound_edge_without_a_path_falls_back_instead_of_panicking() {
    let src = "graph { layout=fdp; splines=compound; overlap=ipsep;
subgraph cluster_mid { m1 -- m2; m3 -- m1; m4 -- m1 }
a -- m1; b -- m2; c -- m3; d -- c }
";
    let (out, err, ok) = run(&["-Tdot"], src);
    assert!(ok && !err.contains("panicked"), "{err}");
    // C's own warnings, plus the port's for the missing path.
    assert_eq!(
        err,
        "Warning: compoundEdges: could not find a path - falling back to straight line edges\n\
         Warning: compoundEdges: nodes touch - falling back to straight line edges\n\
         Warning: some nodes with margin (3.20,3.20) touch - falling back to straight line edges\n"
    );
    assert!(
        out.contains("m3 -- m1\t[pos=\"83.797,51.424 80.429,48.095 76.858,44.566 73.484,41.231\"]"),
        "m3 -- m1 falls back to a straight line:\n{out}"
    );
}

/// A root graph label is made by `graph_init` (`lib/common/input.c:711`), before the
/// engine's node and edge labels, so its diagnostics come first.
#[test]
fn issue30_root_label_warnings_come_first() {
    let src = "digraph G { label=<<TABLE X=\"1\"><TR><TD>g</TD></TR></TABLE>>; \
a [label=<<TABLE Y=\"1\"><TR><TD>a</TD></TR></TABLE>>] }";
    let want = "Warning: Illegal attribute X in <TABLE> - ignored\n\
                in label of graph G\n\
                Warning: Illegal attribute Y in <TABLE> - ignored\n\
                in label of node a\n";
    for engine in ["dot", "neato", "patchwork"] {
        assert_eq!(warnings(engine, src).0, want, "{engine}");
    }
}

/// `addXLabels` (`lib/common/postproc.c:500-504`, `517`, `532`, `547`): an unset label of an
/// edge without a spline is reported with its `lp->text` (escapes substituted, an HTML label
/// without its delimiters), and the edge's remaining labels are skipped.
#[test]
fn issue30_unplaced_edge_label_warnings() {
    let src = "digraph G { splines=none; a [xlabel=q]; a -> b [label=\"x\\E\", taillabel=t, \
headlabel=h, xlabel=xx]; b -> c [headlabel=<<b>h2</b>>]; c -> d [taillabel=t3, xlabel=x3]; }";
    let want = "Warning: no position for edge with label xa->b\n\
                Warning: no position for edge with head label <b>h2</b>\n\
                Warning: no position for edge with tail label t3\n";
    for engine in ["dot", "neato", "fdp", "twopi", "osage"] {
        assert_eq!(warnings(engine, src).0, want, "{engine}");
    }
}

/// `spline_edges_` (`lib/neatogen/neatosplines.c:623`, `:625`) and `compoundEdges`
/// (`lib/fdpgen/clusteredges.c:238-246`) when node obstacles overlap.
#[test]
fn issue30_touching_nodes_warnings() {
    let pinned = "a [pos=\"0,0!\"]; b [pos=\"0.1,0!\"]; c [pos=\"2,0!\"]; a -- c;";
    assert_eq!(
        warnings("neato", &format!("graph G {{ splines=true; {pinned} }}")).0,
        "Warning: some nodes with margin (3.20,3.20) touch - falling back to straight line \
         edges\n"
    );
    assert_eq!(
        warnings("neato", &format!("graph G {{ splines=ortho; {pinned} }}")).0,
        "Warning: the bounding boxes of some nodes touch - falling back to straight line \
         edges\n"
    );
    let compound = "graph G { splines=compound; pack=1; a [pos=\"0,0!\"]; b [pos=\"2,0!\"]; \
d [pos=\"2.1,0!\"]; c [pos=\"4,0!\"]; a -- c; }";
    assert_eq!(
        warnings("fdp", compound).0,
        "Warning: compoundEdges: nodes touch - falling back to straight line edges\n\
         pack value 1 is smaller than esep (3.200,3.200)\n\
         Warning: some nodes with margin (3.20,3.20) touch - falling back to straight line \
         edges\n"
    );
}

/// Node shape diagnostics of `common_init_node`: `user_shape`'s unknown-shape warning, once
/// per name (`lib/common/shapes.c:3961`), `record_init`'s `bad label format` error, once
/// per node (`shapes.c:3682`), and `poly_init`'s fixed-size check (`shapes.c:2115-2118`).
#[test]
fn issue30_node_shape_warnings() {
    let src = "digraph Named { a [shape=foo]; a2 [shape=foo]; a3 [shape=Box]; \
b [shape=record, label=\"{a|\"]; b2 [shape=record, label=\"{a|\"]; \
c [fixedsize=true, width=0.1, label=\"long label\"]; c3 [fixedsize=true, width=3, label=ok] }";
    let want = "Warning: using box for unknown shape foo\n\
                Warning: using box for unknown shape Box\n\
                Error: bad label format {a|\n\
                Error: bad label format {a|\n\
                Warning: node 'c', graph 'Named' size too small for label\n";
    for engine in ["dot", "neato"] {
        let (err, ok) = warnings(engine, src);
        assert!(!ok, "{engine}: an error sets the exit status");
        assert_eq!(err, want, "{engine}");
    }
}

/// Each label is made, and reported, once in C, even when two labels of one object, or two
/// parallel edges, have the same text.
#[test]
fn issue30_identical_labels_each_warn() {
    let font = "<<FONT Q=\"1\">x</FONT>>";
    let src = format!(
        "digraph G {{ a [label={font} xlabel={font}]; \
         a -> b [label={font} headlabel={font} taillabel={font}] }}"
    );
    let node = "Warning: Illegal attribute Q in <FONT> - ignored\nin label of node a\n";
    let edge = "Warning: Illegal attribute Q in <FONT> - ignored\nin label of edge a -> b\n";
    let want = format!("{node}{node}{edge}{edge}{edge}");
    for engine in ["dot", "neato", "circo"] {
        assert_eq!(warnings(engine, &src).0, want, "{engine}");
    }
    let src = format!("digraph {{ a -> b [label={font}]; a -> b [label={font}] }}");
    assert_eq!(warnings("dot", &src).0, format!("{edge}{edge}"));
}

// C10 (c-numerics): C's numeric idioms as the reference build evaluates them
// (`graphviz_pure_core::cnum`). Expected outputs are Graphviz 16.1.0's.

fn c10_plain(engine: &str, src: &str) -> String {
    let (out, err, ok) = run(&[&format!("-K{engine}"), "-Tplain"], src);
    assert!(ok, "{engine}: {err}");
    out
}

/// #192: `-Tjson` prints the `_draw_` numbers with `%.03f`, which writes `nan`.
#[test]
fn json_draw_numbers_print_nan_like_printf() {
    let src = "digraph{layout=osage splines=curved{o2\n  subgraph cluster_inner4{j1 j2}}\
               subgraph cluster_outer{{label=\"outer\"subgraph cluster_inner4{j1 j2}}->o1}\
               subgraph cluster_inner4{j1 j2}}";
    let (out, _, ok) = run(&["-Tjson"], src);
    assert!(ok);
    assert!(
        out.contains("\"points\": [[nan,inf],[nan,nan],[nan,nan],[nan,inf]]"),
        "{out}"
    );
    assert!(out.contains("\"pt\": [nan,nan],"), "{out}");
    assert!(!out.contains("NaN"), "{out}");
}

/// #194: `job->width` is `ROUND` converted through 64 bits, keeping the low 32 bits, so an
/// infinite page width is -1.
#[test]
fn svg_infinite_page_width_wraps_like_c() {
    let src = "digraph{layout=osage{o2\n  subgraph cluster_inner4{j1 j2}}\
               subgraph cluster_outer{{label=\"outer\"subgraph cluster_inner4{j1 j2}};o1->o2}\
               subgraph cluster_inner4{j1 j2}}";
    let (out, _, ok) = run(&["-Tsvg"], src);
    assert!(ok);
    assert!(
        out.contains("<svg width=\"-1pt\" height=\"88pt\"\n viewBox=\"0.00 0.00 -1.00 88.00\""),
        "{out}"
    );
}

/// #216: fdp's `initPositions` builds the pins' box with C's `MIN`/`MAX`, which keep a NaN.
#[test]
fn fdp_pin_box_keeps_a_nan_like_c_min_max() {
    let out = c10_plain("fdp", "graph { a [pos=\"nan,nan!\"]; b [pos=\"1,1!\"] }");
    assert_eq!(
        out,
        "graph 1 -5.9652e+07 -5.9652e+07\n\
         node a nan nan 0.75 0.5 a solid ellipse black lightgrey\n\
         node b nan nan 0.75 0.5 b solid ellipse black lightgrey\n\
         stop\n"
    );
}

/// #272: integer attributes narrow to `int` as `atoi`, `sscanf("%d")`, `strtol` and `ROUND`
/// do in C. fdp with `maxiter=INT_MAX` hung: C's `int` pass-1 count wraps to 0.
#[test]
fn int_attributes_narrow_like_c() {
    assert_eq!(
        c10_plain("fdp", "graph{maxiter=\"2147483647\"; a--b}"),
        "graph 1 1.7751 0.65315\n\
         node a 0.375 0.40315 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 1.4001 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge a b 4 0.74507 0.34786 0.83912 0.33381 0.94012 0.31872 1.034 0.3047 solid black\n\
         stop\n"
    );
    let cases = [
        // MaxIter = atoi(str) = INT_MIN: no iterations
        (
            "neato",
            "graph{maxiter=\"2147483648\"; a--b; b--c; c--d; d--a; a--c}",
            "graph 1 0.75 0.5\nnode a 0.375 0.25 0.75 0.5 a",
        ),
        // late_int: strtol gives LONG_MIN, below the minimum 0
        (
            "fdp",
            "graph{subgraph cluster_x{margin=\"-99999999999999999999\"; a; b} a--c}",
            "graph 1 2.5276 1.2568\nnode a 0.375 0.25 0.75 0.5 a",
        ),
        // mapBool: atoi("4294967296") is 0, false
        (
            "neato",
            "graph{overlap=\"4294967296\"; a--b; a--c; a--d}",
            "graph 1 5.6728 5.7386\nnode a 2.2796 3.1024 0.75 0.5 a",
        ),
        // setSeed: sscanf("%ld") saturates at LONG_MAX
        (
            "neato",
            "graph{start=\"99999999999999999999\"; a--b; b--c}",
            "graph 1 1.7603 2.2053\nnode a 0.375 0.25 0.75 0.5 a",
        ),
        // getPack: sscanf("%d") gives INT_MIN, so no packing
        (
            "fdp",
            "graph{pack=\"2147483648\"; a; b}",
            "graph 1 1.6528 1.2639\n",
        ),
        // GD_nodesep = POINTS(1e300) saturates at INT_MAX
        (
            "circo",
            "graph{nodesep=\"1e300\"; a--b; a--b}",
            "graph 1 2.5 2.237e+07\nnode a 0.375 1.1185e+07 0.75 0.5 a",
        ),
    ];
    for (engine, src, want) in cases {
        let out = c10_plain(engine, src);
        assert!(out.starts_with(want), "{engine} {src}:\n{out}");
    }
}

/// #274: C interns a label's font in a dictionary whose comparison finds a NaN size equal
/// to any size, so a `fontsize=nan` label is measured at another font's size, and a NaN
/// font interned first is found for later sizes.
#[test]
fn nan_fontsize_takes_an_interned_fonts_size() {
    let out = c10_plain(
        "dot",
        "digraph{b [fontsize=30]; a [label=\"X\", fontsize=nan]}",
    );
    assert!(
        out.contains("node a 1.375 0.43212 0.75 0.86424 X "),
        "{out}"
    );
    let out = c10_plain(
        "dot",
        "digraph{b [fontsize=30]; c [fontsize=20]; a [label=\"X\", fontsize=nan]}",
    );
    assert!(
        out.contains("node a 2.375 0.43212 0.75 0.62854 X "),
        "{out}"
    );
    let out = c10_plain(
        "dot",
        "digraph{a [label=\"X\", fontsize=nan]; b [fontsize=30]}",
    );
    assert!(out.contains("node b 1.375 0.25 0.75 0.5 b "), "{out}");
    let (out, _, ok) = run(
        &["-Tdot"],
        "digraph{subgraph cluster_x{label=\"X\"; fontsize=nan; a}}",
    );
    assert!(ok);
    assert!(out.contains("bb=\"8,8,78,84.8\""), "{out}");
    assert!(out.contains("lheight=0.23"), "{out}");
    // The port measures some labels again (the root label, dot's node pre-pass, the
    // renderer); those lookups find the fonts C's single lookup found.
    let out = c10_plain(
        "dot",
        "digraph G0 { label=\"GL\"; fontsize=40; n1 [fontsize=nan, label=\"q\\nr\"]; n3 [label=\"X\"] }",
    );
    assert!(
        out.starts_with("graph 1 1.75 2.8205\nnode n1 0.375 1.7992 0.75 2.0428 "),
        "{out}"
    );
    let (out, _, ok) = run(
        &["-Tsvg"],
        "digraph { n0 [fontsize=30]; n1 [fontname=Courier, fontsize=40]; \
         subgraph cluster_3 { label=C3; fontsize=10; n3 [fontsize=nan, label=Q] } }",
    );
    assert!(ok);
    assert!(out.contains("font-size=\"30.00\">Q</text>"), "{out}");
    // Each object keeps the sizes its first measurement interned: dot's node pass after
    // the pre-pass, and the renderer, which measures edges in another order.
    let out = c10_plain(
        "dot",
        "digraph { a [fontsize=30]; b [fontsize=20]; c [fontsize=nan]; d [fontsize=30]; \
         e [fontsize=nan]; d -> e; e -> f }",
    );
    assert!(out.starts_with("graph 1 3.75 3.2285\n"), "{out}");
    assert!(out.contains("node e 3.375 1.4321 0.75 0.86424 e "), "{out}");
    let (out, _, ok) = run(
        &["-Tsvg"],
        "digraph { a; b; b -> x [fontsize=30, label=E1]; b -> y [fontsize=nan, label=E2]; \
         a -> z [fontsize=nan, label=E3] }",
    );
    assert!(ok);
    for (size, label) in [("14.00", "E3"), ("30.00", "E1"), ("30.00", "E2")] {
        assert!(
            out.contains(&format!("font-size=\"{size}\">{label}<")),
            "{label}: {out}"
        );
    }
    // dot's aux graph for flat adjacent edges measures its edge copies afresh.
    let out = c10_plain(
        "dot",
        "digraph G { c -> d [label=\"first\", fontsize=30]; {rank=same; a; b}\n\
         a:e -> b:w [label=\"WIDE LABEL HERE\", fontsize=40];\n\
         a:e -> b:w [label=\"small\", fontsize=8]; }",
    );
    assert!(out.starts_with("graph 1 7.7222 3.5503\n"), "{out}");
    assert!(out.contains(" small 4.3611 2.1452 "), "{out}");
    // An HTML label's text takes the object's fontcolor (lp->fontcolor).
    let (out, _, ok) = run(
        &["-Tsvg"],
        "digraph { a [fontcolor=blue, label=<x <B>y</B>>]; \
         a -> b [fontcolor=gray, label=<<I>e</I>>, headlabel=<h>, labelfontcolor=red] }",
    );
    assert!(ok);
    for c in ["blue", "gray", "red"] {
        assert!(out.contains(&format!("fill=\"{c}\"")), "{c}: {out}");
    }
    // A line's height uses the size size_html_txt looked up, NaN included; drawing falls
    // back to the environment's size for a font whose size is not positive.
    let src =
        "digraph { a [fontsize=14, label=<x<I>i</I>>]; n2 [fontsize=nan, label=<x<I>i</I>>] }";
    let (port, _, ok) = run(&["-Txdot"], src);
    assert!(ok);
    assert!(port.contains("T 93.56 28.8 -1 7 1 -x "), "{port}");
    let (out, _, ok) = run(
        &["-Tsvg"],
        "digraph { n0 [fontsize=nan, xlabel=<<TABLE><TR><TD><FONT FACE=\"Courier\">c</FONT></TD></TR></TABLE>>]; \
         n1 -> n3 [xlabel=<same>] }",
    );
    assert!(ok);
    assert!(out.contains("font-size=\"14.00\">same</text>"), "{out}");
    // -n labels take their fontcolor too.
    let (out, _, ok) = run(
        &["-Kneato", "-n", "-Txdot"],
        "digraph { a [pos=\"0,0\", fontsize=30, label=<<TABLE><TR><TD>abc</TD></TR></TABLE>>]; \
         b [pos=\"100,0\", fontcolor=red, fontsize=8, label=<<TABLE><TR><TD>abc</TD></TR></TABLE>>]; \
         subgraph cluster_q { bb=\"150,-20,300,80\"; fontcolor=red; fontsize=nan; \
         label=<<TABLE><TR><TD>cluster</TD></TR></TABLE>>; c [pos=\"200,0\"] } }",
    );
    assert!(ok);
    assert!(out.contains("lheight=0.30"), "{out}");
    // One dictionary for the run, as C has one per GVC.
    let out = c10_plain(
        "dot",
        "digraph { a -> b [fontsize=20, label=E] }\n\
         digraph { label=\"GL\"; fontsize=nan; n3 [fontname=Courier] }",
    );
    assert!(out.contains("stop\ngraph 1 0.75 0.94444\n"), "{out}");
    // HTML labels intern their fonts too (mkFont, pushFont, size_html_txt).
    let out = c10_plain(
        "neato",
        "digraph { label=\"G0\"; fontsize=14; n0 [fontsize=14]; n1 [fontsize=20]; \
         n2 [fontsize=14, label=<<B>h</B>>]; n3 [fontsize=nan]; \
         n1 -> n0 [fontsize=30, label=E]; n1 -> n2 [fontsize=10, label=E] }",
    );
    assert_eq!(
        out,
        "graph 1 2.5556 3.0111\n\
         node n0 0.375 2.3372 0.75 0.5 n0 solid ellipse black lightgrey\n\
         node n1 0.9864 1.541 0.75 0.62854 n1 solid ellipse black lightgrey\n\
         node n2 1.3212 0.59444 0.75 0.5 <<B>h</B>> solid ellipse black lightgrey\n\
         node n3 2.1806 2.7611 0.75 0.5 n3 solid ellipse black lightgrey\n\
         edge n1 n0 4 0.77941 1.8106 0.73615 1.8669 0.68993 1.9271 0.64478 1.9859 E 0.83935 2.1482 solid black\n\
         edge n1 n2 4 1.0942 1.2363 1.1217 1.1586 1.1516 1.0739 1.1805 0.99226 E 1.0949 1.031 solid black\n\
         stop\n"
    );
}

/// #292: `get_inputscale` tests `is_exactly_zero`, which is false for `-0`, so positions
/// are not scaled.
#[test]
fn inputscale_minus_zero_does_not_scale() {
    for engine in ["fdp", "neato"] {
        let (out, _, ok) = run(
            &[&format!("-K{engine}"), "-Tdot"],
            "graph { inputscale=\"-0\"; a [pos=\"100,100!\"]; b [pos=\"200,100!\"]; a -- b }",
        );
        assert!(ok);
        assert!(out.contains("bb=\"0,0,7254,36\""), "{engine}: {out}");
    }
    // `-s` sets `PSinputscale` the same way: `-s-0` keeps -0, which does not scale.
    let src = "graph { a [pos=\"100,100!\"]; b [pos=\"200,100!\"]; a -- b }";
    for (flag, bb) in [("-s-0", "0,0,7254,36"), ("-s0", "0,0,154,36")] {
        let (out, _, ok) = run(&["-Kneato", flag, "-Tdot"], src);
        assert!(ok);
        assert!(out.contains(&format!("bb=\"{bb}\"")), "{flag}: {out}");
    }
}

/// Every graph of a file is laid out, and its labels reported, on its own. Only the
/// unknown-shape warning is once per process (`user_shape` adds the name to `Shapes`).
#[test]
fn issue30_warnings_repeat_per_graph() {
    let g = "digraph G { a [label=<<TABLE Q=\"1\"><TR><TD>x</TD></TR></TABLE>> shape=foo]; \
a:zz -> b; c [shape=record label=\"{a|\"]; d [fixedsize=true width=0.1 label=\"long label\"] }\n";
    let per_graph = "Warning: Illegal attribute Q in <TABLE> - ignored\n\
                     in label of node a\n\
                     Error: bad label format {a|\n\
                     Warning: node 'd', graph 'G' size too small for label\n\
                     Warning: node a, port zz unrecognized\n";
    let want = format!("Warning: using box for unknown shape foo\n{per_graph}{per_graph}");
    for engine in ["dot", "neato"] {
        let (err, ok) = warnings(engine, &format!("{g}{g}"));
        assert!(!ok, "{engine}");
        assert_eq!(err, want, "{engine}");
    }
}

/// `neato -n`: neato_init_node_edge runs first whatever the level, so each edge's label
/// warnings are followed by its port warnings, and `-n2` warns about the ports of edges that
/// keep their input splines.
#[test]
fn issue30_neato_nop_port_warnings() {
    let src = "digraph G { node [pos=\"0,0\"]; a [pos=\"0,0\" label=<<FONT A=\"1\">a</FONT>>]; \
b [pos=\"100,0\"]; c [pos=\"10,0\"]; a:zz -> b [label=x pos=\"0,0 30,0 60,0 100,0\" lp=\"50,10\"]; \
b -> c:yy [label=<<FONT B=\"1\">y</FONT>>]; }";
    let want = "Warning: Illegal attribute A in <FONT> - ignored\n\
                in label of node a\n\
                Warning: node a, port zz unrecognized\n\
                Warning: Illegal attribute B in <FONT> - ignored\n\
                in label of edge b -> c\n\
                Warning: node c, port yy unrecognized\n";
    for n in ["-n1", "-n2"] {
        assert_eq!(run(&["-Kneato", n, "-Tsvg"], src).1, want, "{n}");
    }
    let src = "digraph G { a [pos=\"0,0\"]; b [pos=\"100,0\"]; \
a:zz -> b:yy [pos=\"e,80,0 0,0 30,0 60,0 80,0\"]; b -> a [label=x pos=\"e,0,0 100,0 60,0 30,0 0,0\"] }";
    let want = "Warning: node a, port zz unrecognized\nWarning: node b, port yy unrecognized\n";
    for n in ["-n1", "-n2"] {
        assert_eq!(run(&["-Kneato", n, "-Tsvg"], src).1, want, "{n}");
    }
}

/// `sidesfn` prints each unrecognized byte with `%c`, so a UTF-8 character gives one warning
/// per byte, written raw, with the byte's signed `char` value.
#[test]
fn issue30_sides_warning_raw_bytes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .args(["-Tsvg"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            "digraph { a [label=<<TABLE SIDES=\"é\"><TR><TD>x</TD></TR></TABLE>>] }".as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let mut want = b"Warning: Unrecognized character '\xC3' (-61) in sides attribute\n".to_vec();
    want.extend_from_slice(b"Warning: Unrecognized character '\xA9' (-87) in sides attribute\n");
    assert_eq!(out.stderr, want);
}

/// `lexerror` (`lib/common/htmllex.c:67-72`) does not stop expat, so an empty unknown
/// element is reported by both `startElement` and `endElement`; other forms once.
#[test]
fn issue30_unknown_empty_html_element_reported_twice() {
    let err = "Error: Unknown HTML element <FOO> on line 1 \n";
    for (label, n) in [
        (
            "<TABLE A=\"1\"><FOO Z=\"1\"/><TR><TD>x</TD></TR></TABLE>",
            2,
        ),
        ("<FOO/>", 2),
        ("<FOO>x</FOO>", 1),
    ] {
        let warn = if label.contains("A=") {
            "Warning: Illegal attribute A in <TABLE> - ignored\n"
        } else {
            ""
        };
        let want = format!("{warn}{}in label of node a\n", err.repeat(n));
        let (got, ok) = warnings("dot", &format!("digraph {{ a [label=<{label}>] }}"));
        assert!(!ok, "{label}");
        assert_eq!(got, want, "{label}");
    }
}

/// `make_flat_adj_edges` (`lib/dotgen/dotsplines.c:1144-1151`) skips flat edges between
/// adjacent nodes when one is a record, with a warning printed once per process.
#[test]
fn issue30_flat_adjacent_record_edge_warning() {
    let src = "graph { {rank=same; a -- b [label=x]; c -- d [label=y];} b [shape=record]; \
c [shape=Mrecord] }";
    let (err, ok) = warnings("dot", src);
    assert!(!ok);
    assert_eq!(
        err,
        "Warning: flat edge between adjacent nodes one of which has a record shape - replace \
         records with HTML-like labels\n  Edge a -- b\n\
         Error: lost a b edge\n\
         Error: lost c d edge\n"
    );
}

/// fdp's `deriveGraph` (`lib/fdpgen/layout.c:409-413`) labels a graph's clusters before it
/// lays out each of them, so a cluster's label warnings come after all its siblings'.
#[test]
fn issue30_fdp_cluster_label_warning_order() {
    let tbl = |a: &str| format!("<<TABLE {a}=\"1\"><TR><TD>x</TD></TR></TABLE>>");
    let src = format!(
        "graph G {{ subgraph cluster_a {{ label={}; subgraph cluster_a1 {{ label={}; p }} q }} \
         subgraph cluster_b {{ label={}; subgraph cluster_b1 {{ label={}; r }} s }} x -- s; }}",
        tbl("A1"),
        tbl("A11"),
        tbl("B1"),
        tbl("B11")
    );
    let w = |a: &str, g: &str| {
        format!("Warning: Illegal attribute {a} in <TABLE> - ignored\nin label of graph {g}\n")
    };
    let want = [
        w("A1", "cluster_a"),
        w("B1", "cluster_b"),
        w("A11", "cluster_a1"),
        w("B11", "cluster_b1"),
    ]
    .concat();
    assert_eq!(warnings("fdp", &src).0, want);
}

/// fdp's `splines and cluster edges not supported` (`lib/fdpgen/layout.c:1048-1051`) and
/// osage's array packing without `sortv` (`lib/osage/osageinit.c:100-106`, no newline).
#[test]
fn issue30_fdp_and_osage_layout_warnings() {
    assert_eq!(
        warnings(
            "fdp",
            "graph G { splines=true; subgraph cluster_a { x } y -- cluster_a }"
        )
        .0,
        "Warning: splines and cluster edges not supported - using line segments\n"
    );
    let src = "graph G { packmode=\"array_u\"; subgraph cluster_a { packmode=\"array_u\"; x; \
subgraph cluster_c { z } } subgraph cluster_b { y } }";
    let w = |g: &str| {
        format!(
            "Warning: Graph {g} has array packing with user values but no \"sortv\" attributes \
             are defined."
        )
    };
    let want = ["cluster_c", "cluster_a", "cluster_b", "G"].map(w).concat();
    assert_eq!(warnings("osage", src).0, want);
}

/// fdp's cluster edges (`checkCompound`, `lib/common/utils.c:855-900`): cycles and edges
/// within one cluster tree are dropped with a warning. `agcontains` looks at every
/// ancestor, so a cluster two levels down counts as inside.
#[test]
fn issue30_fdp_cluster_edge_warnings() {
    for (src, want) in [
        (
            "graph G { subgraph cluster_b { b1 } cluster_b -- cluster_b; x -- cluster_b }",
            "Warning: cluster cycle cluster_b -- cluster_b not supported\n",
        ),
        (
            "graph G { subgraph cluster_a { a1; subgraph cluster_a2 { a2 } } \
             cluster_a -- cluster_a2; cluster_a2 -- cluster_a; x -- cluster_a }",
            "Warning: head cluster cluster_a2 inside tail cluster cluster_a\n\
             Warning: tail cluster cluster_a2 inside head cluster cluster_a\n",
        ),
        (
            "graph G { subgraph cluster_a { a1 } a1 -- cluster_a; cluster_a -- a1; x -- cluster_a }",
            "Warning: tail node a1 inside head cluster cluster_a\n\
             Warning: head node a1 inside tail cluster cluster_a\n",
        ),
        (
            "graph G { subgraph cluster_a { subgraph cluster_b { subgraph cluster_c { c } } } \
             cluster_a -- cluster_c; x -- cluster_a }",
            "Warning: head cluster cluster_c inside tail cluster cluster_a\n",
        ),
    ] {
        assert_eq!(warnings("fdp", src).0, want, "{src}");
    }
}

// Issue #269: at 1e300 points, `update_bb_bz`'s pieces stop shrinking (one recurs two splits
// down) and `ptToLine2` overflows to NaN, so C recurses until its stack overflows (SIGSEGV).
// The port refines on an explicit stack and takes the pieces whole past C's stack depth.
#[test]
fn issue269_update_bb_bz_at_1e300_does_not_overflow_the_stack() {
    for (engine, src) in [
        ("dot", "digraph{ratio=\"1e300\"; f:n -> b}"),
        (
            "twopi",
            "digraph G { a -> b; a -> c; subgraph cluster_x { } c -> d; d -> e; { } \
             f:p -> b; ranksep=\"1e300\"; }",
        ),
    ] {
        let (out, err, ok) = run(&["-K", engine, "-Tplain"], src);
        assert!(ok, "{src}: {err}");
        assert!(out.ends_with("stop\n"), "{src}: {out}");
    }
}

// Issue #270: huge scales make some path boxes empty, `add_box` drops them, and
// `adjustregularpath`'s first loop runs past the boxes kept. C touches only stale slots there,
// so its output is the port's (Graphviz 16.1.0). Where every box is dropped, C segfaults in
// `Pshortestpath`; the port loses the edge.
#[test]
fn issue270_dropped_path_boxes_lay_out_like_c() {
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph{quantum=\"1e300\"; a -> b [label=\"ab\"]}",
    );
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "graph 1 1.4142e+300 2.8284e+300
node a 7.0711e+299 2.1213e+300 1.4142e+300 1.4142e+300 a solid ellipse black lightgrey
node b 7.0711e+299 7.0711e+299 1.4142e+300 1.4142e+300 b solid ellipse black lightgrey
edge a b 7 3.9088e+299 1.4889e+300 2.0258e+299 1.1123e+300 0 7.0711e+299 0 7.0711e+299 0 \
7.0711e+299 0 7.0711e+299 1.3218e+284 7.0711e+299 ab 7.0711e+299 1.4142e+300 solid black
stop
"
    );
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph{ratio=\"9223372036854775808\"; a -> b [label=\"ab\"]; b -> c; a -> c}",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains(
            "edge a b 4 0.93056 1.0946e+19 0.93056 1.0946e+19 0.375 5.5399e+18 \
                      0.375 5.5399e+18 ab 1.0223 8.2429e+18 solid black\n"
        ),
        "{out}"
    );
    let (_, err, ok) = run(
        &["-Tplain"],
        "digraph{ratio=\"inf\"; a -> c; b -> c; {rank=same; f [shape=record]; a} f:p -> b}",
    );
    assert!(!ok);
    assert!(
        err.contains("Error: lost a c edge\n") && !err.contains("panicked"),
        "{err}"
    );
}

// Issue #301. An infinite (or 400-digit) edge label font size gives the label an infinite
// size. C's `xlhdxload` (lib/label/xlabels.c:513) asserts that the label area's centre fits
// an `int` and aborts; past that point, `adjust`'s slides stay at `-inf` forever. The
// port takes `xlhdxload`'s own failure path there: no exterior label is placed.
#[test]
fn infinite_edge_label_size_does_not_hang_xlabel_placement() {
    let huge = format!("1{}", "0".repeat(400));
    for (engine, src) in [
        (
            "dot",
            "digraph G { edge [labelfontsize=inf]; a -> b [headlabel=h]; }".to_string(),
        ),
        (
            "neato",
            "graph G { edge [labelfontsize=inf]; a -- b [headlabel=h]; }".to_string(),
        ),
        (
            "dot",
            format!("digraph G {{ edge [labelfontsize={huge}]; a -> b [headlabel=h]; }}"),
        ),
        // dot places the edge label itself; only the head label goes to xlabels.
        (
            "dot",
            "digraph G { edge [fontsize=inf]; a -> b [label=ab, headlabel=h]; }".to_string(),
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
            .args(["-Tplain", &format!("-K{engine}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(src.as_bytes())
            .unwrap();
        // The output is a few lines, well under the pipe buffer, so polling cannot block.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                panic!("xlabel placement must terminate: {src}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let out = child.wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{src}: {err}");
        assert!(
            err.contains("0 out of 1 exterior labels positioned"),
            "{src}: {err}"
        );
    }
}

// Issue #273. sfdp's `rotate` (lib/sfdpgen/spring_electrical.c:935) asserts `dim == 2`, so
// C aborts on `rotation` with `dim` or `dimen` above 2; the port indexed its 2-element centre
// out of bounds. It now lays out as without `rotation`, with a warning.
#[test]
fn sfdp_rotation_in_3d_is_skipped_with_a_warning() {
    for dims in ["dim=3", "dimen=3", "dimen=2147483647"] {
        let src = |rot: &str| format!("graph {{ {dims}; {rot} a -- b; b -- c; c -- a }}");
        let (out, err, ok) = run(&["-Ksfdp", "-Tplain"], &src("rotation=10;"));
        assert!(ok, "{dims}: {err}");
        assert!(
            err.contains("rotation needs a 2-dimensional layout"),
            "{dims}: {err}"
        );
        let (unrotated, err, ok) = run(&["-Ksfdp", "-Tplain"], &src(""));
        assert!(ok, "{dims}: {err}");
        assert_eq!(out, unrotated, "{dims}");
    }
}

/// Runs dot on `src` and returns (stdout, stderr, success), failing if it takes more than
/// 60 s: the inputs of #358 used to hang.
fn run_dot_with_deadline(name: &str, args: &[&str], src: &str) -> (String, String, bool) {
    let path = tmp(name);
    std::fs::write(&path, src).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphviz-pure"))
        .args(args)
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(60) {
            child.kill().unwrap();
            panic!("dot did not finish in 60 s on {src}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// #357: `nslimit1` too small for one iteration stops network simplex before it normalises
/// the ranks. Where the root's ranks then start below 0, C writes before its rank arrays
/// (`allocate_ranks`), reads a NULL label (`label_vnode`) or reads before a cluster's rank
/// leaders (`build_skeleton`), and crashes. The port fails the layout before mincross.
#[test]
fn nslimit1_ranks_below_zero_fail_the_layout() {
    for src in [
        "digraph { nslimit1=0; {rank=sink; a} a -> d [minlen=3]; b -> c }\n",
        "digraph { nslimit1=0; subgraph r4 { rank=sink; n10; } \
         n1 -> n4 [label=\"L\", headlabel=\"l\"]; n10 -> n9 [minlen=3]; }\n",
        "digraph G { newrank=true; nslimit1=0.01; subgraph cluster_0 { n5 -> n0; } \
         n3 -> n0 [minlen=0, weight=0, lhead=cluster_1]; n3 -> n1 [ltail=cluster_1]; }\n",
    ] {
        let (_, err, ok) = run(&["-Tsvg"], src);
        assert!(!ok, "{src}");
        assert!(!err.contains("panicked"), "{err}");
        assert!(
            err.contains("stopped before normalising its ranks"),
            "{err}"
        );
    }
    // Ranks that start above 0 are kept, as C keeps them: C draws this one (fuzz-gen-ranks
    // seed 6807), and the port draws it the same way.
    let (out, err, ok) = run(
        &["-Tsvg"],
        "digraph G { compound=true; pack=true; nslimit1=0.01; \
         subgraph cluster_0 { n1; n0; subgraph cluster_1 { label=\"label\"; n1; n0; } } \
         subgraph { rank=source; n0; n1; } \
         n0 -> n1 [label=\"\", minlen=0, constraint=false]; \
         n0 -> n1 [label=\"l\", lhead=cluster_0]; \
         n1 -> n0 [label=\"a long edge label\", lhead=cluster_0]; n0 -> n1; \
         n0 -> n1 [label=\"l\", constraint=false, dir=none]; }\n",
    );
    assert!(ok, "{err}");
    assert!(out.contains("</svg>"), "{out}");
}

/// #358 (and #357's fourth input): a root rank with no node reaches `set_ycoords`, where C
/// dereferences its NULL first node. The port fails the layout there.
#[test]
fn empty_root_rank_in_set_ycoords_fails_the_layout() {
    for src in [
        "digraph G { subgraph cluster_0 { subgraph { rank=sink; n4; } n1 -> n3 [minlen=2]; } \
         subgraph r17 { rank=source; n0; n1; } n4 -> n4; }\n",
        "digraph G { nslimit1=0; subgraph cluster_0 { n1 -> n3 [label=\"L\", minlen=3]; } \
         subgraph { rank=same; n1; n2; n3; } n1 -> n2; n3 -> n2; z2a -> z2b; }\n",
    ] {
        let (_, err, ok) = run(&["-Tsvg"], src);
        assert!(!ok, "{src}");
        assert!(!err.contains("panicked"), "{err}");
        assert!(err.contains("set_ycoords: rank"), "{err}");
    }
}

/// #358: a freed rank leader's stale order made `furthestnode` walk in a cycle. C's
/// `neighbor` asserts that the order moves the way the walk goes; the port fails there.
#[test]
fn furthestnode_on_a_freed_leader_does_not_hang() {
    let src = "digraph G {\nsubgraph cluster_0 { n6; n9; n3 -> n9; }\n\
               subgraph cluster_1 { subgraph cluster_2 { n5; n1; } }\nsubgraph cluster_3 { }\n\
               subgraph r32 { rank=sink; n5; n0; }\nsubgraph r38 { rank=min; n9; }\n\
               subgraph { rank=same; n3; }\nn3 -> n0 [label=\"\"];\nn5 -> n0;\n\
               n1 -> n5 [label=\"\"];\n}\n";
    let (_, err, ok) = run_dot_with_deadline("issue358-furthestnode.gv", &["-Tsvg"], src);
    assert!(!ok);
    assert!(err.contains("neighbor: on rank"), "{err}");
}

/// #358: a cycle of constraining flat edges left on a rank leaves `flat_reorder`'s
/// topological order short. C's `LIST_GET` asserts; the port used to stop copying, leave the
/// rank half rewritten, and hang in `transpose`.
#[test]
fn flat_reorder_with_a_short_order_does_not_hang() {
    let src = "strict digraph G {\nordering=in;\nsubgraph cluster_0 {\n  n6; n0;\n\
               subgraph r7 { rank=sink; n5; n2; n4; }\n  subgraph cluster_1 { }\n\
               subgraph cluster_2 { n0; }\n}\nsubgraph { rank=min; n4; n1; n2; n0; }\n\
               n0 -> n2 [label=\"x\\ny\"];\nn2 -> n1 [label=\"L\", lhead=cluster_0];\n\
               n1 -> n2 [label=\"L\", constraint=false, headlabel=\"label\"];\n\
               n2 -> n0 [label=\"l\"];\nn6 -> n0 [weight=0, samehead=h0];\nn6 -> n3;\n\
               n1 -> n0 [dir=back, ltail=cluster_0];\nn1 -> n6 [label=\"\"];\n}\n";
    let (_, err, ok) = run_dot_with_deadline("issue358-flat-reorder.gv", &["-Tsvg"], src);
    assert!(!ok);
    assert!(err.contains("flat_reorder: rank"), "{err}");
}

/// `flat_reorder` reverses a non-constraining flat edge that points against the new order;
/// C asserts that it is not constraining (`mincross.c:1387`). Newrank with `ordering=in`
/// reaches it with no membership change (fuzz-gen-endpoints seed 42519): C aborts, the port
/// fails the layout there.
#[test]
fn flat_reorder_reversing_a_constraining_edge_fails_the_layout() {
    let src = "digraph G { ranksep=0.02; newrank=true; ordering=in;\n\
               n0 [shape=square, width=0, height=0, fixedsize=shape];\n\
               n1 [shape=underline, label=\"x\", width=0.3, height=0.001];\n\
               n2 [shape=Mrecord, label=\"{<p0>  |<p1>  |<p2>  }\", peripheries=0];\n\
               subgraph cluster_0 { n2; n0; n1; }\n\
               n1:w -> n0:e [headclip=true];\n\
               n1:e -> n1:n [headclip=false, tailclip=true, dir=none, arrowhead=dot, \
               headport=ne, tailport=se];\n\
               n1:nw -> n2:p0 [label=\"\", headlabel=\"h\", arrowhead=normal, tailport=n];\n\
               n1:e -> n2 [headclip=true, tailclip=true];\nn0:w -> n2:nw;\n\
               n2:p0 -> n1 [headclip=false, samehead=h0, ltail=cluster_0];\n\
               n0:c -> n2 [tailclip=false, headport=e];\n\
               n1 -> n2:nw [samehead=h1, sametail=t0, headlabel=\"\", ltail=cluster_0, \
               arrowsize=3];\n\
               n2:p1:n -> n1:_ [samehead=h1, sametail=t0, lhead=cluster_0];\n}\n";
    let (_, err, ok) = run(&["-Tsvg"], src);
    assert!(!ok);
    assert!(!err.contains("panicked"), "{err}");
    assert!(
        err.contains("flat_reorder: a constraining flat edge"),
        "{err}"
    );
}

/// #359: when every point tests inside the end nodes, `clip_and_install`'s walks cross.
/// C's `size_t` size wraps and it asserts in `bezier_bb`; the port keeps the one segment
/// where the walks would meet, so every reader gets a bezier with points.
#[test]
fn clip_and_install_crossed_walks_keep_one_segment() {
    let (out, err, ok) = run(
        &["-Tdot"],
        "digraph { splines=ortho; a [shape=plain, fixedsize=true]; a -> a }\n",
    );
    assert!(ok, "{err}");
    let pos = out
        .lines()
        .find_map(|l| l.split("pos=\"e,").nth(1))
        .unwrap_or_else(|| panic!("{out}"));
    // The arrow's end point, then four control points.
    assert_eq!(
        pos.trim_end_matches(['"', ']', ';']).split(' ').count(),
        5,
        "{out}"
    );
    // One segment apart, C installs a one-point bezier and draws it; that is kept.
    let (out, err, ok) = run(
        &["-Tdot"],
        "digraph { a [shape=plain, fixedsize=true]; b [shape=plain, fixedsize=true]; \
         {rank=same; a b} a -> a }\n",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("pos=\"e,0.28551,0.092285 0.14171,0.046509\""),
        "{out}"
    );
    for (args, src) in [
        (
            ["-Tsvg"],
            "digraph { splines=ortho; a [shape=plain, fixedsize=true]; a -> a [label=x] }\n",
        ),
        (
            ["-Tsvg"],
            "graph G { splines=polyline; nodesep=0.02; n4 [shape=plain, label=\"\"]; \
             subgraph { rank=same; n3; n4; } n4 -- n3:_ [headlabel=\"h\"]; }\n",
        ),
        (
            ["-Tsvg"],
            "graph { layout=neato; splines=ortho; a [shape=plain, fixedsize=true, width=0, \
             height=0, label=\"\"]; b [shape=plain, label=\"\"]; a -- a; a -- b [label=x]; \
             b -- b [headlabel=h]; }\n",
        ),
    ] {
        let (_, err, ok) = run(&args, src);
        assert!(ok && !err.contains("panicked"), "{src}: {err}");
    }
}

// Issues #267, #344, #346. cgraph's `agnameof` (lib/cgraph/id.c) names an anonymous graph or
// subgraph, or one with a local `%` name, by its id: the root is `%1`. The port printed the
// stored name, empty for an anonymous object. Expected values are C 16.1.0's.
#[test]
fn anonymous_graphs_are_named_by_their_id() {
    // #267: the charset warnings name the root.
    for (i, head) in ["digraph", "digraph \"%g\""].iter().enumerate() {
        let path = tmp(&format!("agnameof_utf8_{i}.gv"));
        let mut src = format!("{head} {{ a [label=\"").into_bytes();
        src.extend_from_slice(b"\xc3x\"] }");
        std::fs::write(&path, src).unwrap();
        let (_, err, ok) = run(&["-Tsvg", path.to_str().unwrap()], "");
        assert!(ok, "{err}");
        assert!(
            err.starts_with("Warning: Invalid 2-byte UTF8 found in input of graph %1 - "),
            "{head}: {err}"
        );
    }
    // #267: `\G` is the root's agnameof.
    let (out, err, ok) = run(&["-Tplain"], r#"digraph { a [label="\G"] }"#);
    assert!(ok, "{err}");
    assert!(
        out.contains(r#"node a 0.375 0.25 0.75 0.5 "%1" solid"#),
        "{out}"
    );
    // #346: an anonymous cluster's SVG title.
    let (out, err, ok) = run(
        &["-Tsvg"],
        "digraph{subgraph {cluster=true; a} subgraph {cluster=true; b}}",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("<title>%3</title>") && out.contains("<title>%5</title>"),
        "{out}"
    );
    // #344: fdp's non-comparable clusters error.
    let (_, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        "graph { subgraph cluster_x { a } subgraph { cluster=true; a } }",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node \"a\" is contained in two non-comparable clusters \"%3\" and \"cluster_x\"\n"
    );
    // fdp's coords warning names the cluster.
    let (_, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        r#"graph { subgraph { cluster=true; coords="1,2"; a } }"#,
    );
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: graph %3, coord 1,2, expected four doubles\n");
    // fdp's cluster-edge graph keeps the names: an empty-named root, an anonymous cluster.
    let (out, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        r#"graph "" { label="\G."; subgraph cluster_x { b } a -- cluster_x }"#,
    );
    assert!(ok, "{err}");
    assert!(out.contains(">.</text>"), "{out}");
    let (out, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        r#"graph { subgraph cluster_x { b } subgraph { cluster=true; label="\G"; c } a -- cluster_x }"#,
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("<title>%3</title>") && out.contains(">%3</text>"),
        "{out}"
    );
    // ccomps' projectG finds a component's nodes by `agfindnode(g, agnameof(n))`, which
    // misses an anonymous node: the cluster is not projected into the component.
    let (out, err, ok) = run(
        &["-Tplain"],
        r#"graph { pack=true; a; subgraph { cluster=true; "%x" -- y } }"#,
    );
    assert!(ok, "{err}");
    assert!(out.starts_with("graph 1 2.25 1.6111\n"), "{out}");
    // make_label names a bad HTML label's owner by its kind: a node named "" is a node.
    let (_, err, _) = run(&["-Tsvg"], r#"digraph { "" [label=<<b>x</c>>] }"#);
    assert!(err.ends_with("in label of node \n"), "{err}");
    // A name read from an attribute after parsing is not found when it is a local `%` name:
    // twopi's `root` and the `viewport` focus node.
    let (out, err, ok) = run(
        &["-Ktwopi", "-Tplain"],
        r#"graph { root="%3"; "%x" -- b; b -- c; c -- d }"#,
    );
    assert!(ok, "{err}");
    assert_eq!(
        err,
        "Warning: specified root node \"%3\" was not found.Using default calculation for root node\n"
    );
    assert!(out.starts_with("graph 1 0.75 3.5\n"), "{out}");
    let (out, err, ok) = run(
        &["-Tsvg"],
        r#"graph { viewport="100,100,1,%3"; "%x" -- b; b -- c }"#,
    );
    assert!(ok, "{err}");
    assert!(out.contains("translate(23 140)"), "{out}");
}

// Issues #325, #344, #354. `agnameof` formats an anonymous object's name into one static
// buffer (lib/cgraph/id.c:162), so when one C call takes two such names, both show the one
// formatted last. Expected values are C 16.1.0's.
#[test]
fn anonymous_names_in_one_call_share_agnameofs_buffer() {
    // #325: the edge title of `%3 -> %7` (strdup_and_subst_obj's `\E`).
    let (out, err, ok) = run(&["-Tsvg"], r#"digraph { "%l" -> b; "%l" -> "%3" }"#);
    assert!(ok, "{err}");
    assert!(out.contains("<title>%3&#45;&gt;b</title>"), "{out}");
    assert!(out.contains("<title>%7&#45;&gt;%7</title>"), "{out}");
    // `\G \N` of an anonymous node in an anonymous root.
    let (out, err, ok) = run(
        &["-Tplain"],
        r#"digraph { "%x" [label="\G \N"]; b [label="\G \N"] }"#,
    );
    assert!(ok, "{err}");
    assert!(
        out.contains(r#"node "%3" "#) && out.contains(r#" "%3 %3" solid"#),
        "{out}"
    );
    assert!(out.contains(r#" "%1 b" solid"#), "{out}");
    // #344: two anonymous clusters.
    let (_, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        "graph { subgraph { cluster=true; a } subgraph { cluster=true; a b } }",
    );
    assert!(!ok);
    assert!(
        err.ends_with(
            "Error: node \"a\" is contained in two non-comparable clusters \"%3\" and \"%3\"\n"
        ),
        "{err}"
    );
    // dot's flat edge between records (dotsplines.c:1147).
    let (_, err, _) = run(
        &["-Tsvg"],
        r#"digraph { node [shape=record]; {rank=same; "%a" -> "%b"} }"#,
    );
    assert!(
        err.contains("\n  Edge %7 -> %7\nError: lost %7 %7 edge\n"),
        "{err}"
    );
    // #354: install_in_rank names the graph and a virtual node (`%0`).
    let (_, err, ok) = run(
        &["-Tcanon"],
        r#"digraph G {
pack=true; compound=true;
subgraph { cluster=true;
node[color=red]; edge[color=blue];
n2; n1; n0;
subgraph cluster_37 { n3; n4; n0; }
}
{ rank=min; n0; n3; n4; }
subgraph s27 { n4; n3; n0; n2; }
n3 -> n1; n0 -> n4; n3 -> n0 [ltail="%3"]; n1 -> n3; n2 -> n0 [ltail="%3"]; n3 -> n0; n4 -> n2;
}"#,
    );
    assert!(!ok);
    assert!(
        err.ends_with("Error: install_in_rank, line 1158: %0 %0 rank 1 i = 0 an = 0\n"),
        "{err}"
    );
}

// Issue #151: a real U+10FF00..=U+10FFFF char in UTF-8 input is not one of the port's raw
// bytes. C keeps its four bytes: it writes them, measures each one (warning about byte 244)
// and parses records and HTML labels on them. stdout and stderr are C 16.1.0's.
#[test]
fn issue151_real_chars_of_the_raw_byte_range_keep_their_bytes() {
    let (out, err, ok) = run(&["-Tplain"], "digraph G { \"x\u{10ff41}y\":qq -> b }");
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Warning: no value for width of non-ASCII character 244. Falling back to \
         width of space character\nWarning: node x\u{10ff41}y, port qq unrecognized\n"
    );
    assert_eq!(
        out,
        "graph 1 0.84307 1.5\n\
         node x\u{10ff41}y 0.42153 1.25 0.84307 0.5 x\u{10ff41}y solid ellipse black lightgrey\n\
         node b 0.42153 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge x\u{10ff41}y b 4 0.42153 0.99579 0.42153 0.89454 0.42153 0.77398 0.42153 0.66022 \
         solid black\nstop\n"
    );

    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { a [shape=record, label=\"<p\u{10ff41}>f\u{10ff41}|g\"]; b; a:\"p\u{10ff41}\" -> b }",
    );
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Warning: no value for width of non-ASCII character 143. Falling back to \
         width of space character\nWarning: node a, port p\u{10ff41} unrecognized\n"
    );
    assert_eq!(
        out,
        "graph 1 0.75 1.5139\n\
         node a 0.375 1.2569 0.75 0.51389 \"<p\u{10ff41}>f\u{10ff41}|g\" solid record black \
         lightgrey\n\
         node b 0.375 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
         edge a b 4 0.375 1.0012 0.375 0.89773 0.375 0.77584 0.375 0.66129 solid black\n\
         stop\n"
    );

    // HTML text: expat's character data is measured on its bytes too.
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { b [label=<<table><tr><td>\u{10ffff}</td></tr></table>>] }",
    );
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Warning: no value for width of non-ASCII character 244. Falling back to \
         width of space character\n"
    );
    assert_eq!(
        out,
        "graph 1 0.82496 0.72282\n\
         node b 0.41248 0.36141 0.82496 0.72282 <<table><tr><td>\u{10ffff}</td></tr></table>> \
         solid ellipse black lightgrey\n\
         stop\n"
    );

    // expat gives the char back as its bytes: `sides` warns about each one.
    let (out, err, ok) = run(
        &["-Tplain"],
        "digraph { a [label=<<table sides=\"l\u{10ff41}\"><tr><td title=\"\u{10ff41}\">x</td></tr></table>>] }",
    );
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Unrecognized character '\u{fffd}' (-12) in sides attribute\n\
         Warning: Unrecognized character '\u{fffd}' (-113) in sides attribute\n\
         Warning: Unrecognized character '\u{fffd}' (-67) in sides attribute\n\
         Warning: Unrecognized character '\u{fffd}' (-127) in sides attribute\n"
    );
    assert_eq!(
        out,
        "graph 1 0.75 0.72282\n\
         node a 0.375 0.36141 0.75 0.72282 \
         <<table sides=\"l\u{10ff41}\"><tr><td title=\"\u{10ff41}\">x</td></tr></table>> \
         solid ellipse black lightgrey\n\
         stop\n"
    );
}

// Issue #291: an attribute declared only by a subgraph's defaults is declared for the whole
// graph (cgraph puts it in the root's dictionary), so `agfindnodeattr`/`agfindedgeattr` find
// it: neato's `addZ` writes `z`, dot reads `labelangle`, patchwork sizes the empty `fontsize`.
// Values are C 16.1.0's.
#[test]
fn issue291_attributes_declared_by_a_subgraph_are_declared_for_the_graph() {
    let src = "graph { layout=neato; dim=3; subgraph s { node [z=1] } \
               a [pos=\"1,2\"]; b [pos=\"3,4\"]; a -- b }";
    let (out, _, ok) = run(&["-Tdot"], src);
    assert!(ok);
    assert!(
        out.contains("z=-10.783886]") && out.contains("z=10.783886]"),
        "{out}"
    );
    let (out, _, ok) = run(&["-n", "-Tdot"], src);
    assert!(ok);
    assert!(
        out.contains("z=57.090927]") && out.contains("z=121.029893]"),
        "{out}"
    );
    // A nested subgraph declares on the root too, and pack's components see the declaration.
    let (out, _, ok) = run(
        &["-Tdot"],
        "graph { layout=neato; dim=3; subgraph s { subgraph t { node [z=1] } } \
         a [pos=\"1,2\"]; b [pos=\"3,4\"]; a -- b }",
    );
    assert!(ok);
    assert!(
        out.contains("z=-10.783886]") && out.contains("z=10.783886]"),
        "{out}"
    );
    let (out, _, ok) = run(
        &["-Tdot"],
        "graph { layout=neato; dim=3; pack=true; subgraph s { node [z=1] } \
         a [pos=\"1,2\"]; b [pos=\"3,4\"]; a -- b; c -- d }",
    );
    assert!(ok);
    for z in ["-19.144376", "19.144376", "33.678597", "-33.678597"] {
        assert!(out.contains(&format!("z={z}]")), "{out}");
    }

    let (out, _, ok) = run(
        &["-Tplain"],
        "digraph { subgraph s { edge [labelangle=60] } \
         a -> b [headlabel=\"H\", taillabel=\"T\"]; b -> c [headlabel=\"X\"] }",
    );
    assert!(ok);
    assert_eq!(
        out,
        "graph 1 0.75 2.6167\n\
         node a 0.375 2.3667 0.75 0.5 a solid ellipse black lightgrey\n\
         node b 0.375 1.3667 0.75 0.5 b solid ellipse black lightgrey\n\
         node c 0.375 0.36667 0.75 0.5 c solid ellipse black lightgrey\n\
         edge a b 4 0.375 2.1125 0.375 2.0112 0.375 1.8906 0.375 1.7769 solid black\n\
         edge b c 4 0.375 1.1125 0.375 1.0112 0.375 0.89065 0.375 0.77688 solid black\n\
         stop\n"
    );

    let (out, _, ok) = run(
        &["-Tplain"],
        "graph { layout=patchwork; subgraph s { node [fontsize=\"\"] } a [area=2]; b; c [area=3] }",
    );
    assert!(ok);
    assert_eq!(
        out,
        "graph 1 1.0758 1.0758\n\
         node a 0.35861 0.26896 0.75 0.56296 a solid box black lightgrey\n\
         node b 0.89653 0.26896 0.75 0.56296 b solid box black lightgrey\n\
         node c 0.53792 0.80688 0.75 0.56296 c solid box black lightgrey\n\
         stop\n"
    );
}

// Issue #290: C's `drand48` state is process-wide, so the second graph of a run draws its
// unseeded `user_pos` jitter (and `-n`'s, which never seeds) from where the first left it.
// Values are C 16.1.0's.
#[test]
fn issue290_later_graphs_continue_the_runs_random_state() {
    let (out, _, ok) = run(
        &["-Kneato", "-Tdot"],
        "graph { a -- b }\ngraph { dim=3; a [pos=\"1,2\" z=\"5\"]; b [pos=\"3,4\"]; a -- b }",
    );
    assert!(ok);
    let second = out.split("graph {\n").nth(2).unwrap_or_default();
    assert!(second.contains("bb=\"0,0,84.046,66.046\""), "{out}");
    assert!(
        second.contains("z=29.062271") && second.contains("z=-29.062271"),
        "{out}"
    );

    let g = "graph { dim=3; a [pos=\"10,20,5\" z=\"3\"]; b [pos=\"30,40\"]; a -- b }\n";
    let (out, _, ok) = run(&["-Kneato", "-n", "-Tdot"], &g.repeat(2));
    assert!(ok);
    let zs: Vec<&str> = out
        .split("z=")
        .skip(1)
        .map(|s| {
            let end = s
                .find([',', ']'])
                .unwrap_or_else(|| panic!("unterminated z in {out}"));
            &s[..end]
        })
        .collect();
    assert_eq!(
        zs,
        ["5.000000", "57.090927", "5.000000", "121.029893"],
        "{out}"
    );
}

// Issue #347: a file C can load is sized from its header (`gvusershape_size` and the
// `*_size` readers in `lib/gvc/gvusershape.c`), and drawn by the loadimage plugin of the
// output's renderer (`gvrender_usershape`, `gvloadimage`); C warns where there is none and
// draws with the plugin it selected last. Expected output is C 16.1.0's.

/// A scratch directory with a 40x20 PNG header, an EPS file whose `%%BoundingBox` is
/// 100x50 and an SVG 2in wide whose `viewBox` gives its height, and a graph at 150 dpi
/// that uses them through `image`, `<IMG>` and `shape=epsf`.
fn image_files(name: &str) -> (PathBuf, &'static str) {
    let dir = usershape_dir(name);
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend(40u32.to_be_bytes());
    png.extend(20u32.to_be_bytes());
    png.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(dir.join("a.png"), png).unwrap();
    std::fs::write(
        dir.join("a.svg"),
        "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2in\" \
         viewBox=\"0 0 10 30\"/>\n",
    )
    .unwrap();
    let src = "digraph { dpi=150; a [image=\"a.png\"]; b [image=\"box.eps\" imagescale=true]; \
               c [label=<<TABLE><TR><TD><IMG SRC=\"a.svg\"/></TD></TR></TABLE>>]; \
               d [shape=epsf shapefile=\"box.eps\"]; a -> b -> c -> d }";
    (dir, src)
}

#[test]
fn loadable_images_are_sized_and_drawn_like_c() {
    let (dir, src) = image_files("i347-svg");
    // svg has no loadimage plugin for PostScript: C warns and draws it with the png's
    let (svg, err, code) = run_in(&dir, &["-Tsvg"], src);
    assert_eq!(code, 0);
    assert_eq!(err, "Warning: No loadimage plugin for \"ps:svg\"\n");
    for image in [
        "<image xlink:href=\"a.png\" width=\"19px\" height=\"9px\" \
         preserveAspectRatio=\"xMinYMin meet\" x=\"62.6249\" y=\"-302.122\"/>\n",
        "<image xlink:href=\"box.eps\" width=\"144.25px\" height=\"72.1249px\" \
         preserveAspectRatio=\"xMinYMin meet\" x=\"0\" y=\"-242.915\"/>\n",
        "<polygon fill=\"none\" stroke=\"black\" points=\"34.62,-100.04 34.62,-120.04 \
         109.62,-120.04 109.62,-100.04 34.62,-100.04\"/>\n<image xlink:href=\"a.svg\" \
         width=\"69px\" height=\"14px\" preserveAspectRatio=\"xMinYMin meet\" x=\"37.6249\" \
         y=\"-117.042\"/>\n</g>\n",
    ] {
        assert!(svg.contains(image), "{image}\n{svg}");
    }
    // the sizes, at 150 dpi (PostScript is at 72), plus 2
    let (xdot, err, _) = run_in(&dir, &["-Txdot"], src);
    assert_eq!(err, "");
    for op in [
        "a\t[_draw_=\"c 7 -#000000 e 72.12 297.62 27 18 I 62.62 293.12 19 9 5 -a.png \",",
        "e 72.12 206.85 72.12 36.77 I 0 170.79 144.25 72.12 7 -box.eps \",",
        "109.62 120.04 109.62 100.04 I 37.62 103.04 69 14 5 -a.svg \",",
        "height=1.0214,",
        "height=0.66782,",
        "width=1.9053];",
    ] {
        assert!(xdot.contains(op), "{op}\n{xdot}");
    }
    // json's own job has no loadimage plugin at all; its xdot job draws the images, and
    // the json writer prints an image operation as an empty object
    let (json, err, _) = run_in(&dir, &["-Tjson"], src);
    assert_eq!(
        err,
        "Warning: No loadimage plugin for \"png:json\"\n\
         Warning: No loadimage plugin for \"ps:json\"\n\
         Warning: No loadimage plugin for \"svg:json\"\n"
    );
    assert!(
        json.contains(
            "\"rect\": [72.120,297.620,27.000,18.000]\n        },\n        {\n                  }\n"
        ),
        "{json}"
    );
}

// Issue #347: `epsf_gencode` writes its PostScript call straight into the output file
// (`fprintf(job->output_file, ...)`), whatever the format: inside the node's group in
// -Tsvg, before the graph in the formats written at the end, and for -Tjson also from
// the xdot job `json_begin_graph` runs, whose output is the standard output.
#[test]
fn epsf_writes_its_postscript_call_into_every_output_like_c() {
    let (dir, src) = image_files("i347-epsf");
    let line = "12.125 -20 translate newpath user_shape_0\n";
    let (svg, _, _) = run_in(&dir, &["-Tsvg"], src);
    assert!(
        svg.contains(&format!("<title>d</title>\n{line}<text ")),
        "{svg}"
    );
    for fmt in ["-Tdot", "-Txdot", "-Tplain", "-Tcanon", "-Tjson0"] {
        let (out, _, _) = run_in(&dir, &[fmt], src);
        assert!(
            out.starts_with(line) && !out[1..].contains(line),
            "{fmt}\n{out}"
        );
    }
    let (json, _, _) = run_in(&dir, &["-Tjson"], src);
    assert!(json.starts_with(&format!("{line}{line}{{\n")), "{json}");
    let _ = std::fs::remove_file(dir.join("o.json"));
    let (stdout, _, code) = run_in(&dir, &["-Tjson", "-oo.json"], src);
    assert_eq!(code, 0);
    assert_eq!(stdout, line);
    let file = std::fs::read_to_string(dir.join("o.json")).unwrap();
    assert!(file.starts_with(&format!("{line}{{\n")), "{file}");
    // the macro number counts the files read, not the nodes
    let src = "digraph { a [shape=epsf shapefile=\"box.eps\"]; b [shape=epsf shapefile=\"box.eps\"]; \
               c [shape=epsf shapefile=\"./box.eps\"] }";
    let (dot, _, _) = run_in(&dir, &["-Tdot"], src);
    let calls: Vec<&str> = dot.lines().take(3).collect();
    assert_eq!(
        calls,
        [
            "-10 -20 translate newpath user_shape_0",
            "108 -20 translate newpath user_shape_0",
            "226 -20 translate newpath user_shape_1"
        ],
        "{dot}"
    );
}

// Issue #347: `user_init` keys `EPSF_contents` by the pointer `safefile` returned. A
// graph's own string no longer matches once the graph is closed, so a later graph reads
// the file again under a new number; `findPath`'s static buffer matches every lookup made
// through it, so with `imagepath` the first file read answers for every other.
#[test]
fn epsf_files_are_found_again_as_c_finds_them() {
    let dir = usershape_dir("i347-epsf-keys");
    std::fs::write(dir.join("small.eps"), "%%BoundingBox: 0 0 20 30\n").unwrap();
    let src = "digraph A { a [shape=epsf shapefile=\"box.eps\"] }\n\
               digraph B { b [shape=epsf shapefile=\"box.eps\"] }\n\
               digraph C { imagepath=\".\"; c [shape=epsf shapefile=\"box.eps\"]; \
               d [shape=epsf shapefile=\"small.eps\"] }\n";
    let (dot, err, code) = run_in(&dir, &["-Tdot"], src);
    assert_eq!((err.as_str(), code), ("", 0));
    let calls: Vec<&str> = dot
        .lines()
        .filter(|l| l.ends_with(char::is_numeric) && l.contains("user_shape_"))
        .collect();
    assert_eq!(
        calls,
        [
            "-10 -20 translate newpath user_shape_0",
            "-10 -20 translate newpath user_shape_1",
            "-10 -20 translate newpath user_shape_2",
            "108 -20 translate newpath user_shape_2"
        ],
        "{dot}"
    );
    assert!(dot.contains("graph [bb=\"0,0,218,50\""), "{dot}");
    let shape_calls = |dot: &str| -> Vec<String> {
        dot.lines()
            .filter(|l| l.contains("newpath user_shape_"))
            .map(str::to_string)
            .collect()
    };
    // C reads each node's file once: a file without a box is reported, and stored nowhere
    // (the port's repeat sizings must not find it later)
    let src = "digraph { imagepath=\".\"; a [shape=epsf shapefile=\"notes.txt\"]; \
               c [shape=epsf shapefile=\"small.eps\"] }";
    let (dot, err, _) = run_in(&dir, &["-Tdot"], src);
    assert_eq!(
        err,
        "Warning: BoundingBox not found in epsf file ./notes.txt\n"
    );
    assert_eq!(shape_calls(&dot), ["72 3 translate newpath user_shape_0"]);
    // keys compare by text: an absolute name finds the file the buffer read, and the
    // buffer's lookup finds the file last read under its text (paths follow C's Unix rules,
    // so the scratch directory is an absolute name only on Unix)
    if !cfg!(unix) {
        return;
    }
    let d = dir.display();
    let src = format!(
        "digraph {{ imagepath=\"{d}\"; a [shape=epsf shapefile=\"box.eps\"]; \
         b [shape=epsf shapefile=\"{d}/box.eps\"]; c [shape=epsf shapefile=\"{d}/small.eps\"]; \
         e [shape=epsf shapefile=\"small.eps\"] }}"
    );
    let (dot, _, _) = run_in(&dir, &["-Tsvg", "-Tdot"], &src);
    let calls = shape_calls(&dot);
    assert_eq!(
        calls[4..],
        [
            "-10 -20 translate newpath user_shape_0",
            "108 -20 translate newpath user_shape_0",
            "236 10 translate newpath user_shape_1",
            "274 10 translate newpath user_shape_1"
        ],
        "{dot}"
    );
}

// Issue #347: the size readers read headers only. Odd headers give what C reads, and a
// JPEG that ends before its frame header is warned about once, at its first sizing.
#[test]
fn image_headers_are_read_like_c() {
    let dir = scratch("i347-headers");
    let jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff";
    std::fs::write(dir.join("t.jpg"), jpeg).unwrap();
    let mut gif = b"GIF89a".to_vec();
    gif.extend([30, 0, 15, 0]);
    gif.extend([0; 10]);
    std::fs::write(dir.join("a.gif"), gif).unwrap();
    std::fs::write(
        dir.join("m.pdf"),
        "%PDF-1.4\n1 0 obj << /Type /Page /MediaBox [0 0 200.5 100] >> endobj\n",
    )
    .unwrap();
    let src = "digraph { node [shape=box label=\"\" margin=0 width=0 height=0]; \
               a [image=\"t.jpg\"]; b [image=\"t.jpg\"]; c [image=\"a.gif\"]; d [image=\"m.pdf\"] }";
    let (dot, err, code) = run_in(&dir, &["-Tdot"], src);
    assert_eq!(code, 0);
    assert_eq!(
        err,
        "Warning: Parsing of \"t.jpg\" failed\n\
         Warning: No loadimage plugin for \"pdf:dot\"\n"
    );
    for size in [
        "height=0.027778,\n\t\timage=\"t.jpg\",",
        "height=0.18056,\n\t\timage=\"a.gif\",",
        "\t\twidth=0.33333];",
        "height=1.0694,\n\t\timage=\"m.pdf\",",
        "\t\twidth=2.1111];",
    ] {
        assert!(dot.contains(size), "{size}\n{dot}");
    }
}

// Issue #268: a style with more than 63 items. C's `parse_style` warns `truncating style` and
// returns its static array unfilled (no items on a first call), so the node is drawn with no
// style at all and no `unsupported style` warnings. Output is C 16.1.0's.
#[test]
fn issue268_a_truncated_style_draws_no_style_items() {
    let style: Vec<String> = ["filled", "bold"]
        .into_iter()
        .map(String::from)
        .chain((0..62).map(|i| format!("s{i}")))
        .collect();
    let style = style.join(",");
    let (out, err, ok) = run(
        &["-Txdot"],
        &format!("digraph {{ a [style=\"{style}\" fillcolor=red] }}"),
    );
    assert!(ok);
    assert_eq!(
        err,
        format!("Warning: truncating style '{style}'\n").repeat(2)
    );
    let (head, tail) = style.split_at(style.find(",s32").unwrap() + 1);
    assert_eq!(
        out,
        format!(
            "digraph {{\n\
             \tgraph [_draw_=\"c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 36 54 36 54 0 \",\n\
             \t\tbb=\"0,0,54,36\",\n\
             \t\txdotversion=1.7\n\
             \t];\n\
             \tnode [label=\"\\N\"];\n\
             \ta\t[_draw_=\"c 7 -#000000 e 27 18 27 18 \",\n\
             \t\t_ldraw_=\"F 14 11 -Times-Roman c 7 -#000000 T 27 13.8 0 6.21 1 -a \",\n\
             \t\tfillcolor=red,\n\
             \t\theight=0.5,\n\
             \t\tpos=\"27,18\",\n\
             \t\tstyle=\"{head}\\\n{tail}\",\n\
             \t\twidth=0.75];\n\
             }}\n"
        )
    );
}

/// Issue #360: where a samehead/sametail port lies in no field of a record, C's
/// `record_path` writes no box and the route uses the uninitialised `tend`/`hend` of
/// `make_regular_edge` (or of the flat edge router), which hold the previous call's
/// ends. That is undefined behaviour in C: these coordinates are what C 16.1.0 draws on
/// every run and on two builds, the behaviour `docs/porting/crashes.md` ("Record ports
/// in no field") chooses to follow.
#[test]
fn issue360_record_port_in_no_field_reuses_the_previous_ends() {
    let regular = "digraph G {
n2 [shape=Mrecord, label=\"\"];
n3:c -> n2 [tailclip=true, samehead=h0, headport=se];
n2 -> n1:p0 [label=\"l\", penwidth=5];
n1:p1:nw -> n2 [samehead=h0, arrowhead=normal];
}";
    let (out, _, ok) = run(&["-Tplain"], regular);
    assert!(ok);
    for e in [
        "edge n3 n2 4 0.75 2.2563 0.75 1.8413 0.75 1.0028 0.75 0.65578 solid black",
        "edge n1 n2 4 0.45089 0.49801 0.52622 0.73043 0.63935 1.0795 0.7027 1.2749 solid black",
    ] {
        assert!(out.contains(e), "{out}");
    }

    let flat = "digraph G {
concentrate=true;
n4 [shape=Mrecord, label=\"<p0>  |<p1> xy|<p2> \"];
{ rank=same; n4; n0; n8; n9; n2; n6; }
n8:ne -> n2 [headclip=false, label=\"l\", arrowhead=normal];
n7:sw -> n6:e [tailclip=true, samehead=h0, constraint=false];
n6:p1:nw -> n0:_ [label=\"a label\"];
n4:s -> n9:n [headclip=false, sametail=t0, label=\"l\"];
n4 -> n2 [sametail=t0];
}";
    let (out, _, ok) = run(&["-Tplain"], flat);
    assert!(ok);
    assert!(
        out.contains("edge n4 n2 4 5.4549 0.29426 5.4549 0.29426 4.999 0.29426 4.6116 0.29426"),
        "{out}"
    );
}

/// Issue #323: deep nesting lays out in every engine that reads cluster members, with
/// the members each subgraph's lists hold (see `subgraph_members_are_the_subgraphs_own_lists`
/// for the size of what they collect). Clusters and plain subgraphs, 400 levels.
#[test]
fn deep_subgraph_nesting_lays_out_in_every_engine() {
    let nest = |prefix: &str| {
        let n = 400;
        let mut src = String::from("digraph G {\n");
        for i in 0..n {
            src.push_str(&format!("subgraph {prefix}{i} {{ x{i}\n"));
        }
        src.push_str(&"}\n".repeat(n));
        src.push('}');
        src
    };
    for prefix in ["cluster", "s"] {
        let src = nest(prefix);
        for engine in ["dot", "fdp", "osage", "patchwork"] {
            let (out, err, ok) = run(&[&format!("-K{engine}"), "-Tsvg"], &src);
            assert!(ok, "{engine} {prefix}: {err}");
            assert_eq!(err, "", "{engine} {prefix}");
            assert!(out.contains("<title>x399</title>"), "{engine} {prefix}");
        }
    }
}

/// Issue #323: fdp's cluster node is in its cluster and every ancestor (`clustNode`'s
/// `agsubnode(cg, cn, 1)`), so a layer that draws only the cluster node still draws the
/// outer cluster, as C does. An input node already named like the cluster node stays in
/// each subgraph once, and the layout is not collapsed.
#[test]
fn fdp_cluster_node_is_in_every_ancestor_cluster_once() {
    let (out, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        "digraph { layers=\"a:b\"; subgraph cluster_a { subgraph cluster_b { x [layer=a] } } \
         y [layer=a]; y -> cluster_b }",
    );
    assert!(ok, "{err}");
    assert!(
        out.contains("<g id=\"b_clust1\" class=\"cluster\">"),
        "{out}"
    );
    assert!(
        out.contains("<g id=\"b_clust2\" class=\"cluster\">"),
        "{out}"
    );
    let (out, err, ok) = run(
        &["-Kfdp", "-Tsvg"],
        "digraph { subgraph cluster_a { subgraph cluster_b { x; \"__0:cluster_b\" } } \
         y -> cluster_b }",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "");
    assert!(
        out.contains("<title>y&#45;&gt;__0:cluster_b</title>"),
        "{out}"
    );
}

// #268 on an edge and a cluster: with no items, the edge is neither `tapered` nor `invis` and
// gets no `bold`, and the cluster is not filled, radial, striped or rounded. Output is C 16.1.0's.
#[test]
fn issue268_a_truncated_edge_or_cluster_style_draws_no_style_items() {
    let items = |first: &[&str], n: usize| {
        let mut v: Vec<String> = first.iter().map(|s| s.to_string()).collect();
        v.extend((0..n).map(|i| format!("s{i}")));
        v.join(",")
    };
    let edge = items(&["tapered", "invis", "bold"], 61);
    let cluster = items(&["filled", "radial", "striped", "rounded"], 60);
    for (src, style, warnings, expected) in [
        (
            format!("digraph {{ a -> b [style=\"{edge}\" penwidth=3] }}"),
            &edge,
            1,
            r#"digraph {
	graph [_draw_="c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 108 54 108 54 0 ",
		bb="0,0,54,108",
		xdotversion=1.7
	];
	node [label="\N"];
	a	[_draw_="c 7 -#000000 e 27 90 27 18 ",
		_ldraw_="F 14 11 -Times-Roman c 7 -#000000 T 27 85.8 0 6.21 1 -a ",
		height=0.5,
		pos="27,90",
		width=0.75];
	b	[_draw_="c 7 -#000000 e 27 18 27 18 ",
		_ldraw_="F 14 11 -Times-Roman c 7 -#000000 T 27 13.8 0 7 1 -b ",
		height=0.5,
		pos="27,18",
		width=0.75];
	a -> b	[_draw_="S 15 -setlinewidth(3) c 7 -#000000 B 4 27 71.7 27 65.25 27 57.73 27 50.41 ",
		_hdraw_="S 15 -setlinewidth(3) S 5 -solid c 7 -#000000 C 7 -#000000 P 3 30.5 50.64 27 40.64 23.5 50.64 ",
		penwidth=3,
		pos="e,27,36.104 27,71.697 27,65.254 27,57.726 27,50.41",
		style="tapered,invis,bold,s0,s1,s2,s3,s4,s5,s6,s7,s8,s9,s10,s11,s12,s13,s14,s15,s16,s17,s18,s19,s20,s21,s22,s23,s24,s25,s26,s27,s28,s29,\
s30,s31,s32,s33,s34,s35,s36,s37,s38,s39,s40,s41,s42,s43,s44,s45,s46,s47,s48,s49,s50,s51,s52,s53,s54,s55,s56,s57,s58,s59,s60"];
}
"#,
        ),
        (
            format!(
                "digraph {{ subgraph cluster_x {{ style=\"{cluster}\"; fillcolor=\"red:blue\"; a }} }}"
            ),
            &cluster,
            1,
            r#"digraph {
	graph [_draw_="c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 68 86 68 86 0 ",
		bb="0,0,86,68",
		xdotversion=1.7
	];
	node [label="\N"];
	subgraph cluster_x {
		graph [_draw_="c 7 -#000000 p 4 8 8 8 60 78 60 78 8 ",
			bb="8,8,78,60",
			fillcolor="red:blue",
			style="filled,radial,striped,rounded,s0,s1,s2,s3,s4,s5,s6,s7,s8,s9,s10,s11,s12,s13,s14,s15,s16,s17,s18,s19,s20,s21,s22,s23,s24,s25,s26,\
s27,s28,s29,s30,s31,s32,s33,s34,s35,s36,s37,s38,s39,s40,s41,s42,s43,s44,s45,s46,s47,s48,s49,s50,s51,s52,s53,s54,s55,s56,s57,s58,\
s59"
		];
		a	[_draw_="c 7 -#000000 e 43 34 27 18 ",
			_ldraw_="F 14 11 -Times-Roman c 7 -#000000 T 43 29.8 0 6.21 1 -a ",
			height=0.5,
			pos="43,34",
			width=0.75];
	}
}
"#,
        ),
    ] {
        let (out, err, ok) = run(&["-Txdot"], &src);
        assert!(ok);
        assert_eq!(
            err,
            format!("Warning: truncating style '{style}'\n").repeat(warnings)
        );
        assert_eq!(out, expected, "{src}");
    }
}

// Issue #283: when `multicolor` fails, C draws the edge in `black` but keeps the original
// string's `:` count, and sets the pen for every `strtok` token because it compares the
// colours by pointer. With no token at all (`color=":"`), the arrows get the whole string,
// which `gvrender_set_pencolor` cuts at its `:`. Output is C 16.1.0's.
#[test]
fn issue283_failed_multicolor_draws_black_with_the_original_color_count() {
    let (out, err, ok) = run(&["-Txdot"], "digraph { a -> b [color=\"red;;0.5:blue\"] }");
    assert!(!ok);
    assert_eq!(
        err,
        "Error: Illegal value in \"red;;0.5:blue\" color attribute; float expected after ';'\n\
         in edge a -> b\n"
    );
    assert_eq!(
        out,
        "digraph {\n\
             \tgraph [_draw_=\"c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 108 54 108 54 0 \",\n\
             \t\tbb=\"0,0,54,108\",\n\
             \t\txdotversion=1.7\n\
             \t];\n\
             \tnode [label=\"\\N\"];\n\
             \ta\t[_draw_=\"c 7 -#000000 e 27 90 27 18 \",\n\
             \t\t_ldraw_=\"F 14 11 -Times-Roman c 7 -#000000 T 27 85.8 0 6.21 1 -a \",\n\
             \t\theight=0.5,\n\
             \t\tpos=\"27,90\",\n\
             \t\twidth=0.75];\n\
             \tb\t[_draw_=\"c 7 -#000000 e 27 18 27 18 \",\n\
             \t\t_ldraw_=\"F 14 11 -Times-Roman c 7 -#000000 T 27 13.8 0 7 1 -b \",\n\
             \t\theight=0.5,\n\
             \t\tpos=\"27,18\",\n\
             \t\twidth=0.75];\n\
             \ta -> b\t[_draw_=\"c 7 -#000000 B 4 26 71.7 26 64.41 26 55.73 26 47.54 \",\n\
             \t\t_hdraw_=\"S 5 -solid c 7 -#000000 C 7 -#000000 P 3 30.5 47.62 27 37.62 23.5 47.62 \",\n\
             \t\tcolor=\"red;;0.5:blue\",\n\
             \t\tpos=\"e,27,36.104 27,71.697 27,64.407 27,55.726 27,47.536\"];\n\
             }\n"
    );
    let (out, err, ok) = run(
        &["-Txdot"],
        "digraph { a -> b [color=\":\" dir=both arrowtail=dot] }",
    );
    assert!(ok);
    assert_eq!(err, "Warning:  is not a known color.\n");
    assert_eq!(
        out,
        "digraph {\n\
             \tgraph [_draw_=\"c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 108 54 108 54 0 \",\n\
             \t\tbb=\"0,0,54,108\",\n\
             \t\txdotversion=1.7\n\
             \t];\n\
             \tnode [label=\"\\N\"];\n\
             \ta\t[_draw_=\"c 7 -#000000 e 27 90 27 18 \",\n\
             \t\t_ldraw_=\"F 14 11 -Times-Roman c 7 -#000000 T 27 85.8 0 6.21 1 -a \",\n\
             \t\theight=0.5,\n\
             \t\tpos=\"27,90\",\n\
             \t\twidth=0.75];\n\
             \tb\t[_draw_=\"c 7 -#000000 e 27 18 27 18 \",\n\
             \t\t_ldraw_=\"F 14 11 -Times-Roman c 7 -#000000 T 27 13.8 0 7 1 -b \",\n\
             \t\theight=0.5,\n\
             \t\tpos=\"27,18\",\n\
             \t\twidth=0.75];\n\
             \ta -> b\t[_hdraw_=\"S 5 -solid c 7 -#000000 C 7 -#000000 P 3 30.5 47.62 27 37.62 23.5 47.62 \",\n\
             \t\t_tdraw_=\"S 5 -solid c 7 -#000000 C 7 -#000000 E 27 67.2 4 4 \",\n\
             \t\tarrowtail=dot,\n\
             \t\tcolor=\":\",\n\
             \t\tdir=both,\n\
             \t\tpos=\"s,27,71.697 e,27,36.104 27,62.797 27,57.776 27,52.497 27,47.411\"];\n\
             }\n"
    );
}

// Issue #283: `parse_layers` and `selectedLayer` split with `strtok`, on bytes: `layersep="é"`
// is the two delimiters 0xC3 and 0xA9, so `aèb` (0xC3 0xA8) splits too. The second layer's id
// starts with the lone byte 0xA8. Output is C 16.1.0's.
#[test]
#[cfg(unix)]
fn issue283_layers_split_on_the_bytes_of_layersep() {
    let (out, err, ok) = run_bytes(
        &[b"-Tsvg"],
        "digraph { layers=\"a\u{e8}b:c\"; layersep=\"\u{e9}\"; \
         a [layer=\"a\u{e8}b\"]; b [layer=\"c\"] }",
    );
    assert!(ok);
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    let start = out.windows(4).position(|w| w == b"-->\n").unwrap() + 4;
    // `@` stands for the lone byte 0xA8.
    let expected: Vec<u8> = "\
             <!-- Pages: 1 -->\n\
             <svg width=\"134pt\" height=\"44pt\"\n\
              \x20viewBox=\"0.00 0.00 134.00 44.00\" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n\
             <g id=\"a\" class=\"layer\">\n\
             <g id=\"graph0\" class=\"graph\" transform=\"scale(1 1) rotate(0) translate(4 40)\">\n\
             <polygon fill=\"white\" stroke=\"none\" points=\"-4,4 -4,-40 130,-40 130,4 -4,4\"/>\n\
             <!-- a -->\n\
             <g id=\"node1\" class=\"node\">\n\
             <title>a</title>\n\
             <ellipse fill=\"none\" stroke=\"black\" cx=\"27\" cy=\"-18\" rx=\"27\" ry=\"18\"/>\n\
             <text xml:space=\"preserve\" text-anchor=\"middle\" x=\"27\" y=\"-13.8\" font-family=\"Times,serif\" font-size=\"14.00\">a</text>\n\
             </g>\n\
             </g>\n\
             </g>\n\
             <g id=\"@b:c\" class=\"layer\">\n\
             <g id=\"@b:c_graph0\" class=\"graph\" transform=\"scale(1 1) rotate(0) translate(4 40)\">\n\
             <polygon fill=\"white\" stroke=\"none\" points=\"-4,4 -4,-40 130,-40 130,4 -4,4\"/>\n\
             </g>\n\
             </g>\n\
             </svg>\n\
         "
    .bytes()
    .map(|b| if b == b'@' { 0xA8 } else { b })
    .collect();
    assert_eq!(
        String::from_utf8_lossy(&out[start..]),
        String::from_utf8_lossy(&expected)
    );
    assert_eq!(out[start..], expected[..]);
}

// Issue #324: C's JSON `write_attrs` walks the root's attribute dictionaries, and writes
// `label` even when it is empty. `{} -> c [label=x]` makes no edge but declares the edge
// `label`, so `a -> b` gets `"label": ""`. Output is C 16.1.0's.
#[test]
fn issue324_json_writes_a_declared_edge_label_with_no_value() {
    let (out, err, ok) = run(&["-Tjson0"], "digraph { a -> b; {} -> c [label=x] }");
    assert!(ok);
    assert_eq!(err, "");
    assert_eq!(
        out,
        r#"{
  "name": "%1",
  "directed": true,
  "strict": false,
  "bb": "0,0,126,108",
  "_subgraph_cnt": 1,
  "objects": [
    {
      "name": "%5",
      "_gvid": 0
    },
    {
      "_gvid": 1,
      "name": "a",
      "height": "0.5",
      "label": "\\N",
      "pos": "27,90",
      "width": "0.75"
    },
    {
      "_gvid": 2,
      "name": "b",
      "height": "0.5",
      "label": "\\N",
      "pos": "27,18",
      "width": "0.75"
    },
    {
      "_gvid": 3,
      "name": "c",
      "height": "0.5",
      "label": "\\N",
      "pos": "99,90",
      "width": "0.75"
    }
  ],
  "edges": [
    {
      "_gvid": 0,
      "tail": 1,
      "head": 2,
      "label": "",
      "pos": "e,27,36.104 27,71.697 27,64.407 27,55.726 27,47.536"
    }
  ]
}
"#
    );
    // A subgraph's `edge [label=x]` declares `label` on the root too; the unattached
    // `-Tdot_json` writer.
    let (out, err, ok) = run(
        &["-Tdot_json"],
        "digraph { a -> b; subgraph s { edge [label=x] } }",
    );
    assert!(ok);
    assert_eq!(err, "");
    assert_eq!(
        out,
        r#"{
  "name": "%1",
  "directed": true,
  "strict": false,
  "_subgraph_cnt": 1,
  "objects": [
    {
      "name": "s",
      "_gvid": 0
    },
    {
      "_gvid": 1,
      "name": "a",
      "label": "\\N"
    },
    {
      "_gvid": 2,
      "name": "b",
      "label": "\\N"
    }
  ],
  "edges": [
    {
      "_gvid": 0,
      "tail": 1,
      "head": 2,
      "label": ""
    }
  ]
}
"#
    );

    // The attached writers: `-Tjson` (through the xdot job) and `-Txdot_json`, here for a
    // strict graph's keyed edge statement that makes no edge.
    for (format, src, expected) in [
        (
            "-Tjson",
            "digraph { a -> b; {} -> c [label=x] }",
            r##"{
  "name": "%1",
  "directed": true,
  "strict": false,
  "_draw_": 
  [
    {
      "op": "c",
      "grad": "none",
      "color": "#fffffe00"
    },
    {
      "op": "C",
      "grad": "none",
      "color": "#ffffff"
    },
    {
      "op": "P",
      "points": [[0.000,0.000],[0.000,108.000],[126.000,108.000],[126.000,0.000]]
    }
  ],
  "bb": "0,0,126,108",
  "xdotversion": "1.7",
  "_subgraph_cnt": 1,
  "objects": [
    {
      "name": "%5",
      "_gvid": 0
    },
    {
      "_gvid": 1,
      "name": "a",
      "_draw_": 
      [
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "e",
          "rect": [27.000,90.000,27.000,18.000]
        }
      ],
      "_ldraw_": 
      [
        {
          "op": "F",
          "size": 14.000,
          "face": "Times-Roman"
        },
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "T",
          "pt": [27.000,85.800],
          "align": "c",
          "width": 6.210,
          "text": "a"
        }
      ],
      "height": "0.5",
      "label": "\\N",
      "pos": "27,90",
      "width": "0.75"
    },
    {
      "_gvid": 2,
      "name": "b",
      "_draw_": 
      [
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "e",
          "rect": [27.000,18.000,27.000,18.000]
        }
      ],
      "_ldraw_": 
      [
        {
          "op": "F",
          "size": 14.000,
          "face": "Times-Roman"
        },
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "T",
          "pt": [27.000,13.800],
          "align": "c",
          "width": 7.000,
          "text": "b"
        }
      ],
      "height": "0.5",
      "label": "\\N",
      "pos": "27,18",
      "width": "0.75"
    },
    {
      "_gvid": 3,
      "name": "c",
      "_draw_": 
      [
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "e",
          "rect": [99.000,90.000,27.000,18.000]
        }
      ],
      "_ldraw_": 
      [
        {
          "op": "F",
          "size": 14.000,
          "face": "Times-Roman"
        },
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "T",
          "pt": [99.000,85.800],
          "align": "c",
          "width": 6.210,
          "text": "c"
        }
      ],
      "height": "0.5",
      "label": "\\N",
      "pos": "99,90",
      "width": "0.75"
    }
  ],
  "edges": [
    {
      "_gvid": 0,
      "tail": 1,
      "head": 2,
      "_draw_": 
      [
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "b",
          "points": [[27.000,71.700],[27.000,64.410],[27.000,55.730],[27.000,47.540]]
        }
      ],
      "_hdraw_": 
      [
        {
          "op": "S",
          "style": "solid"
        },
        {
          "op": "c",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "C",
          "grad": "none",
          "color": "#000000"
        },
        {
          "op": "P",
          "points": [[30.500,47.620],[27.000,37.620],[23.500,47.620]]
        }
      ],
      "label": "",
      "pos": "e,27,36.104 27,71.697 27,64.407 27,55.726 27,47.536"
    }
  ]
}
"##,
        ),
        (
            "-Txdot_json",
            "strict digraph { a -> b; a -> b [key=k, label=L] }",
            r##"{
  "name": "%1",
  "directed": true,
  "strict": true,
  "_subgraph_cnt": 0,
  "objects": [
    {
      "_gvid": 0,
      "name": "a",
      "label": "\\N"
    },
    {
      "_gvid": 1,
      "name": "b",
      "label": "\\N"
    }
  ],
  "edges": [
    {
      "_gvid": 0,
      "tail": 0,
      "head": 1,
      "label": ""
    }
  ]
}
"##,
        ),
    ] {
        let (out, err, ok) = run(&[format], src);
        assert!(ok);
        assert_eq!(err, "");
        assert_eq!(out, expected, "{format} {src}");
    }
}

// Issue #304: `bindattrs` declares a new graph attribute with `agattr(S->g, AGRAPH, name, "")`,
// whose `agxset_` gives the subgraph a local symbol. The statement then updates that local
// symbol, so `unviewsubgraphsattr` gives each child subgraph its own `""`, an anonymous one
// too. Output is C 16.1.0's.
#[test]
fn issue304_a_new_graph_attribute_gives_child_subgraphs_their_empty_value() {
    for (src, expected) in [
        (
            "graph { subgraph cluster_a { subgraph cluster_b { x } graph [xlabel=\"1\"] } }",
            r#"graph {
	graph [bb="0,0,102,84"];
	node [label="\N"];
	subgraph cluster_a {
		graph [bb="8,8,94,76",
			xlabel=1
		];
		subgraph cluster_b {
			graph [bb="16,16,86,68",
				xlabel=""
			];
			x	[height=0.5,
				pos="51,42",
				width=0.75];
		}
	}
}
"#,
        ),
        (
            "digraph { subgraph cluster0 { a -> {c b}; label=\"cluster0\" } }",
            r#"digraph {
	graph [bb="0,0,158,164.8"];
	node [label="\N"];
	subgraph cluster0 {
		graph [bb="8,8,150,156.8",
			label=cluster0,
			lheight=0.23,
			lp="79,144.4",
			lwidth=0.62
		];
		{
			graph [label=""];
			c	[height=0.5,
				pos="43,34",
				width=0.75];
			b	[height=0.5,
				pos="115,34",
				width=0.75];
		}
		a	[height=0.5,
			pos="79,106",
			width=0.75];
		a -> c	[pos="e,51.304,51.147 70.65,88.765 66.425,80.548 61.192,70.373 56.419,61.093"];
		a -> b	[pos="e,106.7,51.147 87.35,88.765 91.575,80.548 96.808,70.373 101.58,61.093"];
	}
}
"#,
        ),
        (
            // A subgraph opened again by name: its children are the first opening's.
            "graph { subgraph cluster_a { subgraph cluster_b { x } } subgraph cluster_a { label=L } }",
            r#"graph {
	graph [bb="0,0,102,108.8"];
	node [label="\N"];
	subgraph cluster_a {
		graph [bb="8,8,94,100.8",
			label=L,
			lheight=0.23,
			lp="51,88.4",
			lwidth=0.12
		];
		subgraph cluster_b {
			graph [bb="16,16,86,68",
				label=""
			];
			x	[height=0.5,
				pos="51,42",
				width=0.75];
		}
	}
}
"#,
        ),
    ] {
        let (out, err, ok) = run(&["-Tdot"], src);
        assert!(ok);
        assert_eq!(err, "");
        assert_eq!(out, expected, "{src}");
    }
}

/// Issue #332: `Nop` is a global of the C process. `nop1_layout`/`nop2_layout`
/// (`gvlayout_neato_layout.c:48-57`) and `fdpSplines` with splines (`fdpgen/layout.c:1057`)
/// set it to 0 after their layout, so `-n2` no longer applies to the graphs after them;
/// fdp with line edges leaves it.
#[test]
fn issue332_nop_engines_and_fdp_splines_clear_dash_n() {
    let laid_out = "graph 1 1.739 0.64777
node b 0.375 0.39777 0.75 0.5 b solid ellipse black lightgrey
node c 1.364 0.25 0.75 0.5 c solid ellipse black lightgrey
edge b c 4 0.74328 0.34274 0.82456 0.3306 0.91072 0.31773 0.99214 0.30556 solid black
stop
";
    let b = "graph B { layout=neato; b; c; b -- c }\n";
    let fdp = "graph 1 1.6592 0.63584
node a 0.375 0.38584 0.75 0.5 a solid ellipse black lightgrey
node x 1.2842 0.25 0.75 0.5 x solid ellipse black lightgrey
edge a x 4 0.7448 0.33059 0.79986 0.32236 0.85664 0.31388 0.91176 0.30564 solid black
stop
";
    let nop = "graph 1 0.75 0.5
node a 0.375 0.25 0.75 0.5 a solid ellipse black lightgrey
stop
";
    for engine in ["nop", "nop1", "nop2"] {
        let src = format!("graph A {{ layout={engine}; a [pos=\"0,0\"] }}\n{b}");
        let (out, err, ok) = run(&["-n2", "-Tplain"], &src);
        assert!(ok, "{err}");
        assert_eq!(out, format!("{nop}{laid_out}"));
        assert_eq!(err, "");
    }
    let src = format!("graph A {{ layout=fdp; splines=true; a -- x }}\n{b}");
    let (out, err, ok) = run(&["-n2", "-Tplain"], &src);
    assert!(ok, "{err}");
    assert_eq!(out, format!("{fdp}{laid_out}"));
    assert_eq!(err, "");

    let src = format!("graph A {{ layout=fdp; a -- x }}\n{b}");
    let (out, err, ok) = run(&["-n2", "-Tplain"], &src);
    assert!(!ok);
    assert_eq!(
        out,
        format!(
            "{fdp}graph 1 0 0
node b 0 0 0.75 0.5 b solid ellipse black lightgrey
node c 0 0 0.75 0.5 c solid ellipse black lightgrey
 solid black
stop
"
        )
    );
    assert_eq!(
        err,
        "Error: node b in graph B has no position\nas required by the -n flag\n"
    );
}

/// Issue #332: `Nop` is set while every engine runs, as C's global, so `-n2` reaches the
/// `Nop > 1` branches of `neato_set_aspect` and `spline_edges_` in fdp, sfdp and circo too.
/// C's output under `-n2` is the output without it, for `ratio`, splines and pack.
#[test]
fn issue332_dash_n2_with_fdp_sfdp_and_circo_matches_c() {
    let fill = r#"graph G { ratio=fill; size="3,2!"; splines=true; pack=true; a -- b; b -- c; c -- a [label=x]; d -- e; e -- e }"#;
    let ortho = r#"graph G { ratio=2; splines=ortho; pack=10; a -- b [pos="0,0 10,10 20,20 30,30"]; b -- c; d -- e [label=y] }"#;
    for (src, engine, want) in [
        (
            fill,
            "fdp",
            r#"graph 0.87298 3.4365 2.1243
node a 0.375 1.713 0.75 0.5 a solid ellipse black lightgrey
node b 1.3008 1.6932 0.75 0.5 b solid ellipse black lightgrey
node c 0.82775 0.41642 0.75 0.5 c solid ellipse black lightgrey
node d 1.9023 0.63484 0.75 0.5 d solid ellipse black lightgrey
node e 2.8115 0.41134 0.75 0.5 e solid ellipse black lightgrey
edge a b 4 0.75158 1.7049 0.80765 1.7037 0.86547 1.7025 0.9216 1.7013 solid black
edge b c 4 1.2096 1.447 1.1259 1.2209 1.0024 0.88768 0.91869 0.66183 solid black
edge c a 4 0.74046 0.66641 0.66029 0.896 0.54211 1.2344 0.46203 1.4638 x 0.5526 1.1818 solid black
edge d e 4 2.2564 0.54779 2.3222 0.53161 2.391 0.51471 2.4568 0.49852 solid black
edge e e 7 3.1649 0.50427 3.3119 0.51068 3.4365 0.4797 3.4365 0.41134 3.4365 0.34298 3.3119 0.31201 3.1649 0.31842 solid black
stop
"#,
        ),
        (
            fill,
            "sfdp",
            r#"graph 0.44388 6.7586 3.9674
node a 0.375 3.7174 0.75 0.5 a solid ellipse black lightgrey
node b 0.61482 0.25 0.75 0.5 b solid ellipse black lightgrey
node c 3.5088 2.1906 0.75 0.5 c solid ellipse black lightgrey
node d 3.5 0.94444 0.75 0.5 d solid ellipse black lightgrey
node e 6.1336 0.94444 0.75 0.5 e solid ellipse black lightgrey
edge a b 4 0.39263 3.4624 0.43698 2.8212 0.55278 1.147 0.59716 0.50534 solid black
edge b c 4 0.88189 0.42909 1.4332 0.79879 2.6916 1.6426 3.2424 2.012 solid black
edge c a 4 3.2026 2.3398 2.6035 2.6317 1.2825 3.2753 0.68243 3.5676 x 1.8943 3.0702 solid black
edge d e 4 3.8773 0.94444 4.3766 0.94444 5.2517 0.94444 5.7527 0.94444 solid black
edge e e 7 6.487 1.0374 6.634 1.0438 6.7586 1.0128 6.7586 0.94444 6.7586 0.87609 6.634 0.84511 6.487 0.85152 solid black
stop
"#,
        ),
        (
            fill,
            "circo",
            r#"graph 0.6389 4.6956 2.9637
node a 0.375 0.30293 0.75 0.5 a solid ellipse black lightgrey
node b 0.375 2.0566 0.75 0.5 b solid ellipse black lightgrey
node c 1.6283 1.1798 0.75 0.5 c solid ellipse black lightgrey
node d 2.3206 2.6608 0.75 0.5 d solid ellipse black lightgrey
node e 4.0706 2.6608 0.75 0.5 e solid ellipse black lightgrey
edge a b 4 0.375 0.55793 0.375 0.89041 0.375 1.4687 0.375 1.8013 solid black
edge b c 4 0.63469 1.8749 0.84836 1.7254 1.1499 1.5145 1.3647 1.3642 solid black
edge c a 4 1.3687 0.99809 1.155 0.8486 0.85343 0.63764 0.63867 0.4874 x 0.95546 0.85969 solid black
edge d e 4 2.7003 2.6608 2.9938 2.6608 3.3982 2.6608 3.6916 2.6608 solid black
edge e e 7 4.4239 2.7537 4.5709 2.7601 4.6956 2.7291 4.6956 2.6608 4.6956 2.5924 4.5709 2.5614 4.4239 2.5678 solid black
stop
"#,
        ),
        (
            ortho,
            "fdp",
            r#"graph 1 3.2879 6.5758
node a 0.375 4.6115 0.75 0.5 a solid ellipse black lightgrey
node b 1.2228 2.9589 0.75 0.5 b solid ellipse black lightgrey
node c 1.2364 0.67521 0.75 0.5 c solid ellipse black lightgrey
node d 2.0037 5.9006 0.75 0.5 d solid ellipse black lightgrey
node e 2.9129 5.5337 0.75 0.5 e solid ellipse black lightgrey
edge a b 7 0.75428 4.6111 0.98003 4.6111 1.2228 4.6111 1.2228 4.6111 1.2228 4.6111 1.2228 3.6769 1.2228 3.2134 solid black
edge b c 4 1.2296 2.7058 1.2296 2.2668 1.2296 1.3692 1.2296 0.92947 solid black
edge d e 4 2.2765 5.7222 2.3979 5.7222 2.5399 5.7222 2.659 5.7222 y 2.4191 5.8389 solid black
stop
"#,
        ),
        (
            ortho,
            "sfdp",
            r#"graph 1 7.1614 1.8333
node a 0.375 0.25 0.75 0.5 a solid ellipse black lightgrey
node b 2.8237 0.25 0.75 0.5 b solid ellipse black lightgrey
node c 5.2535 0.25 0.75 0.5 c solid ellipse black lightgrey
node d 4.1528 1.5833 0.75 0.5 d solid ellipse black lightgrey
node e 6.7864 1.5833 0.75 0.5 e solid ellipse black lightgrey
edge a b 4 0.75224 0.25 1.2111 0.25 1.9812 0.25 2.442 0.25 solid black
edge b c 4 3.2033 0.25 3.6585 0.25 4.4168 0.25 4.8725 0.25 solid black
edge d e 4 4.5301 1.5833 5.0293 1.5833 5.9045 1.5833 6.4055 1.5833 y 5.4192 1.7 solid black
stop
"#,
        ),
        (
            ortho,
            "circo",
            r#"graph 1 5.75 11.5
node a 0.375 1.6299 0.75 0.5 a solid ellipse black lightgrey
node b 2.125 1.6299 0.75 0.5 b solid ellipse black lightgrey
node c 3.875 1.6299 0.75 0.5 c solid ellipse black lightgrey
node d 3.625 9.8701 0.75 0.5 d solid ellipse black lightgrey
node e 5.375 9.8701 0.75 0.5 e solid ellipse black lightgrey
edge a b 4 0.75472 1.625 1.0483 1.625 1.4527 1.625 1.746 1.625 solid black
edge b c 4 2.5047 1.625 2.7983 1.625 3.2027 1.625 3.496 1.625 solid black
edge d e 4 4.0047 9.875 4.2983 9.875 4.7027 9.875 4.996 9.875 y 4.4518 9.9917 solid black
stop
"#,
        ),
    ] {
        let (out, err, ok) = run(&[&format!("-K{engine}"), "-n2", "-Tplain"], src);
        assert!(ok, "{engine}: {err}");
        assert_eq!(err, "", "{engine}");
        assert_eq!(out, want, "{engine}");
    }
}

/// Issue #331: `neato_init_node_edge` (`lib/neatogen/neatoinit.c:131-141`) runs each node's
/// `common_init_node` then its `user_pos`, so a node's `pos` error comes before the next
/// node's shape warning; with `-n`, every edge is initialised after that.
#[test]
fn issue331_user_pos_errors_follow_each_nodes_init() {
    let src = "graph G { a [pos=\" 3 , 4 \"]; b [shape=foo, pos=\"10,10\"]; a -- b; }";
    let pos = "Error: node a, position  3 , 4 , expected two doubles\n\
               Warning: using box for unknown shape foo\n";
    let (_, err, ok) = run(&["-Kneato", "-Tdot"], src);
    assert!(!ok);
    assert_eq!(err, pos);
    let (_, err, ok) = run(&["-Kneato", "-n", "-Tdot"], src);
    assert!(!ok);
    assert_eq!(
        err,
        format!("{pos}Error: node a in graph G has no position\nas required by the -n flag\n")
    );
}

/// Issue #331: C warns about an unknown arrow type each time `arrow_flags` runs, and only
/// the layout runs it: `arrow_clip` as each edge is clipped (dot routes the self loop
/// first), `user_spline` for an edge with a `pos` (after the empty `pos` test), and the
/// opposing edge of a concentrated one. Without routed edges (`splines=none`, patchwork)
/// or before `-n` stops at a node with no position, there is no warning.
#[test]
fn issue331_arrow_warnings_come_from_the_layouts_arrow_flags() {
    let src = "digraph { a -> b [arrowhead=bogus]; b -> c [arrowtail=foo, dir=both]; \
               c -> c [arrowhead=zzz] }";
    let routed = "Warning: Arrow type \"zzz\" unknown - ignoring\n\
                  Warning: Arrow type \"bogus\" unknown - ignoring\n\
                  Warning: Arrow type \"foo\" unknown - ignoring\n";
    for args in [&["-Tsvg"][..], &["-Gconcentrate=true", "-Tsvg"]] {
        let (_, err, ok) = run(args, src);
        assert!(ok);
        assert_eq!(err, routed, "{args:?}");
    }
    for args in [&["-Gsplines=none", "-Tsvg"][..], &["-Kpatchwork", "-Tsvg"]] {
        let (_, err, ok) = run(args, src);
        assert!(ok);
        assert_eq!(err, "", "{args:?}");
    }
    // An edge without a `pos` warns once, in arrow_clip, not in user_spline too.
    let (_, err, ok) = run(
        &["-Kneato", "-n2", "-Tsvg"],
        "digraph G { a [pos=\"0,0\"]; b [pos=\"100,100\"]; a -> b [arrowhead=bogus]; \
         b -> a [arrowhead=x2, pos=\"\"] }",
    );
    assert!(ok);
    assert_eq!(
        err,
        "Warning: Arrow type \"bogus\" unknown - ignoring\n\
         Warning: Arrow type \"x2\" unknown - ignoring\n"
    );
    let (_, err, ok) = run(
        &["-Kneato", "-n2", "-Tdot"],
        "digraph G { a -> b [arrowhead=bogus]; c [pos=\"1,1\"] }",
    );
    assert!(!ok);
    assert_eq!(
        err,
        "Error: node a in graph G has no position\nas required by the -n flag\n"
    );
}

/// Issue #285: `arrow_clip` (splines.c:64) swaps a backwards-routed edge's arrow flags
/// (`swap_ends`) and `emit_edge_graphics` draws the spline's `sflag`/`eflag`. The renderer
/// rebuilt the types from the edge's attributes and, for `f -> g:n`, took its swapped start
/// arrow from the opposing `g -> f`: a normal arrowhead where C draws a dot. The expected
/// SVG is C 16.1.0's, after the generator comment.
#[test]
fn issue285_swapped_end_draws_the_layouts_arrow_type() {
    let src = "digraph G { rankdir=LR; subgraph cluster1 { c; e; a subgraph cluster1_in { f } \
               label=C1; } f -> g [minlen=0]; g -> f; f -> g [headport=n,arrowhead=dot]; e -> f; }";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    // `f -> g:n`'s start arrowhead, at `g`, is C's dot (an ellipse), not a polygon.
    let edge3 = out
        .split("<g id=\"edge3\"")
        .nth(1)
        .and_then(|e| e.split("</g>").next());
    assert!(
        edge3.is_some_and(|e| e.contains(
            "<ellipse fill=\"black\" stroke=\"black\" \
            cx=\"135.74\" cy=\"-241.89\" rx=\"4\" ry=\"4\"/>"
        ) && !e.contains("<polygon")),
        "{out}"
    );
    let want = r#"<!-- Title: G Pages: 1 -->
<svg width="192pt" height="254pt"
 viewBox="0.00 0.00 192.00 254.00" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
<g id="graph0" class="graph" transform="scale(1 1) rotate(0) translate(4 250.11)">
<title>G</title>
<polygon fill="white" stroke="none" points="-4,4 -4,-250.11 188,-250.11 188,4 -4,4"/>
<g id="clust1" class="cluster">
<title>cluster1</title>
<polygon fill="none" stroke="black" points="8,-8 8,-193 176,-193 176,-8 8,-8"/>
<text xml:space="preserve" text-anchor="middle" x="92" y="-176.4" font-family="Times,serif" font-size="14.00">C1</text>
</g>
<g id="clust2" class="cluster">
<title>cluster1_in</title>
<polygon fill="none" stroke="black" points="98,-108 98,-160 168,-160 168,-108 98,-108"/>
</g>
<!-- c -->
<g id="node1" class="node">
<title>c</title>
<ellipse fill="none" stroke="black" cx="43" cy="-34" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="43" y="-29.8" font-family="Times,serif" font-size="14.00">c</text>
</g>
<!-- e -->
<g id="node2" class="node">
<title>e</title>
<ellipse fill="none" stroke="black" cx="43" cy="-88" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="43" y="-83.8" font-family="Times,serif" font-size="14.00">e</text>
</g>
<!-- f -->
<g id="node4" class="node">
<title>f</title>
<ellipse fill="none" stroke="black" cx="133" cy="-134" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="133" y="-129.8" font-family="Times,serif" font-size="14.00">f</text>
</g>
<!-- e&#45;&gt;f -->
<g id="edge4" class="edge">
<title>e&#45;&gt;f</title>
<path fill="none" stroke="black" d="M64.85,-98.9C75.63,-104.53 88.98,-111.51 100.94,-117.77"/>
<polygon fill="black" stroke="black" points="99.16,-120.79 109.65,-122.32 102.41,-114.58 99.16,-120.79"/>
</g>
<!-- a -->
<g id="node3" class="node">
<title>a</title>
<ellipse fill="none" stroke="black" cx="43" cy="-142" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="43" y="-137.8" font-family="Times,serif" font-size="14.00">a</text>
</g>
<!-- g -->
<g id="node5" class="node">
<title>g</title>
<ellipse fill="none" stroke="black" cx="133" cy="-219" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="133" y="-214.8" font-family="Times,serif" font-size="14.00">g</text>
</g>
<!-- f&#45;&gt;g -->
<g id="edge1" class="edge">
<title>f&#45;&gt;g</title>
<path fill="none" stroke="black" d="M133,-189.66C133,-177.32 133,-163.24 133,-152.46"/>
<polygon fill="black" stroke="black" points="129.5,-189.29 133,-199.29 136.5,-189.29 129.5,-189.29"/>
</g>
<!-- f&#45;&gt;g -->
<g id="edge3" class="edge">
<title>f&#45;&gt;g:n</title>
<path fill="none" stroke="black" d="M138.44,-245.39C144.61,-246.82 154.53,-243 160,-237.32 182.95,-213.51 160.28,-172.6 144.79,-150.4"/>
<ellipse fill="black" stroke="black" cx="135.74" cy="-241.89" rx="4" ry="4"/>
</g>
<!-- g&#45;&gt;f -->
<g id="edge2" class="edge">
<title>g&#45;&gt;f</title>
<path fill="none" stroke="black" d="M133,-200.75C133,-188.38 133,-176 133,-163.63"/>
<polygon fill="black" stroke="black" points="136.5,-163.78 133,-153.78 129.5,-163.78 136.5,-163.78"/>
</g>
</g>
</svg>
"#;
    assert_eq!(out.split_once(" -->\n").map(|(_, b)| b), Some(want));
}

/// Issue #285, every engine: the shared `write_back` passes `bz.sflag`/`bz.eflag` too, so
/// neato's arrowheads are the ones its `arrow_clip` clipped for (the SVG, C 16.1.0's after
/// the generator comment, was already right: neato swaps no ends). What fails without the
/// change is the warning: under `concentrate`, neato routes one edge of `b -> a`'s pair; the
/// other, with an unknown `invfoo`, is never clipped, so C prints nothing for it. The
/// warnings come from the layout's `arrow_flags`, not the renderer's.
#[test]
fn issue285_neato_concentrate_draws_and_warns_from_the_layouts_arrow_flags() {
    let src = "digraph G { layout=neato; concentrate=true; { rank=same; a b } b -> a [dir=none,minlen=2]; b -> b [arrowhead=inv,tailport=s]; b -> a [arrowhead=invfoo,dir=forward,minlen=2]; a -> a; }";
    let (out, err, ok) = run(&["-Tsvg"], src);
    assert!(ok, "{err}");
    assert_eq!(err, "");
    let want = r#"<!-- Title: G Pages: 1 -->
<svg width="151pt" height="64pt"
 viewBox="0.00 0.00 151.00 64.00" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
<g id="graph0" class="graph" transform="scale(1 1) rotate(0) translate(4 59.96)">
<title>G</title>
<polygon fill="white" stroke="none" points="-4,4 -4,-59.96 147.21,-59.96 147.21,4 -4,4"/>
<!-- a -->
<g id="node1" class="node">
<title>a</title>
<ellipse fill="none" stroke="black" cx="27" cy="-37.96" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="27" y="-33.76" font-family="Times,serif" font-size="14.00">a</text>
</g>
<!-- a&#45;&gt;a -->
<g id="edge4" class="edge">
<title>a&#45;&gt;a</title>
<path fill="none" stroke="black" d="M52.44,-44.65C63.03,-45.11 72,-42.88 72,-37.96 72,-34.96 68.67,-32.96 63.67,-31.96"/>
<polygon fill="black" stroke="black" points="64.15,-28.48 53.95,-31.36 63.72,-35.47 64.15,-28.48"/>
</g>
<!-- b -->
<g id="node2" class="node">
<title>b</title>
<ellipse fill="none" stroke="black" cx="98.21" cy="-27.32" rx="27" ry="18"/>
<text xml:space="preserve" text-anchor="middle" x="98.21" y="-23.12" font-family="Times,serif" font-size="14.00">b</text>
</g>
<!-- b&#45;&gt;a -->
<g id="edge1" class="edge">
<title>b&#45;&gt;a</title>
<path fill="none" stroke="black" d="M71.69,-31.28C65.84,-32.16 59.64,-33.08 53.78,-33.96"/>
</g>
<!-- b&#45;&gt;b -->
<g id="edge2" class="edge">
<title>b:s&#45;&gt;b</title>
<path fill="none" stroke="black" d="M98.21,-9C113.21,0 143.21,0 143.21,-18.16 143.21,-24.43 139.63,-28.54 134.33,-30.85"/>
<polygon fill="black" stroke="black" points="123.83,-29.61 134.35,-30.85 125.35,-36.44 123.83,-29.61"/>
</g>
</g>
</svg>
"#;
    assert_eq!(out.split_once(" -->\n").map(|(_, b)| b), Some(want));
}

/// Issue #285: an unknown arrow name warns where the layout reads the edge's arrow flags
/// (`arrow_clip`, `user_spline`), once per layout whatever the number of output formats, as
/// in C 16.1.0.
#[test]
fn issue285_arrow_type_warnings_print_once_per_layout() {
    let src = "digraph G { a -> b [arrowhead=bogus]; b -> c [arrowtail=nope,dir=both]; \
               c -> a [arrowhead=xx]; }";
    for k in ["-Kdot", "-Kneato", "-Kfdp", "-Kcirco", "-Ktwopi", "-Kosage"] {
        let (_, err, ok) = run(&[k, "-Tsvg", "-Tdot"], src);
        assert!(ok, "{err}");
        assert_eq!(
            err,
            "Warning: Arrow type \"bogus\" unknown - ignoring\n\
             Warning: Arrow type \"nope\" unknown - ignoring\n\
             Warning: Arrow type \"xx\" unknown - ignoring\n",
            "{k}"
        );
    }
    // `neato -n2`: `user_spline` returns before `arrow_flags` for an edge without `pos`
    // (neatoinit.c:272), which is then routed and clipped: one warning. This guards that
    // early return, which the layout's parser needs now that it warns.
    let (_, err, ok) = run(
        &["-Kneato", "-n2", "-Tsvg"],
        "digraph { a [pos=\"0,0\"]; b [pos=\"100,0\"]; a -> b [arrowhead=foo]; }",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "Warning: Arrow type \"foo\" unknown - ignoring\n");
}

/// Issue #370: `parse_reclbl` measures each field as it closes (`make_label` at
/// `lib/common/shapes.c:3444`), so a field read before a syntax error, nested ones included,
/// is measured and warned about before `bad label format`. The width warning prints once
/// per process, so each label runs alone. Output is C 16.1.0's.
#[test]
fn issue370_a_record_field_before_a_syntax_error_is_measured() {
    for (label, byte) in [("{¤|", 194), ("{x|{ÿ|", 195)] {
        let src = format!("digraph {{ a [shape=record, label=\"{label}\"] }}");
        let (out, err, ok) = run(&["-Tcanon"], &src);
        assert!(!ok);
        assert_eq!(
            err,
            format!(
                "Warning: Warning: no value for width of non-ASCII character {byte}. Falling \
                 back to width of space character\nError: bad label format {label}\n"
            )
        );
        assert_eq!(
            out,
            format!(
                "digraph {{\n\tnode [label=\"\\N\"];\n\ta\t[label=\"{label}\",\n\t\t\
                 shape=record];\n}}\n"
            )
        );
    }
}

/// Issue #370: in an HTML record label, a hard space after a sub-table (`x|{a}\ |`) adds
/// text to the field before the table, so the last close measures field 0 a second time
/// (`shapes.c:3444` with `fp` still field 0) and keeps that label. Output is C 16.1.0's.
#[test]
fn issue370_html_record_hard_space_after_a_subtable_remeasures_the_field() {
    let (out, err, ok) = run(
        &["-Txdot"],
        r"digraph { a [shape=record, label=<x|{a}\ |>] }",
    );
    assert!(ok, "{err}");
    assert_eq!(err, "");
    assert_eq!(
        out,
        r#"digraph {
	graph [_draw_="c 9 -#fffffe00 C 7 -#ffffff P 4 0 0 0 37 61.6 37 61.6 0 ",
		bb="0,0,61.604,37",
		xdotversion=1.7
	];
	node [label="\N"];
	a	[_draw_="c 7 -#000000 p 4 0 0.5 0 36.5 61.6 36.5 61.6 0.5 c 7 -#000000 L 2 19.89 0.5 19.89 36.5 c 7 -#000000 L 2 42.1 0.5 42.1 36.5 ",
		_ldraw_="F 14 11 -Times-Roman c 7 -#000000 T 8 12.9 -1 3.89 1 -\\ F 14 11 -Times-Roman c 7 -#000000 T 27.89 13 -1 6.21 1 -a F 14 11 -Times-Roman \
c 7 -#000000 T 50.1 12.9 -1 3.5 1 -  ",
		height=0.51389,
		label=<x|{a}\ |>,
		pos="30.802,18.5",
		rects="0,0.5,19.89,36.5 19.89,0.7,42.104,36.5 42.104,0.5,61.604,36.5",
		shape=record,
		width=0.8556];
}
"#
    );
}

/// neato ortho obstacles (`makeObstacle` with `isOrtho`) keep `margin = {0}`: a scaling
/// `sep`/`esep` shrinks each box to its node's centre, so touching boxes are not reported.
#[test]
fn issue30_neato_ortho_scaled_margin_does_not_touch() {
    let src = "graph G { splines=ortho; esep=0.5; a [pos=\"0,0!\"]; b [pos=\"0.5,0!\"]; \
c [pos=\"3,2!\"]; a -- c; b -- c }";
    assert!(!warnings("neato", src).0.contains("touch"));
    let src = src.replace("esep=0.5", "esep=\"+0.5\"");
    assert!(
        warnings("neato", &src)
            .0
            .contains("the bounding boxes of some nodes touch")
    );
}

/// Issue #127: when the ortho maze fails, C prints `add_segment`'s and `partition`'s bare
/// `fprintf` messages (`lib/ortho/trapezoid.c:623`, `partition.c:694/711`), and before its
/// `chkSgraph` assertion, the node it fails at (`maze.c:315/317`). No prefix, no error status.
#[test]
fn issue127_ortho_maze_failures_print_cs_messages() {
    let src = "graph G { layout=neato; splines=ortho;
a [shape=circle, fixedsize=shape, width=0.3, label=\"a very long label here\", pos=\"0,0!\"];
b [pos=\"1.0,0!\"]; c [pos=\"0,1.0!\"]; a -- b; b -- c; a -- c; }";
    let (err, ok) = warnings("neato", src);
    assert!(ok);
    assert_eq!(
        err,
        "trapezoid segment construction failed\nvertical trapezoid construction failed\n"
    );
    let src = "graph { splines=ortho; a [pos=\"1e300,0!\"]; b [pos=\"1.125,0!\"]; \
               c [pos=\"0,1!\"]; a -- b; b -- c }";
    assert_eq!(
        warnings("neato", src).0,
        "trapezoid segment construction failed\nhorizontal trapezoid construction failed\n"
    );
    // C fails its assertion after the message (exit 134); the port draws straight edges.
    let src = "digraph{layout=osage
splines=ortho{label=\"inner\"{o2{i1ortho
cluster_outer{o1}subgraph cluster_inner4{j1 j2}}subgrapa}subgraph cluster_inner4{j2}}x i2 i3}
";
    assert_eq!(warnings("osage", src).0, "failed at node 1[1]\n");
}

/// A `fixedsize=shape` node's ortho obstacle is its polygon's box (`polyBB`,
/// `lib/neatogen/neatosplines.c:351-358`), not the node size that holds the label.
#[test]
fn issue30_neato_ortho_fixedshape_obstacle_is_the_polygon() {
    let src = "graph { splines=ortho; k [shape=box fixedsize=shape width=0.3 \
label=\"a very wide label\" pos=\"0,0!\"]; b [pos=\"0.9,0.3!\"]; c [pos=\"0,2!\"]; k -- c; b -- c }";
    assert!(!warnings("neato", src).0.contains("touch"));
}

/// Issue #88: graph_init makes the root graph label before dot sizes the nodes, so the
/// label's font warnings come first.
#[test]
fn issue88_root_label_font_warnings_first() {
    let src = "graph G {\nlabel=\"tab\there\";\na [fontname=\"Bookman\"];\n}\n";
    let want = "Warning: Warning: no value for width of ASCII character 9. Falling back to 0\n\
                Warning: Warning: no hard-coded metrics for 'Bookman'.  Falling back to 'Times' \
                metrics\n";
    for engine in ["dot", "neato", "circo"] {
        assert_eq!(warnings(engine, src).0, want, "{engine}");
    }
}

/// Issue #130: `size_html_img`'s error (`lib/common/htmltable.c:1088-1093`), after
/// `gvusershape_file_access`'s warning (`lib/gvc/gvusershape.c`); C exits 1.
#[test]
fn issue30_html_missing_image_error() {
    let src = "digraph { a [label=<<TABLE><TR><TD><IMG SRC=\"nope.png\"/></TD></TR></TABLE>>] }";
    let (err, ok) = warnings("dot", src);
    assert!(!ok);
    assert_eq!(
        err,
        "Warning: No such file or directory while opening nope.png\n\
         Error: No or improper image file=\"nope.png\"\n\
         in label of node a\n"
    );
}

/// Issue #130: `poly_init`'s missing-shapefile warning (`lib/common/shapes.c:2030-2032`) and
/// `epsf_init`'s (`lib/common/psusershape.c:118`).
#[test]
fn issue30_missing_shapefile_warnings() {
    let src = "digraph { a4 [shape=custom]; a5 [shape=epsf] }";
    assert_eq!(
        warnings("dot", src).0,
        "Warning: No or improper shapefile=\"<nil>\" for node \"a4\"\n\
         Warning: shapefile not set or not found for epsf node a5\n"
    );
}
