use crate::geom::{BoundingBox, Point, Spline};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubgraphId(pub u32);

/// An object's attributes (cgraph's `agxget`/`agxset` values), by name. The values are
/// strings, and [`AttrMap::is_html`] tells which are HTML strings (cgraph's `aghtmlstr`, a
/// flag on the string), whose value is the inner text. The flag is not an attribute: no
/// attribute name sets it (#365). The map reads like a `BTreeMap<String, String>`, which it
/// dereferences to; it changes only through its methods, which keep the flags in step with
/// the values (writing a plain value clears the flag, as `agxset` of a plain string does).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttrMap {
    values: BTreeMap<String, String>,
    /// The keys whose value is an HTML string.
    html: BTreeSet<String>,
}

impl AttrMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// A map of plain (not HTML) values.
    pub fn from_plain(values: impl IntoIterator<Item = (String, String)>) -> Self {
        AttrMap {
            values: values.into_iter().collect(),
            html: BTreeSet::new(),
        }
    }

    /// `aghtmlstr(agget(obj, key))`: whether the value of `key` is an HTML string.
    pub fn is_html(&self, key: &str) -> bool {
        self.html.contains(key)
    }

    /// `agxset` / `agxset_html`: sets `key` to `value`, an HTML string's inner text when
    /// `html`. Returns the previous value.
    pub fn set(&mut self, key: &str, value: String, html: bool) -> Option<String> {
        if html {
            self.html.insert(key.to_string());
        } else {
            self.html.remove(key);
        }
        self.values.insert(key.to_string(), value)
    }

    /// Sets `key` to the plain string `value` (`agxset`), clearing its HTML flag.
    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        self.html.remove(&key);
        self.values.insert(key, value)
    }

    /// Removes `key` and its HTML flag.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.html.remove(key);
        self.values.remove(key)
    }

    /// The map with `f` applied to every key and value, keeping the HTML flags.
    pub fn map(&self, f: impl Fn(&str) -> Cow<'_, str>) -> AttrMap {
        let mut out = AttrMap::new();
        for (k, v) in &self.values {
            out.set(&f(k), f(v).into_owned(), self.is_html(k));
        }
        out
    }

    /// Keeps the attributes `keep` accepts, with their flags.
    pub fn retain(&mut self, mut keep: impl FnMut(&String, &String) -> bool) {
        let html = &mut self.html;
        self.values.retain(|k, v| {
            let kept = keep(k, v);
            if !kept {
                html.remove(k);
            }
            kept
        });
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.html.clear();
    }

    /// Drops the HTML flag of `key`'s value, keeping the value.
    pub fn unmark_html(&mut self, key: &str) {
        self.html.remove(key);
    }

    /// Sets every attribute of `src` on `self`, each with its HTML flag (`applyattrs`).
    pub fn extend_from(&mut self, src: &AttrMap) {
        for (k, v) in &src.values {
            self.set(k, v.clone(), src.is_html(k));
        }
    }

    /// The attributes `keep` accepts, with their flags.
    pub fn filtered(&self, mut keep: impl FnMut(&String, &String) -> bool) -> AttrMap {
        let mut out = AttrMap::new();
        for (k, v) in &self.values {
            if keep(k, v) {
                out.set(k, v.clone(), self.is_html(k));
            }
        }
        out
    }
}

impl std::ops::Deref for AttrMap {
    type Target = BTreeMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<'a> IntoIterator for &'a AttrMap {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::btree_map::Iter<'a, String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphKind {
    Undirected,
    Directed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Port {
    pub name: Option<String>,
    pub compass: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeData {
    pub id: NodeId,
    pub name: String,
    pub attrs: AttrMap,
    /// Layout center in points (72 pt/inch).
    pub pos: Point,
    /// Width in inches (Graphviz convention).
    pub width: f64,
    /// Height in inches (Graphviz convention).
    pub height: f64,
    pub rank: i32,
    pub order: usize,
    /// The name was an HTML string (`<b>`): `name` holds its inner text, and cgraph's
    /// `aghtmlstr` is set on it. Writers print it as `<b>`; everything else sees `b`.
    pub html_name: bool,
    /// The layout's result beyond the position and size.
    pub layout: NodeLayout,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeData {
    pub id: EdgeId,
    pub tail: NodeId,
    pub head: NodeId,
    pub tail_port: Port,
    pub head_port: Port,
    pub attrs: AttrMap,
    pub spline: Spline,
    pub label_pos: Option<Point>,
    /// The layout could not place the label (`ED_label(e)->set` is false):
    /// `lp` is still printed, but renderers do not draw it.
    pub label_unplaced: bool,
    /// The cgraph name of the edge: the `key` of the statement that made it (`agedge`'s
    /// `name`). Defaults never name an edge (`edge [key=…]`, `-Ekey=…`), although `-E`
    /// gives every edge a `key` attribute.
    pub key: Option<String>,
    /// The key is an HTML string (`<...>`): cgraph's string identity includes the flag, so
    /// `key=<k>` and `key="k"` name two edges, and `agwrite` prints it back as `<k>`.
    pub key_html: bool,
    /// The layout's result beyond the spline and the main label.
    pub layout: EdgeLayout,
}

/// The name test of `is_a_cluster` (lib/common/utils.c): `strncasecmp(name, "cluster", 7)`.
///
/// This compares bytes, as C does. Slicing the `str` at byte 7 instead panics when byte 7
/// falls inside a multi-byte character (issue #165).
pub fn has_cluster_prefix(name: &str) -> bool {
    name.as_bytes()
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"cluster"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubgraphData {
    pub id: SubgraphId,
    pub name: String,
    pub is_cluster: bool,
    pub graph_attrs: AttrMap,
    pub node_defaults: AttrMap,
    pub edge_defaults: AttrMap,
    /// Every node of the subgraph, its nested subgraphs' included, each once: cgraph's
    /// `agsubnode` installs a node in the subgraph and in each of its ancestors.
    pub nodes: Vec<NodeId>,
    /// Every edge of the subgraph, its nested subgraphs' included, each once
    /// (`installedge`).
    pub edges: Vec<EdgeId>,
    pub subgraphs: Vec<SubgraphId>,
    pub bb: BoundingBox,
    /// The name was an HTML string; `name` holds its inner text (see [`NodeData::html_name`]).
    pub html_name: bool,
    /// The subgraph is named by the empty string (`subgraph "" {…}`), which cgraph keeps
    /// as a name: only a missing name makes a subgraph anonymous (grammar.y `opensubg`).
    pub empty_name: bool,
    /// The layout's result beyond the bounding box.
    pub layout: GraphLayout,
}

impl SubgraphData {
    /// `agnameof` gives no name of the subgraph's own: it has none, or a local `%` name.
    pub fn is_anonymous(&self) -> bool {
        (self.name.is_empty() && !self.empty_name) || self.name.starts_with('%')
    }
}

/// A label a layout positioned (`textlabel_t`'s `pos` and `set`): `set` is false where the
/// layout could not place it, and C still prints the position but draws nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LabelPlace {
    pub pos: Point,
    pub set: bool,
}

/// What a layout leaves on a node for the renderers and writers, beyond [`NodeData::pos`]
/// and the size: the `Agnodeinfo_t` fields C reads in `emit.c` and `attach_attrs_and_arrows`.
/// It is kept out of the node's attributes, which hold only what the user and cgraph set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeLayout {
    /// `AGSEQ(n)`, where it is not the node's index + 1 (after [`Graph::delete_nodes`], and
    /// in fdp's [`Graph::render_view`]). Renderers print it in object ids.
    pub seq: Option<usize>,
    /// `IS_CLUST_NODE(n)`: fdp's `processClusterEdges` replaced the node by a cluster node,
    /// which renderers neither draw nor give layout attributes.
    pub clust_node: bool,
    /// `ND_lw`, `ND_rw` in points, where they differ (dot keeps the self-loop space in
    /// `ND_rw`): renderers split the width with them instead of halving it.
    pub lw_rw: Option<(f64, f64)>,
    /// `ND_xlabel(n)->pos` of a placed (`set`) external label.
    pub xlabel: Option<Point>,
    /// `ND_pos(n)[2..GD_odim(g)]` in points: the coordinates after `x` and `y` that
    /// `attach_attrs_and_arrows` writes in `pos` when `GD_odim(g) >= 3` (`dimen`).
    pub higher_dims: Vec<f64>,
}

/// Where an edge's final port names come from ([`EdgeLayout::port_names`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PortNames {
    /// The layout keeps the input's ports, so renderers read the edge's own `tailport` and
    /// `headport`: every engine but dot, and dot's `phase` outputs, which stop before
    /// `dot_sameports`.
    #[default]
    Input,
    /// dot's final names, which `sameport` may have cleared (empty). dot sets them for every
    /// edge, drawn or not, so a renderer never rebuilds a dot edge's names from its input.
    Layout { tail: String, head: String },
}

/// What a layout leaves on an edge for the renderers and writers, beyond the spline and the
/// main label ([`EdgeData::spline`], [`EdgeData::label_pos`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdgeLayout {
    /// `AGSEQ(e)`, as [`NodeLayout::seq`].
    pub seq: Option<usize>,
    /// `ED_xlabel(e)->pos` of a placed (`set`) external label.
    pub xlabel: Option<Point>,
    /// `ED_head_label(e)` and `ED_tail_label(e)`: drawn only when `set`.
    pub head_label: Option<LabelPlace>,
    pub tail_label: Option<LabelPlace>,
    /// The tail and head port names `\E` substitutes (`ED_tail_port(e).name`,
    /// `ED_head_port(e).name`).
    pub port_names: PortNames,
    /// The edge was not given layout attributes although it may have a spline:
    /// `attach_attrs_and_arrows` skips an `IGNORED` edge (dot's `orthoEdges` under
    /// `concentrate` routes the first edge of each node pair whatever its type), and every
    /// edge when `State < GVSPLINES` (`neato -n` with given splines). Renderers draw its
    /// spline (`emit_edge_graphics` has no type check).
    pub no_attrs: bool,
}

/// Whether a layout put a cluster in its parent's `GD_clust`, which `emit_clusters` and
/// `rec_attach_bb` walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClusterReg {
    /// The layout said nothing: a cluster with a box is drawn.
    #[default]
    Unknown,
    /// In `GD_clust`, so drawn even with an empty box (a layout that stopped on an error).
    Registered,
    /// Left out of `GD_clust` (clusters below neato's top-level ones): it keeps its input
    /// `bb`, which C prints, but is not drawn.
    Unregistered,
}

/// What a layout leaves on a cluster or the root graph for the renderers and writers,
/// beyond the bounding box ([`Graph::bb`], [`SubgraphData::bb`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphLayout {
    /// The cluster's place in `GD_clust` (always [`ClusterReg::Unknown`] on the root).
    pub reg: ClusterReg,
    /// `AGSEQ(g)` of a subgraph, where it is not its index + 1 (after
    /// [`Graph::agdelete_subgraphs`]). Renderers print it in cluster ids.
    pub seq: Option<usize>,
    /// `GD_label(g)` stays NULL: the layout registered the cluster without making its label
    /// (patchwork's `mkClusters`), so renderers neither draw nor size it.
    pub no_label: bool,
    /// `GD_label(g)->pos` as the layout placed it.
    pub label_pos: Option<Point>,
    /// `GD_clust(g)` where it is not the subgraph tree: subgraph indices in order, with
    /// repeats. dot's pack branch (`copyClusterInfo`) builds the root's list in component
    /// order and appends a cluster once per component that kept it; `emit_clusters` draws
    /// the list as it is.
    pub clusters: Option<Vec<usize>>,
    /// Root only: the layout bound edge records without `common_init_edge`
    /// (`patchwork_init_edge`), so edges have no labels.
    pub no_edge_init: bool,
    /// Root only: `GD_odim(g)`, the output dimension neato and sfdp set from `dimen`.
    pub odim: usize,
    /// Root only: the image files and epsf shapes the layout found, which the renderers draw.
    pub usershapes: crate::usershape::UserShapes,
    /// Root only: `GD_drawing(g)->xdots`, the `_background` ops `graph_init` parsed.
    pub background: Option<crate::xdot::Background>,
}

/// A cgraph object, as [`Graph::agnameof`] takes it: the root graph, a subgraph (by index)
/// or a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obj {
    Root,
    Subgraph(usize),
    Node(NodeId),
}

/// A name as `agnameof` (lib/cgraph/id.c) returns it: a string of the object's own, or
/// `%<id>` formatted into its one `static char buf[32]` (id.c:162).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgName<'a> {
    pub name: Cow<'a, str>,
    /// The name is agnameof's static buffer.
    pub buffer: bool,
}

impl<'a> AgName<'a> {
    /// A name the object keeps as a string of its own.
    pub fn own(name: impl Into<Cow<'a, str>>) -> Self {
        Self {
            name: name.into(),
            buffer: false,
        }
    }

    /// A name formatted into agnameof's static buffer.
    pub fn buffer(name: impl Into<Cow<'a, str>>) -> Self {
        Self {
            name: name.into(),
            buffer: true,
        }
    }
}

/// The names `N` calls of `agnameof` give as the arguments of one C call (`agerr`,
/// `fprintf`, the substitutions of `strdup_and_subst_obj0`), in the order C evaluates them.
///
/// Every argument that is agnameof's static buffer points at the same bytes, so all of them
/// show the name formatted last: `node '%3', graph '%1'` prints as `node '%1', graph '%1'`.
/// Names of the objects' own are left as they are. C evaluates the arguments left to right
/// (clang), which is the order of `names`.
pub fn agnameof_buffer<'a, const N: usize>(names: [AgName<'a>; N]) -> [Cow<'a, str>; N] {
    let last = names
        .iter()
        .rev()
        .find(|n| n.buffer)
        .map(|n| n.name.clone());
    names.map(|n| match &last {
        Some(buf) if n.buffer => buf.clone(),
        _ => n.name,
    })
}

/// Whether a node's name is agnameof's static buffer: the parser names a local (`%…`) node
/// `%<id>`, and `endgraph` drops the local names from cgraph's internal map, so agnameof
/// formats the name. A graph a layout makes later keeps the `%` names it is given as strings
/// ([`Graph::local_node_names_kept`]).
pub fn node_name_is_buffer(name: &str, local_node_names_kept: bool) -> bool {
    name.starts_with('%') && !local_node_names_kept
}

/// Arena-backed Graph representation (Fixes GV-BUG-008 and GV-BUG-010).
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub name: String,
    pub strict: bool,
    pub kind: GraphKind,
    pub graph_attrs: AttrMap,
    pub node_defaults: AttrMap,
    pub edge_defaults: AttrMap,
    pub nodes: Vec<NodeData>,
    pub node_lookup: BTreeMap<String, NodeId>,
    pub edges: Vec<EdgeData>,
    pub subgraphs: Vec<SubgraphData>,
    pub bb: BoundingBox,
    /// cgraph's internal ids of anonymous subgraphs (subgraph index -> id); the root graph,
    /// anonymous subgraphs and edges draw odd ids from one counter in creation order. C
    /// prints an anonymous subgraph's name as `%<id>`.
    pub anon_subgraph_ids: BTreeMap<u32, u64>,
    /// The next id from the root's shared anonymous-object counter (`idmap`'s `counter`).
    pub next_anon_id: u64,
    /// cgraph attribute dictionaries (`Agdatadict_t`) as built while parsing: index 0 is the
    /// root graph, index `i + 1` subgraph `i`. Writers that reproduce `agwrite` use them to
    /// tell local declarations from inherited values.
    pub dicts: Vec<AttrDicts>,
    /// The graph as renderers that run before `undoClusterEdges` see it: fdp leaves its
    /// cluster nodes (`__0:cluster`) and cloned cluster edges in the graph it lays out, and
    /// only the output writers (dot, xdot, json, canon, plain) undo them. `-Tsvg` draws this
    /// graph when it is set.
    pub render_view: Option<Box<Graph>>,
    /// The root is named by the empty string (`digraph "" {…}`); an empty `name` is
    /// otherwise an anonymous root (`agopen(NULL)`), which cgraph calls `%1`.
    pub empty_name: bool,
    /// The graph is one a layout made after parsing (dot's `auxg`): `agnode(auxg,
    /// agnameof(orig), 1)` stores a `%` name in its internal map, so agnameof returns that
    /// string, not its static buffer.
    pub local_node_names_kept: bool,
    /// The subgraphs are ccomps' projections (`projectG`'s `agsubg(g, agnameof(subg), 1)`),
    /// which keep an anonymous original's `%<id>` as a string in the internal map.
    pub local_subgraph_names_kept: bool,
    /// The layout's result beyond the bounding box.
    pub layout: GraphLayout,
}

/// One declared attribute (`Agsym_t`): its default value and whether a root attribute
/// statement set it (`sym->print`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttrSym {
    pub defval: String,
    pub print: bool,
    /// `sym->id`: declaration order in the root dictionary (cgraph's ProtoGraph
    /// `label` first). A subgraph's local symbol has its root symbol's id, as `agnewsym`
    /// gives it.
    pub id: usize,
    /// `aghtmlstr(sym->name)`: the attribute was first named by an HTML string (`<zzz>=1`).
    /// The name is its inner text, which is how cgraph looks attributes up.
    pub html: bool,
    /// `aghtmlstr(sym->defval)`: the default is an HTML string (its inner text).
    pub html_defval: bool,
    /// Declared on cgraph's ProtoGraph (the node `label`, `-G`/`-N`/`-E`), which every root
    /// graph copies with its ids: a graph made later, like dot's aux graph, numbers these
    /// symbols the same way.
    pub proto: bool,
}

/// The local graph, node and edge dictionaries of one graph or subgraph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttrDicts {
    pub graph: BTreeMap<String, AttrSym>,
    pub node: BTreeMap<String, AttrSym>,
    pub edge: BTreeMap<String, AttrSym>,
}

impl Graph {
    /// The first free anonymous id in a root opened with `name` (`agopen` and `idmap`).
    pub fn initial_next_anon_id(name: Option<&str>) -> u64 {
        if name.is_none_or(|n| n.starts_with('%')) {
            3
        } else {
            1
        }
    }

    pub fn new(name: impl Into<String>, kind: GraphKind, strict: bool) -> Self {
        let name = name.into();
        let next_anon_id = Self::initial_next_anon_id((!name.is_empty()).then_some(name.as_str()));
        Self {
            name,
            next_anon_id,
            strict,
            kind,
            graph_attrs: AttrMap::new(),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: Vec::new(),
            node_lookup: BTreeMap::new(),
            edges: Vec::new(),
            subgraphs: Vec::new(),
            bb: BoundingBox::default(),
            anon_subgraph_ids: BTreeMap::new(),
            dicts: Vec::new(),
            render_view: None,
            empty_name: false,
            local_node_names_kept: false,
            local_subgraph_names_kept: false,
            layout: GraphLayout::default(),
        }
    }

    /// The graph attributes of a graph a layout opens itself (`agopen`, like circo's and fdp's
    /// derived graphs): a new root copies cgraph's ProtoGraph, so it declares only the
    /// command-line attributes (`-G`), with their values as its defaults.
    pub fn protograph_attrs(&self) -> AttrMap {
        AttrMap::from_plain(self.dicts.first().into_iter().flat_map(|d| {
            d.graph
                .iter()
                .filter(|(_, s)| s.proto)
                .map(|(k, s)| (k.clone(), s.defval.clone()))
        }))
    }

    /// An empty root a layout opens itself (`agopen(name, ...)`) while it lays out `self`: it
    /// declares only the ProtoGraph's (command-line) graph, node and edge attributes, whose
    /// defaults are its values and its node and edge defaults.
    pub fn protograph_root(&self, name: &str, kind: GraphKind, strict: bool) -> Graph {
        let mut g = Graph::new(name, kind, strict);
        g.graph_attrs = self.protograph_attrs();
        if let Some(d) = self.dicts.first() {
            let proto = |dict: &BTreeMap<String, AttrSym>| -> BTreeMap<String, AttrSym> {
                dict.iter()
                    .filter(|(_, s)| s.proto)
                    .map(|(k, s)| (k.clone(), s.clone()))
                    .collect()
            };
            let dicts = AttrDicts {
                graph: proto(&d.graph),
                node: proto(&d.node),
                edge: proto(&d.edge),
            };
            let defaults = |dict: &BTreeMap<String, AttrSym>| {
                AttrMap::from_plain(dict.iter().map(|(k, s)| (k.clone(), s.defval.clone())))
            };
            g.node_defaults = defaults(&dicts.node);
            g.edge_defaults = defaults(&dicts.edge);
            g.dicts = vec![dicts];
        }
        g
    }

    /// Whether nodes declare `key`: `agattr(g, AGNODE, key, NULL) != NULL`
    /// (`agfindnodeattr`). cgraph declares an attribute on the root's dictionary wherever it is
    /// first set, so one set only by a subgraph's `node [...]` is declared for the whole graph.
    /// The parser records that dictionary in `dicts[0]`, and it is the whole answer: a value a
    /// layout stored on a node without declaring it does not count, as in C. Only a graph built
    /// without dictionaries falls back to its root defaults and its nodes' values.
    pub fn node_attr_declared(&self, key: &str) -> bool {
        match self.dicts.first() {
            Some(d) => d.node.contains_key(key),
            None => {
                self.node_defaults.contains_key(key)
                    || self.nodes.iter().any(|n| n.attrs.contains_key(key))
            }
        }
    }

    /// [`Graph::node_attr_declared`] for edges (`agfindedgeattr`).
    pub fn edge_attr_declared(&self, key: &str) -> bool {
        match self.dicts.first() {
            Some(d) => d.edge.contains_key(key),
            None => {
                self.edge_defaults.contains_key(key)
                    || self.edges.iter().any(|e| e.attrs.contains_key(key))
            }
        }
    }

    /// [`Graph::node_attr_declared`] for graphs (`agfindgraphattr`): a subgraph's `key=value`
    /// declares `key` on the root too.
    pub fn graph_attr_declared(&self, key: &str) -> bool {
        match self.dicts.first() {
            Some(d) => d.graph.contains_key(key),
            None => self.graph_attrs.contains_key(key),
        }
    }

    /// The root has no name of its own (see [`Graph::empty_name`]).
    pub fn is_anonymous(&self) -> bool {
        (self.name.is_empty() && !self.empty_name) || self.name.starts_with('%')
    }

    /// `agnameof` (lib/cgraph/id.c): the name of the root, a subgraph or a node as C's
    /// messages and writers print it, and whether it is agnameof's static buffer.
    ///
    /// cgraph names an anonymous object (no name, or a local `%` name, which `endgraph`
    /// forgets) by its id, `%<id>`. The root, anonymous subgraphs, local names and keyless
    /// edges draw odd ids from one counter per root, in creation order: an anonymous root is
    /// `%1`, an anonymous subgraph has its id in [`Graph::anon_subgraph_ids`], and the parser
    /// already names a local node `%<id>`.
    pub fn agnameof(&self, obj: Obj) -> AgName<'_> {
        match obj {
            Obj::Root => AgName {
                name: Cow::Borrowed(self.root_name()),
                buffer: self.is_anonymous(),
            },
            Obj::Subgraph(s) if self.subgraphs[s].is_anonymous() => AgName {
                name: Cow::Owned(format!("%{}", self.anon_subgraph_id(s))),
                buffer: !self.local_subgraph_names_kept,
            },
            Obj::Subgraph(s) => AgName::own(self.subgraphs[s].name.as_str()),
            Obj::Node(n) => {
                let name = self.nodes[n.0 as usize].name.as_str();
                AgName {
                    name: Cow::Borrowed(name),
                    buffer: node_name_is_buffer(name, self.local_node_names_kept),
                }
            }
        }
    }

    /// The cgraph id of anonymous subgraph `s`: the parser's, or, for a subgraph made without
    /// one, a fresh odd id above every recorded one, so that no two subgraphs share a name.
    fn anon_subgraph_id(&self, s: usize) -> u64 {
        if let Some(&id) = self.anon_subgraph_ids.get(&(s as u32)) {
            return id;
        }
        let top = self.anon_subgraph_ids.values().copied().max().unwrap_or(1);
        let unrecorded = (0..s)
            .filter(|&i| {
                self.subgraphs[i].is_anonymous()
                    && !self.anon_subgraph_ids.contains_key(&(i as u32))
            })
            .count() as u64;
        top + 2 * (unrecorded + 1)
    }

    /// `agnameof(g)` of the root graph: its name, or `%1` when it is anonymous.
    pub fn root_name(&self) -> &str {
        if self.is_anonymous() {
            "%1"
        } else {
            &self.name
        }
    }

    /// `agnameof` of subgraph `s`.
    pub fn subgraph_name(&self, s: usize) -> String {
        self.agnameof(Obj::Subgraph(s)).name.into_owned()
    }

    /// Each subgraph lists every node and edge once, and every node and edge of its
    /// nested subgraphs too (cgraph's `agsubnode`/`installedge`), as the parser builds
    /// them. Layouts read a subgraph's members from its own lists.
    pub fn subgraph_lists_are_closed(&self) -> bool {
        let mut node_in = vec![usize::MAX; self.nodes.len()];
        let mut edge_in = vec![usize::MAX; self.edges.len()];
        self.subgraphs.iter().enumerate().all(|(i, s)| {
            let nodes_once = s.nodes.iter().all(|n| {
                node_in
                    .get_mut(n.0 as usize)
                    .is_some_and(|m| std::mem::replace(m, i) != i)
            });
            let edges_once = s.edges.iter().all(|e| {
                edge_in
                    .get_mut(e.0 as usize)
                    .is_some_and(|m| std::mem::replace(m, i) != i)
            });
            nodes_once
                && edges_once
                && s.subgraphs.iter().all(|c| {
                    self.subgraphs.get(c.0 as usize).is_some_and(|c| {
                        c.nodes
                            .iter()
                            .all(|n| node_in.get(n.0 as usize) == Some(&i))
                            && c.edges
                                .iter()
                                .all(|e| edge_in.get(e.0 as usize) == Some(&i))
                    })
                })
        })
    }

    /// `agfindnode(g, name)` of a name read after parsing (an attribute value): a local name
    /// (`%…`) is looked up in cgraph's internal map, which `endgraph` cleared of them, so it
    /// finds nothing, unless the graph keeps them ([`Graph::local_node_names_kept`]).
    pub fn agfindnode(&self, name: &str, html: bool) -> Option<NodeId> {
        if node_name_is_buffer(name, self.local_node_names_kept) {
            return None;
        }
        self.node_lookup
            .get(Self::node_key(name, html).as_ref())
            .copied()
    }

    pub fn ensure_node(&mut self, name: &str) -> NodeId {
        self.ensure_node_html(name, false)
    }

    /// The `node_lookup` key of a node named `name`. cgraph keeps an HTML string apart from
    /// the plain string with the same text (`refstr_eq` compares the flag), so `<b>` and `b`
    /// are two nodes. An HTML name is keyed with a NUL in front, which no C string holds.
    pub fn node_key(name: &str, html: bool) -> std::borrow::Cow<'_, str> {
        if html {
            std::borrow::Cow::Owned(format!("\0{name}"))
        } else {
            std::borrow::Cow::Borrowed(name)
        }
    }

    /// [`Graph::ensure_node`] for a node whose name may be an HTML string.
    pub fn ensure_node_html(&mut self, name: &str, html: bool) -> NodeId {
        let key = Self::node_key(name, html);
        if let Some(&id) = self.node_lookup.get(key.as_ref()) {
            return id;
        }
        let key = key.into_owned();
        let id = NodeId(self.nodes.len() as u32);
        let mut attrs = self.node_defaults.clone();
        if !attrs.contains_key("label") {
            attrs.insert("label".to_string(), "\\N".to_string());
        }
        self.nodes.push(NodeData {
            id,
            name: name.to_string(),
            attrs,
            pos: Point::default(),
            width: 0.75,
            height: 0.5,
            rank: 0,
            order: id.0 as usize,
            html_name: html,
            layout: NodeLayout::default(),
        });
        self.node_lookup.insert(key, id);
        id
    }

    /// `agdelete(root, n)` of every node `delete` marks: the nodes go, with their edges, from
    /// the graph and every subgraph. The objects left keep their cgraph sequence numbers
    /// ([`NodeLayout::seq`], [`EdgeLayout::seq`]), which object ids in the output are made of.
    pub fn delete_nodes(&mut self, delete: &[bool]) {
        let mut node_map = vec![None; self.nodes.len()];
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for (i, mut n) in std::mem::take(&mut self.nodes).into_iter().enumerate() {
            if delete.get(i).copied().unwrap_or(false) {
                continue;
            }
            n.layout.seq.get_or_insert(i + 1);
            let id = NodeId(nodes.len() as u32);
            node_map[i] = Some(id);
            n.id = id;
            nodes.push(n);
        }
        self.nodes = nodes;
        let mut edge_map = vec![None; self.edges.len()];
        let mut edges = Vec::with_capacity(self.edges.len());
        for (i, mut e) in std::mem::take(&mut self.edges).into_iter().enumerate() {
            let (Some(t), Some(h)) = (node_map[e.tail.0 as usize], node_map[e.head.0 as usize])
            else {
                continue;
            };
            e.layout.seq.get_or_insert(i + 1);
            let id = EdgeId(edges.len() as u32);
            edge_map[i] = Some(id);
            (e.id, e.tail, e.head) = (id, t, h);
            edges.push(e);
        }
        self.edges = edges;
        for sg in &mut self.subgraphs {
            sg.nodes = sg
                .nodes
                .iter()
                .filter_map(|n| node_map[n.0 as usize])
                .collect();
            sg.edges = sg
                .edges
                .iter()
                .filter_map(|e| edge_map[e.0 as usize])
                .collect();
        }
        self.node_lookup = self
            .nodes
            .iter()
            .map(|n| (Self::node_key(&n.name, n.html_name).into_owned(), n.id))
            .collect();
    }

    /// `agdelete(g, subg)` for each of `del` (subgraph indices): the subgraphs and the ones
    /// nested in them are gone, with their local dictionaries; their nodes and edges stay in
    /// the graph. The other subgraphs are renumbered in order, and every subgraph index the
    /// graph holds follows.
    pub fn agdelete_subgraphs(&mut self, del: &[usize]) {
        let n = self.subgraphs.len();
        let mut gone = vec![false; n];
        let mut stack: Vec<usize> = del.iter().copied().filter(|&s| s < n).collect();
        while let Some(s) = stack.pop() {
            if !std::mem::replace(&mut gone[s], true) {
                stack.extend(self.subgraphs[s].subgraphs.iter().map(|c| c.0 as usize));
            }
        }
        if !gone.contains(&true) {
            return;
        }
        let mut new_index = vec![None; n];
        let mut next = 0u32;
        for (s, g) in gone.iter().enumerate() {
            if !g {
                new_index[s] = Some(next);
                next += 1;
            }
        }
        let map = |s: usize| new_index.get(s).copied().flatten();
        let dicts = self.dicts.len() == n + 1;
        let old = std::mem::take(&mut self.subgraphs);
        for (s, mut sg) in old.into_iter().enumerate() {
            let Some(id) = map(s) else { continue };
            // agdelete leaves the others' sequence numbers as they were
            sg.layout.seq = sg
                .layout
                .seq
                .or(Some(s + 1))
                .filter(|&q| q != id as usize + 1);
            sg.id = SubgraphId(id);
            sg.subgraphs = sg
                .subgraphs
                .iter()
                .filter_map(|c| map(c.0 as usize).map(SubgraphId))
                .collect();
            if let Some(list) = sg.layout.clusters.as_mut() {
                *list = list
                    .iter()
                    .filter_map(|&c| map(c))
                    .map(|c| c as usize)
                    .collect();
            }
            self.subgraphs.push(sg);
        }
        if let Some(list) = self.layout.clusters.as_mut() {
            *list = list
                .iter()
                .filter_map(|&c| map(c))
                .map(|c| c as usize)
                .collect();
        }
        self.anon_subgraph_ids = std::mem::take(&mut self.anon_subgraph_ids)
            .into_iter()
            .filter_map(|(s, id)| map(s as usize).map(|s| (s, id)))
            .collect();
        if dicts {
            let mut it = std::mem::take(&mut self.dicts).into_iter();
            self.dicts.extend(it.next());
            self.dicts
                .extend(it.zip(&gone).filter(|(_, g)| !**g).map(|(d, _)| d));
        }
    }

    pub fn add_edge(&mut self, tail: NodeId, head: NodeId, attrs: AttrMap) -> EdgeId {
        if self.strict
            && let Some(existing) = self.edges.iter_mut().find(|e| {
                (e.tail == tail && e.head == head)
                    || (self.kind == GraphKind::Undirected && e.tail == head && e.head == tail)
            })
        {
            existing.attrs.extend_from(&attrs);
            return existing.id;
        }
        let mut merged_attrs = self.edge_defaults.clone();
        merged_attrs.extend_from(&attrs);
        self.new_edge(tail, head, merged_attrs)
    }

    /// Makes an edge with exactly `attrs`, even where a strict graph already has one
    /// (`agedge` does, for a new key in a subgraph that lacks the edge).
    pub fn new_edge(&mut self, tail: NodeId, head: NodeId, attrs: AttrMap) -> EdgeId {
        let id = EdgeId(self.edges.len() as u32);
        self.edges.push(EdgeData {
            id,
            tail,
            head,
            tail_port: Port::default(),
            head_port: Port::default(),
            attrs,
            spline: Spline::default(),
            label_pos: None,
            label_unplaced: false,
            key: None,
            key_html: false,
            layout: EdgeLayout::default(),
        });
        id
    }
}

#[cfg(test)]
mod cluster_prefix_tests {
    use super::has_cluster_prefix;

    #[test]
    fn compares_the_first_seven_bytes_case_insensitively() {
        assert!(has_cluster_prefix("cluster"));
        assert!(has_cluster_prefix("Cluster_a"));
        assert!(has_cluster_prefix("CLUSTERé"));
        assert!(!has_cluster_prefix("cluste"));
        assert!(!has_cluster_prefix("aaaaaaé"));
        assert!(!has_cluster_prefix("clusé_x"));
        assert!(!has_cluster_prefix(""));
    }
}

#[cfg(test)]
mod protograph_tests {
    use super::*;

    // Issue #398: a root a layout opens keeps only the ProtoGraph's (command-line) symbols.
    #[test]
    fn a_new_root_declares_only_the_protograph_attributes() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let sym = |defval: &str, proto: bool| AttrSym {
            defval: defval.to_string(),
            proto,
            ..AttrSym::default()
        };
        let mut d = AttrDicts::default();
        d.graph.insert("overlap".into(), sym("false", true));
        d.graph.insert("splines".into(), sym("", false));
        d.node.insert("label".into(), sym("\\N", true));
        d.node.insert("shape".into(), sym("box", false));
        d.edge.insert("weight".into(), sym("", false));
        d.edge.insert("len".into(), sym("2", true));
        g.dicts = vec![d];
        g.graph_attrs.insert("splines".into(), "true".into());
        g.graph_attrs.insert("overlap".into(), "false".into());
        let r = g.protograph_root("derived", GraphKind::Undirected, true);
        assert_eq!(
            r.graph_attrs.get("overlap").map(String::as_str),
            Some("false")
        );
        assert!(!r.graph_attrs.contains_key("splines"));
        assert!(r.node_attr_declared("label") && !r.node_attr_declared("shape"));
        assert!(r.edge_attr_declared("len") && !r.edge_attr_declared("weight"));
        assert_eq!(r.edge_defaults.get("len").map(String::as_str), Some("2"));
        assert!(r.nodes.is_empty() && r.edges.is_empty());
    }
}

#[cfg(test)]
mod agnameof_tests {
    use super::*;

    #[test]
    fn anonymous_objects_are_named_by_their_id() {
        let mut g = Graph::new("", GraphKind::Directed, false);
        assert_eq!(g.agnameof(Obj::Root), AgName::buffer("%1"));
        g.name = "%r".to_string();
        assert_eq!(g.root_name(), "%1");
        g.name = String::new();
        g.empty_name = true;
        assert_eq!(g.agnameof(Obj::Root), AgName::own(""));
        let a = g.ensure_node("a");
        let l = g.ensure_node("%3");
        assert_eq!(g.agnameof(Obj::Node(a)), AgName::own("a"));
        assert_eq!(g.agnameof(Obj::Node(l)), AgName::buffer("%3"));
        assert_eq!(g.agfindnode("a", false), Some(a));
        assert_eq!(g.agfindnode("%3", false), None);
        // A graph made after parsing keeps the `%` names it is given (dot's auxg).
        g.local_node_names_kept = true;
        assert_eq!(g.agnameof(Obj::Node(l)), AgName::own("%3"));
        assert_eq!(g.agfindnode("%3", false), Some(l));
    }

    #[test]
    fn every_anonymous_subgraph_has_its_own_id() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        for name in ["", "named", "", ""] {
            g.subgraphs.push(SubgraphData {
                id: SubgraphId(g.subgraphs.len() as u32),
                name: name.to_string(),
                is_cluster: false,
                graph_attrs: AttrMap::new(),
                node_defaults: AttrMap::new(),
                edge_defaults: AttrMap::new(),
                nodes: Vec::new(),
                edges: Vec::new(),
                subgraphs: Vec::new(),
                bb: BoundingBox::default(),
                html_name: false,
                empty_name: false,
                layout: Default::default(),
            });
        }
        g.anon_subgraph_ids.insert(2, 7);
        let names: Vec<String> = (0..4).map(|s| g.subgraph_name(s)).collect();
        assert_eq!(names, ["%9", "named", "%7", "%11"]);
        assert!(g.agnameof(Obj::Subgraph(2)).buffer);
        // ccomps' projections keep the original's `%<id>` as a string.
        g.local_subgraph_names_kept = true;
        assert_eq!(g.agnameof(Obj::Subgraph(2)), AgName::own("%7"));
    }

    #[test]
    fn buffer_names_in_one_call_show_the_last_one() {
        let b = AgName::buffer;
        let o = AgName::own;
        assert_eq!(agnameof_buffer([b("%3"), b("%1")]), ["%1", "%1"]);
        assert_eq!(agnameof_buffer([b("%3"), o("G")]), ["%3", "G"]);
        assert_eq!(
            agnameof_buffer([b("%1"), b("%3"), o("b")]),
            ["%3", "%3", "b"]
        );
        // A `%` name a derived graph keeps (ccomps' dg, dot's auxg) is not the buffer.
        assert_eq!(
            agnameof_buffer([b("%5"), b("%7"), o("%3")]),
            ["%7", "%7", "%3"]
        );
    }

    /// `digraph { subgraph s0 { x0 subgraph s1 { x1 … subgraph s{n-1} { x{n-1} } } } }` with
    /// an edge `x{i} -> x{i+1}` in each `s{i}`, its lists closed as the parser makes them.
    fn nested(n: usize) -> Graph {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let xs: Vec<NodeId> = (0..n).map(|i| g.ensure_node(&format!("x{i}"))).collect();
        let es: Vec<EdgeId> = (1..n)
            .map(|i| g.add_edge(xs[i - 1], xs[i], AttrMap::new()))
            .collect();
        for i in 0..n {
            g.subgraphs.push(SubgraphData {
                id: SubgraphId(i as u32),
                name: format!("s{i}"),
                is_cluster: false,
                graph_attrs: AttrMap::new(),
                node_defaults: AttrMap::new(),
                edge_defaults: AttrMap::new(),
                nodes: xs[i..].to_vec(),
                edges: es[i..].to_vec(),
                subgraphs: (i + 1 < n)
                    .then(|| SubgraphId(i as u32 + 1))
                    .into_iter()
                    .collect(),
                bb: BoundingBox::default(),
                html_name: false,
                empty_name: false,
                layout: Default::default(),
            });
        }
        g
    }

    /// Issue #323: layouts read a subgraph's members from its own lists, so they must hold
    /// the nested subgraphs' members, each once.
    #[test]
    fn subgraph_lists_are_closed_checks_nesting_and_duplicates() {
        let mut g = nested(4);
        assert!(g.subgraph_lists_are_closed());
        g.subgraphs[1].nodes.retain(|&n| n != NodeId(3));
        assert!(!g.subgraph_lists_are_closed(), "s1 lacks s2's x3");
        let mut g = nested(4);
        g.subgraphs[0].edges.retain(|&e| e != EdgeId(2));
        assert!(!g.subgraph_lists_are_closed(), "s0 lacks s1's edge");
        let mut g = nested(4);
        g.subgraphs[2].nodes.push(NodeId(3));
        assert!(!g.subgraph_lists_are_closed(), "s2 lists x3 twice");
    }
}

#[cfg(test)]
mod attr_map_tests {
    use super::*;

    // #365: the HTML flag is not an attribute, so no attribute name sets it.
    #[test]
    fn html_flag_is_not_an_attribute() {
        let mut a = AttrMap::new();
        a.insert("\u{1}html:label".into(), "<b>x</b>".into());
        a.insert("label".into(), "<b>x</b>".into());
        assert!(!a.is_html("label"));
        assert_eq!(a.len(), 2);
        a.set("label", "x".into(), true);
        assert!(a.is_html("label"));
        let mut b = AttrMap::new();
        b.set("xlabel", "z".into(), true);
        a.extend_from(&b);
        assert!(a.is_html("xlabel"));
        a.unmark_html("xlabel");
        assert!(!a.is_html("xlabel"));
    }

    // agxset of a plain string clears the flag, even with the same text; removing the
    // attribute clears it too.
    #[test]
    fn plain_writes_clear_the_flag() {
        let mut a = AttrMap::new();
        a.set("label", "x".into(), true);
        a.insert("label".into(), "x".into());
        assert!(!a.is_html("label"));
        a.set("label", "x".into(), true);
        a.remove("label");
        a.insert("label".into(), "x".into());
        assert!(!a.is_html("label"));
        a.set("label", "x".into(), true);
        a.retain(|k, _| k != "label");
        a.insert("label".into(), "x".into());
        assert!(!a.is_html("label"));
        a.set("label", "x".into(), true);
        a.set("shape", "box".into(), false);
        let f = a.filtered(|k, _| k == "label");
        assert!(f.is_html("label") && !f.contains_key("shape"));
    }

    // The public `add_edge` keeps the flags of the attributes it is given, on a new edge and
    // on a strict graph's existing one.
    #[test]
    fn add_edge_keeps_html_flags() {
        for strict in [false, true] {
            let mut g = Graph::new("G", GraphKind::Directed, strict);
            g.edge_defaults.set("color", "red".into(), false);
            let (a, b) = (g.ensure_node("a"), g.ensure_node("b"));
            let mut attrs = AttrMap::new();
            attrs.set("label", "<b>e</b>".into(), true);
            let e = g.add_edge(a, b, attrs);
            assert!(g.edges[e.0 as usize].attrs.is_html("label"));
            let mut more = AttrMap::new();
            more.set("headlabel", "h".into(), true);
            let e2 = g.add_edge(a, b, more);
            let ed = &g.edges[e2.0 as usize];
            assert!(ed.attrs.is_html("headlabel"));
            assert_eq!(ed.attrs.get("color").map(String::as_str), Some("red"));
            assert_eq!(e == e2, strict);
        }
    }
}

#[cfg(test)]
mod agdelete_tests {
    use super::*;

    fn sub(id: u32, name: &str, kids: &[u32]) -> SubgraphData {
        SubgraphData {
            id: SubgraphId(id),
            name: name.into(),
            is_cluster: name.starts_with("cluster"),
            graph_attrs: AttrMap::new(),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            subgraphs: kids.iter().map(|&k| SubgraphId(k)).collect(),
            bb: BoundingBox::default(),
            html_name: false,
            empty_name: false,
            layout: GraphLayout::default(),
        }
    }

    // #335: deleting a subgraph drops its nested ones and renumbers every subgraph index.
    #[test]
    fn deleting_a_subgraph_renumbers_the_rest() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        g.subgraphs = vec![
            sub(0, "cluster_a", &[]),
            sub(1, "_cc_0", &[2]),
            sub(2, "inner", &[]),
            sub(3, "cluster_b", &[4]),
            sub(4, "%5", &[]),
        ];
        g.dicts = (0..6).map(|_| AttrDicts::default()).collect();
        g.dicts[4].graph.insert("color".into(), AttrSym::default());
        g.anon_subgraph_ids.insert(4, 5);
        g.layout.clusters = Some(vec![0, 3]);
        g.subgraphs[3].layout.clusters = Some(vec![4]);
        g.agdelete_subgraphs(&[1]);
        let names: Vec<_> = g.subgraphs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["cluster_a", "cluster_b", "%5"]);
        assert!(
            g.subgraphs
                .iter()
                .enumerate()
                .all(|(i, s)| s.id.0 as usize == i)
        );
        assert_eq!(g.subgraphs[1].subgraphs, [SubgraphId(2)]);
        assert_eq!(g.layout.clusters, Some(vec![0, 1]));
        assert_eq!(g.subgraphs[1].layout.clusters, Some(vec![2]));
        assert_eq!(g.anon_subgraph_ids.get(&2), Some(&5));
        assert_eq!(g.dicts.len(), 4);
        assert!(g.dicts[2].graph.contains_key("color"));
        // AGSEQ stays: only the renumbered ones record it
        let seqs: Vec<_> = g.subgraphs.iter().map(|s| s.layout.seq).collect();
        assert_eq!(seqs, [None, Some(4), Some(5)]);
    }
}
