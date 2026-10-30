//! Port of Graphviz `lib/vpsc` (Tim Dwyer's Variable Placement with
//! Separation Constraints solver): `variable.h`, `constraint.cpp`,
//! `block.cpp`, `blocks.cpp`, `solve_VPSC.cpp` (the incremental `IncVPSC`
//! that Graphviz instantiates exclusively, via `newIncVPSC`),
//! `generate-constraints.cpp` and the `csolve_VPSC.cpp` bridge.
//!
//! Variables, constraints and blocks live in arenas indexed by `usize`.
//! Constraint `delete` removes the constraint from its variables' lists, as
//! the C++ destructor does. The block set `Blocks : std::set<Block*>` is
//! ordered by pointer in C++; here a block's address is its allocation
//! sequence number (see [`Solver::bs`]). `std::sort` is libc++'s.
//!
//! Unreachable from Graphviz and not ported: `VPSC::satisfy/refine/solve`
//! (non-incremental), `Blocks::mergeLeft/mergeRight/split/totalOrder`, the
//! block constraint heaps, the cycle checks and logging.

use crate::cutil::libcxx;
use graphviz_pure_core::math;
use std::collections::BTreeSet;

const NONE: usize = usize::MAX;

/// `Variable`.
#[derive(Debug, Clone)]
pub struct Variable {
    pub id: i32,
    pub desired_position: f64,
    pub weight: f64,
    pub offset: f64,
    pub block: usize,
    pub visited: bool,
    pub in_: Vec<usize>,
    pub out: Vec<usize>,
}

/// `Constraint`.
#[derive(Debug, Clone)]
pub struct Constraint {
    pub left: usize,
    pub right: usize,
    pub gap: f64,
    pub lm: f64,
    pub time_stamp: i64,
    pub active: bool,
    pub visited: bool,
}

/// `Block`.
#[derive(Debug, Clone)]
struct Block {
    vars: Vec<usize>,
    posn: f64,
    weight: f64,
    wposn: f64,
    deleted: bool,
    /// Stand-in for the C++ object address (ordering of `Blocks`).
    addr: u64,
}

/// `Rectangle` (generate-constraints.h).
#[derive(Debug, Clone, Copy)]
pub struct Rectangle {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl Rectangle {
    pub fn new(x: f64, xx: f64, y: f64, yy: f64) -> Self {
        Rectangle {
            min_x: x,
            max_x: xx,
            min_y: y,
            max_y: yy,
        }
    }
    fn width(&self) -> f64 {
        self.max_x - self.min_x
    }
    fn height(&self) -> f64 {
        self.max_y - self.min_y
    }
    fn centre_x(&self) -> f64 {
        self.min_x + self.width() / 2.0
    }
    fn centre_y(&self) -> f64 {
        self.min_y + self.height() / 2.0
    }
    fn overlap_x(&self, r: &Rectangle) -> f64 {
        if self.centre_x() <= r.centre_x() && r.min_x < self.max_x {
            return self.max_x - r.min_x;
        }
        if r.centre_x() <= self.centre_x() && self.min_x < r.max_x {
            return r.max_x - self.min_x;
        }
        0.0
    }
    fn overlap_y(&self, r: &Rectangle) -> f64 {
        if self.centre_y() <= r.centre_y() && r.min_y < self.max_y {
            return self.max_y - r.min_y;
        }
        if r.centre_y() <= self.centre_y() && self.min_y < r.max_y {
            return r.max_y - self.min_y;
        }
        0.0
    }
}

/// The C++ heap objects of one solving session: variables and constraints
/// persist across `IncVPSC` instances (`quad_prog_vpsc.c` reuses them).
#[derive(Debug, Default, Clone)]
pub struct Solver {
    pub vars: Vec<Variable>,
    pub cons: Vec<Constraint>,
    blocks: Vec<Block>,
    /// Not in C: indices in `blocks` that `free_unused_blocks` freed.
    free_blocks: Vec<usize>,
    /// `Blocks` (`std::set<Block*>`), keyed by `(addr, block)`.
    bs: BTreeSet<(u64, usize)>,
    next_addr: u64,
    /// `blockTimeCtr`.
    block_time_ctr: i64,
    /// `VPSC::cs` of the current instance.
    cs: Vec<usize>,
    /// `IncVPSC::inactive`.
    inactive: Vec<usize>,
    pub split_cnt: u32,
}

/// `std::runtime_error` thrown by the solver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpscError(pub &'static str);

impl VpscError {
    /// `IncVPSC::satisfy`'s "Cycle Error!", after 10000 in-block splits.
    pub const CYCLE: Self = Self("Cycle Error!");
}

const EPS: f64 = 0.0000001;

impl Solver {
    pub fn new() -> Self {
        Self::default()
    }

    /// `newVariable(id, desiredPos, weight)`.
    pub fn new_variable(&mut self, id: i32, desired: f64, weight: f64) -> usize {
        self.vars.push(Variable {
            id,
            desired_position: desired,
            weight,
            offset: 0.0,
            block: NONE,
            visited: false,
            in_: Vec::new(),
            out: Vec::new(),
        });
        self.vars.len() - 1
    }

    /// `newConstraint(left, right, gap)`.
    pub fn new_constraint(&mut self, left: usize, right: usize, gap: f64) -> usize {
        let c = self.cons.len();
        self.cons.push(Constraint {
            left,
            right,
            gap,
            lm: 0.0,
            time_stamp: 0,
            active: false,
            visited: false,
        });
        self.vars[left].out.push(c);
        self.vars[right].in_.push(c);
        c
    }

    /// `delete c` (`Constraint::~Constraint`).
    pub fn delete_constraint(&mut self, c: usize) {
        let (l, r) = (self.cons[c].left, self.cons[c].right);
        if let Some(i) = self.vars[l].out.iter().position(|&x| x == c) {
            self.vars[l].out.remove(i);
        }
        if let Some(i) = self.vars[r].in_.iter().position(|&x| x == c) {
            self.vars[r].in_.remove(i);
        }
    }

    /// `remapInConstraints(u, v, dgap)`.
    pub fn remap_in_constraints(&mut self, u: usize, v: usize, dgap: f64) {
        let ins = std::mem::take(&mut self.vars[u].in_);
        for c in ins {
            self.cons[c].right = v;
            self.cons[c].gap += dgap;
            self.vars[v].in_.push(c);
        }
    }

    /// `remapOutConstraints(u, v, dgap)`.
    pub fn remap_out_constraints(&mut self, u: usize, v: usize, dgap: f64) {
        let outs = std::mem::take(&mut self.vars[u].out);
        for c in outs {
            self.cons[c].left = v;
            self.cons[c].gap += dgap;
            self.vars[v].out.push(c);
        }
    }

    #[inline]
    pub fn position(&self, v: usize) -> f64 {
        self.blocks[self.vars[v].block].posn + self.vars[v].offset
    }

    #[inline]
    fn slack(&self, c: usize) -> f64 {
        let k = &self.cons[c];
        self.position(k.right) - k.gap - self.position(k.left)
    }

    // ---- Block ----

    /// `new Block(v)`.
    fn new_block(&mut self, v: Option<usize>) -> usize {
        let block = Block {
            vars: Vec::new(),
            posn: 0.0,
            weight: 0.0,
            wposn: 0.0,
            deleted: false,
            addr: self.next_addr,
        };
        let b = if let Some(b) = self.free_blocks.pop() {
            self.blocks[b] = block;
            b
        } else {
            self.blocks.push(block);
            self.blocks.len() - 1
        };
        self.next_addr += 1;
        if let Some(v) = v {
            self.vars[v].offset = 0.0;
            self.add_variable(b, v);
        }
        b
    }

    /// `Block::addVariable`.
    fn add_variable(&mut self, b: usize, v: usize) {
        self.vars[v].block = b;
        self.blocks[b].vars.push(v);
        let var = &self.vars[v];
        let blk = &mut self.blocks[b];
        blk.weight += var.weight;
        blk.wposn = math::fma(var.weight, var.desired_position - var.offset, blk.wposn);
        blk.posn = blk.wposn / blk.weight;
    }

    /// `Block::desiredWeightedPosition`.
    fn desired_weighted_position(&self, b: usize) -> f64 {
        let mut wp = 0.0f64;
        for &v in &self.blocks[b].vars {
            let var = &self.vars[v];
            wp = math::fma(var.desired_position - var.offset, var.weight, wp);
        }
        wp
    }

    /// `Block::merge(b, c)`.
    fn merge2(&mut self, this: usize, b: usize, c: usize) {
        let k = &self.cons[c];
        let dist = self.vars[k.right].offset - self.vars[k.left].offset - k.gap;
        let l = self.vars[k.left].block;
        let r = self.vars[k.right].block;
        if self.blocks[this].vars.len() < self.blocks[b].vars.len() {
            self.merge3(r, l, c, dist);
        } else {
            self.merge3(l, r, c, -dist);
        }
    }

    /// `Block::merge(b, c, dist)`: merge `b` into `this` across `c`.
    fn merge3(&mut self, this: usize, b: usize, c: usize, dist: f64) {
        self.cons[c].active = true;
        let (bw, bwp) = (self.blocks[b].weight, self.blocks[b].wposn);
        {
            let t = &mut self.blocks[this];
            t.wposn += math::fma(-dist, bw, bwp);
            t.weight += bw;
            t.posn = t.wposn / t.weight;
        }
        let bvars = std::mem::take(&mut self.blocks[b].vars);
        for &v in &bvars {
            self.vars[v].block = this;
            self.vars[v].offset += dist;
            self.blocks[this].vars.push(v);
        }
        self.blocks[b].vars = bvars;
        self.blocks[b].deleted = true;
    }

    #[inline]
    fn can_follow_left(&self, this: usize, c: usize, last: usize) -> bool {
        let k = &self.cons[c];
        self.vars[k.left].block == this && k.active && last != k.left
    }

    #[inline]
    fn can_follow_right(&self, this: usize, c: usize, last: usize) -> bool {
        let k = &self.cons[c];
        self.vars[k.right].block == this && k.active && last != k.right
    }

    /// `Block::compute_dfdv`.
    fn compute_dfdv(&mut self, this: usize, v: usize, u: usize, min_lm: &mut usize) -> f64 {
        let mut dfdv = self.vars[v].weight * (self.position(v) - self.vars[v].desired_position);
        for i in 0..self.vars[v].out.len() {
            let c = self.vars[v].out[i];
            if self.can_follow_right(this, c, u) {
                let r = self.cons[c].right;
                let lm = self.compute_dfdv(this, r, v, min_lm);
                self.cons[c].lm = lm;
                dfdv += lm;
                if *min_lm == NONE || self.cons[c].lm < self.cons[*min_lm].lm {
                    *min_lm = c;
                }
            }
        }
        for i in 0..self.vars[v].in_.len() {
            let c = self.vars[v].in_[i];
            if self.can_follow_left(this, c, u) {
                let l = self.cons[c].left;
                let lm = -self.compute_dfdv(this, l, v, min_lm);
                self.cons[c].lm = lm;
                dfdv -= lm;
                if *min_lm == NONE || self.cons[c].lm < self.cons[*min_lm].lm {
                    *min_lm = c;
                }
            }
        }
        dfdv
    }

    /// `Block::compute_dfdv_between`: `(dfdv, m)`; `dir` 0 NONE, 1 LEFT, 2 RIGHT.
    fn compute_dfdv_between(
        &mut self,
        this: usize,
        mut r: usize,
        v: usize,
        u: usize,
        dir: u8,
        mut changed_direction: bool,
    ) -> (f64, usize) {
        let mut dfdv = self.vars[v].weight * (self.position(v) - self.vars[v].desired_position);
        let mut m = NONE;
        for i in 0..self.vars[v].in_.len() {
            let c = self.vars[v].in_[i];
            if self.can_follow_left(this, c, u) {
                if dir == 2 {
                    changed_direction = true;
                }
                let cl = self.cons[c].left;
                if cl == r {
                    r = NONE;
                    m = c;
                }
                let p = self.compute_dfdv_between(this, r, cl, v, 1, changed_direction);
                self.cons[c].lm = -p.0;
                dfdv -= self.cons[c].lm;
                if r != NONE && p.1 != NONE {
                    m = p.1;
                }
            }
        }
        for i in 0..self.vars[v].out.len() {
            let c = self.vars[v].out[i];
            if self.can_follow_right(this, c, u) {
                if dir == 1 {
                    changed_direction = true;
                }
                let cr = self.cons[c].right;
                if cr == r {
                    r = NONE;
                    m = c;
                }
                let p = self.compute_dfdv_between(this, r, cr, v, 2, changed_direction);
                self.cons[c].lm = p.0;
                dfdv += self.cons[c].lm;
                if r != NONE && p.1 != NONE {
                    m = if changed_direction && self.cons[c].lm < self.cons[p.1].lm {
                        c
                    } else {
                        p.1
                    };
                }
            }
        }
        (dfdv, m)
    }

    /// `Block::reset_active_lm`.
    fn reset_active_lm(&mut self, this: usize, v: usize, u: usize) {
        for i in 0..self.vars[v].out.len() {
            let c = self.vars[v].out[i];
            if self.can_follow_right(this, c, u) {
                self.cons[c].lm = 0.0;
                let r = self.cons[c].right;
                self.reset_active_lm(this, r, v);
            }
        }
        for i in 0..self.vars[v].in_.len() {
            let c = self.vars[v].in_[i];
            if self.can_follow_left(this, c, u) {
                self.cons[c].lm = 0.0;
                let l = self.cons[c].left;
                self.reset_active_lm(this, l, v);
            }
        }
    }

    /// `Block::findMinLM`.
    fn find_min_lm(&mut self, this: usize) -> usize {
        let mut min_lm = NONE;
        let front = self.blocks[this].vars[0];
        self.reset_active_lm(this, front, NONE);
        self.compute_dfdv(this, front, NONE, &mut min_lm);
        min_lm
    }

    /// `Block::findMinLMBetween`.
    fn find_min_lm_between(&mut self, this: usize, lv: usize, rv: usize) -> usize {
        let front = self.blocks[this].vars[0];
        self.reset_active_lm(this, front, NONE);
        self.compute_dfdv_between(this, rv, lv, NONE, 0, false).1
    }

    /// `Block::populateSplitBlock`.
    fn populate_split_block(&mut self, this: usize, b: usize, v: usize, u: usize) {
        self.add_variable(b, v);
        for i in 0..self.vars[v].in_.len() {
            let c = self.vars[v].in_[i];
            if self.can_follow_left(this, c, u) {
                let l = self.cons[c].left;
                self.populate_split_block(this, b, l, v);
            }
        }
        for i in 0..self.vars[v].out.len() {
            let c = self.vars[v].out[i];
            if self.can_follow_right(this, c, u) {
                let r = self.cons[c].right;
                self.populate_split_block(this, b, r, v);
            }
        }
    }

    /// `Block::split(l, r, c)`: `(l, r)`.
    fn split(&mut self, this: usize, c: usize) -> (usize, usize) {
        self.cons[c].active = false;
        let l = self.new_block(None);
        let (cl, cr) = (self.cons[c].left, self.cons[c].right);
        self.populate_split_block(this, l, cl, cr);
        let r = self.new_block(None);
        self.populate_split_block(this, r, cr, cl);
        (l, r)
    }

    /// `Block::splitBetween(vl, vr, lb, rb)`: `(c, lb, rb)`. `None` when
    /// `findMinLMBetween` finds no active constraint between `vl` and `vr`;
    /// C passes that NULL on to `Block::split`, which dereferences it (#185).
    fn split_between(
        &mut self,
        this: usize,
        vl: usize,
        vr: usize,
    ) -> Option<(usize, usize, usize)> {
        let c = self.find_min_lm_between(this, vl, vr);
        if c == NONE {
            return None;
        }
        let (lb, rb) = self.split(this, c);
        self.blocks[this].deleted = true;
        Some((c, lb, rb))
    }

    /// `Block::cost`.
    fn block_cost(&self, b: usize) -> f64 {
        let mut c = 0.0f64;
        for &v in &self.blocks[b].vars {
            let var = &self.vars[v];
            let diff = self.position(v) - var.desired_position;
            c = math::fma(var.weight * diff, diff, c);
        }
        c
    }

    // ---- Blocks ----

    fn bs_insert(&mut self, b: usize) {
        self.bs.insert((self.blocks[b].addr, b));
    }

    /// `Blocks::cleanup`.
    fn cleanup(&mut self) {
        let blocks = &self.blocks;
        self.bs.retain(|&(_, b)| !blocks[b].deleted);
    }

    /// `Blocks::cost`.
    fn bs_cost(&self) -> f64 {
        let mut c = 0.0;
        for &(_, b) in &self.bs {
            c += self.block_cost(b);
        }
        c
    }

    // ---- IncVPSC ----

    /// `newIncVPSC(n, vs, m, cs)`: builds `Blocks(n, vs)` (one block per
    /// variable, `blockTimeCtr = 0`) and marks the constraints inactive.
    /// Replaces any previous instance (`deleteVPSC`).
    pub fn new_inc_vpsc(&mut self, vs: &[usize], cs: &[usize]) {
        self.delete_vpsc();
        self.block_time_ctr = 0;
        for &v in vs {
            let b = self.new_block(Some(v));
            self.bs_insert(b);
        }
        self.cs = cs.to_vec();
        self.inactive = cs.to_vec();
        for &c in cs {
            self.cons[c].active = false;
        }
    }

    /// `deleteVPSC`: `~Blocks` frees the blocks and resets `blockTimeCtr`.
    pub fn delete_vpsc(&mut self) {
        self.bs.clear();
        self.block_time_ctr = 0;
        self.cs.clear();
        self.inactive.clear();
    }

    /// `IncVPSC::solve`.
    pub fn solve(&mut self) -> Result<(), VpscError> {
        let mut cost = self.bs_cost();
        loop {
            let lastcost = cost;
            self.satisfy()?;
            self.split_blocks();
            cost = self.bs_cost();
            // C's `while (fabs(lastcost - cost) > 0.0001)`: a NaN cost (a node at infinity)
            // ends the loop, where `<= 0.0001` would never hold.
            let changed = (lastcost - cost).abs() > 0.0001;
            if !changed {
                break;
            }
        }
        Ok(())
    }

    /// Not in C: frees the blocks that no variable belongs to and that are
    /// not in `Blocks`, for `new_block` to reuse. C `delete`s the deleted
    /// blocks in `Blocks::cleanup` and `~Blocks`, and leaks the ones that
    /// `satisfy` merges away without inserting; nothing reads either again.
    /// Without this, every block ever made stayed in `blocks`, and a run of
    /// cycle errors (10001 splits each) used gigabytes (#197). Block order
    /// comes from `addr`, not the index, so reusing an index changes nothing.
    fn free_unused_blocks(&mut self) {
        let mut used = vec![false; self.blocks.len()];
        for v in &self.vars {
            if v.block != NONE {
                used[v.block] = true;
            }
        }
        for &(_, b) in &self.bs {
            used[b] = true;
        }
        self.free_blocks.clear();
        for (b, &u) in used.iter().enumerate() {
            if !u {
                self.blocks[b].vars = Vec::new();
                self.free_blocks.push(b);
            }
        }
    }

    /// `IncVPSC::satisfy`.
    pub fn satisfy(&mut self) -> Result<(), VpscError> {
        self.free_unused_blocks();
        self.split_blocks();
        let mut split_ctr: i64 = 0;
        let mut v = NONE;
        while self.most_violated(&mut v) < -EPS {
            let lb = self.vars[self.cons[v].left].block;
            let rb = self.vars[self.cons[v].right].block;
            if lb != rb {
                self.merge2(lb, rb, v);
            } else {
                let c = split_ctr;
                split_ctr += 1;
                if c > 10000 {
                    return Err(VpscError::CYCLE);
                }
                let (vl, vr) = (self.cons[v].left, self.cons[v].right);
                let Some((sc, lb, rb)) = self.split_between(lb, vl, vr) else {
                    return Err(VpscError("no constraint to split on"));
                };
                self.inactive.push(sc);
                self.merge2(lb, rb, v);
                self.bs_insert(lb);
            }
        }
        self.cleanup();
        for i in 0..self.cs.len() {
            if self.slack(self.cs[i]) < -EPS {
                return Err(VpscError("Unsatisfied constraint"));
            }
        }
        Ok(())
    }

    /// `IncVPSC::moveBlocks`.
    fn move_blocks(&mut self) {
        let keys: Vec<usize> = self.bs.iter().map(|&(_, b)| b).collect();
        for b in keys {
            let wp = self.desired_weighted_position(b);
            self.blocks[b].wposn = wp;
            self.blocks[b].posn = wp / self.blocks[b].weight;
        }
    }

    /// `IncVPSC::splitBlocks`. Blocks inserted while iterating are visited
    /// when they sort after the current one, as with `std::set`.
    fn split_blocks(&mut self) {
        self.move_blocks();
        self.split_cnt = 0;
        let mut cur: Option<(u64, usize)> = None;
        loop {
            let next = match cur {
                None => self.bs.iter().next().copied(),
                Some(k) => self
                    .bs
                    .range((std::ops::Bound::Excluded(k), std::ops::Bound::Unbounded))
                    .next()
                    .copied(),
            };
            let Some(key) = next else {
                break;
            };
            cur = Some(key);
            let b = key.1;
            let v = self.find_min_lm(b);
            if v != NONE && self.cons[v].lm < -EPS {
                self.split_cnt += 1;
                let b2 = self.vars[self.cons[v].left].block;
                let pos = self.blocks[b2].posn;
                let (l, r) = self.split(b2, v);
                self.blocks[l].posn = pos;
                self.blocks[r].posn = pos;
                self.blocks[l].wposn = self.blocks[l].posn * self.blocks[l].weight;
                self.blocks[r].wposn = self.blocks[r].posn * self.blocks[r].weight;
                self.bs_insert(l);
                self.bs_insert(r);
                self.blocks[b2].deleted = true;
                self.inactive.push(v);
            }
        }
        self.cleanup();
    }

    /// `IncVPSC::mostViolated(inactive, v)`.
    fn most_violated(&mut self, v: &mut usize) -> f64 {
        let mut min_slack = f64::MAX;
        let mut delete_point = None;
        for i in 0..self.inactive.len() {
            let c = self.inactive[i];
            let slack = self.slack(c);
            if slack < min_slack {
                min_slack = slack;
                *v = c;
                delete_point = Some(i);
            }
        }
        if let Some(i) = delete_point
            && min_slack < -EPS
        {
            let last = *self.inactive.last().unwrap();
            self.inactive[i] = last;
            self.inactive.pop();
        }
        min_slack
    }
}

// ---- generate-constraints.cpp ----

/// `Node` of the scanline (`NodeSet` is ordered by `pos`, then address).
struct SNode {
    v: usize,
    r: Rectangle,
    pos: f64,
    first_above: usize,
    first_below: usize,
    left: BTreeSet<NodeKey>,
    right: BTreeSet<NodeKey>,
}

/// `CmpNodePos` key: position, then the node's address, here its creation order in the
/// `generateX/YConstraints` call (`docs/porting/neato.md`, VPSC, "Ties by address"). The
/// position is finite: [`rectangles`] refuses every box C's `Rectangle` asserts on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NodeKey(f64, usize);

impl Eq for NodeKey {}

impl PartialOrd for NodeKey {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for NodeKey {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        if self.0 < o.0 {
            std::cmp::Ordering::Less
        } else if o.0 < self.0 {
            std::cmp::Ordering::Greater
        } else {
            self.1.cmp(&o.1)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Event {
    open: bool,
    v: usize,
    pos: f64,
}

fn compare_events(ea: &Event, eb: &Event) -> bool {
    if ea.v == eb.v {
        // open before close for the same rectangle
        return ea.open && !eb.open;
    }
    if ea.pos > eb.pos {
        return false;
    }
    ea.pos < eb.pos
}

fn key(nodes: &[SNode], i: usize) -> NodeKey {
    NodeKey(nodes[i].pos, i)
}

fn get_left_neighbours(
    nodes: &[SNode],
    scanline: &BTreeSet<NodeKey>,
    v: usize,
) -> BTreeSet<NodeKey> {
    let mut leftv = BTreeSet::new();
    for &k in scanline.range(..key(nodes, v)).rev() {
        let u = k.1;
        let ox = nodes[u].r.overlap_x(&nodes[v].r);
        if ox <= 0.0 {
            leftv.insert(k);
            return leftv;
        }
        if ox <= nodes[u].r.overlap_y(&nodes[v].r) {
            leftv.insert(k);
        }
    }
    leftv
}

fn get_right_neighbours(
    nodes: &[SNode],
    scanline: &BTreeSet<NodeKey>,
    v: usize,
) -> BTreeSet<NodeKey> {
    let mut rightv = BTreeSet::new();
    let kv = key(nodes, v);
    for &k in scanline.range((std::ops::Bound::Excluded(kv), std::ops::Bound::Unbounded)) {
        let u = k.1;
        let ox = nodes[u].r.overlap_x(&nodes[v].r);
        if ox <= 0.0 {
            rightv.insert(k);
            return rightv;
        }
        if ox <= nodes[u].r.overlap_y(&nodes[v].r) {
            rightv.insert(k);
        }
    }
    rightv
}

fn scan_neighbours(nodes: &mut [SNode], scanline: &BTreeSet<NodeKey>, v: usize) {
    let kv = key(nodes, v);
    if let Some(&k) = scanline.range(..kv).next_back() {
        let u = k.1;
        nodes[v].first_above = u;
        nodes[u].first_below = v;
    }
    if let Some(&k) = scanline
        .range((std::ops::Bound::Excluded(kv), std::ops::Bound::Unbounded))
        .next()
    {
        let u = k.1;
        nodes[v].first_below = u;
        nodes[u].first_above = v;
    }
}

/// `generateXConstraints(rs, vars, cs, useNeighbourLists)`: returns the new
/// constraints in creation order.
pub fn generate_x_constraints(
    s: &mut Solver,
    rs: &[Rectangle],
    vars: &[usize],
    use_neighbour_lists: bool,
) -> Vec<usize> {
    let mut nodes: Vec<SNode> = Vec::with_capacity(rs.len());
    let mut events: Vec<Event> = Vec::with_capacity(2 * rs.len());
    for (i, r) in rs.iter().enumerate() {
        s.vars[vars[i]].desired_position = r.centre_x();
        nodes.push(SNode {
            v: vars[i],
            r: *r,
            pos: r.centre_x(),
            first_above: NONE,
            first_below: NONE,
            left: BTreeSet::new(),
            right: BTreeSet::new(),
        });
        events.push(Event {
            open: true,
            v: i,
            pos: r.min_y,
        });
        events.push(Event {
            open: false,
            v: i,
            pos: r.max_y,
        });
    }
    libcxx::sort(&mut events, compare_events);
    let mut scanline: BTreeSet<NodeKey> = BTreeSet::new();
    let mut constraints = Vec::new();
    for e in &events {
        let v = e.v;
        if e.open {
            scanline.insert(key(&nodes, v));
            if use_neighbour_lists {
                let left = get_left_neighbours(&nodes, &scanline, v);
                let right = get_right_neighbours(&nodes, &scanline, v);
                // setNeighbours
                let kv = key(&nodes, v);
                for k in &left {
                    nodes[k.1].right.insert(kv);
                }
                for k in &right {
                    nodes[k.1].left.insert(kv);
                }
                nodes[v].left = left;
                nodes[v].right = right;
            } else {
                scan_neighbours(&mut nodes, &scanline, v);
            }
        } else {
            if use_neighbour_lists {
                let kv = key(&nodes, v);
                let left: Vec<NodeKey> = nodes[v].left.iter().copied().collect();
                for k in left {
                    let u = k.1;
                    let sep = (nodes[v].r.width() + nodes[u].r.width()) / 2.0;
                    constraints.push(s.new_constraint(nodes[u].v, nodes[v].v, sep));
                    nodes[u].right.remove(&kv);
                }
                let right: Vec<NodeKey> = nodes[v].right.iter().copied().collect();
                for k in right {
                    let u = k.1;
                    let sep = (nodes[v].r.width() + nodes[u].r.width()) / 2.0;
                    constraints.push(s.new_constraint(nodes[v].v, nodes[u].v, sep));
                    nodes[u].left.remove(&kv);
                }
            } else {
                let (l, r) = (nodes[v].first_above, nodes[v].first_below);
                if l != NONE {
                    let sep = (nodes[v].r.width() + nodes[l].r.width()) / 2.0;
                    constraints.push(s.new_constraint(nodes[l].v, nodes[v].v, sep));
                    nodes[l].first_below = nodes[v].first_below;
                }
                if r != NONE {
                    let sep = (nodes[v].r.width() + nodes[r].r.width()) / 2.0;
                    constraints.push(s.new_constraint(nodes[v].v, nodes[r].v, sep));
                    nodes[r].first_above = nodes[v].first_above;
                }
            }
            scanline.remove(&key(&nodes, v));
        }
    }
    constraints
}

/// `generateYConstraints(rs, vars, cs)`.
pub fn generate_y_constraints(s: &mut Solver, rs: &[Rectangle], vars: &[usize]) -> Vec<usize> {
    let mut nodes: Vec<SNode> = Vec::with_capacity(rs.len());
    let mut events: Vec<Event> = Vec::with_capacity(2 * rs.len());
    for (i, r) in rs.iter().enumerate() {
        s.vars[vars[i]].desired_position = r.centre_y();
        nodes.push(SNode {
            v: vars[i],
            r: *r,
            pos: r.centre_y(),
            first_above: NONE,
            first_below: NONE,
            left: BTreeSet::new(),
            right: BTreeSet::new(),
        });
        events.push(Event {
            open: true,
            v: i,
            pos: r.min_x,
        });
        events.push(Event {
            open: false,
            v: i,
            pos: r.max_x,
        });
    }
    libcxx::sort(&mut events, compare_events);
    let mut scanline: BTreeSet<NodeKey> = BTreeSet::new();
    let mut constraints = Vec::new();
    for e in &events {
        let v = e.v;
        if e.open {
            scanline.insert(key(&nodes, v));
            scan_neighbours(&mut nodes, &scanline, v);
        } else {
            let (l, r) = (nodes[v].first_above, nodes[v].first_below);
            if l != NONE {
                let sep = (nodes[v].r.height() + nodes[l].r.height()) / 2.0;
                constraints.push(s.new_constraint(nodes[l].v, nodes[v].v, sep));
                nodes[l].first_below = nodes[v].first_below;
            }
            if r != NONE {
                let sep = (nodes[v].r.height() + nodes[r].r.height()) / 2.0;
                constraints.push(s.new_constraint(nodes[v].v, nodes[r].v, sep));
                nodes[r].first_above = nodes[v].first_above;
            }
            scanline.remove(&key(&nodes, v));
        }
    }
    constraints
}

/// `boxf` as passed to `genXConstraints`/`genYConstraints`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoxF {
    pub llx: f64,
    pub lly: f64,
    pub urx: f64,
    pub ury: f64,
}

/// The rectangles of `bb`, or `None` where C's `Rectangle` (`x <= X`, `y <= Y`) or `Node`
/// (`width < 1e40`) constructor asserts: a NaN, infinite or huge position, which C aborts on
/// and on which the solver would never settle (`overlap=vpsc` with a pin at `nan` or 1e300).
fn rectangles(bb: &[BoxF]) -> Option<Vec<Rectangle>> {
    bb.iter()
        .map(|b| {
            let r = Rectangle::new(b.llx, b.urx, b.lly, b.ury);
            (b.llx <= b.urx && b.lly <= b.ury && r.width() < 1e40).then_some(r)
        })
        .collect()
}

/// `genXConstraints(n, bb, vs, cs, transitiveClosure)`; `None` where C asserts.
pub fn gen_x_constraints(
    s: &mut Solver,
    bb: &[BoxF],
    vs: &[usize],
    transitive_closure: bool,
) -> Option<Vec<usize>> {
    let rs = rectangles(bb)?;
    Some(generate_x_constraints(s, &rs, vs, transitive_closure))
}

/// `genYConstraints(n, bb, vs, cs)`; `None` where C asserts.
pub fn gen_y_constraints(s: &mut Solver, bb: &[BoxF], vs: &[usize]) -> Option<Vec<usize>> {
    let rs = rectangles(bb)?;
    Some(generate_y_constraints(s, &rs, vs))
}

#[cfg(test)]
mod tests {
    use super::*;

    // #225: C breaks a position tie by the `Node` pointer; the port by creation order, the
    // order a fresh heap's addresses would have.
    #[test]
    fn node_key_breaks_position_ties_in_creation_order() {
        let mut set = BTreeSet::new();
        for k in [
            NodeKey(1.0, 3),
            NodeKey(0.5, 7),
            NodeKey(1.0, 1),
            NodeKey(1.0, 2),
        ] {
            set.insert(k);
        }
        let order: Vec<usize> = set.iter().map(|k| k.1).collect();
        assert_eq!(order, [7, 1, 2, 3]);
    }

    /// #197: `a + 1 <= b` and `b + 1 <= a` cannot both hold, so `satisfy`
    /// fails with a cycle error after 10001 splits. The ipsep stress loop
    /// builds a new `IncVPSC` and satisfies again on every iteration. Without
    /// `free_unused_blocks`, every block of every round stayed in `blocks`.
    #[test]
    fn failed_satisfy_reuses_unused_blocks() {
        let mut s = Solver::new();
        let a = s.new_variable(0, 0.0, 1.0);
        let b = s.new_variable(1, 0.0, 1.0);
        let cs = [s.new_constraint(a, b, 1.0), s.new_constraint(b, a, 1.0)];
        let mut first = 0;
        for round in 0..20 {
            s.new_inc_vpsc(&[a, b], &cs);
            assert_eq!(s.satisfy(), Err(VpscError::CYCLE));
            if round == 0 {
                first = s.blocks.len();
            }
        }
        assert!(
            s.blocks.len() <= 2 * first,
            "{first} blocks after one round, {} after 20",
            s.blocks.len()
        );
    }
}
