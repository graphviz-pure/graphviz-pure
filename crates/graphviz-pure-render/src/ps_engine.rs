//! C: plugin/core/gvrender_core_ps.c; lib/common/psusershape.c
//! Core PostScript writer and the standard Graphviz PostScript prolog.
use crate::emit::{
    job::*,
    model::TextSpan,
    num::{gvdouble_into, gvprintnum},
};
use graphviz_pure_core::{
    Point,
    charset::{decode_slice, encode},
};
use std::fmt::Write;

/// C: plugin/core/gvrender_core_ps.c:psgen_engine
#[derive(Default)]
pub(crate) struct PsEngine {
    pub out: String,
    pages: usize,
    streaming: bool,
    libraries: Vec<String>,
    bounding_box: Option<[i32; 4]>,
}

/// C: lib/common/psusershape.c:ps_string
fn ps_string(s: &str) -> String {
    let bytes = encode(s);
    let force_latin =
        graphviz_pure_core::charset::current() == graphviz_pure_core::charset::Charset::Latin1;
    let mut latin = true;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        if c < 0x7f {
            continue;
        }
        if c & 0xfc == 0xc0 {
            i += 1;
        } else {
            latin = false;
            break;
        }
    }
    if !force_latin && !latin {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            graphviz_pure_core::diag::agwarningf(
                "UTF-8 input uses non-Latin1 characters which cannot be handled by this PostScript driver\n",
            );
        }
    }
    let bytes = if force_latin || latin {
        let mut result = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            i += 1;
            if c < 0x7f {
                result.push(c);
            } else {
                let next = bytes.get(i).copied().unwrap_or(0);
                i += 1;
                result.push(((c & 3) << 6) | (next & 0x3f));
            }
        }
        result
    } else {
        bytes.into_owned()
    };
    let mut out = vec![b'('];
    for b in bytes.into_iter().take_while(|b| *b != 0) {
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    decode_slice(&out)
}
impl PsEngine {
    fn finish(&mut self) {
        if let Some([x, y, u, v]) = self.bounding_box.take() {
            writeln!(
                self.out,
                "%%Trailer\n%%Pages: {}\n%%BoundingBox: {x} {y} {u} {v}\nend\nrestore\n%%EOF",
                self.pages
            )
            .unwrap();
        }
    }
    fn point(&mut self, p: Point) {
        self.out.push_str(&gvprintnum(p.x));
        self.out.push(' ');
        self.out.push_str(&gvprintnum(p.y));
    }
    fn points(&mut self, a: &[Point]) {
        for (i, p) in a.iter().enumerate() {
            if i > 0 {
                self.out.push(' ');
            }
            self.point(*p);
        }
    }
    /// C: plugin/core/gvrender_core_ps.c:ps_set_color
    fn color(&mut self, j: &Job, c: &GvColor) {
        let hsva = match c {
            GvColor::Str(s) => {
                crate::ps_engine::hsva(s.split(':').next().unwrap_or(""), j.color_scheme.as_deref())
            }
            _ => {
                let [r, g, b, a] = c.rgba();
                let (h, s, v) = graphviz_pure_core::color::rgb2hsv(
                    f64::from(r) / 255.,
                    f64::from(g) / 255.,
                    f64::from(b) / 255.,
                );
                [h, s, v, f64::from(a) / 255.]
            }
        };
        let kind = match j.obj().kind {
            ObjKind::Root | ObjKind::Cluster(_) => "graph",
            ObjKind::Node(_) => "node",
            ObjKind::Edge(_) => "edge",
        };
        let g = |v| graphviz_pure_core::cnum::fmt_g(v, 5);
        writeln!(
            self.out,
            "{} {} {} {kind}color",
            g(hsva[0]),
            g(hsva[1]),
            g(hsva[2])
        )
        .unwrap();
    }
    fn alpha(&self, j: &Job, c: &GvColor) -> f64 {
        match c {
            GvColor::Str(s) => {
                hsva(s.split(':').next().unwrap_or(""), j.color_scheme.as_deref())[3]
            }
            _ => f64::from(c.rgba()[3]) / 255.,
        }
    }
    /// C: plugin/core/gvrender_core_ps.c:ps_set_pen_style
    fn pen(&mut self, j: &Job) {
        gvdouble_into(&mut self.out, j.obj().penwidth);
        self.out.push_str(" setlinewidth\n");
        if let Some(style) = &j.obj().rawstyle {
            for s in style {
                if s.name == "setlinewidth" {
                    continue;
                }
                for arg in &s.args {
                    write!(self.out, "{arg} ").unwrap();
                }
                writeln!(self.out, "{}", s.name).unwrap();
            }
        }
    }
    /// C: plugin/core/gvrender_core_ps.c:psgen_polygon,psgen_bezier,psgen_polyline
    fn path(&mut self, j: &Job, a: &[Point], filled: i32, bezier: bool, closed: bool) {
        if a.is_empty() {
            return;
        }
        for fill in [true, false] {
            let c = if fill {
                &j.obj().fillcolor
            } else {
                &j.obj().pencolor
            };
            if (fill && filled == 0) || self.alpha(j, c) <= 0.5 {
                continue;
            }
            if !fill {
                self.pen(j);
            }
            self.color(j, c);
            self.out.push_str("newpath ");
            self.point(a[0]);
            self.out.push_str(" moveto\n");
            if bezier {
                for p in a[1..].as_chunks::<3>().0 {
                    self.points(p);
                    self.out.push_str(" curveto\n");
                }
            } else {
                for p in &a[1..] {
                    self.point(*p);
                    self.out.push_str(" lineto\n");
                }
            }
            if closed || fill {
                self.out.push_str("closepath ");
            }
            self.out.push_str(if fill { "fill\n" } else { "stroke\n" });
        }
    }
}
impl Engine for PsEngine {
    fn features(&self) -> Features {
        Features {
            flags: GVDEVICE_DOES_PAGES
                | GVDEVICE_DOES_LAYERS
                | GVRENDER_DOES_TRANSFORM
                | GVRENDER_DOES_MAPS
                | GVRENDER_DOES_MAP_RECTANGLE
                | GVRENDER_NO_WHITE_BG,
            default_pad: 4.,
            default_dpi: 72.,
            knowncolors: &[],
            rgba: false,
        }
    }
    fn write_output_file(&mut self, s: &str) -> bool {
        self.out.push_str(s);
        true
    }
    /// C: plugin/core/gvloadimage_core.c:core_loadimage_ps
    fn loadimage(
        &mut self,
        _j: &Job,
        _li: LoadImage,
        us: &graphviz_pure_core::usershape::UserShape,
        b: graphviz_pure_core::BoundingBox,
    ) {
        if let Some(data) = ps_file(&us.name) {
            writeln!(
                self.out,
                "gsave {} {} translate newpath",
                graphviz_pure_core::cnum::fmt_g(b.ll.x - us.x, 6),
                graphviz_pure_core::cnum::fmt_g(b.ll.y - us.y, 6)
            )
            .unwrap();
            self.out.push_str(&epsf_body(&data));
            self.out.push_str("grestore\n");
        }
    }
    /// C: plugin/core/gvrender_core_ps.c:psgen_library_shape
    fn library_shape(&mut self, j: &Job, name: &str, a: &[Point], filled: bool) {
        if a.is_empty() {
            return;
        }
        for fill in [true, false] {
            let c = if fill {
                &j.obj().fillcolor
            } else {
                &j.obj().pencolor
            };
            if (fill && !filled) || self.alpha(j, c) <= 0.5 {
                continue;
            }
            if !fill {
                self.pen(j);
            }
            self.color(j, c);
            self.out.push_str("[ ");
            self.points(a);
            self.out.push(' ');
            self.point(a[0]);
            writeln!(self.out, " ]  {} {} {name}", a.len(), fill).unwrap();
        }
    }
    fn output_langname(&self) -> &str {
        "ps"
    }
    fn render_type(&self) -> &str {
        "ps"
    }
    fn begin_job(&mut self, _j: &Job) {
        if self.pages != 0 {
            return;
        }
        self.out
            .push_str("%!PS-Adobe-3.0\n%%Creator: graphviz version 16.1.0 (20260904.0139)\n");
    }
    fn begin_graph(&mut self, j: &Job) {
        if self.pages == 0 {
            writeln!(self.out, "%%Title: {}", j.m.g.root_name()).unwrap();
            self.out
                .push_str("%%Pages: (atend)\n%%BoundingBox: (atend)\n%%EndComments\nsave\n");
            if !self.libraries.iter().any(String::is_empty) {
                self.out.push_str(include_str!("ps_prolog.ps"));
            }
            for library in &self.libraries {
                if library.is_empty() {
                    continue;
                }
                match ps_file(library) {
                    Some(data) => {
                        self.out.push_str(&data);
                        self.out.push('\n');
                    }
                    None => graphviz_pure_core::diag::agwarningf(&format!(
                        "can't open library file {library}\n"
                    )),
                }
            }
            for shape in &j.m.g.layout.usershapes.epsf_definitions {
                if shape.must_inline {
                    continue;
                }
                writeln!(
                    self.out,
                    "/user_shape_{} {{\n%%BeginDocument:",
                    shape.macro_id
                )
                .unwrap();
                self.out.push_str(&epsf_body(&decode_slice(&shape.body)));
                self.out.push_str("%%EndDocument\n} bind def\n");
            }
        }
        self.out.push_str("setupLatin1\n");
        if let Some(url) = &j.obj().url {
            writeln!(
                self.out,
                "[ {{Catalog}} << /URI << /Base {} >> >>\n/PUT pdfmark",
                ps_string(url)
            )
            .unwrap();
        }
    }
    fn begin_layer(&mut self, j: &Job, _name: &str) {
        writeln!(self.out, "{} {} setlayer", j.layer_num, j.num_layers).unwrap();
    }
    fn begin_page(&mut self, j: &Job) {
        self.pages += 1;
        let [x, y, u, v] = j.page_bounding_box;
        writeln!(self.out,"%%Page: {0} {0}\n%%PageBoundingBox: {x} {y} {u} {v}\n%%PageOrientation: {1}\n{4} {5} {6} beginpage\ngsave\n{x} {y} {2} {3} boxprim clip newpath",self.pages,if j.rotation!=0 {"Landscape"}else{"Portrait"},u-x,v-y,j.page_elem.0,j.page_elem.1,j.pages.0*j.pages.1).unwrap();
        let g = |v| graphviz_pure_core::cnum::fmt_g(v, 6);
        writeln!(
            self.out,
            "{} {} set_scale {} rotate {} {} translate",
            g(j.scale.x),
            g(j.scale.y),
            j.rotation,
            g(j.translation.x),
            g(j.translation.y)
        )
        .unwrap();
    }
    fn end_page(&mut self, _j: &Job) {
        writeln!(
            self.out,
            "endpage\nshowpage\ngrestore\n%%PageTrailer\n%%EndPage: {}",
            self.pages
        )
        .unwrap();
    }
    fn end_job(&mut self, j: &Job) {
        let b = j.page_bounding_box;
        self.bounding_box = Some(match self.bounding_box {
            None => b,
            Some(a) => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ],
        });
        if !self.streaming {
            self.finish();
        }
    }
    fn begin_cluster(&mut self, j: &Job) {
        if let ObjKind::Cluster(i) = j.obj().kind {
            writeln!(self.out, "% {}", j.m.clusters[i].name).unwrap();
        }
        self.out.push_str("gsave\n");
    }
    fn end_cluster(&mut self, _j: &Job) {
        self.out.push_str("grestore\n");
    }
    fn begin_node(&mut self, _j: &Job) {
        self.out.push_str("gsave\n");
    }
    fn end_node(&mut self, _j: &Job) {
        self.out.push_str("grestore\n");
    }
    fn begin_edge(&mut self, _j: &Job) {
        self.out.push_str("gsave\n");
    }
    fn end_edge(&mut self, _j: &Job) {
        self.out.push_str("grestore\n");
    }
    fn comment(&mut self, _j: &Job, s: &str) {
        writeln!(self.out, "% {s}").unwrap();
    }
    fn textspan(&mut self, j: &Job, mut p: Point, s: &TextSpan) {
        if self.alpha(j, &j.obj().pencolor) < 0.5 {
            return;
        }
        self.color(j, &j.obj().pencolor);
        gvdouble_into(&mut self.out, s.font.size);
        if s.font.name.len() > 29 {
            graphviz_pure_core::diag::agwarningf(&format!(
                "font name {} is longer than 29 characters which may be rejected by some PS viewers\n",
                s.font.name
            ));
        }
        if s.font
            .name
            .chars()
            .any(|c| !c.is_ascii() || c == ' ' || c.is_ascii_control())
        {
            graphviz_pure_core::diag::agwarningf(&format!(
                "font name {} contains characters that may not be accepted by some PS viewers\n",
                s.font.name
            ));
        }
        writeln!(self.out, " /{} set_font", s.font.name).unwrap();
        p.x -= match s.just {
            'r' => s.width,
            'l' => 0.,
            _ => s.width / 2.,
        };
        p.y += s.yoffset_centerline;
        self.point(p);
        self.out.push_str(" moveto ");
        gvdouble_into(&mut self.out, s.width);
        writeln!(self.out, " {} alignedtext", ps_string(&s.str)).unwrap();
    }
    fn ellipse(&mut self, j: &Job, a: [Point; 2], filled: i32) {
        let a = [a[0], Point::new(a[1].x - a[0].x, a[1].y - a[0].y)];
        for fill in [true, false] {
            let c = if fill {
                &j.obj().fillcolor
            } else {
                &j.obj().pencolor
            };
            if (fill && filled == 0) || self.alpha(j, c) <= 0.5 {
                continue;
            }
            if !fill {
                self.pen(j);
            }
            self.color(j, c);
            self.points(&a);
            self.out.push_str(if fill {
                " ellipse_path fill\n"
            } else {
                " ellipse_path stroke\n"
            });
        }
    }
    fn polygon(&mut self, j: &Job, a: &[Point], f: i32) {
        self.path(j, a, f, false, true);
    }
    fn bezier(&mut self, j: &Job, a: &[Point], f: i32) {
        self.path(j, a, f, true, false);
    }
    fn polyline(&mut self, j: &Job, a: &[Point]) {
        self.path(j, a, 0, false, false);
    }
    fn begin_anchor(
        &mut self,
        j: &Job,
        url: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
    ) {
        if let Some(url) = url.filter(|_| j.obj().url_map_p.len() >= 2) {
            self.out.push_str("[ /Rect [ ");
            self.points(&j.obj().url_map_p[..2]);
            writeln!(self.out," ]\n  /Border [ 0 0 0 ]\n  /Action << /Subtype /URI /URI {} >>\n  /Subtype /Link\n/ANN pdfmark",ps_string(url)).unwrap();
        }
    }
}

/// C: lib/common/colxlate.c:colorxlate (HSVA_DOUBLE)
fn hsva(s: &str, scheme: Option<&str>) -> [f64; 4] {
    graphviz_pure_core::color::resolve_hsva_with_scheme(s, scheme).unwrap_or([0., 0., 0., 1.])
}

/// C: lib/common/psusershape.c:epsf_emit_body
fn epsf_body(data: &str) -> String {
    let mut out = String::new();
    let data = data.split('\0').next().unwrap_or_default();
    let normalized = data.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.split_terminator('\n') {
        let upper = line.to_ascii_uppercase();
        if ["%%EOF", "%%BEGIN", "%%END", "%%TRAILER"]
            .iter()
            .any(|p| upper.starts_with(p))
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
/// C: lib/common/psusershape.c:user_init; plugin/core/gvloadimage_core.c:core_loadimage_ps
fn ps_file(name: &str) -> Option<String> {
    let path = graphviz_pure_core::usershape::safefile(name)?;
    std::fs::read(path).ok().map(|b| decode_slice(&b))
}

/// C: lib/gvc/gvrender.c:gvrender_begin_job,gvrender_end_job
/// A PostScript device shared by successive graphs. Call `finish` after the last graph.
#[derive(Default)]
pub struct PostscriptJob {
    engine: PsEngine,
}
impl PostscriptJob {
    /// C: lib/common/psusershape.c:cat_libfile
    /// Sets the `-l` libraries included when this document starts.
    pub fn set_libraries(&mut self, libraries: &[String]) {
        self.engine.libraries = libraries.to_vec();
    }

    /// Emits the next laid-out graph, retaining the document's pages and bounding box.
    pub fn render(
        &mut self,
        graph: &graphviz_pure_core::Graph,
        job: usize,
    ) -> graphviz_pure_core::Result<String> {
        graphviz_pure_core::charset::begin_render(
            graph.root_name(),
            graph.graph_attrs.get("charset").map(String::as_str),
        );
        graphviz_pure_core::usershape::rendering(&graph.layout.usershapes, || {
            let drawn = graph.render_view.as_deref().unwrap_or(graph);
            let model = crate::emit::model::Model::build(drawn)?;
            let mut engine = std::mem::take(&mut self.engine);
            engine.streaming = true;
            self.engine = crate::emit::run_job(
                &model,
                engine,
                crate::emit::RunJob {
                    job: Some(job),
                    output_file: true,
                },
            )
            .0;
            Ok(std::mem::take(&mut self.engine.out))
        })
    }
    /// Emits the trailer and resets the device for another document.
    pub fn finish(&mut self) -> String {
        self.engine.finish();
        let out = std::mem::take(&mut self.engine.out);
        self.engine = PsEngine::default();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ps_strings_escape_and_convert_as_c() {
        graphviz_pure_core::charset::begin_render("G", None);
        assert_eq!(
            encode(&ps_string("café (x)\\")),
            b"(caf\xe9 \\(x\\)\\\\)".as_slice()
        );
        assert_eq!(encode(&ps_string("x\u{7f}")), b"(x\x7f)".as_slice());
        graphviz_pure_core::charset::begin_render("G", Some("latin1"));
        assert_eq!(encode(&ps_string("€")), b"(\x82)".as_slice());
    }
    #[test]
    fn eps_dsc_lines_are_removed_and_line_endings_normalized() {
        assert_eq!(
            epsf_body("%!PS\r\n%%BeginDocument:x\rfoo\r\n\r\n%%Trailer\nbar"),
            "%!PS\nfoo\n\nbar\n"
        );
    }
}
