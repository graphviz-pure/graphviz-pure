//! Edge port resolution: `chkPort`, `poly_port`, `record_port`,
//! `compassPort`, `compassPoint`, `closestSide` and `resolvePort(s)` from
//! `lib/common/shapes.c` and `lib/common/utils.c`, shared by all engines
//! through [`PortGraph`].

use super::shapes::InsideFn;
use super::splines::{SplineGraph, bezier_clip};
use super::types::{LGraph, PortInfo};
use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, NodeData, Point};
use std::f64::consts::PI;

pub const BOTTOM: u8 = 1 << 0;
pub const RIGHT: u8 = 1 << 1;
pub const TOP: u8 = 1 << 2;
pub const LEFT: u8 = 1 << 3;

const MC_SCALE: i32 = 256;
const RANKDIR_LR: i32 = 1;
const RANKDIR_BT: i32 = 2;
const RANKDIR_RL: i32 = 3;

/// What port resolution needs beyond [`SplineGraph`]: the node's input
/// record, from which the shape geometry (record fields, HTML cells) is
/// derived.
pub trait PortGraph: SplineGraph {
    /// Input data of real node `n`.
    fn port_node(&self, n: usize) -> &NodeData;
    /// Graph-level parameters of node sizing.
    fn port_env(&self) -> graphviz_pure_geom::NodeSizeEnv<'_>;
}

/// `cwrotatepf(p, cwrot)` (geom.c).
pub fn cwrotatepf(p: Point, cwrot: i32) -> Point {
    match cwrot {
        90 => Point::new(p.y, -p.x),
        180 => Point::new(p.x, -p.y),
        270 => Point::new(p.y, p.x),
        _ => p,
    }
}

/// `ccwrotatepf(p, ccwrot)` (geom.c).
pub fn ccwrotatepf(p: Point, ccwrot: i32) -> Point {
    match ccwrot {
        90 => Point::new(-p.y, p.x),
        180 => Point::new(p.x, -p.y),
        270 => Point::new(p.y, p.x),
        _ => p,
    }
}

/// `invflip_side(side, rankdir)`.
fn invflip_side(side: u8, rankdir: i32) -> u8 {
    match rankdir {
        RANKDIR_BT => match side {
            TOP => BOTTOM,
            BOTTOM => TOP,
            s => s,
        },
        RANKDIR_LR => match side {
            TOP => RIGHT,
            BOTTOM => LEFT,
            LEFT => TOP,
            RIGHT => BOTTOM,
            s => s,
        },
        RANKDIR_RL => match side {
            TOP => RIGHT,
            BOTTOM => LEFT,
            LEFT => BOTTOM,
            RIGHT => TOP,
            s => s,
        },
        _ => side,
    }
}

/// `invflip_angle(angle, rankdir)`.
fn invflip_angle(angle: f64, rankdir: i32) -> f64 {
    match rankdir {
        RANKDIR_BT => -angle,
        RANKDIR_LR => angle - PI * 0.5,
        RANKDIR_RL => {
            if angle == PI {
                -0.5 * PI
            } else if angle == PI * 0.75 {
                -0.25 * PI
            } else if angle == PI * 0.5 {
                0.0
            } else if angle == 0.0 {
                PI * 0.5
            } else if angle == PI * -0.25 {
                PI * 0.75
            } else if angle == PI * -0.5 {
                PI
            } else {
                angle
            }
        }
        _ => angle,
    }
}

/// `compassPoint(ictxt, y, x)`: where the ray from the centre towards
/// `(x, y)` leaves the node shape.
fn compass_point<S: PortGraph + ?Sized>(g: &S, n: usize, y: f64, x: f64) -> Point {
    let rd = g.rankdir();
    let mut p = Point::new(x, y);
    if rd != 0 {
        p = cwrotatepf(p, 90 * rd);
    }
    let mut curve = [Point::new(0.0, 0.0), Point::new(0.0, 0.0), p, p];
    let mut ctx = super::splines::inside_ctx(g, n, None);
    if let Some(shape) = g.shape(n) {
        bezier_clip(&mut |q| shape.inside(&mut ctx, q), &mut curve, true);
    }
    if rd != 0 {
        curve[0] = ccwrotatepf(curve[0], 90 * rd);
    }
    curve[0]
}

/// `compassPort(n, bp, pp, compass, sides, ictxt)`: fills `pp`; returns
/// whether the compass point was unrecognised.
pub fn compass_port<S: PortGraph + ?Sized>(
    g: &S,
    n: usize,
    bp: Option<BoundingBox>,
    pp: &mut PortInfo,
    compass: &str,
    sides: u8,
    ictxt: bool,
) -> bool {
    let mut b;
    let mut p;
    let mut rv = false;
    let mut theta = 0.0;
    let mut constrain = false;
    let mut dyna = false;
    let mut side = 0u8;
    let mut clip = true;
    let mut defined;
    if let Some(bb) = bp {
        b = bb;
        p = Point::new((b.ll.x + b.ur.x) / 2.0, (b.ll.y + b.ur.y) / 2.0);
        defined = true;
    } else {
        p = Point::new(0.0, 0.0);
        b = BoundingBox::default();
        let (lw, ht) = (g.lw(n), g.ht(n));
        if g.flip() {
            b.ur.x = ht / 2.0;
            b.ll.x = -b.ur.x;
            b.ur.y = lw;
            b.ll.y = -b.ur.y;
        } else {
            b.ur.y = ht / 2.0;
            b.ll.y = -b.ur.y;
            b.ur.x = lw;
            b.ll.x = -b.ur.x;
        }
        defined = false;
    }
    let maxv = b.ur.x.max(b.ur.y) * 4.0;
    let ctr = p;
    let cb = compass.as_bytes();
    if !cb.is_empty() {
        let rest = &cb[1..];
        match cb[0] {
            b'e' => {
                if !rest.is_empty() {
                    rv = true;
                } else {
                    if ictxt {
                        p = compass_point(g, n, ctr.y, maxv);
                    } else {
                        p.x = b.ur.x;
                    }
                    theta = 0.0;
                    constrain = true;
                    defined = true;
                    clip = false;
                    side = sides & RIGHT;
                }
            }
            b's' => {
                p.y = b.ll.y;
                constrain = true;
                clip = false;
                match rest.first() {
                    None => {
                        theta = -PI * 0.5;
                        defined = true;
                        if ictxt {
                            p = compass_point(g, n, -maxv, ctr.x);
                        } else {
                            p.x = ctr.x;
                        }
                        side = sides & BOTTOM;
                    }
                    Some(b'e') => {
                        theta = -PI * 0.25;
                        defined = true;
                        if ictxt {
                            p = compass_point(g, n, -maxv, maxv);
                        } else {
                            p.x = b.ur.x;
                        }
                        side = sides & (BOTTOM | RIGHT);
                    }
                    Some(b'w') => {
                        theta = -PI * 0.75;
                        defined = true;
                        if ictxt {
                            p = compass_point(g, n, -maxv, -maxv);
                        } else {
                            p.x = b.ll.x;
                        }
                        side = sides & (BOTTOM | LEFT);
                    }
                    _ => {
                        p.y = ctr.y;
                        constrain = false;
                        clip = true;
                        rv = true;
                    }
                }
            }
            b'w' => {
                if !rest.is_empty() {
                    rv = true;
                } else {
                    if ictxt {
                        p = compass_point(g, n, ctr.y, -maxv);
                    } else {
                        p.x = b.ll.x;
                    }
                    theta = PI;
                    constrain = true;
                    defined = true;
                    clip = false;
                    side = sides & LEFT;
                }
            }
            b'n' => {
                p.y = b.ur.y;
                constrain = true;
                clip = false;
                match rest.first() {
                    None => {
                        defined = true;
                        theta = PI * 0.5;
                        if ictxt {
                            p = compass_point(g, n, maxv, ctr.x);
                        } else {
                            p.x = ctr.x;
                        }
                        side = sides & TOP;
                    }
                    Some(b'e') => {
                        defined = true;
                        theta = PI * 0.25;
                        if ictxt {
                            p = compass_point(g, n, maxv, maxv);
                        } else {
                            p.x = b.ur.x;
                        }
                        side = sides & (TOP | RIGHT);
                    }
                    Some(b'w') => {
                        defined = true;
                        theta = PI * 0.75;
                        if ictxt {
                            p = compass_point(g, n, maxv, -maxv);
                        } else {
                            p.x = b.ll.x;
                        }
                        side = sides & (TOP | LEFT);
                    }
                    _ => {
                        p.y = ctr.y;
                        constrain = false;
                        clip = true;
                        rv = true;
                    }
                }
            }
            b'_' => {
                dyna = true;
                side = sides;
            }
            b'c' => {}
            _ => rv = true,
        }
    }
    let rankdir = g.rankdir();
    p = cwrotatepf(p, 90 * rankdir);
    pp.side = if dyna {
        side
    } else {
        invflip_side(side, rankdir)
    };
    pp.bp = bp;
    pp.p = p;
    pp.theta = invflip_angle(theta, rankdir);
    if p.x == 0.0 && p.y == 0.0 {
        pp.order = (MC_SCALE / 2) as u8;
    } else {
        let mut angle = math::atan2(p.y, p.x) + 1.5 * PI;
        if angle >= 2.0 * PI {
            angle -= 2.0 * PI;
        }
        pp.order = ((MC_SCALE as f64 * angle) / (2.0 * PI)) as i32 as u8;
    }
    pp.constrained = constrain;
    pp.defined = defined;
    pp.clip = clip;
    pp.dyna = dyna;
    rv
}

/// `Center` in shapes.c: the port of an edge without a port name.
pub fn port_center() -> PortInfo {
    PortInfo {
        theta: -1.0,
        clip: true,
        ..PortInfo::default()
    }
}

/// `portToTbl` / `portToCell` (`htmltable.c`): the box and sides of the
/// table or cell whose `PORT` matches (case-insensitively).
fn port_to_tbl(
    tbl: &graphviz_pure_geom::html_label::HtmlTbl,
    id: &str,
) -> Option<(BoundingBox, u8)> {
    if tbl
        .data
        .port
        .as_deref()
        .is_some_and(|p| p.eq_ignore_ascii_case(id))
    {
        return Some((tbl.data.bx, tbl.data.sides));
    }
    for cell in &tbl.cells {
        if cell
            .data
            .port
            .as_deref()
            .is_some_and(|p| p.eq_ignore_ascii_case(id))
        {
            return Some((cell.data.bx, cell.data.sides));
        }
        if let graphviz_pure_geom::html_label::HtmlContent::Table(t) = &cell.child
            && let Some(r) = port_to_tbl(t, id)
        {
            return Some(r);
        }
    }
    None
}

/// `ND_shape(n)->fns->portfn(n, portname, compass)`: `record_port` for
/// records, `poly_port` otherwise.
pub fn shape_port<S: PortGraph + ?Sized>(
    g: &S,
    n: usize,
    portname: &str,
    compass: Option<&str>,
) -> graphviz_pure_core::Result<PortInfo> {
    if portname.is_empty() {
        return Ok(port_center());
    }
    let compass = compass.unwrap_or("_");
    let all = BOTTOM | RIGHT | TOP | LEFT;
    let src = g.port_node(n);
    // The node was sized, and its diagnostics reported, by common_init_node.
    let geom = graphviz_pure_core::diag::quietly(|| {
        graphviz_pure_geom::node_geometry(src, &g.port_env())
    })?;
    let mut rv = PortInfo::default();
    let is_record = g.shape(n).is_some_and(|s| s.inside == InsideFn::Record);
    if is_record && let Some(rec) = geom.record.as_ref() {
        match rec.find_port(portname) {
            Some(f) => {
                let (fb, sides) = (f.bbox, f.sides);
                if compass_port(g, n, Some(fb), &mut rv, compass, sides, false) {
                    bad_compass(src, portname, compass);
                }
            }
            None => {
                if compass_port(g, n, Some(rec.bbox), &mut rv, portname, all, false) {
                    unrecognized(src, portname);
                }
            }
        }
        return Ok(rv);
    }
    if let Some(graphviz_pure_geom::html_label::HtmlLabel::Table(tbl)) =
        geom.label.html_label.as_deref()
        && let Some((bp, sides)) = port_to_tbl(tbl, portname)
    {
        if compass_port(g, n, Some(bp), &mut rv, compass, sides, false) {
            bad_compass(src, portname, compass);
        }
        rv.name = None;
        return Ok(rv);
    }
    // IS_BOX: the shape's polygon is p_box, as for box, rect, rectangle, unknown names
    // (bind_shape copies Shapes[0]) and shapefile user shapes.
    let shape_name = src
        .attrs
        .get("shape")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("ellipse");
    let custom = graphviz_pure_core::usershape::binds_custom(
        shape_name,
        src.attrs.get("shapefile").map(String::as_str),
    );
    let is_box = custom
        || graphviz_pure_geom::ShapeKind::parse(shape_name) == graphviz_pure_geom::ShapeKind::Box;
    if compass_port(g, n, None, &mut rv, portname, all, !is_box) {
        unrecognized(src, portname);
    }
    rv.name = None;
    Ok(rv)
}

/// `unrecognized(n, p)` (`lib/common/shapes.c:362-365`).
fn unrecognized(n: &NodeData, p: &str) {
    if !graphviz_pure_core::diag::reporting_ports() {
        return;
    }
    graphviz_pure_core::diag::agwarningf(&format!("node {}, port {p} unrecognized\n", n.name));
}

/// The compass warning of `poly_port` and `record_port` (`shapes.c:2894-2896`,
/// `3748-3749`).
fn bad_compass(n: &NodeData, portname: &str, compass: &str) {
    if !graphviz_pure_core::diag::reporting_ports() {
        return;
    }
    graphviz_pure_core::diag::agwarningf(&format!(
        "node {}, port {portname}, unrecognized compass point '{compass}' - ignored\n",
        n.name
    ));
}

/// `chkPort(portfn, n, s)` for a parsed port (`name[:compass]`).
pub fn chk_port<S: PortGraph + ?Sized>(
    g: &S,
    n: usize,
    p: &graphviz_pure_core::Port,
) -> graphviz_pure_core::Result<PortInfo> {
    let name = p.name.clone().unwrap_or_default();
    let mut pt = shape_port(g, n, &name, p.compass.as_deref())?;
    pt.name = if name.is_empty() && p.compass.is_none() {
        None
    } else if let Some(c) = &p.compass {
        Some(c.clone())
    } else {
        Some(name)
    };
    Ok(pt)
}

/// `closestSide(n, other, oldport)`: compass of the side of the port box
/// nearest to `other`, or `""` for the centre.
fn closest_side<S: PortGraph + ?Sized>(
    g: &S,
    n: usize,
    other: usize,
    old: &PortInfo,
) -> &'static str {
    const SIDE_PORT: [&str; 4] = ["s", "e", "n", "w"];
    let rkd = g.rankdir();
    let cvt = |p: Point| match rkd {
        RANKDIR_BT => Point::new(p.x, -p.y),
        RANKDIR_LR => Point::new(-p.y, p.x),
        RANKDIR_RL => Point::new(p.y, p.x),
        _ => p,
    };
    let pt = cvt(g.coord(n));
    let opt = cvt(g.coord(other));
    let sides = old.side;
    if sides == 0 || sides == (TOP | BOTTOM | LEFT | RIGHT) {
        return "";
    }
    let b = old.bp.unwrap_or_else(|| {
        let (lw, ht) = (g.lw(n), g.ht(n));
        if g.flip() {
            BoundingBox::new(-ht / 2.0, -lw, ht / 2.0, lw)
        } else {
            BoundingBox::new(-lw, -ht / 2.0, lw, ht / 2.0)
        }
    });
    let mut rv: Option<&'static str> = None;
    let mut mind = 0.0;
    for (i, name) in SIDE_PORT.iter().enumerate() {
        if sides & (1 << i) == 0 {
            continue;
        }
        let p = match i {
            0 => Point::new((b.ll.x + b.ur.x) / 2.0, b.ll.y),
            1 => Point::new(b.ur.x, (b.ll.y + b.ur.y) / 2.0),
            2 => Point::new((b.ll.x + b.ur.x) / 2.0, b.ur.y),
            _ => Point::new(b.ll.x, (b.ll.y + b.ur.y) / 2.0),
        };
        let p = Point::new(p.x + pt.x, p.y + pt.y);
        let d = super::geom::dist2(p, opt);
        if rv.is_none() || d < mind {
            mind = d;
            rv = Some(name);
        }
    }
    rv.unwrap_or("")
}

/// `resolvePort(n, other, oldport)`.
pub fn resolve_port<S: PortGraph + ?Sized>(
    g: &S,
    n: usize,
    other: usize,
    old: &PortInfo,
) -> PortInfo {
    let compass = closest_side(g, n, other, old);
    let mut rv = PortInfo {
        name: old.name.clone(),
        ..PortInfo::default()
    };
    compass_port(g, n, old.bp, &mut rv, compass, old.side, false);
    rv
}

/// `resolvePorts(e)`: pick the side of dynamic (`_`) ports facing the other end.
/// `compassPort` regenerates every field but the name, so `tailclip=false` is lost.
pub fn resolve_ports(g: &mut LGraph<'_>, e: usize) {
    let (t, h) = (g.edges[e].tail, g.edges[e].head);
    if g.edges[e].tail_port.dyna {
        let old = g.edges[e].tail_port.clone();
        g.edges[e].tail_port = resolve_port(g, t, h, &old);
    }
    if g.edges[e].head_port.dyna {
        let old = g.edges[e].head_port.clone();
        g.edges[e].head_port = resolve_port(g, h, t, &old);
    }
}

impl PortGraph for LGraph<'_> {
    fn port_node(&self, n: usize) -> &NodeData {
        &self.src.nodes[n]
    }
    fn port_env(&self) -> graphviz_pure_geom::NodeSizeEnv<'_> {
        graphviz_pure_geom::NodeSizeEnv::from_graph(self.src)
    }
}

/// Resolves `tailport`/`headport` of edge `ei` (`chkPort` in `common_init_edge`), keeping
/// `*clip=false`.
pub fn resolve_edge_ports(g: &mut LGraph<'_>, ei: usize) -> graphviz_pure_core::Result<()> {
    let src = &g.src.edges[ei];
    let (t, h) = (g.edges[ei].tail, g.edges[ei].head);
    let (tsrc, hsrc) = (src.tail_port.clone(), src.head_port.clone());
    let tclip = g.edges[ei].tail_port.clip;
    let hclip = g.edges[ei].head_port.clip;
    let mut tp = chk_port(g, t, &tsrc)?;
    let mut hp = chk_port(g, h, &hsrc)?;
    if !tclip {
        tp.clip = false;
    }
    if !hclip {
        hp.clip = false;
    }
    g.edges[ei].tail_port = tp;
    g.edges[ei].head_port = hp;
    Ok(())
}
