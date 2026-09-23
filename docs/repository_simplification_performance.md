# Repository simplification performance evidence

This comparison measures the original pre-overhaul production code at
`77806da210bc548235a76ba2ea5b6f647d792f30` against the integrated
nonbibliography implementation at `1f32318e0ce5fb57fe8b70b25130b51f235f1dff`.
The [structured receipt](repository_simplification_measurements.json) records
each trial, source and fixture identity, executable hash, output digest,
counter, case status, and reported mismatch. All measurements in this document
belong to those frozen revisions. Later repairs to benchmark source or memory
accounting require their own gate result; they do not change these timings.

The baseline measurement checkout adds only the identical selected-census test
instrumentation and independently corrected `count:0=0` fixture projection.
Its production `crates` tree is the original tree. The fixture tree and census
source hashes match the final checkout, so these test-only changes cannot
explain a production comparison. No expected output was blessed from the new
engine. The receipt pins those trees and the baseline instrumentation patch.

## Protocol and interpretation

The quiet host was an x86-64 Linux 6.8 machine with Rust/Cargo 1.98.1, 12
available CPUs, `SOURCE_DATE_EPOCH=1787080434`, `LC_ALL=C.UTF-8`, and four
Cargo jobs. No other build or benchmark process ran during timing. Each side
used its own checkout-local targets. Cold `tex-exec` lib-test compilation used
three fresh target directories per side in baseline/final, final/baseline,
baseline/final order. The GNU `/usr/bin/time` wall and maximum RSS measurements
cover Cargo and its children; maximum RSS is a process peak, not the sum of
parallel compiler processes.

Ordinary release binaries were built without profiling. The existing
[`paired-measure.py`](../scripts/paired-measure.py) ran three paired samples
with one warm-up per pair on a fixed CPU for `expand`, `paragraph-wide`, and
`pages`. It launched each binary directly with the same absolute benchmark
inputs, TeX search paths, and fresh output directory. Every process exited
successfully, and its stdout and raw `output.dvi` bytes matched the pinned
hashes; the DVI files were respectively 192, 502,060, and 316,388 bytes. No
normalization or output omission was used. Pair ratios below divide final by
baseline within each pair; values below one are lower elapsed time. Three
pairs characterize obvious regressions, but are too few to claim a stable
speedup, especially for the sub-100-ms `expand` case.

| Workload or gate         | Baseline wall median | Final wall median | Paired final/baseline median (range) | Baseline/final peak RSS median |
| ------------------------ | -------------------- | ----------------- | ------------------------------------ | ------------------------------ |
| Cold `tex-exec` compile  | 138.51 s             | 138.65 s          | 1.005 (1.001–1.008)                  | 1,595,680 / 1,586,956 KiB      |
| `expand` release         | 0.0913 s             | 0.0835 s          | 0.938 (0.840–1.056)                  | 34,432 / 34,176 KiB            |
| `paragraph-wide` release | 1.3266 s             | 1.2918 s          | 0.972 (0.909–1.044)                  | 135,296 / 135,040 KiB          |
| `pages` release          | 1.9737 s             | 1.8884 s          | 0.953 (0.911–1.017)                  | 98,688 / 99,200 KiB            |
| Warm `scripts/check.sh`  | 7.82 s               | 7.96 s            | 1.000 (0.997–1.026)                  | 136,320 / 137,160 KiB          |

All six cold builds succeeded. Warm quality timing followed one untimed pass
per side; each of the six timed passes passed all four selected dprint, Biome,
rustfmt, and clippy gates. The cold-build and release results show no stable
greater-than-10% regression. RSS medians are observations for these workloads,
not a general memory bound. The release preparation builds took approximately
2m51s and 2m50s; those are separate from cold test compile/link and from
steady-state runtime.

The separate `profiling` build counted allocation scopes and owner samples.
Its timing is diagnostic and is not included in release latency above. All
three repetitions on each side produced the same structural counters:

| Workload         | `semantic_apply` allocation calls, baseline/final | Requested bytes, baseline/final | Sample node live blocks, baseline/final |
| ---------------- | ------------------------------------------------: | ------------------------------: | --------------------------------------: |
| `expand`         |                                     6,255 / 6,240 |           1,022,347 / 1,019,995 |                                   1 / 1 |
| `paragraph-wide` |                                 534,292 / 401,398 |       294,840,073 / 280,581,193 |                                 27 / 27 |
| `pages`          |                               1,036,510 / 923,895 |       297,289,382 / 286,480,050 |                                 32 / 32 |

The one-shot generation census reports one generation created and dropped,
zero live after exit, and a peak of one for every profiling sample. An absent
counter is not treated as zero; the receipt records only named scopes present
on both revisions. Allocation requests count calls and requested bytes within
the instrumented scope, not total process allocations or retained memory.

## Incremental retention and compatibility

The unchanged `gentle-profile` public-session `prefix` edit probe alternated
two fixed 524-byte inputs for 6, 12, and 24 measured accepted edits, following
two warm-ups. It used no pure-query memo layers and checked every accepted DVI
against a cold run. Both revisions passed at every length with one retained
generation, no candidate generation after acceptance, four retained checkpoint
roots, zero replay-memo bytes, and chain depth one. The reported
`checkpoint_root_bytes` were 3,154,144 / 3,592,894 / 4,470,394 at baseline
and 3,154,096 / 3,592,846 / 4,470,346 at final. Their roughly 73,125-byte
per-edit rise on _both_ sides is not evidence of physical retained-memory
growth or a plateau. On these frozen revisions,
`SourceMapMark::checkpoint_retained_bytes` charges an absolute monotone logical
source position as if it were bytes. This contaminates the checkpoint-root
estimate and can affect soft pruning; a later correction and a new repeated-edit
receipt are required to assess actual retained payload. The frozen comparison
only establishes no added measured owner-count or reported-byte regression.

The identical 210-case manual command-semantic selection produced 79 matched
and 131 other-failure statuses on each side, with zero unexpected passes or
executed known failures. The same 131 case IDs failed. Reported mismatch
reasons were unchanged for 129 of them. The original revision panicked in
`main-control/current-font-selection` and
`page-output/insertion-split-footnote`; the final revision panicked in neither
and reached normal event/artifact comparisons. There are no new failing cases.
These results do not turn the 131 mismatches into strict known failures, and
unchanged mismatch messages do not prove byte equality for every unreported
channel. The independently checked three release DVI outputs above provide
exact output-byte evidence for their own inputs.

The named `scripts/check-snapshot-budgets.sh` could not exercise its benchmark
in either frozen checkout. `tex-state-benchmarks` failed to compile with the
same four API diagnostics (`E0061`, `E0308`, `E0599`, `E0624`), including a
missing `NodeTokenKey::words` method. Its budget invariants are unavailable
here, not passed, failed at runtime, or assigned a zero result. The separately
recorded final repair and gate run will determine the current invariant status.
