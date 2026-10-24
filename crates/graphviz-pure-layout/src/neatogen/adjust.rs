//! Node overlap removal and spacing factors, ported from
//! `lib/neatogen/adjust.c`: `graphAdjustMode`/`getAdjustMode`,
//! `removeOverlapWith`, `normalize`, `simpleScale`, `sepFactor`,
//! `esepFactor`, and the scaling adjusters `scAdjust`.
//!
//! Graphviz 16.1.0 on Homebrew is built with GTS and SFDP, so
//! `overlap=false` selects the prism algorithm (`AM_PRISM`).

use crate::common::types::{LGraph, Sub};
use crate::common::utils::{map_bool, mapbool};
use graphviz_pure_core::diag::fmt_f;
use graphviz_pure_core::math;
use graphviz_pure_core::strtod::{sscanf, strtod_rest};

pub const DFLT_MARGIN: f64 = 4.0;
pub const SEPFACT: f64 = 0.8;

/// `expand_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Expand {
    pub x: f64,
    pub y: f64,
    /// If true, `x` and `y` are additive margins in points; else scale factors.
    pub do_add: bool,
}

/// `adjust_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdjustMode {
    None,
    Vor,
    Scale,
    NScale,
    ScaleXY,
    Ortho,
    OrthoYX,
    OrthoXY,
    OrthoYXrev,
    POrtho,
    POrthoYX,
    POrthoXY,
    POrthoYXrev,
    Compress,
    Vpsc,
    Ipsep,
    Prism,
}

/// `adjust_data`.
#[derive(Debug, Clone, Copy)]
pub struct AdjustData {
    pub mode: AdjustMode,
    pub value: i32,
    pub scaling: f64,
}

/// What `parseFactor`'s uninitialised `x` and `y` hold after sscanf's EOF, as the port models
/// it. In C they are stack residue: pointers or small integers, which as doubles are
/// nonnegative subnormals (or 0), different in each call by a few bytes. Added to a size or
/// to 1 they vanish, and the one comparison of two of them (`compoundEdges`' note) has a
/// factor of 1.25 between its sides, so one fixed positive subnormal gives C's results.
/// Measured on one C build; see `docs/porting/crashes.md`, "parseFactor's uninitialised
/// x, y".
const STALE_POINTER: f64 = f64::from_bits(1 << 32);

/// What `sscanf(s, "%lf,%lf", &x, &y)` assigns.
enum Scan2 {
    /// It returned 0: nothing converted.
    None,
    /// It returned EOF (`-1`: the string is empty or blank), which C tests as true, with `x`
    /// and `y` left uninitialised.
    Eof,
    /// One value, `x`.
    One(f64),
    /// Two values, `x` and `y`.
    Two(f64, f64),
}

fn scan2(s: &str) -> Scan2 {
    let r = sscanf(s, "%lf,%lf");
    let [x, y, ..] = r.values;
    match r.count {
        2 => Scan2::Two(x, y),
        1 => Scan2::One(x),
        -1 => Scan2::Eof,
        _ => Scan2::None,
    }
}

/// `parseFactor(s, pp, sepfact, dflt)`.
fn parse_factor(s: &str, sepfact: f64, dflt: f64) -> Option<Expand> {
    // while (gv_isspace(*s)) s++;
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (do_add, s) = match s.strip_prefix('+') {
        Some(r) => (true, r),
        None => (false, s),
    };
    // `if (i == 1) y = x`; on EOF, C's uninitialised x, y.
    let (x, y) = match scan2(s) {
        Scan2::None => return None,
        Scan2::Eof => (STALE_POINTER, STALE_POINTER),
        Scan2::One(x) => (x, x),
        Scan2::Two(x, y) => (x, y),
    };
    Some(if do_add {
        if sepfact > 1.0 {
            Expand {
                x: dflt.min(x / sepfact),
                y: dflt.min(y / sepfact),
                do_add,
            }
        } else if sepfact < 1.0 {
            Expand {
                x: dflt.max(x / sepfact),
                y: dflt.max(y / sepfact),
                do_add,
            }
        } else {
            Expand { x, y, do_add }
        }
    } else {
        Expand {
            x: 1.0 + x / sepfact,
            y: 1.0 + y / sepfact,
            do_add,
        }
    })
}

/// `sepFactor(g)`: node separation for overlap removal.
pub fn sep_factor(g: &LGraph<'_>) -> Expand {
    let e = g
        .gattr("sep")
        .and_then(|s| parse_factor(s, 1.0, 0.0))
        .or_else(|| {
            g.gattr("esep")
                .and_then(|s| parse_factor(s, SEPFACT, DFLT_MARGIN))
        })
        .unwrap_or(Expand {
            x: DFLT_MARGIN,
            y: DFLT_MARGIN,
            do_add: true,
        });
    verbose_separation("Node", &e);
    e
}

/// sepFactor's and esepFactor's `-v` line.
fn verbose_separation(what: &str, e: &Expand) {
    graphviz_pure_core::verbose_msg!(
        "{what} separation: add={} ({},{})\n",
        i32::from(e.do_add),
        fmt_f(e.x, 6),
        fmt_f(e.y, 6)
    );
}

/// `esepFactor(g)`: edge separation used to grow obstacles for routing.
pub fn esep_factor(g: &LGraph<'_>) -> Expand {
    let e = g
        .gattr("esep")
        .and_then(|s| parse_factor(s, 1.0, 0.0))
        .or_else(|| {
            g.gattr("sep")
                .and_then(|s| parse_factor(s, 1.0 / SEPFACT, SEPFACT * DFLT_MARGIN))
        })
        .unwrap_or(Expand {
            x: SEPFACT * DFLT_MARGIN,
            y: SEPFACT * DFLT_MARGIN,
            do_add: true,
        });
    verbose_separation("Edge", &e);
    e
}

const TABLE: &[(AdjustMode, &str)] = &[
    (AdjustMode::Prism, "prism"),
    (AdjustMode::Vor, "voronoi"),
    (AdjustMode::NScale, "scale"),
    (AdjustMode::Compress, "compress"),
    (AdjustMode::Vpsc, "vpsc"),
    (AdjustMode::Ipsep, "ipsep"),
    (AdjustMode::Scale, "oscale"),
    (AdjustMode::ScaleXY, "scalexy"),
    (AdjustMode::Ortho, "ortho"),
    (AdjustMode::OrthoYX, "ortho_yx"),
    (AdjustMode::OrthoXY, "orthoxy"),
    (AdjustMode::OrthoYXrev, "orthoyx"),
    (AdjustMode::POrtho, "portho"),
    (AdjustMode::POrthoYX, "portho_yx"),
    (AdjustMode::POrthoXY, "porthoxy"),
    (AdjustMode::POrthoYXrev, "porthoyx"),
];

fn set_prism_values(g: &LGraph<'_>, s: &str, dp: &mut AdjustData) {
    // sscanf(s, "%d", &v) > 0 && v >= 0
    dp.value = match crate::common::utils::scan_d(s) {
        Some(v) if v >= 0 => v,
        _ => 1000,
    };
    dp.scaling =
        crate::common::utils::late_double(&g.src.graph_attrs, "overlap_scaling", -4.0, -1.0e10);
}

/// `graphAdjustMode(G, dp, dflt)`.
pub fn graph_adjust_mode(g: &LGraph<'_>, dflt: Option<&str>) -> AdjustData {
    let s = g.gattr("overlap").or(dflt).unwrap_or("");
    get_adjust_mode(g, s)
}

/// `getAdjustMode(g, s, dp)`.
pub fn get_adjust_mode(g: &LGraph<'_>, s: &str) -> AdjustData {
    let dp = adjust_mode(g, s);
    // dp->value is left uninitialised outside prism; the port prints 0
    graphviz_pure_core::verbose_msg!(
        "overlap: {} value {} scaling {}\n",
        mode_print(dp.mode),
        dp.value,
        fmt_f(dp.scaling, 4)
    );
    dp
}

/// `adjustMode[].print`: the name `getAdjustMode` prints for a mode.
fn mode_print(m: AdjustMode) -> &'static str {
    match m {
        AdjustMode::None => "none",
        AdjustMode::Prism => "prism",
        AdjustMode::Vor => "Voronoi",
        AdjustMode::NScale => "scaling",
        AdjustMode::Compress => "compress",
        AdjustMode::Vpsc => "vpsc",
        AdjustMode::Ipsep => "ipsep",
        AdjustMode::Scale => "old scaling",
        AdjustMode::ScaleXY => "x and y scaling",
        AdjustMode::Ortho | AdjustMode::OrthoYX => "orthogonal constraints",
        AdjustMode::OrthoXY => "xy orthogonal constraints",
        AdjustMode::OrthoYXrev => "yx orthogonal constraints",
        AdjustMode::POrtho | AdjustMode::POrthoYX => "pseudo-orthogonal constraints",
        AdjustMode::POrthoXY => "xy pseudo-orthogonal constraints",
        AdjustMode::POrthoYXrev => "yx pseudo-orthogonal constraints",
    }
}

/// The mode [`get_adjust_mode`] picks.
fn adjust_mode(g: &LGraph<'_>, s: &str) -> AdjustData {
    let mut dp = AdjustData {
        mode: AdjustMode::None,
        value: 0,
        scaling: 0.0,
    };
    if s.is_empty() {
        return dp;
    }
    for &(mode, attrib) in TABLE {
        // `strncasecmp` compares bytes: slicing the `str` at `attrib.len()` would panic
        // when that byte falls inside a multi-byte character ("prisé").
        let matches = s.eq_ignore_ascii_case(attrib)
            || (mode == AdjustMode::Prism
                && s.as_bytes()
                    .get(..attrib.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(attrib.as_bytes())));
        if matches {
            dp.mode = mode;
            if mode == AdjustMode::Prism {
                set_prism_values(g, &s[attrib.len()..], &mut dp);
            }
            return dp;
        }
    }
    let mut v = mapbool(Some(s));
    if v != map_bool(Some(s), true) {
        graphviz_pure_core::diag::agwarningf(&format!(
            "Unrecognized overlap value \"{s}\" - using false\n"
        ));
        v = false;
    }
    if v {
        dp.mode = AdjustMode::None;
    } else {
        dp.mode = AdjustMode::Prism;
        set_prism_values(g, "", &mut dp);
    }
    dp
}

/// `angleSet(g, &phi)`.
fn angle_set(g: &LGraph<'_>) -> Option<f64> {
    let a = g.gattr("normalize").filter(|s| !s.is_empty())?;
    let mut ang = match strtod_rest(a) {
        Some((v, _)) => v,
        None => {
            if mapbool(Some(a)) {
                0.0
            } else {
                return None;
            }
        }
    };
    while ang > 180.0 {
        ang -= 360.0;
    }
    while ang <= -180.0 {
        ang += 360.0;
    }
    // RADIANS(ang)
    Some(ang / 180.0 * std::f64::consts::PI)
}

/// `normalize(g)`: translate the first node to the origin and rotate so the
/// first edge has the angle given by `normalize`.
pub fn normalize(g: &mut LGraph<'_>, sub: &Sub) -> bool {
    let Some(mut phi) = angle_set(g) else {
        return false;
    };
    let Some(&v0) = sub.nodes.first() else {
        return false;
    };
    let p = (g.nodes[v0].pos[0], g.nodes[v0].pos[1]);
    for &v in &sub.nodes {
        g.nodes[v].pos[0] -= p.0;
        g.nodes[v].pos[1] -= p.1;
    }
    let ret = p.0 != 0.0 || p.1 != 0.0;
    let mut e = None;
    for &v in &sub.nodes {
        if let Some(x) = g.outs(sub, v).next() {
            e = Some(x);
            break;
        }
    }
    let Some(e) = e else { return ret };
    let (t, h) = (g.edges[e].tail, g.edges[e].head);
    phi -= math::atan2(
        g.nodes[h].pos[1] - g.nodes[t].pos[1],
        g.nodes[h].pos[0] - g.nodes[t].pos[0],
    );
    if phi != 0.0 {
        let orig = (g.nodes[t].pos[0], g.nodes[t].pos[1]);
        let (cosv, sinv) = (math::cos(phi), math::sin(phi));
        for &v in &sub.nodes {
            let px = g.nodes[v].pos[0] - orig.0;
            let py = g.nodes[v].pos[1] - orig.1;
            g.nodes[v].pos[0] = px.mul_add(cosv, -(py * sinv)) + orig.0;
            g.nodes[v].pos[1] = px.mul_add(sinv, py * cosv) + orig.1;
        }
        return true;
    }
    ret
}

/// `simpleScale(g)`: the `scale` attribute.
fn simple_scale(g: &mut LGraph<'_>, sub: &Sub) -> bool {
    let Some(p) = g.gattr("scale") else {
        return false;
    };
    let iszero = |d: f64| d.abs() < 0.000_000_001;
    let (sx, sy) = match scan2(p) {
        // On EOF C reads its uninitialised `sc.x`, stack residue, which `ISZERO` takes as 0
        // (`docs/porting/crashes.md`, "parseFactor's uninitialised x, y").
        Scan2::None | Scan2::Eof => return false,
        Scan2::One(sx) if iszero(sx) => return false,
        Scan2::One(sx) => (sx, sx),
        Scan2::Two(sx, sy) if iszero(sx) || iszero(sy) => return false,
        Scan2::Two(sx, sy) => (sx, sy),
    };
    if sy == 1.0 && sx == 1.0 {
        return false;
    }
    graphviz_pure_core::verbose_msg!("scale = ({},{})\n", fmt_f(sx, 3), fmt_f(sy, 3));
    for &n in &sub.nodes {
        g.nodes[n].pos[0] *= sx;
        g.nodes[n].pos[1] *= sy;
    }
    true
}

/// `removeOverlapWith(G, am)`; returns whether nodes moved.
pub fn remove_overlap_with(g: &mut LGraph<'_>, sub: &Sub, am: &AdjustData) -> bool {
    if sub.nodes.len() < 2 {
        return false;
    }
    let mut nret = normalize(g, sub);
    nret |= simple_scale(g, sub);
    if am.mode == AdjustMode::None {
        return nret;
    }
    graphviz_pure_core::verbose_msg!("Adjusting {} using {}\n", sub.name, mode_print(am.mode));
    let ret = match am.mode {
        AdjustMode::NScale => super::scadjust::sc_adjust(g, sub, 1),
        AdjustMode::ScaleXY => super::scadjust::sc_adjust(g, sub, 0),
        AdjustMode::Compress => super::scadjust::sc_adjust(g, sub, -1),
        AdjustMode::Ortho
        | AdjustMode::OrthoYX
        | AdjustMode::OrthoXY
        | AdjustMode::OrthoYXrev
        | AdjustMode::POrtho
        | AdjustMode::POrthoYX
        | AdjustMode::POrthoXY
        | AdjustMode::POrthoYXrev => {
            super::constraint::c_adjust(g, sub, am.mode);
            false
        }
        AdjustMode::Prism => {
            super::overlap::fdp_adjust(g, sub, am);
            false
        }
        AdjustMode::Ipsep => return nret,
        AdjustMode::Vpsc => super::scadjust::vpsc_adjust(g, sub),
        AdjustMode::Vor | AdjustMode::Scale => super::voronoi::adjust(g, sub, am.mode),
        AdjustMode::None => false,
    };
    nret || ret
}

/// `removeOverlapAs(G, flag)`: remove overlaps using an explicit `overlap` value.
pub fn remove_overlap_as(g: &mut LGraph<'_>, sub: &Sub, flag: Option<&str>) -> bool {
    if sub.nodes.len() < 2 {
        return false;
    }
    let am = get_adjust_mode(g, flag.unwrap_or(""));
    remove_overlap_with(g, sub, &am)
}

/// `adjustNodes(G)`: remove overlaps according to the graph's `overlap` attribute
/// (used by twopi and circo).
pub fn adjust_nodes(g: &mut LGraph<'_>, sub: &Sub) -> bool {
    let flag = g.gattr("overlap");
    remove_overlap_as(g, sub, flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue #372: on a blank `sep`/`esep` C's sscanf returns EOF, which `parseFactor` takes
    // as success with uninitialised `x`, `y`: a stale pointer, a positive subnormal. The
    // values are C 16.1.0's (`dot -v`: `Node separation`, `Edge separation`, which print the
    // subnormal as 0).
    #[test]
    fn parse_factor_reads_a_blank_value_as_a_stale_pointer() {
        let f = |s, sepfact, dflt| parse_factor(s, sepfact, dflt).map(|e| (e.do_add, e.x, e.y));
        let g = STALE_POINTER;
        assert!(g > 0.0 && !g.is_normal());
        for blank in ["", " ", "\t"] {
            assert_eq!(f(blank, 1.0, 0.0), Some((false, 1.0, 1.0)));
            assert_eq!(f(blank, SEPFACT, DFLT_MARGIN), Some((false, 1.0, 1.0)));
        }
        for plus in ["+", " + ", "+\n"] {
            assert_eq!(f(plus, 1.0, 0.0), Some((true, g, g)));
            assert_eq!(f(plus, SEPFACT, DFLT_MARGIN), Some((true, 4.0, 4.0)));
            let e = f(plus, 1.0 / SEPFACT, SEPFACT * DFLT_MARGIN).unwrap();
            assert!(e.0 && e.1 < g && e.1 > 0.0 && e.1 == e.2);
        }
        assert_eq!(f("x", 1.0, 0.0), None);
        assert_eq!(f("+,1", 1.0, 0.0), None);
    }
}
