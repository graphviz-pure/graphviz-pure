# Musl allocation churn: wall gains rejected on RSS (#610)

No production change qualified. Removing temporary parser attribute maps reduces wall time and
system calls, but raises patchwork RSS. Keeping a copy of the final value, borrowing token text, or
allocating exactly one temporary entry does not fix that tradeoff. Static musl, its malloc, safe
Rust, dependencies and the ratchet baseline remain unchanged.

## Where the syscalls come from

The baseline osage 20k SVG trace has 2,650 `munmap` calls: 2,626 release regions below 128 KiB;
1,981 are 8 KiB, 282 are 16 KiB and 193 are 4 KiB. Patchwork has 3,589 calls, 3,564 below 128 KiB.
These are predominantly releases of allocator groups holding small allocations, rather than
thousands of independently mapped large vectors. Requests immediately preceding mappings include
192-byte attribute vectors and 240-byte packing arrays.

A separate unstripped v3 static-musl diagnostic build, without PGO, supplies symbols and frame
pointers. Entry probes capture `free` return addresses and malloc/realloc sizes; the next unmap on
that thread is correlated with the last free, and callers are resolved against that binary.
Unwinding through the prebuilt musl allocator is incomplete, so attribution uses the recorded caller
rather than pretending every allocator stack unwinds. No probe or allocator override ships.

| Last-free caller before group release | osage 20k SVG | patchwork 20k SVG |
| :-- | --: | --: |
| Temporary statement attributes in `Parser::rcompound` | 632 | 632 |
| `SubgraphData` destruction | 868 | 868 |
| Packing cleanup in `put_rects` | 438 | — |
| Layout-node destruction | 27 | 715 |
| Render-node destruction | 60 | 750 |
| Node-name lookup B-tree destruction | 134 | 134 |

Counts identify the free that triggers group release, not exclusive ownership of everything in that
mapping. A group can contain objects allocated by several callers. The large model-drop counts do
not demonstrate repeated per-stage realloc growth. The parser intervention below removes 621
mapping/unmapping pairs, independently confirming that its temporary allocations cause a removable
part of the churn.

## Paired experiment

The primary candidate is
[musl-churn-direct.patch](../../scripts/bench/experiments/musl-churn-direct.patch). It keeps the
first statement attribute inline, moves it into a single-node statement, and falls back to the
normal map for wider lists or edge/node lists. Concatenated strings are trimmed before retention. C
counterparts are `lib/cgraph/grammar.y:optattr, appendattr, endnode` and `lib/cgraph/attr.c:agxset`.
There are no floating-point changes. This patch is an experiment, not an applied product change.

Baseline source is `78a0cece`; the subsequent #780 merge changes documentation and diagnostic files
only. Both baseline and candidate are separately trained fresh x86-64-v3 `x86_64-unknown-linux-musl`
PGO builds made by `scripts/build-pgo.py`, using Rust 1.99.1, LLVM 23.1.1, the same 276 training
commands and cargo-slot. Full build metadata and patch/binary hashes are in the [raw
data](data/musl-churn-610.json). Control is a byte-for-byte copy of baseline. The 1.0 reference is
the shipped-shape v1 musl release-candidate artifact `f89e21d9`.

Three warmups per binary/case, then 40 rounds with the four binaries shuffled independently in each
round, seed 6101134. Measurements are serial, nice 15, with an 8 GB virtual-memory limit and output
discarded. A fresh Python helper performs `wait4` around each executable; wall time includes process
startup, and user/kernel CPU time, faults and peak RSS come from that child’s resource usage. The
helper records its current RSS and requires the child’s high-water mark to exceed it: Linux inherits
the parent RSS floor on fork, which otherwise makes small cases look artificially identical. Earlier
samples affected by that floor were discarded. Bootstrapped intervals resample paired log wall
ratios, 5,000 times.

Baseline/control SHA-256: `16478d5f1d96134b6c5ab9f8cd7ab8631a35db3ccfb36456c253312900d20a18`.
Candidate: `45679d07e5cc5d3b63ac9deddac772e83266d8909ba352461e8ce490fee4b2a3`.

Start/end load:

```text
20:45:40 up 1 day, 16:46,  0 users,  load average: 2.13, 1.58, 1.23
20:51:35 up 1 day, 16:52,  0 users,  load average: 1.37, 1.40, 1.27
```

Candidate build metadata records the base git commit because each prototype was an uncommitted
patch. The recorded patch SHA-256 identifies its actual source; patches apply independently, not as
a stack.

## Time, faults and RSS

Medians. Every arrow is baseline → candidate; CPU and wall times are milliseconds, RSS is MiB,
faults are minor faults. All measured runs have zero major faults. `Paired ×` is the geometric mean
of within-round baseline/candidate ratios; control uses baseline/identical-copy. An RSS increase is
not accepted just because time improves.

| Case | Wall ms | User ms | Kernel ms | Minor faults | Peak RSS MiB | Paired × [95%] | Control × |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| osage 5k svg | 46.08 → 43.10 | 28.34 → 27.29 | 16.52 → 15.66 | 4824 → 4673 | 18.98 → 18.98 | 1.075 [1.056, 1.095] | 1.001 |
| osage 20k svg | 176.05 → 162.51 | 119.86 → 113.65 | 55.71 → 47.74 | 13192 → 12602 | 59.74 → 59.86 | 1.086 [1.075, 1.096] | 1.000 |
| osage 20k plain | 171.33 → 158.34 | 120.15 → 111.37 | 50.09 → 49.33 | 11962 → 11339 | 49.32 → 49.32 | 1.084 [1.071, 1.098] | 0.997 |
| patchwork 5k svg | 54.93 → 51.53 | 36.04 → 31.01 | 19.47 → 20.07 | 5547 → 5401 | 19.67 → 19.91 | 1.061 [1.048, 1.075] | 0.999 |
| patchwork 20k svg | 200.34 → 187.85 | 134.74 → 130.96 | 64.04 → 56.07 | 14310 → 13673 | 63.20 → 63.41 | 1.066 [1.058, 1.075] | 0.998 |
| patchwork 20k plain | 192.06 → 179.02 | 133.31 → 126.30 | 59.95 → 53.89 | 12950 → 12333 | 50.83 → 51.02 | 1.070 [1.060, 1.081] | 0.996 |
| dot 1k svg | 88.95 → 89.16 | 68.21 → 65.12 | 20.27 → 23.87 | 5828 → 5815 | 16.34 → 16.34 | 1.002 [0.990, 1.015] | 0.990 |
| twopi 20k svg | 272.54 → 272.24 | 205.58 → 205.24 | 66.42 → 63.60 | 18544 → 18608 | 85.62 → 85.31 | 1.003 [0.993, 1.014] | 1.002 |

The patchwork 5k SVG RSS rise is reproducible: mean paired difference +253.5 KiB versus baseline
(95% bootstrap interval +34.6 to +470.9), and +246.0 KiB versus the identical copy (+33.5 to
+451.1). Its median rises about 240 KiB. Patchwork 20k plain also rises versus the identical copy:
mean +506.6 KiB, interval +49.7 to +949.2. Larger cases have considerable RSS noise, exposed by the
identical control; the smaller case independently rejects the candidate.

## Syscall intervention

Separate `strace -f -c` runs, three repeats per binary/case; counts below are medians. The four
target cases each remove exactly 621 `mmap` and 621 `munmap` calls. `mremap` and `brk` counts do not
move.

| Case | mmap | munmap | mremap | brk |
| :-- | --: | --: | --: | --: |
| osage 20k svg | 2655 → 2034 | 2650 → 2029 | 22 → 22 | 107 → 107 |
| osage 20k plain | 2650 → 2029 | 2645 → 2024 | 17 → 17 | 107 → 107 |
| patchwork 20k svg | 3593 → 2972 | 3589 → 2968 | 27 → 27 | 120 → 120 |
| patchwork 20k plain | 3588 → 2967 | 3584 → 2963 | 22 → 22 | 120 → 120 |

## Narrower variants also fail RSS

Each uses a fresh v3 musl PGO build, the same training set, three warmups and 40 shuffled rounds on
patchwork 5k SVG, with an identical-binary control. These checks stop candidates before spending
another full-suite run on a known memory failure. They do not establish their 20k/control
performance. Patches are diagnostics only.

| Variant | Baseline / candidate / identical-control RSS MiB | Mean candidate − control KiB [95%] |
| :-- | --: | --: |
| [Omit map, preserve final string copy](../../scripts/bench/experiments/musl-churn-copy.patch) | 19.715 / 19.949 / 19.688 | +270.4 [+116.3, +423.4] |
| [Borrow lexer text until final insertion](../../scripts/bench/experiments/musl-churn-borrow.patch) | 19.699 / 19.955 / 19.686 | +267.8 [+109.2, +420.6] |
| [Keep map, allocate exactly one entry](../../scripts/bench/experiments/musl-churn-exact.patch) | 19.680 / 19.967 / 19.674 | +334.4 [+264.4, +446.2] |

The exact-entry variant retains the temporary allocation and final-copy behavior, replacing the
four-slot vector’s 192 bytes with one 48-byte entry only when the complete list is known to have one
entry. It still increases RSS. This rules out blaming only the retained capacity or ownership of
moved strings.

Earlier bounded temporary-map reuse and compact inherited graph-attribute copies also reduced
syscalls/time but failed RSS in at least one case. The [reuse/compact
prototype](../../scripts/bench/experiments/musl-churn-reuse-compact.patch) additionally fails the
full allocation ratchet on circo by 16 allocations: inherited maps grow when a child adds `rank`. No
baseline adjustment or acceptance label is used to hide that failure. Its before/after deterministic
counts are retained in the data.

## Progress toward the floor

The primary rejected candidate has a geometric mean paired gain of 1.0817× for osage and 1.0657× for
patchwork over their three headline cases. Applying those factors to the owner’s anchors would give
3.072× and 2.632×, respectively, but **no gain is credited** because of RSS. The unchanged shipped
anchors remain osage 2.84× and patchwork 2.47×.

Fresh 1.0 comparison and C reference numbers are informational: 1.0 ran in the same shuffled rounds;
C 16.1.0 is the stock pango-free reference, without FMA preload, measured separately five times.
Their absolute numbers should not be substituted for the owner’s measurement window.

| Case | 1.0 ms | Candidate ms | ×1.0 | C ms | ×C |
| :-- | --: | --: | --: | --: | --: |
| osage 5k svg | 135.58 | 43.10 | 3.15 | 56.35 | 1.31 |
| osage 20k svg | 542.90 | 162.51 | 3.34 | 219.95 | 1.35 |
| osage 20k plain | 540.17 | 158.34 | 3.41 | 185.47 | 1.17 |
| patchwork 5k svg | 132.31 | 51.53 | 2.57 | 65.43 | 1.27 |
| patchwork 20k svg | 544.21 | 187.85 | 2.90 | 262.31 | 1.40 |
| patchwork 20k plain | 533.50 | 179.02 | 2.98 | 198.89 | 1.11 |
| dot 1k svg | 295.42 | 89.16 | 3.31 | 302.47 | 3.39 |
| twopi 20k svg | 903.08 | 272.24 | 3.32 | 668.25 | 2.45 |

The fresh local engine geometric means against 1.0 are osage 3.058× → 3.297× and patchwork 2.629× →
2.809×. Dot/twopi control paired wall intervals include 1; their median changes do not establish a
gain or slowdown.

## What the result supports

The churn is neither entirely large-vector growth nor entirely outside our code. The parser
allocations are removable and wall time responds; these implementations fail the independent RSS
requirement. Most remaining releases coincide with destruction of small objects in the
graph/layout/render models. Reserving the outer node arrays cannot remove those object frees, and
delaying releases by retaining buffers is not a memory-free optimization. The next representation
experiment must demonstrate a resident-memory reduction as well as fewer group releases. These
measurements do not prove that the entire residual cost is allocator-internal or that a shared model
cannot help.

## Reproduction and verification

Apply only one experimental patch to the baseline product source. Build baseline and candidate with
separate, new profile directories using `scripts/build-pgo.py --target x86_64-unknown-linux-musl
--cargo-wrapper /usr/local/bin/cargo-slot.sh`, `RUSTFLAGS="-C target-cpu=x86-64-v3"`, nice 15, and
an extra target directory on the jobs volume. Copy the baseline for the control and verify binary
hashes before measuring. Use the same synthetic graphs and the explicit `-K<engine> -T<format>`
arguments from the tables. The companion [measurement
script](../../scripts/bench/experiments/musl-churn.py) implements the fresh-helper resource
measurement and paired protocol. To trace counts, run each target command under `strace -qq -f -c -e
trace=mmap,munmap,mremap,madvise,brk` separately from timings.

Primary candidate and baseline stdout SHA-256 and complete stderr match on all eight measured cases.
Parser tests (44 unit tests and 16 syntax-error tests) pass for all four inline variants; they cover
HTML flags, defaults, duplicates, wider lists, multiple nodes, edges and compact retained
concatenations. Rejected variants were not advanced through the full fidelity and large-heap gates
after their RSS failure. The full gates reported for this documentation PR run against the unchanged
production source, not those rejected variants.

Local gates for the unchanged production head pass: formatting, workspace Clippy with all targets
and warnings denied, the panic-site check, full workspace tests (including all existing fidelity
gates), and the full 19-case performance ratchet. Allocations and peak heap exactly match the fresh
baseline in every case; the maximum matched instruction-count movement is 0.7457%. These counts are
also in the data. The published helper is checked on all eight cases separately from the recorded
40-round measurement.

Verification status is recorded in the PR. `ratchet-baseline.json`, all Rust product source,
manifests and lockfiles are unchanged. This is a measured negative result, not an optimization ready
for release.

Refs #610.
