"""Tests for provenance-report.py (run with `python3 -m unittest` in scripts/tests): the C
function scanner, the tag parser, and the report end to end over a small C tree and crate."""
import contextlib
import importlib.util
import io
import pathlib
import tempfile
import textwrap
import unittest

_spec = importlib.util.spec_from_file_location(
    "provenance_report", pathlib.Path(__file__).resolve().parents[1] / "provenance-report.py")
pr = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pr)


def names(source):
    return [n for n, _ in pr.c_functions(textwrap.dedent(source))[0]]


class CFunctionsTest(unittest.TestCase):
    def test_definitions_and_lines(self):
        src = """\
            #include <stdio.h>
            static int cmp(const void *a, const void *b);  /* a prototype { */
            static int
            cmp(const void *a,
                const void *b) {
              return 0;
            }
            void make_lrvn(graph_t *g) { if (g) { int x[] = {1, 2}; } }
            """
        found, unnamed = pr.c_functions(textwrap.dedent(src))
        self.assertEqual(found, [("cmp", 4), ("make_lrvn", 8)])
        self.assertEqual(unnamed, 0)

    def test_not_functions(self):
        self.assertEqual(names("""\
            static const char *s = "f() {";
            static int table[] = {1, 2, 3};
            struct point { double x, y; };
            typedef struct { int a; } anon_t;
            enum { A, B };
            static pointf (*fn)(int) = NULL;
            """), [])

    def test_only_the_first_branch_of_an_if(self):
        # Both branches open a brace; keeping both would leave the rest of the file inside f.
        self.assertEqual(names("""\
            void f(int et) {
            #ifdef ORTHO
              if (et) {
            #else
              if (!et) {
            #endif
              }
            }
            #if 0
            void dead(void) {}
            #else
            void alive(void) {}
            #endif
            void g(void) {}
            """), ["f", "alive", "g"])

    def test_macro_bodies_are_counted_not_named(self):
        found, unnamed = pr.c_functions("DEFINE_LIST(edges, edge_t *)\nLIST_FN(edges) {\n}\n"
                                        "int h(void) { return 0; }\n")
        self.assertEqual([n for n, _ in found], ["h"])
        self.assertEqual(unnamed, 1)

    def test_cpp(self):
        self.assertEqual(names("""\
            extern "C" {
            void csolve(int n) {}
            }
            namespace {
            int helper() { return 1; }
            }
            Block::Block(Variable *const v) : posn(0), weight(0) {
            }
            void Block::merge(Block *b, Constraint *c) const {}
            class Variable {
            public:
              double position() const { return 0; }
              int id;
            };
            struct Event {
              Event(int t) : type(t) {}
              int type;
            };
            """), ["csolve", "helper", "Block::Block", "Block::merge", "Variable::position",
                   "Event::Event"])

    def test_grammar_code(self):
        src = "%{\nstatic void f(void) {}\n%}\n%%\nrule: A { g(); } ;\n%%\nvoid h(void) {}\n"
        self.assertEqual(names(pr.grammar_code(src)), ["f", "h"])


class ParseTagTest(unittest.TestCase):
    def test_forms(self):
        self.assertEqual(pr.parse_tag("lib/dotgen/position.c:make_lrvn"),
                         [("lib/dotgen/position.c", "make_lrvn")])
        self.assertEqual(
            pr.parse_tag("lib/dotgen/position.c:make_lrvn, make_aux_edge, lib/dotgen/rank.c:rank1"),
            [("lib/dotgen/position.c", "make_lrvn"), ("lib/dotgen/position.c", "make_aux_edge"),
             ("lib/dotgen/rank.c", "rank1")])
        self.assertEqual(pr.parse_tag("`lib/vpsc/block.cpp:Block::merge`."),
                         [("lib/vpsc/block.cpp", "Block::merge")])

    def test_not_a_tag(self):
        self.assertIsNone(pr.parse_tag("`Error: <file>: syntax error in line 3 near '}'`"))

    def test_malformed_part(self):
        self.assertEqual(pr.parse_tag("lib/common/utils.c:Bezier, the de Casteljau step"),
                         [("lib/common/utils.c", "Bezier"), None])


class ReportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = pathlib.Path(self.tmp.name)
        self.src, self.root = t / "graphviz-16.1.0", t / "repo"
        for d in pr.COVERED:
            (self.src / d).mkdir(parents=True)
        (self.src / "lib/dotgen/position.c").write_text(
            "static void make_lrvn(graph_t *g) {}\nstatic void make_aux_edge(void) {}\n"
            "void dot_position(graph_t *g) {}\n")
        (self.src / "lib/gvc/gvevent.c").write_text("void gvevent_refresh(void) {}\n")
        (self.root / "crates/a/src").mkdir(parents=True)
        (self.root / "scripts").mkdir()
        (self.root / "tests").mkdir()
        (self.root / "scripts/provenance-exclude.txt").write_text("# viewers\nlib/gvc/gvevent.c\n")
        (self.root / "crates/a/src/lib.rs").write_text(textwrap.dedent("""\
            //! Port of `lib/dotgen/position.c`.

            /// C: lib/dotgen/position.c:make_lrvn, make_lrnv
            fn make_lrvn() {}

            /// `dot_position`: the entry point.
            fn dot_position() {}

            /// C: `Warning: ...` is what C prints here.
            fn warn() {}
            """))
        self.saved = pr.ROOT
        pr.ROOT = self.root

    def tearDown(self):
        pr.ROOT = self.saved
        self.tmp.cleanup()

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = pr.main([str(self.src), *args])
        return code, out.getvalue(), err.getvalue()

    def test_report(self):
        code, out, _ = self.run_main("--legacy", "--list")
        self.assertEqual(code, 0)
        row = next(l.split() for l in out.splitlines() if l.startswith("lib/dotgen"))
        # functions, tagged, untagged, prose, name only, coverage
        self.assertEqual(row[1:], ["3", "1", "2", "1", "0", "33.3%"])
        self.assertIn("lib/dotgen/position.c:2 make_aux_edge\n", out)
        self.assertIn("lib/dotgen/position.c:3 dot_position  (prose)", out)
        self.assertIn("crates/a/src/lib.rs:3: lib/dotgen/position.c:make_lrnv "
                      "(no such function in it)", out)
        self.assertNotIn("gvevent", out)
        self.assertNotIn("warn", out)

    def test_check_ratchet(self):
        self.assertEqual(self.run_main("--update")[0], 0)
        baseline = self.root / "tests/provenance-untagged.tsv"
        self.assertEqual(baseline.read_text(), "directory\tuntagged\nlib/dotgen\t2\n")
        code, _, err = self.run_main("--check")
        self.assertEqual(code, 1)  # the typo make_lrnv
        self.assertIn("1 tags name no C function", err)
        lib = self.root / "crates/a/src/lib.rs"
        lib.write_text(lib.read_text().replace(", make_lrnv", ""))
        self.assertEqual(self.run_main("--check")[0], 0)
        lib.write_text(lib.read_text().replace("/// C: lib/dotgen/position.c:make_lrvn\n", ""))
        code, _, err = self.run_main("--check")
        self.assertEqual(code, 1)
        self.assertIn("lib/dotgen: 3 untagged, baseline 2", err)


if __name__ == "__main__":
    unittest.main()
