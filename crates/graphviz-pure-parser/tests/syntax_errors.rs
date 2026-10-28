//! DOT syntax error messages, checked against C Graphviz 16.1.0 (issue #31).
//!
//! C's message comes from `aagerror` (`lib/cgraph/scan.l`): `syntax error in line N`, then
//! `near '<yytext>'`, or, when yytext is empty (end of input), the scanner state. N is the
//! scanner's `line_num` once bison has read the lookahead token it rejects. Each expected
//! message below is `dot`'s stderr without its `Error: <file>: ` prefix.

use graphviz_pure_parser::{GlobalAttrs, parse_dot_all_partial};

fn error_of(src: &str) -> Option<String> {
    parse_dot_all_partial(src, &GlobalAttrs::default())
        .1
        .map(|e| e.to_string())
}

#[track_caller]
fn check(src: &str, expected: &str) {
    assert_eq!(error_of(src).as_deref(), Some(expected), "input: {src:?}");
}

#[test]
fn token_errors_report_the_lookahead_and_its_line() {
    let cases = [
        // tests/fuzz-open/022-stderr-syntax-error-message/repro2.gv
        (
            "digraph G {\n a -> b\n c -> ;\n}\n",
            "syntax error in line 3 near ';'",
        ),
        ("foo G {\n}\n", "syntax error in line 1 near 'foo'"),
        (
            "strict strict digraph {}\n",
            "syntax error in line 1 near 'strict'",
        ),
        ("digraph G ; {\n}\n", "syntax error in line 1 near ';'"),
        ("digraph G\n a -> b\n}\n", "syntax error in line 2 near 'a'"),
        (
            "digraph G {\n a -> b [color=red\n}\n",
            "syntax error in line 3 near '}'",
        ),
        (
            "digraph G {\n a -> b\n \"x\"\n \"y\" = \n}\n",
            "syntax error in line 5 near '}'",
        ),
        (
            "digraph G {\n a = = b\n}\n",
            "syntax error in line 2 near '='",
        ),
        (
            "digraph G {\n a:b:c:d\n}\n",
            "syntax error in line 2 near ':'",
        ),
        (
            "digraph G {\n a -> b [label=\"x\" + ]\n}\n",
            "syntax error in line 2 near ']'",
        ),
        ("digraph G {\n edge\n}\n", "syntax error in line 3 near '}'"),
        // keywords: yytext as written
        (
            "digraph G {\n node -> b\n}\n",
            "syntax error in line 2 near '->'",
        ),
        (
            "DiGraph G {\n a -> b\n NODE [x=1]\n Node -> ;\n}\n",
            "syntax error in line 4 near '->'",
        ),
        (
            "digraph G {\n subgraph -> a\n}\n",
            "syntax error in line 2 near '->'",
        ),
        // the other graph type's edge operator is scan.l's '-' token
        (
            "graph G {\n a -> b\n}\n",
            "syntax error in line 2 near '->'",
        ),
        (
            "digraph G {\n a -- b\n}\n",
            "syntax error in line 2 near '--'",
        ),
        // characters no rule matches are tokens of their own
        (
            "digraph G {\n a $ b\n}\n",
            "syntax error in line 2 near '$'",
        ),
        (
            "digraph G {\n a - b\n}\n",
            "syntax error in line 2 near '-'",
        ),
        (
            "digraph G {\n a -. b\n}\n",
            "syntax error in line 2 near '-'",
        ),
        (
            "digraph G {\n a . b\n}\n",
            "syntax error in line 2 near '.'",
        ),
        (
            "digraph G {\n -1.5 -> .5 -> 1. ; x = -\n}\n",
            "syntax error in line 2 near '-'",
        ),
        // `@` is GRAPH_EOF_TOKEN: end of input, but yytext is "@"
        (
            "digraph G {\n a @ b\n}\n",
            "syntax error in line 2 near '@'",
        ),
        // a quoted string's yytext is its closing quote, an HTML string's its `>`; the line
        // is where the token ends
        (
            "digraph G {\n a = \"multi\nline\" ]\n}\n",
            "syntax error in line 3 near ']'",
        ),
        (
            "digraph G {\n a -> <x>\n <y> ;\n [\n}\n",
            "syntax error in line 4 near '['",
        ),
        (
            "digraph {\n a [label=\"ab\\\n\ncd\" x y]\n}\n",
            "syntax error in line 4 near 'y'",
        ),
        (
            "digraph G {\r\n a [label=\"x\\\r\ny\" z]\r\n}\r\n",
            "syntax error in line 3 near ']'",
        ),
    ];
    for (src, expected) in cases {
        check(src, expected);
    }
}

#[test]
fn end_of_input_reports_no_token() {
    check("digraph G {\n a -> b\n", "syntax error in line 3");
    check("digraph G {\n a -> b", "syntax error in line 2");
    check("digraph", "syntax error in line 1");
}

#[test]
fn end_of_input_inside_a_string_or_comment_reports_the_scanner_state() {
    // tests/fuzz-open/022-stderr-syntax-error-message/repro.gv
    check(
        "digraph G {\n a -> b\n c [label=\"abc\n\n]\n d\n",
        "syntax error in line 7 scanning a quoted string (missing endquote? longer than \
         16384?)\nString starting:\"abc\n\n]\n d\n",
    );
    // Sbuf holds the escapes as scan.l stores them
    check(
        "digraph G {\n a [label=\"\\\\x\\\"y\\q\\\nz\n",
        "syntax error in line 4 scanning a quoted string (missing endquote? longer than \
         16384?)\nString starting:\"\\\\x\"y\\qz\n",
    );
    // `%.80s`: the first 80 bytes
    check(
        &format!("digraph G {{\n a [label=\"{}\n", "x".repeat(100)),
        &format!(
            "syntax error in line 3 scanning a quoted string (missing endquote? longer than \
             16384?)\nString starting:\"{}",
            "x".repeat(80)
        ),
    );
    check(
        "digraph G {\n a [label=<<b>x</b>\n]\n}\n",
        "syntax error in line 5 scanning a HTML string (missing '>'? bad nesting? longer than \
         16384?)\nString starting:<<b>x</b>\n]\n}\n",
    );
    check(
        "digraph G {\n a -> b [label=<x<y>\n",
        "syntax error in line 3 scanning a HTML string (missing '>'? bad nesting? longer than \
         16384?)\nString starting:<x<y>\n",
    );
    check(
        "digraph G {\n a /* never closed\n b\n",
        "syntax error in line 4 scanning a /*...*/ comment (missing '*/? longer than 16384?)",
    );
}

#[test]
fn the_parser_error_wins_over_a_later_unterminated_string() {
    // C lexes on demand, so it never reaches the string.
    check(
        "digraph G {\n a -> ;\n \"unterminated\n",
        "syntax error in line 2 near ';'",
    );
}

#[test]
fn input_ending_in_a_string_or_comment_before_a_graph_is_no_error() {
    assert_eq!(error_of("/* abc\n"), None);
    let (graphs, err) = parse_dot_all_partial("digraph{a}\n\"abc\n", &GlobalAttrs::default());
    assert_eq!((graphs.len(), err), (1, None));
    // `@` ends the input
    let (graphs, err) = parse_dot_all_partial(
        "digraph G {\n a -> b\n}\n@\ndigraph H {\n c -> ;\n}\n",
        &GlobalAttrs::default(),
    );
    assert_eq!((graphs.len(), err), (1, None));
}

#[test]
fn line_numbers_restart_with_each_graph() {
    check(
        "digraph G { a }\ndigraph H { b -> ; }\n",
        "syntax error in line 1 near ';'",
    );
    check(
        "digraph G {\n a -> b; }\ndigraph H {\n x\n -> \n ]\n}\n",
        "syntax error in line 4 near ']'",
    );
    check(
        "digraph G {\n a -> b\n}\n \"x\"\n",
        "syntax error in line 1 near '\"'",
    );
}

#[test]
fn line_directives_renumber_lines() {
    check(
        "digraph G {\n#line 50\n a -> ;\n}\n",
        "syntax error in line 50 near ';'",
    );
    check(
        "digraph G {\n# 9 other.gv\n a -> ;\n}\n",
        "syntax error in line 9 near ';'",
    );
    // ignored: a line number that is not positive, and `#` that does not start a line
    check(
        "digraph G {\n# -3\n a -> ;\n}\n",
        "syntax error in line 3 near ';'",
    );
    check(
        "digraph G {\n a -> b # 7 \"z.gv\"\n c -> ;\n}\n",
        "syntax error in line 3 near ';'",
    );
    // `line_num` is an `int`: `%d` keeps the low 32 bits, and the count wraps
    check(
        "digraph G {\n# 99999999999\n a -> ;\n}\n",
        "syntax error in line 1215752191 near ';'",
    );
    check(
        "digraph G {\n# 2147483648\n a -> ;\n}\n",
        "syntax error in line 3 near ';'",
    );
    check(
        "digraph G {\n# 4294967297\n a -> ;\n}\n",
        "syntax error in line 1 near ';'",
    );
    check(
        "digraph G {\n# 2147483647\n\n a -> ;\n}\n",
        "syntax error in line -2147483648 near ';'",
    );
    // a negative overflow saturates to INT64_MIN, whose low 32 bits are 0: ignored
    check(
        "digraph G {\n# -9223372036854775808\n a -> ;\n}\n",
        "syntax error in line 3 near ';'",
    );
    check(
        "digraph G {\n# -99999999999999999999\n a -> ;\n}\n",
        "syntax error in line 3 near ';'",
    );
    // `# N "file"` also renames the input
    let (_, err) = parse_dot_all_partial(
        "digraph G {\n# 100 \"other.gv\"\n a -> ;\n}\n",
        &GlobalAttrs::default(),
    );
    match err {
        Some(err) => {
            assert_eq!(err.kind(), graphviz_pure_core::GraphvizErrorKind::Syntax);
            assert_eq!(err.message(), "syntax error in line 100 near ';'");
            assert_eq!(err.file(), Some("other.gv"));
            assert_eq!(err.line(), Some(100));
        }
        other => panic!("{other:?}"),
    }
}

/// C: `Error: <file>: syntax error in line 3 near '}'` (grammar.y `subgraph: optsubghdr
/// body`: a subgraph always has a body), issue #111.
#[test]
fn subgraph_without_body() {
    check(
        "digraph G {\n subgraph x\n}\n",
        "syntax error in line 3 near '}'",
    );
    check("digraph G { subgraph }", "syntax error in line 1 near '}'");
    check(
        "digraph G { a -> subgraph x; }",
        "syntax error in line 1 near ';'",
    );
}

/// C: `Error: <file>: syntax error in line 2 near 'y'` (grammar.y `qatom: qatom '+'
/// T_qatom`: only quoted and HTML strings concatenate), issue #80.
#[test]
fn plus_concatenates_only_quoted_strings() {
    check(
        "digraph G {\n a -> b [label=\"x\" + y]\n}\n",
        "syntax error in line 2 near 'y'",
    );
    check(
        "digraph { a [label=x + \"y\"] }",
        "syntax error in line 1 near '+'",
    );
    check(
        "digraph { a [label=\"x\" + 1] }",
        "syntax error in line 1 near '1'",
    );
    check("digraph { \"x\" + y }", "syntax error in line 1 near 'y'");
    check("digraph a + \"b\" { }", "syntax error in line 1 near '+'");
    assert_eq!(error_of("digraph { a [label=\"x\" + <y> + \"z\"] }"), None);
}

/// Issue #116: `stmt : attrstmt optsemi | compound optsemi`, so a `;` only ends a statement.
#[test]
fn a_lone_semicolon_is_an_error() {
    check("digraph { ; }", "syntax error in line 1 near ';'");
    check("digraph { a; ; }", "syntax error in line 1 near ';'");
    check("digraph { { ; }", "syntax error in line 1 near ';'");
    check(
        "digraph { a [x=y]; b = c;; }",
        "syntax error in line 1 near ';'",
    );
    assert_eq!(error_of("digraph { a; {b;}; c = d; node [x=y]; }"), None);
}

/// Issue #115: `attrstmt : attrtype optmacroname attrlist`, `optmacroname : atom '='`.
#[test]
fn an_attribute_statement_may_name_a_macro() {
    check("digraph { node a }", "syntax error in line 1 near '}'");
    check("digraph { node a b }", "syntax error in line 1 near 'b'");
    check("digraph { edge a = }", "syntax error in line 1 near '}'");
    check(
        "digraph { graph \"a\" + b = [x=y] }",
        "syntax error in line 1 near 'b'",
    );
    check(
        "digraph { node a = [x] }",
        "syntax error in line 1 near ']'",
    );
    assert_eq!(error_of("digraph { node a = [color=red] b }"), None);
}

/// `attrlist : optattr '[' optattrdefs ']'`, `attrdefs : attrassignment optseparator`: one
/// separator at most, after an assignment.
#[test]
fn attribute_lists_take_one_separator_after_each_assignment() {
    check(
        "digraph { a [x=y,,z=w] }",
        "syntax error in line 1 near ','",
    );
    check("digraph { a [,] }", "syntax error in line 1 near ','");
    check("digraph { a [x=y] = }", "syntax error in line 1 near '='");
    assert_eq!(error_of("digraph { a [x=y; z=w,][] [v=u] }"), None);
}

fn nested(levels: usize, before: &str, inner: &str) -> String {
    format!(
        "digraph G {{{}{inner}{}}}",
        format!("{before}{{").repeat(levels),
        "}".repeat(levels)
    )
}

/// Issue #118: bison's stack holds `YYMAXDEPTH` (10000) states, three for each anonymous
/// subgraph level (`optsubghdr`, `$@1`, `{`) and four with a statement before it, or for
/// each edge operator. C 16.1.0 reads 3331 nested levels and stops at 3332 with
/// `memory exhausted`, keeping the graph read so far.
#[test]
fn nesting_runs_out_of_bisons_stack_where_c_does() {
    let (graphs, err) = parse_dot_all_partial(&nested(3331, "", "a"), &GlobalAttrs::default());
    assert_eq!(err, None);
    assert_eq!(graphs[0].subgraphs.len(), 3331);
    let (graphs, err) = parse_dot_all_partial(&nested(3332, "", "a"), &GlobalAttrs::default());
    assert_eq!(
        err.map(|e| e.to_string()).as_deref(),
        Some("memory exhausted in line 1 near 'a'")
    );
    // `agread` returns the graph as read: every subgraph opened, no node yet.
    assert_eq!(graphs.len(), 1);
    assert_eq!(graphs[0].subgraphs.len(), 3332);
    assert!(graphs[0].nodes.is_empty());
    check(
        &nested(3333, "", "a"),
        "memory exhausted in line 1 near '{'",
    );
    check(&nested(3332, "", ""), "memory exhausted in line 1 near '}'");
    // `optstmtlist`'s default reduction comes before the bad token is found
    check(
        &nested(3332, "", "]"),
        "memory exhausted in line 1 near ']'",
    );
    check(&nested(3331, "", "]"), "syntax error in line 1 near ']'");
    assert_eq!(error_of(&nested(2498, "x ", "a")), None);
    check(
        &nested(2499, "x ", "a"),
        "memory exhausted in line 1 near 'a'",
    );
    check(
        &nested(2500, "x ", "a"),
        "memory exhausted in line 1 near 'x'",
    );
    let chain = |n: usize| format!("digraph G {{{}b }}", "a -> ".repeat(n));
    assert_eq!(error_of(&chain(2498)), None);
    check(&chain(2499), "memory exhausted in line 1 near '}'");
    check(&chain(2500), "memory exhausted in line 1 near '->'");
}
