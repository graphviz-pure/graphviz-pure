#!/usr/bin/env python3
"""Writes site/src/data/fidelity.json, the data behind the site's Fidelity page.

    python3 -I scripts/site-data.py              # count the committed ratchet baselines
    python3 -I scripts/site-data.py --measure    # build gv-diff and run the full -Tsvg check again

Every build of the port computes with the same portable math library, on every platform, so
there is one set of numbers. The reference is C Graphviz 16.1.0 computing with the same math
functions (scripts/dot-portable-libm.sh interposes the port's math into C's dot).

Without --measure the numbers come from the baseline files that `cargo test --workspace`
enforces (tests/golden/*baseline*.portable.json): a file listed there is checked on every test
run and fails the build if it stops matching. With --measure the full -Tsvg check is run again
here, and the identical set is taken from the hashes it writes; the script fails if a file in the
committed baseline no longer matches, so the page never shows a number the ratchets do not back.

The same numbers against stock C on macOS (C computing with Apple's libm) are reported too, for
information: tests/golden/e2e.json and tests/golden/e2e-baseline.portable.json. A file that
differs from stock C but equals C with the same math differs only through Apple's libm.

The C reference is only needed to regenerate the goldens, not for this script: the e2e check
compares against the stored hashes of C's output.
"""

import argparse
import datetime as dt
import json
import platform
import re
import subprocess
import sys
import tempfile
from collections import Counter, OrderedDict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GOLDEN = ROOT / "tests" / "golden"
OUT = ROOT / "site" / "src" / "data" / "fidelity.json"

# Explanatory text only; every number below is computed.
E2E_CORPORA = OrderedDict(
    [
        ("main", ("tests/wild, tests/fixtures", "Graphs from the wild, the Graphviz gallery and test suite, upstream fixtures and fuzz reproducers, laid out by dot unless the file says otherwise.")),
        ("neato", ("tests/neato", "neato: stress majorization, KK, circuit and subset models, overlap removal, pinned nodes, packing.")),
        ("sfdp", ("tests/sfdp", "sfdp: multilevel force-directed layout with quadtree approximation.")),
        ("circo", ("tests/circo", "circo: circular layout of biconnected components.")),
        ("twopi", ("tests/twopi", "twopi: radial layout around a root.")),
        ("fdp", ("tests/fdp", "fdp: force-directed layout with clusters.")),
        ("osage", ("tests/osage", "osage: cluster packing.")),
        ("patchwork", ("tests/patchwork", "patchwork: squarified treemaps.")),
        ("render", ("tests/render", "Renderer edge cases: shapes, styles, arrows, colour lists, gradients and charsets.")),
    ]
)
LAYOUT_CORPORA = ["main", "neato", "sfdp", "circo", "twopi", "fdp", "osage", "patchwork"]


def load(p):
    with open(p, encoding="utf-8") as f:
        return json.load(f)


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def layout_file(corpus):
    stem = "baseline" if corpus == "main" else f"{corpus}-baseline"
    return GOLDEN / f"{stem}.portable.json"


def exact(r):
    # graphviz_pure_diff::FileReport::exact
    return (
        r.get("error") is None
        and r["missing"] == 0
        and r["bb_ok"]
        and r["nodes_ok"] == r["nodes"]
        and r["edges_ok"] == r["edges"]
        and r["clusters_ok"] == r["clusters"]
    )


def render_unstable():
    """The (format, stem) pairs gv-diff render skips, read from its source."""
    src = (ROOT / "crates" / "graphviz-pure-diff" / "src" / "render.rs").read_text(encoding="utf-8")
    block = src.split("pub const UNSTABLE", 1)[1].split("];", 1)[0]
    return re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', block)


def fuzz_rounds():
    """The rounds table of docs/porting/fuzzing.md."""
    text = (ROOT / "docs" / "porting" / "fuzzing.md").read_text(encoding="utf-8")
    sec = text.split("## Rounds", 1)[1]
    rows = [l for l in sec.splitlines() if l.startswith("|")]
    header = [c.strip() for c in rows[0].strip("|").split("|")]
    out = []
    for l in rows[2:]:
        cells = [c.strip() for c in l.strip("|").split("|")]
        if len(cells) != len(header):
            break
        row = dict(zip(header, cells))
        out.append(
            {
                "round": row["Round"],
                "seeds": row["Seeds"].replace("–", "-"),
                "cases": int(row["Cases"]),
                "engines": {k: float(row[k]) for k in header[3:-1]},
                "all": float(row["all"]),
            }
        )
    return out


def measure(*extra):
    """Runs gv-diff e2e (with EXTRA arguments, e.g. --stock); returns the hash of every output
    it renders, which are the inputs of that run's golden."""
    exe = ROOT / "target" / "release" / "gv-diff"
    with tempfile.TemporaryDirectory() as td:
        hp = Path(td) / "hashes.json"
        r = subprocess.run([str(exe), "e2e", *extra, "--hashes", str(hp)], cwd=ROOT, capture_output=True, text=True)
        sys.stderr.write(r.stdout[-1500:])
        return load(hp)


def matching(hashes, golden):
    return {k for k, h in hashes.items() if h is not None and golden.get(k) == h}


def corpus(f):
    return f.split("/")[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--measure", action="store_true", help="build gv-diff and run gv-diff e2e")
    ap.add_argument("--out", type=Path, default=OUT)
    a = ap.parse_args()

    golden = load(GOLDEN / "e2e.libm-matched.json")
    stock_golden = load(GOLDEN / "e2e.json")
    ident = set(load(GOLDEN / "e2e-baseline.libm-matched.portable.json")["identical"])
    stock_ident = set(load(GOLDEN / "e2e-baseline.portable.json")["identical"])
    if a.measure:
        subprocess.run(["cargo", "build", "--release", "-p", "graphviz-pure-diff"], cwd=ROOT, check=True)
        hashes = measure()
        got = matching(hashes, golden["files"])
        lost = ident - got
        if lost:
            sys.exit(f"{len(lost)} files in the committed baseline no longer match C, e.g. {sorted(lost)[:5]}")
        ident = got
        stock_ident = matching(measure("--stock"), stock_golden["files"])

    files = sorted(golden["files"])
    unstable = sorted(golden["unstable"])
    total = Counter(map(corpus, files))
    unstable_by = Counter(map(corpus, unstable))
    ident_by = Counter(map(corpus, ident))

    stock_files = sorted(stock_golden["files"])
    stock_total = Counter(map(corpus, stock_files))
    stock_ident_by = Counter(map(corpus, stock_ident))
    stock_differs = sorted(set(stock_files) - stock_ident)
    libm_only = [f for f in stock_differs if f in ident]
    # As gv-diff e2e --stock: inputs C with the same math has no stable output for.
    no_reference = [f for f in stock_differs if f not in golden["files"]]

    e2e = {
        "corpora": [
            {
                "name": name,
                "dirs": dirs,
                "description": desc,
                "total": total[name],
                "identical": ident_by[name],
                "unstable": unstable_by[name],
            }
            for name, (dirs, desc) in E2E_CORPORA.items()
        ],
        "total": len(files),
        "identical": len(ident),
        "unstable": unstable,
        "files": files,
        "differs": sorted(set(files) - ident),
        # Informational: stock C on macOS computes with Apple's libm, which no build follows.
        "stock": {
            "corpora": [
                {"name": name, "total": stock_total[name], "identical": stock_ident_by[name]}
                for name in E2E_CORPORA
            ],
            "total": len(stock_files),
            "identical": len(stock_ident),
            "unstable": sorted(stock_golden["unstable"]),
            "differs": stock_differs,
            "libm_only": len(libm_only),
            "no_reference": sorted(no_reference),
            "other": sorted(set(stock_differs) - set(libm_only) - set(no_reference)),
        },
    }

    layout = []
    for c in LAYOUT_CORPORA:
        rep = load(layout_file(c))["files"]
        layout.append(
            {
                "name": c,
                "total": len(rep),
                "exact": sum(1 for r in rep.values() if exact(r)),
                "inexact": sorted(k for k, r in rep.items() if not exact(r)),
            }
        )

    skipped = render_unstable()
    render_base = load(GOLDEN / "render" / "baseline.portable.json")["formats"]
    render = []
    for fmt, m in render_base.items():
        render.append(
            {
                "format": fmt,
                "total": len(m),
                "identical": sum(1 for r in m.values() if r["identical"]),
                "skipped": sum(1 for f, _ in skipped if f == fmt),
                "differs": sorted(k for k, r in m.items() if not r["identical"]),
            }
        )

    commit = git("rev-parse", "HEAD")
    data = {
        "generated": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
        "commit": commit,
        "commit_short": commit[:7],
        "commit_date": git("show", "-s", "--format=%cI", "HEAD"),
        "source": "measured" if a.measure else "baselines",
        "measured_on": f"{platform.system()} {platform.machine()}" if a.measure else None,
        "oracle": {
            "version": "16.1.0",
            "math": "the port's portable math functions, interposed into C's dot (crates/graphviz-pure-portable-libm)",
            "command": "GVBINDIR=~/src/gv-ref/lutplugins scripts/dot-portable-libm.sh -Tsvg FILE",
            "stock_command": "GVBINDIR=~/src/gv-ref/lutplugins dot -Tsvg FILE",
            "normalisation": "the line starting with '<!-- Generated by ' is dropped from both outputs",
        },
        "e2e": e2e,
        "layout": layout,
        "render": render,
        "fuzz": {"source": "docs/porting/fuzzing.md", "rounds": fuzz_rounds()},
        # Not reproducible from committed files: needs the Graphviz source tree and the C
        # oracle (scripts/dot-broad-validate.sh then gv-diff pairs). Carried from README.md.
        "c_graphs": {
            "source": "README.md (scripts/dot-broad-validate.sh + gv-diff pairs, layout geometry)",
            "inputs": 379,
            "variants": [
                {"name": "base", "flags": "", "exact": 379, "total": 379},
                {"name": "LR", "flags": "-Grankdir=LR", "exact": 379, "total": 379},
                {"name": "ortho", "flags": "-Gsplines=ortho", "exact": 379, "total": 379, "note": "C needs over 60 s on 3 inputs; given longer, they match too"},
                {"name": "concentrate", "flags": "-Gconcentrate=true", "exact": 379, "total": 379},
                {"name": "newrank", "flags": "-Gnewrank=true", "exact": 379, "total": 379},
            ],
        },
    }
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    st = e2e["stock"]
    print(
        f"wrote {a.out.relative_to(ROOT)}: e2e {e2e['identical']}/{e2e['total']} against C with the same math; "
        f"stock C on macOS {st['identical']}/{st['total']} ({st['libm_only']} libm-only) ({data['source']})"
    )


if __name__ == "__main__":
    main()
