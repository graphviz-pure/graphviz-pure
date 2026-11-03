# PostScript writer (Graphviz 16.1.0)

`-Tps` uses the core PostScript renderer. `ps_engine.rs` ports
`plugin/core/gvrender_core_ps.c`, the prolog in `plugin/core/ps.h`, and
`lib/common/psusershape.c`. The emitter now supports `init_job_pagination`,
`pagecode`, `firstpage`, `nextpage`, and `setup_page` for paging devices. Other
formats retain their existing single-page path. `PostscriptJob` retains the
page count and union bounding box across graphs and writes the trailer when
the device closes. The CLI's `-l` libraries go through `cat_libfile` semantics. EPS macro bodies
are retained with the layout, in C's dictionary traversal order, so rendering
uses the cached contents even if the source file changes.

Colors resolve directly to `HSVA_DOUBLE`: named colors use the HSV bytes of
C's `colortbl.h`, hex colors use `rgb2hsv`, and numeric HSV keeps its original
precision. Converting through the existing RGB-byte renderer colors would
lose both the table's HSV quantization and numeric HSV precision. PostScript
strings use C's UTF-8/Latin-1 classification, byte conversion, and escaping,
including its lossy explicit-Latin-1 conversion of characters outside Latin-1.

## Oracle and fonts

Verification uses the FMA-enabled C 16.1.0 oracle described in
[fuzzing.md](fuzzing.md), preloading the portable libm with `apple-libc`.
The preload exports `qsort`, `rand`, and `srand` as well as the math functions.
The pango-free plugin directory **does support PostScript**: `dot -v -Tps`
reports `Using render: ps:core` and `Using device: ps:ps:core`. Its
`textlayout` list is empty. Verbose text sizing reports
`fontname: "Times-Roman" resolved to: [internal hard-coded]`.

Text sizes therefore come from `lib/common/textspan.c:estimate_textspan_size`
and `textspan_lut.c:estimate_text_width_1pt`, already ported by the core font
module. The default Times-Roman resolves to the Times New Roman-derived
ASCII table (2048 units per em). These are Graphviz's built-in tables, with the family
lookup, HTML bold/italic flags, fallback to Times metrics for unknown families,
and fallback to a space width for unsupported characters. They are not Pango,
Fontconfig, FreeType, or the fonts a PostScript viewer eventually uses to draw
the document. Line height is `fontsize * LINESPACING`; the centerline offset
is `0.1 * fontsize`.

## Reproducing the comparisons

Build the CLI, then run from the repository root:

```sh
GVBINDIR="$oracle_plugins" LD_PRELOAD="$apple_libc_preload" \
  nice -n 15 python3 scripts/check-postscript.py \
  --oracle "$c_dot" --port "$port_binary" --out "$scratch_directory" --timeout 600
```

The script compares raw bytes, records exit statuses, saves differing outputs,
and repeats successful differing C outputs five times in total. It covers
`tests/wild`, `tests/fixtures`, `tests/render`, and all seven other engine
corpora, using four workers. The render ratchet also compares PostScript
against C's `neato -n2` rendering of the committed C layouts, independently
of layout fidelity. Its archive includes dedicated pagination/layer and
multi-graph/rotation cases. CLI tests separately pin their end-to-end bytes.

**No version or date comment lines are excluded.** C writes a static
`%%Creator: graphviz version 16.1.0 (20260904.0139)` line for this oracle;
it has no run-date comment. The writer and goldens retain that line.

## Engine-corpus result

The final byte-and-exit-status comparison covered **2,612 inputs**, including
20 fixtures and three new PostScript inputs. **2,594 matched exactly**. Error
inputs count as matches only when both exit status and all output bytes match.
No output normalization is applied.

| Requested engine | Exact / compared |
| --- | ---: |
| dot | 589 / 590 |
| neato | 442 / 445 |
| fdp | 383 / 388 |
| sfdp | 297 / 297 |
| circo | 282 / 282 |
| twopi | 281 / 289 |
| osage | 179 / 180 |
| patchwork | 141 / 141 |

The 18 differences are recorded rather than hidden by exclusions:

- **NaN sign:** `wild/fuzz_cylinder_diagonals_nan.gv`. This is also the sole
  differing render-only reference (588/589 exact). See [libm.md](libm.md).
- **C aborts/segfaults:** neato's `fuzz_ipsep_cluster_other_component.gv`,
  `fuzz_ipsep_vpsc_clusters.gv`, `fuzz_nan_pin_edge.gv`; osage's
  `fuzz_pack_degenerate_box.gv`. The port returns output instead of crashing.
- **fdp heap-dependent layouts:** `wild_gv16_graphs_b15_8c212e.gv`,
  `wild_gv16_graphs_b51_33bcb0.gv`, `wild_gv16_infosrc_plugins_1a797f.gv`.
  Each C result changes with `GLIBC_TUNABLES=glibc.malloc.tcache_count=0`;
  all three also differ in SVG. `wild_gv16_graphs_badvoro_2e8cb8.gv` is the
  already documented heap-order case in [libm.md](libm.md). It exceeded the
  sweep's 180-second C limit; a separate 600-second-limit run completed in
  182.2 seconds, and its full PS bytes were compared (different layout).
- **C varies between runs:** fdp's `wild_gv16_undirected_ngk10_4_793340.gv`
  produced two distinct outputs in five runs. The following twopi inputs
  each produced two or three distinct outputs in five runs: `fx_arrows.gv`,
  `wild_gv16_directed_oldarrows_f70da3.gv`,
  `wild_gv16_graphs_arrows_148b6b.gv`,
  `wild_gv16_graphs_arrowsize_46ccd8.gv`,
  `wild_gv16_graphs_newarrows_769825.gv`,
  `wild_gv16_linux_x86_arrows_dot_1ce23f.gv`,
  `wild_wild_graphs_arrows_a4826e.gv`,
  `wild_wild_graphs_arrowsize_ccef6e.gv`.

The unstable differences are drawing geometry, not version/date comments.

The render ratchet retains all C PS outputs, including six inputs that report
errors but still emit a document: 588/589 match. The 768-case pagination
matrix (all engines, directions, rotations, DPIs, and asymmetric margins)
also matches byte for byte with fractional page dimensions and viewport.

The musl release independently reproduces 588/589 render references and
768/768 pagination cases. The existing 17-case performance ratchet passes:
allocation counts unchanged, instruction deltas within the +2% gate, and
peak heap at most +0.17%. This change makes no speedup claim.
