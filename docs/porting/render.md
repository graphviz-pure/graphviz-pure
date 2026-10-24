# Renderer port (`crates/graphviz-pure-render`)

The output formats are produced by a function-by-function port of the Graphviz
16.1.0 emit pipeline, so SVG, xdot and JSON share one traversal exactly as in C.
Node, label, record and HTML geometry come from `graphviz_pure_geom::node_size` /
`graphviz_pure_geom::html_label` (the init half of `shapes.c`, `labels.c`, `htmltable.c`);
positions come from the layout.

The layout passes its results to the renderers in typed fields, never in the
user's attributes: `NodeData::pos`/`width`/`height` and `NodeData::layout`
(`AGSEQ`, `IS_CLUST_NODE`, `ND_lw`/`ND_rw`, a placed xlabel, `ND_pos[2..odim]`),
`EdgeData::spline`/`label_pos`/`label_unplaced` and `EdgeData::layout` (xlabel,
head and tail labels with their `set` flag, port names, concentrated arrow
flags, `IGNORED`), and `Graph::bb`/`SubgraphData::bb` with `GraphLayout`
(`GD_clust` registration and order, `GD_label` position, `GD_odim`). An `xlp` or
`lp` in the input is an attribute like any other: only `neato -n` reads one, as
its layout. The writers turn the typed results into attributes as
`attach_attrs_and_arrows` does, and `ND_shape_info`'s polygon (`vertices`) is
the model's node geometry.

## Module map

| Rust | C source |
|---|---|
| `emit/model.rs` | the post-layout `Agraphinfo_t`/`Agnodeinfo_t`/`Agedgeinfo_t` state: `GD_clust` tree, `ND_coord`/`lw`/`rw`/`ht`, `ND_shape_info`, `ED_spl`, `textlabel_t`/`textspan_t`, `place_root_label`/`place_graph_label` fallbacks, `agfstout` order (head id, then sequence) |
| `emit/job.rs` | `lib/gvc/gvrender.c` (`gvrender_*`, `gvrender_resolve_color`, `gvrender_set_style`), `obj_state_t` stack (`push_obj_state`/`pop_obj_state`), job geometry from `gvRenderJobs` (`init_job_pad`/`margin`/`dpi`/`viewport`/`pagination`, `setup_page`, `emit_graph` scale) |
| `emit/emit.rs` | `lib/common/emit.c`: `emit_graph`, `emit_page`, `emit_background`, `emit_view` (outputorder), `emit_clusters`, `emit_node`, `emit_edge`, `emit_begin_edge`/`emit_end_edge`, `emit_edge_graphics` (multicolor, parallel colors, tapered, ortho rounded corners), `emit_edge_label`, `emit_attachment`, `initObjMapData`/`getObjId`/`preprocessTooltip`, layers (`init_layering`, `parse_layers`, `parse_layerselect`, `selectedLayer`, `*_in_layer`), `parseSegs`, `findStopColor`, `stripedBox`; `emit_label` from `labels.c` |
| `emit/shapes.rs` | `shapes.c` gencode: `poly_gencode`, `point_gencode`, `record_gencode`/`gen_fields`, `checkStyle`, `stylenode`, `penColor`, `findFill`, `Mcircle_hack`, `round_corners` (rounded, diagonals, cylinder), `wedgedEllipse` |
| `emit/corners.rs` | `round_corners` shape cases: note, tab, folder, box3d, component and all SBOL shapes |
| `emit/arrows.rs` | `arrows.c`: `arrow_flags`, `arrow_match_name`, `arrow_gen`, every arrow type with its miter/penwidth corrections, `Bezier` |
| `emit/html.rs` | `htmltable.c` emit half: `emit_html_label`, `emit_html_tbl`, `emit_html_cell`, `emit_html_rules`, `doBorder`, `setFill`, `emit_htextspans`, `initAnchor`/`endAnchor` |
| `emit/geom.rs` | `dotneato_closest`, `splitBSpline`, `computeoffset_p/qr`, `get_gradient_points` (`utils.c`), `taper` (`taper.c`), `ellipticWedge` (`ellipse.c`) |
| `emit/output.rs` | `output.c`: `attach_attrs_and_arrows`, `set_record_rects`, `rec_attach_bb`, `write_plain` |
| `emit/num.rs` | `gvprintdouble`, `gvprintnum`, `agxbuf_trim_zeros`, `%.Ng` |
| `emit/attrs.rs` | `late_*`, `mapbool`, `parse_style` (`strtod` is `graphviz_pure_core::strtod`) |
| `svg_engine.rs` | `plugin/core/gvrender_core_svg.c` (incl. gradients, layers, anchors, `gv_xml_escape`) |
| `xdot_engine.rs` | xdot half of `plugin/core/gvrender_core_dot.c` (per-state buffers, penwidth/textflags state, gradients) |
| `json.rs` | `plugin/core/gvrender_core_json.c`; its xdot parser (`lib/xdot/xdot.c`) is `graphviz_pure_core::xdot`, which also has `init_xdot` and `xdotBB` for `_background` (drawn by `emit_xdot` in `emit/emit.rs`) |
| `agwrite.rs` | `lib/cgraph/write.c` (`-Tdot`, `-Txdot`, `-Tcanon`) over the cgraph dictionaries recorded by the parser (`Graph::dicts`: `bindattrs`, `attrstmt`/`setattr`, `unviewsubgraphsattr`, `sym->print`, ProtoGraph `label`) |
| `lib.rs` | format dispatch, `safe_dcl`/`agxset` bookkeeping (`sync_dicts`), dot's `mark_lowclusters` edge membership |

**Invalid JSON where C writes it.** For an xdot attribute none of whose ops parses, C's
`write_xdots` warns `Could not parse xdot "…"` and writes nothing after the key, so
`-Tjson` and `-Txdot_json` print `"_tldraw_": ,`, which is not JSON (#377). The CLI writes
those bytes, as it writes `epsf_gencode`'s text (`GraphJobs::write_output_file_text`). The
library (`render_dot`, WASM's `render`) writes `""` there instead, so its result is always
one JSON document. The warning is the same in both.

`-n`, `-n1`, `-n2`, `-n3` and `-Knop`/`nop1`/`nop2` run `neato_layout`'s `Nop`
branch (`graphviz-pure-layout/src/neatogen/neatoinit.rs`: `init_nop`, `user_pos`,
`user_spline`, `nop_init_graphs`, `nop_init_edges`, `chkBB`), then neato's own
`neato_translate`, `neato_set_aspect`, `spline_edges0` (with C's `Nop` global in
`neatosplines.rs`) and `gv_postprocess`, as C does.

## Fidelity

The render corpus isolates the renderers: C lays every `tests/wild`,
`tests/fixtures` and `tests/render` graph out (`dot -Tdot`, with the port's math functions
interposed, `scripts/dot-portable-libm.sh`), then both C and
the port render that file in no-layout mode (`neato -n2`). See
`scripts/gen-render-golden.sh` and `gv-diff render`
(`crates/graphviz-pure-diff/src/render.rs`, ratchet `tests/golden/render/baseline.portable.json`,
test `crates/graphviz-pure-diff/tests/render_fidelity.rs`). The SVG generator comment is
the only line ignored. `tests/render/*.gv` are 21 feature graphs (all arrow
types, styles, shapes, gradients, HTML, records, URLs, labels, rotation,
size/ratio, layers, output order).

Byte-identical files, original 508-file corpus (before = legacy renderers with
`-n2` support, after = this port; plain formats exclude 4 nondeterministic C
references):

| format | before | after |
|---|---|---|
| svg | 36 / 508 | 507 / 508 |
| json | 0 / 472 | 472 / 472 |
| json0 | 0 / 508 | 507 / 508 |
| xdot | 2 / 508 | 507 / 508 |
| dot | 207 / 508 | 507 / 508 |
| plain | 276 / 504 | 501 / 504 |
| plain-ext | 260 / 504 | 501 / 504 |

All 21 feature graphs are identical in every format. `-Tcanon` (agwrite on the
parsed input) matches C on 493 of 508 inputs (454 before).

## Known differences

- `-Tplain` of labels changed by `htmlEntityUTF8`/`latin1ToUTF8` that need no
  quoting: `canon` in `write_plain` (`lib/common/output.c`) interns the text,
  `agstrcanon` returns it unquoted and `agstrfree` frees it before it is
  printed, so C prints whatever the freed memory holds (`gv16_graphs_Latin1`,
  `b60`; the `charset_*` Latin-1 cases, `gallery__index_7`, `Symbol`, `b34`,
  `b56` change from run to run and are skipped, `UNSTABLE` in `gv-diff`). The
  port prints the converted label; the garbage is not emulated.
- HTML cell `STYLE="dashed"` borders: C's `xdot_style` walks past the end of a
  one-element style list and prints adjacent string literals (undefined
  behaviour); not reproduced (`tests/fuzz-open/034-html-cell-dashed-style-overread`).
- `-Tpng` and `-Tjpg` draw no images (see [Images](#images)).
- The SVG generator comment names graphviz-pure.
- Gradient and HTML anchor ids are numbered per `render_dot_all` call, like C's
  process-wide statics for a single invocation.
- `-v` (#416): the layouts' `Verbose`/`GV_INFO` messages are C's, at C's sites. The time in a
  `[Graphviz] file:line:` line is UTC, since the port reads no time zone, and the timings
  (`<n> sec`) are wall time where C's `clock()` measures processor time. `printData`'s `-v2`
  lists (`lib/common/postproc.c`) print a stand-in for each `%p` heap address. On wasm32
  there is no clock: times and timings read 0. Sites under `DEBUG`, `DEBUG_PRINT` or `TIME`,
  which a release build of C does not compile, are not ported.

## Images

`image=`, `shapefile=` (`shape=custom`), HTML `<IMG>` and `shape=epsf` follow
`lib/gvc/gvusershape.c`, `lib/gvc/gvrender.c` and `plugin/core/gvloadimage_core.c`
(`graphviz_pure_core::usershape`, `emit/usershape.rs`):

- **Sizes.** `imagetype` classifies the file by its first 20 bytes and the `*_size`
  readers read the size from the header only (no image is decoded): PNG, GIF, JPEG,
  BMP, ICO, WEBP, SVG (`width`/`height` with units, else the `viewBox`), PostScript
  (`%%BoundingBox`, at 72 dpi) and PDF (`/MediaBox`). A size in pixels is converted
  at the graph's `dpi` (96 below 1) when the node or cell is sized, and at the job's
  (72 unless `dpi` is set) when it is drawn. Files are kept by name for the run, as
  `ImageDict` keeps them, until a graph has its own `imagepath`. Where C asserts on an
  SVG size outside `0..=INT_MAX` (it aborts), the port takes 0.
- **State.** What C keeps for its process (`ImageDict`, `EPSF_contents` and
  `N_EPSF_files`, the `UserShape` list, `safefile`'s one warning) the port keeps for a
  run: the CLI, or one top-level call of the library (`reset_render_state`), per
  thread. The layout leaves what it found on the graph (`GraphLayout::usershapes`: the
  graph's `imagepath` and `dpi`, the files it loaded, each epsf node's answer), and the
  renderers draw from that (`usershape::rendering`), whatever was laid out since and on
  whichever thread they run.
- **Drawing.** The plugins are those of the reference build (`GVBINDIR` with the core
  plugins only): `-Tsvg` writes an `<image>` element for PNG, GIF, JPEG and SVG files,
  `-Txdot` (and the xdot job of `-Tjson`) an `I` operation for those and PostScript,
  and the `dot` formats draw nothing. For any other pair C warns
  `No loadimage plugin for "TYPE:RENDERER"`, and then draws with the plugin the job
  selected last, which the port reproduces (each `-T` job keeps its own from graph to
  graph, `GraphJobs::render_as`).
- **`shape=epsf`.** `epsf_gencode` writes `X Y translate newpath user_shape_N` straight
  into the output file whatever the format, and so does the CLI (the library's output is
  the writer's document alone, `GraphJobs::write_output_file_text`): inside the node's
  group in `-Tsvg`, before the graph in the formats written at the end (`dot`, `xdot`,
  `plain`, `canon`, `json`), and for `-Tjson` once more from its xdot job, which C
  writes to the standard output (`GraphJobs::take_stdout`). It is not PostScript
  anywhere but C's `-Tps`, which the port does not have; the line is kept so that
  every output matches C byte for byte. The files are numbered as `user_init` keys
  `EPSF_contents`, by the pointer `safefile` returned: a later graph reads a file
  again under a new number, and with `imagepath` (or `--filepath`) a lookup finds the
  file last read under its path's text, if the search reaches it: `EPSF_contents` is
  cdt's splay tree, which places a buffer key by the text it held at insertion and
  compares it with the buffer's current text, so a search steered by other keys can
  miss it (#380). The port keeps the same tree. A closed graph's key points at freed
  memory; the port compares it as the empty string, as the reference build's allocator
  leaves it ([crashes.md](crashes.md#freed-keys-in-epsf_contents)). Where C reads freed
  memory otherwise (a path buffer that moved as it grew past its 30 characters of inline
  storage, a closed graph's string whose memory was reused) the port does not follow it. Each node's file is
  looked up once per layout, as in C.
- **Raster output.** The reference build has no `-Tpng`. The port rasterizes its SVG
  with resvg, which resolves no `<image>` (neither a file nor a `data:` URL), so a
  PNG shows no images; the warnings are the SVG job's.
- **WASM** has no filesystem: every file is missing, and is reported as C reports a
  file that does not exist (`No such file or directory while opening NAME`), so no
  node takes an image's size and no image is drawn.

## Charset

C keeps the input's bytes and converts only label text. `graphviz_pure_core::charset`
gives the port the same model with `String`s: bytes that are not part of valid
UTF-8 become the code points `U+10FF00 + byte` (`decode`), and `encode` turns
them back into bytes. The CLI decodes its input and encodes its output, and
`gv-diff` reads inputs and C references the same way, so attribute values,
names, `-Tdot`/`-Tcanon`/`-Txdot` strings, SVG titles and ids, tooltips and
URLs carry C's bytes (SVG always declares `encoding="UTF-8"`; `gv_xml_escape`
leaves bytes `>= 0x80` alone, and so does the port).

- `make_label`, HTML fallbacks and tooltips (`preprocessTooltip`) convert with
  `latin1ToUTF8` (`charset=latin1`) or `htmlEntityUTF8` (else) on the encoded
  bytes, once, where C does. `htmlEntityUTF8` reproduces C's recovery for
  invalid UTF-8 (the offending byte is taken as Latin-1, possibly leaving
  invalid UTF-8, kept as raw bytes) and its once-per-graph warning.
- Text width is measured per encoded byte, with `textspan_lut.c`'s
  once-per-process "no value for width" warnings.
- HTML labels: expat gets the graph's encoding. Latin-1 bytes are decoded to
  UTF-8, invalid UTF-8 is a fatal error, and `BIG-5` is an unknown encoding,
  so every Big5 HTML label fails. A failed HTML label is plain text with its
  unconverted source afterwards (`lp->html` cleared): `-Tplain` quotes it.
- `charset=big5`: labels convert like UTF-8; `make_simple_label` keeps the byte
  after a `0xA1..=0xFE` byte with it (so a `\` there is not an escape).
  Unknown charsets warn `Unsupported charset` and act as UTF-8.
- `-Txdot` string lengths count bytes; `-Tjson` runs `latin1ToUTF8` over every
  string of a Latin-1 graph (so label text, already UTF-8, is converted again,
  as in C) and escapes only `"`, `\`, `/`, `\b`, `\f`, `\n`, `\r`, `\t`; other
  control characters and bytes are written as is.
- The charset of the graph being processed is kept per thread
  (`charset::begin_layout`/`begin_render`, called by `layout_graph_with_options`
  and `render_graph_named`); `NodeSizeEnv::new` and `-n2` labels use it.
- The public API's root functions (`graphviz_pure::render_dot` and the others) take and
  return text, converting with `charset::from_text`/`to_text`; `graphviz_pure::decoded`
  takes decoded input (`charset::Decoded`), as the CLI does (#378).
- Not reproduced: HTML lexer error messages (`Error: not well-formed ...`,
  `unknown encoding`).

## Debugging aid

`scripts/c-probe.c` prints C's exact (`%.17g`) node coordinates, sizes and shape
vertices after `-n2`, for chasing last-bit floating-point differences.
