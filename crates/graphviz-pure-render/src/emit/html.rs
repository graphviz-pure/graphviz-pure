//! HTML-like label drawing: port of the emit half of `lib/common/htmltable.c`
//! (`emit_html_label`, `emit_html_tbl`, `emit_html_cell`, `emit_html_rules`, `doBorder`,
//! `setFill`, `emit_html_txt`, `emit_htextspans`, `initAnchor`, `endAnchor`,
//! `pushFontInfo`). Sizing and positioning come from `graphviz_pure_geom::html_label`.

use super::attrs::{StyleItem, default_line_style};
use super::emit::{DEFAULT_COLOR, find_stop_color};
use super::job::{EMIT_CLUSTERS_LAST, Emitter, Engine, FILL, GRADIENT, ObjKind, RGRADIENT};
use super::model::{TextFont, TextLabel, TextSpan};
use super::shapes::PolyStyle;
use graphviz_pure_core::font::strdup_and_subst_obj;
use graphviz_pure_core::{BoundingBox, Point};
use graphviz_pure_geom::html_label::{
    BORDER_BOTTOM, BORDER_LEFT, BORDER_MASK, BORDER_RIGHT, BORDER_TOP, HtmlCellData, HtmlContent,
    HtmlData, HtmlImage, HtmlLabel, HtmlTbl, HtmlText,
};

struct Env {
    pos: Point,
    name: String,
    color: String,
    size: f64,
    objid: Option<String>,
    /// `env.imgscale`: the object's `imagescale`, `false` when unset or empty.
    imgscale: String,
}

struct Saved {
    url: Option<String>,
    tooltip: Option<String>,
    target: Option<String>,
    id: Option<String>,
    explicit_tooltip: bool,
}

fn offset(b: BoundingBox, p: Point) -> BoundingBox {
    BoundingBox::new(b.ll.x + p.x, b.ll.y + p.y, b.ur.x + p.x, b.ur.y + p.y)
}

/// `mkPts`.
fn mk_pts(b: BoundingBox, border: u8) -> [Point; 4] {
    let (mut ll, mut ur) = (b.ll, b.ur);
    if border > 1 {
        let d = f64::from(border) / 2.0;
        ll.x += d;
        ll.y += d;
        ur.x -= d;
        ur.y -= d;
    }
    [ll, Point::new(ur.x, ll.y), ur, Point::new(ll.x, ur.y)]
}

thread_local! {
    static ANCHOR_ID: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// Resets per-process HTML anchor numbering (`static int anchorId`).
pub(crate) fn reset_anchor_ids() {
    ANCHOR_ID.with(|c| c.set(0));
}

impl<E: Engine> Emitter<'_, E> {
    /// `emit_html_label`.
    pub(crate) fn emit_html_label(&mut self, tp: &TextLabel) {
        let Some(lp) = tp.html.as_deref() else {
            return;
        };
        // allocObj
        let parent = self.job.obj().clone();
        let kind = parent.kind;
        self.job.push_obj(kind);
        {
            let o = self.job.obj_mut();
            o.emit_state = parent.emit_state;
            o.url = parent.url.clone();
            o.tooltip = parent.tooltip.clone();
            o.target = parent.target.clone();
            o.explicit_tooltip = parent.explicit_tooltip;
        }
        let height = match lp {
            HtmlLabel::Table(t) => t.data.bx.ur.y - t.data.bx.ll.y,
            HtmlLabel::Text(t) => t.bx.ur.y - t.bx.ll.y,
        };
        let mut p = tp.pos;
        match tp.valign {
            't' => p.y = tp.pos.y + (tp.space.y - height) / 2.0 - 1.0,
            'b' => p.y = tp.pos.y - (tp.space.y - height) / 2.0 - 1.0,
            _ => {}
        }
        let mut env = Env {
            pos: p,
            name: tp.fontname.clone().into_owned(),
            color: tp.fontcolor.clone(),
            size: tp.fontsize,
            objid: parent.id.clone(),
            imgscale: self
                .obj_attrs(kind)
                .nonempty("imagescale")
                .unwrap_or("false")
                .to_string(),
        };
        match lp {
            HtmlLabel::Table(tbl) => {
                self.set_style(&default_line_style());
                let pc = tbl
                    .data
                    .pencolor
                    .clone()
                    .unwrap_or(DEFAULT_COLOR.to_string());
                self.set_pencolor(&pc);
                self.emit_html_tbl(tbl, &mut env);
            }
            HtmlLabel::Text(t) => self.emit_html_txt(t, &env),
        }
        self.job.pop_obj();
    }

    /// `emit_html_img`: the image fills its box, scaled by its `SCALE` or the object's
    /// `imagescale`.
    fn emit_html_img(&mut self, cp: &HtmlImage, env: &Env) {
        let bb = offset(cp.bx, env.pos);
        let a = [
            bb.ur,
            Point::new(bb.ll.x, bb.ur.y),
            bb.ll,
            Point::new(bb.ur.x, bb.ll.y),
        ];
        let scale = cp.scale.as_deref().unwrap_or(&env.imgscale);
        // the parser gives every IMG a non-empty SRC
        if let Some(src) = cp.src.as_deref().filter(|s| !s.is_empty()) {
            self.usershape(src, &a, true, scale, "mc");
        }
    }

    fn emit_html_txt(&mut self, tp: &HtmlText, env: &Env) {
        if tp.spans.is_empty() {
            return;
        }
        let b = tp.bx;
        let halfwidth_x = (b.ur.x - b.ll.x) / 2.0;
        let center_x = env.pos.x + (b.ur.x + b.ll.x) / 2.0;
        let py = env.pos.y + (b.ur.y + b.ll.y) / 2.0;
        let left_x = center_x - halfwidth_x;
        let right_x = center_x + halfwidth_x;
        let mut base_y = py + (b.ur.y - b.ll.y) / 2.0;
        for span in &tp.spans {
            let mut x = match span.just {
                'l' => left_x,
                'r' => right_x - span.size,
                _ => center_x - span.size / 2.0,
            };
            base_y -= span.lfsize;
            for item in &span.items {
                // emit_htextspans: tl.font = ti->font, the font size_html_txt interned
                // ti->font->size > 0 ? ti->font->size : finfo.size
                let size = if item.font_size > 0.0 {
                    item.font_size
                } else {
                    env.size
                };
                let name = &item.font_name;
                let color = item.color.clone().unwrap_or_else(|| env.color.clone());
                let flags = item.flags;
                self.set_pencolor(&color);
                let mut font = TextFont::new(
                    graphviz_pure_core::font::shared_font_name(name),
                    size,
                    flags,
                );
                font.ps = graphviz_pure_core::font::translate_postscript_fontname(&item.font_name);
                let ts = TextSpan {
                    str: item.text.clone(),
                    just: 'l',
                    width: item.width,
                    height: span.lfsize,
                    yoffset_centerline: if tp.simple {
                        item.yoffset_centerline
                    } else {
                        1.0
                    },
                    font,
                };
                self.textspan(Point::new(x, base_y), &ts);
                x += item.width;
            }
        }
    }

    fn set_fill(&mut self, color: &str, angle: u16, radial: bool) -> i32 {
        let filled = if let Some((c0, c1, frac)) = find_stop_color(color) {
            self.set_fillcolor(&c0);
            self.set_gradient_vals(
                c1.as_deref().unwrap_or(DEFAULT_COLOR),
                i32::from(angle),
                frac,
            );
            if radial { RGRADIENT } else { GRADIENT }
        } else {
            self.set_fillcolor(color);
            FILL
        };
        self.set_pencolor("transparent");
        filled
    }

    fn do_border(&mut self, dp: &HtmlData, mut b: BoundingBox) {
        let color = dp.pencolor.clone().unwrap_or(DEFAULT_COLOR.to_string());
        self.set_pencolor(&color);
        if dp.style.dashed || dp.style.dotted {
            let s = if dp.style.dashed { "dashed" } else { "dotted" };
            self.set_style(&[StyleItem::simple(s)]);
        } else {
            self.set_style(&default_line_style());
        }
        self.set_penwidth(f64::from(dp.border));
        let sides = dp.flags & BORDER_MASK;
        if dp.style.rounded {
            let af = mk_pts(b, dp.border);
            let style = PolyStyle {
                rounded: true,
                ..Default::default()
            };
            self.round_corners(&af, style, 0);
        } else if sides != 0 {
            let q = mk_pts(b, dp.border);
            let mut af = [Point::default(); 7];
            af[1..5].copy_from_slice(&q);
            let bb = BORDER_BOTTOM;
            let (r, t, l) = (BORDER_RIGHT, BORDER_TOP, BORDER_LEFT);
            if sides == bb {
                self.polyline(&af[1..3]);
            } else if sides == r {
                self.polyline(&af[2..4]);
            } else if sides == t {
                self.polyline(&af[3..5]);
            } else if sides == l {
                af[0] = af[4];
                self.polyline(&af[0..2]);
            } else if sides == bb | r {
                self.polyline(&af[1..4]);
            } else if sides == r | t {
                self.polyline(&af[2..5]);
            } else if sides == t | l {
                af[5] = af[1];
                self.polyline(&af[3..6]);
            } else if sides == l | bb {
                af[0] = af[4];
                self.polyline(&af[0..3]);
            } else if sides == bb | r | t {
                self.polyline(&af[1..5]);
            } else if sides == r | t | l {
                af[5] = af[1];
                self.polyline(&af[2..6]);
            } else if sides == t | l | bb {
                af[5] = af[1];
                af[6] = af[2];
                self.polyline(&af[3..7]);
            } else if sides == l | bb | r {
                af[0] = af[4];
                self.polyline(&af[0..4]);
            } else if sides == t | bb {
                self.polyline(&af[1..3]);
                self.polyline(&af[3..5]);
            } else if sides == l | r {
                af[0] = af[4];
                self.polyline(&af[0..2]);
                self.polyline(&af[2..4]);
            }
        } else {
            if dp.border > 1 {
                let d = f64::from(dp.border) / 2.0;
                b.ll.x += d;
                b.ll.y += d;
                b.ur.x -= d;
                b.ur.y -= d;
            }
            self.boxf(b, 0);
        }
    }

    fn init_anchor(
        &mut self,
        env: &mut Env,
        data: &HtmlData,
        b: BoundingBox,
        saved: &mut Saved,
    ) -> bool {
        let o = self.job.obj();
        *saved = Saved {
            url: o.url.clone(),
            tooltip: o.tooltip.clone(),
            target: o.target.clone(),
            id: o.id.clone(),
            explicit_tooltip: o.explicit_tooltip,
        };
        let id = match data.id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => {
                if env.objid.is_none() {
                    env.objid = o.id.clone();
                }
                let n = ANCHOR_ID.with(|c| {
                    let v = c.get();
                    c.set(v + 1);
                    v
                });
                format!("{}_{n}", env.objid.clone().unwrap_or_default())
            }
        };
        let kind = o.kind;
        let ctx = self.ctx(kind);
        let flags = self.job.flags;
        let o = self.job.obj_mut();
        let mut assigned = false;
        if flags & super::job::GVRENDER_DOES_MAPS != 0 {
            o.id = Some(strdup_and_subst_obj(&id, &ctx));
            if let Some(u) = data.href.as_deref().filter(|s| !s.is_empty()) {
                o.url = Some(strdup_and_subst_obj(u, &ctx));
            }
            assigned = true;
        }
        if flags & super::job::GVRENDER_DOES_TOOLTIPS != 0
            && let Some(t) = data.title.as_deref().filter(|s| !s.is_empty())
        {
            o.tooltip = Some(strdup_and_subst_obj(t, &ctx));
            o.explicit_tooltip = true;
            assigned = true;
        }
        if flags & super::job::GVRENDER_DOES_TARGETS != 0
            && let Some(t) = data.target.as_deref().filter(|s| !s.is_empty())
        {
            o.target = Some(strdup_and_subst_obj(t, &ctx));
            assigned = true;
        }
        let _ = kind == ObjKind::Root;
        let anchored = o.url.is_some() || o.explicit_tooltip;
        if assigned && anchored {
            self.emit_map_rect(b);
            self.begin_obj_anchor();
        }
        assigned
    }

    fn end_anchor_html(&mut self, saved: Saved) {
        let anchored = {
            let o = self.job.obj();
            o.url.is_some() || o.explicit_tooltip
        };
        if anchored {
            self.end_anchor();
        }
        let o = self.job.obj_mut();
        o.url = saved.url;
        o.tooltip = saved.tooltip;
        o.target = saved.target;
        o.id = saved.id;
        o.explicit_tooltip = saved.explicit_tooltip;
    }

    fn emit_html_rules(&mut self, tbl: &HtmlTbl, k: usize, env: &Env) {
        let cp = &tbl.cells[k];
        let nextc = tbl.cells.get(k + 1);
        let color = tbl
            .data
            .pencolor
            .clone()
            .unwrap_or(DEFAULT_COLOR.to_string());
        self.set_fillcolor(&color);
        self.set_pencolor(&color);
        let pts = offset(cp.data.bx, env.pos);
        let space = f64::from(tbl.data.space);
        let border = f64::from(tbl.data.border);
        let (col, colspan) = (usize::from(cp.col), usize::from(cp.colspan));
        let (row, rowspan) = (usize::from(cp.row), usize::from(cp.rowspan));
        let do_side = |em: &mut Self, p: Point, wd: f64, ht: f64| {
            em.boxf(BoundingBox::new(p.x, p.y, p.x + wd, p.y + ht), 1);
        };
        if cp.vruled && col + colspan < tbl.column_count {
            let (base, y) = if row == 0 {
                (border + space / 2.0, pts.ll.y - space / 2.0)
            } else if row + rowspan == tbl.row_count {
                let base = border + space / 2.0;
                (base, pts.ll.y - space / 2.0 - base)
            } else {
                (0.0, pts.ll.y - space / 2.0)
            };
            let rule_pt = Point::new(pts.ur.x + space / 2.0, y);
            let len = base + pts.ur.y - pts.ll.y + space;
            do_side(self, rule_pt, 0.0, len);
        }
        if cp.hruled && row + rowspan < tbl.row_count {
            let mut base;
            let x;
            let pos = env.pos;
            if col == 0 {
                base = border + space / 2.0;
                x = pts.ll.x - base - space / 2.0;
                if col + colspan == tbl.column_count {
                    base *= 2.0;
                } else if nextc.is_some_and(|n| n.row != cp.row) {
                    base += tbl.data.bx.ur.x + pos.x - (pts.ur.x + space / 2.0);
                }
            } else if col + colspan == tbl.column_count {
                base = border + space / 2.0;
                x = pts.ll.x - space / 2.0;
            } else {
                base = 0.0;
                x = pts.ll.x - space / 2.0;
                if nextc.is_some_and(|n| n.row != cp.row) {
                    base += tbl.data.bx.ur.x + pos.x - (pts.ur.x + space / 2.0);
                }
            }
            let rule_pt = Point::new(x, pts.ll.y - space / 2.0);
            let len = base + pts.ur.x - pts.ll.x + space;
            do_side(self, rule_pt, len, 0.0);
        }
    }

    fn emit_html_tbl(&mut self, tbl: &HtmlTbl, env: &mut Env) {
        let saved_font = tbl.font.as_ref().map(|f| {
            let save = (env.name.clone(), env.color.clone(), env.size);
            if let Some(n) = &f.name {
                env.name = n.clone();
            }
            if let Some(c) = &f.color {
                env.color = c.clone();
            }
            if f.size >= 0.0 {
                env.size = f.size;
            }
            save
        });
        let pts = offset(tbl.data.bx, env.pos);
        let do_anchor =
            tbl.data.href.is_some() || tbl.data.target.is_some() || tbl.data.title.is_some();
        let mut saved = Saved {
            url: None,
            tooltip: None,
            target: None,
            id: None,
            explicit_tooltip: false,
        };
        let anchor = do_anchor
            && self.job.flags & EMIT_CLUSTERS_LAST == 0
            && self.init_anchor(env, &tbl.data, pts, &mut saved);
        if !tbl.data.style.invisible {
            if let Some(bg) = &tbl.data.bgcolor {
                let filled = self.set_fill(bg, tbl.data.gradientangle, tbl.data.style.radial);
                if tbl.data.style.rounded {
                    let af = mk_pts(pts, tbl.data.border);
                    let st = PolyStyle {
                        rounded: true,
                        ..Default::default()
                    };
                    self.round_corners(&af, st, filled);
                } else {
                    self.boxf(pts, filled);
                }
            }
            self.set_penwidth(1.0);
            for k in 0..tbl.cells.len() {
                let cp = &tbl.cells[k];
                if cp.hruled || cp.vruled {
                    self.emit_html_rules(tbl, k, env);
                }
            }
            if tbl.data.border != 0 {
                self.do_border(&tbl.data, pts);
            }
            for cp in &tbl.cells {
                self.emit_html_cell(cp, env);
            }
        }
        if anchor
            || (do_anchor
                && self.job.flags & EMIT_CLUSTERS_LAST != 0
                && self.init_anchor(env, &tbl.data, pts, &mut saved))
        {
            self.end_anchor_html(saved);
        }
        if let Some((n, c, s)) = saved_font {
            env.name = n;
            env.color = c;
            env.size = s;
        }
    }

    fn emit_html_cell(&mut self, cp: &HtmlCellData, env: &mut Env) {
        let pts = offset(cp.data.bx, env.pos);
        let do_anchor =
            cp.data.href.is_some() || cp.data.target.is_some() || cp.data.title.is_some();
        let mut saved = Saved {
            url: None,
            tooltip: None,
            target: None,
            id: None,
            explicit_tooltip: false,
        };
        let anchor = do_anchor
            && self.job.flags & EMIT_CLUSTERS_LAST == 0
            && self.init_anchor(env, &cp.data, pts, &mut saved);
        if !cp.data.style.invisible {
            if let Some(bg) = &cp.data.bgcolor {
                let filled = self.set_fill(bg, cp.data.gradientangle, cp.data.style.radial);
                if cp.data.style.rounded {
                    let af = mk_pts(pts, cp.data.border);
                    let st = PolyStyle {
                        rounded: true,
                        ..Default::default()
                    };
                    self.round_corners(&af, st, filled);
                } else {
                    self.boxf(pts, filled);
                }
            }
            if cp.data.border != 0 {
                self.do_border(&cp.data, pts);
            }
            match &cp.child {
                HtmlContent::Table(t) => self.emit_html_tbl(t, env),
                HtmlContent::Image(img) => self.emit_html_img(img, env),
                HtmlContent::Text(t) => self.emit_html_txt(t, env),
            }
        }
        if anchor
            || (do_anchor
                && self.job.flags & EMIT_CLUSTERS_LAST != 0
                && self.init_anchor(env, &cp.data, pts, &mut saved))
        {
            self.end_anchor_html(saved);
        }
    }
}
