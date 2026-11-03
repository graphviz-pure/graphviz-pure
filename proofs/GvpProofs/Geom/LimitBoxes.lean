import FloatLib.Floats.Formats.BinaryInterchange.Semantics

/-!
# `limit_boxes`: after `delta` doubles, the even samples need not be taken again

Rust: `crates/graphviz-pure-layout/src/common/routespl.rs`, `limit_boxes_samples` and the retry
loop of `routesplines_`. C: `lib/common/routespl.c`, `limitBoxes` and its caller.

C's retry loop calls `limitBoxes(boxes, boxn, ps, pn, delta)` with `delta = 10`, and while some box
is still unbounded doubles `delta` and calls it again, on the same `boxes` (they are not reset), at
most 15 times. Each call takes the samples `si = 0, 1, .., num_div` of every cubic segment, with
`num_div = delta * boxn` and `t = si / num_div`, and widens every box whose `y` range holds the
sample's `y` with `fmin`/`fmax` on `x`.

## What is proved

With `D = 10 * 2^j * boxn` the `num_div` of pass `j` (everything in binary64, round to nearest even):

1. `deltaSeq_eq`, `numDiv_eq`: `delta` and `num_div` are the whole numbers `10 * 2^j` and `D`, with
   no rounding, while `10 * 2^j * boxn < 2^53`. `siSeq_eq`, `oddSeq_eq`: `si` is the whole number
   `k` after `k` steps of `si++` (and `2i + 1` after `i` steps of `si += 2`). `le_ofNat64_iff`: the
   loop test `si <= num_div` is the order of those numbers.
2. `div_ofNat64_double`: `fl(2k) / fl(2N)` and `fl(k) / fl(N)` are the **same binary64 word**
   (sign of zero included). No power-of-two scaling, overflow or underflow argument is needed, and
   the ticket's premise that one is needed was too narrow: a binary64 quotient is the real
   quotient rounded once, and `2k / 2N = k / N` as reals.
3. `t_even_eq`: so the parameter of sample `2k` at pass `j + 1` is the parameter of sample `k` at
   pass `j`, bit for bit; `t_odd_eq`: the odd retry's `si += 2.0` produces the odd samples'
   parameters. A sample is a function of its segment and `t` alone, so the sample *values* repeat
   too.
4. `retry_odd_eq_full`, for any step function that absorbs a sample it has already taken
   (`Absorbing`): the full pass at `j + 1` and the odd-only retry leave the same state, when
   started from the state after pass `j`. `absorbing_box`: `fmin`/`fmax` on `x` behind the `y`
   test is such a step. `limit_boxes_retry_exact` is the statement for one box.

## Model against Rust

* Rust calls `f64::min`/`f64::max`, C `fmin`/`fmax`. For `<` that is a total order they agree
  with `stepMin`/`stepMax` here except where the two arguments are `+0.0` and `-0.0`: then either
  one may come back, and which one is the code generator's choice (x86 `minsd` returns its second
  operand, wasm `f64.min` returns `-0.0`). The Lean step keeps the current limit on a tie. The
  Rust guard `may_skip_even_samples` takes the retry only when no limit is a zero, where a tie
  means equal bits. That argument is in prose (`docs/perf/limit-boxes-reuse.md`), not in Lean.
* The sample's value is an abstract function `sample : segment -> t -> Sample`: that it reads only
  its segment's four control points and `t` is read off the Rust (`sp` is copied from `pps` for
  every `si`), not proved. The `y` test `inBox` is likewise a function of the sample, because the
  loop writes only `x`.
* The Rust loops are the lists `fullItems`/`oddItems` (segments in order, `si` ascending). The
  `<` on box limits is any strict order (`htrans`, `hirr`): binary64's `<`, NaNs included, is one.
* Nothing here is extracted from the Rust (Aeneas has no floats): the statement is over FloatLib's
  `Model binary64`, the Flocq-style bit-exact model of the format.
-/

open FloatLib FloatLib.Floats.Formats FloatLib.Floats.Formats.BinaryInterchange
  FloatLib.Floats.Formats.Flocq FloatLib.Floats

namespace GvpProofs.LimitBoxes

abbrev F64 : FloatFormat := FloatFormat.binary64
abbrev D64 : Type := Model F64

theorem F64_isIEEE : F64.isIEEE = true := by decide
theorem F64_frac : F64.fracWidth = 52 := rfl
theorem F64_minSub : F64.minSubnormalExponent = -1074 := by decide

/-- `n as f64` -/
noncomputable def ofNat64 (n : ℕ) : D64 := Model.roundDyadic F64 ⟨false, n, 0⟩

theorem posMax_ge : (2 : ℝ) ^ 53 ≤ Model.toReal (Model.posMaxFinite F64) := by
  rw [Model.toReal_posMaxFinite]
  have h3 : F64.maxFiniteFracField = 2 ^ 52 - 1 := by decide
  have h4 : Model.pow2 F64.fracWidth = 2 ^ 52 := by decide
  have h5 : F64.maxNormalExponent - Int.ofNat F64.fracWidth = 971 := by decide
  rw [h3, h4, h5]
  have hb : (bpow Numerics.binaryRadix 971 : ℝ) = 2 ^ 971 := by
    simp [bpow, Numerics.binaryRadix, Numerics.Radix.toReal]
  unfold Model.bpow
  rw [hb]
  push_cast
  norm_num
  calc (9007199254740992 : ℝ) ≤ 9007199254740991 * 2 := by norm_num
    _ ≤ 9007199254740991 * 2 ^ 971 := by
      have h2 : (2:ℝ) ≤ 2 ^ 971 := by
        calc (2:ℝ) = 2 ^ 1 := by norm_num
          _ ≤ 2 ^ 971 := pow_le_pow_right₀ (by norm_num) (by norm_num)
      gcongr

theorem ofNat64_spec (n : ℕ) (hn : n < 2 ^ 53) :
    Model.isFinite (ofNat64 n) = true ∧ Model.toReal (ofNat64 n) = n := by
  have h := Model.roundDyadic_of_representable F64 F64_isIEEE ⟨false, n, 0⟩ n 0
    (by simp) (by rw [F64_frac]; simpa using hn) (by rw [F64_minSub]; simp)
    (by
      have : |(⟨false, n, 0⟩ : Numerics.Dyadic).toReal| = n := by
        simp [Numerics.Dyadic.toReal]
      rw [this]
      exact le_trans (by exact_mod_cast hn.le) posMax_ge)
  simpa [ofNat64, Numerics.Dyadic.toReal] using h


theorem ofNat64_finite {n : ℕ} (hn : n < 2 ^ 53) : Model.isFinite (ofNat64 n) = true :=
  (ofNat64_spec n hn).1

theorem toReal_ofNat64 {n : ℕ} (hn : n < 2 ^ 53) : Model.toReal (ofNat64 n) = n :=
  (ofNat64_spec n hn).2

theorem ofNat64_abs_le {n : ℕ} (hn : n < 2 ^ 53) :
    |(n : ℝ)| ≤ Model.toReal (Model.posMaxFinite F64) := by
  rw [abs_of_nonneg (Nat.cast_nonneg n)]
  exact le_trans (by exact_mod_cast hn.le) posMax_ge

/-- Every natural number below `2^53` is a binary64 value: nearest-even rounding fixes it. -/
theorem roundAt_natCast {n : ℕ} (hn : n < 2 ^ 53) : Model.roundAt F64 (n : ℝ) = n := by
  apply Model.roundAt_of_genericFormat
  refine generic_of_fltFormat (β := Numerics.binaryRadix) F64.minSubnormalExponent
    (F64.fracWidth + 1) (by norm_num [F64_frac]) _ ?_
  refine ⟨by norm_num [F64_frac], ⟨⟨n, 0⟩, ?_, ?_, by rw [F64_minSub]; norm_num⟩⟩
  · simp [toReal, bpow]
  · have hb : Numerics.binaryRadix.base = 2 := rfl
    rw [F64_frac, hb]; simpa using hn


theorem ofNat64_ne_zero {n : ℕ} (hn : n < 2 ^ 53) (h0 : 0 < n) : Model.toReal (ofNat64 n) ≠ 0 := by
  rw [toReal_ofNat64 hn]; exact_mod_cast h0.ne'

/-- Adding naturals below `2^53` in binary64 is exact: `fl(m + n) = m + n`. -/
theorem add_ofNat64 {m n : ℕ} (h : m + n < 2 ^ 53) (h0 : 0 < m + n) :
    Model.add (ofNat64 m) (ofNat64 n) = ofNat64 (m + n) := by
  have hm : m < 2 ^ 53 := by omega
  have hn : n < 2 ^ 53 := by omega
  have hfin : Model.isFinite (Model.add (ofNat64 m) (ofNat64 n)) = true :=
    Model.isFinite_add_of_abs_toReal_add_le_posMaxFinite _ _ F64_isIEEE (ofNat64_finite hm)
      (ofNat64_finite hn) (by
        rw [toReal_ofNat64 hm, toReal_ofNat64 hn]
        have := ofNat64_abs_le h
        push_cast at this
        exact this)
  have hreal := Model.toReal_add_eq_roundAt _ _ F64_isIEEE (ofNat64_finite hm)
    (ofNat64_finite hn) hfin
  rw [toReal_ofNat64 hm, toReal_ofNat64 hn, ← Nat.cast_add, roundAt_natCast h] at hreal
  exact Model.eq_of_toReal_eq_of_nonzero hfin (ofNat64_finite h) (hreal.trans (toReal_ofNat64 h).symm)
    (by rw [hreal]; exact_mod_cast h0.ne')

/-- Multiplying naturals whose product is below `2^53` in binary64 is exact. -/
theorem mul_ofNat64 {m n : ℕ} (h : m * n < 2 ^ 53) (h0 : 0 < m * n) :
    Model.mul (ofNat64 m) (ofNat64 n) = ofNat64 (m * n) := by
  have hm0 : 0 < m := Nat.pos_of_mul_pos_right h0
  have hn0 : 0 < n := Nat.pos_of_mul_pos_left h0
  have hm : m < 2 ^ 53 := lt_of_le_of_lt (Nat.le_mul_of_pos_right m hn0) h
  have hn : n < 2 ^ 53 := lt_of_le_of_lt (Nat.le_mul_of_pos_left n hm0) h
  have hfin : Model.isFinite (Model.mul (ofNat64 m) (ofNat64 n)) = true :=
    Model.isFinite_mul_of_abs_mul_le_posMaxFinite _ _ F64_isIEEE (ofNat64_finite hm)
      (ofNat64_finite hn) (by
        rw [toReal_ofNat64 hm, toReal_ofNat64 hn]
        have := ofNat64_abs_le h
        rw [Nat.cast_mul] at this
        rw [← abs_mul]; exact this)
  have hreal := Model.toReal_mul_eq_roundAt _ _ F64_isIEEE (ofNat64_finite hm)
    (ofNat64_finite hn) hfin
  rw [toReal_ofNat64 hm, toReal_ofNat64 hn, ← Nat.cast_mul, roundAt_natCast h] at hreal
  exact Model.eq_of_toReal_eq_of_nonzero hfin (ofNat64_finite h) (hreal.trans (toReal_ofNat64 h).symm)
    (by rw [hreal]; exact_mod_cast h0.ne')


/-- `2^-53` is a binary64 value, so nearest-even rounding fixes it. -/
theorem roundAt_bpow_neg53 : Model.roundAt F64 (bpow Numerics.binaryRadix (-53)) = bpow Numerics.binaryRadix (-53) := by
  apply Model.roundAt_of_genericFormat
  apply generic_format_bpow
  simp [Model.fexpOf, fltExp, F64_minSub, F64_frac]

/-- A positive quotient of naturals below `2^53` does not round to zero. -/
theorem roundAt_div_pos {k N : ℕ} (hk : 0 < k) (hN : 0 < N) (hN2 : N < 2 ^ 53) :
    0 < Model.roundAt F64 ((k : ℝ) / N) := by
  have h1 : bpow Numerics.binaryRadix (-53) ≤ (k : ℝ) / N := by
    have hb : (bpow Numerics.binaryRadix (-53) : ℝ) = 1 / 2 ^ 53 := by
      simp [bpow, Numerics.binaryRadix, Numerics.Radix.toReal]
    rw [hb]
    have hN' : (N : ℝ) ≤ 2 ^ 53 := by exact_mod_cast hN2.le
    have hk' : (1 : ℝ) ≤ k := by exact_mod_cast hk
    have hNpos : (0 : ℝ) < N := by exact_mod_cast hN
    rw [div_le_div_iff₀ (by positivity) hNpos]
    nlinarith
  have := Model.roundAt_mono F64 h1
  rw [roundAt_bpow_neg53] at this
  exact lt_of_lt_of_le (by simp [bpow, Numerics.binaryRadix, Numerics.Radix.toReal]) this


/-- binary64 division of two naturals below `2^53` (each exact) is one rounding of the real
quotient, and the quotient does not overflow. -/
theorem toReal_div_ofNat64 {k N : ℕ} (hk : k < 2 ^ 53) (hN : 0 < N) (hN2 : N < 2 ^ 53) :
    Model.isFinite (Model.div (ofNat64 k) (ofNat64 N)) = true ∧
    Model.toReal (Model.div (ofNat64 k) (ofNat64 N)) = Model.roundAt F64 ((k : ℝ) / N) := by
  have hbound : |Model.toReal (ofNat64 k) / Model.toReal (ofNat64 N)| ≤
      Model.toReal (Model.posMaxFinite F64) := by
    rw [toReal_ofNat64 hk, toReal_ofNat64 hN2, abs_of_nonneg (by positivity)]
    have hNpos : (1 : ℝ) ≤ N := by exact_mod_cast hN
    calc (k : ℝ) / N ≤ k := div_le_self (Nat.cast_nonneg k) hNpos
      _ ≤ _ := by simpa using ofNat64_abs_le hk
  have hy0 : Model.isZero (ofNat64 N) = false := by
    by_contra h
    have h' : Model.isZero (ofNat64 N) = true := by simpa using h
    have := Model.toReal_eq_zero_of_isZero _ h'
    rw [toReal_ofNat64 hN2] at this
    exact absurd this (by exact_mod_cast hN.ne')
  exact ⟨Model.isFinite_div_of_abs_div_le_posMaxFinite _ _ F64_isIEEE (ofNat64_finite hk)
    (ofNat64_finite hN2) hy0 hbound, by
      rw [Model.toReal_div_eq_roundAt_of_abs_div_le_posMaxFinite _ _ F64_isIEEE
        (ofNat64_finite hk) (ofNat64_finite hN2) hy0 hbound, toReal_ofNat64 hk,
        toReal_ofNat64 hN2]⟩

/-- A zero numerator over a positive finite denominator gives a zero of the numerator's sign. -/
theorem div_zero_numerator {x y : D64} (hx : Model.isZero x = true) (hy : Model.isFinite y = true)
    (hy0 : Model.isZero y = false) (hp : 0 < Model.toReal y) :
    Model.div x y = Model.zero F64 (Model.signBit x) := by
  obtain ⟨dx, hdx⟩ := Model.exists_toDyadic?_of_isFinite
    (Model.isFinite_eq_true_of_isZero_eq_true x hx)
  obtain ⟨dy, hdy⟩ := Model.exists_toDyadic?_of_isFinite hy
  have hdx' := Model.toDyadic?_eq_zero_of_isZero_eq_true x hx
  rw [hdx] at hdx'
  cases hdx'
  have hdyMant : dy.significand ≠ 0 := by
    intro hmant
    have hzero := Model.isZero_eq_true_of_toDyadic?_some_of_mant_eq_zero hdy hmant
    simp [hy0] at hzero
  have hneg : dy.negative = false := by
    by_contra h
    have h' : dy.negative = true := by simpa using h
    have : Model.toReal y = dy.toReal := by simp [Model.toReal_eq, hdy]
    rw [this] at hp
    simp [Numerics.Dyadic.toReal, Numerics.Dyadic.signedSignificand, h'] at hp
    have : 0 ≤ (dy.significand : ℝ) * 2 ^ dy.exponent := by positivity
    linarith
  simp [Model.Proof.div_eq_spec, Model.Spec.div, hdx, hdy, hdyMant, hneg]

theorem ofNat64_isZero_zero : Model.isZero (ofNat64 0) = true := by
  have hf := ofNat64_finite (n := 0) (by norm_num)
  have h0 : Model.toReal (ofNat64 0) = 0 := by
    simpa using toReal_ofNat64 (n := 0) (by norm_num)
  exact (Model.isZero_eq_true_iff_toReal_eq_zero _ hf).2 h0

theorem ofNat64_isZero_false {n : ℕ} (hn : n < 2 ^ 53) (h0 : 0 < n) :
    Model.isZero (ofNat64 n) = false := by
  by_contra h
  have h' : Model.isZero (ofNat64 n) = true := by simpa using h
  have := Model.toReal_eq_zero_of_isZero _ h'
  rw [toReal_ofNat64 hn] at this
  exact absurd this (by exact_mod_cast h0.ne')

theorem toReal_ofNat64_pos {n : ℕ} (hn : n < 2 ^ 53) (h0 : 0 < n) :
    0 < Model.toReal (ofNat64 n) := by
  rw [toReal_ofNat64 hn]; exact_mod_cast h0

/-- **The lemma of the ticket.** Doubling numerator and denominator, both whole numbers that
binary64 holds exactly, does not change the quotient: `fl(2k) / fl(2N)` and `fl(k) / fl(N)` are the
same word. Nothing about powers of two is used. A binary64 quotient is the real quotient rounded
once, and `2k / 2N = k / N` as reals. -/
theorem div_ofNat64_double {k N : ℕ} (hN : 0 < N) (hk : 2 * k < 2 ^ 53) (hN2 : 2 * N < 2 ^ 53) :
    Model.div (ofNat64 (2 * k)) (ofNat64 (2 * N)) = Model.div (ofNat64 k) (ofNat64 N) := by
  rcases Nat.eq_zero_or_pos k with rfl | hk0
  · have hN20 : 0 < 2 * N := by omega
    rw [Nat.mul_zero,
      div_zero_numerator ofNat64_isZero_zero (ofNat64_finite hN2)
        (ofNat64_isZero_false hN2 hN20) (toReal_ofNat64_pos hN2 hN20),
      div_zero_numerator ofNat64_isZero_zero (ofNat64_finite (by omega))
        (ofNat64_isZero_false (by omega) hN) (toReal_ofNat64_pos (by omega) hN)]
  have hk1 : k < 2 ^ 53 := by omega
  have hN1 : N < 2 ^ 53 := by omega
  have hN20 : 0 < 2 * N := by omega
  obtain ⟨hf2, hr2⟩ := toReal_div_ofNat64 hk hN20 hN2
  obtain ⟨hf1, hr1⟩ := toReal_div_ofNat64 hk1 hN hN1
  have hq : ((2 * k : ℕ) : ℝ) / ((2 * N : ℕ) : ℝ) = (k : ℝ) / N := by
    push_cast
    exact mul_div_mul_left _ _ (by norm_num)
  rw [hq] at hr2
  exact Model.eq_of_toReal_eq_of_nonzero hf2 hf1 (hr2.trans hr1.symm)
    (by rw [hr2]; exact (roundAt_div_pos hk0 hN hN1).ne')


/-- The loop test `si <= num_div` on two whole numbers is the order of the numbers, so the loop
runs for `si = 0, 1, .., D` and no further. -/
theorem le_ofNat64_iff {n D : ℕ} (hn : n < 2 ^ 53) (hD : D < 2 ^ 53) :
    Model.le (ofNat64 n) (ofNat64 D) ↔ n ≤ D := by
  have hlt := Model.compare_eq_some_lt_iff_toReal_lt_of_isFinite (ofNat64 n) (ofNat64 D)
    (ofNat64_finite hn) (ofNat64_finite hD)
  have heq := Model.compare_eq_some_eq_iff_toReal_eq_of_isFinite (ofNat64 n) (ofNat64 D)
    (ofNat64_finite hn) (ofNat64_finite hD)
  have hgt := Model.compare_eq_some_gt_iff_toReal_gt_of_isFinite (ofNat64 n) (ofNat64 D)
    (ofNat64_finite hn) (ofNat64_finite hD)
  rw [toReal_ofNat64 hn, toReal_ofNat64 hD] at hlt heq hgt
  unfold Model.le
  rcases hc : Model.compare (ofNat64 n) (ofNat64 D) with _ | o
  · -- no comparison: impossible for finite operands
    exfalso
    rcases lt_trichotomy (n : ℝ) D with h | h | h
    · rw [← hlt] at h; rw [hc] at h; simp at h
    · rw [← heq] at h; rw [hc] at h; simp at h
    · have := hgt.mpr h; rw [hc] at this; simp at this
  · cases o
    · have := hlt.mp hc
      exact ⟨fun _ => by exact_mod_cast this.le, fun _ => trivial⟩
    · have := heq.mp hc
      exact ⟨fun _ => by exact_mod_cast this.le, fun _ => trivial⟩
    · simp only [false_iff, not_le]
      have := hgt.mp hc
      exact_mod_cast this

/-! ### The sample parameters of `limit_boxes`, operation by operation -/

/-- `delta` after `j` doublings: `let mut delta = INIT_DELTA` (`10.0`), then `delta *= 2.0`. -/
noncomputable def deltaSeq : ℕ → D64
  | 0 => ofNat64 10
  | j + 1 => Model.mul (deltaSeq j) (ofNat64 2)

theorem deltaSeq_eq (j : ℕ) (h : 10 * 2 ^ j < 2 ^ 53) : deltaSeq j = ofNat64 (10 * 2 ^ j) := by
  induction j with
  | zero => rfl
  | succ j ih =>
    have h' : 10 * 2 ^ j < 2 ^ 53 := by
      have : 2 ^ j ≤ 2 ^ (j + 1) := Nat.pow_le_pow_right (by norm_num) (by omega)
      omega
    have h2 : 10 * 2 ^ j * 2 < 2 ^ 53 := by rw [pow_succ] at h; omega
    simp only [deltaSeq, ih h']
    rw [mul_ofNat64 h2 (by positivity)]
    congr 1
    ring

/-- `let num_div = delta * boxn as f64`, `boxn` boxes, after `j` doublings. -/
noncomputable def numDiv (boxn j : ℕ) : D64 := Model.mul (deltaSeq j) (ofNat64 boxn)

theorem numDiv_eq {boxn : ℕ} (j : ℕ) (hb : 0 < boxn) (h : 10 * 2 ^ j * boxn < 2 ^ 53) :
    numDiv boxn j = ofNat64 (10 * 2 ^ j * boxn) := by
  have h1 : 10 * 2 ^ j < 2 ^ 53 := lt_of_le_of_lt (Nat.le_mul_of_pos_right _ hb) h
  rw [numDiv, deltaSeq_eq j h1, mul_ofNat64 h (by positivity)]

/-- `si` after `k` steps of `si += 1.0` from `0.0` (C: `for (double si = 0; ...; si++)`). -/
noncomputable def siSeq : ℕ → D64
  | 0 => ofNat64 0
  | k + 1 => Model.add (siSeq k) (ofNat64 1)

theorem siSeq_eq (k : ℕ) (h : k < 2 ^ 53) : siSeq k = ofNat64 k := by
  induction k with
  | zero => rfl
  | succ k ih =>
    simp only [siSeq, ih (by omega)]
    rw [add_ofNat64 (by omega) (by omega)]

/-- `si` after `i` steps of `si += 2.0` from `1.0`: the odd-samples-only retry. -/
noncomputable def oddSeq : ℕ → D64
  | 0 => ofNat64 1
  | i + 1 => Model.add (oddSeq i) (ofNat64 2)

theorem oddSeq_eq (i : ℕ) (h : 2 * i + 1 < 2 ^ 53) : oddSeq i = ofNat64 (2 * i + 1) := by
  induction i with
  | zero => rfl
  | succ i ih =>
    simp only [oddSeq, ih (by omega)]
    rw [add_ofNat64 (by omega) (by omega)]
    congr 1

/-- The sample parameter, `t = si / num_div`. -/
noncomputable def tParam (si nd : D64) : D64 := Model.div si nd

/-- **Even samples repeat.** After `delta` doubles, the sample `si = 2k` has the parameter that
the sample `si = k` had before, as the same binary64 word. -/
theorem t_even_eq {boxn : ℕ} (j k : ℕ) (hb : 0 < boxn) (hbound : 10 * 2 ^ (j + 1) * boxn < 2 ^ 53)
    (hk : k ≤ 10 * 2 ^ j * boxn) :
    tParam (siSeq (2 * k)) (numDiv boxn (j + 1)) = tParam (siSeq k) (numDiv boxn j) := by
  have h2 : 10 * 2 ^ (j + 1) * boxn = 2 * (10 * 2 ^ j * boxn) := by ring
  have hj : 10 * 2 ^ j * boxn < 2 ^ 53 := by omega
  have hpos : 0 < 10 * 2 ^ j * boxn := by positivity
  rw [tParam, tParam, numDiv_eq j hb hj, numDiv_eq (j + 1) hb hbound,
    siSeq_eq _ (by omega), siSeq_eq _ (by omega), h2]
  exact div_ofNat64_double hpos (by omega) (by omega)

/-- The odd retry's parameters are those of the odd samples of the full pass. -/
theorem t_odd_eq {boxn : ℕ} (j i : ℕ) (hbound : 10 * 2 ^ (j + 1) * boxn < 2 ^ 53)
    (hi : 2 * i + 1 ≤ 10 * 2 ^ (j + 1) * boxn) :
    tParam (oddSeq i) (numDiv boxn (j + 1)) = tParam (siSeq (2 * i + 1)) (numDiv boxn (j + 1)) := by
  rw [oddSeq_eq i (by omega), siSeq_eq _ (by omega)]


section Fold
variable {σ β : Type*}

structure Absorbing (step : σ → β → σ) (cover : σ → β → Prop) : Prop where
  keep : ∀ c b, cover c b → step c b = c
  stable : ∀ c b b', cover c b' → cover (step c b) b'
  covers : ∀ c b, cover (step c b) b

variable {step : σ → β → σ} {cover : σ → β → Prop}

theorem Absorbing.foldl_stable (h : Absorbing step cover) (L : List β) :
    ∀ (c : σ) (b : β), cover c b → cover (L.foldl step c) b := by
  induction L with
  | nil => intro c b hc; exact hc
  | cons a L ih => intro c b hc; exact ih _ _ (h.stable _ _ _ hc)

theorem Absorbing.foldl_covers (h : Absorbing step cover) (L : List β) :
    ∀ (c : σ), ∀ b ∈ L, cover (L.foldl step c) b := by
  induction L with
  | nil => intro c b hb; simp at hb
  | cons a L ih =>
    intro c b hb
    rcases List.mem_cons.mp hb with rfl | hb
    · exact h.foldl_stable L _ _ (h.covers _ _)
    · exact ih _ _ hb

theorem Absorbing.foldl_skip (h : Absorbing step cover) {ι : Type*} (f : ι → β) (p : ι → Prop)
    [DecidablePred p] (L : List ι) :
    ∀ (c : σ), (∀ i ∈ L, p i → cover c (f i)) →
      (L.map f).foldl step c = ((L.filter (fun i => ¬ p i)).map f).foldl step c := by
  induction L with
  | nil => intro c _; rfl
  | cons a L ih =>
    intro c hc
    by_cases hpa : p a
    · have hk := h.keep c (f a) (hc a (by simp) hpa)
      simp only [List.map_cons, List.foldl_cons, hk, List.filter_cons, hpa, not_true_eq_false,
        decide_false, Bool.false_eq_true, ↓reduceIte]
      exact ih c (fun i hi => hc i (List.mem_cons_of_mem _ hi))
    · simp only [List.map_cons, List.foldl_cons, List.filter_cons, hpa, not_false_eq_true,
        decide_true, ↓reduceIte]
      exact ih _ (fun i hi hpi => h.stable _ _ _ (hc i (List.mem_cons_of_mem _ hi) hpi))

end Fold

/-! ### `fmin` / `fmax` and the box limits -/

section Order
variable {α : Type*} [LT α] [DecidableRel (α := α) (· < ·)]

/-- `ll.x = fmin(ll.x, x)`: a ties keeps the limit. -/
def stepMin (c x : α) : α := if x < c then x else c

/-- `ur.x = fmax(ur.x, x)`: a tie keeps the limit. -/
def stepMax (c x : α) : α := if c < x then x else c

/-- `x` does not lower the limit `c` -/
def CoverMin (c x : α) : Prop := ¬ x < c

/-- `x` does not raise the limit `c` -/
def CoverMax (c x : α) : Prop := ¬ c < x

variable (htrans : ∀ a b c : α, a < b → b < c → a < c) (hirr : ∀ a : α, ¬ a < a)
include htrans hirr

theorem absorbing_min : Absorbing (stepMin (α := α)) CoverMin where
  keep c x h := by simp only [stepMin, CoverMin] at *; simp [h]
  stable c x y h := by
    unfold stepMin CoverMin at *
    by_cases hx : x < c
    · simp only [hx, ↓reduceIte]
      intro hy; exact h (htrans _ _ _ hy hx)
    · simpa [hx] using h
  covers c x := by
    unfold stepMin CoverMin
    by_cases hx : x < c
    · simp [hx, hirr]
    · simp [hx]

theorem absorbing_max : Absorbing (stepMax (α := α)) CoverMax where
  keep c x h := by simp only [stepMax, CoverMax] at *; simp [h]
  stable c x y h := by
    unfold stepMax CoverMax at *
    by_cases hx : c < x
    · simp only [hx, ↓reduceIte]
      intro hy; exact h (htrans _ _ _ hx hy)
    · simpa [hx] using h
  covers c x := by
    unfold stepMax CoverMax
    by_cases hx : c < x
    · simp [hx, hirr]
    · simp [hx]

end Order


/-! ### One box over a pass -/

section Box
variable {α β : Type*} [LT α] [DecidableRel (α := α) (· < ·)]

/-- What `limit_boxes` does to one box for one sample: the `y` test, then `fmin`/`fmax` on `x`.
The box's `y` edges never change, so `inBox` depends on the sample alone. -/
def boxStep (inBox : β → Prop) [DecidablePred inBox] (xOf : β → α) (st : α × α) (s : β) :
    α × α :=
  if inBox s then (stepMin st.1 (xOf s), stepMax st.2 (xOf s)) else st

def boxCover (inBox : β → Prop) (xOf : β → α) (st : α × α) (s : β) : Prop :=
  inBox s → CoverMin st.1 (xOf s) ∧ CoverMax st.2 (xOf s)

theorem absorbing_box (htrans : ∀ a b c : α, a < b → b < c → a < c) (hirr : ∀ a : α, ¬ a < a)
    (inBox : β → Prop) [DecidablePred inBox] (xOf : β → α) :
    Absorbing (boxStep inBox xOf) (boxCover inBox xOf) := by
  have hmin := absorbing_min htrans hirr
  have hmax := absorbing_max htrans hirr
  refine ⟨?_, ?_, ?_⟩
  · intro st s h
    by_cases hs : inBox s
    · obtain ⟨h1, h2⟩ := h hs
      simp [boxStep, hs, hmin.keep _ _ h1, hmax.keep _ _ h2]
    · simp [boxStep, hs]
  · intro st s s' h
    by_cases hs : inBox s
    · intro hs'
      simp only [boxStep, hs, ↓reduceIte]
      exact ⟨hmin.stable _ _ _ (h hs').1, hmax.stable _ _ _ (h hs').2⟩
    · simpa [boxStep, hs] using h
  · intro st s hs
    simp only [boxStep, hs, ↓reduceIte]
    exact ⟨hmin.covers _ _, hmax.covers _ _⟩

end Box

/-! ### The passes -/

/-- The samples of a pass with `D` as `num_div`, in C's order: each spline segment `s` in turn,
`si = 0, 1, .., D` within it. -/
def fullItems (S D : ℕ) : List (ℕ × ℕ) :=
  (List.range S).flatMap fun s => (List.range (D + 1)).map fun n => (s, n)

/-- The retry's samples when `num_div` is `2D`: `si = 1, 3, .., 2D - 1` in each segment. -/
def oddItems (S D : ℕ) : List (ℕ × ℕ) :=
  (List.range S).flatMap fun s => (List.range D).map fun i => (s, 2 * i + 1)

/-- The samples of the pass before. -/
def prevItems (S D : ℕ) : List (ℕ × ℕ) := fullItems S D

theorem range_succ_two (m : ℕ) : List.range (m + 2) = List.range m ++ [m, m + 1] := by
  simp [List.range_succ]

theorem range_double_filter_odd (D : ℕ) :
    (List.range (2 * D + 1)).filter (fun n => ¬ n % 2 = 0) = (List.range D).map (fun i => 2 * i + 1) := by
  induction D with
  | zero => simp
  | succ D ih =>
    have e : 2 * (D + 1) + 1 = (2 * D + 1) + 2 := by omega
    rw [e, range_succ_two, List.filter_append, ih]
    simp [List.range_succ]
    omega

theorem range_double_filter_even (D : ℕ) :
    (List.range (2 * D + 1)).filter (fun n => n % 2 = 0) = (List.range (D + 1)).map (fun i => 2 * i) := by
  induction D with
  | zero => simp
  | succ D ih =>
    have e : 2 * (D + 1) + 1 = (2 * D + 1) + 2 := by omega
    rw [e, range_succ_two, List.filter_append, ih]
    have h2 : (2 * D + 1 + 1) % 2 = 0 := by omega
    simp [List.range_succ, h2]
    omega


theorem fullItems_filter_odd (S D : ℕ) :
    (fullItems S (2 * D)).filter (fun x => ¬ x.2 % 2 = 0) = oddItems S D := by
  unfold fullItems oddItems
  rw [List.filter_flatMap]
  congr 1
  funext s
  rw [List.filter_map]
  have h := range_double_filter_odd D
  have : (List.filter ((fun x : ℕ × ℕ => decide ¬x.2 % 2 = 0) ∘ fun n => (s, n))
      (List.range (2 * D + 1))) = List.filter (fun n => decide ¬n % 2 = 0) (List.range (2 * D + 1)) := rfl
  rw [this, h, List.map_map]
  rfl


/-! ### The retry theorem -/

section Retry
variable {σ β : Type*} {step : σ → β → σ} {cover : σ → β → Prop}

/-- The samples a pass takes, as values: `sample s t` is the sample on spline segment `s` at
parameter `t`. A function of the segment's control points and `t` only, which is what
`limit_boxes`'s body is: `sp` is copied from `pps` afresh for every `si`. -/
noncomputable def passVals (sample : ℕ → D64 → β) (boxn S j : ℕ) : List β :=
  (fullItems S (10 * 2 ^ j * boxn)).map fun x => sample x.1 (tParam (siSeq x.2) (numDiv boxn j))

/-- The retry's samples at `j + 1`: `si = 1.0, 3.0, ..` by `si += 2.0`, `D` of them per segment. -/
noncomputable def oddRetryVals (sample : ℕ → D64 → β) (boxn S j : ℕ) : List β :=
  (List.range S).flatMap fun s => (List.range (10 * 2 ^ j * boxn)).map fun i =>
    sample s (tParam (oddSeq i) (numDiv boxn (j + 1)))

theorem oddRetryVals_eq (sample : ℕ → D64 → β) {boxn : ℕ} (S j : ℕ)
    (hbound : 10 * 2 ^ (j + 1) * boxn < 2 ^ 53) :
    oddRetryVals sample boxn S j =
      ((oddItems S (10 * 2 ^ j * boxn)).map fun x =>
        sample x.1 (tParam (siSeq x.2) (numDiv boxn (j + 1)))) := by
  have h2 : 10 * 2 ^ (j + 1) * boxn = 2 * (10 * 2 ^ j * boxn) := by ring
  unfold oddRetryVals oddItems
  rw [List.map_flatMap]
  congr 1
  funext s
  rw [List.map_map]
  apply List.map_congr_left
  intro i hi
  have hi' := List.mem_range.mp hi
  simp only [Function.comp]
  rw [t_odd_eq j i hbound (by omega)]

/-- **The retry is the full pass minus the repeated samples.** Let `c0` be the box limits before
the pass at `delta`, `c1` the limits after it (the boxes persist from one pass to the next). The
pass at `2 * delta`, which takes every sample, and the retry, which takes only the odd ones, leave
the same limits. -/
theorem retry_odd_eq_full (h : Absorbing step cover) (sample : ℕ → D64 → β) {boxn : ℕ}
    (S j : ℕ) (hb : 0 < boxn) (hbound : 10 * 2 ^ (j + 1) * boxn < 2 ^ 53) (c0 : σ) :
    (passVals sample boxn S (j + 1)).foldl step ((passVals sample boxn S j).foldl step c0) =
      (oddRetryVals sample boxn S j).foldl step ((passVals sample boxn S j).foldl step c0) := by
  have h2 : 10 * 2 ^ (j + 1) * boxn = 2 * (10 * 2 ^ j * boxn) := by ring
  set D := 10 * 2 ^ j * boxn with hD
  set c1 := (passVals sample boxn S j).foldl step c0 with hc1
  rw [oddRetryVals_eq sample S j hbound, ← fullItems_filter_odd S D]
  have key := h.foldl_skip (fun x : ℕ × ℕ => sample x.1 (tParam (siSeq x.2) (numDiv boxn (j + 1))))
    (fun x => x.2 % 2 = 0) (fullItems S (2 * D)) c1 ?_
  · have hp : passVals sample boxn S (j + 1) =
        (fullItems S (2 * D)).map fun x => sample x.1 (tParam (siSeq x.2) (numDiv boxn (j + 1))) := by
      unfold passVals
      rw [h2]
    rw [hp, key]
  · intro x hx hx2
    obtain ⟨s, n⟩ := x
    simp only [fullItems, List.mem_flatMap, List.mem_range, List.mem_map, Prod.mk.injEq] at hx
    obtain ⟨s', hs', n', hn', rfl, rfl⟩ := hx
    obtain ⟨k, hk⟩ : ∃ k, n' = 2 * k := ⟨n' / 2, by simp at hx2; omega⟩
    have hkD : k ≤ D := by omega
    have hmem : sample s' (tParam (siSeq k) (numDiv boxn j)) ∈ passVals sample boxn S j := by
      unfold passVals fullItems
      simp only [List.mem_map, List.mem_flatMap, List.mem_range, Prod.exists]
      exact ⟨s', k, ⟨s', hs', k, by omega, rfl⟩, rfl⟩
    have := h.foldl_covers _ c0 _ hmem
    simp only [hk]
    rw [t_even_eq j k hb hbound hkD]
    exact this

end Retry

/-- **`limit_boxes`'s retry, for one box.** Whatever the box's state before the pass at `delta`
(`st0`), the pass at `2 * delta` that takes every sample and the retry that takes only the odd
ones end in the same `(ll.x, ur.x)`. -/
theorem limit_boxes_retry_exact {α Sample : Type*} [LT α] [DecidableRel (α := α) (· < ·)]
    (htrans : ∀ a b c : α, a < b → b < c → a < c) (hirr : ∀ a : α, ¬ a < a)
    (sample : ℕ → D64 → Sample) (inBox : Sample → Prop) [DecidablePred inBox] (xOf : Sample → α)
    {boxn : ℕ} (S j : ℕ) (hb : 0 < boxn) (hbound : 10 * 2 ^ (j + 1) * boxn < 2 ^ 53)
    (st0 : α × α) :
    (passVals sample boxn S (j + 1)).foldl (boxStep inBox xOf)
        ((passVals sample boxn S j).foldl (boxStep inBox xOf) st0) =
      (oddRetryVals sample boxn S j).foldl (boxStep inBox xOf)
        ((passVals sample boxn S j).foldl (boxStep inBox xOf) st0) :=
  retry_odd_eq_full (absorbing_box htrans hirr inBox xOf) sample S j hb hbound st0

end GvpProofs.LimitBoxes
