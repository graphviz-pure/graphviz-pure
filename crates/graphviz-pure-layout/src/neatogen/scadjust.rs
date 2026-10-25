//! Scaling and constraint-based overlap removal from `lib/neatogen/constraint.c`
//! (`scAdjust`, `cAdjust`) and `adjust.c` (`vpscAdjust`).

use crate::common::types::{LGraph, Sub};
use graphviz_pure_core::{BoundingBox, Point};

struct Info {
    pos: Point,
    bb: BoundingBox,
    wd2: f64,
    ht2: f64,
    np: usize,
}

fn overlap(b0: &BoundingBox, b1: &BoundingBox) -> bool {
    b0.ur.x >= b1.ll.x && b1.ur.x >= b0.ll.x && b0.ur.y >= b1.ll.y && b1.ur.y >= b0.ll.y
}

fn compress(nl: &[Info]) -> f64 {
    let mut sc = 0.0f64;
    for i in 0..nl.len() {
        for j in (i + 1)..nl.len() {
            let (p, q) = (&nl[i], &nl[j]);
            if overlap(&p.bb, &q.bb) {
                return 0.0;
            }
            let px = if p.pos.x == q.pos.x {
                f64::INFINITY
            } else {
                (p.wd2 + q.wd2) / (p.pos.x - q.pos.x).abs()
            };
            let py = if p.pos.y == q.pos.y {
                f64::INFINITY
            } else {
                (p.ht2 + q.ht2) / (p.pos.y - q.pos.y).abs()
            };
            let s = if py < px { py } else { px };
            if s > sc {
                sc = s;
            }
        }
    }
    sc
}

fn mk_overlap_set(nl: &[Info]) -> Vec<Point> {
    let mut s = vec![Point::new(0.0, 0.0)];
    for i in 0..nl.len() {
        for j in (i + 1)..nl.len() {
            let (p, q) = (&nl[i], &nl[j]);
            if overlap(&p.bb, &q.bb) {
                let x = if p.pos.x == q.pos.x {
                    f64::INFINITY
                } else {
                    ((p.wd2 + q.wd2) / (p.pos.x - q.pos.x).abs()).max(1.0)
                };
                let y = if p.pos.y == q.pos.y {
                    f64::INFINITY
                } else {
                    ((p.ht2 + q.ht2) / (p.pos.y - q.pos.y).abs()).max(1.0)
                };
                s.push(Point::new(x, y));
            }
        }
    }
    s
}

/// `computeScaleXY`. `None` where C asserts (`bestcost < HUGE_VAL` fails).
fn compute_scale_xy(aarr: &mut [Point]) -> Option<Point> {
    let m = aarr.len();
    aarr[0] = Point::new(1.0, f64::INFINITY);
    crate::cutil::qsort_by(&mut aarr[1..], |p, q| {
        if p.x < q.x {
            -1
        } else if p.x > q.x {
            1
        } else if p.y < q.y {
            -1
        } else if p.y > q.y {
            1
        } else {
            0
        }
    });
    let mut barr = vec![Point::default(); m];
    barr[m - 1] = Point::new(aarr[m - 1].x, 1.0);
    let mut k = m - 1;
    while k > 0 {
        k -= 1;
        barr[k] = Point::new(aarr[k].x, aarr[k + 1].y.max(barr[k + 1].y));
    }
    let mut best = 0;
    let mut bestcost = f64::INFINITY;
    for (k, b) in barr.iter().enumerate() {
        let cost = b.x * b.y;
        if cost < bestcost {
            bestcost = cost;
            best = k;
        }
    }
    // C asserts `bestcost < HUGE_VAL` (`constraint.c:729`, `computeScaleXY`) and
    // aborts. Overlapping nodes at the same position, or so close that the
    // required scale overflows, make every candidate non-finite: scaling about
    // the origin cannot separate them. There is no output to match. NaN fails
    // the comparison too, as it fails C's assert.
    if bestcost < f64::INFINITY {
        Some(barr[best])
    } else {
        None
    }
}

fn compute_scale(aarr: &[Point]) -> f64 {
    let mut sc = 0.0f64;
    for p in &aarr[1..] {
        let v = p.x.min(p.y);
        if v > sc {
            sc = v;
        }
    }
    sc
}

/// `scAdjust(g, equal)`: scale uniformly (`equal > 0`), per axis (`0`) or
/// compress (`< 0`) to remove overlaps.
pub fn sc_adjust(g: &mut LGraph<'_>, sub: &Sub, equal: i32) -> bool {
    let mut margin = super::adjust::sep_factor(g);
    if margin.do_add {
        margin.x /= 72.0;
        margin.y /= 72.0;
    }
    let mut nl = Vec::with_capacity(sub.nodes.len());
    for &n in &sub.nodes {
        let nd = &g.nodes[n];
        let (w2, h2) = if margin.do_add {
            (nd.width / 2.0 + margin.x, nd.height / 2.0 + margin.y)
        } else {
            (margin.x * nd.width / 2.0, margin.y * nd.height / 2.0)
        };
        let pos = Point::new(nd.pos[0], nd.pos[1]);
        nl.push(Info {
            pos,
            bb: BoundingBox::new(pos.x - w2, pos.y - h2, pos.x + w2, pos.y + h2),
            wd2: w2,
            ht2: h2,
            np: n,
        });
    }
    let s = if equal < 0 {
        let c = compress(&nl);
        if c == 0.0 {
            return false;
        }
        graphviz_pure_core::verbose_msg!("compress {} \n", graphviz_pure_core::diag::fmt_g(c, 6));
        Point::new(c, c)
    } else {
        let mut aarr = mk_overlap_set(&nl);
        if aarr.len() == 1 {
            return false;
        }
        let s = if equal != 0 {
            let c = compute_scale(&aarr);
            Point::new(c, c)
        } else {
            // `scAdjust` returns 0 when it cannot help (no overlapping pair, or
            // `compress` finds one). C crashes in `computeScaleXY` instead; the
            // port keeps the positions the layout already computed.
            let Some(s) = compute_scale_xy(&mut aarr) else {
                graphviz_pure_core::diag::agwarningf(
                    "overlap=scalexy has no finite scale; scaling skipped\n",
                );
                return false;
            };
            s
        };
        graphviz_pure_core::verbose_msg!(
            "scale by {},{} \n",
            graphviz_pure_core::diag::fmt_g(s.x, 6),
            graphviz_pure_core::diag::fmt_g(s.y, 6)
        );
        s
    };
    for p in &nl {
        g.nodes[p.np].pos[0] = s.x * p.pos.x;
        g.nodes[p.np].pos[1] = s.y * p.pos.y;
    }
    true
}

/// `vpscAdjust(G)`: `removeoverlaps` on single-precision coordinates; C
/// returns 0 even when nodes move.
pub fn vpsc_adjust(g: &mut LGraph<'_>, sub: &Sub) -> bool {
    use super::adjust::{DFLT_MARGIN, sep_factor};
    use super::quad_prog_vpsc::{IpsepOptions, removeoverlaps};
    let nnodes = sub.nodes.len();
    let mut coords = vec![vec![0.0f32; nnodes]; 2];
    let mut nsize = Vec::with_capacity(nnodes);
    for (j, &v) in sub.nodes.iter().enumerate() {
        for (i, c) in coords.iter_mut().enumerate() {
            c[j] = g.nodes[v].pos[i] as f32;
        }
        nsize.push((g.nodes[v].width, g.nodes[v].height));
    }
    let exp_margin = sep_factor(g);
    // Multiply by 2 since opt.gap is the gap size, not the margin
    let gap = if exp_margin.do_add {
        (2.0 * (exp_margin.x / 72.0), 2.0 * (exp_margin.y / 72.0))
    } else {
        (2.0 * (DFLT_MARGIN / 72.0), 2.0 * (DFLT_MARGIN / 72.0))
    };
    let mut opt = IpsepOptions {
        diredges: 0,
        edge_gap: 0.0,
        noverlap: 2,
        gap,
        nsize,
        ..Default::default()
    };
    if !removeoverlaps(nnodes, &mut coords, &mut opt) {
        // C aborts in `Rectangle` here; the port leaves the layout's positions.
        graphviz_pure_core::diag::agwarningf(
            "node positions are not finite; overlap removal skipped\n",
        );
        return false;
    }
    for (j, &v) in sub.nodes.iter().enumerate() {
        for (i, c) in coords.iter().enumerate() {
            g.nodes[v].pos[i] = c[j] as f64;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue #434: `computeScaleXY` (constraint.c:729) asserts when every
    // candidate cost is non-finite. A pair at one position is `(inf, inf)`.
    #[test]
    fn compute_scale_xy_skips_a_non_finite_scale() {
        let mut coincident = vec![
            Point::new(0.0, 0.0),
            Point::new(f64::INFINITY, f64::INFINITY),
        ];
        assert!(compute_scale_xy(&mut coincident).is_none());

        // One coincident pair makes the whole set infeasible, as C's assert does.
        let mut mixed = vec![
            Point::new(0.0, 0.0),
            Point::new(4.0, 3.0),
            Point::new(f64::INFINITY, f64::INFINITY),
        ];
        assert!(compute_scale_xy(&mut mixed).is_none());

        // Same x only: C scales in y (`barr[0] = (1, y)`).
        let mut same_x = vec![Point::new(0.0, 0.0), Point::new(f64::INFINITY, 2.5)];
        assert_eq!(compute_scale_xy(&mut same_x), Some(Point::new(1.0, 2.5)));
        // Same y only: C scales in x.
        let mut same_y = vec![Point::new(0.0, 0.0), Point::new(5.4, f64::INFINITY)];
        assert_eq!(compute_scale_xy(&mut same_y), Some(Point::new(5.4, 1.0)));
    }
}
