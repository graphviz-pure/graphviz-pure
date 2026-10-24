//! `lib/twopigen/twopiinit.c`: the twopi driver.

use super::circle::{RData, circle_layout};
use crate::common::postproc::gv_postprocess;
use crate::common::types::{EdgeType, LGraph};
use crate::common::utils::mapbool;
use crate::neatogen::adjust::adjust_nodes;
use crate::neatogen::neatoinit::CL_OFFSET;
use crate::neatogen::neatosplines::spline_edges;
use crate::pack::{PackInfo, PackMode, ccomps, get_pack_info, pack_graphs};
use graphviz_pure_core::{Graph, Result};

/// `findRootNode(sg, rootattr)`: first node whose `root` is true.
fn find_root_node(g: &LGraph<'_>, nodes: &[usize]) -> Option<usize> {
    nodes.iter().copied().find(|&n| mapbool(g.nattr(n, "root")))
}

/// `twopi_layout(g)`.
pub fn twopi_layout(graph: &mut Graph) -> Result<()> {
    if graph.nodes.is_empty() {
        crate::common::postproc::labels_at_origin(graph);
        return Ok(());
    }
    let src = graph.clone();
    // twopi_init_graph: EDGETYPE_LINE by default, 2 dimensions.
    let mut g = LGraph::new(&src, 2, EdgeType::Line);
    let mut ctr: Option<usize> = None;
    let mut set_root = false;
    if let Some(s) = g.gattr("root") {
        if !s.is_empty() {
            // `agfindnode` maps the interned value, flag included, to a node.
            let html = src.graph_attrs.is_html("root");
            ctr = src.agfindnode(s, html).map(|n| n.0 as usize);
            if ctr.is_none() {
                graphviz_pure_core::diag::agwarningf(&format!(
                    "specified root node \"{s}\" was not found."
                ));
                graphviz_pure_core::diag::agerr_prev("Using default calculation for root node\n");
                set_root = true;
            }
        } else {
            set_root = true;
        }
    }
    let set_local_root = g.node_attr_declared("root");
    let mut local_roots: Vec<usize> = Vec::new();

    let mut r = RData::new(g.nodes.len());
    let mut ccs = ccomps(&g);
    if ccs.len() == 1 {
        let lctr = if ctr.is_some() {
            ctr
        } else if set_local_root {
            find_root_node(&g, &ccs[0].nodes)
        } else {
            None
        };
        let c = circle_layout(&mut g, &ccs[0], &mut r, lctr);
        if set_root && ctr.is_none() {
            ctr = c;
        }
        if set_local_root
            && lctr.is_none()
            && let Some(c) = c
        {
            local_roots.push(c);
        }
        let mut root = g.root_sub();
        adjust_nodes(&mut g, &root);
        spline_edges(&mut g, &mut root, true);
    } else {
        let mut pinfo = PackInfo::default();
        get_pack_info(&g, PackMode::Node, CL_OFFSET, &mut pinfo);
        pinfo.do_splines = false;
        for sg in &ccs {
            let lctr = match ctr {
                Some(c) if sg.member[c] => Some(c),
                _ if set_local_root => find_root_node(&g, &sg.nodes),
                _ => None,
            };
            let c = circle_layout(&mut g, sg, &mut r, lctr);
            if set_root && ctr.is_none() {
                ctr = c;
            }
            if set_local_root
                && (lctr.is_none() || lctr == ctr)
                && let Some(c) = c
            {
                local_roots.push(c);
            }
            adjust_nodes(&mut g, sg);
        }
        pack_graphs(&mut g, &mut ccs, &pinfo);
        let mut root = g.root_sub();
        spline_edges(&mut g, &mut root, true);
    }
    gv_postprocess(&mut g, true);
    g.write_back(graph);
    for c in local_roots {
        graph.nodes[c]
            .attrs
            .insert("root".to_string(), "1".to_string());
    }
    if set_root && let Some(c) = ctr {
        // `agnameof(ctr)` keeps an HTML name's flag (`agstrdup` of an interned string).
        let (name, html) = {
            let nd = &graph.nodes[c];
            (nd.name.clone(), nd.html_name)
        };
        // agset on the root graph also updates the attribute's default.
        if let Some(d) = graph.dicts.first_mut() {
            let sym = d.graph.entry("root".to_string()).or_default();
            sym.defval = name.clone();
            sym.html_defval = html;
        }
        graph.graph_attrs.set("root", name, html);
    }
    Ok(())
}
