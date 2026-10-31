# Pathplan without the repeated work

Three loops in dot's and the other engines' routing test things they already know, or test
things that cannot matter. This page shows that each shortcut gives **the same answer as the C-mirror
loop, bit for bit**, including for inputs that are not simple polygons and for coordinates where
rounding is at work. The answers go on to decide routes, so an answer that was merely geometrically
right would not be enough.

Issue #704; measurements are in its pull request.

## 1. Ear clipping

`Pshortestpath` triangulates its polygon by ear clipping (`shortest.c:triangulate`). It takes the
first vertex `i` whose diagonal to `i + 2` passes `isdiagonal`, clips `i + 1`, and starts again
from the first vertex. So a polygon of n vertices re-tests the same vertices about n times, and each
`isdiagonal` can read every side. `clip::triangulate` clips the same vertices in the same order.

`isdiagonal(i, ip2)` reads nothing but coordinates and has no side effects. It is

```text
corner(pts[i-1], pts[i], pts[i+1], pts[ip2])  and  no side (j, j+1), not touching i or ip2, intersects (i, ip2)
```

so what it answers depends only on the points it reads. Hence:

> **Lemma 1.** A refusal stands for as long as what caused it does.
>
> - If `corner` failed, `isdiagonal` says no whenever the vertex has the same `i-1`, `i+1` and
>   `ip2`. The same call on the same four points gives the same answer.
> - If side (a, b) intersected the diagonal to `far`, it says no whenever the vertex's `ip2` is still
>   `far` and (a, b) is still a side. a and b touch neither `i` nor `far`, so the side is still among
>   those tested, the same call gives the same answer, and the answer to "does any side intersect"
>   is yes in whatever order the sides are tried.

Each vertex keeps its last refusal, by vertex rather than by position, since positions shift when
a vertex is clipped. The loop still goes through the vertices in C's array order, and it skips only
those whose refusal stands. So:

> **Theorem 1.** At every step it clips the first vertex for which `isdiagonal` is true: a skipped
> vertex is false by Lemma 1, and any other is computed. If none is, it fails where C fails.

It therefore makes the same triangles, in the same order, from the same vertices, and C's failure
message comes at the same point. This holds for any input, simple polygon or not: the argument uses
only that `isdiagonal` is a pure function.

The cost that remains is the vertices whose refusal no longer stands, plus the diagonal that is
accepted, which must read every side.

## 2. Which triangles share a side

C's `connecttris` compares every pair of triangles on all nine pairs of sides, and records the two as
neighbours where the two ends match, in either direction. Vertices here are distinct by index, so
"the ends match" means the same two vertices.

> **Lemma 2.** Two triangles share a pair of vertices exactly when a clip's diagonal is used by a
> later triangle, and no pair is in three.
>
> *Proof.* A triangle's sides are two sides of the polygon left to clip, and the diagonal, which
> becomes a side of what is left. A pair of vertices can only be in a triangle while it is a side of
> what is left. It becomes one once, either from the start or as a diagonal, and when a triangle uses
> it as a side other than its diagonal, one of the two vertices is clipped, unless that triangle
> is the last. So an original side is in one triangle and a diagonal in two. ∎

So `clip::triangulate` records, per vertex, which triangle's third edge made the side leaving it,
and links the two when a later triangle uses that side. `right_index` ends with the same value on
every edge as C's, whichever way it is reached, because no edge can be matched twice. If the
triangulation fails part way, the sides not yet used have no second triangle, as in C.

## 3. `limit_boxes`

`limit_boxes` samples each Bézier segment and, for each sample, widens every box whose y range holds
it, so `delta × boxes` samples test every box. The test, with its fudge, is:

```text
y <= b.ur.y + 0.0001  and  y >= b.ll.y - 0.0001
```

The loop writes only x, so y never changes, and the order in which boxes are widened does not matter,
since each box is widened from its own values and the sample.

If `ll.y` and `ur.y` both never rise along the list, then `b.ll.y - 0.0001` and `b.ur.y + 0.0001` do
not rise either: rounding to nearest is monotonic. So `y < b.ll.y - 0.0001` holds on a prefix, and
`y <= b.ur.y + 0.0001` holds on a prefix. What passes the test is the second prefix minus the first,
which is an interval that two `partition_point`s find. The case of both never falling is the mirror
image. With NaN in `y` both predicates are false everywhere and the interval is empty, as the test
is false on every box. With NaN in a box's y, `y_order` says `Neither` unless there is only one box,
for which the search is the test itself. `Neither` loops over the whole list. The order is read once per
call, in O(boxes), against O(samples × boxes).

## 4. `visibility`

`Pobsopen`'s `visibility` and `ptVis` test about V² pairs of points against up to V sides of
obstacles, so it is cubic, and most of the sides are nowhere near the segment. `clear` asks whether
any side intersects the segment. The order of the tests cannot matter, and a side can be skipped if
`intersect` is certainly false for it.

`intersect(a, b, c, d)` is true in three ways:

1. `wind(a, b, c) == 0 && in_between(a, b, c)`;
2. `wind(a, b, d) == 0 && in_between(a, b, d)`;
3. `wind(a, b, c) * wind(a, b, d) < 0 && wind(c, d, a) * wind(c, d, b) < 0`.

`wind` is the sign of `area2`, except that values within ±0.0001 count as 0.

> **Lemma 3.** If all four points have coordinates of magnitude at most M = 10⁵, the computed `area2`
> is within 3.2 × 10⁻⁵ of the true value.
>
> *Proof.* `area2(a, b, c)` is `fma(x, y, z)`, with x = a.y − b.y, y = c.x − b.x, and
> z = −((c.y − b.y) · (a.x − b.x)). Each difference has true magnitude at most 2M and relative error
> at most u = 2⁻⁵³. So xy has relative error at most 2u + u² and z at most 3u + 3u² + u³, and `fma`
> rounds once more. The total is at most 4M²(2u) + 4M²(3u) + 8M²u + O(M²u²) = 28M²u(1 + O(u)),
> which is 3.11 × 10⁻⁵ for M = 10⁵. ∎

`apart` first compares `gap_x` and `gap_y`, the distances between the two segments' x and y ranges.
A float subtraction has the sign of the true difference, so `gap > 0` is exact. With `dx` the
x-extent of `ab` and `dy` its y-extent, `apart` is true when

- `a.x != b.x`, and `gap_x > 0` or (`gap_y > 0` and `gap_y · dx > 0.0002`); or
- `a.x == b.x`, and `gap_y > 0` or (`gap_x > 0` and `gap_x · dy > 0.0002`).

> **Theorem 2.** For points within M, `apart` implies that `intersect` is false.
>
> *Proof.* `apart` implies that the two segments' bounding boxes are disjoint.
>
> *Way 3.* The computed values have magnitude over 10⁻⁴, so by Lemma 3 the true values have the same
> signs. Then c and d are strictly on opposite sides of line ab, and a and b strictly on opposite
> sides of line cd, so the segments cross at a point in both boxes. They are disjoint, so this cannot
> happen.
>
> *Ways 1 and 2*, with `a.x != b.x`. `in_between` then requires the end of `cd` to have an x strictly
> within ab's x range. If `gap_x > 0`, none does. Otherwise `gap_y · dx > 0.0002`. Say such an end
> e exists. The true `area2` has magnitude dx · |e.y − L|, where L is the height of line ab at e.x,
> which lies in ab's y range. So |e.y − L| ≥ gap_y, and the true magnitude is at least
> 0.0002(1 − 4u), allowing for the rounding of the product. By Lemma 3 the computed value exceeds
> 10⁻⁴, so `wind` is not 0.
>
> *Ways 1 and 2*, with `a.x == b.x`, are the same with x and y exchanged, since `in_between` then
> reads y. If a and b are the same point, `in_between` is false. ∎

The limit is checked: `clear` skips only if every point is within M, which NaN is not. A graph whose
coordinates pass 10⁵ points (about 1,400 inches) runs C's loop.

The two conditions on the second axis are needed. A segment starting a hair above `ab`, between its
ends, has disjoint boxes yet `wind` calls it collinear, and `intersect` says yes. A test holds that
case.

## 5. What stays

- `isdiagonal`, `intersects`, `ccw`, `intersect` and `wind` are unchanged, and everything still decides
  through them. `isdiagonal` now reports through `refusal`, the same code saying why.
- C's triangulation and connection stay in `shortest::triangulate_c` as the reference, used by check
  mode and tests.
- `Proutespline` and `solve3` are untouched. Skipping their tests would need a bound on how far
  `solve3`'s rounding can move a root, and there is no such simple bound.

## 6. How it is checked

**Check mode** (`--features graphviz-pure-layout/routing-check`, on in CI over the test suite, and
`cfg(test)`):

- every `Pshortestpath` also runs `triangulate_c` and asserts the same triangles, neighbours and
  outcome;
- every side `clear` skips also runs `intersect` and asserts false;
- every sample in `limit_boxes` asserts that every box passing the test is in the range searched.

**Tests:**

- `clips_the_same_triangles_as_c`: 12,000 polygons, corridors as dot makes them, star shapes with many
  long-standing refusals, and closed walks that are not simple polygons, including ones where
  triangulation fails;
- `apart_segments_never_intersect`: 2 million quadruples, at scales up to the limit, a hair off a
  grid, with offsets around the tolerance;
- `a_small_gap_along_the_other_axis_is_not_apart`, `the_limit_keeps_large_and_nan_coordinates_out`
  and `clear_answers_as_it_does_without_the_skip`;
- `limit_boxes` against the plain loop on random boxes and splines, falling, rising and neither,
  with NaN;
- `standing_refusals_save_the_tests`, which counts `refusal` calls, because without the shortcut
  the output is C's too and no test of output could tell.

Mutants of `clip.rs`, `refusal`, `apart` and the box ranges (reported in the pull request) are
killed by these tests, but for one that cannot change anything: a corner refusal needs no check of
the vertex after the corner, since if that vertex goes, `far` changes as well.
