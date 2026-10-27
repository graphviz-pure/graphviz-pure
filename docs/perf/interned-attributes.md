# Interned, id-indexed attributes (T4): a design note

How graphviz-pure could store attributes the way cgraph does (one value slot per declared
attribute, holding a shared string) without a global dictionary or a reference count. The note
also covers how the output writers keep their attribute order, and how typed values can be parsed
once without moving any of C's warnings. This is technique T4 of the
[post-1.0 plan](../roadmap/post-1.0.md) (#514), issue #536. It is a design for 1.x: it needs the
1.0 decisions D1 (the facade's `Graph` is opaque, #520) and D3 (no serde on core's graph types),
and it starts no 1.0 API work. Every step it proposes must be byte-identical and pass every
fidelity ratchet.

Numbers are for `master` at 0.2.2 (`f9c2fd1d`), measured on the shared 28-thread Ryzen 9 5900XT
of [baseline.md](baseline.md) (#518). [Reproducing](#reproducing) has the method.

## Contents

- [Summary](#summary)
- [Today's representation](#todays-representation)
- [Measurements](#measurements)
- [cgraph's model](#cgraphs-model)
- [The proposed representation](#the-proposed-representation)
- [Writer iteration order](#writer-iteration-order)
- [Typed values parsed once, warnings unmoved](#typed-values-parsed-once-warnings-unmoved)
- [Migration plan](#migration-plan)
- [Expected gains](#expected-gains)
- [Risks](#risks)
- [Open questions for the maintainer](#open-questions-for-the-maintainer)
- [Side finding: `agwrite` is quadratic in nodes × edges](#side-finding-agwrite-is-quadratic-in-nodes--edges)
- [Reproducing](#reproducing)

## Summary

- **Attributes are most of what a parsed graph holds.** On a 21 MB synthetic graph (100k nodes,
  200k edges, 4 attributes per object), the parsed `Graph` holds 309 MB, and 175 MB of that is
  `AttrMap`s: about 585 bytes per object, or 120–190 bytes per attribute. Across the 2,656
  corpus inputs that parse as UTF-8, the `AttrMap`s are 53% of the graphs' heap. C's `gc` reads the
  same 21 MB file in 91 MB of RSS for the whole process.
- **An interned, id-indexed store is 9–10× smaller.** A standalone prototype (a per-graph
  interner plus one `u32` per declared attribute per object, in safe Rust with no dependency)
  holds the same attributes in 19.2 MB (big), 7.7 MB (chain) and 5.3 MB (whole corpus, against
  52.4 MB).
- **Dense, as cgraph does it, rather than sparse.** One slot per declared attribute (`Box<[Val]>`
  indexed by `AttrId`) and a sorted list of only the non-default values take the same memory: 5.3
  MB against 5.1 MB over the corpus, with a median per-file ratio of 1.21 and dense smaller on both
  large inputs. Dense keeps cgraph's semantics exactly (every object has a value for every
  declared attribute) and makes a read an index operation. It is the recommendation.
- **A read by id costs 3–11 ns, against 41–51 ns** for `AttrMap::get` by name. Resolving the
  name to an id at every read would cost another 13–15 ns, so hot reads take ids resolved once,
  like C's `N_*`/`E_*` globals. `late_double(agget(n, "fontsize"))` costs 160–168 ns, so about 110 ns
  of it is the parse. That is the case for parsing typed values once.
- **Writer order is name order, and `sym->id` order also matters.** Every writer that walks
  attributes (`agwrite` for `-Tdot`/`-Txdot`/`-Tcanon`, the canon fallback, `-Tjson`) walks them
  by name (`strcmp`), because cgraph's dictionaries are ordered trees keyed by name. The ids
  must also stay C's ids: dot's aux graph reads main-graph symbol ids on aux objects (the
  `penwidth` quirk, #84), so id order is observable.
- **C's `late_*` never warn.** `late_int`, `late_double`, `late_string`, `late_nnstring`,
  `late_bool` and `mapbool` are pure (`lib/common/utils.c:40–103`). Their results can be
  memoised per interned string with no effect on stderr. The parses that do warn (arrow types,
  shapes, colours, `style`, `overlap`, `mode`, `model`, `charset`, …) keep their call sites. They
  can memoise the pure part and replay the diagnostics at each call, so the warnings come at the
  same points in the same order. One gap: no ratchet compares stderr over the corpus today.
  Adding one is a precondition of the typed-value step.
- **Time** is a secondary gain. Attribute work is at most 2–3% of samples on attribute-heavy dot
  runs (chain, 5k nodes) and 0% on layout-bound ones (b29). Memory is the main gain: fewer
  allocations, and graph clones (`attach_attrs` clones the whole graph per output) become
  copies of `u32` arrays.

## Today's representation

`graphviz_pure_core::AttrMap` (`crates/graphviz-pure-core/src/graph.rs:22`) is

```rust
pub struct AttrMap {
    values: BTreeMap<String, String>,
    html: BTreeSet<String>,   // keys whose value is an HTML string
}
```

with `Deref<Target = BTreeMap<String, String>>`. So every read site uses the `BTreeMap` API
directly (`get`, `contains_key`, `keys`, `iter`), and iteration is in name order.

- **Per object.** `NodeData.attrs`, `EdgeData.attrs`, and per graph and subgraph `graph_attrs`,
  `node_defaults` and `edge_defaults`. A node is created with a clone of the current node
  defaults (`parser.rs:1083`) and then gets its own values. So, as in cgraph, an object keeps
  the defaults that were in force when it was made. Each key and each value is its own heap
  `String`, and an HTML value also copies its key into `html`.
- **Declarations.** `Graph.dicts: Vec<AttrDicts>` (index 0 the root, `i + 1` subgraph `i`), each
  holding three `BTreeMap<String, AttrSym>`. `AttrSym` (`graph.rs:595`) has `defval`, `print`,
  `id` (C's `sym->id`), `html`, `html_defval` and `proto`. Writers use these dictionaries to
  tell local declarations from inherited ones. The parser fills them, and the renderers'
  `sync_dicts` (`render/src/lib.rs:163`) adds what the layout declared (`safe_dcl`).
- **Absence is a state.** cgraph gives every object a value for every declared attribute
  (`agmakeattrs`, `attr.c:165`). An `AttrMap` can lack a declared key, which happens for objects
  that layouts make and for attributes declared after an object was made. The port reproduces
  what C reads there case by case (`dotsplines.rs:293`, "cgraph gave them as \"\""). The first
  migration step must keep that state representable, and
  [open question 2](#open-questions-for-the-maintainer) asks whether to remove it later.

Read sites, by `grep -F` over `crates/*/src`. The columns are core, parser, geom, layout,
render and the facade:

| pattern | core | parser | geom | layout | render | facade | total |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `agget(` | 2 | 1 | 0 | 48 | 3 | 1 | 55 |
| `late_double(` | 4 | 0 | 20 | 28 | 6 | 0 | 58 |
| `late_int(` | 7 | 0 | 5 | 28 | 6 | 0 | 46 |
| `mapbool(` | 2 | 0 | 6 | 31 | 8 | 0 | 47 |
| `late_nnstring(` | 0 | 0 | 10 | 0 | 13 | 0 | 23 |
| `late_string(` | 0 | 0 | 0 | 0 | 9 | 0 | 9 |
| `late_bool(` / `agxget(` | 0 | 0 | 0 | 2 / 2 | 0 | 0 | 4 |
| `.attrs.get(` | 1 | 16 | 1 | 19 | 8 | 0 | 45 |
| `graph_attrs.get(` | 3 | 6 | 0 | 11 | 6 | 2 | 28 |
| `.attrs.insert(` / `.attrs.set(` | 0 | 2 | 2 | 7 | 0 | 0 | 11 |

There are six separate `late_double` helpers (`core::cnum`, `geom::node_size`,
`layout::common::utils`, `layout::common::shapes`, `layout::dotgen::utils`, `render::emit::attrs`),
and every one of them looks the name up and parses on every call. The table counts the
helpers' definitions and their tests along with the calls. A handful of sites walk an object's
attributes rather than reading one (`json.rs:521–531`, `render/lib.rs:170–210`,
`parser/canon.rs:411`, `agwrite.rs` throughout, `cccomps.rs:372`, `node_size.rs:1781`).

## Measurements

Heap figures come from a counting `#[global_allocator]` in a scratch crate outside the repo, the
same method as the baseline's. "Holds" is the drop in live heap when that part of the parsed
graph is cleared. RSS is `/usr/bin/time -f %M`. `big` and `chain` come from the generator under
[Reproducing](#reproducing). `big` is 21 MB, not the baseline's 12 MB, because every edge here
has 3–5 attributes.

**What a parsed graph holds:**

| input | parse peak live | graph holds | node `AttrMap`s | edge `AttrMap`s | `node_lookup` | `Vec<NodeData>`/`Vec<EdgeData>` and the rest | C `gc` RSS (whole process) |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| big (100k nodes, 200k edges) | 1,415 MB | 309.4 MB | 59.0 MB | 116.2 MB | 5.7 MB | 127.9 MB | 90.8 MB |
| chain (50k nodes, 50k edges) | 377 MB | 101.0 MB | 30.1 MB | 28.4 MB | 2.9 MB | 39.3 MB | 36.8 MB |
| b29 (110 nodes, 386 edges) | 2.7 MB | 0.5 MB | 0.1 MB | 0.2 MB | 0.0 MB | 0.2 MB | — |

The gap between the parse peak and what the graph holds (1.1 GB on big) is the token vector of
T3 (#535), not attributes. The "rest" is mostly the `NodeData` (224 B) and `EdgeData` (376 B)
vectors after capacity doubling, which is a separate item.

**Per object, big:** 590 B per node and 581 B per edge for 4 attributes each. cgraph's
equivalent is an `Agattr_t` record (32 B, a 48-byte malloc chunk) plus a `char *` array with one
slot per declared attribute (5 × 8 = 40 B, also a 48-byte chunk): about 96 B, with every value
string shared in the refstr dictionary.

**Prototype stores** (interner and per-object arrays, built from the parsed graph):

| input | `AttrMap`s today | distinct strings | dense `Box<[Val]>` + interner | sparse `Box<[(u32, Val)]>` + interner |
| :--- | ---: | ---: | ---: | ---: |
| big | 175.2 MB | 266,689 | **19.2 MB** | 22.8 MB |
| chain | 58.5 MB | 150,004 | **7.7 MB** | 8.1 MB |
| corpus (2,656 files) | 52.4 MB (53% of the graphs' 98.6 MB) | — | **5.3 MB** | 5.1 MB |

Per file, the dense/sparse ratio has a median of 1.21 (p10 0.89, p90 1.71, extremes 0.42 and
9.4, the latter on tiny files). Dense is smaller in 533 of the 2,656 files. Ten corpus files
are Latin-1, which the probe could not read as UTF-8, and were skipped.

**Read cost** (big, 20 passes over 6 node attributes; chain in parentheses):

| read | ns per read |
| :--- | ---: |
| `AttrMap::get(name)` | 50.3–50.7 (41.0) |
| dense store by `AttrId`, value as `&str` | 10.0–11.4 (3.3) |
| name → `AttrId` in a `BTreeMap` dictionary (per read) | 13.2–14.2 (14.6) |
| `late_double(attrs.get("fontsize"), 14, 1)` | 167.4–167.8 (159.9) |

**End to end** (one run each, ±10%):

| run | port | C 16.1.0 |
| :--- | ---: | ---: |
| dot `-Tsvg` chain5k (5k nodes) | 0.35 s, 65 MB | 0.24 s, 43 MB |
| dot `-Tdot` chain5k | 0.43 s, 76 MB | 0.24 s, 43 MB |
| dot `-Tcanon` chain (50k nodes) | 6.39 s, 586 MB | 2.12 s, 396 MB |
| sfdp `-Tsvg` mid (5k nodes, 10k edges) | 9.91 s, 78 MB | 5.51 s, 27 MB |
| dot `-Tsvg` b29 | 0.91 s, 20 MB | 1.72 s, 16 MB |

**Time share** (`sudo perf record`, self time): frames that name `AttrMap` or `BTreeMap` clones and
drops are 1.8% of samples (dot `-Tdot` chain5k), 1.1% (`-Tsvg` chain5k) and 0.02% (b29).
`strtod`, `late_*` and `map_bool` are 0.16%, 0.14% and 0%. Lookups are inlined, so they aren't
counted here; with stacks, 2–3% of chain5k samples have an `AttrMap` or `BTreeMap` frame. malloc
and free are 8–10% of chain5k, and attribute maps are one of their sources.

## cgraph's model

- **Dictionaries.** Each graph has an `Agdatadict_t` with three `Dict_t`s (graph, node, edge) of
  `Agsym_t { name, defval, id, kind, fixed, print }`. The discipline (`AgDataDictDisc`,
  `attr.c:34`) keys on `name` with the default comparator, so a dictionary is an ordered tree
  in `strcmp` order. A subgraph's dictionary is a *view* onto its parent's. A local symbol
  shadows the parent's, and it has the parent's `id` (`agnewsym(..., psym->id, ...)`,
  `attr.c:249`).
- **Ids.** `sym->id` is the declaration order in the root's dictionary of that kind. ProtoGraph
  symbols (the node `label`, `-G`/`-N`/`-E`) come first, which is why `AttrSym.proto` exists.
- **Values.** Each object has an `Agattr_t` record whose `str` array is indexed by `sym->id`
  (`agxget`: `data->str[sym->id]`, `attr.c:454`). The array has `topdictsize` slots, one per
  root symbol. A new declaration grows every existing object's array (`gv_recalloc`,
  `attr.c:214`) and fills the slot with the default.
- **Strings.** Each slot points into the graph's refstr dictionary (`agstrdup`/`agstrfree`): one
  copy per distinct string, with a reference count and an HTML bit on the string itself
  (`aghtmlstr`). "Is this value the default?" is a pointer comparison,
  `data->str[sym->id] != sym->defval` (`write.c:427,494`). Equal strings are the same pointer,
  so a value explicitly set to its default counts as the default unless the HTML bit differs.
- **The global.** The refstr dictionary is per root graph, but a NULL graph falls back to one
  process-wide dictionary with a bitfield refcount that can overflow. That is GV-BUG-010
  ([ledger](../c-bugs-ledger.md)), "not applicable" in the port because it has no interner.

## The proposed representation

Everything below is owned by one root `Graph`. There is no global, no `Rc`, no refcount and no
`unsafe`, so GV-BUG-010 stays "not applicable": a symbol is an index into one graph's table and
cannot outlive it or be shared across threads by accident.

```rust
/// A string interned in one graph. Never freed before the graph is (see Risks: refstr).
pub struct Sym(u32);
/// A value slot: a Sym and the HTML bit (aghtmlstr), packed: sym << 1 | html.
/// Val::ABSENT = u32::MAX keeps today's "no value" state (migration step 3).
pub struct Val(u32);
/// An attribute of one kind: cgraph's sym->id, the same numbers C gives.
pub struct AttrId(u32);

pub struct Interner {      // per root Graph
    buf: String,           // every distinct string, back to back
    spans: Vec<(u32, u32)>,// Sym -> (start, len) in buf
    table: Vec<u32>,       // open addressing, Sym + 1 (0 = empty)
}

pub struct Schema {        // per root Graph and per kind: cgraph's root Dict_t
    syms: Vec<SymInfo>,    // by AttrId: name: Sym, name_html, defval: Val, print, proto
    by_name: Vec<AttrId>,  // sorted by name bytes (strcmp): the writers' order
}
pub struct LocalDict {     // per subgraph and kind: a view's local symbols
    local: Vec<(AttrId, Val, bool /* print */)>, // sorted by AttrId; few entries
}

// Per object: one slot per declared attribute, as cgraph's Agattr_t.str.
pub struct Attrs(Box<[Val]>);
```

- **Interner.** One `String` arena with spans and a hand-written open-addressing table of
  `u32`. That avoids storing each string twice (`HashMap<Box<str>, Sym>` plus `Vec<Box<str>>`)
  and needs no `raw_entry` and no dependency. The prototype's hash is a 15-line multiply-rotate
  (FxHash's constant). Whether `rustc-hash` beats it is not worth measuring until the
  interner shows up in a profile. It doesn't in the prototype: interning 266k strings while
  building the store is part of the 19.2 MB measurement, and it takes milliseconds.
- **HTML-ness is a bit on the value** (`Val`'s low bit), as in cgraph. The `html: BTreeSet`
  goes away. An attribute *name* first spelled as HTML (`<zzz>=1`) keeps `name_html` on its
  `SymInfo`, as `AttrSym.html` does today. "Is this the default?" becomes `val == defval`,
  which is exactly cgraph's pointer comparison, HTML bit included.
- **Dense rather than sparse.** The measurements give no memory reason to prefer a sparse list,
  and dense has three advantages: a read is `attrs.0[id]`; the semantics are cgraph's (a new
  declaration grows every object's slice, which is what `attr.c:214` does, O(objects) per
  declaration); and "explicitly set to the default" needs no separate state. A sparse store
  would have to fill in the old default on every existing object whenever a default changes
  after objects exist (`a; node [color=red]; b`), which is easy to get wrong.
- **Names → ids once.** A per-graph `Syms` struct resolved at layout init (`label`, `shape`,
  `width`, … for nodes; `weight`, `minlen`, … for edges) plays the role of C's `N_*`/`E_*`
  globals, without a global: it hangs off the layout's context. Cold reads can keep using names
  (`g.node_attr(n, "samplepoints")`), which costs one binary search over `by_name`.
- **Reading API.** Functions that take `&AttrMap` today (`arrow_flags`, the `late_*` helpers,
  `node_geometry`) take an `AttrsRef<'g>`, a `Copy` view of (`&Interner`, `&Schema`, `&Attrs`)
  with the same read methods. Its `iter()` yields `(&str, &str, html)` in `by_name` order, which
  is what `for (k, v) in &attrs` gives today. Writes go through the graph
  (`g.set_node_attr(n, key, value, html)`), because a write may intern and may declare.
- **Clones are cheap.** `Graph::clone` (one per output in `attach_attrs`, `output.rs:83`,
  and more in layouts) copies the interner once and memcpys each `Box<[Val]>`, instead of
  allocating two `String`s per attribute per object.
- **`root_holds_plain` and refcounts.** Hotspot 2 of the baseline (#541) asks C's question: is
  this string in the root's refstr dictionary *right now*? cgraph frees a string when its last
  reference goes, so an interner that never frees answers "yes" too often. The writer must
  keep its own answer (#541's "set of the graph's strings once per output", computed from the
  live slots), or the interner must track live counts per `Sym`. [Open question
  3](#open-questions-for-the-maintainer).

## Writer iteration order

Attribute order is output. Every writer that walks attributes, and the order it must keep:

| writer | outputs | what it walks | order C uses | port today |
| :--- | :--- | :--- | :--- | :--- |
| `agwrite` (`render/agwrite.rs`) | `-Tdot`, `-Txdot`, `-Tcanon` (graphs with dictionaries) | header: each graph's local `graph`, `node`, `edge` dictionaries (`write_dict`); objects: the dictionary view from the object's graph up to the root (`write_nondefault`), skipping `tailport`/`headport` on edges, which go out as `:port`; an edge's `key=` first | `dtfirst`/`dtnext` on a name-keyed `Dttree`: `strcmp` byte order; a view yields the nearest definition of each name in that order | `BTreeMap<String, AttrSym>` per dictionary, `view_all` merges into a `BTreeMap<&str, _>`: byte order |
| canon fallback (`parser/canon.rs` `write_canon_forced`) | `-Tdot`/`-Txdot`/`-Tcanon` for graphs without dictionaries (built through the API) | the object's `AttrMap` against the defaults | as `agwrite` | `AttrMap` iteration: byte order |
| `json::write_json` | `-Tjson`, `-Tjson0`, `-Tdot_json`, `-Txdot_json` | the root's dictionary of the object's kind (`agnxtattr(root, kind, …)`, `gvrender_core_json.c:342`), every symbol, skipping empty values except `label`; `_draw_`-style values as xdot ops | name order | `BTreeSet<String>` of the root dictionary's names (or the union of the objects' keys): byte order |
| `sync_dicts`, `attach_attrs` (`render/lib.rs`, `emit/output.rs`) | which names the writers above see (`safe_dcl` of `pos`, `bb`, `lp`, `_draw_`, …) | the objects' keys | declaration only: order is not output, but *which* names are declared is | `BTreeSet`s of keys |
| `-Tplain`, `-Tplain-ext` (`emit/output.rs` `write_plain`) | fixed fields | fixed reads (`style`, `color`, `fillcolor`, `label`, `tailport`/`headport`) | code order | code order; no walk |
| SVG, xdot draw ops, the other renderers | — | fixed reads (`id`, `class`, `URL`, `tooltip`, …) | code order | no walk |

So writers need name order only, which `Schema.by_name` gives. For `agwrite`'s views, walk the
root's `by_name` and take each name's nearest local definition up the parent chain (a
`LocalDict` lookup by `AttrId`). That yields the same sequence as today's merged `BTreeMap`,
because every subgraph symbol also exists in the root (`agnewsym` declares it there first).

**Byte order, not Unicode order.** `BTreeMap<String, _>` compares UTF-8 bytes, which is `strcmp`.
`by_name` must sort by `as_bytes()`. Names spelled as HTML sort by their inner text, as C's
lookup does.

**Id order is observable too.** `sym->id` doesn't decide any writer's order, but dot's aux graph
copies the root's symbols in name order (after the ProtoGraph ones), so its ids differ from the
root's. `dot_init_node(auxg)` then reads aux objects with *main-graph* symbols, which yields
another attribute's value (`dotsplines.rs:272–313`, #84). Today the port rebuilds both numberings
from `AttrSym.id`. With `AttrId` equal to C's `sym->id`, the same code becomes an index into the
aux object's slots. Ids must therefore be assigned exactly as cgraph assigns them: ProtoGraph
symbols first, then in order of first declaration on the root, whatever subgraph the
declaration came from.

**Tests that prove it** (each added in the step that changes the representation, all
byte-for-byte against committed C output):

1. The existing ratchets: `cargo test --workspace` runs the golden `-Tdot` comparison (every
   corpus input) and `gv-diff render`, which compares C's `-Tdot`, `-Txdot`, `-Tcanon`,
   `-Tjson` and the other render formats.
2. A focused fixture with names that sort differently by bytes, Unicode and declaration order:
   `Z`, `a`, `_draw_`, `é`, an HTML name `<b>`, a name declared first in a subgraph, one
   redeclared locally with a new default, and one set only on an edge with a `tailport`. Check
   `-Tcanon`, `-Tdot`, `-Txdot` and `-Tjson` against C.
3. A property test in core: for random declare/set sequences, `AttrsRef::iter()` equals today's
   `AttrMap` iteration, and `Schema` ids equal what the parser stores in `AttrSym.id` today. It
   runs against the old representation while both exist (steps 2–3).
4. The regression tests of #84 and #113 (#340: the aux graph's stale `penwidth`/`arrowsize`) stay
   green: they pin id order.

## Typed values parsed once, warnings unmoved

**The `late_*` family is pure.** C's `late_int`, `late_double`, `late_string`, `late_nnstring`,
`late_bool` and `mapbool`/`mapBool` (`lib/common/utils.c:40–103` and `mapBool`) print nothing:
an unparsable value silently gives the default. These are the 55 + 58 + 46 + 47 + 23 + 9 + 4
sites counted above. For them, caching changes nothing on stderr by construction.

**The parses that warn** take a string, and some of them print a warning *every* time C calls
them:

| parse | message (C) | when C prints it | port |
| :--- | :--- | :--- | :--- |
| arrow types (`arrow_match_name`) | `Arrow type "x" unknown - ignoring` | every `arrow_flags` call: `arrow_clip` (`splines.c:79`), the call on a virtual edge's original (`arrows.c:247`), neato's `user_spline` (`neatoinit.c:278`); so more than once per edge | `core::arrows`, inside `reporting()` |
| shapes (`bind_shape` → `user_shape`) | `using box for unknown shape x` | once per name | `node_size.rs:485`, `emit_once` |
| colours (`colorxlate` via `gvrender_resolve_color`) | `x is not a known color.` | once per distinct name (`emit_once("color <name>")`, `gvrender.c:205`) | `core::color:238` |
| colour lists (`parseSegs`) | `Illegal value in "…" color attribute; …`, `Total size > 1 in "…" color spec` | per call | `render/emit/emit.rs:72,84` |
| `style` (`parse_style`) | `truncating style 'x'` | per call | `render/emit/attrs.rs:157` |
| `gvrender_set_style` | `unsupported style x - ignoring` | per call | `render/emit/job.rs:634` |
| `overlap` | `Unrecognized overlap value "x" - using false` | per graph | `neatogen/adjust.rs:274` |
| neato `mode`, `model`, `stresswt` | `Illegal value x for attribute "mode" …`, `Unknown value x for attribute "model" …`, `stresswt attribute value must be 1 or 2 …` | per graph | `neatoinit.rs:183,213,293` |
| `len` | `bad edge len "x"` (continued by ` in g - setting to 1.00`) | per edge | `neatogen/stuff.rs:104,216` |
| `charset` | `Unsupported charset "x" - assuming utf-8` | per graph | `core::charset:249` |
| sfdp `label_scheme`; `pagedir`; `layerselect` | `label_scheme = n > 4 : ignoring`; `pagedir=x ignored`; `The layerselect attribute "x" does not match …` | per graph or job | `sfdpinit.rs:140`, `job.rs:742`, `emit.rs:484` |
| node `image`, `shapefile` | `No or improper image="x" for node "n"` | per node | `node_size.rs:1003` |
| edge `pos`, fdp `coords` | `pos attribute for edge (t,h) doesn't have 3n+1 points`, `graph g, coord p, expected four doubles` | per object | `neatoinit.rs:752`, `fdpgen/layout.rs:488` |

The port also runs some of C's work twice (label and node sizing for ports, splines and
rendering) and silences the repeats with `diag::quietly`/`silently`. So the port already treats
"call sites" and "work" as two different things.

**The design: memoise by symbol, not by (object, attribute).**

1. **Pure parses are memoised per `Sym`.** A value string never changes once interned, so the
   parse of a `Sym` never goes stale. Setting an attribute only points the slot at another `Sym`.
   There is nothing to invalidate. The cache holds only the default-free part:
   `strtod_rest(s) -> Option<f64>` and `strtol(s) -> (i64, usize)`. `late_double(v, dflt, min)`
   then applies `dflt` and `min` to the memoised result, which is the same IEEE comparison on
   the same `f64`. Shape: a per-`Interner` `Vec<OnceLock<NumParse>>` filled lazily, which is
   safe, keeps `Graph: Sync` for T11/T12 and allocates nothing for strings never parsed. Every
   `fontsize=14` in a graph shares one parse.
2. **Warning parses keep every call site and replay.** A parse that warns becomes
   `fn parse_x(s: &str) -> (T, SmallDiags)`, a pure function whose diagnostics depend on the string
   only (true of all the rows above except the last two, which name the object and are
   formatted at the call from a memoised error kind). It may be memoised per `Sym`, but every
   C call site still calls it, and the *call site* emits the returned diagnostics, through
   `reporting()`/`quietly` and `emit_once` exactly as now. With D14 (#522), "emit" means
   pushing the values onto the job's diagnostics list.
3. **No eager parse with deferred warnings.** Parsing everything at load time and holding the
   warnings until "C's point" would need a model of C's order for every attribute on every
   path, including the paths where C never reads a value and so never warns. Memoising lazily
   at the first C-ordered call is equivalent and needs no such model.

Why the output stays identical:

- **stdout:** a memoised parse returns the same value as a fresh one because the function is
  pure and its input (the interned string) is immutable. `late_double` still applies the same
  `dflt`/`min` comparisons to the same `f64`.
- **stderr:** the sequence of call sites, the arguments and the `quietly`/`emit_once` state at
  each call don't change, so each call emits the same diagnostics in the same order. The cache
  removes repeated *work*, never a call.
- **Exit status:** errors are counted where they are emitted (`diag::agerr`), so the count is
  unchanged.

**Tests.** The corpus ratchets don't look at stderr: `gv-diff e2e` sends it to `/dev/null`
(`e2e.rs:113`), and only `graphviz-pure-cli/tests/cli.rs` checks warnings (319 `Warning:`
assertions). Before the typed-value step there should be a **stderr ratchet**: C's stderr for
every corpus input (hashed, recorded by `gv-diff e2e --generate` next to the SVG hashes) and a
baseline of the inputs whose port stderr matches it byte for byte. The step must not lose any
of those. The memoisation itself gets unit tests that call each warning parse twice on one `Sym`
and check that both calls return the same diagnostics.

## Migration plan

Each step is one PR, byte-identical, passing every ratchet (`cargo test --workspace`,
`scripts/update-baselines.sh` with no file lost, `gv-diff render` and `e2e`,
`scripts/panic-sites.sh`, clippy, fmt), with before/after numbers from `scripts/bench/`. The
fidelity class is the post-1.0 plan's: *none*, *ordering* or *float-order*.

| step | change | fidelity | size | gain |
| :--- | :--- | :--- | :--- | :--- |
| 0 | Preconditions: D1 and D3 land (#520), so `Graph`'s fields and `AttrMap`'s serde derives can change. A stderr ratchet over the corpus (above). | none (test-only) | S | — |
| 1 | **Close `AttrMap`'s API.** Remove `Deref<Target = BTreeMap>` and `IntoIterator`. Add the methods the read sites use (`get`, `contains`, `iter` in name order, `keys`, `len`, `is_empty`). Fold the six `late_*` helper copies into `core::cnum` ones over `&AttrMap`. | none | M (mechanical, ~200 sites) | none: makes step 3 local |
| 2 | **`Schema` from `AttrDicts`.** Introduce `AttrId`, `Schema` (ids as `AttrSym.id`, `by_name`) and `LocalDict`, built by the parser alongside `dicts`. The property test checks they agree, then `agwrite`, json and `dotsplines`' aux numbering read the new types and `AttrDicts` goes. | ordering | M | small |
| 3 | **Interner and dense store.** `Attrs(Box<[Val]>)` replaces `AttrMap` on nodes, edges and graphs; `Val::ABSENT` keeps "no value" where today's maps lack a key; `AttrsRef<'g>` for readers; writes through `Graph`. Parser interns as it builds. | ordering | L | **memory: ~156 MB of 309 MB on big, ~51 MB of 101 MB on chain, ~47 MB of the corpus' 52 MB of attributes**; every graph clone shrinks the same way |
| 4 | **Ids resolved once.** A `Syms` table per layout context for the hot reads (dot `dot_init_node`/`_edge`, `common_init_node`, neato/sfdp init, emit). | none | M | lookups 41–51 ns → 3–11 ns per read |
| 5 | **Pure parses memoised per `Sym`** (`late_double`, `late_int`, `mapbool`). | none, *if* the stderr ratchet is in | S | about 110 ns of the 160–168 ns of a repeated numeric read |
| 6 | **Warning parses memoised, diagnostics replayed** at each call site (arrows, style, colour lists). Only where a profile shows them. | none | S each | unknown; probably small |
| 7 | **Copies share the interner.** `LGraph::new`, `Model::build` and `attach_attrs` keep `Sym`s instead of copying `String`s (baseline hotspot 15). | none | M | part of the 1.5–3.1× RSS gap at 20k nodes |
| 8 | (optional) `Val::ABSENT` removed: every object gets every declared attribute, as in cgraph, which retires the per-site "cgraph gave them \"\"" emulation (`dotsplines.rs:293`). | ordering | M | simpler code |

Steps 1–3 are the risky part, and step 3 is the large PR. It could be split by object kind
(edges first: the largest share, and fewest read sites outside dot and the writers), with
`AttrMap` and `Attrs` coexisting behind `AttrsRef` for one release.

## Expected gains

- **Memory, parse:** the retained graph on big drops from 309 MB to about 153 MB (175.2 MB of maps
  become 19.2 MB), and on chain from 101 MB to about 50 MB. The parse *peak* (1,415 MB on big)
  is dominated by the token vector, so on its own T4 brings it down by about the same 156 MB.
  T3 (#535) does the rest. Together they make the parsed big graph about 1.7× C's whole-process
  RSS instead of 3.4×. The remaining gap is mostly `Vec<EdgeData>` at 376 B per edge.
- **Memory, end to end:** every live copy of the graph (the parsed graph, the clone per output in
  `attach_attrs`, the layouts' working copies) loses about 90% of its attribute bytes. I have
  not measured how many copies are live at the peak. That is step 3's first measurement.
- **Allocations:** two heap blocks per attribute per object (key, value), plus B-tree nodes,
  become one block per object. On big that is about 2.4 M fewer live allocations after parsing.
- **Time:** small. Attribute frames are at most 2–3% of attribute-heavy dot runs, and malloc
  and free (8–10% there) shrink with the allocation count. Expect low single-digit percent on
  `-Tdot`/`-Tsvg` of attribute-heavy inputs, and nothing measurable on layout-bound ones (b29,
  sfdp, neato).

## Risks

- **Order.** A missed walk that today relies on `BTreeMap` order and gets `AttrId` order after
  step 3. Step 1 makes every walk an explicit `iter()` call, and `iter()` is defined as name
  order. The writers' fixture (test 2) and the property test (test 3) catch the rest.
- **Ids.** `AttrId` must equal cgraph's `sym->id`, including for attributes a layout declares
  later (`safe_dcl`) and for the aux graph. The regression tests of #84 and #113 pin this.
- **Absence.** Today's per-object "no value" is load-bearing in places (objects made by layouts,
  attributes declared after an object). `Val::ABSENT` keeps it until step 8, which should be a
  separate, ordering-class PR.
- **Interned strings are never freed.** A graph that rewrites a value many times keeps every
  version. Layouts set a bounded number of values (positions, `_draw_` ops) per object, so this
  stays a constant factor. It also changes the answer to C's refstr-membership questions
  (#541, `root_holds_plain`). That one is a correctness risk, not a memory one.
- **Repeated declarations.** Declaring after objects exist grows every object's slice, as C
  does. A layout that declares in a loop would be quadratic. Today none does.
- **API churn.** Steps 1 and 3 touch every crate. They are only possible because core, parser,
  geom, layout and render are implementation crates after D1, with the facade's `Graph` opaque.
  A downstream user of the internal crates sees breaking changes, which D1 allows.
- **wasm and 32-bit.** `u32` symbols and ids cap one graph at 2³¹ distinct strings (the HTML bit
  takes one). That is far beyond any input that fits in memory.

## Open questions for the maintainer

1. **Dense (recommended) or sparse per-object storage?** The memory is the same within 5% over the
   corpus. Dense matches cgraph and gives O(1) reads.
2. **Keep `Val::ABSENT` permanently, or remove it (step 8)?** Removing it matches cgraph and
   deletes per-site emulation, but it is a second ordering-class change.
3. **`root_holds_plain` (#541):** should the interner track live counts per `Sym` (a `u32` per
   string, no `Rc`) so that it can answer C's refstr question exactly, or should #541 keep its
   own per-output string set? I lean towards the latter: it keeps the interner simple and the
   question local to the one writer that asks it.
4. **A corpus stderr ratchet** (step 0) would be new test infrastructure, and the C references
   would need regenerating to record stderr. Is that acceptable, or is the CLI test suite's
   coverage of warnings judged enough for steps 5–6?
5. **Issue scope.** #536 asks for "a prototype on the parser and one engine". This PR has a
   standalone prototype (store and interner, built from the parsed graph) with memory and
   read-cost numbers. An in-tree prototype on the parser and one engine is essentially step 3
   for one object kind. Should that be the next PR under #536, or a new issue per step of the
   migration plan?

## Side finding: `agwrite` is quadratic in nodes × edges

`agwrite`'s `node_test` (`render/agwrite.rs:291`) calls `has_no_edges` (`:284`), which scans
every edge of the graph (`members_e[gi]`) for each node it considers writing. On the 50k-node
chain, `-Tcanon` spends **55.7%** of its samples in `node_test`, and takes 6.39 s where C takes
2.12 s (dot `-Tdot` chain5k: 6.4%). C's `has_no_edges` is
`agfstin(g, n) == NULL && agfstout(g, n) == NULL` (`write.c:414`), which is O(1). A per-graph
"has an edge" bit array built once (or the out/in lists `agwrite` already builds) fixes it with
no change in order. That is a separate fix like #541, not part of T4. It is not filed yet.

## Reproducing

Inputs (`python3 gen.py` writes `in/big.gv` and `in/chain.gv`). `mid` and `chain5k` are
`big(f, 5000, 10000)` and `chain(f, 5000)`:

```python
import random
random.seed(536)
def big(f, n=100_000, m=200_000):
    f.write("digraph big {\n")
    shapes = ["box", "ellipse", "circle", "diamond"]
    colors = ["red", "blue", "green", "black", "gray"]
    for i in range(n):
        k = 3 + i % 3
        a = [f'label="node {i}"', f"shape={shapes[i%4]}", f"color={colors[i%5]}",
             f"fontsize={10+i%7}", f'tooltip="t{i}"'][:k]
        f.write(f"  n{i} [{', '.join(a)}];\n")
    for j in range(m):
        k = 3 + j % 3
        a = [f"color={colors[j%5]}", f"weight={1+j%4}", f"penwidth={1+j%3}",
             f'label="e{j}"', "arrowhead=vee"][:k]
        f.write(f"  n{random.randrange(n)} -> n{random.randrange(n)} [{', '.join(a)}];\n")
    f.write("}\n")
def chain(f, n=50_000):
    f.write("digraph chain {\n  node [shape=box, color=blue, fontsize=11];\n")
    for i in range(n):
        f.write(f'  n{i} [label="node {i}", tooltip="tip {i}"];\n')
    for i in range(n - 1):
        f.write(f'  n{i} -> n{i+1} [color=red, weight=2, label="{i}"];\n')
    f.write("}\n")
big(open("in/big.gv", "w")); chain(open("in/chain.gv", "w"))
```

The probe is a scratch crate outside the repo, depending on `graphviz-pure-parser` and
`graphviz-pure-core` by path. It has a counting `#[global_allocator]` (the only `unsafe`, as in
the baseline's counter) and the prototype interner and stores described above, in safe Rust.
It measures:

- `mem FILE`: parse time, allocation count and peak live heap; the prototype stores' size as a
  live-heap delta; then what the graph holds, by clearing one part at a time.
- `density FILE`: the same sizes per corpus file (run over the 2,666 `.gv`/`.dot` files outside
  `tests/golden`).
- `lookup FILE`: the read costs above.

C's numbers are `/usr/bin/time -f %M /opt/graphviz-16.1.0/bin/gc FILE` and `dot -K… -T…` with
`GVBINDIR=/opt/graphviz-16.1.0/lutplugins`. Profiles are
`sudo perf record -F 4000 -g` on the release CLI built with `CARGO_PROFILE_RELEASE_DEBUG=1`.
