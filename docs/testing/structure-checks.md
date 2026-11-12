# Structural checks (`gv-diff structure`)

`gv-diff e2e` requires the port's `-Tsvg` to equal C's byte for byte, with C computing with the
port's math functions. Against stock C on macOS, which calls Apple's libm, 120 corpus inputs come
out as a different, equally valid layout of the same graph (mostly sfdp; see
`docs/porting/libm.md`). Byte identity cannot judge those against stock C. The structural checks
can: they test what must hold whatever the layout, on every corpus input, with stock C's output
as the reference.

## What is checked

For each graph in each corpus input, the port's drawing is compared with a summary of C's:

| check | fails when | geometric |
| :--- | :--- | :---: |
| `nodes` | the multiset of node titles differs from C's | |
| `edges` | the multiset of edge titles (`a->b`) differs | |
| `clusters` | the multiset of cluster titles differs | |
| `labels` | the multiset of `<text>` contents differs (a missing, extra or changed label) | |
| `finite` | the port has more non-finite coordinates than C (C itself writes `inf` for two fuzz reproducers) | |
| `on-canvas` | more drawn points lie over 1pt outside the `viewBox` than in C's drawing | yes |
| `canvas-size` | the canvas area is outside 2/3 to 3/2 of C's (`AREA_RANGE`) | yes |
| `overlaps` | more node pairs overlap than C's count plus 25% plus 3 (`OVERLAP_SLACK`) | yes |

Geometric checks are skipped for the inputs that `tests/golden/e2e.json` records as unstable,
because C's own geometry changes from run to run there.
Inputs listed in `tests/golden/deviations.tsv`, where the port deliberately differs from C
(#846), are skipped; `gv-diff e2e` checks them against the port's pinned output instead.

Where the tolerances come from: in a review of the 119 inputs that differed from stock C at the time
(the automated measures below, plus looking at every pair side by side), the
canvas area ratio ranges from 0.79 to 1.19, and the port has more overlapping node pairs than C
on 12 inputs, fewer on 9 and the same on 98 (16,694 in total against C's 16,740). The worst
case, `sfdp/fx_abstract`, has 14 against C's 10.

## Running it

```sh
cargo run --release -p graphviz-pure-diff -- structure                  # every corpus input
cargo run --release -p graphviz-pure-diff -- structure --filter sfdp/   # a subset
```

`cargo test --workspace` runs it (`crates/graphviz-pure-diff/tests/structure_fidelity.rs`), so CI checks
it on Linux, and the cross-platform workflow on Windows, arm64 Linux and macOS (arm64 and x86-64).

The reference is `tests/golden/structure.json`, a summary per input: the counts and hashes of
the four multisets, the canvas size, and C's own non-finite, off-canvas and overlap counts.
Regenerate it after adding corpus inputs or changing what a summary records:

```sh
GVBINDIR=~/src/gv-ref/lutplugins cargo run --release -p graphviz-pure-diff -- structure --generate
```

This needs the C reference build (see `scripts/gen-golden.sh`). `--c-dot PROG` runs another C
command, such as `scripts/dot-portable-libm.sh`. One input, `main/charset_json_escapes`, is
skipped: C writes a control character into its SVG, which is not valid XML.

## Adding a check

Everything lives in `crates/graphviz-pure-diff/src/structure.rs`.

1. **If the check needs something the summary does not record**, add a field to `Doc` and fill
   it in `summarize_one`, which walks one SVG document. Keep it small and deterministic: the
   reference stores it for every input. Counts and hashes of sorted multisets (`Set::of`) work
   well; raw coordinates do not, because a different layout moves them all.
2. **Add an entry to `CHECKS`**: a `name`, whether it is `geometric` (skipped where C is
   unstable), and a function `fn(port: &Doc, c: &Doc) -> Option<String>` that returns a
   message when the port's drawing fails. Compare against C's value rather than an absolute
   threshold where C can itself break the rule (as `finite` and `on-canvas` do).
3. **Add a unit test** in the `tests` module of `structure.rs` that breaks a real drawing in
   the way the check is meant to catch and asserts that the check fires. The existing tests
   remove a node, change a label, move the drawing off its canvas, insert `nan`, shrink the
   `viewBox` and stack nodes on top of each other.
4. **Regenerate the reference** (`--generate`, above) if you added a field, and run the check
   (`cargo test --workspace`). If it reports problems, look at the drawings before loosening anything:
   `gv-diff structure --filter NAME` lists the failing inputs, and the C and port SVGs can be
   compared in a browser or with the site's diff viewer.
5. **Add the check to the table above** with its rule and tolerance.

## Adding inputs

The checks cover every input of the corpora `gv-diff e2e` uses (`CORPORA` in
`crates/graphviz-pure-diff/src/e2e.rs`): put new `.gv` files in one of those directories, regenerate
`tests/golden/structure.json` and `tests/golden/e2e.json`, and refresh the baselines
(`scripts/update-baselines.sh`).
