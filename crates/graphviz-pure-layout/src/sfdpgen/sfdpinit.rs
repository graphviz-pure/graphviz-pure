//! The sfdp driver, ported from `lib/sfdpgen/sfdpinit.c`: `sfdp_layout`,
//! `sfdp_init_graph`, `sfdpLayout`, `tuneControl`, `late_smooth`,
//! `late_quadtree_scheme`.
//!
//! Graphviz 16.1.0 on Homebrew is built with GTS, so the default overlap
//! mode is `prism0` and the triangle/RNG smoothers are available.

use super::spring_electrical::*;
use crate::common::postproc::gv_postprocess;
use crate::common::types::*;
use crate::common::utils::{compute_bb, late_double, late_int, map_bool, mapbool};
use crate::neatogen::adjust::{
    AdjustMode, DFLT_MARGIN, graph_adjust_mode, remove_overlap_with, sep_factor,
};
use crate::neatogen::neatoinit::{CL_OFFSET, set_seed};
use crate::neatogen::neatosplines::spline_edges;
use crate::neatogen::overlap::{get_sizes, make_matrix};
use crate::neatogen::{INIT_RANDOM, INIT_REGULAR, INIT_SELF};
use crate::pack::{PackInfo, PackMode, ccomps, get_pack_info, pack_graphs};
use graphviz_pure_core::{Graph, Result};

/// `ELS`: prefix of nodes that stand for edge labels.
const ELS: &str = "|edgelabel|";

/// `sfdpLayout(g, ctrl, pad)`: lay out one connected component.
fn sfdp_layout_component(
    g: &mut LGraph<'_>,
    sub: &Sub,
    ctrl: &mut SpringElectricalControl,
    pad: (f64, f64),
) {
    let ndim = g.ndim;
    let mut a = make_matrix(g, sub);
    let mut edge_label_nodes = Vec::new();
    let sizes = if ctrl.overlap >= 0 {
        if ctrl.edge_labeling_scheme > 0 {
            for &v in &sub.nodes {
                if g.src.nodes[v].name.starts_with(ELS) {
                    edge_label_nodes.push(g.nodes[v].id);
                }
            }
        }
        Some(get_sizes(g, sub, pad))
    } else {
        None
    };
    // getPos: sfdp always starts from a random layout, so this is all zeros.
    let mut pos = vec![0.0f64; ndim * sub.nodes.len()];
    let mut flag = 0;
    multilevel_spring_electrical_embedding(
        ndim,
        &mut a,
        ctrl,
        sizes.as_deref(),
        &mut pos,
        &edge_label_nodes,
        &mut flag,
    );
    for &v in &sub.nodes {
        let i = g.nodes[v].id;
        for k in 0..ndim {
            g.nodes[v].pos[k] = pos[ndim * i + k];
        }
    }
}

fn is_digit(s: &str) -> bool {
    s.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

fn is_alpha(s: &str) -> bool {
    s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

/// `late_smooth(g, sym, dflt)`.
fn late_smooth(s: Option<&str>, dflt: i32) -> i32 {
    let Some(s) = s else { return dflt };
    if is_digit(s) {
        let v = crate::common::utils::atoi(s);
        if v <= SMOOTHING_RNG { v } else { dflt }
    } else if is_alpha(s) {
        match s.to_ascii_lowercase().as_str() {
            "avg_dist" => SMOOTHING_STRESS_MAJORIZATION_AVG_DIST,
            "graph_dist" => SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST,
            "none" => SMOOTHING_NONE,
            "power_dist" => SMOOTHING_STRESS_MAJORIZATION_POWER_DIST,
            "rng" => SMOOTHING_RNG,
            "spring" => SMOOTHING_SPRING,
            "triangle" => SMOOTHING_TRIANGLE,
            _ => dflt,
        }
    } else {
        dflt
    }
}

/// `late_quadtree_scheme(g, sym, dflt)`.
fn late_quadtree_scheme(s: Option<&str>, dflt: i32) -> i32 {
    let Some(s) = s else { return dflt };
    if is_digit(s) {
        let v = crate::common::utils::atoi(s);
        if (QUAD_TREE_NONE..=QUAD_TREE_FAST).contains(&v) {
            v
        } else {
            dflt
        }
    } else if is_alpha(s) {
        match s.to_ascii_lowercase().as_str() {
            "none" | "false" => QUAD_TREE_NONE,
            "normal" | "true" | "yes" => QUAD_TREE_NORMAL,
            "fast" => QUAD_TREE_FAST,
            _ => dflt,
        }
    } else {
        dflt
    }
}

/// `tuneControl(g, ctrl)`: user attributes override the defaults.
fn tune_control(g: &LGraph<'_>, ctrl: &mut SpringElectricalControl) {
    let attrs = &g.src.graph_attrs;
    let mut seed = i64::from(ctrl.random_seed);
    let init = set_seed(g, INIT_RANDOM, &mut seed);
    if init != INIT_RANDOM {
        let _ = (INIT_SELF, INIT_REGULAR);
        graphviz_pure_core::diag::agwarningf("sfdp only supports start=random\n");
    }
    ctrl.random_seed = seed as i32;
    ctrl.k = late_double(attrs, "K", -1.0, 0.0);
    ctrl.p = -late_double(attrs, "repulsiveforce", -AUTOP, 0.0);
    ctrl.multilevels = late_int(attrs, "levels", i32::MAX, 0);
    ctrl.smoothing = late_smooth(g.gattr("smoothing"), SMOOTHING_NONE);
    ctrl.tscheme = late_quadtree_scheme(g.gattr("quadtree"), QUAD_TREE_NORMAL);
    ctrl.beautify_leaves = mapbool(g.gattr("beautify"));
    ctrl.do_shrinking = map_bool(g.gattr("overlap_shrink"), true);
    ctrl.rotation = late_double(attrs, "rotation", 0.0, -f64::MAX);
    ctrl.edge_labeling_scheme = late_int(attrs, "label_scheme", 0, 0);
    if ctrl.edge_labeling_scheme > 4 {
        graphviz_pure_core::diag::agwarningf(&format!(
            "label_scheme = {} > 4 : ignoring\n",
            ctrl.edge_labeling_scheme
        ));
        ctrl.edge_labeling_scheme = 0;
    }
}

/// `sfdp_layout(g)`.
pub fn sfdp_layout(graph: &mut Graph) -> Result<()> {
    // sfdp_init_graph
    let attrs = &graph.graph_attrs;
    let outdim = late_int(attrs, "dimen", 2, 2);
    let ndim = late_int(attrs, "dim", outdim, 2).min(MAXDIM as i32) as usize;
    let src = graph.clone();
    let mut g = LGraph::new(&src, ndim, EdgeType::Line);
    let mut do_adjust = ndim == 2;

    if !g.nodes.is_empty() {
        let mut ctrl = SpringElectricalControl::default();
        tune_control(&g, &mut ctrl);
        let am = graph_adjust_mode(&g, Some("prism0"));
        let mut pad = (DFLT_MARGIN / 72.0, DFLT_MARGIN / 72.0);
        if am.mode == AdjustMode::Prism && do_adjust {
            do_adjust = false;
            ctrl.overlap = am.value;
            ctrl.initial_scaling = am.scaling;
            let sep = sep_factor(&g);
            if sep.do_add {
                pad = (sep.x / 72.0, sep.y / 72.0);
            }
        } else {
            ctrl.overlap = -1;
        }
        if graphviz_pure_core::diag::verbose_on() {
            super::spring_electrical::spring_electrical_control_print(&ctrl);
        }

        let mut ccs = ccomps(&g);
        if ccs.len() == 1 {
            let mut root = g.root_sub();
            sfdp_layout_component(&mut g, &root, &mut ctrl, pad);
            if do_adjust {
                remove_overlap_with(&mut g, &root, &am);
            }
            spline_edges(&mut g, &mut root, true);
        } else {
            let mut pinfo = PackInfo::default();
            get_pack_info(&g, PackMode::Node, CL_OFFSET, &mut pinfo);
            pinfo.do_splines = true;
            for sg in ccs.iter_mut() {
                sfdp_layout_component(&mut g, sg, &mut ctrl, pad);
                if do_adjust {
                    remove_overlap_with(&mut g, sg, &am);
                }
                crate::common::init::component_set_edge_type(&g);
                spline_edges(&mut g, sg, false);
            }
            // packSubgraphs: packGraphs, then the root's bounding box.
            if pack_graphs(&mut g, &mut ccs, &pinfo) {
                let mut root = g.root_sub();
                compute_bb(&mut g, &mut root, true);
            }
        }
    }
    gv_postprocess(&mut g, true);
    // GD_odim(g->root) = MIN(outdim, Ndim)
    g.odim = (outdim as usize).min(ndim);
    g.write_back(graph);
    Ok(())
}
