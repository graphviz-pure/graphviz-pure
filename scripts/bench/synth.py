#!/usr/bin/env python3
"""Writes the synthetic scaling graphs of the benchmark sets (scripts/bench/sets/*.tsv).

Every graph is a pure function of its name: a small linear congruential generator, not Python's
`random`, so the files are the same on every Python. Each file sets `layout=` to its engine.

  python3 -I scripts/bench/synth.py --out target/bench/synth

Shapes:
- dag_N (dot): N nodes, about 1.5 N edges from a node to one of the next 40 nodes, so the
  graph has long ranks and crossings to remove; every 7th node has a label of two words.
- sparse_N (neato, fdp, sfdp, circo): a random spanning tree plus N/2 extra edges.
- tree_N (twopi): a random tree, each node's parent among the previous nodes.
- components_N (dot): N independent two-node components.
- small_clusters_N (dot): N independent clusters containing one two-node component each.
- clustered_N (osage, patchwork): N nodes in nested clusters of about 20.
"""
import argparse
import pathlib


class Lcg:
    """Numerical Recipes' 32-bit LCG; deterministic and enough for graph shapes."""

    def __init__(self, seed):
        self.x = seed & 0xFFFFFFFF

    def below(self, n):
        self.x = (1664525 * self.x + 1013904223) & 0xFFFFFFFF
        return (self.x >> 8) % n


def dag(n, engine="dot"):
    r = Lcg(n)
    out = [f"digraph dag_{n} {{", f"layout={engine};", "node [shape=box];"]
    for i in range(0, n, 7):
        out.append(f'n{i} [label="node {i}"];')
    for i in range(n - 1):
        out.append(f"n{i} -> n{i + 1 + r.below(min(40, n - 1 - i))};")
        if r.below(2):
            j = i + 1 + r.below(min(40, n - 1 - i))
            out.append(f"n{i} -> n{j};")
    out.append("}")
    return "\n".join(out) + "\n"


def sparse(n, engine):
    r = Lcg(n * 31 + len(engine))
    out = [f"graph sparse_{n} {{", f"layout={engine};", "node [shape=circle, width=0.3, label=\"\"];"]
    for i in range(1, n):
        out.append(f"n{r.below(i)} -- n{i};")
    for _ in range(n // 2):
        a, b = r.below(n), r.below(n)
        if a != b:
            out.append(f"n{a} -- n{b};")
    out.append("}")
    return "\n".join(out) + "\n"


def tree(n, engine="twopi"):
    r = Lcg(n * 17)
    out = [f"graph tree_{n} {{", f"layout={engine};", "node [shape=point];"]
    for i in range(1, n):
        out.append(f"n{max(0, i - 1 - r.below(min(i, 50)))} -- n{i};")
    out.append("}")
    return "\n".join(out) + "\n"


def components(n):
    out = [f"digraph components_{n} {{", "layout=dot;"]
    for i in range(n):
        out.append(f"a{i} -> b{i};")
    out.append("}")
    return "\n".join(out) + "\n"


def small_clusters(n):
    out = [f"digraph small_clusters_{n} {{", "layout=dot;"]
    for i in range(n):
        out.append(f"subgraph cluster_{i} {{ a{i} -> b{i}; }}")
    out.append("}")
    return "\n".join(out) + "\n"


def clustered(n, engine):
    r = Lcg(n * 13)
    out = [f"graph clustered_{n} {{", f"layout={engine};"]
    nid = 0
    c = 0
    while nid < n:
        out.append(f"subgraph cluster_{c} {{")
        for _ in range(4):
            if nid >= n:
                break
            out.append(f"subgraph cluster_{c}_{nid} {{")
            for _ in range(5 + r.below(6)):
                if nid >= n:
                    break
                out.append(f"n{nid} [width={0.3 + r.below(10) / 10}];")
                nid += 1
            out.append("}")
        out.append("}")
        c += 1
    out.append("}")
    return "\n".join(out) + "\n"


GRAPHS = {}
for n in (1000, 5000, 20000):
    GRAPHS[f"dot_dag_{n}"] = lambda n=n: dag(n)
    GRAPHS[f"sfdp_sparse_{n}"] = lambda n=n: sparse(n, "sfdp")
    GRAPHS[f"twopi_tree_{n}"] = lambda n=n: tree(n)
    GRAPHS[f"osage_clustered_{n}"] = lambda n=n: clustered(n, "osage")
    GRAPHS[f"patchwork_clustered_{n}"] = lambda n=n: clustered(n, "patchwork")
for n in (1000, 3000, 10000):
    GRAPHS[f"dot_components_{n}"] = lambda n=n: components(n)
    GRAPHS[f"dot_small_clusters_{n}"] = lambda n=n: small_clusters(n)
# circo, fdp and neato grow much faster than linearly (circo_sparse_1000 takes the port 130 s
# and C more than 8 minutes), so their curves stop lower.
for n in (100, 200, 400):
    GRAPHS[f"circo_sparse_{n}"] = lambda n=n: sparse(n, "circo")
for n in (250, 500, 1000):
    GRAPHS[f"fdp_sparse_{n}"] = lambda n=n: sparse(n, "fdp")
for n in (250, 500, 1000, 2000):
    GRAPHS[f"neato_sparse_{n}"] = lambda n=n: sparse(n, "neato")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default="target/bench/synth")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    for name, make in GRAPHS.items():
        (out / f"{name}.gv").write_text(make())
    print(f"wrote {len(GRAPHS)} graphs to {out}")


if __name__ == "__main__":
    main()
