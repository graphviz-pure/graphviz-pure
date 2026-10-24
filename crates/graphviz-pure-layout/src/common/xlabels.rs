//! External label placement: `addXLabels` (`lib/common/postproc.c`) and
//! `placeLabels` with its R-tree (`lib/label/xlabels.c`, `index.c`,
//! `node.c`, `rectangle.c`, `split.q.c`).
//!
//! Positions every unset node `xlabel` and edge `label`/`xlabel`/`headlabel`/
//! `taillabel` next to its anchor so it avoids nodes and other labels.

use super::splines::{
    LabelKind, LabelView, SplineGraph, add_label_bb_dims, edge_midpoint, getsplinepoints,
    update_bb_label,
};
use super::types::{EdgeType, LGraph};
use crate::cutil::DtTree;
use graphviz_pure_core::cnum::{c_max, c_min};
use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, Point};

// ---------------------------------------------------------------- R-tree

const NODECARD: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Rect {
    b: [f64; 4],
}

fn undefined(r: &Rect) -> bool {
    r.b[0] > r.b[2]
}

fn null_rect() -> Rect {
    Rect {
        b: [1.0, 0.0, -1.0, 0.0],
    }
}

/// `RectArea`: integral area (C converts each side to `uint64_t`).
fn rect_area(r: &Rect) -> u64 {
    if undefined(r) {
        return 0;
    }
    let mut area: u64 = 1;
    for i in 0..2 {
        let dim = (r.b[i + 2] - r.b[i]) as u64;
        if dim == 0 {
            return 0;
        }
        area = area.wrapping_mul(dim);
    }
    area
}

/// `CombineRect`. C takes `fmin` of the high sides too; kept as is.
fn combine_rect(r: &Rect, rr: &Rect) -> Rect {
    if undefined(r) {
        return *rr;
    }
    if undefined(rr) {
        return *r;
    }
    let mut n = Rect::default();
    for i in 0..2 {
        n.b[i] = r.b[i].min(rr.b[i]);
        n.b[i + 2] = r.b[i + 2].min(rr.b[i + 2]);
    }
    n
}

fn overlap(r: &Rect, s: &Rect) -> bool {
    for i in 0..2 {
        if r.b[i] > s.b[i + 2] || s.b[i] > r.b[i + 2] {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy)]
enum Child {
    None,
    Node(usize),
    Data(usize),
}

#[derive(Clone, Copy)]
struct Branch {
    rect: Rect,
    child: Child,
}

impl Default for Branch {
    fn default() -> Self {
        Branch {
            rect: Rect::default(),
            child: Child::None,
        }
    }
}

#[derive(Clone)]
struct RNode {
    count: usize,
    level: i32,
    branch: Vec<Branch>,
}

impl RNode {
    fn new() -> Self {
        RNode {
            count: 0,
            level: -1,
            branch: vec![Branch::default(); NODECARD],
        }
    }
}

struct RTree {
    nodes: Vec<RNode>,
    root: usize,
}

fn has(b: &Branch) -> bool {
    !matches!(b.child, Child::None)
}

impl RTree {
    fn new() -> Self {
        let mut n = RNode::new();
        n.level = 0;
        RTree {
            nodes: vec![n],
            root: 0,
        }
    }

    fn node_cover(&self, n: usize) -> Rect {
        let mut r = Rect::default();
        let mut first = true;
        for b in &self.nodes[n].branch {
            if has(b) {
                if first {
                    r = b.rect;
                    first = false;
                } else {
                    r = combine_rect(&r, &b.rect);
                }
            }
        }
        r
    }

    fn pick_branch(&self, r: &Rect, n: usize) -> usize {
        let (mut best_incr, mut best_area, mut best, mut set) = (0u64, 0u64, 0usize, false);
        for (i, b) in self.nodes[n].branch.iter().enumerate() {
            if has(b) {
                let area = rect_area(&b.rect);
                let rect = combine_rect(r, &b.rect);
                let increase = rect_area(&rect).wrapping_sub(area);
                if !set || increase < best_incr {
                    best = i;
                    best_area = area;
                    best_incr = increase;
                    set = true;
                } else if increase == best_incr && area < best_area {
                    best = i;
                    best_area = area;
                    best_incr = increase;
                }
            }
        }
        best
    }

    /// `AddBranch`: returns the new node when `n` had to be split.
    fn add_branch(&mut self, b: Branch, n: usize) -> Option<usize> {
        if self.nodes[n].count < NODECARD {
            let node = &mut self.nodes[n];
            let i = node
                .branch
                .iter()
                .position(|x| !has(x))
                .expect("free branch");
            node.branch[i] = b;
            node.count += 1;
            None
        } else {
            Some(self.split_node(n, b))
        }
    }

    /// `SplitNode` with the quadratic `MethodZero`.
    fn split_node(&mut self, n: usize, b: Branch) -> usize {
        let level = self.nodes[n].level;
        let mut buf: Vec<Branch> = self.nodes[n].branch.clone();
        buf.push(b);
        self.nodes[n] = RNode::new();
        let total = NODECARD + 1;
        let mut count = [0usize; 2];
        let mut cover = [null_rect(), null_rect()];
        let mut area = [0u64; 2];
        let mut taken = vec![false; total];
        let mut partition = vec![-1i32; total];
        let classify = |i: usize,
                        group: usize,
                        count: &mut [usize; 2],
                        cover: &mut [Rect; 2],
                        area: &mut [u64; 2],
                        taken: &mut [bool],
                        partition: &mut [i32]| {
            partition[i] = group as i32;
            taken[i] = true;
            cover[group] = if count[group] == 0 {
                buf[i].rect
            } else {
                combine_rect(&buf[i].rect, &cover[group])
            };
            area[group] = rect_area(&cover[group]);
            count[group] += 1;
        };
        // PickSeeds
        let areas: Vec<u64> = buf.iter().map(|b| rect_area(&b.rect)).collect();
        let (mut seed0, mut seed1, mut worst) = (0usize, 0usize, 0u64);
        for i in 0..NODECARD {
            for j in (i + 1)..total {
                let rect = combine_rect(&buf[i].rect, &buf[j].rect);
                let waste = rect_area(&rect)
                    .wrapping_sub(areas[i])
                    .wrapping_sub(areas[j]);
                if waste > worst {
                    worst = waste;
                    seed0 = i;
                    seed1 = j;
                }
            }
        }
        classify(
            seed0,
            0,
            &mut count,
            &mut cover,
            &mut area,
            &mut taken,
            &mut partition,
        );
        classify(
            seed1,
            1,
            &mut count,
            &mut cover,
            &mut area,
            &mut taken,
            &mut partition,
        );
        while count[0] + count[1] < total && count[0] < total && count[1] < total {
            let (mut set, mut biggest, mut chosen, mut better) = (false, 0u64, 0usize, 0usize);
            for i in 0..total {
                if taken[i] {
                    continue;
                }
                let r = buf[i].rect;
                let g0 = rect_area(&combine_rect(&r, &cover[0])).wrapping_sub(area[0]);
                let g1 = rect_area(&combine_rect(&r, &cover[1])).wrapping_sub(area[1]);
                let (diff, group) = if g1 >= g0 { (g1 - g0, 0) } else { (g0 - g1, 1) };
                if !set || diff > biggest {
                    biggest = diff;
                    set = true;
                    chosen = i;
                    better = group;
                } else if diff == biggest && count[group] < count[better] {
                    chosen = i;
                    better = group;
                }
            }
            classify(
                chosen,
                better,
                &mut count,
                &mut cover,
                &mut area,
                &mut taken,
                &mut partition,
            );
        }
        if count[0] + count[1] < total {
            let group = if count[0] >= total { 1 } else { 0 };
            for i in 0..total {
                if !taken[i] {
                    classify(
                        i,
                        group,
                        &mut count,
                        &mut cover,
                        &mut area,
                        &mut taken,
                        &mut partition,
                    );
                }
            }
        }
        let mut q = RNode::new();
        q.level = level;
        self.nodes.push(q);
        let qn = self.nodes.len() - 1;
        self.nodes[n].level = level;
        for i in 0..total {
            let dst = if partition[i] == 0 { n } else { qn };
            let _ = self.add_branch(buf[i], dst);
        }
        qn
    }

    fn insert2(&mut self, r: Rect, data: usize, n: usize, level: i32) -> Option<usize> {
        if self.nodes[n].level > level {
            let i = self.pick_branch(&r, n);
            let Child::Node(c) = self.nodes[n].branch[i].child else {
                unreachable!()
            };
            match self.insert2(r, data, c, level) {
                None => {
                    let old = self.nodes[n].branch[i].rect;
                    self.nodes[n].branch[i].rect = combine_rect(&r, &old);
                    None
                }
                Some(n2) => {
                    self.nodes[n].branch[i].rect = self.node_cover(c);
                    let b = Branch {
                        rect: self.node_cover(n2),
                        child: Child::Node(n2),
                    };
                    self.add_branch(b, n)
                }
            }
        } else {
            self.add_branch(
                Branch {
                    rect: r,
                    child: Child::Data(data),
                },
                n,
            )
        }
    }

    /// `RTreeInsert`.
    fn insert(&mut self, r: Rect, data: usize) {
        let root = self.root;
        if let Some(newnode) = self.insert2(r, data, root, 0) {
            let mut nr = RNode::new();
            nr.level = self.nodes[root].level + 1;
            self.nodes.push(nr);
            let nri = self.nodes.len() - 1;
            let b1 = Branch {
                rect: self.node_cover(root),
                child: Child::Node(root),
            };
            let _ = self.add_branch(b1, nri);
            let b2 = Branch {
                rect: self.node_cover(newnode),
                child: Child::Node(newnode),
            };
            let _ = self.add_branch(b2, nri);
            self.root = nri;
        }
    }

    /// `RTreeSearch`: data ids in C's list order.
    fn search(&self, n: usize, r: &Rect) -> Vec<usize> {
        let node = &self.nodes[n];
        let mut out = Vec::new();
        if node.level > 0 {
            for b in &node.branch {
                if let Child::Node(c) = b.child
                    && overlap(r, &b.rect)
                {
                    out.extend(self.search(c, r));
                }
            }
        } else {
            for b in &node.branch {
                if let Child::Data(d) = b.child
                    && overlap(r, &b.rect)
                {
                    // RTreeLeafListAdd prepends.
                    out.insert(0, d);
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------ placement

#[derive(Debug, Clone, Copy, Default)]
struct XLabel {
    sz: Point,
    pos: Point,
    set: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Object {
    pos: Point,
    sz: Point,
    lbl: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BestPos {
    n: i32,
    area: f64,
    pos: Point,
}

const XLXDENOM: f64 = 8.0;
const XLYDENOM: f64 = 2.0;

struct Ctx<'a> {
    objs: &'a [Object],
    lbls: &'a mut [XLabel],
    tree: RTree,
}

fn objp2rect(op: &Object) -> Rect {
    Rect {
        b: [
            op.pos.x.round(),
            op.pos.y.round(),
            (op.pos.x + op.sz.x).round(),
            (op.pos.y + op.sz.y).round(),
        ],
    }
}

fn lbl2rect(lp: &XLabel) -> Rect {
    Rect {
        b: [
            lp.pos.x.round(),
            lp.pos.y.round(),
            (lp.pos.x + lp.sz.x).round(),
            (lp.pos.y + lp.sz.y).round(),
        ],
    }
}

fn aabbaabb(r: &Rect, s: &Rect) -> f64 {
    if !overlap(r, s) {
        return 0.0;
    }
    let iminx = r.b[0].max(s.b[0]);
    let iminy = r.b[1].max(s.b[1]);
    let imaxx = r.b[2].min(s.b[2]);
    let imaxy = r.b[3].min(s.b[3]);
    (imaxx - iminx) * (imaxy - iminy)
}

impl Ctx<'_> {
    fn lblenclosing(&self, oi: usize, o1: usize) -> bool {
        let Some(l) = self.objs[oi].lbl else {
            return false;
        };
        let xlp = &self.lbls[l];
        let p = self.objs[o1].pos;
        p.x > xlp.pos.x
            && p.x < xlp.pos.x + xlp.sz.x
            && p.y > xlp.pos.y
            && p.y < xlp.pos.y + xlp.sz.y
    }

    fn getintrsxi(&self, op: usize, cp: usize) -> i32 {
        let (o, c) = (&self.objs[op], &self.objs[cp]);
        let (Some(lp), Some(clp)) = (o.lbl, c.lbl) else {
            return -1;
        };
        if !self.lbls[lp].set || !self.lbls[clp].set {
            return -1;
        }
        if (o.pos.x == 0.0 && o.pos.y == 0.0) || (c.pos.x == 0.0 && c.pos.y == 0.0) {
            return -1;
        }
        if c.pos.y < o.pos.y {
            if c.pos.x < o.pos.x {
                0
            } else if c.pos.x > o.pos.x {
                2
            } else {
                1
            }
        } else if c.pos.y > o.pos.y {
            if c.pos.x < o.pos.x {
                6
            } else if c.pos.x > o.pos.x {
                8
            } else {
                7
            }
        } else if c.pos.x < o.pos.x {
            3
        } else if c.pos.x > o.pos.x {
            5
        } else {
            -1
        }
    }

    /// `recordointrsx` / `recordlintrsx` (identical logic).
    fn record(
        &self,
        op: usize,
        cp: usize,
        rp: &Rect,
        a: f64,
        intrsx: &mut [Option<usize>; 9],
    ) -> f64 {
        let mut i = self.getintrsxi(op, cp);
        if i < 0 {
            i = 5;
        }
        let i = i as usize;
        if let Some(prev) = intrsx[i] {
            let mut maxa = 0.0f64;
            let sa = aabbaabb(rp, &objp2rect(&self.objs[prev]));
            if sa > a {
                maxa = sa;
            }
            if let Some(pl) = self.objs[prev].lbl {
                let sa = aabbaabb(rp, &lbl2rect(&self.lbls[pl]));
                if sa > a {
                    maxa = sa.max(maxa);
                }
            }
            if maxa > 0.0 {
                return maxa;
            }
            intrsx[i] = Some(cp);
            return a;
        }
        intrsx[i] = Some(cp);
        a
    }

    fn intersections(&self, oi: usize, intrsx: &mut [Option<usize>; 9]) -> BestPos {
        let l = self.objs[oi].lbl.unwrap();
        let mut bp = BestPos {
            pos: self.lbls[l].pos,
            ..BestPos::default()
        };
        for i in 0..self.objs.len() {
            if i == oi {
                continue;
            }
            if self.objs[i].sz.x > 0.0 && self.objs[i].sz.y > 0.0 {
                continue;
            }
            if self.lblenclosing(oi, i) {
                bp.n += 1;
            }
        }
        let rect = lbl2rect(&self.lbls[l]);
        let root = self.tree.root;
        for cp in self.tree.search(root, &rect) {
            if cp == oi {
                continue;
            }
            let a = aabbaabb(&rect, &objp2rect(&self.objs[cp]));
            if a > 0.0 {
                let ra = self.record(oi, cp, &rect, a, intrsx);
                bp.n += 1;
                bp.area += ra;
            }
            let Some(cl) = self.objs[cp].lbl else {
                continue;
            };
            if !self.lbls[cl].set {
                continue;
            }
            let a = aabbaabb(&rect, &lbl2rect(&self.lbls[cl]));
            if a > 0.0 {
                let ra = self.record(oi, cp, &rect, a, intrsx);
                bp.n += 1;
                bp.area += ra;
            }
        }
        bp
    }

    fn adjust(&mut self, oi: usize) -> BestPos {
        let o = self.objs[oi];
        let l = o.lbl.unwrap();
        let sz = self.lbls[l].sz;
        let xincr = (2.0 * sz.x + o.sz.x) / XLXDENOM;
        let yincr = (2.0 * sz.y + o.sz.y) / XLYDENOM;
        let mut intrsx: [Option<usize>; 9] = [None; 9];
        macro_rules! try_at {
            ($x:expr, $y:expr, $bp:ident) => {{
                self.lbls[l].pos = Point::new($x, $y);
                let nbp = self.intersections(oi, &mut intrsx);
                if nbp.n == 0 {
                    return nbp;
                }
                if nbp.area < $bp.area {
                    $bp = nbp;
                }
            }};
        }
        self.lbls[l].pos = Point::new(o.pos.x - sz.x, o.pos.y + o.sz.y);
        let mut bp = self.intersections(oi, &mut intrsx);
        if bp.n == 0 {
            return bp;
        }
        try_at!(o.pos.x - sz.x, o.pos.y, bp);
        try_at!(o.pos.x - sz.x, o.pos.y - sz.y, bp);
        try_at!(o.pos.x, o.pos.y + o.sz.y, bp);
        try_at!(o.pos.x, o.pos.y - sz.y, bp);
        try_at!(o.pos.x + o.sz.x, o.pos.y + o.sz.y, bp);
        try_at!(o.pos.x + o.sz.x, o.pos.y, bp);
        try_at!(o.pos.x + o.sz.x, o.pos.y - sz.y, bp);
        let has = |i: usize, ix: &[Option<usize>; 9]| ix[i].is_some();
        if has(6, &intrsx)
            || has(7, &intrsx)
            || has(8, &intrsx)
            || has(3, &intrsx)
            || has(0, &intrsx)
        {
            if !has(7, &intrsx) && !has(8, &intrsx) {
                let mut x = o.pos.x - sz.x;
                let y = o.pos.y + o.sz.y;
                while x <= o.pos.x + o.sz.x {
                    try_at!(x, y, bp);
                    x += xincr;
                }
            }
            if !has(3, &intrsx) && !has(0, &intrsx) {
                let x = o.pos.x - sz.x;
                let mut y = o.pos.y + o.sz.y;
                while y >= o.pos.y - sz.y {
                    try_at!(x, y, bp);
                    y -= yincr;
                }
            }
        }
        self.lbls[l].pos = Point::new(o.pos.x + o.sz.x, o.pos.y - sz.y);
        if has(2, &intrsx)
            || has(1, &intrsx)
            || has(0, &intrsx)
            || has(5, &intrsx)
            || has(8, &intrsx)
        {
            if !has(1, &intrsx) && !has(0, &intrsx) {
                let mut x = o.pos.x + o.sz.x;
                let y = o.pos.y - sz.y;
                while x >= o.pos.x - sz.x {
                    try_at!(x, y, bp);
                    x -= xincr;
                }
            }
            if !has(5, &intrsx) && !has(8, &intrsx) {
                let x = o.pos.x + o.sz.x;
                let mut y = o.pos.y - sz.y;
                while y <= o.pos.y + o.sz.y {
                    try_at!(x, y, bp);
                    y += yincr;
                }
            }
        }
        bp
    }
}

/// `hd_hil_s_from_xy(p, n)`: Hilbert curve index.
fn hilbert(x0: i32, y0: i32, n: u32) -> u32 {
    let (mut x, mut y) = (x0, y0);
    let mut s: u32 = 0;
    if n == 0 {
        return 0;
    }
    let mut i = n - 1;
    loop {
        // An `order` above 32 (a graph bb reaching 2^32) shifts an `int` by
        // 32 or more, undefined in C; arm64's `asr` takes the count mod 32.
        let xi = x.wrapping_shr(i) & 1;
        let yi = y.wrapping_shr(i) & 1;
        s = s
            .wrapping_mul(4)
            .wrapping_add(2 * xi as u32)
            .wrapping_add((xi ^ yi) as u32);
        x ^= y;
        y ^= x & (yi - 1);
        x ^= y;
        x ^= (-xi) & (yi - 1);
        y ^= (-xi) & (yi - 1);
        if i == 0 {
            break;
        }
        i -= 1;
    }
    s
}

/// `xlhorder(obj_bb)`: `(unsigned)floor(log2(round(fmax(maxx, maxy)))) + 1`.
/// `as u32` saturates like arm64's `fcvtzu`, and the add wraps as C's
/// `unsigned` does: an infinite bb (#191) gives order 0, the order C computes
/// before `xlhdxload` fails its `INT_MIN..INT_MAX` assertion on the label
/// centre, which [`place_labels`] ports as a failure.
fn xlhorder(bb: BoundingBox) -> u32 {
    (math::log2((bb.ur.x.max(bb.ur.y)).round()).floor() as u32).wrapping_add(1)
}

/// `placeLabels(objs, n_objs, params)`.
fn place_labels(objs: &[Object], lbls: &mut [XLabel], bb: BoundingBox, force: bool) {
    let order = xlhorder(bb);
    // `HDict_t.key` is an `int`, and `Hdisc`'s `icompare` compares the keys
    // as `int`, so the unsigned Hilbert index is reinterpreted as signed:
    // keys with bit 31 set (coordinates reaching 2^15) sort first, not last.
    let mut keyed: Vec<(i32, Rect)> = Vec::with_capacity(objs.len());
    for o in objs {
        let p = o.lbl.map(|l| lbls[l].sz).unwrap_or_default();
        let r = Rect {
            b: [
                (o.pos.x - p.x).floor(),
                (o.pos.y - p.y).floor(),
                (o.pos.x + o.sz.x + p.x).ceil(),
                (o.pos.y + o.sz.y + p.y).ceil(),
            ],
        };
        let x = r.b[0] + (r.b[2] - r.b[0]) / 2.0;
        let y = r.b[1] + (r.b[3] - r.b[1]) / 2.0;
        // xlhdxload asserts `x >= INT_MIN && x <= INT_MAX` (and the same for
        // `y`) before the `(int)` cast, and aborts on an infinite, NaN or huge
        // centre (#191, #301; past it, on an infinite label, `adjust`'s
        // downward and leftward slides stay at `-inf` forever). The port stops
        // there as `xlhdxload`'s own failure does: `placeLabels` returns -1
        // and `addXLabels` places no label.
        let in_int = |v: f64| v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX);
        if !in_int(x) || !in_int(y) {
            graphviz_pure_core::diag::agwarningf(
                "xlabels: a label area's centre does not fit an int; exterior labels are not placed\n",
            );
            return;
        }
        keyed.push((hilbert(x as i32, y as i32, order) as i32, r));
    }
    // lib/label/xlabels.c:xlnew/xlhdxload: `hdx` is a Dtobag keyed by the
    // Hilbert index. Where equal keys land depends on the splay tree's shape
    // (lib/cdt/dttree.c inserts a duplicate just before the equal element its
    // search stops at), so the tree is reproduced rather than sorted.
    let mut hdx = DtTree::new(true);
    for i in 0..keyed.len() {
        hdx.insert(i, |a, b| keyed[a].0.cmp(&keyed[b].0));
    }
    // lib/label/xlabels.c:xlspdxload: dtfirst/dtnext walk the dictionary in order.
    let mut tree = RTree::new();
    for i in hdx.flatten() {
        tree.insert(keyed[i].1, i);
    }
    let mut ctx = Ctx { objs, lbls, tree };
    for i in 0..objs.len() {
        let Some(l) = objs[i].lbl else { continue };
        let bp = ctx.adjust(i);
        if bp.n == 0 {
            ctx.lbls[l].set = true;
        } else if bp.area == 0.0 || force {
            ctx.lbls[l].pos = bp.pos;
            ctx.lbls[l].set = true;
        }
    }
}

/// Extra layout state `addXLabels` reads beyond [`SplineGraph`].
pub trait XLabelGraph: SplineGraph {
    /// Nodes in `agfstnode` order.
    fn xl_nodes(&self) -> Vec<usize>;
    /// Out-edges of `n` in `agfstout` order.
    fn xl_out_edges(&self, n: usize) -> Vec<usize>;
    /// `INCH2PS(ND_width(n))`, `INCH2PS(ND_height(n))`.
    fn xl_node_size(&self, n: usize) -> Point;
    /// `ND_xlabel(n)`.
    fn node_xlabel(&self, n: usize) -> Option<LabelView>;
    fn set_node_xlabel_pos(&mut self, n: usize, pos: Point);
    /// Set cluster labels in `addClusterObj` order (children first).
    fn xl_cluster_labels(&self) -> Vec<LabelView>;
    /// `EDGE_TYPE(g)`.
    fn xl_edge_type(&self) -> EdgeType;
    /// `late_bool(gp, agfindgraphattr(gp, "forcelabels"), true)`.
    fn xl_forcelabels(&self) -> bool;
    /// `lp->text` of edge label `k` of `e` (empty when the edge has no such label).
    fn xl_label_text(&self, e: usize, k: LabelKind) -> String;
    /// `ND_xlabel(n)->text` (empty without one).
    fn xl_node_xlabel_text(&self, n: usize) -> String;
}

#[derive(Debug, Clone, Copy)]
enum LblRef {
    NodeX(usize),
    Edge(usize, LabelKind),
}

fn adjust_bb(o: &Object, bb: &mut BoundingBox) {
    bb.ll.x = c_min(bb.ll.x, o.pos.x);
    bb.ll.y = c_min(bb.ll.y, o.pos.y);
    bb.ur.x = c_max(bb.ur.x, o.pos.x + o.sz.x);
    bb.ur.y = c_max(bb.ur.y, o.pos.y + o.sz.y);
}

/// `addLabelObj(lp, objp, bb)`.
fn label_obj(lp: &LabelView, flip: bool, bb: &mut BoundingBox) -> Object {
    let sz = if flip {
        Point::new(lp.dimen.y, lp.dimen.x)
    } else {
        lp.dimen
    };
    let o = Object {
        sz,
        pos: Point::new(lp.pos.x - sz.x / 2.0, lp.pos.y - sz.y / 2.0),
        lbl: None,
    };
    adjust_bb(&o, bb);
    o
}

/// `addXLabels(gp)`.
pub fn add_xlabels<S: XLabelGraph + ?Sized>(g: &mut S) {
    let et = g.xl_edge_type();
    let flip = g.flip();
    let have_edge = |g: &S, e: usize| et != EdgeType::None && g.spl(e).is_some();
    let kinds = [
        LabelKind::Main,
        LabelKind::Tail,
        LabelKind::Head,
        LabelKind::Xlabel,
    ];
    let nodes = g.xl_nodes();
    let mut n_lbls = 0;
    for &n in &nodes {
        if g.node_xlabel(n).is_some_and(|l| !l.set) {
            n_lbls += 1;
        }
        for e in g.xl_out_edges(n) {
            for k in kinds {
                if g.label(e, k).is_some_and(|l| !l.set) && have_edge(g, e) {
                    n_lbls += 1;
                }
            }
        }
    }
    if n_lbls == 0 {
        return;
    }
    let mut objs: Vec<Object> = Vec::new();
    let mut lbls: Vec<XLabel> = Vec::new();
    let mut refs: Vec<LblRef> = Vec::new();
    let mut bb = BoundingBox::new(f64::MAX, f64::MAX, -f64::MAX, -f64::MAX);
    let xsz = |d: Point| if flip { Point::new(d.y, d.x) } else { d };
    for &n in &nodes {
        let s = g.xl_node_size(n);
        let sz = if flip { Point::new(s.y, s.x) } else { s };
        let c = g.coord(n);
        let o = Object {
            sz,
            pos: Point::new(c.x - sz.x / 2.0, c.y - sz.y / 2.0),
            lbl: None,
        };
        adjust_bb(&o, &mut bb);
        objs.push(o);
        if let Some(lp) = g.node_xlabel(n) {
            if lp.set {
                let lo = label_obj(&lp, flip, &mut bb);
                objs.push(lo);
            } else {
                let li = lbls.len();
                lbls.push(XLabel {
                    sz: xsz(lp.dimen),
                    pos: Point::default(),
                    set: false,
                });
                refs.push(LblRef::NodeX(n));
                let last = objs.len() - 1;
                objs[last].lbl = Some(li);
            }
        }
        'edges: for e in g.xl_out_edges(n) {
            for k in kinds {
                let Some(lp) = g.label(e, k) else { continue };
                if lp.set {
                    let lo = label_obj(&lp, flip, &mut bb);
                    objs.push(lo);
                } else if have_edge(g, e) {
                    let spl = g.spl(e).unwrap();
                    let pos = match k {
                        LabelKind::Tail => {
                            let b = &getsplinepoints(g, e).unwrap()[0];
                            if b.sflag != 0 { b.sp } else { b.list[0] }
                        }
                        LabelKind::Head => {
                            let sp = getsplinepoints(g, e).unwrap();
                            let b = &sp[sp.len() - 1];
                            if b.eflag != 0 {
                                b.ep
                            } else {
                                b.list[b.list.len() - 1]
                            }
                        }
                        _ => edge_midpoint(et, spl),
                    };
                    let li = lbls.len();
                    lbls.push(XLabel {
                        sz: xsz(lp.dimen),
                        pos: Point::default(),
                        set: false,
                    });
                    refs.push(LblRef::Edge(e, k));
                    objs.push(Object {
                        pos,
                        sz: Point::default(),
                        lbl: Some(li),
                    });
                } else {
                    // postproc.c:500-504, 517, 532, 547: warn and skip the edge's
                    // remaining labels.
                    let what = match k {
                        LabelKind::Main => "label",
                        LabelKind::Tail => "tail label",
                        LabelKind::Head => "head label",
                        LabelKind::Xlabel => "xlabel",
                    };
                    graphviz_pure_core::diag::agwarningf(&format!(
                        "no position for edge with {what} {}\n",
                        g.xl_label_text(e, k)
                    ));
                    continue 'edges;
                }
            }
        }
    }
    for l in g.xl_cluster_labels() {
        objs.push(label_obj(&l, flip, &mut bb));
    }
    let force = g.xl_forcelabels();
    place_labels(&objs, &mut lbls, bb, force);
    use graphviz_pure_core::diag::{fmt_f, fprintf_stderr, verbose_on};
    if verbose_on() {
        // printData
        let f = |v: f64| fmt_f(v, 2);
        let mut m = format!(
            "{} objs {n_lbls} xlabels force={} bb=({},{}) ({},{})\n",
            objs.len(),
            i32::from(force),
            f(bb.ll.x),
            f(bb.ll.y),
            f(bb.ur.x),
            f(bb.ur.y)
        );
        if graphviz_pure_core::diag::verbose_byte() >= 2 {
            let text = |li: usize| match refs.get(li) {
                Some(&LblRef::NodeX(n)) => g.xl_node_xlabel_text(n),
                Some(&LblRef::Edge(e, k)) => g.xl_label_text(e, k),
                None => String::new(),
            };
            // `%p` of a label: the port has no heap address, so it prints a stand-in
            let ptr = |li: Option<usize>| match li {
                None => "0x0".to_string(),
                Some(li) => format!(
                    "0x{:x}",
                    (li as u64).wrapping_mul(48).wrapping_add(0x6000_0000_0000)
                ),
            };
            m.push_str("objects\n");
            for (i, o) in objs.iter().enumerate() {
                m.push_str(&format!(
                    " [{i}] ({},{}) ({},{}) {} \"{}\"\n",
                    f(o.pos.x),
                    f(o.pos.y),
                    f(o.sz.x),
                    f(o.sz.y),
                    ptr(o.lbl),
                    o.lbl.map(text).unwrap_or_default()
                ));
            }
            m.push_str("xlabels\n");
            for (i, l) in lbls.iter().enumerate() {
                m.push_str(&format!(
                    " [{i}] {} set {} ({},{}) ({},{}) {}\n",
                    ptr(Some(i)),
                    i32::from(l.set),
                    f(l.pos.x),
                    f(l.pos.y),
                    f(l.sz.x),
                    f(l.sz.y),
                    text(i)
                ));
            }
        }
        fprintf_stderr(&m);
    }
    let cnt = lbls.iter().filter(|l| l.set).count();
    if verbose_on() {
        fprintf_stderr(&format!("{cnt} out of {n_lbls} labels positioned.\n"));
    } else if cnt != n_lbls {
        graphviz_pure_core::diag::agwarningf(&format!(
            "{cnt} out of {n_lbls} exterior labels positioned.\n"
        ));
    }
    for (li, r) in refs.iter().enumerate() {
        let xl = lbls[li];
        if !xl.set {
            continue;
        }
        let center = Point::new(xl.pos.x + xl.sz.x / 2.0, xl.pos.y + xl.sz.y / 2.0);
        match *r {
            LblRef::NodeX(n) => {
                g.set_node_xlabel_pos(n, center);
                let l = g.node_xlabel(n).unwrap();
                add_label_bb_dims(g.bb_mut(), l.dimen, l.pos, flip);
            }
            LblRef::Edge(e, k) => {
                g.set_label_pos(e, k, center);
                update_bb_label(g, e, k);
            }
        }
    }
}

impl XLabelGraph for LGraph<'_> {
    fn xl_node_xlabel_text(&self, n: usize) -> String {
        self.nodes
            .get(n)
            .and_then(|nd| nd.xlabel.as_ref())
            .map(|l| l.text.clone())
            .unwrap_or_default()
    }
    fn xl_label_text(&self, e: usize, k: LabelKind) -> String {
        let ed = &self.edges[e];
        match k {
            LabelKind::Main => ed.label.as_ref(),
            LabelKind::Head => ed.head_label.as_ref(),
            LabelKind::Tail => ed.tail_label.as_ref(),
            LabelKind::Xlabel => ed.xlabel.as_ref(),
        }
        .map(|l| l.text.clone())
        .unwrap_or_default()
    }
    fn xl_nodes(&self) -> Vec<usize> {
        (0..self.nodes.len()).collect()
    }
    fn xl_out_edges(&self, n: usize) -> Vec<usize> {
        self.out[n].clone()
    }
    fn xl_node_size(&self, n: usize) -> Point {
        Point::new(self.nodes[n].width * 72.0, self.nodes[n].height * 72.0)
    }
    fn node_xlabel(&self, n: usize) -> Option<LabelView> {
        self.nodes[n].xlabel.as_ref().map(|l| LabelView {
            dimen: l.dimen,
            pos: l.pos,
            set: l.set,
        })
    }
    fn set_node_xlabel_pos(&mut self, n: usize, pos: Point) {
        if let Some(l) = self.nodes[n].xlabel.as_mut() {
            l.pos = pos;
            l.set = true;
        }
    }
    fn xl_cluster_labels(&self) -> Vec<LabelView> {
        fn rec(g: &LGraph<'_>, c: usize, out: &mut Vec<LabelView>) {
            for &k in &g.clusters[c].children {
                rec(g, k, out);
            }
            if let Some(l) = g.clusters[c].label.as_ref().filter(|l| l.set) {
                out.push(LabelView {
                    dimen: l.dimen,
                    pos: l.pos,
                    set: true,
                });
            }
        }
        let mut out = Vec::new();
        for &c in &self.root_clusters {
            rec(self, c, &mut out);
        }
        out
    }
    fn xl_edge_type(&self) -> EdgeType {
        self.edge_type
    }
    fn xl_forcelabels(&self) -> bool {
        crate::common::utils::map_bool(self.gattr("forcelabels"), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hilbert_order_above_32_does_not_overflow_the_shift() {
        // C's `x >> i` with `i >= 32` is undefined; arm64 takes the count mod
        // 32, so order 40 walks bits 7..0 and then 31..0. The value comes
        // from an independent model of `hd_hil_s_from_xy` with that masking.
        assert_eq!(hilbert(3, 5, 40), 2_863_311_518);
        assert_eq!(hilbert(3, 5, 8), 52);
    }

    #[test]
    fn xlhorder_matches_c_on_arm64() {
        let bb = |x: f64| BoundingBox {
            ll: Point::new(0.0, 0.0),
            ur: Point::new(x, 0.0),
        };
        assert_eq!(xlhorder(bb(100.0)), 7);
        assert_eq!(xlhorder(bb(0.0)), 1);
        assert_eq!(xlhorder(bb(f64::NAN)), 1);
        // #191: `(unsigned)floor(log2(inf))` saturates to UINT_MAX and `+ 1` wraps.
        assert_eq!(xlhorder(bb(f64::INFINITY)), 0);
    }

    #[test]
    fn infinite_object_places_no_label() {
        // #191: `log2(inf)` used to overflow the order computation. The
        // object at infinity has a NaN centre, where C's `xlhdxload` asserts;
        // the port places no label instead.
        let objs = [
            Object {
                pos: Point::new(0.0, 0.0),
                sz: Point::new(10.0, 10.0),
                lbl: Some(0),
            },
            Object {
                pos: Point::new(f64::INFINITY, 0.0),
                sz: Point::new(10.0, 10.0),
                lbl: None,
            },
        ];
        let mut lbls = [XLabel {
            sz: Point::new(4.0, 4.0),
            pos: Point::default(),
            set: false,
        }];
        let bb = BoundingBox {
            ll: Point::new(0.0, 0.0),
            ur: Point::new(f64::INFINITY, 10.0),
        };
        place_labels(&objs, &mut lbls, bb, true);
        assert!(!lbls[0].set);
    }

    #[test]
    fn infinite_label_places_no_label() {
        // #301: an infinite label size used to keep `adjust` sliding at
        // `-inf >= -inf` forever; C asserts on the centre in `xlhdxload`.
        let objs = [Object {
            pos: Point::new(0.0, 0.0),
            sz: Point::default(),
            lbl: Some(0),
        }];
        let mut lbls = [XLabel {
            sz: Point::new(f64::INFINITY, f64::INFINITY),
            pos: Point::default(),
            set: false,
        }];
        let bb = BoundingBox {
            ll: Point::new(0.0, 0.0),
            ur: Point::new(10.0, 10.0),
        };
        place_labels(&objs, &mut lbls, bb, true);
        assert!(!lbls[0].set);
    }
}
