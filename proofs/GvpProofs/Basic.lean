import Mathlib.Combinatorics.SimpleGraph.Metric

/-!
# graphviz-pure proofs

Root module of the `GvpProofs` library. Each proof lives in its own file under
`GvpProofs/<Engine>/` and is picked up by the glob in `lakefile.toml`; nothing imports this
file. It exists so an empty library still type-checks Mathlib's graph metric, the dependency
most proofs here use.

Conventions:
* A theorem states that a fast path computes the same integers (or the same IEEE doubles via
  the same rounding sequence) as the C-mirror path. Never "a better approximation".
* Prefer stating theorems over the Aeneas-extracted Rust in `GvpProofs/Extracted/`; when
  extraction rejects a module, state them over a Lean model and name the gap in the PR.
* No `sorry` on `master`. A proof in progress stays on its branch.
-/

namespace GvpProofs

/-- Smoke theorem: adjacent vertices are at distance one. Keeps the Mathlib import honest. -/
theorem dist_adj_eq_one {V : Type*} (G : SimpleGraph V) {u v : V} (h : G.Adj u v) :
    G.dist u v = 1 :=
  SimpleGraph.dist_eq_one_iff_adj.mpr h

end GvpProofs
