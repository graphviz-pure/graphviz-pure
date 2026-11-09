//! Ports of the helpers dotgen uses from `lib/common/utils.c` (attribute
//! readers with C defaults, union-find) and `lib/util` (`scale_clamp`).

use super::Dot;
use super::types::*;

pub use graphviz_pure_core::cnum::{atoi, late_int, map_bool, mapbool};

/// `late_double`
pub fn late_double(v: Option<&str>, default: f64, minimum: f64) -> f64 {
    let Some(p) = v else { return default };
    if p.is_empty() {
        return default;
    }
    let Some((rv, _)) = graphviz_pure_core::strtod::strtod_rest(p) else {
        return default;
    };
    if rv < minimum {
        return minimum;
    }
    rv
}

/// `scale_clamp` (lib/util/gv_math.h)
pub fn scale_clamp(original: i32, scale: f64) -> i32 {
    if scale < 0.0 {
        return 0;
    }
    if scale > 1.0 && (original as f64) > (i32::MAX as f64) / scale {
        return i32::MAX;
    }
    (original as f64 * scale) as i32
}

/// `ROUND(f)` from lib/common/macros.h.
#[inline]
pub fn round_c(f: f64) -> i32 {
    graphviz_pure_core::cnum::c_round(f)
}

/// `POINTS(a_inches)` = `ROUND(a * 72)`, whose `a * 72 ± .5` clang fuses.
#[inline]
pub fn points(inches: f64) -> i32 {
    graphviz_pure_core::cnum::c_round_mul(inches, 72.0)
}

impl Dot<'_> {
    /// `agget(g, name)`
    pub fn agget(&self, g: G, name: &str) -> Option<&str> {
        self.graphs[g].attrs.get(name).map(String::as_str)
    }

    /// `agxget(n, sym)` for a node.
    pub fn node_attr(&self, n: N, name: &str) -> Option<&str> {
        let r = self.cold(n).real?;
        self.input.nodes[r].attrs.get(name).map(String::as_str)
    }

    /// `agxget(e, sym)` for an edge.
    pub fn edge_attr(&self, e: E, name: &str) -> Option<&str> {
        let r = self.edges[e].real?;
        self.input.edges[r].attrs.get(name).map(String::as_str)
    }

    /// `is_a_cluster(g)`
    pub fn is_a_cluster(&self, g: G) -> bool {
        if g == 0 {
            // g == g->root; a pack component (a "_cc_N" subgraph in C) is not the root.
            return !self.comp || mapbool(self.agget(0, "cluster"));
        }
        let name = &self.graphs[g].name;
        graphviz_pure_core::has_cluster_prefix(name) || mapbool(self.agget(g, "cluster"))
    }

    /// `nonconstraint_edge(e)` (class1.c)
    pub fn nonconstraint_edge(&self, e: E) -> bool {
        if let Some(c) = self.edge_attr(e, "constraint")
            && !c.is_empty()
            && !mapbool(Some(c))
        {
            return true;
        }
        false
    }

    pub fn uf_find(&mut self, mut n: N) -> N {
        while let Some(p) = self.nodes[n].uf_parent {
            if p == n {
                break;
            }
            if let Some(pp) = self.nodes[p].uf_parent {
                self.nodes[n].uf_parent = Some(pp);
            }
            n = self.nodes[n].uf_parent.unwrap();
        }
        n
    }

    pub fn uf_union(&mut self, mut u: N, mut v: N) -> N {
        if u == v {
            return u;
        }
        if self.nodes[u].uf_parent.is_none() {
            self.nodes[u].uf_parent = Some(u);
            self.nodes[u].uf_size = 1;
        } else {
            u = self.uf_find(u);
        }
        if self.nodes[v].uf_parent.is_none() {
            self.nodes[v].uf_parent = Some(v);
            self.nodes[v].uf_size = 1;
        } else {
            v = self.uf_find(v);
        }
        if u == v {
            return u;
        }
        // ND_id is never set in dot, so `ND_id(u) > ND_id(v)` is always false.
        if self.nodes[u].id > self.nodes[v].id {
            self.nodes[u].uf_parent = Some(v);
            self.nodes[v].uf_size += self.nodes[u].uf_size;
            v
        } else {
            self.nodes[v].uf_parent = Some(u);
            self.nodes[u].uf_size += self.nodes[v].uf_size;
            u
        }
    }

    pub fn uf_singleton(&mut self, u: N) {
        self.nodes[u].uf_size = 1;
        self.nodes[u].uf_parent = None;
        self.nodes[u].ranktype = NORMAL;
    }

    pub fn uf_setname(&mut self, u: N, v: N) {
        self.nodes[u].uf_parent = Some(v);
        self.nodes[v].uf_size += self.nodes[u].uf_size;
    }
}
