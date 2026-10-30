//! Client-side image map renderer (`cmapx`, `cmapx_np`): port of `plugin/core/gvrender_core_map.c`.

use crate::emit::job::{
    EMIT_CLUSTERS_LAST, Engine, Features, GVRENDER_DOES_LABELS, GVRENDER_DOES_MAP_CIRCLE,
    GVRENDER_DOES_MAP_POLYGON, GVRENDER_DOES_MAP_RECTANGLE, GVRENDER_DOES_MAPS,
    GVRENDER_DOES_TARGETS, GVRENDER_DOES_TOOLTIPS, GVRENDER_Y_GOES_DOWN, Job, MapShape,
};
use crate::svg_engine::{XmlFlags, xml, xml_escape};
use graphviz_pure_core::Point;

pub(crate) struct MapEngine<'a> {
    pub out: String,
    nopoly: bool,
    langname: &'a str,
}

impl<'a> MapEngine<'a> {
    pub(crate) fn new(nopoly: bool, langname: &'a str) -> Self {
        Self {
            out: String::new(),
            nopoly,
            langname,
        }
    }

    fn output_shape(
        &mut self,
        shape: MapShape,
        af: &[Point],
        url: Option<&str>,
        tooltip: Option<&str>,
        target: Option<&str>,
        id: Option<&str>,
    ) {
        if af.is_empty() {
            return;
        }
        match shape {
            MapShape::Circle => self.out.push_str("<area shape=\"circle\""),
            MapShape::Rectangle => self.out.push_str("<area shape=\"rect\""),
            MapShape::Polygon => self.out.push_str("<area shape=\"poly\""),
        }
        if let Some(id) = id.filter(|s| !s.is_empty()) {
            self.out.push_str(" id=\"");
            self.out.push_str(&xml_escape(id, XmlFlags::default()));
            self.out.push('"');
        }
        if let Some(url) = url.filter(|s| !s.is_empty()) {
            self.out.push_str(" href=\"");
            self.out.push_str(&xml_escape(url, XmlFlags::default()));
            self.out.push('"');
        }
        if let Some(target) = target.filter(|s| !s.is_empty()) {
            self.out.push_str(" target=\"");
            self.out.push_str(&xml(target));
            self.out.push('"');
        }
        if let Some(tooltip) = tooltip.filter(|s| !s.is_empty()) {
            self.out.push_str(" title=\"");
            self.out.push_str(&xml(tooltip));
            self.out.push('"');
        }
        self.out.push_str(" alt=\"\" coords=\"");
        let f0 = |out: &mut String, v: f64| {
            let _ = graphviz_pure_core::cnum::fmt_f_into(out, v, 0);
        };
        match shape {
            MapShape::Circle => {
                f0(&mut self.out, af[0].x);
                self.out.push(',');
                f0(&mut self.out, af[0].y);
                self.out.push(',');
                f0(&mut self.out, af[1].x - af[0].x);
            }
            MapShape::Rectangle => {
                // Y_GOES_DOWN so need UL to LR
                f0(&mut self.out, af[0].x);
                self.out.push(',');
                f0(&mut self.out, af[1].y);
                self.out.push(',');
                f0(&mut self.out, af[1].x);
                self.out.push(',');
                f0(&mut self.out, af[0].y);
            }
            MapShape::Polygon => {
                f0(&mut self.out, af[0].x);
                self.out.push(',');
                f0(&mut self.out, af[0].y);
                for p in &af[1..] {
                    self.out.push(',');
                    f0(&mut self.out, p.x);
                    self.out.push(',');
                    f0(&mut self.out, p.y);
                }
            }
        }
        self.out.push_str("\"/>\n");
    }
}

impl Engine for MapEngine<'_> {
    fn features(&self) -> Features {
        let mut flags = EMIT_CLUSTERS_LAST
            | GVRENDER_Y_GOES_DOWN
            | GVRENDER_DOES_MAPS
            | GVRENDER_DOES_LABELS
            | GVRENDER_DOES_TOOLTIPS
            | GVRENDER_DOES_TARGETS
            | GVRENDER_DOES_MAP_RECTANGLE;
        if !self.nopoly {
            flags |= GVRENDER_DOES_MAP_CIRCLE | GVRENDER_DOES_MAP_POLYGON;
        }
        Features {
            flags,
            default_pad: 4.0,
            default_dpi: 96.0,
            knowncolors: &[],
            rgba: false,
        }
    }

    fn output_langname(&self) -> &str {
        self.langname
    }

    fn render_type(&self) -> &str {
        "map"
    }

    fn begin_page(&mut self, j: &Job) {
        let name = xml(j.m.g.root_name());
        self.out.push_str("<map id=\"");
        self.out.push_str(&name);
        self.out.push_str("\" name=\"");
        self.out.push_str(&name);
        self.out.push_str("\">\n");
    }

    fn end_page(&mut self, j: &Job) {
        let o = j.obj();
        self.output_shape(
            o.url_map_shape,
            &o.url_map_p,
            o.url.as_deref(),
            o.tooltip.as_deref(),
            o.target.as_deref(),
            o.id.as_deref(),
        );
        self.out.push_str("</map>\n");
    }

    fn begin_anchor(
        &mut self,
        j: &Job,
        href: Option<&str>,
        tooltip: Option<&str>,
        target: Option<&str>,
        id: Option<&str>,
    ) {
        let o = j.obj();
        self.output_shape(o.url_map_shape, &o.url_map_p, href, tooltip, target, id);
    }

    fn write_output_file(&mut self, s: &str) -> bool {
        self.out.push_str(s);
        true
    }
}
