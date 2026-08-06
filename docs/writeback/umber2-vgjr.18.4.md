# Benchmark and prototype retirement

Issue: `umber2-vgjr.18.4`

## Fresh authority audit

The audit began at base `0ef0c0b58fdbbc893edd10f57e6b233cb742f12f`.
Tracked-source search and intervening history found no caller, baseline, or
authority added for `dvi_page_snapshot`, `mode_list_rollback`, or the
standalone PNG importer after the owner decision. The shared immutable DVI
representation and its correctness tests survive. The production inverse
journal, mutation-boundary gate, semantic comparison, and
`docs/mode_list_rollback_journal.md` survive. Production PNG decoding still
uses the bounded `StreamingDecoder`, with PDF/image correctness and corpus
gates unchanged.

Full-document TeX82 traces, canonical oracle traces, public
`tex_incr::{TraceOperation, TraceSummary}`, both scripted fuzz tiers, the
Cargo-fuzz target and seed, profiling tools, and every other benchmark row
remain. Absence from routine CI was not used as retirement evidence.

## Ownership result

The pure layout, allocation, and compact-width workloads moved from the mixed
`tex-exec` package to `benchmarks/tex-typeset`. The pure-memo accepted-edit
diagnostic moved to `benchmarks/tex-incr`. Shipout remains in
`benchmarks/tex-exec`; command and state workloads were already with their
measured owners. Every final manifest and command is documented in its README
and in `docs/testing_infrastructure.md`.

The five moved workload/baseline files retain their row names, sizes,
constants, failure behavior, and byte-identical `node-width-budgets.json`.
No baseline was updated.

## Deletion accounting

Retirement credit is 488 authored lines: 355 in the PNG prototype package
excluding its generated lockfile, 112 in the two historical Criterion sources,
and 21 manifest/README lines that exposed only those two targets. The 583
moved workload and baseline lines receive no deletion credit. The deleted PNG
lockfile, 24-line dependency-only shrink of the surviving `tex-exec` lockfile,
and the two new owner-package lockfiles are generated churn and also receive no
credit. The new locks preserve the original `tex-exec` dependency versions
rather than silently changing the measurement resolution. Documentation and
owner-package scaffolding are not retirement credit.

## Validation

All three final benchmark packages compile in release/bench profiles with six
Cargo jobs. The pure-memo, shipout, and every Criterion layout row execute
under 512 MiB guards. The unchanged width workload executes from its final
owner but rejects this x86 Linux host against its committed Apple/aarch64
timing baseline; follow-up `umber2-9508` owns host qualification without
rebaselining.

The unchanged deterministic layout-allocation gate also executes under 512
MiB and reproduces current-base ceiling drift under the original dependency
versions: line breaking uses 13 allocations against 12, deep sublists use
340,041 allocations/50,653,668 bytes against 180,000/42,000,000, and flat math
uses 606,616 bytes against 560,000. `umber2-dtis` owns diagnosis; no ceiling
changed here. The full native `cargo test -q --tests` suite passes under a 1
GiB guard, and all four default `scripts/check.sh` gates pass uncapped.
