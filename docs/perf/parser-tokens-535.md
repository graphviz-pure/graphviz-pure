# Streaming parser tokens (#535)

`Parser` now retains one lookahead token and the last scanner diagnostics, rather than all
previous tokens. Identifiers, numbers, unchanged quoted strings, HTML strings and `yytext`
borrow the source. Removing escaped quotes or escaped LF, and `+` concatenation, use owned
`Cow` text only where the spelling changes. Graph names, ports and attributes still become
owned at the existing graph-storage boundary; `AttrMap` and the stable facade are unchanged.
A directly constructed lexer with an owned NUL-normalized file buffer copies returned slices;
the CLI's `GraphStream` borrows its separately held normalized input instead.

Technique T3. C provenance: `lib/cgraph/grammar.y:aagparse` (lookahead/shift/atom/qatom),
`lib/cgraph/scan.l:yylex` (name/number/qstring/hstring rules), and `scan.l:aagerror` (last
`yytext`, line, filename and EOF state). Fidelity class **none**: scanning, grammar reductions,
warning timing, byte text, graph insertion and attribute declaration order stay the same.
No floating-point operation or dependency changes.

The new tests check borrowed text and source pointers after the scanner is dropped,
ownership only for transformed strings, owned normalization fallback, and the last error
context after a shift without speculative scanning. Existing error/warning assertions and
all workspace fidelity tests pass unchanged.

The two lookahead `expect` calls cannot fail: `peek` fills an empty lookahead
before returning, and `advance` calls `peek` before taking that same cached token.
The scanner-context `expect` in `push` is likewise guarded by initialization through
`peek` when absent. Both `unreachable!` arms in `atom` are excluded by the token-kind
checks immediately before `shift`; shifting returns that cached token unchanged, or
propagates a stack-exhaustion error before either match. `panic-sites.sh` scans the
layout and geometry crates, so these parser invariants are documented explicitly here.

Raw measurements: [parser-tokens-535.json](data/parser-tokens-535.json).

## Performance

On the 11.6 MB parser-only input, peak heap falls **73.4%** (755,115,653 →
200,845,000 B) and allocations fall **69.5%** (17,833,395 → 5,433,360).
The parse-heavy canon readback uses **17.39% fewer instructions in CI**
(158,705,292 → 131,104,138). These deterministic counts are the performance results.
The end-to-end RSS gap on the 200,000-node CLI case (989,920 KiB) remains #536's work.

Matched musl v1 release builds, before `b35ebbe2` and after. The Rust sources at the base
are identical to the supplied 1.0 release candidate (`f89e21d9`). Counts below are local
Callgrind/dhat measurements. Every full-ratchet case passes; instructions and allocations
fall on all 15 cases. The old fdp b69 +612-allocation failure (#600) is present before this
change and disappears under the parser's larger allocation reduction.

### Full ratchet

| case | instructions before → after | Δ | allocations before → after | Δ | peak heap B before → after |
| :-- | --: | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,817,839 → 131,136,378 | -17.43% | 227,582 → 149,449 | -78,133 | 5,232,504 → 3,242,338 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,451,819 → 134,712,028 | -17.08% | 210,281 → 132,148 | -78,133 | 5,232,500 → 2,470,442 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,576,764,535 → 1,553,358,160 | -1.48% | 418,625 → 387,552 | -31,073 | 13,948,024 → 13,948,024 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,768,923 → 818,582,501 | -3.78% | 1,024,852 → 934,800 | -90,052 | 29,377,345 → 29,377,345 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,544,023,520 → 1,542,435,955 | -0.10% | 678,425 → 673,828 | -4,597 | 991,897 → 991,897 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,686,832 → 5,219,205,051 | -0.14% | 321,513 → 301,806 | -19,707 | 1,407,992 → 1,221,317 |
| `tests/fixtures/unix.gv -Tpng` | 147,087,717 → 146,767,932 | -0.22% | 26,276 → 25,403 | -873 | 7,460,280 → 7,460,280 |
| `tests/fixtures/unix.gv -Tsvg` | 27,657,698 → 27,342,155 | -1.14% | 14,253 → 13,380 | -873 | 569,128 → 569,128 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,533,010,042 → 2,531,937,335 | -0.04% | 26,433 → 23,513 | -2,920 | 798,636 → 798,636 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,868,908 → 39,764,087 | -5.03% | 42,661 → 37,382 | -5,279 | 983,896 → 983,896 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,214,400 → 29,075,878 | -6.85% | 36,848 → 31,569 | -5,279 | 908,277 → 908,277 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,529,209 → 972,118,316 | -0.45% | 97,178 → 83,432 | -13,746 | 3,066,521 → 3,066,521 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,974,486 → 4,303,379,703 | -0.01% | 22,546 → 20,544 | -2,002 | 909,648 → 909,648 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,032,557,242 → 2,012,030,038 | -1.01% | 245,032 → 184,011 | -61,021 | 8,293,735 → 8,293,735 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,261,330 → 1,995,098,859 | -1.00% | 250,586 → 189,565 | -61,021 | 8,293,731 → 8,293,731 |

### Large CLI inputs

The first three cases run their normal layouts to SVG. The DAG uses `-Knop2 -Tcanon`
on a copy with `layout=dot` removed and `pos="0,0"` supplied for all 20,000 nodes. `-Tcanon`
alone still performs layout in both CLIs, so it does not isolate parsing on the large DAG.
The 200,000-node CLI row likewise has positions and uses `-Knop2 -Tcanon`.

| input | allocations before → after | Δ | peak heap B before → after |
| :-- | --: | --: | --: |
| `osage_clustered_20000` | 3,603,164 → 3,019,643 | -16.19% | 87,382,226 → 87,382,226 |
| `patchwork_clustered_20000` | 3,521,797 → 2,938,276 | -16.57% | 89,862,688 → 89,862,688 |
| `twopi_tree_20000` | 5,639,393 → 5,279,339 | -6.38% | 117,871,237 → 117,871,237 |
| `dot_dag_20000_pos` | 5,785,036 → 4,621,642 | -20.11% | 150,533,020 → 150,533,020 |

CLI peak heap is unchanged on these large cases: it is set by graph/layout/render work
after the parser has been dropped. No claim is made that removing token retention fixes
all graph-model memory use (#536).

Wall times are **noise at shared-host load 30–60**: medians of three runs, with 1.0,
after and C interleaved for each sample. No speedup is inferred from these timings.
C is stock Graphviz 16.1.0 with the pango-free plugin directory.

| input | 1.0 ms (noise) | after ms (noise) | C ms (noise) |
| :-- | --: | --: | --: |
| `osage_clustered_20000` | 1,393 | 868 | 525 |
| `patchwork_clustered_20000` | 1,161 | 1,133 | 548 |
| `twopi_tree_20000` | 2,370 | 2,346 | 2,473 |
| `dot_dag_20000_pos` | 1,876 | 1,559 | 730 |
| `parse_200000_pos` | 11,621 | 9,294 | 3,736 |

| input | 1.0 RSS KiB | after RSS KiB | C RSS KiB |
| :-- | --: | --: | --: |
| `osage_clustered_20000` | 97,652 | 95,236 | 66,636 |
| `patchwork_clustered_20000` | 100,304 | 100,444 | 69,668 |
| `twopi_tree_20000` | 123,996 | 124,484 | 45,844 |
| `dot_dag_20000_pos` | 169,540 | 168,760 | 52,816 |
| `parse_200000_pos` | 1,036,568 | 989,920 | 286,416 |

Uptime and load for each paired measurement:

- `osage_clustered_20000`: `18:15:47 up 4 days, 14:57,  2 users,  load average: 38.37, 41.88, 56.39` → `18:15:55 up 4 days, 14:57,  2 users,  load average: 37.78, 41.70, 56.25`
- `patchwork_clustered_20000`: `18:15:55 up 4 days, 14:57,  2 users,  load average: 37.78, 41.70, 56.25` → `18:16:04 up 4 days, 14:57,  2 users,  load average: 37.61, 41.54, 56.05`
- `twopi_tree_20000`: `18:16:04 up 4 days, 14:57,  2 users,  load average: 37.61, 41.54, 56.05` → `18:16:25 up 4 days, 14:58,  2 users,  load average: 35.93, 40.90, 55.53`
- `dot_dag_20000_pos`: `18:16:25 up 4 days, 14:58,  2 users,  load average: 35.93, 40.90, 55.53` → `18:16:38 up 4 days, 14:58,  2 users,  load average: 36.20, 40.70, 55.23`
- `parse_200000_pos`: `18:16:38 up 4 days, 14:58,  2 users,  load average: 36.20, 40.70, 55.23` → `18:17:52 up 4 days, 14:59,  2 users,  load average: 30.83, 38.06, 53.18`

### Parser alone on 11.6 MB

The input is 200,000 isolated nodes with unique labels and three per-node attributes,
11,600,022 bytes. A small safe Rust driver reads it into a String, calls `parse_dot`,
checks the node/edge counts and keeps the graph alive. Both drivers link the corresponding
release parser/core libraries (the pre-change sources equal 1.0). A second build uses the
existing dhat counting allocator. C's driver calls `agread`, checks `agnnodes`/`agnedges`,
and closes the graph. These measurements exclude layout and rendering; they are separate
from the shipped CLI results above.

| metric | 1.0 parser | after | Δ | C cgraph |
| :-- | --: | --: | --: | --: |
| allocations | 17,833,395 | 5,433,360 | -69.53% | — |
| bytes allocated | 1,648,905,910 | 521,964,509 | -68.34% | — |
| peak heap B | 755,115,653 | 200,845,000 | -73.40% | — |
| peak RSS KiB | 676,004 | 217,236 | -67.86% | 63,412 |
| wall ms (noise) | 3,734 | 1,936 | — | 2,108 |

Parser-only timing uptime (before / after / C):

- `18:12:23 up 4 days, 14:54,  2 users,  load average: 41.99, 44.54, 60.64`
- `18:13:15 up 4 days, 14:55,  2 users,  load average: 40.62, 43.62, 59.39`
- `18:13:15 up 4 days, 14:55,  2 users,  load average: 40.62, 43.62, 59.39`

The parser-only peak RSS falls by 67.9%.
The larger CLI experiment still peaks near 1 GiB because subsequent graph/render views
own their own data. Attribute interning remains #536's work.

## Reproduction notes

Run cargo through the build-slot wrapper. Put build outputs and all generated inputs,
Callgrind files, RSS files and renders on the data disk. The stock ratchet hardcodes
worktree targets and bench.py's RSS file uses `/tmp`; this run overrides only those paths
and reuses the shared synthetic inputs. The before/after ratchet uses identical argument
paths. The committed baseline now matches all four metrics in every row of the successful
[perf-ratchet CI artifact](https://github.com/graphviz-pure/graphviz-pure/actions/runs/40930072152)
for head `3b5500b0`. Local before/after figures remain recorded separately from CI results.

Generate the parser-only input with:

```python
with open("parse_200000.gv", "w") as out:
    out.write("digraph big_parse {\n")
    for i in range(200000):
        out.write(f'n{i:08d} [label="node {i:08d}", color=blue, shape=box];\n')
    out.write("}\n")
```

The standalone Rust driver (compiled with musl, opt-level=3, thin LTO, one codegen unit;
`--cfg heap` enables the existing dev-only allocator):

```rust
#![forbid(unsafe_code)]
#[cfg(heap)]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;
fn main() {
    #[cfg(heap)]
    let _profiler = dhat::Profiler::builder().testing().build();
    let input = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let graph = graphviz_pure_parser::parse_dot(&input).unwrap();
    println!("{} {}", graph.nodes.len(), graph.edges.len());
    #[cfg(heap)]
    {
        let s = dhat::HeapStats::get();
        eprintln!("{{\"heap_blocks\":{},\"heap_bytes\":{},\"heap_peak_bytes\":{}}}", s.total_blocks,s.total_bytes,s.max_bytes);
    }
    std::hint::black_box(&graph);
}
```

The C reference driver (linked to the installed 16.1.0 cgraph/cdt libraries):

```c
#include <graphviz/cgraph.h>
#include <stdio.h>
int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "r");
    Agraph_t *g = agread(f, NULL);
    if (!g) return 1;
    printf("%d %d\n", agnnodes(g), agnedges(g));
    agclose(g);
    fclose(f);
    return 0;
}
```

Native runs use `/usr/bin/time -f %M` for peak RSS. The dhat driver reports total blocks,
total bytes and max live bytes. No profiler is part of the shipped binary.
