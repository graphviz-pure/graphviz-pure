//! The shape-specific cases of `round_corners` in `lib/common/shapes.c` (note, tab, folder,
//! box3d, component and the SBOL shapes).

use super::job::{Emitter, Engine};
use super::shapes::CornerShape;
use graphviz_pure_core::Point;
use graphviz_pure_core::math;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn mid_x(line: &[Point]) -> f64 {
    (line[0].x + line[1].x) / 2.0
}

fn mid_y(line: &[Point]) -> f64 {
    (line[0].y + line[1].y) / 2.0
}

pub(crate) fn draw_special<E: Engine>(
    em: &mut Emitter<'_, E>,
    af: &[Point],
    b: &[Point],
    shape: CornerShape,
    filled: i32,
) {
    use CornerShape as S;
    let sides = af.len();
    // common measures
    let bw = || b[2].x - b[3].x;
    let bh = || b[3].y - b[4].y;
    // the dsDNA center line
    let dna = |em: &mut Emitter<'_, E>, x0: f64, x1: f64| {
        let c0 = p(x0, mid_y(&af[1..]));
        let c1 = p(x1, af[2].y + (af[0].y - af[3].y) / 2.0);
        em.polyline(&[c0, c1]);
    };
    match shape {
        S::Dogear => {
            let mut d = vec![Point::default(); sides + 1];
            d[1..sides].copy_from_slice(&af[1..sides]);
            d[0] = b[3 * (sides - 1) + 4];
            d[sides] = b[3 * (sides - 1) + 2];
            em.polygon(&d, filled);
            let sseg = sides - 1;
            let c0 = b[3 * sseg + 2];
            let c1 = b[3 * sseg + 4];
            let c2 = p(
                c1.x + (c0.x - b[3 * sseg + 3].x),
                c1.y + (c0.y - b[3 * sseg + 3].y),
            );
            em.polyline(&[c1, c2]);
            em.polyline(&[c0, c2]);
        }
        S::Tab => {
            let mut d = vec![Point::default(); sides + 2];
            d[0] = af[0];
            d[1] = b[2];
            d[2] = p(
                b[2].x + (b[3].x - b[4].x) / 3.0,
                b[2].y + (b[3].y - b[4].y) / 3.0,
            );
            d[3] = p(
                b[3].x + (b[3].x - b[4].x) / 3.0,
                b[3].y + (b[3].y - b[4].y) / 3.0,
            );
            d[4..sides + 2].copy_from_slice(&af[2..sides]);
            em.polygon(&d, filled);
            em.polyline(&[b[3], b[2]]);
        }
        S::Folder => {
            let mut d = vec![Point::default(); sides + 3];
            d[0] = af[0];
            d[1] = p(
                af[0].x - (af[0].x - b[1].x) / 4.0,
                af[0].y + (b[3].y - b[4].y) / 3.0,
            );
            d[2] = p(af[0].x - 2.0 * (af[0].x - b[1].x), d[1].y);
            // C (clang) fuses `AF[0].x - 2.25 * (AF[0].x - B[1].x)` (#96).
            d[3] = p(math::fma(af[0].x - b[1].x, -2.25, af[0].x), b[3].y);
            d[4] = p(b[3].x, b[3].y);
            d[4..sides + 3].copy_from_slice(&af[1..sides]);
            em.polygon(&d, filled);
        }
        S::Box3d => {
            let d = [af[0], b[2], b[4], af[2], b[8], b[10]];
            em.polygon(&d, filled);
            let c0 = p(b[1].x + (b[11].x - b[0].x), b[1].y + (b[11].y - b[0].y));
            em.polyline(&[c0, b[4]]);
            em.polyline(&[c0, b[8]]);
            em.polyline(&[c0, b[0]]);
        }
        S::Component => {
            let mut d = [Point::default(); 12];
            d[0] = af[0];
            d[1] = af[1];
            d[2] = p(b[3].x + (b[4].x - b[3].x), b[3].y + (b[4].y - b[3].y));
            d[3] = p(d[2].x + (b[3].x - b[2].x), d[2].y + (b[3].y - b[2].y));
            d[4] = p(d[3].x + (b[4].x - b[3].x), d[3].y + (b[4].y - b[3].y));
            d[5] = p(d[4].x + (d[2].x - d[3].x), d[4].y + (d[2].y - d[3].y));
            d[9] = p(b[6].x + (b[5].x - b[6].x), b[6].y + (b[5].y - b[6].y));
            d[8] = p(d[9].x + (b[6].x - b[7].x), d[9].y + (b[6].y - b[7].y));
            d[7] = p(d[8].x + (b[5].x - b[6].x), d[8].y + (b[5].y - b[6].y));
            d[6] = p(d[7].x + (d[9].x - d[8].x), d[7].y + (d[9].y - d[8].y));
            d[10] = af[2];
            d[11] = af[3];
            em.polygon(&d, filled);
            let c1 = p(d[2].x - (d[3].x - d[2].x), d[2].y - (d[3].y - d[2].y));
            let c2 = p(c1.x + (d[4].x - d[3].x), c1.y + (d[4].y - d[3].y));
            em.polyline(&[d[2], c1, c2, d[5]]);
            let c1 = p(d[6].x - (d[7].x - d[6].x), d[6].y - (d[7].y - d[6].y));
            let c2 = p(c1.x + (d[8].x - d[7].x), c1.y + (d[8].y - d[7].y));
            em.polyline(&[d[6], c1, c2, d[9]]);
        }
        S::Promoter => {
            let mut d = [Point::default(); 9];
            d[0] = p(
                mid_x(af) + (af[0].x - af[1].x) / 8.0,
                mid_y(&af[1..]) + bh() * 3.0 / 2.0,
            );
            d[1] = p(mid_x(af) - (af[0].x - af[1].x) / 4.0, d[0].y);
            d[2] = p(d[1].x, mid_y(&af[1..]));
            d[3] = p(d[2].x + bw() / 2.0, mid_y(&af[1..]));
            d[4] = p(d[3].x, mid_y(&af[1..]) + bh());
            d[5] = p(d[0].x, d[4].y);
            d[6] = p(d[0].x, d[4].y - bh() / 4.0);
            d[7] = p(d[6].x + bw(), d[6].y + bh() / 2.0);
            d[8] = p(d[0].x, d[0].y + bh() / 4.0);
            em.polygon(&d, filled);
            dna(em, af[1].x, af[0].x);
        }
        S::Cds => {
            let d = [
                p(b[1].x, b[1].y - bh() / 2.0),
                p(b[3].x, b[3].y - bh() / 2.0),
                p(af[2].x, af[2].y + bh() / 2.0),
                p(b[1].x, af[2].y + bh() / 2.0),
                p(af[0].x, af[0].y - (af[0].y - af[3].y) / 2.0),
            ];
            em.polygon(&d, filled);
        }
        S::Terminator => {
            let mut d = [Point::default(); 8];
            d[0] = p(mid_x(af) + bw() / 4.0, mid_y(&af[1..]));
            d[1] = p(d[0].x, d[0].y + bh() / 2.0);
            d[2] = p(d[1].x + bw() / 2.0, d[1].y);
            d[3] = p(d[2].x, d[2].y + bh() / 2.0);
            d[4] = p(mid_x(af) - bw() * 3.0 / 4.0, d[3].y);
            d[5] = p(d[4].x, d[2].y);
            d[6] = p(mid_x(af) - bw() / 4.0, d[1].y);
            d[7] = p(d[6].x, d[0].y);
            em.polygon(&d, filled);
            dna(em, af[1].x, af[0].x);
        }
        S::Utr => {
            let mut d = [Point::default(); 6];
            d[0] = p(mid_x(af) + bw() * 3.0 / 4.0, mid_y(&af[1..]));
            d[1] = p(d[0].x, d[0].y + bh() / 4.0);
            d[2] = p(mid_x(af) + bw() / 4.0, d[1].y + bh() / 2.0);
            d[3] = p(mid_x(af) - bw() / 4.0, d[2].y);
            d[4] = p(mid_x(af) - bw() * 3.0 / 4.0, d[1].y);
            d[5] = p(d[4].x, d[0].y);
            em.polygon(&d, filled);
            dna(em, af[1].x, af[0].x);
        }
        S::Primersite => {
            let mut d = [Point::default(); 5];
            d[0] = p(mid_x(af) + bw(), mid_y(&af[1..]) + bh() / 4.0);
            d[1] = p(d[0].x - bw(), d[0].y + bh());
            d[2] = p(d[1].x, d[0].y + bh() / 2.0);
            d[3] = p(mid_x(af) - (af[0].x - af[1].x) / 4.0, d[2].y);
            d[4] = p(d[3].x, d[0].y);
            em.polygon(&d, filled);
            dna(em, af[1].x, af[0].x);
        }
        S::Restrictionsite => {
            let mut d = [Point::default(); 8];
            d[0] = p(
                mid_x(af) + (af[0].x - af[1].x) / 8.0 + bw() / 2.0,
                mid_y(&af[1..]) + bh() / 4.0,
            );
            d[1] = p(mid_x(af) - (af[0].x - af[1].x) / 8.0, d[0].y);
            d[2] = p(d[1].x, d[1].y + bh() / 2.0);
            d[3] = p(d[2].x - bw() / 2.0, d[2].y);
            d[4] = p(d[3].x, mid_y(&af[1..]) - bh() / 4.0);
            d[5] = p(d[0].x - bw() / 2.0, d[4].y);
            d[6] = p(d[5].x, d[5].y - bh() / 2.0);
            d[7] = p(d[0].x, d[6].y);
            em.polygon(&d, filled);
            dna(em, af[1].x, d[4].x);
            dna(em, d[7].x, af[0].x);
        }
        S::Fivepoverhang => {
            let mut d = [Point::default(); 4];
            d[0] = p(af[1].x, mid_y(&af[1..]) + bh() / 8.0);
            d[1] = p(d[0].x + 2.0 * bw(), d[0].y);
            d[2] = p(d[1].x, d[1].y + bh() / 2.0);
            d[3] = p(d[0].x, d[2].y);
            em.polygon(&d, filled);
            let mut d = [Point::default(); 4];
            d[0] = p(af[1].x + bw(), mid_y(&af[1..]) - bh() * 5.0 / 8.0);
            d[1] = p(d[0].x + bw(), d[0].y);
            d[2] = p(d[1].x, d[1].y + bh() / 2.0);
            d[3] = p(d[0].x, d[2].y);
            em.polygon(&d, filled);
            dna(em, d[1].x, af[0].x);
        }
        S::Threepoverhang => {
            let mut d = [Point::default(); 4];
            d[0] = p(af[0].x, mid_y(&af[1..]) + bh() / 8.0);
            d[1] = p(d[0].x, d[0].y + bh() / 2.0);
            d[2] = p(d[1].x - 2.0 * bh(), d[1].y);
            d[3] = p(d[2].x, d[0].y);
            em.polygon(&d, filled);
            let mut d = [Point::default(); 4];
            d[0] = p(af[0].x - bw(), mid_y(&af[1..]) - bh() * 5.0 / 8.0);
            d[1] = p(d[0].x, d[0].y + bh() / 2.0);
            d[2] = p(d[1].x - bh(), d[1].y);
            d[3] = p(d[2].x, d[0].y);
            em.polygon(&d, filled);
            dna(em, af[1].x, d[3].x);
        }
        S::Noverhang => {
            let rect = |x: f64, y: f64| {
                let d0 = p(x, y);
                let d1 = p(d0.x + bw(), d0.y);
                let d2 = p(d1.x, d1.y + bh() / 2.0);
                let d3 = p(d0.x, d2.y);
                [d0, d1, d2, d3]
            };
            em.polygon(
                &rect(mid_x(af) - bw() * 9.0 / 8.0, mid_y(&af[1..]) + bh() / 8.0),
                filled,
            );
            em.polygon(
                &rect(
                    mid_x(af) - bw() * 9.0 / 8.0,
                    mid_y(&af[1..]) - bh() * 5.0 / 8.0,
                ),
                filled,
            );
            em.polygon(
                &rect(mid_x(af) + bw() / 8.0, mid_y(&af[1..]) - bh() * 5.0 / 8.0),
                filled,
            );
            let d = rect(mid_x(af) + bw() / 8.0, mid_y(&af[1..]) + bh() / 8.0);
            em.polygon(&d, filled);
            dna(em, d[1].x, af[0].x);
            dna(em, mid_x(af) - bw() * 9.0 / 8.0, af[1].x);
        }
        S::Assembly => {
            let rect = |y: f64| {
                let d0 = p(mid_x(af) - bw(), y);
                let d1 = p(d0.x + 2.0 * bw(), d0.y);
                let d2 = p(d1.x, d1.y + bh() / 2.0);
                let d3 = p(d0.x, d2.y);
                [d0, d1, d2, d3]
            };
            em.polygon(&rect(mid_y(&af[1..]) + bh() / 8.0), filled);
            let d = rect(mid_y(&af[1..]) - bh() * 5.0 / 8.0);
            em.polygon(&d, filled);
            dna(em, d[1].x, af[0].x);
            dna(em, af[1].x, d[0].x);
        }
        S::Signature => {
            let d = [
                p(af[0].x, b[1].y - bh() / 2.0),
                p(b[3].x, b[3].y - bh() / 2.0),
                p(af[2].x, af[2].y + bh() / 2.0),
                p(af[0].x, af[2].y + bh() / 2.0),
            ];
            em.polygon(&d, filled);
            let c0 = p(af[1].x + bw() / 4.0, mid_y(&af[1..]) + bh() / 8.0);
            let c1 = p(c0.x + bw() / 4.0, c0.y - bh() / 4.0);
            em.polyline(&[c0, c1]);
            let c0 = p(af[1].x + bw() / 4.0, mid_y(&af[1..]) - bh() / 8.0);
            let c1 = p(c0.x + bw() / 4.0, c0.y + bh() / 4.0);
            em.polyline(&[c0, c1]);
            let c0 = p(af[1].x + bw() / 4.0, af[2].y + bh() * 3.0 / 4.0);
            let c1 = p(af[0].x - bw() / 4.0, c0.y);
            em.polyline(&[c0, c1]);
        }
        S::Insulator => {
            let d = [
                p(mid_x(af) + bw() / 2.0, mid_y(&af[1..]) + bw() / 2.0),
                p(mid_x(af) + bw() / 2.0, mid_y(&af[1..]) - bw() / 2.0),
                p(mid_x(af) - bw() / 2.0, mid_y(&af[1..]) - bw() / 2.0),
                p(mid_x(af) - bw() / 2.0, mid_y(&af[1..]) + bw() / 2.0),
            ];
            em.polygon(&d, filled);
            let c0 = p(
                mid_x(af) + bw() * 3.0 / 4.0,
                mid_y(&af[1..]) + bw() * 3.0 / 4.0,
            );
            let c1 = p(c0.x, mid_y(&af[1..]) - bw() * 3.0 / 4.0);
            let c2 = p(mid_x(af) - bw() * 3.0 / 4.0, c1.y);
            let c3 = p(c2.x, c0.y);
            em.polyline(&[c0, c1, c2, c3, c0]);
            dna(em, mid_x(af) + bw() * 3.0 / 4.0, af[0].x);
            dna(em, af[1].x, mid_x(af) - bw() * 3.0 / 4.0);
        }
        S::Ribosite | S::Proteasesite => {
            let mut d = [Point::default(); 16];
            d[0] = p(mid_x(af) + bw() / 4.0, mid_y(&af[1..]) + bh() / 2.0);
            d[1] = p(d[0].x, d[0].y + bh() / 8.0);
            d[2] = p(d[0].x - bw() / 8.0, d[1].y + bh() / 8.0);
            d[3] = p(d[0].x, d[2].y + bh() / 8.0);
            d[4] = p(d[0].x, d[3].y + bh() / 8.0);
            d[5] = p(d[2].x, d[4].y);
            d[6] = p(mid_x(af), d[3].y);
            d[7] = p(d[6].x - bw() / 8.0, d[5].y);
            d[8] = p(d[7].x - bw() / 8.0, d[7].y);
            d[9] = p(d[8].x, d[3].y);
            d[10] = p(d[8].x + bw() / 8.0, d[2].y);
            d[11] = p(d[8].x, d[1].y);
            d[12] = p(d[8].x, d[0].y);
            d[13] = p(d[10].x, d[12].y);
            d[14] = p(d[6].x, d[1].y);
            d[15] = p(d[2].x, d[0].y);
            em.polygon(&d, filled);
            if shape == S::Ribosite {
                let c0 = p(d[14].x, mid_y(&af[1..]));
                em.polyline(&[c0, p(c0.x, c0.y + bh() / 8.0)]);
                let c0 = p(d[14].x, mid_y(&af[1..]) + bh() / 4.0);
                em.polyline(&[c0, p(c0.x, c0.y + bh() / 8.0)]);
            } else {
                em.polyline(&[d[14], p(d[14].x, mid_y(&af[1..]))]);
            }
            dna(em, af[1].x, af[0].x);
        }
        S::Rnastab | S::Proteinstab => {
            let mut d = [Point::default(); 8];
            d[0] = p(mid_x(af) + bw() / 8.0, mid_y(&af[1..]) + bh() / 2.0);
            d[1] = p(d[0].x + bw() / 8.0, d[0].y + bh() / 8.0);
            d[2] = p(d[1].x, d[1].y + bh() / 4.0);
            d[3] = p(d[0].x, d[2].y + bh() / 8.0);
            d[4] = p(d[3].x - bw() / 4.0, d[3].y);
            d[5] = p(d[4].x - bw() / 8.0, d[2].y);
            d[6] = p(d[5].x, d[1].y);
            d[7] = p(d[4].x, d[0].y);
            em.polygon(&d, filled);
            if shape == S::Rnastab {
                let c0 = p(mid_x(af), mid_y(&af[1..]));
                em.polyline(&[c0, p(c0.x, c0.y + bh() / 8.0)]);
                let c0 = p(mid_x(af), mid_y(&af[1..]) + bh() / 4.0);
                em.polyline(&[c0, p(c0.x, c0.y + bh() / 8.0)]);
            } else {
                em.polyline(&[p(mid_x(af), d[0].y), p(mid_x(af), mid_y(&af[1..]))]);
            }
            dna(em, af[1].x, af[0].x);
        }
        S::Rpromoter => {
            let bx = |k: usize| b[k].x;
            let d = [
                p(bx(1) - bw() / 2.0, b[1].y - bh() / 2.0),
                p(bx(3), b[3].y - bh() / 2.0),
                p(af[2].x, af[2].y),
                p(bx(2) + bw() / 2.0, af[2].y),
                p(bx(2) + bw() / 2.0, af[2].y + bh() / 2.0),
                p(bx(1) - bw() / 2.0, af[2].y + bh() / 2.0),
                p(bx(1) - bw() / 2.0, af[3].y),
                p(af[0].x, af[0].y - (af[0].y - af[3].y) / 2.0),
                p(bx(1) - bw() / 2.0, af[0].y),
            ];
            em.polygon(&d, filled);
        }
        S::Rarrow => {
            let d = [
                p(b[1].x - bw() / 2.0, b[1].y - bh() / 2.0),
                p(b[3].x, b[3].y - bh() / 2.0),
                p(af[2].x, af[2].y + bh() / 2.0),
                p(b[1].x - bw() / 2.0, af[2].y + bh() / 2.0),
                p(b[1].x - bw() / 2.0, af[3].y),
                p(af[0].x, af[0].y - (af[0].y - af[3].y) / 2.0),
                p(b[1].x - bw() / 2.0, af[0].y),
            ];
            em.polygon(&d, filled);
        }
        S::Larrow => {
            let x = b[2].x + bw() / 2.0;
            let d = [
                p(af[0].x, af[0].y - bh() / 2.0),
                p(x, af[0].y - bh() / 2.0),
                p(x, b[2].y),
                p(af[1].x, af[1].y - (af[1].y - af[2].y) / 2.0),
                p(x, af[2].y),
                p(x, af[2].y + bh() / 2.0),
                p(af[0].x, af[3].y + bh() / 2.0),
            ];
            em.polygon(&d, filled);
        }
        S::Lpromoter => {
            let x = b[2].x + bw() / 2.0;
            let d = [
                p(af[0].x, af[0].y - bh() / 2.0),
                p(x, af[0].y - bh() / 2.0),
                p(x, b[2].y),
                p(af[1].x, af[1].y - (af[1].y - af[2].y) / 2.0),
                p(x, af[2].y),
                p(x, af[2].y + bh() / 2.0),
                p(b[1].x - bw() / 2.0, af[3].y + bh() / 2.0),
                p(b[1].x - bw() / 2.0, af[3].y),
                p(af[3].x, af[3].y),
            ];
            em.polygon(&d, filled);
        }
        S::None | S::Cylinder => {}
    }
}
