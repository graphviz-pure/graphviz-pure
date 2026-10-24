//! Squarified treemap, ported from `lib/patchwork/tree_map.c`.

use graphviz_pure_core::diag::{fmt_f, fprintf_stderr, verbose_on};

/// `rectangle`: centre `x` and full `size`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rectangle {
    pub x: [f64; 2],
    pub size: [f64; 2],
}

/// `squarify(n, area, recs, nadded, maxarea, minarea, totalarea, asp, fillrec)`.
///
/// C recurses once per added item; the recursion is a tail call in every
/// branch, so this is the same state machine written as a loop.
fn squarify(area: &[f64], recs: &mut [Rectangle], mut fillrec: Rectangle) {
    let mut n = area.len();
    let mut off = 0usize;
    let mut nadded = 0usize;
    let mut maxarea = 0.0f64;
    let mut minarea = 0.0f64;
    let mut totalarea = 0.0f64;
    let mut asp = 1.0f64;
    loop {
        let w = fillrec.size[0].min(fillrec.size[1]);
        if n == 0 {
            return;
        }
        if verbose_on() {
            fprintf_stderr(&format!(
                "trying to add to rect {{{} +/- {}, {} +/- {}}}\ntotal added so far = {nadded}\n",
                fmt_f(fillrec.x[0], 6),
                fmt_f(fillrec.size[0], 6),
                fmt_f(fillrec.x[1], 6),
                fmt_f(fillrec.size[1], 6)
            ));
        }
        let area = &area[off..];
        if nadded == 0 {
            nadded = 1;
            maxarea = area[0];
            minarea = area[0];
            asp = (area[0] / (w * w)).max(w * w / area[0]);
            totalarea = area[0];
            continue;
        }
        let (mut newmaxarea, mut newminarea, mut s, mut newasp) = (0.0, 0.0, 0.0, 0.0);
        if nadded < n {
            newmaxarea = maxarea.max(area[nadded]);
            newminarea = minarea.min(area[nadded]);
            s = totalarea + area[nadded];
            let h = s / w;
            let maxw = newmaxarea / h;
            let minw = newminarea / h;
            newasp = (h / minw).max(maxw / h);
        }
        if nadded < n && newasp <= asp {
            nadded += 1;
            maxarea = newmaxarea;
            minarea = newminarea;
            totalarea = s;
            asp = newasp;
            continue;
        }
        let recs = &mut recs[off..];
        graphviz_pure_core::verbose_msg!(
            "adding {nadded} items, total area = {}, w = {}, area/w={}\n",
            fmt_f(totalarea, 6),
            fmt_f(w, 6),
            fmt_f(totalarea / w, 6)
        );
        if fillrec.size[0] <= fillrec.size[1] {
            // Tall rectangle: lay the items along x, left to right, at the top.
            let hh = totalarea / w;
            let mut xx = fillrec.x[0] - fillrec.size[0] / 2.0;
            for i in 0..nadded {
                recs[i].size[1] = hh;
                let ww = area[i] / hh;
                recs[i].size[0] = ww;
                recs[i].x[1] = fillrec.x[1] + 0.5 * (fillrec.size[1]) - hh / 2.0;
                recs[i].x[0] = xx + ww / 2.0;
                xx += ww;
            }
            fillrec.x[1] -= hh / 2.0;
            fillrec.size[1] -= hh;
        } else {
            // Short rectangle: lay the items along y, top to bottom, at the left.
            let ww = totalarea / w;
            let mut yy = fillrec.x[1] + fillrec.size[1] / 2.0;
            for i in 0..nadded {
                recs[i].size[0] = ww;
                let hh = area[i] / ww;
                recs[i].size[1] = hh;
                recs[i].x[0] = fillrec.x[0] - 0.5 * (fillrec.size[0]) + ww / 2.0;
                recs[i].x[1] = yy - hh / 2.0;
                yy -= hh;
            }
            fillrec.x[0] += ww / 2.0;
            fillrec.size[0] -= ww;
        }
        n -= nadded;
        off += nadded;
        nadded = 0;
        maxarea = 0.0;
        minarea = 0.0;
        totalarea = 0.0;
        asp = 1.0;
    }
}

/// `tree_map(n, area, fillrec)`: fill `fillrec` with rectangles of the given
/// areas; `None` when the areas do not fit.
pub fn tree_map(area: &[f64], fillrec: Rectangle) -> Option<Vec<Rectangle>> {
    let mut total = 0.0;
    for &a in area {
        total += a;
    }
    // C (clang) fuses this multiply-add.
    if total > fillrec.size[0].mul_add(fillrec.size[1], 0.001) {
        return None;
    }
    let mut recs = vec![Rectangle::default(); area.len()];
    squarify(area, &mut recs, fillrec);
    Some(recs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_square_exactly() {
        let r = Rectangle {
            x: [0.0, 0.0],
            size: [2.0, 2.0],
        };
        let recs = tree_map(&[1.0, 1.0, 1.0, 1.0], r).unwrap();
        let total: f64 = recs.iter().map(|r| r.size[0] * r.size[1]).sum();
        assert!((total - 4.0).abs() < 1e-9);
        assert!(tree_map(&[5.0], r).is_none());
    }
}
