//! Structural checks of the port's `-Tsvg` output (`gv-diff structure`).
//!
//! `gv-diff e2e` requires the port's SVG to equal C's byte for byte, with C computing with the
//! same math functions. The reference here is stock C on macOS (Apple's libm), where sfdp and a
//! few other layouts take a different but equally valid shape. This mode checks what must hold
//! whatever the layout: that each drawing contains the same nodes, edges, clusters and label
//! text as C's, that nothing is drawn outside the canvas or at a non-finite coordinate, that
//! the canvas is about the size of C's, and that nodes do not overlap much more than in C's
//! drawing. Each check is a [`Check`] in [`CHECKS`]; `docs/testing/structure-checks.md`
//! explains how to add one.
//!
//! The reference is `tests/golden/structure.json`: a [`Summary`] of C's output for every
//! corpus input, written by `gv-diff structure --generate` (needs the pango-free C build, as
//! for `gv-diff e2e --generate`). Inputs that `tests/golden/e2e.json` records as unstable in C
//! are checked for content only, since C's own geometry changes from run to run there.

use crate::e2e;
use graphviz_pure_core::math;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What one SVG document (one graph) contains, reduced to what the checks compare.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    /// Node, edge and cluster titles and label texts: count and a hash of the sorted multiset.
    pub nodes: Set,
    pub edges: Set,
    pub clusters: Set,
    pub texts: Set,
    /// Canvas size (the `viewBox`), in points.
    pub width: f64,
    pub height: f64,
    /// Coordinates that do not parse as finite numbers.
    pub non_finite: usize,
    /// Drawn points more than 1pt outside the canvas.
    pub off_canvas: usize,
    /// Pairs of nodes whose bounding boxes overlap by more than 1pt in both directions.
    pub overlaps: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Set {
    pub count: usize,
    pub hash: String,
}

impl Set {
    fn of(mut items: Vec<String>) -> Self {
        items.sort();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for s in &items {
            for &b in s.as_bytes().iter().chain(std::iter::once(&0u8)) {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        Set {
            count: items.len(),
            hash: format!("{h:016x}"),
        }
    }
}

/// The summary of one input: one [`Doc`] per graph in the file.
pub type Summary = Vec<Doc>;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Golden {
    /// `corpus/stem` (as in `e2e.json`) → summary of C's `-Tsvg`.
    pub files: BTreeMap<String, Summary>,
}

/// Whether C's geometry for this input is stable enough to compare against.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Geometry {
    Stable,
    Unstable,
}

/// One structural check: compares the port's document with C's and returns a problem, if any.
pub struct Check {
    pub name: &'static str,
    /// Only meaningful when C's geometry is stable (sizes, positions).
    pub geometric: bool,
    pub run: fn(port: &Doc, c: &Doc) -> Option<String>,
}

/// Canvas area, port over C, outside which the drawing is reported.
pub const AREA_RANGE: (f64, f64) = (2.0 / 3.0, 1.5);

/// Overlapping node pairs the port may have beyond C's: a share of C's count plus a constant,
/// since a different layout of the same graph trades overlaps in both directions (over the
/// 119 inputs that differ under portable math: more than C in 12, fewer in 9, same total).
pub const OVERLAP_SLACK: (f64, usize) = (0.25, 3);

fn same_set(what: &str, port: &Set, c: &Set) -> Option<String> {
    (port != c).then(|| {
        if port.count == c.count {
            format!("{what} differ (same count {}, different names)", c.count)
        } else {
            format!("{what}: {} in the port, {} in C", port.count, c.count)
        }
    })
}

pub const CHECKS: &[Check] = &[
    Check {
        name: "nodes",
        geometric: false,
        run: |p, c| same_set("node titles", &p.nodes, &c.nodes),
    },
    Check {
        name: "edges",
        geometric: false,
        run: |p, c| same_set("edge titles", &p.edges, &c.edges),
    },
    Check {
        name: "clusters",
        geometric: false,
        run: |p, c| same_set("cluster titles", &p.clusters, &c.clusters),
    },
    Check {
        name: "labels",
        geometric: false,
        run: |p, c| same_set("label texts", &p.texts, &c.texts),
    },
    Check {
        name: "finite",
        geometric: false,
        // Relative to C: for a few fuzz reproducers C itself writes `inf` or `nan`.
        run: |p, c| {
            (p.non_finite > c.non_finite).then(|| {
                format!(
                    "{} non-finite coordinates (C: {})",
                    p.non_finite, c.non_finite
                )
            })
        },
    },
    Check {
        name: "on-canvas",
        geometric: true,
        run: |p, c| {
            (p.off_canvas > c.off_canvas).then(|| {
                format!(
                    "{} points off the canvas (C: {})",
                    p.off_canvas, c.off_canvas
                )
            })
        },
    },
    Check {
        name: "canvas-size",
        geometric: true,
        run: |p, c| {
            let ratio = (p.width * p.height) / (c.width * c.height).max(1e-9);
            (!(AREA_RANGE.0..=AREA_RANGE.1).contains(&ratio)).then(|| {
                format!(
                    "canvas {:.0}x{:.0}pt is {ratio:.2}x the area of C's {:.0}x{:.0}pt",
                    p.width, p.height, c.width, c.height
                )
            })
        },
    },
    Check {
        name: "overlaps",
        geometric: true,
        run: |p, c| {
            let allowed = c.overlaps
                + (c.overlaps as f64 * OVERLAP_SLACK.0).ceil() as usize
                + OVERLAP_SLACK.1;
            (p.overlaps > allowed).then(|| {
                format!(
                    "{} overlapping node pairs (C: {}, allowed {allowed})",
                    p.overlaps, c.overlaps
                )
            })
        },
    },
];

/// Splits concatenated SVG documents (several graphs in one file) and summarizes each.
pub fn summarize(svg: &str) -> Result<Summary, String> {
    let mut docs = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<svg") {
        let end = rest[start..].find("</svg>").ok_or("unterminated <svg>")? + start + 6;
        docs.push(summarize_one(&rest[start..end])?);
        rest = &rest[end..];
    }
    Ok(docs)
}

/// An affine transform `[a b c d e f]`: (x, y) -> (a x + c y + e, b x + d y + f).
type Affine = [f64; 6];

fn compose(m: Affine, n: Affine) -> Affine {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

/// Parses the `scale`, `rotate` and `translate` functions Graphviz writes on its graph group.
fn parse_transform(t: &str) -> Affine {
    let mut m: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for part in t.split(')') {
        let Some((name, args)) = part.split_once('(') else {
            continue;
        };
        let v: Vec<f64> = args
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|s| s.parse().ok())
            .collect();
        let step: Affine = match (name.trim(), v.as_slice()) {
            ("scale", [s]) => [*s, 0.0, 0.0, *s, 0.0, 0.0],
            ("scale", [sx, sy, ..]) => [*sx, 0.0, 0.0, *sy, 0.0, 0.0],
            ("translate", [tx, ty, ..]) => [1.0, 0.0, 0.0, 1.0, *tx, *ty],
            ("translate", [tx]) => [1.0, 0.0, 0.0, 1.0, *tx, 0.0],
            ("rotate", [deg, ..]) => {
                let r = deg.to_radians();
                let (s, c) = (math::sin(r), math::cos(r));
                [c, s, -s, c, 0.0, 0.0]
            }
            _ => continue,
        };
        m = compose(m, step);
    }
    m
}

fn numbers(s: &str, bad: &mut usize) -> Vec<f64> {
    let mut out = Vec::new();
    for tok in s.split(|c: char| {
        c == ','
            || c.is_whitespace()
            || c.is_ascii_alphabetic() && !matches!(c, 'e' | 'n' | 'a' | 'i' | 'f')
    }) {
        if tok.is_empty() {
            continue;
        }
        match tok.parse::<f64>() {
            Ok(v) if v.is_finite() => out.push(v),
            _ => *bad += 1,
        }
    }
    out
}

fn summarize_one(svg: &str) -> Result<Doc, String> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(svg, opts).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let mut d = Doc::default();
    let vb: Vec<f64> = root
        .attribute("viewBox")
        .unwrap_or("0 0 0 0")
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let (vx, vy) = (
        vb.first().copied().unwrap_or(0.0),
        vb.get(1).copied().unwrap_or(0.0),
    );
    d.width = vb.get(2).copied().unwrap_or(0.0);
    d.height = vb.get(3).copied().unwrap_or(0.0);

    let (mut nodes, mut edges, mut clusters, mut texts) = (vec![], vec![], vec![], vec![]);
    let mut boxes: std::collections::HashMap<roxmltree::NodeId, [f64; 4]> = Default::default();
    let graph = root.children().find(|n| n.has_tag_name("g"));
    let m = graph
        .and_then(|g| g.attribute("transform"))
        .map_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], parse_transform);
    for el in root.descendants().filter(|n| n.is_element()) {
        let tag = el.tag_name().name();
        if tag == "g" {
            let title = || {
                el.children()
                    .find(|c| c.has_tag_name("title"))
                    .and_then(|t| t.text())
                    .unwrap_or("")
                    .to_string()
            };
            match el.attribute("class") {
                Some("node") => nodes.push(title()),
                Some("edge") => edges.push(title()),
                Some("cluster") => clusters.push(title()),
                _ => {}
            }
            continue;
        }
        if tag == "text" {
            texts.push(el.text().unwrap_or("").trim().to_string());
        }
        // Only drawn shapes carry coordinates; gradient definitions use percentages.
        if !matches!(
            tag,
            "path" | "polygon" | "polyline" | "ellipse" | "text" | "image"
        ) || el.ancestors().any(|a| a.has_tag_name("defs"))
        {
            continue;
        }
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for attr in ["points", "d"] {
            if let Some(v) = el.attribute(attr) {
                let ns = numbers(v, &mut d.non_finite);
                pts.extend(ns.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)));
            }
        }
        for (a, b) in [("x", "y"), ("cx", "cy")] {
            if let (Some(x), Some(y)) = (el.attribute(a), el.attribute(b)) {
                let ns = numbers(&format!("{x} {y}"), &mut d.non_finite);
                if let [x, y] = ns[..] {
                    pts.push((x, y));
                }
            }
        }
        if tag == "ellipse" {
            let g = |k| {
                el.attribute(k)
                    .and_then(|v: &str| v.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            let (cx, cy, rx, ry) = (g("cx"), g("cy"), g("rx"), g("ry"));
            pts.extend([(cx - rx, cy - ry), (cx + rx, cy + ry)]);
        }
        let pts: Vec<(f64, f64)> = pts
            .into_iter()
            .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]))
            .collect();
        d.off_canvas += pts
            .iter()
            .filter(|(x, y)| {
                *x < vx - 1.0
                    || *y < vy - 1.0
                    || *x > vx + d.width + 1.0
                    || *y > vy + d.height + 1.0
            })
            .count();
        // The shapes of a node (not its label) give its bounding box.
        let node = el
            .ancestors()
            .find(|a| a.has_tag_name("g") && a.attribute("class").is_some());
        if let Some(g) = node.filter(|g| g.attribute("class") == Some("node"))
            && matches!(tag, "ellipse" | "polygon" | "path" | "polyline")
            && !pts.is_empty()
        {
            let b = boxes
                .entry(g.id())
                .or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
            for &(x, y) in &pts {
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    d.nodes = Set::of(nodes);
    d.edges = Set::of(edges);
    d.clusters = Set::of(clusters);
    d.texts = Set::of(texts);
    d.overlaps = count_overlaps(&boxes.into_values().collect::<Vec<_>>());
    Ok(d)
}

fn count_overlaps(boxes: &[[f64; 4]]) -> usize {
    let mut sorted = boxes.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut n = 0;
    for (i, a) in sorted.iter().enumerate() {
        for b in &sorted[i + 1..] {
            if b[0] > a[2] - 1.0 {
                break;
            }
            if a[2].min(b[2]) - a[0].max(b[0]) > 1.0 && a[3].min(b[3]) - a[1].max(b[1]) > 1.0 {
                n += 1;
            }
        }
    }
    n
}

/// One problem found in one graph of one input.
pub struct Problem {
    pub file: String,
    pub graph: usize,
    pub check: &'static str,
    pub detail: String,
}

/// Compares the port's summary of an input with C's, running every check.
pub fn compare(file: &str, port: &Summary, c: &Summary, geometry: Geometry) -> Vec<Problem> {
    if port.len() != c.len() {
        return vec![Problem {
            file: file.into(),
            graph: 0,
            check: "graphs",
            detail: format!(
                "{} graphs in the port's output, {} in C's",
                port.len(),
                c.len()
            ),
        }];
    }
    let mut out = Vec::new();
    for (i, (p, c)) in port.iter().zip(c).enumerate() {
        for check in CHECKS {
            if check.geometric && geometry == Geometry::Unstable {
                continue;
            }
            if let Some(detail) = (check.run)(p, c) {
                out.push(Problem {
                    file: file.into(),
                    graph: i,
                    check: check.name,
                    detail,
                });
            }
        }
    }
    out
}

pub fn main(args: Vec<String>) -> ExitCode {
    let mut golden_path = PathBuf::from("tests/golden/structure.json");
    let e2e_path = PathBuf::from("tests/golden/e2e.json");
    let (mut generate, mut c_dot, mut filter) = (false, String::from("dot"), None::<String>);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--generate" => generate = true,
            "--golden" => golden_path = it.next().expect("--golden FILE").into(),
            "--c-dot" => c_dot = it.next().expect("--c-dot PROG"),
            "--filter" => filter = it.next(),
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let mut items = e2e::inputs(Path::new("."));
    if let Some(f) = &filter {
        items.retain(|(k, _)| k.contains(f.as_str()));
    }

    if generate {
        let outs = e2e::parallel(&items, |p| e2e::run_c(&c_dot, p).map(|s| summarize(&s)));
        let mut g = Golden::default();
        let mut failed = 0;
        for (k, v) in outs {
            match v {
                Some(Ok(sum)) if !sum.is_empty() => {
                    g.files.insert(k, sum);
                }
                Some(Err(e)) => {
                    eprintln!("{k}: C's SVG does not parse: {e}");
                    failed += 1;
                }
                _ => {}
            }
        }
        let json = serde_json::to_string(&g).expect("serialize");
        std::fs::write(&golden_path, json + "\n").expect("write golden");
        println!(
            "wrote {} summaries to {} ({failed} unparsable)",
            g.files.len(),
            golden_path.display()
        );
        return ExitCode::SUCCESS;
    }

    let golden: Golden = match std::fs::read_to_string(&golden_path) {
        Ok(s) => serde_json::from_str(&s).expect("parse structure golden"),
        Err(_) => {
            eprintln!(
                "no {}; run gv-diff structure --generate",
                golden_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let unstable: BTreeSet<String> = std::fs::read_to_string(&e2e_path)
        .ok()
        .and_then(|s| serde_json::from_str::<e2e::Golden>(&s).ok())
        .map(|g| g.unstable)
        .unwrap_or_default();
    items.retain(|(k, _)| golden.files.contains_key(k));
    std::panic::set_hook(Box::new(|_| {}));
    let outs = e2e::parallel(&items, |p| {
        std::fs::read(p)
            .ok()
            .and_then(|b| e2e::run_port(&graphviz_pure_render::decoded::Decoded::decode(b)))
            .map(|s| summarize(&s))
    });
    let _ = std::panic::take_hook();

    let mut problems = Vec::new();
    for (k, out) in &outs {
        let c = &golden.files[k];
        let geometry = if unstable.contains(k) {
            Geometry::Unstable
        } else {
            Geometry::Stable
        };
        match out {
            Some(Ok(port)) => problems.extend(compare(k, port, c, geometry)),
            Some(Err(e)) => problems.push(Problem {
                file: k.clone(),
                graph: 0,
                check: "parse",
                detail: e.clone(),
            }),
            None => problems.push(Problem {
                file: k.clone(),
                graph: 0,
                check: "render",
                detail: "no output".into(),
            }),
        }
    }
    let files: BTreeSet<&str> = problems.iter().map(|p| p.file.as_str()).collect();
    for p in &problems {
        println!(
            "  {} (graph {}): {}: {}",
            p.file,
            p.graph + 1,
            p.check,
            p.detail
        );
    }
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &problems {
        *per.entry(p.check).or_default() += 1;
    }
    println!(
        "{} inputs checked ({} with C geometry unstable: content only); {} with problems",
        outs.len(),
        outs.keys().filter(|k| unstable.contains(*k)).count(),
        files.len()
    );
    for c in CHECKS {
        println!("  {:<12} {}", c.name, per.get(c.name).copied().unwrap_or(0));
    }
    crate::print_baseline(&golden_path);
    if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str =
        r#"digraph G { a -> b [label="ab"]; subgraph cluster_x { label="X"; c; } b -> c; }"#;

    fn port_svg() -> String {
        let (parts, _) = graphviz_pure_render::decoded::render_dot_all_partial(
            &graphviz_pure_render::decoded::Decoded::from_text(GRAPH),
            "dot",
            "svg",
            &graphviz_pure_parser::GlobalAttrs::default(),
            graphviz_pure_layout::LayoutOptions::default(),
        );
        parts.concat()
    }

    /// The checks that fire when `broken` is compared with the intact drawing as C's.
    fn fired(broken: &str) -> Vec<&'static str> {
        let c = summarize(&port_svg()).unwrap();
        let p = summarize(broken).unwrap();
        compare("t", &p, &c, Geometry::Stable)
            .into_iter()
            .map(|p| p.check)
            .collect()
    }

    #[test]
    fn an_intact_drawing_passes() {
        assert!(fired(&port_svg()).is_empty());
    }

    #[test]
    fn a_missing_node_is_caught() {
        let svg = port_svg();
        let start = svg.find("<!-- a -->").unwrap();
        let end = start + svg[start..].find("</g>").unwrap() + 4;
        let broken = format!("{}{}", &svg[..start], &svg[end..]);
        assert!(fired(&broken).contains(&"nodes"));
    }

    #[test]
    fn a_changed_label_is_caught() {
        assert!(fired(&port_svg().replacen(">ab<", ">a?<", 1)).contains(&"labels"));
    }

    #[test]
    fn a_drawing_off_the_canvas_is_caught() {
        let svg = port_svg();
        let shifted = svg.replacen("translate(4 ", "translate(400 ", 1);
        assert!(fired(&shifted).contains(&"on-canvas"));
    }

    #[test]
    fn a_non_finite_coordinate_is_caught() {
        let svg = port_svg();
        let i = svg.find("<ellipse").unwrap();
        let broken = format!("{}{}", &svg[..i], svg[i..].replacen("cx=\"", "cx=\"nan", 1));
        assert!(fired(&broken).contains(&"finite"));
    }

    #[test]
    fn a_much_smaller_canvas_is_caught() {
        let svg = port_svg();
        let vb = svg.find("viewBox=\"").unwrap() + 9;
        let end = vb + svg[vb..].find('"').unwrap();
        let broken = format!("{}0.00 0.00 10.00 10.00{}", &svg[..vb], &svg[end..]);
        assert!(fired(&broken).contains(&"canvas-size"));
    }

    #[test]
    fn overlapping_nodes_are_caught() {
        // Six copies of node a's drawing on top of each other: 15 overlapping pairs.
        let svg = port_svg();
        let start = svg.find("<!-- a -->").unwrap();
        let end = start + svg[start..].find("</g>").unwrap() + 4;
        let node = &svg[start..end];
        let extra: String = (0..5)
            .map(|i| node.replace("<title>a</title>", &format!("<title>a{i}</title>")))
            .collect();
        let broken = format!("{}{extra}{}", &svg[..end], &svg[end..]);
        let checks = fired(&broken);
        assert!(checks.contains(&"overlaps"), "{checks:?}");
    }
}
