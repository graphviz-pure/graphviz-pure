//! The plugins of a run (`gvc->packages` and `gvc->apis`) and what `dot -v` and `dot -P` say
//! about them: `gvplugin_load`'s verbose lines, `gvplugin_write_status` and `gvplugin_graph`
//! (`lib/gvc/gvplugin.c`).
//!
//! The port's renderers, devices, layouts and image loaders are built in. They are reported as
//! a statically linked C build (`dot_static`, `cmd/dot/dot_builtins.cpp`) reports its own: no
//! plugin library is activated, demand loading is disabled, and the packages are recorded in
//! its built-in order. The packages are those the port's tables name: `core`, `dot_layout`,
//! `neato_layout`, and `cairo` for the raster devices (`-T png:cairo`, as the port has always
//! named them; in C only `png` comes from that package, `jpeg` from `gd` or `gdk`).

use std::collections::{HashMap, HashSet};

use graphviz_pure_core::Graph;
use graphviz_pure_core::diag::fprintf_stderr;
pub use graphviz_pure_core::diag::{set_verbose, verbose};
use graphviz_pure_layout::LayoutEngine;
use graphviz_pure_parser::GlobalAttrs;

/// The layout plugins, as `gvplugin_list(gvc, API_layout, "")` lists them, with their
/// packages.
pub const LAYOUT_PLUGINS: [(&str, &str); 11] = [
    ("circo", "neato_layout"),
    ("dot", "dot_layout"),
    ("fdp", "neato_layout"),
    ("neato", "neato_layout"),
    ("nop", "neato_layout"),
    ("nop1", "neato_layout"),
    ("nop2", "neato_layout"),
    ("osage", "neato_layout"),
    ("patchwork", "neato_layout"),
    ("sfdp", "neato_layout"),
    ("twopi", "neato_layout"),
];

/// The output device plugins: type (`format:render`) and package, in `gvplugin_list` order.
pub const DEVICE_PLUGINS: [(&str, &str); 20] = [
    ("canon:dot", "core"),
    ("cmapx:map", "core"),
    ("cmapx_np:map", "core"),
    ("dot:dot", "core"),
    ("dot_json:json", "core"),
    ("gv:dot", "core"),
    ("jpe:cairo", "cairo"),
    ("jpeg:cairo", "cairo"),
    ("jpg:cairo", "cairo"),
    ("json:json", "core"),
    ("json0:json", "core"),
    ("plain:dot", "core"),
    ("plain-ext:dot", "core"),
    ("png:cairo", "cairo"),
    ("svg:svg", "core"),
    ("svg_inline:svg", "core"),
    ("xdot:xdot", "core"),
    ("xdot1.2:xdot", "core"),
    ("xdot1.4:xdot", "core"),
    ("xdot_json:json", "core"),
];

/// `gvplugin_load`'s match of a requested `type[:dependency[:package]]` against `plugins`:
/// the index of the first match. Names compare as C's `strcmp` does, case and all.
pub fn plugin_load(plugins: &[(&str, &str)], req: &str) -> Option<usize> {
    let mut parts = req.splitn(3, ':');
    let reqtyp = parts.next().unwrap_or("");
    let reqdep = parts.next();
    let reqpkg = parts.next();
    plugins.iter().position(|&(typestr, package)| {
        let (typ, dep) = match typestr.split_once(':') {
            Some((t, d)) => (t, Some(d)),
            None => (typestr, None),
        };
        typ == reqtyp
            && !matches!((dep, reqdep), (Some(d), Some(r)) if d != r)
            && reqpkg.is_none_or(|p| p == package)
    })
}

/// `gvplugin_list(gvc, api, str)`: `" type:package"` for each plugin of the requested type
/// when `str` names one with a `:`, else `" type"` for each type.
pub fn plugin_list(plugins: &[(&str, &str)], req: &str) -> String {
    let mut out = String::new();
    if let Some((typ, _)) = req.split_once(':') {
        for &(typestr, package) in plugins {
            let t = typestr.split(':').next().unwrap_or("");
            if typ.is_empty() || typ.eq_ignore_ascii_case(t) {
                out.push_str(&format!(" {typestr}:{package}"));
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    let mut last: Option<&str> = None;
    for &(typestr, _) in plugins {
        let t = typestr.split(':').next().unwrap_or("");
        if last.is_none_or(|l| !l.eq_ignore_ascii_case(t)) {
            out.push(' ');
            out.push_str(t);
        }
        last = Some(t);
    }
    out
}

/// `gvlayout_select`: the engine a layout name (`-K`, the `layout` attribute) selects.
pub fn layout_select(name: &str) -> Option<LayoutEngine> {
    let i = load_layout(name)?;
    LayoutEngine::parse(LAYOUT_PLUGINS[i].0).ok()
}

/// `api_names`, indexed by `api_t`.
pub const API_NAMES: [&str; 5] = ["render", "layout", "textlayout", "device", "loadimage"];
const API_RENDER: usize = 0;
const API_LAYOUT: usize = 1;
const API_TEXTLAYOUT: usize = 2;
const API_DEVICE: usize = 3;
const API_LOADIMAGE: usize = 4;

/// The render plugins: type and package, in `gvc->apis[API_render]` order (by type). The
/// `json` renderer writes four formats, which C lists as four types, as here.
pub const RENDER_PLUGINS: [(&str, &str); 10] = [
    ("cairo", "cairo"),
    ("dot", "core"),
    ("dot_json", "core"),
    ("json", "core"),
    ("json0", "core"),
    ("map", "core"),
    ("svg", "core"),
    ("svg_inline", "core"),
    ("xdot", "core"),
    ("xdot_json", "core"),
];

/// The loadimage plugins (`gvloadimage_core_types` for the port's renderers): `type:render`
/// and package, in `gvc->apis[API_loadimage]` order: by type, the plugin installed last first.
pub const LOADIMAGE_PLUGINS: [(&str, &str); 31] = [
    ("eps:xdot", "core"),
    ("eps:dot", "core"),
    ("eps:map", "core"),
    ("gif:xdot", "core"),
    ("gif:dot", "core"),
    ("gif:map", "core"),
    ("gif:svg", "core"),
    ("jpe:xdot", "core"),
    ("jpe:dot", "core"),
    ("jpe:map", "core"),
    ("jpe:svg", "core"),
    ("jpeg:xdot", "core"),
    ("jpeg:dot", "core"),
    ("jpeg:map", "core"),
    ("jpeg:svg", "core"),
    ("jpg:xdot", "core"),
    ("jpg:dot", "core"),
    ("jpg:map", "core"),
    ("jpg:svg", "core"),
    ("png:xdot", "core"),
    ("png:dot", "core"),
    ("png:map", "core"),
    ("png:svg", "core"),
    // `pango`'s, recorded before `core`'s: of equal quality, it ends up after them.
    ("png:cairo", "cairo"),
    ("ps:xdot", "core"),
    ("ps:dot", "core"),
    ("ps:map", "core"),
    ("svg:svg", "core"),
    ("svg:xdot", "core"),
    ("svg:dot", "core"),
    ("svg:map", "core"),
];

/// The plugins of a run.
#[derive(Debug, Clone, Copy)]
pub struct Plugins<'a> {
    /// `gvc->packages`, in list order: a package recorded later comes first.
    pub packages: &'a [&'a str],
    /// `gvc->apis`, indexed like [`API_NAMES`]: each plugin's type and package, in list
    /// order.
    pub apis: [&'a [(&'a str, &'a str)]; 5],
}

/// The port's plugins. The packages are in the order a static C build records them:
/// `lt_preloaded_symbols` lists `dot_layout`, `neato_layout`, `pango` (package `cairo`) and
/// `core`, and `gvplugin_package_record` puts each new one first.
pub const PORT_PLUGINS: Plugins<'static> = Plugins {
    packages: &["core", "cairo", "neato_layout", "dot_layout"],
    apis: [
        &RENDER_PLUGINS,
        &LAYOUT_PLUGINS,
        &[],
        &DEVICE_PLUGINS,
        &LOADIMAGE_PLUGINS,
    ],
};

/// `gvplugin_load(gvc, api, req)`'s verbose line for the plugin found at `i` of `plugins`.
fn using(api: usize, plugins: &[(&str, &str)], i: usize) {
    if verbose() >= 1 {
        let (typestr, package) = plugins[i];
        fprintf_stderr(&format!("Using {}: {typestr}:{package}\n", API_NAMES[api]));
    }
}

/// `gvplugin_load(gvc, API_layout, req)`: the index in [`LAYOUT_PLUGINS`], with `Using
/// layout: …` at verbose level 1.
pub fn load_layout(req: &str) -> Option<usize> {
    let i = plugin_load(&LAYOUT_PLUGINS, req)?;
    using(API_LAYOUT, &LAYOUT_PLUGINS, i);
    Some(i)
}

/// `gvplugin_load(gvc, API_device, req)`: the index in [`DEVICE_PLUGINS`]. The device's
/// renderer is loaded first, as its dependency, so at verbose level 1 `Using render: …`
/// precedes `Using device: …`.
pub fn load_device(req: &str) -> Option<usize> {
    let i = plugin_load(&DEVICE_PLUGINS, req)?;
    if let Some((_, dep)) = DEVICE_PLUGINS[i].0.split_once(':')
        && let Some(r) = plugin_load(&RENDER_PLUGINS, dep)
    {
        using(API_RENDER, &RENDER_PLUGINS, r);
    }
    using(API_DEVICE, &DEVICE_PLUGINS, i);
    Some(i)
}

/// The type of the device `req` loads (`json` for `json:json`), without a verbose line.
pub fn load_device_quietly(req: &str) -> Option<&'static str> {
    let i = plugin_load(&DEVICE_PLUGINS, req)?;
    DEVICE_PLUGINS[i].0.split(':').next()
}

/// `gvplugin_write_status`: each API's plugins, as types, or at level 2 and up as
/// `type:package`, after the line a build without demand loading writes for it.
pub fn write_status(plugins: &Plugins<'_>) {
    fprintf_stderr("Demand loading of plugins is disabled.\n");
    let req = if verbose() >= 2 { ":" } else { "?" };
    for (api, list) in plugins.apis.iter().enumerate() {
        fprintf_stderr(&format!(
            "    {}\t: {}\n",
            API_NAMES[api],
            plugin_list(list, req)
        ));
    }
}

/// `gvplugin_graph`'s aliases: a device or loadimage type's node name and label.
fn alias(q: &str) -> (&str, &str) {
    if q.starts_with("jp") {
        ("jpg", "jpeg\\njpe\\njpg")
    } else if q.starts_with("tif") {
        ("tif", "tiff\\ntif")
    } else if q == "x11" || q == "xlib" {
        ("x11", "x11\\nxlib")
    } else if q == "dot" || q == "gv" {
        ("gv", "gv\\ndot")
    } else {
        (q, q)
    }
}

/// The cgraph calls of `gvplugin_graph`, written as DOT statements that make the same graph
/// when read: each in the subgraphs the call names, reopened by name.
#[derive(Default)]
struct PluginGraph {
    out: String,
    /// A name or value had no quoted DOT form.
    unquotable: bool,
    /// The nodes of each subgraph (`""` for the root), for `agfindnode`.
    members: HashMap<String, HashSet<String>>,
    /// The edges, for `agfindedge`.
    edges: HashSet<(String, String)>,
}

/// `s` as a quoted DOT string. The scanner turns `\"` into `"` and drops a backslash before
/// a newline, and keeps every other backslash; a string that ends with a backslash, or has
/// one before a newline, has no quoted form.
fn quote(s: &str) -> Option<String> {
    if s.ends_with('\\') || s.contains("\\\n") {
        return None;
    }
    Some(format!("\"{}\"", s.replace('"', "\\\"")))
}

impl PluginGraph {
    fn quote(&mut self, s: &str) -> String {
        quote(s).unwrap_or_else(|| {
            self.unquotable = true;
            String::new()
        })
    }

    fn stmt(&mut self, path: &[&str], stmt: &str) {
        for sg in path {
            let sg = self.quote(sg);
            self.out.push_str(&format!("subgraph {sg} {{ "));
        }
        self.out.push_str(stmt);
        for _ in path {
            self.out.push_str(" }");
        }
        self.out.push('\n');
    }

    fn add_member(&mut self, path: &[&str], name: &str) {
        for sg in std::iter::once(&"").chain(path) {
            self.members
                .entry(sg.to_string())
                .or_default()
                .insert(name.to_string());
        }
    }

    /// `agfindnode(sg, name)`; `""` is the root.
    fn has(&self, sg: &str, name: &str) -> bool {
        self.members.get(sg).is_some_and(|m| m.contains(name))
    }

    /// `agsubg` of the last of `path` and `agxset(sg, key, value)`.
    fn subgraph_attr(&mut self, path: &[&str], key: &str, value: &str) {
        let value = self.quote(value);
        self.stmt(path, &format!("{key}={value};"));
    }

    /// `agnode(sg, name, 1)` for the last of `path` (the root when empty).
    fn node(&mut self, path: &[&str], name: &str) {
        self.add_member(path, name);
        let name = self.quote(name);
        self.stmt(path, &format!("{name};"));
    }

    /// `agxset(n, key, value)`: set from the root, which every node is in already.
    fn node_attr(&mut self, name: &str, key: &str, value: &str) {
        let (name, value) = (self.quote(name), self.quote(value));
        self.stmt(&[], &format!("{name} [{key}={value}];"));
    }

    /// `agedge(sg, t, h, NULL, 1)`, a new edge, with its `style` when given.
    fn edge(&mut self, path: &[&str], t: &str, h: &str, style: Option<&str>) {
        self.add_member(path, t);
        self.add_member(path, h);
        self.edges.insert((t.to_string(), h.to_string()));
        let attrs = style.map_or_else(String::new, |s| format!(" [style={}]", self.quote(s)));
        let (t, h) = (self.quote(t), self.quote(h));
        self.stmt(path, &format!("{t} -> {h}{attrs};"));
    }

    /// `agfindedge(g, t, h)`.
    fn has_edge(&self, t: &str, h: &str) -> bool {
        self.edges.contains(&(t.to_string(), h.to_string()))
    }
}

/// `gvplugin_graph`: the graph `dot -P` lays out and renders instead of its inputs
/// ([`crate::decoded::plugin_graph`]), on decoded `globals`.
///
/// The graph is built as DOT and read back. C declares its attributes with `agattr` and sets
/// them with `agxset`, neither of which marks a symbol to be written (`sym->print`), as the
/// root's attribute statements of the DOT text do: the marks are cleared again.
///
/// `globals` are the `-G`, `-N`, `-E` and `-A` attributes given before `-P`, on which
/// `agopen` builds the graph (`ProtoGraph`). Those `gvplugin_graph` declares itself are
/// replaced: its `agattr_text` gives them its own defaults.
///
/// `None` when a plugin or package name has no quoted DOT form (it ends with a backslash);
/// the port's own tables have none.
pub(crate) fn plugin_graph(plugins: &Plugins<'_>, globals: &GlobalAttrs) -> Option<Graph> {
    const OWN: [&[&str]; 3] = [
        &["label", "rankdir", "rank", "ranksep"],
        &["label", "shape", "style", "width"],
        &["style"],
    ];
    let mut globals = globals.clone();
    let maps = [&mut globals.graph, &mut globals.node, &mut globals.edge];
    for (map, own) in maps.into_iter().zip(OWN) {
        for name in own {
            map.remove(name);
        }
    }
    let mut graph = graphviz_pure_parser::parse_dot_all(&plugin_graph_dot(plugins)?, &globals)
        .ok()?
        .into_iter()
        .next()?;
    if let Some(d) = graph.dicts.first_mut() {
        for (dict, own) in [&mut d.graph, &mut d.node, &mut d.edge]
            .into_iter()
            .zip(OWN)
        {
            for name in own {
                if let Some(sym) = dict.get_mut(*name) {
                    sym.print = false;
                }
            }
        }
    }
    Some(graph)
}

/// The cgraph calls of `gvplugin_graph`, as DOT; `None` when a name has no quoted form.
fn plugin_graph_dot(plugins: &Plugins<'_>) -> Option<String> {
    let mut g = PluginGraph::default();
    g.out.push_str("digraph \"G\" {\n");
    g.out
        .push_str("graph [label=\"\", rankdir=\"\", rank=\"\", ranksep=\"\"];\n");
    g.out
        .push_str("node [label=\"\\N\", shape=\"\", style=\"\", width=\"\"];\n");
    g.out.push_str("edge [style=\"\"];\n");
    g.subgraph_attr(&[], "rankdir", "LR");
    g.subgraph_attr(&[], "ranksep", "2.5");
    g.subgraph_attr(&[], "label", "Plugins");

    for &package in plugins.packages {
        let mut loadimage_n: Option<String> = None;
        let mut renderer_n: Option<String> = None;
        let mut device_n: Option<String> = None;
        let mut textlayout_n: Option<String> = None;
        let mut layout_n: Option<String> = None;
        let mut neededge_loadimage = false;
        let mut neededge_device = false;
        let sg = format!("cluster_{package}");
        g.subgraph_attr(&[&sg], "label", package);
        for (api, list) in plugins.apis.iter().enumerate() {
            let ssg = format!("{package}_{}", API_NAMES[api]);
            let path = [sg.as_str(), ssg.as_str()];
            g.subgraph_attr(&path, "rank", "same");
            for &(typestr, _) in list.iter().filter(|&&(_, p)| p == package) {
                let (q, p) = match typestr.split_once(':') {
                    Some((q, p)) => (q, Some(p)),
                    None => (typestr, None),
                };
                match api {
                    API_DEVICE | API_LOADIMAGE => {
                        let (q, lq) = alias(q);
                        let n = format!("{package}_{}_{q}", API_NAMES[api]);
                        g.node(&path, &n);
                        g.node_attr(&n, "label", lq);
                        g.node_attr(&n, "width", "1.0");
                        g.node_attr(&n, "shape", "box");
                        if !p.is_some_and(|p| !p.is_empty()) {
                            if !g.has(&sg, "render_cg") {
                                g.node(&[&sg], "render_cg");
                                // C sets it through the graph's `label` symbol, whose id is
                                // also the node `label`'s.
                                g.node_attr("render_cg", "label", "cg");
                            }
                            g.edge(&[&sg], "render_cg", &n, None);
                        }
                        if api == API_DEVICE {
                            device_n = Some(n);
                        } else {
                            loadimage_n = Some(n);
                        }
                    }
                    API_RENDER => {
                        let n = format!("{}_{q}", API_NAMES[api]);
                        g.node(&path, &n);
                        g.node_attr(&n, "label", q);
                        renderer_n = Some(n);
                    }
                    API_TEXTLAYOUT => {
                        let n = format!("{}_{q}", API_NAMES[api]);
                        g.node(&path, &n);
                        g.node_attr(&n, "shape", "invtriangle");
                        g.node_attr(&n, "label", "T");
                        textlayout_n = Some(n);
                    }
                    _ => {
                        let n = format!("{}_{q}", API_NAMES[api]);
                        g.node(&path, &n);
                        g.node_attr(&n, "shape", "hexagon");
                        g.node_attr(&n, "label", q);
                        layout_n = Some(n);
                    }
                }
            }
            // invisible nodes and edges that improve the cluster's layout
            if api == API_LOADIMAGE && loadimage_n.is_none() {
                neededge_loadimage = true;
                let n = format!("{package}_{}_invis", API_NAMES[api]);
                g.node(&path, &n);
                g.node_attr(&n, "style", "invis");
                g.node_attr(&n, "label", "");
                g.node_attr(&n, "width", "1.0");
                let src = format!("{package}_{}_invis_src", API_NAMES[api]);
                g.node(&[], &src);
                g.node_attr(&src, "style", "invis");
                g.node_attr(&src, "label", "");
                g.edge(&[], &src, &n, Some("invis"));
                loadimage_n = Some(n);
            }
            if api == API_RENDER && renderer_n.is_none() {
                neededge_loadimage = true;
                neededge_device = true;
                let n = format!("{package}_{}_invis", API_NAMES[api]);
                g.node(&path, &n);
                g.node_attr(&n, "style", "invis");
                g.node_attr(&n, "label", "");
                renderer_n = Some(n);
            }
            if api == API_DEVICE && device_n.is_none() {
                neededge_device = true;
                let n = format!("{package}_{}_invis", API_NAMES[api]);
                g.node(&path, &n);
                g.node_attr(&n, "style", "invis");
                g.node_attr(&n, "label", "");
                g.node_attr(&n, "width", "1.0");
                device_n = Some(n);
            }
        }
        // Each is set: the loop gave every API a node, an invisible one if need be.
        let (Some(loadimage_n), Some(renderer_n), Some(device_n)) =
            (loadimage_n, renderer_n, device_n)
        else {
            continue;
        };
        if neededge_loadimage {
            g.edge(&[&sg], &loadimage_n, &renderer_n, Some("invis"));
        }
        if neededge_device {
            g.edge(&[&sg], &renderer_n, &device_n, Some("invis"));
        }
        if let Some(t) = textlayout_n {
            g.edge(&[&sg], &loadimage_n, &t, Some("invis"));
        }
        if let Some(l) = layout_n {
            g.edge(&[&sg], &loadimage_n, &l, Some("invis"));
        }
    }

    let ssg = "output_formats";
    g.subgraph_attr(&[ssg], "rank", "same");
    for &package in plugins.packages {
        for (api, list) in plugins.apis.iter().enumerate() {
            for &(typestr, _) in list.iter().filter(|&&(_, p)| p == package) {
                let (q, p) = match typestr.split_once(':') {
                    Some((q, p)) => (q, Some(p)),
                    None => (typestr, None),
                };
                let (q, lq) = alias(q);
                match api {
                    API_DEVICE => {
                        let n = format!("{package}_{}_{q}", API_NAMES[api]);
                        g.node(&[], &n);
                        let output = format!("output_{q}");
                        if !g.has(ssg, &output) {
                            g.node(&[ssg], &output);
                            g.node_attr(&output, "label", lq);
                            g.node_attr(&output, "shape", "note");
                        }
                        if !g.has_edge(&n, &output) {
                            g.edge(&[], &n, &output, None);
                        }
                        if let Some(p) = p.filter(|p| !p.is_empty()) {
                            // C looks the renderer up in `output_formats`, where no renderer
                            // is, and makes it in the root: `agnode` finds an existing one.
                            let render = format!("render_{p}");
                            g.node(&[], &render);
                            if !g.has_edge(&render, &n) {
                                g.edge(&[], &render, &n, None);
                            }
                        }
                    }
                    API_LOADIMAGE => {
                        let n = format!("{package}_{}_{q}", API_NAMES[api]);
                        g.node(&[], &n);
                        let input = format!("input_{q}");
                        if !g.has("", &input) {
                            g.node(&[], &input);
                            g.node_attr(&input, "label", lq);
                            g.node_attr(&input, "shape", "note");
                        }
                        if !g.has_edge(&input, &n) {
                            g.edge(&[], &input, &n, None);
                        }
                        let render = format!("render_{}", p.unwrap_or("(null)"));
                        g.node(&[], &render);
                        if !g.has_edge(&n, &render) {
                            g.edge(&[], &n, &render, None);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    g.out.push_str("}\n");
    (!g.unquotable).then_some(g.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The plugins of the reference build (`dot -v2`'s listing with `GVBINDIR` set to the
    // core, dot_layout and neato_layout plugins).
    const RENDER: [(&str, &str); 14] = [
        ("dot", "core"),
        ("dot_json", "core"),
        ("fig", "core"),
        ("json", "core"),
        ("json0", "core"),
        ("map", "core"),
        ("pic", "core"),
        ("pov", "core"),
        ("ps", "core"),
        ("svg", "core"),
        ("svg_inline", "core"),
        ("tk", "core"),
        ("xdot", "core"),
        ("xdot_json", "core"),
    ];
    const LAYOUT: [(&str, &str); 11] = [
        ("circo", "neato_layout"),
        ("dot", "dot_layout"),
        ("fdp", "neato_layout"),
        ("neato", "neato_layout"),
        ("nop", "neato_layout"),
        ("nop1", "neato_layout"),
        ("nop2", "neato_layout"),
        ("osage", "neato_layout"),
        ("patchwork", "neato_layout"),
        ("sfdp", "neato_layout"),
        ("twopi", "neato_layout"),
    ];
    const DEVICE: [(&str, &str); 28] = [
        ("canon:dot", "core"),
        ("cmap:map", "core"),
        ("cmapx:map", "core"),
        ("cmapx_np:map", "core"),
        ("dot:dot", "core"),
        ("dot_json:json", "core"),
        ("eps:ps", "core"),
        ("fig:fig", "core"),
        ("gv:dot", "core"),
        ("imap:map", "core"),
        ("imap_np:map", "core"),
        ("ismap:map", "core"),
        ("json:json", "core"),
        ("json0:json", "core"),
        ("pic:pic", "core"),
        ("plain:dot", "core"),
        ("plain-ext:dot", "core"),
        ("pov:pov", "core"),
        ("ps:ps", "core"),
        ("ps2:ps", "core"),
        ("svg:svg", "core"),
        ("svg_inline:svg", "core"),
        ("svgz:svg", "core"),
        ("tk:tk", "core"),
        ("xdot:xdot", "core"),
        ("xdot1.2:xdot", "core"),
        ("xdot1.4:xdot", "core"),
        ("xdot_json:json", "core"),
    ];
    const LOADIMAGE: [(&str, &str); 39] = [
        ("(lib):ps", "core"),
        ("eps:xdot", "core"),
        ("eps:dot", "core"),
        ("eps:map", "core"),
        ("eps:ps", "core"),
        ("gif:tk", "core"),
        ("gif:xdot", "core"),
        ("gif:dot", "core"),
        ("gif:map", "core"),
        ("gif:fig", "core"),
        ("gif:svg", "core"),
        ("jpe:xdot", "core"),
        ("jpe:dot", "core"),
        ("jpe:map", "core"),
        ("jpe:fig", "core"),
        ("jpe:svg", "core"),
        ("jpeg:xdot", "core"),
        ("jpeg:dot", "core"),
        ("jpeg:map", "core"),
        ("jpeg:fig", "core"),
        ("jpeg:svg", "core"),
        ("jpg:xdot", "core"),
        ("jpg:dot", "core"),
        ("jpg:map", "core"),
        ("jpg:fig", "core"),
        ("jpg:svg", "core"),
        ("png:xdot", "core"),
        ("png:dot", "core"),
        ("png:map", "core"),
        ("png:fig", "core"),
        ("png:svg", "core"),
        ("ps:xdot", "core"),
        ("ps:dot", "core"),
        ("ps:map", "core"),
        ("ps:ps", "core"),
        ("svg:svg", "core"),
        ("svg:xdot", "core"),
        ("svg:dot", "core"),
        ("svg:map", "core"),
    ];
    const REFERENCE: Plugins<'static> = Plugins {
        packages: &["neato_layout", "dot_layout", "core"],
        apis: [&RENDER, &LAYOUT, &[], &DEVICE, &LOADIMAGE],
    };

    // A name with no quoted DOT form gives no graph rather than a panic.
    #[test]
    fn plugin_graph_with_an_unquotable_name_is_none() {
        let odd = Plugins {
            packages: &["core\\"],
            apis: [&[("dot", "core\\")], &[], &[], &[], &[]],
        };
        assert!(plugin_graph(&odd, &GlobalAttrs::default()).is_none());
        assert!(plugin_graph(&PORT_PLUGINS, &GlobalAttrs::default()).is_some());
    }

    // The loadimage list is the renderer's own table: each `TYPE:RENDER` pair is listed
    // exactly when `gvloadimage_select` finds a plugin for it.
    #[test]
    fn loadimage_list_is_the_renderers_table() {
        let types = [
            "(lib)", "bmp", "eps", "gif", "jpe", "jpeg", "jpg", "pdf", "png", "ps", "svg",
        ];
        let renders = RENDER_PLUGINS
            .iter()
            .map(|&(r, _)| r)
            .chain(["fig", "map", "ps"]);
        for render in renders {
            for t in types {
                let pair = format!("{t}:{render}");
                let listed = LOADIMAGE_PLUGINS.iter().any(|&(ts, _)| ts == pair);
                assert_eq!(
                    listed,
                    crate::loadimage_installed(t, render),
                    "{t}:{render}"
                );
            }
        }
    }

    // `gvplugin_graph` of the reference build's plugins is C's `dot -P` graph: before
    // layout (`-Tcanon`) and laid out (`-Tdot`), under the port's math.
    #[test]
    fn plugin_graph_of_the_reference_build_is_cs() {
        let mut g = plugin_graph(&REFERENCE, &GlobalAttrs::default()).expect("quotable");
        graphviz_pure_layout::layout_graph(&mut g, graphviz_pure_layout::LayoutEngine::Dot)
            .expect("layout");
        for (format, want) in [
            (
                "canon",
                include_str!("../../graphviz-pure/tests/data/plugin_graph_reference.canon"),
            ),
            (
                "dot",
                include_str!("../../graphviz-pure/tests/data/plugin_graph_reference.dot"),
            ),
        ] {
            let fmt = crate::OutputFormat::parse(format).expect("format");
            let got = crate::decoded::render_graph_named(&g, fmt, format).expect("render");
            assert_eq!(got, want, "{format}");
        }
    }
}
