//! Port of the drawing traversal in `lib/common/emit.c` (`emit_graph`, `emit_page`,
//! `emit_background`, `emit_view`, `emit_clusters`, `emit_node`, `emit_edge`,
//! `emit_edge_graphics`, `emit_begin_edge`, `emit_end_edge`, `emit_edge_label`,
//! `initObjMapData`, `getObjId`, `parseSegs`, `findStopColor`, `stripedBox`) and of
//! `emit_label` in `lib/common/labels.c`.

use super::attrs::{Attrs, PtrStr, StyleItem, default_line_style, mapbool, parse_style};
use super::job::{
    EMIT_CLUSTERS_LAST, EMIT_EDGE_SORTED, EMIT_SORTED, EmitState, Emitter, Engine, FILL, GRADIENT,
    GVDEVICE_DOES_TRUECOLOR, GVRENDER_DOES_LABELS, GVRENDER_DOES_MAP_POLYGON,
    GVRENDER_DOES_MAP_RECTANGLE, GVRENDER_DOES_MAPS, GVRENDER_DOES_TARGETS, GVRENDER_DOES_TOOLTIPS,
    GVRENDER_DOES_TRANSFORM, GVRENDER_NO_WHITE_BG, MapShape, ObjKind, RGRADIENT,
};
use super::model::{Bz, TextLabel};
use super::shapes::{is_exactly_zero, is_rect, p_ellipse, rect2poly};
use graphviz_pure_core::diag::{agerr_prev, agerrorf, agwarningf};
use graphviz_pure_core::font::{
    LabelSubstContext, NamesInBuffer, convert_label_text, strdup_and_subst_obj,
};
use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, Obj, Point};

pub(crate) const DEFAULT_COLOR: &str = "black";
pub(crate) const DEFAULT_FILL: &str = "lightgrey";

/// A color segment of a `color` list (`colorseg_t`).
#[derive(Debug, Clone)]
pub(crate) struct ColorSeg {
    pub color: Option<String>,
    pub t: f64,
    pub has_fraction: bool,
}

fn aeq0(x: f64) -> bool {
    x < 1e-5 && x > -1e-5
}

/// `tok(clrs, ":")` of `lib/util/tokenize.h`: the tokens between runs of `:`. Unlike
/// `strtok`, it yields an empty token before a leading `:` and after a trailing one (and
/// `""` gives one empty token), but none between two `:` in a row (#385). That is
/// `split(':')` with its empty pieces dropped, except the first and the last.
fn tok_colon(clrs: &str) -> impl Iterator<Item = &str> {
    let n = clrs.split(':').count();
    clrs.split(':')
        .enumerate()
        .filter(move |&(i, t)| i == 0 || i + 1 == n || !t.is_empty())
        .map(|(_, t)| t)
}

/// `parseSegs`: `Err(1|2)` on error, `Ok((segs, rval))` with `rval` 0 or 3. Like C's
/// `static atomic_flag warned`, only the first bad spec reports (2, 3); later ones give 1
/// and 0. The flag is never reset, so on a thread that renders again a bad spec is drawn as
/// C's second one is.
pub(crate) fn parse_segs(clrs: &str) -> Result<(Vec<ColorSeg>, i32), i32> {
    thread_local! {
        static WARNED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    let test_and_set = || WARNED.with(|w| w.replace(true));
    let mut segs: Vec<ColorSeg> = Vec::new();
    let mut left = 1.0f64;
    let mut rval = 0;
    for tok in tok_colon(clrs) {
        let (color, v) = match tok.find(';') {
            None => (tok, 0.0),
            Some(i) => {
                let rest = &tok[i + 1..];
                match graphviz_pure_core::strtod::strtod_rest(rest) {
                    Some((v, _)) if v >= 0.0 => (&tok[..i], v),
                    _ => {
                        if test_and_set() {
                            return Err(1);
                        }
                        agerrorf(&format!(
                            "Illegal value in \"{clrs}\" color attribute; float expected after ';'\n"
                        ));
                        return Err(2);
                    }
                }
            }
        };
        let mut v = v;
        let del = v - left;
        if del > 0.0 {
            if !aeq0(del) && !test_and_set() {
                // C's message has no newline: the next one continues the line.
                agwarningf(&format!("Total size > 1 in \"{clrs}\" color spec "));
                rval = 3;
            }
            v = left;
        }
        left -= v;
        segs.push(ColorSeg {
            color: (!color.is_empty()).then(|| color.to_string()),
            t: v,
            has_fraction: v > 0.0,
        });
        if aeq0(left) {
            left = 0.0;
            break;
        }
    }
    if left > 0.0 {
        let nseg = segs.iter().filter(|s| s.t <= 0.0).count();
        if nseg > 0 {
            let delta = left / nseg as f64;
            for s in segs.iter_mut().filter(|s| s.t <= 0.0) {
                s.t = delta;
            }
        } else if let Some(last) = segs.last_mut() {
            last.t += left;
        }
    }
    while segs.last().is_some_and(|s| s.t <= 0.0) {
        segs.pop();
    }
    Ok((segs, rval))
}

/// `findStopColor`: `(clr0, clr1, frac)` for a two-color gradient spec.
pub(crate) fn find_stop_color(colorlist: &str) -> Option<(String, Option<String>, f64)> {
    let (segs, rv) = parse_segs(colorlist).ok()?;
    if rv != 0 || segs.len() < 2 || segs[0].color.is_none() {
        return None;
    }
    if segs.len() > 2 {
        agwarningf("More than 2 colors specified for a gradient - ignoring remaining\n");
    }
    let frac = if segs[0].has_fraction {
        segs[0].t
    } else if segs[1].has_fraction {
        1.0 - segs[1].t
    } else {
        0.0
    };
    Some((segs[0].color.clone().unwrap(), segs[1].color.clone(), frac))
}

fn overlap(a: BoundingBox, b: BoundingBox) -> bool {
    a.ll.x <= b.ur.x && a.ur.x >= b.ll.x && a.ll.y <= b.ur.y && a.ur.y >= b.ll.y
}

fn expand(bb: &mut BoundingBox, p: Point) {
    bb.ll.x = bb.ll.x.min(p.x);
    bb.ll.y = bb.ll.y.min(p.y);
    bb.ur.x = bb.ur.x.max(p.x);
    bb.ur.y = bb.ur.y.max(p.y);
}

/// `arrow_bb`.
fn arrow_bb(p: Point, u: Point, arrowsize: f64) -> BoundingBox {
    let mut u = Point::new(u.x - p.x, u.y - p.y);
    let s = 10.0 * arrowsize / (math::hypot(u.x, u.y) + 0.0001);
    u.x += if u.x >= 0.0 { 0.0001 } else { -0.0001 };
    u.y += if u.y >= 0.0 { 0.0001 } else { -0.0001 };
    u.x *= s;
    u.y *= s;
    let (ux2, uy2) = (u.x / 2.0, u.y / 2.0);
    let (ax, ay) = (p.x - uy2, p.y - ux2);
    let (bx, by) = (p.x + uy2, p.y + ux2);
    let (cx, cy) = (ax + u.x, ay + u.y);
    let (dx, dy) = (bx + u.x, by + u.y);
    BoundingBox::new(
        ax.min(bx.min(cx.min(dx))),
        ay.min(by.min(cy.min(dy))),
        ax.max(bx.max(cx.max(dx))),
        ay.max(by.max(cy.max(dy))),
    )
}

/// `init_splines_bb`.
fn splines_bb(bz: &[Bz]) -> Option<BoundingBox> {
    let first = bz.first()?;
    let bezier_bb = |b: &Bz| {
        let mut bb = BoundingBox {
            ll: b.list[0],
            ur: b.list[0],
        };
        let mut i = 1;
        while i + 2 < b.list.len() + 1 && i + 1 < b.list.len() {
            let (p1, p2) = (b.list[i], b.list[i + 1]);
            expand(
                &mut bb,
                Point::new((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0),
            );
            if let Some(p) = b.list.get(i + 2) {
                expand(&mut bb, *p);
            }
            i += 3;
        }
        bb
    };
    if first.list.is_empty() {
        return None;
    }
    let mut bb = bezier_bb(first);
    for (i, b) in bz.iter().enumerate() {
        if b.list.is_empty() {
            continue;
        }
        if i > 0 {
            let x = bezier_bb(b);
            expand(&mut bb, x.ll);
            expand(&mut bb, x.ur);
        }
        if b.sflag != 0 {
            let x = arrow_bb(b.sp, b.list[0], 1.0);
            expand(&mut bb, x.ll);
            expand(&mut bb, x.ur);
        }
        if b.eflag != 0 {
            let x = arrow_bb(b.ep, *b.list.last().unwrap(), 1.0);
            expand(&mut bb, x.ll);
            expand(&mut bb, x.ur);
        }
    }
    Some(bb)
}

fn label_overlaps(l: &TextLabel, b: BoundingBox) -> bool {
    let s = Point::new(l.dimen.x / 2.0, l.dimen.y / 2.0);
    overlap(
        BoundingBox::new(l.pos.x - s.x, l.pos.y - s.y, l.pos.x + s.x, l.pos.y + s.y),
        b,
    )
}

/// `ptToLine2` (`lib/common/geom.c:218`).
fn pt_to_line2(a: Point, b: Point, p: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut a2 = (p.y - a.y).mul_add(dx, -((p.x - a.x) * dy));
    a2 *= a2;
    if a2 < 0.000_000_000_1 {
        return 0.0;
    }
    a2 / dx.mul_add(dx, dy * dy)
}

/// `check_control_points` (`emit.c:745`).
fn check_control_points(cp: &[Point; 4]) -> bool {
    const HW: f64 = 2.0;
    pt_to_line2(cp[0], cp[3], cp[1]) < HW * HW && pt_to_line2(cp[0], cp[3], cp[2]) < HW * HW
}

/// `approx_bezier` (`emit.c:838`).
fn approx_bezier(cp: [Point; 4], segments: &mut Vec<Point>) {
    let mut stack = vec![(cp, 0_usize)];
    let mut budget = 1_u32 << 20;
    while let Some((cp, depth)) = stack.pop() {
        let ok = !cp.iter().all(|p| p.x.is_finite() && p.y.is_finite())
            || check_control_points(&cp)
            || depth >= (1 << 16)
            || budget == 0;
        if ok {
            if segments.is_empty() {
                segments.push(cp[0]);
            }
            segments.push(cp[3]);
        } else {
            budget -= 1;
            let (_, left, right) = super::arrows::bezier_split(&cp, 0.5);
            stack.push((right, depth + 1));
            stack.push((left, depth + 1));
        }
    }
}

/// `bisect` (`emit.c:851`).
fn bisect(pp: Point, cp: Point, np: Point) -> f64 {
    let theta = math::atan2(np.y - cp.y, np.x - cp.x);
    let phi = math::atan2(pp.y - cp.y, pp.x - cp.x);
    let mut ang = theta - phi;
    if ang > 0.0 {
        ang -= 2.0 * std::f64::consts::PI;
    }
    phi + ang / 2.0
}

/// `mkSegPts` (`emit.c:868`).
fn mk_seg_pts(prev: Option<Point>, cur: Point, next: Option<Point>, w2: f64) -> (Point, Point) {
    let cp = cur;
    let (pp, np) = match (prev, next) {
        (Some(pp), Some(np)) => (pp, np),
        (Some(pp), None) => (pp, Point::new(2.0 * (cp.x - pp.x), 2.0 * (cp.y - pp.y))),
        (None, Some(np)) => (Point::new(2.0 * (cp.x - np.x), 2.0 * (cp.y - np.y)), np),
        (None, None) => (cp, cp),
    };
    let theta = bisect(pp, cp, np);
    let del = Point::new(w2 * math::cos(theta), w2 * math::sin(theta));
    (
        Point::new(cp.x + del.x, cp.y + del.y),
        Point::new(cp.x - del.x, cp.y - del.y),
    )
}

/// `map_output_bspline` (`emit.c:902-930`).
fn map_output_bspline(b: &Bz, w2: f64) -> Vec<Vec<Point>> {
    let mut segments = Vec::new();
    let nc = b.list.len().saturating_sub(1) / 3;
    for j in 0..nc {
        let pts = [
            b.list[3 * j],
            b.list[3 * j + 1],
            b.list[3 * j + 2],
            b.list[3 * j + 3],
        ];
        approx_bezier(pts, &mut segments);
    }
    let mut polys = Vec::new();
    let mut pt1 = [Point::default(); 50];
    let mut pt2 = [Point::default(); 50];
    let mut cnt = 0;
    for i in 0..segments.len() {
        let prev = (i > 0).then(|| segments[i - 1]);
        let next = segments.get(i + 1).copied();
        let (p1, p2) = mk_seg_pts(prev, segments[i], next, w2);
        pt1[cnt] = p1;
        pt2[cnt] = p2;
        cnt += 1;
        if i + 1 == segments.len() || cnt == 50 {
            let mut poly = Vec::with_capacity(2 * cnt);
            poly.extend_from_slice(&pt1[..cnt]);
            for k in 0..cnt {
                poly.push(pt2[cnt - k - 1]);
            }
            polys.push(poly);
            pt1[0] = pt1[cnt - 1];
            pt2[0] = pt2[cnt - 1];
            cnt = 1;
        }
    }
    polys
}

/// `interpretCRNL`.
fn interpret_crnl(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bs = false;
    for c in s.chars() {
        if bs {
            out.push(match c {
                'n' | 'l' => '\n',
                'r' => '\r',
                c => c,
            });
            bs = false;
        } else if c == '\\' {
            bs = true;
        } else {
            out.push(c);
        }
    }
    out
}

impl<'a, E: Engine> Emitter<'a, E> {
    /// Substitution context for an object.
    pub(crate) fn ctx(&self, kind: ObjKind) -> LabelSubstContext<'a> {
        let m = self.job.m;
        match kind {
            ObjKind::Root => LabelSubstContext {
                graph_name: m.g.root_name(),
                label_text: m.label.as_ref().map_or("", |l| l.text.as_str()),
                has_label: m.label.is_some(),
                ..Default::default()
            },
            ObjKind::Cluster(c) => LabelSubstContext {
                graph_name: &m.clusters[c].name,
                label_text: m.clusters[c].label.as_ref().map_or("", |l| l.text.as_str()),
                has_label: m.clusters[c].label.is_some(),
                ..Default::default()
            },
            ObjKind::Node(n) => LabelSubstContext {
                graph_name: m.g.root_name(),
                node_name: m.nodes[n].name,
                in_buffer: NamesInBuffer {
                    graph: m.g.agnameof(Obj::Root).buffer,
                    node: m.node_agnameof(n).buffer,
                    ..Default::default()
                },
                is_node: true,
                label_text: &m.nodes[n].label.text,
                has_label: true,
                ..Default::default()
            },
            ObjKind::Edge(e) => {
                let ed = &m.edges[e];
                LabelSubstContext {
                    graph_name: m.g.root_name(),
                    tail_name: m.nodes[ed.tail].name,
                    head_name: m.nodes[ed.head].name,
                    in_buffer: NamesInBuffer {
                        graph: m.g.agnameof(Obj::Root).buffer,
                        tail: m.node_agnameof(ed.tail).buffer,
                        head: m.node_agnameof(ed.head).buffer,
                        ..Default::default()
                    },
                    is_edge: true,
                    is_directed: m.directed,
                    tail_port: ed.tail_port,
                    head_port: ed.head_port,
                    label_text: ed.label.as_ref().map_or("", |l| l.text.as_str()),
                    has_label: ed.label.is_some(),
                    ..Default::default()
                }
            }
        }
    }

    pub(crate) fn obj_attrs(&self, kind: ObjKind) -> Attrs<'a> {
        let m = self.job.m;
        match kind {
            ObjKind::Root => m.root_attrs(),
            ObjKind::Cluster(c) => m.clusters[c].attrs,
            ObjKind::Node(n) => m.nodes[n].attrs,
            ObjKind::Edge(e) => m.edges[e].attrs,
        }
    }

    /// `getObjId`.
    fn obj_id(&self, kind: ObjKind) -> String {
        let a = self.obj_attrs(kind);
        let mut s = self.layer_page_prefix();
        if let Some(id) = a.nonempty("id") {
            s.push_str(id);
            return s;
        }
        if kind != ObjKind::Root
            && let Some(gid) = self.job.m.root_attrs().nonempty("id")
        {
            s.push_str(gid);
            s.push('_');
        }
        match kind {
            ObjKind::Root => s.push_str("graph0"),
            ObjKind::Cluster(c) => {
                s.push_str(&format!("clust{}", self.job.m.clusters[c].seq));
            }
            ObjKind::Node(n) => s.push_str(&format!("node{}", self.job.m.node_seq(n))),
            ObjKind::Edge(e) => s.push_str(&format!("edge{}", self.job.m.edge_seq(e))),
        }
        s
    }

    fn preprocess_tooltip(&self, s: &str) -> String {
        interpret_crnl(&convert_label_text(
            s,
            graphviz_pure_core::charset::current(),
        ))
    }

    /// `initObjMapData`.
    fn init_obj_map_data(&mut self, label: Option<String>, kind: ObjKind) {
        let a = self.obj_attrs(kind);
        let ctx = self.ctx(kind);
        let url = a.nonempty("href").or_else(|| a.nonempty("URL"));
        let tooltip = a.get("tooltip").map(|t| self.preprocess_tooltip(t));
        let target = a.get("target");
        let id = self.obj_id(kind);
        let flags = self.job.flags;
        let o = self.job.obj_mut();
        if flags & GVRENDER_DOES_LABELS != 0 {
            o.label = label;
        }
        if flags & GVRENDER_DOES_MAPS != 0 {
            o.id = Some(strdup_and_subst_obj(&id, &ctx));
            if let Some(u) = url {
                o.url = Some(strdup_and_subst_obj(u, &ctx));
            }
        }
        if flags & GVRENDER_DOES_TOOLTIPS != 0 {
            if let Some(t) = tooltip.filter(|t| !t.is_empty()) {
                o.tooltip = Some(strdup_and_subst_obj(&t, &ctx));
                o.explicit_tooltip = true;
            } else if let Some(l) = &o.label {
                o.tooltip = Some(l.clone());
            }
        }
        if flags & GVRENDER_DOES_TARGETS != 0
            && let Some(t) = target.filter(|t| !t.is_empty())
        {
            o.target = Some(strdup_and_subst_obj(t, &ctx));
        }
    }

    fn set_scheme(&mut self, s: Option<&str>) -> Option<String> {
        std::mem::replace(&mut self.job.color_scheme, s.map(str::to_string))
    }

    /// `emit_map_rect` (`emit.c:930-949`).
    pub(crate) fn emit_map_rect(&mut self, b: BoundingBox) {
        let flags = self.job.flags;
        if flags & (GVRENDER_DOES_MAPS | GVRENDER_DOES_TOOLTIPS) == 0 {
            return;
        }
        if flags & GVRENDER_DOES_MAP_RECTANGLE != 0 {
            let mut p = vec![b.ll, b.ur];
            if flags & GVRENDER_DOES_TRANSFORM == 0 {
                self.job.ptf_a(&mut p);
            }
            let o = self.job.obj_mut();
            o.url_map_shape = MapShape::Rectangle;
            o.url_map_p = p;
        } else if flags & GVRENDER_DOES_MAP_POLYGON != 0 {
            let mut p = rect2poly([b.ll, b.ur]).to_vec();
            if flags & GVRENDER_DOES_TRANSFORM == 0 {
                self.job.ptf_a(&mut p);
            }
            let o = self.job.obj_mut();
            o.url_map_shape = MapShape::Polygon;
            o.url_map_p = p;
        }
    }

    /// `map_label` (`emit.c:951-971`).
    fn map_label(&mut self, lab: &TextLabel) {
        let b = BoundingBox::new(
            lab.pos.x - lab.dimen.x / 2.0,
            lab.pos.y - lab.dimen.y / 2.0,
            lab.pos.x + lab.dimen.x / 2.0,
            lab.pos.y + lab.dimen.y / 2.0,
        );
        self.emit_map_rect(b);
    }

    /// `emit_graph`.
    pub(crate) fn emit_graph(&mut self) {
        let m = self.job.m;
        self.eng.begin_job(&self.job);
        let comment = m.root_attrs().get_or_empty("comment");
        // gvrender_comment needs an object state only for engines that look at it
        self.job.push_obj(ObjKind::Root);
        self.job.pop_obj();
        if !comment.is_empty() {
            self.job.push_obj(ObjKind::Root);
            self.eng.comment(&self.job, comment);
            self.job.pop_obj();
        }
        // emit_begin_graph
        self.job.push_obj(ObjKind::Root);
        self.job.obj_mut().emit_state = EmitState::GDraw;
        let label = m.label.as_ref().map(|l| l.text.clone());
        self.init_obj_map_data(label, ObjKind::Root);
        self.eng.begin_graph(&self.job);
        self.init_layering();
        // firstlayer / validlayer / nextlayer
        self.job.num_layers = self.job.num_layers.max(1);
        let does_layers = self.job.flags & super::job::GVDEVICE_DOES_LAYERS != 0;
        let mut list: Option<Vec<i32>> = self.job.layer_list.clone();
        if let Some(l) = &mut list {
            if l.len() > 1 && !does_layers {
                agwarningf(&format!(
                    "layers not supported in {} output\n",
                    self.eng.output_langname()
                ));
                l.truncate(1);
            }
        } else if self.job.num_layers > 1 && !does_layers {
            agwarningf(&format!(
                "layers not supported in {} output\n",
                self.eng.output_langname()
            ));
            self.job.num_layers = 1;
        }
        let layers: Vec<i32> = match &list {
            Some(l) => l.clone(),
            None => (1..=self.job.num_layers).collect(),
        };
        let physical = layers.len();
        for ln in layers {
            self.job.layer_num = ln;
            if physical > 1 {
                let name = self
                    .job
                    .layer_ids
                    .get(ln as usize)
                    .cloned()
                    .unwrap_or_default();
                self.eng.begin_layer(&self.job, &name);
            }
            self.emit_page();
            if physical > 1 {
                self.eng.end_layer(&self.job);
            }
        }
        self.eng.end_graph(&self.job);
        self.job.pop_obj();
    }

    /// `init_layering` / `parse_layers` / `parse_layerselect`.
    fn init_layering(&mut self) {
        let a = self.job.m.root_attrs();
        let Some(layers) = a.get("layers") else {
            self.job.num_layers = 1;
            return;
        };
        let encode = |s: &str| graphviz_pure_core::charset::encode(s).into_owned();
        let delims = encode(a.get("layersep").unwrap_or(":\t "));
        let mut list_delims = encode(a.get("layerlistsep").unwrap_or(","));
        // strpbrk(layerDelims, layerListDelims), on C's bytes.
        if let Some(&tok) = delims.iter().find(|b| list_delims.contains(b)) {
            // `'%c'` of `*tok`: one byte, which need not be a whole character.
            let tok = match tok {
                b if b.is_ascii() => b as char,
                b => graphviz_pure_core::charset::raw_char(b),
            };
            agwarningf(&format!(
                "The character '{tok}' appears in both the layersep and layerlistsep attributes - layerlistsep ignored.\n"
            ));
            list_delims.clear();
        }
        let id_bytes: Vec<Vec<u8>> = std::iter::once(Vec::new())
            .chain(strtok(&encode(layers), &delims).map(<[u8]>::to_vec))
            .collect();
        self.job.num_layers = (id_bytes.len() - 1) as i32;
        self.job.layer_ids = id_bytes
            .iter()
            .map(|b| graphviz_pure_core::charset::decode_slice(b))
            .collect();
        self.job.layer_id_bytes = id_bytes;
        self.job.layer_delims = delims;
        self.job.layer_list_delims = list_delims;
        if let Some(sel) = a.nonempty("layerselect") {
            let n = self.job.num_layers;
            let l: Vec<i32> = (1..=n)
                .filter(|&i| self.selected_layer(i, n, sel))
                .collect();
            if l.is_empty() {
                agwarningf(&format!(
                    "The layerselect attribute \"{sel}\" does not match any layer specifed by the layers attribute - ignored.\n"
                ));
            } else {
                self.job.layer_list = Some(l);
            }
        }
    }

    fn layer_index(&self, s: &[u8], all: i32) -> i32 {
        if s == b"all" {
            return all;
        }
        if s.iter().all(u8::is_ascii_digit) {
            return super::attrs::atoi(std::str::from_utf8(s).unwrap_or_default()) as i32;
        }
        for (i, id) in self.job.layer_id_bytes.iter().enumerate().skip(1) {
            if id == s {
                return i as i32;
            }
        }
        -1
    }

    /// `selectedLayer`.
    fn selected_layer(&self, layer: i32, num: i32, spec: &str) -> bool {
        let spec = graphviz_pure_core::charset::encode(spec);
        for part in strtok(&spec, &self.job.layer_list_delims) {
            let mut ws = strtok(part, &self.job.layer_delims);
            let r = match (ws.next(), ws.next()) {
                (Some(a), Some(b)) => {
                    let (mut n0, mut n1) = (self.layer_index(a, 0), self.layer_index(b, num));
                    if n0 >= 0 || n1 >= 0 {
                        if n0 > n1 {
                            std::mem::swap(&mut n0, &mut n1);
                        }
                        n0 <= layer && layer <= n1
                    } else {
                        false
                    }
                }
                (Some(a), None) => self.layer_index(a, layer) == layer,
                _ => false,
            };
            if r {
                return true;
            }
        }
        false
    }

    fn selectedlayer(&self, spec: &str) -> bool {
        self.selected_layer(self.job.layer_num, self.job.num_layers, spec)
    }

    pub(crate) fn node_in_layer(&self, n: usize) -> bool {
        if self.job.num_layers <= 1 {
            return true;
        }
        let m = self.job.m;
        let pn = m.nodes[n].attrs.late_string("layer", "");
        if self.selectedlayer(pn) {
            return true;
        }
        if !pn.is_empty() {
            return false;
        }
        let edges: Vec<usize> = (0..m.edges.len())
            .filter(|&e| m.edges[e].tail == n || m.edges[e].head == n)
            .collect();
        if edges.is_empty() {
            return true;
        }
        edges.iter().any(|&e| {
            let pe = m.edges[e].attrs.late_string("layer", "");
            pe.is_empty() || self.selectedlayer(pe)
        })
    }

    fn edge_in_layer(&self, e: usize) -> bool {
        if self.job.num_layers <= 1 {
            return true;
        }
        let m = self.job.m;
        let pe = m.edges[e].attrs.late_string("layer", "");
        if self.selectedlayer(pe) {
            return true;
        }
        if !pe.is_empty() {
            return false;
        }
        [m.edges[e].tail, m.edges[e].head].iter().any(|&n| {
            let pn = m.nodes[n].attrs.late_string("layer", "");
            pn.is_empty() || self.selectedlayer(pn)
        })
    }

    fn clust_in_layer(&self, c: usize) -> bool {
        if self.job.num_layers <= 1 {
            return true;
        }
        let m = self.job.m;
        let pg = m.clusters[c].attrs.late_string("layer", "");
        if self.selectedlayer(pg) {
            return true;
        }
        if !pg.is_empty() {
            return false;
        }
        m.clusters[c].nodes.iter().any(|&n| self.node_in_layer(n))
    }

    /// `layerPagePrefix`.
    pub(crate) fn layer_page_prefix(&self) -> String {
        if self.job.layer_num > 1 {
            format!(
                "{}_",
                self.job
                    .layer_ids
                    .get(self.job.layer_num as usize)
                    .cloned()
                    .unwrap_or_default()
            )
        } else {
            String::new()
        }
    }

    fn emit_page(&mut self) {
        let m = self.job.m;
        let saved_id = if self.job.layer_num > 1 {
            let prefix = self.layer_page_prefix();
            let o = self.job.obj_mut();
            let save = o.id.clone();
            o.id = Some(format!("{prefix}{}", save.as_deref().unwrap_or("layer")));
            Some(save)
        } else {
            None
        };
        let prev = self.set_scheme(m.root_attrs().get("colorscheme"));
        let flags = self.job.flags;
        let anchored = {
            let o = self.job.obj();
            o.url.is_some() || o.explicit_tooltip
        };
        if flags & (GVRENDER_DOES_MAPS | GVRENDER_DOES_TOOLTIPS) != 0 && anchored {
            let pb = self.job.page_box;
            if flags & GVRENDER_DOES_MAP_RECTANGLE != 0 {
                let mut p = vec![pb.ll, pb.ur];
                if flags & GVRENDER_DOES_TRANSFORM == 0 {
                    self.job.ptf_a(&mut p);
                }
                let o = self.job.obj_mut();
                o.url_map_shape = MapShape::Rectangle;
                o.url_map_p = p;
            } else if flags & GVRENDER_DOES_MAP_POLYGON != 0 {
                let mut p = rect2poly([pb.ll, pb.ur]).to_vec();
                if flags & GVRENDER_DOES_TRANSFORM == 0 {
                    self.job.ptf_a(&mut p);
                }
                let o = self.job.obj_mut();
                o.url_map_shape = MapShape::Polygon;
                o.url_map_p = p;
            }
        }
        self.eng.begin_page(&self.job);
        self.set_pencolor(DEFAULT_COLOR);
        self.set_fillcolor(DEFAULT_FILL);
        if flags & GVRENDER_DOES_LABELS != 0
            && let Some(l) = &m.label
        {
            self.job.obj_mut().label = Some(l.text.clone());
        }
        if flags & EMIT_CLUSTERS_LAST == 0 && anchored {
            self.emit_map_rect(self.job.clip);
            self.begin_obj_anchor();
        }
        self.emit_background();
        if let Some(l) = &m.label {
            self.emit_label(EmitState::GLabel, l);
        }
        if flags & EMIT_CLUSTERS_LAST == 0 && anchored {
            self.end_anchor();
        }
        self.emit_view();
        self.eng.end_page(&self.job);
        if let Some(save) = saved_id {
            self.job.obj_mut().id = save;
        }
        self.set_scheme(prev.as_deref());
    }

    fn emit_background(&mut self) {
        let m = self.job.m;
        let a = m.root_attrs();
        let (mut s, mut dflt) = match a.nonempty("bgcolor") {
            Some(s) => (s, false),
            None => ("white", true),
        };
        let flags = self.job.flags;
        if flags & GVDEVICE_DOES_TRUECOLOR == 0 && s == "transparent" {
            s = "white";
            dflt = true;
        }
        if !((flags & GVDEVICE_DOES_TRUECOLOR != 0 && s == "transparent")
            || (flags & GVRENDER_NO_WHITE_BG != 0 && dflt))
        {
            let clip = self.job.clip;
            if let Some((c0, c1, frac)) = find_stop_color(s) {
                self.set_fillcolor(&c0);
                self.set_pencolor("transparent");
                let radial = a
                    .nonempty("style")
                    .is_some_and(|st| parse_style(st).iter().any(|i| i.name == "radial"));
                let angle = a.late_int("gradientangle", 0, 0);
                self.set_gradient_vals(c1.as_deref().unwrap_or(DEFAULT_COLOR), angle, frac);
                self.boxf(clip, if radial { RGRADIENT } else { GRADIENT });
            } else {
                self.set_fillcolor(s);
                self.set_pencolor("transparent");
                self.boxf(clip, FILL);
            }
        }
        if let Some(xd) = &m.g.layout.background {
            self.emit_xdot(xd);
        }
    }

    /// `emit_xdot`: draws the `_background` ops `init_xdot` parsed. An op is drawn when its
    /// `bb` overlaps the clip box; outside `neato -n` (`xdotBB`) every `bb` is the zero box.
    fn emit_xdot(&mut self, xd: &graphviz_pure_core::xdot::Background) {
        use graphviz_pure_core::xdot::{Grad, Op};
        let pts = |v: &[(f64, f64)]| v.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>();
        let mut image_warn = true;
        let mut styled = false;
        let mut filled = FILL;
        for (op, b) in &xd.ops {
            let bb = BoundingBox {
                ll: Point::new(b.ll.0, b.ll.1),
                ur: Point::new(b.ur.0, b.ur.1),
            };
            let visible = overlap(bb, self.job.clip);
            match op {
                Op::Ellipse(f, [x, y, w, h]) => {
                    if visible {
                        let a = [Point::new(x - w, y - h), Point::new(x + w, y + h)];
                        self.ellipse(a, if *f { filled } else { 0 });
                    }
                }
                Op::Polygon(f, v) => {
                    if visible {
                        self.polygon(&pts(v), if *f { filled } else { 0 });
                    }
                }
                Op::Bezier(f, v) => {
                    if visible {
                        self.bezier(&pts(v), if *f { filled } else { 0 });
                    }
                }
                Op::Polyline(v) => {
                    if visible {
                        self.polyline(&pts(v));
                    }
                }
                Op::Text(x, y, align, _, text) => {
                    // Only `xdotBB` makes the span; without it C passes NULL to
                    // `gvrender_textspan` and crashes. The port draws no text there.
                    if visible && let Some(sp) = &b.span {
                        let span = super::model::TextSpan {
                            str: text.clone(),
                            just: match align.signum() {
                                -1 => 'l',
                                1 => 'r',
                                _ => 'n',
                            },
                            width: sp.width,
                            height: sp.height,
                            yoffset_centerline: sp.yoffset_centerline,
                            font: super::model::TextFont::new(
                                &sp.fontname,
                                sp.fontsize,
                                sp.fontflags as u8,
                            ),
                        };
                        self.textspan(Point::new(*x, *y), &span);
                    }
                }
                Op::Color(true, Grad::None(c)) => {
                    self.set_fillcolor(c);
                    filled = FILL;
                }
                Op::Color(false, Grad::None(c)) => {
                    self.set_pencolor(c);
                    filled = FILL;
                }
                Op::Color(true, Grad::Radial(x0, y0, r0, x1, y1, _, stops)) => {
                    let angle = if x1 == x0 && y1 == y0 {
                        0
                    } else {
                        (180.0 * math::acos((x0 - x1) / r0) / std::f64::consts::PI) as i32
                    };
                    if self.xdot_gradient(stops, angle) {
                        filled = RGRADIENT;
                    }
                }
                Op::Color(true, Grad::Linear(x0, y0, x1, y1, stops)) => {
                    let angle =
                        (180.0 * math::atan2(y1 - y0, x1 - x0) / std::f64::consts::PI) as i32;
                    if self.xdot_gradient(stops, angle) {
                        filled = GRADIENT;
                    }
                }
                Op::Color(false, _) => agwarningf("gradient pen colors not yet supported.\n"),
                Op::Style(st) => {
                    let items = parse_style(st);
                    self.set_style(&items);
                    styled = true;
                }
                Op::Image => {
                    if image_warn {
                        agwarningf("Images unsupported in \"background\" attribute\n");
                        image_warn = false;
                    }
                }
                Op::Font(..) | Op::FontChar(_) => {}
            }
        }
        if styled {
            self.set_style(&default_line_style());
        }
    }

    /// The fill and gradient colours of an `xd_grad_fill_color` op. C reads `stops[0]` and
    /// `stops[1]` unchecked; with fewer stops the port leaves the colours as they are.
    fn xdot_gradient(&mut self, stops: &[(f64, String)], angle: i32) -> bool {
        let [(_, c0), (frac, c1), ..] = stops else {
            return false;
        };
        self.set_fillcolor(c0);
        self.set_gradient_vals(c1, angle, *frac);
        true
    }

    fn emit_view(&mut self) {
        let m = self.job.m;
        let flags = self.job.flags;
        let mut drawn = vec![false; m.nodes.len()];
        if flags & EMIT_CLUSTERS_LAST == 0 {
            self.emit_clusters(&m.root_clusters);
        }
        if flags & EMIT_SORTED != 0 {
            for n in 0..m.nodes.len() {
                self.emit_node(n, &mut drawn);
            }
            for n in 0..m.nodes.len() {
                for &e in &m.out_edges[n] {
                    self.emit_edge(e);
                }
            }
        } else if flags & EMIT_EDGE_SORTED != 0 {
            for n in 0..m.nodes.len() {
                for &e in &m.out_edges[n] {
                    self.emit_edge(e);
                }
            }
            for n in 0..m.nodes.len() {
                self.emit_node(n, &mut drawn);
            }
        } else {
            for n in 0..m.nodes.len() {
                self.emit_node(n, &mut drawn);
                for &e in &m.out_edges[n] {
                    self.emit_node(m.edges[e].head, &mut drawn);
                    self.emit_edge(e);
                }
            }
        }
        if flags & EMIT_CLUSTERS_LAST != 0 {
            self.emit_clusters(&m.root_clusters);
        }
    }

    fn emit_clusters(&mut self, list: &[usize]) {
        let m = self.job.m;
        for &c in list {
            if !self.clust_in_layer(c) {
                continue;
            }
            let cl = &m.clusters[c];
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.emit_clusters(&cl.children);
            }
            let a = cl.attrs;
            self.job.push_obj(ObjKind::Cluster(c));
            self.job.obj_mut().emit_state = EmitState::CDraw;
            self.init_obj_map_data(
                cl.label.as_ref().map(|l| l.text.clone()),
                ObjKind::Cluster(c),
            );
            self.eng.begin_cluster(&self.job);
            let anchored = {
                let o = self.job.obj();
                o.url.is_some() || o.explicit_tooltip
            };
            let prev = self.set_scheme(a.get("colorscheme"));
            if anchored && self.job.flags & EMIT_CLUSTERS_LAST == 0 {
                self.emit_map_rect(cl.bb);
                self.begin_obj_anchor();
            }
            let mut filled = 0;
            let mut rounded = false;
            let mut striped = false;
            let mut radial = false;
            if let Some(st) = a.nonempty("style") {
                let mut items = parse_style(st);
                let mut kept = Vec::new();
                for it in items.drain(..) {
                    match it.name.as_str() {
                        "filled" => {
                            filled = FILL;
                            kept.push(it);
                        }
                        "radial" => {
                            filled = FILL;
                            radial = true;
                        }
                        "striped" => striped = true,
                        "rounded" => rounded = true,
                        _ => kept.push(it),
                    }
                }
                self.set_style(&kept);
            }
            let mut pencolor: Option<&str> = None;
            let mut fillcolor: Option<&str> = None;
            if let Some(c) = a.nonempty("color") {
                pencolor = Some(c);
                fillcolor = Some(c);
            }
            if let Some(c) = a.nonempty("pencolor") {
                pencolor = Some(c);
            }
            if let Some(c) = a.nonempty("fillcolor") {
                fillcolor = Some(c);
            }
            if (filled == 0 || fillcolor.is_none())
                && let Some(c) = a.nonempty("bgcolor")
            {
                fillcolor = Some(c);
                filled = FILL;
            }
            let pencolor = pencolor.unwrap_or(DEFAULT_COLOR);
            let fillcolor = fillcolor.unwrap_or(DEFAULT_FILL);
            if filled != 0 {
                if let Some((c0, c1, frac)) = find_stop_color(fillcolor) {
                    self.set_fillcolor(&c0);
                    let angle = a.late_int("gradientangle", 0, 0);
                    self.set_gradient_vals(c1.as_deref().unwrap_or(DEFAULT_COLOR), angle, frac);
                    filled = if radial { RGRADIENT } else { GRADIENT };
                } else {
                    self.set_fillcolor(fillcolor);
                }
            }
            if a.nonempty("penwidth").is_some() {
                let pw = a.late_double("penwidth", 1.0, 0.0);
                self.set_penwidth(pw);
            }
            let bb = cl.bb;
            let peripheries = a.late_int("peripheries", 1, 0);
            let af = [
                bb.ll,
                Point::new(bb.ur.x, bb.ll.y),
                bb.ur,
                Point::new(bb.ll.x, bb.ur.y),
            ];
            if rounded {
                if peripheries != 0 || filled != 0 {
                    if peripheries != 0 {
                        self.set_pencolor(pencolor);
                    } else {
                        self.set_pencolor("transparent");
                    }
                    let style = super::shapes::PolyStyle {
                        rounded: true,
                        filled: filled != 0,
                        radial,
                        striped,
                        ..Default::default()
                    };
                    self.round_corners(&af, style, filled);
                }
            } else if striped {
                if peripheries == 0 {
                    self.set_pencolor("transparent");
                } else {
                    self.set_pencolor(pencolor);
                }
                if self.striped_box(&af, fillcolor, false) > 1 {
                    agerr_prev(&format!("in cluster {}\n", cl.name));
                }
                self.boxf(bb, 0);
            } else if peripheries != 0 {
                self.set_pencolor(pencolor);
                self.boxf(bb, filled);
            } else if filled != 0 {
                self.set_pencolor("transparent");
                self.boxf(bb, filled);
            }
            if let Some(l) = &cl.label {
                self.emit_label(EmitState::CLabel, l);
            }
            if anchored {
                if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                    self.emit_map_rect(cl.bb);
                    self.begin_obj_anchor();
                }
                self.end_anchor();
            }
            self.eng.end_cluster(&self.job);
            self.job.pop_obj();
            if self.job.flags & EMIT_CLUSTERS_LAST == 0 {
                self.emit_clusters(&cl.children);
            }
            self.set_scheme(prev.as_deref());
        }
    }

    /// `stripedBox`.
    pub(crate) fn striped_box(&mut self, af: &[Point], clrs: &str, rotate: bool) -> i32 {
        let (segs, rv) = match parse_segs(clrs) {
            Ok(x) => x,
            Err(e) => return e,
        };
        let mut pts = if rotate {
            [af[2], af[3], af[0], af[1]]
        } else {
            [af[0], af[1], af[2], af[3]]
        };
        let lastx = pts[1].x;
        let xdelta = pts[1].x - pts[0].x;
        pts[1].x = pts[0].x;
        pts[2].x = pts[0].x;
        let save = self.job.obj().penwidth;
        if save > 0.5 {
            self.set_penwidth(0.5);
        }
        let n = segs.len();
        for (i, s) in segs.iter().enumerate() {
            let Some(c) = &s.color else { break };
            if s.t <= 0.0 {
                continue;
            }
            self.set_fillcolor(c);
            let x = if i + 1 == n {
                lastx
            } else {
                // C (clang) fuses `pts[0].x + xdelta * s.t`.
                xdelta.mul_add(s.t, pts[0].x)
            };
            pts[1].x = x;
            pts[2].x = x;
            self.polygon(&pts, FILL);
            pts[0].x = pts[1].x;
            pts[3].x = pts[1].x;
        }
        if save > 0.5 {
            self.set_penwidth(save);
        }
        rv
    }

    fn emit_node(&mut self, n: usize, drawn: &mut [bool]) {
        let m = self.job.m;
        let nd = &m.nodes[n];
        if drawn[n] || m.g.nodes[n].layout.clust_node {
            return;
        }
        let half = Point::new(nd.lw, nd.ht / 2.0);
        let nbb = BoundingBox::new(
            nd.coord.x - half.x,
            nd.coord.y - half.y,
            nd.coord.x + nd.rw,
            nd.coord.y + half.y,
        );
        if !self.node_in_layer(n) || !overlap(nbb, self.job.clip) {
            return;
        }
        drawn[n] = true;
        self.job.push_obj(ObjKind::Node(n));
        self.comment(nd.name);
        let c = nd.attrs.get_or_empty("comment");
        self.comment(c);
        self.job.pop_obj();
        if let Some(st) = nd.attrs.nonempty("style")
            && parse_style(st).iter().any(|i| i.name == "invis")
        {
            return;
        }
        // emit_begin_node
        self.job.push_obj(ObjKind::Node(n));
        self.job.obj_mut().emit_state = EmitState::NDraw;
        self.init_obj_map_data(Some(nd.label.text.clone()), ObjKind::Node(n));
        let anchored = {
            let o = self.job.obj();
            o.url.is_some() || o.explicit_tooltip
        };
        let flags = self.job.flags;
        if flags & (GVRENDER_DOES_MAPS | GVRENDER_DOES_TOOLTIPS) != 0 && anchored {
            let filled = nd
                .attrs
                .nonempty("style")
                .is_some_and(|st| parse_style(st).iter().any(|i| i.name == "filled"));
            let coord = nd.coord;
            let is_rect_node = nd
                .geom
                .polygon
                .as_ref()
                .is_some_and(|poly| is_rect(poly) && (poly.peripheries > 0 || filled));
            let (mut shape, mut p) = if let Some(poly) = &nd.geom.polygon
                && !is_rect_node
                && flags & GVRENDER_DOES_MAP_POLYGON != 0
            {
                let sides = if poly.sides < 3 { 1 } else { poly.sides };
                let peripheries = if poly.peripheries < 2 {
                    1
                } else {
                    poly.peripheries
                };
                let nump_int = nd.attrs.get("samplepoints").map_or(0, super::attrs::atoi);
                let nump = if !(4..=60).contains(&nump_int) {
                    20
                } else {
                    nump_int as usize
                };
                if poly.peripheries == 0 && !filled {
                    (
                        MapShape::Rectangle,
                        vec![
                            Point::new(coord.x - nd.lw, coord.y - nd.ht / 2.0),
                            Point::new(coord.x + nd.lw, coord.y + nd.ht / 2.0),
                        ],
                    )
                } else if poly.sides < 3
                    && is_exactly_zero(poly.skew)
                    && is_exactly_zero(poly.distortion)
                {
                    let rad = poly.vertices[2 * peripheries - 1];
                    if poly.regular {
                        (
                            MapShape::Circle,
                            vec![coord, Point::new(coord.x + rad.x, coord.y + rad.y)],
                        )
                    } else {
                        let mut pts = p_ellipse(rad.x, rad.y, nump);
                        for pt in &mut pts {
                            pt.x += coord.x;
                            pt.y += coord.y;
                        }
                        (MapShape::Polygon, pts)
                    }
                } else {
                    let offset = (peripheries - 1) * poly.sides;
                    if poly.sides >= nump {
                        let delta = poly.sides / nump;
                        let mut pts = Vec::with_capacity(nump);
                        let mut j = 0;
                        for _ in 0..nump {
                            let v = poly.vertices[j + offset];
                            pts.push(Point::new(coord.x + v.x, coord.y + v.y));
                            j += delta;
                        }
                        (MapShape::Polygon, pts)
                    } else {
                        let mut pts = Vec::with_capacity(sides);
                        for j in 0..sides {
                            let v = poly.vertices[j + offset];
                            pts.push(Point::new(coord.x + v.x, coord.y + v.y));
                        }
                        (MapShape::Polygon, pts)
                    }
                }
            } else {
                (
                    MapShape::Rectangle,
                    vec![
                        Point::new(coord.x - nd.lw, coord.y - nd.ht / 2.0),
                        Point::new(coord.x + nd.rw, coord.y + nd.ht / 2.0),
                    ],
                )
            };
            if shape == MapShape::Rectangle && flags & GVRENDER_DOES_MAP_RECTANGLE == 0 {
                shape = MapShape::Polygon;
                p = rect2poly([p[0], p[1]]).to_vec();
            }
            if flags & GVRENDER_DOES_TRANSFORM == 0 {
                self.job.ptf_a(&mut p);
            }
            let o = self.job.obj_mut();
            o.url_map_shape = shape;
            o.url_map_p = p;
        }
        let prev = self.set_scheme(nd.attrs.get("colorscheme"));
        self.eng.begin_node(&self.job);
        self.node_gencode(n);
        if let Some(xl) = &nd.xlabel
            && xl.set
        {
            self.emit_label(EmitState::NLabel, xl);
        }
        self.eng.end_node(&self.job);
        self.set_scheme(prev.as_deref());
        self.job.pop_obj();
    }

    /// `emit_label`.
    pub(crate) fn emit_label(&mut self, state: EmitState, lp: &TextLabel) {
        let old = self.job.obj().emit_state;
        self.job.obj_mut().emit_state = state;
        if lp.is_html {
            self.emit_html_label(lp);
            self.job.obj_mut().emit_state = old;
            return;
        }
        if lp.spans.is_empty() {
            return;
        }
        self.set_pencolor(&lp.fontcolor);
        let mut p = Point::new(0.0, 0.0);
        p.y = match lp.valign {
            't' => lp.pos.y + lp.space.y / 2.0 - lp.fontsize,
            'b' => lp.pos.y - lp.space.y / 2.0 + lp.dimen.y - lp.fontsize,
            _ => lp.pos.y + lp.dimen.y / 2.0 - lp.fontsize,
        };
        if self.job.obj().labeledgealigned {
            p.y -= lp.pos.y;
        }
        for span in &lp.spans {
            p.x = match span.just {
                'l' => lp.pos.x - lp.space.x / 2.0,
                'r' => lp.pos.x + lp.space.x / 2.0,
                _ => lp.pos.x,
            };
            self.textspan(p, span);
            p.y -= span.height;
        }
        self.job.obj_mut().emit_state = old;
    }

    fn emit_edge(&mut self, e: usize) {
        let m = self.job.m;
        let ed = &m.edges[e];
        let clip = self.job.clip;
        let in_box = splines_bb(&ed.bz).is_some_and(|b| overlap(b, clip))
            || ed.label.as_ref().is_some_and(|l| label_overlaps(l, clip))
            || ed
                .xlabel
                .as_ref()
                .is_some_and(|l| l.set && label_overlaps(l, clip));
        if !in_box || !self.edge_in_layer(e) {
            return;
        }
        let name = format!(
            "{}{}{}",
            m.nodes[ed.tail].name,
            if m.directed { "->" } else { "--" },
            m.nodes[ed.head].name
        );
        self.job.push_obj(ObjKind::Edge(e));
        self.comment(&name);
        let c = ed.attrs.get_or_empty("comment");
        self.comment(c);
        self.job.pop_obj();
        let styles = ed.attrs.nonempty("style").map(parse_style);
        if styles
            .as_ref()
            .is_some_and(|s| s.iter().any(|i| i.name == "invis"))
        {
            return;
        }
        let prev = self.set_scheme(ed.attrs.get("colorscheme"));
        self.emit_begin_edge(e, styles.as_deref());
        self.emit_edge_graphics(e, styles.as_deref());
        self.emit_end_edge(e);
        self.set_scheme(prev.as_deref());
    }

    fn emit_begin_edge(&mut self, e: usize, styles: Option<&[StyleItem]>) {
        let m = self.job.m;
        let ed = &m.edges[e];
        let a = ed.attrs;
        let flags = self.job.flags;
        self.job.push_obj(ObjKind::Edge(e));
        self.job.obj_mut().emit_state = EmitState::EDraw;
        if ed.label.as_ref().is_some_and(|l| !l.is_html) && a.mapbool("labelaligned") {
            self.job.obj_mut().labeledgealigned = true;
        }
        if let Some(s) = styles
            && !ed.bz.is_empty()
        {
            self.set_style(s);
        }
        if a.nonempty("penwidth").is_some() {
            let pw = a.late_double("penwidth", 1.0, 0.0);
            self.set_penwidth(pw);
        }
        let ctx = self.ctx(ObjKind::Edge(e));
        let layer_prefix = self.layer_page_prefix();
        let sub = |s: &str| strdup_and_subst_obj(s, &ctx);
        let o = self.job.obj_mut();
        if flags & GVRENDER_DOES_LABELS != 0 {
            o.label = ed.label.as_ref().map(|l| l.text.clone());
            o.taillabel = o.label.clone();
            o.headlabel = o.label.clone();
            if let Some(t) = &ed.tail_label {
                o.taillabel = Some(t.text.clone());
            }
            if let Some(h) = &ed.head_label {
                o.headlabel = Some(h.text.clone());
            }
        }
        let pick = |k1: &str, k2: &str| a.nonempty(k1).or_else(|| a.nonempty(k2));
        if flags & GVRENDER_DOES_MAPS != 0 {
            let id = {
                let prefix = layer_prefix.clone();
                if let Some(id) = a.nonempty("id") {
                    format!("{prefix}{id}")
                } else {
                    let mut s = prefix;
                    if let Some(gid) = m.root_attrs().nonempty("id") {
                        s.push_str(gid);
                        s.push('_');
                    }
                    s.push_str(&format!("edge{}", m.edge_seq(e)));
                    s
                }
            };
            o.id = Some(sub(&id));
            let dflt_url = pick("href", "URL").map(sub);
            o.url = pick("edgehref", "edgeURL").map(sub).or(dflt_url.clone());
            o.labelurl = pick("labelhref", "labelURL").map(sub).or(dflt_url.clone());
            match pick("tailhref", "tailURL") {
                Some(s) => {
                    o.tailurl = Some(sub(s));
                    o.explicit_tailurl = true;
                }
                None => o.tailurl = dflt_url.clone(),
            }
            match pick("headhref", "headURL") {
                Some(s) => {
                    o.headurl = Some(sub(s));
                    o.explicit_headurl = true;
                }
                None => o.headurl = dflt_url,
            }
        }
        if flags & GVRENDER_DOES_TARGETS != 0 {
            let dflt = a.nonempty("target").map(sub);
            o.target = a.nonempty("edgetarget").map(sub).or(dflt.clone());
            o.labeltarget = a.nonempty("labeltarget").map(sub).or(dflt.clone());
            o.tailtarget = a.nonempty("tailtarget").map(sub).or(dflt.clone());
            o.headtarget = a.nonempty("headtarget").map(sub).or(dflt);
        }
        if flags & GVRENDER_DOES_TOOLTIPS != 0 {
            let tt = |s: &str| {
                sub(&interpret_crnl(&convert_label_text(
                    s,
                    graphviz_pure_core::charset::current(),
                )))
            };
            if let Some(s) = pick("tooltip", "edgetooltip") {
                o.tooltip = Some(tt(s));
                o.explicit_tooltip = true;
            } else if let Some(l) = &o.label {
                o.tooltip = Some(l.clone());
            }
            if let Some(s) = a.nonempty("labeltooltip") {
                o.labeltooltip = Some(tt(s));
                o.explicit_labeltooltip = true;
            } else if let Some(l) = &o.label {
                o.labeltooltip = Some(l.clone());
            }
            if let Some(s) = a.nonempty("tailtooltip") {
                o.tailtooltip = Some(tt(s));
                o.explicit_tailtooltip = true;
            } else if let Some(l) = &o.taillabel {
                o.tailtooltip = Some(l.clone());
            }
            if let Some(s) = a.nonempty("headtooltip") {
                o.headtooltip = Some(tt(s));
                o.explicit_headtooltip = true;
            } else if let Some(l) = &o.headlabel {
                o.headtooltip = Some(l.clone());
            }
        }
        let has_map_target = {
            let o = self.job.obj();
            o.url.is_some() || o.tooltip.is_some()
        };
        if flags & (GVRENDER_DOES_MAPS | GVRENDER_DOES_TOOLTIPS) != 0
            && !ed.bz.is_empty()
            && has_map_target
            && flags & GVRENDER_DOES_MAP_POLYGON != 0
        {
            let w2 = (self.job.obj().penwidth / 2.0).max(2.0);
            let mut polys = Vec::with_capacity(ed.bz.len());
            for bz in &ed.bz {
                for mut poly in map_output_bspline(bz, w2) {
                    if flags & GVRENDER_DOES_TRANSFORM == 0 {
                        self.job.ptf_a(&mut poly);
                    }
                    polys.push(poly);
                }
            }
            let o = self.job.obj_mut();
            o.url_map_shape = MapShape::Polygon;
            o.url_map_p = polys.first().cloned().unwrap_or_default();
            o.url_bsplinemap_polys = polys;
        }
        self.eng.begin_edge(&self.job);
        let anchored = {
            let o = self.job.obj();
            o.url.is_some() || o.explicit_tooltip
        };
        if anchored {
            self.begin_obj_anchor();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_edge_label(
        &mut self,
        lbl: Option<&TextLabel>,
        lkind: EmitState,
        explicit: bool,
        url: Option<String>,
        tooltip: Option<String>,
        target: Option<String>,
        id: Option<String>,
        decorate: Option<&[Bz]>,
    ) {
        let Some(lbl) = lbl.filter(|l| l.set) else {
            return;
        };
        let newid = id.map(|id| {
            let ty = match lkind {
                EmitState::HLabel => "headlabel",
                EmitState::TLabel => "taillabel",
                _ => "label",
            };
            format!("{id}-{ty}")
        });
        let old = self.job.obj().emit_state;
        self.job.obj_mut().emit_state = lkind;
        let anchored = url.is_some() || explicit;
        if anchored && self.job.flags & EMIT_CLUSTERS_LAST == 0 {
            self.map_label(lbl);
            self.begin_anchor(
                url.as_deref(),
                tooltip.as_deref(),
                target.as_deref(),
                newid.as_deref(),
            );
        }
        self.emit_label(lkind, lbl);
        if let Some(spl) = decorate {
            self.emit_attachment(lbl, spl);
        }
        if anchored {
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.map_label(lbl);
                self.begin_anchor(
                    url.as_deref(),
                    tooltip.as_deref(),
                    target.as_deref(),
                    newid.as_deref(),
                );
            }
            self.end_anchor();
        }
        self.job.obj_mut().emit_state = old;
    }

    /// `emit_attachment`.
    fn emit_attachment(&mut self, lp: &TextLabel, spl: &[Bz]) {
        if lp.text.trim_start().is_empty() {
            return;
        }
        let sz = lp.dimen;
        let a0 = Point::new(lp.pos.x + sz.x / 2.0, lp.pos.y - sz.y / 2.0);
        let a1 = Point::new(a0.x - sz.x, a0.y);
        let a2 = super::geom::dotneato_closest(spl, lp.pos);
        self.set_style(&default_line_style());
        self.set_pencolor(&lp.fontcolor.clone());
        self.polyline(&[a0, a1, a2]);
    }

    fn emit_end_edge(&mut self, e: usize) {
        let m = self.job.m;
        let ed = &m.edges[e];
        let anchored = {
            let o = self.job.obj();
            o.url.is_some() || o.explicit_tooltip
        };
        if anchored {
            self.end_anchor();
            let polys = std::mem::take(&mut self.job.obj_mut().url_bsplinemap_polys);
            if polys.len() > 1 {
                for poly in polys.into_iter().skip(1) {
                    self.job.obj_mut().url_map_p = poly;
                    self.begin_obj_anchor();
                    self.end_anchor();
                }
            }
            self.job.obj_mut().url_map_p.clear();
        }
        let o = self.job.obj().clone();
        let decorate = (mapbool(Some(ed.attrs.late_string("decorate", "false")))
            && !ed.bz.is_empty())
        .then_some(ed.bz.as_slice());
        self.emit_edge_label(
            ed.label.as_ref(),
            EmitState::ELabel,
            o.explicit_labeltooltip,
            o.labelurl.clone(),
            o.labeltooltip.clone(),
            o.labeltarget.clone(),
            o.id.clone(),
            decorate,
        );
        self.emit_edge_label(
            ed.xlabel.as_ref(),
            EmitState::ELabel,
            o.explicit_labeltooltip,
            o.labelurl.clone(),
            o.labeltooltip.clone(),
            o.labeltarget.clone(),
            o.id.clone(),
            decorate,
        );
        self.emit_edge_label(
            ed.head_label.as_ref(),
            EmitState::HLabel,
            o.explicit_headtooltip,
            o.headurl.clone(),
            o.headtooltip.clone(),
            o.headtarget.clone(),
            o.id.clone(),
            None,
        );
        self.emit_edge_label(
            ed.tail_label.as_ref(),
            EmitState::TLabel,
            o.explicit_tailtooltip,
            o.tailurl.clone(),
            o.tailtooltip.clone(),
            o.tailtarget.clone(),
            o.id.clone(),
            None,
        );
        self.eng.end_edge(&self.job);
        self.job.pop_obj();
    }

    fn emit_edge_graphics(&mut self, e: usize, styles: Option<&[StyleItem]>) {
        let m = self.job.m;
        let ed = &m.edges[e];
        if ed.bz.is_empty() {
            return;
        }
        let a = ed.attrs;
        let penwidth = self.job.obj().penwidth;
        let arrowsize = a.late_double("arrowsize", 1.0, 0.0);
        let color = a.ptr_string("color", "");
        let tapered = styles.is_some_and(|s| s.iter().any(|i| i.name == "tapered"));
        let numc = color.text.matches(':').count();
        let numsemi = color.text.matches(';').count();
        let mut color = color;
        if numsemi > 0 && numc > 0 {
            if self.multicolor(e, styles, color.text, arrowsize, penwidth) {
                color = PtrStr::literal(DEFAULT_COLOR);
            } else {
                return;
            }
        }
        // C compares `fillcolor != color` by pointer (`emit.c:2418`, `:2433`).
        let fillcolor = a.ptr_nnstring("fillcolor", color);
        if !a.same_ptr(fillcolor, color) {
            self.set_fillcolor(fillcolor.text);
        }
        if tapered {
            // A set `fillcolor` is never empty; an unset one is `color`, and both become
            // the same `DEFAULT_COLOR` literal.
            let dflt = |c: PtrStr<'a>| {
                if c.text.is_empty() {
                    PtrStr::literal(DEFAULT_COLOR)
                } else {
                    c
                }
            };
            let (color, fillcolor) = (dflt(color), dflt(fillcolor));
            self.set_pencolor("transparent");
            self.set_fillcolor(color.text);
            let bz = &ed.bz[0];
            let dir = super::geom::taper_dir(a, m.directed);
            let stroke = super::geom::taper(&bz.list, dir, penwidth);
            self.polygon(&stroke, 1);
            self.set_pencolor(color.text);
            if !a.same_ptr(fillcolor, color) {
                self.set_fillcolor(fillcolor.text);
            }
            if bz.sflag != 0 {
                self.arrow_gen(
                    EmitState::TDraw,
                    bz.sp,
                    bz.list[0],
                    arrowsize,
                    penwidth,
                    bz.sflag,
                );
            }
            if bz.eflag != 0 {
                self.arrow_gen(
                    EmitState::HDraw,
                    bz.ep,
                    *bz.list.last().unwrap(),
                    arrowsize,
                    penwidth,
                    bz.eflag,
                );
            }
        } else if numc > 0 {
            self.parallel_colors(e, color.text, numc, arrowsize, penwidth);
        } else {
            let (color, fillcolor) = (color.text, fillcolor.text);
            if !color.is_empty() {
                self.set_pencolor(color);
                self.set_fillcolor(fillcolor);
            } else {
                self.set_pencolor(DEFAULT_COLOR);
                if !fillcolor.is_empty() {
                    self.set_fillcolor(fillcolor);
                } else {
                    self.set_fillcolor(DEFAULT_COLOR);
                }
            }
            let n = ed.bz.len();
            let is_ortho = m.root_attrs().get("splines") == Some("ortho");
            let mut want_rounded = styles.is_some_and(|s| s.iter().any(|i| i.name == "rounded"));
            let mut radius = 0.0;
            if let Some(r) = a.nonempty("radius") {
                radius = graphviz_pure_core::strtod::atof(r);
                want_rounded = radius > 0.0;
            }
            if want_rounded && radius == 0.0 {
                radius = (penwidth * 8.0).max(12.0);
            }
            for bz in &ed.bz {
                if is_ortho && want_rounded && radius > 0.0 {
                    self.ortho_rounded(&bz.list, radius, color);
                } else {
                    self.bezier(&bz.list, 0);
                }
                if bz.sflag != 0 {
                    self.arrow_gen(
                        EmitState::TDraw,
                        bz.sp,
                        bz.list[0],
                        arrowsize,
                        penwidth,
                        bz.sflag,
                    );
                }
                if bz.eflag != 0 {
                    self.arrow_gen(
                        EmitState::HDraw,
                        bz.ep,
                        *bz.list.last().unwrap(),
                        arrowsize,
                        penwidth,
                        bz.eflag,
                    );
                }
                if n > 1
                    && (bz.sflag != 0 || bz.eflag != 0)
                    && let Some(s) = styles
                {
                    self.set_style(s);
                }
            }
        }
    }

    /// The multi-color (`a:b:c`) branch of `emit_edge_graphics`: parallel offset Béziers.
    /// `numc` is the `:` count of the edge's `color`, which is no longer `color` when a
    /// `multicolor` failure has replaced it with `black`.
    fn parallel_colors(
        &mut self,
        e: usize,
        color: &str,
        numc: usize,
        arrowsize: f64,
        penwidth: f64,
    ) {
        const SEP: f64 = 2.0;
        let m = self.job.m;
        let ed = &m.edges[e];
        let numc2 = (2.0 + numc as f64) / 2.0;
        let mut offs: Vec<Vec<Point>> = Vec::new();
        let mut tmps: Vec<Vec<Point>> = Vec::new();
        for bz in &ed.bz {
            let n = bz.list.len();
            let mut off = vec![Point::default(); n];
            let mut tmp = vec![Point::default(); n];
            let mut pf3 = bz.list[0];
            let mut pf2 = Point::default();
            let mut j = 0;
            while j + 1 < n {
                let pf0 = pf3;
                let pf1 = bz.list[j + 1];
                off[j] = if j == 0 {
                    super::geom::computeoffset_p(pf0, pf1, SEP)
                } else {
                    super::geom::computeoffset_p(pf2, pf1, SEP)
                };
                pf2 = bz.list[j + 2];
                pf3 = bz.list[j + 3];
                let q = super::geom::computeoffset_qr(pf0, pf1, pf2, pf3, SEP);
                off[j + 1] = q;
                off[j + 2] = q;
                // C (clang) fuses each `pf.x - numc2 * offlist[j].x`.
                let outer = |pf: Point, o: Point| {
                    Point::new((-numc2).mul_add(o.x, pf.x), (-numc2).mul_add(o.y, pf.y))
                };
                tmp[j] = outer(pf0, off[j]);
                tmp[j + 1] = outer(pf1, off[j + 1]);
                tmp[j + 2] = outer(pf2, off[j + 2]);
                j += 3;
            }
            off[j] = super::geom::computeoffset_p(pf2, pf3, SEP);
            tmp[j] = Point::new(
                (-numc2).mul_add(off[j].x, pf3.x),
                (-numc2).mul_add(off[j].y, pf3.y),
            );
            offs.push(off);
            tmps.push(tmp);
        }
        // C compares these colours by pointer. Each `strtok` token of the `gv_strdup` copy
        // is a new pointer, so every token sets the pen and fill (even a repeated name, or
        // the `black` of a failed `multicolor`). After the loop `color` is the NULL that
        // ended `strtok`, so the tail arrow always sets its colour, and the head arrow does
        // unless the tail arrow just set the same pointer: one token or none. With no token
        // `headcolor` and `tailcolor` are still the whole string.
        let toks: Vec<&str> = color.split(':').filter(|c| !c.is_empty()).collect();
        let head = toks.first().copied().unwrap_or(color);
        let tail = toks.get(1).or(toks.first()).copied().unwrap_or(color);
        for c in &toks {
            self.set_pencolor(c);
            self.set_fillcolor(c);
            for (i, tmp) in tmps.iter_mut().enumerate() {
                for (t, o) in tmp.iter_mut().zip(&offs[i]) {
                    t.x += o.x;
                    t.y += o.y;
                }
                let pts = tmp.clone();
                self.bezier(&pts, 0);
            }
        }
        let bz = ed.bz.last().unwrap();
        if bz.sflag != 0 {
            self.set_pencolor(tail);
            self.set_fillcolor(tail);
            self.arrow_gen(
                EmitState::TDraw,
                bz.sp,
                bz.list[0],
                arrowsize,
                penwidth,
                bz.sflag,
            );
        }
        if bz.eflag != 0 {
            if !(bz.sflag != 0 && toks.len() < 2) {
                self.set_pencolor(head);
                self.set_fillcolor(head);
            }
            self.arrow_gen(
                EmitState::HDraw,
                bz.ep,
                *bz.list.last().unwrap(),
                arrowsize,
                penwidth,
                bz.eflag,
            );
        }
    }

    /// `multicolor`: an edge drawn as consecutive colored pieces. Returns `true` on error.
    fn multicolor(
        &mut self,
        e: usize,
        styles: Option<&[StyleItem]>,
        colors: &str,
        arrowsize: f64,
        penwidth: f64,
    ) -> bool {
        let m = self.job.m;
        let ed = &m.edges[e];
        let in_edge = || {
            let arrow = if m.directed { " -> " } else { " -- " };
            // emit.c:1988: one agerr of both names, which share agnameof's static buffer
            // when anonymous.
            let [t, h] = graphviz_pure_core::agnameof_buffer([
                m.node_agnameof(ed.tail),
                m.node_agnameof(ed.head),
            ]);
            agerr_prev(&format!("in edge {t}{arrow}{h}\n"));
        };
        let segs = match parse_segs(colors) {
            Ok((s, rv)) => {
                if rv > 1 {
                    in_edge();
                }
                s
            }
            Err(rv) => {
                if rv > 1 {
                    in_edge();
                }
                return true;
            }
        };
        let n = ed.bz.len();
        for bz in &ed.bz {
            let mut left = 1.0;
            let mut first = true;
            let mut endcolor: Option<String> = None;
            let mut right: Vec<Point> = Vec::new();
            for s in &segs {
                let Some(c) = &s.color else { break };
                if aeq0(s.t) {
                    continue;
                }
                self.set_pencolor(c);
                left -= s.t;
                endcolor = Some(c.clone());
                if first {
                    first = false;
                    let (l, r) = super::geom::split_bspline(&bz.list, s.t);
                    self.bezier(&l, 0);
                    right = r;
                    if aeq0(left) {
                        break;
                    }
                } else if aeq0(left) {
                    let r = right.clone();
                    self.bezier(&r, 0);
                    break;
                } else {
                    let (l, r) = super::geom::split_bspline(&right, s.t / (left + s.t));
                    self.bezier(&l, 0);
                    right = r;
                }
            }
            // emit.c:2036-2045 sets the arrows' colors with `LIST_FRONT(&segs)->color`, NULL
            // where the first segment is empty (`":red;0.3"`), and `endcolor`, NULL too where
            // no colored segment with a weight above 0 comes before the first empty one
            // (`"red;0.000001:"`); `gvrender_set_pencolor` dereferences the NULL and C crashes
            // (#385). The port draws no arrow without a color: C's drawing of the same edge
            // with `dir=none`, the input C handles nearest (crashes.md, "Empty colour
            // segments", says why not #283's black fallback).
            if bz.sflag != 0
                && let Some(c) = segs.first().and_then(|s| s.color.clone())
            {
                self.set_pencolor(&c);
                self.set_fillcolor(&c);
                self.arrow_gen(
                    EmitState::TDraw,
                    bz.sp,
                    bz.list[0],
                    arrowsize,
                    penwidth,
                    bz.sflag,
                );
            }
            if bz.eflag != 0
                && let Some(c) = &endcolor
            {
                self.set_pencolor(c);
                self.set_fillcolor(c);
                self.arrow_gen(
                    EmitState::HDraw,
                    bz.ep,
                    *bz.list.last().unwrap(),
                    arrowsize,
                    penwidth,
                    bz.eflag,
                );
            }
            if n > 1
                && (bz.sflag != 0 || bz.eflag != 0)
                && let Some(s) = styles
            {
                self.set_style(s);
            }
        }
        false
    }
}

/// An orthogonal corner of an edge (`corner_info_t`).
struct Corner {
    idx: usize,
    trunc_prev: Point,
    trunc_next: Point,
    center: Point,
    angle1: f64,
    angle2: f64,
}

fn find_ortho_corners(pts: &[Point], radius: f64) -> Vec<Corner> {
    use std::f64::consts::PI;
    const TOL: f64 = 0.1;
    const DT: f64 = 0.01;
    let n = pts.len();
    let mut corners: Vec<Corner> = Vec::new();
    for i in 0..n {
        let prev = (0..i)
            .rev()
            .find(|&j| math::hypot(pts[j].x - pts[i].x, pts[j].y - pts[i].y) > DT);
        let next = (i + 1..n).find(|&j| math::hypot(pts[j].x - pts[i].x, pts[j].y - pts[i].y) > DT);
        let (Some(pi), Some(ni)) = (prev, next) else {
            continue;
        };
        let (prev, curr, next) = (pts[pi], pts[i], pts[ni]);
        let (mut dx1, mut dy1) = (curr.x - prev.x, curr.y - prev.y);
        let (mut dx2, mut dy2) = (next.x - curr.x, next.y - curr.y);
        let s1h = dy1.abs() < TOL && dx1.abs() > TOL;
        let s1v = dx1.abs() < TOL && dy1.abs() > TOL;
        let s2h = dy2.abs() < TOL && dx2.abs() > TOL;
        let s2v = dx2.abs() < TOL && dy2.abs() > TOL;
        if !((s1h && s2v) || (s1v && s2h)) {
            continue;
        }
        if corners
            .iter()
            .any(|c| math::hypot(curr.x - pts[c.idx].x, curr.y - pts[c.idx].y) < DT)
        {
            continue;
        }
        let l1 = math::hypot(dx1, dy1);
        let l2 = math::hypot(dx2, dy2);
        dx1 /= l1;
        dy1 /= l1;
        dx2 /= l2;
        dy2 /= l2;
        let (cx, cy, a1, a2) = if s1h && s2v {
            if dx1 > 0.0 && dy2 < 0.0 {
                (curr.x - radius, curr.y - radius, 0.0, PI / 2.0)
            } else if dx1 > 0.0 && dy2 > 0.0 {
                (curr.x - radius, curr.y + radius, -PI / 2.0, 0.0)
            } else if dx1 < 0.0 && dy2 < 0.0 {
                (curr.x + radius, curr.y - radius, PI / 2.0, PI)
            } else {
                (curr.x + radius, curr.y + radius, PI, 3.0 * PI / 2.0)
            }
        } else if dy1 < 0.0 && dx2 > 0.0 {
            (curr.x + radius, curr.y + radius, PI, 3.0 * PI / 2.0)
        } else if dy1 < 0.0 && dx2 < 0.0 {
            (curr.x - radius, curr.y + radius, 3.0 * PI / 2.0, 2.0 * PI)
        } else if dy1 > 0.0 && dx2 > 0.0 {
            (curr.x + radius, curr.y - radius, PI / 2.0, PI)
        } else {
            (curr.x - radius, curr.y - radius, 0.0, PI / 2.0)
        };
        corners.push(Corner {
            idx: i,
            // C (clang) fuses `curr ± d * radius`.
            trunc_prev: Point::new(
                (-dx1).mul_add(radius, curr.x),
                (-dy1).mul_add(radius, curr.y),
            ),
            trunc_next: Point::new(dx2.mul_add(radius, curr.x), dy2.mul_add(radius, curr.y)),
            center: Point::new(cx, cy),
            angle1: a1,
            angle2: a2,
        });
    }
    corners.sort_by_key(|c| c.idx);
    corners
}

impl<E: Engine> Emitter<'_, E> {
    /// The rounded-corner rendering of an orthogonal edge in `emit_edge_graphics`.
    fn ortho_rounded(&mut self, pts: &[Point], radius: f64, color: &str) {
        let corners = find_ortho_corners(pts, radius);
        if corners.is_empty() {
            self.bezier(pts, 0);
            return;
        }
        let at_corner = |p: Point| {
            corners
                .iter()
                .any(|c| math::hypot(p.x - pts[c.idx].x, p.y - pts[c.idx].y) < 0.01)
        };
        let mut start_idx = 0;
        let mut start_pt = pts[0];
        for c in 0..=corners.len() {
            let (end_idx, end_pt) = if c < corners.len() {
                (corners[c].idx, corners[c].trunc_prev)
            } else {
                (pts.len() - 1, pts[pts.len() - 1])
            };
            let mut seg = vec![start_pt];
            for p in pts.iter().take(end_idx).skip(start_idx + 1) {
                if !at_corner(*p) {
                    seg.push(*p);
                }
            }
            seg.push(end_pt);
            self.polyline(&seg);
            if c < corners.len() {
                let mut next = corners[c].idx + 1;
                while next < pts.len() && at_corner(pts[next]) {
                    next += 1;
                }
                start_idx = next;
                start_pt = corners[c].trunc_next;
            }
        }
        for c in &corners {
            let w = super::geom::elliptic_wedge(c.center, radius, radius, c.angle1, c.angle2);
            if w.len() > 4 {
                let (s, e) = (3, w.len() - 4);
                if e >= s && e - s + 1 >= 2 {
                    self.set_pencolor(if color.is_empty() {
                        DEFAULT_COLOR
                    } else {
                        color
                    });
                    self.polyline(&w[s..=e]);
                }
            }
        }
    }
}

/// The tokens `strtok` gives for `s` and `delims`, both C's bytes: a multibyte character
/// of `delims` is several delimiters, and a token can end inside a character.
fn strtok<'s>(s: &'s [u8], delims: &'s [u8]) -> impl Iterator<Item = &'s [u8]> {
    s.split(|b| delims.contains(b)).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::job::FILLCOLOR_CALLS;
    use graphviz_pure_core::{BezierSegment, BoundingBox, Point, Spline, arrows};

    /// #385: `tok` yields an empty token at a leading or trailing `:`, none inside a run.
    #[test]
    fn tok_colon_yields_c_tokens() {
        let toks = |s| super::tok_colon(s).collect::<Vec<_>>();
        assert_eq!(toks("red:blue"), ["red", "blue"]);
        assert_eq!(toks(":red;0.3"), ["", "red;0.3"]);
        assert_eq!(toks("red;0.3:"), ["red;0.3", ""]);
        assert_eq!(toks("red::blue"), ["red", "blue"]);
        assert_eq!(toks("::red:::"), ["", "red", ""]);
        assert_eq!(toks(":"), ["", ""]);
        assert_eq!(toks(""), [""]);
    }

    /// #385: an empty segment is kept with no color and takes the unassigned weight, as in
    /// C's `parseSegs`.
    #[test]
    fn parse_segs_keeps_empty_segments() {
        let segs = |s| {
            let (v, rv) = super::parse_segs(s).unwrap();
            assert_eq!(rv, 0, "{s}");
            v.into_iter()
                .map(|g| (g.color, (g.t * 100.0).round()))
                .collect::<Vec<_>>()
        };
        assert_eq!(segs("red;0.3:"), [(Some("red".into()), 30.0), (None, 70.0)]);
        assert_eq!(segs(":red;0.3"), [(None, 70.0), (Some("red".into()), 30.0)]);
        assert_eq!(segs(":red"), [(None, 50.0), (Some("red".into()), 50.0)]);
        assert_eq!(segs(""), [(None, 100.0)]);
    }

    /// #385: a gradient's empty second segment is no second color, as in C's `findStopColor`.
    #[test]
    fn find_stop_color_with_an_empty_second_segment() {
        assert_eq!(
            super::find_stop_color("red:"),
            Some(("red".to_string(), None, 0.0))
        );
        assert_eq!(
            super::find_stop_color("red;0.3:"),
            Some(("red".to_string(), None, 0.3))
        );
        assert_eq!(super::find_stop_color(":red"), None);
    }

    /// The `set_fillcolor` calls of rendering `a -> b [attrs]` with invisible nodes.
    fn edge_fillcolor_calls(attrs: &str) -> Vec<String> {
        let src = format!("digraph {{ node [style=invis]; a -> b [{attrs}] }}");
        let mut g = graphviz_pure_parser::parse_dot(&src).expect("parse");
        g.bb = BoundingBox::new(0.0, 0.0, 54.0, 108.0);
        g.nodes[0].pos = Point::new(27.0, 90.0);
        g.nodes[1].pos = Point::new(27.0, 18.0);
        g.edges[0].spline = Spline {
            segments: vec![BezierSegment {
                points: vec![
                    Point::new(27.0, 72.0),
                    Point::new(27.0, 64.0),
                    Point::new(27.0, 56.0),
                    Point::new(27.0, 48.0),
                ],
                start_arrow: None,
                end_arrow: Some(Point::new(27.0, 36.0)),
                sflag: 0,
                eflag: arrows::ARR_TYPE_NORM,
            }],
        };
        FILLCOLOR_CALLS.with_borrow_mut(Vec::clear);
        crate::render_graph(&g, crate::OutputFormat::Svg).expect("render");
        FILLCOLOR_CALLS.take()
    }

    // Issue #389: C's `fillcolor != color` compares pointers (`emit.c:2418`, `:2433`). An
    // equal text is one interned pointer unless the HTML flags differ; the `DEFAULT_COLOR`
    // literal (a failed `multicolor`, an empty tapered `color`) is never a set `fillcolor`'s.
    // Each sequence is C's: the early call, then the branch's own (tapered: its fill and
    // the second test; parallel: each token and the head arrow; single colour: one).
    #[test]
    fn edge_fillcolor_is_set_when_c_pointers_differ() {
        for (attrs, calls) in [
            ("color=red fillcolor=red", &["red"][..]),
            ("color=<red> fillcolor=red", &["red", "red"]),
            ("color=red fillcolor=<red>", &["red", "red"]),
            ("color=red fillcolor=blue", &["blue", "blue"]),
            ("color=red", &["red"]),
            ("style=tapered color=red fillcolor=red", &["red"]),
            (
                "style=tapered color=\"\" fillcolor=black",
                &["black", "black", "black"],
            ),
            ("style=tapered color=\"\"", &["black"]),
            (
                "color=\"red;x:blue\" fillcolor=black",
                &["black", "black", "black"],
            ),
            ("color=\"red;x:blue\"", &["black", "black"]),
        ] {
            // The graph's background and page come first.
            let want: Vec<&str> = ["lightgrey", "white"]
                .iter()
                .chain(calls)
                .copied()
                .collect();
            assert_eq!(edge_fillcolor_calls(attrs), want, "{attrs}");
        }
    }
}
