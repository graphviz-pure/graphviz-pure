import Mathlib.Logic.Function.Basic
import Mathlib.Tactic

/-!
# `transpose` on the rank-pair memo (`docs/perf/mincross-scans.md` §3)

The Rust shortcut is `dotgen/mincross/scan.rs` (`build_scan`, `scan_rank`, `forget_pairs`,
`publish_scans`); the mirror of C is `transpose_step` and `transpose_loop` in `dotgen/mincross.rs`.
This file states the page's invariants over a Lean model of both and proves §3.3: the scan makes
the same exchange decisions from the same counts as C's loop, for any number of rounds, and leaves
the same state once it publishes. Rounds are counted because `transpose` need not stop (#682).

| prose | here |
| :-- | :-- |
| (W) | `Inv.w`, `holds_iff_w`, `buildScan_inv`, preserved by `exchange_inv` |
| (F) | `count_congr`, `pair_swap` |
| (E) | `exchange_sameSign` |
| (M) | `Inv.m`, preserved by `exchange_inv` (with `forget_le`, `forget_hits`) and `stepS_inv` |
| §3.2 bullets | the cases of `exchange_m` |
| §3.3 | `stepS_sim`, `loop_sim`, `transpose_same` |
| §3.3, the three deferred items | `Rel` (`cand`, `valid`, `buf`, `arena`), `publish_eq` |
| ports, NaN (#751) | `View.portGt`, `crosses_congr`, `F64.gt`, `transpose_same_f64`, `transpose_same_float` |

§2 (`reorder_settled`) is not formalised here.

## Model vs Rust

* Nodes are the compact slots (`N`); `View` holds what a `transpose` does not change. Edge lists
  are `EdgeEnd` lists (other end, penalty, port x); the counting loops read orders live, so the
  crossing copies and `refresh`/`live` are not modelled.
* `in_cross`/`out_cross` are one loop, `View.crossLoop`, with the accumulation step a parameter
  (`Arith`): `i64` products for `in_cross`, wrapping `i32` for `out_cross`, wrapping `i64` for `c0`,
  `c1`, `rv` and `delta` (`Arith.rust`). The theorems hold for any choice.
* The order difference `t = e1.order - inv` is exact (`ℤ`). Rust subtracts `i32`s; orders are rank
  positions or stale positions, far from overflow.
* `left2right` is a fixed function asked only when the view is constrained, as
  `transpose_step::<CONSTRAINED>` does; that an unconstrained view's `left2right` is false (§2.1)
  is not proved here.
* C's loop runs a bounded number of rounds (`loopC`); the theorem holds for every bound, which is
  how it covers a `transpose` that never ends (#682).
* `buildScan` starts the arena orders equal to the compact orders, which `prepare_mincross` and
  `write_crossing_orders` maintain; `positioned_rank`'s cached verdict is recomputed (`rowOk`).
* The memo is a function of `(rank, position)`; `RankPair::slot` is `slot`.
* Ports are an abstract type `P` with a comparison `View.portGt : P → P → Bool`, standing for
  Rust's `e1.port_x > e2.port_x` on `f64`. The theorems assume nothing of it (issue #751; see
  "Ports"): not irreflexivity, not transitivity, not totality. So a NaN port, which makes `>` false
  both ways, is inside them, and no caller invariant "ports are finite" is needed. `F64` and
  `transpose_same_f64` instantiate `P` with IEEE `>` (`F64.gt`, false if either side is NaN);
  `transpose_same_float` does it with Lean's `Float`. What is still assumed is that C's loop and
  the scan evaluate the same comparison on the same port values, which holds because both call
  `in_cross`/`out_cross` on the same `f64`s.
-/

set_option linter.unusedSectionVars false

namespace GvpProofs.Dot.MincrossScans

open Function

variable {N P : Type} [DecidableEq N]

/-! ## The model -/

/-- One edge seen from a node: `ResolvedCrossing` without its order, which is read live. -/
structure EdgeEnd (N P : Type) where
  other : N
  penalty : ℤ
  port : P

/-- The arithmetic of the counting loops. The theorems hold for any of it, which is §4's point:
wrapped or not, the same inputs give the same integer. `Arith.rust` below is the shipped one. -/
structure Arith where
  /-- One step of `in_cross`: `acc + a * b`. -/
  inAdd : ℤ → ℤ → ℤ → ℤ
  /-- One step of `out_cross`: `acc + a * b` on `i32`. -/
  outAdd : ℤ → ℤ → ℤ → ℤ
  /-- `wrapping_add` on `i64`. -/
  add : ℤ → ℤ → ℤ
  /-- `wrapping_sub` on `i64`. -/
  sub : ℤ → ℤ → ℤ

/-- What does not change during a `transpose`: edge lists, penalties, ports, window sizes,
`left2right` (read from `low`, clusters and the flat matrix, never from orders). -/
structure View (N P : Type) where
  rankOf : N → ℕ
  inE : N → List (EdgeEnd N P)
  outE : N → List (EdgeEnd N P)
  minr : ℕ
  maxr : ℕ
  /-- Where the view's window of rank `r` starts in the root's rank (`ScanRank::base`). -/
  base : ℕ → ℤ
  /-- `rank.n`. -/
  n : ℕ → ℕ
  /-- `rank_n(g, r + 1) > 0`. -/
  hasOut : ℕ → Bool
  left2right : N → N → Bool
  constrained : Bool
  reverse : Bool
  arith : Arith
  /-- Rust's `e1.port_x > e2.port_x`. The proof asks nothing of it (no irreflexivity, no
  transitivity, no totality): see "Ports" below. -/
  portGt : P → P → Bool
  /-- `store.meta`: every compact slot. -/
  slots : List N

namespace View

variable (V : View N P)

/-- `minr ≤ r ≤ maxr`. -/
def InView (r : ℕ) : Prop := V.minr ≤ r ∧ r ≤ V.maxr

instance (r : ℕ) : Decidable (V.InView r) := by unfold InView; infer_instance

/-- `for r in minr..=maxr`. -/
def ranks : List ℕ := List.range' V.minr (V.maxr + 1 - V.minr)

theorem mem_ranks {r : ℕ} : r ∈ V.ranks ↔ V.InView r := by
  simp only [ranks, List.mem_range'_1, InView]; omega

/-- C's comparison in `in_cross` and `out_cross`: `t > 0 || (t == 0 && port > port)`, with the
port comparison `gt` left abstract (`View.portGt`). -/
def crosses (gt : P → P → Bool) (ord : N → ℤ) (e1 e2 : EdgeEnd N P) : Prop :=
  ord e1.other - ord e2.other > 0 ∨ (ord e1.other - ord e2.other = 0 ∧ gt e1.port e2.port = true)

instance (gt : P → P → Bool) (ord : N → ℤ) (e1 e2 : EdgeEnd N P) :
    Decidable (crosses gt ord e1 e2) := by
  unfold crosses; infer_instance

/-- The two nested loops of `in_cross` and `out_cross`: the outer over `w`'s edges. -/
def crossLoop (gt : P → P → Bool) (add : ℤ → ℤ → ℤ → ℤ) (ord : N → ℤ) (vs ws : List (EdgeEnd N P)) : ℤ :=
  ws.foldl (fun acc e2 => vs.foldl
    (fun acc e1 => if crosses gt ord e1 e2 then add acc e1.penalty e2.penalty else acc) acc) 0

/-- `c0` for the pair `(v, w)` of rank `r`, and `c1` when called with `w, v`: `in_cross` only if
`r > 0`, `out_cross` only if rank `r + 1` is not empty. -/
def count (ord : N → ℤ) (r : ℕ) (v w : N) : ℤ :=
  let c := if r > 0 then V.arith.add 0 (crossLoop V.portGt V.arith.inAdd ord (V.inE v) (V.inE w)) else 0
  if V.hasOut r then V.arith.add c (crossLoop V.portGt V.arith.outAdd ord (V.outE v) (V.outE w)) else c

/-- `(c0, c1)`. -/
def pair (ord : N → ℤ) (r : ℕ) (v w : N) : ℤ × ℤ := (V.count ord r v w, V.count ord r w v)

/-- `c1 < c0 || (c0 > 0 && reverse && c1 == c0)`. -/
def exch (c : ℤ × ℤ) : Bool :=
  decide (c.2 < c.1) || (decide (c.1 > 0) && V.reverse && decide (c.2 = c.1))

/-- What an exchange in rank `r` does to `candidate`. -/
def markCand (r : ℕ) (f : ℕ → Bool) : ℕ → Bool := fun r' =>
  if r' = r then true
  else if r > V.minr ∧ r' = r - 1 then true
  else if r < V.maxr ∧ r' = r + 1 then true
  else f r'

/-- What an exchange in rank `r` does to the root's `valid`. -/
def markValid (r : ℕ) (f : ℕ → Bool) : ℕ → Bool := fun r' =>
  if r' = r then false
  else if r > V.minr ∧ r' = r - 1 then false
  else if r < V.maxr ∧ r' = r + 1 then false
  else f r'

theorem markCand_idem (r : ℕ) (f : ℕ → Bool) : V.markCand r (V.markCand r f) = V.markCand r f := by
  funext r'; simp only [markCand]; split_ifs <;> rfl

theorem markValid_idem (r : ℕ) (f : ℕ → Bool) :
    V.markValid r (V.markValid r f) = V.markValid r f := by
  funext r'; simp only [markValid]; split_ifs <;> rfl

end View

/-- A read of `transpose_step`, as `scan::check::Read` logs it, with the decision taken on it. -/
structure Event where
  r : ℕ
  i : ℕ
  c : ℤ × ℤ
  exchanged : Bool
  deriving DecidableEq

/-! ## C's `transpose` -/

/-- C's state. `buf r p` is `GD_rank(Root)[r].v[p]`: the view's window is `buf r (base r + i)`. -/
structure CState (N : Type) where
  order : N → ℤ
  buf : ℕ → ℤ → N
  cand : ℕ → Bool
  valid : ℕ → Bool
  rv : ℤ
  log : List Event

section C

variable (V : View N P)

/-- C's `exchange(v, w)`, reading the two orders and writing through them. -/
def exchangeC (c : CState N) (v w : N) : CState N :=
  let r := V.rankOf v
  let vi := c.order v
  let wi := c.order w
  { c with
    order := update (update c.order v wi) w vi
    buf := update c.buf r (update (update (c.buf r) wi v) vi w) }

/-- One pass of C's `for` loop in `transpose_step`. -/
def stepC (r : ℕ) (c : CState N) (i : ℕ) : CState N :=
  let v := c.buf r (V.base r + i)
  let w := c.buf r (V.base r + i + 1)
  if V.constrained = true ∧ V.left2right v w = true then c else
  let p := V.pair c.order r v w
  let x := V.exch p
  let c' := { c with log := c.log ++ [⟨r, i, p, x⟩] }
  if x = true then
    { exchangeC V c' v w with
      rv := V.arith.add c.rv (V.arith.sub p.1 p.2)
      valid := V.markValid r c.valid
      cand := V.markCand r c.cand }
  else c'

/-- `transpose_step(g, r, reverse)`; its return value is the final `rv`. -/
def stepAllC (r : ℕ) (c : CState N) : CState N :=
  (List.range (V.n r - 1)).foldl (stepC V r) { c with cand := update c.cand r false, rv := 0 }

/-- One round of the `do … while (delta >= 1)` in `transpose`: the state and `delta`. -/
def roundC (c : CState N) : CState N × ℤ :=
  V.ranks.foldl (fun (st : CState N × ℤ) r =>
    if st.1.cand r = true then
      let c' := stepAllC V r st.1
      (c', V.arith.add st.2 c'.rv)
    else st) (c, 0)

/-- At most `k` rounds of C's loop: the state, and whether the loop ended. -/
def loopC : ℕ → CState N → CState N × Bool
  | 0, c => (c, false)
  | k + 1, c =>
    let st := roundC V c
    if st.2 < 1 then (st.1, true) else loopC k st.1

end C

/-! ## The scan -/

/-- `RankPair`'s `known`, `c0`, `c1` per rank and position. -/
abbrev Memo := ℕ → ℕ → Option (ℤ × ℤ)

/-- Writes one record of the memo. -/
def setMemo (m : Memo) (r i : ℕ) (x : Option (ℤ × ℤ)) : Memo :=
  fun r' i' => if r' = r ∧ i' = i then x else m r' i'

/-- The scan's state. `order` is `store.order`, `slot` is `RankPair::slot`; `arena` and `buf` are
`NodeInfo::order` and the rank buffers, which it writes once, in `publish`. -/
structure SState (N : Type) where
  order : N → ℤ
  slot : ℕ → ℕ → N
  memo : Memo
  moved : ℕ → Bool
  exchanged : Bool
  cand : ℕ → Bool
  valid : ℕ → Bool
  rv : ℤ
  log : List Event
  arena : N → ℤ
  buf : ℕ → ℤ → N

section Scan

variable (V : View N P)

/-- `cell.known = false` for the pair starting at `first`, if the window of `rk` has one there. -/
def clearAt (rk : ℕ) (first : ℤ) (m : Memo) : Memo :=
  if 0 ≤ first - V.base rk ∧ first - V.base rk < (V.n rk : ℤ) - 1 then
    setMemo m rk (first - V.base rk).toNat none
  else m

/-- `forget_pairs`: for every edge `x` of `v` and `y` of `w` on one side, forgets the pair of
rank `rk` that `x.other` and `y.other` form, when their orders are adjacent. -/
def forget (ord : N → ℤ) (rk : ℕ) (xs ys : List (EdgeEnd N P)) (m : Memo) : Memo :=
  xs.foldl (fun m x => ys.foldl (fun m y =>
    let ox := ord x.other
    let oy := ord y.other
    if ox - oy = 1 then clearAt V rk oy m
    else if ox - oy = -1 then clearAt V rk ox m
    else m) m) m

/-- Swaps positions `i` and `i + 1` of rank `r`. -/
def swapSlot (slot : ℕ → ℕ → N) (r i : ℕ) : ℕ → ℕ → N := fun r' i' =>
  if r' = r then
    if i' = i then slot r (i + 1) else if i' = i + 1 then slot r i else slot r i'
  else slot r' i'

/-- The memo after the exchange at position `i` of rank `r`, whose counts were `p`. -/
def memoAfter (ord' : N → ℤ) (r i : ℕ) (v w : N) (p : ℤ × ℤ) (m : Memo) : Memo :=
  let m1 := setMemo (setMemo m r i (some (p.2, p.1))) r (i + 1) none
  let m2 := if i > 0 then setMemo m1 r (i - 1) none else m1
  let m3 := if r > V.minr then forget V ord' (r - 1) (V.inE v) (V.inE w) m2 else m2
  if r < V.maxr then forget V ord' (r + 1) (V.outE v) (V.outE w) m3 else m3

/-- The counts `scan_rank` uses: from the memo if `known`, else computed. -/
def readCounts (s : SState N) (r i : ℕ) : ℤ × ℤ :=
  match s.memo r i with
  | some p => p
  | none => V.pair s.order r (s.slot r i) (s.slot r (i + 1))

/-- One pass of the `for` loop in `scan_rank`. The flags wait for the end of the scan. -/
def stepS (r : ℕ) (s : SState N) (i : ℕ) : SState N :=
  let v := s.slot r i
  let w := s.slot r (i + 1)
  if V.constrained = true ∧ V.left2right v w = true then s else
  let p := readCounts V s r i
  let x := V.exch p
  let log := s.log ++ [⟨r, i, p, x⟩]
  if x = true then
    let place := V.base r + i
    let ord' := update (update s.order v (place + 1)) w place
    { s with
      order := ord'
      slot := swapSlot s.slot r i
      memo := memoAfter V ord' r i v w p s.memo
      rv := V.arith.add s.rv (V.arith.sub p.1 p.2)
      exchanged := true
      log := log }
  else
    match s.memo r i with
    | some _ => { s with log := log }
    | none => { s with log := log, memo := setMemo s.memo r i (some p) }

/-- `scan_rank`. -/
def stepAllS (r : ℕ) (s : SState N) : SState N :=
  let s' := (List.range (V.n r - 1)).foldl (stepS V r)
    { s with cand := update s.cand r false, rv := 0, exchanged := false }
  if s'.exchanged = true then
    { s' with
      moved := update s'.moved r true
      valid := V.markValid r s'.valid
      cand := V.markCand r s'.cand
      exchanged := false }
  else s'

/-- One round of `run_scans`. -/
def roundS (s : SState N) : SState N × ℤ :=
  V.ranks.foldl (fun (st : SState N × ℤ) r =>
    if st.1.cand r = true then
      let s' := stepAllS V r st.1
      (s', V.arith.add st.2 s'.rv)
    else st) (s, 0)

/-- At most `k` rounds of `run_scans`. -/
def loopS : ℕ → SState N → SState N × Bool
  | 0, s => (s, false)
  | k + 1, s =>
    let st := roundS V s
    if st.2 < 1 then (st.1, true) else loopS k st.1

/-- One position of `publish_scans`: the buffer cell and the arena order. -/
def publishAt (r : ℕ) (s : SState N) (i : ℕ) : SState N :=
  { s with
    buf := update s.buf r (update (s.buf r) (V.base r + i) (s.slot r i))
    arena := update s.arena (s.slot r i) (V.base r + i) }

/-- `publish_scans`: the moved ranks are written once. -/
def publish (s : SState N) : SState N :=
  V.ranks.foldl (fun s r =>
    if s.moved r = true then (List.range (V.n r)).foldl (publishAt V r) s else s) s

/-- `windows_hold_their_orders` on one slot: (W) as the code tests it. -/
def holdsAt (ord : N → ℤ) (slot : ℕ → ℕ → N) (x : N) : Bool :=
  let r := V.rankOf x
  if V.InView r then
    let place := ord x - V.base r
    decide (place < 0) || decide (place ≥ (V.n r : ℤ)) || decide (slot r place.toNat = x)
  else true

/-- `meta.rank == r` and `snapshot_valid`, `snapshot_adjacent`: `rank_positions_match`'s test and
`build_scan`'s on one node, as this model's `Inv` needs them. -/
def rowOk (ord : N → ℤ) (r : ℕ) (x : N) (place : ℤ) : Bool :=
  decide (V.rankOf x = r) && decide (ord x = place) &&
    (V.inE x).all (fun e => decide (V.rankOf e.other + 1 = r)) &&
    (V.outE x).all (fun e => decide (V.rankOf e.other = r + 1))

/-- `build_scan`, taken before anything changes: `none` means C's loop runs. On `some`, every
position starts unknown. -/
def buildScan (c : CState N) : Option (SState N) :=
  let slot : ℕ → ℕ → N := fun r i => c.buf r (V.base r + i)
  if V.ranks.all (fun r => (List.range (V.n r)).all
        (fun i => rowOk V c.order r (slot r i) (V.base r + i))) ∧
      V.slots.all (holdsAt V c.order slot) then
    some { order := c.order, slot := slot, memo := fun _ _ => none, moved := fun _ => false,
           exchanged := false, cand := c.cand, valid := c.valid, rv := c.rv, log := c.log,
           arena := c.order, buf := c.buf }
  else none

end Scan

/-! ## The invariants -/

section Invariants

variable (V : View N P)

/-- Both ends of every edge list it: `forget_pairs` walks v's and w's edges to find a pair that
has edges to v and w. `prepare_mincross` reads both from `ND_in` and `ND_out`, one edge each. -/
def View.Sym : Prop :=
  ∀ x z, (∃ e ∈ V.inE x, e.other = z) ↔ (∃ e ∈ V.outE z, e.other = x)

/-- The scan's invariant at every read. -/
structure Inv (s : SState N) : Prop where
  /-- The window is a proven permutation (`positioned_rank`), kept by exchanges. -/
  pos : ∀ r i, V.InView r → i < V.n r →
    s.order (s.slot r i) = V.base r + i ∧ V.rankOf (s.slot r i) = r
  /-- `snapshot_adjacent`: the window's edges reach only the next ranks. -/
  adj : ∀ r i, V.InView r → i < V.n r →
    (∀ e ∈ V.inE (s.slot r i), V.rankOf e.other + 1 = r) ∧
    (∀ e ∈ V.outE (s.slot r i), V.rankOf e.other = r + 1)
  /-- (W), in the form the code tests: an order inside the window is that position's node. -/
  w : ∀ x i, V.InView (V.rankOf x) → i < V.n (V.rankOf x) →
    s.order x = V.base (V.rankOf x) + i → x = s.slot (V.rankOf x) i
  /-- (M): a `known` position holds what C would compute now. -/
  m : ∀ r i p, V.InView r → i + 1 < V.n r → s.memo r i = some p →
    p = V.pair s.order r (s.slot r i) (s.slot r (i + 1))

end Invariants

/-! ## (F): what a pair's counts depend on -/

section F

variable (V : View N P)

/-- Two order functions that agree on the sign of `a - b`. -/
def SameSign (ord ord' : N → ℤ) (a b : N) : Prop :=
  (ord a - ord b > 0 ↔ ord' a - ord' b > 0) ∧ (ord a - ord b = 0 ↔ ord' a - ord' b = 0)

theorem sameSign_of_eq {ord ord' : N → ℤ} {a b : N} (ha : ord' a = ord a) (hb : ord' b = ord b) :
    SameSign ord ord' a b := by
  simp [SameSign, ha, hb]

/-- The loops use an order only through the sign of a difference (first sentence of (E)). -/
theorem crosses_congr (gt : P → P → Bool) {ord ord' : N → ℤ} {e1 e2 : EdgeEnd N P}
    (h : SameSign ord ord' e1.other e2.other) :
    View.crosses gt ord e1 e2 ↔ View.crosses gt ord' e1 e2 := by
  unfold View.crosses; rw [h.1, h.2]

theorem crossLoop_congr (gt : P → P → Bool) (add : ℤ → ℤ → ℤ → ℤ) {ord ord' : N → ℤ}
    {vs ws : List (EdgeEnd N P)}
    (h : ∀ e1 ∈ vs, ∀ e2 ∈ ws, SameSign ord ord' e1.other e2.other) :
    View.crossLoop gt add ord vs ws = View.crossLoop gt add ord' vs ws := by
  unfold View.crossLoop
  have inner : ∀ (e2 : EdgeEnd N P) (xs : List (EdgeEnd N P)),
      (∀ e1 ∈ xs, SameSign ord ord' e1.other e2.other) → ∀ a : ℤ,
      xs.foldl (fun acc e1 => if View.crosses gt ord e1 e2
          then add acc e1.penalty e2.penalty else acc) a =
        xs.foldl (fun acc e1 => if View.crosses gt ord' e1 e2
          then add acc e1.penalty e2.penalty else acc) a := by
    intro e2 xs
    induction xs with
    | nil => intro _ a; rfl
    | cons x xs ih =>
      intro hx a
      have hc := crosses_congr gt (hx x (by simp))
      simp only [List.foldl_cons]
      rw [show (if View.crosses gt ord x e2 then add a x.penalty e2.penalty else a) =
          (if View.crosses gt ord' x e2 then add a x.penalty e2.penalty else a) by
        simp only [hc]]
      exact ih (fun e he => hx e (by simp [he])) _
  have outer : ∀ (ys : List (EdgeEnd N P)), (∀ e2 ∈ ys, e2 ∈ ws) → ∀ a : ℤ,
      ys.foldl (fun acc e2 => vs.foldl (fun acc e1 => if View.crosses gt ord e1 e2
          then add acc e1.penalty e2.penalty else acc) acc) a =
        ys.foldl (fun acc e2 => vs.foldl (fun acc e1 => if View.crosses gt ord' e1 e2
          then add acc e1.penalty e2.penalty else acc) acc) a := by
    intro ys
    induction ys with
    | nil => intro _ a; rfl
    | cons y ys ih =>
      intro hy a
      simp only [List.foldl_cons]
      rw [inner y vs (fun e1 he1 => h e1 he1 y (hy y (by simp))) a]
      exact ih (fun e he => hy e (by simp [he])) _
  exact outer ws (fun _ h => h) 0

/-- (F): a count is a function of the edge lists, which are fixed, and of the signs of the
differences between the orders of `v`'s and `w`'s neighbours, side by side. -/
theorem count_congr {ord ord' : N → ℤ} {r : ℕ} {v w : N}
    (hi : r > 0 → ∀ e1 ∈ V.inE v, ∀ e2 ∈ V.inE w, SameSign ord ord' e1.other e2.other)
    (ho : V.hasOut r = true → ∀ e1 ∈ V.outE v, ∀ e2 ∈ V.outE w,
      SameSign ord ord' e1.other e2.other) :
    V.count ord r v w = V.count ord' r v w := by
  unfold View.count
  by_cases h0 : r > 0
  · rw [ite_eq_left h0, ite_eq_left h0, crossLoop_congr _ _ (hi h0)]
    by_cases h1 : V.hasOut r = true
    · simp only [h1, ite_true]; rw [crossLoop_congr _ _ (ho h1)]
    · simp [h1]
  · rw [ite_eq_right h0, ite_eq_right h0]
    by_cases h1 : V.hasOut r = true
    · simp only [h1, ite_true]; rw [crossLoop_congr _ _ (ho h1)]
    · simp [h1]

/-- §3.2, position `i`: the exchanged pair reads the same two counts the other way round. -/
theorem pair_swap (ord : N → ℤ) (r : ℕ) (v w : N) :
    V.pair ord r w v = ((V.pair ord r v w).2, (V.pair ord r v w).1) := rfl

end F

/-! ## (E): what an exchange changes -/

section E

variable (V : View N P)

/-- (E): swapping two adjacent orders `p`, `p + 1` keeps the sign of every comparison between two
nodes of their rank, except the comparison of `v` with `w`. `hv'` and `hw'` are what (W) and the
proven permutation give: no other node of the rank holds either order. -/
theorem exchange_sameSign {ord : N → ℤ} {v w : N} {p : ℤ} {r : ℕ}
    (hv : ord v = p) (hw : ord w = p + 1)
    (hv' : ∀ x, V.rankOf x = r → ord x = p → x = v)
    (hw' : ∀ x, V.rankOf x = r → ord x = p + 1 → x = w)
    {a b : N} (ha : V.rankOf a = r) (hb : V.rankOf b = r)
    (hne : ¬((a = v ∧ b = w) ∨ (a = w ∧ b = v))) :
    SameSign ord (update (update ord v (p + 1)) w p) a b := by
  have hvw : v ≠ w := by rintro rfl; omega
  set ord' := update (update ord v (p + 1)) w p with hord'
  have ev : ord' v = p + 1 := by simp [hord', update_of_ne hvw]
  have ew : ord' w = p := by simp [hord']
  have eo : ∀ x, x ≠ v → x ≠ w → ord' x = ord x := by
    intro x h1 h2; simp [hord', update_of_ne h2, update_of_ne h1]
  have no : ∀ x, V.rankOf x = r → x ≠ v → x ≠ w → ord x ≠ p ∧ ord x ≠ p + 1 := by
    intro x hx h1 h2
    exact ⟨fun h => h1 (hv' x hx h), fun h => h2 (hw' x hx h)⟩
  unfold SameSign
  by_cases hav : a = v
  · subst hav
    by_cases hbv : b = a
    · subst hbv; simp
    · have hbw : b ≠ w := fun h => hne (Or.inl ⟨rfl, h⟩)
      have := no b hb hbv hbw
      rw [ev, eo b hbv hbw, hv]; omega
  · by_cases haw : a = w
    · subst haw
      by_cases hbw : b = a
      · subst hbw; simp
      · have hbv : b ≠ v := fun h => hne (Or.inr ⟨rfl, h⟩)
        have := no b hb hbv hbw
        rw [ew, eo b hbv hbw, hw]; omega
    · have := no a ha hav haw
      by_cases hbv : b = v
      · subst hbv; rw [eo a hav haw, ev, hv]; omega
      · by_cases hbw : b = w
        · subst hbw; rw [eo a hav haw, ew, hw]; omega
        · rw [eo a hav haw, eo b hbv hbw]; simp

end E

/-! ## `forget_pairs` -/

section Forget

variable (V : View N P)

/-- `m'` knows no more than `m`, and what it knows it got from `m`: over-forgetting is harmless. -/
def MemoLe (m m' : Memo) : Prop := ∀ r i c, m' r i = some c → m r i = some c

theorem MemoLe.refl (m : Memo) : MemoLe m m := fun _ _ _ h => h

theorem MemoLe.trans {a b c : Memo} (h1 : MemoLe a b) (h2 : MemoLe b c) : MemoLe a c :=
  fun r i x h => h1 r i x (h2 r i x h)

theorem MemoLe.none {a b : Memo} (h : MemoLe a b) {r i : ℕ} (ha : a r i = none) : b r i = none := by
  cases hb : b r i with
  | none => rfl
  | some x => have := h r i x hb; simp [ha] at this

theorem setMemo_none_le (m : Memo) (r i : ℕ) : MemoLe m (setMemo m r i none) := by
  intro r' i' c h
  unfold setMemo at h
  split_ifs at h
  exact h

theorem clearAt_le (rk : ℕ) (first : ℤ) (m : Memo) : MemoLe m (clearAt V rk first m) := by
  unfold clearAt
  split_ifs
  · exact setMemo_none_le _ _ _
  · exact MemoLe.refl _

theorem clearAt_hit {rk j : ℕ} (hj : j + 1 < V.n rk) (m : Memo) :
    clearAt V rk (V.base rk + j) m rk j = none := by
  unfold clearAt
  have h1 : V.base rk + (j : ℤ) - V.base rk = j := by ring
  rw [h1, ite_eq_left (by constructor <;> omega)]
  simp [setMemo]

/-- One step of `forget_pairs`' inner loop. -/
def forgetStep (ord : N → ℤ) (rk : ℕ) (x : EdgeEnd N P) (m : Memo) (y : EdgeEnd N P) : Memo :=
  if ord x.other - ord y.other = 1 then clearAt V rk (ord y.other) m
  else if ord x.other - ord y.other = -1 then clearAt V rk (ord x.other) m
  else m

theorem forgetStep_le (ord : N → ℤ) (rk : ℕ) (x : EdgeEnd N P) (m : Memo) (y : EdgeEnd N P) :
    MemoLe m (forgetStep V ord rk x m y) := by
  unfold forgetStep
  split_ifs
  · exact clearAt_le _ _ _ _
  · exact clearAt_le _ _ _ _
  · exact MemoLe.refl _

theorem forget_eq (ord : N → ℤ) (rk : ℕ) (xs ys : List (EdgeEnd N P)) (m : Memo) :
    forget V ord rk xs ys m = xs.foldl (fun m x => ys.foldl (forgetStep V ord rk x) m) m := rfl

theorem inner_le (ord : N → ℤ) (rk : ℕ) (x : EdgeEnd N P) (ys : List (EdgeEnd N P)) (m : Memo) :
    MemoLe m (ys.foldl (forgetStep V ord rk x) m) := by
  induction ys generalizing m with
  | nil => exact MemoLe.refl _
  | cons y ys ih => exact (forgetStep_le V ord rk x m y).trans (ih _)

/-- `forget_pairs` only forgets. -/
theorem forget_le (ord : N → ℤ) (rk : ℕ) (xs ys : List (EdgeEnd N P)) (m : Memo) :
    MemoLe m (forget V ord rk xs ys m) := by
  rw [forget_eq]
  induction xs generalizing m with
  | nil => exact MemoLe.refl _
  | cons x xs ih => exact (inner_le V ord rk x ys m).trans (ih _)

theorem forgetStep_hit {ord : N → ℤ} {rk j : ℕ} {x y : EdgeEnd N P} (hj : j + 1 < V.n rk)
    (h : (ord x.other = V.base rk + j ∧ ord y.other = V.base rk + j + 1) ∨
      (ord y.other = V.base rk + j ∧ ord x.other = V.base rk + j + 1)) (m : Memo) :
    forgetStep V ord rk x m y rk j = none := by
  unfold forgetStep
  rcases h with ⟨h1, h2⟩ | ⟨h1, h2⟩
  · rw [ite_eq_right (by omega), ite_eq_left (by omega), h1]
    exact clearAt_hit V hj m
  · rw [ite_eq_left (by omega), h1]
    exact clearAt_hit V hj m

/-- §3.2, ranks `r − 1` and `r + 1`: a pair at adjacent positions `j`, `j + 1` of `rk` with an edge
to `v` on one side and to `w` on the other is forgotten, whichever way round. -/
theorem forget_hits {ord : N → ℤ} {rk j : ℕ} {xs ys : List (EdgeEnd N P)} {x y : EdgeEnd N P}
    (hx : x ∈ xs) (hy : y ∈ ys) (hj : j + 1 < V.n rk)
    (h : (ord x.other = V.base rk + j ∧ ord y.other = V.base rk + j + 1) ∨
      (ord y.other = V.base rk + j ∧ ord x.other = V.base rk + j + 1)) (m : Memo) :
    forget V ord rk xs ys m rk j = none := by
  rw [forget_eq]
  have inner : ∀ (ys : List (EdgeEnd N P)), y ∈ ys → ∀ m : Memo,
      ys.foldl (forgetStep V ord rk x) m rk j = none := by
    intro ys
    induction ys with
    | nil => simp
    | cons y' ys ih =>
      intro hy m
      simp only [List.foldl_cons]
      rcases List.mem_cons.mp hy with rfl | hy
      · exact (inner_le V ord rk x ys _).none (forgetStep_hit V hj h m)
      · exact ih hy _
  have outer : ∀ (zs : List (EdgeEnd N P)), x ∈ zs → ∀ m : Memo,
      zs.foldl (fun m x => ys.foldl (forgetStep V ord rk x) m) m rk j = none := by
    intro zs
    induction zs with
    | nil => simp
    | cons x' zs ih =>
      intro hz m
      simp only [List.foldl_cons]
      rcases List.mem_cons.mp hz with rfl | hz
      · have hle : MemoLe (ys.foldl (forgetStep V ord rk x) m)
            (zs.foldl (fun m x => ys.foldl (forgetStep V ord rk x) m)
              (ys.foldl (forgetStep V ord rk x) m)) := by
          have := forget_le V ord rk zs ys (ys.foldl (forgetStep V ord rk x) m)
          rwa [forget_eq] at this
        exact hle.none (inner ys hy m)
      · exact ih hz _
  exact outer xs hx m

end Forget

/-! ## The exchange keeps the invariants -/

section Exchange

variable (V : View N P) {s : SState N} {r i : ℕ}

/-- The order function after the exchange at position `i` of rank `r`. -/
def ordAfter (V : View N P) (s : SState N) (r i : ℕ) : N → ℤ :=
  update (update s.order (s.slot r i) (V.base r + i + 1)) (s.slot r (i + 1)) (V.base r + i)

theorem Inv.slot_inj (hs : Inv V s) {r j k : ℕ} (hr : V.InView r) (hj : j < V.n r) (hk : k < V.n r)
    (h : s.slot r j = s.slot r k) : j = k := by
  have h1 := (hs.pos r j hr hj).1
  have h2 := (hs.pos r k hr hk).1
  rw [h] at h1
  omega

theorem Inv.order_at (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) :
    s.order (s.slot r i) = V.base r + i ∧ s.order (s.slot r (i + 1)) = V.base r + i + 1 := by
  have h1 := (hs.pos r i hr (by omega)).1
  have h2 := (hs.pos r (i + 1) hr hi).1
  push_cast at h2
  exact ⟨h1, by omega⟩

theorem ordAfter_v (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) :
    ordAfter V s r i (s.slot r i) = V.base r + i + 1 := by
  have hne : s.slot r i ≠ s.slot r (i + 1) := fun h => by
    have := hs.slot_inj V hr (by omega) hi h; omega
  simp [ordAfter, update_of_ne hne]

theorem ordAfter_w (s : SState N) (r i : ℕ) :
    ordAfter V s r i (s.slot r (i + 1)) = V.base r + i := by
  simp [ordAfter]

theorem ordAfter_other {x : N} (h1 : x ≠ s.slot r i) (h2 : x ≠ s.slot r (i + 1)) :
    ordAfter V s r i x = s.order x := by
  simp [ordAfter, update_of_ne h2, update_of_ne h1]

/-- A node of another rank is neither of the exchanged two. -/
theorem ordAfter_far (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) {x : N}
    (h : V.rankOf x ≠ r) : ordAfter V s r i x = s.order x := by
  refine ordAfter_other V ?_ ?_
  · rintro rfl; exact h (hs.pos r i hr (by omega)).2
  · rintro rfl; exact h (hs.pos r (i + 1) hr hi).2

/-- (E) at this exchange, with (W) and the proven permutation discharging its side conditions. -/
theorem exchange_sign (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) {a b : N}
    (ha : V.rankOf a = r) (hb : V.rankOf b = r)
    (hne : ¬((a = s.slot r i ∧ b = s.slot r (i + 1)) ∨ (a = s.slot r (i + 1) ∧ b = s.slot r i))) :
    SameSign s.order (ordAfter V s r i) a b := by
  obtain ⟨h1, h2⟩ := hs.order_at V hr hi
  refine exchange_sameSign V h1 h2 ?_ ?_ ha hb hne
  · intro x hx h
    have := hs.w x i (by rw [hx]; exact hr) (by rw [hx]; omega) (by rw [hx]; exact h)
    rwa [hx] at this
  · intro x hx h
    have := hs.w x (i + 1) (by rw [hx]; exact hr) (by rw [hx]; exact hi)
      (by rw [hx]; push_cast; omega)
    rwa [hx] at this

/-- No pair of neighbours of `xs` and `ys` is `{v, w}`. -/
def NoCross (xs ys : List (EdgeEnd N P)) (v w : N) : Prop :=
  ∀ e1 ∈ xs, ∀ e2 ∈ ys, ¬((e1.other = v ∧ e2.other = w) ∨ (e1.other = w ∧ e2.other = v))

theorem NoCross.symm {xs ys : List (EdgeEnd N P)} {v w : N} (h : NoCross xs ys v w) :
    NoCross ys xs v w := by
  intro e1 he1 e2 he2 hc
  refine h e2 he2 e1 he1 ?_
  rcases hc with ⟨a, b⟩ | ⟨a, b⟩
  · exact Or.inr ⟨b, a⟩
  · exact Or.inl ⟨b, a⟩

/-- A pair of rank `r'` keeps its counts through the exchange in rank `r`, when none of its sides
that face `r` has an edge to `v` on one node and to `w` on the other. -/
theorem pair_stable (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) {r' : ℕ} {x y : N}
    (hxin : ∀ e ∈ V.inE x, V.rankOf e.other + 1 = r') (hyin : ∀ e ∈ V.inE y, V.rankOf e.other + 1 = r')
    (hxout : ∀ e ∈ V.outE x, V.rankOf e.other = r' + 1)
    (hyout : ∀ e ∈ V.outE y, V.rankOf e.other = r' + 1)
    (hin : r' = r + 1 → NoCross (V.inE x) (V.inE y) (s.slot r i) (s.slot r (i + 1)))
    (hout : r' + 1 = r → NoCross (V.outE x) (V.outE y) (s.slot r i) (s.slot r (i + 1))) :
    V.pair (ordAfter V s r i) r' x y = V.pair s.order r' x y := by
  have key : ∀ (xs ys : List (EdgeEnd N P)) (d : ℕ),
      (∀ e ∈ xs, V.rankOf e.other = d) → (∀ e ∈ ys, V.rankOf e.other = d) →
      (d = r → NoCross xs ys (s.slot r i) (s.slot r (i + 1))) →
      ∀ e1 ∈ xs, ∀ e2 ∈ ys, SameSign s.order (ordAfter V s r i) e1.other e2.other := by
    intro xs ys d hxs hys hc e1 he1 e2 he2
    by_cases hd : d = r
    · refine exchange_sign V hs hr hi ?_ ?_ (hc hd e1 he1 e2 he2)
      · rw [hxs e1 he1, hd]
      · rw [hys e2 he2, hd]
    · refine sameSign_of_eq (ordAfter_far V hs hr hi ?_) (ordAfter_far V hs hr hi ?_)
      · rw [hxs e1 he1]; exact hd
      · rw [hys e2 he2]; exact hd
  have inRank : ∀ {z : N}, (∀ e ∈ V.inE z, V.rankOf e.other + 1 = r') →
      ∀ e ∈ V.inE z, V.rankOf e.other = r' - 1 := by
    intro z hz e he; have := hz e he; omega
  have hinC : r' - 1 = r → r' > 0 → NoCross (V.inE x) (V.inE y) (s.slot r i) (s.slot r (i + 1)) :=
    fun h h0 => hin (by omega)
  unfold View.pair
  ext
  · symm
    refine count_congr V ?_ ?_
    · intro h0
      exact key _ _ (r' - 1) (inRank hxin) (inRank hyin) (fun hd => hinC hd h0)
    · intro _
      exact key _ _ (r' + 1) hxout hyout hout
  · symm
    refine count_congr V ?_ ?_
    · intro h0
      exact key _ _ (r' - 1) (inRank hyin) (inRank hxin) (fun hd => (hinC hd h0).symm)
    · intro _
      exact key _ _ (r' + 1) hyout hxout (fun hd => (hout hd).symm)

theorem swapSlot_rank_ne (slot : ℕ → ℕ → N) {r r' : ℕ} (i j : ℕ) (h : r' ≠ r) :
    swapSlot slot r i r' j = slot r' j := by
  simp [swapSlot, h]

theorem swapSlot_i (slot : ℕ → ℕ → N) (r i : ℕ) : swapSlot slot r i r i = slot r (i + 1) := by
  simp [swapSlot]

theorem swapSlot_succ (slot : ℕ → ℕ → N) (r i : ℕ) : swapSlot slot r i r (i + 1) = slot r i := by
  simp [swapSlot]

theorem swapSlot_other (slot : ℕ → ℕ → N) (r i : ℕ) {j : ℕ} (h1 : j ≠ i) (h2 : j ≠ i + 1) :
    swapSlot slot r i r j = slot r j := by
  simp [swapSlot, h1, h2]

/-- (M) after the exchange: the page's bullets of §3.2, one case each. -/
theorem exchange_m (sym : V.Sym) (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r)
    {p : ℤ × ℤ} (hp : p = V.pair s.order r (s.slot r i) (s.slot r (i + 1))) :
    ∀ r' j c, V.InView r' → j + 1 < V.n r' →
      memoAfter V (ordAfter V s r i) r i (s.slot r i) (s.slot r (i + 1)) p s.memo r' j = some c →
      c = V.pair (ordAfter V s r i) r' (swapSlot s.slot r i r' j)
        (swapSlot s.slot r i r' (j + 1)) := by
  intro r' j c hr' hj hc
  set v := s.slot r i with hv
  set w := s.slot r (i + 1) with hw
  set ord' := ordAfter V s r i with hord'
  set m1 := setMemo (setMemo s.memo r i (some (p.2, p.1))) r (i + 1) none with hm1
  set m2 := (if i > 0 then setMemo m1 r (i - 1) none else m1) with hm2
  set m3 := (if r > V.minr then forget V ord' (r - 1) (V.inE v) (V.inE w) m2 else m2) with hm3
  have hres : memoAfter V ord' r i v w p s.memo =
      (if r < V.maxr then forget V ord' (r + 1) (V.outE v) (V.outE w) m3 else m3) := rfl
  rw [hres] at hc
  have le34 : MemoLe m3 (if r < V.maxr then forget V ord' (r + 1) (V.outE v) (V.outE w) m3
      else m3) := by
    split_ifs
    · exact forget_le _ _ _ _ _ _
    · exact MemoLe.refl _
  have le23 : MemoLe m2 m3 := by
    rw [hm3]; split_ifs
    · exact forget_le _ _ _ _ _ _
    · exact MemoLe.refl _
  have hc3 := le34 _ _ _ hc
  have hc2 := le23 _ _ _ hc3
  -- neighbours of the two nodes of position `j`, in the old slots
  have adj1 := hs.adj
  -- ranks `r ± 1`: a pair with edges to `v` and `w` is forgotten (the third bullet)
  have hup : r' = r + 1 → ∀ {x y : N}, x = s.slot r' j → y = s.slot r' (j + 1) →
      NoCross (V.inE x) (V.inE y) v w := by
    intro hrr x y hx hy e1 he1 e2 he2 hcross
    have hlt : r < V.maxr := by have := hr'.2; omega
    have ox : ord' x = V.base r' + j := by
      rw [hord', ordAfter_far V hs hr hi (by rw [hx, (hs.pos r' j hr' (by omega)).2]; omega), hx]
      exact (hs.pos r' j hr' (by omega)).1
    have oy : ord' y = V.base r' + j + 1 := by
      rw [hord', ordAfter_far V hs hr hi (by rw [hy, (hs.pos r' (j + 1) hr' hj).2]; omega), hy]
      have := (hs.pos r' (j + 1) hr' hj).1
      push_cast at this
      omega
    have hnone : forget V ord' (r + 1) (V.outE v) (V.outE w) m3 r' j = none := by
      subst hrr
      rcases hcross with ⟨a, b⟩ | ⟨a, b⟩
      · obtain ⟨g1, hg1, hg1o⟩ := (sym x v).mp ⟨e1, he1, a⟩
        obtain ⟨g2, hg2, hg2o⟩ := (sym y w).mp ⟨e2, he2, b⟩
        exact forget_hits V hg1 hg2 hj (Or.inl ⟨by rw [hg1o, ox], by rw [hg2o, oy]⟩) _
      · obtain ⟨g1, hg1, hg1o⟩ := (sym x w).mp ⟨e1, he1, a⟩
        obtain ⟨g2, hg2, hg2o⟩ := (sym y v).mp ⟨e2, he2, b⟩
        exact forget_hits V hg2 hg1 hj (Or.inr ⟨by rw [hg1o, ox], by rw [hg2o, oy]⟩) _
    rw [ite_eq_left hlt, hnone] at hc
    simp at hc
  have hdown : r' + 1 = r → ∀ {x y : N}, x = s.slot r' j → y = s.slot r' (j + 1) →
      NoCross (V.outE x) (V.outE y) v w := by
    intro hrr x y hx hy e1 he1 e2 he2 hcross
    have hgt : r > V.minr := by have := hr'.1; omega
    have ox : ord' x = V.base r' + j := by
      rw [hord', ordAfter_far V hs hr hi (by rw [hx, (hs.pos r' j hr' (by omega)).2]; omega), hx]
      exact (hs.pos r' j hr' (by omega)).1
    have oy : ord' y = V.base r' + j + 1 := by
      rw [hord', ordAfter_far V hs hr hi (by rw [hy, (hs.pos r' (j + 1) hr' hj).2]; omega), hy]
      have := (hs.pos r' (j + 1) hr' hj).1
      push_cast at this
      omega
    have hr1 : r - 1 = r' := by omega
    have hnone : m3 r' j = none := by
      rw [hm3, ite_eq_left hgt]
      rw [hr1]
      rcases hcross with ⟨a, b⟩ | ⟨a, b⟩
      · obtain ⟨g1, hg1, hg1o⟩ := (sym v x).mpr ⟨e1, he1, a⟩
        obtain ⟨g2, hg2, hg2o⟩ := (sym w y).mpr ⟨e2, he2, b⟩
        exact forget_hits V hg1 hg2 hj (Or.inl ⟨by rw [hg1o, ox], by rw [hg2o, oy]⟩) _
      · obtain ⟨g1, hg1, hg1o⟩ := (sym w x).mpr ⟨e1, he1, a⟩
        obtain ⟨g2, hg2, hg2o⟩ := (sym v y).mpr ⟨e2, he2, b⟩
        exact forget_hits V hg2 hg1 hj (Or.inr ⟨by rw [hg1o, ox], by rw [hg2o, oy]⟩) _
    rw [hnone] at hc3
    simp at hc3
  -- a pair that keeps its nodes
  have stable : ∀ (x y : N), x = s.slot r' j → y = s.slot r' (j + 1) →
      V.pair ord' r' x y = V.pair s.order r' x y := by
    intro x y hx hy
    have ax := hs.adj r' j hr' (by omega)
    have ay := hs.adj r' (j + 1) hr' hj
    rw [← hx] at ax
    rw [← hy] at ay
    exact pair_stable V hs hr hi ax.1 ay.1 ax.2 ay.2 (fun h => hup h hx hy) (fun h => hdown h hx hy)
  by_cases hrr : r' = r
  · subst hrr
    by_cases hji : j = i
    · -- position i: (c1, c0), still known
      subst hji
      have hm : m2 r' j = some (p.2, p.1) := by
        have h1 : m1 r' j = some (p.2, p.1) := by simp [hm1, setMemo]
        rw [hm2]
        split_ifs with h0
        · simp only [setMemo]
          rw [ite_eq_right (by omega)]
          exact h1
        · exact h1
      rw [hm] at hc2
      obtain rfl : (p.2, p.1) = c := by simpa using hc2
      rw [swapSlot_i, swapSlot_succ, pair_swap V ord' r' v w]
      have hp' : p = V.pair s.order r' v w := hp
      have hA : V.pair ord' r' v w = V.pair s.order r' v w := by
        have ax := hs.adj r' j hr' (by omega)
        have ay := hs.adj r' (j + 1) hr' hj
        exact pair_stable V hs hr hi ax.1 ay.1 ax.2 ay.2 (fun h => by omega) (fun h => by omega)
      rw [hA, ← hp']
    · by_cases hj1 : j = i + 1
      · -- position i + 1: a new pair, forgotten
        subst hj1
        have : m1 r' (i + 1) = none := by simp [hm1, setMemo]
        have h2 : m2 r' (i + 1) = none := by
          rw [hm2]; split_ifs
          · exact (setMemo_none_le m1 r' (i - 1)).none this
          · exact this
        rw [h2] at hc2; simp at hc2
      · by_cases hj2 : j + 1 = i
        · -- position i - 1: a new pair, forgotten
          have h0 : i > 0 := by omega
          have h2 : m2 r' j = none := by
            rw [hm2, ite_eq_left h0]; simp only [setMemo]; rw [ite_eq_left ⟨trivial, by omega⟩]
          rw [h2] at hc2; simp at hc2
        · -- other positions of r: same nodes
          have h1 : m1 r' j = some c := by
            rw [hm2] at hc2
            split_ifs at hc2 with h0
            · simp only [setMemo] at hc2
              rwa [ite_eq_right (by omega)] at hc2
            · exact hc2
          have h0 : s.memo r' j = some c := by
            simp only [hm1, setMemo] at h1
            rwa [ite_eq_right (by omega), ite_eq_right (by omega)] at h1
          rw [swapSlot_other _ _ _ hji hj1, swapSlot_other _ _ _ (by omega) (by omega)]
          rw [stable _ _ rfl rfl]
          exact hs.m r' j c hr' hj h0
  · -- other ranks: same nodes
    have h1 : m1 r' j = some c := by
      rw [hm2] at hc2
      split_ifs at hc2 with h0
      · simp only [setMemo] at hc2
        rwa [ite_eq_right (by omega)] at hc2
      · exact hc2
    have h0 : s.memo r' j = some c := by
      simp only [hm1, setMemo] at h1
      rwa [ite_eq_right (by omega), ite_eq_right (by omega)] at h1
    rw [swapSlot_rank_ne _ _ _ hrr, swapSlot_rank_ne _ _ _ hrr, stable _ _ rfl rfl]
    exact hs.m r' j c hr' hj h0

end Exchange

/-! ## One step of the scan keeps the invariants -/

section Step

variable (V : View N P) {s : SState N} {r i : ℕ}

theorem Inv.congr {s s' : SState N} (hs : Inv V s) (ho : s'.order = s.order)
    (hl : s'.slot = s.slot) (hm : s'.memo = s.memo) : Inv V s' where
  pos := by rw [ho, hl]; exact hs.pos
  adj := by rw [hl]; exact hs.adj
  w := by rw [ho, hl]; exact hs.w
  m := by rw [ho, hl, hm]; exact hs.m

/-- (M) at a read: every count the scan uses is the integer C would compute. -/
theorem read_eq (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) :
    readCounts V s r i = V.pair s.order r (s.slot r i) (s.slot r (i + 1)) := by
  unfold readCounts
  cases h : s.memo r i with
  | none => rfl
  | some p => exact hs.m r i p hr hi h

/-- The exchange branch of `stepS`. -/
def exchangeS (V : View N P) (s : SState N) (r i : ℕ) (p : ℤ × ℤ) : SState N :=
  { s with
    order := ordAfter V s r i
    slot := swapSlot s.slot r i
    memo := memoAfter V (ordAfter V s r i) r i (s.slot r i) (s.slot r (i + 1)) p s.memo
    rv := V.arith.add s.rv (V.arith.sub p.1 p.2)
    exchanged := true
    log := s.log ++ [⟨r, i, p, V.exch p⟩] }

/-- Every node of a window after the exchange is a node of that window before it. -/
theorem swapSlot_mem (s : SState N) {r' j : ℕ} (hi : i + 1 < V.n r) (hj : j < V.n r') :
    ∃ k, k < V.n r' ∧ swapSlot s.slot r i r' j = s.slot r' k := by
  by_cases hrr : r' = r
  · subst hrr
    by_cases h1 : j = i
    · subst h1; exact ⟨j + 1, hi, swapSlot_i _ _ _⟩
    · by_cases h2 : j = i + 1
      · subst h2; exact ⟨i, by omega, swapSlot_succ _ _ _⟩
      · exact ⟨j, hj, swapSlot_other _ _ _ h1 h2⟩
  · exact ⟨j, hj, swapSlot_rank_ne _ _ _ hrr⟩

/-- The converse: every node of a window before the exchange is one after it. -/
theorem swapSlot_mem' (s : SState N) {r' k : ℕ} (hi : i + 1 < V.n r) (hk : k < V.n r') :
    ∃ j, j < V.n r' ∧ s.slot r' k = swapSlot s.slot r i r' j := by
  by_cases hrr : r' = r
  · subst hrr
    by_cases h1 : k = i
    · subst h1; exact ⟨k + 1, hi, (swapSlot_succ _ _ _).symm⟩
    · by_cases h2 : k = i + 1
      · subst h2; exact ⟨i, by omega, (swapSlot_i _ _ _).symm⟩
      · exact ⟨k, hk, (swapSlot_other _ _ _ h1 h2).symm⟩
  · exact ⟨k, hk, (swapSlot_rank_ne _ _ _ hrr).symm⟩

/-- The exchange keeps all four parts of the invariant: (W) and the proven permutation, which the
page takes for granted between exchanges, and (M). -/
theorem exchange_inv (sym : V.Sym) (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r)
    {p : ℤ × ℤ} (hp : p = V.pair s.order r (s.slot r i) (s.slot r (i + 1))) :
    Inv V (exchangeS V s r i p) := by
  have hne : s.slot r i ≠ s.slot r (i + 1) := fun h => by
    have := hs.slot_inj V hr (by omega) hi h; omega
  have rv : V.rankOf (s.slot r i) = r := (hs.pos r i hr (by omega)).2
  have rw' : V.rankOf (s.slot r (i + 1)) = r := (hs.pos r (i + 1) hr hi).2
  refine ⟨?_, ?_, ?_, ?_⟩
  · -- the proven permutation
    intro r' j hr' hj
    simp only [exchangeS]
    by_cases hrr : r' = r
    · subst hrr
      by_cases h1 : j = i
      · subst h1
        rw [swapSlot_i, ordAfter_w]
        exact ⟨rfl, rw'⟩
      · by_cases h2 : j = i + 1
        · subst h2
          rw [swapSlot_succ, ordAfter_v V hs hr hi]
          exact ⟨by push_cast; ring, rv⟩
        · rw [swapSlot_other _ _ _ h1 h2]
          refine ⟨?_, (hs.pos r' j hr' hj).2⟩
          rw [ordAfter_other]
          · exact (hs.pos r' j hr' hj).1
          · intro h; exact h1 (hs.slot_inj V hr hj (by omega) h)
          · intro h; exact h2 (hs.slot_inj V hr hj hi h)
    · rw [swapSlot_rank_ne _ _ _ hrr]
      refine ⟨?_, (hs.pos r' j hr' hj).2⟩
      rw [ordAfter_far V hs hr hi (by rw [(hs.pos r' j hr' hj).2]; exact hrr)]
      exact (hs.pos r' j hr' hj).1
  · -- `snapshot_adjacent`
    intro r' j hr' hj
    obtain ⟨k, hk, he⟩ := swapSlot_mem V s (r := r) (i := i) hi hj
    simp only [exchangeS, he]
    exact hs.adj r' k hr' hk
  · -- (W)
    intro x k hx hk hord
    simp only [exchangeS] at hord ⊢
    by_cases h1 : x = s.slot r i
    · have hxr : V.rankOf x = r := by rw [h1]; exact rv
      rw [hxr] at hord ⊢
      rw [h1, ordAfter_v V hs hr hi] at hord
      have hk' : k = i + 1 := by omega
      rw [hk', swapSlot_succ, h1]
    · by_cases h2 : x = s.slot r (i + 1)
      · have hxr : V.rankOf x = r := by rw [h2]; exact rw'
        rw [hxr] at hord ⊢
        rw [h2, ordAfter_w] at hord
        have hk' : k = i := by omega
        rw [hk', swapSlot_i, h2]
      · rw [ordAfter_other V h1 h2] at hord
        have hw := hs.w x k hx hk hord
        by_cases hxr : V.rankOf x = r
        · rw [hxr] at hw ⊢
          by_cases hk1 : k = i
          · exact absurd (hk1 ▸ hw) h1
          · by_cases hk2 : k = i + 1
            · exact absurd (hk2 ▸ hw) h2
            · rw [swapSlot_other _ _ _ hk1 hk2]; exact hw
        · rw [swapSlot_rank_ne _ _ _ hxr]; exact hw
  · -- (M)
    exact exchange_m V sym hs hr hi hp

/-- `stepS` unfolded on the exchange branch. -/
theorem stepS_exch (hskip : ¬(V.constrained = true ∧ V.left2right (s.slot r i) (s.slot r (i + 1)) = true))
    (hx : V.exch (readCounts V s r i) = true) :
    stepS V r s i = exchangeS V s r i (readCounts V s r i) := by
  unfold stepS exchangeS ordAfter
  simp only [ite_eq_right hskip, hx, ite_true]

/-- `stepS` unfolded on the branch without an exchange. -/
theorem stepS_noexch
    (hskip : ¬(V.constrained = true ∧ V.left2right (s.slot r i) (s.slot r (i + 1)) = true))
    (hx : V.exch (readCounts V s r i) ≠ true) :
    stepS V r s i =
      match s.memo r i with
      | some _ => { s with log := s.log ++ [⟨r, i, readCounts V s r i, false⟩] }
      | none => { s with
          log := s.log ++ [⟨r, i, readCounts V s r i, false⟩]
          memo := setMemo s.memo r i (some (readCounts V s r i)) } := by
  have hf : V.exch (readCounts V s r i) = false := by simpa using hx
  unfold stepS
  simp only [ite_eq_right hskip, hf]
  rfl

theorem stepS_skip
    (hskip : V.constrained = true ∧ V.left2right (s.slot r i) (s.slot r (i + 1)) = true) :
    stepS V r s i = s := by
  unfold stepS
  exact ite_eq_left hskip

/-- A step of the scan keeps the invariant. -/
theorem stepS_inv (sym : V.Sym) (hs : Inv V s) (hr : V.InView r) (hi : i + 1 < V.n r) :
    Inv V (stepS V r s i) := by
  by_cases hskip : V.constrained = true ∧ V.left2right (s.slot r i) (s.slot r (i + 1)) = true
  · rw [stepS_skip V hskip]; exact hs
  · by_cases hx : V.exch (readCounts V s r i) = true
    · rw [stepS_exch V hskip hx]
      exact exchange_inv V sym hs hr hi (read_eq V hs hr hi)
    · rw [stepS_noexch V hskip hx]
      cases hm : s.memo r i with
      | some q => exact hs.congr V rfl rfl rfl
      | none =>
        refine ⟨hs.pos, hs.adj, hs.w, ?_⟩
        intro r' j c hr' hj hc
        simp only [setMemo] at hc
        split_ifs at hc with h
        · obtain ⟨rfl, rfl⟩ := h
          obtain rfl := Option.some.inj hc
          exact read_eq V hs hr' hj
        · exact hs.m r' j c hr' hj hc

end Step

/-! ## §3.3: the scan is C's loop -/

section Sim

variable (V : View N P)

/-- A rank whose window the state's `buf` no longer shows. -/
def Dirty (cur : ℕ) (s : SState N) (r : ℕ) : Prop :=
  s.moved r = true ∨ (r = cur ∧ s.exchanged = true)

/-- `p` is a position of the window of `r`. -/
def InWin (r : ℕ) (p : ℤ) : Prop := V.base r ≤ p ∧ p < V.base r + V.n r

/-- C's state against the scan's, in the middle of the scan of rank `cur`. The three deferred
things of §3.3 are the differences this relation allows: `valid` and `candidate` (`cand`,
`valid`), and `ND_order` and the rank buffers (`buf`, `arena`). -/
structure Rel (cur : ℕ) (c : CState N) (s : SState N) : Prop where
  order : c.order = s.order
  slot : ∀ r i, V.InView r → i < V.n r → c.buf r (V.base r + i) = s.slot r i
  buf : ∀ r p, (¬Dirty cur s r ∨ ¬InWin V r p) → c.buf r p = s.buf r p
  arena : ∀ x, ¬Dirty cur s (V.rankOf x) ∨ ¬V.InView (V.rankOf x) ∨
    (∀ i, i < V.n (V.rankOf x) → x ≠ s.slot (V.rankOf x) i) → s.order x = s.arena x
  cand : c.cand = if s.exchanged = true then V.markCand cur s.cand else s.cand
  valid : c.valid = if s.exchanged = true then V.markValid cur s.valid else s.valid
  rv : c.rv = s.rv
  log : c.log = s.log
  moved : ∀ r, s.moved r = true → V.InView r
  curView : s.exchanged = true → V.InView cur

theorem Rel.rebase {cur cur' : ℕ} {c : CState N} {s : SState N} (h : Rel V cur c s)
    (hx : s.exchanged = false) : Rel V cur' c s := by
  have hd : ∀ r, Dirty cur' s r ↔ Dirty cur s r := by
    intro r; simp [Dirty, hx]
  refine ⟨h.order, h.slot, ?_, ?_, ?_, ?_, h.rv, h.log, h.moved, by simp [hx]⟩
  · intro r p hp; rw [hd] at hp; exact h.buf r p hp
  · intro x hp; rw [hd] at hp; exact h.arena x hp
  · have := h.cand; simpa [hx] using this
  · have := h.valid; simpa [hx] using this

theorem c_slot_succ {cur r i : ℕ} {c : CState N} {s : SState N} (h : Rel V cur c s)
    (hr : V.InView r) (hi : i + 1 < V.n r) :
    c.buf r (V.base r + i + 1) = s.slot r (i + 1) := by
  have := h.slot r (i + 1) hr hi
  push_cast at this
  rwa [← add_assoc] at this

/-- §3.3's induction step: same pair, same counts by (M), same decision, same state. -/
theorem stepS_sim {r i : ℕ} {c : CState N} {s : SState N} (hrel : Rel V r c s) (hs : Inv V s)
    (hr : V.InView r) (hi : i + 1 < V.n r) :
    Rel V r (stepC V r c i) (stepS V r s i) := by
  have hv : c.buf r (V.base r + i) = s.slot r i := hrel.slot r i hr (by omega)
  have hw : c.buf r (V.base r + i + 1) = s.slot r (i + 1) := c_slot_succ V hrel hr hi
  have hread := read_eq V hs hr hi
  have hC : V.pair c.order r (s.slot r i) (s.slot r (i + 1)) = readCounts V s r i := by
    rw [hread, hrel.order]
  by_cases hskip : V.constrained = true ∧ V.left2right (s.slot r i) (s.slot r (i + 1)) = true
  · rw [stepS_skip V hskip]
    have : stepC V r c i = c := by
      unfold stepC
      simp only [hv, hw]
      exact ite_eq_left hskip
    rw [this]; exact hrel
  · have hstepC : stepC V r c i =
        (let p := readCounts V s r i
         let x := V.exch p
         let c' : CState N := { c with log := c.log ++ [⟨r, i, p, x⟩] }
         if x = true then
           { exchangeC V c' (s.slot r i) (s.slot r (i + 1)) with
             rv := V.arith.add c.rv (V.arith.sub p.1 p.2)
             valid := V.markValid r c.valid
             cand := V.markCand r c.cand }
         else c') := by
      unfold stepC
      simp only [hv, hw, ite_eq_right hskip, hC]
    by_cases hx : V.exch (readCounts V s r i) = true
    · rw [stepS_exch V hskip hx, hstepC]
      simp only [hx, ite_true]
      obtain ⟨hov, how⟩ := hs.order_at V hr hi
      have hne : s.slot r i ≠ s.slot r (i + 1) := fun h => by
        have := hs.slot_inj V hr (by omega) hi h; omega
      have rv : V.rankOf (s.slot r i) = r := (hs.pos r i hr (by omega)).2
      have rw' : V.rankOf (s.slot r (i + 1)) = r := (hs.pos r (i + 1) hr hi).2
      have hovC : c.order (s.slot r i) = V.base r + i := by rw [hrel.order]; exact hov
      have howC : c.order (s.slot r (i + 1)) = V.base r + i + 1 := by rw [hrel.order]; exact how
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, hrel.moved, fun _ => hr⟩
      · -- orders: C's reads-and-writes through the orders are the scan's absolute writes
        simp only [exchangeC, exchangeS, ordAfter, hovC, howC, hrel.order]
      · -- the windows
        intro r' j hr' hj
        simp only [exchangeC, exchangeS, rv, hovC, howC]
        by_cases hrr : r' = r
        · subst hrr
          simp only [update_self]
          by_cases h1 : j = i
          · subst h1; rw [swapSlot_i]; simp
          · by_cases h2 : j = i + 1
            · subst h2
              rw [swapSlot_succ]
              have e1 : V.base r' + ((i + 1 : ℕ) : ℤ) ≠ V.base r' + i := by push_cast; omega
              have e2 : V.base r' + ((i + 1 : ℕ) : ℤ) = V.base r' + i + 1 := by push_cast; ring
              rw [update_of_ne e1, e2, update_self]
            · have e1 : V.base r' + (j : ℤ) ≠ V.base r' + i := by omega
              have e2 : V.base r' + (j : ℤ) ≠ V.base r' + i + 1 := by omega
              rw [update_of_ne e1, update_of_ne e2, swapSlot_other _ _ _ h1 h2]
              exact hrel.slot r' j hr' hj
        · rw [update_of_ne hrr, swapSlot_rank_ne _ _ _ hrr]
          exact hrel.slot r' j hr' hj
      · -- the buffers outside what is dirty
        intro r' q hq
        simp only [exchangeC, exchangeS, rv, hovC, howC]
        by_cases hrr : r' = r
        · subst hrr
          simp only [update_self]
          have hnw : ¬InWin V r' q := by
            rcases hq with hq | hq
            · exact absurd (Or.inr ⟨rfl, rfl⟩) hq
            · exact hq
          have e1 : q ≠ V.base r' + i := by
            rintro rfl; exact hnw ⟨by omega, by omega⟩
          have e2 : q ≠ V.base r' + i + 1 := by
            rintro rfl; exact hnw ⟨by omega, by omega⟩
          rw [update_of_ne e1, update_of_ne e2]
          exact hrel.buf r' q (Or.inr hnw)
        · rw [update_of_ne hrr]
          refine hrel.buf r' q ?_
          rcases hq with hq | hq
          · left
            intro hd
            apply hq
            rcases hd with hd | ⟨h1, _⟩
            · exact Or.inl hd
            · exact absurd h1 hrr
          · exact Or.inr hq
      · -- the arena's orders
        intro x hq
        simp only [exchangeS] at hq ⊢
        have hxne : x ≠ s.slot r i ∧ x ≠ s.slot r (i + 1) → ordAfter V s r i x = s.arena x := by
          rintro ⟨h1, h2⟩
          rw [ordAfter_other V h1 h2]
          refine hrel.arena x ?_
          rcases hq with hq | hq | hq
          · left
            intro hd
            apply hq
            rcases hd with hd | ⟨h3, h4⟩
            · exact Or.inl hd
            · exact Or.inr ⟨h3, rfl⟩
          · exact Or.inr (Or.inl hq)
          · refine Or.inr (Or.inr ?_)
            intro k hk he
            obtain ⟨j, hj, hje⟩ := swapSlot_mem' V s (r := r) (i := i) hi hk
            exact hq j hj (he.trans hje)
        apply hxne
        constructor
        · rintro rfl
          rcases hq with hq | hq | hq
          · exact absurd (Or.inr ⟨rv, rfl⟩) hq
          · rw [rv] at hq; exact absurd hr hq
          · have := hq (i + 1) (by rw [rv]; exact hi)
            rw [rv, swapSlot_succ] at this
            exact absurd rfl this
        · rintro rfl
          rcases hq with hq | hq | hq
          · exact absurd (Or.inr ⟨rw', rfl⟩) hq
          · rw [rw'] at hq; exact absurd hr hq
          · have := hq i (by rw [rw']; omega)
            rw [rw', swapSlot_i] at this
            exact absurd rfl this
      · -- `candidate`, deferred
        simp only [exchangeS, ite_true]
        rw [hrel.cand]
        split_ifs
        · exact V.markCand_idem r s.cand
        · rfl
      · -- `valid`, deferred
        simp only [exchangeS, ite_true]
        rw [hrel.valid]
        split_ifs
        · exact V.markValid_idem r s.valid
        · rfl
      · simp only [exchangeS]; rw [hrel.rv]
      · simp only [exchangeS, exchangeC]; rw [hrel.log, hx]
    · have hf : V.exch (readCounts V s r i) = false := by simpa using hx
      rw [stepS_noexch V hskip hx, hstepC]
      simp only [hf]
      cases hm : s.memo r i with
      | some q =>
        exact ⟨hrel.order, hrel.slot, hrel.buf, hrel.arena, hrel.cand, hrel.valid, hrel.rv,
          by simp [hrel.log], hrel.moved, hrel.curView⟩
      | none =>
        exact ⟨hrel.order, hrel.slot, hrel.buf, hrel.arena, hrel.cand, hrel.valid, hrel.rv,
          by simp [hrel.log], hrel.moved, hrel.curView⟩

/-- A fold over the positions of a rank keeps both relations. -/
theorem steps_sim (sym : V.Sym) {r : ℕ} (hr : V.InView r) :
    ∀ (l : List ℕ), (∀ i ∈ l, i + 1 < V.n r) → ∀ (c : CState N) (s : SState N),
      Rel V r c s → Inv V s →
      Rel V r (l.foldl (stepC V r) c) (l.foldl (stepS V r) s) ∧ Inv V (l.foldl (stepS V r) s) := by
  intro l
  induction l with
  | nil => intro _ c s h1 h2; exact ⟨h1, h2⟩
  | cons i l ih =>
    intro hl c s h1 h2
    simp only [List.foldl_cons]
    have hi := hl i (by simp)
    exact ih (fun j hj => hl j (by simp [hj])) _ _
      (stepS_sim V h1 h2 hr hi) (stepS_inv V sym h2 hr hi)

/-- `transpose_step` against `scan_rank`: the flags are the same once the scan ends. -/
theorem stepAll_sim (sym : V.Sym) {r : ℕ} (hr : V.InView r) {c : CState N} {s : SState N}
    (hrel : Rel V r c s) (hs : Inv V s) (hx : s.exchanged = false) :
    Rel V r (stepAllC V r c) (stepAllS V r s) ∧ Inv V (stepAllS V r s) ∧
      (stepAllS V r s).exchanged = false := by
  set s0 : SState N := { s with cand := update s.cand r false, rv := 0, exchanged := false } with hs0
  set c0 : CState N := { c with cand := update c.cand r false, rv := 0 } with hc0
  have hdirty : ∀ r', Dirty r s0 r' ↔ Dirty r s r' := by
    intro r'; simp [Dirty, hs0, hx]
  have hrel0 : Rel V r c0 s0 := by
    refine ⟨hrel.order, hrel.slot, ?_, ?_, ?_, ?_, rfl, hrel.log, hrel.moved, by simp [hs0]⟩
    · intro r' p hp; rw [hdirty] at hp; exact hrel.buf r' p hp
    · intro x hp; rw [hdirty] at hp; exact hrel.arena x hp
    · have := hrel.cand; simp only [hx, Bool.false_eq_true, ite_false] at this
      simp [hc0, hs0, this]
    · have := hrel.valid; simp only [hx, Bool.false_eq_true, ite_false] at this
      simp [hc0, hs0, this]
  have hinv0 : Inv V s0 := hs.congr V rfl rfl rfl
  obtain ⟨h1, h2⟩ := steps_sim V sym hr (List.range (V.n r - 1))
    (fun i hi => by have := List.mem_range.mp hi; omega) c0 s0 hrel0 hinv0
  have hC : stepAllC V r c = (List.range (V.n r - 1)).foldl (stepC V r) c0 := rfl
  have hS : stepAllS V r s =
      (let s' := (List.range (V.n r - 1)).foldl (stepS V r) s0
       if s'.exchanged = true then
         { s' with
           moved := update s'.moved r true
           valid := V.markValid r s'.valid
           cand := V.markCand r s'.cand
           exchanged := false }
       else s') := rfl
  rw [hC, hS]
  set s' := (List.range (V.n r - 1)).foldl (stepS V r) s0 with hs'
  dsimp only
  by_cases hex : s'.exchanged = true
  · rw [ite_eq_left hex]
    refine ⟨?_, h2.congr V rfl rfl rfl, rfl⟩
    have hdirty' : ∀ r', Dirty r { s' with moved := update s'.moved r true, valid := V.markValid r s'.valid, cand := V.markCand r s'.cand, exchanged := false } r' ↔ (Dirty r s' r') := by
      intro r'
      by_cases hrr : r' = r
      · subst hrr; simp [Dirty, hex]
      · simp [Dirty, hrr, update_of_ne]
    refine ⟨h1.order, h1.slot, ?_, ?_, ?_, ?_, h1.rv, h1.log, ?_, by simp⟩
    · intro r' p hp; rw [hdirty'] at hp; exact h1.buf r' p hp
    · intro x hp; rw [hdirty'] at hp; exact h1.arena x hp
    · have := h1.cand; rw [ite_eq_left hex] at this; simpa using this
    · have := h1.valid; rw [ite_eq_left hex] at this; simpa using this
    · intro r' hm
      by_cases hrr : r' = r
      · subst hrr; exact hr
      · apply h1.moved r'
        simpa [update_of_ne hrr] using hm
  · rw [ite_eq_right hex]
    refine ⟨h1, h2, by simpa using hex⟩

/-- A round: the same ranks are scanned, and `delta` is the same. -/
theorem ranks_sim (sym : V.Sym) :
    ∀ (l : List ℕ), (∀ r ∈ l, V.InView r) → ∀ (st : CState N × ℤ) (st' : SState N × ℤ) (cur : ℕ),
      Rel V cur st.1 st'.1 → Inv V st'.1 → st'.1.exchanged = false → st.2 = st'.2 →
      let a := l.foldl (fun (st : CState N × ℤ) r =>
        if st.1.cand r = true then
          let c' := stepAllC V r st.1
          (c', V.arith.add st.2 c'.rv)
        else st) st
      let b := l.foldl (fun (st : SState N × ℤ) r =>
        if st.1.cand r = true then
          let s' := stepAllS V r st.1
          (s', V.arith.add st.2 s'.rv)
        else st) st'
      Rel V cur a.1 b.1 ∧ Inv V b.1 ∧ b.1.exchanged = false ∧ a.2 = b.2 := by
  intro l
  induction l with
  | nil => intro _ st st' cur h1 h2 h3 h4; exact ⟨h1, h2, h3, h4⟩
  | cons r l ih =>
    intro hl st st' cur h1 h2 h3 h4
    simp only [List.foldl_cons]
    have hr := hl r (by simp)
    have hcand : st.1.cand r = st'.1.cand r := by
      have := h1.cand; simp only [h3, Bool.false_eq_true, ite_false] at this; rw [this]
    by_cases hc : st'.1.cand r = true
    · have hc' : st.1.cand r = true := by rw [hcand]; exact hc
      rw [ite_eq_left hc, ite_eq_left hc']
      obtain ⟨a1, a2, a3⟩ := stepAll_sim V sym hr (h1.rebase V h3) h2 h3
      refine ih (fun r' hr' => hl r' (by simp [hr'])) _ _ cur (a1.rebase V a3) a2 a3 ?_
      show V.arith.add st.2 (stepAllC V r st.1).rv = V.arith.add st'.2 (stepAllS V r st'.1).rv
      rw [h4, a1.rv]
    · have hc' : ¬st.1.cand r = true := by rw [hcand]; exact hc
      rw [ite_eq_right hc, ite_eq_right hc']
      exact ih (fun r' hr' => hl r' (by simp [hr'])) _ _ cur h1 h2 h3 h4

theorem round_sim (sym : V.Sym) {cur : ℕ} {c : CState N} {s : SState N} (hrel : Rel V cur c s)
    (hs : Inv V s) (hx : s.exchanged = false) :
    Rel V cur (roundC V c).1 (roundS V s).1 ∧ Inv V (roundS V s).1 ∧
      (roundS V s).1.exchanged = false ∧ (roundC V c).2 = (roundS V s).2 :=
  ranks_sim V sym V.ranks (fun _ h => V.mem_ranks.mp h) (c, 0) (s, 0) cur hrel hs hx rfl

/-- §3.3, whole: after any number of rounds C's loop and the scan are in step, and each stops
exactly when the other does. Stated per round because the loop need not stop (#682). -/
theorem loop_sim (sym : V.Sym) {cur : ℕ} (k : ℕ) {c : CState N} {s : SState N}
    (hrel : Rel V cur c s) (hs : Inv V s) (hx : s.exchanged = false) :
    Rel V cur (loopC V k c).1 (loopS V k s).1 ∧ Inv V (loopS V k s).1 ∧
      (loopS V k s).1.exchanged = false ∧ (loopC V k c).2 = (loopS V k s).2 := by
  induction k generalizing c s with
  | zero => exact ⟨hrel, hs, hx, rfl⟩
  | succ k ih =>
    obtain ⟨a1, a2, a3, a4⟩ := round_sim V sym hrel hs hx
    simp only [loopC, loopS]
    by_cases hd : (roundS V s).2 < 1
    · have hd' : (roundC V c).2 < 1 := by rw [a4]; exact hd
      rw [ite_eq_left hd, ite_eq_left hd']
      exact ⟨a1, a2, a3, rfl⟩
    · have hd' : ¬(roundC V c).2 < 1 := by rw [a4]; exact hd
      rw [ite_eq_right hd, ite_eq_right hd']
      exact ih a1 a2 a3

end Sim

/-! ## `publish_scans` and the end state -/

section Publish

variable (V : View N P)

/-- One rank of `publish_scans`, position by position. -/
theorem publishRow_spec {s : SState N} {r : ℕ}
    (hinj : ∀ j k, j < V.n r → k < V.n r → s.slot r j = s.slot r k → j = k) :
    ∀ m, m ≤ V.n r →
      let t := (List.range m).foldl (publishAt V r) s
      t.order = s.order ∧ t.slot = s.slot ∧ t.moved = s.moved ∧
      (∀ r', r' ≠ r → t.buf r' = s.buf r') ∧
      (∀ i, i < m → t.buf r (V.base r + i) = s.slot r i) ∧
      (∀ q, (∀ i, i < m → q ≠ V.base r + i) → t.buf r q = s.buf r q) ∧
      (∀ i, i < m → t.arena (s.slot r i) = V.base r + i) ∧
      (∀ x, (∀ i, i < m → x ≠ s.slot r i) → t.arena x = s.arena x) := by
  intro m
  induction m with
  | zero => intro _; simp
  | succ m ih =>
    intro hm
    obtain ⟨o1, o2, o3, o4, o5, o6, o7, o8⟩ := ih (by omega)
    rw [List.range_succ, List.foldl_append]
    simp only [List.foldl_cons, List.foldl_nil]
    set t := (List.range m).foldl (publishAt V r) s
    refine ⟨o1, o2, o3, ?_, ?_, ?_, ?_, ?_⟩
    · intro r' hr'; simp only [publishAt, update_of_ne hr']; exact o4 r' hr'
    · intro i hi
      simp only [publishAt, update_self, o2]
      by_cases him : i = m
      · subst him; simp
      · rw [update_of_ne (by omega)]; exact o5 i (by omega)
    · intro q hq
      simp only [publishAt, update_self]
      rw [update_of_ne (hq m (by omega))]
      exact o6 q (fun i hi => hq i (by omega))
    · intro i hi
      simp only [publishAt, o2]
      by_cases him : i = m
      · subst him; simp
      · rw [update_of_ne (fun h => him (hinj _ _ (by omega) (by omega) h))]
        exact o7 i (by omega)
    · intro x hx
      simp only [publishAt, o2]
      rw [update_of_ne (hx m (by omega))]
      exact o8 x (fun i hi => hx i (by omega))

theorem publishRow_fields (s : SState N) (r : ℕ) (l : List ℕ) :
    (l.foldl (publishAt V r) s).order = s.order ∧ (l.foldl (publishAt V r) s).slot = s.slot ∧
      (l.foldl (publishAt V r) s).moved = s.moved := by
  induction l generalizing s with
  | nil => simp
  | cons i l ih => exact ih _

/-- After `publish_scans`, the rank buffers and `ND_order` are C's (§3.3, second deferred item). -/
theorem publish_eq {cur : ℕ} {c : CState N} {s : SState N} (hrel : Rel V cur c s) (hs : Inv V s)
    (hx : s.exchanged = false) :
    (publish V s).buf = c.buf ∧ (publish V s).arena = c.order := by
  have nd : ∀ r, Dirty cur s r ↔ s.moved r = true := by intro r; simp [Dirty, hx]
  -- the fold over the ranks, with what has been published so far
  have key : ∀ (rest done : List ℕ) (t : SState N), (done ++ rest).Nodup →
      (∀ r ∈ done ++ rest, V.InView r) →
      t.order = s.order → t.slot = s.slot → t.moved = s.moved →
      (∀ r, r ∈ done → t.buf r = c.buf r) → (∀ r, r ∉ done → t.buf r = s.buf r) →
      (∀ x, V.rankOf x ∈ done → t.arena x = c.order x) →
      (∀ x, V.rankOf x ∉ done → t.arena x = s.arena x) →
      let u := rest.foldl (fun s r =>
        if s.moved r = true then (List.range (V.n r)).foldl (publishAt V r) s else s) t
      (∀ r, r ∈ done ++ rest → u.buf r = c.buf r) ∧ (∀ r, r ∉ done ++ rest → u.buf r = s.buf r) ∧
      (∀ x, V.rankOf x ∈ done ++ rest → u.arena x = c.order x) ∧
      (∀ x, V.rankOf x ∉ done ++ rest → u.arena x = s.arena x) := by
    intro rest
    induction rest with
    | nil =>
      intro done t _ _ _ _ _ b1 b2 a1 a2
      simp only [List.append_nil, List.foldl_nil]
      exact ⟨b1, b2, a1, a2⟩
    | cons r rest ih =>
      intro done t hnd hv hto ts tm b1 b2 a1 a2
      simp only [List.foldl_cons]
      have hrd : r ∉ done := by
        intro h; have := (List.nodup_append.mp hnd).2.2 r h r (by simp); exact this rfl
      have hr : V.InView r := hv r (by simp)
      have e : done ++ r :: rest = (done ++ [r]) ++ rest := by simp
      rw [e] at hnd hv ⊢
      by_cases hm : t.moved r = true
      · rw [ite_eq_left hm]
        have hsm : s.moved r = true := by rw [← tm]; exact hm
        have hinj : ∀ j k, j < V.n r → k < V.n r → t.slot r j = t.slot r k → j = k := by
          rw [ts]; exact fun j k hj hk h => hs.slot_inj V hr hj hk h
        obtain ⟨_, _, _, p4, p5, p6, p7, p8⟩ := publishRow_spec V hinj (V.n r) le_rfl
        obtain ⟨f1, f2, f3⟩ := publishRow_fields V t r (List.range (V.n r))
        refine ih (done ++ [r]) _ hnd hv (f1.trans hto) (f2.trans ts) (f3.trans tm) ?_ ?_ ?_ ?_
        · intro r' hr'
          rcases List.mem_append.mp hr' with hr' | hr'
          · have hne : r' ≠ r := fun h => hrd (h ▸ hr')
            rw [p4 r' hne]; exact b1 r' hr'
          · obtain rfl : r' = r := by simpa using hr'
            funext q
            by_cases hq : ∃ i, i < V.n r' ∧ q = V.base r' + i
            · obtain ⟨i, hi, rfl⟩ := hq
              rw [p5 i hi, ts]; exact (hrel.slot r' i hr hi).symm
            · push Not at hq
              rw [p6 q hq, b2 r' hrd]
              refine (hrel.buf r' q (Or.inr ?_)).symm
              rintro ⟨h1, h2⟩
              exact hq (q - V.base r').toNat (by omega) (by omega)
        · intro r' hr'
          have hne : r' ≠ r := fun h => hr' (by simp [h])
          rw [p4 r' hne]; exact b2 r' (fun h => hr' (by simp [h]))
        · intro x hx'
          rcases List.mem_append.mp hx' with hx' | hx'
          · by_cases hxi : ∃ i, i < V.n r ∧ x = t.slot r i
            · obtain ⟨i, hi, rfl⟩ := hxi
              rw [ts] at hx'
              exact absurd ((hs.pos r i hr hi).2 ▸ hx') hrd
            · push Not at hxi
              rw [p8 x hxi]; exact a1 x hx'
          · have hxr : V.rankOf x = r := by simpa using hx'
            by_cases hxi : ∃ i, i < V.n r ∧ x = t.slot r i
            · obtain ⟨i, hi, rfl⟩ := hxi
              rw [p7 i hi, hrel.order, ts]
              exact (hs.pos r i hr hi).1.symm
            · push Not at hxi
              rw [p8 x hxi, a2 x (by rw [hxr]; exact hrd), hrel.order]
              refine (hrel.arena x (Or.inr (Or.inr ?_))).symm
              intro i hi
              rw [← ts, hxr]
              exact hxi i (by rw [← hxr]; exact hi)
        · intro x hx'
          have hnr : V.rankOf x ≠ r := fun h => hx' (by simp [h])
          have hxi : ∀ i, i < V.n r → x ≠ t.slot r i := by
            intro i hi h
            rw [ts] at h
            exact hnr (h ▸ (hs.pos r i hr hi).2)
          rw [p8 x hxi]; exact a2 x (fun h => hx' (by simp [h]))
      · rw [ite_eq_right hm]
        have hsm : ¬s.moved r = true := by rw [← tm]; exact hm
        refine ih (done ++ [r]) t hnd hv hto ts tm ?_ ?_ ?_ ?_
        · intro r' hr'
          rcases List.mem_append.mp hr' with hr' | hr'
          · exact b1 r' hr'
          · obtain rfl : r' = r := by simpa using hr'
            funext q
            rw [b2 r' hrd]
            exact (hrel.buf r' q (Or.inl (by rw [nd]; exact hsm))).symm
        · intro r' hr'; exact b2 r' (fun h => hr' (by simp [h]))
        · intro x hx'
          rcases List.mem_append.mp hx' with hx' | hx'
          · exact a1 x hx'
          · have hxr : V.rankOf x = r := by simpa using hx'
            rw [a2 x (by rw [hxr]; exact hrd), hrel.order]
            exact (hrel.arena x (Or.inl (by rw [nd, hxr]; exact hsm))).symm
        · intro x hx'; exact a2 x (fun h => hx' (by simp [h]))
  obtain ⟨b1, b2, a1, a2⟩ := key V.ranks [] s (by simp [View.ranks, List.nodup_range'])
    (fun r h => V.mem_ranks.mp (by simpa using h)) rfl rfl rfl (by simp) (fun _ _ => rfl)
    (by simp) (fun _ _ => rfl)
  unfold publish
  constructor
  · funext r
    by_cases h : r ∈ V.ranks
    · exact b1 r (by simpa using h)
    · rw [b2 r (by simpa using h)]
      funext q
      exact (hrel.buf r q (Or.inl (by
        rw [nd]; intro hm; exact h (V.mem_ranks.mpr (hrel.moved r hm))))).symm
  · funext x
    by_cases h : V.rankOf x ∈ V.ranks
    · exact a1 x (by simpa using h)
    · rw [a2 x (by simpa using h), hrel.order]
      exact (hrel.arena x (Or.inr (Or.inl (fun hv => h (V.mem_ranks.mpr hv))))).symm

end Publish

/-! ## (W) and `build_scan` -/

section Build

variable (V : View N P)

/-- (W) as the page states it: a node of the view that is not in its rank's window has an order
outside that window. -/
def WProse (ord : N → ℤ) (slot : ℕ → ℕ → N) : Prop :=
  ∀ x, V.InView (V.rankOf x) → (∀ i, i < V.n (V.rankOf x) → x ≠ slot (V.rankOf x) i) →
    ¬InWin V (V.rankOf x) (ord x)

/-- The code's test of (W), `windows_hold_their_orders`, is the page's (W) on a window that is a
proven permutation. So the check declines exactly the views that break (W). -/
theorem holds_iff_w {ord : N → ℤ} {slot : ℕ → ℕ → N}
    (hpos : ∀ r i, V.InView r → i < V.n r → ord (slot r i) = V.base r + i) :
    (∀ x, holdsAt V ord slot x = true) ↔ WProse V ord slot := by
  constructor
  · intro h x hv hnot hw
    have := h x
    unfold holdsAt at this
    rw [ite_eq_left hv] at this
    simp only [Bool.or_eq_true, decide_eq_true_eq] at this
    obtain ⟨h1, h2⟩ := hw
    rcases this with (h3 | h3) | h3
    · omega
    · omega
    · exact hnot (ord x - V.base (V.rankOf x)).toNat (by omega) h3.symm
  · intro h x
    simp only [holdsAt]
    split_ifs with hv
    · simp only [Bool.or_eq_true, decide_eq_true_eq]
      by_cases hin : InWin V (V.rankOf x) (ord x)
      · right
        by_contra hne
        have := h x hv ?_
        · exact this hin
        · intro i hi hxi
          apply hne
          have := hpos _ i hv hi
          rw [← hxi] at this
          rw [this]
          have e : (V.base (V.rankOf x) + (i : ℤ) - V.base (V.rankOf x)).toNat = i := by omega
          rw [e]; exact hxi.symm
      · left
        unfold InWin at hin
        omega
    · rfl

/-- What `build_scan` establishes when it accepts: the invariant, with every position unknown, and
the relation to C's state before the first scan. `hslots`: the compact slots are all the nodes
the counts can read (`prepare_mincross` takes the windows' nodes and both ends of their edges). -/
theorem buildScan_spec {c : CState N} {s : SState N} (hslots : ∀ x, x ∈ V.slots)
    (hb : buildScan V c = some s) : Inv V s ∧ Rel V 0 c s ∧ s.exchanged = false := by
  unfold buildScan at hb
  dsimp only at hb
  split_ifs at hb with hchk
  obtain rfl := (Option.some.inj hb).symm
  obtain ⟨hrows, hw⟩ := hchk
  simp only [List.all_eq_true, List.mem_range, rowOk, Bool.and_eq_true, decide_eq_true_eq] at hrows
  have row : ∀ r i, V.InView r → i < V.n r → _ := fun r i hr hi =>
    hrows r (V.mem_ranks.mpr hr) i hi
  have hpos : ∀ r i, V.InView r → i < V.n r →
      c.order (c.buf r (V.base r + i)) = V.base r + i ∧ V.rankOf (c.buf r (V.base r + i)) = r :=
    fun r i hr hi => ⟨(row r i hr hi).1.1.2, (row r i hr hi).1.1.1⟩
  refine ⟨⟨hpos, ?_, ?_, ?_⟩, ?_, rfl⟩
  · intro r i hr hi
    obtain ⟨⟨_, h3⟩, h4⟩ := row r i hr hi
    exact ⟨h3, h4⟩
  · intro x i hv hi ho
    dsimp only at ho ⊢
    simp only [List.all_eq_true] at hw
    have := hw x (hslots x)
    simp only [holdsAt, ite_eq_left hv, Bool.or_eq_true, decide_eq_true_eq] at this
    have hk : (c.order x - V.base (V.rankOf x)).toNat = i := by omega
    rcases this with (h1 | h1) | h1
    · omega
    · omega
    · rw [hk] at h1; exact h1.symm
  · intro r i p _ _ h; simp at h
  · refine ⟨rfl, fun _ _ _ _ => rfl, fun _ _ _ => rfl, fun _ _ => rfl, by simp, by simp, rfl, rfl,
      by simp, by simp⟩

/-- **§3.3.** When `build_scan` accepts, C's loop and the scan, run for any number of rounds,
make the same reads with the same counts and the same exchange decisions in the same order
(`log`), stop together (`.2`), and agree on everything C leaves: orders, `candidate`, `valid` and,
once `publish_scans` has run, the rank buffers and `ND_order`. -/
theorem transpose_same (sym : V.Sym) (hslots : ∀ x, x ∈ V.slots) {c : CState N} {s : SState N}
    (hb : buildScan V c = some s) (k : ℕ) :
    let a := loopC V k c
    let b := loopS V k s
    a.2 = b.2 ∧ a.1.log = b.1.log ∧ a.1.order = b.1.order ∧ a.1.cand = b.1.cand ∧
      a.1.valid = b.1.valid ∧ (publish V b.1).buf = a.1.buf ∧ (publish V b.1).arena = a.1.order := by
  obtain ⟨hs, hrel, hx⟩ := buildScan_spec V hslots hb
  obtain ⟨r1, r2, r3, r4⟩ := loop_sim V sym k hrel hs hx
  have hc := r1.cand
  have hv := r1.valid
  simp only [r3, Bool.false_eq_true, ite_false] at hc hv
  obtain ⟨p1, p2⟩ := publish_eq V r1 r2 r3
  exact ⟨r4, r1.log, r1.order, hc, hv, p1, p2⟩

end Build

/-! ## Ports

The Rust comparison is `e1.port_x > e2.port_x` on `f64`. Nothing above looks inside it: `portGt` is
used once, in `View.crosses`, and `crosses_congr` (the only lemma about `crosses`) rewrites the
order part of the test and leaves the port part alone, because the two sides of every congruence
are the same edges with the same ports. So `transpose_same` holds for **every** `portGt`: it does
not need irreflexivity, asymmetry, transitivity, totality or even that equal ports compare
alike. A NaN port, `±∞`, `-0.0` against `0.0`, or a comparison that is not a function of the
value at all, are all inside the theorem. What the theorem does need is that C's loop and the
scan evaluate the same `portGt` on the same pair of port values, which holds because both call
`in_cross`/`out_cross`.

`F64 α` is a port as IEEE `>` sees it: a non-NaN value in a linear order `α` (so `±∞` and the
signed zeros live in `α`) or NaN (`none`). `F64.gt` is `>` with the IEEE rule that an ordered
comparison with a NaN operand is false. Its failures, below, are the properties a proof about
`ℤ` ports could have used and this one does not. -/

section Ports

/-- A binary64 port x: `some a` for any non-NaN value (including `±∞`), `none` for NaN. -/
abbrev F64 (α : Type) := Option α

/-- IEEE `>`: false when either operand is NaN. -/
def F64.gt {α : Type} [LT α] [DecidableRel (α := α) (· < ·)] : F64 α → F64 α → Bool
  | some a, some b => decide (b < a)
  | _, _ => false

variable {α : Type} [LinearOrder α]

/-- On non-NaN values `F64.gt` is the `>` of `α`, which is the old model with `α := ℤ`. -/
theorem F64.gt_some (a b : α) : F64.gt (some a) (some b) = decide (a > b) := rfl

theorem F64.gt_nan_left (b : F64 α) : F64.gt none b = false := by cases b <;> rfl

theorem F64.gt_nan_right (a : F64 α) : F64.gt a none = false := by cases a <;> rfl

/-- Irreflexivity survives, NaN included: `NaN > NaN` is false. -/
theorem F64.gt_irrefl (a : F64 α) : F64.gt a a = false := by
  cases a with
  | none => rfl
  | some a => simp [F64.gt]

/-- Asymmetry survives. -/
theorem F64.gt_asymm (a b : F64 α) (h : F64.gt a b = true) : F64.gt b a = false := by
  cases a <;> cases b <;> simp_all [F64.gt]
  rename_i x y
  exact le_of_lt h

/-- What does not survive: totality. With a NaN, neither `a > b`, nor `b > a`, nor equality. The
ℤ model's order embedding of the ports needs this, so a NaN port is outside that argument; the
theorem for an abstract `portGt` does not use it. -/
theorem F64.not_total (a : α) :
    F64.gt none (some a) = false ∧ F64.gt (some a) none = false ∧ (none : F64 α) ≠ some a :=
  ⟨rfl, rfl, by simp⟩

/-- Also gone: "not greater" is not transitive, so `¬ (· > ·)` is not a preorder. With `c < a`:
`a ≯ NaN` and `NaN ≯ c`, yet `a > c`. A proof that read `≯` as `≤` (to sort ports, say) would
break at the first NaN. -/
theorem F64.not_gt_not_trans (a c : α) (h : c < a) :
    F64.gt (some a) none = false ∧ F64.gt none (some c) = false ∧ F64.gt (some a) (some c) = true := by
  refine ⟨rfl, rfl, ?_⟩
  simp [F64.gt, h]

end Ports

/-- **`transpose_same` for IEEE ports.** `V` is a view whose port comparison is `F64.gt`, NaN
ports allowed. The conclusion is the one of `transpose_same`; the proof is that theorem. -/
theorem transpose_same_f64 {α : Type} [LT α] [DecidableRel (α := α) (· < ·)] (V : View N (F64 α))
    (_hgt : V.portGt = F64.gt) (sym : V.Sym) (hslots : ∀ x, x ∈ V.slots) {c : CState N}
    {s : SState N} (hb : buildScan V c = some s) (k : ℕ) :
    let a := loopC V k c
    let b := loopS V k s
    a.2 = b.2 ∧ a.1.log = b.1.log ∧ a.1.order = b.1.order ∧ a.1.cand = b.1.cand ∧
      a.1.valid = b.1.valid ∧ (publish V b.1).buf = a.1.buf ∧ (publish V b.1).arena = a.1.order :=
  transpose_same V sym hslots hb k

/-- The same for Lean's own `Float`, whose `<` is the machine `<` (false with a NaN operand). Rust's
`a > b` is `b < a`. -/
theorem transpose_same_float (V : View N Float) (_hgt : V.portGt = fun a b => decide (b < a))
    (sym : V.Sym) (hslots : ∀ x, x ∈ V.slots) {c : CState N} {s : SState N}
    (hb : buildScan V c = some s) (k : ℕ) :
    let a := loopC V k c
    let b := loopS V k s
    a.2 = b.2 ∧ a.1.log = b.1.log ∧ a.1.order = b.1.order ∧ a.1.cand = b.1.cand ∧
      a.1.valid = b.1.valid ∧ (publish V b.1).buf = a.1.buf ∧ (publish V b.1).arena = a.1.order :=
  transpose_same V sym hslots hb k

/-! ## The shipped arithmetic -/

/-- Two's-complement wrapping to `bits` bits. -/
def wrap (bits : ℕ) (x : ℤ) : ℤ := Int.bmod x (2 ^ bits)

/-- `in_cross` adds `i64` products of `i32` penalties (`cross += a as i64 * b`), which cannot
overflow on real penalties; `out_cross` multiplies and adds on `i32`, wrapping; the scan's and the
reference's `c0`, `c1`, `rv` and `delta` add on `i64`, wrapping. `transpose_same` holds for this
and for any other choice. -/
def Arith.rust : Arith where
  inAdd acc a b := acc + a * b
  outAdd acc a b := wrap 32 (acc + wrap 32 (a * b))
  add a b := wrap 64 (a + b)
  sub a b := wrap 64 (a - b)

/-! ## The hypotheses can hold

Two ranks of two nodes, `0 1` over `2 3`, with edges `0 → 3` and `1 → 2`: one crossing. The
first round exchanges `0` and `1`, the second finds nothing, and the scan reads what C reads. -/

section Example

/-- The view. -/
def exV : View (Fin 4) ℤ where
  rankOf x := if x.val < 2 then 0 else 1
  inE x := match x.val with
    | 2 => [⟨1, 1, 0⟩]
    | 3 => [⟨0, 1, 0⟩]
    | _ => []
  outE x := match x.val with
    | 0 => [⟨3, 1, 0⟩]
    | 1 => [⟨2, 1, 0⟩]
    | _ => []
  minr := 0
  maxr := 1
  base _ := 0
  n _ := 2
  hasOut r := r == 0
  left2right _ _ := false
  constrained := false
  reverse := false
  arith := Arith.rust
  portGt a b := decide (a > b)
  slots := [0, 1, 2, 3]

/-- C's state before `transpose`. -/
def exC : CState (Fin 4) where
  order x := if x.val < 2 then x.val else x.val - 2
  buf r p := if r = 0 then (if p = 0 then 0 else 1) else (if p = 0 then 2 else 3)
  cand _ := true
  valid _ := true
  rv := 0
  log := []

theorem exV_sym : exV.Sym := by unfold View.Sym; decide

theorem exV_slots : ∀ x, x ∈ exV.slots := by decide

theorem exC_build : (buildScan exV exC).isSome = true := by decide

theorem exC_log : (loopC exV 3 exC).2 = true ∧ (loopC exV 3 exC).1.log =
    [⟨0, 0, (1, 0), true⟩, ⟨1, 0, (0, 1), false⟩, ⟨0, 0, (0, 1), false⟩] := by decide

end Example

end GvpProofs.Dot.MincrossScans
