//! Implementation crate of graphviz-pure, with no semver promise. The stable crates are
//! `graphviz-pure` and `graphviz-pure-raster`.
//!
//! Core data structures, error types, geometry primitives, and tables for `graphviz-pure`.

pub mod arrows;
pub mod cdt;
pub mod charset;
pub mod cnum;
pub mod color;
mod color_table;
pub mod diag;
pub mod error;
pub mod font;
pub mod geom;
pub mod graph;
pub mod math;
pub mod strtod;
pub mod usershape;
pub mod xdot;

pub use color::{
    RgbaColor, format_svg_color, resolve_color, resolve_color_with_scheme, warn_unknown_color,
};
pub use error::{GraphvizError, GraphvizErrorKind, Result};
pub use font::{
    DEFAULT_FONT_NAME, DEFAULT_FONT_SIZE, DEFAULT_LABEL_FONT_SIZE, LINESPACING, LabelSubstContext,
    TextJustify, TextLabelLayout, TextLineLayout, estimate_text_size, estimate_text_width_1pt,
    measure_multiline_label, measure_multiline_label_with_context,
};
pub use geom::{BezierSegment, BoundingBox, Point, Spline};
pub use graph::{
    AgName, AttrDicts, AttrMap, AttrSym, ClusterReg, EdgeData, EdgeId, EdgeLayout, Graph,
    GraphKind, GraphLayout, LabelPlace, NodeData, NodeId, NodeLayout, Obj, Port, PortNames,
    SubgraphData, SubgraphId, agnameof_buffer, has_cluster_prefix, node_name_is_buffer,
};
