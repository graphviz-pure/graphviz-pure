# Allocation stages on the 20,000-node inputs (#610)

Full Rust dhat profiles, `-Tsvg`, on master `b35ebbe2` with only #620's parser source
applied temporarily. The three stage PRs start independently from master; profiling with
#620 makes the next copies visible without repeating the token work already queued there.
`AttrMap` is unchanged. These profiles still include the sites addressed by queued #604
(twopi relaxation), #606 (osage tracing), and #617 (number formatting).

Each allocation belongs to exactly one stage: parser frames; node/label geometry frames;
LGraph construction excluding geometry; render/model frames excluding geometry; remaining
layout frames; or input/orchestration/output. The outer `reading_file` wrapper is not a
parser allocation. Inline frames are retained. Counts include reallocations as dhat does;
bytes are cumulative requested bytes, not peak. Per-stage top-five sites in both rankings
are in [the summary data](data/allocation-stages-610.json).

## osage_clustered_20000

| stage | count rank | allocations | byte rank | allocated bytes |
| :-- | --: | --: | --: | --: |
| Model::build / render (excluding geometry) | 1 | 987,674 | 1 | 74,189,601 |
| layout (excluding init/geometry) | 2 | 832,270 | 3 | 39,877,619 |
| node_geometry / make_text_label | 3 | 606,703 | 5 | 21,369,600 |
| parse -> graph | 4 | 372,951 | 2 | 59,919,942 |
| LGraph::new / new_unported (excluding geometry) | 5 | 220,003 | 4 | 23,759,115 |
| input / orchestration / output | 6 | 42 | 6 | 6,786,947 |

Largest site per stage by count and by bytes (full rankings in the data):

| stage | count leader | count | byte leader | bytes |
| :-- | :-- | --: | :-- | --: |
| layout (excluding init/geometry) | `core::diag::fmt_f` | 351,070 | `<core::graph::AttrMap>::clone` | 13,066,700 |
| parse -> graph | `<core::graph::Graph>::ensure_node_html` | 83,106 | `<core::graph::AttrMap>::insert` | 21,760,000 |
| Model::build / render (excluding geometry) | `core::cnum::fmt_f` | 173,410 | `<render::emit::model::Model>::build_quiet` | 25,710,352 |
| node_geometry / make_text_label | `core::font::storeline` | 80,000 | `core::font::storeline` | 10,240,000 |
| input / orchestration / output | `input/output/std` | 30 | `parser::lexer::reading_file` | 6,267,527 |
| LGraph::new / new_unported (excluding geometry) | `<core::graph::AttrMap>::clone` | 100,000 | `<core::graph::AttrMap>::clone` | 11,210,225 |

Copy sites selected for the stage-local work:

| site | allocations | bytes |
| :-- | --: | --: |
| whole graph snapshot | 160,495 | 20,890,592 |
| temporary NodeData clone during LGraph init | 120,000 | 11,319,115 |
| render label copying from measured geometry | 140,000 | 9,117,780 |

Whole process: 3,019,643 allocations, 225,902,824 requested bytes, 87,382,316 B peak heap.

## patchwork_clustered_20000

| stage | count rank | allocations | byte rank | allocated bytes |
| :-- | --: | --: | --: | --: |
| Model::build / render (excluding geometry) | 1 | 1,107,674 | 1 | 76,809,190 |
| node_geometry / make_text_label | 2 | 890,005 | 3 | 33,529,684 |
| parse -> graph | 3 | 372,951 | 2 | 59,933,330 |
| LGraph::new / new_unported (excluding geometry) | 4 | 300,003 | 5 | 28,879,115 |
| layout (excluding init/geometry) | 5 | 267,601 | 4 | 33,286,219 |
| input / orchestration / output | 6 | 42 | 6 | 7,813,481 |

Largest site per stage by count and by bytes (full rankings in the data):

| stage | count leader | count | byte leader | bytes |
| :-- | :-- | --: | :-- | --: |
| input / orchestration / output | `input/output/std` | 30 | `parser::lexer::reading_file` | 7,294,025 |
| layout (excluding init/geometry) | `<core::graph::AttrMap>::clone` | 150,038 | `<core::graph::AttrMap>::clone` | 13,240,632 |
| node_geometry / make_text_label | `core::font::textfont_intern` | 120,000 | `core::font::storeline` | 15,360,000 |
| Model::build / render (excluding geometry) | `core::cnum::fmt_f` | 292,423 | `<render::emit::model::Model>::build_quiet` | 25,710,352 |
| parse -> graph | `<core::graph::Graph>::ensure_node_html` | 83,106 | `<core::graph::AttrMap>::insert` | 21,760,000 |
| LGraph::new / new_unported (excluding geometry) | `<core::graph::AttrMap>::clone` | 140,000 | `<core::graph::AttrMap>::clone` | 11,370,225 |

Copy sites selected for the stage-local work:

| site | allocations | bytes |
| :-- | --: | --: |
| whole graph snapshot | 200,500 | 21,064,536 |
| render label copying from measured geometry | 140,000 | 9,117,780 |
| temporary NodeData clone during LGraph init | 160,000 | 11,479,115 |

Whole process: 2,938,276 allocations, 240,251,019 requested bytes, 89,862,778 B peak heap.

## twopi_tree_20000

| stage | count rank | allocations | byte rank | allocated bytes |
| :-- | --: | --: | --: | --: |
| layout (excluding init/geometry) | 1 | 2,426,385 | 1 | 112,860,041 |
| Model::build / render (excluding geometry) | 2 | 1,557,169 | 2 | 93,502,027 |
| node_geometry / make_text_label | 3 | 679,814 | 5 | 26,396,824 |
| parse -> graph | 4 | 363,141 | 3 | 68,611,187 |
| LGraph::new / new_unported (excluding geometry) | 5 | 252,788 | 4 | 38,928,522 |
| input / orchestration / output | 6 | 42 | 6 | 7,855,990 |

Largest site per stage by count and by bytes (full rankings in the data):

| stage | count leader | count | byte leader | bytes |
| :-- | :-- | --: | :-- | --: |
| parse -> graph | `<core::graph::AttrMap>::clone` | 120,000 | `<core::graph::Graph>::new_edge` | 24,640,032 |
| node_geometry / make_text_label | `core::font::textfont_intern` | 80,000 | `core::font::storeline` | 10,240,000 |
| layout (excluding init/geometry) | `layout::twopigen::circle::set_n_steps_to_leaf` | 2,066,072 | `layout::twopigen::circle::set_n_steps_to_leaf` | 67,864,192 |
| LGraph::new / new_unported (excluding geometry) | `<core::graph::AttrMap>::clone` | 100,000 | `<layout::common::types::LGraph>::new_unported_with` | 24,249,848 |
| input / orchestration / output | `input/output/std` | 30 | `parser::lexer::reading_file` | 7,515,645 |
| Model::build / render (excluding geometry) | `core::cnum::fmt_f` | 304,567 | `<render::emit::model::Model>::build_quiet` | 42,329,744 |

Copy sites selected for the stage-local work:

| site | allocations | bytes |
| :-- | --: | --: |
| whole graph snapshot | 143,110 | 24,470,495 |
| temporary NodeData clone during LGraph init | 120,000 | 11,328,890 |
| render label copying from measured geometry | 140,000 | 9,117,780 |

Whole process: 5,279,339 allocations, 348,154,591 requested bytes, 117,871,327 B peak heap.

## Instruction share and method

In the matching profiling build, allocator self instructions alone remain at least
33.8% on osage, 30.6% on patchwork and 36.0% on twopi, plus 3.0–4.8% in memcpy/memmove.
These lower bounds sum the named musl allocator routines and mallocng source files;
thread locks and kernel page work are excluded. They are well above the 15% stopping
threshold. Profiling-build counts are separate from the normal shipped-profile before/after
measurements in each stage PR.

Build the musl CLI's existing `heap_counts` example into a data-disk profiling target with
`RUSTFLAGS="-C force-frame-pointers=yes -C debuginfo=1 -C strip=none"`. In the scratch build,
replace its testing profiler and statistics print with a full profiler using
`.file_name(data_disk_path).trim_backtraces(Some(32)).build()`, then run the same CLI.
The profiler's default summary supplies total blocks/bytes and peak heap; its JSON supplies
per-site `tbk`/`tb` and the call stacks. This temporary source instrumentation is not shipped.
The 90 B difference from the ordinary counting example on these inputs is the longer
profiling-executable argument; allocation-site totals are unaffected.

Callgrind runs the matching ordinary profiling CLI, without dhat as allocator. Instruction
counts for the stage gains and the performance ratchet use the ordinary release binaries.
All profiling, generated data, targets and RSS scratch stay on the data disk.
