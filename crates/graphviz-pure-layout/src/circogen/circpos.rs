//! `lib/circogen/circpos.c`: position the blocks of the block tree.

use super::block::COALESCED_F;
use super::blockpath::layout_block;
use super::circular::{Circ, PARENT_F};
use super::graph::CGraph;
use graphviz_pure_core::math;
use std::f64::consts::PI;

/// `getRotation(sn, x, y, theta)`: how far to rotate block `sn`, centred at
/// `(x, y)` relative to its parent, so its attaching node faces the parent.
fn get_rotation(c: &Circ, sn: usize, x: f64, y: f64, mut theta: f64) -> f64 {
    let b = &c.blocks[sn];
    if b.parent_pos >= 0.0 {
        theta += PI - b.parent_pos;
        if theta < 0.0 {
            theta += 2.0 * PI;
        }
        return theta;
    }
    if b.circle_list.len() == 2 {
        return theta - PI / 2.0;
    }

    // Find node in block connected to block's parent
    let neighbor = b.child.expect("child block has an attaching node");
    let new_x = c.pos[neighbor][0] + x;
    let new_y = c.pos[neighbor][1] + y;
    let mut mindist2 = math::hypot(new_x, new_y);
    let mut closest_node = neighbor;
    for n in b.sub_graph.nodes() {
        if n == neighbor {
            continue;
        }
        let new_x = c.pos[n][0] + x;
        let new_y = c.pos[n][1] + y;
        let len2 = math::hypot(new_x, new_y);
        if len2 < mindist2 {
            mindist2 = len2;
            closest_node = n;
        }
    }

    if neighbor != closest_node {
        let rho = b.rad0;
        let r = b.radius - rho;
        let n_x = c.pos[neighbor][0];
        if b.flags & COALESCED_F != 0 && -r < n_x {
            let big_r = math::hypot(x, y);
            let n_y = c.pos[neighbor][1];
            let phi = math::atan2(n_y, n_x + r);
            let l = r - rho / math::cos(phi);
            theta += PI / 2.0 - phi - math::asin(l / big_r * math::cos(phi));
        } else {
            // Origin still at center of this block
            let phi = math::atan2(c.pos[neighbor][1], c.pos[neighbor][0]);
            theta += PI - phi - c.nd[neighbor].psi;
            if theta > 2.0 * PI {
                theta -= 2.0 * PI;
            }
        }
    } else {
        theta = 0.0;
    }
    theta
}

/// `applyDelta(sn, x, y, rotate)`: rotate then translate block `sn` and its
/// descendants.
fn apply_delta(c: &mut Circ, sn: usize, x: f64, y: f64, rotate: f64) {
    for n in c.blocks[sn].sub_graph.nodes() {
        let tmp_x = c.pos[n][0];
        let tmp_y = c.pos[n][1];
        let cos_r = math::cos(rotate);
        let sin_r = math::sin(rotate);
        // clang contracts `a*b - c*d` / `a*b + c*d` into one fused multiply-add.
        let big_x = math::fma(tmp_x, cos_r, -(tmp_y * sin_r));
        let big_y = math::fma(tmp_x, sin_r, tmp_y * cos_r);
        c.pos[n] = [big_x + x, big_y + y];
    }
    for child in c.blocks[sn].children.clone() {
        apply_delta(c, child, x, y, rotate);
    }
}

/// `posstate`.
struct PosState {
    radius: f64,
    subtree_r: f64,
    node_angle: f64,
    first_angle: f64,
    last_angle: f64,
    /// `cp`: the children of the block.
    cp: Vec<usize>,
    neighbor: Option<usize>,
}

/// `posinfo_t`.
#[derive(Default, Clone)]
struct PosInfo {
    n: usize,
    theta: f64,
    min_radius: f64,
    max_radius: f64,
    diameter: f64,
    scale: f64,
    child_count: i32,
}

/// `getInfo(pi, stp, min_dist)`: size of the blocks attached at `pi.n`.
fn get_info(c: &Circ, pi: &mut PosInfo, stp: &PosState, min_dist: f64) -> f64 {
    let mut max_radius = 0.0f64;
    let mut diameter = 0.0;
    let mut child_count = 0;
    for &child in &stp.cp {
        if c.blk_parent(child) == Some(pi.n) {
            child_count += 1;
            max_radius = max_radius.max(c.blocks[child].radius);
            diameter += 2.0 * c.blocks[child].radius + min_dist;
        }
    }
    pi.diameter = diameter;
    pi.child_count = child_count;
    pi.min_radius = stp.radius + min_dist + max_radius;
    pi.max_radius = max_radius;
    max_radius
}

/// `setInfo(p0, p1, delta)`.
fn set_info(p0: &mut PosInfo, p1: &mut PosInfo, delta: f64) {
    let mut t = math::fma(p0.diameter, p1.min_radius, p1.diameter * p0.min_radius);
    t /= 2.0 * delta * p0.min_radius * p1.min_radius;
    t = t.max(1.0);
    p0.scale = p0.scale.max(t);
    p1.scale = p1.scale.max(t);
}

/// `positionChildren(info, stp, length, min_dist)`.
fn position_children(
    c: &mut Circ,
    info: &PosInfo,
    stp: &mut PosState,
    length: usize,
    mut min_dist: f64,
) {
    let mut child_angle;
    let mut child_radius;
    let mut mid_angle = 0.0;
    let mut cnt = 0;
    let mut sn_radius = stp.subtree_r;
    let mut first_angle = stp.first_angle;
    let mut last_angle = stp.last_angle;

    child_radius = info.scale * info.min_radius;
    if length == 1 {
        child_angle = 0.0;
        let d = info.diameter / (2.0 * PI);
        child_radius = child_radius.max(d);
        let d = math::fma(2.0 * PI, child_radius, -info.diameter);
        if d > 0.0 {
            min_dist += d / f64::from(info.child_count);
        }
    } else {
        child_angle = info.theta - info.diameter / (2.0 * child_radius);
    }

    sn_radius = sn_radius.max(child_radius + info.max_radius);

    let mindist_angle = min_dist / child_radius;

    let mid_child = (info.child_count + 1) / 2;
    for child in stp.cp.clone() {
        if c.blk_parent(child) != Some(info.n) {
            continue;
        }
        if c.blocks[child].circle_list.is_empty() {
            continue;
        }

        let incident_angle = c.blocks[child].radius / child_radius;
        if length == 1 {
            if child_angle != 0.0 {
                if info.child_count == 2 {
                    child_angle = PI;
                } else {
                    child_angle += incident_angle;
                }
            }
            if first_angle < 0.0 {
                first_angle = child_angle;
            }
            last_angle = child_angle;
        } else if info.child_count == 1 {
            child_angle = info.theta;
        } else {
            child_angle += incident_angle + mindist_angle / 2.0;
        }

        let delta_x = child_radius * math::cos(child_angle);
        let delta_y = child_radius * math::sin(child_angle);

        // first apply the delta to the immediate child and see if we need
        // to rotate it for better edge link
        let rotate_angle = get_rotation(c, child, delta_x, delta_y, child_angle);
        apply_delta(c, child, delta_x, delta_y, rotate_angle);

        if length == 1 {
            child_angle += incident_angle + mindist_angle;
        } else {
            child_angle += incident_angle + mindist_angle / 2.0;
        }
        cnt += 1;
        if cnt == mid_child {
            mid_angle = child_angle;
        }
    }

    if length > 1 && Some(info.n) == stp.neighbor {
        c.nd[info.n].psi = mid_angle;
    }

    stp.subtree_r = sn_radius;
    stp.first_angle = first_angle;
    stp.last_angle = last_angle;
}

/// `position(childCount, length, nodepath, sn, min_dist)`: place the child
/// blocks of `sn` around their parent nodes; returns the centre angle used
/// when `sn` has a single node.
fn position(
    c: &mut Circ,
    child_count: usize,
    length: usize,
    nodepath: &[usize],
    sn: usize,
    min_dist: f64,
) -> f64 {
    let mut max_radius = 0.0;
    let mut parents: Vec<PosInfo> = Vec::with_capacity(child_count);
    let mut state = PosState {
        cp: c.blocks[sn].children.clone(),
        subtree_r: c.blocks[sn].radius,
        radius: c.blocks[sn].radius,
        neighbor: c.blocks[sn].child,
        node_angle: 2.0 * PI / length as f64,
        first_angle: -1.0,
        last_angle: -1.0,
    };

    for (counter, &n) in nodepath.iter().enumerate() {
        let theta = counter as f64 * state.node_angle;
        if c.flag(n, PARENT_F) {
            let mut pi = PosInfo {
                n,
                theta,
                ..PosInfo::default()
            };
            max_radius = get_info(c, &mut pi, &state, min_dist);
            parents.push(pi);
        }
    }

    let num_parents = parents.len();
    if num_parents == 1 {
        parents[0].scale = 1.0;
    } else if num_parents == 2 {
        let mut delta = parents[1].theta - parents[0].theta;
        if delta > PI {
            delta = 2.0 * PI - delta;
        }
        let (a, b) = parents.split_at_mut(1);
        set_info(&mut a[0], &mut b[0], delta);
    } else {
        for i in 0..num_parents {
            let (ni, delta) = if i + 1 == num_parents {
                (0, parents[0].theta - parents[i].theta + 2.0 * PI)
            } else {
                (i + 1, parents[i + 1].theta - parents[i].theta)
            };
            let (mut p0, mut p1) = (parents[i].clone(), parents[ni].clone());
            set_info(&mut p0, &mut p1, delta);
            parents[i] = p0;
            parents[ni] = p1;
        }
    }

    for i in 0..num_parents {
        let info = parents[i].clone();
        position_children(c, &info, &mut state, length, min_dist);
    }

    // If block has only 1 child, to save space, we coalesce it with the
    // child: the origin of the block is no longer its centre.
    if child_count == 1 {
        apply_delta(c, sn, -(max_radius + min_dist / 2.0), 0.0, 0.0);
        c.blocks[sn].radius += min_dist / 2.0 + max_radius;
        c.blocks[sn].flags |= COALESCED_F;
    } else {
        c.blocks[sn].radius = state.subtree_r;
    }

    (state.first_angle + state.last_angle) / 2.0 - PI
}

/// `doBlock(g, sn, min_dist, state)`: lay out the subtree of blocks at `sn`.
fn do_block(c: &mut Circ, g: &CGraph, sn: usize, min_dist: f64) {
    let mut center_angle = PI;
    let children = c.blocks[sn].children.clone();
    let child_count = children.len();
    for child in children {
        do_block(c, g, child, min_dist);
    }

    let longest_path = layout_block(c, g, sn, min_dist);
    c.blocks[sn].circle_list = longest_path.clone();
    let length = longest_path.len();

    if child_count > 0 {
        center_angle = position(c, child_count, length, &longest_path, sn, min_dist);
    }

    if length == 1 && c.blk_parent(sn).is_some() {
        let b = &mut c.blocks[sn];
        b.parent_pos = center_angle;
        if b.parent_pos < 0.0 {
            b.parent_pos += 2.0 * PI;
        }
    }
}

/// `circPos(g, sn, state)`.
pub fn circ_pos(c: &mut Circ, g: &CGraph, sn: usize) {
    let md = c.min_dist;
    do_block(c, g, sn, md);
}
