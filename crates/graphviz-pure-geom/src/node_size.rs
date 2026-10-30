//! Exact node sizing, ported function by function from Graphviz 16.1.0 `lib/common/shapes.c`
//! (`poly_init`, `point_init`, `record_init`, `parse_reclbl`, `size_reclbl`, `resize_reclbl`,
//! `pos_reclbl`, `star_size`, `star_vertices`, `cylinder_size`, `cylinder_vertices`,
//! `userSize`, `bind_shape`), `lib/common/utils.c` (`common_init_node`, `late_*`, `mapbool`,
//! `gv_nodesize`) and the label part of `lib/common/labels.c` (`make_label`).
//!
//! Entry point: [`node_geometry`], which returns everything C stores on a node after
//! `common_init_node` + the shape `initfn`: `ND_width`/`ND_height` (inches), the outline size,
//! the label geometry (`ND_label->dimen`, `space`, `valign`), the polygon description and, for
//! records, the field tree. [`NodeGeometry::nodesize`] mirrors `gv_nodesize` (`ND_lw`, `ND_rw`,
//! `ND_ht`).

use crate::html_label::HtmlLabel;
use graphviz_pure_core::alloc::{element_count, gv_calloc_vec};
use graphviz_pure_core::cnum::{is_exactly_equal, is_exactly_zero};
use graphviz_pure_core::math;
use graphviz_pure_core::strtod::{sscanf, strtod};
use graphviz_pure_core::usershape;

use crate::shapes::{RecordField, ShapeKind, html_label_layout};
use graphviz_pure_core::charset::Charset;
use graphviz_pure_core::font::{
    DEFAULT_FONT_NAME, DEFAULT_FONT_SIZE, LabelSubstContext, MIN_FONT_SIZE, NamesInBuffer,
    TextLabelLayout, make_text_label,
};
use graphviz_pure_core::{AgName, AttrMap, BoundingBox, Graph, NodeData, Point, Result};
use std::f64::consts::PI;

/// `GAP` from `lib/common/const.h`: whitespace in points between peripheries.
const GAP: f64 = 4.0;
/// `DEFAULT_NODEWIDTH`.
const DEFAULT_NODEWIDTH: f64 = 0.75;
/// `MIN_NODEWIDTH`.
const MIN_NODEWIDTH: f64 = 0.01;
/// `DEFAULT_NODEHEIGHT`.
const DEFAULT_NODEHEIGHT: f64 = 0.5;
/// `MIN_NODEHEIGHT`.
const MIN_NODEHEIGHT: f64 = 0.02;
/// `DEFAULT_NODEPENWIDTH`.
const DEFAULT_NODEPENWIDTH: f64 = 1.0;
/// `MIN_NODEPENWIDTH`.
const MIN_NODEPENWIDTH: f64 = 0.0;
/// `DEF_POINT` (inches) from `shapes.c`.
const DEF_POINT: f64 = 0.05;
/// `MIN_POINT` (inches) from `shapes.c`.
const MIN_POINT: f64 = 0.0003;
/// `SQRT2` from `lib/common/arith.h`.
const SQRT2: f64 = std::f64::consts::SQRT_2;

#[inline]
fn inch2ps(x: f64) -> f64 {
    x * 72.0
}

#[inline]
fn ps2inch(x: f64) -> f64 {
    x / 72.0
}

// ---------------------------------------------------------------------------------------------
// C attribute helpers (`lib/common/utils.c`)
// ---------------------------------------------------------------------------------------------

/// `late_double` from `lib/common/utils.c`.
pub fn late_double(attrs: &AttrMap, key: &str, default: f64, minimum: f64) -> f64 {
    let Some(p) = attrs.get(key).filter(|p| !p.is_empty()) else {
        return default;
    };
    let (rv, used) = strtod(p);
    if used == 0 {
        return default;
    }
    if rv < minimum { minimum } else { rv }
}

/// `late_int` from `lib/common/utils.c`.
pub fn late_int(attrs: &AttrMap, key: &str, default: i32, minimum: i32) -> i32 {
    graphviz_pure_core::cnum::late_int(attrs.get(key).map(String::as_str), default, minimum)
}

/// `late_nnstring` from `lib/common/utils.c`.
pub fn late_nnstring<'a>(attrs: &'a AttrMap, key: &str, default: &'a str) -> &'a str {
    attrs
        .get(key)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
}

pub use graphviz_pure_core::cnum::{map_bool, mapbool};

// ---------------------------------------------------------------------------------------------
// Shape descriptors (`Shapes[]`, `polygon_t p_*` in shapes.c)
// ---------------------------------------------------------------------------------------------

/// Which shape `initfn` C runs for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeInit {
    /// `poly_init` (all polygon shapes, `star`, `cylinder`).
    Poly,
    /// `point_init`.
    Point,
    /// `record_init` (`record`, `Mrecord`).
    Record,
    /// `epsf_init`.
    Epsf,
}

/// Built-in polygon description (`polygon_t`) of a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolygonDesc {
    pub regular: bool,
    pub peripheries: usize,
    pub sides: usize,
    pub orientation: f64,
    pub distortion: f64,
    pub skew: f64,
}

const fn pd(regular: bool, peripheries: usize, sides: usize) -> PolygonDesc {
    PolygonDesc {
        regular,
        peripheries,
        sides,
        orientation: 0.0,
        distortion: 0.0,
        skew: 0.0,
    }
}

impl ShapeKind {
    /// `bind_shape` + `polygon_t` lookup: which init function C runs and the built-in polygon
    /// parameters (`None` for records/epsf).
    pub fn shape_desc(self) -> (ShapeInit, Option<PolygonDesc>) {
        use ShapeKind as S;
        let p = match self {
            S::Record | S::Mrecord => return (ShapeInit::Record, None),
            S::Epsf => return (ShapeInit::Epsf, None),
            S::Point => return (ShapeInit::Point, Some(pd(true, 1, 1))),
            S::Polygon => pd(false, 1, 0),
            S::Ellipse => pd(false, 1, 1),
            S::Circle => pd(true, 1, 1),
            S::Egg => PolygonDesc {
                distortion: -0.3,
                ..pd(false, 1, 1)
            },
            S::Triangle => pd(false, 1, 3),
            S::Plaintext | S::None | S::Plain => pd(false, 0, 4),
            S::Diamond | S::Mdiamond => PolygonDesc {
                orientation: 45.0,
                ..pd(false, 1, 4)
            },
            S::Trapezium => PolygonDesc {
                distortion: -0.4,
                ..pd(false, 1, 4)
            },
            S::Parallelogram => PolygonDesc {
                skew: 0.6,
                ..pd(false, 1, 4)
            },
            S::House => PolygonDesc {
                distortion: -0.64,
                ..pd(false, 1, 5)
            },
            S::Pentagon => pd(false, 1, 5),
            S::Hexagon => pd(false, 1, 6),
            S::Septagon => pd(false, 1, 7),
            S::Octagon => pd(false, 1, 8),
            S::Cylinder => pd(false, 1, 19),
            S::Square | S::Msquare => pd(true, 1, 4),
            S::DoubleCircle => pd(true, 2, 1),
            S::Mcircle => pd(true, 1, 1),
            S::DoubleOctagon => pd(false, 2, 8),
            S::TripleOctagon => pd(false, 3, 8),
            S::InvTriangle => PolygonDesc {
                orientation: 180.0,
                ..pd(false, 1, 3)
            },
            S::InvTrapezium => PolygonDesc {
                orientation: 180.0,
                distortion: -0.4,
                ..pd(false, 1, 4)
            },
            S::InvHouse => PolygonDesc {
                orientation: 180.0,
                distortion: -0.64,
                ..pd(false, 1, 5)
            },
            S::Star => pd(false, 1, 10),
            // box and every 4-sided styled shape (note, tab, folder, box3d, component,
            // underline and the SBOL shapes)
            _ => pd(false, 1, 4),
        };
        (ShapeInit::Poly, Some(p))
    }
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// Graph-level inputs that affect node sizing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSizeEnv<'a> {
    /// Root graph name (for `\G`).
    pub graph_name: &'a str,
    /// `GD_realflip`: `rankdir` is `LR` or `RL` (records lay out their fields transposed).
    pub realflip: bool,
    /// Graph `quantum` in inches (`GD_drawing(g)->quantum`), `0` if unset.
    pub quantum: f64,
    /// `GD_charset(g)` of the root graph.
    pub charset: Charset,
    /// The root's name is agnameof's static buffer (an anonymous root).
    pub graph_name_buffer: bool,
    /// The graph keeps its nodes' `%` names as strings ([`Graph::local_node_names_kept`]).
    pub local_node_names_kept: bool,
}

impl<'a> NodeSizeEnv<'a> {
    /// Environment with defaults (TB, no quantum, the charset of the graph being processed).
    pub fn new(graph_name: &'a str) -> Self {
        Self {
            graph_name,
            realflip: false,
            quantum: 0.0,
            charset: graphviz_pure_core::charset::current(),
            graph_name_buffer: false,
            local_node_names_kept: false,
        }
    }

    /// Environment from an explicit `rankdir` string.
    pub fn with_rankdir(graph_name: &'a str, rankdir: &str) -> Self {
        Self {
            realflip: rankdir_is_flipped(rankdir),
            ..Self::new(graph_name)
        }
    }

    /// Reads `rankdir`, `quantum` and `charset` from the root graph attributes
    /// (`graph_init` / `init_ugraph` in `lib/common/input.c`).
    pub fn from_graph(graph: &'a Graph) -> Self {
        Self::from_graph_attrs(graph, &graph.graph_attrs)
    }

    /// Like [`Self::from_graph`] with another attribute map for the graph's.
    pub fn from_graph_attrs(graph: &'a Graph, attrs: &AttrMap) -> Self {
        Self {
            graph_name_buffer: graph.agnameof(graphviz_pure_core::Obj::Root).buffer,
            local_node_names_kept: graph.local_node_names_kept,
            ..Self::from_attrs(graph.root_name(), attrs)
        }
    }

    /// `rankdir`, `quantum` and `charset` from `attrs`, for a graph named `graph_name`.
    fn from_attrs(graph_name: &'a str, attrs: &AttrMap) -> Self {
        Self {
            realflip: rankdir_is_flipped(attrs.get("rankdir").map_or("", String::as_str)),
            quantum: late_double(attrs, "quantum", 0.0, 0.0),
            charset: Charset::from_attr(attrs.get("charset").map(String::as_str)),
            ..Self::new(graph_name)
        }
    }

    /// Which names of a node's or an edge's label context are agnameof's static buffer.
    pub fn names_in_buffer(&self, node: &str, tail: &str, head: &str) -> NamesInBuffer {
        let kept = self.local_node_names_kept;
        NamesInBuffer {
            graph: self.graph_name_buffer,
            node: graphviz_pure_core::node_name_is_buffer(node, kept),
            tail: graphviz_pure_core::node_name_is_buffer(tail, kept),
            head: graphviz_pure_core::node_name_is_buffer(head, kept),
        }
    }
}

/// `rankdir` parsing in `graph_init`: only exact `LR`/`BT`/`RL` are recognised;
/// returns `true` for `LR` and `RL` (`GD_realflip`).
pub fn rankdir_is_flipped(rankdir: &str) -> bool {
    matches!(rankdir, "LR" | "RL")
}

/// Label geometry of a node (`textlabel_t` fields relevant to layout).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LabelGeometry {
    /// `dimen`: label size in points, without node margins.
    pub dimen: Point,
    /// `space`: room available to the label inside the node, in points.
    pub space: Point,
    /// `valign`: `'t'`, `'b'` or `'c'`.
    pub valign: char,
    /// `true` for HTML-like labels.
    pub html: bool,
    /// Parsed, sized and positioned HTML label (`lp->u.html`); `None` for plain text or when
    /// C rejected the HTML (the label then has `dimen = (0, 0)`).
    pub html_label: Option<Box<HtmlLabel>>,
    /// Measured lines of a plain-text label (`u.txt.span`); `None` for HTML and records.
    pub text: Option<TextLabelLayout>,
    /// `make_html_label` failed (`html` is set, `html_label` is `None`): C then clears
    /// `lp->html`, so emitting and `-Tplain` treat the label as plain text with the
    /// unconverted source as its text and no spans.
    pub html_failed: bool,
    /// `fontname`, `fontsize` used for the label.
    pub font_name: String,
    pub font_size: f64,
}

/// Shape geometry produced by `poly_init`/`point_init` (`polygon_t`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PolygonGeometry {
    pub regular: bool,
    pub peripheries: usize,
    pub sides: usize,
    pub orientation: f64,
    pub distortion: f64,
    pub skew: f64,
    /// `option.fixedshape` (`fixedsize=shape`).
    pub fixedshape: bool,
    /// `sides * outp` vertices in points relative to the node centre: periphery `j` occupies
    /// `vertices[j * sides .. (j + 1) * sides]`; the last ring (if present) is the outline at
    /// half the pen width.
    pub vertices: Vec<Point>,
}

/// Everything C computes for a node before layout.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeGeometry {
    pub shape: ShapeKind,
    /// `ND_width`, `ND_height` in inches.
    pub width: f64,
    pub height: f64,
    /// `ND_outline_width`, `ND_outline_height` in inches (outermost periphery + pen width).
    pub outline_width: f64,
    pub outline_height: f64,
    /// `ND_label`.
    pub label: LabelGeometry,
    /// `ND_xlabel` dimen (points), if the node has a non-empty `xlabel`.
    pub xlabel: Option<LabelGeometry>,
    /// `ND_shape_info` for polygon and point shapes.
    pub polygon: Option<PolygonGeometry>,
    /// `ND_shape_info` for records: field tree positioned around `(0, 0)`.
    pub record: Option<RecordField>,
    /// `ND_shape_info` for `epsf`: set only when the `shapefile` loaded (`epsf_t.offset`,
    /// in points, and `macro_id`). Without it, `epsf_gencode` draws nothing.
    pub epsf: Option<(Point, i32)>,
    /// `ND_shape(n)->usershape`: the shape is `custom` (a `shapefile`, or the name).
    pub usershape: bool,
}

impl NodeGeometry {
    /// `gv_nodesize`: `(ND_lw, ND_rw, ND_ht)` in points. `flip` is `GD_flip(g)` (rankdir
    /// `LR`/`RL` in dot).
    pub fn nodesize(&self, flip: bool) -> (f64, f64, f64) {
        if flip {
            let w = inch2ps(self.height);
            (w / 2.0, w / 2.0, inch2ps(self.width))
        } else {
            let w = inch2ps(self.width);
            (w / 2.0, w / 2.0, inch2ps(self.height))
        }
    }
}

/// `getPenColor` (htmltable.c): the object's `pencolor`, else its `color`.
fn get_pen_color(attrs: &AttrMap) -> Option<&str> {
    ["pencolor", "color"]
        .into_iter()
        .filter_map(|k| attrs.get(k))
        .map(String::as_str)
        .find(|s| !s.is_empty())
}

/// `lp->text` of a non-record label made by [`make_label`] from attribute value `raw`: the
/// substituted, charset-converted text of a plain label, the source of an HTML label, or
/// `"<TABLE>"` for an HTML table.
pub fn label_text(l: &LabelGeometry, raw: &str) -> String {
    label_l_text(l, raw, false).to_string()
}

/// `lp->text` of a made label, which a later label's `\L` expands to (`make_label`):
/// the raw string for a record, the substituted text for plain text, the HTML source
/// for HTML text and `<TABLE>` for a table (`make_html_label`).
fn label_l_text<'a>(l: &'a LabelGeometry, raw: &'a str, is_record: bool) -> &'a str {
    if is_record {
        return raw;
    }
    if let Some(t) = &l.text {
        return &t.source;
    }
    if l.html {
        if matches!(l.html_label.as_deref(), Some(HtmlLabel::Table(_))) {
            return "<TABLE>";
        }
        return raw;
    }
    ""
}

/// `make_label` for the plain-text and HTML cases: returns the label geometry with
/// `space = dimen`. `obj` describes the owning object for escape substitution; `is_html` is
/// `aghtmlstr(str)` ([`graphviz_pure_core::AttrMap::is_html`] for an attribute value), and
/// `label` is then the HTML string's inner text.
pub fn make_label(
    label: &str,
    is_html: bool,
    obj: &LabelSubstContext<'_>,
    font_name: &str,
    font_size: f64,
    charset: Charset,
) -> LabelGeometry {
    // disregard HTML intent for empty string labels
    let html = is_html && !label.is_empty();
    if html {
        let lay = html_label_layout(label, obj, font_name, font_size, charset);
        let dimen = lay.dimen;
        return LabelGeometry {
            dimen,
            // only make_simple_label sets lp->space; an HTML label's stays (0, 0)
            // unless the shape sets it (poly_init, except fixedsize=shape).
            space: Point::default(),
            valign: 'c',
            html: true,
            html_failed: lay.label.is_none(),
            html_label: lay.label.map(Box::new),
            text: None,
            font_name: font_name.to_string(),
            font_size,
        };
    }
    let layout = make_text_label(label, obj, font_name, font_size, charset);
    let dimen = Point::new(layout.width, layout.height);
    LabelGeometry {
        dimen,
        space: dimen,
        valign: 'c',
        html: false,
        html_label: None,
        html_failed: false,
        text: Some(layout),
        font_name: font_name.to_string(),
        font_size,
    }
}

/// Font of a node label (`common_init_node`): `(fontname, fontsize)`.
pub fn node_font(attrs: &AttrMap) -> (&str, f64) {
    (
        late_nnstring(attrs, "fontname", DEFAULT_FONT_NAME),
        late_double(attrs, "fontsize", DEFAULT_FONT_SIZE, MIN_FONT_SIZE),
    )
}

/// `common_init_node` + shape `initfn`: the exact size and label geometry of `node`.
///
/// The shape's `initfn` allocates the vertices of the node's peripheries, and C's
/// `gv_calloc` reports a failed allocation and exits; the port returns the error.
pub fn node_geometry(node: &NodeData, env: &NodeSizeEnv<'_>) -> Result<NodeGeometry> {
    // `<G>` and `G` are two nodes: the tag keeps the HTML flag, as the node lookup does
    let key = graphviz_pure_core::Graph::node_key(&node.name, node.html_name);
    graphviz_pure_core::font::textfont_object(format!("node:{key}"), || {
        node_geometry_measured(node, env)
    })
}

fn node_geometry_measured(node: &NodeData, env: &NodeSizeEnv<'_>) -> Result<NodeGeometry> {
    let attrs = &node.attrs;
    let shape_name = late_nnstring(attrs, "shape", "ellipse");
    // bind_shape: a `shapefile` that `safefile` accepts (except with epsf) makes the shape
    // `custom`, a user shape that copies `Shapes[0]` (box).
    let custom = usershape::binds_custom(shape_name, attrs.get("shapefile").map(String::as_str));
    let unknown = !custom
        && ShapeKind::parse(shape_name) == ShapeKind::Box
        && !matches!(shape_name, "box" | "rect" | "rectangle");
    // `ND_shape(n)->usershape`: `custom`, by shapefile or by name.
    let usershape = custom || shape_name == "custom";
    // (once, as C binds each node's shape once: not in the port's repeat sizings)
    if graphviz_pure_core::diag::reporting() {
        if usershape {
            usershape::register_user_shape("custom");
        } else if unknown {
            usershape::register_user_shape(shape_name);
        }
    }
    // user_shape (shapes.c:3955-3962): a name outside the Shapes table becomes a copy of
    // `box`, with a warning the first time the name is seen.
    if unknown
        && shape_name != "custom"
        && graphviz_pure_core::diag::reporting()
        && graphviz_pure_core::diag::emit_once(&format!("\0shape\0{shape_name}"))
    {
        graphviz_pure_core::diag::agwarningf(&format!(
            "using box for unknown shape {shape_name}\n"
        ));
    }
    let shape = if custom {
        ShapeKind::Box
    } else {
        ShapeKind::parse(shape_name)
    };
    let (init, desc) = shape.shape_desc();
    let nd_width = late_double(attrs, "width", DEFAULT_NODEWIDTH, MIN_NODEWIDTH);
    let nd_height = late_double(attrs, "height", DEFAULT_NODEHEIGHT, MIN_NODEHEIGHT);
    let (font_name, font_size) = node_font(attrs);
    let label_str = attrs.get("label").map_or("\\N", String::as_str);
    let obj = LabelSubstContext::for_node(&node.name);
    let obj = LabelSubstContext {
        graph_name: env.graph_name,
        in_buffer: env.names_in_buffer(&node.name, "", ""),
        pen_color: get_pen_color(attrs),
        font_color: Some(late_nnstring(attrs, "fontcolor", "black")),
        ..obj
    };

    let label = if init == ShapeInit::Record {
        // `make_label(..., is_record = true, ...)` stores the raw text; fields are labelled by
        // `parse_reclbl`.
        LabelGeometry {
            dimen: Point::default(),
            space: Point::default(),
            valign: 'c',
            html: attrs.is_html("label"),
            html_label: None,
            html_failed: false,
            text: None,
            font_name: font_name.to_string(),
            font_size,
        }
    } else {
        make_label(
            label_str,
            attrs.is_html("label"),
            &obj,
            font_name,
            font_size,
            env.charset,
        )
    };
    // common_init_node makes ND_label first, so the xlabel's `\L` is its text.
    let xlabel = attrs.get("xlabel").filter(|s| !s.is_empty()).map(|s| {
        let xobj = LabelSubstContext {
            label_text: label_l_text(&label, label_str, init == ShapeInit::Record),
            has_label: true,
            ..obj.clone()
        };
        let html = attrs.is_html("xlabel");
        make_label(s, html, &xobj, font_name, font_size, env.charset)
    });

    let mut geom = NodeGeometry {
        shape,
        width: nd_width,
        height: nd_height,
        outline_width: nd_width,
        outline_height: nd_height,
        label,
        xlabel,
        polygon: None,
        record: None,
        epsf: None,
        usershape,
    };
    match init {
        ShapeInit::Poly => {
            let desc = desc.unwrap_or(pd(false, 1, 4));
            poly_init(node, shape, desc, usershape, env, &mut geom)?;
        }
        ShapeInit::Epsf => epsf_init(node, &mut geom),
        ShapeInit::Point => point_init(node, desc.map_or(1, |d| d.peripheries), &mut geom)?,
        ShapeInit::Record => {
            let xobj = LabelSubstContext {
                label_text: label_str,
                has_label: true,
                ..obj
            };
            record_init(node, label_str, &xobj, env, &mut geom);
        }
    }
    Ok(geom)
}

/// `GD_label_pos` flags (`lib/common/const.h`).
pub const LABEL_AT_BOTTOM: u8 = 0;
pub const LABEL_AT_TOP: u8 = 1;
pub const LABEL_AT_LEFT: u8 = 2;
pub const LABEL_AT_RIGHT: u8 = 4;

/// Index into `GD_border` (`BOTTOM_IX`, `RIGHT_IX`, `TOP_IX`, `LEFT_IX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderSide {
    Bottom = 0,
    Right = 1,
    Top = 2,
    Left = 3,
}

/// Graph or cluster label (`do_graph_label` in `lib/common/input.c`).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphLabelGeometry {
    /// `GD_label`: `dimen` is what `-Tdot` prints as `lwidth`/`lheight` (in inches, `%.2f`).
    pub label: LabelGeometry,
    /// `GD_label_pos` (`LABEL_AT_*` flags).
    pub pos_flag: u8,
    /// Clusters only: the `GD_border` entry reserved for the label (`dimen` + `PAD`, swapped
    /// when the root graph is flipped).
    pub border: Option<(BorderSide, Point)>,
}

/// `do_graph_label`: the label of the root graph (`is_root`) or of a cluster named `name`.
/// `attrs` are the graph's effective attributes; `flip` is `GD_flip(root)` (`rankdir` LR/RL in
/// dot). Returns `None` when there is no (non-empty) label.
pub fn graph_label_geometry(
    name: &str,
    attrs: &AttrMap,
    is_root: bool,
    flip: bool,
    charset: Charset,
) -> Option<GraphLabelGeometry> {
    let env = NodeSizeEnv {
        charset,
        ..NodeSizeEnv::new(name)
    };
    graph_label_geometry_in(name, false, attrs, is_root, flip, &env)
}

/// [`graph_label_geometry`] with HTML strings and charset taken from `env`. `html_name`: the
/// graph is named by an HTML string, which makes it another object than the graph named by
/// the same plain text.
pub fn graph_label_geometry_in(
    name: &str,
    html_name: bool,
    attrs: &AttrMap,
    is_root: bool,
    flip: bool,
    env: &NodeSizeEnv<'_>,
) -> Option<GraphLabelGeometry> {
    let key = graphviz_pure_core::Graph::node_key(name, html_name);
    let tag = format!("{}:{key}", if is_root { "root" } else { "graph" });
    graphviz_pure_core::font::textfont_object(tag, || {
        graph_label_geometry_measured(name, attrs, is_root, flip, env)
    })
}

fn graph_label_geometry_measured(
    name: &str,
    attrs: &AttrMap,
    is_root: bool,
    flip: bool,
    env: &NodeSizeEnv<'_>,
) -> Option<GraphLabelGeometry> {
    let charset = env.charset;
    let s = attrs.get("label").filter(|s| !s.is_empty())?;
    let obj = LabelSubstContext {
        graph_name: name,
        pen_color: get_pen_color(attrs),
        font_color: Some(late_nnstring(attrs, "fontcolor", "black")),
        ..LabelSubstContext::default()
    };
    let label = make_label(
        s,
        attrs.is_html("label"),
        &obj,
        late_nnstring(attrs, "fontname", DEFAULT_FONT_NAME),
        late_double(attrs, "fontsize", DEFAULT_FONT_SIZE, MIN_FONT_SIZE),
        charset,
    );
    let pos = attrs.get("labelloc").and_then(|p| p.bytes().next());
    let mut pos_flag = if is_root {
        if pos == Some(b't') {
            LABEL_AT_TOP
        } else {
            LABEL_AT_BOTTOM
        }
    } else if pos == Some(b'b') {
        LABEL_AT_BOTTOM
    } else {
        LABEL_AT_TOP
    };
    match attrs.get("labeljust").and_then(|p| p.bytes().next()) {
        Some(b'l') => pos_flag |= LABEL_AT_LEFT,
        Some(b'r') => pos_flag |= LABEL_AT_RIGHT,
        _ => {}
    }
    let border = (!is_root).then(|| {
        let d = Point::new(label.dimen.x + 16.0, label.dimen.y + 8.0);
        let top = pos_flag & LABEL_AT_TOP != 0;
        if flip {
            (
                if top {
                    BorderSide::Right
                } else {
                    BorderSide::Left
                },
                Point::new(d.y, d.x),
            )
        } else {
            (
                if top {
                    BorderSide::Top
                } else {
                    BorderSide::Bottom
                },
                d,
            )
        }
    });
    Some(GraphLabelGeometry {
        label,
        pos_flag,
        border,
    })
}

/// Edge labels made by `common_init_edge` (`lib/common/utils.c`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgeLabelGeometry {
    /// `ED_label` (`label`, font `fontname`/`fontsize`).
    pub label: Option<LabelGeometry>,
    /// `ED_xlabel`.
    pub xlabel: Option<LabelGeometry>,
    /// `ED_head_label` / `ED_tail_label` (`labelfontname`/`labelfontsize`, defaulting to the
    /// edge font).
    pub head_label: Option<LabelGeometry>,
    pub tail_label: Option<LabelGeometry>,
    /// `ED_label_ontop` (`labelfloat`).
    pub label_float: bool,
}

/// [`edge_label_geometry`] for the edge `id`, whose fonts are interned once per layout
/// (`font::textfont_object`).
pub fn edge_label_geometry_of(
    id: u64,
    attrs: &AttrMap,
    tail_name: &str,
    head_name: &str,
    directed: bool,
    env: &NodeSizeEnv<'_>,
) -> EdgeLabelGeometry {
    let tag = format!("edge:{tail_name}->{head_name}#{id}");
    graphviz_pure_core::font::textfont_object(tag, || {
        edge_label_geometry(attrs, tail_name, head_name, directed, env)
    })
}

/// `common_init_edge`: sizes the labels of an edge with effective attributes `attrs`
/// between nodes `tail_name` and `head_name` of root graph `env.graph_name`.
pub fn edge_label_geometry(
    attrs: &AttrMap,
    tail_name: &str,
    head_name: &str,
    directed: bool,
    env: &NodeSizeEnv<'_>,
) -> EdgeLabelGeometry {
    let fontsize = late_double(attrs, "fontsize", DEFAULT_FONT_SIZE, MIN_FONT_SIZE);
    let fontname = late_nnstring(attrs, "fontname", DEFAULT_FONT_NAME);
    let lfontsize = late_double(attrs, "labelfontsize", fontsize, MIN_FONT_SIZE);
    let lfontname = late_nnstring(attrs, "labelfontname", fontname);
    let fontcolor = late_nnstring(attrs, "fontcolor", "black");
    let lfontcolor = late_nnstring(attrs, "labelfontcolor", fontcolor);
    // Ports are bound after the labels are made, so `\E` never shows them.
    let mut obj = LabelSubstContext {
        graph_name: env.graph_name,
        tail_name,
        head_name,
        in_buffer: env.names_in_buffer("", tail_name, head_name),
        is_edge: true,
        is_directed: directed,
        pen_color: get_pen_color(attrs),
        font_color: Some(fontcolor),
        ..LabelSubstContext::default()
    };
    let mk = |key: &str, obj: &LabelSubstContext<'_>, name: &str, size: f64| {
        attrs
            .get(key)
            .filter(|s| !s.is_empty())
            .map(|s| make_label(s, attrs.is_html(key), obj, name, size, env.charset))
    };
    let label = mk("label", &obj, fontname, fontsize);
    // Later labels see `\L` = the main label's `lp->text`.
    let ltext = match (&label, attrs.get("label")) {
        (Some(l), Some(raw)) => label_l_text(l, raw, false),
        _ => "",
    };
    obj.label_text = ltext;
    obj.has_label = label.is_some();
    let lobj = LabelSubstContext {
        font_color: Some(lfontcolor),
        ..obj.clone()
    };
    EdgeLabelGeometry {
        xlabel: mk("xlabel", &obj, fontname, fontsize),
        head_label: mk("headlabel", &lobj, lfontname, lfontsize),
        tail_label: mk("taillabel", &lobj, lfontname, lfontsize),
        label,
        label_float: mapbool(Some(
            attrs.get("labelfloat").map_or("false", String::as_str),
        )),
    }
}

/// Sizes every node of `graph` (sets `NodeData::width`/`height` to `ND_width`/`ND_height`),
/// reading `rankdir`, `quantum` and `charset` from the root graph.
pub fn size_graph_nodes(graph: &mut Graph) -> Result<()> {
    let env_attrs = graph.graph_attrs.clone();
    let name = graph.root_name().to_string();
    let env = NodeSizeEnv {
        graph_name_buffer: graph.agnameof(graphviz_pure_core::Obj::Root).buffer,
        local_node_names_kept: graph.local_node_names_kept,
        ..NodeSizeEnv::from_attrs(&name, &env_attrs)
    };
    for node in &mut graph.nodes {
        let geom = node_geometry(node, &env)?;
        node.width = geom.width;
        node.height = geom.height;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// poly_init
// ---------------------------------------------------------------------------------------------

/// `star_size` from `shapes.c`.
fn star_size(sz0: Point) -> Point {
    let alpha = PI / 10.0;
    let alpha2 = 2.0 * alpha;
    let alpha3 = 3.0 * alpha;
    let alpha4 = 2.0 * alpha2;
    let rx = sz0.x / (2.0 * math::cos(alpha));
    let ry = sz0.y / (math::sin(alpha) + math::sin(alpha3));
    let r0 = rx.max(ry);
    let r = r0 * math::sin(alpha4) * math::cos(alpha2) / (math::cos(alpha) * math::cos(alpha4));
    Point::new(2.0 * r * math::cos(alpha), r * (1.0 + math::sin(alpha3)))
}

/// `star_vertices` from `shapes.c`: fills 10 vertices and adjusts `bb` to the star aspect.
fn star_vertices(vertices: &mut [Point], bb: &mut Point) {
    let alpha = PI / 10.0;
    let alpha2 = 2.0 * alpha;
    let alpha3 = 3.0 * alpha;
    let alpha4 = 2.0 * alpha2;
    let mut sz = *bb;
    let aspect = (1.0 + math::sin(alpha3)) / (2.0 * math::cos(alpha));
    let a = sz.y / sz.x;
    if a > aspect {
        sz.x = sz.y / aspect;
    } else if a < aspect {
        sz.y = sz.x * aspect;
    }
    let r = sz.x / (2.0 * math::cos(alpha));
    let r0 = r * math::cos(alpha) * math::cos(alpha4) / (math::sin(alpha4) * math::cos(alpha2));
    let offset = (r * (1.0 - math::sin(alpha3))) / 2.0;
    let mut theta = alpha;
    for i in (0..10).step_by(2) {
        // C (clang) fuses `r * sin(theta) - offset`.
        vertices[i] = Point::new(
            r * math::cos(theta),
            math::fma(r, math::sin(theta), -offset),
        );
        theta += alpha2;
        vertices[i + 1] = Point::new(
            r0 * math::cos(theta),
            math::fma(r0, math::sin(theta), -offset),
        );
        theta += alpha2;
    }
    *bb = sz;
}

/// `cylinder_size` from `shapes.c`.
fn cylinder_size(mut sz: Point) -> Point {
    sz.y *= 1.375;
    sz
}

/// `cylinder_vertices` from `shapes.c` (19 vertices).
fn cylinder_vertices(v: &mut [Point], bb: &mut Point) {
    let x = bb.x / 2.0;
    let y = bb.y / 2.0;
    let yr = bb.y / 11.0;
    v[0] = Point::new(x, y - yr);
    // C (clang) fuses `y - (1 - 0.551784) * yr`.
    v[1] = Point::new(x, math::fma(-(1.0 - 0.551784f64), yr, y));
    v[2] = Point::new(0.551784 * x, y);
    v[3] = Point::new(0.0, y);
    v[4] = Point::new(-0.551784 * x, y);
    v[5] = Point::new(-x, v[1].y);
    v[6] = Point::new(-x, y - yr);
    v[7] = v[6];
    v[8] = Point::new(-x, yr - y);
    v[9] = v[8];
    v[10] = Point::new(-x, -v[1].y);
    v[11] = Point::new(v[4].x, -v[4].y);
    v[12] = Point::new(v[3].x, -v[3].y);
    v[13] = Point::new(v[2].x, -v[2].y);
    v[14] = Point::new(v[1].x, -v[1].y);
    v[15] = Point::new(v[0].x, -v[0].y);
    v[16] = v[15];
    v[17] = v[0];
    v[18] = v[0];
}

/// `quant` from `shapes.c`.
fn quant(val: f64, q: f64) -> f64 {
    (val / q).ceil() * q
}

/// Applies the node `margin` (or `PAD`) to a label size, as `poly_init` and `size_reclbl` do.
/// `clamp` reproduces poly_init's `fmax(margin, 0)` (record fields do not clamp).
fn pad_dimen(attrs: &AttrMap, mut dimen: Point, clamp: bool) -> Point {
    let margin = attrs.get("margin").map(String::as_str).unwrap_or("");
    let r = sscanf(margin, "%lf,%lf");
    let (i, [mut mx, mut my, ..]) = (r.count, r.values);
    if clamp {
        mx = mx.max(0.0);
        my = my.max(0.0);
    }
    if i > 0 {
        dimen.x += 2.0 * inch2ps(mx);
        if i > 1 {
            dimen.y += 2.0 * inch2ps(my);
        } else {
            dimen.y += 2.0 * inch2ps(mx);
        }
    } else {
        // PAD(d): XPAD = 4 * GAP, YPAD = 2 * GAP
        dimen.x += 4.0 * GAP;
        dimen.y += 2.0 * GAP;
    }
    dimen
}

/// `epsf_init`: a node whose `shapefile` has a `%%BoundingBox` takes its size, and the
/// offset `epsf_gencode` draws it at; otherwise it keeps `width` and `height` and no shape
/// information.
fn epsf_init(node: &NodeData, geom: &mut NodeGeometry) {
    let sfile = node.attrs.get("shapefile").map(String::as_str);
    let Some(us) = usershape::epsf_init(sfile, &node.name) else {
        return;
    };
    let [lx, ly, ux, uy] = us.bb;
    let dx = f64::from(ux) - f64::from(lx);
    let dy = f64::from(uy) - f64::from(ly);
    geom.width = ps2inch(dx);
    geom.height = ps2inch(dy);
    geom.outline_width = geom.width;
    geom.outline_height = geom.height;
    geom.epsf = Some((
        Point::new(-f64::from(lx) - dx / 2.0, -f64::from(ly) - dy / 2.0),
        us.macro_id,
    ));
}

fn poly_init(
    node: &NodeData,
    shape: ShapeKind,
    desc: PolygonDesc,
    usershape: bool,
    env: &NodeSizeEnv<'_>,
    geom: &mut NodeGeometry,
) -> Result<()> {
    let attrs = &node.attrs;
    let is_plain = shape == ShapeKind::Plain;
    let mut regular = desc.regular;
    let mut peripheries = desc.peripheries;
    let mut sides = desc.sides;
    let mut orientation = desc.orientation;
    let mut skew = desc.skew;
    let mut distortion = desc.distortion;
    regular |= mapbool(attrs.get("regular").map(String::as_str));

    let (mut width, mut height);
    if is_plain {
        width = 0.0;
        height = 0.0;
    } else if regular {
        // userSize
        let w = late_double(attrs, "width", 0.0, MIN_NODEWIDTH);
        let h = late_double(attrs, "height", 0.0, MIN_NODEHEIGHT);
        let sz = inch2ps(w.max(h));
        if sz > 0.0 {
            width = sz;
            height = sz;
        } else {
            width = inch2ps(geom.width.min(geom.height));
            height = width;
        }
    } else {
        width = inch2ps(geom.width);
        height = inch2ps(geom.height);
    }

    peripheries = late_int(attrs, "peripheries", peripheries as i32, 0) as usize;
    orientation += late_double(attrs, "orientation", 0.0, -360.0);
    if sides == 0 {
        skew = late_double(attrs, "skew", 0.0, -100.0);
        sides = late_int(attrs, "sides", 4, 0) as usize;
        distortion = late_double(attrs, "distortion", 0.0, -100.0);
    }

    let label_dimen = geom.label.dimen;
    let mut dimen = label_dimen;
    if (dimen.x > 0.0 || dimen.y > 0.0) && !is_plain {
        dimen = pad_dimen(attrs, dimen, true);
    }
    let spacex = dimen.x - label_dimen.x;

    if env.quantum > 0.0 {
        let q = inch2ps(env.quantum);
        dimen.x = quant(dimen.x, q);
        dimen.y = quant(dimen.y, q);
    }

    // `gvusershape_size`, plus some fixed padding; a file C cannot load is reported, and
    // keeps imagesize (0, 0)
    let image = |sfile: Option<&str>, attr: &str| {
        let sz = usershape::gvusershape_size(sfile);
        if sz == (-1, -1) {
            if graphviz_pure_core::diag::reporting() {
                graphviz_pure_core::diag::agwarningf(&format!(
                    "No or improper {attr}=\"{}\" for node \"{}\"\n",
                    sfile.unwrap_or("<nil>"),
                    node.name
                ));
            }
            Point::new(0.0, 0.0)
        } else {
            // `imagesize` is a `point`: the padding is an int sum
            Point::new(
                f64::from(sz.0.wrapping_add(2)),
                f64::from(sz.1.wrapping_add(2)),
            )
        }
    };
    let imagesize = if usershape {
        // custom requires a shapefile
        image(
            usershape::agget_shapefile(attrs.get("shapefile").map(String::as_str)),
            "shapefile",
        )
    } else if let Some(sfile) = attrs.get("image").filter(|f| !f.is_empty()) {
        image(Some(sfile), "image")
    } else {
        Point::new(0.0, 0.0)
    };
    let mut bb = Point::new(dimen.x.max(imagesize.x), dimen.y.max(imagesize.y));

    if sides <= 2 && (!is_exactly_zero(distortion) || !is_exactly_zero(skew)) {
        sides = 120;
    }

    let valign = match attrs.get("labelloc").and_then(|p| p.bytes().next()) {
        Some(b't') => 't',
        Some(b'b') => 'b',
        _ => 'c',
    };
    geom.label.valign = valign;

    let is_box = sides == 4
        && (orientation % 90.0).abs() < 0.5
        && is_exactly_zero(distortion)
        && is_exactly_zero(skew);
    let has_gen = matches!(shape, ShapeKind::Star | ShapeKind::Cylinder);
    if is_box {
        // exact fit
    } else if has_gen {
        bb = if shape == ShapeKind::Star {
            star_size(bb)
        } else {
            cylinder_size(bb)
        };
    } else {
        let temp = bb.y * SQRT2;
        if height > temp && valign == 'c' {
            // clang contracts `1. - SQR(r)` into a fused multiply-subtract.
            let r = bb.y / height;
            bb.x *= (1.0 / math::fma(-r, r, 1.0)).sqrt();
        } else {
            bb.x *= SQRT2;
            bb.y = temp;
        }
        if sides > 2 {
            let temp = math::cos(PI / sides as f64);
            bb.x /= temp;
            bb.y /= temp;
        }
    }

    let min_bb = bb;
    let mut fixedshape = false;
    let fxd = attrs.get("fixedsize").map_or("false", String::as_str);
    if fxd == "shape" {
        bb = Point::new(width, height);
        fixedshape = true;
    } else if mapbool(Some(fxd)) {
        // check only label, as images we can scale to fit
        if (width < label_dimen.x || height < label_dimen.y)
            && graphviz_pure_core::diag::reporting()
        {
            // shapes.c:2115-2118; agraphof(n) is the root graph. Both names are agnameof's,
            // which keeps anonymous ones in one static buffer.
            let names = env.names_in_buffer(&node.name, "", "");
            let [n, g] = graphviz_pure_core::agnameof_buffer([
                AgName {
                    name: node.name.as_str().into(),
                    buffer: names.node,
                },
                AgName {
                    name: env.graph_name.into(),
                    buffer: names.graph,
                },
            ]);
            graphviz_pure_core::diag::agwarningf(&format!(
                "node '{n}', graph '{g}' size too small for label\n"
            ));
        }
        bb = Point::new(width, height);
    } else {
        width = width.max(bb.x);
        height = height.max(bb.y);
        bb = Point::new(width, height);
    }

    if regular {
        let m = bb.x.max(bb.y);
        width = m;
        height = m;
        bb = Point::new(m, m);
    }

    if !mapbool(Some(attrs.get("nojustify").map_or("false", String::as_str))) {
        if is_box {
            geom.label.space.x = dimen.x.max(bb.x) - spacex;
        } else if dimen.y < bb.y {
            let temp = bb.x * (1.0 - (dimen.y * dimen.y) / (bb.y * bb.y)).sqrt();
            geom.label.space.x = dimen.x.max(temp) - spacex;
        } else {
            geom.label.space.x = dimen.x - spacex;
        }
    } else {
        geom.label.space.x = dimen.x - spacex;
    }
    if !fixedshape {
        let mut temp = bb.y - min_bb.y;
        if dimen.y < imagesize.y {
            temp += imagesize.y - dimen.y;
        }
        geom.label.space.y = dimen.y + temp;
    }

    let penwidth = late_double(attrs, "penwidth", DEFAULT_NODEPENWIDTH, MIN_NODEPENWIDTH);
    let mut outp = peripheries.max(1);
    if peripheries >= 1 && penwidth > 0.0 {
        outp += 1;
    }

    let mut vertices;
    let mut outline_bb;
    if sides < 3 {
        sides = 2;
        // `gv_calloc(outp * sides, sizeof(pointf))`
        vertices = gv_calloc_vec::<Point>(element_count(outp, sides))?;
        let mut p = Point::new(bb.x / 2.0, bb.y / 2.0);
        vertices[0] = Point::new(-p.x, -p.y);
        vertices[1] = p;
        if peripheries > 1 {
            let mut i = 2;
            for _ in 1..peripheries {
                p.x += GAP;
                p.y += GAP;
                vertices[i] = Point::new(-p.x, -p.y);
                vertices[i + 1] = p;
                i += 2;
            }
            bb = Point::new(2.0 * p.x, 2.0 * p.y);
        }
        outline_bb = bb;
        if outp > peripheries {
            p.x += penwidth / 2.0;
            p.y += penwidth / 2.0;
            let i = sides * peripheries;
            vertices[i] = Point::new(-p.x, -p.y);
            vertices[i + 1] = p;
            outline_bb = Point::new(2.0 * p.x, 2.0 * p.y);
        }
    } else {
        // `gv_calloc(outp * sides, sizeof(pointf))`
        vertices = gv_calloc_vec::<Point>(element_count(outp, sides))?;
        let (mut sinx, mut cosx) = (0.0_f64, 0.0_f64);
        let (mut xmax, mut ymax);
        if has_gen {
            if shape == ShapeKind::Star {
                star_vertices(&mut vertices, &mut bb);
            } else {
                cylinder_vertices(&mut vertices, &mut bb);
            }
            xmax = bb.x / 2.0;
            ymax = bb.y / 2.0;
        } else {
            let sectorangle = 2.0 * PI / sides as f64;
            let sidelength = math::sin(sectorangle / 2.0);
            let skewdist = math::hypot(distortion.abs() + skew.abs(), 1.0);
            let gdistortion = distortion * SQRT2 / math::cos(sectorangle / 2.0);
            let gskew = skew / 2.0;
            let mut angle = (sectorangle - PI) / 2.0;
            sinx = math::sin(angle);
            cosx = math::cos(angle);
            let mut r = Point::new(0.5 * cosx, 0.5 * sinx);
            xmax = 0.0;
            ymax = 0.0;
            angle += (PI - sectorangle) / 2.0;
            for i in 0..sides {
                angle += sectorangle;
                sinx = math::sin(angle);
                cosx = math::cos(angle);
                // C (clang, -ffp-contract=on) fuses these multiply-adds; see
                // the `fmadd`s in poly_init of libgvc.
                r.x = math::fma(sidelength, cosx, r.x);
                r.y = math::fma(sidelength, sinx, r.y);
                let mut p = Point::new(
                    math::fma(r.x, math::fma(r.y, gdistortion, skewdist), r.y * gskew),
                    r.y,
                );
                // RADIANS(orientation) + atan2(..) = (o / 180) * M_PI + atan2(..), fused.
                let alpha = math::fma(orientation / 180.0, PI, math::atan2(p.y, p.x));
                sinx = math::sin(alpha);
                cosx = math::cos(alpha);
                let h = math::hypot(p.x, p.y);
                p.x = h * cosx;
                p.y = h * sinx;
                p.x *= bb.x;
                p.y *= bb.y;
                xmax = p.x.abs().max(xmax);
                ymax = p.y.abs().max(ymax);
                vertices[i] = p;
                if is_box {
                    vertices[1] = Point::new(-p.x, p.y);
                    vertices[2] = Point::new(-p.x, -p.y);
                    vertices[3] = Point::new(p.x, -p.y);
                    break;
                }
            }
        }

        xmax *= 2.0;
        ymax *= 2.0;
        bb = Point::new(width.max(xmax), height.max(ymax));
        outline_bb = bb;
        let scalex = bb.x / xmax;
        let scaley = bb.y / ymax;
        for v in vertices.iter_mut().take(sides) {
            v.x *= scalex;
            v.y *= scaley;
        }

        if outp > 1 {
            let i0 = sides; // C's `i` after the scaling loop
            let mut r = vertices[0];
            let mut q = r;
            for j in 1..sides {
                q = vertices[(i0 - j) % sides];
                if !is_exactly_equal(q.x, r.x) || !is_exactly_equal(q.y, r.y) {
                    break;
                }
            }
            let mut beta = math::atan2(r.y - q.y, r.x - q.x);
            let mut qprev = q;
            for i in 0..sides {
                q = vertices[i];
                if !(is_exactly_equal(q.x, qprev.x) && is_exactly_equal(q.y, qprev.y)) {
                    for j in 1..sides {
                        r = vertices[(i + j) % sides];
                        if !is_exactly_equal(r.x, q.x) || !is_exactly_equal(r.y, q.y) {
                            break;
                        }
                    }
                    let alpha = beta;
                    beta = math::atan2(r.y - q.y, r.x - q.x);
                    let gamma = (alpha + PI - beta) / 2.0;
                    let temp = GAP / math::sin(gamma);
                    sinx = math::sin(alpha - gamma) * temp;
                    cosx = math::cos(alpha - gamma) * temp;
                }
                qprev = q;
                for j in 1..peripheries {
                    q.x += cosx;
                    q.y += sinx;
                    vertices[i + j * sides] = q;
                }
                if outp > peripheries {
                    q.x += cosx * penwidth / 2.0 / GAP;
                    q.y += sinx * penwidth / 2.0 / GAP;
                    vertices[i + peripheries * sides] = q;
                }
            }
            for i in 0..sides {
                let p = vertices[i + peripheries.saturating_sub(1) * sides];
                bb = Point::new((2.0 * p.x.abs()).max(bb.x), (2.0 * p.y.abs()).max(bb.y));
                let q = vertices[i + (outp - 1) * sides];
                outline_bb = Point::new(
                    (2.0 * q.x.abs()).max(outline_bb.x),
                    (2.0 * q.y.abs()).max(outline_bb.y),
                );
            }
        }
    }

    if fixedshape {
        geom.width = ps2inch(dimen.x.max(bb.x));
        geom.height = ps2inch(dimen.y.max(bb.y));
        geom.outline_width = ps2inch(dimen.x.max(outline_bb.x));
        geom.outline_height = ps2inch(dimen.y.max(outline_bb.y));
    } else {
        geom.width = ps2inch(bb.x);
        geom.height = ps2inch(bb.y);
        geom.outline_width = ps2inch(outline_bb.x);
        geom.outline_height = ps2inch(outline_bb.y);
    }
    geom.polygon = Some(PolygonGeometry {
        regular,
        peripheries,
        sides,
        orientation,
        distortion,
        skew,
        fixedshape,
        vertices,
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// point_init
// ---------------------------------------------------------------------------------------------

fn point_init(node: &NodeData, default_peripheries: usize, geom: &mut NodeGeometry) -> Result<()> {
    let attrs = &node.attrs;
    let w = late_double(attrs, "width", f64::MAX, MIN_NODEWIDTH);
    let h = late_double(attrs, "height", f64::MAX, MIN_NODEHEIGHT);
    let wm = w.min(h);
    let nd = if w == f64::MAX && h == f64::MAX {
        DEF_POINT
    } else if wm > 0.0 {
        wm.max(MIN_POINT)
    } else {
        wm
    };
    let mut sz = nd * 72.0;
    let peripheries = late_int(attrs, "peripheries", default_peripheries as i32, 0) as usize;
    let mut outp = peripheries.max(1);
    let sides = 2;
    let penwidth = late_double(attrs, "penwidth", DEFAULT_NODEPENWIDTH, MIN_NODEPENWIDTH);
    if peripheries >= 1 && penwidth > 0.0 {
        outp += 1;
    }
    // `gv_calloc(outp * sides, sizeof(pointf))`
    let mut vertices = gv_calloc_vec::<Point>(element_count(outp, sides))?;
    let mut p = Point::new(sz / 2.0, sz / 2.0);
    vertices[0] = Point::new(-p.x, -p.y);
    vertices[1] = p;
    let mut i;
    if peripheries > 1 {
        i = 2;
        for _ in 1..peripheries {
            p.x += GAP;
            p.y += GAP;
            vertices[i] = Point::new(-p.x, -p.y);
            vertices[i + 1] = p;
            i += 2;
        }
        sz = 2.0 * p.x;
    } else {
        i = sides;
    }
    if peripheries >= 1 && penwidth > 0.0 && outp > peripheries {
        p.x += penwidth / 2.0;
        p.y += penwidth / 2.0;
        vertices[i] = Point::new(-p.x, -p.y);
        vertices[i + 1] = p;
    }
    let sz_outline = 2.0 * p.x;
    geom.width = ps2inch(sz);
    geom.height = geom.width;
    geom.outline_width = ps2inch(sz_outline);
    geom.outline_height = geom.outline_width;
    geom.polygon = Some(PolygonGeometry {
        regular: true,
        peripheries,
        sides: 2,
        orientation: 0.0,
        distortion: 0.0,
        skew: 0.0,
        fixedshape: false,
        vertices,
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// record_init and helpers
// ---------------------------------------------------------------------------------------------

const HASTEXT: u8 = 1;
const HASPORT: u8 = 2;
const HASTABLE: u8 = 4;
const INTEXT: u8 = 8;
const INPORT: u8 = 16;

fn is_ctrl(c: u8) -> bool {
    matches!(c, b'{' | b'}' | b'|' | b'<' | b'>')
}

/// A record field as built by `parse_reclbl` (`field_t`).
#[derive(Debug, Clone, Default)]
struct Field {
    id: Option<String>,
    /// Field label text (`fp->lp`) for leaf fields.
    text: Option<String>,
    lp: Option<LabelGeometry>,
    lr: bool,
    flds: Vec<Field>,
    size: Point,
    b: BoundingBox,
    /// `f->sides`: the sides of the record box the field lies on (`BOTTOM` = 1,
    /// `RIGHT` = 2, `TOP` = 4, `LEFT` = 8).
    sides: u8,
}

/// Byte-level port of `parse_reclbl`, sharing one text buffer between recursion levels as C
/// does (ports and text live in the same buffer, which C relies on).
struct RecParser<'a> {
    src: &'a [u8],
    pos: usize,
    html: bool,
    buf: Vec<u8>,
    /// `make_label` for a field's text, run as the field closes, so a field read before
    /// a syntax error is measured (and warned about) too.
    measure: &'a dyn Fn(&str) -> LabelGeometry,
}

impl RecParser<'_> {
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    fn put(&mut self, i: usize, c: u8) {
        if i >= self.buf.len() {
            self.buf.resize(i + 1, 0);
        }
        self.buf[i] = c;
    }

    fn get(&self, i: isize) -> u8 {
        if i < 0 {
            0
        } else {
            self.buf.get(i as usize).copied().unwrap_or(0)
        }
    }

    fn cstr(&self) -> String {
        let end = self
            .buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.buf.len());
        graphviz_pure_core::charset::decode_slice(&self.buf[..end])
    }

    fn parse(&mut self, lr: bool, flag: bool) -> Option<Field> {
        let mut rv = Field {
            lr,
            ..Field::default()
        };
        let mut fp: Option<usize> = None;
        let mut tmpport: Option<String> = None;
        let mut mode: u8 = 0;
        let mut tsp: isize = 0;
        let mut hstsp: isize = 0;
        let mut psp: isize = 0;
        let mut hspsp: isize = isize::MIN;
        let mut wflag = true;
        let mut ishardspace = false;
        while wflag {
            let uc = self.at(self.pos);
            if uc != 0 && uc < b' ' {
                self.pos += 1;
                continue;
            }
            let mut dotext = false;
            match uc {
                b'<' if !self.html => {
                    if mode & (HASTABLE | HASPORT) != 0 {
                        return None;
                    }
                    mode |= HASPORT | INPORT;
                    self.pos += 1;
                    psp = 0;
                    hspsp = 0;
                }
                b'<' => {
                    if mode & (HASTABLE | HASPORT) != 0 {
                        return None;
                    }
                    dotext = true;
                }
                b'>' if !self.html => {
                    if mode & INPORT == 0 {
                        return None;
                    }
                    if psp > 1 && psp - 1 != hspsp && self.get(psp - 1) == b' ' {
                        psp -= 1;
                    }
                    self.put(psp as usize, 0);
                    tmpport = Some(self.cstr());
                    mode &= !INPORT;
                    self.pos += 1;
                }
                b'{' => {
                    self.pos += 1;
                    if mode != 0 || self.at(self.pos) == 0 {
                        return None;
                    }
                    mode = HASTABLE;
                    let sub = self.parse(!lr, false)?;
                    rv.flds.push(sub);
                }
                b'}' | b'|' | 0 => {
                    if (uc == 0 && !flag) || mode & INPORT != 0 {
                        return None;
                    }
                    if mode & HASTABLE == 0 {
                        rv.flds.push(Field::default());
                        fp = Some(rv.flds.len() - 1);
                    }
                    if let Some(tp) = tmpport.take()
                        && let Some(f) = fp
                    {
                        rv.flds[f].id = Some(tp);
                    }
                    if mode & (HASTEXT | HASTABLE) == 0 {
                        mode |= HASTEXT;
                        self.put(tsp as usize, b' ');
                        tsp += 1;
                    }
                    if mode & HASTEXT != 0 {
                        if tsp > 1 && tsp - 1 != hstsp && self.get(tsp - 1) == b' ' {
                            tsp -= 1;
                        }
                        self.put(tsp as usize, 0);
                        let text = self.cstr();
                        if let Some(f) = fp {
                            let fld = &mut rv.flds[f];
                            fld.lp = Some((self.measure)(&text));
                            fld.text = Some(text);
                            fld.lr = true;
                        }
                        tsp = 0;
                        hstsp = 0;
                    }
                    if uc != 0 {
                        if uc == b'}' {
                            self.pos += 1;
                            return Some(rv);
                        }
                        mode = 0;
                        self.pos += 1;
                    } else {
                        wflag = false;
                    }
                }
                b'\\' => {
                    let nx = self.at(self.pos + 1);
                    if nx != 0 {
                        if is_ctrl(nx) {
                            // the control character is taken literally
                        } else if nx == b' ' && !self.html {
                            ishardspace = true;
                        } else {
                            self.put(tsp as usize, b'\\');
                            tsp += 1;
                            mode |= INTEXT | HASTEXT;
                        }
                        self.pos += 1;
                    }
                    dotext = true;
                }
                _ => dotext = true,
            }
            if dotext {
                let c = self.at(self.pos);
                if mode & HASTABLE != 0 && c != b' ' {
                    return None;
                }
                if mode & (INTEXT | INPORT) == 0 && c != b' ' {
                    mode |= INTEXT | HASTEXT;
                }
                if mode & INTEXT != 0 {
                    if !(c == b' ' && !ishardspace && self.get(tsp - 1) == b' ' && !self.html) {
                        self.put(tsp as usize, c);
                        tsp += 1;
                    }
                    if ishardspace {
                        hstsp = tsp - 1;
                    }
                } else if mode & INPORT != 0 {
                    if !(c == b' ' && !ishardspace && (psp == 0 || self.get(psp - 1) == b' ')) {
                        self.put(psp as usize, c);
                        psp += 1;
                    }
                    if ishardspace {
                        hspsp = psp - 1;
                    }
                }
                self.pos += 1;
                while self.at(self.pos) & 0xc0 == 0x80 {
                    let c = self.at(self.pos);
                    self.put(tsp as usize, c);
                    tsp += 1;
                    self.pos += 1;
                }
            }
        }
        Some(rv)
    }
}

/// `size_reclbl` from `shapes.c`.
fn size_reclbl(attrs: &AttrMap, f: &mut Field) -> Point {
    let d = if let Some(lp) = &f.lp {
        let mut dimen = lp.dimen;
        if dimen.x > 0.0 || dimen.y > 0.0 {
            dimen = pad_dimen(attrs, dimen, false);
        }
        dimen
    } else {
        let mut d = Point::new(0.0, 0.0);
        let lr = f.lr;
        for sub in &mut f.flds {
            let d0 = size_reclbl(attrs, sub);
            if lr {
                d.x += d0.x;
                d.y = d.y.max(d0.y);
            } else {
                d.y += d0.y;
                d.x = d.x.max(d0.x);
            }
        }
        d
    };
    f.size = d;
    d
}

/// `resize_reclbl` from `shapes.c` (note the `(int)` truncation when distributing space).
fn resize_reclbl(f: &mut Field, sz: Point, nojustify: bool) {
    let d = Point::new(sz.x - f.size.x, sz.y - f.size.y);
    f.size = sz;
    if let Some(lp) = &mut f.lp
        && !nojustify
    {
        lp.space.x += d.x;
        lp.space.y += d.y;
    }
    let n = f.flds.len();
    if n > 0 {
        let inc = if f.lr { d.x / n as f64 } else { d.y / n as f64 };
        let lr = f.lr;
        for (i, sf) in f.flds.iter_mut().enumerate() {
            let amt = f64::from(((i + 1) as f64 * inc) as i32 - (i as f64 * inc) as i32);
            let newsz = if lr {
                Point::new(sf.size.x + amt, sz.y)
            } else {
                Point::new(sz.x, sf.size.y + amt)
            };
            resize_reclbl(sf, newsz, nojustify);
        }
    }
}

/// `pos_reclbl` from `shapes.c`: field boxes and the record sides each field lies on.
fn pos_reclbl(f: &mut Field, mut ul: Point, sides: u8) {
    const BOTTOM: u8 = 1;
    const RIGHT: u8 = 2;
    const TOP: u8 = 4;
    const LEFT: u8 = 8;
    f.sides = sides;
    f.b = BoundingBox::new(ul.x, ul.y - f.size.y, ul.x + f.size.x, ul.y);
    let lr = f.lr;
    let last = f.flds.len().saturating_sub(1);
    for (i, sub) in f.flds.iter_mut().enumerate() {
        let mask = match (lr, i == 0, i == last) {
            (_, true, true) => TOP | BOTTOM | RIGHT | LEFT,
            (true, true, false) => TOP | BOTTOM | LEFT,
            (true, false, true) => TOP | BOTTOM | RIGHT,
            (true, false, false) => TOP | BOTTOM,
            (false, true, false) => TOP | RIGHT | LEFT,
            (false, false, true) => LEFT | BOTTOM | RIGHT,
            (false, false, false) => LEFT | RIGHT,
        };
        pos_reclbl(sub, ul, sides & mask);
        if lr {
            ul.x += sub.size.x;
        } else {
            ul.y -= sub.size.y;
        }
    }
}

fn to_record_field(f: &Field) -> RecordField {
    RecordField {
        port: f.id.clone(),
        text: f.text.clone().unwrap_or_default(),
        bbox: f.b,
        children: f.flds.iter().map(to_record_field).collect(),
        horizontal: f.lr,
        label: f.lp.clone(),
        sides: f.sides,
    }
}

/// `parse_reclbl` on `label` for `record_init`, falling back to `\N` on a syntax error.
/// `measure` is `make_label` for each field's text, run as the field closes.
fn parse_record_label(
    label: &str,
    html: bool,
    lr: bool,
    measure: &dyn Fn(&str) -> LabelGeometry,
) -> Field {
    // C parses the label's bytes (`graphviz_pure_core::charset`).
    let src = graphviz_pure_core::charset::encode(label);
    let len = src.len().max(1).max(2);
    let mut p = RecParser {
        src: &src,
        pos: 0,
        html,
        buf: vec![0; len + 1],
        measure,
    };
    if let Some(f) = p.parse(lr, true) {
        return f;
    }
    if graphviz_pure_core::diag::reporting() {
        graphviz_pure_core::diag::agerrorf(&format!("bad label format {label}\n"));
    }
    let mut p = RecParser {
        src: b"\\N",
        pos: 0,
        html,
        buf: vec![0; len + 1],
        measure,
    };
    p.parse(lr, true).unwrap_or_default()
}

fn record_init(
    node: &NodeData,
    label: &str,
    obj: &LabelSubstContext<'_>,
    env: &NodeSizeEnv<'_>,
    geom: &mut NodeGeometry,
) {
    let attrs = &node.attrs;
    let flip = !env.realflip;
    let html = geom.label.html;
    let (font_name, font_size) = (&geom.label.font_name, geom.label.font_size);
    let measure = |text: &str| make_label(text, html, obj, font_name, font_size, env.charset);
    let mut info = parse_record_label(label, html, flip, &measure);
    size_reclbl(attrs, &mut info);
    let mut sz = Point::new(inch2ps(geom.width), inch2ps(geom.height));
    if !mapbool(Some(attrs.get("fixedsize").map_or("false", String::as_str))) {
        sz.x = info.size.x.max(sz.x);
        sz.y = info.size.y.max(sz.y);
    }
    let nojustify = mapbool(Some(attrs.get("nojustify").map_or("false", String::as_str)));
    resize_reclbl(&mut info, sz, nojustify);
    let ul = Point::new(-sz.x / 2.0, sz.y / 2.0);
    pos_reclbl(&mut info, ul, 0b1111);
    geom.width = ps2inch(info.size.x);
    // Kluge!! +1 to fix rounding diff between layout and rendering
    geom.height = ps2inch(info.size.y + 1.0);
    geom.outline_width = geom.width;
    geom.outline_height = geom.height;
    geom.record = Some(to_record_field(&info));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(attrs: &[(&str, &str)]) -> NodeData {
        let mut n = NodeData {
            id: graphviz_pure_core::NodeId(0),
            name: "n".into(),
            attrs: AttrMap::new(),
            pos: Point::default(),
            width: 0.0,
            height: 0.0,
            rank: 0,
            order: 0,
            html_name: false,
            layout: Default::default(),
        };
        for (k, v) in attrs {
            n.attrs.insert((*k).into(), (*v).into());
        }
        n
    }

    fn wh(attrs: &[(&str, &str)]) -> (String, String) {
        let g = node_geometry(&node(attrs), &NodeSizeEnv::new("G")).unwrap();
        (format!("{:.5}", g.width), format!("{:.5}", g.height))
    }

    // Expected values are `dot -Tdot` output of Graphviz 16.1.0 with the LUT text estimator.
    #[test]
    fn ellipse_hello_world() {
        let g = node_geometry(&node(&[("label", "hello world")]), &NodeSizeEnv::new("G")).unwrap();
        assert!((g.width - 1.5358).abs() < 5e-5, "{}", g.width);
        assert!((g.height - 0.5).abs() < 1e-12);
    }

    #[test]
    fn record_height_kluge() {
        let (_, h) = wh(&[("shape", "record"), ("label", "a|b")]);
        assert_eq!(h, "0.51389");
    }

    #[test]
    fn record_port_overwrites_text_like_c() {
        let obj = LabelSubstContext::for_node("n");
        let measure =
            |text: &str| make_label(text, false, &obj, "Times-Roman", 14.0, Charset::Utf8);
        let f = parse_record_label("abc <p> def|x", false, true, &measure);
        assert_eq!(f.flds[0].text.as_deref(), Some(""));
        assert_eq!(f.flds[0].id.as_deref(), Some(""));
        assert!(f.flds[0].lp.as_ref().unwrap().dimen.x == 0.0);
    }

    #[test]
    fn record_hard_spaces() {
        let unmeasured = |_: &str| LabelGeometry::default();
        let f = parse_record_label("a\\ \\ b  c|d", false, true, &unmeasured);
        assert_eq!(f.flds[0].text.as_deref(), Some("a  b  c"));
        let f = parse_record_label("", false, true, &unmeasured);
        assert_eq!(f.flds[0].text.as_deref(), Some(" "));
    }

    // `dot -Txdot` 16.1.0 on: digraph G { label="My Graph\nline2"; fontsize=20;
    //   subgraph cluster_x { label=<<B>Bold</B> x>; a }
    //   a -> b [label="e\E" fontsize=10 headlabel="h" labelfontsize=8] }
    #[test]
    fn graph_and_cluster_labels() {
        let mut a = AttrMap::new();
        a.insert("label".into(), "My Graph\\nline2".into());
        a.insert("fontsize".into(), "20".into());
        let g = graph_label_geometry("G", &a, true, false, Charset::Utf8).unwrap();
        assert_eq!(format!("{:.2}", g.label.dimen.x / 72.0), "1.15");
        assert_eq!(format!("{:.2}", g.label.dimen.y / 72.0), "0.67");
        assert_eq!(g.pos_flag, LABEL_AT_BOTTOM);
        assert!(g.border.is_none());
        a.set("label", "<B>Bold</B> x".into(), true);
        let c = graph_label_geometry("cluster_x", &a, false, false, Charset::Utf8).unwrap();
        assert!((c.label.dimen.x - (40.02 + 15.0)).abs() < 0.01);
        assert!((c.label.dimen.y - 24.0).abs() < 1e-9);
        assert_eq!(c.pos_flag, LABEL_AT_TOP);
        let (side, d) = c.border.unwrap();
        assert_eq!(side, BorderSide::Top);
        assert!((d.y - 32.0).abs() < 1e-9);
    }

    #[test]
    fn edge_labels() {
        let mut a = AttrMap::new();
        a.insert("label".into(), "e\\E".into());
        a.insert("fontsize".into(), "10".into());
        a.insert("headlabel".into(), "h".into());
        a.insert("labelfontsize".into(), "8".into());
        let e = edge_label_geometry(&a, "a", "b", true, &NodeSizeEnv::new("G"));
        let l = e.label.unwrap();
        assert_eq!(l.text.as_ref().unwrap().lines[0].text, "ea->b");
        assert!((l.dimen.x - 22.85).abs() < 0.01);
        assert!((e.head_label.unwrap().dimen.x - 4.0).abs() < 1e-9);
        assert!(e.tail_label.is_none());
    }

    #[test]
    fn late_double_is_strtod_prefix() {
        let mut a = AttrMap::new();
        a.insert("w".into(), "1.5in".into());
        assert_eq!(late_double(&a, "w", 0.0, 0.0), 1.5);
        a.insert("w".into(), "x".into());
        assert_eq!(late_double(&a, "w", 7.0, 0.0), 7.0);
        a.insert("w".into(), "-3".into());
        assert_eq!(late_double(&a, "w", 7.0, 0.01), 0.01);
    }

    #[test]
    fn quoted_angle_label_is_text_not_html() {
        // `label="<x>"` is plain text in C (aghtmlstr is false); `label=<x>` is HTML.
        let env = NodeSizeEnv::new("G");
        let q = node_geometry(&node(&[("label", "<x>")]), &env).unwrap();
        let mut n = node(&[]);
        n.attrs.set("label", "x".into(), true);
        let h = node_geometry(&n, &env).unwrap();
        assert!(!q.label.html);
        assert!(h.label.html);
        assert!(
            q.label.dimen.x > h.label.dimen.x,
            "the text form also measures '<' and '>'"
        );
    }
}
