//! SVG renderer: port of `plugin/core/gvrender_core_svg.c`.

use crate::emit::job::{
    Engine, Features, GRADIENT, GVDEVICE_DOES_TRUECOLOR, GVRENDER_DOES_LABELS, GVRENDER_DOES_MAPS,
    GVRENDER_DOES_TARGETS, GVRENDER_DOES_TOOLTIPS, GVRENDER_DOES_TRANSFORM, GVRENDER_Y_GOES_DOWN,
    GvColor, Job, LoadImage, ObjKind, PENWIDTH_NORMAL, Pen, RGRADIENT,
};
use crate::emit::model::TextSpan;
use crate::emit::num::{fmt_f6, fmt_g, gvdouble};
use graphviz_pure_core::math;
use graphviz_pure_core::usershape::UserShape;
use graphviz_pure_core::{BoundingBox, Point};
use graphviz_pure_geom::html_label::{
    HTML_BF, HTML_IF, HTML_OL, HTML_S, HTML_SUB, HTML_SUP, HTML_UL,
};

/// Flags for `gv_xml_escape`.
#[derive(Clone, Copy, Default)]
pub(crate) struct XmlFlags {
    pub raw: bool,
    pub dash: bool,
    pub nbsp: bool,
}

fn xml_isentity(s: &[u8]) -> bool {
    let mut i = 1;
    if s.get(i) == Some(&b';') {
        return false;
    }
    if s.get(i) == Some(&b'#') {
        i += 1;
        if matches!(s.get(i), Some(b'x' | b'X')) {
            i += 1;
            while s.get(i).is_some_and(u8::is_ascii_hexdigit) {
                i += 1;
            }
        } else {
            while s.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    } else {
        while s.get(i).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
        }
    }
    s.get(i) == Some(&b';')
}

/// `gv_xml_escape` (without the `utf8` flag).
pub(crate) fn xml_escape(s: &str, f: XmlFlags) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 8);
    let mut prev = 0u8;
    for i in 0..b.len() {
        let c = b[i];
        match c {
            b'&' if f.raw || !xml_isentity(&b[i..]) => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'-' if f.dash => out.extend_from_slice(b"&#45;"),
            b' ' if prev == b' ' && f.nbsp => out.extend_from_slice(b"&#160;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\'' => out.extend_from_slice(b"&#39;"),
            b'\n' if f.raw => out.extend_from_slice(b"&#10;"),
            b'\r' if f.raw => out.extend_from_slice(b"&#13;"),
            _ => out.push(c),
        }
        prev = c;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn xml_raw_id(s: &str) -> String {
    s.to_string()
}

/// `gvputs_xml`.
pub(crate) fn xml(s: &str) -> String {
    xml_escape(
        s,
        XmlFlags {
            dash: true,
            nbsp: true,
            ..Default::default()
        },
    )
}

/// The Graphviz release this engine is a port of, written in the generator comment the way
/// `svg_begin_job` writes it from `gvc->common.info`. The port's own releases do not change it.
const GRAPHVIZ_VERSION: &str = "16.1.0";

const SVG_KNOWNCOLORS: &[&str] = &[
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "transparent",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

thread_local! {
    static GRAD_IDS: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
}

/// Resets the per-process gradient numbering (`static int gradId`, `rgradId`).
pub(crate) fn reset_gradient_ids() {
    GRAD_IDS.with(|c| c.set((0, 0)));
}

#[derive(Default)]
pub(crate) struct SvgEngine {
    pub out: String,
    /// Set when this SVG is drawn for a raster format, which C draws with its cairo
    /// renderer: the job is then C's cairo job, not the SVG plugin's (#355).
    cairo: Option<CairoJob>,
}

/// What C's cairo job for a raster format is, where it differs from the SVG plugin's: its
/// features (`CAIRO_FEATURES`), render type (`cairo`, which picks the image loaders), `-T`
/// name, and gradients that use no id, so none of the SVG plugin's per-process numbers.
struct CairoJob {
    langname: String,
    /// The SVG still needs unique gradient ids: these count for this job alone.
    grad_ids: (i32, i32),
}

/// `render_features_cairo` with the flags of the png and jpeg devices
/// (`plugin/pango/gvrender_pango.c`, `plugin/gdk/gvdevice_gdk.c`): no maps, tooltips,
/// targets or labels for the renderer, no layers for the device, no known colour names.
const CAIRO_FEATURES: Features = Features {
    flags: GVRENDER_Y_GOES_DOWN | GVRENDER_DOES_TRANSFORM | GVDEVICE_DOES_TRUECOLOR,
    default_pad: 4.0,
    knowncolors: &[],
    rgba: true,
};

impl SvgEngine {
    /// The writer for raster format `langname` (the `-T` value), run as C's cairo job.
    pub(crate) fn cairo(langname: &str) -> Self {
        SvgEngine {
            out: String::new(),
            cairo: Some(CairoJob {
                langname: langname.to_string(),
                grad_ids: (0, 0),
            }),
        }
    }

    /// The next linear (`radial` false) or radial gradient number: the SVG plugin's
    /// `static int gradId`/`rgradId`, or the cairo job's own count.
    fn next_grad_id(&mut self, radial: bool) -> i32 {
        let bump = |(l, r): (i32, i32)| {
            if radial {
                (r, (l, r + 1))
            } else {
                (l, (l + 1, r))
            }
        };
        match &mut self.cairo {
            Some(c) => {
                let (id, next) = bump(c.grad_ids);
                c.grad_ids = next;
                id
            }
            None => GRAD_IDS.with(|c| {
                let (id, next) = bump(c.get());
                c.set(next);
                id
            }),
        }
    }

    /// Whether the edge label follows its spline (`obj->labeledgealigned`): a `textPath`
    /// along the spline's `<path id=…_p>`. That needs the object's id, which only a job
    /// with maps has (`initMapData`); C's cairo renderer, which has none, draws the span at
    /// its point (`cairogen_textspan`), and so does the writer without an id (#355).
    fn aligned(j: &Job) -> bool {
        j.obj().labeledgealigned && j.obj().id.is_some()
    }

    fn put(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn dbl(&mut self, v: f64) {
        self.out.push_str(&gvdouble(v));
    }

    fn print_id_class(&mut self, id: &str, kind: &str, class: Option<&str>) {
        // A job without maps has no ids (the cairo job): no `id` attribute then.
        self.put("<g");
        if !id.is_empty() {
            self.put(" id=\"");
            self.put(&xml(id));
            self.put("\"");
        }
        self.put(" class=\"");
        self.put(kind);
        if let Some(c) = class.filter(|c| !c.is_empty()) {
            self.put(" ");
            self.put(&xml(c));
        }
        self.put("\"");
    }

    fn paint(&mut self, c: &GvColor) {
        match c {
            GvColor::Str(s) => {
                if s == "transparent" {
                    self.put("none");
                } else {
                    self.put(s);
                }
            }
            GvColor::Rgba(r) => {
                if r[3] == 0 {
                    self.put("none");
                } else {
                    self.put(&format!("#{:02x}{:02x}{:02x}", r[0], r[1], r[2]));
                }
            }
        }
    }

    fn grstyle(&mut self, j: &Job, filled: i32, gid: i32) {
        let o = j.obj();
        self.put(" fill=\"");
        if filled == GRADIENT || filled == RGRADIENT {
            self.put("url(#");
            if let Some(id) = &o.id {
                self.put(&xml(id));
                self.put("_");
            }
            self.put(&format!(
                "{}_{gid})",
                if filled == GRADIENT { 'l' } else { 'r' }
            ));
        } else if filled != 0 {
            let fc = o.fillcolor.clone();
            self.paint(&fc);
            if let GvColor::Rgba(r) = fc
                && r[3] > 0
                && r[3] < 255
            {
                self.put(&format!(
                    "\" fill-opacity=\"{}",
                    fmt_f6(f64::from(f32::from(r[3]) / 255.0))
                ));
            }
        } else {
            self.put("none");
        }
        self.put("\" stroke=\"");
        let pc = o.pencolor.clone();
        self.paint(&pc);
        if (o.penwidth - PENWIDTH_NORMAL).abs() >= 0.005 {
            self.put("\" stroke-width=\"");
            self.dbl(o.penwidth);
        }
        match o.pen {
            Pen::Dashed => self.put("\" stroke-dasharray=\"5,2"),
            Pen::Dotted => self.put("\" stroke-dasharray=\"1,5"),
            _ => {}
        }
        if let GvColor::Rgba(r) = pc
            && r[3] > 0
            && r[3] < 255
        {
            self.put(&format!(
                "\" stroke-opacity=\"{}",
                fmt_f6(f64::from(f32::from(r[3]) / 255.0))
            ));
        }
        self.put("\"");
    }

    fn print_stop(&mut self, offset: f64, c: &GvColor) {
        if offset.abs() < 0.0005 {
            self.put("<stop offset=\"0\" style=\"stop-color:");
        } else if (offset - 1.0).abs() < 0.0005 {
            self.put("<stop offset=\"1\" style=\"stop-color:");
        } else {
            self.put(&format!(
                "<stop offset=\"{}\" style=\"stop-color:",
                graphviz_pure_core::cnum::fmt_f(offset, 3)
            ));
        }
        match c {
            GvColor::Str(s) if s == "transparent" => self.put("black"),
            GvColor::Str(s) => self.put(s),
            GvColor::Rgba(r) => self.put(&format!("#{:02x}{:02x}{:02x}", r[0], r[1], r[2])),
        }
        self.put(";stop-opacity:");
        match c {
            GvColor::Rgba(r) if r[3] < 255 => {
                self.put(&fmt_f6(f64::from(f32::from(r[3]) / 255.0)));
            }
            GvColor::Str(s) if s == "transparent" => self.put("0"),
            _ => self.put("1."),
        }
        self.put(";\"/>\n");
    }

    fn gradstyle(&mut self, j: &Job, a: &[Point]) -> i32 {
        let id = self.next_grad_id(false);
        let o = j.obj();
        let angle = o.gradient_angle as f64 * std::f64::consts::PI / 180.0;
        let g = crate::emit::geom::gradient_points(a, angle, 0);
        self.put("<defs>\n<linearGradient id=\"");
        if let Some(oid) = &o.id {
            self.put(&xml(oid));
            self.put("_");
        }
        self.put(&format!("l_{id}\" gradientUnits=\"userSpaceOnUse\" "));
        self.put("x1=\"");
        self.dbl(g[0].x);
        self.put("\" y1=\"");
        self.dbl(g[0].y);
        self.put("\" x2=\"");
        self.dbl(g[1].x);
        self.put("\" y2=\"");
        self.dbl(g[1].y);
        self.put("\" >\n");
        let frac = o.gradient_frac;
        let (fc, sc) = (o.fillcolor.clone(), o.stopcolor.clone());
        self.print_stop(if frac > 0.0 { frac - 0.001 } else { 0.0 }, &fc);
        self.print_stop(if frac > 0.0 { frac } else { 1.0 }, &sc);
        self.put("</linearGradient>\n</defs>\n");
        id
    }

    fn rgradstyle(&mut self, j: &Job) -> i32 {
        let id = self.next_grad_id(true);
        let o = j.obj();
        let (ifx, ify) = if o.gradient_angle == 0 {
            (50.0, 50.0)
        } else {
            let angle = o.gradient_angle as f64 * std::f64::consts::PI / 180.0;
            (
                (50.0 * (1.0 + math::cos(angle))).round(),
                (50.0 * (1.0 - math::sin(angle))).round(),
            )
        };
        self.put("<defs>\n<radialGradient id=\"");
        if let Some(oid) = &o.id {
            self.put(&xml(oid));
            self.put("_");
        }
        self.put(&format!(
            "r_{id}\" cx=\"50%\" cy=\"50%\" r=\"75%\" fx=\"{}%\" fy=\"{}%\">\n",
            graphviz_pure_core::cnum::fmt_f(ifx, 0),
            graphviz_pure_core::cnum::fmt_f(ify, 0)
        ));
        let (fc, sc) = (o.fillcolor.clone(), o.stopcolor.clone());
        self.print_stop(0.0, &fc);
        self.print_stop(1.0, &sc);
        self.put("</radialGradient>\n</defs>\n");
        id
    }

    fn grad(&mut self, j: &Job, filled: i32, a: &[Point]) -> i32 {
        if filled == GRADIENT {
            self.gradstyle(j, a)
        } else if filled == RGRADIENT {
            self.rgradstyle(j)
        } else {
            0
        }
    }

    fn obj_class<'a>(j: &'a Job, kind: ObjKind) -> Option<&'a str> {
        let m = j.m;
        match kind {
            ObjKind::Root => m.root_attrs().get("class"),
            ObjKind::Cluster(c) => m.clusters[c].attrs.get("class"),
            ObjKind::Node(n) => m.nodes[n].attrs.get("class"),
            ObjKind::Edge(e) => m.edges[e].attrs.get("class"),
        }
    }
}

impl Engine for SvgEngine {
    fn features(&self) -> Features {
        if self.cairo.is_some() {
            return CAIRO_FEATURES;
        }
        Features {
            flags: crate::emit::job::GVDEVICE_DOES_LAYERS
                | GVRENDER_Y_GOES_DOWN
                | GVRENDER_DOES_TRANSFORM
                | GVRENDER_DOES_LABELS
                | GVRENDER_DOES_MAPS
                | GVRENDER_DOES_TARGETS
                | GVRENDER_DOES_TOOLTIPS
                | GVDEVICE_DOES_TRUECOLOR,
            default_pad: 4.0,
            knowncolors: SVG_KNOWNCOLORS,
            rgba: true,
        }
    }

    fn begin_job(&mut self, j: &Job) {
        self.put("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
        if let Some(s) = j.m.root_attrs().nonempty("stylesheet") {
            self.put("<?xml-stylesheet href=\"");
            self.put(s);
            self.put("\" type=\"text/css\"?>\n");
        }
        self.put(
            "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"\n \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n",
        );
        self.put(&format!(
            "<!-- Generated by graphviz version {GRAPHVIZ_VERSION} (graphviz-pure)\n -->\n"
        ));
    }

    fn begin_graph(&mut self, j: &Job) {
        let name = j.m.g.root_name();
        self.put("<!--");
        if !name.is_empty() && !name.starts_with('%') {
            self.put(" Title: ");
            self.put(&xml(name));
        }
        self.put(&format!(" Pages: {} -->\n", j.pages.0 * j.pages.1));
        self.put(&format!(
            "<svg width=\"{}pt\" height=\"{}pt\"\n",
            j.width, j.height
        ));
        let b = j.page_bounding_box;
        self.put(&format!(
            " viewBox=\"{}.00 {}.00 {}.00 {}.00\"",
            b[0], b[1], b[2], b[3]
        ));
        self.put(
            " xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
        );
        if let Some(id) = j.m.root_attrs().nonempty("svgid") {
            self.put(" id=\"");
            self.put(&xml(id));
            self.put("\"");
        }
        self.put(">\n");
    }

    fn end_graph(&mut self, _j: &Job) {
        self.put("</svg>\n");
    }

    fn begin_layer(&mut self, j: &Job, name: &str) {
        self.print_id_class(name, "layer", Self::obj_class(j, ObjKind::Root));
        self.put(">\n");
    }

    fn end_layer(&mut self, _j: &Job) {
        self.put("</g>\n");
    }

    fn begin_page(&mut self, j: &Job) {
        let o = j.obj();
        let id = o.id.clone().unwrap_or_default();
        self.print_id_class(&id, "graph", Self::obj_class(j, ObjKind::Root));
        self.put(" transform=\"scale(");
        self.put(&format!("{} {}", fmt_g(j.scale.x, 6), fmt_g(j.scale.y, 6)));
        self.put(&format!(") rotate({}) translate(", -j.rotation));
        self.dbl(j.translation.x);
        self.put(" ");
        self.dbl(-j.translation.y);
        self.put(")\">\n");
        let name = j.m.g.root_name();
        if !name.is_empty() && !name.starts_with('%') {
            self.put("<title>");
            self.put(&xml(name));
            self.put("</title>\n");
        }
    }

    fn end_page(&mut self, _j: &Job) {
        self.put("</g>\n");
    }

    fn begin_cluster(&mut self, j: &Job) {
        let o = j.obj();
        let ObjKind::Cluster(c) = o.kind else { return };
        let id = o.id.clone().unwrap_or_default();
        self.print_id_class(&id, "cluster", Self::obj_class(j, o.kind));
        self.put(">\n<title>");
        self.put(&xml(&j.m.clusters[c].name));
        self.put("</title>\n");
    }

    fn end_cluster(&mut self, _j: &Job) {
        self.put("</g>\n");
    }

    fn begin_node(&mut self, j: &Job) {
        let o = j.obj();
        let ObjKind::Node(n) = o.kind else { return };
        let mut id = o.id.clone().unwrap_or_default();
        if j.layer_num > 1 {
            id.push('_');
            id.push_str(&xml_raw_id(
                j.layer_ids
                    .get(j.layer_num as usize)
                    .map_or("", String::as_str),
            ));
        }
        self.print_id_class(&id, "node", Self::obj_class(j, o.kind));
        self.put(">\n<title>");
        self.put(&xml(j.m.nodes[n].name));
        self.put("</title>\n");
    }

    fn end_node(&mut self, _j: &Job) {
        self.put("</g>\n");
    }

    fn begin_edge(&mut self, j: &Job) {
        let o = j.obj();
        let ObjKind::Edge(e) = o.kind else { return };
        let id = o.id.clone().unwrap_or_default();
        self.print_id_class(&id, "edge", Self::obj_class(j, o.kind));
        self.put(">\n<title>");
        let m = j.m;
        let ed = &m.edges[e];
        // `strdup_and_subst_obj("\\E", e)`: both names are agnameof's, which keeps anonymous
        // ones in one static buffer, so an edge between two of them is titled head->head.
        let [tail, head] = graphviz_pure_core::agnameof_buffer([
            m.node_agnameof(ed.tail),
            m.node_agnameof(ed.head),
        ]);
        let mut name = tail.into_owned();
        if let Some(p) = ed.tail_port.filter(|p| !p.is_empty()) {
            name.push(':');
            name.push_str(p);
        }
        name.push_str(if m.directed { "->" } else { "--" });
        name.push_str(&head);
        if let Some(p) = ed.head_port.filter(|p| !p.is_empty()) {
            name.push(':');
            name.push_str(p);
        }
        self.put(&xml(&name));
        self.put("</title>\n");
    }

    fn end_edge(&mut self, _j: &Job) {
        self.put("</g>\n");
    }

    fn begin_anchor(
        &mut self,
        _j: &Job,
        href: Option<&str>,
        tooltip: Option<&str>,
        target: Option<&str>,
        id: Option<&str>,
    ) {
        self.put("<g");
        if let Some(id) = id {
            self.put(" id=\"a_");
            self.put(&xml(id));
            self.put("\"");
        }
        self.put("><a");
        if let Some(h) = href.filter(|h| !h.is_empty()) {
            self.put(" xlink:href=\"");
            self.put(&xml_escape(h, XmlFlags::default()));
            self.put("\"");
        }
        if let Some(t) = tooltip.filter(|t| !t.is_empty()) {
            self.put(" xlink:title=\"");
            self.put(&xml_escape(
                t,
                XmlFlags {
                    raw: true,
                    dash: true,
                    nbsp: true,
                },
            ));
            self.put("\"");
        }
        if let Some(t) = target.filter(|t| !t.is_empty()) {
            self.put(" target=\"");
            self.put(&xml(t));
            self.put("\"");
        }
        self.put(">\n");
    }

    fn end_anchor(&mut self, _j: &Job) {
        self.put("</a>\n</g>\n");
    }

    fn textspan(&mut self, j: &Job, mut p: Point, span: &TextSpan) {
        let o = j.obj();
        let aligned = Self::aligned(j);
        self.put("<text xml:space=\"preserve\"");
        match span.just {
            'l' => self.put(" text-anchor=\"start\""),
            'r' => self.put(" text-anchor=\"end\""),
            _ => self.put(" text-anchor=\"middle\""),
        }
        p.y += span.yoffset_centerline;
        if !aligned {
            self.put(" x=\"");
            self.dbl(p.x);
            self.put("\" y=\"");
            self.dbl(-p.y);
            self.put("\"");
        }
        let mut weight = None;
        let mut style = None;
        if let Some(pa) = span.font.ps {
            let fontnames = j.m.root_attrs().get("fontnames").unwrap_or("");
            let family;
            match fontnames {
                "ps" => {
                    family = pa.name;
                    weight = pa.weight;
                    style = pa.style;
                }
                "svg" => {
                    family = pa.svg_font_family;
                    weight = pa.svg_font_weight;
                    style = pa.svg_font_style;
                }
                _ => {
                    family = pa.family;
                    weight = pa.weight;
                    style = pa.style;
                }
            }
            self.put(&format!(" font-family=\"{family}"));
            if !pa.svg_font_family.is_empty() && pa.svg_font_family != family {
                self.put(&format!(",{}", pa.svg_font_family));
            }
            self.put("\"");
            if let Some(w) = weight {
                self.put(&format!(" font-weight=\"{w}\""));
            }
            if let Some(s) = pa.stretch {
                self.put(&format!(" font-stretch=\"{s}\""));
            }
            if let Some(s) = style {
                self.put(&format!(" font-style=\"{s}\""));
            }
        } else {
            self.put(&format!(" font-family=\"{}\"", span.font.name));
        }
        let flags = span.font.flags;
        if flags != 0 {
            if flags & HTML_BF != 0 && weight.is_none() {
                self.put(" font-weight=\"bold\"");
            }
            if flags & HTML_IF != 0 && style.is_none() {
                self.put(" font-style=\"italic\"");
            }
            if flags & (HTML_UL | HTML_S | HTML_OL) != 0 {
                self.put(" text-decoration=\"");
                let mut comma = false;
                if flags & HTML_UL != 0 {
                    self.put("underline");
                    comma = true;
                }
                if flags & HTML_OL != 0 {
                    self.put(if comma { ",overline" } else { "overline" });
                    comma = true;
                }
                if flags & HTML_S != 0 {
                    self.put(if comma {
                        ",line-through"
                    } else {
                        "line-through"
                    });
                }
                self.put("\"");
            }
            if flags & HTML_SUP != 0 {
                self.put(" baseline-shift=\"super\"");
            }
            if flags & HTML_SUB != 0 {
                self.put(" baseline-shift=\"sub\"");
            }
        }
        self.put(&format!(
            " font-size=\"{}\"",
            graphviz_pure_core::cnum::fmt_f(span.font.size, 2)
        ));
        match &o.pencolor {
            GvColor::Str(s) => {
                if !s.eq_ignore_ascii_case("black") {
                    self.put(&format!(" fill=\"{s}\""));
                }
            }
            GvColor::Rgba(r) => {
                self.put(&format!(" fill=\"#{:02x}{:02x}{:02x}\"", r[0], r[1], r[2]));
                if r[3] < 255 {
                    self.put(&format!(
                        " fill-opacity=\"{}\"",
                        fmt_f6(f64::from(f32::from(r[3]) / 255.0))
                    ));
                }
            }
        }
        self.put(">");
        if aligned {
            self.put("<textPath xlink:href=\"#");
            self.put(&xml(o.id.as_deref().unwrap_or("")));
            self.put("_p\" startOffset=\"50%\"><tspan x=\"0\" dy=\"");
            self.dbl(-p.y);
            self.put("\">");
        }
        self.put(&xml_escape(
            &span.str,
            XmlFlags {
                raw: true,
                dash: true,
                nbsp: true,
            },
        ));
        if aligned {
            self.put("</tspan></textPath>");
        }
        self.put("</text>\n");
    }

    fn ellipse(&mut self, j: &Job, a: [Point; 2], filled: i32) {
        let gid = self.grad(j, filled, &a);
        self.put("<ellipse");
        self.grstyle(j, filled, gid);
        self.put(" cx=\"");
        self.dbl(a[0].x);
        self.put("\" cy=\"");
        self.dbl(-a[0].y);
        self.put("\" rx=\"");
        self.dbl(a[1].x - a[0].x);
        self.put("\" ry=\"");
        self.dbl(a[1].y - a[0].y);
        self.put("\"/>\n");
    }

    fn bezier(&mut self, j: &Job, a: &[Point], filled: i32) {
        let gid = self.grad(j, filled, a);
        self.put("<path");
        if Self::aligned(j) {
            self.put(" id=\"");
            self.put(&xml(j.obj().id.as_deref().unwrap_or("")));
            self.put("_p\" ");
        }
        self.grstyle(j, filled, gid);
        self.put(" d=\"");
        let mut c = 'M';
        for (i, p) in a.iter().enumerate() {
            self.out.push(c);
            self.dbl(p.x);
            self.put(",");
            self.dbl(-p.y);
            c = if i == 0 { 'C' } else { ' ' };
        }
        self.put("\"/>\n");
    }

    fn polygon(&mut self, j: &Job, a: &[Point], filled: i32) {
        let gid = self.grad(j, filled, a);
        self.put("<polygon");
        self.grstyle(j, filled, gid);
        self.put(" points=\"");
        for p in a {
            self.dbl(p.x);
            self.put(",");
            self.dbl(-p.y);
            self.put(" ");
        }
        // The closing point is `A[0]`; with no points (an xdot `P 0`) C reads it past the
        // end of a zero-size `gv_calloc` block, zeroed on the reference build.
        let p = a.first().copied().unwrap_or_default();
        self.dbl(p.x);
        self.put(",");
        self.dbl(-p.y);
        self.put("\"/>\n");
    }

    fn polyline(&mut self, j: &Job, a: &[Point]) {
        self.put("<polyline");
        self.grstyle(j, 0, 0);
        self.put(" points=\"");
        for (i, p) in a.iter().enumerate() {
            self.dbl(p.x);
            self.put(",");
            self.dbl(-p.y);
            if i + 1 != a.len() {
                self.put(" ");
            }
        }
        self.put("\"/>\n");
    }

    fn comment(&mut self, _j: &Job, s: &str) {
        self.put("<!-- ");
        self.put(&xml(s));
        self.put(" -->\n");
    }

    fn output_langname(&self) -> &str {
        self.cairo.as_ref().map_or("svg", |c| &c.langname)
    }

    fn render_type(&self) -> &str {
        if self.cairo.is_some() { "cairo" } else { "svg" }
    }

    /// `core_loadimage_svg` (`plugin/core/gvloadimage_core.c`): an `<image>` element for
    /// the file, its name as it is.
    fn loadimage(&mut self, j: &Job, li: LoadImage, us: &UserShape, b: BoundingBox) {
        if li != LoadImage::Svg {
            return;
        }
        let width = b.ur.x - b.ll.x;
        let height = b.ur.y - b.ll.y;
        let originx = (b.ur.x + b.ll.x - width) / 2.0;
        let originy = (b.ur.y + b.ll.y + height) / 2.0;
        let g = |v: f64| fmt_g(v, 6);
        self.put("<image xlink:href=\"");
        self.put(&us.name);
        if j.rotation != 0 {
            // C's own FIXME: the size is exchanged and the position is not rotated
            self.put(&format!(
                "\" width=\"{}px\" height=\"{}px\" preserveAspectRatio=\"xMidYMid meet\" x=\"{}\" y=\"{}\"",
                g(height),
                g(width),
                g(originx),
                g(-originy)
            ));
            self.put(&format!(
                " transform=\"rotate({} {} {})\"",
                j.rotation,
                g(originx),
                g(-originy)
            ));
        } else {
            self.put(&format!(
                "\" width=\"{}px\" height=\"{}px\" preserveAspectRatio=\"xMinYMin meet\" x=\"{}\" y=\"{}\"",
                g(width),
                g(height),
                g(originx),
                g(-originy)
            ));
        }
        self.put("/>\n");
    }

    /// The output file gets the text where the SVG has got to.
    fn write_output_file(&mut self, s: &str) -> bool {
        self.put(s);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::{OutputFormat, render_graph};
    use graphviz_pure_core::{BoundingBox, Graph, GraphKind};

    // Issue #461: the generator comment is version-free — it names the ported Graphviz and the
    // port, neither of which changes with a release, so the byte-for-byte release smoke checks
    // never move between releases. The exact text is pinned: it also begins with the prefix
    // Python's `graphviz` package tests for (`<!-- Generated by graphviz version `).
    #[test]
    fn svg_generator_comment_is_pinned() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        g.bb = BoundingBox::new(0.0, 0.0, 54.0, 108.0);
        let svg = render_graph(&g, OutputFormat::Svg).expect("render");
        let expected = "<!-- Generated by graphviz version 16.1.0 (graphviz-pure)\n -->\n";
        assert!(svg.contains(expected), "{expected:?} in {svg}");
    }
}
