# Wave 2: current master profile

Source: e314f74f (code identical to a2596f55). Port: release musl, x86-64-v3, release PGO profile from a2596f55, frame pointers, line tables, unstripped. C: Graphviz 16.1.0, LUT plugins. Thirty serial SVG executions per capture, cycles at 1999 Hz; FP stacks for port, DWARF stacks for C. All heavy commands nice 15. Three native perf-stat runs per input.

**Limits:** these are sampled CPU estimates, not phase wall times. musl allocator/kernel stack unwinding loses the owning stage in many samples: the “other” bucket is largely allocation and page management. Do not assign it proportionally to a phase or add allocator percentages to the table. Shared-machine timing and frame-pointer code generation limit comparison with the quiet shipped build.

## Approximate phase CPU, milliseconds

| Phase | patchwork port | C | osage port | C | twopi port | C |
|---|---:|---:|---:|---:|---:|---:|
| parse | 56.7 | 55.2 | 59.9 | 41.4 | 41.0 | 36.2 |
| labels | 79.6 | 40.4 | 49.1 | 23.5 | 36.2 | 15.1 |
| layout init | 5.4 | 0.0 | 5.4 | 0.0 | 17.9 | 10.7 |
| layout/pack | 12.0 | 25.3 | 7.4 | 20.9 | 100.5 | 514.1 |
| postprocess/splines | 0.4 | 3.3 | 0.5 | 1.3 | 25.0 | 16.3 |
| model | 6.8 | 0.0 | 8.4 | 0.0 | 14.7 | 0.0 |
| render | 56.7 | 76.0 | 45.6 | 34.6 | 91.0 | 104.1 |
| output attrs | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| teardown | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 5.5 |
| other | 119.8 | 85.2 | 104.8 | 118.4 | 129.1 | 50.0 |

C Model construction is included in rendering. Labels cover node sizing, shape initialization and text/font measurement.

## Native medians

| Engine | port cycles | C cycles | port instructions | C instructions |
|---|---:|---:|---:|---:|---:|
| patchwork | 1,083,545,734 | 897,503,323 | 1,903,834,651 | 1,966,101,941 |
| osage | 900,381,226 | 745,760,227 | 1,539,670,279 | 1,566,407,744 |
| twopi | 1,464,066,546 | 2,418,165,881 | 2,170,971,629 | 3,795,743,433 |

## patchwork-port: top 15 self samples

| Symbol | self % |
|---|---:|
| `[kernel]` | 25.27 |
| `graphviz_pure_geom::node_size::node_geometry_measured` | 6.83 |
| `memcmp` | 6.42 |
| `__libc_free` | 6.34 |
| `__libc_malloc_impl` | 6.06 |
| `memcpy` | 5.87 |
| `<graphviz_pure_render::emit::job::Emitter<graphviz_pure_render::svg_engine::SvgEngine>>::emit_node` | 2.09 |
| `<hashbrown::map::HashMap<alloc::string::String, graphviz_pure_core::font::FontSizes, std::hash::random::RandomState>>::contains_key::<alloc::string::String>` | 1.83 |
| `graphviz_pure_core::cnum::fixed_into_buf` | 1.79 |
| `<graphviz_pure_parser::parser::Parser>::subgraph` | 1.52 |
| `alloc_slot` | 1.42 |
| `__lock` | 1.38 |
| `<graphviz_pure_parser::parser::Parser>::peek` | 1.25 |
| `<graphviz_pure_core::graph::Graph>::ensure_node_html` | 1.24 |
| `libm::math::sin::sin` | 1.12 |

## patchwork-C: top 15 self samples

| Symbol | self % |
|---|---:|
| `[kernel]` | 16.74 |
| `__printf_fp_buffer_1` | 7.26 |
| `dttree` | 6.64 |
| `__strlen_avx2` | 4.29 |
| `__strcmp_avx2` | 3.96 |
| `__mpn_divrem` | 3.49 |
| `hack_digit` | 2.97 |
| `__printf_buffer` | 2.91 |
| `aaglex` | 1.95 |
| `_IO_new_file_xsputn` | 1.81 |
| `__memcpy_avx_unaligned_erms` | 1.75 |
| `_int_malloc` | 1.70 |
| `__GI__IO_fwrite` | 1.63 |
| `agstrdup` | 1.54 |
| `strdict_hash` | 1.37 |

## osage-port: top 15 self samples

| Symbol | self % |
|---|---:|
| `[kernel]` | 24.83 |
| `memcmp` | 7.56 |
| `__libc_free` | 6.70 |
| `__libc_malloc_impl` | 6.60 |
| `memcpy` | 6.07 |
| `graphviz_pure_geom::node_size::node_geometry_measured` | 4.43 |
| `<graphviz_pure_parser::parser::Parser>::subgraph` | 2.15 |
| `<graphviz_pure_render::emit::job::Emitter<graphviz_pure_render::svg_engine::SvgEngine>>::emit_node` | 2.11 |
| `alloc_slot` | 1.85 |
| `<graphviz_pure_parser::parser::Parser>::peek` | 1.54 |
| `<graphviz_pure_core::graph::Graph>::ensure_node_html` | 1.48 |
| `graphviz_pure_core::cnum::fixed_into_buf` | 1.40 |
| `__lock` | 1.38 |
| `<hashbrown::map::HashMap<alloc::string::String, graphviz_pure_core::font::FontSizes, std::hash::random::RandomState>>::contains_key::<alloc::string::String>` | 1.34 |
| `<graphviz_pure_parser::lexer::Lexer>::scan` | 1.33 |

## osage-C: top 15 self samples

| Symbol | self % |
|---|---:|
| `[kernel]` | 19.35 |
| `dttree` | 7.56 |
| `__printf_fp_buffer_1` | 5.16 |
| `__strlen_avx2` | 4.96 |
| `__strcmp_avx2` | 4.19 |
| `__printf_buffer` | 2.42 |
| `aaglex` | 2.27 |
| `hack_digit` | 2.23 |
| `__mpn_divrem` | 2.22 |
| `_IO_new_file_xsputn` | 2.15 |
| `_int_malloc` | 1.90 |
| `agstrdup` | 1.71 |
| `node_set_find` | 1.61 |
| `__memcpy_avx_unaligned_erms` | 1.58 |
| `__GI__IO_fwrite` | 1.56 |

## twopi-port: top 15 self samples

| Symbol | self % |
|---|---:|
| `[kernel]` | 19.33 |
| `graphviz_pure_layout::twopigen::circle::circle_layout` | 17.31 |
| `memcpy` | 6.37 |
| `__libc_free` | 6.24 |
| `__libc_malloc_impl` | 4.79 |
| `memcmp` | 2.31 |
| `<graphviz_pure_render::emit::job::Emitter<graphviz_pure_render::svg_engine::SvgEngine>>::emit_node` | 2.29 |
| `graphviz_pure_geom::node_size::node_geometry_measured` | 1.82 |
| `<graphviz_pure_render::emit::job::Emitter<graphviz_pure_render::svg_engine::SvgEngine>>::emit_edge` | 1.76 |
| `<hashbrown::map::HashMap<alloc::string::String, graphviz_pure_core::font::FontSizes, std::hash::random::RandomState>>::contains_key::<alloc::string::String>` | 1.54 |
| `graphviz_pure_core::cnum::fixed_into_buf` | 1.30 |
| `alloc_slot` | 1.28 |
| `<graphviz_pure_render::emit::job::Job>::resolve_color` | 1.20 |
| `<graphviz_pure_core::graph::Graph>::ensure_node_html` | 1.19 |
| `__lock` | 1.18 |

## twopi-C: top 15 self samples

| Symbol | self % |
|---|---:|
| `setNStepsToLeaf` | 35.21 |
| `dttree` | 16.79 |
| `circleLayout` | 6.50 |
| `dtrestore` | 4.55 |
| `[kernel]` | 4.08 |
| `agnxtedge` | 3.27 |
| `agsubnode` | 1.69 |
| `__strlen_avx2` | 1.68 |
| `__printf_fp_buffer_1` | 1.63 |
| `__strcmp_avx2` | 1.36 |
| `__mpn_divrem` | 0.90 |
| `dtextract` | 0.89 |
| `__printf_buffer` | 0.87 |
| `_IO_new_file_xsputn` | 0.79 |
| `hack_digit` | 0.75 |

## Allocation evidence and candidate fixes

Full dhat allocation stacks were collected separately in a scratch executable using the same CLI. These builds do not use PGO. Counts include CLI setup and SVG output.

| Engine | total allocations | total allocated bytes | allocations under node geometry |
|---|---:|---:|---:|
| patchwork | 1,490,969 | 166,005,708 | 510,037 |
| osage | 1,308,586 | 151,916,029 | 346,725 |
| twopi | 1,885,628 | 213,768,658 | 419,836 |

1. Patchwork sizes every node in walk order for diagnostics, discards the geometry, then sizes again quietly in insertion order. Keep the first result and consume it at initialization (#665).
2. Label layouts, geometry and render spans repeatedly own copies of standard font names (#670). Borrow exact static alias names, retaining other spellings verbatim.
3. Osage and patchwork continue measuring identical common shapes repeatedly. Cache only pure computations with exact input-bit keys; preserve warning execution and allocation-failure semantics. Needs focused measurements before selecting.
4. Parser node creation constructs root-default attributes, then replaces them with scoped-default attributes. This creates avoidable temporary attribute allocations; attribute clones themselves are not the largest allocation site.
5. SVG object IDs and XML escaping still allocate for simple text; investigate the recorded emitter stacks.
6. Twopi’s center search relaxes nearest-leaf distances repeatedly. Integer multi-source traversal may compute the same final distances with less work; preserve node-order center tie breaks.

Raw captures and full heap traces are retained outside the source checkout. This report records the current-master snapshot; each fix has its own before/after measurements.

## Allocation-owner estimates

Counts and allocated bytes, assigned to the nearest recognized owning stage in the full dhat stack. Inline standard-library frames are excluded; these categories are separate from the sampled CPU table.

| Stage | patchwork count / bytes | osage count / bytes | twopi count / bytes |
|---|---:|---:|---:|
| parse/graph | 372,956 / 67,227,430 | 372,956 / 66,187,544 | 363,146 / 76,126,899 |
| node geometry/text | 533,382 / 18,627,728 | 370,070 / 11,267,564 | 459,836 / 16,434,684 |
| layout (other) | 107,104 / 21,881,689 | 88,033 / 17,858,622 | 264,236 / 46,946,620 |
| render/model | 477,484 / 57,749,377 | 477,484 / 56,082,879 | 772,595 / 73,073,190 |
| other | 43 / 519,484 | 43 / 519,420 | 25,815 / 1,187,265 |

## Reproducing the captures

Build the release musl CLI with `-C target-cpu=x86-64-v3 -C force-frame-pointers=yes`,
line tables and stripping disabled, including the CLI package's `strip` override. This capture
also used the release PGO profile generated from a2596f55; e314f74f changes only the ratchet
baseline relative to that commit. Keep targets and data under `/jobs`.

Record each graph with `sudo -n nice -n 15 perf record -F 1999 --call-graph fp`;
use `--call-graph dwarf,8192` for the C binary because its frame pointers do not unwind
reliably. Use `/opt/graphviz-16.1.0/bin/dot` with its LUT plugin directory and explicit
`-Kpatchwork`, `-Kosage` or `-Ktwopi`, always `-Tsvg`. Sample weights are cycles periods,
not raw sample counts. Record native `cycles:u,instructions:u,task-clock` three times
separately. The full dhat profile uses the CLI under dhat's full allocator profiler,
with 32 backtrace frames, in a scratch executable outside the checkout.
