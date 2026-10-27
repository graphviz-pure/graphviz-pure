#!/usr/bin/env python3
"""The provenance report (CONTRIBUTING.md, "Name the C"): lists the functions of the Graphviz
16.1.0 sources the port covers that no `/// C: <file>:<function>` tag names, and the tags that
name a function the C sources do not define. Report-only: exits 0 unless --check is given.

    scripts/provenance-report.py SRC                 # the summary per directory and the bad tags
    scripts/provenance-report.py SRC --files         # ... and the untagged count of every C file
    scripts/provenance-report.py SRC --list          # ... and every untagged function
    scripts/provenance-report.py SRC --legacy        # also count the older prose forms (below)
    scripts/provenance-report.py SRC --check         # fail if a directory has more untagged
                                                     # functions than tests/provenance-untagged.tsv,
                                                     # or if any tag names no C function
    scripts/provenance-report.py SRC --update        # rewrite that baseline from this tree

SRC is graphviz-16.1.0.tar.xz (its sha256 is checked) or the directory it unpacks to.

The tag is a doc comment on the Rust item, `///` (or `//!` for a whole module), naming the C file
from the top of the source tree and the function:

    /// C: lib/dotgen/position.c:make_lrvn
    /// C: lib/dotgen/position.c:make_lrvn, make_aux_edge, lib/dotgen/rank.c:rank1

A bare name after a comma takes the file before it. C++ methods are named `Class::method`.

The C side: every function definition in the .c, .cpp and .h files of the covered directories
(COVERED below, less the files and directories of scripts/provenance-exclude.txt) and in the C
sections of their .y and .l files. A scanner drops comments, strings and preprocessor lines and
takes `name(...) {` at file scope or in a C++ class body (named `Class::method`). Of each `#if`
it keeps one branch; code the reference build compiles out (OFF_RE: debugging, timing, Windows)
is left out. It cannot name a function a macro defines (`NAME(args) {` with no return type,
which is also how it sees an implicit-int definition); their count is printed under the summary.
Checked against the symbols of a build's object files: it finds every function they define but
the ones bison, flex and the C++ compiler generate.

--legacy adds two columns of functions the older prose names, the candidates for a mechanical
backfill (#539). "prose": a comment block (consecutive comment lines) has the function in
backticks, and it or the Rust file's `//!` module doc mentions the function's C file, as
`lib/dir/file.c` or `file.c` (`` `make_lrvn` in `lib/dotgen/position.c` ``). "name only": the
block has the name in backticks, no mentioned file defines it, and only one covered C file does
(a guess: the name may be a Rust item's, or a C variable's elsewhere).
"""
import argparse
import hashlib
import pathlib
import re
import sys
import tarfile
import tempfile
from collections import defaultdict

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = "tests/provenance-untagged.tsv"  # from ROOT
EXCLUDE = "scripts/provenance-exclude.txt"
TARBALL_SHA256 = "0f661718f3e5268dc3bad0fb53ab646605d26db70f4080799aff1dc2f61783c3"

# The directories of the C tree whose code the port carries (in whole or in part). Directories
# not listed here (lib/ast, lib/expr, lib/gvpr, lib/sfio, lib/glcomp, lib/topfish, lib/edgepaint,
# lib/mingle, lib/rbtree and the other plugins) back tools and outputs the port does not have.
COVERED = [
    "lib/cdt", "lib/cgraph", "lib/circogen", "lib/common", "lib/dotgen", "lib/fdpgen",
    "lib/gvc", "lib/label", "lib/neatogen", "lib/ortho", "lib/osage", "lib/pack",
    "lib/pathplan", "lib/patchwork", "lib/sfdpgen", "lib/sparse", "lib/twopigen", "lib/util",
    "lib/vpsc", "lib/xdot", "plugin/core", "plugin/dot_layout", "plugin/neato_layout",
]
C_SUFFIXES = (".c", ".cpp", ".h", ".y", ".l")

# Macros the reference build leaves undefined: debugging, timing and other platforms. Code under
# `#ifdef` one of them (or `#if defined(..) && ..`) is not counted.
OFF_RE = re.compile(r"^(\w*DEBUG\w*|TIME|PQCHECK|CONMAJ_LOGGING|TEST|_WIN32|_MSC_VER|__APPLE__"
                    r"|GVDLL)$")

KEYWORDS = {"if", "while", "for", "switch", "return", "sizeof", "do", "else", "case",
            "defined", "__attribute__", "__declspec", "alignof", "_Alignof", "decltype"}


# ---- the C side ----------------------------------------------------------------------------

def strip_c(text):
    """The C text with comments, string and character literals blanked and preprocessor lines
    removed; newlines are kept so line numbers still hold. Of each `#if` only one branch is kept,
    so the braces of alternative branches do not add up: the first, or the next when the first
    is `#if 0` or needs a macro of OFF_RE."""
    out = []
    # One entry per open `#if`: True while its current branch is dropped, and whether a branch
    # has been kept yet.
    conds = []

    def emit(s):
        if any(skip for skip, _ in conds):
            out.append("\n" * s.count("\n"))
        else:
            out.append(s)

    i, n = 0, len(text)
    line_start = True
    while i < n:
        c = text[i]
        if line_start and c in " \t":
            i += 1
            continue
        if line_start and c == "#":
            # A preprocessor line, continued by a trailing backslash; a comment inside it may
            # run past its end.
            directive = []
            while i < n and text[i] != "\n":
                if text.startswith("/*", i):
                    j = text.find("*/", i + 2)
                    j = n if j < 0 else j + 2
                    out.append("\n" * text.count("\n", i, j))
                    i = j
                    continue
                if text.startswith("//", i):
                    while i < n and text[i] != "\n":
                        i += 1
                    break
                if text[i] == "\\" and i + 1 < n and text[i + 1] == "\n":
                    out.append("\n")
                    i += 2
                    continue
                directive.append(text[i])
                i += 1
            d = re.match(r"#\s*(\w+)\s*(.*)", "".join(directive).strip())
            word, arg = (d.group(1), d.group(2).strip()) if d else ("", "")
            if word in ("if", "ifdef", "ifndef"):
                dead = ((word == "if" and arg == "0") or (word == "ifdef" and OFF_RE.match(arg))
                        or (word == "if" and "||" not in arg and any(
                            OFF_RE.match(m) for m in re.findall(r"defined\s*\(?\s*(\w+)", arg))))
                conds.append((dead, not dead))
            elif word in ("elif", "else", "elifdef", "elifndef") and conds:
                kept = conds[-1][1]
                conds[-1] = (kept, True)
            elif word == "endif" and conds:
                conds.pop()
            continue
        line_start = False
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            emit(" " + "\n" * text.count("\n", i, j))
            i = j
            continue
        if c in "\"'":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            emit(c + c + "\n" * text.count("\n", i, min(j + 1, n)))
            i = j + 1
            continue
        if c == "\n":
            line_start = True
        emit(c)
        i += 1
    return "".join(out)


DEF_RE = re.compile(r"((?:[A-Za-z_]\w*\s*::\s*)*~?[A-Za-z_]\w*)\s*\(([^()]|\([^()]*\))*\)"
                    r"(\s*(const|noexcept|override|final))*\s*(:[^{;]*)?$", re.S)
CLASS_RE = re.compile(r"\b(class|struct)\s+(\w+)[^(){};]*$")
SCOPE_RE = re.compile(r"\b(namespace(\s+\w+)?|extern\s*\"\")$")


def grammar_code(source):
    """The C code of a bison or flex source: its `%{ ... %}` blocks and the part after its second
    `%%`; the other lines are blanked (the rules' actions are not functions)."""
    out, in_block, marks = [], False, 0
    for line in source.split("\n"):
        keep = in_block or marks >= 2
        if line.startswith("%{"):
            in_block, keep = True, False
        elif line.startswith("%}"):
            in_block, keep = False, False
        elif line.startswith("%%") and not in_block:
            marks += 1
            keep = False
        out.append(line if keep else "")
    return "\n".join(out)


def c_functions(source):
    """The functions `source` defines, as (name, line) pairs (`Class::method` for a C++ method
    defined in its class body), and the count of file-scope bodies opened by `NAME(...) {` with
    no return type, which are a macro defining a function the scanner cannot name."""
    text = strip_c(source)
    found, unnamed = [], 0
    # One entry per open brace: "scope" (file scope, a namespace or `extern "C"`), a class
    # name, or None (a function body or an initializer, whose inside is not looked at).
    stack = []
    head_start = 0  # where the text since the last `;`, `{` or `}` at an open scope starts

    def open_scope():
        return not stack or stack[-1] is not None

    for m in re.finditer(r"[{};]", text):
        ch, pos = m.group(), m.start()
        if ch == "{":
            if not open_scope():
                stack.append(None)
                continue
            head = text[head_start:pos]
            lead = len(head) - len(head.lstrip())
            head = head.strip()
            kind = None
            if SCOPE_RE.search(head):
                kind = "scope"
            elif CLASS_RE.search(head):
                kind = CLASS_RE.search(head).group(2)
            else:
                d = DEF_RE.search(head)
                if d and "=" not in head[:d.start()]:
                    name = re.sub(r"\s+", "", d.group(1))
                    if name.rsplit("::", 1)[-1].lstrip("~") in KEYWORDS:
                        pass
                    elif ("::" not in name and all(k == "scope" for k in stack)
                          and re.search(r"(^|\))\s*$", head[:d.start()])):
                        # No return type before the name: a macro that defines a function.
                        unnamed += 1
                    else:
                        if stack and stack[-1] != "scope":
                            name = f"{stack[-1]}::{name}"
                        line = text.count("\n", 0, head_start + lead + d.start(1)) + 1
                        found.append((name, line))
            stack.append(kind)
            head_start = pos + 1
        elif ch == "}":
            if stack:
                stack.pop()
            if open_scope():
                head_start = pos + 1
        elif open_scope():
            head_start = pos + 1
    return found, unnamed


def load_excludes(path):
    """The files and directories (prefixes, from the top of the C tree) left out of the report."""
    out = []
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.append(line)
    return out


def c_inventory(src, excludes):
    """{file: {function: line}} over the covered C files, and the count of unnamed bodies."""
    inv, unnamed = {}, 0
    for top in COVERED:
        base = src / top
        if not base.is_dir():
            sys.exit(f"provenance-report: {base} is missing; is SRC the Graphviz 16.1.0 tree?")
        for p in sorted(base.rglob("*")):
            if not p.is_file() or p.suffix not in C_SUFFIXES:
                continue
            rel = p.relative_to(src).as_posix()
            if any(rel == e or rel.startswith(e.rstrip("/") + "/") for e in excludes):
                continue
            source = p.read_text(errors="replace")
            if p.suffix in (".y", ".l"):
                source = grammar_code(source)
            funcs, u = c_functions(source)
            unnamed += u
            inv[rel] = {}
            for name, line in funcs:
                inv[rel].setdefault(name, line)
    return inv, unnamed


def open_source(arg, tmp):
    p = pathlib.Path(arg)
    if p.is_dir():
        return p
    if not p.is_file():
        sys.exit(f"provenance-report: {arg}: no such file or directory")
    digest = hashlib.sha256(p.read_bytes()).hexdigest()
    if digest != TARBALL_SHA256:
        sys.exit(f"provenance-report: {arg}: sha256 {digest}, expected {TARBALL_SHA256} "
                 "(graphviz-16.1.0.tar.xz)")
    with tarfile.open(p) as t:
        members = [m for m in t.getmembers()
                   if m.isfile() and m.name.endswith(C_SUFFIXES)
                   and not m.name.startswith("/") and ".." not in m.name.split("/")]
        t.extractall(tmp, members=members, filter="data")
    tops = [d for d in pathlib.Path(tmp).iterdir() if d.is_dir()]
    return tops[0] if len(tops) == 1 else pathlib.Path(tmp)


# ---- the Rust side -------------------------------------------------------------------------

TAG_RE = re.compile(r"^\s*//[/!]\s*C:\s*(.*)$")
REF_RE = re.compile(r"^((?:lib|plugin|cmd)/[\w./+-]+\.(?:c|cpp|h)):(~?[A-Za-z_][\w:~]*)$")
NAME_RE = re.compile(r"^~?[A-Za-z_][\w:~]*$")


def parse_tag(body):
    """The (file, function) pairs of a tag's text, or None when the text is not a tag (a quoted
    C message, say); a list holding a None marks a malformed part."""
    parts = [s.strip().rstrip(".").strip("`").strip() for s in body.split(",")]
    first = REF_RE.match(parts[0])
    if not first:
        return None
    out, file = [], None
    for part in parts:
        r = REF_RE.match(part)
        if r:
            file = r.group(1)
            out.append((file, r.group(2)))
        elif NAME_RE.match(part):
            out.append((file, part))
        else:
            out.append(None)
    return out


def rust_files(root):
    for p in sorted((root / "crates").rglob("*.rs")):
        if "/target/" not in p.as_posix():
            yield p


def scan_tags(files, root):
    """Canonical tags: [(rust path:line, file, function)], and malformed tag lines."""
    tags, malformed = [], []
    for p in files:
        rel = p.relative_to(root).as_posix()
        for no, line in enumerate(p.read_text(errors="replace").splitlines(), 1):
            m = TAG_RE.match(line)
            if not m:
                continue
            parsed = parse_tag(m.group(1).strip())
            if parsed is None:
                if re.match(r"`?(lib|plugin)/", m.group(1).strip()):
                    malformed.append((f"{rel}:{no}", line.strip()))
                continue
            for ref in parsed:
                if ref is None:
                    malformed.append((f"{rel}:{no}", line.strip()))
                else:
                    tags.append((f"{rel}:{no}", ref[0], ref[1]))
    return tags, malformed


COMMENT_RE = re.compile(r"^\s*//[/!]?(.*)$")
FILE_MENTION_RE = re.compile(r"(?<![\w/])((?:(?:lib|plugin)/[\w/]+/)?[\w-]+\.(?:c|cpp|h))\b")
BACKTICK_RE = re.compile(r"`([^`]+)`")
IDENT_RE = re.compile(r"~?[A-Za-z_][\w]*(?:::~?[A-Za-z_]\w*)*")


def scan_legacy(files, inv):
    """Functions named by prose: ((file, function) pairs whose comment block, or the Rust file's
    `//!` module doc, mentions the C file and whose block has the function in backticks), and
    (the pairs of a backticked name that only one covered C file defines, with no file named)."""
    by_base = defaultdict(list)
    for f in inv:
        by_base[f.rsplit("/", 1)[-1]].append(f)
    definers = defaultdict(set)
    for f, funcs in inv.items():
        for name in funcs:
            definers[name].add(f)

    def mentioned(text):
        out = set()
        for f in FILE_MENTION_RE.findall(text):
            if f in inv:
                out.add(f)
            elif "/" not in f:
                out.update(by_base.get(f, []))
        return out

    with_file, name_only = set(), set()
    for p in files:
        lines = p.read_text(errors="replace").splitlines()
        module = mentioned("\n".join(l for l in lines if l.lstrip().startswith("//!")))
        block = []
        for line in lines + [""]:
            m = COMMENT_RE.match(line)
            if m:
                block.append(m.group(1))
                continue
            if not block:
                continue
            text = "\n".join(block)
            block = []
            cfiles = mentioned(text) | module
            for span in BACKTICK_RE.findall(text):
                for ident in IDENT_RE.findall(span):
                    hits = [f for f in cfiles if ident in inv[f]]
                    for f in hits:
                        with_file.add((f, ident))
                    if not hits and len(definers.get(ident, ())) == 1:
                        name_only.add((next(iter(definers[ident])), ident))
    return with_file, name_only - with_file


# ---- the report ----------------------------------------------------------------------------

def directory(f):
    return f.rsplit("/", 1)[0]


def untagged_by_dir(inv, named):
    counts = defaultdict(lambda: [0, 0])  # dir -> [functions, untagged]
    for f, funcs in inv.items():
        d = directory(f)
        for name in funcs:
            counts[d][0] += 1
            if (f, name) not in named:
                counts[d][1] += 1
    return counts


def read_baseline(path):
    out = {}
    for line in path.read_text().splitlines()[1:]:
        d, n = line.split("\t")
        out[d] = int(n)
    return out


def write_baseline(counts, path):
    with open(path, "w") as f:
        f.write("directory\tuntagged\n")
        for d in sorted(counts):
            f.write(f"{d}\t{counts[d][1]}\n")


def table(rows, header):
    widths = [max(len(str(r[i])) for r in [header] + rows) for i in range(len(header))]
    def fmt(r):
        return "  ".join(str(c).ljust(w) if i == 0 else str(c).rjust(w)
                         for i, (c, w) in enumerate(zip(r, widths)))
    return "\n".join([fmt(header)] + [fmt(r) for r in rows])


def pct(a, b):
    return f"{100 * a / b:.1f}%" if b else "-"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("src", help="graphviz-16.1.0.tar.xz or the directory it unpacks to")
    ap.add_argument("--files", action="store_true", help="also print the counts of every C file")
    ap.add_argument("--list", action="store_true", help="also print every untagged function")
    ap.add_argument("--legacy", action="store_true",
                    help="also count functions named by prose (a file and a backticked name)")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true",
                      help="fail when a directory has more untagged functions than the baseline, "
                      "or a tag names no C function")
    mode.add_argument("--update", action="store_true", help="rewrite the baseline")
    args = ap.parse_args(argv)

    with tempfile.TemporaryDirectory() as tmp:
        src = open_source(args.src, tmp)
        inv, unnamed = c_inventory(src, load_excludes(ROOT / EXCLUDE))

    files = list(rust_files(ROOT))
    tags, malformed = scan_tags(files, ROOT)
    bad = [(loc, f, name) for loc, f, name in tags if f not in inv or name not in inv[f]]
    named = {(f, name) for _, f, name in tags if f in inv and name in inv[f]}
    legacy, loose = scan_legacy(files, inv) if args.legacy else (set(), set())
    legacy -= named
    loose -= named | legacy

    counts = untagged_by_dir(inv, named)
    total = sum(c[0] for c in counts.values())
    untagged = sum(c[1] for c in counts.values())

    rows = []
    for d in sorted(counts, key=lambda d: (-counts[d][1], d)):
        n, u = counts[d]
        row = [d, n, n - u, u, pct(n - u, n)]
        if args.legacy:
            row[4:4] = [sum(1 for f, _ in legacy if directory(f) == d),
                        sum(1 for f, _ in loose if directory(f) == d)]
        rows.append(row)
    total_row = ["TOTAL", total, total - untagged, untagged, pct(total - untagged, total)]
    header = ["directory", "functions", "tagged", "untagged", "coverage"]
    if args.legacy:
        header[4:4] = ["prose", "name only"]
        total_row[4:4] = [len(legacy), len(loose)]
    print(table(rows + [total_row], header))
    print(f"\n{len(tags)} tags in {len(files)} Rust files; {len(inv)} C files covered; "
          f"the scanner could not name {unnamed} bodies a macro opens.")

    if args.files:
        frows = []
        for f in inv:
            n = len(inv[f])
            u = sum((f, name) not in named for name in inv[f])
            frows.append([f, n, n - u, u, pct(n - u, n)])
        frows.sort(key=lambda r: (directory(r[0]), -r[3], r[0]))
        print("\n" + table(frows, ["file", "functions", "tagged", "untagged", "coverage"]))

    if args.list:
        print("\nUntagged functions (file:line function):")
        for f in sorted(inv):
            for name, line in sorted(inv[f].items(), key=lambda kv: kv[1]):
                if (f, name) not in named:
                    mark = ("  (prose)" if (f, name) in legacy
                            else "  (name only)" if (f, name) in loose else "")
                    print(f"  {f}:{line} {name}{mark}")

    if bad or malformed:
        print(f"\nTags that name no C function ({len(bad) + len(malformed)}):")
        for loc, f, name in bad:
            why = "no such file" if f not in inv else "no such function in it"
            print(f"  {loc}: {f}:{name} ({why})")
        for loc, line in malformed:
            print(f"  {loc}: malformed: {line}")

    if args.update:
        write_baseline(counts, ROOT / BASELINE)
        print(f"\nwrote {BASELINE}")
    elif args.check:
        base = read_baseline(ROOT / BASELINE)
        fail = [f"{d}: {counts[d][1]} untagged, baseline {base.get(d, 0)}"
                for d in sorted(counts) if counts[d][1] > base.get(d, 0)]
        if bad or malformed:
            fail.append(f"{len(bad) + len(malformed)} tags name no C function")
        if fail:
            print("\nprovenance check failed:\n  " + "\n  ".join(fail), file=sys.stderr)
            return 1
        lower = [d for d in counts if counts[d][1] < base.get(d, 0)]
        if lower:
            print(f"\n{len(lower)} directories have fewer untagged functions than the baseline; "
                  "run with --update to lock that in.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
