//! Implementation crate of graphviz-pure, with no semver promise. The stable crates are
//! `graphviz-pure` and `graphviz-pure-raster`.
//!
//! Node and label geometry (the init half of `shapes.c`, `labels.c`,
//! `htmltable.c`) and `lib/pathplan` (`graphviz-pure-geom`).

#![forbid(unsafe_code)]

mod expat;
pub mod html_label;
pub mod node_size;
pub mod pathplan;
pub mod shapes;

pub use node_size::{
    EdgeLabelGeometry, GraphLabelGeometry, LabelGeometry, NodeGeometry, NodeSizeEnv,
    PolygonGeometry, edge_label_geometry, graph_label_geometry, graph_label_geometry_in,
    label_text, make_label, node_geometry, size_graph_nodes,
};
pub use shapes::{RecordField, ShapeKind};
