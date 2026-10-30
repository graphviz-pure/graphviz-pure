# sfdp: pending-head integration, single-threaded v3 profile

Integration source head: `3b5a6e8595644a72f2b670caff4188666a2e9d94`. Musl release with x86-64-v3, frame pointers and line tables, unstripped; no PGO or parallel feature. `perf record -e cycles:u -F 1999 --call-graph fp`, 20 SVG processes per input. Self and phase percentages below are weighted by sampled cycles (perf sample periods), restricted to the CLI worker (`dot`). `point_distance` is inlined into the quadtree query. No lost samples reported.

Phase attribution uses the deepest recoverable marker: query, tree build, coarsening, smoothing/overlap, force/driver, render, parse. Inlined driver work and some allocator/memcpy callers cannot be separated by frame pointers, so these are sampled phase estimates, with unassigned allocator time shown separately.

| Phase | gen_rnd2000 | sparse_5000 |
| :-- | --: | --: |
| multilevel coarsening | 0.33% | 0.27% |
| spring-electrical force / driver (excluding tree) | 13.95% | 13.50% |
| quadtree build / rebuild | 11.91% | 12.35% |
| quadtree query / get_supernodes | 60.72% | 63.34% |
| smoothing / overlap | 0.06% | 0.06% |
| render | 7.76% | 6.47% |
| parse | 1.50% | 1.39% |
| allocator, broken/unattributable caller | 2.55% | 1.75% |
| other layout / setup / routing | 1.21% | 0.89% |

## rnd2000: top 15 self functions (14,095 samples)

| Function | Self |
| :-- | --: |
| `<sparse::quad_tree::QuadTree>::get_supernodes_internal` | 60.55% |
| `sfdpgen::spring_electrical::repulsive_supernodes` | 9.10% |
| `<sparse::quad_tree::QuadTree>::add_internal` | 5.81% |
| `<sparse::quad_tree::QuadTree>::child_for` | 3.49% |
| `memcpy` | 3.10% |
| `sfdpgen::spring_electrical::attractive` | 2.69% |
| `sfdpgen::spring_electrical::multilevel_spring_electrical_embedding` | 1.83% |
| `__libc_free` | 1.17% |
| `__libc_malloc_impl` | 1.00% |
| `memset` | 0.70% |
| `alloc_slot` | 0.59% |
| `<sparse::quad_tree::QuadTree>::push_cell` | 0.50% |
| `<emit::job::Emitter<svg_engine::SvgEngine>>::emit_node` | 0.42% |
| `<emit::job::Emitter<svg_engine::SvgEngine>>::emit_edge` | 0.37% |
| `<sparse::quad_tree::QuadTree>::add` | 0.34% |

## sparse5000: top 15 self functions (40,296 samples)

| Function | Self |
| :-- | --: |
| `<sparse::quad_tree::QuadTree>::get_supernodes_internal` | 63.18% |
| `sfdpgen::spring_electrical::repulsive_supernodes` | 8.84% |
| `<sparse::quad_tree::QuadTree>::add_internal` | 6.40% |
| `<sparse::quad_tree::QuadTree>::child_for` | 3.65% |
| `sfdpgen::spring_electrical::attractive` | 3.01% |
| `memcpy` | 2.55% |
| `sfdpgen::spring_electrical::multilevel_spring_electrical_embedding` | 1.40% |
| `__libc_free` | 0.81% |
| `__libc_malloc_impl` | 0.78% |
| `memset` | 0.57% |
| `memcmp` | 0.42% |
| `alloc_slot` | 0.34% |
| `<emit::job::Emitter<svg_engine::SvgEngine>>::emit_node` | 0.32% |
| `<sparse::quad_tree::QuadTree>::add` | 0.32% |
| `<emit::job::Emitter<svg_engine::SvgEngine>>::emit_edge` | 0.30% |

## Choice of work

The query consumes 60.72–63.34% of user cycles; tree build consumes 11.91–12.35%. Coarsening is 0.27–0.33%, so even eliminating it cannot achieve a 3% end-to-end gain on these inputs. Keep the force loop owned by the parallel implementation unchanged. First measure a constant-dimension 2D query, retaining the general-dimensional fallback, existing API, child traversal, leaf-list order, output order and subtraction/FMA/sqrt sequence. Then measure a separate tree-build specialization if its end-to-end gain reaches 3%.

## Profile runs

- rnd2000: 2026-10-29T22:47:53.201184+00:00 → 2026-10-29T22:48:01.762670+00:00
  - start: `22:47:53 up 4 days, 19:29,  2 users,  load average: 5.58, 4.66, 4.70`
  - end: `22:48:01 up 4 days, 19:29,  2 users,  load average: 5.87, 4.76, 4.73`
- sparse5000: 2026-10-29T22:48:01.764989+00:00 → 2026-10-29T22:48:23.777142+00:00
  - start: `22:48:01 up 4 days, 19:29,  2 users,  load average: 5.87, 4.76, 4.73`
  - end: `22:48:23 up 4 days, 19:30,  2 users,  load average: 5.62, 4.79, 4.74`
